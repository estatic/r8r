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

/// Query-param-scoped variant (e.g. GitHub's `?page=1` pagination): only
/// matches requests whose query string has `name=value`.
#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with query parameter "([^"]*)" equal to "([^"]*)" with status (\d+) and body:$"#)]
async fn respond_query_body(w: &mut R8rWorld, m: String, p: String, qname: String, qvalue: String, status: u16, step: &Step) {
    let body = w.expand(docstring(step));
    Mock::given(method(m.as_str()))
        .and(path(p.as_str()))
        .and(wiremock::matchers::query_param(qname.as_str(), qvalue.as_str()))
        .respond_with(response(status, Some(&body)))
        .mount(mock(w).await)
        .await;
}

/// As above, plus a response header (used for GitHub's `Link: <...>; rel="next"`
/// pagination header).
#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" with query parameter "([^"]*)" equal to "([^"]*)" with status (\d+), header "([^"]*)" "([^"]*)" and body:$"#)]
#[allow(clippy::too_many_arguments)]
async fn respond_query_header_body(w: &mut R8rWorld, m: String, p: String, qname: String, qvalue: String, status: u16, hname: String, hvalue: String, step: &Step) {
    let body = w.expand(docstring(step));
    let hvalue = w.expand(&hvalue);
    Mock::given(method(m.as_str()))
        .and(path(p.as_str()))
        .and(wiremock::matchers::query_param(qname.as_str(), qvalue.as_str()))
        .respond_with(response(status, Some(&body)).insert_header(hname.as_str(), hvalue.as_str()))
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

/// Gmail's `format=raw` response: wraps a doc-string MIME message (its
/// `\n`s normalized to `\r\n`) as the base64url-encoded `raw` field,
/// alongside `id`/`threadId`/`labelIds`/`sizeEstimate`, so scenarios can
/// write MIME messages as readable text instead of precomputed base64url.
#[given(regex = r#"^the mock service responds to GET "([^"]*)" with status 200 and the raw message \(id "([^"]*)", thread "([^"]*)"\):$"#)]
async fn respond_raw_message(w: &mut R8rWorld, p: String, id: String, thread_id: String, step: &Step) {
    use base64::Engine as _;
    let mime = w.expand(docstring(step)).replace('\n', "\r\n");
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mime.as_bytes());
    let body = json!({"id": id, "threadId": thread_id, "labelIds": ["INBOX"], "sizeEstimate": mime.len(), "raw": raw});
    Mock::given(method("GET")).and(path(p.as_str())).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(mock(w).await).await;
}

/// Gmail's draft `format=raw` response: wraps a doc-string MIME message as
/// `{"id": ..., "message": {"id": ..., "threadId": ..., "raw": ...}}`.
#[given(regex = r#"^the mock service responds to GET "([^"]*)" with status 200 and the raw draft \(id "([^"]*)", message id "([^"]*)", thread "([^"]*)"\):$"#)]
async fn respond_raw_draft(w: &mut R8rWorld, p: String, id: String, message_id: String, thread_id: String, step: &Step) {
    use base64::Engine as _;
    let mime = w.expand(docstring(step)).replace('\n', "\r\n");
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(mime.as_bytes());
    let body = json!({"id": id, "message": {"id": message_id, "threadId": thread_id, "raw": raw}});
    Mock::given(method("GET")).and(path(p.as_str())).respond_with(ResponseTemplate::new(200).set_body_json(body)).mount(mock(w).await).await;
}

/// Gmail (and other Google APIs) send MIME messages as a base64url-encoded
/// `raw` field in the JSON body. Decodes it so scenarios can assert on
/// headers/body text without a full MIME parser in the step definitions.
/// Finds the `raw` field directly on the body, or nested under `message`
/// (Gmail's drafts.create body shape is `{"message": {"raw": ...}}`).
fn find_raw_field(body: &Value) -> Option<&str> {
    body.get("raw").and_then(Value::as_str).or_else(|| body.get("message").and_then(|m| m.get("raw")).and_then(Value::as_str))
}

