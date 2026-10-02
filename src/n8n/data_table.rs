//! Data Tables (n8n's Data Tables feature): per-project user-defined
//! tables. Metadata lives in `data_table`/`data_table_column`; each
//! table's rows live in a real SQL table named `data_table_user_<id>`,
//! created and dropped dynamically here.
//!
//! Faithful to n8n 2.35.7's `@n8n/api-types` schemas
//! (`data-table.schema.ts`, `data-table-filter.schema.ts`) for naming
//! rules and filter conditions, and to its `sql-utils.ts` for the
//! physical table name. Column values are stored as TEXT (string, date:
//! RFC3339), `REAL`/`DOUBLE PRECISION` (number) and `INTEGER` (boolean,
//! 0/1 -- r8r's existing convention for cross-backend booleans through
//! sqlx's `Any` driver, e.g. `workflow_entity.active`) rather than n8n's
//! native `TIMESTAMPTZ`/`DOUBLE`/`BOOLEAN` types: a deliberate
//! simplification so every column type round-trips through the one
//! driver r8r uses for both SQLite and PostgreSQL. Identifiers are never
//! interpolated unvalidated: table and column names are checked against
//! the same regexes n8n uses before they are quoted and spliced into DDL,
//! and every value is bound as a query parameter.

use super::store::{new_id, now, Store};
use serde_json::{json, Map, Value};
use sqlx::{Column, Row};

// ---- validation (n8n's `@n8n/api-types` schemas) --------------------------

pub const NAME_MAX: usize = 128;
pub const COLUMN_NAME_MAX: usize = 63;
pub const ID_MAX: usize = 36;
pub const COLUMN_ERROR_MESSAGE: &str =
    "Only alphabetical characters and non-leading numbers and underscores are allowed for column names, and the maximum length is 63 characters.";

pub const SYSTEM_COLUMNS: [&str; 3] = ["id", "createdAt", "updatedAt"];

pub fn valid_table_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= NAME_MAX
}

pub fn valid_table_id(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= ID_MAX && id.chars().all(|c| c.is_ascii_alphanumeric())
}

pub fn valid_column_name(name: &str) -> bool {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > COLUMN_NAME_MAX {
        return false;
    }
    let mut chars = n.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphanumeric() || c == '_'),
        _ => false,
    }
}

fn qi(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn table_name(id: &str) -> String {
    format!("data_table_user_{id}")
}

// ---- types ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColType {
    String,
    Number,
    Boolean,
    Date,
}

impl ColType {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "string" => Some(Self::String),
            "number" => Some(Self::Number),
            "boolean" => Some(Self::Boolean),
            "date" => Some(Self::Date),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Date => "date",
        }
    }
    fn sql_type(&self, postgres: bool) -> &'static str {
        match self {
            Self::String => "TEXT",
            Self::Number if postgres => "DOUBLE PRECISION",
            Self::Number => "REAL",
            Self::Boolean => "INTEGER",
            Self::Date => "TEXT",
        }
    }
}

