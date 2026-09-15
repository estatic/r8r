use crate::domain::Item;
use crate::expr::{eval_js, EvalContext};
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use std::collections::HashMap;

const CODE_TIMEOUT_SECS: u64 = 2;

pub struct CodeNode;

#[async_trait]
impl Node for CodeNode {
    fn type_name(&self) -> &'static str {
        "core.code"
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
        // Known limitation: if the blocking thread is still running when the
        // timeout fires, there is no safe way in Rust to forcibly kill it — the
        // thread is abandoned (orphaned) and keeps running to completion in the
        // background, consuming a blocking-pool slot until it finishes. This is
        // an accepted limitation, not something this fix attempts to solve.
        let handle = tokio::task::spawn_blocking(move || {
            let empty_node_json: HashMap<String, serde_json::Value> = HashMap::new();
            let eval_ctx = EvalContext {
                json: first_json,
                items: &items_json,
                node_json: &empty_node_json,
                workflow_name: "",
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
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"doubled": 2}));
        assert_eq!(result[0][1].json, serde_json::json!({"doubled": 4}));
    }

    #[tokio::test]
    async fn missing_script_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn non_array_result_returns_error() {
        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"script": "return 42;"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    // Deliberately NOT `#[tokio::test]`: that macro drops its runtime at the
    // end of the test function, and `Runtime`'s default `Drop` blocks the
    // *current thread* until every outstanding `spawn_blocking` task
    // completes — including the never-ending `while (true) {}` this test
    // spawns. That would hang the test (and the whole suite) even though
    // `execute()`'s own `tokio::time::timeout` correctly returns at ~2s. So
    // this test builds its own runtime and explicitly calls
    // `shutdown_timeout` afterwards, which abandons the still-running
    // blocking thread after a short grace period instead of waiting on it
    // forever — the same orphaned-thread limitation documented in
    // `execute()`, just made non-fatal for the test process too.
    #[test]
    fn runaway_script_is_preempted_by_timeout() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();

        let node = CodeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "script": "while (true) {}"
            }),
            input_items: vec![],
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
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
