//! Expression engine core: a QuickJS wrapper that evaluates JS snippets
//! against a small set of JSON-derived globals (`$json`, `$items`, `$node`,
//! `$now`, `$workflow`, and `$args` for library tool calls).
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
    /// What each earlier node sent (its first output's items), by id and
    /// by name. Bound as `$node["name"]` and `$("name")`, each with
    /// `.json` (the first item's data), `.item`, `.first()`, `.last()` and
    /// `.all()`, as in n8n.
    pub node_items: &'a HashMap<String, Vec<serde_json::Value>>,
    /// Bound as `$workflow.name`.
    pub workflow_name: &'a str,
    /// Bound as `$args`: a tool call's arguments (library tools only).
    /// `None` leaves `$args` undefined.
    pub args: Option<&'a serde_json::Value>,
}

/// Wall-clock deadline for a single `eval_js` call, enforced via QuickJS's
/// own interrupt handler (checked periodically during bytecode execution,
/// including inside a tight loop) rather than an external OS-level
/// mechanism -- this is what lets a runaway script actually stop, instead
/// of merely being abandoned on a background thread. Applies uniformly to
/// every `eval_js` caller: both `core.code` scripts and every `{{ }}`
/// parameter expression across all node types, the latter of which
/// previously had no timeout protection at all.
const SCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Memory ceiling for a single `eval_js` call's QuickJS runtime. Generous
/// for any legitimate transform script; bounds a runaway allocation loop
/// (e.g. repeated string/array doubling) well short of exhausting real
/// process memory.
const SCRIPT_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ExprError {
    #[error("expression runtime error: {0}")]
    Runtime(String),
    /// The script threw: its message, and QuickJS's stack (`at f
    /// (eval_script:3:20)` lines) when it has one.
    #[error("expression runtime error: {message}")]
    Thrown { message: String, stack: Option<String> },
    #[error("expression value conversion error: {0}")]
    Conversion(String),
    #[error("script execution timed out")]
    Timeout,
}

/// `$node` and `$()` over `__r8r_node_items` (node -> its items' data).
const NODE_ACCESS: &str = r#"
var $node = {};
for (const name of Object.keys(__r8r_node_items)) {
  const its = __r8r_node_items[name].map((json) => ({ json }));
  $node[name] = { json: its.length ? its[0].json : {}, item: its[0], first: () => its[0], last: () => its[its.length - 1], all: () => its };
}
function $(name) {
  if (!Object.prototype.hasOwnProperty.call($node, name)) throw new Error(`No node "${name}" ran before this one`);
  return $node[name];
}
"#;

/// Evaluates `script` as JavaScript in a fresh QuickJS context, with
/// `$json`, `$items`, `$node`, `$now`, and `$workflow` bound as globals from
/// `ctx`, and converts the script's result back to a [`serde_json::Value`].
///
/// Bounded by [`SCRIPT_TIMEOUT`] and [`SCRIPT_MEMORY_LIMIT_BYTES`] -- see
/// their docs.
pub fn eval_js(script: &str, ctx: &EvalContext) -> Result<serde_json::Value, ExprError> {
    let runtime = rquickjs::Runtime::new().map_err(|e| ExprError::Runtime(e.to_string()))?;
    runtime.set_memory_limit(SCRIPT_MEMORY_LIMIT_BYTES);
    let start = std::time::Instant::now();
    runtime.set_interrupt_handler(Some(Box::new(move || start.elapsed() >= SCRIPT_TIMEOUT)));
    let js_context =
        rquickjs::Context::full(&runtime).map_err(|e| ExprError::Runtime(e.to_string()))?;

    js_context.with(|js| -> Result<serde_json::Value, ExprError> {
        if crate::intl::is_used_by(script) {
            crate::intl::install(&js).map_err(|e| ExprError::Runtime(format!("could not set up Intl: {e}")))?;
        }
        if crate::url_js::is_used_by(script) {
            crate::url_js::install(&js)
                .map_err(|e| ExprError::Runtime(format!("could not set up URL: {e}")))?;
        }
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

        let node_items = serde_json::to_value(ctx.node_items).map_err(|e| ExprError::Conversion(e.to_string()))?;
        let node_items = json_to_js(&js, &node_items)?;
        globals
            .set("__r8r_node_items", node_items)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        js.eval::<(), _>(NODE_ACCESS)
            .map_err(|e| ExprError::Runtime(format!("could not set up $node: {e}")))?;

        globals
            .set("$now", chrono::Utc::now().to_rfc3339())
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let workflow_val = json_to_js(&js, &serde_json::json!({"name": ctx.workflow_name}))?;
        globals
            .set("$workflow", workflow_val)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        if let Some(args) = ctx.args {
            let args_val = json_to_js(&js, args)?;
            globals
                .set("$args", args_val)
                .map_err(|e| ExprError::Runtime(e.to_string()))?;
        }

        let result: rquickjs::Value = match js.eval(script) {
            Ok(v) => v,
            Err(e) => {
                // The interrupt handler's own exception carries no
                // reliable, stable message text -- detect the timeout by
                // elapsed time instead, so callers get a clean, predictable
                // error regardless of QuickJS's internal wording.
                if start.elapsed() >= SCRIPT_TIMEOUT {
                    return Err(ExprError::Timeout);
                }
                return Err(thrown(&js, e));
            }
        };
        js_to_json(&js, result)
    })
}

