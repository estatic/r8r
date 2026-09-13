pub mod auth;
pub mod workflows;

use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/rest/workflows", post(workflows::create_workflow).get(workflows::list_workflows))
        .route("/rest/workflows/:id", get(workflows::get_workflow))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
}
