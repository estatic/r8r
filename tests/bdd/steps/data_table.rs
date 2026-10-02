//! Seeds a Data Table before a headless node scenario runs. Headless
//! `r8r execute` (see `cli.rs`'s `execute_current`) has no running server
//! to call the REST API through, so this goes via the `r8r data-table`
//! CLI subcommand instead, against the same per-scenario database.

use super::docstring;
use crate::support::json::parse_strict;
use crate::world::R8rWorld;
use cucumber::gherkin::Step;
use cucumber::given;
use serde_json::Value;

async fn seed(w: &mut R8rWorld, name: String, body: &str) {
    let mut spec = parse_strict(&w.expand(body), "data table spec");
    spec["name"] = Value::String(name.clone());
    let file = w.dir.path().join(format!("data-table-{name}.json"));
    std::fs::write(&file, serde_json::to_vec(&spec).unwrap()).unwrap();
    let args = vec!["data-table".to_string(), format!("--input={}", file.display())];
    let out = w.cli(&args, std::time::Duration::from_secs(30)).await;
    assert_eq!(out.code, Some(0), "r8r data-table failed:\n{}\n{}", out.stdout, out.stderr);
    let table: Value = serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("data-table output is not JSON: {e}\n{}", out.stdout));
    let id = table["id"].as_str().unwrap().to_string();
    w.vars.insert("DATA_TABLE_ID".into(), id.clone());
    w.vars.insert(format!("DATA_TABLE_ID:{name}"), id);
}

/// `{"columns": [...], "rows": [...]}` (both optional); remembers the
/// created table's id as `DATA_TABLE_ID` and `DATA_TABLE_ID:<name>`.
#[given(expr = "a data table {string} defined as:")]
async fn data_table_defined(w: &mut R8rWorld, name: String, step: &Step) {
    seed(w, name, docstring(step)).await;
}

/// Shorthand for a table with no columns and no rows (table-resource
/// scenarios that create/rename/delete/list/clear tables themselves).
#[given(expr = "an empty data table {string}")]
async fn empty_data_table(w: &mut R8rWorld, name: String) {
    seed(w, name, "{}").await;
}
