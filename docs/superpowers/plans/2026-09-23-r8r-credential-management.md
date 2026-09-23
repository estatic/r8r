# Credential Management Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Edit (name + secret values, blank = keep), delete (blocked while used), and a Credentials page, without ever returning secrets.

**Architecture:** Storage gains `update_credential`/`delete_credential`. Pure helpers in `src/credentials.rs` compute usage (`workflows_using_credential`), merge patches (`merge_credential_data`), and pick non-secret fields (`non_secret_fields`). `src/api/credentials.rs` adds GET/PATCH/DELETE by id and `used_by` on the list. The frontend extracts Plan 8.4's create form into `CredentialForm.vue` (create + edit modes), reuses it in `CredentialPicker` (plus an Edit link) and a new `/credentials` page.

**Tech Stack:** Rust (axum, sqlx/SQLite), Vue 3 + TypeScript + Pinia, Vitest.

**Spec:** `docs/superpowers/specs/2026-09-23-r8r-credential-management-design.md`

## Global Constraints

- Never return password-type field values or untyped credential data from any endpoint; never log credential data.
- PATCH never changes `credential_type`, `owner_id`, `created_at`; `updated_at` = now.
- Typed PATCH: merge per schema field; `""`/`null`/absent = keep; keys outside the schema ignored. Untyped PATCH: replace data. Blank/whitespace `name` → 400; non-object `data` → 400; missing id → 404.
- DELETE in use → 409 `{ "error": "credential is in use", "workflows": [{ "id", "name" }] }`.
- "In use" = some node's `parameters.auth.credential_id` equals the credential id.
- Edit-mode password placeholder text: `•••••• (unchanged)`.

## Review Focus

- A credential whose stored `data` is not an object (legacy/raw JSON) must still accept a typed PATCH without panicking — pinned in Task 2 (`merge_into_non_object_starts_fresh`).
- Renaming only (no `data`) must not touch secrets — pinned in Task 3 (`patch_name_only_keeps_secret`).
- Edit mode must not wipe pre-filled values when the type is set programmatically (create-mode "type change clears fields" logic) — pinned in Task 4 (`edit mode pre-fills text fields`).
- The picker's existing create flow must behave exactly as before after extraction — pinned by the 11 existing `CredentialPicker.spec.ts` tests staying green (Task 4).
- A 409 body that isn't the expected JSON must still produce a readable delete error — pinned in Task 5 (`inUseWorkflowNames` returns null → generic message).

---

### Task 1: Storage update and delete

**Files:**
- Modify: `src/storage/mod.rs` (trait, after `list_credentials` at line 32)
- Modify: `src/storage/sqlite.rs` (impl after `list_credentials` ~line 233; tests module)
- Modify: test `Storage` impls — `src/execution_runner.rs` (`CountingStorage`, ~line 312), `src/telegram_poller.rs` (`RecordingStorage`, ~line 534), `tests/api_test.rs` (`FailingUpdateStorage`, ~line 113)

**Interfaces:**
- Produces: `Storage::update_credential(&self, credential: &Credential) -> anyhow::Result<bool>`, `Storage::delete_credential(&self, id: Uuid) -> anyhow::Result<bool>` (`false` = no such id).

- [ ] **Step 1: Write the failing tests** — append inside `mod tests` in `src/storage/sqlite.rs`:

```rust
    #[tokio::test]
    async fn update_credential_rewrites_name_and_data_but_not_type_or_created_at() {
        let storage = storage_with_test_key().await;
        let user = sample_user();
        storage.create_user(&user).await.unwrap();
        let cred = sample_credential(user.id);
        storage.create_credential(&cred).await.unwrap();

        let mut changed = cred.clone();
        changed.name = "renamed".into();
        changed.data = serde_json::json!({"token": "new-secret"});
        changed.credential_type = "somethingElse".into();
        changed.updated_at = Utc::now() + chrono::Duration::seconds(5);
        assert!(storage.update_credential(&changed).await.unwrap());

        let fetched = storage.get_credential(cred.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "renamed");
        assert_eq!(fetched.data, serde_json::json!({"token": "new-secret"}));
        assert_eq!(fetched.credential_type, cred.credential_type);
        assert_eq!(fetched.created_at.timestamp(), cred.created_at.timestamp());
        assert_eq!(fetched.updated_at.timestamp(), changed.updated_at.timestamp());
    }

    #[tokio::test]
    async fn update_credential_returns_false_when_missing() {
        let storage = storage_with_test_key().await;
        assert!(!storage.update_credential(&sample_credential(Uuid::new_v4())).await.unwrap());
    }

    #[tokio::test]
    async fn delete_credential_removes_it_and_reports_missing_ids() {
        let storage = storage_with_test_key().await;
        let user = sample_user();
        storage.create_user(&user).await.unwrap();
        let cred = sample_credential(user.id);
        storage.create_credential(&cred).await.unwrap();

        assert!(storage.delete_credential(cred.id).await.unwrap());
        assert!(storage.get_credential(cred.id).await.unwrap().is_none());
        assert!(!storage.delete_credential(cred.id).await.unwrap());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib storage::sqlite 2>&1 | grep -E "^error\[" | sort | uniq -c`
Expected: `no method named update_credential` / `delete_credential`.

- [ ] **Step 3: Implement** — in `src/storage/mod.rs` add after `list_credentials`:

```rust
    /// Rewrites `name`, `data` (re-encrypted) and `updated_at`; never the
    /// type, owner or `created_at`. `Ok(false)` if no such credential.
    async fn update_credential(&self, credential: &Credential) -> anyhow::Result<bool>;
    /// `Ok(false)` if no such credential.
    async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool>;
```

In `src/storage/sqlite.rs` add after `list_credentials` in `impl Storage for SqliteStorage`:

```rust
    async fn update_credential(&self, credential: &Credential) -> anyhow::Result<bool> {
        let plaintext = serde_json::to_string(&credential.data)?;
        let encrypted = crate::crypto::encrypt(&self.encryption_key, &plaintext)?;
        let result = sqlx::query("UPDATE credentials SET name = ?, data = ?, updated_at = ? WHERE id = ?")
            .bind(&credential.name)
            .bind(encrypted)
            .bind(credential.updated_at.to_rfc3339())
            .bind(credential.id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM credentials WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
```

