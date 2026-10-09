use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::sync::OnceLock;
use std::time::Duration;
use uuid::Uuid;

pub struct HttpRequestNode;

/// Timeout applied to every request made by the shared HTTP client below.
/// Manual workflow execution runs synchronously inside the Axum request
/// handler, so without a timeout an unresponsive upstream host would hang
/// the request forever, leaving the persisted `Execution` row stuck at
/// `status: Running` (and, for a schedule-triggered run, pinning a
/// scheduler task slot) with no way to recover short of restarting the
/// process.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Returns a `reqwest::Client` shared across every `execute()` call,
/// building it lazily on first use. Constructing a brand-new client per
/// call would discard connection pooling and TLS session reuse, which is
/// an anti-pattern per reqwest's own documentation.
fn http_client() -> Result<&'static reqwest::Client, NodeError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| NodeError::ExecutionFailed(format!("failed to build HTTP client: {e}")))
}

#[async_trait]
impl Node for HttpRequestNode {
    fn type_name(&self) -> &'static str {
        "core.httpRequest"
    }
    fn runs_per_item(&self) -> bool {
        true
    }
    fn display_name(&self) -> &'static str {
        "HTTP Request"
    }
    fn description(&self) -> &'static str {
        "Makes an HTTP request to an external URL."
    }
    fn credential_types(&self) -> &'static [&'static str] {
        // Picked by `auth.type`: bearer, apiKey, basic.
        &["bearerToken", "apiKeyHeader", "basicAuth"]
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn keeps_input_fields(&self) -> bool {
        true
    }
    fn icon(&self) -> &'static str {
        "🌐"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        execute_with_client(http_client()?, ctx).await
    }
}

/// The actual request-building and dispatch logic, parameterized over the
/// client so tests can exercise it with a client configured for a much
/// shorter timeout than the real 30s default (see the timeout test below).
async fn execute_with_client(
    client: &reqwest::Client,
    ctx: &NodeExecutionContext,
) -> Result<NodeOutput, NodeError> {
    let method = ctx.parameters.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
    let url = ctx
        .parameters
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed("core.httpRequest requires a \"url\" parameter".into()))?;

    let method: reqwest::Method = method
        .parse()
        .map_err(|_| NodeError::ExecutionFailed(format!("invalid HTTP method: {method}")))?;
    let mut request = client.request(method, url);

    // Values may be numbers or booleans (an expression's result): sent as text.
    if let Some(headers) = ctx.parameters.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(v) = scalar_text(v) {
                request = request.header(k, v);
            }
        }
    }
    if let Some(query) = ctx.parameters.get("query").and_then(|v| v.as_object()) {
        let pairs: Vec<(String, String)> = query.iter().filter_map(|(k, v)| scalar_text(v).map(|s| (k.clone(), s))).collect();
        request = request.query(&pairs);
    }
    // The body as JSON (default), a form (`body_type: "form"`, an object of
    // fields) or plain text (`"text"`).
    if let Some(body) = ctx.parameters.get("body").filter(|b| !b.is_null()) {
        request = match ctx.parameters.get("body_type").and_then(|v| v.as_str()).unwrap_or("json") {
            "json" => request.json(body),
            "form" => {
                let fields = body
                    .as_object()
                    .ok_or_else(|| NodeError::ExecutionFailed("core.httpRequest: a form body must be an object of fields".into()))?;
                let pairs: Vec<(String, String)> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), scalar_text(v).unwrap_or_else(|| v.to_string())))
                    .collect();
                request.form(&pairs)
            }
            "text" => {
                let text = body.as_str().map(str::to_string).unwrap_or_else(|| body.to_string());
                let has_type = ctx.parameters.get("headers").and_then(|h| h.as_object()).is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("content-type")));
                let request = request.body(text);
                if has_type { request } else { request.header("content-type", "text/plain; charset=utf-8") }
            }
            other => return Err(NodeError::ExecutionFailed(format!("core.httpRequest: unknown body_type \"{other}\" (expected json, form or text)"))),
        };
    }
    // Per request, within the client's own limit.
    if let Some(ms) = ctx.parameters.get("timeout_ms").and_then(|v| v.as_u64()).filter(|ms| *ms > 0) {
        request = request.timeout(std::time::Duration::from_millis(ms));
    }

    request = apply_auth(request, &ctx.parameters, &ctx.credentials, &ctx.credential_types)?;

    let response = request
        .send()
        .await
        .map_err(|e| NodeError::ExecutionFailed(format!("request failed: {e}")))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| NodeError::ExecutionFailed(format!("reading the response failed: {e}")))?;
    // `response_format`: auto (JSON as itself, anything else under `data`),
    // json (must be JSON) or text (always under `data`).
    let body: serde_json::Value = match ctx.parameters.get("response_format").and_then(|v| v.as_str()).unwrap_or("auto") {
        "text" => serde_json::json!({ "data": text }),
        "json" => serde_json::from_str(&text).map_err(|_| {
            NodeError::ExecutionFailed(format!("core.httpRequest: the response isn't JSON (HTTP {status}); set the response format to text to take it as it is"))
        })?,
        _ => serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({ "data": text })),
    };

    if !status.is_success() {
        return Err(NodeError::ExecutionFailed(format!("HTTP {status}: {body}")));
    }

    Ok(vec![response_items(body)])
}

