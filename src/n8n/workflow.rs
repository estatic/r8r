//! n8n workflow JSON (spec §2.3, §6.1). The raw JSON is kept as-is
//! (key order included, via serde_json's `preserve_order`) so import →
//! export is lossless; typed views are derived from it.

use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnError {
    StopWorkflow,
    ContinueRegularOutput,
    ContinueErrorOutput,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub node_type: String,
    pub type_version: f64,
    pub parameters: Value,
    pub disabled: bool,
    pub position: (f64, f64),
    pub retry_on_fail: bool,
    pub max_tries: u32,
    pub wait_between_tries: u64,
    pub always_output_data: bool,
    pub execute_once: bool,
    pub on_error: OnError,
    pub credentials: Map<String, Value>,
    pub webhook_id: Option<String>,
    pub raw: Value,
}

impl Node {
    pub fn from_json(raw: &Value) -> anyhow::Result<Self> {
        let name = raw["name"].as_str().ok_or_else(|| anyhow::anyhow!("a node has no name"))?.to_string();
        let node_type = raw["type"].as_str().ok_or_else(|| anyhow::anyhow!("node \"{name}\" has no type"))?.to_string();
        let position = raw["position"]
            .as_array()
            .map(|p| (p.first().and_then(Value::as_f64).unwrap_or(0.0), p.get(1).and_then(Value::as_f64).unwrap_or(0.0)))
            .unwrap_or((0.0, 0.0));
        let legacy_continue = raw["continueOnFail"].as_bool().unwrap_or(false);
        let on_error = match raw["onError"].as_str() {
            Some("continueRegularOutput") => OnError::ContinueRegularOutput,
            Some("continueErrorOutput") => OnError::ContinueErrorOutput,
            Some("stopWorkflow") => OnError::StopWorkflow,
            _ if legacy_continue => OnError::ContinueRegularOutput,
            _ => OnError::StopWorkflow,
        };
        Ok(Self {
            name,
            node_type,
            type_version: raw["typeVersion"].as_f64().unwrap_or(1.0),
            parameters: raw.get("parameters").cloned().unwrap_or_else(|| Value::Object(Map::new())),
            disabled: raw["disabled"].as_bool().unwrap_or(false),
            position,
            retry_on_fail: raw["retryOnFail"].as_bool().unwrap_or(false),
            max_tries: raw["maxTries"].as_u64().map(|v| v as u32).unwrap_or(3).max(1),
            wait_between_tries: raw["waitBetweenTries"].as_u64().unwrap_or(1000),
            always_output_data: raw["alwaysOutputData"].as_bool().unwrap_or(false),
            execute_once: raw["executeOnce"].as_bool().unwrap_or(false),
            on_error,
            credentials: raw["credentials"].as_object().cloned().unwrap_or_default(),
            webhook_id: raw["webhookId"].as_str().map(String::from),
            raw: raw.clone(),
        })
    }

    /// The part of the type after the package, e.g. `set` for
    /// `n8n-nodes-base.set`.
    pub fn short_type(&self) -> &str {
        self.node_type.rsplit('.').next().unwrap_or(&self.node_type)
    }
}

/// One edge: `node`'s input `index` of connection type `kind`.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub node: String,
    pub kind: String,
    pub index: usize,
}

/// A parent of a node on one of its inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct Parent {
    pub node: String,
    pub output: usize,
    pub input: usize,
}

#[derive(Debug, Clone)]
pub struct Workflow {
    pub raw: Map<String, Value>,
    pub id: Option<String>,
    pub name: String,
    pub active: bool,
    pub nodes: Vec<Node>,
    /// source node -> connection type -> output index -> targets
    pub connections: HashMap<String, HashMap<String, Vec<Vec<Target>>>>,
    pub settings: Map<String, Value>,
    pub pin_data: Map<String, Value>,
    pub static_data: Value,
}

