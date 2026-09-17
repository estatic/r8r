# r8r Live Execution Status (Plan 7.1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Push per-node start/finished/errored/skipped events over a WebSocket
while a workflow runs, and persist `node_outputs` incrementally (per node) instead
of only at the very end — closing roadmap §7.1 and unblocking §6.3.1 (live
execution status in the frontend editor).

**Architecture:** The engine (`execute_workflow_seeded`) gains an
`ExecutionObserver` trait hook called at each node's start/finish/error/skip.
A new `execution_runner` module provides one shared `run_and_track_execution`
helper — used by all four places that currently run a workflow — whose
`LiveExecutionTracker` observer both persists incrementally and broadcasts
onto a single `tokio::sync::broadcast` channel in `AppState`. A new
`GET /ws/workflows/:id/executions` route forwards matching broadcast events
to any connected, authenticated WebSocket client. The frontend opens that
socket when the editor mounts and uses it to fill in `ExecutionResultsPanel`
live; the existing synchronous `POST .../execute` response is unchanged and
still finalizes the same `execution` ref, so a client that never connects
degrades to exactly today's behavior.

**Tech Stack:** Rust/Axum/SQLite backend (`src/`), Vue3/TS/Pinia frontend
(`frontend/src/`). New: axum's `ws` feature, `tokio-tungstenite` (dev-only,
for a real-socket integration test).

**Spec:** `docs/superpowers/specs/2026-09-17-r8r-plan7-live-execution-design.md`
— reachable, read directly before writing this plan; authoritative.

## Global Constraints

- **Engine stays sequential.** `execute_workflow_seeded` walks nodes one at a
  time (no concurrent branches yet — roadmap §7.6.1 is separate and
  deferred). This plan relies on that: at most one node is ever "in flight"
  per execution, so event ordering needs no extra coordination.
- **No cancellation, no async `/execute`.** `POST /rest/workflows/:id/execute`
  keeps its existing synchronous request/response contract exactly as every
  current test in `tests/api_test.rs` already depends on. The WebSocket is a
  read-only live view into that same call, not a replacement for it.
- **One global broadcast channel**, not a per-workflow registry — lives on
  `AppState` as `execution_events: tokio::sync::broadcast::Sender<execution_runner::ExecutionEvent>`,
  filtered per-connection by `workflow_id`.
- **WS auth is a first-frame handshake**: `{"token": "<jwt>"}` as the first
  text message, validated via the existing `crate::auth::verify_token`
  (already a standalone function `AuthUser` itself calls — no new
  factoring-out needed). No frame within 5s, or an invalid one, closes the
  socket with nothing ever forwarded.
- **A `storage.update_execution` failure during a run never aborts the run**
  — only logged (`tracing::error!`), matching how every other
  persistence-failure path in this codebase's execution call sites already
  behaves.
- **`run_and_track_execution`'s `Err` return means only "couldn't even
  create the execution row"** (a storage failure) — an engine failure during
  the run itself is captured as `ExecutionStatus::Error` on the returned
  `Execution`, never as a Rust `Err`, exactly matching how every one of the
  four existing call sites already treats it today (they only ever
  early-return on the `create_execution` failure; the engine's own
  `execute_workflow_seeded` failure is always folded into `execution.status`).

---

### Task 1: Engine — `ExecutionObserver` hook

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Produces: `pub trait ExecutionObserver: Send + Sync` with
  `on_node_started(&self, node_id: &str)`,
  `on_node_finished(&self, node_id: &str, items: &[Item])`,
  `on_node_errored(&self, node_id: &str, error: &str)`,
  `on_node_skipped(&self, node_id: &str, items: &[Item])` (all `async`, via
  `#[async_trait::async_trait]`); `pub struct NoopObserver` implementing it
  with empty bodies. `execute_workflow_seeded` gains a fifth parameter
  `observer: &dyn ExecutionObserver`; `execute_workflow` (the existing
  no-credentials wrapper used by ~14 unit tests in this file) passes
  `&NoopObserver`, so those tests need no changes.
- Consumes: nothing new — this task only touches `src/engine.rs`.

- [ ] **Step 1: Write the failing spy test**

Add to the existing `#[cfg(test)] mod tests` block in `src/engine.rs`, after
the `disabled_node_is_skipped_as_passthrough` test:

```rust
    struct SpyObserver {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl SpyObserver {
        fn new() -> Self {
            Self { calls: std::sync::Mutex::new(Vec::new()) }
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl ExecutionObserver for SpyObserver {
        async fn on_node_started(&self, node_id: &str) {
            self.calls.lock().unwrap().push(format!("started:{node_id}"));
        }
        async fn on_node_finished(&self, node_id: &str, items: &[Item]) {
            self.calls.lock().unwrap().push(format!("finished:{node_id}:{}", items.len()));
        }
        async fn on_node_errored(&self, node_id: &str, error: &str) {
            self.calls.lock().unwrap().push(format!("errored:{node_id}:{error}"));
        }
        async fn on_node_skipped(&self, node_id: &str, items: &[Item]) {
            self.calls.lock().unwrap().push(format!("skipped:{node_id}:{}", items.len()));
        }
    }

    #[tokio::test]
    async fn observer_sees_started_then_finished_for_a_normal_run() {
        // trigger -> set_disabled (disabled passthrough) -> set_final, same
        // shape as disabled_node_is_skipped_as_passthrough above.
        let mut wf = linear_workflow();
        wf.nodes[1].id = "set_disabled".into();
        wf.nodes[1].parameters = serde_json::json!({"fields": {"skipped": "yes"}});
        wf.nodes[1].disabled = true;
        wf.connections[0].to_node = "set_disabled".into();
        wf.nodes.push(NodeInstance {
            id: "set_final".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"final": "yes"}}),
            disabled: false,
        });
        wf.connections.push(Connection {
            from_node: "set_disabled".into(),
            from_output: 0,
            to_node: "set_final".into(),
            to_input: 0,
        });

        let spy = SpyObserver::new();
        execute_workflow_seeded(&wf, &registry(), None, &HashMap::new(), &spy).await.unwrap();

        assert_eq!(
            spy.calls(),
            vec![
                "started:trigger".to_string(),
                "finished:trigger:1".to_string(),
                "skipped:set_disabled:1".to_string(),
                "started:set_final".to_string(),
                "finished:set_final:1".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn observer_sees_errored_for_both_routed_and_hard_failures() {
        // Routed: trigger -> set1 (fails, routed to error_handler).
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "test.alwaysFails".into();
        wf.nodes.push(NodeInstance {
            id: "error_handler".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"handled": true}}),
            disabled: false,
        });
        wf.connections.push(Connection {
            from_node: "set1".into(),
            from_output: crate::node::ERROR_OUTPUT,
            to_node: "error_handler".into(),
            to_input: 0,
        });

        let spy = SpyObserver::new();
        execute_workflow_seeded(&wf, &registry_with_failing_node(), None, &HashMap::new(), &spy)
            .await
            .unwrap();

        let calls = spy.calls();
        assert_eq!(calls[0], "started:trigger");
        assert_eq!(calls[1], "finished:trigger:1");
        assert_eq!(calls[2], "started:set1");
        assert!(calls[3].starts_with("errored:set1:"));
        assert_eq!(calls[4], "started:error_handler");
        assert_eq!(calls[5], "finished:error_handler:1");

        // Hard failure (no error route): still reports errored before the
        // engine aborts the whole run.
        let mut hard_wf = linear_workflow();
        hard_wf.nodes[1].node_type = "test.alwaysFails".into();
        let spy2 = SpyObserver::new();
        let result =
            execute_workflow_seeded(&hard_wf, &registry_with_failing_node(), None, &HashMap::new(), &spy2).await;
        assert!(result.is_err());
        assert_eq!(spy2.calls()[0], "started:trigger");
        assert_eq!(spy2.calls()[1], "finished:trigger:1");
        assert_eq!(spy2.calls()[2], "started:set1");
        assert!(spy2.calls()[3].starts_with("errored:set1:"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib engine::tests::observer_sees`
