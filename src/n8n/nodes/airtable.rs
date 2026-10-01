//! Airtable node (spec §6.6), v2.x (2.0/2.1/2.2 -- 2.2 is n8n's
//! `defaultVersion`), faithful to n8n's `nodes/Airtable/v2/*`.
//!
//! Implements:
//!
//! - `base`: getMany, getSchema
//! - `record`: create, deleteRecord, get, search (incl. `filterByFormula`,
//!   `sort`, `returnAll`/`limit`, `view`, `fields`), update, upsert (incl.
//!   the `columns` resourceMapper's `autoMapInputData`/`defineBelow` shape,
//!   `columnToMatchOn`, `ignoreFields`, `typecast`, `updateAllMatches`, and
//!   batching up to 10 records per PATCH)
//!
//! Anything else returns "Airtable \"<resource>\" / \"<operation>\" is not
//! supported natively yet". Airtable Trigger is out of scope.
//!
//! Faithful quirks kept from n8n's `v2/actions/*`/`helpers/utils.js`:
//! - `record`'s `base`/`table` resource locators are resolved *once*, from
//!   item 0 (`router.js`'s `this.getNodeParameter('base', 0, ...)`), even
//!   when later items in the same execution would resolve a different
//!   (expression-resolved) value.
//! - `legacyFlattenOutput` only applies to `record:get` and `record:search`:
//!   for `nodeVersion >= 2.2` (n8n's default) it is a no-op (the record
//!   stays `{id, createdTime, fields: {...}}`); below 2.2 the `fields`
//!   object is spread into the top level instead (`{id, createdTime,
//!   ...fields}`). `create`/`update`/`upsert`/`deleteRecord` never flatten,
//!   at any version.
//! - `record:update`'s `defineBelow` + non-`id` matching branch removes
//!   `columnsToMatchOn` (not `options.ignoreFields`) from the fields sent
//!   back to Airtable -- an asymmetry versus every other branch, which all
//!   use `options.ignoreFields`. Kept as-is.
//! - `record:update`'s `get`-many-records-for-matching pre-fetch
//!   (`apiRequestAllItems` with `fields: columnsToMatchOn`) and
//!   `columns.mappingMode`/`columns.matchingColumns` are all read *once*,
//!   before the per-item loop.
//! - `record:search`'s `options`/`sort` are read per item (`i`), matching
//!   n8n's nodeVersion >= 2.1 (our default) per-item loop; the pre-2.1
//!   once-only-at-item-0 fallback loop is not implemented (see Known
//!   simplifications).
//!
//! Known simplifications vs real n8n: only `airtableTokenApi` (Personal
//! Access Token) and `airtableOAuth2Api` (pre-connected
//! `oauthTokenData.access_token`, like `slack.rs`) authentication; the
//! `Download Attachments` option on `record:get`/`record:search` is
//! accepted but binary attachments are never downloaded (same
//! simplification as Notion's `downloadFiles`); `record:upsert`'s two
//! Airtable-API-version error-recovery fallbacks (a 422 on an
//! `id`-matching upsert retried as a plain `create`, and "Cannot update
//! more than one record" retried via a `filterByFormula` re-fetch) are not
//! implemented -- Airtable's `performUpsert` is relied on directly;
//! `record:search`'s legacy (pre-2.1) once-only loop is not implemented;
//! `processAirtableError`'s record-ID substitution only threads through
//! for `record:get`/`record:deleteRecord` (the ID is cheap to obtain
//! there); continueOnFail error items always use the generic `{"error":
//! message}` shape rather than each operation's own (some add a nested
//! `error` object, others don't -- irrelevant since none of that extra
//! structure carries the record ID or other data the BDD suite asserts on).

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Map, Value};

pub struct Airtable;

