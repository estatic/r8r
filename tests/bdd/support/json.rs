//! JSON helpers for assertions: path lookup, subset/exact matching with
//! wildcard tokens, and key-order extraction (serde_json's `Value` sorts
//! object keys, so order checks parse the raw text instead).

use serde_json::Value;

/// Looks up `path` in `value`. Syntax: `a.b[0].c`, and `["key.with.dots"]`
/// for keys that contain dots or brackets. An empty path returns `value`.
pub fn lookup<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in parse_path(path) {
        current = match segment {
            Segment::Key(k) => current.get(k.as_str())?,
            Segment::Index(i) => current.get(i)?,
        };
    }
    Some(current)
}

enum Segment {
    Key(String),
    Index(usize),
}

fn parse_path(path: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let chars: Vec<char> = path.chars().collect();
    let mut i = 0;
    let mut key = String::new();
    while i < chars.len() {
        match chars[i] {
            '.' => {
                if !key.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut key)));
                }
                i += 1;
            }
            '[' => {
                if !key.is_empty() {
                    out.push(Segment::Key(std::mem::take(&mut key)));
                }
                let end = chars[i..].iter().position(|c| *c == ']').map(|p| p + i).unwrap_or(chars.len());
                let inner: String = chars[i + 1..end].iter().collect();
                let inner = inner.trim();
                if let Some(quoted) = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    out.push(Segment::Key(quoted.to_string()));
                } else if let Ok(n) = inner.parse::<usize>() {
                    out.push(Segment::Index(n));
                } else {
                    out.push(Segment::Key(inner.to_string()));
                }
                i = end + 1;
            }
            c => {
                key.push(c);
                i += 1;
            }
        }
    }
    if !key.is_empty() {
        out.push(Segment::Key(key));
    }
    out
}

/// How strictly [`assert_matches`] compares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Objects must have exactly the expected keys.
    Exact,
    /// Objects may have extra keys; arrays must still match element-wise.
    Subset,
}

/// Compares `actual` against `expected`. String values in `expected` may be
/// wildcard tokens:
///
/// | token             | matches                                  |
/// |-------------------|------------------------------------------|
/// | `$any`            | any value, including null                |
/// | `$string`         | any string                               |
/// | `$nonempty`       | a non-empty string, array or object      |
/// | `$number`         | any number                               |
/// | `$boolean`        | true or false                            |
/// | `$datetime`       | an ISO-8601 date-time string             |
/// | `$regex:<re>`     | a string matching the regex              |
/// | `$contains:<s>`   | a string containing `s`                  |
pub fn assert_matches(expected: &Value, actual: &Value, mode: Mode) -> Result<(), String> {
    check(expected, actual, mode, "$")
}

fn check(expected: &Value, actual: &Value, mode: Mode, at: &str) -> Result<(), String> {
    if let Value::String(s) = expected {
        if let Some(result) = token_match(s, actual) {
            return result.map_err(|why| format!("at {at}: {why}; actual: {actual}"));
        }
    }
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (k, ev) in e {
                match a.get(k) {
                    Some(av) => check(ev, av, mode, &format!("{at}.{k}"))?,
                    None => return Err(format!("at {at}: missing key \"{k}\"; actual: {actual}")),
                }
            }
            if mode == Mode::Exact {
                if let Some(extra) = a.keys().find(|k| !e.contains_key(*k)) {
                    return Err(format!("at {at}: unexpected key \"{extra}\"; actual: {actual}"));
                }
            }
            Ok(())
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                return Err(format!("at {at}: expected {} elements, got {}; actual: {actual}", e.len(), a.len()));
            }
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                check(ev, av, mode, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        (Value::Number(e), Value::Number(a)) => {
            if e.as_f64() == a.as_f64() {
                Ok(())
            } else {
                Err(format!("at {at}: expected {e}, got {a}"))
            }
        }
        (e, a) if e == a => Ok(()),
        (e, a) => Err(format!("at {at}: expected {e}, got {a}")),
    }
}

