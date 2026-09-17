# r8r Live Execution Status (Plan 7.1) Design Spec

## 1. Summary

A WebSocket channel that pushes per-node start/finish/error/skip events to
the frontend while a workflow runs, plus incremental persistence of
`node_outputs` as each node completes (instead of only at the very end).
Together these close roadmap §7.1 (`docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`)
and unblock §6.3.1 (live execution status in `WorkflowEditorView`), the
last gap left by Plan 6a/6b.

The workflow-execution request/response model is unchanged: `POST
/rest/workflows/:id/execute` still runs synchronously and returns the
final `Execution`. The WebSocket is a live view *into* that same run, not
a replacement for it — a client that never connects, connects late, or
loses the socket still gets the correct final result via the existing
REST response.

## 2. Goals / Non-Goals

**Goals:**
- The frontend, with a workflow open in the editor, sees each node's
  status (started / success / error / skipped) and output items as a run
  progresses, not only after it finishes.
- This applies to *any* execution of that workflow while the editor is
  open, not only ones the user themselves triggered — a manual click, a
  cron fire, a webhook call, or a Telegram update all produce the same
  events on the same channel.
- A process killed mid-run leaves the last-completed node's output
  persisted, not just whatever the final snapshot would have been
  (closes the persistence half of Foundation's known gap, tracked as
  roadmap §7.1.2).
- The four call sites that currently duplicate identical
  create-run-update boilerplate (`api/workflows.rs`, `api/webhook.rs`,
  `triggers.rs::fire_schedule`, `telegram_poller.rs`) collapse onto one
  shared helper.

**Non-Goals:**
- Execution cancellation, or making `/execute` asynchronous. The
  request/response contract is unchanged; see §3 for why.
- Concurrent branch execution (roadmap §7.6.1) — the engine's DAG walk
  stays sequential. This is what makes event ordering trivial: at most
  one node is ever "in flight" per execution.
- Reconnect/replay semantics for a WebSocket that drops mid-run. If the
  socket disconnects, the live view stalls until the final REST response
  arrives; reconnecting mid-run does not backfill missed events. Given
  this project's read-only "watch a run" use case (not a control
  channel), a stale live view that the final REST response corrects a
  few seconds later is an acceptable v1 gap, not silently swept under the
  rug.
- Reaping executions stuck in `Running` after a crash (roadmap §7.5.2) —
  a separate, already-tracked item. Incremental persistence here makes a
  stuck row's last known state more useful, but doesn't reap it.
- Multi-execution disambiguation beyond "most recently active" (see §6).

## 3. Why the request/response model stays synchronous

The client cannot know an execution's id before the run starts — it's
generated server-side inside the handler that also runs the workflow
synchronously. Making `/execute` return immediately (202 + id, run in a
background task) would let the client subscribe to an exact execution id,
but pulls in a job-tracking model, cancellation questions, and changes to
all four call sites' calling convention for what is, today, a working and
simple contract every test in `tests/api_test.rs` already depends on.

Instead, the client subscribes **per-workflow**, before triggering a run:
open `GET /ws/workflows/:id/executions` when the editor mounts (§6), send
the auth frame, and start listening. By the time any HTTP-triggered
`/execute` call reaches the engine, the socket is already receiving that
workflow's events — no race, no id round-trip needed, and it equally
covers runs the editor's own user didn't initiate (a cron fire while the
editor happens to be open).

## 4. Event Vocabulary & Engine Hook

New trait in `src/engine.rs`, alongside the existing `NodeOutput`/`Item`
types:

```rust
#[async_trait::async_trait]
pub trait ExecutionObserver: Send + Sync {
    async fn on_node_started(&self, node_id: &str);
    async fn on_node_finished(&self, node_id: &str, items: &[Item]);
    async fn on_node_errored(&self, node_id: &str, error: &str);
    async fn on_node_skipped(&self, node_id: &str, items: &[Item]);
}

pub struct NoopObserver;

#[async_trait::async_trait]
impl ExecutionObserver for NoopObserver {
    async fn on_node_started(&self, _node_id: &str) {}
    async fn on_node_finished(&self, _node_id: &str, _items: &[Item]) {}
    async fn on_node_errored(&self, _node_id: &str, _error: &str) {}
    async fn on_node_skipped(&self, _node_id: &str, _items: &[Item]) {}
}
```

