use crate::domain::Item;
use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolDefinition};
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct AgentNode;

/// One tool declared in `parameters.tools` -- see
/// `docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`
/// section 4.
struct ToolDecl {
    name: String,
    description: String,
    node_type: String,
    argument_schema: serde_json::Value,
    base_parameters: serde_json::Value,
    /// Library tool (spec B1): `base_parameters` are `{{ $args.x }}`
    /// templates, not a key-merge target.
    template: bool,
}

fn parse_tools(tools_param: &serde_json::Value) -> Result<Vec<ToolDecl>, NodeError> {
    let array = tools_param.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let mut decls = Vec::new();
    for entry in array {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: each tool requires a \"name\"".into()))?
            .to_string();
        let node_type = entry
            .get("node_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: tool \"{name}\" requires a \"node_type\"")))?
            .to_string();
        if node_type == "ai.agent" {
            return Err(NodeError::ExecutionFailed(format!(
                "ai.agent: tool \"{name}\" declares node_type \"ai.agent\" -- agent-to-agent tool calls are not supported"
            )));
        }
        decls.push(ToolDecl {
            name,
            description: entry.get("description").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            node_type,
            argument_schema: entry.get("argument_schema").cloned().unwrap_or(serde_json::json!({})),
            base_parameters: entry.get("base_parameters").cloned().unwrap_or(serde_json::json!({})),
            template: false,
        });
    }
    Ok(decls)
}

/// Argument keys the model may never supply, regardless of whether a tool
/// author declares them in `argument_schema` -- these configure a tool
/// call's infrastructure (which credential, which host, what code runs),
/// not its task-level input, so they must stay under the workflow author's
/// control alone. See the merge in `run_agent_loop` below, which relies on
/// this same deny-list holding for every tool dispatch.
const DENIED_ARGUMENT_KEYS: [&str; 6] = ["auth", "credential_id", "api_base_url", "base_url", "headers", "script"];

/// Shallow validation: no argument key is one of `DENIED_ARGUMENT_KEYS`;
/// every property the schema marks `required` is present; and each present
/// property's JSON type matches the schema's declared `type` (`"string"`,
/// `"number"`/`"integer"`, `"boolean"`, `"object"`, `"array"`). Not full
/// JSON Schema (no `pattern`, no `minimum`, no nested-schema checks) --
/// this exists only to catch an obviously malformed tool call before it
/// reaches a real node's own `execute()`, which does its own validation
/// regardless.
fn validate_arguments(schema: &serde_json::Value, arguments: &serde_json::Value) -> Result<(), String> {
    if let Some(args_obj) = arguments.as_object() {
        for key in args_obj.keys() {
            if DENIED_ARGUMENT_KEYS.contains(&key.as_str()) {
                return Err(format!("argument \"{key}\" may not be set by the model"));
            }
        }
    }
    let required = schema.get("required").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    for req in &required {
        let Some(key) = req.as_str() else { continue };
        if arguments.get(key).is_none() {
            return Err(format!("missing required argument \"{key}\""));
        }
    }
    let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) else { return Ok(()) };
    for (key, prop_schema) in properties {
        let Some(value) = arguments.get(key) else { continue };
        let Some(expected_type) = prop_schema.get("type").and_then(|v| v.as_str()) else { continue };
        let matches = match expected_type {
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64(),
            "boolean" => value.is_boolean(),
            "object" => value.is_object(),
            "array" => value.is_array(),
            _ => true, // unknown declared type -- don't block on it
        };
        if !matches {
            return Err(format!("argument \"{key}\" does not match declared type \"{expected_type}\""));
        }
    }
    Ok(())
}

fn item_to_tool_result_content(output: &NodeOutput) -> String {
    let primary = output.first().cloned().unwrap_or_default();
    let json_values: Vec<serde_json::Value> = primary.iter().map(|item| item.json.clone()).collect();
    serde_json::to_string(&json_values).unwrap_or_else(|_| "[]".to_string())
}

