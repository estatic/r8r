//! Spawning the r8r binary: one-shot CLI commands and long-running server
//! processes. Everything runs black-box, so the suite keeps compiling however
//! the crate's internals are restructured.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

/// Passphrase given to every process as `N8N_ENCRYPTION_KEY`.
pub const ENCRYPTION_KEY: &str = "bdd-n8n-encryption-key";

/// The binary under test: `R8R_BIN` if set, else the one Cargo built for
/// this package.
pub fn binary() -> PathBuf {
    if let Ok(p) = std::env::var("R8R_BIN") {
        return PathBuf::from(p);
    }
    match option_env!("CARGO_BIN_EXE_r8r") {
        Some(p) => PathBuf::from(p),
        None => panic!("no r8r binary: set R8R_BIN to the executable under test"),
    }
}

/// Environment every spawned process starts from. The process environment
/// is cleared first so a developer's `.env`/shell can't leak in, and the
/// working directory is the scenario's scratch folder.
///
/// Only spec variable names (n8n's, plus `R8R_*` for r8r-only settings) go
/// here. Variables the *current* implementation needs to boot a server are
/// added separately by [`legacy_server_env`].
pub fn base_env(user_folder: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PATH".into(), std::env::var("PATH").unwrap_or_default());
    env.insert("HOME".into(), user_folder.display().to_string());
    env.insert("N8N_USER_FOLDER".into(), user_folder.display().to_string());
    env.insert("N8N_ENCRYPTION_KEY".into(), ENCRYPTION_KEY.into());
    env.insert("N8N_DIAGNOSTICS_ENABLED".into(), "false".into());
    env.insert("N8N_LOG_LEVEL".into(), "info".into());
    // Scenarios call wiremock on 127.0.0.1; the SSRF guard (spec §5.3)
    // must be told that's allowed. The SSRF feature clears this.
    env.insert("R8R_SSRF_ALLOWED_HOSTS".into(), "127.0.0.1,localhost".into());
    env.insert("TZ".into(), "UTC".into());
    // n8n's task broker defaults to port 5679; give every process its own
    // so parallel scenarios don't collide.
    env.insert("N8N_RUNNERS_BROKER_PORT".into(), free_port().to_string());
    env.insert("GENERIC_TIMEZONE".into(), "UTC".into());
    // R8R_BDD_STORAGE=postgres runs every scenario on PostgreSQL storage,
    // each in its own schema (derived from the scenario's folder).
    if std::env::var("R8R_BDD_STORAGE").is_ok_and(|s| s == "postgres") {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        user_folder.hash(&mut h);
        let url = std::env::var("R8R_BDD_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".into());
        env.insert("DB_TYPE".into(), "postgresdb".into());
        env.insert("R8R_DATABASE_URL".into(), url);
        env.insert("DB_POSTGRESDB_SCHEMA".into(), format!("bdd_{:016x}", h.finish()));
    }
    env
}

/// Server-only variables on top of [`base_env`]. The server and the CLI
/// share `<user folder>/.n8n/database.sqlite`, as n8n's do.
pub fn legacy_server_env(_user_folder: &Path, port: u16) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert("PORT".into(), port.to_string());
    env
}

#[derive(Debug, Clone)]
pub struct CliOutput {
    pub args: Vec<String>,
    /// `None` when the process was killed by the timeout or a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
    pub timed_out: bool,
}

impl CliOutput {
    pub fn describe(&self) -> String {
        format!(
            "`r8r {}` exit={:?}{} after {:?}\n--- stdout (tail) ---\n{}\n--- stderr (tail) ---\n{}",
            self.args.join(" "),
            self.code,
            if self.timed_out { " (TIMED OUT, killed)" } else { "" },
            self.elapsed,
            tail(&self.stdout, 40),
            tail(&self.stderr, 40)
        )
    }
}

pub fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

pub async fn run_cli(
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: &Path,
    stdin: Option<&str>,
    timeout: Duration,
) -> CliOutput {
    let started = Instant::now();
    let mut cmd = Command::new(binary());
    cmd.args(args)
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => panic!("cannot spawn {}: {e}", binary().display()),
    };
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        let _ = pipe.write_all(input.as_bytes()).await;
    }
    let mut stdout_pipe = child.stdout.take().unwrap();
    let mut stderr_pipe = child.stderr.take().unwrap();
    let read_out = tokio::spawn(async move {
        let mut s = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stdout_pipe, &mut s).await;
        String::from_utf8_lossy(&s).into_owned()
    });
    let read_err = tokio::spawn(async move {
        let mut s = Vec::new();
        let _ = tokio::io::AsyncReadExt::read_to_end(&mut stderr_pipe, &mut s).await;
        String::from_utf8_lossy(&s).into_owned()
    });
    let (code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => (status.ok().and_then(|s| s.code()), false),
        Err(_) => {
            let _ = child.kill().await;
            (None, true)
        }
    };
    let stdout = tokio::time::timeout(Duration::from_secs(5), read_out).await.ok().and_then(Result::ok).unwrap_or_default();
    let stderr = tokio::time::timeout(Duration::from_secs(5), read_err).await.ok().and_then(Result::ok).unwrap_or_default();
    CliOutput { args: args.to_vec(), code, stdout, stderr, elapsed: started.elapsed(), timed_out }
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// A running `r8r` process (server, worker, webhook processor).
#[derive(Debug)]
pub struct ServerProcess {
    pub child: Child,
    pub port: u16,
    pub log_path: PathBuf,
    /// Time from spawn until the port accepted a TCP connection.
    pub ready_after: Option<Duration>,
}

impl ServerProcess {
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Resident set size in MB, read from /proc (Linux only).
    pub fn rss_mb(&self) -> Option<f64> {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.pid()?)).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb / 1024.0)
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }

    /// SIGTERM, then wait up to `grace` for a clean exit.
    pub async fn terminate(&mut self, grace: Duration) -> Option<i32> {
        if let Some(pid) = self.pid() {
            let _ = std::process::Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
        }
        match tokio::time::timeout(grace, self.child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => {
                self.kill().await;
                None
            }
        }
    }
}

/// Spawns `r8r <args>` with output captured to `log_path`, and when
/// `wait_for_port` is set waits until the port accepts connections (any
/// HTTP semantics are for the scenarios to check). Fails with the log tail
/// if the process exits or doesn't listen within `timeout`.
pub async fn spawn_server(
    args: &[String],
    env: &BTreeMap<String, String>,
    cwd: &Path,
    port: u16,
    log_path: PathBuf,
    wait_for_port: bool,
    timeout: Duration,
) -> Result<ServerProcess, String> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let child = Command::new(binary())
        .args(args)
        .env_clear()
        .envs(env)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("cannot spawn {}: {e}", binary().display()))?;
    let mut server = ServerProcess { child, port, log_path, ready_after: None };
    if !wait_for_port {
        return Ok(server);
    }
    let deadline = Instant::now() + timeout;
    loop {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            server.ready_after = Some(started.elapsed());
            return Ok(server);
        }
        if let Ok(Some(status)) = server.child.try_wait() {
            return Err(format!(
                "`r8r {}` exited with {status} before listening on port {port}\n--- log (tail) ---\n{}",
                args.join(" "),
                tail(&server.log(), 40)
            ));
        }
        if Instant::now() > deadline {
            return Err(format!(
                "`r8r {}` did not listen on port {port} within {timeout:?}\n--- log (tail) ---\n{}",
                args.join(" "),
                tail(&server.log(), 40)
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Splits a command line on whitespace, honouring double quotes.
pub fn split_args(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in line.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(current);
    }
    args
}
