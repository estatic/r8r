//! Microsoft SQL (spec §6.6): `executeQuery`, `insert`, `update`, `delete`
//! against the `microsoftSql` credential. Faithful to n8n's Microsoft SQL
//! node (`Microsoft/Sql/MicrosoftSql.node.js` + `GenericFunctions.js`),
//! `version: [1, 1.1, 1.2]` -- the n8n 2.35.7 editor creates the newest,
//! `1.2`, which is the only shape r8r implements (see "Not implemented"
//! below).
//!
//! This is an older, simpler node than Postgres'/MySQL's v2 actions: no
//! resourceMapper, no `dataMode`/`valuesToSend`, no batching-mode option.
//! `table`/`columns`/`updateKey`/`deleteKey` are plain `string` parameters
//! (not resource locators), and `insert`/`update` read their row data
//! straight from each input item's `json` at the named columns.
//!
//! Uses the `tiberius` crate (pure-Rust TDS over a `tokio` `TcpStream`,
//! `rustls` for TLS) instead of `sqlx`, since `sqlx` has no MSSQL driver.
//! Query/row values are bound as real typed parameters (`tiberius::Query`'s
//! `@P1, @P2, ...` RPC parameters), matching what the real node does via
//! `request.input(name, value)` -- unlike `postgres.rs`/`mysql.rs`, which
//! splice escaped literals into the SQL text. Column reads are decoded from
//! `tiberius`'s `ColumnData` enum (its wire-level payload variant, e.g.
//! `ColumnData::I16` for `smallint`) rather than `ColumnType` metadata:
//! metadata alone can't tell `tinyint` from `bigint` for a *nullable*
//! column (both report as the generic `Intn` `ColumnType`), but the actual
//! decoded `ColumnData` variant always reflects the real declared width,
//! nullable or not -- see the crate's `tds/codec/column_data/int.rs`
//! decoder, which sizes the Rust variant from the wire length byte that
//! mirrors the column's declared type, not the value's magnitude.
//!
//! No `tokio::spawn` split (unlike `postgres.rs`/`mysql.rs`): that split
//! exists there to dodge an `sqlx`-specific `Send`/`Executor` HRTB issue.
//! `tiberius`'s `Client` has no such constraint, and nothing here needs to
//! outlive the `execute()` call, so parameter resolution (`ctx`-borrowing)
//! and the database work are simply interleaved in one async function.
//!
//! ## continueOnFail: three different shapes, faithfully reproduced
//!
//! The reference `execute()` has three *different* error-handling shapes
//! depending on where the failure happens -- all reproduced here exactly
//! rather than normalized to one behaviour:
//!
//! 1. **Connection failure** (`pool.connect()` throws): `continueOnFail`
//!    returns a single `{error: message}` item (not one per input item).
//!    Reproduced as one `ctx.push_error_item` + an empty primary output.
//! 2. **`executeQuery`** (nodeVersion >= 1.1, i.e. always, since r8r only
//!    targets 1.2): each item is its own try/catch in a loop --
//!    `continueOnFail` turns a failing item into its own `{error}` item and
//!    the loop continues to the next item. Reproduced per-item via
//!    `ctx.push_error_item`.
//! 3. **`insert`/`update`/`delete`**: the whole operation (parameter
//!    resolution *and* every query) is one `try`, and its `catch` sets
//!    `responseData = items` but never recomputes `returnData` from it --
//!    `returnData` keeps its initial value of `[]`. This is a genuine n8n
//!    bug (dead code), not a documented behaviour, but it's what the
//!    pinned reference does: on `continueOnFail`, any failure in
//!    `insert`/`update`/`delete` makes the node succeed with **zero**
//!    output items, not an error item and not the echoed input. Reproduced
//!    as `Ok(vec![vec![]])` with nothing pushed to `ctx.push_error_item`.
//!
//! ## Not implemented
//!
//! - `executeQuery` on `nodeVersion < 1.1` (exactly `1.0`): a different,
//!   simpler single-batch code path (no `queryReplacement`, pairedItem
//!   spans every input item). Out of scope: r8r's default typeVersion for
//!   this node is `1.2`, the version the pinned 2.35.7 editor creates.
//! - `nodeVersion >= 1.2`'s binary-column routing (`routeBinaryProperties`,
//!   which moves `varbinary`/`image` columns into the item's `binary`
//!   output instead of `json`). `varbinary`/`image`/`binary` columns stay
//!   in `json` here as a `\x`-prefixed hex string -- the same documented
//!   simplification `mysql.rs`'s `decode_column` uses for BLOB columns
//!   (real n8n would hand back a Node `Buffer`, JSON-serialised as
//!   `{"type":"Buffer","data":[...]}`).
//! - Credential `domain` (Windows/NTLM auth): `tiberius`'s `AuthMethod::
//!   windows` needs the `winauth` (Windows-only) or `sspi-rs` (extra
//!   system deps) feature, neither enabled. The field is accepted and
//!   stored but r8r always authenticates as `AuthMethod::sql_server`.
//! - Credential `tdsVersion`: `tiberius` negotiates the TDS version itself
//!   (gated by the compile-time `tds73` feature, which r8r enables); the
//!   credential's explicit version choice has no equivalent knob and is
//!   accepted but unused.
//! - `date`/`time` JSON string formatting mirrors node-mssql's documented
//!   `Date`-object-then-`toISOString()` behaviour (full
//!   `YYYY-MM-DDTHH:MM:SS.sssZ`, `time` anchored at `1970-01-01`) rather
//!   than a stripped-down date/time-only string, but this was not checked
//!   against a live n8n run against this exact SQL Server image --
//!   flagged as a best-effort reproduction in the final report.

