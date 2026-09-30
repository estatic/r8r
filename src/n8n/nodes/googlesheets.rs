//! Google Sheets node (spec §6.6), v2.x (4.5/4.6/4.7), faithful to n8n's
//! `nodes/Google/Sheet/v2/*`. Implements the `sheet` resource's append,
//! appendOrUpdate, clear, create, delete (rows/columns), read, remove
//! (delete sheet) and update operations, plus the `spreadsheet` resource's
//! create and deleteSpreadsheet. The trigger node is out of scope. Anything
//! else returns `Google Sheets "<resource>" / "<operation>" is not
//! supported natively yet`.
//!
//! Faithful quirks kept from n8n's router/GoogleSheet helper:
//! - Every operation (except `create`) resolves the `sheetName` resource
//!   locator against a live `GET /v4/spreadsheets/{id}?fields=sheets.properties`
//!   call, exactly like n8n's router does, so tests must stub that request.
//! - `delete`/`remove` are parameter-compatible with n8n's router, which
//!   substitutes the resolved numeric sheet ID (or a `"id||sheetId"` pair)
//!   for `sheetName` before calling the operation; we pass the resolved
//!   `sheet_id`/`spreadsheet_id` directly instead of that string-encoding.
//! - `read` always synthesizes a virtual `row_number` column the way
//!   `addRowNumber`/`prepareSheetData` do.
//! - A single try/catch wraps the whole node (matching n8n's `router()`):
//!   on error with `continueOnFail`, one error item is emitted (not one per
//!   input item), and any partial API writes already issued are not
//!   undone -- same as n8n.
//!
//! Known simplifications vs real n8n (see the implementation report):
//! resourceMapper `schema` change-detection is not enforced, `row_number`
//! is not treated as a special matching column in `update`, service-account
//! tokens are fetched once per node execution rather than once per HTTP
//! call, and the `documentId`/`sheetName` "by URL" extraction/validation
//! regexes are approximated.

use super::check_ssrf;
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Map, Value};

pub struct GoogleSheets;

const ROW_NUMBER: &str = "row_number";
const SHEETS_SCOPES: &str = "https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/spreadsheets https://www.googleapis.com/auth/drive.metadata";

// ---- small value / A1 helpers ----------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn locator_mode(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("mode").and_then(Value::as_str).unwrap_or("id").to_string(),
        _ => "id".to_string(),
    }
}

fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// n8n's `encodeURIComponent`.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// n8n's `GoogleSheet.encodeRange`.
fn encode_range(range: &str) -> String {
    match range.split_once('!') {
        Some((sheet, rest)) => format!("{}!{}", encode_uri_component(sheet), rest),
        None => encode_uri_component(&format!("'{range}'")),
    }
}

/// n8n's `getColumnName`: 1-based column number -> "A".."Z", "AA"...
fn column_name(mut n: i64) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        let rem = (n % 26) as u8;
        s.insert(0, (b'A' + rem) as char);
        n /= 26;
    }
    s
}

/// n8n's `getColumnNumber`: "A".."Z", "AA"... -> 1-based column number.
fn column_number(col: &str) -> i64 {
    col.chars().fold(0i64, |acc, c| acc * 26 + (c.to_ascii_uppercase() as i64 - 'A' as i64 + 1))
}

fn get_column_with_offset(start_column: &str, offset: usize) -> String {
    column_name(column_number(start_column) + offset as i64)
}

/// n8n's `getSpreadsheetId`.
fn get_spreadsheet_id(mode: &str, value: &str) -> NodeResult<String> {
    if value.is_empty() {
        return Err(NodeError::new("Can not get sheet 'Spreadsheet' with an empty value"));
    }
    if mode == "url" {
        return Ok(extract_id_from_url(value).unwrap_or_default());
    }
    Ok(value.to_string())
}

