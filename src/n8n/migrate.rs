//! `r8r migrate-from-n8n` (spec goal G3): imports an n8n 1.x/2.x database,
//! SQLite or PostgreSQL, into r8r's store. The n8n database is only read,
//! so n8n keeps working on it and a rollback is simply using n8n again.
//!
//! What comes across, with n8n's ids: users (bcrypt passwords as they are),
//! public API keys, projects, workflows (active ones as published), tags,
//! credentials (still encrypted with the same `N8N_ENCRYPTION_KEY`),
//! variables and executions, including waiting ones, whose n8n resume URLs
//! keep working.

use super::cipher;
use super::config::Config;
use super::engine::WaitState;
use super::server::auth::{hash_api_key, ALL_SCOPES, MEMBER_SCOPES};
use super::store::Store;
use super::store_import::ImportedExecution;
use serde_json::{json, Map, Value};
use sqlx::any::AnyPoolOptions;
use sqlx::{AnyPool, Executor, Row};
use std::collections::HashMap;

pub struct Options {
    /// `sqlite:<path>`, a plain path, or a `postgres://` URL.
    pub db: String,
    /// PostgreSQL schema of the n8n tables (default `public`).
    pub schema: Option<String>,
    /// n8n's `DB_TABLE_PREFIX`.
    pub table_prefix: String,
    pub skip_executions: bool,
}

#[derive(Default, Debug)]
pub struct Report {
    pub users: usize,
    pub api_keys: usize,
    pub projects: usize,
    pub workflows: usize,
    pub active_workflows: usize,
    pub unpublished_drafts: usize,
    pub archived_skipped: usize,
    pub credentials: usize,
    pub tags: usize,
    pub variables: usize,
    pub executions: usize,
    pub waiting_executions: usize,
    pub executions_already_present: usize,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Imported from n8n:")?;
        writeln!(f, "  users:        {}", self.users)?;
        writeln!(f, "  API keys:     {}", self.api_keys)?;
        writeln!(f, "  projects:     {} (team)", self.projects)?;
        writeln!(f, "  workflows:    {} ({} active)", self.workflows, self.active_workflows)?;
        writeln!(f, "  credentials:  {}", self.credentials)?;
        writeln!(f, "  tags:         {}", self.tags)?;
        writeln!(f, "  variables:    {}", self.variables)?;
        writeln!(f, "  executions:   {} ({} waiting)", self.executions, self.waiting_executions)?;
        if self.executions_already_present > 0 {
            writeln!(f, "  executions already in r8r (kept): {}", self.executions_already_present)?;
        }
        if self.unpublished_drafts > 0 {
            writeln!(f, "Note: {} active workflow(s) had unpublished changes in n8n; their published version was imported.", self.unpublished_drafts)?;
        }
        if self.archived_skipped > 0 {
            writeln!(f, "Note: {} archived workflow(s) were not imported.", self.archived_skipped)?;
        }
        Ok(())
    }
}

type Record = HashMap<String, Option<String>>;

struct Source {
    pool: AnyPool,
    postgres: bool,
    prefix: String,
    /// Columns per table (lower-case table names without prefix).
    columns: HashMap<String, Vec<String>>,
}

impl Source {
    fn table(&self, name: &str) -> String {
        format!("\"{}{name}\"", self.prefix)
    }

    fn has(&self, table: &str, column: &str) -> bool {
        self.columns.get(table).is_some_and(|c| c.iter().any(|x| x == column))
    }

    fn has_table(&self, table: &str) -> bool {
        self.columns.contains_key(table)
    }

    /// Every wanted column that exists, read as text (the one type both
    /// databases and sqlx's `Any` driver agree on).
    async fn rows(&self, table: &str, wanted: &[&str], tail: &str) -> anyhow::Result<Vec<Record>> {
        let cols: Vec<&str> = wanted.iter().copied().filter(|c| self.has(table, c)).collect();
        if cols.is_empty() {
            return Ok(vec![]);
        }
        let select: Vec<String> = cols.iter().map(|c| format!("CAST(\"{c}\" AS TEXT) AS \"{c}\"")).collect();
        let sql = format!("SELECT {} FROM {} {tail}", select.join(", "), self.table(table));
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await.map_err(|e| anyhow::anyhow!("reading n8n's {table}: {e}"))?;
        Ok(rows
            .iter()
            .map(|r| cols.iter().map(|c| (c.to_string(), r.try_get_unchecked::<Option<String>, _>(*c).ok().flatten())).collect())
            .collect())
    }
}

