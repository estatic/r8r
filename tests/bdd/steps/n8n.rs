//! n8n-compat driver (spec "n8n in Rust", sections 4.3, 6.1-6.5, 6.9).
//!
//! These steps speak n8n's contract, not r8r's: owner setup + `n8n-auth`
//! cookie, public API keys (`X-N8N-API-KEY`), workflow JSON with
//! name-keyed connections, manual runs via `/rest/workflows/:id/run`, and
//! results read from `data.resultData.runData`. They fail until r8n
//! implements that contract; the failure message names the missing piece.

use crate::world::AppWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::{json, Value};

const OWNER_EMAIL: &str = "owner@r8n.test";
const OWNER_PASSWORD: &str = "Owner-Password-123";

fn docstring(step: &Step) -> String {
    step.docstring.clone().expect("this step needs a \"\"\" doc string \"\"\"")
}

fn expect_status(w: &AppWorld, ok: &[u16], what: &str) {
    let status = w.last_status.map(|s| s.as_u16()).unwrap_or(0);
    assert!(ok.contains(&status), "{what}: expected HTTP {ok:?}, got {status}: {}", w.last_text);
}

// ---- instance, owner, API key -----------------------------------------

#[given("an r8n instance with an owner account")]
async fn owner_account(w: &mut AppWorld) {
    let body = json!({"email": OWNER_EMAIL, "firstName": "Owner", "lastName": "User", "password": OWNER_PASSWORD});
    w.request("POST", "/rest/owner/setup", Some(body.to_string()), &[]).await;
    expect_status(w, &[200], "owner setup (POST /rest/owner/setup)");
    assert!(w.cookie.is_some(), "owner setup must set the n8n-auth session cookie");
}

#[given("I have a public API key")]
async fn api_key(w: &mut AppWorld) {
    let body = json!({"label": "bdd", "expiresAt": null, "scopes": ["workflow:create", "workflow:read", "workflow:update", "workflow:delete", "workflow:activate", "workflow:deactivate", "execution:read", "execution:list", "credential:create", "credential:delete", "credential:read"]});
    w.request("POST", "/rest/api-keys", Some(body.to_string()), &[]).await;
    expect_status(w, &[200], "create API key (POST /rest/api-keys)");
    let key = w.last_body.pointer("/data/rawApiKey").and_then(Value::as_str).unwrap_or_else(|| panic!("no data.rawApiKey: {}", w.last_text));
    w.api_key = Some(key.to_string());
}

// ---- workflows ---------------------------------------------------------

/// Creates a workflow through the public API and remembers its id as `name`.
#[given(expr = "the n8n workflow {string}:")]
async fn given_workflow(w: &mut AppWorld, name: String, step: &Step) {
    let mut wf: Value = serde_json::from_str(&w.expand(&docstring(step))).expect("workflow doc string must be JSON");
    wf["name"] = json!(name.clone());
    if wf.get("settings").is_none() {
        wf["settings"] = json!({"executionOrder": "v1"});
    }
    w.request("POST", "/api/v1/workflows", Some(wf.to_string()), &[]).await;
    expect_status(w, &[200, 201], "create workflow (POST /api/v1/workflows)");
    let id = w.last_body["id"].as_str().unwrap_or_else(|| panic!("no workflow id: {}", w.last_text)).to_string();
    w.vars.insert(name, id);
}

#[when("I import the n8n workflow:")]
async fn import_workflow(w: &mut AppWorld, step: &Step) {
    w.request("POST", "/api/v1/workflows", Some(docstring(step)), &[]).await;
}

#[when(expr = "I export the workflow {string}")]
async fn export_workflow(w: &mut AppWorld, name: String) {
    let id = w.vars[&name].clone();
    w.request("GET", &format!("/api/v1/workflows/{id}"), None, &[]).await;
    expect_status(w, &[200], "read workflow (GET /api/v1/workflows/:id)");
}

#[given(expr = "the workflow {string} is active")]
async fn activate(w: &mut AppWorld, name: String) {
    let id = w.vars[&name].clone();
    w.request("POST", &format!("/api/v1/workflows/{id}/activate"), None, &[]).await;
    expect_status(w, &[200], "activate workflow (POST /api/v1/workflows/:id/activate)");
}

// ---- running -----------------------------------------------------------

