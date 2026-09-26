//! The n8n-compatible server (spec §6.3, §6.9–6.11): editor REST API,
//! public API, webhooks and forms, schedules, push, health and metrics, all
//! running workflows on [`crate::n8n::engine`].

pub mod activation;
pub mod auth;
pub mod public_api;
pub mod push;
pub mod rest;
pub mod runner;
pub mod webhooks;

use super::config::Config;
use super::node::Services;
use super::nodes::Registry;
use super::store::Store;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

pub use runner::{RunOutcome, RunRequest};

/// Shared server state.
pub struct N8n {
    pub config: Config,
    pub store: Store,
    pub registry: Arc<Registry>,
    pub services: Arc<Services>,
    pub jwt_secret: Vec<u8>,
    /// Production webhooks and forms of active workflows.
    pub webhooks: RwLock<Vec<webhooks::Registration>>,
    /// One-shot test webhooks registered by editor runs.
    pub test_webhooks: Mutex<Vec<webhooks::TestRegistration>>,
    /// Schedule tasks per active workflow.
    pub schedules: Mutex<HashMap<String, Vec<tokio::task::JoinHandle<()>>>>,
    /// Running executions, for stopping them.
    pub running: Mutex<HashMap<i64, super::engine::ExecuteOptions>>,
    pub inflight: AtomicUsize,
    pub inflight_done: tokio::sync::Notify,
    pub push: tokio::sync::broadcast::Sender<push::PushMessage>,
    pub metrics: Metrics,
    pub login_failures: Mutex<HashMap<String, Vec<Instant>>>,
    pub payload_limit: usize,
}

#[derive(Default)]
pub struct Metrics {
    pub success: AtomicU64,
    pub error: AtomicU64,
    pub canceled: AtomicU64,
    pub waiting: AtomicU64,
}

/// An error response in n8n's shape: `{ "code", "message" }`.
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR), message.into())
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, message)
    }
    pub fn forbidden() -> Self {
        Self::new(403, "Forbidden")
    }
    pub fn unauthorized() -> Self {
        Self::new(401, "Unauthorized")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"code": self.0.as_u16(), "message": self.1}))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!(error = %e, "request failed");
        Self::new(500, e.to_string())
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        anyhow::Error::from(e).into()
    }
}

pub type ApiResult<T = Response> = Result<T, ApiError>;

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name).ok().map(|v| v.to_ascii_lowercase()) {
        Some(v) if v == "true" || v == "1" => true,
        Some(v) if v == "false" || v == "0" => false,
        _ => default,
    }
}

impl N8n {
    /// Opens the database and builds the state. Fails when the encryption
    /// key cannot read existing credentials.
    pub async fn new(config: Config) -> anyhow::Result<Arc<Self>> {
        let store = Store::open(&config.database_url, &config.encryption_key).await?;
        // Like n8n, refuse to start with a key that can't read existing data.
        if let Some(first) = store.list_credentials().await?.into_iter().next() {
            if store.decrypt_credential(&first).await.is_err() {
                anyhow::bail!(
                    "Mismatching encryption keys. The N8N_ENCRYPTION_KEY in use cannot decrypt the stored credentials (e.g. \"{}\"). Start with the key they were saved with.",
                    first.name
                );
            }
        }
        use sha2::Digest;
        let jwt_secret = sha2::Sha256::digest(format!("n8n-auth:{}", config.encryption_key).as_bytes()).to_vec();
        let payload_limit = std::env::var("N8N_PAYLOAD_SIZE_MAX")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .map(|mib| (mib * 1024.0 * 1024.0) as usize)
            .unwrap_or(16 * 1024 * 1024);
        let (push, _) = tokio::sync::broadcast::channel(1024);
        let state = Arc::new_cyclic(|weak: &std::sync::Weak<N8n>| {
            let mut services = Services::new(config.clone(), Some(store.clone()));
            services.sub_workflows = Some(Arc::new(runner::SubRunner(weak.clone())));
            N8n {
                config,
                store,
                registry: Arc::new(Registry::default()),
                services: Arc::new(services),
                jwt_secret,
                webhooks: RwLock::new(Vec::new()),
                test_webhooks: Mutex::new(Vec::new()),
                schedules: Mutex::new(HashMap::new()),
                running: Mutex::new(HashMap::new()),
                inflight: AtomicUsize::new(0),
                inflight_done: tokio::sync::Notify::new(),
                push,
                metrics: Metrics::default(),
                login_failures: Mutex::new(HashMap::new()),
                payload_limit,
            }
        });
        Ok(state)
    }