fn text<'a>(r: &'a Record, k: &str) -> Option<&'a str> {
    r.get(k).and_then(|v| v.as_deref())
}

fn truthy(v: Option<&str>) -> bool {
    matches!(v, Some("1" | "true" | "t" | "TRUE"))
}

fn json_col(r: &Record, k: &str) -> Value {
    text(r, k).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

/// n8n timestamps (`2026-09-26 13:07:16.019` in SQLite, `...+00` from
/// PostgreSQL) as ISO-8601 UTC.
pub fn iso(ts: &str) -> String {
    let t = ts.trim();
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(t) {
        return d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    }
    let spaced = t.replacen('T', " ", 1);
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"] {
        if let Ok(d) = chrono::DateTime::parse_from_str(&spaced, fmt) {
            return d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
    }
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(d) = chrono::NaiveDateTime::parse_from_str(&spaced, fmt) {
            return d.and_utc().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
    }
    t.to_string()
}

/// Reverses n8n's "flatted" encoding of execution data: a JSON array whose
/// first entry is the root, where strings inside objects and arrays are
/// indices of other entries.
pub fn unflatten(flat: &Value) -> Value {
    let Some(arr) = flat.as_array() else { return flat.clone() };
    let Some(root) = arr.first() else { return Value::Null };
    fn resolve(arr: &[Value], v: &Value, depth: usize) -> Value {
        if depth > 10_000 {
            return Value::Null;
        }
        let deref = |x: &Value| match x {
            Value::String(idx) => match idx.parse::<usize>().ok().and_then(|i| arr.get(i)) {
                Some(t @ (Value::Object(_) | Value::Array(_))) => resolve(arr, t, depth + 1),
                Some(t) => t.clone(),
                None => Value::Null,
            },
            other => other.clone(),
        };
        match v {
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), deref(x))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(deref).collect()),
            other => other.clone(),
        }
    }
    match root {
        Value::Object(_) | Value::Array(_) => resolve(arr, root, 0),
        other => other.clone(),
    }
}

async fn connect(opts: &Options) -> anyhow::Result<Source> {
    static DRIVERS: std::sync::Once = std::sync::Once::new();
    DRIVERS.call_once(sqlx::any::install_default_drivers);
    let postgres = opts.db.starts_with("postgres://") || opts.db.starts_with("postgresql://");
    let pool = if postgres {
        let schema = opts.schema.clone().unwrap_or_else(|| "public".into());
        if !schema.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            anyhow::bail!("invalid --schema \"{schema}\": only letters, digits and _ are allowed");
        }
        AnyPoolOptions::new()
            .max_connections(2)
            .after_connect(move |conn, _| {
                let set = format!("SET search_path TO \"{schema}\"");
                Box::pin(async move {
                    conn.execute(set.as_str()).await?;
                    Ok(())
                })
            })
            .connect(&opts.db)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to the n8n database at {}: {e}", crate::logging::redact_url(&opts.db)))?
    } else {
        let path = sqlite_path(&opts.db);
        let bytes = std::fs::read(&path).map_err(|e| anyhow::anyhow!("cannot read the n8n database {path}: {e}"))?;
        if !bytes.starts_with(b"SQLite format 3\0") {
            anyhow::bail!("{path} is not an n8n SQLite database");
        }
        AnyPoolOptions::new().max_connections(1).connect(&format!("sqlite:{path}?mode=ro")).await?
    };
    let listing = if postgres {
        "SELECT CAST(table_name AS TEXT) AS t, CAST(column_name AS TEXT) AS c FROM information_schema.columns WHERE table_schema = current_schema()"
    } else {
        "SELECT m.name AS t, p.name AS c FROM sqlite_master m JOIN pragma_table_info(m.name) p WHERE m.type = 'table'"
    };
    let mut columns: HashMap<String, Vec<String>> = HashMap::new();
    for r in sqlx::query(listing).fetch_all(&pool).await? {
        let t: String = r.get("t");
        let Some(t) = t.strip_prefix(&opts.table_prefix) else { continue };
        columns.entry(t.to_string()).or_default().push(r.get("c"));
    }
    Ok(Source { pool, postgres, prefix: opts.table_prefix.clone(), columns })
}

