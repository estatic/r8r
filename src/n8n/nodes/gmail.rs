//! Gmail node (spec §6.6), v2.x (2/2.1/2.2), faithful to n8n's
//! `nodes/Google/Gmail/v2/*`. Implements the `message` resource's send,
//! reply, get, getAll, delete, markAsRead, markAsUnread, addLabels and
//! removeLabels operations; the `draft` resource's create, get, getAll and
//! delete; the `label` resource's create, get, getAll and delete; and the
//! `thread` resource's get, getAll, delete, reply, trash, untrash,
//! addLabels and removeLabels. The Gmail Trigger node and the
//! `sendAndWait` message operation are out of scope. Anything else returns
//! `Gmail "<resource>" / "<operation>" is not supported natively yet`.
//!
//! Authentication (OAuth2 with 401 -> refresh -> retry + persist, and
//! service-account RS256 JWT exchange with impersonation via
//! `delegatedEmail`) is shared with the Google Sheets node via
//! `google_auth.rs`.
//!
//! MIME messages are built by hand on top of lettre's `SinglePart`/
//! `MultiPart`/`Attachment` builders (which produce the body's own
//! `Content-Type`/`Content-Transfer-Encoding` headers, boundaries and
//! base64/quoted-printable encoding) with our own top-level headers
//! (To/Cc/Bcc/From/Subject/Reply-To/In-Reply-To/References/Date) prepended
//! -- lettre's full `Message` builder requires a `From` header, which
//! n8n's raw messages often omit (Gmail then fills it in with the
//! authenticated account). The base64url `raw` field is then decoded by a
//! small hand-rolled MIME parser for `get`/`getAll` with `simple: false`
//! (no mail-parsing crate is in the dependency tree); it handles
//! multipart/mixed/alternative, base64 and quoted-printable bodies, which
//! covers what Gmail's API itself produces, but -- unlike n8n's
//! `mailparser` dependency -- does not decode non-UTF-8 charsets, RFC 2047
//! encoded-words, or fully RFC 5322-compliant address/comment syntax.
//!
//! The "Append n8n Attribution" text is reproduced byte-for-byte from
//! n8n's `GenericFunctions.prepareEmailBody` (including the literal word
//! "n8n" and the link to n8n.io), matching the Send Email node's choice
//! (see `email.rs`): the option controls whether n8n's own attribution
//! notice is appended, and changing its wording would be a different
//! feature.
//!
//! Known simplifications vs real n8n (see the implementation report):
//! no RFC 2047 subject/address encoding on send, no MIME line folding,
//! address-list parsing is comma-split rather than a full RFC 5322
//! parser, and `receivedAfter`/`receivedBefore` timestamp parsing accepts
//! RFC 3339 strings and raw (seconds or millisecond) integers rather than
//! every format Luxon's `DateTime.fromISO` accepts.

use super::google_auth::{self, GoogleAuth};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine as _;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, MultiPart, SinglePart};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub struct Gmail;

const GMAIL_SCOPES: &str = "https://www.googleapis.com/auth/gmail.labels https://www.googleapis.com/auth/gmail.addons.current.action.compose https://www.googleapis.com/auth/gmail.addons.current.message.action https://mail.google.com/ https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/gmail.compose";
const METADATA_HEADERS: [&str; 5] = ["From", "To", "Cc", "Bcc", "Subject"];

fn unsupported(resource: &str, operation: &str) -> NodeError {
    NodeError::new(format!("Gmail \"{resource}\" / \"{operation}\" is not supported natively yet"))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---- authentication & the underlying HTTP call -----------------------------
// (shared with the Google Sheets node; see `google_auth.rs`.)

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<GoogleAuth> {
    google_auth::resolve_auth(ctx, "gmailOAuth2", "Gmail", GMAIL_SCOPES, "https://www.googleapis.com", "oAuth2").await
}

/// Maps a Gmail API `{error: {message, ...}}` error payload the way n8n's
/// `googleApiRequest` catch block does: a few status-specific hints keyed
/// off the current `resource`, else the raw Google message.
fn gmail_error(status: u16, body: &Value, resource: &str) -> NodeError {
    let message = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("").to_string();
    if status == 400 && message.contains("Invalid id value") {
        return NodeError::api(format!("Invalid {resource} ID"), Some(status), Some(body.to_string()))
            .describe(format!("{} IDs should look something like this: 182b676d244938bd", capitalize(resource)));
    }
    if status == 404 {
        let label = if resource == "label" { "label ID" } else { resource };
        return NodeError::api(format!("{} not found", capitalize(label)), Some(status), Some(body.to_string()));
    }
    if status == 409 && resource == "label" {
        return NodeError::api("Label name exists already", Some(status), Some(body.to_string()));
    }
    let msg = if message.is_empty() { format!("Gmail API request failed with status code {status}") } else { message };
    NodeError::api(msg, Some(status), Some(body.to_string()))
}

async fn gmail_request(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, method: &str, endpoint: &str, body: Option<Value>, query: &[(String, String)], resource: &str) -> NodeResult<Value> {
    let url = format!("{}{endpoint}", auth.base);
    google_auth::api_request(ctx, auth, method, &url, body, query, |status, b| gmail_error(status, b, resource)).await
}

fn metadata_query() -> Vec<(String, String)> {
    let mut q = vec![("format".to_string(), "metadata".to_string())];
    for h in METADATA_HEADERS {
        q.push(("metadataHeaders".to_string(), h.to_string()));
    }
    q
}

/// n8n's `googleApiRequestAllItems`: pages through `nextPageToken` with
/// `maxResults=100` per page, collecting `property` from each response.
async fn request_all_items(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, property: &str, endpoint: &str, resource: &str, base_query: &[(String, String)]) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut q: Vec<(String, String)> = base_query.to_vec();
        q.push(("maxResults".to_string(), "100".to_string()));
        if let Some(t) = &page_token {
            q.push(("pageToken".to_string(), t.clone()));
        }
        let resp = gmail_request(ctx, auth, "GET", endpoint, None, &q, resource).await?;
        if let Some(arr) = resp.get(property).and_then(Value::as_array) {
            out.extend(arr.iter().cloned());
        }
        match resp.get("nextPageToken").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => page_token = Some(t.to_string()),
            _ => break,
        }
    }
    Ok(out)
}

