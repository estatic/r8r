//! MySQL (spec §6.6): `executeQuery`, `insert`, `update`, `upsert`
//! (`ON DUPLICATE KEY UPDATE`), `deleteTable` (delete/truncate/drop) and
//! `select`, against the `mySql` credential. Faithful to n8n's MySQL node
//! v2 (`MySql/v2/actions/database/*.operation.js`), typeVersions 2.x as the
//! n8n 2.35.7 editor creates them (2, 2.1 .. 2.5).
//!
//! Unlike Postgres' node (`postgres.rs`), MySQL's `insert`/`update`/
//! `upsert` operations do **not** use a resourceMapper "columns" parameter
//! -- the compiled `n8n-nodes-base` JS (pinned in the reference scratchpad;
//! checked against package.json `"version": "2.35.5"`, adjacent to the
//! target 2.35.7) shows the older `dataMode`
//! (`autoMapInputData`/`defineBelow`) + `valuesToSend` (a fixedCollection
//! of `{column, value}` rows) + `columnToMatchOn` (a single column, not
//! Postgres' `matchingColumns` array) shape. That reference JS is ground
//! truth here over the task brief's parenthetical, which appears to assume
//! Postgres' newer resourceMapper shape.
//!
//! MySQL also has no schema-lookup phase: n8n's MySQL node never fetches
//! `information_schema.columns` (no array-type coercion, no nullability
//! checks, no "column does not exist" error) -- it just builds SQL text
//! from whatever keys the input item / `valuesToSend` happen to have. This
//! means (unlike `postgres.rs`) there is no need for the
//! resolve-then-tokio::spawn split to shuttle a fetched table schema across
//! an await boundary; every operation resolves straight to a final SQL
//! string during the synchronous ctx-reading phase, and phase 2 (the
//! `tokio::spawn`ed task) is pure "run these owned `(item_index, sql)`
//! pairs" work. The split is kept anyway, mirroring `postgres.rs`, both for
//! consistency and because it's still the cleanest boundary between
//! ctx-dependent parameter/expression resolution and pool/connection work.
//!
//! Like `postgres.rs`, every value is escaped and spliced into the SQL text
//! as a literal (`value_literal`/`mysql_string_literal`) rather than bound
//! via sqlx's extended/prepared protocol -- this mirrors what n8n's node
//! itself actually does: `mysql2`'s `connection.format(query, values)`
//! (called from `configureQueryRunner` in the reference `helpers/utils.js`)
//! also just substitutes escaped literals into the query text *before*
//! sending it, for every one of MySQL node's three batching modes. So
//! literal-substitution isn't a shortcut here, it's the faithful
//! reproduction of what the real node sends over the wire.
//!
//! Batching modes (`options.queryBatching`, matching
//! `helpers/utils.js#configureQueryRunner`):
//! - `single` (default): every resolved statement runs sequentially over
//!   one connection, no explicit transaction (MySQL does not implicitly
//!   wrap a multi-statement text query in a transaction the way Postgres'
//!   simple protocol does; each statement autocommits on its own). `insert`
//!   gets an extra n8n-specific optimization in this mode only: instead of
//!   one `INSERT` per item, all items are combined into a single bulk
//!   `INSERT ... VALUES (...), (...), ...` (see `insert.operation.js`'s
//!   `BATCH_MODE.SINGLE` branch) -- built once in `build_insert_single_batch`
//!   rather than per item. On the first failing statement, r8r stops
//!   (matching the real node, which sends everything as one blob and gets
//!   back one failure for the whole thing) and reports exactly one error
//!   item (or throws), discarding any rows already read back from earlier
//!   statements in *this* call -- those earlier statements' writes already
//!   committed (autocommit), but n8n's actual multi-statement COM_QUERY
//!   would never have delivered their result rows to the caller once a
//!   later statement in the same call fails, so r8r doesn't surface them
//!   either.
//! - `independently`: one statement per item over one shared connection
//!   (matching `pool.getConnection()` once, then looping); a failing item
//!   either aborts the whole node or (continueOnFail) becomes its own error
//!   item while the rest continue.
//! - `transaction`: one statement per item inside one `BEGIN`/`COMMIT`.
//!   Unlike Postgres' node (which uses a per-item `SAVEPOINT` so
//!   continueOnFail can keep prior successful items), MySQL's reference
//!   implementation has **no savepoints** -- on any failure the whole
//!   transaction rolls back and the loop stops immediately, regardless of
//!   continueOnFail (continueOnFail there only controls whether the error
//!   is thrown or returned as a partial result alongside whatever items
//!   were already produced *before* the failing one -- their SQL was rolled
//!   back, but their already-computed output rows are still returned, just
//!   like the reference `return returnData` right after the rollback).
//!
//! Generic pieces (identifier quoting with backtick, `where`/`sort`
//! parameter parsing) are reused from `sql_common.rs`. WHERE-value numeric
//! coercion is *not* shared with Postgres: MySQL's `addWhereClauses`
//! throws when a `>`/`<`/`>=`/`<=` value doesn't parse as a number, while
//! Postgres' `coerce_where_value` silently leaves it as text -- genuinely
//! different per-dialect behaviour, so it stays local to each node.