In each of the three test wrappers, add next to their `list_credentials` (all three delegate to `self.inner`; use the domain path each file already uses for `Credential`, e.g. `crate::domain::Credential` / `r8r::domain::Credential`):

```rust
        async fn update_credential(&self, credential: &crate::domain::Credential) -> anyhow::Result<bool> {
            self.inner.update_credential(credential).await
        }
        async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool> {
            self.inner.delete_credential(id).await
        }
```

- [ ] **Step 4: Run to verify**

Run: `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/storage src/execution_runner.rs src/telegram_poller.rs tests/api_test.rs
git commit -m "feat: storage update_credential and delete_credential"
```

---

### Task 2: Usage, merge and non-secret helpers

**Files:**
- Modify: `src/credentials.rs` (new pub fns; tests module)

**Interfaces:**
- Produces:
  - `pub fn workflows_using_credential(workflows: &[Workflow], id: Uuid) -> Vec<(Uuid, String)>`
  - `pub fn merge_credential_data(schema: Option<&crate::credential_types::CredentialTypeSchema>, stored: &serde_json::Value, patch: &serde_json::Value) -> serde_json::Value`
  - `pub fn non_secret_fields(schema: Option<&crate::credential_types::CredentialTypeSchema>, stored: &serde_json::Value) -> serde_json::Map<String, serde_json::Value>`
  - `pub fn schema_for(credential_type: &str) -> Option<&'static crate::credential_types::CredentialTypeSchema>`

- [ ] **Step 1: Write the failing tests** — append inside the tests module of `src/credentials.rs` (create `#[cfg(test)] mod tests { use super::*; }` at the end of the file if absent; reuse its existing workflow helper if one exists, otherwise use the one below):

```rust
    fn wf_with_auth(name: &str, credential_id: Option<&str>) -> crate::domain::Workflow {
        let params = match credential_id {
            Some(id) => serde_json::json!({"auth": {"type": "bearer", "credential_id": id}}),
            None => serde_json::json!({"url": "https://example.com"}),
        };
        crate::domain::Workflow {
            id: Uuid::new_v4(),
            name: name.into(),
            active: false,
            nodes: vec![crate::domain::NodeInstance {
                id: "n1".into(),
                node_type: "core.httpRequest".into(),
                position: (0.0, 0.0),
                parameters: params,
                disabled: false,
                settings: Default::default(),
            }],
            connections: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn finds_workflows_that_reference_the_credential() {
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let using = wf_with_auth("uses it", Some(&id.to_string()));
        let workflows = vec![using.clone(), wf_with_auth("other cred", Some(&other.to_string())), wf_with_auth("no auth", None)];
        assert_eq!(workflows_using_credential(&workflows, id), vec![(using.id, "uses it".to_string())]);
    }

    #[test]
    fn typed_merge_keeps_blank_fields_and_ignores_unknown_keys() {
        let schema = schema_for("apiKeyHeader");
        let stored = serde_json::json!({"header_name": "X-Key", "value": "old-secret"});
        let patch = serde_json::json!({"header_name": "X-Api-Key", "value": "", "extra": "nope"});
        assert_eq!(
            merge_credential_data(schema, &stored, &patch),
            serde_json::json!({"header_name": "X-Api-Key", "value": "old-secret"})
        );
        let patch = serde_json::json!({"value": "new-secret"});
        assert_eq!(
            merge_credential_data(schema, &stored, &patch),
            serde_json::json!({"header_name": "X-Key", "value": "new-secret"})
        );
    }

    #[test]
    fn untyped_merge_replaces_everything() {
        let stored = serde_json::json!({"a": 1, "b": 2});
        let patch = serde_json::json!({"c": 3});
        assert_eq!(merge_credential_data(None, &stored, &patch), serde_json::json!({"c": 3}));
    }

    #[test]
    fn merge_into_non_object_starts_fresh() {
        let schema = schema_for("bearerToken");
        let stored = serde_json::json!("legacy string");
        let patch = serde_json::json!({"token": "t"});
        assert_eq!(merge_credential_data(schema, &stored, &patch), serde_json::json!({"token": "t"}));
    }

    #[test]
    fn non_secret_fields_returns_text_fields_only() {
        let schema = schema_for("apiKeyHeader");
        let stored = serde_json::json!({"header_name": "X-Key", "value": "secret"});
        let fields = non_secret_fields(schema, &stored);
        assert_eq!(fields.get("header_name"), Some(&serde_json::json!("X-Key")));
        assert!(!fields.contains_key("value"));
        assert!(non_secret_fields(None, &stored).is_empty());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib credentials::tests 2>&1 | grep -E "^error\[" | sort | uniq -c`
Expected: cannot find `workflows_using_credential`, `merge_credential_data`, `non_secret_fields`, `schema_for`.

- [ ] **Step 3: Implement** — add to `src/credentials.rs` (above the tests module):

