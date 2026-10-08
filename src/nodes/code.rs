use crate::domain::Item;
use crate::expr::{eval_js, EvalContext, ExprError};
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::collections::HashMap;

/// A backstop, not the primary deadline: `eval_js` (`src/expr.rs`) enforces
/// its own 2s timeout via QuickJS's interrupt handler, which reliably
/// returns control even from inside a tight infinite loop. This value is
/// kept comfortably above that inner deadline so the inner one always
/// fires first in the ordinary case; this only matters for a pathological
/// script the interrupt handler can't reach.
const CODE_TIMEOUT_SECS: u64 = 5;

pub struct CodeNode;

#[async_trait]
impl Node for CodeNode {
    fn type_name(&self) -> &'static str {
        "core.code"
    }
    fn display_name(&self) -> &'static str {
        "Code"
    }
    fn description(&self) -> &'static str {
        "Runs custom JavaScript or Python to transform items."
    }
    fn category(&self) -> crate::node::NodeCategory {
        crate::node::NodeCategory::Action
    }
    fn icon(&self) -> &'static str {
        "💻"
    }

    // core.code's "parameter" IS the script to run, not a value to
    // interpolate. Running it through `expr::resolve_parameters` would
    // corrupt (or throw on) any script that happens to contain literal
    // `{{ }}` text, e.g. when building a template string for a downstream
    // node. See `src/engine.rs`'s `execute_workflow`, which checks this.
    fn resolves_parameters(&self) -> bool {
        false
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let script = ctx
            .parameters
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("core.code requires a \"script\" parameter".into()))?
            .to_string();

        // `eval_js` wraps each entry of `EvalContext.items` in `{"json": ...}`
        // itself when building the `$items()` array (see `src/expr.rs`), so
        // these must be the raw item values, not pre-wrapped here — otherwise
        // scripts would see a double-wrapped `{json: {json: ...}}` shape.
        let result = match ctx.parameters.get("language").and_then(|v| v.as_str()).unwrap_or("javaScript") {
            "javaScript" => self.run_javascript(ctx, script).await?,
            "python" => run_python(ctx, &script).await?,
            other => {
                return Err(NodeError::ExecutionFailed(format!(
                    "core.code: unknown language \"{other}\" (expected \"javaScript\" or \"python\")"
                )))
            }
        };

        let result_array = result
            .as_array()
            .ok_or_else(|| NodeError::ExecutionFailed("core.code script must return an array".into()))?;

        let out_items = result_array
            .iter()
            .map(|entry| {
                let json = entry.get("json").cloned().unwrap_or_else(|| entry.clone());
                Item { json, binary: serde_json::json!({}) }
            })
            .collect();

        Ok(vec![out_items])
    }
}

/// Python code over all items at once: it sees `items` (each with `.json`)
/// and returns the items to pass on, as the JavaScript does.
async fn run_python(ctx: &NodeExecutionContext, script: &str) -> Result<serde_json::Value, NodeError> {
    let items: Vec<serde_json::Value> = ctx.input_items.iter().map(|i| serde_json::json!({"json": i.json})).collect();
    let timeout = std::env::var("N8N_RUNNERS_TASK_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(CODE_TIMEOUT_SECS);
    // `_json` (the first item) and `_node["id"]["json"]`, as n8n's Python
    // Code node names them, beside `items`.
    let globals = serde_json::json!({
        "items": items,
        "_json": ctx.input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})),
        "_node": ctx.upstream.iter().map(|(id, json)| (id.clone(), serde_json::json!({"json": json}))).collect::<serde_json::Map<_, _>>(),
    });
    let (results, _console) = crate::n8n::nodes::python::run_process(script, &items, false, Some(globals), timeout).await.map_err(|e| {
        NodeError::ExecutionFailed(match e.hint {
            Some(hint) => format!("{} ({hint})", e.message),
            None if !e.trace.is_empty() => python_trace(script, &e.error, &e.trace),
            None => e.message,
        })
    })?;
    Ok(results.into_iter().next().unwrap_or(serde_json::Value::Null))
}

impl CodeNode {
    async fn run_javascript(&self, ctx: &NodeExecutionContext, script: String) -> Result<serde_json::Value, NodeError> {
        let items_json: Vec<serde_json::Value> =
            ctx.input_items.iter().map(|i| i.json.clone()).collect();
        let first_json = ctx.input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({}));
        let wrapped_script = format!("{JS_PREFIX}{script} }})($items())");
        let script_text = script.clone();

