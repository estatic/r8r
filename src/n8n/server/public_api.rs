//! n8n's public API, `/api/v1` (spec §6.9): API-key authentication with
//! scopes, cursor pagination, and n8n's request validation and shapes.

use super::activation;
use super::auth::ApiUser;
use super::runner::{self, RunRequest};
use super::{workflow_json, ApiError, ApiResult, N8n};
use crate::n8n::credential_types;
use crate::n8n::store::new_id;
use crate::n8n::store_ext::{ExecutionFilter, ExecutionRow, User, WorkflowRow};
use crate::n8n::types::status;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

type Q = Query<HashMap<String, String>>;

pub fn router() -> Router<Arc<N8n>> {
    Router::new()
        .route("/api/v1/workflows", get(list_workflows).post(create_workflow))
        .route("/api/v1/workflows/:id", get(get_workflow).put(update_workflow).delete(delete_workflow))
        .route("/api/v1/workflows/:id/activate", post(activate))
        .route("/api/v1/workflows/:id/deactivate", post(deactivate))
        .route("/api/v1/workflows/:id/tags", get(get_workflow_tags).put(set_workflow_tags))
        .route("/api/v1/workflows/:id/transfer", put(transfer_workflow))
        .route("/api/v1/executions", get(list_executions))
        .route("/api/v1/executions/:id", get(get_execution).delete(delete_execution))
        .route("/api/v1/executions/:id/retry", post(retry_execution))
        .route("/api/v1/credentials", get(list_credentials).post(create_credential))
        .route("/api/v1/credentials/:id", axum::routing::delete(delete_credential))
        .route("/api/v1/credentials/schema/:type", get(credential_schema))
        .route("/api/v1/tags", get(list_tags).post(create_tag))
        .route("/api/v1/tags/:id", get(get_tag).put(update_tag).delete(delete_tag))
        .route("/api/v1/variables", get(list_variables).post(create_variable))
        .route("/api/v1/variables/:id", put(update_variable).delete(delete_variable))
        .route("/api/v1/users", get(list_users).post(create_users))
        .route("/api/v1/users/:id", get(get_user).delete(delete_user))
        .route("/api/v1/audit", post(audit))
        .route("/api/v1/projects", get(list_projects).post(create_project))
        .route("/api/v1/projects/:id", put(update_project).delete(delete_project))
        .route("/api/v1/projects/:id/users", post(add_project_users))
        .route("/api/v1/openapi.yml", get(openapi))
        .route("/api/v1/docs", get(docs))
}

fn created(v: Value) -> Response {
    (StatusCode::CREATED, Json(v)).into_response()
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn ok(v: Value) -> Response {
    Json(v).into_response()
}

fn limit_of(q: &HashMap<String, String>) -> usize {
    q.get("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(100).clamp(1, 250)
}

fn encode_cursor(n: i64) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!("{{\"last\":{n}}}"))
}

fn decode_cursor(c: &str) -> Option<i64> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(c.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["last"].as_i64()
}

/// One page of `items` from the cursor's offset.
fn paginate(items: Vec<Value>, q: &HashMap<String, String>) -> Value {
    let limit = limit_of(q);
    let offset = q.get("cursor").and_then(|c| decode_cursor(c)).unwrap_or(0).max(0) as usize;
    let total = items.len();
    let page: Vec<Value> = items.into_iter().skip(offset).take(limit).collect();
    let next = (offset + limit < total).then(|| encode_cursor((offset + limit) as i64));
    json!({"data": page, "nextCursor": next})
}

// ---- access -------------------------------------------------------------------

#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub enum Access {
    None,
    Read,
    Write,
}

