use crate::domain::{Item, NodeInstance, Workflow};
use crate::node::{NodeExecutionContext, NodeRegistry};
use std::collections::{HashMap, HashSet, VecDeque};

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

pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &std::sync::Arc<NodeRegistry>,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None, &HashMap::new(), &NoopObserver).await
}

/// The real `ToolExecutor` used by every live execution: dispatches a
/// tool call to a registered node type via the same `NodeRegistry` the
/// main engine loop uses. A called tool's own context gets `tool_executor:
/// None` (see `call_tool` below) -- a tool can never itself call further
/// tools, which is what makes the `node_type != "ai.agent"` validation in
/// the Agent node's own parameter parsing (Task 5) sufficient to prevent
/// all agent-to-agent recursion without needing a depth counter here.
struct EngineToolExecutor {
    registry: std::sync::Arc<NodeRegistry>,
    credentials: HashMap<uuid::Uuid, serde_json::Value>,
}

impl EngineToolExecutor {
    fn new(registry: std::sync::Arc<NodeRegistry>, credentials: HashMap<uuid::Uuid, serde_json::Value>) -> Self {
        Self { registry, credentials }
    }
}

#[async_trait::async_trait]
impl crate::node::ToolExecutor for EngineToolExecutor {
    async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
        let node = self.registry.get(node_type).ok_or_else(|| {
            crate::node::NodeError::ExecutionFailed(format!("unknown tool node_type: {node_type}"))
        })?;
        let ctx = NodeExecutionContext {
            parameters,
            input_items: vec![],
            credentials: self.credentials.clone(),
            tool_executor: None,
        };
        node.execute(&ctx).await
    }
}

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

/// Runs one node under its `NodeSettings`: each attempt optionally bounded
/// by `timeout_ms`, retried up to `retry.max_tries` with `retry.wait_ms`
/// between attempts (never after the last). Default settings = exactly one
/// untimed `execute()` call, i.e. the pre-Plan-8.5 behavior.
async fn run_node_with_policy(
    node: &dyn crate::node::Node,
    ctx: &NodeExecutionContext,
    settings: &crate::domain::NodeSettings,
) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
    let max_tries = settings.retry.as_ref().map_or(1, |r| r.max_tries);
    let wait = std::time::Duration::from_millis(settings.retry.as_ref().map_or(0, |r| r.wait_ms));
    let mut attempt = 1;
    loop {
        let result = match settings.timeout_ms {
            Some(ms) => tokio::time::timeout(std::time::Duration::from_millis(ms), node.execute(ctx))
                .await
                .unwrap_or_else(|_| Err(crate::node::NodeError::ExecutionFailed(format!("timed out after {ms}ms")))),
            None => node.execute(ctx).await,
        };
        match result {
            Ok(output) => return Ok(output),
            Err(e) if attempt >= max_tries => {
                if max_tries == 1 {
                    return Err(e);
                }
                // Unwrap the message so the Display prefix ("node execution
                // failed: ") isn't repeated. NodeError has one variant, so
                // this let is irrefutable; add match arms if that changes.
                let crate::node::NodeError::ExecutionFailed(last) = e;
                return Err(crate::node::NodeError::ExecutionFailed(format!("failed after {max_tries} attempts: {last}")));
            }
            Err(_) => {
                tokio::time::sleep(wait).await;
                attempt += 1;
            }
        }
    }
}

pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &std::sync::Arc<NodeRegistry>,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<uuid::Uuid, serde_json::Value>,
    observer: &dyn ExecutionObserver,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = topological_order(workflow)?;
    let start_id = order.first().map(|n| n.id.clone());
    let mut produced: HashMap<String, crate::node::NodeOutput> = HashMap::new();
    let mut error_produced: HashMap<String, Vec<Item>> = HashMap::new();
    let tool_executor: std::sync::Arc<dyn crate::node::ToolExecutor> =
        std::sync::Arc::new(EngineToolExecutor::new(registry.clone(), credentials.clone()));

    for node_instance in &order {
        // Aggregate this node's input items from every incoming connection,
        // pulling each upstream node's items from the SPECIFIC from_output
        // port index that connection names (not just port 0). A connection
        // marked `error` is routed via the separate error_produced side-map
        // instead, since error output isn't a real port index into a
        // NodeOutput.
        let mut input_items: Vec<Item> = Vec::new();
        for conn in &workflow.connections {
            if conn.to_node != node_instance.id {
                continue;
            }
            if conn.error {
                if let Some(items) = error_produced.get(&conn.from_node) {
                    input_items.extend(items.iter().cloned());
                }
            } else if let Some(outputs) = produced.get(&conn.from_node) {
                if let Some(port_items) = outputs.get(conn.from_output) {
                    input_items.extend(port_items.iter().cloned());
                }
            }
        }

        if node_instance.disabled {
            // A disabled node is a no-op passthrough: its input items flow
            // through unchanged as its (single-port) output, and it is never
            // handed to the registry, executed, or given resolved parameters.
            observer.on_node_skipped(&node_instance.id, &input_items).await;
            produced.insert(node_instance.id.clone(), vec![input_items]);
            continue;
        }

        // Seeded start node — inject trigger_items and skip execute() entirely
        // for this node only. Must come after the disabled check (a disabled start
        // node keeps its existing passthrough semantics, not the seed) and before
        // the registry lookup / empty-input skip (the seed always "counts" as
        // having produced real output, regardless of what input_items ended up
        // being — a start node has no incoming connections, so input_items is
        // always empty anyway; the seed replaces it, not merges with it).
        if let (Some(items), Some(start)) = (&trigger_items, &start_id) {
            if &node_instance.id == start {
                observer.on_node_started(&node_instance.id).await;
                observer.on_node_finished(&node_instance.id, items).await;
                produced.insert(node_instance.id.clone(), vec![items.clone()]);
                continue;
            }
        }

        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;

        // A node with at least one incoming connection that nonetheless
        // received zero items (e.g. it sits on the untaken branch of an
        // upstream If/Switch) must NOT execute: several nodes (e.g. core.set)
        // synthesize a default item when given empty input, which is correct
        // only for a genuine start node (one with no incoming connections at
        // all). Without this check, branching semantics break: the untaken
        // branch would still "run" and fabricate output. The topological
        // order guarantees at most one node (the start node) has zero
        // incoming connections, so this can't accidentally skip a real start
        // node.
        let has_incoming_connection = workflow.connections.iter().any(|c| c.to_node == node_instance.id);
        if has_incoming_connection && input_items.is_empty() {
            observer.on_node_skipped(&node_instance.id, &[]).await;
            produced.insert(node_instance.id.clone(), vec![]);
            continue;
        }

        // Resolve this node's parameters via the expression engine before
        // execute(), with $node built from every already-executed node's
        // PRIMARY (port 0) output's first item only — unless the node opts
        // out via `resolves_parameters() == false` (e.g. core.code, whose
        // "parameter" is a script to run verbatim, not a value to
        // interpolate).
        let parameters = if node.resolves_parameters() {
            let items_json: Vec<serde_json::Value> = input_items.iter().map(|i| i.json.clone()).collect();
            let node_json: HashMap<String, serde_json::Value> = produced
                .iter()
                .filter_map(|(id, ports)| {
                    let first_item_json = ports.first().and_then(|p| p.first()).map(|item| item.json.clone());
                    first_item_json.map(|j| (id.clone(), j))
                })
                .collect();
            let eval_ctx = crate::expr::EvalContext {
                json: input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})),
                items: &items_json,
                node_json: &node_json,
                workflow_name: &workflow.name,
                args: None,
            };
            crate::expr::resolve_parameters(&node_instance.parameters, &eval_ctx)
                .map_err(|e| anyhow::anyhow!("node {} parameter resolution failed: {e}", node_instance.id))?
        } else {
            node_instance.parameters.clone()
        };

        let ctx = NodeExecutionContext {
            parameters,
            input_items,
            credentials: credentials.clone(),
            tool_executor: Some(tool_executor.clone()),
        };
        observer.on_node_started(&node_instance.id).await;
        match run_node_with_policy(node, &ctx, &node_instance.settings).await {
            Ok(output) => {
                let primary = output.first().cloned().unwrap_or_default();
                observer.on_node_finished(&node_instance.id, &primary).await;
                produced.insert(node_instance.id.clone(), output);
            }
            Err(e) => {
                // Precedence (spec §4): an error connection wins, then
                // continue_on_fail (error item on port 0 only), else the
                // whole run fails.
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
    }

    // Persist/return only each node's primary (port 0) output, flattened
    // into the pre-existing HashMap<String, Vec<Item>> shape.
    let flattened = order
        .into_iter()
        .map(|n| {
            let items = produced
                .get(&n.id)
                .and_then(|ports| ports.first())
                .cloned()
                .unwrap_or_default();
            (n.id, items)
        })
        .collect();
    Ok(flattened)
}

