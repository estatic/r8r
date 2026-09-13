use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:r8r.db".to_string());
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        tracing::warn!("JWT_SECRET not set; using an insecure default for development");
        "dev-insecure-secret-change-me".to_string()
    });

    let storage = SqliteStorage::new(&db_url)
        .await
        .expect("failed to initialize storage");
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
    };

    let app = r8r::api::build_router(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("r8r listening on :3000");
    axum::serve(listener, app).await.unwrap();
}
