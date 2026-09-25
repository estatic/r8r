use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use r8r::storage::Storage;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tower::ServiceExt;
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn test_state() -> AppState {
    let storage = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
        open_registration: false,
    }
}

async fn test_app() -> axum::Router {
    r8r::api::build_router(test_state().await)
}

/// Same as `test_app()`, but with `open_registration: true` -- registration
/// stays open even after a user already exists.
async fn test_app_with_open_registration() -> axum::Router {
    let mut state = test_state().await;
    state.open_registration = true;
    r8r::api::build_router(state)
}

/// Same as `test_app()`, but also hands back the `AppState` so a test can
/// inspect `trigger_registry` directly (e.g. to confirm a cron job is or
/// isn't registered after a request).
async fn test_app_with_state() -> (axum::Router, AppState) {
    let state = test_state().await;
    (r8r::api::build_router(state.clone()), state)
}

/// A `Storage` wrapper that delegates every method to a real, in-memory
/// `SqliteStorage`, except `update_workflow`, which fails on demand — used to
/// simulate the DB write in `set_workflow_active` failing after trigger state
/// has already been changed, so we can prove the handler's compensating
/// action actually runs.
struct FailingUpdateStorage {
    inner: SqliteStorage,
    fail_update: Arc<AtomicBool>,
    fail_execution_update: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Storage for FailingUpdateStorage {
    async fn create_workflow(&self, workflow: &r8r::domain::Workflow) -> anyhow::Result<()> {
        self.inner.create_workflow(workflow).await
    }
    async fn update_workflow(&self, workflow: &r8r::domain::Workflow) -> anyhow::Result<()> {
        if self.fail_update.load(Ordering::SeqCst) {
            anyhow::bail!("simulated storage failure");
        }
        self.inner.update_workflow(workflow).await
    }
    async fn delete_workflow(&self, id: uuid::Uuid) -> anyhow::Result<()> {
        self.inner.delete_workflow(id).await
    }
    async fn get_workflow(&self, id: uuid::Uuid) -> anyhow::Result<Option<r8r::domain::Workflow>> {
        self.inner.get_workflow(id).await
    }
    async fn list_workflows(&self) -> anyhow::Result<Vec<r8r::domain::Workflow>> {
        self.inner.list_workflows().await
    }
    async fn create_execution(&self, execution: &r8r::domain::Execution) -> anyhow::Result<()> {
        self.inner.create_execution(execution).await
    }
    async fn update_execution(&self, execution: &r8r::domain::Execution) -> anyhow::Result<()> {
        if self.fail_execution_update.load(Ordering::SeqCst) {
            anyhow::bail!("simulated storage failure");
        }
        self.inner.update_execution(execution).await
    }
    async fn get_execution(&self, id: uuid::Uuid) -> anyhow::Result<Option<r8r::domain::Execution>> {
        self.inner.get_execution(id).await
    }
    async fn list_executions_for_workflow(
        &self,
        workflow_id: uuid::Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<r8r::domain::Execution>> {
        self.inner.list_executions_for_workflow(workflow_id, limit).await
    }
    async fn create_user(&self, user: &r8r::domain::User) -> anyhow::Result<()> {
        self.inner.create_user(user).await
    }
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<r8r::domain::User>> {
        self.inner.get_user_by_email(email).await
    }
    async fn any_user_exists(&self) -> anyhow::Result<bool> {
        self.inner.any_user_exists().await
    }
    async fn create_credential(&self, credential: &r8r::domain::Credential) -> anyhow::Result<()> {
        self.inner.create_credential(credential).await
    }
    async fn get_credential(&self, id: uuid::Uuid) -> anyhow::Result<Option<r8r::domain::Credential>> {
        self.inner.get_credential(id).await
    }
    async fn list_credentials(&self) -> anyhow::Result<Vec<r8r::domain::CredentialSummary>> {
        self.inner.list_credentials().await
    }
    async fn update_credential(&self, credential: &r8r::domain::Credential) -> anyhow::Result<bool> {
        self.inner.update_credential(credential).await
    }
    async fn delete_credential(&self, id: uuid::Uuid) -> anyhow::Result<bool> {
        self.inner.delete_credential(id).await
    }
    async fn create_tool(&self, tool: &r8r::domain::Tool) -> anyhow::Result<()> {
        self.inner.create_tool(tool).await
    }
    async fn get_tool(&self, id: uuid::Uuid) -> anyhow::Result<Option<r8r::domain::Tool>> {
        self.inner.get_tool(id).await
    }
    async fn list_tools(&self) -> anyhow::Result<Vec<r8r::domain::Tool>> {
        self.inner.list_tools().await
    }
    async fn update_tool(&self, tool: &r8r::domain::Tool) -> anyhow::Result<bool> {
        self.inner.update_tool(tool).await
    }
    async fn delete_tool(&self, id: uuid::Uuid) -> anyhow::Result<bool> {
        self.inner.delete_tool(id).await
    }
}

/// Builds a router backed by `FailingUpdateStorage`, plus the `AppState` (to
/// inspect `trigger_registry`) and the flag that toggles whether
/// `update_workflow` fails — starts `false` so setup calls (e.g. an initial
/// successful activation) succeed; flip it to `true` right before the call
/// under test.
async fn test_app_with_failing_update() -> (axum::Router, AppState, Arc<AtomicBool>, Arc<AtomicBool>) {
    let inner = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    let fail_update = Arc::new(AtomicBool::new(false));
    let fail_execution_update = Arc::new(AtomicBool::new(false));
    let storage = FailingUpdateStorage {
        inner,
        fail_update: fail_update.clone(),
        fail_execution_update: fail_execution_update.clone(),
    };
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
        open_registration: false,
    };
    (r8r::api::build_router(state.clone()), state, fail_update, fail_execution_update)
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
async fn second_registration_is_rejected_by_default() {
    let app = test_app().await;
    register_and_get_token(&app, "first@example.com").await;

    let register_body = serde_json::json!({"email": "second@example.com", "password": "hunter2"});
    let response = app
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
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn second_registration_succeeds_when_open_registration_is_enabled() {
    let app = test_app_with_open_registration().await;
    register_and_get_token(&app, "first@example.com").await;

    let register_body = serde_json::json!({"email": "second@example.com", "password": "hunter2"});
    let response = app
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

/// Polls GET /rest/executions/:id until the run leaves `Running` (runs are
/// background tasks since Plan 8.7). Fails the test after 5s.
async fn wait_for_execution(app: &axum::Router, token: &str, execution_id: &str) -> serde_json::Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = app.clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/rest/executions/{execution_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if execution["status"] != "Running" {
                return execution;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("execution should finish within 5s")
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
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(started["status"], "Running");
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
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
async fn list_executions_for_workflow_returns_newest_first_and_honors_limit() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "exec-history@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "history-wf",
        "nodes": [{"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false}],
        "connections": []
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let mut execution_ids = Vec::new();
    for _ in 0..3 {
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
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
        execution_ids.push(execution["id"].as_str().unwrap().to_string());
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/rest/workflows/{workflow_id}/executions?limit=2"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let executions: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(executions.len(), 2);
    assert_eq!(executions[0]["id"], execution_ids[2]);
    assert_eq!(executions[1]["id"], execution_ids[1]);
}

#[tokio::test]
async fn list_executions_for_a_nonexistent_workflow_returns_404() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "exec-history-404@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/rest/workflows/{}/executions", uuid::Uuid::new_v4()))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
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

#[tokio::test]
async fn webhook_trigger_executes_workflow_end_to_end_then_404s_after_deactivation() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "webhook-e2e@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "webhook-wf",
        "nodes": [
            {"id": "hook", "node_type": "core.webhook", "position": [0.0, 0.0], "parameters": {"path": "my-test-hook", "method": "POST"}, "disabled": false},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"received": "{{ $json.body.name }}"}}, "disabled": false}
        ],
        "connections": [
            {"from_node": "hook", "from_output": 0, "to_node": "set1", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Fire the webhook — no authorization header, matching real external callers.
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri(format!("/webhook/{workflow_id}/my-test-hook"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"name": "Ada"}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["received"], "Ada");

    // Deactivate, then confirm the webhook path is dark.
    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": false}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(Request::builder().method("POST").uri(format!("/webhook/{workflow_id}/my-test-hook"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"name": "Ada"}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webhook_run_that_succeeds_but_fails_to_persist_still_returns_200_with_the_result() {
    // Documents a deliberate contract (see Plan 7.1's design spec, section
    // 11): run_and_track_execution's Err means only "couldn't create the
    // execution row" -- a failure to persist the FINAL result after an
    // otherwise-successful run is logged, not surfaced as an error
    // response, since the workflow's real side effects already happened
    // regardless of whether the DB write succeeded.
    let (app, _state, _fail_update, fail_execution_update) = test_app_with_failing_update().await;
    let token = register_and_get_token(&app, "webhook-persist-fail@example.com").await;

    let create_body = serde_json::json!({
        "name": "webhook-persist-fail-wf",
        "nodes": [{"id": "hook", "node_type": "core.webhook", "position": [0.0, 0.0], "parameters": {"path": "persist-fail", "method": "POST"}, "disabled": false}],
        "connections": []
    });
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let activate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    fail_execution_update.store(true, Ordering::SeqCst);

    let webhook_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/webhook/{workflow_id}/persist-fail"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The run itself succeeded (no engine failure) -- only the persistence
    // write failed, which must not turn into a 500.
    assert_eq!(webhook_response.status(), StatusCode::OK);
    let bytes = webhook_response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
}

fn scheduled_workflow_body() -> serde_json::Value {
    serde_json::json!({
        "name": "scheduled-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {"cron": "0 0 * * * *"}, "disabled": false}
        ],
        "connections": []
    })
}

// Happy-path regression check: a successful activate leaves a cron job
// registered, and a successful deactivate removes it. This must keep working
// once the compensating-action logic below is added to set_workflow_active.
#[tokio::test]
async fn activate_then_deactivate_updates_trigger_registry_correctly() {
    let (app, state) = test_app_with_state().await;
    let token = register_and_get_token(&app, "registry-happy-path@example.com").await;

    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(scheduled_workflow_body().to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();
    let workflow_uuid: uuid::Uuid = workflow_id.parse().unwrap();

    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // A successful activation must leave a cron job registered. `take_cron_job`
    // removes it as a side effect, so put it straight back — the deactivate
    // step below needs something registered to unregister.
    let job_id = state.trigger_registry.take_cron_job(workflow_uuid);
    assert!(job_id.is_some(), "expected a cron job to be registered after a successful activation");
    state.trigger_registry.record_cron_job(workflow_uuid, job_id.unwrap());

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": false}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        state.trigger_registry.take_cron_job(workflow_uuid).is_none(),
        "expected no cron job to remain registered after a successful deactivation"
    );
}

// Regression test for the finding: if activate_workflow_triggers succeeds but
// the subsequent update_workflow persist fails, the handler must compensate
// by unregistering the cron job it just registered — otherwise the job leaks
// and keeps firing forever, unreachable via the API (because the later
// idempotency short-circuit reads active=false from storage and never calls
// deactivate again).
#[tokio::test]
async fn activation_persist_failure_unregisters_the_cron_job() {
    let (app, state, fail_update, _fail_execution_update) = test_app_with_failing_update().await;
    let token = register_and_get_token(&app, "compensate-activate@example.com").await;

    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(scheduled_workflow_body().to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();
    let workflow_uuid: uuid::Uuid = workflow_id.parse().unwrap();

    fail_update.store(true, Ordering::SeqCst);

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    assert!(
        state.trigger_registry.take_cron_job(workflow_uuid).is_none(),
        "activate_workflow_triggers succeeded but update_workflow failed — the handler must have \
         compensated by unregistering the cron job, but it's still registered"
    );
}

// Symmetric regression test: if deactivate_workflow_triggers already
// unregistered the cron job but the subsequent update_workflow persist fails,
// the handler must compensate by re-registering it — otherwise the workflow
// silently stops running its schedule until a process restart.
#[tokio::test]
async fn deactivation_persist_failure_reregisters_the_cron_job() {
    let (app, state, fail_update, _fail_execution_update) = test_app_with_failing_update().await;
    let token = register_and_get_token(&app, "compensate-deactivate@example.com").await;

    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(scheduled_workflow_body().to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();
    let workflow_uuid: uuid::Uuid = workflow_id.parse().unwrap();

    // Activate normally first, while update_workflow still succeeds, so the
    // workflow is genuinely persisted active=true with a registered cron job.
    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Now make the persist step fail for the deactivation attempt.
    fail_update.store(true, Ordering::SeqCst);

    let response = app
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": false}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

    assert!(
        state.trigger_registry.take_cron_job(workflow_uuid).is_some(),
        "deactivate_workflow_triggers ran but update_workflow failed — the handler must have \
         compensated by re-registering the cron job, but it's missing"
    );
}

#[tokio::test]
async fn create_credential_never_returns_data_field() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred1@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({
                        "name": "my-bearer-cred",
                        "credential_type": "bearer",
                        "data": {"token": "super-secret-123"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["name"], "my-bearer-cred");
    assert!(body.get("data").is_none());
    let raw = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!raw.contains("super-secret-123"));
}

#[tokio::test]
async fn list_credentials_returns_created_ones_without_data() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred2@example.com").await;

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "cred-a", "credential_type": "bearer", "data": {"token": "x"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/rest/credentials")
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
    assert_eq!(list[0]["name"], "cred-a");
    assert!(list[0].get("data").is_none());
}

// Regression test for the fix in src/api/workflows.rs::execute_workflow:
// credential resolution now runs BEFORE the Execution{status: Running} row is
// built and persisted, so a workflow referencing a nonexistent credential
// fails fast with 400 instead of leaving a permanently stuck "Running"
// execution row behind (the old ordering created+persisted the Running row
// first, then resolved credentials, then bailed with 400 on failure —
// abandoning that row with no update_execution call ever able to reach it,
// since its id was never returned to the caller).
//
// There is no `GET /rest/executions` list endpoint, so we can't directly
// enumerate storage and assert "zero execution rows exist" after the failed
// call. What we CAN assert with the existing public HTTP surface:
//   1. The execute call returns 400, not 200/500 — credential resolution
//      failure is surfaced correctly.
//   2. The 400 response body is plain text (from `format!(...)`), not an
//      `Execution` JSON object — it must not contain a `"status"` field,
//      which is what a persisted (even if stuck) execution's response would
//      always carry. This at least confirms the handler returned before
//      ever constructing/serializing an `Execution`.
//   3. Most importantly: after the failed call, a SECOND, unrelated workflow
//      is created and executed to completion (200, status "Success") on the
//      exact same `test_app()` / in-memory storage instance. This proves the
//      failed credential-resolution attempt didn't leave the app, the
//      storage layer, or the DB connection in any broken/inconsistent state
//      — i.e. nothing about the reordering introduced a partial-write hazard
//      that would poison subsequent requests.
#[tokio::test]
async fn execute_workflow_with_nonexistent_credential_returns_400_before_touching_executions() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-exec@example.com").await;

    // A credential id that was never created via POST /rest/credentials.
    let missing_credential_id = uuid::Uuid::new_v4();

    let workflow_body = serde_json::json!({
        "name": "bad-credential-wf",
        "nodes": [
            {
                "id": "http1",
                "node_type": "core.httpRequest",
                "position": [0.0, 0.0],
                "parameters": {"auth": {"type": "bearer", "credential_id": missing_credential_id.to_string()}},
                "disabled": false
            }
        ],
        "connections": []
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

    // Attempt to execute — credential resolution must fail BEFORE any
    // Execution row is created, so this must be a 400, not 200.
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
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        body_text.contains("credential resolution failed"),
        "expected the plain-text credential-resolution error, got: {body_text}"
    );
    // A real (even if stuck) persisted Execution would serialize as JSON with
    // a "status" field (e.g. `{"id":...,"status":"Running",...}`). The 400
    // body here is a plain error string, not that shape at all.
    assert!(!body_text.contains("\"status\""));

    // Now prove the app/storage is still fully healthy: create and execute a
    // second, unrelated, valid workflow on the SAME app instance and confirm
    // it runs to completion normally.
    let workflow_body_2 = serde_json::json!({
        "name": "healthy-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"ok": true}}, "disabled": false}
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
                .body(Body::from(workflow_body_2.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow_2: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_2_id = workflow_2["id"].as_str().unwrap();

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_2_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["ok"], true);
}

#[tokio::test]
async fn create_credential_requires_auth() {
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "x", "credential_type": "bearer", "data": {}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn credential_authenticated_http_request_executes_end_to_end() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .and(header("authorization", "Bearer e2e-secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"orders": [1, 2, 3]})))
        .mount(&mock_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "http-e2e@example.com").await;

    let cred_response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "orders-api", "credential_type": "bearer", "data": {"token": "e2e-secret-token"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cred_response.status(), StatusCode::CREATED);
    let bytes = cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let credential_id = credential["id"].as_str().unwrap();

    let workflow_body = serde_json::json!({
        "name": "http-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "http1", "node_type": "core.httpRequest", "position": [1.0, 0.0], "parameters": {
                "method": "GET",
                "url": format!("{}/orders", mock_server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id}
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "http1", "to_input": 0}
        ]
    });
    let wf_response = app.clone()
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
    assert_eq!(wf_response.status(), StatusCode::CREATED);
    let bytes = wf_response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let exec_response = app.clone()
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
    assert_eq!(exec_response.status(), StatusCode::ACCEPTED);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["http1"][0]["json"], serde_json::json!({"orders": [1, 2, 3]}));
}

#[tokio::test]
async fn credential_authenticated_telegram_send_message_executes_end_to_end() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot987:XYZ/sendMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {"message_id": 7, "text": "hello from r8r"}
        })))
        .mount(&mock_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "telegram-e2e@example.com").await;

    let cred_response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "my-bot", "credential_type": "telegramApi", "data": {"bot_token": "987:XYZ"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cred_response.status(), StatusCode::CREATED);
    let bytes = cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let credential_id = credential["id"].as_str().unwrap();

    let workflow_body = serde_json::json!({
        "name": "telegram-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "send", "node_type": "telegram.sendMessage", "position": [1.0, 0.0], "parameters": {
                "chat_id": "42",
                "text": "hello from r8r",
                "auth": {"credential_id": credential_id},
                "api_base_url": mock_server.uri()
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "send", "to_input": 0}
        ]
    });
    let wf_response = app.clone()
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
    assert_eq!(wf_response.status(), StatusCode::CREATED);
    let bytes = wf_response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let exec_response = app.clone()
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
    assert_eq!(exec_response.status(), StatusCode::ACCEPTED);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["send"][0]["json"]["result"]["message_id"], 7);
}

