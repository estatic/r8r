//! n8n's default document loader for AI nodes (`N8nJsonLoader`, n8n
//! 2.35.7) and the LangChain loaders under it (`JSONLoader`, `TextLoader`
//! from `@langchain/classic`), optionally followed by a text splitter.
//! `testdata/json_loader_fixtures.json` holds outputs of the real
//! `JSONLoader`, quirks included (strings inside nested objects come out
//! more than once with the default pointer `""`).

use super::text_split::{Document, Splitter};
use serde_json::{json, Map, Value};

/// JS truthiness, as `if (!json)` / `if (targetedEntry)` use it.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// `Array.prototype.includes`: objects and arrays by identity, the rest by value.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(_), _) | (Value::Array(_), _) => std::ptr::eq(a, b),
        _ => a == b,
    }
}

/// `JSONLoader.extractArrayStringsFromObject`.
fn extract<'a>(json: &'a Value, pointers: &[String], all: bool, key_found: bool, out: &mut Vec<&'a str>) {
    if !truthy(json) {
        return;
    }
    if let (Value::String(s), true) = (json, all) {
        out.push(s);
        return;
    }
    if let (Value::Array(a), true) = (json, all) {
        for e in a {
            extract(e, pointers, true, false, out);
        }
        return;
    }
    if matches!(json, Value::Object(_) | Value::Array(_)) {
        let values: Vec<&Value> = match json {
            Value::Object(o) => o.values().collect(),
            Value::Array(a) => a.iter().collect(),
            _ => unreachable!(),
        };
        if all {
            for v in values {
                extract(v, pointers, true, false, out);
            }
            return;
        }
        let targeted: Vec<&Value> = pointers.iter().filter_map(|p| json.pointer(p)).filter(|v| truthy(v)).collect();
        let others: Vec<&Value> = values.into_iter().filter(|v| !targeted.iter().any(|t| same(t, v))).collect();
        if !targeted.is_empty() {
            for t in &targeted {
                extract(t, pointers, true, true, out);
            }
            for o in others {
                extract(o, pointers, false, true, out);
            }
        } else if !key_found {
            for o in others {
                extract(o, pointers, false, false, out);
            }
        }
    }
}

/// `TextLoader.load`'s documents: one per text, `line` added when there
/// are several.
fn documents(texts: Vec<String>, blob_type: &str) -> Vec<Document> {
    let many = texts.len() != 1;
    texts
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let mut metadata = Map::from_iter([("source".to_string(), json!("blob")), ("blobType".to_string(), json!(blob_type))]);
            if many {
                metadata.insert("line".into(), json!(i + 1));
            }
            Document { page_content: t, metadata }
        })
        .collect()
}

/// `JSONLoader` over a JSON value with RFC 6901 pointers (`jsonpointer`
/// rejects pointers that don't start with `/`, except the root `""`).
pub fn json_loader(json: &Value, pointers: &[String]) -> Result<Vec<Document>, String> {
    if let Some(bad) = pointers.iter().find(|p| !p.is_empty() && !p.starts_with('/')) {
        return Err(format!("Invalid JSON pointer: {bad}"));
    }
    let mut out = Vec::new();
    extract(json, pointers, pointers.is_empty(), false, &mut out);
    Ok(documents(out.into_iter().map(String::from).collect(), "application/json"))
}

/// `N8nJsonLoader.processItem`: the item's JSON (`allInputData`) or the
/// `jsonData` parameter (`expressionData`; text as is, objects as JSON),
/// split when a splitter is given.
pub fn n8n_json_loader(item: &Map<String, Value>, mode: &str, json_data: &Value, pointers: &str, splitter: Option<&Splitter>) -> Result<Vec<Document>, String> {
    let pointers: Vec<String> = pointers.split(',').map(|p| p.trim().to_string()).collect();
    let docs = match (mode, json_data) {
        ("allInputData", _) => json_loader(&Value::Object(item.clone()), &pointers)?,
        ("expressionData", Value::String(s)) => documents(vec![s.clone()], "text/plain"),
        ("expressionData", Value::Object(_) | Value::Array(_)) => json_loader(json_data, &pointers)?,
        _ => return Err("Document loader is not initialized".into()),
    };
    Ok(match splitter {
        Some(s) => {
            let texts: Vec<String> = docs.iter().map(|d| d.page_content.clone()).collect();
            let metas: Vec<Map<String, Value>> = docs.into_iter().map(|d| d.metadata).collect();
            s.create_documents(&texts, &metas)
        }
        None => docs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_loader_matches_langchain() {
        let cases: Value = serde_json::from_str(include_str!("testdata/json_loader_fixtures.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let pointers: Vec<String> = case["pointers"].as_str().unwrap().split(',').map(|p| p.trim().to_string()).collect();
            let docs = json_loader(&case["json"], &pointers).unwrap();
            let got: Vec<Value> = docs.iter().map(|d| json!({"pageContent": d.page_content, "metadata": d.metadata})).collect();
            assert_eq!(Value::Array(got), case["docs"], "json {} pointers {:?}", case["json"], case["pointers"]);
        }
    }

    #[test]
    fn text_and_splitting() {
        let item = Map::new();
        let s = Splitter::recursive(10, 0).unwrap();
        let docs = n8n_json_loader(&item, "expressionData", &json!("one two three four"), "", Some(&s)).unwrap();
        let chunks: Vec<&str> = docs.iter().map(|d| d.page_content.as_str()).collect();
        // As LangChain splits it (the kept separator counts toward the size).
        assert_eq!(chunks, ["one two", "three", "four"]);
        assert_eq!(docs[0].metadata["blobType"], "text/plain");
        assert_eq!(docs[2].metadata["loc"]["lines"], json!({"from": 1, "to": 1}));
        assert!(json_loader(&json!({}), &["text".into()]).is_err());
    }
}
