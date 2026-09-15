use axum::body::Body;
use axum::http::{Request, StatusCode};
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

async fn register_and_get_token(app: &axum::Router, email: &str) -> String {
    let body = serde_json::json!({"email": email, "password": "hunter2"});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn create_workflow_requires_auth() {
    let app = test_app().await;
    let body = serde_json::json!({"name": "wf1", "nodes": [], "connections": []});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_then_get_then_list_workflow() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "wfuser@example.com").await;

    let body = serde_json::json!({"name": "wf1", "nodes": [], "connections": []});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let response = app.clone()
        .oneshot(
            Request::builder()
                .uri(format!("/rest/workflows/{id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/rest/workflows")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn create_execute_and_fetch_execution_end_to_end() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "exec@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "exec-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"greeting": "hi"}}, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "set1", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(workflow_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["greeting"], "hi");
    let execution_id = execution["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/rest/executions/{execution_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn branching_workflow_with_if_and_merge_executes_end_to_end() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "branch@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "branch-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "branch", "node_type": "core.if", "position": [1.0, 0.0], "parameters": {"condition": "{{ 1 == 1 }}"}, "disabled": false},
            {"id": "true_branch", "node_type": "core.set", "position": [2.0, 0.0], "parameters": {"fields": {"path": "true"}}, "disabled": false},
            {"id": "false_branch", "node_type": "core.set", "position": [2.0, 1.0], "parameters": {"fields": {"path": "false"}}, "disabled": false},
            {"id": "merged", "node_type": "core.merge", "position": [3.0, 0.0], "parameters": {"mode": "append"}, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "branch", "to_input": 0},
            {"from_node": "branch", "from_output": 0, "to_node": "true_branch", "to_input": 0},
            {"from_node": "branch", "from_output": 1, "to_node": "false_branch", "to_input": 0},
            {"from_node": "true_branch", "from_output": 0, "to_node": "merged", "to_input": 0},
            {"from_node": "false_branch", "from_output": 0, "to_node": "merged", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(workflow_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");

    // Only the true branch ran (condition is always true), so the merge node's
    // output should show exactly one item, from true_branch.
    let merged_output = &execution["node_outputs"]["merged"];
    assert_eq!(merged_output.as_array().unwrap().len(), 1);
    assert_eq!(merged_output[0]["json"]["path"], "true");
    // The false branch produced no items (If routed nothing to output 1).
    assert_eq!(execution["node_outputs"]["false_branch"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn activate_workflow_with_valid_schedule_succeeds() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "activate1@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "scheduled-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {"cron": "0 0 * * * *"}, "disabled": false}
        ],
        "connections": []
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let updated: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated["active"], true);
}

#[tokio::test]
async fn activate_workflow_with_invalid_cron_param_returns_400() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "activate2@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "broken-schedule",
        "nodes": [
            {"id": "trigger", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {}, "disabled": false}
        ],
        "connections": []
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