Expected: compile error — `execute_workflow_seeded` takes 4 arguments, not
5, and `ExecutionObserver` doesn't exist yet.

- [ ] **Step 3: Add the trait, `NoopObserver`, and the observer parameter**

In `src/engine.rs`, add after the imports at the top of the file:

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

Change `execute_workflow`'s body to pass the no-op observer:

```rust
pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None, &HashMap::new(), &NoopObserver).await
}
```

Change `execute_workflow_seeded`'s signature to add the parameter:

```rust
pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &NodeRegistry,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<uuid::Uuid, serde_json::Value>,
    observer: &dyn ExecutionObserver,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
```

- [ ] **Step 4: Wire the four hook points**

Still in `execute_workflow_seeded`, change the disabled-passthrough branch:

```rust
        if node_instance.disabled {
            observer.on_node_skipped(&node_instance.id, &input_items).await;
            produced.insert(node_instance.id.clone(), vec![input_items]);
            continue;
        }
```

Change the seeded-start-node branch:

```rust
        if let (Some(items), Some(start)) = (&trigger_items, &start_id) {
            if &node_instance.id == start {
                observer.on_node_started(&node_instance.id).await;
                observer.on_node_finished(&node_instance.id, items).await;
                produced.insert(node_instance.id.clone(), vec![items.clone()]);
                continue;
            }
        }
```

Change the untaken-branch (empty-input) skip:

```rust
        let has_incoming_connection = workflow.connections.iter().any(|c| c.to_node == node_instance.id);
        if has_incoming_connection && input_items.is_empty() {
            observer.on_node_skipped(&node_instance.id, &[]).await;
            produced.insert(node_instance.id.clone(), vec![]);
            continue;
        }
```

Change the normal-execution block — add `on_node_started` before
`node.execute`, and `on_node_finished`/`on_node_errored` in the match arms:

```rust
        let ctx = NodeExecutionContext {
            parameters,
            input_items,
            credentials: credentials.clone(),
        };
        observer.on_node_started(&node_instance.id).await;
        match node.execute(&ctx).await {
            Ok(output) => {
                let primary = output.first().cloned().unwrap_or_default();
                observer.on_node_finished(&node_instance.id, &primary).await;
                produced.insert(node_instance.id.clone(), output);
            }
            Err(e) => {
                let has_error_route = workflow.connections.iter().any(|c| {
                    c.from_node == node_instance.id && c.from_output == crate::node::ERROR_OUTPUT
                });
                observer.on_node_errored(&node_instance.id, &e.to_string()).await;
                if has_error_route {
                    error_produced.insert(
                        node_instance.id.clone(),
                        vec![Item {
                            json: serde_json::json!({ "error": e.to_string() }),
                            binary: serde_json::json!({}),
                        }],
                    );
                    produced.insert(node_instance.id.clone(), vec![]);
                } else {
                    return Err(anyhow::anyhow!("node {} failed: {e}", node_instance.id));
                }
            }
        }
```

- [ ] **Step 5: Fix the remaining callers so the crate compiles**

Every other caller of `execute_workflow_seeded` in the crate now needs a
5th argument. For this task only, pass `&crate::engine::NoopObserver` at
each — they'll be replaced with the real tracker in Task 3:

- `src/api/workflows.rs`: `crate::engine::execute_workflow_seeded(&workflow, &state.registry, None, &credentials, &crate::engine::NoopObserver).await`
- `src/api/webhook.rs`: `crate::engine::execute_workflow_seeded(&workflow, &state.registry, Some(vec![trigger_item]), &std::collections::HashMap::new(), &crate::engine::NoopObserver).await`
- `src/triggers.rs`: `crate::engine::execute_workflow_seeded(&workflow, &registry, Some(trigger_items), &std::collections::HashMap::new(), &crate::engine::NoopObserver).await`
- `src/telegram_poller.rs`: `crate::engine::execute_workflow_seeded(&current_workflow, &registry, Some(vec![trigger_item]), &credentials, &crate::engine::NoopObserver).await`