#[tokio::test]
async fn telegram_trigger_fires_downstream_node_on_incoming_update() {
    let telegram = MockServer::start().await;

    // The "incoming" bot: the trigger polls this.
    Mock::given(method("GET"))
        .and(path("/bot111:AAA/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": [{"update_id": 1, "message": {"chat": {"id": 918273}, "text": "ping from the trigger test"}}]
        })))
        .up_to_n_times(1)
        .mount(&telegram)
        .await;
    Mock::given(method("GET"))
        .and(path("/bot111:AAA/getUpdates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": []})))
        .mount(&telegram)
        .await;

    // The "outgoing" bot: the downstream telegram.sendMessage node calls
    // this. Matching on the request body (not just method+path) proves the
    // incoming update's JSON actually flowed through the trigger into the
    // downstream node's templated parameters -- see the workflow's `echo`
    // node parameters below, which template `chat_id`/`text` off
    // `$json.message.chat.id`/`$json.message.text` (the seeded update
    // above). Without this, the test would pass identically even if the
    // trigger's Update JSON never reached the workflow.
    Mock::given(method("POST"))
        .and(path("/bot222:BBB/sendMessage"))
        .and(body_json(serde_json::json!({
            "chat_id": 918273,
            "text": "echo: ping from the trigger test"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "result": {"message_id": 7}
        })))
        .mount(&telegram)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "telegram-trigger-e2e@example.com").await;

    let incoming_cred = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "incoming-bot", "credential_type": "telegramApi", "data": {"bot_token": "111:AAA"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(incoming_cred.status(), StatusCode::CREATED);
    let bytes = incoming_cred.into_body().collect().await.unwrap().to_bytes();
    let incoming_cred_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let outgoing_cred = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "outgoing-bot", "credential_type": "telegramApi", "data": {"bot_token": "222:BBB"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(outgoing_cred.status(), StatusCode::CREATED);
    let bytes = outgoing_cred.into_body().collect().await.unwrap().to_bytes();
    let outgoing_cred_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let workflow_body = serde_json::json!({
        "name": "telegram-trigger-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "telegram.trigger", "position": [0.0, 0.0], "parameters": {
                "auth": {"credential_id": incoming_cred_id},
                "api_base_url": telegram.uri()
            }, "disabled": false},
            {"id": "echo", "node_type": "telegram.sendMessage", "position": [1.0, 0.0], "parameters": {
                "chat_id": "{{ $json.message.chat.id }}",
                "text": "echo: {{ $json.message.text }}",
                "auth": {"credential_id": outgoing_cred_id},
                "api_base_url": telegram.uri()
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "echo", "to_input": 0}
        ]
    });
    let wf_response = app.clone()
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
    assert_eq!(wf_response.status(), StatusCode::CREATED);
    let bytes = wf_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let activate_response = app.clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    // Bounded wait for the background poll task to fetch the queued update
    // and fire the downstream telegram.sendMessage node against the same
    // mock server. Outer timeout is a belt-and-braces bound, matching the
    // pattern already established in http_request.rs's timeout test and
    // this plan's own telegram_poller.rs tests.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let requests = telegram.received_requests().await.unwrap();
            if requests.iter().any(|r| r.url.path().contains("sendMessage")) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("telegram.trigger should have fired the downstream telegram.sendMessage node within 5s");

    // Clean up: deactivate so the background poll task stops before the
    // test process exits (avoids a dangling task hammering a mock server
    // that's about to be dropped).
    let deactivate_response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deactivate_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn update_workflow_persists_new_nodes_and_name() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "update-wf@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "original", "nodes": [], "connections": []}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let update_body = serde_json::json!({
        "name": "renamed",
        "nodes": [{"id": "n1", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false}],
        "connections": []
    });
    let update_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(update_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);
    let bytes = update_response.into_body().collect().await.unwrap().to_bytes();
    let updated: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated["name"], "renamed");
    assert_eq!(updated["nodes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn update_workflow_on_missing_id_returns_404() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "update-missing@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/rest/workflows/{}", uuid::Uuid::new_v4()))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"name": "x", "nodes": [], "connections": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_workflow_removes_it_and_then_404s() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "delete-wf@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"name": "to-delete", "nodes": [], "connections": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let delete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let get_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_an_active_workflow_deactivates_its_trigger_first() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "delete-active-wf@example.com").await;

    let create_body = serde_json::json!({
        "name": "active-to-delete",
        "nodes": [{"id": "t", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {"cron": "0 0 0 1 1 *"}, "disabled": false}],
        "connections": []
    });
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let activate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    // Must not hang or error just because the workflow is active with a
    // live cron job registered — deletion has to tear that down cleanly.
    let delete_response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn deleting_an_executed_workflow_succeeds() {
    // Regression test: executions.workflow_id has no ON DELETE CASCADE, so
    // deleting a workflow that was previously executed used to 500 with a
    // FOREIGN KEY constraint violation. Proves the fix at the HTTP level,
    // exercising the exact "execute then delete" path a real user hits.
    let app = test_app().await;
    let token = register_and_get_token(&app, "delete-executed-wf@example.com").await;

    let create_body = serde_json::json!({
        "name": "executed-then-deleted",
        "nodes": [{"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false}],
        "connections": []
    });
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let execute_response = app
        .clone()
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
    assert_eq!(execute_response.status(), StatusCode::ACCEPTED);
    let bytes = execute_response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;

    let delete_response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn node_types_lists_registered_types_with_metadata() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "node-types@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/rest/node-types")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let types = json.as_array().unwrap();
    assert!(!types.is_empty());
    let agent = types.iter().find(|t| t["type_name"] == "ai.agent").expect("ai.agent should be listed");
    assert_eq!(agent["display_name"], "AI Agent");
    assert_eq!(agent["category"], "ai");
    assert_eq!(agent["credential_types"], serde_json::json!(["anthropicApi", "openaiApi"]));
    let if_node = types.iter().find(|t| t["type_name"] == "core.if").expect("core.if should be listed");
    assert_eq!(if_node["output_ports"], serde_json::json!(["true", "false"]));
}

#[tokio::test]
async fn output_ports_for_switch_reflects_case_count() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "ports@example.com").await;

    let body = serde_json::json!({"parameters": {"cases": [1, 2, 3]}});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/node-types/core.switch/output-ports")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["output_ports"], serde_json::json!(["case 0", "case 1", "case 2", "default"]));
}

