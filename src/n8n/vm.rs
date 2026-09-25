//! A sandboxed QuickJS context preloaded with Luxon, jmespath and r8r's
//! prelude (n8n's data proxy and extension methods). Used for expressions
//! (spec §6.4) and, until the out-of-process runner lands (Phase 3), the
//! Code node.
//!
//! Values cross the boundary as JSON text only. The context has no host
//! capabilities beyond a few pure helpers (time-zone offsets, hashing);
//! `Function`-constructor escapes are closed off by the prelude.

use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LUXON: &str = include_str!("js/luxon.min.js");
const JMESPATH: &str = include_str!("js/jmespath.js");
const PRELUDE: &str = include_str!("js/prelude.js");

#[derive(Debug, Clone, thiserror::Error)]
pub enum VmError {
    #[error("{message}")]
    Js { message: String, name: String, stack: String },
    #[error("{0}")]
    Syntax(String),
    #[error("execution timed out after {0} ms")]
    Timeout(u64),
    #[error("{0}")]
    Internal(String),
}

impl VmError {
    pub fn message(&self) -> String {
        self.to_string()
    }
}

pub struct VmOptions {
    pub memory_limit: usize,
    pub timezone: String,
    /// Whether the data handed to scripts is deep-frozen (expressions) or
    /// mutable (Code node).
    pub freeze_data: bool,
}

impl Default for VmOptions {
    fn default() -> Self {
        Self { memory_limit: 64 * 1024 * 1024, timezone: "UTC".into(), freeze_data: true }
    }
}

pub struct Vm {
    // Field order matters: the context must drop before the runtime.
    context: rquickjs::Context,
    runtime: rquickjs::Runtime,
    deadline: Arc<Mutex<Option<Instant>>>,
}

fn tz_offset_minutes(name: String, ts_ms: f64) -> f64 {
    use chrono::{Offset, TimeZone};
    let Ok(tz) = name.parse::<chrono_tz::Tz>() else { return f64::NAN };
    let Some(utc) = chrono::DateTime::from_timestamp_millis(ts_ms as i64) else { return f64::NAN };
    tz.offset_from_utc_datetime(&utc.naive_utc()).fix().local_minus_utc() as f64 / 60.0
}

fn tz_abbreviation(name: String, ts_ms: f64) -> String {
    let Ok(tz) = name.parse::<chrono_tz::Tz>() else { return String::new() };
    let Some(utc) = chrono::DateTime::from_timestamp_millis(ts_ms as i64) else { return String::new() };
    utc.with_timezone(&tz).format("%Z").to_string()
}

fn tz_valid(name: String) -> bool {
    name.parse::<chrono_tz::Tz>().is_ok()
}

/// Hex or base64 digest of `text` (`key` makes it an HMAC).
pub fn digest(algorithm: &str, text: &[u8], key: Option<&[u8]>, encoding: &str) -> Result<String, String> {
    macro_rules! run {
        ($t:ty) => {
            match key {
                Some(k) => {
                    let mut mac = <hmac::Hmac<$t> as hmac::Mac>::new_from_slice(k).expect("HMAC takes any key length");
                    hmac::Mac::update(&mut mac, text);
                    hmac::Mac::finalize(mac).into_bytes().to_vec()
                }
                None => <$t as sha2::Digest>::digest(text).to_vec(),
            }
        };
    }
    let bytes = match algorithm.to_ascii_lowercase().replace('-', "").as_str() {
        "md5" => run!(md5::Md5),
        "sha1" => run!(sha1::Sha1),
        "sha256" => run!(sha2::Sha256),
        "sha384" => run!(sha2::Sha384),
        "sha512" => run!(sha2::Sha512),
        other => return Err(format!("unsupported hash algorithm: {other}")),
    };
    Ok(match encoding {
        "base64" => {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(bytes)
        }
        _ => hex::encode(bytes),
    })
}

