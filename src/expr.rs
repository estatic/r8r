//! Expression engine core: a QuickJS wrapper that evaluates JS snippets
//! against a small set of JSON-derived globals (`$json`, `$items`, `$node`,
//! `$now`, `$workflow`).
//!
//! This module is intentionally self-contained: it depends only on
//! `serde_json`, `thiserror`, `chrono`, and `rquickjs`, not on `domain.rs`
//! or `node.rs`. Later tasks wire `eval_js`/`EvalContext` into node
//! parameter resolution.

use std::collections::HashMap;

/// Inputs bound as globals when evaluating a script with [`eval_js`].
#[derive(Debug, Clone)]
pub struct EvalContext<'a> {
    /// Bound as `$json`: the current item's JSON data.
    pub json: serde_json::Value,
    /// Bound as `$items()`: all input items for the current node, each
    /// wrapped as `{ json: <item> }`.
    pub items: &'a [serde_json::Value],
    /// Bound as `$node`: a map of node name to `{ json: <output> }`.
    pub node_json: &'a HashMap<String, serde_json::Value>,
    /// Bound as `$workflow.name`.
    pub workflow_name: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum ExprError {
    #[error("expression runtime error: {0}")]
    Runtime(String),
    #[error("expression value conversion error: {0}")]
    Conversion(String),
}

/// Evaluates `script` as JavaScript in a fresh QuickJS context, with
/// `$json`, `$items`, `$node`, `$now`, and `$workflow` bound as globals from
/// `ctx`, and converts the script's result back to a [`serde_json::Value`].
pub fn eval_js(script: &str, ctx: &EvalContext) -> Result<serde_json::Value, ExprError> {
    let runtime = rquickjs::Runtime::new().map_err(|e| ExprError::Runtime(e.to_string()))?;
    let js_context =
        rquickjs::Context::full(&runtime).map_err(|e| ExprError::Runtime(e.to_string()))?;

    js_context.with(|js| -> Result<serde_json::Value, ExprError> {
        let globals = js.globals();

        let json_val = json_to_js(&js, &ctx.json)?;
        globals
            .set("$json", json_val)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let items_json = serde_json::Value::Array(
            ctx.items
                .iter()
                .map(|item| serde_json::json!({"json": item}))
                .collect(),
        );
        let items_fn = make_items_fn(&js, items_json)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        globals
            .set("$items", items_fn)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let node_json = serde_json::Value::Object(
            ctx.node_json
                .iter()
                .map(|(name, output)| (name.clone(), serde_json::json!({"json": output})))
                .collect(),
        );
        let node_val = json_to_js(&js, &node_json)?;
        globals
            .set("$node", node_val)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        globals
            .set("$now", chrono::Utc::now().to_rfc3339())
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let workflow_val = json_to_js(&js, &serde_json::json!({"name": ctx.workflow_name}))?;
        globals
            .set("$workflow", workflow_val)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let result: rquickjs::Value = js
            .eval(script)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        js_to_json(&js, result)
    })
}

/// Builds the `$items()` native function bound to the precomputed items
/// array.
///
/// The array is captured as plain Rust data (`serde_json::Value`, not a
/// `rquickjs::Value`) and re-parsed into the calling context on each
/// invocation via a `Ctx` parameter supplied fresh per call (rather than a
/// `Ctx` captured by the closure). Capturing a `Ctx` clone in the closure
/// instead would create a reference cycle — the native function object is
/// itself reachable from the context's global object, so a `Ctx` clone
/// captured inside it keeps the context alive indefinitely and QuickJS
/// aborts with a `JS_FreeRuntime` assertion once the runtime is dropped
/// while such an object is still live.
///
/// This is a free function with an explicit `'js` lifetime parameter
/// (rather than a closure written inline in [`eval_js`]) because closures do
/// not get ordinary fn-style lifetime elision: an elided lifetime in a
/// closure's return type is inferred independently of one in its argument
/// position, which fails to unify against `rquickjs`'s invariant `Value<'js>`.
/// Naming `'js` on an enclosing function and reusing it in the closure's
/// signature sidesteps that.
fn make_items_fn<'js>(
    ctx: &rquickjs::Ctx<'js>,
    items_json: serde_json::Value,
) -> rquickjs::Result<rquickjs::Function<'js>> {
    rquickjs::Function::new(
        ctx.clone(),
        move |call_ctx: rquickjs::Ctx<'js>| -> rquickjs::Result<rquickjs::Value<'js>> {
            let text = serde_json::to_string(&items_json)
                .expect("serde_json::Value serialization cannot fail");
            call_ctx.json_parse(text)
        },
    )
}

