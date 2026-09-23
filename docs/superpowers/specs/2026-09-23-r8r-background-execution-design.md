# r8r Background Execution (Plan 8.7) — Design Spec

## 1. Summary

Every run today is driven by `execution_runner::run_and_track_execution`,
which creates the `Running` execution row, runs the whole workflow, and
persists the final status — all inside the caller's future. For the
manual execute endpoint and webhooks that future is an axum handler, so a
client/proxy disconnect drops it and cancels the run mid-node, leaving the
row stuck in `Running`. The telegram poller awaits each run inside its
polling loop, so one slow run blocks every later update. Plan 8.5's
retries and timeouts make long runs routine, turning this latent problem
into a real one (roadmap 8.7).

This spec splits "start" from "finish": the run always executes in a
spawned task, and each caller chooses whether to wait for it.

## 2. Goals / Non-Goals

**Goals:**
- No run is ever cancelled by a caller going away.
- Manual execute returns immediately; the editor follows progress over
  the existing WebSocket, with a polling fallback.
- Webhooks keep today's response by default, with an opt-in immediate
  202.
- Telegram polling never waits on a run; updates from the same chat
  still run in arrival order; different chats run in parallel.

**Non-Goals:** see §8.

## 3. Runner (`src/execution_runner.rs`)

```rust
pub async fn start_execution(
    storage: Arc<dyn Storage>,
    events: broadcast::Sender<ExecutionEvent>,
    registry: Arc<NodeRegistry>,
    workflow: Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<(Execution, tokio::task::JoinHandle<Execution>)>
```

- Creates and persists the `Running` execution row before returning (a
  persistence failure is returned as `Err`, exactly as today), so the
  caller always has a valid execution id.
- Moves everything else — `LiveExecutionTracker`, the
  `execute_workflow_seeded` call, final status, `update_execution`, and
  the `ExecutionFinished` event — into a `tokio::spawn`ed task whose
  output is the final `Execution`. Arguments are owned (the `Arc`s,
  `Workflow`, credentials map) because the task outlives the caller.
- Dropping the `JoinHandle` detaches the task; it keeps running.

`run_and_track_execution` keeps its signature and becomes a thin wrapper:
clone its borrowed arguments, call `start_execution`, await the handle.
A `JoinError` (the task panicked) maps to `Err(anyhow!(...))`. Schedule
triggers and existing tests keep using it unchanged.

## 4. Manual Execute

`POST /rest/workflows/:id/execute` (`api/workflows.rs::execute_workflow`)
calls `start_execution`, drops the handle, and responds **202 Accepted**
with the `Running` `Execution` JSON (same shape as today, `status:
"Running"`, empty `node_outputs`, `finished_at: null`). Workflow-not-found
(404) and credential-resolution failure (400) are unchanged and happen
before anything starts.

### Frontend (`frontend/src/views/WorkflowEditorView.vue`)

`execution` is already `live.execution` (the WebSocket composable's ref).
`execute()` changes to:

1. POST; on success read the returned id as `startedId`.
2. If `execution.value?.id !== startedId`, set `execution.value` to the
   response. If it is already that id, the socket has delivered events
   first — keep the socket's (richer) copy instead of overwriting it.
3. `executing` stays `true` until the execution with id `startedId` has
   a status other than `Running`, observed via a `watch` on
   `execution.value`.
4. Fallback: while waiting, `GET /rest/executions/:startedId` every 2s;
   when it returns a non-`Running` status, assign it to
   `execution.value` (covers a dropped or never-opened socket). The
   interval is cleared when the run finishes and in `onUnmounted`.

The socket composable's comment that "the final REST response from
execute() is authoritative" is updated: the polling fallback is now the
backstop.

## 5. Webhook

`core.webhook` gains an optional `respond` parameter:

- `"when_finished"` (default, also used when absent or unrecognised):
  the handler awaits the `JoinHandle` and responds exactly as today —
  200 with the final `Execution`, or 500 when its status is `Error`. If
  the caller disconnects, only the wait is dropped; the run completes.
- `"immediately"`: respond **202** with `{"execution_id": "<uuid>"}`
  without waiting.

A `JoinError` on the awaited handle (task panicked) returns 500, like a
persistence failure. The node's `description()` mentions the option; no
other webhook matching or trigger-item behaviour changes.

## 6. Telegram (`src/telegram_poller.rs`)

The per-update `run_and_track_execution(...).await` in
`poll_telegram_updates` is replaced with routing into per-chat queues:

- `fn chat_key(update: &serde_json::Value) -> Option<i64>` reads
  `chat.id` from the first present of `message`, `edited_message`,
  `channel_post`, `edited_channel_post`, `callback_query.message`.
  Updates with no chat (`None`) share one queue.
- The poller owns `HashMap<Option<i64>, mpsc::UnboundedSender<Job>>`,
  where `Job` carries the trigger `Item` plus the batch's `Workflow` and
  credentials snapshot (so a job runs against the workflow as it was when
  polled, same as today).
- For each update: look up the chat's sender; if missing or closed
  (`send` fails), spawn a new worker and retry the send once on its
  sender.
- A worker loops: `tokio::time::timeout(5 min, rx.recv())`; on a job,
  `start_execution(...)` then await its handle before taking the next
  job (per-chat order); on timeout or a closed channel, exit. Errors are
  logged exactly as the poller logs them today.
- When the poller stops (workflow deactivated), dropping the map closes
  every channel; workers finish the jobs already queued, then exit.

Trade-off (accepted): the update offset now advances before queued runs
finish, so updates still queued when the process crashes are lost.
Previously a crash mid-run caused Telegram to redeliver the batch.

## 7. Testing

**Runner** (`execution_runner.rs` tests)
- `start_execution` returns while the stored row is still `Running`;
  awaiting the handle yields `Success` and the stored row matches.
- Dropping the handle (and the caller's future) does not cancel the run:
  with a test node that sleeps briefly, the stored row still reaches a
  final status.
- `run_and_track_execution` behaviour is unchanged (existing tests stay
  green).

**API** (`tests/api_test.rs`)
- `POST .../execute` returns 202 with `status: "Running"`; polling
  `GET /rest/executions/:id` reaches `Success` with the expected outputs.
- Webhook with no `respond` parameter: unchanged 200 body/500 on error
  (existing tests stay green).
- Webhook with `respond: "immediately"`: 202 with an `execution_id`
  whose execution later reaches a final status.

**Telegram** (`telegram_poller.rs` tests)
- `chat_key` for each supported update shape, and `None` for an update
  without a chat.
- Two updates from the same chat run in order even when the first is
  slow (execution start/finish timestamps or an ordered observer).
- An update from a second chat finishes while the first chat's slow run
  is still in progress.

**Frontend** (`WorkflowEditorView` spec or a focused test of the execute
logic)
- After a 202, the button shows "Running…" until an `execution_finished`
  event for that id arrives.
- A POST response arriving after socket events for the same id does not
  wipe the socket's node outputs.
- With no socket events, the 2s fallback fetch resolves the run.

## 8. Out of Scope / Deferred

- Reaping executions left `Running` by a process crash or restart
  (roadmap 7.5.2).
- A global concurrency limit or worker pool for runs.
- Per-retry live events and exponential backoff (deferred from 8.5).
- Persisting queued Telegram updates across restarts.
- Cancelling a running execution from the UI.
