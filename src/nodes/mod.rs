pub mod if_node;
pub mod manual_trigger;
pub mod set;

use crate::node::NodeRegistry;

pub fn register_all(registry: &mut NodeRegistry) {
    registry.register(Box::new(manual_trigger::ManualTriggerNode));
    registry.register(Box::new(set::SetNode));
    registry.register(Box::new(if_node::IfNode));
}
