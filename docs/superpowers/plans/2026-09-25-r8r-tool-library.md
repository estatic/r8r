# Tool Library and AI Agent Settings Form (B1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reusable, id-referenced agent tools with `{{ $args.x }}` templates and run-time credential resolution, plus an AI Agent settings form.

**Architecture:** `$args` joins the expression engine. A `Tool` entity (table `tools`, `src/tools.rs` validation) with `/rest/tools`. A `RunResources { credentials, tools }` replaces the bare credentials map through runner and engine, resolved on every run path (fixing schedule/webhook credentials). `ai.agent` merges library tools (`tool_ids`, template-resolved) with inline tools (unchanged). Frontend adds a Tools page and an agent form in the node panel.

**Tech Stack:** Rust (axum, sqlx/SQLite, rquickjs), Vue 3 + TypeScript + Pinia, Vitest.

**Spec:** `docs/superpowers/specs/2026-09-25-r8r-tool-library-design.md`

## Global Constraints

- Tool name `^[A-Za-z0-9_-]{1,64}$`, unique. `node_type` ∈ `core.httpRequest`, `telegram.sendMessage`, `core.code`. `argument_schema.type == "object"`, property types ∈ string/number/integer/boolean, `required` ⊆ properties. `parameters` is an object.
- `$args` is data (bound like `$json`), `undefined` outside tool calls.
- Library tool calls: validate args (existing validator incl. denied keys) → resolve templates → call node. No key-merge. Template error → `is_error` tool result.
- Inline `parameters.tools` behaviour unchanged. Duplicate tool name across inline + library → execution error naming it.
- Every run path resolves `RunResources`; manual execute keeps its 400 body prefix `credential resolution failed: `.
- Tool DELETE in use → 409 `{ "error": "tool is in use", "workflows": [{id, name}] }`. Credential DELETE 409 gains `tools: [{id, name}]`; credential `used_by` = workflows + tools.
- Agent form: model and user message required; max iterations integer 1–50 (default 10).

## Review Focus

- An agent whose `tool_ids` names a deleted/missing tool must fail the run with a clear message naming the tool, not panic — pinned in Task 3 (`missing_tool_is_an_error_naming_it`).
- A model argument containing `{{ ... }}` or JS source must stay literal — pinned in Task 1 (`args_values_are_data_not_code`).
- Schedule- and webhook-triggered runs of a workflow whose node uses a credential must now authenticate — pinned in Task 3 (`fire_schedule_resolves_credentials`) and Task 5 (webhook test).
- Opening an existing agent node that has only raw JSON (pre-form) must show its values in the form, not blank them on Apply — pinned in Task 7 (`loads existing agent parameters into the form`).
- A credential used only by a tool must not be deletable — pinned in Task 5.

---

### Task 1: `$args` in the expression engine

**Files:** Modify `src/expr.rs` (struct `EvalContext` line 14; bindings ~line 103; tests module incl. `empty_ctx()` ~line 270); Modify `src/engine.rs:212` and `src/nodes/code.rs:89` (add `args: None`).

**Interfaces:** Produces `EvalContext.args: Option<&'a serde_json::Value>` bound as `$args`.

- [ ] **Step 1: Failing tests** — inside `mod tests` in `src/expr.rs`:

```rust
    #[test]
    fn args_resolve_inside_a_template() {
        let args = serde_json::json!({"city": "Warsaw"});
        let ctx = EvalContext { args: Some(&args), ..empty_ctx() };
        let params = serde_json::json!({"url": "https://api.example.com/weather?city={{ $args.city }}"});
        assert_eq!(
            resolve_parameters(&params, &ctx).unwrap(),
            serde_json::json!({"url": "https://api.example.com/weather?city=Warsaw"})
        );
    }

    #[test]
    fn args_is_undefined_without_args() {
        let ctx = empty_ctx();
        let params = serde_json::json!({"t": "{{ typeof $args }}"});
        assert_eq!(resolve_parameters(&params, &ctx).unwrap(), serde_json::json!({"t": "undefined"}));
    }

    #[test]
    fn args_values_are_data_not_code() {
        let args = serde_json::json!({"x": "{{ 1 + 1 }}", "y": "'); throw new Error('pwned'); ('"});
        let ctx = EvalContext { args: Some(&args), ..empty_ctx() };
        let params = serde_json::json!({"a": "{{ $args.x }}", "b": "{{ $args.y }}"});
        assert_eq!(
            resolve_parameters(&params, &ctx).unwrap(),
            serde_json::json!({"a": "{{ 1 + 1 }}", "b": "'); throw new Error('pwned'); ('"})
        );
    }
```

- [ ] **Step 2: Run** `cargo test --lib expr 2>&1 | grep -E "^error\[" | sort | uniq -c` — Expected: `struct EvalContext has no field named args`.

- [ ] **Step 3: Implement** — add to `EvalContext`:

```rust
    /// Bound as `$args`: a tool call's arguments (library tools only).
    /// `None` leaves `$args` undefined.
    pub args: Option<&'a serde_json::Value>,
```

After the `$workflow` binding in the eval function:

```rust
        if let Some(args) = ctx.args {
            let args_val = json_to_js(&js, args)?;
            globals
                .set("$args", args_val)
                .map_err(|e| ExprError::Runtime(e.to_string()))?;
        }
```

Add `args: None,` to `empty_ctx()` and to the struct literals at `src/engine.rs:212` and `src/nodes/code.rs:89` (any other literal the compiler flags too). Update the module doc line listing globals to include `$args`.

- [ ] **Step 4: Run** `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"` — all pass.

- [ ] **Step 5: Commit** `git add src/expr.rs src/engine.rs src/nodes/code.rs && git commit -m "feat: \$args binding in the expression engine"`

---

### Task 2: Tool entity, validation, migration, storage

**Files:** Modify `src/domain.rs`; Create `src/tools.rs` (+ `pub mod tools;` in `src/lib.rs`); Create `migrations/0004_tools.sql`; Modify `src/storage/mod.rs`, `src/storage/sqlite.rs`; the three test `Storage` wrappers (`src/execution_runner.rs`, `src/telegram_poller.rs`, `tests/api_test.rs`).

**Interfaces:** Produces `crate::domain::Tool` (derive `Debug, Clone, Serialize, Deserialize, PartialEq`); `crate::tools::{ALLOWED_TOOL_NODE_TYPES, validate_tool(&Tool) -> Result<(), String>}`; `Storage::{create_tool(&Tool) -> Result<()>, get_tool(Uuid) -> Result<Option<Tool>>, list_tools() -> Result<Vec<Tool>>, update_tool(&Tool) -> Result<bool>, delete_tool(Uuid) -> Result<bool>}`.

- [ ] **Step 1: Failing tests** — `src/tools.rs`:

