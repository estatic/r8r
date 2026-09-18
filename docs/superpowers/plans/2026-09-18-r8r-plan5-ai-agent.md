# r8r AI Agent Node (Plan 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An `ai.agent` node with multi-turn tool-calling against other
nodes in the same workflow (declared inline on the Agent's own
parameters, no canvas/connection changes) and a per-execution
conversational memory buffer, backed directly by the Anthropic Messages
API and an OpenAI-compatible Chat Completions API — no agent-framework
dependency.

**Architecture:** `NodeExecutionContext` gains a narrow `tool_executor`
field so the Agent node (and only the Agent node) can invoke another
registered node type without touching the `Node` trait or any of the 14
existing node implementations. A `ProviderClient` trait isolates the two
providers' wire-format differences into two small modules; the Agent
node's own loop is provider-agnostic and unit-tested against a test
double. Both new LLM credential types (`anthropicApi`/`openaiApi`) reuse
the existing free-form `credential_type` string and the existing
`parameters.auth.credential_id` resolution path — no storage, migration,
or frontend changes anywhere in this plan.

**Tech Stack:** Rust/Axum/reqwest backend (`src/`). New dependency:
`tiktoken-rs` (OpenAI-side local token counting only — Anthropic counts
via its own hosted endpoint, no new dependency needed there).

**Spec:** `docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`
— reachable, read directly before writing this plan; authoritative.

## Global Constraints

- **No frontend changes.** `ai.agent`'s parameters are edited through the
  existing raw-JSON-textarea mechanism; `auth.credential_id` is already
  picked up by the existing credential-picker dropdown; the credential
  creation form already accepts an arbitrary `credential_type` string and
  arbitrary JSON `data`. Nothing in `frontend/` changes in this plan.
- **`Node` trait and all 14 existing node implementations are
  unchanged.** Tool-calling access is added purely via a new field on
  `NodeExecutionContext`, never via a trait signature change.
- **No sub-workflow tools, no canvas-wired tools, no streaming, no
  cross-execution memory, no agent-to-agent tool calls** (a tool
  `node_type` of `"ai.agent"` is a hard config-time error). See spec §2/§11.
- **A tool execution failure never aborts the run** — it becomes a
  `ToolResult { is_error: true, .. }` fed back to the model. A provider
  HTTP failure (the LLM API itself unreachable) is a hard `NodeError` —
  the "let the model see and react" tolerance is specifically for tool
  execution, not provider connectivity. See spec §6/§9.
- **Exceeding `max_iterations` without a final text response is a hard
  `NodeError`** — never a silently truncated answer. See spec §6.
- **Never interpolate a raw API key into any `NodeError` message.** Both
  providers are header-based auth (not URL-embedded like Telegram's bot
  token), so the redirect/referer hardening `telegram_send_message.rs`
  needed doesn't apply here — but error messages still never explicitly
  include the resolved `api_key` value, as baseline hygiene.
