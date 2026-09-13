# r8r Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the minimal end-to-end r8r skeleton: a user can register/log in, create a two-node workflow (Manual Trigger → Set) over the REST API, execute it, and read back the resulting execution data — all backed by SQLite, in one Rust binary.

**Architecture:** Single Axum-based binary crate. A `Storage` trait abstracts persistence (SQLite implementation only, for now). A `Node` trait + `NodeRegistry` abstracts workflow steps (Manual Trigger and Set implemented here). A linear (non-branching) execution engine walks a workflow's single chain of nodes, feeding each node's output items to the next. Branching (If/Switch/Merge), the QuickJS expression engine, triggers (Webhook/Schedule), HTTP-based nodes, and the AI Agent node are explicitly out of scope for this plan — see Roadmap.

**Tech Stack:** Rust, Axum, Tokio, sqlx (SQLite), serde/serde_json, argon2, jsonwebtoken, uuid, chrono, async-trait, thiserror, anyhow.

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md`

## Global Constraints

- Single binary, no external services required to run (SQLite only) — spec §2, §9.
- `Storage` is a trait so Postgres can be added later without touching callers — spec §9.
- No dynamic/plugin node loading in v1; nodes are compiled in and registered at startup — spec §5.2.
- Slack and Email nodes are out of scope entirely — spec §2, §10.
- Branching/merge execution, expressions, and triggers beyond what's listed above are out of scope for *this* plan specifically (later plans in the Roadmap below cover them) — this plan only needs to prove the skeleton works end-to-end.

---

## File Structure

- `Cargo.toml` — single binary crate `r8r`.
- `migrations/0001_init.sql` — `users`, `workflows`, `executions` tables.
- `src/main.rs` — process entrypoint: load config, connect storage, build registry, build router, serve.
- `src/domain.rs` — `Workflow`, `NodeInstance`, `Connection`, `Item`, `Execution`, `ExecutionStatus`, `ExecutionMode`, `User`, `UserRole`.
- `src/storage/mod.rs` — `Storage` trait.
- `src/storage/sqlite.rs` — `SqliteStorage` (impl of `Storage` over sqlx SQLite pool).
- `src/node.rs` — `Node` trait, `NodeExecutionContext`, `NodeError`, `NodeRegistry`.
- `src/nodes/mod.rs` — `register_all(&mut NodeRegistry)`.
- `src/nodes/manual_trigger.rs` — `ManualTriggerNode`.
- `src/nodes/set.rs` — `SetNode` (static field assignment; expression support comes in a later plan).
- `src/engine.rs` — `execute_workflow()` + linear ordering.
- `src/auth.rs` — password hashing, JWT issue/verify.
- `src/state.rs` — `AppState`.
- `src/api/mod.rs` — router assembly, auth extractor.
- `src/api/auth.rs` — `POST /rest/auth/register`, `POST /rest/auth/login`.
- `src/api/workflows.rs` — `POST /rest/workflows`, `GET /rest/workflows`, `GET /rest/workflows/:id`, `POST /rest/workflows/:id/execute`.
- `src/api/executions.rs` — `GET /rest/executions/:id`.
- `tests/api_test.rs` — full-stack integration tests against an in-memory SQLite DB.

---

### Task 1: Project scaffold + health endpoint

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Test: `tests/health_test.rs`

**Interfaces:**
- Produces: a runnable Axum server exposing `GET /health` → `200 OK` with body `"ok"`.

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "r8r"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-rustls", "migrate", "uuid", "chrono"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
argon2 = "0.5"
jsonwebtoken = "9"
async-trait = "0.1"
thiserror = "1"
anyhow = "1"
tower = { version = "0.4", features = ["util"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
http-body-util = "0.1"
```

- [ ] **Step 2: Write `src/main.rs` with just the health route**

```rust
use axum::{routing::get, Router};

pub fn health_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let app = health_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("r8r listening on :3000");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 3: Write the integration test**

```rust
// tests/health_test.rs
use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[path = "../src/main.rs"]
mod app_main;