```rust
use crate::credential_types::{known_credential_types, CredentialTypeSchema, FieldType};

/// The field schema for a credential type, if it is one of the known types.
pub fn schema_for(credential_type: &str) -> Option<&'static CredentialTypeSchema> {
    known_credential_types().iter().find(|s| s.credential_type == credential_type)
}

/// `(id, name)` of every workflow with a node whose
/// `parameters.auth.credential_id` is `id` -- the field
/// `resolve_credentials_for_workflow` reads.
pub fn workflows_using_credential(workflows: &[crate::domain::Workflow], id: Uuid) -> Vec<(Uuid, String)> {
    let id = id.to_string();
    workflows
        .iter()
        .filter(|wf| {
            wf.nodes.iter().any(|n| {
                n.parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str()) == Some(id.as_str())
            })
        })
        .map(|wf| (wf.id, wf.name.clone()))
        .collect()
}

/// Applies a PATCH body to stored credential data. Typed (schema known):
/// per schema field, a non-blank patch value replaces the stored one;
/// blank/absent keeps it; other keys are ignored. Untyped: replace.
pub fn merge_credential_data(
    schema: Option<&CredentialTypeSchema>,
    stored: &serde_json::Value,
    patch: &serde_json::Value,
) -> serde_json::Value {
    let Some(schema) = schema else {
        return patch.clone();
    };
    let mut merged = stored.as_object().cloned().unwrap_or_default();
    for field in schema.fields {
        match patch.get(field.name) {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::String(s)) if s.is_empty() => {}
            Some(v) => {
                merged.insert(field.name.to_string(), v.clone());
            }
        }
    }
    serde_json::Value::Object(merged)
}

/// Stored values of the schema's text fields only -- never password
/// fields, and nothing for an untyped credential.
pub fn non_secret_fields(
    schema: Option<&CredentialTypeSchema>,
    stored: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    if let Some(schema) = schema {
        for field in schema.fields.iter().filter(|f| matches!(f.field_type, FieldType::Text)) {
            if let Some(v) = stored.get(field.name) {
                out.insert(field.name.to_string(), v.clone());
            }
        }
    }
    out
}
```

If `Uuid` isn't already imported in `src/credentials.rs`, add `use uuid::Uuid;`.

- [ ] **Step 4: Run to verify**

Run: `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add src/credentials.rs
git commit -m "feat: credential usage, patch-merge and non-secret-field helpers"
```

---

### Task 3: Credential API — used_by, GET, PATCH, DELETE

**Files:**
- Modify: `src/api/credentials.rs`
- Modify: `src/api/mod.rs` (route `/rest/credentials/:id`)
- Test: `tests/api_test.rs`

**Interfaces:**
- Consumes: Task 1 storage methods; Task 2 helpers.
- Produces (JSON): list item / PATCH response = `CredentialSummary` fields + `used_by: number`; GET-by-id = those + `fields: {name: value}`; DELETE 409 body `{error, workflows: [{id, name}]}`.

- [ ] **Step 1: Write the failing tests** — append to `tests/api_test.rs`:

```rust
async fn send(app: &axum::Router, method: &str, uri: &str, token: &str, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
    let mut req = Request::builder().method(method).uri(uri).header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(b) => {
            req = req.header("content-type", "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

async fn create_cred(app: &axum::Router, token: &str, name: &str, ty: &str, data: serde_json::Value) -> String {
    let (status, body) = send(app, "POST", "/rest/credentials", token, Some(serde_json::json!({"name": name, "credential_type": ty, "data": data}))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn get_credential_by_id_returns_text_fields_but_never_secrets() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-get@example.com").await;
    let id = create_cred(&app, &token, "hdr", "apiKeyHeader", serde_json::json!({"header_name": "X-Key", "value": "top-secret"})).await;
    let (status, body) = send(&app, "GET", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["fields"]["header_name"], "X-Key");
    assert!(body["fields"].get("value").is_none());
    assert!(!body.to_string().contains("top-secret"));
    assert_eq!(body["used_by"], 0);
    let (status, _) = send(&app, "GET", &format!("/rest/credentials/{}", uuid::Uuid::new_v4()), &token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn patch_name_only_keeps_secret() {
    let (app, state) = test_app_with_state().await;
    let token = register_and_get_token(&app, "cred-rename@example.com").await;
    let id = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "123:ABC"})).await;
    let (status, body) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"name": "  renamed bot  "}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "renamed bot");
    let stored = state.storage.get_credential(id.parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(stored.data, serde_json::json!({"bot_token": "123:ABC"}));
    assert_eq!(stored.credential_type, "telegramApi");
}

#[tokio::test]
async fn patch_merges_typed_data_and_replaces_untyped_data() {
    let (app, state) = test_app_with_state().await;
    let token = register_and_get_token(&app, "cred-patch@example.com").await;
    let typed = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "old"})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{typed}"), &token, Some(serde_json::json!({"data": {"bot_token": ""}}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.storage.get_credential(typed.parse().unwrap()).await.unwrap().unwrap().data, serde_json::json!({"bot_token": "old"}));
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{typed}"), &token, Some(serde_json::json!({"data": {"bot_token": "new"}, "credential_type": "bearerToken"}))).await;
    assert_eq!(s, StatusCode::OK);
    let stored = state.storage.get_credential(typed.parse().unwrap()).await.unwrap().unwrap();
    assert_eq!(stored.data, serde_json::json!({"bot_token": "new"}));
    assert_eq!(stored.credential_type, "telegramApi");

    let untyped = create_cred(&app, &token, "custom", "myCustomThing", serde_json::json!({"a": 1})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{untyped}"), &token, Some(serde_json::json!({"data": {"b": 2}}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(state.storage.get_credential(untyped.parse().unwrap()).await.unwrap().unwrap().data, serde_json::json!({"b": 2}));
}

#[tokio::test]
async fn patch_rejects_blank_name_bad_data_and_missing_ids() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-bad@example.com").await;
    let id = create_cred(&app, &token, "bot", "telegramApi", serde_json::json!({"bot_token": "x"})).await;
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"name": "   "}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{id}"), &token, Some(serde_json::json!({"data": "not an object"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = send(&app, "PATCH", &format!("/rest/credentials/{}", uuid::Uuid::new_v4()), &token, Some(serde_json::json!({"name": "x"}))).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_is_refused_while_a_workflow_uses_the_credential() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred-del@example.com").await;
    let id = create_cred(&app, &token, "bearer", "bearerToken", serde_json::json!({"token": "t"})).await;
    let wf_body = serde_json::json!({
        "name": "uses-cred",
        "nodes": [{"id": "h", "node_type": "core.httpRequest", "position": [0.0, 0.0], "parameters": {"url": "https://example.com", "auth": {"type": "bearer", "credential_id": id}}}],
        "connections": []
    });
    let (s, wf) = send(&app, "POST", "/rest/workflows", &token, Some(wf_body)).await;
    assert_eq!(s, StatusCode::CREATED);
    let wf_id = wf["id"].as_str().unwrap().to_string();

    let (s, list) = send(&app, "GET", "/rest/credentials", &token, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().iter().find(|c| c["id"] == id.as_str()).unwrap()["used_by"], 1);

    let (s, body) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(body["error"], "credential is in use");
    assert_eq!(body["workflows"][0]["name"], "uses-cred");

    let (s, _) = send(&app, "DELETE", &format!("/rest/workflows/{wf_id}"), &token, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = send(&app, "DELETE", &format!("/rest/credentials/{id}"), &token, None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --test api_test cred 2>&1 | grep -E "^test |test result"`
