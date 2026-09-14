use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct FilterNode;

#[async_trait]
impl Node for FilterNode {
    fn type_name(&self) -> &'static str {
        "core.filter"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = ctx.parameters.get("condition").and_then(|v| v.as_bool()).unwrap_or(false);
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
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": true}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items());
    }

    #[tokio::test]
    async fn false_condition_drops_all_items() {
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": false}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