#[tokio::test]
async fn output_ports_for_unknown_type_returns_404() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "ports2@example.com").await;

    let body = serde_json::json!({"parameters": {}});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/node-types/does.not.exist/output-ports")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn root_path_serves_the_embedded_frontend() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::builder().method("GET").uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}

#[tokio::test]
async fn a_client_side_route_falls_back_to_the_frontend_not_a_404() {
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/workflows/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}

#[tokio::test]
async fn rest_routes_still_take_priority_over_the_frontend_fallback() {
    let app = test_app().await;
    // An unauthenticated request to a real, known API route must get that
    // route's own real behavior (here: 400/422 for a malformed body, from
    // axum's own JSON extractor), never the SPA's index.html -- proving
    // the fallback genuinely only catches unmatched paths.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from("not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}

#[tokio::test]
async fn unmatched_api_paths_return_404_not_the_spa() {
    // The test above only covers a *matched* route's own error response. An
    // unmatched path under /rest or /webhook reaches the SPA fallback, and
    // must get a clean 404 rather than 200 + index.html -- otherwise a
    // frontend typo or a renamed endpoint looks like a successful HTML page.
    for uri in ["/rest/definitely-not-a-real-route", "/webhook/definitely-not-a-real-route"] {
        let app = test_app().await;
        let response = app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri} must 404");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(!String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"), "{uri} must not serve the SPA");
    }
}

