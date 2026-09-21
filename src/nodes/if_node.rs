use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct IfNode;

#[async_trait]
impl Node for IfNode {
    fn type_name(&self) -> &'static str {
        "core.if"
    }
    fn display_name(&self) -> &'static str {
        "If"
    }
    fn description(&self) -> &'static str {
        "Routes items to a \"true\" or \"false\" output based on a boolean condition."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "❓"
    }
    fn output_ports(&self, _parameters: &serde_json::Value) -> Vec<String> {
        vec!["true".to_string(), "false".to_string()]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = match ctx.parameters.get("condition") {
            None => false,
            Some(serde_json::Value::Bool(b)) => *b,
            Some(other) => {
                return Err(NodeError::ExecutionFailed(format!(
                    "core.if \"condition\" must be a boolean, got: {other}"
                )));
            }
        };

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
            ..Default::default()
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
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }

    #[tokio::test]
    async fn missing_condition_defaults_to_false() {
        let node = IfNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items(), ..Default::default() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }

    #[tokio::test]
    async fn present_non_boolean_condition_returns_error() {
        // A present-but-non-boolean condition (e.g. `{{ $json.count }}`
        // resolving to the number 5) must be a loud error, not a silent
        // fall-through to the `false` branch — see missing_condition_defaults_to_false
        // above for the (unrelated, still-correct) absent-key case.
        let node = IfNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": 5}),
            input_items: items(),
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[test]
    fn output_ports_are_fixed_true_false_regardless_of_parameters() {
        let node = IfNode;
        assert_eq!(node.output_ports(&serde_json::json!({})), vec!["true".to_string(), "false".to_string()]);
        assert_eq!(
            node.output_ports(&serde_json::json!({"condition": true})),
            vec!["true".to_string(), "false".to_string()]
        );
    }
}