#[tokio::test]
async fn health_returns_ok() {
    let app = app_main::health_router();
    let response = app
        .oneshot(axum::http::Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
```

Note: including `main.rs` via `#[path]` is a Task-1-only convenience to get a runnable skeleton fast. Task 2 onward moves logic into a library (`src/lib.rs`) so `main.rs` becomes a thin binary entrypoint and tests import the crate normally instead of path-including `main.rs`.

- [ ] **Step 4: Run the test, expect it to fail to compile (no `main.rs` yet / not a lib)**

Run: `cargo test --test health_test`
Expected: compiles once Steps 1-3 are saved, then passes. If it fails here, fix `Cargo.toml`/`main.rs` before proceeding — this task has no separate "red" phase since the scaffold has no prior code to contrast against.

- [ ] **Step 5: Run `cargo build` and `cargo test` to confirm a clean baseline**

Run: `cargo build && cargo test`
Expected: builds and the one test passes.

- [ ] **Step 6: Commit**

```bash
git init
git add Cargo.toml src/main.rs tests/health_test.rs
git commit -m "chore: scaffold r8r binary with health endpoint"
```

---

### Task 2: Convert to lib + binary split, add domain types

**Files:**
- Create: `src/lib.rs`
- Modify: `src/main.rs` (becomes thin entrypoint calling into the lib)
- Create: `src/domain.rs`
- Test: `src/domain.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Produces: `r8r::domain::{Workflow, NodeInstance, Connection, Item, Execution, ExecutionStatus, ExecutionMode, User, UserRole}`, all `Serialize + Deserialize + Clone + Debug`.
- Produces: `r8r::health_router()` (moved from `main.rs`).

- [ ] **Step 1: Add `[lib]` section to `Cargo.toml`**

```toml
[lib]
name = "r8r"
path = "src/lib.rs"
```

- [ ] **Step 2: Create `src/lib.rs`, move `health_router` into it**

```rust
pub mod domain;

use axum::{routing::get, Router};

pub fn health_router() -> Router {
    Router::new().route("/health", get(|| async { "ok" }))
}
```

- [ ] **Step 3: Slim `src/main.rs` down to an entrypoint**

```rust
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let app = r8r::health_router();
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("r8r listening on :3000");
    axum::serve(listener, app).await.unwrap();
}
```

- [ ] **Step 4: Delete the `#[path]` hack and rewrite `tests/health_test.rs` against the crate**

```rust
use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok() {
    let app = r8r::health_router();
    let response = app
        .oneshot(axum::http::Request::builder().uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"ok");
}
```

- [ ] **Step 5: Write failing domain tests in `src/domain.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeInstance {
    pub id: String,
    pub node_type: String,
    pub position: (f64, f64),
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub from_node: String,
    pub from_output: usize,
    pub to_node: String,
    pub to_input: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub active: bool,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Item {
    pub json: serde_json::Value,
    #[serde(default)]
    pub binary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExecutionMode {
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: Uuid,
    pub workflow_id: Uuid,
    pub status: ExecutionStatus,
    pub mode: ExecutionMode,
    pub node_outputs: HashMap<String, Vec<Item>>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserRole {
    Owner,
    Member,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_round_trips_through_json() {
        let wf = Workflow {
            id: Uuid::new_v4(),
            name: "test".into(),
            active: false,
            nodes: vec![NodeInstance {
                id: "n1".into(),
                node_type: "core.manualTrigger".into(),
                position: (0.0, 0.0),
                parameters: serde_json::json!({}),
                disabled: false,
            }],
            connections: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&wf).unwrap();
        let parsed: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(wf, parsed);
    }
}
```

Add `pub mod domain;` to `src/lib.rs` (already added in Step 2).

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: `workflow_round_trips_through_json` and `health_returns_ok` both pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/lib.rs src/main.rs src/domain.rs tests/health_test.rs
git commit -m "refactor: split lib/bin, add domain types"
```

---

### Task 3: `Storage` trait + SQLite migrations + connection bootstrap

**Files:**
- Create: `migrations/0001_init.sql`
- Create: `src/storage/mod.rs`
- Create: `src/storage/sqlite.rs`
- Modify: `src/lib.rs` (add `pub mod storage;`)
- Test: `src/storage/sqlite.rs` (inline)

**Interfaces:**
- Consumes: `r8r::domain::*` from Task 2.
- Produces: `r8r::storage::Storage` trait (async methods listed below); `r8r::storage::sqlite::SqliteStorage::new(db_url: &str) -> anyhow::Result<Self>`, implementing `Storage`.

- [ ] **Step 1: Write the migration**

```sql
-- migrations/0001_init.sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0,
    definition TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE executions (
    id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL REFERENCES workflows(id),
    status TEXT NOT NULL,
    mode TEXT NOT NULL,
    data TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT
);
```

- [ ] **Step 2: Define the `Storage` trait**

```rust
// src/storage/mod.rs
pub mod sqlite;

use crate::domain::{Execution, User, Workflow};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()>;
    async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>>;
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>>;

    async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()>;
    async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()>;
    async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>>;

    async fn create_user(&self, user: &User) -> anyhow::Result<()>;
    async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>>;
}
```

- [ ] **Step 3: Write a failing test for `SqliteStorage::new` connecting + migrating**

```rust
// bottom of src/storage/sqlite.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new_connects_and_migrates() {
        let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
        let workflows = storage.list_workflows().await.unwrap();
        assert!(workflows.is_empty());
    }
}
```

- [ ] **Step 4: Run to confirm it fails to compile (no `SqliteStorage` yet)**

Run: `cargo test storage::sqlite::tests::new_connects_and_migrates`
Expected: compile error, `SqliteStorage` not found.

- [ ] **Step 5: Implement `SqliteStorage` (connection + migration only; CRUD comes in Task 4)**

```rust
// src/storage/sqlite.rs (top, above the tests module)
use super::Storage;
use crate::domain::{Execution, User, Workflow};
use async_trait::async_trait;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub async fn new(db_url: &str) -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new().connect(db_url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn create_workflow(&self, _workflow: &Workflow) -> anyhow::Result<()> {
        unimplemented!("added in Task 4")
    }
    async fn get_workflow(&self, _id: Uuid) -> anyhow::Result<Option<Workflow>> {
        unimplemented!("added in Task 4")
    }
    async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
        Ok(vec![])
    }
    async fn create_execution(&self, _execution: &Execution) -> anyhow::Result<()> {
        unimplemented!("added in Task 5")
    }
    async fn update_execution(&self, _execution: &Execution) -> anyhow::Result<()> {
        unimplemented!("added in Task 5")
    }
    async fn get_execution(&self, _id: Uuid) -> anyhow::Result<Option<Execution>> {
        unimplemented!("added in Task 5")
    }
    async fn create_user(&self, _user: &User) -> anyhow::Result<()> {
        unimplemented!("added in Task 6")
    }
    async fn get_user_by_email(&self, _email: &str) -> anyhow::Result<Option<User>> {
        unimplemented!("added in Task 6")
    }
}
```

Add `pub mod storage;` to `src/lib.rs`, and add `async-trait = "0.1"` usage (already a dependency).

- [ ] **Step 6: Run tests**

Run: `cargo test storage::sqlite::tests::new_connects_and_migrates`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add migrations/0001_init.sql src/storage/mod.rs src/storage/sqlite.rs src/lib.rs
git commit -m "feat: add Storage trait and SQLite connection/migration bootstrap"
```

---

### Task 4: SQLite workflow CRUD

**Files:**
- Modify: `src/storage/sqlite.rs`

**Interfaces:**
- Consumes: `Storage` trait from Task 3.
- Produces: working `create_workflow`, `get_workflow`, `list_workflows`.

- [ ] **Step 1: Write failing tests**

```rust
// add to the tests module in src/storage/sqlite.rs
use crate::domain::{NodeInstance};
use chrono::Utc;

fn sample_workflow() -> Workflow {
    Workflow {
        id: Uuid::new_v4(),
        name: "sample".into(),
        active: false,
        nodes: vec![NodeInstance {
            id: "n1".into(),
            node_type: "core.manualTrigger".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
        }],
        connections: vec![],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn create_and_get_workflow_round_trips() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let wf = sample_workflow();
    storage.create_workflow(&wf).await.unwrap();

    let fetched = storage.get_workflow(wf.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, wf.id);
    assert_eq!(fetched.name, wf.name);
    assert_eq!(fetched.nodes, wf.nodes);
}

#[tokio::test]
async fn list_workflows_returns_created_ones() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    storage.create_workflow(&sample_workflow()).await.unwrap();
    storage.create_workflow(&sample_workflow()).await.unwrap();

    let all = storage.list_workflows().await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn get_workflow_returns_none_when_missing() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let result = storage.get_workflow(Uuid::new_v4()).await.unwrap();
    assert!(result.is_none());
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test storage::sqlite::tests::create_and_get_workflow_round_trips`
Expected: FAIL (`unimplemented`).

