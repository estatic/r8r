//! Notion node (spec §6.6), v2.x (2.0/2.2), faithful to n8n's
//! `nodes/Notion/v2/NotionV2.node.js` + `nodes/Notion/shared/GenericFunctions.js`.
//!
//! Implements:
//!
//! - `block`: append, getAll (incl. `fetchNestedBlocks`)
//! - `database`: get, getAll, search
//! - `databasePage`: create, get, getAll (filters incl. manual/JSON, sorts,
//!   `simple` output), update
//! - `page`: archive, create, get, search
//! - `user`: get, getAll
//!
//! Anything else returns "Notion \"<resource>\" / \"<operation>\" is not
//! supported natively yet". Notion Trigger is out of scope.
//!
//! Faithful quirks kept from n8n's `NotionV2.node.js`/`GenericFunctions.js`:
//! - `databasePage:create` fetches the database schema (to find the title
//!   property's key) *once*, using item 0's `databaseId` -- even when later
//!   items in the same execution have a different (expression-resolved)
//!   `databaseId`. The page's own `parent.database_id` is still read
//!   per-item.
//! - `databasePage:getAll`'s `options.downloadFiles`/`simple`/`filterType`/
//!   `matchType` are all read once from item 0, while `databaseId`,
//!   `returnAll`, the filter conditions and sort are read per item.
//! - `database:get`/`database:getAll`'s `simple` flag is read once from item
//!   0.
//! - Pagination (`notionApiRequestAllItems`) always walks pages via
//!   `start_cursor`/`has_more` (continuing whenever `has_more` is anything
//!   other than exactly `false`, including a missing/`null` value). `block`
//!   and `user` endpoints are `GET`s that carry the cursor as a query
//!   parameter; every other endpoint is a `POST` that carries it in the
//!   JSON body.
//! - `block:getAll`/`database:getAll`/`databasePage:getAll`'s non-`returnAll`
//!   path sends `page_size = min(limit, 100)` and stops paging once it has
//!   `limit` items. `database:search`/`page:search`/`user:getAll`'s
//!   non-`returnAll` path does *not* bound the fetch at all: it walks every
//!   page first and only then truncates client-side to `limit`.
//! - `databasePage:create`/`page:create`'s `options.icon` is written as
//!   `{"emoji": ...}` / `{"external": {"url": ...}}` (no `"type"` field);
//!   `databasePage:update`/`page` icon on update (n8n exposes this only for
//!   `databasePage:update`) is written as `{"type": "emoji", "emoji": ...}`
//!   / `{"type": "external", "external": {"url": ...}}`.
//! - The property/filter value field names are asymmetric in n8n itself:
//!   `date`'s filter value lives in `value.date` (not `dateValue`) and
//!   `last_edited_time`'s in `value.lastEditedTime` (not
//!   `lastEditedTimeValue`); every other filter type follows the
//!   `camelCase(type) + "Value"` pattern.
//! - `NotionApi.credentials.js`'s `authenticate()` sends
//!   `Authorization: Bearer <key> ` with a trailing space -- kept here.
//!
//! Known simplifications vs real n8n: only `apiKey` authentication
//! (`notionOAuth2Api` is out of scope); `options.downloadFiles` on
//! `databasePage:getAll` is accepted but never downloads binary data;
//! `simplifyBlocksOutput` (nodeVersion > 2 only) is not implemented since it
//! does not apply to v2.0/2.2; the `page` resource additionally supports a
//! `get` operation (real n8n's v2 UI does not offer it -- only v1 does -- but
//! it is cheap and useful to support since it is the same `GET /pages/:id`
//! call as `databasePage:get`).

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use chrono::TimeZone;
use serde_json::{json, Map, Value};

pub struct Notion;