/// The first run of 25+ `[-\w]` characters in `value` (n8n's URL-ID regex).
fn extract_id_from_url(value: &str) -> Option<String> {
    let is_word = |c: char| c == '-' || c == '_' || c.is_ascii_alphanumeric();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if is_word(chars[i]) {
            let start = i;
            while i < chars.len() && is_word(chars[i]) {
                i += 1;
            }
            if i - start >= 25 {
                return Some(chars[start..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// n8n's `getSheetId`.
fn get_sheet_id_value(value: &str) -> i64 {
    if value == "gid=0" {
        return 0;
    }
    value.trim_start_matches("gid=").parse().unwrap_or(0)
}

fn hex_to_rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim_start_matches('#');
    let h = if h.len() == 3 { h.chars().flat_map(|c| [c, c]).collect::<String>() } else { h.to_string() };
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
}

fn unsupported(resource: &str, operation: &str) -> NodeError {
    NodeError::new(format!("Google Sheets \"{resource}\" / \"{operation}\" is not supported natively yet"))
}

// ---- authentication & the underlying HTTP call -----------------------------

enum AuthKind {
    ServiceAccount,
    OAuth2 { cred_id: String, cred_data: Value },
}

struct Auth {
    bearer: String,
    sheets_base: String,
    drive_base: String,
    kind: AuthKind,
}

fn drive_base_from(sheets_base: &str) -> String {
    if sheets_base.contains("sheets.googleapis.com") {
        "https://www.googleapis.com".to_string()
    } else {
        sheets_base.to_string()
    }
}

/// n8n's `formatPemBlock`, plus the `\n`-literal unescape n8n's
/// `getGoogleAccessToken`/`GoogleApi.credentials` both apply first.
fn format_pem_block(key: &str) -> String {
    let unescaped = key.replace("\\n", "\n");
    let trimmed = unescaped.trim();
    if trimmed.contains("-----BEGIN") {
        return trimmed.to_string();
    }
    let body: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    let mut lines = vec!["-----BEGIN PRIVATE KEY-----".to_string()];
    for chunk in body.as_bytes().chunks(64) {
        lines.push(String::from_utf8_lossy(chunk).into_owned());
    }
    lines.push("-----END PRIVATE KEY-----".to_string());
    lines.join("\n")
}

async fn service_account_token(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let (_, cred) = ctx.credentials("googleApi").await?;
    let email = cred["email"].as_str().unwrap_or("").trim().to_string();
    let private_key_raw = cred["privateKey"].as_str().unwrap_or("").to_string();
    if email.is_empty() || private_key_raw.is_empty() {
        return Err(NodeError::new("Google Service Account credentials are not set")
            .describe("Add a Service Account Email and Private Key to the Google Service Account API credential."));
    }
    let delegated = cred["delegatedEmail"].as_str().filter(|s| !s.is_empty()).unwrap_or(&email).to_string();
    let token_url = cred["tokenUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://oauth2.googleapis.com/token").to_string();
    let sheets_base = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://sheets.googleapis.com").trim_end_matches('/').to_string();

    let pem = format_pem_block(&private_key_raw);
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).map_err(|e| NodeError::new(format!("The private key could not be parsed: {e}")))?;
    let now = chrono::Utc::now().timestamp();
    let claims = json!({"iss": email, "sub": delegated, "scope": SHEETS_SCOPES, "aud": token_url, "iat": now, "exp": now + 3600});
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    let assertion = jsonwebtoken::encode(&header, &claims, &key).map_err(|e| NodeError::new(format!("Could not sign the service-account JWT: {e}")))?;

    let url = reqwest::Url::parse(&token_url).map_err(|_| NodeError::new(format!("Invalid token URL: {token_url}")))?;
    check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
    let form = [("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())];
    let resp = ctx.services.http.post(url).form(&form).send().await.map_err(|e| NodeError::new(format!("Could not get a Google OAuth2 access token: {e}")))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let token = body["access_token"]
        .as_str()
        .filter(|_| status < 400)
        .ok_or_else(|| NodeError::api("Could not get a Google OAuth2 access token", Some(status), Some(body.to_string())))?
        .to_string();
    Ok(Auth { bearer: token, drive_base: drive_base_from(&sheets_base), sheets_base, kind: AuthKind::ServiceAccount })
}

async fn oauth2_token(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let (cred_id, cred) = ctx.credentials("googleSheetsOAuth2Api").await?;
    let token = cred
        .pointer("/oauthTokenData/access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| NodeError::new("The Google Sheets OAuth2 credential is not connected").describe("Complete the OAuth2 authorization for this credential before using it."))?
        .to_string();
    let sheets_base = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://sheets.googleapis.com").trim_end_matches('/').to_string();
    Ok(Auth { bearer: token, drive_base: drive_base_from(&sheets_base), sheets_base, kind: AuthKind::OAuth2 { cred_id, cred_data: cred } })
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let method = ctx.param_str("authentication", 0, "serviceAccount")?;
    if method == "serviceAccount" {
        service_account_token(ctx).await
    } else {
        oauth2_token(ctx).await
    }
}

async fn refresh_oauth2(ctx: &ExecCtx<'_>, auth: &mut Auth) -> NodeResult<()> {
    let AuthKind::OAuth2 { cred_id, cred_data } = &mut auth.kind else { return Ok(()) };
    let refresh_token = cred_data
        .pointer("/oauthTokenData/refresh_token")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| NodeError::new("The Google Sheets OAuth2 credential has no refresh token"))?;
    let token_url = cred_data["accessTokenUrl"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://oauth2.googleapis.com/token").to_string();
    let client_id = cred_data["clientId"].as_str().unwrap_or("").to_string();
    let client_secret = cred_data["clientSecret"].as_str().unwrap_or("").to_string();
    let url = reqwest::Url::parse(&token_url).map_err(|_| NodeError::new(format!("Invalid access token URL: {token_url}")))?;
    check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh_token.as_str()), ("client_id", client_id.as_str()), ("client_secret", client_secret.as_str())];
    let resp = ctx.services.http.post(url).form(&form).send().await.map_err(|e| NodeError::new(format!("Could not refresh the Google OAuth2 access token: {e}")))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let token = body["access_token"]
        .as_str()
        .filter(|_| status < 400)
        .ok_or_else(|| NodeError::api("Could not refresh the Google OAuth2 access token", Some(status), Some(body.to_string())))?
        .to_string();
    auth.bearer = token;
    cred_data["oauthTokenData"] = body;
    if let Some(store) = &ctx.services.store {
        let _ = store.update_credential_data(cred_id, cred_data).await;
    }
    Ok(())
}

/// Maps a Google API `{error: {message, ...}}` error payload the way n8n's
/// `NodeApiError` construction (plus the sheets-specific PERMISSION_DENIED
/// hint in `transport/index.js`) does.
fn google_error(status: u16, body: &Value) -> NodeError {
    let message = body.pointer("/error/message").and_then(Value::as_str).map(String::from);
    let msg = message.unwrap_or_else(|| format!("Google Sheets API request failed with status code {status}"));
    let mut err = NodeError::api(msg, Some(status), Some(body.to_string()));
    if status == 403 {
        err = err.describe("Please check that the account you're using has the right permissions. (If you're trying to modify the sheet, you'll need edit access.)");
    }
    err
}

async fn api_request(ctx: &ExecCtx<'_>, auth: &mut Auth, method: &str, url_str: &str, body: Option<Value>, query: &[(String, String)]) -> NodeResult<Value> {
    let mut refreshed = false;
    loop {
        let mut url = reqwest::Url::parse(url_str).map_err(|_| NodeError::new(format!("Invalid Google Sheets API URL: {url_str}")))?;
        check_ssrf(&url, ctx.config()).await.map_err(NodeError::new)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
        }
        let m = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")))?;
        let mut req = ctx.services.http.request(m, url).bearer_auth(&auth.bearer);
        if let Some(b) = &body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Google Sheets failed: {e}"), None, None))?;
        let status = resp.status().as_u16();
        if status == 401 && !refreshed && matches!(auth.kind, AuthKind::OAuth2 { .. }) {
            refreshed = true;
            refresh_oauth2(ctx, auth).await?;
            continue;
        }
        let text = resp.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status >= 400 {
            return Err(google_error(status, &value));
        }
        return Ok(value);
    }
}

// ---- Sheets/Drive API calls (n8n's `helpers/GoogleSheet.js`) ---------------

async fn get_data(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, range: &str, value_render_mode: &str, date_time_render_option: Option<&str>) -> NodeResult<Vec<Vec<Value>>> {
    let url = format!("{}/v4/spreadsheets/{}/values/{}", auth.sheets_base, ssid, encode_range(range));
    let query = vec![
        ("valueRenderOption".to_string(), value_render_mode.to_string()),
        ("dateTimeRenderOption".to_string(), date_time_render_option.unwrap_or("FORMATTED_STRING").to_string()),
    ];
    let resp = api_request(ctx, auth, "GET", &url, None, &query).await?;
    let values = resp.get("values").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(values.into_iter().map(|row| row.as_array().cloned().unwrap_or_default()).collect())
}

async fn clear_data(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, range: &str) -> NodeResult<Value> {
    let url = format!("{}/v4/spreadsheets/{}/values/{}:clear", auth.sheets_base, ssid, encode_range(range));
    api_request(ctx, auth, "POST", &url, Some(json!({"spreadsheetId": ssid, "range": range})), &[]).await
}

async fn spreadsheet_get_sheets(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str) -> NodeResult<Vec<Value>> {
    let url = format!("{}/v4/spreadsheets/{}", auth.sheets_base, ssid);
    let resp = api_request(ctx, auth, "GET", &url, None, &[("fields".to_string(), "sheets.properties".to_string())]).await?;
    Ok(resp.get("sheets").and_then(Value::as_array).cloned().unwrap_or_default())
}

async fn spreadsheet_get_sheet(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, mode: &str, value: &str) -> NodeResult<(i64, String)> {
    let sheets = spreadsheet_get_sheets(ctx, auth, ssid).await?;
    let found = sheets.iter().find(|item| {
        let props = &item["properties"];
        if mode == "name" {
            props["title"].as_str() == Some(value)
        } else {
            props["sheetId"].as_i64() == Some(get_sheet_id_value(value))
        }
    });
    match found {
        Some(item) => Ok((item["properties"]["sheetId"].as_i64().unwrap_or(0), item["properties"]["title"].as_str().unwrap_or("").to_string())),
        None => Err(NodeError::new(format!("Sheet with {} {} not found", if mode == "name" { "name" } else { "ID" }, value))),
    }
}

async fn spreadsheet_batch_update(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, requests: Value) -> NodeResult<Value> {
    let url = format!("{}/v4/spreadsheets/{}:batchUpdate", auth.sheets_base, ssid);
    api_request(ctx, auth, "POST", &url, Some(json!({"requests": requests})), &[]).await
}

async fn batch_update_values(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, data: Vec<Value>, value_input_mode: &str) -> NodeResult<Value> {
    let url = format!("{}/v4/spreadsheets/{}/values:batchUpdate", auth.sheets_base, ssid);
    api_request(ctx, auth, "POST", &url, Some(json!({"data": data, "valueInputOption": value_input_mode})), &[]).await
}

#[allow(clippy::too_many_arguments)]
async fn update_rows(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_name: &str, data: Vec<Vec<Value>>, value_input_mode: &str, row: i64, rows_length: Option<i64>, use_append: bool) -> NodeResult<Value> {
    let name = sheet_name.split('!').next().unwrap_or(sheet_name);
    let end = match rows_length {
        Some(n) if n > 1 => row + n - 1,
        _ => row,
    };
    let range = format!("{name}!{row}:{end}");
    let url_base = format!("{}/v4/spreadsheets/{}/values/{}", auth.sheets_base, ssid, encode_range(&range));
    let body = json!({"range": range, "values": data});
    let query = [("valueInputOption".to_string(), value_input_mode.to_string())];
    if use_append {
        api_request(ctx, auth, "POST", &format!("{url_base}:append"), Some(body), &query).await
    } else {
        api_request(ctx, auth, "PUT", &url_base, Some(body), &query).await
    }
}

async fn append_empty_rows_or_columns(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_id: i64, rows_to_add: i64, columns_to_add: i64) -> NodeResult<Value> {
    let mut requests = Vec::new();
    if rows_to_add > 0 {
        requests.push(json!({"appendDimension": {"sheetId": sheet_id, "dimension": "ROWS", "length": rows_to_add}}));
    }
    if columns_to_add > 0 {
        requests.push(json!({"appendDimension": {"sheetId": sheet_id, "dimension": "COLUMNS", "length": columns_to_add}}));
    }
    spreadsheet_batch_update(ctx, auth, ssid, Value::Array(requests)).await
}

// ---- row <-> object conversion (n8n's `GoogleSheets.utils`/`GoogleSheet`) --

fn cell_has_value(v: &Value) -> bool {
    match v {
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        _ => true,
    }
}

fn add_row_number(data: &[Vec<Value>], header_row: usize) -> Vec<Vec<Value>> {
    if data.is_empty() {
        return vec![];
    }
    let mut out: Vec<Vec<Value>> = data
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = vec![json!(i as i64 + 1)];
            r.extend(row.clone());
            r
        })
        .collect();
    if header_row < out.len() {
        out[header_row][0] = json!(ROW_NUMBER);
    }
    out
}

fn remove_empty_rows(data: Vec<Vec<Value>>, includes_row_number: bool) -> Vec<Vec<Value>> {
    let base = usize::from(includes_row_number);
    let mut out: Vec<Vec<Value>> = data.into_iter().filter(|row| row.iter().skip(base).any(cell_has_value)).collect();
    if includes_row_number && !out.is_empty() {
        out[0][0] = json!(ROW_NUMBER);
    }
    out
}

fn trim_to_first_empty_row(data: Vec<Vec<Value>>, includes_row_number: bool) -> Vec<Vec<Value>> {
    let base = usize::from(includes_row_number);
    match data.iter().position(|row| row.iter().skip(base).all(|c| !cell_has_value(c))) {
        Some(idx) => data[..idx].to_vec(),
        None => data,
    }
}

fn trim_leading_empty_rows(data: Vec<Vec<Value>>, includes_row_number: bool) -> Vec<Vec<Value>> {
    let base = usize::from(includes_row_number);
    let start = data.iter().position(|row| row.iter().skip(base).any(cell_has_value)).unwrap_or(data.len());
    let mut out = data[start..].to_vec();
    if includes_row_number && !out.is_empty() {
        out[0][0] = json!(ROW_NUMBER);
    }
    out
}

fn remove_empty_columns(data: &[Vec<Value>]) -> Vec<Vec<Value>> {
    if data.is_empty() {
        return vec![];
    }
    let longest = data.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut cols: Vec<Vec<Value>> = Vec::new();
    for col in 0..longest {
        let column: Vec<Value> = data.iter().map(|r| r.get(col).cloned().unwrap_or(json!(""))).collect();
        let header_is_empty = matches!(&column[0], Value::String(s) if s.is_empty());
        if !header_is_empty {
            cols.push(column);
            continue;
        }
        if column[1..].iter().any(cell_has_value) {
            cols.push(column);
        }
    }
    if cols.is_empty() {
        return vec![];
    }
    let nrows = cols[0].len();
    (0..nrows).map(|r| cols.iter().map(|c| c[r].clone()).collect()).collect()
}

/// n8n's `prepareSheetData`. Returns `(data, header_row_index, first_data_row_index)`.
fn prepare_sheet_data(data: Vec<Vec<Value>>, range_definition: &str, header_row_opt: Option<i64>, first_data_row_opt: Option<i64>, read_rows_until: &str) -> (Vec<Vec<Value>>, usize, usize) {
    let mut header_row = 0usize;
    let mut first_data_row = 1usize;
    if range_definition == "specifyRange" {
        header_row = (header_row_opt.unwrap_or(1).max(1) - 1) as usize;
        first_data_row = (first_data_row_opt.unwrap_or(2).max(1) - 1) as usize;
    }
    let mut out = add_row_number(&data, header_row);
    if range_definition == "detectAutomatically" {
        out = remove_empty_columns(&out);
        out = trim_leading_empty_rows(out, true);
        if read_rows_until == "firstEmptyRow" {
            out = trim_to_first_empty_row(out, true);
        } else {
            out = remove_empty_rows(out, true);
        }
    }
    (out, header_row, first_data_row)
}

fn get_range_string(sheet_name: &str, range_definition: &str, range: Option<&str>) -> String {
    if range_definition == "specifyRangeA1" {
        match range {
            Some(r) if !r.is_empty() => format!("{sheet_name}!{r}"),
            _ => sheet_name.to_string(),
        }
    } else {
        sheet_name.to_string()
    }
}

fn convert_sheet_data_array_to_object_array(sheet: &[Vec<Value>], start_row: usize, column_keys: &[String], add_empty: bool, include_headers_with_empty_cells: bool) -> Vec<Map<String, Value>> {
    let mut out = Vec::new();
    for row_index in start_row..sheet.len() {
        let mut item = Map::new();
        let row = &sheet[row_index];
        let column_count = if include_headers_with_empty_cells { column_keys.len() } else { row.len() };
        for (c, key) in column_keys.iter().enumerate().take(column_count) {
            if !key.is_empty() {
                item.insert(key.clone(), row.get(c).cloned().unwrap_or(json!("")));
            }
        }
        if !item.is_empty() || add_empty {
            out.push(item);
        }
    }
    out
}

fn structure_array_data_by_column(input: &[Vec<Value>], key_row: usize, data_start_row: usize, include_headers_with_empty_cells: bool) -> Vec<Map<String, Value>> {
    if key_row >= input.len() || data_start_row < key_row {
        return vec![];
    }
    let longest = input.iter().map(|r| r.len()).max().unwrap_or(0);
    let keys: Vec<String> = (0..longest).map(|c| input[key_row].get(c).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| format!("col_{c}"))).collect();
    convert_sheet_data_array_to_object_array(input, data_start_row, &keys, false, include_headers_with_empty_cells)
}

