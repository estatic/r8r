use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct NoOpNode;

#[async_trait]
impl Node for NoOpNode {
    fn type_name(&self) -> &'static str {
        "core.noop"
    }
    fn display_name(&self) -> &'static str {
        "No-Op"
    }
    fn description(&self) -> &'static str {
        "Passes items through unchanged."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn icon(&self) -> &'static str {
        "⚪"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![ctx.input_items.clone()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Item;

    #[tokio::test]
    async fn passes_items_through_unchanged() {
        let node = NoOpNode;
        let items = vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }
}
