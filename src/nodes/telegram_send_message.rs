use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

pub struct TelegramSendMessageNode;

/// Timeout applied to every request made by the client below. Mirrors
/// `core.httpRequest`'s rationale (see that node's `REQUEST_TIMEOUT` doc
/// comment): manual workflow execution runs synchronously inside the Axum
/// request handler, so an unresponsive upstream would otherwise hang the
/// request forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_API_BASE_URL: &str = "https://api.telegram.org";

/// Returns a `reqwest::Client` shared across every `execute()` call, built
/// lazily on first use. Deliberately a separate client/`OnceLock` from
/// `core.httpRequest`'s -- see this plan's Global Constraints -- not shared,
/// as a documented simplicity choice.
fn http_client() -> Result<&'static reqwest::Client, NodeError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                // Telegram's Bot API embeds the bot token in the request URL
                // (see the SECURITY note on `execute_with_client` below). A
                // redirect response would make reqwest re-issue the request
                // to a new host while auto-attaching a `Referer` header
                // containing that token-bearing URL -- leaking it outside
                // this node's error-message discipline. Telegram's real API
                // never legitimately redirects `sendMessage` calls, so
                // disabling both costs nothing functionally.
                .redirect(reqwest::redirect::Policy::none())
                .referer(false)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|_| NodeError::ExecutionFailed("telegram.sendMessage: failed to build HTTP client".into()))
}

#[async_trait]
impl Node for TelegramSendMessageNode {
    fn type_name(&self) -> &'static str {
        "telegram.sendMessage"
    }
    fn runs_per_item(&self) -> bool {
        true
    }
    fn display_name(&self) -> &'static str {
        "Send Telegram Message"
    }
    fn description(&self) -> &'static str {
        "Sends a message, photo, document, video, audio or animation via the configured Telegram bot."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn keeps_input_fields(&self) -> bool {
        true
    }
    fn icon(&self) -> &'static str {
        "📤"
    }
    fn credential_types(&self) -> &'static [&'static str] {
        &["telegramApi"]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        execute_with_client(http_client()?, ctx).await
    }
}

/// The actual request-building and dispatch logic, parameterized over the
/// client so tests can exercise it against a mock server.
///
/// SECURITY: Telegram's Bot API embeds the bot token directly in the
/// request URL (`https://api.telegram.org/bot<TOKEN>/sendMessage`), unlike
/// `core.httpRequest` where the URL is never secret. `reqwest::Error`'s
/// `Display` output can include the URL it was building/sending, so every
/// error path below is written to never interpolate the raw
/// `reqwest::Error` value, the constructed `url` variable, or the raw
/// `bot_token` value -- only generic, constant descriptions (plus, in the
/// `ok: false` branch, Telegram's own non-secret `description` field from
/// the parsed response body).
/// The `sendMessage` body: chat and text, plus the options that are set
/// (n8n's "Reply Markup" and "Additional Fields"). Unset, empty or false
/// options are left out, as Telegram's defaults then apply.
fn message_body(p: &serde_json::Value, chat_id: serde_json::Value, text: &str) -> serde_json::Value {
    let mut body = serde_json::json!({"chat_id": chat_id, "text": text});
    let set = |v: &serde_json::Value| !(v.is_null() || v == &serde_json::json!("") || v == &serde_json::json!(false));
    for key in ["parse_mode", "reply_markup", "disable_notification", "protect_content", "message_thread_id"] {
        if let Some(v) = p.get(key).filter(|v| set(v)) {
            body[key] = v.clone();
        }
    }
    if let Some(id) = p.get("reply_to_message_id").filter(|v| set(v)) {
        body["reply_parameters"] = serde_json::json!({"message_id": id});
    }
    if p.get("disable_web_page_preview").and_then(|v| v.as_bool()) == Some(true) {
        body["link_preview_options"] = serde_json::json!({"is_disabled": true});
    }
    body
}

/// The operations: the Bot API method, and the field its file goes in
/// (None for a text message).
const OPERATIONS: [(&str, Option<&str>); 6] = [
    ("sendMessage", None),
    ("sendPhoto", Some("photo")),
    ("sendDocument", Some("document")),
    ("sendVideo", Some("video")),
    ("sendAudio", Some("audio")),
    ("sendAnimation", Some("animation")),
];

