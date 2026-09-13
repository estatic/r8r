// tests/health_test.rs
use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "../src/main.rs"]
mod app_main;

#[tokio::test]
async fn health_returns_ok() {
    let app = app_main::health_router();
    let response = app
        .oneshot(axum::http::Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
