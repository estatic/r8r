//! Crypto, Date & Time, Markdown, XML.

use super::set_path;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use crate::n8n::vm::digest;
use serde_json::{json, Map, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Crypto), Box::new(DateTimeNode), Box::new(Markdown), Box::new(Xml)]
}

fn with_field(item: &Item, i: usize, path: &str, value: Value) -> Item {
    let mut json = item.json.clone();
    set_path(&mut json, path, value);
    Item { json, binary: item.binary.clone(), paired_item: None }.paired(i)
}

struct Crypto;

#[async_trait::async_trait]
impl NodeType for Crypto {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.crypto"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        for (i, item) in input.iter().enumerate() {
            let action = ctx.param_str("action", i, "hash")?;
            let field = ctx.param_str("dataPropertyName", i, "data")?;
            let value = match action.as_str() {
                "hash" | "hmac" => {
                    let alg = ctx.param_str("type", i, "MD5")?;
                    let text = ctx.param_str("value", i, "")?;
                    let encoding = ctx.param_str("encoding", i, "hex")?;
                    let secret = if action == "hmac" { Some(ctx.param_str("secret", i, "")?) } else { None };
                    json!(digest(&alg, text.as_bytes(), secret.as_deref().map(str::as_bytes), &encoding).map_err(|e| NodeError::new(e).at(i))?)
                }
                "generate" => {
                    let kind = ctx.param_str("encodingType", i, "uuid")?;
                    let len = ctx.param_f64("stringLength", i, 32.0)? as usize;
                    match kind.as_str() {
                        "uuid" => json!(uuid::Uuid::new_v4().to_string()),
                        "hex" => json!(hex::encode((0..len.div_ceil(2)).map(|_| rand::random::<u8>()).collect::<Vec<_>>())[..len].to_string()),
                        _ => {
                            use rand::Rng;
                            let s: String = rand::thread_rng().sample_iter(&rand::distributions::Alphanumeric).take(len).map(char::from).collect();
                            json!(s)
                        }
                    }
                }
                other => return Err(NodeError::new(format!("The crypto action \"{other}\" is not supported yet")).at(i)),
            };
            out.push(with_field(item, i, &field, value));
        }
        Ok(vec![out])
    }
}

struct DateTimeNode;

const DT_SCRIPT: &str = r#"(() => {
  const toDt = (v) => {
    if (v === null || v === undefined || v === '') throw new Error('The date is empty');
    if (typeof v === 'number') return DateTime.fromMillis(v);
    if (DateTime.isDateTime(v)) return v;
    return String(v).toDateTime();
  };
  const o = __r8r_dt;
  const tz = o.timezone || undefined;
  switch (o.operation) {
    case 'formatDate': {
      let d = toDt(o.date);
      if (tz) d = d.setZone(tz);
      const f = o.format === 'custom' ? o.customFormat : o.format;
      return f === 'X' ? d.toSeconds() : f === 'x' ? d.toMillis() : d.toFormat(f);
    }
    case 'addToDate': return toDt(o.date).plus({ [o.unit]: Number(o.duration) }).toISO();
    case 'subtractFromDate': return toDt(o.date).minus({ [o.unit]: Number(o.duration) }).toISO();
    case 'getCurrentDate': return (o.includeTime === false ? $today : $now).toISO();
    case 'extractDate': return toDt(o.date).get(o.part || 'year');
    case 'roundDate': return (o.mode === 'roundUp' ? toDt(o.date).endOf(o.toNearest || 'day') : toDt(o.date).startOf(o.toNearest || 'day')).toISO();
    case 'getTimeBetweenDates': return toDt(o.endDate).diff(toDt(o.date), o.units || ['days']).toObject();
    default: throw new Error('Unknown operation: ' + o.operation);
  }
})()"#;