Expected: the new tests FAIL (405/404 — routes missing, no `used_by`).

- [ ] **Step 3: Implement** — in `src/api/credentials.rs`: add `use axum::extract::Path;` and `use crate::credentials::{merge_credential_data, non_secret_fields, schema_for, workflows_using_credential};`, then add:

```rust
/// A list/PATCH response item: the summary plus how many workflows use it.
#[derive(serde::Serialize)]
pub struct CredentialListItem {
    #[serde(flatten)]
    pub summary: CredentialSummary,
    pub used_by: usize,
}

/// GET-by-id response: never contains password-type values.
#[derive(serde::Serialize)]
pub struct CredentialDetail {
    #[serde(flatten)]
    pub summary: CredentialSummary,
    pub used_by: usize,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
pub struct UpdateCredentialRequest {
    pub name: Option<String>,
    pub data: Option<serde_json::Value>,
}

async fn all_workflows(state: &AppState) -> Result<Vec<crate::domain::Workflow>, axum::response::Response> {
    state.storage.list_workflows().await.map_err(|e| {
        tracing::error!(error = %e, "failed to list workflows for credential usage");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })
}

pub async fn get_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let credential = match state.storage.get_credential(id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(CredentialDetail {
        used_by: workflows_using_credential(&workflows, id).len(),
        fields: non_secret_fields(schema_for(&credential.credential_type), &credential.data),
        summary: CredentialSummary::from(&credential),
    })
    .into_response()
}

pub async fn update_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateCredentialRequest>,
) -> axum::response::Response {
    if let Some(name) = &payload.name {
        if name.trim().is_empty() {
            return (StatusCode::BAD_REQUEST, "name must not be blank").into_response();
        }
    }
    if let Some(data) = &payload.data {
        if !data.is_object() {
            return (StatusCode::BAD_REQUEST, "data must be a JSON object").into_response();
        }
    }
    let mut credential = match state.storage.get_credential(id).await {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch credential for update");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if let Some(name) = payload.name {
        credential.name = name.trim().to_string();
    }
    if let Some(data) = payload.data {
        credential.data = merge_credential_data(schema_for(&credential.credential_type), &credential.data, &data);
    }
    credential.updated_at = chrono::Utc::now();
    match state.storage.update_credential(&credential).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to update credential");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }
    tracing::info!(credential_id = %credential.id, name = %credential.name, "credential updated");
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    Json(CredentialListItem {
        used_by: workflows_using_credential(&workflows, id).len(),
        summary: CredentialSummary::from(&credential),
    })
    .into_response()
}

pub async fn delete_credential(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let users = workflows_using_credential(&workflows, id);
    if !users.is_empty() {
        let list: Vec<serde_json::Value> =
            users.iter().map(|(wf_id, name)| serde_json::json!({"id": wf_id, "name": name})).collect();
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "credential is in use", "workflows": list})),
        )
            .into_response();
    }
    match state.storage.delete_credential(id).await {
        Ok(true) => {
            tracing::info!(credential_id = %id, "credential deleted");
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete credential");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Replace the body of `list_credentials` with:

```rust
    let summaries = match state.storage.list_credentials().await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to list credentials");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let workflows = match all_workflows(&state).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let items: Vec<CredentialListItem> = summaries
        .into_iter()
        .map(|summary| CredentialListItem { used_by: workflows_using_credential(&workflows, summary.id).len(), summary })
        .collect();
    Json(items).into_response()
```

(and change `list_credentials`'s return type to `axum::response::Response` if `impl IntoResponse` complains about the early return type).

In `src/api/mod.rs`, after the `/rest/credentials` route add:

```rust
        .route(
            "/rest/credentials/:id",
            get(credentials::get_credential)
                .patch(credentials::update_credential)
                .delete(credentials::delete_credential),
        )
```

- [ ] **Step 4: Run to verify**

Run: `cargo test 2>&1 | grep -E "^test result|FAILED|^(warning|error)"`
Expected: all pass (existing credential list test still passes — it only reads existing fields).

- [ ] **Step 5: Commit**

```bash
git add src/api/credentials.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: credential GET/PATCH/DELETE by id and used_by counts"
```

---

### Task 4: `CredentialForm` (create + edit) and the picker's Edit link

**Files:**
- Modify: `frontend/src/types/domain.ts` (`CredentialSummary`, add `CredentialDetail`)
- Modify: `frontend/src/stores/credentials.ts`
- Create: `frontend/src/components/CredentialForm.vue`
- Create: `frontend/src/components/CredentialForm.spec.ts`
- Modify: `frontend/src/components/CredentialPicker.vue`
- Modify: `frontend/src/components/CredentialPicker.spec.ts` (one new test)

**Interfaces:**
- Consumes: Task 3 JSON.
- Produces: `CredentialForm` props `{ mode: 'create' | 'edit'; acceptedTypes?: string[]; credentialId?: string; offerAllTypes?: boolean }`, emits `saved(summary: CredentialSummary)`, `cancel()`. Store: `get(id): Promise<CredentialDetail>`, `update(id, patch: { name?: string; data?: Record<string, unknown> }): Promise<CredentialSummary>`, `remove(id): Promise<void>`, and `export function inUseWorkflowNames(e: unknown): string[] | null`.

- [ ] **Step 1: Types and store** — in `types/domain.ts` add `used_by: number` to `CredentialSummary` and:

```typescript
export interface CredentialDetail extends CredentialSummary {
  fields: Record<string, string>
}
```

Replace `frontend/src/stores/credentials.ts` with:

```typescript
import { defineStore } from 'pinia'
import { api, ApiError } from '../api/client'
import type { CredentialDetail, CredentialSummary } from '../types/domain'

