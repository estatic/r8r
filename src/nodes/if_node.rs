use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct IfNode;

#[async_trait]
impl Node for IfNode {
    fn type_name(&self) -> &'static str {
        "core.if"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = ctx
            .parameters
            .get("condition")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if condition {
            Ok(vec![ctx.input_items.clone(), vec![]])
        } else {
            Ok(vec![vec![], ctx.input_items.clone()])
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
    async fn true_condition_routes_to_output_zero() {
        let node = IfNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": true}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items());
        assert!(result[1].is_empty());
    }

    #[tokio::test]
    async fn false_condition_routes_to_output_one() {
        let node = IfNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": false}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }

    #[tokio::test]
    async fn missing_condition_defaults_to_false() {
        let node = IfNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }
}
