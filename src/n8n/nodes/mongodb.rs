//! MongoDB (spec §6.6): `aggregate`, `delete` (`deleteMany`), `find`,
//! `findOneAndReplace`, `findOneAndUpdate`, `insert`, `update`, plus the
//! Atlas-only search-index operations (`listSearchIndexes`,
//! `createSearchIndex`, `dropSearchIndex`, `updateSearchIndex`), against
//! the `mongoDb` credential. Faithful to n8n's MongoDb node
//! (`nodes/MongoDb/MongoDb.node.js` / `GenericFunctions.js`, pinned n8n
//! 2.35.5, adjacent to the target 2.35.7), typeVersion 1.4 -- the latest
//! in the node's `version: [1, 1.1, 1.2, 1.3, 1.4]` array and what the
//! n8n 2.35.7 editor creates by default. Only the >=1.3/>=1.4 code paths
//! (per-item loop semantics for findOneAndReplace/findOneAndUpdate/
//! insert/update, and `serializeMongoItems`'s deep ObjectId/Date
//! stringification for output) are implemented -- r8r registers a single
//! typeVersion, so the reference's older <1.3 batch-mode and <1.4
//! shallow-`stringifyObjectIDs` branches are out of scope.
//!
//! ## EJSON, deviating from the pinned reference
//!
//! The pinned compiled JS has **no** `parseJsonToEjson` helper (that
//! name, mentioned in the task brief, isn't in this n8n version) --
//! `query`/pipeline JSON is parsed with plain `JSON.parse`
//! (`parseAndResolveQueryParameters`), and the only ObjectId coercion is
//! a hardcoded one: if the parsed `find`/`aggregate` query has a
//! *top-level* `_id` that's a plain string, it's replaced with `new
//! ObjectId(...)`. r8r reproduces that hardcoded case exactly (see
//! `to_bson_filter`), but also converts the query through `bson`'s
//! MongoDB Extended JSON v2 support (`Bson::try_from(serde_json::Value)`,
//! which understands `{"$oid": ...}` / `{"$date": ...}` wrapper objects
//! anywhere in the document) -- a deliberate superset of the pinned
//! reference, since the task explicitly asks for EJSON support and it's
//! a strict improvement (any query the reference accepts parses
//! identically; `{"$oid": ...}` queries, which the reference can't
//! express at all beyond the top-level `_id` special case, additionally
//! work).
//!
//! ## Phase split
//!
//! `execute` resolves every per-item parameter (expressions,
//! `options.*`/`fields`/`updateKey` lookups, JSON/EJSON parsing) into an
//! owned [`OpPlan`] synchronously first -- nothing here touches a
//! connection -- then does the actual Mongo work inside a
//! `tokio::spawn`ed task fed only that owned (`Document`/`String`) data,
//! mirroring `mysql.rs`/`postgres.rs`'s split (`ctx` is referenced again
//! after the spawned task's `.await`, via `push_error_item`).
//!
//! ## Known gaps
//! - TLS: `ca`/`cert`/`key` PEM content is written to short-lived temp
//!   files (the driver's `TlsOptions` wants file paths, not in-memory PEM
//!   like n8n's `createSecureContext`); `passphrase`-protected keys
//!   aren't supported (rustls doesn't decrypt encrypted PKCS#8 keys).
//!   Untested here -- the BDD Mongo container (`r8r-bdd-mongo`) runs
//!   without TLS.
//! - Binary field routing (`routeBinaryProperties`, nodeVersion >= 1.4):
//!   not implemented. A `Binary`/`Buffer` value is emitted as a base64
//!   string within `json` instead of being moved to the item's `binary`
//!   output -- matches the reference's own fallback for *nested* binary
//!   values, just applied uniformly (including at the top level).
//! - Search-index operations are implemented against the driver's native
//!   `create_search_index`/`drop_search_index`/`update_search_index`/
//!   `list_search_indexes` methods, but those are Atlas Search-only
//!   server commands -- against the plain community `mongod` this suite
//!   runs against, they fail with the server's own "not supported"
//!   error, same as real n8n would get from the same server.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::nodes::get_path;
use crate::n8n::types::{Item, NodeOutput};
use chrono::TimeZone;
use futures_util::TryStreamExt;
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document};
use mongodb::options::{ClientOptions, Credential, ServerAddress, Tls, TlsOptions};
use mongodb::{Client, Database, SearchIndexModel};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::time::Duration;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(MongoDb)]
}

struct MongoDb;

#[async_trait::async_trait]
impl NodeType for MongoDb {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.mongoDb"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let (_, cred) = ctx.credentials("mongoDb").await?;
        let conn = resolve_connection(&cred)?;

