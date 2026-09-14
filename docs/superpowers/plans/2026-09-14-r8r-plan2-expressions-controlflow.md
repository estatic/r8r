# r8r Plan 2 — Expression Engine + Control-Flow Nodes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give r8r an n8n-compatible `{{ }}` expression language (embedded QuickJS), upgrade the execution engine from Foundation's linear-only walk to a real topologically-ordered DAG with branching/merge and per-node error-output routing, and add the control-flow node set (If, Switch, Merge, Filter, Wait, NoOp, Code).

**Architecture:** A new `src/expr.rs` module wraps QuickJS (`rquickjs`) behind two functions — `eval_js` (evaluate one script against a small set of JSON globals) and `resolve_parameters` (walk a node's parameter JSON tree, substituting `{{ }}` expressions) — with zero dependency on the engine or domain types beyond plain `serde_json::Value`. The `Node` trait's `execute()` return type changes from a single `Vec<Item>` to `NodeOutput = Vec<Vec<Item>>` (one `Vec<Item>` per named output port), enabling If/Switch's multi-way branching and a reserved error-output sentinel. `src/engine.rs` is rewritten around Kahn's-algorithm topological sort (replacing `linear_order`), aggregating each node's inputs from every incoming connection and routing each node's multiple output ports to the right downstream connections. The persisted `Execution.node_outputs` shape (`HashMap<String, Vec<Item>>`) is deliberately left untouched — the engine flattens each node's primary (port 0) output before returning, so **no changes are needed to `domain.rs`, `storage/`, or `api/`** in this plan.

**Tech Stack:** Rust, `rquickjs` (QuickJS embedding), the existing Axum/sqlx/Tokio stack (unchanged).

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md` (§5.3, §6, §7)
**Roadmap reference:** `docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`, Plan 2 (§2.1–2.4)

## Global Constraints

- Expression evaluation is scoped **per-node, not per-item**: `$json` is bound to the *first* input item's json (or `{}` if there are none). True per-item expression evaluation (each item seeing its own `$json`) is out of scope for this plan — nodes that need to transform many items individually (the future Code node's per-item mode, or a richer Set node) are a later concern. This is a deliberate v1 simplification; note it in any node whose behavior depends on it.
- `$now` is bound as an RFC3339 string, not a JS `Date` object — no Date polyfill needed.
- `$items()` returns the array of the *current node's own input items* (each as `{json: ...}`), not an arbitrary node/run reference — n8n's fuller `$items('NodeName', outputIndex, runIndex)` signature is out of scope.
- Branch execution is **sequential, not concurrently spawned** — the topological order is walked node-by-node on one task, same as Foundation's engine. True concurrent branch execution via `tokio::spawn`/`JoinSet` is deferred; nothing in this plan's design blocks adding it later (each node's inputs are already fully computed from a `produced` map before it runs, independent of iteration order beyond dependency order).
- Still exactly **one start node required** (a node with no incoming connections) — multiple independent trigger nodes in one workflow remain out of scope, carried forward from Foundation.
- `Execution.node_outputs` (the persisted, API-visible shape) stays `HashMap<String, Vec<Item>>` — one node maps to its **primary (output-port-0) items only**. Per-output-port execution inspection (e.g. seeing an If node's `false` branch data separately) is deferred to a later plan (frontend/execution-hardening territory) — no `domain.rs`/`storage/`/`api/` changes happen in this plan.
- No dynamic/plugin node loading (unchanged from Foundation) — every node in this plan is compiled in and registered via `nodes::register_all`.
- Slack and Email nodes remain out of scope entirely (unchanged from Foundation).

### A note on `rquickjs`

The exact API surface of `rquickjs` (method names on `Value`/`Object`/`Array`, whether a `serde`-based JSON bridge is available in the installed version) could not be verified against live documentation while writing this plan. **Task 1's code blocks are a reference implementation, not literal code to transcribe blindly** — the implementer must check the actually-installed crate version's API (`cargo doc -p rquickjs --open`, or read the source under `~/.cargo/registry/src/.../rquickjs-*/`) and adapt method names as needed, preserving the *behavior and signatures* of `eval_js`/`EvalContext`/`resolve_parameters` described here (those are r8r's own functions, not rquickjs's). Every other task in this plan only calls `eval_js`/`resolve_parameters` — none of them touch rquickjs directly — so the risk is contained entirely to Task 1.

---

## File Structure

- `Cargo.toml` — add `rquickjs` dependency.
- `src/expr.rs` — new. `EvalContext`, `ExprError`, `eval_js`, `resolve_parameters`. Self-contained: depends only on `serde_json`, not on `domain.rs`/`node.rs`.
- `src/node.rs` — modify. `NodeOutput` type alias, `ERROR_OUTPUT` constant, `Node::execute`'s return type changes.
- `src/nodes/manual_trigger.rs`, `src/nodes/set.rs` — modify. Adapt to the new `NodeOutput` return type (wrap their single output in `vec![items]`); no behavior change.
- `src/nodes/if_node.rs` — new. If node.
- `src/nodes/switch.rs` — new. Switch node.
- `src/nodes/merge.rs` — new. Merge node (append/mergeByKey/waitForAll modes).
- `src/nodes/filter.rs`, `src/nodes/wait.rs`, `src/nodes/noop.rs` — new. Filter, Wait, NoOp/Sticky.
- `src/nodes/code.rs` — new. Code node (full-script execution via `expr::eval_js`-adjacent machinery, with a per-run timeout).
- `src/nodes/mod.rs` — modify. Register every new node type.
- `src/engine.rs` — rewrite. `topological_order` replaces `linear_order`; `execute_workflow` aggregates multi-input/multi-output and routes the error output; wires in `expr::resolve_parameters`.
- `src/lib.rs` — modify. Add `pub mod expr;`.
- `tests/api_test.rs` — modify. One new end-to-end test: a branching workflow (If → two Set branches → Merge) through the real HTTP API.

---

### Task 1: Expression engine core — QuickJS wrapper + `eval_js`

**Files:**
- Create: `src/expr.rs`
- Modify: `Cargo.toml` (add `rquickjs`)
- Modify: `src/lib.rs` (add `pub mod expr;`)

**Interfaces:**
- Produces: `r8r::expr::EvalContext<'a> { json: serde_json::Value, items: &'a [serde_json::Value], node_json: &'a std::collections::HashMap<String, serde_json::Value>, workflow_name: &'a str }`
- Produces: `r8r::expr::ExprError` (`thiserror`, at least a `Runtime(String)` variant and a `Conversion(String)` variant)
- Produces: `r8r::expr::eval_js(script: &str, ctx: &EvalContext) -> Result<serde_json::Value, ExprError>` — evaluates `script` as JavaScript with `$json`, `$items`, `$node`, `$now`, `$workflow` bound as globals per the Global Constraints above, returns the script's result converted back to `serde_json::Value`.

- [ ] **Step 1: Add the dependency**

```toml
# Cargo.toml, in [dependencies]
rquickjs = "0.6"
```

- [ ] **Step 2: Write failing tests**

```rust
// src/expr.rs, tests module
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
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib expr::tests`
Expected: compile error, `EvalContext`/`eval_js` undefined.

- [ ] **Step 3: Implement (reference implementation — verify against the installed `rquickjs` API per the Global Constraints note above)**

```rust
// top of src/expr.rs, above the tests module
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct EvalContext<'a> {
    pub json: serde_json::Value,
    pub items: &'a [serde_json::Value],
    pub node_json: &'a HashMap<String, serde_json::Value>,
    pub workflow_name: &'a str,
}

