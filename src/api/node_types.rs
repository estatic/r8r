use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

#[derive(serde::Serialize)]
pub struct NodeTypeMeta {
    pub type_name: String,
    pub display_name: String,
    pub icon: String,
    pub category: crate::node::NodeCategory,
    pub description: String,
    pub credential_types: Vec<String>,
    pub output_ports: Vec<String>,
}

fn meta_for(name: &str, node: &dyn crate::node::Node) -> NodeTypeMeta {
    NodeTypeMeta {
        type_name: name.to_string(),
        display_name: node.display_name().to_string(),
        icon: node.icon().to_string(),
        category: node.category(),
        description: node.description().to_string(),
        credential_types: node.credential_types().iter().map(|s| s.to_string()).collect(),
        output_ports: node.output_ports(&serde_json::json!({})),
    }
}

pub async fn list_node_types(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
) -> impl IntoResponse {
    let metas: Vec<NodeTypeMeta> = state
        .registry
        .type_names()
        .into_iter()
        .filter_map(|name| state.registry.get(name).map(|node| meta_for(name, node)))
        .collect();
    Json(metas).into_response()
}

#[derive(serde::Deserialize)]
pub struct OutputPortsRequest {
    #[serde(default)]
    pub parameters: serde_json::Value,
}

pub async fn output_ports_for_type(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
    Path(type_name): Path<String>,
    Json(payload): Json<OutputPortsRequest>,
) -> impl IntoResponse {
    match state.registry.get(&type_name) {
        Some(node) => Json(serde_json::json!({"output_ports": node.output_ports(&payload.parameters)})).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
