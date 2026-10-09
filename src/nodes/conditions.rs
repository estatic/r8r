//! Conditions for If and Filter: `conditions: {combinator, rules}`, each
//! rule `{left, operator, right}` with its values already resolved against
//! the item (the engine runs these nodes per item).
//!
//! Nodes saved before the rules form use a single resolved boolean,
//! `condition`; it is still read when `conditions` is absent.

use crate::node::NodeError;
use serde_json::Value;

/// Longest regex a rule may compile, so one can't eat the server's memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

/// Whether the node's conditions hold for the item they were resolved for.
pub fn evaluate(node: &str, parameters: &Value) -> Result<bool, NodeError> {
    let fail = |msg: String| NodeError::ExecutionFailed(format!("{node}: {msg}"));
    let Some(conditions) = parameters.get("conditions") else {
        return match parameters.get("condition") {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(fail(format!("\"condition\" must be a boolean, got: {other}"))),
        };
    };
    let any = match conditions.get("combinator").and_then(Value::as_str).unwrap_or("and") {
        "and" => false,
        "or" => true,
        other => return Err(fail(format!("unknown combinator \"{other}\" (expected \"and\" or \"or\")"))),
    };
    let rules = conditions.get("rules").and_then(Value::as_array).cloned().unwrap_or_default();
    if rules.is_empty() {
        return Err(fail("add at least one condition".into()));
    }
    for (i, rule) in rules.iter().enumerate() {
        let holds = rule_holds(rule).map_err(|e| fail(format!("condition {}: {e}", i + 1)))?;
        if holds == any {
            return Ok(any);
        }
    }
    Ok(!any)
}

fn rule_holds(rule: &Value) -> Result<bool, String> {
    let left = rule.get("left").unwrap_or(&Value::Null);
    let right = rule.get("right").unwrap_or(&Value::Null);
    let operator = rule.get("operator").and_then(Value::as_str).ok_or("it has no operator")?;
    Ok(match operator {
        "exists" => !left.is_null(),
        "notExists" => left.is_null(),
        "isEmpty" => is_empty(left),
        "isNotEmpty" => !is_empty(left),
        "isTrue" => truthy(left)?,
        "isFalse" => !truthy(left)?,
        "equals" => equal(left, right),
        "notEquals" => !equal(left, right),
        "contains" => text(left).contains(&text(right)),
        "notContains" => !text(left).contains(&text(right)),
        "startsWith" => text(left).starts_with(&text(right)),
        "endsWith" => text(left).ends_with(&text(right)),
        "matchesRegex" => {
            let re = regex::RegexBuilder::new(&text(right))
                .size_limit(REGEX_SIZE_LIMIT)
                .build()
                .map_err(|e| format!("invalid regex: {e}"))?;
            re.is_match(&text(left))
        }
        "gt" => number(left)? > number(right)?,
        "gte" => number(left)? >= number(right)?,
        "lt" => number(left)? < number(right)?,
        "lte" => number(left)? <= number(right)?,
        other => return Err(format!("unknown operator \"{other}\"")),
    })
}

/// A string as itself; anything else as JSON (`null` as "").
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn number(v: &Value) -> Result<f64, String> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| format!("{n} is not a number")),
        Value::String(s) => s.trim().parse::<f64>().map_err(|_| format!("\"{s}\" is not a number")),
        other => Err(format!("{other} is not a number")),
    }
}

/// Equal as numbers when either side is a number and both read as one;
/// otherwise as text.
fn equal(a: &Value, b: &Value) -> bool {
    if a.is_number() || b.is_number() {
        if let (Ok(x), Ok(y)) = (number(a), number(b)) {
            return x == y;
        }
    }
    if a.is_boolean() || b.is_boolean() {
        return text(a) == text(b);
    }
    match (a, b) {
        (Value::String(_), _) | (_, Value::String(_)) | (Value::Null, _) | (_, Value::Null) => text(a) == text(b),
        _ => a == b,
    }
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

fn truthy(v: &Value) -> Result<bool, String> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) if s == "true" => Ok(true),
        Value::String(s) if s == "false" => Ok(false),
        other => Err(format!("{other} is not true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check(rules: Value, combinator: &str) -> Result<bool, NodeError> {
        evaluate("core.if", &json!({"conditions": {"combinator": combinator, "rules": rules}}))
    }
    fn one(left: Value, operator: &str, right: Value) -> bool {
        check(json!([{"left": left, "operator": operator, "right": right}]), "and").unwrap()
    }

    #[test]
    fn compares_numbers_as_numbers_even_when_typed_as_text() {
        assert!(one(json!(5), "equals", json!("5")));
        assert!(one(json!(10), "gt", json!("9")));
        assert!(one(json!("2.5"), "lte", json!(2.5)));
        assert!(!one(json!(3), "lt", json!(3)));
    }

    #[test]
    fn compares_text() {
        assert!(one(json!("Hello world"), "contains", json!("world")));
        assert!(one(json!("Hello"), "startsWith", json!("He")));
        assert!(one(json!("Hello"), "endsWith", json!("lo")));
        assert!(one(json!("abc"), "notEquals", json!("abd")));
        assert!(one(json!("order-42"), "matchesRegex", json!(r"^order-\d+$")));
        assert!(one(json!(true), "equals", json!("true")));
    }

    #[test]
    fn checks_presence_emptiness_and_booleans() {
        assert!(one(json!(null), "notExists", json!(null)));
        assert!(one(json!(""), "isEmpty", json!(null)));
        assert!(one(json!([1]), "isNotEmpty", json!(null)));
        assert!(one(json!(true), "isTrue", json!(null)));
        assert!(one(json!("false"), "isFalse", json!(null)));
    }

    #[test]
    fn and_needs_every_rule_or_needs_one() {
        let rules = json!([{"left": 1, "operator": "equals", "right": 1}, {"left": 1, "operator": "equals", "right": 2}]);
        assert!(!check(rules.clone(), "and").unwrap());
        assert!(check(rules, "or").unwrap());
    }

    #[test]
    fn a_bad_rule_says_which_one_and_why() {
        let err = check(json!([{"left": 1, "operator": "equals", "right": 1}, {"left": "abc", "operator": "gt", "right": 1}]), "and").unwrap_err();
        assert!(err.to_string().contains("condition 2: \"abc\" is not a number"), "{err}");
        let err = check(json!([{"left": "a", "operator": "matchesRegex", "right": "("}]), "and").unwrap_err();
        assert!(err.to_string().contains("invalid regex"), "{err}");
        assert!(check(json!([]), "and").unwrap_err().to_string().contains("at least one condition"));
    }

    #[test]
    fn the_old_single_boolean_still_works() {
        assert!(evaluate("core.if", &json!({"condition": true})).unwrap());
        assert!(!evaluate("core.if", &json!({})).unwrap());
        assert!(evaluate("core.if", &json!({"condition": "yes"})).is_err());
    }
}
