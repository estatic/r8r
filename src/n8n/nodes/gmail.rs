//! Gmail node (spec §6.6), v2.x (2/2.1/2.2), faithful to n8n's
//! `nodes/Google/Gmail/v2/*`. Placeholder -- filled in incrementally.

use crate::n8n::node::{ExecCtx, NodeResult, NodeType};
use crate::n8n::types::NodeOutput;

pub struct Gmail;

#[async_trait::async_trait]
impl NodeType for Gmail {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.gmail"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let _ = ctx;
        Ok(vec![vec![]])
    }
}