#[tokio::test]
async fn websocket_streams_node_events_during_a_run_and_a_bad_token_closes_it() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let state = test_state().await;
    let app = r8r::api::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_app = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, serve_app).await.unwrap();
    });

    let token = register_and_get_token(&app, "ws-live@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({
                        "name": "ws-live",
                        "nodes": [
                            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
                            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"a": 1}}, "disabled": false}
                        ],
                        "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "set1", "to_input": 0}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    // A bad token closes the socket without forwarding anything.
    let (mut bad_ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/workflows/{workflow_id}/executions"))
        .await
        .unwrap();
    bad_ws.send(WsMessage::Text(serde_json::json!({"token": "not-a-real-token"}).to_string())).await.unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(2), bad_ws.next()).await.unwrap();
    assert!(matches!(next, Some(Ok(WsMessage::Close(_))) | None));

    // A good token streams the run's events.
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/workflows/{workflow_id}/executions"))
        .await
        .unwrap();
    ws.send(WsMessage::Text(serde_json::json!({"token": token}).to_string())).await.unwrap();

    // `handle_execution_socket` (src/api/executions.rs) only calls
    // `state.execution_events.subscribe()` AFTER verifying this auth frame --
    // there's no ack sent back over the socket confirming the subscription is
    // live. Without this wait, spawning the execute() call immediately below
    // races that server-side subscribe(): under load (e.g. many tests
    // running in parallel), the trivial workflow below can finish and
    // broadcast its events before the server has subscribed, and this test
    // then waits forever for events that already happened and were never
    // delivered to this receiver (tokio::sync::broadcast only delivers
    // events sent after a receiver subscribes). A short wait here is
    // overwhelmingly longer than the actual auth-verify-then-subscribe path
    // takes, and keeps this test decoupled from adding a new WS protocol
    // message just to fix its own synchronization.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let exec_app = app.clone();
    let exec_token = token.clone();
    let exec_workflow_id = workflow_id.clone();
    tokio::spawn(async move {
        exec_app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/rest/workflows/{exec_workflow_id}/execute"))
                    .header("authorization", format!("Bearer {exec_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    });

    let mut seen_types = Vec::new();
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for a websocket event")
            .expect("socket closed early")
            .unwrap();
        if let WsMessage::Text(text) = msg {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value["workflow_id"], workflow_id);
            assert!(value["execution_id"].as_str().is_some(), "every event must carry a non-null execution_id");
            let event_type = value["type"].as_str().unwrap().to_string();
            if event_type == "node_finished" && value["node_id"] == "set1" {
                assert_eq!(value["node_id"], "set1");
                assert_eq!(value["items"][0]["json"]["a"], 1);
            }
            let is_final = event_type == "execution_finished";
            seen_types.push(event_type);
            if is_final {
                break;
            }
        }
    }

    assert!(seen_types.contains(&"node_started".to_string()));
    assert!(seen_types.contains(&"node_finished".to_string()));
    assert_eq!(seen_types.last().unwrap(), "execution_finished");
}

#[tokio::test]
async fn websocket_never_forwards_events_for_a_different_workflow() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let state = test_state().await;
    let app = r8r::api::build_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve_app = app.clone();
    tokio::spawn(async move {
        axum::serve(listener, serve_app).await.unwrap();
    });

    let token = register_and_get_token(&app, "ws-isolation@example.com").await;

    async fn create_manual_workflow(app: &axum::Router, token: &str, name: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rest/workflows")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(
                        serde_json::json!({
                            "name": name,
                            "nodes": [{"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false}],
                            "connections": []
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string()
    }

    async fn execute(app: &axum::Router, token: &str, workflow_id: &str) {
        app.clone()
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
    }

    let workflow_a = create_manual_workflow(&app, &token, "ws-isolation-a").await;
    let workflow_b = create_manual_workflow(&app, &token, "ws-isolation-b").await;

    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/workflows/{workflow_a}/executions"))
        .await
        .unwrap();
    ws.send(WsMessage::Text(serde_json::json!({"token": token}).to_string())).await.unwrap();

    // Executing workflow B must produce nothing on the socket subscribed to A.
    execute(&app, &token, &workflow_b).await;
    let nothing_arrived = tokio::time::timeout(std::time::Duration::from_millis(500), ws.next()).await;
    assert!(nothing_arrived.is_err(), "socket subscribed to workflow A must not receive workflow B's events");

    // Now execute A -- its events must arrive.
    execute(&app, &token, &workflow_a).await;
    let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
        .await
        .expect("timed out waiting for workflow A's event")
        .expect("socket closed early")
        .unwrap();
    if let WsMessage::Text(text) = msg {
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["workflow_id"], workflow_a);
    } else {
        panic!("expected a text frame");
    }
}

#[tokio::test]
async fn agent_node_calls_a_tool_then_returns_a_final_response_end_to_end() {
    let anthropic_server = wiremock::MockServer::start().await;
    let tool_server = wiremock::MockServer::start().await;

    // Turn 1: Anthropic responds with a tool_use call to core.httpRequest.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/messages"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "id": "msg_1",
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "tool_use", "id": "toolu_1", "name": "fetch_status", "input": {}}],
                    "stop_reason": "tool_use"
                }))
                .append_header("content-type", "application/json"),
        )
        .up_to_n_times(1)
        .mount(&anthropic_server)
        .await;

    // Turn 2: Anthropic responds with the final text answer.
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/messages"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_2",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "The status is ok."}],
            "stop_reason": "end_turn"
        })))
        .mount(&anthropic_server)
        .await;

    // The tool itself: core.httpRequest hitting a second mocked endpoint.
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/status"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": "ok"})))
        .mount(&tool_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "agent-e2e@example.com").await;

    let create_cred_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "test-anthropic", "credential_type": "anthropicApi", "data": {"api_key": "test-key"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let workflow_body = serde_json::json!({
        "name": "agent-e2e",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {
                "id": "agent1",
                "node_type": "ai.agent",
                "position": [1.0, 0.0],
                "parameters": {
                    "provider": "anthropic",
                    "model": "claude-opus-5",
                    "system_prompt": "You are a status-checking assistant.",
                    "user_message": "What is the status?",
                    "auth": {"credential_id": credential_id},
                    "api_base_url": anthropic_server.uri(),
                    "max_iterations": 5,
                    "tools": [{
                        "name": "fetch_status",
                        "description": "Fetch the current status",
                        "node_type": "core.httpRequest",
                        "argument_schema": {"type": "object", "properties": {}},
                        "base_parameters": {"method": "GET", "url": format!("{}/status", tool_server.uri())}
                    }]
                },
                "disabled": false
            }
        ],
        "connections": [{"from_node": "trigger", "from_output": 0, "to_node": "agent1", "to_input": 0}]
    });
    let create_response = app
        .clone()
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
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

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
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let started: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, started["id"].as_str().unwrap()).await;
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["agent1"][0]["json"]["response"], "The status is ok.");
    assert_eq!(execution["node_outputs"]["agent1"][0]["json"]["tool_calls_made"], 1);
}

