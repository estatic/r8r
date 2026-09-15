use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct ScheduleNode;

#[async_trait]
impl Node for ScheduleNode {
    fn type_name(&self) -> &'static str {
        "core.schedule"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_execute_returns_a_single_empty_item() {
        let node = ScheduleNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"cron": "0 0 * * * *"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }
}