#[derive(Debug, thiserror::Error)]
pub enum ExprError {
    #[error("expression runtime error: {0}")]
    Runtime(String),
    #[error("expression value conversion error: {0}")]
    Conversion(String),
}

pub fn eval_js(script: &str, ctx: &EvalContext) -> Result<serde_json::Value, ExprError> {
    let runtime = rquickjs::Runtime::new().map_err(|e| ExprError::Runtime(e.to_string()))?;
    let js_context = rquickjs::Context::full(&runtime).map_err(|e| ExprError::Runtime(e.to_string()))?;

    js_context.with(|js| -> Result<serde_json::Value, ExprError> {
        let globals = js.globals();

        let json_val = json_to_js(&js, &ctx.json).map_err(|e| ExprError::Conversion(e.to_string()))?;
        globals.set("$json", json_val).map_err(|e| ExprError::Runtime(e.to_string()))?;

        let items_val = json_to_js(
            &js,
            &serde_json::Value::Array(
                ctx.items.iter().map(|j| serde_json::json!({"json": j})).collect(),
            ),
        )
        .map_err(|e| ExprError::Conversion(e.to_string()))?;
        // Bind as a zero-argument function returning the precomputed array, per
        // the $items() call syntax r8r's expressions use.
        let items_fn = rquickjs::Function::new(js.clone(), move || items_val.clone())
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        globals.set("$items", items_fn).map_err(|e| ExprError::Runtime(e.to_string()))?;

        let node_obj = rquickjs::Object::new(js.clone()).map_err(|e| ExprError::Runtime(e.to_string()))?;
        for (name, json) in ctx.node_json.iter() {
            let entry = rquickjs::Object::new(js.clone()).map_err(|e| ExprError::Runtime(e.to_string()))?;
            let json_js = json_to_js(&js, json).map_err(|e| ExprError::Conversion(e.to_string()))?;
            entry.set("json", json_js).map_err(|e| ExprError::Runtime(e.to_string()))?;
            node_obj.set(name.as_str(), entry).map_err(|e| ExprError::Runtime(e.to_string()))?;
        }
        globals.set("$node", node_obj).map_err(|e| ExprError::Runtime(e.to_string()))?;

        globals
            .set("$now", chrono::Utc::now().to_rfc3339())
            .map_err(|e| ExprError::Runtime(e.to_string()))?;

        let workflow_obj = rquickjs::Object::new(js.clone()).map_err(|e| ExprError::Runtime(e.to_string()))?;
        workflow_obj
            .set("name", ctx.workflow_name)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        globals.set("$workflow", workflow_obj).map_err(|e| ExprError::Runtime(e.to_string()))?;

        let result: rquickjs::Value = js
            .eval(script)
            .map_err(|e| ExprError::Runtime(e.to_string()))?;
        js_to_json(result).map_err(|e| ExprError::Conversion(e.to_string()))
    })
}

fn json_to_js<'js>(
    ctx: &rquickjs::Ctx<'js>,
    value: &serde_json::Value,
) -> Result<rquickjs::Value<'js>, String> {
    match value {
        serde_json::Value::Null => Ok(rquickjs::Value::new_null(ctx.clone())),
        serde_json::Value::Bool(b) => Ok(rquickjs::Value::new_bool(ctx.clone(), *b)),
        serde_json::Value::Number(n) => {
            Ok(rquickjs::Value::new_number(ctx.clone(), n.as_f64().unwrap_or(0.0)))
        }
        serde_json::Value::String(s) => rquickjs::String::from_str(ctx.clone(), s)
            .map(|v| v.into_value())
            .map_err(|e| e.to_string()),
        serde_json::Value::Array(items) => {
            let arr = rquickjs::Array::new(ctx.clone()).map_err(|e| e.to_string())?;
            for (i, item) in items.iter().enumerate() {
                arr.set(i, json_to_js(ctx, item)?).map_err(|e| e.to_string())?;
            }
            Ok(arr.into_value())
        }
        serde_json::Value::Object(map) => {
            let obj = rquickjs::Object::new(ctx.clone()).map_err(|e| e.to_string())?;
            for (k, v) in map.iter() {
                obj.set(k.as_str(), json_to_js(ctx, v)?).map_err(|e| e.to_string())?;
            }
            Ok(obj.into_value())
        }
    }
}

fn js_to_json(value: rquickjs::Value) -> Result<serde_json::Value, String> {
    if value.is_null() || value.is_undefined() {
        Ok(serde_json::Value::Null)
    } else if value.is_bool() {
        Ok(serde_json::Value::Bool(value.as_bool().unwrap_or(false)))
    } else if value.is_number() {
        Ok(serde_json::json!(value.as_float().unwrap_or(0.0)))
    } else if value.is_string() {
        let s = value
            .as_string()
            .ok_or("expected string")?
            .to_string()
            .map_err(|e| e.to_string())?;
        Ok(serde_json::Value::String(s))
    } else if value.is_array() {
        let arr = value.as_array().ok_or("expected array")?;
        let mut out = Vec::new();
        for item in arr.iter::<rquickjs::Value>() {
            out.push(js_to_json(item.map_err(|e| e.to_string())?)?);
        }
        Ok(serde_json::Value::Array(out))
    } else if value.is_object() {
        let obj = value.as_object().ok_or("expected object")?;
        let mut map = serde_json::Map::new();
        for key in obj.keys::<String>() {
            let key = key.map_err(|e| e.to_string())?;
            let v: rquickjs::Value = obj.get(&key).map_err(|e| e.to_string())?;
            map.insert(key, js_to_json(v)?);
        }
        Ok(serde_json::Value::Object(map))
    } else {
        Err("unsupported JS value type in expression result".to_string())
    }
}
```

Add `pub mod expr;` to `src/lib.rs`.

- [ ] **Step 4: Run tests, adapting the implementation to the installed `rquickjs` API as needed**

Run: `cargo build` first to surface any API mismatches from Step 3, fix them (consulting the installed crate's actual types/methods), then:

Run: `cargo test --lib expr::tests`
Expected: all four tests PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml src/expr.rs src/lib.rs
git commit -m "feat: embed QuickJS expression engine (eval_js)"
```

---

### Task 2: `$node`/`$items`/`$now`/`$workflow` coverage + `{{ }}` interpolation + `resolve_parameters`

**Files:**
- Modify: `src/expr.rs`

**Interfaces:**
- Consumes: `eval_js`, `EvalContext` from Task 1.
- Produces: `r8r::expr::resolve_parameters(params: &serde_json::Value, ctx: &EvalContext) -> Result<serde_json::Value, ExprError>` — recursively walks `params`; for each string value:
  - if the *entire* string is a single `{{ ... }}` expression (optionally surrounded by whitespace), replace it with the expression's *raw* evaluated JSON value (so `"{{ 1 + 1 }}"` becomes the number `2`, not the string `"2"`);
  - if the string contains `{{ ... }}` mixed with other text, evaluate each expression, stringify its result (numbers/bools as their literal text, objects/arrays as compact JSON, `null` as empty string), and splice back into the surrounding text;
  - a string with no `{{ }}` at all passes through unchanged.
  Non-string values (numbers, bools, null, nested objects/arrays) recurse structurally; objects/arrays resolve every string leaf.

