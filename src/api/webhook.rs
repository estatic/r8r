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

    if !webhook_node_matches(start_node, &path, method.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let trigger_item = build_trigger_item(&headers, &query, &body);

    let execution = match crate::execution_runner::run_and_track_execution(
        &state.storage,
        &state.execution_events,
        &state.registry,
        &workflow,
        ExecutionMode::Webhook,
        Some(vec![trigger_item]),
        &std::collections::HashMap::new(),
    )
    .await
    {
        Ok(execution) => execution,
        Err(e) => {
            tracing::error!(error = %e, "webhook: failed to persist new execution");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let response_status = if execution.status == ExecutionStatus::Error {
        StatusCode::INTERNAL_SERVER_ERROR
    } else {
        StatusCode::OK
    };

    (response_status, Json(execution)).into_response()
}

fn webhook_node_matches(node: &crate::domain::NodeInstance, path: &str, method: &str) -> bool {
    if node.node_type != "core.webhook" {
        return false;
    }
    let node_path = node.parameters.get("path").and_then(|v| v.as_str()).unwrap_or("");
    if node_path != path {
        return false;
    }
    let node_method = node.parameters.get("method").and_then(|v| v.as_str()).unwrap_or("ANY");
    node_method.eq_ignore_ascii_case("ANY") || node_method.eq_ignore_ascii_case(method)
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
        }
    }

    #[test]
    fn matches_exact_path_and_method() {
        let node = webhook_node("my-hook", "POST");
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
        assert!(!webhook_node_matches(&node, "other-hook", "POST"));
        assert!(!webhook_node_matches(&node, "my-hook", "GET"));
    }

    #[test]
    fn any_method_matches_get_and_post() {
        let node = webhook_node("my-hook", "ANY");
        assert!(webhook_node_matches(&node, "my-hook", "GET"));
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
    }

    #[test]
    fn method_match_is_case_insensitive() {
        let node = webhook_node("my-hook", "post");
        assert!(webhook_node_matches(&node, "my-hook", "POST"));
    }

    #[test]
    fn non_webhook_node_type_never_matches() {
        let mut node = webhook_node("my-hook", "ANY");
        node.node_type = "core.manualTrigger".into();
        assert!(!webhook_node_matches(&node, "my-hook", "GET"));
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