/// Drops the oldest whole "turn unit" (an `Assistant` tool-calls message
/// plus every `ToolResult` immediately following it) from the front of
/// `messages`, repeatedly, until `count_tokens` reports a value at or
/// under `budget` -- or until only the original seed message (index 0,
/// the very first `User` message) remains, which is never dropped:
/// removing it would leave the loop with no seed input at all, a
/// correctness failure worse than staying over budget (the provider's
/// own API still enforces the real limit either way -- see spec section
/// 7). Given `run_agent_loop`'s own message-building logic, index 1 (when
/// present) is always an `Assistant` tool-calls message -- it is the only
/// kind ever pushed there, immediately followed by that turn's
/// `ToolResult`(s), which is what this function relies on.
async fn trim_to_budget(
    provider: &dyn ProviderClient,
    system_prompt: &str,
    messages: &mut Vec<LlmMessage>,
    model: &str,
    api_key: &str,
    max_context_tokens: Option<usize>,
) -> Result<(), NodeError> {
    let Some(budget) = max_context_tokens else { return Ok(()) };
    loop {
        let count = provider.count_tokens(system_prompt, messages, model, api_key).await?;
        if count <= budget || messages.len() <= 1 {
            return Ok(());
        }
        let mut remove_count = 1; // the Assistant message itself, at index 1
        while messages.get(1 + remove_count).map(|m| matches!(m, LlmMessage::ToolResult { .. })).unwrap_or(false) {
            remove_count += 1;
        }
        messages.drain(1..1 + remove_count);
    }
}

