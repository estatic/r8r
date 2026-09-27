//! Writing records imported from n8n (`r8r migrate-from-n8n`): same ids and
//! timestamps as in n8n, and re-running an import updates rather than
//! duplicates.

use super::store::Store;
use serde_json::Value;
use sqlx::Row;

pub struct ImportedExecution {
    pub id: i64,
    pub workflow_id: Option<String>,
    pub mode: String,
    pub status: String,
    pub finished: bool,
    pub retry_of: Option<String>,
    pub started_at: String,
    pub stopped_at: Option<String>,
    pub wait_till: Option<String>,
    pub workflow_data: Value,
    pub data: Value,
    pub wait_state: Option<Value>,
}

impl Store {
    #[allow(clippy::too_many_arguments)]
    pub async fn import_user(
        &self,
        id: &str,
        email: &str,
        first: Option<&str>,
        last: Option<&str>,
        password: Option<&str>,
        role: &str,
        created: &str,
        updated: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(&self.sql(
            "INSERT INTO \"user\" (id, email, first_name, last_name, password, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET email = excluded.email, first_name = excluded.first_name, last_name = excluded.last_name,
               password = excluded.password, role = excluded.role, updated_at = excluded.updated_at",
        ))
        .bind(id)
        .bind(email.to_ascii_lowercase())
        .bind(first.unwrap_or(""))
        .bind(last.unwrap_or(""))
        .bind(password.unwrap_or(""))
        .bind(role)
        .bind(created)
        .bind(updated)
        .execute(&self.writer)
        .await?;
        // A pending invitation has no password yet.
        if password.is_none() {
            sqlx::query(&self.sql("UPDATE \"user\" SET password = NULL WHERE id = ?")).bind(id).execute(&self.writer).await?;
        }
        Ok(())
    }

