# Adding a new node to r8r

This guide walks through everything needed to add a new node type to r8r, using
the real, merged `telegram.sendMessage` node (`src/nodes/telegram_send_message.rs`)
as the worked example throughout. By the end you should be able to add your own
node end-to-end: trait impl, parameter parsing, error handling, credentials (if
needed), registration, and tests.

## 1. The `Node` trait

Every node type implements the `Node` trait defined in `src/node.rs:52-71`:

```rust
#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;

    fn resolves_parameters(&self) -> bool {
        true
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError>;
}
```

- **`type_name()`** returns the string identifier used to look the node up in
  the registry and to reference it from workflow JSON (e.g. `"telegram.sendMessage"`,
  `"core.httpRequest"`). This is a plain method, not `const`, but every real node
  implements it as a one-line literal return.
- **`resolves_parameters()`** has a default (`true`) that is correct for nearly
  every node — see the doc comment at `src/node.rs:56-65`. It controls whether
  the engine runs the node's `parameters` through the expression engine
  (`expr::resolve_parameters`, `{{ }}` interpolation) before calling `execute()`.
  Override it to `false` only when a "parameter" is really verbatim source/text
  that must not be touched (the doc comment's example is `core.code`'s `script`
  field). `telegram_send_message.rs` does not override it, so its parameters go
  through expression resolution like most nodes.
- **`execute()`** does the actual work and is the method you'll spend most of
  your time implementing.

### Node-type metadata

Five more methods, added for the node-type metadata system
(`docs/superpowers/specs/2026-09-21-r8r-node-type-metadata-design.md`):

- **`display_name()`**, **`description()`**, **`category()`** — no default,
  every node type must implement these three. `display_name` is what the
  canvas and add-node menu show instead of the raw `type_name()` string;
  `description` is one sentence shown in the add-node menu; `category` is
  one of `NodeCategory::{Trigger, Action, FlowControl, Ai}`.
- **`icon()`** — defaults to `"⚙️"`; override with a single emoji specific
  to your node.
- **`credential_types()`** — defaults to `&[]`. If your node reads
  `parameters.auth.credential_id` (see §4 below), declare the credential
  type string(s) it expects, e.g. `&["telegramApi"]` — this drives the
  credential picker's type dropdown. Leave it at the default `&[]` if your
  node accepts any credential shape (like `core.httpRequest`, which picks
  the shape from its own `auth.type` parameter) or needs no credential at
  all — the picker treats an empty list as "don't filter", not "don't
  allow".
- **`output_ports()`** — takes `parameters: &serde_json::Value` and
  defaults to `vec!["main".to_string()]`, correct for any node with a
  single, fixed output port. Override it only if your node has more than
  one output port (like `core.if`'s `["true", "false"]`) or a
  data-dependent port count (like `core.switch`, which reads its own
  `cases` parameter the same way `execute()` does — see
  `src/nodes/switch.rs` for the pattern).

Every one of these except `output_ports()` is a small, fixed addition —
copy the shape from any existing node in `src/nodes/` (e.g.
`src/nodes/telegram_send_message.rs`) rather than writing it from scratch.

### `NodeExecutionContext`

Defined at `src/node.rs:5-11`:

```rust
#[derive(Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
    pub tool_executor: Option<std::sync::Arc<dyn ToolExecutor>>,
}
```

Note the derive is only `Clone, Default` — `Debug` is hand-written separately
(`src/node.rs:13-22`) rather than derived, because `tool_executor`'s type
(`Option<Arc<dyn ToolExecutor>>`) doesn't implement `Debug` on its own; the
hand-written impl prints a placeholder (`"<tool_executor>"`) for that field
instead of the trait object itself.

- **`parameters`** — the node's configured parameters as a `serde_json::Value`
  (already expression-resolved, unless `resolves_parameters()` was overridden
  to `false`). Nodes read fields off this with `.get("field_name")`.
- **`input_items`** — the `Vec<Item>` fed into this node from the workflow's
  upstream connections. `Item` (`src/domain.rs:36-40`) has a `json` field and a
  `binary` field (the latter defaults via `#[serde(default)]`).
- **`credentials`** — a map from credential UUID to the credential's decrypted
  data (`serde_json::Value`), pre-populated for the whole run before any node
  executes. See §4 below for how this gets filled in and how to use it.
- **`tool_executor`** — an optional handle letting a node invoke another
  registered node type mid-execution (used only by `ai.agent`, the mechanism
  behind its tool-calling loop; see `src/node.rs:24-30` for the full rationale).
  Every other node type ignores this field entirely. When constructing a
  `NodeExecutionContext` in a test, pass `tool_executor: None` explicitly or
  use `..Default::default()`, which sets it to `None` for you.

The struct derives `Default`, so tests that don't need credentials (or a
`tool_executor`) can build a context with `..Default::default()` — every test
in `telegram_send_message.rs` that doesn't set `credentials` explicitly uses
this pattern (e.g. `missing_chat_id_returns_error` at
`src/nodes/telegram_send_message.rs:200-210`).