        let resource = ctx.param_str("resource", 0, "document")?;
        let default_op = if resource == "searchIndexes" { "createSearchIndex" } else { "find" };
        let operation = ctx.param_str("operation", 0, default_op)?;
        let items_len = ctx.input().len();

        let plans = resolve_plan(ctx, &resource, &operation, items_len)?;
        let continue_on_fail = ctx.continue_on_fail();

        let password = conn.password.clone();
        let handle = tokio::spawn(run(conn, plans, continue_on_fail));
        let outcome = handle.await.map_err(|e| NodeError::new(format!("The MongoDB task panicked: {e}")))?;
        let outcome = outcome.map_err(|e| scrub(e, &password))?;

        for (idx, e) in outcome.errors {
            ctx.push_error_item(&scrub(e, &password), idx);
        }
        Ok(vec![outcome.items])
    }
}

/// Removes any occurrence of the credential password from an error's
/// message/description, so a connection/auth failure never echoes it
/// back into execution data (matching the other DB nodes' "no password
/// leak" guarantee) regardless of whether the leak would have come from
/// the driver's own error text or from a connection string r8r built.
fn scrub(mut e: NodeError, password: &str) -> NodeError {
    if !password.is_empty() {
        e.message = e.message.replace(password, "***");
        e.description = e.description.map(|d| d.replace(password, "***"));
    }
    e
}

// ---- connection / credentials -------------------------------------------------

struct ConnConfig {
    configuration_type: String,
    connection_string: String,
    database: String,
    host: String,
    port: Option<u32>,
    user: String,
    password: String,
    tls: bool,
    ca: String,
    cert: String,
    key: String,
}

fn cred_str(cred: &Value, key: &str) -> String {
    cred.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn cred_bool(cred: &Value, key: &str) -> bool {
    cred.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(false)
}

/// n8n's `validateAndResolveMongoCredentials` / `buildMongoConnectionParams`.
fn resolve_connection(cred: &Value) -> NodeResult<ConnConfig> {
    let configuration_type = { let s = cred_str(cred, "configurationType"); if s.is_empty() { "values".to_string() } else { s } };
    let database = cred_str(cred, "database").trim().to_string();
    let host = { let s = cred_str(cred, "host"); if s.is_empty() { "localhost".to_string() } else { s } };
    let port = cred.get("port").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|p| p as u32);
    let user = cred_str(cred, "user");
    let password = cred_str(cred, "password");
    let tls = cred_bool(cred, "tls");
    let ca = cred_str(cred, "ca");
    let cert = cred_str(cred, "cert");
    let key = cred_str(cred, "key");

    let connection_string = if configuration_type == "connectionString" {
        let cs = cred_str(cred, "connectionString").trim().to_string();
        if cs.is_empty() {
            return Err(NodeError::new("Cannot override credentials: valid MongoDB connection string not provided "));
        }
        cs
    } else {
        String::new() // built directly from fields below, never as a password-bearing string
    };

    Ok(ConnConfig { configuration_type, connection_string, database, host, port, user, password, tls, ca, cert, key })
}

/// Loads TLS PEM content into short-lived temp files (the driver's
/// `TlsOptions` is file-path based; n8n builds an in-memory
/// `SecureContext` instead -- see module doc).
struct TlsFiles {
    ca: Option<std::path::PathBuf>,
    cert_key: Option<std::path::PathBuf>,
}