Also fix the two other direct callers already inside `src/engine.rs`'s own
test module — `execute_workflow_seeded_injects_trigger_items_as_start_node_output`
and `execute_workflow_seeded_with_none_seed_behaves_exactly_as_before` — by
adding `, &NoopObserver` as their final argument.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test --lib engine::`
Expected: all pass, including the two new spy tests and every pre-existing
`engine.rs` test unchanged.

Run: `cargo build`
Expected: the whole crate compiles (Step 5's four production call sites now
build against the new signature).

- [ ] **Step 7: Commit**

```bash
git add src/engine.rs src/api/workflows.rs src/api/webhook.rs src/triggers.rs src/telegram_poller.rs
git commit -m "feat: add ExecutionObserver hook to the execution engine"
```

---

### Task 2: `execution_runner` module

**Files:**
- Create: `src/execution_runner.rs`
- Modify: `src/lib.rs` (register the module)

**Interfaces:**
- Produces:
  ```rust
  pub enum ExecutionEventKind {
      NodeStarted { node_id: String },
      NodeFinished { node_id: String, items: Vec<crate::domain::Item> },
      NodeErrored { node_id: String, error: String },
      NodeSkipped { node_id: String, items: Vec<crate::domain::Item> },
      ExecutionFinished { status: crate::domain::ExecutionStatus },
  }
  pub struct ExecutionEvent {
      pub execution_id: uuid::Uuid,
      pub workflow_id: uuid::Uuid,
      pub kind: ExecutionEventKind,
  }
  pub async fn run_and_track_execution(
      storage: &std::sync::Arc<dyn crate::storage::Storage>,
      events: &tokio::sync::broadcast::Sender<ExecutionEvent>,
      registry: &crate::node::NodeRegistry,
      workflow: &crate::domain::Workflow,
      mode: crate::domain::ExecutionMode,
      trigger_items: Option<Vec<crate::domain::Item>>,
      credentials: &std::collections::HashMap<uuid::Uuid, serde_json::Value>,
  ) -> anyhow::Result<crate::domain::Execution>;
  ```
- Consumes: `crate::engine::{execute_workflow_seeded, ExecutionObserver}` (Task 1),
  `crate::storage::Storage` (existing).

- [ ] **Step 1: Write the failing tests**

Create `src/execution_runner.rs` with just the test module first (the
`run_and_track_execution` and `ExecutionEvent`/`ExecutionEventKind` items it
references don't exist yet, so this won't compile):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Connection, Execution, ExecutionMode, ExecutionStatus, NodeInstance, Workflow};
    use crate::node::NodeRegistry;
    use crate::storage::sqlite::SqliteStorage;
    use crate::storage::Storage;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r
    }

    fn linear_workflow() -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "linear".into(),
            active: false,
            nodes: vec![
                NodeInstance {
                    id: "trigger".into(),
                    node_type: "core.manualTrigger".into(),
                    position: (0.0, 0.0),
                    parameters: serde_json::json!({}),
                    disabled: false,
                },
                NodeInstance {
                    id: "set1".into(),
                    node_type: "core.set".into(),
                    position: (1.0, 0.0),
                    parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
                    disabled: false,
                },
            ],
            connections: vec![Connection {
                from_node: "trigger".into(),
                from_output: 0,
                to_node: "set1".into(),
                to_input: 0,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    struct CountingStorage {
        inner: SqliteStorage,
        update_execution_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Storage for CountingStorage {
        async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
            self.inner.create_workflow(workflow).await
        }
        async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
            self.inner.update_workflow(workflow).await
        }
        async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()> {
            self.inner.delete_workflow(id).await
        }
        async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>> {
            self.inner.get_workflow(id).await
        }
        async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
            self.inner.list_workflows().await
        }
        async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()> {
            self.inner.create_execution(execution).await
        }
        async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()> {
            self.update_execution_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.update_execution(execution).await
        }
        async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>> {
            self.inner.get_execution(id).await
        }
        async fn list_executions_for_workflow(&self, workflow_id: Uuid, limit: i64) -> anyhow::Result<Vec<Execution>> {
            self.inner.list_executions_for_workflow(workflow_id, limit).await
        }
        async fn create_user(&self, user: &crate::domain::User) -> anyhow::Result<()> {
            self.inner.create_user(user).await
        }
        async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<crate::domain::User>> {
            self.inner.get_user_by_email(email).await
        }
        async fn create_credential(&self, credential: &crate::domain::Credential) -> anyhow::Result<()> {
            self.inner.create_credential(credential).await
        }
        async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<crate::domain::Credential>> {
            self.inner.get_credential(id).await
        }
        async fn list_credentials(&self) -> anyhow::Result<Vec<crate::domain::CredentialSummary>> {
            self.inner.list_credentials().await
        }
    }

    #[tokio::test]
    async fn persists_node_outputs_incrementally_not_only_at_the_end() {
        let inner = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
        let wf = linear_workflow();
        inner.create_workflow(&wf).await.unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let storage: Arc<dyn Storage> = Arc::new(CountingStorage { inner, update_execution_calls: counter.clone() });
        let (events, _rx) = tokio::sync::broadcast::channel(16);

        run_and_track_execution(&storage, &events, &registry(), &wf, ExecutionMode::Manual, None, &HashMap::new())
            .await
            .unwrap();

        // trigger finishes (1 update) + set1 finishes (1 update) + the
        // runner's own final update = 3, not 1 -- proving this isn't only
        // writing once at the end.
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn returns_the_final_execution_with_success_status_and_full_outputs() {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap());
        let wf = linear_workflow();
        storage.create_workflow(&wf).await.unwrap();
        let (events, _rx) = tokio::sync::broadcast::channel(16);

        let execution =
            run_and_track_execution(&storage, &events, &registry(), &wf, ExecutionMode::Manual, None, &HashMap::new())
                .await
                .unwrap();

        assert_eq!(execution.status, ExecutionStatus::Success);
        assert_eq!(execution.node_outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
        let persisted = storage.get_execution(execution.id).await.unwrap().unwrap();
        assert_eq!(persisted.status, ExecutionStatus::Success);
        assert_eq!(persisted.node_outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[tokio::test]
    async fn broadcasts_node_and_execution_events_in_order() {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap());
        let wf = linear_workflow();
        storage.create_workflow(&wf).await.unwrap();
        let (events, mut rx) = tokio::sync::broadcast::channel(16);

        run_and_track_execution(&storage, &events, &registry(), &wf, ExecutionMode::Manual, None, &HashMap::new())
            .await
            .unwrap();

        let mut kinds = Vec::new();
        while let Ok(event) = rx.try_recv() {
            kinds.push(event.kind);
        }
        assert!(matches!(&kinds[0], ExecutionEventKind::NodeStarted { node_id } if node_id == "trigger"));
        assert!(matches!(
            kinds.last().unwrap(),
            ExecutionEventKind::ExecutionFinished { status: ExecutionStatus::Success }
        ));
    }
}
```