### `NodeOutput`

```rust
pub type NodeOutput = Vec<Vec<Item>>;
```

(`src/node.rs:45`.) The outer `Vec` is indexed by **output port**; the inner
`Vec<Item>` is the items flowing out of that port. A node with a single output
port returns `Ok(vec![items])` — one element in the outer vec. A node with
multiple output ports (e.g. `if_node.rs`'s true/false branches) returns
`Ok(vec![true_items, false_items])`.

## 2. A minimal single-output-port node

`telegram_send_message.rs` is a good template for a node that calls an external
API and produces exactly one output port. Its shape:

```rust
pub struct TelegramSendMessageNode;

#[async_trait]
impl Node for TelegramSendMessageNode {
    fn type_name(&self) -> &'static str {
        "telegram.sendMessage"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        execute_with_client(http_client()?, ctx).await
    }
}
```

(`src/nodes/telegram_send_message.rs:8, 35-44`.) The node struct itself is a
zero-field marker (`pub struct TelegramSendMessageNode;`) — all the real logic
lives in a free function, `execute_with_client`, that takes the HTTP client and
context as parameters. Telegram's own tests don't call `execute_with_client`
directly — they go through `node.execute()` and redirect requests via the
`api_base_url` parameter instead (see below). The split is kept anyway for
structural consistency with `http_request.rs`'s pattern, and to keep
`execute_with_client` unit-testable in principle against a hand-built client
(see §6 for a node that actually does this).

### Parameter parsing: required fields → `Err` on missing

Every node in this codebase follows the same pattern for a required parameter:
`.get("field")` (optionally chained with `.and_then(|v| v.as_str())` for typed
extraction), then `.ok_or_else(|| NodeError::ExecutionFailed("..."))?`. From
`execute_with_client` (`src/nodes/telegram_send_message.rs:62-71`):

```rust
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
```

Note `chat_id` is kept as a raw `serde_json::Value` (`.clone()`, no
`.and_then(as_str)`) because Telegram's API accepts either a numeric or string
chat ID and the node just forwards whatever was given; `text` is specifically
coerced to `&str` because it's interpolated into a JSON body. Match the
extraction to what the field actually needs to be, not a blanket convention.

`core.httpRequest` uses the identical pattern for its own required `url`
parameter (`src/nodes/http_request.rs:55-59`), and the same `.ok_or_else(||
NodeError::ExecutionFailed(...))?` idiom for every other required lookup in
that file (e.g. `apply_auth`'s `credential_id` extraction at
`src/nodes/http_request.rs:117-120`). This is the codebase-wide convention:
required parameter missing or wrong shape → `NodeError::ExecutionFailed` with a
message naming the node type and the missing field, propagated with `?`.

### The success return

On success, a single-output-port node returns one output port containing one
item — `Ok(vec![vec![Item { .. }]])`:

```rust
Ok(vec![vec![Item { json: response_json, binary: serde_json::json!({}) }]])
```

(`src/nodes/telegram_send_message.rs:125`.) `core.httpRequest` returns the same
shape (`src/nodes/http_request.rs:100`). The `binary` field is set to an empty
JSON object (`serde_json::json!({})`) when there's no binary payload to attach.

## 3. Error handling conventions

`NodeError` is a small enum with a single variant today (`src/node.rs:39-43`):

```rust
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("node execution failed: {0}")]
    ExecutionFailed(String),
}
```

Every failure path inside `execute()` — a missing parameter, a failed HTTP
request, an unresolved credential, an unexpected API response — becomes
`Err(NodeError::ExecutionFailed(message))`. Nothing extra is required to wire
this into the engine's error handling: `NodeError` returned from `execute()` is
what the engine's per-node error-output routing already dispatches on — if the
node's error output is connected in the workflow, the connected downstream
node(s) receive it; if it isn't connected, the whole run aborts. You don't
write any of that routing yourself; it's automatic the moment `execute()`
returns `Err`. Your only job as a node author is to make sure every failure
path actually returns `Err(NodeError::ExecutionFailed(..))` with a message
useful enough for a workflow author to debug from — and, per §4 below, never
leaks a secret into that message.

## 4. Using credentials

### How `ctx.credentials` gets populated

Before a workflow run starts, `resolve_credentials_for_workflow` (defined at
`src/credentials.rs:6-28`) scans every node in the workflow for
`parameters.auth.credential_id`:

```rust
for node in &workflow.nodes {
    if let Some(id_str) = node.parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str()) {
        let id = Uuid::parse_str(id_str)
            .map_err(|e| anyhow::anyhow!("node {} has an invalid credential_id: {e}", node.id))?;
        ids.insert(id);
    }
}
```

It dedupes the collected IDs into a `HashSet`, then fetches and decrypts each
credential exactly once per run (`storage.get_credential(id)`), building the
`HashMap<Uuid, serde_json::Value>` (`src/credentials.rs:19-27`) that becomes
`ctx.credentials` for every node in that run. This means: **your node's
convention for referencing a credential is `parameters.auth.credential_id`** —
follow that exact shape (a nested `auth` object with a `credential_id` string
that parses as a `Uuid`) or the resolver won't find it and your node's
credential lookup will always fail with "not resolved for this run".

### Reading a credential inside `execute()`

`telegram_send_message.rs` demonstrates the pattern
(`src/nodes/telegram_send_message.rs:73-88`):

```rust
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
```

`core.httpRequest`'s `apply_auth` (`src/nodes/http_request.rs:103-159`) follows
the identical shape for its own `bearer` / `apiKey` / `basic` auth types.

### The security discipline: never let a secret leak into a `NodeError` message

This is the part that needs active thought for every new integration, not
blind copy-paste. Once you have a secret value in scope (a token, an API key,
a password), trace every place it could end up embedded in something you might
then format into an error message:

- **A URL.** Some APIs (Telegram's Bot API among them) embed the secret
  directly in the request path:
  `https://api.telegram.org/bot<TOKEN>/sendMessage`
  (`src/nodes/telegram_send_message.rs:95`, `DEFAULT_API_BASE_URL` at line 16).
  `core.httpRequest` doesn't have this problem — its `url` parameter is always
  caller-supplied and never secret (see the doc comment at
  `src/nodes/telegram_send_message.rs:49-57`) — so it's fine for
  `core.httpRequest` to freely `format!("request failed: {e}")` with the raw
  `reqwest::Error` (`src/nodes/http_request.rs:88-91`). `telegram_send_message.rs`
  cannot do that: `reqwest::Error`'s `Display` output can include the URL it
  was building or sending, and that URL contains the bot token. So its
  equivalent line deliberately discards `e` instead of formatting it:
  ```rust
  .map_err(|_| NodeError::ExecutionFailed("telegram.sendMessage: request to Telegram API failed".into()))?;
  ```
  (`src/nodes/telegram_send_message.rs:101-106`.) Every error path in that
  function follows the same rule — see the block comment right above it
  (`src/nodes/telegram_send_message.rs:97-100`) and the one on
  `execute_with_client` itself (`src/nodes/telegram_send_message.rs:49-57`):
  never interpolate the raw `reqwest::Error`, the constructed `url` variable,
  or the raw `bot_token` value into any `NodeError::ExecutionFailed` message.
- **A response body field.** It's fine to surface non-secret fields the remote
  API returns about *why* a call failed — `telegram_send_message.rs` does this
  with Telegram's own `description` field from the parsed response body
  (`src/nodes/telegram_send_message.rs:116-123`), which is safe because
  Telegram doesn't echo the bot token back in that field.
- **A header.** Not applicable to the Telegram integration (the token is in the
  URL, not a header), but the same discipline applies: if your integration
  sends a secret in a header (as `core.httpRequest`'s `bearer` / `apiKey` auth
  types do), don't format the outgoing `RequestBuilder` or any header map into
  an error message.
- **A log line.** r8r doesn't currently log raw request/response details from
  node execution, but if you add any `tracing`/`log` call inside your node,
  apply the same rule to it as to `NodeError` messages.

When you add a new integration with a credential, ask explicitly: *where does
the secret physically end up* (URL, header, request body) and *what values do
I format into a string anywhere downstream of that point*? Then make sure none
of those format calls include the secret or anything (like a `reqwest::Error`
or a URL) that might transitively contain it. `telegram_send_message.rs`'s test
`network_failure_error_message_never_contains_bot_token`
(`src/nodes/telegram_send_message.rs:228-262`) is a good pattern to copy: it
provokes a real connection-level `reqwest::Error` against a dead server and
asserts the bot token never appears anywhere in the resulting error message —
proving the discipline actually holds at runtime, not just that the code is
written that way.

## 5. Registering the node

Every node type is wired into the registry in `src/nodes/mod.rs`. Two
additions, both already present for the Telegram node:

1. Declare the module (`src/nodes/mod.rs:11`, alphabetically among the other
   `pub mod` lines):
   ```rust
   pub mod telegram_send_message;
   ```
2. Register an instance inside `register_all()` (`src/nodes/mod.rs:30`):
   ```rust
   registry.register(Box::new(telegram_send_message::TelegramSendMessageNode));
   ```

`NodeRegistry::register` (`src/node.rs:83-85`) keys the registry by
`node.type_name()`, so the node becomes reachable under
`"telegram.sendMessage"` — whatever string your `type_name()` returns — the
moment `register_all()` runs. There is no other registration step; a node not
added to `src/nodes/mod.rs` is unreachable from workflow execution even if the
rest of the file compiles.

## 6. Testing

Both real HTTP-calling nodes in this codebase use
[`wiremock`](https://docs.rs/wiremock) to test against a local mock server
instead of the real external API. The pattern, from
`telegram_send_message.rs`'s tests (`src/nodes/telegram_send_message.rs:128-263`):

```rust
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

let server = MockServer::start().await;
Mock::given(method("POST"))
    .and(path_regex(r"^/bot123:ABC/sendMessage$"))
    .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "ok": true,
        "result": {"message_id": 42}
    })))
    .mount(&server)
    .await;
```

`http_request.rs`'s tests use the same three types plus the `header` matcher
(`src/nodes/http_request.rs:164, 189-194`) for asserting an auth header was
sent correctly — `wiremock::matchers::{header, method, path}` there, versus
`wiremock::matchers::{method, path_regex}` in the Telegram tests (`path_regex`
because the bot token is embedded in the path itself and the test needs to
match on it).

### Why Telegram needs a test-injectable base URL and `core.httpRequest` doesn't

`core.httpRequest`'s `url` parameter is already fully caller-supplied — a test
just points it straight at `server.uri()` (e.g.
`"url": format!("{}/data", server.uri())` at `src/nodes/http_request.rs:178`).
There's no separate concept of a "base URL" to inject; the whole URL is a
parameter already.

`telegram.sendMessage`, by contrast, hardcodes its base URL
(`DEFAULT_API_BASE_URL = "https://api.telegram.org"`,
`src/nodes/telegram_send_message.rs:16`) because that's what a real Telegram
integration should default to — a workflow author configuring this node
shouldn't have to know or supply Telegram's API host. But that means a test
has no caller-supplied URL to redirect at a mock server. The node solves this
by adding an **optional override parameter**, `api_base_url`
(`src/nodes/telegram_send_message.rs:90-95`), intended for tests:

```rust
let base_url = ctx
    .parameters
    .get("api_base_url")
    .and_then(|v| v.as_str())
    .unwrap_or(DEFAULT_API_BASE_URL);
let url = format!("{base_url}/bot{bot_token}/sendMessage");
```

Tests set `"api_base_url": server.uri()` in their parameters
(`src/nodes/telegram_send_message.rs:156, 188, 250`) to redirect requests at
the `MockServer`; real workflows simply never set it and get
`DEFAULT_API_BASE_URL`. **The rule of thumb:** if your node's target host is a
caller-supplied parameter already, you don't need this — just point the test
at `server.uri()` directly. If your node hardcodes a specific external API's
host (because the whole point of the node is that one integration), add an
optional override parameter like `api_base_url` so it stays testable without
hitting the real API in CI.

**Security caveat for URL-embedded-secret integrations:** nothing marks
`api_base_url` as test-only at runtime — it's an ordinary workflow parameter,
readable and settable by anyone who can author a workflow. That's fine for
integrations where the secret travels in a header (like `core.httpRequest`'s
Bearer/apiKey/Basic auth): redirecting the request doesn't hand the secret to
the redirect target. It's *not* fine for an integration like this one where
the secret is embedded in the URL path itself (`/bot<TOKEN>/sendMessage`) —
pointing `api_base_url` at a host you control lets you read the bot token
straight out of the incoming request. If you're adding a node with a
URL-embedded secret, treat `api_base_url` (or any similar override) as a
capability, not a convenience, and say so in the node's own docs.

### Combining with `execute_with_client` for finer-grained tests

Because `execute_with_client` is a free function separate from the trait's
`execute()`, tests (or a node with more complex client configuration needs)
can call it directly with a hand-built `reqwest::Client`, bypassing the
`http_client()` singleton entirely. `http_request.rs`'s
`slow_response_past_configured_timeout_returns_execution_failed_error` test
does exactly this to test timeout behavior with a much shorter timeout than
the real 30-second default, without slowing down the test suite
(`src/nodes/http_request.rs:233-269`).

### Other cases worth covering

Beyond the happy path, `telegram_send_message.rs`'s test module covers:
missing required parameter (`missing_chat_id_returns_error`), the upstream API
returning a structured failure (`telegram_ok_false_returns_error_with_description`),
an unresolved credential (`unresolved_credential_returns_error`), and — per the
security discipline in §4 — a network-level failure whose error message is
asserted to never contain the secret (`network_failure_error_message_never_contains_bot_token`).
Use this as your test checklist for any new credentialed, HTTP-calling node.

## 7. Closing checklist

When adding a new node, confirm:

- [ ] **Parameter validation** — every required parameter is extracted with
      `.get(...)` (+ typed accessor as needed) and `.ok_or_else(|| NodeError::ExecutionFailed(...))?`,
      with a message naming the node's `type_name()` and the missing field.
- [ ] **Error handling** — every fallible operation inside `execute()` (or the
      free function it delegates to) returns `Err(NodeError::ExecutionFailed(..))`
      on failure; no `.unwrap()`/`.expect()` on data that can come from outside
      the process (API responses, parsed parameters, external I/O).
- [ ] **Credential security (if applicable)** — if the node uses
      `parameters.auth.credential_id` / `ctx.credentials`, trace every place the
      secret value could end up (URL, header, body) and confirm no
      `NodeError` message, log line, or other formatted string downstream of
      that point can leak it — including via a wrapped library error's
      `Display` output.
- [ ] **Tests** — a happy-path test (via `wiremock` if the node makes HTTP
      calls), a missing-required-parameter test, an unresolved-credential test
      if applicable, an upstream-error-response test, and — for any
      credentialed node — a test proving secrets never leak into error
      messages. If the node hardcodes a specific external API's host, add a
      test-injectable override parameter (like `api_base_url`) so tests never
      hit the real API.
- [ ] **Registration** — `pub mod your_node;` added to `src/nodes/mod.rs`, and
      `registry.register(Box::new(your_node::YourNode));` added inside
      `register_all()`.
- [ ] **Metadata** — `display_name()`, `description()`, `category()`
      implemented (required); `icon()` and `credential_types()` overridden
      if the defaults (`"⚙️"`, no credential) aren't right for this node;
      `output_ports()` overridden if this node has more than one output
      port or a data-dependent port count.