- [ ] **Step 3: Implement the three methods**

```rust
// replace the three workflow methods in the Storage impl for SqliteStorage
async fn create_workflow(&self, workflow: &Workflow) -> anyhow::Result<()> {
    let definition = serde_json::json!({
        "nodes": workflow.nodes,
        "connections": workflow.connections,
    });
    sqlx::query(
        "INSERT INTO workflows (id, name, active, definition, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(workflow.id.to_string())
    .bind(&workflow.name)
    .bind(workflow.active as i64)
    .bind(definition.to_string())
    .bind(workflow.created_at.to_rfc3339())
    .bind(workflow.updated_at.to_rfc3339())
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn get_workflow(&self, id: Uuid) -> anyhow::Result<Option<Workflow>> {
    let row = sqlx::query_as::<_, (String, String, i64, String, String, String)>(
        "SELECT id, name, active, definition, created_at, updated_at FROM workflows WHERE id = ?"
    )
    .bind(id.to_string())
    .fetch_optional(&self.pool)
    .await?;
    Ok(row.map(row_to_workflow).transpose()?)
}

async fn list_workflows(&self) -> anyhow::Result<Vec<Workflow>> {
    let rows = sqlx::query_as::<_, (String, String, i64, String, String, String)>(
        "SELECT id, name, active, definition, created_at, updated_at FROM workflows"
    )
    .fetch_all(&self.pool)
    .await?;
    rows.into_iter().map(row_to_workflow).collect()
}
```

```rust
// free function near the bottom of src/storage/sqlite.rs, above the tests module
fn row_to_workflow(
    row: (String, String, i64, String, String, String),
) -> anyhow::Result<Workflow> {
    let (id, name, active, definition, created_at, updated_at) = row;
    let def: serde_json::Value = serde_json::from_str(&definition)?;
    Ok(Workflow {
        id: Uuid::parse_str(&id)?,
        name,
        active: active != 0,
        nodes: serde_json::from_value(def["nodes"].clone())?,
        connections: serde_json::from_value(def["connections"].clone())?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test storage::sqlite::tests::`
Expected: all four SQLite tests (from Task 3 and this task) PASS.

- [ ] **Step 5: Commit**

```bash
git add src/storage/sqlite.rs
git commit -m "feat: implement SQLite workflow CRUD"
```

---

### Task 5: SQLite execution CRUD

**Files:**
- Modify: `src/storage/sqlite.rs`

**Interfaces:**
- Produces: working `create_execution`, `update_execution`, `get_execution`.

- [ ] **Step 1: Write failing tests**

```rust
use crate::domain::{Execution, ExecutionMode, ExecutionStatus, Item};
use std::collections::HashMap;

fn sample_execution(workflow_id: Uuid) -> Execution {
    Execution {
        id: Uuid::new_v4(),
        workflow_id,
        status: ExecutionStatus::Running,
        mode: ExecutionMode::Manual,
        node_outputs: HashMap::new(),
        started_at: Utc::now(),
        finished_at: None,
    }
}

#[tokio::test]
async fn create_and_get_execution_round_trips() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let wf = sample_workflow();
    storage.create_workflow(&wf).await.unwrap();
    let exec = sample_execution(wf.id);
    storage.create_execution(&exec).await.unwrap();

    let fetched = storage.get_execution(exec.id).await.unwrap().unwrap();
    assert_eq!(fetched.id, exec.id);
    assert_eq!(fetched.status, ExecutionStatus::Running);
}

#[tokio::test]
async fn update_execution_persists_new_status_and_outputs() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let wf = sample_workflow();
    storage.create_workflow(&wf).await.unwrap();
    let mut exec = sample_execution(wf.id);
    storage.create_execution(&exec).await.unwrap();

    exec.status = ExecutionStatus::Success;
    exec.finished_at = Some(Utc::now());
    exec.node_outputs.insert(
        "n1".into(),
        vec![Item { json: serde_json::json!({"a": 1}), binary: serde_json::json!({}) }],
    );
    storage.update_execution(&exec).await.unwrap();

    let fetched = storage.get_execution(exec.id).await.unwrap().unwrap();
    assert_eq!(fetched.status, ExecutionStatus::Success);
    assert!(fetched.finished_at.is_some());
    assert_eq!(fetched.node_outputs["n1"][0].json, serde_json::json!({"a": 1}));
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test storage::sqlite::tests::create_and_get_execution_round_trips`
Expected: FAIL (`unimplemented`).

- [ ] **Step 3: Implement**

```rust
async fn create_execution(&self, execution: &Execution) -> anyhow::Result<()> {
    let data = serde_json::to_string(&execution.node_outputs)?;
    sqlx::query(
        "INSERT INTO executions (id, workflow_id, status, mode, data, started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(execution.id.to_string())
    .bind(execution.workflow_id.to_string())
    .bind(serde_json::to_string(&execution.status)?)
    .bind(serde_json::to_string(&execution.mode)?)
    .bind(data)
    .bind(execution.started_at.to_rfc3339())
    .bind(execution.finished_at.map(|t| t.to_rfc3339()))
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn update_execution(&self, execution: &Execution) -> anyhow::Result<()> {
    let data = serde_json::to_string(&execution.node_outputs)?;
    sqlx::query(
        "UPDATE executions SET status = ?, data = ?, finished_at = ? WHERE id = ?"
    )
    .bind(serde_json::to_string(&execution.status)?)
    .bind(data)
    .bind(execution.finished_at.map(|t| t.to_rfc3339()))
    .bind(execution.id.to_string())
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn get_execution(&self, id: Uuid) -> anyhow::Result<Option<Execution>> {
    let row = sqlx::query_as::<_, (String, String, String, String, String, String, Option<String>)>(
        "SELECT id, workflow_id, status, mode, data, started_at, finished_at FROM executions WHERE id = ?"
    )
    .bind(id.to_string())
    .fetch_optional(&self.pool)
    .await?;
    row.map(row_to_execution).transpose()
}
```

