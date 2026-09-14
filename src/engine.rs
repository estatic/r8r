use crate::domain::{Item, NodeInstance, Workflow};
use crate::node::{NodeExecutionContext, NodeRegistry};
use std::collections::{HashMap, HashSet, VecDeque};

pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = topological_order(workflow)?;
    let mut produced: HashMap<String, crate::node::NodeOutput> = HashMap::new();

    for node_instance in &order {
        // Aggregate this node's input items from every incoming connection,
        // pulling each upstream node's items from the SPECIFIC from_output
        // port index that connection names (not just port 0).
        let mut input_items: Vec<Item> = Vec::new();
        for conn in &workflow.connections {
            if conn.to_node != node_instance.id {
                continue;
            }
            if let Some(outputs) = produced.get(&conn.from_node) {
                if let Some(port_items) = outputs.get(conn.from_output) {
                    input_items.extend(port_items.iter().cloned());
                }
            }
        }

        if node_instance.disabled {
            // A disabled node is a no-op passthrough: its input items flow
            // through unchanged as its (single-port) output, and it is never
            // handed to the registry, executed, or given resolved parameters.
            produced.insert(node_instance.id.clone(), vec![input_items]);
            continue;
        }

        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;

        // Resolve this node's parameters via the expression engine before
        // execute(), with $node built from every already-executed node's
        // PRIMARY (port 0) output's first item only.
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
        };
        let resolved_parameters = crate::expr::resolve_parameters(&node_instance.parameters, &eval_ctx)
            .map_err(|e| anyhow::anyhow!("node {} parameter resolution failed: {e}", node_instance.id))?;

        let ctx = NodeExecutionContext {
            parameters: resolved_parameters,
            input_items,
        };
        let output = node
            .execute(&ctx)
            .await
            .map_err(|e| anyhow::anyhow!("node {} failed: {e}", node_instance.id))?;
        produced.insert(node_instance.id.clone(), output);
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
    use crate::node::NodeRegistry;
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

    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r
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
        });
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
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
        });
        wf.connections.clear();
        wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "b".into(), to_input: 0 });
        wf.connections.push(Connection { from_node: "b".into(), from_output: 0, to_node: "c".into(), to_input: 0 });
        wf.connections.push(Connection { from_node: "c".into(), from_output: 0, to_node: "b".into(), to_input: 0 });

        let result = topological_order(&wf);
        assert!(result.is_err());
    }

    #[test]
    fn topological_order_rejects_dangling_connection_endpoints() {
        let mut wf = linear_workflow();
        wf.connections.push(Connection { from_node: "ghost".into(), from_output: 0, to_node: "set1".into(), to_input: 0 });
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
        });
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
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
        });
        wf.nodes.push(NodeInstance {
            id: "set3".into(),
            node_type: "core.set".into(),
            position: (3.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
        });
        wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "set2".into(), to_input: 0 });
        wf.connections.push(Connection { from_node: "set1".into(), from_output: 0, to_node: "set3".into(), to_input: 0 });
        wf.connections.push(Connection { from_node: "set2".into(), from_output: 0, to_node: "set3".into(), to_input: 0 });

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
        });
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (1.0, 1.0),
            parameters: serde_json::json!({"fields": {"other": "value"}}),
            disabled: false,
        });
        wf.connections.push(Connection {
            from_node: "trigger2".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
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
        });
        wf.connections.clear();
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "b".into(),
            to_input: 0,
        });
        wf.connections.push(Connection {
            from_node: "b".into(),
            from_output: 0,
            to_node: "c".into(),
            to_input: 0,
        });
        wf.connections.push(Connection {
            from_node: "c".into(),
            from_output: 0,
            to_node: "b".into(),
            to_input: 0,
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
        });
        wf.connections.push(Connection {
            from_node: "set_disabled".into(),
            from_output: 0,
            to_node: "set_final".into(),
            to_input: 0,
        });

        let outputs = execute_workflow(&wf, &registry()).await.unwrap();

        // The disabled node passed its (empty-object) input through unchanged.
        assert_eq!(outputs["set_disabled"][0].json, serde_json::json!({}));
        // The final node's own field is present...
        assert_eq!(outputs["set_final"][0].json, serde_json::json!({"final": "yes"}));
        // ...and critically, the disabled node's field never made it downstream.
        assert!(outputs["set_final"][0].json.get("skipped").is_none());
    }
}