`execute_workflow_seeded` gains an `observer: &dyn ExecutionObserver`
parameter and calls it at the exact points it already distinguishes each
outcome:

- **Disabled-passthrough node** (current `if node_instance.disabled`
  branch) → `on_node_skipped` with the passed-through `input_items` (the
  same items this branch already inserts into `produced`).
- **Untaken-branch node** (current `has_incoming_connection &&
  input_items.is_empty()` branch) → `on_node_skipped` with an empty item
  list (matching the empty `Vec` this branch already inserts).
- **Seeded start node** (trigger-item injection branch) → `on_node_started`
  immediately followed by `on_node_finished` (it doesn't call `execute()`,
  but it does produce real output — the UI should see it as having run,
  not been skipped).
- **Normal node execution**: `on_node_started` immediately before
  `node.execute(&ctx).await`, then `on_node_finished` (success) or
  `on_node_errored` (error-routed failure — the existing `has_error_route`
  branch). A **hard failure** (`!has_error_route`, which aborts the whole
  execution via `return Err(...)`) still calls `on_node_errored` first, so
  the UI sees which node killed the run before the caller's `Err` result
  causes the runner (§5) to mark the whole execution `Error`.

`execute_workflow` (the existing no-credentials convenience wrapper, used
by ~14 unit tests in `engine.rs`) passes `&NoopObserver` — those tests are
unaffected by this change.

## 5. Execution Runner — collapsing four duplicated call sites

New module `src/execution_runner.rs`:

```rust
pub async fn run_and_track_execution(
    storage: &std::sync::Arc<dyn Storage>,
    events: &tokio::sync::broadcast::Sender<ExecutionEvent>,
    registry: &NodeRegistry,
    workflow: &Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<Execution>
```

It performs, in order, exactly what each of the four call sites does
today: build the initial `Execution` (`status: Running`), `create_execution`,
construct a `LiveExecutionTracker` (the concrete `ExecutionObserver`, §6),
call `execute_workflow_seeded(workflow, registry, trigger_items,
credentials, &tracker)`, set final `status`/`finished_at` from the
result, do one last `update_execution`, broadcast an `execution_finished`
event, and return the final `Execution` (or propagate the engine's
`anyhow::Error` for callers that need to distinguish "ran and failed" from
"couldn't even start", matching each call site's current error handling —
see §9 for how each of the four call sites' surrounding logic, e.g.
`api/webhook.rs`'s path/method matching, stays in place and only the
duplicated block is replaced by a call to this function).

## 6. Transport

```rust
pub enum ExecutionEventKind {
    NodeStarted { node_id: String },
    NodeFinished { node_id: String, items: Vec<Item> },
    NodeErrored { node_id: String, error: String },
    NodeSkipped { node_id: String },
    ExecutionFinished { status: ExecutionStatus },
}

pub struct ExecutionEvent {
    pub execution_id: Uuid,
    pub workflow_id: Uuid,
    pub kind: ExecutionEventKind,
}
```

