//! Postgres (spec §6.6): `executeQuery`, `insert`, `update`, `upsert`,
//! `deleteTable` (delete/truncate/drop) and `select`, against the
//! `postgres` credential. Faithful to n8n's Postgres node v2
//! (`Postgres/v2/actions/database/*.operation.js`), typeVersions 2.x as
//! the n8n 2.35.7 editor creates them (2.5/2.6): the `columns`
//! resourceMapper for column mapping (not the legacy `dataMode`/
//! `valuesToSend`/`columnToMatchOn` shape from versions < 2.2).
//!
//! Query text is built the way pg-promise (which n8n's node uses) itself
//! works: parameter values are escaped and spliced into the SQL text as
//! literals, and the resulting string is sent as a single statement (see
//! `value_literal`, `substitute_placeholders`). This sidesteps sqlx's
//! extended-protocol parameter type checking (which requires every bind's
//! Rust type to match the type Postgres infers for that placeholder --
//! unworkable when column types are only known at runtime) and is
//! executed via `sqlx::raw_sql`, matching pg-promise's own text-protocol
//! formatting model. Identifiers are always quoted (`quote_ident`) and
//! string values always escaped (`sql_string_literal`), so this is not an
//! injection risk: user-supplied text always ends up as a properly
//! quoted/escaped literal, never spliced in raw.
//!
//! Generic pieces (identifier quoting, literal escaping, `where`/`sort`
//! parameter parsing) live in `sql_common.rs` for the MySQL node to reuse
//! later.

use super::sql_common::{get_sort_rules, get_where_clauses, is_select_query, quote_ident, rl_value, string_list, value_to_text, IdentQuote, WhereClause};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use regex::Regex;
use serde_json::{json, Map, Value};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgRow, PgSslMode};
use sqlx::{Column, PgPool, Row, TypeInfo, ValueRef};
use std::time::Duration;

const Q: IdentQuote = IdentQuote::Double;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(Postgres)]
}

struct Postgres;

#[async_trait::async_trait]
impl NodeType for Postgres {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.postgres"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input_len = if ctx.input().is_empty() { 1 } else { ctx.input().len() };
        let (_, cred) = ctx.credentials("postgres").await?;
        let node_version = ctx.node.type_version;

        let connection_timeout = ctx.param_f64("options.connectionTimeout", 0, 30.0)? as u64;
        let query_batching = ctx.param_str("options.queryBatching", 0, "single")?;
        let large_numbers = ctx.param_str("options.largeNumbersOutput", 0, "text")?;
        let operation = ctx.param_str("operation", 0, "insert")?;

        let pool = connect(&cred, connection_timeout).await?;

        let mut schema_cache: Option<(String, String, Vec<ColumnInfo>)> = None;
        let mut built: Vec<(usize, String)> = Vec::with_capacity(input_len);
        for i in 0..input_len {
            let sql = build_item_sql(ctx, &pool, &operation, node_version, &mut schema_cache, i).await;
            match sql {
                Ok(sql) => built.push((i, sql)),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e, i),
                Err(e) => {
                    pool.close().await;
                    return Err(e);
                }
            }
        }

        let batch = run_batch(&pool, &built, &query_batching, &large_numbers, ctx.continue_on_fail()).await;
        pool.close().await;
        let batch = batch?;
        for (idx, e) in batch.errors {
            ctx.push_error_item(&e, idx);
        }
        Ok(vec![batch.items])
    }
}

/// Result of running a built batch of queries: successful items, plus any
/// per-item errors collected under `continueOnFail` (the caller turns
/// these into error items via `ExecCtx::push_error_item`).
///
/// Kept as a plain (non-trait) `async fn`: sqlx's `Executor` bound needs a
/// concrete lifetime that `#[async_trait]`'s boxed-future transform can't
/// always satisfy ("implementation of `Executor` is not general enough")
/// when the query calls sit directly inside a `NodeType::execute` body, so
/// the transaction/pool work lives here instead and `execute` just awaits
/// this function's future.
struct BatchResult {
    items: Vec<Item>,
    errors: Vec<(usize, NodeError)>,
}

