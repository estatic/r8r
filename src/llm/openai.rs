use crate::llm::{LlmMessage, ProviderClient, ProviderResponse, ToolCall, ToolDefinition};
use crate::node::NodeError;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct OpenAiClient {
    client: reqwest::Client,
    base_url: String,
}

impl OpenAiClient {
    pub fn new(base_url: Option<String>) -> Result<Self, NodeError> {
        Self::with_base_url_result(base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()))
    }

    #[cfg(test)]
    fn with_base_url(base_url: String) -> Self {
        Self::with_base_url_result(base_url).expect("failed to build reqwest client")
    }

    fn with_base_url_result(base_url: String) -> Result<Self, NodeError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| NodeError::ExecutionFailed(format!("failed to build OpenAI HTTP client: {e}")))?;
        Ok(Self { client, base_url })
    }

    fn message_to_json(msg: &LlmMessage) -> serde_json::Value {
        match msg {
            LlmMessage::User { content } => serde_json::json!({"role": "user", "content": content}),
            LlmMessage::Assistant { content, tool_calls } => {
                let mut obj = serde_json::json!({"role": "assistant", "content": content});
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = serde_json::json!(tool_calls
                        .iter()
                        .map(|call| serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": serde_json::to_string(&call.arguments).unwrap_or_default()
                            }
                        }))
                        .collect::<Vec<_>>());
                }
                obj
            }
            LlmMessage::ToolResult { tool_call_id, content, is_error: _ } => serde_json::json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content
            }),
        }
    }

    fn build_messages_json(&self, system_prompt: &str, messages: &[LlmMessage]) -> Vec<serde_json::Value> {
        let mut out = vec![serde_json::json!({"role": "system", "content": system_prompt})];
        out.extend(messages.iter().map(Self::message_to_json));
        out
    }
}