// ---- small value helpers ---------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// A `resourceLocator` value's `.value`, or the value itself when it is
/// already a plain string (r8r workflows may supply either shape).
fn locator_str(v: &Value) -> Option<String> {
    match v {
        Value::Object(o) => o.get("value").map(value_to_string),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

fn unsupported(resource: &str, operation: &str, i: usize) -> NodeError {
    NodeError::new(format!("Airtable \"{resource}\" / \"{operation}\" is not supported natively yet")).at(i)
}

/// n8n's `wrapData`/`returnJsonArray`: an array response becomes one item
/// per element (non-object elements wrapped under `data`), anything else
/// one item.
fn to_items(value: Value, i: usize) -> Vec<Item> {
    match value {
        Value::Array(a) => a.into_iter().map(|v| Item::from_value(v).paired(i)).collect(),
        other => vec![Item::from_value(other).paired(i)],
    }
}

/// `encodeURI` semantics: reserved characters (incl. `/`) are left alone.
/// Used for the table name in the record endpoint (`router.js`'s
/// `encodeURI(this.getNodeParameter('table', ...))`).
fn encode_uri(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'('
            | b')' | b';' | b',' | b'/' | b'?' | b':' | b'@' | b'&' | b'=' | b'+' | b'$' | b'#' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `qs.stringify`'s default (`arrayFormat: 'indices'`) bracket notation:
/// arrays become `key[0]=a&key[1]=b`, objects become `key[k]=v`.
fn qs_push(prefix: &str, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Array(a) => {
            for (idx, v) in a.iter().enumerate() {
                qs_push(&format!("{prefix}[{idx}]"), v, out);
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                qs_push(&format!("{prefix}[{k}]"), v, out);
            }
        }
        Value::Null => {}
        other => out.push((prefix.to_string(), value_to_string(other))),
    }
}

// ---- authentication & the underlying HTTP call -----------------------------

struct Auth {
    bearer: String,
    base_url: String,
}

async fn resolve_auth(ctx: &ExecCtx<'_>) -> NodeResult<Auth> {
    let method = ctx.param_str("authentication", 0, "airtableTokenApi")?;
    if method == "airtableOAuth2Api" {
        let (_, cred) = ctx.credentials("airtableOAuth2Api").await?;
        let token = cred
            .pointer("/oauthTokenData/access_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| NodeError::new("The Airtable OAuth2 credential is not connected").describe("Complete the OAuth2 authorization for this credential before using it."))?
            .to_string();
        let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.airtable.com/v0").trim_end_matches('/').to_string();
        Ok(Auth { bearer: token, base_url })
    } else {
        let (_, cred) = ctx.credentials("airtableTokenApi").await?;
        let token = cred["accessToken"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            return Err(NodeError::new("Airtable credentials are not set").describe("Add an Access Token to the Airtable Personal Access Token credential."));
        }
        let base_url = cred["url"].as_str().filter(|s| !s.is_empty()).unwrap_or("https://api.airtable.com/v0").trim_end_matches('/').to_string();
        Ok(Auth { bearer: token, base_url })
    }
}

/// Descriptive messages for common HTTP status codes, mirroring
/// n8n-workflow's `NodeApiError` `STATUS_CODE_MESSAGES` table (same table
/// used by the GitHub/Notion/Slack nodes).
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

fn extract_description(body: &Value) -> Option<String> {
    body.pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| body.get("error").and_then(Value::as_str))
        .or_else(|| body.pointer("/error/type").and_then(Value::as_str))
        .or_else(|| body.get("message").and_then(Value::as_str))
        .map(String::from)
}

fn airtable_error(status: u16, body: &Value) -> NodeError {
    let message = status_code_message(status);
    let description = extract_description(body);
    NodeError::api(message, Some(status), description)
}

/// `helpers/utils.js`'s `processAirtableError`: substitutes a clearer
/// description when the API reports `NOT_FOUND` (or the up-to-10-records
/// validation error) for a known record ID.
fn process_airtable_error(mut err: NodeError, id: Option<&str>) -> NodeError {
    if let Some(id) = id {
        let replace = match &err.description {
            Some(d) if d == "NOT_FOUND" => true,
            Some(d) if d.contains("You must provide an array of up to 10 record objects") => true,
            _ => false,
        };
        if replace {
            err.description = Some(format!("{id} is not a valid Record ID"));
        }
    }
    err
}

async fn airtable_request(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, endpoint: &str, body: Option<&Value>, query: &[(String, String)]) -> NodeResult<Value> {
    let url_str = format!("{}/{}", auth.base_url, endpoint);
    let mut url = reqwest::Url::parse(&url_str).map_err(|_| NodeError::new(format!("Invalid Airtable API URL: {url_str}")).at(i))?;
    super::check_ssrf(&url, ctx.config()).await.map_err(|m| NodeError::new(m).at(i))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (k, v) in query {
            pairs.append_pair(k, v);
        }
    }
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|_| NodeError::new(format!("Invalid HTTP method \"{method}\"")).at(i))?;
    let mut req = ctx.services.http.request(method, url).bearer_auth(&auth.bearer);
    if let Some(b) = body {
        if b.as_object().map(|o| !o.is_empty()).unwrap_or(true) {
            req = req.json(b);
        }
    }
    let resp = req.send().await.map_err(|e| NodeError::api(format!("The request to Airtable failed: {e}"), None, None).at(i))?;
    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.unwrap_or_default();
    if status >= 400 {
        let parsed: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        return Err(airtable_error(status, &parsed).at(i));
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|e| NodeError::new(format!("Airtable returned invalid JSON: {e}")).at(i))
}