```rust
fn row_to_execution(
    row: (String, String, String, String, String, String, Option<String>),
) -> anyhow::Result<Execution> {
    let (id, workflow_id, status, mode, data, started_at, finished_at) = row;
    Ok(Execution {
        id: Uuid::parse_str(&id)?,
        workflow_id: Uuid::parse_str(&workflow_id)?,
        status: serde_json::from_str(&status)?,
        mode: serde_json::from_str(&mode)?,
        node_outputs: serde_json::from_str(&data)?,
        started_at: chrono::DateTime::parse_from_rfc3339(&started_at)?.with_timezone(&chrono::Utc),
        finished_at: finished_at
            .map(|t| chrono::DateTime::parse_from_rfc3339(&t).map(|d| d.with_timezone(&chrono::Utc)))
            .transpose()?,
    })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test storage::sqlite::tests::`
Expected: all PASS, including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add src/storage/sqlite.rs
git commit -m "feat: implement SQLite execution CRUD"
```

---

### Task 6: SQLite user storage + password hashing

**Files:**
- Modify: `src/storage/sqlite.rs`
- Create: `src/auth.rs`
- Modify: `src/lib.rs` (add `pub mod auth;`)

**Interfaces:**
- Produces: `r8r::auth::hash_password(&str) -> anyhow::Result<String>`, `r8r::auth::verify_password(&str, &str) -> anyhow::Result<bool>`.
- Produces: working `Storage::create_user`, `Storage::get_user_by_email`.

- [ ] **Step 1: Write failing tests for password hashing**

```rust
// src/auth.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(verify_password("correct-horse", &hash).unwrap());
    }

    #[test]
    fn verify_fails_for_wrong_password() {
        let hash = hash_password("correct-horse").unwrap();
        assert!(!verify_password("wrong", &hash).unwrap());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure (no functions yet)**

Run: `cargo test auth::tests::`
Expected: compile error.

- [ ] **Step 3: Implement hashing**

```rust
// top of src/auth.rs, above the tests module
use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash failed: {e}"))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> anyhow::Result<bool> {
    let parsed = PasswordHash::new(hash).map_err(|e| anyhow::anyhow!("bad hash: {e}"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}
```

Add `pub mod auth;` to `src/lib.rs`.

- [ ] **Step 4: Run auth tests**

Run: `cargo test auth::tests::`
Expected: PASS.

- [ ] **Step 5: Write failing tests for user storage**

```rust
// add to tests module in src/storage/sqlite.rs
use crate::domain::UserRole;

fn sample_user() -> User {
    User {
        id: Uuid::new_v4(),
        email: "user@example.com".into(),
        password_hash: "irrelevant-for-storage-test".into(),
        role: UserRole::Owner,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn create_and_get_user_by_email_round_trips() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let user = sample_user();
    storage.create_user(&user).await.unwrap();

    let fetched = storage.get_user_by_email(&user.email).await.unwrap().unwrap();
    assert_eq!(fetched.id, user.id);
    assert_eq!(fetched.role, UserRole::Owner);
}

#[tokio::test]
async fn get_user_by_email_returns_none_when_missing() {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    assert!(storage.get_user_by_email("nobody@example.com").await.unwrap().is_none());
}
```

- [ ] **Step 6: Run, confirm failure**

Run: `cargo test storage::sqlite::tests::create_and_get_user_by_email_round_trips`
Expected: FAIL (`unimplemented`).

- [ ] **Step 7: Implement**

```rust
async fn create_user(&self, user: &User) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)"
    )
    .bind(user.id.to_string())
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(serde_json::to_string(&user.role)?)
    .bind(user.created_at.to_rfc3339())
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn get_user_by_email(&self, email: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT id, email, password_hash, role, created_at FROM users WHERE email = ?"
    )
    .bind(email)
    .fetch_optional(&self.pool)
    .await?;
    row.map(|(id, email, password_hash, role, created_at)| {
        Ok::<_, anyhow::Error>(User {
            id: Uuid::parse_str(&id)?,
            email,
            password_hash,
            role: serde_json::from_str(&role)?,
            created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        })
    })
    .transpose()
}
```

- [ ] **Step 8: Run tests**

Run: `cargo test`
Expected: full suite PASSES (health, domain, auth, all storage tests).

- [ ] **Step 9: Commit**

```bash
git add src/auth.rs src/storage/sqlite.rs src/lib.rs
git commit -m "feat: add password hashing and SQLite user storage"
```

---

### Task 7: JWT issue/verify + auth API endpoints

**Files:**
- Modify: `src/auth.rs`
- Create: `src/state.rs`
- Create: `src/api/mod.rs`
- Create: `src/api/auth.rs`
- Modify: `src/lib.rs` (add `pub mod state; pub mod api;`)
- Test: `tests/api_test.rs`

**Interfaces:**
- Consumes: `Storage`, `auth::hash_password/verify_password` from Tasks 3-6.
- Produces: `r8r::auth::issue_token(Uuid, &str) -> anyhow::Result<String>`, `r8r::auth::verify_token(&str, &str) -> anyhow::Result<Uuid>`.
- Produces: `r8r::state::AppState { storage: Arc<dyn Storage>, registry: Arc<NodeRegistry>, jwt_secret: String }` (registry field added in Task 9; stub it out as `Arc<()>`-free placeholder is wrong — instead this task defines `AppState` with just `storage` and `jwt_secret`, and Task 9 adds `registry`).
- Produces: `r8r::api::build_router(AppState) -> axum::Router`, mounting `POST /rest/auth/register` and `POST /rest/auth/login`.

- [ ] **Step 1: Write failing JWT unit tests**

```rust
// add to tests module in src/auth.rs
#[test]
fn issue_then_verify_returns_same_user_id() {
    let user_id = uuid::Uuid::new_v4();
    let token = issue_token(user_id, "test-secret").unwrap();
    let verified = verify_token(&token, "test-secret").unwrap();
    assert_eq!(verified, user_id);
}

#[test]
fn verify_fails_with_wrong_secret() {
    let user_id = uuid::Uuid::new_v4();
    let token = issue_token(user_id, "test-secret").unwrap();
    assert!(verify_token(&token, "other-secret").is_err());
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test auth::tests::issue_then_verify`
Expected: compile error, `issue_token`/`verify_token` undefined.

- [ ] **Step 3: Implement JWT functions**

```rust
// add to src/auth.rs, above the tests module
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: usize,
}

pub fn issue_token(user_id: Uuid, secret: &str) -> anyhow::Result<String> {
    let exp = (chrono::Utc::now() + chrono::Duration::hours(24)).timestamp() as usize;
    let claims = Claims { sub: user_id.to_string(), exp };
    Ok(encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))?)
}

pub fn verify_token(token: &str, secret: &str) -> anyhow::Result<Uuid> {
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )?;
    Ok(Uuid::parse_str(&data.claims.sub)?)
}
```

- [ ] **Step 4: Run JWT tests**

Run: `cargo test auth::tests::`
Expected: PASS.

- [ ] **Step 5: Define `AppState`**

```rust
// src/state.rs
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub jwt_secret: String,
}
```

Add `pub mod state;` to `src/lib.rs`.

- [ ] **Step 6: Write failing integration test for register/login**

```rust
// tests/api_test.rs
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use std::sync::Arc;
use tower::ServiceExt;

async fn test_app() -> axum::Router {
    let storage = SqliteStorage::new("sqlite::memory:").await.unwrap();
    let state = AppState {
        storage: Arc::new(storage),
        jwt_secret: "test-secret".into(),
    };
    r8r::api::build_router(state)
}

#[tokio::test]
async fn register_then_login_returns_tokens() {
    let app = test_app().await;

    let register_body = serde_json::json!({"email": "a@b.com", "password": "hunter2"});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(register_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["token"].as_str().unwrap().len() > 10);

    let login_body = serde_json::json!({"email": "a@b.com", "password": "hunter2"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn login_with_wrong_password_returns_401() {
    let app = test_app().await;
    let register_body = serde_json::json!({"email": "c@d.com", "password": "right"});
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(register_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let login_body = serde_json::json!({"email": "c@d.com", "password": "wrong"});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(login_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 7: Run, confirm failure**

Run: `cargo test --test api_test`
Expected: compile error (`r8r::api` doesn't exist yet).

- [ ] **Step 8: Implement the auth API**

```rust
// src/api/auth.rs
use crate::domain::{User, UserRole};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<Credentials>,
) -> impl IntoResponse {
    let password_hash = match crate::auth::hash_password(&payload.password) {
        Ok(h) => h,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "hash failed").into_response(),
    };
    let user = User {
        id: Uuid::new_v4(),
        email: payload.email,
        password_hash,
        role: UserRole::Owner,
        created_at: chrono::Utc::now(),
    };
    if state.storage.create_user(&user).await.is_err() {
        return (StatusCode::CONFLICT, "user already exists").into_response();
    }
    let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
    (StatusCode::CREATED, Json(serde_json::json!({"token": token}))).into_response()
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<Credentials>,
) -> impl IntoResponse {
    let user = match state.storage.get_user_by_email(&payload.email).await {
        Ok(Some(u)) => u,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match crate::auth::verify_password(&payload.password, &user.password_hash) {
        Ok(true) => {
            let token = crate::auth::issue_token(user.id, &state.jwt_secret).unwrap();
            Json(serde_json::json!({"token": token})).into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}
```

```rust
// src/api/mod.rs
pub mod auth;

use crate::state::AppState;
use axum::routing::post;
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .with_state(state)
}
```

Add `pub mod api;` to `src/lib.rs`. Remove the old standalone `health_router()` from `src/lib.rs` (superseded by the route inside `build_router`); update `tests/health_test.rs` to call `r8r::api::build_router` with a throwaway in-memory `AppState` instead, mirroring `test_app()` in `tests/api_test.rs`.

- [ ] **Step 9: Run tests**

Run: `cargo test`
Expected: all tests PASS, including the two new `api_test.rs` tests.

- [ ] **Step 10: Commit**

```bash
git add src/auth.rs src/state.rs src/api/mod.rs src/api/auth.rs src/lib.rs tests/api_test.rs tests/health_test.rs
git commit -m "feat: add JWT auth and register/login endpoints"
```

---

### Task 8: `Node` trait + registry

**Files:**
- Create: `src/node.rs`
- Modify: `src/lib.rs` (add `pub mod node;`)

**Interfaces:**
- Produces: `r8r::node::{Node, NodeExecutionContext, NodeError, NodeRegistry}`.

- [ ] **Step 1: Write failing tests using a throwaway test node**

```rust
// src/node.rs, tests module
#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use crate::domain::Item;

    struct EchoNode;

    #[async_trait]
    impl Node for EchoNode {
        fn type_name(&self) -> &'static str {
            "test.echo"
        }
        async fn execute(&self, ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError> {
            Ok(ctx.input_items.clone())
        }
    }

    #[tokio::test]
    async fn registry_dispatches_to_registered_node() {
        let mut registry = NodeRegistry::new();
        registry.register(Box::new(EchoNode));

        let node = registry.get("test.echo").expect("node should be registered");
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({}),
            input_items: vec![Item { json: serde_json::json!({"x": 1}), binary: serde_json::json!({}) }],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].json, serde_json::json!({"x": 1}));
    }

    #[test]
    fn registry_returns_none_for_unknown_type() {
        let registry = NodeRegistry::new();
        assert!(registry.get("does.not.exist").is_none());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test node::tests::`
Expected: compile error, types undefined.

- [ ] **Step 3: Implement**

```rust
// top of src/node.rs, above the tests module
use crate::domain::Item;
use async_trait::async_trait;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("node execution failed: {0}")]
    ExecutionFailed(String),
}