async fn run_batch(pool: &PgPool, built: &[(usize, String)], batching: &str, large_numbers: &str, continue_on_fail: bool) -> NodeResult<BatchResult> {
    let mut result = BatchResult { items: Vec::new(), errors: Vec::new() };
    if batching == "independently" {
        for (idx, sql) in built {
            match sqlx::raw_sql(sql).fetch_all(pool).await {
                Ok(rows) => push_rows(&mut result.items, rows, sql, *idx, large_numbers),
                Err(e) => {
                    let e = friendly_query_error(e, sql).at(*idx);
                    if continue_on_fail {
                        result.errors.push((*idx, e));
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        return Ok(result);
    }

    // "single" and "transaction" both run the whole batch inside one
    // database transaction (matching pg-promise's own db.multi, which
    // sends everything as one implicit-transaction simple query): a
    // per-item SAVEPOINT lets continueOnFail skip just the failing item
    // without poisoning the rest of the transaction.
    let mut tx = pool.begin().await.map_err(|e| NodeError::new(e.to_string()))?;
    for (idx, sql) in built {
        let _ = sqlx::query("SAVEPOINT pg_node_sp").execute(&mut *tx).await;
        match sqlx::raw_sql(sql).fetch_all(&mut *tx).await {
            Ok(rows) => {
                let _ = sqlx::query("RELEASE SAVEPOINT pg_node_sp").execute(&mut *tx).await;
                push_rows(&mut result.items, rows, sql, *idx, large_numbers);
            }
            Err(e) => {
                let _ = sqlx::query("ROLLBACK TO SAVEPOINT pg_node_sp").execute(&mut *tx).await;
                let e = friendly_query_error(e, sql).at(*idx);
                if continue_on_fail {
                    result.errors.push((*idx, e));
                } else {
                    let _ = tx.rollback().await;
                    return Err(e);
                }
            }
        }
    }
    tx.commit().await.map_err(|e| NodeError::new(e.to_string()))?;
    Ok(result)
}

// ---- connection -------------------------------------------------------------

fn cred_str<'a>(cred: &'a Value, key: &str, default: &'a str) -> String {
    cred.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(default).to_string()
}

fn cred_u64(cred: &Value, key: &str, default: u64) -> u64 {
    cred.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(default)
}

fn cred_bool(cred: &Value, key: &str, default: bool) -> bool {
    cred.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(default)
}

async fn connect(cred: &Value, connect_timeout_secs: u64) -> NodeResult<PgPool> {
    let host = cred_str(cred, "host", "localhost");
    let port = cred_u64(cred, "port", 5432) as u16;
    let database = cred_str(cred, "database", "postgres");
    let user = cred_str(cred, "user", "postgres");
    let password = cred_str(cred, "password", "");
    let allow_unauthorized = cred_bool(cred, "allowUnauthorizedCerts", false);
    let ssl_mode = if allow_unauthorized {
        PgSslMode::Require
    } else {
        match cred.get("ssl").and_then(Value::as_str) {
            Some("allow") => PgSslMode::Allow,
            Some("require") => PgSslMode::Require,
            _ => PgSslMode::Disable,
        }
    };
    let opts = PgConnectOptions::new().host(&host).port(port).database(&database).username(&user).password(&password).ssl_mode(ssl_mode);
    PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(connect_timeout_secs.max(1)))
        .connect_with(opts)
        .await
        .map_err(|e| connection_error(&e))
}

/// Maps a connection-time `sqlx::Error` onto n8n's short, password-free
/// wording (`parsePostgresError`/`postgresConnectionTest` both special-case
/// ECONNREFUSED/ENOTFOUND/ETIMEDOUT). The underlying error text never
/// contains the password (it isn't part of libpq's connection-refused /
/// DNS-failure / timeout messages), so nothing further needs to be redacted.
fn connection_error(e: &sqlx::Error) -> NodeError {
    let raw = e.to_string();
    let lower = raw.to_lowercase();
    let message = if lower.contains("connection refused") {
        "Connection refused".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "Connection timed out".to_string()
    } else if lower.contains("failed to lookup address")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname provided")
        || lower.contains("no address associated with hostname")
    {
        "Host not found, please check your host name".to_string()
    } else if lower.contains("password authentication failed") {
        "Password authentication failed".to_string()
    } else {
        raw
    };
    NodeError::new(message)
}

/// Maps a query-execution `sqlx::Error` onto n8n's `parsePostgresError`
/// shape: the driver's own message, with `detail`/`hint` (or the failed
/// query, as a fallback) as the description.
fn friendly_query_error(e: sqlx::Error, sql: &str) -> NodeError {
    if let sqlx::Error::Database(db_err) = &e {
        let message = db_err.message().to_string();
        let pg = db_err.try_downcast_ref::<sqlx::postgres::PgDatabaseError>();
        let description = pg.and_then(|p| p.detail().or_else(|| p.hint())).map(str::to_string).unwrap_or_else(|| format!("Failed query: {sql}"));
        return NodeError::new(message).describe(description);
    }
    connection_error(&e).describe(format!("Failed query: {sql}"))
}

// ---- table schema -------------------------------------------------------------

#[derive(Debug, Clone)]
struct ColumnInfo {
    column_name: String,
    data_type: String,
    is_nullable: String,
}

fn text_col(row: &PgRow, i: usize) -> NodeResult<String> {
    let vref = row.try_get_raw(i).map_err(|e| NodeError::new(e.to_string()))?;
    if vref.is_null() {
        return Ok(String::new());
    }
    Ok(vref.as_str().unwrap_or("").to_string())
}

async fn get_table_schema(pool: &PgPool, schema: &str, table: &str) -> NodeResult<Vec<ColumnInfo>> {
    let sql = format!(
        "SELECT column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = {} AND table_name = {}",
        super::sql_common::sql_string_literal(schema),
        super::sql_common::sql_string_literal(table)
    );
    let rows = sqlx::raw_sql(&sql).fetch_all(pool).await.map_err(|e| NodeError::new(e.to_string()))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        out.push(ColumnInfo { column_name: text_col(row, 0)?, data_type: text_col(row, 1)?, is_nullable: text_col(row, 2)? });
    }
    Ok(out)
}

async fn get_or_refresh_schema(pool: &PgPool, cache: &mut Option<(String, String, Vec<ColumnInfo>)>, schema: &str, table: &str) -> NodeResult<Vec<ColumnInfo>> {
    if let Some((s, t, info)) = cache.as_ref() {
        if s == schema && t == table {
            return Ok(info.clone());
        }
    }
    let info = get_table_schema(pool, schema, table).await?;
    *cache = Some((schema.to_string(), table.to_string(), info.clone()));
    Ok(info)
}

fn check_item_against_schema(item: &Map<String, Value>, schema: &[ColumnInfo], i: usize) -> NodeResult<()> {
    if schema.is_empty() {
        return Ok(());
    }
    for (key, value) in item {
        match schema.iter().find(|c| &c.column_name == key) {
            None => return Err(NodeError::new(format!("Column '{key}' does not exist in selected table")).at(i)),
            Some(col) => {
                if value.is_null() && col.is_nullable != "YES" {
                    return Err(NodeError::new(format!("Column '{key}' is not nullable")).at(i));
                }
            }
        }
    }
    Ok(())
}

/// n8n's `convertArraysToPostgresFormat`: an ARRAY-typed column given a
/// JSON array (or a string that parses to one) is rewritten to Postgres'
/// `{a,b,c}` array-literal text, which the INSERT/UPDATE target column
/// then implicitly casts.
fn convert_arrays(item: &mut Map<String, Value>, schema: &[ColumnInfo], i: usize) -> NodeResult<()> {
    for col in schema.iter().filter(|c| c.data_type.eq_ignore_ascii_case("ARRAY")) {
        let Some(v) = item.get(&col.column_name).cloned() else { continue };
        let arr = match &v {
            Value::Array(a) => Some(a.clone()),
            Value::String(s) => serde_json::from_str::<Value>(s).ok().and_then(|p| p.as_array().cloned()),
            _ => None,
        };
        match arr {
            Some(a) => {
                item.insert(col.column_name.clone(), Value::String(pg_array_literal(&a)));
            }
            None if v.is_null() => {
                if col.is_nullable != "YES" {
                    return Err(NodeError::new(format!("Column '{}' has to be an array", col.column_name)).at(i));
                }
            }
            None => return Err(NodeError::new(format!("Column '{}' has to be an array", col.column_name)).at(i)),
        }
    }
    Ok(())
}

fn pg_array_literal(arr: &[Value]) -> String {
    let items: Vec<String> = arr
        .iter()
        .map(|entry| match entry {
            Value::Null => "NULL".to_string(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => format!("\"{b}\""),
            Value::Object(_) | Value::Array(_) => format!("\"{}\"", serde_json::to_string(entry).unwrap_or_default().replace('\\', "\\\\").replace('"', "\\\"")),
            Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        })
        .collect();
    format!("{{{}}}", items.join(","))
}

async fn row_exists(pool: &PgPool, schema: &str, table: &str, match_values: &[(String, Value)]) -> NodeResult<bool> {
    let qs = quote_ident(schema, Q);
    let qt = quote_ident(table, Q);
    let conds: Vec<String> = match_values.iter().map(|(c, v)| format!("{} = {}", quote_ident(c, Q), value_literal(v))).collect();
    let sql = format!("SELECT EXISTS(SELECT 1 FROM {qs}.{qt} WHERE {})", conds.join(" AND "));
    let rows = sqlx::raw_sql(&sql).fetch_all(pool).await.map_err(|e| NodeError::new(e.to_string()))?;
    let Some(row) = rows.into_iter().next() else { return Ok(false) };
    Ok(text_col(&row, 0)? == "t")
}

// ---- SQL literal formatting ---------------------------------------------------

fn value_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => super::sql_common::sql_string_literal(s),
        Value::Array(_) | Value::Object(_) => super::sql_common::sql_string_literal(&serde_json::to_string(v).unwrap_or_default()),
    }
}

