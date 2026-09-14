use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct ManualTriggerNode;

#[async_trait]
impl Node for ManualTriggerNode {
    fn type_name(&self) -> &'static str {
        "core.manualTrigger"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn produces_exactly_one_empty_item() {
        let node = ManualTriggerNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }
}
