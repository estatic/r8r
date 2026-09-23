//! Log-output tests. They live in their own test binary with one global
//! capture subscriber: a thread-local (`set_default`) subscriber can miss
//! events when parallel tests hit the same callsites with no subscriber and
//! tracing caches that callsite as disabled. Each test filters the shared
//! output by its own unique id, so they can still run in parallel.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use r8r::domain::{Connection, ExecutionMode, NodeInstance, Workflow};
use r8r::node::{Node, NodeCategory, NodeError, NodeExecutionContext, NodeOutput, NodeRegistry};
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use r8r::storage::Storage;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;
use uuid::Uuid;

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Installs the process-wide capture subscriber once; returns the lines
/// captured so far that mention `marker`.
fn lines_mentioning(marker: &str) -> Vec<String> {
    static LOGS: OnceLock<LogBuffer> = OnceLock::new();
    let logs = LOGS.get_or_init(|| {
        let logs = LogBuffer::default();
        tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .init();
        logs
    });
    let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    text.lines().filter(|l| l.contains(marker)).map(str::to_string).collect()
}

struct KaboomNode;

#[async_trait::async_trait]
impl Node for KaboomNode {
    fn type_name(&self) -> &'static str {
        "test.kaboom"
    }
    fn display_name(&self) -> &'static str {
        "Kaboom"
    }
    fn description(&self) -> &'static str {
        "Test-only node that always fails with \"kaboom\"."
    }
    fn category(&self) -> NodeCategory {
        NodeCategory::Action
    }
    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Err(NodeError::ExecutionFailed("kaboom".into()))
    }
}

fn registry() -> Arc<NodeRegistry> {
    let mut r = NodeRegistry::new();
    r8r::nodes::register_all(&mut r);
    r.register(Box::new(KaboomNode));
    Arc::new(r)
}

