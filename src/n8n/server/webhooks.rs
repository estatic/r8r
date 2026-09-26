//! Webhooks, forms and resume URLs (spec §6.3): `/webhook/:path` for
//! active workflows, `/webhook-test/:path` while the editor listens,
//! `/webhook-waiting/:executionId` to resume a Wait node, and `/form/:path`
//! for form triggers. Paths, response modes, auth and CORS follow n8n.

use super::runner::{self, RunHandle, RunRequest};
use super::{ApiError, N8n};
use crate::n8n::engine::WebhookResponse;
use crate::n8n::node::Mode;
use crate::n8n::types::{status, Item};
use crate::n8n::workflow::Workflow;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Webhook,
    Form,
}

#[derive(Clone, Debug)]
pub struct Registration {
    pub workflow_id: String,
    pub node: String,
    /// Upper-case HTTP method; forms answer GET and POST.
    pub method: String,
    /// Without leading or trailing slashes; `:name` segments are parameters.
    pub path: String,
    pub kind: Kind,
}

impl Registration {
    fn segments(&self) -> Vec<&str> {
        self.path.split('/').filter(|s| !s.is_empty()).collect()
    }

    /// Path parameters when `path` matches.
    pub fn match_path(&self, path: &str) -> Option<HashMap<String, String>> {
        let want = self.segments();
        let got: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if want.len() != got.len() {
            return None;
        }
        let mut params = HashMap::new();
        for (w, g) in want.iter().zip(&got) {
            if let Some(name) = w.strip_prefix(':') {
                params.insert(name.to_string(), percent_decode(g));
            } else if w != g {
                return None;
            }
        }
        Some(params)
    }

    pub fn conflicts_with(&self, other: &Registration) -> bool {
        let shape = |r: &Registration| r.segments().iter().map(|s| if s.starts_with(':') { ":" } else { s }).collect::<Vec<_>>().join("/");
        self.kind == other.kind && self.method == other.method && shape(self) == shape(other)
    }
}

pub struct TestRegistration {
    pub workflow: Value,
    pub reg: Registration,
    pub push_ref: Option<String>,
}

fn clean_path(p: &str) -> String {
    p.trim().trim_matches('/').to_string()
}

fn methods(parameters: &Value) -> Vec<String> {
    let multiple = parameters["multipleMethods"].as_bool().unwrap_or(false);
    match &parameters["httpMethod"] {
        Value::Array(a) if multiple || !a.is_empty() => a.iter().filter_map(|m| m.as_str().map(|s| s.to_ascii_uppercase())).collect(),
        Value::String(s) => vec![s.to_ascii_uppercase()],
        _ => vec!["GET".into()],
    }
}

/// Webhook and form registrations of a workflow's trigger nodes.
pub fn registrations_for(workflow: &Workflow) -> Vec<Registration> {
    let id = workflow.id.clone().unwrap_or_default();
    let mut out = Vec::new();
    for node in workflow.nodes.iter().filter(|n| !n.disabled) {
        let webhook_id = node.raw.get("webhookId").and_then(Value::as_str).unwrap_or("").to_string();
        match node.node_type.as_str() {
            "n8n-nodes-base.webhook" => {
                let path = clean_path(node.parameters["path"].as_str().unwrap_or(""));
                // n8n registers dynamic paths (and empty ones) under the webhook id.
                let path = if path.is_empty() {
                    webhook_id.clone()
                } else if path.split('/').any(|s| s.starts_with(':')) {
                    format!("{webhook_id}/{path}")
                } else {
                    path
                };
                for method in methods(&node.parameters) {
                    out.push(Registration { workflow_id: id.clone(), node: node.name.clone(), method, path: path.clone(), kind: Kind::Webhook });
                }
            }
            "n8n-nodes-base.formTrigger" => {
                let path = [node.parameters["path"].as_str(), node.parameters.pointer("/options/path").and_then(Value::as_str)]
                    .into_iter()
                    .flatten()
                    .map(clean_path)
                    .find(|p| !p.is_empty())
                    .unwrap_or(webhook_id);
                out.push(Registration { workflow_id: id.clone(), node: node.name.clone(), method: "*".into(), path, kind: Kind::Form });
            }
            _ => {}
        }
    }
    out
}

pub fn router() -> Router<Arc<N8n>> {
    Router::new()
        .route("/webhook/*path", any(production))
        .route("/webhook-test/*path", any(test))
        .route("/webhook-waiting/:id", any(waiting))
        .route("/webhook-waiting/:id/*rest", any(waiting_with_path))
        .route("/form-waiting/:id", any(waiting))
        .route("/form/*path", get(form_page).post(form_submit))
        .route("/form-test/*path", get(form_page).post(form_submit))
}

