//! Nodes that need the server: HTTP-triggered starts, responding to the
//! caller, sub-workflows and persisted waits.

use crate::n8n::engine::WebhookResponse;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![
        Box::new(Passthrough("n8n-nodes-base.webhook")),
        Box::new(Passthrough("n8n-nodes-base.formTrigger")),
        Box::new(Passthrough("n8n-nodes-base.scheduleTrigger")),
        Box::new(RespondToWebhook),
        Box::new(ExecuteWorkflow),
    ]
}

/// Triggers whose data comes from outside the run (request, form, clock).
struct Passthrough(&'static str);

#[async_trait::async_trait]
impl NodeType for Passthrough {
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

struct RespondToWebhook;

fn body_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::String(s) => s.clone().into_bytes(),
        other => serde_json::to_vec(other).unwrap_or_default(),
    }
}

#[async_trait::async_trait]
impl NodeType for RespondToWebhook {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.respondToWebhook"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items: Vec<Item> = ctx.input().iter().cloned().enumerate().map(|(i, it)| it.paired(i)).collect();
        let Some(slot) = ctx.options.response.clone() else { return Ok(vec![items]) };
        let respond_with = ctx.param_str("respondWith", 0, "firstIncomingItem")?;
        let mut headers: Vec<(String, String)> = Vec::new();
        for (i, h) in ctx.raw_param("options.responseHeaders.entries").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
            let name = h["name"].as_str().unwrap_or_default().to_string();
            let value = ctx.param_str(&format!("options.responseHeaders.entries.{i}.value"), 0, "")?;
            headers.push((name, value));
        }
        let default_code = if respond_with == "redirect" { 307 } else { 200 };
        let status = ctx.param_f64("options.responseCode", 0, default_code as f64)? as u16;
        let json_ct = ("content-type".to_string(), "application/json; charset=utf-8".to_string());
        let body = match respond_with.as_str() {
            "allIncomingItems" => {
                headers.push(json_ct);
                serde_json::to_vec(&ctx.input().iter().map(Item::json_value).collect::<Vec<_>>()).unwrap()
            }
            "firstIncomingItem" => {
                headers.push(json_ct);
                serde_json::to_vec(&ctx.input().first().map(Item::json_value).unwrap_or(json!({}))).unwrap()
            }
            "json" => {
                headers.push(json_ct);
                let v = ctx.param("responseBody", 0)?;
                match v {
                    Value::String(s) => serde_json::from_str::<Value>(&s)
                        .map(|p| serde_json::to_vec(&p).unwrap())
                        .map_err(|e| NodeError::new("Invalid JSON in 'Response Body' field").describe(e.to_string()))?,
                    other => serde_json::to_vec(&other).unwrap(),
                }
            }
            "text" => {
                headers.push(("content-type".into(), "text/html; charset=utf-8".into()));
                body_bytes(&ctx.param("responseBody", 0)?)
            }
            "redirect" => {
                headers.push(("location".into(), ctx.param_str("redirectURL", 0, "")?));
                Vec::new()
            }
            _ => Vec::new(),
        };
        if let Some(sender) = slot.lock().unwrap().take() {
            let _ = sender.send(WebhookResponse { status, headers, body });
        }
        Ok(vec![items])
    }
}

struct ExecuteWorkflow;

#[async_trait::async_trait]
impl NodeType for ExecuteWorkflow {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.executeWorkflow"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let runner = ctx.services.sub_workflows.clone().ok_or_else(|| NodeError::new("Sub-workflows need a running r8r server"))?;
        let id = match ctx.param("workflowId", 0)? {
            Value::Object(o) => o.get("value").map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default(),
            Value::String(s) => s,
            other => other.to_string(),
        };
        if id.is_empty() {
            return Err(NodeError::new("No workflow to execute was selected"));
        }
        let wait = ctx.param_bool("options.waitForSubWorkflow", 0, true)?;
        let items = ctx.input().to_vec();
        let parent = ctx.workflow.id.clone();
        if ctx.param_str("mode", 0, "once")? == "each" {
            let mut out = Vec::new();
            for (i, item) in items.into_iter().enumerate() {
                let result = runner.run_sub_workflow(parent.as_deref(), &id, vec![item.clone()], wait).await.map_err(|e| e.at(i))?;
                out.extend(result.into_iter().map(|it| it.paired(i)));
            }
            return Ok(vec![out]);
        }
        let result = runner.run_sub_workflow(parent.as_deref(), &id, items.clone(), wait).await?;
        if !wait {
            return Ok(vec![items.into_iter().enumerate().map(|(i, it)| it.paired(i)).collect()]);
        }
        Ok(vec![result])
    }
}
