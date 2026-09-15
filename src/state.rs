use crate::node::NodeRegistry;
use crate::scheduler::Scheduler;
use crate::storage::Storage;
use crate::trigger_registry::TriggerRegistry;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub registry: Arc<NodeRegistry>,
    pub jwt_secret: String,
    pub scheduler: Arc<Scheduler>,
    pub trigger_registry: Arc<TriggerRegistry>,
}
