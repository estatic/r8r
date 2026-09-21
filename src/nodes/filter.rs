use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct FilterNode;

#[async_trait]
impl Node for FilterNode {
    fn type_name(&self) -> &'static str {
        "core.filter"
    }
    fn display_name(&self) -> &'static str {
        "Filter"
    }
    fn description(&self) -> &'static str {
        "Keeps only items where a boolean condition is true."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "🚦"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = match ctx.parameters.get("condition") {
            None => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(other) => {
                return Err(NodeError::ExecutionFailed(format!(
                    "core.filter \"condition\" must be a boolean, got: {other}"
                )));
            }
        };
        if condition {
            Ok(vec![ctx.input_items.clone()])
        } else {
            Ok(vec![Vec::new()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Item;

    fn items() -> Vec<Item> {
        vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }]
    }

    #[tokio::test]
    async fn true_condition_keeps_all_items() {
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": true}), input_items: items(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items());
    }

    #[tokio::test]
    async fn false_condition_drops_all_items() {
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": false}), input_items: items(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }

    #[tokio::test]
    async fn missing_condition_defaults_to_false() {
        // Mirrors if_node.rs's test of the same name: an ABSENT "condition"
        // key is sanctioned to default to `false` (drop all items), distinct
        // from a PRESENT non-boolean value, which must error instead (see
        // present_non_boolean_condition_returns_error below).
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }

    #[tokio::test]
    async fn present_non_boolean_condition_returns_error() {
        let node = FilterNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": 5}),
            input_items: items(),
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