fn append_where(sql: &mut String, clauses: &[WhereClause], combine: &str) {
    if clauses.is_empty() {
        return;
    }
    let joiner = if combine == "OR" { " OR " } else { " AND " };
    let parts: Vec<String> = clauses
        .iter()
        .map(|c| {
            let col = quote_ident(&c.column, Q);
            if c.condition == "IS NULL" || c.condition == "IS NOT NULL" {
                format!("{col} {}", c.condition)
            } else {
                format!("{col} {} {}", c.condition, value_literal(&coerce_where_value(&c.condition, &c.value)))
            }
        })
        .collect();
    sql.push_str(" WHERE ");
    sql.push_str(&parts.join(joiner));
}

/// n8n's `addWhereClauses`: a comparison operator's value is coerced to a
/// number when it parses as one (so `price > "9"` compares numerically,
/// not lexically).
fn coerce_where_value(condition: &str, value: &Value) -> Value {
    if matches!(condition, ">" | "<" | ">=" | "<=") {
        if let Value::String(s) = value {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                if let Ok(n) = trimmed.parse::<f64>() {
                    return json!(n);
                }
            }
        }
    }
    value.clone()
}

fn append_returning(sql: &mut String, output_columns: &[String]) {
    if output_columns.is_empty() || output_columns.iter().any(|c| c == "*") {
        sql.push_str(" RETURNING *");
    } else {
        sql.push_str(" RETURNING ");
        sql.push_str(&output_columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", "));
    }
}

