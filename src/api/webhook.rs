use crate::domain::{ExecutionMode, ExecutionStatus, Item};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn handle_webhook(
    State(state): State<AppState>,
    Path((workflow_id, path)): Path<(Uuid, String)>,
    method: Method,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(workflow_id).await {
        Ok(Some(wf)) if wf.active => wf,
        Ok(_) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "webhook: failed to fetch workflow");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let start_id = match crate::engine::start_node_id(&workflow) {
        Ok(id) => id,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let start_node = match workflow.nodes.iter().find(|n| n.id == start_id) {
        Some(n) => n,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let Some(params) = webhook_node_matches(start_node, &path, method.as_str()) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    tracing::info!(
        workflow_id = %workflow.id,
        workflow_name = %workflow.name,
        %method,
        path = %format!("/webhook-r8r/{workflow_id}/{path}"),
        "webhook received"
    );
    let mut trigger_item = build_trigger_item(&headers, &query, &body);
    trigger_item.json["params"] = serde_json::Value::Object(params);

    let resources = match crate::credentials::resolve_run_resources(state.storage.as_ref(), &workflow).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, workflow_id = %workflow.id, "webhook: failed to resolve credentials");
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("credential resolution failed: {e}")).into_response();
        }
    };

    if let Err(refusal) = check_auth(start_node, &headers, &resources) {
        return refusal;
    }

    // The run is always a detached background task (Plan 8.7), so a caller
    // disconnecting never cancels it; `respond` only decides whether this
    // handler waits for the result.
    let p = &start_node.parameters;
    let respond = p.get("respond").and_then(|v| v.as_str());
    let respond_immediately = respond == Some("immediately");
    let response_code = p
        .get("response_code")
        .and_then(|v| v.as_u64())
        .and_then(|c| StatusCode::from_u16(c as u16).ok())
        // Nodes saved before response codes answered "immediately" with 202.
        .unwrap_or(if respond_immediately { StatusCode::ACCEPTED } else { StatusCode::OK });
    let (started, handle) = match crate::execution_runner::start_execution(
        state.storage.clone(),
        state.execution_events.clone(),
        state.registry.clone(),
        workflow.clone(),
        ExecutionMode::Webhook,
        Some(vec![trigger_item]),
        resources,
    )
    .await
    {
        Ok(started) => started,
        Err(e) => {
            tracing::error!(error = %e, "webhook: failed to persist new execution");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if respond_immediately {
        return (response_code, Json(serde_json::json!({ "message": "Workflow was started", "execution_id": started.id }))).into_response();
    }
    let execution = match handle.await {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(error = %e, "webhook: execution task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if respond == Some("lastNode") {
        if execution.status == ExecutionStatus::Error {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "message": "Error in workflow" }))).into_response();
        }
        let items = last_node_items(&workflow, &execution);
        return match p.get("response_data").and_then(|v| v.as_str()).unwrap_or("firstEntryJson") {
            "noData" => response_code.into_response(),
            "allEntries" => (response_code, Json(serde_json::Value::Array(items.iter().map(|i| i.json.clone()).collect()))).into_response(),
            _ => (response_code, Json(items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})))).into_response(),
        };
    }
    // Nodes saved before response modes: the whole run record.
    let response_status = if execution.status == ExecutionStatus::Error {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    (response_status, Json(execution)).into_response()
}

/// The items of the last node (in workflow order) that ran and succeeded.
fn last_node_items(workflow: &crate::domain::Workflow, execution: &crate::domain::Execution) -> Vec<Item> {
    let order = crate::engine::topological_order(workflow).unwrap_or_default();
    order
        .iter()
        .rev()
        .find(|n| execution.node_runs.get(&n.id).is_some_and(|r| r.status == crate::domain::NodeRunStatus::Success))
        .and_then(|n| execution.node_outputs.get(&n.id))
        .cloned()
        .unwrap_or_default()
}

