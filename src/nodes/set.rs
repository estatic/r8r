use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct SetNode;

#[async_trait]
impl Node for SetNode {
    fn type_name(&self) -> &'static str {
        "core.set"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let fields = ctx.parameters.get("fields").cloned().unwrap_or(serde_json::json!({}));
        let fields_obj = fields.as_object().cloned().unwrap_or_default();

        let base_items = if ctx.input_items.is_empty() {
            vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]
        } else {
            ctx.input_items.clone()
        };

        let items = base_items
            .into_iter()
            .map(|mut item| {
                let json_for_error = item.json.clone();
                let obj = item.json.as_object_mut().ok_or_else(|| {
                    NodeError::ExecutionFailed(format!(
                        "core.set expects object items, got {}",
                        json_for_error
                    ))
                })?;
                for (k, v) in fields_obj.iter() {
                    obj.insert(k.clone(), v.clone());
                }
                Ok(item)
            })
            .collect::<Result<Vec<_>, NodeError>>()?;

        Ok(vec![items])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn merges_static_fields_into_each_input_item() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![Item { json: serde_json::json!({"existing": true}), binary: serde_json::json!({}) }],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"existing": true, "greeting": "hi"}));
    }

    #[tokio::test]
    async fn with_no_input_items_produces_one_item_from_fields_only() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[0][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[tokio::test]
    async fn non_object_item_json_returns_error_instead_of_panicking() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![Item { json: serde_json::json!([1, 2, 3]), binary: serde_json::json!({}) }],
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