/// The Bot API method and its body for the node's `operation` (a text
/// message by default; a photo, document, video, audio or animation by
/// link or file_id, with an optional caption).
fn request_body(
    p: &serde_json::Value,
    chat_id: serde_json::Value,
) -> Result<(&'static str, serde_json::Value), NodeError> {
    let fail = |m: String| NodeError::ExecutionFailed(format!("telegram.sendMessage: {m}"));
    let op = p
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("sendMessage");
    let (method, field) = OPERATIONS
        .iter()
        .find(|(m, _)| *m == op)
        .copied()
        .ok_or_else(|| {
            fail(format!(
                "unknown operation \"{op}\" (expected one of {})",
                OPERATIONS.map(|(m, _)| m).join(", ")
            ))
        })?;
    let Some(field) = field else {
        let text = p.get("text").and_then(|v| v.as_str()).ok_or_else(|| {
            NodeError::ExecutionFailed("telegram.sendMessage requires a \"text\" parameter".into())
        })?;
        return Ok((method, message_body(p, chat_id, text)));
    };
    let file = p
        .get("file")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .ok_or_else(|| fail(format!("enter the {field}'s link (or a Telegram file_id)")))?;
    // A link Telegram downloads itself; anything without "://" is a file_id.
    if file.contains("://") && !(file.starts_with("https://") || file.starts_with("http://")) {
        return Err(fail(format!("\"{file}\" isn't an http(s) link")));
    }
    let mut body = message_body(p, chat_id, "");
    let obj = body.as_object_mut().expect("message_body builds an object");
    obj.remove("text");
    // Link previews are for text messages only.
    obj.remove("link_preview_options");
    obj.insert(field.to_string(), serde_json::json!(file));
    if let Some(caption) = p
        .get("caption")
        .and_then(|v| v.as_str())
        .filter(|c| !c.is_empty())
    {
        obj.insert("caption".into(), serde_json::json!(caption));
    }
    // The additional fields each kind of file takes, when set.
    let extras: &[&str] = match field {
        "photo" => &["has_spoiler"],
        "video" => &[
            "has_spoiler",
            "supports_streaming",
            "duration",
            "width",
            "height",
        ],
        "animation" => &["has_spoiler", "duration", "width", "height"],
        "audio" => &["performer", "title", "duration"],
        _ => &[],
    };
    for key in extras {
        if let Some(v) = p.get(*key).filter(|v| {
            !(v.is_null() || **v == serde_json::json!("") || **v == serde_json::json!(false))
        }) {
            obj.insert(key.to_string(), v.clone());
        }
    }
    Ok((method, body))
}