/** Workflow names from a DELETE 409 body, or null if `e` isn't one. */
export function inUseWorkflowNames(e: unknown): string[] | null {
  if (!(e instanceof ApiError) || e.status !== 409) return null
  try {
    const body = JSON.parse(e.message) as { workflows?: { name: string }[] }
    return Array.isArray(body.workflows) ? body.workflows.map((w) => w.name) : null
  } catch {
    return null
  }
}

export const useCredentialsStore = defineStore('credentials', {
  state: () => ({
    credentials: [] as CredentialSummary[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      this.credentials = await api.get<CredentialSummary[]>('/rest/credentials')
      this.loaded = true
    },
    async create(name: string, credentialType: string, data: Record<string, unknown>): Promise<CredentialSummary> {
      const summary = await api.post<CredentialSummary>('/rest/credentials', {
        name,
        credential_type: credentialType,
        data,
      })
      const withUsage = { ...summary, used_by: summary.used_by ?? 0 }
      this.credentials.push(withUsage)
      return withUsage
    },
    async get(id: string): Promise<CredentialDetail> {
      return api.get<CredentialDetail>(`/rest/credentials/${id}`)
    },
    async update(id: string, patch: { name?: string; data?: Record<string, unknown> }): Promise<CredentialSummary> {
      const summary = await api.patch<CredentialSummary>(`/rest/credentials/${id}`, patch)
      await this.fetchAll()
      return summary
    },
    async remove(id: string): Promise<void> {
      await api.delete<void>(`/rest/credentials/${id}`)
      await this.fetchAll()
    },
  },
})
```

- [ ] **Step 2: Write the failing tests** — create `frontend/src/components/CredentialForm.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialForm from './CredentialForm.vue'

const SCHEMAS = [
  { credential_type: 'apiKeyHeader', display_name: 'API Key (Header)', generic: true, fields: [
    { name: 'header_name', label: 'Header Name', field_type: 'text', required: true },
    { name: 'value', label: 'Value', field_type: 'password', required: true },
  ] },
]

function stub(detail: unknown, onPatch: (body: unknown) => void) {
  vi.stubGlobal('fetch', vi.fn((url: string, options?: RequestInit) => {
    if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => SCHEMAS })
    if (url === '/rest/credentials/c1' && !options?.method) return Promise.resolve({ ok: true, status: 200, json: async () => detail })
    if (url === '/rest/credentials/c1' && options?.method === 'PATCH') {
      onPatch(JSON.parse(options.body as string))
      return Promise.resolve({ ok: true, status: 200, json: async () => ({ ...(detail as object), used_by: 0 }) })
    }
    if (url === '/rest/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => [] })
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  }))
}

describe('CredentialForm', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('edit mode pre-fills text fields and leaves secrets blank with the unchanged placeholder', async () => {
    stub({ id: 'c1', name: 'My header', credential_type: 'apiKeyHeader', owner_id: 'u', created_at: '', updated_at: '', used_by: 2, fields: { header_name: 'X-Key' } }, () => {})
    const wrapper = mount(CredentialForm, { props: { mode: 'edit', credentialId: 'c1' } })
    await flushPromises()
    expect((wrapper.find('input[aria-label="Name"]').element as HTMLInputElement).value).toBe('My header')
    expect((wrapper.find('input[aria-label="Header Name"]').element as HTMLInputElement).value).toBe('X-Key')
    const secret = wrapper.find('input[aria-label="Value"]')
    expect((secret.element as HTMLInputElement).value).toBe('')
    expect(secret.attributes('placeholder')).toBe('•••••• (unchanged)')
    expect(wrapper.text()).toContain('apiKeyHeader')
    vi.unstubAllGlobals()
  })

  it('edit mode submits the name and only non-empty field values', async () => {
    let patched: unknown = null
    stub({ id: 'c1', name: 'My header', credential_type: 'apiKeyHeader', owner_id: 'u', created_at: '', updated_at: '', used_by: 0, fields: { header_name: 'X-Key' } }, (b) => { patched = b })
    const wrapper = mount(CredentialForm, { props: { mode: 'edit', credentialId: 'c1' } })
    await flushPromises()
    await wrapper.find('input[aria-label="Name"]').setValue('Renamed')
    await wrapper.find('button.bg-blue-600').trigger('click')
    await flushPromises()
    expect(patched).toEqual({ name: 'Renamed', data: { header_name: 'X-Key' } })
    expect(wrapper.emitted('saved')).toBeTruthy()
    vi.unstubAllGlobals()
  })
})
```

Append to `CredentialPicker.spec.ts` inside its `describe`:

```typescript
  it('shows an Edit link only when a credential is selected', async () => {
    stubFetch([{ type_name: 'core.httpRequest', display_name: 'HTTP Request', icon: '🌐', category: 'action', description: '', credential_types: [], output_ports: ['main'] }])
    const none = mount(CredentialPicker, { props: { modelValue: null, nodeType: 'core.httpRequest' } })
    expect(none.find('[data-testid="edit-credential"]').exists()).toBe(false)
    const some = mount(CredentialPicker, { props: { modelValue: 'c1', nodeType: 'core.httpRequest' } })
    expect(some.find('[data-testid="edit-credential"]').exists()).toBe(true)
    vi.unstubAllGlobals()
  })
```

- [ ] **Step 3: Run to verify they fail**

Run: `cd frontend && npx vitest run src/components/CredentialForm.spec.ts src/components/CredentialPicker.spec.ts 2>&1 | grep -E "×|FAIL|Tests"`
Expected: CredentialForm spec fails to resolve the component; the new picker test fails.

- [ ] **Step 4: Create `CredentialForm.vue`**:

```vue
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useCredentialTypesStore } from '../stores/credentialTypes'

