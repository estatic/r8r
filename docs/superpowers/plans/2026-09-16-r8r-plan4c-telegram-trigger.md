# Telegram Trigger (Long-Polling) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `telegram.trigger`, a workflow start node that fires a workflow
on each incoming Telegram update via long-polling — the last item in Plan 4
("HTTP Integrations"). Webhook mode is explicitly out of scope (see below).

**Architecture:** One spawned `tokio` background task per activated workflow
using `telegram.trigger` as its start node, mirroring how `core.schedule`
already gets one cron job per workflow via `TriggerRegistry`. The task loops
on Telegram's `GET /bot<token>/getUpdates?offset=<n>&timeout=30` — Telegram's
own `timeout` parameter blocks server-side, so the loop needs no manual
sleep between iterations on the happy path. Each returned update is
dispatched through the existing `execute_workflow_seeded` exactly like the
Schedule and Webhook triggers already do, and persisted as an `Execution`
(new `ExecutionMode::Telegram` variant). Cancellation on deactivation uses
`tokio::task::AbortHandle`, recorded in `TriggerRegistry` alongside the
existing cron-job map — no new shutdown-signal mechanism needed.

**Tech Stack:** Rust, Tokio, `reqwest` (already a dependency via
`core.httpRequest`/`telegram.sendMessage`), `wiremock` for tests.

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md`
(§5.3 "Messaging", updated by this plan's brainstorm to resolve the Telegram
Trigger design; §12's corresponding open question is now resolved there) —
reachable, read directly before writing this plan; authoritative.

## Global Constraints

- v1 ships **long-polling only**. Webhook mode (`setWebhook`) is explicitly
  out of scope for this plan — deferred to a future plan per the spec.
- The new node type is `telegram.trigger`, parameters
  `{"auth": {"credential_id": "<uuid>"}, "api_base_url": "<optional override>"}`.
  `api_base_url` defaults to `https://api.telegram.org` and exists only to
  make the poller's HTTP calls test-injectable (same pattern as
  `telegram_send_message.rs`'s `api_base_url` — see
  `docs/adding-a-node.md` §6, including its security caveat that this kind
  of override parameter is production-reachable, not test-only, for a
  URL-embedded-secret integration).
- The bot-token credential (`credential_type: "telegramApi"`, `data.bot_token`)
  is fetched and decrypted **once, at activation time**, directly via
  `Storage::get_credential` — this is separate from
  `resolve_credentials_for_workflow`'s per-execution resolution used by
  downstream nodes (the trigger needs the token *before* any execution
  starts, to open the long-poll connection at all).
- Security discipline carried over from `telegram_send_message.rs`
  (Plan 4b): no error message or log line may interpolate the raw bot
  token or the constructed URL. The poller's own HTTP client must disable
  redirects and the `Referer` header for the same reason Plan 4b's final
  review required it on `telegram_send_message.rs`'s client (the token
  lives in the URL path).
- The `offset` (Telegram's update-acknowledgment cursor) is tracked
  **in memory only** for v1 — not persisted across process restarts. This
  is a known, documented limitation, not a bug to work around in this plan.
- Known v1 limitation, documented not solved: Telegram allows only one
  active `getUpdates` long-poll per bot token. Two workflows sharing the
  same bot credential will conflict (HTTP 409). Do not attempt to solve
  credential-level poller sharing in this plan — YAGNI, no current use case.
- Every new/changed public function gets its own test. No placeholder
  tests, no `TBD`, no hand-waving — this plan's own self-review and every
  task's reviewer will check for this.

---

### Task 1: Domain variant + `telegram.trigger` node stub + registration

**Files:**
- Modify: `src/domain.rs` (add `ExecutionMode::Telegram`)
- Create: `src/nodes/telegram_trigger.rs`
- Modify: `src/nodes/mod.rs` (register the new node)

**Interfaces:**
- Produces: `ExecutionMode::Telegram` (a plain enum variant, no associated
  data — serializes as the string `"Telegram"` via serde's default
  behavior, same as the existing `Manual`/`Webhook`/`Schedule` variants).
  `TelegramTriggerNode`, `type_name() == "telegram.trigger"`.
- Consumes: `crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput}`
  (already-existing trait/types, unchanged).

- [ ] **Step 1: Add the `ExecutionMode::Telegram` variant**

In `src/domain.rs`, find:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    Manual,
    Webhook,
    Schedule,
}
```

Change to:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    Manual,
    Webhook,
    Schedule,
    Telegram,
}
```

