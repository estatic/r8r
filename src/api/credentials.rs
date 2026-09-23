use crate::api::workflows::AuthUser;
use crate::domain::{Credential, CredentialSummary};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;
use crate::credentials::{merge_credential_data, non_secret_fields, schema_for, workflows_using_credential};

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
) -> axum::response::Response {
    let summaries = match state.storage.list_credentials().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to list credentials");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let items: Vec<CredentialListItem> = summaries
        .into_iter()
        .map(|summary| CredentialListItem { used_by: workflows_using_credential(&workflows, summary.id).len(), summary })
        .collect();
    Json(items).into_response()
}

/// A list/PATCH response item: the summary plus how many workflows use it.
#[derive(serde::Serialize)]
pub struct CredentialListItem {
    #[serde(flatten)]
    pub summary: CredentialSummary,
    pub used_by: usize,
}

/// GET-by-id response: never contains password-type values.
#[derive(serde::Serialize)]
pub struct CredentialDetail {
    #[serde(flatten)]
    pub summary: CredentialSummary,
    pub used_by: usize,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
pub struct UpdateCredentialRequest {
    pub name: Option<String>,
    pub data: Option<serde_json::Value>,
}

async fn all_workflows(state: &AppState) -> Result<Vec<crate::domain::Workflow>, axum::response::Response> {
    state.storage.list_workflows().await.map_err(|e| {
        tracing::error!(error = %e, "failed to list workflows for credential usage");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

pub async fn get_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let credential = match state.storage.get_credential(id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(CredentialDetail {
        used_by: workflows_using_credential(&workflows, id).len(),
        fields: non_secret_fields(schema_for(&credential.credential_type), &credential.data),
        summary: CredentialSummary::from(&credential),
    })
    .into_response()
}

pub async fn update_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCredentialRequest>,
) -> axum::response::Response {
    if let Some(name) = &payload.name {
        if name.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, "name must not be blank").into_response();
        }
    }
    if let Some(data) = &payload.data {
        if !data.is_object() {
            return (StatusCode::BAD_REQUEST, "data must be a JSON object").into_response();
        }
    }
    let mut credential = match state.storage.get_credential(id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch credential for update");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(name) = payload.name {
        credential.name = name.trim().to_string();
    }
    if let Some(data) = payload.data {
        credential.data = merge_credential_data(schema_for(&credential.credential_type), &credential.data, &data);
    }
    credential.updated_at = chrono::Utc::now();
    match state.storage.update_credential(&credential).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to update credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    tracing::info!(credential_id = %credential.id, name = %credential.name, "credential updated");
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(CredentialListItem {
        used_by: workflows_using_credential(&workflows, id).len(),
        summary: CredentialSummary::from(&credential),
    })
    .into_response()
}

pub async fn delete_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let users = workflows_using_credential(&workflows, id);
    if !users.is_empty() {
        let list: Vec<serde_json::Value> =
            users.iter().map(|(wf_id, name)| serde_json::json!({"id": wf_id, "name": name})).collect();
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "credential is in use", "workflows": list})),
        )
            .into_response();
    }
    match state.storage.delete_credential(id).await {
        Ok(true) => {
            tracing::info!(credential_id = %id, "credential deleted");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete credential");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
