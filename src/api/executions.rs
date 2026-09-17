use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

pub async fn get_execution(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.storage.get_execution(id).await {
        Ok(Some(exec)) => Json(exec).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch execution");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct ListExecutionsQuery {
    limit: Option<i64>,
}

/// Server-side cap on `?limit=`, independent of whatever a caller asks for,
/// so a very large or negative value can't turn this into an unbounded scan.
const MAX_EXECUTIONS_LIMIT: i64 = 200;
const DEFAULT_EXECUTIONS_LIMIT: i64 = 50;

pub async fn list_executions_for_workflow(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
    Path(workflow_id): Path<Uuid>,
    Query(query): Query<ListExecutionsQuery>,
) -> impl IntoResponse {
    match state.storage.get_workflow(workflow_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for execution history");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_EXECUTIONS_LIMIT)
        .clamp(1, MAX_EXECUTIONS_LIMIT);
    match state.storage.list_executions_for_workflow(workflow_id, limit).await {
        Ok(executions) => Json(executions).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list executions for workflow");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(serde::Deserialize)]
struct AuthFrame {
    token: String,
}

pub async fn subscribe_executions(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(workflow_id): Path<Uuid>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_execution_socket(socket, state, workflow_id))
}

async fn handle_execution_socket(mut socket: WebSocket, state: AppState, workflow_id: Uuid) {
    let authed = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await,
        Ok(Some(Ok(Message::Text(ref text))))
            if serde_json::from_str::<AuthFrame>(text)
                .ok()
                .and_then(|frame| crate::auth::verify_token(&frame.token, &state.jwt_secret).ok())
                .is_some()
    );
    if !authed {
        let _ = socket.close().await;
        return;
    }

    let mut receiver = state.execution_events.subscribe();
    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(event) if event.workflow_id == workflow_id => {
                        let Ok(json) = serde_json::to_string(&event) else { continue };
                        if socket.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                if incoming.is_none() {
                    break;
                }
            }
        }
    }
}