```rust
//! Library tools for `ai.agent` (spec B1 §3): validation shared by the
//! create and update endpoints.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Tool;

    fn tool() -> Tool {
        Tool {
            id: uuid::Uuid::new_v4(),
            name: "get_weather".into(),
            description: "Weather for a city".into(),
            node_type: "core.httpRequest".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}),
            parameters: serde_json::json!({"method": "GET", "url": "https://x/?q={{ $args.city }}"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn accepts_a_well_formed_tool() {
        assert_eq!(validate_tool(&tool()), Ok(()));
    }

    #[test]
    fn rejects_bad_names() {
        for bad in ["", "has space", "dots.not.ok", &"x".repeat(65)] {
            let mut t = tool();
            t.name = bad.to_string();
            assert!(validate_tool(&t).unwrap_err().contains("name"), "{bad:?}");
        }
    }

    #[test]
    fn rejects_disallowed_node_types() {
        for bad in ["ai.agent", "core.set", "telegram.trigger"] {
            let mut t = tool();
            t.node_type = bad.into();
            assert!(validate_tool(&t).unwrap_err().contains("node_type"), "{bad}");
        }
    }

    #[test]
    fn rejects_malformed_schemas_and_parameters() {
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "array"});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "object", "properties": {"x": {"type": "object"}}});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.argument_schema = serde_json::json!({"type": "object", "properties": {}, "required": ["ghost"]});
        assert!(validate_tool(&t).is_err());
        let mut t = tool();
        t.parameters = serde_json::json!("not an object");
        assert!(validate_tool(&t).is_err());
    }
}
```

In `src/storage/sqlite.rs` tests:

```rust
    fn sample_tool(name: &str) -> crate::domain::Tool {
        crate::domain::Tool {
            id: Uuid::new_v4(),
            name: name.into(),
            description: "d".into(),
            node_type: "core.code".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {}}),
            parameters: serde_json::json!({"script": "return []"}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn tool_crud_round_trips() {
        let storage = storage_with_test_key().await;
        let tool = sample_tool("t1");
        storage.create_tool(&tool).await.unwrap();
        assert_eq!(storage.get_tool(tool.id).await.unwrap().unwrap().parameters, tool.parameters);
        assert_eq!(storage.list_tools().await.unwrap().len(), 1);
        let mut changed = tool.clone();
        changed.description = "changed".into();
        assert!(storage.update_tool(&changed).await.unwrap());
        assert_eq!(storage.get_tool(tool.id).await.unwrap().unwrap().description, "changed");
        assert!(storage.delete_tool(tool.id).await.unwrap());
        assert!(storage.get_tool(tool.id).await.unwrap().is_none());
        assert!(!storage.update_tool(&changed).await.unwrap());
        assert!(!storage.delete_tool(tool.id).await.unwrap());
    }

    #[tokio::test]
    async fn tool_names_are_unique() {
        let storage = storage_with_test_key().await;
        storage.create_tool(&sample_tool("same")).await.unwrap();
        assert!(storage.create_tool(&sample_tool("same")).await.is_err());
    }
```

- [ ] **Step 2: Run** `cargo test --lib 2>&1 | grep -E "^error\[" | sort | uniq -c` — Expected: `Tool` / `validate_tool` / tool storage methods not found.

- [ ] **Step 3: Implement.** `src/domain.rs`:

```rust
/// A reusable `ai.agent` tool (spec B1 §3). `parameters` are the called
/// node's fixed parameters and may contain `{{ $args.<name> }}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub node_type: String,
    pub argument_schema: serde_json::Value,
    pub parameters: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

`src/tools.rs` (above the tests):

```rust
use crate::domain::Tool;

/// Node types a library tool may call.
pub const ALLOWED_TOOL_NODE_TYPES: [&str; 3] = ["core.httpRequest", "telegram.sendMessage", "core.code"];

const ARGUMENT_TYPES: [&str; 4] = ["string", "number", "integer", "boolean"];

pub fn validate_tool(tool: &Tool) -> Result<(), String> {
    let name_ok = !tool.name.is_empty()
        && tool.name.len() <= 64
        && tool.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !name_ok {
        return Err("name must be 1-64 characters of letters, digits, '_' or '-'".into());
    }
    if !ALLOWED_TOOL_NODE_TYPES.contains(&tool.node_type.as_str()) {
        return Err(format!("node_type must be one of {}", ALLOWED_TOOL_NODE_TYPES.join(", ")));
    }
    let schema = &tool.argument_schema;
    if schema.get("type").and_then(|v| v.as_str()) != Some("object") {
        return Err("argument_schema.type must be \"object\"".into());
    }
    let empty = serde_json::Map::new();
    let properties = match schema.get("properties") {
        None => &empty,
        Some(p) => p.as_object().ok_or("argument_schema.properties must be an object")?,
    };
    for (key, prop) in properties {
        let ty = prop.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if !ARGUMENT_TYPES.contains(&ty) {
            return Err(format!("argument \"{key}\" type must be one of {}", ARGUMENT_TYPES.join(", ")));
        }
    }
    if let Some(required) = schema.get("required") {
        let required = required.as_array().ok_or("argument_schema.required must be an array")?;
        for r in required {
            let r = r.as_str().ok_or("argument_schema.required entries must be strings")?;
            if !properties.contains_key(r) {
                return Err(format!("required argument \"{r}\" is not declared in properties"));
            }
        }
    }
    if !tool.parameters.is_object() {
        return Err("parameters must be a JSON object".into());
    }
    Ok(())
}
```

`migrations/0004_tools.sql`: the `CREATE TABLE tools (...)` from spec §3.3.

`Storage` trait additions (after `delete_credential`):

```rust
    async fn create_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<()>;
    async fn get_tool(&self, id: Uuid) -> anyhow::Result<Option<crate::domain::Tool>>;
    async fn list_tools(&self) -> anyhow::Result<Vec<crate::domain::Tool>>;
    /// `Ok(false)` if no such tool.
    async fn update_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<bool>;
    /// `Ok(false)` if no such tool.
    async fn delete_tool(&self, id: Uuid) -> anyhow::Result<bool>;
```

SQLite impl:

```rust
    async fn create_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO tools (id, name, description, node_type, argument_schema, parameters, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
            .bind(tool.id.to_string())
            .bind(&tool.name)
            .bind(&tool.description)
            .bind(&tool.node_type)
            .bind(tool.argument_schema.to_string())
            .bind(tool.parameters.to_string())
            .bind(tool.created_at.to_rfc3339())
            .bind(tool.updated_at.to_rfc3339())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_tool(&self, id: Uuid) -> anyhow::Result<Option<crate::domain::Tool>> {
        let row = sqlx::query_as::<_, ToolRow>("SELECT id, name, description, node_type, argument_schema, parameters, created_at, updated_at FROM tools WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;
        row.map(row_to_tool).transpose()
    }

    async fn list_tools(&self) -> anyhow::Result<Vec<crate::domain::Tool>> {
        let rows = sqlx::query_as::<_, ToolRow>("SELECT id, name, description, node_type, argument_schema, parameters, created_at, updated_at FROM tools ORDER BY name")
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter().map(row_to_tool).collect()
    }

    async fn update_tool(&self, tool: &crate::domain::Tool) -> anyhow::Result<bool> {
        let result = sqlx::query("UPDATE tools SET name = ?, description = ?, node_type = ?, argument_schema = ?, parameters = ?, updated_at = ? WHERE id = ?")
            .bind(&tool.name)
            .bind(&tool.description)
            .bind(&tool.node_type)
            .bind(tool.argument_schema.to_string())
            .bind(tool.parameters.to_string())
            .bind(tool.updated_at.to_rfc3339())
            .bind(tool.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_tool(&self, id: Uuid) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM tools WHERE id = ?").bind(id.to_string()).execute(&self.pool).await?;
        Ok(result.rows_affected() > 0)
    }
```

and at module level:

```rust
type ToolRow = (String, String, String, String, String, String, String, String);

fn row_to_tool(row: ToolRow) -> anyhow::Result<crate::domain::Tool> {
    let (id, name, description, node_type, argument_schema, parameters, created_at, updated_at) = row;
    Ok(crate::domain::Tool {
        id: Uuid::parse_str(&id)?,
        name,
        description,
        node_type,
        argument_schema: serde_json::from_str(&argument_schema)?,
        parameters: serde_json::from_str(&parameters)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}
```

Each test wrapper gets five delegating methods (`self.inner.<method>(...).await`), following its existing `update_credential` delegation.

- [ ] **Step 4: Run** `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"` — all pass.

- [ ] **Step 5: Commit** `git add -A src migrations tests && git commit -m "feat: Tool entity, validation, migration and storage"`

---

### Task 3: `RunResources` threaded through every run

**Files:** Modify `src/credentials.rs` (replace `resolve_credentials_for_workflow`), `src/node.rs` (`NodeExecutionContext`), `src/engine.rs`, `src/execution_runner.rs`, `src/api/workflows.rs` (execute), `src/api/webhook.rs`, `src/triggers.rs` (`fire_schedule`), `src/telegram_poller.rs` (batch resolution + `TelegramJob`), plus every test call site the compiler flags.

**Interfaces:**
- Produces `crate::credentials::RunResources { pub credentials: HashMap<Uuid, serde_json::Value>, pub tools: HashMap<Uuid, crate::domain::Tool> }` (derive `Debug, Clone, Default`), `pub async fn resolve_run_resources(storage: &dyn Storage, workflow: &Workflow) -> anyhow::Result<RunResources>`, `NodeExecutionContext.tools: HashMap<Uuid, Tool>`.
- Signatures change: `execute_workflow_seeded(..., resources: &RunResources, observer)`, `start_execution(..., resources: RunResources)`, `run_and_track_execution(..., resources: &RunResources)`; `TelegramJob.resources: RunResources` (replaces `credentials`).

- [ ] **Step 1: Failing tests** — in `src/credentials.rs` tests:

```rust
    async fn storage() -> std::sync::Arc<dyn Storage> {
        std::sync::Arc::new(crate::storage::sqlite::SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap())
    }

    async fn seed_credential(storage: &dyn Storage, data: serde_json::Value) -> Uuid {
        let owner = crate::domain::User {
            id: Uuid::new_v4(),
            email: format!("{}@x.io", Uuid::new_v4()),
            password_hash: "x".into(),
            role: crate::domain::UserRole::Owner,
            created_at: chrono::Utc::now(),
        };
        storage.create_user(&owner).await.unwrap();
        let id = Uuid::new_v4();
        storage
            .create_credential(&crate::domain::Credential {
                id,
                name: "c".into(),
                credential_type: "bearerToken".into(),
                data,
                owner_id: owner.id,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        id
    }

    fn agent_wf(tool_ids: Vec<Uuid>) -> crate::domain::Workflow {
        let mut wf = wf_with_auth("agent", None);
        wf.nodes[0].node_type = "ai.agent".into();
        wf.nodes[0].parameters = serde_json::json!({"tool_ids": tool_ids.iter().map(|i| i.to_string()).collect::<Vec<_>>()});
        wf
    }

    #[tokio::test]
    async fn resolves_agent_tools_and_their_credentials() {
        let storage = storage().await;
        let cred = seed_credential(storage.as_ref(), serde_json::json!({"token": "t"})).await;
        let tool = crate::domain::Tool {
            id: Uuid::new_v4(),
            name: "search".into(),
            description: "d".into(),
            node_type: "core.httpRequest".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {}}),
            parameters: serde_json::json!({"url": "https://x", "auth": {"type": "bearer", "credential_id": cred.to_string()}}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        storage.create_tool(&tool).await.unwrap();
        let resources = resolve_run_resources(storage.as_ref(), &agent_wf(vec![tool.id])).await.unwrap();
        assert_eq!(resources.tools.get(&tool.id).unwrap().name, "search");
        assert_eq!(resources.credentials.get(&cred), Some(&serde_json::json!({"token": "t"})));
    }

    #[tokio::test]
    async fn missing_tool_is_an_error_naming_it() {
        let storage = storage().await;
        let ghost = Uuid::new_v4();
        let err = resolve_run_resources(storage.as_ref(), &agent_wf(vec![ghost])).await.unwrap_err().to_string();
        assert!(err.contains(&ghost.to_string()) && err.contains("tool"), "{err}");
    }
```

In `src/triggers.rs` tests (uses its `test_state()` / `schedule_workflow()`):

```rust
    #[tokio::test]
    async fn fire_schedule_resolves_credentials() {
        let state = test_state().await;
        let owner = crate::domain::User {
            id: Uuid::new_v4(), email: "o@x.io".into(), password_hash: "x".into(),
            role: crate::domain::UserRole::Owner, created_at: chrono::Utc::now(),
        };
        state.storage.create_user(&owner).await.unwrap();
        let cred_id = Uuid::new_v4();
        state.storage.create_credential(&crate::domain::Credential {
            id: cred_id, name: "c".into(), credential_type: "telegramApi".into(),
            data: serde_json::json!({"bot_token": "1:A"}), owner_id: owner.id,
            created_at: chrono::Utc::now(), updated_at: chrono::Utc::now(),
        }).await.unwrap();
        // A code node that fails unless its credential was resolved is not
        // available; instead reference a credential that DOESN'T exist and
        // assert no execution starts (resolution ran and refused), then one
        // that does and assert the run succeeds.
        let mut wf = schedule_workflow("* * * * * *");
        wf.nodes[1].parameters = serde_json::json!({"fields": {"fired": true}, "auth": {"credential_id": Uuid::new_v4().to_string()}});
        state.storage.create_workflow(&wf).await.unwrap();
        fire_schedule(state.storage.clone(), state.registry.clone(), state.execution_events.clone(), wf.id).await;
        assert!(state.storage.list_executions_for_workflow(wf.id, 10).await.unwrap().is_empty(), "a run with an unresolvable credential must not start");

        let mut wf2 = schedule_workflow("* * * * * *");
        wf2.nodes[1].parameters = serde_json::json!({"fields": {"fired": true}, "auth": {"credential_id": cred_id.to_string()}});
        state.storage.create_workflow(&wf2).await.unwrap();
        fire_schedule(state.storage.clone(), state.registry.clone(), state.execution_events.clone(), wf2.id).await;
        let runs = state.storage.list_executions_for_workflow(wf2.id, 10).await.unwrap();
        assert_eq!(runs.len(), 1);
    }
```

- [ ] **Step 2: Run** `cargo test --lib 2>&1 | grep -E "^error\[|FAILED" | head` — Expected: `resolve_run_resources` not found (compile error).

- [ ] **Step 3: Implement `RunResources`** in `src/credentials.rs`, replacing `resolve_credentials_for_workflow`:

```rust
/// Everything a run needs from storage beyond the workflow itself: the
/// decrypted credentials its nodes and its agents' tools reference, and
/// the library tools its agents use (spec B1 §5).
#[derive(Debug, Clone, Default)]
pub struct RunResources {
    pub credentials: HashMap<Uuid, serde_json::Value>,
    pub tools: HashMap<Uuid, crate::domain::Tool>,
}

fn credential_ref(parameters: &serde_json::Value) -> Option<&str> {
    parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str())
}

pub async fn resolve_run_resources(storage: &dyn Storage, workflow: &Workflow) -> anyhow::Result<RunResources> {
    let mut tool_ids = std::collections::HashSet::new();
    for node in workflow.nodes.iter().filter(|n| n.node_type == "ai.agent") {
        if let Some(ids) = node.parameters.get("tool_ids").and_then(|v| v.as_array()) {
            for v in ids {
                let s = v.as_str().ok_or_else(|| anyhow::anyhow!("node {} has a non-string entry in tool_ids", node.id))?;
                let id = Uuid::parse_str(s).map_err(|e| anyhow::anyhow!("node {} has an invalid tool id {s}: {e}", node.id))?;
                tool_ids.insert(id);
            }
        }
    }
    let mut tools = HashMap::new();
    for id in tool_ids {
        let tool = storage.get_tool(id).await?.ok_or_else(|| anyhow::anyhow!("referenced tool {id} does not exist"))?;
        tools.insert(id, tool);
    }

    let mut credential_ids = std::collections::HashSet::new();
    for node in &workflow.nodes {
        if let Some(id_str) = credential_ref(&node.parameters) {
            let id = Uuid::parse_str(id_str).map_err(|e| anyhow::anyhow!("node {} has an invalid credential_id: {e}", node.id))?;
            credential_ids.insert(id);
        }
    }
    for tool in tools.values() {
        if let Some(id_str) = credential_ref(&tool.parameters) {
            let id = Uuid::parse_str(id_str).map_err(|e| anyhow::anyhow!("tool {} has an invalid credential_id: {e}", tool.name))?;
            credential_ids.insert(id);
        }
    }
    let mut credentials = HashMap::new();
    for id in credential_ids {
        let credential = storage.get_credential(id).await?.ok_or_else(|| anyhow::anyhow!("referenced credential {id} does not exist"))?;
        credentials.insert(id, credential.data);
    }
    Ok(RunResources { credentials, tools })
}
```

Refactor `workflows_using_credential` to use `credential_ref` (behaviour unchanged).

- [ ] **Step 4: Thread it.**
  - `src/node.rs`: add `pub tools: std::collections::HashMap<uuid::Uuid, crate::domain::Tool>,` to `NodeExecutionContext`.
  - `src/engine.rs`: `execute_workflow_seeded(workflow, registry, trigger_items, resources: &crate::credentials::RunResources, observer)`; `EngineToolExecutor::new(registry.clone(), resources.credentials.clone())`; the node `ctx` gets `credentials: resources.credentials.clone(), tools: resources.tools.clone()`; `call_tool`'s ctx gets `tools: Default::default()`; `execute_workflow` passes `&Default::default()`.
  - `src/execution_runner.rs`: `start_execution(..., resources: RunResources)` passing `&resources` to the engine; `run_and_track_execution(..., resources: &RunResources)` passing `resources.clone()`.
  - `src/api/workflows.rs` execute: `crate::credentials::resolve_run_resources(...)`, keep the `credential resolution failed: {e}` 400 text, pass the resources.
  - `src/api/webhook.rs`: before `start_execution`, resolve; on error `tracing::warn!` and return `(StatusCode::INTERNAL_SERVER_ERROR, format!("credential resolution failed: {e}"))`; pass resources.
  - `src/triggers.rs::fire_schedule`: resolve before running; on error `tracing::warn!(error = %e, %workflow_id, "fire_schedule: failed to resolve credentials, skipping this firing")` and `return`; pass `&resources`.
  - `src/telegram_poller.rs`: the per-batch `resolve_credentials_for_workflow` becomes `resolve_run_resources` (same backoff on error); `TelegramJob { item, workflow, resources }`; the worker passes `job.resources`.
  - Every remaining compile error in tests: `&HashMap::new()` / `&std::collections::HashMap::new()` → `&Default::default()`, owned `HashMap::new()` → `Default::default()`, `credentials: HashMap::new()` in `TelegramJob` literals → `resources: Default::default()`. Struct literals of `NodeExecutionContext` without `..Default::default()` get `tools: Default::default()`.

- [ ] **Step 5: Run** `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"` — all pass, including the three new tests.

- [ ] **Step 6: Commit** `git add -A src tests && git commit -m "feat: RunResources (credentials + tools) resolved on every run path"`

---

### Task 4: Library tools in `ai.agent`

**Files:** Modify `src/nodes/agent.rs` (`ToolDecl`, `run_agent_loop`, tests).

**Interfaces:** Consumes `ctx.tools`, `EvalContext.args`. `ToolDecl` gains `template: bool`.

- [ ] **Step 1: Failing tests** — in `agent.rs` tests, reusing the module's existing mock provider and recording tool executor (see `an_undeclared_argument_key_is_not_merged_into_tool_parameters` ~line 418 for their shapes). Add:

```rust
    fn library_tool(name: &str) -> crate::domain::Tool {
        crate::domain::Tool {
            id: uuid::Uuid::new_v4(),
            name: name.into(),
            description: "search the web".into(),
            node_type: "core.httpRequest".into(),
            argument_schema: serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]}),
            parameters: serde_json::json!({"method": "GET", "url": "https://s.example/?q={{ $args.q }}", "q": "fixed"}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
```

Tests (write each with the same mock-provider script pattern the existing tool-call tests use: first response = one tool call, second = final text; the recording executor captures `(node_type, parameters)`):
  - `library_tool_resolves_args_templates_without_key_merge` — tool call `search {"q": "rust"}`; assert recorded parameters == `{"method":"GET","url":"https://s.example/?q=rust","q":"fixed"}` (template filled, `q` NOT overwritten by the argument).
  - `library_tool_template_error_is_reported_to_the_model` — tool whose `parameters` is `{"url": "{{ $args.q.missing.deep }}"}`; assert the executor was not called and the second provider request contains a tool result with `is_error: true`.
  - `inline_and_library_tools_with_the_same_name_are_rejected` — inline `tools: [{"name":"search", "node_type":"core.code"}]` + library `search`; `run_agent_loop` returns `Err` containing `duplicate tool name "search"`.
  - `inline_tools_still_merge_arguments` — the existing inline tests stay green (no change).

Library tools are supplied via the ctx: `NodeExecutionContext { parameters: json!({"tool_ids": [tool.id.to_string()]}), tools: HashMap::from([(tool.id, tool.clone())]), tool_executor: Some(...), ..Default::default() }`.

- [ ] **Step 2: Run** `cargo test --lib nodes::agent 2>&1 | grep -E "^test |test result|^error"` — new tests FAIL (library tools not offered).

- [ ] **Step 3: Implement.** Add `template: bool` to `ToolDecl` (`false` in `parse_tools`). In `run_agent_loop`, after `let tool_decls = parse_tools(tools_param)?;` make it `let mut tool_decls = ...;` and append:

```rust
    if let Some(ids) = ctx.parameters.get("tool_ids").and_then(|v| v.as_array()) {
        for id in ids.iter().filter_map(|v| v.as_str()).filter_map(|s| uuid::Uuid::parse_str(s).ok()) {
            let tool = ctx
                .tools
                .get(&id)
                .ok_or_else(|| NodeError::ExecutionFailed(format!("ai.agent: tool {id} was not resolved for this run")))?;
            if tool_decls.iter().any(|t| t.name == tool.name) {
                return Err(NodeError::ExecutionFailed(format!("ai.agent: duplicate tool name \"{}\"", tool.name)));
            }
            tool_decls.push(ToolDecl {
                name: tool.name.clone(),
                description: tool.description.clone(),
                node_type: tool.node_type.clone(),
                argument_schema: tool.argument_schema.clone(),
                base_parameters: tool.parameters.clone(),
                template: true,
            });
        }
    }
```

