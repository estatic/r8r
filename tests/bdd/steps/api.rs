//! Deploying workflows and reading executions through the public API
//! (`/api/v1`, spec §6.9), plus editor-style manual runs through `/rest`.

use crate::world::{pretty, Auth, R8rWorld};
use cucumber::{given, then, when};
use serde_json::{json, Value};
use std::time::Duration;

const TERMINAL: &[&str] = &["success", "error", "canceled", "crashed"];

/// Calls the API as `auth` without disturbing the scenario's current auth
/// or last response.
pub async fn call(w: &mut R8rWorld, auth: Auth, method: &str, path: &str, body: Option<Value>) -> crate::world::HttpResponse {
    let saved_auth = std::mem::replace(&mut w.auth, auth);
    let saved_resp = w.response.take();
    let resp = w.request(method, path, &[], body.map(|b| b.to_string())).await;
    w.auth = saved_auth;
    w.response = saved_resp;
    resp
}

/// The API key identity to deploy with: the current one if the scenario
/// uses an API key, otherwise the owner's.
fn deployer(w: &R8rWorld) -> Auth {
    match &w.auth {
        Auth::ApiKey(u) => Auth::ApiKey(u.clone()),
        _ => Auth::ApiKey("owner".into()),
    }
}

pub fn workflow_id(w: &R8rWorld, name: &str) -> Option<String> {
    w.vars.get(&format!("WORKFLOW_ID:{name}")).cloned()
}

pub async fn create_workflow(w: &mut R8rWorld, name: Option<&str>) -> String {
    if let Some(n) = name {
        w.wf_named(n);
    }
    let spec = w.wf().clone();
    if let Some(id) = workflow_id(w, &spec.name) {
        return id;
    }
    let body = w.workflow_public_json(&spec);
    let auth = deployer(w);
    let resp = call(w, auth, "POST", "/api/v1/workflows", Some(body)).await;
    let id = serde_json::from_str::<Value>(&resp.body)
        .ok()
        .and_then(|j| j["id"].as_str().map(String::from))
        .unwrap_or_else(|| panic!("creating workflow \"{}\" failed:\n{}", spec.name, resp.describe()));
    w.vars.insert(format!("WORKFLOW_ID:{}", spec.name), id.clone());
    w.vars.insert("WORKFLOW_ID".into(), id.clone());
    id
}

#[given(expr = "the workflow is created")]
async fn created(w: &mut R8rWorld) {
    create_workflow(w, None).await;
}

#[given(expr = "the workflow {string} is created")]
async fn created_named(w: &mut R8rWorld, name: String) {
    create_workflow(w, Some(&name)).await;
}

async fn activate(w: &mut R8rWorld, name: Option<&str>) -> crate::world::HttpResponse {
    let id = create_workflow(w, name).await;
    let auth = deployer(w);
    call(w, auth, "POST", &format!("/api/v1/workflows/{id}/activate"), None).await
}

#[given(expr = "the workflow is active")]
async fn active(w: &mut R8rWorld) {
    let resp = activate(w, None).await;
    assert!((200..300).contains(&resp.status), "activation failed:\n{}\n{}", resp.describe(), w.server_log_tail());
}

#[given(expr = "the workflow {string} is active")]
async fn active_named(w: &mut R8rWorld, name: String) {
    let resp = activate(w, Some(&name)).await;
    assert!((200..300).contains(&resp.status), "activation failed:\n{}\n{}", resp.describe(), w.server_log_tail());
}

#[when(expr = "I activate the workflow")]
async fn try_activate(w: &mut R8rWorld) {
    let resp = activate(w, None).await;
    w.response = Some(resp);
}

#[when(expr = "I activate the workflow {string}")]
async fn try_activate_named(w: &mut R8rWorld, name: String) {
    let resp = activate(w, Some(&name)).await;
    w.response = Some(resp);
}

#[when(expr = "I deactivate the workflow")]
async fn deactivate(w: &mut R8rWorld) {
    let id = create_workflow(w, None).await;
    let auth = deployer(w);
    let resp = call(w, auth, "POST", &format!("/api/v1/workflows/{id}/deactivate"), None).await;
    w.response = Some(resp);
}

async fn list_executions(w: &mut R8rWorld, workflow: &str) -> Vec<Value> {
    let path = format!("/api/v1/executions?workflowId={workflow}&includeData=true&limit=250");
    let resp = call(w, Auth::ApiKey("owner".into()), "GET", &path, None).await;
    let json: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
    json["data"].as_array().cloned().unwrap_or_default()
}

