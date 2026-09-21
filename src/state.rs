use crate::execution_runner::ExecutionEvent;
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
    pub execution_events: tokio::sync::broadcast::Sender<ExecutionEvent>,
    /// When false (the default), `POST /rest/auth/register` refuses to
    /// create a second user once any user already exists -- see
    /// `api::auth::register`. Set from `R8R_ALLOW_OPEN_REGISTRATION` at
    /// startup; never re-read per request.
    pub open_registration: bool,
}