/// What `user` may do with a workflow: admins everything, otherwise its
/// owner, or members of the project it was moved to (viewers read only).
pub async fn workflow_access(n8n: &N8n, user: &User, row: &WorkflowRow) -> Access {
    if user.is_admin() {
        return Access::Write;
    }
    match &row.project_id {
        Some(p) => match n8n.store.project_role(p, &user.id).await.ok().flatten().as_deref() {
            Some("project:viewer") => Access::Read,
            Some(_) => Access::Write,
            None => Access::None,
        },
        None if row.owner_id.as_deref() == Some(user.id.as_str()) => Access::Write,
        None => Access::None,
    }
}

pub async fn load_workflow(n8n: &N8n, user: &User, id: &str, need: Access) -> Result<WorkflowRow, ApiError> {
    let not_found = || ApiError::not_found("Not Found");
    let row = n8n.store.workflow_row(id).await?.ok_or_else(not_found)?;
    let access = workflow_access(n8n, user, &row).await;
    if access == Access::None {
        return Err(not_found());
    }
    if access < need {
        return Err(ApiError::forbidden());
    }
    Ok(row)
}

async fn render_workflow(n8n: &N8n, row: &WorkflowRow) -> Result<Value, ApiError> {
    let id = row.data["id"].as_str().unwrap_or_default();
    Ok(workflow_json(row, n8n.store.workflow_tags(id).await?))
}

// ---- workflows ----------------------------------------------------------------

const WRITABLE: &[&str] = &["name", "nodes", "connections", "settings", "staticData", "pinData", "meta"];
const READ_ONLY: &[&str] = &["id", "active", "createdAt", "updatedAt", "versionId", "tags", "isArchived", "shared", "triggerCount", "activeVersionId"];

/// n8n's request validation for workflow bodies.
pub fn validate_workflow_body(body: &Value) -> Result<(), ApiError> {
    let obj = body.as_object().ok_or_else(|| ApiError::bad_request("request/body must be object"))?;
    for key in obj.keys() {
        if READ_ONLY.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!("request/body/{key} is read-only")));
        }
        if !WRITABLE.contains(&key.as_str()) {
            return Err(ApiError::bad_request("request/body must NOT have additional properties"));
        }
    }
    for key in ["name", "nodes", "connections", "settings"] {
        if !obj.contains_key(key) {
            return Err(ApiError::bad_request(format!("request/body must have required property '{key}'")));
        }
    }
    if !obj["name"].is_string() {
        return Err(ApiError::bad_request("request/body/name must be string"));
    }
    let nodes = obj["nodes"].as_array().ok_or_else(|| ApiError::bad_request("request/body/nodes must be array"))?;
    for (i, n) in nodes.iter().enumerate() {
        for key in ["name", "type"] {
            if !n[key].is_string() {
                return Err(ApiError::bad_request(format!("request/body/nodes/{i} must have required property '{key}'")));
            }
        }
    }
    if !obj["connections"].is_object() {
        return Err(ApiError::bad_request("request/body/connections must be object"));
    }
    if !obj["settings"].is_object() {
        return Err(ApiError::bad_request("request/body/settings must be object"));
    }
    Ok(())
}

async fn list_workflows(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("workflow:list")?;
    let wanted_tags: Vec<String> = q.get("tags").map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let mut out = Vec::new();
    for row in n8n.store.workflow_rows().await? {
        if workflow_access(&n8n, &api.user, &row).await == Access::None {
            continue;
        }
        if let Some(active) = q.get("active") {
            if (active == "true") != row.active {
                continue;
            }
        }
        if let Some(name) = q.get("name") {
            if row.data["name"].as_str() != Some(name.as_str()) {
                continue;
            }
        }
        if let Some(project) = q.get("projectId") {
            if row.project_id.as_deref() != Some(project.as_str()) {
                continue;
            }
        }
        let mut w = render_workflow(&n8n, &row).await?;
        if !wanted_tags.is_empty() {
            let names: Vec<&str> = w["tags"].as_array().into_iter().flatten().filter_map(|t| t["name"].as_str()).collect();
            if !wanted_tags.iter().any(|t| names.contains(&t.as_str())) {
                continue;
            }
        }
        if q.get("excludePinnedData").is_some_and(|v| v == "true") {
            if let Some(o) = w.as_object_mut() {
                o.remove("pinData");
            }
        }
        out.push(w);
    }
    Ok(ok(paginate(out, &q)))
}