- [ ] **Step 2: Register the module and run the tests to verify they fail**

In `src/lib.rs`, add (alphabetically, between `engine` and `expr`):

```rust
pub mod execution_runner;
```

Run: `cargo test --lib execution_runner::`
Expected: compile errors — `run_and_track_execution`, `ExecutionEventKind`,
`ExecutionEvent` don't exist yet.

- [ ] **Step 3: Implement `ExecutionEvent`/`ExecutionEventKind` and `LiveExecutionTracker`**

At the top of `src/execution_runner.rs` (above the `#[cfg(test)]` module):

```rust
use crate::domain::{Execution, ExecutionMode, ExecutionStatus, Item, Workflow};
use crate::engine::ExecutionObserver;
use crate::node::NodeRegistry;
use crate::storage::Storage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExecutionEventKind {
    NodeStarted { node_id: String },
    NodeFinished { node_id: String, items: Vec<Item> },
    NodeErrored { node_id: String, error: String },
    NodeSkipped { node_id: String, items: Vec<Item> },
    ExecutionFinished { status: ExecutionStatus },
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionEvent {
    pub execution_id: Uuid,
    pub workflow_id: Uuid,
    #[serde(flatten)]
    pub kind: ExecutionEventKind,
}

/// The concrete `ExecutionObserver` used for every real (non-test) run:
/// persists each node's output to `storage` as it completes, and broadcasts
/// the same information as an `ExecutionEvent`. Holds the running
/// `Execution` behind a `tokio::sync::Mutex` since `ExecutionObserver`
/// methods take `&self` (the engine calls them sequentially, never
/// concurrently, but this keeps that an enforced guarantee rather than an
/// unchecked convention).
struct LiveExecutionTracker {
    execution_id: Uuid,
    workflow_id: Uuid,
    execution: Mutex<Execution>,
    storage: Arc<dyn Storage>,
    events: broadcast::Sender<ExecutionEvent>,
}

impl LiveExecutionTracker {
    fn new(execution: Execution, storage: Arc<dyn Storage>, events: broadcast::Sender<ExecutionEvent>) -> Self {
        Self {
            execution_id: execution.id,
            workflow_id: execution.workflow_id,
            execution: Mutex::new(execution),
            storage,
            events,
        }
    }

    async fn persist_node_output(&self, node_id: &str, items: Vec<Item>) {
        let mut execution = self.execution.lock().await;
        execution.node_outputs.insert(node_id.to_string(), items);
        if let Err(e) = self.storage.update_execution(&execution).await {
            tracing::error!(error = %e, execution_id = %execution.id, node_id, "failed to persist incremental execution update");
        }
    }

    fn emit(&self, kind: ExecutionEventKind) {
        let _ = self.events.send(ExecutionEvent {
            execution_id: self.execution_id,
            workflow_id: self.workflow_id,
            kind,
        });
    }

    async fn into_execution(self) -> Execution {
        self.execution.into_inner()
    }
}

#[async_trait::async_trait]
impl ExecutionObserver for LiveExecutionTracker {
    async fn on_node_started(&self, node_id: &str) {
        self.emit(ExecutionEventKind::NodeStarted { node_id: node_id.to_string() });
    }
    async fn on_node_finished(&self, node_id: &str, items: &[Item]) {
        self.persist_node_output(node_id, items.to_vec()).await;
        self.emit(ExecutionEventKind::NodeFinished { node_id: node_id.to_string(), items: items.to_vec() });
    }
    async fn on_node_errored(&self, node_id: &str, error: &str) {
        let error_item = Item { json: serde_json::json!({ "error": error }), binary: serde_json::json!({}) };
        self.persist_node_output(node_id, vec![error_item]).await;
        self.emit(ExecutionEventKind::NodeErrored { node_id: node_id.to_string(), error: error.to_string() });
    }
    async fn on_node_skipped(&self, node_id: &str, items: &[Item]) {
        self.persist_node_output(node_id, items.to_vec()).await;
        self.emit(ExecutionEventKind::NodeSkipped { node_id: node_id.to_string(), items: items.to_vec() });
    }
}
```

- [ ] **Step 4: Implement `run_and_track_execution`**

Add, still above the test module:

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
    let execution = Execution {
        id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Running,
        mode,
        node_outputs: Default::default(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    };
    storage.create_execution(&execution).await?;

    let tracker = LiveExecutionTracker::new(execution, storage.clone(), events.clone());
    let result = crate::engine::execute_workflow_seeded(workflow, registry, trigger_items, credentials, &tracker).await;

    let mut final_execution = tracker.into_execution().await;
    match &result {
        Ok(outputs) => {
            final_execution.status = ExecutionStatus::Success;
            final_execution.node_outputs = outputs.clone();
        }
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "workflow execution failed");
            final_execution.status = ExecutionStatus::Error;
        }
    }
    final_execution.finished_at = Some(chrono::Utc::now());
    if let Err(e) = storage.update_execution(&final_execution).await {
        tracing::error!(error = %e, "failed to persist final execution result");
    }

    let _ = events.send(ExecutionEvent {
        execution_id: final_execution.id,
        workflow_id: workflow.id,
        kind: ExecutionEventKind::ExecutionFinished { status: final_execution.status.clone() },
    });

    Ok(final_execution)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib execution_runner::`
Expected: all three tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/execution_runner.rs src/lib.rs
git commit -m "feat: add execution_runner module (incremental persistence + event broadcast)"
```

---

### Task 3: Wire `AppState` and migrate all four call sites

**Files:**
- Modify: `src/state.rs`
- Modify: `src/main.rs`
- Modify: `src/api/workflows.rs`
- Modify: `src/api/webhook.rs`
- Modify: `src/triggers.rs`
- Modify: `src/telegram_poller.rs`
- Modify: `tests/api_test.rs`
- Modify: `tests/health_test.rs`

