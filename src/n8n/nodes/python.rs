//! Python for the Code node (spec §6.7), run like n8n 2.x's native Python
//! runner: a separate `python3` process per run with the items passed as
//! JSON, imports limited to `N8N_RUNNERS_STDLIB_ALLOW` /
//! `N8N_RUNNERS_EXTERNAL_ALLOW`, and the `N8N_RUNNERS_TASK_TIMEOUT` limit.
//! Items support both `item["json"]` and `item.json` access.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult};
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const RUNNER: &str = r#"
import sys, json, builtins, io, traceback

class Obj(dict):
    def __getattr__(self, k):
        try:
            return self[k]
        except KeyError:
            raise AttributeError(k)
    def __setattr__(self, k, v):
        self[k] = v

def wrap(v):
    if isinstance(v, dict):
        return Obj({k: wrap(x) for k, x in v.items()})
    if isinstance(v, list):
        return [wrap(x) for x in v]
    return v

req = json.loads(sys.stdin.read())
allowed = set(req["allow"])
real_import = builtins.__import__

def guarded(name, globals=None, locals=None, fromlist=(), level=0):
    root = name.split(".")[0]
    if "*" not in allowed and root not in allowed:
        raise ImportError("Import of '%s' is not allowed. Allow it with N8N_RUNNERS_STDLIB_ALLOW or N8N_RUNNERS_EXTERNAL_ALLOW" % name)
    return real_import(name, globals, locals, fromlist, level)

src = "def __r8r_user():\n" + "\n".join("    " + line for line in (req["code"] or "return []").splitlines()) + "\n"
out = sys.stdout
logs = io.StringIO()
sys.stdout = logs

def run(scope):
    exec(compile(src, "<code>", "exec"), scope)
    return scope["__r8r_user"]()

def user_frames(e):
    frames = []
    tb = e.__traceback__
    while tb is not None:
        code = tb.tb_frame.f_code
        if code.co_filename == "<code>":
            name = "main code" if code.co_name == "__r8r_user" else code.co_name
            frames.append([tb.tb_lineno - 1, name])
        tb = tb.tb_next
    return frames[::-1]

def user_line(e):
    line = None
    tb = e.__traceback__
    while tb is not None:
        if tb.tb_frame.f_code.co_filename == "<code>":
            line = tb.tb_lineno - 1
        tb = tb.tb_next
    return line

try:
    items = wrap(req["items"])
    builtins.__import__ = guarded
    extra = {k: wrap(v) for k, v in (req.get("globals") or {}).items()}
    if req["each"]:
        result = []
        for i, it in enumerate(items):
            result.append(run({"_item": it, "_items": items, "__builtins__": builtins, **extra}))
    else:
        result = run({"_items": items, "__builtins__": builtins, **extra})
    builtins.__import__ = real_import
    out.write(json.dumps({"ok": result, "console": logs.getvalue().splitlines()}, default=str))
except BaseException as e:
    builtins.__import__ = real_import
    out.write(json.dumps({"error": "%s: %s" % (type(e).__name__, e), "line": user_line(e), "trace": user_frames(e), "console": logs.getvalue().splitlines()}))
"#;

fn allow_list() -> Vec<String> {
    ["N8N_RUNNERS_STDLIB_ALLOW", "N8N_RUNNERS_EXTERNAL_ALLOW"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Why a Python run failed: a message, and maybe a hint for fixing it.
#[derive(Debug)]
pub struct PythonError {
    pub message: String,
    pub hint: Option<String>,
    /// For an error in the code: the error alone (`message` adds the line),
    /// and the frames in the code, nearest first, as (line, function).
    pub error: String,
    pub trace: Vec<(u64, String)>,
}

impl PythonError {
    fn new(message: impl Into<String>, hint: Option<String>) -> Self {
        let message = message.into();
        Self { error: message.clone(), message, hint, trace: Vec::new() }
    }
}

/// Runs Python code over the node's input. `each`: once per item
/// (`_item`), else once for all (`_items`). Returns one result per run.
pub async fn run(ctx: &ExecCtx<'_>, code: &str, each: bool) -> NodeResult<(Vec<Value>, Vec<String>)> {
    run_process(code, ctx.input(), each, None, ctx.config().runners_task_timeout_secs).await.map_err(|e| {
        let error = NodeError::new(e.message);
        match e.hint {
            Some(hint) => error.describe(hint),
            None => error,
        }
    })
}

/// The engine-independent runner: `python3` in a subprocess, imports
/// limited to the allow lists, killed after `timeout_secs`. `globals`:
/// more names for the code (the legacy Code node's `items`, `_json`, ...).
pub async fn run_process(code: &str, items: impl serde::Serialize, each: bool, globals: Option<Value>, timeout_secs: u64) -> Result<(Vec<Value>, Vec<String>), PythonError> {
    let python = std::env::var("R8R_PYTHON_PATH").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "python3".into());
    let request = json!({"code": code, "items": items, "each": each, "allow": allow_list(), "globals": globals});
    let mut child = tokio::process::Command::new(&python)
        .arg("-I")
        .arg("-c")
        .arg(RUNNER)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| PythonError::new(format!("The Python runner is not available: could not start \"{python}\" ({e})"), Some("Install Python 3, or point R8R_PYTHON_PATH at it".into())))?;
    let mut stdin = child.stdin.take().expect("piped");
    let body = serde_json::to_vec(&request).unwrap();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&body).await;
    });
    let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| PythonError::new(format!("The Python runner failed: {e}"), None))?,
        Err(_) => {
            return Err(PythonError::new(
                format!("Task execution timed out after {timeout_secs} seconds"),
                Some("The Code node took too long and was stopped. Raise N8N_RUNNERS_TASK_TIMEOUT if this is expected.".into()),
            ));
        }
    };
    let _ = writer.await;
    let reply: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        PythonError::new("The Python runner stopped unexpectedly", Some(stderr.lines().last().unwrap_or("no output").to_string()))
    })?;
    let console: Vec<String> = reply["console"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect();
    if let Some(err) = reply["error"].as_str() {
        let message = match reply["line"].as_u64() {
            Some(line) => format!("{err} [line {line}]"),
            None => err.to_string(),
        };
        let trace = reply["trace"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| Some((f.get(0)?.as_u64()?, f.get(1)?.as_str()?.to_string())))
            .collect();
        return Err(PythonError { message, hint: None, error: err.to_string(), trace });
    }
    let results = if each { reply["ok"].as_array().cloned().unwrap_or_default() } else { vec![reply["ok"].clone()] };
    Ok((results, console))
}