async fn create_workflow(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("workflow:create")?;
    validate_workflow_body(&body)?;
    let mut data = body.clone();
    let id = new_id();
    data["id"] = json!(id);
    data["active"] = json!(false);
    data["versionId"] = json!(uuid::Uuid::new_v4().to_string());
    n8n.store.save_workflow(&data).await?;
    n8n.store.set_workflow_owner(&id, Some(&api.user.id), None).await?;
    let row = n8n.store.workflow_row(&id).await?.expect("saved");
    Ok(ok(render_workflow(&n8n, &row).await?))
}

async fn get_workflow(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("workflow:read")?;
    let row = load_workflow(&n8n, &api.user, &id, Access::Read).await?;
    Ok(ok(render_workflow(&n8n, &row).await?))
}

async fn update_workflow(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("workflow:update")?;
    validate_workflow_body(&body)?;
    let row = load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let mut data = row.data.clone();
    for (k, v) in body.as_object().unwrap() {
        data[k] = v.clone();
    }
    data["versionId"] = json!(uuid::Uuid::new_v4().to_string());
    if row.active {
        activation::register(&n8n, &data).await?;
    }
    n8n.store.save_workflow(&data).await?;
    let row = n8n.store.workflow_row(&id).await?.expect("saved");
    Ok(ok(render_workflow(&n8n, &row).await?))
}

async fn delete_workflow(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("workflow:delete")?;
    let row = load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let body = render_workflow(&n8n, &row).await?;
    activation::unregister(&n8n, &id);
    n8n.store.delete_workflow(&id).await?;
    Ok(ok(body))
}

async fn activate(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("workflow:activate")?;
    let row = load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let row = activation::activate(&n8n, &row).await?;
    Ok(ok(render_workflow(&n8n, &row).await?))
}

async fn deactivate(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("workflow:deactivate")?;
    let row = load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let row = activation::deactivate(&n8n, &row).await?;
    Ok(ok(render_workflow(&n8n, &row).await?))
}

async fn get_workflow_tags(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("workflowTags:list")?;
    load_workflow(&n8n, &api.user, &id, Access::Read).await?;
    Ok(ok(Value::Array(n8n.store.workflow_tags(&id).await?)))
}

async fn set_workflow_tags(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("workflowTags:update")?;
    load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let list = body.as_array().ok_or_else(|| ApiError::bad_request("request/body must be array"))?;
    let mut ids = Vec::new();
    for t in list {
        let tag_id = t["id"].as_str().ok_or_else(|| ApiError::bad_request("request/body must have required property 'id'"))?;
        if n8n.store.get_tag(tag_id).await?.is_none() {
            return Err(ApiError::not_found(format!("Tag {tag_id} not found")));
        }
        ids.push(tag_id.to_string());
    }
    n8n.store.set_workflow_tags(&id, &ids).await?;
    Ok(ok(Value::Array(n8n.store.workflow_tags(&id).await?)))
}

async fn transfer_workflow(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("workflow:move")?;
    load_workflow(&n8n, &api.user, &id, Access::Write).await?;
    let project = body["destinationProjectId"].as_str().ok_or_else(|| ApiError::bad_request("request/body must have required property 'destinationProjectId'"))?;
    if !n8n.store.project_exists(project).await? {
        return Err(ApiError::not_found(format!("Project {project} not found")));
    }
    n8n.store.set_workflow_owner(&id, None, Some(project)).await?;
    Ok(no_content())
}

// ---- executions ---------------------------------------------------------------

