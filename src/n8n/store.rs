//! Persistence for n8n entities: workflows (full JSON), credentials
//! (n8n-encrypted) and executions.

use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

#[derive(Clone)]
pub struct Store {
    pub(super) pool: SqlitePool,
    /// All writes go through one connection: SQLite has a single writer, and
    /// queueing here is much faster than connections retrying on SQLITE_BUSY.
    pub(super) writer: SqlitePool,
    /// Group-committed execution writes (see `store_batch`).
    pub(super) batch: tokio::sync::mpsc::UnboundedSender<super::store_batch::Op>,
    encryption_key: String,
}

#[derive(Debug, Clone)]
pub struct CredentialRecord {
    pub id: String,
    pub name: String,
    pub cred_type: String,
    /// Encrypted blob as stored.
    pub data: String,
    pub created_at: String,
    pub updated_at: String,
}

pub(super) fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// n8n-style id: 16 alphanumeric characters.
pub fn new_id() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..16).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

impl Store {
    pub async fn open(database_url: &str, encryption_key: &str) -> anyhow::Result<Self> {
        if let Some(path) = database_url.strip_prefix("sqlite:") {
            let path = path.split('?').next().unwrap_or(path);
            if let Some(parent) = std::path::Path::new(path).parent() {
                if !parent.as_os_str().is_empty() && !path.contains(":memory:") {
                    std::fs::create_dir_all(parent)?;
                }
            }
        }
        let options = SqliteConnectOptions::from_str(database_url)?
            .create_if_missing(true)
            // WAL lets readers run alongside the writer; NORMAL sync is safe with WAL.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(10));
        let mut pool_options = SqlitePoolOptions::new();
        if database_url.contains(":memory:") {
            pool_options = pool_options.max_connections(1);
        }
        let writer = SqlitePoolOptions::new().max_connections(1).connect_with(options.clone()).await?;
        sqlx::migrate!("./migrations").run(&writer).await?;
        let pool = if database_url.contains(":memory:") { writer.clone() } else { pool_options.connect_with(options).await? };
        let batch = super::store_batch::spawn(writer.clone());
        Ok(Self { pool, writer, batch, encryption_key: encryption_key.to_string() })
    }

    pub fn encryption_key(&self) -> &str {
        &self.encryption_key
    }

    // ---- workflows -------------------------------------------------------

    /// Inserts or replaces a workflow, keyed by its `id` (one is assigned if
    /// missing). Returns the stored JSON.
    pub async fn save_workflow(&self, workflow: &Value) -> anyhow::Result<Value> {
        let mut workflow = workflow.clone();
        let obj = workflow.as_object_mut().ok_or_else(|| anyhow::anyhow!("a workflow must be a JSON object"))?;
        let id = match obj.get("id") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            _ => new_id(),
        };
        obj.insert("id".into(), json!(id));
        let name = obj.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let active = obj.get("active").and_then(Value::as_bool).unwrap_or(false);
        let version_id = obj
            .get("versionId")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let now = now();
        let existing = self.get_workflow(&id).await?;
        let created_at = existing
            .as_ref()
            .and_then(|w| w.get("createdAt").and_then(Value::as_str).map(String::from))
            .unwrap_or_else(|| now.clone());
        sqlx::query(
            "INSERT INTO workflow_entity (id, name, active, version_id, data, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, active = excluded.active,
               version_id = excluded.version_id, data = excluded.data, updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(&name)
        .bind(active as i64)
        .bind(&version_id)
        .bind(serde_json::to_string(&workflow)?)
        .bind(&created_at)
        .bind(&now)
        .execute(&self.writer)
        .await?;
        Ok(workflow)
    }

    pub async fn get_workflow(&self, id: &str) -> anyhow::Result<Option<Value>> {
        let row = sqlx::query("SELECT data FROM workflow_entity WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        Ok(match row {
            Some(r) => Some(serde_json::from_str(&r.get::<String, _>("data"))?),
            None => None,
        })
    }

    pub async fn list_workflows(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query("SELECT data FROM workflow_entity ORDER BY created_at, id").fetch_all(&self.pool).await?;
        rows.iter().map(|r| Ok(serde_json::from_str(&r.get::<String, _>("data"))?)).collect()
    }

    // ---- credentials -----------------------------------------------------

    /// Stores a credential. `data` may be an object (encrypted here) or an
    /// already-encrypted n8n blob (stored as-is).
    pub async fn save_credential(&self, id: Option<&str>, name: &str, cred_type: &str, data: &Value) -> anyhow::Result<String> {
        let id = id.map(String::from).unwrap_or_else(new_id);
        let blob = match data {
            Value::String(s) => s.clone(),
            other => super::cipher::encrypt_json(&self.encryption_key, other),
        };
        let now = now();
        sqlx::query(
            "INSERT INTO credentials_entity (id, name, type, data, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, type = excluded.type, data = excluded.data, updated_at = excluded.updated_at",
        )
        .bind(&id)
        .bind(name)
        .bind(cred_type)
        .bind(&blob)
        .bind(&now)
        .bind(&now)
        .execute(&self.writer)
        .await?;
        Ok(id)
    }

    pub async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialRecord>> {
        let rows = sqlx::query("SELECT id, name, type, data, created_at, updated_at FROM credentials_entity ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_credential).collect())
    }

    pub async fn get_credential(&self, id: &str) -> anyhow::Result<Option<CredentialRecord>> {
        let row = sqlx::query("SELECT id, name, type, data, created_at, updated_at FROM credentials_entity WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(row_to_credential))
    }

    pub async fn decrypt_credential(&self, record: &CredentialRecord) -> anyhow::Result<Value> {
        super::cipher::decrypt_json(&self.encryption_key, &record.data)
    }

    /// Replaces a credential's decrypted data (e.g. a refreshed OAuth2 token).
    pub async fn update_credential_data(&self, id: &str, data: &Value) -> anyhow::Result<()> {
        let blob = super::cipher::encrypt_json(&self.encryption_key, data);
        sqlx::query("UPDATE credentials_entity SET data = ?, updated_at = ? WHERE id = ?")
            .bind(blob)
            .bind(now())
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    // ---- executions ------------------------------------------------------

    pub async fn create_execution(&self, workflow: &Value, mode: &str) -> anyhow::Result<String> {
        let result = sqlx::query(
            "INSERT INTO execution_entity (workflow_id, mode, status, finished, started_at, workflow_data) VALUES (?, ?, 'running', 0, ?, ?)",
        )
        .bind(workflow.get("id").and_then(Value::as_str))
        .bind(mode)
        .bind(now())
        .bind(serde_json::to_string(workflow)?)
        .execute(&self.writer)
        .await?;
        Ok(result.last_insert_rowid().to_string())
    }

    pub async fn finish_execution(&self, id: &str, status: &str, data: &Value, wait_till: Option<&str>) -> anyhow::Result<()> {
        sqlx::query("UPDATE execution_entity SET status = ?, finished = ?, stopped_at = ?, wait_till = ?, data = ? WHERE id = ?")
            .bind(status)
            .bind((status == super::types::status::SUCCESS) as i64)
            .bind(now())
            .bind(wait_till)
            .bind(serde_json::to_string(data)?)
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }
}

fn row_to_credential(r: &sqlx::sqlite::SqliteRow) -> CredentialRecord {
    CredentialRecord {
        id: r.get("id"),
        name: r.get("name"),
        cred_type: r.get("type"),
        data: r.get("data"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}