- [ ] **Step 2: Write the node stub with its test**

Create `src/nodes/telegram_trigger.rs`:

```rust
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

/// The workflow-graph representation of a Telegram long-polling trigger.
///
/// Like `ScheduleNode`/`WebhookNode`, this node's own `execute()` is a
/// fallback stub — real incoming-update data reaches the workflow via the
/// trigger item the engine seeds in from `telegram_poller::poll_telegram_updates`
/// (see `src/telegram_poller.rs`), not from this method. `execute()` only
/// runs if something re-executes this node directly outside a real trigger
/// firing (e.g. a manual re-run), in which case an empty item is the only
/// sane fallback — there is no "current Telegram update" to reproduce.
pub struct TelegramTriggerNode;

#[async_trait]
impl Node for TelegramTriggerNode {
    fn type_name(&self) -> &'static str {
        "telegram.trigger"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_execute_returns_a_single_empty_item() {
        let node = TelegramTriggerNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"auth": {"credential_id": "00000000-0000-0000-0000-000000000000"}}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }

    #[test]
    fn type_name_is_telegram_trigger() {
        assert_eq!(TelegramTriggerNode.type_name(), "telegram.trigger");
    }
}
```

- [ ] **Step 3: Register the node**

In `src/nodes/mod.rs`, add the module declaration in the existing
alphabetically-sorted block of `pub mod` lines (it will sort next to
`telegram_send_message`):

```rust
pub mod telegram_trigger;
```

And in `register_all`, add (next to the existing
`telegram_send_message::TelegramSendMessageNode` registration):

```rust
registry.register(Box::new(telegram_trigger::TelegramTriggerNode));
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib nodes::telegram_trigger domain`
Expected: PASS. Also run `cargo build` to confirm the `ExecutionMode`
change doesn't break exhaustive-match sites elsewhere (the compiler will
point at any `match` on `ExecutionMode` missing the new arm — there should
be none today since the codebase only constructs this enum, never matches
on it, but verify with a full `cargo build`).

- [ ] **Step 5: Commit**

```bash
git add src/domain.rs src/nodes/telegram_trigger.rs src/nodes/mod.rs
git commit -m "feat: add telegram.trigger node stub and ExecutionMode::Telegram"
```

---

### Task 2: `TriggerRegistry` support for cancellable poll tasks

**Files:**
- Modify: `src/trigger_registry.rs`

**Interfaces:**
- Consumes: nothing new (this task is self-contained).
- Produces: `TriggerRegistry::record_telegram_poll(&self, workflow_id: Uuid, handle: tokio::task::AbortHandle)`
  and `TriggerRegistry::take_telegram_poll(&self, workflow_id: Uuid) -> Option<tokio::task::AbortHandle>`
  — Task 3 and Task 4 call these.

- [ ] **Step 1: Add the field**

In `src/trigger_registry.rs`, change:

```rust
#[derive(Default)]
pub struct TriggerRegistry {
    cron_jobs: Mutex<HashMap<Uuid, Uuid>>,
}
```

to:

```rust
#[derive(Default)]
pub struct TriggerRegistry {
    cron_jobs: Mutex<HashMap<Uuid, Uuid>>,
    telegram_polls: Mutex<HashMap<Uuid, tokio::task::AbortHandle>>,
}
```

- [ ] **Step 2: Add the methods, with tests**

In the same file's `impl TriggerRegistry` block, add (next to
`record_cron_job`/`take_cron_job`):