/// n8n's `GoogleSheet.lookupValues`.
fn lookup_values(input_data: &mut [Vec<Value>], key_row_index: usize, data_start_row_index: usize, lookup: &[(String, String)], return_all_matches: bool, combine_filters: &str, node_version: f64) -> NodeResult<Vec<Map<String, Value>>> {
    if key_row_index >= input_data.len() || data_start_row_index < key_row_index {
        return Err(NodeError::new("The key row does not exist"));
    }
    let longest = input_data[key_row_index].len();
    let keys: Vec<String> = (0..longest).map(|c| input_data[key_row_index].get(c).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| format!("col_{c}"))).collect();
    for row in input_data.iter_mut() {
        while row.len() < keys.len() {
            row.push(json!(""));
        }
    }
    let mut return_data: Vec<Vec<Value>> = vec![keys.iter().map(|k| json!(k)).collect()];
    let mut added_rows: Vec<usize> = Vec::new();
    if combine_filters == "OR" {
        'outer: for (col, val) in lookup {
            let Some(col_idx) = keys.iter().position(|k| k == col) else { return Err(NodeError::new(format!("The column \"{col}\" could not be found"))) };
            for r in data_start_row_index..input_data.len() {
                if value_to_string(&input_data[r][col_idx]) == *val {
                    if !added_rows.contains(&r) {
                        return_data.push(input_data[r].clone());
                        added_rows.push(r);
                    }
                    if !return_all_matches {
                        if node_version >= 4.6 {
                            break 'outer;
                        }
                        continue 'outer;
                    }
                }
            }
        }
    } else {
        'rows: for r in data_start_row_index..input_data.len() {
            let mut all_match = true;
            for (col, val) in lookup {
                let Some(col_idx) = keys.iter().position(|k| k == col) else { return Err(NodeError::new(format!("The column \"{col}\" could not be found"))) };
                if value_to_string(&input_data[r][col_idx]) != *val {
                    all_match = false;
                    break;
                }
            }
            if all_match {
                if !added_rows.contains(&r) {
                    return_data.push(input_data[r].clone());
                    added_rows.push(r);
                }
                if !return_all_matches {
                    break 'rows;
                }
            }
        }
    }
    let cleaned = remove_empty_columns(&return_data);
    if cleaned.is_empty() {
        return Ok(vec![]);
    }
    let header_keys: Vec<String> = cleaned[0].iter().map(value_to_string).collect();
    Ok(convert_sheet_data_array_to_object_array(&cleaned, 1, &header_keys, true, false))
}