/// `transport/index.js`'s `apiRequestAllItems`: pages via `pageSize=100` +
/// `offset`, continuing while the response carries an `offset`.
async fn paginate(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, method: &str, endpoint: &str, mut query: Vec<(String, String)>) -> NodeResult<Vec<Value>> {
    query.retain(|(k, _)| k != "pageSize" && k != "offset");
    query.push(("pageSize".into(), "100".into()));
    let mut out = Vec::new();
    loop {
        let resp = airtable_request(ctx, auth, i, method, endpoint, None, &query).await?;
        out.extend(resp.get("records").and_then(Value::as_array).cloned().unwrap_or_default());
        query.retain(|(k, _)| k != "offset");
        match resp.get("offset").and_then(Value::as_str) {
            Some(o) => query.push(("offset".into(), o.to_string())),
            None => break,
        }
    }
    Ok(out)
}

/// `transport/index.js`'s `batchUpdate`: PATCHes up to 10 records per
/// request, concatenating the `records` arrays of each response.
async fn batch_update(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, extra: &Map<String, Value>, records: Vec<Value>) -> NodeResult<Vec<Value>> {
    if records.is_empty() {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    for chunk in records.chunks(10) {
        let mut body = extra.clone();
        body.insert("records".into(), Value::Array(chunk.to_vec()));
        let resp = airtable_request(ctx, auth, i, "PATCH", endpoint, Some(&Value::Object(body)), &[]).await?;
        out.extend(resp.get("records").and_then(Value::as_array).cloned().unwrap_or_default());
    }
    Ok(out)
}

/// `legacyFlattenOutput`: a no-op from node version 2.2 on (n8n's
/// default); below that, `fields` is spread into the top level.
fn legacy_flatten_output(record: Value, node_version: f64) -> Value {
    if node_version >= 2.2 {
        return record;
    }
    let Value::Object(mut obj) = record else { return record };
    let fields = obj.remove("fields").and_then(|f| f.as_object().cloned()).unwrap_or_default();
    for (k, v) in fields {
        obj.insert(k, v);
    }
    Value::Object(obj)
}

/// `helpers/utils.js`'s `removeIgnored`: a falsy (absent/empty-string)
/// `ignore` returns `data` untouched.
fn remove_ignored(data: &Map<String, Value>, ignore: &Value) -> Map<String, Value> {
    let ignore_fields: Vec<String> = match ignore {
        Value::String(s) if !s.is_empty() => s.split(',').map(|p| p.trim().to_string()).collect(),
        Value::Array(a) => a.iter().map(value_to_string).collect(),
        _ => return data.clone(),
    };
    data.iter().filter(|(k, _)| !ignore_fields.contains(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// `helpers/utils.js`'s `valuesMatch`: Airtable returns array values for
/// Lookup/Linked-Record fields while user input is typically scalar.
fn values_match(record_value: &Value, input_value: &Value) -> bool {
    if record_value == input_value {
        return true;
    }
    if let Value::Array(rv) = record_value {
        if let Value::Array(iv) = input_value {
            return rv.len() == iv.len() && rv.iter().zip(iv.iter()).all(|(a, b)| a == b);
        }
        return rv.contains(input_value);
    }
    false
}

/// `helpers/utils.js`'s `findMatches`.
fn find_matches(data: &[Value], keys: &[String], fields: &Map<String, Value>, update_all: bool) -> NodeResult<Vec<Value>> {
    let null = Value::Null;
    let is_match = |record: &&Value| -> bool { keys.iter().all(|k| values_match(record.pointer(&format!("/fields/{k}")).unwrap_or(&null), fields.get(k).unwrap_or(&null))) };
    if update_all {
        let matches: Vec<Value> = data.iter().filter(is_match).cloned().collect();
        if matches.is_empty() {
            return Err(NodeError::new("No records match provided keys"));
        }
        Ok(matches)
    } else {
        match data.iter().find(is_match) {
            Some(r) => Ok(vec![r.clone()]),
            None => Err(NodeError::new("Record matching provided keys was not found")),
        }
    }
}

/// `helpers/utils.js`'s `coerceArrayTypeFields`: when `typecast` bypasses
/// client-side validation, a `schema`-declared `array`-type field's
/// JSON-stringified value is parsed back into an actual array.
fn coerce_array_type_fields(fields: &mut Map<String, Value>, raw_columns: &Value) {
    let Some(schema) = raw_columns.get("schema").and_then(Value::as_array) else { return };
    for field in schema {
        if field.get("type").and_then(Value::as_str) != Some("array") {
            continue;
        }
        let Some(id) = field.get("id").and_then(Value::as_str) else { continue };
        if let Some(Value::String(s)) = fields.get(id) {
            if let Ok(parsed @ Value::Array(_)) = serde_json::from_str::<Value>(s) {
                fields.insert(id.to_string(), parsed);
            }
        }
    }
}

// ---- resourceMapper (`columns`) helpers ------------------------------------

fn mapping_value(columns: &Value) -> Map<String, Value> {
    columns.get("value").and_then(Value::as_object).cloned().unwrap_or_default()
}

fn matching_columns(columns: &Value) -> Vec<String> {
    columns.get("matchingColumns").and_then(Value::as_array).map(|a| a.iter().map(value_to_string).collect()).unwrap_or_default()
}

// ---- per-operation execution ------------------------------------------------

async fn record_create(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, data_mode: &str, item: &Item) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let typecast = options.get("typecast").and_then(Value::as_bool).unwrap_or(false);
    let fields = if data_mode == "autoMapInputData" {
        remove_ignored(&item.json, options.get("ignoreFields").unwrap_or(&Value::Null))
    } else {
        let columns = ctx.param("columns", i)?;
        let mut fields = mapping_value(&columns);
        if typecast {
            coerce_array_type_fields(&mut fields, &columns);
        }
        fields
    };
    let body = json!({"typecast": typecast, "fields": fields});
    let resp = airtable_request(ctx, auth, i, "POST", endpoint, Some(&body), &[]).await.map_err(|e| process_airtable_error(e, None))?;
    Ok(to_items(resp, i))
}

async fn record_get(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, node_version: f64) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("id", i, "")?;
    let resp = airtable_request(ctx, auth, i, "GET", &format!("{endpoint}/{id}"), None, &[]).await.map_err(|e| process_airtable_error(e, Some(&id)))?;
    Ok(to_items(legacy_flatten_output(resp, node_version), i))
}

async fn record_delete(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str) -> NodeResult<Vec<Item>> {
    let id = ctx.param_str("id", i, "")?;
    let resp = airtable_request(ctx, auth, i, "DELETE", &format!("{endpoint}/{id}"), None, &[]).await.map_err(|e| process_airtable_error(e, Some(&id)))?;
    Ok(to_items(resp, i))
}

async fn record_search(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, node_version: f64) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", i, true)?;
    let options = ctx.param("options", i)?;
    let sort = ctx.param("sort", i)?;
    let filter_by_formula = ctx.param_str("filterByFormula", i, "")?;

    let mut query: Vec<(String, String)> = Vec::new();
    if !filter_by_formula.is_empty() {
        query.push(("filterByFormula".into(), filter_by_formula));
    }
    if let Some(fields) = options.get("fields") {
        let arr: Value = match fields {
            Value::String(s) if !s.is_empty() => json!(s.split(',').map(|p| p.trim().to_string()).collect::<Vec<_>>()),
            Value::Array(a) if !a.is_empty() => json!(a),
            _ => Value::Null,
        };
        if !arr.is_null() {
            qs_push("fields", &arr, &mut query);
        }
    }
    if let Some(props) = sort.get("property").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        qs_push("sort", &json!(props), &mut query);
    }
    if let Some(view) = options.pointer("/view/value").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        query.push(("view".into(), view.to_string()));
    }

    let records = if return_all {
        paginate(ctx, auth, i, "GET", endpoint, query).await?
    } else {
        let limit = ctx.param_f64("limit", i, 100.0)? as i64;
        query.push(("maxRecords".into(), limit.to_string()));
        let resp = airtable_request(ctx, auth, i, "GET", endpoint, None, &query).await?;
        resp.get("records").and_then(Value::as_array).cloned().unwrap_or_default()
    };
    let items = records.into_iter().map(|r| Item::from_value(legacy_flatten_output(r, node_version)).paired(i)).collect();
    Ok(items)
}

