use crate::domain::{Connection, ExecutionMode, NodeInstance, Workflow};
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

#[derive(Deserialize)]
pub struct UpdateWorkflowRequest {
    pub name: String,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
}

/// Range-checks every node's `settings` (Plan 8.5). Returns the first
/// violation as a user-facing message naming the node and field.
fn validate_nodes(nodes: &[NodeInstance]) -> Result<(), String> {
    for node in nodes {
        if let Some(retry) = &node.settings.retry {
            if !(2..=10).contains(&retry.max_tries) {
                return Err(format!("node {}: retry.max_tries must be between 2 and 10", node.id));
            }
            if retry.wait_ms > 60_000 {
                return Err(format!("node {}: retry.wait_ms must be between 0 and 60000", node.id));
            }
        }
        if let Some(timeout) = node.settings.timeout_ms {
            if !(1..=3_600_000).contains(&timeout) {
                return Err(format!("node {}: timeout_ms must be between 1 and 3600000", node.id));
            }
        }
    }
    Ok(())
}

pub async fn create_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Json(payload): Json<CreateWorkflowRequest>,
) -> impl IntoResponse {
    if let Err(msg) = validate_nodes(&payload.nodes) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
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

    let activating = payload.active;

    if activating {
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

        // The DB write failed, so `workflow.active` in storage still holds the
        // OLD value, but we've already mutated in-memory trigger state above.
        // Run the compensating action so trigger-registry state never diverges
        // from what's actually persisted, once this request is done.
        if activating {
            // We just registered a cron job; unregister it since the flip to
            // active=true never actually took effect in storage.
            crate::triggers::deactivate_workflow_triggers(&state, workflow.id).await;
        } else if let Err(e2) = crate::triggers::activate_workflow_triggers(&state, &workflow).await {
            // We just unregistered the cron job; re-register it, best-effort,
            // since the flip to active=false never actually took effect in
            // storage. If this also fails, there's nothing more we can safely
            // retry here — log it and still return the original 500.
            tracing::error!(
                error = %e2,
                workflow_id = %workflow.id,
                "failed to re-activate workflow triggers while compensating for a storage failure"
            );
        }

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

    let credentials = match crate::credentials::resolve_credentials_for_workflow(state.storage.as_ref(), &workflow).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "failed to resolve workflow credentials");
            return (StatusCode::BAD_REQUEST, format!("credential resolution failed: {e}")).into_response();
        }
    };

    // The run is a detached background task (Plan 8.7): respond at once;
    // the editor follows progress over the WebSocket.
    match crate::execution_runner::start_execution(
        state.storage.clone(),
        state.execution_events.clone(),
        state.registry.clone(),
        workflow,
        ExecutionMode::Manual,
        None,
        credentials,
    )
    .await
    {
        Ok((execution, _handle)) => (StatusCode::ACCEPTED, Json(execution)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to persist new execution");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Deliberately does not touch `active` or trigger activation state — that
/// stays the job of `PATCH .../active`. This is safe even for a currently
/// active workflow: every trigger implementation in this codebase
/// (`fire_schedule`, `handle_webhook`, `poll_telegram_updates`) already
/// re-fetches the workflow fresh on each firing, so an edit here takes
/// effect on the trigger's next firing with no special-casing needed.
pub async fn update_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateWorkflowRequest>,
) -> impl IntoResponse {
    if let Err(msg) = validate_nodes(&payload.nodes) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    let mut workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for update");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    workflow.name = payload.name;
    workflow.nodes = payload.nodes;
    workflow.connections = payload.connections;
    workflow.updated_at = chrono::Utc::now();
    match state.storage.update_workflow(&workflow).await {
        Ok(()) => Json(workflow).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to persist workflow update");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn delete_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for deletion");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // A deleted workflow's background trigger (a cron job or a Telegram
    // long-poll task) must be torn down explicitly — deleting the row
    // doesn't stop a task that's already spawned and holding no reference
    // back to storage's row-existence.
    if workflow.active {
        crate::triggers::deactivate_workflow_triggers(&state, workflow.id).await;
    }
    match state.storage.delete_workflow(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete workflow");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{NodeSettings, RetryPolicy};

    fn node_with(settings: NodeSettings) -> NodeInstance {
        NodeInstance {
            id: "n1".into(),
            node_type: "core.set".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings,
        }
    }

    fn retry(max_tries: u32, wait_ms: u64) -> NodeSettings {
        NodeSettings { retry: Some(RetryPolicy { max_tries, wait_ms }), ..Default::default() }
    }

    fn timeout(ms: u64) -> NodeSettings {
        NodeSettings { timeout_ms: Some(ms), ..Default::default() }
    }

    #[test]
    fn accepts_defaults_and_boundaries() {
        for s in [NodeSettings::default(), retry(2, 0), retry(10, 60_000), timeout(1), timeout(3_600_000)] {
            assert_eq!(validate_nodes(&[node_with(s.clone())]), Ok(()), "{s:?}");
        }
    }

    #[test]
    fn rejects_out_of_range_values_naming_node_and_field() {
        let cases = [
            (retry(1, 0), "node n1: retry.max_tries must be between 2 and 10"),
            (retry(11, 0), "node n1: retry.max_tries must be between 2 and 10"),
            (retry(3, 60_001), "node n1: retry.wait_ms must be between 0 and 60000"),
            (timeout(0), "node n1: timeout_ms must be between 1 and 3600000"),
            (timeout(3_600_001), "node n1: timeout_ms must be between 1 and 3600000"),
        ];
        for (s, expected) in cases {
            assert_eq!(validate_nodes(&[node_with(s)]), Err(expected.to_string()));
        }
    }
}
