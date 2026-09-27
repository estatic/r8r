//! HTML (spec §6.6): generate an HTML template, extract content out of HTML
//! with CSS selectors, and convert items into an HTML table.

use super::get_path;
use crate::n8n::expr::stringify;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use scraper::{ElementRef, Html, Node as HtmlNodeKind, Selector};
use serde_json::{json, Map, Value};

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Html_)]
}

struct Html_;

#[async_trait::async_trait]
impl NodeType for Html_ {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.html"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let operation = ctx.param_str("operation", 0, "generateHtmlTemplate")?;
        match operation.as_str() {
            "convertToHtmlTable" => convert_to_html_table(ctx),
            "extractHtmlContent" => extract_html_content(ctx),
            _ => generate_html_template(ctx),
        }
    }
}

// ---- generateHtmlTemplate --------------------------------------------------

fn generate_html_template(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let mut out = Vec::new();
    for (i, _item) in ctx.input().to_vec().iter().enumerate() {
        let raw = ctx.raw_param("html").cloned().unwrap_or_else(|| Value::String(String::new()));
        let raw_str = match &raw {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        // The HTML Template field evaluates every `{{ }}` block it contains
        // directly (n8n disables the field's own expression toggle -- see
        // `noDataExpression`/`editor: htmlEditor` on this parameter), so an
        // optional leading `=` (r8r's usual "this whole value is an
        // expression" marker) is stripped rather than required.
        let template_src = raw_str.strip_prefix('=').unwrap_or(&raw_str);
        let ev = ctx.evaluator()?;
        ev.set_item(i).map_err(|e| NodeError::from(e).at(i))?;
        let rendered = ev.template(template_src).map_err(|e| NodeError::from(e).at(i))?;
        let html_string = match rendered {
            Some(Value::String(s)) => s,
            Some(other) => stringify(Some(&other)),
            None => String::new(),
        };
        let mut json = Map::new();
        json.insert("html".into(), json!(html_string));
        out.push(Item::new(json).paired(i));
    }
    Ok(vec![out])
}

// ---- extractHtmlContent ----------------------------------------------------

fn value_as_html_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn html_source_strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(value_as_html_string).collect(),
        other => vec![value_as_html_string(other)],
    }
}

/// `<select>`/`<input>`/`<textarea>` "value", matching cheerio's `.val()`.
fn extract_form_value(el: ElementRef) -> Option<String> {
    match el.value().name() {
        "textarea" => Some(el.text().collect::<String>()),
        "select" => {
            let option_sel = Selector::parse("option").ok()?;
            let options: Vec<ElementRef> = el.select(&option_sel).collect();
            let option_value = |o: &ElementRef| o.attr("value").map(str::to_string).unwrap_or_else(|| o.text().collect());
            let selected: Vec<String> = options.iter().filter(|o| o.attr("selected").is_some()).map(&option_value).collect();
            if !selected.is_empty() {
                Some(selected.join(","))
            } else {
                options.first().map(&option_value)
            }
        }
        _ => Some(el.attr("value").unwrap_or("").to_string()),
    }
}

/// Extraction for a single matched element (used directly for
/// `returnArray`, and as the "first element" behind `attribute`/`html`/
/// `value` when a selector matches more than one element -- cheerio applies
/// those to the first element of a selection).
fn extract_one(el: ElementRef, return_value: &str, attribute: &str, skip_selectors: &str, node_version: f64) -> Option<String> {
    match return_value {
        "attribute" => el.attr(attribute).map(str::to_string),
        "html" => {
            let html = el.inner_html();
            if html.is_empty() {
                None
            } else {
                Some(html)
            }
        }
        "value" => extract_form_value(el),
        _ => {
            if node_version <= 1.1 {
                let text: String = el.text().collect();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            } else {
                Some(html_to_text(&el.inner_html(), skip_selectors))
            }
        }
    }
}

/// Extraction over a whole (possibly multi-element) selector match, matching
/// cheerio: `.text()` aggregates every matched element (pre-1.2 behaviour);
/// `.attr()`/`.html()`/`.val()` (and the >1.1 `.html()`-based text
/// extraction) only ever look at the first one.
fn extract_aggregate(matches: &[ElementRef], return_value: &str, attribute: &str, skip_selectors: &str, node_version: f64) -> Option<String> {
    if return_value == "text" && node_version <= 1.1 {
        let text: String = matches.iter().flat_map(|el| el.text()).collect();
        return if text.is_empty() { None } else { Some(text) };
    }
    matches.first().and_then(|el| extract_one(*el, return_value, attribute, skip_selectors, node_version))
}

