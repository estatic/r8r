//! Merge v3: append, combine by matching fields or position, choose branch.

use super::{field_list, get_path};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{Map, Value};

pub struct Merge;

/// `a` then `b`'s fields; `b` wins on clashes (n8n's default "prefer input 2").
fn merged(a: &Item, b: &Item, prefer_first: bool) -> Map<String, Value> {
    let (base, over) = if prefer_first { (&b.json, &a.json) } else { (&a.json, &b.json) };
    let mut out = base.clone();
    for (k, v) in over {
        out.insert(k.clone(), v.clone());
    }
    // Keep input 1's key order first.
    if prefer_first {
        return out;
    }
    let mut ordered = a.json.clone();
    for (k, v) in out {
        ordered.insert(k, v);
    }
    ordered
}

fn key_of(item: &Item, fields: &[String]) -> Option<String> {
    let v: Vec<Value> = fields.iter().map(|f| get_path(&item.json_value(), f).cloned().unwrap_or(Value::Null)).collect();
    if v.iter().all(Value::is_null) {
        return None;
    }
    Some(serde_json::to_string(&v).unwrap())
}

#[async_trait::async_trait]
impl NodeType for Merge {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.merge"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let n_inputs = ctx.raw_param("numberInputs").and_then(Value::as_u64).unwrap_or(2) as usize;
        let mut inputs = ctx.inputs.clone();
        inputs.resize(n_inputs.max(inputs.len()), Vec::new());
        let mode = ctx.raw_param("mode").and_then(Value::as_str).unwrap_or("append").to_string();
        let prefer_first = ctx.raw_param("options.clashHandling.values.resolveClash").and_then(Value::as_str) == Some("preferInput1");
        let tag = |items: &[Item], input: usize| -> Vec<Item> { items.iter().cloned().enumerate().map(|(i, it)| it.paired_input(i, input)).collect() };
        let (a, b) = (inputs[0].clone(), inputs.get(1).cloned().unwrap_or_default());
        let out: Vec<Item> = match mode.as_str() {
            "append" => inputs.iter().enumerate().flat_map(|(k, items)| tag(items, k)).collect(),
            "chooseBranch" => {
                let which = ctx.raw_param("useDataOfInput").and_then(Value::as_u64).unwrap_or(1) as usize;
                let output = ctx.raw_param("output").and_then(Value::as_str).unwrap_or("specifiedInput");
                if output == "empty" {
                    vec![Item::default()]
                } else {
                    let k = which.saturating_sub(1);
                    tag(inputs.get(k).map(Vec::as_slice).unwrap_or(&[]), k)
                }
            }
            "combine" | "combineByFields" | "combineByPosition" | "combineAll" => {
                let by = if mode == "combine" {
                    ctx.raw_param("combineBy").and_then(Value::as_str).unwrap_or("combineByFields").to_string()
                } else {
                    mode.clone()
                };
                match by.as_str() {
                    "combineByPosition" => {
                        let include_unpaired = ctx.raw_param("options.includeUnpaired").and_then(Value::as_bool).unwrap_or(false);
                        let len = if include_unpaired { a.len().max(b.len()) } else { a.len().min(b.len()) };
                        (0..len)
                            .map(|i| {
                                let json = match (a.get(i), b.get(i)) {
                                    (Some(x), Some(y)) => merged(x, y, prefer_first),
                                    (Some(x), None) => x.json.clone(),
                                    (None, Some(y)) => y.json.clone(),
                                    (None, None) => Map::new(),
                                };
                                Item { json, binary: None, paired_item: Some(serde_json::json!([{"item": i}, {"item": i, "input": 1}])) }
                            })
                            .collect()
                    }
                    "combineAll" => a.iter().enumerate().flat_map(|(i, x)| b.iter().enumerate().map(move |(j, y)| {
                        Item { json: merged(x, y, prefer_first), binary: None, paired_item: Some(serde_json::json!([{"item": i}, {"item": j, "input": 1}])) }
                    })).collect(),
                    _ => {
                        let (f1, f2) = match ctx.raw_param("fieldsToMatchString") {
                            Some(v) => (field_list(v), field_list(v)),
                            None => {
                                let pairs = ctx.raw_param("mergeByFields.values").and_then(Value::as_array).cloned().unwrap_or_default();
                                (
                                    pairs.iter().filter_map(|p| p["field1"].as_str().map(String::from)).collect(),
                                    pairs.iter().filter_map(|p| p["field2"].as_str().map(String::from)).collect(),
                                )
                            }
                        };
                        if f1.is_empty() {
                            return Err(NodeError::new("You need to define at least one pair of fields in \"Fields to Match\""));
                        }
                        let join = ctx.raw_param("joinMode").and_then(Value::as_str).unwrap_or("keepMatches").to_string();
                        let output_from = ctx.raw_param("outputDataFrom").and_then(Value::as_str).unwrap_or("both").to_string();
                        let keys_b: Vec<Option<String>> = b.iter().map(|it| key_of(it, &f2)).collect();
                        let mut used_b = vec![false; b.len()];
                        let mut matches: Vec<Item> = Vec::new();
                        let mut unmatched_a: Vec<Item> = Vec::new();
                        for (i, x) in a.iter().enumerate() {
                            let key = key_of(x, &f1);
                            let hits: Vec<usize> = keys_b.iter().enumerate().filter(|(_, k)| key.is_some() && **k == key).map(|(j, _)| j).collect();
                            if hits.is_empty() {
                                unmatched_a.push(x.clone().paired(i));
                                continue;
                            }
                            for j in hits {
                                used_b[j] = true;
                                let json = match output_from.as_str() {
                                    "input1" => x.json.clone(),
                                    "input2" => b[j].json.clone(),
                                    _ => merged(x, &b[j], prefer_first),
                                };
                                matches.push(Item { json, binary: None, paired_item: Some(serde_json::json!([{"item": i}, {"item": j, "input": 1}])) });
                            }
                        }
                        let unmatched_b: Vec<Item> = b.iter().enumerate().filter(|(j, _)| !used_b[*j]).map(|(j, y)| y.clone().paired_input(j, 1)).collect();
                        match join.as_str() {
                            "keepMatches" => matches,
                            "keepNonMatches" => match output_from.as_str() {
                                "input1" => unmatched_a,
                                "input2" => unmatched_b,
                                _ => unmatched_a.into_iter().chain(unmatched_b).collect(),
                            },
                            "keepEverything" => matches.into_iter().chain(unmatched_a).chain(unmatched_b).collect(),
                            "enrichInput1" => matches.into_iter().chain(unmatched_a).collect(),
                            "enrichInput2" => matches.into_iter().chain(unmatched_b).collect(),
                            other => return Err(NodeError::new(format!("Unknown join mode: {other}"))),
                        }
                    }
                }
            }
            other => return Err(NodeError::new(format!("The merge mode \"{other}\" is not supported"))),
        };
        Ok(vec![out])
    }
}
