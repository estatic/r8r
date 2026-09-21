use crate::api::workflows::AuthUser;
use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

pub async fn list_credential_types(State(_state): State<AppState>, AuthUser(_user_id): AuthUser) -> impl IntoResponse {
    Json(crate::credential_types::known_credential_types()).into_response()
}