- **Existing code patterns to follow exactly** (not re-derived — see
  `src/nodes/http_request.rs` and `src/nodes/telegram_send_message.rs`):
  the `OnceLock`-lazy shared `reqwest::Client` pattern; a thin `execute()`
  wrapper delegating to a testable `execute_with_...(...)` free function;
  raw `serde_json::Value` navigation via `.get()`/`.as_str()` for both
  request-building and response-parsing (this codebase defines no typed
  request/response structs anywhere — stay consistent, do not introduce
  `#[derive(Serialize, Deserialize)]` API structs for the provider
  clients); the `api_base_url` optional-override-with-default parameter
  pattern (needed for OpenAI's configurable base URL).

---

### Task 1: Registry Arc-threading prerequisite

**Files:**
- Modify: `src/engine.rs`
- Modify: `src/execution_runner.rs`

**Interfaces:**
- Produces: `execute_workflow_seeded`'s and `run_and_track_execution`'s
  `registry` parameter changes from `&NodeRegistry` to
  `&std::sync::Arc<NodeRegistry>`. No other signature changes. No
  production call site needs updating (see rationale below).
- Consumes: nothing new.

**Why this task exists:** Task 2 needs `EngineToolExecutor` (a new type
in `src/engine.rs`) to hold an *owned* `Arc<NodeRegistry>` so it can be
wrapped in `Arc<dyn ToolExecutor>` without adding a lifetime parameter to
`NodeExecutionContext` (which would otherwise force a signature change on
every one of the 14 existing `impl Node for ...` blocks' `execute()`
method — exactly what this plan must not do). `execute_workflow_seeded`
currently only has a *borrowed* `&NodeRegistry`, not an owned `Arc`, so
this task widens that borrow to `&Arc<NodeRegistry>` first — a plain
`Arc::clone()` inside the function is then all Task 2 needs.

Every production caller of `run_and_track_execution`
(`src/api/workflows.rs`, `src/api/webhook.rs`,
`src/triggers.rs::fire_schedule`, `src/telegram_poller.rs`) already holds
an `Arc<NodeRegistry>` at the point it calls it (`AppState.registry`, or
an owned `registry: Arc<NodeRegistry>` parameter) and already passes
`&state.registry` / `&registry` — which today auto-derefs from
`&Arc<NodeRegistry>` down to `&NodeRegistry` to match the current
parameter type. Once the parameter type becomes `&Arc<NodeRegistry>`
itself, those exact same argument expressions keep compiling with **zero
changes**, since they were already `&Arc<NodeRegistry>` all along — only
the function signatures narrow back to accepting what's already there.
`registry.get(...)` inside both functions' bodies also needs no change —
`Arc<T>` derefs transparently through method calls.

- [ ] **Step 1: Change `execute_workflow_seeded`'s and `execute_workflow`'s registry parameter**

In `src/engine.rs`, change:

```rust
pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None, &HashMap::new(), &NoopObserver).await
}
```

to:

```rust
pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &std::sync::Arc<NodeRegistry>,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None, &HashMap::new(), &NoopObserver).await
}
```

and change:

```rust
pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &NodeRegistry,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<uuid::Uuid, serde_json::Value>,
    observer: &dyn ExecutionObserver,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
```

to:

```rust
pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &std::sync::Arc<NodeRegistry>,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<uuid::Uuid, serde_json::Value>,
    observer: &dyn ExecutionObserver,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
```

- [ ] **Step 2: Fix `execute_workflow_seeded`'s own test module**

In `src/engine.rs`'s `#[cfg(test)] mod tests` block, find:

```rust
    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r
    }
```

Change it to:

```rust
    fn registry() -> std::sync::Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        std::sync::Arc::new(r)
    }
```

Find `registry_with_failing_node`:

```rust
    fn registry_with_failing_node() -> NodeRegistry {
        let mut r = registry();
        r.register(Box::new(AlwaysFailsNode));
        r
    }
```

Change it to build independently instead of delegating to `registry()`
(you can't get `&mut` through a shared `Arc`, and `NodeRegistry` isn't
`Clone`):

```rust
    fn registry_with_failing_node() -> std::sync::Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r.register(Box::new(AlwaysFailsNode));
        std::sync::Arc::new(r)
    }
```

Every other test in this module calls `execute_workflow(&wf, &registry())`
or `execute_workflow_seeded(&wf, &registry(), ..., &observer)` — these do
**not** need any changes; `&registry()` now naturally produces
`&Arc<NodeRegistry>` instead of `&NodeRegistry`, matching the new
parameter type.

- [ ] **Step 3: Fix `execution_runner.rs`'s `registry` parameter and test helper**

In `src/execution_runner.rs`, change:

```rust
pub async fn run_and_track_execution(
    storage: &Arc<dyn Storage>,
    events: &broadcast::Sender<ExecutionEvent>,
    registry: &NodeRegistry,
    workflow: &Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<Execution> {
```

to:

```rust
pub async fn run_and_track_execution(
    storage: &Arc<dyn Storage>,
    events: &broadcast::Sender<ExecutionEvent>,
    registry: &Arc<NodeRegistry>,
    workflow: &Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<Execution> {
```

In its own `#[cfg(test)] mod tests` block, find:

```rust
    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r
    }
```

Change it to:

```rust
    fn registry() -> Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        Arc::new(r)
    }
```

Every test in this module calling `run_and_track_execution(&storage,
&events, &registry(), &wf, ...)` needs no changes — same reasoning as
Step 2.

- [ ] **Step 4: Run the full test suite to verify nothing else broke**

Run: `cargo build`
Expected: succeeds — this confirms every production call site
(`src/api/workflows.rs`, `src/api/webhook.rs`, `src/triggers.rs`,
`src/telegram_poller.rs`, `src/main.rs`) compiles unchanged.

Run: `cargo test`
Expected: all tests pass, same total as before this task (this is a pure
type-signature change with no behavior change — the suite's count and
results must be identical to the pre-task baseline; if anything fails,
the fix is in this task's steps, not a downstream one).

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs src/execution_runner.rs
git commit -m "refactor: thread Arc<NodeRegistry> through execute_workflow_seeded"
```

---

### Task 2: `ToolExecutor` trait, `NodeExecutionContext.tool_executor`, `EngineToolExecutor`

**Files:**
- Modify: `src/node.rs`
- Modify: `src/engine.rs`
- Modify: `src/nodes/http_request.rs`
- Modify: `src/nodes/telegram_send_message.rs`

**Interfaces:**
- Consumes: `Arc<NodeRegistry>` registry parameter (Task 1).
- Produces:
  ```rust
  // src/node.rs
  #[async_trait::async_trait]
  pub trait ToolExecutor: Send + Sync {
      async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<NodeOutput, NodeError>;
  }
  ```
  `NodeExecutionContext.tool_executor: Option<std::sync::Arc<dyn ToolExecutor>>`
  — populated with `Some(...)` for every node the engine runs (not just
  `ai.agent`), exactly like `credentials` already is.

- [ ] **Step 1: Write the failing test**

Add to `src/engine.rs`'s `#[cfg(test)] mod tests` block, after the
existing spy-observer tests (Plan 7.1's Task 1 pattern — a test double
recording calls):

```rust
    struct SpyToolExecutor {
        calls: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
    }

    impl SpyToolExecutor {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(Vec::new()) }
        }
    }

    #[async_trait::async_trait]
    impl ToolExecutor for SpyToolExecutor {
        async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            self.calls.lock().unwrap().push((node_type.to_string(), parameters.clone()));
            Ok(vec![vec![Item { json: serde_json::json!({"tool": "called"}), binary: serde_json::json!({}) }]])
        }
    }

    #[tokio::test]
    async fn every_node_receives_a_tool_executor_and_engine_tool_executor_dispatches_to_the_registry() {
        // Part A: prove every node's context carries a tool_executor (not
        // just some special-cased node type) by running a normal
        // single-node workflow and confirming it doesn't error just
        // because a tool_executor now exists in scope -- this is an
        // indirect check since core.set itself never calls it.
        let wf = linear_workflow(); // trigger -> set1
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));

        // Part B: prove EngineToolExecutor itself correctly dispatches to
        // a real registered node type via the registry.
        let tool_executor = EngineToolExecutor::new(registry(), HashMap::new());
        let output = tool_executor
            .call_tool("core.set", serde_json::json!({"fields": {"x": 1}}))
            .await
            .unwrap();
        assert_eq!(output[0][0].json, serde_json::json!({"x": 1}));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test --lib engine::tests::every_node_receives_a_tool_executor`
Expected: compile error — `EngineToolExecutor` doesn't exist yet, and
`ToolExecutor` isn't imported/defined.

- [ ] **Step 3: Add `ToolExecutor` and the `tool_executor` field to `NodeExecutionContext`**

In `src/node.rs`, `NodeExecutionContext` currently derives `Debug` —
adding an `Arc<dyn ToolExecutor>` field breaks that derive, since a trait
object only implements `Debug` if the trait itself requires it (`dyn
ToolExecutor` here doesn't). Replace the derive with a hand-written
`Debug` impl that prints a placeholder for the new field. Change:

```rust
#[derive(Debug, Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
}
```

to:

```rust
#[derive(Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
    pub tool_executor: Option<std::sync::Arc<dyn ToolExecutor>>,
}

impl std::fmt::Debug for NodeExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeExecutionContext")
            .field("parameters", &self.parameters)
            .field("input_items", &self.input_items)
            .field("credentials", &self.credentials)
            .field("tool_executor", &self.tool_executor.as_ref().map(|_| "<tool_executor>"))
            .finish()
    }
}

/// Lets a node (in practice, only `ai.agent`) invoke another registered
/// node type mid-execution -- the mechanism behind tool-calling. Kept
/// deliberately narrow (one method, no registry/workflow access exposed
/// directly) so every other node type is completely unaffected by its
/// existence; see `docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`
/// section 3 for the full rationale, including why this is `Arc<dyn
/// ToolExecutor>` (a 'static trait object) rather than a borrowed
/// reference: a borrowed reference would need a lifetime parameter on
/// `NodeExecutionContext` itself, which would then need to appear on
/// every `Node::execute()` signature across all existing node types.
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<NodeOutput, NodeError>;
}
```

`#[derive(Default)]` still works: `Option<T>` defaults to `None` for any
`T`, and `Arc<dyn ToolExecutor>` doesn't need to implement `Clone`,
`Debug`, or `Default` itself for `Option<Arc<dyn ToolExecutor>>` to have
those derives work — `Arc<T>` is always `Clone` regardless of `T`.

- [ ] **Step 4: Add `EngineToolExecutor` and wire it into `execute_workflow_seeded`**

In `src/engine.rs`, add (near the top, after the `NoopObserver` impl):

```rust
/// The real `ToolExecutor` used by every live execution: dispatches a
/// tool call to a registered node type via the same `NodeRegistry` the
/// main engine loop uses. A called tool's own context gets `tool_executor:
/// None` (see `call_tool` below) -- a tool can never itself call further
/// tools, which is what makes the `node_type != "ai.agent"` validation in
/// the Agent node's own parameter parsing (Task 5) sufficient to prevent
/// all agent-to-agent recursion without needing a depth counter here.
struct EngineToolExecutor {
    registry: std::sync::Arc<NodeRegistry>,
    credentials: HashMap<uuid::Uuid, serde_json::Value>,
}

impl EngineToolExecutor {
    fn new(registry: std::sync::Arc<NodeRegistry>, credentials: HashMap<uuid::Uuid, serde_json::Value>) -> Self {
        Self { registry, credentials }
    }
}

#[async_trait::async_trait]
impl crate::node::ToolExecutor for EngineToolExecutor {
    async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
        let node = self.registry.get(node_type).ok_or_else(|| {
            crate::node::NodeError::ExecutionFailed(format!("unknown tool node_type: {node_type}"))
        })?;
        let ctx = NodeExecutionContext {
            parameters,
            input_items: vec![],
            credentials: self.credentials.clone(),
            tool_executor: None,
        };
        node.execute(&ctx).await
    }
}
```

In `execute_workflow_seeded`'s body, construct one `EngineToolExecutor`
near the top (right after `let mut error_produced...`):

```rust
    let tool_executor: std::sync::Arc<dyn crate::node::ToolExecutor> =
        std::sync::Arc::new(EngineToolExecutor::new(registry.clone(), credentials.clone()));
```

Then find the per-node `NodeExecutionContext` construction:

```rust
        let ctx = NodeExecutionContext {
            parameters,
            input_items,
            credentials: credentials.clone(),
        };
```

and change it to:

```rust
        let ctx = NodeExecutionContext {
            parameters,
            input_items,
            credentials: credentials.clone(),
            tool_executor: Some(tool_executor.clone()),
        };
```

- [ ] **Step 5: Fix the 4 other `NodeExecutionContext` full-literal sites**

These four sites construct `NodeExecutionContext { ... }` without
`..Default::default()`, so they need an explicit `tool_executor: None`
field or they won't compile (verified exhaustively — every other
`NodeExecutionContext` literal in the codebase already uses
`..Default::default()` and needs no change).

In `src/nodes/http_request.rs`, in test
`bearer_auth_sends_authorization_header_from_resolved_credential`, find:

```rust
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": format!("{}/secure", server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
        };