/// The public API leaves out executions that are still running; the
/// editor's `/rest/executions` includes them.
async fn list_running(w: &mut R8rWorld, workflow: &str) -> Vec<Value> {
    let filter = format!("{{\"workflowId\":\"{workflow}\",\"status\":[\"running\"]}}");
    let path = format!("/rest/executions?filter={}", urlencode(&filter));
    let resp = call(w, Auth::Session("owner".into()), "GET", &path, None).await;
    let json: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
    json.pointer("/data/results").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Waits for the newest execution of `workflow` whose status satisfies
/// `accept`, stores it as the scenario's execution and returns it.
async fn wait_for_execution(w: &mut R8rWorld, workflow_name: Option<&str>, accept: &dyn Fn(&str) -> bool, what: &str) {
    let name = workflow_name.map(String::from).unwrap_or_else(|| w.wf().name.clone());
    let id = workflow_id(w, &name).unwrap_or_else(|| panic!("workflow \"{name}\" was never created on the server"));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        // The public API lists newest first.
        let mut last = list_executions(w, &id).await;
        if accept("running") {
            last.extend(list_running(w, &id).await);
        }
        if let Some(found) = last.iter().find(|e| accept(e["status"].as_str().unwrap_or(""))) {
            let exec_id = found["id"].as_str().map(String::from).unwrap_or_else(|| found["id"].to_string());
            w.vars.insert("EXECUTION_ID".into(), exec_id);
            w.run = Some(found.clone());
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!(
                "no {what} execution of \"{name}\" within 30s; executions: {}\n--- server log ---\n{}",
                pretty(&Value::Array(last)),
                w.server_log_tail()
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[given(expr = "I wait for the execution to finish")]
#[when(expr = "I wait for the execution to finish")]
#[then(expr = "I wait for the execution to finish")]
async fn wait_finished(w: &mut R8rWorld) {
    wait_for_execution(w, None, &|s| TERMINAL.contains(&s), "finished").await;
}

#[when(expr = "I wait for the execution of {string} to finish")]
#[then(expr = "I wait for the execution of {string} to finish")]
async fn wait_finished_named(w: &mut R8rWorld, name: String) {
    wait_for_execution(w, Some(&name), &|s| TERMINAL.contains(&s), "finished").await;
}

#[when(expr = "I wait for an execution with the status {string}")]
#[then(expr = "I wait for an execution with the status {string}")]
async fn wait_status(w: &mut R8rWorld, status: String) {
    let s = status.clone();
    wait_for_execution(w, None, &move |x| x == s, &status).await;
}

/// Re-reads the remembered execution (`EXECUTION_ID`) until it is finished.
#[when(expr = "I wait for that execution to finish")]
#[then(expr = "I wait for that execution to finish")]
async fn wait_that(w: &mut R8rWorld) {
    let id = w.var("EXECUTION_ID");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let resp = call(w, Auth::ApiKey("owner".into()), "GET", &format!("/api/v1/executions/{id}?includeData=true"), None).await;
        let json: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
        if TERMINAL.contains(&json["status"].as_str().unwrap_or("")) {
            w.run = Some(json);
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("execution {id} did not finish within 30s; last:\n{}", resp.describe());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[then(regex = r#"^the workflow has (\d+) executions?$"#)]
async fn execution_count(w: &mut R8rWorld, count: usize) {
    // Give asynchronous persistence a moment before counting.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    let list = list_executions(w, &id).await;
    assert_eq!(list.len(), count, "executions: {}", pretty(&Value::Array(list.clone())));
}

#[then(regex = r#"^the workflow "([^"]*)" has (\d+) executions?$"#)]
async fn execution_count_named(w: &mut R8rWorld, name: String, count: usize) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    let id = workflow_id(w, &name).expect("workflow created");
    let list = list_executions(w, &id).await;
    assert_eq!(list.len(), count, "executions: {}", pretty(&Value::Array(list.clone())));
}

#[given(expr = "within {int} seconds the workflow has at least {int} executions")]
#[then(expr = "within {int} seconds the workflow has at least {int} executions")]
async fn eventually_count(w: &mut R8rWorld, secs: u64, count: usize) {
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let list = list_executions(w, &id).await;
        if list.len() >= count {
            w.run = list.first().cloned();
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("only {} executions after {secs}s\n{}", list.len(), w.server_log_tail());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[when(expr = "I remember the executions count of the workflow")]
async fn remember_count(w: &mut R8rWorld) {
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    // Let an in-flight firing land before taking the baseline.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let n = list_executions(w, &id).await.len();
    w.vars.insert("EXECUTION_COUNT".into(), n.to_string());
}

#[then(expr = "after {int} seconds the workflow has no new executions")]
async fn no_new(w: &mut R8rWorld, secs: u64) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    let before: usize = w.var("EXECUTION_COUNT").parse().unwrap();
    let now = list_executions(w, &id).await.len();
    assert_eq!(now, before, "executions kept arriving after deactivation");
}

#[then(expr = "within {int} seconds the workflow has new executions")]
async fn has_new(w: &mut R8rWorld, secs: u64) {
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    let before: usize = w.var("EXECUTION_COUNT").parse().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        if list_executions(w, &id).await.len() > before {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("no new executions within {secs}s\n{}", w.server_log_tail());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[then(expr = "every execution of the workflow has the mode {string}")]
async fn every_mode(w: &mut R8rWorld, mode: String) {
    let name = w.wf().name.clone();
    let id = workflow_id(w, &name).expect("workflow created");
    for e in list_executions(w, &id).await {
        assert_eq!(e["mode"], mode.as_str(), "execution: {}", pretty(&e));
    }
}

/// Editor-style manual run: `POST /rest/workflows/:id/run` with the saved
/// workflow as `workflowData`; the response carries `data.executionId`.
async fn manual_run(w: &mut R8rWorld, extra: Value) {
    let id = create_workflow(w, None).await;
    let spec = w.wf().clone();
    let mut workflow = w.workflow_json(&spec);
    workflow["id"] = json!(id);
    let mut body = json!({ "workflowData": workflow });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let push_ref = w.vars.get("PUSH_REF").cloned().unwrap_or_else(|| "bdd".into());
    let saved = std::mem::replace(&mut w.auth, Auth::Session("owner".into()));
    let resp = w
        .request("POST", &format!("/rest/workflows/{id}/run"), &[("push-ref".into(), push_ref)], Some(body.to_string()))
        .await;
    w.auth = saved;
    if let Some(exec) = serde_json::from_str::<Value>(&resp.body).ok().and_then(|j| j.pointer("/data/executionId").cloned()) {
        let exec = exec.as_str().map(String::from).unwrap_or_else(|| exec.to_string());
        w.vars.insert("EXECUTION_ID".into(), exec);
    }
}

/// Full manual run from the workflow's first node (its trigger), as the
/// editor's "Execute workflow" button sends it (n8n 2.x `ManualRunDto`).
#[when(expr = "I run the workflow manually from the editor")]
async fn run_manual(w: &mut R8rWorld) {
    let trigger = w.wf().first_node_name();
    manual_run(w, json!({ "triggerToStartFrom": { "name": trigger } })).await;
}

#[when(expr = "I run the workflow manually from the editor up to the node {string}")]
async fn run_manual_to(w: &mut R8rWorld, node: String) {
    manual_run(w, json!({ "destinationNode": { "nodeName": node, "mode": "inclusive" } })).await;
}

/// Partial execution: run up to `node`, marking it dirty and reusing the
/// previous run's data for everything upstream.
#[when(expr = "I re-run the node {string} manually reusing the previous run data")]
async fn rerun_node(w: &mut R8rWorld, node: String) {
    let run_data = w.run().pointer("/data/resultData/runData").cloned().expect("a previous execution with runData");
    manual_run(
        w,
        json!({ "runData": run_data, "destinationNode": { "nodeName": node, "mode": "inclusive" }, "dirtyNodeNames": [node] }),
    )
    .await;
}

#[when(expr = "I stop that execution")]
async fn stop_execution(w: &mut R8rWorld) {
    let id = w.var("EXECUTION_ID");
    let saved = std::mem::replace(&mut w.auth, Auth::Session("owner".into()));
    w.request("POST", &format!("/rest/executions/{id}/stop"), &[], None).await;
    w.auth = saved;
}

#[then(expr = "the execution has a wait time in the future")]
async fn wait_till(w: &mut R8rWorld) {
    let till = w.run()["waitTill"].as_str().unwrap_or_else(|| panic!("no waitTill: {}", pretty(w.run()))).to_string();
    let t = chrono::DateTime::parse_from_rfc3339(&till).unwrap_or_else(|e| panic!("waitTill {till}: {e}"));
    assert!(t > chrono::Utc::now(), "waitTill {till} is not in the future");
}

/// Saves the full workflow JSON (pin data included) the way the editor does:
/// `POST /rest/workflows`, answered with `{ data: { id, ... } }`.
#[given(expr = "the workflow is saved from the editor")]
async fn saved_from_editor(w: &mut R8rWorld) {
    let spec = w.wf().clone();
    let body = w.workflow_json(&spec);
    let resp = call(w, Auth::Session("owner".into()), "POST", "/rest/workflows", Some(body)).await;
    let id = serde_json::from_str::<Value>(&resp.body)
        .ok()
        .and_then(|j| j.pointer("/data/id").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| panic!("saving workflow \"{}\" failed:\n{}", spec.name, resp.describe()));
    w.vars.insert(format!("WORKFLOW_ID:{}", spec.name), id.clone());
    w.vars.insert("WORKFLOW_ID".into(), id);
}
