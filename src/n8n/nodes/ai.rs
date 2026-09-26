//! AI cluster nodes (spec §6.8): root nodes (AI Agent, Basic LLM Chain)
//! take their model, memory and tools from sub-nodes connected over
//! `ai_languageModel`, `ai_memory` and `ai_tool`. Each sub-node call is
//! recorded in run data under the sub-node's name, as n8n's AI log view
//! expects, with token usage on the model's runs.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{now_ms, Item, NodeOutput};
use crate::n8n::workflow::Node;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const LC: &str = "@n8n/n8n-nodes-langchain.";

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![
        Box::new(Agent),
        Box::new(ChainLlm),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatOpenAi")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.toolCalculator")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.memoryBufferWindow")),
    ]
}

/// Sub-nodes only run when a root node calls them.
struct SubNode(&'static str);

#[async_trait::async_trait]
impl NodeType for SubNode {
    fn type_name(&self) -> &'static str {
        self.0
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        Err(NodeError::new(format!("\"{}\" is a sub-node: connect it to an AI Agent or chain", ctx.node.name)))
    }
}

// ---- run data for sub-nodes ----------------------------------------------------

fn record(ctx: &ExecCtx<'_>, sub: &str, kind: &str, input: Value, output: Result<Value, &NodeError>, started: i64) {
    let mut task = json!({
        "startTime": started,
        "executionIndex": 0,
        "executionTime": now_ms() - started,
        "executionStatus": if output.is_ok() { "success" } else { "error" },
        "source": [{"previousNode": ctx.node.name, "previousNodeRun": ctx.run_index}],
        "hints": [],
        "inputOverride": {kind: [[{"json": input}]]},
    });
    match output {
        Ok(out) => task["data"] = json!({kind: [[{"json": out}]]}),
        Err(e) => {
            if let Some(node) = ctx.workflow.node(sub) {
                task["error"] = e.to_json(node);
            }
        }
    }
    ctx.run.lock().unwrap().sub_runs.push((sub.to_string(), task));
}

// ---- chat model ------------------------------------------------------------------

struct Model<'a> {
    node: &'a Node,
    base_url: String,
    api_key: String,
    organization: Option<String>,
    model: String,
    options: Map<String, Value>,
}

