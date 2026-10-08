pub mod sqlite;

use crate::domain::{Credential, CredentialSummary, Execution, User, Workflow};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
    async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
    async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()>;
    /// An AI Agent's remembered conversation (`[{role, content}]`) by key.
    /// Storages without memory remember nothing.
    async fn get_agent_memory(&self, _key: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        Ok(Vec::new())
    }
    async fn put_agent_memory(&self, _key: &str, _messages: &[serde_json::Value]) -> anyhow::Result<()> {
        Ok(())
    }
    async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>>;
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>>;

    async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()>;
    async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()>;
    async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>>;
    async fn list_executions_for_workflow(
        &self,
        workflow_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<Execution>>;

    async fn create_user(&self, user: &User) -> anyhow::Result<()>;
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>>;
    /// True once at least one user has ever been created -- used to gate
    /// self-registration after the first (owner) account exists.
    async fn any_user_exists(&self) -> anyhow::Result<bool>;

    async fn create_credential(&self, credential: &Credential) -> anyhow::Result<()>;
    async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>>;
    async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>>;
    /// Rewrites `name`, `data` (re-encrypted) and `updated_at`; never the
    /// type, owner or `created_at`. `Ok(false)` if no such credential.
    async fn update_credential(&self, credential: &Credential) -> anyhow::Result<bool>;
    /// `Ok(false)` if no such credential.
    async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool>;
    async fn create_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<()>;
    async fn get_tool(&self, id: Uuid) -> anyhow::Result<Option<crate::domain::Tool>>;
    async fn list_tools(&self) -> anyhow::Result<Vec<crate::domain::Tool>>;
    /// `Ok(false)` if no such tool.
    async fn update_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<bool>;
    /// `Ok(false)` if no such tool.
    async fn delete_tool(&self, id: Uuid) -> anyhow::Result<bool>;
}

/// Lets nodes keep chat memory in a [`Storage`].
pub struct StorageMemory(pub std::sync::Arc<dyn Storage>);

#[async_trait]
impl crate::node::MemoryStore for StorageMemory {
    async fn load(&self, key: &str) -> anyhow::Result<Vec<serde_json::Value>> {
        self.0.get_agent_memory(key).await
    }
    async fn save(&self, key: &str, messages: &[serde_json::Value]) -> anyhow::Result<()> {
        self.0.put_agent_memory(key, messages).await
    }
}