pub fn execution_json(row: &ExecutionRow, include_data: bool) -> Value {
    let mut v = json!({
        "id": row.id,
        "finished": row.finished,
        "mode": row.mode,
        "retryOf": row.retry_of,
        "retrySuccessId": null,
        "startedAt": row.started_at,
        "stoppedAt": row.stopped_at,
        "workflowId": row.workflow_id,
        "waitTill": row.wait_till,
        "status": row.status,
        "customData": {},
    });
    if include_data {
        v["data"] = row.data.clone().unwrap_or(json!({"resultData": {"runData": {}}}));
        v["workflowData"] = row.workflow_data.clone();
    }
    v
}

async fn execution_visible(n8n: &N8n, user: &User, row: &ExecutionRow) -> bool {
    if user.is_admin() {
        return true;
    }
    match &row.workflow_id {
        Some(w) => match n8n.store.workflow_row(w).await {
            Ok(Some(wr)) => workflow_access(n8n, user, &wr).await != Access::None,
            _ => false,
        },
        None => false,
    }
}

async fn list_executions(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("execution:list")?;
    let limit = limit_of(&q) as i64;
    let filter = ExecutionFilter {
        workflow_id: q.get("workflowId").cloned(),
        statuses: q.get("status").map(|s| vec![s.clone()]).unwrap_or_default(),
        before_id: q.get("cursor").and_then(|c| decode_cursor(c)),
        limit,
        include_running: false,
    };
    let include = q.get("includeData").is_some_and(|v| v == "true");
    let rows = n8n.store.list_executions(&filter).await?;
    let mut out = Vec::new();
    for row in &rows {
        if execution_visible(&n8n, &api.user, row).await {
            out.push(execution_json(row, include));
        }
    }
    let next = (rows.len() as i64 == limit).then(|| rows.last().map(|r| encode_cursor(r.id))).flatten();
    Ok(ok(json!({"data": out, "nextCursor": next})))
}

async fn load_execution(n8n: &N8n, user: &User, id: &str) -> Result<ExecutionRow, ApiError> {
    let not_found = || ApiError::not_found("Not Found");
    let id: i64 = id.parse().map_err(|_| not_found())?;
    let row = n8n.store.get_execution(id).await?.ok_or_else(not_found)?;
    if !execution_visible(n8n, user, &row).await {
        return Err(not_found());
    }
    Ok(row)
}

async fn get_execution(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Query(q): Q) -> ApiResult {
    api.require("execution:read")?;
    let row = load_execution(&n8n, &api.user, &id).await?;
    Ok(ok(execution_json(&row, q.get("includeData").is_some_and(|v| v == "true"))))
}

async fn delete_execution(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("execution:delete")?;
    let row = load_execution(&n8n, &api.user, &id).await?;
    if row.status == status::RUNNING {
        return Err(ApiError::bad_request("A running execution cannot be deleted; stop it first"));
    }
    n8n.store.delete_execution(row.id).await?;
    Ok(ok(execution_json(&row, false)))
}

