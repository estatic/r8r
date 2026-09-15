use axum::body::Body;
use http_body_util::BodyExt;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use std::sync::Arc;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
    };
    r8r::api::build_router(state)
}

#[tokio::test]
async fn health_returns_ok() {
    let app = test_app().await;
    let response = app
        .oneshot(axum::http::Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
