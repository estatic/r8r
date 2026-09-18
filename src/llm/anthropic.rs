use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
use crate::node::NodeError;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TOKENS: u32 = 4096;

pub struct AnthropicClient {
    client: reqwest::Client,
    base_url: String,
}

impl AnthropicClient {
    pub fn new() -> Result<Self, NodeError> {
        Self::with_base_url_result(DEFAULT_BASE_URL.to_string())
    }

    /// Test-only constructor pointing at a wiremock server; panics on a
    /// client-build failure since a test that can't build a client can't
    /// proceed anyway -- matches this codebase's other test-only
    /// constructors' tolerance for a hard failure in test setup.
    #[cfg(test)]
    fn with_base_url(base_url: String) -> Self {
        Self::with_base_url_result(base_url).expect("failed to build reqwest client")
    }

    /// Non-test override for a custom base URL (e.g. a node-level
    /// `api_base_url` parameter pointing at a mock or a compatible
    /// self-hosted endpoint) -- unlike `with_base_url` (test-only, panics
    /// on a build failure), this returns a proper `Result` since it's
    /// reachable from real node execution.
    pub fn with_base_url_for_node(base_url: String) -> Result<Self, NodeError> {
        Self::with_base_url_result(base_url)
    }

    fn with_base_url_result(base_url: String) -> Result<Self, NodeError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| NodeError::ExecutionFailed(format!("failed to build Anthropic HTTP client: {e}")))?;
        Ok(Self { client, base_url })
    }

    fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
        match msg {
            LlmMessage::User { content } => serde_json::json!({"role": "user", "content": content}),
            LlmMessage::Assistant { content, tool_calls } => {
                let mut blocks = Vec::new();
                if let Some(text) = content {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    blocks.push(serde_json::json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments
                    }));
                }
                serde_json::json!({"role": "assistant", "content": blocks})
            }
            LlmMessage::ToolResult { tool_call_id, content, is_error } => serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                    "is_error": is_error
                }]
            }),
        }
    }

    fn messages_to_json(messages: &[LlmMessage]) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < messages.len() {
            if matches!(messages[i], LlmMessage::ToolResult { .. }) {
                let mut blocks = Vec::new();
                while let Some(LlmMessage::ToolResult { tool_call_id, content, is_error }) = messages.get(i) {
                    blocks.push(serde_json::json!({
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "content": content,
                        "is_error": is_error
                    }));
                    i += 1;
                }
                out.push(serde_json::json!({"role": "user", "content": blocks}));
            } else {
                out.push(Self::message_to_json(&messages[i]));
                i += 1;
            }
        }
        out
    }

    fn build_request_body(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        include_max_tokens: bool,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": model,
            "system": system_prompt,
            "messages": Self::messages_to_json(messages),
        });
        if include_max_tokens {
            body["max_tokens"] = serde_json::json!(MAX_TOKENS);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools
                .iter()
                .map(|t| serde_json::json!({"name": t.name, "description": t.description, "input_schema": t.parameters}))
                .collect::<Vec<_>>());
        }
        body
    }

    async fn post(&self, path: &str, body: serde_json::Value, api_key: &str) -> Result<serde_json::Value, NodeError> {
        let response = self
            .client
            .post(format!("{}{}", self.base_url, path))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(|_| NodeError::ExecutionFailed("ai.agent: request to Anthropic API failed".into()))?;

        let status = response.status();
        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("ai.agent: Anthropic API returned HTTP {status} with a non-JSON body")))?;

        if !status.is_success() {
            let message = response_json
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no message provided");
            return Err(NodeError::ExecutionFailed(format!("ai.agent: Anthropic API error: {message}")));
        }
        Ok(response_json)
    }
}

#[async_trait::async_trait]
impl ProviderClient for AnthropicClient {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, NodeError> {
        let body = self.build_request_body(system_prompt, messages, tools, model, true);
        let response_json = self.post("/v1/messages", body, api_key).await?;