use super::sql_common::{is_select_query, quote_ident, IdentQuote};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::types::{Item, NodeOutput};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::time::Duration;
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel, Query, Row};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

const Q: IdentQuote = IdentQuote::Bracket;

type MssqlClient = Client<Compat<TcpStream>>;

pub fn all() -> Vec<Box<dyn NodeType>> {
    vec![Box::new(MicrosoftSql)]
}

struct MicrosoftSql;

#[async_trait::async_trait]
impl NodeType for MicrosoftSql {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.microsoftSql"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let input_len = if ctx.input().is_empty() { 1 } else { ctx.input().len() };
        let (_, cred) = ctx.credentials("microsoftSql").await?;
        let continue_on_fail = ctx.continue_on_fail();

        let mut client = match connect(&cred).await {
            Ok(c) => c,
            Err(e) => {
                if continue_on_fail {
                    ctx.push_error_item(&e, 0);
                    return Ok(vec![vec![]]);
                }
                return Err(e);
            }
        };

        let operation = ctx.param_str("operation", 0, "insert")?;

        match operation.as_str() {
            "executeQuery" => {
                let mut items = Vec::new();
                for i in 0..input_len {
                    let plan = match resolve_execute_query(ctx, i) {
                        Ok(p) => p,
                        Err(e) => {
                            if continue_on_fail {
                                ctx.push_error_item(&e, i);
                                continue;
                            }
                            return Err(e);
                        }
                    };
                    match run_execute_query_item(&mut client, plan, i).await {
                        Ok(mut its) => items.append(&mut its),
                        Err(e) => {
                            if continue_on_fail {
                                ctx.push_error_item(&e, i);
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }
                Ok(vec![items])
            }
            "insert" => match do_insert(ctx, &mut client, input_len).await {
                Ok(()) => Ok(vec![echo_items(ctx, input_len)]),
                Err(_) if continue_on_fail => Ok(vec![vec![]]),
                Err(e) => Err(e),
            },
            "update" => match do_update(ctx, &mut client, input_len).await {
                Ok(()) => Ok(vec![echo_items(ctx, input_len)]),
                Err(_) if continue_on_fail => Ok(vec![vec![]]),
                Err(e) => Err(e),
            },
            "delete" => match do_delete(ctx, &mut client, input_len).await {
                Ok(total) => {
                    let mut m = Map::new();
                    m.insert("rowsAffected".into(), json!(total));
                    Ok(vec![vec![Item::new(m).paired(0)]])
                }
                Err(_) if continue_on_fail => Ok(vec![vec![]]),
                Err(e) => Err(e),
            },
            other => Err(NodeError::new(format!("The operation \"{other}\" is not supported!"))),
        }
    }
}

fn echo_items(ctx: &ExecCtx<'_>, input_len: usize) -> Vec<Item> {
    (0..input_len).map(|i| Item::new(ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default()).paired(i)).collect()
}

// ---- executeQuery ----------------------------------------------------------------

struct QueryPlan {
    sql: String,
    values: Vec<Value>,
}

fn eval_template(ctx: &ExecCtx<'_>, body: &str, i: usize) -> NodeResult<String> {
    let ev = ctx.evaluator()?;
    ev.set_item(i).map_err(|e| NodeError::from(e).at(i))?;
    let rendered = ev.template(body).map_err(|e| NodeError::from(e).at(i))?;
    Ok(rendered.map(|v| super::sql_common::value_to_text(&v)).unwrap_or_default())
}

/// `options.queryReplacement`: a plain comma-separated string (split and
/// trimmed), or -- when the field carries an `=`-prefixed expression that
/// evaluates to a non-string -- an array used as-is, or a single scalar
/// wrapped into a one-element array. Mirrors the reference's
/// `queryReplacement` handling in `execute()`.
fn resolve_query_replacement(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Vec<Value>> {
    let raw = ctx.param("options.queryReplacement", i)?;
    Ok(match raw {
        Value::String(s) if !s.is_empty() => s.split(',').map(|t| Value::String(t.trim().to_string())).collect(),
        Value::String(_) => vec![],
        Value::Array(arr) => arr,
        Value::Null => vec![],
        other => vec![other],
    })
}

/// Replaces `$1`, `$2`, ... (1-based) with `@P1`, `@P2`, ... for every `n`
/// in `1..=max`; a `$n` outside that range is left as literal text (the
/// reference only ever replaces up to `queryValues.length`, so an
/// out-of-range reference reaches the server as-is and fails there, not as
/// a friendly n8n validation error). All `max` values are always bound in
/// order via `bind_value`, regardless of whether the text references them
/// -- matching the reference's `request.input('p'+i, ...)` loop, which
/// runs for every value independent of which placeholders actually appear.
fn substitute_dollar_placeholders(query: &str, max: usize) -> String {
    let re = Regex::new(r"\$(\d+)").expect("static regex");
    re.replace_all(query, |caps: &regex::Captures| {
        let n: usize = caps[1].parse().unwrap_or(0);
        if n >= 1 && n <= max {
            format!("@P{n}")
        } else {
            caps[0].to_string()
        }
    })
    .into_owned()
}

fn resolve_execute_query(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<QueryPlan> {
    let raw = ctx.raw_param("query").cloned().unwrap_or_else(|| Value::String(String::new()));
    let raw_str = super::sql_common::value_to_text(&raw);
    let body = raw_str.strip_prefix('=').unwrap_or(&raw_str).to_string();
    let query = eval_template(ctx, &body, i)?;

    let values = resolve_query_replacement(ctx, i)?;
    let sql = substitute_dollar_placeholders(&query, values.len());
    Ok(QueryPlan { sql, values })
}

fn bind_value(q: &mut Query<'static>, v: &Value) {
    match v {
        Value::Null => q.bind(Option::<String>::None),
        Value::Bool(b) => q.bind(*b),
        Value::String(s) => q.bind(s.clone()),
        Value::Array(_) | Value::Object(_) => q.bind(serde_json::to_string(v).unwrap_or_default()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
                    q.bind(i as i32);
                } else {
                    q.bind(i);
                }
            } else if let Some(f) = n.as_f64() {
                q.bind(f);
            } else {
                q.bind(n.to_string());
            }
        }
    }
}

async fn run_execute_query_item(client: &mut MssqlClient, plan: QueryPlan, i: usize) -> NodeResult<Vec<Item>> {
    let trimmed = plan.sql.trim().to_string();
    let is_select = is_select_query(&trimmed);

    let mut q = Query::new(plan.sql.clone());
    for v in &plan.values {
        bind_value(&mut q, v);
    }

    if is_select {
        let results = q.query(client).await.map_err(|e| friendly_query_error(e, &plan.sql).at(i))?.into_results().await.map_err(|e| friendly_query_error(e, &plan.sql).at(i))?;
        if results.is_empty() {
            let mut m = Map::new();
            m.insert("message".into(), json!("Query executed successfully, but no rows were affected"));
            return Ok(vec![Item::new(m).paired(i)]);
        }
        let rows: Vec<Row> = if results.len() > 1 { results.into_iter().flatten().collect() } else { results.into_iter().next().unwrap_or_default() };
        Ok(rows.iter().map(|r| Item::new(row_to_json_map(r)).paired(i)).collect())
    } else {
        let res = q.execute(client).await.map_err(|e| friendly_query_error(e, &plan.sql).at(i))?;
        let affected = res.rows_affected();
        if affected.is_empty() {
            let mut m = Map::new();
            m.insert("message".into(), json!("Query executed successfully, but no rows were affected"));
            Ok(vec![Item::new(m).paired(i)])
        } else {
            Ok(affected
                .iter()
                .enumerate()
                .map(|(idx, n)| {
                    let mut m = Map::new();
                    m.insert("message".into(), json!(format!("Query {} executed successfully", idx + 1)));
                    m.insert("rowsAffected".into(), json!(n));
                    Item::new(m).paired(i)
                })
                .collect())
        }
    }
}

// ---- insert -----------------------------------------------------------------------

struct InsertRow {
    table: String,
    columns_key: String,
    columns: Vec<String>,
    row: Map<String, Value>,
}

fn resolve_insert_row(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<InsertRow> {
    let table = ctx.param_str("table", i, "")?;
    let columns_key = ctx.param_str("columns", i, "")?;
    let columns: Vec<String> = columns_key.split(',').map(|s| s.trim().to_string()).collect();
    let item_json = ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default();
    let mut row = Map::new();
    for c in &columns {
        row.insert(c.clone(), item_json.get(c).cloned().unwrap_or(Value::Null));
    }
    Ok(InsertRow { table, columns_key, columns, row })
}

struct InsertGroup {
    table: String,
    columns: Vec<String>,
    rows: Vec<Map<String, Value>>,
}

/// Groups by `(table, raw columns string)`, exactly like the reference's
/// `tables[table][columnString]` nesting -- the raw column-list text is
/// the grouping key, not a normalized column set, so `"a, b"` and `"a,b"`
/// land in different groups even though they parse to the same columns.
fn group_insert_rows(items: Vec<InsertRow>) -> Vec<InsertGroup> {
    let mut groups: Vec<(String, String, InsertGroup)> = Vec::new();
    for r in items {
        if let Some((_, _, g)) = groups.iter_mut().find(|(t, c, _)| *t == r.table && *c == r.columns_key) {
            g.rows.push(r.row);
        } else {
            groups.push((r.table.clone(), r.columns_key.clone(), InsertGroup { table: r.table, columns: r.columns, rows: vec![r.row] }));
        }
    }
    groups.into_iter().map(|(_, _, g)| g).collect()
}

/// The reference's `mssqlChunk`: packs rows into statements of at most
/// `MSSQL_PARAMETER_LIMIT` (2100) bound parameters, using `>=` (so a chunk
/// starts a new statement as soon as adding the next row would meet or
/// exceed the limit, not only once it's exceeded).
const MSSQL_PARAMETER_LIMIT: usize = 2100;

fn mssql_chunk(rows: &[Map<String, Value>]) -> Vec<Vec<&Map<String, Value>>> {
    let mut chunks: Vec<Vec<&Map<String, Value>>> = vec![vec![]];
    let mut current = 0usize;
    for row in rows {
        let n = row.len();
        if current + n >= MSSQL_PARAMETER_LIMIT {
            chunks.push(vec![]);
            current = 0;
        }
        chunks.last_mut().expect("always has at least one chunk").push(row);
        current += n;
    }
    chunks
}

async fn do_insert(ctx: &ExecCtx<'_>, client: &mut MssqlClient, input_len: usize) -> NodeResult<()> {
    let mut rows = Vec::with_capacity(input_len);
    for i in 0..input_len {
        rows.push(resolve_insert_row(ctx, i)?);
    }
    let groups = group_insert_rows(rows);
    for g in groups {
        let table_sql = escape_table_name(&g.table);
        let cols_sql = g.columns.iter().map(|c| quote_ident(c, Q)).collect::<Vec<_>>().join(", ");
        for chunk in mssql_chunk(&g.rows) {
            if chunk.is_empty() {
                continue;
            }
            let mut placeholders = Vec::with_capacity(chunk.len());
            let mut values: Vec<Value> = Vec::new();
            let mut p = 1usize;
            for row in &chunk {
                let mut tuple = Vec::with_capacity(g.columns.len());
                for c in &g.columns {
                    tuple.push(format!("@P{p}"));
                    values.push(row.get(c).cloned().unwrap_or(Value::Null));
                    p += 1;
                }
                placeholders.push(format!("({})", tuple.join(", ")));
            }
            let sql = format!("INSERT INTO {table_sql} ({cols_sql}) VALUES {}", placeholders.join(", "));
            let mut q = Query::new(sql.clone());
            for v in &values {
                bind_value(&mut q, v);
            }
            q.execute(client).await.map_err(|e| friendly_query_error(e, &sql))?;
        }
    }
    Ok(())
}

// ---- update -----------------------------------------------------------------------

struct UpdateRow {
    table: String,
    columns: Vec<String>,
    update_key: String,
    set: Map<String, Value>,
    match_value: Value,
}

fn resolve_update_row(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<UpdateRow> {
    let table = ctx.param_str("table", i, "")?;
    let columns_key = ctx.param_str("columns", i, "")?;
    let columns: Vec<String> = columns_key.split(',').map(|s| s.trim().to_string()).collect();
    let update_key = ctx.param_str("updateKey", i, "id")?;
    let item_json = ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default();
    let mut set = Map::new();
    for c in &columns {
        set.insert(c.clone(), item_json.get(c).cloned().unwrap_or(Value::Null));
    }
    let match_value = item_json.get(&update_key).cloned().unwrap_or(Value::Null);
    Ok(UpdateRow { table, columns, update_key, set, match_value })
}

/// One `UPDATE` per item (unlike `insert`, the reference never batches
/// update rows into one multi-row statement).
async fn do_update(ctx: &ExecCtx<'_>, client: &mut MssqlClient, input_len: usize) -> NodeResult<()> {
    for i in 0..input_len {
        let r = resolve_update_row(ctx, i)?;
        let table_sql = escape_table_name(&r.table);
        let mut set_parts = Vec::with_capacity(r.columns.len());
        let mut values: Vec<Value> = Vec::new();
        let mut p = 1usize;
        for c in &r.columns {
            set_parts.push(format!("{} = @P{p}", quote_ident(c, Q)));
            values.push(r.set.get(c).cloned().unwrap_or(Value::Null));
            p += 1;
        }
        let condition = format!("{} = @P{p}", quote_ident(&r.update_key, Q));
        values.push(r.match_value.clone());
        let sql = format!("UPDATE {table_sql} SET {} WHERE {condition}", set_parts.join(", "));
        let mut q = Query::new(sql.clone());
        for v in &values {
            bind_value(&mut q, v);
        }
        q.execute(client).await.map_err(|e| friendly_query_error(e, &sql))?;
    }
    Ok(())
}

// ---- delete -----------------------------------------------------------------------

struct DeleteRow {
    table: String,
    delete_key: String,
    value: Value,
}

fn resolve_delete_row(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<DeleteRow> {
    let table = ctx.param_str("table", i, "")?;
    let delete_key = ctx.param_str("deleteKey", i, "id")?;
    let item_json = ctx.input().get(i).map(|it| it.json.clone()).unwrap_or_default();
    let value = item_json.get(&delete_key).cloned().unwrap_or(Value::Null);
    Ok(DeleteRow { table, delete_key, value })
}

struct DeleteGroup {
    table: String,
    delete_key: String,
    values: Vec<Value>,
}

/// The reference's `deleteOperation`: grouped by `(table, deleteKey)`,
/// chunked in flat groups of at most 1000 rows (`utilities.chunk`) --
/// unlike `insert`'s 2100-*parameter* limit, this is a flat row-count cap
/// regardless of parameter count (each row binds exactly one parameter
/// here, so the two limits rarely interact in practice).
const DELETE_CHUNK_SIZE: usize = 1000;

async fn do_delete(ctx: &ExecCtx<'_>, client: &mut MssqlClient, input_len: usize) -> NodeResult<u64> {
    let mut rows = Vec::with_capacity(input_len);
    for i in 0..input_len {
        rows.push(resolve_delete_row(ctx, i)?);
    }
    let mut groups: Vec<DeleteGroup> = Vec::new();
    for r in rows {
        if let Some(g) = groups.iter_mut().find(|g| g.table == r.table && g.delete_key == r.delete_key) {
            g.values.push(r.value);
        } else {
            groups.push(DeleteGroup { table: r.table, delete_key: r.delete_key, values: vec![r.value] });
        }
    }

    let mut total = 0u64;
    for g in groups {
        let table_sql = escape_table_name(&g.table);
        let key_sql = quote_ident(&g.delete_key, Q);
        for chunk in g.values.chunks(DELETE_CHUNK_SIZE) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|n| format!("@P{n}")).collect();
            let sql = format!("DELETE FROM {table_sql} WHERE {key_sql} IN ({})", placeholders.join(", "));
            let mut q = Query::new(sql.clone());
            for v in chunk {
                bind_value(&mut q, v);
            }
            let res = q.execute(client).await.map_err(|e| friendly_query_error(e, &sql))?;
            total += res.rows_affected().iter().sum::<u64>();
        }
    }
    Ok(total)
}

// ---- identifier quoting (MSSQL dialect: `[...]` brackets) --------------------------

/// The reference's `escapeTableName`: a `[db].[schema].[table]`-style
/// bracketed dotted name is split on `].[` and each part re-escaped
/// separately; anything else is escaped as one identifier.
fn escape_table_name(table: &str) -> String {
    let table = table.trim();
    if let Some(inner) = table.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        inner.split("].[").map(|part| quote_ident(&format!("[{part}]"), Q)).collect::<Vec<_>>().join(".")
    } else {
        quote_ident(table, Q)
    }
}

// ---- connection ---------------------------------------------------------------------

fn cred_str(cred: &Value, key: &str, default: &str) -> String {
    cred.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(default).to_string()
}

fn cred_u64(cred: &Value, key: &str, default: u64) -> u64 {
    cred.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(default)
}

fn cred_bool(cred: &Value, key: &str, default: bool) -> bool {
    cred.get(key).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s == "true"))).unwrap_or(default)
}

