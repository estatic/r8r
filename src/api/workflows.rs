use crate::domain::{Connection, Execution, ExecutionMode, ExecutionStatus, NodeInstance, Workflow};
use crate::state::AppState;
use axum::async_trait;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

pub struct AuthUser(pub Uuid);

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let token = header.strip_prefix("Bearer ").ok_or(StatusCode::UNAUTHORIZED)?;
        let user_id = crate::auth::verify_token(token, &state.jwt_secret)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(AuthUser(user_id))
    }
}

#[derive(Deserialize)]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
}

pub async fn create_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Json(payload): Json<CreateWorkflowRequest>,
) -> impl IntoResponse {
    let now = chrono::Utc::now();
    let workflow = Workflow {
        id: Uuid::new_v4(),
        name: payload.name,
        active: false,
        nodes: payload.nodes,
        connections: payload.connections,
        created_at: now,
        updated_at: now,
    };
    match state.storage.create_workflow(&workflow).await {
        Ok(()) => (StatusCode::CREATED, Json(workflow)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to create workflow");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn get_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => Json(wf).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn list_workflows(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
) -> impl IntoResponse {
    match state.storage.list_workflows().await {
        Ok(workflows) => Json(workflows).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list workflows");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct SetActiveRequest {
    pub active: bool,
}

pub async fn set_workflow_active(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<SetActiveRequest>,
) -> impl IntoResponse {
    let mut workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for activation");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if payload.active == workflow.active {
        return Json(workflow).into_response();
    }

    if payload.active {
        if let Err(e) = crate::triggers::activate_workflow_triggers(&state, &workflow).await {
            return (StatusCode::BAD_REQUEST, format!("failed to activate workflow: {e}")).into_response();
        }
    } else {
        crate::triggers::deactivate_workflow_triggers(&state, workflow.id).await;
    }

    workflow.active = payload.active;
    workflow.updated_at = chrono::Utc::now();
    if let Err(e) = state.storage.update_workflow(&workflow).await {
        tracing::error!(error = %e, "failed to persist workflow activation state");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(workflow).into_response()
}

pub async fn execute_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for execution");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut execution = Execution {
        id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Running,
        mode: ExecutionMode::Manual,
        node_outputs: Default::default(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    };
    if let Err(e) = state.storage.create_execution(&execution).await {
        tracing::error!(error = %e, "failed to persist new execution");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    match crate::engine::execute_workflow(&workflow, &state.registry).await {
        Ok(outputs) => {
            execution.status = ExecutionStatus::Success;
            execution.node_outputs = outputs;
        }
        Err(e) => {
            // Workflow execution failures here are almost always due to the
            // shape of the user-authored workflow (unknown node type, cycle,
            // disconnected components, a node's own validation) rather than a
            // server-side bug, so this is a warning, not an error.
            tracing::warn!(error = %e, workflow_id = %workflow.id, "workflow execution failed");
            execution.status = ExecutionStatus::Error;
        }
    }
    execution.finished_at = Some(chrono::Utc::now());
    if let Err(e) = state.storage.update_execution(&execution).await {
        tracing::error!(error = %e, "failed to persist execution result");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(execution).into_response()
}