Replace the `let mut merged_parameters = ...` block with:

```rust
                    let parameters = if decl.template {
                        // Library tool: arguments reach the node only where
                        // the tool author placed {{ $args.x }}.
                        let no_items: Vec<serde_json::Value> = Vec::new();
                        let no_nodes = std::collections::HashMap::new();
                        let eval_ctx = crate::expr::EvalContext {
                            json: serde_json::json!({}),
                            items: &no_items,
                            node_json: &no_nodes,
                            workflow_name: "",
                            args: Some(&call.arguments),
                        };
                        match crate::expr::resolve_parameters(&decl.base_parameters, &eval_ctx) {
                            Ok(p) => p,
                            Err(e) => {
                                messages.push(LlmMessage::ToolResult {
                                    tool_call_id: call.id.clone(),
                                    content: format!("tool parameter template failed: {e}"),
                                    is_error: true,
                                });
                                continue;
                            }
                        }
                    } else {
                        let mut merged_parameters = decl.base_parameters.clone();
                        // (existing inline merge loop, unchanged)
                        merged_parameters
                    };
                    match tool_executor.call_tool(&decl.node_type, parameters).await {
```

(Move the existing merge loop, with its comment, into the `else` branch verbatim.)

- [ ] **Step 4: Run** `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"` — all pass.

- [ ] **Step 5: Commit** `git add src/nodes/agent.rs && git commit -m "feat: ai.agent calls library tools with \$args templates"`

---

### Task 5: Tools API and credential usage through tools

**Files:** Create `src/api/tools.rs` (+ `pub mod tools;` and routes in `src/api/mod.rs`); Modify `src/credentials.rs` (`tools_using_credential`, `workflows_using_tool`), `src/api/credentials.rs` (used_by + 409 include tools); Test `tests/api_test.rs`.

**Interfaces:** Produces `/rest/tools` per spec §4; `crate::credentials::{tools_using_credential(&[Tool], Uuid) -> Vec<(Uuid, String)>, workflows_using_tool(&[Workflow], Uuid) -> Vec<(Uuid, String)>}`.

- [ ] **Step 1: Failing tests** — append to `tests/api_test.rs` (reusing `send`, `create_cred` from the credential tests):

```rust
fn tool_body(name: &str, cred: Option<&str>) -> serde_json::Value {
    let mut parameters = serde_json::json!({"method": "GET", "url": "https://s.example/?q={{ $args.q }}"});
    if let Some(c) = cred {
        parameters["auth"] = serde_json::json!({"type": "bearer", "credential_id": c});
    }
    serde_json::json!({
        "name": name,
        "description": "search",
        "node_type": "core.httpRequest",
        "argument_schema": {"type": "object", "properties": {"q": {"type": "string"}}, "required": ["q"]},
        "parameters": parameters
    })
}

#[tokio::test]
async fn tool_crud_and_validation() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "tools@example.com").await;
    let (s, created) = send(&app, "POST", "/rest/tools", &token, Some(tool_body("search", None))).await;
    assert_eq!(s, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    let (s, _) = send(&app, "POST", "/rest/tools", &token, Some(tool_body("search", None))).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = send(&app, "POST", "/rest/tools", &token, Some(tool_body("bad name", None))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, list) = send(&app, "GET", "/rest/tools", &token, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list[0]["used_by"], 0);
    let (s, patched) = send(&app, "PATCH", &format!("/rest/tools/{id}"), &token, Some(serde_json::json!({"description": "web search"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(patched["description"], "web search");
    let (s, _) = send(&app, "PATCH", &format!("/rest/tools/{id}"), &token, Some(serde_json::json!({"node_type": "ai.agent"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "GET", &format!("/rest/tools/{}", uuid::Uuid::new_v4()), &token, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn tool_delete_is_refused_while_an_agent_uses_it() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "tools-del@example.com").await;
    let (_, tool) = send(&app, "POST", "/rest/tools", &token, Some(tool_body("lookup", None))).await;
    let tool_id = tool["id"].as_str().unwrap().to_string();
    let wf = serde_json::json!({
        "name": "agent-wf",
        "nodes": [{"id": "a", "node_type": "ai.agent", "position": [0.0, 0.0], "parameters": {"tool_ids": [tool_id]}}],
        "connections": []
    });
    let (s, wf) = send(&app, "POST", "/rest/workflows", &token, Some(wf)).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, body) = send(&app, "DELETE", &format!("/rest/tools/{tool_id}"), &token, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["workflows"][0]["name"], "agent-wf");
    let wf_id = wf["id"].as_str().unwrap();
    send(&app, "DELETE", &format!("/rest/workflows/{wf_id}"), &token, None).await;
    let (s, _) = send(&app, "DELETE", &format!("/rest/tools/{tool_id}"), &token, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn a_credential_used_only_by_a_tool_cannot_be_deleted() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "tools-cred@example.com").await;
    let cred = create_cred(&app, &token, "api", "bearerToken", serde_json::json!({"token": "t"})).await;
    let (s, _) = send(&app, "POST", "/rest/tools", &token, Some(tool_body("secured", Some(&cred)))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, list) = send(&app, "GET", "/rest/credentials", &token, None).await;
    assert_eq!(list.as_array().unwrap().iter().find(|c| c["id"] == cred.as_str()).unwrap()["used_by"], 1);
    let (s, body) = send(&app, "DELETE", &format!("/rest/credentials/{cred}"), &token, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["tools"][0]["name"], "secured");
}
```

Plus a webhook-credentials test: a webhook workflow whose second node references a **non-existent** credential id now answers 500 with body containing `credential resolution failed` (previously it ran with an empty map):

```rust
#[tokio::test]
async fn webhook_runs_resolve_credentials_first() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "hook-cred@example.com").await;
    let wf = serde_json::json!({
        "name": "hook-cred",
        "nodes": [
            {"id": "hook", "node_type": "core.webhook", "position": [0.0, 0.0], "parameters": {"path": "hc", "method": "POST"}},
            {"id": "h", "node_type": "core.httpRequest", "position": [1.0, 0.0], "parameters": {"url": "https://x", "auth": {"type": "bearer", "credential_id": uuid::Uuid::new_v4().to_string()}}}
        ],
        "connections": [{"from_node": "hook", "from_output": 0, "to_node": "h", "to_input": 0}]
    });
    let (_, wf) = send(&app, "POST", "/rest/workflows", &token, Some(wf)).await;
    let wf_id = wf["id"].as_str().unwrap().to_string();
    send(&app, "PATCH", &format!("/rest/workflows/{wf_id}/active"), &token, Some(serde_json::json!({"active": true}))).await;
    let response = fire_webhook(&app, &wf_id, "hc").await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("credential resolution failed"));
}
```

(If activating that workflow fails because activation itself validates credentials, assert on the activation response instead and note it as a ruling.)

- [ ] **Step 2: Run** `cargo test --test api_test tool 2>&1 | grep -E "^test |test result"` — new tests FAIL (404 routes).