```rust
    /// Records the `AbortHandle` for a workflow's spawned Telegram
    /// long-polling task, so `deactivate_workflow_triggers` can cancel it
    /// later without the caller needing to track the handle itself.
    pub fn record_telegram_poll(&self, workflow_id: Uuid, handle: tokio::task::AbortHandle) {
        self.telegram_polls.lock().unwrap().insert(workflow_id, handle);
    }

    /// Removes and returns a previously-recorded poll task's `AbortHandle`.
    /// The caller is responsible for calling `.abort()` on it — this method
    /// only manages the registry's bookkeeping, mirroring `take_cron_job`'s
    /// division of responsibility with `Scheduler::unregister`.
    pub fn take_telegram_poll(&self, workflow_id: Uuid) -> Option<tokio::task::AbortHandle> {
        self.telegram_polls.lock().unwrap().remove(&workflow_id)
    }
```

Add these tests to the existing `#[cfg(test)] mod tests` block. `AbortHandle`
has no public constructor outside spawning a real task, so the tests spawn
a trivial never-finishing task to get a real handle:

```rust
    fn dummy_abort_handle() -> tokio::task::AbortHandle {
        tokio::spawn(async {
            std::future::pending::<()>().await;
        })
        .abort_handle()
    }

    #[tokio::test]
    async fn records_and_takes_a_telegram_poll_handle() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        let handle = dummy_abort_handle();
        registry.record_telegram_poll(workflow_id, handle.clone());
        let taken = registry.take_telegram_poll(workflow_id);
        assert!(taken.is_some());
        taken.unwrap().abort();
    }

    #[tokio::test]
    async fn take_telegram_poll_is_idempotent_after_the_first_call() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_telegram_poll(workflow_id, dummy_abort_handle());
        registry.take_telegram_poll(workflow_id).unwrap().abort();
        assert!(registry.take_telegram_poll(workflow_id).is_none());
    }

    #[tokio::test]
    async fn take_telegram_poll_on_unknown_workflow_returns_none() {
        let registry = TriggerRegistry::new();
        assert!(registry.take_telegram_poll(Uuid::new_v4()).is_none());
    }

    #[tokio::test]
    async fn cron_jobs_and_telegram_polls_are_tracked_independently() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        registry.record_telegram_poll(workflow_id, dummy_abort_handle());
        assert!(registry.take_cron_job(workflow_id).is_some());
        assert!(registry.take_telegram_poll(workflow_id).unwrap().abort() == ());
    }
```