```

Add `tool_executor: None,` after `credentials,`:

```rust
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": format!("{}/secure", server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
            tool_executor: None,
        };
```

In `src/nodes/telegram_send_message.rs`, the same fix applies to three
tests — in each, add `tool_executor: None,` as the last field in the
`NodeExecutionContext { ... }` literal:

- `sends_message_with_bot_token_in_url_and_returns_response`:
  ```rust
          let ctx = NodeExecutionContext {
              parameters: serde_json::json!({
                  "chat_id": "42",
                  "text": "hello",
                  "auth": {"credential_id": credential_id.to_string()},
                  "api_base_url": server.uri()
              }),
              input_items: vec![],
              credentials,
              tool_executor: None,
          };
  ```
- `telegram_ok_false_returns_error_with_description`:
  ```rust
          let ctx = NodeExecutionContext {
              parameters: serde_json::json!({
                  "chat_id": "999999",
                  "text": "hello",
                  "auth": {"credential_id": credential_id.to_string()},
                  "api_base_url": server.uri()
              }),
              input_items: vec![],
              credentials,
              tool_executor: None,
          };
  ```
- `network_failure_error_message_never_contains_bot_token`:
  ```rust
          let ctx = NodeExecutionContext {
              parameters: serde_json::json!({
                  "chat_id": "1",
                  "text": "hi",
                  "auth": {"credential_id": credential_id.to_string()},
                  "api_base_url": dead_uri
              }),
              input_items: vec![],
              credentials,
              tool_executor: None,
          };
  ```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib engine::tests::every_node_receives_a_tool_executor`
Expected: passes.

Run: `cargo build`
Expected: succeeds with no errors (confirms all 5 fixed sites compile and
no other `NodeExecutionContext` literal was missed).

Run: `cargo test`
Expected: all tests pass, same total as Task 1's baseline plus this
task's one new test.

- [ ] **Step 7: Commit**

```bash
git add src/node.rs src/engine.rs src/nodes/http_request.rs src/nodes/telegram_send_message.rs
git commit -m "feat: add ToolExecutor and wire it into every node's execution context"
```

---

### Task 3: `src/llm` shared types + Anthropic provider client

**Files:**
- Create: `src/llm/mod.rs`
- Create: `src/llm/anthropic.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  ```rust
  // src/llm/mod.rs
  pub mod anthropic;

  #[derive(Debug, Clone)]
  pub enum LlmMessage {
      User { content: String },
      Assistant { content: Option<String>, tool_calls: Vec<ToolCall> },
      ToolResult { tool_call_id: String, content: String, is_error: bool },
  }

  #[derive(Debug, Clone)]
  pub struct ToolCall {
      pub id: String,
      pub name: String,
      pub arguments: serde_json::Value,
  }

  #[derive(Debug, Clone)]
  pub struct ToolDefinition {
      pub name: String,
      pub description: String,
      pub parameters: serde_json::Value,
  }

  pub enum ProviderResponse {
      Text(String),
      ToolCalls(Vec<ToolCall>),
  }

  #[async_trait::async_trait]
  pub trait ProviderClient: Send + Sync {
      async fn send_message(
          &self,
          system_prompt: &str,
          messages: &[LlmMessage],
          tools: &[ToolDefinition],
          model: &str,
          api_key: &str,
      ) -> Result<ProviderResponse, crate::node::NodeError>;

      async fn count_tokens(
          &self,
          system_prompt: &str,
          messages: &[LlmMessage],
          model: &str,
          api_key: &str,
      ) -> Result<usize, crate::node::NodeError>;
  }
  ```
  `pub struct AnthropicClient` implementing `ProviderClient`.
- Consumes: nothing new from earlier tasks (this module is
  self-contained, mirroring `src/expr.rs`'s own "intentionally
  self-contained" convention).

- [ ] **Step 1: Write the failing tests**

Create `src/llm/anthropic.rs` with just its test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_message_returns_text_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello there"}],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let response = client
            .send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "Hello there"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_message_returns_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_2",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "toolu_1", "name": "fetch_weather", "input": {"city": "Warsaw"}}],
                "stop_reason": "tool_use"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "weather in Warsaw?".into() }];
        let tools = vec![ToolDefinition {
            name: "fetch_weather".into(),
            description: "Get weather".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        let response = client
            .send_message("You are helpful.", &messages, &tools, "claude-opus-5", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "fetch_weather");
                assert_eq!(calls[0].arguments, serde_json::json!({"city": "Warsaw"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn count_tokens_calls_the_hosted_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"input_tokens": 42})))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let count = client
            .count_tokens("You are helpful.", &messages, "claude-opus-5", "test-key")
            .await
            .unwrap();
        assert_eq!(count, 42);
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "type": "error",
                "error": {"type": "authentication_error", "message": "invalid x-api-key"}
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let result = client.send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key").await;
        assert!(matches!(result, Err(crate::node::NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn tool_use_and_tool_result_messages_round_trip_through_a_second_turn() {
        // Proves the assistant tool_calls / user tool_result translation
        // is wired correctly, not just the first-turn request.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_3",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "It's sunny."}],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![
            LlmMessage::User { content: "weather in Warsaw?".into() },
            LlmMessage::Assistant {
                content: None,
                tool_calls: vec![ToolCall { id: "toolu_1".into(), name: "fetch_weather".into(), arguments: serde_json::json!({"city": "Warsaw"}) }],
            },
            LlmMessage::ToolResult { tool_call_id: "toolu_1".into(), content: "{\"temp_c\": 22}".into(), is_error: false },
        ];
        let response = client.send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key").await.unwrap();
        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "It's sunny."),
            other => panic!("expected Text, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Create `src/llm/mod.rs` and run the tests to verify they fail**

Create `src/llm/mod.rs`:

```rust
//! LLM provider clients for the `ai.agent` node. Intentionally
//! self-contained (mirrors `src/expr.rs`'s own convention): depends only
//! on `serde_json`/`async-trait`/`reqwest`, not on `domain.rs` or
//! `node.rs`, except for reusing `crate::node::NodeError` as the error
//! type so provider-client failures compose naturally with every other
//! node's error handling.