#[tokio::test]
async fn credential_types_requires_authentication() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::builder().method("GET").uri("/rest/credential-types").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn credential_types_lists_all_known_schemas() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-types@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/rest/credential-types")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let types = json.as_array().unwrap();
    assert_eq!(types.len(), 6);
    let telegram = types.iter().find(|t| t["credential_type"] == "telegramApi").expect("telegramApi should be listed");
    assert_eq!(telegram["generic"], false);
    assert_eq!(telegram["fields"][0]["name"], "bot_token");
    assert_eq!(telegram["fields"][0]["field_type"], "password");
    let bearer = types.iter().find(|t| t["credential_type"] == "bearerToken").expect("bearerToken should be listed");
    assert_eq!(bearer["generic"], true);
}

async fn post_workflow(app: &axum::Router, token: &str, body: serde_json::Value) -> axum::response::Response {
    app.clone()
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
        .unwrap()
}

fn settings_workflow(settings: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": "wf-settings",
        "nodes": [{"id": "n1", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "settings": settings}],
        "connections": []
    })
}

#[tokio::test]
async fn create_workflow_rejects_out_of_range_node_settings() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "settings-bad@example.com").await;
    let response = post_workflow(&app, &token, settings_workflow(serde_json::json!({"retry": {"max_tries": 11, "wait_ms": 0}}))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(String::from_utf8_lossy(&bytes), "node n1: retry.max_tries must be between 2 and 10");
}

