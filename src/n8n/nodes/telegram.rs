//! Telegram node (spec §6.6), v1/1.1/1.2, faithful to n8n's
//! `nodes/Telegram/Telegram.node.js` + `GenericFunctions.js`.
//!
//! Implements:
//!
//! - `chat`: get, administrators, member, leave, setDescription, setTitle
//! - `callback`: answerQuery, answerInlineQuery
//! - `file`: get (incl. downloading the file as binary data)
//! - `message`: sendMessage, sendPhoto, sendDocument, sendAudio, sendVideo,
//!   sendAnimation, sendSticker, sendLocation, sendMediaGroup,
//!   sendChatAction, editMessageText, deleteMessage, pinChatMessage,
//!   unpinChatMessage -- both URL/file_id and binary-data (multipart)
//!   sending for the six media-send operations.
//!
//! Telegram Trigger and `sendAndWait` (and the newer `sendMessageDraft`/
//! `sendRichMessage(Draft)` operations, absent from the classic node this
//! targets) are out of scope; anything else returns a clear "not supported
//! natively yet" error.
//!
//! Faithful quirks kept from n8n's `GenericFunctions.js`:
//! - `sendMessage` defaults `additionalFields.parse_mode` to `'Markdown'`
//!   when unset, and (node version >= 1.1, which is always true for the
//!   default typeVersion 1.2 used here) defaults `appendAttribution` to
//!   `true`, appending `\n\n_This message was sent automatically with
//!   _[n8n](<utm link>)` (or the HTML equivalent for `parse_mode: 'HTML'`)
//!   to `text` unless the caller explicitly disables it.
//! - `sendMessage`/`editMessageText` default `disable_web_page_preview` to
//!   `true` when node version >= 1.2 (our default) and the caller didn't
//!   set it explicitly, regardless of whether `text` contains a URL (the
//!   older, version < 1.2 behaviour -- default only when `text` has no
//!   URL-like substring -- is implemented too, for completeness).
//! - Binary-data (multipart) sends: the file name comes from
//!   `additionalFields.fileName` or the binary property's own file name (an
//!   error if neither is set); `disable_notification` is coerced to the
//!   string `"true"`/`"false"` (multipart fields are text); `reply_markup`
//!   is JSON-stringified.
//! - `chat:administrators` explodes the Telegram API's `result` array into
//!   one output item per administrator; every other operation's output is
//!   the full `{ok, result}` API response as a single item.
//! - `file:get` with `download` (default `true`) fetches the file's bytes
//!   from `{baseUrl}/file/bot<token>/<file_path>` and returns the original
//!   `getFile` response as `json` plus the file under the binary property
//!   `data`.
//! - A failing item under `continueOnFail` reports `error.description`
//!   (Telegram's own error text) when present, falling back to the
//!   generic status-code message -- matching n8n's `error.description ??
//!   error.message`, not r8r's usual `push_error_item` (which always uses
//!   `.message`).
//!
//! Known simplification vs real n8n: the bot token appears in the request
//! URL's path (Telegram's API convention, not ours); every error path
//! redacts it from transport-level error text so it can never leak into
//! execution data.

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

pub struct Telegram;

const MEDIA_OPERATIONS: &[&str] = &["sendAnimation", "sendAudio", "sendDocument", "sendPhoto", "sendSticker", "sendVideo"];

// ---- small value helpers ---------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("Telegram \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

/// n8n's `returnJsonArray`: an array response becomes one item per element
/// (non-object elements wrapped under `data`), anything else one item.
fn to_items(value: Value, i: usize) -> Vec<Item> {
    match value {
        Value::Array(a) => a.into_iter().map(|v| Item::from_value(v).paired(i)).collect(),
        other => vec![Item::from_value(other).paired(i)],
    }
}

/// n8n's `getPropertyName`: `sendPhoto` -> `photo`.
fn property_name(operation: &str) -> String {
    operation.strip_prefix("send").unwrap_or(operation).to_ascii_lowercase()
}

fn merge_obj(body: &mut Map<String, Value>, additional: &Value) {
    if let Some(obj) = additional.as_object() {
        for (k, v) in obj {
            body.insert(k.clone(), v.clone());
        }
    }
}

fn binary_bytes<'a>(item: &'a Item, name: &str, i: usize) -> NodeResult<(Vec<u8>, &'a Map<String, Value>)> {
    let entry = item.binary.as_ref().and_then(|b| b.get(name)).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let meta = entry.as_object().ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let data = meta.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Item has no binary field '{name}'")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("The binary field '{name}' does not contain valid base64 data: {e}")).at(i))?;
    Ok((bytes, meta))
}