fn sqlite_path(db: &str) -> String {
    let p = db.trim_start_matches("sqlite://").trim_start_matches("sqlite:");
    p.split('?').next().unwrap_or(p).to_string()
}

pub async fn run(config: &Config, opts: Options) -> anyhow::Result<Report> {
    let src = connect(&opts).await?;
    let location = if src.postgres { crate::logging::redact_url(&opts.db) } else { sqlite_path(&opts.db) };
    if !src.has("workflow_entity", "nodes") || !src.has_table("credentials_entity") {
        anyhow::bail!("{location} is not an n8n database (no n8n workflow_entity table{})", if opts.table_prefix.is_empty() { "" } else { " with that prefix" });
    }
    if !src.has_table("project") || !src.has_table("shared_workflow") {
        anyhow::bail!("{location} is from n8n before 1.0, which is not supported: upgrade n8n to 1.x or 2.x first");
    }
    // The target must be another database: r8r's tables have n8n's names.
    if !src.postgres && config.database_url.starts_with("sqlite") {
        let same = std::fs::canonicalize(sqlite_path(&opts.db)).ok() == std::fs::canonicalize(sqlite_path(&config.database_url)).ok();
        if same {
            anyhow::bail!("r8r's database is the n8n database itself ({location}); set DB_SQLITE_DATABASE to another file for r8r");
        }
    }
    // Credentials only work with the key they were encrypted with.
    let credentials = src.rows("credentials_entity", &["id", "name", "type", "data", "createdAt", "updatedAt"], "").await?;
    if let Some(first) = credentials.iter().find(|c| text(c, "data").is_some()) {
        if cipher::decrypt(&config.encryption_key, text(first, "data").unwrap()).is_err() {
            anyhow::bail!(
                "cannot decrypt n8n's credentials (e.g. \"{}\") with this N8N_ENCRYPTION_KEY: use the key of the n8n instance",
                text(first, "name").unwrap_or("?")
            );
        }
    }

    let store = Store::open(&config.database_url, &config.encryption_key).await?;
    let mut report = Report::default();

    // ---- users ----------------------------------------------------------
    let role_col = if src.has("user", "roleSlug") { "roleSlug" } else { "role" };
    let users = src.rows("user", &["id", "email", "firstName", "lastName", "password", role_col, "createdAt", "updatedAt"], "").await?;
    let mut roles: HashMap<String, String> = HashMap::new();
    for u in &users {
        let (Some(id), Some(email)) = (text(u, "id"), text(u, "email")) else { continue };
        let role = match text(u, role_col) {
            Some(r @ ("global:owner" | "global:admin" | "global:member")) => r,
            Some("owner") => "global:owner",
            _ => "global:member",
        };
        let created = iso(text(u, "createdAt").unwrap_or(""));
        let updated = iso(text(u, "updatedAt").unwrap_or(""));
        store.import_user(id, email, text(u, "firstName"), text(u, "lastName"), text(u, "password"), role, &created, &updated).await?;
        roles.insert(id.to_string(), role.to_string());
        report.users += 1;
    }

    // ---- API keys (the key is stored as issued; r8r keeps its hash) ------
    if src.has_table("user_api_keys") {
        for k in src.rows("user_api_keys", &["id", "userId", "label", "apiKey", "scopes", "createdAt", "audience"], "").await? {
            if text(&k, "audience").is_some_and(|a| a != "public-api") {
                continue;
            }
            let (Some(id), Some(user), Some(raw)) = (text(&k, "id"), text(&k, "userId"), text(&k, "apiKey")) else { continue };
            let scopes = match text(&k, "scopes").and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()) {
                Some(s) => s,
                None => {
                    let all = if roles.get(user).is_some_and(|r| r != "global:member") { ALL_SCOPES } else { MEMBER_SCOPES };
                    all.iter().map(|s| s.to_string()).collect()
                }
            };
            let created = iso(text(&k, "createdAt").unwrap_or(""));
            store.import_api_key(id, user, text(&k, "label").unwrap_or("n8n"), &hash_api_key(raw), &serde_json::to_string(&scopes)?, &created).await?;
            report.api_keys += 1;
        }
    }

    // ---- projects: personal ones map to their owner, team ones come along --
    let projects = src.rows("project", &["id", "name", "type", "createdAt", "updatedAt", "creatorId"], "").await?;
    let relations = src.rows("project_relation", &["projectId", "userId", "role"], "").await?;
    let mut personal_owner: HashMap<String, String> = HashMap::new();
    for r in &relations {
        if text(r, "role") == Some("project:personalOwner") {
            if let (Some(p), Some(u)) = (text(r, "projectId"), text(r, "userId")) {
                personal_owner.insert(p.to_string(), u.to_string());
            }
        }
    }
    let mut team: HashMap<String, Option<String>> = HashMap::new();
    for p in &projects {
        let Some(id) = text(p, "id") else { continue };
        if text(p, "type") == Some("team") {
            store
                .import_project(id, text(p, "name").unwrap_or(""), "team", &iso(text(p, "createdAt").unwrap_or("")), &iso(text(p, "updatedAt").unwrap_or("")))
                .await?;
            team.insert(id.to_string(), text(p, "creatorId").map(String::from));
            report.projects += 1;
        } else if !personal_owner.contains_key(id) {
            if let Some(creator) = text(p, "creatorId") {
                personal_owner.insert(id.to_string(), creator.to_string());
            }
        }
    }
    for r in &relations {
        if let (Some(p), Some(u), Some(role)) = (text(r, "projectId"), text(r, "userId"), text(r, "role")) {
            if team.contains_key(p) && role != "project:personalOwner" {
                store.add_project_relation(p, u, role).await?;
            }
        }
    }
    // Who owns a workflow or credential: its owning project's user, or the
    // team project itself.
    let ownership = |project: Option<&str>| -> (Option<String>, Option<String>) {
        match project {
            Some(p) if team.contains_key(p) => (team.get(p).cloned().flatten(), Some(p.to_string())),
            Some(p) => (personal_owner.get(p).cloned(), None),
            None => (None, None),
        }
    };

    // ---- tags --------------------------------------------------------------
    for t in src.rows("tag_entity", &["id", "name", "createdAt", "updatedAt"], "").await? {
        let (Some(id), Some(name)) = (text(&t, "id"), text(&t, "name")) else { continue };
        store.import_tag(id, name, &iso(text(&t, "createdAt").unwrap_or("")), &iso(text(&t, "updatedAt").unwrap_or(""))).await?;
        report.tags += 1;
    }
    let mut workflow_tags: HashMap<String, Vec<String>> = HashMap::new();
    for wt in src.rows("workflows_tags", &["workflowId", "tagId"], "").await? {
        if let (Some(w), Some(t)) = (text(&wt, "workflowId"), text(&wt, "tagId")) {
            workflow_tags.entry(w.to_string()).or_default().push(t.to_string());
        }
    }

    // ---- workflows -----------------------------------------------------------
    let owners: HashMap<String, String> = src
        .rows("shared_workflow", &["workflowId", "projectId", "role"], "")
        .await?
        .iter()
        .filter(|r| text(r, "role") == Some("workflow:owner"))
        .filter_map(|r| Some((text(r, "workflowId")?.to_string(), text(r, "projectId")?.to_string())))
        .collect();
    let history: HashMap<String, Record> = if src.has_table("workflow_history") {
        src.rows("workflow_history", &["versionId", "nodes", "connections"], "")
            .await?
            .into_iter()
            .filter_map(|r| Some((text(&r, "versionId")?.to_string(), r)))
            .collect()
    } else {
        HashMap::new()
    };
    let wf_cols = [
        "id", "name", "active", "nodes", "connections", "settings", "staticData", "pinData", "versionId", "meta", "createdAt", "updatedAt", "isArchived",
        "activeVersionId",
    ];
    for w in src.rows("workflow_entity", &wf_cols, "").await? {
        let Some(id) = text(&w, "id") else { continue };
        if truthy(text(&w, "isArchived")) {
            report.archived_skipped += 1;
            continue;
        }
        // n8n 2.x publishes a version; 1.x has only `active`.
        let active = if src.has("workflow_entity", "activeVersionId") { text(&w, "activeVersionId").is_some() } else { truthy(text(&w, "active")) };
        let mut nodes = json_col(&w, "nodes");
        let mut connections = json_col(&w, "connections");
        let mut version = text(&w, "versionId").map(String::from);
        if let Some(published) = text(&w, "activeVersionId").filter(|v| Some(*v) != text(&w, "versionId")) {
            if let Some(h) = history.get(published) {
                nodes = json_col(h, "nodes");
                connections = json_col(h, "connections");
                version = Some(published.to_string());
                report.unpublished_drafts += 1;
            }
        }
        let mut data = Map::new();
        data.insert("id".into(), json!(id));
        data.insert("name".into(), json!(text(&w, "name").unwrap_or("")));
        data.insert("active".into(), json!(active));
        data.insert("nodes".into(), if nodes.is_array() { nodes } else { json!([]) });
        data.insert("connections".into(), if connections.is_object() { connections } else { json!({}) });
        data.insert("settings".into(), match json_col(&w, "settings") {
            Value::Object(o) => Value::Object(o),
            _ => json!({}),
        });
        data.insert("staticData".into(), json_col(&w, "staticData"));
        data.insert("pinData".into(), match json_col(&w, "pinData") {
            Value::Object(o) => Value::Object(o),
            _ => json!({}),
        });
        data.insert("versionId".into(), json!(version.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())));
        if let Value::Object(meta) = json_col(&w, "meta") {
            data.insert("meta".into(), Value::Object(meta));
        }
        let (owner, project) = ownership(owners.get(id).map(String::as_str));
        let created = iso(text(&w, "createdAt").unwrap_or(""));
        let updated = iso(text(&w, "updatedAt").unwrap_or(""));
        store.import_workflow(&Value::Object(data), active, owner.as_deref(), project.as_deref(), &created, &updated).await?;
        store.set_workflow_tags(id, workflow_tags.get(id).map(Vec::as_slice).unwrap_or(&[])).await?;
        report.workflows += 1;
        if active {
            report.active_workflows += 1;
        }
    }

    // ---- credentials -----------------------------------------------------------
    let cred_owners: HashMap<String, String> = src
        .rows("shared_credentials", &["credentialsId", "projectId", "role"], "")
        .await?
        .iter()
        .filter(|r| text(r, "role") == Some("credential:owner"))
        .filter_map(|r| Some((text(r, "credentialsId")?.to_string(), text(r, "projectId")?.to_string())))
        .collect();
    for c in &credentials {
        let (Some(id), Some(blob)) = (text(c, "id"), text(c, "data")) else { continue };
        let (owner, _) = ownership(cred_owners.get(id).map(String::as_str));
        let created = iso(text(c, "createdAt").unwrap_or(""));
        let updated = iso(text(c, "updatedAt").unwrap_or(""));
        store.import_credential(id, text(c, "name").unwrap_or(""), text(c, "type").unwrap_or(""), blob, owner.as_deref(), &created, &updated).await?;
        report.credentials += 1;
    }

    // ---- variables ---------------------------------------------------------------
    if src.has_table("variables") {
        for v in src.rows("variables", &["id", "key", "value", "type"], "").await? {
            let (Some(id), Some(key)) = (text(&v, "id"), text(&v, "key")) else { continue };
            store.import_variable(id, key, text(&v, "value").unwrap_or(""), text(&v, "type").unwrap_or("string")).await?;
            report.variables += 1;
        }
    }

    // ---- executions (in pages, oldest first) ----------------------------------------
    if !opts.skip_executions && src.has_table("execution_data") {
        import_executions(&src, &store, &mut report).await?;
        store.sync_execution_sequence().await?;
    }
    Ok(report)
}