fn topological_order(workflow: &Workflow) -> anyhow::Result<Vec<NodeInstance>> {
    if workflow.nodes.is_empty() {
        return Ok(Vec::new());
    }

    for conn in &workflow.connections {
        if !workflow.nodes.iter().any(|n| n.id == conn.from_node) {
            return Err(anyhow::anyhow!("connection references unknown from_node {}", conn.from_node));
        }
        if !workflow.nodes.iter().any(|n| n.id == conn.to_node) {
            return Err(anyhow::anyhow!("connection references unknown to_node {}", conn.to_node));
        }
    }

    let targets: HashSet<&str> = workflow.connections.iter().map(|c| c.to_node.as_str()).collect();
    let start_candidates: Vec<&str> = workflow
        .nodes
        .iter()
        .map(|n| n.id.as_str())
        .filter(|id| !targets.contains(id))
        .collect();
    if workflow.nodes.len() > 1 && start_candidates.len() != 1 {
        return Err(anyhow::anyhow!(
            "expected exactly one start node, found {}",
            start_candidates.len()
        ));
    }

    let position_of = |id: &str| workflow.nodes.iter().position(|n| n.id == id).unwrap();

    let mut in_degree: HashMap<&str, usize> = workflow.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut seen_edges: HashSet<(&str, &str)> = HashSet::new();
    for conn in &workflow.connections {
        if seen_edges.insert((conn.from_node.as_str(), conn.to_node.as_str())) {
            adjacency.entry(conn.from_node.as_str()).or_default().push(conn.to_node.as_str());
            *in_degree.entry(conn.to_node.as_str()).or_insert(0) += 1;
        }
    }

    let mut ready: Vec<&str> = in_degree.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
    ready.sort_by_key(|id| position_of(id));
    let mut queue: VecDeque<&str> = ready.into();

    let mut order_ids: Vec<String> = Vec::new();
    while let Some(id) = queue.pop_front() {
        order_ids.push(id.to_string());
        if let Some(next_ids) = adjacency.get(id) {
            let mut newly_ready: Vec<&str> = Vec::new();
            for &next in next_ids {
                let degree = in_degree.get_mut(next).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    newly_ready.push(next);
                }
            }
            newly_ready.sort_by_key(|id| position_of(id));
            for id in newly_ready {
                queue.push_back(id);
            }
        }
    }

    if order_ids.len() != workflow.nodes.len() {
        return Err(anyhow::anyhow!(
            "cycle detected: only {} of {} nodes are reachable via a valid topological order",
            order_ids.len(),
            workflow.nodes.len()
        ));
    }

    Ok(order_ids
        .into_iter()
        .map(|id| workflow.nodes.iter().find(|n| n.id == id).unwrap().clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Connection, NodeInstance, Workflow};
    use crate::node::{NodeRegistry, ToolExecutor};
    use uuid::Uuid;

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

    fn registry() -> std::sync::Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        std::sync::Arc::new(r)
    }

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
        let outputs = execute_workflow_seeded(&wf, &registry(), Some(seeded_items.clone()), &HashMap::new(), &NoopObserver).await.unwrap();
        // trigger's own execute() was never called — its output IS the seeded item, verbatim.
        assert_eq!(outputs["trigger"], seeded_items);
        // set1 (which merges a static "greeting" field into its input item, per
        // core.set's actual merge semantics in src/nodes/set.rs) still ran normally
        // downstream, on the seeded item — this just proves seeding didn't break
        // normal downstream execution.
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"from": "webhook", "greeting": "hi"}));
    }

    #[tokio::test]
    async fn execute_workflow_with_none_seed_behaves_exactly_as_before() {
        let wf = linear_workflow();
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(outputs["trigger"][0].json, serde_json::json!({}));
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[test]
    fn topological_order_handles_simple_linear_chain() {
        let wf = linear_workflow();
        let order = topological_order(&wf).unwrap();
        assert_eq!(order.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["trigger", "set1"]);
    }

    #[test]
    fn topological_order_allows_branching() {
        let mut wf = linear_workflow();
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
            error: false,
        });
        let order = topological_order(&wf).unwrap();
        assert_eq!(order[0].id, "trigger");
        let rest: std::collections::HashSet<&str> = order[1..].iter().map(|n| n.id.as_str()).collect();
        assert_eq!(rest, std::collections::HashSet::from(["set1", "set2"]));
    }

    #[test]
    fn topological_order_detects_cycle_downstream_of_valid_start() {
        let mut wf = linear_workflow();
        wf.nodes[1].id = "b".into();
        wf.nodes.push(NodeInstance {
            id: "c".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.clear();
        wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "b".into(), to_input: 0, error: false });
        wf.connections.push(Connection { from_node: "b".into(), from_output: 0, to_node: "c".into(), to_input: 0, error: false });
        wf.connections.push(Connection { from_node: "c".into(), from_output: 0, to_node: "b".into(), to_input: 0, error: false });

        let result = topological_order(&wf);
        assert!(result.is_err());
    }

    #[test]
    fn topological_order_rejects_dangling_connection_endpoints() {
        let mut wf = linear_workflow();
        wf.connections.push(Connection { from_node: "ghost".into(), from_output: 0, to_node: "set1".into(), to_input: 0, error: false });
        let result = topological_order(&wf);
        assert!(result.is_err());
    }

    #[test]
    fn topological_order_rejects_multiple_start_candidates() {
        let mut wf = linear_workflow();
        wf.nodes.push(NodeInstance {
            id: "trigger2".into(),
            node_type: "core.manualTrigger".into(),
            position: (0.0, 1.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        let result = topological_order(&wf);
        assert!(result.is_err());
    }

    #[test]
    fn topological_order_handles_empty_workflow() {
        let mut wf = linear_workflow();
        wf.nodes.clear();
        wf.connections.clear();
        let order = topological_order(&wf).unwrap();
        assert!(order.is_empty());
    }

    #[tokio::test]
    async fn executes_trigger_then_set_in_order() {
        let wf = linear_workflow();
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();

        assert_eq!(outputs["trigger"][0].json, serde_json::json!({}));
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[tokio::test]
    async fn empty_workflow_produces_empty_outputs() {
        let mut wf = linear_workflow();
        wf.nodes.clear();
        wf.connections.clear();
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert!(outputs.is_empty());
    }

    #[tokio::test]
    async fn unknown_node_type_returns_error() {
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "core.doesNotExist".into();
        let result = execute_workflow(&wf, &registry()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn branching_workflow_executes_both_downstream_nodes() {
        let mut wf = linear_workflow();
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"other": "value"}}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
            error: false,
        });
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
        assert_eq!(outputs["set2"][0].json, serde_json::json!({"other": "value"}));
    }

    #[tokio::test]
    async fn node_with_two_incoming_connections_receives_both_upstream_outputs() {
        // trigger -> set1 (adds greeting), trigger -> set2 (adds other),
        // set1 -> set3, set2 -> set3: set3 should see items carrying BOTH fields
        // aggregated from its two incoming connections.
        let mut wf = linear_workflow();
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"other": "value"}}),
            disabled: false,
            settings: Default::default(),
        });
        wf.nodes.push(NodeInstance {
            id: "set3".into(),
            node_type: "core.set".into(),
            position: (3.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "set2".into(), to_input: 0, error: false });
        wf.connections.push(Connection { from_node: "set1".into(), from_output: 0, to_node: "set3".into(), to_input: 0, error: false });
        wf.connections.push(Connection { from_node: "set2".into(), from_output: 0, to_node: "set3".into(), to_input: 0, error: false });

        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        // set3 received one item from each upstream branch.
        assert_eq!(outputs["set3"].len(), 2);
    }

    #[tokio::test]
    async fn expression_in_parameters_is_resolved_before_node_execution() {
        let mut wf = linear_workflow();
        wf.nodes[1].parameters = serde_json::json!({"fields": {"doubled": "{{ 21 * 2 }}"}});
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"doubled": 42}));
    }

    #[tokio::test]
    async fn disconnected_components_return_error() {
        let mut wf = linear_workflow();
        // Add a second, disjoint linear chain: "trigger2" -> "set2".
        wf.nodes.push(NodeInstance {
            id: "trigger2".into(),
            node_type: "core.manualTrigger".into(),
            position: (0.0, 1.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (1.0, 1.0),
            parameters: serde_json::json!({"fields": {"other": "value"}}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "trigger2".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
            error: false,
        });
        let result = execute_workflow(&wf, &registry()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn cycle_downstream_of_valid_start_returns_error_and_terminates() {
        // a -> b -> c -> b: "a" is the unique, valid start node (never a
        // to_node), and every node has exactly one outgoing connection, so
        // neither the branching check nor the disconnected-start check fires.
        // Without cycle tracking the b -> c -> b walk loops forever.
        let mut wf = linear_workflow();
        wf.nodes[1].id = "b".into(); // was "set1"
        wf.nodes.push(NodeInstance {
            id: "c".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.clear();
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "b".into(),
            to_input: 0,
            error: false,
        });
        wf.connections.push(Connection {
            from_node: "b".into(),
            from_output: 0,
            to_node: "c".into(),
            to_input: 0,
            error: false,
        });
        wf.connections.push(Connection {
            from_node: "c".into(),
            from_output: 0,
            to_node: "b".into(),
            to_input: 0,
            error: false,
        });

        // Race the call against a short timeout: a regression that reintroduces
        // the infinite loop must fail this test instead of hanging the suite.
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            execute_workflow(&wf, &registry()),
        )
        .await
        .expect("execute_workflow must terminate promptly, not hang on a cycle");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn dangling_from_node_returns_error() {
        // Connection "ghost" -> "b": "ghost" is not a node in the workflow, so
        // it's never walked as current_id, and "b" (the only real to_node)
        // makes "a" look like the unique valid start. Without the
        // order.len() != nodes.len() reachability check, this would silently
        // return Ok([a]) and "b" would never execute.
        let mut wf = linear_workflow();
        wf.nodes[1].id = "b".into();
        wf.connections.clear();
        wf.connections.push(Connection {
            from_node: "ghost".into(),
            from_output: 0,
            to_node: "b".into(),
            to_input: 0,
            error: false,
        });

        let result = execute_workflow(&wf, &registry()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn disabled_node_is_skipped_as_passthrough() {
        // trigger -> disabled "set_disabled" (would add {"skipped": "yes"} if
        // it ran) -> "set_final" (adds {"final": "yes"}). The disabled node's
        // field must NOT appear in the final output, while both the trigger's
        // (empty) output and the final node's field must.
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
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "set_disabled".into(),
            from_output: 0,
            to_node: "set_final".into(),
            to_input: 0,
            error: false,
        });

        let outputs = execute_workflow(&wf, &registry()).await.unwrap();

        // The disabled node passed its (empty-object) input through unchanged.
        assert_eq!(outputs["set_disabled"][0].json, serde_json::json!({}));
        // The final node's own field is present...
        assert_eq!(outputs["set_final"][0].json, serde_json::json!({"final": "yes"}));
        // ...and critically, the disabled node's field never made it downstream.
        assert!(outputs["set_final"][0].json.get("skipped").is_none());
    }

    struct AlwaysFailsNode;

    #[async_trait::async_trait]
    impl crate::node::Node for AlwaysFailsNode {
        fn type_name(&self) -> &'static str {
            "test.alwaysFails"
        }
        fn display_name(&self) -> &'static str {
            "Always Fails"
        }
        fn description(&self) -> &'static str {
            "Test-only node that always returns an execution error."
        }
        fn category(&self) -> crate::node::NodeCategory {
            crate::node::NodeCategory::Action
        }
        async fn execute(&self, _ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            Err(crate::node::NodeError::ExecutionFailed("boom".into()))
        }
    }

    fn registry_with_failing_node() -> std::sync::Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r.register(Box::new(AlwaysFailsNode));
        std::sync::Arc::new(r)
    }

    use crate::domain::{NodeSettings, RetryPolicy};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Fails its first `failures` calls, then returns one `{"ok": true}` item.
    struct FlakyNode {
        failures: u32,
        calls: Arc<AtomicU32>,
    }

    #[async_trait::async_trait]
    impl crate::node::Node for FlakyNode {
        fn type_name(&self) -> &'static str {
            "test.flaky"
        }
        fn display_name(&self) -> &'static str {
            "Flaky"
        }
        fn description(&self) -> &'static str {
            "Test-only node that fails a fixed number of times, then succeeds."
        }
        fn category(&self) -> crate::node::NodeCategory {
            crate::node::NodeCategory::Action
        }
        async fn execute(&self, _ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n <= self.failures {
                Err(crate::node::NodeError::ExecutionFailed(format!("flaky failure {n}")))
            } else {
                Ok(vec![vec![Item { json: serde_json::json!({"ok": true}), binary: serde_json::json!({}) }]])
            }
        }
    }

    /// Sleeps 10s before succeeding; counts calls.
    struct SlowNode {
        calls: Arc<AtomicU32>,
    }

    #[async_trait::async_trait]
    impl crate::node::Node for SlowNode {
        fn type_name(&self) -> &'static str {
            "test.slow"
        }
        fn display_name(&self) -> &'static str {
            "Slow"
        }
        fn description(&self) -> &'static str {
            "Test-only node that takes 10 seconds."
        }
        fn category(&self) -> crate::node::NodeCategory {
            crate::node::NodeCategory::Action
        }
        async fn execute(&self, _ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(vec![vec![]])
        }
    }

    fn registry_with(extra: Vec<Box<dyn crate::node::Node>>) -> Arc<NodeRegistry> {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r.register(Box::new(AlwaysFailsNode));
        for node in extra {
            r.register(node);
        }
        Arc::new(r)
    }

    /// linear_workflow() with set1 turned into `node_type` carrying
    /// `settings`, plus a `core.noop` "after" node on set1's port 0.
    fn workflow_with_policy(node_type: &str, settings: NodeSettings) -> Workflow {
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = node_type.into();
        wf.nodes[1].settings = settings;
        wf.nodes.push(NodeInstance {
            id: "after".into(),
            node_type: "core.noop".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: NodeSettings::default(),
        });
        wf.connections.push(Connection { from_node: "set1".into(), from_output: 0, to_node: "after".into(), to_input: 0, error: false });
        wf
    }

    fn retry(max_tries: u32, wait_ms: u64) -> NodeSettings {
        NodeSettings { retry: Some(RetryPolicy { max_tries, wait_ms }), ..Default::default() }
    }

    #[tokio::test]
    async fn retry_succeeds_when_max_tries_exceeds_failures() {
        let calls = Arc::new(AtomicU32::new(0));
        let registry = registry_with(vec![Box::new(FlakyNode { failures: 2, calls: calls.clone() })]);
        let wf = workflow_with_policy("test.flaky", retry(3, 0));
        let outputs = execute_workflow(&wf, &registry).await.unwrap();
        assert_eq!(outputs["after"][0].json, serde_json::json!({"ok": true}));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn exhausted_retries_fail_with_attempt_count() {
        let calls = Arc::new(AtomicU32::new(0));
        let registry = registry_with(vec![Box::new(FlakyNode { failures: 5, calls: calls.clone() })]);
        let wf = workflow_with_policy("test.flaky", retry(3, 0));
        let err = execute_workflow(&wf, &registry).await.unwrap_err().to_string();
        assert_eq!(err, "node set1 failed: node execution failed: failed after 3 attempts: flaky failure 3");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_fails_a_slow_node() {
        let calls = Arc::new(AtomicU32::new(0));
        let registry = registry_with(vec![Box::new(SlowNode { calls: calls.clone() })]);
        let settings = NodeSettings { timeout_ms: Some(100), ..Default::default() };
        let wf = workflow_with_policy("test.slow", settings);
        let err = execute_workflow(&wf, &registry).await.unwrap_err().to_string();
        assert!(err.contains("timed out after 100ms"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "timeout without retry makes exactly one attempt");
    }

    #[tokio::test(start_paused = true)]
    async fn timeout_is_retried() {
        let calls = Arc::new(AtomicU32::new(0));
        let registry = registry_with(vec![Box::new(SlowNode { calls: calls.clone() })]);
        let settings = NodeSettings { retry: Some(RetryPolicy { max_tries: 3, wait_ms: 0 }), timeout_ms: Some(100), continue_on_fail: false };
        let wf = workflow_with_policy("test.slow", settings);
        let err = execute_workflow(&wf, &registry).await.unwrap_err().to_string();
        assert!(err.contains("failed after 3 attempts") && err.contains("timed out after 100ms"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn wait_elapses_between_attempts_but_not_after_the_last() {
        let calls = Arc::new(AtomicU32::new(0));
        let registry = registry_with(vec![Box::new(FlakyNode { failures: 5, calls: calls.clone() })]);
        let wf = workflow_with_policy("test.flaky", retry(3, 500));
        let start = tokio::time::Instant::now();
        let _ = execute_workflow(&wf, &registry).await;
        let elapsed = start.elapsed();
        assert!(elapsed >= std::time::Duration::from_millis(1000), "{elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(1500), "{elapsed:?}");
    }

    fn continue_on_fail() -> NodeSettings {
        NodeSettings { continue_on_fail: true, ..Default::default() }
    }

    #[tokio::test]
    async fn continue_on_fail_delivers_error_item_on_port_zero() {
        let wf = workflow_with_policy("test.alwaysFails", continue_on_fail());
        let outputs = execute_workflow(&wf, &registry_with(vec![])).await.unwrap();
        assert_eq!(outputs["after"][0].json, serde_json::json!({"error": "node execution failed: boom"}));
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"error": "node execution failed: boom"}));
    }

    #[tokio::test]
    async fn error_connection_takes_precedence_over_continue_on_fail() {
        let mut wf = workflow_with_policy("test.alwaysFails", continue_on_fail());
        wf.nodes.push(NodeInstance {
            id: "error_handler".into(),
            node_type: "core.set".into(),
            position: (2.0, 1.0),
            parameters: serde_json::json!({"fields": {"handled": true}}),
            disabled: false,
            settings: NodeSettings::default(),
        });
        wf.connections.push(Connection { from_node: "set1".into(), from_output: 0, to_node: "error_handler".into(), to_input: 0, error: true });
        let outputs = execute_workflow(&wf, &registry_with(vec![])).await.unwrap();
        assert_eq!(outputs["error_handler"][0].json["handled"], serde_json::json!(true));
        assert!(outputs["after"].is_empty(), "port-0 downstream must be skipped when the error route is taken");
    }

    #[tokio::test]
    async fn continue_on_fail_only_feeds_port_zero() {
        // "after" is rewired to set1's port 1: it must receive nothing.
        let mut wf = workflow_with_policy("test.alwaysFails", continue_on_fail());
        wf.connections.last_mut().unwrap().from_output = 1;
        let outputs = execute_workflow(&wf, &registry_with(vec![])).await.unwrap();
        assert!(outputs["after"].is_empty());
    }

    #[tokio::test]
    async fn error_without_connected_error_route_aborts_the_run() {
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "test.alwaysFails".into();
        let result = execute_workflow(&wf, &registry_with_failing_node()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn code_node_script_bypasses_parameter_resolution() {
        // A core.code script containing literal `{{ }}` text (e.g. building a
        // template string for a downstream node) must reach CodeNode::execute
        // verbatim, NOT be run through expr::resolve_parameters first. If it
        // were resolved, `{{ not an expression }}` would be evaluated as an
        // r8r expression and either throw (ReferenceError: not is not
        // defined) or otherwise corrupt the literal text before the script's
        // own logic ever runs.
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "core.code".into();
        wf.nodes[1].parameters = serde_json::json!({
            "script": "return [{json: {text: \"literal {{ not an expression }}\"}}];"
        });
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(
            outputs["set1"][0].json,
            serde_json::json!({"text": "literal {{ not an expression }}"})
        );
    }

    #[tokio::test]
    async fn error_with_connected_error_route_continues_and_routes_error_item() {
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "test.alwaysFails".into();
        wf.nodes.push(NodeInstance {
            id: "error_handler".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"handled": true}}),
            disabled: false,
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "set1".into(),
            from_output: 0,
            to_node: "error_handler".into(),
            to_input: 0,
            error: true,
        });

        let outputs = execute_workflow(&wf, &registry_with_failing_node()).await.unwrap();
        assert_eq!(outputs["error_handler"][0].json["handled"], serde_json::json!(true));
        // set1 itself produced no primary-output items (it errored).
        assert!(outputs["set1"].is_empty());
    }

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
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "set_disabled".into(),
            from_output: 0,
            to_node: "set_final".into(),
            to_input: 0,
            error: false,
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
            settings: Default::default(),
        });
        wf.connections.push(Connection {
            from_node: "set1".into(),
            from_output: 0,
            to_node: "error_handler".into(),
            to_input: 0,
            error: true,
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

    #[tokio::test]
    async fn engine_tool_executor_errors_on_an_unknown_node_type() {
        let tool_executor = EngineToolExecutor::new(registry(), HashMap::new());
        let result = tool_executor.call_tool("does.not.exist", serde_json::json!({})).await;
        assert!(matches!(result, Err(crate::node::NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn every_node_receives_a_tool_executor_and_engine_tool_executor_dispatches_to_the_registry() {
        // Part A: prove every node's context carries a tool_executor (not
        // just some special-cased node type) by running a normal
        // single-node workflow and confirming it doesn't error just
        // because a tool_executor now exists in scope -- this is an
        // indirect check since core.set itself never calls it.
        let wf = linear_workflow(); // trigger -> set1
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));

        // Part B: prove EngineToolExecutor itself correctly dispatches to
        // a real registered node type via the registry.
        let tool_executor = EngineToolExecutor::new(registry(), HashMap::new());
        let output = tool_executor
            .call_tool("core.set", serde_json::json!({"fields": {"x": 1}}))
            .await
            .unwrap();
        assert_eq!(output[0][0].json, serde_json::json!({"x": 1}));
    }
}
