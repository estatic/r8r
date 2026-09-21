pub mod auth;
pub mod credentials;
pub mod credential_types;
pub mod executions;
pub mod webhook;
pub mod workflows;
pub mod node_types;

use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/rest/workflows", post(workflows::create_workflow).get(workflows::list_workflows))
        .route(
            "/rest/workflows/:id",
            get(workflows::get_workflow)
                .put(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
        .route("/rest/workflows/:id/execute", post(workflows::execute_workflow))
        .route("/rest/workflows/:id/active", axum::routing::patch(workflows::set_workflow_active))
        .route("/rest/workflows/:id/executions", get(executions::list_executions_for_workflow))
        .route("/ws/workflows/:id/executions", get(executions::subscribe_executions))
        .route("/rest/credential-types", get(credential_types::list_credential_types))
        .route("/rest/node-types", get(node_types::list_node_types))
        .route("/rest/node-types/:type_name/output-ports", post(node_types::output_ports_for_type))
        .route("/rest/executions/:id", get(executions::get_execution))
        .route("/rest/credentials", post(credentials::create_credential).get(credentials::list_credentials))
        .route(
            "/webhook/:workflow_id/:path",
            axum::routing::get(webhook::handle_webhook).post(webhook::handle_webhook),
        )
        .route("/health", get(|| async { "ok" }))
        .fallback(crate::static_files::serve_frontend)
        .with_state(state)
}