        let content = response_json
            .get("content")
            .and_then(|c| c.as_array())
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: Anthropic response missing \"content\"".into()))?;

        let mut tool_calls = Vec::new();
        let mut text = String::new();
        for block in content {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    tool_calls.push(ToolCall {
                        id: block.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        name: block.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        arguments: block.get("input").cloned().unwrap_or(serde_json::json!({})),
                    });
                }
                _ => {}
            }
        }

        if !tool_calls.is_empty() {
            let text = if text.is_empty() { None } else { Some(text) };
            Ok(ProviderResponse::ToolCalls { text, calls: tool_calls })
        } else {
            Ok(ProviderResponse::Text(text))
        }
    }

    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        api_key: &str,
    ) -> Result<usize, NodeError> {
        let body = self.build_request_body(system_prompt, messages, &[], model, false);
        let response_json = self.post("/v1/messages/count_tokens", body, api_key).await?;
        response_json
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: Anthropic count_tokens response missing \"input_tokens\"".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn send_message_returns_text_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .and(body_partial_json(serde_json::json!({
                "messages": [{"role": "user", "content": "hi"}]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_1",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "Hello there"}],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let response = client
            .send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "Hello there"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_message_returns_tool_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_2",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "tool_use", "id": "toolu_1", "name": "fetch_weather", "input": {"city": "Warsaw"}}],
                "stop_reason": "tool_use"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "weather in Warsaw?".into() }];
        let tools = vec![ToolDefinition {
            name: "fetch_weather".into(),
            description: "Get weather".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        let response = client
            .send_message("You are helpful.", &messages, &tools, "claude-opus-5", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::ToolCalls { text: _, calls } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "fetch_weather");
                assert_eq!(calls[0].arguments, serde_json::json!({"city": "Warsaw"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn count_tokens_calls_the_hosted_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/count_tokens"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"input_tokens": 42})))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let count = client
            .count_tokens("You are helpful.", &messages, "claude-opus-5", "test-key")
            .await
            .unwrap();
        assert_eq!(count, 42);
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "type": "error",
                "error": {"type": "authentication_error", "message": "invalid x-api-key"}
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let result = client.send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key").await;
        assert!(matches!(result, Err(crate::node::NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn tool_use_and_tool_result_messages_round_trip_through_a_second_turn() {
        // Proves the assistant tool_calls / user tool_result translation
        // is wired correctly, not just the first-turn request.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    {"role": "user", "content": "weather in Warsaw?"},
                    {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "fetch_weather", "input": {"city": "Warsaw"}}]},
                    {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "{\"temp_c\": 22}", "is_error": false}]}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_3",
                "type": "message",
                "role": "assistant",
                "content": [{"type": "text", "text": "It's sunny."}],
                "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![
            LlmMessage::User { content: "weather in Warsaw?".into() },
            LlmMessage::Assistant {
                content: None,
                tool_calls: vec![ToolCall { id: "toolu_1".into(), name: "fetch_weather".into(), arguments: serde_json::json!({"city": "Warsaw"}) }],
            },
            LlmMessage::ToolResult { tool_call_id: "toolu_1".into(), content: "{\"temp_c\": 22}".into(), is_error: false },
        ];
        let response = client.send_message("You are helpful.", &messages, &[], "claude-opus-5", "test-key").await.unwrap();
        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "It's sunny."),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn parallel_tool_results_are_grouped_into_a_single_user_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    {"role": "user", "content": "check two things"},
                    {"role": "assistant", "content": [
                        {"type": "tool_use", "id": "t1", "name": "a", "input": {}},
                        {"type": "tool_use", "id": "t2", "name": "b", "input": {}}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "t1", "content": "result-a", "is_error": false},
                        {"type": "tool_result", "tool_use_id": "t2", "content": "result-b", "is_error": false}
                    ]}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "msg_x", "type": "message", "role": "assistant",
                "content": [{"type": "text", "text": "done"}], "stop_reason": "end_turn"
            })))
            .mount(&server)
            .await;

        let client = AnthropicClient::with_base_url(server.uri());
        let messages = vec![
            LlmMessage::User { content: "check two things".into() },
            LlmMessage::Assistant {
                content: None,
                tool_calls: vec![
                    ToolCall { id: "t1".into(), name: "a".into(), arguments: serde_json::json!({}) },
                    ToolCall { id: "t2".into(), name: "b".into(), arguments: serde_json::json!({}) },
                ],
            },
            LlmMessage::ToolResult { tool_call_id: "t1".into(), content: "result-a".into(), is_error: false },
            LlmMessage::ToolResult { tool_call_id: "t2".into(), content: "result-b".into(), is_error: false },
        ];
        let response = client.send_message("sys", &messages, &[], "claude-opus-5", "test-key").await.unwrap();
        assert!(matches!(response, ProviderResponse::Text(t) if t == "done"));
    }
}
