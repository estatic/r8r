//! Discord node (spec §6.6), v2, faithful to n8n's `nodes/Discord/v2/*`.
//!
//! Implements:
//!
//! - `channel`: create, deleteChannel, get, getAll, update
//! - `member`: getAll, roleAdd, roleRemove
//! - `message`: deleteMessage, get, getAll, react, send (incl. embeds and
//!   file attachments uploaded as `multipart/form-data`)
//! - `webhook`: sendLegacy
//!
//! Authentication: `botToken` (`discordBotApi`), `oAuth2` (`discordOAuth2Api`),
//! `webhook` (`discordWebhookApi`). `sendAndWait` and everything else return a
//! clear "not supported natively yet" error.
//!
//! Faithful quirks kept from n8n's `helpers/utils.js` + `transport/*`:
//! - All requests go to `https://discord.com/api/v10` (overridable via the
//!   credential's `url` field for tests, the same convention as
//!   `slack.rs`/`notion.rs`), except under `webhook` authentication, where
//!   every request goes straight to the credential's `webhookUri`.
//! - Under `oAuth2` authentication, *every* API call except
//!   `GET /users/@me/guilds` is authenticated with `Bot <discordOAuth2Api
//!   .botToken>` (not the OAuth access token!); only the one-time guild list
//!   fetch uses the real OAuth bearer token
//!   (`oauthTokenData.access_token`, like `slack.rs`). This mirrors n8n's
//!   `requestApi`'s `useCustomAuth` branch exactly.
//! - Before running any `channel`/`message`/`member` operation under
//!   `oAuth2`, the node fetches `/users/@me/guilds` once and checks the
//!   selected `guildId` is in it (`checkAccessToGuild`); operations that
//!   take a `channelId` RLC directly (`channel.get/deleteChannel/update`,
//!   `message.get/getAll/deleteMessage/react`, and `message.send` with
//!   `sendTo: channel`) additionally look up that channel's guild via
//!   `GET /channels/:id` and check it the same way
//!   (`checkAccessToChannel`/`setupChannelGetter`) -- a *second* GET to the
//!   same channel beyond the operation's own fetch, exactly as n8n does.
//! - `channel.getAll` and `member.getAll` run once per node execution (not
//!   once per input item), matching n8n's operation functions, which never
//!   call `getInputData()` and hard-code item index 0.
//! - `parseDiscordError`: a 400 response whose body has
//!   `errors.embeds[0]` becomes "The parameter(s) ... (is/are) not properly
//!   formatted"; `errors.message_reference` becomes "The message to reply to
//!   ID can't be found"; otherwise the body's own `message` field is used
//!   when present, else the generic n8n-workflow `STATUS_CODE_MESSAGES` text
//!   for the status code (401/403/404/429/etc, same table as `telegram.rs`).
//! - `message.send`/`webhook.sendLegacy` always send a `content` key (even
//!   `""`), and embeds/files are only added when the `embeds`/`files`
//!   fixedCollection parameters are present.
//!
//! Known simplifications vs real n8n:
//! - Discord's 429 rate-limit retry/backoff (`transport/helpers.js`) is not
//!   implemented; a 429 response is surfaced as an error immediately rather
//!   than retried, so BDD scenarios stay deterministic.
//! - `continueOnFail` always suppresses the item's error uniformly,
//!   including for `channel.getAll`/`member.getAll`; real n8n's own
//!   operation functions for those two push the error item but then
//!   unconditionally re-throw anyway (missing a `return`/`continue`), which
//!   looks like an n8n bug and is not replicated.
//! - Error items under `continueOnFail` are `{"error": "<message>"}` (r8r's
//!   usual shape via `ExecCtx::push_error_item`), not n8n's
//!   `{error, description}` pair.

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};

pub struct Discord;