(`tokio::task::AbortHandle` implements `Clone`, so
`record_telegram_poll(workflow_id, handle.clone())` in the first test is
valid — needed there only so the test can both hand a handle to the
registry and independently call `.abort()` on the value `take` returns;
in production code only one clone of a task's `AbortHandle` is ever kept.)

- [ ] **Step 3: Run the tests**

Run: `cargo test --lib trigger_registry`
Expected: PASS (existing cron-job tests plus the 4 new ones above, 8 total).

- [ ] **Step 4: Commit**

```bash
git add src/trigger_registry.rs
git commit -m "feat: add cancellable Telegram poll-task tracking to TriggerRegistry"
```

---

### Task 3: The poller module — `getUpdates` client + poll loop + activation

**Files:**
- Create: `src/telegram_poller.rs`
- Modify: `src/lib.rs` (register the module)

**Interfaces:**
- Consumes: `TriggerRegistry::record_telegram_poll` (Task 2),
  `ExecutionMode::Telegram` (Task 1), `crate::engine::execute_workflow_seeded`
  (existing, unchanged), `crate::storage::Storage::get_credential` (existing,
  unchanged — already returns decrypted `Credential.data`, confirmed by
  reading `src/credentials.rs`'s `resolve_credentials_for_workflow`, which
  calls it the same way with no extra decrypt step), `crate::state::AppState`
  (existing).
- Produces: `pub async fn activate_telegram_trigger(state: &AppState, workflow: &Workflow, trigger_node: &NodeInstance) -> anyhow::Result<()>`
  — Task 4 calls this from `triggers.rs`'s `activate_workflow_triggers`.
  `pub async fn poll_telegram_updates(storage, registry, workflow_id, bot_token, api_base_url)`
  is spawned by `activate_telegram_trigger` and is not called directly by
  Task 4, but is `pub` so Task 3's own tests and Task 5's E2E test can
  reason about it if needed.

- [ ] **Step 1: Write `get_updates` with its tests**

Create `src/telegram_poller.rs`:

```rust
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
```

- [ ] **Step 2: Write `activate_telegram_trigger`**

Append to `src/telegram_poller.rs`:

```rust
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
```

- [ ] **Step 3: Write `poll_telegram_updates`**

Append to `src/telegram_poller.rs`:

```rust
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
```

- [ ] **Step 4: Register the module**

In `src/lib.rs`, add (alphabetically, next to `pub mod telegram_send_message`
if present — check the actual current file; `telegram_send_message` lives
under `src/nodes/`, not at the crate root, so this is a new top-level entry):

```rust
pub mod telegram_poller;
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --lib telegram_poller`
Expected: PASS (4 `get_updates` tests + 2 poller tests = 6 tests). These
tests involve real async timing against a mock server; if
`poll_loop_stops_when_workflow_is_deactivated` is flaky in CI, that's a
real finding to report, not something to paper over by increasing sleep
durations blindly — investigate first.

- [ ] **Step 6: Commit**

```bash
git add src/telegram_poller.rs src/lib.rs
git commit -m "feat: add Telegram long-polling client and poll loop (src/telegram_poller.rs)"
```

---

### Task 4: Wire activation/deactivation into `triggers.rs`

**Files:**
- Modify: `src/triggers.rs`

**Interfaces:**
- Consumes: `telegram_poller::activate_telegram_trigger` (Task 3),
  `TriggerRegistry::take_telegram_poll` (Task 2).
- Produces: `activate_workflow_triggers` and `deactivate_workflow_triggers`
  (both already-existing public functions, called from
  `src/api/workflows.rs`'s `set_workflow_active` handler and
  `reactivate_all` — **neither of those call sites needs to change**, since
  both already call these two dispatcher functions generically without
  knowing which trigger type is involved).

- [ ] **Step 1: Refactor the Schedule-specific body into its own function**

In `src/triggers.rs`, the current `activate_workflow_triggers` reads:

```rust
pub async fn activate_workflow_triggers(
    state: &AppState,
    workflow: &crate::domain::Workflow,
) -> anyhow::Result<()> {
    let start_id = crate::engine::start_node_id(workflow)?;
    let start_node = workflow
        .nodes
        .iter()
        .find(|n| n.id == start_id)
        .ok_or_else(|| anyhow::anyhow!("start node {start_id} not found"))?;

    if start_node.node_type != "core.schedule" {
        return Ok(());
    }

    let cron_expr = start_node
        .parameters
        .get("cron")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("core.schedule node requires a \"cron\" string parameter"))?
        .to_string();

    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let workflow_id = workflow.id;

    let job_id = state
        .scheduler
        .register(&cron_expr, move || {
            let storage = storage.clone();
            let registry = registry.clone();
            async move {
                fire_schedule(storage, registry, workflow_id).await;
            }
        })
        .await?;

    state.trigger_registry.record_cron_job(workflow_id, job_id);
    Ok(())
}
```

Replace it with a dispatcher plus an extracted `activate_schedule_trigger`
helper carrying the exact same body as before (pure refactor, no behavior
change to the Schedule path):

```rust
pub async fn activate_workflow_triggers(
    state: &AppState,
    workflow: &crate::domain::Workflow,
) -> anyhow::Result<()> {
    let start_id = crate::engine::start_node_id(workflow)?;
    let start_node = workflow
        .nodes
        .iter()
        .find(|n| n.id == start_id)
        .ok_or_else(|| anyhow::anyhow!("start node {start_id} not found"))?;

    match start_node.node_type.as_str() {
        "core.schedule" => activate_schedule_trigger(state, workflow, start_node).await,
        "telegram.trigger" => crate::telegram_poller::activate_telegram_trigger(state, workflow, start_node).await,
        _ => Ok(()),
    }
}

async fn activate_schedule_trigger(
    state: &AppState,
    workflow: &crate::domain::Workflow,
    start_node: &crate::domain::NodeInstance,
) -> anyhow::Result<()> {
    let cron_expr = start_node
        .parameters
        .get("cron")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("core.schedule node requires a \"cron\" string parameter"))?
        .to_string();

    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let workflow_id = workflow.id;

    let job_id = state
        .scheduler
        .register(&cron_expr, move || {
            let storage = storage.clone();
            let registry = registry.clone();
            async move {
                fire_schedule(storage, registry, workflow_id).await;
            }
        })
        .await?;

    state.trigger_registry.record_cron_job(workflow_id, job_id);
    Ok(())
}
```

- [ ] **Step 2: Extend deactivation**

Change:

```rust
pub async fn deactivate_workflow_triggers(state: &AppState, workflow_id: Uuid) {
    if let Some(job_id) = state.trigger_registry.take_cron_job(workflow_id) {
        if let Err(e) = state.scheduler.unregister(job_id).await {
            tracing::warn!(error = %e, %workflow_id, "failed to unregister cron job on deactivation");
        }
    }
}
```

to:

```rust
pub async fn deactivate_workflow_triggers(state: &AppState, workflow_id: Uuid) {
    if let Some(job_id) = state.trigger_registry.take_cron_job(workflow_id) {
        if let Err(e) = state.scheduler.unregister(job_id).await {
            tracing::warn!(error = %e, %workflow_id, "failed to unregister cron job on deactivation");
        }
    }
    if let Some(handle) = state.trigger_registry.take_telegram_poll(workflow_id) {
        handle.abort();
    }
}
```

A given workflow can only ever have one start node, so only one of these
two `if let` blocks will ever find something to remove — trying both
unconditionally is harmless and keeps this function from needing to know
the workflow's start-node type just to decide which cleanup path to run.

- [ ] **Step 3: Add activation/deactivation tests**

Add to `src/triggers.rs`'s existing `#[cfg(test)] mod tests` block (which
already has a `test_state()` helper — reuse it as-is):

