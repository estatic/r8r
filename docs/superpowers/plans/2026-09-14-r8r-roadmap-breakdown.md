# r8r Roadmap — Task Breakdown (Plans 2–7)

Status: Planning reference (not an execution-ready implementation plan)
Date: 2026-09-14

## Purpose

This is a hierarchical work-breakdown of everything remaining in the r8r
roadmap after the Foundation plan (`docs/superpowers/plans/2026-09-13-r8r-foundation.md`,
merged to `main`). It expands the six Roadmap phases named in the spec
(`docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md`
§9) into a 4-level outline: **Phase → Feature → Task → Subtask**.

This document is a planning aid, not a subagent-executable plan. Each
phase still gets its own detailed, TDD-structured implementation plan
(via the brainstorming → writing-plans flow, with literal code and
review checkpoints) when work on it actually starts — the same way the
Foundation plan was produced. Numbering below (`2.1.1.1`) is stable
across edits so tasks can be referenced and checked off without
renumbering everything each time.

Phases are numbered 2–7 to match the spec's own Roadmap numbering
(Plan 1 was Foundation).

---

## Plan 2 — Expression Engine + Control-Flow Nodes

**Goal:** n8n-compatible `{{ }}` expressions via embedded QuickJS, plus
the control-flow node set (If/Switch/Merge/Filter/Wait/Code/NoOp), and
upgrade the engine from Foundation's linear-only walk to real DAG
scheduling with branching, merge, and per-node error-output routing.

### 2.1 Expression Engine
- 2.1.1 Embed QuickJS runtime
  - 2.1.1.1 Add `rquickjs` dependency; verify it builds on target platforms (Linux at minimum)
  - 2.1.1.2 Wrap QuickJS in a per-evaluation-call context — no shared mutable state leaks across items
  - 2.1.1.3 Benchmark cold-start/eval latency to confirm the "lightweight" goal still holds
- 2.1.2 Expression syntax support
  - 2.1.2.1 `{{ }}` interpolation parsing inside string parameter fields
  - 2.1.2.2 Expose `$json` bound to the current item
  - 2.1.2.3 Expose `$node["Name"].json`, `$items()`, `$now`, `$workflow`
  - 2.1.2.4 Malformed expression surfaces as `NodeError`, never a panic
- 2.1.3 Parameter resolution pipeline
  - 2.1.3.1 Walk a node's parameter JSON tree, evaluate any string containing `{{ }}`
  - 2.1.3.2 Wire resolved parameters into `NodeExecutionContext` before `execute()` runs
  - 2.1.3.3 Test resolution against nested objects/arrays in parameters, not just top-level strings

### 2.2 Control-Flow Nodes
- 2.2.1 If node
  - 2.2.1.1 Boolean condition via expression
  - 2.2.1.2 Two named outputs (`true`/`false`)
- 2.2.2 Switch node
  - 2.2.2.1 Multi-way branch on an expression value
  - 2.2.2.2 Default/fallthrough output for unmatched values
- 2.2.3 Merge node
  - 2.2.3.1 Append mode
  - 2.2.3.2 Merge-by-key mode
  - 2.2.3.3 Wait-for-all mode
- 2.2.4 Filter, Wait, NoOp/Sticky nodes
  - 2.2.4.1 Filter: keep items matching a condition, drop the rest
  - 2.2.4.2 Wait: fixed-duration delay before continuing
  - 2.2.4.3 NoOp/Sticky: passthrough execution / canvas-only annotation