fn get_column_values_from_sheet(sheet_data: &[Vec<Value>], key_index: usize, data_start_row_index: usize) -> Vec<Value> {
    sheet_data.get(data_start_row_index..).unwrap_or(&[]).iter().map(|row| row.get(key_index).cloned().unwrap_or(json!(""))).collect()
}

fn convert_object_array_to_sheet_data(input: &[Map<String, Value>], column_names: &[String]) -> Vec<Vec<Value>> {
    input
        .iter()
        .map(|item| {
            column_names
                .iter()
                .map(|k| match item.get(k) {
                    None | Some(Value::Null) => json!(""),
                    Some(v @ (Value::Object(_) | Value::Array(_))) => json!(v.to_string()),
                    Some(v) => v.clone(),
                })
                .collect()
        })
        .collect()
}

// ---- columns resourceMapper helpers ----------------------------------------

fn mapping_mode(columns: &Value) -> String {
    columns.get("mappingMode").and_then(Value::as_str).unwrap_or("defineBelow").to_string()
}

fn mapping_value(columns: &Value) -> Map<String, Value> {
    columns.get("value").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn matching_columns(columns: &Value) -> Vec<String> {
    columns.get("matchingColumns").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// n8n's `autoMapInputData`'s per-item mapping (without the pre-fetch /
/// header-write side effects, which callers handle themselves).
fn auto_map_items(items: &[Item], column_names: &mut Vec<String>, handling: &str, item_offset: usize) -> NodeResult<Vec<Map<String, Value>>> {
    let mut new_columns: Vec<String> = Vec::new();
    if handling == "insertInNewColumn" {
        for item in items {
            for k in item.json.keys() {
                if k != ROW_NUMBER && !column_names.contains(k) && !new_columns.contains(k) {
                    new_columns.push(k.clone());
                }
            }
        }
    }
    if handling == "error" {
        for (idx, item) in items.iter().enumerate() {
            for k in item.json.keys() {
                if !column_names.contains(k) {
                    return Err(NodeError::new("Unexpected fields in node input")
                        .describe(format!("The input field '{k}' doesn't match any column in the Sheet. You can ignore this by changing the 'Handling extra data' field, which you can find under 'Options'."))
                        .at(item_offset + idx));
                }
            }
        }
    }
    let out = items
        .iter()
        .map(|item| {
            let mut obj = item.json.clone();
            obj.remove(ROW_NUMBER);
            obj
        })
        .collect();
    column_names.extend(new_columns);
    Ok(out)
}

// ---- per-operation execution ------------------------------------------------

async fn op_append(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_title: &str, sheet_id: i64, items: &[Item]) -> NodeResult<Vec<Item>> {
    if items.is_empty() {
        return Ok(vec![]);
    }
    let columns0 = ctx.param("columns", 0)?;
    let mut mode = mapping_mode(&columns0);
    if mode == "nothing" {
        return Ok(vec![]);
    }
    let options = ctx.param("options", 0)?;
    let key_row_index = options.pointer("/locationDefine/values/headerRow").and_then(Value::as_f64).unwrap_or(1.0) as i64;
    let sheet_data = get_data(ctx, auth, ssid, sheet_title, "FORMATTED_VALUE", None).await?;
    if sheet_data.is_empty() {
        mode = "autoMapInputData".to_string();
    }
    let mut column_names: Vec<String> = sheet_data.get((key_row_index - 1).max(0) as usize).map(|r| r.iter().map(value_to_string).collect()).unwrap_or_default();
    let cell_format = options.get("cellFormat").and_then(Value::as_str).unwrap_or("USER_ENTERED").to_string();

    let input_data: Vec<Map<String, Value>> = if mode == "autoMapInputData" {
        let handling = options.get("handlingExtraData").and_then(Value::as_str).unwrap_or("insertInNewColumn").to_string();
        if column_names.is_empty() {
            let first_keys: Vec<String> = items[0].json.keys().filter(|k| k.as_str() != ROW_NUMBER).cloned().collect();
            update_rows(ctx, auth, ssid, sheet_title, vec![first_keys.iter().map(|k| json!(k)).collect()], &cell_format, key_row_index, Some(1), false).await?;
            column_names = first_keys;
        }
        let before = column_names.len();
        let mapped = auto_map_items(items, &mut column_names, &handling, 0)?;
        if column_names.len() > before {
            update_rows(ctx, auth, ssid, sheet_title, vec![column_names.iter().map(|k| json!(k)).collect()], &cell_format, key_row_index, Some(1), false).await?;
        }
        mapped
    } else {
        let mut vals = Vec::new();
        for i in 0..items.len() {
            let cols_i = ctx.param("columns", i)?;
            let v = mapping_value(&cols_i);
            if v.is_empty() {
                return Err(NodeError::new("At least one value has to be added under 'Values to Send'").at(i));
            }
            vals.push(v);
        }
        vals
    };
    if input_data.is_empty() {
        return Ok(vec![]);
    }
    let use_append = options.get("useAppend").and_then(Value::as_bool).unwrap_or(false);
    let rows = convert_object_array_to_sheet_data(&input_data, &column_names);
    // n8n's `appendData`: `(sheetData ?? [{}]).length + 1` -- a truly empty
    // sheet (Google omits `values` entirely) behaves like a one-element
    // dummy array, landing data on row 2 (below the header just written),
    // not row 1.
    let last_row = if sheet_data.is_empty() { 2 } else { sheet_data.len() as i64 + 1 };
    if use_append {
        update_rows(ctx, auth, ssid, sheet_title, rows.clone(), &cell_format, last_row, Some(rows.len() as i64), true).await?;
    } else {
        append_empty_rows_or_columns(ctx, auth, ssid, sheet_id, 1, 0).await?;
        update_rows(ctx, auth, ssid, sheet_title, rows.clone(), &cell_format, last_row, Some(rows.len() as i64), false).await?;
    }
    Ok(input_data.into_iter().enumerate().map(|(i, obj)| Item::new(obj).paired(i)).collect())
}

/// Shared by `appendOrUpdate` (`upsert = true`) and `update` (`upsert = false`).
async fn op_upsert(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_title: &str, sheet_id: i64, items: &[Item], upsert: bool) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", 0)?;
    let cell_format = options.get("cellFormat").and_then(Value::as_str).unwrap_or("USER_ENTERED").to_string();
    let key_row_index = options.pointer("/locationDefine/values/headerRow").and_then(Value::as_f64).map(|v| (v as i64 - 1).max(0) as usize).unwrap_or(0);
    let data_start_row_index = options.pointer("/locationDefine/values/firstDataRow").and_then(Value::as_f64).map(|v| (v as i64 - 1).max(0) as usize).unwrap_or(1);

    let sheet_data = get_data(ctx, auth, ssid, sheet_title, "FORMATTED_VALUE", None).await?;
    let columns0 = ctx.param("columns", 0)?;
    let mut mode = mapping_mode(&columns0);
    if sheet_data.get(key_row_index).is_none() {
        if sheet_data.is_empty() {
            mode = "autoMapInputData".to_string();
        } else {
            return Err(NodeError::new(format!("Could not retrieve the column names from row {}", key_row_index + 1)));
        }
    }
    let mut column_names: Vec<String> = sheet_data.get(key_row_index).map(|r| r.iter().map(value_to_string).collect()).unwrap_or_default();

    let matching = matching_columns(&columns0);
    if mode != "autoMapInputData" && matching.is_empty() {
        let op_name = if upsert { "Append or Update Row" } else { "Update Row" };
        return Err(NodeError::new(format!("`columns.matchingColumns` is required for the {op_name} operation"))
            .describe("Set `columns.matchingColumns` to a non-empty `string[]` of header names that uniquely identify the row to update."));
    }
    let index_key = matching.first().cloned().unwrap_or_else(|| ROW_NUMBER.to_string());
    if mode != "autoMapInputData" && !upsert && !column_names.contains(&index_key) {
        return Err(NodeError::new(format!("Could not find column for key \"{index_key}\"")));
    }
    let key_index = column_names.iter().position(|c| c == &index_key).unwrap_or(0);
    let column_values = get_column_values_from_sheet(&sheet_data, key_index, data_start_row_index);

    let mut update_data: Vec<Value> = Vec::new();
    let mut append_data: Vec<Map<String, Value>> = Vec::new();
    let mut mapped_values: Vec<Map<String, Value>> = Vec::new();
    let handling = options.get("handlingExtraData").and_then(Value::as_str).unwrap_or("insertInNewColumn").to_string();

    for i in 0..items.len() {
        if mode == "nothing" {
            continue;
        }
        let input_obj: Map<String, Value> = if mode == "autoMapInputData" {
            let single = [items[i].clone()];
            let mapped = auto_map_items(&single, &mut column_names, &handling, i)?;
            mapped.into_iter().next().unwrap_or_default()
        } else {
            let cols_i = ctx.param("columns", i)?;
            let mut v = mapping_value(&cols_i);
            if v.is_empty() {
                return Err(NodeError::new("At least one value has to be added under 'Values to Send'").at(i));
            }
            for val in v.values_mut() {
                if val.is_null() {
                    *val = json!("");
                }
            }
            mapped_values.push(v.clone());
            v
        };

        match input_obj.get(&index_key).filter(|v| !v.is_null()) {
            None => {
                if upsert {
                    append_data.push(input_obj);
                }
            }
            Some(key_val) => {
                let key_str = value_to_string(key_val);
                if let Some(pos) = column_values.iter().position(|cv| value_to_string(cv) == key_str) {
                    let update_row_index = pos + data_start_row_index + 1;
                    for name in column_names.iter() {
                        if name == &index_key {
                            continue;
                        }
                        if let Some(v) = input_obj.get(name) {
                            if v.is_null() {
                                continue;
                            }
                            let col_pos = column_names.iter().position(|c| c == name).unwrap_or(0);
                            let col_letter = get_column_with_offset("A", col_pos);
                            let out_v = if v.is_object() || v.is_array() { json!(v.to_string()) } else { v.clone() };
                            update_data.push(json!({"range": format!("{sheet_title}!{col_letter}{update_row_index}"), "values": [[out_v]]}));
                        }
                    }
                } else if upsert {
                    append_data.push(input_obj);
                }
            }
        }
    }

    if !update_data.is_empty() {
        batch_update_values(ctx, auth, ssid, update_data, &cell_format).await?;
    }
    if !append_data.is_empty() {
        let rows = convert_object_array_to_sheet_data(&append_data, &column_names);
        let last_row = sheet_data.len() as i64 + 1;
        let use_append = options.get("useAppend").and_then(Value::as_bool).unwrap_or(false);
        if use_append {
            update_rows(ctx, auth, ssid, sheet_title, rows.clone(), &cell_format, last_row, Some(rows.len() as i64), true).await?;
        } else {
            append_empty_rows_or_columns(ctx, auth, ssid, sheet_id, 1, 0).await?;
            update_rows(ctx, auth, ssid, sheet_title, rows.clone(), &cell_format, last_row, Some(rows.len() as i64), false).await?;
        }
    }

    if mode == "autoMapInputData" {
        Ok(items.iter().enumerate().map(|(i, it)| it.clone().paired(i)).collect())
    } else {
        Ok(mapped_values.into_iter().enumerate().map(|(i, v)| Item::new(v).paired(i)).collect())
    }
}

async fn op_clear(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_title: &str, items: &[Item]) -> NodeResult<Vec<Item>> {
    for i in 0..items.len() {
        let clear_type = ctx.param_str("clear", i, "wholeSheet")?;
        let keep_first_row = ctx.param_bool("keepFirstRow", i, false)?;
        let range = match clear_type.as_str() {
            "specificRows" => {
                let start = ctx.param_f64("startIndex", i, 1.0)? as i64;
                let n = ctx.param_f64("rowsToDelete", i, 1.0)? as i64;
                let end = if n == 1 { start } else { start + n - 1 };
                format!("{sheet_title}!{start}:{end}")
            }
            "specificColumns" => {
                let start_col = ctx.param_str("startIndex", i, "A")?;
                let n = ctx.param_f64("columnsToDelete", i, 1.0)? as i64;
                let colnum = column_number(&start_col);
                let end = if n == 1 { colnum } else { colnum + n - 1 };
                format!("{sheet_title}!{start_col}:{}", column_name(end))
            }
            "specificRange" => {
                let r = ctx.param_str("range", i, "A:F")?;
                let region = if r.contains('!') { r.split('!').nth(1).unwrap_or("").to_string() } else { r };
                format!("{sheet_title}!{region}")
            }
            _ => sheet_title.to_string(),
        };
        if keep_first_row {
            let first_row = get_data(ctx, auth, ssid, &format!("{range}!1:1"), "FORMATTED_VALUE", None).await?;
            clear_data(ctx, auth, ssid, &range).await?;
            update_rows(ctx, auth, ssid, &range, first_row, "RAW", 1, Some(1), false).await?;
        } else {
            clear_data(ctx, auth, ssid, &range).await?;
        }
    }
    Ok(items.iter().enumerate().map(|(i, it)| it.clone().paired(i)).collect())
}

async fn op_create_sheet(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, items: &[Item]) -> NodeResult<Vec<Item>> {
    let mut existing: Vec<String> = spreadsheet_get_sheets(ctx, auth, ssid).await?.iter().map(|s| s["properties"]["title"].as_str().unwrap_or("").to_string()).collect();
    let mut out = Vec::new();
    for i in 0..items.len() {
        let title = ctx.param_str("title", i, "n8n-sheet")?;
        if existing.contains(&title) {
            continue;
        }
        let options = ctx.param("options", i)?;
        let mut properties = Map::new();
        properties.insert("title".into(), json!(title));
        for key in ["hidden", "rightToLeft", "sheetId", "index"] {
            if let Some(v) = options.get(key) {
                properties.insert(key.to_string(), v.clone());
            }
        }
        if let Some(tc) = options.get("tabColor").and_then(Value::as_str) {
            if let Some((r, g, b)) = hex_to_rgb(tc) {
                properties.insert("tabColor".into(), json!({"red": r, "green": g, "blue": b}));
            }
        }
        let resp = spreadsheet_batch_update(ctx, auth, ssid, json!([{"addSheet": {"properties": Value::Object(properties)}}])).await?;
        let props = resp.pointer("/replies/0/addSheet/properties").cloned().unwrap_or(json!({}));
        existing.push(title);
        out.push(Item::from_value(props).paired(i));
    }
    Ok(out)
}

async fn op_delete_rows_columns(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_id: i64, items: &[Item]) -> NodeResult<Vec<Item>> {
    for i in 0..items.len() {
        let to_delete = ctx.param_str("toDelete", i, "rows")?;
        let (dimension, start_index, end_index) = if to_delete == "rows" {
            let start = ctx.param_f64("startIndex", i, 2.0)? as i64 - 1;
            let n = ctx.param_f64("numberToDelete", i, 1.0)? as i64;
            let end = if n == 1 { start + 1 } else { start + n };
            ("ROWS", start, end)
        } else {
            let start_col = ctx.param_str("startIndex", i, "A")?;
            let n = ctx.param_f64("numberToDelete", i, 1.0)? as i64;
            let start = column_number(&start_col) - 1;
            let end = if n == 1 { start + 1 } else { start + n };
            ("COLUMNS", start, end)
        };
        spreadsheet_batch_update(ctx, auth, ssid, json!([{"deleteDimension": {"range": {"sheetId": sheet_id, "dimension": dimension, "startIndex": start_index, "endIndex": end_index}}}])).await?;
    }
    let mut m = Map::new();
    m.insert("success".into(), json!(true));
    Ok(vec![Item::new(m).paired(0)])
}

async fn op_remove_sheet(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_id: i64, items: &[Item]) -> NodeResult<Vec<Item>> {
    let mut out = Vec::new();
    for i in 0..items.len() {
        let resp = spreadsheet_batch_update(ctx, auth, ssid, json!([{"deleteSheet": {"sheetId": sheet_id}}])).await?;
        let mut obj = resp.as_object().cloned().unwrap_or_default();
        obj.remove("replies");
        out.push(Item::new(obj).paired(i));
    }
    Ok(out)
}

async fn op_read(ctx: &ExecCtx<'_>, auth: &mut Auth, ssid: &str, sheet_title: &str, items: &[Item], node_version: f64) -> NodeResult<Vec<Item>> {
    let mut out = Vec::new();
    for i in 0..items.len() {
        let options = ctx.param("options", i)?;
        let output_formatting = options.pointer("/outputFormatting/values").cloned().unwrap_or(json!({}));
        let general = output_formatting.get("general").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("UNFORMATTED_VALUE").to_string();
        let date = output_formatting.get("date").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("FORMATTED_STRING").to_string();
        let location = options.pointer("/dataLocationOnSheet/values").cloned().unwrap_or(json!({}));
        let range_definition = location.get("rangeDefinition").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("detectAutomatically").to_string();
        let read_rows_until = location.get("readRowsUntil").and_then(Value::as_str).unwrap_or("lastRowInSheet").to_string();
        let header_row = location.get("headerRow").and_then(Value::as_f64);
        let first_data_row = location.get("firstDataRow").and_then(Value::as_f64);
        let range_a1 = location.get("range").and_then(Value::as_str);
        let range_string = get_range_string(sheet_title, &range_definition, range_a1);

        let sheet_data = get_data(ctx, auth, ssid, &range_string, &general, Some(&date)).await?;
        if sheet_data.is_empty() {
            continue;
        }
        let (mut data, key_row_index, data_start_row_index) = prepare_sheet_data(sheet_data, &range_definition, header_row.map(|v| v as i64), first_data_row.map(|v| v as i64), &read_rows_until);

        let filters = ctx.param("filtersUI.values", i)?;
        let filters_vec: Vec<(String, String)> = filters
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let col = f.get("lookupColumn").and_then(Value::as_str)?.to_string();
                let val = f.get("lookupValue").map(value_to_string).unwrap_or_default();
                Some((col, val))
            })
            .collect();
        let results: Vec<Map<String, Value>> = if !filters_vec.is_empty() {
            let combine_filters = ctx.param_str("combineFilters", i, "AND")?;
            let return_first_match = ctx.param_bool("options.returnFirstMatch", i, false)?;
            lookup_values(&mut data, key_row_index, data_start_row_index, &filters_vec, !return_first_match, &combine_filters, node_version)?
        } else {
            structure_array_data_by_column(&data, key_row_index, data_start_row_index, false)
        };
        for r in results {
            out.push(Item::new(r).paired(i));
        }
    }
    Ok(out)
}