#[async_trait::async_trait]
impl NodeType for DateTimeNode {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.dateTime"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        for (i, item) in input.iter().enumerate() {
            let operation = ctx.param_str("operation", i, "getCurrentDate")?;
            let default_field = match operation.as_str() {
                "formatDate" => "formattedDate",
                "getCurrentDate" => "currentDate",
                "extractDate" => "datePart",
                "getTimeBetweenDates" => "timeDifference",
                _ => "newDate",
            };
            let field = ctx.param_str("outputFieldName", i, default_field)?;
            let date = match operation.as_str() {
                "addToDate" | "subtractFromDate" => ctx.param("magnitude", i)?,
                "getTimeBetweenDates" => ctx.param("startDate", i)?,
                _ => ctx.param("date", i)?,
            };
            let args = json!({
                "operation": operation,
                "date": date,
                "endDate": ctx.param("endDate", i)?,
                "format": ctx.param_str("format", i, "yyyy-MM-dd")?,
                "customFormat": ctx.param_str("customFormat", i, "")?,
                "unit": ctx.param_str("timeUnit", i, "days")?,
                "duration": ctx.param("duration", i)?,
                "includeTime": ctx.param_bool("includeTime", i, true)?,
                "part": ctx.param_str("part", i, "year")?,
                "mode": ctx.param_str("mode", i, "roundDown")?,
                "toNearest": ctx.param_str("toNearest", i, "day")?,
                "timezone": ctx.param_str("options.timezone", i, "")?,
            });
            let ev = ctx.evaluator()?;
            ev.set_extra(&json!({ "__r8r_dt": args })).map_err(|e| NodeError::from(e).at(i))?;
            let value = ev.template(&format!("{{{{ {DT_SCRIPT} }}}}")).map_err(|e| NodeError::from(e).at(i))?.unwrap_or(Value::Null);
            out.push(with_field(item, i, &field, value));
        }
        Ok(vec![out])
    }
}

struct Markdown;

fn html_to_markdown(html: &str) -> String {
    let mut s = html.to_string();
    for (tag, prefix) in [("h1", "# "), ("h2", "## "), ("h3", "### "), ("h4", "#### ")] {
        s = regex::Regex::new(&format!(r"(?is)<{tag}[^>]*>(.*?)</{tag}>")).unwrap().replace_all(&s, format!("{prefix}$1\n\n")).to_string();
    }
    s = regex::Regex::new(r"(?is)<(strong|b)>(.*?)</(strong|b)>").unwrap().replace_all(&s, "**$2**").to_string();
    s = regex::Regex::new(r"(?is)<(em|i)>(.*?)</(em|i)>").unwrap().replace_all(&s, "*$2*").to_string();
    s = regex::Regex::new(r#"(?is)<a [^>]*href="([^"]*)"[^>]*>(.*?)</a>"#).unwrap().replace_all(&s, "[$2]($1)").to_string();
    s = regex::Regex::new(r"(?is)<li[^>]*>(.*?)</li>").unwrap().replace_all(&s, "- $1\n").to_string();
    s = regex::Regex::new(r"(?is)</p>|<br\s*/?>").unwrap().replace_all(&s, "\n\n").to_string();
    s = regex::Regex::new(r"(?s)<[^>]+>").unwrap().replace_all(&s, "").to_string();
    s.trim().to_string()
}

#[async_trait::async_trait]
impl NodeType for Markdown {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.markdown"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        for (i, item) in input.iter().enumerate() {
            let mode = ctx.param_str("mode", i, "htmlToMarkdown")?;
            let key = ctx.param_str("destinationKey", i, "data")?;
            let value = if mode == "markdownToHtml" {
                let md = ctx.param_str("markdown", i, "")?;
                let mut html = String::new();
                pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new_ext(&md, pulldown_cmark::Options::all()));
                html
            } else {
                html_to_markdown(&ctx.param_str("html", i, "")?)
            };
            out.push(with_field(item, i, &key, json!(value)));
        }
        Ok(vec![out])
    }
}

struct Xml;

#[derive(Default, Debug)]
struct Element {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
    text: String,
}