async fn wait_for(w: &mut AppWorld, execution_id: &str) {
    for _ in 0..250 {
        w.request("GET", &format!("/api/v1/executions/{execution_id}?includeData=true"), None, &[]).await;
        expect_status(w, &[200], "read execution (GET /api/v1/executions/:id)");
        let status = w.last_body["status"].as_str().unwrap_or("");
        if !matches!(status, "new" | "running") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("execution {execution_id} did not finish within 5s");
}

#[when(expr = "I run the workflow {string} manually")]
async fn run_manually(w: &mut AppWorld, name: String) {
    let id = w.vars[&name].clone();
    w.request("GET", &format!("/api/v1/workflows/{id}"), None, &[]).await;
    expect_status(w, &[200], "read workflow before run");
    let workflow_data = w.last_body.clone();
    w.request("POST", &format!("/rest/workflows/{id}/run"), Some(json!({"workflowData": workflow_data}).to_string()), &[]).await;
    expect_status(w, &[200], "manual run (POST /rest/workflows/:id/run)");
    let exec = w.last_body.pointer("/data/executionId").and_then(Value::as_str).unwrap_or_else(|| panic!("no data.executionId: {}", w.last_text)).to_string();
    w.vars.insert("execution".into(), exec.clone());
    wait_for(w, &exec).await;
}

// ---- expressions (spec 6.4) --------------------------------------------
//
// An expression is evaluated by running Manual Trigger (pinned input) ->
// Edit Fields in raw mode with jsonOutput `={{ { result: <expr> } }}`, so
// the result keeps its native type.

async fn evaluate(w: &mut AppWorld, expression: &str, input: Value) {
    let wf = json!({
        "name": "evaluate",
        "nodes": [
            {"id": "a1", "name": "Trigger", "type": "n8n-nodes-base.manualTrigger", "typeVersion": 1, "position": [0, 0], "parameters": {}},
            {"id": "a2", "name": "Evaluate", "type": "n8n-nodes-base.set", "typeVersion": 3.4, "position": [200, 0],
             "parameters": {"mode": "raw", "jsonOutput": format!("={{{{ {{ result: {expression} }} }}}}"), "options": {}}}
        ],
        "connections": {"Trigger": {"main": [[{"node": "Evaluate", "type": "main", "index": 0}]]}},
        "pinData": {"Trigger": [{"json": input}]},
        "settings": {"executionOrder": "v1"}
    });
    w.request("POST", "/api/v1/workflows", Some(wf.to_string()), &[]).await;
    expect_status(w, &[200, 201], "create evaluation workflow");
    let id = w.last_body["id"].as_str().unwrap_or_default().to_string();
    w.vars.insert("evaluate".into(), id);
    run_manually(w, "evaluate".into()).await;
}

#[when(expr = "I evaluate the expression {string}")]
async fn eval_plain(w: &mut AppWorld, expression: String) {
    evaluate(w, &expression, json!({})).await;
}

#[when(expr = "I evaluate the expression {string} against:")]
async fn eval_against(w: &mut AppWorld, expression: String, step: &Step) {
    let input: Value = serde_json::from_str(&docstring(step)).expect("input must be JSON");
    evaluate(w, &expression, input).await;
}

fn run_data(w: &AppWorld) -> &Value {
    w.last_body.pointer("/data/resultData/runData").unwrap_or_else(|| panic!("no data.resultData.runData: {}", w.last_text))
}

fn last_run<'a>(w: &'a AppWorld, node: &str) -> &'a Value {
    run_data(w)
        .get(node)
        .and_then(Value::as_array)
        .and_then(|runs| runs.last())
        .unwrap_or_else(|| panic!("node {node:?} has no run in runData: {}", w.last_text))
}

#[then("the expression result is:")]
async fn expression_result(w: &mut AppWorld, step: &Step) {
    let expected: Value = serde_json::from_str(&docstring(step)).expect("expected result must be JSON");
    assert_eq!(w.last_body["status"], "success", "evaluation failed: {}", w.last_text);
    let actual = last_run(w, "Evaluate").pointer("/data/main/0/0/json/result").cloned().unwrap_or(Value::Null);
    assert_eq!(actual, expected, "execution: {}", w.last_text);
}

#[then(expr = "the expression result is {string}")]
async fn expression_result_str(w: &mut AppWorld, expected: String) {
    assert_eq!(w.last_body["status"], "success", "evaluation failed: {}", w.last_text);
    let actual = last_run(w, "Evaluate").pointer("/data/main/0/0/json/result").cloned().unwrap_or(Value::Null);
    let expected: Value = serde_json::from_str(&expected).unwrap_or(Value::String(expected));
    assert_eq!(actual, expected, "execution: {}", w.last_text);
}

#[then(expr = "the expression fails with an error containing {string}")]
async fn expression_fails(w: &mut AppWorld, text: String) {
    assert_eq!(w.last_body["status"], "error", "expected the evaluation to fail: {}", w.last_text);
    assert!(w.last_text.contains(&text), "{text:?} not in {}", w.last_text);
}

// ---- execution results (spec 2.4, 6.2) ---------------------------------

#[then(expr = "the execution status is {string}")]
async fn execution_status(w: &mut AppWorld, status: String) {
    assert_eq!(w.last_body["status"], json!(status), "execution: {}", w.last_text);
}

#[then(expr = "node {string} output {int} item {int} has JSON:")]
async fn node_item_json(w: &mut AppWorld, node: String, output: usize, item: usize, step: &Step) {
    let expected: Value = serde_json::from_str(&w.expand(&docstring(step))).expect("expected JSON");
    let actual = last_run(w, &node).pointer(&format!("/data/main/{output}/{item}/json")).cloned();
    assert_eq!(actual, Some(expected), "execution: {}", w.last_text);
}