// ---- per-operation query builders ----------------------------------------------

async fn build_item_sql(
    ctx: &ExecCtx<'_>,
    pool: &PgPool,
    operation: &str,
    node_version: f64,
    schema_cache: &mut Option<(String, String, Vec<ColumnInfo>)>,
    i: usize,
) -> NodeResult<String> {
    match operation {
        "executeQuery" => build_execute_query(ctx, i),
        "select" => build_select(ctx, i),
        "deleteTable" => build_delete_table(ctx, i),
        "insert" => build_insert(ctx, pool, schema_cache, i).await,
        "update" => build_update(ctx, pool, schema_cache, i).await,
        "upsert" => build_upsert(ctx, pool, schema_cache, i).await,
        other => Err(NodeError::new(format!("The operation \"{other}\" is not supported!")).at(i)),
    }
}

fn resolved_item(ctx: &ExecCtx<'_>, mapping_mode: &str, columns: &Value, i: usize) -> Map<String, Value> {
    if mapping_mode == "defineBelow" {
        if let Some(Value::Object(m)) = columns.get("value") {
            return m.clone();
        }
        Map::new()
    } else {
        ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default()
    }
}

fn apply_replace_empty_strings(item: &mut Map<String, Value>, replace: bool) {
    if !replace {
        return;
    }
    for v in item.values_mut() {
        if matches!(v, Value::String(s) if s.is_empty()) {
            *v = Value::Null;
        }
    }
}

fn matching_columns_of(columns: &Value) -> Vec<String> {
    columns.get("matchingColumns").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()
}

// executeQuery -------------------------------------------------------------------

fn mustache_spans(s: &str) -> Vec<String> {
    let re = Regex::new(r"(?s)\{\{.*?\}\}").expect("static regex");
    re.find_iter(s).map(|m| m.as_str().to_string()).collect()
}

fn eval_span(ctx: &ExecCtx<'_>, span: &str, i: usize) -> NodeResult<Option<Value>> {
    let ev = ctx.evaluator()?;
    ev.set_item(i).map_err(|e| crate::n8n::node::NodeError::from(e).at(i))?;
    ev.template(span).map_err(|e| crate::n8n::node::NodeError::from(e).at(i))
}

