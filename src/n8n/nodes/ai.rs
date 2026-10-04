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
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatOllama")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatAnthropic")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatOpenRouter")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatGroq")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatMistralCloud")),
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

/// Which chat-model provider a sub-node talks to. Adding a new
/// OpenAI-compatible provider (Groq, Mistral, OpenRouter) only needs a new
/// variant plus its credential/base-URL lookup in `load_model`; `chat`/
/// `send` below already carry the OpenAI-shaped request for `OpenAi`.
/// A provider with its own wire format (Anthropic, Ollama) adds its own
/// request/response branch the way `Ollama` does here.
enum Provider {
    /// Shared by the OpenAI-compatible providers (OpenAI itself, OpenRouter,
    /// Groq, Mistral): bearer auth, `POST {base_url}/chat/completions`, an
    /// OpenAI-shaped request/response. `organization` is only ever set for
    /// OpenAI (its `OpenAI-Organization` header).
    OpenAi { organization: Option<String> },
    Ollama,
    /// Anthropic's own wire format: `POST {base_url}/v1/messages`, headers
    /// `x-api-key` + `anthropic-version`, top-level `system`, required
    /// `max_tokens`, content blocks (`tool_use`/`tool_result`).
    Anthropic,
}

struct Model<'a> {
    node: &'a Node,
    base_url: String,
    api_key: String,
    model: String,
    options: Map<String, Value>,
    provider: Provider,
}

fn model_param(params: &Value, default: &str) -> String {
    match &params["model"] {
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or(default).to_string(),
        Value::String(s) => s.clone(),
        _ => default.into(),
    }
}

async fn load_model<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Model<'a>> {
    let name = ctx
        .workflow
        .sub_nodes(&ctx.node.name, "ai_languageModel")
        .into_iter()
        .next()
        .ok_or_else(|| NodeError::new("A Chat Model sub-node must be connected and enabled"))?;
    let node = ctx.workflow.node(&name).expect("connected nodes exist");
    let params = ctx.resolve_value(&node.parameters, item)?;
    let options = params["options"].as_object().cloned().unwrap_or_default();
    if node.node_type == format!("{LC}lmChatOpenAi") {
        let (_, cred) = ctx.credentials_for(node, "openAiApi").await?;
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
            model: model_param(&params, "gpt-4o-mini"),
            options,
            provider: Provider::OpenAi { organization: cred["organizationId"].as_str().filter(|s| !s.is_empty()).map(String::from) },
        })
    } else if node.node_type == format!("{LC}lmChatOllama") {
        let (_, cred) = ctx.credentials_for(node, "ollamaApi").await?;
        let base_url = cred["baseUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("http://localhost:11434").trim_end_matches('/').to_string();
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model_param(&params, "llama3.2"),
            options,
            provider: Provider::Ollama,
        })
    } else if node.node_type == format!("{LC}lmChatAnthropic") {
        let (_, cred) = ctx.credentials_for(node, "anthropicApi").await?;
        let base_url = options
            .get("baseURL")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(cred["url"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("https://api.anthropic.com")
            .trim_end_matches('/')
            .to_string();
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model_param(&params, "claude-3-5-sonnet-20241022"),
            options,
            provider: Provider::Anthropic,
        })
    } else if node.node_type == format!("{LC}lmChatOpenRouter") {
        let (_, cred) = ctx.credentials_for(node, "openRouterApi").await?;
        let base_url = options
            .get("baseURL")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(cred["url"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("https://openrouter.ai/api/v1")
            .trim_end_matches('/')
            .to_string();
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model_param(&params, "openai/gpt-4.1-mini"),
            options,
            provider: Provider::OpenAi { organization: None },
        })
    } else if node.node_type == format!("{LC}lmChatGroq") {
        let (_, cred) = ctx.credentials_for(node, "groqApi").await?;
        let base_url = options
            .get("baseURL")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(cred["url"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("https://api.groq.com/openai/v1")
            .trim_end_matches('/')
            .to_string();
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model_param(&params, "llama3-8b-8192"),
            options,
            provider: Provider::OpenAi { organization: None },
        })
    } else if node.node_type == format!("{LC}lmChatMistralCloud") {
        let (_, cred) = ctx.credentials_for(node, "mistralCloudApi").await?;
        let base_url = options
            .get("baseURL")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or(cred["url"].as_str().filter(|s| !s.is_empty()))
            .unwrap_or("https://api.mistral.ai/v1")
            .trim_end_matches('/')
            .to_string();
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model_param(&params, "mistral-small"),
            options,
            provider: Provider::OpenAi { organization: None },
        })
    } else {
        Err(NodeError::new(format!("The chat model \"{}\" ({}) is not supported natively yet", node.name, node.node_type)))
    }
}

