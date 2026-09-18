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

    if let Some(headers) = ctx.parameters.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(v_str) = v.as_str() {
                request = request.header(k, v_str);
            }
        }
    }
    if let Some(query) = ctx.parameters.get("query").and_then(|v| v.as_object()) {
        let pairs: Vec<(String, String)> = query
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        request = request.query(&pairs);
    }
    if let Some(body) = ctx.parameters.get("body") {
        if !body.is_null() {
            request = request.json(body);
        }
    }

    request = apply_auth(request, &ctx.parameters, &ctx.credentials)?;

    let response = request
        .send()
        .await
        .map_err(|e| NodeError::ExecutionFailed(format!("request failed: {e}")))?;

    let status = response.status();
    let response_json: serde_json::Value = response.json().await.unwrap_or(serde_json::Value::Null);

    if !status.is_success() {
        return Err(NodeError::ExecutionFailed(format!("HTTP {status}: {response_json}")));
    }

    Ok(vec![vec![Item { json: response_json, binary: serde_json::json!({}) }]])
}

fn apply_auth(
    mut request: reqwest::RequestBuilder,
    parameters: &serde_json::Value,
    credentials: &std::collections::HashMap<Uuid, serde_json::Value>,
) -> Result<reqwest::RequestBuilder, NodeError> {
    let auth = match parameters.get("auth") {
        Some(a) => a,
        None => return Ok(request),
    };
    let auth_type = auth.get("type").and_then(|v| v.as_str()).unwrap_or("none");
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
            tool_executor: None,
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