fn workflow(name: &str, second_node_type: &str) -> Workflow {
    let node = |id: &str, node_type: &str, parameters: serde_json::Value| NodeInstance {
        id: id.into(),
        node_type: node_type.into(),
        position: (0.0, 0.0),
        parameters,
        disabled: false,
        settings: Default::default(),
    };
    Workflow {
        id: Uuid::new_v4(),
        name: name.into(),
        active: false,
        nodes: vec![
            node("trigger", "core.manualTrigger", serde_json::json!({})),
            node("step", second_node_type, serde_json::json!({"fields": {"ok": true}})),
        ],
        connections: vec![Connection { from_node: "trigger".into(), from_output: 0, to_node: "step".into(), to_input: 0, error: false }],
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

async fn run(wf: Workflow) -> Uuid {
    let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap());
    storage.create_workflow(&wf).await.unwrap();
    let (events, _rx) = tokio::sync::broadcast::channel(16);
    let (started, handle) =
        r8r::execution_runner::start_execution(storage, events, registry(), wf, ExecutionMode::Manual, None, HashMap::new())
            .await
            .unwrap();
    handle.await.unwrap();
    started.id
}

#[tokio::test]
async fn logs_execution_start_and_successful_finish() {
    lines_mentioning(""); // install the subscriber before anything logs
    let name = format!("ok-{}", Uuid::new_v4());
    let execution_id = run(workflow(&name, "core.set")).await;

    let lines = lines_mentioning(&name);
    let started = lines.iter().find(|l| l.contains("execution started")).unwrap_or_else(|| panic!("{lines:?}"));
    assert!(started.contains("INFO") && started.contains(&execution_id.to_string()) && started.contains("Manual"), "{started}");
    let finished = lines.iter().find(|l| l.contains("execution finished")).unwrap_or_else(|| panic!("{lines:?}"));
    assert!(finished.contains("INFO") && finished.contains("Success") && finished.contains("duration_ms"), "{finished}");
}

#[tokio::test]
async fn logs_a_failed_execution_as_a_warning_with_the_error() {
    lines_mentioning("");
    let name = format!("boom-{}", Uuid::new_v4());
    run(workflow(&name, "test.kaboom")).await;

    let lines = lines_mentioning(&name);
    let failed = lines.iter().find(|l| l.contains("execution failed")).unwrap_or_else(|| panic!("{lines:?}"));
    assert!(failed.contains("WARN") && failed.contains("kaboom"), "{failed}");
}

#[tokio::test]
async fn api_requests_are_logged_at_debug_with_method_path_and_status() {
    lines_mentioning("");
    let storage = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    let state = AppState {
        storage: Arc::new(storage),
        registry: registry(),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
        open_registration: false,
    };
    let path = format!("/rest/executions/{}", Uuid::new_v4());
    let response = r8r::api::build_router(state)
        .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let lines = lines_mentioning(&path);
    let line = lines.iter().find(|l| l.contains("request")).unwrap_or_else(|| panic!("{lines:?}"));
    assert!(line.contains("DEBUG") && line.contains("GET") && line.contains("401") && line.contains("ms"), "{line}");
}

async fn post_json(app: axum::Router, path: &str, body: serde_json::Value) -> StatusCode {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

async fn fresh_app() -> axum::Router {
    let storage = SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap();
    r8r::api::build_router(AppState {
        storage: Arc::new(storage),
        registry: registry(),
        jwt_secret: "test-secret".into(),
        scheduler: Arc::new(r8r::scheduler::Scheduler::new().await.unwrap()),
        trigger_registry: Arc::new(r8r::trigger_registry::TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(16).0,
        open_registration: false,
    })
}

#[tokio::test]
async fn logs_login_outcomes_without_the_password() {
    lines_mentioning("");
    let app = fresh_app().await;
    let email = format!("{}@example.com", Uuid::new_v4());
    let creds = |pw: &str| serde_json::json!({"email": email, "password": pw});

    assert_eq!(post_json(app.clone(), "/rest/auth/register", creds("right-pass-123")).await, StatusCode::CREATED);
    assert_eq!(post_json(app.clone(), "/rest/auth/login", creds("wrong-pass-456")).await, StatusCode::UNAUTHORIZED);
    assert_eq!(post_json(app.clone(), "/rest/auth/login", creds("right-pass-123")).await, StatusCode::OK);
    let unknown = format!("{}@example.com", Uuid::new_v4());
    let unknown_body = serde_json::json!({"email": unknown, "password": "x"});
    assert_eq!(post_json(app, "/rest/auth/login", unknown_body).await, StatusCode::UNAUTHORIZED);

    let lines = lines_mentioning(&email);
    assert!(lines.iter().any(|l| l.contains("user registered") && l.contains("INFO")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("login failed") && l.contains("wrong password") && l.contains("WARN")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("login succeeded") && l.contains("INFO")), "{lines:?}");
    let unknown_lines = lines_mentioning(&unknown);
    assert!(unknown_lines.iter().any(|l| l.contains("login failed") && l.contains("unknown email")), "{unknown_lines:?}");
    let all = lines_mentioning("");
    assert!(!all.iter().any(|l| l.contains("right-pass-123") || l.contains("wrong-pass-456")), "password leaked into logs");
}

#[tokio::test]
async fn a_panicking_handler_returns_500_and_logs_the_panic() {
    lines_mentioning("");
    let marker = format!("boom-{}", Uuid::new_v4());
    let m = marker.clone();
    let app = r8r::api::catch_panics(axum::Router::new().route(
        "/boom",
        axum::routing::get(move || {
            let m = m.clone();
            async move {
                panic!("{m}");
                #[allow(unreachable_code)]
                ""
            }
        }),
    ));
    let response = app.oneshot(Request::builder().uri("/boom").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let lines = lines_mentioning(&marker);
    assert!(lines.iter().any(|l| l.contains("ERROR") && l.contains("panicked")), "{lines:?}");
}