        // `eval_js` is purely synchronous (no internal `.await` points), so
        // wrapping it directly in `tokio::time::timeout(..., async { eval_js(...) })`
        // does NOT work: `timeout` can only re-check its deadline at an `.await`
        // suspension point, and a future that runs to completion inside a single
        // `poll()` call never yields one. A runaway script would hang forever.
        //
        // Instead, run `eval_js` on tokio's blocking thread pool via
        // `spawn_blocking`, and race the resulting `JoinHandle` (a genuine
        // `.await` point backed by a separate OS thread) against the timeout.
        // Everything the closure needs (`items_json`, `node_json`,
        // `first_json`, `wrapped_script`) is already owned data local to this
        // function, so it can move into the `'static` closure without
        // borrowing from `ctx`; `EvalContext` is constructed entirely inside
        // the closure so its borrowed fields reference only closure-local data.
        //
        // `eval_js` enforces its own 2s deadline internally via QuickJS's
        // interrupt handler (checked during bytecode execution, including
        // inside a tight loop), so it reliably returns on its own well
        // before this outer timeout -- unlike an external OS-level
        // mechanism, this actually stops a runaway script rather than
        // merely abandoning a thread that keeps running in the background.
        // The remaining known limitation is narrower: only a pathological
        // script whose execution the interrupt handler can't interrupt
        // between checks (e.g. one dominated by a single very expensive
        // native call) could still run to completion on an orphaned
        // blocking-pool thread after this outer timeout fires.
        // A library tool call's arguments, exposed to the script as `$args`.
        let tool_args = ctx.tool_args.clone();
        let upstream = ctx.upstream.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let node_json: HashMap<String, serde_json::Value> = upstream;
            let eval_ctx = EvalContext {
                json: first_json,
                items: &items_json,
                node_json: &node_json,
                workflow_name: "",
                args: tool_args.as_ref(),
            };
            eval_js(&wrapped_script, &eval_ctx)
        });

        let result = tokio::time::timeout(std::time::Duration::from_secs(CODE_TIMEOUT_SECS), handle)
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("core.code timed out after {CODE_TIMEOUT_SECS}s")))?
            .map_err(|e| NodeError::ExecutionFailed(format!("core.code script execution panicked: {e}")))?
            .map_err(|e| match e {
                ExprError::Thrown { message, stack } => NodeError::ExecutionFailed(js_trace(&script_text, &message, stack.as_deref())),
                other => NodeError::ExecutionFailed(other.to_string()),
            })?;
        Ok(result)
    }
}

/// Where the user's code is wrapped: `(function(items) { <script> })(...)`.
const JS_PREFIX: &str = "(function(items) { ";

/// The error, then each frame in the user's code, nearest first:
/// `at f (line 3:20)    return x.missing.deep;`. Lines and columns count
/// from the top of the code box; r8r's own wrapper frame is left out.
fn js_trace(script: &str, message: &str, stack: Option<&str>) -> String {
    let lines: Vec<&str> = script.lines().collect();
    let mut out = message.to_string();
    for frame in stack.unwrap_or_default().lines() {
        let frame = frame.trim().trim_start_matches("at ");
        let Some((name, place)) = frame.rsplit_once(" (") else { continue };
        if name == "<eval>" {
            continue;
        }
        let mut parts = place.trim_end_matches(')').rsplit(':');
        let column: usize = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
        let Some(line) = parts.next().and_then(|l| l.parse::<usize>().ok()) else { continue };
        let column = if line == 1 { column.saturating_sub(JS_PREFIX.len()) } else { column };
        let name = if name == "<anonymous>" { "main code" } else { name };
        let source = lines.get(line.saturating_sub(1)).map(|l| l.trim()).unwrap_or("");
        out.push_str(&format!("\n  at {name} (line {line}:{column})    {source}"));
    }
    out
}

