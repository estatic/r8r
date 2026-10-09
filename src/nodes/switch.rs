use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use serde_json::Value;

pub struct SwitchNode;

#[async_trait]
impl Node for SwitchNode {
    fn type_name(&self) -> &'static str {
        "core.switch"
    }
    fn runs_per_item(&self) -> bool {
        true
    }
    fn display_name(&self) -> &'static str {
        "Switch"
    }
    fn description(&self) -> &'static str {
        "Routes each item to an output: the first rule it meets, or the output an expression gives."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "🔀"
    }
    fn output_ports(&self, parameters: &serde_json::Value) -> Vec<String> {
        match mode(parameters) {
            Mode::Rules(rules) => {
                let mut ports: Vec<String> = rules
                    .iter()
                    .enumerate()
                    .map(|(i, r)| {
                        r.get("output_name")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| i.to_string())
                    })
                    .collect();
                if fallback(parameters) == Fallback::Extra {
                    ports.push("Fallback".into());
                }
                if ports.is_empty() {
                    ports.push("0".into());
                }
                ports
            }
            Mode::Expression => (0..number_outputs(parameters)).map(|i| i.to_string()).collect(),
            Mode::Cases(cases) => (0..cases.len()).map(|i| format!("case {i}")).chain(std::iter::once("default".to_string())).collect(),
        }
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let p = &ctx.parameters;
        let ports = self.output_ports(p).len();
        let mut out: Vec<Vec<Item>> = vec![Vec::new(); ports];
        let targets: Vec<usize> = match mode(p) {
            Mode::Rules(rules) => {
                let all = p.get("all_matching_outputs").and_then(Value::as_bool).unwrap_or(false);
                let mut hits = Vec::new();
                for (i, rule) in rules.iter().enumerate() {
                    let conditions = rule.get("conditions").ok_or_else(|| fail(format!("rule {} has no conditions", i + 1)))?;
                    if super::conditions::holds(&format!("core.switch rule {}", i + 1), conditions)? {
                        hits.push(i);
                        if !all {
                            break;
                        }
                    }
                }
                if hits.is_empty() {
                    match fallback(p) {
                        Fallback::None => {}
                        Fallback::Extra => hits.push(rules.len()),
                        Fallback::Output(n) if n < rules.len() => hits.push(n),
                        Fallback::Output(n) => return Err(fail(format!("the fallback output {n} doesn't exist"))),
                    }
                }
                hits
            }
            Mode::Expression => {
                let n = number_outputs(p);
                let index = match p.get("output") {
                    Some(Value::Number(x)) => x.as_u64(),
                    Some(Value::String(s)) => s.trim().parse().ok(),
                    _ => None,
                }
                .ok_or_else(|| fail(format!("the output index must be a whole number, got {}", p.get("output").unwrap_or(&Value::Null))))?;
                if index as usize >= n {
                    return Err(fail(format!("output {index} doesn't exist (the node has {n} outputs, 0 to {})", n - 1)));
                }
                vec![index as usize]
            }
            Mode::Cases(cases) => {
                let value = p.get("value").cloned().unwrap_or(Value::Null);
                vec![cases.iter().position(|c| *c == value).unwrap_or(cases.len())]
            }
        };
        for t in targets {
            out[t].extend(ctx.input_items.iter().cloned());
        }
        Ok(out)
    }
}

fn fail(msg: impl std::fmt::Display) -> NodeError {
    NodeError::ExecutionFailed(format!("core.switch: {msg}"))
}

enum Mode {
    /// Each rule is `{output_name?, conditions}`; outputs follow the rules.
    Rules(Vec<Value>),
    /// `output` (an expression) gives the output index, of `number_outputs`.
    Expression,
    /// Nodes saved before rules: `value` matched exactly against `cases`.
    Cases(Vec<Value>),
}

fn mode(p: &Value) -> Mode {
    match p.get("mode").and_then(Value::as_str) {
        Some("expression") => Mode::Expression,
        Some("rules") => Mode::Rules(p.get("rules").and_then(Value::as_array).cloned().unwrap_or_default()),
        _ => match p.get("rules").and_then(Value::as_array) {
            Some(rules) => Mode::Rules(rules.clone()),
            None => Mode::Cases(p.get("cases").and_then(Value::as_array).cloned().unwrap_or_default()),
        },
    }
}

fn number_outputs(p: &Value) -> usize {
    p.get("number_outputs").and_then(Value::as_u64).map(|n| n.clamp(1, 32) as usize).unwrap_or(4)
}

#[derive(PartialEq)]
enum Fallback {
    None,
    Extra,
    Output(usize),
}

/// Where an item that meets no rule goes: dropped (n8n's default), an
/// extra "Fallback" output, or one of the rule outputs.
fn fallback(p: &Value) -> Fallback {
    match p.get("fallback_output") {
        Some(Value::String(s)) if s == "extra" => Fallback::Extra,
        Some(Value::Number(n)) => n.as_u64().map_or(Fallback::None, |n| Fallback::Output(n as usize)),
        Some(Value::String(s)) => s.parse().map_or(Fallback::None, Fallback::Output),
        _ => Fallback::None,
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

    fn rule(name: &str, left: serde_json::Value, op: &str, right: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"output_name": name, "conditions": {"combinator": "and", "rules": [{"left": left, "operator": op, "right": right}]}})
    }

    async fn route(params: serde_json::Value) -> Result<Vec<usize>, NodeError> {
        let out = SwitchNode.execute(&NodeExecutionContext { parameters: params, input_items: items(), ..Default::default() }).await?;
        Ok(out.iter().map(Vec::len).collect())
    }

    #[tokio::test]
    async fn rules_send_an_item_to_the_first_rule_it_meets() {
        let params = serde_json::json!({"mode": "rules", "rules": [rule("small", 3.into(), "lt", "2".into()), rule("big", 3.into(), "gte", "2".into()), rule("any", 3.into(), "exists", serde_json::Value::Null)]});
        assert_eq!(route(params.clone()).await.unwrap(), vec![0, 1, 0]);
        assert_eq!(SwitchNode.output_ports(&params), vec!["small", "big", "any"]);
        let mut all = params;
        all["all_matching_outputs"] = true.into();
        assert_eq!(route(all).await.unwrap(), vec![0, 1, 1]);
    }

    #[tokio::test]
    async fn an_item_meeting_no_rule_is_dropped_or_goes_to_the_fallback() {
        let rules = serde_json::json!([rule("", "a".into(), "equals", "b".into())]);
        assert_eq!(route(serde_json::json!({"rules": rules})).await.unwrap(), vec![0]);
        let extra = serde_json::json!({"rules": rules, "fallback_output": "extra"});
        assert_eq!(SwitchNode.output_ports(&extra), vec!["0", "Fallback"]);
        assert_eq!(route(extra).await.unwrap(), vec![0, 1]);
        assert_eq!(route(serde_json::json!({"rules": rules, "fallback_output": 0})).await.unwrap(), vec![1]);
        assert!(route(serde_json::json!({"rules": rules, "fallback_output": 3})).await.is_err());
    }

    #[tokio::test]
    async fn expression_mode_uses_the_output_it_gives() {
        let params = serde_json::json!({"mode": "expression", "number_outputs": 3, "output": "2"});
        assert_eq!(SwitchNode.output_ports(&params), vec!["0", "1", "2"]);
        assert_eq!(route(params).await.unwrap(), vec![0, 0, 1]);
        let err = route(serde_json::json!({"mode": "expression", "number_outputs": 2, "output": 5})).await.unwrap_err();
        assert!(err.to_string().contains("output 5 doesn't exist"), "{err}");
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
