//! Item-list transformations.

use super::{field_list, get_path, set_path};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::workflow::Node;
use serde_json::{json, Map, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![
        Box::new(SplitInBatches),
        Box::new(SplitOut),
        Box::new(Aggregate),
        Box::new(Sort),
        Box::new(Limit),
        Box::new(RemoveDuplicates),
        Box::new(Summarize),
        Box::new(CompareDatasets),
    ]
}

fn indexed(items: &[Item]) -> Vec<Item> {
    items.iter().cloned().enumerate().map(|(i, it)| it.paired(i)).collect()
}

/// Loop Over Items (v3): output 0 "done", output 1 "loop". The first run
/// takes all input items; later runs receive the processed batch back.
struct SplitInBatches;

#[async_trait::async_trait]
impl NodeType for SplitInBatches {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.splitInBatches"
    }
    fn outputs(&self, _: &Node) -> usize {
        2
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let batch = ctx.param_f64("batchSize", 0, 1.0)?.max(1.0) as usize;
        let reset = ctx.param_bool("options.reset", 0, false)?;
        let key = ctx.node.name.clone();
        let mut run = ctx.run.lock().unwrap();
        let state = run.node_state.entry(key.clone()).or_insert(Value::Null);
        let fresh = state.is_null() || reset;
        let (mut remaining, mut processed): (Vec<Value>, Vec<Value>) = if fresh {
            (ctx.input().iter().map(|i| serde_json::to_value(i).unwrap()).collect(), Vec::new())
        } else {
            let mut processed: Vec<Value> = state["processed"].as_array().cloned().unwrap_or_default();
            processed.extend(ctx.input().iter().map(|i| serde_json::to_value(i).unwrap()));
            (state["remaining"].as_array().cloned().unwrap_or_default(), processed)
        };
        if remaining.is_empty() {
            *state = Value::Null;
            let done: Vec<Item> = processed.drain(..).filter_map(|v| serde_json::from_value(v).ok()).collect();
            return Ok(vec![indexed(&done), vec![]]);
        }
        let next: Vec<Value> = remaining.drain(..batch.min(remaining.len())).collect();
        *state = json!({"remaining": remaining, "processed": processed});
        let next: Vec<Item> = next.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
        Ok(vec![vec![], indexed(&next)])
    }
}

struct SplitOut;

#[async_trait::async_trait]
impl NodeType for SplitOut {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.splitOut"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        for (i, item) in ctx.input().to_vec().iter().enumerate() {
            let fields = field_list(&ctx.param("fieldToSplitOut", i)?);
            let include = ctx.param_str("include", i, "noOtherFields")?;
            let dest = ctx.param_str("options.destinationFieldName", i, "")?;
            for field in fields {
                let values = match get_path(&item.json_value(), &field) {
                    Some(Value::Array(a)) => a.clone(),
                    Some(Value::Object(o)) => o.values().cloned().collect(),
                    Some(Value::Null) | None => {
                        return Err(NodeError::new(format!("The field '{field}' wasn't found in item {i}")).at(i));
                    }
                    Some(other) => vec![other.clone()],
                };
                let name = if dest.is_empty() { field.clone() } else { dest.clone() };
                for v in values {
                    let mut json = Map::new();
                    match include.as_str() {
                        "allOtherFields" => {
                            json = item.json.clone();
                            json.remove(&field);
                            json.insert(name.clone(), v);
                        }
                        "selectedOtherFields" => {
                            json.insert(name.clone(), v);
                            for f in field_list(&ctx.param("fieldsToInclude", i)?) {
                                if let Some(x) = get_path(&item.json_value(), &f) {
                                    set_path(&mut json, &f, x.clone());
                                }
                            }
                        }
                        _ => match v {
                            Value::Object(o) if dest.is_empty() => json = o,
                            other => {
                                json.insert(name.clone(), other);
                            }
                        },
                    }
                    out.push(Item::new(json).paired(i));
                }
            }
        }
        Ok(vec![out])
    }
}

struct Aggregate;

