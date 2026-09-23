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

**Status:** Shipped, scoped to same-workflow tool-calling
(`docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`,
`docs/superpowers/plans/2026-09-18-r8r-plan5-ai-agent.md`). The last
unbuilt major roadmap feature — everything else (Plans 2/3/4/6, 7.1) was
already shipped before this one. Both providers landed in v1 (the
brainstorm's explicit scope call, rather than shipping one first):
`AnthropicClient` (Messages API, token counting via Anthropic's own
hosted `count_tokens` endpoint) and `OpenAiClient` (Chat Completions
shape, local `tiktoken-rs` counting). Tools are declared inline as JSON
on the Agent node's own `parameters.tools` — no canvas/connection-model
changes, no frontend changes at all (verified by the final review: node
parameter editing and the credential picker already work generically for
any node). Tool-calling reaches another node type via a new, narrow
`ToolExecutor` trait on `NodeExecutionContext`, not a `Node` trait
change — every other of the 14 existing node types is untouched.
Sub-workflow-as-tool (5.2.1.2) was deliberately descoped to v1 during
brainstorming — a genuinely separate capability (cross-workflow
invocation, its own recursion question) that doesn't exist in this
codebase at all yet, deferred to its own future plan the same way
Telegram webhook mode was deferred.

Plan 5's final whole-branch review (Opus, independently re-ran the full
suite and traced the 6-task tool-call round trip end-to-end rather than
trusting the per-task reviews) found 4 Important issues before merge,
fixed in one wave and re-verified clean on 4 of 5 points: parallel tool
calls (a single turn returning more than one tool call) were serializing
as multiple separate Anthropic messages instead of one grouped message,
a real gap against strict role-alternation-enforcing API proxies, fixed
by grouping consecutive tool results into a single message; an
assistant's own text accompanying a tool call was being silently
discarded rather than preserved in conversation history, degrading
multi-turn coherence, fixed by widening `ProviderResponse::ToolCalls` to
carry the text; a stale line in `docs/adding-a-node.md`; and dead test
scaffolding that was the repo's only two compiler warnings. One residual
(5 further stale doc line-citations the fix wave's own text missed) was
fixed directly post-review rather than spending a second review round on
a docs-only, zero-code-impact correction.

A follow-up `/security-review` pass (3-candidate Step 1, parallel
false-positive filtering in Step 2, ≥8/10 threshold in Step 3) confirmed
one High finding at 8/10: the tool-call argument merge in
`src/nodes/agent.rs` let model-controlled `call.arguments` overwrite any
key in a tool's `base_parameters`, including keys never declared in
`argument_schema` (e.g. `api_base_url`, `auth.credential_id`, `script`)
— an indirect-prompt-injection path to credential exfiltration or SSRF
via a Telegram-triggered workflow. Fixed (commit `1645c65`): the merge
now only applies keys present in `argument_schema.properties`, and
`validate_arguments` hard-denies `auth`/`credential_id`/`api_base_url`/
`base_url`/`headers`/`script` regardless of schema declaration. Two
related candidates did not clear the threshold and were left as-is: a
credential-scoping gap in `EngineToolExecutor` (6/10 — real but not
independently exploitable, already tracked as 7.7.4) and raw tool-error
text reaching the LLM provider (5/10 — mostly duplicates what the
success path already sends by design).

### 5.1 LLM Provider Clients — **shipped**
- 5.1.1 Anthropic-compatible HTTP client — done (`src/llm/anthropic.rs`)
  - 5.1.1.1 Request/response types for the Messages API — done (raw `serde_json::Value` navigation, matching this codebase's existing HTTP-node convention — no typed API structs anywhere)
  - 5.1.1.2 Decide streaming vs. non-streaming scope for v1 — resolved: non-streaming only. r8r's execution model is fully synchronous (one HTTP response per run) and Plan 7.1's WebSocket only streams node-level events, not tokens — nothing would consume a stream
- 5.1.2 OpenAI-compatible HTTP client — done (`src/llm/openai.rs`; configurable base URL for self-hosted/local-model compatibility)
- 5.1.3 Provider abstraction — done (`ProviderClient` trait, `src/llm/mod.rs`; provider selection via the Agent node's own `parameters.provider`, credential resolution reuses the existing generic `auth.credential_id` mechanism with zero new code)

### 5.2 Tool-Calling Loop — **shipped, same-workflow only**
- 5.2.1 Tool exposure
  - 5.2.1.1 Expose other workflow nodes as callable tools — done, but not via "map Node parameter schema → tool schema" as originally phrased (this codebase has no per-node parameter schema system at all, confirmed absent during brainstorming). Instead: inline JSON tool declarations on the Agent node itself (`name`, `description`, `node_type`, `argument_schema`, `base_parameters`), each hand-authored by the workflow author — the model's arguments merge over `base_parameters` and dispatch via the new `ToolExecutor` trait
  - 5.2.1.2 Expose sub-workflows as callable tools — **deliberately deferred**, own future plan (see Status above)
- 5.2.2 Loop control — done (`src/nodes/agent.rs::run_agent_loop`)
  - 5.2.2.1 Multi-turn tool-use loop with a max-iteration cap — done (`max_iterations` parameter, default 10; exceeding it without a final response is a hard error, never a silently truncated answer)
  - 5.2.2.2 Error handling when a tool node fails mid-agent-run — done (a failed tool call becomes a `ToolResult{is_error: true}` fed back to the model, which gets to see and react to the failure, rather than aborting the run; an unknown tool name gets the same treatment, caught before ever reaching the tool executor)

### 5.3 Conversational Memory — **shipped**
- 5.3.1 Per-execution buffer — done
  - 5.3.1.1 Short-term memory scoped to a single execution, not persisted across executions (per spec's v1 scope) — done
  - 5.3.1.2 Token/size budget for the memory buffer — done, with REAL per-provider token counts (not an approximation) — Anthropic's own hosted `count_tokens` endpoint, OpenAI via local `tiktoken-rs`. Trims whole tool-call/result turn units from the oldest end of history when over budget, re-checking after each drop; never drops the original seed message

### 5.4 Agent Node Wiring — **shipped**
- 5.4.1 Node implementation — done (`src/nodes/agent.rs::AgentNode`, `type_name() -> "ai.agent"`)
  - 5.4.1.1 System prompt, model/provider, tool list as node parameters — done
  - 5.4.1.2 Register in `NodeRegistry` alongside existing nodes — done

---

## Plan 6 — Frontend

**Goal:** Vue 3 + Vue Flow SPA against the existing REST/WebSocket API —
canvas editing, inline JSON data preview, execution log/replay.

**Status:** §6.1, §6.2, §6.4, and part of §6.3 (6.3.2) shipped as "Plan 6a —
First Slice" (`docs/superpowers/plans/2026-09-16-r8r-plan6a-frontend.md`,
spec `docs/superpowers/specs/2026-09-16-r8r-plan6a-frontend-design.md`,
merged to `main`): login/register, workflow list (create/delete/toggle
active), a Vue Flow canvas editor (add/drag/connect/configure nodes,
credential picker), and save/execute with an inline per-node JSON results
panel for the just-run execution. 6.3.3 (execution log/replay of *past*
runs) shipped as "Plan 6b" — a bounded change (brainstormed and
implemented directly, no separate plan doc, per this project's
spec/plan-doc threshold): `GET /rest/workflows/:id/executions?limit=N`
(newest-first, default 50, capped at 200 server-side) plus an index on
`executions.workflow_id`, and a "History" selector in
`ExecutionResultsPanel` that swaps between past runs' already-fetched
per-node output. 6.3.1 (live execution status) shipped as "Plan 7.1"
(`docs/superpowers/specs/2026-09-17-r8r-plan7-live-execution-design.md`,
`docs/superpowers/plans/2026-09-17-r8r-plan7-live-execution.md`): a
`GET /ws/workflows/:id/executions` WebSocket, authenticated via a
first-message `{"token": "<jwt>"}` handshake, streams per-node
started/finished/errored/skipped events plus a final `execution_finished`
event over a single global broadcast channel filtered by `workflow_id`.
`WorkflowEditorView` opens the socket on mount and fills in
`ExecutionResultsPanel` live, with the existing synchronous
`POST .../execute` response still landing as the authoritative final
state. §7.1.2 (incremental per-node persistence, the other half of the
same plan) shipped alongside it — `LiveExecutionTracker` now writes
`node_outputs` to storage as each node completes, not only once at the
very end, closing that half of Foundation's known gap.

Plan 7.1's final whole-branch review (Opus, independently re-ran the full
backend/frontend suites and `cargo clippy` rather than trusting the
per-task reviews) found 4 Important issues before merge, all fixed in one
wave and re-verified clean: a frontend identity bug where a live event
could silently mutate a user-selected *historical* execution object (a
live reference into the Pinia store's cached history, not just the
in-flight one) because execution identity was tracked via a closure
variable that could desync from the actual displayed ref; no test proved
the single global broadcast channel's per-workflow filter actually
isolated workflows (a real cross-workflow data-leak risk with one
process-wide channel); the WebSocket test asserted event `type` but never
the payload fields (`execution_id`, `node_id`, `items`) the frontend
actually consumes; and an undocumented HTTP-contract change where
`execute`/`webhook` now return `200` (not the prior `500`) if the run
succeeded but the final DB persist failed — ruled to keep deliberately
(the workflow's real side effects already happened either way) and
documented rather than reverted. One Minor was parked, not fixed: closing
the live results panel (`@close`) during an in-flight run can have it
silently reappear on a later stray event, since the fix only treats
*object* reassignment of the ref as a respected external change, not
reassignment to `null` — cosmetic, no data loss, worth a follow-up.

Plan 6a's final whole-branch review (browser-verified with Playwright
against the built binary) found and fixed 7 issues before merge, folded
into commit `270d069`: Vue Flow's `DefaultNode` (and its connection
`<Handle>` elements) was being fully replaced by the custom node slot,
making it impossible to drag a connection in a real browser (critical);
failed API calls silently looked like success (no inline error surfaced,
editor state could be lost); a 401 from the login/register endpoints
themselves triggered the client's global redirect-to-`/login`, destroying
`LoginView` before it could render "invalid credentials"; `Backspace`
deleted a node from Vue Flow's internal store only, so it reappeared on
the next prop sync and a subsequent Save silently kept it;
`crypto.randomUUID()` threw outside a secure context (plain-HTTP LAN
access, r8r's typical self-hosted deployment); and two frontend test-setup
issues (a non-portable `NODE_OPTIONS` flag, an untyped Vitest `test` config
key). All fixes verified in a real browser, not just unit tests.

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
- 6.3.1 Live execution status — **shipped** (Plan 7.1)
  - 6.3.1.1 WebSocket client subscribing to execution progress (depends on Plan 7's WebSocket push) — done
- 6.3.2 Inline JSON data preview
  - 6.3.2.1 Per-node input/output item viewer
- 6.3.3 Execution log/replay — **shipped** (Plan 6b)
  - 6.3.3.1 List past executions for a workflow; inspect a past run's per-node data — done

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

### 7.1 Live Execution Status — **shipped**
- 7.1.1 WebSocket endpoint — done
  - 7.1.1.1 Push per-node start/finish/error events during a run — done (`GET /ws/workflows/:id/executions`, first-message JWT handshake, single global broadcast channel filtered by `workflow_id`)
- 7.1.2 Incremental persistence — done
  - 7.1.2.1 Persist execution data per-node as it completes, not only the final snapshot (Foundation's known gap) — done (`LiveExecutionTracker` in `src/execution_runner.rs`)

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
- 7.4.1 First-run owner claim or registration gate — **shipped** (commit `aec7496`)
  - 7.4.1.1 Block/gate self-registration after the first user, or an env-flag toggle, consistent with spec's single-shared-workspace v1 model — done. `POST /rest/auth/register` always succeeds for the first user; once any user exists, further registrations return 403 unless `R8R_ALLOW_OPEN_REGISTRATION` is set. New `Storage::any_user_exists` (SQLite: `SELECT EXISTS(...)`) and `AppState.open_registration` (read once at startup, not per-request, to keep the check deterministic under parallel tests instead of mutating process env vars).
- 7.4.2 Per-workflow ACL (stretch — may move to a later plan) — not started
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
- 7.6.3 Code node sandbox hardening — **shipped** (commit `c03fa28`)
  - 7.6.3.1 Add `rquickjs::Runtime::set_memory_limit` and a deadline-based `set_interrupt_handler` in `eval_js`, superseding the current `spawn_blocking`-based timeout (which leaves a script's thread running/allocating in the background after a 2s timeout — bounded by tokio's blocking-pool cap, but unbounded in wall-clock/RAM until then) — done. `eval_js` now self-bounds every call (both `core.code` scripts and every `{{ }}` parameter expression, which previously had no timeout at all) to a 2s wall-clock deadline enforced by QuickJS's own interrupt handler, plus a 64MB memory ceiling. `core.code`'s outer `spawn_blocking` + `tokio::time::timeout` (raised to 5s) is now a rarely-hit backstop rather than the primary mechanism — the inner deadline reliably interrupts even a tight infinite loop from inside the interpreter, so the blocking thread now actually returns instead of being abandoned.

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

## Plan 8 — UI/UX Hardening
*(identified from a hands-on manual trial of the running app, first real end-user pass since Plan 6a shipped the frontend)*

Six sub-projects, decomposed and prioritized by dependency analysis
(full reasoning: `docs/superpowers/specs/2026-09-21-r8r-node-type-metadata-design.md`
§1). Two — multi-port canvas and structured credential forms — hard-depend
on node-type metadata existing first (both would otherwise need to
duplicate per-node-type logic in the frontend, which the metadata system
exists to eliminate); the rest are structurally independent.

- 8.1 Connection arrows — **shipped** (b40b32f). Zero dependencies, trivial (VueFlow `markerEnd`)
- 8.2 Node-type metadata system — **shipped** (plan: `docs/superpowers/plans/2026-09-21-r8r-plan8.2-node-type-metadata.md`), spec (`docs/superpowers/specs/2026-09-21-r8r-node-type-metadata-design.md`). Foundational: display name/icon/category/description/credential-type/output-port metadata on the `Node` trait, richer `/rest/node-types`, a new per-instance output-ports endpoint. Unlocks 8.3 and 8.4.
- 8.3 Multi-port canvas + error routing — **shipped** (plan: `docs/superpowers/plans/2026-09-21-r8r-plan8.3-multi-port-canvas.md`). Depends on 8.2. Render N output handles per node (not the hardcoded single handle today), expose the already-fully-working `ERROR_OUTPUT` engine mechanism as a connectable port. Highest end-user impact: today there is no way to wire `core.if`'s false branch or `core.switch`'s cases at all.
- 8.4 Structured credential forms — **shipped** (7a81e7a..9e26103; plan: `docs/superpowers/plans/2026-09-21-r8r-plan8.4-structured-credential-forms.md`). Depends on 8.2's `credential_types`. Per-credential-type field schema (e.g. `telegramApi` → a labeled "Bot Token" input) replacing the raw-JSON data textarea; type becomes a dropdown filtered to what the node accepts instead of free text.
- 8.5 Per-node retry/timeout/continue-on-fail — **shipped** (spec: `docs/superpowers/specs/2026-09-23-r8r-node-execution-settings-design.md`, plan: `docs/superpowers/plans/2026-09-23-r8r-plan8.5-node-execution-settings.md`). Independent. New `NodeInstance` fields, an engine execution-loop retry/timeout wrapper, and a config-panel UI section. A real production-readiness gap: today nothing is configurable per node beyond raw parameters.
- 8.6 AI Agent UX — independent, but needs its own dedicated brainstorm before implementation. Two parts: (a) small — surface provider/model in the canvas label/panel; (b) large, open design fork — whether/how `ai.agent`'s tools and model config become reusable across multiple agent nodes rather than inlined per-node (deliberate Plan 5 v1 scope decision, revisited here). Least urgent: affects one node type, not the whole canvas.
- 8.7 Background execution — **shipped** (spec: `docs/superpowers/specs/2026-09-23-r8r-background-execution-design.md`, plan: `docs/superpowers/plans/2026-09-23-r8r-plan8.7-background-execution.md`). Follow-up from Plan 8.5's final review. Manual executes (`POST /rest/workflows/:id/execute`), webhook calls, and the telegram poller run the whole workflow inside the request/poll loop, so a node with long retries or timeouts holds the connection for minutes; a client disconnect drops the handler future, cancelling the run mid-node and leaving the execution stuck in `Running` (overlaps 7.5.2), and one retrying telegram run blocks later updates. Run executions in a spawned task and return immediately (live progress already streams over the WebSocket). Pairs naturally with per-retry live events and exponential backoff (both deferred in the 8.5 spec).

---

## Sequencing Notes

- **2 → everything else.** The expression engine and DAG scheduler are load-bearing for nearly every later phase (HTTP Request's parameters, the Agent node's tool schemas, the frontend's node forms all assume expressions exist).
- **3 and 4 are independent of each other** and could be built in either order once Plan 2 lands.
- **5 depends on 4** (the Agent node's tool-exposure story is cleanest once the HTTP Request node and Credential storage already exist as the pattern to follow).
- **6 depends on 2 and 3 at minimum** for a genuinely useful editor (branching nodes and triggers are core to what a workflow editor needs to display); **6.3.1 (live status)** specifically depends on **7.1** (WebSocket push).
- **7** is mostly independent and could be pulled earlier — 7.3 and 7.4 in particular are already-known gaps, not new discovery, so they could be scheduled right after Plan 2 if hardening is prioritized over new features.
- **8.1 has no dependencies** and can land whenever, independent of everything else in Plan 8 or elsewhere. **8.3 and 8.4 depend on 8.2.** **8.5 and 8.6 are independent** of the rest of Plan 8 and of each other — either could be pulled earlier or later without disrupting 8.1-8.4's sequence.
