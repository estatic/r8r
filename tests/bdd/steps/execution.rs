//! Assertions on an execution's result (`status`, `data.resultData`), shared
//! by headless (CLI) and server (API) executions.

use super::{docstring, list};
use crate::support::json::{assert_matches, lookup, parse_loose, parse_strict, Mode};
use crate::world::{pretty, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::then;
use serde_json::Value;

fn status(w: &R8rWorld) -> String {
    w.run()["status"].as_str().unwrap_or("<missing>").to_string()
}

fn error_summary(w: &R8rWorld) -> String {
    w.run().pointer("/data/resultData/error").map(pretty).unwrap_or_else(|| "no error".into())
}

#[then(expr = "the execution succeeds")]
async fn succeeds(w: &mut R8rWorld) {
    assert_eq!(status(w), "success", "execution error: {}", error_summary(w));
}

#[then(expr = "the execution fails")]
async fn fails(w: &mut R8rWorld) {
    assert_eq!(status(w), "error", "execution:\n{}", pretty(w.run()));
}

#[then(expr = "the execution status is {string}")]
async fn status_is(w: &mut R8rWorld, expected: String) {
    assert_eq!(status(w), expected, "execution error: {}", error_summary(w));
}

#[then(expr = "the execution status is one of {string}")]
async fn status_one_of(w: &mut R8rWorld, expected: String) {
    let s = status(w);
    assert!(expected.split(',').any(|e| e.trim() == s), "status {s} not in [{expected}]; error: {}", error_summary(w));
}

#[then(expr = "the execution mode is {string}")]
async fn mode_is(w: &mut R8rWorld, expected: String) {
    assert_eq!(w.run()["mode"].as_str().unwrap_or("<missing>"), expected);
}

#[then(expr = "the execution error message contains {string}")]
async fn error_contains(w: &mut R8rWorld, needle: String) {
    let msg = w.run().pointer("/data/resultData/error/message").and_then(Value::as_str).unwrap_or("").to_string();
    assert!(msg.contains(&needle), "error message {msg:?} does not contain {needle:?}\nerror: {}", error_summary(w));
}

#[then(expr = "the last node executed is {string}")]
async fn last_node(w: &mut R8rWorld, expected: String) {
    let actual = w.run().pointer("/data/resultData/lastNodeExecuted").and_then(Value::as_str).unwrap_or("<missing>");
    assert_eq!(actual, expected);
}

fn compare_output(w: &R8rWorld, node: &str, output: usize, run: Option<usize>, step: &Step, mode: Mode) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected items");
    let actual = Value::Array(w.node_output(node, output, run));
    assert_matches(&expected, &actual, mode)
        .unwrap_or_else(|e| panic!("node \"{node}\" output {output}: {e}\nactual items:\n{}", pretty(&actual)));
}

#[then(expr = "the node {string} outputs:")]
async fn outputs_exact(w: &mut R8rWorld, node: String, step: &Step) {
    compare_output(w, &node, 0, None, step, Mode::Exact);
}

#[then(expr = "the node {string} outputs items matching:")]
async fn outputs_subset(w: &mut R8rWorld, node: String, step: &Step) {
    compare_output(w, &node, 0, None, step, Mode::Subset);
}

#[then(expr = "output {int} of the node {string} is:")]
async fn output_n_exact(w: &mut R8rWorld, output: usize, node: String, step: &Step) {
    compare_output(w, &node, output, None, step, Mode::Exact);
}

#[then(expr = "output {int} of the node {string} has items matching:")]
async fn output_n_subset(w: &mut R8rWorld, output: usize, node: String, step: &Step) {
    compare_output(w, &node, output, None, step, Mode::Subset);
}

#[then(expr = "output {int} of the node {string} is empty")]
async fn output_n_empty(w: &mut R8rWorld, output: usize, node: String) {
    let items = w.node_output(&node, output, None);
    assert!(items.is_empty(), "expected no items, got {}", pretty(&Value::Array(items)));
}