pub(crate) async fn run_agent_loop(
    provider: &dyn ProviderClient,
    model: &str,
    api_key: &str,
    system_prompt: &str,
    user_message: String,
    tools_param: &serde_json::Value,
    max_iterations: u64,
    max_context_tokens: Option<usize>,
    ctx: &NodeExecutionContext,
) -> Result<NodeOutput, NodeError> {
    let mut tool_decls = parse_tools(tools_param)?;
    if let Some(ids) = ctx.parameters.get("tool_ids").and_then(|v| v.as_array()) {
        for id in ids.iter().filter_map(|v| v.as_str()).filter_map(|s| uuid::Uuid::parse_str(s).ok()) {
            let tool = ctx
                .tools
                .get(&id)
                .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: tool {id} was not resolved for this run")))?;
            if tool_decls.iter().any(|t| t.name == tool.name) {
                return Err(NodeError::ExecutionFailed(format!("ai.agent: duplicate tool name \"{}\"", tool.name)));
            }
            tool_decls.push(ToolDecl {
                name: tool.name.clone(),
                description: tool.description.clone(),
                node_type: tool.node_type.clone(),
                argument_schema: tool.argument_schema.clone(),
                base_parameters: tool.parameters.clone(),
                template: true,
            });
        }
    }
    let tool_definitions: Vec<ToolDefinition> = tool_decls
        .iter()
        .map(|t| ToolDefinition { name: t.name.clone(), description: t.description.clone(), parameters: t.argument_schema.clone() })
        .collect();

    let tool_executor = ctx
        .tool_executor
        .as_ref()
        .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: no tool_executor available in this execution context".into()))?;

    let mut messages = vec![LlmMessage::User { content: user_message }];
    let mut tool_calls_made: u64 = 0;

    for _ in 0..max_iterations {
        trim_to_budget(provider, system_prompt, &mut messages, model, api_key, max_context_tokens).await?;
        match provider.send_message(system_prompt, &messages, &tool_definitions, model, api_key).await? {
            ProviderResponse::Text(text) => {
                return Ok(vec![vec![Item {
                    json: serde_json::json!({"response": text, "tool_calls_made": tool_calls_made}),
                    binary: serde_json::json!({}),
                }]]);
            }
            ProviderResponse::ToolCalls { text, calls } => {
                messages.push(LlmMessage::Assistant { content: text, tool_calls: calls.clone() });
                for call in &calls {
                    tool_calls_made += 1;
                    let Some(decl) = tool_decls.iter().find(|t| t.name == call.name) else {
                        messages.push(LlmMessage::ToolResult {
                            tool_call_id: call.id.clone(),
                            content: format!("unknown tool: \"{}\"", call.name),
                            is_error: true,
                        });
                        continue;
                    };
                    if let Err(reason) = validate_arguments(&decl.argument_schema, &call.arguments) {
                        messages.push(LlmMessage::ToolResult { tool_call_id: call.id.clone(), content: reason, is_error: true });
                        continue;
                    }
                    let verbatim = decl.template && !tool_executor.resolves_parameters(&decl.node_type);
                    let parameters = if verbatim {
                        // A node that runs its parameters as code (core.code)
                        // gets the arguments as data via `$args`, never
                        // spliced into its source.
                        decl.base_parameters.clone()
                    } else if decl.template {
                        // Library tool: arguments reach the node only where
                        // the tool author placed {{ $args.x }}.
                        let no_items: Vec<serde_json::Value> = Vec::new();
                        let no_nodes = std::collections::HashMap::new();
                        let eval_ctx = crate::expr::EvalContext {
                            json: serde_json::json!({}),
                            items: &no_items,
                            node_json: &no_nodes,
                            workflow_name: "",
                            args: Some(&call.arguments),
                        };
                        match crate::expr::resolve_parameters(&decl.base_parameters, &eval_ctx) {
                            Ok(p) => p,
                            Err(e) => {
                                messages.push(LlmMessage::ToolResult {
                                    tool_call_id: call.id.clone(),
                                    content: format!("tool parameter template failed: {e}"),
                                    is_error: true,
                                });
                                continue;
                            }
                        }
                    } else {
                        let mut merged_parameters = decl.base_parameters.clone();
                        if let (Some(merged_obj), Some(args_obj)) = (merged_parameters.as_object_mut(), call.arguments.as_object()) {
                            let declared = decl.argument_schema.get("properties").and_then(|v| v.as_object());
                            for (k, v) in args_obj {
                                // Only a key the tool author explicitly declared
                                // may override base_parameters -- an undeclared
                                // key is silently dropped rather than merged, so
                                // the model can never inject configuration the
                                // author never opted the tool into.
                                if declared.is_some_and(|d| d.contains_key(k)) {
                                    merged_obj.insert(k.clone(), v.clone());
                                }
                            }
                        }
                        merged_parameters
                    };
                    let tool_args = decl.template.then(|| call.arguments.clone());
                    match tool_executor.call_tool(&decl.node_type, parameters, tool_args).await {
                        Ok(output) => {
                            messages.push(LlmMessage::ToolResult {
                                tool_call_id: call.id.clone(),
                                content: item_to_tool_result_content(&output),
                                is_error: false,
                            });
                        }
                        Err(e) => {
                            messages.push(LlmMessage::ToolResult { tool_call_id: call.id.clone(), content: e.to_string(), is_error: true });
                        }
                    }
                }
            }
        }
    }

    Err(NodeError::ExecutionFailed("ai.agent: exceeded max_iterations without a final response".into()))
}