/// What a script threw. For a JS exception rquickjs only returns
/// `Error::Exception`, whose text is a generic "Exception generated by
/// QuickJS"; the thrown value itself (and its stack) waits in the context.
fn thrown(js: &rquickjs::Ctx<'_>, e: rquickjs::Error) -> ExprError {
    if !matches!(e, rquickjs::Error::Exception) {
        return ExprError::Runtime(e.to_string());
    }
    let value = js.catch();
    let stack = value.as_exception().and_then(|ex| ex.stack()).filter(|s| !s.trim().is_empty());
    ExprError::Thrown { message: exception_message(js, value, e), stack }
}

fn exception_message<'js>(js: &rquickjs::Ctx<'js>, thrown: rquickjs::Value<'js>, e: rquickjs::Error) -> String {
    if let Some(ex) = thrown.as_exception() {
        // As JS prints it ("TypeError: …"); rquickjs's Display would give
        // "Error:<line>:<col> …" plus the stack.
        let name: Option<String> = ex.get("name").ok();
        let message = ex.message().unwrap_or_default();
        return match name.filter(|n| !n.is_empty()) {
            Some(n) if !message.is_empty() => format!("{n}: {message}"),
            Some(n) => n,
            None => message,
        };
    }
    match thrown.as_string().and_then(|s| s.to_string().ok()) {
        Some(s) => s,
        None => js_to_json(js, thrown).map(|v| v.to_string()).unwrap_or_else(|_| e.to_string()),
    }
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
            return eval_js(inner.trim(), ctx).map_err(|e| explain(inner.trim(), e));
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
        let value = eval_js(expr_src, ctx).map_err(|e| explain(expr_src, e))?;
        result.push_str(&stringify_for_splice(&value));
        rest = &after_open[end + 2..];
    }
    result.push_str(rest);
    Ok(serde_json::Value::String(result))
}