async fn import_executions(src: &Source, store: &Store, report: &mut Report) -> anyhow::Result<()> {
    let cols = ["id", "workflowId", "finished", "mode", "retryOf", "startedAt", "stoppedAt", "waitTill", "status", "deletedAt", "createdAt"];
    let mut last: i64 = 0;
    loop {
        let page = src.rows("execution_entity", &cols, &format!("WHERE \"id\" > {last} ORDER BY \"id\" LIMIT 200")).await?;
        if page.is_empty() {
            break;
        }
        let ids: Vec<i64> = page.iter().filter_map(|r| text(r, "id")?.parse().ok()).collect();
        let (lo, hi) = (ids.iter().min().copied().unwrap_or(0), ids.iter().max().copied().unwrap_or(0));
        let data: HashMap<i64, Record> = src
            .rows("execution_data", &["executionId", "workflowData", "data"], &format!("WHERE \"executionId\" BETWEEN {lo} AND {hi}"))
            .await?
            .into_iter()
            .filter_map(|r| Some((text(&r, "executionId")?.parse().ok()?, r)))
            .collect();
        for e in &page {
            let Some(id) = text(e, "id").and_then(|s| s.parse::<i64>().ok()) else { continue };
            last = last.max(id);
            if text(e, "deletedAt").is_some() {
                continue;
            }
            let d = data.get(&id);
            let run = d.and_then(|d| text(d, "data")).and_then(|s| serde_json::from_str::<Value>(s).ok()).map(|v| unflatten(&v)).unwrap_or(json!({}));
            let workflow_data = d.map(|d| json_col(d, "workflowData")).unwrap_or(Value::Null);
            let status = text(e, "status").unwrap_or(if truthy(text(e, "finished")) { "success" } else { "error" }).to_string();
            let wait_till = text(e, "waitTill").map(iso);
            let wait_state = if status == "waiting" { wait_state_of(&run, wait_till.as_deref()) } else { None };
            if wait_state.is_some() {
                report.waiting_executions += 1;
            }
            let started = text(e, "startedAt").or(text(e, "createdAt")).map(iso).unwrap_or_default();
            let imported = store
                .import_execution(&ImportedExecution {
                    id,
                    workflow_id: text(e, "workflowId").map(String::from),
                    mode: text(e, "mode").unwrap_or("manual").to_string(),
                    finished: truthy(text(e, "finished")),
                    status,
                    retry_of: text(e, "retryOf").map(String::from),
                    started_at: started,
                    stopped_at: text(e, "stoppedAt").map(iso),
                    wait_till,
                    workflow_data,
                    data: run,
                    wait_state,
                })
                .await?;
            if imported {
                report.executions += 1;
            } else {
                report.executions_already_present += 1;
            }
        }
    }
    Ok(())
}

