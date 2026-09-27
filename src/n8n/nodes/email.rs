//! Send Email (spec §6.6): sends mail over SMTP using the `smtp` credential,
//! matching n8n's `EmailSend` node (`n8n-nodes-base.emailSend`) v2/v2.1,
//! which under the hood is nodemailer's SMTP transport.
//!
//! `sendAndWait` (the v2.1 second operation) is out of scope: it returns a
//! clear "not supported natively yet" error.
//!
//! The "Append n8n Attribution" text is reproduced byte-for-byte from n8n's
//! `send.operation.js` (including the literal word "n8n" and the link to
//! n8n.io) rather than rebranded to r8r: the option controls whether n8n's
//! own attribution notice is appended, and changing its wording would be a
//! different feature. r8r has no "instance ID" concept, so the UTM link
//! omits the `_<instanceId>` suffix n8n appends when it has one.

use super::field_list;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, Mailboxes, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde_json::{json, Value};
use std::str::FromStr;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(EmailSend)]
}

pub struct EmailSend;

/// n8n's `createUtmCampaignLink` + the literal attribution text from
/// `send.operation.js`.
fn attribution_link() -> String {
    "https://n8n.io/?utm_source=n8n-internal&utm_medium=powered_by&utm_campaign=n8n-nodes-base.emailSend".to_string()
}

fn append_attribution(email_format: &str, text: &str, html: &str) -> (String, String) {
    let attribution_text = "This email was sent automatically with ";
    let link = attribution_link();
    if email_format == "html" || (email_format == "both" && !html.is_empty()) {
        let new_html = format!("\n\t\t\t\t\t{html}\n\t\t\t\t\t<br>\n\t\t\t\t\t<br>\n\t\t\t\t\t---\n\t\t\t\t\t<br>\n\t\t\t\t\t<em>{attribution_text}<a href=\"{link}\" target=\"_blank\">n8n</a></em>\n\t\t\t\t\t");
        (text.to_string(), new_html)
    } else {
        let new_text = format!("{text}\n\n---\n{attribution_text}n8n\nhttps://n8n.io");
        (new_text, html.to_string())
    }
}

/// Parses a (possibly comma-separated, possibly `Name <email>`) address
/// list, the way nodemailer accepts `to`/`cc`/`bcc`/`replyTo`/`from`.
fn parse_mailboxes(field: &str, value: &str) -> NodeResult<Vec<Mailbox>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let boxes = Mailboxes::from_str(value)
        .map_err(|e| NodeError::new(format!("The \"{field}\" field has an invalid email address \"{value}\": {e}")))?;
    Ok(boxes.into_iter().collect())
}

fn parse_one_mailbox(field: &str, value: &str) -> NodeResult<Option<Mailbox>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    Mailbox::from_str(value.trim())
        .map(Some)
        .map_err(|e| NodeError::new(format!("The \"{field}\" field has an invalid email address \"{value}\": {e}")))
}

