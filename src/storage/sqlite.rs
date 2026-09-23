use super::Storage;
use crate::domain::{Credential, CredentialSummary, Execution, User, Workflow};
use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;
use uuid::Uuid;

pub struct SqliteStorage {
    pool: SqlitePool,
    encryption_key: [u8; 32],
}

impl SqliteStorage {
    pub async fn new(db_url: &str, encryption_key: [u8; 32]) -> anyhow::Result<Self> {
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
        Ok(Self { pool, encryption_key })
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
        let definition = serde_json::json!({
            "nodes": workflow.nodes,
            "connections": workflow.connections,
        });
        sqlx::query(
            "INSERT INTO workflows (id, name, active, definition, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(workflow.id.to_string())
        .bind(&workflow.name)
        .bind(workflow.active as i64)
        .bind(definition.to_string())
        .bind(workflow.created_at.to_rfc3339())
        .bind(workflow.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    async fn update_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
        let definition = serde_json::json!({
            "nodes": workflow.nodes,
            "connections": workflow.connections,
        });
        sqlx::query(
            "UPDATE workflows SET name = ?, active = ?, definition = ?, updated_at = ? WHERE id = ?"
        )
        .bind(&workflow.name)
        .bind(workflow.active as i64)
        .bind(definition.to_string())
        .bind(workflow.updated_at.to_rfc3339())
        .bind(workflow.id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()> {
        // `executions.workflow_id` references `workflows(id)` with no
        // `ON DELETE CASCADE`, so a workflow that has ever been executed
        // must have its executions removed first or the DELETE below fails
        // with a FOREIGN KEY constraint violation. Do both deletes in a
        // transaction so they're atomic — either the workflow and its
        // executions are gone, or neither is touched.
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM executions WHERE workflow_id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>> {
        let row = sqlx::query_as::<_, (String, String, i64, String, String, String)>(
            "SELECT id, name, active, definition, created_at, updated_at FROM workflows WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_workflow).transpose()?)
    }
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
        let rows = sqlx::query_as::<_, (String, String, i64, String, String, String)>(
            "SELECT id, name, active, definition, created_at, updated_at FROM workflows"
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_workflow).collect()
    }
    async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()> {
        let data = serde_json::to_string(&execution.node_outputs)?;
        sqlx::query(
            "INSERT INTO executions (id, workflow_id, status, mode, data, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(execution.id.to_string())
        .bind(execution.workflow_id.to_string())
        .bind(serde_json::to_string(&execution.status)?)
        .bind(serde_json::to_string(&execution.mode)?)
        .bind(data)
        .bind(execution.started_at.to_rfc3339())
        .bind(execution.finished_at.map(|t| t.to_rfc3339()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()> {
        let data = serde_json::to_string(&execution.node_outputs)?;
        sqlx::query("UPDATE executions SET status = ?, data = ?, finished_at = ? WHERE id = ?")
            .bind(serde_json::to_string(&execution.status)?)
            .bind(data)
            .bind(execution.finished_at.map(|t| t.to_rfc3339()))
            .bind(execution.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>> {
        let row = sqlx::query_as::<_, (String, String, String, String, String, String, Option<String>)>(
            "SELECT id, workflow_id, status, mode, data, started_at, finished_at FROM executions WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_execution).transpose()
    }
    async fn list_executions_for_workflow(
        &self,
        workflow_id: Uuid,
        limit: i64,
    ) -> anyhow::Result<Vec<Execution>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, String, String, Option<String>)>(
            "SELECT id, workflow_id, status, mode, data, started_at, finished_at \
             FROM executions WHERE workflow_id = ? ORDER BY started_at DESC LIMIT ?"
        )
        .bind(workflow_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_execution).collect()
    }
    async fn create_user(&self, user: &User) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(user.id.to_string())
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(serde_json::to_string(&user.role)?)
        .bind(user.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
        let row = sqlx::query_as::<_, (String, String, String, String, String)>(
            "SELECT id, email, password_hash, role, created_at FROM users WHERE email = ?"
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|(id, email, password_hash, role, created_at)| {
            Ok::<_, anyhow::Error>(User {
                id: Uuid::parse_str(&id)?,
                email,
                password_hash,
                role: serde_json::from_str(&role)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
            })
        })
        .transpose()
    }

    async fn any_user_exists(&self) -> anyhow::Result<bool> {
        let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users)")
            .fetch_one(&self.pool)
            .await?;
        Ok(exists != 0)
    }

    async fn create_credential(&self, credential: &Credential) -> anyhow::Result<()> {
        let plaintext = serde_json::to_string(&credential.data)?;
        let encrypted = crate::crypto::encrypt(&self.encryption_key, &plaintext)?;
        sqlx::query(
            "INSERT INTO credentials (id, name, credential_type, data, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(credential.id.to_string())
        .bind(&credential.name)
        .bind(&credential.credential_type)
        .bind(encrypted)
        .bind(credential.owner_id.to_string())
        .bind(credential.created_at.to_rfc3339())
        .bind(credential.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>> {
        let row = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
            "SELECT id, name, credential_type, data, owner_id, created_at, updated_at FROM credentials WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| row_to_credential(r, &self.encryption_key)).transpose()
    }

    async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, String, String)>(
            "SELECT id, name, credential_type, owner_id, created_at, updated_at FROM credentials"
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_credential_summary).collect()
    }
}

fn row_to_credential(
    row: (String, String, String, String, String, String, String),
    key: &[u8; 32],
) -> anyhow::Result<Credential> {
    let (id, name, credential_type, encrypted_data, owner_id, created_at, updated_at) = row;
    let plaintext = crate::crypto::decrypt(key, &encrypted_data)?;
    Ok(Credential {
        id: Uuid::parse_str(&id)?,
        name,
        credential_type,
        data: serde_json::from_str(&plaintext)?,
        owner_id: Uuid::parse_str(&owner_id)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}

fn row_to_credential_summary(
    row: (String, String, String, String, String, String),
) -> anyhow::Result<CredentialSummary> {
    let (id, name, credential_type, owner_id, created_at, updated_at) = row;
    Ok(CredentialSummary {
        id: Uuid::parse_str(&id)?,
        name,
        credential_type,
        owner_id: Uuid::parse_str(&owner_id)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}

fn row_to_workflow(
    row: (String, String, i64, String, String, String),
) -> anyhow::Result<Workflow> {
    let (id, name, active, definition, created_at, updated_at) = row;
    let def: serde_json::Value = serde_json::from_str(&definition)?;
    Ok(Workflow {
        id: Uuid::parse_str(&id)?,
        name,
        active: active != 0,
        nodes: serde_json::from_value(def["nodes"].clone())?,
        connections: serde_json::from_value(def["connections"].clone())?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}

fn row_to_execution(
    row: (String, String, String, String, String, String, Option<String>),
) -> anyhow::Result<Execution> {
    let (id, workflow_id, status, mode, data, started_at, finished_at) = row;
    Ok(Execution {
        id: Uuid::parse_str(&id)?,
        workflow_id: Uuid::parse_str(&workflow_id)?,
        status: serde_json::from_str(&status)?,
        mode: serde_json::from_str(&mode)?,
        node_outputs: serde_json::from_str(&data)?,
        started_at: chrono::DateTime::parse_from_rfc3339(&started_at)?.with_timezone(&chrono::Utc),
        finished_at: finished_at
            .map(|t| chrono::DateTime::parse_from_rfc3339(&t).map(|d| d.with_timezone(&chrono::Utc)))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Credential, Execution, ExecutionMode, ExecutionStatus, Item, NodeInstance, User, UserRole};
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_key() -> [u8; 32] {
        [5u8; 32]
    }

    async fn storage_with_test_key() -> SqliteStorage {
        SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap()
    }

    fn sample_credential(owner_id: Uuid) -> Credential {
        Credential {
            id: Uuid::new_v4(),
            name: "test-api".into(),
            credential_type: "bearer".into(),
            data: serde_json::json!({"token": "abc123secret"}),
            owner_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn sample_workflow() -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "sample".into(),
            active: false,
            nodes: vec![NodeInstance {
                id: "n1".into(),
                node_type: "core.manualTrigger".into(),
                position: (0.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
                settings: Default::default(),
            }],
            connections: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn create_and_get_workflow_round_trips() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();

        let fetched = storage.get_workflow(wf.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, wf.id);
        assert_eq!(fetched.name, wf.name);
        assert_eq!(fetched.nodes, wf.nodes);
    }

    #[tokio::test]
    async fn list_workflows_returns_created_ones() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        storage.create_workflow(&sample_workflow()).await.unwrap();
        storage.create_workflow(&sample_workflow()).await.unwrap();

        let all = storage.list_workflows().await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn get_workflow_returns_none_when_missing() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let result = storage.get_workflow(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    fn sample_execution(workflow_id: Uuid) -> Execution {
        Execution {
            id: Uuid::new_v4(),
            workflow_id,
            status: ExecutionStatus::Running,
            mode: ExecutionMode::Manual,
            node_outputs: HashMap::new(),
            started_at: Utc::now(),
            finished_at: None,
        }
    }

    #[tokio::test]
    async fn create_and_get_execution_round_trips() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();
        let exec = sample_execution(wf.id);
        storage.create_execution(&exec).await.unwrap();

        let fetched = storage.get_execution(exec.id).await.unwrap().unwrap();
        assert_eq!(fetched.id, exec.id);
        assert_eq!(fetched.status, ExecutionStatus::Running);
    }

    #[tokio::test]
    async fn update_execution_persists_new_status_and_outputs() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();
        let mut exec = sample_execution(wf.id);
        storage.create_execution(&exec).await.unwrap();

        exec.status = ExecutionStatus::Success;
        exec.finished_at = Some(Utc::now());
        exec.node_outputs.insert(
            "n1".into(),
            vec![Item {
                json: serde_json::json!({"a": 1}),
                binary: serde_json::json!({}),
            }],
        );
        storage.update_execution(&exec).await.unwrap();

        let fetched = storage.get_execution(exec.id).await.unwrap().unwrap();
        assert_eq!(fetched.status, ExecutionStatus::Success);
        assert!(fetched.finished_at.is_some());
        assert_eq!(
            fetched.node_outputs["n1"][0].json,
            serde_json::json!({"a": 1})
        );
    }

    #[tokio::test]
    async fn list_executions_for_workflow_returns_newest_first_and_respects_limit() {
        let storage = storage_with_test_key().await;
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();

        let mut oldest = sample_execution(wf.id);
        oldest.started_at = Utc::now() - chrono::Duration::minutes(2);
        let mut middle = sample_execution(wf.id);
        middle.started_at = Utc::now() - chrono::Duration::minutes(1);
        let mut newest = sample_execution(wf.id);
        newest.started_at = Utc::now();
        for exec in [&oldest, &middle, &newest] {
            storage.create_execution(exec).await.unwrap();
        }

        // A different workflow's execution must never appear in this list.
        let other_wf = sample_workflow();
        storage.create_workflow(&other_wf).await.unwrap();
        storage.create_execution(&sample_execution(other_wf.id)).await.unwrap();

        let page = storage.list_executions_for_workflow(wf.id, 2).await.unwrap();

        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, newest.id);
        assert_eq!(page[1].id, middle.id);
    }

    #[tokio::test]
    async fn new_connects_and_migrates() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
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
        let storage = SqliteStorage::new(&db_url, test_key()).await.unwrap();

        assert!(
            db_path.exists(),
            "SqliteStorage::new should create the missing db file"
        );

        let workflows = storage.list_workflows().await.unwrap();
        assert!(workflows.is_empty());

        drop(storage);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample_user() -> User {
        User {
            id: Uuid::new_v4(),
            email: "user@example.com".into(),
            password_hash: "irrelevant-for-storage-test".into(),
            role: UserRole::Owner,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn create_and_get_user_by_email_round_trips() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let user = sample_user();
        storage.create_user(&user).await.unwrap();

        let fetched = storage.get_user_by_email(&user.email).await.unwrap().unwrap();
        assert_eq!(fetched.id, user.id);
        assert_eq!(fetched.role, UserRole::Owner);
    }

    #[tokio::test]
    async fn get_user_by_email_returns_none_when_missing() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        assert!(storage.get_user_by_email("nobody@example.com").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn any_user_exists_reflects_whether_a_user_has_been_created() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        assert!(!storage.any_user_exists().await.unwrap());

        storage.create_user(&sample_user()).await.unwrap();

        assert!(storage.any_user_exists().await.unwrap());
    }

    #[tokio::test]
    async fn update_workflow_persists_active_flag_and_name() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let mut wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();

        wf.active = true;
        wf.name = "renamed".into();
        wf.updated_at = Utc::now();
        storage.update_workflow(&wf).await.unwrap();

        let fetched = storage.get_workflow(wf.id).await.unwrap().unwrap();
        assert!(fetched.active);
        assert_eq!(fetched.name, "renamed");
    }

    #[tokio::test]
    async fn update_workflow_on_unknown_id_does_not_error() {
        // Matches sqlx's own UPDATE-affecting-zero-rows behavior: no error, just a no-op.
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        let result = storage.update_workflow(&wf).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn delete_workflow_removes_it() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();

        storage.delete_workflow(wf.id).await.unwrap();

        assert!(storage.get_workflow(wf.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_workflow_on_a_nonexistent_id_does_not_error() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        storage.delete_workflow(Uuid::new_v4()).await.unwrap();
    }

    #[tokio::test]
    async fn delete_workflow_with_executions_removes_both() {
        // Regression test: executions.workflow_id references workflows(id)
        // with no ON DELETE CASCADE, so deleting a workflow that has been
        // executed used to fail with a FOREIGN KEY constraint violation.
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();
        let exec = sample_execution(wf.id);
        storage.create_execution(&exec).await.unwrap();

        storage.delete_workflow(wf.id).await.unwrap();

        assert!(storage.get_workflow(wf.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn create_and_get_credential_round_trips_decrypted() {
        let storage = storage_with_test_key().await;
        let user = sample_user();
        storage.create_user(&user).await.unwrap();
        let cred = sample_credential(user.id);
        storage.create_credential(&cred).await.unwrap();

        let fetched = storage.get_credential(cred.id).await.unwrap().unwrap();
        assert_eq!(fetched.data, serde_json::json!({"token": "abc123secret"}));
        assert_eq!(fetched.name, "test-api");
    }

    #[tokio::test]
    async fn get_credential_returns_none_when_missing() {
        let storage = storage_with_test_key().await;
        assert!(storage.get_credential(Uuid::new_v4()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_credentials_never_includes_decrypted_data() {
        let storage = storage_with_test_key().await;
        let user = sample_user();
        storage.create_user(&user).await.unwrap();
        storage.create_credential(&sample_credential(user.id)).await.unwrap();

        let summaries = storage.list_credentials().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "test-api");
        // CredentialSummary has no `data` field at the type level -- this test
        // exists to document that guarantee at the storage layer, not just the
        // API-serialization layer (Task 2's test covers serialization).
    }

    #[tokio::test]
    async fn stored_data_is_actually_encrypted_at_rest() {
        // Read the raw column value directly (bypassing get_credential's decrypt
        // step) and confirm the plaintext secret never appears in it.
        let storage = storage_with_test_key().await;
        let user = sample_user();
        storage.create_user(&user).await.unwrap();
        let cred = sample_credential(user.id);
        storage.create_credential(&cred).await.unwrap();

        let raw: (String,) = sqlx::query_as("SELECT data FROM credentials WHERE id = ?")
            .bind(cred.id.to_string())
            .fetch_one(&storage.pool)
            .await
            .unwrap();
        assert!(!raw.0.contains("abc123secret"));
    }
}
