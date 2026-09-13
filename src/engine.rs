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
    let targets: HashSet<&str> = workflow.connections.iter().map(|c| c.to_node.as_str()).collect();
    let start = workflow
        .nodes
        .iter()
        .find(|n| !targets.contains(n.id.as_str()))
        .ok_or_else(|| anyhow::anyhow!("no start node found (cycle or empty graph)"))?;

    let mut order = vec![start.clone()];
    let mut current_id = start.id.clone();
    while let Some(conn) = workflow.connections.iter().find(|c| c.from_node == current_id) {
        let next = workflow
            .nodes
            .iter()
            .find(|n| n.id == conn.to_node)
            .ok_or_else(|| anyhow::anyhow!("dangling connection to {}", conn.to_node))?;
        order.push(next.clone());
        current_id = next.id.clone();
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
}