### 2.3 DAG Execution Engine Upgrade
- 2.3.1 Replace `linear_order` with topological scheduling
  - 2.3.1.1 Build a dependency graph from `workflow.connections`
  - 2.3.1.2 Topologically sort nodes; detect and reject genuine cycles (extend Foundation's cycle-detection logic rather than duplicate it)
  - 2.3.1.3 Support multiple outgoing connections per node — branching is now a feature, not a validation error
- 2.3.2 Branch/merge semantics
  - 2.3.2.1 A node executes only once every required input has produced data
  - 2.3.2.2 Independent branches execute concurrently via Tokio
- 2.3.3 Per-node error-output routing
  - 2.3.3.1 A connected error output receives failure data instead of aborting the run
  - 2.3.3.2 Execution-level error only when no error output is connected
- 2.3.4 Carry forward disabled-node semantics
  - 2.3.4.1 Re-verify Foundation's disabled-node passthrough still holds under the new scheduler

### 2.4 Code Node
- 2.4.1 Full-script execution (not single-expression eval)
  - 2.4.1.1 Accept a JS script parameter, run with `items` in scope
  - 2.4.1.2 Returned items validated against the `Item` shape before continuing
- 2.4.2 Sandbox constraints
  - 2.4.2.1 Execution timeout per Code node run
  - 2.4.2.2 No filesystem/network access from inside the QuickJS sandbox

---

## Plan 3 — Triggers

**Goal:** Webhook and Schedule triggers, with a real activation lifecycle
so a workflow's `active` flag actually registers/unregisters live
listeners instead of being inert metadata.

### 3.1 Webhook Trigger
- 3.1.1 Route registration
  - 3.1.1.1 `/webhook/:workflow_id/:path` mounted on the existing Axum listener
  - 3.1.1.2 GET/POST support; capture headers/query/body as the first item
- 3.1.2 Workflow activation wiring
  - 3.1.2.1 Activating a workflow with a Webhook trigger registers its route
  - 3.1.2.2 Deactivating a workflow unregisters the route
- 3.1.3 Concurrency/safety
  - 3.1.3.1 Decide and test respond-then-execute vs. execute-then-respond so the webhook caller isn't blocked indefinitely

### 3.2 Schedule (Cron) Trigger
- 3.2.1 `tokio-cron-scheduler` integration
  - 3.2.1.1 Add dependency; wire a scheduler instance into `AppState`
  - 3.2.1.2 Parse standard cron syntax from node parameters
- 3.2.2 Activation lifecycle
  - 3.2.2.1 Register/unregister scheduled jobs on workflow activate/deactivate
  - 3.2.2.2 Re-register active schedules on process startup (survive a restart)

### 3.3 Workflow Active-Flag Plumbing
- 3.3.1 Storage support
  - 3.3.1.1 Verify/extend the update path for persisting `active` reliably
- 3.3.2 API support
  - 3.3.2.1 An endpoint to toggle `active`, triggering register/unregister as a side effect
- 3.3.3 Trigger registry
  - 3.3.3.1 A central `workflow_id -> registered trigger handles` map so deactivation can clean up webhook routes and cron jobs alike

---

## Plan 4 — HTTP Integrations

**Goal:** Generic HTTP Request node, encrypted Credential storage, and
Telegram as the reference third-party integration built on top of the
HTTP Request pattern.

**Status:** Plan 4 is complete. §4.1 and §4.2 shipped as "Plan 4a"
(`docs/superpowers/plans/2026-09-15-r8r-plan4a-credentials-http.md`, merged
to `main`). §4.3.2 (Telegram action node) and §4.3.3 (developer docs)
shipped as "Plan 4b" (`docs/superpowers/plans/2026-09-16-r8r-plan4b-telegram-node.md`,
merged to `main`). §4.3.1 (Telegram Trigger, long-polling only — webhook mode
deliberately deferred, see below) shipped as "Plan 4c"
(`docs/superpowers/plans/2026-09-16-r8r-plan4c-telegram-trigger.md`, merged
to `main`).

### 4.1 HTTP Request Node
- 4.1.1 Core request building
  - 4.1.1.1 Method, URL, headers, query params, body (JSON/form/raw)
  - 4.1.1.2 Expression-resolved parameters (reuses Plan 2's expression engine)
- 4.1.2 Auth support
  - 4.1.2.1 Bearer / API-key / basic auth via a Credential reference
- 4.1.3 Response handling
  - 4.1.3.1 Parse JSON responses into items
  - 4.1.3.2 Non-2xx handling: route to error output vs. execution failure

### 4.2 Credential Storage
- 4.2.1 Domain + storage
  - 4.2.1.1 `credentials` table migration; `Credential` domain type (already speced, not yet built)
  - 4.2.1.2 Encrypt at rest (AES-256-GCM), key from a local keyfile or env var
- 4.2.2 Storage trait methods
  - 4.2.2.1 `create_credential`/`get_credential`/`list_credentials` on the `Storage` trait + SQLite impl
- 4.2.3 API surface
  - 4.2.3.1 Auth-gated credential endpoints; list responses never include decrypted secret material
  - 4.2.3.2 Node execution context can request decrypted credential data scoped to what the node declares needing

### 4.3 Telegram Nodes (reference integration)
- 4.3.1 Telegram Trigger — **shipped, long-polling only** (`telegram.trigger`, `src/nodes/telegram_trigger.rs` + `src/telegram_poller.rs`)
  - 4.3.1.1 Long-polling mode — done; one spawned background task per activated workflow, mirroring `core.schedule`'s per-workflow job pattern
  - 4.3.1.2 Webhook mode (`setWebhook`), reusing Plan 3's webhook infrastructure — **deliberately deferred**, own future plan (needs a design decision about interop with `core.webhook`'s existing single-node-type-per-route model)
- 4.3.2 Telegram (action node) — **shipped** (`telegram.sendMessage`, `src/nodes/telegram_send_message.rs`)
  - 4.3.2.1 Send message — done; `sendPhoto`/other methods not built (4.3.2.2 partially deferred)
  - 4.3.2.2 Send photo / other basic methods — not built; same pattern as `sendMessage`, add when needed
  - 4.3.2.3 Built as a thin wrapper over the HTTP Request pattern — doubles as the "add a new integration" reference (own dedicated `reqwest::Client`, not shared with `core.httpRequest`, a deliberate scoped-simplicity choice)
- 4.3.3 Developer docs — **shipped** (`docs/adding-a-node.md`)
  - 4.3.3.1 Short "adding a new node" guide using Telegram as the worked example

Plan 4b's final review found and fixed a real security issue before merge:
Telegram's dedicated HTTP client used reqwest's default redirect/referer
behavior, which could leak the bot-token-bearing URL (the token lives in the
URL path per Telegram's Bot API scheme) via the `Referer` header on a
redirect — a channel outside the "never put secrets in error messages"
discipline the node's error paths were built around. Fixed by disabling both
on that client. Deferred hardening items tracked in issues #47 (comment) and
#48 (milestone 25, "Plan 7 — Execution Hardening").

Plan 4c's final review found and fixed three issues before merge: a
production-only activation race (the poller's first active-check could race
ahead of the DB write that activated it — invisible in tests because the
in-memory test pool's single connection accidentally serializes the two
queries; real under a production multi-connection pool), a
credential-resolution-ordering bug that could silently drop a Telegram
update on a transient failure (offset advanced before resolution ran — fixed
by resolving once per batch, before any offset mutation), and test coverage
that didn't actually prove execution persistence or that the Update payload
reached the workflow (fixed with a request-recording `Storage` test wrapper
and expression-templated E2E assertions). All three independently
re-reviewed and confirmed fixed. Deferred hardening items — including the
root-cause fix for `fire_schedule`/`handle_webhook`'s identical
missing-credential-resolution gap, which Plan 4c's mid-flight fix only
patched for the Telegram poller — tracked in issue #49 (milestone 25).

---

## Plan 5 — AI Agent Node

**Goal:** An LLM-backed Agent node with tool-calling (other nodes and
sub-workflows exposed as callable tools) and per-execution memory,
implemented against provider HTTP APIs directly — no LangChain
dependency, per the spec's lightweight-footprint goal.

### 5.1 LLM Provider Clients
- 5.1.1 Anthropic-compatible HTTP client
  - 5.1.1.1 Request/response types for the Messages API
  - 5.1.1.2 Decide streaming vs. non-streaming scope for v1
- 5.1.2 OpenAI-compatible HTTP client
  - 5.1.2.1 Request/response types for a Chat Completions-style API
- 5.1.3 Provider abstraction
  - 5.1.3.1 A trait/enum so the Agent node switches providers via credential config

### 5.2 Tool-Calling Loop
- 5.2.1 Tool exposure
  - 5.2.1.1 Expose other workflow nodes as callable tools (map Node parameter schema → tool schema)
  - 5.2.1.2 Expose sub-workflows as callable tools
- 5.2.2 Loop control
  - 5.2.2.1 Multi-turn tool-use loop with a max-iteration cap
  - 5.2.2.2 Error handling when a tool node fails mid-agent-run

### 5.3 Conversational Memory
- 5.3.1 Per-execution buffer
  - 5.3.1.1 Short-term memory scoped to a single execution, not persisted across executions (per spec's v1 scope)
  - 5.3.1.2 Token/size budget for the memory buffer

### 5.4 Agent Node Wiring
- 5.4.1 Node implementation
  - 5.4.1.1 System prompt, model/provider, tool list as node parameters
  - 5.4.1.2 Register in `NodeRegistry` alongside existing nodes

---

## Plan 6 — Frontend

**Goal:** Vue 3 + Vue Flow SPA against the existing REST/WebSocket API —
canvas editing, inline JSON data preview, execution log/replay.

### 6.1 Project Scaffold
- 6.1.1 Vue 3 + Vite setup
  - 6.1.1.1 Base project, TypeScript config, dev-server proxy to the Rust API
- 6.1.2 Vue Flow canvas integration
  - 6.1.2.1 Render `workflow.nodes`/`connections` as a Vue Flow graph
  - 6.1.2.2 Drag/connect interactions write back to workflow state

### 6.2 Workflow Editor
- 6.2.1 Node parameter panel
  - 6.2.1.1 Dynamic form generation from a node's parameter schema
- 6.2.2 Canvas persistence
  - 6.2.2.1 Save node positions/connections back via the workflow update endpoint
- 6.2.3 Credential picker
  - 6.2.3.1 UI to select/create a Credential for nodes that need one

### 6.3 Execution View
- 6.3.1 Live execution status
  - 6.3.1.1 WebSocket client subscribing to execution progress (depends on Plan 7's WebSocket push)
- 6.3.2 Inline JSON data preview
  - 6.3.2.1 Per-node input/output item viewer
- 6.3.3 Execution log/replay
  - 6.3.3.1 List past executions for a workflow; inspect a past run's per-node data

### 6.4 Auth UI
- 6.4.1 Login/register screens
  - 6.4.1.1 JWT storage decision (memory vs. localStorage); API client auth-header wiring

---

## Plan 7 — Execution Hardening

**Goal:** Close the gaps intentionally deferred out of Foundation's
final review — incremental execution persistence, retry, typed storage
errors, and the access-control composition risk flagged during that
review (spec §8's single-shared-workspace model plus open
self-registration).

### 7.1 Live Execution Status
- 7.1.1 WebSocket endpoint
  - 7.1.1.1 Push per-node start/finish/error events during a run
- 7.1.2 Incremental persistence
  - 7.1.2.1 Persist execution data per-node as it completes, not only the final snapshot (Foundation's known gap)

### 7.2 Retry Semantics
- 7.2.1 Retry-from-failed-node
  - 7.2.1.1 Re-run a failed execution from its persisted input snapshot at the failed node
  - 7.2.1.2 A retry endpoint on the executions API

### 7.3 Storage Error Typing
*(carried over from Foundation's final review, Important #6 — deferred as a cross-cutting change)*
- 7.3.1 `StorageError` enum
  - 7.3.1.1 `NotFound` / `Conflict` / `Backend(anyhow::Error)` variants on the `Storage` trait
  - 7.3.1.2 Update the SQLite impl and every API call site currently pattern-matching on `.is_err()`

### 7.4 Access Control Hardening
*(carried over from Foundation's final review, Important #3 — documented but not fixed there)*
- 7.4.1 First-run owner claim or registration gate
  - 7.4.1.1 Block/gate self-registration after the first user, or an env-flag toggle, consistent with spec's single-shared-workspace v1 model
- 7.4.2 Per-workflow ACL (stretch — may move to a later plan)
  - 7.4.2.1 Revisit whether v1's "no per-workflow permissions" decision still holds once real multi-user usage exists

### 7.5 Operational Polish
*(carried over minor findings from Foundation's final review)*
- 7.5.1 Email normalization (lowercase/trim on register and login)
- 7.5.2 Reap executions stuck in `Running` after a process crash
- 7.5.3 Pagination on workflow/execution listing endpoints
- 7.5.4 Index on `executions.workflow_id`

### 7.6 Plan 2 Deferred Items
*(carried over from Plan 2's execution — deliberate v1 simplifications and one findings the final whole-branch review explicitly flagged as worth fixing before untrusted input reaches it)*
- 7.6.1 Concurrent branch execution
  - 7.6.1.1 Replace the sequential topological walk in `execute_workflow` with `tokio::spawn`/`JoinSet` for independent branches (Plan 2's engine already computes each node's inputs from a `produced` map before running it, independent of iteration order beyond dependency order, so nothing blocks this later)
- 7.6.2 Per-item expression evaluation
  - 7.6.2.1 Let `$json` (and per-item evaluation generally) see each item's own data within one node's execution, instead of only the first input item — needed for a genuinely per-item Filter condition and a richer Set node
- 7.6.3 Code node sandbox hardening
  - 7.6.3.1 Add `rquickjs::Runtime::set_memory_limit` and a deadline-based `set_interrupt_handler` in `eval_js`, superseding the current `spawn_blocking`-based timeout (which leaves a script's thread running/allocating in the background after a 2s timeout — bounded by tokio's blocking-pool cap, but unbounded in wall-clock/RAM until then)

### 7.7 Credential/HTTP Request hardening
*(parked by Plan 4a's final whole-branch review — none individually blocking, none exploitable under the current single-shared-workspace trust model, but a coherent hardening batch)*
- 7.7.1 Truncate/redact upstream response body in `core.httpRequest`'s non-2xx error message
- 7.7.2 Mark `apiKey` auth header value as sensitive (`HeaderValue::set_sensitive(true)`)
- 7.7.3 Prevent duplicate `Authorization` headers when a user supplies `headers.authorization` alongside `auth.type: "bearer"`
- 7.7.4 Scope `NodeExecutionContext.credentials` per-node instead of cloning the full run's credential map into every node's context
- 7.7.5 Bind `credential.id` as AEAD associated data on encrypt/decrypt
- 7.7.6 Cap `core.httpRequest`'s response body read size (currently unbounded)
- 7.7.7 Make `auth.type` a hard-required field when `auth`/`credential_id` is present
- 7.7.8 Fix `.env.example`'s `CREDENTIALS_KEY` placeholder (not valid base64 as shipped) and add the `(see .env.example)` suffix to its error message
- 7.7.9 Remove unused `rand` direct dependency from `Cargo.toml`
- 7.7.10 Give `Credential` a hand-written `Debug` impl that redacts `data`

---

## Sequencing Notes

- **2 → everything else.** The expression engine and DAG scheduler are load-bearing for nearly every later phase (HTTP Request's parameters, the Agent node's tool schemas, the frontend's node forms all assume expressions exist).
- **3 and 4 are independent of each other** and could be built in either order once Plan 2 lands.
- **5 depends on 4** (the Agent node's tool-exposure story is cleanest once the HTTP Request node and Credential storage already exist as the pattern to follow).
- **6 depends on 2 and 3 at minimum** for a genuinely useful editor (branching nodes and triggers are core to what a workflow editor needs to display); **6.3.1 (live status)** specifically depends on **7.1** (WebSocket push).
- **7** is mostly independent and could be pulled earlier — 7.3 and 7.4 in particular are already-known gaps, not new discovery, so they could be scheduled right after Plan 2 if hardening is prioritized over new features.
