//! Email Trigger (IMAP) (plan task 1.15), faithful to n8n's
//! `EmailReadImap` v2 node (`n8n-nodes-base.emailReadImap`, typeVersion
//! 2/2.1/2.2 -- the 2.35.7 editor creates 2.2): connects over IMAP, selects
//! the configured mailbox, and watches it for messages matching the search
//! criteria (`UNSEEN` by default), starting one execution per batch of new
//! mail found, with that mail as the trigger's output items (n8n's
//! `this.emit([items])`).
//!
//! This is r8r's first long-lived trigger: it plugs into the general
//! activation path in `server/triggers.rs` (leader-only; started on
//! activation/takeover, stopped on deactivation/stepdown), which the
//! upcoming RabbitMQ/Kafka/MQTT triggers (plan task 1.13) will reuse by
//! implementing [`crate::n8n::server::triggers::LongLivedTrigger`] and
//! adding one line to `triggers::for_node_type`.
//!
//! Deviations from n8n's `@n8n/imap` (a `node-imap` fork) + `mailparser`:
//! - IDLE is not implemented; this polls every [`POLL_INTERVAL`] instead
//!   of holding an IMAP IDLE command open. n8n's `onMail` callback fires
//!   immediately on a server push; here a new mail is seen at most
//!   `POLL_INTERVAL` late.
//! - UID tracking (`options.trackLastMessageId`) lives in the listener
//!   task's own memory, not in the workflow's persisted `staticData` the
//!   way n8n's `getWorkflowStaticData('node')` does, so it resets across a
//!   restart (r8r has no hook to flush a trigger's own bookkeeping back to
//!   the workflow row outside of an execution).
//! - "Simple" format's `textPlain`/`textHtml`/`metadata` are built from the
//!   hand-rolled MIME parser shared with the Gmail node (`super::mime`)
//!   rather than node-imap's own `BODYSTRUCTURE`-driven part walker, and
//!   "resolved"/"raw" are reshaped from the same parser rather than
//!   `mailparser`'s exact output shape.
//! - `allowUnauthorizedCerts` is accepted by the `imap` credential but not
//!   honoured for `secure: true` connections yet: TLS always verifies
//!   against the Mozilla root store via `webpki-roots`.

use crate::n8n::server::triggers::{fire, resolve_credential, LongLivedTrigger};
use crate::n8n::server::N8n;
use crate::n8n::types::Item;
use crate::n8n::workflow::Node;
use async_imap::types::Fetch;
use async_imap::Session;
use base64::Engine;
use futures_util::TryStreamExt;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

/// How often an active listener checks the mailbox again (see the module
/// doc comment: n8n holds an IMAP IDLE command open instead).
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
/// How long to wait before retrying after a connection/login/select
/// failure.
const RECONNECT_BACKOFF: std::time::Duration = std::time::Duration::from_secs(5);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub struct ImapTrigger;

/// A plain or TLS-wrapped TCP connection, so the rest of the listener is
/// written once against `Session<Stream>` regardless of the credential's
/// `secure` setting.
enum Stream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Stream::Plain(_) => "Stream::Plain",
            Stream::Tls(_) => "Stream::Tls",
        })
    }
}

