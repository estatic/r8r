//! Per-scenario state. Each scenario gets a fresh scratch directory (used as
//! `N8N_USER_FOLDER`, so the database lives there too) and its own
//! processes, which are killed when the world is dropped.

use crate::support::process::{self, CliOutput, ServerProcess};
use crate::support::workflow::WorkflowSpec;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Auth {
    None,
    /// Cookie session of the named user ("owner", or a member's email).
    Session(String),
    /// Public API key of the named user.
    ApiKey(String),
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub method: String,
    pub url: String,
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub body: String,
    pub elapsed: Duration,
}

impl HttpResponse {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("response body is not JSON ({e}):\n{}", self.describe()))
    }

    pub fn describe(&self) -> String {
        let mut body = self.body.clone();
        if body.len() > 2000 {
            body.truncate(2000);
            body.push_str("…");
        }
        format!("{} {} -> {} in {:?}\n{}", self.method, self.url, self.status, self.elapsed, body)
    }
}

/// Expression under test (see steps/expressions.rs).
#[derive(Debug, Default, Clone)]
pub struct ExprState {
    pub input_items: Option<Value>,
    pub expression: Option<String>,
    pub settings: serde_json::Map<String, Value>,
}

#[derive(cucumber::World)]
#[world(init = Self::new)]
pub struct R8rWorld {
    pub dir: tempfile::TempDir,
    /// Scenario-level env overrides and removals, applied over
    /// `process::base_env` for every process the scenario spawns.
    pub env: BTreeMap<String, String>,
    pub unset_env: BTreeSet<String>,
    pub workflows: Vec<WorkflowSpec>,
    pub current: Option<usize>,
    pub cli: Option<CliOutput>,
    /// Last execution: `r8r execute --rawOutput` output or an execution
    /// fetched from the API. Both carry `status` and `data.resultData`.
    pub run: Option<Value>,
    pub servers: BTreeMap<String, ServerProcess>,
    pub port: Option<u16>,
    pub http: reqwest::Client,
    pub auth: Auth,
    /// Cookie header value per user label.
    pub sessions: HashMap<String, String>,
    pub api_keys: HashMap<String, String>,
    pub response: Option<HttpResponse>,
    /// `%{NAME}` placeholders.
    pub vars: HashMap<String, String>,
    pub mock: Option<wiremock::MockServer>,
    pub push_messages: Arc<Mutex<Vec<Value>>>,
    pub push_task: Option<tokio::task::JoinHandle<()>>,
    pub expr: ExprState,
    pub load: Vec<(u16, Duration)>,
    /// Headers added to the next request only.
    pub next_headers: Vec<(String, String)>,
}

impl std::fmt::Debug for R8rWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("R8rWorld")
            .field("dir", &self.dir.path())
            .field("env", &self.env)
            .field("workflows", &self.workflows.iter().map(|w| &w.name).collect::<Vec<_>>())
            .field("servers", &self.servers.keys().collect::<Vec<_>>())
            .field("vars", &self.vars)
            .field("last_response", &self.response.as_ref().map(|r| (r.method.clone(), r.url.clone(), r.status)))
            .finish()
    }
}

impl Drop for R8rWorld {
    fn drop(&mut self) {
        if let Some(task) = self.push_task.take() {
            task.abort();
        }
        // wiremock's MockServer blocks on an async verify() in its Drop; on
        // the runtime's own thread that spins forever, so drop it elsewhere.
        if let Some(mock) = self.mock.take() {
            std::thread::spawn(move || drop(mock));
        }
        // ServerProcess children are kill_on_drop.
    }
}

impl R8rWorld {
    pub fn new() -> Self {
        Self {
            dir: tempfile::Builder::new().prefix("r8r-bdd-").tempdir().expect("temp dir"),
            env: BTreeMap::new(),
            unset_env: BTreeSet::new(),
            workflows: Vec::new(),
            current: None,
            cli: None,
            run: None,
            servers: BTreeMap::new(),
            port: None,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
            auth: Auth::None,
            sessions: HashMap::new(),
            api_keys: HashMap::new(),
            response: None,
            vars: HashMap::new(),
            mock: None,
            push_messages: Arc::new(Mutex::new(Vec::new())),
            push_task: None,
            expr: ExprState::default(),
            load: Vec::new(),
            next_headers: Vec::new(),
        }
    }

