use crate::domain::Item;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
    /// Library tools the run's agents reference (spec B1 §5.2).
    pub tools: std::collections::HashMap<uuid::Uuid, crate::domain::Tool>,
    pub tool_executor: Option<std::sync::Arc<dyn ToolExecutor>>,
}

impl std::fmt::Debug for NodeExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeExecutionContext")
            .field("parameters", &self.parameters)
            .field("input_items", &self.input_items)
            .field("credentials", &self.credentials)
            .field("tool_executor", &self.tool_executor.as_ref().map(|_| "<tool_executor>"))
            .finish()
    }
}

/// Lets a node (in practice, only `ai.agent`) invoke another registered
/// node type mid-execution -- the mechanism behind tool-calling. Kept
/// deliberately narrow (one method, no registry/workflow access exposed
/// directly) so every other node type is completely unaffected by its
/// existence; see `docs/superpowers/specs/2026-09-18-r8r-plan5-ai-agent-design.md`
/// section 3 for the full rationale, including why this is `Arc<dyn
/// ToolExecutor>` (a 'static trait object) rather than a borrowed
/// reference: a borrowed reference would need a lifetime parameter on
/// `NodeExecutionContext` itself, which would then need to appear on
/// every `Node::execute()` signature across all existing node types.
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call_tool(&self, node_type: &str, parameters: serde_json::Value) -> Result<NodeOutput, NodeError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("node execution failed: {0}")]
    ExecutionFailed(String),
}

pub type NodeOutput = Vec<Vec<Item>>;

/// Groups node types for the add-node menu and canvas styling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeCategory {
    Trigger,
    Action,
    FlowControl,
    Ai,
}

#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;

    /// Whether this node's parameters should be run through the expression
    /// engine (`expr::resolve_parameters`) before `execute()` is called.
    ///
    /// Defaults to `true`, which is correct for nearly every node: its
    /// parameters are values that may contain `{{ }}` expressions to
    /// interpolate. A node should override this to return `false` only when
    /// its "parameters" include something that is not itself a value to
    /// interpolate but must be passed through verbatim — e.g. `core.code`'s
    /// `script`, which is source code that may legitimately contain literal
    /// `{{ }}` text with no relation to r8r's expression syntax.
    fn resolves_parameters(&self) -> bool {
        true
    }

    /// Human-readable name shown in the canvas and add-node menu, e.g.
    /// "Telegram Trigger" for `telegram.trigger`. No default -- every node
    /// type must declare one.
    fn display_name(&self) -> &'static str;

    /// One sentence describing what the node does, shown in the add-node
    /// menu. No default.
    fn description(&self) -> &'static str;

    /// Groups this node type in the add-node menu and drives canvas
    /// styling. No default.
    fn category(&self) -> NodeCategory;

    /// A single emoji shown next to the node's name.
    fn icon(&self) -> &'static str {
        "⚙️"
    }

    /// Credential type string(s) this node's `auth.credential_id` accepts,
    /// e.g. `&["telegramApi"]`. Empty means "no credential" for a node with
    /// no `auth` parameter, or "any credential" for one (like
    /// `core.httpRequest`) whose own `auth.type` parameter picks the shape
    /// at runtime -- the credential picker treats an empty list as "don't
    /// filter", not "don't allow".
    fn credential_types(&self) -> &'static [&'static str] {
        &[]
    }

    /// This node instance's current output ports, labeled. Takes
    /// `parameters` because a port count can be data-dependent (see
    /// `core.switch`'s override in Task 1); every other node type ignores
    /// the argument and returns a fixed list.
    fn output_ports(&self, parameters: &serde_json::Value) -> Vec<String> {
        let _ = parameters;
        vec!["main".to_string()]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError>;
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

    /// All registered node type names, sorted for a stable, predictable
    /// order in any UI listing them (e.g. an "add node" picker).
    pub fn type_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.nodes.keys().copied().collect();
        names.sort_unstable();
        names
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
        fn display_name(&self) -> &'static str {
            "Echo"
        }
        fn description(&self) -> &'static str {
            "Test-only node that returns its input unchanged."
        }
        fn category(&self) -> NodeCategory {
            NodeCategory::Action
        }
        async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
            Ok(vec![ctx.input_items.clone()])
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
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1); // one output port
        assert_eq!(result[0][0].json, serde_json::json!({"x": 1}));
    }

    #[test]
    fn registry_returns_none_for_unknown_type() {
        let registry = NodeRegistry::new();
        assert!(registry.get("does.not.exist").is_none());
    }

    #[test]
    fn node_execution_context_default_has_empty_credentials() {
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({}),
            input_items: vec![],
            ..Default::default()
        };
        assert!(ctx.credentials.is_empty());
    }

    #[test]
    fn type_names_returns_registered_types_sorted() {
        let mut registry = NodeRegistry::new();
        registry.register(Box::new(EchoNode));
        assert_eq!(registry.type_names(), vec!["test.echo"]);
    }

    #[test]
    fn node_metadata_defaults_are_correct() {
        // EchoNode doesn't override icon/credential_types/output_ports, so this
        // proves the trait's own defaults, not any node's override.
        let node = EchoNode;
        assert_eq!(node.icon(), "⚙️");
        assert!(node.credential_types().is_empty());
        assert_eq!(node.output_ports(&serde_json::json!({})), vec!["main".to_string()]);
    }

    #[test]
    fn every_registered_node_has_non_empty_metadata_and_a_unique_display_name() {
        let mut registry = NodeRegistry::new();
        crate::nodes::register_all(&mut registry);
        let mut seen_display_names = std::collections::HashSet::new();
        for type_name in registry.type_names() {
            let node = registry.get(type_name).expect("type_names() and get() must agree");
            assert!(!node.display_name().is_empty(), "{type_name} has an empty display_name");
            assert!(!node.description().is_empty(), "{type_name} has an empty description");
            assert!(!node.icon().is_empty(), "{type_name} has an empty icon");
            assert!(
                seen_display_names.insert(node.display_name()),
                "{type_name}'s display_name \"{}\" collides with another node type's",
                node.display_name()
            );
        }
    }
}
