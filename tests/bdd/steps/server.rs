//! Server lifecycle, environment, users and API keys.
//!
//! Auth contract (spec G2: `/rest` behaves like n8n's): `POST
//! /rest/owner/setup` creates the owner and sets the `n8n-auth` cookie,
//! `POST /rest/login` (`emailOrLdapLoginId`, `password`) logs in, `POST
//! /rest/api-keys` returns `data.rawApiKey`, and invitations go through
//! `POST /rest/invitations` + `POST /rest/invitations/:id/accept`.

use crate::support::process::{self, legacy_server_env, spawn_server};
use crate::world::{Auth, R8rWorld};
use cucumber::{given, then, when};
use serde_json::{json, Value};
use std::time::Duration;

pub const OWNER_EMAIL: &str = "owner@example.com";
pub const PASSWORD: &str = "Passw0rd!Passw0rd";

/// Every public API scope n8n defines for an owner key.
pub const ALL_SCOPES: &[&str] = &[
    "workflow:create", "workflow:read", "workflow:update", "workflow:delete", "workflow:list",
    "workflow:activate", "workflow:deactivate", "workflow:move", "workflowTags:update", "workflowTags:list",
    "execution:read", "execution:list", "execution:delete", "execution:retry",
    "credential:create", "credential:delete", "credential:move", "credential:list",
    "tag:create", "tag:read", "tag:update", "tag:delete", "tag:list",
    "variable:create", "variable:delete", "variable:list", "variable:update",
    "user:read", "user:list", "user:create", "user:delete", "user:changeRole",
    "project:create", "project:list", "project:update", "project:delete",
    "sourceControl:pull", "securityAudit:generate",
];

#[given(expr = "the environment variable {string} is {string}")]
async fn set_env(w: &mut R8rWorld, key: String, value: String) {
    w.unset_env.remove(&key);
    w.env.insert(key, value);
}

#[given(expr = "the environment variable {string} is not set")]
async fn unset_env(w: &mut R8rWorld, key: String) {
    w.env.remove(&key);
    w.unset_env.insert(key);
}

/// How long a process may take to listen: `R8R_BDD_START_TIMEOUT` seconds,
/// default 30.
pub fn start_timeout() -> Duration {
    Duration::from_secs(std::env::var("R8R_BDD_START_TIMEOUT").ok().and_then(|s| s.parse().ok()).unwrap_or(30))
}

/// Starts `r8r <args>` as the scenario's main server on a free port.
///
/// A port from `free_port()` can be taken by another process before r8r
/// binds it; then r8r exits with "Address already in use" while the other
/// process answers on the port. The first start of a scenario retries on a
/// new port when that happens. (Restarts keep their port: remembered URLs
/// such as resume URLs include it.)
pub async fn start_main(w: &mut R8rWorld, args: &[&str]) -> Result<(), String> {
    let fresh = w.port.is_none();
    let mut attempt = 0;
    loop {
        attempt += 1;
        match start_main_once(w, args).await {
            Err(e) if fresh && attempt < 4 && e.contains("Address already in use") => {
                w.port = Some(process::free_port());
                continue;
            }
            other => return other,
        }
    }
}

