//! OpenAI node (`@n8n/n8n-nodes-langchain.openAi`, the standalone "OpenAI"
//! action node -- not the chat-model sub-node in `ai.rs`), faithful to
//! n8n's `nodes/vendors/OpenAi/{v1,v2}` sources.
//!
//! The editor's default `typeVersion` is 2.3 (v2 of the compiled node),
//! which this targets. Where v1's parameter shapes coincide with v2's
//! (`text:classify`), both versions are handled from the node's
//! `typeVersion`; where they genuinely differ (`text:message` on v1 is a
//! `/chat/completions` call, `text:response` on v2 is a `/responses` call)
//! both are implemented since the task explicitly asks for "chat
//! completions / responses API per version".
//!
//! Implements:
//!
//! - `text`: `response` (v2, Responses API), `message` (v1, Chat
//!   Completions API), `classify` (both, `/moderations`)
//! - `image`: `generate` (`/images/generations`, incl. binary output when
//!   `returnImageUrls` is false), `analyze` (`/responses`, URL or binary
//!   image input)
//! - `audio`: `generate` (TTS, `/audio/speech` -> binary), `transcribe`,
//!   `translate` (both multipart `/audio/transcriptions` /
//!   `/audio/translations`)
//! - `file`: `upload` (multipart), `list`, `deleteFile`
//!
//! Out of scope (clear "not supported natively yet" error): `assistant`,
//! `conversation`, `video` resources; built-in tools (web search, file
//! search, code interpreter) and tool-calling with connected AI tool
//! sub-nodes on `text:response`/`text:message`.
//!
//! Faithful quirks kept from the n8n source:
//! - `text:response`/`text:message` `jsonOutput`/`textFormat: json_object`
//!   prepend a `You are a helpful assistant designed to output JSON.`
//!   system/instructions message and parse the returned text as JSON.
//! - `text:message`'s `simplify` (default `true`) returns one item per
//!   `choices` entry; `text:response`'s `simplify` returns a single item
//!   `{ output: <message items> }`.
//! - `image:generate` only sends `response_format` for non-`gpt-image*`
//!   models; `returnImageUrls` picks `url` vs `b64_json` (-> binary) for
//!   those models only.
//! - `file:upload`'s JSONL-format 400 gets n8n's clearer message.
//! - Error mapping: n8n-workflow's `STATUS_CODE_MESSAGES` table for the
//!   top-level message (same table used by `telegram.rs`/`discord.rs`/
//!   `github.rs`/`notion.rs`), OpenAI's own `error.message` as the
//!   description, and the `insufficient_quota`/`rate_limit_exceeded`
//!   overrides from `helpers/error-handling.js`.
//! - The API key is sent only as a bearer header (never in the URL or
//!   body), and is redacted from any transport-level error text.

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};

pub struct OpenAi;

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("OpenAI \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

// ---- small value helpers ---------------------------------------------------

/// A resource-locator parameter (`{mode, value}` or a plain string).
fn rlc_value(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").and_then(Value::as_str).map(String::from),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

fn to_items(value: Value, i: usize) -> Vec<Item> {
    match value {
        Value::Array(a) => a.into_iter().map(|v| Item::from_value(v).paired(i)).collect(),
        other => vec![Item::from_value(other).paired(i)],
    }
}

fn binary_bytes<'a>(item: &'a Item, name: &str, i: usize) -> NodeResult<(Vec<u8>, &'a Map<String, Value>)> {
    let entry = item.binary.as_ref().and_then(|b| b.get(name)).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let meta = entry.as_object().ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let data = meta.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("The binary field '{name}' does not contain valid base64 data: {e}")).at(i))?;
    Ok((bytes, meta))
}

fn binary_entry(data: &[u8], file_name: &str, mime: &str) -> Value {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": format!("{} B", data.len()),
        "fileName": file_name,
    })
}

/// Strips the `data` field a binary entry carries, as n8n's
/// `Object.assign({}, binaryData, { data: undefined })` does for the item
/// `json` sibling of a binary-output item.
fn binary_json(entry: &Value) -> Value {
    let mut m = entry.as_object().cloned().unwrap_or_default();
    m.remove("data");
    Value::Object(m)
}

// ---- auth & transport --------------------------------------------------------