/// A Python error with the frames in the user's code, as [`js_trace`].
fn python_trace(script: &str, message: &str, trace: &[(u64, String)]) -> String {
    let lines: Vec<&str> = script.lines().collect();
    let mut out = message.to_string();
    for (line, name) in trace {
        let source = lines.get((*line as usize).saturating_sub(1)).map(|l| l.trim()).unwrap_or("");
        out.push_str(&format!("\n  at {name} (line {line})    {source}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(parameters: serde_json::Value, inputs: Vec<serde_json::Value>) -> NodeExecutionContext {
        NodeExecutionContext {
            parameters,
            input_items: inputs.into_iter().map(|json| Item { json, binary: serde_json::json!({}) }).collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn python_code_maps_the_items_like_javascript_does() {
        let ctx = ctx_with(
            serde_json::json!({"language": "python", "script": "return [{'json': {**item.json, 'double': item.json['n'] * 2}} for item in items]"}),
            vec![serde_json::json!({"n": 2}), serde_json::json!({"n": 5})],
        );
        let out = CodeNode.execute(&ctx).await.unwrap();
        let json: Vec<_> = out[0].iter().map(|i| i.json.clone()).collect();
        assert_eq!(json, vec![serde_json::json!({"n": 2, "double": 4}), serde_json::json!({"n": 5, "double": 10})]);
    }

    #[tokio::test]
    async fn python_may_return_plain_dicts() {
        let ctx = ctx_with(serde_json::json!({"language": "python", "script": "return [{'total': sum(i.json['n'] for i in items)}]"}), vec![serde_json::json!({"n": 2}), serde_json::json!({"n": 5})]);
        let out = CodeNode.execute(&ctx).await.unwrap();
        assert_eq!(out[0][0].json, serde_json::json!({"total": 7}));
    }

    #[tokio::test]
    async fn a_python_error_names_its_line() {
        let ctx = ctx_with(serde_json::json!({"language": "python", "script": "x = 1\nreturn items[0].json['missing']"}), vec![serde_json::json!({})]);
        let err = CodeNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("KeyError") && err.contains("line 2"), "{err}");
    }

    fn upstream_ctx(language: &str, script: &str) -> NodeExecutionContext {
        NodeExecutionContext {
            upstream: HashMap::from([("tg".to_string(), serde_json::json!({"message": {"chat": {"id": 42}, "text": "hi"}}))]),
            ..ctx_with(serde_json::json!({"language": language, "script": script}), vec![serde_json::json!({"response": "hello", "message": {"chat": {"id": 42}}})])
        }
    }

    #[tokio::test]
    async fn javascript_reads_json_and_earlier_nodes() {
        let ctx = upstream_ctx("javaScript", "return [{json: {chat: $json.message.chat.id, answer: $json.response, said: $node['tg'].json.message.text}}]");
        let out = CodeNode.execute(&ctx).await.unwrap();
        assert_eq!(out[0][0].json, serde_json::json!({"chat": 42, "answer": "hello", "said": "hi"}));
    }

    #[tokio::test]
    async fn python_reads_json_and_earlier_nodes() {
        let ctx = upstream_ctx("python", "return [{'chat': _json['message']['chat']['id'], 'answer': _json.response, 'said': _node['tg']['json']['message']['text']}]");
        let out = CodeNode.execute(&ctx).await.unwrap();
        assert_eq!(out[0][0].json, serde_json::json!({"chat": 42, "answer": "hello", "said": "hi"}));
    }

    #[tokio::test]
    async fn a_javascript_error_shows_where_it_happened_in_the_users_code() {
        let script = "const a = 1;\nfunction f(x) {\n  return x.missing.deep;\n}\nreturn [f({})];";
        let ctx = ctx_with(serde_json::json!({"script": script}), vec![serde_json::json!({})]);
        let err = CodeNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("TypeError: cannot read property 'deep' of undefined"), "{err}");
        assert!(err.contains("at f (line 3:20)    return x.missing.deep;"), "{err}");
        assert!(err.contains("at main code (line 5:9)    return [f({})];"), "{err}");
        assert!(!err.contains("eval_script") && !err.contains("<eval>"), "r8r's wrapper stays out of it: {err}");
    }

    #[tokio::test]
    async fn a_javascript_error_on_the_first_line_counts_columns_from_the_users_code() {
        let ctx = ctx_with(serde_json::json!({"script": "return null.x;"}), vec![]);
        let err = CodeNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("(line 1:"), "{err}");
        let column: usize = err.split("(line 1:").nth(1).unwrap().split(')').next().unwrap().parse().unwrap();
        assert!(column <= "return null.x;".len(), "column {column} must point into the user's line: {err}");
    }

    #[tokio::test]
    async fn a_python_error_shows_where_it_happened_in_the_users_code() {
        let script = "def pick(item):\n    return item.json['missing']\n\nreturn [pick(i) for i in items]";
        let ctx = ctx_with(serde_json::json!({"language": "python", "script": script}), vec![serde_json::json!({"a": 1})]);
        let err = CodeNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("KeyError: 'missing'"), "{err}");
        assert!(err.contains("at pick (line 2)    return item.json['missing']"), "{err}");
        assert!(err.contains("(line 4)    return [pick(i) for i in items]"), "{err}");
    }

    #[tokio::test]
    async fn javascript_stays_the_default_language() {
        let ctx = ctx_with(serde_json::json!({"script": "return items.map(i => ({json: {seen: i.json.n}}))"}), vec![serde_json::json!({"n": 3})]);
        let out = CodeNode.execute(&ctx).await.unwrap();
        assert_eq!(out[0][0].json, serde_json::json!({"seen": 3}));
    }

    #[tokio::test]
    async fn an_unknown_language_is_refused() {
        let ctx = ctx_with(serde_json::json!({"language": "ruby", "script": "1"}), vec![]);
        let err = CodeNode.execute(&ctx).await.unwrap_err().to_string();
        assert!(err.contains("\"ruby\""), "{err}");
    }

    #[test]
    fn code_node_opts_out_of_parameter_resolution() {
        assert!(!CodeNode.resolves_parameters());
    }

    #[test]
    fn trait_default_resolves_parameters_is_true() {
        // Any node that does NOT override `resolves_parameters` must keep the
        // trait's default (`true`), so its existing resolve-then-execute
        // behavior is unaffected by adding this method. Use a minimal local
        // node here rather than reaching into another `src/nodes/*` module.
        struct DefaultNode;
        #[async_trait]
        impl Node for DefaultNode {
            fn type_name(&self) -> &'static str {
                "test.default"
            }
            fn display_name(&self) -> &'static str {
                "Default"
            }
            fn description(&self) -> &'static str {
                "Test-only node used to verify the trait's resolves_parameters default."
            }
            fn category(&self) -> crate::node::NodeCategory {
                crate::node::NodeCategory::Action
            }
            async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
                Ok(vec![ctx.input_items.clone()])
            }
        }
        assert!(DefaultNode.resolves_parameters());
    }

    #[tokio::test]
    async fn script_transforms_items() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "script": "return items.map(i => ({json: {doubled: i.json.n * 2}}));"
            }),
            input_items: vec![
                Item { json: serde_json::json!({"n": 1}), binary: serde_json::json!({}) },
                Item { json: serde_json::json!({"n": 2}), binary: serde_json::json!({}) },
            ],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"doubled": 2}));
        assert_eq!(result[0][1].json, serde_json::json!({"doubled": 4}));
    }

    #[tokio::test]
    async fn missing_script_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![], ..Default::default() };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn non_array_result_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"script": "return 42;"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    // Built manually (not `#[tokio::test]`) and explicitly shut down below.
    // `eval_js`'s own interrupt handler now reliably returns control from
    // the `while (true) {}` script within ~2s, so the spawn_blocking task
    // actually completes rather than running forever -- but
    // `shutdown_timeout` (rather than an implicit `#[tokio::test]` runtime
    // drop) is kept as a defensive bound against a hang if that assumption
    // is ever wrong, rather than risking the whole suite blocking on it.
    #[test]
    fn runaway_script_is_preempted_by_timeout() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "script": "while (true) {}"
            }),
            input_items: vec![],
            ..Default::default()
        };

        let start = std::time::Instant::now();
        let result = rt.block_on(node.execute(&ctx));
        let elapsed = start.elapsed();

        match &result {
            Err(NodeError::ExecutionFailed(msg)) => {
                assert!(msg.contains("timed out"), "expected a timeout error, got: {msg}");
            }
            other => panic!("expected a timeout ExecutionFailed error, got: {other:?}"),
        }
        // Bounded by the 2s timeout, not by the infinite loop — a generous
        // upper bound proves the timeout actually preempted execution rather
        // than waiting for the (never-ending) script to finish on its own.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "expected the timeout to cut execution short (~2s), but took {elapsed:?}"
        );

        rt.shutdown_timeout(std::time::Duration::from_millis(100));
    }

    #[tokio::test]
    async fn empty_input_produces_empty_items_array() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"script": "return items;"}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