async fn start_main_once(w: &mut R8rWorld, args: &[&str]) -> Result<(), String> {
    let port = match w.port {
        Some(p) => p,
        None => {
            let p = process::free_port();
            w.port = Some(p);
            p
        }
    };
    let mut env = legacy_server_env(w.dir.path(), port);
    env.insert("N8N_PORT".into(), port.to_string());
    env.insert("N8N_LISTEN_ADDRESS".into(), "127.0.0.1".into());
    env.insert("WEBHOOK_URL".into(), format!("http://127.0.0.1:{port}/"));
    env.extend(w.process_env());
    // Scenario overrides win over the harness port too (e.g. default-port tests).
    if w.unset_env.contains("N8N_PORT") {
        env.remove("N8N_PORT");
        env.remove("PORT");
    }
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let log = w.dir.path().join(format!("server-{}.log", w.servers.len()));
    let wait_port = if w.unset_env.contains("N8N_PORT") { 5678 } else { port };
    let mut server = spawn_server(&args, &env, w.dir.path(), wait_port, log, true, start_timeout()).await?;
    // Listening is not ready: n8n, for one, answers "n8n is starting up"
    // until migrations finish. Wait for /healthz/readiness to settle, and
    // make sure the process answering is ours.
    let ready_url = format!("http://127.0.0.1:{wait_port}/healthz/readiness");
    let deadline = std::time::Instant::now() + start_timeout();
    loop {
        if let Ok(Some(status)) = server.child.try_wait() {
            return Err(format!("`r8r {}` exited with {status}\n--- log (tail) ---\n{}", args.join(" "), process::tail(&server.log(), 40)));
        }
        let settled = match w.http.get(&ready_url).send().await {
            Ok(r) => r.status().as_u16() != 503 && !r.text().await.unwrap_or_default().contains("starting up"),
            Err(_) => false,
        };
        if settled || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // A process that lost the port race exits right after logging it.
    tokio::time::sleep(Duration::from_millis(50)).await;
    if let Ok(Some(status)) = server.child.try_wait() {
        return Err(format!("`r8r {}` exited with {status}\n--- log (tail) ---\n{}", args.join(" "), process::tail(&server.log(), 40)));
    }
    w.servers.insert("main".into(), server);
    if wait_port != port {
        w.port = Some(wait_port);
        w.servers.get_mut("main").unwrap().port = wait_port;
    }
    Ok(())
}

#[given(expr = "a running r8r server")]
async fn running_server(w: &mut R8rWorld) {
    if let Err(e) = start_main(w, &["start"]).await {
        panic!("{e}");
    }
}

#[given(expr = "a running r8r server with an owner account")]
async fn running_with_owner(w: &mut R8rWorld) {
    running_server(w).await;
    setup_owner(w).await;
    w.auth = Auth::Session("owner".into());
}

#[given(expr = "a running r8r server with an owner and an API key")]
async fn running_with_key(w: &mut R8rWorld) {
    running_with_owner(w).await;
    let key = create_api_key(w, "owner", ALL_SCOPES).await;
    w.api_keys.insert("owner".into(), key);
    w.auth = Auth::ApiKey("owner".into());
}

#[when(expr = "I start the r8r server")]
async fn start_server_step(w: &mut R8rWorld) {
    if let Err(e) = start_main(w, &["start"]).await {
        w.vars.insert("SERVER_START_ERROR".into(), e);
    }
}

#[then(expr = "the server fails to start")]
async fn fails_to_start(w: &mut R8rWorld) {
    assert!(w.vars.contains_key("SERVER_START_ERROR"), "server started, but it should have refused to");
}

#[then(expr = "the server fails to start with a message containing {string}")]
async fn fails_to_start_msg(w: &mut R8rWorld, needle: String) {
    let err = w.vars.get("SERVER_START_ERROR").cloned().unwrap_or_else(|| panic!("server started, but it should have refused to"));
    assert!(err.contains(&needle), "startup failure does not mention {needle:?}:\n{err}");
}

fn cookie_from(resp: &crate::world::HttpResponse) -> Option<String> {
    resp.headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("n8n-auth="))
        .map(|v| v.split(';').next().unwrap().to_string())
}

