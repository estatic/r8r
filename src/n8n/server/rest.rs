//! The editor's REST API, `/rest` (spec §6.9): what the pinned n8n editor
//! build calls. Responses are wrapped in `{ "data": ... }` and need an
//! `n8n-auth` session.

use super::auth::{self, SessionUser};
use super::public_api::{execution_json, load_workflow, workflow_access, Access};
use super::runner::{self, RunRequest};
use super::webhooks::{registrations_for, TestRegistration};
use super::{activation, data, workflow_json, ApiError, ApiResult, N8n};
use crate::n8n::credential_types::{self, BLANK};
use crate::n8n::engine::find_start_node;
use crate::n8n::node::Mode;
use crate::n8n::node_types;
use crate::n8n::store::new_id;
use crate::n8n::store_ext::{ExecutionFilter, User};
use crate::n8n::types::status;
use crate::n8n::workflow::Workflow;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub fn router() -> Router<Arc<N8n>> {
    Router::new()
        .route("/rest/settings", get(auth::settings))
        .route("/rest/owner/setup", post(auth::owner_setup))
        .route("/rest/login", post(auth::login).get(auth::current_user))
        .route("/rest/logout", post(auth::logout))
        .route("/rest/me", get(auth::current_user))
        .route("/rest/api-keys", get(auth::list_api_keys).post(auth::create_api_key))
        .route("/rest/api-keys/:id", delete(auth::delete_api_key))
        .route("/rest/invitations", post(auth::create_invitations))
        .route("/rest/invitations/accept", post(auth::accept_with_token))
        .route("/rest/invitations/:id/accept", post(auth::accept_by_id))
        .route("/rest/users", get(auth::list_users_rest))
        .route("/rest/workflows", get(list_workflows).post(create_workflow))
        .route("/rest/workflows/:id", get(get_workflow).patch(update_workflow).delete(delete_workflow))
        .route("/rest/workflows/:id/run", post(run_workflow))
        .route("/rest/workflows/:id/activate", post(activate_workflow))
        .route("/rest/workflows/:id/deactivate", post(deactivate_workflow))
        .route("/rest/executions", get(list_executions))
        .route("/rest/executions/:id", get(get_execution).delete(delete_execution))
        .route("/rest/executions/:id/stop", post(stop_execution))
        .route("/rest/executions/:id/retry", post(retry_execution))
        .route("/rest/credentials", get(list_credentials).post(create_credential))
        .route("/rest/credentials/:id", get(get_credential).patch(update_credential).delete(delete_credential))
        .route("/rest/oauth2-credential/auth", get(oauth2_auth))
        .route("/rest/oauth2-credential/callback", get(oauth2_callback))
        .route("/rest/push", get(super::push::connect))
        .route("/types/nodes.json", get(node_types_json))
        .route("/types/credentials.json", get(credential_types_json))
        .route("/rest/node-types", post(node_types_json))
        .route("/rest/variables", get(list_variables))
        .route("/rest/tags", get(list_tags))
        .route("/rest/active-workflows", get(active_workflows))
        .route("/rest/workflows/:id/pin-data", patch(set_pin_data))
}

// ---- workflows ----------------------------------------------------------------

async fn render(n8n: &N8n, row: &crate::n8n::store_ext::WorkflowRow) -> Result<Value, ApiError> {
    let id = row.data["id"].as_str().unwrap_or_default();
    Ok(workflow_json(row, n8n.store.workflow_tags(id).await?))
}

async fn list_workflows(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser) -> ApiResult {
    let mut out = Vec::new();
    for row in n8n.store.workflow_rows().await? {
        if workflow_access(&n8n, &user, &row).await != Access::None {
            out.push(render(&n8n, &row).await?);
        }
    }
    let count = out.len();
    Ok(Json(json!({"data": out, "count": count})).into_response())
}

async fn create_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Json(body): Json<Value>) -> ApiResult {
    let mut wf = body.as_object().cloned().ok_or_else(|| ApiError::bad_request("A workflow must be a JSON object"))?;
    if !wf.get("name").is_some_and(Value::is_string) {
        return Err(ApiError::bad_request("The workflow needs a name"));
    }
    let id = new_id();
    wf.insert("id".into(), json!(id));
    wf.insert("active".into(), json!(false));
    wf.insert("versionId".into(), json!(uuid::Uuid::new_v4().to_string()));
    for (k, default) in [("nodes", json!([])), ("connections", json!({})), ("settings", json!({}))] {
        wf.entry(k).or_insert(default);
    }
    let tags = wf.remove("tags");
    n8n.store.save_workflow(&Value::Object(wf)).await?;
    n8n.store.set_workflow_owner(&id, Some(&user.id), None).await?;
    if let Some(Value::Array(tags)) = tags {
        let ids: Vec<String> = tags.iter().filter_map(|t| t.as_str().or(t["id"].as_str()).map(String::from)).collect();
        n8n.store.set_workflow_tags(&id, &ids).await?;
    }
    let row = n8n.store.workflow_row(&id).await?.expect("saved");
    Ok(data(render(&n8n, &row).await?))
}