async fn load_model<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Model<'a>> {
    let name = ctx
        .workflow
        .sub_nodes(&ctx.node.name, "ai_languageModel")
        .into_iter()
        .next()
        .ok_or_else(|| NodeError::new("A Chat Model sub-node must be connected and enabled"))?;
    let node = ctx.workflow.node(&name).expect("connected nodes exist");
    if node.node_type != format!("{LC}lmChatOpenAi") {
        return Err(NodeError::new(format!("The chat model \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
    }
    let params = ctx.resolve_value(&node.parameters, item)?;
    let model = match &params["model"] {
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("gpt-4o-mini").to_string(),
        Value::String(s) => s.clone(),
        _ => "gpt-4o-mini".into(),
    };
    let (_, cred) = ctx.credentials_for(node, "openAiApi").await?;
    let options = params["options"].as_object().cloned().unwrap_or_default();
    let base_url = options
        .get("baseURL")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or(cred["url"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("https://api.openai.com/v1")
        .trim_end_matches('/')
        .to_string();
    Ok(Model {
        node,
        base_url,
        api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
        organization: cred["organizationId"].as_str().filter(|s| !s.is_empty()).map(String::from),
        model,
        options,
    })
}

impl Model<'_> {
    /// One chat completion; returns the assistant message.
    async fn chat(&self, ctx: &ExecCtx<'_>, messages: &[Value], tools: &[Value]) -> NodeResult<Value> {
        let started = now_ms();
        let mut body = json!({"model": self.model, "messages": messages});
        let opt = |k: &str| self.options.get(k).cloned();
        if let Some(t) = opt("temperature") {
            body["temperature"] = t;
        }
        if let Some(t) = opt("maxTokens").filter(|t| t.as_i64().unwrap_or(-1) > 0) {
            body["max_tokens"] = t;
        }
        if let Some(t) = opt("topP") {
            body["top_p"] = t;
        }
        if let Some(t) = opt("frequencyPenalty") {
            body["frequency_penalty"] = t;
        }
        if let Some(t) = opt("presencePenalty") {
            body["presence_penalty"] = t;
        }
        if opt("responseFormat").and_then(|v| v.as_str().map(String::from)).as_deref() == Some("json_object") {
            body["response_format"] = json!({"type": "json_object"});
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }
        let input = json!({"messages": messages, "options": {"model": self.model}});
        let result = self.send(ctx, &body).await;
        match result {
            Ok(resp) => {
                let message = resp.pointer("/choices/0/message").cloned().unwrap_or(json!({"role": "assistant", "content": ""}));
                let usage = &resp["usage"];
                let out = json!({
                    "response": {"generations": [[{"text": message["content"].as_str().unwrap_or(""), "message": message}]]},
                    "tokenUsage": {
                        "completionTokens": usage["completion_tokens"].as_i64().unwrap_or(0),
                        "promptTokens": usage["prompt_tokens"].as_i64().unwrap_or(0),
                        "totalTokens": usage["total_tokens"].as_i64().unwrap_or(0),
                    },
                });
                record(ctx, &self.node.name, "ai_languageModel", input, Ok(out), started);
                Ok(message)
            }
            Err(e) => {
                record(ctx, &self.node.name, "ai_languageModel", input, Err(&e), started);
                Err(e)
            }
        }
    }

    async fn send(&self, ctx: &ExecCtx<'_>, body: &Value) -> NodeResult<Value> {
        let mut req = ctx.services.http.post(format!("{}/chat/completions", self.base_url)).bearer_auth(&self.api_key).json(body);
        if let Some(org) = &self.organization {
            req = req.header("OpenAI-Organization", org);
        }
        let timeout = self.options.get("timeout").and_then(Value::as_u64).unwrap_or(60_000);
        let resp = req
            .timeout(std::time::Duration::from_millis(timeout))
            .send()
            .await
            .map_err(|e| NodeError::new(format!("The model provider could not be reached: {}", e.without_url())))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let message = json.pointer("/error/message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("The model provider answered {status}"));
            return Err(NodeError::api(redact(&message, &self.api_key), Some(status.as_u16()), None));
        }
        if json.is_null() {
            return Err(NodeError::new("The model provider returned an invalid response"));
        }
        Ok(json)
    }
}

/// Keeps an API key out of error messages echoed from a provider.
fn redact(message: &str, key: &str) -> String {
    if key.len() >= 4 {
        message.replace(key, "***")
    } else {
        message.to_string()
    }
}

// ---- tools ---------------------------------------------------------------------

struct Tool<'a> {
    node: &'a Node,
    /// Tool names are the node names (n8n does the same).
    name: String,
}

fn tool_name(node: &Node) -> String {
    node.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn load_tools<'a>(ctx: &'a ExecCtx<'_>) -> NodeResult<Vec<Tool<'a>>> {
    let mut tools = Vec::new();
    for name in ctx.workflow.sub_nodes(&ctx.node.name, "ai_tool") {
        let node = ctx.workflow.node(&name).expect("connected nodes exist");
        if node.disabled {
            continue;
        }
        if node.node_type != format!("{LC}toolCalculator") {
            return Err(NodeError::new(format!("The tool \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
        }
        tools.push(Tool { node, name: tool_name(node) });
    }
    Ok(tools)
}

impl Tool<'_> {
    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": "Useful for getting the result of a math expression. The input to this tool should be a valid mathematical expression that could be executed by a simple calculator.",
                "parameters": {"type": "object", "properties": {"input": {"type": "string"}}, "required": ["input"], "additionalProperties": false},
            }
        })
    }

    fn call(&self, ctx: &ExecCtx<'_>, arguments: &str) -> String {
        let started = now_ms();
        let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({"input": arguments}));
        let input = args["input"].as_str().map(String::from).unwrap_or_else(|| args.to_string());
        let (result, output) = match calculate(&input) {
            Ok(n) => {
                let s = format_number(n);
                (s.clone(), Ok(json!({"response": s})))
            }
            Err(e) => (format!("Error: {e}"), Ok(json!({"response": format!("Error: {e}")}))),
        };
        record(ctx, &self.node.name, "ai_tool", json!({"input": input}), output, started);
        result
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// A calculator for `+ - * / % ^`, parentheses and a few functions.
pub fn calculate(expr: &str) -> Result<f64, String> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }
    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn peek(&mut self) -> Option<u8> {
            self.ws();
            self.s.get(self.i).copied()
        }
        fn expr(&mut self) -> Result<f64, String> {
            let mut v = self.term()?;
            while let Some(c @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let r = self.term()?;
                v = if c == b'+' { v + r } else { v - r };
            }
            Ok(v)
        }
        fn term(&mut self) -> Result<f64, String> {
            let mut v = self.power()?;
            loop {
                match self.peek() {
                    Some(c @ (b'*' | b'/' | b'%')) => {
                        self.i += 1;
                        let r = self.power()?;
                        v = match c {
                            b'*' => v * r,
                            b'/' => v / r,
                            _ => v % r,
                        };
                    }
                    Some(b'x') => {
                        self.i += 1;
                        v *= self.power()?;
                    }
                    _ => return Ok(v),
                }
            }
        }
        fn power(&mut self) -> Result<f64, String> {
            let base = self.unary()?;
            if self.peek() == Some(b'^') {
                self.i += 1;
                let exp = self.power()?;
                return Ok(base.powf(exp));
            }
            Ok(base)
        }
        fn unary(&mut self) -> Result<f64, String> {
            match self.peek() {
                Some(b'-') => {
                    self.i += 1;
                    Ok(-self.unary()?)
                }
                Some(b'+') => {
                    self.i += 1;
                    self.unary()
                }
                _ => self.atom(),
            }
        }
        fn atom(&mut self) -> Result<f64, String> {
            match self.peek() {
                Some(b'(') => {
                    self.i += 1;
                    let v = self.expr()?;
                    if self.peek() != Some(b')') {
                        return Err("missing )".into());
                    }
                    self.i += 1;
                    Ok(v)
                }
                Some(c) if c.is_ascii_digit() || c == b'.' => {
                    let start = self.i;
                    while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.' || self.s[self.i] == b'e') {
                        self.i += 1;
                    }
                    std::str::from_utf8(&self.s[start..self.i]).unwrap().parse().map_err(|_| "invalid number".to_string())
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i].is_ascii_alphanumeric() {
                        self.i += 1;
                    }
                    let name = std::str::from_utf8(&self.s[start..self.i]).unwrap().to_ascii_lowercase();
                    match name.as_str() {
                        "pi" => return Ok(std::f64::consts::PI),
                        "e" => return Ok(std::f64::consts::E),
                        _ => {}
                    }
                    let arg = self.atom()?;
                    Ok(match name.as_str() {
                        "sqrt" => arg.sqrt(),
                        "abs" => arg.abs(),
                        "sin" => arg.sin(),
                        "cos" => arg.cos(),
                        "tan" => arg.tan(),
                        "log" => arg.log10(),
                        "ln" => arg.ln(),
                        "exp" => arg.exp(),
                        "round" => arg.round(),
                        "floor" => arg.floor(),
                        "ceil" => arg.ceil(),
                        other => return Err(format!("unknown function {other}")),
                    })
                }
                _ => Err("unexpected end of expression".into()),
            }
        }
    }
    let mut p = P { s: expr.as_bytes(), i: 0 };
    let v = p.expr()?;
    if p.peek().is_some() {
        return Err(format!("unexpected input at position {}", p.i));
    }
    if !v.is_finite() {
        return Err("the result is not a finite number".into());
    }
    Ok(v)
}