// ---- small value helpers ---------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A `resourceLocator` value's `.value`, or the value itself when it is
/// already a plain string (r8r workflows may supply either shape).
fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("Discord \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

/// n8n's `returnJsonArray`: an array response becomes one item per element
/// (non-object elements wrapped under `data`), anything else one item.
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
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| NodeError::new(format!("The binary field '{name}' does not contain valid base64 data: {e}")).at(i))?;
    Ok((bytes, meta))
}

fn mime_extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "application/json" => "json",
        "application/zip" => "zip",
        "audio/mpeg" => "mp3",
        "audio/ogg" => "ogg",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        _ => "bin",
    }
}

/// n8n's `encodeURIComponent`, used on the reaction emoji in the URL path.
fn percent_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---- authentication & the underlying HTTP calls ----------------------------

struct Auth {
    is_webhook: bool,
    /// `Authorization` header value for every request except the oAuth2
    /// guild-list check (`Bot <token>`, always from the bot-token field --
    /// see module docs).
    authorization: String,
    /// The real OAuth2 access token, used only for `GET /users/@me/guilds`.
    oauth_bearer: Option<String>,
    /// `https://discord.com/api/v10` (overridable), or the full webhook URI
    /// under `webhook` authentication.
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>, authentication: &str) -> NodeResult<Auth> {
    match authentication {
        "oAuth2" => {
            let (_, cred) = ctx.credentials("discordOAuth2Api").await?;
            let bot_token = cred["botToken"].as_str().unwrap_or("").to_string();
            if bot_token.is_empty() {
                return Err(NodeError::new("Discord credentials are not set").describe("Add a Bot Token to the Discord OAuth2 API credential."));
            }
            let access_token = cred
                .pointer("/oauthTokenData/access_token")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    NodeError::new("The Discord OAuth2 credential is not connected").describe("Complete the OAuth2 authorization for this credential before using it.")
                })?
                .to_string();
            let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://discord.com/api/v10").trim_end_matches('/').to_string();
            Ok(Auth { is_webhook: false, authorization: format!("Bot {bot_token}"), oauth_bearer: Some(access_token), base_url })
        }
        "webhook" => {
            let (_, cred) = ctx.credentials("discordWebhookApi").await?;
            let uri = cred["webhookUri"].as_str().unwrap_or("").to_string();
            if uri.is_empty() {
                return Err(NodeError::new("Discord credentials are not set").describe("Add a Webhook URL to the Discord Webhook credential."));
            }
            Ok(Auth { is_webhook: true, authorization: String::new(), oauth_bearer: None, base_url: uri })
        }
        _ => {
            let (_, cred) = ctx.credentials("discordBotApi").await?;
            let token = cred["botToken"].as_str().unwrap_or("").to_string();
            if token.is_empty() {
                return Err(NodeError::new("Discord credentials are not set").describe("Add a Bot Token to the Discord Bot API credential."));
            }
            let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://discord.com/api/v10").trim_end_matches('/').to_string();
            Ok(Auth { is_webhook: false, authorization: format!("Bot {token}"), oauth_bearer: None, base_url })
        }
    }
}

/// Descriptive messages for common HTTP status codes, mirroring n8n-workflow's
/// `NodeApiError` `STATUS_CODE_MESSAGES` table (same table as `telegram.rs`).
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

/// Mirrors n8n's `parseDiscordError` (helpers/utils.js).
fn discord_error(status: u16, body: &Value) -> NodeError {
    let body_message = body.get("message").and_then(Value::as_str).map(String::from);
    let mut message = body_message.clone().unwrap_or_else(|| status_code_message(status));
    let mut description: Option<String> = None;
    if status == 400 {
        if let Some(embed_errors) = body.pointer("/errors/embeds/0").and_then(Value::as_object) {
            let keys: Vec<String> = embed_errors.keys().map(|k| capitalize(k)).collect();
            if !keys.is_empty() {
                message = if keys.len() == 1 {
                    format!("The parameter {} is not properly formatted", keys[0])
                } else {
                    format!("The parameters {} are not properly formatted", keys.join(", "))
                };
                description = Some("Review the formatting or clear it".to_string());
            }
        } else if body.pointer("/errors/message_reference").is_some() {
            message = "The message to reply to ID can't be found".to_string();
            description = Some("Check the \"Message to Reply to\" parameter and remove it if you don't want to reply to an existing message".to_string());
        } else if message == "Cannot send an empty message" {
            description = Some("Something has to be send to the channel whether it is a message, an embed or a file".to_string());
        }
    }
    NodeError::api(message, Some(status), description)
}