#[allow(clippy::too_many_arguments)]
async fn record_update_item(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, data_mode: &str, matching: &[String], table_data: &[Value], item: &Item) -> NodeResult<(Vec<Item>, Option<String>)> {
    let options = ctx.param("options", i)?;
    let typecast = options.get("typecast").and_then(Value::as_bool).unwrap_or(false);
    let update_all = options.get("updateAllMatches").and_then(Value::as_bool).unwrap_or(false);
    let has_id_match = matching.iter().any(|m| m == "id");

    let mut records = Vec::new();
    let mut record_id = None;
    if data_mode == "autoMapInputData" {
        if has_id_match {
            let mut fields = item.json.clone();
            let id = fields.remove("id").map(|v| value_to_string(&v)).unwrap_or_default();
            record_id = Some(id.clone());
            records.push(json!({"id": id, "fields": remove_ignored(&fields, options.get("ignoreFields").unwrap_or(&Value::Null))}));
        } else {
            let matches = find_matches(table_data, matching, &item.json, update_all).map_err(|e| e.at(i))?;
            for m in matches {
                let id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                records.push(json!({"id": id, "fields": remove_ignored(&item.json, options.get("ignoreFields").unwrap_or(&Value::Null))}));
            }
        }
    } else {
        let columns = ctx.param("columns", i)?;
        let mut fields = mapping_value(&columns);
        if typecast {
            coerce_array_type_fields(&mut fields, &columns);
        }
        if has_id_match {
            let id = fields.remove("id").map(|v| value_to_string(&v)).unwrap_or_default();
            record_id = Some(id.clone());
            records.push(json!({"id": id, "fields": fields}));
        } else {
            let matches = find_matches(table_data, matching, &fields, update_all).map_err(|e| e.at(i))?;
            for m in matches {
                let id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                // Faithful n8n quirk: this branch ignores `options.ignoreFields`
                // and instead strips the matching columns themselves.
                records.push(json!({"id": id, "fields": remove_ignored(&fields, &json!(matching))}));
            }
        }
    }

    let mut extra = Map::new();
    extra.insert("typecast".into(), json!(typecast));
    let updated = batch_update(ctx, auth, i, endpoint, &extra, records).await?;
    Ok((updated.into_iter().map(|v| Item::from_value(v).paired(i)).collect(), record_id))
}

