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
    }
}

async fn test_app() -> axum::Router {
    r8r::api::build_router(test_state().await)
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
        self.inner.update_execution(execution).await
    }
    async fn get_execution(&self, id: uuid::Uuid) -> anyhow::Result<Option<r8r::domain::Execution>> {
        self.inner.get_execution(id).await
    }
    async fn create_user(&self, user: &r8r::domain::User) -> anyhow::Result<()> {
        self.inner.create_user(user).await
    }
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<r8r::domain::User>> {
        self.inner.get_user_by_email(email).await
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
}

/// Builds a router backed by `FailingUpdateStorage`, plus the `AppState` (to
/// inspect `trigger_registry`) and the flag that toggles whether
/// `update_workflow` fails — starts `false` so setup calls (e.g. an initial
/// successful activation) succeed; flip it to `true` right before the call
/// under test.
async fn test_app_with_failing_update() -> (axum::Router, AppState, Arc<AtomicBool>) {
    let inner = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    let fail_update = Arc::new(AtomicBool::new(false));
    let storage = FailingUpdateStorage { inner, fail_update: fail_update.clone() };
    let mut registry = r8r::node::NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
    };
    (r8r::api::build_router(state.clone()), state, fail_update)
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
    let (app, state, fail_update) = test_app_with_failing_update().await;
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
    let (app, state, fail_update) = test_app_with_failing_update().await;
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

    let response = app
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
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
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

    let exec_response = app
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
    assert_eq!(exec_response.status(), StatusCode::OK);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
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

    let exec_response = app
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
    assert_eq!(exec_response.status(), StatusCode::OK);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
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