struct Auth {
    api_key: String,
    base_url: String,
    organization: Option<String>,
    header: Option<(String, String)>,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let (_, cred) = ctx.credentials("openAiApi").await?;
    let api_key = cred["apiKey"].as_str().unwrap_or("").to_string();
    if api_key.is_empty() {
        return Err(NodeError::new("OpenAI credentials are not set").describe("Add an API Key to the OpenAi credential."));
    }
    let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.openai.com/v1").trim_end_matches('/').to_string();
    let organization = cred["organizationId"].as_str().filter(|s| !s.is_empty()).map(String::from);
    let header = if cred["header"].as_bool().unwrap_or(false) {
        let name = cred["headerName"].as_str().unwrap_or("").to_string();
        let value = cred["headerValue"].as_str().unwrap_or("").to_string();
        if !name.is_empty() {
            Some((name, value))
        } else {
            None
        }
    } else {
        None
    };
    Ok(Auth { api_key, base_url, organization, header })
}

/// Keeps the API key out of transport-level error text.
fn redact(s: &str, key: &str) -> String {
    if key.len() >= 4 {
        s.replace(key, "***")
    } else {
        s.to_string()
    }
}

fn build_url(auth: &Auth, endpoint: &str, i: usize) -> NodeResult<reqwest::Url> {
    reqwest::Url::parse(&format!("{}{}", auth.base_url, endpoint)).map_err(|_| NodeError::new("Invalid OpenAI API URL").at(i))
}

/// Mirrors n8n-workflow's `NodeApiError` `STATUS_CODE_MESSAGES` table (same
/// table used by `telegram.rs`/`discord.rs`/`github.rs`/`notion.rs`).
fn status_code_message(code: u16) -> String {
    match code {
        400 => "Bad request - please check your parameters".into(),
        401 => "Authorization failed - please check your credentials".into(),
        402 => "Payment required - perhaps check your payment details?".into(),
        403 => "Forbidden - perhaps check your credentials?".into(),
        404 => "The resource you are requesting could not be found".into(),
        405 => "Method not allowed - please check you are using the right HTTP method".into(),
        429 => "The service is receiving too many requests from you".into(),
        500 => "The service was not able to process your request".into(),
        502 => "Bad gateway - the service failed to handle your request".into(),
        503 => "Service unavailable - try again later or consider setting this node to retry automatically (in the node settings)".into(),
        504 => "Gateway timed out - perhaps try again later?".into(),
        c if (400..500).contains(&c) => "Your request is invalid or could not be processed by the service".into(),
        c if (500..600).contains(&c) => "The service failed to process your request".into(),
        c => format!("Request failed with status code {c}"),
    }
}

/// Mirrors `helpers/error-handling.js`'s `openAiFailedAttemptHandler` and
/// the generic `STATUS_CODE_MESSAGES` fallback: OpenAI's own `error.message`
/// becomes the description, with `insufficient_quota`/rate-limit overrides
/// on the headline message for 429s.
fn openai_error(status: u16, body: &Value) -> NodeError {
    let err = body.get("error");
    let api_message = err.and_then(|e| e.get("message")).and_then(Value::as_str).map(String::from);
    let code = err.and_then(|e| e.get("code")).and_then(Value::as_str);
    let message = match (status, code) {
        (429, Some("insufficient_quota")) => {
            "Insufficient quota detected. Learn more about resolving this issue at https://docs.n8n.io/integrations/builtin/app-nodes/n8n-nodes-langchain.openai/common-issues/#insufficient-quota".to_string()
        }
        (429, _) => "OpenAI: Rate limit reached".to_string(),
        _ => status_code_message(status),
    };
    NodeError::api(message, Some(status), api_message)
}

async fn finish_response(resp: reqwest::Response, i: usize) -> NodeResult<Value> {
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(openai_error(status, &value).at(i));
    }
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok(value)
}