fn mime_for_name(file_name: &str) -> String {
    let ext = file_name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// n8n's binary data shape: base64 `data`, `mimeType`, `fileExtension`,
/// `fileSize` and `fileName`.
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

/// n8n's URL-ish text detector: `/(https?|ftp|file):\/\/\S+|www\.\S+|\S+\.\S+/`.
fn contains_url(text: &str) -> bool {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(https?|ftp|file)://\S+|www\.\S+|\S+\.\S+").unwrap());
    re.is_match(text)
}

// ---- authentication & the underlying HTTP calls ----------------------------

struct Auth {
    token: String,
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let (_, cred) = ctx.credentials("telegramApi").await?;
    let token = cred["accessToken"].as_str().unwrap_or("").to_string();
    if token.is_empty() {
        return Err(NodeError::new("Telegram credentials are not set").describe("Add an Access Token to the Telegram API credential."));
    }
    let base_url = cred["baseUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.telegram.org").trim_end_matches('/').to_string();
    Ok(Auth { token, base_url })
}

/// The bot token lives in the request URL's path (Telegram's own
/// convention); this keeps it out of any transport-level error text so it
/// never reaches execution data.
fn redact(s: &str, token: &str) -> String {
    if token.is_empty() {
        s.to_string()
    } else {
        s.replace(token, "[REDACTED]")
    }
}

/// Descriptive messages for common HTTP status codes, mirroring n8n-workflow's
/// `NodeApiError` `STATUS_CODE_MESSAGES` table.
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

fn telegram_error(status: u16, body: &Value) -> NodeError {
    let message = status_code_message(status);
    let description = body.get("description").and_then(Value::as_str).map(String::from);
    NodeError::api(message, Some(status), description)
}

async fn finish_response(resp: reqwest::Response, i: usize) -> NodeResult<Value> {
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if status >= 400 {
        return Err(telegram_error(status, &value).at(i));
    }
    Ok(value)
}

async fn telegram_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, endpoint: &str, body: &Value) -> NodeResult<Value> {
    let url_str = format!("{}/bot{}/{}", auth.base_url, auth.token, endpoint);
    let url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new("Invalid Telegram API URL").at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let req = ctx.services.http.request(method, url).json(body);
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Telegram failed: {}", redact(&e.to_string(), &auth.token)), None, None).at(i))?;
    finish_response(resp, i).await
}

#[allow(clippy::too_many_arguments)]
async fn telegram_multipart_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, fields: Vec<(String, String)>, file_field: &str, bytes: Vec<u8>, filename: String, mime: String) -> NodeResult<Value> {
    let url_str = format!("{}/bot{}/{}", auth.base_url, auth.token, endpoint);
    let url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new("Invalid Telegram API URL").at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let mut form = reqwest::multipart::Form::new();
    for (k, v) in fields {
        form = form.text(k, v);
    }
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename).mime_str(&mime).unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
    form = form.part(file_field.to_string(), part);
    let resp = ctx
        .services
        .http
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| NodeError::api(format!("The request to Telegram failed: {}", redact(&e.to_string(), &auth.token)), None, None).at(i))?;
    finish_response(resp, i).await
}

async fn download_file(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, file_path: &str) -> NodeResult<Vec<u8>> {
    let url_str = format!("{}/file/bot{}/{}", auth.base_url, auth.token, file_path);
    let url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new("Invalid Telegram file URL").at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let resp = ctx
        .services
        .http
        .get(url)
        .send()
        .await
        .map_err(|e| NodeError::api(format!("The request to Telegram failed: {}", redact(&e.to_string(), &auth.token)), None, None).at(i))?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(telegram_error(status, &value).at(i));
    }
    Ok(bytes.to_vec())
}

// ---- addAdditionalFields / addReplyMarkup ----------------------------------