- [ ] **Step 1: Write failing tests for the remaining globals**

```rust
// add to the tests module in src/expr.rs
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
```

- [ ] **Step 2: Run, confirm pass (these should already pass from Task 1's implementation — this step is verification, not new implementation)**

Run: `cargo test --lib expr::tests`
Expected: all pass. If any fail, Task 1's globals wiring has a bug — fix it here before proceeding (this task owns full coverage of the globals).

- [ ] **Step 3: Write failing tests for `resolve_parameters`**

```rust
// add to the tests module in src/expr.rs
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
```

- [ ] **Step 4: Run, confirm compile failure**

Run: `cargo test --lib expr::tests::resolve_parameters`
Expected: compile error, `resolve_parameters` undefined.

- [ ] **Step 5: Implement**

```rust
// add to src/expr.rs, above the tests module

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
        let end = after_open.find("}}").ok_or_else(|| {
            ExprError::Runtime(format!("unterminated expression in: {s}"))
        })?;
        let expr_src = after_open[..end].trim();
        let value = eval_js(expr_src, ctx)?;
        result.push_str(&stringify_for_splice(&value));
        rest = &after_open[end + 2..];
    }
    result.push_str(rest);
    Ok(serde_json::Value::String(result))
}

fn stringify_for_splice(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib expr::tests`
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src/expr.rs
git commit -m "feat: add {{ }} interpolation and resolve_parameters"
```

---

### Task 3: `Node` trait upgrade to multi-output (`NodeOutput`)

**Files:**
- Modify: `src/node.rs`
- Modify: `src/nodes/manual_trigger.rs`
- Modify: `src/nodes/set.rs`

**Interfaces:**
- Produces: `r8r::node::NodeOutput` — type alias for `Vec<Vec<Item>>`, one `Vec<Item>` per named output port (index 0 = primary/success output for every node in this plan except If/Switch, which use more ports — see Tasks 7–8).
- Produces: `r8r::node::ERROR_OUTPUT: usize` — a sentinel constant (`usize::MAX`) identifying a node's *error* output in a `Connection.from_output`. Not an index into `NodeOutput` — routed separately by the engine (Task 6).
- Changes: `Node::execute`'s return type becomes `Result<NodeOutput, NodeError>` (was `Result<Vec<Item>, NodeError>`).

- [ ] **Step 1: Update the failing test for the registry's `EchoNode` and add a multi-output test**

```rust
// replace the EchoNode impl and its test in src/node.rs's tests module
struct EchoNode;

#[async_trait]
impl Node for EchoNode {
    fn type_name(&self) -> &'static str {
        "test.echo"
    }
    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![ctx.input_items.clone()])
    }
}

#[tokio::test]
async fn registry_dispatches_to_registered_node() {
    let mut registry = NodeRegistry::new();
    registry.register(Box::new(EchoNode));

    let node = registry.get("test.echo").expect("node should be registered");
    let ctx = NodeExecutionContext {
        parameters: serde_json::json!({}),
        input_items: vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }],
    };
    let result = node.execute(&ctx).await.unwrap();
    assert_eq!(result.len(), 1); // one output port
    assert_eq!(result[0][0].json, serde_json::json!({"x": 1}));
}

#[test]
fn registry_returns_none_for_unknown_type() {
    let registry = NodeRegistry::new();
    assert!(registry.get("does.not.exist").is_none());
}

#[test]
fn error_output_is_distinct_from_any_real_port_index() {
    // ERROR_OUTPUT must never collide with a legitimate small port index like 0, 1, 2.
    assert_ne!(ERROR_OUTPUT, 0);
    assert_ne!(ERROR_OUTPUT, 1);
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib node::tests`
Expected: compile error — `NodeOutput`/`ERROR_OUTPUT` undefined, `execute`'s return type mismatch.

- [ ] **Step 3: Implement the trait changes**

```rust
// in src/node.rs, replace the Node trait and add the new items
use crate::domain::Item;

pub type NodeOutput = Vec<Vec<Item>>;

/// Sentinel `Connection.from_output` value identifying a node's error output.
/// Never a valid index into a `NodeOutput`'s ports — routed separately by the
/// engine (see Task 6), not looked up via `NodeOutput`'s `Vec` indexing.
pub const ERROR_OUTPUT: usize = usize::MAX;

#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;
    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError>;
}
```

(`NodeExecutionContext`, `NodeError`, `NodeRegistry` are unchanged — only the trait's `execute` signature and the two new items above.)

- [ ] **Step 4: Update `ManualTriggerNode` and `SetNode` to the new return type**

```rust
// src/nodes/manual_trigger.rs — change the execute body's return
async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
    Ok(vec![vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]])
}
```

Add `use crate::node::NodeOutput;` to that file's imports.

```rust
// src/nodes/set.rs — change the execute signature and final return
async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
    // ... existing body unchanged down to building `items` ...
    Ok(vec![items])
}
```

Add `use crate::node::NodeOutput;` to that file's imports. Update that file's own tests to index into the new shape: e.g. `result[0]` (the port) `[0].json` (the first item) wherever they previously wrote `result[0].json`.

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: compiles; `src/engine.rs` will now fail to compile (its call sites still assume the old `Vec<Item>` shape) — that's expected and is Task 4/5/6's job to fix. For this task, confirm `cargo test --lib node::tests --lib nodes::` passes and note the expected `engine.rs` compile failure in your report; do not fix `engine.rs` here.

- [ ] **Step 6: Commit**

```bash
git add src/node.rs src/nodes/manual_trigger.rs src/nodes/set.rs
git commit -m "feat: upgrade Node::execute to multi-output NodeOutput"
```

---

### Task 4: Topological ordering (`topological_order` replaces `linear_order`)

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `Workflow`, `NodeInstance`, `Connection` from `domain.rs` (unchanged).
- Produces: `topological_order(workflow: &Workflow) -> anyhow::Result<Vec<NodeInstance>>` (private `fn`, same visibility as the `linear_order` it replaces) — a valid topological order over `workflow.nodes`/`workflow.connections` via Kahn's algorithm. Still requires exactly one node with no incoming connections (the start node) when there is more than one node. Detects and rejects: a dangling `from_node`/`to_node` (references a node id not in `workflow.nodes`), and a genuine cycle (fewer nodes reachable than exist). Unlike the `linear_order` it replaces, this function does **not** reject a node with more than one outgoing connection — branching is now legal.
- This task only replaces the *ordering* function; `execute_workflow` itself is rewired in Task 5 (it will not compile after this task alone — that's expected, same pattern as Task 3).

- [ ] **Step 1: Write failing tests for the new ordering function directly**

```rust
// add to the tests module in src/engine.rs (keep the existing linear_workflow/registry helpers)
#[test]
fn topological_order_handles_simple_linear_chain() {
    let wf = linear_workflow();
    let order = topological_order(&wf).unwrap();
    assert_eq!(order.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["trigger", "set1"]);
}

#[test]
fn topological_order_allows_branching() {
    let mut wf = linear_workflow();
    wf.nodes.push(NodeInstance {
        id: "set2".into(),
        node_type: "core.set".into(),
        position: (2.0, 0.0),
        parameters: serde_json::json!({}),
        disabled: false,
    });
    wf.connections.push(Connection {
        from_node: "trigger".into(),
        from_output: 0,
        to_node: "set2".into(),
        to_input: 0,
    });
    let order = topological_order(&wf).unwrap();
    assert_eq!(order[0].id, "trigger");
    let rest: std::collections::HashSet<&str> = order[1..].iter().map(|n| n.id.as_str()).collect();
    assert_eq!(rest, std::collections::HashSet::from(["set1", "set2"]));
}

