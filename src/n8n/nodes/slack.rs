//! Slack node (spec §6.6), v2.x (2.2/2.3), faithful to n8n's
//! `nodes/Slack/V2/*`. Implements the most-used resources/operations:
//!
//! - `message`: post, update, delete, getPermalink, search (`sendAndWait`
//!   excluded per spec)
//! - `channel`: create, get, getAll, history, invite, join, kick, leave,
//!   rename, archive, unarchive, setPurpose, setTopic
//! - `user`: info, getAll, getPresence
//! - `reaction`: add, get, remove
//! - `file`: upload (v2.2+ external-upload flow: getUploadURLExternal ->
//!   raw upload -> completeUploadExternal)
//!
//! Anything else (message: schedule/deleteScheduled/getManyScheduled;
//! channel: close/member/open/replies; user: lookupByEmail/getProfile/
//! updateProfile; file: get/getAll; resources `star`/`userGroup`) returns a
//! clear "not supported natively yet" error.
//!
//! Faithful quirks kept from n8n's `GenericFunctions.js`:
//! - Any Slack response with a top-level `ts` has it renamed to
//!   `message_timestamp` (the field is deleted, not duplicated).
//! - `otherOptions.sendAsUser` and `otherOptions.includeLinkToWorkflow` are
//!   read for their own purposes but never deleted from `otherOptions`, so
//!   they leak into the outgoing `chat.postMessage`/`chat.postEphemeral`
//!   body as literal (harmless, Slack ignores unknown fields) extra keys.
//! - `includeLinkToWorkflow` defaults to `true` from node version 2.1, so
//!   for our 2.2/2.3 target it defaults on: an attribution line/block/
//!   attachment pointing back at this workflow is appended unless the user
//!   explicitly disabled it.
//!
//! Known simplifications vs real n8n (see the implementation report):
//! `instanceId` is always empty (r8r has no persisted instance id), and
//! `slackOAuth2Api` expects the credential's `oauthTokenData.access_token`
//! to already be populated (no interactive 3-legged OAuth dance, and no
//! `authed_user.access_token` nesting).

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use serde_json::{json, Map, Value};

pub struct Slack;

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

fn locator_mode(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("mode").and_then(Value::as_str).unwrap_or("id").to_string(),
        _ => "id".to_string(),
    }
}

/// n8n's `toMultiOptionsCsv`: accepts an array or a comma-separated string.
fn multi_csv(v: &Value) -> String {
    match v {
        Value::Array(a) => a.iter().map(value_to_string).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(","),
        Value::String(s) => s.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(","),
        _ => String::new(),
    }
}

fn channel_id(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("channelId", i)?;
    Ok(locator_str(&v).unwrap_or_default())
}

fn user_value(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("user", i)?;
    Ok(locator_str(&v).unwrap_or_default())
}

/// n8n's `getTarget`: a channel id/name, or a user id/@-prefixed username.
fn get_target(ctx: &ExecCtx<'_>, i: usize, select: &str) -> NodeResult<String> {
    if select == "channel" {
        channel_id(ctx, i)
    } else {
        let v = ctx.param("user", i)?;
        let val = locator_str(&v).unwrap_or_default();
        if locator_mode(&v) == "username" && !val.starts_with('@') {
            Ok(format!("@{val}"))
        } else {
            Ok(val)
        }
    }
}

fn to_epoch_seconds(s: &str) -> NodeResult<String> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok((dt.timestamp_millis() as f64 / 1000.0).to_string());
    }
    Err(NodeError::new(format!("\"{s}\" is not a valid date")))
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("Slack \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
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

// ---- authentication & the underlying HTTP call -----------------------------

struct Auth {
    bearer: String,
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let method = ctx.param_str("authentication", 0, "accessToken")?;
    if method == "oAuth2" {
        let (_, cred) = ctx.credentials("slackOAuth2Api").await?;
        let token = cred
            .pointer("/oauthTokenData/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                NodeError::new("The Slack OAuth2 credential is not connected").describe("Complete the OAuth2 authorization for this credential before using it.")
            })?
            .to_string();
        let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://slack.com/api").trim_end_matches('/').to_string();
        Ok(Auth { bearer: token, base_url })
    } else {
        let (_, cred) = ctx.credentials("slackApi").await?;
        let token = cred["accessToken"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            return Err(NodeError::new("Slack credentials are not set").describe("Add an Access Token to the Slack API credential."));
        }
        let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://slack.com/api").trim_end_matches('/').to_string();
        Ok(Auth { bearer: token, base_url })
    }
}