use super::sql_common::{get_sort_rules, get_where_clauses, is_select_query, quote_ident, rl_value, string_list, value_to_text, IdentQuote};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use chrono::Timelike;
use regex::Regex;
use serde_json::{json, Map, Value};
use sqlx::mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow, MySqlSslMode};
use sqlx::{Column, MySqlConnection, Row, TypeInfo};
use std::time::Duration;

const Q: IdentQuote = IdentQuote::Backtick;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(MySql)]
}

struct MySql;

#[async_trait::async_trait]
impl NodeType for MySql {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.mySql"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input_len = if ctx.input().is_empty() { 1 } else { ctx.input().len() };
        let (_, cred) = ctx.credentials("mySql").await?;

        let operation = ctx.param_str("operation", 0, "insert")?;
        let query_batching = ctx.param_str("options.queryBatching", 0, "single")?;
        let large_numbers = ctx.param_str("options.largeNumbersOutput", 0, "text")?;
        let decimal_numbers = ctx.param_bool("options.decimalNumbers", 0, false)?;
        let detailed_output = ctx.param_bool("options.detailedOutput", 0, false)?;
        let continue_on_fail = ctx.continue_on_fail();

        // Phase 1 (synchronous, ctx-only): resolve every statement's SQL
        // text. No `.await` here, so nothing from `ExecCtx<'_>` needs to
        // cross into the spawned task below.
        let statements: Vec<(usize, String)> = if operation == "insert" && query_batching == "single" {
            match build_insert_single_batch(ctx, input_len) {
                Ok(sql) => vec![(0, sql)],
                Err(e) if continue_on_fail => {
                    ctx.push_error_item(&e, 0);
                    vec![]
                }
                Err(e) => return Err(e),
            }
        } else {
            let mut v = Vec::with_capacity(input_len);
            for i in 0..input_len {
                match resolve_item_plan(ctx, &operation, i) {
                    Ok(sql) => v.push((i, sql)),
                    Err(e) if continue_on_fail => ctx.push_error_item(&e, i),
                    Err(e) => return Err(e),
                }
            }
            v
        };

        let connect_timeout_secs = (cred_u64(&cred, "connectTimeout", 10000) / 1000).max(1);

        // Phase 2 (MySQL work only, fully owned/'static): connect and run
        // the batch in its own task -- see `postgres.rs`'s module doc for
        // why this split is kept even though it's no longer required to
        // dodge an `Executor`/HRTB compile error here (no schema lookup
        // means no ctx-borrow ever crosses an await point).
        let handle = tokio::spawn(run_all(cred, connect_timeout_secs, statements, query_batching, large_numbers, decimal_numbers, detailed_output, continue_on_fail));
        let batch = handle.await.map_err(|e| NodeError::new(format!("The MySQL task panicked: {e}")))??;

        for (idx, e) in batch.errors {
            ctx.push_error_item(&e, idx);
        }
        Ok(vec![batch.items])
    }
}

// ---- item plans (built from ctx, no pool access) -------------------------------

fn resolve_item_plan(ctx: &ExecCtx<'_>, operation: &str, i: usize) -> NodeResult<String> {
    match operation {
        "executeQuery" => build_execute_query(ctx, i),
        "select" => build_select(ctx, i),
        "deleteTable" => build_delete_table(ctx, i),
        "insert" => {
            let table = rl_value(ctx, "table", i)?;
            let mut item = resolve_insert_item(ctx, i)?;
            apply_replace_empty_strings(&mut item, ctx.param_bool("options.replaceEmptyStrings", i, false)?);
            let priority = ctx.param_str("options.priority", i, "")?;
            let ignore = ctx.param_bool("options.skipOnConflict", i, false)?;
            Ok(build_insert_sql_single_item(&item, &table, &priority, ignore))
        }
        "update" => {
            let table = rl_value(ctx, "table", i)?;
            let replace_empty = ctx.param_bool("options.replaceEmptyStrings", i, false)?;
            let (item, column_to_match_on, value_to_match_on) = resolve_update_item(ctx, i, replace_empty)?;
            Ok(build_update_sql(&item, &table, &column_to_match_on, &value_to_match_on))
        }
        "upsert" => {
            let table = rl_value(ctx, "table", i)?;
            let replace_empty = ctx.param_bool("options.replaceEmptyStrings", i, false)?;
            let (item, column_to_match_on) = resolve_upsert_item(ctx, i, replace_empty)?;
            Ok(build_upsert_sql(&item, &table, &column_to_match_on))
        }
        other => Err(NodeError::new(format!("The operation \"{other}\" is not supported!")).at(i)),
    }
}