pub async fn setup_owner(w: &mut R8rWorld) {
    let saved = std::mem::replace(&mut w.auth, Auth::None);
    let body = json!({"email": OWNER_EMAIL, "firstName": "Olivia", "lastName": "Owner", "password": PASSWORD});
    let resp = w.request("POST", "/rest/owner/setup", &[], Some(body.to_string())).await;
    let cookie = match cookie_from(&resp) {
        Some(c) => c,
        None => {
            let login = login(w, OWNER_EMAIL, PASSWORD).await;
            cookie_from(&login).unwrap_or_else(|| {
                panic!("owner setup did not yield an n8n-auth session:\n{}\n{}", resp.describe(), login.describe())
            })
        }
    };
    w.sessions.insert("owner".into(), cookie);
    w.auth = saved;
    // Remember the owner's user id for invitations.
    let saved = std::mem::replace(&mut w.auth, Auth::Session("owner".into()));
    let me = w.request("GET", "/rest/login", &[], None).await;
    if me.status == 200 {
        if let Some(id) = me.json().pointer("/data/id").and_then(Value::as_str) {
            w.vars.insert("USER_ID:owner".into(), id.to_string());
        }
    }
    w.auth = saved;
}

pub async fn login(w: &mut R8rWorld, email: &str, password: &str) -> crate::world::HttpResponse {
    let saved = std::mem::replace(&mut w.auth, Auth::None);
    let body = json!({"emailOrLdapLoginId": email, "password": password});
    let resp = w.request("POST", "/rest/login", &[], Some(body.to_string())).await;
    w.auth = saved;
    resp
}

pub async fn create_api_key(w: &mut R8rWorld, user: &str, scopes: &[&str]) -> String {
    let saved = std::mem::replace(&mut w.auth, Auth::Session(user.into()));
    let body = json!({"label": format!("bdd-{}", uuid::Uuid::new_v4()), "scopes": scopes, "expiresAt": null});
    let resp = w.request("POST", "/rest/api-keys", &[], Some(body.to_string())).await;
    w.auth = saved;
    let json: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
    json.pointer("/data/rawApiKey")
        .or_else(|| json.pointer("/data/apiKey"))
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| panic!("could not create an API key for {user}:\n{}", resp.describe()))
}

#[when(expr = "I log in as the owner")]
async fn login_owner(w: &mut R8rWorld) {
    login(w, OWNER_EMAIL, PASSWORD).await;
}

#[when(expr = "I log in as the owner with the password {string}")]
async fn login_owner_pw(w: &mut R8rWorld, pw: String) {
    login(w, OWNER_EMAIL, &pw).await;
}

#[then(expr = "the response sets the session cookie {string} with the attributes {string}")]
async fn cookie_attrs(w: &mut R8rWorld, name: String, attrs: String) {
    let r = w.response();
    let cookie = r
        .headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} cookie set: {}", r.describe()));
    let lower = cookie.to_ascii_lowercase();
    for attr in attrs.split(',') {
        let attr = attr.trim().to_ascii_lowercase();
        assert!(lower.contains(&attr), "cookie {cookie:?} lacks {attr}");
    }
}

/// Invites `email` as a global member and accepts the invitation, leaving a
/// session for them under their email.
#[given(expr = "a member user {string}")]
async fn member_user(w: &mut R8rWorld, email: String) {
    let saved = std::mem::replace(&mut w.auth, Auth::Session("owner".into()));
    let body = json!([{"email": email, "role": "global:member"}]);
    let resp = w.request("POST", "/rest/invitations", &[], Some(body.to_string())).await;
    let json: Value = serde_json::from_str(&resp.body).unwrap_or(Value::Null);
    let invitee = json
        .pointer("/data/0/user/id")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("invitation failed:\n{}", resp.describe()))
        .to_string();
    // n8n 2.x: the accept URL carries a signed token (`/signup?token=...`)
    // that is posted to /rest/invitations/accept.
    let token = json
        .pointer("/data/0/user/inviteAcceptUrl")
        .and_then(Value::as_str)
        .and_then(|u| u.split("token=").nth(1))
        .map(|t| t.split('&').next().unwrap().to_string())
        .unwrap_or_default();
    w.auth = Auth::None;
    let accept = json!({"token": token, "firstName": "Morgan", "lastName": "Member", "password": PASSWORD});
    let resp = w.request("POST", "/rest/invitations/accept", &[], Some(accept.to_string())).await;
    let cookie = match cookie_from(&resp) {
        Some(c) => c,
        None => {
            let l = login(w, &email, PASSWORD).await;
            cookie_from(&l).unwrap_or_else(|| panic!("member could not log in:\n{}\n{}", resp.describe(), l.describe()))
        }
    };
    w.sessions.insert(email.clone(), cookie);
    w.vars.insert(format!("USER_ID:{email}"), invitee);
    w.auth = saved;
}

