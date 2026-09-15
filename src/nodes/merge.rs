use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct MergeNode;

#[async_trait]
impl Node for MergeNode {
    fn type_name(&self) -> &'static str {
        "core.merge"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let mode = ctx.parameters.get("mode").and_then(|v| v.as_str()).unwrap_or("append");

        match mode {
            "mergeByKey" => {
                let key = ctx
                    .parameters
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| NodeError::ExecutionFailed("mergeByKey mode requires a \"key\" parameter".into()))?;

                let mut order: Vec<serde_json::Value> = Vec::new();
                let mut grouped: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>> =
                    std::collections::HashMap::new();
                for item in &ctx.input_items {
                    let key_value = item.json.get(key).cloned().unwrap_or(serde_json::Value::Null);
                    let key_str = key_value.to_string();
                    let obj = item.json.as_object().cloned().unwrap_or_default();
                    let entry = grouped.entry(key_str.clone()).or_insert_with(|| {
                        order.push(key_value.clone());
                        serde_json::Map::new()
                    });
                    for (k, v) in obj {
                        entry.insert(k, v);
                    }
                }
                let merged_items = order
                    .into_iter()
                    .map(|key_value| {
                        let obj = grouped.remove(&key_value.to_string()).unwrap_or_default();
                        Item { json: serde_json::Value::Object(obj), binary: serde_json::json!({}) }
                    })
                    .collect();
                Ok(vec![merged_items])
            }
            _ => Ok(vec![ctx.input_items.clone()]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn append_mode_passes_items_through_unchanged() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"b": 2}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "append"}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn default_mode_is_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn wait_for_all_mode_behaves_like_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "waitForAll"}), input_items: items.clone(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn merge_by_key_combines_items_sharing_a_key_value() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"id": 1, "name": "Ada"}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 1, "age": 30}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 2, "name": "Grace"}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey", "key": "id"}),
            input_items: items,
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].len(), 2);
        assert_eq!(result[0][0].json, serde_json::json!({"id": 1, "name": "Ada", "age": 30}));
        assert_eq!(result[0][1].json, serde_json::json!({"id": 2, "name": "Grace"}));
    }

    #[tokio::test]
    async fn merge_by_key_without_key_parameter_returns_error() {
        let node = MergeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