#[async_trait::async_trait]
impl NodeType for Aggregate {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.aggregate"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items = ctx.input().to_vec();
        let mode = ctx.param_str("aggregate", 0, "aggregateIndividualFields")?;
        let paired: Vec<Value> = (0..items.len()).map(|i| json!({"item": i})).collect();
        let mut json = Map::new();
        if mode == "aggregateAllItemData" {
            let dest = ctx.param_str("destinationFieldName", 0, "data")?;
            json.insert(dest, Value::Array(items.iter().map(Item::json_value).collect()));
        } else {
            let keep_missing = ctx.param_bool("options.keepMissing", 0, false)?;
            let merge_lists = ctx.param_bool("options.mergeLists", 0, false)?;
            let fields = ctx.raw_param("fieldsToAggregate.fieldToAggregate").and_then(Value::as_array).cloned().unwrap_or_default();
            for f in fields {
                let field = f["fieldToAggregate"].as_str().unwrap_or_default().to_string();
                let name = if f["renameField"].as_bool().unwrap_or(false) { f["outputFieldName"].as_str().unwrap_or(&field).to_string() } else { field.clone() };
                let mut values = Vec::new();
                for item in &items {
                    match get_path(&item.json_value(), &field) {
                        Some(Value::Array(a)) if merge_lists => values.extend(a.iter().cloned()),
                        Some(Value::Null) | None if !keep_missing => {}
                        Some(v) => values.push(v.clone()),
                        None => values.push(Value::Null),
                    }
                }
                json.insert(name, Value::Array(values));
            }
        }
        Ok(vec![vec![Item { json, binary: None, paired_item: Some(Value::Array(paired)) }]])
    }
}

fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64().partial_cmp(&y.as_f64()).unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Greater,
        (_, Value::Null) => Ordering::Less,
        _ => a.to_string().cmp(&b.to_string()),
    }
}

struct Sort;

#[async_trait::async_trait]
impl NodeType for Sort {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.sort"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut items = indexed(ctx.input());
        let kind = ctx.param_str("type", 0, "simple")?;
        match kind.as_str() {
            "random" => {
                use rand::seq::SliceRandom;
                items.shuffle(&mut rand::thread_rng());
            }
            _ => {
                let fields = ctx.raw_param("sortFieldsUi.sortField").and_then(Value::as_array).cloned().unwrap_or_default();
                items.sort_by(|a, b| {
                    for f in &fields {
                        let name = f["fieldName"].as_str().unwrap_or_default();
                        let ord = compare_values(get_path(&a.json_value(), name).unwrap_or(&Value::Null), get_path(&b.json_value(), name).unwrap_or(&Value::Null));
                        let ord = if f["order"].as_str() == Some("descending") { ord.reverse() } else { ord };
                        if ord != std::cmp::Ordering::Equal {
                            return ord;
                        }
                    }
                    std::cmp::Ordering::Equal
                });
            }
        }
        Ok(vec![items])
    }
}

struct Limit;

#[async_trait::async_trait]
impl NodeType for Limit {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.limit"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let max = ctx.param_f64("maxItems", 0, 1.0)?.max(0.0) as usize;
        let items = indexed(ctx.input());
        let out = if ctx.param_str("keep", 0, "firstItems")? == "lastItems" {
            items[items.len().saturating_sub(max)..].to_vec()
        } else {
            items.into_iter().take(max).collect()
        };
        Ok(vec![out])
    }
}

struct RemoveDuplicates;

#[async_trait::async_trait]
impl NodeType for RemoveDuplicates {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.removeDuplicates"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let compare = ctx.param_str("compare", 0, "allFields")?;
        let selected = field_list(&ctx.param("fieldsToCompare", 0)?);
        let except = field_list(&ctx.param("fieldsToExclude", 0)?);
        let mut seen: Vec<Value> = Vec::new();
        let mut out = Vec::new();
        for (i, item) in ctx.input().iter().enumerate() {
            let key = match compare.as_str() {
                "selectedFields" => Value::Array(selected.iter().map(|f| get_path(&item.json_value(), f).cloned().unwrap_or(Value::Null)).collect()),
                "allFieldsExcept" => Value::Object(item.json.iter().filter(|(k, _)| !except.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect()),
                _ => item.json_value(),
            };
            if !seen.contains(&key) {
                seen.push(key);
                out.push(item.clone().paired(i));
            }
        }
        Ok(vec![out])
    }
}

struct Summarize;

