//! The node contract (spec §6.6): a node type receives its input items and
//! resolves its parameters per item through [`ExecCtx`].

use super::config::Config;
use super::expr::{contains_expression, Evaluator, ExprError};
use super::store::Store;
use super::types::{Item, NodeOutput};
use super::workflow::{Node, OnError, Workflow};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Cli,
    Manual,
    Webhook,
    Trigger,
    Integrated,
    Retry,
    Error,
}

impl Mode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Cli => "cli",
            Mode::Manual => "manual",
            Mode::Webhook => "webhook",
            Mode::Trigger => "trigger",
            Mode::Integrated => "integrated",
            Mode::Retry => "retry",
            Mode::Error => "error",
        }
    }
}

/// A node failure, serialised the way n8n stores it in run data.
#[derive(Debug, Clone)]
pub struct NodeError {
    pub message: String,
    pub description: Option<String>,
    pub item_index: Option<usize>,
    pub http_code: Option<String>,
    pub name: &'static str,
    /// Set when the error should end the run whatever the node's `onError`.
    pub fatal: bool,
}

impl NodeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), description: None, item_index: None, http_code: None, name: "NodeOperationError", fatal: false }
    }

    pub fn api(message: impl Into<String>, http_code: Option<u16>, description: Option<String>) -> Self {
        Self { http_code: http_code.map(|c| c.to_string()), description, name: "NodeApiError", ..Self::new(message) }
    }

    pub fn at(mut self, item: usize) -> Self {
        self.item_index.get_or_insert(item);
        self
    }

    pub fn describe(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn to_json(&self, node: &Node) -> Value {
        let mut v = json!({
            "message": self.message,
            "name": self.name,
            "level": "warning",
            "functionality": "regular",
            "timestamp": super::types::now_ms(),
            "context": self.item_index.map(|i| json!({"itemIndex": i})).unwrap_or_else(|| json!({})),
            "node": {"name": node.name, "type": node.node_type, "typeVersion": node.type_version, "parameters": node.parameters},
            "description": self.description,
        });
        if let Some(code) = &self.http_code {
            v["httpCode"] = json!(code);
        }
        v
    }
}

impl From<ExprError> for NodeError {
    fn from(e: ExprError) -> Self {
        Self { name: "ExpressionError", description: e.description, ..Self::new(e.message) }
    }
}

pub type NodeResult<T> = Result<T, NodeError>;

/// Shared services for node runs.
pub struct Services {
    pub config: Config,
    pub store: Option<Store>,
    pub http: reqwest::Client,
}

impl Services {
    pub fn new(config: Config, store: Option<Store>) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(21))
            .build()
            .expect("HTTP client");
        Self { config, store, http }
    }
}

/// Execution-scoped mutable state shared by all nodes of one run.
#[derive(Default)]
pub struct RunState {
    /// Per-node state that survives between runs of the node within one
    /// execution (e.g. Loop Over Items' remaining items).
    pub node_state: Map<String, Value>,
    pub custom_data: Map<String, Value>,
    pub static_data: Value,
    pub console: Vec<String>,
}

pub struct ExecCtx<'a> {
    pub node: &'a Node,
    pub workflow: &'a Workflow,
    /// Main inputs, by input index.
    pub inputs: Vec<Vec<Item>>,
    pub run_index: usize,
    pub mode: Mode,
    pub execution_id: String,
    pub services: &'a Services,
    pub run: Arc<Mutex<RunState>>,
    /// Builds the JSON the expression VM sees (see `js/prelude.js`).
    pub expr_data: Box<dyn Fn() -> Value + Send + Sync + 'a>,
    /// Items that failed while `onError` lets the run continue.
    pub error_items: Vec<Item>,
    evaluator: OnceLock<Result<Evaluator, ExprError>>,
}