// ---- memory --------------------------------------------------------------------

/// Conversations of the window buffer memory, per session (in-process, as
/// n8n's Simple Memory).
fn sessions() -> &'static Mutex<HashMap<String, Vec<Value>>> {
    static S: OnceLock<Mutex<HashMap<String, Vec<Value>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

struct Memory<'a> {
    node: &'a Node,
    key: String,
    window: usize,
}

fn load_memory<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Option<Memory<'a>>> {
    let Some(name) = ctx.workflow.sub_nodes(&ctx.node.name, "ai_memory").into_iter().next() else { return Ok(None) };
    let node = ctx.workflow.node(&name).expect("connected nodes exist");
    if node.disabled {
        return Ok(None);
    }
    if node.node_type != format!("{LC}memoryBufferWindow") {
        return Err(NodeError::new(format!("The memory \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
    }
    let params = ctx.resolve_value(&node.parameters, item)?;
    let session = if params["sessionIdType"].as_str() == Some("customKey") {
        params["sessionKey"].as_str().map(String::from).unwrap_or_default()
    } else {
        ctx.input().get(item).and_then(|i| i.json.get("sessionId")).and_then(Value::as_str).map(String::from).unwrap_or_default()
    };
    if session.is_empty() {
        return Err(NodeError::new("No session ID found").describe("Set a session key on the memory node, or pass sessionId in the input"));
    }
    let workflow = ctx.workflow.id.clone().unwrap_or_default();
    let window = params["contextWindowLength"].as_u64().unwrap_or(5).max(1) as usize;
    Ok(Some(Memory { node, key: format!("{workflow}:{}:{session}", node.name), window }))
}

impl Memory<'_> {
    fn load(&self, ctx: &ExecCtx<'_>) -> Vec<Value> {
        let started = now_ms();
        let all = sessions().lock().unwrap().get(&self.key).cloned().unwrap_or_default();
        let keep = self.window * 2;
        let history: Vec<Value> = all[all.len().saturating_sub(keep)..].to_vec();
        record(ctx, &self.node.name, "ai_memory", json!({"action": "loadMemoryVariables"}), Ok(json!({"action": "loadMemoryVariables", "chatHistory": history})), started);
        history
    }

    fn save(&self, ctx: &ExecCtx<'_>, user: &str, assistant: &str) {
        let started = now_ms();
        let pair = [json!({"role": "user", "content": user}), json!({"role": "assistant", "content": assistant})];
        let mut s = sessions().lock().unwrap();
        let list = s.entry(self.key.clone()).or_default();
        list.extend(pair.iter().cloned());
        let excess = list.len().saturating_sub(self.window * 2);
        list.drain(..excess);
        drop(s);
        record(ctx, &self.node.name, "ai_memory", json!({"action": "saveContext", "input": user, "output": assistant}), Ok(json!({"action": "saveContext"})), started);
    }
}

// ---- root nodes ------------------------------------------------------------------

fn prompt(ctx: &ExecCtx<'_>, item: usize) -> NodeResult<String> {
    let text = if ctx.param_str("promptType", item, "auto")? == "define" {
        ctx.param_str("text", item, "")?
    } else {
        ctx.input().get(item).and_then(|i| i.json.get("chatInput")).and_then(Value::as_str).unwrap_or("").to_string()
    };
    if text.trim().is_empty() {
        return Err(NodeError::new("No prompt specified").describe("Expected to find the prompt in an input field called 'chatInput', or define it on the node").at(item));
    }
    Ok(text)
}

struct Agent;

#[async_trait::async_trait]
impl NodeType for Agent {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.agent"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        for i in 0..ctx.input().len() {
            match run_agent(ctx, i).await {
                Ok(answer) => out.push(Item::new(Map::from_iter([("output".to_string(), json!(answer))])).paired(i)),
                Err(e) if ctx.continue_on_fail() => {
                    let e = e.at(i);
                    ctx.push_error_item(&e, i);
                }
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}

async fn run_agent(ctx: &ExecCtx<'_>, item: usize) -> NodeResult<String> {
    let user = prompt(ctx, item)?;
    let model = load_model(ctx, item).await?;
    let tools = load_tools(ctx)?;
    let memory = load_memory(ctx, item)?;
    let system = ctx.param_str("options.systemMessage", item, "You are a helpful assistant")?;
    let max_iterations = ctx.param_f64("options.maxIterations", item, 10.0)?.max(1.0) as usize;
    let mut messages = vec![json!({"role": "system", "content": system})];
    if let Some(m) = &memory {
        messages.extend(m.load(ctx));
    }
    messages.push(json!({"role": "user", "content": user}));
    let schemas: Vec<Value> = tools.iter().map(Tool::schema).collect();
    let mut answer = None;
    for _ in 0..max_iterations {
        let reply = model.chat(ctx, &messages, &schemas).await?;
        let calls = reply["tool_calls"].as_array().cloned().unwrap_or_default();
        if calls.is_empty() {
            answer = Some(reply["content"].as_str().unwrap_or("").to_string());
            break;
        }
        messages.push(json!({"role": "assistant", "content": reply["content"], "tool_calls": calls}));
        for call in &calls {
            let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
            let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
            let result = match tools.iter().find(|t| t.name == name) {
                Some(tool) => tool.call(ctx, args),
                None => format!("Error: there is no tool called \"{name}\""),
            };
            messages.push(json!({"role": "tool", "tool_call_id": call["id"], "content": result}));
        }
    }
    let answer = answer.unwrap_or_else(|| "Agent stopped due to max iterations.".to_string());
    if let Some(m) = &memory {
        m.save(ctx, &user, &answer);
    }
    Ok(answer)
}

struct ChainLlm;

#[async_trait::async_trait]
impl NodeType for ChainLlm {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.chainLlm"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        for i in 0..ctx.input().len() {
            let result = async {
                let user = prompt(ctx, i)?;
                let model = load_model(ctx, i).await?;
                let mut messages = Vec::new();
                for m in ctx.raw_param("messages.messageValues").and_then(Value::as_array).cloned().unwrap_or_default().iter() {
                    let text = ctx.resolve_value(&m["message"], i)?;
                    let role = match m["type"].as_str() {
                        Some("AIMessagePromptTemplate") => "assistant",
                        Some("HumanMessagePromptTemplate") => "user",
                        _ => "system",
                    };
                    messages.push(json!({"role": role, "content": text}));
                }
                messages.push(json!({"role": "user", "content": user}));
                let reply = model.chat(ctx, &messages, &[]).await?;
                Ok::<String, NodeError>(reply["content"].as_str().unwrap_or("").to_string())
            }
            .await;
            match result {
                Ok(text) => out.push(Item::new(Map::from_iter([("text".to_string(), json!(text))])).paired(i)),
                Err(e) if ctx.continue_on_fail() => {
                    let e = e.at(i);
                    ctx.push_error_item(&e, i);
                }
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculator_handles_precedence_and_functions() {
        assert_eq!(calculate("6 * 7").unwrap(), 42.0);
        assert_eq!(calculate("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(calculate("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(calculate("2 ^ 3 ^ 2").unwrap(), 512.0);
        assert_eq!(calculate("sqrt(16) + -1").unwrap(), 3.0);
        assert!(calculate("1 +").is_err());
        assert!(calculate("1 / 0").is_err());
        assert_eq!(format_number(42.0), "42");
        assert_eq!(format_number(0.5), "0.5");
    }

    #[test]
    fn provider_messages_never_echo_the_key() {
        assert_eq!(redact("bad key sk-test-123 given", "sk-test-123"), "bad key *** given");
    }
}
