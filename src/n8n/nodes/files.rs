//! Extract from File / Convert to File (spec §6.6): binary <-> JSON
//! conversions for CSV, XLSX, JSON, plain text and HTML tables.
//!
//! Implemented operations:
//! - `extractFromFile`: csv, xlsx, fromJson, text, html.
//! - `convertToFile`: csv, xlsx, toJson, toText, html, toBinary.
//!
//! Everything else (ics, ods, pdf, rtf, xls for extraction; iCal, ods, rtf,
//! xls for conversion) returns a clear "not supported natively yet" error
//! rather than silently doing the wrong thing.

use super::{flatten_json, get_path};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use base64::Engine;
use calamine::Reader;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(ExtractFromFile), Box::new(ConvertToFile)]
}

// ---- binary helpers --------------------------------------------------

fn extension_for_mime(mime: &str) -> String {
    match mime {
        "text/csv" => "csv",
        "text/html" => "html",
        "application/json" => "json",
        "text/plain" => "txt",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/rtf" => "rtf",
        "text/calendar" => "ics",
        "application/pdf" => "pdf",
        "application/octet-stream" => "bin",
        other => return other.split('/').nth(1).unwrap_or("bin").split(';').next().unwrap_or("bin").to_string(),
    }
    .to_string()
}

/// Human-readable byte count, the same shape n8n's `prettyBytes`-based
/// `fileSize` uses (e.g. "512 B", "1.2 kB").
fn pretty_bytes(n: usize) -> String {
    if n < 1000 {
        format!("{n} B")
    } else if n < 1_000_000 {
        format!("{:.1} kB", n as f64 / 1000.0)
    } else {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    }
}

/// Builds a binary data entry (`data`, `mimeType`, `fileExtension`,
/// `fileSize`, `fileName`) the way n8n's `prepareBinaryData` does.
fn make_binary(data: &[u8], mime: &str, file_name: Option<String>) -> Value {
    let ext = extension_for_mime(mime);
    let name = file_name.filter(|f| !f.is_empty()).unwrap_or_else(|| format!("file.{ext}"));
    json!({
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime,
        "fileExtension": ext,
        "fileSize": pretty_bytes(data.len()),
        "fileName": name,
    })
}

/// Looks up a binary property on an item, with the same two-stage error n8n
/// raises: no binary data at all, vs. binary data but not this property.
fn binary_entry<'a>(item: &'a Item, prop: &str, idx: usize) -> NodeResult<&'a Value> {
    let binary = item.binary.as_ref().ok_or_else(|| {
        NodeError::new(format!(
            "This operation expects the node's input data to contain a binary file '{prop}', but none was found [item {idx}]"
        ))
        .at(idx)
    })?;
    binary
        .get(prop)
        .ok_or_else(|| NodeError::new(format!("The item has no binary field '{prop}' [item {idx}]")).at(idx))
}

fn binary_bytes(entry: &Value, idx: usize) -> NodeResult<Vec<u8>> {
    let data = entry.get("data").and_then(Value::as_str).ok_or_else(|| NodeError::new("Binary data has no \"data\" payload").at(idx))?;
    base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|e| NodeError::new(format!("Binary data is not valid base64: {e}")).at(idx))
}

