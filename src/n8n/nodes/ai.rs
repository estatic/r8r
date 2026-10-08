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
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatGoogleGemini")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatAzureOpenAi")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.lmChatAwsBedrock")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.toolCalculator")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.toolCode")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.toolWorkflow")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.outputParserStructured")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.memoryBufferWindow")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.memoryPostgresChat")),
        Box::new(SubNode("@n8n/n8n-nodes-langchain.memoryRedisChat")),
    ]
}

/// Sub-nodes only run when a root node calls them.
pub(super) struct SubNode(pub &'static str);

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

pub(super) fn record(ctx: &ExecCtx<'_>, sub: &str, kind: &str, input: Value, output: Result<Value, &NodeError>, started: i64) {
    record_items(ctx, sub, kind, input, output.map(|o| vec![o]), started);
}

/// Like `record`, with several output items.
fn record_items(ctx: &ExecCtx<'_>, sub: &str, kind: &str, input: Value, output: Result<Vec<Value>, &NodeError>, started: i64) {
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
        Ok(out) => task["data"] = json!({kind: [out.into_iter().map(|o| json!({"json": o})).collect::<Vec<_>>()]}),
        // n8n records a sub-node's input as its data first and only
        // replaces it on success, so a failed run keeps its input there.
        Err(e) => {
            task["data"] = task["inputOverride"].clone();
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
    /// Google's Generative Language API (what `@google/generative-ai`, under
    /// LangChain's `ChatGoogleGenerativeAI`, calls): `POST
    /// {host}/v1beta/models/{model}:generateContent`, `x-goog-api-key`,
    /// `contents` with `user`/`model` roles and `parts`, a top-level
    /// `systemInstruction`, `functionCall`/`functionResponse` parts.
    Gemini,
    /// Azure OpenAI: the OpenAI wire format against a deployment URL
    /// (`{endpoint}/openai/deployments/{deployment}/chat/completions
    /// ?api-version=...`) with an `api-key` header instead of bearer auth.
    AzureOpenAi { api_version: String },
    /// Bedrock's Converse API (what LangChain's `ChatBedrockConverse`
    /// calls): `POST {runtime}/model/{modelId}/converse`, SigV4-signed
    /// (`api_key` holds the secret key), top-level `system` blocks,
    /// `toolUse`/`toolResult` content blocks.
    Bedrock { region: String, access_key_id: String, session_token: Option<String> },
}

pub(super) struct Model<'a> {
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

pub(super) async fn load_model<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Model<'a>> {
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
    } else if node.node_type == format!("{LC}lmChatGoogleGemini") {
        let (_, cred) = ctx.credentials_for(node, "googlePalmApi").await?;
        let base_url = cred["host"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://generativelanguage.googleapis.com").trim_end_matches('/').to_string();
        // n8n's default model depends on the node version; LangChain drops
        // the `models/` prefix and the SDK puts it back in the URL.
        let default = if node.type_version >= 1.1 { "models/gemini-3-flash-preview" } else { "models/gemini-2.5-flash" };
        let model = params["modelName"].as_str().filter(|s| !s.is_empty()).unwrap_or(default);
        check_path_name(model, "model")?;
        Ok(Model {
            node,
            base_url,
            api_key: cred["apiKey"].as_str().unwrap_or("").to_string(),
            model: model.strip_prefix("models/").unwrap_or(model).to_string(),
            options,
            provider: Provider::Gemini,
        })
    } else if node.node_type == format!("{LC}lmChatAzureOpenAi") {
        if params["authentication"].as_str().is_some_and(|a| a != "azureOpenAiApi") {
            return Err(NodeError::new("Azure Entra ID (OAuth2) authentication is not supported natively yet; use an API key credential"));
        }
        let (_, cred) = ctx.credentials_for(node, "azureOpenAiApi").await?;
        let api_key = cred["apiKey"].as_str().unwrap_or("").to_string();
        if api_key.is_empty() {
            return Err(NodeError::new("API Key is missing in the selected Azure OpenAI API credential. Please configure the API Key or choose Entra ID authentication."));
        }
        // The deployment name is the node's `model` parameter; LangChain
        // builds the URL from the endpoint when set, else the resource name.
        let deployment = model_param(&params, "");
        check_path_name(&deployment, "deployment")?;
        let root = match cred["endpoint"].as_str().map(|e| e.trim_end_matches('/')).filter(|e| !e.is_empty()) {
            Some(endpoint) => endpoint.to_string(),
            None => format!("https://{}.openai.azure.com", cred["resourceName"].as_str().unwrap_or("")),
        };
        Ok(Model {
            node,
            base_url: format!("{root}/openai/deployments/{}", path_segment(&deployment)),
            api_key,
            model: deployment,
            options,
            provider: Provider::AzureOpenAi { api_version: cred["apiVersion"].as_str().filter(|s| !s.is_empty()).unwrap_or("2025-03-01-preview").to_string() },
        })
    } else if node.node_type == format!("{LC}lmChatAwsBedrock") {
        if params["authentication"].as_str().is_some_and(|a| a != "iam") {
            return Err(NodeError::new("AWS assume-role authentication is not supported natively yet; use an AWS (IAM) credential"));
        }
        let (_, cred) = ctx.credentials_for(node, "aws").await?;
        let model = model_param(&params, "");
        if model.is_empty() || model == "." || model == ".." {
            return Err(NodeError::new(format!("Invalid model \"{model}\"")));
        }
        // A model given as an ARN carries its own region (n8n's
        // `resolveBedrockRegion`).
        let arn_region = regex::Regex::new(r"^arn:(?:aws|aws-cn|aws-us-gov):bedrock:([a-z0-9-]+):").unwrap().captures(&model).map(|c| c[1].to_string());
        let region = arn_region.unwrap_or_else(|| cred["region"].as_str().filter(|s| !s.is_empty()).unwrap_or("us-east-1").to_string());
        if !regex::Regex::new(r"^[a-z]{2,4}(-[a-z]+)+-\d+$").unwrap().is_match(&region) {
            return Err(NodeError::new(format!("Invalid AWS region \"{region}\"")));
        }
        let base_url = match cred["bedrockRuntimeEndpoint"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
            Some(custom) => custom.replace("{region}", &region).trim_end_matches('/').to_string(),
            None => format!("https://bedrock-runtime.{region}.{}", if region.starts_with("cn-") { "amazonaws.com.cn" } else { "amazonaws.com" }),
        };
        let session_token = if cred["temporaryCredentials"].as_bool() == Some(true) { cred["sessionToken"].as_str().filter(|s| !s.is_empty()).map(String::from) } else { None };
        Ok(Model {
            node,
            base_url,
            api_key: cred["secretAccessKey"].as_str().unwrap_or("").to_string(),
            model,
            options,
            provider: Provider::Bedrock { region, access_key_id: cred["accessKeyId"].as_str().unwrap_or("").to_string(), session_token },
        })
    } else {
        Err(NodeError::new(format!("The chat model \"{}\" ({}) is not supported natively yet", node.name, node.node_type)))
    }
}

impl Model<'_> {
    /// One chat completion; returns the assistant message (OpenAI-shaped:
    /// `role`/`content`/`tool_calls` with stringified arguments, whatever
    /// the provider's wire format).
    pub(super) async fn chat(&self, ctx: &ExecCtx<'_>, messages: &[Value], tools: &[Value]) -> NodeResult<Value> {
        let started = now_ms();
        let body = match &self.provider {
            Provider::OpenAi { .. } | Provider::AzureOpenAi { .. } => self.openai_body(messages, tools),
            Provider::Ollama => self.ollama_body(messages, tools),
            Provider::Anthropic => self.anthropic_body(messages, tools),
            Provider::Gemini => self.gemini_body(messages, tools),
            Provider::Bedrock { .. } => self.bedrock_body(messages, tools)?,
        };
        let input = json!({"messages": messages, "options": {"model": self.model}});
        let result = self.send(ctx, &body).await;
        match result {
            Ok(resp) => {
                let (message, usage) = match &self.provider {
                    Provider::OpenAi { .. } | Provider::AzureOpenAi { .. } => self.openai_response(&resp),
                    Provider::Ollama => self.ollama_response(&resp),
                    Provider::Anthropic => self.anthropic_response(&resp),
                    Provider::Gemini => self.gemini_response(&resp),
                    Provider::Bedrock { .. } => self.bedrock_response(&resp),
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
            // n8n's ChatOpenAI-based models send LangChain's conversion of
            // each tool: the JSON schema with its `$schema` and `strict: false`.
            let langchain_openai = [format!("{LC}lmChatOpenAi"), format!("{LC}lmChatAzureOpenAi"), format!("{LC}lmChatOpenRouter")].contains(&self.node.node_type);
            let tools: Vec<Value> = tools
                .iter()
                .map(|t| {
                    let mut t = t.clone();
                    if langchain_openai {
                        if let Some(params) = t["function"]["parameters"].as_object_mut() {
                            params.entry("$schema").or_insert(json!("http://json-schema.org/draft-07/schema#"));
                        }
                        if let Some(f) = t["function"].as_object_mut() {
                            f.entry("strict").or_insert(json!(false));
                        }
                    }
                    t
                })
                .collect();
            body["tools"] = Value::Array(tools);
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

    /// Converts the OpenAI-shaped messages into Gemini `contents`, as
    /// LangChain's `convertBaseMessagesToContent` does: the leading system
    /// message becomes `systemInstruction`, assistant turns are role
    /// `model` with `functionCall` parts, and each tool reply is a `user`
    /// turn with a `functionResponse` part named after the tool it answers
    /// (Gemini matches calls to results by name, not id).
    fn gemini_body(&self, messages: &[Value], tools: &[Value]) -> Value {
        let mut system = Vec::new();
        let mut contents = Vec::new();
        let mut call_names: HashMap<String, String> = HashMap::new();
        for m in messages {
            let text = m["content"].as_str().unwrap_or("");
            match m["role"].as_str() {
                Some("system") => system.push(json!({"text": text})),
                Some("user") => contents.push(json!({"role": "user", "parts": [{"text": text}]})),
                Some("assistant") => {
                    let mut parts = Vec::new();
                    if !text.is_empty() {
                        parts.push(json!({"text": text}));
                    }
                    for call in m["tool_calls"].as_array().into_iter().flatten() {
                        let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
                        if let Some(id) = call["id"].as_str() {
                            call_names.insert(id.to_string(), name.to_string());
                        }
                        let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
                        let mut part = json!({"functionCall": {"name": name, "args": serde_json::from_str::<Value>(args).unwrap_or(json!({}))}});
                        // Gemini 3 rejects a replayed function call without
                        // the thought signature it came with.
                        if let Some(sig) = call["thought_signature"].as_str() {
                            part["thoughtSignature"] = json!(sig);
                        } else if self.model.contains("gemini-3") {
                            part["thoughtSignature"] = json!(GEMINI_DUMMY_SIGNATURE);
                        }
                        parts.push(part);
                    }
                    contents.push(json!({"role": "model", "parts": parts}));
                }
                Some("tool") => {
                    let id = m["tool_call_id"].as_str().unwrap_or("");
                    let name = m["name"].as_str().map(String::from).or_else(|| call_names.get(id).cloned()).unwrap_or_default();
                    contents.push(json!({"role": "user", "parts": [{"functionResponse": {"name": name, "response": {"result": text}}}]}));
                }
                _ => {}
            }
        }
        let opt = |k: &str| self.options.get(k).cloned();
        let mut config = Map::new();
        for key in ["maxOutputTokens", "temperature", "topP", "topK"] {
            if let Some(v) = opt(key) {
                config.insert(key.to_string(), v);
            }
        }
        let mut body = json!({"contents": contents, "generationConfig": config});
        if !system.is_empty() {
            body["systemInstruction"] = json!({"role": "system", "parts": system});
        }
        let safety: Vec<Value> = match self.options.get("safetySettings").map(|s| &s["values"]) {
            Some(Value::Array(v)) => v.clone(),
            Some(v @ Value::Object(_)) => vec![v.clone()],
            _ => vec![],
        };
        if !safety.is_empty() {
            body["safetySettings"] = Value::Array(safety);
        }
        if !tools.is_empty() {
            let declarations: Vec<Value> = tools
                .iter()
                .map(|t| json!({"name": t["function"]["name"], "description": t["function"]["description"], "parameters": gemini_schema(&t["function"]["parameters"])}))
                .collect();
            body["tools"] = json!([{"functionDeclarations": declarations}]);
        }
        body
    }

    /// Normalises the first candidate into the OpenAI-shaped message:
    /// non-thought `text` parts as `content`, `functionCall` parts as
    /// `tool_calls` (with a generated id when Gemini sends none, and the
    /// part's thought signature kept for the replay).
    fn gemini_response(&self, resp: &Value) -> (Value, Value) {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for (i, part) in resp.pointer("/candidates/0/content/parts").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(call) = part.get("functionCall") {
                let id = call["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{}_{i}", uuid::Uuid::new_v4().simple()));
                let arguments = serde_json::to_string(call.get("args").unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".to_string());
                let mut tc = json!({"id": id, "type": "function", "function": {"name": call["name"], "arguments": arguments}});
                if let Some(sig) = part["thoughtSignature"].as_str() {
                    tc["thought_signature"] = json!(sig);
                }
                tool_calls.push(tc);
            } else if part["thought"].as_bool() != Some(true) {
                text.push_str(part["text"].as_str().unwrap_or(""));
            }
        }
        let mut message = json!({"role": "assistant", "content": text});
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        let usage = &resp["usageMetadata"];
        let usage = json!({
            "completionTokens": usage["candidatesTokenCount"].as_i64().unwrap_or(0),
            "promptTokens": usage["promptTokenCount"].as_i64().unwrap_or(0),
            "totalTokens": usage["totalTokenCount"].as_i64().unwrap_or(0),
        });
        (message, usage)
    }

    /// Converts the OpenAI-shaped messages into Converse `messages` the way
    /// LangChain's `convertToConverseMessages` does: system messages become
    /// top-level `system` blocks, tool calls `toolUse` blocks, and tool
    /// replies `toolResult` blocks in a user turn (consecutive ones merged
    /// into one turn).
    fn bedrock_body(&self, messages: &[Value], tools: &[Value]) -> NodeResult<Value> {
        let mut system = Vec::new();
        let mut out: Vec<Value> = Vec::new();
        for m in messages {
            let text = m["content"].as_str().unwrap_or("");
            match m["role"].as_str() {
                Some("system") => system.push(json!({"text": text})),
                Some("user") => out.push(json!({"role": "user", "content": [{"text": text}]})),
                Some("assistant") => {
                    let mut content = Vec::new();
                    if !text.is_empty() {
                        content.push(json!({"text": text}));
                    }
                    for call in m["tool_calls"].as_array().into_iter().flatten() {
                        let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
                        let input: Value = serde_json::from_str(args).unwrap_or(json!({}));
                        content.push(json!({"toolUse": {"toolUseId": call["id"], "name": call.pointer("/function/name").cloned().unwrap_or(json!("")), "input": input}}));
                    }
                    out.push(json!({"role": "assistant", "content": content}));
                }
                Some("tool") => {
                    let block = json!({"toolResult": {"toolUseId": m["tool_call_id"], "content": [{"text": text}]}});
                    let has_result = |turn: &Value| turn["role"] == "user" && turn["content"].as_array().is_some_and(|c| c.iter().any(|b| b.get("toolResult").is_some()));
                    match out.last_mut() {
                        Some(last) if has_result(last) => last["content"].as_array_mut().expect("checked above").push(block),
                        _ => out.push(json!({"role": "user", "content": [block]})),
                    }
                }
                _ => {}
            }
        }
        let mut body = json!({"messages": out});
        if !system.is_empty() {
            body["system"] = Value::Array(system);
        }
        let opt = |k: &str| self.options.get(k).cloned();
        let mut inference = Map::new();
        for (n8n_key, key) in [("maxTokensToSample", "maxTokens"), ("temperature", "temperature"), ("topP", "topP")] {
            if let Some(v) = opt(n8n_key) {
                inference.insert(key.to_string(), v);
            }
        }
        if !inference.is_empty() {
            body["inferenceConfig"] = Value::Object(inference);
        }
        if !tools.is_empty() {
            let specs: Vec<Value> = tools
                .iter()
                .map(|t| json!({"toolSpec": {"name": t["function"]["name"], "description": t["function"]["description"], "inputSchema": {"json": t["function"]["parameters"]}}}))
                .collect();
            body["toolConfig"] = json!({"tools": specs});
        }
        if let Some(fields) = opt("additionalModelRequestFields").and_then(|v| v.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty() && s != "{}") {
            body["additionalModelRequestFields"] = serde_json::from_str(&fields).map_err(|_| NodeError::new("Additional Model Request Fields must be valid JSON"))?;
        }
        let guardrail = self.options.get("guardrail").map(|g| &g["values"]).cloned().unwrap_or(Value::Null);
        if let Some(id) = guardrail["guardrailIdentifier"].as_str().filter(|s| !s.is_empty()) {
            let version = guardrail["guardrailVersion"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("DRAFT");
            let mut config = json!({"guardrailIdentifier": id, "guardrailVersion": version});
            if let Some(trace) = guardrail["trace"].as_str().filter(|s| !s.is_empty()) {
                config["trace"] = json!(trace);
            }
            body["guardrailConfig"] = config;
        }
        // n8n also sets `performanceConfig` from the Latency option, but
        // LangChain 1.0.3 only reads it from call options, so it never
        // reaches the request; neither does it here.
        Ok(body)
    }

    /// Normalises the Converse output message: `text` blocks as `content`,
    /// `toolUse` blocks as `tool_calls`.
    fn bedrock_response(&self, resp: &Value) -> (Value, Value) {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for (i, block) in resp.pointer("/output/message/content").and_then(Value::as_array).into_iter().flatten().enumerate() {
            if let Some(t) = block["text"].as_str() {
                text.push_str(t);
            } else if let Some(call) = block.get("toolUse") {
                let id = call["toolUseId"].as_str().map(String::from).unwrap_or_else(|| format!("call_{i}"));
                let arguments = serde_json::to_string(call.get("input").unwrap_or(&json!({}))).unwrap_or_else(|_| "{}".to_string());
                tool_calls.push(json!({"id": id, "type": "function", "function": {"name": call["name"], "arguments": arguments}}));
            }
        }
        let mut message = json!({"role": "assistant", "content": text});
        if !tool_calls.is_empty() {
            message["tool_calls"] = Value::Array(tool_calls);
        }
        let input = resp["usage"]["inputTokens"].as_i64().unwrap_or(0);
        let output = resp["usage"]["outputTokens"].as_i64().unwrap_or(0);
        let total = resp["usage"]["totalTokens"].as_i64().unwrap_or(input + output);
        (message, json!({"completionTokens": output, "promptTokens": input, "totalTokens": total}))
    }

    fn endpoint(&self) -> String {
        match &self.provider {
            Provider::OpenAi { .. } => format!("{}/chat/completions", self.base_url),
            Provider::Ollama => format!("{}/api/chat", self.base_url),
            Provider::Anthropic => format!("{}/v1/messages", self.base_url),
            // As the Google SDK: a name with a `/` (`tunedModels/x`) is a
            // full resource path, anything else lives under `models/`.
            Provider::Gemini if self.model.contains('/') => format!("{}/v1beta/{}:generateContent", self.base_url, self.model.split('/').map(path_segment).collect::<Vec<_>>().join("/")),
            Provider::Gemini => format!("{}/v1beta/models/{}:generateContent", self.base_url, path_segment(&self.model)),
            Provider::AzureOpenAi { api_version } => format!("{}/chat/completions?api-version={}", self.base_url, path_segment(api_version)),
            Provider::Bedrock { .. } => format!("{}/model/{}/converse", self.base_url, path_segment(&self.model)),
        }
    }

    /// Message for an unreachable provider, mirroring n8n's generic
    /// `NodeApiError` wrap of the underlying fetch failure.
    fn unreachable_message(&self, err: reqwest::Error) -> String {
        match self.provider {
            Provider::OpenAi { .. } | Provider::Anthropic | Provider::Gemini | Provider::AzureOpenAi { .. } | Provider::Bedrock { .. } => format!("The model provider could not be reached: {}", err.without_url()),
            Provider::Ollama => format!("Ollama could not be reached at {}: {}", self.base_url, err.without_url()),
        }
    }

    fn error_message(&self, status: reqwest::StatusCode, json: &Value) -> String {
        match self.provider {
            // AWS errors carry `message` (or `Message`) at the top level.
            Provider::Bedrock { .. } => json["message"].as_str().or(json["Message"].as_str()).map(String::from).unwrap_or_else(|| format!("The model provider answered {status}")),
            Provider::OpenAi { .. } | Provider::Anthropic | Provider::Gemini | Provider::AzureOpenAi { .. } => json.pointer("/error/message").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("The model provider answered {status}")),
            Provider::Ollama => json["error"].as_str().map(String::from).unwrap_or_else(|| format!("Ollama answered {status}")),
        }
    }

    async fn send(&self, ctx: &ExecCtx<'_>, body: &Value) -> NodeResult<Value> {
        // Base URLs come from credentials and node options, so they go
        // through the same SSRF guard as every other outbound node.
        let url = reqwest::Url::parse(&self.endpoint()).map_err(|e| NodeError::new(format!("Invalid model provider URL: {e}")))?;
        super::check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        let mut req = ctx.services.http.post(url);
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
            Provider::AzureOpenAi { .. } => {
                req = req.header("api-key", &self.api_key);
            }
            Provider::Gemini => {
                req = req.header("x-goog-api-key", &self.api_key).header("x-goog-api-client", "genai-js/0.24.0");
            }
            Provider::Bedrock { .. } => {}
        }
        if let Provider::Bedrock { region, access_key_id, session_token } = &self.provider {
            let bytes = serde_json::to_vec(body).map_err(|e| NodeError::new(e.to_string()))?;
            let url = reqwest::Url::parse(&self.endpoint()).expect("parsed above");
            let host = match url.port() {
                Some(port) => format!("{}:{port}", url.host_str().unwrap_or("")),
                None => url.host_str().unwrap_or("").to_string(),
            };
            let creds = super::aws_sigv4::AwsCredentials { access_key_id, secret_access_key: &self.api_key, session_token: session_token.as_deref() };
            let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            for (name, value) in super::aws_sigv4::sign(&creds, region, "bedrock", "POST", &host, url.path(), url.query().unwrap_or(""), Some("application/json"), &bytes, &amz_date) {
                req = req.header(name, value);
            }
            req = req.header("content-type", "application/json").body(bytes);
        } else {
            req = req.json(body);
        }
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

/// The placeholder thought signature LangChain sends for a Gemini 3
/// function call it has no real signature for (`@langchain/google-genai`).
const GEMINI_DUMMY_SIGNATURE: &str = "ErYCCrMCAdHtim9kOoOkrPiCNVsmlpMIKd7ZMxgiFbVQOkgp7nlLcDMzVsZwIzvuT7nQROivoXA72ccC2lSDvR0Gh7dkWaGuj7ctv6t7ZceHnecx0QYa+ix8tYpRfjhyWozQ49lWiws6+YGjCt10KRTyWsZ2h6O7iHTYJwKIRwGUHRKy/qK/6kFxJm5ML00gLq4D8s5Z6DBpp2ZlR+uF4G8jJgeWQgyHWVdx2wGYElaceVAc66tZdPQRdOHpWtgYSI1YdaXgVI8KHY3/EfNc2YqqMIulvkDBAnuMhkAjV9xmBa54Tq+ih3Im4+r3DzqhGqYdsSkhS0kZMwte4Hjs65dZzCw9lANxIqYi1DJ639WNPYihp/DCJCos7o+/EeSPJaio5sgWDyUnMGkY1atsJZ+m7pj7DD5tvQ==";

/// Rejects a model/deployment name whose `/`-separated parts would be dot
/// segments (or empty) once in a URL path: URL parsing resolves those,
/// even percent-encoded, which would send the request to another path.
fn check_path_name(name: &str, what: &str) -> NodeResult<()> {
    if name.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Err(NodeError::new(format!("Invalid {what} name \"{name}\"")));
    }
    Ok(())
}

/// Percent-encodes everything but RFC 3986 unreserved characters, so a
/// model or deployment name (which may come from an expression) stays one
/// path segment instead of steering the request elsewhere (`../`, `?`, `#`).
fn path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Gemini's function parameters accept only a subset of JSON Schema:
/// strips `additionalProperties` (everywhere) and `$schema`, like
/// LangChain's `removeAdditionalProperties`.
fn gemini_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(o) => Value::Object(o.iter().filter(|(k, _)| *k != "additionalProperties" && *k != "$schema").map(|(k, v)| (k.clone(), gemini_schema(v))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(gemini_schema).collect()),
        v => v.clone(),
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

/// A native node used as an AI tool (`<type>Tool`): only runs when an
/// agent calls it, as its base node with the model's arguments as input.
pub struct NodeAsTool {
    pub name: &'static str,
}

#[async_trait::async_trait]
impl NodeType for NodeAsTool {
    fn type_name(&self) -> &'static str {
        self.name
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        Err(NodeError::new(format!("\"{}\" is a tool: connect it to an AI Agent", ctx.node.name)))
    }
}

/// The node implementations a node-as-tool runs (the registry is
/// stateless, so one shared copy serves every run).
fn registry() -> &'static super::Registry {
    static REGISTRY: OnceLock<super::Registry> = OnceLock::new();
    REGISTRY.get_or_init(super::Registry::default)
}

enum ToolKind {
    Calculator,
    /// Runs `base` with the model's arguments as its one input item.
    Node { base: &'static str, description: String, args: Vec<FromAi> },
    /// The Code Tool: JavaScript run with `query` = the model's input, a
    /// single string unless an input schema is given.
    Code { description: String, code: String, schema: Option<Value> },
    /// The Workflow Tool: runs a sub-workflow with the model's input. With
    /// workflow inputs mapped through `$fromAI` it takes those arguments,
    /// otherwise a single string.
    Workflow { description: String, workflow_id: String, args: Option<Vec<FromAi>> },
    /// A vector store in "retrieve-as-tool" mode: an `input` query (plus any
    /// `$fromAI` arguments) answered with the closest documents.
    VectorStore { description: String, args: Vec<FromAi>, k: usize, with_metadata: bool },
}

struct Tool<'a> {
    node: &'a Node,
    /// n8n's `nodeNameToToolName`: the node name, other characters as `_`
    /// (a Code Tool before v1.2 names itself).
    name: String,
    kind: ToolKind,
}

fn tool_name(node: &Node) -> String {
    let mut name = regex::Regex::new(r"[^a-zA-Z0-9_-]+").unwrap().replace_all(&node.name, "_").to_string();
    if name.len() > 64 {
        name.truncate(64);
        name = name.trim_end_matches(['_', '-']).to_string();
    }
    name
}

fn load_tools<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Vec<Tool<'a>>> {
    let mut tools = Vec::new();
    for name in ctx.workflow.sub_nodes(&ctx.node.name, "ai_tool") {
        let node = ctx.workflow.node(&name).expect("connected nodes exist");
        if node.disabled {
            continue;
        }
        let mut name = tool_name(node);
        let kind = if node.node_type == format!("{LC}toolCalculator") {
            ToolKind::Calculator
        } else if node.node_type == format!("{LC}toolCode") {
            let p = ctx.resolve_value(&node.parameters, item)?;
            if node.type_version <= 1.1 {
                name = p["name"].as_str().unwrap_or("").to_string();
            }
            if p["language"].as_str().unwrap_or("javaScript") != "javaScript" {
                return Err(NodeError::new(format!("The Code Tool \"{}\" uses Python, which is not supported natively yet", node.name)));
            }
            ToolKind::Code { description: p["description"].as_str().unwrap_or("").to_string(), code: node.parameters["jsCode"].as_str().unwrap_or("").to_string(), schema: code_tool_schema(node, &p)? }
        } else if [format!("{LC}vectorStoreInMemory"), format!("{LC}vectorStorePGVector"), format!("{LC}vectorStoreQdrant"), format!("{LC}vectorStorePinecone"), format!("{LC}vectorStoreSupabase")].contains(&node.node_type) && node.parameters["mode"].as_str() == Some("retrieve-as-tool") {
            let p = ctx.resolve_value(&node.parameters, item)?;
            if node.type_version < 1.3 {
                name = p["toolName"].as_str().unwrap_or("").to_string();
            }
            let mut args = tool_arguments(node)?;
            args.push(FromAi { key: "input".into(), description: Some("Query to search for. Required".into()), kind: "string".into(), default: None });
            ToolKind::VectorStore {
                description: p["toolDescription"].as_str().unwrap_or("").to_string(),
                args,
                k: p["topK"].as_f64().unwrap_or(4.0) as usize,
                with_metadata: p["includeDocumentMetadata"].as_bool().unwrap_or(true),
            }
        } else if node.node_type == format!("{LC}toolWorkflow") {
            let p = &node.parameters;
            if node.type_version <= 2.1 {
                name = ctx.resolve_value(&p["name"], item)?.as_str().unwrap_or("").to_string();
            }
            if node.type_version < 2.0 {
                return Err(NodeError::new(format!("The Workflow Tool \"{}\" is version {}; only version 2 and later are supported natively", node.name, node.type_version)));
            }
            if p["source"].as_str().unwrap_or("database") != "database" {
                return Err(NodeError::new(format!("The Workflow Tool \"{}\" defines its workflow inline, which is not supported natively yet; pick a saved workflow", node.name)));
            }
            let workflow_id = match ctx.resolve_value(&p["workflowId"], item)? {
                Value::Object(o) => o.get("value").map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default(),
                Value::String(s) => s,
                _ => String::new(),
            };
            // n8n's WorkflowToolService: a structured tool only when the
            // workflow inputs have a schema and the parameters use $fromAI.
            let use_schema = p.pointer("/workflowInputs/schema").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
            let args = if use_schema { Some(tool_arguments(node)?).filter(|a: &Vec<FromAi>| !a.is_empty()) } else { None };
            let description = ctx.resolve_value(&p["description"], item)?.as_str().unwrap_or("").to_string();
            ToolKind::Workflow { description, workflow_id, args }
        } else if let Some(base) = registry().tool_base(&node.node_type).filter(|_| !registry().is_excluded(&node.node_type, &ctx.config().nodes_exclude)) {
            ToolKind::Node { base, description: tool_description(ctx, node, base, item)?, args: tool_arguments(node)? }
        } else {
            return Err(NodeError::new(format!("The tool \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
        };
        tools.push(Tool { node, name, kind });
    }
    Ok(tools)
}

/// n8n's `getToolDescriptionForNode`: the manual `toolDescription` (an
/// expression is resolved in the agent's context), else one made from
/// resource and operation, else the node type's description.
fn tool_description(ctx: &ExecCtx<'_>, node: &Node, base: &str, item: usize) -> NodeResult<String> {
    let (display, fallback) = crate::n8n::node_types::summary(base).unwrap_or(("", ""));
    let p = &node.parameters;
    let manual = p["toolDescription"].as_str().filter(|s| !s.trim().is_empty());
    match manual {
        Some(raw) if p["descriptionType"].as_str() != Some("auto") => {
            if raw.starts_with('=') {
                Ok(ctx.resolve_value(&json!(raw), item)?.as_str().map(String::from).unwrap_or_default())
            } else {
                Ok(raw.to_string())
            }
        }
        _ => match (p["resource"].as_str(), p["operation"].as_str()) {
            (Some(resource), Some(operation)) => Ok(format!("{operation} {resource} in {display}")),
            _ => Ok(fallback.to_string()),
        },
    }
}

/// A Code Tool's input schema when "Specify Input Schema" is on: from a
/// JSON example (all fields required from v1.3, as n8n) or given as JSON
/// Schema; objects get `additionalProperties: false` like LangChain's zod
/// round trip.
fn code_tool_schema(node: &Node, p: &Value) -> NodeResult<Option<Value>> {
    if p["specifyInputSchema"].as_bool() != Some(true) {
        return Ok(None);
    }
    let bad = |e: String| NodeError::new(format!("Error during parsing of JSON Schema. \n {e}"));
    let mut schema = if p["schemaType"].as_str() == Some("manual") {
        serde_json::from_str::<Value>(p["inputSchema"].as_str().unwrap_or("")).map_err(|e| bad(e.to_string()))?
    } else {
        let example: Value = serde_json::from_str(p["jsonSchemaExample"].as_str().unwrap_or("")).map_err(|e| bad(e.to_string()))?;
        schema_from_example(&example, node.type_version >= 1.3)
    };
    close_objects(&mut schema);
    if let Some(o) = schema.as_object_mut() {
        o.remove("$schema");
    }
    Ok(Some(schema))
}

/// The `generate-schema` package's `json()`, as n8n uses it.
pub(super) fn schema_from_example(v: &Value, all_required: bool) -> Value {
    match v {
        Value::Object(o) => {
            let props: Map<String, Value> = o.iter().map(|(k, v)| (k.clone(), schema_from_example(v, all_required))).collect();
            let mut s = json!({"type": "object", "properties": props});
            if all_required && !o.is_empty() {
                s["required"] = json!(o.keys().collect::<Vec<_>>());
            }
            s
        }
        Value::Array(a) => json!({"type": "array", "items": a.first().map(|x| schema_from_example(x, all_required)).unwrap_or(json!({}))}),
        Value::String(_) => json!({"type": "string"}),
        Value::Number(_) => json!({"type": "number"}),
        Value::Bool(_) => json!({"type": "boolean"}),
        Value::Null => json!({"type": "null"}),
    }
}

pub(super) fn close_objects(s: &mut Value) {
    if let Some(o) = s.as_object_mut() {
        if o.get("type").and_then(Value::as_str) == Some("object") && !o.contains_key("additionalProperties") {
            o.insert("additionalProperties".into(), json!(false));
        }
        for v in o.values_mut() {
            close_objects(v);
        }
    } else if let Some(a) = s.as_array_mut() {
        a.iter_mut().for_each(close_objects);
    }
}

/// One `$fromAI(key, description, type, default)` call found in a node's
/// parameters (n8n's `extractFromAICalls`).
#[derive(Clone, Debug, PartialEq)]
struct FromAi {
    key: String,
    description: Option<String>,
    kind: String,
    default: Option<Value>,
}

/// Every `$fromAI` call in the node's parameters, deduplicated by key, with
/// n8n's errors for bad or conflicting keys.
fn tool_arguments(node: &Node) -> NodeResult<Vec<FromAi>> {
    fn walk(v: &Value, out: &mut Vec<FromAi>) -> Result<(), String> {
        match v {
            Value::String(s) => out.extend(extract_from_ai(s)?),
            Value::Array(a) => a.iter().try_for_each(|x| walk(x, out))?,
            Value::Object(o) => o.values().try_for_each(|x| walk(x, out))?,
            _ => {}
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(&node.parameters, &mut found).map_err(NodeError::new)?;
    let valid = regex::Regex::new(r"^[a-zA-Z0-9_-]{1,64}$").unwrap();
    let mut unique: Vec<FromAi> = Vec::new();
    for arg in found {
        if !valid.is_match(&arg.key) {
            let msg = if arg.key.is_empty() { "You must specify a key when using $fromAI()".to_string() } else { format!("Parameter key `{}` is invalid", arg.key) };
            return Err(NodeError::new(msg).describe("Invalid parameter key, must be between 1 and 64 characters long and only contain letters, numbers, underscores, and hyphens"));
        }
        match unique.iter_mut().find(|u| u.key == arg.key) {
            Some(existing) if existing.description != arg.description || existing.kind != arg.kind => {
                return Err(NodeError::new(format!("Duplicate key '{}' found with different description or type", arg.key))
                    .describe("Ensure all $fromAI() calls with the same key have consistent descriptions and types"));
            }
            // n8n keeps the last occurrence of a key.
            Some(existing) => *existing = arg,
            None => unique.push(arg),
        }
    }
    Ok(unique)
}

/// Parses the `$fromAI(...)` calls in one string (n8n's
/// `extractFromAICalls` + `parseArguments`).
fn extract_from_ai(s: &str) -> Result<Vec<FromAi>, String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let re = regex::Regex::new(r"(?i)\$fromAI\s*\(\s*").unwrap();
    for m in re.find_iter(s) {
        // Byte offset -> char offset.
        let mut i = s[..m.end()].chars().count();
        let (mut depth, mut quote, mut args) = (1, None::<char>, String::new());
        while i < chars.len() && depth > 0 {
            let c = chars[i];
            if let Some(q) = quote {
                if c == '\\' && i + 1 < chars.len() {
                    args.push(c);
                    args.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                args.push(c);
            } else {
                match c {
                    '"' | '\'' | '`' => quote = Some(c),
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                if depth > 0 || c != ')' {
                    args.push(c);
                }
            }
            i += 1;
        }
        if depth != 0 {
            return Err(format!("Unbalanced parentheses while parsing $fromAI call: {}", &s[m.end()..]));
        }
        out.push(parse_from_ai_args(&args).map_err(|e| format!("Failed to parse $fromAI arguments: {args}: Error: {e}"))?);
    }
    Ok(out)
}

fn parse_from_ai_args(args: &str) -> Result<FromAi, String> {
    let mut parts = Vec::new();
    let (mut current, mut quote, mut escape) = (String::new(), None::<char>, false);
    for c in args.chars() {
        if escape {
            current.push(c);
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if matches!(c, '"' | '\'' | '`') {
            match quote {
                None => quote = Some(c),
                Some(q) if q == c => quote = None,
                _ => {}
            }
            current.push(c);
            continue;
        }
        if c == ',' && quote.is_none() {
            parts.push(current.trim().to_string());
            current.clear();
            continue;
        }
        current.push(c);
    }
    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }
    let clean: Vec<String> = parts
        .iter()
        .map(|p| {
            let t = p.trim();
            let quoted = t.len() >= 2 && [('\'', '\''), ('`', '`'), ('"', '"')].iter().any(|(a, b)| t.starts_with(*a) && t.ends_with(*b));
            if quoted {
                t[1..t.len() - 1].replace("\\'", "'").replace("\\`", "`").replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                t.to_string()
            }
        })
        .collect();
    let raw_type = clean.get(2).cloned().unwrap_or_else(|| "string".into());
    let kind = raw_type.to_lowercase();
    if !["string", "number", "boolean", "json"].contains(&kind.as_str()) {
        return Err(format!("Invalid type: {raw_type}"));
    }
    let default = clean.get(3).map(|v| match kind.as_str() {
        "string" => json!(v),
        "boolean" if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") => json!(v.eq_ignore_ascii_case("true")),
        "number" if v.parse::<f64>().is_ok() => serde_json::from_str(v).unwrap_or_else(|_| json!(v.parse::<f64>().unwrap())),
        _ => serde_json::from_str(v).unwrap_or_else(|_| json!(v)),
    });
    Ok(FromAi { key: clean.first().cloned().unwrap_or_default(), description: clean.get(1).cloned(), kind, default })
}

/// n8n's `generateZodSchema` for each `$fromAI` argument, as JSON schema;
/// every key required.
fn from_ai_schema(args: &[FromAi]) -> Value {
    let mut props = Map::new();
    for a in args {
        let mut prop = match a.kind.as_str() {
            "json" => json!({"anyOf": [{"type": "object", "minProperties": 1, "additionalProperties": true}, {"type": "array", "minItems": 1}]}),
            k => json!({"type": k}),
        };
        if let Some(d) = a.description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            prop["description"] = json!(d);
        }
        if let Some(d) = &a.default {
            prop["default"] = d.clone();
        }
        props.insert(a.key.clone(), prop);
    }
    let required: Vec<&String> = args.iter().map(|a| &a.key).collect();
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

/// A context for running `node` as a tool: its one input item is `json`
/// (what `$json` and `$fromAI` read), the rest as in the agent's run.
fn tool_ctx<'a>(ctx: &ExecCtx<'a>, node: &'a Node, json: Map<String, Value>) -> ExecCtx<'a> {
    let item = Item::new(json);
    let mut data = (ctx.expr_data)();
    data["input"] = json!([item]);
    data["inputs"] = json!([[item]]);
    data["source"] = json!([null]);
    data["node"] = json!({"name": node.name, "type": node.node_type, "parameters": node.parameters});
    let run_index = ctx.run.lock().unwrap().sub_runs.iter().filter(|(n, _)| *n == node.name).count();
    ExecCtx::new(node, ctx.workflow, vec![vec![item]], run_index, ctx.mode, ctx.execution_id.clone(), ctx.services, ctx.options, ctx.run.clone(), Box::new(move || data.clone()))
}

impl Tool<'_> {
    fn schema(&self) -> Value {
        let (description, parameters) = match &self.kind {
            ToolKind::Calculator => (
                "Useful for getting the result of a math expression. The input to this tool should be a valid mathematical expression that could be executed by a simple calculator.".to_string(),
                json!({"type": "object", "properties": {"input": {"type": "string"}}, "required": ["input"], "additionalProperties": false}),
            ),
            ToolKind::Node { description, args, .. } | ToolKind::VectorStore { description, args, .. } => (description.clone(), from_ai_schema(args)),
            ToolKind::Workflow { description, args: Some(args), .. } => (description.clone(), from_ai_schema(args)),
            ToolKind::Workflow { description, args: None, .. } => (description.clone(), json!({"type": "object", "properties": {"input": {"type": "string"}}, "additionalProperties": false})),
            // Without a schema, LangChain's `DynamicTool`: one optional string.
            ToolKind::Code { description, schema, .. } => (
                description.clone(),
                schema.clone().unwrap_or_else(|| json!({"type": "object", "properties": {"input": {"type": "string"}}, "additionalProperties": false})),
            ),
        };
        json!({"type": "function", "function": {"name": self.name, "description": description, "parameters": parameters}})
    }

    /// Runs the Workflow Tool's sub-workflow on one item: the tool input as
    /// `query` plus the workflow inputs (resolved against it, so `$fromAI`
    /// reads the model's arguments), minus inputs the schema removed. Returns
    /// the last node's items (from v2.1; the first item's JSON before).
    async fn run_workflow(&self, ctx: &ExecCtx<'_>, workflow_id: &str, query: &Value, use_schema: bool) -> NodeResult<Vec<Value>> {
        let runner = ctx.services.sub_workflows.clone().ok_or_else(|| NodeError::new("Sub-workflows need a running r8r server"))?;
        let p = &self.node.parameters;
        let mut json = Map::from_iter([("query".to_string(), query.clone())]);
        if use_schema {
            let sub = tool_ctx(ctx, self.node, json.clone());
            if let Value::Object(values) = sub.resolve_value(&p.pointer("/workflowInputs/value").cloned().unwrap_or(json!({})), 0)? {
                json.extend(values);
            }
            for field in p.pointer("/workflowInputs/schema").and_then(Value::as_array).into_iter().flatten() {
                if field["removed"].as_bool() == Some(true) {
                    if let Some(name) = field["displayName"].as_str() {
                        json.remove(name);
                    }
                }
            }
        } else if let Value::Object(o) = query {
            json = o.clone();
        }
        let items = runner.run_sub_workflow(ctx.workflow.id.as_deref(), workflow_id, vec![Item::new(json)], true).await?;
        let mut out: Vec<Value> = items.into_iter().map(|i| Value::Object(i.json)).collect();
        if self.node.type_version <= 2.0 {
            out.truncate(1);
        }
        if out.is_empty() {
            return Err(NodeError::new("There was an error: \"The workflow did not return a response\""));
        }
        Ok(out)
    }

    async fn call(&self, ctx: &ExecCtx<'_>, arguments: &str) -> NodeResult<String> {
        let started = now_ms();
        match &self.kind {
            ToolKind::Calculator => {
                let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({"input": arguments}));
                let input = args["input"].as_str().map(String::from).unwrap_or_else(|| args.to_string());
                let result = match calculate(&input) {
                    Ok(n) => format_number(n),
                    Err(e) => format!("Error: {e}"),
                };
                record(ctx, &self.node.name, "ai_tool", json!({"input": input}), Ok(json!({"response": result})), started);
                Ok(result)
            }
            ToolKind::Code { code, schema, .. } => {
                let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({"input": arguments}));
                let query = if schema.is_some() { args } else { args.get("input").cloned().unwrap_or(Value::Null) };
                // n8n's Code Tool handler: numbers become strings, anything
                // else that isn't a string is an error the model is told about.
                let outcome = match super::code::run_js_tool(ctx, code, &query) {
                    Ok(Some(Value::String(s))) => Ok(s),
                    Ok(Some(Value::Number(n))) => Ok(n.to_string()),
                    Ok(other) => {
                        let kind = match other {
                            None | Some(Value::Null) => "undefined",
                            Some(Value::Bool(_)) => "boolean",
                            _ => "object",
                        };
                        Err(NodeError::new("Wrong output type returned").describe(format!("The response property should be a string, but it is an {kind}")))
                    }
                    Err(e) => Err(e),
                };
                match outcome {
                    Ok(response) => {
                        record(ctx, &self.node.name, "ai_tool", json!({"query": query}), Ok(json!({"response": response})), started);
                        Ok(response)
                    }
                    Err(e) => {
                        record(ctx, &self.node.name, "ai_tool", json!({"query": query}), Err(&e), started);
                        Ok(format!("There was an error: \"{}\"", e.message))
                    }
                }
            }
            ToolKind::Workflow { workflow_id, args: schema, .. } => {
                let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({"input": arguments}));
                let query = if schema.is_some() { args } else { args.get("input").cloned().unwrap_or(Value::Null) };
                let input = json!({"query": query});
                match self.run_workflow(ctx, workflow_id, &query, schema.is_some()).await {
                    Ok(items) => {
                        // n8n's handleToolResponse: the items' JSON (v2.0: the
                        // first item's), pretty-printed.
                        let response = if self.node.type_version <= 2.0 { serde_json::to_string_pretty(&items[0]) } else { serde_json::to_string_pretty(&items) }.unwrap_or_default();
                        record_items(ctx, &self.node.name, "ai_tool", input, Ok(items), started);
                        Ok(response)
                    }
                    // The error goes to the model, and is logged as the
                    // tool's (successful) output, as n8n does.
                    Err(e) => {
                        let response = format!("There was an error: \"{}\"", e.message);
                        record(ctx, &self.node.name, "ai_tool", input, Ok(json!({"error": response})), started);
                        Ok(response)
                    }
                }
            }
            ToolKind::VectorStore { k, with_metadata, .. } => {
                let args: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!({"input": arguments}));
                let query = args["input"].as_str().map(String::from).or_else(|| args.as_str().map(String::from)).unwrap_or_default();
                // Each hit as a text content block holding the document JSON;
                // the model gets the block list as JSON text, as from n8n.
                let blocks: Vec<Value> = match super::ai_vector::store_search(ctx, self.node, &query, *k, 0, false).await {
                    Ok(hits) => hits
                        .iter()
                        .map(|h| {
                            let doc = if *with_metadata { h.to_json() } else { json!({"pageContent": h.doc.page_content}) };
                            json!({"type": "text", "text": doc.to_string()})
                        })
                        .collect(),
                    Err(e) => {
                        record(ctx, &self.node.name, "ai_tool", args.clone(), Err(&e), started);
                        return Ok(String::new());
                    }
                };
                record(ctx, &self.node.name, "ai_tool", args, Ok(json!({"response": blocks})), started);
                Ok(Value::Array(blocks).to_string())
            }
            ToolKind::Node { base, .. } => {
                let args = match serde_json::from_str::<Value>(arguments) {
                    Ok(Value::Object(m)) => m,
                    _ => Map::new(),
                };
                let mut sub = tool_ctx(ctx, self.node, args.clone());
                let node_type = registry().get(base).expect("tool bases are registered");
                match node_type.execute(&mut sub).await {
                    Ok(outputs) => {
                        // n8n's `mapResult`: the first output's JSON, stringified.
                        let response: Vec<Value> = outputs.into_iter().next().unwrap_or_default().into_iter().map(|i| Value::Object(i.json)).collect();
                        let response = Value::Array(response);
                        record(ctx, &self.node.name, "ai_tool", Value::Object(args), Ok(json!({"response": response})), started);
                        Ok(serde_json::to_string(&response).unwrap_or_default())
                    }
                    // n8n's agent records the tool's error and hands the model
                    // an empty result; the run carries on.
                    Err(e) => {
                        record(ctx, &self.node.name, "ai_tool", Value::Object(args), Err(&e), started);
                        Ok(String::new())
                    }
                }
            }
        }
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

/// Where a memory sub-node keeps the conversation.
enum MemoryStore {
    /// In-process, scoped to the workflow and node (n8n's Simple Memory).
    Simple { key: String },
    /// LangChain's `PostgresChatMessageHistory`: rows `(id, session_id,
    /// message JSONB)`, the message being `{type, content, ...}`.
    Postgres { cred: Value, table: String, session: String },
    /// LangChain's `RedisChatMessageHistory`: a list at the session key,
    /// newest first (LPUSH), entries `{type, data: {content, ...}}`.
    Redis { cred: Value, session: String, ttl: i64 },
}

struct Memory<'a> {
    node: &'a Node,
    store: MemoryStore,
    /// Exchanges sent to the model (`None`: all, LangChain's `BufferMemory`
    /// in early node versions).
    window: Option<usize>,
}

/// n8n's `getSessionId`: the input's `sessionId` or the node's key.
fn session_id(ctx: &ExecCtx<'_>, params: &Value, item: usize) -> NodeResult<String> {
    let session = if params["sessionIdType"].as_str().unwrap_or("fromInput") == "fromInput" {
        let s = ctx.input().get(item).and_then(|i| i.json.get("sessionId")).and_then(Value::as_str).unwrap_or("").to_string();
        if s.is_empty() {
            return Err(NodeError::new("No session ID found").describe("Expected to find the session ID in an input field called 'sessionId' (this is what the chat trigger node outputs). To use something else, change the 'Session ID' parameter").at(item));
        }
        s
    } else {
        let s = params["sessionKey"].as_str().map(String::from).unwrap_or_else(|| params["sessionKey"].to_string().trim_matches('"').to_string());
        if s.is_empty() || params["sessionKey"].is_null() {
            return Err(NodeError::new("Key parameter is empty").describe("Provide a key to use as session ID in the 'Key' parameter or use the 'Connected Chat Trigger Node' option to use the session ID from your Chat Trigger").at(item));
        }
        s
    };
    Ok(session)
}

fn load_memory<'a>(ctx: &'a ExecCtx<'_>, item: usize) -> NodeResult<Option<Memory<'a>>> {
    let Some(name) = ctx.workflow.sub_nodes(&ctx.node.name, "ai_memory").into_iter().next() else { return Ok(None) };
    let node = ctx.workflow.node(&name).expect("connected nodes exist");
    if node.disabled {
        return Ok(None);
    }
    let params = ctx.resolve_value(&node.parameters, item)?;
    let k = params["contextWindowLength"].as_u64().unwrap_or(5).max(1) as usize;
    let v = node.type_version;
    let (store, window) = if node.node_type == format!("{LC}memoryBufferWindow") {
        let workflow = ctx.workflow.id.clone().unwrap_or_default();
        (MemoryStore::Simple { key: format!("{workflow}:{}:{}", node.name, session_id(ctx, &params, item)?) }, Some(k))
    } else if node.node_type == format!("{LC}memoryPostgresChat") {
        let table = params["tableName"].as_str().filter(|t| !t.is_empty()).unwrap_or("n8n_chat_histories").to_string();
        // n8n puts the name into the SQL as is; r8r only takes plain
        // identifiers (optionally schema-qualified) so it can't inject SQL.
        if !regex::Regex::new(r"^[A-Za-z_][A-Za-z0-9_$]*(\.[A-Za-z_][A-Za-z0-9_$]*)?$").unwrap().is_match(&table) {
            return Err(NodeError::new(format!("Invalid table name \"{table}\"")).describe("Use letters, digits and underscores, optionally as schema.table"));
        }
        (MemoryStore::Postgres { cred: Value::Null, table, session: session_id(ctx, &params, item)? }, (v >= 1.1).then_some(k))
    } else if node.node_type == format!("{LC}memoryRedisChat") {
        let session = if v >= 1.2 { session_id(ctx, &params, item)? } else { params["sessionKey"].as_str().unwrap_or("").to_string() };
        (MemoryStore::Redis { cred: Value::Null, session, ttl: params["sessionTTL"].as_i64().unwrap_or(0) }, (v >= 1.3).then_some(k))
    } else {
        return Err(NodeError::new(format!("The memory \"{}\" ({}) is not supported natively yet", node.name, node.node_type)));
    };
    Ok(Some(Memory { node, store, window }))
}

/// A LangChain stored message (`toDict()` data) as an OpenAI-shaped chat
/// message; `None` for kinds the agent doesn't replay (tool messages).
fn from_stored(kind: &str, data: &Value) -> Option<Value> {
    let role = match kind {
        "human" => "user",
        "ai" => "assistant",
        "system" => "system",
        _ => return None,
    };
    let content = match &data["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str().or(p.as_str())).collect::<Vec<_>>().join(""),
        _ => String::new(),
    };
    Some(json!({"role": role, "content": content}))
}

/// What LangChain's `toDict()` writes for a human or AI message.
fn to_stored(kind: &str, content: &str) -> Value {
    if kind == "ai" {
        json!({"content": content, "tool_calls": [], "invalid_tool_calls": [], "additional_kwargs": {}, "response_metadata": {}})
    } else {
        json!({"content": content, "additional_kwargs": {}, "response_metadata": {}})
    }
}

impl Memory<'_> {
    /// Fills in the credentials (resolved per run, as n8n's `supplyData`).
    async fn connect(mut self, ctx: &ExecCtx<'_>) -> NodeResult<Self> {
        match &mut self.store {
            MemoryStore::Postgres { cred, .. } => *cred = ctx.credentials_for(self.node, "postgres").await?.1,
            MemoryStore::Redis { cred, .. } => *cred = ctx.credentials_for(self.node, "redis").await?.1,
            MemoryStore::Simple { .. } => {}
        }
        Ok(self)
    }

    async fn stored(&self) -> NodeResult<Vec<Value>> {
        match &self.store {
            MemoryStore::Simple { key } => Ok(sessions().lock().unwrap().get(key).cloned().unwrap_or_default()),
            MemoryStore::Postgres { cred, table, session } => {
                let (cred, table, session) = (cred.clone(), table.clone(), session.clone());
                tokio::spawn(async move {
                    let pool = super::postgres::connect(&cred, 30).await?;
                    let pg = |e: sqlx::Error| NodeError::new(e.to_string());
                    sqlx::query(&format!("CREATE TABLE IF NOT EXISTS {table} (id SERIAL PRIMARY KEY, session_id VARCHAR(255) NOT NULL, message JSONB NOT NULL)")).execute(&pool).await.map_err(pg)?;
                    let rows = sqlx::query(&format!("SELECT message::text AS m FROM {table} WHERE session_id = $1 ORDER BY id")).bind(&session).fetch_all(&pool).await.map_err(pg)?;
                    pool.close().await;
                    Ok(rows
                        .iter()
                        .filter_map(|r| serde_json::from_str::<Value>(&sqlx::Row::get::<String, _>(r, "m")).ok())
                        .filter_map(|m| from_stored(m["type"].as_str().unwrap_or(""), &m))
                        .collect())
                })
                .await
                .map_err(|e| NodeError::new(e.to_string()))?
            }
            MemoryStore::Redis { cred, session, .. } => {
                let mut conn = super::redis::connect(&super::redis::read_creds(cred)).await?;
                let raw: Vec<String> = redis::cmd("LRANGE").arg(session).arg(0).arg(-1).query_async(&mut conn).await.map_err(|e| NodeError::new(format!("Redis Error: {e}")))?;
                Ok(raw
                    .iter()
                    .rev()
                    .filter_map(|s| serde_json::from_str::<Value>(s).ok())
                    .filter_map(|m| from_stored(m["type"].as_str().unwrap_or(""), &m["data"]))
                    .collect())
            }
        }
    }

    async fn load(&self, ctx: &ExecCtx<'_>) -> NodeResult<Vec<Value>> {
        let started = now_ms();
        let all = match self.stored().await {
            Ok(all) => all,
            Err(e) => {
                record(ctx, &self.node.name, "ai_memory", json!({"action": "loadMemoryVariables"}), Err(&e), started);
                return Err(e);
            }
        };
        let history: Vec<Value> = match self.window {
            Some(k) => all[all.len().saturating_sub(k * 2)..].to_vec(),
            None => all,
        };
        record(ctx, &self.node.name, "ai_memory", json!({"action": "loadMemoryVariables"}), Ok(json!({"action": "loadMemoryVariables", "chatHistory": history})), started);
        Ok(history)
    }

    async fn append(&self, user: &str, assistant: &str) -> NodeResult<()> {
        match &self.store {
            MemoryStore::Simple { key } => {
                let mut s = sessions().lock().unwrap();
                let list = s.entry(key.clone()).or_default();
                list.extend([json!({"role": "user", "content": user}), json!({"role": "assistant", "content": assistant})]);
                let excess = list.len().saturating_sub(self.window.unwrap_or(usize::MAX / 2) * 2);
                list.drain(..excess);
                Ok(())
            }
            MemoryStore::Postgres { cred, table, session } => {
                let (cred, table, session) = (cred.clone(), table.clone(), session.clone());
                let rows: Vec<String> = [("human", user), ("ai", assistant)]
                    .iter()
                    .map(|(kind, content)| {
                        let mut m = to_stored(kind, content);
                        m["type"] = json!(kind);
                        m.to_string()
                    })
                    .collect();
                tokio::spawn(async move {
                    let pool = super::postgres::connect(&cred, 30).await?;
                    let pg = |e: sqlx::Error| NodeError::new(e.to_string());
                    sqlx::query(&format!("CREATE TABLE IF NOT EXISTS {table} (id SERIAL PRIMARY KEY, session_id VARCHAR(255) NOT NULL, message JSONB NOT NULL)")).execute(&pool).await.map_err(pg)?;
                    for row in rows {
                        sqlx::query(&format!("INSERT INTO {table} (session_id, message) VALUES ($1, $2::jsonb)")).bind(&session).bind(row).execute(&pool).await.map_err(pg)?;
                    }
                    pool.close().await;
                    Ok(())
                })
                .await
                .map_err(|e| NodeError::new(e.to_string()))?
            }
            MemoryStore::Redis { cred, session, ttl } => {
                let mut conn = super::redis::connect(&super::redis::read_creds(cred)).await?;
                let redis_err = |e: redis::RedisError| NodeError::new(format!("Redis Error: {e}"));
                for (kind, content) in [("human", user), ("ai", assistant)] {
                    let entry = json!({"type": kind, "data": to_stored(kind, content)});
                    let _: i64 = redis::cmd("LPUSH").arg(session).arg(entry.to_string()).query_async(&mut conn).await.map_err(redis_err)?;
                    if *ttl > 0 {
                        let _: i64 = redis::cmd("EXPIRE").arg(session).arg(*ttl).query_async(&mut conn).await.map_err(redis_err)?;
                    }
                }
                Ok(())
            }
        }
    }

    async fn save(&self, ctx: &ExecCtx<'_>, user: &str, assistant: &str) -> NodeResult<()> {
        let started = now_ms();
        let input = json!({"action": "saveContext", "input": user, "output": assistant});
        match self.append(user, assistant).await {
            Ok(()) => {
                record(ctx, &self.node.name, "ai_memory", input, Ok(json!({"action": "saveContext"})), started);
                Ok(())
            }
            Err(e) => {
                record(ctx, &self.node.name, "ai_memory", input, Err(&e), started);
                Err(e)
            }
        }
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
                Ok(Value::Object(json)) => out.push(Item::new(json).paired(i)),
                Ok(other) => out.push(Item::new(Map::from_iter([("output".to_string(), other)])).paired(i)),
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

/// The formatting instructions the Tools Agent adds with an output parser.
const FORMATTING_INSTRUCTIONS: &str = "IMPORTANT: For your response to user, you MUST use the `format_final_json_response` tool with your complete answer formatted according to the required schema. Do not attempt to format the JSON manually - always use this tool. Your response will be rejected if it is not properly formatted through this tool. Only use this tool once you are ready to provide your final answer.";

/// One agent run; the item's JSON (`{output: ...}`).
async fn run_agent(ctx: &ExecCtx<'_>, item: usize) -> NodeResult<Value> {
    let user = prompt(ctx, item)?;
    let model = load_model(ctx, item).await?;
    let tools = load_tools(ctx, item)?;
    let memory = match load_memory(ctx, item)? {
        Some(m) => Some(m.connect(ctx).await?),
        None => None,
    };
    let parser = super::ai_chains::load_output_parser(ctx, item)?;
    // n8n's `prepareMessages`: from v1.9 a system message only when one is
    // set (before, "You are a helpful assistant" by default), followed by
    // the formatting instructions when an output parser is connected.
    let system = match ctx.param("options.systemMessage", item) {
        Ok(Value::String(s)) => Some(s).filter(|s| !s.is_empty()),
        _ if ctx.node.type_version < 1.9 => Some("You are a helpful assistant".to_string()),
        _ => None,
    };
    let system = match (system, &parser) {
        (Some(s), Some(_)) => Some(format!("{s}\n\n{FORMATTING_INSTRUCTIONS}")),
        (None, Some(_)) => Some(FORMATTING_INSTRUCTIONS.to_string()),
        (s, None) => s,
    };
    let max_iterations = ctx.param_f64("options.maxIterations", item, 10.0)?.max(1.0) as usize;
    let mut messages: Vec<Value> = system.into_iter().map(|s| json!({"role": "system", "content": s})).collect();
    if let Some(m) = &memory {
        messages.extend(m.load(ctx).await?);
    }
    messages.push(json!({"role": "user", "content": user}));
    let mut schemas: Vec<Value> = tools.iter().map(Tool::schema).collect();
    if let Some(p) = &parser {
        schemas.push(p.tool());
    }
    let mut answer = None;
    for _ in 0..max_iterations {
        let reply = model.chat(ctx, &messages, &schemas).await?;
        let calls = reply["tool_calls"].as_array().cloned().unwrap_or_default();
        // The format tool ends the run with its (parsed) arguments.
        if let (Some(p), Some(call)) = (&parser, calls.iter().find(|c| c.pointer("/function/name").and_then(Value::as_str) == Some(super::ai_chains::FORMAT_TOOL))) {
            let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
            let parsed = p.parse(ctx, args)?;
            return finish_structured(ctx, &memory, &user, parsed).await;
        }
        if calls.is_empty() {
            let text = reply["content"].as_str().unwrap_or("").to_string();
            if let Some(p) = &parser {
                let parsed = p.parse_final_text(ctx, &text)?;
                return finish_structured(ctx, &memory, &user, parsed).await;
            }
            answer = Some(json!(text));
            break;
        }
        messages.push(json!({"role": "assistant", "content": reply["content"], "tool_calls": calls}));
        for call in &calls {
            let name = call.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
            let args = call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or("{}");
            let result = match tools.iter().find(|t| t.name == name) {
                Some(tool) => tool.call(ctx, args).await?,
                None => format!("Error: there is no tool called \"{name}\""),
            };
            messages.push(json!({"role": "tool", "tool_call_id": call["id"], "content": result}));
        }
    }
    let answer = answer.and_then(|a| a.as_str().map(String::from)).unwrap_or_else(|| "Agent stopped due to max iterations.".to_string());
    if let Some(m) = &memory {
        m.save(ctx, &user, &answer).await?;
    }
    Ok(json!({"output": answer}))
}

/// A parsed structured answer: with memory, n8n saves it as JSON text and
/// returns `{output: <inner output>}`; otherwise the parsed object as is.
async fn finish_structured(ctx: &ExecCtx<'_>, memory: &Option<Memory<'_>>, user: &str, parsed: Value) -> NodeResult<Value> {
    if let Some(m) = memory {
        m.save(ctx, user, &parsed.to_string()).await?;
        return Ok(json!({"output": parsed.get("output").cloned().unwrap_or(parsed)}));
    }
    Ok(parsed)
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
    fn from_ai_calls_are_parsed_like_n8n() {
        let calls = extract_from_ai("={{ $fromAI('city', 'The city, e.g. \"Paris\"', 'string') }}/{{ $fromai(\"n\", ``, 'number', 5) }}").unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], FromAi { key: "city".into(), description: Some("The city, e.g. \"Paris\"".into()), kind: "string".into(), default: None });
        assert_eq!(calls[1], FromAi { key: "n".into(), description: Some("".into()), kind: "number".into(), default: Some(json!(5)) });
        let only_key = extract_from_ai("{{ $fromAI('q') }}").unwrap();
        assert_eq!(only_key[0].kind, "string");
        assert_eq!(only_key[0].description, None);
        assert!(extract_from_ai("{{ $fromAI('x', '', 'date') }}").is_err());
        assert!(extract_from_ai("{{ $fromAI('x' }}").is_err());
        assert_eq!(extract_from_ai("{{ $fromAI('b', 'flag', 'boolean', 'true') }}").unwrap()[0].default, Some(json!(true)));
    }

    #[test]
    fn model_names_stay_inside_their_path_segment() {
        assert_eq!(path_segment("gemini-2.5-flash"), "gemini-2.5-flash");
        assert_eq!(path_segment("anthropic.claude-v2:1"), "anthropic.claude-v2%3A1");
        assert_eq!(path_segment("a/b?x#y"), "a%2Fb%3Fx%23y");
        assert!(check_path_name("tunedModels/my-model", "model").is_ok());
        assert!(check_path_name("x/../../v1/files", "model").is_err());
        assert!(check_path_name("..", "deployment").is_err());
        assert!(check_path_name("", "deployment").is_err());
    }

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