#[test]
fn topological_order_detects_cycle_downstream_of_valid_start() {
    let mut wf = linear_workflow();
    wf.nodes[1].id = "b".into();
    wf.nodes.push(NodeInstance {
        id: "c".into(),
        node_type: "core.set".into(),
        position: (2.0, 0.0),
        parameters: serde_json::json!({}),
        disabled: false,
    });
    wf.connections.clear();
    wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "b".into(), to_input: 0 });
    wf.connections.push(Connection { from_node: "b".into(), from_output: 0, to_node: "c".into(), to_input: 0 });
    wf.connections.push(Connection { from_node: "c".into(), from_output: 0, to_node: "b".into(), to_input: 0 });

    let result = topological_order(&wf);
    assert!(result.is_err());
}

#[test]
fn topological_order_rejects_dangling_connection_endpoints() {
    let mut wf = linear_workflow();
    wf.connections.push(Connection { from_node: "ghost".into(), from_output: 0, to_node: "set1".into(), to_input: 0 });
    let result = topological_order(&wf);
    assert!(result.is_err());
}

#[test]
fn topological_order_rejects_multiple_start_candidates() {
    let mut wf = linear_workflow();
    wf.nodes.push(NodeInstance {
        id: "trigger2".into(),
        node_type: "core.manualTrigger".into(),
        position: (0.0, 1.0),
        parameters: serde_json::json!({}),
        disabled: false,
    });
    let result = topological_order(&wf);
    assert!(result.is_err());
}

#[test]
fn topological_order_handles_empty_workflow() {
    let mut wf = linear_workflow();
    wf.nodes.clear();
    wf.connections.clear();
    let order = topological_order(&wf).unwrap();
    assert!(order.is_empty());
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib engine::tests::topological_order`
Expected: compile error, `topological_order` undefined (only `linear_order` exists so far).

- [ ] **Step 3: Implement, replacing `linear_order` entirely**

```rust
// src/engine.rs — replace the whole `linear_order` function with this
use std::collections::VecDeque;

fn topological_order(workflow: &Workflow) -> anyhow::Result<Vec<NodeInstance>> {
    if workflow.nodes.is_empty() {
        return Ok(Vec::new());
    }

    for conn in &workflow.connections {
        if !workflow.nodes.iter().any(|n| n.id == conn.from_node) {
            return Err(anyhow::anyhow!("connection references unknown from_node {}", conn.from_node));
        }
        if !workflow.nodes.iter().any(|n| n.id == conn.to_node) {
            return Err(anyhow::anyhow!("connection references unknown to_node {}", conn.to_node));
        }
    }

    let targets: HashSet<&str> = workflow.connections.iter().map(|c| c.to_node.as_str()).collect();
    let start_candidates: Vec<&str> = workflow
        .nodes
        .iter()
        .map(|n| n.id.as_str())
        .filter(|id| !targets.contains(id))
        .collect();
    if workflow.nodes.len() > 1 && start_candidates.len() != 1 {
        return Err(anyhow::anyhow!(
            "expected exactly one start node, found {}",
            start_candidates.len()
        ));
    }

    let position_of = |id: &str| workflow.nodes.iter().position(|n| n.id == id).unwrap();

    let mut in_degree: HashMap<&str, usize> = workflow.nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut seen_edges: HashSet<(&str, &str)> = HashSet::new();
    for conn in &workflow.connections {
        if seen_edges.insert((conn.from_node.as_str(), conn.to_node.as_str())) {
            adjacency.entry(conn.from_node.as_str()).or_default().push(conn.to_node.as_str());
            *in_degree.entry(conn.to_node.as_str()).or_insert(0) += 1;
        }
    }

    let mut ready: Vec<&str> = in_degree.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
    ready.sort_by_key(|id| position_of(id));
    let mut queue: VecDeque<&str> = ready.into();

    let mut order_ids: Vec<String> = Vec::new();
    while let Some(id) = queue.pop_front() {
        order_ids.push(id.to_string());
        if let Some(next_ids) = adjacency.get(id) {
            let mut newly_ready: Vec<&str> = Vec::new();
            for &next in next_ids {
                let degree = in_degree.get_mut(next).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    newly_ready.push(next);
                }
            }
            newly_ready.sort_by_key(|id| position_of(id));
            for id in newly_ready {
                queue.push_back(id);
            }
        }
    }

    if order_ids.len() != workflow.nodes.len() {
        return Err(anyhow::anyhow!(
            "cycle detected: only {} of {} nodes are reachable via a valid topological order",
            order_ids.len(),
            workflow.nodes.len()
        ));
    }

    Ok(order_ids
        .into_iter()
        .map(|id| workflow.nodes.iter().find(|n| n.id == id).unwrap().clone())
        .collect())
}
```

`execute_workflow` (above this function) and several existing `#[tokio::test]`s in this file still reference `linear_order` and the old single-output execution shape — leave them exactly as broken as they are; do not attempt to fix `execute_workflow` or the pre-existing engine tests in this task. That is Task 5's job.

- [ ] **Step 4: Run the new tests directly**

Run: `cargo test --lib engine::tests::topological_order`
Expected: all six new tests PASS. (The file as a whole will not compile yet — `cargo test` for the whole crate will fail because `execute_workflow` still calls the now-deleted `linear_order` and still assumes the pre-Task-3 `Vec<Item>` shape. That's expected; report it, don't fix it.)

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs
git commit -m "feat: replace linear_order with topological_order (Kahn's algorithm)"
```

---

### Task 5: Rewrite `execute_workflow` for multi-input aggregation, multi-output routing, and expression resolution

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `topological_order` (Task 4), `NodeOutput`/`ERROR_OUTPUT` (Task 3), `expr::resolve_parameters`/`expr::EvalContext` (Tasks 1–2).
- Produces: `execute_workflow(workflow: &Workflow, registry: &NodeRegistry) -> anyhow::Result<HashMap<String, Vec<Item>>>` — **signature unchanged from Foundation** (so `src/api/workflows.rs` needs zero changes). Internally: walks nodes in topological order; a node's `input_items` is the concatenation of items from every incoming connection (pulling each upstream node's items from the specific `from_output` port that connection names); resolves the node's parameters via `expr::resolve_parameters` before calling `execute()` (with `$json` bound to the first input item, `$items` to all input items, `$node` to every already-executed node's primary-output first item, `$workflow` to the workflow's name); persists/returns only each node's **primary (port 0)** output, flattened to the pre-existing `HashMap<String, Vec<Item>>` shape.
- Error-output routing is a separate task (Task 6) — for this task, a node `execute()` error still aborts the whole run with `Err`, exactly like Foundation. Do not implement `ERROR_OUTPUT` routing yet; just make sure the new multi-output/multi-input plumbing compiles and passes Foundation's original test intent under the new shape.

- [ ] **Step 1: Update the existing tests in `src/engine.rs` to the new multi-output assertion shape**

Every existing `#[tokio::test]` in this file's tests module that indexes `outputs["node_id"][0].json` (Foundation's `Vec<Item>` shape) now needs `outputs["node_id"][0].json` — **wait, this doesn't change**: the *public* `execute_workflow` return type is still `HashMap<String, Vec<Item>>` (flattened), so none of Foundation's or Task 4's existing assertions on `execute_workflow`'s return value need to change at all. Only the *internal* per-node dispatch changed shape. Confirm this by re-reading the existing tests (`executes_trigger_then_set_in_order`, `empty_workflow_produces_empty_outputs`, `unknown_node_type_returns_error`, `node_with_two_outgoing_connections_returns_error` — this one's assertion must change, see below, `disconnected_components_return_error`, `cycle_downstream_of_valid_start_returns_error_and_terminates`, `dangling_from_node_returns_error`, `disabled_node_is_skipped_as_passthrough`) — no line changes needed except:

```rust
// node_with_two_outgoing_connections_returns_error no longer applies: branching
// is now legal. DELETE this test entirely (its premise contradicts Task 4's
// topological_order, which explicitly allows multiple outgoing connections).
```

Delete `node_with_two_outgoing_connections_returns_error` from the tests module. Add one replacement test proving branching now *works* rather than erroring:

```rust
// add to the tests module in src/engine.rs
#[tokio::test]
async fn branching_workflow_executes_both_downstream_nodes() {
    let mut wf = linear_workflow();
    wf.nodes.push(NodeInstance {
        id: "set2".into(),
        node_type: "core.set".into(),
        position: (2.0, 0.0),
        parameters: serde_json::json!({"fields": {"other": "value"}}),
        disabled: false,
    });
    wf.connections.push(Connection {
        from_node: "trigger".into(),
        from_output: 0,
        to_node: "set2".into(),
        to_input: 0,
    });
    let outputs = execute_workflow(&wf, &registry()).await.unwrap();
    assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
    assert_eq!(outputs["set2"][0].json, serde_json::json!({"other": "value"}));
}

#[tokio::test]
async fn node_with_two_incoming_connections_receives_both_upstream_outputs() {
    // trigger -> set1 (adds greeting), trigger -> set2 (adds other),
    // set1 -> set3, set2 -> set3: set3 should see items carrying BOTH fields
    // aggregated from its two incoming connections.
    let mut wf = linear_workflow();
    wf.nodes.push(NodeInstance {
        id: "set2".into(),
        node_type: "core.set".into(),
        position: (2.0, 0.0),
        parameters: serde_json::json!({"fields": {"other": "value"}}),
        disabled: false,
    });
    wf.nodes.push(NodeInstance {
        id: "set3".into(),
        node_type: "core.set".into(),
        position: (3.0, 0.0),
        parameters: serde_json::json!({}),
        disabled: false,
    });
    wf.connections.push(Connection { from_node: "trigger".into(), from_output: 0, to_node: "set2".into(), to_input: 0 });
    wf.connections.push(Connection { from_node: "set1".into(), from_output: 0, to_node: "set3".into(), to_input: 0 });
    wf.connections.push(Connection { from_node: "set2".into(), from_output: 0, to_node: "set3".into(), to_input: 0 });

    let outputs = execute_workflow(&wf, &registry()).await.unwrap();
    // set3 received one item from each upstream branch.
    assert_eq!(outputs["set3"].len(), 2);
}

#[tokio::test]
async fn expression_in_parameters_is_resolved_before_node_execution() {
    let mut wf = linear_workflow();
    wf.nodes[1].parameters = serde_json::json!({"fields": {"doubled": "{{ 21 * 2 }}"}});
    let outputs = execute_workflow(&wf, &registry()).await.unwrap();
    assert_eq!(outputs["set1"][0].json, serde_json::json!({"doubled": 42}));
}
```

- [ ] **Step 2: Run, confirm failures**

Run: `cargo test --lib engine::tests`
Expected: compile errors / failures — `execute_workflow` still calls the deleted `linear_order` and assumes the old single-output shape internally.

- [ ] **Step 3: Rewrite `execute_workflow`**

```rust
// src/engine.rs — replace the whole execute_workflow function
pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = topological_order(workflow)?;
    let mut produced: HashMap<String, crate::node::NodeOutput> = HashMap::new();

    for node_instance in &order {
        let mut input_items: Vec<Item> = Vec::new();
        for conn in &workflow.connections {
            if conn.to_node != node_instance.id {
                continue;
            }
            if let Some(outputs) = produced.get(&conn.from_node) {
                if let Some(port_items) = outputs.get(conn.from_output) {
                    input_items.extend(port_items.iter().cloned());
                }
            }
        }

        if node_instance.disabled {
            produced.insert(node_instance.id.clone(), vec![input_items]);
            continue;
        }

        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;

        let items_json: Vec<serde_json::Value> = input_items.iter().map(|i| i.json.clone()).collect();
        let node_json: HashMap<String, serde_json::Value> = produced
            .iter()
            .filter_map(|(id, ports)| {
                let name = workflow.nodes.iter().find(|n| &n.id == id).map(|n| n.id.clone())?;
                let first_item_json = ports.first().and_then(|p| p.first()).map(|item| item.json.clone());
                first_item_json.map(|j| (name, j))
            })
            .collect();
        let eval_ctx = crate::expr::EvalContext {
            json: input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})),
            items: &items_json,
            node_json: &node_json,
            workflow_name: &workflow.name,
        };
        let resolved_parameters = crate::expr::resolve_parameters(&node_instance.parameters, &eval_ctx)
            .map_err(|e| anyhow::anyhow!("node {} parameter resolution failed: {e}", node_instance.id))?;

        let ctx = NodeExecutionContext {
            parameters: resolved_parameters,
            input_items,
        };
        let output = node
            .execute(&ctx)
            .await
            .map_err(|e| anyhow::anyhow!("node {} failed: {e}", node_instance.id))?;
        produced.insert(node_instance.id.clone(), output);
    }

    let flattened = order
        .into_iter()
        .map(|n| {
            let items = produced
                .get(&n.id)
                .and_then(|ports| ports.first())
                .cloned()
                .unwrap_or_default();
            (n.id, items)
        })
        .collect();
    Ok(flattened)
}
```

- [ ] **Step 4: Run the full engine test suite**

Run: `cargo test --lib engine::tests`
Expected: all tests PASS, including the pre-existing Foundation tests (unchanged assertions, per Step 1's note) and the new ones from this task.

- [ ] **Step 5: Run the whole crate**

Run: `cargo test`
Expected: everything PASSES now — `src/api/workflows.rs`'s call site needed no changes since `execute_workflow`'s signature is unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/engine.rs
git commit -m "feat: rewrite execute_workflow for multi-input/multi-output DAG execution"
```

---

### Task 6: Error-output routing

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Consumes: `ERROR_OUTPUT` (Task 3).
- Changes: `execute_workflow`'s error-handling — when a node's `execute()` returns `Err`, check whether any `Connection` has `from_node == node_instance.id && from_output == ERROR_OUTPUT`. If yes: synthesize a single error `Item` (`json: {"error": "<message>"}`), route it to whichever downstream node(s) that error connection targets (as if it were a normal output port, just looked up via a side channel rather than `NodeOutput` indexing — `ERROR_OUTPUT` is `usize::MAX` and cannot be a `Vec` index), and continue the run. If no such connection exists, abort the whole run with `Err` — same as Task 5's (and Foundation's) behavior.

- [ ] **Step 1: Write failing tests**

```rust
// add to the tests module in src/engine.rs
struct AlwaysFailsNode;

#[async_trait::async_trait]
impl crate::node::Node for AlwaysFailsNode {
    fn type_name(&self) -> &'static str {
        "test.alwaysFails"
    }
    async fn execute(&self, _ctx: &crate::node::NodeExecutionContext) -> Result<crate::node::NodeOutput, crate::node::NodeError> {
        Err(crate::node::NodeError::ExecutionFailed("boom".into()))
    }
}

