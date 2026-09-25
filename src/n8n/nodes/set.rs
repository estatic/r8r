//! Edit Fields (Set) v3: typed assignments or a raw JSON template, applied
//! to each item.

use super::{field_list, set_path};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{Map, Value};

pub struct Set;

fn type_name_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Converts `value` to the assignment's declared type, as n8n's
/// `validateFieldType` does.
fn convert(name: &str, value: Value, ty: &str) -> Result<Value, String> {
    let fail = |v: &Value| Err(format!("'{name}' expects a {ty} but we got '{}'", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())));
    match (ty, &value) {
        (_, Value::Null) => Ok(Value::Null),
        ("string", Value::String(_)) => Ok(value),
        ("string", Value::Array(_) | Value::Object(_)) => Ok(Value::String(value.to_string())),
        ("string", other) => Ok(Value::String(other.to_string())),
        ("number", Value::Number(_)) => Ok(value),
        ("number", Value::String(s)) => match s.trim().parse::<f64>() {
            Ok(n) if !s.trim().is_empty() => Ok(serde_json::Number::from_f64(n)
                .map(|n| if n.as_f64().is_some_and(|f| f.fract() == 0.0 && f.abs() < 9e15) { Value::from(n.as_f64().unwrap() as i64) } else { Value::Number(n) })
                .unwrap_or(Value::Null)),
            _ => fail(&value),
        },
        ("number", Value::Bool(b)) => Ok(Value::from(*b as i64)),
        ("boolean", Value::Bool(_)) => Ok(value),
        ("boolean", Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(Value::Bool(true)),
            "false" | "0" | "no" => Ok(Value::Bool(false)),
            _ => fail(&value),
        },
        ("boolean", Value::Number(n)) => Ok(Value::Bool(n.as_f64() != Some(0.0))),
        ("array", Value::Array(_)) => Ok(value),
        ("array", Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Array(_)) => Ok(v),
            _ => fail(&value),
        },
        ("object", Value::Object(_)) => Ok(value),
        ("object", Value::String(s)) => match serde_json::from_str::<Value>(s) {
            Ok(v @ Value::Object(_)) => Ok(v),
            _ => fail(&value),
        },
        ("any" | "", _) => Ok(value),
        _ => Err(format!("'{name}' expects a {ty} but we got a {}", type_name_of(&value))),
    }
}

#[async_trait::async_trait]
impl NodeType for Set {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.set"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input: Vec<Item> = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let mode = ctx.raw_param("mode").and_then(Value::as_str).unwrap_or("manual").to_string();
        let include_other = ctx.raw_param("includeOtherFields").and_then(Value::as_bool).unwrap_or(false);
        let include = ctx.raw_param("include").and_then(Value::as_str).unwrap_or("all").to_string();
        let dot = ctx.raw_param("options.dotNotation").and_then(Value::as_bool).unwrap_or(true);
        let ignore_conversion = ctx.raw_param("options.ignoreConversionErrors").and_then(Value::as_bool).unwrap_or(false);
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            let result: NodeResult<Item> = (|| {
                let mut json = Map::new();
                if include_other {
                    match include.as_str() {
                        "selected" => {
                            for f in field_list(ctx.raw_param("includeFields").unwrap_or(&Value::Null)) {
                                if let Some(v) = item.json.get(&f) {
                                    json.insert(f, v.clone());
                                }
                            }
                        }
                        "except" => {
                            let except = field_list(ctx.raw_param("excludeFields").unwrap_or(&Value::Null));
                            json = item.json.iter().filter(|(k, _)| !except.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
                        }
                        _ => json = item.json.clone(),
                    }
                }
                if mode == "raw" {
                    let raw = ctx.param("jsonOutput", i)?;
                    let parsed = match raw {
                        Value::String(s) => serde_json::from_str::<Value>(&s)
                            .map_err(|e| NodeError::new(format!("The 'JSON Output' in item {i} contains invalid JSON")).describe(e.to_string()))?,
                        other => other,
                    };
                    let Value::Object(obj) = parsed else {
                        return Err(NodeError::new(format!("The 'JSON Output' in item {i} does not contain a valid JSON object")));
                    };
                    for (k, v) in obj {
                        json.insert(k, v);
                    }
                } else {
                    let assignments = ctx.raw_param("assignments.assignments").and_then(Value::as_array).cloned().unwrap_or_default();
                    for (a, assignment) in assignments.iter().enumerate() {
                        let name = match ctx.param(&format!("assignments.assignments.{a}.name"), i)? {
                            Value::String(s) => s,
                            other => other.to_string(),
                        };
                        let value = ctx.param(&format!("assignments.assignments.{a}.value"), i)?;
                        let ty = assignment["type"].as_str().unwrap_or("string");
                        let value = match convert(&name, value.clone(), ty) {
                            Ok(v) => v,
                            Err(_) if ignore_conversion => value,
                            Err(m) => return Err(NodeError::new(m).at(i)),
                        };
                        if dot {
                            set_path(&mut json, &name, value);
                        } else {
                            json.insert(name, value);
                        }
                    }
                }
                Ok(Item { json, binary: if include_other { item.binary.clone() } else { None }, paired_item: None }.paired(i))
            })();
            match result {
                Ok(item) => out.push(item),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}