- [ ] **Step 3: Implement.** In `src/credentials.rs`:

```rust
/// `(id, name)` of every library tool whose parameters reference credential `id`.
pub fn tools_using_credential(tools: &[crate::domain::Tool], id: Uuid) -> Vec<(Uuid, String)> {
    let id = id.to_string();
    tools.iter().filter(|t| credential_ref(&t.parameters) == Some(id.as_str())).map(|t| (t.id, t.name.clone())).collect()
}

/// `(id, name)` of every workflow with an `ai.agent` node listing `tool_id` in `tool_ids`.
pub fn workflows_using_tool(workflows: &[Workflow], tool_id: Uuid) -> Vec<(Uuid, String)> {
    let id = tool_id.to_string();
    workflows
        .iter()
        .filter(|wf| {
            wf.nodes.iter().any(|n| {
                n.node_type == "ai.agent"
                    && n.parameters.get("tool_ids").and_then(|v| v.as_array()).is_some_and(|ids| ids.iter().any(|v| v.as_str() == Some(id.as_str())))
            })
        })
        .map(|wf| (wf.id, wf.name.clone()))
        .collect()
}
```

In `src/api/credentials.rs`: load `state.storage.list_tools()` alongside workflows (same error handling as `all_workflows`); `used_by = workflows_using_credential(..).len() + tools_using_credential(..).len()` in list/get/patch; DELETE refuses when either is non-empty, body `{"error": "credential is in use", "workflows": [...], "tools": [{"id","name"}]}`.

`src/api/tools.rs` mirrors `src/api/credentials.rs`'s structure:
- `ToolListItem { #[serde(flatten)] tool: Tool, used_by: usize }`.
- `CreateToolRequest { name, description, node_type, argument_schema, parameters }` (all required); `UpdateToolRequest` with every field `Option`.
- `create_tool`: build `Tool` (new id, now timestamps), `validate_tool` → 400 with the message; name taken (`list_tools` contains the name) → 409 `"a tool named <name> already exists"`; `create_tool`; 201 `Json(tool)`; `tracing::info!(tool_id, name, "tool created")`.
- `list_tools`, `get_tool` (404), `update_tool` (fetch → 404; apply present fields; `validate_tool` → 400; name clash with a *different* tool → 409; `updated_at = now`; `update_tool`; 200 `ToolListItem`), `delete_tool` (usage → 409 `{"error":"tool is in use","workflows":[...]}`; `delete_tool` → 204/404).
- Routes: `.route("/rest/tools", post(tools::create_tool).get(tools::list_tools))` and `.route("/rest/tools/:id", get(tools::get_tool).patch(tools::update_tool).delete(tools::delete_tool))`.

- [ ] **Step 4: Run** `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"` — all pass.

- [ ] **Step 5: Commit** `git add -A src tests && git commit -m "feat: /rest/tools API; credentials used by tools count as in use"`

---

### Task 6: Tools page (frontend)

**Files:** Modify `frontend/src/types/domain.ts`; Create `frontend/src/tools/schema.ts` + `schema.spec.ts`; Create `frontend/src/stores/tools.ts`; Create `frontend/src/components/ToolForm.vue`; Create `frontend/src/views/ToolsView.vue` + `ToolsView.spec.ts`; Modify `frontend/src/router/index.ts`; header links in `WorkflowListView.vue` and `CredentialsView.vue`; `frontend/src/stores/credentials.ts` (`inUseWorkflowNames` also lists tools).

**Interfaces:** `Tool`, `ToolArgument { name: string; type: 'string'|'number'|'integer'|'boolean'; description: string; required: boolean }`; `argsToSchema(args) → schema`, `schemaToArgs(schema) → args`, `PARAMETER_TEMPLATES: Record<string, Record<string, unknown>>`, `authForCredentialType(nodeType, credentialId, credentialType) → object`; store `useToolsStore` with `fetchAll/get/create/update/remove` and `inUseToolWorkflowNames(e)`.

- [ ] **Step 1: Types** — `types/domain.ts`:

```typescript
export type ToolArgumentType = 'string' | 'number' | 'integer' | 'boolean'

export interface Tool {
  id: string
  name: string
  description: string
  node_type: string
  argument_schema: { type: 'object'; properties?: Record<string, { type: ToolArgumentType; description?: string }>; required?: string[] }
  parameters: Record<string, unknown>
  created_at: string
  updated_at: string
  used_by?: number
}
```

- [ ] **Step 2: Failing tests** — `frontend/src/tools/schema.spec.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { argsToSchema, schemaToArgs, authForCredentialType, PARAMETER_TEMPLATES } from './schema'

describe('tool schema helpers', () => {
  it('round-trips arguments through a JSON schema', () => {
    const args = [
      { name: 'city', type: 'string' as const, description: 'City name', required: true },
      { name: 'days', type: 'integer' as const, description: '', required: false },
    ]
    const schema = argsToSchema(args)
    expect(schema).toEqual({
      type: 'object',
      properties: { city: { type: 'string', description: 'City name' }, days: { type: 'integer' } },
      required: ['city'],
    })
    expect(schemaToArgs(schema)).toEqual(args)
  })

  it('offers an $args template per tool node type', () => {
    expect(JSON.stringify(PARAMETER_TEMPLATES['core.httpRequest'])).toContain('{{ $args.query }}')
    expect(JSON.stringify(PARAMETER_TEMPLATES['telegram.sendMessage'])).toContain('{{ $args.message }}')
    expect(JSON.stringify(PARAMETER_TEMPLATES['core.code'])).toContain('$args')
  })

  it('maps a generic credential type to the HTTP auth type', () => {
    expect(authForCredentialType('core.httpRequest', 'c1', 'bearerToken')).toEqual({ type: 'bearer', credential_id: 'c1' })
    expect(authForCredentialType('core.httpRequest', 'c1', 'apiKeyHeader')).toEqual({ type: 'apiKey', credential_id: 'c1' })
    expect(authForCredentialType('core.httpRequest', 'c1', 'basicAuth')).toEqual({ type: 'basic', credential_id: 'c1' })
    expect(authForCredentialType('telegram.sendMessage', 'c2', 'telegramApi')).toEqual({ credential_id: 'c2' })
  })
})
```

`frontend/src/views/ToolsView.spec.ts` (mirrors `CredentialsView.spec.ts`): lists rows with "used by N workflows"; delete 409 names the workflow; 204 removes the row; "+ New tool" shows the form with the HTTP template pre-filled in the parameters textarea (`textarea[aria-label="Parameters (JSON)"]` value contains `{{ $args.query }}`); saving a new tool POSTs `argument_schema` built from one argument row (fill `input[aria-label="Argument name"]` = `query`, type select `string`, required checkbox) and `name`/`description`.

- [ ] **Step 3: Run** `cd frontend && npx vitest run src/tools src/views/ToolsView.spec.ts 2>&1 | grep -E "FAIL|Tests"` — fail (modules missing).

- [ ] **Step 4: Implement** `frontend/src/tools/schema.ts`:

