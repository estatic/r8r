//! Persistence for n8n entities: workflows (full JSON), credentials
//! (n8n-encrypted) and executions.

use serde_json::{json, Value};
use sqlx::any::{AnyPoolOptions, AnyRow};
use sqlx::AnyPool;
use sqlx::{Executor, Row};
use std::borrow::Cow;

#[derive(Clone)]
pub struct Store {
    pub(super) pool: AnyPool,
    /// All writes go through one connection: SQLite has a single writer, and
    /// queueing here is much faster than connections retrying on SQLITE_BUSY.
    /// (On PostgreSQL it is a small pool of its own.)
    pub(super) writer: AnyPool,
    /// PostgreSQL rather than SQLite (placeholders are `$n`).
    pub(super) postgres: bool,
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
    /// Opens (and migrates) the database: SQLite (`sqlite:...`) or
    /// PostgreSQL (`postgres://...`, tables in `DB_POSTGRESDB_SCHEMA`).
    pub async fn open(database_url: &str, encryption_key: &str) -> anyhow::Result<Self> {
        static DRIVERS: std::sync::Once = std::sync::Once::new();
        DRIVERS.call_once(sqlx::any::install_default_drivers);
        let postgres = database_url.starts_with("postgres:") || database_url.starts_with("postgresql:");
        let (pool, writer) = if postgres { Self::open_postgres(database_url).await? } else { Self::open_sqlite(database_url).await? };
        let batch = super::store_batch::spawn(writer.clone(), postgres);
        Ok(Self { pool, writer, postgres, batch, encryption_key: encryption_key.to_string() })
    }

    async fn open_sqlite(database_url: &str) -> anyhow::Result<(AnyPool, AnyPool)> {
        let path = database_url.trim_start_matches("sqlite://").trim_start_matches("sqlite:");
        let file = path.split('?').next().unwrap_or(path);
        let memory = file.contains(":memory:");
        if let Some(parent) = std::path::Path::new(file).parent() {
            if !parent.as_os_str().is_empty() && !memory {
                std::fs::create_dir_all(parent)?;
            }
        }
        let url = if memory || path.contains("mode=") { database_url.to_string() } else { format!("sqlite:{file}?mode=rwc") };
        let options = || {
            AnyPoolOptions::new().after_connect(|conn, _| {
                Box::pin(async move {
                    // WAL lets readers run alongside the writer; NORMAL sync is safe with WAL.
                    conn.execute("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 10000;").await?;
                    Ok(())
                })
            })
        };
        let writer = options().max_connections(1).connect(&url).await?;
        sqlx::migrate!("./migrations").run(&writer).await?;
        let pool = if memory { writer.clone() } else { options().connect(&url).await? };
        Ok((pool, writer))
    }