fn apply_value_options(value: Option<String>, trim_values: bool, clean_up_text: bool) -> Option<String> {
    let mut v = value?;
    if trim_values {
        v = v.trim().to_string();
    }
    if clean_up_text {
        v = v.trim().to_string();
        v = v.replace("\r\n", "").replace(['\n', '\r'], "");
        let mut collapsed = String::with_capacity(v.len());
        let mut in_whitespace = false;
        for ch in v.chars() {
            if ch.is_whitespace() {
                if !in_whitespace {
                    collapsed.push(' ');
                }
                in_whitespace = true;
            } else {
                collapsed.push(ch);
                in_whitespace = false;
            }
        }
        v = collapsed;
    }
    Some(v)
}

/// A crude approximation of the `html-to-text` npm package used by n8n
/// v1.2's text extraction: walks the fragment, drops `<script>`/`<style>`
/// and anything matching `skip_selectors` (comma-separated CSS selectors,
/// n8n's "Skip Selectors" option), and separates block-level elements with
/// newlines. It does not replicate `html-to-text`'s table/list-specific
/// layout or link/image formatting.
fn html_to_text(html: &str, skip_selectors: &str) -> String {
    const BLOCK_TAGS: &[&str] = &[
        "p", "div", "h1", "h2", "h3", "h4", "h5", "h6", "li", "tr", "table", "ul", "ol", "blockquote", "section", "article", "header", "footer", "form",
        "pre",
    ];
    let fragment = Html::parse_fragment(html);
    let skip: Vec<Selector> = skip_selectors.split(',').map(str::trim).filter(|s| !s.is_empty()).filter_map(|s| Selector::parse(s).ok()).collect();

    fn render(node: ego_tree::NodeRef<'_, HtmlNodeKind>, skip: &[Selector], block_tags: &[&str], out: &mut String) {
        match node.value() {
            HtmlNodeKind::Text(t) => out.push_str(t),
            HtmlNodeKind::Element(_) => {
                let Some(el) = ElementRef::wrap(node) else { return };
                let tag = el.value().name();
                if tag == "script" || tag == "style" {
                    return;
                }
                if skip.iter().any(|s| s.matches(&el)) {
                    return;
                }
                if tag == "br" {
                    out.push('\n');
                    return;
                }
                for child in node.children() {
                    render(child, skip, block_tags, out);
                }
                if block_tags.contains(&tag) {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }

    let mut out = String::new();
    for child in fragment.tree.root().children() {
        render(child, &skip, BLOCK_TAGS, &mut out);
    }
    out
}

fn extract_html_content(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let node_version = ctx.node.type_version;
    let mut out = Vec::new();
    for (i, item) in ctx.input().to_vec().iter().enumerate() {
        let source_data = ctx.param_str("sourceData", i, "json")?;
        let data_property_name = ctx.param_str("dataPropertyName", i, "data")?;
        let trim_values = ctx.param_bool("options.trimValues", i, true)?;
        let clean_up_text = ctx.param_bool("options.cleanUpText", i, true)?;
        let extraction_values = ctx.param("extractionValues", i)?;
        let values: Vec<Value> = extraction_values.get("values").and_then(Value::as_array).cloned().unwrap_or_default();

        let html_array: Vec<String> = if source_data == "binary" {
            let field = item
                .binary
                .as_ref()
                .and_then(|b| b.get(&data_property_name))
                .ok_or_else(|| NodeError::new(format!("This operation expects the node's input data to contain a binary file '{data_property_name}', but none was found [item {i}]")).at(i))?;
            let data_b64 = field.get("data").and_then(Value::as_str).unwrap_or("");
            use base64::Engine as _;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data_b64.trim())
                .map_err(|e| NodeError::new(format!("The binary data in '{data_property_name}' is not valid base64: {e}")).at(i))?;
            vec![String::from_utf8_lossy(&bytes).into_owned()]
        } else if node_version <= 1.0 {
            let value = item
                .json
                .get(&data_property_name)
                .ok_or_else(|| NodeError::new(format!("No property named \"{data_property_name}\" exists!")).at(i))?;
            html_source_strings(value)
        } else {
            let item_json = item.json_value();
            let value = get_path(&item_json, &data_property_name)
                .ok_or_else(|| NodeError::new(format!("No property named \"{data_property_name}\" exists!")).at(i))?;
            html_source_strings(value)
        };

        for html in html_array {
            let document = Html::parse_document(&html);
            let mut new_json = Map::new();
            for value_data in &values {
                let key = value_data.get("key").and_then(Value::as_str).unwrap_or("").to_string();
                let css_selector = value_data.get("cssSelector").and_then(Value::as_str).unwrap_or("");
                let return_value = value_data.get("returnValue").and_then(Value::as_str).unwrap_or("text");
                let attribute = value_data.get("attribute").and_then(Value::as_str).unwrap_or("");
                let skip_selectors = value_data.get("skipSelectors").and_then(Value::as_str).unwrap_or("");
                let return_array = value_data.get("returnArray").and_then(Value::as_bool).unwrap_or(false);

                let selector = Selector::parse(css_selector).map_err(|e| NodeError::new(format!("The CSS selector \"{css_selector}\" is not valid: {e:?}")).at(i))?;
                let matches: Vec<ElementRef> = document.select(&selector).collect();

                if return_array {
                    let arr: Vec<Value> = matches
                        .iter()
                        .map(|el| {
                            let raw = extract_one(*el, return_value, attribute, skip_selectors, node_version);
                            match apply_value_options(raw, trim_values, clean_up_text) {
                                Some(s) => json!(s),
                                None => Value::Null,
                            }
                        })
                        .collect();
                    new_json.insert(key, Value::Array(arr));
                } else {
                    let raw = extract_aggregate(&matches, return_value, attribute, skip_selectors, node_version);
                    if let Some(s) = apply_value_options(raw, trim_values, clean_up_text) {
                        new_json.insert(key, json!(s));
                    }
                }
            }
            out.push(Item::new(new_json).paired(i));
        }
    }
    Ok(vec![out])
}

// ---- convertToHtmlTable -----------------------------------------------------

fn capitalize_header(header: &str, capitalize: bool) -> String {
    if !capitalize {
        return header.to_string();
    }
    header
        .split('_')
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// JS-style `String(value)` for a cell's own value (used directly, not
/// inside an array): `undefined` -> "undefined" (a field entirely absent
/// from this item, since other items had it), `null` -> "null".
fn js_cell_to_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(v) => js_to_string(v),
    }
}

/// JS-style `String(value)` for a value already known to exist (used for
/// array elements, where `Array.prototype.join` turns `null`/`undefined`
/// into `""` rather than the literal words).
fn js_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(js_to_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

fn convert_to_html_table(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let items = ctx.input().to_vec();
    if items.is_empty() {
        return Ok(vec![vec![]]);
    }

    let capitalize = ctx.param_bool("options.capitalize", 0, false)?;
    let custom_styling = ctx.param_bool("options.customStyling", 0, false)?;
    let caption = ctx.param_str("options.caption", 0, "")?;
    let table_attributes = ctx.param_str("options.tableAttributes", 0, "")?;
    let header_attributes = ctx.param_str("options.headerAttributes", 0, "")?;

    let (table_style, header_style, cell_style) = if custom_styling {
        ("", "", "")
    } else {
        (
            "style='border-spacing:0; font-family:helvetica,arial,sans-serif'",
            "style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee; text-align:left; color:#888; font-weight:normal'",
            "style='margin:0; padding:7px 20px 7px 0px; border-bottom:1px solid #eee'",
        )
    };

    let mut headers: Vec<String> = Vec::new();
    for item in &items {
        for key in item.json.keys() {
            if !headers.iter().any(|h| h == key) {
                headers.push(key.clone());
            }
        }
    }

    let mut table = format!("<table {table_style} {table_attributes}>");
    if !caption.is_empty() {
        table.push_str(&format!("<caption>{caption}</caption>"));
    }
    table.push_str(&format!("<thead {header_style} {header_attributes}>"));
    table.push_str("<tr>");
    for header in &headers {
        table.push_str(&format!("<th>{}</th>", capitalize_header(header, capitalize)));
    }
    table.push_str("</tr>");
    table.push_str("</thead>");
    table.push_str("<tbody>");
    for (entry_index, item) in items.iter().enumerate() {
        let row_attributes = ctx.param_str("options.rowAttributes", entry_index, "")?;
        table.push_str(&format!("<tr  {row_attributes}>"));
        let cell_attributes = ctx.param_str("options.cellAttributes", entry_index, "")?;
        for header in &headers {
            let value = item.json.get(header);
            table.push_str(&format!("<td {cell_style} {cell_attributes}>"));
            match value {
                Some(Value::Bool(b)) => {
                    let checked = if *b { "checked=\"checked\"" } else { "" };
                    table.push_str(&format!("<input type=\"checkbox\" {checked}/>"));
                }
                _ => table.push_str(&js_cell_to_string(value)),
            }
            table.push_str("</td>");
        }
        table.push_str("</tr>");
    }
    table.push_str("</tbody>");
    table.push_str("</table>");

    let paired: Vec<Value> = (0..items.len()).map(|i| json!({"item": i})).collect();
    let mut json = Map::new();
    json.insert("table".into(), json!(table));
    Ok(vec![vec![Item { json, binary: None, paired_item: Some(Value::Array(paired)) }]])
}
