//! Triggers and flow nodes that need little logic.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Map, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![
        Box::new(Trigger("n8n-nodes-base.manualTrigger")),
        Box::new(Trigger("n8n-nodes-base.executeWorkflowTrigger")),
        Box::new(Trigger("n8n-nodes-base.errorTrigger")),
        Box::new(Trigger("n8n-nodes-base.start")),
        Box::new(NoOp),
        Box::new(StopAndError),
        Box::new(Wait),
        Box::new(ExecuteCommand),
    ]
}

/// A trigger inside a run emits its input: one empty item when the
/// execution starts here, or the data the execution was started with.
struct Trigger(&'static str);

#[async_trait::async_trait]
impl NodeType for Trigger {
    fn type_name(&self) -> &'static str {
        self.0
    }
    fn is_trigger(&self) -> bool {
        true
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        Ok(vec![items.into_iter().enumerate().map(|(i, it)| it.paired(i)).collect()])
    }
}

struct NoOp;

#[async_trait::async_trait]
impl NodeType for NoOp {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.noOp"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        Ok(vec![ctx.input().iter().cloned().enumerate().map(|(i, it)| it.paired(i)).collect()])
    }
}

struct StopAndError;

#[async_trait::async_trait]
impl NodeType for StopAndError {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.stopAndError"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let kind = ctx.param_str("errorType", 0, "errorMessage")?;
        let message = if kind == "errorObject" {
            let raw = ctx.param("errorObject", 0)?;
            let obj = match raw {
                Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
                other => other,
            };
            obj.get("message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| obj.to_string())
        } else {
            ctx.param_str("errorMessage", 0, "An error occurred")?
        };
        Err(NodeError::new(message))
    }
}

/// Wait: timed waits run in-process. Webhook/form resumes need the server's
/// waiting-execution support (Phase 2).
struct Wait;

#[async_trait::async_trait]
impl NodeType for Wait {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.wait"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let resume = ctx.param_str("resume", 0, "timeInterval")?;
        let server = ctx.services.is_server();
        match resume.as_str() {
            "webhook" | "form" if server => return Err(NodeError::waiting(None)),
            "timeInterval" => {
                let amount = ctx.param_f64("amount", 0, 1.0)?;
                let unit = ctx.param_str("unit", 0, "hours")?;
                let secs = amount
                    * match unit.as_str() {
                        "seconds" => 1.0,
                        "minutes" => 60.0,
                        "hours" => 3600.0,
                        "days" => 86400.0,
                        _ => 1.0,
                    };
                // Like n8n, waits over 65 s are persisted instead of held in memory.
                if server && secs > 65.0 {
                    return Err(NodeError::waiting(Some(chrono::Utc::now().timestamp_millis() + (secs * 1000.0) as i64)));
                }
                tokio::time::sleep(std::time::Duration::from_secs_f64(secs.max(0.0))).await;
            }
            "specificTime" => {
                let when = ctx.param_str("dateTime", 0, "")?;
                if let Ok(t) = chrono::NaiveDateTime::parse_from_str(&when, "%Y-%m-%dT%H:%M:%S") {
                    let wait = t.and_utc().timestamp_millis() - chrono::Utc::now().timestamp_millis();
                    if wait > 65_000 && server {
                        return Err(NodeError::waiting(Some(t.and_utc().timestamp_millis())));
                    }
                    if wait > 65_000 {
                        return Err(NodeError::new("Waiting this long needs a running r8r server; the CLI can't persist a waiting execution").describe(format!("resume at {when}")));
                    }
                    if wait > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(wait as u64)).await;
                    }
                }
            }
            other => return Err(NodeError::new(format!("Resuming on \"{other}\" needs a running r8r server"))),
        }
        Ok(vec![ctx.input().iter().cloned().enumerate().map(|(i, it)| it.paired(i)).collect()])
    }
}

/// Execute Command: off by default (`NODES_EXCLUDE`), as in n8n 2.0.
struct ExecuteCommand;

#[async_trait::async_trait]
impl NodeType for ExecuteCommand {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.executeCommand"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let once = ctx.raw_param("executeOnce").and_then(Value::as_bool).unwrap_or(true);
        let count = if once { 1 } else { ctx.input().len().max(1) };
        let mut out = Vec::new();
        for i in 0..count {
            let command = ctx.param_str("command", i, "")?;
            let result = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
                .await
                .map_err(|e| NodeError::new(format!("Could not run the command: {e}")).at(i))?;
            let mut json = Map::new();
            json.insert("exitCode".into(), json!(result.status.code().unwrap_or(-1)));
            json.insert("stderr".into(), json!(String::from_utf8_lossy(&result.stderr).trim_end().to_string()));
            json.insert("stdout".into(), json!(String::from_utf8_lossy(&result.stdout).trim_end().to_string()));
            out.push(Item::new(json).paired(i));
        }
        Ok(vec![out])
    }
}