async fn connect(cred: &Value) -> NodeResult<MssqlClient> {
    let server = cred_str(cred, "server", "localhost");
    let port = cred_u64(cred, "port", 1433) as u16;
    let database = cred_str(cred, "database", "master");
    let user = cred_str(cred, "user", "sa");
    let password = cred_str(cred, "password", "");
    let tls = cred_bool(cred, "tls", true);
    let allow_unauthorized = cred_bool(cred, "allowUnauthorizedCerts", false);
    let connect_timeout_ms = cred_u64(cred, "connectTimeout", 15000).max(1);
    let request_timeout_ms = cred_u64(cred, "requestTimeout", 15000).max(1);

    let mut config = Config::new();
    config.host(&server);
    config.port(port);
    config.database(&database);
    config.authentication(AuthMethod::sql_server(&user, &password));
    config.encryption(if tls { EncryptionLevel::Required } else { EncryptionLevel::NotSupported });
    if allow_unauthorized {
        config.trust_cert();
    }
    config.handshake_timeout(Some(Duration::from_millis(connect_timeout_ms)));
    config.command_timeout(Some(Duration::from_millis(request_timeout_ms)));

    let tcp = tokio::time::timeout(Duration::from_millis(connect_timeout_ms), TcpStream::connect(config.get_addr()))
        .await
        .map_err(|_| NodeError::new("Failed to connect to Microsoft SQL: connection timed out"))?
        .map_err(|e| NodeError::new(format!("Failed to connect to Microsoft SQL: {e}")))?;
    let _ = tcp.set_nodelay(true);

    Client::connect(config, tcp.compat_write()).await.map_err(|e| connection_error(&e))
}

