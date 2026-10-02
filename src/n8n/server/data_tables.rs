//! Data Tables' editor REST API (spec: n8n's Data Tables feature), under
//! `/rest/projects/:projectId/data-tables`. Faithful to n8n 2.35.7's
//! `DataTableController` for routes and request/response shapes; see
//! `crate::n8n::data_table` for storage, validation and filters.
//!
//! r8r simplifies project scoping: every user gets a personal project
//! lazily (created on first use, see `Store::personal_project`) rather
//! than always having one from account creation, but once created it
//! behaves exactly like any other project for these routes.

use super::auth::SessionUser;
use super::public_api::Access;
use super::{data, ApiError, ApiResult, N8n};
use crate::n8n::data_table::{self, parse_filter, DtError};
use crate::n8n::store_ext::User;
use axum::extract::{Path, Query, State};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub fn router() -> Router<Arc<N8n>> {
    Router::new()
        .route("/rest/projects", get(list_projects))
        .route("/rest/projects/:projectId/data-tables", get(list_tables).post(create_table))
        .route("/rest/projects/:projectId/data-tables/:dataTableId", get(get_table).patch(rename_table).delete(delete_table))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/columns", get(get_columns).post(add_column))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/columns/:columnId", delete(delete_column))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/columns/:columnId/move", patch(move_column))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/columns/:columnId/rename", patch(rename_column))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/rows", get(get_rows).patch(update_rows).delete(delete_rows))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/insert", post(insert_rows))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/upsert", post(upsert_row))
        .route("/rest/projects/:projectId/data-tables/:dataTableId/clear", post(clear_table))
        .route("/rest/data-tables-global", get(list_global))
}

impl From<DtError> for ApiError {
    fn from(e: DtError) -> Self {
        ApiError::new(e.status, e.message)
    }
}

/// n8n's `/projects/:projectId/...` scoping: `None` is r8r's "no project"
/// (personal/headless) scope, used by "the project id of my personal
/// project" via `/rest/projects`'s own entry, or by callers that pass the
/// literal id of their own personal project once created.
async fn project_scope(n8n: &N8n, user: &User, project_id: &str, need: Access) -> Result<Option<String>, ApiError> {
    if !n8n.store.project_exists(project_id).await? {
        return Err(ApiError::not_found("Project not found"));
    }
    if user.is_admin() {
        return Ok(Some(project_id.to_string()));
    }
    let access = match n8n.store.project_role(project_id, &user.id).await?.as_deref() {
        Some("project:viewer") => Access::Read,
        Some(_) => Access::Write,
        None => Access::None,
    };
    if access == Access::None {
        return Err(ApiError::not_found("Project not found"));
    }
    if access < need {
        return Err(ApiError::forbidden());
    }
    Ok(Some(project_id.to_string()))
}

async fn list_projects(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser) -> ApiResult {
    let personal = n8n.store.personal_project(&user.id).await?;
    let mut out = vec![json!({"id": personal, "name": "Personal", "type": "personal"})];
    for p in n8n.store.list_projects().await? {
        if p["type"] == "team" {
            let role = n8n.store.project_role(p["id"].as_str().unwrap_or_default(), &user.id).await?;
            if user.is_admin() || role.is_some() {
                out.push(p);
            }
        }
    }
    Ok(data(Value::Array(out)))
}

// ---- tables -----------------------------------------------------------------

async fn create_table(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path(project_id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let name = body["name"].as_str().ok_or_else(|| ApiError::bad_request("Invalid data table name"))?;
    let columns: Vec<(String, String)> = body["columns"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| Some((c["name"].as_str()?.to_string(), c["type"].as_str()?.to_string())))
        .collect();
    let table = n8n.store.create_data_table(scope.as_deref(), name, &columns).await?;
    Ok(data(table))
}

async fn list_tables(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path(project_id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Read).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let filter: Value = q.get("filter").and_then(|f| serde_json::from_str(f).ok()).unwrap_or(json!({}));
    let name = filter["name"].as_str();
    let id = filter["id"].as_str();
    let skip: i64 = q.get("skip").and_then(|s| s.parse().ok()).unwrap_or(0);
    let take: i64 = q.get("take").and_then(|s| s.parse().ok()).unwrap_or(10).min(250);
    let (rows, count) = n8n.store.list_data_tables(scope.as_deref(), name, id, q.get("sortBy").map(String::as_str), skip, take).await?;
    Ok(data(json!({"data": rows, "count": count})))
}

async fn get_table(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path((project_id, table_id)): Path<(String, String)>) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Read).await?;
    let table = verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    Ok(data(table))
}