impl AsyncRead for Stream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Stream::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Stream::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match &mut *self {
            Stream::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Stream::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Stream::Plain(s) => Pin::new(s).poll_flush(cx),
            Stream::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Stream::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Stream::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

struct ImapCreds {
    user: String,
    password: String,
    host: String,
    port: u16,
    secure: bool,
}

fn creds_from(data: &Value) -> ImapCreds {
    ImapCreds {
        user: data["user"].as_str().unwrap_or("").to_string(),
        password: data["password"].as_str().unwrap_or("").to_string(),
        host: data["host"].as_str().unwrap_or("").trim().to_string(),
        port: data["port"].as_u64().unwrap_or(993) as u16,
        secure: data["secure"].as_bool().unwrap_or(true),
    }
}

async fn connect(creds: &ImapCreds) -> Result<Session<Stream>, String> {
    if creds.host.is_empty() {
        return Err("The IMAP credential has no host set".to_string());
    }
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((creds.host.as_str(), creds.port)))
        .await
        .map_err(|_| format!("Connection to {}:{} timed out", creds.host, creds.port))?
        .map_err(|e| format!("Could not connect to {}:{}: {e}", creds.host, creds.port))?;
    let stream = if creds.secure {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let domain = rustls::pki_types::ServerName::try_from(creds.host.clone()).map_err(|e| format!("Invalid IMAP host name \"{}\": {e}", creds.host))?;
        let tls = connector.connect(domain, tcp).await.map_err(|e| format!("TLS handshake with {} failed: {e}", creds.host))?;
        Stream::Tls(Box::new(tls))
    } else {
        Stream::Plain(tcp)
    };
    let mut client = async_imap::Client::new(stream);
    client
        .read_response()
        .await
        .map_err(|e| format!("Could not read the IMAP server greeting from {}:{}: {e}", creds.host, creds.port))?;
    client.login(&creds.user, &creds.password).await.map_err(|(e, _)| format!("IMAP login to {}:{} failed: {e}", creds.host, creds.port))
}

fn str_param(params: &Value, path: &str, default: &str) -> String {
    params.pointer(path).and_then(Value::as_str).unwrap_or(default).to_string()
}

fn bool_param(params: &Value, path: &str, default: bool) -> bool {
    params.pointer(path).and_then(Value::as_bool).unwrap_or(default)
}

/// Converts n8n's node-imap-style search criteria (a JSON array of strings
/// and `[key, value]` pairs, e.g. `["UNSEEN", ["SINCE", "1-Jan-2024"]]`)
/// into a raw IMAP search query string.
fn build_criteria(custom_config: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(custom_config).map_err(|_| "Custom email config is not valid JSON.".to_string())?;
    let arr = parsed.as_array().ok_or_else(|| "Custom email config is not valid JSON.".to_string())?;
    if arr.is_empty() {
        return Err("Custom email config is not valid JSON.".to_string());
    }
    Ok(criteria_to_string(arr))
}

fn criteria_to_string(arr: &[Value]) -> String {
    arr.iter().map(criterion_to_string).collect::<Vec<_>>().join(" ")
}

fn criterion_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) if a.len() == 2 && a[0].is_string() => {
            let key = a[0].as_str().unwrap();
            let val = match &a[1] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            if val.chars().any(char::is_whitespace) {
                format!("{key} \"{val}\"")
            } else {
                format!("{key} {val}")
            }
        }
        Value::Array(a) => format!("({})", criteria_to_string(a)),
        other => other.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn poll_once(
    session: &mut Session<Stream>,
    base_criteria: &str,
    last_uid: &mut Option<u32>,
    track_last_message_id: bool,
    activated_at: chrono::DateTime<chrono::Utc>,
    post_process_action: &str,
    format: &str,
    download_attachments: bool,
    attachment_prefix: &str,
) -> Result<Option<Vec<Item>>, String> {
    let mut criteria = base_criteria.to_string();
    if let Some(uid) = *last_uid {
        criteria = format!("{criteria} UID {uid}:*");
    } else if track_last_message_id {
        criteria = format!("{criteria} SINCE {}", activated_at.format("%d-%b-%Y"));
    }
    let found: HashSet<u32> = session.uid_search(&criteria).await.map_err(|e| format!("IMAP search failed: {e}"))?;
    let mut uids: Vec<u32> = found.into_iter().filter(|u| last_uid.is_none_or(|l| *u > l)).collect();
    uids.sort_unstable();
    if uids.is_empty() {
        return Ok(None);
    }
    let seq = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let fetches: Vec<Fetch> = session
        .uid_fetch(&seq, "(UID RFC822 FLAGS)")
        .await
        .map_err(|e| format!("IMAP fetch failed: {e}"))?
        .try_collect()
        .await
        .map_err(|e| format!("IMAP fetch failed: {e}"))?;

    let mut by_uid: Vec<(u32, Vec<u8>)> = fetches.iter().filter_map(|f| Some((f.uid?, f.body()?.to_vec()))).collect();
    by_uid.sort_by_key(|(uid, _)| *uid);

    let mut max_uid = last_uid.unwrap_or(0);
    let mut items = Vec::new();
    for (uid, body) in &by_uid {
        max_uid = max_uid.max(*uid);
        items.push(build_item(*uid, body, format, download_attachments, attachment_prefix));
    }
    *last_uid = Some(max_uid);

    if post_process_action == "read" {
        session
            .uid_store(&seq, "+FLAGS (\\Seen)")
            .await
            .map_err(|e| format!("Could not mark mail as read: {e}"))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| format!("Could not mark mail as read: {e}"))?;
    }

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(items))
    }
}