fn build_execute_query(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let raw_query = ctx.raw_param("query").cloned().unwrap_or_else(|| Value::String(String::new()));
    let raw_query_str = value_to_text(&raw_query);
    let body = raw_query_str.strip_prefix('=').unwrap_or(&raw_query_str).to_string();
    let rendered = eval_span_template(ctx, &body, i)?;
    let mut query = rendered.map(|v| value_to_text(&v)).unwrap_or_default();

    let treat_quoted_as_text = ctx.param_bool("options.treatQueryParametersInSingleQuotesAsText", i, false)?;
    let query_replacement_raw = ctx.raw_param("options.queryReplacement").cloned();
    let has_replacement = match &query_replacement_raw {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    };

    let mut values: Vec<Value> = Vec::new();
    match query_replacement_raw {
        Some(Value::Array(arr)) => values = arr,
        Some(Value::String(s)) if !s.is_empty() => {
            let body2 = s.strip_prefix('=').unwrap_or(&s).to_string();
            let spans = mustache_spans(&body2);
            if !spans.is_empty() {
                for span in spans {
                    match eval_span(ctx, &span, i)? {
                        Some(Value::Array(arr)) => {
                            for item in arr {
                                match item {
                                    Value::Null => {}
                                    Value::Object(_) | Value::Array(_) => values.push(Value::String(serde_json::to_string(&item).unwrap_or_default())),
                                    other => values.push(other),
                                }
                            }
                        }
                        Some(other) => values.push(other),
                        None => {}
                    }
                }
            } else {
                for tok in body2.split(',').filter(|s| !s.is_empty()) {
                    values.push(Value::String(tok.trim().to_string()));
                }
            }
        }
        _ => {}
    }

    if !has_replacement || treat_quoted_as_text {
        let re = Regex::new(r"'\$([0-9]+)'").expect("static regex");
        let mut next_index = values.len() + 1;
        let mut extra: Vec<Value> = Vec::new();
        let replaced = {
            let mut out = String::new();
            let mut last = 0;
            for cap in re.captures_iter(&query.clone()) {
                let m = cap.get(0).unwrap();
                out.push_str(&query[last..m.start()]);
                out.push('$');
                out.push_str(&next_index.to_string());
                extra.push(Value::String(format!("${}", &cap[1])));
                next_index += 1;
                last = m.end();
            }
            out.push_str(&query[last..]);
            out
        };
        query = replaced;
        values.extend(extra);
    }

    substitute_placeholders(&query, &values, i)
}

fn eval_span_template(ctx: &ExecCtx<'_>, body: &str, i: usize) -> NodeResult<Option<Value>> {
    let ev = ctx.evaluator()?;
    ev.set_item(i).map_err(|e| crate::n8n::node::NodeError::from(e).at(i))?;
    ev.template(body).map_err(|e| crate::n8n::node::NodeError::from(e).at(i))
}

fn substitute_placeholders(query: &str, values: &[Value], i: usize) -> NodeResult<String> {
    let re = Regex::new(r"\$([0-9]+)").expect("static regex");
    let mut error: Option<NodeError> = None;
    let mut out = String::new();
    let mut last = 0;
    for cap in re.captures_iter(query) {
        let m = cap.get(0).unwrap();
        let n: usize = cap[1].parse().unwrap_or(0);
        out.push_str(&query[last..m.start()]);
        if n == 0 || n > values.len() {
            error.get_or_insert_with(|| {
                NodeError::new(format!("Query parameter \"${n}\" was not provided")).describe("Add it to 'Query Parameters' in the node's Options, or remove it from the query.").at(i)
            });
        } else {
            out.push_str(&value_literal(&values[n - 1]));
        }
        last = m.end();
    }
    out.push_str(&query[last..]);
    if let Some(e) = error {
        return Err(e);
    }
    Ok(out)
}

// select ---------------------------------------------------------------------

fn build_select(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let schema = rl_value(ctx, "schema", i)?;
    let table = rl_value(ctx, "table", i)?;
    let qs = quote_ident(&schema, Q);
    let qt = quote_ident(&table, Q);

    let output_columns = string_list(&ctx.param("options.outputColumns", i)?);
    let mut sql = if output_columns.is_empty() || output_columns.iter().any(|c| c == "*") {
        format!("SELECT * FROM {qs}.{qt}")
    } else {
        format!("SELECT {} FROM {qs}.{qt}", output_columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", "))
    };

    let clauses = get_where_clauses(ctx, i)?;
    let combine = ctx.param_str("combineConditions", i, "AND")?;
    append_where(&mut sql, &clauses, &combine);

    let sort_rules = get_sort_rules(ctx, i)?;
    if !sort_rules.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&sort_rules.iter().map(|r| format!("{} {}", quote_ident(&r.column, Q), r.direction)).collect::<Vec<_>>().join(", "));
    }

    let return_all = ctx.param_bool("returnAll", i, false)?;
    if !return_all {
        let limit = ctx.param_f64("limit", i, 50.0)? as i64;
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    Ok(sql)
}

// deleteTable ------------------------------------------------------------------