impl Drop for TlsFiles {
    fn drop(&mut self) {
        if let Some(p) = &self.ca {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = &self.cert_key {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn write_tls_files(conn: &ConnConfig) -> std::io::Result<TlsFiles> {
    let dir = std::env::temp_dir();
    let unique = uuid::Uuid::new_v4();
    let ca = if conn.ca.trim().is_empty() {
        None
    } else {
        let path = dir.join(format!("r8r-mongo-ca-{unique}.pem"));
        std::fs::write(&path, &conn.ca)?;
        Some(path)
    };
    let cert_key = if conn.cert.trim().is_empty() && conn.key.trim().is_empty() {
        None
    } else {
        let path = dir.join(format!("r8r-mongo-certkey-{unique}.pem"));
        std::fs::write(&path, format!("{}\n{}\n", conn.cert, conn.key))?;
        Some(path)
    };
    Ok(TlsFiles { ca, cert_key })
}

async fn connect(conn: &ConnConfig) -> NodeResult<(Client, Database)> {
    let mut options = if conn.configuration_type == "connectionString" {
        ClientOptions::parse(&conn.connection_string).await.map_err(|e| NodeError::new(e.to_string()))?
    } else {
        let port = conn.port.filter(|p| *p != 0);
        let credential = if conn.user.is_empty() { None } else { Some(Credential::builder().username(conn.user.clone()).password(conn.password.clone()).build()) };
        ClientOptions::builder().hosts(vec![ServerAddress::Tcp { host: conn.host.clone(), port: port.map(|p| p as u16) }]).credential(credential).build()
    };
    options.server_selection_timeout = Some(options.server_selection_timeout.unwrap_or(Duration::from_secs(5)));
    options.connect_timeout = Some(options.connect_timeout.unwrap_or(Duration::from_secs(5)));

    let _tls_files = if conn.tls {
        let files = write_tls_files(conn).map_err(|e| NodeError::new(format!("Could not write TLS certificate files: {e}")))?;
        let mut tls_opts = TlsOptions::default();
        tls_opts.ca_file_path = files.ca.clone();
        tls_opts.cert_key_file_path = files.cert_key.clone();
        options.tls = Some(Tls::Enabled(tls_opts));
        Some(files)
    } else {
        None
    };

    let client = Client::with_options(options).map_err(|e| NodeError::new(e.to_string()))?;
    let db = client.database(&conn.database);
    Ok((client, db))
}

// ---- parameter resolution (ctx phase, synchronous) -----------------------------

enum OpPlan {
    Aggregate { collection: String, pipeline: Vec<Document> },
    Delete { collection: String, filter: Document },
    Find { collection: String, filter: Document, limit: i64, skip: i64, sort: Option<Document>, projection: Option<Document> },
    FindOneAndReplace { collection: String, filter: Document, document: Document, upsert: bool },
    FindOneAndUpdate { collection: String, filter: Document, document: Document, upsert: bool },
    Update { collection: String, filter: Document, document: Document, upsert: bool },
    Insert { collection: String, document: Document },
    ListSearchIndexes { collection: String, name: Option<String> },
    CreateSearchIndex { collection: String, name: String, definition: Document, index_type: String },
    DropSearchIndex { collection: String, name: String },
    UpdateSearchIndex { collection: String, name: String, definition: Document },
}

fn resolve_plan(ctx: &mut ExecCtx<'_>, resource: &str, operation: &str, items_len: usize) -> NodeResult<Vec<(usize, OpPlan)>> {
    let continue_on_fail = ctx.continue_on_fail();
    let mut plans = Vec::with_capacity(items_len);
    for i in 0..items_len {
        let resolved: NodeResult<OpPlan> = match (resource, operation) {
            ("document", "aggregate") => resolve_aggregate(ctx, i),
            ("document", "delete") => resolve_delete(ctx, i),
            ("document", "find") => resolve_find(ctx, i),
            ("document", "findOneAndReplace") => resolve_find_one_and_replace(ctx, i),
            ("document", "findOneAndUpdate") => resolve_find_one_and_update(ctx, i),
            ("document", "insert") => resolve_insert(ctx, i),
            ("document", "update") => resolve_update(ctx, i),
            ("searchIndexes", "listSearchIndexes") => resolve_list_search_indexes(ctx, i),
            ("searchIndexes", "createSearchIndex") => resolve_create_search_index(ctx, i),
            ("searchIndexes", "dropSearchIndex") => resolve_drop_search_index(ctx, i),
            ("searchIndexes", "updateSearchIndex") => resolve_update_search_index(ctx, i),
            _ => Err(NodeError::new(format!("The operation \"{operation}\" is not supported!")).at(i)),
        };
        match resolved {
            Ok(plan) => plans.push((i, plan)),
            Err(e) if continue_on_fail => ctx.push_error_item(&e, i),
            Err(e) => return Err(e),
        }
    }
    Ok(plans)
}

fn prepare_fields(raw: &str) -> Vec<String> {
    raw.split(',').map(|f| f.trim().to_string()).filter(|f| !f.is_empty()).collect()
}

/// n8n's `jsonParse(query, { errorMessage: "Invalid JSON in 'Query'" })`
/// followed by `$1`/`$2`/... placeholder substitution from `queryParameters`
/// (`parseAndResolveQueryParameters`).
fn parse_and_resolve_query_parameters(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Value> {
    let raw_query = ctx.param_str("query", i, "{}")?;
    let parsed: Value = serde_json::from_str(&raw_query).map_err(|_| NodeError::new("Invalid JSON in 'Query'").at(i))?;

    let raw_params = ctx.param_str("queryParameters", i, "[]")?;
    let params_val: Value = serde_json::from_str(&raw_params).map_err(|_| NodeError::new("Query Parameters must be valid JSON").describe("Enter the parameters as a JSON array").at(i))?;
    let Value::Array(params) = params_val else {
        return Err(NodeError::new("Query Parameters must be a JSON array").describe("Enter the parameters as a JSON array").at(i));
    };
    for (idx, p) in params.iter().enumerate() {
        let ok = is_scalar_json(p) || matches!(p, Value::Array(a) if a.iter().all(is_scalar_json));
        if !ok {
            return Err(NodeError::new(format!("Query parameter {} must be a scalar or an array of scalars", idx + 1)).describe("Objects and nested arrays are not supported").at(i));
        }
    }
    if params.is_empty() {
        return Ok(parsed);
    }

    let mut used = HashSet::new();
    let resolved = resolve_placeholders(parsed, &params, &mut used, i)?;
    if let Some(unused) = (0..params.len()).find(|idx| !used.contains(idx)) {
        return Err(NodeError::new(format!("Query parameter {} is not used", unused + 1)).describe(format!("Add ${} to the query or remove the unused parameter", unused + 1)).at(i));
    }
    Ok(resolved)
}

fn is_scalar_json(v: &Value) -> bool {
    matches!(v, Value::Null | Value::String(_) | Value::Number(_) | Value::Bool(_))
}

fn resolve_placeholders(value: Value, params: &[Value], used: &mut HashSet<usize>, i: usize) -> NodeResult<Value> {
    match value {
        Value::String(s) => {
            let digits = s.strip_prefix('$').filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()));
            let Some(digits) = digits else { return Ok(Value::String(s)) };
            let n: usize = digits.parse().unwrap_or(0);
            let idx = n.wrapping_sub(1);
            if n == 0 || idx >= params.len() {
                return Err(NodeError::new(format!("Query placeholder ${n} has no matching value")).describe(format!("Add a value for ${n} to Query Parameters")).at(i));
            }
            used.insert(idx);
            Ok(params[idx].clone())
        }
        Value::Array(arr) => Ok(Value::Array(arr.into_iter().map(|v| resolve_placeholders(v, params, used, i)).collect::<NodeResult<Vec<_>>>()?)),
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k, resolve_placeholders(v, params, used, i)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other),
    }
}

/// Converts a resolved query JSON value into a `Document`, via `bson`'s
/// Extended JSON v2 support (see module doc for how this is a superset of
/// the pinned reference). `apply_id_coercion` additionally mirrors the
/// reference's hardcoded `_id`-string-to-`ObjectId` coercion, used by
/// `find`/`aggregate` only.
fn to_bson_filter(value: Value, apply_id_coercion: bool, i: usize) -> NodeResult<Document> {
    let bson_val = Bson::try_from(value).map_err(|e| NodeError::new(format!("Invalid JSON in 'Query': {e}")).at(i))?;
    let Bson::Document(mut doc) = bson_val else {
        return Err(NodeError::new("Invalid JSON in 'Query'").describe("The query must be a JSON object").at(i));
    };
    if apply_id_coercion {
        if let Some(Bson::String(s)) = doc.get("_id").cloned() {
            let oid = mongodb::bson::oid::ObjectId::parse_str(&s).map_err(|e| NodeError::new(e.to_string()).at(i))?;
            doc.insert("_id", Bson::ObjectId(oid));
        }
    }
    Ok(doc)
}

fn parse_options_doc(ctx: &ExecCtx<'_>, path: &str, i: usize) -> NodeResult<Option<Document>> {
    let raw = ctx.param_str(path, i, "{}")?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let v: Value = serde_json::from_str(&raw).map_err(|e| NodeError::new(format!("Invalid JSON: {e}")).at(i))?;
    let bson_val = Bson::try_from(v).map_err(|e| NodeError::new(e.to_string()).at(i))?;
    match bson_val {
        Bson::Document(d) if !d.is_empty() => Ok(Some(d)),
        _ => Ok(None),
    }
}

fn resolve_aggregate(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let value = parse_and_resolve_query_parameters(ctx, i)?;
    let bson_val = Bson::try_from(value).map_err(|e| NodeError::new(format!("Invalid JSON in 'Query': {e}")).at(i))?;
    let Bson::Array(stages) = bson_val else {
        return Err(NodeError::new("MongoDB aggregation pipeline in JSON format must be an array").at(i));
    };
    let pipeline = stages
        .into_iter()
        .map(|b| match b {
            Bson::Document(d) => Ok(d),
            _ => Err(NodeError::new("Each aggregation pipeline stage must be a JSON object").at(i)),
        })
        .collect::<NodeResult<Vec<_>>>()?;
    Ok(OpPlan::Aggregate { collection, pipeline })
}

fn resolve_delete(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let value = parse_and_resolve_query_parameters(ctx, i)?;
    let filter = to_bson_filter(value, false, i)?;
    Ok(OpPlan::Delete { collection, filter })
}

fn resolve_find(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let value = parse_and_resolve_query_parameters(ctx, i)?;
    let filter = to_bson_filter(value, true, i)?;
    let limit = ctx.param_f64("options.limit", i, 0.0)? as i64;
    let skip = ctx.param_f64("options.skip", i, 0.0)? as i64;
    let sort = parse_options_doc(ctx, "options.sort", i)?;
    let projection = parse_options_doc(ctx, "options.projection", i)?;
    Ok(OpPlan::Find { collection, filter, limit, skip, sort, projection })
}

fn describe_value_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

fn is_js_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `new Date(value)`: a number is epoch milliseconds, a string is parsed
/// as RFC3339/ISO-8601, `YYYY-MM-DD HH:MM:SS` or `YYYY-MM-DD`.
fn parse_js_date(value: &Value) -> Option<BsonDateTime> {
    match value {
        Value::Number(n) => Some(BsonDateTime::from_millis(n.as_f64()? as i64)),
        Value::String(s) => {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                return Some(BsonDateTime::from_millis(dt.with_timezone(&chrono::Utc).timestamp_millis()));
            }
            if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
                return Some(BsonDateTime::from_millis(chrono::Utc.from_utc_datetime(&ndt).timestamp_millis()));
            }
            if let Ok(nd) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                let ndt = nd.and_hms_opt(0, 0, 0)?;
                return Some(BsonDateTime::from_millis(chrono::Utc.from_utc_datetime(&ndt).timestamp_millis()));
            }
            None
        }
        _ => None,
    }
}

