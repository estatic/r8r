use axum::{routing::get, Router};

pub fn health_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let app = health_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("r8r listening on :3000");
    axum::serve(listener, app).await.unwrap();
}