impl Model<'_> {
    /// One chat completion; returns the assistant message (OpenAI-shaped:
    /// `role`/`content`/`tool_calls` with stringified arguments, whatever
    /// the provider's wire format).
    async fn chat(&self, ctx: &ExecCtx<'_>, messages: &[Value], tools: &[Value]) -> NodeResult<Value> {
        let started = now_ms();
        let body = match &self.provider {
            Provider::OpenAi { .. } => self.openai_body(messages, tools),
            Provider::Ollama => self.ollama_body(messages, tools),
            Provider::Anthropic => self.anthropic_body(messages, tools),
        };
        let input = json!({"messages": messages, "options": {"model": self.model}});
        let result = self.send(ctx, &body).await;
        match result {
            Ok(resp) => {
                let (message, usage) = match &self.provider {
                    Provider::OpenAi { .. } => self.openai_response(&resp),
                    Provider::Ollama => self.ollama_response(&resp),
                    Provider::Anthropic => self.anthropic_response(&resp),
                };
                let out = json!({
                    "response": {"generations": [[{"text": message["content"].as_str().unwrap_or(""), "message": message}]]},
                    "tokenUsage": usage,
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

    fn openai_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let mut body = json!({"model": self.model, "messages": messages});
        let opt = |k: &str| self.options.get(k).cloned();
        if let Some(t) = opt("temperature") {
            body["temperature"] = t;
        }
        // Groq's node calls this option "maxTokensToSample" (same as
        // Anthropic's) rather than OpenAI's "maxTokens".
        if let Some(t) = opt("maxTokens").or_else(|| opt("maxTokensToSample")).filter(|t| t.as_i64().unwrap_or(-1) > 0) {
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
        // Mistral-only options (harmless no-ops for the other OpenAI-shaped
        // providers, whose node parameters never populate these keys).
        if opt("safeMode").and_then(|v| v.as_bool()) == Some(true) {
            body["safe_prompt"] = json!(true);
        }
        if let Some(t) = opt("randomSeed") {
            body["random_seed"] = t;
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }
        body
    }

    fn openai_response(&self, resp: &Value) -> (Value, Value) {
        let message = resp.pointer("/choices/0/message").cloned().unwrap_or(json!({"role": "assistant", "content": ""}));
        let usage = &resp["usage"];
        let usage = json!({
            "completionTokens": usage["completion_tokens"].as_i64().unwrap_or(0),
            "promptTokens": usage["prompt_tokens"].as_i64().unwrap_or(0),
            "totalTokens": usage["total_tokens"].as_i64().unwrap_or(0),
        });
        (message, usage)
    }

    /// Ollama's native `/api/chat` (non-streaming): LangChain-style camelCase
    /// node options map onto Ollama's snake_case `options` object, plus the
    /// top-level `keep_alive`/`format` and `stream: false`.
    fn ollama_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let mut body = json!({"model": self.model, "messages": messages, "stream": false});
        let opt = |k: &str| self.options.get(k).cloned();
        let mut options = Map::new();
        let map = [
            ("temperature", "temperature"),
            ("topK", "top_k"),
            ("topP", "top_p"),
            ("frequencyPenalty", "frequency_penalty"),
            ("presencePenalty", "presence_penalty"),
            ("repeatPenalty", "repeat_penalty"),
            ("numCtx", "num_ctx"),
            ("numPredict", "num_predict"),
            ("lowVram", "low_vram"),
            ("mainGpu", "main_gpu"),
            ("numBatch", "num_batch"),
            ("numGpu", "num_gpu"),
            ("numThread", "num_thread"),
            ("penalizeNewline", "penalize_newline"),
            ("useMLock", "use_mlock"),
            ("useMMap", "use_mmap"),
            ("vocabOnly", "vocab_only"),
        ];
        for (n8n_key, ollama_key) in map {
            if let Some(v) = opt(n8n_key) {
                options.insert(ollama_key.to_string(), v);
            }
        }
        if !options.is_empty() {
            body["options"] = Value::Object(options);
        }
        if let Some(v) = opt("keepAlive") {
            body["keep_alive"] = v;
        }
        if opt("format").and_then(|v| v.as_str().map(String::from)).as_deref() == Some("json") {
            body["format"] = json!("json");
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools.to_vec());
        }
        body
    }

    /// Normalises Ollama's response into the OpenAI-shaped message the
    /// agent loop expects: `tool_calls[].function.arguments` as a JSON
    /// string (Ollama sends an object), with a synthetic call id so the
    /// loop's `tool_call_id` round-trip has something to echo.
    fn ollama_response(&self, resp: &Value) -> (Value, Value) {
        let mut message = resp["message"].clone();
        if message.is_null() {
            message = json!({"role": "assistant", "content": ""});
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array).cloned() {
            let calls: Vec<Value> = calls
                .into_iter()
                .enumerate()
                .map(|(i, mut call)| {
                    let args = call.pointer("/function/arguments").cloned().unwrap_or(json!({}));
                    let args_str = if args.is_string() { args.as_str().unwrap().to_string() } else { args.to_string() };
                    call["function"]["arguments"] = json!(args_str);
                    if call.get("id").is_none() {
                        call["id"] = json!(format!("call_{i}"));
                    }
                    call
                })
                .collect();
            message["tool_calls"] = Value::Array(calls);
        }
        let prompt = resp["prompt_eval_count"].as_i64().unwrap_or(0);
        let completion = resp["eval_count"].as_i64().unwrap_or(0);
        let usage = json!({
            "completionTokens": completion,
            "promptTokens": prompt,
            "totalTokens": prompt + completion,
        });
        (message, usage)
    }

    /// Converts the OpenAI-shaped messages the agent loop builds into
    /// Anthropic's Messages API shape: `system` lifted to a top-level
    /// field, assistant tool calls become `tool_use` content blocks, and
    /// `tool` role replies become a user turn with a `tool_result` block.
    fn anthropic_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let mut system = String::new();
        let mut out = Vec::new();
        for m in messages {
            match m["role"].as_str() {
                Some("system") => {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(m["content"].as_str().unwrap_or(""));
                }
                Some("user") => out.push(json!({"role": "user", "content": m["content"].as_str().unwrap_or("")})),
                Some("assistant") => {
                    let mut content = Vec::new();
                    if let Some(text) = m["content"].as_str().filter(|s| !s.is_empty()) {
                        content.push(json!({"type": "text", "text": text}));
                    }
                    for call in m["tool_calls"].as_array().into_iter().flatten() {
                        let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
                        let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
                        let input: Value = serde_json::from_str(args).unwrap_or(json!({}));
                        content.push(json!({"type": "tool_use", "id": call["id"], "name": name, "input": input}));
                    }
                    out.push(json!({"role": "assistant", "content": content}));
                }
                Some("tool") => out.push(json!({
                    "role": "user",
                    "content": [{"type": "tool_result", "tool_use_id": m["tool_call_id"], "content": m["content"].as_str().unwrap_or("")}],
                })),
                _ => {}
            }
        }
        let opt = |k: &str| self.options.get(k).cloned();
        let max_tokens = opt("maxTokensToSample").and_then(|v| v.as_i64()).filter(|v| *v > 0).unwrap_or(4096);
        let mut body = json!({"model": self.model, "max_tokens": max_tokens, "messages": out});
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(t) = opt("temperature") {
            body["temperature"] = t;
        }
        if let Some(t) = opt("topK") {
            body["top_k"] = t;
        }
        if let Some(t) = opt("topP") {
            body["top_p"] = t;
        }
        if !tools.is_empty() {
            let schemas: Vec<Value> = tools
                .iter()
                .map(|t| json!({"name": t["function"]["name"], "description": t["function"]["description"], "input_schema": t["function"]["parameters"]}))
                .collect();
            body["tools"] = Value::Array(schemas);
        }
        body
    }

    /// Normalises Anthropic's content blocks into the OpenAI-shaped
    /// message the agent loop expects: concatenated `text` blocks as
    /// `content`, `tool_use` blocks as `tool_calls` with stringified
    /// arguments.
    fn anthropic_response(&self, resp: &Value) -> (Value, Value) {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for (i, block) in resp["content"].as_array().into_iter().flatten().enumerate() {
            match block["type"].as_str() {
                Some("text") => text.push_str(block["text"].as_str().unwrap_or("")),
                Some("tool_use") => {
                    let id = block["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{i}"));
                    let arguments = serde_json::to_string(&block["input"]).unwrap_or_else(|_| "{}".to_string());
                    tool_calls.push(json!({"id": id, "type": "function", "function": {"name": block["name"], "arguments": arguments}}));
                }
                _ => {}
            }
        }
        let mut message = json!({"role": "assistant", "content": text});
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        let input = resp["usage"]["input_tokens"].as_i64().unwrap_or(0);
        let output = resp["usage"]["output_tokens"].as_i64().unwrap_or(0);
        let usage = json!({"completionTokens": output, "promptTokens": input, "totalTokens": input + output});
        (message, usage)
    }

    fn endpoint(&self) -> String {
        match self.provider {
            Provider::OpenAi { .. } => format!("{}/chat/completions", self.base_url),
            Provider::Ollama => format!("{}/api/chat", self.base_url),
            Provider::Anthropic => format!("{}/v1/messages", self.base_url),
        }
    }

    /// Message for an unreachable provider, mirroring n8n's generic
    /// `NodeApiError` wrap of the underlying fetch failure.
    fn unreachable_message(&self, err: reqwest::Error) -> String {
        match self.provider {
            Provider::OpenAi { .. } | Provider::Anthropic => format!("The model provider could not be reached: {}", err.without_url()),
            Provider::Ollama => format!("Ollama could not be reached at {}: {}", self.base_url, err.without_url()),
        }
    }

    fn error_message(&self, status: reqwest::StatusCode, json: &Value) -> String {
        match self.provider {
            Provider::OpenAi { .. } | Provider::Anthropic => json.pointer("/error/message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("The model provider answered {status}")),
            Provider::Ollama => json["error"].as_str().map(String::from).unwrap_or_else(|| format!("Ollama answered {status}")),
        }
    }

    async fn send(&self, ctx: &ExecCtx<'_>, body: &Value) -> NodeResult<Value> {
        let mut req = ctx.services.http.post(self.endpoint());
        match &self.provider {
            Provider::OpenAi { organization } => {
                req = req.bearer_auth(&self.api_key);
                if let Some(org) = organization {
                    req = req.header("OpenAI-Organization", org);
                }
            }
            Provider::Ollama => {
                if !self.api_key.is_empty() {
                    req = req.bearer_auth(&self.api_key);
                }
            }
            Provider::Anthropic => {
                req = req.header("x-api-key", &self.api_key).header("anthropic-version", "2023-06-01");
            }
        }
        req = req.json(body);
        let timeout = self.options.get("timeout").and_then(Value::as_u64).unwrap_or(60_000);
        let resp = req.timeout(std::time::Duration::from_millis(timeout)).send().await.map_err(|e| NodeError::new(self.unreachable_message(e)))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let message = self.error_message(status, &json);
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