pub mod anthropic;

#[derive(Debug, Clone)]
pub enum LlmMessage {
    User { content: String },
    Assistant { content: Option<String>, tool_calls: Vec<ToolCall> },
    ToolResult { tool_call_id: String, content: String, is_error: bool },
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// A JSON Schema object, passed close to verbatim into each
    /// provider's own tool-definition wrapper shape.
    pub parameters: serde_json::Value,
}

#[derive(Debug)]
pub enum ProviderResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

#[async_trait::async_trait]
pub trait ProviderClient: Send + Sync {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, crate::node::NodeError>;

    /// A real per-provider token count for the memory-budget trim -- not
    /// an approximation.
    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        api_key: &str,
    ) -> Result<usize, crate::node::NodeError>;
}
```

In `src/lib.rs`, add (alphabetically, between `expr` and `node`):

```rust
pub mod llm;
```

Run: `cargo test --lib llm::anthropic::tests`
Expected: compile error — `AnthropicClient` doesn't exist yet.

- [ ] **Step 3: Implement `AnthropicClient`**

At the top of `src/llm/anthropic.rs` (above the test module), add:

```rust
use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
use crate::node::NodeError;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TOKENS: u32 = 4096;

pub struct AnthropicClient {
    client: reqwest::Client,
    base_url: String,
}

impl AnthropicClient {
    pub fn new() -> Result<Self, NodeError> {
        Self::with_base_url_result(DEFAULT_BASE_URL.to_string())
    }

    /// Test-only constructor pointing at a wiremock server; panics on a
    /// client-build failure since a test that can't build a client can't
    /// proceed anyway -- matches this codebase's other test-only
    /// constructors' tolerance for a hard failure in test setup.
    #[cfg(test)]
    fn with_base_url(base_url: String) -> Self {
        Self::with_base_url_result(base_url).expect("failed to build reqwest client")
    }

    fn with_base_url_result(base_url: String) -> Result<Self, NodeError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| NodeError::ExecutionFailed(format!("failed to build Anthropic HTTP client: {e}")))?;
        Ok(Self { client, base_url })
    }

    fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
        match msg {
            LlmMessage::User { content } => serde_json::json!({"role": "user", "content": content}),
            LlmMessage::Assistant { content, tool_calls } => {
                let mut blocks = Vec::new();
                if let Some(text) = content {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                serde_json::json!({"role": "assistant", "content": blocks})
            }
            LlmMessage::ToolResult { tool_call_id, content, is_error } => serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error
                }]
            }),
        }
    }

    fn build_request_body(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        include_max_tokens: bool,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": model,
            "system": system_prompt,
            "messages": messages.iter().map(Self::message_to_json).collect::<Vec<_>>(),
        });
        if include_max_tokens {
            body["max_tokens"] = serde_json::json!(MAX_TOKENS);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools
                .iter()
                .map(|t| serde_json::json!({"name": t.name, "description": t.description, "input_schema": t.parameters}))
                .collect::<Vec<_>>());
        }
        body
    }

    async fn post(&self, path: &str, body: serde_json::Value, api_key: &str) -> Result<serde_json::Value, NodeError> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|_| NodeError::ExecutionFailed("ai.agent: request to Anthropic API failed".into()))?;

        let status = response.status();
        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("ai.agent: Anthropic API returned HTTP {status} with a non-JSON body")))?;

        if !status.is_success() {
            let message = response_json
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no message provided");
            return Err(NodeError::ExecutionFailed(format!("ai.agent: Anthropic API error: {message}")));
        }
        Ok(response_json)
    }
}

#[async_trait::async_trait]
impl ProviderClient for AnthropicClient {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, NodeError> {
        let body = self.build_request_body(system_prompt, messages, tools, model, true);
        let response_json = self.post("/v1/messages", body, api_key).await?;

        let content = response_json
            .get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: Anthropic response missing \"content\"".into()))?;

        let mut tool_calls = Vec::new();
        let mut text = String::new();
        for block in content {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    tool_calls.push(ToolCall {
                        id: block.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        name: block.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        arguments: block.get("input").cloned().unwrap_or(serde_json::json!({})),
                    });
                }
                _ => {}
            }
        }

        if !tool_calls.is_empty() {
            Ok(ProviderResponse::ToolCalls(tool_calls))
        } else {
            Ok(ProviderResponse::Text(text))
        }
    }

    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        api_key: &str,
    ) -> Result<usize, NodeError> {
        let body = self.build_request_body(system_prompt, messages, &[], model, false);
        let response_json = self.post("/v1/messages/count_tokens", body, api_key).await?;
        response_json
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: Anthropic count_tokens response missing \"input_tokens\"".into()))
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib llm::`
Expected: all 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/llm/mod.rs src/llm/anthropic.rs src/lib.rs
git commit -m "feat: add ProviderClient trait and Anthropic provider client"
```

---

### Task 4: OpenAI-compatible provider client

**Files:**
- Create: `src/llm/openai.rs`
- Modify: `src/llm/mod.rs`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `ProviderClient`, `LlmMessage`, `ToolCall`, `ToolDefinition`,
  `ProviderResponse` (Task 3).
- Produces: `pub struct OpenAiClient` implementing `ProviderClient`, with
  `OpenAiClient::new(base_url: Option<String>) -> Result<Self, NodeError>`
  (defaults to `https://api.openai.com/v1` when `None`).

- [ ] **Step 1: Add the `tiktoken-rs` dependency**

In `Cargo.toml`, add to `[dependencies]` (alphabetically, after `thiserror`):

```toml
tiktoken-rs = "0.6"
```

Run: `cargo build`
Expected: succeeds (confirms the dependency resolves; if `0.6` conflicts
with something already pinned, adjust to the nearest compatible release
and note the change — same allowance this project's other plans use for
version pins).

- [ ] **Step 2: Write the failing tests**

