//! Code node (spec §6.7), JavaScript only for now.
//!
//! Runs in its own QuickJS sandbox: no network, no file system, a memory cap
//! and the `N8N_RUNNERS_TASK_TIMEOUT` limit. `require` only resolves
//! built-ins listed in `NODE_FUNCTION_ALLOW_BUILTIN` (currently `crypto`).
//! Moving this into the out-of-process `r8r runner` is Phase 3.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::vm::{Vm, VmError, VmOptions};
use serde_json::{json, Map, Value};
use std::time::Duration;

pub struct Code;

const CODE_PRELUDE: &str = r#"
(function () {
  const g = globalThis;
  g.__r8r_console = [];
  const fmt = (a) => (typeof a === 'string' ? a : (() => { try { return JSON.stringify(a); } catch (e) { return String(a); } })());
  g.console = {
    log: (...a) => g.__r8r_console.push(a.map(fmt).join(' ')),
    info: (...a) => g.__r8r_console.push(a.map(fmt).join(' ')),
    warn: (...a) => g.__r8r_console.push(a.map(fmt).join(' ')),
    error: (...a) => g.__r8r_console.push(a.map(fmt).join(' ')),
  };
  const cryptoShim = {
    createHash(alg) {
      let data = '';
      return { update(s) { data += s; return this; }, digest(enc) { return __r8r_hash(alg, data, undefined, enc || 'hex'); } };
    },
    createHmac(alg, key) {
      let data = '';
      return { update(s) { data += s; return this; }, digest(enc) { return __r8r_hash(alg, data, key, enc || 'hex'); } };
    },
    randomUUID() {
      const h = '0123456789abcdef';
      let s = '';
      for (let i = 0; i < 36; i++) s += [8, 13, 18, 23].includes(i) ? '-' : i === 14 ? '4' : h[Math.floor(Math.random() * 16)];
      return s;
    },
  };
  const allowed = __r8r_allow_builtin;
  g.require = function (name) {
    const bare = String(name).replace(/^node:/, '');
    if (bare === 'crypto' && (allowed.includes('*') || allowed.includes('crypto'))) return cryptoShim;
    if (allowed.includes('*') || allowed.includes(bare)) throw new Error(`Module '${bare}' is not available in r8r's Code node`);
    throw new Error(`Cannot find module '${bare}' [not in the allowed list NODE_FUNCTION_ALLOW_BUILTIN]`);
  };
})();
"#;

/// Line of `error` within the user's code (the body starts on script line 2).
fn user_line(stack: &str) -> Option<usize> {
    let re = regex::Regex::new(r":(\d+)(?::\d+)?\)?").ok()?;
    let line = re.captures_iter(stack).filter_map(|c| c[1].parse::<usize>().ok()).find(|l| *l >= 2);
    line.map(|l| l - 1)
}

fn to_node_error(e: VmError, timeout_secs: u64) -> NodeError {
    match e {
        VmError::Timeout(_) => NodeError::new(format!("Task execution timed out after {timeout_secs} seconds"))
            .describe("The Code node took too long and was stopped. Raise N8N_RUNNERS_TASK_TIMEOUT if this is expected."),
        VmError::Js { message, stack, .. } => {
            let message = match user_line(&stack) {
                Some(line) => format!("{message} [line {line}]"),
                None => message,
            };
            NodeError::new(message)
        }
        other => NodeError::new(other.message()),
    }
}

/// n8n's item normalisation: `{json, binary}` objects stay, plain objects
/// are wrapped.
fn normalize(value: Value, each: bool) -> Result<Vec<Item>, NodeError> {
    let invalid = || {
        NodeError::new("Code doesn't return items properly")
            .describe(if each { "Please return an object representing the output item" } else { "Please return an array of objects, one for each item you would like to output." })
    };
    let list = match value {
        Value::Array(a) if !each => a,
        Value::Object(o) => vec![Value::Object(o)],
        _ => return Err(invalid()),
    };
    list.into_iter()
        .map(|v| match v {
            Value::Object(o) => {
                if let Some(Value::Object(json)) = o.get("json") {
                    Ok(Item {
                        json: json.clone(),
                        binary: o.get("binary").and_then(Value::as_object).cloned(),
                        paired_item: o.get("pairedItem").cloned(),
                    })
                } else {
                    Ok(Item::new(o))
                }
            }
            _ => Err(invalid()),
        })
        .collect()
}