async fn get_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Read).await?;
    Ok(data(render(&n8n, &row).await?))
}

async fn update_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Write).await?;
    let mut wf = row.data.clone();
    let patch = body.as_object().cloned().unwrap_or_default();
    let want_active = patch.get("active").and_then(Value::as_bool);
    for (k, v) in patch {
        if !["id", "active", "createdAt", "updatedAt", "tags", "versionId"].contains(&k.as_str()) {
            wf[&k] = v;
        }
    }
    wf["versionId"] = json!(uuid::Uuid::new_v4().to_string());
    if row.active && want_active != Some(false) {
        activation::register(&n8n, &wf).await?;
    }
    n8n.store.save_workflow(&wf).await?;
    if let Some(Value::Array(tags)) = body.get("tags") {
        let ids: Vec<String> = tags.iter().filter_map(|t| t.as_str().or(t["id"].as_str()).map(String::from)).collect();
        n8n.store.set_workflow_tags(&id, &ids).await?;
    }
    let mut row = n8n.store.workflow_row(&id).await?.expect("saved");
    match want_active {
        Some(true) if !row.active => row = activation::activate(&n8n, &row).await?,
        Some(false) if row.active => row = activation::deactivate(&n8n, &row).await?,
        _ => {}
    }
    Ok(data(render(&n8n, &row).await?))
}

async fn delete_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    load_workflow(&n8n, &user, &id, Access::Write).await?;
    activation::unregister(&n8n, &id);
    n8n.store.delete_workflow(&id).await?;
    Ok(data(json!(true)))
}

async fn activate_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Write).await?;
    let row = activation::activate(&n8n, &row).await?;
    Ok(data(render(&n8n, &row).await?))
}

async fn deactivate_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Write).await?;
    let row = activation::deactivate(&n8n, &row).await?;
    Ok(data(render(&n8n, &row).await?))
}

async fn set_pin_data(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Write).await?;
    let mut wf = row.data.clone();
    wf["pinData"] = body.get("pinData").cloned().unwrap_or(body);
    n8n.store.save_workflow(&wf).await?;
    Ok(data(wf["pinData"].clone()))
}

async fn active_workflows(State(n8n): State<Arc<N8n>>, SessionUser(_user): SessionUser) -> ApiResult {
    let ids: Vec<Value> = n8n.store.workflow_rows().await?.into_iter().filter(|r| r.active).map(|r| r.data["id"].clone()).collect();
    Ok(data(Value::Array(ids)))
}

