//! Server-side persistence: users, API keys, tags, variables, projects,
//! workflow ownership and execution queries (spec §6.9, §6.10, §7.1).

use super::store::{new_id, now, Store};
use super::store_batch::{Bind, Op};
use serde_json::{json, Value};
use sqlx::Row;

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub email: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub password: Option<String>,
    pub role: String,
    pub created_at: String,
    pub updated_at: String,
}

impl User {
    pub fn is_admin(&self) -> bool {
        self.role == "global:owner" || self.role == "global:admin"
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "email": self.email,
            "firstName": self.first_name,
            "lastName": self.last_name,
            "role": self.role,
            "isPending": self.password.is_none(),
            "createdAt": self.created_at,
            "updatedAt": self.updated_at,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub label: String,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct WorkflowRow {
    pub data: Value,
    pub owner_id: Option<String>,
    pub project_id: Option<String>,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct ExecutionRow {
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
    pub data: Option<Value>,
    pub wait_state: Option<Value>,
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionFilter {
    pub workflow_id: Option<String>,
    pub statuses: Vec<String>,
    /// Only ids below this (cursor pagination, newest first).
    pub before_id: Option<i64>,
    pub limit: i64,
    pub include_running: bool,
}

fn user_from(r: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: r.get("id"),
        email: r.get("email"),
        first_name: r.get("first_name"),
        last_name: r.get("last_name"),
        password: r.get("password"),
        role: r.get("role"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn execution_from(r: &sqlx::sqlite::SqliteRow) -> ExecutionRow {
    let parse = |s: Option<String>| s.and_then(|s| serde_json::from_str::<Value>(&s).ok());
    ExecutionRow {
        id: r.get("id"),
        workflow_id: r.get("workflow_id"),
        mode: r.get("mode"),
        status: r.get("status"),
        finished: r.get::<i64, _>("finished") != 0,
        retry_of: r.get("retry_of"),
        started_at: r.get("started_at"),
        stopped_at: r.get("stopped_at"),
        wait_till: r.get("wait_till"),
        workflow_data: parse(r.get("workflow_data")).unwrap_or(Value::Null),
        data: parse(r.get("data")),
        wait_state: parse(r.get("wait_state")),
    }
}

const EXEC_COLS: &str = "id, workflow_id, mode, status, finished, retry_of, started_at, stopped_at, wait_till, workflow_data, data, wait_state";

impl Store {
    // ---- users -------------------------------------------------------------

    pub async fn user_count(&self) -> anyhow::Result<i64> {
        Ok(sqlx::query("SELECT COUNT(*) AS n FROM user WHERE password IS NOT NULL").fetch_one(&self.pool).await?.get("n"))
    }

    pub async fn owner(&self) -> anyhow::Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM user WHERE role = 'global:owner' LIMIT 1").fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(user_from))
    }

    pub async fn create_user(&self, email: &str, first: Option<&str>, last: Option<&str>, password_hash: Option<&str>, role: &str) -> anyhow::Result<User> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = now();
        sqlx::query("INSERT INTO user (id, email, first_name, last_name, password, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(email.to_ascii_lowercase())
            .bind(first)
            .bind(last)
            .bind(password_hash)
            .bind(role)
            .bind(&now)
            .bind(&now)
            .execute(&self.writer)
            .await?;
        Ok(self.get_user(&id).await?.expect("just inserted"))
    }

    pub async fn get_user(&self, id: &str) -> anyhow::Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM user WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(user_from))
    }

    pub async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
        let row = sqlx::query("SELECT * FROM user WHERE email = ?").bind(email.to_ascii_lowercase()).fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(user_from))
    }

    pub async fn list_users(&self) -> anyhow::Result<Vec<User>> {
        let rows = sqlx::query("SELECT * FROM user ORDER BY created_at, id").fetch_all(&self.pool).await?;
        Ok(rows.iter().map(user_from).collect())
    }

    pub async fn complete_user(&self, id: &str, first: &str, last: &str, password_hash: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE user SET first_name = ?, last_name = ?, password = ?, updated_at = ? WHERE id = ?")
            .bind(first)
            .bind(last)
            .bind(password_hash)
            .bind(now())
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    pub async fn delete_user(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM user WHERE id = ?").bind(id).execute(&self.writer).await?;
        sqlx::query("DELETE FROM user_api_keys WHERE user_id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- API keys ----------------------------------------------------------

    pub async fn create_api_key(&self, user_id: &str, label: &str, hash: &str, scopes: &[String], expires_at: Option<i64>) -> anyhow::Result<ApiKey> {
        let id = new_id();
        sqlx::query("INSERT INTO user_api_keys (id, user_id, label, api_key_hash, scopes, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(user_id)
            .bind(label)
            .bind(hash)
            .bind(serde_json::to_string(scopes)?)
            .bind(now())
            .bind(expires_at)
            .execute(&self.writer)
            .await?;
        Ok(ApiKey { id, user_id: user_id.into(), label: label.into(), scopes: scopes.to_vec(), created_at: now(), expires_at })
    }

    fn api_key_from(r: &sqlx::sqlite::SqliteRow) -> ApiKey {
        ApiKey {
            id: r.get("id"),
            user_id: r.get("user_id"),
            label: r.get("label"),
            scopes: serde_json::from_str(&r.get::<String, _>("scopes")).unwrap_or_default(),
            created_at: r.get("created_at"),
            expires_at: r.get("expires_at"),
        }
    }

    pub async fn api_key_by_hash(&self, hash: &str) -> anyhow::Result<Option<ApiKey>> {
        let row = sqlx::query("SELECT * FROM user_api_keys WHERE api_key_hash = ?").bind(hash).fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(Self::api_key_from))
    }

    pub async fn api_keys_of(&self, user_id: &str) -> anyhow::Result<Vec<ApiKey>> {
        let rows = sqlx::query("SELECT * FROM user_api_keys WHERE user_id = ? ORDER BY created_at").bind(user_id).fetch_all(&self.pool).await?;
        Ok(rows.iter().map(Self::api_key_from).collect())
    }

    pub async fn delete_api_key(&self, user_id: &str, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM user_api_keys WHERE id = ? AND user_id = ?").bind(id).bind(user_id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- workflows ---------------------------------------------------------

    fn workflow_from(r: &sqlx::sqlite::SqliteRow) -> anyhow::Result<WorkflowRow> {
        Ok(WorkflowRow {
            data: serde_json::from_str(&r.get::<String, _>("data"))?,
            owner_id: r.get("owner_id"),
            project_id: r.get("project_id"),
            active: r.get::<i64, _>("active") != 0,
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
    }

    pub async fn workflow_row(&self, id: &str) -> anyhow::Result<Option<WorkflowRow>> {
        let row = sqlx::query("SELECT data, owner_id, project_id, active, created_at, updated_at FROM workflow_entity WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(Self::workflow_from).transpose()
    }

    pub async fn workflow_rows(&self) -> anyhow::Result<Vec<WorkflowRow>> {
        let rows = sqlx::query("SELECT data, owner_id, project_id, active, created_at, updated_at FROM workflow_entity ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(Self::workflow_from).collect()
    }

    pub async fn set_workflow_owner(&self, id: &str, owner_id: Option<&str>, project_id: Option<&str>) -> anyhow::Result<()> {
        sqlx::query("UPDATE workflow_entity SET owner_id = COALESCE(?, owner_id), project_id = ? WHERE id = ?")
            .bind(owner_id)
            .bind(project_id)
            .bind(id)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    pub async fn delete_workflow(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM workflow_entity WHERE id = ?").bind(id).execute(&self.writer).await?;
        sqlx::query("DELETE FROM workflows_tags WHERE workflow_id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- tags --------------------------------------------------------------

    pub async fn create_tag(&self, name: &str) -> anyhow::Result<Option<Value>> {
        let id = new_id();
        let now = now();
        let r = sqlx::query("INSERT OR IGNORE INTO tag_entity (id, name, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id)
            .bind(name)
            .bind(&now)
            .bind(&now)
            .execute(&self.writer)
            .await?;
        if r.rows_affected() == 0 {
            return Ok(None);
        }
        Ok(Some(json!({"id": id, "name": name, "createdAt": now, "updatedAt": now})))
    }

    fn tag_json(r: &sqlx::sqlite::SqliteRow) -> Value {
        json!({"id": r.get::<String, _>("id"), "name": r.get::<String, _>("name"), "createdAt": r.get::<String, _>("created_at"), "updatedAt": r.get::<String, _>("updated_at")})
    }

    pub async fn list_tags(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query("SELECT * FROM tag_entity ORDER BY created_at, id").fetch_all(&self.pool).await?;
        Ok(rows.iter().map(Self::tag_json).collect())
    }

    pub async fn get_tag(&self, id: &str) -> anyhow::Result<Option<Value>> {
        let row = sqlx::query("SELECT * FROM tag_entity WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(Self::tag_json))
    }

    pub async fn rename_tag(&self, id: &str, name: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("UPDATE tag_entity SET name = ?, updated_at = ? WHERE id = ?").bind(name).bind(now()).bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_tag(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM tag_entity WHERE id = ?").bind(id).execute(&self.writer).await?;
        sqlx::query("DELETE FROM workflows_tags WHERE tag_id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn set_workflow_tags(&self, workflow_id: &str, tag_ids: &[String]) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM workflows_tags WHERE workflow_id = ?").bind(workflow_id).execute(&self.writer).await?;
        for t in tag_ids {
            sqlx::query("INSERT OR IGNORE INTO workflows_tags (workflow_id, tag_id) VALUES (?, ?)").bind(workflow_id).bind(t).execute(&self.writer).await?;
        }
        Ok(())
    }

    pub async fn workflow_tags(&self, workflow_id: &str) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query("SELECT t.* FROM tag_entity t JOIN workflows_tags wt ON wt.tag_id = t.id WHERE wt.workflow_id = ? ORDER BY t.name")
            .bind(workflow_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(Self::tag_json).collect())
    }

    // ---- variables ---------------------------------------------------------

    pub async fn create_variable(&self, key: &str, value: &str) -> anyhow::Result<Option<Value>> {
        let id = new_id();
        let r = sqlx::query("INSERT OR IGNORE INTO variables (id, key, value, type) VALUES (?, ?, ?, 'string')")
            .bind(&id)
            .bind(key)
            .bind(value)
            .execute(&self.writer)
            .await?;
        Ok((r.rows_affected() > 0).then(|| json!({"id": id, "key": key, "value": value, "type": "string"})))
    }

    pub async fn list_variables(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query("SELECT * FROM variables ORDER BY key").fetch_all(&self.pool).await?;
        Ok(rows
            .iter()
            .map(|r| json!({"id": r.get::<String, _>("id"), "key": r.get::<String, _>("key"), "value": r.get::<String, _>("value"), "type": r.get::<String, _>("type")}))
            .collect())
    }

    pub async fn update_variable(&self, id: &str, key: &str, value: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("UPDATE variables SET key = ?, value = ? WHERE id = ?").bind(key).bind(value).bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_variable(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM variables WHERE id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- projects ----------------------------------------------------------

    pub async fn create_project(&self, name: &str, kind: &str) -> anyhow::Result<Value> {
        let id = new_id();
        let now = now();
        sqlx::query("INSERT INTO project (id, name, type, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&id)
            .bind(name)
            .bind(kind)
            .bind(&now)
            .bind(&now)
            .execute(&self.writer)
            .await?;
        Ok(json!({"id": id, "name": name, "type": kind}))
    }

    pub async fn list_projects(&self) -> anyhow::Result<Vec<Value>> {
        let rows = sqlx::query("SELECT * FROM project ORDER BY created_at").fetch_all(&self.pool).await?;
        Ok(rows.iter().map(|r| json!({"id": r.get::<String, _>("id"), "name": r.get::<String, _>("name"), "type": r.get::<String, _>("type")})).collect())
    }

    pub async fn project_exists(&self, id: &str) -> anyhow::Result<bool> {
        Ok(sqlx::query("SELECT id FROM project WHERE id = ?").bind(id).fetch_optional(&self.pool).await?.is_some())
    }

    pub async fn update_project(&self, id: &str, name: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("UPDATE project SET name = ?, updated_at = ? WHERE id = ?").bind(name).bind(now()).bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn delete_project(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM project WHERE id = ?").bind(id).execute(&self.writer).await?;
        sqlx::query("DELETE FROM project_relation WHERE project_id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn add_project_relation(&self, project_id: &str, user_id: &str, role: &str) -> anyhow::Result<()> {
        sqlx::query("INSERT OR REPLACE INTO project_relation (project_id, user_id, role) VALUES (?, ?, ?)")
            .bind(project_id)
            .bind(user_id)
            .bind(role)
            .execute(&self.writer)
            .await?;
        Ok(())
    }

    /// The user's role in a project, if any.
    pub async fn project_role(&self, project_id: &str, user_id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT role FROM project_relation WHERE project_id = ? AND user_id = ?")
            .bind(project_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("role")))
    }

    // ---- credentials -------------------------------------------------------

    pub async fn set_credential_owner(&self, id: &str, owner_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE credentials_entity SET owner_id = ? WHERE id = ?").bind(owner_id).bind(id).execute(&self.writer).await?;
        Ok(())
    }

    pub async fn credential_owner(&self, id: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query("SELECT owner_id FROM credentials_entity WHERE id = ?").bind(id).fetch_optional(&self.pool).await?;
        Ok(row.and_then(|r| r.get("owner_id")))
    }

    pub async fn delete_credential(&self, id: &str) -> anyhow::Result<bool> {
        let r = sqlx::query("DELETE FROM credentials_entity WHERE id = ?").bind(id).execute(&self.writer).await?;
        Ok(r.rows_affected() > 0)
    }

    // ---- executions --------------------------------------------------------

    async fn batched(&self, sql: &'static str, binds: Vec<Bind>) -> anyhow::Result<(u64, i64)> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.batch.send(Op { sql, binds, reply }).map_err(|_| anyhow::anyhow!("the database writer has stopped"))?;
        rx.await.map_err(|_| anyhow::anyhow!("the database writer has stopped"))?.map_err(|e| anyhow::anyhow!(e))
    }

    pub async fn insert_execution(&self, workflow: &Value, mode: &str, retry_of: Option<&str>, parent: Option<&str>) -> anyhow::Result<i64> {
        let (_, id) = self
            .batched(
                "INSERT INTO execution_entity (workflow_id, mode, status, finished, retry_of, started_at, workflow_data, parent_execution_id)
                 VALUES (?, ?, 'running', 0, ?, ?, ?, ?)",
                vec![
                    Bind::Text(workflow.get("id").and_then(Value::as_str).map(String::from)),
                    Bind::Text(Some(mode.to_string())),
                    Bind::Text(retry_of.map(String::from)),
                    Bind::Text(Some(now())),
                    Bind::Text(Some(serde_json::to_string(workflow)?)),
                    Bind::Text(parent.map(String::from)),
                ],
            )
            .await?;
        Ok(id)
    }

    /// Records the end (or pause) of an execution.
    #[allow(clippy::too_many_arguments)]
    pub async fn save_execution_result(
        &self,
        id: i64,
        status: &str,
        started_at: &str,
        stopped_at: Option<&str>,
        data: &Value,
        wait_till: Option<&str>,
        wait_state: Option<&Value>,
    ) -> anyhow::Result<()> {
        self.batched(
            "UPDATE execution_entity SET status = ?, finished = ?, started_at = ?, stopped_at = ?, wait_till = ?, data = ?, wait_state = ? WHERE id = ?",
            vec![
                Bind::Text(Some(status.to_string())),
                Bind::Int(Some((status == super::types::status::SUCCESS) as i64)),
                Bind::Text(Some(started_at.to_string())),
                Bind::Text(stopped_at.map(String::from)),
                Bind::Text(wait_till.map(String::from)),
                Bind::Text(Some(serde_json::to_string(data)?)),
                Bind::Text(wait_state.map(|w| w.to_string())),
                Bind::Int(Some(id)),
            ],
        )
        .await?;
        Ok(())
    }

    /// Moves an execution from `from` to `to` atomically; false when it was
    /// not in `from` (e.g. already resumed).
    pub async fn transition_execution(&self, id: i64, from: &str, to: &str) -> anyhow::Result<bool> {
        let (n, _) = self
            .batched(
                "UPDATE execution_entity SET status = ? WHERE id = ? AND status = ?",
                vec![Bind::Text(Some(to.to_string())), Bind::Int(Some(id)), Bind::Text(Some(from.to_string()))],
            )
            .await?;
        Ok(n > 0)
    }

    pub async fn get_execution(&self, id: i64) -> anyhow::Result<Option<ExecutionRow>> {
        let row = sqlx::query(&format!("SELECT {EXEC_COLS} FROM execution_entity WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.as_ref().map(execution_from))
    }

    pub async fn list_executions(&self, f: &ExecutionFilter) -> anyhow::Result<Vec<ExecutionRow>> {
        let mut sql = format!("SELECT {EXEC_COLS} FROM execution_entity WHERE 1 = 1");
        let mut binds: Vec<String> = Vec::new();
        if let Some(w) = &f.workflow_id {
            sql.push_str(" AND workflow_id = ?");
            binds.push(w.clone());
        }
        if !f.statuses.is_empty() {
            sql.push_str(&format!(" AND status IN ({})", vec!["?"; f.statuses.len()].join(", ")));
            binds.extend(f.statuses.iter().cloned());
        } else if !f.include_running {
            sql.push_str(" AND status NOT IN ('running', 'new')");
        }
        if let Some(b) = f.before_id {
            sql.push_str(&format!(" AND id < {b}"));
        }
        sql.push_str(&format!(" ORDER BY id DESC LIMIT {}", if f.limit > 0 { f.limit } else { 100 }));
        let mut q = sqlx::query(&sql);
        for b in &binds {
            q = q.bind(b);
        }
        Ok(q.fetch_all(&self.pool).await?.iter().map(execution_from).collect())
    }

    pub async fn delete_execution(&self, id: i64) -> anyhow::Result<bool> {
        let (n, _) = self.batched("DELETE FROM execution_entity WHERE id = ?", vec![Bind::Int(Some(id))]).await?;
        Ok(n > 0)
    }

    /// Marks executions left running by a previous process as crashed.
    pub async fn mark_crashed(&self) -> anyhow::Result<u64> {
        let r = sqlx::query("UPDATE execution_entity SET status = 'crashed', stopped_at = ? WHERE status IN ('running', 'new')")
            .bind(now())
            .execute(&self.writer)
            .await?;
        Ok(r.rows_affected())
    }

    /// Waiting executions whose time has come.
    pub async fn due_waiting(&self, now_iso: &str) -> anyhow::Result<Vec<i64>> {
        let rows = sqlx::query("SELECT id FROM execution_entity WHERE status = 'waiting' AND wait_till IS NOT NULL AND wait_till <= ?")
            .bind(now_iso)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|r| r.get("id")).collect())
    }

    pub async fn prune_executions(&self, older_than_iso: &str) -> anyhow::Result<u64> {
        let r = sqlx::query("DELETE FROM execution_entity WHERE stopped_at IS NOT NULL AND stopped_at < ? AND status NOT IN ('running', 'waiting', 'new')")
            .bind(older_than_iso)
            .execute(&self.writer)
            .await?;
        Ok(r.rows_affected())
    }

    pub async fn active_workflow_count(&self) -> anyhow::Result<i64> {
        Ok(sqlx::query("SELECT COUNT(*) AS n FROM workflow_entity WHERE active = 1").fetch_one(&self.pool).await?.get("n"))
    }
}