#[tokio::test]
async fn node_settings_round_trip_and_invalid_update_leaves_workflow_unchanged() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "settings-ok@example.com").await;
    let settings = serde_json::json!({"retry": {"max_tries": 3, "wait_ms": 500}, "timeout_ms": 1000, "continue_on_fail": true});
    let response = post_workflow(&app, &token, settings_workflow(settings.clone())).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let bad = settings_workflow(serde_json::json!({"timeout_ms": 0}));
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/rest/workflows/{id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(bad.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let fetched: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(fetched["nodes"][0]["settings"], settings);
}

async fn create_active_webhook_workflow(app: &axum::Router, token: &str, hook_path: &str, respond: Option<&str>) -> String {
    let mut params = serde_json::json!({"path": hook_path, "method": "POST"});
    if let Some(r) = respond {
        params["respond"] = serde_json::json!(r);
    }
    let workflow_body = serde_json::json!({
        "name": "webhook-respond",
        "nodes": [
            {"id": "hook", "node_type": "core.webhook", "position": [0.0, 0.0], "parameters": params},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"received": "{{ $json.body.name }}"}}}
        ],
        "connections": [{"from_node": "hook", "from_output": 0, "to_node": "set1", "to_input": 0}]
    });
    let response = app.clone()
        .oneshot(Request::builder().method("POST").uri("/rest/workflows")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(workflow_body.to_string())).unwrap())
        .await.unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();
    let response = app.clone()
        .oneshot(Request::builder().method("PATCH").uri(format!("/rest/workflows/{workflow_id}/active"))
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(serde_json::json!({"active": true}).to_string())).unwrap())
        .await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    workflow_id
}