fn connection_error(e: &tiberius::error::Error) -> NodeError {
    if let tiberius::error::Error::Server(te) = e {
        NodeError::new(te.message().to_string())
    } else {
        NodeError::new(e.to_string())
    }
}

fn friendly_query_error(e: tiberius::error::Error, sql: &str) -> NodeError {
    if let tiberius::error::Error::Server(te) = &e {
        return NodeError::new(te.message().to_string()).describe(format!("sql: {sql}, code: {}", te.code()));
    }
    connection_error(&e).describe(format!("Failed query: {sql}"))
}

// ---- result rows -> items -----------------------------------------------------------

/// n8n's `mssql`/node-mssql driver (`useUTC: true`, the default) hands
/// back `datetime`/`datetime2`/`smalldatetime` as a JS `Date`, serialised
/// to JSON as a full UTC ISO-8601 string.
fn mssql_datetime_string(d: chrono::NaiveDateTime) -> String {
    format!("{}Z", d.format("%Y-%m-%dT%H:%M:%S%.3f"))
}

/// `date`-only columns still become a full-timestamp `Date` object at
/// midnight UTC in the reference driver.
fn mssql_date_string(d: chrono::NaiveDate) -> String {
    format!("{}T00:00:00.000Z", d.format("%Y-%m-%d"))
}