Create `src/llm/openai.rs` with just its test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_message_returns_text_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "Hello there", "tool_calls": null},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let response = client
            .send_message("You are helpful.", &messages, &[], "gpt-4o", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "Hello there"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_message_returns_tool_calls_with_parsed_arguments() {
        // OpenAI's function-call arguments arrive as a JSON-encoded
        // STRING, not a nested object -- this is the one real structural
        // difference from Anthropic worth its own test.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "fetch_weather", "arguments": "{\"city\": \"Warsaw\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "weather in Warsaw?".into() }];
        let tools = vec![ToolDefinition {
            name: "fetch_weather".into(),
            description: "Get weather".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        let response = client
            .send_message("You are helpful.", &messages, &tools, "gpt-4o", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "fetch_weather");
                assert_eq!(calls[0].arguments, serde_json::json!({"city": "Warsaw"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let result = client.send_message("You are helpful.", &messages, &[], "gpt-4o", "test-key").await;
        assert!(matches!(result, Err(crate::node::NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn count_tokens_computes_a_real_count_without_a_network_call() {
        let client = OpenAiClient::with_base_url("http://unreachable.invalid".into());
        let messages = vec![LlmMessage::User { content: "hello world, this is a test message".into() }];
        let count = client.count_tokens("You are helpful.", &messages, "gpt-4o", "test-key").await.unwrap();
        // Not asserting an exact number (tokenizer internals aren't this
        // test's concern) -- just that it's a real, nonzero, sane count
        // computed locally (the unreachable base_url above proves no
        // network call happened, since a real call would time out/fail).
        assert!(count > 0 && count < 100);
    }

    #[tokio::test]
    async fn defaults_to_the_real_openai_base_url_when_none_given() {
        let client = OpenAiClient::new(None).unwrap();
        assert_eq!(client.base_url, "https://api.openai.com/v1");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib llm::openai::tests`
Expected: compile error — `OpenAiClient` doesn't exist yet.

- [ ] **Step 4: Register the module**

In `src/llm/mod.rs`, add (alphabetically, before `anthropic` — wait,
`anthropic` < `openai` alphabetically, so add after it):

```rust
pub mod anthropic;
pub mod openai;
```

- [ ] **Step 5: Implement `OpenAiClient`**

At the top of `src/llm/openai.rs` (above the test module), add:

```rust
use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
use crate::node::NodeError;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiClient {
    client: reqwest::Client,
    base_url: String,
}

impl OpenAiClient {
    pub fn new(base_url: Option<String>) -> Result<Self, NodeError> {
        Self::with_base_url_result(base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()))
    }

    #[cfg(test)]
    fn with_base_url(base_url: String) -> Self {
        Self::with_base_url_result(base_url).expect("failed to build reqwest client")
    }

    fn with_base_url_result(base_url: String) -> Result<Self, NodeError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| NodeError::ExecutionFailed(format!("failed to build OpenAI HTTP client: {e}")))?;
        Ok(Self { client, base_url })
    }

    fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
        match msg {
            LlmMessage::User { content } => serde_json::json!({"role": "user", "content": content}),
            LlmMessage::Assistant { content, tool_calls } => {
                let mut obj = serde_json::json!({"role": "assistant", "content": content});
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = serde_json::json!(tool_calls
                        .iter()
                        .map(|call| serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": serde_json::to_string(&call.arguments).unwrap_or_default()
                            }
                        }))
                        .collect::<Vec<_>>());
                }
                obj
            }
            LlmMessage::ToolResult { tool_call_id, content, is_error: _ } => serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content
            }),
        }
    }

    fn build_messages_json(&self, system_prompt: &str, messages: &[LlmMessage]) -> Vec<serde_json::Value> {
        let mut out = vec![serde_json::json!({"role": "system", "content": system_prompt})];
        out.extend(messages.iter().map(Self::message_to_json));
        out
    }
}

#[async_trait::async_trait]
impl ProviderClient for OpenAiClient {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, NodeError> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": self.build_messages_json(system_prompt, messages),
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools
                .iter()
                .map(|t| serde_json::json!({
                    "type": "function",
                    "function": {"name": t.name, "description": t.description, "parameters": t.parameters}
                }))
                .collect::<Vec<_>>());
        }

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|_| NodeError::ExecutionFailed("ai.agent: request to OpenAI-compatible API failed".into()))?;

        let status = response.status();
        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("ai.agent: OpenAI-compatible API returned HTTP {status} with a non-JSON body")))?;

        if !status.is_success() {
            let message = response_json
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no message provided");
            return Err(NodeError::ExecutionFailed(format!("ai.agent: OpenAI-compatible API error: {message}")));
        }

        let message = response_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: OpenAI-compatible response missing choices[0].message".into()))?;

        if let Some(tool_calls_json) = message.get("tool_calls").and_then(|v| v.as_array()) {
            if !tool_calls_json.is_empty() {
                let tool_calls = tool_calls_json
                    .iter()
                    .map(|tc| {
                        let arguments_str = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        ToolCall {
                            id: tc.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                            name: tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            arguments: serde_json::from_str(arguments_str).unwrap_or(serde_json::json!({})),
                        }
                    })
                    .collect();
                return Ok(ProviderResponse::ToolCalls(tool_calls));
            }
        }

        let text = message.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok(ProviderResponse::Text(text))
    }

    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        _api_key: &str,
    ) -> Result<usize, NodeError> {
        // No hosted counting endpoint on this API shape -- compute
        // locally via tiktoken-rs, entirely offline, no network call.
        let bpe = tiktoken_rs::get_bpe_from_model(model).unwrap_or_else(|_| {
            tiktoken_rs::cl100k_base().expect("cl100k_base tokenizer must always be constructible")
        });
        let mut total = bpe.encode_with_special_tokens(system_prompt).len();
        for msg in messages {
            let text = match msg {
                LlmMessage::User { content } => content.clone(),
                LlmMessage::Assistant { content, tool_calls } => {
                    let mut s = content.clone().unwrap_or_default();
                    for call in tool_calls {
                        s.push_str(&call.name);
                        s.push_str(&call.arguments.to_string());
                    }
                    s
                }
                LlmMessage::ToolResult { content, .. } => content.clone(),
            };
            total += bpe.encode_with_special_tokens(&text).len();
        }
        Ok(total)
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib llm::openai::tests`
Expected: all 5 tests pass.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add src/llm/openai.rs src/llm/mod.rs Cargo.toml Cargo.lock
git commit -m "feat: add OpenAI-compatible provider client"
```

---

### Task 5: The Agent node's core tool-calling loop

**Files:**
- Create: `src/nodes/agent.rs`

**Interfaces:**
- Consumes: `ProviderClient`, `LlmMessage`, `ToolCall`, `ToolDefinition`,
  `ProviderResponse` (Tasks 3-4); `NodeExecutionContext.tool_executor`
  (Task 2).
- Produces:
  ```rust
  pub struct AgentNode;
  impl Node for AgentNode { fn type_name(&self) -> &'static str { "ai.agent" } ... }

  /// The testable core -- everything except real-provider construction
  /// (Task 6). Public within the crate so Task 6's integration test can
  /// exercise the same function real execution uses.
  pub(crate) async fn run_agent_loop(
      provider: &dyn crate::llm::ProviderClient,
      model: &str,
      api_key: &str,
      system_prompt: &str,
      user_message: String,
      tools_param: &serde_json::Value,
      max_iterations: u64,
      max_context_tokens: Option<usize>,
      ctx: &NodeExecutionContext,
  ) -> Result<NodeOutput, NodeError>;
  ```
  `max_context_tokens: None` skips memory trimming entirely (spec §7's
  budget is optional — "a sane per-provider default if unset"; this plan
  treats "no default configured" as "don't trim," leaving the provider's
  own API to enforce its real context limit, since a bogus made-up
  default would risk trimming more aggressively than actually necessary
  for whatever model the author picked). This mirrors `http_request.rs`'s
  `execute()`/`execute_with_client()`
  split: `AgentNode::execute()` (Task 6) parses `parameters.provider` and
  constructs the real client, then delegates here.

This task does **not** register `ai.agent` in `NodeRegistry` yet (Task 6
does, once real-provider selection exists) — `src/nodes/agent.rs` exists
and is fully unit-tested in this task, but isn't reachable through the
engine until Task 6.

- [ ] **Step 1: Write the failing tests**

Create `src/nodes/agent.rs` with just its test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
    use crate::node::{NodeError, NodeExecutionContext, NodeOutput, ToolExecutor};
    use std::sync::Mutex;

    /// A scripted provider: returns each entry in `responses` in order,
    /// one per `send_message` call. Panics if called more times than
    /// scripted -- a test asserting the loop's turn count wrong will fail
    /// loudly here rather than silently returning a stale response.
    struct ScriptedProvider {
        responses: Mutex<Vec<ProviderResponse>>,
    }

    #[async_trait::async_trait]
    impl ProviderClient for ScriptedProvider {
        async fn send_message(
            &self,
            _system_prompt: &str,
            _messages: &[LlmMessage],
            _tools: &[ToolDefinition],
            _model: &str,
            _api_key: &str,
        ) -> Result<ProviderResponse, NodeError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                panic!("ScriptedProvider called more times than scripted");
            }
            Ok(responses.remove(0))
        }

        async fn count_tokens(&self, _: &str, _: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
            Ok(0) // never trims in these tests -- token-budget behavior is a separate test
        }
    }

    /// Always returns the same tool result (or error), recording every
    /// call it received.
    struct SpyToolExecutor {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
        result: Result<NodeOutput, String>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for SpyToolExecutor {
        async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<NodeOutput, NodeError> {
            self.calls.lock().unwrap().push((node_type.to_string(), parameters));
            match &self.result {
                Ok(output) => Ok(output.clone()),
                Err(msg) => Err(NodeError::ExecutionFailed(msg.clone())),
            }
        }
    }

    fn ctx_with_tool_executor(executor: std::sync::Arc<dyn ToolExecutor>) -> NodeExecutionContext {
        NodeExecutionContext { tool_executor: Some(executor), ..Default::default() }
    }

    #[tokio::test]
    async fn a_direct_text_response_needs_no_tool_calls() {
        let provider = ScriptedProvider { responses: Mutex::new(vec![ProviderResponse::Text("Hi there!".into())]) };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![]]),
        });
        let ctx = ctx_with_tool_executor(spy.clone());

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "hi".into(), &serde_json::json!([]), 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Hi there!");
        assert_eq!(output[0][0].json["tool_calls_made"], 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_successful_tool_call_feeds_its_result_back_and_the_loop_continues() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls(vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({"x": 1}) }]),
                ProviderResponse::Text("Done.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![Item { json: serde_json::json!({"ok": true}), binary: serde_json::json!({}) }]]),
        });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]},
            "base_parameters": {"method": "GET"}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Done.");
        assert_eq!(output[0][0].json["tool_calls_made"], 1);
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls[0].0, "core.httpRequest");
        // base_parameters merged with the model's own arguments (arguments win).
        assert_eq!(calls[0].1, serde_json::json!({"method": "GET", "x": 1}));
    }

    #[tokio::test]
    async fn a_failing_tool_call_becomes_a_tool_result_and_the_run_continues() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls(vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }]),
                ProviderResponse::Text("I couldn't fetch that.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Err("upstream 500".into()),
        });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        // The run succeeds overall -- the tool failure was surfaced to
        // the model, not propagated as a hard NodeError.
        assert_eq!(output[0][0].json["response"], "I couldn't fetch that.");
    }

    #[tokio::test]
    async fn exceeding_max_iterations_without_a_final_response_is_a_hard_error() {
        let always_tool_calls = ProviderResponse::ToolCalls(vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }]);
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![always_tool_calls; 3]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![]]),
        });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let result = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 3, None, &ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn a_tool_declared_with_node_type_ai_agent_is_rejected_before_any_provider_call() {
        let provider = ScriptedProvider { responses: Mutex::new(vec![]) }; // never called
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "recurse", "description": "nope", "node_type": "ai.agent",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let result = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => assert!(msg.contains("ai.agent")),
            other => panic!("expected ExecutionFailed mentioning ai.agent, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_name_becomes_a_tool_result_not_a_hard_failure() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls(vec![ToolCall { id: "t1".into(), name: "does_not_exist".into(), arguments: serde_json::json!({}) }]),
                ProviderResponse::Text("Never mind.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([]); // "does_not_exist" isn't declared at all

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Never mind.");
        assert!(spy.calls.lock().unwrap().is_empty()); // never dispatched -- caught before ToolExecutor
    }

    #[tokio::test]
    async fn memory_is_trimmed_to_the_token_budget_before_each_turn() {
        // A provider whose count_tokens reflects the CURRENT message
        // count (10 "tokens" per message) so trimming is directly
        // observable: after each tool-calling round adds 2 messages
        // (an Assistant-with-tool_calls plus its ToolResult), the next
        // pre-turn check goes over a budget of 25, forcing that round's
        // 2 messages to be dropped before the turn proceeds.
        struct CountingProvider {
            responses: Mutex<Vec<ProviderResponse>>,
            counts_seen: Mutex<Vec<usize>>,
        }
        #[async_trait::async_trait]
        impl ProviderClient for CountingProvider {
            async fn send_message(&self, _: &str, _: &[LlmMessage], _: &[ToolDefinition], _: &str, _: &str) -> Result<ProviderResponse, NodeError> {
                Ok(self.responses.lock().unwrap().remove(0))
            }
            async fn count_tokens(&self, _: &str, messages: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
                let count = messages.len() * 10;
                self.counts_seen.lock().unwrap().push(count);
                Ok(count)
            }
        }

        let provider = CountingProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls(vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }]),
                ProviderResponse::ToolCalls(vec![ToolCall { id: "t2".into(), name: "fetch".into(), arguments: serde_json::json!({}) }]),
                ProviderResponse::Text("Done.".into()),
            ]),
            counts_seen: Mutex::new(Vec::new()),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]]),
        });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}}, "base_parameters": {}
        }]);

        run_agent_loop(&provider, "claude-opus-5", "key", "sys", "go".into(), &tools, 10, Some(25), &ctx)
            .await
            .unwrap();

        // messages starts at [user] (10 tokens, under 25 -- proceeds).
        // After round 1: [user, assistant, tool_result] = 30 tokens --
        // over budget on the next pre-turn check, so a trim must occur
        // before round 2's send_message. The trimmed count (back to just
        // [user] = 10) must appear as a later, lower reading -- proving
        // the trim loop actually re-checks after dropping, not just
        // once-and-gives-up.
        let counts = provider.counts_seen.lock().unwrap();
        assert!(counts.contains(&30), "expected an over-budget reading triggering a trim, got {counts:?}");
        assert_eq!(*counts.last().unwrap(), 10, "history must be back under budget by the final pre-turn check; got {counts:?}");
        assert!(counts.len() >= 4, "expected repeated count_tokens calls as trimming re-checks after each drop, got {counts:?}");
    }
}
```

- [ ] **Step 2: Register the (empty) module and run the tests to verify they fail**

In `src/nodes/mod.rs`, add (alphabetically, before `code`):

```rust
pub mod agent;
```

Do **not** add `registry.register(Box::new(agent::AgentNode))` to
`register_all` yet — that's Task 6, once real provider selection exists.

Run: `cargo test --lib nodes::agent::tests`
Expected: compile error — `run_agent_loop` doesn't exist yet.

- [ ] **Step 3: Implement `run_agent_loop`**

At the top of `src/nodes/agent.rs` (above the test module), add:

```rust
use crate::domain::Item;
use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct AgentNode;

