//! Steps for the legacy r8r editor API (`/rest/r8r`, JWT auth), which r8r's
//! own Vue editor uses until it moves onto the n8n-compatible API
//! (plan 2026-09-27, Phase 3). Scenarios using these are `@r8r-only`.

use crate::world::{Auth, R8rWorld};
use cucumber::{given, then, when};
use serde_json::{json, Value};

const PASSWORD: &str = "correct-horse-battery";

/// Registers (first user) or logs in, then sends the JWT on every request.
#[given(expr = "I am logged in to the legacy r8r API as {string}")]
#[when(expr = "I am logged in to the legacy r8r API as {string}")]
async fn legacy_login(w: &mut R8rWorld, email: String) {
    w.auth = Auth::None;
    let body = json!({"email": email, "password": PASSWORD}).to_string();
    let mut r = w.request("POST", "/rest/r8r/auth/register", &[], Some(body.clone())).await;
    if r.status != 201 {
        r = w.request("POST", "/rest/r8r/auth/login", &[], Some(body)).await;
    }
    assert!(matches!(r.status, 200 | 201), "legacy login failed ({}): {}", r.status, r.body);
    let token = r.json()["token"].as_str().unwrap_or_else(|| panic!("no token: {}", r.body)).to_string();
    w.auth = Auth::Bearer(token);
}

#[given(expr = "my legacy r8r token is {string}")]
async fn legacy_token(w: &mut R8rWorld, token: String) {
    w.auth = Auth::Bearer(token);
}

/// Polls `GET /rest/r8r/executions/:id` until the run leaves `Running`;
/// the finished execution is the last response.
#[when(expr = "I wait for the legacy execution {string} to finish")]
#[then(expr = "I wait for the legacy execution {string} to finish")]
async fn wait_legacy_execution(w: &mut R8rWorld, var: String) {
    let id = w.var(&var);
    for _ in 0..100 {
        let r = w.request("GET", &format!("/rest/r8r/executions/{id}"), &[], None).await;
        if r.json()["status"] != Value::from("Running") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("legacy execution {id} still running after 10s");
}
