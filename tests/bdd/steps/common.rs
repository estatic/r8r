//! Steps shared by every feature: users, raw HTTP, JSON assertions, and
//! remembered values. Feature files stay readable because paths and bodies
//! can reference remembered values as `{name}`.

use crate::world::AppWorld;
use cucumber::gherkin::Step;
use cucumber::{given, then, when};

fn docstring(step: &Step) -> String {
    step.docstring.clone().expect("this step needs a \"\"\" doc string \"\"\"")
}

fn parse_expected(raw: &str) -> serde_json::Value {
    // Accept JSON literals ("x", 1, true, {...}); bare words are strings.
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

// ---- users -------------------------------------------------------------

#[given(expr = "I am logged in as {string}")]
async fn logged_in_as(w: &mut AppWorld, email: String) {
    let body = serde_json::json!({"email": email, "password": "correct-horse-battery"}).to_string();
    w.request("POST", "/rest/auth/register", Some(body), &[]).await;
    if w.last_status.map(|s| s.as_u16()) != Some(201) {
        let body = serde_json::json!({"email": email, "password": "correct-horse-battery"}).to_string();
        w.request("POST", "/rest/auth/login", Some(body), &[]).await;
    }
    let token = w.last_body["token"].as_str().unwrap_or_else(|| panic!("no token: {}", w.last_text)).to_string();
    w.token = Some(token);
}

#[given("I am not logged in")]
async fn not_logged_in(w: &mut AppWorld) {
    let _ = w.app().await;
    w.token = None;
}

#[given(expr = "my session token is {string}")]
async fn session_token(w: &mut AppWorld, token: String) {
    let _ = w.app().await;
    w.token = Some(token);
}

// ---- requests ----------------------------------------------------------

#[when(regex = r#"^I send a (GET|POST|PUT|PATCH|DELETE) request to "([^"]*)"$"#)]
async fn send(w: &mut AppWorld, method: String, path: String) {
    w.request(&method, &path, None, &[]).await;
}

#[when(regex = r#"^I send a (GET|POST|PUT|PATCH|DELETE) request to "([^"]*)" with JSON:$"#)]
async fn send_json(w: &mut AppWorld, method: String, path: String, step: &Step) {
    w.request(&method, &path, Some(docstring(step)), &[]).await;
}

#[given(regex = r#"^I (?:have )?sent a (GET|POST|PUT|PATCH|DELETE) request to "([^"]*)" with JSON:$"#)]
async fn given_sent_json(w: &mut AppWorld, method: String, path: String, step: &Step) {
    w.request(&method, &path, Some(docstring(step)), &[]).await;
}

#[when(regex = r#"^I send a (GET|POST|PUT|PATCH|DELETE) request to "([^"]*)" with header "([^"]*)" set to "([^"]*)"$"#)]
async fn send_with_header(w: &mut AppWorld, method: String, path: String, name: String, value: String) {
    let value = w.expand(&value);
    w.request(&method, &path, None, &[(name.as_str(), value)]).await;
}

// ---- remembering values ------------------------------------------------

#[then(regex = r#"^I remember the JSON at "([^"]*)" as "([^"]*)"$"#)]
#[when(regex = r#"^I remember the JSON at "([^"]*)" as "([^"]*)"$"#)]
#[given(regex = r#"^I remember the JSON at "([^"]*)" as "([^"]*)"$"#)]
async fn remember(w: &mut AppWorld, pointer: String, name: String) {
    let value = w.at(&pointer).unwrap_or_else(|| panic!("nothing at {pointer} in {}", w.last_text)).clone();
    let text = match value {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    w.vars.insert(name, text);
}

// ---- assertions --------------------------------------------------------

#[then(expr = "the response status is {int}")]
async fn status_is(w: &mut AppWorld, expected: u16) {
    assert_eq!(w.last_status.map(|s| s.as_u16()), Some(expected), "body: {}", w.last_text);
}

#[then(regex = r#"^the JSON at "([^"]*)" is (.+)$"#)]
async fn json_at_is(w: &mut AppWorld, pointer: String, raw: String) {
    let expected = parse_expected(&w.expand(&raw));
    assert_eq!(w.at(&pointer), Some(&expected), "body: {}", w.last_text);
}

#[then(regex = r#"^the JSON at "([^"]*)" equals:$"#)]
async fn json_at_equals(w: &mut AppWorld, pointer: String, step: &Step) {
    let expected: serde_json::Value = serde_json::from_str(&w.expand(&docstring(step))).expect("valid JSON doc string");
    assert_eq!(w.at(&pointer), Some(&expected), "body: {}", w.last_text);
}

#[then(regex = r#"^the JSON at "([^"]*)" contains:$"#)]
async fn json_at_contains(w: &mut AppWorld, pointer: String, step: &Step) {
    let expected: serde_json::Value = serde_json::from_str(&w.expand(&docstring(step))).expect("valid JSON doc string");
    let actual = w.at(&pointer).unwrap_or_else(|| panic!("nothing at {pointer} in {}", w.last_text));
    assert!(json_contains(actual, &expected), "{actual} does not contain {expected}");
}

#[then(regex = r#"^there is no JSON at "([^"]*)"$"#)]
async fn json_absent(w: &mut AppWorld, pointer: String) {
    assert!(w.at(&pointer).is_none(), "unexpected value at {pointer}: {}", w.last_text);
}

#[then(regex = r#"^the JSON at "([^"]*)" has (\d+) items?$"#)]
async fn json_len(w: &mut AppWorld, pointer: String, n: usize) {
    let arr = w.at(&pointer).and_then(|v| v.as_array()).unwrap_or_else(|| panic!("no array at {pointer}: {}", w.last_text));
    assert_eq!(arr.len(), n, "body: {}", w.last_text);
}

#[then(expr = "the response body contains {string}")]
async fn body_contains(w: &mut AppWorld, text: String) {
    let text = w.expand(&text);
    assert!(w.last_text.contains(&text), "{:?} not in {}", text, w.last_text);
}

#[then(expr = "the response body does not contain {string}")]
async fn body_not_contains(w: &mut AppWorld, text: String) {
    let text = w.expand(&text);
    assert!(!w.last_text.contains(&text), "{:?} unexpectedly in {}", text, w.last_text);
}

/// `expected` is a subset of `actual`: every object key present with a
/// matching value, arrays matched element-wise by index prefix.
pub fn json_contains(actual: &serde_json::Value, expected: &serde_json::Value) -> bool {
    use serde_json::Value::*;
    match (actual, expected) {
        (Object(a), Object(e)) => e.iter().all(|(k, v)| a.get(k).is_some_and(|av| json_contains(av, v))),
        (Array(a), Array(e)) => e.len() <= a.len() && e.iter().zip(a).all(|(ev, av)| json_contains(av, ev)),
        _ => actual == expected,
    }
}

// ---- executions --------------------------------------------------------

#[when(expr = "I wait for execution {string} to finish")]
#[then(expr = "I wait for execution {string} to finish")]
async fn wait_for_execution(w: &mut AppWorld, var: String) {
    let id = w.vars.get(&var).unwrap_or_else(|| panic!("no remembered {var}")).clone();
    for _ in 0..250 {
        w.request("GET", &format!("/rest/executions/{id}"), None, &[]).await;
        if w.last_body["status"] != "Running" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("execution {id} still running after 5s");
}
