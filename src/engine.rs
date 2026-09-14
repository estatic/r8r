use crate::domain::{Item, NodeInstance, Workflow};
use crate::node::{NodeExecutionContext, NodeRegistry};
use std::collections::{HashMap, HashSet};

pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = linear_order(workflow)?;
    let mut outputs: HashMap<String, Vec<Item>> = HashMap::new();
    let mut current_items: Vec<Item> = Vec::new();

    for node_instance in order {
        if node_instance.disabled {
            // A disabled node is a no-op passthrough: it still occupies a slot
            // in the chain (already validated by `linear_order`), but its
            // input items flow through to the next node unchanged, and it is
            // never handed to the registry or executed.
            outputs.insert(node_instance.id.clone(), current_items.clone());
            continue;
        }

        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;
        let ctx = NodeExecutionContext {
            parameters: node_instance.parameters.clone(),
            input_items: current_items.clone(),
        };
        let result = node
            .execute(&ctx)
            .await
            .map_err(|e| anyhow::anyhow!("node {} failed: {e}", node_instance.id))?;
        outputs.insert(node_instance.id.clone(), result.clone());
        current_items = result;
    }
    Ok(outputs)
}

fn linear_order(workflow: &Workflow) -> anyhow::Result<Vec<NodeInstance>> {
    if workflow.nodes.is_empty() {
        return Ok(Vec::new());
    }

    let mut outgoing_counts: HashMap<&str, usize> = HashMap::new();
    for conn in &workflow.connections {
        *outgoing_counts.entry(conn.from_node.as_str()).or_insert(0) += 1;
    }
    if let Some((node_id, _)) = outgoing_counts.iter().find(|(_, count)| **count > 1) {
        return Err(anyhow::anyhow!(
            "node {} has more than one outgoing connection (branching is not supported)",
            node_id
        ));
    }

    let targets: HashSet<&str> = workflow.connections.iter().map(|c| c.to_node.as_str()).collect();
    let start_candidates: Vec<&NodeInstance> = workflow
        .nodes
        .iter()
        .filter(|n| !targets.contains(n.id.as_str()))
        .collect();
    if workflow.nodes.len() > 1 && start_candidates.len() > 1 {
        return Err(anyhow::anyhow!(
            "found {} disconnected start candidates (disconnected components are not supported)",
            start_candidates.len()
        ));
    }
    let start = start_candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no start node found (cycle or empty graph)"))?;

    let mut order = vec![start.clone()];
    let mut current_id = start.id.clone();
    let mut seen: HashSet<String> = HashSet::from([start.id.clone()]);
    while let Some(conn) = workflow.connections.iter().find(|c| c.from_node == current_id) {
        let next = workflow
            .nodes
            .iter()
            .find(|n| n.id == conn.to_node)
            .ok_or_else(|| anyhow::anyhow!("dangling connection to {}", conn.to_node))?;
        if !seen.insert(next.id.clone()) {
            return Err(anyhow::anyhow!("cycle detected at node {}", next.id));
        }
        order.push(next.clone());
        current_id = next.id.clone();
    }
    if order.len() != workflow.nodes.len() {
        return Err(anyhow::anyhow!(
            "workflow is not a single linear chain: {} of {} nodes reachable from start",
            order.len(),
            workflow.nodes.len()
        ));
    }
    Ok(order)
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
    async fn node_with_two_outgoing_connections_returns_error() {
        let mut wf = linear_workflow();
        wf.nodes.push(NodeInstance {
            id: "set2".into(),
            node_type: "core.set".into(),
            position: (2.0, 0.0),
            parameters: serde_json::json!({"fields": {"other": "value"}}),
            disabled: false,
        });
        // "trigger" now has two outgoing connections: -> set1 and -> set2.
        wf.connections.push(Connection {
            from_node: "trigger".into(),
            from_output: 0,
            to_node: "set2".into(),
            to_input: 0,
        });
        let result = execute_workflow(&wf, &registry()).await;
        assert!(result.is_err());
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