impl Vm {
    pub fn new(options: VmOptions) -> Result<Self, VmError> {
        let runtime = rquickjs::Runtime::new().map_err(|e| VmError::Internal(e.to_string()))?;
        runtime.set_memory_limit(options.memory_limit);
        runtime.set_max_stack_size(1024 * 1024);
        let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let d = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || d.lock().unwrap().is_some_and(|t| Instant::now() >= t))));
        let context = rquickjs::Context::full(&runtime).map_err(|e| VmError::Internal(e.to_string()))?;
        let vm = Self { context, runtime, deadline };
        vm.context.with(|ctx| -> Result<(), VmError> {
            let g = ctx.globals();
            install(&g, "__r8r_tz_offset", rquickjs::Function::new(ctx.clone(), tz_offset_minutes))?;
            install(&g, "__r8r_tz_abbr", rquickjs::Function::new(ctx.clone(), tz_abbreviation))?;
            install(&g, "__r8r_tz_valid", rquickjs::Function::new(ctx.clone(), tz_valid))?;
            install(
                &g,
                "__r8r_hash",
                rquickjs::Function::new(
                    ctx.clone(),
                    |alg: String, text: String, key: rquickjs::function::Opt<String>, enc: rquickjs::function::Opt<String>| {
                        digest(&alg, text.as_bytes(), key.0.as_deref().map(str::as_bytes), enc.0.as_deref().unwrap_or("hex")).unwrap_or_default()
                    },
                ),
            )?;
            g.set("__r8r_timezone", options.timezone.clone()).map_err(|e| VmError::Internal(e.to_string()))?;
            g.set("__r8r_freeze", options.freeze_data).map_err(|e| VmError::Internal(e.to_string()))?;
            Ok(())
        })?;
        for (name, src) in [("luxon", LUXON), ("jmespath", JMESPATH), ("prelude", PRELUDE)] {
            vm.run_script(src, Duration::from_secs(10)).map_err(|e| VmError::Internal(format!("loading {name}: {e}")))?;
        }
        Ok(vm)
    }

    /// Runs statements for their effect (no result).
    pub fn run_script(&self, src: &str, timeout: Duration) -> Result<(), VmError> {
        self.with_deadline(timeout, |ctx| {
            ctx.eval::<rquickjs::Value, _>(src).map(|_| ()).map_err(|e| describe(&ctx, e))
        })
        .map_err(|e| self.timeout_or(e, timeout))
    }

    /// Calls a prelude function that takes one JSON argument.
    pub fn call_json(&self, function: &str, arg: &Value) -> Result<(), VmError> {
        let src = format!("{function}({})", serde_json::to_string(arg).unwrap());
        self.run_script(&src, Duration::from_secs(30))
    }

    /// Evaluates a JS expression. `Ok(None)` means `undefined`.
    pub fn eval_expression(&self, expr: &str, timeout: Duration) -> Result<Option<Value>, VmError> {
        let src = format!("__r8r_run(function () {{ return (\n{expr}\n); }})");
        self.eval_wrapped(&src, timeout)
    }

    /// Runs a function body (may use `await`) and returns its result.
    pub fn eval_async_body(&self, body: &str, timeout: Duration) -> Result<Option<Value>, VmError> {
        let src = format!("__r8r_run_async(async function () {{\n{body}\n}})");
        let started = Instant::now();
        self.eval_wrapped(&src, timeout)?;
        // Drive promises until the body settles or time runs out.
        loop {
            let settled = self.with_deadline(timeout.saturating_sub(started.elapsed()), |ctx| {
                while ctx.execute_pending_job() {}
                let done: bool = ctx.globals().get("__r8r_async_done").unwrap_or(false);
                Ok(done)
            });
            match settled {
                Ok(true) => break,
                Ok(false) => {
                    if started.elapsed() >= timeout {
                        return Err(VmError::Timeout(timeout.as_millis() as u64));
                    }
                    // Nothing can make progress without host I/O, which the
                    // sandbox doesn't offer: a pending promise never settles.
                    return Err(VmError::Js {
                        message: "The code awaited something that never finished".into(),
                        name: "Error".into(),
                        stack: String::new(),
                    });
                }
                Err(e) => return Err(self.timeout_or(e, timeout)),
            }
        }
        let text: String = self.context.with(|ctx| ctx.globals().get("__r8r_async_result").unwrap_or_default());
        decode_result(&text)
    }

    fn eval_wrapped(&self, src: &str, timeout: Duration) -> Result<Option<Value>, VmError> {
        let text = self
            .with_deadline(timeout, |ctx| {
                let v: rquickjs::Value = ctx.eval(src).map_err(|e| describe(&ctx, e))?;
                Ok(v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_default())
            })
            .map_err(|e| self.timeout_or(e, timeout))?;
        decode_result(&text)
    }

    fn with_deadline<T>(&self, timeout: Duration, f: impl FnOnce(rquickjs::Ctx) -> Result<T, VmError>) -> Result<T, VmError> {
        *self.deadline.lock().unwrap() = Some(Instant::now() + timeout);
        let result = self.context.with(f);
        *self.deadline.lock().unwrap() = None;
        result
    }

    fn timeout_or(&self, e: VmError, timeout: Duration) -> VmError {
        match &e {
            VmError::Js { message, .. } | VmError::Internal(message) if message.contains("interrupted") => {
                VmError::Timeout(timeout.as_millis() as u64)
            }
            _ => e,
        }
    }

    pub fn memory_used(&self) -> i64 {
        self.runtime.memory_usage().memory_used_size
    }
}

/// Named (not a closure) so `'js` ties the function to the globals' context.
fn install<'js>(globals: &rquickjs::Object<'js>, name: &str, f: rquickjs::Result<rquickjs::Function<'js>>) -> Result<(), VmError> {
    let f = f.map_err(|e| VmError::Internal(e.to_string()))?;
    globals.set(name, f).map_err(|e| VmError::Internal(e.to_string()))
}

fn describe(ctx: &rquickjs::Ctx, error: rquickjs::Error) -> VmError {
    if let rquickjs::Error::Exception = error {
        let exc = ctx.catch();
        if let Some(obj) = exc.as_object() {
            let message: String = obj.get("message").unwrap_or_default();
            let name: String = obj.get("name").unwrap_or_default();
            let stack: String = obj.get("stack").unwrap_or_default();
            if name == "SyntaxError" {
                return VmError::Syntax(format!("invalid syntax: {message}"));
            }
            return VmError::Js { message, name, stack };
        }
        let text = exc.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| "unknown error".into());
        return VmError::Js { message: text, name: "Error".into(), stack: String::new() };
    }
    VmError::Internal(error.to_string())
}

/// `{"v": ...}` = a value, `{"u": 1}` = undefined, `{"e": {...}}` = thrown.
fn decode_result(text: &str) -> Result<Option<Value>, VmError> {
    let parsed: Value = serde_json::from_str(text).map_err(|e| VmError::Internal(format!("bad result from the VM ({e}): {text}")))?;
    if let Some(v) = parsed.get("v") {
        return Ok(Some(v.clone()));
    }
    if parsed.get("u").is_some() {
        return Ok(None);
    }
    let e = &parsed["e"];
    let message = e["message"].as_str().unwrap_or("error").to_string();
    if message.contains("interrupted") {
        return Err(VmError::Internal(message));
    }
    Err(VmError::Js {
        message,
        name: e["name"].as_str().unwrap_or("Error").to_string(),
        stack: e["stack"].as_str().unwrap_or("").to_string(),
    })
}
