use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

/// The workflow-graph representation of a Telegram long-polling trigger.
///
/// Like `ScheduleNode`/`WebhookNode`, this node's own `execute()` is a
/// fallback stub — real incoming-update data reaches the workflow via the
/// trigger item the engine seeds in from `telegram_poller::poll_telegram_updates`
/// (see `src/telegram_poller.rs`), not from this method. `execute()` only
/// runs if something re-executes this node directly outside a real trigger
/// firing (e.g. a manual re-run), in which case an empty item is the only
/// sane fallback — there is no "current Telegram update" to reproduce.
pub struct TelegramTriggerNode;

#[async_trait]
impl Node for TelegramTriggerNode {
    fn type_name(&self) -> &'static str {
        "telegram.trigger"
    }
    fn display_name(&self) -> &'static str {
        "Telegram Trigger"
    }
    fn description(&self) -> &'static str {
        "Fires when a message arrives for the configured Telegram bot."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Trigger
    }
    fn icon(&self) -> &'static str {
        "📨"
    }
    fn credential_types(&self) -> &'static [&'static str] {
        &["telegramApi"]
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fallback_execute_returns_a_single_empty_item() {
        let node = TelegramTriggerNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"auth": {"credential_id": "00000000-0000-0000-0000-000000000000"}}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[0][0].json, serde_json::json!({}));
    }

    #[test]
    fn type_name_is_telegram_trigger() {
        assert_eq!(TelegramTriggerNode.type_name(), "telegram.trigger");
    }
}