/// Editor run (n8n 2.x `ManualRunDto`): a full run from a trigger, a run up
/// to a destination node, or a partial re-run reusing earlier run data.
/// Webhook and form triggers first wait for a test call.
async fn run_workflow(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, headers: HeaderMap, Json(body): Json<Value>) -> ApiResult {
    let row = load_workflow(&n8n, &user, &id, Access::Read).await?;
    let mut workflow = body.get("workflowData").filter(|w| w.is_object()).cloned().unwrap_or_else(|| row.data.clone());
    workflow["id"] = json!(id);
    // Pin data comes from the saved workflow when the request has none.
    let has_pins = workflow["pinData"].as_object().is_some_and(|p| !p.is_empty());
    if !has_pins {
        if let Some(p) = row.data.get("pinData").filter(|p| p.as_object().is_some_and(|o| !o.is_empty())) {
            workflow["pinData"] = p.clone();
        }
    }
    let wf = Workflow::from_json(&workflow).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let push_ref = headers.get("push-ref").and_then(|v| v.to_str().ok()).map(String::from);
    let destination = match &body["destinationNode"] {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o.get("nodeName").and_then(Value::as_str).map(String::from),
        _ => None,
    };
    let run_data = body.get("runData").and_then(Value::as_object).filter(|r| !r.is_empty()).cloned();
    let mut req = RunRequest::new(workflow.clone(), Mode::Manual);
    req.use_pin_data = true;
    req.push_ref = push_ref.clone();
    req.destination_node = destination.clone();
    if let Some(run_data) = run_data {
        let dirty: Vec<String> = body["dirtyNodeNames"].as_array().into_iter().flatten().filter_map(|d| d.as_str().map(String::from)).collect();
        let start = dirty.into_iter().find(|d| wf.node(d).is_some()).or_else(|| destination.clone());
        req.start_node = start;
        req.previous_run_data = Some(run_data);
    } else {
        let trigger = body.pointer("/triggerToStartFrom/name").and_then(Value::as_str).map(String::from);
        let start = trigger.or_else(|| find_start_node(&wf, &n8n.registry));
        if let Some(start) = &start {
            let node = wf.node(start).ok_or_else(|| ApiError::bad_request(format!("The node \"{start}\" does not exist")))?;
            let listens = ["n8n-nodes-base.webhook", "n8n-nodes-base.formTrigger"].contains(&node.node_type.as_str());
            if listens && !wf.pin_data.contains_key(start) {
                let regs: Vec<_> = registrations_for(&wf).into_iter().filter(|r| &r.node == start).collect();
                let mut tests = n8n.test_webhooks.lock().unwrap();
                tests.retain(|t| t.reg.workflow_id != id);
                for reg in regs {
                    tests.push(TestRegistration { workflow: workflow.clone(), reg, push_ref: push_ref.clone() });
                }
                return Ok(data(json!({"waitingForWebhook": true})));
            }
        }
        req.start_node = start;
    }
    let handle = runner::start(&n8n, req).await?;
    Ok(data(json!({"executionId": handle.execution_id.to_string()})))
}

// ---- executions ---------------------------------------------------------------

async fn visible(n8n: &N8n, user: &User, workflow_id: Option<&str>) -> bool {
    if user.is_admin() {
        return true;
    }
    match workflow_id {
        Some(w) => match n8n.store.workflow_row(w).await {
            Ok(Some(row)) => workflow_access(n8n, user, &row).await != Access::None,
            _ => false,
        },
        None => false,
    }
}

async fn list_executions(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let filter: Value = q.get("filter").and_then(|f| serde_json::from_str(f).ok()).unwrap_or(json!({}));
    let statuses: Vec<String> = match &filter["status"] {
        Value::Array(a) => a.iter().filter_map(|s| s.as_str().map(String::from)).collect(),
        Value::String(s) => vec![s.clone()],
        _ => vec![],
    };
    let f = ExecutionFilter {
        workflow_id: filter["workflowId"].as_str().map(String::from),
        statuses,
        before_id: q.get("lastId").and_then(|l| l.parse().ok()),
        limit: q.get("limit").and_then(|l| l.parse().ok()).unwrap_or(100),
        include_running: true,
    };
    let mut results = Vec::new();
    for row in n8n.store.list_executions(&f).await? {
        if visible(&n8n, &user, row.workflow_id.as_deref()).await {
            let mut v = execution_json(&row, false);
            v["workflowName"] = row.workflow_data["name"].clone();
            results.push(v);
        }
    }
    let count = results.len();
    Ok(data(json!({"results": results, "count": count, "estimated": false})))
}