impl Workflow {
    pub fn from_json(value: &Value) -> anyhow::Result<Self> {
        let raw = value.as_object().cloned().ok_or_else(|| anyhow::anyhow!("a workflow must be a JSON object"))?;
        let nodes = raw
            .get("nodes")
            .and_then(Value::as_array)
            .map(|nodes| nodes.iter().map(Node::from_json).collect::<anyhow::Result<Vec<_>>>())
            .transpose()?
            .unwrap_or_default();
        let mut connections: HashMap<String, HashMap<String, Vec<Vec<Target>>>> = HashMap::new();
        if let Some(conns) = raw.get("connections").and_then(Value::as_object) {
            for (source, by_type) in conns {
                let Some(by_type) = by_type.as_object() else { continue };
                for (kind, outputs) in by_type {
                    let outputs: Vec<Vec<Target>> = outputs
                        .as_array()
                        .map(|outs| {
                            outs.iter()
                                .map(|targets| {
                                    targets
                                        .as_array()
                                        .map(|ts| {
                                            ts.iter()
                                                .filter_map(|t| {
                                                    Some(Target {
                                                        node: t["node"].as_str()?.to_string(),
                                                        kind: t["type"].as_str().unwrap_or(kind).to_string(),
                                                        index: t["index"].as_u64().unwrap_or(0) as usize,
                                                    })
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default()
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    connections.entry(source.clone()).or_default().insert(kind.clone(), outputs);
                }
            }
        }
        Ok(Self {
            id: raw.get("id").and_then(|v| v.as_str().map(String::from).or_else(|| v.as_i64().map(|i| i.to_string()))),
            name: raw.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            active: raw.get("active").and_then(Value::as_bool).unwrap_or(false),
            settings: raw.get("settings").and_then(Value::as_object).cloned().unwrap_or_default(),
            pin_data: raw.get("pinData").and_then(Value::as_object).cloned().unwrap_or_default(),
            static_data: raw.get("staticData").cloned().unwrap_or(Value::Null),
            nodes,
            connections,
            raw,
        })
    }

    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }

    /// Targets of `node`'s `output` on the `main` connection.
    pub fn children(&self, node: &str, output: usize) -> Vec<Target> {
        self.connections
            .get(node)
            .and_then(|t| t.get("main"))
            .and_then(|outs| outs.get(output))
            .cloned()
            .unwrap_or_default()
    }

    /// Number of main outputs that have connections.
    pub fn connected_outputs(&self, node: &str) -> usize {
        self.connections.get(node).and_then(|t| t.get("main")).map(Vec::len).unwrap_or(0)
    }

    /// Main-connection parents of `node`.
    pub fn parents(&self, node: &str) -> Vec<Parent> {
        self.parents_of_kind(node, "main")
    }

    pub fn parents_of_kind(&self, node: &str, kind: &str) -> Vec<Parent> {
        let mut out = Vec::new();
        for n in &self.nodes {
            if let Some(outs) = self.connections.get(&n.name).and_then(|t| t.get(kind)) {
                for (output, targets) in outs.iter().enumerate() {
                    for t in targets {
                        if t.node == node {
                            out.push(Parent { node: n.name.clone(), output, input: t.index });
                        }
                    }
                }
            }
        }
        out
    }

    /// Sub-nodes connected to `node` over a non-main connection type
    /// (`ai_languageModel`, `ai_tool`, ...).
    pub fn sub_nodes(&self, node: &str, kind: &str) -> Vec<String> {
        self.parents_of_kind(node, kind).into_iter().map(|p| p.node).collect()
    }

    /// Number of distinct main inputs that have a parent.
    pub fn connected_inputs(&self, node: &str) -> usize {
        self.parents(node).iter().map(|p| p.input + 1).max().unwrap_or(0)
    }

    pub fn setting_str(&self, key: &str) -> Option<&str> {
        self.settings.get(key).and_then(Value::as_str)
    }

    /// `v1` unless the workflow says otherwise. A workflow saved without the
    /// setting predates it and runs in the legacy `v0` order, as in n8n.
    pub fn execution_order_v1(&self) -> bool {
        self.setting_str("executionOrder") == Some("v1")
    }

    /// The full JSON as stored and exported.
    pub fn to_json(&self) -> Value {
        Value::Object(self.raw.clone())
    }
}
