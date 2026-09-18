# r8r AI Agent Node (Plan 5) Design Spec

## 1. Summary

An LLM-backed `ai.agent` node with multi-turn tool-calling (other nodes in
the same workflow, called directly, not via canvas wiring) and a
per-execution conversational memory buffer. Implements Anthropic's
Messages API and an OpenAI-compatible Chat Completions client directly —
no LangChain or other agent-framework dependency, per this project's
lightweight-footprint goal (spec `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md`
§4 AI section, §12's flagged open question).

This closes roadmap Plan 5 in full (`docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`
§5) — the last unbuilt major feature on the roadmap; every other phase
(Plans 2/3/4/6, 7.1) is already shipped.

## 2. Goals / Non-Goals

**Goals:**
- A workflow author can add an `ai.agent` node, give it a system prompt
  and a user-message template, pick a provider (Anthropic or an
  OpenAI-compatible API) via a credential, and declare a set of *tools* —
  other node types in the same workflow the model can choose to invoke —
  and get back a final text response after the model has (optionally)
  made one or more tool calls to gather information or take action.
- Tool declaration and invocation work without any change to the
  Connection model or the frontend canvas — tools are inline JSON on the
  Agent node's own parameters, resolved to real node execution via a new,
  narrow `ToolExecutor` interface that every other node type is
  unaffected by.
- A tool call that fails doesn't abort the run — the model sees the
  failure as a tool result and can react to it (retry differently, or
  give up and explain), matching how tool-calling models are designed to
  be used.
- The loop is bounded (a configurable max-iteration cap) and the memory
  buffer is bounded (a real per-provider token count, trimmed to a
  budget) — no unbounded growth in either dimension.
- Both providers ship in v1 (the user's explicit call — see the
  brainstorm's provider-scope question) via one `ProviderClient` trait,
  so wire-format differences between Anthropic's Messages API and an
  OpenAI-compatible Chat Completions API are isolated to two modules, not
  spread through the node/loop logic.

**Non-Goals:**
- Sub-workflow-as-tool (invoking a *different* workflow by id as a tool).
  A genuinely separate capability — cross-workflow invocation, id
  resolution, its own recursion/cycle question — deferred to its own
  future plan once same-workflow tool-calling ships and real usage shows
  it's needed. Mirrors this project's existing pattern of deferring
  Telegram webhook mode the same way.
- Canvas-wired tools (a tool node connected to the Agent via a new
  connection kind, n8n-style). Would require a new `Connection` semantic
  and a per-node parameter-schema system neither of which exist today —
  inline JSON tool declarations sidestep both.
- Streaming provider responses. r8r's execution model is fully
  synchronous (one HTTP response per run;
  `POST /rest/workflows/:id/execute` blocks until the whole workflow
  finishes) and there is no token-level UI surface — Plan 7.1's
  WebSocket only streams node-level start/finish/error events, not
  individual LLM tokens. Nothing would consume a stream.
- Long-term, cross-execution Agent memory (explicitly out of scope in the
  original design spec §10).
- Agent-to-agent tool calls of any kind — see §4's recursion guard, which
  rejects `ai.agent` as a valid tool `node_type` outright, closing off
  all agent-to-agent invocation (not just literal self-reference) rather
  than building a depth-limit mechanism.
- A general per-node parameter-schema system. Each tool's `argument_schema`
  (§5) is authored by hand on the Agent node itself, describing only what
  shape of arguments the model should produce for that one tool — it does
  not describe the underlying node's parameters in general, and no other
  node gains any schema metadata from this plan.
- Full JSON Schema validation of a tool call's arguments against its
  declared `argument_schema`. See §6 for the validation this plan does do.

## 3. Tool-Calling Mechanism

`Node` and every existing node implementation are unchanged. `NodeExecutionContext`
(`src/node.rs`) gains one new field:

```rust
#[derive(Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
    pub tool_executor: Option<std::sync::Arc<dyn ToolExecutor>>,
}

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<NodeOutput, NodeError>;
}
```

`execute_workflow_seeded` (`src/engine.rs`) constructs one concrete
`ToolExecutor` impl — call it `EngineToolExecutor`, holding `&NodeRegistry`
and the run's already-resolved `&HashMap<Uuid, Value>` credentials (both
already in scope in that function) — once, at the top, wrapped in an
`Arc`. It's included in *every* node's `NodeExecutionContext` unconditionally
(`tool_executor: Some(tool_executor.clone())`), exactly like `credentials`
is already cloned into every node's context today whether or not that
node type uses it. Only `ai.agent`'s own `execute()` will ever read this
field; the other 14 node types simply never touch it — no branching on
node type needed in the engine loop.

`EngineToolExecutor::call_tool` builds a fresh `NodeExecutionContext`
(`parameters` = the merged tool arguments from §5, `input_items: vec![]`,
`credentials` = the same resolved map, `tool_executor` = `None` — a
called tool cannot itself call further tools, which is also what makes
the `node_type != "ai.agent"` guard in §4 sufficient rather than needing
a depth counter), looks up the node via `registry.get(node_type)`, and
calls its `execute()`.

## 4. Tool Declaration

Inline JSON on the Agent node's own `parameters.tools`:

```json
{
  "tools": [
    {
      "name": "fetch_weather",
      "description": "Get the current weather for a city",
      "node_type": "core.httpRequest",
      "argument_schema": {
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"]
      },
      "base_parameters": {
        "method": "GET",
        "url": "https://api.example.com/weather"
      }
    }
  ]
}
```

`ai.agent` uses the *default* `Node::resolves_parameters() == true` (no
override, unlike `core.code`) — so like every other node's structured
parameters (e.g. `core.switch`'s `cases`, `core.merge`'s config),
`parameters.tools` already gets walked by the engine's standard
`expr::resolve_parameters` pass before `execute()` ever sees it, exactly
like `system_prompt`/`user_message` do. This is normal, not a special
case: a tool's `base_parameters.url` legitimately templating
`{{ $json.base_url }}` is a reasonable and supported thing to write.

At call time (inside `execute()`, once per tool call the model makes),
the model's *own* produced arguments (validated per §6 — already-concrete
JSON values the model returned, never containing `{{ }}` syntax to
resolve) are merged **over** the (already expression-resolved)
`base_parameters` — arguments win on key collision — to produce the tool
node's actual execution `parameters`, then dispatched via
`ToolExecutor::call_tool(node_type, merged_parameters)`. This merge step
itself is plain object merging, not expression resolution — there's
nothing left to resolve by the time it runs.

