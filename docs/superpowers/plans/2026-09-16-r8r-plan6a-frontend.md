# r8r Frontend (Plan 6a — First Slice) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working Vue 3 SPA — login/register, a workflow list, and a
canvas editor (add/connect/configure nodes, pick credentials, save,
execute, see results) — served from the same r8r binary as the API.

**Architecture:** Vite + Vue 3 + TypeScript + Pinia + Vue Router + Vue Flow
+ Tailwind, built to `frontend/dist/` and embedded into the Rust binary via
`rust-embed`, served by Axum as a fallback route (so `/rest/*`, `/webhook/*`,
and `/health` keep priority, and any other path serves the SPA's
`index.html` for Vue Router's history-mode routing to take over). Dev mode
runs Vite's own dev server with a proxy to a separately-running `cargo run`
backend.

**Tech Stack:** Vue 3, TypeScript, Vite, Pinia, Vue Router 4, `@vue-flow/core`,
Tailwind CSS, Vitest + `@vue/test-utils`. Backend: `rust-embed` (new
dependency).

**Spec:** `docs/superpowers/specs/2026-09-16-r8r-plan6a-frontend-design.md`
— reachable, read directly before writing this plan; authoritative.

## Global Constraints

- **Serving:** the built SPA is embedded into the binary at Rust compile
  time via `rust-embed` reading `frontend/dist/`. This means
  `frontend/dist/` must exist (via `npm run build`) before `cargo build`
  succeeds — a real, accepted build-order dependency for this deployment
  choice (see spec §3). `frontend/node_modules/` and `frontend/dist/` are
  gitignored build artifacts, never committed.
- **Auth:** JWT stored in `localStorage` under the key `r8r_token`. A 401
  from any API call clears it and redirects to `/login`.
- **Styling:** Tailwind CSS utility classes, hand-built components — no
  component library (PrimeVue, Naive UI, etc.).
- **Node parameters and credential data:** edited as raw, client-validated
  JSON (a textarea, parsed before being accepted). The backend has no
  per-node parameter schema today; dynamic schema-driven forms are
  explicitly out of scope for this plan (spec §2 Non-Goals). The one
  exception is `parameters.auth.credential_id`, which gets a first-class
  dropdown control (§Task 9) since that exact shape is already consistent
  across every credential-using node in this codebase.
- **No execution-conflict handling:** last save wins. No optimistic
  locking, no multi-editor awareness.
- **`@vue-flow/core`'s exact prop/event names**: this plan's code uses
  Vue Flow's documented v1 API to the best of the plan author's knowledge,
  but — unlike this project's Rust dependencies, which have all been used
  and verified in this exact codebase already — this is this project's
  first use of this library. If a prop, event, or type name in this
  plan's Vue Flow code doesn't match what the actually-installed version
  exposes (check its own TypeScript types / node_modules, or its docs),
  adjust to the real API while preserving the same intent (node
  positions render and update, connections can be drawn, node clicks are
  detected) — this is a normal "verify against the real
  compiler/library", not a sign the plan is broken, the same allowance
  this project's Rust plans have used for less-familiar crate APIs.
- **TypeScript strict mode** is on (`tsconfig.json`'s `"strict": true`).
  Every file must type-check cleanly (`npm run build` runs `vue-tsc
  --noEmit` first).

---

### Task 1: Backend — workflow update/delete + node-types endpoint

**Files:**
- Modify: `src/storage/mod.rs` (add `delete_workflow` to the `Storage` trait)
- Modify: `src/storage/sqlite.rs` (implement `delete_workflow`)
- Modify: `src/node.rs` (add `NodeRegistry::type_names`)
- Modify: `src/api/workflows.rs` (add `update_workflow`, `delete_workflow` handlers)
- Create: `src/api/node_types.rs`
- Modify: `src/api/mod.rs` (register the new module + routes)
- Test: inline `#[cfg(test)]` modules in the above, plus new tests in `tests/api_test.rs`

**Interfaces:**
- Produces: `PUT /rest/workflows/:id` (body: `{"name", "nodes", "connections"}`,
  returns the updated `Workflow`), `DELETE /rest/workflows/:id` (returns
  204, deactivates triggers first if the workflow was active), `GET
  /rest/node-types` (returns `["core.httpRequest", "core.manualTrigger",
  ...]`, sorted). All three require the existing `AuthUser` extractor,
  matching every other authenticated route in this file.
- Consumes: nothing new — reuses `Storage::update_workflow` (already
  exists, already used internally by `set_workflow_active`),
  `crate::triggers::deactivate_workflow_triggers` (already exists).

- [ ] **Step 1: `Storage::delete_workflow` + SQLite implementation, with tests**

In `src/storage/mod.rs`, add to the `Storage` trait (next to
`update_workflow`):

```rust
    async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()>;
```

In `src/storage/sqlite.rs`, add the implementation (next to
`update_workflow`'s), matching the existing bind/execute style exactly:

```rust
    async fn delete_workflow(&self, id: Uuid) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM workflows WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

Add a test alongside the existing `update_workflow_persists_active_flag_and_name`
test:

```rust
    #[tokio::test]
    async fn delete_workflow_removes_it() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        let wf = sample_workflow();
        storage.create_workflow(&wf).await.unwrap();

        storage.delete_workflow(wf.id).await.unwrap();

        assert!(storage.get_workflow(wf.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_workflow_on_a_nonexistent_id_does_not_error() {
        let storage = SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap();
        storage.delete_workflow(Uuid::new_v4()).await.unwrap();
    }
```

- [ ] **Step 2: `NodeRegistry::type_names`, with a test**

In `src/node.rs`, add to `impl NodeRegistry` (next to `get`):

```rust
    /// All registered node type names, sorted for a stable, predictable
    /// order in any UI listing them (e.g. an "add node" picker).
    pub fn type_names(&self) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self.nodes.keys().copied().collect();
        names.sort_unstable();
        names
    }
```

Add a test to the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn type_names_returns_registered_types_sorted() {
        let mut registry = NodeRegistry::new();
        registry.register(Box::new(EchoNode));
        assert_eq!(registry.type_names(), vec!["test.echo"]);
    }
```

- [ ] **Step 3: `update_workflow` and `delete_workflow` HTTP handlers, with tests**

In `src/api/workflows.rs`, add (next to `CreateWorkflowRequest`):

```rust
#[derive(Deserialize)]
pub struct UpdateWorkflowRequest {
    pub name: String,
    pub nodes: Vec<NodeInstance>,
    pub connections: Vec<Connection>,
}

/// Deliberately does not touch `active` or trigger activation state — that
/// stays the job of `PATCH .../active`. This is safe even for a currently
/// active workflow: every trigger implementation in this codebase
/// (`fire_schedule`, `handle_webhook`, `poll_telegram_updates`) already
/// re-fetches the workflow fresh on each firing, so an edit here takes
/// effect on the trigger's next firing with no special-casing needed.
pub async fn update_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateWorkflowRequest>,
) -> impl IntoResponse {
    let mut workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for update");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    workflow.name = payload.name;
    workflow.nodes = payload.nodes;
    workflow.connections = payload.connections;
    workflow.updated_at = chrono::Utc::now();
    match state.storage.update_workflow(&workflow).await {
        Ok(()) => Json(workflow).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to persist workflow update");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn delete_workflow(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let workflow = match state.storage.get_workflow(id).await {
        Ok(Some(wf)) => wf,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch workflow for deletion");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // A deleted workflow's background trigger (a cron job or a Telegram
    // long-poll task) must be torn down explicitly — deleting the row
    // doesn't stop a task that's already spawned and holding no reference
    // back to storage's row-existence.
    if workflow.active {
        crate::triggers::deactivate_workflow_triggers(&state, workflow.id).await;
    }
    match state.storage.delete_workflow(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to delete workflow");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

Add integration tests to `tests/api_test.rs` (check its existing imports/
helpers first — `test_app()`, `register_and_get_token()` already exist and
should be reused exactly as other tests in that file use them):

```rust
#[tokio::test]
async fn update_workflow_persists_new_nodes_and_name() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "update-wf@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "original", "nodes": [], "connections": []}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let update_body = serde_json::json!({
        "name": "renamed",
        "nodes": [{"id": "n1", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false}],
        "connections": []
    });
    let update_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(update_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::OK);
    let bytes = update_response.into_body().collect().await.unwrap().to_bytes();
    let updated: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(updated["name"], "renamed");
    assert_eq!(updated["nodes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn update_workflow_on_missing_id_returns_404() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "update-missing@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/rest/workflows/{}", Uuid::new_v4()))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"name": "x", "nodes": [], "connections": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_workflow_removes_it_and_then_404s() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "delete-wf@example.com").await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"name": "to-delete", "nodes": [], "connections": []}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let delete_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let get_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_an_active_workflow_deactivates_its_trigger_first() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "delete-active-wf@example.com").await;

    let create_body = serde_json::json!({
        "name": "active-to-delete",
        "nodes": [{"id": "t", "node_type": "core.schedule", "position": [0.0, 0.0], "parameters": {"cron": "0 0 0 1 1 *"}, "disabled": false}],
        "connections": []
    });
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/workflows")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(create_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = create_response.into_body().collect().await.unwrap().to_bytes();
    let workflow_id = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["id"].as_str().unwrap().to_string();

    let activate_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/rest/workflows/{workflow_id}/active"))
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::json!({"active": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(activate_response.status(), StatusCode::OK);

    // Must not hang or error just because the workflow is active with a
    // live cron job registered — deletion has to tear that down cleanly.
    let delete_response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rest/workflows/{workflow_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
}
```

- [ ] **Step 4: `GET /rest/node-types`, with a test**

Create `src/api/node_types.rs`:

```rust
use crate::state::AppState;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;

pub async fn list_node_types(
    State(state): State<AppState>,
    super::workflows::AuthUser(_user_id): super::workflows::AuthUser,
) -> impl IntoResponse {
    Json(state.registry.type_names()).into_response()
}
```

Add an integration test to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn node_types_lists_registered_types() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "node-types@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/rest/node-types")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let types: Vec<String> = serde_json::from_slice(&bytes).unwrap();
    assert!(types.contains(&"core.manualTrigger".to_string()));
    assert!(types.contains(&"telegram.sendMessage".to_string()));
    // Sorted.
    let mut sorted = types.clone();
    sorted.sort();
    assert_eq!(types, sorted);
}
```

- [ ] **Step 5: Wire routes**

In `src/api/mod.rs`, add `pub mod node_types;` to the module list, and
change:

```rust
        .route("/rest/workflows/:id", get(workflows::get_workflow))
```

to:

```rust
        .route(
            "/rest/workflows/:id",
            get(workflows::get_workflow)
                .put(workflows::update_workflow)
                .delete(workflows::delete_workflow),
        )
```

and add a new route line (anywhere among the other `/rest/*` routes):

```rust
        .route("/rest/node-types", get(node_types::list_node_types))
```

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: all tests pass, including the 7 new ones added in this task.

- [ ] **Step 7: Commit**

```bash
git add src/storage/mod.rs src/storage/sqlite.rs src/node.rs src/api/workflows.rs src/api/node_types.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: add workflow update/delete and node-types listing endpoints"
```

---

### Task 2: Frontend project scaffold

**Files:**
- Create: `frontend/package.json`, `frontend/vite.config.ts`,
  `frontend/tsconfig.json`, `frontend/tsconfig.node.json`,
  `frontend/tailwind.config.js`, `frontend/postcss.config.js`,
  `frontend/index.html`, `frontend/src/main.ts`, `frontend/src/App.vue`,
  `frontend/src/style.css`, `frontend/src/App.spec.ts`
- Modify: `.gitignore` (repo root)

**Interfaces:**
- Produces: a buildable Vite/Vue project — `npm run build` (run from
  `frontend/`) produces `frontend/dist/index.html` and its assets, which
  Task 3 embeds. No router, no Pinia, no API calls yet — deliberately
  minimal so this task has zero dependency on anything not yet built.

- [ ] **Step 1: `package.json`**

Create `frontend/package.json`:

```json
{
  "name": "r8r-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vue-tsc --noEmit && vite build",
    "preview": "vite preview",
    "test": "vitest run"
  },
  "dependencies": {
    "vue": "^3.5.0",
    "vue-router": "^4.4.0",
    "pinia": "^2.2.0",
    "@vue-flow/core": "^1.42.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^5.1.0",
    "@vue/test-utils": "^2.4.0",
    "autoprefixer": "^10.4.0",
    "jsdom": "^25.0.0",
    "postcss": "^8.4.0",
    "tailwindcss": "^3.4.0",
    "typescript": "^5.6.0",
    "vite": "^5.4.0",
    "vitest": "^2.1.0",
    "vue-tsc": "^2.1.0"
  }
}
```

- [ ] **Step 2: Vite, TypeScript, Tailwind config**

Create `frontend/vite.config.ts`:

```typescript
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    proxy: {
      '/rest': 'http://localhost:3000',
      '/webhook': 'http://localhost:3000',
    },
  },
  test: {
    environment: 'jsdom',
  },
})
```

Create `frontend/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2020",
    "useDefineForClassFields": true,
    "module": "ESNext",
    "lib": ["ES2020", "DOM", "DOM.Iterable"],
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "preserve",
    "strict": true
  },
  "include": ["src/**/*.ts", "src/**/*.d.ts", "src/**/*.vue"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

Create `frontend/tsconfig.node.json`:

```json
{
  "compilerOptions": {
    "composite": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true
  },
  "include": ["vite.config.ts"]
}
```

Create `frontend/tailwind.config.js`:

```javascript
/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{vue,js,ts,jsx,tsx}'],
  theme: {
    extend: {},
  },
  plugins: [],
}
```

Create `frontend/postcss.config.js`:

```javascript
export default {
  plugins: {
    tailwindcss: {},
    autoprefixer: {},
  },
}
```

- [ ] **Step 3: App entry point**

Create `frontend/index.html`:

```html
<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>r8r</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

Create `frontend/src/style.css`:

```css
@tailwind base;
@tailwind components;
@tailwind utilities;
```

Create `frontend/src/App.vue`:

```vue
<script setup lang="ts"></script>

<template>
  <main class="min-h-screen bg-gray-50 flex items-center justify-center">
    <h1 class="text-3xl font-semibold text-gray-800">r8r</h1>
  </main>
</template>
```

Create `frontend/src/main.ts`:

```typescript
import { createApp } from 'vue'
import App from './App.vue'
import './style.css'

createApp(App).mount('#app')
```

- [ ] **Step 4: Smoke test**

Create `frontend/src/App.spec.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import App from './App.vue'

describe('App', () => {
  it('renders the r8r heading', () => {
    const wrapper = mount(App)
    expect(wrapper.text()).toContain('r8r')
  })
})
```

- [ ] **Step 5: Update root `.gitignore`**

Add to `.gitignore` (repo root):

```
frontend/node_modules
frontend/dist
```

- [ ] **Step 6: Install, test, build**

Run (from `frontend/`):
```bash
npm install
npm test
npm run build
```
Expected: `npm test` passes (1 test), `npm run build` succeeds and
produces `frontend/dist/index.html` plus a hashed JS/CSS bundle. If any
package version pinned above has a real incompatibility with another
(dependency resolution conflicts happen), adjust the minor/patch version
in `package.json` to the nearest compatible release and note the change
in your report — the versions above are current-as-of-writing, not
gospel.

- [ ] **Step 7: Commit**

```bash
git add frontend/ .gitignore
git commit -m "feat: scaffold frontend (Vite + Vue 3 + TypeScript + Tailwind)"
```

---

### Task 3: Embed the frontend into the Rust binary

**Files:**
- Modify: `Cargo.toml` (add `rust-embed`)
- Create: `src/static_files.rs`
- Modify: `src/lib.rs` (register the module)
- Modify: `src/api/mod.rs` (add the fallback route)
- Test: `tests/api_test.rs` (new tests)

**Interfaces:**
- Consumes: `frontend/dist/` (Task 2's build output — must exist before
  `cargo build` runs; see Global Constraints).
- Produces: any HTTP request that doesn't match `/rest/*`, `/webhook/*`,
  or `/health` serves the embedded SPA (a real embedded asset by path, or
  `index.html` as the SPA-routing fallback).

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, add to `[dependencies]`:

```toml
rust-embed = { version = "8", features = ["mime-guess"] }
```

- [ ] **Step 2: The embed + serving logic, with tests**

Create `src/static_files.rs`:

```rust
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "frontend/dist/"]
struct FrontendAssets;

/// Axum fallback handler: serves the embedded Vue SPA. A real embedded
/// asset (e.g. `/assets/index-abc123.js`) is served by exact path with its
/// correct MIME type. Anything else — a client-side route like
/// `/workflows/<uuid>`, or a genuinely missing asset — falls back to
/// `index.html` so Vue Router's history-mode routing can take over. This
/// only ever runs for requests that didn't match any `/rest/*`,
/// `/webhook/*`, or `/health` route, since Axum only calls a router's
/// fallback after every other route fails to match.
pub async fn serve_frontend(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    serve_embedded(if path.is_empty() { "index.html" } else { path })
}

fn serve_embedded(path: &str) -> Response {
    match FrontendAssets::get(path) {
        Some(file) => ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string())], file.data).into_response(),
        None => match FrontendAssets::get("index.html") {
            Some(file) => ([(header::CONTENT_TYPE, file.metadata.mimetype().to_string())], file.data).into_response(),
            None => (StatusCode::NOT_FOUND, "frontend not built").into_response(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_html_is_embedded_and_is_real_html() {
        // If this fails, frontend/dist/ either doesn't exist or wasn't built
        // (Task 2's `npm run build` must run before this crate compiles) --
        // not a bug in this module's own logic.
        let file = FrontendAssets::get("index.html").expect("frontend/dist/index.html must exist (run `npm run build` in frontend/ first)");
        let body = String::from_utf8_lossy(&file.data);
        assert!(body.contains("<div id=\"app\">"));
    }

    #[test]
    fn unknown_path_falls_back_to_index_html() {
        let response = serve_embedded("some/client/route/that/is/not/a/real/file");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

- [ ] **Step 3: Register the module**

In `src/lib.rs`, add (alphabetically):

```rust
pub mod static_files;
```

- [ ] **Step 4: Wire the fallback route**

In `src/api/mod.rs`, add `.fallback(crate::static_files::serve_frontend)`
to the router chain, placed before `.with_state(state)` (order among the
other `.route(...)` calls doesn't matter — Axum only consults the
fallback after every explicit route fails to match):

```rust
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/rest/auth/register", post(auth::register))
        // ... (all existing routes, unchanged) ...
        .route("/health", get(|| async { "ok" }))
        .fallback(crate::static_files::serve_frontend)
        .with_state(state)
}
```

- [ ] **Step 5: End-to-end tests proving route priority**

Add to `tests/api_test.rs`:

```rust
#[tokio::test]
async fn root_path_serves_the_embedded_frontend() {
    let app = test_app().await;
    let response = app
        .oneshot(Request::builder().method("GET").uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}

#[tokio::test]
async fn a_client_side_route_falls_back_to_the_frontend_not_a_404() {
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/workflows/00000000-0000-0000-0000-000000000000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}

#[tokio::test]
async fn rest_routes_still_take_priority_over_the_frontend_fallback() {
    let app = test_app().await;
    // An unauthenticated request to a real, known API route must get that
    // route's own real behavior (here: 400/422 for a malformed body, from
    // axum's own JSON extractor), never the SPA's index.html -- proving
    // the fallback genuinely only catches unmatched paths.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/auth/login")
                .header("content-type", "application/json")
                .body(Body::from("not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(!String::from_utf8_lossy(&bytes).contains("<div id=\"app\">"));
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test`
Expected: all tests pass, including the 5 new ones (2 in `static_files.rs`,
3 in `tests/api_test.rs`). If `cargo build`/`cargo test` fails because
`frontend/dist/` doesn't exist, run `npm install && npm run build` inside
`frontend/` first (Task 2's own verification step should already have
produced it in this same worktree, but confirm before debugging further).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/static_files.rs src/lib.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: embed and serve the frontend SPA from the Rust binary"
```

---

### Task 4: App shell — types, API client, auth store, router, login/register

**Files:**
- Create: `frontend/src/types/domain.ts`
- Create: `frontend/src/api/client.ts`
- Create: `frontend/src/stores/auth.ts`
- Create: `frontend/src/stores/auth.spec.ts`
- Create: `frontend/src/router/index.ts`
- Create: `frontend/src/views/LoginView.vue`
- Create: `frontend/src/views/RegisterView.vue`
- Modify: `frontend/src/main.ts` (install Pinia + router)
- Modify: `frontend/src/App.vue` (render `<router-view>`)

**Interfaces:**
- Produces: `Workflow`, `NodeInstance`, `Connection`, `Item`, `Execution`,
  `CredentialSummary` TypeScript types (mirroring the Rust domain types in
  `src/domain.rs` exactly); an `api` object (`api.get/post/put/patch/delete`)
  handling the auth header and 401 redirect; `useAuthStore()` (Pinia) with
  `isAuthenticated`, `login()`, `register()`, `logout()`; `/login` and
  `/register` routes; a `requiresAuth` route-meta guard.
- Consumes: the backend endpoints `POST /rest/auth/login`,
  `POST /rest/auth/register` (both already exist, return `{"token": "..."}"`).

- [ ] **Step 1: Domain types**

Create `frontend/src/types/domain.ts`:

```typescript
export interface NodeInstance {
  id: string
  node_type: string
  position: [number, number]
  parameters: Record<string, unknown>
  disabled: boolean
}

export interface Connection {
  from_node: string
  from_output: number
  to_node: string
  to_input: number
}

export interface Workflow {
  id: string
  name: string
  active: boolean
  nodes: NodeInstance[]
  connections: Connection[]
  created_at: string
  updated_at: string
}

export interface Item {
  json: unknown
  binary: unknown
}

export type ExecutionStatus = 'Running' | 'Success' | 'Error'
export type ExecutionMode = 'Manual' | 'Webhook' | 'Schedule' | 'Telegram'

export interface Execution {
  id: string
  workflow_id: string
  status: ExecutionStatus
  mode: ExecutionMode
  node_outputs: Record<string, Item[]>
  started_at: string
  finished_at: string | null
}

export interface CredentialSummary {
  id: string
  name: string
  credential_type: string
  owner_id: string
  created_at: string
  updated_at: string
}
```

- [ ] **Step 2: API client**

Create `frontend/src/api/client.ts`:

```typescript
const TOKEN_KEY = 'r8r_token'

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY)
}

export function setToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token)
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY)
}

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message)
    this.name = 'ApiError'
  }
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const token = getToken()
  const headers: Record<string, string> = {
    'content-type': 'application/json',
    ...(options.headers as Record<string, string> | undefined),
  }
  if (token) {
    headers['authorization'] = `Bearer ${token}`
  }

  const response = await fetch(path, { ...options, headers })

  if (response.status === 401) {
    clearToken()
    window.location.href = '/login'
    throw new ApiError(401, 'unauthorized')
  }

  if (!response.ok) {
    const text = await response.text().catch(() => '')
    throw new ApiError(response.status, text || `request failed with status ${response.status}`)
  }

  if (response.status === 204) {
    return undefined as T
  }

  return (await response.json()) as T
}

export const api = {
  get: <T>(path: string) => request<T>(path),
  post: <T>(path: string, body?: unknown) =>
    request<T>(path, { method: 'POST', body: body !== undefined ? JSON.stringify(body) : undefined }),
  put: <T>(path: string, body: unknown) => request<T>(path, { method: 'PUT', body: JSON.stringify(body) }),
  patch: <T>(path: string, body: unknown) => request<T>(path, { method: 'PATCH', body: JSON.stringify(body) }),
  delete: <T>(path: string) => request<T>(path, { method: 'DELETE' }),
}
```

- [ ] **Step 3: Auth store, with tests**

Create `frontend/src/stores/auth.ts`:

```typescript
import { defineStore } from 'pinia'
import { api, getToken, setToken, clearToken } from '../api/client'

export const useAuthStore = defineStore('auth', {
  state: () => ({
    token: getToken() as string | null,
  }),
  getters: {
    isAuthenticated: (state) => state.token !== null,
  },
  actions: {
    async login(email: string, password: string) {
      const response = await api.post<{ token: string }>('/rest/auth/login', { email, password })
      this.token = response.token
      setToken(response.token)
    },
    async register(email: string, password: string) {
      const response = await api.post<{ token: string }>('/rest/auth/register', { email, password })
      this.token = response.token
      setToken(response.token)
    },
    logout() {
      this.token = null
      clearToken()
    },
  },
})
```

Create `frontend/src/stores/auth.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useAuthStore } from './auth'

describe('auth store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('starts unauthenticated when no token is stored', () => {
    const store = useAuthStore()
    expect(store.isAuthenticated).toBe(false)
  })

  it('login stores the token and marks authenticated', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ({ token: 'abc123' }),
      }),
    )
    const store = useAuthStore()
    await store.login('a@b.com', 'password')
    expect(store.isAuthenticated).toBe(true)
    expect(localStorage.getItem('r8r_token')).toBe('abc123')
    vi.unstubAllGlobals()
  })

  it('logout clears the token', () => {
    localStorage.setItem('r8r_token', 'abc123')
    const store = useAuthStore()
    store.logout()
    expect(store.isAuthenticated).toBe(false)
    expect(localStorage.getItem('r8r_token')).toBeNull()
  })
})
```

- [ ] **Step 4: Router**

Create `frontend/src/router/index.ts`:

```typescript
import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '../stores/auth'

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: '/login', name: 'login', component: () => import('../views/LoginView.vue') },
    { path: '/register', name: 'register', component: () => import('../views/RegisterView.vue') },
    {
      path: '/workflows',
      name: 'workflows',
      component: () => import('../views/WorkflowListView.vue'),
      meta: { requiresAuth: true },
    },
    {
      path: '/workflows/:id',
      name: 'workflow-editor',
      component: () => import('../views/WorkflowEditorView.vue'),
      meta: { requiresAuth: true },
    },
    { path: '/', redirect: '/workflows' },
  ],
})

router.beforeEach((to) => {
  const auth = useAuthStore()
  if (to.meta.requiresAuth && !auth.isAuthenticated) {
    return { name: 'login' }
  }
  return true
})

export default router
```

Note: `WorkflowListView.vue`/`WorkflowEditorView.vue` don't exist until
Tasks 5-6 — this is fine, Vite/`vue-tsc` resolve dynamic `import()`
lazily and won't error at this task's `npm run build` as long as no
other file imports them eagerly. If `vue-tsc --noEmit` complains about
these two specific missing files (and only these two), that's expected
at this point in the plan; if it complains about anything else, that's
a real problem to fix.

- [ ] **Step 5: Login and register views**

Create `frontend/src/views/LoginView.vue`:

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { ApiError } from '../api/client'

const email = ref('')
const password = ref('')
const error = ref('')
const submitting = ref(false)

const auth = useAuthStore()
const router = useRouter()

async function onSubmit() {
  error.value = ''
  submitting.value = true
  try {
    await auth.login(email.value, password.value)
    router.push({ name: 'workflows' })
  } catch (e) {
    error.value = e instanceof ApiError ? 'Invalid email or password.' : 'Something went wrong. Try again.'
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <main class="min-h-screen bg-gray-50 flex items-center justify-center">
    <form class="bg-white shadow rounded p-8 w-80 space-y-4" @submit.prevent="onSubmit">
      <h1 class="text-xl font-semibold text-gray-800">Log in to r8r</h1>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="email">Email</label>
        <input id="email" v-model="email" type="email" required class="w-full border rounded px-3 py-2" />
      </div>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="password">Password</label>
        <input id="password" v-model="password" type="password" required class="w-full border rounded px-3 py-2" />
      </div>
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <button type="submit" :disabled="submitting" class="w-full bg-blue-600 text-white rounded py-2 disabled:opacity-50">
        {{ submitting ? 'Logging in…' : 'Log in' }}
      </button>
      <router-link to="/register" class="block text-sm text-blue-600 text-center">Need an account? Register</router-link>
    </form>
  </main>
</template>
```

Create `frontend/src/views/RegisterView.vue` (same shape, calling
`auth.register` instead):

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '../stores/auth'
import { ApiError } from '../api/client'

const email = ref('')
const password = ref('')
const error = ref('')
const submitting = ref(false)

const auth = useAuthStore()
const router = useRouter()

async function onSubmit() {
  error.value = ''
  submitting.value = true
  try {
    await auth.register(email.value, password.value)
    router.push({ name: 'workflows' })
  } catch (e) {
    error.value = e instanceof ApiError && e.status === 409 ? 'That email is already registered.' : 'Something went wrong. Try again.'
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <main class="min-h-screen bg-gray-50 flex items-center justify-center">
    <form class="bg-white shadow rounded p-8 w-80 space-y-4" @submit.prevent="onSubmit">
      <h1 class="text-xl font-semibold text-gray-800">Create an r8r account</h1>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="email">Email</label>
        <input id="email" v-model="email" type="email" required class="w-full border rounded px-3 py-2" />
      </div>
      <div>
        <label class="block text-sm text-gray-600 mb-1" for="password">Password</label>
        <input id="password" v-model="password" type="password" required class="w-full border rounded px-3 py-2" />
      </div>
      <p v-if="error" class="text-sm text-red-600">{{ error }}</p>
      <button type="submit" :disabled="submitting" class="w-full bg-blue-600 text-white rounded py-2 disabled:opacity-50">
        {{ submitting ? 'Creating account…' : 'Register' }}
      </button>
      <router-link to="/login" class="block text-sm text-blue-600 text-center">Already have an account? Log in</router-link>
    </form>
  </main>
</template>
```

- [ ] **Step 6: Wire Pinia + router into the app**

Replace `frontend/src/main.ts`:

```typescript
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './style.css'

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.mount('#app')
```

Replace `frontend/src/App.vue`:

```vue
<script setup lang="ts"></script>

<template>
  <router-view />
</template>
```

(The "r8r" placeholder heading from Task 2 is gone now that real routed
pages exist — Task 2's `App.spec.ts` test will need updating in this
task since it asserted on that placeholder text; see Step 7.)

- [ ] **Step 7: Update the now-stale smoke test**

`frontend/src/App.spec.ts` (from Task 2) asserts `wrapper.text()` contains
`"r8r"`, which no longer renders directly from `App.vue` now that it's
just a `<router-view>`. Replace its content with a test that mounts
`App.vue` with a real router/pinia instance and confirms it renders
*something* routed (e.g. redirects `/` to `/workflows`, which — since
there's no token — the `beforeEach` guard further redirects to `/login`,
so the login form's heading becomes the actual observable behavior to
assert on):

```typescript
import { describe, it, expect, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { createRouter, createWebHistory } from 'vue-router'
import App from './App.vue'
import LoginView from './views/LoginView.vue'
import WorkflowListView from './views/WorkflowListView.vue'

describe('App', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    localStorage.clear()
  })

  it('redirects an unauthenticated visitor to the login page', async () => {
    const router = createRouter({
      history: createWebHistory(),
      routes: [
        { path: '/login', name: 'login', component: LoginView },
        { path: '/workflows', name: 'workflows', component: WorkflowListView, meta: { requiresAuth: true } },
        { path: '/', redirect: '/workflows' },
      ],
    })
    router.beforeEach((to) => {
      if (to.meta.requiresAuth) return { name: 'login' }
      return true
    })
    router.push('/')
    await router.isReady()

    const wrapper = mount(App, { global: { plugins: [router] } })
    expect(wrapper.text()).toContain('Log in to r8r')
  })
})
```

This test builds its own minimal router (rather than importing the real
`WorkflowListView.vue`, which doesn't exist until Task 5) to stay
self-contained to what exists at this point in the plan — replace the
inline `WorkflowListView` stand-in with the real import once Task 5 lands,
if you're executing tasks in order within the same session (not required
by this task, just worth noting for whoever next touches this file).

- [ ] **Step 8: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass; build succeeds (dynamic imports of not-yet-created
`WorkflowListView.vue`/`WorkflowEditorView.vue` in the router are fine per
Step 4's note — `vue-tsc` type-checks the router file itself, and Vite's
build only needs those imports to *resolve* at runtime, not exist at type-
check time for an unused dynamic branch... if `vue-tsc --noEmit` DOES hard-
error on the missing files (rather than just being unable to fully type
the lazy-loaded component), create trivial one-line placeholder files for
`WorkflowListView.vue` and `WorkflowEditorView.vue` (a single `<template><div>
placeholder</div></template>`) so this task's build succeeds standalone —
Tasks 5 and 6 will then overwrite them with real content. Note in your
report which path you had to take.

- [ ] **Step 9: Commit**

```bash
git add frontend/src/types frontend/src/api frontend/src/stores frontend/src/router frontend/src/views/LoginView.vue frontend/src/views/RegisterView.vue frontend/src/main.ts frontend/src/App.vue frontend/src/App.spec.ts
git commit -m "feat: add app shell (auth, routing, API client, domain types)"
```

---

### Task 5: Workflow list

**Files:**
- Create: `frontend/src/stores/workflows.ts`
- Create: `frontend/src/stores/workflows.spec.ts`
- Create (or overwrite the Task 4 placeholder for): `frontend/src/views/WorkflowListView.vue`

**Interfaces:**
- Consumes: `api` (Task 4), `Workflow` type (Task 4), the existing
  `GET/POST /rest/workflows`, `DELETE /rest/workflows/:id` (Task 1),
  `PATCH /rest/workflows/:id/active` (already existed).
- Produces: `useWorkflowsStore()` with `workflows`, `loading`,
  `fetchAll()`, `create(name)`, `remove(id)`, `setActive(id, active)`.

- [ ] **Step 1: Workflows store, with tests**

Create `frontend/src/stores/workflows.ts`:

```typescript
import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { Workflow } from '../types/domain'

export const useWorkflowsStore = defineStore('workflows', {
  state: () => ({
    workflows: [] as Workflow[],
    loading: false,
  }),
  actions: {
    async fetchAll() {
      this.loading = true
      try {
        this.workflows = await api.get<Workflow[]>('/rest/workflows')
      } finally {
        this.loading = false
      }
    },
    async create(name: string): Promise<Workflow> {
      const workflow = await api.post<Workflow>('/rest/workflows', { name, nodes: [], connections: [] })
      this.workflows.push(workflow)
      return workflow
    },
    async remove(id: string) {
      await api.delete(`/rest/workflows/${id}`)
      this.workflows = this.workflows.filter((w) => w.id !== id)
    },
    async setActive(id: string, active: boolean) {
      const updated = await api.patch<Workflow>(`/rest/workflows/${id}/active`, { active })
      const idx = this.workflows.findIndex((w) => w.id === id)
      if (idx !== -1) this.workflows[idx] = updated
    },
  },
})
```

Create `frontend/src/stores/workflows.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useWorkflowsStore } from './workflows'

function mockFetchOnce(body: unknown, status = 200) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: status < 400,
      status,
      json: async () => body,
    }),
  )
}

describe('workflows store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('fetchAll populates workflows from the API', async () => {
    mockFetchOnce([{ id: '1', name: 'a' }])
    const store = useWorkflowsStore()
    await store.fetchAll()
    expect(store.workflows).toHaveLength(1)
    expect(store.loading).toBe(false)
    vi.unstubAllGlobals()
  })

  it('create appends the new workflow', async () => {
    mockFetchOnce({ id: '2', name: 'new one' })
    const store = useWorkflowsStore()
    const created = await store.create('new one')
    expect(created.id).toBe('2')
    expect(store.workflows).toContainEqual(created)
    vi.unstubAllGlobals()
  })

  it('remove drops the workflow from local state', async () => {
    const store = useWorkflowsStore()
    store.workflows = [{ id: '1', name: 'a' } as never]
    mockFetchOnce(undefined, 204)
    await store.remove('1')
    expect(store.workflows).toHaveLength(0)
    vi.unstubAllGlobals()
  })
})
```

- [ ] **Step 2: Workflow list view**

Create (overwriting Task 4's placeholder, if one exists)
`frontend/src/views/WorkflowListView.vue`:

```vue
<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useWorkflowsStore } from '../stores/workflows'
import { useAuthStore } from '../stores/auth'

const store = useWorkflowsStore()
const auth = useAuthStore()
const router = useRouter()
const newName = ref('')

onMounted(() => store.fetchAll())

async function createWorkflow() {
  if (!newName.value.trim()) return
  const workflow = await store.create(newName.value.trim())
  newName.value = ''
  router.push({ name: 'workflow-editor', params: { id: workflow.id } })
}

function logout() {
  auth.logout()
  router.push({ name: 'login' })
}
</script>

<template>
  <main class="min-h-screen bg-gray-50">
    <header class="bg-white border-b px-6 py-4 flex justify-between items-center">
      <h1 class="text-xl font-semibold text-gray-800">Workflows</h1>
      <button class="text-sm text-gray-500" @click="logout">Log out</button>
    </header>
    <div class="p-6 max-w-3xl mx-auto space-y-4">
      <form class="flex gap-2" @submit.prevent="createWorkflow">
        <input v-model="newName" placeholder="New workflow name" class="flex-1 border rounded px-3 py-2" />
        <button type="submit" class="bg-blue-600 text-white rounded px-4 py-2">+ New workflow</button>
      </form>
      <ul class="divide-y bg-white rounded shadow">
        <li v-for="wf in store.workflows" :key="wf.id" class="flex justify-between items-center px-4 py-3">
          <router-link :to="{ name: 'workflow-editor', params: { id: wf.id } }" class="text-blue-600">{{ wf.name }}</router-link>
          <div class="flex items-center gap-3">
            <label class="text-sm flex items-center gap-1">
              <input
                type="checkbox"
                :checked="wf.active"
                @change="store.setActive(wf.id, ($event.target as HTMLInputElement).checked)"
              />
              Active
            </label>
            <button class="text-sm text-red-600" @click="store.remove(wf.id)">Delete</button>
          </div>
        </li>
      </ul>
      <p v-if="!store.loading && store.workflows.length === 0" class="text-gray-400 text-center py-8">No workflows yet.</p>
    </div>
  </main>
</template>
```

- [ ] **Step 3: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass, build succeeds.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/stores/workflows.ts frontend/src/stores/workflows.spec.ts frontend/src/views/WorkflowListView.vue
git commit -m "feat: add workflow list view (create, delete, toggle active)"
```

---

### Task 6: Canvas editor — render and manipulate the workflow graph

**Files:**
- Create: `frontend/src/components/WorkflowCanvas.vue`
- Create (or overwrite the Task 4 placeholder for): `frontend/src/views/WorkflowEditorView.vue`

**Interfaces:**
- Consumes: `NodeInstance`/`Connection` types (Task 4), `api.get` (Task 4).
- Produces: `WorkflowEditorView` at `/workflows/:id`, fetching the real
  workflow and rendering it on a Vue Flow canvas; local (in-memory, not
  yet persisted — that's Task 10) node position updates and new
  connections; a tracked `selectedNodeId` that Task 8's config panel will
  consume.

- [ ] **Step 1: Install Vue Flow's CSS import path check**

`@vue-flow/core` was already added to `frontend/package.json` in Task 2
and installed then. Confirm `node_modules/@vue-flow/core` exists (it
should, from Task 2's `npm install`); if this task's own `npm install`
run adds anything new, note it in your report.

- [ ] **Step 2: The canvas component**

Create `frontend/src/components/WorkflowCanvas.vue`:

```vue
<script setup lang="ts">
import { computed } from 'vue'
import { VueFlow, useVueFlow, type Node as FlowNode, type Edge as FlowEdge } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import type { NodeInstance, Connection } from '../types/domain'

const props = defineProps<{
  nodes: NodeInstance[]
  connections: Connection[]
}>()

const emit = defineEmits<{
  'node-select': [nodeId: string]
  'node-move': [nodeId: string, position: [number, number]]
  connect: [connection: Connection]
}>()

const { onConnect, onNodeDragStop, onNodeClick } = useVueFlow()

const flowNodes = computed<FlowNode[]>(() =>
  props.nodes.map((n) => ({
    id: n.id,
    position: { x: n.position[0], y: n.position[1] },
    label: `${n.id}\n${n.node_type}`,
    data: { nodeType: n.node_type, disabled: n.disabled },
  })),
)

const flowEdges = computed<FlowEdge[]>(() =>
  props.connections.map((c) => ({
    id: `${c.from_node}:${c.from_output}->${c.to_node}:${c.to_input}`,
    source: c.from_node,
    target: c.to_node,
    sourceHandle: String(c.from_output),
    targetHandle: String(c.to_input),
  })),
)

onNodeClick((event) => {
  emit('node-select', event.node.id)
})

onNodeDragStop((event) => {
  emit('node-move', event.node.id, [event.node.position.x, event.node.position.y])
})

onConnect((connection) => {
  emit('connect', {
    from_node: connection.source,
    from_output: Number(connection.sourceHandle ?? 0),
    to_node: connection.target,
    to_input: Number(connection.targetHandle ?? 0),
  })
})
</script>

<template>
  <div class="w-full h-full">
    <VueFlow :nodes="flowNodes" :edges="flowEdges" fit-view-on-init>
      <template #node-default="{ data, label }">
        <div class="px-3 py-2 rounded border bg-white shadow text-xs whitespace-pre-line" :class="{ 'opacity-50': data.disabled }">
          {{ label }}
        </div>
      </template>
    </VueFlow>
  </div>
</template>
```

Per this plan's Global Constraints: if any prop/event/type name above
(`onNodeClick`'s event shape, `onNodeDragStop`'s event shape,
`onConnect`'s `Connection` type's field names, the `#node-default` slot's
exact scoped-slot props) doesn't match what the actually-installed
`@vue-flow/core` version exposes, check its real TypeScript types
(`node_modules/@vue-flow/core/dist/types/*.d.ts` or its own docs) and
adjust — preserving the intent (a node click emits its id; a node drag-
stop emits its new x/y; connecting two handles emits a `Connection`
shaped like this codebase's own `Connection` type). Note any such
adjustment in your report.

- [ ] **Step 3: The editor view**

Create (overwriting Task 4's placeholder, if one exists)
`frontend/src/views/WorkflowEditorView.vue`:

```vue
<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { api } from '../api/client'
import type { Workflow, Connection } from '../types/domain'
import WorkflowCanvas from '../components/WorkflowCanvas.vue'

const route = useRoute()
const workflowId = route.params.id as string

const workflow = ref<Workflow | null>(null)
const selectedNodeId = ref<string | null>(null)

onMounted(async () => {
  workflow.value = await api.get<Workflow>(`/rest/workflows/${workflowId}`)
})

function onNodeSelect(nodeId: string) {
  selectedNodeId.value = nodeId
}

function onNodeMove(nodeId: string, position: [number, number]) {
  if (!workflow.value) return
  const node = workflow.value.nodes.find((n) => n.id === nodeId)
  if (node) node.position = position
}

function onConnect(connection: Connection) {
  if (!workflow.value) return
  workflow.value.connections.push(connection)
}
</script>

<template>
  <div class="h-screen flex flex-col">
    <header class="bg-white border-b px-6 py-3 flex items-center gap-4">
      <router-link to="/workflows" class="text-sm text-gray-500">&larr; Workflows</router-link>
      <input
        v-if="workflow"
        v-model="workflow.name"
        class="text-lg font-medium border-none focus:outline-none focus:ring-1 focus:ring-blue-300 rounded px-1"
      />
    </header>
    <div class="flex-1 relative">
      <WorkflowCanvas
        v-if="workflow"
        :nodes="workflow.nodes"
        :connections="workflow.connections"
        @node-select="onNodeSelect"
        @node-move="onNodeMove"
        @connect="onConnect"
      />
    </div>
  </div>
</template>
```

- [ ] **Step 4: A component test for `WorkflowCanvas`**

Create `frontend/src/components/WorkflowCanvas.spec.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import WorkflowCanvas from './WorkflowCanvas.vue'
import type { NodeInstance } from '../types/domain'

describe('WorkflowCanvas', () => {
  it('renders one canvas node per workflow node', () => {
    const nodes: NodeInstance[] = [
      { id: 'a', node_type: 'core.manualTrigger', position: [0, 0], parameters: {}, disabled: false },
      { id: 'b', node_type: 'core.set', position: [200, 0], parameters: {}, disabled: false },
    ]
    const wrapper = mount(WorkflowCanvas, { props: { nodes, connections: [] } })
    expect(wrapper.text()).toContain('core.manualTrigger')
    expect(wrapper.text()).toContain('core.set')
  })
})
```

- [ ] **Step 5: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass, build succeeds. If Vue Flow's real API required
adjustments per Step 2's note, `WorkflowCanvas.spec.ts` should still pass
once those adjustments are made — if it doesn't render the expected
labels, that's a genuine signal the adjustment didn't fully preserve
intent, not something to work around by weakening the test's assertion.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/components/WorkflowCanvas.vue frontend/src/components/WorkflowCanvas.spec.ts frontend/src/views/WorkflowEditorView.vue
git commit -m "feat: add canvas editor (render/drag/connect workflow nodes)"
```

---

### Task 7: Add-node menu

**Files:**
- Create: `frontend/src/stores/nodeTypes.ts`
- Create: `frontend/src/components/AddNodeMenu.vue`
- Create: `frontend/src/components/AddNodeMenu.spec.ts`
- Modify: `frontend/src/views/WorkflowEditorView.vue`

**Interfaces:**
- Consumes: `GET /rest/node-types` (Task 1), `api.get` (Task 4).
- Produces: `useNodeTypesStore()` (`types`, `fetchAll()`), an
  `AddNodeMenu` component emitting `add: [nodeType: string]`, wired into
  the editor toolbar so choosing a type inserts a new `NodeInstance` into
  `workflow.value.nodes`.

- [ ] **Step 1: Node-types store**

Create `frontend/src/stores/nodeTypes.ts`:

```typescript
import { defineStore } from 'pinia'
import { api } from '../api/client'

export const useNodeTypesStore = defineStore('nodeTypes', {
  state: () => ({
    types: [] as string[],
    loaded: false,
  }),
  actions: {
    async fetchAll() {
      if (this.loaded) return
      this.types = await api.get<string[]>('/rest/node-types')
      this.loaded = true
    },
  },
})
```

- [ ] **Step 2: The menu component**

Create `frontend/src/components/AddNodeMenu.vue`:

```vue
<script setup lang="ts">
import { computed, ref } from 'vue'
import { useNodeTypesStore } from '../stores/nodeTypes'

const emit = defineEmits<{ add: [nodeType: string] }>()
const store = useNodeTypesStore()
const open = ref(false)
const search = ref('')

store.fetchAll()

const filtered = computed(() => store.types.filter((t) => t.toLowerCase().includes(search.value.toLowerCase())))

function choose(type: string) {
  emit('add', type)
  open.value = false
  search.value = ''
}
</script>

<template>
  <div class="relative">
    <button class="bg-blue-600 text-white rounded px-3 py-1.5 text-sm" @click="open = !open">+ Add node</button>
    <div v-if="open" class="absolute z-10 mt-1 w-64 bg-white border rounded shadow">
      <input v-model="search" autofocus placeholder="Search node types…" class="w-full border-b px-3 py-2 text-sm" />
      <ul class="max-h-64 overflow-auto">
        <li v-for="t in filtered" :key="t" class="px-3 py-2 text-sm hover:bg-gray-50 cursor-pointer" @click="choose(t)">
          {{ t }}
        </li>
        <li v-if="filtered.length === 0" class="px-3 py-2 text-sm text-gray-400">No matches</li>
      </ul>
    </div>
  </div>
</template>
```

Create `frontend/src/components/AddNodeMenu.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import AddNodeMenu from './AddNodeMenu.vue'

describe('AddNodeMenu', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: async () => ['core.set', 'core.httpRequest', 'telegram.sendMessage'],
      }),
    )
  })

  it('emits add with the chosen type when clicked', async () => {
    const wrapper = mount(AddNodeMenu)
    await wrapper.find('button').trigger('click')
    await new Promise((r) => setTimeout(r, 0)) // let fetchAll's promise resolve
    const items = wrapper.findAll('li')
    const httpItem = items.find((li) => li.text() === 'core.httpRequest')
    expect(httpItem).toBeTruthy()
    await httpItem!.trigger('click')
    expect(wrapper.emitted('add')).toEqual([['core.httpRequest']])
    vi.unstubAllGlobals()
  })
})
```

- [ ] **Step 3: Wire into the editor**

In `frontend/src/views/WorkflowEditorView.vue`, add the import and an
`onAddNode` handler, and place `<AddNodeMenu>` in the toolbar:

```typescript
import AddNodeMenu from '../components/AddNodeMenu.vue'

function onAddNode(nodeType: string) {
  if (!workflow.value) return
  const count = workflow.value.nodes.length
  workflow.value.nodes.push({
    id: crypto.randomUUID(),
    node_type: nodeType,
    position: [100 + count * 40, 100 + count * 40],
    parameters: {},
    disabled: false,
  })
}
```

```vue
<header class="bg-white border-b px-6 py-3 flex items-center gap-4">
  <router-link to="/workflows" class="text-sm text-gray-500">&larr; Workflows</router-link>
  <input
    v-if="workflow"
    v-model="workflow.name"
    class="text-lg font-medium border-none focus:outline-none focus:ring-1 focus:ring-blue-300 rounded px-1"
  />
  <div class="flex-1"></div>
  <AddNodeMenu @add="onAddNode" />
</header>
```

- [ ] **Step 4: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass, build succeeds.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/stores/nodeTypes.ts frontend/src/components/AddNodeMenu.vue frontend/src/components/AddNodeMenu.spec.ts frontend/src/views/WorkflowEditorView.vue
git commit -m "feat: add node-type picker (add nodes to the canvas)"
```

---

### Task 8: Node configuration panel

**Files:**
- Create: `frontend/src/components/NodeConfigPanel.vue`
- Create: `frontend/src/components/NodeConfigPanel.spec.ts`
- Modify: `frontend/src/views/WorkflowEditorView.vue`

**Interfaces:**
- Consumes: `NodeInstance` type (Task 4), `selectedNodeId` (Task 6).
- Produces: a slide-in panel showing the selected node's `id`/`node_type`,
  a `disabled` toggle, and a JSON-validated `parameters` textarea; emits
  `update: [node: NodeInstance]` and `close: []`, wired into the editor so
  double-clicking a node opens it and Apply writes changes back into
  `workflow.value.nodes`.

- [ ] **Step 1: The panel component**

Create `frontend/src/components/NodeConfigPanel.vue`:

```vue
<script setup lang="ts">
import { ref, watch } from 'vue'
import type { NodeInstance } from '../types/domain'

const props = defineProps<{ node: NodeInstance | null }>()
const emit = defineEmits<{ update: [node: NodeInstance]; close: [] }>()

const paramsText = ref('')
const error = ref('')
const disabled = ref(false)

watch(
  () => props.node,
  (node) => {
    if (node) {
      paramsText.value = JSON.stringify(node.parameters, null, 2)
      disabled.value = node.disabled
      error.value = ''
    }
  },
  { immediate: true },
)

function apply() {
  if (!props.node) return
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(paramsText.value)
  } catch {
    error.value = 'Parameters must be valid JSON.'
    return
  }
  emit('update', { ...props.node, parameters: parsed, disabled: disabled.value })
  error.value = ''
}
</script>

<template>
  <aside v-if="node" class="absolute top-0 right-0 bottom-0 w-96 bg-white border-l shadow-lg flex flex-col">
    <header class="px-4 py-3 border-b flex justify-between items-center">
      <div>
        <div class="text-xs text-gray-400">{{ node.node_type }}</div>
        <div class="font-medium">{{ node.id }}</div>
      </div>
      <button class="text-gray-400" @click="emit('close')">&times;</button>
    </header>
    <div class="p-4 flex-1 overflow-auto space-y-3">
      <label class="flex items-center gap-2 text-sm">
        <input v-model="disabled" type="checkbox" />
        Disabled
      </label>
      <div>
        <label class="block text-sm text-gray-600 mb-1">Parameters (JSON)</label>
        <textarea v-model="paramsText" rows="14" class="w-full border rounded px-2 py-1.5 font-mono text-xs"></textarea>
        <p v-if="error" class="text-sm text-red-600 mt-1">{{ error }}</p>
      </div>
    </div>
    <footer class="px-4 py-3 border-t">
      <button class="w-full bg-blue-600 text-white rounded py-2 text-sm" @click="apply">Apply</button>
    </footer>
  </aside>
</template>
```

Create `frontend/src/components/NodeConfigPanel.spec.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import NodeConfigPanel from './NodeConfigPanel.vue'
import type { NodeInstance } from '../types/domain'

const node: NodeInstance = {
  id: 'n1',
  node_type: 'core.set',
  position: [0, 0],
  parameters: { foo: 'bar' },
  disabled: false,
}

describe('NodeConfigPanel', () => {
  it('emits update with the parsed parameters on Apply', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{"foo":"baz"}')
    await wrapper.find('button').trigger('click')
    const events = wrapper.emitted('update')
    expect(events).toBeTruthy()
    expect((events![0][0] as NodeInstance).parameters).toEqual({ foo: 'baz' })
  })

  it('shows an error and does not emit update on invalid JSON', async () => {
    const wrapper = mount(NodeConfigPanel, { props: { node } })
    const textarea = wrapper.find('textarea')
    await textarea.setValue('{not valid json')
    await wrapper.find('button').trigger('click')
    expect(wrapper.text()).toContain('must be valid JSON')
    expect(wrapper.emitted('update')).toBeFalsy()
  })
})
```

- [ ] **Step 2: Wire into the editor**

In `frontend/src/views/WorkflowEditorView.vue`, add:

```typescript
import { computed } from 'vue'
import NodeConfigPanel from '../components/NodeConfigPanel.vue'
import type { NodeInstance } from '../types/domain'

const selectedNode = computed<NodeInstance | null>(
  () => workflow.value?.nodes.find((n) => n.id === selectedNodeId.value) ?? null,
)

function onNodeUpdate(updated: NodeInstance) {
  if (!workflow.value) return
  const idx = workflow.value.nodes.findIndex((n) => n.id === updated.id)
  if (idx !== -1) workflow.value.nodes[idx] = updated
}
```

Add `<NodeConfigPanel :node="selectedNode" @update="onNodeUpdate"
@close="selectedNodeId = null" />` inside the `<div class="flex-1
relative">` wrapper, after `<WorkflowCanvas>`.

- [ ] **Step 3: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass, build succeeds.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/components/NodeConfigPanel.vue frontend/src/components/NodeConfigPanel.spec.ts frontend/src/views/WorkflowEditorView.vue
git commit -m "feat: add node config panel (parameters JSON editor, disabled toggle)"
```

---

### Task 9: Credential picker

**Files:**
- Create: `frontend/src/stores/credentials.ts`
- Create: `frontend/src/stores/credentials.spec.ts`
- Create: `frontend/src/components/CredentialPicker.vue`
- Modify: `frontend/src/components/NodeConfigPanel.vue`

**Interfaces:**
- Consumes: `GET`/`POST /rest/credentials` (already existed),
  `CredentialSummary` type (Task 4).
- Produces: `useCredentialsStore()` (`credentials`, `fetchAll()`,
  `create(name, credentialType, data)`); a `CredentialPicker` component
  (`v-model` of `string | null`, a credential id) with an inline
  create-new form; integrated into `NodeConfigPanel` as a first-class
  control above the raw JSON textarea, reading/writing
  `parameters.auth.credential_id`.

- [ ] **Step 1: Credentials store, with tests**

Create `frontend/src/stores/credentials.ts`:

```typescript
import { defineStore } from 'pinia'
import { api } from '../api/client'
import type { CredentialSummary } from '../types/domain'

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
      this.credentials.push(summary)
      return summary
    },
  },
})
```

Create `frontend/src/stores/credentials.spec.ts`:

```typescript
import { describe, it, expect, beforeEach, vi } from 'vitest'
import { setActivePinia, createPinia } from 'pinia'
import { useCredentialsStore } from './credentials'

describe('credentials store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('create appends the new credential summary (never the secret data)', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 201,
        json: async () => ({ id: '1', name: 'my-bot', credential_type: 'telegramApi', owner_id: 'u1', created_at: 'x', updated_at: 'x' }),
      }),
    )
    const store = useCredentialsStore()
    const summary = await store.create('my-bot', 'telegramApi', { bot_token: 'secret' })
    expect(summary.id).toBe('1')
    expect('data' in summary).toBe(false)
    expect(store.credentials).toContainEqual(summary)
    vi.unstubAllGlobals()
  })
})
```

- [ ] **Step 2: The picker component**

Create `frontend/src/components/CredentialPicker.vue`:

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { useCredentialsStore } from '../stores/credentials'

const props = defineProps<{ modelValue: string | null }>()
const emit = defineEmits<{ 'update:modelValue': [id: string | null] }>()

const store = useCredentialsStore()
const creating = ref(false)
const newName = ref('')
const newType = ref('')
const newDataText = ref('{}')
const error = ref('')

if (!store.loaded) store.fetchAll()

async function createCredential() {
  error.value = ''
  let data: Record<string, unknown>
  try {
    data = JSON.parse(newDataText.value)
  } catch {
    error.value = 'Data must be valid JSON.'
    return
  }
  const summary = await store.create(newName.value, newType.value, data)
  emit('update:modelValue', summary.id)
  creating.value = false
  newName.value = ''
  newType.value = ''
  newDataText.value = '{}'
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
    <button type="button" class="text-xs text-blue-600" @click="creating = !creating">+ New credential</button>
    <div v-if="creating" class="border rounded p-2 space-y-2 bg-gray-50">
      <input v-model="newName" placeholder="Name" class="w-full border rounded px-2 py-1 text-sm" />
      <input v-model="newType" placeholder="Type (e.g. telegramApi)" class="w-full border rounded px-2 py-1 text-sm" />
      <textarea v-model="newDataText" rows="3" placeholder="{}" class="w-full border rounded px-2 py-1 text-xs font-mono"></textarea>
      <p v-if="error" class="text-xs text-red-600">{{ error }}</p>
      <button type="button" class="text-xs bg-blue-600 text-white rounded px-2 py-1" @click="createCredential">Create</button>
    </div>
  </div>
</template>
```

- [ ] **Step 3: Integrate into `NodeConfigPanel`**

In `frontend/src/components/NodeConfigPanel.vue`, add the import and a
`credentialId` field kept in sync with `parameters.auth.credential_id`:

```typescript
import CredentialPicker from './CredentialPicker.vue'

const credentialId = ref<string | null>(null)
```

In the existing `watch(() => props.node, (node) => { ... })` callback,
add (inside the `if (node)` branch, alongside the existing
`paramsText.value`/`disabled.value` assignments):

```typescript
      const auth = node.parameters?.auth as { credential_id?: string } | undefined
      credentialId.value = auth?.credential_id ?? null
```

In `apply()`, before the `emit('update', ...)` call, add:

```typescript
  if (credentialId.value) {
    parsed.auth = { ...((parsed.auth as object) ?? {}), credential_id: credentialId.value }
  }
```

In the template, add above the `<textarea>`:

```vue
<div>
  <label class="block text-sm text-gray-600 mb-1">Credential (for nodes that need auth)</label>
  <CredentialPicker v-model="credentialId" />
</div>
```

- [ ] **Step 4: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass (including the existing `NodeConfigPanel.spec.ts`
from Task 8, which must still pass unmodified), build succeeds.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/stores/credentials.ts frontend/src/stores/credentials.spec.ts frontend/src/components/CredentialPicker.vue frontend/src/components/NodeConfigPanel.vue
git commit -m "feat: add credential picker, integrated into the node config panel"
```

---

### Task 10: Save, Execute, and results — completing the loop

**Files:**
- Create: `frontend/src/components/ExecutionResultsPanel.vue`
- Create: `frontend/src/components/ExecutionResultsPanel.spec.ts`
- Modify: `frontend/src/views/WorkflowEditorView.vue`

**Interfaces:**
- Consumes: `PUT /rest/workflows/:id` (Task 1), `POST
  /rest/workflows/:id/execute` (already existed), `Execution` type (Task 4).
- Produces: a **Save** button persisting the current in-memory
  `workflow` (name/nodes/connections) and an **Execute** button running it
  and showing per-node results in a bottom panel — completing this plan's
  end-to-end loop: create → add/connect/configure nodes → save → execute
  → see results.

- [ ] **Step 1: The results panel**

Create `frontend/src/components/ExecutionResultsPanel.vue`:

```vue
<script setup lang="ts">
import type { Execution } from '../types/domain'

defineProps<{ execution: Execution | null }>()
defineEmits<{ close: [] }>()
</script>

<template>
  <aside v-if="execution" class="absolute bottom-0 left-0 right-0 h-64 bg-white border-t shadow-lg flex flex-col">
    <header class="px-4 py-2 border-b flex justify-between items-center">
      <span
        class="text-sm font-medium"
        :class="{
          'text-green-600': execution.status === 'Success',
          'text-red-600': execution.status === 'Error',
          'text-gray-500': execution.status === 'Running',
        }"
      >
        {{ execution.status }}
      </span>
      <button class="text-gray-400" @click="$emit('close')">&times;</button>
    </header>
    <div class="p-4 overflow-auto flex-1 space-y-3">
      <div v-for="(items, nodeId) in execution.node_outputs" :key="nodeId">
        <div class="text-xs font-medium text-gray-500 mb-1">{{ nodeId }}</div>
        <pre class="bg-gray-50 rounded p-2 text-xs overflow-auto">{{ JSON.stringify(items, null, 2) }}</pre>
      </div>
      <p v-if="Object.keys(execution.node_outputs).length === 0" class="text-gray-400 text-sm">No node output.</p>
    </div>
  </aside>
</template>
```

Create `frontend/src/components/ExecutionResultsPanel.spec.ts`:

```typescript
import { describe, it, expect } from 'vitest'
import { mount } from '@vue/test-utils'
import ExecutionResultsPanel from './ExecutionResultsPanel.vue'
import type { Execution } from '../types/domain'

const execution: Execution = {
  id: 'e1',
  workflow_id: 'w1',
  status: 'Success',
  mode: 'Manual',
  node_outputs: { n1: [{ json: { hello: 'world' }, binary: {} }] },
  started_at: 'x',
  finished_at: 'y',
}

describe('ExecutionResultsPanel', () => {
  it('renders the status and each node\'s output', () => {
    const wrapper = mount(ExecutionResultsPanel, { props: { execution } })
    expect(wrapper.text()).toContain('Success')
    expect(wrapper.text()).toContain('n1')
    expect(wrapper.text()).toContain('hello')
  })

  it('renders nothing when there is no execution', () => {
    const wrapper = mount(ExecutionResultsPanel, { props: { execution: null } })
    expect(wrapper.text()).toBe('')
  })
})
```

- [ ] **Step 2: Wire Save and Execute into the editor**

In `frontend/src/views/WorkflowEditorView.vue`, add:

```typescript
import ExecutionResultsPanel from '../components/ExecutionResultsPanel.vue'
import type { Execution } from '../types/domain'

const saving = ref(false)
const executing = ref(false)
const execution = ref<Execution | null>(null)

async function save() {
  if (!workflow.value) return
  saving.value = true
  try {
    workflow.value = await api.put<Workflow>(`/rest/workflows/${workflowId}`, {
      name: workflow.value.name,
      nodes: workflow.value.nodes,
      connections: workflow.value.connections,
    })
  } finally {
    saving.value = false
  }
}

async function execute() {
  executing.value = true
  try {
    execution.value = await api.post<Execution>(`/rest/workflows/${workflowId}/execute`)
  } finally {
    executing.value = false
  }
}
```

In the toolbar, after `<AddNodeMenu @add="onAddNode" />`, add:

```vue
<button
  class="bg-gray-200 text-gray-800 rounded px-3 py-1.5 text-sm disabled:opacity-50"
  :disabled="saving"
  @click="save"
>
  {{ saving ? 'Saving…' : 'Save' }}
</button>
<button
  class="bg-green-600 text-white rounded px-3 py-1.5 text-sm disabled:opacity-50"
  :disabled="executing"
  @click="execute"
>
  {{ executing ? 'Running…' : 'Execute' }}
</button>
```

Add `<ExecutionResultsPanel :execution="execution" @close="execution =
null" />` inside the `<div class="flex-1 relative">` wrapper, after
`<NodeConfigPanel>`.

- [ ] **Step 3: Run tests and build**

Run (from `frontend/`): `npm test && npm run build`
Expected: all tests pass, build succeeds.

- [ ] **Step 4: Full-stack manual verification**

This is the plan's final task — before committing, manually verify the
complete loop against a real running backend, matching this project's
established practice (not a substitute for the automated tests above,
an addition to them):

1. From the repo root: `cd frontend && npm install && npm run build && cd ..`
2. `cargo run` (with `DATABASE_URL`, `JWT_SECRET`, `CREDENTIALS_KEY` set,
   per `.env.example`).
3. Open `http://localhost:3000/` in a browser (or via a tool capable of
   driving one, if available in your environment) — confirm it redirects
   to `/login`, register a new account, confirm redirect to `/workflows`.
4. Create a workflow, open its editor, add a `core.manualTrigger` node and
   a `core.set` node, connect them, open the `core.set` node's config
   panel and set its parameters to something like `{"fields": {"hello":
   "world"}}`, Save, then Execute — confirm the results panel shows
   `Success` and the `core.set` node's output containing `{"hello":
   "world"}`.
5. Go back to `/workflows`, confirm the workflow appears, toggle it
   active/inactive, delete it, confirm it's gone from the list.

Report exactly what you verified and any deviation from expected
behavior in your report — this step's purpose is catching integration
issues no unit test can (real Axum routing, real embedded-asset serving,
real end-to-end data flow), so do not skip it or treat a passing
automated-test suite as a substitute for actually running the app.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/components/ExecutionResultsPanel.vue frontend/src/components/ExecutionResultsPanel.spec.ts frontend/src/views/WorkflowEditorView.vue
git commit -m "feat: add save/execute and results panel, completing the editor loop"
```

---

## Explicitly Out of Scope (this plan)

Carried forward per the spec's own §8 and the roadmap breakdown's
structure:

- Live execution status (WebSocket) — depends on Plan 7 §7.1, not built.
- Execution log/replay — needs an executions-listing API endpoint,
  deliberately not added in this plan (see spec §8's reasoning).
- Dynamic, schema-driven node parameter forms — the backend has no
  per-node parameter schema; this plan's raw-JSON editor is the
  deliberate v1 approach.
- Workflow edit-conflict handling beyond "last save wins."
- End-to-end browser test automation (Playwright etc.) — manual
  verification (Task 10, Step 4) substitutes for this in the first slice.