// ---- small value helpers ---------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn locator_mode(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("mode").and_then(Value::as_str).unwrap_or("id").to_string(),
        _ => "id".to_string(),
    }
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("Notion \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

/// n8n's `returnJsonArray`: an array response becomes one item per element
/// (non-object elements wrapped under `data`), anything else one item.
fn to_items(value: Value, i: usize) -> Vec<Item> {
    match value {
        Value::Array(a) => a.into_iter().map(|v| Item::from_value(v).paired(i)).collect(),
        other => vec![Item::from_value(other).paired(i)],
    }
}

/// `GenericFunctions.getNameAndType`: splits a "Name|type" key on its last
/// `|`.
fn name_and_type(key: &str) -> (String, String) {
    match key.rfind('|') {
        Some(idx) => (key[..idx].to_string(), key[idx + 1..].to_string()),
        None => (key.to_string(), String::new()),
    }
}

/// `change-case`'s `camelCase` for our purposes: `snake_case` -> `camelCase`.
fn camel_case(s: &str) -> String {
    let mut parts = s.split(['_', '-', ' ']).filter(|p| !p.is_empty());
    let mut out = parts.next().map(str::to_lowercase).unwrap_or_default();
    for p in parts {
        let mut c = p.chars();
        if let Some(f) = c.next() {
            out.extend(f.to_uppercase());
            out.push_str(&c.as_str().to_lowercase());
        }
    }
    out
}

/// `change-case`'s `snakeCase`, folding non-ASCII to spaces first (n8n's
/// `foldedSnakeCase`, used by `simplifyObjects`'s `prepend` for our
/// (pre-v3) target versions).
fn snake_case(s: &str) -> String {
    let folded: String = s.chars().map(|c| if (0x20..=0x7E).contains(&(c as u32)) { c } else { ' ' }).collect();
    let mut out = String::new();
    let mut prev_lower_or_digit = false;
    for c in folded.chars() {
        if c.is_whitespace() || c == '-' || c == '_' {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            prev_lower_or_digit = false;
            continue;
        }
        if c.is_uppercase() {
            if prev_lower_or_digit {
                out.push('_');
            }
            out.extend(c.to_lowercase());
            prev_lower_or_digit = false;
        } else {
            out.push(c);
            prev_lower_or_digit = c.is_lowercase() || c.is_numeric();
        }
    }
    out.trim_matches('_').to_string()
}

// ---- authentication & the underlying HTTP call -----------------------------

struct Auth {
    bearer: String,
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let method = ctx.param_str("authentication", 0, "apiKey")?;
    if method != "apiKey" {
        return Err(NodeError::new("Notion OAuth2 authentication is not supported natively yet").describe("Use API Key (Internal Integration Secret) authentication instead."));
    }
    let (_, cred) = ctx.credentials("notionApi").await?;
    let token = cred["apiKey"].as_str().unwrap_or("").to_string();
    if token.is_empty() {
        return Err(NodeError::new("Notion credentials are not set").describe("Add an Internal Integration Secret to the Notion API credential."));
    }
    let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.notion.com").trim_end_matches('/').to_string();
    Ok(Auth { bearer: token, base_url })
}

/// Descriptive messages for common HTTP status codes, mirroring n8n-workflow's
/// `NodeApiError` `STATUS_CODE_MESSAGES` table (same table used by the
/// GitHub node).
fn status_code_message(code: u16) -> String {
    match code {
        400 => "Bad request - please check your parameters".into(),
        401 => "Authorization failed - please check your credentials".into(),
        402 => "Payment required - perhaps check your payment details?".into(),
        403 => "Forbidden - perhaps check your credentials?".into(),
        404 => "The resource you are requesting could not be found".into(),
        405 => "Method not allowed - please check you are using the right HTTP method".into(),
        429 => "The service is receiving too many requests from you".into(),
        500 => "The service was not able to process your request".into(),
        502 => "Bad gateway - the service failed to handle your request".into(),
        503 => "Service unavailable - try again later or consider setting this node to retry automatically (in the node settings)".into(),
        504 => "Gateway timed out - perhaps try again later?".into(),
        c if (400..500).contains(&c) => "Your request is invalid or could not be processed by the service".into(),
        c if (500..600).contains(&c) => "The service failed to process your request".into(),
        c => format!("Request failed with status code {c}"),
    }
}

fn notion_error(status: u16, body: &Value) -> NodeError {
    let message = status_code_message(status);
    let description = body.get("message").and_then(Value::as_str).map(String::from);
    NodeError::api(message, Some(status), description)
}

async fn notion_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, path: &str, body: Option<&Value>, query: &[(String, String)]) -> NodeResult<Value> {
    let url_str = format!("{}/v1{}", auth.base_url, path);
    let mut url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid Notion API URL: {url_str}")).at(i))?;
    super::check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    // `NotionApi.credentials.js` sends `Bearer <key> ` with a trailing space.
    let mut req = ctx.services.http.request(method, url).header("Authorization", format!("Bearer {} ", auth.bearer)).header("Notion-Version", "2021-08-16");
    if let Some(b) = body {
        if b.as_object().map(|o| !o.is_empty()).unwrap_or(true) {
            req = req.json(b);
        }
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Notion failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(notion_error(status, &parsed).at(i));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| NodeError::new(format!("Notion returned invalid JSON: {e}")).at(i))
}

/// `notionApiRequestAllItems`: pages via `start_cursor`/`has_more`,
/// continuing whenever `has_more` is anything but exactly `false`.
/// `cursor_in_body` selects `GET`-style (query param) vs `POST`-style (JSON
/// body) cursor placement. `stop_at` bounds collection length (`None` walks
/// every page).
#[allow(clippy::too_many_arguments)]
async fn paginate(
    ctx: &ExecCtx<'_>,
    auth: &Auth,
    i: usize,
    method: &str,
    path: &str,
    mut body: Option<Map<String, Value>>,
    mut query: Vec<(String, String)>,
    property: &str,
    cursor_in_body: bool,
    stop_at: Option<i64>,
) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    loop {
        let body_val = body.clone().map(Value::Object);
        let resp = notion_request(ctx, auth, i, method, path, body_val.as_ref(), &query).await?;
        out.extend(resp.get(property).and_then(Value::as_array).cloned().unwrap_or_default());
        let next_cursor = resp.get("next_cursor").and_then(Value::as_str).map(String::from);
        if cursor_in_body {
            let b = body.get_or_insert_with(Map::new);
            match &next_cursor {
                Some(c) => {
                    b.insert("start_cursor".into(), json!(c));
                }
                None => {
                    b.remove("start_cursor");
                }
            }
        } else {
            query.retain(|(k, _)| k != "start_cursor");
            if let Some(c) = &next_cursor {
                query.push(("start_cursor".into(), c.clone()));
            }
        }
        if let Some(limit) = stop_at {
            if out.len() as i64 >= limit {
                out.truncate(limit as usize);
                return Ok(out);
            }
        }
        let has_more = resp.get("has_more").and_then(Value::as_bool);
        if has_more == Some(false) {
            break;
        }
    }
    Ok(out)
}

// ---- date/time (moment-timezone equivalents) --------------------------------

fn parse_flexible(date_str: &str, tz: chrono_tz::Tz) -> NodeResult<chrono::DateTime<chrono_tz::Tz>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(date_str) {
        return Ok(dt.with_timezone(&tz));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        let ndt = d.and_hms_opt(0, 0, 0).unwrap();
        return tz.from_local_datetime(&ndt).single().ok_or_else(|| NodeError::new(format!("\"{date_str}\" is ambiguous in this timezone")));
    }
    Err(NodeError::new(format!("\"{date_str}\" is not a valid date")))
}

fn resolve_tz(name: &str) -> NodeResult<chrono_tz::Tz> {
    name.parse::<chrono_tz::Tz>().map_err(|_| NodeError::new(format!("\"{name}\" is not a known timezone")))
}

