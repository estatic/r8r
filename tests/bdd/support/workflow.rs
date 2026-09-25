//! Builds n8n workflow JSON (the frozen contract, spec §2.3 / §6.1) from
//! Gherkin tables and connection lines.

use serde_json::{json, Map, Value};

/// Default `typeVersion` per node type, used when a table omits it. These
/// are the n8n 2.x versions the scenarios' parameter shapes are written for;
/// check them against the pinned n8n version in Phase 0.
pub fn default_type_version(node_type: &str) -> Value {
    let short = node_type.rsplit('.').next().unwrap_or(node_type);
    let v = match short {
        "set" => json!(3.4),
        "if" => json!(2.2),
        "filter" => json!(2.2),
        "switch" => json!(3.2),
        "merge" => json!(3.1),
        "code" => json!(2),
        "httpRequest" => json!(4.2),
        "webhook" => json!(2),
        "respondToWebhook" => json!(1.1),
        "wait" => json!(1.1),
        "splitInBatches" => json!(3),
        "removeDuplicates" => json!(2),
        "summarize" => json!(1.1),
        "dateTime" => json!(2),
        "executeWorkflow" => json!(1.2),
        "executeWorkflowTrigger" => json!(1.1),
        "scheduleTrigger" => json!(1.2),
        "formTrigger" => json!(2.2),
        "agent" => json!(2),
        "lmChatOpenAi" => json!(1.2),
        "chainLlm" => json!(1.5),
        "memoryBufferWindow" => json!(1.3),
        _ => json!(1),
    };
    v
}