fn system_col_type(name: &str) -> Option<ColType> {
    match name {
        "id" => Some(ColType::Number),
        "createdAt" | "updatedAt" => Some(ColType::Date),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct ColumnMeta {
    pub id: String,
    pub name: String,
    pub col_type: ColType,
    pub index: i64,
}

impl ColumnMeta {
    pub fn to_json(&self, table_id: &str) -> Value {
        json!({"id": self.id, "dataTableId": table_id, "name": self.name, "type": self.col_type.as_str(), "index": self.index})
    }
}

/// An error carrying the status it maps to in the REST API.
#[derive(Debug)]
pub struct DtError {
    pub status: u16,
    pub message: String,
}

impl DtError {
    fn not_found(msg: impl Into<String>) -> Self {
        Self { status: 404, message: msg.into() }
    }
    fn conflict(msg: impl Into<String>) -> Self {
        Self { status: 409, message: msg.into() }
    }
    fn bad(msg: impl Into<String>) -> Self {
        Self { status: 400, message: msg.into() }
    }
}

impl std::fmt::Display for DtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for DtError {}

impl From<sqlx::Error> for DtError {
    fn from(e: sqlx::Error) -> Self {
        Self { status: 500, message: e.to_string() }
    }
}
impl From<anyhow::Error> for DtError {
    fn from(e: anyhow::Error) -> Self {
        Self { status: 500, message: e.to_string() }
    }
}

pub type DtResult<T> = Result<T, DtError>;

// ---- filters ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FilterCond {
    pub column: String,
    pub condition: String,
    pub value: Value,
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub or: bool,
    pub conditions: Vec<FilterCond>,
}

/// Parses `{"type": "and"|"or", "filters": [{"columnName","condition","value"}]}`.
pub fn parse_filter(v: &Value) -> DtResult<Filter> {
    if v.is_null() {
        return Ok(Filter::default());
    }
    let or = v.get("type").and_then(Value::as_str) == Some("or");
    let mut conditions = Vec::new();
    if let Some(arr) = v.get("filters").and_then(Value::as_array) {
        for f in arr {
            let column = f.get("columnName").and_then(Value::as_str).ok_or_else(|| DtError::bad("Invalid filter fields"))?.to_string();
            let condition = f.get("condition").and_then(Value::as_str).unwrap_or("eq").to_string();
            if !["eq", "neq", "like", "ilike", "gt", "gte", "lt", "lte"].contains(&condition.as_str()) {
                return Err(DtError::bad(format!("Invalid filter condition \"{condition}\"")));
            }
            let value = f.get("value").cloned().unwrap_or(Value::Null);
            conditions.push(FilterCond { column, condition, value });
        }
    }
    Ok(Filter { or, conditions })
}

enum Bind {
    S(String),
    F(f64),
    I(i64),
    Null,
}

fn coerce(col_type: ColType, value: &Value) -> Bind {
    if value.is_null() {
        return Bind::Null;
    }
    match col_type {
        ColType::String | ColType::Date => Bind::S(value.as_str().map(String::from).unwrap_or_else(|| value.to_string())),
        ColType::Number => Bind::F(value.as_f64().unwrap_or(0.0)),
        ColType::Boolean => Bind::I(if value.as_bool().unwrap_or(false) { 1 } else { 0 }),
    }
}

/// Builds a `WHERE ...` fragment (placeholders as `?`; empty when there are
/// no conditions) and its bind values, validating every column name
/// against the table's real schema.
fn where_clause(filter: &Filter, columns: &[ColumnMeta]) -> DtResult<(String, Vec<Bind>)> {
    if filter.conditions.is_empty() {
        return Ok((String::new(), Vec::new()));
    }
    let lookup = |name: &str| -> Option<ColType> { system_col_type(name).or_else(|| columns.iter().find(|c| c.name == name).map(|c| c.col_type)) };
    let mut unknown: Vec<&str> = Vec::new();
    for c in &filter.conditions {
        if lookup(&c.column).is_none() {
            unknown.push(&c.column);
        }
    }
    if !unknown.is_empty() {
        return Err(DtError::bad(format!("Filter validation failed: Column(s) \"{}\" do not exist in the selected table.", unknown.join(", "))));
    }
    let mut parts = Vec::new();
    let mut binds = Vec::new();
    for c in &filter.conditions {
        let ty = lookup(&c.column).unwrap();
        let col = qi(&c.column);
        match c.condition.as_str() {
            "eq" if c.value.is_null() => parts.push(format!("{col} IS NULL")),
            "neq" if c.value.is_null() => parts.push(format!("{col} IS NOT NULL")),
            "eq" => {
                parts.push(format!("{col} = ?"));
                binds.push(coerce(ty, &c.value));
            }
            "neq" => {
                parts.push(format!("{col} != ?"));
                binds.push(coerce(ty, &c.value));
            }
            "gt" => {
                parts.push(format!("{col} > ?"));
                binds.push(coerce(ty, &c.value));
            }
            "gte" => {
                parts.push(format!("{col} >= ?"));
                binds.push(coerce(ty, &c.value));
            }
            "lt" => {
                parts.push(format!("{col} < ?"));
                binds.push(coerce(ty, &c.value));
            }
            "lte" => {
                parts.push(format!("{col} <= ?"));
                binds.push(coerce(ty, &c.value));
            }
            "like" => {
                parts.push(format!("{col} LIKE ?"));
                binds.push(Bind::S(c.value.as_str().unwrap_or_default().to_string()));
            }
            "ilike" => {
                parts.push(format!("{col} ILIKE ?"));
                binds.push(Bind::S(c.value.as_str().unwrap_or_default().to_string()));
            }
            other => return Err(DtError::bad(format!("Invalid filter condition \"{other}\""))),
        }
    }
    let joiner = if filter.or { " OR " } else { " AND " };
    Ok((format!(" WHERE {}", parts.join(joiner)), binds))
}

/// `ilike` on SQLite: `LIKE` is already ASCII-case-insensitive there, so it
/// is used as-is. On PostgreSQL the native `ILIKE` is used for `ilike` and
/// `LIKE` (case-sensitive) for `like` -- SQLite's `LIKE` cannot easily be
/// made case-sensitive, a known, documented deviation from n8n on SQLite.
fn adapt_like(sql: &str, postgres: bool) -> String {
    if postgres {
        sql.to_string()
    } else {
        sql.replace("ILIKE", "LIKE")
    }
}

fn row_to_json(row: &sqlx::any::AnyRow, columns: &[ColumnMeta]) -> Value {
    let mut out = Map::new();
    for col in row.columns() {
        let name = col.name().to_string();
        if name == "id" {
            let v: Option<i64> = row.try_get("id").ok();
            out.insert(name, v.map(Value::from).unwrap_or(Value::Null));
            continue;
        }
        if name == "createdAt" || name == "updatedAt" {
            let v: Option<String> = row.try_get(name.as_str()).ok();
            out.insert(name, v.map(Value::from).unwrap_or(Value::Null));
            continue;
        }
        let Some(meta) = columns.iter().find(|c| c.name == name) else { continue };
        let value = match meta.col_type {
            ColType::String | ColType::Date => row.try_get::<Option<String>, _>(name.as_str()).ok().flatten().map(Value::from).unwrap_or(Value::Null),
            ColType::Number => row.try_get::<Option<f64>, _>(name.as_str()).ok().flatten().map(Value::from).unwrap_or(Value::Null),
            ColType::Boolean => row.try_get::<Option<i64>, _>(name.as_str()).ok().flatten().map(|i| Value::Bool(i != 0)).unwrap_or(Value::Null),
        };
        out.insert(name, value);
    }
    Value::Object(out)
}

impl Store {
    // ---- table metadata ---------------------------------------------------

    pub async fn create_data_table(&self, project_id: Option<&str>, name: &str, columns: &[(String, String)]) -> DtResult<Value> {
        let name = name.trim();
        if !valid_table_name(name) {
            return Err(DtError::bad("Invalid data table name"));
        }
        for (cname, ctype) in columns {
            if !valid_column_name(cname) || SYSTEM_COLUMNS.contains(&cname.as_str()) {
                return Err(DtError::bad(COLUMN_ERROR_MESSAGE));
            }
            if ColType::parse(ctype).is_none() {
                return Err(DtError::bad(format!("Invalid column type \"{ctype}\"")));
            }
        }
        if self.data_table_name_taken(project_id, name, None).await? {
            return Err(DtError::conflict(format!("A data table with the name \"{name}\" already exists")));
        }
        let id = new_id();
        let now_s = now();
        sqlx::query(&self.sql("INSERT INTO data_table (id, project_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)"))
            .bind(&id)
            .bind(project_id)
            .bind(name)
            .bind(&now_s)
            .bind(&now_s)
            .execute(&self.writer)
            .await?;
        let mut col_metas = Vec::new();
        for (i, (cname, ctype)) in columns.iter().enumerate() {
            col_metas.push(self.insert_column_row(&id, cname, ColType::parse(ctype).unwrap(), i as i64).await?);
        }
        let ddl_cols: Vec<String> = col_metas.iter().map(|c| format!("{} {}", qi(&c.name), c.col_type.sql_type(self.postgres))).collect();
        let create = format!(
            "CREATE TABLE {} (\"id\" {} PRIMARY KEY {}, {}\"createdAt\" TEXT NOT NULL, \"updatedAt\" TEXT NOT NULL)",
            qi(&table_name(&id)),
            if self.postgres { "SERIAL" } else { "INTEGER" },
            if self.postgres { "" } else { "AUTOINCREMENT" },
            if ddl_cols.is_empty() { String::new() } else { format!("{}, ", ddl_cols.join(", ")) },
        );
        sqlx::query(&create).execute(&self.writer).await?;
        Ok(self.data_table_json(&id, name, project_id, &now_s, &now_s, &col_metas).await)
    }

    async fn insert_column_row(&self, table_id: &str, name: &str, col_type: ColType, index: i64) -> DtResult<ColumnMeta> {
        let id = new_id();
        let now_s = now();
        sqlx::query(&self.sql("INSERT INTO data_table_column (id, data_table_id, name, type, col_index, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"))
            .bind(&id)
            .bind(table_id)
            .bind(name)
            .bind(col_type.as_str())
            .bind(index)
            .bind(&now_s)
            .bind(&now_s)
            .execute(&self.writer)
            .await?;
        Ok(ColumnMeta { id, name: name.to_string(), col_type, index })
    }

    async fn data_table_name_taken(&self, project_id: Option<&str>, name: &str, except_id: Option<&str>) -> anyhow::Result<bool> {
        let sql = match (project_id, except_id) {
            (Some(_), Some(_)) => "SELECT id FROM data_table WHERE project_id = ? AND name = ? AND id != ?",
            (Some(_), None) => "SELECT id FROM data_table WHERE project_id = ? AND name = ?",
            (None, Some(_)) => "SELECT id FROM data_table WHERE project_id IS NULL AND name = ? AND id != ?",
            (None, None) => "SELECT id FROM data_table WHERE project_id IS NULL AND name = ?",
        };
        let sql_s = self.sql(sql);
        let mut q = sqlx::query(&sql_s);
        if let Some(p) = project_id {
            q = q.bind(p);
        }
        q = q.bind(name);
        if let Some(e) = except_id {
            q = q.bind(e);
        }
        Ok(q.fetch_optional(&self.pool).await?.is_some())
    }

    async fn data_table_json(&self, id: &str, name: &str, project_id: Option<&str>, created_at: &str, updated_at: &str, columns: &[ColumnMeta]) -> Value {
        json!({
            "id": id, "name": name, "projectId": project_id,
            "columns": columns.iter().map(|c| c.to_json(id)).collect::<Vec<_>>(),
            "createdAt": created_at, "updatedAt": updated_at,
        })
    }

    /// Row only (no columns), for internal lookups.
    async fn data_table_row(&self, id: &str) -> anyhow::Result<Option<(Option<String>, String, String, String)>> {
        let row = sqlx::query(&self.sql("SELECT project_id, name, created_at, updated_at FROM data_table WHERE id = ?")).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| (r.try_get::<Option<String>, _>("project_id").unwrap_or(None), r.get("name"), r.get("created_at"), r.get("updated_at"))))
    }

    pub async fn get_data_table(&self, id: &str) -> DtResult<Value> {
        let Some((project_id, name, created_at, updated_at)) = self.data_table_row(id).await? else {
            return Err(DtError::not_found(format!("Data table with ID \"{id}\" not found")));
        };
        let columns = self.get_data_table_columns(id).await?;
        Ok(self.data_table_json(id, &name, project_id.as_deref(), &created_at, &updated_at, &columns).await)
    }

    /// `None` project_id means the project scope is "no project"
    /// (personal/headless); `Some(p)` restricts to that project.
    pub async fn list_data_tables(
        &self,
        project_id: Option<&str>,
        name_filter: Option<&str>,
        id_filter: Option<&str>,
        sort_by: Option<&str>,
        skip: i64,
        take: i64,
    ) -> DtResult<(Vec<Value>, i64)> {
        let mut sql = "SELECT id, project_id, name, created_at, updated_at FROM data_table".to_string();
        let mut clauses = Vec::new();
        if project_id.is_some() {
            clauses.push("project_id = ?".to_string());
        } else {
            clauses.push("project_id IS NULL".to_string());
        }
        if name_filter.is_some() {
            clauses.push("name = ?".to_string());
        }
        if id_filter.is_some() {
            clauses.push("id = ?".to_string());
        }
        if !clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&clauses.join(" AND "));
        }
        let (col, dir) = match sort_by.and_then(|s| s.split_once(':')) {
            Some(("name", d)) => ("name", d),
            Some(("createdAt", d)) => ("created_at", d),
            Some(("updatedAt", d)) => ("updated_at", d),
            _ => ("updated_at", "desc"),
        };
        let dir = if dir.eq_ignore_ascii_case("asc") { "ASC" } else { "DESC" };
        sql.push_str(&format!(" ORDER BY {col} {dir}, id"));
        let sql_s = self.sql(&sql);
        let mut q = sqlx::query(&sql_s);
        if let Some(p) = project_id {
            q = q.bind(p);
        }
        if let Some(n) = name_filter {
            q = q.bind(n);
        }
        if let Some(i) = id_filter {
            q = q.bind(i);
        }
        let rows = q.fetch_all(&self.pool).await?;
        let total = rows.len() as i64;
        let mut out = Vec::new();
        for r in rows.iter().skip(skip.max(0) as usize).take(if take <= 0 { rows.len() } else { take as usize }) {
            let id: String = r.get("id");
            let columns = self.get_data_table_columns(&id).await?;
            out.push(
                self.data_table_json(
                    &id,
                    &r.get::<String, _>("name"),
                    r.try_get::<Option<String>, _>("project_id").unwrap_or(None).as_deref(),
                    &r.get::<String, _>("created_at"),
                    &r.get::<String, _>("updated_at"),
                    &columns,
                )
                .await,
            );
        }
        Ok((out, total))
    }

    pub async fn rename_data_table(&self, id: &str, project_id: Option<&str>, name: &str) -> DtResult<Value> {
        let Some((existing_project, _, _, _)) = self.data_table_row(id).await? else {
            return Err(DtError::not_found(format!("Data table with ID \"{id}\" not found")));
        };
        if existing_project.as_deref() != project_id {
            return Err(DtError::not_found(format!("Data table with ID \"{id}\" not found")));
        }
        let name = name.trim();
        if !valid_table_name(name) {
            return Err(DtError::bad("Invalid data table name"));
        }
        if self.data_table_name_taken(project_id, name, Some(id)).await? {
            return Err(DtError::conflict(format!("A data table with the name \"{name}\" already exists")));
        }
        sqlx::query(&self.sql("UPDATE data_table SET name = ?, updated_at = ? WHERE id = ?")).bind(name).bind(now()).bind(id).execute(&self.writer).await?;
        self.get_data_table(id).await
    }

    pub async fn delete_data_table(&self, id: &str, project_id: Option<&str>) -> DtResult<()> {
        let Some((existing_project, _, _, _)) = self.data_table_row(id).await? else {
            return Err(DtError::not_found(format!("Data table with ID \"{id}\" not found")));
        };
        if existing_project.as_deref() != project_id {
            return Err(DtError::not_found(format!("Data table with ID \"{id}\" not found")));
        }
        sqlx::query(&format!("DROP TABLE {}", qi(&table_name(id)))).execute(&self.writer).await?;
        sqlx::query(&self.sql("DELETE FROM data_table_column WHERE data_table_id = ?")).bind(id).execute(&self.writer).await?;
        sqlx::query(&self.sql("DELETE FROM data_table WHERE id = ?")).bind(id).execute(&self.writer).await?;
        Ok(())
    }

    pub async fn clear_data_table(&self, id: &str) -> DtResult<()> {
        sqlx::query(&format!("DELETE FROM {}", qi(&table_name(id)))).execute(&self.writer).await?;
        Ok(())
    }

    // ---- columns ------------------------------------------------------------

    pub async fn get_data_table_columns(&self, table_id: &str) -> anyhow::Result<Vec<ColumnMeta>> {
        let rows = sqlx::query(&self.sql("SELECT id, name, type, col_index FROM data_table_column WHERE data_table_id = ? ORDER BY col_index"))
            .bind(table_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(|r| ColumnMeta { id: r.get("id"), name: r.get("name"), col_type: ColType::parse(&r.get::<String, _>("type")).unwrap_or(ColType::String), index: r.get("col_index") })
            .collect())
    }

    pub async fn add_data_table_column(&self, table_id: &str, project_id: Option<&str>, name: &str, col_type: &str, index: Option<i64>) -> DtResult<Value> {
        self.must_own(table_id, project_id).await?;
        if !valid_column_name(name) || SYSTEM_COLUMNS.contains(&name) {
            return Err(DtError::bad(COLUMN_ERROR_MESSAGE));
        }
        let Some(ty) = ColType::parse(col_type) else { return Err(DtError::bad(format!("Invalid column type \"{col_type}\""))) };
        let existing = self.get_data_table_columns(table_id).await?;
        if existing.iter().any(|c| c.name == name) {
            return Err(DtError::conflict(format!("Column \"{name}\" already exists in this data table")));
        }
        let idx = index.unwrap_or(existing.len() as i64);
        sqlx::query(&format!("ALTER TABLE {} ADD {} {}", qi(&table_name(table_id)), qi(name), ty.sql_type(self.postgres))).execute(&self.writer).await?;
        let meta = self.insert_column_row(table_id, name, ty, idx).await?;
        sqlx::query(&self.sql("UPDATE data_table SET updated_at = ? WHERE id = ?")).bind(now()).bind(table_id).execute(&self.writer).await?;
        Ok(meta.to_json(table_id))
    }

    pub async fn delete_data_table_column(&self, table_id: &str, project_id: Option<&str>, column_id: &str) -> DtResult<()> {
        self.must_own(table_id, project_id).await?;
        let Some(col) = self.get_data_table_columns(table_id).await?.into_iter().find(|c| c.id == column_id) else {
            return Err(DtError::not_found("Column not found"));
        };
        sqlx::query(&format!("ALTER TABLE {} DROP COLUMN {}", qi(&table_name(table_id)), qi(&col.name))).execute(&self.writer).await?;
        sqlx::query(&self.sql("DELETE FROM data_table_column WHERE id = ?")).bind(column_id).execute(&self.writer).await?;
        sqlx::query(&self.sql("UPDATE data_table SET updated_at = ? WHERE id = ?")).bind(now()).bind(table_id).execute(&self.writer).await?;
        Ok(())
    }

    pub async fn rename_data_table_column(&self, table_id: &str, project_id: Option<&str>, column_id: &str, new_name: &str) -> DtResult<Value> {
        self.must_own(table_id, project_id).await?;
        if !valid_column_name(new_name) || SYSTEM_COLUMNS.contains(&new_name) {
            return Err(DtError::bad(COLUMN_ERROR_MESSAGE));
        }
        let cols = self.get_data_table_columns(table_id).await?;
        let Some(col) = cols.iter().find(|c| c.id == column_id) else { return Err(DtError::not_found("Column not found")) };
        if cols.iter().any(|c| c.id != column_id && c.name == new_name) {
            return Err(DtError::conflict(format!("Column \"{new_name}\" already exists in this data table")));
        }
        sqlx::query(&format!("ALTER TABLE {} RENAME COLUMN {} TO {}", qi(&table_name(table_id)), qi(&col.name), qi(new_name))).execute(&self.writer).await?;
        sqlx::query(&self.sql("UPDATE data_table_column SET name = ?, updated_at = ? WHERE id = ?")).bind(new_name).bind(now()).bind(column_id).execute(&self.writer).await?;
        sqlx::query(&self.sql("UPDATE data_table SET updated_at = ? WHERE id = ?")).bind(now()).bind(table_id).execute(&self.writer).await?;
        let updated = self.get_data_table_columns(table_id).await?.into_iter().find(|c| c.id == column_id).unwrap();
        Ok(updated.to_json(table_id))
    }

    pub async fn move_data_table_column(&self, table_id: &str, project_id: Option<&str>, column_id: &str, target_index: i64) -> DtResult<Value> {
        self.must_own(table_id, project_id).await?;
        let mut cols = self.get_data_table_columns(table_id).await?;
        let Some(pos) = cols.iter().position(|c| c.id == column_id) else { return Err(DtError::not_found("Column not found")) };
        let moved = cols.remove(pos);
        let target = (target_index.max(0) as usize).min(cols.len());
        cols.insert(target, moved);
        for (i, c) in cols.iter().enumerate() {
            sqlx::query(&self.sql("UPDATE data_table_column SET col_index = ? WHERE id = ?")).bind(i as i64).bind(&c.id).execute(&self.writer).await?;
        }
        sqlx::query(&self.sql("UPDATE data_table SET updated_at = ? WHERE id = ?")).bind(now()).bind(table_id).execute(&self.writer).await?;
        Ok(cols.iter().find(|c| c.id == column_id).unwrap().to_json(table_id))
    }

    async fn must_own(&self, table_id: &str, project_id: Option<&str>) -> DtResult<()> {
        let Some((existing, _, _, _)) = self.data_table_row(table_id).await? else {
            return Err(DtError::not_found(format!("Data table with ID \"{table_id}\" not found")));
        };
        if existing.as_deref() != project_id {
            return Err(DtError::not_found(format!("Data table with ID \"{table_id}\" not found")));
        }
        Ok(())
    }

    // ---- rows -----------------------------------------------------------

    fn row_data_binds<'a>(&self, data: &'a Map<String, Value>, columns: &[ColumnMeta]) -> DtResult<(Vec<&'a str>, Vec<Bind>)> {
        let mut names = Vec::new();
        let mut binds = Vec::new();
        for (k, v) in data {
            if !valid_column_name(k) || SYSTEM_COLUMNS.contains(&k.as_str()) {
                return Err(DtError::bad(COLUMN_ERROR_MESSAGE));
            }
            let Some(meta) = columns.iter().find(|c| &c.name == k) else {
                return Err(DtError::bad(format!("Column \"{k}\" does not exist in this data table")));
            };
            names.push(k.as_str());
            binds.push(coerce(meta.col_type, v));
        }
        Ok((names, binds))
    }

    /// `return_type`: "all" (full rows), "id" (ids only), "count" (just a count).
    pub async fn insert_data_table_rows(&self, table_id: &str, rows: &[Map<String, Value>], return_type: &str) -> DtResult<Value> {
        let columns = self.get_data_table_columns(table_id).await?;
        let mut conn = self.writer.acquire().await?;
        let mut inserted = Vec::new();
        for data in rows {
            let (names, binds) = self.row_data_binds(data, &columns)?;
            let now_s = now();
            let mut cols = vec!["createdAt".to_string(), "updatedAt".to_string()];
            cols.extend(names.iter().map(|n| n.to_string()));
            let placeholders: Vec<&str> = (0..cols.len()).map(|_| "?").collect();
            let sql = format!("INSERT INTO {} ({}) VALUES ({})", qi(&table_name(table_id)), cols.iter().map(|c| qi(c)).collect::<Vec<_>>().join(", "), placeholders.join(", "));
            let sql_s = self.sql(&sql);
            let mut q = sqlx::query(&sql_s).bind(now_s.clone()).bind(now_s.clone());
            for b in &binds {
                q = bind_one(q, b);
            }
            q.execute(&mut *conn).await?;
            let id = super::store_batch::new_id(&mut conn, self.postgres).await?;
            inserted.push((id, now_s));
        }
        drop(conn);
        match return_type {
            "count" => Ok(json!({"count": inserted.len()})),
            "id" => Ok(Value::Array(inserted.iter().map(|(id, _)| json!({"id": id})).collect())),
            _ => {
                let mut out = Vec::new();
                for (id, _) in &inserted {
                    if let Some(row) = self.get_data_table_row_by_id(table_id, *id, &columns).await? {
                        out.push(row);
                    }
                }
                Ok(Value::Array(out))
            }
        }
    }

    async fn get_data_table_row_by_id(&self, table_id: &str, id: i64, columns: &[ColumnMeta]) -> anyhow::Result<Option<Value>> {
        let sql = format!("SELECT * FROM {} WHERE id = ?", qi(&table_name(table_id)));
        let row = sqlx::query(&self.sql(&sql)).bind(id).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| row_to_json(&r, columns)))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_data_table_rows(
        &self,
        table_id: &str,
        filter: &Filter,
        sort_by: Option<(&str, &str)>,
        skip: i64,
        take: i64,
    ) -> DtResult<(Vec<Value>, i64)> {
        let columns = self.get_data_table_columns(table_id).await?;
        let (where_sql, binds) = where_clause(filter, &columns)?;
        let where_sql = adapt_like(&where_sql, self.postgres);
        let count_sql = format!("SELECT COUNT(*) AS n FROM {}{}", qi(&table_name(table_id)), where_sql);
        let count_sql_s = self.sql(&count_sql);
        let mut cq = sqlx::query(&count_sql_s);
        for b in &binds {
            cq = bind_one(cq, b);
        }
        let total: i64 = cq.fetch_one(&self.pool).await?.get("n");

        let (col, dir) = match sort_by {
            Some((c, d)) if c == "id" || c == "createdAt" || c == "updatedAt" || columns.iter().any(|x| x.name == c) => (c, d),
            _ => ("id", "ASC"),
        };
        let dir = if dir.eq_ignore_ascii_case("desc") { "DESC" } else { "ASC" };
        let take = if take <= 0 { 50 } else { take };
        let sql = format!("SELECT * FROM {}{} ORDER BY {} {} LIMIT {} OFFSET {}", qi(&table_name(table_id)), where_sql, qi(col), dir, take, skip.max(0));
        let sql_s = self.sql(&sql);
        let mut q = sqlx::query(&sql_s);
        for b in &binds {
            q = bind_one(q, b);
        }
        let rows = q.fetch_all(&self.pool).await?;
        Ok((rows.iter().map(|r| row_to_json(r, &columns)).collect(), total))
    }

    pub async fn update_data_table_rows(&self, table_id: &str, filter: &Filter, data: &Map<String, Value>, return_data: bool, dry_run: bool) -> DtResult<Vec<Value>> {
        let columns = self.get_data_table_columns(table_id).await?;
        let (where_sql, where_binds) = where_clause(filter, &columns)?;
        let where_sql = adapt_like(&where_sql, self.postgres);
        let (names, data_binds) = self.row_data_binds(data, &columns)?;
        let sel = format!("SELECT * FROM {}{}", qi(&table_name(table_id)), where_sql);
        let sel_s = self.sql(&sel);
        let mut sq = sqlx::query(&sel_s);
        for b in &where_binds {
            sq = bind_one(sq, b);
        }
        let matched = sq.fetch_all(&self.pool).await?;
        let before: Vec<Value> = matched.iter().map(|r| row_to_json(r, &columns)).collect();
        if dry_run {
            return Ok(before);
        }
        let now_s = now();
        let set_cols: Vec<String> = names.iter().map(|n| format!("{} = ?", qi(n))).collect();
        let sql = format!("UPDATE {} SET \"updatedAt\" = ?{} {}", qi(&table_name(table_id)), if set_cols.is_empty() { String::new() } else { format!(", {}", set_cols.join(", ")) }, where_sql);
        let sql_s = self.sql(&sql);
        let mut q = sqlx::query(&sql_s).bind(now_s);
        for b in &data_binds {
            q = bind_one(q, b);
        }
        for b in &where_binds {
            q = bind_one(q, b);
        }
        q.execute(&self.writer).await?;
        if !return_data {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for row in &before {
            if let Some(id) = row["id"].as_i64() {
                if let Some(r) = self.get_data_table_row_by_id(table_id, id, &columns).await? {
                    out.push(r);
                }
            }
        }
        Ok(out)
    }

    pub async fn upsert_data_table_row(&self, table_id: &str, filter: &Filter, data: &Map<String, Value>, return_data: bool, dry_run: bool) -> DtResult<Vec<Value>> {
        let columns = self.get_data_table_columns(table_id).await?;
        let (where_sql, where_binds) = where_clause(filter, &columns)?;
        let where_sql = adapt_like(&where_sql, self.postgres);
        let sel = format!("SELECT * FROM {}{}", qi(&table_name(table_id)), where_sql);
        let sel_s = self.sql(&sel);
        let mut sq = sqlx::query(&sel_s);
        for b in &where_binds {
            sq = bind_one(sq, b);
        }
        let matched = sq.fetch_all(&self.pool).await?;
        if matched.is_empty() {
            if dry_run {
                return Ok(vec![json!({"before": Value::Null, "after": data.clone()})]);
            }
            let inserted = self.insert_data_table_rows(table_id, std::slice::from_ref(data), if return_data { "all" } else { "count" }).await?;
            return Ok(match inserted {
                Value::Array(a) => a,
                _ => Vec::new(),
            });
        }
        self.update_data_table_rows(table_id, filter, data, return_data, dry_run).await
    }

    pub async fn delete_data_table_rows(&self, table_id: &str, filter: &Filter, return_data: bool, dry_run: bool) -> DtResult<Vec<Value>> {
        let columns = self.get_data_table_columns(table_id).await?;
        let (where_sql, where_binds) = where_clause(filter, &columns)?;
        let where_sql = adapt_like(&where_sql, self.postgres);
        let sel = format!("SELECT * FROM {}{}", qi(&table_name(table_id)), where_sql);
        let sel_s = self.sql(&sel);
        let mut sq = sqlx::query(&sel_s);
        for b in &where_binds {
            sq = bind_one(sq, b);
        }
        let matched = sq.fetch_all(&self.pool).await?;
        let before: Vec<Value> = matched.iter().map(|r| row_to_json(r, &columns)).collect();
        if dry_run {
            return Ok(before);
        }
        let del = format!("DELETE FROM {}{}", qi(&table_name(table_id)), where_sql);
        let del_s = self.sql(&del);
        let mut dq = sqlx::query(&del_s);
        for b in &where_binds {
            dq = bind_one(dq, b);
        }
        dq.execute(&self.writer).await?;
        Ok(if return_data { before } else { Vec::new() })
    }
}