/// n8n's `addReplyMarkup`.
fn add_reply_markup(ctx: &ExecCtx<'_>, i: usize, body: &mut Map<String, Value>) -> NodeResult<()> {
    let reply_markup_option = ctx.param_str("replyMarkup", i, "none")?;
    if reply_markup_option.is_empty() || reply_markup_option == "none" {
        return Ok(());
    }
    if reply_markup_option == "inlineKeyboard" || reply_markup_option == "replyKeyboard" {
        let set_key = if reply_markup_option == "replyKeyboard" { "keyboard" } else { "inline_keyboard" };
        let keyboard_data = ctx.param(&reply_markup_option, i)?;
        let mut rows_out = Vec::new();
        if let Some(rows) = keyboard_data.get("rows").and_then(Value::as_array) {
            for row in rows {
                let Some(buttons) = row.pointer("/row/buttons").and_then(Value::as_array) else { continue };
                let mut send_row = Vec::new();
                for button in buttons {
                    let mut b = Map::new();
                    b.insert("text".into(), button.get("text").cloned().unwrap_or(json!("")));
                    if let Some(add) = button.get("additionalFields").and_then(Value::as_object) {
                        for (k, v) in add {
                            b.insert(k.clone(), v.clone());
                        }
                    }
                    send_row.push(Value::Object(b));
                }
                rows_out.push(Value::Array(send_row));
            }
        }
        let mut markup = Map::new();
        markup.insert(set_key.into(), Value::Array(rows_out));
        if reply_markup_option == "replyKeyboard" {
            if let Some(opts) = ctx.param("replyKeyboardOptions", i)?.as_object() {
                for (k, v) in opts {
                    markup.insert(k.clone(), v.clone());
                }
            }
        }
        body.insert("reply_markup".into(), Value::Object(markup));
    } else if reply_markup_option == "forceReply" {
        body.insert("reply_markup".into(), ctx.param("forceReply", i)?);
    } else if reply_markup_option == "replyKeyboardRemove" {
        body.insert("reply_markup".into(), ctx.param("replyKeyboardRemove", i)?);
    }
    Ok(())
}

/// n8n's `addAdditionalFields`. `is_send_message` gates the `sendMessage`-only
/// logic (attribution, forced `parse_mode`/`disable_web_page_preview`
/// defaults); every operation that calls this merges `additionalFields` into
/// `body` and then builds `reply_markup`.
fn add_additional_fields(ctx: &ExecCtx<'_>, i: usize, body: &mut Map<String, Value>, is_send_message: bool) -> NodeResult<()> {
    let mut additional = ctx.param("additionalFields", i)?.as_object().cloned().unwrap_or_default();
    if is_send_message {
        let node_version = ctx.node.type_version;
        if node_version >= 1.1 && !additional.contains_key("appendAttribution") {
            additional.insert("appendAttribution".into(), json!(true));
        }
        let has_parse_mode = additional.get("parse_mode").and_then(Value::as_str).map(|s| !s.is_empty()).unwrap_or(false);
        if !has_parse_mode {
            additional.insert("parse_mode".into(), json!("Markdown"));
        }
        let text = body.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        if !contains_url(&text) {
            body.insert("disable_web_page_preview".into(), json!(true));
        }
        if additional.get("appendAttribution").and_then(Value::as_bool).unwrap_or(false) {
            let parse_mode = additional.get("parse_mode").and_then(Value::as_str).unwrap_or("");
            let link = "https://n8n.io/?utm_source=n8n-internal&utm_medium=powered_by&utm_campaign=n8n-nodes-base.telegram";
            let attribution = "This message was sent automatically with ";
            if parse_mode == "Markdown" {
                body.insert("text".into(), json!(format!("{text}\n\n_{attribution}_[n8n]({link})")));
            } else if parse_mode == "HTML" {
                body.insert("text".into(), json!(format!("{text}\n\n<em>{attribution}</em><a href=\"{link}\" target=\"_blank\">n8n</a>")));
            }
        }
        if node_version >= 1.2 && !additional.contains_key("disable_web_page_preview") {
            body.insert("disable_web_page_preview".into(), json!(true));
        }
        additional.remove("appendAttribution");
    }
    for (k, v) in additional {
        body.insert(k, v);
    }
    add_reply_markup(ctx, i, body)?;
    Ok(())
}

// ---- per-resource dispatch --------------------------------------------------

async fn simple_chat_call(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str) -> NodeResult<Value> {
    let mut body = Map::new();
    body.insert("chat_id".into(), ctx.param("chatId", i)?);
    telegram_request(ctx, auth, i, "POST", endpoint, &Value::Object(body)).await
}