    pub async fn import_api_key(&self, id: &str, user_id: &str, label: &str, hash: &str, scopes_json: &str, created: &str) -> anyhow::Result<()> {
        sqlx::query(&self.sql(
            "INSERT INTO user_api_keys (id, user_id, label, api_key_hash, scopes, created_at) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT DO NOTHING",
        ))
        .bind(id)
        .bind(user_id)
        .bind(label)
        .bind(hash)
        .bind(scopes_json)
        .bind(created)
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    pub async fn import_tag(&self, id: &str, name: &str, created: &str, updated: &str) -> anyhow::Result<()> {
        sqlx::query(&self.sql("INSERT INTO tag_entity (id, name, created_at, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING"))
            .bind(id)
            .bind(name)
            .bind(created)
            .bind(updated)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    pub async fn import_project(&self, id: &str, name: &str, kind: &str, created: &str, updated: &str) -> anyhow::Result<()> {
        sqlx::query(&self.sql(
            "INSERT INTO project (id, name, type, created_at, updated_at) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, updated_at = excluded.updated_at",
        ))
        .bind(id)
        .bind(name)
        .bind(kind)
        .bind(created)
        .bind(updated)
        .execute(&self.writer)
        .await?;
        Ok(())
    }

    /// Inserts or replaces a workflow with its n8n ownership and timestamps.
    #[allow(clippy::too_many_arguments)]
    pub async fn import_workflow(
        &self,
        data: &Value,
        active: bool,
        owner_id: Option<&str>,
        project_id: Option<&str>,
        created: &str,
        updated: &str,
    ) -> anyhow::Result<()> {
        let id = data["id"].as_str().ok_or_else(|| anyhow::anyhow!("a workflow without an id"))?;
        sqlx::query(&self.sql(
            "INSERT INTO workflow_entity (id, name, active, version_id, data, created_at, updated_at, owner_id, project_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, active = excluded.active, version_id = excluded.version_id, data = excluded.data,
               created_at = excluded.created_at, updated_at = excluded.updated_at, owner_id = excluded.owner_id, project_id = excluded.project_id",
        ))
        .bind(id)
        .bind(data["name"].as_str().unwrap_or(""))
        .bind(active as i64)
        .bind(data["versionId"].as_str().unwrap_or(""))
        .bind(serde_json::to_string(data)?)
        .bind(created)
        .bind(updated)
        .bind(owner_id.unwrap_or(""))
        .bind(project_id.unwrap_or(""))
        .execute(&self.writer)
        .await?;
        // Empty strings stand in for NULL above (see `Store::open_postgres`).
        sqlx::query(&self.sql("UPDATE workflow_entity SET owner_id = NULLIF(owner_id, ''), project_id = NULLIF(project_id, '') WHERE id = ?"))
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// Stores an n8n credential; `blob` stays encrypted exactly as n8n wrote it.
    #[allow(clippy::too_many_arguments)]
    pub async fn import_credential(&self, id: &str, name: &str, cred_type: &str, blob: &str, owner_id: Option<&str>, created: &str, updated: &str) -> anyhow::Result<()> {
        sqlx::query(&self.sql(
            "INSERT INTO credentials_entity (id, name, type, data, created_at, updated_at, owner_id) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, type = excluded.type, data = excluded.data,
               created_at = excluded.created_at, updated_at = excluded.updated_at, owner_id = excluded.owner_id",
        ))
        .bind(id)
        .bind(name)
        .bind(cred_type)
        .bind(blob)
        .bind(created)
        .bind(updated)
        .bind(owner_id.unwrap_or(""))
        .execute(&self.writer)
        .await?;
        sqlx::query(&self.sql("UPDATE credentials_entity SET owner_id = NULLIF(owner_id, '') WHERE id = ?")).bind(id).execute(&self.writer).await?;
        Ok(())
    }

    pub async fn import_variable(&self, id: &str, key: &str, value: &str, kind: &str) -> anyhow::Result<()> {
        sqlx::query(&self.sql("INSERT INTO variables (id, key, value, type) VALUES (?, ?, ?, ?) ON CONFLICT DO NOTHING"))
            .bind(id)
            .bind(key)
            .bind(value)
            .bind(kind)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// Inserts an execution under its n8n id; an id already present is kept.
    /// Returns whether it was inserted.
    pub async fn import_execution(&self, e: &ImportedExecution) -> anyhow::Result<bool> {
        let r = sqlx::query(&self.sql(
            "INSERT INTO execution_entity (id, workflow_id, mode, status, finished, retry_of, started_at, stopped_at, wait_till, workflow_data, data, wait_state)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING",
        ))
        .bind(e.id)
        .bind(e.workflow_id.clone().unwrap_or_default())
        .bind(&e.mode)
        .bind(&e.status)
        .bind(e.finished as i64)
        .bind(e.retry_of.clone().unwrap_or_default())
        .bind(&e.started_at)
        .bind(e.stopped_at.clone().unwrap_or_default())
        .bind(e.wait_till.clone().unwrap_or_default())
        .bind(serde_json::to_string(&e.workflow_data)?)
        .bind(serde_json::to_string(&e.data)?)
        .bind(e.wait_state.as_ref().map(|w| w.to_string()).unwrap_or_default())
        .execute(&self.writer)
        .await?;
        if r.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query(&self.sql(
            "UPDATE execution_entity SET workflow_id = NULLIF(workflow_id, ''), retry_of = NULLIF(retry_of, ''), stopped_at = NULLIF(stopped_at, ''),
               wait_till = NULLIF(wait_till, ''), wait_state = NULLIF(wait_state, '') WHERE id = ?",
        ))
        .bind(e.id)
        .execute(&self.writer)
        .await?;
        Ok(true)
    }

    /// After inserting explicit execution ids, lets new executions continue
    /// after the highest one (PostgreSQL sequences don't follow on their own;
    /// SQLite's AUTOINCREMENT does).
    pub async fn sync_execution_sequence(&self) -> anyhow::Result<()> {
        if self.postgres {
            sqlx::query("SELECT setval(pg_get_serial_sequence('execution_entity', 'id'), GREATEST((SELECT COALESCE(MAX(id), 0) FROM execution_entity), 1)) AS v")
                .fetch_all(&self.writer)
                .await?;
        }
        Ok(())
    }

    pub async fn count(&self, table: &str) -> anyhow::Result<i64> {
        let rows = sqlx::query(&format!("SELECT COUNT(*) AS n FROM \"{table}\"")).fetch_all(&self.pool).await?;
        Ok(rows.first().map(|r| r.get::<i64, _>("n")).unwrap_or(0))
    }
}