#[then(expr = "run {int} of the node {string} outputs:")]
async fn run_n_outputs(w: &mut R8rWorld, run: usize, node: String, step: &Step) {
    compare_output(w, &node, 0, Some(run), step, Mode::Exact);
}

#[then(regex = r#"^the node "([^"]*)" outputs (\d+) items?$"#)]
async fn outputs_count(w: &mut R8rWorld, node: String, count: usize) {
    let items = w.node_output(&node, 0, None);
    assert_eq!(items.len(), count, "items:\n{}", pretty(&Value::Array(items.clone())));
}

#[then(regex = r#"^output (\d+) of the node "([^"]*)" has (\d+) items?$"#)]
async fn output_n_count(w: &mut R8rWorld, output: usize, node: String, count: usize) {
    let items = w.node_output(&node, output, None);
    assert_eq!(items.len(), count, "items:\n{}", pretty(&Value::Array(items.clone())));
}

#[then(regex = r#"^the field "([^"]*)" of item (\d+) from the node "([^"]*)" is (.+)$"#)]
async fn item_field(w: &mut R8rWorld, path: String, index: usize, node: String, expected: String) {
    let items = w.node_output(&node, 0, None);
    let item = items.get(index).unwrap_or_else(|| panic!("node \"{node}\" has only {} items", items.len()));
    let actual = lookup(item, &path).cloned().unwrap_or(Value::Null);
    assert_matches(&parse_loose(&w.expand(&expected)), &actual, Mode::Exact)
        .unwrap_or_else(|e| panic!("{e}\nitem: {}", pretty(item)));
}

#[then(expr = "the node {string} was not executed")]
async fn not_executed(w: &mut R8rWorld, node: String) {
    let data = w.run_data();
    assert!(!data.contains_key(&node), "node \"{node}\" has run data: {}", pretty(&data[&node]));
}

#[then(regex = r#"^the node "([^"]*)" was executed (\d+) times?$"#)]
async fn executed_times(w: &mut R8rWorld, node: String, times: usize) {
    assert_eq!(w.node_runs(&node).len(), times);
}

/// Order of node runs by `executionIndex` (n8n ≥ 1.7x), falling back to
/// `startTime`. A node that ran twice appears twice.
fn execution_order(w: &R8rWorld) -> Vec<String> {
    let mut runs: Vec<(i64, i64, String)> = Vec::new();
    for (node, tasks) in w.run_data() {
        for task in tasks.as_array().into_iter().flatten() {
            let index = task["executionIndex"].as_i64().unwrap_or(i64::MAX);
            let start = task["startTime"].as_i64().unwrap_or(i64::MAX);
            runs.push((index, start, node.clone()));
        }
    }
    runs.sort();
    runs.into_iter().map(|(_, _, n)| n).collect()
}

#[then(expr = "the nodes ran in the order {string}")]
async fn order_inline(w: &mut R8rWorld, order: String) {
    let expected: Vec<String> = order.split(',').map(|s| s.trim().to_string()).collect();
    assert_eq!(execution_order(w), expected);
}

#[then(expr = "the nodes ran in the order:")]
async fn order_block(w: &mut R8rWorld, step: &Step) {
    assert_eq!(execution_order(w), list(step, None));
}

#[then(expr = "the node {string} ran before the node {string}")]
async fn ran_before(w: &mut R8rWorld, a: String, b: String) {
    let order = execution_order(w);
    let ia = order.iter().position(|n| *n == a).unwrap_or_else(|| panic!("{a} did not run: {order:?}"));
    let ib = order.iter().position(|n| *n == b).unwrap_or_else(|| panic!("{b} did not run: {order:?}"));
    assert!(ia < ib, "order was {order:?}");
}

