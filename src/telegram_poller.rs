use crate::domain::{ExecutionMode, Item, NodeInstance, Workflow};
use crate::node::NodeRegistry;
use crate::state::AppState;
use crate::storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

const DEFAULT_TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";

/// The `getUpdates` long-poll's own server-side wait (seconds). Telegram
/// blocks the HTTP response until either a new update arrives or this many
/// seconds elapse, whichever is first.
const GETUPDATES_TIMEOUT_SECS: u64 = 30;

/// The HTTP client's own request timeout must exceed `GETUPDATES_TIMEOUT_SECS`
/// — otherwise the client would abort the connection before Telegram's own
/// "no updates, here's an empty array" response at the 30s mark, turning
/// every idle poll into a spurious error. 5s of headroom is generous for
/// network latency on top of Telegram's own wait.
const CLIENT_TIMEOUT_SECS: u64 = GETUPDATES_TIMEOUT_SECS + 5;

/// Delay before retrying after a failed `getUpdates` call or a failed
/// workflow re-fetch, so a persistent failure (bad token, network outage)
/// doesn't spin the loop in a tight, log-flooding retry storm.
const RETRY_BACKOFF_SECS: u64 = 5;

/// A per-chat worker exits after this long without an update; the poller
/// spawns a fresh one on that chat's next update.
const CHAT_WORKER_IDLE: std::time::Duration = std::time::Duration::from_secs(300);

/// Chat id an update belongs to, from whichever update type carries one.
/// `None` for updates with no chat (polls, inline queries, ...), which
/// share one queue.
fn chat_key(update: &serde_json::Value) -> Option<i64> {
    ["message", "edited_message", "channel_post", "edited_channel_post"]
        .iter()
        .find_map(|field| update.get(field))
        .or_else(|| update.get("callback_query").and_then(|c| c.get("message")))
        .and_then(|m| m.get("chat"))
        .and_then(|c| c.get("id"))
        .and_then(|id| id.as_i64())
}

/// One update to run, with the workflow and credentials as they were when
/// its batch was polled.
struct TelegramJob {
    item: Item,
    workflow: Workflow,
    credentials: HashMap<Uuid, serde_json::Value>,
}

/// Routes updates to per-chat workers (Plan 8.7): each chat's updates run
/// one at a time in arrival order, different chats run in parallel, and
/// the poll loop never waits on a run.
struct ChatQueues {
    workers: HashMap<Option<i64>, mpsc::UnboundedSender<TelegramJob>>,
    storage: Arc<dyn Storage>,
    events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
    registry: Arc<NodeRegistry>,
    idle: std::time::Duration,
}

impl ChatQueues {
    fn new(
        storage: Arc<dyn Storage>,
        events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
        registry: Arc<NodeRegistry>,
        idle: std::time::Duration,
    ) -> Self {
        Self { workers: HashMap::new(), storage, events, registry, idle }
    }

    fn dispatch(&mut self, key: Option<i64>, job: TelegramJob) {
        // A worker that idled out has dropped its receiver, so the send
        // fails and hands the job back: spawn a fresh worker for it.
        let job = match self.workers.get(&key) {
            Some(tx) => match tx.send(job) {
                Ok(()) => return,
                Err(mpsc::error::SendError(job)) => job,
            },
            None => job,
        };
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(chat_worker(rx, self.storage.clone(), self.events.clone(), self.registry.clone(), self.idle));
        let _ = tx.send(job);
        self.workers.insert(key, tx);
    }
}

/// Waits up to `idle` for the worker's next job; `None` means the worker
/// should exit. On timeout the channel is closed *before* giving up, so a
/// send racing the timeout either lands here (and is drained) or fails and
/// hands the job back to `dispatch` -- it can never vanish with the
/// receiver.
async fn next_job(rx: &mut mpsc::UnboundedReceiver<TelegramJob>, idle: std::time::Duration) -> Option<TelegramJob> {
    match tokio::time::timeout(idle, rx.recv()).await {
        Ok(job) => job,
        Err(_) => {
            rx.close();
            rx.recv().await
        }
    }
}

