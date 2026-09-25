//! n8n-compatible workflow execution (spec §2.4, §6.2).
//!
//! A stack of nodes to run: each run pops a node, executes it on its input,
//! records n8n task data, and schedules the children that received items.
//! v1 order puts children at the front of the stack sorted by canvas
//! position (depth-first, top branch first); v0 appends them (level by
//! level). Nodes with several inputs wait until each input has data, or
//! until nothing else can run.

use super::node::{ExecCtx, Mode, NodeError, NodeType, RunState, Services};
use super::nodes::Registry;
use super::types::{main_data, now_ms, status, Item, NodeOutput, SourceData, TaskData};
use super::workflow::{Node, OnError, Workflow};
use serde_json::{json, Map, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Guards against runaway loops.
const MAX_NODE_RUNS: usize = 100_000;

#[derive(Clone)]
pub struct ExecuteOptions {
    pub mode: Mode,
    pub execution_id: String,
    /// Use the workflow's pin data (editor manual runs only, as in n8n).
    pub use_pin_data: bool,
    /// Items for the start node instead of running it (e.g. a webhook's
    /// request, an error trigger's report).
    pub start_items: Option<Vec<Item>>,
    /// Start at this node instead of the workflow's trigger.
    pub start_node: Option<String>,
    /// Stop after this node has run ("execute up to node X").
    pub destination_node: Option<String>,
    /// Run data from an earlier run to reuse for nodes before `start_node`.
    pub previous_run_data: Option<Map<String, Value>>,
    pub cancel: Arc<AtomicBool>,
}

impl ExecuteOptions {
    pub fn new(mode: Mode, execution_id: impl Into<String>) -> Self {
        Self {
            mode,
            execution_id: execution_id.into(),
            use_pin_data: false,
            start_items: None,
            start_node: None,
            destination_node: None,
            previous_run_data: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// n8n's `IRun`, plus bookkeeping for the caller.
pub struct RunResult {
    pub status: String,
    pub started_at: String,
    pub stopped_at: String,
    pub run_data: Map<String, Value>,
    pub last_node: Option<String>,
    pub error: Option<Value>,
    pub mode: Mode,
    pub custom_data: Map<String, Value>,
    pub static_data: Value,
    pub console: Vec<String>,
}

impl RunResult {
    pub fn to_irun(&self) -> Value {
        let mut result_data = json!({ "runData": self.run_data, "pinData": {} });
        if let Some(last) = &self.last_node {
            result_data["lastNodeExecuted"] = json!(last);
        }
        if let Some(err) = &self.error {
            result_data["error"] = err.clone();
        }
        json!({
            "data": {
                "startData": {},
                "resultData": result_data,
                "executionData": {
                    "contextData": {},
                    "nodeExecutionStack": [],
                    "metadata": {},
                    "waitingExecution": {},
                    "waitingExecutionSource": {}
                }
            },
            "mode": self.mode.as_str(),
            "startedAt": self.started_at,
            "stoppedAt": self.stopped_at,
            "status": self.status,
            "finished": self.status == status::SUCCESS,
        })
    }
}

/// Raised before any node runs: the workflow can't be executed at all.
#[derive(Debug)]
pub struct SetupError(pub String);

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

struct Entry {
    node: String,
    inputs: Vec<Option<Vec<Item>>>,
    source: Vec<Option<SourceData>>,
}

fn iso(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Checks every node type exists and is allowed.
pub fn check_node_types(workflow: &Workflow, registry: &Registry, services: &Services) -> Result<(), SetupError> {
    for node in &workflow.nodes {
        if node.node_type.ends_with(".stickyNote") {
            continue;
        }
        if services.config.nodes_exclude.iter().any(|t| t == &node.node_type) || registry.get(&node.node_type).is_none() {
            return Err(SetupError(format!("Unrecognized node type: {}", node.node_type)));
        }
    }
    Ok(())
}

/// The node an execution starts from: a manual trigger if there is one,
/// otherwise the first trigger, otherwise the first node without parents.
pub fn find_start_node(workflow: &Workflow, registry: &Registry) -> Option<String> {
    let candidates: Vec<&Node> = workflow.nodes.iter().filter(|n| !n.disabled && !n.node_type.ends_with(".stickyNote")).collect();
    if let Some(n) = candidates.iter().find(|n| n.node_type == "n8n-nodes-base.manualTrigger") {
        return Some(n.name.clone());
    }
    if let Some(n) = candidates.iter().find(|n| registry.get(&n.node_type).is_some_and(|t| t.is_trigger())) {
        return Some(n.name.clone());
    }
    candidates.iter().find(|n| workflow.parents(&n.name).is_empty()).map(|n| n.name.clone())
}

pub async fn execute(
    workflow: &Workflow,
    registry: &Registry,
    services: &Services,
    options: ExecuteOptions,
) -> Result<RunResult, SetupError> {
    check_node_types(workflow, registry, services)?;
    let start = match &options.start_node {
        Some(s) => s.clone(),
        None => find_start_node(workflow, registry).ok_or_else(|| SetupError("Missing node to start execution".into()))?,
    };
    if workflow.node(&start).is_none() {
        return Err(SetupError(format!("The node to start from, \"{start}\", does not exist")));
    }

    let started = now_ms();
    let v1 = workflow.execution_order_v1();
    let timeout_at = workflow.settings.get("executionTimeout").and_then(Value::as_f64).filter(|s| *s > 0.0).map(|s| started + (s * 1000.0) as i64);
    let run_state = Arc::new(Mutex::new(RunState {
        static_data: workflow.static_data.clone(),
        ..Default::default()
    }));

    let mut run_data: Map<String, Value> = Map::new();
    let mut stack: VecDeque<Entry> = VecDeque::new();
    let mut waiting: Vec<Entry> = Vec::new();
    let mut execution_index = 0usize;
    let mut last_node: Option<String> = None;
    let mut error: Option<Value> = None;
    let mut final_status = status::SUCCESS;

    // Reuse earlier run data for a partial run: every node that already has
    // data keeps it, and the start node gets its parents' output as input.
    let mut start_entry = Entry { node: start.clone(), inputs: vec![Some(vec![Item::default()])], source: vec![] };
    if let Some(previous) = &options.previous_run_data {
        for (name, runs) in previous {
            if name != &start && workflow.node(name).is_some() {
                run_data.insert(name.clone(), runs.clone());
            }
        }
        let parents = workflow.parents(&start);
        if !parents.is_empty() {
            let inputs_len = workflow.connected_inputs(&start).max(1);
            let mut inputs = vec![None; inputs_len];
            let mut source = vec![None; inputs_len];
            for p in parents {
                let Some(task) = last_task(&run_data, &p.node) else { continue };
                let run = run_data[&p.node].as_array().map(|a| a.len() - 1).unwrap_or(0);
                inputs[p.input] = Some(task.main_output(p.output));
                source[p.input] = Some(SourceData { previous_node: p.node.clone(), previous_node_output: p.output, previous_node_run: run });
            }
            start_entry = Entry { node: start.clone(), inputs, source };
        }
    }
    if let Some(items) = &options.start_items {
        start_entry.inputs = vec![Some(items.clone())];
    }
    stack.push_back(start_entry);
    let mut runs = 0usize;

    loop {
        if options.cancel.load(Ordering::SeqCst) {
            final_status = status::CANCELED;
            error = Some(json!({"message": "The execution was cancelled", "name": "ExecutionCancelledError"}));
            break;
        }
        if timeout_at.is_some_and(|t| now_ms() >= t) {
            final_status = status::CANCELED;
            error = Some(json!({"message": "The execution was cancelled because it timed out", "name": "TimeoutExecutionCancelledError"}));
            break;
        }
        let entry = match stack.pop_front() {
            Some(e) => e,
            None if !waiting.is_empty() => waiting.remove(0),
            None => break,
        };
        runs += 1;
        if runs > MAX_NODE_RUNS {
            final_status = status::ERROR;
            error = Some(json!({"message": format!("The workflow ran more than {MAX_NODE_RUNS} node executions and was stopped"), "name": "WorkflowOperationError"}));
            break;
        }
        let node = workflow.node(&entry.node).expect("scheduled nodes exist");
        let run_index = run_data.get(&node.name).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        let inputs: Vec<Vec<Item>> = entry.inputs.iter().map(|i| i.clone().unwrap_or_default()).collect();
        let is_start = node.name == start && run_index == 0 && entry.source.is_empty();
        let task_start = now_ms();

        let pinned = if options.use_pin_data { workflow.pin_data.get(&node.name) } else { None };
        let outcome: Result<NodeOutput, NodeError> = if let Some(pinned) = pinned {
            let items: Vec<Item> = pinned
                .as_array()
                .map(|a| a.iter().enumerate().map(|(i, v)| pin_item(v).paired(i)).collect())
                .unwrap_or_default();
            Ok(vec![items])
        } else if node.disabled {
            // A disabled node passes its first input through unchanged.
            Ok(vec![inputs.first().cloned().unwrap_or_default()])
        } else if is_start && options.start_items.is_some() {
            Ok(vec![inputs.first().cloned().unwrap_or_default()])
        } else {
            let node_type = registry.get(&node.node_type).expect("checked before the run");
            let remaining = timeout_at.map(|t| (t - now_ms()).max(1) as u64);
            let run = run_node(node, node_type, workflow, services, &options, &run_data, &entry.source, inputs.clone(), run_index, run_state.clone());
            match remaining {
                Some(ms) => match tokio::time::timeout(std::time::Duration::from_millis(ms), run).await {
                    Ok(r) => r,
                    Err(_) => {
                        final_status = status::CANCELED;
                        error = Some(json!({"message": "The execution was cancelled because it timed out", "name": "TimeoutExecutionCancelledError"}));
                        last_node = Some(node.name.clone());
                        break;
                    }
                },
                None => run.await,
            }
        };

        let task_end = now_ms();
        execution_index += 1;
        let source = entry.source.clone();
        let mut task = TaskData {
            start_time: task_start,
            execution_index: execution_index - 1,
            source,
            hints: vec![],
            execution_time: task_end - task_start,
            execution_status: status::SUCCESS.into(),
            data: None,
            error: None,
        };
        last_node = Some(node.name.clone());

        let outputs = match outcome {
            Ok(outputs) => outputs,
            Err(e) if !e.fatal && node.on_error != OnError::StopWorkflow => {
                let mut item = Map::new();
                item.insert("error".into(), json!(e.message));
                let error_item = Item::new(item).paired(0);
                let n_outputs = registry.get(&node.node_type).map(|t| t.outputs(node)).unwrap_or(1);
                if node.on_error == OnError::ContinueErrorOutput {
                    let mut outs = vec![Vec::new(); n_outputs];
                    outs.push(vec![error_item]);
                    outs
                } else {
                    vec![vec![error_item]]
                }
            }
            Err(e) => {
                let err_json = e.to_json(node);
                task.execution_status = status::ERROR.into();
                task.error = Some(err_json.clone());
                push_task(&mut run_data, &node.name, &task);
                error = Some(err_json);
                final_status = status::ERROR;
                break;
            }
        };

        task.data = Some(main_data(&outputs));
        push_task(&mut run_data, &node.name, &task);

        if options.destination_node.as_deref() == Some(node.name.as_str()) {
            break;
        }
        schedule_children(workflow, node, &outputs, run_index, v1, &mut stack, &mut waiting);
    }

    let state = run_state.lock().unwrap();
    Ok(RunResult {
        status: final_status.to_string(),
        started_at: iso(started),
        stopped_at: iso(now_ms()),
        run_data,
        last_node,
        error,
        mode: options.mode,
        custom_data: state.custom_data.clone(),
        static_data: state.static_data.clone(),
        console: state.console.clone(),
    })
}

fn pin_item(v: &Value) -> Item {
    match v.get("json") {
        Some(Value::Object(j)) => Item { json: j.clone(), binary: v.get("binary").and_then(Value::as_object).cloned(), paired_item: None },
        _ => Item::from_value(v.clone()),
    }
}

fn last_task(run_data: &Map<String, Value>, node: &str) -> Option<TaskData> {
    run_data.get(node)?.as_array()?.last().and_then(|t| serde_json::from_value(t.clone()).ok())
}

fn push_task(run_data: &mut Map<String, Value>, node: &str, task: &TaskData) {
    let runs = run_data.entry(node.to_string()).or_insert_with(|| Value::Array(vec![]));
    runs.as_array_mut().unwrap().push(serde_json::to_value(task).unwrap());
}

#[allow(clippy::too_many_arguments)]
async fn run_node(
    node: &Node,
    node_type: &dyn NodeType,
    workflow: &Workflow,
    services: &Services,
    options: &ExecuteOptions,
    run_data: &Map<String, Value>,
    source: &[Option<SourceData>],
    inputs: Vec<Vec<Item>>,
    run_index: usize,
    run_state: Arc<Mutex<RunState>>,
) -> Result<NodeOutput, NodeError> {
    // Nodes receive items without lineage; they set pairedItem themselves
    // (or get it assigned below when the mapping is one-to-one).
    let mut inputs: Vec<Vec<Item>> = inputs
        .into_iter()
        .map(|items| items.into_iter().map(|mut i| { i.paired_item = None; i }).collect())
        .collect();
    if node.execute_once {
        for items in inputs.iter_mut() {
            items.truncate(1);
        }
    }
    let input_len = inputs.first().map(Vec::len).unwrap_or(0);
    let max_tries = if node.retry_on_fail { node.max_tries.clamp(1, 5) } else { 1 };
    let mut attempt = 0;
    loop {
        attempt += 1;
        let expr_inputs = inputs.clone();
        let expr_source = source.to_vec();
        let run_state_for_expr = run_state.clone();
        let expr_data = move || {
            let state = run_state_for_expr.lock().unwrap();
            expression_data(workflow, node, services, options, run_data, &expr_source, &expr_inputs, run_index, &state)
        };
        let mut ctx = ExecCtx::new(
            node,
            workflow,
            inputs.clone(),
            run_index,
            options.mode,
            options.execution_id.clone(),
            services,
            run_state.clone(),
            Box::new(expr_data),
        );
        let result = node_type.execute(&mut ctx).await;
        match result {
            Ok(mut outputs) => {
                let n_outputs = node_type.outputs(node);
                if outputs.len() < n_outputs {
                    outputs.resize(n_outputs, Vec::new());
                }
                for out in outputs.iter_mut() {
                    let len = out.len();
                    for (i, item) in out.iter_mut().enumerate() {
                        if item.paired_item.is_none() {
                            if len == input_len {
                                item.paired_item = Some(json!({"item": i}));
                            } else if input_len == 1 {
                                item.paired_item = Some(json!({"item": 0}));
                            }
                        }
                    }
                }
                let error_items = std::mem::take(&mut ctx.error_items);
                if !error_items.is_empty() {
                    match node.on_error {
                        OnError::ContinueErrorOutput => outputs.push(error_items),
                        _ => {
                            outputs[0].extend(error_items);
                            outputs[0].sort_by_key(|i| i.paired_item.as_ref().and_then(|p| p["item"].as_u64()).unwrap_or(0));
                        }
                    }
                } else if node.on_error == OnError::ContinueErrorOutput {
                    outputs.push(Vec::new());
                }
                if node.always_output_data && outputs.iter().all(Vec::is_empty) {
                    outputs[0] = vec![Item::default().paired(0)];
                }
                return Ok(outputs);
            }
            Err(e) if attempt < max_tries && !e.fatal => {
                tokio::time::sleep(std::time::Duration::from_millis(node.wait_between_tries.min(5000))).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// The data the expression VM sees for one node run.
#[allow(clippy::too_many_arguments)]
fn expression_data(
    workflow: &Workflow,
    node: &Node,
    services: &Services,
    options: &ExecuteOptions,
    run_data: &Map<String, Value>,
    source: &[Option<SourceData>],
    inputs: &[Vec<Item>],
    run_index: usize,
    state: &RunState,
) -> Value {
    // Inputs as the previous node produced them, lineage included.
    let lineage_inputs: Vec<Value> = source
        .iter()
        .enumerate()
        .map(|(i, s)| match s {
            Some(src) => last_or_run_output(run_data, src),
            None => serde_json::to_value(inputs.get(i).cloned().unwrap_or_default()).unwrap(),
        })
        .collect();
    let input0 = lineage_inputs.first().cloned().unwrap_or_else(|| serde_json::to_value(inputs.first().cloned().unwrap_or_default()).unwrap());
    let config = &services.config;
    let env = if config.block_env_access_in_node {
        Value::Null
    } else {
        Value::Object(std::env::vars().map(|(k, v)| (k, Value::String(v))).collect())
    };
    json!({
        "input": input0,
        "inputs": if lineage_inputs.is_empty() { json!([input0]) } else { Value::Array(lineage_inputs) },
        "source": source,
        "runData": run_data,
        "runIndex": run_index,
        "nodeNames": workflow.nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>(),
        "node": {"name": node.name, "type": node.node_type, "parameters": node.parameters},
        "workflow": {"id": workflow.id, "name": workflow.name, "active": workflow.active},
        "execution": {
            "id": options.execution_id,
            "mode": options.mode.as_str(),
            "resumeUrl": format!("{}webhook-waiting/{}", config.webhook_url, options.execution_id),
            "resumeFormUrl": format!("{}form-waiting/{}", config.webhook_url, options.execution_id),
        },
        "customData": state.custom_data,
        "staticData": {"global": state.static_data.get("global").cloned().unwrap_or(json!({})), "node": json!({})},
        "vars": {},
        "env": env,
    })
}

fn last_or_run_output(run_data: &Map<String, Value>, src: &SourceData) -> Value {
    run_data
        .get(&src.previous_node)
        .and_then(|runs| runs.get(src.previous_node_run))
        .and_then(|t| t.pointer(&format!("/data/main/{}", src.previous_node_output)))
        .cloned()
        .unwrap_or_else(|| json!([]))
}

fn schedule_children(
    workflow: &Workflow,
    node: &Node,
    outputs: &NodeOutput,
    run_index: usize,
    v1: bool,
    stack: &mut VecDeque<Entry>,
    waiting: &mut Vec<Entry>,
) {
    let mut ready: Vec<Entry> = Vec::new();
    for (output, items) in outputs.iter().enumerate() {
        if items.is_empty() {
            continue;
        }
        for target in workflow.children(&node.name, output) {
            if workflow.node(&target.node).is_none() {
                continue;
            }
            let src = SourceData { previous_node: node.name.clone(), previous_node_output: output, previous_node_run: run_index };
            let n_inputs = workflow.connected_inputs(&target.node);
            if n_inputs <= 1 {
                ready.push(Entry { node: target.node.clone(), inputs: vec![Some(items.clone())], source: vec![Some(src)] });
                continue;
            }
            // Multi-input node: fill the first pending set missing this input.
            let slot = target.index;
            let pos = waiting.iter().position(|w| w.node == target.node && w.inputs.get(slot).is_some_and(Option::is_none));
            let idx = match pos {
                Some(i) => i,
                None => {
                    waiting.push(Entry { node: target.node.clone(), inputs: vec![None; n_inputs], source: vec![None; n_inputs] });
                    waiting.len() - 1
                }
            };
            waiting[idx].inputs[slot] = Some(items.clone());
            waiting[idx].source[slot] = Some(src);
            if waiting[idx].inputs.iter().all(Option::is_some) {
                ready.push(waiting.remove(idx));
            }
        }
    }
    if v1 {
        ready.sort_by(|a, b| {
            let pa = workflow.node(&a.node).map(|n| n.position).unwrap_or_default();
            let pb = workflow.node(&b.node).map(|n| n.position).unwrap_or_default();
            pa.1.partial_cmp(&pb.1).unwrap_or(std::cmp::Ordering::Equal).then(pa.0.partial_cmp(&pb.0).unwrap_or(std::cmp::Ordering::Equal))
        });
        for entry in ready.into_iter().rev() {
            stack.push_front(entry);
        }
    } else {
        stack.extend(ready);
    }
}