/// A JSON array of objects is one item per object (as n8n does); any other
/// response is one item, a bare value or array of values under `data`.
fn response_items(body: serde_json::Value) -> Vec<Item> {
    let item = |json| Item { json, binary: serde_json::json!({}) };
    match body {
        serde_json::Value::Array(values) if !values.is_empty() && values.iter().all(|v| v.is_object()) => values.into_iter().map(item).collect(),
        serde_json::Value::Object(_) => vec![item(body)],
        other => vec![item(serde_json::json!({ "data": other }))],
    }
}

fn scalar_text(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn apply_auth(
    mut request: reqwest::RequestBuilder,
    parameters: &serde_json::Value,
    credentials: &std::collections::HashMap<Uuid, serde_json::Value>,
    credential_types: &std::collections::HashMap<Uuid, String>,
) -> Result<reqwest::RequestBuilder, NodeError> {
    let auth = match parameters.get("auth") {
        Some(a) => a,
        None => return Ok(request),
    };
    // No type given: the chosen credential's type says how it is sent.
    let inferred = auth
        .get("credential_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
        .and_then(|id| credential_types.get(&id))
        .and_then(|t| match t.as_str() {
            "bearerToken" => Some("bearer"),
            "apiKeyHeader" => Some("apiKey"),
            "basicAuth" => Some("basic"),
            _ => None,
        });
    let auth_type = auth.get("type").and_then(|v| v.as_str()).or(inferred).unwrap_or("none");
    if auth_type == "none" {
        return Ok(request);
    }

    let credential_id_str = auth
        .get("credential_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed(format!("auth.type \"{auth_type}\" requires a credential_id")))?;
    let credential_id = Uuid::parse_str(credential_id_str)
        .map_err(|e| NodeError::ExecutionFailed(format!("invalid credential_id: {e}")))?;
    let data = credentials
        .get(&credential_id)
        .ok_or_else(|| NodeError::ExecutionFailed(format!("credential {credential_id} was not resolved for this run")))?;

    match auth_type {
        "bearer" => {
            let token = data
                .get("token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("bearer credential missing \"token\"".into()))?;
            request = request.bearer_auth(token);
        }
        "apiKey" => {
            let header_name = data
                .get("header_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("apiKey credential missing \"header_name\"".into()))?;
            let value = data
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("apiKey credential missing \"value\"".into()))?;
            request = request.header(header_name, value);
        }
        "basic" => {
            let username = data
                .get("username")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("basic credential missing \"username\"".into()))?;
            let password = data.get("password").and_then(|v| v.as_str());
            request = request.basic_auth(username, password);
        }
        other => {
            return Err(NodeError::ExecutionFailed(format!("unknown auth.type: {other}")));
        }
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn get(_server: &MockServer, parameters: serde_json::Value, credential_types: std::collections::HashMap<Uuid, String>, credentials: std::collections::HashMap<Uuid, serde_json::Value>) -> Vec<serde_json::Value> {
        let ctx = NodeExecutionContext { parameters, credentials, credential_types, ..Default::default() };
        HttpRequestNode.execute(&ctx).await.unwrap()[0].iter().map(|i| i.json.clone()).collect()
    }

    #[tokio::test]
    async fn sends_a_form_or_text_body_and_reads_the_response_as_asked() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/form"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(wiremock::matchers::body_string("name=Ada&n=2"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\": true}"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/text"))
            .and(header("content-type", "text/plain; charset=utf-8"))
            .and(wiremock::matchers::body_string("hello"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\": true}"))
            .mount(&server)
            .await;
        let form = serde_json::json!({"method": "POST", "url": format!("{}/form", server.uri()), "body_type": "form", "body": {"name": "Ada", "n": 2}});
        assert_eq!(get(&server, form, Default::default(), Default::default()).await, vec![serde_json::json!({"ok": true})]);
        let text = serde_json::json!({"method": "POST", "url": format!("{}/text", server.uri()), "body_type": "text", "body": "hello", "response_format": "text"});
        assert_eq!(get(&server, text, Default::default(), Default::default()).await, vec![serde_json::json!({"data": "{\"ok\": true}"})]);
    }

    #[tokio::test]
    async fn json_response_format_refuses_text_and_a_short_timeout_stops_the_wait() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/html")).respond_with(ResponseTemplate::new(200).set_body_string("<p>hi</p>")).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(500)))
            .mount(&server)
            .await;
        let run = |p: serde_json::Value| async move { HttpRequestNode.execute(&NodeExecutionContext { parameters: p, ..Default::default() }).await };
        let err = run(serde_json::json!({"url": format!("{}/html", server.uri()), "response_format": "json"})).await.unwrap_err();
        assert!(err.to_string().contains("isn't JSON"), "{err}");
        assert!(run(serde_json::json!({"url": format!("{}/slow", server.uri()), "timeout_ms": 50})).await.is_err());
    }

    #[tokio::test]
    async fn a_credential_without_an_auth_type_is_sent_as_its_type_says() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/me"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"me": 1})))
            .mount(&server)
            .await;
        let id = Uuid::new_v4();
        let out = get(
            &server,
            serde_json::json!({"url": format!("{}/me", server.uri()), "auth": {"credential_id": id.to_string()}}),
            std::collections::HashMap::from([(id, "bearerToken".to_string())]),
            std::collections::HashMap::from([(id, serde_json::json!({"token": "tok"}))]),
        )
        .await;
        assert_eq!(out, vec![serde_json::json!({"me": 1})]);
    }

    #[tokio::test]
    async fn numbers_in_query_and_headers_are_sent_as_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/q"))
            .and(wiremock::matchers::query_param("page", "2"))
            .and(header("x-n", "7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;
        let params = serde_json::json!({"url": format!("{}/q", server.uri()), "query": {"page": 2}, "headers": {"x-n": 7}});
        assert_eq!(get(&server, params, Default::default(), Default::default()).await, vec![serde_json::json!({"ok": true})]);
    }

    #[tokio::test]
    async fn an_array_of_objects_becomes_one_item_each_and_text_goes_under_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/list"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([{"id": 1}, {"id": 2}])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/page"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<h1>hi</h1>"))
            .mount(&server)
            .await;
        let list = get(&server, serde_json::json!({"url": format!("{}/list", server.uri())}), Default::default(), Default::default()).await;
        assert_eq!(list, vec![serde_json::json!({"id": 1}), serde_json::json!({"id": 2})]);
        let page = get(&server, serde_json::json!({"url": format!("{}/page", server.uri())}), Default::default(), Default::default()).await;
        assert_eq!(page, vec![serde_json::json!({"data": "<h1>hi</h1>"})]);
    }

    #[tokio::test]
    async fn get_request_returns_json_body_as_item() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"method": "GET", "url": format!("{}/data", server.uri())}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"ok": true}));
    }

    #[tokio::test]
    async fn bearer_auth_sends_authorization_header_from_resolved_credential() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secure"))
            .and(header("authorization", "Bearer secret-token-xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"authed": true})))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"token": "secret-token-xyz"}));

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": format!("{}/secure", server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id.to_string()}
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
        assert_eq!(result[0][0].json, serde_json::json!({"authed": true}));
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/broken"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"method": "GET", "url": format!("{}/broken", server.uri())}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn slow_response_past_configured_timeout_returns_execution_failed_error() {
        // The shared client built by `http_client()` uses a real 30s timeout,
        // which is much too long to wait on in a unit test. To prove the
        // timeout mechanism actually works without slowing down (or
        // flaking) the suite, this test drives `execute_with_client()`
        // directly -- the same request-building/dispatch code `execute()`
        // delegates to -- against a client built with the identical
        // `reqwest::Client::builder().timeout(...)` call but a much shorter
        // duration, hitting a mock server that delays its response past
        // that duration. An outer `tokio::time::timeout` is a belt-and-
        // braces bound so this test itself can never hang the suite even if
        // the timeout wiring were broken.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(500)))
            .mount(&server)
            .await;

        let short_timeout_client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();

        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"method": "GET", "url": format!("{}/slow", server.uri())}),
            input_items: vec![],
            ..Default::default()
        };

        let result = tokio::time::timeout(Duration::from_secs(5), execute_with_client(&short_timeout_client, &ctx))
            .await
            .expect("execute_with_client should not hang past the configured client timeout");

        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn missing_url_returns_error() {
        let node = HttpRequestNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![], ..Default::default() };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn auth_referencing_unresolved_credential_returns_error() {
        // credential_id points at something never put into ctx.credentials --
        // must fail loudly, not silently send an unauthenticated request.
        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": "http://example.invalid/",
                "auth": {"type": "bearer", "credential_id": Uuid::new_v4().to_string()}
            }),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
