//! If, Filter and Switch.

use super::conditions::evaluate_conditions;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::NodeOutput;
use crate::n8n::workflow::Node;
use serde_json::Value;

fn loose(ctx: &ExecCtx<'_>) -> bool {
    ctx.raw_param("looseTypeValidation").and_then(Value::as_bool).unwrap_or(false)
}

pub struct If;

#[async_trait::async_trait]
impl NodeType for If {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.if"
    }
    fn outputs(&self, _: &Node) -> usize {
        2
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let (mut yes, mut no) = (Vec::new(), Vec::new());
        let loose = loose(ctx);
        for (i, item) in ctx.input().to_vec().into_iter().enumerate() {
            match evaluate_conditions(ctx, "conditions", i, loose) {
                Ok(true) => yes.push(item.paired(i)),
                Ok(false) => no.push(item.paired(i)),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![yes, no])
    }
}

pub struct Filter;

#[async_trait::async_trait]
impl NodeType for Filter {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.filter"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut kept = Vec::new();
        let loose = loose(ctx);
        for (i, item) in ctx.input().to_vec().into_iter().enumerate() {
            match evaluate_conditions(ctx, "conditions", i, loose) {
                Ok(true) => kept.push(item.paired(i)),
                Ok(false) => {}
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![kept])
    }
}

pub struct Switch;

impl Switch {
    fn rule_count(node: &Node) -> usize {
        node.parameters.pointer("/rules/values").and_then(Value::as_array).map(Vec::len).unwrap_or(0)
    }
    fn extra_fallback(node: &Node) -> bool {
        node.parameters.pointer("/options/fallbackOutput").and_then(Value::as_str) == Some("extra")
    }
}

#[async_trait::async_trait]
impl NodeType for Switch {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.switch"
    }
    fn outputs(&self, node: &Node) -> usize {
        if node.parameters["mode"].as_str() == Some("expression") {
            return node.parameters["numberOutputs"].as_u64().unwrap_or(4) as usize;
        }
        Self::rule_count(node) + Self::extra_fallback(node) as usize
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let n = self.outputs(ctx.node);
        let mut outs: NodeOutput = vec![Vec::new(); n];
        let items = ctx.input().to_vec();
        if ctx.raw_param("mode").and_then(Value::as_str) == Some("expression") {
            for (i, item) in items.into_iter().enumerate() {
                let idx = ctx.param_f64("output", i, -1.0)?;
                if idx < 0.0 || idx as usize >= n {
                    return Err(NodeError::new(format!("The output {idx} is not allowed. It has to be between 0 and {}!", n - 1)).at(i));
                }
                outs[idx as usize].push(item.paired(i));
            }
            return Ok(outs);
        }
        let rules = Self::rule_count(ctx.node);
        let all_matching = ctx.raw_param("options.allMatchingOutputs").and_then(Value::as_bool).unwrap_or(false);
        let fallback = ctx.raw_param("options.fallbackOutput").cloned().unwrap_or(Value::Null);
        let loose = ctx.raw_param("options.looseTypeValidation").and_then(Value::as_bool).unwrap_or(false);
        for (i, item) in items.into_iter().enumerate() {
            let mut matched = false;
            for r in 0..rules {
                match evaluate_conditions(ctx, &format!("rules.values.{r}.conditions"), i, loose) {
                    Ok(true) => {
                        outs[r].push(item.clone().paired(i));
                        matched = true;
                        if !all_matching {
                            break;
                        }
                    }
                    Ok(false) => {}
                    Err(e) => return Err(e),
                }
            }
            if !matched {
                match &fallback {
                    Value::String(s) if s == "extra" => outs[rules].push(item.paired(i)),
                    Value::Number(n) => {
                        if let Some(o) = n.as_u64().map(|o| o as usize).filter(|o| *o < outs.len()) {
                            outs[o].push(item.paired(i));
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(outs)
    }
}