fn binary_from_attachments(attachments: &[super::mime::ParsedPart], prefix: &str) -> Option<Map<String, Value>> {
    if attachments.is_empty() {
        return None;
    }
    let mut binary = Map::new();
    for (idx, att) in attachments.iter().enumerate() {
        let file_name = att.filename.clone().unwrap_or_else(|| format!("attachment_{idx}"));
        binary.insert(
            format!("{prefix}{idx}"),
            json!({
                "data": base64::engine::general_purpose::STANDARD.encode(&att.body),
                "mimeType": att.content_type,
                "fileName": file_name,
            }),
        );
    }
    Some(binary)
}

/// n8n's "Simple" format (the default): `textHtml`/`textPlain`, `cc`/
/// `date`/`from`/`subject`/`to` at the top level, everything else under
/// `metadata`.
fn build_simple(uid: u32, raw: &str, download_attachments: bool, attachment_prefix: &str) -> Item {
    let (headers, parts) = super::mime::parse_message(raw);
    let (text_plain, html_body, attachments) = super::mime::split_text_and_attachments(parts);

    let mut json = Map::new();
    json.insert("textHtml".into(), json!(html_body.unwrap_or_default()));
    json.insert("textPlain".into(), json!(text_plain));
    const TOP_LEVEL: &[&str] = &["cc", "date", "from", "subject", "to"];
    let mut metadata = Map::new();
    let mut seen = std::collections::HashSet::new();
    for (name, value) in &headers {
        let lower = name.to_ascii_lowercase();
        if !seen.insert(lower.clone()) {
            continue;
        }
        if TOP_LEVEL.contains(&lower.as_str()) {
            json.insert(lower, json!(value));
        } else {
            metadata.insert(lower, json!(value));
        }
    }
    json.insert("metadata".into(), Value::Object(metadata));
    json.insert("attributes".into(), json!({"uid": uid}));

    let binary = if download_attachments { binary_from_attachments(&attachments, attachment_prefix) } else { None };
    Item { json, binary, paired_item: None }
}