fn bson_set_dot_path(doc: &mut Document, path: &str, value: Bson) {
    let parts: Vec<&str> = path.split('.').collect();
    let Some((last, init)) = parts.split_last() else { return };
    let mut cur = doc;
    for part in init {
        let entry = cur.entry((*part).to_string()).or_insert_with(|| Bson::Document(Document::new()));
        if !matches!(entry, Bson::Document(_)) {
            *entry = Bson::Document(Document::new());
        }
        cur = match entry {
            Bson::Document(d) => d,
            _ => unreachable!(),
        };
    }
    cur.insert((*last).to_string(), value);
}

/// n8n's `prepareItems` (single-item form, matching nodeVersion >= 1.3's
/// per-item loop): builds the document to insert/replace/`$set`, applies
/// `dateFields` conversion and `useDotNotation`, and -- when `update_key`
/// is set -- returns `None` if the input item is missing that field
/// (filtered out, matching `data = items.filter(...)`).
#[allow(clippy::too_many_arguments)]
fn prepare_item(ctx: &ExecCtx<'_>, i: usize, fields: &[String], update_key: &str, use_dot_notation: bool, date_fields: &[String], is_update: bool) -> NodeResult<Option<Document>> {
    let json_map = ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default();
    let json_val = Value::Object(json_map.clone());

    let mut fields = fields.to_vec();
    if !update_key.is_empty() {
        if !fields.iter().any(|f| f == update_key) {
            fields.push(update_key.to_string());
        }
        if !json_map.contains_key(update_key) {
            return Ok(None);
        }
    }

    let mut out = Document::new();
    for field in &fields {
        let raw: Value = if use_dot_notation { get_path(&json_val, field).cloned().unwrap_or(Value::Null) } else { json_map.get(field).cloned().unwrap_or(Value::Null) };

        let as_date = is_js_truthy(&raw) && date_fields.iter().any(|f| f == field);
        let (bson_val, is_scalar, type_name): (Bson, bool, &'static str) = if as_date {
            let dt = parse_js_date(&raw).ok_or_else(|| NodeError::new(format!("\"{field}\" could not be parsed as a date")).at(i))?;
            (Bson::DateTime(dt), true, "date")
        } else {
            let scalar = is_scalar_json(&raw);
            let tn = describe_value_type(&raw);
            let b = Bson::try_from(raw).map_err(|e| NodeError::new(e.to_string()).at(i))?;
            (b, scalar, tn)
        };

        if field == update_key && !is_scalar {
            return Err(NodeError::new(format!("The value of \"{update_key}\" must be a string, number, boolean, or date"))
                .describe(format!("Got {type_name} instead. Objects and arrays are not allowed as the match value."))
                .at(i));
        }

        if use_dot_notation && !is_update {
            bson_set_dot_path(&mut out, field, bson_val);
        } else {
            out.insert(field.clone(), bson_val);
        }
    }
    Ok(Some(out))
}