async fn fire_webhook(app: &axum::Router, workflow_id: &str, hook_path: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().method("POST").uri(format!("/webhook/{workflow_id}/{hook_path}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({"name": "Ada"}).to_string())).unwrap())
        .await.unwrap()
}

#[tokio::test]
async fn webhook_respond_immediately_returns_202_with_execution_id() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "webhook-immediate@example.com").await;
    let workflow_id = create_active_webhook_workflow(&app, &token, "fast-hook", Some("immediately")).await;
    let response = fire_webhook(&app, &workflow_id, "fast-hook").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let execution = wait_for_execution(&app, &token, body["execution_id"].as_str().unwrap()).await;
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["received"], "Ada");
}

#[tokio::test]
async fn webhook_with_unrecognised_respond_value_waits_like_the_default() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "webhook-unknown@example.com").await;
    let workflow_id = create_active_webhook_workflow(&app, &token, "odd-hook", Some("whenever")).await;
    let response = fire_webhook(&app, &workflow_id, "odd-hook").await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["received"], "Ada");
}

async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn create_cred(app: &axum::Router, token: &str, name: &str, ty: &str, data: serde_json::Value) -> String {
    let (status, body) = send(app, "POST", "/rest/credentials", token, Some(serde_json::json!({"name": name, "credential_type": ty, "data": data}))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn get_credential_by_id_returns_text_fields_but_never_secrets() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-get@example.com").await;
    let id = create_cred(&app, &token, "hdr", "apiKeyHeader", serde_json::json!({"header_name": "X-Key", "value": "top-secret"})).await;
    let (status, body) = send(&app, "GET", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["fields"]["header_name"], "X-Key");
    assert!(body["fields"].get("value").is_none());
    assert!(!body.to_string().contains("top-secret"));
    assert_eq!(body["used_by"], 0);
    let (status, _) = send(&app, "GET", &format!("/rest/credentials/{}", uuid::Uuid::new_v4()), &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_name_only_keeps_secret() {
    let (app, state) = test_app_with_state().await;
    let token = register_and_get_token(&app, "cred-rename@example.com").await;
    let id = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "123:ABC"})).await;
    let (status, body) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"name": "  renamed bot  "}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed bot");
    let stored = state.storage.get_credential(id.parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(stored.data, serde_json::json!({"bot_token": "123:ABC"}));
    assert_eq!(stored.credential_type, "telegramApi");
}