const props = defineProps<{
  mode: 'create' | 'edit'
  acceptedTypes?: string[]
  credentialId?: string
  offerAllTypes?: boolean
}>()
const emit = defineEmits<{ saved: [summary: import('../types/domain').CredentialSummary]; cancel: [] }>()

const store = useCredentialsStore()
const credentialTypesStore = useCredentialTypesStore()
const newName = ref('')
const newType = ref('')
const useCustomType = ref(false)
const fieldValues = ref<Record<string, string>>({})
const newDataText = ref(props.mode === 'create' ? '{}' : '')
const error = ref('')
const editing = props.mode === 'edit'

if (!credentialTypesStore.loaded) {
  credentialTypesStore.fetchAll().catch(() => {})
}

const accepted = computed(() => props.acceptedTypes ?? [])
const hasRestrictedTypes = computed(() => !editing && accepted.value.length > 0)
const selectableTypes = computed(() =>
  props.offerAllTypes ? credentialTypesStore.types : credentialTypesStore.types.filter((t) => t.generic),
)
const schemaForSelectedType = computed(
  () => credentialTypesStore.types.find((t) => t.credential_type === newType.value) ?? null,
)

// Create mode: a type change drops values typed for the previous schema so
// they can't leak into the submission. Edit mode sets the type once while
// pre-filling, so it must not clear.
watch(newType, () => {
  if (!editing) fieldValues.value = {}
  error.value = ''
})

// Create mode: auto-select a single accepted type, and drop a type picked
// from the generic list before the node's restricted list arrived.
watch(
  accepted,
  (types) => {
    if (editing) return
    if (types.length === 1) {
      newType.value = types[0]
    } else if (types.length > 0 && !types.includes(newType.value)) {
      newType.value = ''
      useCustomType.value = false
    }
  },
  { immediate: true },
)

if (editing && props.credentialId) {
  store
    .get(props.credentialId)
    .then((detail) => {
      newName.value = detail.name
      newType.value = detail.credential_type
      fieldValues.value = { ...detail.fields }
    })
    .catch(() => {
      error.value = 'Failed to load credential.'
    })
}

function selectGenericType(value: string) {
  if (value === '__custom__') {
    useCustomType.value = true
    newType.value = ''
  } else {
    useCustomType.value = false
    newType.value = value
  }
}

function trimmedFieldData(schemaFields: { name: string }[]): Record<string, unknown> {
  // Built from the schema (not the accumulated map) and trimmed; blanks
  // dropped -- omitted on create, "keep current" on edit.
  return Object.fromEntries(
    schemaFields.map((f) => [f.name, (fieldValues.value[f.name] ?? '').trim()]).filter(([, v]) => v !== ''),
  )
}

async function submit() {
  error.value = ''
  const schema = schemaForSelectedType.value
  if (editing) {
    if (!newName.value.trim()) {
      error.value = 'Name is required.'
      return
    }
    const patch: { name: string; data?: Record<string, unknown> } = { name: newName.value.trim() }
    if (schema) {
      const data = trimmedFieldData(schema.fields)
      if (Object.keys(data).length > 0) patch.data = data
    } else if (newDataText.value.trim()) {
      try {
        patch.data = JSON.parse(newDataText.value)
      } catch {
        error.value = 'Data must be valid JSON.'
        return
      }
    }
    try {
      emit('saved', await store.update(props.credentialId!, patch))
    } catch {
      error.value = 'Failed to save credential.'
    }
    return
  }

  let data: Record<string, unknown>
  if (schema) {
    const missing = schema.fields.filter((f) => f.required && !fieldValues.value[f.name]?.trim())
    if (missing.length > 0) {
      error.value = `${missing.map((f) => f.label).join(', ')} ${missing.length === 1 ? 'is' : 'are'} required.`
      return
    }
    data = trimmedFieldData(schema.fields)
  } else {
    try {
      data = JSON.parse(newDataText.value)
    } catch {
      error.value = 'Data must be valid JSON.'
      return
    }
  }
  try {
    emit('saved', await store.create(newName.value, newType.value, data))
  } catch {
    // Keep the form and its contents so the user can correct and retry.
    error.value = 'Failed to create credential.'
  }
}
</script>

<template>
  <div class="border rounded p-2 space-y-2 bg-gray-50">
    <!-- autocomplete opt-outs keep password managers from treating this as
         a login form and filling the r8r login into credential secrets. -->
    <input v-model="newName" placeholder="Name" aria-label="Name" autocomplete="off" class="w-full border rounded px-2 py-1 text-sm" />

    <p v-if="editing" class="text-xs text-gray-600">Type: <span class="font-mono">{{ newType }}</span></p>
    <template v-else>
      <select v-if="hasRestrictedTypes" v-model="newType" class="w-full border rounded px-2 py-1 text-sm">
        <option value="" disabled>Select a type…</option>
        <option v-for="t in accepted" :key="t" :value="t">{{ t }}</option>
      </select>
      <select
        v-else-if="!useCustomType"
        :value="newType"
        class="w-full border rounded px-2 py-1 text-sm"
        @change="selectGenericType(($event.target as HTMLSelectElement).value)"
      >
        <option value="" disabled>Select a type…</option>
        <option v-for="t in selectableTypes" :key="t.credential_type" :value="t.credential_type">{{ t.display_name }}</option>
        <option value="__custom__">Custom…</option>
      </select>
      <input v-else v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />
    </template>

    <div v-if="schemaForSelectedType" class="space-y-1">
      <input
        v-for="f in schemaForSelectedType.fields"
        :key="f.name"
        v-model="fieldValues[f.name]"
        :type="f.field_type === 'password' ? 'password' : 'text'"
        :placeholder="editing && f.field_type === 'password' ? '•••••• (unchanged)' : f.label"
        :aria-label="f.label"
        :autocomplete="f.field_type === 'password' ? 'new-password' : 'off'"
        class="w-full border rounded px-2 py-1 text-sm"
      />
    </div>
    <template v-else>
      <p v-if="editing" class="text-xs text-gray-500">Enter the full JSON to replace the stored data, or leave empty to keep it.</p>
      <textarea v-model="newDataText" rows="3" :placeholder="editing ? '' : '{}'" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
    </template>

    <div class="flex gap-2">
      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="submit">{{ editing ? 'Save' : 'Create' }}</button>
      <button type="button" class="text-xs text-gray-600" @click="emit('cancel')">Cancel</button>
    </div>
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>
```

- [ ] **Step 5: Slim `CredentialPicker.vue` down to use it** — replace the script and template with:

```vue
<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useCredentialsStore } from '../stores/credentials'
import { useNodeTypesStore } from '../stores/nodeTypes'
import CredentialForm from './CredentialForm.vue'
import type { CredentialSummary } from '../types/domain'

