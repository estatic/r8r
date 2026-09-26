//! Generic HTTP requests and response assertions.

use super::{docstring, table};
use crate::support::json::{assert_matches, lookup, parse_strict, Mode};
use crate::world::{pretty, Auth, R8rWorld};
use cucumber::gherkin::Step;
use cucumber::{given, then, when};
use serde_json::Value;

#[given(expr = "I am not authenticated")]
#[when(expr = "I am not authenticated")]
async fn unauthenticated(w: &mut R8rWorld) {
    w.auth = Auth::None;
}

#[given(expr = "I am logged in as the owner")]
#[when(expr = "I am logged in as the owner")]
async fn as_owner(w: &mut R8rWorld) {
    w.auth = Auth::Session("owner".into());
}

#[given(expr = "I am logged in as {string}")]
#[when(expr = "I am logged in as {string}")]
async fn as_user(w: &mut R8rWorld, user: String) {
    w.auth = Auth::Session(user);
}

#[given(expr = "I use the owner's API key")]
#[when(expr = "I use the owner's API key")]
async fn owner_key(w: &mut R8rWorld) {
    w.auth = Auth::ApiKey("owner".into());
}

#[given(expr = "I use the API key of {string}")]
#[when(expr = "I use the API key of {string}")]
async fn user_key(w: &mut R8rWorld, user: String) {
    w.auth = Auth::ApiKey(user);
}

#[given(expr = "I use the API key {string}")]
#[when(expr = "I use the API key {string}")]
async fn literal_key(w: &mut R8rWorld, key: String) {
    w.api_keys.insert("literal".into(), key);
    w.auth = Auth::ApiKey("literal".into());
}

#[given(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)"$"#)]
#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)"$"#)]
async fn send(w: &mut R8rWorld, method: String, path: String) {
    w.request(&method, &path, &[], None).await;
}

#[given(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with body:$"#)]
#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with body:$"#)]
async fn send_body(w: &mut R8rWorld, method: String, path: String, step: &Step) {
    w.request(&method, &path, &[], Some(docstring(step).to_string())).await;
}

/// Table of header | value; an optional doc string is the body.
#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with headers:$"#)]
async fn send_headers(w: &mut R8rWorld, method: String, path: String, step: &Step) {
    let headers: Vec<(String, String)> = table(step).iter().map(|r| (r[0].clone(), r[1].clone())).collect();
    w.request(&method, &path, &headers, step.docstring.clone()).await;
}

#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with content type "([^"]*)" and body "([^"]*)"$"#)]
async fn send_raw(w: &mut R8rWorld, method: String, path: String, content_type: String, body: String) {
    w.request(&method, &path, &[("content-type".into(), content_type)], Some(body)).await;
}

#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with a JSON body of (\d+) (KiB|MiB)$"#)]
async fn send_big(w: &mut R8rWorld, method: String, path: String, size: usize, unit: String) {
    let bytes = size * if unit == "MiB" { 1024 * 1024 } else { 1024 };
    let body = format!("{{\"blob\":\"{}\"}}", "x".repeat(bytes));
    w.request(&method, &path, &[], Some(body)).await;
}

/// Table of header | value, sent with the next request only (Gherkin allows
/// a step one argument, so this pairs with "... with body:").
#[given(expr = "the next request has the headers:")]
#[when(expr = "the next request has the headers:")]
async fn next_headers(w: &mut R8rWorld, step: &Step) {
    w.next_headers = table(step).iter().map(|r| (r[0].clone(), r[1].clone())).collect();
}

/// Signs `{"sub": "bdd"}` with HS256 and sends it as a bearer token.
#[when(regex = r#"^I send an? ([A-Z]+) request to "([^"]*)" with a bearer JWT signed with "([^"]*)"$"#)]
async fn send_jwt(w: &mut R8rWorld, method: String, path: String, secret: String) {
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &serde_json::json!({"sub": "bdd"}),
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .unwrap();
    w.request(&method, &path, &[("authorization".into(), format!("Bearer {token}"))], None).await;
}

