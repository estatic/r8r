pub mod filter;
pub mod if_node;
pub mod manual_trigger;
pub mod merge;
pub mod noop;
pub mod set;
pub mod switch;
pub mod wait;

use crate::node::NodeRegistry;

pub fn register_all(registry: &mut NodeRegistry) {
    registry.register(Box::new(manual_trigger::ManualTriggerNode));
    registry.register(Box::new(set::SetNode));
    registry.register(Box::new(if_node::IfNode));
    registry.register(Box::new(switch::SwitchNode));
    registry.register(Box::new(merge::MergeNode));
    registry.register(Box::new(filter::FilterNode));
    registry.register(Box::new(wait::WaitNode));
    registry.register(Box::new(noop::NoOpNode));
}
