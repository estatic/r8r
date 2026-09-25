//! Library tools for `ai.agent` (spec B1 §3): validation shared by the
//! create and update endpoints.

use crate::domain::Tool;

/// Node types a library tool may call.
pub const ALLOWED_TOOL_NODE_TYPES: [&str; 3] = ["core.httpRequest", "telegram.sendMessage", "core.code"];

const ARGUMENT_TYPES: [&str; 4] = ["string", "number", "integer", "boolean"];

pub fn validate_tool(tool: &Tool) -> Result<(), String> {
    let name_ok = !tool.name.is_empty()
        && tool.name.len() <= 64
        && tool.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !name_ok {
        return Err("name must be 1-64 characters of letters, digits, '_' or '-'".into());
    }
    if !ALLOWED_TOOL_NODE_TYPES.contains(&tool.node_type.as_str()) {
        return Err(format!("node_type must be one of {}", ALLOWED_TOOL_NODE_TYPES.join(", ")));
    }
    let schema = &tool.argument_schema;
    if schema.get("type").and_then(|v| v.as_str()) != Some("object") {
        return Err("argument_schema.type must be \"object\"".into());
    }
    let empty = serde_json::Map::new();
    let properties = match schema.get("properties") {
        None => &empty,
        Some(p) => p.as_object().ok_or("argument_schema.properties must be an object")?,
    };
    for (key, prop) in properties {
        let ty = prop.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if !ARGUMENT_TYPES.contains(&ty) {
            return Err(format!("argument \"{key}\" type must be one of {}", ARGUMENT_TYPES.join(", ")));
        }
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or("argument_schema.required must be an array")?;
        for r in required {
            let r = r.as_str().ok_or("argument_schema.required entries must be strings")?;
            if !properties.contains_key(r) {
                return Err(format!("required argument \"{r}\" is not declared in properties"));
            }
        }
    }
    if !tool.parameters.is_object() {
        return Err("parameters must be a JSON object".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Tool;

    fn tool() -> Tool {
        Tool {
            id: uuid::Uuid::new_v4(),
            name: "get_weather".into(),
            description: "Weather for a city".into(),
            node_type: "core.httpRequest".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
            parameters: serde_json::json!({"method": "GET", "url": "https://x/?q={{ $args.city }}"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn accepts_a_well_formed_tool() {
        assert_eq!(validate_tool(&tool()), Ok(()));
    }

    #[test]
    fn rejects_bad_names() {
        for bad in ["", "has space", "dots.not.ok", &"x".repeat(65)] {
            let mut t = tool();
            t.name = bad.to_string();
            assert!(validate_tool(&t).unwrap_err().contains("name"), "{bad:?}");
        }
    }

    #[test]
    fn rejects_disallowed_node_types() {
        for bad in ["ai.agent", "core.set", "telegram.trigger"] {
            let mut t = tool();
            t.node_type = bad.into();
            assert!(validate_tool(&t).unwrap_err().contains("node_type"), "{bad}");
        }
    }

    #[test]
    fn rejects_malformed_schemas_and_parameters() {
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "array"});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "object", "properties": {"x": {"type": "object"}}});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "object", "properties": {}, "required": ["ghost"]});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.parameters = serde_json::json!("not an object");
        assert!(validate_tool(&t).is_err());
    }
}