async fn chat_worker(
    mut rx: mpsc::UnboundedReceiver<TelegramJob>,
    storage: Arc<dyn Storage>,
    events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
    registry: Arc<NodeRegistry>,
    idle: std::time::Duration,
) {
    while let Some(job) = next_job(&mut rx, idle).await {
        let workflow_id = job.workflow.id;
        match crate::execution_runner::start_execution(
            storage.clone(),
            events.clone(),
            registry.clone(),
            job.workflow,
            ExecutionMode::Telegram,
            Some(vec![job.item]),
            job.credentials,
        )
        .await
        {
            // Await before taking the next job: that's the per-chat ordering.
            Ok((_, handle)) => {
                if let Err(e) = handle.await {
                    tracing::error!(error = %e, workflow_id = %workflow_id, "telegram poller: execution task failed");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, workflow_id = %workflow_id, "telegram poller: failed to persist new execution");
            }
        }
    }
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(CLIENT_TIMEOUT_SECS))
        // Telegram's Bot API never legitimately redirects `getUpdates`, and
        // the bot token lives in the URL path (`/bot<token>/getUpdates`) —
        // disabling both prevents the token-bearing URL from leaking via a
        // redirect's `Referer` header, the same reasoning that drove this
        // fix on `telegram_send_message.rs`'s client in Plan 4b's final
        // review.
        .redirect(reqwest::redirect::Policy::none())
        .referer(false)
        .build()
        .map_err(|e| e.to_string())
}

/// Calls Telegram's `getUpdates` once and returns the raw `Update` objects
/// in Telegram's response `result` array. Never interpolates the raw
/// `bot_token`, the constructed `url`, or a raw `reqwest::Error` into any
/// returned error string — only Telegram's own `description` field (safe,
/// developer-facing text) or a fixed message naming the HTTP status
/// (never secret) ever appears.
async fn get_updates(
    client: &reqwest::Client,
    base_url: &str,
    bot_token: &str,
    offset: Option<i64>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut url = format!("{base_url}/bot{bot_token}/getUpdates?timeout={GETUPDATES_TIMEOUT_SECS}");
    if let Some(offset) = offset {
        url.push_str(&format!("&offset={offset}"));
    }

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|_| "request to Telegram getUpdates failed".to_string())?;
    let status = response.status();

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| format!("Telegram API returned HTTP {status} with a non-JSON response body"))?;

    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let description = body.get("description").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(format!("Telegram API returned an error: {description}"));
    }

    Ok(body.get("result").and_then(|v| v.as_array()).cloned().unwrap_or_default())
}

#[cfg(test)]
mod get_updates_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn parses_updates_from_a_successful_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": [{"update_id": 5, "message": {"text": "hi"}}]
            })))
            .mount(&server)
            .await;

        let client = build_client().unwrap();
        let updates = get_updates(&client, &server.uri(), "111:AAA", None).await.unwrap();
        assert_eq!(updates, vec![serde_json::json!({"update_id": 5, "message": {"text": "hi"}})]);
    }

    #[tokio::test]
    async fn empty_result_array_is_not_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
            .mount(&server)
            .await;

        let client = build_client().unwrap();
        let updates = get_updates(&client, &server.uri(), "111:AAA", None).await.unwrap();
        assert!(updates.is_empty());
    }

    #[tokio::test]
    async fn ok_false_returns_an_error_naming_the_description_only() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "description": "Unauthorized"
            })))
            .mount(&server)
            .await;

        let client = build_client().unwrap();
        let result = get_updates(&client, &server.uri(), "111:AAA", None).await;
        let err = result.unwrap_err();
        assert!(err.contains("Unauthorized"));
        assert!(!err.contains("111:AAA"));
    }

    #[tokio::test]
    async fn offset_is_sent_as_a_query_parameter() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .and(wiremock::matchers::query_param("offset", "42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
            .mount(&server)
            .await;

        let client = build_client().unwrap();
        let updates = get_updates(&client, &server.uri(), "111:AAA", Some(42)).await.unwrap();
        assert!(updates.is_empty());
    }

    #[tokio::test]
    async fn network_failure_error_message_never_contains_bot_token() {
        // Same technique Plan 4b's telegram_send_message.rs uses: start a
        // MockServer, capture its URI, then drop it (freeing the port) so
        // the next request hits a real closed-connection failure rather
        // than a mocked one.
        let server = MockServer::start().await;
        let uri = server.uri();
        drop(server);

        let client = build_client().unwrap();
        let result = get_updates(&client, &uri, "111:AAA", None).await;
        let err = result.unwrap_err();
        assert!(!err.contains("111:AAA"));
        assert!(!err.contains(&uri));
    }
}

