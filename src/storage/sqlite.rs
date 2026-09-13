use super::Storage;
use crate::domain::{Execution, User, Workflow};
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub async fn new(db_url: &str) -> anyhow::Result<Self> {
        // `:memory:` SQLite databases are private per-connection unless a
        // shared-cache URI is used. Cap the pool at a single connection for
        // in-memory URLs so all queries in a test (or process) share the
        // same in-memory database rather than silently getting a fresh,
        // empty one from a different pooled connection.
        let mut options = SqlitePoolOptions::new();
        if db_url.contains(":memory:") {
            options = options.max_connections(1);
        }
        let pool = options.connect(db_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn create_workflow(&self, _workflow: &Workflow) -> anyhow::Result<()> {
        unimplemented!("added in Task 4")
    }
    async fn get_workflow(&self, _id: Uuid) -> anyhow::Result<Option<Workflow>> {
        unimplemented!("added in Task 4")
    }
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
        Ok(vec![])
    }
    async fn create_execution(&self, _execution: &Execution) -> anyhow::Result<()> {
        unimplemented!("added in Task 5")
    }
    async fn update_execution(&self, _execution: &Execution) -> anyhow::Result<()> {
        unimplemented!("added in Task 5")
    }
    async fn get_execution(&self, _id: Uuid) -> anyhow::Result<Option<Execution>> {
        unimplemented!("added in Task 5")
    }
    async fn create_user(&self, _user: &User) -> anyhow::Result<()> {
        unimplemented!("added in Task 6")
    }
    async fn get_user_by_email(&self, _email: &str) -> anyhow::Result<Option<User>> {
        unimplemented!("added in Task 6")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_connects_and_migrates() {
        let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
        let workflows = storage.list_workflows().await.unwrap();
        assert!(workflows.is_empty());
    }
}