async fn load_execution(n8n: &N8n, user: &User, id: &str) -> Result<crate::n8n::store_ext::ExecutionRow, ApiError> {
    let id: i64 = id.parse().map_err(|_| ApiError::not_found("Not Found"))?;
    let row = n8n.store.get_execution(id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?;
    if !visible(n8n, user, row.workflow_id.as_deref()).await {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(row)
}

async fn get_execution(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_execution(&n8n, &user, &id).await?;
    Ok(data(execution_json(&row, true)))
}

async fn delete_execution(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_execution(&n8n, &user, &id).await?;
    n8n.store.delete_execution(row.id).await?;
    Ok(data(json!(true)))
}

async fn stop_execution(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    let row = load_execution(&n8n, &user, &id).await?;
    let running = n8n.running.lock().unwrap().get(&row.id).cloned();
    match running {
        Some(options) => options.cancel(),
        None if row.status == status::WAITING => {
            if n8n.store.transition_execution(row.id, status::WAITING, status::CANCELED).await? {
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                let data_ = row.data.clone().unwrap_or(json!({}));
                n8n.store.save_execution_result(row.id, status::CANCELED, &row.started_at, Some(&now), &data_, None, None).await?;
            }
        }
        None => {}
    }
    // Wait briefly for the run to wind down so the answer is final.
    for _ in 0..50 {
        if !n8n.running.lock().unwrap().contains_key(&row.id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let row = n8n.store.get_execution(row.id).await?.unwrap_or(row);
    Ok(data(json!({"mode": row.mode, "startedAt": row.started_at, "stoppedAt": row.stopped_at, "finished": row.finished, "status": row.status})))
}

async fn retry_execution(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, body: Option<Json<Value>>) -> ApiResult {
    let row = load_execution(&n8n, &user, &id).await?;
    let load = body.as_ref().and_then(|b| b["loadWorkflow"].as_bool()).unwrap_or(false);
    Ok(data(super::public_api::retry(&n8n, &row, load).await?))
}

// ---- credentials --------------------------------------------------------------

fn meta(c: &crate::n8n::store::CredentialRecord) -> Value {
    json!({"id": c.id, "name": c.name, "type": c.cred_type, "createdAt": c.created_at, "updatedAt": c.updated_at, "isManaged": false})
}

async fn can_use(n8n: &N8n, user: &User, id: &str) -> Result<bool, ApiError> {
    Ok(user.is_admin() || n8n.store.credential_owner(id).await?.as_deref() == Some(user.id.as_str()))
}

async fn list_credentials(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser) -> ApiResult {
    let mut out = Vec::new();
    for c in n8n.store.list_credentials().await? {
        if can_use(&n8n, &user, &c.id).await? {
            out.push(meta(&c));
        }
    }
    Ok(data(Value::Array(out)))
}

async fn create_credential(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Json(body): Json<Value>) -> ApiResult {
    let name = body["name"].as_str().filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::bad_request("The credential needs a name"))?;
    let cred_type = body["type"].as_str().ok_or_else(|| ApiError::bad_request("The credential needs a type"))?;
    let data_ = body.get("data").cloned().unwrap_or(json!({}));
    if let Some(t) = credential_types::get(cred_type) {
        t.validate(&data_).map_err(ApiError::bad_request)?;
    }
    let id = n8n.store.save_credential(None, name, cred_type, &data_).await?;
    n8n.store.set_credential_owner(&id, &user.id).await?;
    Ok(data(meta(&n8n.store.get_credential(&id).await?.expect("saved"))))
}

fn redact(cred_type: &str, value: &Value) -> Value {
    match credential_types::get(cred_type) {
        Some(t) => t.redact(value),
        None => Value::Object(value.as_object().into_iter().flatten().map(|(k, _)| (k.clone(), json!(BLANK))).collect()),
    }
}

async fn get_credential(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let record = n8n.store.get_credential(&id).await?.filter(|_| true).ok_or_else(|| ApiError::not_found("Not Found"))?;
    if !can_use(&n8n, &user, &id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    let mut v = meta(&record);
    if q.get("includeData").is_some_and(|v| v == "true") {
        let decrypted = n8n.store.decrypt_credential(&record).await?;
        v["data"] = redact(&record.cred_type, &decrypted);
    }
    Ok(data(v))
}

async fn update_credential(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    let record = n8n.store.get_credential(&id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?;
    if !can_use(&n8n, &user, &id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    let mut current = n8n.store.decrypt_credential(&record).await?;
    // Blanked secrets sent back unchanged keep their stored value.
    for (k, v) in body["data"].as_object().into_iter().flatten() {
        if v != &json!(BLANK) {
            current[k] = v.clone();
        }
    }
    let name = body["name"].as_str().unwrap_or(&record.name).to_string();
    n8n.store.save_credential(Some(&id), &name, &record.cred_type, &current).await?;
    Ok(data(meta(&n8n.store.get_credential(&id).await?.expect("saved"))))
}

async fn delete_credential(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path(id): Path<String>) -> ApiResult {
    if n8n.store.get_credential(&id).await?.is_none() || !can_use(&n8n, &user, &id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    n8n.store.delete_credential(&id).await?;
    Ok(data(json!(true)))
}

// ---- OAuth2 authorization code --------------------------------------------------

fn state_mac(n8n: &N8n, credential_id: &str) -> String {
    use hmac::Mac;
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(n8n.config.encryption_key.as_bytes()).expect("any key length");
    mac.update(format!("oauth2:{credential_id}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn callback_url(n8n: &N8n) -> String {
    format!("{}rest/oauth2-credential/callback", n8n.config.webhook_url)
}

fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The authorization URL the editor opens for an OAuth2 credential.
async fn oauth2_auth(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let id = q.get("id").cloned().ok_or_else(|| ApiError::bad_request("Required credential ID is missing"))?;
    let record = n8n.store.get_credential(&id).await?.ok_or_else(|| ApiError::not_found("Credential not found"))?;
    if !can_use(&n8n, &user, &id).await? {
        return Err(ApiError::not_found("Credential not found"));
    }
    let cred = n8n.store.decrypt_credential(&record).await?;
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json!({"cid": id, "token": state_mac(&n8n, &id)}).to_string());
    let mut url = format!(
        "{}{}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        cred["authUrl"].as_str().unwrap_or(""),
        if cred["authUrl"].as_str().unwrap_or("").contains('?') { "&" } else { "?" },
        query_escape(cred["clientId"].as_str().unwrap_or("")),
        query_escape(&callback_url(&n8n)),
        query_escape(cred["scope"].as_str().unwrap_or("")),
        state
    );
    if let Some(extra) = cred["authQueryParameters"].as_str().filter(|s| !s.is_empty()) {
        url.push('&');
        url.push_str(extra.trim_start_matches(['?', '&']));
    }
    Ok(data(json!(url)))
}

fn callback_error(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Html(format!("<!doctype html><html><body><h1>Error</h1><p>{message}</p></body></html>"))).into_response()
}

/// Finishes the authorization-code flow: checks `state`, exchanges the
/// code for tokens and stores them on the credential.
async fn oauth2_callback(State(n8n): State<Arc<N8n>>, Query(q): Query<HashMap<String, String>>) -> Response {
    let state: Option<Value> = q
        .get("state")
        .and_then(|s| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok());
    let Some(state) = state else { return callback_error("Invalid state format: the OAuth2 state parameter could not be decoded") };
    let cid = state["cid"].as_str().unwrap_or_default().to_string();
    if state["token"].as_str() != Some(state_mac(&n8n, &cid).as_str()) {
        return callback_error("The OAuth2 state is invalid: it was not issued for this credential");
    }
    let Some(code) = q.get("code") else { return callback_error("The OAuth2 callback carries no code (state was valid)") };
    let Ok(Some(record)) = n8n.store.get_credential(&cid).await else { return callback_error("The credential in the OAuth2 state does not exist") };
    let Ok(mut cred) = n8n.store.decrypt_credential(&record).await else { return callback_error("The credential could not be decrypted") };
    let mut form: Vec<(&str, String)> = vec![("grant_type", "authorization_code".into()), ("code", code.clone()), ("redirect_uri", callback_url(&n8n))];
    let client_id = cred["clientId"].as_str().unwrap_or("").to_string();
    let secret = cred["clientSecret"].as_str().unwrap_or("").to_string();
    let mut request = n8n.services.http.post(cred["accessTokenUrl"].as_str().unwrap_or(""));
    if cred["authentication"].as_str() == Some("body") {
        form.push(("client_id", client_id));
        form.push(("client_secret", secret));
    } else {
        request = request.basic_auth(client_id, Some(secret));
    }
    let body: String = form.iter().map(|(k, v)| format!("{k}={}", query_escape(v))).collect::<Vec<_>>().join("&");
    let resp = request.header("content-type", "application/x-www-form-urlencoded").header("accept", "application/json").body(body).send().await;
    let token: Value = match resp {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        Ok(r) => return callback_error(&format!("The token endpoint answered {}", r.status())),
        Err(e) => return callback_error(&format!("The token endpoint could not be reached: {e}")),
    };
    if !token.is_object() {
        return callback_error("The token endpoint returned no token");
    }
    cred["oauthTokenData"] = token;
    if n8n.store.update_credential_data(&cid, &cred).await.is_err() {
        return callback_error("The token could not be stored");
    }
    Html("<!doctype html><html><body><p>Got connected. The window can be closed now.</p><script>window.opener && window.opener.postMessage('success', '*'); setTimeout(() => window.close(), 500)</script></body></html>").into_response()
}

// ---- types and lookups ----------------------------------------------------------

async fn node_types_json(State(n8n): State<Arc<N8n>>) -> Response {
    Json(Value::Array(node_types::descriptions(&n8n.registry, &n8n.config.nodes_exclude))).into_response()
}

async fn credential_types_json() -> Response {
    Json(Value::Array(credential_types::all().iter().map(|t| t.description()).collect())).into_response()
}

async fn list_variables(State(n8n): State<Arc<N8n>>, SessionUser(_u): SessionUser) -> ApiResult {
    Ok(data(Value::Array(n8n.store.list_variables().await?)))
}

async fn list_tags(State(n8n): State<Arc<N8n>>, SessionUser(_u): SessionUser) -> ApiResult {
    Ok(data(Value::Array(n8n.store.list_tags().await?)))
}