/// One tool declared in `parameters.tools` -- see
/// `docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`
/// section 4.
struct ToolDecl {
    name: String,
    description: String,
    node_type: String,
    argument_schema: serde_json::Value,
    base_parameters: serde_json::Value,
}

fn parse_tools(tools_param: &serde_json::Value) -> Result<Vec<ToolDecl>, NodeError> {
    let array = tools_param.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut decls = Vec::new();
    for entry in array {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: each tool requires a \"name\"".into()))?
            .to_string();
        let node_type = entry
            .get("node_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: tool \"{name}\" requires a \"node_type\"")))?
            .to_string();
        if node_type == "ai.agent" {
            return Err(NodeError::ExecutionFailed(format!(
                "ai.agent: tool \"{name}\" declares node_type \"ai.agent\" -- agent-to-agent tool calls are not supported"
            )));
        }
        decls.push(ToolDecl {
            name,
            description: entry.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            node_type,
            argument_schema: entry.get("argument_schema").cloned().unwrap_or(serde_json::json!({})),
            base_parameters: entry.get("base_parameters").cloned().unwrap_or(serde_json::json!({})),
        });
    }
    Ok(decls)
}

/// Shallow validation: every property the schema marks `required` is
/// present, and each present property's JSON type matches the schema's
/// declared `type` (`"string"`, `"number"`/`"integer"`, `"boolean"`,
/// `"object"`, `"array"`). Not full JSON Schema (no `pattern`, no
/// `minimum`, no nested-schema checks) -- this exists only to catch an
/// obviously malformed tool call before it reaches a real node's own
/// `execute()`, which does its own validation regardless.
fn validate_arguments(schema: &serde_json::Value, arguments: &serde_json::Value) -> Result<(), String> {
    let required = schema.get("required").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for req in &required {
        let Some(key) = req.as_str() else { continue };
        if arguments.get(key).is_none() {
            return Err(format!("missing required argument \"{key}\""));
        }
    }
    let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) else { return Ok(()) };
    for (key, prop_schema) in properties {
        let Some(value) = arguments.get(key) else { continue };
        let Some(expected_type) = prop_schema.get("type").and_then(|v| v.as_str()) else { continue };
        let matches = match expected_type {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            _ => true, // unknown declared type -- don't block on it
        };
        if !matches {
            return Err(format!("argument \"{key}\" does not match declared type \"{expected_type}\""));
        }
    }
    Ok(())
}

