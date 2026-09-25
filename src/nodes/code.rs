use crate::domain::Item;
use crate::expr::{eval_js, EvalContext};
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
        "Runs custom JavaScript to transform items."
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
        let items_json: Vec<serde_json::Value> =
            ctx.input_items.iter().map(|i| i.json.clone()).collect();
        let first_json = ctx.input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({}));
        let wrapped_script = format!("(function(items) {{ {script} }})($items())");

        // `eval_js` is purely synchronous (no internal `.await` points), so
        // wrapping it directly in `tokio::time::timeout(..., async { eval_js(...) })`
        // does NOT work: `timeout` can only re-check its deadline at an `.await`
        // suspension point, and a future that runs to completion inside a single
        // `poll()` call never yields one. A runaway script would hang forever.
        //
        // Instead, run `eval_js` on tokio's blocking thread pool via
        // `spawn_blocking`, and race the resulting `JoinHandle` (a genuine
        // `.await` point backed by a separate OS thread) against the timeout.
        // Everything the closure needs (`items_json`, `empty_node_json`,
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
        let handle = tokio::task::spawn_blocking(move || {
            let empty_node_json: HashMap<String, serde_json::Value> = HashMap::new();
            let eval_ctx = EvalContext {
                json: first_json,
                items: &items_json,
                node_json: &empty_node_json,
                workflow_name: "",
                args: tool_args.as_ref(),
            };
            eval_js(&wrapped_script, &eval_ctx)
        });

        let result = tokio::time::timeout(std::time::Duration::from_secs(CODE_TIMEOUT_SECS), handle)
            .await
            .map_err(|_| NodeError::ExecutionFailed(format!("core.code timed out after {CODE_TIMEOUT_SECS}s")))?
            .map_err(|e| NodeError::ExecutionFailed(format!("core.code script execution panicked: {e}")))?
            .map_err(|e| NodeError::ExecutionFailed(e.to_string()))?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