#[then(expr = "the last request to {string} has a decoded raw body containing {string}")]
async fn last_raw_contains(w: &mut R8rWorld, p: String, needle: String) {
    use base64::Engine as _;
    let r = last_to(w, &p).await;
    let body: Value = serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("body is not JSON: {}", String::from_utf8_lossy(&r.body)));
    let raw = find_raw_field(&body).unwrap_or_else(|| panic!("no 'raw' field in body: {body}"));
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).unwrap_or_else(|e| panic!("raw is not base64url: {e}"));
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let expected = w.expand(&needle);
    assert!(text.contains(expected.as_str()), "decoded raw doesn't contain {expected:?}:\n{text}");
}

#[then(expr = "the last request to {string} has a decoded raw body not containing {string}")]
async fn last_raw_not_contains(w: &mut R8rWorld, p: String, needle: String) {
    use base64::Engine as _;
    let r = last_to(w, &p).await;
    let body: Value = serde_json::from_slice(&r.body).unwrap_or_else(|_| panic!("body is not JSON: {}", String::from_utf8_lossy(&r.body)));
    let raw = find_raw_field(&body).unwrap_or_else(|| panic!("no 'raw' field in body: {body}"));
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw).unwrap_or_else(|e| panic!("raw is not base64url: {e}"));
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let expected = w.expand(&needle);
    assert!(!text.contains(expected.as_str()), "decoded raw unexpectedly contains {expected:?}:\n{text}");
}

#[then(regex = r#"^the (\d+)(?:st|nd|rd|th) request to "([^"]*)" had the header "([^"]*)" equal to "([^"]*)"$"#)]
async fn nth_header(w: &mut R8rWorld, n: usize, p: String, name: String, value: String) {
    let all = requests_to(w, &p).await;
    let r = all.get(n - 1).unwrap_or_else(|| panic!("only {} requests to {p}", all.len()));
    let got = r.headers.get(name.as_str()).and_then(|v| v.to_str().ok()).unwrap_or("<absent>");
    assert_eq!(got, value);
}

#[then(regex = r#"^the (\d+)(?:st|nd|rd|th) request to "([^"]*)" had the query parameter "([^"]*)" equal to "([^"]*)"$"#)]
async fn nth_query(w: &mut R8rWorld, n: usize, p: String, name: String, value: String) {
    let all = requests_to(w, &p).await;
    let r = all.get(n - 1).unwrap_or_else(|| panic!("only {} requests to {p}", all.len()));
    let got = r.url.query_pairs().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
    assert_eq!(got.as_deref(), Some(value.as_str()), "url: {}", r.url);
}

// ---- multipart/form-data request bodies ---------------------------------

/// Splits a `multipart/form-data` body (from its `Content-Type` boundary)
/// into `(field name, filename, content bytes)` triples, one per part.
fn multipart_parts(body: &[u8], boundary: &str) -> Vec<(String, Option<String>, Vec<u8>)> {
    fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        if from > hay.len() || needle.is_empty() {
            return None;
        }
        hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
    }
    let delim = format!("--{boundary}").into_bytes();
    let mut positions = Vec::new();
    let mut from = 0;
    while let Some(p) = find(body, &delim, from) {
        positions.push(p);
        from = p + delim.len();
    }
    let mut parts = Vec::new();
    for w in positions.windows(2) {
        let start = w[0] + delim.len();
        let end = w[1];
        let mut chunk = &body[start..end];
        if chunk.starts_with(b"--") {
            continue;
        }
        if let Some(rest) = chunk.strip_prefix(b"\r\n") {
            chunk = rest;
        }
        let Some(hpos) = find(chunk, b"\r\n\r\n", 0) else { continue };
        let header_str = String::from_utf8_lossy(&chunk[..hpos]).to_string();
        let mut content = &chunk[hpos + 4..];
        if let Some(rest) = content.strip_suffix(b"\r\n") {
            content = rest;
        }
        let mut name = String::new();
        let mut filename = None;
        for line in header_str.split("\r\n") {
            if line.to_ascii_lowercase().starts_with("content-disposition:") {
                for kv in line.split(';') {
                    let kv = kv.trim();
                    if let Some(v) = kv.strip_prefix("name=\"") {
                        name = v.trim_end_matches('"').to_string();
                    }
                    if let Some(v) = kv.strip_prefix("filename=\"") {
                        filename = Some(v.trim_end_matches('"').to_string());
                    }
                }
            }
        }
        parts.push((name, filename, content.to_vec()));
    }
    parts
}