/// Converts a `serde_json::Value` into a QuickJS value by round-tripping
/// through JSON text (via QuickJS's own `JSON.parse`), avoiding a manual
/// recursive conversion.
fn json_to_js<'js>(
    ctx: &rquickjs::Ctx<'js>,
    value: &serde_json::Value,
) -> Result<rquickjs::Value<'js>, ExprError> {
    let text = serde_json::to_string(value).map_err(|e| ExprError::Conversion(e.to_string()))?;
    ctx.json_parse(text)
        .map_err(|e| ExprError::Conversion(e.to_string()))
}

/// Converts a QuickJS value back into a `serde_json::Value` by round-tripping
/// through JSON text (via QuickJS's own `JSON.stringify`). Values that
/// `JSON.stringify` has no representation for (e.g. `undefined`, functions)
/// become `null`, matching `JSON.stringify`'s own behavior for such values.
fn js_to_json<'js>(
    ctx: &rquickjs::Ctx<'js>,
    value: rquickjs::Value<'js>,
) -> Result<serde_json::Value, ExprError> {
    match ctx.json_stringify(value) {
        Ok(Some(s)) => {
            let text = s.to_string().map_err(|e| ExprError::Conversion(e.to_string()))?;
            serde_json::from_str(&text).map_err(|e| ExprError::Conversion(e.to_string()))
        }
        Ok(None) => Ok(serde_json::Value::Null),
        Err(e) => Err(ExprError::Conversion(e.to_string())),
    }
}

