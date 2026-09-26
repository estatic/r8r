//! Typed configuration from the environment, using n8n's variable names
//! (spec §3.2, goal G4). Everything is validated once at start-up; an
//! invalid value names its variable.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub user_folder: PathBuf,
    pub encryption_key: String,
    pub port: u16,
    pub listen_address: String,
    /// `sqlite:...` or, with `DB_TYPE=postgresdb`, `postgres://...`.
    pub database_url: String,
    /// The legacy `/rest/r8r` API keeps its tables in SQLite whatever
    /// `DB_TYPE` says.
    pub legacy_database_url: String,
    pub executions_mode: String,
    pub db_type: String,
    pub log_level: String,
    pub log_format: String,
    pub executions_data_max_age_hours: u64,
    pub block_env_access_in_node: bool,
    pub nodes_exclude: Vec<String>,
    /// Hosts the SSRF guard lets through even though they resolve to
    /// private addresses (`R8R_SSRF_ALLOWED_HOSTS`).
    pub ssrf_allowed_hosts: Vec<String>,
    pub webhook_url: String,
    pub timezone: String,
    pub runners_task_timeout_secs: u64,
    pub node_function_allow_builtin: Vec<String>,
    /// Per-expression time limit (spec §6.4: 1 s by default).
    pub expression_timeout_ms: u64,
}

#[derive(Debug)]
pub struct ConfigError {
    pub variable: String,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid configuration: {}: {}", self.variable, self.message)
    }
}

impl std::error::Error for ConfigError {}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn err(variable: &str, message: impl Into<String>) -> ConfigError {
    ConfigError { variable: variable.into(), message: message.into() }
}

fn parse_bool(name: &str, default: bool) -> Result<bool, ConfigError> {
    match var(name).as_deref().map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("true" | "1" | "yes") => Ok(true),
        Some("false" | "0" | "no") => Ok(false),
        Some(other) => Err(err(name, format!("expected true or false, got \"{other}\""))),
    }
}

fn one_of(name: &str, default: &str, allowed: &[&str]) -> Result<String, ConfigError> {
    let value = var(name).unwrap_or_else(|| default.to_string());
    if allowed.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(err(name, format!("expected one of {}, got \"{value}\"", allowed.join(", "))))
    }
}

/// A JSON array of strings, or a comma-separated list.
fn string_list(name: &str, default: &[&str]) -> Result<Vec<String>, ConfigError> {
    let Some(raw) = var(name) else { return Ok(default.iter().map(|s| s.to_string()).collect()) };
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<String>>(trimmed).map_err(|e| err(name, format!("expected a JSON array of strings: {e}")))
    } else {
        Ok(trimmed.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
    }
}