/// The node's credential, when it has one: Basic Auth (user and password)
/// or API Key (Header) (a header and its value), checked as n8n does.
fn check_auth(
    node: &crate::domain::NodeInstance,
    headers: &HeaderMap,
    resources: &crate::credentials::RunResources,
) -> Result<(), axum::response::Response> {
    let Some(id) = node
        .parameters
        .get("auth")
        .and_then(|a| a.get("credential_id"))
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
    else {
        return Ok(());
    };
    let refuse = |code: StatusCode, message: &str| {
        let mut response = (code, Json(serde_json::json!({ "message": message }))).into_response();
        if code == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert("www-authenticate", axum::http::HeaderValue::from_static("Basic realm=\"Webhook\""));
        }
        response
    };
    let (Some(data), Some(kind)) = (resources.credentials.get(&id), resources.credential_types.get(&id)) else {
        return Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "The webhook's credential could not be loaded"));
    };
    let field = |k: &str| data.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match kind.as_str() {
        "basicAuth" => {
            let Some(given) = headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Basic ")) else {
                return Err(refuse(StatusCode::UNAUTHORIZED, "Authorization is required!"));
            };
            use base64::Engine;
            let expected = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", field("username"), field("password")));
            if constant_time_eq(given.trim().as_bytes(), expected.as_bytes()) {
                Ok(())
            } else {
                Err(refuse(StatusCode::FORBIDDEN, "Authorization data is wrong!"))
            }
        }
        "apiKeyHeader" => {
            let given = headers.get(field("header_name")).and_then(|v| v.to_str().ok()).unwrap_or("");
            if !field("value").is_empty() && constant_time_eq(given.as_bytes(), field("value").as_bytes()) {
                Ok(())
            } else {
                Err(refuse(StatusCode::FORBIDDEN, "Authorization data is wrong!"))
            }
        }
        _ => Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "Webhooks take a Basic Auth or API Key (Header) credential")),
    }
}

/// Equal without revealing, by timing, how much of a secret matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The `:param` values when the request matches the node's path and
/// method (`users/:id` matches `users/42` with `{"id": "42"}`), else None.
fn webhook_node_matches(node: &crate::domain::NodeInstance, path: &str, method: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    if node.node_type != "core.webhook" {
        return None;
    }
    let node_method = node.parameters.get("method").and_then(|v| v.as_str()).unwrap_or("ANY");
    if !(node_method.eq_ignore_ascii_case("ANY") || node_method.eq_ignore_ascii_case(method)) {
        return None;
    }
    let node_path = node.parameters.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let want: Vec<&str> = node_path.trim_matches('/').split('/').collect();
    let got: Vec<&str> = path.trim_matches('/').split('/').collect();
    if want.len() != got.len() {
        return None;
    }
    let mut params = serde_json::Map::new();
    for (w, g) in want.iter().zip(&got) {
        match w.strip_prefix(':') {
            Some(name) if !name.is_empty() && !g.is_empty() => {
                params.insert(name.to_string(), serde_json::Value::String(g.to_string()));
            }
            _ if w == g => {}
            _ => return None,
        }
    }
    Some(params)
}