#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;
    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError>;
}

#[derive(Default)]
pub struct NodeRegistry {
    nodes: HashMap<&'static str, Box<dyn Node>>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, node: Box<dyn Node>) {
        self.nodes.insert(node.type_name(), node);
    }

    pub fn get(&self, type_name: &str) -> Option<&dyn Node> {
        self.nodes.get(type_name).map(|b| b.as_ref())
    }
}
```

Add `pub mod node;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test node::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/node.rs src/lib.rs
git commit -m "feat: add Node trait and NodeRegistry"
```

---

### Task 9: Manual Trigger + Set nodes

**Files:**
- Create: `src/nodes/mod.rs`
- Create: `src/nodes/manual_trigger.rs`
- Create: `src/nodes/set.rs`
- Modify: `src/lib.rs` (add `pub mod nodes;`)
- Modify: `src/state.rs` (add `registry: Arc<NodeRegistry>` field)
- Modify: `src/api/mod.rs`, `tests/api_test.rs`, `tests/health_test.rs` (thread the new `AppState` field through)

**Interfaces:**
- Consumes: `Node` trait from Task 8.
- Produces: `r8r::nodes::register_all(&mut NodeRegistry)`, registering `"core.manualTrigger"` and `"core.set"`.
- `core.set` parameters shape: `{"fields": {"<key>": <json value>, ...}}` — merges (overwrites) these keys into each input item's `json`; if there are no input items, produces exactly one item containing just those fields.

- [ ] **Step 1: Write failing tests for both nodes**

```rust
// src/nodes/manual_trigger.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext};
use async_trait::async_trait;