/// Scopes a global member may put on an API key.
pub const MEMBER_SCOPES: &[&str] = &[
    "workflow:create", "workflow:read", "workflow:update", "workflow:delete", "workflow:list",
    "workflow:activate", "workflow:deactivate", "execution:read", "execution:list",
    "credential:create", "tag:read", "tag:list",
];

#[given(expr = "{string} has an API key")]
async fn member_key(w: &mut R8rWorld, user: String) {
    let scopes = if user == "owner" { ALL_SCOPES } else { MEMBER_SCOPES };
    let key = create_api_key(w, &user, scopes).await;
    w.api_keys.insert(user, key);
}

#[given(expr = "{string} has an API key with the scopes {string}")]
async fn member_key_scoped(w: &mut R8rWorld, user: String, scopes: String) {
    let scopes: Vec<&str> = scopes.split(',').map(str::trim).collect();
    let key = create_api_key(w, &user, &scopes).await;
    w.api_keys.insert(user, key);
}

#[given(expr = "I restart the r8r server")]
#[when(expr = "I restart the r8r server")]
async fn restart(w: &mut R8rWorld) {
    if let Some(mut s) = w.servers.remove("main") {
        s.terminate(Duration::from_secs(10)).await;
    }
    start_main(w, &["start"]).await.unwrap_or_else(|e| panic!("{e}"));
}

#[when(expr = "the r8r server is killed")]
async fn kill(w: &mut R8rWorld) {
    if let Some(mut s) = w.servers.remove("main") {
        s.kill().await;
        w.servers.insert("killed".into(), s);
    }
}

#[when(expr = "I start the r8r server again")]
async fn start_again(w: &mut R8rWorld) {
    start_main(w, &["start"]).await.unwrap_or_else(|e| panic!("{e}"));
}

#[when(expr = "I stop the r8r server gracefully")]
async fn stop_gracefully(w: &mut R8rWorld) {
    let mut s = w.servers.remove("main").expect("server running");
    let started = std::time::Instant::now();
    let code = s.terminate(Duration::from_secs(45)).await;
    w.vars.insert("SHUTDOWN_CODE".into(), format!("{code:?}"));
    w.vars.insert("SHUTDOWN_MS".into(), started.elapsed().as_millis().to_string());
    w.servers.insert("stopped".into(), s);
}

#[then(expr = "the server exited cleanly within {int} seconds")]
async fn exited_cleanly(w: &mut R8rWorld, secs: u64) {
    let code = w.vars.get("SHUTDOWN_CODE").cloned().unwrap_or_default();
    let ms: u64 = w.vars.get("SHUTDOWN_MS").and_then(|m| m.parse().ok()).unwrap_or(u64::MAX);
    assert_eq!(code, "Some(0)", "exit code");
    assert!(ms <= secs * 1000, "shutdown took {ms} ms");
}

fn any_log(w: &R8rWorld) -> String {
    w.servers.values().map(|s| s.log()).collect::<Vec<_>>().join("\n")
}

#[then(expr = "the server log contains {string}")]
async fn log_contains(w: &mut R8rWorld, needle: String) {
    let needle = w.expand(&needle);
    let found = super::eventually(Duration::from_secs(5), || {
        let log = any_log(w);
        let needle = needle.clone();
        async move { log.contains(&needle).then_some(()) }
    })
    .await;
    assert!(found.is_some(), "log lacks {needle:?}:\n{}", process::tail(&any_log(w), 40));
}

