use crate::domain::Item;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("node execution failed: {0}")]
    ExecutionFailed(String),
}

#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;
    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError>;
}

#[derive(Default)]
pub struct NodeRegistry {
    nodes: HashMap<&'static str, Box<dyn Node>>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, node: Box<dyn Node>) {
        self.nodes.insert(node.type_name(), node);
    }

    pub fn get(&self, type_name: &str) -> Option<&dyn Node> {
        self.nodes.get(type_name).map(|b| b.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::domain::Item;

    struct EchoNode;

    #[async_trait]
    impl Node for EchoNode {
        fn type_name(&self) -> &'static str {
            "test.echo"
        }
        async fn execute(&self, ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError> {
            Ok(ctx.input_items.clone())
        }
    }

    #[tokio::test]
    async fn registry_dispatches_to_registered_node() {
        let mut registry = NodeRegistry::new();
        registry.register(Box::new(EchoNode));

        let node = registry.get("test.echo").expect("node should be registered");
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({}),
            input_items: vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].json, serde_json::json!({"x": 1}));
    }

    #[test]
    fn registry_returns_none_for_unknown_type() {
        let registry = NodeRegistry::new();
        assert!(registry.get("does.not.exist").is_none());
    }
}