/// Decodes the base64 `data` of a binary property, returning its bytes,
/// mime type and file name, the same two-stage "no binary at all" / "no such
/// property" error compression.rs and files.rs raise.
fn binary_attachment<'a>(item: &'a Item, prop: &str, i: usize) -> NodeResult<(Vec<u8>, String, String)> {
    let binary = item.binary.as_ref().ok_or_else(|| {
        NodeError::new(format!("This operation expects the node's input data to contain a binary file '{prop}', but none was found [item {i}]")).at(i)
    })?;
    let entry = binary.get(prop).ok_or_else(|| NodeError::new(format!("The item has no binary field '{prop}'")).at(i))?;
    let data = entry.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Binary field '{prop}' has no data")).at(i))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| NodeError::new(format!("Binary field '{prop}' is not valid base64: {e}")).at(i))?;
    let mime = entry.get("mimeType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string();
    let file_name = entry.get("fileName").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("unknown").to_string();
    Ok((bytes, mime, file_name))
}

/// SMTP `secure` + `disableStartTls` credential fields -> lettre's `Tls`
/// mode, mirroring nodemailer's `secure` / `ignoreTLS` semantics:
/// `secure: true` connects TLS-wrapped from the start; `secure: false`
/// opportunistically upgrades with STARTTLS unless `disableStartTls` (n8n's
/// analogue of nodemailer's `ignoreTLS`) turns that off entirely.
fn tls_mode(host: &str, secure: bool, disable_start_tls: bool, allow_unauthorized: bool) -> NodeResult<Tls> {
    if !secure && disable_start_tls {
        return Ok(Tls::None);
    }
    let mut builder = TlsParameters::builder(host.to_string());
    if allow_unauthorized {
        builder = builder.dangerous_accept_invalid_certs(true).dangerous_accept_invalid_hostnames(true);
    }
    let params = builder.build().map_err(|e| NodeError::new(format!("Could not set up TLS: {e}")))?;
    Ok(if secure { Tls::Wrapper(params) } else { Tls::Opportunistic(params) })
}

#[async_trait::async_trait]
impl NodeType for EmailSend {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.emailSend"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let node_version = ctx.node.type_version;

        let (_, cred) = ctx.credentials("smtp").await?;
        let host = cred["host"].as_str().unwrap_or("").to_string();
        let port = cred["port"].as_u64().unwrap_or(465) as u16;
        let secure = cred["secure"].as_bool().unwrap_or(true);
        let disable_start_tls = cred["disableStartTls"].as_bool().unwrap_or(false);
        let user = cred["user"].as_str().unwrap_or("").to_string();
        let password = cred["password"].as_str().unwrap_or("").to_string();
        let host_name = cred["hostName"].as_str().unwrap_or("").to_string();

        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            let result = self.send_one(ctx, i, item, node_version, &host, port, secure, disable_start_tls, &user, &password, &host_name).await;
            match result {
                Ok(v) => out.push(v),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}

impl EmailSend {
    #[allow(clippy::too_many_arguments)]
    async fn send_one(
        &self,
        ctx: &ExecCtx<'_>,
        i: usize,
        item: &Item,
        node_version: f64,
        host: &str,
        port: u16,
        secure: bool,
        disable_start_tls: bool,
        user: &str,
        password: &str,
        host_name: &str,
    ) -> NodeResult<Item> {
        let operation = ctx.param_str("operation", i, "send")?;
        if operation != "send" {
            return Err(NodeError::new(format!("The Send Email operation \"{operation}\" is not supported natively yet")).at(i));
        }

        let from_email = ctx.param_str("fromEmail", i, "")?;
        let to_email = ctx.param_str("toEmail", i, "")?;
        let subject = ctx.param_str("subject", i, "")?;
        let default_format = if node_version >= 2.1 { "html" } else { "text" };
        let email_format = ctx.param_str("emailFormat", i, default_format)?;
        let mut text = if email_format == "text" || email_format == "both" { ctx.param_str("text", i, "")? } else { String::new() };
        let mut html = if email_format == "html" || email_format == "both" { ctx.param_str("html", i, "")? } else { String::new() };

        let cc_email = ctx.param_str("options.ccEmail", i, "")?;
        let bcc_email = ctx.param_str("options.bccEmail", i, "")?;
        let reply_to = ctx.param_str("options.replyTo", i, "")?;
        let allow_unauthorized_certs = ctx.param_bool("options.allowUnauthorizedCerts", i, false)?;
        let attachments_param = ctx.param_str("options.attachments", i, "")?;

        let append_attribution_default = node_version >= 2.1;
        let append_attribution_opt = match ctx.raw_param("options.appendAttribution") {
            Some(Value::Null) | None => append_attribution_default,
            Some(_) => ctx.param_bool("options.appendAttribution", i, append_attribution_default)?,
        };
        if append_attribution_opt {
            let (t, h) = append_attribution(&email_format, &text, &html);
            text = t;
            html = h;
        }

        let from_mailbox = parse_one_mailbox("fromEmail", &from_email)?
            .ok_or_else(|| NodeError::new("The \"From Email\" field is required").at(i))?;
        let to_mailboxes = parse_mailboxes("toEmail", &to_email)?;
        if to_mailboxes.is_empty() {
            return Err(NodeError::new("The \"To Email\" field is required").at(i));
        }
        let cc_mailboxes = parse_mailboxes("options.ccEmail", &cc_email)?;
        let bcc_mailboxes = parse_mailboxes("options.bccEmail", &bcc_email)?;
        let reply_to_mailbox = parse_one_mailbox("options.replyTo", &reply_to)?;

        let mut builder = Message::builder().from(from_mailbox).subject(subject.clone()).message_id(None);
        for mbox in to_mailboxes {
            builder = builder.to(mbox);
        }
        for mbox in cc_mailboxes {
            builder = builder.cc(mbox);
        }
        for mbox in bcc_mailboxes {
            builder = builder.bcc(mbox);
        }
        if let Some(mbox) = reply_to_mailbox {
            builder = builder.reply_to(mbox);
        }

        // Build the content body (text, html or both).
        let content: ContentPart = match email_format.as_str() {
            "text" => ContentPart::Single(SinglePart::plain(text.clone())),
            "html" => ContentPart::Single(SinglePart::html(html.clone())),
            "both" => ContentPart::Multi(MultiPart::alternative_plain_html(text.clone(), html.clone())),
            other => return Err(NodeError::new(format!("The Email Format \"{other}\" is not supported")).at(i)),
        };

        // Attachments: comma-separated binary property names, attached as
        // regular files.
        let attachment_names = field_list(&Value::String(attachments_param));
        let mut attachment_parts = Vec::new();
        for name in &attachment_names {
            let (bytes, mime, file_name) = binary_attachment(item, name, i)?;
            let content_type = ContentType::parse(&mime).unwrap_or_else(|_| ContentType::parse("application/octet-stream").unwrap());
            attachment_parts.push(Attachment::new(file_name).body(bytes, content_type));
        }

        let message = if attachment_parts.is_empty() {
            match content {
                ContentPart::Single(p) => builder.singlepart(p),
                ContentPart::Multi(p) => builder.multipart(p),
            }
        } else {
            let mixed_builder = MultiPart::mixed();
            let mut mixed = match content {
                ContentPart::Single(p) => mixed_builder.singlepart(p),
                ContentPart::Multi(p) => mixed_builder.multipart(p),
            };
            for part in attachment_parts {
                mixed = mixed.singlepart(part);
            }
            builder.multipart(mixed)
        }
        .map_err(|e| NodeError::new(format!("Could not build the email: {e}")).at(i))?;

        let envelope_from = message.envelope().from().map(|a| a.to_string()).unwrap_or_default();
        let envelope_to: Vec<String> = message.envelope().to().iter().map(|a| a.to_string()).collect();
        let message_id = message
            .headers()
            .get_raw("Message-ID")
            .map(String::from)
            .unwrap_or_default();

        let tls = tls_mode(host, secure, disable_start_tls, allow_unauthorized_certs).map_err(|e| e.at(i))?;
        let mut transport_builder = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host).port(port).tls(tls);
        if !host_name.is_empty() {
            transport_builder = transport_builder.hello_name(lettre::transport::smtp::extension::ClientId::Domain(host_name.to_string()));
        }
        if !user.is_empty() || !password.is_empty() {
            transport_builder = transport_builder.credentials(Credentials::new(user.to_string(), password.to_string()));
        }
        let transport: AsyncSmtpTransport<Tokio1Executor> = transport_builder.build();

        let response = transport.send(message).await.map_err(|e| {
            NodeError::api(format!("SMTP error: {e}"), None, None).at(i)
        })?;

        let response_text = response.message().collect::<Vec<_>>().join("; ");
        let json_out = json!({
            "accepted": envelope_to.clone(),
            "rejected": Value::Array(vec![]),
            "envelope": {"from": envelope_from, "to": envelope_to},
            "messageId": message_id,
            "response": response_text,
        });
        Ok(Item::from_value(json_out).paired(i))
    }
}

enum ContentPart {
    Single(SinglePart),
    Multi(MultiPart),
}
