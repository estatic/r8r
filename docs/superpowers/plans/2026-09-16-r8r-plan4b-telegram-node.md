# r8r Plan 4b — Telegram Action Node — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `telegram.sendMessage`, r8r's first third-party integration node, built on the same credential-authenticated HTTP pattern Plan 4a established — and a short "adding a new node" developer guide using it as the worked example, per the spec's own framing of Telegram as that reference integration.

**Architecture:** `TelegramSendMessageNode` is its own `Node` impl (not a literal call into `HttpRequestNode` — nodes don't call other nodes in this engine) that follows the same shape Plan 4a's HTTP Request node established: a lazily-initialized, timeout-configured, reused `reqwest::Client`; credential lookup via `ctx.credentials` (never touching `Storage`/`crypto` itself); loud failure on a missing/unresolved credential. It differs from the generic HTTP Request node in one structurally important way: Telegram's Bot API embeds the bot token directly in the request URL (`https://api.telegram.org/bot<TOKEN>/sendMessage`), not in a header — which means, unlike `core.httpRequest` (where the URL never contains a secret), this node's URL itself IS sensitive, and `reqwest::Error`'s `Display` output can include the request URL it was building/sending. Every error path in this node is written to never interpolate the raw `reqwest::Error` or the constructed URL — only a generic, non-URL description.

**Tech Stack:** Rust, `reqwest` (already a dependency from Plan 4a), `wiremock` (already a dev-dependency), the existing credential/engine stack (unchanged).

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md` (§4 Data Model — Credential's `"telegramApi"` example; §5.3 Messaging)
**Roadmap reference:** `docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`, Plan 4 §4.3.2–4.3.3 (§4.3.1, the Telegram Trigger, is explicitly deferred — see Explicitly Out of Scope)

## Global Constraints

- `TelegramSendMessageNode`, `type_name() == "telegram.sendMessage"`. Parameters: `{"auth": {"credential_id": "<uuid string>"}, "chat_id": "<string or number, expression-resolved>", "text": "<string, expression-resolved>", "api_base_url": "<string, optional>"}`. `api_base_url` defaults to `"https://api.telegram.org"` when absent — real users never set it; it exists purely so tests can point the node at a `wiremock` server instead of the real Telegram API. This mirrors how `core.httpRequest`'s `url` parameter is already fully test-injectable; Telegram's URL isn't user-supplied (only the token portion varies), so an override parameter is the equivalent seam.
- Credential shape convention (matches the spec's own `"telegramApi"` example verbatim): `credential_type: "telegramApi"`, `data: {"bot_token": "<string>"}`. No new domain type is needed — `Credential.data` is already a generic `serde_json::Value`; this is purely a documented convention for what a `telegramApi`-typed credential's `data` should contain, enforced only by this node's own parsing (an `Err` if `bot_token` is missing, same as any other required-field check).
- The node reads `ctx.credentials` (already-decrypted, populated by `resolve_credentials_for_workflow` from Plan 4a) directly — it never touches `Storage`/`crypto`. An `auth.credential_id` not present in `ctx.credentials` is a loud `Err`, never a silent unauthenticated request — identical discipline to `core.httpRequest`'s `apply_auth`.
- **Security requirement specific to this node**: because the bot token is embedded in the request URL (not a header), NO error message this node produces may interpolate the raw `reqwest::Error` (via `{e}`/`.to_string()`) or the constructed request URL. Use fixed, generic descriptions for network/request-building failures (e.g. `"telegram.sendMessage: request to Telegram API failed"`) instead. This is a real divergence from `core.httpRequest`'s pattern (which safely does interpolate `{e}`, since its URL is never secret) — do not copy that node's error-handling pattern here without adjusting for this.
- Response handling: Telegram's Bot API returns a JSON body shaped `{"ok": true, "result": {...}}` on success or `{"ok": false, "error_code": <int>, "description": "<string>"}` on failure — check BOTH the HTTP status (non-2xx → `Err`) AND the parsed body's `ok` field (`false` → `Err`, using `description` in the error message if present — Telegram's own `description` field is developer-facing API documentation text, e.g. "Bad Request: chat not found", never secret material, safe to include). On success, the single output item's `json` is the full parsed response body (matching `core.httpRequest`'s convention of returning the whole response, not just `result`).
- Own, separate `reqwest::Client` (own `OnceLock`, same 30-second-timeout pattern as `core.httpRequest`) rather than sharing `core.httpRequest`'s static client — this is a deliberate simplicity choice (two small connection pools instead of one shared one) to avoid refactoring Plan 4a's already-shipped, already-reviewed `src/nodes/http_request.rs`. Not worth extracting a shared helper module for two call sites; reconsider if a third HTTP-based node arrives later.
- The developer-docs deliverable is a real file (`docs/adding-a-node.md`), not a stub — it must reference this node's actual file/structure as the worked example, written for a developer who has never touched this codebase before.
- No Telegram Trigger (long-polling or webhook mode), no `sendPhoto`/other Bot API methods — both explicitly out of scope, see below.

---

## File Structure

- `src/nodes/telegram_send_message.rs` — new. `TelegramSendMessageNode`.
- `src/nodes/mod.rs` — modify. Register the new node.
- `docs/adding-a-node.md` — new. Developer guide, using `telegram_send_message.rs` as the worked example.
- `tests/api_test.rs` — modify. One new end-to-end test: create a `telegramApi` credential, create+execute a workflow (Manual Trigger → `telegram.sendMessage`, `api_base_url` pointed at a `wiremock` server) through the real HTTP API.

---

### Task 1: `TelegramSendMessageNode`

**Files:**
- Create: `src/nodes/telegram_send_message.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::telegram_send_message::TelegramSendMessageNode`, `type_name() == "telegram.sendMessage"`. Parameters and credential shape per Global Constraints. Single output port on success (the full parsed Telegram API response). `Err(NodeError::ExecutionFailed(..))` on: missing `chat_id`/`text`, unresolved `credential_id`, missing `bot_token` in the credential's `data`, a non-2xx HTTP response, or `ok: false` in the parsed body — none of these error paths may interpolate the raw `reqwest::Error` or the constructed URL.

- [ ] **Step 1: Write failing tests**

```rust
// src/nodes/telegram_send_message.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

pub struct TelegramSendMessageNode;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_API_BASE_URL: &str = "https://api.telegram.org";

fn http_client() -> Result<&'static reqwest::Client, NodeError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|_| NodeError::ExecutionFailed("telegram.sendMessage: failed to build HTTP client".into()))
}