// ---- search query / timestamp helpers (n8n's `prepareQuery`) ---------------

fn prepare_timestamp(value: &str, i: usize, label: &str) -> NodeResult<i64> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(dt.timestamp());
    }
    if let Ok(n) = value.trim().parse::<i64>() {
        return Ok(if value.trim().len() < 12 { n } else { n / 1000 });
    }
    Err(NodeError::new(format!("Invalid date/time in 'Received {}' field", capitalize(label)))
        .describe(format!("'{value}' isn't a valid date and time. If you're using an expression, be sure to set an ISO date string or a timestamp."))
        .at(i))
}

fn prepare_query(filters: &Value, i: usize) -> NodeResult<Vec<(String, String)>> {
    let mut q: Vec<(String, String)> = Vec::new();
    if let Some(ids) = filters.get("labelIds").and_then(Value::as_array) {
        for id in ids {
            if let Some(s) = id.as_str().filter(|s| !s.is_empty()) {
                q.push(("labelIds".to_string(), s.to_string()));
            }
        }
    }
    if filters.get("includeSpamTrash").and_then(Value::as_bool).unwrap_or(false) {
        q.push(("includeSpamTrash".to_string(), "true".to_string()));
    }
    let mut query_str = filters.get("q").and_then(Value::as_str).unwrap_or("").to_string();
    let append = |base: &mut String, part: String| {
        if base.is_empty() {
            *base = part;
        } else {
            base.push(' ');
            base.push_str(&part);
        }
    };
    if let Some(sender) = filters.get("sender").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        append(&mut query_str, format!("from:{sender}"));
    }
    let read_status = filters.get("readStatus").and_then(Value::as_str).unwrap_or("");
    if !read_status.is_empty() && read_status != "both" {
        append(&mut query_str, format!("is:{read_status}"));
    }
    if let Some(after) = filters.get("receivedAfter").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        append(&mut query_str, format!("after:{}", prepare_timestamp(after, i, "after")?));
    }
    if let Some(before) = filters.get("receivedBefore").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        append(&mut query_str, format!("before:{}", prepare_timestamp(before, i, "before")?));
    }
    if !query_str.is_empty() {
        q.push(("q".to_string(), query_str));
    }
    Ok(q)
}

// ---- address / attachment / body helpers ------------------------------------

/// n8n's `prepareEmailsInput`: comma-split, each entry must contain `@`,
/// and bare addresses get wrapped in `<>`.
fn prepare_emails_input(field_name: &str, input: &str, i: usize) -> NodeResult<Vec<String>> {
    let mut out = Vec::new();
    for entry in input.split(',') {
        let email = entry.trim();
        if !email.contains('@') {
            return Err(NodeError::new("Invalid email address").describe(format!("The email address '{email}' in the '{field_name}' field isn't valid")).at(i));
        }
        if email.contains('<') && email.contains('>') {
            out.push(email.to_string());
        } else {
            out.push(format!("<{email}>"));
        }
    }
    Ok(out)
}

/// n8n's `prepareEmailBody`: picks text or html based on `emailType`, and
/// optionally appends n8n's attribution notice.
fn prepare_email_body(ctx: &ExecCtx<'_>, i: usize, append_attribution: bool) -> NodeResult<(String, String)> {
    let email_type = ctx.param_str("emailType", i, "html")?;
    let mut message = ctx.param_str("message", i, "")?.trim().to_string();
    if append_attribution {
        let attribution_text = "This email was sent automatically with ";
        let link = "https://n8n.io/?utm_source=n8n-internal&utm_medium=powered_by&utm_campaign=n8n-nodes-base.gmail";
        if email_type == "html" {
            message = format!("\n\t\t\t\t{message}\n\t\t\t\t<br>\n\t\t\t\t<br>\n\t\t\t\t---\n\t\t\t\t<br>\n\t\t\t\t<em>{attribution_text}<a href=\"{link}\" target=\"_blank\">n8n</a></em>\n\t\t\t\t");
        } else {
            message = format!("{message}\n\n---\n{attribution_text}n8n\nhttps://n8n.io");
        }
    }
    if email_type == "html" {
        Ok((String::new(), message))
    } else {
        Ok((message, String::new()))
    }
}