#[allow(clippy::too_many_arguments)]
async fn record_upsert_item(ctx: &ExecCtx<'_>, auth: &Auth, i: usize, endpoint: &str, data_mode: &str, matching: &[String], item: &Item) -> NodeResult<Vec<Item>> {
    let options = ctx.param("options", i)?;
    let typecast = options.get("typecast").and_then(Value::as_bool).unwrap_or(false);
    let has_id_match = matching.iter().any(|m| m == "id");

    let mut records = Vec::new();
    if data_mode == "autoMapInputData" {
        let mut fields = item.json.clone();
        if has_id_match {
            let id = fields.remove("id").map(|v| value_to_string(&v)).unwrap_or_default();
            records.push(json!({"id": id, "fields": remove_ignored(&fields, options.get("ignoreFields").unwrap_or(&Value::Null))}));
        } else {
            records.push(json!({"fields": remove_ignored(&fields, options.get("ignoreFields").unwrap_or(&Value::Null))}));
        }
    } else {
        let columns = ctx.param("columns", i)?;
        let mut fields = mapping_value(&columns);
        if typecast {
            coerce_array_type_fields(&mut fields, &columns);
        }
        if has_id_match {
            let id = fields.remove("id").map(|v| value_to_string(&v)).unwrap_or_default();
            records.push(json!({"id": id, "fields": fields}));
        } else {
            records.push(json!({"fields": fields}));
        }
    }

    let mut extra = Map::new();
    extra.insert("typecast".into(), json!(typecast));
    if !has_id_match {
        extra.insert("performUpsert".into(), json!({"fieldsToMergeOn": matching}));
    }
    let updated = batch_update(ctx, auth, i, endpoint, &extra, records).await.map_err(|e| process_airtable_error(e, None))?;
    Ok(updated.into_iter().map(|v| Item::from_value(v).paired(i)).collect())
}

