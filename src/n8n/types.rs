//! n8n's execution data model: items, run data and the `IRun` result, in
//! the JSON shape n8n stores and the editor replays (spec §2.4).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// One unit of data flowing between nodes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub json: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<Map<String, Value>>,
    #[serde(rename = "pairedItem", default, skip_serializing_if = "Option::is_none")]
    pub paired_item: Option<Value>,
}

impl Item {
    pub fn new(json: Map<String, Value>) -> Self {
        Self { json, binary: None, paired_item: None }
    }

    /// Wraps any JSON value: objects become the item's json, anything else
    /// is stored under `data`.
    pub fn from_value(value: Value) -> Self {
        match value {
            Value::Object(map) => Self::new(map),
            other => {
                let mut map = Map::new();
                map.insert("data".into(), other);
                Self::new(map)
            }
        }
    }

    pub fn paired(mut self, index: usize) -> Self {
        self.paired_item = Some(json!({ "item": index }));
        self
    }

    pub fn paired_input(mut self, index: usize, input: usize) -> Self {
        self.paired_item = Some(if input == 0 { json!({ "item": index }) } else { json!({ "item": index, "input": input }) });
        self
    }

    pub fn json_value(&self) -> Value {
        Value::Object(self.json.clone())
    }
}

/// Where a node run's input came from, per input index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceData {
    #[serde(rename = "previousNode")]
    pub previous_node: String,
    #[serde(rename = "previousNodeOutput", default)]
    pub previous_node_output: usize,
    #[serde(rename = "previousNodeRun", default)]
    pub previous_node_run: usize,
}

/// Output of a node: one list of items per output index.
pub type NodeOutput = Vec<Vec<Item>>;

/// One run of one node (n8n's `ITaskData`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskData {
    #[serde(rename = "startTime")]
    pub start_time: i64,
    #[serde(rename = "executionIndex")]
    pub execution_index: usize,
    pub source: Vec<Option<SourceData>>,
    #[serde(default)]
    pub hints: Vec<Value>,
    #[serde(rename = "executionTime")]
    pub execution_time: i64,
    #[serde(rename = "executionStatus")]
    pub execution_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

impl TaskData {
    /// Items on `output` of the `main` connection.
    pub fn main_output(&self, output: usize) -> Vec<Item> {
        self.data
            .as_ref()
            .and_then(|d| d.get("main"))
            .and_then(|m| m.get(output))
            .and_then(|items| serde_json::from_value(items.clone()).ok())
            .unwrap_or_default()
    }
}

/// Run data keyed by node name, in execution order of first run.
pub type RunData = Map<String, Value>;

pub fn main_data(outputs: &NodeOutput) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("main".into(), serde_json::to_value(outputs).unwrap());
    data
}

/// Execution status values n8n uses.
pub mod status {
    pub const SUCCESS: &str = "success";
    pub const ERROR: &str = "error";
    pub const RUNNING: &str = "running";
    pub const WAITING: &str = "waiting";
    pub const CANCELED: &str = "canceled";
    pub const CRASHED: &str = "crashed";
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
