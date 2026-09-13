use super::Storage;
use crate::domain::{Execution, User, Workflow};
use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;

pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub async fn new(db_url: &str) -> anyhow::Result<Self> {
        // Parse the URL into connect options and explicitly request
        // `create_if_missing(true)`. Without this, sqlx opens file-based
        // databases with SQLITE_OPEN_READWRITE only, which SQLite rejects
        // (SQLITE_CANTOPEN) for a path that doesn't exist yet — breaking
        // "zero-config self-hosting" on a genuine first run against a real
        // db file, regardless of whether the caller's `db_url` happens to
        // include `?mode=rwc`.
        let connect_options = SqliteConnectOptions::from_str(db_url)?.create_if_missing(true);

        // `:memory:` SQLite databases are private per-connection unless a
        // shared-cache URI is used. Cap the pool at a single connection for
        // in-memory URLs so all queries in a test (or process) share the
        // same in-memory database rather than silently getting a fresh,
        // empty one from a different pooled connection.
        let mut pool_options = SqlitePoolOptions::new();
        if db_url.contains(":memory:") {
            pool_options = pool_options.max_connections(1);
        }

        let pool = pool_options.connect_with(connect_options).await?;
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

    #[tokio::test]
    async fn new_creates_missing_file_database() {
        // Regression test: SqliteStorage::new must succeed against a
        // file-based db_url that points at a path with no existing file,
        // and must actually create that file (create_if_missing).
        let dir = std::env::temp_dir().join(format!(
            "r8r-sqlite-new-test-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("r8r.db");
        assert!(!db_path.exists(), "precondition: db file must not exist yet");

        let db_url = format!("sqlite://{}", db_path.display());
        let storage = SqliteStorage::new(&db_url).await.unwrap();

        assert!(
            db_path.exists(),
            "SqliteStorage::new should create the missing db file"
        );

        let workflows = storage.list_workflows().await.unwrap();
        assert!(workflows.is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
