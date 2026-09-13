use crate::node::NodeRegistry;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub registry: Arc<NodeRegistry>,
    pub jwt_secret: String,
}