/// Maps a Slack `{ok: false, error: "..."}` payload the way n8n's
/// `throwOnSlackApiError` does.
fn slack_error(resource: &str, body: &Value) -> NodeError {
    let err = body["error"].as_str().unwrap_or("unknown_error");
    match err {
        "paid_teams_only" => NodeError::new(format!("Your current Slack plan does not include the resource '{resource}'"))
            .describe("Hint: Upgrade to a Slack plan that includes the functionality you want to use."),
        "ratelimited" | "rate_limited" => NodeError::new(format!("Slack error response: \"{err}\""))
            .describe("Wait before running this again, or request less data at a time. Limits differ per operation - see the Slack Documentation - https://docs.slack.dev/apis/web-api/rate-limits"),
        "missing_scope" => {
            let needed = body["needed"].as_str().unwrap_or("");
            NodeError::new("Your Slack credential is missing required Oauth Scopes").describe(format!("Add the following scope(s) to your Slack App: {needed}"))
        }
        "not_allowed_token_type" | "invalid_action_token" => NodeError::new("This Slack operation requires a user token").describe(
            "Bot tokens are not accepted here. Use OAuth2 authentication, or an Access Token credential holding a user token (starts with \"xoxp-\").",
        ),
        "not_admin" => NodeError::new("Need higher Role Level for this Operation (e.g. Owner or Admin Rights)").describe(
            "Hint: Check the Role of your Slack App Integration. For more information see the Slack Documentation - https://slack.com/help/articles/360018112273-Types-of-roles-in-Slack",
        ),
        other => NodeError::new(format!("Slack error response: \"{other}\"")),
    }
}

/// n8n's `ts` -> `message_timestamp` rename, applied to every response.
fn rename_ts(mut value: Value) -> Value {
    if let Value::Object(obj) = &mut value {
        if let Some(ts) = obj.remove("ts") {
            obj.insert("message_timestamp".into(), ts);
        }
    }
    value
}

