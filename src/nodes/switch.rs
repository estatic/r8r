use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct SwitchNode;

#[async_trait]
impl Node for SwitchNode {
    fn type_name(&self) -> &'static str {
        "core.switch"
    }
    fn display_name(&self) -> &'static str {
        "Switch"
    }
    fn description(&self) -> &'static str {
        "Routes items to one of several outputs based on matching a value against a list of cases."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "🔀"
    }
    fn output_ports(&self, parameters: &serde_json::Value) -> Vec<String> {
        let case_count = parameters.get("cases").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        (0..case_count)
            .map(|i| format!("case {i}"))
            .chain(std::iter::once("default".to_string()))
            .collect()
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
            ..Default::default()
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
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 3); // 2 cases + 1 default
        assert!(result[0].is_empty());
        assert!(result[1].is_empty());
        assert_eq!(result[2], items());
    }

    #[test]
    fn output_ports_with_no_cases_is_just_default() {
        let node = SwitchNode;
        assert_eq!(node.output_ports(&serde_json::json!({})), vec!["default".to_string()]);
    }

    #[test]
    fn output_ports_reflects_case_count() {
        let node = SwitchNode;
        let params = serde_json::json!({"cases": ["a", "b"]});
        assert_eq!(node.output_ports(&params), vec!["case 0".to_string(), "case 1".to_string(), "default".to_string()]);
    }
}
