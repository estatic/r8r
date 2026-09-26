//! Terminal + optional rotating-file logging, and the startup summary.

use std::path::Path;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

/// Used when `RUST_LOG` is unset: r8r's own events at info, everything
/// else (sqlx, hyper, ...) only at warn so it doesn't drown them out.
const DEFAULT_FILTER: &str = "r8r=info,warn";

pub fn filter_directive(rust_log: Option<&str>) -> String {
    match rust_log {
        Some(v) if !v.trim().is_empty() => v.to_string(),
        _ => DEFAULT_FILTER.to_string(),
    }
}

/// Keeps the background file writer alive; dropping it flushes the file.
/// Hold it for the life of the process.
pub struct LogGuard(#[allow(dead_code)] Option<tracing_appender::non_blocking::WorkerGuard>);

/// A non-blocking writer to `path`, rotated daily (`<name>.YYYY-MM-DD`).
/// Creates the parent directory if needed.
pub fn file_writer(
    path: &Path,
) -> anyhow::Result<(tracing_appender::non_blocking::NonBlocking, tracing_appender::non_blocking::WorkerGuard)> {
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)
        .map_err(|e| anyhow::anyhow!("cannot create log directory {}: {e}", dir.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("R8R_LOG_FILE must name a file, got {}", path.display()))?;
    let appender = tracing_appender::rolling::daily(dir, file_name);
    Ok(tracing_appender::non_blocking(appender))
}

/// Installs the global subscriber: always the terminal, plus `log_file`
/// (without ANSI colors) when given. Filter from `RUST_LOG`, else
/// `DEFAULT_FILTER`.
pub fn init(log_file: Option<&Path>) -> anyhow::Result<LogGuard> {
    init_with(log_file, &LogOptions::default())
}

/// n8n's logging settings (`N8N_LOG_LEVEL`, `N8N_LOG_FORMAT`).
#[derive(Default)]
pub struct LogOptions {
    /// `error`, `warn`, `info`, `debug` or `silent`; `None` keeps the default.
    pub level: Option<String>,
    /// One JSON object per line instead of text.
    pub json: bool,
}

/// The filter for an n8n log level (`RUST_LOG` still wins when set).
pub fn level_directive(level: Option<&str>) -> String {
    match level {
        Some("silent") => "off".into(),
        Some(l @ ("error" | "warn" | "info" | "debug")) => format!("r8r={l},warn"),
        _ => DEFAULT_FILTER.into(),
    }
}

pub fn init_with(log_file: Option<&Path>, options: &LogOptions) -> anyhow::Result<LogGuard> {
    let directive = match std::env::var("RUST_LOG").ok().filter(|v| !v.trim().is_empty()) {
        Some(v) => v,
        None => level_directive(options.level.as_deref()),
    };
    let terminal = if options.json {
        fmt::layer().json().flatten_event(true).with_current_span(false).with_span_list(false).with_filter(EnvFilter::new(&directive)).boxed()
    } else {
        fmt::layer().with_filter(EnvFilter::new(&directive)).boxed()
    };
    let (file, guard) = match log_file {
        Some(path) => {
            let (writer, guard) = file_writer(path)?;
            let layer = fmt::layer().with_writer(writer).with_ansi(false).with_filter(EnvFilter::new(&directive));
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };
    tracing_subscriber::registry().with(terminal).with(file).try_init()?;
    Ok(LogGuard(guard))
}

/// What the startup summary reports. Deliberately has no secret fields.
pub struct StartupInfo {
    pub version: String,
    pub port: u16,
    pub database_url: String,
    pub open_registration: bool,
    pub node_types: usize,
    pub log_file: Option<String>,
}

/// Hides `user:password@` in a URL-shaped database string.
pub fn redact_url(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme_end), Some(at)) if at > scheme_end => format!("{}***{}", &url[..scheme_end + 3], &url[at..]),
        _ => url.to_string(),
    }
}

pub fn startup_summary(info: &StartupInfo) -> Vec<String> {
    vec![
        format!("r8r v{} starting", info.version),
        format!("  listening on http://0.0.0.0:{port}  (open http://localhost:{port})", port = info.port),
        format!("  database: {}", redact_url(&info.database_url)),
        format!(
            "  registration: {}",
            if info.open_registration { "open" } else { "first user only" }
        ),
        format!("  node types: {}", info.node_types),
        match &info.log_file {
            Some(path) => format!("  log file: {path} (rotated daily)"),
            None => "  log file: terminal only".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_defaults_to_info_for_r8r_and_warn_elsewhere() {
        assert_eq!(filter_directive(None), "r8r=info,warn");
        assert_eq!(filter_directive(Some("")), "r8r=info,warn");
    }

    #[test]
    fn rust_log_overrides_the_default_filter() {
        assert_eq!(filter_directive(Some("debug")), "debug");
    }

    #[test]
    fn redacts_credentials_in_a_database_url() {
        assert_eq!(redact_url("postgres://admin:hunter2@db:5432/r8r"), "postgres://***@db:5432/r8r");
        assert_eq!(redact_url("sqlite:./r8r.db?mode=rwc"), "sqlite:./r8r.db?mode=rwc");
    }

    fn info() -> StartupInfo {
        StartupInfo {
            version: "0.1.0".into(),
            port: 3000,
            database_url: "postgres://admin:hunter2@db/r8r".into(),
            open_registration: false,
            node_types: 14,
            log_file: None,
        }
    }

    #[test]
    fn startup_summary_shows_where_the_app_is_listening() {
        let lines = startup_summary(&info()).join("\n");
        assert!(lines.contains("r8r v0.1.0 starting"), "{lines}");
        assert!(lines.contains("listening on http://0.0.0.0:3000"), "{lines}");
        assert!(lines.contains("http://localhost:3000"), "{lines}");
        assert!(lines.contains("registration: first user only"), "{lines}");
        assert!(lines.contains("node types: 14"), "{lines}");
        assert!(lines.contains("log file: terminal only"), "{lines}");
    }

    #[test]
    fn startup_summary_never_contains_secrets() {
        let lines = startup_summary(&info()).join("\n");
        assert!(!lines.contains("hunter2"), "{lines}");
        assert!(lines.contains("database: postgres://***@db/r8r"), "{lines}");
    }

    #[test]
    fn startup_summary_names_the_log_file_and_open_registration() {
        let mut i = info();
        i.open_registration = true;
        i.log_file = Some("./logs/r8r.log".into());
        let lines = startup_summary(&i).join("\n");
        assert!(lines.contains("registration: open"), "{lines}");
        assert!(lines.contains("log file: ./logs/r8r.log (rotated daily)"), "{lines}");
    }

    #[test]
    fn file_writer_creates_the_directory_and_receives_log_lines() {
        let dir = std::env::temp_dir().join(format!("r8r-log-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("nested").join("r8r.log");
        let (writer, guard) = file_writer(&path).unwrap();
        let subscriber = tracing_subscriber::fmt().with_writer(writer).with_ansi(false).finish();
        tracing::subscriber::with_default(subscriber, || tracing::info!("hello from the file test"));
        drop(guard); // flushes the background writer
        let contents: String = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert!(contents.contains("hello from the file test"), "{contents}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