fn token_match(token: &str, actual: &Value) -> Option<Result<(), String>> {
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(format!("expected {what}")) };
    Some(match token {
        "$any" => Ok(()),
        "$string" => ok(actual.is_string(), "a string"),
        "$number" => ok(actual.is_number(), "a number"),
        "$boolean" => ok(actual.is_boolean(), "a boolean"),
        "$nonempty" => ok(
            match actual {
                Value::String(s) => !s.is_empty(),
                Value::Array(a) => !a.is_empty(),
                Value::Object(o) => !o.is_empty(),
                _ => false,
            },
            "a non-empty value",
        ),
        "$datetime" => ok(
            actual.as_str().is_some_and(|s| {
                chrono::DateTime::parse_from_rfc3339(s).is_ok()
                    || regex_lite(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}", s)
            }),
            "an ISO-8601 date-time",
        ),
        t if t.starts_with("$regex:") => {
            let re = &t["$regex:".len()..];
            ok(actual.as_str().is_some_and(|s| regex_lite(re, s)), &format!("a string matching /{re}/"))
        }
        t if t.starts_with("$contains:") => {
            let needle = &t["$contains:".len()..];
            ok(actual.as_str().is_some_and(|s| s.contains(needle)), &format!("a string containing \"{needle}\""))
        }
        _ => return None,
    })
}

/// cucumber already depends on `regex`, re-exported through its
/// expressions crate; use it rather than adding another dependency.
pub fn regex_lite(pattern: &str, haystack: &str) -> bool {
    cucumber::codegen::Regex::new(pattern)
        .unwrap_or_else(|e| panic!("invalid regex /{pattern}/: {e}"))
        .is_match(haystack)
}

/// Parses `text` as JSON and returns the key order of the object found at
/// `path` (same syntax as [`lookup`], array indices included).
pub fn key_order(text: &str, path: &str) -> Result<Vec<String>, String> {
    let root: Ordered = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let mut current = &root;
    for segment in parse_path(path) {
        current = match (segment, current) {
            (Segment::Key(k), Ordered::Object(entries)) => {
                &entries.iter().find(|(ek, _)| *ek == k).ok_or_else(|| format!("no key {k}"))?.1
            }
            (Segment::Index(i), Ordered::Array(items)) => items.get(i).ok_or_else(|| format!("no index {i}"))?,
            _ => return Err(format!("path {path} does not resolve")),
        };
    }
    match current {
        Ordered::Object(entries) => Ok(entries.iter().map(|(k, _)| k.clone()).collect()),
        _ => Err(format!("value at {path} is not an object")),
    }
}

enum Ordered {
    Object(Vec<(String, Ordered)>),
    Array(Vec<Ordered>),
    Scalar,
}

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Ordered;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("any JSON value")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, Ordered>()? {
                    entries.push((k, v));
                }
                Ok(Ordered::Object(entries))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut s: A) -> Result<Ordered, A::Error> {
                let mut items = Vec::new();
                while let Some(v) = s.next_element::<Ordered>()? {
                    items.push(v);
                }
                Ok(Ordered::Array(items))
            }
            fn visit_bool<E>(self, _: bool) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
            fn visit_str<E>(self, _: &str) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
            fn visit_unit<E>(self) -> Result<Ordered, E> {
                Ok(Ordered::Scalar)
            }
        }
        d.deserialize_any(V)
    }
}

/// Parses a step argument as JSON, falling back to a JSON string for bare
/// text, so `"abc"`, `abc`, `3` and `{"a":1}` are all accepted.
pub fn parse_loose(text: &str) -> Value {
    serde_json::from_str(text.trim()).unwrap_or_else(|_| Value::String(text.to_string()))
}

pub fn parse_strict(text: &str, what: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{what} is not valid JSON ({e}):\n{text}"))
}