#[then(expr = "the server log does not contain {string}")]
async fn log_not_contains(w: &mut R8rWorld, needle: String) {
    let needle = w.expand(&needle);
    assert!(!any_log(w).contains(&needle), "log contains {needle:?}");
}

#[then(expr = "every server log line is a JSON object with {string} and {string}")]
async fn json_logs(w: &mut R8rWorld, a: String, b: String) {
    let log = w.server().log();
    let lines: Vec<&str> = log.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "server wrote no log lines");
    for line in lines {
        let v: Value = serde_json::from_str(line).unwrap_or_else(|_| panic!("log line is not JSON: {line}"));
        assert!(v.get(&a).is_some() && v.get(&b).is_some(), "log line lacks {a}/{b}: {line}");
    }
}

#[then(expr = "a server log line has the fields {string}")]
async fn log_line_fields(w: &mut R8rWorld, fields: String) {
    let fields: Vec<&str> = fields.split(',').map(str::trim).collect();
    let found = super::eventually(Duration::from_secs(5), || {
        let log = w.server().log();
        let fields = fields.clone();
        async move {
            log.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                // n8n nests context under "metadata"; accept either place.
                .any(|v| {
                    fields.iter().all(|f| {
                        crate::support::json::lookup(&v, f).is_some()
                            || crate::support::json::lookup(&v, &format!("metadata.{f}")).is_some()
                    })
                })
                .then_some(())
        }
    })
    .await;
    assert!(found.is_some(), "no JSON log line has all of {fields:?}:\n{}", process::tail(&w.server().log(), 30));
}

// ---- queue mode ---------------------------------------------------------

#[given(expr = "a running r8r worker named {string}")]
#[when(expr = "I start an r8r worker named {string}")]
async fn worker(w: &mut R8rWorld, name: String) {
    let mut env = legacy_server_env(w.dir.path(), process::free_port());
    env.extend(w.process_env());
    let health_port = process::free_port();
    env.insert("QUEUE_HEALTH_CHECK_ACTIVE".into(), "true".into());
    env.insert("QUEUE_HEALTH_CHECK_PORT".into(), health_port.to_string());
    let log = w.dir.path().join(format!("worker-{name}.log"));
    let args = vec!["worker".to_string(), "--concurrency=5".to_string()];
    let proc = spawn_server(&args, &env, w.dir.path(), health_port, log, true, start_timeout())
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    w.servers.insert(format!("worker:{name}"), proc);
}

#[when(expr = "the worker {string} is killed")]
async fn kill_worker(w: &mut R8rWorld, name: String) {
    let s = w.servers.get_mut(&format!("worker:{name}")).unwrap_or_else(|| panic!("no worker {name}"));
    s.kill().await;
}

#[given(expr = "queue mode backed by Redis")]
async fn queue_redis(w: &mut R8rWorld) {
    let host = std::env::var("R8R_BDD_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let port = std::env::var("R8R_BDD_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    w.env.insert("EXECUTIONS_MODE".into(), "queue".into());
    w.env.insert("QUEUE_BULL_REDIS_HOST".into(), host);
    w.env.insert("QUEUE_BULL_REDIS_PORT".into(), port);
    // Keep scenarios isolated from each other on a shared Redis.
    w.env.insert("QUEUE_BULL_PREFIX".into(), format!("bdd-{}", uuid::Uuid::new_v4()));
}

#[given(expr = "queue mode backed by PostgreSQL")]
async fn queue_postgres(w: &mut R8rWorld) {
    let url = std::env::var("R8R_BDD_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".into());
    let schema = format!("bdd_{}", uuid::Uuid::new_v4().simple());
    w.env.insert("EXECUTIONS_MODE".into(), "queue".into());
    w.env.insert("R8R_QUEUE_BACKEND".into(), "postgres".into());
    w.env.insert("DB_TYPE".into(), "postgresdb".into());
    w.env.insert("R8R_DATABASE_URL".into(), url);
    w.env.insert("DB_POSTGRESDB_SCHEMA".into(), schema);
}