fn build_delete_table(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let schema = rl_value(ctx, "schema", i)?;
    let table = rl_value(ctx, "table", i)?;
    let qs = quote_ident(&schema, Q);
    let qt = quote_ident(&table, Q);
    let cascade = ctx.param_bool("options.cascade", i, false)?;
    let cmd = ctx.param_str("deleteCommand", i, "truncate")?;

    match cmd.as_str() {
        "drop" => Ok(format!("DROP TABLE IF EXISTS {qs}.{qt}{}", if cascade { " CASCADE" } else { "" })),
        "truncate" => {
            let restart = ctx.param_bool("restartSequences", i, false)?;
            Ok(format!("TRUNCATE TABLE {qs}.{qt}{}{}", if restart { " RESTART IDENTITY" } else { "" }, if cascade { " CASCADE" } else { "" }))
        }
        "delete" => {
            let clauses = get_where_clauses(ctx, i)?;
            let combine = ctx.param_str("combineConditions", i, "AND")?;
            let mut sql = format!("DELETE FROM {qs}.{qt}");
            append_where(&mut sql, &clauses, &combine);
            Ok(sql)
        }
        other => Err(NodeError::new(format!("Invalid delete command \"{other}\", only drop, delete and truncate are supported")).at(i)),
    }
}

// insert -----------------------------------------------------------------------

async fn build_insert(ctx: &ExecCtx<'_>, pool: &PgPool, schema_cache: &mut Option<(String, String, Vec<ColumnInfo>)>, i: usize) -> NodeResult<String> {
    let schema = rl_value(ctx, "schema", i)?;
    let table = rl_value(ctx, "table", i)?;
    let qs = quote_ident(&schema, Q);
    let qt = quote_ident(&table, Q);
    let table_schema = get_or_refresh_schema(pool, schema_cache, &schema, &table).await?;

    let columns = ctx.param("columns", i)?;
    let mapping_mode = columns.get("mappingMode").and_then(Value::as_str).unwrap_or("autoMapInputData");
    let mut item = resolved_item(ctx, mapping_mode, &columns, i);

    apply_replace_empty_strings(&mut item, ctx.param_bool("options.replaceEmptyStrings", i, false)?);
    convert_arrays(&mut item, &table_schema, i)?;
    check_item_against_schema(&item, &table_schema, i)?;

    let output_columns = string_list(&ctx.param("options.outputColumns", i)?);
    let skip_on_conflict = ctx.param_bool("options.skipOnConflict", i, false)?;

    let mut sql = if item.is_empty() {
        format!("INSERT INTO {qs}.{qt} DEFAULT VALUES")
    } else {
        let cols: Vec<&String> = item.keys().collect();
        let col_list = cols.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
        let val_list = cols.iter().map(|c| value_literal(&item[*c])).collect::<Vec<_>>().join(", ");
        format!("INSERT INTO {qs}.{qt}({col_list}) VALUES({val_list})")
    };
    if skip_on_conflict {
        sql.push_str(" ON CONFLICT DO NOTHING");
    }
    append_returning(&mut sql, &output_columns);
    Ok(sql)
}

// update -----------------------------------------------------------------------

async fn build_update(ctx: &ExecCtx<'_>, pool: &PgPool, schema_cache: &mut Option<(String, String, Vec<ColumnInfo>)>, i: usize) -> NodeResult<String> {
    let schema = rl_value(ctx, "schema", i)?;
    let table = rl_value(ctx, "table", i)?;
    let qs = quote_ident(&schema, Q);
    let qt = quote_ident(&table, Q);
    let table_schema = get_or_refresh_schema(pool, schema_cache, &schema, &table).await?;

    let columns = ctx.param("columns", i)?;
    let mapping_mode = columns.get("mappingMode").and_then(Value::as_str).unwrap_or("autoMapInputData");
    let matching_columns = matching_columns_of(&columns);
    if matching_columns.is_empty() {
        return Err(NodeError::new("At least one column to match on must be selected").at(i));
    }
    let mut item = resolved_item(ctx, mapping_mode, &columns, i);

    apply_replace_empty_strings(&mut item, ctx.param_bool("options.replaceEmptyStrings", i, false)?);
    convert_arrays(&mut item, &table_schema, i)?;
    let item = check_item_against_schema(&item, &table_schema, i).map(|_| item)?;

    let match_values: Vec<(String, Value)> = matching_columns.iter().map(|c| (c.clone(), item.get(c).cloned().unwrap_or(Value::Null))).collect();
    if !row_exists(pool, &schema, &table, &match_values).await? {
        let desc = match_values.iter().map(|(c, v)| format!("{c}={}", value_to_text(v))).collect::<Vec<_>>().join(", ");
        return Err(NodeError::new("The row you are trying to update doesn't exist")
            .describe(format!("No rows matching the provided values ({desc}) were found in the table \"{table}\"."))
            .at(i));
    }

    let update_columns: Vec<&String> = item.keys().filter(|k| !matching_columns.contains(k)).collect();
    if update_columns.is_empty() {
        return Err(NodeError::new("Add values to update to the input item or set the 'Data Mode' to 'Define Below' to define the values to update.").at(i));
    }
    let set_clause = update_columns.iter().map(|c| format!("{} = {}", quote_ident(c, Q), value_literal(&item[*c]))).collect::<Vec<_>>().join(", ");
    let where_clause = match_values.iter().map(|(c, v)| format!("{} = {}", quote_ident(c, Q), value_literal(v))).collect::<Vec<_>>().join(" AND ");

    let output_columns = string_list(&ctx.param("options.outputColumns", i)?);
    let mut sql = format!("UPDATE {qs}.{qt} SET {set_clause} WHERE {where_clause}");
    append_returning(&mut sql, &output_columns);
    Ok(sql)
}

