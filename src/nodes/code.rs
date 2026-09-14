use crate::domain::Item;
use crate::expr::{eval_js, EvalContext};
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::collections::HashMap;

const CODE_TIMEOUT_SECS: u64 = 2;

pub struct CodeNode;

#[async_trait]
impl Node for CodeNode {
    fn type_name(&self) -> &'static str {
        "core.code"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let script = ctx
            .parameters
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("core.code requires a \"script\" parameter".into()))?
            .to_string();

        // `eval_js` wraps each entry of `EvalContext.items` in `{"json": ...}`
        // itself when building the `$items()` array (see `src/expr.rs`), so
        // these must be the raw item values, not pre-wrapped here — otherwise
        // scripts would see a double-wrapped `{json: {json: ...}}` shape.
        let items_json: Vec<serde_json::Value> =
            ctx.input_items.iter().map(|i| i.json.clone()).collect();
        let empty_node_json: HashMap<String, serde_json::Value> = HashMap::new();
        let eval_ctx = EvalContext {
            json: ctx.input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})),
            items: &items_json,
            node_json: &empty_node_json,
            workflow_name: "",
        };
        let wrapped_script = format!("(function(items) {{ {script} }})($items())");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(CODE_TIMEOUT_SECS),
            async { eval_js(&wrapped_script, &eval_ctx) },
        )
        .await
        .map_err(|_| NodeError::ExecutionFailed(format!("core.code timed out after {CODE_TIMEOUT_SECS}s")))?
        .map_err(|e| NodeError::ExecutionFailed(e.to_string()))?;

        let result_array = result
            .as_array()
            .ok_or_else(|| NodeError::ExecutionFailed("core.code script must return an array".into()))?;

        let out_items = result_array
            .iter()
            .map(|entry| {
                let json = entry.get("json").cloned().unwrap_or_else(|| entry.clone());
                Item { json, binary: serde_json::json!({}) }
            })
            .collect();

        Ok(vec![out_items])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn script_transforms_items() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "script": "return items.map(i => ({json: {doubled: i.json.n * 2}}));"
            }),
            input_items: vec![
                Item { json: serde_json::json!({"n": 1}), binary: serde_json::json!({}) },
                Item { json: serde_json::json!({"n": 2}), binary: serde_json::json!({}) },
            ],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"doubled": 2}));
        assert_eq!(result[0][1].json, serde_json::json!({"doubled": 4}));
    }

    #[tokio::test]
    async fn missing_script_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn non_array_result_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"script": "return 42;"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn empty_input_produces_empty_items_array() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"script": "return items;"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
