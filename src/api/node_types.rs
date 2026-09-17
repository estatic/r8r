use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

pub async fn list_node_types(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
) -> impl IntoResponse {
    Json(state.registry.type_names()).into_response()
}