/// Decodes the base64 `data` of a binary property, returning its bytes,
/// mime type and file name (mirrors `email.rs`'s `binary_attachment`).
fn binary_attachment(item: &Item, prop: &str, i: usize) -> NodeResult<(Vec<u8>, String, String)> {
    let binary = item.binary.as_ref().ok_or_else(|| {
        NodeError::new("Attachment not found").describe(format!("The input field '{prop}' doesn't contain an attachment. Please make sure you specify a field containing binary data")).at(i)
    })?;
    let entry = binary
        .get(prop)
        .ok_or_else(|| NodeError::new("Attachment not found").describe(format!("The input field '{prop}' doesn't contain an attachment. Please make sure you specify a field containing binary data")).at(i))?;
    let data = entry.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Binary field '{prop}' has no data")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| NodeError::new(format!("Binary field '{prop}' is not valid base64: {e}")).at(i))?;
    let mime = entry.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let file_name = entry.get("fileName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("unknown").to_string();
    Ok((bytes, mime, file_name))
}

/// n8n's `prepareEmailAttachments`: `options.attachmentsUi.attachmentsBinary[].property`
/// is a comma-separated list of binary property names.
fn prepare_email_attachments(item: &Item, options: &Value, i: usize) -> NodeResult<Vec<(String, Vec<u8>, String)>> {
    let mut out = Vec::new();
    let entries = options.pointer("/attachmentsUi/attachmentsBinary").and_then(Value::as_array).cloned().unwrap_or_default();
    for entry in entries {
        let prop_field = entry.get("property").and_then(Value::as_str).unwrap_or("");
        for name in prop_field.split(',') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let (bytes, mime, file_name) = binary_attachment(item, name, i)?;
            out.push((file_name, bytes, mime));
        }
    }
    Ok(out)
}

fn upload_query(has_attachments: bool) -> Vec<(String, String)> {
    if has_attachments {
        vec![("userId".to_string(), "me".to_string()), ("uploadType".to_string(), "media".to_string())]
    } else {
        vec![]
    }
}

// ---- MIME building (n8n's `encodeEmail`, via mailcomposer) ------------------

struct EmailFields {
    from: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    reply_to: Vec<String>,
    in_reply_to: String,
    references: String,
    subject: String,
    body: String,
    html_body: String,
    attachments: Vec<(String, Vec<u8>, String)>,
}

enum ContentPart {
    Single(SinglePart),
    Multi(MultiPart),
}

impl ContentPart {
    fn formatted(&self) -> Vec<u8> {
        match self {
            ContentPart::Single(p) => p.formatted(),
            ContentPart::Multi(p) => p.formatted(),
        }
    }
}

/// Builds an RFC 822 message (our own top-level headers, prepended to a
/// lettre-built MIME body) and base64url-encodes it the way n8n's
/// `encodeEmail` does for the Gmail API's `raw` field.
fn encode_email(fields: &EmailFields) -> NodeResult<String> {
    let has_text = !fields.body.is_empty();
    let has_html = !fields.html_body.is_empty();
    let content = if has_html && has_text {
        ContentPart::Multi(MultiPart::alternative_plain_html(fields.body.clone(), fields.html_body.clone()))
    } else if has_html {
        ContentPart::Single(SinglePart::html(fields.html_body.clone()))
    } else {
        ContentPart::Single(SinglePart::plain(fields.body.clone()))
    };

    let body_bytes = if fields.attachments.is_empty() {
        content.formatted()
    } else {
        let builder = MultiPart::mixed();
        let mut mixed: MultiPart = match content {
            ContentPart::Single(p) => builder.singlepart(p),
            ContentPart::Multi(p) => builder.multipart(p),
        };
        for (name, bytes, ctype) in &fields.attachments {
            let content_type = ContentType::parse(ctype).unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
            mixed = mixed.singlepart(Attachment::new(name.clone()).body(bytes.clone(), content_type));
        }
        mixed.formatted()
    };

    let mut headers = String::new();
    headers.push_str("MIME-Version: 1.0\r\n");
    if !fields.from.is_empty() {
        headers.push_str(&format!("From: {}\r\n", fields.from));
    }
    if !fields.to.is_empty() {
        headers.push_str(&format!("To: {}\r\n", fields.to.join(", ")));
    }
    if !fields.cc.is_empty() {
        headers.push_str(&format!("Cc: {}\r\n", fields.cc.join(", ")));
    }
    if !fields.bcc.is_empty() {
        headers.push_str(&format!("Bcc: {}\r\n", fields.bcc.join(", ")));
    }
    if !fields.reply_to.is_empty() {
        headers.push_str(&format!("Reply-To: {}\r\n", fields.reply_to.join(", ")));
    }
    headers.push_str(&format!("Subject: {}\r\n", fields.subject));
    if !fields.in_reply_to.is_empty() {
        headers.push_str(&format!("In-Reply-To: {}\r\n", fields.in_reply_to));
    }
    if !fields.references.is_empty() {
        headers.push_str(&format!("References: {}\r\n", fields.references));
    }
    headers.push_str(&format!("Date: {}\r\n", chrono::Utc::now().to_rfc2822()));

    let mut raw = headers.into_bytes();
    raw.extend_from_slice(&body_bytes);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw))
}

// ---- reply address resolution (n8n's `utils/replyToEmail.js`) --------------

fn parse_address_list(value: &str) -> Vec<(Option<String>, String)> {
    value
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            if let (Some(start), Some(end)) = (part.find('<'), part.find('>')) {
                if end > start {
                    let addr = part[start + 1..end].trim().to_string();
                    let name = part[..start].trim().trim_matches('"').to_string();
                    return Some((if name.is_empty() { None } else { Some(name) }, addr));
                }
            }
            Some((None, part.to_string()))
        })
        .collect()
}

