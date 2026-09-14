use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
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