impl Telegram {
    async fn send_media(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize, item: &Item) -> NodeResult<Value> {
        let mut body = Map::new();
        body.insert("chat_id".into(), ctx.param("chatId", i)?);
        let binary_data = ctx.param_bool("binaryData", i, false)?;
        let prop = property_name(operation);
        if !binary_data {
            body.insert(prop.clone(), json!(ctx.param_str("file", i, "")?));
        }
        add_additional_fields(ctx, i, &mut body, false)?;
        if !binary_data {
            return telegram_request(ctx, auth, i, "POST", operation, &Value::Object(body)).await;
        }

        let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
        let (bytes, meta) = binary_bytes(item, &binary_prop, i)?;
        let additional = ctx.param("additionalFields", i)?;
        let explicit_name = additional.get("fileName").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let meta_name = meta.get("fileName").and_then(Value::as_str).map(String::from);
        let filename = explicit_name.or(meta_name).ok_or_else(|| {
            NodeError::new(format!(
                "File name is needed to {operation}. Make sure the property that holds the binary data\n\t\t\thas the file name property set or set it manually in the node using the File Name parameter under\n\t\t\tAdditional Fields."
            ))
            .at(i)
        })?;
        let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();

        let disable_notification = match body.get("disable_notification") {
            Some(v) => value_to_string(v),
            None => "false".to_string(),
        };
        body.insert("disable_notification".into(), json!(disable_notification));

        let mut fields = Vec::new();
        for (k, v) in &body {
            let text = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            fields.push((k.clone(), text));
        }
        telegram_multipart_request(ctx, auth, i, operation, fields, &prop, bytes, filename, mime).await
    }