// upsert -------------------------------------------------------------------------

async fn build_upsert(ctx: &ExecCtx<'_>, pool: &PgPool, schema_cache: &mut Option<(String, String, Vec<ColumnInfo>)>, i: usize) -> NodeResult<String> {
    let schema = rl_value(ctx, "schema", i)?;
    let table = rl_value(ctx, "table", i)?;
    let qs = quote_ident(&schema, Q);
    let qt = quote_ident(&table, Q);
    let table_schema = get_or_refresh_schema(pool, schema_cache, &schema, &table).await?;

    let columns = ctx.param("columns", i)?;
    let mapping_mode = columns.get("mappingMode").and_then(Value::as_str).unwrap_or("autoMapInputData");
    let matching_columns = matching_columns_of(&columns);
    if matching_columns.is_empty() {
        return Err(NodeError::new(
            "Column to match on not found in input item. Add a column to match on or set the 'Data Mode' to 'Define Below' to define the value to match on.",
        )
        .at(i));
    }
    let mut item = resolved_item(ctx, mapping_mode, &columns, i);
    apply_replace_empty_strings(&mut item, ctx.param_bool("options.replaceEmptyStrings", i, false)?);
    convert_arrays(&mut item, &table_schema, i)?;
    let item = check_item_against_schema(&item, &table_schema, i).map(|_| item)?;

    if matching_columns.iter().any(|c| item.get(c).map(Value::is_null).unwrap_or(true)) {
        return Err(NodeError::new(
            "Column to match on not found in input item. Add a column to match on or set the 'Data Mode' to 'Define Below' to define the value to match on.",
        )
        .at(i));
    }

    let update_columns: Vec<&String> = item.keys().filter(|k| !matching_columns.contains(k)).collect();
    if update_columns.is_empty() && item.len() <= matching_columns.len() {
        return Err(NodeError::new("Add values to update or insert to the input item or set the 'Data Mode' to 'Define Below' to define the values to insert or update.").at(i));
    }

    let cols: Vec<&String> = item.keys().collect();
    let col_list = cols.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
    let val_list = cols.iter().map(|c| value_literal(&item[*c])).collect::<Vec<_>>().join(", ");
    let conflict_cols = matching_columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
    let mut sql = format!("INSERT INTO {qs}.{qt}({col_list}) VALUES({val_list}) ON CONFLICT ({conflict_cols})");
    if update_columns.is_empty() {
        sql.push_str(" DO NOTHING");
    } else {
        let set_clause = update_columns.iter().map(|c| format!("{} = {}", quote_ident(c, Q), value_literal(&item[*c]))).collect::<Vec<_>>().join(", ");
        sql.push_str(&format!(" DO UPDATE SET {set_clause}"));
    }
    let output_columns = string_list(&ctx.param("options.outputColumns", i)?);
    append_returning(&mut sql, &output_columns);
    Ok(sql)
}

// ---- result rows -> items -------------------------------------------------------

fn push_rows(out: &mut Vec<Item>, rows: Vec<PgRow>, sql: &str, idx: usize, large_numbers: &str) {
    if rows.is_empty() {
        if !is_select_query(sql) {
            let mut m = Map::new();
            m.insert("success".into(), json!(true));
            out.push(Item::new(m).paired(idx));
        }
        return;
    }
    for row in rows {
        out.push(Item::new(row_to_json_map(&row, large_numbers)).paired(idx));
    }
}