#[async_trait]
impl Node for AgentNode {
    fn type_name(&self) -> &'static str {
        "ai.agent"
    }
    fn display_name(&self) -> &'static str {
        "AI Agent"
    }
    fn description(&self) -> &'static str {
        "Runs an LLM-backed agent loop that can call other nodes as tools."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Ai
    }
    fn icon(&self) -> &'static str {
        "🤖"
    }
    fn credential_types(&self) -> &'static [&'static str] {
        &["anthropicApi", "openaiApi"]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let str_param = |name: &str| ctx.parameters.get(name).and_then(|v| v.as_str());
        // No explicit provider: the credential's type already says which
        // API it is for, so a workflow saved without `provider` still runs.
        let inferred_provider = ctx
            .parameters
            .get("auth")
            .and_then(|a| a.get("credential_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .and_then(|id| ctx.credential_types.get(&id))
            .and_then(|t| match t.as_str() {
                "openaiApi" => Some("openai"),
                "anthropicApi" => Some("anthropic"),
                _ => None,
            });
        let provider = str_param("provider").or(inferred_provider);
        // Report every missing required parameter at once: they're edited as
        // raw JSON, so one-at-a-time errors mean one failed run per field.
        let mut missing: Vec<&str> = Vec::new();
        if provider.is_none() {
            missing.push("provider (\"anthropic\" or \"openai\")");
        }
        for key in ["model", "user_message"] {
            if str_param(key).is_none() {
                missing.push(key);
            }
        }
        if !missing.is_empty() {
            return Err(NodeError::ExecutionFailed(format!(
                "ai.agent is missing required parameters: {}",
                missing.join(", ")
            )));
        }
        let provider_name = provider.unwrap_or_default();
        let model = str_param("model").unwrap_or_default();
        let system_prompt = str_param("system_prompt").unwrap_or("");
        let user_message = str_param("user_message").unwrap_or_default().to_string();
        let max_iterations = ctx.parameters.get("max_iterations").and_then(|v| v.as_u64()).unwrap_or(10);
        let max_context_tokens = ctx.parameters.get("max_context_tokens").and_then(|v| v.as_u64()).map(|n| n as usize);
        let empty_tools = serde_json::json!([]);
        let tools_param = ctx.parameters.get("tools").unwrap_or(&empty_tools);

        let credential_id_str = ctx
            .parameters
            .get("auth")
            .and_then(|a| a.get("credential_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent requires auth.credential_id".into()))?;
        let credential_id = uuid::Uuid::parse_str(credential_id_str)
            .map_err(|e| NodeError::ExecutionFailed(format!("ai.agent: invalid credential_id: {e}")))?;
        let credential_data = ctx
            .credentials
            .get(&credential_id)
            .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: credential {credential_id} was not resolved for this run")))?;
        let api_key = credential_data
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: credential is missing \"api_key\"".into()))?;

        match provider_name {
            "anthropic" => {
                let base_url = ctx.parameters.get("api_base_url").and_then(|v| v.as_str());
                let client = match base_url {
                    Some(url) => crate::llm::anthropic::AnthropicClient::with_base_url_for_node(url.to_string())?,
                    None => crate::llm::anthropic::AnthropicClient::new()?,
                };
                run_agent_loop(&client, model, api_key, system_prompt, user_message, tools_param, max_iterations, max_context_tokens, ctx).await
            }
            "openai" => {
                let base_url = credential_data.get("base_url").and_then(|v| v.as_str()).map(|s| s.to_string());
                let client = crate::llm::openai::OpenAiClient::new(base_url)?;
                run_agent_loop(&client, model, api_key, system_prompt, user_message, tools_param, max_iterations, max_context_tokens, ctx).await
            }
            other => Err(NodeError::ExecutionFailed(format!("ai.agent: unknown provider \"{other}\" (expected \"anthropic\" or \"openai\")"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
    use crate::node::{NodeError, NodeExecutionContext, NodeOutput, ToolExecutor};
    use std::sync::Mutex;

    /// A scripted provider: returns each entry in `responses` in order,
    /// one per `send_message` call. Panics if called more times than
    /// scripted -- a test asserting the loop's turn count wrong will fail
    /// loudly here rather than silently returning a stale response.
    struct ScriptedProvider {
        responses: Mutex<Vec<ProviderResponse>>,
    }

    #[async_trait::async_trait]
    impl ProviderClient for ScriptedProvider {
        async fn send_message(
            &self,
            _system_prompt: &str,
            _messages: &[LlmMessage],
            _tools: &[ToolDefinition],
            _model: &str,
            _api_key: &str,
        ) -> Result<ProviderResponse, NodeError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                panic!("ScriptedProvider called more times than scripted");
            }
            Ok(responses.remove(0))
        }

        async fn count_tokens(&self, _: &str, _: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
            Ok(0) // never trims in these tests -- token-budget behavior is a separate test
        }
    }

    /// Always returns the same tool result (or error), recording every
    /// call it received.
    struct SpyToolExecutor {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
        result: Result<NodeOutput, String>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for SpyToolExecutor {
        async fn call_tool(&self, node_type: &str, parameters: serde_json::Value, _tool_args: Option<serde_json::Value>) -> Result<NodeOutput, NodeError> {
            self.calls.lock().unwrap().push((node_type.to_string(), parameters));
            match &self.result {
                Ok(output) => Ok(output.clone()),
                Err(msg) => Err(NodeError::ExecutionFailed(msg.clone())),
            }
        }
    }

    fn ctx_with_tool_executor(executor: std::sync::Arc<dyn ToolExecutor>) -> NodeExecutionContext {
        NodeExecutionContext { tool_executor: Some(executor), ..Default::default() }
    }

    #[tokio::test]
    async fn a_direct_text_response_needs_no_tool_calls() {
        let provider = ScriptedProvider { responses: Mutex::new(vec![ProviderResponse::Text("Hi there!".into())]) };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![]]),
        });
        let ctx = ctx_with_tool_executor(spy.clone());

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "hi".into(), &serde_json::json!([]), 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Hi there!");
        assert_eq!(output[0][0].json["tool_calls_made"], 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_successful_tool_call_feeds_its_result_back_and_the_loop_continues() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({"x": 1}) }] },
                ProviderResponse::Text("Done.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![Item { json: serde_json::json!({"ok": true}), binary: serde_json::json!({}) }]]),
        });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]},
            "base_parameters": {"method": "GET", "x": 0}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Done.");
        assert_eq!(output[0][0].json["tool_calls_made"], 1);
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls[0].0, "core.httpRequest");
        // base_parameters merged with the model's own arguments (arguments win).
        assert_eq!(calls[0].1, serde_json::json!({"method": "GET", "x": 1}));
    }

    #[tokio::test]
    async fn an_undeclared_argument_key_is_not_merged_into_tool_parameters() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls {
                    text: None,
                    calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({"x": 1, "url": "https://attacker.example"}) }],
                },
                ProviderResponse::Text("Done.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![Item { json: serde_json::json!({"ok": true}), binary: serde_json::json!({}) }]]),
        });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            // "url" is deliberately NOT declared in argument_schema.
            "argument_schema": {"type": "object", "properties": {"x": {"type": "integer"}}, "required": ["x"]},
            "base_parameters": {"method": "GET", "x": 0, "url": "https://pinned.example"}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Done.");
        let calls = spy.calls.lock().unwrap();
        // The model's "url" argument was never declared, so it is dropped --
        // the author's pinned base_parameters value survives untouched.
        assert_eq!(calls[0].1, serde_json::json!({"method": "GET", "x": 1, "url": "https://pinned.example"}));
    }

    #[tokio::test]
    async fn a_denied_argument_key_is_rejected_even_when_declared_in_the_schema() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls {
                    text: None,
                    calls: vec![ToolCall {
                        id: "t1".into(),
                        name: "fetch".into(),
                        arguments: serde_json::json!({"auth": {"credential_id": "11111111-1111-1111-1111-111111111111"}}),
                    }],
                },
                ProviderResponse::Text("Okay.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            // The author even declared "auth" in the schema -- still denied.
            "argument_schema": {"type": "object", "properties": {"auth": {"type": "object"}}},
            "base_parameters": {"auth": {"credential_id": "22222222-2222-2222-2222-222222222222"}}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Okay.");
        // Rejected before dispatch -- the tool executor never sees the model's auth override.
        assert!(spy.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_tool_call_becomes_a_tool_result_and_the_run_continues() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }] },
                ProviderResponse::Text("I couldn't fetch that.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Err("upstream 500".into()),
        });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        // The run succeeds overall -- the tool failure was surfaced to
        // the model, not propagated as a hard NodeError.
        assert_eq!(output[0][0].json["response"], "I couldn't fetch that.");
    }

    #[tokio::test]
    async fn exceeding_max_iterations_without_a_final_response_is_a_hard_error() {
        let always_tool_calls = ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }] };
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![always_tool_calls; 3]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![]]),
        });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch a thing", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let result = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 3, None, &ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn a_tool_declared_with_node_type_ai_agent_is_rejected_before_any_provider_call() {
        let provider = ScriptedProvider { responses: Mutex::new(vec![]) }; // never called
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "recurse", "description": "nope", "node_type": "ai.agent",
            "argument_schema": {"type": "object", "properties": {}},
            "base_parameters": {}
        }]);

        let result = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => assert!(msg.contains("ai.agent")),
            other => panic!("expected ExecutionFailed mentioning ai.agent, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_name_becomes_a_tool_result_not_a_hard_failure() {
        let provider = ScriptedProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: "does_not_exist".into(), arguments: serde_json::json!({}) }] },
                ProviderResponse::Text("Never mind.".into()),
            ]),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_tool_executor(spy.clone());
        let tools = serde_json::json!([]); // "does_not_exist" isn't declared at all

        let output = run_agent_loop(&provider, "claude-opus-5", "key", "You are helpful.", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        assert_eq!(output[0][0].json["response"], "Never mind.");
        assert!(spy.calls.lock().unwrap().is_empty()); // never dispatched -- caught before ToolExecutor
    }

    #[tokio::test]
    async fn memory_is_trimmed_to_the_token_budget_before_each_turn() {
        // A provider whose count_tokens reflects the CURRENT message
        // count (10 "tokens" per message) so trimming is directly
        // observable: after each tool-calling round adds 2 messages
        // (an Assistant-with-tool_calls plus its ToolResult), the next
        // pre-turn check goes over a budget of 25, forcing that round's
        // 2 messages to be dropped before the turn proceeds.
        struct CountingProvider {
            responses: Mutex<Vec<ProviderResponse>>,
            counts_seen: Mutex<Vec<usize>>,
        }
        #[async_trait::async_trait]
        impl ProviderClient for CountingProvider {
            async fn send_message(&self, _: &str, _: &[LlmMessage], _: &[ToolDefinition], _: &str, _: &str) -> Result<ProviderResponse, NodeError> {
                Ok(self.responses.lock().unwrap().remove(0))
            }
            async fn count_tokens(&self, _: &str, messages: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
                let count = messages.len() * 10;
                self.counts_seen.lock().unwrap().push(count);
                Ok(count)
            }
        }

        let provider = CountingProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }] },
                ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t2".into(), name: "fetch".into(), arguments: serde_json::json!({}) }] },
                ProviderResponse::Text("Done.".into()),
            ]),
            counts_seen: Mutex::new(Vec::new()),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]]),
        });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}}, "base_parameters": {}
        }]);

        run_agent_loop(&provider, "claude-opus-5", "key", "sys", "go".into(), &tools, 10, Some(25), &ctx)
            .await
            .unwrap();

        // messages starts at [user] (10 tokens, under 25 -- proceeds).
        // After round 1: [user, assistant, tool_result] = 30 tokens --
        // over budget on the next pre-turn check, so a trim must occur
        // before round 2's send_message. The trimmed count (back to just
        // [user] = 10) must appear as a later, lower reading -- proving
        // the trim loop actually re-checks after dropping, not just
        // once-and-gives-up.
        let counts = provider.counts_seen.lock().unwrap();
        assert!(counts.contains(&30), "expected an over-budget reading triggering a trim, got {counts:?}");
        assert_eq!(*counts.last().unwrap(), 10, "history must be back under budget by the final pre-turn check; got {counts:?}");
        assert!(counts.len() >= 4, "expected repeated count_tokens calls as trimming re-checks after each drop, got {counts:?}");
    }

    #[tokio::test]
    async fn assistant_text_accompanying_a_tool_call_is_preserved_in_history() {
        struct CapturingProvider {
            responses: Mutex<Vec<ProviderResponse>>,
            seen_messages: Mutex<Vec<Vec<LlmMessage>>>,
        }
        #[async_trait::async_trait]
        impl ProviderClient for CapturingProvider {
            async fn send_message(&self, _: &str, messages: &[LlmMessage], _: &[ToolDefinition], _: &str, _: &str) -> Result<ProviderResponse, NodeError> {
                self.seen_messages.lock().unwrap().push(messages.to_vec());
                Ok(self.responses.lock().unwrap().remove(0))
            }
            async fn count_tokens(&self, _: &str, _: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
                Ok(0)
            }
        }

        let provider = CapturingProvider {
            responses: Mutex::new(vec![
                ProviderResponse::ToolCalls {
                    text: Some("Let me check that for you.".into()),
                    calls: vec![ToolCall { id: "t1".into(), name: "fetch".into(), arguments: serde_json::json!({}) }],
                },
                ProviderResponse::Text("Done.".into()),
            ]),
            seen_messages: Mutex::new(Vec::new()),
        };
        let spy = std::sync::Arc::new(SpyToolExecutor {
            calls: Mutex::new(Vec::new()),
            result: Ok(vec![vec![]]),
        });
        let ctx = ctx_with_tool_executor(spy);
        let tools = serde_json::json!([{
            "name": "fetch", "description": "fetch", "node_type": "core.httpRequest",
            "argument_schema": {"type": "object", "properties": {}}, "base_parameters": {}
        }]);

        run_agent_loop(&provider, "claude-opus-5", "key", "sys", "go".into(), &tools, 10, None, &ctx)
            .await
            .unwrap();

        // The second send_message call's message history must contain the
        // assistant's text from the first turn, not None.
        let seen = provider.seen_messages.lock().unwrap();
        let second_call_messages = &seen[1];
        let has_preserved_text = second_call_messages.iter().any(|m| {
            matches!(m, LlmMessage::Assistant { content: Some(text), .. } if text == "Let me check that for you.")
        });
        assert!(has_preserved_text, "assistant's accompanying text must survive into the next turn's history, got: {second_call_messages:?}");
    }

    #[tokio::test]
    async fn names_every_missing_required_parameter_at_once() {
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"auth": {"credential_id": uuid::Uuid::new_v4().to_string()}}),
            ..Default::default()
        };
        let err = AgentNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(
            err.contains("ai.agent is missing required parameters: provider (\"anthropic\" or \"openai\"), model, user_message"),
            "{err}"
        );
    }

    fn library_tool(name: &str, parameters: serde_json::Value) -> crate::domain::Tool {
        crate::domain::Tool {
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            description: "search the web".into(),
            node_type: "core.httpRequest".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}),
            parameters,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn ctx_with_library(executor: std::sync::Arc<dyn ToolExecutor>, tool: &crate::domain::Tool) -> NodeExecutionContext {
        NodeExecutionContext {
            parameters: serde_json::json!({"tool_ids": [tool.id.to_string()]}),
            tools: std::collections::HashMap::from([(tool.id, tool.clone())]),
            tool_executor: Some(executor),
            ..Default::default()
        }
    }

    fn one_call_then_done(name: &str, arguments: serde_json::Value) -> Vec<ProviderResponse> {
        vec![
            ProviderResponse::ToolCalls { text: None, calls: vec![ToolCall { id: "t1".into(), name: name.into(), arguments }] },
            ProviderResponse::Text("Done.".into()),
        ]
    }

    #[tokio::test]
    async fn library_tool_resolves_args_templates_without_key_merge() {
        let tool = library_tool("search", serde_json::json!({"method": "GET", "url": "https://s.example/?q={{ $args.q }}", "q": "fixed"}));
        let provider = ScriptedProvider { responses: Mutex::new(one_call_then_done("search", serde_json::json!({"q": "rust"}))) };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_library(spy.clone(), &tool);

        let output = run_agent_loop(&provider, "m", "key", "sys", "go".into(), &serde_json::json!([]), 10, None, &ctx).await.unwrap();

        assert_eq!(output[0][0].json["response"], "Done.");
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls[0].0, "core.httpRequest");
        assert_eq!(calls[0].1, serde_json::json!({"method": "GET", "url": "https://s.example/?q=rust", "q": "fixed"}));
    }

    #[tokio::test]
    async fn library_tool_template_error_is_reported_to_the_model() {
        struct Capturing {
            responses: Mutex<Vec<ProviderResponse>>,
            seen: Mutex<Vec<Vec<LlmMessage>>>,
        }
        #[async_trait::async_trait]
        impl ProviderClient for Capturing {
            async fn send_message(&self, _: &str, messages: &[LlmMessage], _: &[ToolDefinition], _: &str, _: &str) -> Result<ProviderResponse, NodeError> {
                self.seen.lock().unwrap().push(messages.to_vec());
                Ok(self.responses.lock().unwrap().remove(0))
            }
            async fn count_tokens(&self, _: &str, _: &[LlmMessage], _: &str, _: &str) -> Result<usize, NodeError> {
                Ok(0)
            }
        }
        let tool = library_tool("search", serde_json::json!({"url": "{{ $args.q.missing.deep }}"}));
        let provider = Capturing { responses: Mutex::new(one_call_then_done("search", serde_json::json!({"q": "rust"}))), seen: Mutex::new(Vec::new()) };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_library(spy.clone(), &tool);

        run_agent_loop(&provider, "m", "key", "sys", "go".into(), &serde_json::json!([]), 10, None, &ctx).await.unwrap();

        assert!(spy.calls.lock().unwrap().is_empty(), "the node must not run when its template fails");
        let seen = provider.seen.lock().unwrap();
        assert!(
            seen[1].iter().any(|m| matches!(m, LlmMessage::ToolResult { is_error: true, content, .. } if content.contains("template"))),
            "{:?}",
            seen[1]
        );
    }

    #[tokio::test]
    async fn inline_and_library_tools_with_the_same_name_are_rejected() {
        let tool = library_tool("search", serde_json::json!({}));
        let provider = ScriptedProvider { responses: Mutex::new(vec![ProviderResponse::Text("unused".into())]) };
        let spy = std::sync::Arc::new(SpyToolExecutor { calls: Mutex::new(Vec::new()), result: Ok(vec![vec![]]) });
        let ctx = ctx_with_library(spy, &tool);
        let inline = serde_json::json!([{"name": "search", "node_type": "core.code"}]);

        let err = run_agent_loop(&provider, "m", "key", "sys", "go".into(), &inline, 10, None, &ctx).await.unwrap_err().to_string();

        assert!(err.contains("duplicate tool name \"search\""), "{err}");
    }

    /// Records calls including the args; reports core.code as a node that
    /// runs its parameters verbatim, like the real engine executor.
    struct VerbatimCodeSpy {
        calls: Mutex<Vec<(String, serde_json::Value, Option<serde_json::Value>)>>,
    }

    #[async_trait::async_trait]
    impl ToolExecutor for VerbatimCodeSpy {
        fn resolves_parameters(&self, node_type: &str) -> bool {
            node_type != "core.code"
        }
        async fn call_tool(&self, node_type: &str, parameters: serde_json::Value, tool_args: Option<serde_json::Value>) -> Result<NodeOutput, NodeError> {
            self.calls.lock().unwrap().push((node_type.to_string(), parameters, tool_args));
            Ok(vec![vec![]])
        }
    }

    #[tokio::test]
    async fn a_code_tool_script_is_never_templated_with_model_text() {
        let mut tool = library_tool("code_tool", serde_json::json!({"script": "return [{ json: { r: '{{ $args.q }}' } }]"}));
        tool.node_type = "core.code".into();
        let injection = "x' }}, {json:{pwned: 1+1}}]; var z = [{a:{b:'";
        let provider = ScriptedProvider { responses: Mutex::new(one_call_then_done("code_tool", serde_json::json!({"q": injection}))) };
        let spy = std::sync::Arc::new(VerbatimCodeSpy { calls: Mutex::new(Vec::new()) });
        let ctx = ctx_with_library(spy.clone(), &tool);

        run_agent_loop(&provider, "m", "key", "sys", "go".into(), &serde_json::json!([]), 10, None, &ctx).await.unwrap();

        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls[0].1, tool.parameters, "the script must reach the node exactly as the author wrote it");
        assert_eq!(calls[0].2, Some(serde_json::json!({"q": injection})), "arguments travel as data, for the node's $args global");
    }

    #[tokio::test]
    async fn provider_is_inferred_from_the_credential_type_at_run_time() {
        let cred = uuid::Uuid::new_v4();
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"auth": {"credential_id": cred.to_string()}}),
            credential_types: std::collections::HashMap::from([(cred, "openaiApi".to_string())]),
            ..Default::default()
        };
        let err = AgentNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("missing required parameters: model, user_message"), "{err}");
        assert!(!err.contains("provider"), "provider should come from the openaiApi credential: {err}");
    }
}
