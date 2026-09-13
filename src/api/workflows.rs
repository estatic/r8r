use crate::domain::{Connection, NodeInstance, Workflow};
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
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
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
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_workflows(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
) -> impl IntoResponse {
    match state.storage.list_workflows().await {
        Ok(workflows) => Json(workflows).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
