use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeInstance {
    pub id: String,
    pub node_type: String,
    /// What the user calls the node, unique in its workflow: shown on the
    /// canvas and how expressions refer to it (`$("Send reply")`).
    /// Workflows saved before names existed have none until
    /// [`Workflow::name_nodes`] gives them one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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

impl Workflow {
    /// Gives every node a name no other node in the workflow has: an
    /// unnamed node (saved before names existed) gets `default_name` of its
    /// type, and a repeated name gets a number ("HTTP Request 2"), in node
    /// order. The editor names its new nodes the same way.
    pub fn name_nodes(&mut self, default_name: impl Fn(&str) -> String) {
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        for node in &mut self.nodes {
            let base = match node.name.as_deref().map(str::trim) {
                Some(name) if !name.is_empty() => name.to_string(),
                _ => default_name(&node.node_type),
            };
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name) {
                name = format!("{base} {n}");
                n += 1;
            }
            taken.insert(name.clone());
            node.name = Some(name);
        }
    }

    /// Node id -> name, for nodes that have one.
    pub fn node_names(&self) -> HashMap<String, String> {
        self.nodes
            .iter()
            .filter_map(|n| n.name.clone().map(|name| (n.id.clone(), name)))
            .collect()
    }
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
    /// Stopped by a user before it finished.
    Canceled,
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
    /// How each node that ran ended, and how many items left each output.
    #[serde(default)]
    pub node_runs: HashMap<String, NodeRun>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NodeRunStatus {
    #[default]
    Success,
    Error,
    Skipped,
}

/// One node's outcome in a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NodeRun {
    pub status: NodeRunStatus,
    /// Items sent per output, keyed like the editor's handles: "0", "1",
    /// ..., and "error" for the error output.
    pub counts: std::collections::BTreeMap<String, usize>,
    /// The items it received (all inputs, in order).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<Item>,
    /// The items it sent, per output (index = the output's handle).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Vec<Item>>,
    /// Items sent down its error output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_items: Vec<Item>,
    /// Why it failed, in full (a Code node's includes its stack trace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The node's settings and everything upstream of it when it ran; a
    /// later run may reuse this run's output while it is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// Taken from an earlier run instead of running again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reused: bool,
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

/// A reusable `ai.agent` tool (spec B1 §3). `parameters` are the called
/// node's fixed parameters and may contain `{{ $args.<name> }}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub node_type: String,
    pub argument_schema: serde_json::Value,
    pub parameters: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(id: &str, node_type: &str, name: Option<&str>) -> NodeInstance {
        NodeInstance {
            id: id.into(),
            node_type: node_type.into(),
            name: name.map(Into::into),
            position: (0.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
            settings: NodeSettings::default(),
        }
    }

    #[test]
    fn name_nodes_names_unnamed_nodes_after_their_type_and_keeps_names_unique() {
        let mut wf = Workflow {
            id: Uuid::nil(),
            name: "w".into(),
            active: false,
            nodes: vec![
                named("a", "http", None),
                named("b", "http", None),
                named("c", "set", Some("Reply")),
                named("d", "set", Some("Reply")),
                named("e", "set", Some("  ")),
            ],
            connections: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        wf.name_nodes(|t| if t == "http" { "HTTP Request".into() } else { "Set".into() });
        let names: Vec<_> = wf.nodes.iter().map(|n| n.name.clone().unwrap()).collect();
        assert_eq!(names, ["HTTP Request", "HTTP Request 2", "Reply", "Reply 2", "Set"]);
    }

    #[test]
    fn a_node_without_a_name_saves_without_one() {
        let json = serde_json::to_value(named("a", "set", None)).unwrap();
        assert!(json.get("name").is_none());
    }

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
                name: None,
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