#[allow(clippy::too_many_arguments)]
async fn slack_request(ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, i: usize, method: &str, path: &str, body: Option<Value>, query: &[(String, String)]) -> NodeResult<Value> {
    let url_str = if path.starts_with("http") { path.to_string() } else { format!("{}{}", auth.base_url, path) };
    let mut url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid Slack API URL: {url_str}")).at(i))?;
    check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let mut req = ctx.services.http.request(method, url).bearer_auth(&auth.bearer);
    if let Some(b) = &body {
        req = req.json(b);
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Slack failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if status >= 400 {
        return Err(NodeError::api(format!("Slack API request failed with status code {status}"), Some(status), Some(text)).at(i));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if value["ok"] == Value::Bool(false) {
        return Err(slack_error(resource, &value).at(i));
    }
    Ok(rename_ts(value))
}

/// Cursor/legacy pagination, mirroring `slackApiRequestAllItems`: collects
/// `response[property]` (or `response[property].matches`, for
/// `search.messages`'s shape) across pages until Slack stops offering more.
#[allow(clippy::too_many_arguments)]
async fn paginate(ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, i: usize, method: &str, path: &str, body: Option<Value>, mut query: Vec<(String, String)>, property: &str) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    if path.contains("files.list") {
        query.push(("count".into(), "100".into()));
    } else {
        query.push(("limit".into(), "100".into()));
    }
    let mut page = 1u32;
    let mut cursor: Option<String> = None;
    loop {
        let mut q = query.clone();
        q.push(("page".into(), page.to_string()));
        if let Some(c) = &cursor {
            q.push(("cursor".into(), c.clone()));
        }
        let resp = slack_request(ctx, auth, resource, i, method, path, body.clone(), &q).await?;
        let prop = resp.get(property).cloned().unwrap_or(Value::Null);
        let (batch, prop_pages_more) = match &prop {
            Value::Object(o) => {
                let matches = o.get("matches").and_then(Value::as_array).cloned().unwrap_or_default();
                let more = o.get("paging").map(|p| p["page"].as_u64().unwrap_or(0) < p["pages"].as_u64().unwrap_or(0)).unwrap_or(false);
                (matches, more)
            }
            Value::Array(a) => (a.clone(), false),
            _ => (Vec::new(), false),
        };
        out.extend(batch);
        cursor = resp.pointer("/response_metadata/next_cursor").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
        let top_paging_more = resp.pointer("/paging").map(|p| p["page"].as_u64().unwrap_or(0) < p["pages"].as_u64().unwrap_or(0)).unwrap_or(false);
        page += 1;
        if cursor.is_none() && !top_paging_more && !prop_pages_more {
            break;
        }
    }
    Ok(out)
}

// ---- message content (n8n's `getMessageContent`) ---------------------------

fn automated_message(ctx: &ExecCtx<'_>) -> String {
    let base = &ctx.services.config.webhook_url;
    let workflow_id = ctx.workflow.id.clone().unwrap_or_default();
    format!("_Automated with this <{base}workflow/{workflow_id}?utm_source=n8n-internal&utm_medium=powered_by&utm_campaign=n8n-nodes-base.slack|n8n workflow>_")
}

fn message_content(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Map<String, Value>> {
    let include_link = ctx.param_bool("otherOptions.includeLinkToWorkflow", i, true)?;
    let automated = automated_message(ctx);
    let message_type = ctx.param_str("messageType", i, "text")?;
    let mut content = Map::new();
    match message_type.as_str() {
        "text" => {
            let text = ctx.param_str("text", i, "")?;
            let final_text = if include_link { format!("{text}\n{automated}") } else { text };
            content.insert("text".into(), json!(final_text));
        }
        "block" => {
            let raw = ctx.param("blocksUi", i)?;
            let parsed = match raw {
                Value::String(s) if !s.trim().is_empty() => {
                    serde_json::from_str::<Value>(&s).map_err(|e| NodeError::new(format!("Blocks must be valid JSON: {e}")).at(i))?
                }
                Value::String(_) => Value::Object(Map::new()),
                other => other,
            };
            let mut obj = parsed.as_object().cloned().ok_or_else(|| NodeError::new("Blocks must be a JSON object").at(i))?;
            if include_link {
                let blocks = obj.entry("blocks").or_insert_with(|| json!([]));
                if let Some(arr) = blocks.as_array_mut() {
                    arr.push(json!({"type": "section", "text": {"type": "mrkdwn", "text": automated}}));
                }
            }
            let text = ctx.param_str("text", i, "")?;
            if !text.is_empty() {
                obj.insert("text".into(), json!(text));
            }
            content = obj;
        }
        "attachment" => {
            let raw = ctx.param("attachments", i)?;
            let mut attachments: Vec<Value> = raw
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|mut a| {
                    if let Value::Object(o) = &mut a {
                        if let Some(item) = o.get("fields").and_then(|f| f.get("item")).cloned() {
                            o.insert("fields".into(), item);
                        }
                    }
                    a
                })
                .collect();
            if include_link {
                attachments.push(json!({"text": automated}));
            }
            content.insert("attachments".into(), Value::Array(attachments));
        }
        other => return Err(NodeError::new(format!("The message type \"{other}\" is not known!")).at(i)),
    }
    Ok(content)
}

/// Builds the `chat.postMessage`/`chat.postEphemeral` body, faithfully
/// replicating n8n's handling of `otherOptions` (thread reply, ephemeral,
/// bot profile, and the `sendAsUser`/`includeLinkToWorkflow` leak -- see the
/// module docs).
fn post_message_body(ctx: &ExecCtx<'_>, i: usize, authentication: &str) -> NodeResult<(Value, &'static str)> {
    let select = ctx.param_str("select", i, "channel")?;
    let target = get_target(ctx, i, &select)?;
    let content = message_content(ctx, i)?;
    let mut body = Map::new();
    body.insert("channel".into(), json!(target));
    for (k, v) in content {
        body.insert(k, v);
    }

    let mut other_options = ctx.param("otherOptions", i)?.as_object().cloned().unwrap_or_default();

    if authentication == "accessToken" {
        if let Some(send_as_user) = other_options.get("sendAsUser").and_then(Value::as_str) {
            if !send_as_user.is_empty() {
                body.insert("username".into(), json!(send_as_user));
            }
        }
    }

    let mut action: &'static str = "postMessage";
    if let Some(ephemeral) = other_options.get("ephemeral").cloned() {
        if select == "channel" {
            if let Some(user_rlc) = ephemeral.pointer("/ephemeralValues/user") {
                let mode = locator_mode(user_rlc);
                let val = locator_str(user_rlc).unwrap_or_default();
                let user_val = if mode == "username" && !val.starts_with('@') { format!("@{val}") } else { val };
                body.insert("user".into(), json!(user_val));
                action = "postEphemeral";
            }
        } else if select == "user" && ephemeral.as_bool() == Some(true) {
            body.insert("user".into(), json!(target));
            action = "postEphemeral";
        }
    }

    if let Some(reply) = other_options.get("thread_ts").and_then(|t| t.pointer("/replyValues")).cloned() {
        if let Some(ts) = reply.get("thread_ts").filter(|v| !v.is_null()) {
            body.insert("thread_ts".into(), json!(value_to_string(ts)));
        }
        if let Some(rb) = reply.get("reply_broadcast") {
            body.insert("reply_broadcast".into(), rb.clone());
        }
    }
    other_options.remove("thread_ts");
    other_options.remove("ephemeral");

    if let Some(values) = other_options.get("botProfile").and_then(|b| b.pointer("/imageValues")).cloned() {
        if values.get("profilePhotoType").and_then(Value::as_str) == Some("image") {
            body.insert("icon_url".into(), values.get("icon_url").cloned().unwrap_or(json!("")));
        } else {
            body.insert("icon_emoji".into(), values.get("icon_emoji").cloned().unwrap_or(json!("")));
        }
    }
    other_options.remove("botProfile");

    if select == "user" && action == "postEphemeral" {
        let user_param = ctx.param("user", i)?;
        if locator_mode(&user_param) == "username" {
            return Err(NodeError::new("You cannot send ephemeral messages using User type \"By username\". Please use \"From List\" or \"By ID\".").at(i));
        }
    }

    for (k, v) in other_options {
        body.insert(k, v);
    }

    Ok((Value::Object(body), action))
}

fn update_body(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Value> {
    let channel = channel_id(ctx, i)?;
    let ts = ctx.param_str("ts", i, "")?;
    let content = message_content(ctx, i)?;
    let mut body = Map::new();
    body.insert("channel".into(), json!(channel));
    body.insert("ts".into(), json!(ts));
    for (k, v) in content {
        body.insert(k, v);
    }
    if let Some(obj) = ctx.param("updateFields", i)?.as_object() {
        for (k, v) in obj {
            body.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Object(body))
}

// ---- file upload (v2.2+ external-upload flow) ------------------------------

async fn file_upload(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, item: &Item) -> NodeResult<Value> {
    let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let (bytes, meta) = binary_bytes(item, &binary_prop, i)?;
    let bin_file_name = meta.get("fileName").and_then(Value::as_str).unwrap_or("file").to_string();
    let mime = meta.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let options = ctx.param("options", i)?;

    let mut complete_body = Map::new();
    if let Some(cid) = options.get("channelId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        complete_body.insert("channel_id".into(), json!(cid));
    }
    if let Some(ic) = options.get("initialComment").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        complete_body.insert("initial_comment".into(), json!(ic));
    }
    if let Some(tt) = options.get("threadTs").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        complete_body.insert("thread_ts".into(), json!(tt));
    }
    let upload_name = options.get("fileName").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| bin_file_name.clone());
    let title = options.get("title").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| bin_file_name.clone());

    // Step 1: get an external upload URL.
    let url_resp = slack_request(ctx, auth, "file", i, "GET", "/files.getUploadURLExternal", None, &[("filename".into(), upload_name), ("length".into(), bytes.len().to_string())]).await?;
    let upload_url = url_resp["upload_url"].as_str().ok_or_else(|| NodeError::new("Slack did not return an upload URL").at(i))?.to_string();
    let file_id = url_resp["file_id"].as_str().unwrap_or_default().to_string();

    // Step 2: upload the bytes to that URL.
    let part = reqwest::multipart::Part::bytes(bytes).file_name(bin_file_name).mime_str(&mime).unwrap_or_else(|_| reqwest::multipart::Part::bytes(Vec::new()));
    let form = reqwest::multipart::Form::new().part("file", part);
    let upload_url_parsed = reqwest::Url::parse(&upload_url).map_err(|_| NodeError::new(format!("Invalid Slack upload URL: {upload_url}")).at(i))?;
    check_ssrf(&upload_url_parsed, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    let resp = ctx
        .services
        .http
        .post(upload_url_parsed)
        .bearer_auth(&auth.bearer)
        .multipart(form)
        .send()
        .await
        .map_err(|e| NodeError::api(format!("The Slack file upload request failed: {e}"), None, None).at(i))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        return Err(NodeError::api(format!("Slack file upload failed with status code {status}"), Some(status), None).at(i));
    }

    // Step 3: complete the upload.
    complete_body.insert("files".into(), json!([{"id": file_id, "title": title}]));
    let final_resp = slack_request(ctx, auth, "file", i, "POST", "/files.completeUploadExternal", Some(Value::Object(complete_body)), &[]).await?;
    Ok(final_resp.get("files").cloned().unwrap_or(json!([])))
}

