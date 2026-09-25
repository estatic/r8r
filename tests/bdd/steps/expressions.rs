//! Expression evaluation, observed black-box through a Set node.
//!
//! A single-segment expression `={{ X }}` is evaluated in Set's raw JSON mode
//! as `={{ ({ "result": (X) }) }}`, so the value keeps its native type
//! (number, array, object...). Any other parameter string (mixed literal and
//! `{{ }}` segments, or no `=` prefix) is assigned as a string field, which is
//! what n8n produces for templates.

use super::docstring;
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::support::workflow::WorkflowSpec;
use crate::world::{pretty, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::{json, Value};

#[given(expr = "the input item:")]
async fn input_item(w: &mut R8rWorld, step: &Step) {
    w.expr.input_items = Some(Value::Array(vec![parse_strict(docstring(step), "input item")]));
}

#[given(expr = "the input items:")]
async fn input_items(w: &mut R8rWorld, step: &Step) {
    w.expr.input_items = Some(parse_strict(docstring(step), "input items"));
}

#[given(expr = "the workflow timezone is {string}")]
async fn timezone(w: &mut R8rWorld, tz: String) {
    w.expr.settings.insert("timezone".into(), json!(tz));
}

#[when(expr = "I evaluate the expression {string}")]
async fn evaluate(w: &mut R8rWorld, expression: String) {
    w.expr.expression = Some(expression);
    run_expression(w).await;
}

#[when(expr = "I evaluate the expression:")]
async fn evaluate_block(w: &mut R8rWorld, step: &Step) {
    w.expr.expression = Some(docstring(step).trim_end().to_string());
    run_expression(w).await;
}

fn single_segment(expression: &str) -> Option<&str> {
    let inner = expression.strip_prefix("={{")?.strip_suffix("}}")?;
    (!inner.contains("{{") && !inner.contains("}}")).then_some(inner)
}

async fn run_expression(w: &mut R8rWorld) {
    let expression = w.expr.expression.clone().unwrap();
    let parameters = match single_segment(&expression) {
        Some(inner) => json!({
            "mode": "raw",
            "jsonOutput": format!("={{{{ ({{ \"result\": ({inner}) }}) }}}}"),
            "includeOtherFields": false,
            "options": {}
        }),
        None => json!({
            "mode": "manual",
            "assignments": {"assignments": [{"id": "r", "name": "result", "value": expression, "type": "string"}]},
            "includeOtherFields": false,
            "options": {}
        }),
    };
    let mut spec = WorkflowSpec::new("Expression under test");
    spec.add_nodes_from_table(&[
        vec!["name".into(), "type".into()],
        vec!["Input".into(), "manualTrigger".into()],
        vec!["Expression".into(), "set".into()],
    ]);
    spec.node_mut("Expression").insert("parameters".into(), parameters);
    spec.add_connection_line("Input -> Expression");
    let items = w.expr.input_items.clone().unwrap_or_else(|| json!([{}]));
    spec.pin("Input", &items);
    for (k, v) in &w.expr.settings {
        spec.settings.insert(k.clone(), v.clone());
    }
    w.add_workflow(spec);
    super::cli::execute_current(w, super::cli::DEFAULT_TIMEOUT).await;
}

fn results(w: &R8rWorld) -> Vec<Value> {
    let status = w.run()["status"].as_str().unwrap_or("");
    if status != "success" {
        panic!(
            "expression {:?} failed to evaluate: {}",
            w.expr.expression.as_deref().unwrap_or(""),
            w.run().pointer("/data/resultData/error").map(pretty).unwrap_or_else(|| pretty(w.run()))
        );
    }
    w.node_output("Expression", 0, None)
}

#[then(regex = r"^the result is (.+)$")]
async fn result_is(w: &mut R8rWorld, expected: String) {
    let expected = parse_strict(&w.expand(&expected), "expected result");
    let items = results(w);
    let actual = items.first().and_then(|i| i.get("result")).cloned().unwrap_or(Value::Null);
    assert_matches(&expected, &actual, Mode::Exact)
        .unwrap_or_else(|e| panic!("{:?}: {e}", w.expr.expression.as_deref().unwrap_or("")));
}

#[then(expr = "the expression has no result")]
async fn result_undefined(w: &mut R8rWorld) {
    let items = results(w);
    let first = items.first().cloned().unwrap_or(Value::Null);
    assert!(first.get("result").is_none(), "expected no result, got {}", pretty(&first));
}

#[then(regex = r"^the results for each item are (.+)$")]
async fn results_each(w: &mut R8rWorld, expected: String) {
    let expected = parse_strict(&expected, "expected results");
    let actual: Vec<Value> = results(w).iter().map(|i| i.get("result").cloned().unwrap_or(Value::Null)).collect();
    assert_matches(&expected, &Value::Array(actual), Mode::Exact).unwrap_or_else(|e| panic!("{e}"));
}

#[then(expr = "the expression fails")]
async fn fails(w: &mut R8rWorld) {
    let status = w.run()["status"].as_str().unwrap_or("");
    assert_eq!(status, "error", "expected the expression to fail, got:\n{}", pretty(w.run()));
}

#[then(expr = "the expression fails with an error containing {string}")]
async fn fails_with(w: &mut R8rWorld, needle: String) {
    fails(w).await;
    let text = serde_json::to_string(w.run()).unwrap();
    assert!(text.contains(&needle), "error does not mention {needle:?}:\n{}", pretty(w.run()));
}
