use crate::domain::{Execution, ExecutionMode, ExecutionStatus, Item, Workflow};
use crate::engine::ExecutionObserver;
use crate::node::NodeRegistry;
use crate::storage::Storage;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

/// Capacity of the global execution-events broadcast channel (see
/// `AppState.execution_events`). Chosen generously relative to a typical
/// workflow's node count so a slow WS consumer doesn't miss events under
/// normal load; a consumer that falls behind by more than this many events
/// sees a `Lagged` gap (handled by skipping ahead), not a crash.
pub const EXECUTION_EVENTS_CAPACITY: usize = 256;

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
        if self.events.receiver_count() == 0 {
            return;
        }
        let _ = self.events.send(ExecutionEvent {
            execution_id: self.execution_id,
            workflow_id: self.workflow_id,
            kind,
        });
    }

    fn into_execution(self) -> Execution {
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

/// Persists a `Running` execution, then runs the workflow and records its
/// outcome in a spawned task (Plan 8.7). The returned handle yields the
/// final `Execution`; dropping it detaches the run, which still completes
/// -- no caller going away can cancel a run mid-node.
pub async fn start_execution(
    storage: Arc<dyn Storage>,
    events: broadcast::Sender<ExecutionEvent>,
    registry: Arc<NodeRegistry>,
    workflow: Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<(Execution, tokio::task::JoinHandle<Execution>)> {
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
    tracing::info!(
        execution_id = %execution.id,
        workflow_id = %workflow.id,
        workflow_name = %workflow.name,
        mode = ?execution.mode,
        "execution started"
    );

    let started = execution.clone();
    let handle = tokio::spawn(async move {
        let clock = std::time::Instant::now();
        let tracker = LiveExecutionTracker::new(execution, storage.clone(), events.clone());
        let result =
            crate::engine::execute_workflow_seeded(&workflow, &registry, trigger_items, &credentials, &tracker).await;

        let mut final_execution = tracker.into_execution();
        let duration_ms = clock.elapsed().as_millis() as u64;
        match &result {
            Ok(outputs) => {
                final_execution.status = ExecutionStatus::Success;
                final_execution.node_outputs = outputs.clone();
                tracing::info!(
                    execution_id = %final_execution.id,
                    workflow_name = %workflow.name,
                    status = ?final_execution.status,
                    duration_ms,
                    "execution finished"
                );
            }
            Err(e) => {
                final_execution.status = ExecutionStatus::Error;
                tracing::warn!(
                    execution_id = %final_execution.id,
                    workflow_id = %workflow.id,
                    workflow_name = %workflow.name,
                    duration_ms,
                    error = %e,
                    "execution failed"
                );
            }
        }
        final_execution.finished_at = Some(chrono::Utc::now());
        if let Err(e) = storage.update_execution(&final_execution).await {
            tracing::error!(error = %e, execution_id = %final_execution.id, "failed to persist final execution result");
        }

        let _ = events.send(ExecutionEvent {
            execution_id: final_execution.id,
            workflow_id: workflow.id,
            kind: ExecutionEventKind::ExecutionFinished { status: final_execution.status.clone() },
        });
        final_execution
    });
    Ok((started, handle))
}

/// Starts a run and waits for it. Used where the caller wants the result
/// inline (schedule triggers, tests).
pub async fn run_and_track_execution(
    storage: &Arc<dyn Storage>,
    events: &broadcast::Sender<ExecutionEvent>,
    registry: &Arc<NodeRegistry>,
    workflow: &Workflow,
    mode: ExecutionMode,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<Uuid, serde_json::Value>,
) -> anyhow::Result<Execution> {
    let (_, handle) = start_execution(
        storage.clone(),
        events.clone(),
        registry.clone(),
        workflow.clone(),
        mode,
        trigger_items,
        credentials.clone(),
    )
    .await?;
    handle.await.map_err(|e| anyhow::anyhow!("execution task failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::{run_and_track_execution, ExecutionEventKind};
    use crate::domain::{Connection, Execution, ExecutionMode, ExecutionStatus, NodeInstance, Workflow};
    use crate::node::NodeRegistry;
    use crate::storage::sqlite::SqliteStorage;
    use crate::storage::Storage;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    fn registry() -> Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        Arc::new(r)
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
                    settings: Default::default(),
                },
                NodeInstance {
                    id: "set1".into(),
                    node_type: "core.set".into(),
                    position: (1.0, 0.0),
                    parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
                    disabled: false,
                    settings: Default::default(),
                },
            ],
            connections: vec![Connection {
                from_node: "trigger".into(),
                from_output: 0,
                to_node: "set1".into(),
                to_input: 0,
                error: false,
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
        async fn any_user_exists(&self) -> anyhow::Result<bool> {
            self.inner.any_user_exists().await
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
        async fn update_credential(&self, credential: &crate::domain::Credential) -> anyhow::Result<bool> {
            self.inner.update_credential(credential).await
        }
        async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool> {
            self.inner.delete_credential(id).await
        }
        async fn create_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<()> {
            self.inner.create_tool(tool).await
        }
        async fn get_tool(&self, id: uuid::Uuid) -> anyhow::Result<Option<crate::domain::Tool>> {
            self.inner.get_tool(id).await
        }
        async fn list_tools(&self) -> anyhow::Result<Vec<crate::domain::Tool>> {
            self.inner.list_tools().await
        }
        async fn update_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<bool> {
            self.inner.update_tool(tool).await
        }
        async fn delete_tool(&self, id: uuid::Uuid) -> anyhow::Result<bool> {
            self.inner.delete_tool(id).await
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

    use super::start_execution;

    /// Sleeps `ms` before passing its input through.
    struct SleepNode {
        ms: u64,
    }

    #[async_trait::async_trait]
    impl crate::node::Node for SleepNode {
        fn type_name(&self) -> &'static str {
            "test.sleep"
        }
        fn display_name(&self) -> &'static str {
            "Sleep"
        }
        fn description(&self) -> &'static str {
            "Test-only node that sleeps."
        }
        fn category(&self) -> crate::node::NodeCategory {
            crate::node::NodeCategory::Action
        }
        async fn execute(&self, ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            tokio::time::sleep(std::time::Duration::from_millis(self.ms)).await;
            Ok(vec![ctx.input_items.clone()])
        }
    }

    fn sleepy_setup(ms: u64) -> (Arc<NodeRegistry>, Workflow) {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r.register(Box::new(SleepNode { ms }));
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "test.sleep".into();
        (Arc::new(r), wf)
    }

    async fn memory_storage() -> Arc<dyn Storage> {
        Arc::new(SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap())
    }

    #[tokio::test]
    async fn start_execution_returns_while_the_run_is_still_running() {
        let storage = memory_storage().await;
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let (registry, wf) = sleepy_setup(200);
        storage.create_workflow(&wf).await.unwrap();

        let (started, handle) = start_execution(storage.clone(), events, registry, wf, ExecutionMode::Manual, None, HashMap::new())
            .await
            .unwrap();
        assert_eq!(started.status, ExecutionStatus::Running);
        assert_eq!(storage.get_execution(started.id).await.unwrap().unwrap().status, ExecutionStatus::Running);

        let finished = handle.await.unwrap();
        assert_eq!(finished.id, started.id);
        assert_eq!(finished.status, ExecutionStatus::Success);
        assert_eq!(storage.get_execution(started.id).await.unwrap().unwrap().status, ExecutionStatus::Success);
    }

    #[tokio::test]
    async fn dropping_the_handle_does_not_cancel_the_run() {
        let storage = memory_storage().await;
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let (registry, wf) = sleepy_setup(100);
        storage.create_workflow(&wf).await.unwrap();

        let (started, handle) = start_execution(storage.clone(), events, registry, wf, ExecutionMode::Manual, None, HashMap::new())
            .await
            .unwrap();
        drop(handle);

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let status = storage.get_execution(started.id).await.unwrap().unwrap().status;
                if status != ExecutionStatus::Running {
                    assert_eq!(status, ExecutionStatus::Success);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("detached run should still finish");
    }
}
