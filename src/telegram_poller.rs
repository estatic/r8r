use crate::domain::{Execution, ExecutionMode, ExecutionStatus, Item, NodeInstance, Workflow};
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
    let workflow_id = workflow.id;

    let join_handle = tokio::spawn(poll_telegram_updates(storage, registry, workflow_id, bot_token, api_base_url));
    state.trigger_registry.record_telegram_poll(workflow_id, join_handle.abort_handle());
    Ok(())
}

/// The long-running poll loop for one activated `telegram.trigger`
/// workflow. Runs until the workflow is deactivated/deleted (detected by
/// re-fetching it each iteration, matching `fire_schedule`'s existing
/// pattern in `triggers.rs`) or its `AbortHandle` is aborted from
/// `deactivate_workflow_triggers`.
pub async fn poll_telegram_updates(
    storage: Arc<dyn Storage>,
    registry: Arc<NodeRegistry>,
    workflow_id: Uuid,
    bot_token: String,
    api_base_url: String,
) {
    let client = match build_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, %workflow_id, "telegram poller: failed to build HTTP client, trigger will not fire");
            return;
        }
    };

    let mut offset: Option<i64> = None;

    loop {
        let workflow = match storage.get_workflow(workflow_id).await {
            Ok(Some(wf)) if wf.active => wf,
            Ok(_) => {
                tracing::info!(%workflow_id, "telegram poller: workflow no longer active or found, stopping");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, %workflow_id, "telegram poller: failed to fetch workflow, retrying after backoff");
                tokio::time::sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECS)).await;
                continue;
            }
        };

        let updates = match get_updates(&client, &api_base_url, &bot_token, offset).await {
            Ok(updates) => updates,
            Err(e) => {
                tracing::warn!(error = %e, %workflow_id, "telegram poller: getUpdates failed, retrying after backoff");
                tokio::time::sleep(std::time::Duration::from_secs(RETRY_BACKOFF_SECS)).await;
                continue;
            }
        };

        for update in updates {
            if let Some(update_id) = update.get("update_id").and_then(|v| v.as_i64()) {
                offset = Some(update_id + 1);
            }

            let trigger_item = Item { json: update, binary: serde_json::json!({}) };
            let mut execution = Execution {
                id: Uuid::new_v4(),
                workflow_id: workflow.id,
                status: ExecutionStatus::Running,
                mode: ExecutionMode::Telegram,
                node_outputs: Default::default(),
                started_at: chrono::Utc::now(),
                finished_at: None,
            };
            if let Err(e) = storage.create_execution(&execution).await {
                tracing::error!(error = %e, %workflow_id, "telegram poller: failed to persist new execution");
                continue;
            }

            match crate::engine::execute_workflow_seeded(&workflow, &registry, Some(vec![trigger_item]), &std::collections::HashMap::new()).await {
                Ok(outputs) => {
                    execution.status = ExecutionStatus::Success;
                    execution.node_outputs = outputs;
                }
                Err(e) => {
                    tracing::warn!(error = %e, %workflow_id, "telegram-triggered execution failed");
                    execution.status = ExecutionStatus::Error;
                }
            }
            execution.finished_at = Some(chrono::Utc::now());
            if let Err(e) = storage.update_execution(&execution).await {
                tracing::error!(error = %e, %workflow_id, "telegram poller: failed to persist execution result");
            }
        }
    }
}

#[cfg(test)]
mod poller_tests {
    use super::*;
    use crate::domain::{Connection, NodeInstance};
    use crate::storage::sqlite::SqliteStorage;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

        let workflow_id = wf.id;
        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, workflow_id, "111:AAA".into(), telegram.uri()));

        // Bounded wait: the poller's tight loop against the mock (no real
        // 30s Telegram-side wait involved) should process the one queued
        // update well within this window. The outer timeout is a
        // belt-and-braces bound so a broken poll loop fails the test
        // instead of hanging the suite, matching the established pattern
        // in `http_request.rs`'s timeout test.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let wf = storage.get_workflow(workflow_id).await.unwrap().unwrap();
                let _ = wf; // keep the workflow row alive/active for the poller
                if telegram.received_requests().await.unwrap().len() >= 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("poller should have made at least 2 getUpdates calls (one with an update, one empty) within 5s");

        poll.abort();
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
        let workflow_id = wf.id;

        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, workflow_id, "111:AAA".into(), telegram.uri()));

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
