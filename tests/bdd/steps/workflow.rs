//! Defining workflows in n8n's JSON model.

use super::{docstring, table};
use crate::support::json::{parse_loose, parse_strict};
use crate::support::workflow::{set_node_parameters, WorkflowSpec};
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::given;
use serde_json::{json, Value};

fn remember_webhook_ids(w: &mut R8rWorld) {
    let ids: Vec<(String, String)> = w
        .wf()
        .nodes
        .iter()
        .filter_map(|n| Some((n["name"].as_str()?.to_string(), n["webhookId"].as_str()?.to_string())))
        .collect();
    for (name, id) in ids {
        w.vars.insert(format!("WEBHOOK_ID:{name}"), id);
    }
}

#[given(expr = "a workflow named {string} with nodes:")]
async fn workflow_named(w: &mut R8rWorld, name: String, step: &Step) {
    let mut spec = WorkflowSpec::new(&name);
    spec.add_nodes_from_table(table(step));
    w.add_workflow(spec);
    remember_webhook_ids(w);
}

#[given(expr = "a workflow with nodes:")]
async fn workflow_unnamed(w: &mut R8rWorld, step: &Step) {
    workflow_named(w, "BDD workflow".into(), step).await;
}

#[given(expr = "the workflow JSON:")]
async fn workflow_raw(w: &mut R8rWorld, step: &Step) {
    let text = docstring(step).to_string();
    let parsed = parse_strict(&text, "workflow JSON");
    let mut spec = WorkflowSpec::new(parsed["name"].as_str().unwrap_or("Raw workflow"));
    spec.raw = Some(text);
    w.add_workflow(spec);
}

#[given(expr = "I edit the workflow {string}")]
async fn edit_workflow(w: &mut R8rWorld, name: String) {
    w.wf_named(&name);
}

/// One connection per line (`A -> B`, `If:1 -> No`,
/// `Model -[ai_languageModel]-> Agent`), or a table with columns
/// from | to | output | input | type.
#[given(expr = "the connections:")]
async fn connections(w: &mut R8rWorld, step: &Step) {
    if let Some(doc) = &step.docstring {
        for line in doc.lines() {
            w.wf().add_connection_line(line);
        }
        return;
    }
    let rows = table(step);
    let header = rows[0].clone();
    for row in &rows[1..] {
        let cell = |name: &str| header.iter().position(|h| h == name).map(|i| row[i].trim().to_string()).filter(|s| !s.is_empty());
        let from = cell("from").expect("from column");
        let to = cell("to").expect("to column");
        let output = cell("output").map(|s| s.parse().unwrap()).unwrap_or(0);
        let input = cell("input").map(|s| s.parse().unwrap()).unwrap_or(0);
        let ty = cell("type").unwrap_or_else(|| "main".into());
        w.wf().connect(&from, output, &to, input, &ty);
    }
}

#[given(regex = r#"^the connections? "(.+)"$"#)]
async fn connection_inline(w: &mut R8rWorld, line: String) {
    for part in line.split("\", \"") {
        w.wf().add_connection_line(part);
    }
}

#[given(expr = "the node {string} has parameters:")]
async fn node_parameters(w: &mut R8rWorld, name: String, step: &Step) {
    let params = parse_strict(docstring(step), "parameters");
    w.wf().node_mut(&name).insert("parameters".into(), params);
}

#[given(expr = "the node {string} has properties:")]
async fn node_properties(w: &mut R8rWorld, name: String, step: &Step) {
    let props = parse_strict(docstring(step), "properties");
    let node = w.wf().node_mut(&name);
    for (k, v) in props.as_object().expect("properties must be an object") {
        node.insert(k.clone(), v.clone());
    }
    remember_webhook_ids(w);
}

#[given(regex = r#"^the node "([^"]*)" has the property "([^"]*)" set to (.+)$"#)]
async fn node_property(w: &mut R8rWorld, name: String, key: String, value: String) {
    w.wf().node_mut(&name).insert(key, parse_loose(&value));
}

