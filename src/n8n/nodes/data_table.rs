//! The Data Table node (`n8n-nodes-base.dataTable`, typeVersion 1.1): row
//! insert/get/update/upsert/delete/rowExists/rowNotExists, and table
//! create/list/delete/rename/clear. Faithful to n8n's
//! `nodes/DataTable/actions/row/*.operation.js` for operation semantics
//! (filters, `returnAll`/`limit`, `orderBy`, dry run); backed directly by
//! `crate::n8n::data_table`/`Store` rather than n8n's HTTP data-table
//! proxy, since the node runs in the same process as the store.
//!
//! Parameter shapes are r8r's own (this codebase's node descriptions are
//! flat parameter-name lists, not full `displayOptions` UI schemas -- see
//! `node_types.rs`), chosen to mirror n8n's parameter *names* where there
//! is a natural one (`dataTableId`, `filters`, `matchType`, `returnAll`,
//! `limit`, `orderBy*`, `options.dryRun`) rather than its exact
//! `resourceMapper`/`fixedCollection` wire format:
//! - `dataTableId`: `{"mode": "id"|"name"|"list", "value": "..."}`.
//! - `columns`: `{"mappingMode": "defineBelow"|"autoMapInputData", "value": {...}}`.
//! - `filters`: `{"conditions": [{"keyName","condition","keyValue"}]}`,
//!   `matchType`: `"anyCondition"|"allConditions"`; `isEmpty`/`isNotEmpty`/
//!   `isTrue`/`isFalse` are accepted on `condition` and lowered to the
//!   store's `eq`/`neq` the way n8n's `buildGetManyFilter` does.
//! - table ops take `name`/`columns` (create) or `name` (rename).
//!
//! Row writes always ask the store to return the affected rows (`all`/
//! `returnData: true`), since the node always needs them for its output
//! items; a deviation from the REST API's own `count`/`false` defaults,
//! which exist for callers that do not want that cost.

use crate::n8n::data_table::{self, Filter, FilterCond};
use crate::n8n::node::{ExecCtx, NodeError, NodeResult, NodeType};
use crate::n8n::store::Store;
use crate::n8n::types::{Item, NodeOutput};
use serde_json::{json, Map, Value};

pub struct DataTable;

const DEFAULT_LIMIT: i64 = 50;

#[async_trait::async_trait]
impl NodeType for DataTable {
    fn type_name(&self) -> &'static str {
        "n8n-nodes-base.dataTable"
    }

    async fn execute(&self, ctx: &mut ExecCtx<'_>) -> NodeResult<NodeOutput> {
        let store = ctx.services.store.clone().ok_or_else(|| NodeError::new("Attempted to use Data table node but the module is disabled"))?;
        let project = resolve_project(&store, ctx).await;
        let resource = ctx.param_str("resource", 0, "row")?;
        let operation = ctx.param_str("operation", 0, "insert")?;

        if resource == "table" {
            return execute_table(ctx, &store, &project, &operation).await;
        }

        let input = if ctx.input().is_empty() { vec![Item::default()] } else { ctx.input().to_vec() };
        let mut out = Vec::new();
        for (i, item) in input.iter().enumerate() {
            match execute_row(ctx, &store, &project, &operation, i, item).await {
                Ok(mut items) => out.append(&mut items),
                Err(e) if ctx.continue_on_fail() => ctx.push_error_item(&e.at(i), i),
                Err(e) => return Err(e.at(i)),
            }
        }
        Ok(vec![out])
    }
}

async fn resolve_project(store: &Store, ctx: &ExecCtx<'_>) -> Option<String> {
    let id = ctx.workflow.id.as_deref()?;
    store.workflow_row(id).await.ok().flatten().and_then(|r| r.project_id)
}

