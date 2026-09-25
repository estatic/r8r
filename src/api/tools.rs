//! `/rest/tools`: the `ai.agent` tool library (spec B1 §4).

use crate::api::workflows::AuthUser;
use crate::credentials::workflows_using_tool;
use crate::domain::Tool;
use crate::state::AppState;
use crate::tools::validate_tool;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

/// A tool plus how many workflows' agents reference it.
#[derive(serde::Serialize)]
pub struct ToolListItem {
    #[serde(flatten)]
    pub tool: Tool,
    pub used_by: usize,
}

#[derive(Deserialize)]
pub struct CreateToolRequest {
    pub name: String,
    pub description: String,
    pub node_type: String,
    pub argument_schema: serde_json::Value,
    pub parameters: serde_json::Value,
}

#[derive(Deserialize)]
pub struct UpdateToolRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub node_type: Option<String>,
    pub argument_schema: Option<serde_json::Value>,
    pub parameters: Option<serde_json::Value>,
}

fn internal(e: anyhow::Error, what: &str) -> Response {
    tracing::error!(error = %e, "{what}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

async fn all_workflows(state: &AppState) -> Result<Vec<crate::domain::Workflow>, Response> {
    state.storage.list_workflows().await.map_err(|e| internal(e, "failed to list workflows for tool usage"))
}

/// Whether another tool (not `except`) already has `name`.
async fn name_taken(state: &AppState, name: &str, except: Option<Uuid>) -> Result<bool, Response> {
    let tools = state.storage.list_tools().await.map_err(|e| internal(e, "failed to list tools"))?;
    Ok(tools.iter().any(|t| t.name == name && Some(t.id) != except))
}

fn name_conflict(name: &str) -> Response {
    (StatusCode::CONFLICT, format!("a tool named {name} already exists")).into_response()
}

pub async fn create_tool(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Json(payload): Json<CreateToolRequest>,
) -> Response {
    let now = chrono::Utc::now();
    let tool = Tool {
        id: Uuid::new_v4(),
        name: payload.name.trim().to_string(),
        description: payload.description,
        node_type: payload.node_type,
        argument_schema: payload.argument_schema,
        parameters: payload.parameters,
        created_at: now,
        updated_at: now,
    };
    if let Err(msg) = validate_tool(&tool) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    match name_taken(&state, &tool.name, None).await {
        Ok(true) => return name_conflict(&tool.name),
        Ok(false) => {}
        Err(r) => return r,
    }
    if let Err(e) = state.storage.create_tool(&tool).await {
        return internal(e, "failed to create tool");
    }
    tracing::info!(tool_id = %tool.id, name = %tool.name, "tool created");
    (StatusCode::CREATED, Json(tool)).into_response()
}

pub async fn list_tools(State(state): State<AppState>, AuthUser(_user_id): AuthUser) -> Response {
    let tools = match state.storage.list_tools().await {
        Ok(t) => t,
        Err(e) => return internal(e, "failed to list tools"),
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let items: Vec<ToolListItem> = tools
        .into_iter()
        .map(|tool| ToolListItem { used_by: workflows_using_tool(&workflows, tool.id).len(), tool })
        .collect();
    Json(items).into_response()
}

pub async fn get_tool(State(state): State<AppState>, AuthUser(_user_id): AuthUser, Path(id): Path<Uuid>) -> Response {
    let tool = match state.storage.get_tool(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal(e, "failed to fetch tool"),
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(ToolListItem { used_by: workflows_using_tool(&workflows, id).len(), tool }).into_response()
}

pub async fn update_tool(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateToolRequest>,
) -> Response {
    let mut tool = match state.storage.get_tool(id).await {
        Ok(Some(t)) => t,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal(e, "failed to fetch tool for update"),
    };
    if let Some(name) = payload.name {
        tool.name = name.trim().to_string();
    }
    if let Some(description) = payload.description {
        tool.description = description;
    }
    if let Some(node_type) = payload.node_type {
        tool.node_type = node_type;
    }
    if let Some(schema) = payload.argument_schema {
        tool.argument_schema = schema;
    }
    if let Some(parameters) = payload.parameters {
        tool.parameters = parameters;
    }
    if let Err(msg) = validate_tool(&tool) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    match name_taken(&state, &tool.name, Some(id)).await {
        Ok(true) => return name_conflict(&tool.name),
        Ok(false) => {}
        Err(r) => return r,
    }
    tool.updated_at = chrono::Utc::now();
    match state.storage.update_tool(&tool).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal(e, "failed to update tool"),
    }
    tracing::info!(tool_id = %tool.id, name = %tool.name, "tool updated");
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(ToolListItem { used_by: workflows_using_tool(&workflows, id).len(), tool }).into_response()
}

pub async fn delete_tool(State(state): State<AppState>, AuthUser(_user_id): AuthUser, Path(id): Path<Uuid>) -> Response {
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let users = workflows_using_tool(&workflows, id);
    if !users.is_empty() {
        let list: Vec<serde_json::Value> =
            users.iter().map(|(wf_id, name)| serde_json::json!({"id": wf_id, "name": name})).collect();
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "tool is in use", "workflows": list}))).into_response();
    }
    match state.storage.delete_tool(id).await {
        Ok(true) => {
            tracing::info!(tool_id = %id, "tool deleted");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal(e, "failed to delete tool"),
    }
}