/// Shared by `findOneAndReplace`/`findOneAndUpdate`/`update`: prepares the
/// item, then splits it into `(collection, filter, body)`, extracting
/// `updateKey` into the filter (and, when `updateKey == "_id"`, coercing
/// it to an `ObjectId` and removing it from the body) -- matches the
/// reference's `filter[updateKey] = item[updateKey]; if (updateKey ===
/// '_id') { filter[updateKey] = new ObjectId(item[updateKey]); delete
/// item._id; }`.
fn resolve_prepared_item(ctx: &ExecCtx<'_>, i: usize, is_update: bool) -> NodeResult<Option<(String, Document, Document)>> {
    let collection = ctx.param_str("collection", i, "")?;
    let fields = prepare_fields(&ctx.param_str("fields", i, "")?);
    let use_dot_notation = ctx.param_bool("options.useDotNotation", i, false)?;
    let date_fields = prepare_fields(&ctx.param_str("options.dateFields", i, "")?);
    let update_key = ctx.param_str("updateKey", i, "")?.trim().to_string();

    let Some(mut item) = prepare_item(ctx, i, &fields, &update_key, use_dot_notation, &date_fields, is_update)? else {
        return Ok(None);
    };

    let mut filter = Document::new();
    if !update_key.is_empty() {
        let key_val = item.get(&update_key).cloned().unwrap_or(Bson::Null);
        if update_key == "_id" {
            let id_str = match &key_val {
                Bson::String(s) => s.clone(),
                other => other.to_string(),
            };
            let oid = mongodb::bson::oid::ObjectId::parse_str(&id_str).map_err(|e| NodeError::new(e.to_string()).at(i))?;
            filter.insert("_id", Bson::ObjectId(oid));
            item.remove("_id");
        } else {
            filter.insert(update_key.clone(), key_val);
        }
    }
    Ok(Some((collection, filter, item)))
}