/// Fetches the trigger node's referenced `telegramApi` credential, decrypts
/// it (via `Storage::get_credential`, which already returns decrypted data
/// — see `src/credentials.rs`'s `resolve_credentials_for_workflow` for the
/// identical call shape), and spawns a background long-polling task for
/// this workflow. The task's `AbortHandle` is recorded in
/// `state.trigger_registry` so `deactivate_workflow_triggers` can cancel it
/// later.
pub async fn activate_telegram_trigger(
    state: &AppState,
    workflow: &Workflow,
    trigger_node: &NodeInstance,
) -> anyhow::Result<()> {
    let credential_id_str = trigger_node
        .parameters
        .get("auth")
        .and_then(|a| a.get("credential_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("telegram.trigger requires parameters.auth.credential_id"))?;
    let credential_id = Uuid::parse_str(credential_id_str)
        .map_err(|e| anyhow::anyhow!("telegram.trigger has an invalid credential_id: {e}"))?;

    let credential = state
        .storage
        .get_credential(credential_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("credential {credential_id} does not exist"))?;
    let bot_token = credential
        .data
        .get("bot_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("telegramApi credential {credential_id} is missing \"bot_token\""))?
        .to_string();

    let api_base_url = trigger_node
        .parameters
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_TELEGRAM_API_BASE_URL)
        .to_string();

    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let events = state.execution_events.clone();
    let workflow_id = workflow.id;

    // Pass an owned clone of `workflow` (not just its id) so the poll loop's
    // very first iteration can skip the database active-check and trust
    // this caller-supplied workflow directly — see `poll_telegram_updates`'s
    // doc comment for why (closes an activation-race where the poller's
    // first `storage.get_workflow` read could lose a commit-visibility race
    // against the API handler's own `workflow.active = true` write).
    let join_handle = tokio::spawn(poll_telegram_updates(
        storage,
        registry,
        events,
        workflow.clone(),
        bot_token,
        api_base_url,
    ));
    state.trigger_registry.record_telegram_poll(workflow_id, join_handle.abort_handle());
    Ok(())
}