**Recursion guard:** before making any provider call, `ai.agent`'s
`execute()` validates every declared tool's `node_type` — if any equals
`"ai.agent"`, it's a hard `NodeError` (config-time failure, not a runtime
one). This closes off *all* agent-to-agent tool calls, not merely literal
self-reference, and is the plan's entire recursion story — no depth
counter needed (§3 already ensures a called tool's own context has no
`tool_executor`, so even without this guard a tool node's `execute()`
could never itself make a further tool call; the guard exists purely to
give a clear config-time error instead of a confusing "tool_executor is
None" runtime failure the moment an author points a tool at `ai.agent`).

No other `node_type` restriction exists. Pointing a tool at a trigger
node (`core.webhook`, `core.schedule`, `telegram.trigger`) or a
control-flow node (`core.if`, `core.switch`, `core.merge`) is allowed but
unlikely to be useful — those node types' `execute()` implementations are
either no-ops in this context or depend on the engine's own connection
graph, which a tool call bypasses entirely. Undocumented, caveat-emptor,
matching this codebase's existing trust-the-workflow-author posture
elsewhere (e.g. nothing stops a workflow from creating a real cycle
short of the topological-order check catching it at execution time).

## 5. Providers

```rust
// src/llm/mod.rs
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
    /// A JSON Schema object — passed through close to verbatim into each
    /// provider's own tool-definition wrapper shape (Anthropic:
    /// `{name, description, input_schema}`; OpenAI-compatible:
    /// `{type: "function", function: {name, description, parameters}}`).
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

    /// Real per-provider token count for the memory-budget trim in §7 —
    /// not an approximation.
    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        api_key: &str,
    ) -> Result<usize, crate::node::NodeError>;
}
```

- **`src/llm/anthropic.rs`** — `AnthropicClient`. `send_message` calls
  Anthropic's Messages API (`POST /v1/messages`), translating
  `LlmMessage`/`ToolDefinition` into its `messages`/`tools` shape and its
  `tool_use`/`tool_result` content-block convention. `count_tokens` calls
  Anthropic's own `POST /v1/messages/count_tokens` endpoint with the same
  payload shape minus `max_tokens` — a real hosted counting endpoint, so
  this needs no bundled tokenizer at all, matching the "call provider
  APIs directly" approach already used everywhere else in this codebase.
- **`src/llm/openai.rs`** — `OpenAiClient`. `send_message` calls an
  OpenAI-compatible Chat Completions endpoint (`POST /chat/completions`,
  base URL configurable per credential for self-hosted/local-model
  compatibility, mirroring `telegram.trigger`'s existing `api_base_url`
  parameter pattern), translating into `messages`/`tool_calls`.
  `count_tokens` has no hosted equivalent, so this uses a small
  tiktoken-compatible Rust crate (e.g. `tiktoken-rs`) — the one new
  dependency this plan adds, scoped to exactly this one counting need,
  not a general tokenization framework.
- Both clients build their own shared `reqwest::Client` via the same
  `OnceLock`-lazy-static pattern `HttpRequestNode` already uses (`src/nodes/http_request.rs`),
  with the same request-timeout discipline (a fixed timeout, since manual
  execution runs synchronously inside the Axum handler and an
  unresponsive provider must not hang the request forever).
- **Provider selection** comes from a new `credential_type`: `"anthropicApi"`
  (`data: {"api_key": "..."}`) or `"openaiApi"` (`data: {"api_key": "...", "base_url": "..."}`,
  `base_url` optional, defaulting to `https://api.openai.com/v1`).
  Resolved through the exact same `auth.credential_id` mechanism every
  other credentialed node already uses (`src/credentials.rs`'s
  `resolve_credentials_for_workflow` already walks every node's
  `parameters.auth.credential_id` with no changes needed) — the Agent
  node's own `parameters.provider` (`"anthropic"` | `"openai"`) picks
  which `ProviderClient` impl to construct; the resolved credential
  supplies the API key (and, for OpenAI, the base URL).

## 6. The Tool-Calling Loop

`src/nodes/agent.rs`'s `AgentNode::execute()`:

1. Parse `parameters.tools` (§4), validating the recursion guard.
2. Build the initial `LlmMessage::User` from `parameters.user_message`
   (an expression-resolved string parameter, templated from
   `ctx.input_items` the same way e.g. `telegram.sendMessage`'s `text`
   parameter already is — see §4 for why `parameters.tools` is resolved
   the same way, not treated as opaque data).
3. Loop, up to `parameters.max_iterations` (default `10`) turns:
   - Call `ProviderClient::send_message` with the system prompt, the
     accumulated message history, and the declared `ToolDefinition`s.
   - `ProviderResponse::Text(text)` → done; this is the node's output (§8).
   - `ProviderResponse::ToolCalls(calls)` → for each call: look up its
     tool definition by `name` (unknown name → a `ToolResult` with
     `is_error: true` and an explanatory message fed back to the model,
     not an immediate hard failure — the model gets a chance to correct
     itself, same philosophy as a real tool execution error below);
     shallow-validate the call's `arguments` against the tool's
     `argument_schema` (checks presence of `required` fields and each
     present field's `type` against the schema's declared JSON type —
     not full JSON Schema, e.g. no `pattern`/`minimum`/nested-schema
     validation; deliberately minimal, since its only job is catching an
     obviously malformed tool call before it reaches a real node's
     `execute()`, not replacing that node's own parameter validation);
     merge over `base_parameters` and call `ToolExecutor::call_tool`.
     A tool execution's `Err(NodeError)` becomes a `ToolResult` with
     `is_error: true` and the error's message — the run continues, the
     model sees the failure. A tool execution's `Ok(output)` becomes a
     `ToolResult` with `is_error: false` and `output`'s primary (port 0)
     item(s), JSON-serialized as the result content (mirroring how the
     engine's own final flatten step already takes only a node's primary
     output). Append the assistant's tool-call message and every
     resulting `ToolResult` message to history, then loop.
   - Before each turn, trim the message history (§7) to the configured
     token budget via `ProviderClient::count_tokens`.
4. Exceeding `max_iterations` without a `Text` response is a hard
   `NodeError::ExecutionFailed("agent exceeded max_iterations without a final response")` —
   fails loudly, matching this codebase's existing philosophy (e.g.
   credential-resolution failures) rather than silently returning a
   truncated/partial answer.

## 7. Memory

A per-execution `Vec<LlmMessage>`, live only for the duration of one
`AgentNode::execute()` call — never persisted, never visible across
separate executions (roadmap/original-spec's explicit v1 scope). Before
each turn, if `count_tokens(system_prompt, messages, model, api_key)`
exceeds a configured `parameters.max_context_tokens` (a sane per-provider
default if unset — e.g. a large fraction of the model's typical context
window, left as a plain node parameter an author can lower), the oldest
non-system messages are dropped from the front of the history (in whole
request/response-turn units — never split a tool call from its matching
tool result) until back under budget. This bounds the loop's own context
growth across many tool-calling turns; it is not attempting to precisely
fit a specific model's exact context window down to the last token — the
provider's own API still enforces the real limit and returns a clear
error if this trim were ever insufficient.

## 8. Agent Node Output & Registration

- **`src/nodes/agent.rs`**: `AgentNode`, `type_name() -> "ai.agent"`,
  registered in `src/nodes/mod.rs::register_all` alongside the existing
  14 node types.
- Final `NodeOutput` on success: `vec![vec![Item { json: json!({"response": <final text>, "tool_calls_made": <count>}), binary: json!({}) }]]` —
  a single primary-port item, matching the single-Item-output convention
  most existing action nodes already follow.
- Parameters (full shape): `provider` (`"anthropic"` | `"openai"`),
  `model` (string, e.g. `"claude-opus-5"` or `"gpt-4o"`), `system_prompt`
  (string), `user_message` (string, expression-resolved), `auth`
  (`{"credential_id": "<uuid>"}`), `max_iterations` (number, default
  `10`), `max_context_tokens` (number, optional), `tools` (array, §4).

**No frontend changes.** `ai.agent`'s parameters are edited through the
same raw-JSON-textarea mechanism every node without a first-class UI
already uses (Plan 6a §Global Constraints), and its `auth.credential_id`
field is picked up automatically by the credential-picker dropdown
already special-cased on that exact path (Plan 6a Task 9) — the same
mechanism `core.httpRequest`/`telegram.sendMessage` already use, with
nothing Agent-specific to add. The new `anthropicApi`/`openaiApi`
credential types need no frontend changes either: the credential-creation
form already accepts an arbitrary `credential_type` string and an
arbitrary JSON `data` payload.

## 9. Error Handling

- A malformed/missing required top-level parameter (`provider`, `model`,
  `auth.credential_id`) is a `NodeError::ExecutionFailed` before any
  provider call is made — mirrors `HttpRequestNode`'s missing-`url`
  handling.
- An unresolved `credential_id` (present in `auth` but not in
  `ctx.credentials`) is a `NodeError::ExecutionFailed` — mirrors
  `HttpRequestNode`'s identical check.
- A provider HTTP failure (timeout, non-2xx, malformed response body) is
  a `NodeError::ExecutionFailed` with a descriptive (but credential-safe —
  never includes the API key) message, aborting the whole node — a
  provider being unreachable isn't something the tool-calling loop's
  "let the model see and react to errors" philosophy applies to; that
  philosophy is specifically for *tool execution* failures the model
  might plausibly work around (§6), not "the LLM API itself is down."
- Every tool-execution and unknown-tool-name failure is caught and fed
  back to the model as a `ToolResult { is_error: true, .. }` (§6) rather
  than aborting — the one deliberate departure from
  "fail loudly," because it's the behavior tool-calling models are
  designed to work with.

## 10. Testing

- **Provider clients** (`src/llm/anthropic.rs`, `src/llm/openai.rs`):
  `wiremock`-based tests mirroring `HttpRequestNode`'s and
  `telegram_send_message.rs`'s existing pattern — a mocked provider
  endpoint returning a canned `tool_use`/`tool_calls` response and a
  canned final-text response, asserting the client correctly translates
  both directions (request shape sent, response shape parsed) and that
  `count_tokens` calls the right endpoint (Anthropic) or computes a real
  count via the tokenizer crate (OpenAI) without a network call.
- **The tool-calling loop** (`src/nodes/agent.rs`): unit tests against a
  test-double `ProviderClient` (scripted to return a `ToolCalls` response
  then a `Text` response, or to always return `ToolCalls` to prove the
  `max_iterations` cap fires) and a test-double `ToolExecutor` (a spy
  recording calls, returning canned `Ok`/`Err` results) — proving the
  loop's control flow (turn limit, error-becomes-tool-result, recursion
  guard rejecting `ai.agent` as a tool `node_type`) without needing a
  real provider or a real target node.
- **End-to-end** (`tests/api_test.rs`): one test wiring a real
  `EngineToolExecutor` (real registry, real engine) against a wiremocked
  Anthropic endpoint that first returns a `tool_use` call to
  `core.httpRequest` (itself hitting a second wiremocked endpoint) and
  then a final text response, asserting the persisted `Execution`'s
  output reflects both the tool call having actually run a real node and
  the model's final response — proving the whole chain end-to-end, the
  same "wiremock the external HTTP boundary, exercise everything else
  for real" pattern this codebase's existing credential-authenticated
  integration tests (HTTP Request, Telegram) already use.

## 11. Out of Scope

See §2's Non-Goals. Restated here for the plan-writing handoff: no
sub-workflow tools, no canvas-wired tools, no streaming, no
cross-execution memory, no agent-to-agent tool calls, no general
node-parameter-schema system, no full JSON Schema validation of tool
arguments.