/// n8n's "Resolved" format: the full email with attachments always
/// downloaded as binary data.
fn build_resolved(uid: u32, raw: &str, attachment_prefix: &str) -> Item {
    let (headers, parts) = super::mime::parse_message(raw);
    let (text_body, html_body, attachments) = super::mime::split_text_and_attachments(parts);

    let mut headers_map = Map::new();
    for (name, value) in &headers {
        headers_map.insert(name.to_ascii_lowercase(), json!(format!("{name}: {value}")));
    }
    let lookup = |name: &str| super::mime::header_value(&headers, name).map(str::to_string);

    let mut json = Map::new();
    json.insert("headers".into(), Value::Object(headers_map));
    if let Some(s) = lookup("Subject") {
        json.insert("subject".into(), json!(s));
    }
    if let Some(s) = lookup("From") {
        json.insert("from".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("To") {
        json.insert("to".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("Cc") {
        json.insert("cc".into(), json!({"text": s}));
    }
    if let Some(s) = lookup("Message-ID") {
        json.insert("messageId".into(), json!(s));
    }
    if let Some(s) = lookup("In-Reply-To") {
        json.insert("inReplyTo".into(), json!(s));
    }
    if let Some(s) = lookup("References") {
        json.insert("references".into(), json!(s));
    }
    json.insert("text".into(), json!(text_body));
    json.insert("html".into(), html_body.map(|h| json!(h)).unwrap_or(json!(false)));
    json.insert("attributes".into(), json!({"uid": uid}));

    Item { json, binary: binary_from_attachments(&attachments, attachment_prefix), paired_item: None }
}

/// n8n's "RAW" format. n8n's node returns the literal (possibly
/// non-base64) `TEXT` MIME section via node-imap; we instead base64-encode
/// the whole RFC822 message, which is deterministic regardless of the
/// message's own transfer encoding (documented deviation, module doc
/// comment).
fn build_raw(body: &[u8]) -> Item {
    let mut json = Map::new();
    json.insert("raw".into(), json!(base64::engine::general_purpose::STANDARD.encode(body)));
    Item::new(json)
}

fn build_item(uid: u32, body: &[u8], format: &str, download_attachments: bool, attachment_prefix: &str) -> Item {
    match format {
        "raw" => build_raw(body),
        "resolved" => build_resolved(uid, &String::from_utf8_lossy(body), attachment_prefix),
        _ => build_simple(uid, &String::from_utf8_lossy(body), download_attachments, attachment_prefix),
    }
}

#[async_trait::async_trait]
impl LongLivedTrigger for ImapTrigger {
    async fn validate(&self, n8n: &Arc<N8n>, node: &Node) -> Result<(), String> {
        let creds = creds_from(&resolve_credential(n8n, node, "imap").await?);
        let mut session = connect(&creds).await?;
        let mailbox = str_param(&node.parameters, "/mailbox", "INBOX");
        let selected = session.select(&mailbox).await.map_err(|e| format!("Could not open mailbox \"{mailbox}\": {e}"));
        let _ = session.logout().await;
        selected.map(|_| ())
    }

    async fn run(&self, n8n: Arc<N8n>, workflow_id: String, node: Node) {
        let activated_at = chrono::Utc::now();
        let mailbox = str_param(&node.parameters, "/mailbox", "INBOX");
        let post_process_action = str_param(&node.parameters, "/postProcessAction", "read");
        let format = str_param(&node.parameters, "/format", "simple");
        let download_attachments = bool_param(&node.parameters, "/downloadAttachments", false);
        let attachment_prefix = str_param(&node.parameters, "/dataPropertyAttachmentsPrefixName", "attachment_");
        let custom_email_config = str_param(&node.parameters, "/options/customEmailConfig", "[\"UNSEEN\"]");
        let track_last_message_id = node.parameters.pointer("/options/trackLastMessageId").and_then(Value::as_bool).unwrap_or(true);
        let force_reconnect_mins = node.parameters.pointer("/options/forceReconnect").and_then(Value::as_u64);
        let base_criteria = match build_criteria(&custom_email_config) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(workflowId = %workflow_id, node = %node.name, error = %e, "Email Trigger (IMAP): invalid Custom Email Rules; the listener will not start");
                return;
            }
        };

        let mut last_uid: Option<u32> = None;
        loop {
            let creds = match resolve_credential(&n8n, &node, "imap").await {
                Ok(v) => creds_from(&v),
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "Email Trigger (IMAP): could not read credentials; retrying");
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            let mut session = match connect(&creds).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "Email Trigger (IMAP): connection failed; retrying");
                    tokio::time::sleep(RECONNECT_BACKOFF).await;
                    continue;
                }
            };
            if let Err(e) = session.select(&mailbox).await {
                tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "Email Trigger (IMAP): could not open mailbox; retrying");
                let _ = session.logout().await;
                tokio::time::sleep(RECONNECT_BACKOFF).await;
                continue;
            }

            let reconnect_deadline = force_reconnect_mins.map(|m| tokio::time::Instant::now() + std::time::Duration::from_secs(m.max(1) * 60));
            loop {
                if let Some(deadline) = reconnect_deadline {
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                }
                match poll_once(&mut session, &base_criteria, &mut last_uid, track_last_message_id, activated_at, &post_process_action, &format, download_attachments, &attachment_prefix).await
                {
                    Ok(Some(items)) => fire(&n8n, &workflow_id, &node.name, items).await,
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(workflowId = %workflow_id, node = %node.name, error = %e, "Email Trigger (IMAP): error polling the mailbox; reconnecting");
                        break;
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            let _ = session.logout().await;
        }
    }
}
