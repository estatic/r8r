#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let app = r8r::health_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("r8r listening on :3000");
    axum::serve(listener, app).await.unwrap();
}
