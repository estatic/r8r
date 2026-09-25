//! AI cluster nodes (spec §6.8): only the observable run data is checked.

use crate::world::{pretty, R8rWorld};
use cucumber::then;
use serde_json::Value;

/// Sub-nodes (models, tools, memory) record their runs under their own name
/// with `ai_*` connection data, which the editor's AI log view reads.
#[then(expr = "the node {string} has run data on the {string} connection")]
async fn ai_run_data(w: &mut R8rWorld, node: String, connection: String) {
    let runs = w.node_runs(&node);
    let found = runs.iter().any(|r| r.pointer(&format!("/data/{connection}")).is_some());
    assert!(found, "no {connection} data in runs of \"{node}\": {}", pretty(&Value::Array(runs.clone())));
}

#[then(expr = "the node {string} recorded a token usage of {int} in total")]
async fn token_usage(w: &mut R8rWorld, node: String, total: i64) {
    let text = serde_json::to_string(w.node_runs(&node)).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let mut found = None;
    find_key(&v, "totalTokens", &mut found);
    assert_eq!(found, Some(total), "runs of \"{node}\": {}", pretty(&v));
}

fn find_key(v: &Value, key: &str, out: &mut Option<i64>) {
    match v {
        Value::Object(m) => {
            if let Some(n) = m.get(key).and_then(Value::as_i64) {
                *out = Some(out.unwrap_or(0) + n);
            }
            m.values().for_each(|c| find_key(c, key, out));
        }
        Value::Array(a) => a.iter().for_each(|c| find_key(c, key, out)),
        _ => {}
    }
}