async fn request_json(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, endpoint: &str, body: Option<&Value>, query: &[(&str, String)]) -> NodeResult<Value> {
    let mut url = build_url(auth, endpoint, i)?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let mut req = ctx.services.http.request(method, url).bearer_auth(&auth.api_key);
    if let Some(org) = &auth.organization {
        req = req.header("OpenAI-Organization", org);
    }
    if let Some((k, v)) = &auth.header {
        req = req.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to OpenAI failed: {}", redact(&e.to_string(), &auth.api_key)), None, None).at(i))?;
    finish_response(resp, i).await
}

/// Raw bytes response (TTS's `/audio/speech`).
async fn request_bytes(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, body: &Value) -> NodeResult<Vec<u8>> {
    let url = build_url(auth, endpoint, i)?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let mut req = ctx.services.http.post(url).bearer_auth(&auth.api_key).json(body);
    if let Some(org) = &auth.organization {
        req = req.header("OpenAI-Organization", org);
    }
    if let Some((k, v)) = &auth.header {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to OpenAI failed: {}", redact(&e.to_string(), &auth.api_key)), None, None).at(i))?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(openai_error(status, &value).at(i));
    }
    Ok(bytes.to_vec())
}

#[allow(clippy::too_many_arguments)]
async fn request_multipart(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, fields: Vec<(String, String)>, file_field: &str, bytes: Vec<u8>, filename: String, mime: String) -> NodeResult<Value> {
    let url = build_url(auth, endpoint, i)?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let mut form = reqwest::multipart::Form::new();
    for (k, v) in fields {
        form = form.text(k, v);
    }
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename).mime_str(&mime).unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
    form = form.part(file_field.to_string(), part);
    let mut req = ctx.services.http.post(url).multipart(form).bearer_auth(&auth.api_key);
    if let Some(org) = &auth.organization {
        req = req.header("OpenAI-Organization", org);
    }
    if let Some((k, v)) = &auth.header {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to OpenAI failed: {}", redact(&e.to_string(), &auth.api_key)), None, None).at(i))?;
    finish_response(resp, i).await
}

// ---- text:response (v2, Responses API) --------------------------------------

/// Converts one `responses.values` entry to a Responses-API input message.
async fn format_input_message(ctx: &ExecCtx<'_>, item: &Item, i: usize, m: &Value) -> NodeResult<Value> {
    let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
    let kind = m.get("type").and_then(Value::as_str).unwrap_or("text");
    let content = match kind {
        "image" => {
            let detail = m.get("imageDetail").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("auto");
            let image_type = m.get("imageType").and_then(Value::as_str).unwrap_or("url");
            match image_type {
                "base64" => {
                    let prop = m.get("binaryPropertyName").and_then(Value::as_str).unwrap_or("data");
                    let (bytes, meta) = binary_bytes(item, prop, i)?;
                    let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream");
                    let data_url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
                    json!([{"type": "input_image", "detail": detail, "image_url": data_url}])
                }
                "fileId" => json!([{"type": "input_image", "detail": detail, "file_id": m.get("fileId").cloned().unwrap_or(json!(""))}]),
                _ => json!([{"type": "input_image", "detail": detail, "image_url": m.get("imageUrl").cloned().unwrap_or(json!(""))}]),
            }
        }
        "file" => {
            let file_type = m.get("fileType").and_then(Value::as_str).unwrap_or("url");
            match file_type {
                "base64" => {
                    let prop = m.get("binaryPropertyName").and_then(Value::as_str).unwrap_or("data");
                    let (bytes, meta) = binary_bytes(item, prop, i)?;
                    let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream");
                    let data_url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
                    json!([{"type": "input_file", "filename": m.get("fileName").cloned().unwrap_or(json!("")), "file_data": data_url}])
                }
                "fileId" => json!([{"type": "input_file", "file_id": m.get("fileId").cloned().unwrap_or(json!(""))}]),
                _ => json!([{"type": "input_file", "file_url": m.get("fileUrl").cloned().unwrap_or(json!(""))}]),
            }
        }
        _ => {
            let text = m.get("content").and_then(Value::as_str).unwrap_or("");
            if role == "assistant" {
                json!([{"type": "output_text", "text": text, "annotations": []}])
            } else {
                json!([{"type": "input_text", "text": text}])
            }
        }
    };
    Ok(json!({"role": role, "content": content}))
}

async fn text_response(ctx: &ExecCtx<'_>, i: usize, item: &Item, auth: &Auth) -> NodeResult<Vec<Item>> {
    let model = ctx.param("modelId", i)?;
    let model = rlc_value(&model).ok_or_else(|| NodeError::new("No model specified").at(i))?;
    let messages = ctx.param("responses.values", i)?.as_array().cloned().unwrap_or_default();
    let has_text = messages.iter().any(|m| {
        let kind = m.get("type").and_then(Value::as_str).unwrap_or("text");
        kind == "text" && m.get("content").and_then(Value::as_str).map(|s| !s.trim().is_empty()).unwrap_or(false)
    });
    if !has_text {
        return Err(NodeError::new("A non-empty prompt is required.").at(i));
    }
    let mut input = Vec::new();
    for m in &messages {
        input.push(format_input_message(ctx, item, i, m).await?);
    }

    let instructions = ctx.param_str("options.instructions", i, "")?;
    let max_tokens = ctx.param("options.maxTokens", i)?;
    let temperature = ctx.param("options.temperature", i)?;
    let top_p = ctx.param("options.topP", i)?;
    let store = ctx.param("options.store", i)?.as_bool().unwrap_or(true);
    let text_type = ctx.param_str("options.textFormat.textOptions.type", i, "")?;

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("parallel_tool_calls".into(), json!(true));
    body.insert("store".into(), json!(store));
    if !instructions.is_empty() {
        body.insert("instructions".into(), json!(instructions));
    }
    if let Some(n) = max_tokens.as_f64().filter(|n| *n > 0.0) {
        body.insert("max_output_tokens".into(), json!(n as i64));
    }
    if let Some(n) = temperature.as_f64() {
        body.insert("temperature".into(), json!(n));
    }
    if let Some(n) = top_p.as_f64() {
        body.insert("top_p".into(), json!(n));
    }
    if text_type == "json_object" {
        input.insert(0, json!({"role": "system", "content": [{"type": "input_text", "text": "You are a helpful assistant designed to output JSON."}]}));
        body.insert("text".into(), json!({"format": {"type": "json_object"}}));
    } else if text_type == "json_schema" {
        let name = ctx.param_str("options.textFormat.textOptions.name", i, "my_schema")?;
        let schema_raw = ctx.param_str("options.textFormat.textOptions.schema", i, "{}")?;
        let schema: Value = serde_json::from_str(&schema_raw).map_err(|e| NodeError::new(format!("Failed to parse schema: {e}")).at(i))?;
        let description = ctx.param_str("options.textFormat.textOptions.description", i, "")?;
        let mut format = json!({"type": "json_schema", "name": name, "schema": schema});
        if !description.is_empty() {
            format["description"] = json!(description);
        }
        body.insert("text".into(), json!({"format": format}));
    } else if text_type == "text" {
        body.insert("text".into(), json!({"format": {"type": "text"}}));
    }
    body.insert("input".into(), Value::Array(input));

    let mut response = request_json(ctx, auth, i, "POST", "/responses", Some(&Value::Object(body)), &[]).await?;

    if text_type == "json_object" || text_type == "json_schema" {
        if let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) {
            for item in output.iter_mut() {
                if item.get("type").and_then(Value::as_str) == Some("message") {
                    if let Some(content) = item.get_mut("content").and_then(Value::as_array_mut) {
                        for c in content.iter_mut() {
                            if c.get("type").and_then(Value::as_str) == Some("output_text") {
                                if let Some(text) = c.get("text").and_then(Value::as_str) {
                                    if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                                        c["text"] = parsed;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let simplify = ctx.param_bool("simplify", i, true)?;
    if simplify {
        let messages: Vec<Value> = response["output"].as_array().cloned().unwrap_or_default().into_iter().filter(|item| item.get("type").and_then(Value::as_str) == Some("message")).collect();
        Ok(vec![Item::new(Map::from_iter([("output".to_string(), json!(messages))])).paired(i)])
    } else {
        Ok(vec![Item::from_value(response).paired(i)])
    }
}

// ---- text:message (v1, Chat Completions API) --------------------------------

async fn text_message(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let model = match ctx.param("modelId", i)? {
        Value::Null => ctx.param_str("model", i, "")?,
        other => rlc_value(&other).unwrap_or_default(),
    };
    let mut messages = ctx.param("messages.values", i)?.as_array().cloned().unwrap_or_default();
    let has_text = messages.iter().any(|m| m.get("content").and_then(Value::as_str).map(|s| !s.trim().is_empty()).unwrap_or(false));
    if !has_text {
        return Err(NodeError::new("A non-empty prompt is required.").at(i));
    }
    for m in messages.iter_mut() {
        if m.get("role").is_none() {
            m["role"] = json!("user");
        }
    }

    let json_output = ctx.param_bool("jsonOutput", i, false)?;
    let mut response_format = None;
    if json_output {
        response_format = Some(json!({"type": "json_object"}));
        messages.insert(0, json!({"role": "system", "content": "You are a helpful assistant designed to output JSON."}));
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(rf) = &response_format {
        body.insert("response_format".into(), rf.clone());
    }
    for (src, dst) in [("options.frequency_penalty", "frequency_penalty"), ("options.n", "n"), ("options.presence_penalty", "presence_penalty"), ("options.temperature", "temperature"), ("options.reasoning_effort", "reasoning_effort")] {
        let v = ctx.param(src, i)?;
        if !v.is_null() {
            body.insert(dst.to_string(), v);
        }
    }
    if let Some(n) = ctx.param("options.maxTokens", i)?.as_f64() {
        body.insert("max_completion_tokens".into(), json!(n as i64));
    }
    if let Some(n) = ctx.param("options.topP", i)?.as_f64() {
        body.insert("top_p".into(), json!(n));
    }

    let mut response = request_json(ctx, auth, i, "POST", "/chat/completions", Some(&Value::Object(body)), &[]).await?;

    if response_format.is_some() {
        if let Some(choices) = response.get_mut("choices").and_then(Value::as_array_mut) {
            for choice in choices.iter_mut() {
                if let Some(content) = choice.pointer("/message/content").and_then(Value::as_str) {
                    if let Ok(parsed) = serde_json::from_str::<Value>(content) {
                        choice["message"]["content"] = parsed;
                    }
                }
            }
        }
    }

    let simplify = ctx.param_bool("simplify", i, true)?;
    if simplify {
        let choices = response["choices"].as_array().cloned().unwrap_or_default();
        Ok(choices.into_iter().map(|c| Item::from_value(c).paired(i)).collect())
    } else {
        Ok(vec![Item::from_value(response).paired(i)])
    }
}

// ---- text:classify (both versions, /moderations) -----------------------------

async fn text_classify(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let input = ctx.param_str("input", i, "")?;
    let version = ctx.node.type_version;
    let model = if version < 2.1 {
        if ctx.param_bool("options.useStableModel", i, false)? {
            "text-moderation-stable"
        } else {
            "text-moderation-latest"
        }
    } else {
        "omni-moderation-latest"
    };
    let body = json!({"input": input, "model": model});
    let response = request_json(ctx, auth, i, "POST", "/moderations", Some(&body), &[]).await?;
    let results = response["results"].as_array().cloned().unwrap_or_default();
    let Some(first) = results.into_iter().next() else { return Ok(vec![]) };
    let simplify = ctx.param_bool("simplify", i, false)?;
    if simplify {
        let flagged = first.get("flagged").cloned().unwrap_or(json!(false));
        Ok(vec![Item::new(Map::from_iter([("flagged".to_string(), flagged)])).paired(i)])
    } else {
        Ok(vec![Item::from_value(first).paired(i)])
    }
}

// ---- image:generate -----------------------------------------------------------

async fn image_generate(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let model = match ctx.param("modelId", i)? {
        Value::Null => ctx.param_str("model", i, "dall-e-3")?,
        v if rlc_value(&v).is_some() => rlc_value(&v).unwrap(),
        _ => ctx.param_str("model", i, "dall-e-3")?,
    };
    let prompt = ctx.param_str("prompt", i, "")?;
    if prompt.trim().is_empty() {
        return Err(NodeError::new("A non-empty prompt is required.").at(i));
    }
    let mut options = ctx.param("options", i)?.as_object().cloned().unwrap_or_default();
    let supports_response_format = !model.starts_with("gpt-image");
    let return_urls = options.remove("returnImageUrls").and_then(|v| v.as_bool()).unwrap_or(false);
    let binary_output = options.remove("binaryPropertyOutput").and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "data".to_string());
    if let Some(q) = options.remove("dalleQuality") {
        options.insert("quality".into(), q);
    }
    let response_format = if supports_response_format && return_urls { "url" } else { "b64_json" };

    let mut body = Map::new();
    body.insert("prompt".into(), json!(prompt));
    body.insert("model".into(), json!(model));
    if supports_response_format {
        body.insert("response_format".into(), json!(response_format));
    }
    for (k, v) in options {
        body.insert(k, v);
    }

    let response = request_json(ctx, auth, i, "POST", "/images/generations", Some(&Value::Object(body)), &[]).await?;
    let data = response["data"].as_array().cloned().unwrap_or_default();
    if response_format == "url" {
        Ok(data.into_iter().map(|e| Item::from_value(e).paired(i)).collect())
    } else {
        let mut out = Vec::new();
        for entry in data {
            let Some(b64) = entry.get("b64_json").and_then(Value::as_str) else { continue };
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| NodeError::new(format!("OpenAI returned invalid base64 image data: {e}")).at(i))?;
            let binary = binary_entry(&bytes, "data", "image/png");
            let json_meta = binary_json(&binary);
            let mut binary_map = Map::new();
            binary_map.insert(binary_output.clone(), binary);
            out.push(Item { json: json_meta.as_object().cloned().unwrap_or_default(), binary: Some(binary_map), paired_item: Some(json!({"item": i})) });
        }
        Ok(out)
    }
}

// ---- image:analyze --------------------------------------------------------------

async fn image_analyze(ctx: &ExecCtx<'_>, i: usize, item: &Item, auth: &Auth) -> NodeResult<Vec<Item>> {
    let model = match ctx.param("modelId", i)? {
        Value::Null => "gpt-4o".to_string(),
        v => rlc_value(&v).unwrap_or_else(|| "gpt-4o".to_string()),
    };
    let text = ctx.param_str("text", i, "What's in this image?")?;
    if text.trim().is_empty() {
        return Err(NodeError::new("A non-empty prompt is required.").at(i));
    }
    let input_type = ctx.param_str("inputType", i, "url")?;
    let detail = ctx.param_str("options.detail", i, "auto")?;
    let detail = if detail.is_empty() { "auto".to_string() } else { detail };
    let max_tokens = ctx.param_f64("options.maxTokens", i, 300.0)?;

    let mut content = vec![json!({"type": "input_text", "text": text})];
    if input_type == "url" {
        let urls = ctx.param_str("imageUrls", i, "")?;
        for url in urls.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            content.push(json!({"type": "input_image", "detail": detail, "image_url": url}));
        }
    } else {
        let names = ctx.param_str("binaryPropertyName", i, "data")?;
        for name in names.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (bytes, meta) = binary_bytes(item, name, i)?;
            let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream");
            let data_url = format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&bytes));
            content.push(json!({"type": "input_image", "detail": detail, "image_url": data_url}));
        }
    }

    let body = json!({
        "model": model,
        "input": [{"role": "user", "content": content}],
        "max_output_tokens": max_tokens as i64,
    });
    let response = request_json(ctx, auth, i, "POST", "/responses", Some(&body), &[]).await?;
    let simplify = ctx.param_bool("simplify", i, true)?;
    if simplify {
        Ok(vec![Item::new(Map::from_iter([("output".to_string(), response["output"].clone())])).paired(i)])
    } else {
        Ok(vec![Item::from_value(response).paired(i)])
    }
}

// ---- audio:generate (TTS) --------------------------------------------------------

async fn audio_generate(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let model = ctx.param_str("model", i, "tts-1")?;
    let input = ctx.param_str("input", i, "")?;
    let voice = ctx.param_str("voice", i, "alloy")?;
    let response_format = ctx.param_str("options.response_format", i, "mp3")?;
    let response_format = if response_format.is_empty() { "mp3".to_string() } else { response_format };
    let speed = ctx.param_f64("options.speed", i, 1.0)?;
    let binary_output = ctx.param_str("options.binaryPropertyOutput", i, "data")?;
    let binary_output = if binary_output.is_empty() { "data".to_string() } else { binary_output };

    let body = json!({"model": model, "input": input, "voice": voice, "response_format": response_format, "speed": speed});
    let bytes = request_bytes(ctx, auth, i, "/audio/speech", &body).await?;
    let mime = match response_format.as_str() {
        "mp3" => "audio/mpeg",
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        other => return Err(NodeError::new(format!("Unsupported audio response format \"{other}\"")).at(i)),
    };
    let file_name = format!("audio.{response_format}");
    let binary = binary_entry(&bytes, &file_name, mime);
    let json_meta = binary_json(&binary);
    let mut binary_map = Map::new();
    binary_map.insert(binary_output, binary);
    Ok(vec![Item { json: json_meta.as_object().cloned().unwrap_or_default(), binary: Some(binary_map), paired_item: Some(json!({"item": i})) }])
}

// ---- audio:transcribe / audio:translate -------------------------------------------

async fn audio_transcribe_or_translate(ctx: &ExecCtx<'_>, i: usize, item: &Item, auth: &Auth, endpoint: &str, with_language: bool) -> NodeResult<Vec<Item>> {
    let prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let (bytes, meta) = binary_bytes(item, &prop, i)?;
    let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let filename = meta.get("fileName").and_then(Value::as_str).unwrap_or("file").to_string();

    let mut fields = vec![("model".to_string(), "whisper-1".to_string())];
    if with_language {
        let language = ctx.param_str("options.language", i, "")?;
        if !language.is_empty() {
            fields.push(("language".to_string(), language));
        }
    }
    let temperature = ctx.param_f64("options.temperature", i, 0.0)?;
    if temperature != 0.0 {
        fields.push(("temperature".to_string(), temperature.to_string()));
    }

    let response = request_multipart(ctx, auth, i, endpoint, fields, "file", bytes, filename, mime).await?;
    Ok(vec![Item::from_value(response).paired(i)])
}

// ---- file:upload / file:list / file:deleteFile -------------------------------------

async fn file_upload(ctx: &ExecCtx<'_>, i: usize, item: &Item, auth: &Auth) -> NodeResult<Vec<Item>> {
    let prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let (bytes, meta) = binary_bytes(item, &prop, i)?;
    let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let filename = meta.get("fileName").and_then(Value::as_str).unwrap_or("file").to_string();
    let purpose = ctx.param_str("options.purpose", i, "user_data")?;
    let purpose = if purpose.is_empty() { "user_data".to_string() } else { purpose };

    let fields = vec![("purpose".to_string(), purpose)];
    let result = request_multipart(ctx, auth, i, "/files", fields, "file", bytes, filename, mime).await;
    match result {
        Ok(response) => Ok(vec![Item::from_value(response).paired(i)]),
        Err(e) if e.message.contains("Bad request") && e.description.as_deref().unwrap_or("").contains("Expected file to have JSONL format") => {
            Err(NodeError::new("The file content is not in JSONL format").describe("Fine-tuning accepts only files in JSONL format, where every line is a valid JSON dictionary").at(i))
        }
        Err(e) => Err(e),
    }
}

async fn file_list(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let purpose = ctx.param_str("options.purpose", i, "any")?;
    let query: Vec<(&str, String)> = if purpose.is_empty() || purpose == "any" { vec![] } else { vec![("purpose", purpose)] };
    let response = request_json(ctx, auth, i, "GET", "/files", None, &query).await?;
    let data = response["data"].as_array().cloned().unwrap_or_default();
    Ok(data.into_iter().map(|f| Item::from_value(f).paired(i)).collect())
}

async fn file_delete(ctx: &ExecCtx<'_>, i: usize, auth: &Auth) -> NodeResult<Vec<Item>> {
    let file_id = rlc_value(&ctx.param("fileId", i)?).ok_or_else(|| NodeError::new("No file ID specified").at(i))?;
    let response = request_json(ctx, auth, i, "DELETE", &format!("/files/{file_id}"), None, &[]).await?;
    Ok(vec![Item::from_value(response).paired(i)])
}

// ---- dispatch ------------------------------------------------------------------

impl OpenAi {
    async fn run_item(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
        match (resource, operation) {
            ("text", "response") => text_response(ctx, i, item, auth).await,
            ("text", "message") => text_message(ctx, i, auth).await,
            ("text", "classify") => text_classify(ctx, i, auth).await,
            ("image", "generate") => image_generate(ctx, i, auth).await,
            ("image", "analyze") => image_analyze(ctx, i, item, auth).await,
            ("audio", "generate") => audio_generate(ctx, i, auth).await,
            ("audio", "transcribe") => audio_transcribe_or_translate(ctx, i, item, auth, "/audio/transcriptions", true).await,
            ("audio", "translate") => audio_transcribe_or_translate(ctx, i, item, auth, "/audio/translations", false).await,
            ("file", "upload") => file_upload(ctx, i, item, auth).await,
            ("file", "list") => file_list(ctx, i, auth).await,
            ("file", "deleteFile") => file_delete(ctx, i, auth).await,
            (r, o) => Err(unsupported(r, o, i)),
        }
    }
}

#[async_trait::async_trait]
impl NodeType for OpenAi {
    fn type_name(&self) -> &'static str {
        "@n8n/n8n-nodes-langchain.openAi"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "text")?;
        let operation = ctx.param_str("operation", 0, "response")?;
        let auth = resolve_auth(ctx).await?;
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            match self.run_item(ctx, &auth, &resource, &operation, i, item).await {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