fn resolve_find_one_and_replace(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let upsert = ctx.param_bool("upsert", i, false)?;
    let Some((collection, filter, document)) = resolve_prepared_item(ctx, i, false)? else {
        return Err(NodeError::new("Item is missing the updateKey field").at(i));
    };
    Ok(OpPlan::FindOneAndReplace { collection, filter, document, upsert })
}

fn resolve_find_one_and_update(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let upsert = ctx.param_bool("upsert", i, false)?;
    let Some((collection, filter, document)) = resolve_prepared_item(ctx, i, true)? else {
        return Err(NodeError::new("Item is missing the updateKey field").at(i));
    };
    Ok(OpPlan::FindOneAndUpdate { collection, filter, document, upsert })
}

fn resolve_update(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let upsert = ctx.param_bool("upsert", i, false)?;
    let Some((collection, filter, document)) = resolve_prepared_item(ctx, i, true)? else {
        return Err(NodeError::new("Item is missing the updateKey field").at(i));
    };
    Ok(OpPlan::Update { collection, filter, document, upsert })
}

fn resolve_insert(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let fields = prepare_fields(&ctx.param_str("fields", i, "")?);
    let use_dot_notation = ctx.param_bool("options.useDotNotation", i, false)?;
    let date_fields = prepare_fields(&ctx.param_str("options.dateFields", i, "")?);
    let document = prepare_item(ctx, i, &fields, "", use_dot_notation, &date_fields, false)?.unwrap_or_default();
    Ok(OpPlan::Insert { collection, document })
}

fn resolve_list_search_indexes(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let name = ctx.param_str("indexName", i, "")?;
    Ok(OpPlan::ListSearchIndexes { collection, name: if name.is_empty() { None } else { Some(name) } })
}

fn resolve_create_search_index(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let name = ctx.param_str("indexNameRequired", i, "")?;
    let index_type = ctx.param_str("indexType", i, "vectorSearch")?;
    let raw = ctx.param_str("indexDefinition", i, "{}")?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| NodeError::new(format!("Invalid JSON in 'Index Definition': {e}")).at(i))?;
    let definition = to_bson_filter(v, false, i)?;
    Ok(OpPlan::CreateSearchIndex { collection, name, definition, index_type })
}

fn resolve_drop_search_index(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let name = ctx.param_str("indexNameRequired", i, "")?;
    Ok(OpPlan::DropSearchIndex { collection, name })
}

fn resolve_update_search_index(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<OpPlan> {
    let collection = ctx.param_str("collection", i, "")?;
    let name = ctx.param_str("indexNameRequired", i, "")?;
    let raw = ctx.param_str("indexDefinition", i, "{}")?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| NodeError::new(format!("Invalid JSON in 'Index Definition': {e}")).at(i))?;
    let definition = to_bson_filter(v, false, i)?;
    Ok(OpPlan::UpdateSearchIndex { collection, name, definition })
}