#[async_trait::async_trait]
impl NodeType for Code {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.code"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let language = ctx.raw_param("language").and_then(Value::as_str).unwrap_or("javaScript");
        if language == "python" || language == "pythonNative" {
            return run_python(ctx).await;
        }
        if language != "javaScript" {
            return Err(NodeError::new(format!("The language \"{language}\" is not supported")));
        }
        let code = ctx.raw_param("jsCode").and_then(Value::as_str).unwrap_or("return [];").to_string();
        let each = ctx.raw_param("mode").and_then(Value::as_str) == Some("runOnceForEachItem");
        let timeout_secs = ctx.config().runners_task_timeout_secs;
        let timeout = Duration::from_secs(timeout_secs);
        let timezone = ctx.workflow.setting_str("timezone").unwrap_or(&ctx.config().timezone).to_string();
        let vm = Vm::new(VmOptions { memory_limit: 256 * 1024 * 1024, timezone, freeze_data: false }).map_err(|e| NodeError::new(e.message()))?;
        let allowed = serde_json::to_string(&ctx.config().node_function_allow_builtin).unwrap();
        vm.run_script(&format!("globalThis.__r8r_allow_builtin = {allowed};"), Duration::from_secs(1)).map_err(|e| NodeError::new(e.message()))?;
        vm.run_script(CODE_PRELUDE, Duration::from_secs(5)).map_err(|e| NodeError::new(e.message()))?;
        let data = (ctx.expr_data)();
        vm.set_data(data).map_err(|e| NodeError::new(e.message()))?;
        vm.run_script("globalThis.items = $input.all();", Duration::from_secs(5)).map_err(|e| NodeError::new(e.message()))?;

        let mut out = Vec::new();
        if each {
            for i in 0..ctx.input().len() {
                vm.run_script(&format!("__r8r_set_item({i}); globalThis.item = $input.item;"), Duration::from_secs(1)).map_err(|e| NodeError::new(e.message()))?;
                match vm.eval_async_body(&code, timeout) {
                    Ok(Some(v)) => match normalize(v, true) {
                        Ok(items) => out.extend(items.into_iter().map(|it| it.paired(i))),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e.at(i)),
                    },
                    Ok(None) => {}
                    Err(e) => {
                        let err = to_node_error(e, timeout_secs).at(i);
                        if ctx.continue_on_fail() {
                            ctx.push_error_item(&err, i);
                        } else {
                            return Err(err);
                        }
                    }
                }
            }
        } else {
            let result = vm.eval_async_body(&code, timeout).map_err(|e| to_node_error(e, timeout_secs))?;
            out = normalize(result.unwrap_or(Value::Null), false)?;
        }

        // Carry console output, custom data and static data back.
        let side = vm
            .eval_expression("({ console: __r8r_console, custom: __r8r_custom, statics: __r8r_static })", Duration::from_secs(5))
            .ok()
            .flatten()
            .unwrap_or(json!({}));
        let mut run = ctx.run.lock().unwrap();
        for line in side["console"].as_array().into_iter().flatten() {
            run.console.push(line.as_str().unwrap_or_default().to_string());
        }
        if let Some(custom) = side["custom"].as_object() {
            for (k, v) in custom {
                run.custom_data.insert(k.clone(), v.clone());
            }
        }
        if let Some(global) = side.pointer("/statics/global").cloned() {
            let mut statics = run.static_data.as_object().cloned().unwrap_or_else(Map::new);
            statics.insert("global".into(), global);
            run.static_data = Value::Object(statics);
        }
        Ok(vec![out])
    }
}

async fn run_python(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let code = ctx.raw_param("pythonCode").and_then(Value::as_str).unwrap_or("return []").to_string();
    let each = ctx.raw_param("mode").and_then(Value::as_str) == Some("runOnceForEachItem");
    let (results, console) = super::python::run(ctx, &code, each).await?;
    ctx.run.lock().unwrap().console.extend(console);
    let mut out = Vec::new();
    if each {
        for (i, v) in results.into_iter().enumerate() {
            if v.is_null() {
                continue;
            }
            match normalize(v, true) {
                Ok(items) => out.extend(items.into_iter().map(|it| it.paired(i))),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e.at(i)),
            }
        }
    } else {
        out = normalize(results.into_iter().next().unwrap_or(Value::Null), false)?;
    }
    Ok(vec![out])
}