impl Config {
    /// Reads and validates the environment. Also makes sure an encryption
    /// key exists: like n8n, one is generated into
    /// `<user folder>/.n8n/config` on first use and reused afterwards.
    pub fn load() -> Result<Self, ConfigError> {
        let user_folder = var("N8N_USER_FOLDER")
            .or_else(|| var("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let port = match var("N8N_PORT").or_else(|| var("PORT")) {
            None => 5678,
            Some(p) => p.parse::<u16>().map_err(|_| err("N8N_PORT", format!("expected a port number, got \"{p}\"")))?,
        };
        let executions_mode = one_of("EXECUTIONS_MODE", "regular", &["regular", "queue"])?;
        let db_type = one_of("DB_TYPE", "sqlite", &["sqlite", "postgresdb"])?;
        let log_level = one_of("N8N_LOG_LEVEL", "info", &["error", "warn", "info", "debug", "silent"])?;
        let log_format = one_of("N8N_LOG_FORMAT", "text", &["text", "json"])?;
        let executions_data_max_age_hours = match var("EXECUTIONS_DATA_MAX_AGE") {
            None => 336,
            Some(v) => v
                .parse::<u64>()
                .ok()
                .filter(|h| *h > 0)
                .ok_or_else(|| err("EXECUTIONS_DATA_MAX_AGE", format!("expected a positive number of hours, got \"{v}\"")))?,
        };
        let runners_task_timeout_secs = match var("N8N_RUNNERS_TASK_TIMEOUT") {
            None => 60,
            Some(v) => v.parse().map_err(|_| err("N8N_RUNNERS_TASK_TIMEOUT", format!("expected seconds, got \"{v}\"")))?,
        };
        let expression_timeout_ms = match var("R8R_EXPRESSION_TIMEOUT_MS") {
            None => 1000,
            Some(v) => v.parse().map_err(|_| err("R8R_EXPRESSION_TIMEOUT_MS", format!("expected milliseconds, got \"{v}\"")))?,
        };
        let n8n_dir = user_folder.join(".n8n");
        let sqlite_url = var("DATABASE_URL").filter(|u| u.starts_with("sqlite:")).unwrap_or_else(|| {
            let file = var("DB_SQLITE_DATABASE").map(PathBuf::from).unwrap_or_else(|| n8n_dir.join("database.sqlite"));
            format!("sqlite:{}", file.display())
        });
        let database_url = if db_type == "postgresdb" { postgres_url()? } else { sqlite_url.clone() };
        let encryption_key = match var("N8N_ENCRYPTION_KEY") {
            Some(k) => k,
            None => load_or_create_key(&n8n_dir).map_err(|e| err("N8N_ENCRYPTION_KEY", e.to_string()))?,
        };
        Ok(Self {
            encryption_key,
            port,
            listen_address: var("N8N_LISTEN_ADDRESS").unwrap_or_else(|| "0.0.0.0".into()),
            database_url,
            legacy_database_url: sqlite_url,
            executions_mode,
            db_type,
            log_level,
            log_format,
            executions_data_max_age_hours,
            block_env_access_in_node: parse_bool("N8N_BLOCK_ENV_ACCESS_IN_NODE", true)?,
            nodes_exclude: string_list(
                "NODES_EXCLUDE",
                &["n8n-nodes-base.executeCommand", "n8n-nodes-base.localFileTrigger"],
            )?,
            ssrf_allowed_hosts: string_list("R8R_SSRF_ALLOWED_HOSTS", &[])?,
            webhook_url: var("WEBHOOK_URL").map(|u| if u.ends_with('/') { u } else { format!("{u}/") }).unwrap_or_else(|| format!("http://localhost:{port}/")),
            timezone: var("GENERIC_TIMEZONE").unwrap_or_else(|| "America/New_York".into()),
            runners_task_timeout_secs,
            node_function_allow_builtin: string_list("NODE_FUNCTION_ALLOW_BUILTIN", &[])?,
            expression_timeout_ms,
            user_folder,
        })
    }

    pub fn n8n_dir(&self) -> PathBuf {
        self.user_folder.join(".n8n")
    }

    /// `(variable, value)` pairs for `r8r config check`, secrets redacted.
    pub fn entries(&self) -> Vec<(&'static str, String)> {
        vec![
            ("N8N_USER_FOLDER", self.user_folder.display().to_string()),
            ("N8N_PORT", self.port.to_string()),
            ("N8N_LISTEN_ADDRESS", self.listen_address.clone()),
            ("DB_TYPE", self.db_type.clone()),
            ("DATABASE_URL", crate::logging::redact_url(&self.database_url)),
            ("EXECUTIONS_MODE", self.executions_mode.clone()),
            ("EXECUTIONS_DATA_MAX_AGE", self.executions_data_max_age_hours.to_string()),
            ("N8N_ENCRYPTION_KEY", "********".into()),
            ("N8N_LOG_LEVEL", self.log_level.clone()),
            ("N8N_LOG_FORMAT", self.log_format.clone()),
            ("N8N_BLOCK_ENV_ACCESS_IN_NODE", self.block_env_access_in_node.to_string()),
            ("NODES_EXCLUDE", serde_json::to_string(&self.nodes_exclude).unwrap()),
            ("R8R_SSRF_ALLOWED_HOSTS", self.ssrf_allowed_hosts.join(",")),
            ("WEBHOOK_URL", self.webhook_url.clone()),
            ("GENERIC_TIMEZONE", self.timezone.clone()),
            ("N8N_RUNNERS_TASK_TIMEOUT", self.runners_task_timeout_secs.to_string()),
            ("NODE_FUNCTION_ALLOW_BUILTIN", self.node_function_allow_builtin.join(",")),
            ("R8R_EXPRESSION_TIMEOUT_MS", self.expression_timeout_ms.to_string()),
        ]
    }
}

/// The PostgreSQL URL: `R8R_DATABASE_URL`, or n8n's `DB_POSTGRESDB_HOST`,
/// `_PORT`, `_DATABASE`, `_USER`, `_PASSWORD` (the schema comes from
/// `DB_POSTGRESDB_SCHEMA` when the store connects).
fn postgres_url() -> Result<String, ConfigError> {
    if let Some(url) = var("R8R_DATABASE_URL") {
        if !(url.starts_with("postgres://") || url.starts_with("postgresql://")) {
            return Err(err("R8R_DATABASE_URL", "expected a postgres:// URL when DB_TYPE=postgresdb"));
        }
        return Ok(url);
    }
    let enc = |s: String| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    let host = var("DB_POSTGRESDB_HOST").unwrap_or_else(|| "localhost".into());
    let port = var("DB_POSTGRESDB_PORT").unwrap_or_else(|| "5432".into());
    port.parse::<u16>().map_err(|_| err("DB_POSTGRESDB_PORT", format!("expected a port number, got \"{port}\"")))?;
    let database = var("DB_POSTGRESDB_DATABASE").unwrap_or_else(|| "n8n".into());
    let user = var("DB_POSTGRESDB_USER").unwrap_or_else(|| "postgres".into());
    let password = var("DB_POSTGRESDB_PASSWORD").unwrap_or_default();
    let ssl = if var("DB_POSTGRESDB_SSL_ENABLED").as_deref() == Some("true") { "?sslmode=require" } else { "" };
    Ok(format!("postgres://{}:{}@{host}:{port}/{}{ssl}", enc(user), enc(password), enc(database)))
}

fn load_or_create_key(n8n_dir: &std::path::Path) -> anyhow::Result<String> {
    let path = n8n_dir.join("config");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let json: serde_json::Value = serde_json::from_str(&text)?;
        if let Some(key) = json["encryptionKey"].as_str() {
            return Ok(key.to_string());
        }
    }
    use base64::Engine as _;
    let bytes: [u8; 24] = rand::random();
    let key = base64::engine::general_purpose::STANDARD.encode(bytes);
    std::fs::create_dir_all(n8n_dir)?;
    std::fs::write(&path, serde_json::to_string_pretty(&serde_json::json!({ "encryptionKey": key }))?)?;
    Ok(key)
}
