//! n8n's filter conditions (If v2, Filter v2, Switch v3): typed operators,
//! and/or combinator, case sensitivity and strict or loose type validation.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult};
use serde_json::Value;

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn is_iso_date(s: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(s).is_ok() || chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// Loose conversion of `v` to `ty`, as n8n does when type validation is
/// "loose".
fn loosen(v: &Value, ty: &str) -> Option<Value> {
    match (ty, v) {
        ("number", Value::String(s)) => s.trim().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number),
        ("number", Value::Bool(b)) => Some(Value::from(*b as i64)),
        ("string", Value::Number(_) | Value::Bool(_)) => Some(Value::String(v.to_string())),
        ("boolean", Value::String(s)) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(Value::Bool(true)),
            "false" | "0" => Some(Value::Bool(false)),
            _ => None,
        },
        ("boolean", Value::Number(n)) => Some(Value::Bool(n.as_f64() != Some(0.0))),
        ("array" | "object", Value::String(s)) => serde_json::from_str(s).ok(),
        _ => None,
    }
}

fn validate(v: Value, ty: &str, strict: bool, which: &str, cond: usize, item: usize) -> NodeResult<Value> {
    let actual = type_of(&v);
    let ok = matches!(actual, "null") || ty == "any" || actual == ty || (ty == "dateTime" && v.as_str().is_some_and(is_iso_date));
    if ok {
        return Ok(v);
    }
    if !strict {
        if let Some(converted) = loosen(&v, ty) {
            return Ok(converted);
        }
    }
    let shown = v.as_str().map(|s| format!("'{s}'")).unwrap_or_else(|| v.to_string());
    Err(NodeError::new(format!("Wrong type: {shown} is a {actual} but was expecting a {ty} [condition {cond}, item {item}]"))
        .describe(format!("Try changing the type of comparison, or enable 'Convert types where required' ({which} value)"))
        .at(item))
}

