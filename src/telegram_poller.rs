use crate::domain::{ExecutionMode, Item, NodeInstance, Workflow};
use crate::node::NodeRegistry;
use crate::state::AppState;
use crate::storage::Storage;
use std::sync::Arc;
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

            if let Err(e) = crate::execution_runner::run_and_track_execution(
                &storage,
                &events,
                &registry,
                &current_workflow,
                ExecutionMode::Telegram,
                Some(vec![trigger_item]),
                &credentials,
            )
            .await
            {
                tracing::error!(error = %e, workflow_id = %current_workflow.id, "telegram poller: failed to persist new execution");
            }
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
                },
                downstream,
            ],
            connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: connect_to.into(), to_input: 0 }],
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
            },
            "passthrough",
        );
        storage.create_workflow(&wf).await.unwrap();

        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, events, wf.clone(), "111:AAA".into(), telegram.uri()));

        // Bounded wait: the poller's tight loop against the mock (no real
        // 30s Telegram-side wait involved) should process the one queued
        // update well within this window. The outer timeout is a
        // belt-and-braces bound so a broken poll loop fails the test
        // instead of hanging the suite, matching the established pattern
        // in `http_request.rs`'s timeout test. Because
        // `poll_telegram_updates` processes one batch (get_updates,
        // resolve credentials, create+execute+update every update in it)
        // fully before looping around to its next `get_updates` call, by
        // the time the SECOND getUpdates request is observed here, the
        // first update's `Execution` row has already been created AND
        // updated -- no race with the assertions below.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if telegram.received_requests().await.unwrap().len() >= 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("poller should have made at least 2 getUpdates calls (one with an update, one empty) within 5s");

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
}