#[async_trait]
impl Node for TelegramSendMessageNode {
    fn type_name(&self) -> &'static str {
        "telegram.sendMessage"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        execute_with_client(http_client()?, ctx).await
    }
}

async fn execute_with_client(
    client: &reqwest::Client,
    ctx: &NodeExecutionContext,
) -> Result<NodeOutput, NodeError> {
    let chat_id = ctx
        .parameters
        .get("chat_id")
        .ok_or_else(|| NodeError::ExecutionFailed("telegram.sendMessage requires a \"chat_id\" parameter".into()))?
        .clone();
    let text = ctx
        .parameters
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("telegram.sendMessage requires a \"text\" parameter".into()))?;

    let credential_id_str = ctx
        .parameters
        .get("auth")
        .and_then(|a| a.get("credential_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("telegram.sendMessage requires auth.credential_id".into()))?;
    let credential_id = Uuid::parse_str(credential_id_str)
        .map_err(|e| NodeError::ExecutionFailed(format!("invalid credential_id: {e}")))?;
    let credential_data = ctx
        .credentials
        .get(&credential_id)
        .ok_or_else(|| NodeError::ExecutionFailed(format!("credential {credential_id} was not resolved for this run")))?;
    let bot_token = credential_data
        .get("bot_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("telegramApi credential missing \"bot_token\"".into()))?;

    let base_url = ctx
        .parameters
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_API_BASE_URL);
    let url = format!("{base_url}/bot{bot_token}/sendMessage");

    // Never interpolate `e` (the reqwest::Error) or `url` into any error
    // message below -- both can carry the bot token embedded in `url`,
    // unlike core.httpRequest where the URL is never secret. See this
    // plan's Global Constraints.
    let response = client
        .post(&url)
        .json(&serde_json::json!({"chat_id": chat_id, "text": text}))
        .send()
        .await
        .map_err(|_| NodeError::ExecutionFailed("telegram.sendMessage: request to Telegram API failed".into()))?;

    let status = response.status();
    let response_json: serde_json::Value = response
        .json()
        .await
        .map_err(|_| NodeError::ExecutionFailed("telegram.sendMessage: failed to parse Telegram API response".into()))?;

    let ok = response_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !status.is_success() || !ok {
        let description = response_json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("no description provided");
        return Err(NodeError::ExecutionFailed(format!(
            "telegram.sendMessage: Telegram API returned an error: {description}"
        )));
    }

    Ok(vec![vec![Item { json: response_json, binary: serde_json::json!({}) }]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sends_message_with_bot_token_in_url_and_returns_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendMessage$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 42}
            })))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "123:ABC"}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "42",
                "text": "hello",
                "auth": {"credential_id": credential_id.to_string()},
                "api_base_url": server.uri()
            }),
            input_items: vec![],
            credentials,
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json["result"]["message_id"], 42);
    }

    #[tokio::test]
    async fn telegram_ok_false_returns_error_with_description() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendMessage$"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "ok": false,
                "error_code": 400,
                "description": "Bad Request: chat not found"
            })))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "123:ABC"}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "999999",
                "text": "hello",
                "auth": {"credential_id": credential_id.to_string()},
                "api_base_url": server.uri()
            }),
            input_items: vec![],
            credentials,
        };
        let result = node.execute(&ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => assert!(msg.contains("chat not found")),
            other => panic!("expected ExecutionFailed with description, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_chat_id_returns_error() {
        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"text": "hello"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn unresolved_credential_returns_error() {
        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "1",
                "text": "hi",
                "auth": {"credential_id": Uuid::new_v4().to_string()}
            }),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn network_failure_error_message_never_contains_bot_token() {
        // Point at a server that isn't listening (a MockServer we start then
        // immediately drop, freeing its port) to provoke a real
        // connection-level reqwest::Error, and confirm the bot token never
        // appears anywhere in the resulting error message -- proving the
        // "never interpolate the raw reqwest::Error or URL" discipline
        // actually holds, not just that it's written that way.
        let server = MockServer::start().await;
        let dead_uri = server.uri();
        drop(server);

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "SECRET-BOT-TOKEN-VALUE"}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "1",
                "text": "hi",
                "auth": {"credential_id": credential_id.to_string()},
                "api_base_url": dead_uri
            }),
            input_items: vec![],
            credentials,
        };
        let result = node.execute(&ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => {
                assert!(!msg.contains("SECRET-BOT-TOKEN-VALUE"));
            }
            other => panic!("expected a connection-level ExecutionFailed, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::telegram_send_message`
Expected: compile error — module not wired into `mod.rs` yet.

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod telegram_send_message;
// (add alongside the existing pub mod lines, keep alphabetical)
```

In `register_all`, add:

```rust
registry.register(Box::new(telegram_send_message::TelegramSendMessageNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::telegram_send_message`
Expected: all five PASS. Pay particular attention to `network_failure_error_message_never_contains_bot_token` — this is the test that actually proves the security-critical property in this plan's Global Constraints, not just that the code is written to look safe.

- [ ] **Step 5: Run the whole crate**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/nodes/telegram_send_message.rs src/nodes/mod.rs
git commit -m "feat: add Telegram sendMessage node (telegram.sendMessage)"
```

---

### Task 2: Developer docs — "adding a new node" guide

**Files:**
- Create: `docs/adding-a-node.md`

**Interfaces:**
- Produces: a real, complete guide (not a stub) walking a developer with zero prior context through adding a new node to r8r, using `src/nodes/telegram_send_message.rs` (Task 1, already merged) as the worked example throughout.

- [ ] **Step 1: Write the guide**

Write `docs/adding-a-node.md` covering, in order, with concrete code references (file paths and line-level pointers, not just prose):

1. **The `Node` trait** — `type_name()` and `execute()`, pointing at `src/node.rs`'s actual trait definition. Explain `NodeExecutionContext` (`parameters`, `input_items`, `credentials`) and `NodeOutput` (`Vec<Vec<Item>>`, one `Vec<Item>` per output port).
2. **A minimal single-output-port node** — walk through `telegram_send_message.rs`'s actual structure: parameter parsing (required fields → `Err` on missing, matching the pattern every node in this codebase uses), the single `Ok(vec![vec![Item { .. }]])` return on success.
3. **Error handling conventions** — `NodeError::ExecutionFailed`, and how a node's `Err` interacts with the engine's per-node error-output routing (a connected error output receives it; otherwise the whole run aborts) — no new code needed to get this, it's automatic once `execute()` returns `Err`.
4. **Using credentials** — how `ctx.credentials` gets populated (via `resolve_credentials_for_workflow`, scanning `parameters.auth.credential_id`), and the security discipline this plan's `telegram_send_message.rs` demonstrates: never let a raw external-library error or a URL/string containing a secret flow into a `NodeError` message unexamined — actually trace where secrets could leak (a URL, a header, a log line) for the specific integration being built.
5. **Registering the node** — the two-line addition to `src/nodes/mod.rs` (`pub mod` + `registry.register(...)`).
6. **Testing** — the `wiremock`-based pattern (`MockServer::start()`, `Mock::given(...).respond_with(...).mount(...)`), and why a hardcoded external API's node needs a test-injectable base URL parameter (as `telegram_send_message.rs`'s `api_base_url` demonstrates) while a fully-generic node like `core.httpRequest` doesn't need one (its `url` parameter is already caller-supplied).
7. A short closing checklist (parameter validation, error handling, credential security if applicable, tests, registration).

- [ ] **Step 2: Self-review**

Read the guide once more as if you were the developer with zero context it's written for: does every code reference actually match what's in `src/nodes/telegram_send_message.rs` and `src/node.rs` at this point in the repo's history? Fix any drift before committing.

- [ ] **Step 3: Commit**

```bash
git add docs/adding-a-node.md
git commit -m "docs: add developer guide for adding a new node, using Telegram as the worked example"
```

---

### Task 3: End-to-end integration test — credential-authenticated Telegram send through the real API

**Files:**
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: everything above, plus the existing `test_app()`/`register_and_get_token()` helpers.
- Produces: one new test proving the whole plan works end-to-end: create a `telegramApi` credential via `POST /rest/credentials`, create a workflow (Manual Trigger → `telegram.sendMessage`, `api_base_url` pointed at a `wiremock` server that asserts the bot token appears correctly in the request PATH) via `POST /rest/workflows`, execute via `POST /rest/workflows/:id/execute`, assert success and the node's output reflects the mocked Telegram response.

- [ ] **Step 1: Write the failing test**

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn credential_authenticated_telegram_send_message_executes_end_to_end() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot987:XYZ/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {"message_id": 7, "text": "hello from r8r"}
        })))
        .mount(&mock_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "telegram-e2e@example.com").await;

    let cred_response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "my-bot", "credential_type": "telegramApi", "data": {"bot_token": "987:XYZ"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cred_response.status(), StatusCode::CREATED);
    let bytes = cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let credential_id = credential["id"].as_str().unwrap();

    let workflow_body = serde_json::json!({
        "name": "telegram-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "send", "node_type": "telegram.sendMessage", "position": [1.0, 0.0], "parameters": {
                "chat_id": "42",
                "text": "hello from r8r",
                "auth": {"credential_id": credential_id},
                "api_base_url": mock_server.uri()
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "send", "to_input": 0}
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
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let exec_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exec_response.status(), StatusCode::OK);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["send"][0]["json"]["result"]["message_id"], 7);
}
```

If `tests/api_test.rs` doesn't already import `wiremock::matchers::{method, path}` / `wiremock::{Mock, MockServer, ResponseTemplate}` at the top level (Plan 4a's Task 10 added a similar capstone test to this same file, so these imports likely already exist — check before adding a duplicate `use`), add whatever's missing.

- [ ] **Step 2: Run, confirm it passes**

Run: `cargo test --test api_test credential_authenticated_telegram_send_message_executes_end_to_end`
Expected: PASS if Tasks 1-2 are correctly implemented and wired together (no new production code should be needed for this task — it is purely an integration-proof test). If it fails, the failure identifies exactly which earlier task's implementation has a bug; fix that task's code, not this test.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/api_test.rs
git commit -m "test: add end-to-end credential-authenticated Telegram sendMessage integration test"
```

---

## Explicitly Out of Scope (this plan)

Carried forward as future work per the roadmap breakdown:
- Telegram Trigger, both long-polling and webhook modes (roadmap §4.3.1) — long-polling needs new background-task infrastructure (a continuously-polling process analogous to, but structurally different from, `Scheduler`'s interval-based cron jobs) that doesn't exist anywhere in this codebase yet; webhook mode needs a decision about how a Telegram-specific trigger interoperates with `core.webhook`'s existing single-node-type-per-route model. Both deserve their own dedicated plan rather than being bolted onto this one.
- `sendPhoto` and any other Telegram Bot API method beyond `sendMessage` — add via the exact same pattern `telegram_send_message.rs` demonstrates (the developer-docs guide this plan produces exists specifically to make that addition straightforward).
- AI Agent tool integration with Telegram — Plan 5's job.
