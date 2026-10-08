use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

/// Loop Over Items (n8n's SplitInBatches v3): sends its input through the
/// nodes on its "loop" output `batch_size` items at a time, and once all are
/// done sends everything those nodes brought back out of "done". The engine
/// runs the iteration (`engine::Run::run_loop`); `execute` only runs if the
/// node is used outside the engine's loop handling.
pub struct LoopNode;

#[async_trait]
impl Node for LoopNode {
    fn type_name(&self) -> &'static str {
        crate::engine::LOOP_TYPE
    }
    fn display_name(&self) -> &'static str {
        "Loop Over Items"
    }
    fn description(&self) -> &'static str {
        "Runs the nodes on its \"loop\" output for each batch of items, then continues from \"done\" with all of them."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::FlowControl
    }
    fn icon(&self) -> &'static str {
        "🔁"
    }
    fn output_ports(&self, _parameters: &serde_json::Value) -> Vec<String> {
        vec!["done".to_string(), "loop".to_string()]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![ctx.input_items.clone(), vec![]])
    }
}