fn registry_with_failing_node() -> NodeRegistry {
    let mut r = registry();
    r.register(Box::new(AlwaysFailsNode));
    r
}

#[tokio::test]
async fn error_without_connected_error_route_aborts_the_run() {
    let mut wf = linear_workflow();
    wf.nodes[1].node_type = "test.alwaysFails".into();
    let result = execute_workflow(&wf, &registry_with_failing_node()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn error_with_connected_error_route_continues_and_routes_error_item() {
    let mut wf = linear_workflow();
    wf.nodes[1].node_type = "test.alwaysFails".into();
    wf.nodes.push(NodeInstance {
        id: "error_handler".into(),
        node_type: "core.set".into(),
        position: (2.0, 0.0),
        parameters: serde_json::json!({"fields": {"handled": true}}),
        disabled: false,
    });
    wf.connections.push(Connection {
        from_node: "set1".into(),
        from_output: crate::node::ERROR_OUTPUT,
        to_node: "error_handler".into(),
        to_input: 0,
    });

    let outputs = execute_workflow(&wf, &registry_with_failing_node()).await.unwrap();
    assert_eq!(outputs["error_handler"][0].json["handled"], serde_json::json!(true));
    // set1 itself produced no primary-output items (it errored).
    assert!(outputs["set1"].is_empty());
}
```

- [ ] **Step 2: Run, confirm the first test passes already (Task 5 behavior) and the second fails**

Run: `cargo test --lib engine::tests::error_`
Expected: `error_without_connected_error_route_aborts_the_run` PASSES already; `error_with_connected_error_route_continues_and_routes_error_item` FAILS (whole run currently aborts regardless of a connected error route).

- [ ] **Step 3: Implement**

```rust
// src/engine.rs — inside execute_workflow's loop, replace the `.execute().await.map_err(...)`
// line and the `produced.insert(...)` line that follows it with:
let mut error_produced: HashMap<String, Vec<Item>> = HashMap::new();
// ^ declare this once, alongside `let mut produced = ...` near the top of the function

match node.execute(&ctx).await {
    Ok(output) => {
        produced.insert(node_instance.id.clone(), output);
    }
    Err(e) => {
        let has_error_route = workflow.connections.iter().any(|c| {
            c.from_node == node_instance.id && c.from_output == crate::node::ERROR_OUTPUT
        });
        if has_error_route {
            error_produced.insert(
                node_instance.id.clone(),
                vec![Item {
                    json: serde_json::json!({ "error": e.to_string() }),
                    binary: serde_json::json!({}),
                }],
            );
            produced.insert(node_instance.id.clone(), vec![]);
        } else {
            return Err(anyhow::anyhow!("node {} failed: {e}", node_instance.id));
        }
    }
}
```

Also update the input-aggregation loop (earlier in the same function, where `input_items` is built from incoming connections) to check `error_produced` when a connection's `from_output == ERROR_OUTPUT`:

```rust
// src/engine.rs — replace the input-aggregation block inside the main loop
let mut input_items: Vec<Item> = Vec::new();
for conn in &workflow.connections {
    if conn.to_node != node_instance.id {
        continue;
    }
    if conn.from_output == crate::node::ERROR_OUTPUT {
        if let Some(items) = error_produced.get(&conn.from_node) {
            input_items.extend(items.iter().cloned());
        }
    } else if let Some(outputs) = produced.get(&conn.from_node) {
        if let Some(port_items) = outputs.get(conn.from_output) {
            input_items.extend(port_items.iter().cloned());
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib engine::tests`
Expected: all PASS, including both new tests.

- [ ] **Step 5: Run the whole crate**

Run: `cargo test`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/engine.rs
git commit -m "feat: route a node's error output to a connected downstream node"
```

---

### Task 7: If node

**Files:**
- Create: `src/nodes/if_node.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::if_node::IfNode`, `type_name() == "core.if"`. Parameters shape: `{"condition": "<expression string, already resolved to a JSON boolean by the engine before execute() runs>"}`. Two output ports: index 0 = all input items when the condition is truthy, index 1 = all input items when falsy. The condition itself is a single resolved value in `ctx.parameters.condition` (a `bool`, since expression resolution already ran) — this node does **not** call `expr::eval_js` itself; it only inspects the already-resolved parameter value, keeping it decoupled from the expression engine (consistent with every other node).

- [ ] **Step 1: Write failing tests**

```rust
// src/nodes/if_node.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct IfNode;

#[async_trait]
impl Node for IfNode {
    fn type_name(&self) -> &'static str {
        "core.if"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = ctx
            .parameters
            .get("condition")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if condition {
            Ok(vec![ctx.input_items.clone(), vec![]])
        } else {
            Ok(vec![vec![], ctx.input_items.clone()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }]
    }

    #[tokio::test]
    async fn true_condition_routes_to_output_zero() {
        let node = IfNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": true}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items());
        assert!(result[1].is_empty());
    }

    #[tokio::test]
    async fn false_condition_routes_to_output_one() {
        let node = IfNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"condition": false}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }

    #[tokio::test]
    async fn missing_condition_defaults_to_false() {
        let node = IfNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::if_node`
Expected: compile error (module not wired into `mod.rs` yet).

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod if_node;
// (add alongside the existing `pub mod manual_trigger;` / `pub mod set;` lines)
```

In `register_all`, add:

```rust
registry.register(Box::new(if_node::IfNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::if_node`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/if_node.rs src/nodes/mod.rs
git commit -m "feat: add If node"
```

---

### Task 8: Switch node

**Files:**
- Create: `src/nodes/switch.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::switch::SwitchNode`, `type_name() == "core.switch"`. Parameters shape: `{"value": <already-resolved JSON value>, "cases": [<JSON value>, <JSON value>, ...]}`. Output ports: one per entry in `cases` (index = position in the `cases` array) plus one trailing default/fallthrough port (index = `cases.len()`) for when `value` doesn't equal any case. All input items go to the single matching port; every other port (including the default, when a case matched) is empty.

- [ ] **Step 1: Write failing tests**

```rust
// src/nodes/switch.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct SwitchNode;

#[async_trait]
impl Node for SwitchNode {
    fn type_name(&self) -> &'static str {
        "core.switch"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let value = ctx.parameters.get("value").cloned().unwrap_or(serde_json::Value::Null);
        let cases = ctx
            .parameters
            .get("cases")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let matched_index = cases.iter().position(|c| *c == value);
        let mut ports: Vec<Vec<Item>> = vec![Vec::new(); cases.len() + 1];
        let target = matched_index.unwrap_or(cases.len());
        ports[target] = ctx.input_items.clone();
        Ok(ports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }]
    }

    #[tokio::test]
    async fn matching_case_routes_to_its_port() {
        let node = SwitchNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"value": "b", "cases": ["a", "b", "c"]}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 4); // 3 cases + 1 default
        assert!(result[0].is_empty());
        assert_eq!(result[1], items());
        assert!(result[2].is_empty());
        assert!(result[3].is_empty());
    }

    #[tokio::test]
    async fn no_matching_case_routes_to_default_port() {
        let node = SwitchNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"value": "z", "cases": ["a", "b"]}),
            input_items: items(),
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 3); // 2 cases + 1 default
        assert!(result[0].is_empty());
        assert!(result[1].is_empty());
        assert_eq!(result[2], items());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::switch`
Expected: compile error.

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod switch;
```

In `register_all`, add:

```rust
registry.register(Box::new(switch::SwitchNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::switch`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/switch.rs src/nodes/mod.rs
git commit -m "feat: add Switch node"
```

---

### Task 9: Merge node (append / mergeByKey / waitForAll modes)

**Files:**
- Create: `src/nodes/merge.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::merge::MergeNode`, `type_name() == "core.merge"`. Parameters shape: `{"mode": "append" | "mergeByKey" | "waitForAll", "key": "<field name, mergeByKey only>"}`.
  - `append` (default when `mode` is missing/unrecognized): pass all input items through unchanged — the engine's own input-aggregation (Task 5) already concatenated every upstream branch's items into `ctx.input_items`, so "append" mode's whole job is a no-op passthrough. This is the mode that actually realizes "wait for all branches" at the *data* level, since `ctx.input_items` isn't built until every upstream connection this node has has been executed (topological order guarantees that).
  - `mergeByKey`: group `ctx.input_items` by the value at `key` in each item's `json`; for items sharing a key, shallow-merge their `json` objects together (later items in `ctx.input_items` order win on conflicting fields) into one combined item per distinct key value.
  - `waitForAll`: identical behavior to `append` for this node's purposes (the actual "waiting" is structural — guaranteed by the engine only calling `execute()` once all incoming connections' upstream nodes are done — there is nothing left for the node itself to do differently). Included as a distinct, explicit mode name for parameter-schema clarity/n8n-familiarity, not because its runtime behavior differs from `append`.
  Single output port (index 0).

- [ ] **Step 1: Write failing tests**

```rust
// src/nodes/merge.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct MergeNode;

#[async_trait]
impl Node for MergeNode {
    fn type_name(&self) -> &'static str {
        "core.merge"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let mode = ctx.parameters.get("mode").and_then(|v| v.as_str()).unwrap_or("append");

        match mode {
            "mergeByKey" => {
                let key = ctx
                    .parameters
                    .get("key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| NodeError::ExecutionFailed("mergeByKey mode requires a \"key\" parameter".into()))?;

                let mut order: Vec<serde_json::Value> = Vec::new();
                let mut grouped: std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>> =
                    std::collections::HashMap::new();
                for item in &ctx.input_items {
                    let key_value = item.json.get(key).cloned().unwrap_or(serde_json::Value::Null);
                    let key_str = key_value.to_string();
                    let obj = item.json.as_object().cloned().unwrap_or_default();
                    let entry = grouped.entry(key_str.clone()).or_insert_with(|| {
                        order.push(key_value.clone());
                        serde_json::Map::new()
                    });
                    for (k, v) in obj {
                        entry.insert(k, v);
                    }
                }
                let merged_items = order
                    .into_iter()
                    .map(|key_value| {
                        let obj = grouped.remove(&key_value.to_string()).unwrap_or_default();
                        Item { json: serde_json::Value::Object(obj), binary: serde_json::json!({}) }
                    })
                    .collect();
                Ok(vec![merged_items])
            }
            _ => Ok(vec![ctx.input_items.clone()]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn append_mode_passes_items_through_unchanged() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"b": 2}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "append"}), input_items: items.clone() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn default_mode_is_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items.clone() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn wait_for_all_mode_behaves_like_append() {
        let node = MergeNode;
        let items = vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"mode": "waitForAll"}), input_items: items.clone() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn merge_by_key_combines_items_sharing_a_key_value() {
        let node = MergeNode;
        let items = vec![
            Item { json: serde_json::json!({"id": 1, "name": "Ada"}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 1, "age": 30}), binary: serde_json::json!({}) },
            Item { json: serde_json::json!({"id": 2, "name": "Grace"}), binary: serde_json::json!({}) },
        ];
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey", "key": "id"}),
            input_items: items,
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].len(), 2);
        assert_eq!(result[0][0].json, serde_json::json!({"id": 1, "name": "Ada", "age": 30}));
        assert_eq!(result[0][1].json, serde_json::json!({"id": 2, "name": "Grace"}));
    }

    #[tokio::test]
    async fn merge_by_key_without_key_parameter_returns_error() {
        let node = MergeNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"mode": "mergeByKey"}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::merge`
Expected: compile error.

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod merge;
```

In `register_all`, add:

```rust
registry.register(Box::new(merge::MergeNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::merge`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/merge.rs src/nodes/mod.rs
git commit -m "feat: add Merge node (append/mergeByKey/waitForAll)"
```

---

### Task 10: Filter, Wait, NoOp/Sticky nodes

**Files:**
- Create: `src/nodes/filter.rs`
- Create: `src/nodes/wait.rs`
- Create: `src/nodes/noop.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::filter::FilterNode` (`type_name() == "core.filter"`) — parameters `{"keep": [<already-resolved bool per item, see note below>]}`. Because expression resolution (Task 5) runs once per *node*, not once per *item* (Global Constraints), a per-item boolean condition can't come from a single resolved parameter. For this plan's scope, `FilterNode`'s condition is instead a single node-level boolean (`{"condition": true|false}`, same shape as If): all items pass through when `condition` is true, none do when false. (Genuine per-item filtering predicated on each item's own `$json` is deferred alongside the rest of per-item expression evaluation — see Global Constraints.)
- Produces: `r8r::nodes::wait::WaitNode` (`type_name() == "core.wait"`) — parameters `{"seconds": <number>}`; sleeps via `tokio::time::sleep` for that duration, then passes all input items through unchanged.
- Produces: `r8r::nodes::noop::NoOpNode` (`type_name() == "core.noop"`) — passes all input items through unchanged, no parameters.

- [ ] **Step 1: Write failing tests for all three**

```rust
// src/nodes/filter.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct FilterNode;

#[async_trait]
impl Node for FilterNode {
    fn type_name(&self) -> &'static str {
        "core.filter"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let condition = ctx.parameters.get("condition").and_then(|v| v.as_bool()).unwrap_or(false);
        if condition {
            Ok(vec![ctx.input_items.clone()])
        } else {
            Ok(vec![Vec::new()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<Item> {
        vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }]
    }

    #[tokio::test]
    async fn true_condition_keeps_all_items() {
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": true}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items());
    }

    #[tokio::test]
    async fn false_condition_drops_all_items() {
        let node = FilterNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"condition": false}), input_items: items() };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
```

```rust
// src/nodes/wait.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct WaitNode;

#[async_trait]
impl Node for WaitNode {
    fn type_name(&self) -> &'static str {
        "core.wait"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let seconds = ctx.parameters.get("seconds").and_then(|v| v.as_f64()).unwrap_or(0.0).max(0.0);
        tokio::time::sleep(std::time::Duration::from_secs_f64(seconds)).await;
        Ok(vec![ctx.input_items.clone()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passes_items_through_after_waiting() {
        let node = WaitNode;
        let items = vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({"seconds": 0.01}), input_items: items.clone() };
        let start = std::time::Instant::now();
        let result = node.execute(&ctx).await.unwrap();
        assert!(start.elapsed() >= std::time::Duration::from_millis(10));
        assert_eq!(result[0], items);
    }

    #[tokio::test]
    async fn missing_seconds_defaults_to_zero_wait() {
        let node = WaitNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await.unwrap();
        assert!(result[0].is_empty());
    }
}
```

```rust
// src/nodes/noop.rs
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;

pub struct NoOpNode;

#[async_trait]
impl Node for NoOpNode {
    fn type_name(&self) -> &'static str {
        "core.noop"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        Ok(vec![ctx.input_items.clone()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Item;

    #[tokio::test]
    async fn passes_items_through_unchanged() {
        let node = NoOpNode;
        let items = vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }];
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: items.clone() };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0], items);
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::filter --lib nodes::wait --lib nodes::noop`
Expected: compile errors.

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod filter;
pub mod wait;
pub mod noop;
```

In `register_all`, add:

```rust
registry.register(Box::new(filter::FilterNode));
registry.register(Box::new(wait::WaitNode));
registry.register(Box::new(noop::NoOpNode));
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib nodes::filter --lib nodes::wait --lib nodes::noop`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/filter.rs src/nodes/wait.rs src/nodes/noop.rs src/nodes/mod.rs
git commit -m "feat: add Filter, Wait, and NoOp nodes"
```

---

### Task 11: Code node

**Files:**
- Create: `src/nodes/code.rs`
- Modify: `src/nodes/mod.rs`

**Interfaces:**
- Produces: `r8r::nodes::code::CodeNode`, `type_name() == "core.code"`. Parameters shape: `{"script": "<JS source>"}`. Runs `script` with `items` bound as a JS array of `{json: ...}` objects (mirroring every input item), expects the script to evaluate to an array of the same shape (`[{json: ...}, ...]`) as its result; that array becomes the node's single output port. A per-run timeout (2 seconds) bounds runaway scripts — reuses `expr::eval_js` machinery by calling it directly with the script wrapped so `items` is available (this node does **not** go through `resolve_parameters`, since its "parameter" *is* the script to run, not a value to interpolate).

- [ ] **Step 1: Write failing tests**

Design note before the code: `EvalContext` (Task 1) borrows its fields (`&'a [Value]`, `&'a HashMap<...>`), so it cannot be moved into a `tokio::task::spawn_blocking` closure, which requires `'static` data. Rather than restructure `EvalContext` to own its data just for this one caller, this node evaluates synchronously (no `spawn_blocking`) and bounds runaway scripts with `tokio::time::timeout` instead — QuickJS is single-threaded, and a bounded synchronous eval inside an `async fn` is the same "call QuickJS synchronously" tradeoff already implicit in `expr::eval_js` since Task 1, not a new one this node introduces.

```rust
// src/nodes/code.rs -- replace the whole file with this (the tests module goes
// at the bottom as usual)
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

        let items_json: Vec<serde_json::Value> = ctx
            .input_items
            .iter()
            .map(|i| serde_json::json!({"json": i.json}))
            .collect();
        let empty_node_json: HashMap<String, serde_json::Value> = HashMap::new();
        let eval_ctx = EvalContext {
            json: ctx.input_items.first().map(|i| i.json.clone()).unwrap_or_else(|| serde_json::json!({})),
            items: &items_json,
            node_json: &empty_node_json,
            workflow_name: "",
        };
        let wrapped_script = format!("(function(items) {{ {script} }})($items())");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(CODE_TIMEOUT_SECS),
            async { eval_js(&wrapped_script, &eval_ctx) },
        )
        .await
        .map_err(|_| NodeError::ExecutionFailed(format!("core.code timed out after {CODE_TIMEOUT_SECS}s")))?
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
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib nodes::code`
Expected: compile error (module not wired in yet).

- [ ] **Step 3: Wire into `src/nodes/mod.rs`**

```rust
pub mod code;
```

In `register_all`, add:

```rust
registry.register(Box::new(code::CodeNode));
```

- [ ] **Step 4: Run tests, adapting to `rquickjs`'s actual API if `eval_js`/`EvalContext` needed adaptation back in Task 1**

Run: `cargo test --lib nodes::code`
Expected: all four PASS.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/code.rs src/nodes/mod.rs
git commit -m "feat: add Code node (full-script execution via QuickJS)"
```

---

### Task 12: End-to-end integration test — branching workflow through the real API

**Files:**
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: everything above, plus the existing `test_app()`/`register_and_get_token()` helpers already in `tests/api_test.rs` from Foundation.
- Produces: one new test proving the whole plan works end-to-end over real HTTP: create a workflow with a Manual Trigger → If (branches on an expression) → two Set nodes (one per branch) → Merge, execute it via the API, and assert the merged result reflects only the branch that actually ran.

- [ ] **Step 1: Write the failing test**

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn branching_workflow_with_if_and_merge_executes_end_to_end() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "branch@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "branch-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "branch", "node_type": "core.if", "position": [1.0, 0.0], "parameters": {"condition": "{{ 1 == 1 }}"}, "disabled": false},
            {"id": "true_branch", "node_type": "core.set", "position": [2.0, 0.0], "parameters": {"fields": {"path": "true"}}, "disabled": false},
            {"id": "false_branch", "node_type": "core.set", "position": [2.0, 1.0], "parameters": {"fields": {"path": "false"}}, "disabled": false},
            {"id": "merged", "node_type": "core.merge", "position": [3.0, 0.0], "parameters": {"mode": "append"}, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "branch", "to_input": 0},
            {"from_node": "branch", "from_output": 0, "to_node": "true_branch", "to_input": 0},
            {"from_node": "branch", "from_output": 1, "to_node": "false_branch", "to_input": 0},
            {"from_node": "true_branch", "from_output": 0, "to_node": "merged", "to_input": 0},
            {"from_node": "false_branch", "from_output": 0, "to_node": "merged", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(workflow_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");

    // Only the true branch ran (condition is always true), so the merge node's
    // output should show exactly one item, from true_branch.
    let merged_output = &execution["node_outputs"]["merged"];
    assert_eq!(merged_output.as_array().unwrap().len(), 1);
    assert_eq!(merged_output[0]["json"]["path"], "true");
    // The false branch produced no items (If routed nothing to output 1).
    assert_eq!(execution["node_outputs"]["false_branch"].as_array().unwrap().len(), 0);
}
```

- [ ] **Step 2: Run, confirm it fails or passes on the first try**

Run: `cargo test --test api_test branching_workflow_with_if_and_merge_executes_end_to_end`
Expected: PASS if Tasks 1–11 are correctly implemented and wired together (no new production code should be needed for this task — it is purely an integration-proof test). If it fails, the failure identifies exactly which earlier task's implementation has a bug; fix that task's code, not this test.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/api_test.rs
git commit -m "test: add end-to-end branching+merge workflow integration test"
```

---

## Explicitly Out of Scope (this plan)

Carried forward as future work per the roadmap breakdown:
- True per-item expression evaluation (each item seeing its own `$json` within one node's execution) — see Global Constraints.
- Concurrent branch execution (`tokio::spawn`/`JoinSet`) — sequential topological walk is used instead.
- Multiple independent trigger/start nodes in one workflow.
- Per-output-port execution data persisted/visible via the API (only the primary output is currently persisted) — a natural fit for Plan 7 (execution hardening) or the frontend work in Plan 6.
- Triggers (Webhook/Schedule), HTTP Request node, Credentials, AI Agent, frontend — unchanged, still Plans 3–6.