    async fn open_postgres(database_url: &str) -> anyhow::Result<(AnyPool, AnyPool)> {
        // sqlx's `Any` driver sends every NULL as an INT4 parameter, and a
        // cached statement keeps the parameter types of its first call, so a
        // later call with a string in that slot fails. Uncached statements
        // take their types from each call's arguments.
        let url = if database_url.contains("statement-cache-capacity") {
            database_url.to_string()
        } else {
            format!("{database_url}{}statement-cache-capacity=0", if database_url.contains('?') { '&' } else { '?' })
        };
        let database_url = url.as_str();
        let schema = std::env::var("DB_POSTGRESDB_SCHEMA").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "public".into());
        if !schema.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            anyhow::bail!("invalid configuration: DB_POSTGRESDB_SCHEMA: only letters, digits and _ are allowed");
        }
        let connect = |max: u32| {
            let schema = schema.clone();
            AnyPoolOptions::new()
                .max_connections(max)
                .acquire_timeout(std::time::Duration::from_secs(10))
                .after_connect(move |conn, _| {
                    let set = format!("SET search_path TO \"{schema}\"");
                    Box::pin(async move {
                        conn.execute(set.as_str()).await?;
                        Ok(())
                    })
                })
                .connect(database_url)
        };
        let bootstrap = AnyPoolOptions::new().max_connections(1).connect(database_url).await.map_err(|e| {
            anyhow::anyhow!("cannot connect to PostgreSQL at {}: {e}", crate::logging::redact_url(database_url))
        })?;
        bootstrap.execute(format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"").as_str()).await?;
        bootstrap.close().await;
        let writer = connect(4).await?;
        sqlx::migrate!("./migrations_postgres").run(&writer).await?;
        let pool = connect(10).await?;
        Ok((pool, writer))
    }

    /// The SQL for this database: `?` placeholders become `$1`, `$2`, ...
    /// on PostgreSQL. (No statement here has a `?` inside a string.)
    pub(super) fn sql<'a>(&self, sql: &'a str) -> Cow<'a, str> {
        if !self.postgres {
            return Cow::Borrowed(sql);
        }
        let mut out = String::with_capacity(sql.len() + 8);
        let mut n = 0;
        for c in sql.chars() {
            if c == '?' {
                n += 1;
                out.push_str(&format!("${n}"));
            } else {
                out.push(c);
            }
        }
        Cow::Owned(out)
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
        sqlx::query(&self.sql("INSERT INTO workflow_entity (id, name, active, version_id, data, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, active = excluded.active,
               version_id = excluded.version_id, data = excluded.data, updated_at = excluded.updated_at"))
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
        let row = sqlx::query(&self.sql("SELECT data FROM workflow_entity WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?;
        Ok(match row {
            Some(r) => Some(serde_json::from_str(&r.get::<String, _>("data"))?),
            None => None,
        })
    }

    pub async fn list_workflows(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query(&self.sql("SELECT data FROM workflow_entity ORDER BY created_at, id")).fetch_all(&self.pool).await?;
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
        sqlx::query(&self.sql("INSERT INTO credentials_entity (id, name, type, data, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, type = excluded.type, data = excluded.data, updated_at = excluded.updated_at"))
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
        let rows = sqlx::query(&self.sql("SELECT id, name, type, data, created_at, updated_at FROM credentials_entity ORDER BY created_at, id"))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(row_to_credential).collect())
    }

    pub async fn get_credential(&self, id: &str) -> anyhow::Result<Option<CredentialRecord>> {
        let row = sqlx::query(&self.sql("SELECT id, name, type, data, created_at, updated_at FROM credentials_entity WHERE id = ?"))
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
        sqlx::query(&self.sql("UPDATE credentials_entity SET data = ?, updated_at = ? WHERE id = ?"))
            .bind(blob)
            .bind(now())
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    // ---- executions ------------------------------------------------------

    pub async fn create_execution(&self, workflow: &Value, mode: &str) -> anyhow::Result<String> {
        let mut conn = self.writer.acquire().await?;
        sqlx::query(&self.sql("INSERT INTO execution_entity (workflow_id, mode, status, finished, started_at, workflow_data) VALUES (?, ?, 'running', 0, ?, ?)"))
        .bind(workflow.get("id").and_then(Value::as_str))
        .bind(mode)
        .bind(now())
        .bind(serde_json::to_string(workflow)?)
        .execute(&mut *conn)
        .await?;
        let id = super::store_batch::new_id(&mut conn, self.postgres).await?;
        Ok(id.to_string())
    }

    pub async fn finish_execution(&self, id: &str, status: &str, data: &Value, wait_till: Option<&str>) -> anyhow::Result<()> {
        sqlx::query(&self.sql("UPDATE execution_entity SET status = ?, finished = ?, stopped_at = ?, wait_till = ?, data = ? WHERE id = ?"))
            .bind(status)
            .bind((status == super::types::status::SUCCESS) as i64)
            .bind(now())
            .bind(wait_till)
            .bind(serde_json::to_string(data)?)
            .bind(id.parse::<i64>()?)
            .execute(&self.writer)
            .await?;
        Ok(())
    }
}

fn row_to_credential(r: &AnyRow) -> CredentialRecord {
    CredentialRecord {
        id: r.get("id"),
        name: r.get("name"),
        cred_type: r.get("type"),
        data: r.get("data"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}