// ---- per-resource dispatch --------------------------------------------------

impl Slack {
    async fn message(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Value> {
        match operation {
            "post" => {
                let authentication = ctx.param_str("authentication", 0, "accessToken")?;
                let (body, action) = post_message_body(ctx, i, &authentication)?;
                slack_request(ctx, auth, "message", i, "POST", &format!("/chat.{action}"), Some(body), &[]).await
            }
            "update" => {
                let body = update_body(ctx, i)?;
                slack_request(ctx, auth, "message", i, "POST", "/chat.update", Some(body), &[]).await
            }
            "delete" => {
                let select = ctx.param_str("select", i, "channel")?;
                let target = get_target(ctx, i, &select)?;
                let ts = ctx.param_str("timestamp", i, "")?;
                let body = json!({"channel": target, "ts": ts});
                slack_request(ctx, auth, "message", i, "POST", "/chat.delete", Some(body), &[]).await
            }
            "getPermalink" => {
                let channel = channel_id(ctx, i)?;
                let ts = ctx.param_str("timestamp", i, "")?;
                slack_request(ctx, auth, "message", i, "GET", "/chat.getPermalink", None, &[("channel".into(), channel), ("message_ts".into(), ts)]).await
            }
            "search" => {
                let mut query_text = ctx.param_str("query", i, "")?;
                let sort = ctx.param_str("sort", i, "desc")?;
                let sort_by = if sort == "relevance" { "score" } else { "timestamp" };
                let sort_dir = if sort == "asc" { "asc" } else { "desc" };
                let options = ctx.param("options", i)?;
                for channel in options.get("searchChannel").and_then(Value::as_array).into_iter().flatten() {
                    query_text.push_str(&format!(" in:{}", value_to_string(channel)));
                }
                let return_all = ctx.param_bool("returnAll", i, false)?;
                if return_all {
                    let query = vec![("query".into(), query_text), ("sort".into(), sort_by.into()), ("sort_dir".into(), sort_dir.into())];
                    let matches = paginate(ctx, auth, "message", i, "GET", "/search.messages", None, query, "messages").await?;
                    Ok(Value::Array(matches))
                } else {
                    let limit = ctx.param_f64("limit", i, 25.0)? as i64;
                    let query = [
                        ("query".to_string(), query_text),
                        ("sort".to_string(), sort_by.to_string()),
                        ("sort_dir".to_string(), sort_dir.to_string()),
                        ("count".to_string(), limit.to_string()),
                    ];
                    let resp = slack_request(ctx, auth, "message", i, "POST", "/search.messages", None, &query).await?;
                    Ok(resp.pointer("/messages/matches").cloned().unwrap_or(json!([])))
                }
            }
            other => Err(unsupported("message", other, i)),
        }
    }

    async fn channel(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Value> {
        match operation {
            "archive" => {
                let channel = channel_id(ctx, i)?;
                slack_request(ctx, auth, "channel", i, "POST", "/conversations.archive", Some(json!({"channel": channel})), &[]).await
            }
            "unarchive" => {
                let channel = channel_id(ctx, i)?;
                slack_request(ctx, auth, "channel", i, "POST", "/conversations.unarchive", Some(json!({"channel": channel})), &[]).await
            }
            "create" => {
                let mut name = channel_id(ctx, i)?;
                if let Some(stripped) = name.strip_prefix('#') {
                    name = stripped.to_string();
                }
                let visibility = ctx.param_str("channelVisibility", i, "public")?;
                let body = json!({"name": name, "is_private": visibility == "private"});
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.create", Some(body), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "get" => {
                let channel = channel_id(ctx, i)?;
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.info", None, &[("channel".into(), channel)]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "getAll" => {
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let filters = ctx.param("filters", i)?;
                let mut query = Vec::new();
                if let Some(types) = filters.get("types") {
                    let csv = multi_csv(types);
                    if !csv.is_empty() {
                        query.push(("types".into(), csv));
                    }
                }
                if filters.get("excludeArchived").and_then(Value::as_bool) == Some(true) {
                    query.push(("exclude_archived".into(), "true".into()));
                }
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, "channel", i, "GET", "/conversations.list", None, query, "channels").await?))
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    query.push(("limit".into(), limit.to_string()));
                    let resp = slack_request(ctx, auth, "channel", i, "GET", "/conversations.list", None, &query).await?;
                    Ok(resp.get("channels").cloned().unwrap_or(json!([])))
                }
            }
            "history" => {
                let channel = channel_id(ctx, i)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let filters = ctx.param("filters", i)?;
                let mut query = vec![("channel".into(), channel)];
                if filters.get("inclusive").and_then(Value::as_bool) == Some(true) {
                    query.push(("inclusive".into(), "true".into()));
                }
                if let Some(latest) = filters.get("latest").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    query.push(("latest".into(), to_epoch_seconds(latest).map_err(|e| e.at(i))?));
                }
                if let Some(oldest) = filters.get("oldest").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                    query.push(("oldest".into(), to_epoch_seconds(oldest).map_err(|e| e.at(i))?));
                }
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, "channel", i, "GET", "/conversations.history", None, query, "messages").await?))
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    query.push(("limit".into(), limit.to_string()));
                    let resp = slack_request(ctx, auth, "channel", i, "GET", "/conversations.history", None, &query).await?;
                    Ok(resp.get("messages").cloned().unwrap_or(json!([])))
                }
            }
            "invite" => {
                let channel = channel_id(ctx, i)?;
                let user_ids = multi_csv(&ctx.param("userIds", i)?);
                let body = json!({"channel": channel, "users": user_ids});
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.invite", Some(body), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "join" => {
                let channel = channel_id(ctx, i)?;
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.join", Some(json!({"channel": channel})), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "kick" => {
                let channel = channel_id(ctx, i)?;
                let user_id = ctx.param_str("userId", i, "")?;
                slack_request(ctx, auth, "channel", i, "POST", "/conversations.kick", Some(json!({"channel": channel, "user": user_id})), &[]).await
            }
            "leave" => {
                let channel = channel_id(ctx, i)?;
                slack_request(ctx, auth, "channel", i, "POST", "/conversations.leave", Some(json!({"channel": channel})), &[]).await
            }
            "rename" => {
                let channel = channel_id(ctx, i)?;
                let name = ctx.param_str("name", i, "")?;
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.rename", Some(json!({"channel": channel, "name": name})), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "setPurpose" => {
                let channel = channel_id(ctx, i)?;
                let purpose = ctx.param_str("purpose", i, "")?;
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.setPurpose", Some(json!({"channel": channel, "purpose": purpose})), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            "setTopic" => {
                let channel = channel_id(ctx, i)?;
                let topic = ctx.param_str("topic", i, "")?;
                let resp = slack_request(ctx, auth, "channel", i, "POST", "/conversations.setTopic", Some(json!({"channel": channel, "topic": topic})), &[]).await?;
                Ok(resp.get("channel").cloned().unwrap_or(resp))
            }
            other => Err(unsupported("channel", other, i)),
        }
    }

    async fn user(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Value> {
        match operation {
            "info" => {
                let user = user_value(ctx, i)?;
                let resp = slack_request(ctx, auth, "user", i, "GET", "/users.info", None, &[("user".into(), user)]).await?;
                Ok(resp.get("user").cloned().unwrap_or(resp))
            }
            "getAll" => {
                let return_all = ctx.param_bool("returnAll", i, false)?;
                if return_all {
                    Ok(Value::Array(paginate(ctx, auth, "user", i, "GET", "/users.list", None, Vec::new(), "members").await?))
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    let resp = slack_request(ctx, auth, "user", i, "GET", "/users.list", None, &[("limit".into(), limit.to_string())]).await?;
                    Ok(resp.get("members").cloned().unwrap_or(json!([])))
                }
            }
            "getPresence" => {
                let user = user_value(ctx, i)?;
                slack_request(ctx, auth, "user", i, "GET", "/users.getPresence", None, &[("user".into(), user)]).await
            }
            other => Err(unsupported("user", other, i)),
        }
    }

    async fn reaction(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Value> {
        let channel = channel_id(ctx, i)?;
        let timestamp = ctx.param_str("timestamp", i, "")?;
        match operation {
            "add" => {
                let name = ctx.param_str("name", i, "")?;
                slack_request(ctx, auth, "reaction", i, "POST", "/reactions.add", Some(json!({"channel": channel, "name": name, "timestamp": timestamp})), &[]).await
            }
            "remove" => {
                let name = ctx.param_str("name", i, "")?;
                slack_request(ctx, auth, "reaction", i, "POST", "/reactions.remove", Some(json!({"channel": channel, "name": name, "timestamp": timestamp})), &[]).await
            }
            "get" => slack_request(ctx, auth, "reaction", i, "GET", "/reactions.get", None, &[("channel".into(), channel), ("timestamp".into(), timestamp)]).await,
            other => Err(unsupported("reaction", other, i)),
        }
    }

    async fn file(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize, item: &Item) -> NodeResult<Value> {
        match operation {
            "upload" => file_upload(ctx, auth, i, item).await,
            other => Err(unsupported("file", other, i)),
        }
    }

    async fn run_item(&self, ctx: &ExecCtx<'_>, auth: &Auth, resource: &str, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
        let value = match resource {
            "message" => self.message(ctx, auth, operation, i).await?,
            "channel" => self.channel(ctx, auth, operation, i).await?,
            "user" => self.user(ctx, auth, operation, i).await?,
            "reaction" => self.reaction(ctx, auth, operation, i).await?,
            "file" => self.file(ctx, auth, operation, i, item).await?,
            other => return Err(NodeError::new(format!("Slack resource \"{other}\" is not supported natively yet")).at(i)),
        };
        Ok(to_items(value, i))
    }
}

#[async_trait::async_trait]
impl NodeType for Slack {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.slack"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "message")?;
        let operation = ctx.param_str("operation", 0, "post")?;
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
