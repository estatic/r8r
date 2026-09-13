# r8r — Design Spec (v1)

Status: Draft for review
Date: 2026-09-13

## 1. Summary

r8r is a lightweight, fast, self-hostable workflow-automation engine in the
spirit of [n8n](https://github.com/n8n-io/n8n): a node-based visual editor
where users wire together triggers, logic, and integrations into
executable workflows.

r8r is **not** a byte-for-byte n8n clone or a fork. It targets the same
product shape (visual DAG editor, JSON-based node data model, n8n-style
expressions, execution history) but is a from-scratch implementation in
Rust, explicitly scoped down from n8n's ~400-node catalog and
enterprise feature set, in exchange for a small footprint and fast
cold start / low idle resource use. Feature parity with the wider node
catalog is an explicit non-goal of v1 (see §10).

## 2. Goals / Non-Goals

**Goals**
- Single static-ish binary, sub-second cold start, low idle memory
  (target: comfortably runs on a 512MB VPS or a Raspberry Pi).
- Visual, node-based workflow editor with n8n-familiar UX (canvas,
  drag/connect nodes, inline JSON data preview, execution log/replay).
- n8n-compatible expression syntax (`{{ $json.foo }}`,
  `$node["Name"].json`) so the mental model transfers directly.
- A clean node extension point (`Node` trait) so new integrations can
  be added without touching the engine.
- Core node set: control-flow/logic nodes, an LLM-backed AI Agent
  node with tool-calling, and Telegram as the reference third-party
  integration.
- Self-contained persistence (SQLite) with a swap-in path to Postgres.

**Non-Goals (v1)**
- Parity with n8n's ~400 built-in nodes. Only the node set in §5.3.
- Slack and Email nodes (explicitly excluded per product decision —
  may be added later using the same Node trait, no engine changes
  needed).
- Enterprise features: SSO/LDAP, advanced RBAC, environments,
  git-based source control for workflows, audit logs.
- Distributed/queue execution mode (main/worker/webhook process
  split). The engine is designed so this can be added later (see
  §9), but v1 runs everything in one process.
- Running arbitrary n8n community nodes (npm packages) unmodified.
- Byte-for-byte REST API compatibility with n8n.

## 3. High-Level Architecture

```
┌─────────────────────────────────────────────────────────┐
│                     r8r-server (binary)                 │
│                                                           │
│  ┌───────────┐   ┌────────────┐   ┌───────────────────┐ │
│  │  Axum API  │  │  Webhook    │  │  Scheduler (cron)  │ │
│  │  /rest/*   │  │  /webhook/* │  │  tokio-cron-sched   │ │
│  └─────┬─────┘   └──────┬─────┘   └──────────┬────────┘ │
│        │                │                     │           │
│        └────────────────┴─────────┬───────────┘           │
│                                    ▼                       │
│                         ┌─────────────────────┐            │
│                         │  Execution Engine     │            │
│                         │  (DAG scheduler,      │            │
│                         │   node dispatch)      │            │
│                         └──────────┬────────────┘            │
│                                    │                          │
│              ┌─────────────────────┼─────────────────────┐   │
│              ▼                     ▼                     ▼   │
│      ┌───────────────┐   ┌──────────────────┐   ┌───────────┐│
│      │  Node Registry │   │  Expression Engine│   │  Storage  ││
│      │  (trait impls) │   │  (QuickJS/rquickjs)│  │  trait    ││
│      └───────────────┘   └──────────────────┘   └─────┬─────┘│
│                                                          │      │
└──────────────────────────────────────────────────────────┼──────┘
                                                             ▼
                                                  ┌───────────────────┐
                                                  │ SQLite (default)  │
                                                  │ or Postgres        │
                                                  └───────────────────┘

           ▲ REST + WebSocket (execution status)
           │
┌──────────┴──────────┐
│   Frontend (SPA)     │
│   Vue 3 + Vue Flow    │
└───────────────────────┘
```

The server is one OS process. All components above the storage line
run in-process using Tokio tasks; there is no separate worker process
or message broker in v1.

## 4. Data Model

Core entities (persisted via the `Storage` trait, backed by SQLite by
default):

- **Workflow**: `id`, `name`, `active: bool`, `nodes: Vec<NodeInstance>`,
  `connections: Vec<Connection>`, `settings` (timezone, error workflow
  id, etc.), `created_at`/`updated_at`.
- **NodeInstance**: `id` (unique within workflow), `type` (node type
  key, e.g. `"core.if"`), `position` (x/y for canvas), `parameters`
  (JSON, node-specific, may contain expressions), `disabled: bool`.
- **Connection**: `(from_node, from_output_index) -> (to_node,
  to_input_index)`, with a distinguished `error` output type per node
  (mirrors n8n's error-branch model).
- **Credential**: `id`, `name`, `type` (e.g. `"telegramApi"`), `data`
  (JSON, **encrypted at rest** with AES-256-GCM; key from a local
  keyfile or env var), `owner_id`.
- **Execution**: `id`, `workflow_id`, `status`
  (`running|success|error|waiting`), `started_at`, `finished_at`,
  `mode` (`manual|trigger|webhook|retry`), `data` (per-node
  input/output snapshots for the execution log/replay UI).
- **User**: `id`, `email`, `password_hash` (argon2), `role`
  (`owner|member`), `created_at`.

Item/data shape passed between nodes matches n8n's convention for
familiarity and expression compatibility:

```json
[{ "json": { "foo": "bar" }, "binary": { "file1": { "mimeType": "...", "data": "<base64 or ref>" } } }]
```

## 5. Node System

### 5.1 Node trait

```rust
#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn description(&self) -> NodeDescription; // display name, icon, category,
                                               // parameter schema (JSON Schema-ish),
                                               // input/output port definitions
    async fn execute(&self, ctx: NodeExecutionContext) -> Result<NodeOutput, NodeError>;
}
```

`NodeExecutionContext` carries: resolved parameters (expressions
already evaluated against the current item + prior node outputs),
input items, credential access (decrypted on demand, scoped to what
the node declares it needs), and a cancellation token.

`NodeOutput` is `Vec<Vec<Item>>` — one `Vec<Item>` per output port,
matching n8n's multi-output model (e.g. If node has `true`/`false`
outputs; every node has an implicit error output when it declares
`continueOnFail`/error routing).

### 5.2 Registry

Nodes are compiled into the binary and registered at startup into a
`NodeRegistry: HashMap<&'static str, Box<dyn Node>>`. No dynamic
loading / plugin ABI in v1 — adding a node means adding a Rust module
and registering it, then rebuilding. This is a deliberate simplicity
trade-off; a dylib-based plugin system is a phase-2 candidate if
demand shows up (see §9).

### 5.3 v1 Node Set

**Triggers**
- Manual Trigger — fires on "Execute Workflow" from the editor.
- Webhook — registers `/webhook/:workflow_id/:path`, supports
  GET/POST, captures headers/query/body as the first item.
- Schedule (Cron) — `tokio-cron-scheduler`, standard cron syntax.

**Logic / control flow**
- If — boolean condition (via expression), two outputs.
- Switch — multi-way branch on an expression value.
- Merge — combine multiple input branches (append / merge-by-key /
  wait-for-all modes, matching n8n's Merge node modes).
- Filter — keep items matching a condition.
- Set — add/overwrite fields on items (expression-driven).
- Code — arbitrary JS via the embedded QuickJS engine, given
  `items` in, returns `items` out (n8n "Code node" parity).
- Wait — delay execution (fixed duration or until a webhook/time).
- NoOp / Sticky Note — passthrough / canvas annotation only.

**HTTP**
- HTTP Request — generic REST client node (method, URL, headers,
  auth, body), also serves as the template other HTTP-based
  integrations (like Telegram) are built on top of.

**AI**
- AI Agent — LLM-backed agent node. Configurable: model/provider
  (Anthropic/OpenAI-compatible HTTP APIs called directly — no
  LangChain dependency), system prompt, and a list of **tools**,
  where each tool is either another node in the workflow (exposed as
  a callable function via its parameter schema) or a sub-workflow.
  Maintains a short conversational memory buffer scoped to the
  execution (not persisted across separate executions in v1).

**Messaging (reference third-party integration)**
- Telegram Trigger — long-polling or webhook-based (`setWebhook`),
  emits incoming messages/updates as items.
- Telegram — send message / send photo / etc., built as a thin
  typed wrapper over the HTTP Request pattern; this node doubles as
  the worked example for "how to add a new integration" in
  developer docs.

Slack and Email are explicitly excluded from v1 (product decision);
they can be added later as ordinary nodes with no engine changes.

## 6. Expression Engine

- Embedded QuickJS via `rquickjs` (~1MB, microsecond startup,
  no JIT/V8 weight).
- Expression syntax matches n8n: `{{ }}` interpolation in any string
  parameter field, `$json`, `$node["NodeName"].json`, `$items()`,
  `$now`, `$workflow`, etc. exposed as JS globals injected per
  evaluation call.
- Each expression evaluation spins up a fresh QuickJS context scoped
  to the current item — no shared mutable JS state leaks between
  items/nodes (keeps execution deterministic and parallelizable).
- The Code node reuses the same QuickJS runtime but with a larger
  script (not just a single expression) and full `items` in scope.

## 7. Execution Engine

- Workflow graph is validated (no cycles outside explicitly-looped
  constructs — v1 has no native "loop" node beyond SplitInBatches,
  which iterates without graph cycles) and topologically ordered.
- Async execution over Tokio: independent branches run concurrently;
  a node only executes once all its required inputs have produced
  data (merge/wait-for-all semantics respected).
- Per-node error handling: on failure, if the node has an error
  output connected, execution continues down that branch; otherwise
  the whole execution is marked `error` and (if configured) triggers
  the workflow's designated error workflow.
- Execution data (inputs/outputs per node, per item) is persisted
  incrementally so the editor can show live progress over the
  WebSocket channel and support post-hoc inspection/replay.
- Retry: manual retry re-runs a failed execution from its persisted
  input snapshot at the failed node (not full re-execution from
  trigger), matching n8n's retry UX.

## 8. API & Frontend

**Backend**: Axum. REST under `/rest/*` (workflows, executions,
credentials, users, auth) — shaped similarly to n8n's API for
conceptual familiarity, not guaranteed byte-compatible. WebSocket
endpoint for live execution status push. Webhooks served at
`/webhook/*` on the same listener/port (no separate webhook process
in v1).

**Frontend**: Vue 3 SPA + Vue Flow for the canvas (closest existing
match to n8n's drag/connect/inline-data-preview UX). Talks to the
backend purely over the REST/WebSocket API — no server-side
rendering, no coupling to the Rust backend beyond the API contract,
so the frontend could in principle be swapped for another stack
later without touching the engine.

**Auth**: email + password, argon2 hashing, JWT session cookie.
Two roles: `owner` (full access) and `member` (can be scoped later;
v1 gives members the same access as owner minus user management —
no granular per-workflow permissions yet).

## 9. Storage & Deployment

- `Storage` trait abstracts persistence (workflows, executions,
  credentials, users). Default impl: SQLite via `sqlx`, single file,
  zero external dependencies. Postgres impl behind the same trait for
  users who want it; selected via config, no code changes needed by
  the operator.
- Binary data (files passed between nodes, e.g. a Telegram photo)
  stored on local filesystem under a data directory in v1; an
  S3-compatible backend is a natural phase-2 addition behind a
  `BinaryStore` trait, not a v1 requirement.
- Ships as a single binary + a static frontend bundle (embedded via
  `rust-embed` or served from a sibling directory) — one process,
  one port, no reverse proxy required to get started.
- Distributed mode (separate webhook/worker processes coordinating
  through a shared queue) is **not** built in v1, but the Execution
  Engine and Storage trait boundaries are kept clean enough that
  this can be layered on later without a rewrite: the engine already
  treats "get next work item" and "persist execution state" as
  trait-mediated operations rather than in-memory-only state.

## 10. Explicitly Out of Scope (v1)

- Slack, Email, and the rest of n8n's node catalog beyond §5.3.
- Community/plugin node loading (dylib or JS-package based).
- SSO/LDAP, RBAC beyond owner/member, audit logging, environments,
  git-backed workflow versioning.
- Queue/distributed execution mode.
- Byte-for-byte n8n API or n8n workflow-JSON import/export
  compatibility (a future import shim is plausible but not speced
  here).
- Long-term cross-execution AI Agent memory.

## 11. Testing Strategy

- **Node unit tests**: each `Node` impl tested in isolation with
  fixed input items and mocked HTTP (via `wiremock` for HTTP
  Request/Telegram/AI Agent's provider calls).
- **Expression engine tests**: table-driven tests covering the n8n
  expression syntax subset supported.
- **Execution engine tests**: fixture workflows (JSON) covering
  branching, merge modes, error-branch routing, and retry-from-node,
  asserting on final persisted execution data.
- **API integration tests**: Axum test server + `sqlx` in-memory
  SQLite, covering the REST surface end-to-end (create workflow,
  activate, trigger via webhook, inspect execution).
- **Frontend**: component tests for the canvas interactions
  (connect nodes, edit parameters) plus a small set of Playwright
  end-to-end flows (build a workflow, run it, see the result) against
  a running r8r-server instance.

## 12. Open Questions / Risks

- QuickJS fidelity vs n8n's Node.js-based expressions: most common
  expression patterns should port directly, but any n8n workflow
  relying on Node.js-specific globals/APIs in Code nodes will need
  adaptation. Acceptable given r8r is not claiming workflow-JSON
  compatibility.
- AI Agent tool-calling loop (multi-turn tool use, error handling
  when a tool node fails mid-agent-run) needs its own short design
  pass once implementation starts — this spec fixes the node's
  responsibility and external shape, not its internal control loop.
- Telegram long-polling vs webhook mode both need to work behind the
  same trigger node without doubling implementation effort — worth
  revisiting during implementation planning.
