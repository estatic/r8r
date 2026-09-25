//! Parameter expressions (spec §6.4): a string starting with `=` is a
//! template whose `{{ … }}` blocks are JavaScript. A template that is one
//! block keeps the value's native type; anything else becomes a string.

use super::vm::{Vm, VmError, VmOptions};
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Text(String),
    Code(String),
}

/// Splits a template (without the leading `=`) into literal text and
/// `{{ }}` blocks. Braces inside blocks are balanced and string literals are
/// skipped, so `{{ { a: '}}' } }}` is one block.
pub fn split_template(template: &str) -> Result<Vec<Segment>, String> {
    let chars: Vec<char> = template.chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1) == Some(&'{') {
            if !text.is_empty() {
                out.push(Segment::Text(std::mem::take(&mut text)));
            }
            i += 2;
            let mut code = String::new();
            let mut depth = 0usize;
            let mut quote: Option<char> = None;
            loop {
                let Some(&c) = chars.get(i) else {
                    return Err(format!("unterminated {{{{ in expression: {template}"));
                };
                if let Some(q) = quote {
                    code.push(c);
                    if c == '\\' {
                        if let Some(&n) = chars.get(i + 1) {
                            code.push(n);
                            i += 1;
                        }
                    } else if c == q {
                        quote = None;
                    }
                    i += 1;
                    continue;
                }
                match c {
                    '\'' | '"' | '`' => {
                        quote = Some(c);
                        code.push(c);
                    }
                    '{' => {
                        depth += 1;
                        code.push(c);
                    }
                    '}' if depth > 0 => {
                        depth -= 1;
                        code.push(c);
                    }
                    '}' if chars.get(i + 1) == Some(&'}') => {
                        i += 2;
                        break;
                    }
                    _ => code.push(c),
                }
                i += 1;
            }
            out.push(Segment::Code(code));
        } else {
            text.push(chars[i]);
            i += 1;
        }
    }
    if !text.is_empty() {
        out.push(Segment::Text(text));
    }
    Ok(out)
}

pub fn is_expression(value: &Value) -> bool {
    value.as_str().is_some_and(|s| s.starts_with('='))
}

pub fn contains_expression(value: &Value) -> bool {
    match value {
        Value::String(s) => s.starts_with('='),
        Value::Array(a) => a.iter().any(contains_expression),
        Value::Object(o) => o.values().any(contains_expression),
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct ExprError {
    pub message: String,
    pub description: Option<String>,
}

impl From<VmError> for ExprError {
    fn from(e: VmError) -> Self {
        match e {
            VmError::Timeout(ms) => ExprError {
                message: format!("Expression timed out after {ms} ms"),
                description: Some("The expression took too long to evaluate and was stopped.".into()),
            },
            VmError::Syntax(m) => ExprError { message: m, description: None },
            other => ExprError { message: other.message(), description: None },
        }
    }
}

/// An expression evaluator bound to one node run's data (see
/// `js/prelude.js` for the variables it exposes).
pub struct Evaluator {
    vm: Vm,
    timeout: Duration,
}

impl Evaluator {
    pub fn new(data: &Value, timezone: &str, timeout: Duration) -> Result<Self, ExprError> {
        let vm = Vm::new(VmOptions { timezone: timezone.to_string(), ..Default::default() })?;
        vm.call_json("__r8r_set_data", data)?;
        Ok(Self { vm, timeout })
    }

    pub fn set_item(&self, index: usize) -> Result<(), ExprError> {
        self.vm.run_script(&format!("__r8r_set_item({index})"), Duration::from_secs(5))?;
        Ok(())
    }

    /// Extra globals for this evaluation (e.g. `$response`, `$pageCount`).
    pub fn set_extra(&self, values: &Value) -> Result<(), ExprError> {
        self.vm.call_json("__r8r_set_extra", values)?;
        Ok(())
    }

    /// Resolves every expression string in `value` (recursively).
    pub fn resolve(&self, value: &Value) -> Result<Value, ExprError> {
        match value {
            Value::String(s) if s.starts_with('=') => Ok(self.template(&s[1..])?.unwrap_or(Value::Null)),
            Value::Array(a) => a.iter().map(|v| self.resolve(v)).collect::<Result<Vec<_>, _>>().map(Value::Array),
            Value::Object(o) => {
                let mut out = serde_json::Map::new();
                for (k, v) in o {
                    out.insert(k.clone(), self.resolve(v)?);
                }
                Ok(Value::Object(out))
            }
            other => Ok(other.clone()),
        }
    }

    /// Evaluates a template (no leading `=`). `None` = undefined.
    pub fn template(&self, template: &str) -> Result<Option<Value>, ExprError> {
        let segments = split_template(template).map_err(|m| ExprError { message: m, description: None })?;
        if let [Segment::Code(code)] = segments.as_slice() {
            return self.code(code);
        }
        let mut out = String::new();
        for seg in segments {
            match seg {
                Segment::Text(t) => out.push_str(&t),
                Segment::Code(c) => out.push_str(&stringify(self.code(&c)?.as_ref())),
            }
        }
        Ok(Some(Value::String(out)))
    }

    fn code(&self, code: &str) -> Result<Option<Value>, ExprError> {
        if code.trim().is_empty() {
            return Ok(Some(Value::String(String::new())));
        }
        Ok(self.vm.eval_expression(code, self.timeout)?)
    }
}

/// How a value appears when spliced into text.
pub fn stringify(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(v @ (Value::Array(_) | Value::Object(_))) => v.to_string(),
        Some(v) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_text_and_blocks() {
        assert_eq!(
            split_template("Hi {{ $json.name }}!").unwrap(),
            vec![Segment::Text("Hi ".into()), Segment::Code(" $json.name ".into()), Segment::Text("!".into())]
        );
    }

    #[test]
    fn nested_braces_and_strings_stay_inside_one_block() {
        assert_eq!(split_template("{{ { a: '}}' } }}").unwrap(), vec![Segment::Code(" { a: '}}' } ".into())]);
    }

    fn eval(template: &str, data: Value) -> Option<Value> {
        Evaluator::new(&data, "UTC", Duration::from_secs(1)).unwrap().template(template).unwrap()
    }

    fn data(item: Value) -> Value {
        serde_json::json!({"input": [{"json": item}], "inputs": [[{"json": item}]], "source": [], "runData": {}, "workflow": {"name": "t"}, "execution": {}, "node": {"name": "n"}})
    }

    #[test]
    fn single_block_keeps_native_type_and_mixed_is_a_string() {
        assert_eq!(eval("{{ 1 + 2 }}", data(serde_json::json!({}))), Some(serde_json::json!(3)));
        assert_eq!(eval("Hi {{ $json.n }}", data(serde_json::json!({"n": "Ada"}))), Some(serde_json::json!("Hi Ada")));
    }

    #[test]
    fn luxon_time_zones_work_without_intl() {
        let v = eval(
            "{{ DateTime.fromISO('2024-03-10T12:00:00', { zone: 'UTC' }).setZone('America/New_York').toFormat('HH:mm') }}",
            data(serde_json::json!({})),
        );
        assert_eq!(v, Some(serde_json::json!("08:00")));
    }

    #[test]
    fn the_function_constructor_is_blocked() {
        let e = Evaluator::new(&data(serde_json::json!({})), "UTC", Duration::from_secs(1)).unwrap();
        assert!(e.template("{{ (function(){}).constructor('return 1')() }}").is_err());
    }
}
