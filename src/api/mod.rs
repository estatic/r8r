pub mod auth;

use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .with_state(state)
}