/// A syntax error in an expression, with the line it is on and, for the
/// common slip of a quoted string running onto the next line, the fix.
fn explain(src: &str, err: ExprError) -> ExprError {
    let ExprError::Thrown { message, stack } = err else {
        return err;
    };
    if !message.starts_with("SyntaxError") {
        return ExprError::Thrown { message, stack };
    }
    let line_no = stack
        .as_deref()
        .and_then(|s| s.split("eval_script:").nth(1))
        .and_then(|rest| rest.split(':').next())
        .and_then(|n| n.parse::<usize>().ok());
    let mut text = message.clone();
    if let Some(n) = line_no {
        // For an unclosed string, the line the string opened on.
        let shown = n.max(1);
        if let Some(line) = src.lines().nth(shown - 1) {
            text.push_str(&format!(" (expression line {shown}: {})", line.trim()));
        }
    }
    if message.contains("unexpected end of string") {
        text.push_str(". A '…' or \"…\" string can't continue on the next line: close it on the same line (write \\n for a line break), or use a `backtick` string, which may span lines");
    }
    ExprError::Thrown {
        message: text,
        stack,
    }
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

    #[test]
    fn emoji_and_cyrillic_survive_expressions() {
        let ctx = EvalContext {
            json: serde_json::json!({"sentence": "Кот 🐱 спит 😴", "index": 1, "total": 3}),
            ..empty_ctx()
        };
        let out = resolve_parameters(&serde_json::json!({"c": "📸 {{ $json.index + '/' + $json.total }} — {{ $json.sentence + ' ✅' }}"}), &ctx).unwrap();
        assert_eq!(out["c"], "📸 1/3 — Кот 🐱 спит 😴 ✅");
    }

    #[test]
    fn the_caption_expression_as_pasted() {
        let ctx = EvalContext {
            json: serde_json::json!({"index": 2, "total": 5, "sentence": "Кот 🐱", "search_query": "cat", "image_source": "Wikimedia", "source_page": "https://x.org"}),
            ..empty_ctx()
        };
        let caption = "{{\n$json.index + '/' +\n$json.total +\n'\\n\\n\n' + $json.sentence +\n'\\n\\n\n' + ($json.third_search_query || $json.fallback_search_query || $json.search_query ||\n$json.first_search_query || '') +\n'\\n\nИсточник: ' + ($json.image_source || '') +\n'\\n\n' + ($json.source_page || '')\n}}";
        // A quoted string can't span lines in JavaScript: the error says so and where.
        let err = resolve_parameters(&serde_json::json!({"caption": caption}), &ctx)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("unexpected end of string (expression line 3: '\\n\\n"),
            "{err}"
        );
        assert!(err.contains("use a `backtick` string"), "{err}");

        // The same caption as a backtick string, line breaks and all, works.
        let fixed = "{{ `${$json.index}/${$json.total}\n\n${$json.sentence}\n\n${$json.third_search_query || $json.fallback_search_query || $json.search_query || $json.first_search_query || ''}\nИсточник: ${$json.image_source || ''}\n${$json.source_page || ''}` }}";
        let out = resolve_parameters(&serde_json::json!({"caption": fixed}), &ctx).unwrap();
        assert_eq!(
            out["caption"],
            "2/5\n\nКот 🐱\n\ncat\nИсточник: Wikimedia\nhttps://x.org"
        );
    }

    fn empty_ctx() -> EvalContext<'static> {
        EvalContext {
            json: serde_json::json!({}),
            items: &[],
            node_items: Box::leak(Box::new(HashMap::new())),
            workflow_name: "test-workflow",
            args: None,
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
    fn expressions_inside_a_long_multi_line_prompt_are_resolved_in_place() {
        let ctx = EvalContext {
            json: serde_json::json!({"message": {"text": "Is fire hot?", "from": {"first_name": "Ivan"}}}),
            ..empty_ctx()
        };
        let prompt = "You are a bot.\n\nRules:\n1. Be brief.\n\nQuestion from {{ $json.message.from.first_name }}:\n{{ $json.message.text }}";
        let resolved = resolve_parameters(&serde_json::json!(prompt), &ctx).unwrap();
        assert_eq!(resolved, serde_json::json!("You are a bot.\n\nRules:\n1. Be brief.\n\nQuestion from Ivan:\nIs fire hot?"));
    }

    #[test]
    fn a_thrown_error_reports_its_javascript_message() {
        let result = eval_js("$json.message.text", &empty_ctx());
        match result {
            Err(ExprError::Thrown { message: msg, .. }) => assert!(
                msg == "TypeError: cannot read property 'text' of undefined",
                "expected the TypeError's own message, got {msg:?}"
            ),
            other => panic!("expected ExprError::Thrown, got {other:?}"),
        }
        match eval_js("throw 'plain string'", &empty_ctx()) {
            Err(ExprError::Thrown { message: msg, .. }) => assert!(msg.contains("plain string"), "got {msg:?}"),
            other => panic!("expected ExprError::Thrown, got {other:?}"),
        }
    }

    #[test]
    fn a_runaway_loop_is_interrupted_within_the_deadline_not_left_to_run_forever() {
        let start = std::time::Instant::now();
        let result = eval_js("while (true) {}", &empty_ctx());
        let elapsed = start.elapsed();

        match result {
            Err(ExprError::Timeout) => {}
            other => panic!("expected ExprError::Timeout, got {other:?}"),
        }
        // Bounded by the interrupt handler's own deadline (a few seconds),
        // not left to spin -- this call is fully synchronous, so a real
        // interruption (rather than merely abandoning a background thread,
        // as core.code's outer async timeout does) is the only way this
        // test returns at all within a sane bound.
        assert!(elapsed < std::time::Duration::from_secs(4), "expected the interrupt handler to cut the loop short, took {elapsed:?}");
    }

    #[test]
    fn a_memory_exhausting_script_is_rejected_rather_than_exhausting_real_memory() {
        // Repeatedly doubles a string, doubling memory use each iteration --
        // an unbounded version of this allocates far more than physical
        // memory within a few dozen iterations. The loop count (1000) is
        // far beyond what the memory limit allows, so this proves the
        // limit -- not the loop finishing -- is what stops it.
        let script = "let s = 'x'; for (let i = 0; i < 1000; i++) { s = s + s; } s.length;";
        let result = eval_js(script, &empty_ctx());
        assert!(result.is_err(), "expected the memory limit to reject this script, got {result:?}");
    }

    #[test]
    fn an_ordinary_fast_expression_is_unaffected_by_the_new_limits() {
        // Guards against a regression where the timeout/memory limit are so
        // tight they break normal usage, not just pathological scripts.
        let result = eval_js("({a: 1, b: [1, 2, 3]})", &empty_ctx()).unwrap();
        assert_eq!(result, serde_json::json!({"a": 1, "b": [1, 2, 3]}));
    }

    #[test]
    fn reads_node_json_global() {
        let mut node_items = HashMap::new();
        node_items.insert("Trigger".to_string(), vec![serde_json::json!({"x": 42})]);
        let ctx = EvalContext {
            node_items: Box::leak(Box::new(node_items)),
            ..empty_ctx()
        };
        let result = eval_js(r#"$node["Trigger"].json.x"#, &ctx).unwrap();
        assert_eq!(result, serde_json::json!(42));
    }

    #[test]
    fn reads_an_earlier_node_by_name_like_n8n() {
        let mut node_items = HashMap::new();
        node_items.insert("Get users".to_string(), vec![serde_json::json!({"n": 1}), serde_json::json!({"n": 2})]);
        let ctx = EvalContext { node_items: Box::leak(Box::new(node_items)), ..empty_ctx() };
        for (script, want) in [
            (r#"$("Get users").item.json.n"#, serde_json::json!(1)),
            (r#"$("Get users").first().json.n"#, serde_json::json!(1)),
            (r#"$("Get users").last().json.n"#, serde_json::json!(2)),
            (r#"$("Get users").all().length"#, serde_json::json!(2)),
            (r#"$('Get users').json.n"#, serde_json::json!(1)),
            (r#"$node["Get users"].last().json.n"#, serde_json::json!(2)),
        ] {
            assert_eq!(eval_js(script, &ctx).unwrap(), want, "{script}");
        }
        let resolved = resolve_parameters(&serde_json::json!("n = {{ $(\"Get users\").first().json.n }}"), &ctx).unwrap();
        assert_eq!(resolved, serde_json::json!("n = 1"));
        match eval_js(r#"$("Nope").json"#, &ctx) {
            Err(ExprError::Thrown { message, .. }) => assert!(message.contains(r#"No node "Nope" ran before this one"#), "{message}"),
            other => panic!("expected a thrown error, got {other:?}"),
        }
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

    #[test]
    fn args_resolve_inside_a_template() {
        let args = serde_json::json!({"city": "Warsaw"});
        let ctx = EvalContext { args: Some(&args), ..empty_ctx() };
        let params = serde_json::json!({"url": "https://api.example.com/weather?city={{ $args.city }}"});
        assert_eq!(
            resolve_parameters(&params, &ctx).unwrap(),
            serde_json::json!({"url": "https://api.example.com/weather?city=Warsaw"})
        );
    }

    #[test]
    fn args_is_undefined_without_args() {
        let ctx = empty_ctx();
        let params = serde_json::json!({"t": "{{ typeof $args }}"});
        assert_eq!(resolve_parameters(&params, &ctx).unwrap(), serde_json::json!({"t": "undefined"}));
    }

    #[test]
    fn args_values_are_data_not_code() {
        let args = serde_json::json!({"x": "{{ 1 + 1 }}", "y": "'); throw new Error('pwned'); ('"});
        let ctx = EvalContext { args: Some(&args), ..empty_ctx() };
        let params = serde_json::json!({"a": "{{ $args.x }}", "b": "{{ $args.y }}"});
        assert_eq!(
            resolve_parameters(&params, &ctx).unwrap(),
            serde_json::json!({"a": "{{ 1 + 1 }}", "b": "'); throw new Error('pwned'); ('"})
        );
    }

    #[test]
    fn args_can_be_url_encoded_inside_a_template() {
        // The Tools page's HTTP template relies on this to keep model text
        // from adding query params or path segments.
        let args = serde_json::json!({"query": "a&b #c/../admin"});
        let ctx = EvalContext { args: Some(&args), ..empty_ctx() };
        let params = serde_json::json!({"url": "https://api.example.com/search?q={{ encodeURIComponent($args.query) }}"});
        assert_eq!(
            resolve_parameters(&params, &ctx).unwrap(),
            serde_json::json!({"url": "https://api.example.com/search?q=a%26b%20%23c%2F..%2Fadmin"})
        );
    }
}
