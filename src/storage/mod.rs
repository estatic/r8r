pub mod sqlite;

use crate::domain::{Credential, CredentialSummary, Execution, User, Workflow};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
    async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
    async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()>;
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
}
