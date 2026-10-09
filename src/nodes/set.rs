use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

pub struct SetNode;

#[async_trait]
impl Node for SetNode {
    fn type_name(&self) -> &'static str {
        "core.set"
    }
    fn runs_per_item(&self) -> bool {
        true
    }
    fn display_name(&self) -> &'static str {
        "Set"
    }
    fn description(&self) -> &'static str {
        "Adds or overwrites fields on each item."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn icon(&self) -> &'static str {
        "📝"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let p = &ctx.parameters;
        let fail = |m: String| NodeError::ExecutionFailed(format!("core.set: {m}"));
        // The fields to set: the form's `fields`, or in JSON mode `json_output`.
        let fields = match p.get("mode").and_then(Value::as_str).unwrap_or("manual") {
            "manual" => p.get("fields").cloned().unwrap_or(json!({})),
            "json" => match p.get("json_output") {
                Some(Value::String(text)) => serde_json::from_str(text).map_err(|e| fail(format!("the JSON isn't valid: {e}")))?,
                Some(other) => other.clone(),
                None => json!({}),
            },
            other => return Err(fail(format!("unknown mode \"{other}\" (expected \"manual\" or \"json\")"))),
        };
        let fields = fields.as_object().cloned().ok_or_else(|| fail(format!("the fields must be an object, got {fields}")))?;
        let dot_notation = p.get("dot_notation").and_then(Value::as_bool).unwrap_or(true);
        let listed = |key: &str| -> Vec<String> {
            match p.get(key) {
                Some(Value::String(s)) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect(),
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                _ => vec![],
            }
        };
        let include = p.get("include").and_then(Value::as_str).unwrap_or("all");
        let named = listed("include_fields");

        let base_items = if ctx.input_items.is_empty() {
            vec![Item { json: json!({}), binary: json!({}) }]
        } else {
            ctx.input_items.clone()
        };

        let items = base_items
            .into_iter()
            .map(|item| {
                let input = item
                    .json
                    .as_object()
                    .ok_or_else(|| NodeError::ExecutionFailed(format!("core.set expects object items, got {}", item.json)))?;
                // Which of the input's fields the item keeps, before the new ones go on top.
                let mut out: Map<String, Value> = match include {
                    "all" => input.clone(),
                    "none" => Map::new(),
                    "selected" => input.iter().filter(|(k, _)| named.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect(),
                    "except" => input.iter().filter(|(k, _)| !named.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect(),
                    other => return Err(fail(format!("unknown include \"{other}\" (expected all, none, selected or except)"))),
                };
                for (name, value) in &fields {
                    if dot_notation && name.split('.').count() > MAX_DEPTH {
                        return Err(fail(format!("the field name \"{}…\" nests deeper than {MAX_DEPTH} levels", name.chars().take(40).collect::<String>())));
                    }
                    if dot_notation && name.contains('.') {
                        set_path(&mut out, name, value.clone());
                    } else {
                        out.insert(name.clone(), value.clone());
                    }
                }
                Ok(Item { json: Value::Object(out), binary: item.binary })
            })
            .collect::<Result<Vec<_>, NodeError>>()?;

        Ok(vec![items])
    }
}

/// Deepest `a.b.c` a field name may reach: far deeper data overflows the
/// stack when it is stored or sent on.
const MAX_DEPTH: usize = 64;

/// `a.b.c` = value: creates (or replaces non-object) parents on the way.
fn set_path(obj: &mut Map<String, Value>, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut current = obj;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            current.insert(part.to_string(), value);
            return;
        }
        let next = current.entry(part.to_string()).or_insert_with(|| json!({}));
        if !next.is_object() {
            *next = json!({});
        }
        current = next.as_object_mut().expect("just made an object");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn set(params: Value, input: Value) -> Result<Value, NodeError> {
        let ctx = NodeExecutionContext { parameters: params, input_items: vec![Item { json: input, binary: json!({}) }], ..Default::default() };
        Ok(SetNode.execute(&ctx).await?[0][0].json.clone())
    }

    #[tokio::test]
    async fn chooses_which_input_fields_to_keep() {
        let input = json!({"a": 1, "b": 2, "c": 3});
        let fields = json!({"n": 9});
        assert_eq!(set(json!({"fields": fields}), input.clone()).await.unwrap(), json!({"a": 1, "b": 2, "c": 3, "n": 9}));
        assert_eq!(set(json!({"fields": fields, "include": "none"}), input.clone()).await.unwrap(), json!({"n": 9}));
        assert_eq!(set(json!({"fields": fields, "include": "selected", "include_fields": "a, c"}), input.clone()).await.unwrap(), json!({"a": 1, "c": 3, "n": 9}));
        assert_eq!(set(json!({"fields": fields, "include": "except", "include_fields": ["a"]}), input).await.unwrap(), json!({"b": 2, "c": 3, "n": 9}));
    }

    #[tokio::test]
    async fn dot_notation_sets_nested_fields_unless_turned_off() {
        let input = json!({"user": {"name": "Ada"}, "x": 1});
        assert_eq!(set(json!({"fields": {"user.age": 36}}), input.clone()).await.unwrap(), json!({"user": {"name": "Ada", "age": 36}, "x": 1}));
        assert_eq!(set(json!({"fields": {"x.y": true}}), input.clone()).await.unwrap()["x"], json!({"y": true}));
        assert_eq!(set(json!({"fields": {"user.age": 36}, "dot_notation": false}), input).await.unwrap()["user.age"], json!(36));
    }

    #[tokio::test]
    async fn a_field_name_nesting_too_deep_is_refused() {
        let deep = vec!["a"; 10_000].join(".");
        let err = set(json!({"fields": {deep: 1}}), json!({})).await.unwrap_err();
        assert!(err.to_string().contains("deeper than 64 levels"), "{err}");
    }

    #[tokio::test]
    async fn json_mode_sets_the_fields_of_an_object() {
        let out = set(json!({"mode": "json", "json_output": "{\"id\": 7, \"tags\": [\"a\"]}", "include": "none"}), json!({"old": 1})).await.unwrap();
        assert_eq!(out, json!({"id": 7, "tags": ["a"]}));
        assert!(set(json!({"mode": "json", "json_output": "{oops"}), json!({})).await.unwrap_err().to_string().contains("isn't valid"));
        assert!(set(json!({"mode": "json", "json_output": "[1]"}), json!({})).await.unwrap_err().to_string().contains("must be an object"));
    }

    #[tokio::test]
    async fn merges_static_fields_into_each_input_item() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![Item { json: serde_json::json!({"existing": true}), binary: serde_json::json!({}) }],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"existing": true, "greeting": "hi"}));
    }

    #[tokio::test]
    async fn with_no_input_items_produces_one_item_from_fields_only() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 1);
        assert_eq!(result[0][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[tokio::test]
    async fn non_object_item_json_returns_error_instead_of_panicking() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![Item { json: serde_json::json!([1, 2, 3]), binary: serde_json::json!({}) }],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