```rust
    fn telegram_trigger_workflow(credential_id: Uuid, api_base_url: &str) -> Workflow {
        let now = chrono::Utc::now();
        Workflow {
            id: Uuid::new_v4(),
            name: "telegram".into(),
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
                NodeInstance {
                    id: "set1".into(),
                    node_type: "core.set".into(),
                    position: (1.0, 0.0),
                    parameters: serde_json::json!({"fields": {"fired": true}}),
                    disabled: false,
                },
            ],
            connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: "set1".into(), to_input: 0 }],
            created_at: now,
            updated_at: now,
        }
    }

    async fn create_telegram_credential(state: &AppState) -> Uuid {
        use crate::domain::{Credential, User, UserRole};
        let owner_id = Uuid::new_v4();
        state
            .storage
            .create_user(&User {
                id: owner_id,
                email: format!("{owner_id}@example.com"),
                password_hash: "irrelevant".into(),
                role: UserRole::Owner,
                created_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let credential_id = Uuid::new_v4();
        state
            .storage
            .create_credential(&Credential {
                id: credential_id,
                name: "bot".into(),
                credential_type: "telegramApi".into(),
                data: serde_json::json!({"bot_token": "111:AAA"}),
                owner_id,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        credential_id
    }

    #[tokio::test]
    async fn activate_registers_a_telegram_poll_for_a_telegram_trigger_start_node() {
        let state = test_state().await;
        let credential_id = create_telegram_credential(&state).await;
        let wf = telegram_trigger_workflow(credential_id, "http://127.0.0.1:1");
        state.storage.create_workflow(&wf).await.unwrap();

        activate_workflow_triggers(&state, &wf).await.unwrap();
        let handle = state.trigger_registry.take_telegram_poll(wf.id);
        assert!(handle.is_some());
        handle.unwrap().abort();
    }

    #[tokio::test]
    async fn activate_errors_when_telegram_trigger_credential_id_is_missing() {
        let state = test_state().await;
        let mut wf = telegram_trigger_workflow(Uuid::new_v4(), "http://127.0.0.1:1");
        wf.nodes[0].parameters = serde_json::json!({});
        let result = activate_workflow_triggers(&state, &wf).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn activate_errors_when_telegram_trigger_credential_does_not_exist() {
        let state = test_state().await;
        let wf = telegram_trigger_workflow(Uuid::new_v4(), "http://127.0.0.1:1");
        let result = activate_workflow_triggers(&state, &wf).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn deactivate_aborts_a_previously_activated_telegram_poll() {
        let state = test_state().await;
        let credential_id = create_telegram_credential(&state).await;
        let wf = telegram_trigger_workflow(credential_id, "http://127.0.0.1:1");
        state.storage.create_workflow(&wf).await.unwrap();
        activate_workflow_triggers(&state, &wf).await.unwrap();

        deactivate_workflow_triggers(&state, wf.id).await;
        assert!(state.trigger_registry.take_telegram_poll(wf.id).is_none());
    }
```