/// The long-running poll loop for one activated `telegram.trigger`
/// workflow. Runs until the workflow is deactivated/deleted (detected by
/// re-fetching it each iteration, matching `fire_schedule`'s existing
/// pattern in `triggers.rs`) or its `AbortHandle` is aborted from
/// `deactivate_workflow_triggers`.
///
/// Takes an owned `workflow` (rather than just its id) so the FIRST
/// iteration can skip the database active-check entirely and trust this
/// caller-supplied workflow directly. This closes an activation race: the
/// API handler that calls `activate_telegram_trigger` (and thus spawns this
/// task) persists `workflow.active = true` to storage only AFTER spawning
/// the poll task. Under a real multi-connection database pool, this task's
/// first `storage.get_workflow` read could race ahead of that write's
/// commit-visibility and observe `active: false`, causing the poller to
/// exit immediately and permanently — leaving an "active" workflow (per
/// storage and the API's 200 response) with no running poller and no
/// self-healing. Trusting the caller-supplied `workflow` for the first
/// iteration only avoids that read altogether; every iteration after the
/// first falls back to the normal fresh-fetch-then-check-`.active`
/// behavior, so a later deactivation is still detected exactly as before.
pub async fn poll_telegram_updates(
    storage: Arc<dyn Storage>,
    registry: Arc<NodeRegistry>,
    events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
    workflow: Workflow,
    bot_token: String,
    api_base_url: String,
) {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, workflow_id = %workflow.id, "telegram poller: failed to build HTTP client, trigger will not fire");
            return;
        }
    };

    let mut offset: Option<i64> = None;
    let mut current_workflow = workflow;
    let mut first_iteration = true;
    let mut queues = ChatQueues::new(storage.clone(), events.clone(), registry.clone(), CHAT_WORKER_IDLE);

    loop {
        if !first_iteration {
            current_workflow = match storage.get_workflow(current_workflow.id).await {
                Ok(Some(wf)) if wf.active => wf,
                Ok(_) => {
                    tracing::info!(workflow_id = %current_workflow.id, "telegram poller: workflow no longer active or found, stopping");
                    return;
                }
                Err(e) => {
                    tracing::warn!(error = %e, workflow_id = %current_workflow.id, "telegram poller: failed to fetch workflow, retrying after backoff");
                    tokio::time::sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECS)).await;
                    continue;
                }
            };
        }
        first_iteration = false;

        let updates = match get_updates(&client, &api_base_url, &bot_token, offset).await {
            Ok(updates) => updates,
            Err(e) => {
                tracing::warn!(error = %e, workflow_id = %current_workflow.id, "telegram poller: getUpdates failed, retrying after backoff");
                tokio::time::sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECS)).await;
                continue;
            }
        };

        if updates.is_empty() {
            // Nothing to process — skip the credential resolution DB
            // round-trip below entirely on every idle poll.
            continue;
        }

        // Resolve credentials for any downstream node ONCE per batch,
        // BEFORE any `offset` mutation happens below. If this fails (even a
        // transient DB error, not just a genuinely-missing credential), we
        // must NOT have already advanced `offset` past any update in this
        // batch — otherwise Telegram would never redeliver the skipped
        // update on the next `getUpdates` call, silently and permanently
        // losing it. Retrying the outer loop with `offset` untouched means
        // this exact batch is re-fetched from Telegram next iteration, and
        // once resolution succeeds every update in it is processed
        // normally. This also collapses the previous per-update redundant
        // credential fetch+decrypt into a single resolution per batch.
        let credentials = match crate::credentials::resolve_credentials_for_workflow(storage.as_ref(), &current_workflow).await {
            Ok(credentials) => credentials,
            Err(e) => {
                tracing::warn!(error = %e, workflow_id = %current_workflow.id, "telegram poller: failed to resolve credentials for this batch, retrying after backoff");
                tokio::time::sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECS)).await;
                continue;
            }
        };

        for update in updates {
            if let Some(update_id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = Some(update_id + 1);
            }

            let trigger_item = Item { json: update, binary: serde_json::json!({}) };

            let key = chat_key(&trigger_item.json);
            queues.dispatch(
                key,
                TelegramJob { item: trigger_item, workflow: current_workflow.clone(), credentials: credentials.clone() },
            );
        }
    }
}

#[cfg(test)]
mod poller_tests {
    use super::*;
    use crate::domain::{Connection, Credential, CredentialSummary, Execution, ExecutionStatus, NodeInstance, User, UserRole};
    use crate::storage::sqlite::SqliteStorage;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Wraps a real `SqliteStorage`, delegating every `Storage` method to it
    /// except `create_execution`/`update_execution`, which are ALSO
    /// recorded into `recorded`. `Storage` deliberately has no "list
    /// executions" method (confirmed out of scope to add), so an
    /// `Execution`'s randomly-generated id can't be looked up after the
    /// fact any other way -- this wrapper is the test-only mechanism used
    /// to prove the poll loop actually persisted an execution (Fix 3 Part A
    /// of the final-review fix wave), rather than only checking that the
    /// mock Telegram server received requests.
    struct RecordingStorage {
        inner: SqliteStorage,
        recorded: Arc<Mutex<Vec<Execution>>>,
    }