// ---- request parsing --------------------------------------------------------

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` into an object; repeated keys become arrays, as n8n does.
pub fn parse_query(q: &str) -> Map<String, Value> {
    let mut out = Map::new();
    for pair in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (percent_decode(k), Value::String(percent_decode(v)));
        match out.get_mut(&k) {
            Some(Value::Array(a)) => a.push(v),
            Some(existing) => {
                let first = existing.take();
                *existing = Value::Array(vec![first, v]);
            }
            None => {
                out.insert(k, v);
            }
        }
    }
    out
}

struct Part {
    name: String,
    filename: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

fn parse_multipart(content_type: &str, body: &[u8]) -> Vec<Part> {
    let Some(boundary) = content_type.split(';').find_map(|p| p.trim().strip_prefix("boundary=")).map(|b| b.trim_matches('"').to_string()) else {
        return vec![];
    };
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut rest = body;
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    while let Some(start) = find(rest, &delimiter) {
        rest = &rest[start + delimiter.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        let Some(end) = find(rest, &delimiter) else { break };
        let chunk = &rest[..end];
        let chunk = chunk.strip_prefix(b"\r\n").unwrap_or(chunk);
        let chunk = chunk.strip_suffix(b"\r\n").unwrap_or(chunk);
        if let Some(split) = find(chunk, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&chunk[..split]).to_string();
            let data = chunk[split + 4..].to_vec();
            let mut part = Part { name: String::new(), filename: None, content_type: None, data };
            for line in head.lines() {
                let lower = line.to_ascii_lowercase();
                if lower.starts_with("content-disposition:") {
                    for attr in line.split(';').map(str::trim) {
                        if let Some(v) = attr.strip_prefix("name=") {
                            part.name = v.trim_matches('"').to_string();
                        } else if let Some(v) = attr.strip_prefix("filename=") {
                            part.filename = Some(v.trim_matches('"').to_string());
                        }
                    }
                } else if lower.starts_with("content-type:") {
                    part.content_type = Some(line[13..].trim().to_string());
                }
            }
            parts.push(part);
        }
        rest = &rest[end..];
    }
    parts
}

fn binary_entry(data: &[u8], mime: &str, file_name: Option<&str>) -> Value {
    let ext = mime.split('/').nth(1).unwrap_or("bin").split(';').next().unwrap_or("bin").trim().to_string();
    let mut v = json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": format!("{} B", data.len()),
    });
    if let Some(f) = file_name {
        v["fileName"] = json!(f);
    }
    v
}

/// The request body as n8n puts it in the item: parsed JSON, form fields,
/// text, or binary data.
fn parse_body(headers: &HeaderMap, body: &[u8], binary_property: Option<&str>) -> (Value, Option<Map<String, Value>>) {
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if body.is_empty() {
        return (json!({}), None);
    }
    if mime == "application/json" || mime.ends_with("+json") {
        return (serde_json::from_slice(body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(body).into_owned())), None);
    }
    if mime == "application/x-www-form-urlencoded" {
        return (Value::Object(parse_query(&String::from_utf8_lossy(body))), None);
    }
    if mime == "multipart/form-data" {
        let mut fields = Map::new();
        let mut binary = Map::new();
        for part in parse_multipart(&content_type, body) {
            if part.filename.is_some() {
                let mime = part.content_type.clone().unwrap_or_else(|| "application/octet-stream".into());
                binary.insert(part.name.clone(), binary_entry(&part.data, &mime, part.filename.as_deref()));
            } else {
                fields.insert(part.name, Value::String(String::from_utf8_lossy(&part.data).into_owned()));
            }
        }
        return (Value::Object(fields), (!binary.is_empty()).then_some(binary));
    }
    if let Some(prop) = binary_property.filter(|p| !p.is_empty()) {
        if !mime.starts_with("text/") || mime.is_empty() {
            let mut binary = Map::new();
            let mime = if mime.is_empty() { "application/octet-stream".to_string() } else { mime };
            binary.insert(prop.to_string(), binary_entry(body, &mime, None));
            return (json!({}), Some(binary));
        }
    }
    if mime.starts_with("text/") || mime.is_empty() || mime.contains("xml") {
        return (Value::String(String::from_utf8_lossy(body).into_owned()), None);
    }
    let mut binary = Map::new();
    binary.insert(binary_property.unwrap_or("data").to_string(), binary_entry(body, &mime, None));
    (json!({}), Some(binary))
}

fn headers_json(headers: &HeaderMap, hidden: &[String]) -> Map<String, Value> {
    let mut out = Map::new();
    for (k, v) in headers {
        let name = k.as_str().to_ascii_lowercase();
        if hidden.contains(&name) {
            continue;
        }
        out.insert(name, Value::String(v.to_str().unwrap_or("").to_string()));
    }
    out
}

struct Incoming {
    method: Method,
    query: Map<String, Value>,
    headers: HeaderMap,
    body: Vec<u8>,
}

async fn read(n8n: &N8n, req: Request) -> Result<Incoming, Response> {
    let (parts, body) = req.into_parts();
    let body = axum::body::to_bytes(body, n8n.payload_limit).await.map_err(|_| {
        (StatusCode::PAYLOAD_TOO_LARGE, Json(json!({"code": 413, "message": "The request body is larger than the configured payload limit (N8N_PAYLOAD_SIZE_MAX)"})))
            .into_response()
    })?;
    Ok(Incoming { method: parts.method, query: parts.uri.query().map(parse_query).unwrap_or_default(), headers: parts.headers, body: body.to_vec() })
}

fn json_response(status: u16, body: Value) -> Response {
    (StatusCode::from_u16(status).unwrap_or(StatusCode::OK), Json(body)).into_response()
}

fn not_registered(method: &Method, path: &str, test: bool) -> Response {
    let hint = if test {
        "Click the 'Execute workflow' button on the canvas, then try again. (In test mode, the webhook only works for one call after you click this button)"
    } else {
        "The workflow must be active for a production URL to run successfully. You can activate the workflow using the toggle in the top-right of the editor."
    };
    json_response(404, json!({"code": 404, "message": format!("The requested webhook \"{method} {path}\" is not registered."), "hint": hint}))
}

// ---- CORS -------------------------------------------------------------------

fn allowed_origin(parameters: &Value, origin: &str) -> Option<String> {
    let allowed = parameters.pointer("/options/allowedOrigins").and_then(Value::as_str).unwrap_or("*");
    if allowed.trim() == "*" {
        return Some("*".into());
    }
    allowed.split(',').map(str::trim).any(|o| o == origin).then(|| origin.to_string())
}

fn add_cors(mut resp: Response, parameters: &Value, headers: &HeaderMap) -> Response {
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        if let Some(allow) = allowed_origin(parameters, origin) {
            if let Ok(v) = HeaderValue::from_str(&allow) {
                resp.headers_mut().insert("access-control-allow-origin", v);
            }
        }
    }
    resp
}

fn preflight(parameters: &Value, headers: &HeaderMap, methods: &[String]) -> Response {
    let origin = headers.get("origin").and_then(|v| v.to_str().ok()).unwrap_or("");
    let mut resp = StatusCode::NO_CONTENT.into_response();
    if let Some(allow) = allowed_origin(parameters, origin) {
        let h = resp.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&allow) {
            h.insert("access-control-allow-origin", v);
        }
        let mut m = methods.to_vec();
        m.push("OPTIONS".into());
        if let Ok(v) = HeaderValue::from_str(&m.join(", ")) {
            h.insert("access-control-allow-methods", v);
        }
        let req_headers = headers.get("access-control-request-headers").and_then(|v| v.to_str().ok()).unwrap_or("*").to_string();
        if let Ok(v) = HeaderValue::from_str(&req_headers) {
            h.insert("access-control-allow-headers", v);
        }
        h.insert("access-control-max-age", HeaderValue::from_static("300"));
    }
    resp
}

// ---- authentication ---------------------------------------------------------

/// Checks the node's `authentication`. On success returns the header names
/// to leave out of the item (they carry the credential).
async fn authenticate(n8n: &N8n, node: &Value, headers: &HeaderMap) -> Result<Vec<String>, Response> {
    let method = node.pointer("/parameters/authentication").and_then(Value::as_str).unwrap_or("none");
    let cred_type = match method {
        "basicAuth" => "httpBasicAuth",
        "headerAuth" => "httpHeaderAuth",
        "jwtAuth" => "jwtAuth",
        _ => return Ok(vec![]),
    };
    let no_data = || json_response(500, json!({"code": 500, "message": "No authentication data defined on node!"}));
    let reference = node.pointer(&format!("/credentials/{cred_type}")).ok_or_else(no_data)?;
    let record = match reference["id"].as_str() {
        Some(id) => n8n.store.get_credential(id).await.ok().flatten(),
        None => None,
    };
    let record = match record {
        Some(r) => r,
        None => {
            let name = reference["name"].as_str().unwrap_or_default();
            n8n.store.list_credentials().await.unwrap_or_default().into_iter().find(|c| c.name == name && c.cred_type == cred_type).ok_or_else(no_data)?
        }
    };
    let cred = n8n.store.decrypt_credential(&record).await.map_err(|_| no_data())?;
    let wrong = |status: u16| json_response(status, json!({"code": status, "message": "Authorization data is wrong!"}));
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(String::from);
    match method {
        "basicAuth" => {
            let challenge = |message: &str| {
                let mut r = json_response(401, json!({"code": 401, "message": message}));
                r.headers_mut().insert("www-authenticate", HeaderValue::from_static("Basic realm=\"Webhook\""));
                r
            };
            let Some(auth) = header("authorization") else { return Err(challenge("Authorization is required!")) };
            let decoded = auth
                .strip_prefix("Basic ")
                .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b.trim()).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let expected = format!("{}:{}", cred["user"].as_str().unwrap_or(""), cred["password"].as_str().unwrap_or(""));
            if decoded != expected {
                return Err(challenge("Authorization data is wrong!"));
            }
            Ok(vec!["authorization".into()])
        }
        "headerAuth" => {
            let name = cred["name"].as_str().unwrap_or("").to_ascii_lowercase();
            if name.is_empty() || header(&name).as_deref() != cred["value"].as_str() {
                return Err(wrong(403));
            }
            Ok(vec![name])
        }
        _ => {
            let token = header("authorization").and_then(|a| a.strip_prefix("Bearer ").map(|t| t.trim().to_string())).ok_or_else(|| wrong(403))?;
            let alg: jsonwebtoken::Algorithm = cred["algorithm"].as_str().unwrap_or("HS256").parse().unwrap_or(jsonwebtoken::Algorithm::HS256);
            let key = if cred["keyType"].as_str() == Some("pemKey") {
                let pem = cred["publicKey"].as_str().unwrap_or("").as_bytes().to_vec();
                jsonwebtoken::DecodingKey::from_rsa_pem(&pem).or_else(|_| jsonwebtoken::DecodingKey::from_ec_pem(&pem)).map_err(|_| wrong(403))?
            } else {
                jsonwebtoken::DecodingKey::from_secret(cred["secret"].as_str().unwrap_or("").as_bytes())
            };
            let mut validation = jsonwebtoken::Validation::new(alg);
            validation.required_spec_claims.clear();
            validation.validate_exp = false;
            jsonwebtoken::decode::<Value>(&token, &key, &validation).map_err(|_| wrong(403))?;
            Ok(vec!["authorization".into()])
        }
    }
}

// ---- responding ---------------------------------------------------------------

fn response_code(parameters: &Value) -> u16 {
    let candidates = [
        parameters.pointer("/options/responseCode/values/responseCode"),
        parameters.pointer("/options/responseCode"),
        parameters.get("responseCode"),
    ];
    candidates.into_iter().flatten().find_map(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|c| c as u16).unwrap_or(200)
}

fn from_node_response(r: WebhookResponse) -> Response {
    let mut resp = Response::new(Body::from(r.body));
    *resp.status_mut() = StatusCode::from_u16(r.status).unwrap_or(StatusCode::OK);
    for (k, v) in r.headers {
        if let (Ok(name), Ok(value)) = (axum::http::HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(&v)) {
            resp.headers_mut().insert(name, value);
        }
    }
    resp
}

fn with_response_headers(mut resp: Response, parameters: &Value) -> Response {
    for h in parameters.pointer("/options/responseHeaders/entries").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(k), Some(v)) = (h["name"].as_str(), h["value"].as_str()) {
            if let (Ok(name), Ok(value)) = (axum::http::HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                resp.headers_mut().insert(name, value);
            }
        }
    }
    resp
}

/// Answers the caller according to the trigger's `responseMode`.
async fn respond(parameters: &Value, handle: RunHandle, node_response: Option<tokio::sync::oneshot::Receiver<WebhookResponse>>) -> Response {
    let code = response_code(parameters);
    let mode = parameters["responseMode"].as_str().unwrap_or("onReceived");
    let resp = match mode {
        "lastNode" => {
            let Ok(outcome) = handle.done.await else { return json_response(500, json!({"message": "Error in workflow"})) };
            if outcome.status == status::ERROR || outcome.status == status::CANCELED || outcome.status == status::CRASHED {
                return json_response(500, json!({"code": 0, "message": "Error in workflow"}));
            }
            if outcome.status == status::WAITING {
                return json_response(code, json!({"message": "Workflow was started"}));
            }
            let items = outcome.last_output();
            match parameters["responseData"].as_str().unwrap_or("firstEntryJson") {
                "allEntries" => json_response(code, Value::Array(items.iter().map(Item::json_value).collect())),
                "noData" => (StatusCode::from_u16(code).unwrap_or(StatusCode::OK), "").into_response(),
                "firstEntryBinary" => {
                    let prop = parameters.pointer("/options/binaryPropertyName").and_then(Value::as_str).unwrap_or("data");
                    match items.first().and_then(|i| i.binary.as_ref()).and_then(|b| b.get(prop)) {
                        Some(bin) => {
                            let bytes = base64::engine::general_purpose::STANDARD.decode(bin["data"].as_str().unwrap_or("")).unwrap_or_default();
                            let mime = bin["mimeType"].as_str().unwrap_or("application/octet-stream").to_string();
                            (StatusCode::from_u16(code).unwrap_or(StatusCode::OK), [("content-type", mime)], bytes).into_response()
                        }
                        None => json_response(500, json!({"code": 0, "message": "No binary data to return was found"})),
                    }
                }
                _ => match items.first() {
                    Some(item) => json_response(code, item.json_value()),
                    None => json_response(500, json!({"code": 0, "message": "No item to return was found"})),
                },
            }
        }
        "responseNode" => {
            let Some(mut rx) = node_response else { return json_response(500, json!({"message": "Error in workflow"})) };
            let mut done = handle.done;
            tokio::select! {
                r = &mut rx => match r {
                    Ok(r) => return from_node_response(r),
                    Err(_) => {
                        let outcome = done.await;
                        match outcome {
                            Ok(o) if o.status == status::SUCCESS => json_response(200, json!({"message": "Workflow executed successfully"})),
                            _ => json_response(500, json!({"code": 0, "message": "Error in workflow"})),
                        }
                    }
                },
                outcome = &mut done => {
                    if let Ok(r) = rx.try_recv() {
                        return from_node_response(r);
                    }
                    match outcome {
                        Ok(o) if o.status == status::SUCCESS => json_response(200, json!({"message": "Workflow executed successfully"})),
                        _ => json_response(500, json!({"code": 0, "message": "Error in workflow"})),
                    }
                }
            }
        }
        _ => {
            if parameters.pointer("/options/noResponseBody").and_then(Value::as_bool).unwrap_or(false) {
                (StatusCode::from_u16(code).unwrap_or(StatusCode::OK), "").into_response()
            } else {
                json_response(code, json!({"message": "Workflow was started"}))
            }
        }
    };
    with_response_headers(resp, parameters)
}

// ---- handlers -----------------------------------------------------------------

/// Runs `reg`'s workflow for a webhook call and answers it.
#[allow(clippy::too_many_arguments)]
async fn run_webhook(
    n8n: &Arc<N8n>,
    workflow: Value,
    reg: &Registration,
    params: HashMap<String, String>,
    incoming: Incoming,
    path: &str,
    test: bool,
    push_ref: Option<String>,
) -> Response {
    let Some(node) = workflow["nodes"].as_array().and_then(|nodes| nodes.iter().find(|n| n["name"] == reg.node.as_str())).cloned() else {
        return not_registered(&incoming.method, path, test);
    };
    let parameters = node["parameters"].clone();
    let hidden = match authenticate(n8n, &node, &incoming.headers).await {
        Ok(h) => h,
        Err(resp) => return add_cors(resp, &parameters, &incoming.headers),
    };
    let binary_property = parameters.pointer("/options/binaryPropertyName").and_then(Value::as_str).or_else(|| parameters["binaryPropertyName"].as_str());
    let (body, binary) = parse_body(&incoming.headers, &incoming.body, binary_property);
    let mut json = Map::new();
    json.insert("headers".into(), Value::Object(headers_json(&incoming.headers, &hidden)));
    json.insert("params".into(), serde_json::to_value(&params).unwrap());
    json.insert("query".into(), Value::Object(incoming.query.clone()));
    json.insert("body".into(), body);
    let base = if test { "webhook-test" } else { "webhook" };
    json.insert("webhookUrl".into(), json!(format!("{}{base}/{path}", n8n.config.webhook_url)));
    json.insert("executionMode".into(), json!(if test { "test" } else { "production" }));
    let item = Item { json, binary, paired_item: None }.paired(0);

    let mode = parameters["responseMode"].as_str().unwrap_or("onReceived");
    let (slot, rx) = if mode == "responseNode" {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Some(Arc::new(Mutex::new(Some(tx)))), Some(rx))
    } else {
        (None, None)
    };
    let mut req = RunRequest::new(workflow, if test { Mode::Manual } else { Mode::Webhook });
    req.start_node = Some(reg.node.clone());
    req.start_items = Some(vec![item]);
    req.response = slot;
    req.push_ref = push_ref;
    req.use_pin_data = test;
    let handle = match runner::start(n8n, req).await {
        Ok(h) => h,
        Err(e) => return json_response(500, json!({"code": 500, "message": e.to_string()})),
    };
    add_cors(respond(&parameters, handle, rx).await, &parameters, &incoming.headers)
}

fn find_production(n8n: &N8n, method: &str, path: &str, kind: Kind) -> Option<(Registration, HashMap<String, String>)> {
    let regs = n8n.webhooks.read().unwrap();
    // Static paths win over parameterised ones.
    let mut matches: Vec<(Registration, HashMap<String, String>)> = regs
        .iter()
        .filter(|r| r.kind == kind && (r.method == method || r.method == "*"))
        .filter_map(|r| r.match_path(path).map(|p| (r.clone(), p)))
        .collect();
    matches.sort_by_key(|(_, p)| p.len());
    matches.into_iter().next()
}

async fn production(State(n8n): State<Arc<N8n>>, Path(path): Path<String>, req: Request) -> Response {
    let path = clean_path(&path);
    let incoming = match read(&n8n, req).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    if incoming.method == Method::OPTIONS {
        let regs: Vec<Registration> = n8n.webhooks.read().unwrap().iter().filter(|r| r.kind == Kind::Webhook && r.match_path(&path).is_some()).cloned().collect();
        let Some(first) = regs.first() else { return not_registered(&incoming.method, &path, false) };
        let params = match n8n.store.workflow_row(&first.workflow_id).await {
            Ok(Some(row)) => row.data["nodes"].as_array().and_then(|ns| ns.iter().find(|n| n["name"] == first.node.as_str())).map(|n| n["parameters"].clone()).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        let methods: Vec<String> = regs.iter().map(|r| r.method.clone()).collect();
        return preflight(&params, &incoming.headers, &methods);
    }
    let Some((reg, params)) = find_production(&n8n, incoming.method.as_str(), &path, Kind::Webhook) else {
        return not_registered(&incoming.method, &path, false);
    };
    let Some(workflow) = n8n.active.read().unwrap().get(&reg.workflow_id).map(|w| Value::clone(w)) else {
        return not_registered(&incoming.method, &path, false);
    };
    run_webhook(&n8n, workflow, &reg, params, incoming, &path, false, None).await
}

async fn test(State(n8n): State<Arc<N8n>>, Path(path): Path<String>, req: Request) -> Response {
    let path = clean_path(&path);
    let incoming = match read(&n8n, req).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let method = incoming.method.as_str().to_string();
    let found = {
        let mut regs = n8n.test_webhooks.lock().unwrap();
        let pos = regs.iter().position(|t| t.reg.kind == Kind::Webhook && (t.reg.method == method || incoming.method == Method::OPTIONS) && t.reg.match_path(&path).is_some());
        match pos {
            // A test webhook answers one call (OPTIONS preflights don't count).
            Some(i) if incoming.method != Method::OPTIONS => Some(regs.remove(i)),
            Some(i) => {
                let t = &regs[i];
                let params = t.workflow["nodes"].as_array().and_then(|ns| ns.iter().find(|n| n["name"] == t.reg.node.as_str())).map(|n| n["parameters"].clone()).unwrap_or(Value::Null);
                return preflight(&params, &incoming.headers, std::slice::from_ref(&t.reg.method));
            }
            None => None,
        }
    };
    let Some(t) = found else { return not_registered(&incoming.method, &path, true) };
    let params = t.reg.match_path(&path).unwrap_or_default();
    run_webhook(&n8n, t.workflow, &t.reg, params, incoming, &path, true, t.push_ref).await
}

async fn waiting_with_path(state: State<Arc<N8n>>, Path((id, _rest)): Path<(String, String)>, req: Request) -> Response {
    waiting(state, Path(id), req).await
}

/// `/webhook-waiting/:executionId?signature=...` resumes a Wait node.
async fn waiting(State(n8n): State<Arc<N8n>>, Path(id): Path<String>, req: Request) -> Response {
    let incoming = match read(&n8n, req).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let Ok(exec_id) = id.parse::<i64>() else { return ApiError::not_found(format!("The execution \"{id}\" does not exist")).into_response() };
    let row = match n8n.store.get_execution(exec_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return ApiError::not_found(format!("The execution \"{id}\" does not exist")).into_response(),
        Err(e) => return ApiError::from(e).into_response(),
    };
    let signature = incoming.query.get("signature").and_then(Value::as_str).unwrap_or("");
    if signature != n8n.resume_signature(&id) {
        return json_response(401, json!({"code": 401, "message": "Invalid signature: this resume URL is not signed for the execution"}));
    }
    if row.status != status::WAITING {
        return ApiError::new(409, format!("The execution \"{id}\" is not waiting; it is {}", row.status)).into_response();
    }
    let wait_node = row.wait_state.as_ref().and_then(|w| w["node"].as_str()).unwrap_or_default().to_string();
    let parameters = row.workflow_data["nodes"]
        .as_array()
        .and_then(|ns| ns.iter().find(|n| n["name"] == wait_node.as_str()))
        .map(|n| n["parameters"].clone())
        .unwrap_or(Value::Null);
    let (body, binary) = parse_body(&incoming.headers, &incoming.body, parameters.pointer("/options/binaryPropertyName").and_then(Value::as_str));
    let mut json = Map::new();
    json.insert("headers".into(), Value::Object(headers_json(&incoming.headers, &[])));
    json.insert("params".into(), json!({}));
    json.insert("query".into(), Value::Object(incoming.query.clone()));
    json.insert("body".into(), body);
    let item = Item { json, binary, paired_item: None }.paired(0);
    match runner::resume(&n8n, exec_id, Some(vec![item])).await {
        Ok(handle) => respond(&parameters, handle, None).await,
        Err(e) => e.into_response(),
    }
}

// ---- forms --------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn form_fields(parameters: &Value) -> Vec<Value> {
    parameters.pointer("/formFields/values").and_then(Value::as_array).cloned().unwrap_or_default()
}

fn render_form(parameters: &Value) -> String {
    let title = parameters["formTitle"].as_str().unwrap_or("Form");
    let description = parameters["formDescription"].as_str().unwrap_or("");
    let mut fields = String::new();
    for (i, f) in form_fields(parameters).iter().enumerate() {
        let label = html_escape(f["fieldLabel"].as_str().unwrap_or(""));
        let required = if f["requiredField"].as_bool().unwrap_or(false) { " required" } else { "" };
        let input = match f["fieldType"].as_str().unwrap_or("text") {
            "textarea" => format!("<textarea id=\"field-{i}\" name=\"field-{i}\"{required}></textarea>"),
            "number" => format!("<input type=\"number\" id=\"field-{i}\" name=\"field-{i}\"{required}>"),
            "email" => format!("<input type=\"email\" id=\"field-{i}\" name=\"field-{i}\"{required}>"),
            "password" => format!("<input type=\"password\" id=\"field-{i}\" name=\"field-{i}\"{required}>"),
            "date" => format!("<input type=\"date\" id=\"field-{i}\" name=\"field-{i}\"{required}>"),
            "dropdown" => {
                let options: String = f
                    .pointer("/fieldOptions/values")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|o| o["option"].as_str())
                    .map(|o| format!("<option value=\"{0}\">{0}</option>", html_escape(o)))
                    .collect();
                format!("<select id=\"field-{i}\" name=\"field-{i}\"{required}>{options}</select>")
            }
            _ => format!("<input type=\"text\" id=\"field-{i}\" name=\"field-{i}\"{required}>"),
        };
        fields.push_str(&format!("<div class=\"field\"><label for=\"field-{i}\">{label}</label>{input}</div>\n"));
    }
    let button = parameters.pointer("/options/buttonLabel").and_then(Value::as_str).unwrap_or("Submit");
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{t}</title>\
<style>body{{font-family:system-ui,sans-serif;background:#f5f5f5;margin:0;padding:24px}}form{{max-width:480px;margin:auto;background:#fff;padding:24px;border-radius:8px}}\
.field{{margin-bottom:16px;display:flex;flex-direction:column;gap:4px}}input,textarea,select{{padding:8px;font:inherit}}button{{padding:10px 16px}}</style></head>\
<body><form method=\"POST\" enctype=\"multipart/form-data\"><h1>{t}</h1><p>{d}</p>\n{fields}<button type=\"submit\">{b}</button></form></body></html>",
        t = html_escape(title),
        d = html_escape(description),
        b = html_escape(button),
    )
}

async fn form_workflow(n8n: &N8n, path: &str) -> Option<(Registration, Value)> {
    let (reg, _) = find_production(n8n, "*", path, Kind::Form)?;
    let workflow = n8n.active.read().unwrap().get(&reg.workflow_id).map(|w| Value::clone(w))?;
    Some((reg, workflow))
}

fn node_params(workflow: &Value, node: &str) -> Value {
    workflow["nodes"].as_array().and_then(|ns| ns.iter().find(|n| n["name"] == node)).map(|n| n["parameters"].clone()).unwrap_or(Value::Null)
}

async fn form_page(State(n8n): State<Arc<N8n>>, Path(path): Path<String>) -> Response {
    let path = clean_path(&path);
    let Some((reg, workflow)) = form_workflow(&n8n, &path).await else { return not_registered(&Method::GET, &path, false) };
    let html = render_form(&node_params(&workflow, &reg.node));
    ([("content-type", "text/html; charset=utf-8")], html).into_response()
}

async fn form_submit(State(n8n): State<Arc<N8n>>, Path(path): Path<String>, req: Request) -> Response {
    let path = clean_path(&path);
    let incoming = match read(&n8n, req).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let Some((reg, workflow)) = form_workflow(&n8n, &path).await else { return not_registered(&Method::POST, &path, false) };
    let parameters = node_params(&workflow, &reg.node);
    let (body, binary) = parse_body(&incoming.headers, &incoming.body, None);
    let mut json = Map::new();
    for (i, f) in form_fields(&parameters).iter().enumerate() {
        let label = f["fieldLabel"].as_str().unwrap_or("").to_string();
        let value = body.get(format!("field-{i}")).cloned().unwrap_or(Value::Null);
        let value = match (f["fieldType"].as_str(), &value) {
            (Some("number"), Value::String(s)) => s.parse::<f64>().map(|n| json!(n)).unwrap_or(value.clone()),
            _ => value,
        };
        if f["requiredField"].as_bool().unwrap_or(false) && (value.is_null() || value == json!("")) {
            return json_response(400, json!({"code": 400, "message": format!("The field \"{label}\" is required")}));
        }
        json.insert(label, value);
    }
    json.insert("submittedAt".into(), json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
    json.insert("formMode".into(), json!("production"));
    let mut req = RunRequest::new(workflow, Mode::Webhook);
    req.start_node = Some(reg.node.clone());
    req.start_items = Some(vec![Item { json, binary, paired_item: None }.paired(0)]);
    match runner::start(&n8n, req).await {
        Ok(_) => {
            let text = parameters.pointer("/options/respondWithOptions/values/formSubmittedText").and_then(Value::as_str).unwrap_or("Your response has been recorded");
            json_response(200, json!({"formSubmittedText": text}))
        }
        Err(e) => json_response(500, json!({"code": 500, "message": e.to_string()})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(path: &str, method: &str) -> Registration {
        Registration { workflow_id: "w".into(), node: "n".into(), method: method.into(), path: path.into(), kind: Kind::Webhook }
    }

    #[test]
    fn dynamic_paths_extract_parameters() {
        let r = reg("abc/users/:userId/orders/:orderId", "GET");
        let p = r.match_path("abc/users/42/orders/7").unwrap();
        assert_eq!(p["userId"], "42");
        assert_eq!(p["orderId"], "7");
        assert!(r.match_path("abc/users/42").is_none());
    }

    #[test]
    fn conflicts_need_the_same_method_and_shape() {
        assert!(reg("orders", "POST").conflicts_with(&reg("orders", "POST")));
        assert!(!reg("orders", "POST").conflicts_with(&reg("orders", "GET")));
        assert!(reg("a/:x", "GET").conflicts_with(&reg("a/:y", "GET")));
    }

    #[test]
    fn queries_and_forms_decode() {
        let q = parse_query("sku=F-9&qty=2&name=A%20B+C&t=1&t=2");
        assert_eq!(q["sku"], "F-9");
        assert_eq!(q["name"], "A B C");
        assert_eq!(q["t"], json!(["1", "2"]));
    }

    #[test]
    fn multipart_fields_are_parsed() {
        let body = "--XX\r\nContent-Disposition: form-data; name=\"field-0\"\r\n\r\nAda\r\n--XX\r\nContent-Disposition: form-data; name=\"field-1\"\r\n\r\nHello\r\n--XX--\r\n";
        let parts = parse_multipart("multipart/form-data; boundary=XX", body.as_bytes());
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "field-0");
        assert_eq!(parts[1].data, b"Hello");
    }
}
