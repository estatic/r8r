use crate::api::workflows::AuthUser;
use crate::domain::{Credential, CredentialSummary};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateCredentialRequest {
    pub name: String,
    pub credential_type: String,
    pub data: serde_json::Value,
}

pub async fn create_credential(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Json(payload): Json<CreateCredentialRequest>,
) -> impl IntoResponse {
    let now = chrono::Utc::now();
    let credential = Credential {
        id: Uuid::new_v4(),
        name: payload.name,
        credential_type: payload.credential_type,
        data: payload.data,
        owner_id: user_id,
        created_at: now,
        updated_at: now,
    };
    match state.storage.create_credential(&credential).await {
        Ok(()) => (StatusCode::CREATED, Json(CredentialSummary::from(&credential))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to create credential");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn list_credentials(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
) -> impl IntoResponse {
    match state.storage.list_credentials().await {
        Ok(summaries) => Json(summaries).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list credentials");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