/// Recursively resolves `{{ ... }}` expressions embedded in a JSON parameter
/// tree. A string that is *entirely* a single `{{ ... }}` expression
/// (surrounding whitespace allowed) is replaced by that expression's raw
/// evaluated value, preserving its JSON type. A string containing `{{ ... }}`
/// mixed with other text has each expression evaluated, stringified, and
/// spliced back into the surrounding text. Strings with no `{{ }}` pass
/// through unchanged. Non-string values recurse structurally.
pub fn resolve_parameters(
    params: &serde_json::Value,
    ctx: &EvalContext,
) -> Result<serde_json::Value, ExprError> {
    match params {
        serde_json::Value::String(s) => resolve_string(s, ctx),
        serde_json::Value::Array(items) => {
            let resolved = items
                .iter()
                .map(|v| resolve_parameters(v, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(serde_json::Value::Array(resolved))
        }
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map.iter() {
                out.insert(k.clone(), resolve_parameters(v, ctx)?);
            }
            Ok(serde_json::Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Resolves `{{ }}` expressions within a single string leaf. See
/// [`resolve_parameters`] for the whole-string-vs-mixed-text distinction.
fn resolve_string(s: &str, ctx: &EvalContext) -> Result<serde_json::Value, ExprError> {
    let trimmed = s.trim();
    if let Some(inner) = trimmed.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
        if !inner.contains("}}") {
            // The whole string is exactly one expression: return its raw value.
            return eval_js(inner.trim(), ctx);
        }
    }

    if !s.contains("{{") {
        return Ok(serde_json::Value::String(s.to_string()));
    }

    // Mixed text: splice each {{ ... }} expression's stringified result back in.
    let mut result = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        let end = after_open
            .find("}}")
            .ok_or_else(|| ExprError::Runtime(format!("unterminated expression in: {s}")))?;
        let expr_src = after_open[..end].trim();
        let value = eval_js(expr_src, ctx)?;
        result.push_str(&stringify_for_splice(&value));
        rest = &after_open[end + 2..];
    }
    result.push_str(rest);
    Ok(serde_json::Value::String(result))
}

/// Stringifies an evaluated expression's result for splicing into
/// surrounding text: numbers/bools render as their literal text,
/// objects/arrays as compact JSON, and `null` as an empty string.
fn stringify_for_splice(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn empty_ctx() -> EvalContext<'static> {
        EvalContext {
            json: serde_json::json!({}),
            items: &[],
            node_json: Box::leak(Box::new(HashMap::new())),
            workflow_name: "test-workflow",
        }
    }

    #[test]
    fn evaluates_simple_arithmetic() {
        let result = eval_js("1 + 2", &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!(3));
    }

    #[test]
    fn reads_json_global() {
        let ctx = EvalContext {
            json: serde_json::json!({"name": "Ada"}),
            ..empty_ctx()
        };
        let result = eval_js("$json.name", &ctx).unwrap();
        assert_eq!(result, serde_json::json!("Ada"));
    }

    #[test]
    fn returns_object_and_array_results() {
        let result = eval_js("({a: 1, b: [1, 2, 3]})", &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!({"a": 1, "b": [1, 2, 3]}));
    }

    #[test]
    fn malformed_script_returns_err_not_panic() {
        let result = eval_js("this is not valid js (((", &empty_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn reads_node_json_global() {
        let mut node_json = HashMap::new();
        node_json.insert("Trigger".to_string(), serde_json::json!({"x": 42}));
        let ctx = EvalContext {
            node_json: Box::leak(Box::new(node_json)),
            ..empty_ctx()
        };
        let result = eval_js(r#"$node["Trigger"].json.x"#, &ctx).unwrap();
        assert_eq!(result, serde_json::json!(42));
    }

    #[test]
    fn items_function_returns_current_items_wrapped_in_json_key() {
        let items = vec![serde_json::json!({"a": 1}), serde_json::json!({"a": 2})];
        let ctx = EvalContext { items: &items, ..empty_ctx() };
        let result = eval_js("$items().length", &ctx).unwrap();
        assert_eq!(result, serde_json::json!(2));
        let result = eval_js("$items()[1].json.a", &ctx).unwrap();
        assert_eq!(result, serde_json::json!(2));
    }

    #[test]
    fn now_is_an_rfc3339_string() {
        let result = eval_js("typeof $now", &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!("string"));
        let result = eval_js("$now.length > 10", &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!(true));
    }

    #[test]
    fn workflow_name_is_accessible() {
        let ctx = EvalContext { workflow_name: "my-workflow", ..empty_ctx() };
        let result = eval_js("$workflow.name", &ctx).unwrap();
        assert_eq!(result, serde_json::json!("my-workflow"));
    }

    #[test]
    fn resolve_parameters_passes_through_plain_strings() {
        let params = serde_json::json!({"greeting": "hello"});
        let result = resolve_parameters(&params, &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!({"greeting": "hello"}));
    }

    #[test]
    fn resolve_parameters_substitutes_whole_string_expression_with_raw_type() {
        let ctx = EvalContext { json: serde_json::json!({"count": 5}), ..empty_ctx() };
        let params = serde_json::json!({"n": "{{ $json.count }}"});
        let result = resolve_parameters(&params, &ctx).unwrap();
        assert_eq!(result, serde_json::json!({"n": 5}));
    }

    #[test]
    fn resolve_parameters_splices_mixed_text_expressions_as_strings() {
        let ctx = EvalContext { json: serde_json::json!({"name": "Ada"}), ..empty_ctx() };
        let params = serde_json::json!({"greeting": "Hello, {{ $json.name }}!"});
        let result = resolve_parameters(&params, &ctx).unwrap();
        assert_eq!(result, serde_json::json!({"greeting": "Hello, Ada!"}));
    }

    #[test]
    fn resolve_parameters_recurses_into_nested_objects_and_arrays() {
        let ctx = EvalContext { json: serde_json::json!({"x": 1}), ..empty_ctx() };
        let params = serde_json::json!({
            "list": ["{{ $json.x }}", "plain", {"nested": "{{ $json.x + 1 }}"}]
        });
        let result = resolve_parameters(&params, &ctx).unwrap();
        assert_eq!(result, serde_json::json!({"list": [1, "plain", {"nested": 2}]}));
    }
}