#[allow(clippy::too_many_arguments)]
async fn discord_request_inner(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<Value>, query: &[(String, String)], use_oauth_bearer: bool) -> NodeResult<Value> {
    let url_str = if auth.is_webhook { auth.base_url.clone() } else { format!("{}{}", auth.base_url, path) };
    let mut url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid Discord API URL: {url_str}")).at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let method_v = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let mut req = ctx.services.http.request(method_v, url);
    if use_oauth_bearer {
        if let Some(b) = &auth.oauth_bearer {
            req = req.bearer_auth(b);
        }
    } else if !auth.is_webhook {
        req = req.header("Authorization", &auth.authorization);
    }
    if let Some(b) = &body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Discord failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status >= 400 {
        let body_val: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Err(discord_error(status, &body_val).at(i));
    }
    if text.trim().is_empty() {
        return Ok(json!({"success": true}));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn discord_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<Value>, query: &[(String, String)]) -> NodeResult<Value> {
    discord_request_inner(ctx, auth, i, method, path, body, query, false).await
}

async fn discord_guild_list(ctx: &ExecCtx<'_>, auth: &Auth, i: usize) -> NodeResult<Vec<Value>> {
    let v = discord_request_inner(ctx, auth, i, "GET", "/users/@me/guilds", None, &[], true).await?;
    Ok(v.as_array().cloned().unwrap_or_default())
}

async fn discord_multipart_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, path: &str, form: reqwest::multipart::Form) -> NodeResult<Value> {
    let url_str = if auth.is_webhook { auth.base_url.clone() } else { format!("{}{}", auth.base_url, path) };
    let url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid Discord API URL: {url_str}")).at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let mut req = ctx.services.http.post(url).multipart(form);
    if !auth.is_webhook {
        req = req.header("Authorization", &auth.authorization);
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Discord failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status >= 400 {
        let body_val: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        return Err(discord_error(status, &body_val).at(i));
    }
    if text.trim().is_empty() {
        return Ok(json!({"success": true}));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

// ---- guild/channel access checks (oAuth2 only) ------------------------------

fn check_access_to_guild(guild_id: &str, user_guilds: &[Value], i: usize) -> NodeResult<()> {
    if !user_guilds.iter().any(|g| g.get("id").and_then(Value::as_str) == Some(guild_id)) {
        return Err(NodeError::new(format!("You do not have access to the guild with the id {guild_id}")).at(i));
    }
    Ok(())
}

async fn check_access_to_channel(ctx: &ExecCtx<'_>, auth: &Auth, channel_id: &str, user_guilds: &[Value], i: usize) -> NodeResult<()> {
    let channel = discord_request(ctx, auth, i, "GET", &format!("/channels/{channel_id}"), None, &[]).await;
    let found_guild = match channel {
        Ok(c) => c.get("guild_id").and_then(Value::as_str).map(String::from),
        Err(_) => None,
    };
    let Some(found_guild) = found_guild.filter(|s| !s.is_empty()) else {
        return Err(NodeError::new(format!("Could not find server for channel with the id {channel_id}")).at(i));
    };
    check_access_to_guild(&found_guild, user_guilds, i)
}

/// n8n's `setupChannelGetter`: reads the `channelId` RLC and, under oAuth2,
/// verifies access to its guild.
async fn channel_id_param(ctx: &ExecCtx<'_>, auth: &Auth, user_guilds: &[Value], is_oauth2: bool, i: usize) -> NodeResult<String> {
    let channel_id = locator_str(&ctx.param("channelId", i)?).unwrap_or_default();
    if is_oauth2 {
        check_access_to_channel(ctx, auth, &channel_id, user_guilds, i).await?;
    }
    Ok(channel_id)
}

// ---- prepareOptions / prepareEmbeds / prepareMultiPartForm ------------------

/// n8n's `prepareOptions`.
fn prepare_options(mut options: Map<String, Value>, guild_id: &str) -> Map<String, Value> {
    if let Some(flags) = options.get("flags").cloned() {
        if let Some(names) = flags.as_array() {
            let names: Vec<String> = names.iter().map(value_to_string).collect();
            if names.len() == 2 {
                options.insert("flags".into(), json!((1i64 << 2) + (1i64 << 12)));
            } else if names.iter().any(|f| f == "SUPPRESS_EMBEDS") {
                options.insert("flags".into(), json!(1i64 << 2));
            } else if names.iter().any(|f| f == "SUPPRESS_NOTIFICATIONS") {
                options.insert("flags".into(), json!(1i64 << 12));
            }
        }
    }
    if let Some(mr) = options.get("message_reference").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from) {
        options.insert("message_reference".into(), json!({"message_id": mr, "guild_id": guild_id}));
    }
    options
}

/// n8n's `prepareEmbeds`.
fn prepare_embeds(embeds: &[Value], i: usize) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    for embed in embeds {
        let input_method = embed.get("inputMethod").and_then(Value::as_str).unwrap_or("fields");
        let mut data: Map<String, Value> = Map::new();
        if input_method == "json" {
            let raw = embed.get("json").cloned().unwrap_or(json!("{}"));
            let parsed = match raw {
                Value::Object(_) => raw,
                Value::String(s) => serde_json::from_str(&s).map_err(|_| NodeError::new("Not a valid JSON").at(i))?,
                other => other,
            };
            data = parsed.as_object().cloned().unwrap_or_default();
        } else {
            for (k, v) in embed.as_object().into_iter().flatten() {
                if k == "inputMethod" {
                    continue;
                }
                let empty = matches!(v, Value::String(s) if s.is_empty());
                if !empty {
                    data.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(s) = data.get("author").and_then(Value::as_str).map(String::from) {
            data.insert("author".into(), json!({"name": s}));
        }
        if let Some(s) = data.get("color").and_then(Value::as_str).map(String::from) {
            let hex = s.trim_start_matches('#');
            if let Ok(n) = i64::from_str_radix(hex, 16) {
                data.insert("color".into(), json!(n));
            }
        }
        if let Some(url) = data.get("video").and_then(Value::as_str).map(String::from) {
            data.insert("video".into(), json!({"url": url, "width": 1270, "height": 720}));
        }
        if let Some(url) = data.get("thumbnail").and_then(Value::as_str).map(String::from) {
            data.insert("thumbnail".into(), json!({"url": url}));
        }
        if let Some(url) = data.get("image").and_then(Value::as_str).map(String::from) {
            data.insert("image".into(), json!({"url": url}));
        }
        if !data.is_empty() {
            out.push(Value::Object(data));
        }
    }
    Ok(out)
}

/// n8n's `prepareMultiPartForm`.
fn prepare_multipart_form(files: &[Value], payload: &Value, item: &Item, i: usize) -> NodeResult<reqwest::multipart::Form> {
    let mut attachments = Vec::new();
    let mut files_data: Vec<(String, String, Vec<u8>)> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let field_name = file.get("inputFieldName").and_then(Value::as_str).unwrap_or("data");
        let (bytes, meta) = binary_bytes(item, field_name, i)?;
        let mut filename = meta.get("fileName").and_then(Value::as_str).unwrap_or("file").to_string();
        if !filename.contains('.') {
            if let Some(ext) = meta.get("fileExtension").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                filename = format!("{filename}.{ext}");
            } else if let Some(mime) = meta.get("mimeType").and_then(Value::as_str) {
                filename = format!("{filename}.{}", mime_extension(mime));
            }
        }
        let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
        attachments.push(json!({"id": index, "filename": filename}));
        files_data.push((filename, mime, bytes));
    }
    let mut payload_obj = payload.as_object().cloned().unwrap_or_default();
    payload_obj.insert("attachments".into(), Value::Array(attachments));
    let payload_json = serde_json::to_string(&Value::Object(payload_obj)).unwrap_or_default();
    let mut form = reqwest::multipart::Form::new().text("payload_json", payload_json);
    for (index, (filename, mime, bytes)) in files_data.into_iter().enumerate() {
        let part = reqwest::multipart::Part::bytes(bytes).file_name(filename).mime_str(&mime).unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
        form = form.part(format!("files[{index}]"), part);
    }
    Ok(form)
}

// ---- response simplification -----------------------------------------------

/// n8n's `createSimplifyFunction(['id', 'channel_id', 'author', 'content', 'timestamp', 'type'])`.
fn simplify_message(value: Value) -> Value {
    const FIELDS: &[&str] = &["id", "channel_id", "author", "content", "timestamp", "type"];
    let Value::Object(obj) = value else { return value };
    let mut out = Map::new();
    for f in FIELDS {
        if let Some(v) = obj.get(*f) {
            out.insert((*f).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

/// n8n's `createSimplifyFunction(['user', 'roles', 'permissions'])`.
fn simplify_member(value: Value) -> Value {
    const FIELDS: &[&str] = &["user", "roles", "permissions"];
    let Value::Object(obj) = value else { return value };
    let mut out = Map::new();
    for f in FIELDS {
        if let Some(v) = obj.get(*f) {
            out.insert((*f).to_string(), v.clone());
        }
    }
    Value::Object(out)
}

// ---- per-resource dispatch --------------------------------------------------

impl Discord {
    async fn channel_get_all(&self, ctx: &ExecCtx<'_>, auth: &Auth, guild_id: &str) -> NodeResult<Value> {
        let return_all = ctx.param_bool("returnAll", 0, false)?;
        let mut response = discord_request(ctx, auth, 0, "GET", &format!("/guilds/{guild_id}/channels"), None, &[]).await?.as_array().cloned().unwrap_or_default();
        if !return_all {
            let limit = ctx.param_f64("limit", 0, 50.0)? as usize;
            response.truncate(limit);
        }
        if let Some(filter) = ctx.param("options", 0)?.get("filter").and_then(Value::as_array).cloned() {
            let types: Vec<i64> = filter.iter().filter_map(Value::as_i64).collect();
            response.retain(|c| c.get("type").and_then(Value::as_i64).map(|t| types.contains(&t)).unwrap_or(false));
        }
        Ok(Value::Array(response))
    }

    async fn member_get_all(&self, ctx: &ExecCtx<'_>, auth: &Auth, guild_id: &str) -> NodeResult<Value> {
        let return_all = ctx.param_bool("returnAll", 0, false)?;
        let after_param = ctx.param_str("after", 0, "")?;
        let mut response = Vec::new();
        if !return_all {
            let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
            let mut q = vec![("limit".to_string(), limit.to_string())];
            if !after_param.is_empty() {
                q.push(("after".to_string(), after_param));
            }
            response = discord_request(ctx, auth, 0, "GET", &format!("/guilds/{guild_id}/members"), None, &q).await?.as_array().cloned().unwrap_or_default();
        } else {
            let mut after = if after_param.is_empty() { None } else { Some(after_param) };
            loop {
                let mut q = vec![("limit".to_string(), "100".to_string())];
                if let Some(a) = &after {
                    q.push(("after".to_string(), a.clone()));
                }
                let page = discord_request(ctx, auth, 0, "GET", &format!("/guilds/{guild_id}/members"), None, &q).await?;
                let arr = page.as_array().cloned().unwrap_or_default();
                if arr.is_empty() {
                    break;
                }
                after = arr.last().and_then(|m| m.pointer("/user/id")).and_then(Value::as_str).map(String::from);
                response.extend(arr);
            }
        }
        if ctx.param_bool("options.simplify", 0, false)? {
            response = response.into_iter().map(simplify_member).collect();
        }
        Ok(Value::Array(response))
    }

    #[allow(clippy::too_many_arguments)]
    async fn channel(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, guild_id: &str, user_guilds: &[Value], is_oauth2: bool, i: usize) -> NodeResult<Value> {
        match operation {
            "create" => {
                let name = ctx.param_str("name", i, "")?;
                let typ = ctx.param_str("type", i, "0")?;
                let mut options = ctx.param("options", i)?.as_object().cloned().unwrap_or_default();
                if let Some(cat) = options.remove("categoryId") {
                    if let Some(cat_id) = locator_str(&cat).filter(|s| !s.is_empty()) {
                        options.insert("parent_id".into(), json!(cat_id));
                    }
                }
                let mut body = Map::new();
                body.insert("name".into(), json!(name));
                body.insert("type".into(), json!(typ));
                for (k, v) in options {
                    body.insert(k, v);
                }
                discord_request(ctx, auth, i, "POST", &format!("/guilds/{guild_id}/channels"), Some(Value::Object(body)), &[]).await
            }
            "get" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                discord_request(ctx, auth, i, "GET", &format!("/channels/{channel_id}"), None, &[]).await
            }
            "deleteChannel" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                discord_request(ctx, auth, i, "DELETE", &format!("/channels/{channel_id}"), None, &[]).await
            }
            "update" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                let name = ctx.param_str("name", i, "")?;
                let mut options = ctx.param("options", i)?.as_object().cloned().unwrap_or_default();
                if let Some(cat) = options.remove("categoryId") {
                    if let Some(cat_id) = locator_str(&cat).filter(|s| !s.is_empty()) {
                        options.insert("parent_id".into(), json!(cat_id));
                    }
                }
                let mut body = Map::new();
                body.insert("name".into(), json!(name));
                for (k, v) in options {
                    body.insert(k, v);
                }
                discord_request(ctx, auth, i, "PATCH", &format!("/channels/{channel_id}"), Some(Value::Object(body)), &[]).await
            }
            other => Err(unsupported("channel", other, i)),
        }
    }

    async fn member(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, guild_id: &str, i: usize) -> NodeResult<Value> {
        match operation {
            "roleAdd" | "roleRemove" => {
                let user_id = locator_str(&ctx.param("userId", i)?).unwrap_or_default();
                let roles = ctx.param("role", i)?.as_array().cloned().unwrap_or_default();
                let method = if operation == "roleAdd" { "PUT" } else { "DELETE" };
                for role in &roles {
                    let role_id = value_to_string(role);
                    discord_request(ctx, auth, i, method, &format!("/guilds/{guild_id}/members/{user_id}/roles/{role_id}"), None, &[]).await?;
                }
                Ok(json!({"success": true}))
            }
            other => Err(unsupported("member", other, i)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn message_send(&self, ctx: &ExecCtx<'_>, auth: &Auth, guild_id: &str, user_guilds: &[Value], is_oauth2: bool, i: usize, item: &Item) -> NodeResult<Value> {
        let content = ctx.param_str("content", i, "")?;
        let options = prepare_options(ctx.param("options", i)?.as_object().cloned().unwrap_or_default(), guild_id);
        let embeds_param = ctx.param("embeds", i)?;
        let embeds_values = embeds_param.get("values").and_then(Value::as_array).cloned();
        let files_param = ctx.param("files", i)?;
        let files_values = files_param.get("values").and_then(Value::as_array).cloned();

        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        for (k, v) in options {
            body.insert(k, v);
        }
        if let Some(embeds) = &embeds_values {
            body.insert("embeds".into(), json!(prepare_embeds(embeds, i)?));
        }

        let send_to = ctx.param_str("sendTo", i, "channel")?;
        let channel_id;
        if send_to == "user" {
            let user_id = locator_str(&ctx.param("userId", i)?).unwrap_or_default();
            if is_oauth2 {
                if let Err(e) = discord_request(ctx, auth, i, "GET", &format!("/guilds/{guild_id}/members/{user_id}"), None, &[]).await {
                    if e.http_code.as_deref() == Some("404") {
                        return Err(NodeError::new(format!("User with the id {user_id} is not a member of the selected guild")).at(i));
                    }
                    return Err(e);
                }
            }
            let created = discord_request(ctx, auth, i, "POST", "/users/@me/channels", Some(json!({"recipient_id": user_id})), &[]).await?;
            channel_id = created.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            if channel_id.is_empty() {
                return Err(NodeError::new("Could not create a channel to send direct message to").at(i));
            }
        } else {
            channel_id = locator_str(&ctx.param("channelId", i)?).unwrap_or_default();
            if is_oauth2 {
                check_access_to_channel(ctx, auth, &channel_id, user_guilds, i).await?;
            }
        }
        if channel_id.is_empty() {
            return Err(NodeError::new("Channel ID is required").at(i));
        }

        if let Some(files) = &files_values {
            if !files.is_empty() {
                let form = prepare_multipart_form(files, &Value::Object(body), item, i)?;
                return discord_multipart_request(ctx, auth, i, &format!("/channels/{channel_id}/messages"), form).await;
            }
        }
        discord_request(ctx, auth, i, "POST", &format!("/channels/{channel_id}/messages"), Some(Value::Object(body)), &[]).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn message(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, guild_id: &str, user_guilds: &[Value], is_oauth2: bool, i: usize, item: &Item) -> NodeResult<Value> {
        match operation {
            "send" => self.message_send(ctx, auth, guild_id, user_guilds, is_oauth2, i, item).await,
            "get" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                let message_id = ctx.param_str("messageId", i, "")?;
                let mut resp = discord_request(ctx, auth, i, "GET", &format!("/channels/{channel_id}/messages/{message_id}"), None, &[]).await?;
                if ctx.param_bool("options.simplify", i, false)? {
                    resp = simplify_message(resp);
                }
                Ok(resp)
            }
            "getAll" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let mut response;
                if !return_all {
                    let limit = ctx.param_f64("limit", 0, 50.0)? as i64;
                    response = discord_request(ctx, auth, i, "GET", &format!("/channels/{channel_id}/messages"), None, &[("limit".to_string(), limit.to_string())])
                        .await?
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                } else {
                    response = Vec::new();
                    let mut before: Option<String> = None;
                    loop {
                        let mut q = vec![("limit".to_string(), "100".to_string())];
                        if let Some(b) = &before {
                            q.push(("before".to_string(), b.clone()));
                        }
                        let page = discord_request(ctx, auth, i, "GET", &format!("/channels/{channel_id}/messages"), None, &q).await?;
                        let arr = page.as_array().cloned().unwrap_or_default();
                        if arr.is_empty() {
                            break;
                        }
                        before = arr.last().and_then(|m| m.get("id")).and_then(Value::as_str).map(String::from);
                        response.extend(arr);
                    }
                }
                if ctx.param_bool("options.simplify", i, false)? {
                    response = response.into_iter().map(simplify_message).collect();
                }
                Ok(Value::Array(response))
            }
            "deleteMessage" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                let message_id = ctx.param_str("messageId", i, "")?;
                discord_request(ctx, auth, i, "DELETE", &format!("/channels/{channel_id}/messages/{message_id}"), None, &[]).await?;
                Ok(json!({"success": true}))
            }
            "react" => {
                let channel_id = channel_id_param(ctx, auth, user_guilds, is_oauth2, i).await?;
                let message_id = ctx.param_str("messageId", i, "")?;
                let emoji = ctx.param_str("emoji", i, "")?;
                let encoded = percent_encode(&emoji);
                discord_request(ctx, auth, i, "PUT", &format!("/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"), None, &[]).await?;
                Ok(json!({"success": true}))
            }
            other => Err(unsupported("message", other, i)),
        }
    }

    async fn webhook(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize, item: &Item) -> NodeResult<Value> {
        match operation {
            "sendLegacy" => {
                let content = ctx.param_str("content", i, "")?;
                let mut options = ctx.param("options", i)?.as_object().cloned().unwrap_or_default();
                options = prepare_options(options, "");
                let wait = options.remove("wait").and_then(|v| v.as_bool()).unwrap_or(false);
                let embeds_param = ctx.param("embeds", i)?;
                let embeds_values = embeds_param.get("values").and_then(Value::as_array).cloned();
                let files_param = ctx.param("files", i)?;
                let files_values = files_param.get("values").and_then(Value::as_array).cloned();

                let mut body = Map::new();
                body.insert("content".into(), json!(content));
                for (k, v) in options {
                    body.insert(k, v);
                }
                if let Some(embeds) = &embeds_values {
                    body.insert("embeds".into(), json!(prepare_embeds(embeds, i)?));
                }

                if let Some(files) = &files_values {
                    if !files.is_empty() {
                        let form = prepare_multipart_form(files, &Value::Object(body), item, i)?;
                        return discord_multipart_request(ctx, auth, i, "", form).await;
                    }
                }
                let query: Vec<(String, String)> = if wait { vec![("wait".to_string(), "true".to_string())] } else { vec![] };
                discord_request(ctx, auth, i, "POST", "", Some(Value::Object(body)), &query).await
            }
            other => Err(unsupported("webhook", other, i)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_item(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, guild_id: &str, user_guilds: &[Value], is_oauth2: bool, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
        let value = match resource {
            "channel" => self.channel(ctx, auth, operation, guild_id, user_guilds, is_oauth2, i).await?,
            "message" => self.message(ctx, auth, operation, guild_id, user_guilds, is_oauth2, i, item).await?,
            "member" => self.member(ctx, auth, operation, guild_id, i).await?,
            "webhook" => self.webhook(ctx, auth, operation, i, item).await?,
            other => return Err(unsupported(other, operation, i)),
        };
        Ok(to_items(value, i))
    }
}

#[async_trait::async_trait]
impl NodeType for Discord {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.discord"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let authentication = ctx.param_str("authentication", 0, "botToken")?;
        let is_webhook_auth = authentication == "webhook";
        let resource = if is_webhook_auth { "webhook".to_string() } else { ctx.param_str("resource", 0, "channel")? };
        let operation = ctx.param_str("operation", 0, "")?;
        let auth = resolve_auth(ctx, &authentication).await?;
        let is_oauth2 = authentication == "oAuth2";

        let mut guild_id = String::new();
        let mut user_guilds: Vec<Value> = Vec::new();
        if resource != "webhook" {
            guild_id = locator_str(&ctx.param("guildId", 0)?).unwrap_or_default();
            if is_oauth2 {
                user_guilds = discord_guild_list(ctx, &auth, 0).await?;
                check_access_to_guild(&guild_id, &user_guilds, 0)?;
            }
        }

        // `channel.getAll`/`member.getAll` run once per node execution,
        // ignoring input item count -- see module docs.
        if resource == "channel" && operation == "getAll" {
            return match self.channel_get_all(ctx, &auth, &guild_id).await {
                Ok(value) => Ok(vec![to_items(value, 0)]),
                Err(e) if ctx.continue_on_fail() => {
                    ctx.push_error_item(&e, 0);
                    Ok(vec![Vec::new()])
                }
                Err(e) => Err(e),
            };
        }
        if resource == "member" && operation == "getAll" {
            return match self.member_get_all(ctx, &auth, &guild_id).await {
                Ok(value) => Ok(vec![to_items(value, 0)]),
                Err(e) if ctx.continue_on_fail() => {
                    ctx.push_error_item(&e, 0);
                    Ok(vec![Vec::new()])
                }
                Err(e) => Err(e),
            };
        }

        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            match self.run_item(ctx, &auth, &resource, &operation, &guild_id, &user_guilds, is_oauth2, i, item).await {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