async fn resolve_table_id(ctx: &ExecCtx<'_>, store: &Store, project: &Option<String>, i: usize) -> NodeResult<String> {
    let rl = ctx.param("dataTableId", i)?;
    let value = rl["value"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| NodeError::new("Data table is required"))?.to_string();
    if rl["mode"].as_str() == Some("name") {
        let (rows, _) = store
            .list_data_tables(project.as_deref(), Some(&value), None, None, 0, 1)
            .await
            .map_err(|e| NodeError::new(e.message))?;
        let row = rows.into_iter().next().ok_or_else(|| NodeError::new(format!("Data table with name \"{value}\" not found")))?;
        Ok(row["id"].as_str().unwrap_or_default().to_string())
    } else {
        // Checked eagerly (rather than left to fail inside the first SQL
        // statement that touches the table) so the error message matches
        // n8n's `DataTableNotFoundError` instead of a raw driver error.
        store.get_data_table(&value).await.map_err(dt_err)?;
        Ok(value)
    }
}

/// `columns.mappingMode`: `"defineBelow"` reads `columns.value` (an object
/// of column -> value); `"autoMapInputData"` uses the input item's own
/// json, minus the system columns (id/createdAt/updatedAt), the way n8n's
/// `getAddRow` does for round-tripping one data table's output into
/// another's input.
fn read_row_data(ctx: &ExecCtx<'_>, i: usize, item: &Item) -> NodeResult<Map<String, Value>> {
    let mode = ctx.param_str("columns.mappingMode", i, "defineBelow")?;
    if mode == "autoMapInputData" {
        let mut data = item.json.clone();
        for sys in data_table::SYSTEM_COLUMNS {
            data.remove(sys);
        }
        Ok(data)
    } else {
        Ok(ctx.param("columns.value", i)?.as_object().cloned().unwrap_or_default())
    }
}

fn read_filter(ctx: &ExecCtx<'_>, i: usize) -> NodeResult<Filter> {
    let match_type = ctx.param_str("matchType", i, "anyCondition")?;
    let conditions = ctx.param("filters.conditions", i)?;
    let mut out = Vec::new();
    for c in conditions.as_array().into_iter().flatten() {
        let key_name = c["keyName"].as_str().unwrap_or_default().to_string();
        let condition = c["condition"].as_str().unwrap_or("eq").to_string();
        let (condition, value) = match condition.as_str() {
            "isEmpty" => ("eq".to_string(), Value::Null),
            "isNotEmpty" => ("neq".to_string(), Value::Null),
            "isTrue" => ("eq".to_string(), json!(true)),
            "isFalse" => ("eq".to_string(), json!(false)),
            _ => (condition, c["keyValue"].clone()),
        };
        out.push(FilterCond { column: key_name, condition, value });
    }
    Ok(Filter { or: match_type != "allConditions", conditions: out })
}

fn row_item(v: Value) -> Item {
    Item::new(v.as_object().cloned().unwrap_or_default())
}

