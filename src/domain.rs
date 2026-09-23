use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeInstance {
    pub id: String,
    pub node_type: String,
    pub position: (f64, f64),
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub settings: NodeSettings,
}

/// Per-node execution policy (Plan 8.5). Kept out of `parameters` so it is
/// never expression-resolved and can't collide with a node's own
/// parameters. Every field defaults, so workflows saved before this
/// existed load unchanged.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct NodeSettings {
    /// `None` = exactly one attempt.
    #[serde(default)]
    pub retry: Option<RetryPolicy>,
    /// Per-attempt timeout; `None` = no timeout.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// On final failure with no error connection, emit the error item on
    /// port 0 instead of failing the workflow.
    #[serde(default)]
    pub continue_on_fail: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    pub max_tries: u32,
    pub wait_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub from_node: String,
    pub from_output: usize,
    pub to_node: String,
    pub to_input: usize,
    #[serde(default)]
    pub error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub active: bool,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Item {
    pub json: serde_json::Value,
    #[serde(default)]
    pub binary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    Manual,
    Webhook,
    Schedule,
    Telegram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub status: ExecutionStatus,
    pub mode: ExecutionMode,
    pub node_outputs: HashMap<String, Vec<Item>>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserRole {
    Owner,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Credential {
    pub id: Uuid,
    pub name: String,
    pub credential_type: String,
    pub data: serde_json::Value,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialSummary {
    pub id: Uuid,
    pub name: String,
    pub credential_type: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Credential> for CredentialSummary {
    fn from(c: &Credential) -> Self {
        Self {
            id: c.id,
            name: c.name.clone(),
            credential_type: c.credential_type.clone(),
            owner_id: c.owner_id,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_instance_without_settings_deserializes_to_defaults() {
        let json = r#"{"id":"n1","node_type":"core.set","position":[0.0,0.0],"parameters":{}}"#;
        let node: NodeInstance = serde_json::from_str(json).unwrap();
        assert_eq!(node.settings, NodeSettings::default());
        assert!(node.settings.retry.is_none());
        assert!(node.settings.timeout_ms.is_none());
        assert!(!node.settings.continue_on_fail);
    }

    #[test]
    fn node_settings_round_trip_through_json() {
        let settings = NodeSettings {
            retry: Some(RetryPolicy { max_tries: 3, wait_ms: 500 }),
            timeout_ms: Some(1000),
            continue_on_fail: true,
        };
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json, serde_json::json!({"retry": {"max_tries": 3, "wait_ms": 500}, "timeout_ms": 1000, "continue_on_fail": true}));
        assert_eq!(serde_json::from_value::<NodeSettings>(json).unwrap(), settings);
    }

    #[test]
    fn workflow_round_trips_through_json() {
        let wf = Workflow {
            id: Uuid::new_v4(),
            name: "test".into(),
            active: false,
            nodes: vec![NodeInstance {
                id: "n1".into(),
                node_type: "core.manualTrigger".into(),
                position: (0.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
                settings: Default::default(),
            }],
            connections: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&wf).unwrap();
        let parsed: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(wf, parsed);
    }

    #[test]
    fn execution_mode_serializes_new_variants() {
        assert_eq!(serde_json::to_string(&ExecutionMode::Webhook).unwrap(), "\"Webhook\"");
        assert_eq!(serde_json::to_string(&ExecutionMode::Schedule).unwrap(), "\"Schedule\"");
        let parsed: ExecutionMode = serde_json::from_str("\"Webhook\"").unwrap();
        assert_eq!(parsed, ExecutionMode::Webhook);
    }

    #[test]
    fn credential_summary_never_serializes_a_data_field() {
        let cred = Credential {
            id: Uuid::new_v4(),
            name: "my-api".into(),
            credential_type: "bearer".into(),
            data: serde_json::json!({"token": "super-secret-value"}),
            owner_id: Uuid::new_v4(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let summary = CredentialSummary::from(&cred);
        let json = serde_json::to_value(&summary).unwrap();
        assert!(json.get("data").is_none());
        assert_eq!(json["name"], "my-api");
        // The actual secret value must never appear anywhere in the serialized summary.
        let serialized = serde_json::to_string(&summary).unwrap();
        assert!(!serialized.contains("super-secret-value"));
    }

    #[test]
    fn connection_without_an_error_field_deserializes_as_false() {
        let json = serde_json::json!({
            "from_node": "a",
            "from_output": 0,
            "to_node": "b",
            "to_input": 0
        });
        let conn: Connection = serde_json::from_value(json).unwrap();
        assert!(!conn.error);
    }
}