const props = defineProps<{ modelValue: string | null; nodeType: string }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const nodeTypesStore = useNodeTypesStore()
const creating = ref(false)
const editing = ref(false)
const error = ref('')

if (!store.loaded) {
  store.fetchAll().catch(() => {
    error.value = 'Failed to load credentials.'
  })
}
if (!nodeTypesStore.loaded) {
  nodeTypesStore.fetchAll().catch(() => {})
}

const acceptedTypes = computed(
  () => nodeTypesStore.types.find((t) => t.type_name === props.nodeType)?.credential_types ?? [],
)

watch(
  () => props.nodeType,
  () => {
    creating.value = false
    editing.value = false
  },
)

function toggleCreating() {
  creating.value = !creating.value
  editing.value = false
}

function onCreated(summary: CredentialSummary) {
  emit('update:modelValue', summary.id)
  creating.value = false
}
</script>

<template>
  <div class="space-y-2">
    <select
      :value="modelValue ?? ''"
      class="w-full border rounded px-2 py-1.5 text-sm"
      @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value || null)"
    >
      <option value="">No credential</option>
      <option v-for="c in store.credentials" :key="c.id" :value="c.id">{{ c.name }} ({{ c.credential_type }})</option>
    </select>
    <div class="flex gap-3">
      <button type="button" class="text-xs text-blue-600" @click="toggleCreating">+ New credential</button>
      <button
        v-if="modelValue"
        type="button"
        data-testid="edit-credential"
        class="text-xs text-blue-600"
        @click="editing = !editing; creating = false"
      >
        Edit
      </button>
    </div>
    <CredentialForm v-if="creating" :key="nodeType" mode="create" :accepted-types="acceptedTypes" @saved="onCreated" @cancel="creating = false" />
    <CredentialForm v-if="editing && modelValue" :key="modelValue" mode="edit" :credential-id="modelValue" @saved="editing = false" @cancel="editing = false" />
    <!-- Outside the forms so load failures are visible too. -->
    <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
  </div>
</template>
```

- [ ] **Step 6: Run to verify**

Run: `cd frontend && npx vitest run 2>&1 | grep -E "×|Tests " && npx vue-tsc --noEmit && echo tsc-ok`
Expected: all pass, including all pre-existing `CredentialPicker.spec.ts` tests unchanged, and `tsc-ok`. If a pre-existing picker test fails, the extraction changed behaviour — fix the form, not the test.

- [ ] **Step 7: Commit**

```bash
git add frontend/src/types/domain.ts frontend/src/stores/credentials.ts frontend/src/components/CredentialForm.vue frontend/src/components/CredentialForm.spec.ts frontend/src/components/CredentialPicker.vue frontend/src/components/CredentialPicker.spec.ts
git commit -m "feat: reusable CredentialForm with edit mode; Edit link in the credential picker"
```

---

### Task 5: Credentials page

**Files:**
- Create: `frontend/src/views/CredentialsView.vue`
- Create: `frontend/src/views/CredentialsView.spec.ts`
- Modify: `frontend/src/router/index.ts`
- Modify: `frontend/src/views/WorkflowListView.vue` (header link)

**Interfaces:**
- Consumes: `CredentialForm` (Task 4), store `fetchAll`/`remove`/`inUseWorkflowNames` (Task 4).

- [ ] **Step 1: Write the failing tests** — create `frontend/src/views/CredentialsView.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import CredentialsView from './CredentialsView.vue'

const CREDS = [
  { id: 'c1', name: 'Bot', credential_type: 'telegramApi', owner_id: 'u', created_at: '', updated_at: '2026-09-23T10:00:00Z', used_by: 2 },
  { id: 'c2', name: 'Spare', credential_type: 'bearerToken', owner_id: 'u', created_at: '', updated_at: '2026-09-23T10:00:00Z', used_by: 0 },
]

