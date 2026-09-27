//! Mailpit assertions for the Send Email node's `@requires-mailpit`
//! scenarios. Mailpit runs as the `r8r-bdd-mailpit` Docker container: SMTP
//! at 127.0.0.1:1025 (no auth, no TLS required), HTTP API at
//! 127.0.0.1:8025. Scenarios run in parallel, so each uses its own unique
//! subject and looks it up by `GET /api/v1/search?query=subject:"..."`
//! rather than clearing the mailbox.

use super::{docstring, eventually};
use crate::support::json::{assert_matches, parse_strict, Mode};
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::then;
use serde_json::Value;
use std::time::Duration;

const MAILPIT_URL: &str = "http://127.0.0.1:8025";

/// Minimal query-string escaping (space, quotes, colon and the rest of the
/// non-alphanumerics): enough for subjects made of ASCII test text.
fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn find_message(client: &reqwest::Client, subject: &str) -> Option<Value> {
    let query = format!("subject:\"{subject}\"");
    let url = format!("{MAILPIT_URL}/api/v1/search?query={}", url_encode(&query));
    let resp = client.get(&url).send().await.ok()?;
    let body: Value = resp.json().await.ok()?;
    body["messages"].as_array()?.iter().find(|m| m["Subject"].as_str() == Some(subject)).cloned()
}

async fn get_message(client: &reqwest::Client, id: &str) -> Value {
    let url = format!("{MAILPIT_URL}/api/v1/message/{id}");
    let resp = client.get(&url).send().await.unwrap_or_else(|e| panic!("mailpit GET {url} failed: {e}"));
    resp.json().await.unwrap_or_else(|e| panic!("mailpit response for {url} was not JSON: {e}"))
}

fn mail_var(subject: &str) -> String {
    format!("MAIL_ID:{subject}")
}

/// Polls Mailpit (up to 15s) for a message with this exact subject, and
/// remembers its id (by subject) for later `the Mailpit message ...` steps.
#[then(expr = "Mailpit receives a message with subject {string}")]
async fn receives(w: &mut R8rWorld, subject: String) {
    let subject = w.expand(&subject);
    let client = w.http.clone();
    let msg = eventually(Duration::from_secs(15), || {
        let client = client.clone();
        let subject = subject.clone();
        async move { find_message(&client, &subject).await }
    })
    .await
    .unwrap_or_else(|| panic!("no message with subject {subject:?} arrived at Mailpit within 15s"));
    let id = msg["ID"].as_str().expect("Mailpit message has no ID").to_string();
    w.vars.insert(mail_var(&subject), id);
}

/// Waits a couple of seconds and asserts nothing with this subject shows up
/// -- used for scenarios where the node is expected to fail before send.
#[then(expr = "Mailpit never receives a message with subject {string}")]
async fn never_receives(w: &mut R8rWorld, subject: String) {
    let subject = w.expand(&subject);
    tokio::time::sleep(Duration::from_secs(2)).await;
    if find_message(&w.http, &subject).await.is_some() {
        panic!("a message with subject {subject:?} arrived at Mailpit, but none was expected");
    }
}

/// Subset-matches Mailpit's `GET /api/v1/message/{id}` body (`From`, `To`,
/// `Cc`, `Bcc`, `ReplyTo`, `Subject`, `Text`, `HTML`, `Attachments`, ...)
/// against the given JSON. Requires a prior `Mailpit receives a message
/// with subject "..."` for the same subject.
#[then(expr = "the Mailpit message with subject {string} matches:")]
async fn message_matches(w: &mut R8rWorld, subject: String, step: &Step) {
    let subject = w.expand(&subject);
    let id = w.vars.get(&mail_var(&subject)).unwrap_or_else(|| panic!("no remembered Mailpit message for subject {subject:?}; call 'Mailpit receives a message with subject \"{subject}\"' first")).clone();
    let detail = get_message(&w.http, &id).await;
    let expected = parse_strict(&w.expand(docstring(step)), "expected Mailpit message");
    if let Err(e) = assert_matches(&expected, &detail, Mode::Subset) {
        panic!("Mailpit message mismatch: {e}\nfull message: {detail}");
    }
}

/// Downloads one attachment's raw bytes (by file name) via
/// `GET /api/v1/message/{id}/part/{PartID}` and compares them as text.
#[then(expr = "the Mailpit message with subject {string} has an attachment {string} with content {string}")]
async fn attachment_content(w: &mut R8rWorld, subject: String, file_name: String, expected_content: String) {
    let subject = w.expand(&subject);
    let expected_content = w.expand(&expected_content);
    let id = w.vars.get(&mail_var(&subject)).unwrap_or_else(|| panic!("no remembered Mailpit message for subject {subject:?}")).clone();
    let detail = get_message(&w.http, &id).await;
    let attachments = detail["Attachments"].as_array().cloned().unwrap_or_default();
    let part = attachments
        .iter()
        .find(|a| a["FileName"].as_str() == Some(file_name.as_str()))
        .unwrap_or_else(|| panic!("message {subject:?} has no attachment named {file_name:?}; attachments: {attachments:?}"));
    let part_id = part["PartID"].as_str().expect("attachment has no PartID");
    let url = format!("{MAILPIT_URL}/api/v1/message/{id}/part/{part_id}");
    let resp = w.http.get(&url).send().await.unwrap_or_else(|e| panic!("mailpit GET {url} failed: {e}"));
    let bytes = resp.bytes().await.unwrap_or_else(|e| panic!("reading attachment body failed: {e}"));
    let actual = String::from_utf8_lossy(&bytes);
    assert_eq!(actual, expected_content, "attachment {file_name:?} content mismatch");
}

/// Asserts the count of attachments Mailpit recorded for the message.
#[then(regex = r#"^the Mailpit message with subject "([^"]*)" has (\d+) attachments?$"#)]
async fn attachment_count(w: &mut R8rWorld, subject: String, count: usize) {
    let subject = w.expand(&subject);
    let id = w.vars.get(&mail_var(&subject)).unwrap_or_else(|| panic!("no remembered Mailpit message for subject {subject:?}")).clone();
    let detail = get_message(&w.http, &id).await;
    let attachments = detail["Attachments"].as_array().cloned().unwrap_or_default();
    assert_eq!(attachments.len(), count, "attachment count for {subject:?}: {attachments:?}");
}
