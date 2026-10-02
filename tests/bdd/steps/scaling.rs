//! Multi-main leader election (plan task 4.1): steps for running more than
//! one `r8r start` main against a shared Redis + PostgreSQL, and for
//! targeting a specific named main with an HTTP request or shutting one
//! down to watch leadership move.

use super::server::start_timeout;
use crate::support::process::{self, legacy_server_env, spawn_server};
use crate::world::{HttpResponse, R8rWorld};
use cucumber::{given, then, when};
use serde_json::Value;
use std::time::Duration;

/// Queue mode (spec §7.3) backed by Redis, with the workflow/credentials
/// database on a shared PostgreSQL schema -- what two mains need to agree
/// on both the leader lock and the active-workflow table. Unlike `Given
/// queue mode backed by Redis` (sqlite storage, one process per scenario),
/// this is for scenarios that start more than one main.
#[given(expr = "queue mode backed by Redis and PostgreSQL")]
async fn queue_redis_and_postgres(w: &mut R8rWorld) {
    let redis_host = std::env::var("R8R_BDD_REDIS_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let redis_port = std::env::var("R8R_BDD_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    let pg_url = std::env::var("R8R_BDD_POSTGRES_URL").unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5432/postgres".into());
    let schema = format!("bdd_{}", uuid::Uuid::new_v4().simple());
    w.env.insert("EXECUTIONS_MODE".into(), "queue".into());
    w.env.insert("QUEUE_BULL_REDIS_HOST".into(), redis_host);
    w.env.insert("QUEUE_BULL_REDIS_PORT".into(), redis_port);
    w.env.insert("QUEUE_BULL_PREFIX".into(), format!("bdd-{}", uuid::Uuid::new_v4()));
    w.env.insert("DB_TYPE".into(), "postgresdb".into());
    w.env.insert("R8R_DATABASE_URL".into(), pg_url);
    w.env.insert("DB_POSTGRESDB_SCHEMA".into(), schema);
}

/// Starts a second `r8r start` main with the scenario's current `w.env`
/// (so it shares the Redis leader lock and the PostgreSQL database with
/// "main"), on its own port, tracked under its own name. Unlike `main`,
/// this does not set up an owner -- the first main already did, in the
/// shared database.
#[given(expr = "a running r8r server named {string} with the same configuration")]
async fn second_main(w: &mut R8rWorld, name: String) {
    let port = process::free_port();
    let mut env = legacy_server_env(w.dir.path(), port);
    env.insert("N8N_PORT".into(), port.to_string());
    env.insert("N8N_LISTEN_ADDRESS".into(), "127.0.0.1".into());
    env.insert("WEBHOOK_URL".into(), format!("http://127.0.0.1:{port}/"));
    env.extend(w.process_env());
    let log = w.dir.path().join(format!("server-{name}.log"));
    let server = spawn_server(&["start".to_string()], &env, w.dir.path(), port, log, true, start_timeout())
        .await
        .unwrap_or_else(|e| panic!("could not start second main {name:?}: {e}"));
    let ready_url = format!("http://127.0.0.1:{port}/healthz/readiness");
    let deadline = std::time::Instant::now() + start_timeout();
    loop {
        let settled = w.http.get(&ready_url).send().await.map(|r| r.status().as_u16() != 503).unwrap_or(false);
        if settled || std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    w.servers.insert(name, server);
}

fn base_url(w: &R8rWorld, name: &str) -> String {
    w.servers.get(name).unwrap_or_else(|| panic!("no server named {name:?} running")).base_url()
}

/// Sends a request to a specific named main (as opposed to the usual
/// requests, which always go to "main"), to check that webhooks/forms
/// answer on every main, not only the leader.
#[when(expr = "I send a {word} request to {string} on the server named {string}")]
async fn request_to_named(w: &mut R8rWorld, method: String, path: String, name: String) {
    let url = format!("{}{}", base_url(w, &name), w.expand(&path));
    let method_parsed = reqwest::Method::from_bytes(method.to_uppercase().as_bytes()).expect("HTTP method");
    let started = std::time::Instant::now();
    let resp = w.http.request(method_parsed, &url).send().await.unwrap_or_else(|e| panic!("{method} {url} failed: {e}"));
    let status = resp.status().as_u16();
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_default();
    w.response = Some(HttpResponse { method: method.to_uppercase(), url, status, headers, body, elapsed: started.elapsed() });
}

/// Finds whichever running main currently reports itself as the leader
/// (`/healthz/readiness`'s `isLeader`, plan task 4.1) and terminates it, to
/// check that the other main takes over within the TTL + check interval.
#[when(expr = "I stop the leader main")]
async fn stop_leader(w: &mut R8rWorld) {
    let names: Vec<String> = w.servers.keys().cloned().collect();
    let leader_name;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    'search: loop {
        for name in &names {
            let Some(server) = w.servers.get(name) else { continue };
            let url = format!("{}/healthz/readiness", server.base_url());
            if let Ok(resp) = w.http.get(&url).send().await {
                if let Ok(json) = resp.json::<Value>().await {
                    if json["isLeader"] == true {
                        leader_name = Some(name.clone());
                        break 'search;
                    }
                }
            }
        }
        if std::time::Instant::now() > deadline {
            panic!("no running main reports itself as the leader among {names:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let name = leader_name.expect("leader found");
    let mut server = w.servers.remove(&name).expect("leader server still tracked");
    server.terminate(Duration::from_secs(10)).await;
    w.servers.insert(format!("stopped:{name}"), server);
}

/// Multi-main's webhook/form registration sync (`activation::sync_registrations`)
/// polls every 2 seconds; this waits long enough for every running main to
/// have picked up an activation made through just one of them.
#[given(expr = "the mains have synced")]
#[when(expr = "the mains have synced")]
async fn mains_synced(_w: &mut R8rWorld) {
    tokio::time::sleep(Duration::from_secs(3)).await;
}

/// Like `the workflow has at least N executions`, but also asserts an
/// upper bound, to check a schedule firing on exactly one main rather than
/// once per main (plan task 4.1's acceptance: "two mains fire a schedule
/// once").
#[then(expr = "after {int} seconds the workflow has between {int} and {int} executions")]
async fn count_between(w: &mut R8rWorld, secs: u64, min: usize, max: usize) {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let name = w.wf().name.clone();
    let id = super::api::workflow_id(w, &name).expect("workflow created");
    let list = super::api::list_executions(w, &id).await;
    assert!(
        list.len() >= min && list.len() <= max,
        "expected between {min} and {max} executions after {secs}s, got {}: {}",
        list.len(),
        crate::world::pretty(&Value::Array(list.clone()))
    );
}