async fn op_spreadsheet_create(ctx: &ExecCtx<'_>, auth: &mut Auth, items: &[Item]) -> NodeResult<Vec<Item>> {
    let mut out = Vec::new();
    for i in 0..items.len() {
        let title = ctx.param_str("title", i, "")?;
        let sheets_ui = ctx.param("sheetsUi.sheetValues", i)?;
        let options = ctx.param("options", i)?;
        let mut properties = Map::new();
        properties.insert("title".into(), json!(title));
        if let Some(l) = options.get("locale").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            properties.insert("locale".into(), json!(l));
        }
        if let Some(a) = options.get("autoRecalc").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            properties.insert("autoRecalc".into(), json!(a));
        }
        let sheets: Vec<Value> = sheets_ui.as_array().cloned().unwrap_or_default().into_iter().map(|s| json!({"properties": s})).collect();
        let body = json!({"properties": Value::Object(properties), "sheets": sheets});
        let url = format!("{}/v4/spreadsheets", auth.sheets_base);
        let resp = api_request(ctx, auth, "POST", &url, Some(body), &[]).await?;
        out.push(Item::from_value(resp).paired(i));
    }
    Ok(out)
}

async fn op_spreadsheet_delete(ctx: &ExecCtx<'_>, auth: &mut Auth, items: &[Item]) -> NodeResult<Vec<Item>> {
    let mut out = Vec::new();
    for i in 0..items.len() {
        let doc = ctx.param("documentId", i)?;
        let mode = locator_mode(&doc);
        let value = locator_str(&doc).unwrap_or_default();
        let id = get_spreadsheet_id(&mode, &value)?;
        let url = format!("{}/drive/v3/files/{}", auth.drive_base, id);
        api_request(ctx, auth, "DELETE", &url, None, &[]).await?;
        let mut m = Map::new();
        m.insert("success".into(), json!(true));
        out.push(Item::new(m).paired(i));
    }
    Ok(out)
}

