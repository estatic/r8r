# r8r Per-Node Execution Settings (Plan 8.5) — Design Spec

## 1. Summary

Today nothing about how a node runs is configurable beyond its raw
`parameters`: a node gets exactly one attempt, with no timeout, and a
failure either follows an `error: true` connection (Plan 8.3) or fails
the whole workflow. This spec adds per-node execution settings — retry
with a fixed wait, an opt-in per-attempt timeout, and continue-on-fail —
stored in a nested `settings` object on `NodeInstance`, kept separate
from `parameters` so they never pass through expression resolution or
collide with a node's real parameters. The engine applies them around
`node.execute()`, the API validates them on save, and
`NodeConfigPanel.vue` exposes them in a new "Settings" section.

## 2. Goals / Non-Goals

**Goals:**
- Retry a failing node up to `max_tries` times with a fixed `wait_ms`
  between attempts.
- An opt-in timeout, applied per attempt.
- `continue_on_fail`: when a node finally fails and has no error
  connection, deliver one `{"error": "<msg>"}` item on its main output
  so downstream nodes can react, instead of failing the workflow.
- Full backward compatibility: every previously saved workflow (no
  `settings` key) loads and runs exactly as before.

**Non-Goals:** see §8.

## 3. Data Model

### Rust types (`src/domain.rs`)

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NodeSettings {
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub continue_on_fail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    pub max_tries: u32,
    pub wait_ms: u64,
}

pub struct NodeInstance {
    // ...existing fields (id, node_type, position, parameters, disabled)...
    #[serde(default)]
    pub settings: NodeSettings,
}
```

- `retry: None` means exactly one attempt. A `RetryPolicy` always means
  at least two tries (see §5).
- `timeout_ms: None` means no timeout. This is the default: timeouts are
  opt-in so long-running nodes (`core.wait`, slow `ai.agent` LLM calls)
  that work today are unaffected.
- `#[serde(default)]` on the field plus `Default` on the struct is what
  keeps old workflow JSON (stored in SQLite as serialized `Workflow`)
  deserializing unchanged.

Rejected alternatives: flat fields on `NodeInstance` (clutters the
struct, scatters validation); reserved keys inside `parameters` (would
go through `expr::resolve_parameters` and could collide with a node's
real parameters).

### TypeScript mirror (`frontend/src/types/domain.ts`)

```typescript
export interface RetryPolicy {
  max_tries: number
  wait_ms: number
}

export interface NodeSettings {
  retry: RetryPolicy | null
  timeout_ms: number | null
  continue_on_fail: boolean
}

export interface NodeInstance {
  // ...existing fields (id, node_type, position, parameters, disabled)...
  settings?: NodeSettings
}
```

`settings` is optional on the frontend: the backend always serializes it
after this change, but frontend code treats a missing value as defaults
(e.g. a node created client-side before first save).

## 4. Engine Semantics

### `run_node_with_policy` (`src/engine.rs`)

A new function replaces the direct `node.execute(&ctx).await` call in
`execute_workflow_seeded`:

```rust
async fn run_node_with_policy(
    node: &dyn Node,
    ctx: &NodeExecutionContext,
    settings: &NodeSettings,
) -> Result<NodeOutput, NodeError> {
    let max_tries = settings.retry.as_ref().map_or(1, |r| r.max_tries);
    let wait = Duration::from_millis(settings.retry.as_ref().map_or(0, |r| r.wait_ms));
    let mut attempt = 1;
    loop {
        let result = match settings.timeout_ms {
            Some(ms) => tokio::time::timeout(Duration::from_millis(ms), node.execute(ctx))
                .await
                .unwrap_or_else(|_| Err(NodeError::ExecutionFailed(format!("timed out after {ms}ms")))),
            None => node.execute(ctx).await,
        };
        match result {
            Ok(output) => return Ok(output),
            Err(e) if attempt >= max_tries => {
                return Err(if max_tries > 1 {
                    NodeError::ExecutionFailed(format!("failed after {max_tries} attempts: {e}"))
                } else {
                    e
                });
            }
            Err(_) => {
                tokio::time::sleep(wait).await;
                attempt += 1;
            }
        }
    }
}
```