    // ---- placeholders --------------------------------------------------

    /// Replaces `%{NAME}` with scenario variables. Built-ins: `SERVER_URL`,
    /// `MOCK_URL`, `USER_FOLDER`, plus everything steps remembered
    /// (`WORKFLOW_ID`, `WORKFLOW_ID:<name>`, `CREDENTIAL_ID:<name>`,
    /// `EXECUTION_ID`, ...). An unknown name fails loudly.
    pub fn expand(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("%{") {
            out.push_str(&rest[..start]);
            let end = rest[start..].find('}').map(|e| e + start).unwrap_or_else(|| panic!("unclosed %{{ in: {text}"));
            let name = &rest[start + 2..end];
            out.push_str(&self.var(name));
            rest = &rest[end + 1..];
        }
        out.push_str(rest);
        out
    }

    pub fn var(&self, name: &str) -> String {
        match name {
            "SERVER_URL" => return self.server_url(),
            "MOCK_URL" => return self.mock.as_ref().expect("no mock service: add 'Given a mock HTTP service'").uri(),
            "USER_FOLDER" => return self.dir.path().display().to_string(),
            _ => {}
        }
        if let Some(v) = self.vars.get(name) {
            return v.clone();
        }
        let mut known: Vec<&String> = self.vars.keys().collect();
        known.sort();
        panic!("unknown placeholder %{{{name}}}; known: SERVER_URL, MOCK_URL, USER_FOLDER, {known:?}")
    }

    // ---- workflows -----------------------------------------------------

    pub fn add_workflow(&mut self, spec: WorkflowSpec) {
        if let Some(i) = self.workflows.iter().position(|w| w.name == spec.name) {
            self.workflows[i] = spec;
            self.current = Some(i);
        } else {
            self.workflows.push(spec);
            self.current = Some(self.workflows.len() - 1);
        }
    }

    pub fn wf(&mut self) -> &mut WorkflowSpec {
        let i = self.current.expect("no workflow defined yet in this scenario");
        &mut self.workflows[i]
    }

    pub fn wf_named(&mut self, name: &str) -> &mut WorkflowSpec {
        let i = self
            .workflows
            .iter()
            .position(|w| w.name == name)
            .unwrap_or_else(|| panic!("no workflow named \"{name}\" in this scenario"));
        self.current = Some(i);
        &mut self.workflows[i]
    }

    /// The workflow JSON with placeholders expanded.
    pub fn workflow_json(&self, spec: &WorkflowSpec) -> Value {
        let text = serde_json::to_string(&spec.to_json()).unwrap();
        serde_json::from_str(&self.expand(&text)).unwrap()
    }

    pub fn workflow_public_json(&self, spec: &WorkflowSpec) -> Value {
        let text = serde_json::to_string(&spec.to_public_api_json()).unwrap();
        serde_json::from_str(&self.expand(&text)).unwrap()
    }

    // ---- processes -----------------------------------------------------

    pub fn process_env(&self) -> BTreeMap<String, String> {
        let mut env = process::base_env(self.dir.path());
        for (k, v) in &self.env {
            env.insert(k.clone(), self.expand(v));
        }
        for k in &self.unset_env {
            env.remove(k);
        }
        env
    }

    pub async fn cli(&mut self, args: &[String], timeout: Duration) -> CliOutput {
        let env = self.process_env();
        let out = process::run_cli(args, &env, self.dir.path(), None, timeout).await;
        self.cli = Some(out.clone());
        out
    }

    pub fn server(&self) -> &ServerProcess {
        self.servers.get("main").expect("no r8r server running: add 'Given a running r8r server'")
    }

    pub fn server_url(&self) -> String {
        self.server().base_url()
    }

    // ---- HTTP ----------------------------------------------------------

    pub fn url(&self, path: &str) -> String {
        let path = self.expand(path);
        if path.starts_with("http://") || path.starts_with("https://") {
            path
        } else {
            format!("{}{}", self.server_url(), path)
        }
    }

