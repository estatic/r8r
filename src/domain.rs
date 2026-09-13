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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub from_node: String,
    pub from_output: usize,
    pub to_node: String,
    pub to_input: usize,
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

#[cfg(test)]
mod tests {
    use super::*;

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
            }],
            connections: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&wf).unwrap();
        let parsed: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(wf, parsed);
    }
}