/// `time`-only columns become a `Date` anchored at the Unix epoch
/// (`1970-01-01`) in the reference driver.
fn mssql_time_string(t: chrono::NaiveTime) -> String {
    format!("1970-01-01T{}Z", t.format("%H:%M:%S%.3f"))
}

fn numeric_to_json(n: tiberius::numeric::Numeric) -> Value {
    let s = n.to_string();
    s.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map(Value::Number).unwrap_or(Value::String(s))
}

/// Typed decode per column, dispatched on `ColumnData`'s decoded wire
/// variant (see the module doc comment for why, not `ColumnType`
/// metadata).
fn decode_column(row: &Row, idx: usize) -> Value {
    let data = match row.get_column_data(idx) {
        Ok(d) => d,
        Err(_) => return Value::Null,
    };
    match data {
        ColumnData::U8(v) => v.map(|n| json!(n)).unwrap_or(Value::Null),
        ColumnData::I16(v) => v.map(|n| json!(n)).unwrap_or(Value::Null),
        ColumnData::I32(v) => v.map(|n| json!(n)).unwrap_or(Value::Null),
        ColumnData::I64(v) => v.map(|n| json!(n)).unwrap_or(Value::Null),
        ColumnData::F32(v) => v.and_then(|n| serde_json::Number::from_f64(n as f64)).map(Value::Number).unwrap_or(Value::Null),
        ColumnData::F64(v) => v.and_then(|n| serde_json::Number::from_f64(n)).map(Value::Number).unwrap_or(Value::Null),
        ColumnData::Bit(v) => v.map(Value::Bool).unwrap_or(Value::Null),
        ColumnData::String(v) => v.clone().map(|s| Value::String(s.into_owned())).unwrap_or(Value::Null),
        ColumnData::Guid(v) => v.map(|u| Value::String(u.to_string())).unwrap_or(Value::Null),
        // Binary/varbinary/image: documented gap (see module doc comment)
        // -- real n8n <1.2 keeps this in json as a Buffer; r8r uses a
        // simpler hex string, matching mysql.rs's BLOB handling.
        ColumnData::Binary(v) => v.clone().map(|b| Value::String(format!("\\x{}", hex::encode(b.into_owned())))).unwrap_or(Value::Null),
        ColumnData::Numeric(v) => v.map(numeric_to_json).unwrap_or(Value::Null),
        ColumnData::DateTime(_) | ColumnData::SmallDateTime(_) | ColumnData::DateTime2(_) => {
            row.try_get::<chrono::NaiveDateTime, _>(idx).ok().flatten().map(|d| Value::String(mssql_datetime_string(d))).unwrap_or(Value::Null)
        }
        ColumnData::Date(_) => row.try_get::<chrono::NaiveDate, _>(idx).ok().flatten().map(|d| Value::String(mssql_date_string(d))).unwrap_or(Value::Null),
        ColumnData::Time(_) => row.try_get::<chrono::NaiveTime, _>(idx).ok().flatten().map(|t| Value::String(mssql_time_string(t))).unwrap_or(Value::Null),
        // Out of scope (see module doc comment): `datetimeoffset` and `xml`.
        ColumnData::DateTimeOffset(_) | ColumnData::Xml(_) => Value::Null,
    }
}

fn row_to_json_map(row: &Row) -> Map<String, Value> {
    let mut m = Map::new();
    for (idx, col) in row.columns().iter().enumerate() {
        m.insert(col.name().to_string(), decode_column(row, idx));
    }
    m
}