    pub async fn request(
        &mut self,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<String>,
    ) -> HttpResponse {
        let url = self.url(path);
        let method_parsed = reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).expect("HTTP method");
        let mut req = self.http.request(method_parsed, &url);
        match &self.auth {
            Auth::None => {}
            Auth::Session(user) => {
                let cookie = self.sessions.get(user).unwrap_or_else(|| panic!("no session for {user}"));
                req = req.header("cookie", cookie);
            }
            Auth::ApiKey(user) => {
                let key = self.api_keys.get(user).unwrap_or_else(|| panic!("no API key for {user}"));
                req = req.header("X-N8N-API-KEY", key);
            }
        }
        let mut has_content_type = false;
        let pending = std::mem::take(&mut self.next_headers);
        for (k, v) in headers.iter().chain(&pending) {
            has_content_type |= k.eq_ignore_ascii_case("content-type");
            req = req.header(k.as_str(), self.expand(v));
        }
        if let Some(body) = body {
            if !has_content_type {
                req = req.header("content-type", "application/json");
            }
            req = req.body(self.expand(&body));
        }
        let started = std::time::Instant::now();
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => panic!("{method} {url} failed: {e}\n--- server log (tail) ---\n{}", self.server_log_tail()),
        };
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        let r = HttpResponse { method: method.to_uppercase(), url, status, headers, body, elapsed: started.elapsed() };
        self.response = Some(r.clone());
        r
    }

    pub fn response(&self) -> &HttpResponse {
        self.response.as_ref().expect("no HTTP response yet")
    }

    pub fn server_log_tail(&self) -> String {
        self.servers.get("main").map(|s| process::tail(&s.log(), 30)).unwrap_or_default()
    }

    // ---- executions ----------------------------------------------------

    pub fn run(&self) -> &Value {
        match &self.run {
            Some(r) => r,
            None => match &self.cli {
                Some(cli) => panic!("no execution result was captured; last CLI call:\n{}", cli.describe()),
                None => panic!("no execution has run in this scenario"),
            },
        }
    }

    pub fn run_data(&self) -> &serde_json::Map<String, Value> {
        let run = self.run();
        run.pointer("/data/resultData/runData")
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("execution has no data.resultData.runData:\n{}", pretty(run)))
    }

    /// Task data (one entry per run) of `node`.
    pub fn node_runs(&self, node: &str) -> &Vec<Value> {
        self.run_data().get(node).and_then(Value::as_array).unwrap_or_else(|| {
            panic!(
                "node \"{node}\" has no run data; executed nodes: {:?}\nexecution error: {}",
                self.run_data().keys().collect::<Vec<_>>(),
                self.run().pointer("/data/resultData/error").map(pretty).unwrap_or_else(|| "none".into())
            )
        })
    }

    /// `json` of each item on `output` of `node`'s run `run_index` (last
    /// run when `None`).
    pub fn node_output(&self, node: &str, output: usize, run_index: Option<usize>) -> Vec<Value> {
        self.node_output_items(node, output, run_index).iter().map(|i| i["json"].clone()).collect()
    }

    pub fn node_output_items(&self, node: &str, output: usize, run_index: Option<usize>) -> Vec<Value> {
        let runs = self.node_runs(node);
        let run = match run_index {
            Some(i) => runs.get(i).unwrap_or_else(|| panic!("node \"{node}\" ran {} time(s), no run {i}", runs.len())),
            None => runs.last().expect("empty run list"),
        };
        if let Some(err) = run.get("error").filter(|e| !e.is_null()) {
            if run.pointer("/data/main").is_none() {
                panic!("node \"{node}\" errored instead of producing output: {}", pretty(err));
            }
        }
        run.pointer("/data/main")
            .and_then(Value::as_array)
            .and_then(|outputs| outputs.get(output))
            .map(|items| items.as_array().cloned().unwrap_or_default())
            .unwrap_or_default()
    }
}

pub fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).unwrap();
    if s.len() > 4000 {
        s.truncate(4000);
        s.push_str("\n…(truncated)");
    }
    s
}