- The timeout applies **per attempt**, not to the total.
- `wait_ms` elapses between attempts only — never after the final one.
- When wrapping, an `ExecutionFailed` last error contributes its inner
  message rather than its Display, so the `node execution failed: `
  prefix isn't doubled (the code above is simplified on this point; the
  plan has the exact version).
- With default settings this is exactly one untimed `execute()` call,
  i.e. today's behavior.

### Final-failure precedence

After retries are exhausted, evaluated in this order:

1. **Error connection.** The node has an outgoing connection with
   `error: true` → unchanged existing behavior: one `{"error": msg}`
   item into `error_produced`, `produced` gets `vec![]` for the node,
   the run continues.
2. **Continue on fail.** Else, if `settings.continue_on_fail` →
   `produced` gets `vec![vec![error_item]]`: the error item on port 0.
   Downstream nodes wired to other ports receive nothing
   (`outputs.get(port)` returns `None`) and are skipped by the existing
   empty-input rule. The run continues.
3. **Fail.** Otherwise the workflow fails with `node {id} failed: {msg}`
   as today.

In cases 1 and 2 `observer.on_node_errored` is still called, so the UI
shows the node red and the execution record keeps the error while
downstream nodes keep running. No new `ExecutionObserver` events.

The updated match in `execute_workflow_seeded`:

```rust
observer.on_node_started(&node_instance.id).await;
match run_node_with_policy(node, &ctx, &node_instance.settings).await {
    Ok(output) => { /* unchanged */ }
    Err(e) => {
        let has_error_route = workflow
            .connections
            .iter()
            .any(|c| c.from_node == node_instance.id && c.error);
        observer.on_node_errored(&node_instance.id, &e.to_string()).await;
        let error_item = Item {
            json: serde_json::json!({ "error": e.to_string() }),
            binary: serde_json::json!({}),
        };
        if has_error_route {
            error_produced.insert(node_instance.id.clone(), vec![error_item]);
            produced.insert(node_instance.id.clone(), vec![]);
        } else if node_instance.settings.continue_on_fail {
            produced.insert(node_instance.id.clone(), vec![vec![error_item]]);
        } else {
            return Err(anyhow::anyhow!("node {} failed: {e}", node_instance.id));
        }
    }
}
```

### What settings do not apply to

- **Parameter-resolution errors** happen before `execute()` and are
  deterministic; they still fail the workflow, are not retried, and are
  not continued.
- **Disabled nodes, the seeded trigger start node, and empty-input
  skipped nodes** never reach `run_node_with_policy`.
- **`ai.agent` tool calls** via `EngineToolExecutor::call_tool` dispatch
  by node type, not node instance, so there are no instance settings to
  apply.
- **Cancellation:** on timeout the `execute()` future is dropped, which
  cancels async work such as in-flight HTTP requests. `core.code`
  already self-bounds via its QuickJS interrupt handler (2s deadline),
  so a timed-out code node's blocking thread is not leaked.

## 5. API Validation

Axum handlers in `src/api/workflows.rs` return `impl IntoResponse`, so
validation errors are turned into responses explicitly rather than with
`?`. A pure helper, also in `src/api/workflows.rs`:

```rust
fn validate_nodes(nodes: &[NodeInstance]) -> Result<(), String> {
    for node in nodes {
        if let Some(retry) = &node.settings.retry {
            if !(2..=10).contains(&retry.max_tries) {
                return Err(format!("node {}: retry.max_tries must be between 2 and 10", node.id));
            }
            if retry.wait_ms > 60_000 {
                return Err(format!("node {}: retry.wait_ms must be between 0 and 60000", node.id));
            }
        }
        if let Some(timeout) = node.settings.timeout_ms {
            if !(1..=3_600_000).contains(&timeout) {
                return Err(format!("node {}: timeout_ms must be between 1 and 3600000", node.id));
            }
        }
    }
    Ok(())
}
```

`max_tries` starts at 2 because a one-try retry policy is meaningless —
that is `retry: None`. Both `create_workflow` and `update_workflow` call
it first, before touching storage (in `update_workflow`, before the
existing-workflow fetch):

```rust
if let Err(msg) = validate_nodes(&payload.nodes) {
    return (StatusCode::BAD_REQUEST, msg).into_response();
}
```

## 6. Frontend

### `NodeConfigPanel.vue`

The component keeps emitting `update` and keeps its existing
`paramsText`/`credentialId` handling; `apply()` now also includes
`settings` in the emitted node.