// ---- Mongo work (owned/'static; runs inside tokio::spawn) ----------------------

struct Outcome {
    items: Vec<Item>,
    errors: Vec<(usize, NodeError)>,
}

async fn run(conn: ConnConfig, plans: Vec<(usize, OpPlan)>, continue_on_fail: bool) -> NodeResult<Outcome> {
    let (_client, db) = connect(&conn).await?;

    if matches!(plans.first(), Some((_, OpPlan::Insert { .. }))) {
        return run_insert(&db, plans, continue_on_fail).await;
    }

    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (idx, plan) in plans {
        let outcome: Result<Vec<Item>, mongodb::error::Error> = run_one(&db, plan, idx).await;
        match outcome {
            Ok(new_items) => items.extend(new_items),
            Err(e) => {
                let ne = mongo_error(e);
                if continue_on_fail {
                    errors.push((idx, ne));
                } else {
                    return Err(ne);
                }
            }
        }
    }
    Ok(Outcome { items, errors })
}

fn mongo_error(e: mongodb::error::Error) -> NodeError {
    NodeError::new(e.to_string())
}

async fn run_one(db: &Database, plan: OpPlan, idx: usize) -> Result<Vec<Item>, mongodb::error::Error> {
    match plan {
        OpPlan::Aggregate { collection, pipeline } => {
            let coll = db.collection::<Document>(&collection);
            let mut cursor = coll.aggregate(pipeline).await?;
            let mut out = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                out.push(doc_to_item(doc, idx));
            }
            Ok(out)
        }
        OpPlan::Delete { collection, filter } => {
            let coll = db.collection::<Document>(&collection);
            let res = coll.delete_many(filter).await?;
            let mut m = Map::new();
            m.insert("deletedCount".into(), serde_json::json!(res.deleted_count));
            Ok(vec![Item::new(m).paired(idx)])
        }
        OpPlan::Find { collection, filter, limit, skip, sort, projection } => {
            let coll = db.collection::<Document>(&collection);
            let mut action = coll.find(filter);
            if skip > 0 {
                action = action.skip(skip as u64);
            }
            if limit > 0 {
                action = action.limit(limit);
            }
            if let Some(sort) = sort {
                action = action.sort(sort);
            }
            if let Some(projection) = projection {
                action = action.projection(projection);
            }
            let mut cursor = action.await?;
            let mut out = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                out.push(doc_to_item(doc, idx));
            }
            Ok(out)
        }
        OpPlan::FindOneAndReplace { collection, filter, document, upsert } => {
            let coll = db.collection::<Document>(&collection);
            let mut action = coll.find_one_and_replace(filter, document.clone());
            if upsert {
                action = action.upsert(true);
            }
            action.await?;
            Ok(vec![doc_to_item(document, idx)])
        }
        OpPlan::FindOneAndUpdate { collection, filter, document, upsert } => {
            let coll = db.collection::<Document>(&collection);
            let update = mongodb::bson::doc! { "$set": document.clone() };
            let mut action = coll.find_one_and_update(filter, update);
            if upsert {
                action = action.upsert(true);
            }
            action.await?;
            Ok(vec![doc_to_item(document, idx)])
        }
        OpPlan::Update { collection, filter, document, upsert } => {
            let coll = db.collection::<Document>(&collection);
            let update = mongodb::bson::doc! { "$set": document.clone() };
            let mut action = coll.update_one(filter, update);
            if upsert {
                action = action.upsert(true);
            }
            action.await?;
            Ok(vec![doc_to_item(document, idx)])
        }
        OpPlan::ListSearchIndexes { collection, name } => {
            let coll = db.collection::<Document>(&collection);
            let mut action = coll.list_search_indexes();
            if let Some(name) = name {
                action = action.name(name);
            }
            let mut cursor = action.await?;
            let mut out = Vec::new();
            while let Some(doc) = cursor.try_next().await? {
                out.push(doc_to_item(doc, idx));
            }
            Ok(out)
        }
        OpPlan::CreateSearchIndex { collection, name, definition, index_type } => {
            let coll = db.collection::<Document>(&collection);
            let index_type = match index_type.as_str() {
                "search" => mongodb::SearchIndexType::Search,
                "vectorSearch" => mongodb::SearchIndexType::VectorSearch,
                other => mongodb::SearchIndexType::Other(other.to_string()),
            };
            let model = SearchIndexModel::builder().name(name).definition(definition).index_type(index_type).build();
            coll.create_search_index(model).await?;
            let mut m = Map::new();
            m.insert("indexName".into(), serde_json::json!(coll.name()));
            Ok(vec![Item::new(m).paired(idx)])
        }
        OpPlan::DropSearchIndex { collection, name } => {
            let coll = db.collection::<Document>(&collection);
            coll.drop_search_index(name.clone()).await?;
            let mut m = Map::new();
            m.insert(name, Value::Bool(true));
            Ok(vec![Item::new(m).paired(idx)])
        }
        OpPlan::UpdateSearchIndex { collection, name, definition } => {
            let coll = db.collection::<Document>(&collection);
            coll.update_search_index(name.clone(), definition).await?;
            let mut m = Map::new();
            m.insert(name, Value::Bool(true));
            Ok(vec![Item::new(m).paired(idx)])
        }
        OpPlan::Insert { .. } => unreachable!("insert handled by run_insert"),
    }
}