/// Runs a failed execution again from the node that failed, reusing the
/// data of the nodes before it.
pub async fn retry(n8n: &Arc<N8n>, row: &ExecutionRow, load_workflow: bool) -> Result<Value, ApiError> {
    if row.status == status::SUCCESS {
        return Err(ApiError::new(409, "The execution succeeded, so it cannot be retried."));
    }
    if ![status::ERROR, status::CRASHED, status::CANCELED].contains(&row.status.as_str()) {
        return Err(ApiError::new(409, format!("The execution is {}, so it cannot be retried.", row.status)));
    }
    let workflow = if load_workflow {
        match &row.workflow_id {
            Some(w) => n8n.store.workflow_row(w).await?.map(|r| r.data).unwrap_or_else(|| row.workflow_data.clone()),
            None => row.workflow_data.clone(),
        }
    } else {
        row.workflow_data.clone()
    };
    let result = row.data.as_ref().map(|d| d["resultData"].clone()).unwrap_or(Value::Null);
    let mut run_data: Map<String, Value> = result["runData"].as_object().cloned().unwrap_or_default();
    let mut req = RunRequest::new(workflow, crate::n8n::node::Mode::Retry);
    req.retry_of = Some(row.id.to_string());
    if let Some(last) = result["lastNodeExecuted"].as_str() {
        run_data.remove(last);
        if !run_data.is_empty() {
            req.start_node = Some(last.to_string());
            req.previous_run_data = Some(run_data);
        }
    }
    let handle = runner::start(n8n, req).await?;
    let id = handle.execution_id;
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(300), handle.done)
        .await
        .map_err(|_| ApiError::new(500, "The retry is still running"))?
        .map_err(|_| ApiError::new(500, "The retry was lost"))?;
    match n8n.store.get_execution(id).await? {
        Some(r) => Ok(execution_json(&r, false)),
        None => Ok(json!({"id": id, "mode": "retry", "status": outcome.status, "retryOf": row.id.to_string(), "finished": outcome.status == status::SUCCESS})),
    }
}

async fn retry_execution(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, body: Option<Json<Value>>) -> ApiResult {
    api.require("execution:retry")?;
    let row = load_execution(&n8n, &api.user, &id).await?;
    let load = body.as_ref().and_then(|b| b["loadWorkflow"].as_bool()).unwrap_or(false);
    Ok(ok(retry(&n8n, &row, load).await?))
}

// ---- credentials --------------------------------------------------------------

fn credential_meta(c: &crate::n8n::store::CredentialRecord) -> Value {
    json!({"id": c.id, "name": c.name, "type": c.cred_type, "createdAt": c.created_at, "updatedAt": c.updated_at, "isManaged": false})
}

async fn list_credentials(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("credential:list")?;
    let mut out = Vec::new();
    for c in n8n.store.list_credentials().await? {
        if api.user.is_admin() || n8n.store.credential_owner(&c.id).await?.as_deref() == Some(api.user.id.as_str()) {
            out.push(credential_meta(&c));
        }
    }
    Ok(ok(paginate(out, &q)))
}

async fn create_credential(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("credential:create")?;
    let name = body["name"].as_str().filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::bad_request("request/body must have required property 'name'"))?;
    let cred_type = body["type"].as_str().ok_or_else(|| ApiError::bad_request("request/body must have required property 'type'"))?;
    let t = credential_types::get(cred_type).ok_or_else(|| ApiError::bad_request(format!("req.body.type is not a known type: \"{cred_type}\"")))?;
    let data = body.get("data").cloned().unwrap_or(json!({}));
    t.validate(&data).map_err(ApiError::bad_request)?;
    let id = n8n.store.save_credential(None, name, cred_type, &data).await?;
    n8n.store.set_credential_owner(&id, &api.user.id).await?;
    let record = n8n.store.get_credential(&id).await?.expect("saved");
    Ok(ok(credential_meta(&record)))
}

async fn delete_credential(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("credential:delete")?;
    let record = n8n.store.get_credential(&id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?;
    if !api.user.is_admin() && n8n.store.credential_owner(&id).await?.as_deref() != Some(api.user.id.as_str()) {
        return Err(ApiError::not_found("Not Found"));
    }
    n8n.store.delete_credential(&id).await?;
    Ok(ok(credential_meta(&record)))
}

async fn credential_schema(Path(cred_type): Path<String>, _api: ApiUser) -> ApiResult {
    let t = credential_types::get(&cred_type).ok_or_else(|| ApiError::not_found(format!("Credential type \"{cred_type}\" not found")))?;
    Ok(ok(t.json_schema()))
}

// ---- tags ---------------------------------------------------------------------

async fn list_tags(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("tag:list")?;
    Ok(ok(paginate(n8n.store.list_tags().await?, &q)))
}

async fn create_tag(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("tag:create")?;
    let name = body["name"].as_str().filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::bad_request("request/body must have required property 'name'"))?;
    match n8n.store.create_tag(name.trim()).await? {
        Some(tag) => Ok(created(tag)),
        None => Err(ApiError::new(409, "Tag already exists")),
    }
}