#[then(expr = "node {string} produced {int} item(s) on output {int}")]
async fn node_item_count(w: &mut AppWorld, node: String, count: usize, output: usize) {
    let items = last_run(w, &node).pointer(&format!("/data/main/{output}")).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    assert_eq!(items, count, "execution: {}", w.last_text);
}

#[then(expr = "node {string} did not run")]
async fn node_did_not_run(w: &mut AppWorld, node: String) {
    assert!(run_data(w).get(&node).is_none(), "{node:?} ran: {}", w.last_text);
}

#[then(expr = "node {string} ran {int} time(s)")]
async fn node_ran_times(w: &mut AppWorld, node: String, times: usize) {
    let runs = run_data(w).get(&node).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
    assert_eq!(runs, times, "execution: {}", w.last_text);
}

#[then(expr = "node {string} failed with an error containing {string}")]
async fn node_failed(w: &mut AppWorld, node: String, text: String) {
    let message = last_run(w, &node).pointer("/error/message").and_then(Value::as_str).unwrap_or_else(|| panic!("{node:?} has no error: {}", w.last_text));
    assert!(message.contains(&text), "{text:?} not in {message:?}");
}

#[then(expr = "item {int} of node {string} is paired with item {int} of node {string}")]
async fn paired_item(w: &mut AppWorld, item: usize, node: String, source_item: usize, source_node: String) {
    let run = last_run(w, &node);
    let paired = run.pointer(&format!("/data/main/0/{item}/pairedItem")).unwrap_or_else(|| panic!("no pairedItem on {node}[{item}]: {}", w.last_text));
    let index = paired.get("item").and_then(Value::as_u64).or_else(|| paired.as_u64()).expect("pairedItem.item");
    assert_eq!(index as usize, source_item, "pairedItem: {paired}");
    let source = run.pointer("/source/0/previousNode").and_then(Value::as_str);
    assert_eq!(source, Some(source_node.as_str()), "source: {}", run);
}

/// Order by each node's first run `executionIndex` (n8n 1.x task data).
#[then("the nodes ran in this order:")]
async fn nodes_in_order(w: &mut AppWorld, step: &Step) {
    let expected: Vec<String> = step.table.as_ref().expect("a one-column table of node names").rows.iter().map(|r| r[0].clone()).collect();
    let mut actual: Vec<(u64, String)> = run_data(w)
        .as_object()
        .expect("runData object")
        .iter()
        .map(|(name, runs)| (runs.pointer("/0/executionIndex").and_then(Value::as_u64).unwrap_or(u64::MAX), name.clone()))
        .collect();
    actual.sort();
    let actual: Vec<String> = actual.into_iter().map(|(_, n)| n).collect();
    assert_eq!(actual, expected, "execution: {}", w.last_text);
}

// ---- webhooks (spec 6.3) -----------------------------------------------

#[when(regex = r#"^I call the (production|test) webhook (GET|POST|PUT|PATCH|DELETE|HEAD) "([^"]*)"$"#)]
async fn call_webhook(w: &mut AppWorld, kind: String, method: String, path: String) {
    let prefix = if kind == "production" { "webhook" } else { "webhook-test" };
    let (key, cookie, token) = (w.api_key.take(), w.cookie.take(), w.token.take());
    w.request(&method, &format!("/{prefix}/{path}"), None, &[]).await;
    (w.api_key, w.cookie, w.token) = (key, cookie, token);
}

#[when(regex = r#"^I call the (production|test) webhook (GET|POST|PUT|PATCH|DELETE) "([^"]*)" with JSON:$"#)]
async fn call_webhook_json(w: &mut AppWorld, kind: String, method: String, path: String, step: &Step) {
    let prefix = if kind == "production" { "webhook" } else { "webhook-test" };
    let (key, cookie, token) = (w.api_key.take(), w.cookie.take(), w.token.take());
    w.request(&method, &format!("/{prefix}/{path}"), Some(docstring(step)), &[]).await;
    (w.api_key, w.cookie, w.token) = (key, cookie, token);
}

#[when(regex = r#"^I call the production webhook (GET|POST) "([^"]*)" with header "([^"]*)" set to "([^"]*)"$"#)]
async fn call_webhook_header(w: &mut AppWorld, method: String, path: String, name: String, value: String) {
    let (key, cookie, token) = (w.api_key.take(), w.cookie.take(), w.token.take());
    w.request(&method, &format!("/webhook/{path}"), None, &[(name.as_str(), value)]).await;
    (w.api_key, w.cookie, w.token) = (key, cookie, token);
}

// ---- public API auth (spec 6.9) ----------------------------------------

#[given("I use no API key")]
async fn no_api_key(w: &mut AppWorld) {
    let _ = w.app().await;
    w.api_key = None;
    w.cookie = None;
}

#[given(expr = "I use the API key {string}")]
async fn use_api_key(w: &mut AppWorld, key: String) {
    let _ = w.app().await;
    w.api_key = Some(key);
    w.cookie = None;
}