fn item_to_tool_result_content(output: &NodeOutput) -> String {
    let primary = output.first().cloned().unwrap_or_default();
    let json_values: Vec<serde_json::Value> = primary.iter().map(|item| item.json.clone()).collect();
    serde_json::to_string(&json_values).unwrap_or_else(|_| "[]".to_string())
}

/// Drops the oldest whole "turn unit" (an `Assistant` tool-calls message
/// plus every `ToolResult` immediately following it) from the front of
/// `messages`, repeatedly, until `count_tokens` reports a value at or
/// under `budget` -- or until only the original seed message (index 0,
/// the very first `User` message) remains, which is never dropped:
/// removing it would leave the loop with no seed input at all, a
/// correctness failure worse than staying over budget (the provider's
/// own API still enforces the real limit either way -- see spec section
/// 7). Given `run_agent_loop`'s own message-building logic, index 1 (when
/// present) is always an `Assistant` tool-calls message -- it is the only
/// kind ever pushed there, immediately followed by that turn's
/// `ToolResult`(s), which is what this function relies on.
async fn trim_to_budget(
    provider: &dyn ProviderClient,
    system_prompt: &str,
    messages: &mut Vec<LlmMessage>,
    model: &str,
    api_key: &str,
    max_context_tokens: Option<usize>,
) -> Result<(), NodeError> {
    let Some(budget) = max_context_tokens else { return Ok(()) };
    loop {
        let count = provider.count_tokens(system_prompt, messages, model, api_key).await?;
        if count <= budget || messages.len() <= 1 {
            return Ok(());
        }
        let mut remove_count = 1; // the Assistant message itself, at index 1
        while messages.get(1 + remove_count).map(|m| matches!(m, LlmMessage::ToolResult { .. })).unwrap_or(false) {
            remove_count += 1;
        }
        messages.drain(1..1 + remove_count);
    }
}