async fn project_is_personal(n8n: &N8n, user: &User, project_id: &str) -> Result<bool, ApiError> {
    Ok(n8n.store.personal_project(&user.id).await? == project_id)
}

/// Loads the table and confirms it actually belongs to `project_id`
/// (personal-project-aware): 404 otherwise, so a table cannot be reached
/// through a project the caller only happens to also belong to.
async fn verify_table_in_project(n8n: &N8n, user: &User, project_id: &str, table_id: &str) -> Result<Value, ApiError> {
    let table = n8n.store.get_data_table(table_id).await?;
    let in_scope = table["projectId"].as_str() == Some(project_id) || (table["projectId"].is_null() && project_is_personal(n8n, user, project_id).await?);
    if !in_scope {
        return Err(ApiError::not_found(format!("Data table with ID \"{table_id}\" not found")));
    }
    Ok(table)
}

/// The project id a table is actually stored under: `None` when the
/// caller's own personal project was passed (r8r stores personal-scope
/// tables with `project_id = NULL`), `Some(project_id)` for team projects.
async fn storage_scope(n8n: &N8n, user: &User, project_id: &str) -> Result<Option<String>, ApiError> {
    if project_is_personal(n8n, user, project_id).await? {
        Ok(None)
    } else {
        Ok(Some(project_id.to_string()))
    }
}

async fn rename_table(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let name = body["name"].as_str().ok_or_else(|| ApiError::bad_request("Invalid data table name"))?;
    let table = n8n.store.rename_data_table(&table_id, scope.as_deref(), name).await?;
    Ok(data(table))
}

async fn delete_table(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path((project_id, table_id)): Path<(String, String)>) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    n8n.store.delete_data_table(&table_id, scope.as_deref()).await?;
    Ok(data(json!(true)))
}

async fn clear_table(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path((project_id, table_id)): Path<(String, String)>) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    n8n.store.clear_data_table(&table_id).await?;
    Ok(data(json!(true)))
}

async fn list_global(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Query(q): Query<HashMap<String, String>>) -> ApiResult {
    let personal = n8n.store.personal_project(&user.id).await?;
    let skip: i64 = q.get("skip").and_then(|s| s.parse().ok()).unwrap_or(0);
    let take: i64 = q.get("take").and_then(|s| s.parse().ok()).unwrap_or(10).min(250);
    let mut all = Vec::new();
    let (personal_rows, _) = n8n.store.list_data_tables(Some(&personal), None, None, q.get("sortBy").map(String::as_str), 0, i64::MAX).await?;
    all.extend(personal_rows);
    if user.is_admin() {
        for p in n8n.store.list_projects().await? {
            if p["type"] == "team" {
                let (rows, _) = n8n.store.list_data_tables(p["id"].as_str(), None, None, q.get("sortBy").map(String::as_str), 0, i64::MAX).await?;
                all.extend(rows);
            }
        }
    }
    let count = all.len() as i64;
    let page: Vec<Value> = all.into_iter().skip(skip.max(0) as usize).take(take.max(0) as usize).collect();
    Ok(data(json!({"data": page, "count": count})))
}

// ---- columns ------------------------------------------------------------------

async fn get_columns(State(n8n): State<Arc<N8n>>, SessionUser(user): SessionUser, Path((project_id, table_id)): Path<(String, String)>) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Read).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let columns = n8n.store.get_data_table_columns(&table_id).await?;
    Ok(data(Value::Array(columns.iter().map(|c| c.to_json(&table_id)).collect())))
}

async fn add_column(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let name = body["name"].as_str().ok_or_else(|| ApiError::bad_request(data_table::COLUMN_ERROR_MESSAGE))?;
    let ty = body["type"].as_str().ok_or_else(|| ApiError::bad_request("A column needs a type"))?;
    let index = body["index"].as_i64();
    let col = n8n.store.add_data_table_column(&table_id, scope.as_deref(), name, ty, index).await?;
    Ok(data(col))
}

async fn delete_column(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id, column_id)): Path<(String, String, String)>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    n8n.store.delete_data_table_column(&table_id, scope.as_deref(), &column_id).await?;
    Ok(data(json!(true)))
}