#[then(expr = "the node {string} failed with an error containing {string}")]
async fn node_failed(w: &mut R8rWorld, node: String, needle: String) {
    let runs = w.node_runs(&node);
    let err = runs.last().and_then(|r| r.get("error")).filter(|e| !e.is_null()).unwrap_or_else(|| {
        panic!("node \"{node}\" has no error; last run: {}", pretty(runs.last().unwrap()))
    });
    let text = serde_json::to_string(err).unwrap();
    assert!(text.contains(&needle), "error of \"{node}\" does not contain {needle:?}: {}", pretty(err));
}

#[then(expr = "every run of the node {string} records its timing, status and source")]
async fn task_data_shape(w: &mut R8rWorld, node: String) {
    for task in w.node_runs(&node) {
        let expected = serde_json::json!({
            "startTime": "$number",
            "executionTime": "$number",
            "executionStatus": "$string",
            "source": "$any",
            "data": "$any"
        });
        assert_matches(&expected, task, Mode::Subset).unwrap_or_else(|e| panic!("{e}\ntask: {}", pretty(task)));
    }
}

#[then(expr = "the node {string} received its input from {string}")]
async fn source_is(w: &mut R8rWorld, node: String, previous: String) {
    let runs = w.node_runs(&node);
    let source = &runs.last().unwrap()["source"];
    let found = source.as_array().into_iter().flatten().any(|s| s["previousNode"] == previous.as_str());
    assert!(found, "source of \"{node}\" is {}", pretty(source));
}

/// Table: item | pairedItem (JSON), for output 0 of the last run.
#[then(expr = "the items of the node {string} are paired as:")]
async fn paired_items(w: &mut R8rWorld, node: String, step: &Step) {
    let items = w.node_output_items(&node, 0, None);
    for row in &super::table(step)[1..] {
        let index: usize = row[0].trim().parse().unwrap();
        let expected = parse_strict(row[1].trim(), "pairedItem");
        let actual = items.get(index).map(|i| i["pairedItem"].clone()).unwrap_or(Value::Null);
        // n8n accepts `{item: 0}` and `[{item: 0}]` interchangeably for one source.
        let normalised = match (&expected, &actual) {
            (Value::Object(_), Value::Array(a)) if a.len() == 1 => a[0].clone(),
            _ => actual.clone(),
        };
        assert_matches(&expected, &normalised, Mode::Subset)
            .unwrap_or_else(|e| panic!("item {index} of \"{node}\": {e}\nitems: {}", pretty(&Value::Array(items.clone()))));
    }
}

#[then(expr = "the execution data does not contain {string}")]
async fn data_not_contains(w: &mut R8rWorld, needle: String) {
    let text = serde_json::to_string(w.run()).unwrap();
    assert!(!text.contains(&needle), "execution data contains {needle:?}");
    if let Some(cli) = &w.cli {
        assert!(!cli.stdout.contains(&needle) && !cli.stderr.contains(&needle), "CLI output contains {needle:?}");
    }
}

#[then(expr = "the execution result matches:")]
async fn run_matches(w: &mut R8rWorld, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected execution");
    assert_matches(&expected, w.run(), Mode::Subset).unwrap_or_else(|e| panic!("{e}\nexecution:\n{}", pretty(w.run())));
}

#[then(expr = "run {int} of the node {string} took less than {int} ms")]
async fn run_took_less(w: &mut R8rWorld, run: usize, node: String, ms: i64) {
    let runs = w.node_runs(&node);
    let t = runs.get(run).and_then(|r| r["executionTime"].as_i64()).unwrap_or(i64::MAX);
    assert!(t < ms, "run {run} of \"{node}\" took {t} ms");
}

#[then(expr = "run {int} of the node {string} took at least {int} ms")]
async fn run_took(w: &mut R8rWorld, run: usize, node: String, ms: i64) {
    let runs = w.node_runs(&node);
    let t = runs.get(run).and_then(|r| r["executionTime"].as_i64()).unwrap_or_else(|| panic!("no executionTime: {}", pretty(&Value::Array(runs.clone()))));
    assert!(t >= ms, "run {run} of \"{node}\" took {t} ms");
}