#[tokio::test]
async fn patch_merges_typed_data_and_replaces_untyped_data() {
    let (app, state) = test_app_with_state().await;
    let token = register_and_get_token(&app, "cred-patch@example.com").await;
    let typed = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "old"})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{typed}"), &token, Some(serde_json::json!({"data": {"bot_token": ""}}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.storage.get_credential(typed.parse().unwrap()).await.unwrap().unwrap().data, serde_json::json!({"bot_token": "old"}));
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{typed}"), &token, Some(serde_json::json!({"data": {"bot_token": "new"}, "credential_type": "bearerToken"}))).await;
    assert_eq!(s, StatusCode::OK);
    let stored = state.storage.get_credential(typed.parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(stored.data, serde_json::json!({"bot_token": "new"}));
    assert_eq!(stored.credential_type, "telegramApi");

    let untyped = create_cred(&app, &token, "custom", "myCustomThing", serde_json::json!({"a": 1})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{untyped}"), &token, Some(serde_json::json!({"data": {"b": 2}}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.storage.get_credential(untyped.parse().unwrap()).await.unwrap().unwrap().data, serde_json::json!({"b": 2}));
}

#[tokio::test]
async fn patch_rejects_blank_name_bad_data_and_missing_ids() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-bad@example.com").await;
    let id = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "x"})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"name": "   "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"data": "not an object"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{}", uuid::Uuid::new_v4()), &token, Some(serde_json::json!({"name": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_is_refused_while_a_workflow_uses_the_credential() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-del@example.com").await;
    let id = create_cred(&app, &token, "bearer", "bearerToken", serde_json::json!({"token": "t"})).await;
    let wf_body = serde_json::json!({
        "name": "uses-cred",
        "nodes": [{"id": "h", "node_type": "core.httpRequest", "position": [0.0, 0.0], "parameters": {"url": "https://example.com", "auth": {"type": "bearer", "credential_id": id}}}],
        "connections": []
    });
    let (s, wf) = send(&app, "POST", "/rest/workflows", &token, Some(wf_body)).await;
    assert_eq!(s, StatusCode::CREATED);
    let wf_id = wf["id"].as_str().unwrap().to_string();

    let (s, list) = send(&app, "GET", "/rest/credentials", &token, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().iter().find(|c| c["id"] == id.as_str()).unwrap()["used_by"], 1);

    let (s, body) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["error"], "credential is in use");
    assert_eq!(body["workflows"][0]["name"], "uses-cred");

    let (s, _) = send(&app, "DELETE", &format!("/rest/workflows/{wf_id}"), &token, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
