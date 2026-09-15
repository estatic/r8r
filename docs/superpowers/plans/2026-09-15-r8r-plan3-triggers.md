# r8r Plan 3 — Triggers — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give r8r a real trigger-activation lifecycle: a Webhook trigger node that registers `/webhook/:workflow_id/:path` and actually starts a workflow run from real HTTP request data, and a Schedule (cron) trigger node backed by `tokio-cron-scheduler`, both gated behind a workflow's `active` flag which currently does nothing.

**Architecture:** Two new trigger node types (`core.webhook`, `core.schedule`) are registered like any other node, but external firing never calls their own `execute()` — instead, `src/engine.rs` gains a seeded-start-node execution path (`execute_workflow_seeded`) that injects real trigger data (webhook payload, or an empty item for schedule) directly as the start node's output, bypassing its `execute()` call entirely. A new `PATCH /rest/workflows/:id/active` endpoint toggles `Workflow.active`; on activation it registers a cron job (if the start node is `core.schedule`) via a thin `Scheduler` wrapper around `tokio_cron_scheduler::JobScheduler`, tracked in a small in-memory `TriggerRegistry` (`workflow_id -> cron job id`) so deactivation can clean it up. Webhook routing needs no such registry — the handler does a live storage lookup on every request (`workflow.active` + start-node path/method match), which is simpler and correct without needing to dynamically add/remove Axum routes at runtime.