    async fn call(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, i: usize, item: &Item) -> NodeResult<Value> {
        match (resource, operation) {
            ("callback", "answerQuery") => {
                let mut body = Map::new();
                body.insert("callback_query_id".into(), json!(ctx.param_str("queryId", i, "")?));
                merge_obj(&mut body, &ctx.param("additionalFields", i)?);
                telegram_request(ctx, auth, i, "POST", "answerCallbackQuery", &Value::Object(body)).await
            }
            ("callback", "answerInlineQuery") => {
                let mut body = Map::new();
                body.insert("inline_query_id".into(), json!(ctx.param_str("queryId", i, "")?));
                body.insert("results".into(), ctx.param("results", i)?);
                merge_obj(&mut body, &ctx.param("additionalFields", i)?);
                telegram_request(ctx, auth, i, "POST", "answerInlineQuery", &Value::Object(body)).await
            }
            ("chat", "get") => simple_chat_call(ctx, auth, i, "getChat").await,
            ("chat", "administrators") => simple_chat_call(ctx, auth, i, "getChatAdministrators").await,
            ("chat", "leave") => simple_chat_call(ctx, auth, i, "leaveChat").await,
            ("chat", "member") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("user_id".into(), ctx.param("userId", i)?);
                telegram_request(ctx, auth, i, "POST", "getChatMember", &Value::Object(body)).await
            }
            ("chat", "setDescription") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("description".into(), json!(ctx.param_str("description", i, "")?));
                telegram_request(ctx, auth, i, "POST", "setChatDescription", &Value::Object(body)).await
            }
            ("chat", "setTitle") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("title".into(), json!(ctx.param_str("title", i, "")?));
                telegram_request(ctx, auth, i, "POST", "setChatTitle", &Value::Object(body)).await
            }
            ("file", "get") => {
                let mut body = Map::new();
                body.insert("file_id".into(), ctx.param("fileId", i)?);
                telegram_request(ctx, auth, i, "POST", "getFile", &Value::Object(body)).await
            }
            ("message", "editMessageText") => {
                let mut body = Map::new();
                let message_type = ctx.param_str("messageType", i, "message")?;
                if message_type == "inlineMessage" {
                    body.insert("inline_message_id".into(), ctx.param("inlineMessageId", i)?);
                } else {
                    body.insert("chat_id".into(), ctx.param("chatId", i)?);
                    body.insert("message_id".into(), ctx.param("messageId", i)?);
                }
                body.insert("text".into(), json!(ctx.param_str("text", i, "")?));
                add_additional_fields(ctx, i, &mut body, false)?;
                telegram_request(ctx, auth, i, "POST", "editMessageText", &Value::Object(body)).await
            }
            ("message", "deleteMessage") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("message_id".into(), ctx.param("messageId", i)?);
                telegram_request(ctx, auth, i, "POST", "deleteMessage", &Value::Object(body)).await
            }
            ("message", "pinChatMessage") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("message_id".into(), ctx.param("messageId", i)?);
                let additional = ctx.param("additionalFields", i)?;
                if additional.get("disable_notification").and_then(Value::as_bool).unwrap_or(false) {
                    body.insert("disable_notification".into(), json!(true));
                }
                telegram_request(ctx, auth, i, "POST", "pinChatMessage", &Value::Object(body)).await
            }
            ("message", "unpinChatMessage") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("message_id".into(), ctx.param("messageId", i)?);
                telegram_request(ctx, auth, i, "POST", "unpinChatMessage", &Value::Object(body)).await
            }
            ("message", "sendChatAction") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("action".into(), json!(ctx.param_str("action", i, "typing")?));
                telegram_request(ctx, auth, i, "POST", "sendChatAction", &Value::Object(body)).await
            }
            ("message", "sendLocation") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("latitude".into(), json!(ctx.param_f64("latitude", i, 0.0)?));
                body.insert("longitude".into(), json!(ctx.param_f64("longitude", i, 0.0)?));
                add_additional_fields(ctx, i, &mut body, false)?;
                telegram_request(ctx, auth, i, "POST", "sendLocation", &Value::Object(body)).await
            }
            ("message", "sendMediaGroup") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                merge_obj(&mut body, &ctx.param("additionalFields", i)?);
                let media_param = ctx.param("media", i)?;
                let mut media_out = Vec::new();
                if let Some(items) = media_param.get("media").and_then(Value::as_array) {
                    for m in items {
                        let mut obj = m.as_object().cloned().unwrap_or_default();
                        if let Some(add) = obj.remove("additionalFields").and_then(|v| v.as_object().cloned()) {
                            for (k, v) in add {
                                obj.insert(k, v);
                            }
                        }
                        media_out.push(Value::Object(obj));
                    }
                }
                body.insert("media".into(), Value::Array(media_out));
                telegram_request(ctx, auth, i, "POST", "sendMediaGroup", &Value::Object(body)).await
            }
            ("message", "sendMessage") => {
                let mut body = Map::new();
                body.insert("chat_id".into(), ctx.param("chatId", i)?);
                body.insert("text".into(), json!(ctx.param_str("text", i, "")?));
                add_additional_fields(ctx, i, &mut body, true)?;
                telegram_request(ctx, auth, i, "POST", "sendMessage", &Value::Object(body)).await
            }
            ("message", op) if MEDIA_OPERATIONS.contains(&op) => self.send_media(ctx, auth, op, i, item).await,
            (r, o) => Err(unsupported(r, o, i)),
        }
    }

    async fn run_item(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
        if resource == "file" && operation == "get" {
            let response = self.call(ctx, auth, resource, operation, i, item).await?;
            let download = ctx.param_bool("download", i, true)?;
            if download {
                let file_path = response.pointer("/result/file_path").and_then(Value::as_str).ok_or_else(|| NodeError::new("Telegram did not return a file path").at(i))?.to_string();
                let bytes = download_file(ctx, auth, i, &file_path).await?;
                let file_name = file_path.rsplit('/').next().unwrap_or(&file_path).to_string();
                let additional = ctx.param("additionalFields", i)?;
                let mime = additional.get("mimeType").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| mime_for_name(&file_name));
                let mut binary = Map::new();
                binary.insert("data".into(), binary_entry(&bytes, &file_name, &mime));
                let json = response.as_object().cloned().unwrap_or_default();
                return Ok(vec![Item { json, binary: Some(binary), paired_item: Some(json!({"item": i})) }]);
            }
            return Ok(to_items(response, i));
        }

        let value = self.call(ctx, auth, resource, operation, i, item).await?;
        if resource == "chat" && operation == "administrators" {
            let arr = value.pointer("/result").cloned().unwrap_or(json!([]));
            return Ok(to_items(arr, i));
        }
        Ok(to_items(value, i))
    }
}

#[async_trait::async_trait]
impl NodeType for Telegram {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.telegram"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "message")?;
        let operation = ctx.param_str("operation", 0, "sendMessage")?;
        let auth = resolve_auth(ctx).await?;
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            match self.run_item(ctx, &auth, &resource, &operation, i, item).await {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => {
                    // n8n: `{ error: error.description ?? error.message }`.
                    let message = e.description.clone().unwrap_or_else(|| e.message.clone());
                    let mut json = Map::new();
                    json.insert("error".into(), json!(message));
                    ctx.error_items.push(Item::new(json).paired(i));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