fn build_trigger_item(headers: &HeaderMap, query: &HashMap<String, String>, body: &[u8]) -> Item {
    let headers_json: serde_json::Map<String, serde_json::Value> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_str().unwrap_or("").to_string())))
        .collect();
    let body_json = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice::<serde_json::Value>(body)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(body).to_string()))
    };
    Item {
        json: serde_json::json!({
            "headers": headers_json,
            "query": query,
            "body": body_json,
        }),
        binary: serde_json::json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::NodeInstance;

    fn webhook_node(path: &str, method: &str) -> NodeInstance {
        NodeInstance {
            id: "hook".into(),
            node_type: "core.webhook".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({"path": path, "method": method}),
            disabled: false,
            settings: Default::default(),
        }
    }

    #[test]
    fn a_path_with_params_matches_and_captures_them() {
        let node = webhook_node("users/:id/orders/:order", "GET");
        let params = webhook_node_matches(&node, "users/42/orders/7", "GET").unwrap();
        assert_eq!(serde_json::Value::Object(params), serde_json::json!({"id": "42", "order": "7"}));
        assert!(webhook_node_matches(&node, "users/42/orders", "GET").is_none());
        assert!(webhook_node_matches(&node, "people/42/orders/7", "GET").is_none());
        assert!(webhook_node_matches(&webhook_node("a/b", "ANY"), "/a/b/", "DELETE").is_some());
    }

    fn resources(kind: &str, data: serde_json::Value) -> (NodeInstance, crate::credentials::RunResources) {
        let id = Uuid::new_v4();
        let mut node = webhook_node("h", "POST");
        node.parameters["auth"] = serde_json::json!({"credential_id": id.to_string()});
        let mut r = crate::credentials::RunResources::default();
        r.credentials.insert(id, data);
        r.credential_types.insert(id, kind.into());
        (node, r)
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn basic_auth_needs_the_credentials_user_and_password() {
        let (node, r) = resources("basicAuth", serde_json::json!({"username": "u", "password": "p"}));
        // base64("u:p") = dTpw
        assert!(check_auth(&node, &headers(&[("authorization", "Basic dTpw")]), &r).is_ok());
        assert_eq!(check_auth(&node, &headers(&[]), &r).unwrap_err().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(check_auth(&node, &headers(&[("authorization", "Basic dTp4")]), &r).unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn header_auth_needs_the_header_and_its_value() {
        let (node, r) = resources("apiKeyHeader", serde_json::json!({"header_name": "x-key", "value": "s3cret"}));
        assert!(check_auth(&node, &headers(&[("x-key", "s3cret")]), &r).is_ok());
        assert_eq!(check_auth(&node, &headers(&[("x-key", "nope")]), &r).unwrap_err().status(), StatusCode::FORBIDDEN);
        assert_eq!(check_auth(&node, &headers(&[]), &r).unwrap_err().status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn a_node_without_a_credential_lets_everyone_in() {
        assert!(check_auth(&webhook_node("h", "POST"), &headers(&[]), &Default::default()).is_ok());
    }

    #[test]
    fn matches_exact_path_and_method() {
        let node = webhook_node("my-hook", "POST");
        assert!(webhook_node_matches(&node, "my-hook", "POST").is_some());
        assert!(webhook_node_matches(&node, "other-hook", "POST").is_none());
        assert!(webhook_node_matches(&node, "my-hook", "GET").is_none());
    }

    #[test]
    fn any_method_matches_get_and_post() {
        let node = webhook_node("my-hook", "ANY");
        assert!(webhook_node_matches(&node, "my-hook", "GET").is_some());
        assert!(webhook_node_matches(&node, "my-hook", "POST").is_some());
    }

    #[test]
    fn method_match_is_case_insensitive() {
        let node = webhook_node("my-hook", "post");
        assert!(webhook_node_matches(&node, "my-hook", "POST").is_some());
    }

    #[test]
    fn non_webhook_node_type_never_matches() {
        let mut node = webhook_node("my-hook", "ANY");
        node.node_type = "core.manualTrigger".into();
        assert!(webhook_node_matches(&node, "my-hook", "GET").is_none());
    }

    #[test]
    fn build_trigger_item_parses_json_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, br#"{"hello":"world"}"#);
        assert_eq!(item.json["body"], serde_json::json!({"hello": "world"}));
    }

    #[test]
    fn build_trigger_item_falls_back_to_raw_string_for_non_json_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, b"not json");
        assert_eq!(item.json["body"], serde_json::json!("not json"));
    }

    #[test]
    fn build_trigger_item_uses_null_body_for_empty_body() {
        let headers = HeaderMap::new();
        let query = HashMap::new();
        let item = build_trigger_item(&headers, &query, b"");
        assert_eq!(item.json["body"], serde_json::Value::Null);
    }
}