fn as_f64(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

fn to_ts(v: &Value) -> Option<i64> {
    match v {
        Value::String(s) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp_millis())
            .ok()
            .or_else(|| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// One comparison. `left`/`right` are already type-validated.
fn compare(ty: &str, op: &str, left: &Value, right: &Value, case_sensitive: bool) -> NodeResult<bool> {
    let lc = |v: &Value| -> String {
        let s = v.as_str().map(String::from).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() });
        if case_sensitive { s } else { s.to_lowercase() }
    };
    Ok(match (ty, op) {
        (_, "exists") => !left.is_null(),
        (_, "notExists") => left.is_null(),
        (_, "empty") => is_empty(left),
        (_, "notEmpty") => !is_empty(left),
        ("string", "equals") => lc(left) == lc(right),
        ("string", "notEquals") => lc(left) != lc(right),
        ("string", "contains") => lc(left).contains(&lc(right)),
        ("string", "notContains") => !lc(left).contains(&lc(right)),
        ("string", "startsWith") => lc(left).starts_with(&lc(right)),
        ("string", "notStartsWith") => !lc(left).starts_with(&lc(right)),
        ("string", "endsWith") => lc(left).ends_with(&lc(right)),
        ("string", "notEndsWith") => !lc(left).ends_with(&lc(right)),
        ("string", "regex" | "notRegex") => {
            let pattern = right.as_str().unwrap_or_default();
            let (pattern, flags) = match pattern.strip_prefix('/').and_then(|p| p.rsplit_once('/')) {
                Some((p, f)) => (p.to_string(), f.to_string()),
                None => (pattern.to_string(), String::new()),
            };
            let re = regex::RegexBuilder::new(&pattern)
                .case_insensitive(!case_sensitive || flags.contains('i'))
                .build()
                .map_err(|e| NodeError::new(format!("Invalid regex: {e}")))?;
            let hit = re.is_match(left.as_str().unwrap_or_default());
            if op == "regex" { hit } else { !hit }
        }
        ("number", _) => {
            let (Some(a), b) = (as_f64(left), as_f64(right)) else { return Ok(false) };
            let b = b.unwrap_or(f64::NAN);
            match op {
                "equals" => a == b,
                "notEquals" => a != b,
                "gt" => a > b,
                "lt" => a < b,
                "gte" => a >= b,
                "lte" => a <= b,
                _ => return Err(NodeError::new(format!("Unknown number operator: {op}"))),
            }
        }
        ("dateTime", _) => {
            let (Some(a), Some(b)) = (to_ts(left), to_ts(right)) else { return Ok(false) };
            match op {
                "equals" => a == b,
                "notEquals" => a != b,
                "after" => a > b,
                "before" => a < b,
                "afterOrEquals" => a >= b,
                "beforeOrEquals" => a <= b,
                _ => return Err(NodeError::new(format!("Unknown date operator: {op}"))),
            }
        }
        ("boolean", "true") => left.as_bool() == Some(true),
        ("boolean", "false") => left.as_bool() == Some(false),
        ("boolean", "equals") => left == right,
        ("boolean", "notEquals") => left != right,
        ("array", "contains") => left.as_array().is_some_and(|a| a.iter().any(|v| v == right || (v.as_f64().is_some() && v.as_f64() == as_f64(right)))),
        ("array", "notContains") => !left.as_array().is_some_and(|a| a.contains(right)),
        ("array", "lengthEquals") => left.as_array().map(|a| a.len() as f64) == as_f64(right),
        ("array", "lengthNotEquals") => left.as_array().map(|a| a.len() as f64) != as_f64(right),
        ("array", "lengthGt") => left.as_array().is_some_and(|a| (a.len() as f64) > as_f64(right).unwrap_or(f64::NAN)),
        ("array", "lengthLt") => left.as_array().is_some_and(|a| (a.len() as f64) < as_f64(right).unwrap_or(f64::NAN)),
        ("array", "lengthGte") => left.as_array().is_some_and(|a| (a.len() as f64) >= as_f64(right).unwrap_or(f64::NAN)),
        ("array", "lengthLte") => left.as_array().is_some_and(|a| (a.len() as f64) <= as_f64(right).unwrap_or(f64::NAN)),
        (_, "equals") => left == right,
        (_, "notEquals") => left != right,
        _ => return Err(NodeError::new(format!("Unknown operator: {ty}.{op}"))),
    })
}

/// Evaluates the filter parameter at `path` (e.g. `conditions`) for one item.
pub fn evaluate_conditions(ctx: &ExecCtx<'_>, path: &str, item: usize, loose_override: bool) -> NodeResult<bool> {
    let raw = ctx.raw_param(path).cloned().unwrap_or(Value::Null);
    let options = &raw["options"];
    let case_sensitive = options["caseSensitive"].as_bool().unwrap_or(true);
    let strict = !loose_override && options["typeValidation"].as_str().unwrap_or("strict") != "loose";
    let combinator = raw["combinator"].as_str().unwrap_or("and");
    let conditions = raw["conditions"].as_array().cloned().unwrap_or_default();
    let mut results = Vec::with_capacity(conditions.len());
    for (c, cond) in conditions.iter().enumerate() {
        let op = &cond["operator"];
        let ty = op["type"].as_str().unwrap_or("string");
        let operation = op["operation"].as_str().unwrap_or("equals");
        let single = op["singleValue"].as_bool().unwrap_or(false) || matches!(operation, "exists" | "notExists" | "empty" | "notEmpty" | "true" | "false");
        let left = ctx.param(&format!("{path}.conditions.{c}.leftValue"), item)?;
        let right = if single { Value::Null } else { ctx.param(&format!("{path}.conditions.{c}.rightValue"), item)? };
        let unchecked = matches!(operation, "exists" | "notExists" | "empty" | "notEmpty");
        let left = if unchecked { left } else { validate(left, ty, strict, "left", c, item)? };
        let right_ty = op["rightType"].as_str().unwrap_or(ty);
        let right = if single || right_ty == "any" { right } else { validate(right, right_ty, strict, "right", c, item)? };
        results.push(compare(ty, operation, &left, &right, case_sensitive).map_err(|e| e.at(item))?);
    }
    Ok(if combinator == "or" { results.iter().any(|r| *r) } else { results.iter().all(|r| *r) })
}
