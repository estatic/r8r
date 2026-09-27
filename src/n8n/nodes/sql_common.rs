//! SQL building blocks shared by the relational-database nodes (spec §6.6).
//!
//! n8n's Postgres and MySQL nodes share the same editor UI for resource
//! locators, the "Select Rows" (`where`) and "Sort" fixed collections, and
//! column-mapping. This module holds the engine-agnostic pieces (parsing
//! those parameter shapes, literal escaping, identifier quoting, and a
//! generic "is this a SELECT" sniffer) so the MySQL node (not yet
//! implemented) can reuse them instead of re-deriving its own copy. See
//! `src/n8n/nodes/postgres.rs` for the Postgres-specific query builder that
//! consumes these.

use crate::n8n::node::{ExecCtx, NodeError, NodeResult};
use serde_json::Value;

/// Which character a dialect uses to quote identifiers.
#[derive(Clone, Copy)]
pub enum IdentQuote {
    /// Postgres, standard SQL: `"name"`, embedded `"` doubled.
    Double,
    /// MySQL: `` `name` ``, embedded backtick doubled.
    Backtick,
}

pub fn quote_ident(name: &str, style: IdentQuote) -> String {
    match style {
        IdentQuote::Double => format!("\"{}\"", name.replace('"', "\"\"")),
        IdentQuote::Backtick => format!("`{}`", name.replace('`', "``")),
    }
}

/// Standard SQL string literal: single-quoted, embedded `'` doubled.
pub fn sql_string_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A row from the "Select Rows" (`where`) fixed collection.
#[derive(Debug, Clone)]
pub struct WhereClause {
    pub column: String,
    pub condition: String,
    pub value: Value,
}

/// A row from the "Sort" fixed collection.
#[derive(Debug, Clone)]
pub struct SortRule {
    pub column: String,
    pub direction: String,
}

const OPERATORS: &[&str] = &["equal", "!=", "LIKE", ">", "<", ">=", "<=", "IS NULL", "IS NOT NULL", "="];

/// Reads a `where` fixedCollection parameter (`{"values": [{"column",
/// "condition", "value"}, ...]}`), resolved for item `i`.
pub fn get_where_clauses(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Vec<WhereClause>> {
    let raw = ctx.param("where", i)?;
    let Some(values) = raw.get("values").and_then(Value::as_array) else { return Ok(vec![]) };
    let mut out = Vec::with_capacity(values.len());
    for v in values {
        let column = v.get("column").and_then(Value::as_str).unwrap_or_default().to_string();
        let mut condition = v.get("condition").and_then(Value::as_str).unwrap_or("equal").to_string();
        if !OPERATORS.contains(&condition.as_str()) {
            return Err(NodeError::new("Invalid where clause").at(i));
        }
        if condition == "equal" {
            condition = "=".to_string();
        }
        let value = v.get("value").cloned().unwrap_or(Value::Null);
        out.push(WhereClause { column, condition, value });
    }
    Ok(out)
}

/// Reads a `sort` fixedCollection parameter (`{"values": [{"column",
/// "direction"}, ...]}`), resolved for item `i`.
pub fn get_sort_rules(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Vec<SortRule>> {
    let raw = ctx.param("sort", i)?;
    let Some(values) = raw.get("values").and_then(Value::as_array) else { return Ok(vec![]) };
    Ok(values
        .iter()
        .map(|v| SortRule {
            column: v.get("column").and_then(Value::as_str).unwrap_or_default().to_string(),
            direction: if v.get("direction").and_then(Value::as_str) == Some("DESC") { "DESC".to_string() } else { "ASC".to_string() },
        })
        .collect())
}

/// The `value` of a resource-locator parameter (`{"__rl": true, "mode":
/// ..., "value": ...}`, or a plain string when it came from an
/// expression). Used for `schema`/`table` RL fields.
pub fn rl_value(ctx: &ExecCtx<'_>, path: &str, i: usize) -> NodeResult<String> {
    Ok(match ctx.param(path, i)? {
        Value::Object(o) => o.get("value").map(value_to_text).unwrap_or_default(),
        other => value_to_text(&other),
    })
}

pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A comma-separated list, or array, of column names (`options.outputColumns`).
pub fn string_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Value::String(s) if !s.is_empty() => s.split(',').map(|s| s.trim().to_string()).collect(),
        _ => vec![],
    }
}

/// Whether every top-level SQL statement in `sql` (split on `;`, ignoring
/// blanks) is a `SELECT`. Used to decide the empty-result shape after a
/// batch that touched no rows. Deliberately simple (no string/comment
/// stripping): good enough for the well-formed single-statement queries
/// the node itself generates and for typical user `executeQuery` text.
pub fn is_select_query(sql: &str) -> bool {
    let statements: Vec<&str> = sql.split(';').map(str::trim).filter(|s| !s.is_empty()).collect();
    if statements.is_empty() {
        return true;
    }
    statements.iter().all(|s| s.to_uppercase().starts_with("SELECT") || s.to_uppercase().starts_with("WITH"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_identifiers_per_dialect() {
        assert_eq!(quote_ident("a\"b", IdentQuote::Double), "\"a\"\"b\"");
        assert_eq!(quote_ident("a`b", IdentQuote::Backtick), "`a``b`");
    }

    #[test]
    fn escapes_string_literals() {
        assert_eq!(sql_string_literal("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn detects_select_queries() {
        assert!(is_select_query("SELECT * FROM t"));
        assert!(!is_select_query("INSERT INTO t VALUES (1)"));
        assert!(!is_select_query("SELECT 1; DELETE FROM t"));
    }
}