/// If the name contains an RFC 5322 special, wrap it in quotes and escape
/// any embedded `"` or `\`; otherwise it can be used as-is.
fn format_display_name(name: &str) -> String {
    if name.chars().any(|c| "\"(),:;<>@[]\\".contains(c)) {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        name.to_string()
    }
}

fn add_recipients(value: &str, exclude_self: bool, self_email: &str, out: &mut Vec<String>, seen: &mut HashSet<String>) {
    for (name, addr) in parse_address_list(value) {
        if addr.is_empty() {
            continue;
        }
        if exclude_self && addr.eq_ignore_ascii_case(self_email) {
            continue;
        }
        let token = match name {
            Some(n) => format!("{} <{addr}>", format_display_name(&n)),
            None => format!("<{addr}>"),
        };
        if seen.insert(token.clone()) {
            out.push(token);
        }
    }
}

fn header_find<'a>(headers: &'a [Value], name: &str) -> Option<&'a str> {
    headers.iter().find(|h| h.get("name").and_then(Value::as_str).map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)).and_then(|h| h.get("value")).and_then(Value::as_str)
}

/// Shared by `message:reply` and `thread:reply` (n8n's `replyToEmail`).
async fn reply_to_email(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize, node_version: f64) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let reply_sender_only = options.get("replyToSenderOnly").and_then(Value::as_bool).unwrap_or(false);
    let reply_recipients_only = options.get("replyToRecipientsOnly").and_then(Value::as_bool).unwrap_or(false);
    if reply_sender_only && reply_recipients_only {
        return Err(NodeError::new("Both \"Reply to Sender Only\" and \"Reply to Recipient Only\" cannot be enabled at the same time. Please select only one option.").at(i));
    }
    let cc = match options.get("ccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("CC", v, i)?,
        None => vec![],
    };
    let bcc = match options.get("bccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("BCC", v, i)?,
        None => vec![],
    };
    let attachments = prepare_email_attachments(item, &options, i)?;
    let gmail_id = ctx.param_str("messageId", i, "")?;
    let mut query = upload_query(!attachments.is_empty());
    query.push(("format".to_string(), "metadata".to_string()));

    let get_resp = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/messages/{gmail_id}"), None, &query, "message").await?;
    let thread_id = get_resp.get("threadId").cloned().unwrap_or(Value::Null);
    let headers = get_resp.pointer("/payload/headers").and_then(Value::as_array).cloned().unwrap_or_default();
    let subject = header_find(&headers, "subject").unwrap_or("").to_string();
    let message_id_global = header_find(&headers, "message-id").unwrap_or("").to_string();

    let profile = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/profile", None, &[], "message").await?;
    let email_address = profile.get("emailAddress").and_then(Value::as_str).unwrap_or("").to_string();

    let mut reply_to_header_name = "from";
    if node_version >= 2.2 && headers.iter().any(|h| h.get("name").and_then(Value::as_str).map(|n| n.eq_ignore_ascii_case("reply-to")).unwrap_or(false)) {
        reply_to_header_name = "reply-to";
    }
    let mut to: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for h in &headers {
        let hname = h.get("name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let hvalue = h.get("value").and_then(Value::as_str).unwrap_or("");
        if hname == reply_to_header_name && !reply_recipients_only {
            add_recipients(hvalue, false, &email_address, &mut to, &mut seen);
        }
        if hname == "to" && !reply_sender_only {
            add_recipients(hvalue, true, &email_address, &mut to, &mut seen);
        }
    }

    let mut from = String::new();
    if let Some(sender_name) = options.get("senderName").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        from = format!("{sender_name} <{email_address}>");
    }
    let (body, html_body) = prepare_email_body(ctx, i, false)?;
    let fields = EmailFields {
        from,
        to,
        cc,
        bcc,
        reply_to: vec![],
        in_reply_to: message_id_global.clone(),
        references: message_id_global,
        subject,
        body,
        html_body,
        attachments,
    };
    let raw = encode_email(&fields)?;
    let body_json = json!({"raw": raw, "threadId": thread_id});
    let resp = gmail_request(ctx, auth, "POST", "/gmail/v1/users/me/messages/send", Some(body_json), &query, "message").await?;
    Ok(vec![Item::from_value(resp)])
}

// ---- raw MIME parsing (n8n's `parseRawEmail`, via mailparser) --------------

fn decode_base64url(s: &str) -> NodeResult<Vec<u8>> {
    let trimmed: String = s.trim().chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&trimmed)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&trimmed))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(&trimmed))
        .map_err(|e| NodeError::new(format!("Could not decode the Gmail message: {e}")))
}