/// Decodes bytes to text. Only `utf8`/`utf-8` and `latin1`/`iso-8859-1` are
/// handled specially (as r8r has no general charset conversion crate);
/// anything else falls back to lossy UTF-8, same as an unrecognised
/// encoding falling through in n8n's `iconv-lite`-based decoder would
/// produce garbled (not erroring) output.
fn decode_text_with(encoding: &str, bytes: &[u8]) -> String {
    match encoding.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "latin1" | "iso88591" | "ascii" => bytes.iter().map(|&b| b as char).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_text(ctx: &ExecCtx<'_>, i: usize, bytes: &[u8]) -> NodeResult<String> {
    let encoding = ctx.param_str("options.encoding", i, "utf8")?;
    Ok(decode_text_with(&encoding, bytes))
}

fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

fn is_empty_str(v: &Value) -> bool {
    matches!(v, Value::String(s) if s.is_empty())
}

/// Builds items from string-cell rows (CSV, HTML), sharing the headerRow /
/// includeEmptyCells behaviour n8n's spreadsheet operations use.
fn rows_to_items(headers: Option<&[String]>, rows: Vec<Vec<Value>>, include_empty: bool, i: usize) -> Vec<Item> {
    match headers {
        Some(h) => rows
            .into_iter()
            .map(|row| {
                let mut json = Map::new();
                for (name, v) in h.iter().zip(row) {
                    if !include_empty && is_empty_str(&v) {
                        continue;
                    }
                    json.insert(name.clone(), v);
                }
                Item::new(json).paired(i)
            })
            .collect(),
        None => rows
            .into_iter()
            .map(|row| {
                let arr: Vec<Value> = if include_empty { row } else { row.into_iter().filter(|v| !is_empty_str(v)).collect() };
                let mut json = Map::new();
                json.insert("row".to_string(), Value::Array(arr));
                Item::new(json).paired(i)
            })
            .collect(),
    }
}

// ---- Extract from File -------------------------------------------------

pub struct ExtractFromFile;

#[async_trait::async_trait]
impl NodeType for ExtractFromFile {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.extractFromFile"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let mut out = Vec::new();
        for i in 0..ctx.input().len() {
            match extract_item(ctx, i) {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}

fn extract_item(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Vec<Item>> {
    let operation = ctx.param_str("operation", i, "csv")?;
    let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
    let item = &ctx.input()[i];
    match operation.as_str() {
        "csv" => {
            let entry = binary_entry(item, &binary_prop, i)?;
            let bytes = binary_bytes(entry, i)?;
            extract_csv(ctx, i, &bytes)
        }
        "html" => {
            let entry = binary_entry(item, &binary_prop, i)?;
            let bytes = binary_bytes(entry, i)?;
            extract_html_table(ctx, i, &bytes)
        }
        "xlsx" => {
            let entry = binary_entry(item, &binary_prop, i)?;
            let bytes = binary_bytes(entry, i)?;
            extract_spreadsheet(ctx, i, &bytes)
        }
        "fromJson" => {
            let entry = binary_entry(item, &binary_prop, i)?;
            let bytes = binary_bytes(entry, i)?;
            let text = decode_text(ctx, i, &bytes)?;
            let value: Value = if text.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&text)
                    .map_err(|e| NodeError::new("The file selected in 'Input Binary Field' is not in JSON format").describe(e.to_string()).at(i))?
            };
            Ok(vec![destination_item(ctx, i, item, &binary_prop, value)?])
        }
        "text" => {
            let entry = binary_entry(item, &binary_prop, i)?;
            let bytes = binary_bytes(entry, i)?;
            let text = decode_text(ctx, i, &bytes)?;
            Ok(vec![destination_item(ctx, i, item, &binary_prop, Value::String(text))?])
        }
        other @ ("ics" | "ods" | "pdf" | "rtf" | "xls") => {
            Err(NodeError::new(format!("Extract from File: operation \"{other}\" is not supported natively yet")).at(i))
        }
        other => Err(NodeError::new(format!("Unknown operation \"{other}\" for Extract from File")).at(i)),
    }
}

/// Builds the single output item for `fromJson`/`text`: the decoded value
/// under `destinationKey` (default `data`), with the consumed binary
/// property removed from the item's binary map (other binary properties,
/// if any, are kept -- matching n8n's default `keepSource` behaviour).
fn destination_item(ctx: &ExecCtx<'_>, i: usize, item: &Item, binary_prop: &str, value: Value) -> NodeResult<Item> {
    let dest = ctx.param_str("destinationKey", i, "data")?;
    let mut json = Map::new();
    if dest.is_empty() {
        json.insert("data".to_string(), value);
    } else {
        super::set_path(&mut json, &dest, value);
    }
    let mut binary = item.binary.clone();
    if let Some(b) = binary.as_mut() {
        b.remove(binary_prop);
    }
    if binary.as_ref().is_some_and(Map::is_empty) {
        binary = None;
    }
    Ok(Item { json, binary, paired_item: None }.paired(i))
}

fn csv_error(e: impl std::fmt::Display, i: usize) -> NodeError {
    NodeError::new("The file selected in 'Input Binary Field' is not in csv format").describe(e.to_string()).at(i)
}

fn extract_csv(ctx: &ExecCtx<'_>, i: usize, bytes: &[u8]) -> NodeResult<Vec<Item>> {
    let delimiter = ctx.param_str("options.delimiter", i, ",")?;
    let delim_byte = delimiter.as_bytes().first().copied().unwrap_or(b',');
    let header_row = ctx.param_bool("options.headerRow", i, true)?;
    let include_empty = ctx.param_bool("options.includeEmptyCells", i, false)?;
    let from_line = ctx.param_f64("options.fromLine", i, 0.0)?.max(0.0) as usize;
    let max_rows = ctx.param_f64("options.maxRowCount", i, -1.0)?;
    let encoding = ctx.param_str("options.encoding", i, "utf-8")?;

    let text = decode_text_with(&encoding, bytes);
    let content: String = if from_line > 0 { text.lines().skip(from_line).collect::<Vec<_>>().join("\n") } else { text };

    let mut reader = csv::ReaderBuilder::new().delimiter(delim_byte).has_headers(header_row).from_reader(content.as_bytes());

    if header_row {
        let headers: Vec<String> = reader.headers().map_err(|e| csv_error(e, i))?.iter().map(String::from).collect();
        let mut rows = Vec::new();
        for (row_idx, record) in reader.records().enumerate() {
            if max_rows >= 0.0 && row_idx as f64 >= max_rows {
                break;
            }
            let record = record.map_err(|e| csv_error(e, i))?;
            rows.push(record.iter().map(|v| Value::String(v.to_string())).collect());
        }
        Ok(rows_to_items(Some(&headers), rows, include_empty, i))
    } else {
        let mut rows = Vec::new();
        for (row_idx, record) in reader.records().enumerate() {
            if max_rows >= 0.0 && row_idx as f64 >= max_rows {
                break;
            }
            let record = record.map_err(|e| csv_error(e, i))?;
            rows.push(record.iter().map(|v| Value::String(v.to_string())).collect());
        }
        Ok(rows_to_items(None, rows, include_empty, i))
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(out.trim())
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay_lower = haystack[from..].to_ascii_lowercase();
    hay_lower.find(&needle.to_ascii_lowercase()).map(|p| p + from)
}

/// Extracts `<tr>`/`<td>`/`<th>` cell text from the first `<table>` found.
/// A hand-rolled scanner rather than a full HTML parser: good enough for
/// the well-formed single-table markup `Convert to File` itself produces,
/// and for simple hand-authored fixtures.
fn parse_html_table_rows(html: &str) -> Option<Vec<Vec<String>>> {
    let table_start = find_ci(html, "<table", 0)?;
    let body_start = html[table_start..].find('>')? + table_start + 1;
    let table_end = find_ci(html, "</table", body_start)?;
    let table = &html[body_start..table_end];

    let mut rows = Vec::new();
    let mut pos = 0;
    while let Some(tr_start) = find_ci(table, "<tr", pos) {
        let tr_body_start = table[tr_start..].find('>')? + tr_start + 1;
        let Some(tr_end) = find_ci(table, "</tr", tr_body_start) else { break };
        let row_html = &table[tr_body_start..tr_end];

        let mut cells = Vec::new();
        let mut cpos = 0;
        while let Some(td_start) = find_ci(row_html, "<t", cpos)
            .filter(|&p| row_html.as_bytes()[p..].get(2).is_some_and(|b| *b == b'd' || *b == b'D' || *b == b'h' || *b == b'H'))
        {
            let tag = &row_html[td_start + 1..td_start + 2];
            let close_tag = format!("</{tag}");
            let cell_body_start = row_html[td_start..].find('>')? + td_start + 1;
            let Some(cell_end) = find_ci(row_html, &close_tag, cell_body_start) else { break };
            cells.push(strip_tags(&row_html[cell_body_start..cell_end]));
            cpos = cell_end + close_tag.len();
        }
        rows.push(cells);
        pos = tr_end + 4;
    }
    Some(rows)
}

fn extract_html_table(ctx: &ExecCtx<'_>, i: usize, bytes: &[u8]) -> NodeResult<Vec<Item>> {
    let header_row = ctx.param_bool("options.headerRow", i, true)?;
    let include_empty = ctx.param_bool("options.includeEmptyCells", i, false)?;
    let text = decode_text(ctx, i, bytes)?;
    let mut rows = parse_html_table_rows(&text)
        .ok_or_else(|| NodeError::new("The file selected in 'Input Binary Field' is not in html format").at(i))?;
    if header_row {
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let headers = rows.remove(0);
        let data: Vec<Vec<Value>> = rows.into_iter().map(|r| r.into_iter().map(Value::String).collect()).collect();
        Ok(rows_to_items(Some(&headers), data, include_empty, i))
    } else {
        let data: Vec<Vec<Value>> = rows.into_iter().map(|r| r.into_iter().map(Value::String).collect()).collect();
        Ok(rows_to_items(None, data, include_empty, i))
    }
}

fn cell_to_json(c: &calamine::Data) -> Value {
    match c {
        calamine::Data::Empty => Value::String(String::new()),
        calamine::Data::String(s) => Value::String(s.clone()),
        calamine::Data::Float(f) => number(*f),
        calamine::Data::Int(n) => json!(*n),
        calamine::Data::Bool(b) => json!(*b),
        other => Value::String(other.to_string()),
    }
}

/// Reads xlsx (via `calamine`). `options.range` (arbitrary A1 notation) and
/// `options.rawData` (n8n's "don't format numbers/dates" toggle) are not
/// implemented -- calamine already returns typed, unformatted cell values,
/// which is the closest available approximation.
fn extract_spreadsheet(ctx: &ExecCtx<'_>, i: usize, bytes: &[u8]) -> NodeResult<Vec<Item>> {
    let header_row = ctx.param_bool("options.headerRow", i, true)?;
    let include_empty = ctx.param_bool("options.includeEmptyCells", i, false)?;
    let sheet_name = ctx.param_str("options.sheetName", i, "")?;

    let cursor = std::io::Cursor::new(bytes.to_vec());
    let mut workbook = calamine::open_workbook_from_rs::<calamine::Xlsx<_>, _>(cursor)
        .map_err(|e| NodeError::new("The file selected in 'Input Binary Field' is not in xlsx format").describe(e.to_string()).at(i))?;

    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err(NodeError::new("Spreadsheet does not have any sheets!").at(i));
    }
    let chosen = if sheet_name.is_empty() {
        sheet_names[0].clone()
    } else {
        if !sheet_names.iter().any(|n| n == &sheet_name) {
            return Err(NodeError::new(format!("Spreadsheet does not contain sheet called \"{sheet_name}\"!")).at(i));
        }
        sheet_name
    };
    let range = workbook
        .worksheet_range(&chosen)
        .map_err(|e| NodeError::new(format!("Could not read sheet \"{chosen}\"")).describe(e.to_string()).at(i))?;

    let mut rows_iter = range.rows();
    if header_row {
        let Some(header) = rows_iter.next() else { return Ok(vec![]) };
        let headers: Vec<String> = header.iter().map(|c| c.to_string()).collect();
        let mut out = Vec::new();
        for row in rows_iter {
            let mut json = Map::new();
            for (name, c) in headers.iter().zip(row.iter()) {
                if !include_empty && matches!(c, calamine::Data::Empty) {
                    continue;
                }
                json.insert(name.clone(), cell_to_json(c));
            }
            out.push(Item::new(json).paired(i));
        }
        Ok(out)
    } else {
        let mut out = Vec::new();
        for row in rows_iter {
            let arr: Vec<Value> = row.iter().filter(|c| include_empty || !matches!(c, calamine::Data::Empty)).map(cell_to_json).collect();
            let mut json = Map::new();
            json.insert("row".to_string(), Value::Array(arr));
            out.push(Item::new(json).paired(i));
        }
        Ok(out)
    }
}

// ---- Convert to File -----------------------------------------------------

pub struct ConvertToFile;

#[async_trait::async_trait]
impl NodeType for ConvertToFile {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.convertToFile"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let operation = ctx.param_str("operation", 0, "csv")?;
        match operation.as_str() {
            "csv" | "xlsx" | "html" => write_spreadsheet(ctx, &operation),
            "toJson" => write_to_json(ctx),
            "toText" => write_to_text(ctx),
            "toBinary" => write_to_binary(ctx),
            other @ ("iCal" | "ods" | "rtf" | "xls") => Err(NodeError::new(format!("Convert to File: operation \"{other}\" is not supported natively yet"))),
            other => Err(NodeError::new(format!("Unknown operation \"{other}\" for Convert to File"))),
        }
    }
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn collect_columns(items: &[Item]) -> Vec<String> {
    let mut cols = Vec::new();
    for item in items {
        for (k, _) in flatten_json(&item.json) {
            if !cols.contains(&k) {
                cols.push(k);
            }
        }
    }
    cols
}

fn row_map(item: &Item) -> HashMap<String, Value> {
    flatten_json(&item.json).into_iter().collect()
}

fn write_csv(items: &[Item], columns: &[String], header_row: bool, delimiter: u8) -> Vec<u8> {
    let mut wtr = csv::WriterBuilder::new().delimiter(delimiter).from_writer(Vec::new());
    if header_row {
        wtr.write_record(columns).expect("writing to an in-memory buffer cannot fail");
    }
    for item in items {
        let map = row_map(item);
        let row: Vec<String> = columns.iter().map(|c| map.get(c).map(cell_text).unwrap_or_default()).collect();
        wtr.write_record(&row).expect("writing to an in-memory buffer cannot fail");
    }
    wtr.into_inner().expect("in-memory buffer flush cannot fail")
}

fn write_xlsx(items: &[Item], columns: &[String], header_row: bool, sheet_name: &str) -> NodeResult<Vec<u8>> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let sheet = wb.add_worksheet();
    sheet.set_name(sheet_name).map_err(|e| NodeError::new(format!("Invalid sheet name \"{sheet_name}\": {e}")))?;

    let mut r: u32 = 0;
    if header_row {
        for (c, name) in columns.iter().enumerate() {
            sheet.write_string(r, c as u16, name).map_err(|e| NodeError::new(e.to_string()))?;
        }
        r += 1;
    }
    for item in items {
        let map = row_map(item);
        for (c, name) in columns.iter().enumerate() {
            match map.get(name) {
                None | Some(Value::Null) => {}
                Some(Value::String(s)) => {
                    sheet.write_string(r, c as u16, s).map_err(|e| NodeError::new(e.to_string()))?;
                }
                Some(Value::Number(n)) => {
                    sheet.write_number(r, c as u16, n.as_f64().unwrap_or(0.0)).map_err(|e| NodeError::new(e.to_string()))?;
                }
                Some(Value::Bool(b)) => {
                    sheet.write_boolean(r, c as u16, *b).map_err(|e| NodeError::new(e.to_string()))?;
                }
                Some(other) => {
                    sheet.write_string(r, c as u16, other.to_string()).map_err(|e| NodeError::new(e.to_string()))?;
                }
            }
        }
        r += 1;
    }
    wb.save_to_buffer().map_err(|e| NodeError::new(e.to_string()))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn write_html_table(items: &[Item], columns: &[String], header_row: bool) -> Vec<u8> {
    let mut s = String::from("<table>");
    if header_row {
        s.push_str("<tr>");
        for c in columns {
            s.push_str(&format!("<th>{}</th>", escape_html(c)));
        }
        s.push_str("</tr>");
    }
    for item in items {
        let map = row_map(item);
        s.push_str("<tr>");
        for c in columns {
            let text = map.get(c).map(cell_text).unwrap_or_default();
            s.push_str(&format!("<td>{}</td>", escape_html(&text)));
        }
        s.push_str("</tr>");
    }
    s.push_str("</table>");
    s.into_bytes()
}

/// Combines every input item into a single spreadsheet-like binary file
/// (csv/xlsx/html), the way n8n's `toFile`/`iCal`-style operations do: one
/// output item, `pairedItem` covering every input index, and (when
/// `continueOnFail` is set) a single error item on failure rather than one
/// per input item.
fn write_spreadsheet(ctx: &mut ExecCtx<'_>, format: &str) -> NodeResult<NodeOutput> {
    let items = ctx.input().to_vec();
    let paired: Vec<Value> = (0..items.len()).map(|i| json!({"item": i})).collect();

    let result: NodeResult<Item> = (|| {
        let binary_prop = ctx.param_str("binaryPropertyName", 0, "data")?;
        let header_row = ctx.param_bool("options.headerRow", 0, true)?;
        let sheet_name = ctx.param_str("options.sheetName", 0, "Sheet")?;
        let delimiter = ctx.param_str("options.delimiter", 0, ",")?;
        let delim_byte = delimiter.as_bytes().first().copied().unwrap_or(b',');
        let file_name_opt = ctx.param_str("options.fileName", 0, "")?;
        let columns = collect_columns(&items);

        let (bytes, mime) = match format {
            "csv" => (write_csv(&items, &columns, header_row, delim_byte), "text/csv"),
            "xlsx" => (write_xlsx(&items, &columns, header_row, &sheet_name)?, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
            "html" => (write_html_table(&items, &columns, header_row), "text/html"),
            _ => unreachable!("dispatched only for csv/xlsx/html"),
        };
        let name = if file_name_opt.is_empty() { format!("spreadsheet.{format}") } else { file_name_opt };
        let mut binary = Map::new();
        binary.insert(binary_prop, make_binary(&bytes, mime, Some(name)));
        Ok(Item { json: Map::new(), binary: Some(binary), paired_item: Some(Value::Array(paired.clone())) })
    })();

    match result {
        Ok(item) => Ok(vec![vec![item]]),
        Err(e) if ctx.continue_on_fail() => {
            let mut json = Map::new();
            json.insert("error".into(), json!(e.message));
            Ok(vec![vec![Item { json, binary: None, paired_item: Some(Value::Array(paired)) }]])
        }
        Err(e) => Err(e),
    }
}

/// `mode: "once"` combines every item's json into one array file;
/// `mode: "each"` writes one file per item, containing just that item's
/// json (not wrapped in an array) -- matching n8n's `toJson` exactly.
fn write_to_json(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let mode = ctx.param_str("mode", 0, "once")?;
    if mode == "once" {
        let items = ctx.input().to_vec();
        let paired: Vec<Value> = (0..items.len()).map(|i| json!({"item": i})).collect();
        let result: NodeResult<Item> = (|| {
            let binary_prop = ctx.param_str("binaryPropertyName", 0, "data")?;
            let format_pretty = ctx.param_bool("options.format", 0, false)?;
            let file_name = ctx.param_str("options.fileName", 0, "")?;
            let arr: Vec<Value> = items.iter().map(Item::json_value).collect();
            let bytes = if format_pretty { serde_json::to_vec_pretty(&arr) } else { serde_json::to_vec(&arr) }
                .map_err(|e| NodeError::new(format!("could not serialise items to JSON: {e}")))?;
            let name = if file_name.is_empty() { "file.json".to_string() } else { file_name };
            let mut binary = Map::new();
            binary.insert(binary_prop, make_binary(&bytes, "application/json", Some(name)));
            Ok(Item { json: Map::new(), binary: Some(binary), paired_item: Some(Value::Array(paired.clone())) })
        })();
        match result {
            Ok(item) => Ok(vec![vec![item]]),
            Err(e) if ctx.continue_on_fail() => {
                let mut json = Map::new();
                json.insert("error".into(), json!(e.message));
                Ok(vec![vec![Item { json, binary: None, paired_item: Some(Value::Array(paired)) }]])
            }
            Err(e) => Err(e),
        }
    } else {
        let mut out = Vec::new();
        for i in 0..ctx.input().len() {
            let result: NodeResult<Item> = (|| {
                let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
                let format_pretty = ctx.param_bool("options.format", i, false)?;
                let file_name = ctx.param_str("options.fileName", i, "")?;
                let value = ctx.input()[i].json_value();
                let bytes = if format_pretty { serde_json::to_vec_pretty(&value) } else { serde_json::to_vec(&value) }
                    .map_err(|e| NodeError::new(format!("could not serialise item to JSON: {e}")))?;
                let name = if file_name.is_empty() { "file.json".to_string() } else { file_name };
                let mut binary = Map::new();
                binary.insert(binary_prop, make_binary(&bytes, "application/json", Some(name)));
                Ok(Item { json: Map::new(), binary: Some(binary), paired_item: None }.paired(i))
            })();
            match result {
                Ok(item) => out.push(item),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}

fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn write_to_text(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let mut out = Vec::new();
    for i in 0..ctx.input().len() {
        let result: NodeResult<Item> = (|| {
            let source = ctx.param_str("sourceProperty", i, "")?;
            if source.is_empty() {
                return Err(NodeError::new("Convert to File (toText): \"Text Input Field\" is required").at(i));
            }
            let value = get_path(&ctx.input()[i].json_value(), &source)
                .cloned()
                .ok_or_else(|| NodeError::new(format!("The value in \"{source}\" is not set")).at(i))?;
            let text = value_to_text(&value);
            let file_name = ctx.param_str("options.fileName", i, "")?;
            let name = if file_name.is_empty() { "file.txt".to_string() } else { file_name };
            let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
            let mut binary = Map::new();
            binary.insert(binary_prop, make_binary(text.as_bytes(), "text/plain", Some(name)));
            Ok(Item { json: Map::new(), binary: Some(binary), paired_item: None }.paired(i))
        })();
        match result {
            Ok(item) => out.push(item),
            Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
            Err(e) => return Err(e),
        }
    }
    Ok(vec![out])
}

/// Reads a base64 string from `sourceProperty` and writes it out as
/// arbitrary binary data. n8n's v1 `dataIsBase64: false` (encode a plain
/// string with a text encoding instead) is not implemented; base64 is the
/// default/only mode from v1.1 on and is what "Move file to base64 string" +
/// "Convert to File" round trips need.
fn write_to_binary(ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
    let mut out = Vec::new();
    for i in 0..ctx.input().len() {
        let result: NodeResult<Item> = (|| {
            let source = ctx.param_str("sourceProperty", i, "")?;
            if source.is_empty() {
                return Err(NodeError::new("Convert to File (toBinary): \"Base64 Input Field\" is required").at(i));
            }
            let value = get_path(&ctx.input()[i].json_value(), &source)
                .cloned()
                .ok_or_else(|| NodeError::new(format!("The value in \"{source}\" is not set")).at(i))?;
            let Value::String(text) = &value else {
                return Err(NodeError::new(format!("The value in \"{source}\" is not a base64 string")).at(i));
            };
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(text.trim())
                .map_err(|e| NodeError::new(format!("The value in \"{source}\" is not valid base64")).describe(e.to_string()).at(i))?;
            let mime = ctx.param_str("options.mimeType", i, "")?;
            let mime = if mime.is_empty() { "application/octet-stream".to_string() } else { mime };
            let file_name = ctx.param_str("options.fileName", i, "")?;
            let name = if file_name.is_empty() { format!("file.{}", extension_for_mime(&mime)) } else { file_name };
            let binary_prop = ctx.param_str("binaryPropertyName", i, "data")?;
            let mut binary = Map::new();
            binary.insert(binary_prop, make_binary(&bytes, &mime, Some(name)));
            Ok(Item { json: Map::new(), binary: Some(binary), paired_item: None }.paired(i))
        })();
        match result {
            Ok(item) => out.push(item),
            Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
            Err(e) => return Err(e),
        }
    }
    Ok(vec![out])
}