/// `insert` (nodeVersion >= 1.3): groups items by their (per-item)
/// resolved collection name, runs one `insertMany` per group, then
/// restores original item order (matching the reference's final
/// `returnData.sort(...)` after the per-collection phase).
async fn run_insert(db: &Database, plans: Vec<(usize, OpPlan)>, continue_on_fail: bool) -> NodeResult<Outcome> {
    let mut groups: Vec<(String, Vec<(usize, Document)>)> = Vec::new();
    for (idx, plan) in plans {
        let OpPlan::Insert { collection, document } = plan else { unreachable!() };
        match groups.iter_mut().find(|(c, _)| *c == collection) {
            Some((_, items)) => items.push((idx, document)),
            None => groups.push((collection, vec![(idx, document)])),
        }
    }

    let mut items = Vec::new();
    let mut errors = Vec::new();
    for (collection, group_items) in groups {
        let coll = db.collection::<Document>(&collection);
        let docs: Vec<Document> = group_items.iter().map(|(_, d)| d.clone()).collect();
        match coll.insert_many(docs).await {
            Ok(res) => {
                for (pos, (idx, document)) in group_items.into_iter().enumerate() {
                    let id = res.inserted_ids.get(&pos).cloned().unwrap_or(Bson::Null);
                    let mut out = document;
                    out.insert("id", id);
                    items.push(doc_to_item(out, idx));
                }
            }
            Err(e) => {
                let ne = mongo_error(e);
                if continue_on_fail {
                    for (idx, _) in group_items {
                        errors.push((idx, ne.clone()));
                    }
                } else {
                    return Err(ne);
                }
            }
        }
    }
    items.sort_by_key(|it| it.paired_item.as_ref().and_then(|p| p.get("item")).and_then(Value::as_u64).unwrap_or(0));
    Ok(Outcome { items, errors })
}

// ---- Document -> Item (deep ObjectId/Date stringification) --------------------

/// `serializeMongoItems` (nodeVersion >= 1.4, minus binary routing -- see
/// module doc): deep-converts `ObjectId`/`DateTime`/`Decimal128`/`Binary`
/// values anywhere in the document into JSON-safe strings (hex, RFC3339,
/// decimal text, base64), rather than n8n's `{"$oid": ...}`-style
/// canonical/relaxed Extended JSON.
fn doc_to_item(doc: Document, idx: usize) -> Item {
    let json = bson_to_json(Bson::Document(doc));
    let map = match json {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    Item::new(map).paired(idx)
}

fn bson_to_json(b: Bson) -> Value {
    match b {
        Bson::Double(f) => serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null),
        Bson::String(s) => Value::String(s),
        Bson::Array(a) => Value::Array(a.into_iter().map(bson_to_json).collect()),
        Bson::Document(d) => Value::Object(d.into_iter().map(|(k, v)| (k, bson_to_json(v))).collect()),
        Bson::Boolean(b) => Value::Bool(b),
        Bson::Null => Value::Null,
        Bson::RegularExpression(r) => Value::String(r.pattern),
        Bson::Int32(i) => Value::Number(i.into()),
        Bson::Int64(i) => Value::Number(i.into()),
        Bson::Timestamp(t) => Value::String(format!("{}:{}", t.time, t.increment)),
        Bson::Binary(bin) => Value::String(base64_encode(&bin.bytes)),
        Bson::ObjectId(oid) => Value::String(oid.to_hex()),
        Bson::DateTime(dt) => Value::String(dt.try_to_rfc3339_string().unwrap_or_else(|_| dt.to_string())),
        Bson::Decimal128(d) => Value::String(d.to_string()),
        Bson::Undefined | Bson::MinKey | Bson::MaxKey => Value::Null,
        Bson::JavaScriptCode(s) => Value::String(s),
        Bson::JavaScriptCodeWithScope(s) => Value::String(s.code),
        Bson::Symbol(s) => Value::String(s),
        Bson::DbPointer(_) => Value::Null,
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