async fn get_tag(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("tag:read")?;
    Ok(ok(n8n.store.get_tag(&id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?))
}

async fn update_tag(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("tag:update")?;
    let name = body["name"].as_str().ok_or_else(|| ApiError::bad_request("request/body must have required property 'name'"))?;
    if n8n.store.list_tags().await?.iter().any(|t| t["name"] == name && t["id"] != id.as_str()) {
        return Err(ApiError::new(409, "Tag already exists"));
    }
    if !n8n.store.rename_tag(&id, name).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(ok(n8n.store.get_tag(&id).await?.unwrap()))
}

async fn delete_tag(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("tag:delete")?;
    let tag = n8n.store.get_tag(&id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?;
    n8n.store.delete_tag(&id).await?;
    Ok(ok(tag))
}

// ---- variables ----------------------------------------------------------------

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 50 && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

async fn list_variables(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("variable:list")?;
    Ok(ok(paginate(n8n.store.list_variables().await?, &q)))
}

async fn create_variable(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("variable:create")?;
    let key = body["key"].as_str().unwrap_or_default();
    let value = body["value"].as_str().map(String::from).unwrap_or_else(|| body["value"].to_string());
    if !valid_key(key) {
        return Err(ApiError::bad_request("Variable key may only contain letters, numbers and underscores"));
    }
    match n8n.store.create_variable(key, &value).await? {
        Some(v) => Ok(created(v)),
        None => Err(ApiError::new(409, format!("A variable with the key \"{key}\" already exists"))),
    }
}

async fn update_variable(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("variable:update")?;
    let key = body["key"].as_str().unwrap_or_default();
    if !valid_key(key) {
        return Err(ApiError::bad_request("Variable key may only contain letters, numbers and underscores"));
    }
    if !n8n.store.update_variable(&id, key, body["value"].as_str().unwrap_or_default()).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(no_content())
}

async fn delete_variable(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("variable:delete")?;
    if !n8n.store.delete_variable(&id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(no_content())
}

// ---- users --------------------------------------------------------------------

fn public_user(u: &User, include_role: bool) -> Value {
    let mut v = u.to_json();
    if !include_role {
        v.as_object_mut().unwrap().remove("role");
    }
    v
}

async fn list_users(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("user:list")?;
    let include = q.get("includeRole").is_some_and(|v| v == "true");
    let users: Vec<Value> = n8n.store.list_users().await?.iter().map(|u| public_user(u, include)).collect();
    Ok(ok(paginate(users, &q)))
}

async fn get_user(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Query(q): Q) -> ApiResult {
    api.require("user:read")?;
    let user = match n8n.store.get_user(&id).await? {
        Some(u) => Some(u),
        None => n8n.store.get_user_by_email(&id).await?,
    };
    let user = user.ok_or_else(|| ApiError::not_found("Not Found"))?;
    Ok(ok(public_user(&user, q.get("includeRole").is_some_and(|v| v == "true"))))
}

async fn create_users(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("user:create")?;
    Ok(created(Value::Array(super::auth::invite(&n8n, &api.user, &body).await?)))
}

async fn delete_user(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("user:delete")?;
    let user = n8n.store.get_user(&id).await?.ok_or_else(|| ApiError::not_found("Not Found"))?;
    if user.role == "global:owner" {
        return Err(ApiError::bad_request("The owner cannot be deleted"));
    }
    n8n.store.delete_user(&id).await?;
    Ok(no_content())
}

// ---- audit, projects, docs ----------------------------------------------------

async fn audit(State(n8n): State<Arc<N8n>>, api: ApiUser) -> ApiResult {
    api.require("securityAudit:generate")?;
    let mut sections = Vec::new();
    let unused: Vec<Value> = {
        let workflows = n8n.store.workflow_rows().await?;
        let used: Vec<String> = workflows
            .iter()
            .flat_map(|w| w.data["nodes"].as_array().cloned().unwrap_or_default())
            .flat_map(|n| n["credentials"].as_object().cloned().unwrap_or_default().into_values())
            .filter_map(|c| c["id"].as_str().map(String::from))
            .collect();
        n8n.store
            .list_credentials()
            .await?
            .into_iter()
            .filter(|c| !used.contains(&c.id))
            .map(|c| json!({"kind": "credential", "id": c.id, "name": c.name}))
            .collect()
    };
    if !unused.is_empty() {
        sections.push(json!({"title": "Credentials not used in any workflow", "description": "These credentials are not used in any workflow. Keeping unused credentials increases the attack surface.", "recommendation": "Consider deleting these credentials if you no longer need them.", "location": unused}));
    }
    let mut report = json!({});
    if !sections.is_empty() {
        report["Credentials Risk Report"] = json!({"risk": "credentials", "sections": sections});
    }
    Ok(ok(report))
}

async fn list_projects(State(n8n): State<Arc<N8n>>, api: ApiUser, Query(q): Q) -> ApiResult {
    api.require("project:list")?;
    Ok(ok(paginate(n8n.store.list_projects().await?, &q)))
}

async fn create_project(State(n8n): State<Arc<N8n>>, api: ApiUser, Json(body): Json<Value>) -> ApiResult {
    api.require("project:create")?;
    let name = body["name"].as_str().filter(|n| !n.trim().is_empty()).ok_or_else(|| ApiError::bad_request("request/body must have required property 'name'"))?;
    let project = n8n.store.create_project(name, "team").await?;
    n8n.store.add_project_relation(project["id"].as_str().unwrap(), &api.user.id, "project:admin").await?;
    Ok(created(project))
}

async fn update_project(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("project:update")?;
    if !n8n.store.update_project(&id, body["name"].as_str().unwrap_or_default()).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(no_content())
}

async fn delete_project(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>) -> ApiResult {
    api.require("project:delete")?;
    if !n8n.store.delete_project(&id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    Ok(no_content())
}

async fn add_project_users(State(n8n): State<Arc<N8n>>, api: ApiUser, Path(id): Path<String>, Json(body): Json<Value>) -> ApiResult {
    api.require("project:update")?;
    if !n8n.store.project_exists(&id).await? {
        return Err(ApiError::not_found("Not Found"));
    }
    for rel in body["relations"].as_array().into_iter().flatten() {
        let user_id = rel["userId"].as_str().ok_or_else(|| ApiError::bad_request("relations need a userId"))?;
        let role = rel["role"].as_str().unwrap_or("project:viewer");
        if !["project:admin", "project:editor", "project:viewer"].contains(&role) {
            return Err(ApiError::bad_request(format!("Invalid project role: {role}")));
        }
        if n8n.store.get_user(user_id).await?.is_none() {
            return Err(ApiError::not_found(format!("User {user_id} not found")));
        }
        n8n.store.add_project_relation(&id, user_id, role).await?;
    }
    Ok(created(json!({})))
}

const OPENAPI: &str = include_str!("openapi.yml");

async fn openapi() -> Response {
    ([("content-type", "application/yaml; charset=utf-8")], OPENAPI).into_response()
}

async fn docs() -> Response {
    let html = "<!doctype html><html><head><meta charset=\"utf-8\"><title>r8r Public API</title>\
<link rel=\"stylesheet\" href=\"https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css\"></head>\
<body><div id=\"ui\"></div><script src=\"https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js\"></script>\
<script>SwaggerUIBundle({url: '/api/v1/openapi.yml', dom_id: '#ui'});</script></body></html>";
    ([("content-type", "text/html; charset=utf-8")], html).into_response()
}
