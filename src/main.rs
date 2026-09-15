use r8r::node::NodeRegistry;
use r8r::scheduler::Scheduler;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use r8r::trigger_registry::TriggerRegistry;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./r8r.db?mode=rwc".into());
    let jwt_secret = std::env::var("JWT_SECRET").map_err(|_| {
        anyhow::anyhow!("JWT_SECRET environment variable must be set (see .env.example)")
    })?;
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let storage = SqliteStorage::new(&database_url).await?;
    let mut registry = NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let scheduler = Scheduler::new().await?;

    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
        scheduler: Arc::new(scheduler),
        trigger_registry: Arc::new(TriggerRegistry::new()),
    };

    if let Err(e) = r8r::triggers::reactivate_all(&state).await {
        tracing::warn!(error = %e, "failed to reactivate workflow triggers on startup");
    }

    let app = r8r::api::build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("r8r listening on :{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