/// n8n's `parseRawEmail`: decodes the base64url `raw` field, parses the
/// MIME tree, and shapes it like `mailparser`'s `simpleParser` output
/// (simplified; see the module doc comment). The low-level MIME parsing is
/// shared with the Email Trigger (IMAP) node via `super::mime`.
fn parse_raw_email(message_data: &Value, download_attachments: bool, attachment_prefix: &str) -> NodeResult<Item> {
    let raw = message_data.get("raw").and_then(Value::as_str).unwrap_or("");
    let decoded = decode_base64url(raw)?;
    let text = String::from_utf8_lossy(&decoded).into_owned();
    let (headers, parts) = super::mime::parse_message(&text);
    let (text_body, html_body, attachments_parsed) = super::mime::split_text_and_attachments(parts);

    let mut headers_map = Map::new();
    for (name, value) in &headers {
        headers_map.insert(name.to_ascii_lowercase(), json!(format!("{name}: {value}")));
    }
    let lookup = |name: &str| super::mime::header_value(&headers, name).map(str::to_string);

    let mut json_out = Map::new();
    json_out.insert("id".into(), message_data.get("id").cloned().unwrap_or(Value::Null));
    json_out.insert("threadId".into(), message_data.get("threadId").cloned().unwrap_or(Value::Null));
    json_out.insert("labelIds".into(), message_data.get("labelIds").cloned().unwrap_or(json!([])));
    json_out.insert("sizeEstimate".into(), message_data.get("sizeEstimate").cloned().unwrap_or(Value::Null));
    json_out.insert("headers".into(), Value::Object(headers_map));
    if let Some(s) = lookup("Subject") {
        json_out.insert("subject".into(), json!(s));
    }
    if let Some(s) = lookup("From") {
        json_out.insert("from".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("To") {
        json_out.insert("to".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("Cc") {
        json_out.insert("cc".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("Message-ID") {
        json_out.insert("messageId".into(), json!(s));
    }
    if let Some(s) = lookup("In-Reply-To") {
        json_out.insert("inReplyTo".into(), json!(s));
    }
    if let Some(s) = lookup("References") {
        json_out.insert("references".into(), json!(s));
    }
    json_out.insert("text".into(), json!(text_body));
    match html_body {
        Some(h) => {
            json_out.insert("html".into(), json!(h));
        }
        None => {
            json_out.insert("html".into(), json!(false));
        }
    }

    let mut binary_map = Map::new();
    if download_attachments {
        for (idx, att) in attachments_parsed.iter().enumerate() {
            let file_name = att.filename.clone().unwrap_or_else(|| format!("attachment_{idx}"));
            let entry = json!({
                "data": base64::engine::general_purpose::STANDARD.encode(&att.body),
                "mimeType": att.content_type,
                "fileName": file_name,
            });
            binary_map.insert(format!("{attachment_prefix}{idx}"), entry);
        }
    }
    Ok(Item { json: json_out, binary: if binary_map.is_empty() { None } else { Some(binary_map) }, paired_item: None })
}

// ---- label simplification (n8n's `simplifyOutput`) -------------------------

async fn simplify_output(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, mut data: Vec<Value>) -> NodeResult<Vec<Value>> {
    let labels_resp = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/labels", None, &[], "label").await?;
    let labels: Vec<(String, String)> = labels_resp
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| Some((l.get("id")?.as_str()?.to_string(), l.get("name")?.as_str()?.to_string())))
        .collect();
    for item in data.iter_mut() {
        if let Some(obj) = item.as_object_mut() {
            if let Some(ids) = obj.get("labelIds").and_then(Value::as_array).cloned() {
                let matched: Vec<Value> = labels.iter().filter(|(id, _)| ids.iter().any(|v| v.as_str() == Some(id.as_str()))).map(|(id, name)| json!({"id": id, "name": name})).collect();
                obj.insert("labels".into(), json!(matched));
                obj.remove("labelIds");
            }
            if let Some(headers) = obj.get("payload").and_then(|p| p.get("headers")).and_then(Value::as_array).cloned() {
                for h in &headers {
                    if let (Some(name), Some(value)) = (h.get("name").and_then(Value::as_str), h.get("value").and_then(Value::as_str)) {
                        obj.insert(name.to_string(), json!(value));
                    }
                }
                if let Some(payload) = obj.get_mut("payload").and_then(Value::as_object_mut) {
                    payload.remove("headers");
                }
            }
        }
    }
    Ok(data)
}

/// n8n's `unescapeSnippets` (really a decode: Gmail's `snippet` field
/// comes back HTML-entity-escaped).
fn unescape_snippet(text: &str) -> String {
    text.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&quot;", "\"")
}

// ---- message operations ------------------------------------------------------

async fn op_message_send(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize, node_version: f64) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let send_to = ctx.param_str("sendTo", i, "")?;
    let to = prepare_emails_input("To", &send_to, i)?;
    let cc = match options.get("ccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("CC", v, i)?,
        None => vec![],
    };
    let bcc = match options.get("bccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("BCC", v, i)?,
        None => vec![],
    };
    let reply_to = match options.get("replyTo").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("ReplyTo", v, i)?,
        None => vec![],
    };
    let attachments = prepare_email_attachments(item, &options, i)?;

    let mut from = String::new();
    if let Some(sender_name) = options.get("senderName").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let profile = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/profile", None, &[], "message").await?;
        let email_address = profile.get("emailAddress").and_then(Value::as_str).unwrap_or("").to_string();
        from = format!("{sender_name} <{email_address}>");
    }
    let append_attribution = match options.get("appendAttribution") {
        Some(Value::Bool(b)) => *b,
        _ => node_version >= 2.1,
    };
    let subject = ctx.param_str("subject", i, "")?;
    let (body, html_body) = prepare_email_body(ctx, i, append_attribution)?;
    let query = upload_query(!attachments.is_empty());
    let fields = EmailFields { from, to, cc, bcc, reply_to, in_reply_to: String::new(), references: String::new(), subject, body, html_body, attachments };
    let raw = encode_email(&fields)?;
    let resp = gmail_request(ctx, auth, "POST", "/gmail/v1/users/me/messages/send", Some(json!({"raw": raw})), &query, "message").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_message_get(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    let options = ctx.param("options", i)?;
    let simple = ctx.param_bool("simple", i, true)?;
    let query = if simple { metadata_query() } else { vec![("format".to_string(), "raw".to_string())] };
    let resp = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/messages/{id}"), None, &query, "message").await?;
    if simple {
        let simplified = simplify_output(ctx, auth, vec![resp]).await?;
        Ok(simplified.into_iter().map(Item::from_value).collect())
    } else {
        let download = options.get("downloadAttachments").and_then(Value::as_bool).unwrap_or(false);
        let prefix = options.get("dataPropertyAttachmentsPrefixName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("attachment_").to_string();
        Ok(vec![parse_raw_email(&resp, download, &prefix)?])
    }
}

async fn op_message_get_all(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", i, false)?;
    let options = ctx.param("options", i)?;
    let filters = ctx.param("filters", i)?;
    let query = prepare_query(&filters, i)?;
    let list: Vec<Value> = if return_all {
        request_all_items(ctx, auth, "messages", "/gmail/v1/users/me/messages", "message", &query).await?
    } else {
        let limit = ctx.param_f64("limit", i, 50.0)? as i64;
        let mut q = query.clone();
        q.push(("maxResults".to_string(), limit.to_string()));
        let resp = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/messages", None, &q, "message").await?;
        resp.get("messages").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    let simple = ctx.param_bool("simple", i, true)?;
    let fetch_query = if simple { metadata_query() } else { vec![("format".to_string(), "raw".to_string())] };
    let download = options.get("downloadAttachments").and_then(Value::as_bool).unwrap_or(false);
    let prefix = options.get("dataPropertyAttachmentsPrefixName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("attachment_").to_string();

    let mut full: Vec<Value> = Vec::new();
    let mut parsed_items: Vec<Item> = Vec::new();
    for msg in &list {
        let id = msg.get("id").and_then(Value::as_str).unwrap_or("");
        let detail = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/messages/{id}"), None, &fetch_query, "message").await?;
        if simple {
            full.push(detail);
        } else {
            parsed_items.push(parse_raw_email(&detail, download, &prefix)?);
        }
    }
    if simple {
        let simplified = simplify_output(ctx, auth, full).await?;
        Ok(simplified.into_iter().map(Item::from_value).collect())
    } else {
        Ok(parsed_items)
    }
}

async fn op_message_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    gmail_request(ctx, auth, "DELETE", &format!("/gmail/v1/users/me/messages/{id}"), None, &[], "message").await?;
    Ok(vec![Item::from_value(json!({"success": true}))])
}

async fn op_message_mark(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize, as_read: bool) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    let body = if as_read { json!({"removeLabelIds": ["UNREAD"]}) } else { json!({"addLabelIds": ["UNREAD"]}) };
    let resp = gmail_request(ctx, auth, "POST", &format!("/gmail/v1/users/me/messages/{id}/modify"), Some(body), &[], "message").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_message_labels(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize, add: bool) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    let label_ids = ctx.param("labelIds", i)?;
    let key = if add { "addLabelIds" } else { "removeLabelIds" };
    let resp = gmail_request(ctx, auth, "POST", &format!("/gmail/v1/users/me/messages/{id}/modify"), Some(json!({key: label_ids})), &[], "message").await?;
    Ok(vec![Item::from_value(resp)])
}

// ---- draft operations ---------------------------------------------------------

async fn op_draft_create(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, item: &Item, i: usize) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let to = match options.get("sendTo").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("To", v, i)?,
        None => vec![],
    };
    let cc = match options.get("ccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("CC", v, i)?,
        None => vec![],
    };
    let bcc = match options.get("bccList").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("BCC", v, i)?,
        None => vec![],
    };
    let reply_to = match options.get("replyTo").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(v) => prepare_emails_input("ReplyTo", v, i)?,
        None => vec![],
    };
    let from_alias = options.get("fromAlias").and_then(Value::as_str).unwrap_or("").to_string();
    let thread_id = options.get("threadId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let attachments = prepare_email_attachments(item, &options, i)?;
    let subject = ctx.param_str("subject", i, "")?;
    let (body, html_body) = prepare_email_body(ctx, i, false)?;
    let query = upload_query(!attachments.is_empty());
    let mut fields = EmailFields { from: from_alias, to, cc, bcc, reply_to, in_reply_to: String::new(), references: String::new(), subject, body, html_body, attachments };

    if let Some(tid) = &thread_id {
        // n8n's `addThreadHeadersToEmail`: use the last message's
        // Message-ID so Gmail associates the draft with the thread.
        let thread = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/threads/{tid}"), None, &[("format".to_string(), "metadata".to_string()), ("metadataHeaders".to_string(), "Message-ID".to_string())], "thread").await?;
        if let Some(last) = thread.get("messages").and_then(Value::as_array).and_then(|m| m.last()) {
            if let Some(headers) = last.pointer("/payload/headers").and_then(Value::as_array) {
                let mid = headers.iter().find(|h| {
                    let n = h.get("name").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
                    n.contains("message") && n.contains("id")
                });
                if let Some(mid) = mid.and_then(|h| h.get("value")).and_then(Value::as_str) {
                    fields.in_reply_to = mid.to_string();
                    fields.references = mid.to_string();
                }
            }
        }
    }

    let raw = encode_email(&fields)?;
    let mut message_body = Map::new();
    message_body.insert("raw".into(), json!(raw));
    if let Some(tid) = &thread_id {
        message_body.insert("threadId".into(), json!(tid));
    }
    let resp = gmail_request(ctx, auth, "POST", "/gmail/v1/users/me/drafts", Some(json!({"message": Value::Object(message_body)})), &query, "draft").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_draft_get(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    let options = ctx.param("options", i)?;
    let resp = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/drafts/{id}"), None, &[("format".to_string(), "raw".to_string())], "draft").await?;
    let download = options.get("downloadAttachments").and_then(Value::as_bool).unwrap_or(false);
    let prefix = options.get("dataPropertyAttachmentsPrefixName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("attachment_").to_string();
    let message = resp.get("message").cloned().unwrap_or(json!({}));
    let mut parsed = parse_raw_email(&message, download, &prefix)?;
    let inner_id = parsed.json.get("id").cloned().unwrap_or(Value::Null);
    parsed.json.insert("messageId".into(), inner_id);
    parsed.json.insert("id".into(), resp.get("id").cloned().unwrap_or(Value::Null));
    Ok(vec![parsed])
}

async fn op_draft_get_all(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", i, false)?;
    let options = ctx.param("options", i)?;
    let mut base_query: Vec<(String, String)> = Vec::new();
    if options.get("includeSpamTrash").and_then(Value::as_bool).unwrap_or(false) {
        base_query.push(("includeSpamTrash".to_string(), "true".to_string()));
    }
    let list: Vec<Value> = if return_all {
        request_all_items(ctx, auth, "drafts", "/gmail/v1/users/me/drafts", "draft", &base_query).await?
    } else {
        let limit = ctx.param_f64("limit", i, 50.0)? as i64;
        let mut q = base_query.clone();
        q.push(("maxResults".to_string(), limit.to_string()));
        let resp = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/drafts", None, &q, "draft").await?;
        resp.get("drafts").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    let download = options.get("downloadAttachments").and_then(Value::as_bool).unwrap_or(false);
    let prefix = options.get("dataPropertyAttachmentsPrefixName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("attachment_").to_string();

    let mut out = Vec::new();
    for d in &list {
        let id = d.get("id").and_then(Value::as_str).unwrap_or("").to_string();
        let detail = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/drafts/{id}"), None, &[("format".to_string(), "raw".to_string())], "draft").await?;
        let message = detail.get("message").cloned().unwrap_or(json!({}));
        let mut parsed = parse_raw_email(&message, download, &prefix)?;
        let inner_id = parsed.json.get("id").cloned().unwrap_or(Value::Null);
        parsed.json.insert("messageId".into(), inner_id);
        parsed.json.insert("id".into(), json!(id));
        out.push(parsed);
    }
    Ok(out)
}

async fn op_draft_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("messageId", i, "")?;
    gmail_request(ctx, auth, "DELETE", &format!("/gmail/v1/users/me/drafts/{id}"), None, &[], "draft").await?;
    Ok(vec![Item::from_value(json!({"success": true}))])
}

// ---- label operations -----------------------------------------------------

async fn op_label_create(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let name = ctx.param_str("name", i, "")?;
    let list_vis = ctx.param_str("options.labelListVisibility", i, "labelShow")?;
    let msg_vis = ctx.param_str("options.messageListVisibility", i, "show")?;
    let body = json!({"labelListVisibility": list_vis, "messageListVisibility": msg_vis, "name": name});
    let resp = gmail_request(ctx, auth, "POST", "/gmail/v1/users/me/labels", Some(body), &[], "label").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_label_get(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("labelId", i, "")?;
    let resp = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/labels/{id}"), None, &[], "label").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_label_get_all(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", i, false)?;
    let resp = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/labels", None, &[], "label").await?;
    let mut list: Vec<Value> = resp.get("labels").and_then(Value::as_array).cloned().unwrap_or_default();
    if !return_all {
        let limit = ctx.param_f64("limit", i, 50.0)? as usize;
        list.truncate(limit);
    }
    Ok(list.into_iter().map(Item::from_value).collect())
}

async fn op_label_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("labelId", i, "")?;
    gmail_request(ctx, auth, "DELETE", &format!("/gmail/v1/users/me/labels/{id}"), None, &[], "label").await?;
    Ok(vec![Item::from_value(json!({"success": true}))])
}

// ---- thread operations -----------------------------------------------------

async fn op_thread_get(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("threadId", i, "")?;
    let options = ctx.param("options", i)?;
    let only_messages = options.get("returnOnlyMessages").and_then(Value::as_bool).unwrap_or(false);
    let simple = ctx.param_bool("simple", i, true)?;
    let query = if simple { metadata_query() } else { vec![("format".to_string(), "full".to_string())] };
    let mut resp = gmail_request(ctx, auth, "GET", &format!("/gmail/v1/users/me/threads/{id}"), None, &query, "thread").await?;
    let messages = resp.get("messages").and_then(Value::as_array).cloned().unwrap_or_default();
    let simplified = simplify_output(ctx, auth, messages).await?;
    if only_messages {
        Ok(simplified.into_iter().map(Item::from_value).collect())
    } else {
        if let Some(obj) = resp.as_object_mut() {
            obj.insert("messages".into(), json!(simplified));
        }
        Ok(vec![Item::from_value(resp)])
    }
}

async fn op_thread_get_all(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", i, false)?;
    let filters = ctx.param("filters", i)?;
    let query = prepare_query(&filters, i)?;
    let list: Vec<Value> = if return_all {
        request_all_items(ctx, auth, "threads", "/gmail/v1/users/me/threads", "thread", &query).await?
    } else {
        let limit = ctx.param_f64("limit", i, 50.0)? as i64;
        let mut q = query.clone();
        q.push(("maxResults".to_string(), limit.to_string()));
        let resp = gmail_request(ctx, auth, "GET", "/gmail/v1/users/me/threads", None, &q, "thread").await?;
        resp.get("threads").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    Ok(list.into_iter().map(Item::from_value).collect())
}

async fn op_thread_delete(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("threadId", i, "")?;
    gmail_request(ctx, auth, "DELETE", &format!("/gmail/v1/users/me/threads/{id}"), None, &[], "thread").await?;
    Ok(vec![Item::from_value(json!({"success": true}))])
}

async fn op_thread_trash(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize, trash: bool) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("threadId", i, "")?;
    let action = if trash { "trash" } else { "untrash" };
    let resp = gmail_request(ctx, auth, "POST", &format!("/gmail/v1/users/me/threads/{id}/{action}"), None, &[], "thread").await?;
    Ok(vec![Item::from_value(resp)])
}

async fn op_thread_labels(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, i: usize, add: bool) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("threadId", i, "")?;
    let label_ids = ctx.param("labelIds", i)?;
    let key = if add { "addLabelIds" } else { "removeLabelIds" };
    let resp = gmail_request(ctx, auth, "POST", &format!("/gmail/v1/users/me/threads/{id}/modify"), Some(json!({key: label_ids})), &[], "thread").await?;
    Ok(vec![Item::from_value(resp)])
}

// ---- top-level dispatch -------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn run_one(ctx: &ExecCtx<'_>, auth: &mut GoogleAuth, resource: &str, operation: &str, item: &Item, i: usize, node_version: f64) -> NodeResult<Vec<Item>> {
    match (resource, operation) {
        ("message", "send") => op_message_send(ctx, auth, item, i, node_version).await,
        ("message", "reply") => reply_to_email(ctx, auth, item, i, node_version).await,
        ("message", "get") => op_message_get(ctx, auth, i).await,
        ("message", "getAll") => op_message_get_all(ctx, auth, i).await,
        ("message", "delete") => op_message_delete(ctx, auth, i).await,
        ("message", "markAsRead") => op_message_mark(ctx, auth, i, true).await,
        ("message", "markAsUnread") => op_message_mark(ctx, auth, i, false).await,
        ("message", "addLabels") => op_message_labels(ctx, auth, i, true).await,
        ("message", "removeLabels") => op_message_labels(ctx, auth, i, false).await,
        ("draft", "create") => op_draft_create(ctx, auth, item, i).await,
        ("draft", "get") => op_draft_get(ctx, auth, i).await,
        ("draft", "getAll") => op_draft_get_all(ctx, auth, i).await,
        ("draft", "delete") => op_draft_delete(ctx, auth, i).await,
        ("label", "create") => op_label_create(ctx, auth, i).await,
        ("label", "get") => op_label_get(ctx, auth, i).await,
        ("label", "getAll") => op_label_get_all(ctx, auth, i).await,
        ("label", "delete") => op_label_delete(ctx, auth, i).await,
        ("thread", "get") => op_thread_get(ctx, auth, i).await,
        ("thread", "getAll") => op_thread_get_all(ctx, auth, i).await,
        ("thread", "delete") => op_thread_delete(ctx, auth, i).await,
        ("thread", "trash") => op_thread_trash(ctx, auth, i, true).await,
        ("thread", "untrash") => op_thread_trash(ctx, auth, i, false).await,
        ("thread", "addLabels") => op_thread_labels(ctx, auth, i, true).await,
        ("thread", "removeLabels") => op_thread_labels(ctx, auth, i, false).await,
        ("thread", "reply") => reply_to_email(ctx, auth, item, i, node_version).await,
        ("message" | "draft" | "label" | "thread", other) => Err(unsupported(resource, other).at(i)),
        (other, _) => Err(NodeError::new(format!("Gmail resource \"{other}\" is not supported natively yet")).at(i)),
    }
}