pub struct ManualTriggerNode;

#[async_trait]
impl Node for ManualTriggerNode {
    fn type_name(&self) -> &'static str {
        "core.manualTrigger"
    }

    async fn execute(&self, _ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError> {
        Ok(vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn produces_exactly_one_empty_item() {
        let node = ManualTriggerNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].json, serde_json::json!({}));
    }
}
```

```rust
// src/nodes/set.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext};
use async_trait::async_trait;

pub struct SetNode;

#[async_trait]
impl Node for SetNode {
    fn type_name(&self) -> &'static str {
        "core.set"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<Vec<Item>, NodeError> {
        let fields = ctx.parameters.get("fields").cloned().unwrap_or(serde_json::json!({}));
        let fields_obj = fields.as_object().cloned().unwrap_or_default();

        let base_items = if ctx.input_items.is_empty() {
            vec![Item { json: serde_json::json!({}), binary: serde_json::json!({}) }]
        } else {
            ctx.input_items.clone()
        };

        let items = base_items
            .into_iter()
            .map(|mut item| {
                let obj = item.json.as_object_mut().expect("item.json must be an object");
                for (k, v) in fields_obj.iter() {
                    obj.insert(k.clone(), v.clone());
                }
                item
            })
            .collect();

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn merges_static_fields_into_each_input_item() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![Item { json: serde_json::json!({"existing": true}), binary: serde_json::json!({}) }],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0].json, serde_json::json!({"existing": true, "greeting": "hi"}));
    }

    #[tokio::test]
    async fn with_no_input_items_produces_one_item_from_fields_only() {
        let node = SetNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
            input_items: vec![],
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].json, serde_json::json!({"greeting": "hi"}));
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test nodes::`
Expected: compile error (`src/nodes/mod.rs` doesn't exist / isn't wired up yet).

- [ ] **Step 3: Wire up `src/nodes/mod.rs`**

```rust
pub mod manual_trigger;
pub mod set;

use crate::node::NodeRegistry;

pub fn register_all(registry: &mut NodeRegistry) {
    registry.register(Box::new(manual_trigger::ManualTriggerNode));
    registry.register(Box::new(set::SetNode));
}
```

Add `pub mod nodes;` to `src/lib.rs`.

- [ ] **Step 4: Run node tests**

Run: `cargo test nodes::`
Expected: PASS.

- [ ] **Step 5: Add `registry` to `AppState` and thread it through**

```rust
// src/state.rs
use crate::node::NodeRegistry;
use crate::storage::Storage;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub registry: Arc<NodeRegistry>,
    pub jwt_secret: String,
}
```

Update `test_app()` in `tests/api_test.rs` and the equivalent state construction in `tests/health_test.rs`:

```rust
let mut registry = r8r::node::NodeRegistry::new();
r8r::nodes::register_all(&mut registry);
let state = AppState {
    storage: Arc::new(storage),
    registry: Arc::new(registry),
    jwt_secret: "test-secret".into(),
};
```

- [ ] **Step 6: Run full suite**

Run: `cargo test`
Expected: everything still PASSES (this step only adds a struct field and updates call sites; no behavior change).

- [ ] **Step 7: Commit**

```bash
git add src/nodes/ src/lib.rs src/state.rs tests/api_test.rs tests/health_test.rs
git commit -m "feat: add ManualTrigger and Set nodes, wire registry into AppState"
```

---

### Task 10: Linear execution engine

**Files:**
- Create: `src/engine.rs`
- Modify: `src/lib.rs` (add `pub mod engine;`)

**Interfaces:**
- Consumes: `domain::{Workflow, NodeInstance, Item}`, `node::{NodeRegistry}` from Tasks 2, 8, 9.
- Produces: `r8r::engine::execute_workflow(&Workflow, &NodeRegistry) -> anyhow::Result<HashMap<String, Vec<Item>>>` — maps each node's `id` to its output items. Requires the workflow's connections to form a single linear chain (no branching); returns an `Err` otherwise. Branching support is out of scope for this plan (see Roadmap, Plan 2).

- [ ] **Step 1: Write failing tests**

```rust
// src/engine.rs, tests module
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Connection, NodeInstance, Workflow};
    use crate::node::NodeRegistry;
    use uuid::Uuid;

    fn linear_workflow() -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "linear".into(),
            active: false,
            nodes: vec![
                NodeInstance {
                    id: "trigger".into(),
                    node_type: "core.manualTrigger".into(),
                    position: (0.0, 0.0),
                    parameters: serde_json::json!({}),
                    disabled: false,
                },
                NodeInstance {
                    id: "set1".into(),
                    node_type: "core.set".into(),
                    position: (1.0, 0.0),
                    parameters: serde_json::json!({"fields": {"greeting": "hi"}}),
                    disabled: false,
                },
            ],
            connections: vec![Connection {
                from_node: "trigger".into(),
                from_output: 0,
                to_node: "set1".into(),
                to_input: 0,
            }],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn registry() -> NodeRegistry {
        let mut r = NodeRegistry::new();
        crate::nodes::register_all(&mut r);
        r
    }

    #[tokio::test]
    async fn executes_trigger_then_set_in_order() {
        let wf = linear_workflow();
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();

        assert_eq!(outputs["trigger"][0].json, serde_json::json!({}));
        assert_eq!(outputs["set1"][0].json, serde_json::json!({"greeting": "hi"}));
    }

    #[tokio::test]
    async fn empty_workflow_produces_empty_outputs() {
        let mut wf = linear_workflow();
        wf.nodes.clear();
        wf.connections.clear();
        let outputs = execute_workflow(&wf, &registry()).await.unwrap();
        assert!(outputs.is_empty());
    }

    #[tokio::test]
    async fn unknown_node_type_returns_error() {
        let mut wf = linear_workflow();
        wf.nodes[1].node_type = "core.doesNotExist".into();
        let result = execute_workflow(&wf, &registry()).await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test engine::tests::`
Expected: compile error, `execute_workflow` undefined.

- [ ] **Step 3: Implement**

```rust
// top of src/engine.rs, above the tests module
use crate::domain::{Item, NodeInstance, Workflow};
use crate::node::{NodeExecutionContext, NodeRegistry};
use std::collections::{HashMap, HashSet};

pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    let order = linear_order(workflow)?;
    let mut outputs: HashMap<String, Vec<Item>> = HashMap::new();
    let mut current_items: Vec<Item> = Vec::new();

    for node_instance in order {
        let node = registry
            .get(&node_instance.node_type)
            .ok_or_else(|| anyhow::anyhow!("unknown node type: {}", node_instance.node_type))?;
        let ctx = NodeExecutionContext {
            parameters: node_instance.parameters.clone(),
            input_items: current_items.clone(),
        };
        let result = node
            .execute(&ctx)
            .await
            .map_err(|e| anyhow::anyhow!("node {} failed: {e}", node_instance.id))?;
        outputs.insert(node_instance.id.clone(), result.clone());
        current_items = result;
    }
    Ok(outputs)
}