async fn execute_row(ctx: &mut ExecCtx<'_>, store: &Store, project: &Option<String>, operation: &str, i: usize, item: &Item) -> NodeResult<Vec<Item>> {
    let table_id = resolve_table_id(ctx, store, project, i).await?;
    let dry_run = ctx.param_bool("options.dryRun", i, false)?;

    match operation {
        "insert" => {
            let row = read_row_data(ctx, i, item)?;
            let inserted = store.insert_data_table_rows(&table_id, &[row], "all").await.map_err(dt_err)?;
            Ok(inserted.as_array().into_iter().flatten().cloned().map(row_item).collect())
        }
        "get" => {
            let filter = read_filter(ctx, i)?;
            let return_all = ctx.param_bool("returnAll", i, false)?;
            let limit = if return_all { 1_000_000_000 } else { ctx.param_f64("limit", i, DEFAULT_LIMIT as f64)? as i64 };
            let order_by = ctx.param_bool("orderBy", i, false)?;
            let sort = if order_by {
                let col = ctx.param_str("orderByColumn", i, "createdAt")?;
                let dir = ctx.param_str("orderByDirection", i, "DESC")?;
                Some((col, dir))
            } else {
                None
            };
            let (rows, _) = store
                .get_data_table_rows(&table_id, &filter, sort.as_ref().map(|(c, d)| (c.as_str(), d.as_str())), 0, limit)
                .await
                .map_err(dt_err)?;
            Ok(rows.into_iter().map(row_item).collect())
        }
        "update" => {
            let filter = read_filter(ctx, i)?;
            if filter.conditions.is_empty() {
                return Err(NodeError::new("At least one condition is required"));
            }
            let row = read_row_data(ctx, i, item)?;
            let updated = store.update_data_table_rows(&table_id, &filter, &row, true, dry_run).await.map_err(dt_err)?;
            Ok(updated.into_iter().map(row_item).collect())
        }
        "upsert" => {
            let filter = read_filter(ctx, i)?;
            if filter.conditions.is_empty() {
                return Err(NodeError::new("At least one condition is required"));
            }
            let row = read_row_data(ctx, i, item)?;
            let result = store.upsert_data_table_row(&table_id, &filter, &row, true, dry_run).await.map_err(dt_err)?;
            Ok(result.into_iter().map(row_item).collect())
        }
        "deleteRows" => {
            let filter = read_filter(ctx, i)?;
            if filter.conditions.is_empty() {
                return Err(NodeError::new("At least one condition is required"));
            }
            let result = store.delete_data_table_rows(&table_id, &filter, true, dry_run).await.map_err(dt_err)?;
            Ok(result.into_iter().map(row_item).collect())
        }
        "rowExists" | "rowNotExists" => {
            let filter = read_filter(ctx, i)?;
            let (rows, _) = store.get_data_table_rows(&table_id, &filter, None, 0, 1).await.map_err(dt_err)?;
            let hit = !rows.is_empty();
            let keep = if operation == "rowExists" { hit } else { !hit };
            Ok(if keep { vec![item.clone()] } else { Vec::new() })
        }
        other => Err(NodeError::new(format!("Unknown operation \"{other}\""))),
    }
}

async fn execute_table(ctx: &mut ExecCtx<'_>, store: &Store, project: &Option<String>, operation: &str) -> NodeResult<NodeOutput> {
    match operation {
        "list" => {
            let (rows, _) = store.list_data_tables(project.as_deref(), None, None, None, 0, 250).await.map_err(dt_err)?;
            Ok(vec![rows.into_iter().map(row_item).collect()])
        }
        "create" => {
            let name = ctx.param_str("name", 0, "")?;
            let columns_param = ctx.param("columns", 0)?;
            let columns: Vec<(String, String)> = columns_param
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| Some((c["name"].as_str()?.to_string(), c["type"].as_str()?.to_string())))
                .collect();
            let table = store.create_data_table(project.as_deref(), &name, &columns).await.map_err(dt_err)?;
            Ok(vec![vec![row_item(table)]])
        }
        "delete" => {
            let table_id = resolve_table_id(ctx, store, project, 0).await?;
            store.delete_data_table(&table_id, project.as_deref()).await.map_err(dt_err)?;
            Ok(vec![vec![row_item(json!({"success": true}))]])
        }
        "clear" => {
            let table_id = resolve_table_id(ctx, store, project, 0).await?;
            store.clear_data_table(&table_id).await.map_err(dt_err)?;
            Ok(vec![vec![row_item(json!({"success": true}))]])
        }
        "update" => {
            let table_id = resolve_table_id(ctx, store, project, 0).await?;
            let name = ctx.param_str("name", 0, "")?;
            let table = store.rename_data_table(&table_id, project.as_deref(), &name).await.map_err(dt_err)?;
            Ok(vec![vec![row_item(table)]])
        }
        other => Err(NodeError::new(format!("Unknown operation \"{other}\""))),
    }
}

fn dt_err(e: data_table::DtError) -> NodeError {
    NodeError::new(e.message)
}
