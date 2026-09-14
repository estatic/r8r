use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct WaitNode;

#[async_trait]
impl Node for WaitNode {
    fn type_name(&self) -> &'static str {
        "core.wait"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let seconds = ctx.parameters.get("seconds").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
        tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
        Ok(vec![ctx.input_items.clone()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Item;

    #[tokio::test]
    async fn passes_items_through_after_waiting() {
        let node = WaitNode;
        let items = vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"seconds": 0.01}), input_items: items.clone() };
        let start = std::time::Instant::now();
        let result = node.execute(&ctx).await.unwrap();
        assert!(start.elapsed() >= std::time::Duration::from_millis(10));
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn missing_seconds_defaults_to_zero_wait() {
        let node = WaitNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