fn linear_order(workflow: &Workflow) -> anyhow::Result<Vec<NodeInstance>> {
    if workflow.nodes.is_empty() {
        return Ok(Vec::new());
    }
    let targets: HashSet<&str> = workflow.connections.iter().map(|c| c.to_node.as_str()).collect();
    let start = workflow
        .nodes
        .iter()
        .find(|n| !targets.contains(n.id.as_str()))
        .ok_or_else(|| anyhow::anyhow!("no start node found (cycle or empty graph)"))?;

    let mut order = vec![start.clone()];
    let mut current_id = start.id.clone();
    while let Some(conn) = workflow.connections.iter().find(|c| c.from_node == current_id) {
        let next = workflow
            .nodes
            .iter()
            .find(|n| n.id == conn.to_node)
            .ok_or_else(|| anyhow::anyhow!("dangling connection to {}", conn.to_node))?;
        order.push(next.clone());
        current_id = next.id.clone();
    }
    Ok(order)
}
```

Add `pub mod engine;` to `src/lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test engine::tests::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs src/lib.rs
git commit -m "feat: add linear workflow execution engine"
```

---

### Task 11: Workflow CRUD API + auth middleware

**Files:**
- Create: `src/api/workflows.rs`
- Modify: `src/api/mod.rs`
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: `Storage` workflow methods, `AppState`.
- Produces: an axum extractor `AuthUser(Uuid)` requiring `Authorization: Bearer <token>`, verified via `auth::verify_token`; returns `401` if missing/invalid.
- Produces: `POST /rest/workflows`, `GET /rest/workflows`, `GET /rest/workflows/:id` (all requiring `AuthUser`).

- [ ] **Step 1: Write failing integration tests**

```rust
// add to tests/api_test.rs
async fn register_and_get_token(app: &axum::Router, email: &str) -> String {
    let body = serde_json::json!({"email": email, "password": "hunter2"});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    json["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn create_workflow_requires_auth() {
    let app = test_app().await;
    let body = serde_json::json!({"name": "wf1", "nodes": [], "connections": []});
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn create_then_get_then_list_workflow() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "wfuser@example.com").await;

    let body = serde_json::json!({"name": "wf1", "nodes": [], "connections": []});
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let created: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let response = app.clone()
        .oneshot(
            Request::builder()
                .uri(format!("/rest/workflows/{id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/rest/workflows")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test --test api_test`
Expected: compile error (routes don't exist).

- [ ] **Step 3: Implement the `AuthUser` extractor and workflow handlers**

```rust
// src/api/workflows.rs
use crate::domain::{Connection, NodeInstance, Workflow};
use crate::state::AppState;
use axum::async_trait;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

pub struct AuthUser(pub Uuid);

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let token = header.strip_prefix("Bearer ").ok_or(StatusCode::UNAUTHORIZED)?;
        let user_id = crate::auth::verify_token(token, &state.jwt_secret)
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        Ok(AuthUser(user_id))
    }
}

#[derive(Deserialize)]
pub struct CreateWorkflowRequest {
    pub name: String,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
}

pub async fn create_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Json(payload): Json<CreateWorkflowRequest>,
) -> impl IntoResponse {
    let now = chrono::Utc::now();
    let workflow = Workflow {
        id: Uuid::new_v4(),
        name: payload.name,
        active: false,
        nodes: payload.nodes,
        connections: payload.connections,
        created_at: now,
        updated_at: now,
    };
    match state.storage.create_workflow(&workflow).await {
        Ok(()) => (StatusCode::CREATED, Json(workflow)).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn get_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => Json(wf).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_workflows(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
) -> impl IntoResponse {
    match state.storage.list_workflows().await {
        Ok(workflows) => Json(workflows).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
```

```rust
// src/api/mod.rs
pub mod auth;
pub mod workflows;

use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/rest/workflows", post(workflows::create_workflow).get(workflows::list_workflows))
        .route("/rest/workflows/:id", get(workflows::get_workflow))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: all PASS.

- [ ] **Step 5: Commit**

```bash
git add src/api/workflows.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: add workflow CRUD API with JWT-authenticated routes"
```

---

### Task 12: Execute endpoint + get-execution endpoint

**Files:**
- Create: `src/api/executions.rs`
- Modify: `src/api/workflows.rs` (add `execute_workflow` handler)
- Modify: `src/api/mod.rs`
- Modify: `tests/api_test.rs`

**Interfaces:**
- Consumes: `engine::execute_workflow`, `Storage` execution methods.
- Produces: `POST /rest/workflows/:id/execute` (auth required) → runs the engine, persists an `Execution`, returns it as JSON. `GET /rest/executions/:id` (auth required) → returns the persisted `Execution`.

- [ ] **Step 1: Write failing integration test — the full end-to-end happy path**

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn create_execute_and_fetch_execution_end_to_end() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "exec@example.com").await;

    let workflow_body = serde_json::json!({
        "name": "exec-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "set1", "node_type": "core.set", "position": [1.0, 0.0], "parameters": {"fields": {"greeting": "hi"}}, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "set1", "to_input": 0}
        ]
    });
    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(workflow_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rest/workflows/{workflow_id}/execute"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["set1"][0]["json"]["greeting"], "hi");
    let execution_id = execution["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/rest/executions/{execution_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test --test api_test create_execute_and_fetch_execution_end_to_end`
Expected: compile error / 404 (route doesn't exist).

- [ ] **Step 3: Implement the execute handler**

```rust
// add to src/api/workflows.rs
use crate::domain::{Execution, ExecutionMode, ExecutionStatus};

pub async fn execute_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    let mut execution = Execution {
        id: Uuid::new_v4(),
        workflow_id: workflow.id,
        status: ExecutionStatus::Running,
        mode: ExecutionMode::Manual,
        node_outputs: Default::default(),
        started_at: chrono::Utc::now(),
        finished_at: None,
    };
    if state.storage.create_execution(&execution).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    match crate::engine::execute_workflow(&workflow, &state.registry).await {
        Ok(outputs) => {
            execution.status = ExecutionStatus::Success;
            execution.node_outputs = outputs;
        }
        Err(_) => {
            execution.status = ExecutionStatus::Error;
        }
    }
    execution.finished_at = Some(chrono::Utc::now());
    if state.storage.update_execution(&execution).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    Json(execution).into_response()
}
```

```rust
// src/api/executions.rs
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use uuid::Uuid;

pub async fn get_execution(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.storage.get_execution(id).await {
        Ok(Some(exec)) => Json(exec).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
```

```rust
// src/api/mod.rs
pub mod auth;
pub mod executions;
pub mod workflows;

use crate::state::AppState;
use axum::routing::{get, post};
use axum::Router;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        .route("/rest/auth/login", post(auth::login))
        .route("/rest/workflows", post(workflows::create_workflow).get(workflows::list_workflows))
        .route("/rest/workflows/:id", get(workflows::get_workflow))
        .route("/rest/workflows/:id/execute", post(workflows::execute_workflow))
        .route("/rest/executions/:id", get(executions::get_execution))
        .route("/health", get(|| async { "ok" }))
        .with_state(state)
}
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test`
Expected: all tests PASS, including the new end-to-end test.

- [ ] **Step 5: Commit**

```bash
git add src/api/executions.rs src/api/workflows.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: add workflow execute endpoint and get-execution endpoint"
```

---

### Task 13: Wire up `main.rs` for real runtime use

**Files:**
- Modify: `src/main.rs`
- Create: `.env.example`

**Interfaces:**
- Produces: a runnable binary reading `DATABASE_URL`, `JWT_SECRET`, `PORT` from the environment (with sane local-dev defaults), connecting real SQLite storage, registering real nodes, and serving `r8r::api::build_router`.

- [ ] **Step 1: Write `.env.example`**

```
DATABASE_URL=sqlite:./r8r.db?mode=rwc
JWT_SECRET=change-me-in-production
PORT=3000
```

- [ ] **Step 2: Rewrite `src/main.rs`**

```rust
use r8r::node::NodeRegistry;
use r8r::state::AppState;
use r8r::storage::sqlite::SqliteStorage;
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:./r8r.db?mode=rwc".into());
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "dev-secret-change-me".into());
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);

    let storage = SqliteStorage::new(&database_url).await?;
    let mut registry = NodeRegistry::new();
    r8r::nodes::register_all(&mut registry);

    let state = AppState {
        storage: Arc::new(storage),
        registry: Arc::new(registry),
        jwt_secret,
    };

    let app = r8r::api::build_router(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    tracing::info!("r8r listening on :{port}");
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 3: Build and manually smoke-test**

Run: `cargo build && DATABASE_URL="sqlite:./r8r-dev.db?mode=rwc" JWT_SECRET=dev cargo run &`

Then in another shell:

```bash
curl -s -X POST localhost:3000/rest/auth/register -H 'content-type: application/json' \
  -d '{"email":"me@example.com","password":"hunter2"}'
# → {"token": "..."}

TOKEN="<paste token>"
curl -s -X POST localhost:3000/rest/workflows -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/json' \
  -d '{"name":"demo","nodes":[{"id":"t","node_type":"core.manualTrigger","position":[0,0],"parameters":{},"disabled":false},{"id":"s","node_type":"core.set","position":[1,0],"parameters":{"fields":{"greeting":"hi"}},"disabled":false}],"connections":[{"from_node":"t","from_output":0,"to_node":"s","to_input":0}]}'
# → {"id": "...", ...}

WF_ID="<paste id>"
curl -s -X POST "localhost:3000/rest/workflows/$WF_ID/execute" -H "authorization: Bearer $TOKEN"
# → {"status":"Success", "node_outputs":{"s":[{"json":{"greeting":"hi"},...}], ...}}
```

Expected: each call returns the shape shown above. Stop the background server afterward (`kill %1` or `fg` then Ctrl-C).

- [ ] **Step 4: Run the full automated test suite one more time**

Run: `cargo test`
Expected: all tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs .env.example
echo "/target
*.db
.env" > .gitignore
git add .gitignore
git commit -m "feat: wire up main.rs for real runtime use with env-based config"
```

---

## Roadmap (future plans, not part of this one)

Each of these becomes its own `docs/superpowers/plans/*.md` document, written via the brainstorming → writing-plans flow when work on it starts, per the spec's phased non-goals (spec §10):

- **Plan 2 — Expression engine + control-flow nodes:** embed QuickJS (`rquickjs`), upgrade `Set`/introduce `Code`, add `If`/`Switch`/`Merge`/`Filter`/`Wait`/`NoOp`, upgrade the execution engine from linear-only to full DAG topological scheduling with branch/merge semantics and per-node error-output routing.
- **Plan 3 — Triggers:** `Webhook` node + `/webhook/:workflow_id/:path` route, `Schedule` node via `tokio-cron-scheduler`, workflow `active` flag actually enabling/disabling registered triggers.
- **Plan 4 — HTTP integrations:** generic `HTTP Request` node, `Credential` storage (encrypted at rest) + API, `Telegram Trigger` and `Telegram` nodes built on top of the HTTP Request pattern.
- **Plan 5 — AI Agent node:** LLM provider HTTP clients, tool-calling loop exposing other nodes/sub-workflows as callable tools, per-execution conversational memory buffer.
- **Plan 6 — Frontend:** Vue 3 + Vue Flow SPA against the existing REST/WebSocket API; canvas editing, inline JSON data preview, execution log/replay view.
- **Plan 7 — Execution hardening:** WebSocket live execution status push, manual retry-from-failed-node, incremental per-node execution-data persistence (today's engine persists only the final snapshot).