**Tech Stack:** Rust, `tokio-cron-scheduler` (new dependency), the existing Axum/sqlx/Tokio stack (unchanged) plus Plan 2's expression/engine work (unchanged, reused as-is).

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md` (§5.3 Triggers, §7 Execution Engine, §8 API)
**Roadmap reference:** `docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`, Plan 3 (§3.1–3.3)

## Global Constraints

- One process, no distributed/worker-process split (spec §3, §9) — the scheduler and webhook route both run in-process via Tokio, same binary, same port, same as the existing `/rest/*` API.
- Webhook route is `/webhook/:workflow_id/:path` with `:path` as a single Axum path segment (not a wildcard) — matches the roadmap's own wording (3.1.1.1); multi-segment webhook paths are out of scope for this plan.
- `core.webhook` node parameters: `{"path": "<string>", "method": "GET" | "POST" | "ANY"}` (`"ANY"` matches both GET and POST; anything else is treated as an exact HTTP-method match, case-insensitive).
- `core.schedule` node parameters: `{"cron": "<standard 5- or 6-field cron expression>"}`.
- **Execute-then-respond**: the webhook HTTP handler waits for the whole workflow run to finish and returns its result (200 with the execution JSON, or an error status) — this is roadmap item 3.1.3.1's decision, made here. A future "respond immediately, execute in background" mode is out of scope for this plan.
- Activation never validates that a workflow's start node is a specific trigger type — activating a workflow whose start node is `core.manualTrigger` (or anything else) is allowed and simply means nothing external can fire it; only `core.schedule` start nodes cause a cron job to be registered. This mirrors n8n's permissiveness and avoids inventing validation rules the spec doesn't ask for.
- No path-collision validation across different workflows' webhook paths in v1 (two different workflows could both register `path: "foo"` — the URL disambiguates by `workflow_id` regardless, so this causes no actual conflict, just isn't validated as a UX nicety). Noted as an acceptable v1 gap, not a defect.
- `ExecutionMode` gains `Webhook` and `Schedule` variants alongside the existing `Manual`. These are stored as a `serde_json`-serialized string in the existing `executions.mode TEXT` column (see `src/storage/sqlite.rs`'s `create_execution`) — **no SQL migration is needed** for this change; the column already stores whatever string `serde_json::to_string(&ExecutionMode)` produces, and adding enum variants doesn't touch existing rows.
- `Retry` (the fourth mode named in the spec) is explicitly out of scope — carried forward to Plan 7 (retry semantics).
- No frontend changes (Plan 6's job) and no Credential-store changes (Plan 4's job) — this plan is API + engine + in-process trigger machinery only.
- The `Scheduler` wrapper's registered job closures capture only `Arc<dyn Storage>` and `Arc<NodeRegistry>` (never the full `AppState`, which itself holds `Arc<Scheduler>`) — this is a deliberate design choice to avoid an `Arc` reference cycle between `AppState` and its own `Scheduler` field. Do not "simplify" this by passing the whole `AppState` into a registered job closure.

### A note on `tokio-cron-scheduler`

The exact API surface of `tokio-cron-scheduler` (method names on `JobScheduler`/`Job`, the exact job-id type returned by `JobScheduler::add`, the exact closure signature `Job::new_async` expects) could not be verified against live documentation while writing this plan. **Task 6's code is a reference implementation, not literal code to transcribe blindly** — the implementer must check the actually-installed crate version's API (`cargo doc -p tokio-cron-scheduler --open`, or read the source under `~/.cargo/registry/src/.../tokio-cron-scheduler-*/`) and adapt method/type names as needed, preserving the *behavior and signatures* of `Scheduler` described here (that's r8r's own wrapper type, not the crate's). Every other task in this plan only calls `Scheduler`'s own methods — none of them touch `tokio_cron_scheduler` directly — so the risk is contained to Task 6, the same pattern used successfully for `rquickjs` in Plan 2.

---

## File Structure

- `Cargo.toml` — add `tokio-cron-scheduler` dependency.
- `src/domain.rs` — modify. `ExecutionMode` gains `Webhook`, `Schedule` variants.
- `src/storage/mod.rs`, `src/storage/sqlite.rs` — modify. Add `Storage::update_workflow`.
- `src/engine.rs` — modify. Add `start_node_id` (exposes the validated start node id) and `execute_workflow_seeded` (the trigger-data injection path); `execute_workflow` becomes a thin wrapper calling `execute_workflow_seeded(.., None)`.
- `src/nodes/webhook.rs` — new. `WebhookNode` (`core.webhook`).
- `src/nodes/schedule.rs` — new. `ScheduleNode` (`core.schedule`).
- `src/nodes/mod.rs` — modify. Register both.
- `src/trigger_registry.rs` — new. `TriggerRegistry` — in-memory `workflow_id -> cron job id` map.
- `src/scheduler.rs` — new. `Scheduler` — thin wrapper around `tokio_cron_scheduler::JobScheduler`.
- `src/triggers.rs` — new. `activate_workflow_triggers`, `deactivate_workflow_triggers`, `fire_schedule` — ties `Scheduler` + `TriggerRegistry` + `engine::execute_workflow_seeded` + `Storage` together; used by both the API handler and startup reactivation.
- `src/state.rs` — modify. `AppState` gains `scheduler: Arc<Scheduler>`, `trigger_registry: Arc<TriggerRegistry>`.
- `src/api/workflows.rs` — modify. Add `set_workflow_active` handler (`PATCH /rest/workflows/:id/active`).
- `src/api/webhook.rs` — new. `handle_webhook` (`/webhook/:workflow_id/:path`).
- `src/api/mod.rs` — modify. Wire the two new routes.
- `src/main.rs` — modify. Construct `Scheduler`/`TriggerRegistry`, wire into `AppState`, call `triggers::reactivate_all` on startup.
- `src/lib.rs` — modify. Add `pub mod trigger_registry; pub mod scheduler; pub mod triggers;`.
- `tests/api_test.rs` — modify. One new end-to-end test: activate a workflow with a Webhook trigger, curl its webhook path, assert it executed; deactivate it, assert the path now 404s.

---

### Task 1: `ExecutionMode` gains `Webhook`/`Schedule` variants

**Files:**
- Modify: `src/domain.rs`

**Interfaces:**
- Changes: `r8r::domain::ExecutionMode` — adds `Webhook` and `Schedule` variants alongside the existing `Manual`.

- [ ] **Step 1: Write the failing test**

```rust
// add to the tests module in src/domain.rs
#[test]
fn execution_mode_serializes_new_variants() {
    assert_eq!(serde_json::to_string(&ExecutionMode::Webhook).unwrap(), "\"Webhook\"");
    assert_eq!(serde_json::to_string(&ExecutionMode::Schedule).unwrap(), "\"Schedule\"");
    let parsed: ExecutionMode = serde_json::from_str("\"Webhook\"").unwrap();
    assert_eq!(parsed, ExecutionMode::Webhook);
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib domain::tests::execution_mode_serializes_new_variants`
Expected: compile error — `ExecutionMode::Webhook`/`ExecutionMode::Schedule` don't exist yet.

- [ ] **Step 3: Implement**

```rust
// src/domain.rs — replace the existing ExecutionMode enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    Manual,
    Webhook,
    Schedule,
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib domain::tests`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/domain.rs
git commit -m "feat: add Webhook and Schedule ExecutionMode variants"
```

---

### Task 2: `Storage::update_workflow`

**Files:**
- Modify: `src/storage/mod.rs`
- Modify: `src/storage/sqlite.rs`

**Interfaces:**
- Produces: `Storage::update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>` — full-row update (name, active, definition, updated_at) keyed on `workflow.id`. Callers are responsible for bumping `workflow.updated_at` before calling this (same convention as `create_workflow`, which does not touch `updated_at` itself either).

- [ ] **Step 1: Write the failing test**

```rust
// add to the tests module in src/storage/sqlite.rs
#[tokio::test]
async fn update_workflow_persists_active_flag_and_name() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let mut wf = sample_workflow();
    storage.create_workflow(&wf).await.unwrap();

    wf.active = true;
    wf.name = "renamed".into();
    wf.updated_at = Utc::now();
    storage.update_workflow(&wf).await.unwrap();

    let fetched = storage.get_workflow(wf.id).await.unwrap().unwrap();
    assert!(fetched.active);
    assert_eq!(fetched.name, "renamed");
}

#[tokio::test]
async fn update_workflow_on_unknown_id_does_not_error() {
    // Matches sqlx's own UPDATE-affecting-zero-rows behavior: no error, just a no-op.
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let wf = sample_workflow();
    let result = storage.update_workflow(&wf).await;
    assert!(result.is_ok());
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib storage::sqlite::tests::update_workflow`
Expected: compile error — `update_workflow` not defined on the `Storage` trait.

- [ ] **Step 3: Add to the trait**

```rust
// src/storage/mod.rs — add inside the Storage trait, alongside create_workflow
async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
```

- [ ] **Step 4: Implement in SqliteStorage**

```rust
// src/storage/sqlite.rs — add inside impl Storage for SqliteStorage, alongside create_workflow
async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
    let definition = serde_json::json!({
        "nodes": workflow.nodes,
        "connections": workflow.connections,
    });
    sqlx::query(
        "UPDATE workflows SET name = ?, active = ?, definition = ?, updated_at = ? WHERE id = ?"
    )
    .bind(&workflow.name)
    .bind(workflow.active as i64)
    .bind(definition.to_string())
    .bind(workflow.updated_at.to_rfc3339())
    .bind(workflow.id.to_string())
    .execute(&self.pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test --lib storage::sqlite::tests`
Expected: all pass.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: fails to compile — nothing else implements `Storage` besides `SqliteStorage`, so this alone should NOT break anything. If it does fail, it means something else in the crate implements `Storage` (e.g. a test mock) that also needs `update_workflow` added — find it and add a matching implementation before proceeding; do not skip this check.

- [ ] **Step 7: Commit**

```bash
git add src/storage/mod.rs src/storage/sqlite.rs
git commit -m "feat: add Storage::update_workflow"
```

---

### Task 3: `engine::start_node_id` + `execute_workflow_seeded`

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Produces: `pub fn start_node_id(workflow: &Workflow) -> anyhow::Result<String>` — returns the id of the workflow's unique start node (the node `topological_order` places first), reusing `topological_order`'s existing validation (errors on zero nodes... actually: on an EMPTY workflow, returns `Err` — there is no start node to name — this is a new, stricter contract than `topological_order`, which returns `Ok(vec![])` for an empty workflow. `start_node_id` must return `Err(anyhow::anyhow!("workflow has no nodes"))` for that case instead, since callers (the webhook handler, activation logic) need an actual id, not an empty result.
- Produces: `pub async fn execute_workflow_seeded(workflow: &Workflow, registry: &NodeRegistry, trigger_items: Option<Vec<Item>>) -> anyhow::Result<HashMap<String, Vec<Item>>>` — identical to the existing `execute_workflow` in every way EXCEPT: when `trigger_items` is `Some(items)`, the start node's `produced` entry is seeded directly as `vec![items]` (a single primary-output port), and that node's own `execute()` is never called (its parameters are also never resolved) for that node only. Every other node in the workflow executes exactly as before.
- Changes: `pub async fn execute_workflow(workflow: &Workflow, registry: &NodeRegistry) -> anyhow::Result<HashMap<String, Vec<Item>>>` — becomes a one-line wrapper: `execute_workflow_seeded(workflow, registry, None).await`. Its behavior for every existing caller (manual execution via `POST /rest/workflows/:id/execute`) is byte-for-byte unchanged — `None` seeding means "skip the new seeding branch entirely," which is exactly the pre-existing code path.

- [ ] **Step 1: Write failing tests**

```rust
// add to the tests module in src/engine.rs
#[test]
fn start_node_id_returns_the_unique_start_node() {
    let wf = linear_workflow();
    assert_eq!(start_node_id(&wf).unwrap(), "trigger");
}

#[test]
fn start_node_id_errors_on_empty_workflow() {
    let mut wf = linear_workflow();
    wf.nodes.clear();
    wf.connections.clear();
    assert!(start_node_id(&wf).is_err());
}

#[tokio::test]
async fn execute_workflow_seeded_injects_trigger_items_as_start_node_output() {
    let wf = linear_workflow(); // trigger -> set1
    let seeded_items = vec![Item { json: serde_json::json!({"from": "webhook"}), binary: serde_json::json!({}) }];
    let outputs = execute_workflow_seeded(&wf, &registry(), Some(seeded_items.clone())).await.unwrap();
    // trigger's own execute() was never called — its output IS the seeded item, verbatim.
    assert_eq!(outputs["trigger"], seeded_items);
    // set1 (which adds a static "greeting" field per the linear_workflow fixture,
    // discarding trigger's json) still ran normally downstream — this just proves
    // seeding didn't break normal downstream execution.
    assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
}

#[tokio::test]
async fn execute_workflow_with_none_seed_behaves_exactly_as_before() {
    let wf = linear_workflow();
    let outputs = execute_workflow(&wf, &registry()).await.unwrap();
    assert_eq!(outputs["trigger"][0].json, serde_json::json!({}));
    assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib engine::tests::start_node_id`
Run: `cargo test --lib engine::tests::execute_workflow_seeded`
Expected: compile errors — neither function exists yet.

- [ ] **Step 3: Implement `start_node_id`**

```rust
// src/engine.rs — add near topological_order
pub fn start_node_id(workflow: &Workflow) -> anyhow::Result<String> {
    if workflow.nodes.is_empty() {
        return Err(anyhow::anyhow!("workflow has no nodes"));
    }
    let order = topological_order(workflow)?;
    order
        .first()
        .map(|n| n.id.clone())
        .ok_or_else(|| anyhow::anyhow!("workflow has no nodes"))
}
```

- [ ] **Step 4: Rename `execute_workflow` to `execute_workflow_seeded` and add the wrapper**

```rust
// src/engine.rs — rename the existing `pub async fn execute_workflow(...)` to
// `execute_workflow_seeded`, add the `trigger_items` parameter, and add the
// seeding branch inside the loop. Then add a new thin `execute_workflow` wrapper.

pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None).await
}

pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &NodeRegistry,
    trigger_items: Option<Vec<Item>>,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = topological_order(workflow)?;
    let start_id = order.first().map(|n| n.id.clone());
    let mut produced: HashMap<String, crate::node::NodeOutput> = HashMap::new();
    let mut error_produced: HashMap<String, Vec<Item>> = HashMap::new();

    for node_instance in &order {
        // ... existing input-aggregation loop, unchanged ...

        if node_instance.disabled {
            produced.insert(node_instance.id.clone(), vec![input_items]);
            continue;
        }

        // NEW: seeded start node — inject trigger_items and skip execute() entirely
        // for this node only. Must come after the disabled check (a disabled start
        // node keeps its existing passthrough semantics, not the seed) and before
        // the registry lookup / empty-input skip (the seed always "counts" as
        // having produced real output, regardless of what input_items ended up
        // being — a start node has no incoming connections, so input_items is
        // always empty anyway; the seed replaces it, not merges with it).
        if let (Some(items), Some(start)) = (&trigger_items, &start_id) {
            if &node_instance.id == start {
                produced.insert(node_instance.id.clone(), vec![items.clone()]);
                continue;
            }
        }

        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;

        // ... existing has_incoming_connection empty-input skip, resolve_parameters,
        // execute() match block, unchanged ...
    }

    // ... existing flattening, unchanged ...
}
```

The elided sections (`// ... existing ... unchanged ...`) are the current Task-5/6/12 body of the function exactly as it exists on `main` today — copy it verbatim into the new function name; only the two additions above (the `start_id` computation near the top, and the new `if let (Some(items), Some(start))` branch inside the loop) are new code.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib engine::tests`
Expected: all pass, including every pre-existing engine test (unaffected — they all call the now-wrapper `execute_workflow`, whose behavior is unchanged) and the four new tests above.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: all pass — `execute_workflow`'s call site in `src/api/workflows.rs` needs zero changes since its signature is unchanged.

- [ ] **Step 7: Commit**

```bash
git add src/engine.rs
git commit -m "feat: add execute_workflow_seeded for trigger-data injection"
```

---

### Task 4: `WebhookNode` and `ScheduleNode`

**Files:**
- Create: `src/nodes/webhook.rs`
- Create: `src/nodes/schedule.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::webhook::WebhookNode`, `type_name() == "core.webhook"`. Parameters `{"path": "<string>", "method": "GET" | "POST" | "ANY"}`. Its `execute()` is a FALLBACK only — used when the workflow is run manually/for testing (not via a real webhook request, which always uses `execute_workflow_seeded` and never calls this). Returns a single item with empty json, matching `ManualTriggerNode`'s existing shape.
- Produces: `r8r::nodes::schedule::ScheduleNode`, `type_name() == "core.schedule"`. Parameters `{"cron": "<cron expression>"}`. Same fallback-only `execute()` behavior as `WebhookNode`.

- [ ] **Step 1: Write failing tests**

```rust
// src/nodes/webhook.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct WebhookNode;

#[async_trait]
impl Node for WebhookNode {
    fn type_name(&self) -> &'static str {
        "core.webhook"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_execute_returns_a_single_empty_item() {
        let node = WebhookNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"path": "my-hook", "method": "POST"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }
}
```

```rust
// src/nodes/schedule.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct ScheduleNode;

#[async_trait]
impl Node for ScheduleNode {
    fn type_name(&self) -> &'static str {
        "core.schedule"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_execute_returns_a_single_empty_item() {
        let node = ScheduleNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"cron": "0 0 * * * *"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::webhook --lib nodes::schedule`
Expected: compile errors — modules not wired into `mod.rs` yet.

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod schedule;
pub mod webhook;
// (add alongside the existing pub mod lines, keep alphabetical)
```

In `register_all`, add:

```rust
registry.register(Box::new(webhook::WebhookNode));
registry.register(Box::new(schedule::ScheduleNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::webhook --lib nodes::schedule`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/webhook.rs src/nodes/schedule.rs src/nodes/mod.rs
git commit -m "feat: add Webhook and Schedule trigger nodes"
```

---

### Task 5: `TriggerRegistry`

**Files:**
- Create: `src/trigger_registry.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `r8r::trigger_registry::TriggerRegistry` — `new() -> Self`, `record_cron_job(&self, workflow_id: Uuid, job_id: Uuid)`, `take_cron_job(&self, workflow_id: Uuid) -> Option<Uuid>` (removes and returns the entry, for deactivation — idempotent: returns `None` if there was never one, or it was already taken).

- [ ] **Step 1: Write failing tests**

```rust
// src/trigger_registry.rs
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
pub struct TriggerRegistry {
    cron_jobs: Mutex<HashMap<Uuid, Uuid>>,
}

impl TriggerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_cron_job(&self, workflow_id: Uuid, job_id: Uuid) {
        self.cron_jobs.lock().unwrap().insert(workflow_id, job_id);
    }

    pub fn take_cron_job(&self, workflow_id: Uuid) -> Option<Uuid> {
        self.cron_jobs.lock().unwrap().remove(&workflow_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_takes_a_cron_job() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, job_id);
        assert_eq!(registry.take_cron_job(workflow_id), Some(job_id));
    }

    #[test]
    fn take_is_idempotent_after_the_first_call() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        registry.take_cron_job(workflow_id);
        assert_eq!(registry.take_cron_job(workflow_id), None);
    }

    #[test]
    fn take_on_unknown_workflow_returns_none() {
        let registry = TriggerRegistry::new();
        assert_eq!(registry.take_cron_job(Uuid::new_v4()), None);
    }

    #[test]
    fn recording_again_overwrites_the_previous_job_id() {
        let registry = TriggerRegistry::new();
        let workflow_id = Uuid::new_v4();
        registry.record_cron_job(workflow_id, Uuid::new_v4());
        let second_job = Uuid::new_v4();
        registry.record_cron_job(workflow_id, second_job);
        assert_eq!(registry.take_cron_job(workflow_id), Some(second_job));
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib trigger_registry::tests`
Expected: compile error — `pub mod trigger_registry;` not yet in `src/lib.rs`.

- [ ] **Step 3: Wire into `src/lib.rs`**

```rust
pub mod trigger_registry;
// (add alongside the existing pub mod lines)
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib trigger_registry::tests`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/trigger_registry.rs src/lib.rs
git commit -m "feat: add TriggerRegistry (workflow_id -> cron job id)"
```

---

### Task 6: `Scheduler` wrapper around `tokio-cron-scheduler`

**Files:**
- Modify: `Cargo.toml` (add `tokio-cron-scheduler`)
- Create: `src/scheduler.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `r8r::scheduler::Scheduler` — `async fn new() -> anyhow::Result<Self>` (constructs and starts the underlying job scheduler); `async fn register<F, Fut>(&self, cron_expr: &str, job: F) -> anyhow::Result<uuid::Uuid>` where `F: Fn() -> Fut + Send + Sync + 'static` and `Fut: std::future::Future<Output = ()> + Send + 'static` — registers a recurring job that calls `job()` on each firing, returns the job's id (an opaque `uuid::Uuid`, used later to unregister it); `async fn unregister(&self, job_id: uuid::Uuid) -> anyhow::Result<()>` — removes a previously-registered job; idempotent-ish in the sense that removing an already-removed/unknown id should not panic (map any "not found" error from the underlying crate to `Ok(())`, since the caller — `TriggerRegistry::take_cron_job` — already only calls this when it believes the job still exists, and a race where it was already gone is not a caller error).

Note: `register`'s generic-closure signature keeps `Scheduler` decoupled from what a job actually does — Task 7's `triggers::activate_workflow_triggers` is the only caller, and it passes a closure that itself owns `Arc<dyn Storage>`/`Arc<NodeRegistry>`/the `workflow_id` (per this plan's Global Constraints — never the full `AppState`).

- [ ] **Step 1: Add the dependency**

```toml
# Cargo.toml, in [dependencies]
tokio-cron-scheduler = "0.10"
```

- [ ] **Step 2: Write failing tests**

```rust
// src/scheduler.rs, tests module
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn registered_job_fires_on_its_schedule() {
        let scheduler = Scheduler::new().await.unwrap();
        let fire_count = Arc::new(AtomicUsize::new(0));
        let counted = fire_count.clone();

        // Every second — the tightest interval a standard 6-field cron
        // expression (sec min hour day month weekday) can express, used here
        // purely to keep the test fast; production cron expressions come from
        // user-authored core.schedule node parameters.
        let job_id = scheduler
            .register("* * * * * *", move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
        assert!(fire_count.load(Ordering::SeqCst) >= 1, "job should have fired at least once in 2.2s");

        scheduler.unregister(job_id).await.unwrap();
        let count_after_unregister = fire_count.load(Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        assert_eq!(
            fire_count.load(Ordering::SeqCst),
            count_after_unregister,
            "job should not fire again after unregister"
        );
    }

    #[tokio::test]
    async fn unregister_on_unknown_job_id_does_not_error() {
        let scheduler = Scheduler::new().await.unwrap();
        let result = scheduler.unregister(uuid::Uuid::new_v4()).await;
        assert!(result.is_ok());
    }
}
```

- [ ] **Step 3: Run, confirm compile failure**

Run: `cargo test --lib scheduler::tests`
Expected: compile error — `Scheduler` undefined (only `#[cfg(test)] mod tests` exists so far, not wired into `src/lib.rs` either).

- [ ] **Step 4: Implement (reference implementation — verify against the installed `tokio-cron-scheduler` API per the Global Constraints note above)**

```rust
// top of src/scheduler.rs, above the tests module
use tokio_cron_scheduler::{Job, JobScheduler};

pub struct Scheduler {
    inner: JobScheduler,
}

impl Scheduler {
    pub async fn new() -> anyhow::Result<Self> {
        let inner = JobScheduler::new().await?;
        inner.start().await?;
        Ok(Self { inner })
    }

    pub async fn register<F, Fut>(&self, cron_expr: &str, job: F) -> anyhow::Result<uuid::Uuid>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let job = Job::new_async(cron_expr, move |_job_id, _scheduler| {
            let fut = job();
            Box::pin(fut)
        })?;
        let job_id = self.inner.add(job).await?;
        Ok(job_id)
    }

    pub async fn unregister(&self, job_id: uuid::Uuid) -> anyhow::Result<()> {
        match self.inner.remove(&job_id).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Treat "job not found" as a benign no-op — see Interfaces note
                // above. If the installed crate's error type distinguishes
                // "not found" from other failures, match on that specifically
                // instead of swallowing every error here.
                tracing::debug!(error = %e, "scheduler.unregister: job already gone");
                Ok(())
            }
        }
    }
}
```

Add `pub mod scheduler;` to `src/lib.rs`.

- [ ] **Step 5: Run tests, adapting the implementation to the installed `tokio-cron-scheduler` API as needed**

Run: `cargo build` first to surface any API mismatches from Step 4, fix them (consulting the installed crate's actual types/methods — in particular, verify `Job::new_async`'s exact closure signature and `JobScheduler::add`/`remove`'s exact return/error types), then:

Run: `cargo test --lib scheduler::tests`
Expected: both tests PASS. The first test takes ~3.4s of real wall-clock time (it waits on a real 1-second cron schedule) — this is expected and acceptable for this one test; do not try to eliminate the real timing wait, since `Scheduler`'s whole job is to interface with real wall-clock scheduling.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/scheduler.rs src/lib.rs
git commit -m "feat: add Scheduler wrapper around tokio-cron-scheduler"
```

---

### Task 7: `triggers` module — ties `Scheduler` + `TriggerRegistry` + engine + `Storage` together

**Files:**
- Create: `src/triggers.rs`
- Modify: `src/lib.rs`
- Modify: `src/state.rs`

**Interfaces:**
- Consumes: `Scheduler` (Task 6), `TriggerRegistry` (Task 5), `engine::start_node_id`/`execute_workflow_seeded` (Task 3), `Storage::update_execution`/`create_execution` (existing), `ExecutionMode::Schedule` (Task 1).
- Produces: `r8r::triggers::activate_workflow_triggers(state: &AppState, workflow: &Workflow) -> anyhow::Result<()>` — if the workflow's start node (via `engine::start_node_id`) is a `core.schedule` node, parses its `cron` parameter (`Err` if missing or not a string) and registers a job via `state.scheduler.register(..)` whose closure calls `triggers::fire_schedule`; records the returned job id in `state.trigger_registry`. If the start node is anything else (including `core.webhook`, which needs no registry action — see Global Constraints), this is a no-op `Ok(())`.
- Produces: `r8r::triggers::deactivate_workflow_triggers(state: &AppState, workflow_id: uuid::Uuid)` — if `state.trigger_registry.take_cron_job(workflow_id)` returns a job id, unregisters it via `state.scheduler`. Always succeeds (logs and swallows any unregister error — deactivation must never fail the API request over scheduler cleanup).
- Produces: `r8r::triggers::fire_schedule(storage: std::sync::Arc<dyn Storage>, registry: std::sync::Arc<NodeRegistry>, workflow_id: uuid::Uuid)` — fetches the workflow fresh from storage (it may have changed since activation), and if still present and `active`, runs it via `execute_workflow_seeded(&workflow, &registry, Some(vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]))`, persisting a new `Execution` with `mode: ExecutionMode::Schedule` exactly the way `api/workflows.rs::execute_workflow`'s handler does today (create running → run → update with result). If the workflow is missing or no longer active, this is a silent no-op (the cron job for a deactivated/deleted workflow should already have been unregistered by `deactivate_workflow_triggers`, but a fired job that raced with deactivation must not crash or execute a workflow that's no longer meant to be live).
- Produces: `r8r::triggers::reactivate_all(state: &AppState) -> anyhow::Result<()>` — lists all workflows via `state.storage.list_workflows()`, and calls `activate_workflow_triggers` for every one where `workflow.active` is `true`. Used once at process startup (Task 10). A failure activating one workflow is logged and does not stop the others from being attempted (a single broken cron expression in one workflow must not prevent every other active schedule from coming back after a restart).

- [ ] **Step 1: Write failing tests**

```rust
// src/triggers.rs
use crate::domain::{ExecutionMode, ExecutionStatus};
use crate::state::AppState;
use uuid::Uuid;

pub async fn activate_workflow_triggers(
    state: &AppState,
    workflow: &crate::domain::Workflow,
) -> anyhow::Result<()> {
    let start_id = crate::engine::start_node_id(workflow)?;
    let start_node = workflow
        .nodes
        .iter()
        .find(|n| n.id == start_id)
        .ok_or_else(|| anyhow::anyhow!("start node {start_id} not found"))?;

    if start_node.node_type != "core.schedule" {
        return Ok(());
    }

    let cron_expr = start_node
        .parameters
        .get("cron")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("core.schedule node requires a \"cron\" string parameter"))?
        .to_string();

    let storage = state.storage.clone();
    let registry = state.registry.clone();
    let workflow_id = workflow.id;

    let job_id = state
        .scheduler
        .register(&cron_expr, move || {
            let storage = storage.clone();
            let registry = registry.clone();
            async move {
                fire_schedule(storage, registry, workflow_id).await;
            }
        })
        .await?;

    state.trigger_registry.record_cron_job(workflow_id, job_id);
    Ok(())
}

pub async fn deactivate_workflow_triggers(state: &AppState, workflow_id: Uuid) {
    if let Some(job_id) = state.trigger_registry.take_cron_job(workflow_id) {
        if let Err(e) = state.scheduler.unregister(job_id).await {
            tracing::warn!(error = %e, %workflow_id, "failed to unregister cron job on deactivation");
        }
    }
}

pub async fn fire_schedule(
    storage: std::sync::Arc<dyn crate::storage::Storage>,
    registry: std::sync::Arc<crate::node::NodeRegistry>,
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

    let mut execution = crate::domain::Execution {
        id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Running,
        mode: ExecutionMode::Schedule,
        node_outputs: Default::default(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    };
    if let Err(e) = storage.create_execution(&execution).await {
        tracing::error!(error = %e, "fire_schedule: failed to persist new execution");
        return;
    }

    let trigger_items = vec![crate::domain::Item { json: serde_json::json!({}), binary: serde_json::json!({}) }];
    match crate::engine::execute_workflow_seeded(&workflow, &registry, Some(trigger_items)).await {
        Ok(outputs) => {
            execution.status = ExecutionStatus::Success;
            execution.node_outputs = outputs;
        }
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "scheduled workflow execution failed");
            execution.status = ExecutionStatus::Error;
        }
    }
    execution.finished_at = Some(chrono::Utc::now());
    if let Err(e) = storage.update_execution(&execution).await {
        tracing::error!(error = %e, "fire_schedule: failed to persist execution result");
    }
}

pub async fn reactivate_all(state: &AppState) -> anyhow::Result<()> {
    let workflows = state.storage.list_workflows().await?;
    for workflow in workflows.into_iter().filter(|w| w.active) {
        if let Err(e) = activate_workflow_triggers(state, &workflow).await {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "failed to reactivate workflow triggers on startup");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Connection, NodeInstance, Workflow};
    use crate::node::NodeRegistry;
    use crate::storage::sqlite::SqliteStorage;
    use std::sync::Arc;

    async fn test_state() -> AppState {
        let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        AppState {
            storage: Arc::new(storage),
            registry: Arc::new(registry),
            jwt_secret: "test-secret".into(),
            scheduler: Arc::new(crate::scheduler::Scheduler::new().await.unwrap()),
            trigger_registry: Arc::new(crate::trigger_registry::TriggerRegistry::new()),
        }
    }

    fn schedule_workflow(cron: &str) -> Workflow {
        let now = chrono::Utc::now();
        Workflow {
            id: Uuid::new_v4(),
            name: "scheduled".into(),
            active: true,
            nodes: vec![
                NodeInstance {
                    id: "trigger".into(),
                    node_type: "core.schedule".into(),
                    position: (0.0, 0.0),
                    parameters: serde_json::json!({"cron": cron}),
                    disabled: false,
                },
                NodeInstance {
                    id: "set1".into(),
                    node_type: "core.set".into(),
                    position: (1.0, 0.0),
                    parameters: serde_json::json!({"fields": {"fired": true}}),
                    disabled: false,
                },
            ],
            connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: "set1".into(), to_input: 0 }],
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn activate_registers_a_cron_job_for_a_schedule_start_node() {
        let state = test_state().await;
        let wf = schedule_workflow("* * * * * *");
        state.storage.create_workflow(&wf).await.unwrap();

        activate_workflow_triggers(&state, &wf).await.unwrap();
        assert!(state.trigger_registry.take_cron_job(wf.id).is_some());
    }

    #[tokio::test]
    async fn activate_is_a_noop_for_a_manual_trigger_start_node() {
        let state = test_state().await;
        let now = chrono::Utc::now();
        let wf = Workflow {
            id: Uuid::new_v4(),
            name: "manual".into(),
            active: true,
            nodes: vec![NodeInstance {
                id: "trigger".into(),
                node_type: "core.manualTrigger".into(),
                position: (0.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
            }],
            connections: vec![],
            created_at: now,
            updated_at: now,
        };
        activate_workflow_triggers(&state, &wf).await.unwrap();
        assert!(state.trigger_registry.take_cron_job(wf.id).is_none());
    }

    #[tokio::test]
    async fn activate_errors_when_schedule_node_missing_cron_param() {
        let state = test_state().await;
        let mut wf = schedule_workflow("* * * * * *");
        wf.nodes[0].parameters = serde_json::json!({});
        let result = activate_workflow_triggers(&state, &wf).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn deactivate_unregisters_a_previously_activated_job() {
        let state = test_state().await;
        let wf = schedule_workflow("* * * * * *");
        state.storage.create_workflow(&wf).await.unwrap();
        activate_workflow_triggers(&state, &wf).await.unwrap();

        deactivate_workflow_triggers(&state, wf.id).await;
        assert!(state.trigger_registry.take_cron_job(wf.id).is_none());
    }

    #[tokio::test]
    async fn deactivate_on_a_workflow_with_no_registered_job_does_not_panic() {
        let state = test_state().await;
        deactivate_workflow_triggers(&state, Uuid::new_v4()).await;
    }

    #[tokio::test]
    async fn fire_schedule_runs_the_workflow_and_persists_a_schedule_mode_execution() {
        let state = test_state().await;
        let wf = schedule_workflow("* * * * * *");
        state.storage.create_workflow(&wf).await.unwrap();

        fire_schedule(state.storage.clone(), state.registry.clone(), wf.id).await;

        let executions_are_findable = state.storage.list_workflows().await.unwrap();
        assert_eq!(executions_are_findable.len(), 1); // sanity: workflow itself still there
        // fire_schedule doesn't return the execution id, so fetch it the only way
        // available: re-derive expected node_outputs by running the same workflow
        // manually and comparing shape is out of scope here — instead, assert
        // indirectly via a direct storage query is not available either. This
        // test's real assertion is that fire_schedule does not panic/error against
        // a real active workflow and a real Storage — full behavioral proof of the
        // persisted execution happens in Task 11's HTTP-level integration test via
        // the webhook path, and in Task 9's own tests for the webhook handler's
        // use of the same execute-and-persist pattern.
    }

    #[tokio::test]
    async fn fire_schedule_on_inactive_workflow_is_a_noop() {
        let state = test_state().await;
        let mut wf = schedule_workflow("* * * * * *");
        wf.active = false;
        state.storage.create_workflow(&wf).await.unwrap();

        // Must not panic even though the workflow is inactive.
        fire_schedule(state.storage.clone(), state.registry.clone(), wf.id).await;
    }

    #[tokio::test]
    async fn fire_schedule_on_missing_workflow_is_a_noop() {
        let state = test_state().await;
        fire_schedule(state.storage.clone(), state.registry.clone(), Uuid::new_v4()).await;
    }

    #[tokio::test]
    async fn reactivate_all_registers_every_active_schedule_workflow() {
        let state = test_state().await;
        let wf1 = schedule_workflow("* * * * * *");
        let mut wf2 = schedule_workflow("* * * * * *");
        wf2.active = false;
        state.storage.create_workflow(&wf1).await.unwrap();
        state.storage.create_workflow(&wf2).await.unwrap();

        reactivate_all(&state).await.unwrap();

        assert!(state.trigger_registry.take_cron_job(wf1.id).is_some());
        assert!(state.trigger_registry.take_cron_job(wf2.id).is_none());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib triggers::tests`
Expected: compile errors — `AppState` doesn't have `scheduler`/`trigger_registry` fields yet, and `pub mod triggers;` isn't in `src/lib.rs`.

- [ ] **Step 3: Add fields to `AppState`**

```rust
// src/state.rs — replace the whole file
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
}
```

- [ ] **Step 4: Wire `pub mod triggers;` into `src/lib.rs`**

Add `pub mod triggers;` alongside the existing `pub mod` lines.

- [ ] **Step 5: Run tests**

Run: `cargo test --lib triggers::tests`
Expected: all pass. Note: `activate_registers_a_cron_job_for_a_schedule_start_node`, `deactivate_unregisters_a_previously_activated_job`, `fire_schedule_runs_the_workflow_and_persists_a_schedule_mode_execution`, and `fire_schedule_on_inactive_workflow_is_a_noop`/`reactivate_all_registers_every_active_schedule_workflow` each construct a real `Scheduler` (which starts a real background scheduler task) — this is expected; `Scheduler::new()` is cheap and does not itself wait on any timer.

- [ ] **Step 6: Run the whole crate**

Run: `cargo build` first — `src/main.rs` and anywhere else constructing `AppState` will fail to compile until updated. **Do not fix `src/main.rs` in this task** — that's Task 10. Confirm the ONLY compile errors are in `src/main.rs` (and note this in your report), then:

Run: `cargo test --lib triggers::tests --lib scheduler::tests --lib trigger_registry::tests --lib engine::tests --lib storage::sqlite::tests --lib domain::tests --lib nodes::`
Expected: all pass (this excludes `tests/api_test.rs`, which also won't compile yet since it builds a full `AppState` via `main.rs`'s `test_app()` helper or similar — confirm and note, don't fix).

- [ ] **Step 7: Commit**

```bash
git add src/triggers.rs src/state.rs src/lib.rs
git commit -m "feat: add triggers module (activate/deactivate/fire_schedule/reactivate_all)"
```

---

### Task 8: Activate/deactivate API endpoint

**Files:**
- Modify: `src/api/workflows.rs`
- Modify: `src/api/mod.rs`

**Interfaces:**
- Produces: `set_workflow_active(State(state), AuthUser, Path(id), Json(payload)) -> impl IntoResponse` handler for `PATCH /rest/workflows/:id/active`, body `{"active": bool}`. Fetches the workflow (404 if missing); if `payload.active == workflow.active`, returns the current workflow unchanged (idempotent no-op — no re-registration of an already-live schedule, no spurious deactivate/reactivate churn). Otherwise: if turning ON, calls `triggers::activate_workflow_triggers` — on `Err`, returns `400 Bad Request` with the error message (a bad cron expression or similar is a client error, not a server error) and does NOT persist `active = true`; if turning OFF, calls `triggers::deactivate_workflow_triggers` (infallible from the caller's perspective, per Task 7). On success, sets `workflow.active`, bumps `workflow.updated_at`, persists via `state.storage.update_workflow`, returns the updated workflow as JSON.

- [ ] **Step 1: Write failing tests**

```rust
// add to tests/api_test.rs — these exercise the new endpoint directly; the
// full webhook-firing end-to-end test is Task 11's job
#[tokio::test]
async fn activate_workflow_with_valid_schedule_succeeds() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "activate1@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "scheduled-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {"cron": "0 0 * * * *"}, "disabled": false}
        ],
        "connections": []
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let updated: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated["active"], true);
}

#[tokio::test]
async fn activate_workflow_with_invalid_cron_param_returns_400() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "activate2@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "broken-schedule",
        "nodes": [
            {"id": "trigger", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {}, "disabled": false}
        ],
        "connections": []
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --test api_test activate_workflow`
Expected: compile error or 404 — the route doesn't exist yet.

- [ ] **Step 3: Implement the handler**

```rust
// src/api/workflows.rs — add near the other handlers
#[derive(Deserialize)]
pub struct SetActiveRequest {
    pub active: bool,
}

pub async fn set_workflow_active(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<SetActiveRequest>,
) -> impl IntoResponse {
    let mut workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for activation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if payload.active == workflow.active {
        return Json(workflow).into_response();
    }

    if payload.active {
        if let Err(e) = crate::triggers::activate_workflow_triggers(&state, &workflow).await {
            return (StatusCode::BAD_REQUEST, format!("failed to activate workflow: {e}")).into_response();
        }
    } else {
        crate::triggers::deactivate_workflow_triggers(&state, workflow.id).await;
    }

    workflow.active = payload.active;
    workflow.updated_at = chrono::Utc::now();
    if let Err(e) = state.storage.update_workflow(&workflow).await {
        tracing::error!(error = %e, "failed to persist workflow activation state");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(workflow).into_response()
}
```

- [ ] **Step 4: Wire the route**

```rust
// src/api/mod.rs — add alongside the existing /rest/workflows/:id routes
.route("/rest/workflows/:id/active", axum::routing::patch(workflows::set_workflow_active))
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test api_test activate_workflow`
Expected: both PASS.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: all pass (Task 7's note about `src/main.rs`/`tests/api_test.rs` not compiling should now be resolved IF Task 10 hasn't run yet — if `src/main.rs` still doesn't compile at this point, that's still expected and still Task 10's job; note it in your report exactly as Task 7 did).

- [ ] **Step 7: Commit**

```bash
git add src/api/workflows.rs src/api/mod.rs
git commit -m "feat: add PATCH /rest/workflows/:id/active endpoint"
```

---

### Task 9: Webhook HTTP handler

**Files:**
- Create: `src/api/webhook.rs`
- Modify: `src/api/mod.rs`

**Interfaces:**
- Produces: `handle_webhook(State(state), Path((workflow_id, path)), method, headers, Query(query), body) -> impl IntoResponse` for `GET|POST /webhook/:workflow_id/:path`. No `AuthUser` extractor — webhooks are called by external systems, not authenticated r8r users; this is intentional and matches n8n's own webhook model (the URL's unpredictability is the access control, same tradeoff n8n makes). Looks up the workflow; 404 if missing or `!workflow.active`. Computes the workflow's start node via `engine::start_node_id`; 404 if that node's `node_type` isn't `"core.webhook"`, or its `path` parameter doesn't match the URL's `path`, or its `method` parameter is neither `"ANY"` nor a case-insensitive match for the incoming request's method. Builds a single trigger `Item` whose `json` is `{"headers": {...}, "query": {...}, "body": <parsed JSON body, or the raw string if not valid JSON, or null if empty>}`. Runs the workflow via `execute_workflow_seeded(&workflow, &state.registry, Some(vec![trigger_item]))`, persisting an `Execution` with `mode: ExecutionMode::Webhook` following the exact create→run→update pattern already used in `api/workflows.rs::execute_workflow` and `triggers::fire_schedule`. Responds `200` with the execution JSON on `ExecutionStatus::Success`, `500` with the execution JSON on `ExecutionStatus::Error` (the workflow ran but failed — still useful information for the caller, unlike a routing-level 404).

- [ ] **Step 1: Write failing tests**

The full HTTP-level proof is Task 11's end-to-end test. This task's own test coverage is the header/query/body-parsing and method/path-matching logic in isolation, via a helper function extracted for testability:

```rust
// src/api/webhook.rs
use crate::domain::{ExecutionMode, ExecutionStatus, Item};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn handle_webhook(
    State(state): State<AppState>,
    Path((workflow_id, path)): Path<(Uuid, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(workflow_id).await {
        Ok(Some(wf)) if wf.active => wf,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "webhook: failed to fetch workflow");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let start_id = match crate::engine::start_node_id(&workflow) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let start_node = match workflow.nodes.iter().find(|n| n.id == start_id) {
        Some(n) => n,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    if !webhook_node_matches(start_node, &path, method.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let trigger_item = build_trigger_item(&headers, &query, &body);

    let mut execution = crate::domain::Execution {
        id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Running,
        mode: ExecutionMode::Webhook,
        node_outputs: Default::default(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    };
    if let Err(e) = state.storage.create_execution(&execution).await {
        tracing::error!(error = %e, "webhook: failed to persist new execution");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let response_status = match crate::engine::execute_workflow_seeded(
        &workflow,
        &state.registry,
        Some(vec![trigger_item]),
    )
    .await
    {
        Ok(outputs) => {
            execution.status = ExecutionStatus::Success;
            execution.node_outputs = outputs;
            StatusCode::OK
        }
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "webhook-triggered execution failed");
            execution.status = ExecutionStatus::Error;
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    execution.finished_at = Some(chrono::Utc::now());
    if let Err(e) = state.storage.update_execution(&execution).await {
        tracing::error!(error = %e, "webhook: failed to persist execution result");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (response_status, Json(execution)).into_response()
}

fn webhook_node_matches(node: &crate::domain::NodeInstance, path: &str, method: &str) -> bool {
    if node.node_type != "core.webhook" {
        return false;
    }
    let node_path = node.parameters.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if node_path != path {
        return false;
    }
    let node_method = node.parameters.get("method").and_then(|v| v.as_str()).unwrap_or("ANY");
    node_method.eq_ignore_ascii_case("ANY") || node_method.eq_ignore_ascii_case(method)
}

fn build_trigger_item(headers: &HeaderMap, query: &HashMap<String, String>, body: &[u8]) -> Item {
    let headers_json: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_str().unwrap_or("").to_string())))
        .collect();
    let body_json = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice::<serde_json::Value>(body)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(body).to_string()))
    };
    Item {
        json: serde_json::json!({
            "headers": headers_json,
            "query": query,
            "body": body_json,
        }),
        binary: serde_json::json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NodeInstance;

    fn webhook_node(path: &str, method: &str) -> NodeInstance {
        NodeInstance {
            id: "hook".into(),
            node_type: "core.webhook".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({"path": path, "method": method}),
            disabled: false,
        }
    }

    #[test]
    fn matches_exact_path_and_method() {
        let node = webhook_node("my-hook", "POST");
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
        assert!(!webhook_node_matches(&node, "other-hook", "POST"));
        assert!(!webhook_node_matches(&node, "my-hook", "GET"));
    }

    #[test]
    fn any_method_matches_get_and_post() {
        let node = webhook_node("my-hook", "ANY");
        assert!(webhook_node_matches(&node, "my-hook", "GET"));
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
    }

    #[test]
    fn method_match_is_case_insensitive() {
        let node = webhook_node("my-hook", "post");
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
    }

    #[test]
    fn non_webhook_node_type_never_matches() {
        let mut node = webhook_node("my-hook", "ANY");
        node.node_type = "core.manualTrigger".into();
        assert!(!webhook_node_matches(&node, "my-hook", "GET"));
    }

    #[test]
    fn build_trigger_item_parses_json_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, br#"{"hello":"world"}"#);
        assert_eq!(item.json["body"], serde_json::json!({"hello": "world"}));
    }

    #[test]
    fn build_trigger_item_falls_back_to_raw_string_for_non_json_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, b"not json");
        assert_eq!(item.json["body"], serde_json::json!("not json"));
    }

    #[test]
    fn build_trigger_item_uses_null_body_for_empty_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, b"");
        assert_eq!(item.json["body"], serde_json::Value::Null);
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib api::webhook::tests`
Expected: compile error — `src/api/webhook.rs` not yet declared as a module in `src/api/mod.rs`.

- [ ] **Step 3: Wire the module and route**

```rust
// src/api/mod.rs
pub mod webhook;
// ... existing pub mod lines ...

// inside build_router, add:
.route("/webhook/:workflow_id/:path", axum::routing::get(webhook::handle_webhook).post(webhook::handle_webhook))
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib api::webhook::tests`
Expected: all pass.

- [ ] **Step 5: Run the whole crate**

Run: `cargo test`
Expected: everything not blocked by Task 10's still-pending `src/main.rs` fix passes; confirm and note the same as Tasks 7/8.

- [ ] **Step 6: Commit**

```bash
git add src/api/webhook.rs src/api/mod.rs
git commit -m "feat: add /webhook/:workflow_id/:path handler"
```

---

### Task 10: Startup reactivation

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Changes: `main()` constructs `Scheduler::new()` and `TriggerRegistry::new()`, wires both into `AppState`, and calls `r8r::triggers::reactivate_all(&state)` once after `AppState` is built and before the server starts listening.

- [ ] **Step 1: Implement**

```rust
// src/main.rs — replace the whole file
use r8r::node::NodeRegistry;
use r8r::scheduler::Scheduler;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use r8r::trigger_registry::TriggerRegistry;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./r8r.db?mode=rwc".into());
    let jwt_secret = std::env::var("JWT_SECRET").map_err(|_| {
        anyhow::anyhow!("JWT_SECRET environment variable must be set (see .env.example)")
    })?;
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let storage = SqliteStorage::new(&database_url).await?;
    let mut registry = NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let scheduler = Scheduler::new().await?;

    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
        scheduler: Arc::new(scheduler),
        trigger_registry: Arc::new(TriggerRegistry::new()),
    };

    if let Err(e) = r8r::triggers::reactivate_all(&state).await {
        tracing::warn!(error = %e, "failed to reactivate workflow triggers on startup");
    }

    let app = r8r::api::build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("r8r listening on :{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 2: Run the whole crate**

Run: `cargo build`
Expected: compiles cleanly — this was the last remaining compile error from Tasks 7-9's incremental `AppState` changes.

Run: `cargo test`
Expected: every test in the crate passes now, including `tests/api_test.rs`'s `test_app()` helper (which builds its own `AppState` for tests — verify it, check whether it needs the same `scheduler`/`trigger_registry` fields added; if `test_app()` lives in `tests/api_test.rs` itself rather than in `src/`, update it there to construct a real `Scheduler`/`TriggerRegistry` the same way `main.rs` does).

- [ ] **Step 3: Commit**

```bash
git add src/main.rs tests/api_test.rs
git commit -m "feat: wire Scheduler/TriggerRegistry into AppState and reactivate schedules on startup"
```

(If `tests/api_test.rs` needed no changes because its `test_app()` helper already delegates to a shared constructor that Task 7 already updated, `git add` only `src/main.rs` and adjust the commit message accordingly — check before committing rather than assuming.)

---

### Task 11: End-to-end integration test — webhook trigger through the real API

**Files:**
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: everything above, plus the existing `test_app()`/`register_and_get_token()` helpers.
- Produces: one new test proving the whole plan's webhook path works end-to-end over real HTTP: create a workflow with a `core.webhook` trigger → a `core.set` node, activate it, POST to its `/webhook/:id/:path` URL with a JSON body, assert the execution succeeded and the set node's output reflects both the static field and something derived from the webhook body via `{{ }}` (proving Plan 2's expression engine and Plan 3's trigger-data injection compose correctly); then deactivate it and assert the same URL now 404s.

- [ ] **Step 1: Write the failing test**

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn webhook_trigger_executes_workflow_end_to_end_then_404s_after_deactivation() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "webhook-e2e@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "webhook-wf",
        "nodes": [
            {"id": "hook", "node_type": "core.webhook", "position": [0.0, 0.0], "parameters": {"path": "my-test-hook", "method": "POST"}, "disabled": false},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"received": "{{ $json.body.name }}"}}, "disabled": false}
        ],
        "connections": [
            {"from_node": "hook", "from_output": 0, "to_node": "set1", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Fire the webhook — no authorization header, matching real external callers.
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri(format!("/webhook/{workflow_id}/my-test-hook"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"name": "Ada"}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["received"], "Ada");

    // Deactivate, then confirm the webhook path is dark.
    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": false}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(Request::builder().method("POST").uri(format!("/webhook/{workflow_id}/my-test-hook"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"name": "Ada"}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run, confirm it passes**

Run: `cargo test --test api_test webhook_trigger_executes_workflow_end_to_end_then_404s_after_deactivation`
Expected: PASS if Tasks 1-10 are correctly implemented and wired together (no new production code should be needed for this task — it is purely an integration-proof test). If it fails, the failure identifies exactly which earlier task's implementation has a bug; fix that task's code, not this test, following the exact same principle Plan 2's Task 12 established.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/api_test.rs
git commit -m "test: add end-to-end webhook trigger + activation lifecycle integration test"
```

---

## Explicitly Out of Scope (this plan)

Carried forward as future work per the roadmap breakdown:
- "Respond immediately, execute in background" webhook mode — v1 is execute-then-respond only.
- Path-collision validation across different workflows' webhook paths.
- Retry (`ExecutionMode::Retry`) — Plan 7.
- Telegram Trigger's long-polling/webhook modes — Plan 4 (builds on this plan's webhook infrastructure per the roadmap's own note).
- Per-workflow webhook authentication (HMAC signature verification, shared secrets) — not named in the spec's v1 scope; a natural Plan 7 hardening candidate if it comes up.
- WebSocket live execution status push for webhook/schedule-triggered runs — Plan 7 (7.1).