impl<'a> ExecCtx<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node: &'a Node,
        workflow: &'a Workflow,
        inputs: Vec<Vec<Item>>,
        run_index: usize,
        mode: Mode,
        execution_id: String,
        services: &'a Services,
        run: Arc<Mutex<RunState>>,
        expr_data: Box<dyn Fn() -> Value + Send + Sync + 'a>,
    ) -> Self {
        Self { node, workflow, inputs, run_index, mode, execution_id, services, run, expr_data, error_items: Vec::new(), evaluator: OnceLock::new() }
    }

    /// Main input 0.
    pub fn input(&self) -> &[Item] {
        self.inputs.first().map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn config(&self) -> &Config {
        &self.services.config
    }

    /// The raw (unresolved) parameter at a dotted path.
    pub fn raw_param(&self, path: &str) -> Option<&Value> {
        let mut v = &self.node.parameters;
        for part in path.split('.').filter(|p| !p.is_empty()) {
            v = match v {
                Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
                other => other.get(part)?,
            };
        }
        Some(v)
    }

    pub fn evaluator(&self) -> NodeResult<&Evaluator> {
        let r = self.evaluator.get_or_init(|| {
            Evaluator::new(
                &(self.expr_data)(),
                self.workflow.setting_str("timezone").unwrap_or(&self.services.config.timezone),
                Duration::from_millis(self.services.config.expression_timeout_ms),
            )
        });
        r.as_ref().map_err(|e| NodeError::from(e.clone()))
    }

    /// The parameter at `path`, with expressions resolved for item `item`.
    pub fn param(&self, path: &str, item: usize) -> NodeResult<Value> {
        let Some(raw) = self.raw_param(path) else { return Ok(Value::Null) };
        if !contains_expression(raw) {
            return Ok(raw.clone());
        }
        let ev = self.evaluator()?;
        ev.set_item(item).map_err(|e| NodeError::from(e).at(item))?;
        ev.resolve(raw).map_err(|e| NodeError::from(e).at(item))
    }

    pub fn param_str(&self, path: &str, item: usize, default: &str) -> NodeResult<String> {
        Ok(match self.param(path, item)? {
            Value::Null => default.to_string(),
            Value::String(s) => s,
            other => other.to_string(),
        })
    }

    pub fn param_bool(&self, path: &str, item: usize, default: bool) -> NodeResult<bool> {
        Ok(match self.param(path, item)? {
            Value::Bool(b) => b,
            Value::String(s) => s == "true",
            _ => default,
        })
    }

    pub fn param_f64(&self, path: &str, item: usize, default: f64) -> NodeResult<f64> {
        Ok(match self.param(path, item)? {
            Value::Number(n) => n.as_f64().unwrap_or(default),
            Value::String(s) => s.trim().parse().unwrap_or(default),
            _ => default,
        })
    }

    /// Whether a failing item should become an error item instead of failing
    /// the node (`onError` continue*, or legacy `continueOnFail`).
    pub fn continue_on_fail(&self) -> bool {
        self.node.on_error != OnError::StopWorkflow
    }

    /// Records a failed item (when continuing on fail).
    pub fn push_error_item(&mut self, error: &NodeError, item: usize) {
        let mut json = Map::new();
        json.insert("error".into(), json!(error.message));
        self.error_items.push(Item::new(json).paired(item));
    }

    /// Decrypted data of the credential the node has for `cred_type`.
    pub async fn credentials(&self, cred_type: &str) -> NodeResult<(String, Value)> {
        let reference = self
            .node
            .credentials
            .get(cred_type)
            .ok_or_else(|| NodeError::new(format!("Node \"{}\" does not have any credentials of type \"{cred_type}\" set", self.node.name)))?;
        let id = reference["id"].as_str().map(String::from).unwrap_or_default();
        let store = self.services.store.as_ref().ok_or_else(|| NodeError::new("Credentials are not available in this mode"))?;
        let record = match store.get_credential(&id).await.map_err(|e| NodeError::new(e.to_string()))? {
            Some(r) => r,
            None => {
                // n8n falls back to a unique credential with the same name.
                let name = reference["name"].as_str().unwrap_or_default();
                let all = store.list_credentials().await.map_err(|e| NodeError::new(e.to_string()))?;
                let mut matching = all.into_iter().filter(|c| c.name == name && c.cred_type == cred_type);
                match (matching.next(), matching.next()) {
                    (Some(c), None) => c,
                    _ => return Err(NodeError::new(format!("Credential with ID \"{id}\" does not exist for type \"{cred_type}\"."))),
                }
            }
        };
        let data = store.decrypt_credential(&record).await.map_err(|e| NodeError::new(e.to_string()))?;
        Ok((record.id, data))
    }

    pub fn log(&self, message: String) {
        self.run.lock().unwrap().console.push(message);
    }
}

#[async_trait::async_trait]
pub trait NodeType: Send + Sync {
    fn type_name(&self) -> &'static str;

    /// Trigger nodes start executions; in a run they emit their data.
    fn is_trigger(&self) -> bool {
        false
    }

    /// Number of main outputs for these parameters (before any error output).
    fn outputs(&self, node: &Node) -> usize {
        let _ = node;
        1
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput>;
}