#[async_trait::async_trait]
impl ProviderClient for OpenAiClient {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, NodeError> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": self.build_messages_json(system_prompt, messages),
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools
                .iter()
                .map(|t| serde_json::json!({
                    "type": "function",
                    "function": {"name": t.name, "description": t.description, "parameters": t.parameters}
                }))
                .collect::<Vec<_>>());
        }

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|_| NodeError::ExecutionFailed("ai.agent: request to OpenAI-compatible API failed".into()))?;

        let status = response.status();
        let response_json: serde_json::Value = response
            .json()
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("ai.agent: OpenAI-compatible API returned HTTP {status} with a non-JSON body")))?;

        if !status.is_success() {
            let message = response_json
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no message provided");
            return Err(NodeError::ExecutionFailed(format!("ai.agent: OpenAI-compatible API error: {message}")));
        }

        let message = response_json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .ok_or_else(|| NodeError::ExecutionFailed("ai.agent: OpenAI-compatible response missing choices[0].message".into()))?;

        if let Some(tool_calls_json) = message.get("tool_calls").and_then(|v| v.as_array()) {
            if !tool_calls_json.is_empty() {
                let tool_calls = tool_calls_json
                    .iter()
                    .map(|tc| {
                        let arguments_str = tc
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        ToolCall {
                            id: tc.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                            name: tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            arguments: serde_json::from_str(arguments_str).unwrap_or(serde_json::json!({})),
                        }
                    })
                    .collect();
                let text = message.get("content").and_then(|v| v.as_str()).map(|s| s.to_string());
                return Ok(ProviderResponse::ToolCalls { text, calls: tool_calls });
            }
        }

        let text = message.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok(ProviderResponse::Text(text))
    }

    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        _api_key: &str,
    ) -> Result<usize, NodeError> {
        // No hosted counting endpoint on this API shape -- compute
        // locally via tiktoken-rs, entirely offline, no network call.
        let bpe = tiktoken_rs::get_bpe_from_model(model).unwrap_or_else(|_| {
            tiktoken_rs::cl100k_base().expect("cl100k_base tokenizer must always be constructible")
        });
        let mut total = bpe.encode_with_special_tokens(system_prompt).len();
        for msg in messages {
            let text = match msg {
                LlmMessage::User { content } => content.clone(),
                LlmMessage::Assistant { content, tool_calls } => {
                    let mut s = content.clone().unwrap_or_default();
                    for call in tool_calls {
                        s.push_str(&call.name);
                        s.push_str(&call.arguments.to_string());
                    }
                    s
                }
                LlmMessage::ToolResult { content, .. } => content.clone(),
            };
            total += bpe.encode_with_special_tokens(&text).len();
        }
        Ok(total)
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
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "Hello there", "tool_calls": null},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let response = client
            .send_message("You are helpful.", &messages, &[], "gpt-4o", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "Hello there"),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_message_returns_tool_calls_with_parsed_arguments() {
        // OpenAI's function-call arguments arrive as a JSON-encoded
        // STRING, not a nested object -- this is the one real structural
        // difference from Anthropic worth its own test.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "fetch_weather", "arguments": "{\"city\": \"Warsaw\"}"}
                        }]
                    },
                    "finish_reason": "tool_calls"
                }]
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "weather in Warsaw?".into() }];
        let tools = vec![ToolDefinition {
            name: "fetch_weather".into(),
            description: "Get weather".into(),
            parameters: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        let response = client
            .send_message("You are helpful.", &messages, &tools, "gpt-4o", "test-key")
            .await
            .unwrap();

        match response {
            ProviderResponse::ToolCalls { text: _, calls } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "fetch_weather");
                assert_eq!(calls[0].arguments, serde_json::json!({"city": "Warsaw"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn assistant_tool_calls_are_sent_with_json_string_encoded_arguments() {
        // Guards the *request*-building direction, not just response
        // parsing: LlmMessage::Assistant { tool_calls } must be encoded
        // into the outgoing OpenAI-shape request with `arguments` as a
        // JSON-encoded STRING (via message_to_json's
        // serde_json::to_string(&call.arguments)), not a nested object.
        // A bug here (e.g. embedding the raw Value instead of stringifying
        // it) would still let the OTHER tests above pass, since those only
        // assert on the mocked response -- this test would catch it.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({
                "messages": [
                    {"role": "system", "content": "You are helpful."},
                    {"role": "user", "content": "weather in Warsaw?"},
                    {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call_1",
                            "type": "function",
                            "function": {"name": "fetch_weather", "arguments": "{\"city\":\"Warsaw\"}"}
                        }]
                    },
                    {"role": "tool", "tool_call_id": "call_1", "content": "{\"temp_c\":22}"}
                ]
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "It's sunny.", "tool_calls": null},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![
            LlmMessage::User { content: "weather in Warsaw?".into() },
            LlmMessage::Assistant {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "fetch_weather".into(),
                    arguments: serde_json::json!({"city": "Warsaw"}),
                }],
            },
            LlmMessage::ToolResult { tool_call_id: "call_1".into(), content: "{\"temp_c\":22}".into(), is_error: false },
        ];
        let response = client.send_message("You are helpful.", &messages, &[], "gpt-4o", "test-key").await.unwrap();

        match response {
            ProviderResponse::Text(text) => assert_eq!(text, "It's sunny."),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"message": "Incorrect API key provided", "type": "invalid_request_error"}
            })))
            .mount(&server)
            .await;

        let client = OpenAiClient::with_base_url(server.uri());
        let messages = vec![LlmMessage::User { content: "hi".into() }];
        let result = client.send_message("You are helpful.", &messages, &[], "gpt-4o", "test-key").await;
        assert!(matches!(result, Err(crate::node::NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn count_tokens_computes_a_real_count_without_a_network_call() {
        let client = OpenAiClient::with_base_url("http://unreachable.invalid".into());
        let messages = vec![LlmMessage::User { content: "hello world, this is a test message".into() }];
        let count = client.count_tokens("You are helpful.", &messages, "gpt-4o", "test-key").await.unwrap();
        // Not asserting an exact number (tokenizer internals aren't this
        // test's concern) -- just that it's a real, nonzero, sane count
        // computed locally (the unreachable base_url above proves no
        // network call happened, since a real call would time out/fail).
        assert!(count > 0 && count < 100);
    }

    #[tokio::test]
    async fn defaults_to_the_real_openai_base_url_when_none_given() {
        let client = OpenAiClient::new(None).unwrap();
        assert_eq!(client.base_url, "https://api.openai.com/v1");
    }
}