// ---- top-level dispatch ------------------------------------------------------

impl GoogleSheets {
    async fn run(&self, ctx: &ExecCtx<'_>, auth: &mut Auth, resource: &str, operation: &str, items: &[Item]) -> NodeResult<Vec<Item>> {
        match resource {
            "sheet" => {
                let doc = ctx.param("documentId", 0)?;
                let doc_mode = locator_mode(&doc);
                let doc_value = locator_str(&doc).unwrap_or_default();
                let ssid = get_spreadsheet_id(&doc_mode, &doc_value)?;
                let (sheet_id, sheet_title) = if operation != "create" {
                    let sn = ctx.param("sheetName", 0)?;
                    let sn_mode = locator_mode(&sn);
                    let sn_value = locator_str(&sn).unwrap_or_default();
                    if sn_value.is_empty() {
                        return Err(NodeError::new("Sheet must be selected"));
                    }
                    spreadsheet_get_sheet(ctx, auth, &ssid, &sn_mode, &sn_value).await?
                } else {
                    (0i64, String::new())
                };
                match operation {
                    "append" => op_append(ctx, auth, &ssid, &sheet_title, sheet_id, items).await,
                    "appendOrUpdate" => op_upsert(ctx, auth, &ssid, &sheet_title, sheet_id, items, true).await,
                    "update" => op_upsert(ctx, auth, &ssid, &sheet_title, sheet_id, items, false).await,
                    "clear" => op_clear(ctx, auth, &ssid, &sheet_title, items).await,
                    "create" => op_create_sheet(ctx, auth, &ssid, items).await,
                    "delete" => op_delete_rows_columns(ctx, auth, &ssid, sheet_id, items).await,
                    "remove" => op_remove_sheet(ctx, auth, &ssid, sheet_id, items).await,
                    "read" => op_read(ctx, auth, &ssid, &sheet_title, items, ctx.node.type_version).await,
                    other => Err(unsupported("sheet", other)),
                }
            }
            "spreadsheet" => match operation {
                "create" => op_spreadsheet_create(ctx, auth, items).await,
                "deleteSpreadsheet" => op_spreadsheet_delete(ctx, auth, items).await,
                other => Err(unsupported("spreadsheet", other)),
            },
            other => Err(NodeError::new(format!("Google Sheets resource \"{other}\" is not supported natively yet"))),
        }
    }
}

#[async_trait::async_trait]
impl NodeType for GoogleSheets {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.googleSheets"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let items = ctx.input().to_vec();
        let resource = ctx.param_str("resource", 0, "sheet")?;
        let operation = ctx.param_str("operation", 0, "read")?;
        let mut auth = resolve_auth(ctx).await?;
        match self.run(ctx, &mut auth, &resource, &operation, &items).await {
            Ok(out) => Ok(vec![out]),
            Err(e) if ctx.continue_on_fail() => {
                ctx.push_error_item(&e, 0);
                Ok(vec![vec![]])
            }
            Err(e) => Err(e),
        }
    }
}
