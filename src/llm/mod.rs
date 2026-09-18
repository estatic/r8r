//! LLM provider clients for the `ai.agent` node. Intentionally
//! self-contained (mirrors `src/expr.rs`'s own convention): depends only
//! on `serde_json`/`async-trait`/`reqwest`, not on `domain.rs` or
//! `node.rs`, except for reusing `crate::node::NodeError` as the error
//! type so provider-client failures compose naturally with every other
//! node's error handling.

pub mod anthropic;
pub mod openai;

#[derive(Debug, Clone)]
pub enum LlmMessage {
    User { content: String },
    Assistant { content: Option<String>, tool_calls: Vec<ToolCall> },
    ToolResult { tool_call_id: String, content: String, is_error: bool },
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// A JSON Schema object, passed close to verbatim into each
    /// provider's own tool-definition wrapper shape.
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum ProviderResponse {
    Text(String),
    ToolCalls(Vec<ToolCall>),
}

#[async_trait::async_trait]
pub trait ProviderClient: Send + Sync {
    async fn send_message(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        model: &str,
        api_key: &str,
    ) -> Result<ProviderResponse, crate::node::NodeError>;

    /// A real per-provider token count for the memory-budget trim -- not
    /// an approximation.
    async fn count_tokens(
        &self,
        system_prompt: &str,
        messages: &[LlmMessage],
        model: &str,
        api_key: &str,
    ) -> Result<usize, crate::node::NodeError>;
}