fn values_to_send_map(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Map<String, Value>> {
    let raw = ctx.param("valuesToSend", i)?;
    let mut item = Map::new();
    if let Some(vals) = raw.get("values").and_then(Value::as_array) {
        for v in vals {
            let col = v.get("column").and_then(Value::as_str).unwrap_or_default().to_string();
            if col.is_empty() {
                continue;
            }
            let val = v.get("value").cloned().unwrap_or(Value::Null);
            item.insert(col, val);
        }
    }
    Ok(item)
}

fn resolve_insert_item(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Map<String, Value>> {
    let data_mode = ctx.param_str("dataMode", i, "autoMapInputData")?;
    if data_mode == "defineBelow" {
        values_to_send_map(ctx, i)
    } else {
        Ok(ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default())
    }
}

fn resolve_update_item(ctx: &ExecCtx<'_>, i: usize, replace_empty: bool) -> NodeResult<(Map<String, Value>, String, Value)> {
    let column_to_match_on = ctx.param_str("columnToMatchOn", i, "")?;
    let data_mode = ctx.param_str("dataMode", i, "autoMapInputData")?;
    if data_mode == "defineBelow" {
        let mut item = values_to_send_map(ctx, i)?;
        apply_replace_empty_strings(&mut item, replace_empty);
        let value_to_match_on = ctx.param("valueToMatchOn", i)?;
        Ok((item, column_to_match_on, value_to_match_on))
    } else {
        let mut item = ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default();
        apply_replace_empty_strings(&mut item, replace_empty);
        let value_to_match_on = item.get(&column_to_match_on).cloned().unwrap_or(Value::Null);
        Ok((item, column_to_match_on, value_to_match_on))
    }
}

fn resolve_upsert_item(ctx: &ExecCtx<'_>, i: usize, replace_empty: bool) -> NodeResult<(Map<String, Value>, String)> {
    let column_to_match_on = ctx.param_str("columnToMatchOn", i, "")?;
    let data_mode = ctx.param_str("dataMode", i, "autoMapInputData")?;
    let mut item = if data_mode == "defineBelow" {
        let mut item = values_to_send_map(ctx, i)?;
        let value_to_match_on = ctx.param("valueToMatchOn", i)?;
        item.insert(column_to_match_on.clone(), value_to_match_on);
        item
    } else {
        ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default()
    };
    apply_replace_empty_strings(&mut item, replace_empty);
    Ok((item, column_to_match_on))
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

// ---- insert SQL -----------------------------------------------------------------

fn build_insert_sql_single_item(item: &Map<String, Value>, table: &str, priority: &str, ignore: bool) -> String {
    let qt = quote_ident(table, Q);
    let cols: Vec<&String> = item.keys().collect();
    let escaped_cols = cols.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
    let placeholder = format!("({})", cols.iter().map(|c| value_literal(&item[*c])).collect::<Vec<_>>().join(","));
    let ignore_kw = if ignore { "IGNORE" } else { "" };
    format!("INSERT {priority} {ignore_kw} INTO {qt} ({escaped_cols}) VALUES {placeholder}")
}

/// `insert` + `queryBatching: "single"` (the default): one bulk `INSERT`
/// covering every input item, matching `insert.operation.js`'s
/// `BATCH_MODE.SINGLE` branch -- columns are the union of every item's keys
/// (first-seen order, like the reference's `[...new Set(...)]`), and a
/// missing key in a given row becomes `NULL` (matching `copyInputItems`).
fn build_insert_single_batch(ctx: &ExecCtx<'_>, n: usize) -> NodeResult<String> {
    let table = rl_value(ctx, "table", 0)?;
    let qt = quote_ident(&table, Q);
    let priority = ctx.param_str("options.priority", 0, "")?;
    let ignore = ctx.param_bool("options.skipOnConflict", 0, false)?;
    let replace_empty = ctx.param_bool("options.replaceEmptyStrings", 0, false)?;
    let data_mode = ctx.param_str("dataMode", 0, "autoMapInputData")?;

    let mut items: Vec<Map<String, Value>> = Vec::with_capacity(n);
    let mut columns: Vec<String> = Vec::new();
    for i in 0..n {
        let mut item = if data_mode == "defineBelow" { values_to_send_map(ctx, i)? } else { ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default() };
        apply_replace_empty_strings(&mut item, replace_empty);
        for k in item.keys() {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
        items.push(item);
    }
    let escaped_cols = columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
    let row_tuples: Vec<String> = items.iter().map(|item| format!("({})", columns.iter().map(|c| value_literal(item.get(c).unwrap_or(&Value::Null))).collect::<Vec<_>>().join(","))).collect();
    let ignore_kw = if ignore { "IGNORE" } else { "" };
    Ok(format!("INSERT {priority} {ignore_kw} INTO {qt} ({escaped_cols}) VALUES {}", row_tuples.join(",")))
}

// ---- update / upsert SQL ---------------------------------------------------------

fn build_update_sql(item: &Map<String, Value>, table: &str, column_to_match_on: &str, value_to_match_on: &Value) -> String {
    let qt = quote_ident(table, Q);
    let update_columns: Vec<&String> = item.keys().filter(|k| k.as_str() != column_to_match_on).collect();
    let set_clause = update_columns.iter().map(|c| format!("{} = {}", quote_ident(c, Q), value_literal(&item[*c]))).collect::<Vec<_>>().join(", ");
    let condition = format!("{} = {}", quote_ident(column_to_match_on, Q), value_literal(value_to_match_on));
    format!("UPDATE {qt} SET {set_clause} WHERE {condition}")
}

fn build_upsert_sql(item: &Map<String, Value>, table: &str, column_to_match_on: &str) -> String {
    let qt = quote_ident(table, Q);
    let columns: Vec<&String> = item.keys().collect();
    let escaped_cols = columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
    let placeholder = columns.iter().map(|c| value_literal(&item[*c])).collect::<Vec<_>>().join(",");
    let update_columns: Vec<&String> = item.keys().filter(|k| k.as_str() != column_to_match_on).collect();
    let updates = update_columns.iter().map(|c| format!("{} = {}", quote_ident(c, Q), value_literal(&item[*c]))).collect::<Vec<_>>().join(", ");
    format!("INSERT INTO {qt}({escaped_cols}) VALUES({placeholder}) ON DUPLICATE KEY UPDATE {updates}")
}

// ---- select -----------------------------------------------------------------

fn build_select(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let table = rl_value(ctx, "table", i)?;
    let qt = quote_ident(&table, Q);
    let output_columns = string_list(&ctx.param("options.outputColumns", i)?);
    let distinct = ctx.param_bool("options.selectDistinct", i, false)?;
    let select_kw = if distinct { "SELECT DISTINCT" } else { "SELECT" };
    let mut sql = if output_columns.is_empty() || output_columns.iter().any(|c| c == "*") {
        format!("{select_kw} * FROM {qt}")
    } else {
        format!("{select_kw} {} FROM {qt}", output_columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", "))
    };

    let clauses = get_where_clauses(ctx, i)?;
    let combine = ctx.param_str("combineConditions", i, "AND")?;
    append_where(&mut sql, &clauses, &combine, i)?;

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

// ---- deleteTable --------------------------------------------------------------

fn build_delete_table(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let table = rl_value(ctx, "table", i)?;
    let qt = quote_ident(&table, Q);
    let cmd = ctx.param_str("deleteCommand", i, "truncate")?;
    match cmd.as_str() {
        "drop" => Ok(format!("DROP TABLE IF EXISTS {qt}")),
        "truncate" => Ok(format!("TRUNCATE TABLE {qt}")),
        "delete" => {
            let clauses = get_where_clauses(ctx, i)?;
            let combine = ctx.param_str("combineConditions", i, "AND")?;
            let mut sql = format!("DELETE FROM {qt}");
            append_where(&mut sql, &clauses, &combine, i)?;
            Ok(sql)
        }
        _ => Err(NodeError::new("Invalid delete command, only drop, delete and truncate are supported ").at(i)),
    }
}

// ---- WHERE clause (strict numeric coercion, unlike Postgres') -----------------

/// n8n's MySQL `addWhereClauses`: for `>`/`<`/`>=`/`<=`, the value MUST
/// parse as a number or the node throws -- unlike Postgres' node, which
/// silently leaves a non-numeric value as text.
fn coerce_where_value_mysql(condition: &str, value: &Value, entry: usize, i: usize) -> NodeResult<Value> {
    if matches!(condition, ">" | "<" | ">=" | "<=") {
        let n = match value {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        };
        return match n {
            Some(n) => Ok(json!(n)),
            None => Err(NodeError::new(format!("Operator in entry {} of 'Select Rows' works with numbers, but value {} is not a number", entry + 1, value_to_text(value))).at(i)),
        };
    }
    Ok(value.clone())
}

fn append_where(sql: &mut String, clauses: &[super::sql_common::WhereClause], combine: &str, i: usize) -> NodeResult<()> {
    if clauses.is_empty() {
        return Ok(());
    }
    let joiner = if combine == "OR" { " OR " } else { " AND " };
    let mut parts = Vec::with_capacity(clauses.len());
    for (idx, c) in clauses.iter().enumerate() {
        let col = quote_ident(&c.column, Q);
        if c.condition == "IS NULL" || c.condition == "IS NOT NULL" {
            parts.push(format!("{col} {}", c.condition));
        } else {
            let val = coerce_where_value_mysql(&c.condition, &c.value, idx, i)?;
            parts.push(format!("{col} {} {}", c.condition, value_literal(&val)));
        }
    }
    sql.push_str(" WHERE ");
    sql.push_str(&parts.join(joiner));
    Ok(())
}

// ---- executeQuery ---------------------------------------------------------------

fn mustache_spans(s: &str) -> Vec<String> {
    let re = Regex::new(r"(?s)\{\{.*?\}\}").expect("static regex");
    re.find_iter(s).map(|m| m.as_str().to_string()).collect()
}

fn eval_span(ctx: &ExecCtx<'_>, span: &str, i: usize) -> NodeResult<Option<Value>> {
    let ev = ctx.evaluator()?;
    ev.set_item(i).map_err(|e| NodeError::from(e).at(i))?;
    ev.template(span).map_err(|e| NodeError::from(e).at(i))
}

fn eval_span_template(ctx: &ExecCtx<'_>, body: &str, i: usize) -> NodeResult<Option<Value>> {
    let ev = ctx.evaluator()?;
    ev.set_item(i).map_err(|e| NodeError::from(e).at(i))?;
    ev.template(body).map_err(|e| NodeError::from(e).at(i))
}

/// n8n nodeVersion >= 2.3: `preparedQuery.values.map(v => Number(v) ?
/// Number(v) : v)` -- a numeric-looking replacement string becomes a real
/// number; `"0"` and non-numeric strings stay as-is (both `Number("0")`
/// and `Number("abc")` are falsy/NaN in JS).
fn coerce_query_param_number(v: Value) -> Value {
    if let Value::String(s) = &v {
        if let Ok(n) = s.trim().parse::<f64>() {
            if n != 0.0 {
                return json!(n);
            }
        }
    }
    v
}

fn build_execute_query(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<String> {
    let raw_query = ctx.raw_param("query").cloned().unwrap_or_else(|| Value::String(String::new()));
    let raw_query_str = value_to_text(&raw_query);
    let body = raw_query_str.strip_prefix('=').unwrap_or(&raw_query_str).to_string();
    let rendered = eval_span_template(ctx, &body, i)?;
    let query = rendered.map(|v| value_to_text(&v)).unwrap_or_default();

    let query_replacement_raw = ctx.raw_param("options.queryReplacement").cloned();
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

    let values: Vec<Value> = values.into_iter().map(coerce_query_param_number).collect();
    substitute_placeholders_mysql(&query, &values, i)
}

/// Whether position `idx` in `s` sits inside an odd-quoted span of `'` or
/// `"` (n8n's `isInsideQuotes`, used by the >= 2.5 quote-aware `$N`
/// rewriter to leave `'$5'`-style literal text alone).
fn is_inside_quotes(s: &str, idx: usize) -> bool {
    let before = &s[..idx];
    let single = before.matches('\'').count();
    let double = before.matches('"').count();
    single % 2 != 0 || double % 2 != 0
}

/// Substitutes `$1`, `$2`, ... (value) and `$1:name`, `$2:name`, ...
/// (identifier, via `escapeSqlIdentifier`/`quote_ident`) placeholders,
/// skipping any match that falls inside a quoted string literal (n8n's
/// `prepareSafeQuery`, used from nodeVersion >= 2.5 -- the version r8r
/// targets unconditionally, since it's a strict improvement over the
/// pre-2.5 naive regex for any well-formed query).
fn substitute_placeholders_mysql(query: &str, values: &[Value], i: usize) -> NodeResult<String> {
    let re = Regex::new(r"\$([0-9]+)(:name)?").expect("static regex");
    let mut error: Option<NodeError> = None;
    let mut out = String::new();
    let mut last = 0;
    for cap in re.captures_iter(query) {
        let m = cap.get(0).unwrap();
        out.push_str(&query[last..m.start()]);
        last = m.end();
        if is_inside_quotes(query, m.start()) {
            out.push_str(m.as_str());
            continue;
        }
        let n: usize = cap[1].parse().unwrap_or(0);
        let is_name = cap.get(2).is_some();
        if n == 0 || n > values.len() {
            error.get_or_insert_with(|| {
                NodeError::new(format!("Parameter ${n} referenced in query but no replacement value provided at index {n}"))
                    .describe("Add it to 'Query Parameters' in the node's Options, or remove it from the query.")
                    .at(i)
            });
            continue;
        }
        if is_name {
            out.push_str(&quote_ident(&value_to_text(&values[n - 1]), Q));
        } else {
            out.push_str(&value_literal(&values[n - 1]));
        }
    }
    out.push_str(&query[last..]);
    if let Some(e) = error {
        return Err(e);
    }
    Ok(out)
}

// ---- SQL literal formatting (MySQL dialect: backslash-escaped strings) --------

/// MySQL/`mysql2`-style string literal: single-quoted, with `\0`, `\b`,
/// `\t`, `\n`, `\r`, `\x1a`, `"`, `'` and `\` backslash-escaped (mirrors
/// `mysql2`'s `SqlString.escape`, which is what `connection.format`
/// ultimately calls). Not shared with Postgres' `sql_string_literal`
/// (doubled-quote escaping, no backslash escapes) in `sql_common.rs` --
/// genuinely different dialects.
fn mysql_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\0' => out.push_str("\\0"),
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{1a}' => out.push_str("\\Z"),
            '"' => out.push_str("\\\""),
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn value_literal(v: &Value) -> String {
    match v {
        Value::Null => "NULL".to_string(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => mysql_string_literal(s),
        Value::Array(_) | Value::Object(_) => mysql_string_literal(&serde_json::to_string(v).unwrap_or_default()),
    }
}

// ---- MySQL work (owned/'static; runs inside tokio::spawn) ----------------------

struct BatchResult {
    items: Vec<Item>,
    errors: Vec<(usize, NodeError)>,
}

async fn run_all(
    cred: Value,
    connect_timeout_secs: u64,
    statements: Vec<(usize, String)>,
    batching: String,
    large_numbers: String,
    decimal_numbers: bool,
    detailed: bool,
    continue_on_fail: bool,
) -> NodeResult<BatchResult> {
    let pool = connect(&cred, connect_timeout_secs).await?;
    let result = match batching.as_str() {
        "independently" => run_independently(&pool, statements, &large_numbers, decimal_numbers, detailed, continue_on_fail).await,
        "transaction" => run_transaction(&pool, statements, &large_numbers, decimal_numbers, detailed, continue_on_fail).await,
        _ => run_single(&pool, statements, &large_numbers, decimal_numbers, detailed, continue_on_fail).await,
    };
    pool.close().await;
    result
}

/// One statement outcome: a `SELECT`-shaped result (rows), or a
/// DML/DDL-shaped result (affected-row count + last insert id, from
/// `MySqlQueryResult`, roughly `mysql2`'s `OkPacket`).
enum StmtOutcome {
    Rows(Vec<MySqlRow>),
    Ok { affected: u64, insert_id: u64 },
}

async fn run_statement_pool(pool: &MySqlPool, sql: &str) -> Result<StmtOutcome, sqlx::Error> {
    let trimmed = sql.trim().trim_end_matches(';');
    let mut conn = pool.acquire().await?;
    run_statement(&mut conn, trimmed).await
}

async fn run_statement(conn: &mut MySqlConnection, trimmed: &str) -> Result<StmtOutcome, sqlx::Error> {
    if is_select_query(trimmed) {
        let rows = sqlx::query(trimmed).fetch_all(conn).await?;
        Ok(StmtOutcome::Rows(rows))
    } else {
        let res = sqlx::query(trimmed).execute(conn).await?;
        Ok(StmtOutcome::Ok { affected: res.rows_affected(), insert_id: res.last_insert_id() })
    }
}

/// Pushes this statement's outcome as item(s) (or, with `detailed`, one
/// `{sql, data}` item). Returns whether anything was pushed -- callers use
/// this to decide whether the `{success: true}` empty-result fallback still
/// applies (n8n's `prepareOutput`).
fn push_outcome(items: &mut Vec<Item>, outcome: StmtOutcome, sql: &str, idx: usize, large_numbers: &str, decimal_numbers: bool, detailed: bool) -> bool {
    match outcome {
        StmtOutcome::Rows(rows) => {
            if detailed {
                let data: Vec<Value> = rows.iter().map(|r| Value::Object(row_to_json_map(r, large_numbers, decimal_numbers))).collect();
                let mut m = Map::new();
                m.insert("sql".into(), json!(sql));
                m.insert("data".into(), Value::Array(data));
                items.push(Item::new(m).paired(idx));
                true
            } else {
                let had = !rows.is_empty();
                for row in rows {
                    items.push(Item::new(row_to_json_map(&row, large_numbers, decimal_numbers)).paired(idx));
                }
                had
            }
        }
        StmtOutcome::Ok { affected, insert_id } => {
            if detailed {
                let mut data = Map::new();
                data.insert("affectedRows".into(), json!(affected));
                data.insert("insertId".into(), json!(insert_id));
                let mut m = Map::new();
                m.insert("sql".into(), json!(sql));
                m.insert("data".into(), Value::Object(data));
                items.push(Item::new(m).paired(idx));
                true
            } else {
                false
            }
        }
    }
}

fn maybe_push_success(items: &mut Vec<Item>, already_emitted: bool, sql: &str, idx: usize) {
    if already_emitted || is_select_query(sql) {
        return;
    }
    let mut m = Map::new();
    m.insert("success".into(), json!(true));
    items.push(Item::new(m).paired(idx));
}

// ---- batching: single -------------------------------------------------------------

async fn run_single(pool: &MySqlPool, statements: Vec<(usize, String)>, large_numbers: &str, decimal_numbers: bool, detailed: bool, continue_on_fail: bool) -> NodeResult<BatchResult> {
    if statements.is_empty() {
        return Ok(BatchResult { items: vec![], errors: vec![] });
    }
    let first_idx = statements[0].0;
    match run_single_mode(pool, &statements, large_numbers, decimal_numbers, detailed).await {
        Ok(items) => Ok(BatchResult { items, errors: vec![] }),
        Err(e) => {
            if continue_on_fail {
                Ok(BatchResult { items: vec![], errors: vec![(first_idx, e)] })
            } else {
                Err(e)
            }
        }
    }
}

async fn run_single_mode(pool: &MySqlPool, statements: &[(usize, String)], large_numbers: &str, decimal_numbers: bool, detailed: bool) -> NodeResult<Vec<Item>> {
    let mut items = Vec::new();
    let mut had_any = false;
    for (idx, sql) in statements {
        let outcome = run_statement_pool(pool, sql).await.map_err(|e| friendly_query_error(e, sql).at(*idx))?;
        let emitted = push_outcome(&mut items, outcome, sql, *idx, large_numbers, decimal_numbers, detailed);
        had_any = had_any || emitted;
    }
    if !detailed && !had_any {
        let all_select = statements.iter().all(|(_, sql)| is_select_query(sql));
        if !all_select {
            let first_idx = statements[0].0;
            let mut m = Map::new();
            m.insert("success".into(), json!(true));
            items.push(Item::new(m).paired(first_idx));
        }
    }
    Ok(items)
}

// ---- batching: independently -------------------------------------------------------

async fn run_independently(pool: &MySqlPool, statements: Vec<(usize, String)>, large_numbers: &str, decimal_numbers: bool, detailed: bool, continue_on_fail: bool) -> NodeResult<BatchResult> {
    let mut result = BatchResult { items: Vec::new(), errors: Vec::new() };
    let mut conn = pool.acquire().await.map_err(|e| connection_error(&e))?;
    for (idx, sql) in statements {
        let trimmed = sql.trim().trim_end_matches(';').to_string();
        match run_statement(&mut conn, &trimmed).await {
            Ok(outcome) => {
                let emitted = push_outcome(&mut result.items, outcome, &sql, idx, large_numbers, decimal_numbers, detailed);
                maybe_push_success(&mut result.items, emitted, &sql, idx);
            }
            Err(e) => {
                let e = friendly_query_error(e, &sql).at(idx);
                if continue_on_fail {
                    result.errors.push((idx, e));
                } else {
                    return Err(e);
                }
            }
        }
    }
    Ok(result)
}

// ---- batching: transaction -----------------------------------------------------------

async fn run_transaction(pool: &MySqlPool, statements: Vec<(usize, String)>, large_numbers: &str, decimal_numbers: bool, detailed: bool, continue_on_fail: bool) -> NodeResult<BatchResult> {
    let mut result = BatchResult { items: Vec::new(), errors: Vec::new() };
    let mut tx = pool.begin().await.map_err(|e| connection_error(&e))?;
    for (idx, sql) in statements {
        let trimmed = sql.trim().trim_end_matches(';');
        let outcome = if is_select_query(trimmed) {
            sqlx::query(trimmed).fetch_all(&mut *tx).await.map(StmtOutcome::Rows)
        } else {
            sqlx::query(trimmed).execute(&mut *tx).await.map(|r| StmtOutcome::Ok { affected: r.rows_affected(), insert_id: r.last_insert_id() })
        };
        match outcome {
            Ok(outcome) => {
                let emitted = push_outcome(&mut result.items, outcome, &sql, idx, large_numbers, decimal_numbers, detailed);
                maybe_push_success(&mut result.items, emitted, &sql, idx);
            }
            Err(e) => {
                // No savepoints (unlike Postgres' node): any failure rolls
                // back the whole transaction and stops the loop, matching
                // `helpers/utils.js`'s `BATCH_MODE.TRANSACTION` branch
                // ("Return here because we already rolled back the
                // transaction").
                let _ = tx.rollback().await;
                let e = friendly_query_error(e, &sql).at(idx);
                if continue_on_fail {
                    result.errors.push((idx, e));
                    return Ok(result);
                } else {
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

async fn connect(cred: &Value, connect_timeout_secs: u64) -> NodeResult<MySqlPool> {
    let host = cred_str(cred, "host", "localhost");
    let port = cred_u64(cred, "port", 3306) as u16;
    let database = cred_str(cred, "database", "mysql");
    let user = cred_str(cred, "user", "mysql");
    let password = cred_str(cred, "password", "");
    let ssl_enabled = cred_bool(cred, "ssl", false);

    let mut opts = MySqlConnectOptions::new().host(&host).port(port).database(&database).username(&user).password(&password);
    if ssl_enabled {
        let ca = cred.get("caCertificate").and_then(Value::as_str).filter(|s| !s.is_empty());
        let cert = cred.get("clientCertificate").and_then(Value::as_str).filter(|s| !s.is_empty());
        let key = cred.get("clientPrivateKey").and_then(Value::as_str).filter(|s| !s.is_empty());
        opts = opts.ssl_mode(if ca.is_some() { MySqlSslMode::VerifyCa } else { MySqlSslMode::Required });
        if let Some(ca) = ca {
            opts = opts.ssl_ca_from_pem(ca.as_bytes().to_vec());
        }
        if let (Some(cert), Some(key)) = (cert, key) {
            opts = opts.ssl_client_cert_from_pem(cert.as_bytes()).ssl_client_key_from_pem(key.as_bytes());
        }
    } else {
        opts = opts.ssl_mode(MySqlSslMode::Disabled);
    }

    MySqlPoolOptions::new().max_connections(5).acquire_timeout(Duration::from_secs(connect_timeout_secs.max(1))).connect_with(opts).await.map_err(|e| connection_error(&e))
}

/// n8n's `parseMySqlError` only special-cases `ECONNREFUSED` -> "Connection
/// refused"; everything else (timeouts, unknown host, bad password) passes
/// through the driver's own message unchanged. r8r mirrors that narrow
/// behaviour rather than inventing extra friendly text the reference
/// doesn't have.
fn connection_error(e: &sqlx::Error) -> NodeError {
    let raw = e.to_string();
    if raw.to_lowercase().contains("connection refused") {
        NodeError::new("Connection refused")
    } else {
        NodeError::new(raw)
    }
}

/// n8n's `parseMySqlError`: message + `sql: <query>, code: <code>`
/// description. (The reference also special-cases "You have an error in
/// your SQL syntax" to point at the offending line; not reproduced here --
/// low value relative to the raw driver message, which already names the
/// syntax problem.)
fn friendly_query_error(e: sqlx::Error, sql: &str) -> NodeError {
    if let sqlx::Error::Database(db_err) = &e {
        let message = db_err.message().to_string();
        let code = db_err.try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>().map(|m| m.number().to_string()).unwrap_or_default();
        return NodeError::new(message).describe(format!("sql: {sql}, code: {code}"));
    }
    connection_error(&e).describe(format!("Failed query: {sql}"))
}

// ---- result rows -> items -------------------------------------------------------

fn bigint_json(v: Option<i64>, large_numbers: &str) -> Value {
    match v {
        None => Value::Null,
        Some(n) if large_numbers == "numbers" => json!(n),
        Some(n) => Value::String(n.to_string()),
    }
}

fn bigint_json_u64(v: Option<u64>, large_numbers: &str) -> Value {
    match v {
        None => Value::Null,
        Some(n) if large_numbers == "numbers" => json!(n),
        Some(n) => Value::String(n.to_string()),
    }
}

/// Unlike Postgres' node (one `largeNumbersOutput` option covering both
/// `BIGINT` and `NUMERIC`), MySQL's node has two separate options:
/// `largeNumbersOutput` -> `bigNumberStrings` (BIGINT only) and
/// `decimalNumbers` -> `decimalNumbers` (DECIMAL only) -- see
/// `transport/index.js#createPool`. r8r mirrors that split.
fn numeric_json(v: Option<sqlx::types::Decimal>, decimal_numbers: bool) -> Value {
    match v {
        None => Value::Null,
        Some(d) if decimal_numbers => d.to_string().parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or_else(|| Value::String(d.to_string())),
        Some(d) => Value::String(d.to_string()),
    }
}

/// n8n's transport sets `dateStrings: true` for nodeVersion >= 2.1 (which
/// covers every version this node targets), so `mysql2` hands back
/// DATE/DATETIME/TIMESTAMP as the raw wire string rather than a parsed JS
/// `Date` -- i.e. MySQL's own `YYYY-MM-DD HH:MM:SS[.ffffff]` text, no `T`
/// separator and no timezone conversion. r8r reproduces that string shape
/// directly instead of the Postgres node's ISO-8601 formatting.
fn mysql_datetime_string(d: chrono::NaiveDateTime) -> String {
    if d.nanosecond() == 0 {
        d.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        d.format("%Y-%m-%d %H:%M:%S%.f").to_string()
    }
}

fn decode_column(row: &MySqlRow, idx: usize, type_name: &str, large_numbers: &str, decimal_numbers: bool) -> Value {
    macro_rules! get {
        ($t:ty) => {
            row.try_get::<Option<$t>, _>(idx).ok().flatten()
        };
    }
    match type_name {
        "BOOLEAN" => get!(bool).map(Value::Bool).unwrap_or(Value::Null),
        "TINYINT" => get!(i8).map(|v| json!(v)).unwrap_or(Value::Null),
        "TINYINT UNSIGNED" => get!(u8).map(|v| json!(v)).unwrap_or(Value::Null),
        "SMALLINT" => get!(i16).map(|v| json!(v)).unwrap_or(Value::Null),
        "SMALLINT UNSIGNED" => get!(u16).map(|v| json!(v)).unwrap_or(Value::Null),
        "MEDIUMINT" | "INT" => get!(i32).map(|v| json!(v)).unwrap_or(Value::Null),
        "MEDIUMINT UNSIGNED" | "INT UNSIGNED" => get!(u32).map(|v| json!(v)).unwrap_or(Value::Null),
        "BIGINT" => bigint_json(get!(i64), large_numbers),
        "BIGINT UNSIGNED" => bigint_json_u64(get!(u64), large_numbers),
        "FLOAT" => get!(f32).and_then(|v| serde_json::Number::from_f64(v as f64)).map(Value::Number).unwrap_or(Value::Null),
        "DOUBLE" => get!(f64).and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(Value::Null),
        "DECIMAL" => numeric_json(get!(sqlx::types::Decimal), decimal_numbers),
        "JSON" => get!(Value).unwrap_or(Value::Null),
        "DATE" => get!(chrono::NaiveDate).map(|d| Value::String(d.to_string())).unwrap_or(Value::Null),
        "TIME" => get!(chrono::NaiveTime).map(|t| Value::String(t.format("%H:%M:%S").to_string())).unwrap_or(Value::Null),
        "DATETIME" | "TIMESTAMP" => get!(chrono::NaiveDateTime).map(mysql_datetime_string).map(Value::String).unwrap_or(Value::Null),
        "YEAR" => get!(i32).map(|v| json!(v)).unwrap_or(Value::Null),
        // Binary/blob-family columns: documented gap (see final report) --
        // real n8n hands back a Node `Buffer` here (JSON-serialised as
        // `{"type":"Buffer","data":[...]}`); r8r uses a simpler hex string.
        "BIT" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BINARY" | "VARBINARY" => get!(Vec<u8>).map(|b| Value::String(format!("\\x{}", hex::encode(b)))).unwrap_or(Value::Null),
        // CHAR/VARCHAR/TEXT family/ENUM/SET/GEOMETRY and anything else
        // String-compatible.
        _ => get!(String).map(Value::String).unwrap_or(Value::Null),
    }
}

fn row_to_json_map(row: &MySqlRow, large_numbers: &str, decimal_numbers: bool) -> Map<String, Value> {
    let mut m = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        let type_name = col.type_info().name().to_string();
        m.insert(col.name().to_string(), decode_column(row, idx, &type_name, large_numbers, decimal_numbers));
    }
    m
}