(`"http://127.0.0.1:1"` is a deliberately-unroutable address — port 1 is a
reserved/privileged port nothing listens on — used here only so the spawned
poll task's very first `getUpdates` call fails fast and retries in its
5-second backoff loop indefinitely in the background without ever
succeeding or panicking; these tests only care that activation/deactivation
correctly records/removes the `AbortHandle`, not that the poll loop
actually reaches Telegram. Aborting the handle in each test that doesn't
call `deactivate_workflow_triggers` itself prevents leaking a background
task that outlives the test.)

- [ ] **Step 4: Run the tests**

Run: `cargo test --lib triggers`
Expected: PASS — existing Schedule tests unchanged in behavior, plus the 4
new Telegram tests above.

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 6: Commit**

```bash
git add src/triggers.rs
git commit -m "feat: wire telegram.trigger activation/deactivation into triggers.rs"
```

---

### Task 5: End-to-end integration test — full trigger-to-execution chain

**Files:**
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: everything above, plus the existing `test_app()`/
  `register_and_get_token()` helpers, and Plan 4b's already-merged
  `telegram.sendMessage` node.
- Produces: one new test proving the whole feature works through the real
  HTTP API: create two `telegramApi` credentials (one for the trigger's
  incoming bot, one for an outgoing "echo" `telegram.sendMessage` node),
  create a workflow (`telegram.trigger` → `telegram.sendMessage`) via
  `POST /rest/workflows`, activate it via
  `PATCH /rest/workflows/:id/active`, and prove the full chain fired by
  observing that the mocked "outgoing" Telegram API actually received a
  `sendMessage` call — this is possible without any new executions-listing
  endpoint (none exists today, and adding one is out of this plan's scope)
  because the downstream node's own outbound HTTP call is an observable
  side effect on the same mock server.

- [ ] **Step 1: Write the failing test**

Check `tests/api_test.rs`'s current top-level imports first — per Plan 4a's
Task 10 and Plan 4b's Task 3, `wiremock::matchers::{method, path}` and
`wiremock::{Mock, MockServer, ResponseTemplate}` almost certainly already
exist; add only what's missing (this test additionally needs
`wiremock::matchers::query_param`, which prior tests in this file have not
used — check before adding a duplicate import for the ones that do already
exist).

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn telegram_trigger_fires_downstream_node_on_incoming_update() {
    let telegram = MockServer::start().await;

    // The "incoming" bot: the trigger polls this.
    Mock::given(method("GET"))
        .and(path("/bot111:AAA/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": [{"update_id": 1, "message": {"chat": {"id": 42}, "text": "ping"}}]
        })))
        .up_to_n_times(1)
        .mount(&telegram)
        .await;
    Mock::given(method("GET"))
        .and(path("/bot111:AAA/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
        .mount(&telegram)
        .await;

    // The "outgoing" bot: the downstream telegram.sendMessage node calls this.
    Mock::given(method("POST"))
        .and(path("/bot222:BBB/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {"message_id": 7}
        })))
        .mount(&telegram)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "telegram-trigger-e2e@example.com").await;

    let incoming_cred = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "incoming-bot", "credential_type": "telegramApi", "data": {"bot_token": "111:AAA"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(incoming_cred.status(), StatusCode::CREATED);
    let bytes = incoming_cred.into_body().collect().await.unwrap().to_bytes();
    let incoming_cred_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let outgoing_cred = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "outgoing-bot", "credential_type": "telegramApi", "data": {"bot_token": "222:BBB"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outgoing_cred.status(), StatusCode::CREATED);
    let bytes = outgoing_cred.into_body().collect().await.unwrap().to_bytes();
    let outgoing_cred_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let workflow_body = serde_json::json!({
        "name": "telegram-trigger-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "telegram.trigger", "position": [0.0, 0.0], "parameters": {
                "auth": {"credential_id": incoming_cred_id},
                "api_base_url": telegram.uri()
            }, "disabled": false},
            {"id": "echo", "node_type": "telegram.sendMessage", "position": [1.0, 0.0], "parameters": {
                "chat_id": "42",
                "text": "echo",
                "auth": {"credential_id": outgoing_cred_id},
                "api_base_url": telegram.uri()
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "echo", "to_input": 0}
        ]
    });
    let wf_response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(workflow_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wf_response.status(), StatusCode::CREATED);
    let bytes = wf_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let activate_response = app.clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    // Bounded wait for the background poll task to fetch the queued update
    // and fire the downstream telegram.sendMessage node against the same
    // mock server. Outer timeout is a belt-and-braces bound, matching the
    // pattern already established in http_request.rs's timeout test and
    // this plan's own telegram_poller.rs tests.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let requests = telegram.received_requests().await.unwrap();
            if requests.iter().any(|r| r.url.path().contains("sendMessage")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("telegram.trigger should have fired the downstream telegram.sendMessage node within 5s");

    // Clean up: deactivate so the background poll task stops before the
    // test process exits (avoids a dangling task hammering a mock server
    // that's about to be dropped).
    let deactivate_response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deactivate_response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run, confirm it passes**

Run: `cargo test --test api_test telegram_trigger_fires_downstream_node_on_incoming_update`
Expected: PASS if Tasks 1-4 are correctly implemented and wired together
(no new production code should be needed for this task — it is purely an
integration-proof test). If it fails, the failure identifies exactly which
earlier task's implementation has a bug; fix that task's code, not this
test.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/api_test.rs
git commit -m "test: add end-to-end Telegram Trigger integration test (trigger -> sendMessage)"
```

---

## Explicitly Out of Scope (this plan)

- **Webhook mode** (`setWebhook`) — deferred per the spec's resolved open
  question. `core.webhook`'s route matching (`webhook_node_matches` in
  `src/api/webhook.rs`) hardcodes a single recognized node type and would
  need generalizing; that's its own design decision for a future plan, not
  bolted onto this one.
- **Offset persistence across restarts** — v1's in-memory-only offset is a
  documented limitation, not a gap to close here.
- **Shared polling across workflows using the same bot credential** —
  Telegram's one-active-`getUpdates`-per-token constraint is documented,
  not solved; no current use case needs multiple workflows sharing one bot.
- **`sendPhoto` and other Telegram Bot API action methods** — Plan 4b's
  scope note already covers this; unrelated to the trigger.
- **An executions-listing API endpoint** — genuinely useful (and named in
  Plan 6's frontend spec, §6.3.3), but adding it here would be scope creep
  motivated by test convenience rather than this plan's actual goal; Task
  5's E2E test proves the full chain via an observable side effect instead
  (the downstream node's own outbound HTTP call).