/// Posts `multipart/form-data`, as n8n's hosted forms do. Table: field | value.
#[when(expr = "I submit the form at {string} with the fields:")]
async fn submit_form(w: &mut R8rWorld, path: String, step: &Step) {
    let boundary = format!("----r8rbdd{}", uuid::Uuid::new_v4().simple());
    let mut body = String::new();
    for row in table(step) {
        body.push_str(&format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", row[0], row[1]));
    }
    body.push_str(&format!("--{boundary}--\r\n"));
    let content_type = format!("multipart/form-data; boundary={boundary}");
    w.request("POST", &path, &[("content-type".into(), content_type)], Some(body)).await;
}

#[then(expr = "the response status is {int}")]
async fn status_is(w: &mut R8rWorld, status: u16) {
    let r = w.response();
    assert_eq!(r.status, status, "{}", r.describe());
}

#[then(expr = "the response status is one of {string}")]
async fn status_one_of(w: &mut R8rWorld, statuses: String) {
    let r = w.response();
    let ok = statuses.split(',').any(|s| s.trim().parse::<u16>().ok() == Some(r.status));
    assert!(ok, "expected one of [{statuses}]: {}", r.describe());
}

#[then(expr = "the response status is a client error")]
async fn client_error(w: &mut R8rWorld) {
    let r = w.response();
    assert!((400..500).contains(&r.status), "expected 4xx: {}", r.describe());
}

#[then(expr = "the response status is a success")]
async fn success(w: &mut R8rWorld) {
    let r = w.response();
    assert!((200..300).contains(&r.status), "expected 2xx: {}", r.describe());
}

#[then(expr = "the response JSON matches:")]
async fn json_matches(w: &mut R8rWorld, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected JSON");
    let actual = w.response().json();
    assert_matches(&expected, &actual, Mode::Subset)
        .unwrap_or_else(|e| panic!("{e}\n{}", w.response().describe()));
}

#[then(expr = "the response JSON is:")]
async fn json_is(w: &mut R8rWorld, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected JSON");
    let actual = w.response().json();
    assert_matches(&expected, &actual, Mode::Exact)
        .unwrap_or_else(|e| panic!("{e}\n{}", w.response().describe()));
}

fn at(w: &R8rWorld, path: &str) -> Value {
    let json = w.response().json();
    lookup(&json, path).cloned().unwrap_or_else(|| panic!("no value at {path:?} in response:\n{}", pretty(&json)))
}

#[then(expr = "the response JSON at {string} matches:")]
async fn json_at_matches(w: &mut R8rWorld, path: String, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected JSON");
    let actual = at(w, &path);
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nat {path}: {}", pretty(&actual)));
}

#[then(regex = r#"^the response JSON at "([^"]*)" is (.+)$"#)]
async fn json_at_is(w: &mut R8rWorld, path: String, expected: String) {
    let expected = parse_strict(&w.expand(&expected), "expected JSON");
    let actual = at(w, &path);
    assert_matches(&expected, &actual, Mode::Exact).unwrap_or_else(|e| panic!("{e}\nat {path}: {}", pretty(&actual)));
}

#[then(regex = r#"^the response JSON at "([^"]*)" has (\d+) elements?$"#)]
async fn json_at_len(w: &mut R8rWorld, path: String, len: usize) {
    let actual = at(w, &path);
    let n = actual.as_array().map(Vec::len).unwrap_or_else(|| panic!("{path} is not an array: {}", pretty(&actual)));
    assert_eq!(n, len, "at {path}: {}", pretty(&actual));
}

#[then(expr = "the response JSON at {string} contains an element matching:")]
async fn json_at_contains(w: &mut R8rWorld, path: String, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "expected element");
    let actual = at(w, &path);
    let found = actual.as_array().into_iter().flatten().any(|e| assert_matches(&expected, e, Mode::Subset).is_ok());
    assert!(found, "no element of {path} matches {}:\n{}", pretty(&expected), pretty(&actual));
}