async fn last_multipart_parts(w: &mut R8rWorld, p: &str) -> Vec<(String, Option<String>, Vec<u8>)> {
    let r = last_to(w, p).await;
    let ct = r.headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let boundary = ct.split("boundary=").nth(1).unwrap_or("").trim_matches('"').to_string();
    multipart_parts(&r.body, &boundary)
}

#[then(expr = "the last request to {string} had the multipart field {string} equal to {string}")]
async fn last_multipart_field(w: &mut R8rWorld, p: String, name: String, value: String) {
    let expected = w.expand(&value);
    let parts = last_multipart_parts(w, &p).await;
    let got = parts.iter().find(|(n, _, _)| *n == name).map(|(_, _, b)| String::from_utf8_lossy(b).to_string());
    assert_eq!(
        got.as_deref(),
        Some(expected.as_str()),
        "multipart field {name:?} in request to {p}; parts: {:?}",
        parts.iter().map(|(n, f, _)| (n.clone(), f.clone())).collect::<Vec<_>>()
    );
}

/// Like `had a JSON body matching:`, but for one `multipart/form-data` (or
/// `multipart/related`) field's decoded bytes -- used for Google Drive's
/// upload/createFromText `metadata` part, which is itself a JSON document
/// rather than the whole request body.
#[then(expr = "the last request to {string} had the multipart field {string} with a JSON body matching:")]
async fn last_multipart_field_json(w: &mut R8rWorld, p: String, name: String, step: &Step) {
    let parts = last_multipart_parts(w, &p).await;
    let got = parts.iter().find(|(n, _, _)| *n == name).map(|(_, _, b)| b.clone()).unwrap_or_else(|| panic!("no multipart field {name:?} in request to {p}"));
    let actual: Value = serde_json::from_slice(&got).unwrap_or_else(|_| panic!("multipart field {name:?} is not JSON: {}", String::from_utf8_lossy(&got)));
    let expected = parse_strict(&w.expand(docstring(step)), "expected multipart field body");
    assert_matches(&expected, &actual, Mode::Subset).unwrap_or_else(|e| panic!("{e}\nbody: {}", pretty(&actual)));
}

#[then(expr = "the last request to {string} had a multipart file field {string} with filename {string}")]
async fn last_multipart_file(w: &mut R8rWorld, p: String, name: String, filename: String) {
    let parts = last_multipart_parts(w, &p).await;
    let got = parts.iter().find(|(n, _, _)| *n == name).and_then(|(_, f, _)| f.clone());
    assert_eq!(
        got.as_deref(),
        Some(filename.as_str()),
        "multipart file field {name:?} in request to {p}; parts: {:?}",
        parts.iter().map(|(n, f, _)| (n.clone(), f.clone())).collect::<Vec<_>>()
    );
}

#[then(expr = "the last request to {string} had no multipart field {string}")]
async fn last_multipart_field_absent(w: &mut R8rWorld, p: String, name: String) {
    let parts = last_multipart_parts(w, &p).await;
    assert!(!parts.iter().any(|(n, _, _)| *n == name), "multipart field {name:?} unexpectedly present in request to {p}");
}

/// A sequence of `{"status": N, "body": {...}}` replies for one method+path,
/// returned in order (the last one repeats). Used for pagination scenarios
/// where each page's body must differ (e.g. Notion's `start_cursor`/
/// `has_more`), mirroring the priority/`up_to_n_times` technique used by
/// `oauth_tokens`/`openai` below.
#[given(regex = r#"^the mock service responds to ([A-Z]+) "([^"]*)" in order with:$"#)]
async fn respond_in_order(w: &mut R8rWorld, m: String, p: String, step: &Step) {
    let body = w.expand(docstring(step));
    let replies = parse_strict(&body, "ordered responses");
    let replies = replies.as_array().expect("array of {status, body} replies").clone();
    let server = mock(w).await;
    let count = replies.len();
    for (i, reply) in replies.into_iter().enumerate() {
        let status = reply.get("status").and_then(Value::as_u64).unwrap_or(200) as u16;
        let body_val = reply.get("body").cloned().unwrap_or(json!({}));
        let m_builder = Mock::given(method(m.as_str())).and(path(p.as_str())).respond_with(ResponseTemplate::new(status).set_body_json(body_val));
        let m_builder = if i + 1 < count { m_builder.up_to_n_times(1) } else { m_builder };
        m_builder.with_priority((i + 1) as u8).mount(server).await;
    }
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

