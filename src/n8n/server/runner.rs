//! Running workflows inside the server: persistence and save policies,
//! push messages, cancellation, waiting and resuming, error workflows and
//! sub-workflows (spec §6.2, §7.2).

use super::{ApiError, N8n};
use crate::n8n::engine::{self, ExecuteOptions, Hooks, ResponseSlot, WaitState};
use crate::n8n::node::{Mode, NodeError, NodeResult, SubWorkflowRunner};
use crate::n8n::types::{status, Item, SourceData};
use crate::n8n::workflow::Workflow;
use serde_json::{json, Map, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

pub struct RunRequest {
    pub workflow: Value,
    pub mode: Mode,
    pub start_items: Option<Vec<Item>>,
    pub start_node: Option<String>,
    pub destination_node: Option<String>,
    pub previous_run_data: Option<Map<String, Value>>,
    pub start_source: Option<Vec<Option<SourceData>>>,
    pub use_pin_data: bool,
    pub push_ref: Option<String>,
    pub response: Option<ResponseSlot>,
    pub retry_of: Option<String>,
    pub parent_execution: Option<String>,
    /// Run under this existing execution row (a resumed wait, or a job a
    /// worker took), keeping its start time when given.
    pub resume: Option<(i64, Option<String>)>,
    /// Run in this process even in queue mode (workers, and executions that
    /// need a live connection back to the caller).
    pub local: bool,
}

impl RunRequest {
    pub fn new(workflow: Value, mode: Mode) -> Self {
        Self {
            workflow,
            mode,
            start_items: None,
            start_node: None,
            destination_node: None,
            previous_run_data: None,
            start_source: None,
            use_pin_data: false,
            push_ref: None,
            response: None,
            retry_of: None,
            parent_execution: None,
            resume: None,
            local: false,
        }
    }

    /// The job a worker runs for this request (queue mode).
    fn to_job(&self, execution_id: i64) -> Value {
        json!({
            "executionId": execution_id,
            "workflow": self.workflow,
            "mode": self.mode.as_str(),
            "startItems": self.start_items,
            "startNode": self.start_node,
            "destinationNode": self.destination_node,
            "previousRunData": self.previous_run_data,
            "startSource": self.start_source,
            "usePinData": self.use_pin_data,
            "retryOf": self.retry_of,
            "parentExecution": self.parent_execution,
            "startedAt": self.resume.as_ref().and_then(|r| r.1.clone()),
        })
    }

    fn from_job(job: &Value) -> anyhow::Result<Self> {
        let id = job["executionId"].as_i64().ok_or_else(|| anyhow::anyhow!("job without an execution id"))?;
        let mut req = RunRequest::new(job["workflow"].clone(), mode_from_str(job["mode"].as_str().unwrap_or("webhook")));
        req.start_items = serde_json::from_value(job["startItems"].clone()).ok().flatten();
        req.start_node = job["startNode"].as_str().map(String::from);
        req.destination_node = job["destinationNode"].as_str().map(String::from);
        req.previous_run_data = job["previousRunData"].as_object().cloned();
        req.start_source = serde_json::from_value(job["startSource"].clone()).ok().flatten();
        req.use_pin_data = job["usePinData"].as_bool().unwrap_or(false);
        req.retry_of = job["retryOf"].as_str().map(String::from);
        req.parent_execution = job["parentExecution"].as_str().map(String::from);
        req.resume = Some((id, job["startedAt"].as_str().map(String::from)));
        req.local = true;
        Ok(req)
    }
}

/// Whether a request goes to the queue in queue mode. Editor runs and
/// sub-workflows stay in the calling process (as in n8n), and so does
/// anything that must answer a waiting HTTP caller from inside the run.
fn queueable(req: &RunRequest) -> bool {
    !req.local && req.response.is_none() && matches!(req.mode, Mode::Webhook | Mode::Trigger | Mode::Error | Mode::Retry)
}

/// Queue mode: records the execution as `new`, enqueues it, and follows
/// its row until a worker has finished it.
async fn enqueue(n8n: &Arc<N8n>, queue: &Arc<dyn crate::n8n::queue::Queue>, req: RunRequest) -> anyhow::Result<RunHandle> {
    let execution_id = match &req.resume {
        Some((id, _)) => {
            n8n.store.transition_execution(*id, status::RUNNING, "new").await?;
            *id
        }
        None => n8n.store.insert_execution(&req.workflow, req.mode.as_str(), req.retry_of.as_deref(), req.parent_execution.as_deref(), "new").await?,
    };
    queue.push(&req.to_job(execution_id)).await?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let store = n8n.store.clone();
    tokio::spawn(async move {
        let outcome = loop {
            match store.get_execution(execution_id).await {
                Ok(Some(row)) if row.status == "new" || row.status == status::RUNNING => {}
                Ok(Some(row)) => {
                    let irun = json!({
                        "data": row.data.clone().unwrap_or(json!({"resultData": {"runData": {}}})),
                        "mode": row.mode,
                        "status": row.status,
                        "startedAt": row.started_at,
                        "stoppedAt": row.stopped_at,
                        "waitTill": row.wait_till,
                        "finished": row.finished,
                    });
                    break RunOutcome { execution_id, status: row.status, irun, saved: true };
                }
                // The worker ran it and, by the save policy, didn't keep it.
                Ok(None) => break RunOutcome { execution_id, status: status::SUCCESS.into(), irun: json!({"data": {"resultData": {"runData": {}}}}), saved: false },
                Err(_) => {}
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        let _ = tx.send(Arc::new(outcome));
    });
    Ok(RunHandle { execution_id, done: rx })
}

/// Runs a job taken from the queue (worker side) to completion.
pub async fn run_job(n8n: &Arc<N8n>, job: &Value) -> anyhow::Result<()> {
    let req = RunRequest::from_job(job)?;
    let id = req.resume.as_ref().map(|r| r.0).unwrap_or_default();
    n8n.store.transition_execution(id, "new", status::RUNNING).await?;
    let handle = start(n8n, req).await?;
    let _ = handle.done.await;
    Ok(())
}

/// How an execution ended (or paused).
pub struct RunOutcome {
    pub execution_id: i64,
    pub status: String,
    /// n8n's `IRun`.
    pub irun: Value,
    pub saved: bool,
}

impl RunOutcome {
    pub fn last_node(&self) -> Option<&str> {
        self.irun.pointer("/data/resultData/lastNodeExecuted").and_then(Value::as_str)
    }

    pub fn error(&self) -> Option<&Value> {
        self.irun.pointer("/data/resultData/error")
    }

    /// Items on output 0 of the last node's last run.
    pub fn last_output(&self) -> Vec<Item> {
        let Some(last) = self.last_node() else { return vec![] };
        self.irun
            .pointer(&format!("/data/resultData/runData/{}", last.replace('~', "~0").replace('/', "~1")))
            .and_then(Value::as_array)
            .and_then(|runs| runs.last())
            .and_then(|t| t.pointer("/data/main/0"))
            .and_then(|items| serde_json::from_value(items.clone()).ok())
            .unwrap_or_default()
    }
}

pub struct RunHandle {
    pub execution_id: i64,
    pub done: tokio::sync::oneshot::Receiver<Arc<RunOutcome>>,
}

pub fn mode_from_str(s: &str) -> Mode {
    match s {
        "cli" => Mode::Cli,
        "manual" => Mode::Manual,
        "webhook" => Mode::Webhook,
        "trigger" => Mode::Trigger,
        "integrated" => Mode::Integrated,
        "retry" => Mode::Retry,
        "error" => Mode::Error,
        _ => Mode::Webhook,
    }
}

fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Webhook waits have no deadline; n8n stores this far-future date.
pub const WAIT_FOREVER: &str = "3000-01-01T00:00:00.000Z";

struct PushHooks {
    n8n: Weak<N8n>,
    push_ref: Option<String>,
    execution_id: String,
    workflow_id: String,
}

#[async_trait::async_trait]
impl Hooks for PushHooks {
    async fn node_before(&self, node: &str, task: &Value) {
        tracing::debug!(executionId = %self.execution_id, workflowId = %self.workflow_id, node = %node, "Start executing node");
        if let Some(n8n) = self.n8n.upgrade() {
            n8n.send_push(self.push_ref.as_deref(), "nodeExecuteBefore", json!({"executionId": self.execution_id, "nodeName": node, "data": task}));
        }
    }
    async fn node_after(&self, node: &str, task: &Value) {
        tracing::debug!(executionId = %self.execution_id, workflowId = %self.workflow_id, node = %node, "Finished executing node");
        if let Some(n8n) = self.n8n.upgrade() {
            let items = task.pointer("/data/main/0").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            n8n.send_push(
                self.push_ref.as_deref(),
                "nodeExecuteAfter",
                json!({"executionId": self.execution_id, "nodeName": node, "data": task, "itemCountByConnectionType": {"main": [items]}}),
            );
        }
    }
}

/// Whether a finished execution is stored, per workflow settings and the
/// `EXECUTIONS_DATA_SAVE_*` defaults.
fn should_save(workflow: &Value, mode: Mode, status_: &str) -> bool {
    let settings = &workflow["settings"];
    let env = |name: &str, default: &str| std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.to_string());
    if status_ == status::WAITING {
        return true;
    }
    if mode == Mode::Manual {
        return match &settings["saveManualExecutions"] {
            Value::Bool(b) => *b,
            _ => env("EXECUTIONS_DATA_SAVE_MANUAL_EXECUTIONS", "true") != "false",
        };
    }
    let (key, var) = if status_ == status::SUCCESS {
        ("saveDataSuccessExecution", "EXECUTIONS_DATA_SAVE_ON_SUCCESS")
    } else {
        ("saveDataErrorExecution", "EXECUTIONS_DATA_SAVE_ON_ERROR")
    };
    match settings[key].as_str() {
        Some("all") => true,
        Some("none") => false,
        _ => env(var, "all") != "none",
    }
}

/// A member's workflow may only use credentials they own; owners and
/// admins may use any (n8n's credentials permission check).
pub(super) async fn check_credential_access(n8n: &N8n, workflow: &Value) -> Result<(), String> {
    let Some(id) = workflow["id"].as_str() else { return Ok(()) };
    let Ok(Some(row)) = n8n.store.workflow_row(id).await else { return Ok(()) };
    let owner = match &row.owner_id {
        Some(o) => n8n.store.get_user(o).await.ok().flatten(),
        None => None,
    };
    let Some(owner) = owner else { return Ok(()) };
    if owner.is_admin() {
        return Ok(());
    }
    for node in workflow["nodes"].as_array().into_iter().flatten() {
        for (_, reference) in node["credentials"].as_object().into_iter().flatten() {
            let Some(cred_id) = reference["id"].as_str() else { continue };
            let cred_owner = n8n.store.credential_owner(cred_id).await.ok().flatten();
            if cred_owner.as_deref() != Some(owner.id.as_str()) {
                return Err(format!(
                    "Node \"{}\" does not have access to the credential \"{}\"",
                    node["name"].as_str().unwrap_or_default(),
                    reference["name"].as_str().unwrap_or(cred_id)
                ));
            }
        }
    }
    Ok(())
}

impl N8n {
    pub fn send_push(&self, push_ref: Option<&str>, kind: &str, data: Value) {
        let _ = self.push.send(super::push::PushMessage { push_ref: push_ref.map(String::from), body: json!({"type": kind, "data": data}) });
    }
}

/// Starts an execution in the background. (Boxed: error workflows start
/// executions from inside an execution.)
pub fn start(n8n: &Arc<N8n>, req: RunRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<RunHandle>> + Send + 'static>> {
    let n8n = n8n.clone();
    Box::pin(async move { start_inner(&n8n, req).await })
}

async fn start_inner(n8n: &Arc<N8n>, req: RunRequest) -> anyhow::Result<RunHandle> {
    if let Some(queue) = &n8n.queue {
        if queueable(&req) {
            return enqueue(n8n, queue, req).await;
        }
    }
    let workflow = Workflow::from_json(&req.workflow)?;
    let workflow_id = workflow.id.clone().unwrap_or_default();
    let (execution_id, started_at) = match &req.resume {
        Some((id, started)) => (*id, started.clone()),
        None => (n8n.store.insert_execution(&req.workflow, req.mode.as_str(), req.retry_of.as_deref(), req.parent_execution.as_deref(), status::RUNNING).await?, None),
    };
    let exec_str = execution_id.to_string();
    let mut options = ExecuteOptions::new(req.mode, exec_str.clone());
    options.use_pin_data = req.use_pin_data;
    options.start_items = req.start_items;
    options.start_node = req.start_node;
    options.destination_node = req.destination_node;
    options.previous_run_data = req.previous_run_data;
    options.start_source = req.start_source;
    options.response = req.response;
    options.resume_signature = Some(n8n.resume_signature(&exec_str));
    options.hooks = Some(Arc::new(PushHooks {
        n8n: Arc::downgrade(n8n),
        push_ref: req.push_ref.clone(),
        execution_id: exec_str.clone(),
        workflow_id: workflow_id.clone(),
    }));
    n8n.running.lock().unwrap().insert(execution_id, options.clone());
    n8n.inflight.fetch_add(1, Ordering::SeqCst);

    let (tx, rx) = tokio::sync::oneshot::channel();
    let me = n8n.clone();
    let workflow_json = req.workflow;
    let mode = req.mode;
    let push_ref = req.push_ref;
    let mut options = options;
    n8n.exec.spawn(async move {
        // Variables are read here, off the caller's path (webhook latency).
        if let Ok(vars) = me.store.list_variables().await {
            for v in vars {
                if let (Some(k), Some(val)) = (v["key"].as_str(), v.get("value")) {
                    options.vars.insert(k.to_string(), val.clone());
                }
            }
        }
        me.send_push(
            push_ref.as_deref(),
            "executionStarted",
            json!({"executionId": exec_str, "mode": mode.as_str(), "startedAt": now_iso(), "workflowId": workflow_id, "workflowName": workflow.name}),
        );
        tracing::info!(executionId = %exec_str, workflowId = %workflow_id, mode = mode.as_str(), "Workflow execution started");
        let (status_, mut irun, wait) = match check_credential_access(&me, &workflow_json).await {
            Err(message) => (status::ERROR.to_string(), setup_error_irun(mode, &message), None),
            Ok(()) => match engine::execute(&workflow, &me.registry, &me.services, options).await {
                Ok(result) => {
                    // Code node console output goes to the editor that ran it.
                    if mode == Mode::Manual {
                        for line in &result.console {
                            me.send_push(push_ref.as_deref(), "sendConsoleMessage", json!({"source": format!("[Workflow \"{}\"]", workflow.name), "messages": [line]}));
                        }
                    }
                    let irun = result.to_irun();
                    (result.status.clone(), irun, result.waiting)
                }
                Err(e) => (status::ERROR.to_string(), setup_error_irun(mode, &e.0), None),
            },
        };
        if let Some(started) = &started_at {
            irun["startedAt"] = json!(started);
        }
        let started = irun["startedAt"].as_str().unwrap_or_default().to_string();
        let stopped = irun["stoppedAt"].as_str().map(String::from);
        let saved = should_save(&workflow_json, mode, &status_);
        let persisted = if saved {
            let (wait_till, wait_state) = match &wait {
                Some(w) => (Some(w.till_ms.map(iso_ms).unwrap_or_else(|| WAIT_FOREVER.to_string())), Some(serde_json::to_value(w).unwrap())),
                None => (None, None),
            };
            if let Some(till) = &wait_till {
                irun["waitTill"] = json!(till);
            }
            let stopped = if wait.is_some() { None } else { stopped.as_deref() };
            me.store.save_execution_result(execution_id, &status_, &started, stopped, &irun["data"], wait_till.as_deref(), wait_state.as_ref()).await
        } else {
            me.store.delete_execution(execution_id).await.map(|_| ())
        };
        if let Err(e) = persisted {
            tracing::error!(executionId = %exec_str, error = %e, "could not store execution result");
        }
        let counter = match status_.as_str() {
            status::SUCCESS => &me.metrics.success,
            status::WAITING => &me.metrics.waiting,
            status::CANCELED => &me.metrics.canceled,
            _ => &me.metrics.error,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        tracing::info!(executionId = %exec_str, workflowId = %workflow_id, status = %status_, "Workflow execution finished");
        me.send_push(push_ref.as_deref(), "executionFinished", json!({"executionId": exec_str, "workflowId": workflow_id, "status": status_}));
        me.running.lock().unwrap().remove(&execution_id);
        let outcome = Arc::new(RunOutcome { execution_id, status: status_.clone(), irun, saved });
        if status_ == status::ERROR && !matches!(mode, Mode::Manual | Mode::Error) {
            run_error_workflow(&me, &workflow_json, &outcome, saved).await;
        }
        let _ = tx.send(outcome);
        if me.inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
            me.inflight_done.notify_waiters();
        }
    });
    Ok(RunHandle { execution_id, done: rx })
}

fn setup_error_irun(mode: Mode, message: &str) -> Value {
    let now = now_iso();
    json!({
        "data": {
            "startData": {},
            "resultData": {"runData": {}, "pinData": {}, "error": {"message": message, "name": "WorkflowOperationError"}},
            "executionData": {"contextData": {}, "nodeExecutionStack": [], "metadata": {}, "waitingExecution": {}, "waitingExecutionSource": {}}
        },
        "mode": mode.as_str(),
        "startedAt": now,
        "stoppedAt": now,
        "status": status::ERROR,
        "finished": false,
    })
}

/// Starts a workflow's error workflow (`settings.errorWorkflow`) with an
/// Error Trigger item describing the failure.
async fn run_error_workflow(n8n: &Arc<N8n>, workflow: &Value, outcome: &RunOutcome, saved: bool) {
    let Some(error_wf_id) = workflow.pointer("/settings/errorWorkflow").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return };
    let Ok(Some(row)) = n8n.store.workflow_row(error_wf_id).await else {
        tracing::warn!(workflowId = error_wf_id, "error workflow does not exist");
        return;
    };
    if !row.active {
        tracing::warn!(workflowId = error_wf_id, "error workflow is not published (active); not running it");
        return;
    }
    let Some(trigger) = row.data["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|n| n["type"] == "n8n-nodes-base.errorTrigger")
        .and_then(|n| n["name"].as_str().map(String::from))
    else {
        tracing::warn!(workflowId = error_wf_id, "error workflow has no Error Trigger");
        return;
    };
    let wf_id = workflow["id"].as_str().unwrap_or_default();
    let mut execution = json!({
        "error": outcome.error().cloned().unwrap_or(json!({})),
        "lastNodeExecuted": outcome.last_node(),
        "mode": outcome.irun["mode"],
        "retryOf": null,
    });
    if saved {
        execution["id"] = json!(outcome.execution_id.to_string());
        execution["url"] = json!(format!("{}workflow/{wf_id}/executions/{}", n8n.config.webhook_url, outcome.execution_id));
    }
    let item = json!({"execution": execution, "workflow": {"id": wf_id, "name": workflow["name"]}});
    let mut req = RunRequest::new(row.data.clone(), Mode::Error);
    req.start_node = Some(trigger);
    req.start_items = Some(vec![Item::from_value(item).paired(0)]);
    if let Err(e) = start(n8n, req).await {
        tracing::error!(workflowId = error_wf_id, error = %e, "could not start the error workflow");
    }
}

/// Continues a waiting execution: with the resume request's item for
/// webhook waits, or the Wait node's input when its time has come.
pub async fn resume(n8n: &Arc<N8n>, id: i64, items: Option<Vec<Item>>) -> Result<RunHandle, ApiError> {
    let row = n8n.store.get_execution(id).await?.ok_or_else(|| ApiError::not_found(format!("The execution \"{id}\" does not exist")))?;
    if row.status != status::WAITING {
        return Err(ApiError::new(409, format!("The execution \"{id}\" is not waiting; it is {}", row.status)));
    }
    let wait: WaitState = row
        .wait_state
        .clone()
        .and_then(|w| serde_json::from_value(w).ok())
        .ok_or_else(|| ApiError::new(409, format!("The execution \"{id}\" has no resume point")))?;
    if !n8n.store.transition_execution(id, status::WAITING, status::RUNNING).await? {
        return Err(ApiError::new(409, format!("The execution \"{id}\" is already resuming")));
    }
    let run_data = row.data.as_ref().and_then(|d| d.pointer("/resultData/runData")).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut req = RunRequest::new(row.workflow_data.clone(), mode_from_str(&row.mode));
    req.start_node = Some(wait.node.clone());
    req.start_items = Some(items.unwrap_or_else(|| wait.input.iter().cloned().enumerate().map(|(i, it)| it.paired(i)).collect()));
    req.start_source = Some(wait.source.clone());
    req.previous_run_data = Some(run_data);
    req.retry_of = row.retry_of.clone();
    req.resume = Some((id, Some(row.started_at.clone())));
    Ok(start(n8n, req).await?)
}

/// Runs sub-workflows for the Execute Workflow node.
pub struct SubRunner(pub Weak<N8n>);

#[async_trait::async_trait]
impl SubWorkflowRunner for SubRunner {
    async fn run_sub_workflow(&self, parent_workflow_id: Option<&str>, workflow_id: &str, items: Vec<Item>, wait: bool) -> NodeResult<Vec<Item>> {
        let n8n = self.0.upgrade().ok_or_else(|| NodeError::new("The server is shutting down"))?;
        let row = n8n
            .store
            .workflow_row(workflow_id)
            .await
            .map_err(|e| NodeError::new(e.to_string()))?
            .ok_or_else(|| NodeError::new(format!("Workflow does not exist: the workflow with ID \"{workflow_id}\" could not be found")))?;
        // The callee decides who may call it (settings.callerPolicy).
        let policy = row.data.pointer("/settings/callerPolicy").and_then(Value::as_str).unwrap_or("workflowsFromSameOwner");
        let parent_row = match parent_workflow_id {
            Some(p) => n8n.store.workflow_row(p).await.ok().flatten(),
            None => None,
        };
        let allowed = match policy {
            "any" => true,
            "none" => parent_workflow_id == Some(workflow_id),
            "workflowsFromAList" => {
                let ids = row.data.pointer("/settings/callerIds").and_then(Value::as_str).unwrap_or("");
                parent_workflow_id.is_some_and(|p| ids.split(',').any(|i| i.trim() == p))
            }
            _ => parent_row.as_ref().map(|p| p.owner_id == row.owner_id && p.project_id == row.project_id).unwrap_or(true),
        };
        if !allowed {
            return Err(NodeError::new(format!(
                "The sub-workflow ({workflow_id}) you're trying to execute limits which workflows it can be called by"
            ))
            .describe("Change the sub-workflow's \"This workflow can be called by\" setting to allow this workflow to call it"));
        }
        let trigger = row.data["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|n| n["type"] == "n8n-nodes-base.executeWorkflowTrigger" && n["disabled"] != json!(true))
            .and_then(|n| n["name"].as_str().map(String::from));
        let mut req = RunRequest::new(row.data.clone(), Mode::Integrated);
        req.start_node = trigger;
        req.start_items = Some(items.into_iter().enumerate().map(|(i, it)| Item { paired_item: None, ..it }.paired(i)).collect());
        req.parent_execution = None;
        let handle = start(&n8n, req).await.map_err(|e| NodeError::new(e.to_string()))?;
        if !wait {
            return Ok(vec![]);
        }
        let outcome = handle.done.await.map_err(|_| NodeError::new("The sub-workflow execution was lost"))?;
        if outcome.status != status::SUCCESS {
            let message = outcome.error().and_then(|e| e["message"].as_str()).unwrap_or("The sub-workflow failed").to_string();
            return Err(NodeError::new(message));
        }
        Ok(outcome.last_output().into_iter().map(|it| Item { paired_item: None, ..it }).collect())
    }
}