**Interfaces:**
- Consumes: `execution_runner::{run_and_track_execution, ExecutionEvent}` (Task 2).
- Produces: `AppState.execution_events: tokio::sync::broadcast::Sender<execution_runner::ExecutionEvent>`,
  available to every handler/trigger that already receives `AppState` (or an
  `Arc`/clone derived from it).

- [ ] **Step 1: Add the field to `AppState` and every construction site**

In `src/state.rs`:

```rust
use crate::execution_runner::ExecutionEvent;
use crate::node::NodeRegistry;
use crate::scheduler::Scheduler;
use crate::storage::Storage;
use crate::trigger_registry::TriggerRegistry;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub registry: Arc<NodeRegistry>,
    pub jwt_secret: String,
    pub scheduler: Arc<Scheduler>,
    pub trigger_registry: Arc<TriggerRegistry>,
    pub execution_events: tokio::sync::broadcast::Sender<ExecutionEvent>,
}
```

In `src/main.rs`, add the field when constructing `state`:

```rust
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
        scheduler: Arc::new(scheduler),
        trigger_registry: Arc::new(TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(256).0,
    };
```

In `tests/health_test.rs`'s `test_app()`, add the same field:

```rust
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
    };
```

In `tests/api_test.rs`, add the field in both `test_state()` and
`test_app_with_failing_update()`:

```rust
async fn test_state() -> AppState {
    let storage = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
    }
}
```

(and the identical field addition in `test_app_with_failing_update()`'s
`AppState { .. }` literal, around line 112).

In `src/triggers.rs`'s own test module, add the same field to its
`test_state()`:

```rust
    async fn test_state() -> AppState {
        let storage = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        AppState {
            storage: Arc::new(storage),
            registry: Arc::new(registry),
            jwt_secret: "test-secret".into(),
            scheduler: Arc::new(crate::scheduler::Scheduler::new().await.unwrap()),
            trigger_registry: Arc::new(crate::trigger_registry::TriggerRegistry::new()),
            execution_events: tokio::sync::broadcast::channel(16).0,
        }
    }
```

- [ ] **Step 2: Run the tests to verify the crate compiles**