    /// Start-up work: crash recovery, reactivation, background timers.
    pub async fn start_background(self: &Arc<Self>) {
        match self.store.mark_crashed().await {
            Ok(n) if n > 0 => tracing::warn!("marked {n} execution(s) interrupted by a previous shutdown as crashed"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "crash recovery failed"),
        }
        match self.store.workflow_rows().await {
            Ok(rows) => {
                for row in rows.into_iter().filter(|r| r.active) {
                    let id = row.data["id"].as_str().unwrap_or_default().to_string();
                    if let Err(e) = activation::register(self, &row.data).await {
                        tracing::warn!(workflowId = %id, error = %e.1, "could not reactivate workflow");
                    }
                }
            }
            Err(e) => tracing::error!(error = %e, "could not load workflows for reactivation"),
        }
        // Timed waits resume when due; old executions are pruned.
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(1000));
            let mut ticks = 0u64;
            loop {
                tick.tick().await;
                let Some(n8n) = me.upgrade() else { return };
                let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                if let Ok(due) = n8n.store.due_waiting(&now).await {
                    for id in due {
                        let n = n8n.clone();
                        tokio::spawn(async move {
                            if let Err(e) = runner::resume(&n, id, None).await {
                                tracing::warn!(executionId = id, error = %e.1, "could not resume waiting execution");
                            }
                        });
                    }
                }
                ticks += 1;
                if ticks % 3600 == 1 && env_bool("EXECUTIONS_DATA_PRUNE", true) {
                    let cutoff = chrono::Utc::now() - chrono::Duration::hours(n8n.config.executions_data_max_age_hours as i64);
                    let _ = n8n.store.prune_executions(&cutoff.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).await;
                }
            }
        });
    }

    /// Waits for running executions to finish, up to `timeout`.
    pub async fn drain(&self, timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let notified = self.inflight_done.notified();
            if self.inflight.load(Ordering::SeqCst) == 0 {
                return;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                tracing::warn!("{} execution(s) still running at shutdown", self.inflight.load(Ordering::SeqCst));
                return;
            }
        }
    }

    pub fn resume_signature(&self, execution_id: &str) -> String {
        use hmac::Mac;
        let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(self.config.encryption_key.as_bytes()).expect("any key length");
        mac.update(format!("resume:{execution_id}").as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }
}

/// All n8n routes.
pub fn router(state: Arc<N8n>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/healthz/readiness", get(readiness))
        .route("/metrics", get(metrics))
        .merge(rest::router())
        .merge(public_api::router())
        .merge(webhooks::router())
        .with_state(state)
}

async fn readiness(axum::extract::State(n8n): axum::extract::State<Arc<N8n>>) -> Response {
    match n8n.store.user_count().await {
        Ok(_) => Json(json!({"status": "ok"})).into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"status": "error"}))).into_response(),
    }
}

fn cpu_seconds() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else { return 0.0 };
    let after = stat.rsplit(')').next().unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    let ticks = |i: usize| fields.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
    // utime and stime are fields 14 and 15 of stat (11 and 12 after the name).
    (ticks(11) + ticks(12)) / 100.0
}

fn resident_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok()))
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

async fn metrics(axum::extract::State(n8n): axum::extract::State<Arc<N8n>>) -> Response {
    if !env_bool("N8N_METRICS", false) {
        return (StatusCode::NOT_FOUND, "Cannot GET /metrics").into_response();
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut parts = version.split('.');
    let (major, minor, patch) = (parts.next().unwrap_or("0"), parts.next().unwrap_or("0"), parts.next().unwrap_or("0"));
    let active = n8n.store.active_workflow_count().await.unwrap_or(0);
    let m = &n8n.metrics;
    let body = format!(
        "# HELP n8n_version_info n8n version info.\n# TYPE n8n_version_info gauge\nn8n_version_info{{version=\"v{version}\",major=\"{major}\",minor=\"{minor}\",patch=\"{patch}\"}} 1\n\
# HELP n8n_active_workflow_count Total number of active workflows.\n# TYPE n8n_active_workflow_count gauge\nn8n_active_workflow_count {active}\n\
# HELP n8n_process_cpu_seconds_total Total user and system CPU time spent in seconds.\n# TYPE n8n_process_cpu_seconds_total counter\nn8n_process_cpu_seconds_total {cpu}\n\
# HELP n8n_process_resident_memory_bytes Resident memory size in bytes.\n# TYPE n8n_process_resident_memory_bytes gauge\nn8n_process_resident_memory_bytes {rss}\n\
# HELP n8n_workflow_executions_total Finished workflow executions by status.\n# TYPE n8n_workflow_executions_total counter\n\
n8n_workflow_executions_total{{status=\"success\"}} {s}\nn8n_workflow_executions_total{{status=\"error\"}} {e}\nn8n_workflow_executions_total{{status=\"canceled\"}} {c}\nn8n_workflow_executions_total{{status=\"waiting\"}} {w}\n",
        cpu = cpu_seconds(),
        rss = resident_bytes(),
        s = m.success.load(Ordering::Relaxed),
        e = m.error.load(Ordering::Relaxed),
        c = m.canceled.load(Ordering::Relaxed),
        w = m.waiting.load(Ordering::Relaxed),
    );
    ([("content-type", "text/plain; version=0.0.4; charset=utf-8")], body).into_response()
}

/// `{ "data": ... }`, the editor API's envelope.
pub fn data(v: Value) -> Response {
    Json(json!({ "data": v })).into_response()
}

/// Workflow JSON as the APIs return it: stored JSON plus the columns.
pub fn workflow_json(row: &super::store_ext::WorkflowRow, tags: Vec<Value>) -> Value {
    let mut w = row.data.clone();
    w["active"] = json!(row.active);
    w["createdAt"] = json!(row.created_at);
    w["updatedAt"] = json!(row.updated_at);
    if w.get("settings").is_none() {
        w["settings"] = json!({});
    }
    if w.get("connections").is_none() {
        w["connections"] = json!({});
    }
    if w.get("staticData").is_none() {
        w["staticData"] = Value::Null;
    }
    w["tags"] = Value::Array(tags);
    if let Some(p) = &row.project_id {
        w["shared"] = json!([{"projectId": p, "role": "workflow:owner"}]);
    }
    w
}