#[given(expr = "the node {string} is disabled")]
async fn node_disabled(w: &mut R8rWorld, name: String) {
    w.wf().node_mut(&name).insert("disabled".into(), json!(true));
}

/// Set (Edit Fields) node that outputs only the given fields.
#[given(expr = "the node {string} sets the fields:")]
async fn node_sets(w: &mut R8rWorld, name: String, step: &Step) {
    let fields = parse_strict(docstring(step), "fields");
    let fields: Vec<(String, Value)> = fields.as_object().expect("fields object").iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    w.wf().node_mut(&name).insert("parameters".into(), set_node_parameters(&fields, false));
}

/// Set (Edit Fields) node that keeps input fields and adds these.
#[given(expr = "the node {string} adds the fields:")]
async fn node_adds(w: &mut R8rWorld, name: String, step: &Step) {
    let fields = parse_strict(docstring(step), "fields");
    let fields: Vec<(String, Value)> = fields.as_object().expect("fields object").iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    w.wf().node_mut(&name).insert("parameters".into(), set_node_parameters(&fields, true));
}

#[given(expr = "the node {string} runs the JavaScript:")]
async fn node_code_all(w: &mut R8rWorld, name: String, step: &Step) {
    let code = docstring(step).to_string();
    w.wf().node_mut(&name).insert(
        "parameters".into(),
        json!({"mode": "runOnceForAllItems", "language": "javaScript", "jsCode": code}),
    );
}

#[given(expr = "the node {string} runs the JavaScript for each item:")]
async fn node_code_each(w: &mut R8rWorld, name: String, step: &Step) {
    let code = docstring(step).to_string();
    w.wf().node_mut(&name).insert(
        "parameters".into(),
        json!({"mode": "runOnceForEachItem", "language": "javaScript", "jsCode": code}),
    );
}

#[given(expr = "the node {string} runs the Python:")]
async fn node_code_python(w: &mut R8rWorld, name: String, step: &Step) {
    let code = docstring(step).to_string();
    w.wf().node_mut(&name).insert(
        "parameters".into(),
        json!({"mode": "runOnceForAllItems", "language": "pythonNative", "pythonCode": code}),
    );
}

/// `credentials: { <type>: { id, name } }` referencing a credential created
/// earlier in the scenario (its id is resolved when the workflow is used).
#[given(expr = "the node {string} uses the {string} credential {string}")]
async fn node_credential(w: &mut R8rWorld, node: String, cred_type: String, cred_name: String) {
    let id = format!("%{{CREDENTIAL_ID:{cred_name}}}");
    w.wf().node_mut(&node).insert("credentials".into(), json!({ cred_type: {"id": id, "name": cred_name} }));
}

#[given(expr = "the workflow settings:")]
async fn workflow_settings(w: &mut R8rWorld, step: &Step) {
    let settings = parse_strict(docstring(step), "settings");
    for (k, v) in settings.as_object().expect("settings object") {
        w.wf().settings.insert(k.clone(), v.clone());
    }
}

#[given(regex = r#"^the workflow setting "([^"]*)" is (.+)$"#)]
async fn workflow_setting(w: &mut R8rWorld, key: String, value: String) {
    w.wf().settings.insert(key, parse_loose(&value));
}

#[given(expr = "the node {string} is pinned with the items:")]
async fn pin_items(w: &mut R8rWorld, node: String, step: &Step) {
    let items = parse_strict(docstring(step), "pinned items");
    w.wf().pin(&node, &items);
}

/// Pins the first node (the trigger), which is how scenarios feed input.
#[given(expr = "the trigger outputs the items:")]
async fn trigger_items(w: &mut R8rWorld, step: &Step) {
    let items = parse_strict(docstring(step), "trigger items");
    let first = w.wf().first_node_name();
    w.wf().pin(&first, &items);
}

#[given(expr = "the workflow has no {string} setting")]
async fn remove_setting(w: &mut R8rWorld, key: String) {
    w.wf().settings.remove(&key);
}