`AppState` gains `pub execution_events: tokio::sync::broadcast::Sender<ExecutionEvent>`
(constructed once at startup, alongside `scheduler`/`trigger_registry`).
`LiveExecutionTracker::on_node_*` methods both persist (where applicable,
§7) and `let _ = self.events.send(ExecutionEvent { .. })` — a send with no
receivers is not an error (nothing is listening yet, or ever); this
mirrors how this codebase already treats best-effort broadcast-style work
(e.g. the Telegram poller's own tolerance for a quiet channel).

New route `GET /ws/workflows/:id/executions` (registered in
`src/api/mod.rs` next to the other `/rest/workflows/:id/*` routes,
though it deliberately lives at `/ws/*` — not `/rest/*` — since it's a
different protocol, not a JSON REST resource):

```rust
pub async fn subscribe_executions(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state, workflow_id))
}
```

`handle_socket` first runs the auth handshake (§8), then loops on
`state.execution_events.subscribe()`, forwarding only events where
`event.workflow_id == workflow_id`, serialized as JSON text frames, until
the client disconnects or a send fails.

Requires adding the `ws` feature to the `axum` dependency in `Cargo.toml`
(currently `axum = "0.7"` with no explicit feature list; not pulled in
transitively today — confirmed absent from `Cargo.lock`).

`src/static_files.rs` hardcodes `["rest", "webhook"]` as the set of API
namespaces that get a real `404` (instead of the SPA's `index.html`) for
an unmatched path under them — added in commit `819a1eb` specifically to
fix this class of bug for those two prefixes. `/ws` is a third such
namespace and needs adding to that same list, or an invalid `/ws/*` path
(bad UUID, typo) would regress exactly the bug that commit fixed, just
under a new prefix.

## 7. Incremental Persistence

`LiveExecutionTracker` holds the running `Execution` behind an
`Arc<tokio::sync::Mutex<Execution>>` shared with whatever finalizes it in
§5 (the engine calls observer methods sequentially — never concurrently,
per the sequential-DAG-walk non-goal above — but the mutex keeps the type
honestly `Send + Sync` without relying on that as an implicit invariant
enforced only by convention). On `on_node_finished`/`on_node_errored`/`on_node_skipped`, it inserts
that node's items (a synthesized `{"error": ...}` item for the errored
case, matching the engine's own existing error-item shape; the passed
items directly for finished/skipped) into `node_outputs`, then calls
`storage.update_execution(&execution)` — `status` stays `Running` until
the runner (§5) finalizes it. This is a straightforward extra `UPDATE`
per completed-or-skipped node; at this project's scale (single shared
workspace, SQLite) the added write volume is negligible. Only
`on_node_started` skips persistence — nothing new to record until the
node reaches a terminal state.

## 8. WebSocket Auth

The socket opens unauthenticated. The server waits (5s timeout) for a
single text frame shaped `{"token": "<jwt>"}`, validated with the same
JWT-decode logic `AuthUser` already uses (factored out into a small
shared function both can call, rather than duplicated). Any other first
message, a malformed token, or a timeout closes the socket immediately
with no events ever forwarded. This mirrors the JWT this app already
issues and stores in `localStorage` — the frontend sends the exact same
token it already attaches as a `Bearer` header everywhere else, just as
this first frame instead.

## 9. Call Site Changes

Each of the four sites keeps its own pre-execution logic (workflow
lookup + active check, webhook path/method matching, credential
resolution, trigger-item construction) exactly as today, and replaces
only its `create_execution` → `execute_workflow_seeded` → `update_execution`
block with one call to `execution_runner::run_and_track_execution(...)`:

- `api/workflows.rs::execute_workflow` — `mode: Manual`, `trigger_items:
  None`, resolved credentials as today.
- `api/webhook.rs::handle_webhook` — `mode: Webhook`, `trigger_items:
  Some(vec![trigger_item])`, empty credentials map (unchanged from today
  — webhook-triggered runs don't currently resolve credentials, a
  pre-existing gap out of scope here).
- `triggers.rs::fire_schedule` — `mode: Schedule`, synthesized empty
  trigger item, empty credentials map (same pre-existing gap).
- `telegram_poller.rs`'s per-update loop — `mode: Telegram`, the
  Telegram update as the trigger item, resolved credentials as today.

## 10. Frontend

New `frontend/src/composables/useLiveExecutionSocket.ts`:

- Opens `new WebSocket(...)` to `/ws/workflows/:id/executions` (via
  `location.origin` swapped to `ws:`/`wss:`) when called from
  `WorkflowEditorView`'s `onMounted`, closes it `onUnmounted`.
- Sends the auth frame immediately after `onopen`, reading the token the
  same way `api/client.ts`'s `getToken()` already does.
- Maintains a local `liveExecutionId: string | null`. On any event: if
  `liveExecutionId` is null or differs from the incoming event's
  `execution_id`, adopt the new id (this is the "most recently active
  execution" simplification from §2 — acceptable given this project's
  existing single-shared-workspace trust model, spec §8 of the original
  workflow-automation design). Events for a stale, already-superseded
  execution id are ignored.