async fn execute_with_client(
    client: &reqwest::Client,
    ctx: &NodeExecutionContext,
) -> Result<NodeOutput, NodeError> {
    let chat_id = ctx
        .parameters
        .get("chat_id")
        .ok_or_else(|| NodeError::ExecutionFailed("telegram.sendMessage requires a \"chat_id\" parameter".into()))?
        .clone();
    let (method, body) = request_body(&ctx.parameters, chat_id)?;

    let credential_id_str = ctx
        .parameters
        .get("auth")
        .and_then(|a| a.get("credential_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("telegram.sendMessage requires auth.credential_id".into()))?;
    let credential_id = Uuid::parse_str(credential_id_str)
        .map_err(|e| NodeError::ExecutionFailed(format!("invalid credential_id: {e}")))?;
    let credential_data = ctx
        .credentials
        .get(&credential_id)
        .ok_or_else(|| NodeError::ExecutionFailed(format!("credential {credential_id} was not resolved for this run")))?;
    let bot_token = credential_data
        .get("bot_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("telegramApi credential missing \"bot_token\"".into()))?;

    // The bot token goes only where its credential says: a node-set address
    // could be pointed anywhere by whoever edits the workflow.
    if ctx.parameters.get("api_base_url").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()) {
        return Err(NodeError::ExecutionFailed(
            "telegram.sendMessage: api_base_url can't be set on the node; set the Base URL on the Telegram credential instead".into(),
        ));
    }
    let base_url = credential_data
        .get("base_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_API_BASE_URL)
        .trim_end_matches('/');
    let url = format!("{base_url}/bot{bot_token}/{method}");

    // Never interpolate `e` (the reqwest::Error) or `url` into any error
    // message below -- both can carry the bot token embedded in `url`,
    // unlike core.httpRequest where the URL is never secret. See this
    // plan's Global Constraints.
    let response = client.post(&url).json(&body).send().await.map_err(|_| {
        NodeError::ExecutionFailed("telegram.sendMessage: request to Telegram API failed".into())
    })?;

    let status = response.status();
    let response_json: serde_json::Value = response.json().await.map_err(|_| {
        NodeError::ExecutionFailed(format!(
            "telegram.sendMessage: Telegram API returned HTTP {status} with a non-JSON response body"
        ))
    })?;

    let ok = response_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !status.is_success() || !ok {
        let description = response_json
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("no description provided");
        return Err(NodeError::ExecutionFailed(format!(
            "telegram.sendMessage: Telegram API returned an error: {description}"
        )));
    }

    Ok(vec![vec![Item { json: response_json, binary: serde_json::json!({}) }]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sends_message_with_bot_token_in_url_and_returns_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendMessage$"))
            .and(body_json(serde_json::json!({"chat_id": "42", "text": "hello"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "result": {"message_id": 42}
            })))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "123:ABC", "base_url": server.uri()}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "42",
                "text": "hello",
                "auth": {"credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
            tools: Default::default(),
            credential_types: Default::default(),
            tool_args: None,
            tool_executor: None,
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json["result"]["message_id"], 42);
    }

    fn send_ctx(server_uri: &str, params: serde_json::Value) -> NodeExecutionContext {
        let credential_id = Uuid::new_v4();
        let mut parameters = params;
        parameters["auth"] = serde_json::json!({"credential_id": credential_id.to_string()});
        NodeExecutionContext {
            parameters,
            credentials: std::collections::HashMap::from([(credential_id, serde_json::json!({"bot_token": "123:ABC", "base_url": server_uri}))]),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn sends_formatting_reply_markup_and_additional_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendMessage$"))
            .and(body_json(serde_json::json!({
                "chat_id": 42,
                "text": "*hi*",
                "parse_mode": "MarkdownV2",
                "reply_markup": {"inline_keyboard": [[{"text": "Yes", "callback_data": "yes"}, {"text": "Site", "url": "https://example.com"}]]},
                "disable_notification": true,
                "protect_content": true,
                "message_thread_id": 7,
                "reply_parameters": {"message_id": 99},
                "link_preview_options": {"is_disabled": true}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": {"message_id": 1}})))
            .expect(1)
            .mount(&server)
            .await;
        let ctx = send_ctx(&server.uri(), serde_json::json!({
            "chat_id": 42,
            "text": "*hi*",
            "parse_mode": "MarkdownV2",
            "reply_markup": {"inline_keyboard": [[{"text": "Yes", "callback_data": "yes"}, {"text": "Site", "url": "https://example.com"}]]},
            "disable_notification": true,
            "protect_content": true,
            "message_thread_id": 7,
            "reply_to_message_id": 99,
            "disable_web_page_preview": true
        }));
        TelegramSendMessageNode.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn sends_a_photo_by_link_with_a_caption_and_buttons() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendPhoto$"))
            .and(body_json(serde_json::json!({
                "chat_id": 42,
                "photo": "https://upload.wikimedia.org/a/Cat.jpg",
                "caption": "<b>A cat</b>",
                "parse_mode": "HTML",
                "reply_markup": {"inline_keyboard": [[{"text": "Source", "url": "https://commons.wikimedia.org"}]]}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": {"message_id": 2, "photo": []}})))
            .expect(1)
            .mount(&server)
            .await;
        let ctx = send_ctx(
            &server.uri(),
            serde_json::json!({
                "operation": "sendPhoto",
                "chat_id": 42,
                "file": "https://upload.wikimedia.org/a/Cat.jpg",
                "caption": "<b>A cat</b>",
                "parse_mode": "HTML",
                "reply_markup": {"inline_keyboard": [[{"text": "Source", "url": "https://commons.wikimedia.org"}]]},
                "disable_web_page_preview": true
            }),
        );
        TelegramSendMessageNode.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn sends_a_document_by_file_id_without_a_caption() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendDocument$"))
            .and(body_json(
                serde_json::json!({"chat_id": "42", "document": "BQACAgIAAxkBAAI"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let ctx = send_ctx(
            &server.uri(),
            serde_json::json!({"operation": "sendDocument", "chat_id": "42", "file": "BQACAgIAAxkBAAI"}),
        );
        TelegramSendMessageNode.execute(&ctx).await.unwrap();
    }

    #[test]
    fn a_media_operation_needs_a_file_and_an_http_link() {
        let body = |p: serde_json::Value| {
            request_body(&p, serde_json::json!(1))
                .map(|(m, b)| (m, b))
                .map_err(|e| e.to_string())
        };
        assert!(body(serde_json::json!({"operation": "sendPhoto"}))
            .unwrap_err()
            .contains("enter the photo's link"));
        assert!(
            body(serde_json::json!({"operation": "sendVideo", "file": "ftp://x/v.mp4"}))
                .unwrap_err()
                .contains("isn't an http(s) link")
        );
        assert!(
            body(serde_json::json!({"operation": "sendSticker", "file": "x"}))
                .unwrap_err()
                .contains("unknown operation")
        );
        assert_eq!(
            body(serde_json::json!({"text": "hi"})).unwrap().0,
            "sendMessage",
            "a text message by default"
        );
        // Each kind keeps only its own additional fields.
        let (m, b) = body(serde_json::json!({"operation": "sendAudio", "file": "https://x/a.mp3", "performer": "Ada", "title": "Song", "has_spoiler": true})).unwrap();
        assert_eq!(
            (m, b),
            (
                "sendAudio",
                serde_json::json!({"chat_id": 1, "audio": "https://x/a.mp3", "performer": "Ada", "title": "Song"})
            )
        );
        let (_, v) = body(serde_json::json!({"operation": "sendVideo", "file": "https://x/v.mp4", "has_spoiler": true, "supports_streaming": true, "duration": 30})).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"chat_id": 1, "video": "https://x/v.mp4", "has_spoiler": true, "supports_streaming": true, "duration": 30})
        );
    }

    #[tokio::test]
    async fn options_left_unset_are_not_sent() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_json(serde_json::json!({"chat_id": "1", "text": "plain"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true, "result": {}})))
            .expect(1)
            .mount(&server)
            .await;
        let ctx = send_ctx(&server.uri(), serde_json::json!({
            "chat_id": "1", "text": "plain", "parse_mode": "", "reply_markup": null, "disable_notification": false, "message_thread_id": ""
        }));
        TelegramSendMessageNode.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn the_bot_token_never_goes_to_a_node_set_address() {
        let attacker = MockServer::start().await;
        Mock::given(wiremock::matchers::any()).respond_with(ResponseTemplate::new(200)).mount(&attacker).await;
        let mut ctx = send_ctx("https://api.telegram.org", serde_json::json!({"chat_id": "1", "text": "hi"}));
        ctx.parameters["api_base_url"] = serde_json::json!(attacker.uri());
        let err = TelegramSendMessageNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("api_base_url") && err.contains("credential"), "{err}");
        assert!(attacker.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn telegram_ok_false_returns_error_with_description() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/bot123:ABC/sendMessage$"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "ok": false,
                "error_code": 400,
                "description": "Bad Request: chat not found"
            })))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "123:ABC", "base_url": server.uri()}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "999999",
                "text": "hello",
                "auth": {"credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
            tools: Default::default(),
            credential_types: Default::default(),
            tool_args: None,
            tool_executor: None,
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => assert!(msg.contains("chat not found")),
            other => panic!("expected ExecutionFailed with description, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_chat_id_returns_error() {
        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"text": "hello"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn unresolved_credential_returns_error() {
        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "1",
                "text": "hi",
                "auth": {"credential_id": Uuid::new_v4().to_string()}
            }),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn network_failure_error_message_never_contains_bot_token() {
        // Point at a server that isn't listening (a MockServer we start then
        // immediately drop, freeing its port) to provoke a real
        // connection-level reqwest::Error, and confirm the bot token never
        // appears anywhere in the resulting error message -- proving the
        // "never interpolate the raw reqwest::Error or URL" discipline
        // actually holds, not just that it's written that way.
        let server = MockServer::start().await;
        let dead_uri = server.uri();
        drop(server);

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"bot_token": "SECRET-BOT-TOKEN-VALUE", "base_url": dead_uri}));

        let node = TelegramSendMessageNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "chat_id": "1",
                "text": "hi",
                "auth": {"credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
            tools: Default::default(),
            credential_types: Default::default(),
            tool_args: None,
            tool_executor: None,
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        match result {
            Err(NodeError::ExecutionFailed(msg)) => {
                assert!(!msg.contains("SECRET-BOT-TOKEN-VALUE"));
            }
            other => panic!("expected a connection-level ExecutionFailed, got {other:?}"),
        }
    }
}
