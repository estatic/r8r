use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use std::sync::Arc;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let state = AppState {
        storage: Arc::new(storage),
        jwt_secret: "test-secret".into(),
    };
    r8r::api::build_router(state)
}

#[tokio::test]
async fn register_then_login_returns_tokens() {
    let app = test_app().await;

    let register_body = serde_json::json!({"email": "a@b.com", "password": "hunter2"});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(register_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["token"].as_str().unwrap().len() > 10);

    let login_body = serde_json::json!({"email": "a@b.com", "password": "hunter2"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_with_wrong_password_returns_401() {
    let app = test_app().await;
    let register_body = serde_json::json!({"email": "c@d.com", "password": "right"});
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(register_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let login_body = serde_json::json!({"email": "c@d.com", "password": "wrong"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_with_unregistered_email_returns_401() {
    // Structural check for the login timing/user-enumeration fix: an email
    // that was never registered must take the same "401, no distinguishing
    // body" branch as a wrong password for a registered email (see
    // login_with_wrong_password_returns_401 above and the dummy-hash verify
    // in src/api/auth.rs::login).
    let app = test_app().await;
    let login_body = serde_json::json!({"email": "nobody@nowhere.com", "password": "whatever"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