fn row_to_json_map(row: &PgRow, large_numbers: &str) -> Map<String, Value> {
    let mut m = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let name = col.name().to_string();
        let value = match row.try_get_raw(idx) {
            Ok(vref) if !vref.is_null() => {
                let type_name = vref.type_info().name().to_string();
                let text = vref.as_str().unwrap_or("");
                pg_text_to_json(text, &type_name, large_numbers)
            }
            _ => Value::Null,
        };
        m.insert(name, value);
    }
    m
}

fn pg_text_to_json(text: &str, type_name: &str, large_numbers: &str) -> Value {
    match type_name {
        "INT2" | "INT4" => text.parse::<i64>().map(Value::from).unwrap_or_else(|_| Value::String(text.to_string())),
        "INT8" => {
            if large_numbers == "numbers" {
                text.parse::<i64>().map(Value::from).unwrap_or_else(|_| Value::String(text.to_string()))
            } else {
                Value::String(text.to_string())
            }
        }
        "NUMERIC" => {
            if large_numbers == "numbers" {
                text.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(text.to_string()))
            } else {
                Value::String(text.to_string())
            }
        }
        "FLOAT4" | "FLOAT8" => text.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(text.to_string())),
        "BOOL" => Value::Bool(text == "t"),
        "JSON" | "JSONB" => serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())),
        "TIMESTAMPTZ" => Value::String(format_timestamptz(text)),
        "TIMESTAMP" => Value::String(format_timestamp(text)),
        t if t.ends_with("[]") => parse_pg_array(text, &t[..t.len() - 2], large_numbers),
        _ => Value::String(text.to_string()),
    }
}

/// `2024-01-15 10:30:00[.ffffff][+TZ]` -> RFC3339 UTC
/// (`2024-01-15T10:30:00.000Z`), matching n8n's `parseDateToISO`
/// (`new Date(value).toISOString()`) for TIMESTAMPTZ columns.
fn format_timestamptz(text: &str) -> String {
    for fmt in ["%Y-%m-%d %H:%M:%S%.f%#z", "%Y-%m-%d %H:%M:%S%#z"] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(text, fmt) {
            return dt.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        }
    }
    text.to_string()
}

/// `2024-01-15 10:30:00[.ffffff]` -> `2024-01-15T10:30:00.000`.
///
/// n8n's own behaviour for TIMESTAMP (no time zone) on typeVersion < 2.7
/// is environment-dependent (pg-promise hands back a native JS `Date`,
/// which `new Date("2024-01-15 10:30:00")` interprets in the *server's
/// local* time zone before it gets JSON-serialised) -- see the plan's
/// "unverified n8n behaviour" note. r8r instead formats it as a
/// zone-less ISO string, which is deterministic and avoids guessing the
/// reference n8n process's `TZ`.
fn format_timestamp(text: &str) -> String {
    for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(text, fmt) {
            return dt.format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
        }
    }
    text.to_string()
}

/// A (single-level-recursive) parser for Postgres' array text format
/// (`{1,2,3}`, `{"a","b"}`, nested `{{1,2},{3,4}}`, `NULL` elements).
/// Covers the common cases; unusual element types (box, point, ...) fall
/// back to their raw text.
fn parse_pg_array(text: &str, elem_type: &str, large_numbers: &str) -> Value {
    let trimmed = text.trim();
    let inner = trimmed.strip_prefix('{').and_then(|s| s.strip_suffix('}')).unwrap_or(trimmed);
    if inner.is_empty() {
        return Value::Array(vec![]);
    }
    let parts = split_pg_array_elements(inner);
    Value::Array(
        parts
            .into_iter()
            .map(|p| {
                if p.eq_ignore_ascii_case("null") {
                    Value::Null
                } else if p.starts_with('{') {
                    parse_pg_array(&p, elem_type, large_numbers)
                } else if let Some(unquoted) = p.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    let unescaped = unquoted.replace("\\\"", "\"").replace("\\\\", "\\");
                    if matches!(elem_type, "TEXT" | "VARCHAR" | "CHAR" | "\"CHAR\"" | "UUID" | "JSON" | "JSONB") {
                        Value::String(unescaped)
                    } else {
                        pg_text_to_json(&unescaped, elem_type, large_numbers)
                    }
                } else {
                    pg_text_to_json(&p, elem_type, large_numbers)
                }
            })
            .collect(),
    )
}

fn split_pg_array_elements(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_quotes = false;
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(c);
            }
            '\\' if in_quotes => {
                cur.push(c);
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            '{' if !in_quotes => {
                depth += 1;
                cur.push(c);
            }
            '}' if !in_quotes => {
                depth -= 1;
                cur.push(c);
            }
            ',' if !in_quotes && depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() || !parts.is_empty() {
        parts.push(cur);
    }
    parts
}