fn bind_one<'q>(q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>, b: &'q Bind) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    match b {
        Bind::S(s) => q.bind(s.as_str()),
        Bind::F(f) => q.bind(*f),
        Bind::I(i) => q.bind(*i),
        Bind::Null => q.bind(None::<String>),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::open("sqlite::memory:", "test-key-0123456789abcdef0123456789abcdef").await.unwrap()
    }

    fn cols() -> Vec<(String, String)> {
        vec![("name".into(), "string".into()), ("age".into(), "number".into()), ("active".into(), "boolean".into())]
    }

    #[tokio::test]
    async fn create_insert_get_update_delete() {
        let store = store().await;
        let table = store.create_data_table(None, "People", &cols()).await.unwrap();
        let id = table["id"].as_str().unwrap().to_string();
        assert_eq!(table["columns"].as_array().unwrap().len(), 3);

        let mut row = Map::new();
        row.insert("name".into(), json!("Ada"));
        row.insert("age".into(), json!(30));
        row.insert("active".into(), json!(true));
        let inserted = store.insert_data_table_rows(&id, &[row], "all").await.unwrap();
        let inserted = inserted.as_array().unwrap();
        assert_eq!(inserted.len(), 1);
        assert_eq!(inserted[0]["name"], json!("Ada"));
        assert_eq!(inserted[0]["active"], json!(true));

        let filter = parse_filter(&json!({"type": "and", "filters": [{"columnName": "name", "condition": "eq", "value": "Ada"}]})).unwrap();
        let (rows, count) = store.get_data_table_rows(&id, &filter, None, 0, 50).await.unwrap();
        assert_eq!(count, 1);
        assert_eq!(rows[0]["age"], json!(30.0));

        let mut data = Map::new();
        data.insert("age".into(), json!(31));
        let updated = store.update_data_table_rows(&id, &filter, &data, true, false).await.unwrap();
        assert_eq!(updated[0]["age"], json!(31.0));

        let deleted = store.delete_data_table_rows(&id, &filter, true, false).await.unwrap();
        assert_eq!(deleted.len(), 1);
        let (rows, count) = store.get_data_table_rows(&id, &filter, None, 0, 50).await.unwrap();
        assert_eq!(count, 0);
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn rejects_bad_names() {
        let store = store().await;
        assert!(store.create_data_table(None, "People", &[("1bad".into(), "string".into())]).await.is_err());
        assert!(store.create_data_table(None, "", &cols()).await.is_err());
        let t = store.create_data_table(None, "T", &cols()).await.unwrap();
        assert!(store.create_data_table(None, "T", &cols()).await.is_err(), "duplicate name must conflict");
        assert!(store.add_data_table_column(t["id"].as_str().unwrap(), None, "id", "string", None).await.is_err(), "system column name must be refused");
    }

    #[tokio::test]
    async fn upsert_inserts_then_updates() {
        let store = store().await;
        let table = store.create_data_table(None, "Up", &cols()).await.unwrap();
        let id = table["id"].as_str().unwrap().to_string();
        let filter = parse_filter(&json!({"filters": [{"columnName": "name", "condition": "eq", "value": "Grace"}]})).unwrap();
        let mut data = Map::new();
        data.insert("name".into(), json!("Grace"));
        data.insert("age".into(), json!(40));
        let first = store.upsert_data_table_row(&id, &filter, &data, true, false).await.unwrap();
        assert_eq!(first[0]["age"], json!(40.0));
        let mut data2 = Map::new();
        data2.insert("age".into(), json!(41));
        let second = store.upsert_data_table_row(&id, &filter, &data2, true, false).await.unwrap();
        assert_eq!(second[0]["age"], json!(41.0));
        let (rows, count) = store.get_data_table_rows(&id, &filter, None, 0, 50).await.unwrap();
        assert_eq!(count, 1);
        assert_eq!(rows[0]["age"], json!(41.0));
    }

    #[tokio::test]
    async fn columns_can_be_renamed_moved_and_deleted() {
        let store = store().await;
        let table = store.create_data_table(None, "Cols", &cols()).await.unwrap();
        let id = table["id"].as_str().unwrap().to_string();
        let columns = store.get_data_table_columns(&id).await.unwrap();
        let age_col = columns.iter().find(|c| c.name == "age").unwrap().clone();
        let renamed = store.rename_data_table_column(&id, None, &age_col.id, "years").await.unwrap();
        assert_eq!(renamed["name"], json!("years"));
        store.move_data_table_column(&id, None, &age_col.id, 0).await.unwrap();
        let columns = store.get_data_table_columns(&id).await.unwrap();
        assert_eq!(columns[0].name, "years");
        store.delete_data_table_column(&id, None, &age_col.id).await.unwrap();
        let columns = store.get_data_table_columns(&id).await.unwrap();
        assert!(columns.iter().all(|c| c.name != "years"));
    }
}