- Exposes a reactive `execution: Ref<Execution | null>` that
  `WorkflowEditorView` binds into `ExecutionResultsPanel` exactly where
  the existing `execution` ref from `execute()`'s REST response is bound
  today — `node_started` initializes/updates a skeleton entry,
  `node_finished`/`node_errored`/`node_skipped` fill in `node_outputs`,
  `execution_finished` sets the final `status`. The pre-existing REST
  response from `execute()` still overwrites this ref at the end exactly
  as it does today (Task 10, unchanged) — so a dropped or never-opened
  socket degrades to exactly today's behavior, not a broken one.

`ExecutionResultsPanel.vue` and `useExecutionsStore` (history) are
unchanged — this only adds a second source that can populate the same
`execution` prop earlier/incrementally.

## 11. Error Handling

- WS upgrade on an unknown `workflow_id`: still upgrades (matching this
  project's existing pattern of not needing the workflow to exist yet to
  open a channel for it — no events will ever arrive for an id nothing
  executes) rather than a pre-upgrade 404, since Axum's `WebSocketUpgrade`
  doesn't have a clean way to reject after inspecting async state without
  extra ceremony this feature doesn't need; this is a read-only,
  best-effort channel, not a resource with its own existence to assert.
- A `storage.update_execution` failure inside `on_node_finished` etc. is
  logged (`tracing::error!`) and does **not** abort the run — matches
  every other persistence-failure handling in this codebase's engine/
  runner call sites, which already tolerate a failed final
  `update_execution` without aborting (the workflow's actual side effects
  already happened; failing to record that isn't a reason to also fail
  the response).
- A broadcast `send` with zero receivers is not an error (see §6).

## 12. Testing

- `engine.rs`: extend existing tests (or add new ones) asserting a test
  `ExecutionObserver` spy records the expected sequence of
  started/finished/errored/skipped calls for the existing branching,
  disabled-node, and error-routing test workflows already in that file.
- `execution_runner.rs`: unit tests against the in-memory `SqliteStorage`
  confirming incremental persistence — fetch the execution mid-run isn't
  directly testable without a hook, so this is tested via the *end
  state* (final `node_outputs` populated correctly, same as
  `create_and_get_execution_round_trips`-style existing tests) plus a
  dedicated test that a `Storage` spy/wrapper (same `RecordingStorage`/
  `FailingUpdateStorage` pattern already used in
  `telegram_poller.rs`/`tests/api_test.rs`) observed more than one
  `update_execution` call for a multi-node workflow.
- Every existing integration test in `tests/api_test.rs` drives the
  router in-memory via `tower::ServiceExt::oneshot`, which cannot perform
  a real WebSocket upgrade handshake. WS tests need a real socket, so
  this adds `tokio-tungstenite` as a new dev-dependency and a new test
  pattern for this file: bind the router to a real listener
  (`tokio::net::TcpListener::bind("127.0.0.1:0")` + `axum::serve`, run on
  a spawned task) and connect with `tokio_tungstenite::connect_async`.
  Tests: the expected event sequence (`node_started` →
  `node_finished`/`errored`/`skipped` per node → `execution_finished`)
  arrives on the socket before/while a concurrently-triggered `POST
  .../execute` resolves; an unauthenticated/bad-token/timed-out first
  frame closes the socket without forwarding any event; an event for a
  *different* workflow's execution is never forwarded to a socket
  subscribed to this one.
- Frontend: a `useLiveExecutionSocket.spec.ts` using a mocked
  `WebSocket` (same `vi.stubGlobal` pattern already used for `fetch` in
  every existing store spec) confirming the auth frame is sent first,
  events populate the reactive `execution` ref correctly, and an event
  for a new `execution_id` supersedes a stale one.

## 13. Out of Scope / Deferred

- Execution cancellation (§2).
- Reconnect/backfill on a dropped socket (§2).
- Concurrent branch execution (§2) — if roadmap §7.6.1 ever lands, this
  design's "engine calls observer methods sequentially" assumption (§7)
  needs revisiting.
- Reaping stuck `Running` executions after a crash (roadmap §7.5.2).
- Resolving credentials for webhook/schedule-triggered runs (§9's noted
  pre-existing gaps) — unrelated to this feature, not touched here.