fn parse_xml(text: &str) -> Result<Element, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Element> = vec![Element::default()];
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => {
                let mut el = Element { name: String::from_utf8_lossy(e.name().as_ref()).to_string(), ..Default::default() };
                for a in e.attributes().flatten() {
                    el.attrs.push((String::from_utf8_lossy(a.key.as_ref()).to_string(), a.unescape_value().map(|v| v.to_string()).unwrap_or_default()));
                }
                stack.push(el);
            }
            Event::Empty(e) => {
                let mut el = Element { name: String::from_utf8_lossy(e.name().as_ref()).to_string(), ..Default::default() };
                for a in e.attributes().flatten() {
                    el.attrs.push((String::from_utf8_lossy(a.key.as_ref()).to_string(), a.unescape_value().map(|v| v.to_string()).unwrap_or_default()));
                }
                stack.last_mut().unwrap().children.push(el);
            }
            Event::Text(t) => {
                let s = t.unescape().map_err(|e| e.to_string())?;
                stack.last_mut().unwrap().text.push_str(&s);
            }
            Event::CData(t) => stack.last_mut().unwrap().text.push_str(&String::from_utf8_lossy(&t)),
            Event::End(_) => {
                let el = stack.pop().ok_or("unbalanced XML")?;
                stack.last_mut().ok_or("unbalanced XML")?.children.push(el);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut root = stack.pop().ok_or("empty XML")?;
    root.children.pop().ok_or_else(|| "the XML has no root element".to_string())
}

/// xml2js-style conversion (explicitArray false, mergeAttrs true by default).
fn element_to_json(el: &Element, explicit_array: bool, merge_attrs: bool) -> Value {
    let text = el.text.trim();
    if el.attrs.is_empty() && el.children.is_empty() {
        return json!(el.text);
    }
    let mut obj = Map::new();
    if merge_attrs {
        for (k, v) in &el.attrs {
            obj.insert(k.clone(), json!(v));
        }
    } else if !el.attrs.is_empty() {
        obj.insert("$".into(), Value::Object(el.attrs.iter().map(|(k, v)| (k.clone(), json!(v))).collect()));
    }
    // Names seen more than once become arrays; with explicitArray all do.
    let mut repeated = std::collections::HashSet::new();
    for child in &el.children {
        let v = element_to_json(child, explicit_array, merge_attrs);
        match obj.get_mut(&child.name) {
            Some(Value::Array(a)) if explicit_array || repeated.contains(&child.name) => a.push(v),
            Some(existing) => {
                let prev = existing.take();
                *existing = json!([prev, v]);
                repeated.insert(child.name.clone());
            }
            None => {
                obj.insert(child.name.clone(), if explicit_array { json!([v]) } else { v });
            }
        }
    }
    if !text.is_empty() {
        obj.insert("_".into(), json!(text));
    }
    Value::Object(obj)
}

fn json_to_xml(name: &str, v: &Value, out: &mut String) {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    match v {
        Value::Array(a) => a.iter().for_each(|x| json_to_xml(name, x, out)),
        Value::Object(o) => {
            out.push_str(&format!("<{name}>"));
            for (k, x) in o {
                json_to_xml(k, x, out);
            }
            out.push_str(&format!("</{name}>"));
        }
        Value::Null => out.push_str(&format!("<{name}/>")),
        Value::String(s) => out.push_str(&format!("<{name}>{}</{name}>", esc(s))),
        other => out.push_str(&format!("<{name}>{other}</{name}>")),
    }
}

#[async_trait::async_trait]
impl NodeType for Xml {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.xml"
    }
    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        for (i, item) in ctx.input().to_vec().iter().enumerate() {
            let mode = ctx.param_str("mode", i, "jsonToxml")?;
            let field = ctx.param_str("dataPropertyName", i, "data")?;
            if mode == "xmlToJson" {
                let text = item.json.get(&field).and_then(Value::as_str).ok_or_else(|| NodeError::new(format!("Item has no JSON property called '{field}'")).at(i))?;
                let root = parse_xml(text).map_err(|e| NodeError::new(format!("The XML could not be parsed: {e}")).at(i))?;
                let explicit_array = ctx.param_bool("options.explicitArray", i, false)?;
                let merge_attrs = ctx.param_bool("options.mergeAttrs", i, true)?;
                let mut json = Map::new();
                json.insert(root.name.clone(), element_to_json(&root, explicit_array, merge_attrs));
                out.push(Item::new(json).paired(i));
            } else {
                let root = ctx.param_str("options.rootName", i, "root")?;
                let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
                json_to_xml(&root, &item.json_value(), &mut xml);
                let mut json = Map::new();
                json.insert(field, json!(xml));
                out.push(Item::new(json).paired(i));
            }
        }
        Ok(vec![out])
    }
}