/// Where a waiting n8n execution continues: the node on top of n8n's
/// execution stack, with its input and source.
fn wait_state_of(run: &Value, wait_till: Option<&str>) -> Option<Value> {
    let entry = run.pointer("/executionData/nodeExecutionStack/0")?;
    let node = entry.pointer("/node/name")?.as_str()?.to_string();
    let input = entry.pointer("/data/main/0").cloned().unwrap_or(json!([]));
    let source = entry.pointer("/source/main").cloned().unwrap_or(json!([]));
    // Webhook/form waits have no deadline (n8n stores the year 3000).
    let till_ms = wait_till
        .filter(|t| !t.starts_with("3000-"))
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|d| d.timestamp_millis());
    let state = json!({"node": node, "till_ms": till_ms, "input": input, "source": source});
    // Only a shape r8r can resume from.
    serde_json::from_value::<WaitState>(state.clone()).ok().map(|_| state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flatted_data_is_restored() {
        // Every nested object or array is its own entry, referenced by index.
        let flat = json!([{"a": "1", "b": "2", "n": 5, "t": true}, "text", ["3", "4"], {"x": "5"}, [], "deep"]);
        assert_eq!(unflatten(&flat), json!({"a": "text", "b": [{"x": "deep"}, []], "n": 5, "t": true}));
    }

    #[test]
    fn n8n_timestamps_become_iso() {
        assert_eq!(iso("2026-09-26 13:07:16.019"), "2026-09-26T13:07:16.019Z");
        assert_eq!(iso("2026-09-26 13:07:16.019+00"), "2026-09-26T13:07:16.019Z");
        assert_eq!(iso("2026-09-26 15:07:16+02:00"), "2026-09-26T13:07:16.000Z");
        assert_eq!(iso("2026-09-26T13:07:16.019Z"), "2026-09-26T13:07:16.019Z");
    }

    #[test]
    fn waiting_state_comes_from_the_execution_stack() {
        let run = json!({"executionData": {"nodeExecutionStack": [{
            "node": {"name": "Hold"},
            "data": {"main": [[{"json": {"a": 1}, "pairedItem": {"item": 0}}]]},
            "source": {"main": [{"previousNode": "Remember", "previousNodeOutput": 0, "previousNodeRun": 0}]}
        }]}});
        let s = wait_state_of(&run, Some("3000-01-01T00:00:00.000Z")).unwrap();
        assert_eq!(s["node"], "Hold");
        assert_eq!(s["till_ms"], Value::Null);
        assert_eq!(s["input"][0]["json"]["a"], 1);
    }
}