#[async_trait::async_trait]
impl NodeType for Gmail {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.gmail"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items = ctx.input().to_vec();
        let resource = ctx.param_str("resource", 0, "message")?;
        let operation = ctx.param_str("operation", 0, "send")?;
        let node_version = ctx.node.type_version;
        let mut auth = resolve_auth(ctx).await?;
        let mut out: Vec<Item> = Vec::new();
        for (i, item) in items.iter().enumerate() {
            match run_one(ctx, &mut auth, &resource, &operation, item, i, node_version).await {
                Ok(produced) => out.extend(produced.into_iter().map(|it| it.paired(i))),
                Err(e) if ctx.continue_on_fail() => {
                    let mut json_err = Map::new();
                    json_err.insert("error".into(), json!(format!("{} (item {i})", e.message)));
                    ctx.error_items.push(Item::new(json_err).paired(i));
                }
                Err(e) => return Err(e),
            }
        }
        if matches!(resource.as_str(), "draft" | "message" | "thread") && matches!(operation.as_str(), "get" | "getAll") {
            for it in out.iter_mut() {
                if let Some(Value::String(s)) = it.json.get("snippet").cloned() {
                    it.json.insert("snippet".into(), json!(unescape_snippet(&s)));
                }
            }
        }
        Ok(vec![out])
    }
}