/// `set` -> `n8n-nodes-base.set`, `lc.agent` ->
/// `@n8n/n8n-nodes-langchain.agent`; anything with a dot is left alone.
pub fn expand_type(short: &str) -> String {
    if let Some(rest) = short.strip_prefix("lc.") {
        format!("@n8n/n8n-nodes-langchain.{rest}")
    } else if short.contains('.') {
        short.to_string()
    } else {
        format!("n8n-nodes-base.{short}")
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowSpec {
    pub name: String,
    pub nodes: Vec<Value>,
    pub connections: Map<String, Value>,
    pub settings: Map<String, Value>,
    pub pin_data: Map<String, Value>,
    /// Replaces everything above when a scenario gives the raw JSON.
    pub raw: Option<String>,
}

impl WorkflowSpec {
    pub fn new(name: &str) -> Self {
        let mut settings = Map::new();
        settings.insert("executionOrder".into(), json!("v1"));
        Self { name: name.into(), nodes: vec![], connections: Map::new(), settings, pin_data: Map::new(), raw: None }
    }

    /// Adds nodes from a table whose header row names the columns. Known
    /// columns: `name`, `type`, `typeVersion`, `parameters`, `position`
    /// (`x,y`). Any other column becomes a node property; its cell is parsed
    /// as JSON when possible (`true`, `3`) and kept as a string otherwise
    /// (`continueErrorOutput`). Empty cells are skipped.
    pub fn add_nodes_from_table(&mut self, rows: &[Vec<String>]) {
        let header = &rows[0];
        for (row_index, row) in rows[1..].iter().enumerate() {
            let mut node = Map::new();
            let mut position = json!([250 * (self.nodes.len() as i64 + 1), 300]);
            let mut ty = String::new();
            let mut type_version = None;
            for (col, cell) in header.iter().zip(row) {
                let cell = cell.trim();
                if cell.is_empty() {
                    continue;
                }
                match col.as_str() {
                    "name" => {
                        node.insert("name".into(), json!(cell));
                    }
                    "type" => ty = expand_type(cell),
                    "typeVersion" => type_version = Some(super::json::parse_loose(cell)),
                    "parameters" => {
                        node.insert("parameters".into(), super::json::parse_strict(cell, "parameters cell"));
                    }
                    "position" => {
                        let xy: Vec<f64> = cell.split(',').map(|p| p.trim().parse().expect("position is x,y")).collect();
                        position = json!([xy[0], xy[1]]);
                    }
                    other => {
                        node.insert(other.into(), super::json::parse_loose(cell));
                    }
                }
            }
            assert!(node.contains_key("name"), "node row {} has no name", row_index + 1);
            assert!(!ty.is_empty(), "node row {} has no type", row_index + 1);
            node.insert("id".into(), json!(uuid::Uuid::new_v4().to_string()));
            node.insert("type".into(), json!(ty));
            node.insert("typeVersion".into(), type_version.unwrap_or_else(|| default_type_version(&ty)));
            node.insert("position".into(), position);
            node.entry("parameters").or_insert_with(|| json!({}));
            if ty.ends_with(".webhook") || ty.ends_with(".formTrigger") || ty.ends_with(".wait") {
                node.entry("webhookId").or_insert_with(|| json!(uuid::Uuid::new_v4().to_string()));
            }
            self.nodes.push(Value::Object(node));
        }
    }

    pub fn node_mut(&mut self, name: &str) -> &mut Map<String, Value> {
        let known: Vec<String> = self.nodes.iter().filter_map(|n| n["name"].as_str().map(String::from)).collect();
        self.nodes
            .iter_mut()
            .find(|n| n["name"] == name)
            .and_then(Value::as_object_mut)
            .unwrap_or_else(|| panic!("no node named \"{name}\" in workflow; nodes: {known:?}"))
    }

    /// Parses one connection line:
    ///
    /// * `A -> B` — main output 0 of A to input 0 of B
    /// * `A:1 -> B:0` — explicit output / input indices
    /// * `Model -[ai_languageModel]-> Agent` — non-main connection type
    /// * `A -> B -> C` — a chain (an index on a middle node applies to
    ///   both its input and its output, so prefer separate lines then)
    pub fn add_connection_line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return;
        }
        // Normalise typed arrows to `->[type]` tokens before splitting.
        let mut hops: Vec<(String, String)> = Vec::new(); // (endpoint, connection type into it)
        let mut rest = line;
        let mut pending_type = "main".to_string();
        loop {
            let typed = rest.find("-[");
            let plain = rest.find("->");
            let (idx, arrow_len, conn_type) = match (typed, plain) {
                (Some(t), Some(p)) if t < p => {
                    let close = rest[t..].find("]->").map(|c| c + t).expect("typed arrow must be -[type]->");
                    (t, close + 3 - t, rest[t + 2..close].to_string())
                }
                (_, Some(p)) => (p, 2, "main".to_string()),
                (Some(t), None) => {
                    let close = rest[t..].find("]->").map(|c| c + t).expect("typed arrow must be -[type]->");
                    (t, close + 3 - t, rest[t + 2..close].to_string())
                }
                (None, None) => {
                    hops.push((rest.trim().to_string(), pending_type.clone()));
                    break;
                }
            };
            hops.push((rest[..idx].trim().to_string(), pending_type.clone()));
            pending_type = conn_type;
            rest = &rest[idx + arrow_len..];
        }
        assert!(hops.len() >= 2, "connection line needs at least one arrow: {line}");
        for pair in hops.windows(2) {
            let (from, _) = &pair[0];
            let (to, conn_type) = &pair[1];
            let (from_name, output) = split_index(from);
            let (to_name, input) = split_index(to);
            self.connect(&from_name, output, &to_name, input, conn_type);
        }
    }

    pub fn connect(&mut self, from: &str, output: usize, to: &str, input: usize, conn_type: &str) {
        let by_type = self
            .connections
            .entry(from.to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        let outputs = by_type.entry(conn_type.to_string()).or_insert_with(|| json!([])).as_array_mut().unwrap();
        while outputs.len() <= output {
            outputs.push(json!([]));
        }
        outputs[output].as_array_mut().unwrap().push(json!({"node": to, "type": conn_type, "index": input}));
    }

    pub fn pin(&mut self, node: &str, items: &Value) {
        let items = items.as_array().expect("pinned items must be a JSON array");
        let wrapped: Vec<Value> = items.iter().map(|i| json!({ "json": i })).collect();
        self.pin_data.insert(node.to_string(), Value::Array(wrapped));
    }

    /// The first node in table order: the trigger by convention.
    pub fn first_node_name(&self) -> String {
        self.nodes.first().and_then(|n| n["name"].as_str()).expect("workflow has no nodes").to_string()
    }

    /// Full n8n workflow JSON, as `r8r execute --file` and `import:workflow`
    /// take it.
    pub fn to_json(&self) -> Value {
        if let Some(raw) = &self.raw {
            return super::json::parse_strict(raw, "raw workflow");
        }
        json!({
            "name": self.name,
            "nodes": self.nodes,
            "connections": self.connections,
            "settings": self.settings,
            "pinData": self.pin_data,
            "active": false,
        })
    }

    /// The subset `POST /api/v1/workflows` accepts (n8n's public API
    /// rejects read-only and unknown properties).
    pub fn to_public_api_json(&self) -> Value {
        let full = self.to_json();
        json!({
            "name": full["name"],
            "nodes": full["nodes"],
            "connections": full["connections"],
            "settings": full.get("settings").cloned().unwrap_or_else(|| json!({})),
        })
    }
}

/// Prepares workflow JSON for `execute --id`, which (like `n8n execute`)
/// runs in "cli" mode where pin data is ignored and a trigger is required.
/// Items pinned on a manual trigger are fed instead by renaming the trigger
/// to "<name> (trigger)" and inserting a Code node under the original name
/// that returns those items. Downstream node names, `$('<name>')`
/// references and paired items are unchanged. Also gives the workflow an
/// id if it has none.
pub fn prepare_for_cli(workflow: &mut Value) -> String {
    if workflow.get("id").and_then(Value::as_str).is_none() {
        workflow["id"] = json!(format!("bdd{}", &uuid::Uuid::new_v4().simple().to_string()[..13]));
    }
    let id = workflow["id"].as_str().unwrap().to_string();
    let Some(nodes) = workflow["nodes"].as_array().cloned() else { return id };
    let Some(first) = nodes.first() else { return id };
    let name = first["name"].as_str().unwrap_or_default().to_string();
    let is_manual = first["type"] == "n8n-nodes-base.manualTrigger";
    let pinned = workflow.pointer(&format!("/pinData/{}", name.replace('~', "~0").replace('/', "~1"))).cloned();
    let (true, Some(Value::Array(items))) = (is_manual, pinned) else { return id };

    let trigger_name = format!("{name} (trigger)");
    let jsons: Vec<Value> = items.iter().map(|i| json!({ "json": i.get("json").cloned().unwrap_or(json!({})) })).collect();
    let position = first["position"].clone();
    let mut new_nodes = nodes.clone();
    new_nodes[0]["name"] = json!(trigger_name);
    new_nodes[0]["position"] = json!([position[0].as_f64().unwrap_or(0.0) - 200.0, position[1].as_f64().unwrap_or(0.0)]);
    new_nodes.insert(
        1,
        json!({
            "id": uuid::Uuid::new_v4().to_string(),
            "name": name,
            "type": "n8n-nodes-base.code",
            "typeVersion": 2,
            "position": position,
            "parameters": {"mode": "runOnceForAllItems", "language": "javaScript",
                           "jsCode": format!("return {};", serde_json::to_string(&jsons).unwrap())}
        }),
    );
    workflow["nodes"] = Value::Array(new_nodes);
    workflow["connections"][trigger_name.as_str()] = json!({"main": [[{"node": name, "type": "main", "index": 0}]]});
    workflow["pinData"].as_object_mut().map(|p| p.remove(&name));
    id
}

fn split_index(endpoint: &str) -> (String, usize) {
    match endpoint.rsplit_once(':') {
        Some((name, idx)) if idx.trim().parse::<usize>().is_ok() => (name.trim().to_string(), idx.trim().parse().unwrap()),
        _ => (endpoint.trim().to_string(), 0),
    }
}

/// Set (Edit Fields) v3.4 parameters that output `fields`, keeping each
/// value's native type. Built in raw JSON mode as one JS object literal,
/// because manual-mode assignments coerce every value to a declared type
/// (an expression typed "string" would turn `7` into `"7"`). Values that
/// are n8n expressions (`={{ x }}`, `=a {{ x }} b`) become JS expressions or
/// template literals; anything else is a JSON literal. Dotted names are
/// kept as flat keys (raw mode does not apply dot notation).
pub fn set_node_parameters(fields: &[(String, Value)], include_other_fields: bool) -> Value {
    let mut entries = Vec::new();
    for (name, value) in fields {
        let js = match value.as_str().and_then(|s| s.strip_prefix('=')) {
            Some(template) => expression_to_js(template),
            // Pretty-printed so nested objects never produce a literal `}}`,
            // which would end the surrounding {{ }} block.
            None => serde_json::to_string_pretty(value).unwrap(),
        };
        entries.push(format!("  {}: {}", serde_json::to_string(name).unwrap(), js));
    }
    let object = format!("({{\n{}\n}})", entries.join(",\n"));
    json!({
        "mode": "raw",
        "jsonOutput": format!("={{{{ {object} }}}}"),
        "includeOtherFields": include_other_fields,
        "options": {}
    })
}

/// `{{ x }}` -> `(x)`; `a {{ x }} b` -> `` `a ${(x)} b` ``.
fn expression_to_js(template: &str) -> String {
    let trimmed = template.trim();
    if let Some(inner) = trimmed.strip_prefix("{{").and_then(|t| t.strip_suffix("}}")) {
        if !inner.contains("{{") && !inner.contains("}}") {
            return format!("({})", inner.trim());
        }
    }
    let mut out = String::from("`");
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&escape_template(&rest[..start]));
        let end = rest[start..].find("}}").map(|e| e + start).expect("unclosed {{ in expression");
        out.push_str(&format!("${{({})}}", rest[start + 2..end].trim()));
        rest = &rest[end + 2..];
    }
    out.push_str(&escape_template(rest));
    out.push('`');
    out
}

fn escape_template(literal: &str) -> String {
    literal.replace('\\', "\\\\").replace('`', "\\`").replace("${", "\\${")
}