#[then(expr = "the response JSON at {string} has no element matching:")]
async fn json_at_not_contains(w: &mut R8rWorld, path: String, step: &Step) {
    let expected = parse_strict(&w.expand(docstring(step)), "unexpected element");
    let actual = at(w, &path);
    let found = actual.as_array().into_iter().flatten().any(|e| assert_matches(&expected, e, Mode::Subset).is_ok());
    assert!(!found, "an element of {path} matches {}:\n{}", pretty(&expected), pretty(&actual));
}

/// The response is a list of node type descriptions (`/types/nodes.json`);
/// every type named in the doc string (one per line) must be present.
#[then(expr = "the node types list includes:")]
async fn node_types_include(w: &mut R8rWorld, step: &Step) {
    let json = w.response().json();
    let names: std::collections::HashSet<String> =
        json.as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(String::from)).collect();
    let missing: Vec<String> = super::list(step, None).into_iter().filter(|n| !names.contains(n)).collect();
    assert!(missing.is_empty(), "{} node type(s) missing: {missing:?}", missing.len());
}

#[then(expr = "the response JSON has no key {string}")]
async fn json_no_key(w: &mut R8rWorld, path: String) {
    let json = w.response().json();
    assert!(lookup(&json, &path).is_none(), "response has {path}: {}", pretty(&json));
}

#[then(expr = "the response body contains {string}")]
async fn body_contains(w: &mut R8rWorld, needle: String) {
    let needle = w.expand(&needle);
    let r = w.response();
    assert!(r.body.contains(&needle), "body lacks {needle:?}: {}", r.describe());
}

#[then(expr = "the response body does not contain {string}")]
async fn body_not_contains(w: &mut R8rWorld, needle: String) {
    let needle = w.expand(&needle);
    let r = w.response();
    assert!(!r.body.contains(&needle), "body contains {needle:?}: {}", r.describe());
}

#[then(expr = "the response body is {string}")]
async fn body_is(w: &mut R8rWorld, expected: String) {
    let r = w.response();
    assert_eq!(r.body, expected, "{}", r.describe());
}

fn header<'a>(w: &'a R8rWorld, name: &str) -> Vec<&'a str> {
    w.response().headers.get_all(name).iter().filter_map(|v| v.to_str().ok()).collect()
}

#[then(expr = "the response header {string} is {string}")]
async fn header_is(w: &mut R8rWorld, name: String, expected: String) {
    let values = header(w, &name);
    assert!(values.contains(&expected.as_str()), "header {name}: {values:?}\n{}", w.response().describe());
}

#[then(expr = "the response header {string} contains {string}")]
async fn header_contains(w: &mut R8rWorld, name: String, needle: String) {
    let values = header(w, &name);
    assert!(values.iter().any(|v| v.contains(&needle)), "header {name}: {values:?}\n{}", w.response().describe());
}

#[then(expr = "the response has no header {string}")]
async fn no_header(w: &mut R8rWorld, name: String) {
    let values = header(w, &name);
    assert!(values.is_empty(), "header {name} present: {values:?}");
}

#[then(expr = "the response took at most {int} ms")]
async fn took(w: &mut R8rWorld, ms: u64) {
    let r = w.response();
    assert!(r.elapsed.as_millis() as u64 <= ms, "{}", r.describe());
}

#[then(expr = "the response took at least {int} ms")]
async fn took_at_least(w: &mut R8rWorld, ms: u64) {
    let r = w.response();
    assert!(r.elapsed.as_millis() as u64 >= ms, "{}", r.describe());
}

#[given(expr = "I remember the response JSON at {string} as {string}")]
#[when(expr = "I remember the response JSON at {string} as {string}")]
#[then(expr = "I remember the response JSON at {string} as {string}")]
async fn remember(w: &mut R8rWorld, path: String, name: String) {
    let v = at(w, &path);
    let s = match v {
        Value::String(s) => s,
        other => other.to_string(),
    };
    w.vars.insert(name, s);
}