async fn base_get_many(ctx: &ExecCtx<'_>, auth: &Auth) -> NodeResult<Vec<Item>> {
    let return_all = ctx.param_bool("returnAll", 0, true)?;
    let mut bases: Vec<Value> = if return_all {
        paginate_bases(ctx, auth).await?
    } else {
        let resp = airtable_request(ctx, auth, 0, "GET", "meta/bases", None, &[]).await?;
        let mut all = resp.get("bases").and_then(Value::as_array).cloned().unwrap_or_default();
        let limit = ctx.param_f64("limit", 0, 100.0)? as usize;
        all.truncate(limit);
        all
    };
    let levels: Vec<String> = ctx.param("options.permissionLevel", 0)?.as_array().into_iter().flatten().map(value_to_string).collect();
    if !levels.is_empty() {
        bases.retain(|b| b.get("permissionLevel").and_then(Value::as_str).map(|p| levels.iter().any(|l| l == p)).unwrap_or(false));
    }
    Ok(to_items(Value::Array(bases), 0))
}

/// `meta/bases` pagination: the same `offset`-param style as records, but
/// walked manually (not via `apiRequestAllItems`, which expects a
/// `records` array).
async fn paginate_bases(ctx: &ExecCtx<'_>, auth: &Auth) -> NodeResult<Vec<Value>> {
    let mut out = Vec::new();
    let mut query: Vec<(String, String)> = Vec::new();
    loop {
        let resp = airtable_request(ctx, auth, 0, "GET", "meta/bases", None, &query).await?;
        out.extend(resp.get("bases").and_then(Value::as_array).cloned().unwrap_or_default());
        query.retain(|(k, _)| k != "offset");
        match resp.get("offset").and_then(Value::as_str) {
            Some(o) => query.push(("offset".into(), o.to_string())),
            None => break,
        }
    }
    Ok(out)
}