New refs: `continueOnFail`, `retryEnabled`, `maxTries`, `waitMs`,
`timeoutMs` (`timeoutMs` is bound to an `<input type="number">`, where
`''` means no timeout).

Loaded in the existing `watch(() => props.node, ...)`, using `??` (not
`||`) so a stored `0` is preserved and `!!` so a missing `settings`
object means retry off:

```typescript
continueOnFail.value = node.settings?.continue_on_fail ?? false
retryEnabled.value = !!node.settings?.retry
maxTries.value = node.settings?.retry?.max_tries ?? 3
waitMs.value = node.settings?.retry?.wait_ms ?? 1000
timeoutMs.value = node.settings?.timeout_ms ?? ''
```

In `apply()`:

```typescript
const settings: NodeSettings = {
  retry: retryEnabled.value ? { max_tries: Number(maxTries.value), wait_ms: Number(waitMs.value) } : null,
  timeout_ms: timeoutMs.value === '' ? null : Number(timeoutMs.value),
  continue_on_fail: continueOnFail.value,
}
```

Client-side range checks mirror §5 and set `error.value` (e.g. "Max
tries must be between 2 and 10.") and return without emitting — the same
path as the existing "Parameters must be valid JSON." error. The backend
400 remains the authority.

Template: a "Settings" section below the existing Disabled checkbox —
"Continue on fail" checkbox; "Retry on fail" checkbox revealing (via
`v-if="retryEnabled"`) "Max tries" (min 2, max 10) and "Wait between
tries (ms)" (min 0, max 60000); "Timeout (ms)" (min 1, empty = none).

## 7. Testing

**Engine (`src/engine.rs`)** — test-only `FlakyNode` (fails its first N
calls, counted with an `AtomicU32`, then returns one item) and
`SlowNode` (`tokio::time::sleep`s):
- Retry succeeds when `max_tries > N`, and the node's output reaches
  downstream.
- Exhausted retries fail the workflow with "failed after 3 attempts";
  `FlakyNode`'s call count equals `max_tries`.
- `#[tokio::test(start_paused = true)]`: a `SlowNode` sleeping 10s with
  `timeout_ms: 100` fails with "timed out after 100ms".
- `start_paused`: 3 tries with `wait_ms: 500` → elapsed
  `tokio::time::Instant` is ≥ 1000ms and < 1500ms (two waits, none
  after the last).
- A timeout without retry makes exactly one attempt.
- `continue_on_fail` delivers `{"error": ...}` to a port-0 downstream
  node and the run succeeds.
- With both an error connection and `continue_on_fail`, the error goes
  to the error-connected node and the port-0 downstream node is skipped.
- With neither, the workflow fails as before.

**Domain (`src/domain.rs`)**
- `NodeInstance` JSON without `settings` deserializes to
  `NodeSettings::default()`.

**Validation (`src/api/workflows.rs` unit tests)**
- `validate_nodes` accepts the boundaries (`max_tries` 2 and 10,
  `wait_ms` 0 and 60000, `timeout_ms` 1 and 3600000) and rejects
  `max_tries` 1 and 11, `wait_ms` 60001, `timeout_ms` 0 and 3600001.

**API (`tests/api_test.rs`)**
- `POST /rest/workflows` with `max_tries: 11` → 400, body names the node
  id.
- `PUT /rest/workflows/:id` with `timeout_ms: 0` → 400, and the stored
  workflow is unchanged.
- Valid settings round-trip through `GET`.

**Frontend (`NodeConfigPanel.spec.ts`)**
- Checking "Retry on fail" reveals the Max tries / Wait inputs.
- Apply emits `update` with the expected `settings` object.
- An empty timeout emits `timeout_ms: null`.
- A node without `settings` loads with retry unchecked and defaults
  3 / 1000.
- Out-of-range max tries shows an error and emits nothing.

## 8. Out of Scope / Deferred

- Exponential backoff.
- A global default timeout.
- Retrying parameter-resolution failures.
- Per-retry live events or attempt counts in the execution view.
- Settings for `ai.agent` tool calls.
- A "pass input through" continue-on-fail mode.
- Canvas badges showing configured settings.
- Retry-from-failed-node at the execution level (roadmap 7.2, separate).
