//! External services, faked with wiremock. `%{MOCK_URL}` is its base URL.

use super::docstring;
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::world::{pretty, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then};
use serde_json::{json, Value};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

pub async fn mock(w: &mut R8rWorld) -> &MockServer {
    if w.mock.is_none() {
        w.mock = Some(MockServer::start().await);
    }
    w.mock.as_ref().unwrap()
}

fn response(status: u16, body: Option<&str>) -> ResponseTemplate {
    let t = ResponseTemplate::new(status);
    match body.map(str::trim) {
        None | Some("") => t,
        Some(b) => match serde_json::from_str::<Value>(b) {
            Ok(v) => t.set_body_json(v),
            Err(_) => t.set_body_string(b.to_string()).insert_header("content-type", "text/plain"),
        },
    }
}

#[given(expr = "a mock HTTP service")]
async fn start(w: &mut R8rWorld) {
    mock(w).await;
}

#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with status (\d+)$"#)]
async fn respond(w: &mut R8rWorld, m: String, p: String, status: u16) {
    Mock::given(method(m.as_str())).and(path(p)).respond_with(response(status, None)).mount(mock(w).await).await;
}

#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with status (\d+) and body:$"#)]
async fn respond_body(w: &mut R8rWorld, m: String, p: String, status: u16, step: &Step) {
    let body = w.expand(docstring(step));
    Mock::given(method(m.as_str())).and(path(p)).respond_with(response(status, Some(&body))).mount(mock(w).await).await;
}