```typescript
import type { Tool, ToolArgumentType } from '../types/domain'

export interface ToolArgument {
  name: string
  type: ToolArgumentType
  description: string
  required: boolean
}

export const TOOL_NODE_TYPES: { value: string; label: string }[] = [
  { value: 'core.httpRequest', label: 'HTTP Request' },
  { value: 'telegram.sendMessage', label: 'Telegram: Send Message' },
  { value: 'core.code', label: 'Code' },
]

/** Starter parameters per node type, showing where {{ $args.x }} goes. */
export const PARAMETER_TEMPLATES: Record<string, Record<string, unknown>> = {
  'core.httpRequest': { method: 'GET', url: 'https://api.example.com/search?q={{ $args.query }}' },
  'telegram.sendMessage': { chat_id: '', text: '{{ $args.message }}' },
  'core.code': { script: 'return [{ json: { result: $args } }]' },
}

const HTTP_AUTH_TYPE: Record<string, string> = { bearerToken: 'bearer', apiKeyHeader: 'apiKey', basicAuth: 'basic' }

export function authForCredentialType(nodeType: string, credentialId: string, credentialType: string | undefined): Record<string, string> {
  const type = nodeType === 'core.httpRequest' && credentialType ? HTTP_AUTH_TYPE[credentialType] : undefined
  return type ? { type, credential_id: credentialId } : { credential_id: credentialId }
}

export function argsToSchema(args: ToolArgument[]): Tool['argument_schema'] {
  return {
    type: 'object',
    properties: Object.fromEntries(
      args.map((a) => [a.name, a.description ? { type: a.type, description: a.description } : { type: a.type }]),
    ),
    required: args.filter((a) => a.required).map((a) => a.name),
  }
}

export function schemaToArgs(schema: Tool['argument_schema']): ToolArgument[] {
  const required = schema.required ?? []
  return Object.entries(schema.properties ?? {}).map(([name, p]) => ({
    name,
    type: p.type,
    description: p.description ?? '',
    required: required.includes(name),
  }))
}
```

`frontend/src/stores/tools.ts` — same shape as `stores/credentials.ts` (`fetchAll` → `/rest/tools`, `get`, `create(body)` POST, `update(id, patch)` PATCH then `fetchAll`, `remove(id)` DELETE then `fetchAll`) plus:

```typescript
export function inUseToolWorkflowNames(e: unknown): string[] | null {
  if (!(e instanceof ApiError) || e.status !== 409) return null
  try {
    const body = JSON.parse(e.message) as { workflows?: { name: string }[] }
    return Array.isArray(body.workflows) ? body.workflows.map((w) => w.name) : null
  } catch {
    return null
  }
}
```

In `stores/credentials.ts`, `inUseWorkflowNames` also appends `body.tools?.map((t) => `${t.name} (tool)`) ?? []`.

`ToolForm.vue` (props `toolId?: string`; emits `saved(tool)`, `cancel`): fields with `aria-label`s — `Name`, `Description` (textarea, hint "The model reads this to decide when to call the tool."), `Node type` (select from `TOOL_NODE_TYPES`), a `CredentialPicker` (`:node-type="nodeType"`, `v-model="credentialId"`) shown for all three types, an arguments table (each row: `Argument name` input, `Argument type` select, `Argument description` input, `Argument required` checkbox, remove button; "+ Add argument"), and `Parameters (JSON)` textarea (hint "Use {{ $args.<name> }} to insert an argument."). Behaviour:
  - New tool: `nodeType = 'core.httpRequest'`, parameters textarea = `JSON.stringify(PARAMETER_TEMPLATES[type], null, 2)`; changing the node type on a **new** tool replaces the textarea with that type's template.
  - Edit: load via `store.get(toolId)`; `schemaToArgs(argument_schema)`; `credentialId = parameters.auth?.credential_id ?? null`; textarea = parameters **without** `auth`.
  - Save: client checks — name `/^[A-Za-z0-9_-]{1,64}$/` ("Name must be 1-64 letters, digits, _ or -."), argument names non-empty, unique, `/^[A-Za-z_][A-Za-z0-9_]*$/`, parameters valid JSON object. Build `parameters = { ...parsed, ...(credentialId ? { auth: authForCredentialType(nodeType, credentialId, credentialType) } : {}) }` where `credentialType` comes from `useCredentialsStore().credentials`. POST (create) or PATCH (edit) via the store; on `ApiError` show its message text; emit `saved`.

`ToolsView.vue` — same layout as `CredentialsView.vue`: header "Tools" with links Workflows / Credentials; "+ New tool" toggles `<ToolForm>`; rows (`data-testid="tool-row"`) show name, node type label, description, "used by N workflows" / "not used"; Edit (inline `<ToolForm :tool-id>`) and Delete (`data-testid="delete-tool"`, confirm, 409 → `Can't delete "<name>": used by <names>. Remove it from those agents first.`).

Router: `{ path: '/tools', name: 'tools', component: () => import('../views/ToolsView.vue'), meta: { requiresAuth: true } }`. Header links: `WorkflowListView` gains `<router-link to="/tools">Tools</router-link>` next to Credentials; `CredentialsView` header gains a Tools link.

- [ ] **Step 5: Run** `cd frontend && npx vitest run 2>&1 | grep -E "×|Tests " && npx vue-tsc --noEmit && echo tsc-ok` — all pass.

- [ ] **Step 6: Commit** `git add -A frontend/src && git commit -m "feat: Tools page with argument table and \$args parameter templates"`

---

### Task 7: AI Agent settings form

**Files:** Create `frontend/src/components/AgentSettings.vue`; Modify `frontend/src/components/NodeConfigPanel.vue` + `NodeConfigPanel.spec.ts`.

**Interfaces:** `AgentSettings` props `{ modelValue: AgentFields }`, emits `update:modelValue`; `AgentFields = { provider: string; model: string; system_prompt: string; user_message: string; max_iterations: number | string; tool_ids: string[] }`.

- [ ] **Step 1: Update the quick-fix tests' fixtures** — the three agent tests added in the quick fix call `agent({ auth: ... })`; the form now requires model and user message, so change the helper to `agent(parameters)` returning `{ ..., parameters: { model: 'm', user_message: 'hi', ...parameters } }`. (Their assertions stay as they are.)

- [ ] **Step 2: Failing tests** — append to `NodeConfigPanel.spec.ts`:

```typescript
  it('loads existing agent parameters into the form', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' }, provider: 'openai', model: 'qwen', system_prompt: 'be brief', user_message: '{{ $json.text }}', max_iterations: 5 }) } })
    expect((wrapper.find('select[aria-label="Provider"]').element as HTMLSelectElement).value).toBe('openai')
    expect((wrapper.find('input[aria-label="Model"]').element as HTMLInputElement).value).toBe('qwen')
    expect((wrapper.find('textarea[aria-label="System prompt"]').element as HTMLTextAreaElement).value).toBe('be brief')
    expect((wrapper.find('input[aria-label="User message"]').element as HTMLInputElement).value).toBe('{{ $json.text }}')
    expect((wrapper.find('input[aria-label="Max iterations"]').element as HTMLInputElement).value).toBe('5')
  })

  it('writes the form fields into the agent parameters on Apply', async () => {
    seedCredential('c1', 'openaiApi')
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({ auth: { credential_id: 'c1' } }) } })
    await wrapper.find('input[aria-label="Model"]').setValue('llama3')
    await wrapper.find('input[aria-label="User message"]').setValue('Hello')
    await wrapper.find('textarea[aria-label="System prompt"]').setValue('sys')
    await clickApply(wrapper)
    const params = (wrapper.emitted('update')![0][0] as NodeInstance).parameters
    expect(params).toMatchObject({ provider: 'openai', model: 'llama3', user_message: 'Hello', system_prompt: 'sys', max_iterations: 10, tool_ids: [] })
  })

  it('requires model and user message for an agent', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node: { id: 'a1', node_type: 'ai.agent', position: [0, 0], parameters: {}, disabled: false } } })
    await clickApply(wrapper)
    expect(wrapper.text()).toContain('Model is required.')
    expect(wrapper.emitted('update')).toBeFalsy()
  })

  it('stores checked library tools as tool_ids', async () => {
    const { useToolsStore } = await import('../stores/tools')
    const tools = useToolsStore()
    tools.tools = [{ id: 't1', name: 'search', description: 'web search', node_type: 'core.httpRequest', argument_schema: { type: 'object' }, parameters: {}, created_at: '', updated_at: '' }]
    tools.loaded = true
    const wrapper = mount(NodeConfigPanel, { props: { node: agent({}) } })
    await wrapper.find('input[aria-label="Use tool search"]').setValue(true)
    await clickApply(wrapper)
    expect((wrapper.emitted('update')![0][0] as NodeInstance).parameters.tool_ids).toEqual(['t1'])
  })
```