    #[async_trait]
    impl Storage for RecordingStorage {
        async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
            self.inner.create_workflow(workflow).await
        }
        async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
            self.inner.update_workflow(workflow).await
        }
        async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()> {
            self.inner.delete_workflow(id).await
        }
        async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>> {
            self.inner.get_workflow(id).await
        }
        async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
            self.inner.list_workflows().await
        }
        async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()> {
            self.inner.create_execution(execution).await?;
            self.recorded.lock().unwrap().push(execution.clone());
            Ok(())
        }
        async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()> {
            self.inner.update_execution(execution).await?;
            let mut recorded = self.recorded.lock().unwrap();
            if let Some(existing) = recorded.iter_mut().find(|e| e.id == execution.id) {
                *existing = execution.clone();
            } else {
                recorded.push(execution.clone());
            }
            Ok(())
        }
        async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>> {
            self.inner.get_execution(id).await
        }
        async fn list_executions_for_workflow(
            &self,
            workflow_id: Uuid,
            limit: i64,
        ) -> anyhow::Result<Vec<Execution>> {
            self.inner.list_executions_for_workflow(workflow_id, limit).await
        }
        async fn create_user(&self, user: &User) -> anyhow::Result<()> {
            self.inner.create_user(user).await
        }
        async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
            self.inner.get_user_by_email(email).await
        }
        async fn any_user_exists(&self) -> anyhow::Result<bool> {
            self.inner.any_user_exists().await
        }
        async fn create_credential(&self, credential: &Credential) -> anyhow::Result<()> {
            self.inner.create_credential(credential).await
        }
        async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>> {
            self.inner.get_credential(id).await
        }
        async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>> {
            self.inner.list_credentials().await
        }
        async fn update_credential(&self, credential: &crate::domain::Credential) -> anyhow::Result<bool> {
            self.inner.update_credential(credential).await
        }
        async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool> {
            self.inner.delete_credential(id).await
        }
    }

    fn trigger_workflow(credential_id: Uuid, api_base_url: &str, downstream: NodeInstance, connect_to: &str) -> Workflow {
        let now = chrono::Utc::now();
        Workflow {
            id: Uuid::new_v4(),
            name: "telegram-triggered".into(),
            active: true,
            nodes: vec![
                NodeInstance {
                    id: "trigger".into(),
                    node_type: "telegram.trigger".into(),
                    position: (0.0, 0.0),
                    parameters: serde_json::json!({
                        "auth": {"credential_id": credential_id.to_string()},
                        "api_base_url": api_base_url,
                    }),
                    disabled: false,
                    settings: Default::default(),
                },
                downstream,
            ],
            connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: connect_to.into(), to_input: 0, error: false }],
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn poll_loop_persists_a_telegram_mode_execution_per_update() {
        let telegram = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": [{"update_id": 1, "message": {"text": "hello"}}]
            })))
            .up_to_n_times(1)
            .mount(&telegram)
            .await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
            .mount(&telegram)
            .await;

        let inner = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
        let recorded: Arc<Mutex<Vec<Execution>>> = Arc::new(Mutex::new(Vec::new()));
        let storage: Arc<dyn Storage> = Arc::new(RecordingStorage { inner, recorded: recorded.clone() });
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        let registry = Arc::new(registry);

        // A real user + credential, so `resolve_credentials_for_workflow`
        // (which the poll loop now calls once per batch -- Fix 2) actually
        // succeeds: it scans every node's `auth.credential_id`, including
        // the trigger node's own, so that id must resolve to a real,
        // persisted credential for the loop to ever reach
        // `execute_workflow_seeded` and thus ever persist an execution.
        let owner_id = Uuid::new_v4();
        storage
            .create_user(&User {
                id: owner_id,
                email: format!("{owner_id}@example.com"),
                password_hash: "irrelevant-for-this-test".into(),
                role: UserRole::Owner,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let credential_id = Uuid::new_v4();
        storage
            .create_credential(&Credential {
                id: credential_id,
                name: "incoming-bot".into(),
                credential_type: "telegramApi".into(),
                data: serde_json::json!({"bot_token": "111:AAA"}),
                owner_id,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();

        let wf = trigger_workflow(
            credential_id,
            &telegram.uri(),
            NodeInstance {
                id: "passthrough".into(),
                node_type: "core.noop".into(),
                position: (1.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
                settings: Default::default(),
            },
            "passthrough",
        );
        storage.create_workflow(&wf).await.unwrap();

        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, events, wf.clone(), "111:AAA".into(), telegram.uri()));

        // Runs are background tasks since Plan 8.7: wait for the persisted
        // Telegram-mode execution to reach Success rather than for the
        // poller's next getUpdates call.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if recorded
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e.mode == ExecutionMode::Telegram && e.status == ExecutionStatus::Success)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("a Telegram-mode execution should reach Success within 5s");

        poll.abort();

        // Genuine proof of persistence: at least one `Execution` row was
        // created (and then updated) with `mode: Telegram` and
        // `status: Success` -- not just that the mock Telegram server saw
        // requests, which would pass even if the update's JSON never
        // reached the workflow or the execution was never persisted.
        let recorded = recorded.lock().unwrap();
        let modes_and_statuses: Vec<(ExecutionMode, ExecutionStatus)> =
            recorded.iter().map(|e| (e.mode.clone(), e.status.clone())).collect();
        assert!(
            recorded.iter().any(|e| e.mode == ExecutionMode::Telegram && e.status == ExecutionStatus::Success),
            "expected at least one persisted Execution with mode=Telegram, status=Success; recorded: {modes_and_statuses:?}"
        );
    }

    #[tokio::test]
    async fn poll_loop_stops_when_workflow_is_deactivated() {
        let telegram = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/bot111:AAA/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
            .mount(&telegram)
            .await;

        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap());
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        let registry = Arc::new(registry);

        let wf = trigger_workflow(
            Uuid::new_v4(),
            &telegram.uri(),
            NodeInstance {
                id: "passthrough".into(),
                node_type: "core.noop".into(),
                position: (1.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
                settings: Default::default(),
            },
            "passthrough",
        );
        storage.create_workflow(&wf).await.unwrap();

        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, events, wf.clone(), "111:AAA".into(), telegram.uri()));

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while telegram.received_requests().await.unwrap().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("poller should have made at least one getUpdates call within 5s");

        let mut deactivated = wf.clone();
        deactivated.active = false;
        storage.update_workflow(&deactivated).await.unwrap();

        // The loop re-checks `workflow.active` at the top of every
        // iteration (it's a tight loop against this mock, no 30s real-world
        // wait involved), so it should observe the flip and return on its
        // own without needing `poll.abort()` — proving the "workflow no
        // longer active" exit path, not just the AbortHandle path (which
        // Task 4's tests cover via `deactivate_workflow_triggers`).
        tokio::time::timeout(std::time::Duration::from_secs(5), poll)
            .await
            .expect("poll task should exit on its own once the workflow is deactivated")
            .expect("poll task must not panic");
    }

    use std::sync::Mutex as StdMutex;

    #[test]
    fn chat_key_reads_every_supported_update_shape() {
        for field in ["message", "edited_message", "channel_post", "edited_channel_post"] {
            let update = serde_json::json!({"update_id": 1, field: {"chat": {"id": 42}}});
            assert_eq!(chat_key(&update), Some(42), "{field}");
        }
        let callback = serde_json::json!({"update_id": 1, "callback_query": {"message": {"chat": {"id": -7}}}});
        assert_eq!(chat_key(&callback), Some(-7));
        let no_chat = serde_json::json!({"update_id": 1, "poll": {"id": "x"}});
        assert_eq!(chat_key(&no_chat), None);
    }

    /// Records `json.seq` after sleeping `json.sleep_ms`.
    struct RecordNode {
        seen: Arc<StdMutex<Vec<i64>>>,
    }

    #[async_trait::async_trait]
    impl crate::node::Node for RecordNode {
        fn type_name(&self) -> &'static str {
            "test.record"
        }
        fn display_name(&self) -> &'static str {
            "Record"
        }
        fn description(&self) -> &'static str {
            "Test-only node that records the order items arrive in."
        }
        fn category(&self) -> crate::node::NodeCategory {
            crate::node::NodeCategory::Action
        }
        async fn execute(&self, ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            let json = &ctx.input_items[0].json;
            tokio::time::sleep(std::time::Duration::from_millis(json["sleep_ms"].as_u64().unwrap_or(0))).await;
            self.seen.lock().unwrap().push(json["seq"].as_i64().unwrap());
            Ok(vec![ctx.input_items.clone()])
        }
    }

    async fn queue_fixture(idle: std::time::Duration) -> (ChatQueues, Workflow, Arc<StdMutex<Vec<i64>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        registry.register(Box::new(RecordNode { seen: seen.clone() }));
        let storage: Arc<dyn Storage> =
            Arc::new(crate::storage::sqlite::SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap());
        let wf = Workflow {
            id: Uuid::new_v4(),
            name: "queue".into(),
            active: true,
            nodes: vec![
                NodeInstance { id: "trigger".into(), node_type: "core.manualTrigger".into(), position: (0.0, 0.0), parameters: serde_json::json!({}), disabled: false, settings: Default::default() },
                NodeInstance { id: "rec".into(), node_type: "test.record".into(), position: (1.0, 0.0), parameters: serde_json::json!({}), disabled: false, settings: Default::default() },
            ],
            connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: "rec".into(), to_input: 0, error: false }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        storage.create_workflow(&wf).await.unwrap();
        let (events, _rx) = tokio::sync::broadcast::channel(64);
        (ChatQueues::new(storage, events, Arc::new(registry), idle), wf, seen)
    }

    fn job(wf: &Workflow, seq: i64, sleep_ms: u64) -> TelegramJob {
        TelegramJob {
            item: Item { json: serde_json::json!({"seq": seq, "sleep_ms": sleep_ms}), binary: serde_json::json!({}) },
            workflow: wf.clone(),
            credentials: HashMap::new(),
        }
    }

    async fn wait_until(seen: &Arc<StdMutex<Vec<i64>>>, pred: impl Fn(&[i64]) -> bool) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !pred(&seen.lock().unwrap()) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("condition not reached within 5s");
    }

    #[tokio::test]
    async fn same_chat_updates_run_in_arrival_order() {
        let (mut queues, wf, seen) = queue_fixture(CHAT_WORKER_IDLE).await;
        queues.dispatch(Some(1), job(&wf, 1, 200));
        queues.dispatch(Some(1), job(&wf, 2, 0));
        wait_until(&seen, |s| s.len() == 2).await;
        assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn a_slow_chat_does_not_block_another_chat() {
        let (mut queues, wf, seen) = queue_fixture(CHAT_WORKER_IDLE).await;
        queues.dispatch(Some(1), job(&wf, 1, 1000));
        queues.dispatch(Some(2), job(&wf, 2, 0));
        wait_until(&seen, |s| s.contains(&2)).await;
        assert!(!seen.lock().unwrap().contains(&1), "chat 2 finished while chat 1 was still running");
    }

    #[tokio::test]
    async fn dispatch_after_idle_exit_respawns_worker() {
        let (mut queues, wf, seen) = queue_fixture(std::time::Duration::from_millis(50)).await;
        queues.dispatch(Some(1), job(&wf, 1, 0));
        wait_until(&seen, |s| s.len() == 1).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await; // worker idles out
        queues.dispatch(Some(1), job(&wf, 2, 0));
        wait_until(&seen, |s| s.len() == 2).await;
        assert_eq!(*seen.lock().unwrap(), vec![1, 2]);
    }

    #[tokio::test]
    async fn an_idle_worker_refuses_sends_once_it_decides_to_exit() {
        // The race: the idle timeout fires, and before the worker drops its
        // receiver the poller's send succeeds -- the job would vanish with
        // the receiver. next_job must close the channel before returning
        // None, so that late send fails and dispatch respawns a worker.
        let (_, wf, _) = queue_fixture(CHAT_WORKER_IDLE).await;
        let (tx, mut rx) = mpsc::unbounded_channel();
        assert!(next_job(&mut rx, std::time::Duration::from_millis(1)).await.is_none());
        // rx is still alive here, exactly like inside chat_worker before it returns.
        assert!(tx.send(job(&wf, 1, 0)).is_err(), "a send after the exit decision must be refused");
    }

    #[tokio::test]
    async fn an_idle_worker_still_runs_a_job_that_arrived_before_it_closed() {
        let (_, wf, _) = queue_fixture(CHAT_WORKER_IDLE).await;
        let (tx, mut rx) = mpsc::unbounded_channel::<TelegramJob>();
        tx.send(job(&wf, 7, 0)).unwrap();
        let got = next_job(&mut rx, std::time::Duration::from_millis(1)).await;
        assert_eq!(got.map(|j| j.item.json["seq"].as_i64().unwrap()), Some(7));
    }
}