async fn move_column(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id, column_id)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let target = body["targetIndex"].as_i64().ok_or_else(|| ApiError::bad_request("targetIndex is required"))?;
    let col = n8n.store.move_data_table_column(&table_id, scope.as_deref(), &column_id, target).await?;
    Ok(data(col))
}

async fn rename_column(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id, column_id)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    let scope = storage_scope(&n8n, &user, &project_id).await?;
    let name = body["name"].as_str().ok_or_else(|| ApiError::bad_request(data_table::COLUMN_ERROR_MESSAGE))?;
    let col = n8n.store.rename_data_table_column(&table_id, scope.as_deref(), &column_id, name).await?;
    Ok(data(col))
}

// ---- rows -----------------------------------------------------------------

fn parse_sort_by(s: Option<&String>) -> Option<(String, String)> {
    let s = s?;
    let (col, dir) = s.split_once(':')?;
    Some((col.to_string(), dir.to_string()))
}

async fn get_rows(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Read).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let filter_json: Value = q.get("filter").and_then(|f| serde_json::from_str(f).ok()).unwrap_or(json!({}));
    let filter = parse_filter(&filter_json)?;
    let skip: i64 = q.get("skip").and_then(|s| s.parse().ok()).unwrap_or(0);
    let take: i64 = q.get("take").and_then(|s| s.parse().ok()).unwrap_or(0);
    let sort = parse_sort_by(q.get("sortBy"));
    let (rows, count) = n8n.store.get_data_table_rows(&table_id, &filter, sort.as_ref().map(|(c, d)| (c.as_str(), d.as_str())), skip, take).await?;
    Ok(data(json!({"data": rows, "count": count})))
}

async fn insert_rows(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let rows: Vec<serde_json::Map<String, Value>> = body["data"].as_array().into_iter().flatten().filter_map(|v| v.as_object().cloned()).collect();
    let return_type = body["returnType"].as_str().unwrap_or("count");
    let result = n8n.store.insert_data_table_rows(&table_id, &rows, return_type).await?;
    Ok(data(result))
}

async fn update_rows(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let filter = parse_filter(&body["filter"])?;
    if filter.conditions.is_empty() {
        return Err(ApiError::bad_request("filter must not be empty"));
    }
    let Some(data_obj) = body["data"].as_object() else { return Err(ApiError::bad_request("data must not be empty")) };
    if data_obj.is_empty() {
        return Err(ApiError::bad_request("data must not be empty"));
    }
    let return_data = body["returnData"].as_bool().unwrap_or(false);
    let dry_run = body["dryRun"].as_bool().unwrap_or(false);
    let rows = n8n.store.update_data_table_rows(&table_id, &filter, data_obj, return_data, dry_run).await?;
    Ok(data(Value::Array(rows)))
}

async fn upsert_row(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let filter = parse_filter(&body["filter"])?;
    if filter.conditions.is_empty() {
        return Err(ApiError::bad_request("filter must not be empty"));
    }
    let Some(data_obj) = body["data"].as_object() else { return Err(ApiError::bad_request("data must not be empty")) };
    if data_obj.is_empty() {
        return Err(ApiError::bad_request("data must not be empty"));
    }
    let return_data = body["returnData"].as_bool().unwrap_or(false);
    let dry_run = body["dryRun"].as_bool().unwrap_or(false);
    let rows = n8n.store.upsert_data_table_row(&table_id, &filter, data_obj, return_data, dry_run).await?;
    Ok(data(Value::Array(rows)))
}

async fn delete_rows(
    State(n8n): State<Arc<N8n>>,
    SessionUser(user): SessionUser,
    Path((project_id, table_id)): Path<(String, String)>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    project_scope(&n8n, &user, &project_id, Access::Write).await?;
    verify_table_in_project(&n8n, &user, &project_id, &table_id).await?;
    let filter_json: Value = q.get("filter").and_then(|f| serde_json::from_str(f).ok()).ok_or_else(|| ApiError::bad_request("Filter is required for delete operations"))?;
    let filter = parse_filter(&filter_json)?;
    if filter.conditions.is_empty() {
        return Err(ApiError::bad_request("At least one filter condition is required for delete operations"));
    }
    let return_data = q.get("returnData").map(|v| v == "true").unwrap_or(false);
    let dry_run = q.get("dryRun").map(|v| v == "true").unwrap_or(false);
    let rows = n8n.store.delete_data_table_rows(&table_id, &filter, return_data, dry_run).await?;
    Ok(data(Value::Array(rows)))
}