pub(crate) async fn run_agent_loop(
    provider: &dyn ProviderClient,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    user_message: String,
    tools_param: &serde_json::Value,
    max_iterations: u64,
    max_context_tokens: Option<usize>,
    ctx: &NodeExecutionContext,
) -> Result<NodeOutput, NodeError> {
    let tool_decls = parse_tools(tools_param)?;
    let tool_definitions: Vec<ToolDefinition> = tool_decls
        .iter()
        .map(|t| ToolDefinition { name: t.name.clone(), description: t.description.clone(), parameters: t.argument_schema.clone() })
        .collect();

    let tool_executor = ctx
        .tool_executor
        .as_ref()
        .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: no tool_executor available in this execution context".into()))?;

    let mut messages = vec![LlmMessage::User { content: user_message }];
    let mut tool_calls_made: u64 = 0;

    for _ in 0..max_iterations {
        trim_to_budget(provider, system_prompt, &mut messages, model, api_key, max_context_tokens).await?;
        match provider.send_message(system_prompt, &messages, &tool_definitions, model, api_key).await? {
            ProviderResponse::Text(text) => {
                return Ok(vec![vec![Item {
                    json: serde_json::json!({"response": text, "tool_calls_made": tool_calls_made}),
                    binary: serde_json::json!({}),
                }]]);
            }
            ProviderResponse::ToolCalls(calls) => {
                messages.push(LlmMessage::Assistant { content: None, tool_calls: calls.clone() });
                for call in &calls {
                    tool_calls_made += 1;
                    let Some(decl) = tool_decls.iter().find(|t| t.name == call.name) else {
                        messages.push(LlmMessage::ToolResult {
                            tool_call_id: call.id.clone(),
                            content: format!("unknown tool: \"{}\"", call.name),
                            is_error: true,
                        });
                        continue;
                    };
                    if let Err(reason) = validate_arguments(&decl.argument_schema, &call.arguments) {
                        messages.push(LlmMessage::ToolResult { tool_call_id: call.id.clone(), content: reason, is_error: true });
                        continue;
                    }
                    let mut merged_parameters = decl.base_parameters.clone();
                    if let (Some(merged_obj), Some(args_obj)) = (merged_parameters.as_object_mut(), call.arguments.as_object()) {
                        for (k, v) in args_obj {
                            merged_obj.insert(k.clone(), v.clone());
                        }
                    }
                    match tool_executor.call_tool(&decl.node_type, merged_parameters).await {
                        Ok(output) => {
                            messages.push(LlmMessage::ToolResult {
                                tool_call_id: call.id.clone(),
                                content: item_to_tool_result_content(&output),
                                is_error: false,
                            });
                        }
                        Err(e) => {
                            messages.push(LlmMessage::ToolResult { tool_call_id: call.id.clone(), content: e.to_string(), is_error: true });
                        }
                    }
                }
            }
        }
    }

    Err(NodeError::ExecutionFailed("ai.agent: exceeded max_iterations without a final response".into()))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test --lib nodes::agent::tests`
Expected: all 7 tests pass.

Run: `cargo build`
Expected: succeeds (note `AgentNode` and the `Node` impl for it don't
exist yet — that's fine, `run_agent_loop` and its test module don't need
them; Task 6 adds the `Node` impl and wires `AgentNode` for real).

- [ ] **Step 5: Commit**

```bash
git add src/nodes/agent.rs src/nodes/mod.rs
git commit -m "feat: add ai.agent's tool-calling loop core"
```

---

### Task 6: Real provider wiring, registration, end-to-end test

**Files:**
- Modify: `src/nodes/agent.rs`
- Modify: `src/nodes/mod.rs`
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: `run_agent_loop` (Task 5), `AnthropicClient` (Task 3),
  `OpenAiClient` (Task 4).
- Produces: `AgentNode` registered as `"ai.agent"` in `NodeRegistry`, a
  real, working end-to-end node.

- [ ] **Step 1: Write the failing integration test**

Add to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn agent_node_calls_a_tool_then_returns_a_final_response_end_to_end() {
    let anthropic_server = wiremock::MockServer::start().await;
    let tool_server = wiremock::MockServer::start().await;

    // Turn 1: Anthropic responds with a tool_use call to core.httpRequest.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/messages"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "tool_use", "id": "toolu_1", "name": "fetch_status", "input": {}}],
                    "stop_reason": "tool_use"
                }))
                .append_header("content-type", "application/json"),
        )
        .up_to_n_times(1)
        .mount(&anthropic_server)
        .await;

    // Turn 2: Anthropic responds with the final text answer.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/messages"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_2",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "The status is ok."}],
            "stop_reason": "end_turn"
        })))
        .mount(&anthropic_server)
        .await;

    // The tool itself: core.httpRequest hitting a second mocked endpoint.
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/status"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})))
        .mount(&tool_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "agent-e2e@example.com").await;

    let create_cred_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "test-anthropic", "credential_type": "anthropicApi", "data": {"api_key": "test-key"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let workflow_body = serde_json::json!({
        "name": "agent-e2e",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {
                "id": "agent1",
                "node_type": "ai.agent",
                "position": [1.0, 0.0],
                "parameters": {
                    "provider": "anthropic",
                    "model": "claude-opus-5",
                    "system_prompt": "You are a status-checking assistant.",
                    "user_message": "What is the status?",
                    "auth": {"credential_id": credential_id},
                    "api_base_url": anthropic_server.uri(),
                    "max_iterations": 5,
                    "tools": [{
                        "name": "fetch_status",
                        "description": "Fetch the current status",
                        "node_type": "core.httpRequest",
                        "argument_schema": {"type": "object", "properties": {}},
                        "base_parameters": {"method": "GET", "url": format!("{}/status", tool_server.uri())}
                    }]
                },
                "disabled": false
            }
        ],
        "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "agent1", "to_input": 0}]
    });
    let create_response = app
        .clone()
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
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let response = app
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
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["agent1"][0]["json"]["response"], "The status is ok.");
    assert_eq!(execution["node_outputs"]["agent1"][0]["json"]["tool_calls_made"], 1);
}
```

The test above already includes `"api_base_url": anthropic_server.uri()`
in `agent1`'s parameters — Step 3 below makes `AgentNode::execute()`
respect it for the Anthropic branch (optional, `None` falls through to
`AnthropicClient::new()`'s real default), the same
optional-override-with-default pattern `telegram.trigger`/
`telegram.sendMessage` already use for their own `api_base_url`. Without
Step 3, this test would otherwise call the real
`https://api.anthropic.com`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test agent_node_calls_a_tool_then_returns_a_final_response`
Expected: fails with `unknown node type: ai.agent` (not yet registered)
or a 400/500 from the execute endpoint — confirms the test reaches real
execution and fails for the expected reason, not a request-building typo.

- [ ] **Step 3: Wire real provider selection in `AgentNode::execute()`**

In `src/nodes/agent.rs`, add (below `run_agent_loop`, above the test
module):

```rust
#[async_trait]
impl Node for AgentNode {
    fn type_name(&self) -> &'static str {
        "ai.agent"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let provider_name = ctx
            .parameters
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent requires a \"provider\" parameter (\"anthropic\" or \"openai\")".into()))?;
        let model = ctx
            .parameters
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent requires a \"model\" parameter".into()))?;
        let system_prompt = ctx.parameters.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("");
        let user_message = ctx
            .parameters
            .get("user_message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent requires a \"user_message\" parameter".into()))?
            .to_string();
        let max_iterations = ctx.parameters.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(10);
        let max_context_tokens = ctx.parameters.get("max_context_tokens").and_then(|v| v.as_u64()).map(|n| n as usize);
        let empty_tools = serde_json::json!([]);
        let tools_param = ctx.parameters.get("tools").unwrap_or(&empty_tools);

        let credential_id_str = ctx
            .parameters
            .get("auth")
            .and_then(|a| a.get("credential_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent requires auth.credential_id".into()))?;
        let credential_id = uuid::Uuid::parse_str(credential_id_str)
            .map_err(|e| NodeError::ExecutionFailed(format!("ai.agent: invalid credential_id: {e}")))?;
        let credential_data = ctx
            .credentials
            .get(&credential_id)
            .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: credential {credential_id} was not resolved for this run")))?;
        let api_key = credential_data
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: credential is missing \"api_key\"".into()))?;

        match provider_name {
            "anthropic" => {
                let base_url = ctx.parameters.get("api_base_url").and_then(|v| v.as_str());
                let client = match base_url {
                    Some(url) => crate::llm::anthropic::AnthropicClient::with_base_url_for_node(url.to_string())?,
                    None => crate::llm::anthropic::AnthropicClient::new()?,
                };
                run_agent_loop(&client, model, api_key, system_prompt, user_message, tools_param, max_iterations, max_context_tokens, ctx).await
            }
            "openai" => {
                let base_url = credential_data.get("base_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                let client = crate::llm::openai::OpenAiClient::new(base_url)?;
                run_agent_loop(&client, model, api_key, system_prompt, user_message, tools_param, max_iterations, max_context_tokens, ctx).await
            }
            other => Err(NodeError::ExecutionFailed(format!("ai.agent: unknown provider \"{other}\" (expected \"anthropic\" or \"openai\")"))),
        }
    }
}
```

This calls `AnthropicClient::with_base_url_for_node`, a *non-test*
constructor Task 3 didn't add (it only added the `#[cfg(test)]`-gated
`with_base_url`). Go back to `src/llm/anthropic.rs` and add, next to the
existing `new()`:

```rust
    /// Non-test override for a custom base URL (e.g. a node-level
    /// `api_base_url` parameter pointing at a mock or a compatible
    /// self-hosted endpoint) -- unlike `with_base_url` (test-only, panics
    /// on a build failure), this returns a proper `Result` since it's
    /// reachable from real node execution.
    pub fn with_base_url_for_node(base_url: String) -> Result<Self, NodeError> {
        Self::with_base_url_result(base_url)
    }
```

- [ ] **Step 4: Register `ai.agent`**

In `src/nodes/mod.rs`, add to `register_all` (anywhere among the other
`registry.register(...)` lines):

```rust
    registry.register(Box::new(agent::AgentNode));
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test agent_node_calls_a_tool_then_returns_a_final_response`
Expected: passes.

- [ ] **Step 6: Run the full test suite**

Run: `cargo test`
Expected: all tests pass — the new total is the pre-Plan-5 baseline plus
this plan's new tests (Task 2: +1, Task 3: +5, Task 4: +5, Task 5: +7,
Task 6: +1 = 19 new tests).

- [ ] **Step 7: Commit**

```bash
git add src/nodes/agent.rs src/nodes/mod.rs src/llm/anthropic.rs tests/api_test.rs
git commit -m "feat: wire real provider selection and register ai.agent"
```
