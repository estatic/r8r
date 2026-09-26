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
    if req["each"]:
        result = []
        for i, it in enumerate(items):
            result.append(run({"_item": it, "_items": items, "__builtins__": builtins}))
    else:
        result = run({"_items": items, "__builtins__": builtins})
    builtins.__import__ = real_import
    out.write(json.dumps({"ok": result, "console": logs.getvalue().splitlines()}, default=str))
except BaseException as e:
    builtins.__import__ = real_import
    out.write(json.dumps({"error": "%s: %s" % (type(e).__name__, e), "line": user_line(e), "console": logs.getvalue().splitlines()}))
"#;

fn allow_list() -> Vec<String> {
    ["N8N_RUNNERS_STDLIB_ALLOW", "N8N_RUNNERS_EXTERNAL_ALLOW"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .flat_map(|v| v.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Runs Python code over the node's input. `each`: once per item
/// (`_item`), else once for all (`_items`). Returns one result per run.
pub async fn run(ctx: &ExecCtx<'_>, code: &str, each: bool) -> NodeResult<(Vec<Value>, Vec<String>)> {
    let python = std::env::var("R8R_PYTHON_PATH").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "python3".into());
    let timeout_secs = ctx.config().runners_task_timeout_secs;
    let request = json!({"code": code, "items": ctx.input(), "each": each, "allow": allow_list()});
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
        .map_err(|e| NodeError::new(format!("The Python runner is not available: could not start \"{python}\" ({e})")).describe("Install Python 3, or point R8R_PYTHON_PATH at it"))?;
    let mut stdin = child.stdin.take().expect("piped");
    let body = serde_json::to_vec(&request).unwrap();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&body).await;
    });
    let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
        Ok(r) => r.map_err(|e| NodeError::new(format!("The Python runner failed: {e}")))?,
        Err(_) => {
            return Err(NodeError::new(format!("Task execution timed out after {timeout_secs} seconds"))
                .describe("The Code node took too long and was stopped. Raise N8N_RUNNERS_TASK_TIMEOUT if this is expected."));
        }
    };
    let _ = writer.await;
    let reply: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        NodeError::new("The Python runner stopped unexpectedly").describe(stderr.lines().last().unwrap_or("no output").to_string())
    })?;
    let console: Vec<String> = reply["console"].as_array().into_iter().flatten().filter_map(|l| l.as_str().map(String::from)).collect();
    if let Some(err) = reply["error"].as_str() {
        let message = match reply["line"].as_u64() {
            Some(line) => format!("{err} [line {line}]"),
            None => err.to_string(),
        };
        return Err(NodeError::new(message));
    }
    let results = if each { reply["ok"].as_array().cloned().unwrap_or_default() } else { vec![reply["ok"].clone()] };
    Ok((results, console))
}