Run: `cargo build && cargo test --lib state`
Expected: compiles. (No behavior changed yet — every call site still uses
`NoopObserver` from Task 1's Step 5.)

- [ ] **Step 3: Migrate `src/api/workflows.rs::execute_workflow`**

Replace the whole body from `let mut execution = Execution { ... }` through
`Json(execution).into_response()` with:

```rust
    let credentials = match crate::credentials::resolve_credentials_for_workflow(state.storage.as_ref(), &workflow).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "failed to resolve workflow credentials");
            return (StatusCode::BAD_REQUEST, format!("credential resolution failed: {e}")).into_response();
        }
    };

    match crate::execution_runner::run_and_track_execution(
        &state.storage,
        &state.execution_events,
        &state.registry,
        &workflow,
        ExecutionMode::Manual,
        None,
        &credentials,
    )
    .await
    {
        Ok(execution) => Json(execution).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to persist new execution");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

(The `credentials` resolution block already existed immediately above this
in the file — keep it in place; only the block after it changes.)

`Execution` and `ExecutionStatus` were only ever used inside the block just
replaced — `ExecutionMode` is still used (in the `ExecutionMode::Manual`
argument above), but `Execution`/`ExecutionStatus` are now unused imports.
Change the top of the file:

```rust
use crate::domain::{Connection, Execution, ExecutionMode, ExecutionStatus, NodeInstance, Workflow};
```

to:

```rust
use crate::domain::{Connection, ExecutionMode, NodeInstance, Workflow};
```

- [ ] **Step 4: Migrate `src/api/webhook.rs::handle_webhook`**

Replace the block from `let mut execution = crate::domain::Execution { ... }`
through `(response_status, Json(execution)).into_response()` with:

```rust
    let execution = match crate::execution_runner::run_and_track_execution(
        &state.storage,
        &state.execution_events,
        &state.registry,
        &workflow,
        ExecutionMode::Webhook,
        Some(vec![trigger_item]),
        &std::collections::HashMap::new(),
    )
    .await
    {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(error = %e, "webhook: failed to persist new execution");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let response_status = if execution.status == ExecutionStatus::Error {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    (response_status, Json(execution)).into_response()
}
```

- [ ] **Step 5: Migrate `src/triggers.rs::fire_schedule` and its caller**

Change `fire_schedule`'s signature to accept the events sender, and replace
its create/run/update block:

```rust
pub async fn fire_schedule(
    storage: std::sync::Arc<dyn crate::storage::Storage>,
    registry: std::sync::Arc<crate::node::NodeRegistry>,
    events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
    workflow_id: Uuid,
) {
    let workflow = match storage.get_workflow(workflow_id).await {
        Ok(Some(wf)) if wf.active => wf,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!(error = %e, %workflow_id, "fire_schedule: failed to fetch workflow");
            return;
        }
    };

    let trigger_items = vec![crate::domain::Item { json: serde_json::json!({}), binary: serde_json::json!({}) }];
    if let Err(e) = crate::execution_runner::run_and_track_execution(
        &storage,
        &events,
        &registry,
        &workflow,
        ExecutionMode::Schedule,
        Some(trigger_items),
        &std::collections::HashMap::new(),
    )
    .await
    {
        tracing::error!(error = %e, "fire_schedule: failed to persist new execution");
    }
}
```

`ExecutionStatus` was only used inside the block just replaced; `ExecutionMode`
is still used above. Change the top of the file:

```rust
use crate::domain::{ExecutionMode, ExecutionStatus};
```

to:

```rust
use crate::domain::ExecutionMode;
```

In `activate_schedule_trigger` (same file), add the events sender alongside
the existing `storage`/`registry` clones:

```rust
    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let events = state.execution_events.clone();
    let workflow_id = workflow.id;

    let job_id = state
        .scheduler
        .register(&cron_expr, move || {
            let storage = storage.clone();
            let registry = registry.clone();
            let events = events.clone();
            async move {
                fire_schedule(storage, registry, events, workflow_id).await;
            }
        })
        .await?;
```

Update `fire_schedule`'s three test call sites in `src/triggers.rs`'s
`#[cfg(test)] mod tests` — each currently calls
`fire_schedule(state.storage.clone(), state.registry.clone(), wf.id)` (or
`Uuid::new_v4()`); add `state.execution_events.clone()` as the third
argument in all three:

```rust
        fire_schedule(state.storage.clone(), state.registry.clone(), state.execution_events.clone(), wf.id).await;
```

(applied to `fire_schedule_runs_the_workflow_and_persists_a_schedule_mode_execution`,
`fire_schedule_on_inactive_workflow_is_a_noop`, and
`fire_schedule_on_missing_workflow_is_a_noop`).

- [ ] **Step 6: Migrate `src/telegram_poller.rs`**

Change `poll_telegram_updates`'s signature and its per-update block. First,
the signature:

```rust
pub async fn poll_telegram_updates(
    storage: Arc<dyn Storage>,
    registry: Arc<NodeRegistry>,
    events: tokio::sync::broadcast::Sender<crate::execution_runner::ExecutionEvent>,
    workflow: Workflow,
    bot_token: String,
    api_base_url: String,
) {
```

Then, inside the `for update in updates` loop, replace the block from
`let mut execution = Execution { ... }` through the closing
`if let Err(e) = storage.update_execution(&execution).await { ... }` with:

```rust
            let trigger_item = Item { json: update, binary: serde_json::json!({}) };

            if let Err(e) = crate::execution_runner::run_and_track_execution(
                &storage,
                &events,
                &registry,
                &current_workflow,
                ExecutionMode::Telegram,
                Some(vec![trigger_item]),
                &credentials,
            )
            .await
            {
                tracing::error!(error = %e, workflow_id = %current_workflow.id, "telegram poller: failed to persist new execution");
            }
```

In `activate_telegram_trigger` (same file), add the events sender alongside
the existing `storage`/`registry` clones and pass it through:

```rust
    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let events = state.execution_events.clone();
    let workflow_id = workflow.id;

    let join_handle = tokio::spawn(poll_telegram_updates(
        storage,
        registry,
        events,
        workflow.clone(),
        bot_token,
        api_base_url,
    ));
```

Update the two test call sites (in `src/telegram_poller.rs`'s test module,
around what are currently lines 532 and 601) — each currently calls
`poll_telegram_updates(storage.clone(), registry, wf.clone(), "111:AAA".into(), telegram.uri())`;
add a fresh broadcast sender as the third argument at each:

```rust
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let poll = tokio::spawn(poll_telegram_updates(storage.clone(), registry, events, wf.clone(), "111:AAA".into(), telegram.uri()));
```

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: all tests pass (this migrates behavior, not just signatures — the
existing execution-related integration tests in `tests/api_test.rs`, and
`triggers.rs`'/`telegram_poller.rs`'s own test modules, are the regression
check that nothing about persisted `Execution` rows changed for the
non-incremental parts of their assertions).

- [ ] **Step 8: Commit**

```bash
git add src/state.rs src/main.rs src/api/workflows.rs src/api/webhook.rs src/triggers.rs src/telegram_poller.rs tests/api_test.rs tests/health_test.rs
git commit -m "feat: migrate all execution call sites onto execution_runner"
```

---

### Task 4: WebSocket transport

**Files:**
- Modify: `Cargo.toml` (axum `ws` feature; `tokio-tungstenite` + `futures-util` dev-dependencies)
- Modify: `src/api/executions.rs` (the WS handler)
- Modify: `src/api/mod.rs` (route)
- Modify: `src/static_files.rs` (namespace list)
- Modify: `tests/api_test.rs` (real-socket integration test)

**Interfaces:**
- Produces: `GET /ws/workflows/:id/executions` — upgrades to a WebSocket,
  expects `{"token": "<jwt>"}` as the first text frame within 5s, then
  forwards every `ExecutionEvent` (Task 2) whose `workflow_id` matches, as a
  JSON text frame, until the client disconnects.
- Consumes: `state.execution_events.subscribe()`, `crate::auth::verify_token`
  (existing).

- [ ] **Step 1: Add the axum `ws` feature and test-only WebSocket client**

In `Cargo.toml`, change:

```toml
axum = "0.7"
```

to:

```toml
axum = { version = "0.7", features = ["ws"] }
```

Add to `[dev-dependencies]`:

```toml
tokio-tungstenite = "0.24"
futures-util = "0.3"
```

Run: `cargo build`
Expected: succeeds (confirms the `ws` feature resolves; if
`tokio-tungstenite = "0.24"` fails to resolve against this project's pinned
`tokio` version, adjust to the nearest compatible release on crates.io and
note the change — same allowance this project's other plans already use for
version pins).

- [ ] **Step 2: Update `static_files.rs`'s API-namespace list**

In `src/static_files.rs`, change:

```rust
fn is_api_path(path: &str) -> bool {
    ["rest", "webhook"]
        .iter()
        .any(|prefix| path == *prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/')))
}
```

to:

```rust
fn is_api_path(path: &str) -> bool {
    ["rest", "webhook", "ws"]
        .iter()
        .any(|prefix| path == *prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/')))
}
```

Update the existing test's path list in the same file (add `/ws/...` cases,
matching the existing `/rest`/`/webhook` ones):

```rust
    #[tokio::test]
    async fn unmatched_api_paths_404_instead_of_serving_the_spa() {
        for path in [
            "/rest/definitely-not-a-real-route",
            "/webhook/definitely-not-a-real-route",
            "/ws/definitely-not-a-real-route",
            "/rest",
            "/webhook",
            "/ws",
        ] {
            let response = serve_frontend(path.parse::<Uri>().unwrap()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path} should not fall through to index.html");
        }
    }
```

Run: `cargo test --lib static_files::`
Expected: passes.

- [ ] **Step 3: Write the WS handler**

In `src/api/executions.rs`, change the top import line:

```rust
use axum::extract::{Path, Query, State};
```

to:

```rust
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
```

Then add, below the existing `list_executions_for_workflow` function:

```rust
#[derive(serde::Deserialize)]
struct AuthFrame {
    token: String,
}

pub async fn subscribe_executions(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_execution_socket(socket, state, workflow_id))
}

async fn handle_execution_socket(mut socket: WebSocket, state: AppState, workflow_id: Uuid) {
    let authed = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await,
        Ok(Some(Ok(Message::Text(ref text))))
            if serde_json::from_str::<AuthFrame>(text)
                .ok()
                .and_then(|frame| crate::auth::verify_token(&frame.token, &state.jwt_secret).ok())
                .is_some()
    );
    if !authed {
        let _ = socket.close().await;
        return;
    }

    let mut receiver = state.execution_events.subscribe();
    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(event) if event.workflow_id == workflow_id => {
                        let Ok(json) = serde_json::to_string(&event) else { continue };
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                if incoming.is_none() {
                    break;
                }
            }
        }
    }
}
```

`Uuid`, `State`, `Path`, `IntoResponse`, and `Json` are all already imported
in this file; the import change above is the only new one this step needs.

- [ ] **Step 4: Wire the route**

In `src/api/mod.rs`, add (next to the other `/rest/workflows/:id/*` routes,
though the path itself is `/ws/*`):

```rust
        .route("/ws/workflows/:id/executions", get(executions::subscribe_executions))
```

- [ ] **Step 5: Write the failing integration test**

Add to `tests/api_test.rs`. This is the first WebSocket test in this file —
every other test drives the router in-memory via `tower::ServiceExt::oneshot`,
which cannot perform a real upgrade handshake, so this one binds a real
listener instead:

```rust
#[tokio::test]
async fn websocket_streams_node_events_during_a_run_and_a_bad_token_closes_it() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let state = test_state().await;
    let app = r8r::api::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_app = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, serve_app).await.unwrap();
    });

    let token = register_and_get_token(&app, "ws-live@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({
                        "name": "ws-live",
                        "nodes": [
                            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
                            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"a": 1}}, "disabled": false}
                        ],
                        "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "set1", "to_input": 0}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    // A bad token closes the socket without forwarding anything.
    let (mut bad_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/workflows/{workflow_id}/executions"))
        .await
        .unwrap();
    bad_ws.send(WsMessage::Text(serde_json::json!({"token": "not-a-real-token"}).to_string())).await.unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), bad_ws.next()).await.unwrap();
    assert!(matches!(next, Some(Ok(WsMessage::Close(_))) | None));

    // A good token streams the run's events.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/workflows/{workflow_id}/executions"))
        .await
        .unwrap();
    ws.send(WsMessage::Text(serde_json::json!({"token": token}).to_string())).await.unwrap();

    let exec_app = app.clone();
    let exec_token = token.clone();
    let exec_workflow_id = workflow_id.clone();
    tokio::spawn(async move {
        exec_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/rest/workflows/{exec_workflow_id}/execute"))
                    .header("authorization", format!("Bearer {exec_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    });

    let mut seen_types = Vec::new();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for a websocket event")
            .expect("socket closed early")
            .unwrap();
        if let WsMessage::Text(text) = msg {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["workflow_id"], workflow_id);
            let event_type = value["type"].as_str().unwrap().to_string();
            let is_final = event_type == "execution_finished";
            seen_types.push(event_type);
            if is_final {
                break;
            }
        }
    }

    assert!(seen_types.contains(&"node_started".to_string()));
    assert!(seen_types.contains(&"node_finished".to_string()));
    assert_eq!(seen_types.last().unwrap(), "execution_finished");
}
```

- [ ] **Step 6: Run the test to verify it fails, then passes**

Run: `cargo test websocket_streams_node_events`
Expected first: fails to compile if Step 3/4 aren't done yet, or fails at
runtime if the handshake/forwarding logic has a bug — confirm the failure
is one of those, not a typo in the test itself, before moving on.

Run again after Steps 3-4 are in place: `cargo test websocket_streams_node_events`
Expected: passes.

- [ ] **Step 7: Run the full test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock src/api/executions.rs src/api/mod.rs src/static_files.rs tests/api_test.rs
git commit -m "feat: add /ws/workflows/:id/executions live execution status endpoint"
```

---

### Task 5: Frontend — live execution socket

**Files:**
- Create: `frontend/src/composables/useLiveExecutionSocket.ts`
- Create: `frontend/src/composables/useLiveExecutionSocket.spec.ts`
- Modify: `frontend/src/views/WorkflowEditorView.vue`

**Interfaces:**
- Consumes: `getToken` (`frontend/src/api/client.ts`, existing), `Execution`/`Item`/`ExecutionStatus`
  (`frontend/src/types/domain.ts`, existing).
- Produces: `useLiveExecutionSocket(workflowId: string): { execution: Ref<Execution | null>; connect: () => void; disconnect: () => void }`.

- [ ] **Step 1: Write the failing tests**

Create `frontend/src/composables/useLiveExecutionSocket.spec.ts`:

```typescript
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { useLiveExecutionSocket } from './useLiveExecutionSocket'

class MockWebSocket {
  static instances: MockWebSocket[] = []
  listeners: Record<string, Array<(e: unknown) => void>> = {}
  sent: string[] = []
  constructor(public url: string) {
    MockWebSocket.instances.push(this)
  }
  addEventListener(type: string, cb: (e: unknown) => void) {
    ;(this.listeners[type] ??= []).push(cb)
  }
  send(data: string) {
    this.sent.push(data)
  }
  close() {}
  emitOpen() {
    this.listeners['open']?.forEach((cb) => cb({}))
  }
  emitMessage(data: unknown) {
    this.listeners['message']?.forEach((cb) => cb({ data: JSON.stringify(data) }))
  }
}