async fn base_get_schema(ctx: &ExecCtx<'_>, auth: &Auth, i: usize) -> NodeResult<Vec<Item>> {
    let base_id = locator_str(&ctx.param("base", i)?).unwrap_or_default();
    let resp = airtable_request(ctx, auth, i, "GET", &format!("meta/bases/{base_id}/tables"), None, &[]).await.map_err(|e| process_airtable_error(e, None))?;
    Ok(to_items(resp.get("tables").cloned().unwrap_or(json!([])), i))
}

#[async_trait::async_trait]
impl NodeType for Airtable {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.airtable"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let resource = ctx.param_str("resource", 0, "record")?;
        let operation = ctx.param_str("operation", 0, "get")?;
        let node_version = ctx.node.type_version;
        let auth = resolve_auth(ctx).await?;

        if resource == "base" {
            let mut out = Vec::new();
            match operation.as_str() {
                "getMany" => match base_get_many(ctx, &auth).await {
                    Ok(items) => out.extend(items),
                    Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, 0),
                    Err(e) => return Err(e),
                },
                "getSchema" => {
                    for (i, _) in input.iter().enumerate() {
                        match base_get_schema(ctx, &auth, i).await {
                            Ok(items) => out.extend(items),
                            Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                            Err(e) => return Err(e),
                        }
                    }
                }
                other => return Err(unsupported("base", other, 0)),
            }
            return Ok(vec![out]);
        }

        if resource != "record" {
            return Err(NodeError::new(format!("Airtable resource \"{resource}\" is not supported natively yet")));
        }

        // `router.js` resolves `base`/`table` once, from item 0.
        let base_id = locator_str(&ctx.param("base", 0)?).unwrap_or_default();
        let table = encode_uri(&locator_str(&ctx.param("table", 0)?).unwrap_or_default());
        let endpoint = format!("{base_id}/{table}");

        let mut out = Vec::new();
        match operation.as_str() {
            "create" => {
                let data_mode = ctx.param_str("columns.mappingMode", 0, "defineBelow")?;
                for (i, item) in input.iter().enumerate() {
                    match record_create(ctx, &auth, i, &endpoint, &data_mode, item).await {
                        Ok(items) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            "get" => {
                for (i, _) in input.iter().enumerate() {
                    match record_get(ctx, &auth, i, &endpoint, node_version).await {
                        Ok(items) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            "deleteRecord" => {
                for (i, _) in input.iter().enumerate() {
                    match record_delete(ctx, &auth, i, &endpoint).await {
                        Ok(items) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            "search" => {
                for (i, _) in input.iter().enumerate() {
                    match record_search(ctx, &auth, i, &endpoint, node_version).await {
                        Ok(items) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            "update" => {
                let data_mode = ctx.param_str("columns.mappingMode", 0, "defineBelow")?;
                let columns0 = ctx.param("columns", 0)?;
                let matching = matching_columns(&columns0);
                let table_data = if !matching.iter().any(|m| m == "id") {
                    let mut q = Vec::new();
                    qs_push("fields", &json!(matching), &mut q);
                    paginate(ctx, &auth, 0, "GET", &endpoint, q).await?
                } else {
                    vec![]
                };
                for (i, item) in input.iter().enumerate() {
                    match record_update_item(ctx, &auth, i, &endpoint, &data_mode, &matching, &table_data, item).await {
                        Ok((items, _)) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            "upsert" => {
                let data_mode = ctx.param_str("columns.mappingMode", 0, "defineBelow")?;
                let columns0 = ctx.param("columns", 0)?;
                let matching = matching_columns(&columns0);
                for (i, item) in input.iter().enumerate() {
                    match record_upsert_item(ctx, &auth, i, &endpoint, &data_mode, &matching, item).await {
                        Ok(items) => out.extend(items),
                        Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                        Err(e) => return Err(e),
                    }
                }
            }
            other => return Err(unsupported("record", other, 0)),
        }
        Ok(vec![out])
    }
}
