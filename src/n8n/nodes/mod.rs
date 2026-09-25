//! Native node implementations (spec §6.6), keyed by n8n type name.

mod code;
mod conditions;
mod core;
mod http;
mod merge;
mod routing;
mod set;
mod transform;
mod utility;

pub use conditions::evaluate_conditions;
pub use http::check_ssrf;

use super::node::NodeType;
use serde_json::{Map, Value};
use std::collections::HashMap;

pub struct Registry {
    types: HashMap<&'static str, Box<dyn NodeType>>,
}

impl Registry {
    pub fn get(&self, type_name: &str) -> Option<&dyn NodeType> {
        self.types.get(type_name).map(|b| b.as_ref())
    }

    pub fn type_names(&self) -> Vec<&'static str> {
        let mut v: Vec<_> = self.types.keys().copied().collect();
        v.sort_unstable();
        v
    }

    fn add(&mut self, node: Box<dyn NodeType>) {
        self.types.insert(node.type_name(), node);
    }
}

impl Default for Registry {
    fn default() -> Self {
        let mut r = Registry { types: HashMap::new() };
        for n in core::all() {
            r.add(n);
        }
        r.add(Box::new(set::Set));
        r.add(Box::new(routing::If));
        r.add(Box::new(routing::Filter));
        r.add(Box::new(routing::Switch));
        r.add(Box::new(merge::Merge));
        r.add(Box::new(code::Code));
        r.add(Box::new(http::HttpRequest));
        for n in transform::all() {
            r.add(n);
        }
        for n in utility::all() {
            r.add(n);
        }
        r
    }
}

// ---- helpers shared by nodes ----------------------------------------------

/// Reads a dotted path (`a.b.0.c`, `a.b[0].c`) from a JSON value.
pub fn get_path<'v>(value: &'v Value, path: &str) -> Option<&'v Value> {
    let mut v = value;
    for part in split_path(path) {
        v = match v {
            Value::Object(o) => o.get(&part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Writes a dotted path, creating objects on the way.
pub fn set_path(target: &mut Map<String, Value>, path: &str, value: Value) {
    let parts = split_path(path);
    let Some((last, init)) = parts.split_last() else { return };
    let mut cur = target;
    for part in init {
        let entry = cur.entry(part.clone()).or_insert_with(|| Value::Object(Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        cur = entry.as_object_mut().unwrap();
    }
    cur.insert(last.clone(), value);
}

fn split_path(path: &str) -> Vec<String> {
    path.replace('[', ".").replace(']', "").split('.').filter(|p| !p.is_empty()).map(String::from).collect()
}

/// A comma-separated field list parameter (`"a, b"`), or an array.
pub fn field_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect(),
        Value::Array(a) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => vec![],
    }
}