describe('useLiveExecutionSocket', () => {
  beforeEach(() => {
    MockWebSocket.instances = []
    vi.stubGlobal('WebSocket', MockWebSocket)
    localStorage.setItem('r8r_token', 'test-token')
  })

  it('sends the auth frame once the socket opens', () => {
    const { connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]
    ws.emitOpen()
    expect(ws.sent).toEqual([JSON.stringify({ token: 'test-token' })])
  })

  it('adopts a new execution_id and fills node_outputs from node_finished/skipped events', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_started', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'trigger' })
    expect(execution.value?.id).toBe('e1')
    expect(execution.value?.status).toBe('Running')

    ws.emitMessage({
      type: 'node_finished',
      execution_id: 'e1',
      workflow_id: 'wf-1',
      node_id: 'trigger',
      items: [{ json: {}, binary: {} }],
    })
    expect(execution.value?.node_outputs['trigger']).toEqual([{ json: {}, binary: {} }])

    ws.emitMessage({ type: 'execution_finished', execution_id: 'e1', workflow_id: 'wf-1', status: 'Success' })
    expect(execution.value?.status).toBe('Success')
    expect(execution.value?.finished_at).not.toBeNull()
  })

  it('a new execution_id supersedes a stale one', () => {
    const { execution, connect } = useLiveExecutionSocket('wf-1')
    connect()
    const ws = MockWebSocket.instances[0]

    ws.emitMessage({ type: 'node_finished', execution_id: 'e1', workflow_id: 'wf-1', node_id: 'a', items: [] })
    ws.emitMessage({ type: 'node_started', execution_id: 'e2', workflow_id: 'wf-1', node_id: 'b' })

    expect(execution.value?.id).toBe('e2')
    expect(execution.value?.node_outputs['a']).toBeUndefined()
  })

  it('does not connect when there is no stored token', () => {
    localStorage.removeItem('r8r_token')
    const { connect } = useLiveExecutionSocket('wf-1')
    connect()
    expect(MockWebSocket.instances).toHaveLength(0)
  })
})
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `frontend/`): `npx vitest run src/composables/useLiveExecutionSocket.spec.ts`
Expected: fails to resolve `./useLiveExecutionSocket` (the module doesn't
exist yet).

- [ ] **Step 3: Implement the composable**

Create `frontend/src/composables/useLiveExecutionSocket.ts`:

```typescript
import { ref, type Ref } from 'vue'
import { getToken } from '../api/client'
import type { Execution, ExecutionStatus, Item } from '../types/domain'

export type LiveExecutionEvent = { execution_id: string; workflow_id: string } & (
  | { type: 'node_started'; node_id: string }
  | { type: 'node_finished'; node_id: string; items: Item[] }
  | { type: 'node_errored'; node_id: string; error: string }
  | { type: 'node_skipped'; node_id: string; items: Item[] }
  | { type: 'execution_finished'; status: ExecutionStatus }
)

export interface LiveExecutionSocket {
  execution: Ref<Execution | null>
  connect: () => void
  disconnect: () => void
}

export function useLiveExecutionSocket(workflowId: string): LiveExecutionSocket {
  const execution = ref<Execution | null>(null)
  let socket: WebSocket | null = null
  let liveExecutionId: string | null = null

  function applyEvent(event: LiveExecutionEvent) {
    if (liveExecutionId !== event.execution_id) {
      liveExecutionId = event.execution_id
      execution.value = {
        id: event.execution_id,
        workflow_id: event.workflow_id,
        status: 'Running',
        mode: 'Manual',
        node_outputs: {},
        started_at: new Date().toISOString(),
        finished_at: null,
      }
    }
    const current = execution.value
    if (!current) return
    switch (event.type) {
      case 'node_finished':
      case 'node_skipped':
        current.node_outputs[event.node_id] = event.items
        break
      case 'node_errored':
        current.node_outputs[event.node_id] = [{ json: { error: event.error }, binary: {} }]
        break
      case 'execution_finished':
        current.status = event.status
        current.finished_at = new Date().toISOString()
        break
      case 'node_started':
        break
    }
  }

  function connect() {
    const token = getToken()
    if (!token) return
    const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
    socket = new WebSocket(`${protocol}//${location.host}/ws/workflows/${workflowId}/executions`)
    socket.addEventListener('open', () => socket?.send(JSON.stringify({ token })))
    socket.addEventListener('message', (e) => {
      try {
        applyEvent(JSON.parse((e as MessageEvent).data as string) as LiveExecutionEvent)
      } catch {
        // Malformed frame -- ignore. The final REST response from execute()
        // is still authoritative and will land regardless.
      }
    })
  }

  function disconnect() {
    socket?.close()
    socket = null
  }

  return { execution, connect, disconnect }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `frontend/`): `npx vitest run src/composables/useLiveExecutionSocket.spec.ts`
Expected: all four tests pass.

- [ ] **Step 5: Wire it into `WorkflowEditorView.vue`**

In `frontend/src/views/WorkflowEditorView.vue`, change the Vue import to add
`onUnmounted`:

```typescript
import { computed, onMounted, onUnmounted, ref } from 'vue'
```

Add the composable import (next to the other component/store imports):

```typescript
import { useLiveExecutionSocket } from '../composables/useLiveExecutionSocket'
```

Replace:

```typescript
const execution = ref<Execution | null>(null)
```

with:

```typescript
const live = useLiveExecutionSocket(workflowId)
const execution = live.execution
```

Change the existing `onMounted` to connect the socket, and add an
`onUnmounted` to disconnect it:

```typescript
onMounted(async () => {
  live.connect()
  try {
    workflow.value = await api.get<Workflow>(`/rest/workflows/${workflowId}`)
  } catch (e) {
    loadError.value = messageFor(e, 'Failed to load workflow.')
  }
})

onUnmounted(() => {
  live.disconnect()
})
```

No other changes to this file are needed: `execute()` still does
`execution.value = await api.post<Execution>(...)` at the end exactly as
before, which now simply overwrites the same ref the socket has been
filling in live — the final REST response remains authoritative, matching
the spec's explicit no-new-failure-mode requirement.

- [ ] **Step 6: Run the full frontend test suite and build**

Run (from `frontend/`): `npm test`
Expected: all tests pass, including the 4 new composable tests.

Run (from `frontend/`): `npm run build`
Expected: `vue-tsc --noEmit` and `vite build` both succeed.

- [ ] **Step 7: Run the full backend test suite**

Run (from the repo root): `cargo test`
Expected: all tests pass (this rebuild picks up the freshly-built
`frontend/dist/` the embedded-frontend tests assert against).

- [ ] **Step 8: Commit**

```bash
git add frontend/src/composables frontend/src/views/WorkflowEditorView.vue
git commit -m "feat: wire live execution status into the workflow editor"
```
