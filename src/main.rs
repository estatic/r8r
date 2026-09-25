use r8r::node::NodeRegistry;
use r8r::scheduler::Scheduler;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use r8r::trigger_registry::TriggerRegistry;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present; real process env vars still take precedence and
    // this is a no-op (not an error) when no .env file exists, e.g. in
    // production where vars are set directly.
    dotenvy::dotenv().ok();

    let cli = <r8r::cli::Cli as clap::Parser>::parse();
    match cli.command {
        None | Some(r8r::cli::Command::Start) => start_server().await,
        Some(command) => {
            // CLI output (e.g. `execute --rawOutput`) owns stdout; logs go to stderr.
            let _ = tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_env_filter(tracing_subscriber::EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into())))
                .try_init();
            std::process::exit(r8r::cli::run(command).await);
        }
    }
}

/// A secret derived from the instance encryption key, for installs that
/// only set `N8N_ENCRYPTION_KEY` (as n8n derives its JWT secret).
fn derived_secret(encryption_key: &str, purpose: &str) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(format!("{purpose}:{encryption_key}").as_bytes()).into()
}

async fn start_server() -> anyhow::Result<()> {
    let config = match r8r::n8n::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let log_file = std::env::var("R8R_LOG_FILE").ok().filter(|v| !v.trim().is_empty());
    // Held until main returns so the log file's background writer flushes.
    let _log_guard = r8r::logging::init(log_file.as_deref().map(std::path::Path::new))?;

    let database_url = config.database_url.clone();
    if let Some(parent) = database_url.strip_prefix("sqlite:").map(|p| std::path::Path::new(p.split('?').next().unwrap_or(p)).to_path_buf()).and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(&parent).ok();
        }
    }
    let jwt_secret = match std::env::var("JWT_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => hex::encode(derived_secret(&config.encryption_key, "jwt")),
    };
    let credentials_key = match std::env::var("CREDENTIALS_KEY") {
        Ok(_) => r8r::crypto::load_key_from_env("CREDENTIALS_KEY")?,
        Err(_) => derived_secret(&config.encryption_key, "credentials"),
    };
    let port = config.port;
    let open_registration = std::env::var("R8R_ALLOW_OPEN_REGISTRATION")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mut registry = NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);
    for line in r8r::logging::startup_summary(&r8r::logging::StartupInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        port,
        database_url: database_url.clone(),
        open_registration,
        node_types: registry.type_names().len(),
        log_file: log_file.clone(),
    }) {
        tracing::info!("{line}");
    }

    let storage = SqliteStorage::new(&database_url, credentials_key)
        .await
        .map_err(|e| anyhow::anyhow!("cannot open database {}: {e}", r8r::logging::redact_url(&database_url)))?;
    let scheduler = Scheduler::new().await?;

    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
        scheduler: Arc::new(scheduler),
        trigger_registry: Arc::new(TriggerRegistry::new()),
        execution_events: tokio::sync::broadcast::channel(r8r::execution_runner::EXECUTION_EVENTS_CAPACITY).0,
        open_registration,
    };

    match r8r::triggers::reactivate_all(&state).await {
        Ok(s) => tracing::info!(
            "reactivated {} active workflow(s): {} schedule, {} webhook, {} telegram{}",
            s.schedule + s.webhook + s.telegram,
            s.schedule,
            s.webhook,
            s.telegram,
            if s.failed > 0 { format!(" ({} failed, see warnings above)", s.failed) } else { String::new() }
        ),
        Err(e) => tracing::warn!(error = %e, "failed to reactivate workflow triggers on startup"),
    }

    let app = r8r::api::build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("{}:{port}", config.listen_address))
        .await
        .map_err(|e| anyhow::anyhow!("cannot listen on port {port}: {e} (set PORT to use another port)"))?;
    tracing::info!("r8r ready on http://localhost:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown requested, finishing in-flight requests");
        })
        .await?;
    tracing::info!("r8r stopped");
    Ok(())
}