/// `moment.tz(date, timezone).format(format)` for a property's `date` value:
/// `yyyy-MM-DD` when `includeTime` is false, an offset ISO 8601 timestamp
/// otherwise.
fn format_property_date(date_str: &str, timezone: &str, include_time: bool) -> NodeResult<String> {
    let tz = resolve_tz(timezone)?;
    let dt = parse_flexible(date_str, tz)?;
    Ok(if include_time { dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string() } else { dt.format("%Y-%m-%d").to_string() })
}

/// `moment.tz(date, timezone).utc().format()` for a filter's `date` value.
fn format_filter_date(date_str: &str, timezone: &str) -> NodeResult<String> {
    let tz = resolve_tz(timezone)?;
    let dt = parse_flexible(date_str, tz)?;
    Ok(dt.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
}

// ---- rich text / blocks (GenericFunctions.getTexts / formatBlocks) ---------

fn get_texts(texts: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    for t in texts {
        let text_type = t.get("textType").and_then(Value::as_str).unwrap_or("text");
        let annotations = t.get("annotationUi").cloned().unwrap_or(json!({}));
        match text_type {
            "mention" => {
                let mention_type = t.get("mentionType").and_then(Value::as_str).unwrap_or("user").to_string();
                let mention = if mention_type == "date" {
                    if t.get("range").and_then(Value::as_bool) == Some(true) {
                        json!({"type": "date", "date": {"start": t.get("dateStart"), "end": t.get("dateEnd")}})
                    } else {
                        json!({"type": "date", "date": {"start": t.get("date"), "end": Value::Null}})
                    }
                } else {
                    let id = t.get(mention_type.as_str()).cloned().unwrap_or(Value::Null);
                    json!({"type": mention_type, mention_type.clone(): {"id": id}})
                };
                out.push(json!({"type": "mention", "mention": mention, "annotations": annotations}));
            }
            "equation" => {
                out.push(json!({"type": "equation", "equation": {"expression": t.get("expression").cloned().unwrap_or(json!(""))}, "annotations": annotations}));
            }
            _ => {
                let content = t.get("text").cloned().unwrap_or(json!(""));
                let mut text_obj = json!({"content": content});
                if t.get("isLink").and_then(Value::as_bool) == Some(true) {
                    if let Some(link) = t.get("textLink").and_then(Value::as_str).filter(|s| !s.is_empty()) {
                        text_obj["link"] = json!({"url": link});
                    }
                }
                out.push(json!({"type": "text", "text": text_obj, "annotations": annotations}));
            }
        }
    }
    out
}

fn format_title(content: &str) -> Value {
    json!({"title": [{"text": {"content": content}}]})
}

fn get_text_blocks(block: &Value) -> Vec<Value> {
    if block.get("richText").and_then(Value::as_bool) == Some(false) {
        let content = block.get("textContent").and_then(Value::as_str).unwrap_or("");
        vec![json!({"text": {"content": content}})]
    } else {
        let texts = block.pointer("/text/text").and_then(Value::as_array).cloned().unwrap_or_default();
        get_texts(&texts)
    }
}

fn format_blocks(blocks: &[Value]) -> Vec<Value> {
    blocks
        .iter()
        .map(|block| {
            let block_type = block.get("type").and_then(Value::as_str).unwrap_or("paragraph").to_string();
            let mut inner = Map::new();
            if block_type == "to_do" {
                inner.insert("checked".into(), block.get("checked").cloned().unwrap_or(json!(false)));
            }
            if block_type == "image" {
                let url = block.get("url").and_then(Value::as_str).unwrap_or("");
                inner.insert("type".into(), json!("external"));
                inner.insert("external".into(), json!({"url": url}));
            } else {
                inner.insert("text".into(), Value::Array(get_text_blocks(block)));
            }
            json!({"object": "block", "type": block_type, block_type: inner})
        })
        .collect()
}

// ---- property mapping (GenericFunctions.getPropertyKeyValue / mapProperties)

fn is_empty(v: &Value) -> bool {
    matches!(v, Value::Null) || v.as_str() == Some("")
}

/// `GenericFunctions.getPropertyKeyValue`: returns `None` when the property
/// should be skipped entirely (n8n's `url`/`ignoreIfEmpty` case).
fn get_property_key_value(value: &Value, prop_type: &str, timezone: &str, i: usize) -> NodeResult<Option<Value>> {
    let ignore_if_empty = value.get("ignoreIfEmpty").and_then(Value::as_bool).unwrap_or(false);
    Ok(Some(match prop_type {
        "rich_text" => {
            if value.get("richText").and_then(Value::as_bool) == Some(false) {
                json!({"rich_text": [{"text": {"content": value.get("textContent").cloned().unwrap_or(json!(""))}}]})
            } else {
                let texts = value.pointer("/text/text").and_then(Value::as_array).cloned().unwrap_or_default();
                json!({"rich_text": get_texts(&texts)})
            }
        }
        "title" => json!({"title": [{"text": {"content": value.get("title").cloned().unwrap_or(json!(""))}}]}),
        "number" => json!({"type": "number", "number": value.get("numberValue").cloned().unwrap_or(Value::Null)}),
        "url" => {
            let url_value = value.get("urlValue").cloned().unwrap_or(Value::Null);
            if is_empty(&url_value) && ignore_if_empty {
                return Ok(None);
            }
            json!({"type": "url", "url": url_value})
        }
        "checkbox" => json!({"type": "checkbox", "checkbox": value.get("checkboxValue").cloned().unwrap_or(json!(false))}),
        "relation" => {
            let raw = value.get("relationValue").and_then(Value::as_array).cloned().unwrap_or_default();
            let ids: Vec<Value> = raw
                .iter()
                .filter_map(Value::as_str)
                .flat_map(|s| s.split(',').map(|p| p.trim().to_string()))
                .filter(|s| !s.is_empty())
                .map(|s| json!({"id": s}))
                .collect();
            json!({"type": "relation", "relation": ids})
        }
        "multi_select" => {
            let raw = value.get("multiSelectValue").cloned().unwrap_or(Value::Null);
            if is_empty(&raw) {
                json!({"type": "multi_select", "multi_select": []})
            } else {
                let items: Vec<String> = match &raw {
                    Value::Array(a) => a.iter().map(value_to_string).collect(),
                    Value::String(s) => s.split(',').map(|p| p.trim().to_string()).collect(),
                    _ => vec![],
                };
                let options: Vec<Value> = items
                    .into_iter()
                    .filter(|s| s != "null")
                    .map(|s| if uuid::Uuid::parse_str(&s).is_ok() { json!({"id": s}) } else { json!({"name": s}) })
                    .collect();
                json!({"type": "multi_select", "multi_select": options})
            }
        }
        "email" => json!({"type": "email", "email": value.get("emailValue").cloned().unwrap_or(Value::Null)}),
        "people" => {
            let raw = value.get("peopleValue").cloned().unwrap_or(json!([]));
            let arr: Vec<Value> = match raw {
                Value::Array(a) => a,
                other => vec![other],
            };
            json!({"type": "people", "people": arr.into_iter().map(|v| json!({"id": v})).collect::<Vec<_>>()})
        }
        "phone_number" => json!({"type": "phone_number", "phone_number": value.get("phoneValue").cloned().unwrap_or(Value::Null)}),
        "select" => {
            let raw = value.get("selectValue").cloned().unwrap_or(Value::Null);
            if is_empty(&raw) {
                json!({"type": "select", "select": null})
            } else {
                json!({"type": "select", "select": {"name": raw}})
            }
        }
        "status" => json!({"type": "status", "status": {"name": value.get("statusValue").cloned().unwrap_or(Value::Null)}}),
        "date" => {
            let include_time = value.get("includeTime").and_then(Value::as_bool).unwrap_or(false);
            let tz_param = value.get("timezone").and_then(Value::as_str).unwrap_or("default");
            let tz = if tz_param == "default" { timezone } else { tz_param };
            let range = value.get("range").and_then(Value::as_bool).unwrap_or(false);
            let (start_raw, end_raw) = if range {
                (value.get("dateStart").and_then(Value::as_str).unwrap_or("").to_string(), value.get("dateEnd").and_then(Value::as_str).unwrap_or("").to_string())
            } else {
                (value.get("date").and_then(Value::as_str).unwrap_or("").to_string(), String::new())
            };
            let empty = if range { start_raw.is_empty() && end_raw.is_empty() } else { start_raw.is_empty() };
            if empty {
                json!({"type": "date", "date": null})
            } else if range {
                json!({"type": "date", "date": {"start": format_property_date(&start_raw, tz, include_time).map_err(|e| e.at(i))?, "end": format_property_date(&end_raw, tz, include_time).map_err(|e| e.at(i))?}})
            } else {
                json!({"type": "date", "date": {"start": format_property_date(&start_raw, tz, include_time).map_err(|e| e.at(i))?, "end": null}})
            }
        }
        "files" => {
            let file_urls = value.pointer("/fileUrls/fileUrl").and_then(Value::as_array).cloned().unwrap_or_default();
            let files: Vec<Value> = file_urls
                .iter()
                .map(|f| json!({"name": f.get("name").cloned().unwrap_or(json!("")), "type": "external", "external": {"url": f.get("url").cloned().unwrap_or(json!(""))}}))
                .collect();
            json!({"type": "files", "files": files})
        }
        _ => return Ok(None),
    }))
}

fn map_properties(properties: &[Value], timezone: &str, i: usize) -> NodeResult<Map<String, Value>> {
    let mut out = Map::new();
    for property in properties {
        let Some(key_raw) = property.get("key").and_then(Value::as_str) else { continue };
        let (name, prop_type) = name_and_type(key_raw);
        if let Some(v) = get_property_key_value(property, &prop_type, timezone, i)? {
            out.insert(name, v);
        }
    }
    Ok(out)
}

/// `GenericFunctions.mapSorting` (`databasePage:getAll`'s `sorts`, an
/// array).
fn map_sorting(list: &[Value]) -> Vec<Value> {
    list.iter()
        .map(|sort| {
            let key_raw = sort.get("key").and_then(Value::as_str).unwrap_or("");
            let (name, _) = name_and_type(key_raw);
            let direction = sort.get("direction").cloned().unwrap_or(json!("ascending"));
            let field = if sort.get("timestamp").and_then(Value::as_bool) == Some(true) { "timestamp" } else { "property" };
            json!({"direction": direction, field: name})
        })
        .collect()
}

/// `GenericFunctions.mapFilters`, applied to one condition at a time (as
/// `NotionV2.node.js` calls it: `conditions.map(data => mapFilters([data],
/// timezone))`).
fn map_filter_one(value: &Value, timezone: &str, i: usize) -> NodeResult<Value> {
    let key_raw = value.get("key").and_then(Value::as_str).unwrap_or("");
    let (name, prop_type) = name_and_type(key_raw);
    let condition = value.get("condition").and_then(Value::as_str).unwrap_or("equals").to_string();
    let is_empty_cond = matches!(condition.as_str(), "is_empty" | "is_not_empty");
    let is_relative = matches!(condition.as_str(), "past_week" | "past_month" | "past_year" | "next_week" | "next_month" | "next_year");

    if prop_type == "formula" {
        let return_type = value.get("returnType").and_then(Value::as_str).unwrap_or("").to_string();
        if is_empty_cond {
            return Ok(json!({"property": name, return_type.clone(): {condition: true}}));
        }
        let field = format!("{}Value", camel_case(&return_type));
        let val = value.get(&field).cloned().unwrap_or(Value::Null);
        return Ok(json!({"property": name, return_type.clone(): {return_type: {condition: val}}}));
    }

    let mut val = if prop_type == "last_edited_time" { value.get("lastEditedTime").cloned().unwrap_or(Value::Null) } else { value.get(format!("{}Value", camel_case(&prop_type))).cloned().unwrap_or(Value::Null) };
    if is_empty_cond {
        val = json!(true);
    } else if is_relative {
        val = json!({});
    }

    let mut out_key = prop_type.clone();
    if prop_type == "rich_text" || prop_type == "text" {
        out_key = "text".into();
    } else if prop_type == "phone_number" {
        out_key = "phone".into();
    } else if prop_type == "date" && !is_empty_cond {
        let raw = value.get("date").and_then(Value::as_str).unwrap_or("");
        val = if raw.is_empty() { json!({}) } else { json!(format_filter_date(raw, timezone).map_err(|e| e.at(i))?) };
    } else if prop_type == "boolean" {
        out_key = "checkbox".into();
    }

    Ok(json!({"property": name, out_key: {condition: val}}))
}

// ---- simplified output (GenericFunctions.simplifyObjects) ------------------

fn simplify_property(property: &Value) -> Value {
    let ptype = property.get("type").and_then(Value::as_str).unwrap_or("");
    let inner = property.get(ptype).cloned().unwrap_or(Value::Null);
    match ptype {
        "rich_text" | "title" => match inner.as_array() {
            Some(a) if !a.is_empty() => json!(a.iter().map(|t| t.get("plain_text").and_then(Value::as_str).unwrap_or("").to_string()).collect::<Vec<_>>().join("")),
            _ => json!(""),
        },
        "url" | "created_time" | "checkbox" | "number" | "last_edited_time" | "email" | "phone_number" | "date" => inner,
        "created_by" | "last_edited_by" | "select" => {
            if inner.is_null() {
                Value::Null
            } else {
                inner.get("name").cloned().unwrap_or(Value::Null)
            }
        }
        "people" => match inner.as_array() {
            Some(a) => json!(a.iter().map(|p| p.pointer("/person/email").cloned().unwrap_or(json!({}))).collect::<Vec<_>>()),
            None => inner,
        },
        "multi_select" => match inner.as_array() {
            Some(a) => json!(a.iter().map(|e| e.get("name").cloned().unwrap_or(json!({}))).collect::<Vec<_>>()),
            None => json!(inner.get("options").and_then(Value::as_array).map(|a| a.iter().map(|e| e.get("name").cloned().unwrap_or(json!({}))).collect::<Vec<_>>()).unwrap_or_default()),
        },
        "relation" => match inner.as_array() {
            Some(a) => json!(a.iter().map(|e| e.get("id").cloned().unwrap_or(json!({}))).collect::<Vec<_>>()),
            None => inner.get("database_id").cloned().unwrap_or(Value::Null),
        },
        "formula" => {
            let sub = inner.get("type").and_then(Value::as_str).unwrap_or("");
            inner.get(sub).cloned().unwrap_or(Value::Null)
        }
        "rollup" => {
            let func = inner.get("function").and_then(Value::as_str).unwrap_or("");
            if func.starts_with("count") || func.contains("empty") {
                let n = inner.get("number").and_then(Value::as_f64).unwrap_or(0.0);
                json!(if func.contains("percent") { n * 100.0 } else { n })
            } else if func.starts_with("show") && inner.get("type").and_then(Value::as_str) == Some("array") {
                let elements: Vec<Value> = inner.get("array").and_then(Value::as_array).into_iter().flatten().map(simplify_property).collect();
                if func == "show_unique" {
                    let mut seen = Vec::new();
                    for e in elements {
                        if !seen.contains(&e) {
                            seen.push(e);
                        }
                    }
                    json!(seen)
                } else {
                    json!(elements)
                }
            } else {
                Value::Null
            }
        }
        "files" => match inner.as_array() {
            Some(a) => json!(a.iter().map(|f| f.get(f.get("type").and_then(Value::as_str).unwrap_or("")).and_then(|x| x.get("url")).cloned().unwrap_or(Value::Null)).collect::<Vec<_>>()),
            None => Value::Null,
        },
        "status" => inner.get("name").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn simplify_properties(properties: &Map<String, Value>) -> Map<String, Value> {
    properties.iter().map(|(k, v)| (k.clone(), simplify_property(v))).collect()
}

fn get_property_title(properties: &Map<String, Value>) -> String {
    properties
        .values()
        .find(|p| p.get("type") == Some(&json!("title")))
        .and_then(|p| p.pointer("/title/0/plain_text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn prepend_property(properties: Map<String, Value>) -> Map<String, Value> {
    properties.into_iter().map(|(k, v)| (format!("property_{}", snake_case(&k)), v)).collect()
}

/// `GenericFunctions.simplifyObjects` for a single object (`download` is
/// always false; see module docs).
fn simplify_object(object: Value) -> Value {
    let obj_type = object.get("object").and_then(Value::as_str).unwrap_or("");
    let parent_type = object.pointer("/parent/type").and_then(Value::as_str).unwrap_or("");
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let url = object.get("url").cloned().unwrap_or(Value::Null);
    if obj_type == "page" && (parent_type == "page_id" || parent_type == "workspace") {
        let name = object.pointer("/properties/title/title/0/plain_text").cloned().unwrap_or(json!(""));
        return json!({"id": id, "name": name, "url": url});
    }
    if obj_type == "page" {
        let properties = object.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
        let name = get_property_title(&properties);
        let mut out = Map::new();
        out.insert("id".into(), id);
        out.insert("name".into(), json!(name));
        out.insert("url".into(), url);
        for (k, v) in prepend_property(simplify_properties(&properties)) {
            out.insert(k, v);
        }
        return Value::Object(out);
    }
    if obj_type == "database" {
        let name = object.pointer("/title/0/plain_text").cloned().unwrap_or(json!(""));
        return json!({"id": id, "name": name, "url": url});
    }
    object
}

fn simplify_objects(objects: Vec<Value>) -> Vec<Value> {
    objects.into_iter().map(simplify_object).collect()
}

// ---- id extraction (GenericFunctions.getPageId / extractPageId / extractResourceId / extractBlockId)

/// `GenericFunctions.extractPageId`, used for the URL mode of `pageId`/
/// `blockId` resource locators.
fn extract_page_id_from_url(raw: &str) -> String {
    if raw.contains("p=") {
        if let Some(v) = raw.split("p=").nth(1) {
            return v.split(['&', '#']).next().unwrap_or(v).to_string();
        }
    }
    if raw.contains('-') && raw.contains("https") {
        return raw.rsplit('-').next().unwrap_or(raw).to_string();
    }
    raw.to_string()
}

/// `GenericFunctions.extractResourceId`, used for `databaseId`.
fn extract_resource_id(raw: &str) -> String {
    if let Some(idx) = raw.find("?v=") {
        let data = &raw[..idx];
        return data.rsplit('/').next().unwrap_or(data).to_string();
    }
    if raw.contains('/') {
        return raw.rsplit('/').next().unwrap_or(raw).to_string();
    }
    raw.to_string()
}

fn database_id(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("databaseId", i)?;
    Ok(extract_resource_id(&locator_str(&v).unwrap_or_default()))
}

/// `GenericFunctions.getPageId`.
fn get_page_id(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("pageId", i)?;
    let mode = locator_mode(&v);
    let value = locator_str(&v).unwrap_or_default();
    let page_id = if mode == "id" { value.clone() } else { extract_page_id_from_url(&value) };
    if page_id.is_empty() {
        return Err(NodeError::new(format!("Could not extract page ID from URL: {value}")).at(i));
    }
    Ok(page_id)
}

/// `GenericFunctions.extractBlockId`.
fn extract_block_id(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let v = ctx.param("blockId", i)?;
    let mode = locator_mode(&v);
    let value = locator_str(&v).unwrap_or_default();
    if mode == "id" {
        return Ok(value);
    }
    if let Some(idx) = value.find('#') {
        let frag = &value[idx + 1..];
        if frag.len() >= 2 && frag.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(frag.to_string());
        }
    }
    let block_id = extract_page_id_from_url(&value);
    if block_id.is_empty() {
        return Err(NodeError::new("Invalid URL, could not find block ID or page ID").at(i));
    }
    Ok(block_id)
}

// ---- filters (getAll manual/JSON build) -------------------------------------

fn build_filter(ctx: &ExecCtx<'_>, i: usize, timezone: &str) -> NodeResult<Option<Value>> {
    let filter_type = ctx.param_str("filterType", 0, "none")?;
    match filter_type.as_str() {
        "manual" => {
            let match_type = ctx.param_str("matchType", 0, "anyFilter")?;
            let conditions = ctx.param("filters.conditions", i)?.as_array().cloned().unwrap_or_default();
            if conditions.is_empty() {
                return Ok(None);
            }
            let mapped: Vec<Value> = conditions.iter().map(|c| map_filter_one(c, timezone, i)).collect::<NodeResult<_>>()?;
            let key = if match_type == "allFilters" { "and" } else { "or" };
            Ok(Some(json!({key: mapped})))
        }
        "json" => {
            let raw = ctx.param_str("filterJson", i, "")?;
            match serde_json::from_str::<Value>(&raw) {
                Ok(v) => Ok(Some(v)),
                Err(_) => Err(NodeError::api("Filters (JSON) must be a valid json", None, None).at(i)),
            }
        }
        _ => Ok(None),
    }
}

// ---- per-resource dispatch --------------------------------------------------

#[allow(clippy::too_many_lines)]
async fn fetch_nested_children(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, blocks: Vec<Value>, limit: Option<i64>) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    let mut stack: Vec<Value> = blocks;
    stack.reverse();
    while let Some(block) = stack.pop() {
        out.push(block.clone());
        if let Some(l) = limit {
            if out.len() as i64 == l {
                return Ok(out);
            }
            if out.len() as i64 > l {
                out.truncate(l as usize);
                return Ok(out);
            }
        }
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        if block_type == "child_page" || block_type == "unsupported" {
            continue;
        }
        if block.get("has_children").and_then(Value::as_bool) == Some(true) {
            let block_id = block.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let endpoint = format!("/blocks/{block_id}/children");
            let children = paginate(ctx, auth, i, "GET", &endpoint, None, vec![], "results", false, None).await?;
            let mut children: Vec<Value> = children
                .into_iter()
                .map(|mut c| {
                    if let Value::Object(o) = &mut c {
                        o.insert("parent_id".into(), json!(block_id));
                    }
                    c
                })
                .collect();
            children.reverse();
            for c in children {
                stack.push(c);
            }
        }
    }
    Ok(out)
}

impl Notion {
    async fn block(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Vec<Item>> {
        match operation {
            "append" => {
                let block_id = extract_block_id(ctx, i)?;
                let block_values = ctx.param("blockUi.blockValues", i)?.as_array().cloned().unwrap_or_default();
                let body = json!({"children": format_blocks(&block_values)});
                let resp = notion_request(ctx, auth, i, "PATCH", &format!("/blocks/{block_id}/children"), Some(&body), &[]).await?;
                Ok(to_items(resp, i))
            }
            "getAll" => {
                let block_id = extract_block_id(ctx, i)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let fetch_nested = ctx.param_bool("fetchNestedBlocks", i, false)?;
                let endpoint = format!("/blocks/{block_id}/children");
                let (mut results, limit) = if return_all {
                    (paginate(ctx, auth, i, "GET", &endpoint, None, vec![], "results", false, None).await?, None)
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    let query = vec![("page_size".to_string(), limit.min(100).to_string())];
                    (paginate(ctx, auth, i, "GET", &endpoint, None, query, "results", false, Some(limit)).await?, Some(limit))
                };
                for r in results.iter_mut() {
                    if let Value::Object(o) = r {
                        o.insert("parent_id".into(), json!(block_id));
                    }
                }
                if fetch_nested {
                    results = fetch_nested_children(ctx, auth, i, results, limit).await?;
                }
                Ok(to_items(Value::Array(results), i))
            }
            other => Err(unsupported("block", other, i)),
        }
    }

    async fn database(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize, timezone: &str) -> NodeResult<Vec<Item>> {
        match operation {
            "get" => {
                let simple = ctx.param_bool("simple", 0, true)?;
                let id = database_id(ctx, i)?;
                let mut resp = notion_request(ctx, auth, i, "GET", &format!("/databases/{id}"), None, &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            "getAll" => {
                let simple = ctx.param_bool("simple", 0, true)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let mut body = Map::new();
                body.insert("filter".into(), json!({"property": "object", "value": "database"}));
                let (mut results, _) = if return_all {
                    (paginate(ctx, auth, i, "POST", "/search", Some(body), vec![], "results", true, None).await?, None::<i64>)
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    body.insert("page_size".into(), json!(limit.min(100)));
                    (paginate(ctx, auth, i, "POST", "/search", Some(body), vec![], "results", true, Some(limit)).await?, Some(limit))
                };
                if simple {
                    results = simplify_objects(results);
                }
                Ok(to_items(Value::Array(results), i))
            }
            "search" => {
                let text = ctx.param_str("text", i, "")?;
                let options = ctx.param("options", i)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut body = Map::new();
                body.insert("filter".into(), json!({"property": "object", "value": "database"}));
                if !text.is_empty() {
                    body.insert("query".into(), json!(text));
                }
                if let Some(sort) = options.get("sort").filter(|s| !s.is_null()) {
                    body.insert("sort".into(), sort.pointer("/sortValue").cloned().unwrap_or(json!({})));
                }
                let mut results = paginate(ctx, auth, i, "POST", "/search", Some(body), vec![], "results", true, None).await?;
                if !return_all {
                    let limit = ctx.param_f64("limit", i, 50.0)? as usize;
                    results.truncate(limit);
                }
                if simple {
                    results = simplify_objects(results);
                }
                let _ = timezone;
                Ok(to_items(Value::Array(results), i))
            }
            other => Err(unsupported("database", other, i)),
        }
    }

    async fn database_page(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize, timezone: &str, title_key: Option<&str>) -> NodeResult<Vec<Item>> {
        match operation {
            "create" => {
                let title_key = title_key.unwrap_or("Name").to_string();
                let title = ctx.param_str("title", i, "")?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut properties = Map::new();
                if !title.is_empty() {
                    properties.insert(title_key, json!({"title": [{"text": {"content": title}}]}));
                }
                let db_id = database_id(ctx, i)?;
                let property_values = ctx.param("propertiesUi.propertyValues", i)?.as_array().cloned().unwrap_or_default();
                if !property_values.is_empty() {
                    for (k, v) in map_properties(&property_values, timezone, i)? {
                        properties.insert(k, v);
                    }
                }
                let block_values = ctx.param("blockUi.blockValues", i)?.as_array().cloned().unwrap_or_default();
                let mut body = Map::new();
                body.insert("parent".into(), json!({"database_id": db_id}));
                body.insert("properties".into(), Value::Object(properties));
                body.insert("children".into(), Value::Array(format_blocks(&block_values)));
                apply_icon(ctx, i, &mut body, false)?;
                let mut resp = notion_request(ctx, auth, i, "POST", "/pages", Some(&Value::Object(body)), &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            "get" => {
                let page_id = get_page_id(ctx, i)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut resp = notion_request(ctx, auth, i, "GET", &format!("/pages/{page_id}"), None, &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            "getAll" => {
                let simple = ctx.param_bool("simple", 0, true)?;
                let db_id = database_id(ctx, i)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let sort_list = ctx.param("options.sort.sortValue", i)?.as_array().cloned().unwrap_or_default();
                let mut body = Map::new();
                if let Some(filter) = build_filter(ctx, i, timezone)? {
                    body.insert("filter".into(), filter);
                }
                if !sort_list.is_empty() {
                    body.insert("sorts".into(), Value::Array(map_sorting(&sort_list)));
                }
                let endpoint = format!("/databases/{db_id}/query");
                let mut results = if return_all {
                    paginate(ctx, auth, i, "POST", &endpoint, Some(body), vec![], "results", true, None).await?
                } else {
                    let limit = ctx.param_f64("limit", i, 50.0)? as i64;
                    body.insert("page_size".into(), json!(limit.min(100)));
                    paginate(ctx, auth, i, "POST", &endpoint, Some(body), vec![], "results", true, Some(limit)).await?
                };
                if simple {
                    results = simplify_objects(results);
                }
                Ok(to_items(Value::Array(results), i))
            }
            "update" => {
                let page_id = get_page_id(ctx, i)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let property_values = ctx.param("propertiesUi.propertyValues", i)?.as_array().cloned().unwrap_or_default();
                let mut body = Map::new();
                let properties = if property_values.is_empty() { Map::new() } else { map_properties(&property_values, timezone, i)? };
                body.insert("properties".into(), Value::Object(properties));
                apply_icon(ctx, i, &mut body, true)?;
                let mut resp = notion_request(ctx, auth, i, "PATCH", &format!("/pages/{page_id}"), Some(&Value::Object(body)), &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            other => Err(unsupported("databasePage", other, i)),
        }
    }

    async fn page(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Vec<Item>> {
        match operation {
            "archive" => {
                let page_id = get_page_id(ctx, i)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut resp = notion_request(ctx, auth, i, "PATCH", &format!("/pages/{page_id}"), Some(&json!({"archived": true})), &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            "create" => {
                let simple = ctx.param_bool("simple", i, true)?;
                let parent_id = get_page_id(ctx, i)?;
                let title = ctx.param_str("title", i, "")?;
                let block_values = ctx.param("blockUi.blockValues", i)?.as_array().cloned().unwrap_or_default();
                let mut body = Map::new();
                body.insert("parent".into(), json!({"page_id": parent_id}));
                body.insert("properties".into(), format_title(&title));
                body.insert("children".into(), Value::Array(format_blocks(&block_values)));
                apply_icon(ctx, i, &mut body, false)?;
                let mut resp = notion_request(ctx, auth, i, "POST", "/pages", Some(&Value::Object(body)), &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            // Real n8n's v2 UI does not expose "page":"get" (only v1 does),
            // but it is the same GET /pages/:id call as databasePage:get.
            "get" => {
                let page_id = get_page_id(ctx, i)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut resp = notion_request(ctx, auth, i, "GET", &format!("/pages/{page_id}"), None, &[]).await?;
                if simple {
                    resp = simplify_object(resp);
                }
                Ok(to_items(resp, i))
            }
            "search" => {
                let text = ctx.param_str("text", i, "")?;
                let options = ctx.param("options", i)?;
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let simple = ctx.param_bool("simple", i, true)?;
                let mut body = Map::new();
                if !text.is_empty() {
                    body.insert("query".into(), json!(text));
                }
                if let Some(filter) = options.get("filter").filter(|f| !f.is_null()) {
                    body.insert("filter".into(), filter.get("filters").cloned().unwrap_or(json!([])));
                }
                if let Some(sort) = options.get("sort").filter(|s| !s.is_null()) {
                    body.insert("sort".into(), sort.pointer("/sortValue").cloned().unwrap_or(json!({})));
                }
                let mut results = paginate(ctx, auth, i, "POST", "/search", Some(body), vec![], "results", true, None).await?;
                if !return_all {
                    let limit = ctx.param_f64("limit", i, 50.0)? as usize;
                    results.truncate(limit);
                }
                if simple {
                    results = simplify_objects(results);
                }
                Ok(to_items(Value::Array(results), i))
            }
            other => Err(unsupported("page", other, i)),
        }
    }

    async fn user(&self, ctx: &ExecCtx<'_>, auth: &Auth, operation: &str, i: usize) -> NodeResult<Vec<Item>> {
        match operation {
            "get" => {
                let user_id = ctx.param_str("userId", i, "")?;
                let resp = notion_request(ctx, auth, i, "GET", &format!("/users/{user_id}"), None, &[]).await?;
                Ok(to_items(resp, i))
            }
            "getAll" => {
                let return_all = ctx.param_bool("returnAll", i, false)?;
                let mut results = paginate(ctx, auth, i, "GET", "/users", None, vec![], "results", false, None).await?;
                if !return_all {
                    let limit = ctx.param_f64("limit", i, 50.0)? as usize;
                    results.truncate(limit);
                }
                Ok(to_items(Value::Array(results), i))
            }
            other => Err(unsupported("user", other, i)),
        }
    }
}

/// `options.icon`/`options.iconType`: on create, written without a `"type"`
/// field; on update, with one.
fn apply_icon(ctx: &ExecCtx<'_>, i: usize, body: &mut Map<String, Value>, tagged: bool) -> NodeResult<()> {
    let options = ctx.param("options", i)?;
    let icon = options.get("icon").and_then(Value::as_str).filter(|s| !s.is_empty());
    let Some(icon) = icon else { return Ok(()) };
    let icon_type = options.get("iconType").and_then(Value::as_str).unwrap_or("emoji");
    let value = if icon_type == "file" {
        if tagged {
            json!({"type": "external", "external": {"url": icon}})
        } else {
            json!({"external": {"url": icon}})
        }
    } else if tagged {
        json!({"type": "emoji", "emoji": icon})
    } else {
        json!({"emoji": icon})
    };
    body.insert("icon".into(), value);
    Ok(())
}

#[async_trait::async_trait]
impl NodeType for Notion {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.notion"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "page")?;
        let operation = ctx.param_str("operation", 0, "create")?;
        let timezone = ctx.workflow.setting_str("timezone").unwrap_or(&ctx.services.config.timezone).to_string();
        let auth = resolve_auth(ctx).await?;

        // `databasePage:create` fetches the database schema once, from item
        // 0's `databaseId`, to find the title property's key -- see module
        // docs.
        let title_key = if resource == "databasePage" && operation == "create" && !input.is_empty() {
            match database_id(ctx, 0) {
                Ok(id) => match notion_request(ctx, &auth, 0, "GET", &format!("/databases/{id}"), None, &[]).await {
                    Ok(schema) => schema
                        .get("properties")
                        .and_then(Value::as_object)
                        .and_then(|props| props.iter().find(|(_, v)| v.get("type") == Some(&json!("title"))))
                        .map(|(k, _)| k.clone()),
                    Err(e) => return Err(e),
                },
                Err(e) => return Err(e),
            }
        } else {
            None
        };

        let mut out = Vec::new();
        for (i, _item) in input.iter().enumerate() {
            let result = match resource.as_str() {
                "block" => self.block(ctx, &auth, &operation, i).await,
                "database" => self.database(ctx, &auth, &operation, i, &timezone).await,
                "databasePage" => self.database_page(ctx, &auth, &operation, i, &timezone, title_key.as_deref()).await,
                "page" => self.page(ctx, &auth, &operation, i).await,
                "user" => self.user(ctx, &auth, &operation, i).await,
                other => Err(NodeError::new(format!("Notion resource \"{other}\" is not supported natively yet")).at(i)),
            };
            match result {
                Ok(items) => out.extend(items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => return Err(e),
            }
        }
        Ok(vec![out])
    }
}