function stub(deleteStatus: number, deleteBody = '') {
  let list = [...CREDS]
  vi.stubGlobal('fetch', vi.fn((url: string, options?: RequestInit) => {
    if (url === '/rest/credentials') return Promise.resolve({ ok: true, status: 200, json: async () => list })
    if (url === '/rest/credential-types') return Promise.resolve({ ok: true, status: 200, json: async () => [] })
    if (options?.method === 'DELETE') {
      if (deleteStatus === 204) {
        list = list.filter((c) => !url.endsWith(c.id))
        return Promise.resolve({ ok: true, status: 204, json: async () => undefined })
      }
      return Promise.resolve({ ok: false, status: deleteStatus, text: async () => deleteBody })
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`))
  }))
}

const mountView = () => mount(CredentialsView, { global: { stubs: { RouterLink: true } } })

describe('CredentialsView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.spyOn(window, 'confirm').mockReturnValue(true)
  })

  it('lists credentials with their usage counts', async () => {
    stub(204)
    const wrapper = mountView()
    await flushPromises()
    const rows = wrapper.findAll('[data-testid="credential-row"]')
    expect(rows).toHaveLength(2)
    expect(rows[0].text()).toContain('Bot')
    expect(rows[0].text()).toContain('used by 2 workflows')
    expect(rows[1].text()).toContain('not used')
    vi.unstubAllGlobals()
  })

  it('names the workflows when deletion is refused', async () => {
    stub(409, JSON.stringify({ error: 'credential is in use', workflows: [{ id: 'w1', name: 'Daily digest' }] }))
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Can\'t delete "Bot": used by Daily digest. Remove it from those workflows first.')
    vi.unstubAllGlobals()
  })

  it('falls back to a generic message for an unexpected 409 body', async () => {
    stub(409, 'not json')
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[0].trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('Failed to delete credential.')
    vi.unstubAllGlobals()
  })

  it('removes the row after a successful delete', async () => {
    stub(204)
    const wrapper = mountView()
    await flushPromises()
    await wrapper.findAll('[data-testid="delete-credential"]')[1].trigger('click')
    await flushPromises()
    expect(wrapper.findAll('[data-testid="credential-row"]')).toHaveLength(1)
    vi.unstubAllGlobals()
  })
})
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd frontend && npx vitest run src/views/CredentialsView.spec.ts 2>&1 | grep -E "FAIL|Error:|Tests" | head -3`
Expected: cannot resolve `./CredentialsView.vue`.

- [ ] **Step 3: Implement** — create `frontend/src/views/CredentialsView.vue`:

```vue
<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { inUseWorkflowNames, useCredentialsStore } from '../stores/credentials'
import CredentialForm from '../components/CredentialForm.vue'
import type { CredentialSummary } from '../types/domain'

const store = useCredentialsStore()
const creating = ref(false)
const editingId = ref<string | null>(null)
const error = ref('')

onMounted(() => {
  store.fetchAll().catch(() => {
    error.value = 'Failed to load credentials.'
  })
})

function usage(c: CredentialSummary): string {
  if (!c.used_by) return 'not used'
  return `used by ${c.used_by} workflow${c.used_by === 1 ? '' : 's'}`
}

async function remove(c: CredentialSummary) {
  error.value = ''
  if (!window.confirm(`Delete credential "${c.name}"?`)) return
  try {
    await store.remove(c.id)
  } catch (e) {
    const names = inUseWorkflowNames(e)
    error.value = names
      ? `Can't delete "${c.name}": used by ${names.join(', ')}. Remove it from those workflows first.`
      : 'Failed to delete credential.'
  }
}

async function afterSave() {
  creating.value = false
  editingId.value = null
  await store.fetchAll().catch(() => {})
}
</script>

<template>
  <main class="min-h-screen bg-gray-50">
    <header class="bg-white border-b px-6 py-4 flex justify-between items-center">
      <h1 class="text-xl font-semibold text-gray-800">Credentials</h1>
      <router-link to="/workflows" class="text-sm text-blue-600">Workflows</router-link>
    </header>
    <div class="p-6 max-w-3xl mx-auto space-y-4">
      <button type="button" class="bg-blue-600 text-white rounded px-4 py-2 text-sm" @click="creating = !creating; editingId = null">
        + New credential
      </button>
      <CredentialForm v-if="creating" mode="create" offer-all-types @saved="afterSave" @cancel="creating = false" />
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <ul class="divide-y bg-white rounded shadow">
        <li v-for="c in store.credentials" :key="c.id" data-testid="credential-row" class="px-4 py-3 space-y-2">
          <div class="flex justify-between items-center gap-4">
            <div>
              <div class="font-medium text-gray-800">{{ c.name }}</div>
              <div class="text-xs text-gray-500">
                <span class="font-mono">{{ c.credential_type }}</span> · {{ usage(c) }} · updated {{ new Date(c.updated_at).toLocaleString() }}
              </div>
            </div>
            <div class="flex gap-3 text-sm">
              <button type="button" class="text-blue-600" @click="editingId = editingId === c.id ? null : c.id; creating = false">Edit</button>
              <button type="button" data-testid="delete-credential" class="text-red-600" @click="remove(c)">Delete</button>
            </div>
          </div>
          <CredentialForm v-if="editingId === c.id" :key="c.id" mode="edit" :credential-id="c.id" @saved="afterSave" @cancel="editingId = null" />
        </li>
      </ul>
      <p v-if="store.loaded && store.credentials.length === 0" class="text-sm text-gray-500">No credentials yet.</p>
    </div>
  </main>
</template>
```

In `frontend/src/router/index.ts` add after the `/workflows/:id` route:

```typescript
    {
      path: '/credentials',
      name: 'credentials',
      component: () => import('../views/CredentialsView.vue'),
      meta: { requiresAuth: true },
    },
```

In `WorkflowListView.vue`'s header, replace `<button class="text-sm text-gray-500" @click="logout">Log out</button>` with:

```html
      <div class="flex gap-4 items-center">
        <router-link to="/credentials" class="text-sm text-blue-600">Credentials</router-link>
        <button class="text-sm text-gray-500" @click="logout">Log out</button>
      </div>
```

- [ ] **Step 4: Run to verify**

Run: `cd frontend && npx vitest run 2>&1 | grep -E "×|Tests " && npx vue-tsc --noEmit && echo tsc-ok`
Expected: all pass, `tsc-ok`. (If an existing `WorkflowListView`/`App` test counts header buttons, adjust nothing in the test — the new element is a link, not a button.)

- [ ] **Step 5: Commit**

```bash
git add frontend/src/views/CredentialsView.vue frontend/src/views/CredentialsView.spec.ts frontend/src/router/index.ts frontend/src/views/WorkflowListView.vue
git commit -m "feat: Credentials page with edit and guarded delete"
```

---

### Task 6: Rebuild the embedded frontend and verify end to end

**Files:** none tracked (`frontend/dist` is gitignored).

- [ ] **Step 1: Build**

Run: `cd frontend && npm run build 2>&1 | tail -3`
Expected: `✓ built`.

- [ ] **Step 2: Full suites**

Run: `cargo test 2>&1 | grep -E "^test result|FAILED"` and `cd frontend && npx vitest run 2>&1 | grep -E "Tests "`
Expected: all pass.

- [ ] **Step 3: Smoke-test the running app** — start `r8r` on a spare port with a fresh temp database, register, create a `telegramApi` credential via the API, then in a real browser: open `/credentials`, rename it via Edit (secret left blank), and delete it. Expected: the row shows the new name; the stored token is unchanged (check via `GET /rest/credentials/:id` shows no secret and the rename persisted); delete removes the row. No commit.