- [ ] **Step 3: Run** `cd frontend && npx vitest run src/components/NodeConfigPanel.spec.ts 2>&1 | grep -E "×|Tests"` — the 4 new tests fail.

- [ ] **Step 4: Implement** `AgentSettings.vue`:

```vue
<script setup lang="ts">
import { useToolsStore } from '../stores/tools'

export interface AgentFields {
  provider: string
  model: string
  system_prompt: string
  user_message: string
  max_iterations: number | string
  tool_ids: string[]
}

const fields = defineModel<AgentFields>({ required: true })
defineProps<{ inlineToolCount: number }>()
const toolsStore = useToolsStore()
if (!toolsStore.loaded) toolsStore.fetchAll().catch(() => {})

function toggleTool(id: string, on: boolean) {
  const ids = new Set(fields.value.tool_ids)
  if (on) ids.add(id)
  else ids.delete(id)
  fields.value = { ...fields.value, tool_ids: [...ids] }
}
</script>

<template>
  <fieldset class="border rounded p-2 space-y-2">
    <legend class="text-sm text-gray-600 px-1">AI Agent</legend>
    <label class="block text-xs text-gray-600">
      Provider
      <select v-model="fields.provider" aria-label="Provider" class="w-full border rounded px-2 py-1 text-sm">
        <option value="">(from credential)</option>
        <option value="openai">OpenAI-compatible (OpenAI, Ollama, …)</option>
        <option value="anthropic">Anthropic</option>
      </select>
    </label>
    <label class="block text-xs text-gray-600">
      Model
      <input v-model="fields.model" aria-label="Model" placeholder="e.g. qwen3-coding:latest" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <label class="block text-xs text-gray-600">
      System prompt
      <textarea v-model="fields.system_prompt" aria-label="System prompt" rows="3" class="w-full border rounded px-2 py-1 text-sm"></textarea>
    </label>
    <label class="block text-xs text-gray-600">
      User message
      <input v-model="fields.user_message" aria-label="User message" class="w-full border rounded px-2 py-1 text-sm" />
      <span class="text-gray-400">Expressions like {{ '{' + '{ $json.message.text }' + '}' }} work here.</span>
    </label>
    <label class="block text-xs text-gray-600">
      Max iterations
      <input v-model="fields.max_iterations" aria-label="Max iterations" type="number" min="1" max="50" class="w-full border rounded px-2 py-1 text-sm" />
    </label>
    <div class="text-xs text-gray-600 space-y-1">
      <div class="flex justify-between"><span>Tools</span><router-link to="/tools" class="text-blue-600">Manage tools</router-link></div>
      <label v-for="t in toolsStore.tools" :key="t.id" class="flex gap-2 items-start">
        <input
          type="checkbox"
          :aria-label="`Use tool ${t.name}`"
          :checked="fields.tool_ids.includes(t.id)"
          @change="toggleTool(t.id, ($event.target as HTMLInputElement).checked)"
        />
        <span><span class="font-mono">{{ t.name }}</span> — {{ t.description }}</span>
      </label>
      <p v-if="toolsStore.loaded && toolsStore.tools.length === 0" class="text-gray-400">No tools yet.</p>
      <p v-if="inlineToolCount > 0" class="text-gray-400">{{ inlineToolCount }} inline tool(s) (edit in Advanced).</p>
    </div>
  </fieldset>
</template>
```

`NodeConfigPanel.vue`:
- Import `AgentSettings` and its `AgentFields` type; `const isAgent = computed(() => props.node?.node_type === 'ai.agent')`; `const agentFields = ref<AgentFields>(emptyAgentFields())` where `emptyAgentFields()` = `{ provider: '', model: '', system_prompt: '', user_message: '', max_iterations: 10, tool_ids: [] }`.
- In the node `watch`, when the node is an agent, load from `node.parameters` (strings default `''`, `max_iterations ?? 10`, `tool_ids` array of strings or `[]`).
- Template: when `isAgent`, render `<AgentSettings v-model="agentFields" :inline-tool-count="inlineToolCount" />` above the credential picker, and wrap the existing Parameters textarea in `<details><summary class="text-sm text-gray-600">Advanced (JSON)</summary> … </details>` (for non-agent nodes keep it as it is, not collapsed).
- In `apply()`, after parsing the JSON and before the provider inference: if `isAgent`, validate `model` ("Model is required."), `user_message` ("User message is required."), `max_iterations` integer 1–50 ("Max iterations must be between 1 and 50."), then assign `parsed.model`, `parsed.user_message`, `parsed.system_prompt`, `parsed.max_iterations = Number(...)`, `parsed.tool_ids`, and `parsed.provider = agentFields.provider` **only if non-empty** (so the existing credential-type inference still fills it when empty). Mount-time `useToolsStore` import is inside `AgentSettings`, so non-agent panels don't fetch tools.
- `inlineToolCount = computed(() => Array.isArray(props.node?.parameters?.tools) ? props.node.parameters.tools.length : 0)`.

- [ ] **Step 5: Run** `cd frontend && npx vitest run 2>&1 | grep -E "×|Tests " && npx vue-tsc --noEmit && echo tsc-ok` — all pass.

- [ ] **Step 6: Commit** `git add -A frontend/src && git commit -m "feat: AI Agent settings form with library tool selection"`

---

### Task 8: Build and end-to-end check

- [ ] **Step 1:** `cd frontend && npm run build 2>&1 | tail -1` → `✓ built`.
- [ ] **Step 2:** `cargo test 2>&1 | grep -E "^test result|FAILED"` and `cd frontend && npx vitest run 2>&1 | grep "Tests "` → all pass.
- [ ] **Step 3: Smoke test** on a fresh temp database: start r8r on a spare port with a wiremock-free setup — register; create a `core.code` tool `double` with argument `n` (integer, required) and parameters `{"script": "return [{ json: { doubled: $args.n * 2 } }]"}` via the Tools page in the browser; create a workflow with an `ai.agent` node, set provider/model/user message in the new form and tick `double`; Apply + Save; verify the saved parameters contain `tool_ids` with the tool id and the Tools page shows "used by 1 workflow"; delete the tool → refused naming the workflow. (No LLM call needed.)