/// Takes priority over plain responses for the first `times` calls.
#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with status (\d+) the first (\d+) times?$"#)]
async fn respond_n(w: &mut R8rWorld, m: String, p: String, status: u16, times: u64) {
    Mock::given(method(m.as_str()))
        .and(path(p))
        .respond_with(response(status, Some(r#"{"message":"temporary failure"}"#)))
        .up_to_n_times(times)
        .with_priority(1)
        .mount(mock(w).await)
        .await;
}

#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with status (\d+) after (\d+) ms$"#)]
async fn respond_delay(w: &mut R8rWorld, m: String, p: String, status: u16, ms: u64) {
    Mock::given(method(m.as_str()))
        .and(path(p))
        .respond_with(response(status, Some(r#"{"ok":true}"#)).set_delay(Duration::from_millis(ms)))
        .mount(mock(w).await)
        .await;
}

async fn requests_to(w: &mut R8rWorld, p: &str) -> Vec<Request> {
    let all = mock(w).await.received_requests().await.unwrap_or_default();
    all.into_iter().filter(|r| r.url.path() == p).collect()
}

#[then(regex = r#"^the mock service received (\d+) requests? to "([^"]*)"$"#)]
async fn received_n(w: &mut R8rWorld, n: usize, p: String) {
    let got = requests_to(w, &p).await;
    assert_eq!(got.len(), n, "requests to {p}: {:?}", got.iter().map(|r| r.url.to_string()).collect::<Vec<_>>());
}

#[then(expr = "the mock service received no requests")]
async fn received_none(w: &mut R8rWorld) {
    let all = mock(w).await.received_requests().await.unwrap_or_default();
    assert!(all.is_empty(), "requests: {:?}", all.iter().map(|r| r.url.to_string()).collect::<Vec<_>>());
}

async fn last_to(w: &mut R8rWorld, p: &str) -> Request {
    requests_to(w, p).await.pop().unwrap_or_else(|| panic!("no request to {p}"))
}

#[then(expr = "the last request to {string} had the header {string} equal to {string}")]
async fn last_header(w: &mut R8rWorld, p: String, name: String, value: String) {
    let r = last_to(w, &p).await;
    let got = r.headers.get(name.as_str()).and_then(|v| v.to_str().ok()).unwrap_or("<absent>").to_string();
    assert_eq!(got, w.expand(&value), "header {name}");
}

#[then(expr = "the last request to {string} had the query parameter {string} equal to {string}")]
async fn last_query(w: &mut R8rWorld, p: String, name: String, value: String) {
    let r = last_to(w, &p).await;
    let got = r.url.query_pairs().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
    assert_eq!(got.as_deref(), Some(value.as_str()), "url: {}", r.url);
}

#[then(expr = "the last request to {string} had a JSON body matching:")]
async fn last_body(w: &mut R8rWorld, p: String, step: &Step) {
    let r = last_to(w, &p).await;
    let actual: Value = serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("body is not JSON: {}", String::from_utf8_lossy(&r.body)));
    let expected = parse_strict(&w.expand(docstring(step)), "expected body");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nbody: {}", pretty(&actual)));
}

#[then(regex = r#"^the (\d+)(?:st|nd|rd|th) request to "([^"]*)" had the header "([^"]*)" equal to "([^"]*)"$"#)]
async fn nth_header(w: &mut R8rWorld, n: usize, p: String, name: String, value: String) {
    let all = requests_to(w, &p).await;
    let r = all.get(n - 1).unwrap_or_else(|| panic!("only {} requests to {p}", all.len()));
    let got = r.headers.get(name.as_str()).and_then(|v| v.to_str().ok()).unwrap_or("<absent>");
    assert_eq!(got, value);
}

// ---- OAuth2 token endpoint ---------------------------------------------

#[given(expr = "the mock service issues the OAuth2 access tokens {string} in order at {string}")]
async fn oauth_tokens(w: &mut R8rWorld, tokens: String, p: String) {
    let tokens: Vec<String> = tokens.split(',').map(|t| t.trim().to_string()).collect();
    let server = mock(w).await;
    for (i, token) in tokens.iter().enumerate() {
        let body = json!({"access_token": token, "token_type": "Bearer", "expires_in": 3600, "refresh_token": format!("refresh-{i}")});
        let m = Mock::given(method("POST")).and(path(p.as_str())).respond_with(ResponseTemplate::new(200).set_body_json(body));
        let m = if i + 1 < tokens.len() { m.up_to_n_times(1) } else { m };
        m.with_priority((i + 1) as u8).mount(server).await;
    }
}

/// Responds 401 unless the request carries `Authorization: Bearer <token>`.
#[given(expr = "the mock service only accepts the bearer token {string} on GET {string}")]
async fn bearer_only(w: &mut R8rWorld, token: String, p: String) {
    let server = mock(w).await;
    Mock::given(method("GET"))
        .and(path(p.as_str()))
        .and(wiremock::matchers::header("authorization", format!("Bearer {token}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(p.as_str()))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error": "invalid_token"})))
        .with_priority(10)
        .mount(server)
        .await;
}

// ---- OpenAI-compatible chat API ------------------------------------------

/// Doc string: a JSON array of assistant messages returned in order by
/// `POST /v1/chat/completions` (the last one repeats). Each reply reports
/// usage of 10 prompt + 5 completion tokens.
#[given(expr = "a mock OpenAI API that replies in order:")]
async fn openai(w: &mut R8rWorld, step: &Step) {
    let replies = parse_strict(docstring(step), "assistant messages");
    let replies = replies.as_array().expect("array of messages").clone();
    let server = mock(w).await;
    let count = replies.len();
    for (i, message) in replies.into_iter().enumerate() {
        let finish = if message.get("tool_calls").is_some() { "tool_calls" } else { "stop" };
        let body = json!({
            "id": format!("chatcmpl-{i}"),
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "gpt-4o-mini",
            "choices": [{"index": 0, "message": message, "finish_reason": finish}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let m = Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body));
        let m = if i + 1 < count { m.up_to_n_times(1) } else { m };
        m.with_priority((i + 1) as u8).mount(server).await;
    }
}

#[then(regex = r#"^the mock OpenAI API received (\d+) chat requests?$"#)]
async fn openai_count(w: &mut R8rWorld, n: usize) {
    assert_eq!(requests_to(w, "/v1/chat/completions").await.len(), n);
}

#[then(expr = "chat request {int} contains a {string} message containing {string}")]
async fn openai_message(w: &mut R8rWorld, n: usize, role: String, needle: String) {
    let all = requests_to(w, "/v1/chat/completions").await;
    let r = all.get(n - 1).unwrap_or_else(|| panic!("only {} chat requests", all.len()));
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    let found = body["messages"].as_array().into_iter().flatten().any(|m| {
        m["role"] == role.as_str() && serde_json::to_string(&m["content"]).unwrap_or_default().contains(&needle)
    });
    assert!(found, "no {role} message containing {needle:?} in: {}", pretty(&body["messages"]));
}

#[then(expr = "chat request {int} offers the tool {string}")]
async fn openai_tool(w: &mut R8rWorld, n: usize, tool: String) {
    let all = requests_to(w, "/v1/chat/completions").await;
    let r = all.get(n - 1).unwrap_or_else(|| panic!("only {} chat requests", all.len()));
    let body: Value = serde_json::from_slice(&r.body).unwrap();
    let found = body["tools"].as_array().into_iter().flatten().any(|t| t["function"]["name"] == tool.as_str());
    assert!(found, "tool {tool} not offered: {}", pretty(&body["tools"]));
}

