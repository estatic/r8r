use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct SwitchNode;

#[async_trait]
impl Node for SwitchNode {
    fn type_name(&self) -> &'static str {
        "core.switch"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let value = ctx
            .parameters
            .get("value")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let cases = ctx
            .parameters
            .get("cases")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let matched_index = cases.iter().position(|c| *c == value);
        let mut ports: Vec<Vec<Item>> = vec![Vec::new(); cases.len() + 1];
        let target = matched_index.unwrap_or(cases.len());
        ports[target] = ctx.input_items.clone();
        Ok(ports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![Item {
            json: serde_json::json!({"x": 1}),
            binary: serde_json::json!({}),
        }]
    }

    #[tokio::test]
    async fn matching_case_routes_to_its_port() {
        let node = SwitchNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"value": "b", "cases": ["a", "b", "c"]}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 4); // 3 cases + 1 default
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
        assert!(result[2].is_empty());
        assert!(result[3].is_empty());
    }

    #[tokio::test]
    async fn no_matching_case_routes_to_default_port() {
        let node = SwitchNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"value": "z", "cases": ["a", "b"]}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 3); // 2 cases + 1 default
        assert!(result[0].is_empty());
        assert!(result[1].is_empty());
        assert_eq!(result[2], items());
    }
}