#[async_trait::async_trait]
impl NodeType for Summarize {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.summarize"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let aggs = ctx.raw_param("fieldsToSummarize.values").and_then(Value::as_array).cloned().unwrap_or_default();
        let split_by = field_list(&ctx.param("fieldsToSplitBy", 0)?);
        let items = ctx.input().to_vec();
        // Groups in order of first appearance.
        let mut groups: Vec<(Vec<Value>, Vec<usize>)> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let key: Vec<Value> = split_by.iter().map(|f| get_path(&item.json_value(), f).cloned().unwrap_or(Value::Null)).collect();
            match groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, members)) => members.push(i),
                None => groups.push((key, vec![i])),
            }
        }
        let mut out = Vec::new();
        for (key, members) in groups {
            let mut json = Map::new();
            for (f, v) in split_by.iter().zip(key) {
                json.insert(f.clone(), v);
            }
            for agg in &aggs {
                let kind = agg["aggregation"].as_str().unwrap_or("count");
                let field = agg["field"].as_str().unwrap_or_default();
                let values: Vec<Value> = members.iter().filter_map(|i| get_path(&items[*i].json_value(), field).cloned()).filter(|v| !v.is_null()).collect();
                let nums: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
                let result = match kind {
                    "count" => json!(values.len()),
                    "countUnique" => {
                        let mut u: Vec<&Value> = Vec::new();
                        for v in &values {
                            if !u.contains(&v) {
                                u.push(v);
                            }
                        }
                        json!(u.len())
                    }
                    "sum" => number(nums.iter().sum()),
                    "average" => if nums.is_empty() { Value::Null } else { number(nums.iter().sum::<f64>() / nums.len() as f64) },
                    "min" => nums.iter().cloned().reduce(f64::min).map(number).unwrap_or(Value::Null),
                    "max" => nums.iter().cloned().reduce(f64::max).map(number).unwrap_or(Value::Null),
                    "append" => Value::Array(values.clone()),
                    "concatenate" => json!(values.iter().map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join(agg["separateBy"].as_str().map(|s| if s == "other" { "" } else { s }).unwrap_or(","))),
                    other => return Err(NodeError::new(format!("Unknown aggregation: {other}"))),
                };
                json.insert(format!("{kind}_{field}"), result);
            }
            let paired: Vec<Value> = members.iter().map(|i| json!({"item": i})).collect();
            out.push(Item { json, binary: None, paired_item: Some(Value::Array(paired)) });
        }
        Ok(vec![out])
    }
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// Outputs: 0 in A only, 1 same, 2 different, 3 in B only.
struct CompareDatasets;

#[async_trait::async_trait]
impl NodeType for CompareDatasets {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.compareDatasets"
    }
    fn outputs(&self, _: &Node) -> usize {
        4
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let pairs = ctx.raw_param("mergeByFields.values").and_then(Value::as_array).cloned().unwrap_or_default();
        let f1: Vec<String> = pairs.iter().filter_map(|p| p["field1"].as_str().map(String::from)).collect();
        let f2: Vec<String> = pairs.iter().filter_map(|p| p["field2"].as_str().map(String::from)).collect();
        let a = ctx.inputs.first().cloned().unwrap_or_default();
        let b = ctx.inputs.get(1).cloned().unwrap_or_default();
        let key = |it: &Item, fs: &[String]| -> Vec<Value> { fs.iter().map(|f| get_path(&it.json_value(), f).cloned().unwrap_or(Value::Null)).collect() };
        let mut used = vec![false; b.len()];
        let mut outs: NodeOutput = vec![Vec::new(); 4];
        for (i, x) in a.iter().enumerate() {
            let k = key(x, &f1);
            match b.iter().enumerate().position(|(j, y)| !used[j] && key(y, &f2) == k) {
                None => outs[0].push(x.clone().paired(i)),
                Some(j) => {
                    used[j] = true;
                    let y = &b[j];
                    if x.json == y.json {
                        outs[1].push(x.clone().paired(i));
                    } else {
                        let mut different = Map::new();
                        let mut same = Map::new();
                        for k in x.json.keys().chain(y.json.keys()) {
                            let (va, vb) = (x.json.get(k), y.json.get(k));
                            if va == vb {
                                same.insert(k.clone(), va.cloned().unwrap_or(Value::Null));
                            } else {
                                different.insert(k.clone(), json!({"inputA": va, "inputB": vb}));
                            }
                        }
                        let mut keys = Map::new();
                        for (f, v) in f1.iter().zip(k) {
                            keys.insert(f.clone(), v);
                        }
                        let mut json = Map::new();
                        json.insert("keys".into(), Value::Object(keys));
                        json.insert("same".into(), Value::Object(same));
                        json.insert("different".into(), Value::Object(different));
                        outs[2].push(Item { json, binary: None, paired_item: Some(json!([{"item": i}, {"item": j, "input": 1}])) });
                    }
                }
            }
        }
        for (j, y) in b.iter().enumerate() {
            if !used[j] {
                outs[3].push(y.clone().paired_input(j, 1));
            }
        }
        Ok(outs)
    }
}
