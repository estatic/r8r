# r8r Credential Management — Design Spec

## 1. Summary

Credentials can only be created and listed today (`POST`/`GET
/rest/credentials`, `Storage::{create,get,list}_credential(s)`); the only
UI is the "+ New credential" form inside a node's `CredentialPicker`
(Plan 8.4). An expired token means creating a new credential and
re-selecting it on every node; a typo in a name is permanent; nothing can
be removed. This spec adds editing (secret values and name), deletion
guarded by usage, and a Credentials page.

## 2. Goals / Non-Goals

**Goals:**
- Change a credential's secret values without re-entering the ones that
  didn't change; rename it. The credential keeps its id, so nodes that
  reference it keep working.
- Delete a credential that no workflow uses; refuse, naming the
  workflows, when one does.
- One page listing every credential with how many workflows use it.
- Secrets never leave the server once stored (unchanged guarantee).

**Non-Goals:** see §7.

## 3. Backend

### 3.1 Storage (`src/storage/mod.rs`, `src/storage/sqlite.rs`)

Two new `Storage` methods:

```rust
async fn update_credential(&self, credential: &Credential) -> anyhow::Result<bool>; // false = not found
async fn delete_credential(&self, id: Uuid) -> anyhow::Result<bool>;               // false = not found
```

`update_credential` re-encrypts `data` exactly as `create_credential`
does and writes `name`, `data`, `updated_at` (never `credential_type`,
`owner_id`, `created_at`). The test-only `Storage` impls
(`execution_runner` `CountingStorage`, `telegram_poller`
`RecordingStorage`, `tests/api_test.rs` `FailingUpdateStorage`) delegate
to their inner storage or `unimplemented!()` as their other unused
methods do.

### 3.2 Usage (`src/credentials.rs`)

```rust
pub fn workflows_using_credential(workflows: &[Workflow], id: Uuid) -> Vec<(Uuid, String)>
```

Returns `(workflow id, name)` for every workflow with a node whose
`parameters.auth.credential_id` equals `id` — the same field
`resolve_credentials_for_workflow` reads. Computed from
`Storage::list_workflows` in the API layer; no new storage method. (The
design discussion mentioned a storage-level `credential_usage`; a pure
function over the workflow list is simpler and equally correct at this
scale.)

### 3.3 API (`src/api/credentials.rs`, routes in `src/api/mod.rs`)

All endpoints require `AuthUser`, like the existing two.

- **`GET /rest/credentials`** — each summary gains `used_by: usize`
  (count of workflows from §3.2). Existing fields unchanged.
- **`GET /rest/credentials/:id`** → 200
  `{ ...CredentialSummary, used_by, fields: { <name>: <value> } }` where
  `fields` holds only the stored values of the schema's **text**-type
  fields (`known_credential_types()`, `FieldType::Text`). Password-type
  fields are never included; a credential whose type has no schema gets
  `fields: {}`. 404 if missing.
- **`PATCH /rest/credentials/:id`** body `{ name?: string, data?: object }`
  → 200 with the updated summary (+ `used_by`), 404 if missing, 400 if
  `name` is present but blank or `data` is not a JSON object.
  - `name`: trimmed, replaces the name.
  - `data`, **typed** credential (schema exists): merged key by key into
    the stored data; a key whose value is `""` or absent is left
    unchanged ("blank = keep current"); keys not in the schema are
    ignored. A required field can therefore never be cleared.
  - `data`, **untyped** credential (no schema): replaces the stored data
    entirely.
  - `credential_type` in the body is ignored (type is fixed after
    creation).
  - `updated_at` is set to now.
- **`DELETE /rest/credentials/:id`** → 204; 404 if missing; **409** with
  `{ "error": "credential is in use", "workflows": [{ "id", "name" }] }`
  when §3.2 returns any workflow.

Each successful update/delete logs at info (`credential updated` /
`credential deleted`, with id and name — never data).

## 4. Frontend

### 4.1 `CredentialForm.vue` (new, `frontend/src/components/`)

Extracted from `CredentialPicker.vue`'s create form (Plan 8.4 logic:
schema-driven fields, generic types + Custom, raw-JSON fallback,
autocomplete opt-outs, aria labels, trimming). Props:
`mode: 'create' | 'edit'`, `acceptedTypes: string[]` (create only),
`credentialId?: string` (edit only). Emits `saved(summary)` and
`cancel`.

In **edit** mode it loads `GET /rest/credentials/:id` and:
- shows the type read-only;
- pre-fills the name and the returned text fields;
- renders password fields empty with placeholder `•••••• (unchanged)`
  and no required-check (blank = keep);
- for an untyped credential shows an empty JSON textarea with the note
  "Enter the full JSON to replace the stored data, or leave empty to
  keep it" — empty sends no `data`;
- submits `PATCH` with `name` and only the non-empty field values.

`CredentialPicker.vue` renders `<CredentialForm mode="create">` in place
of its inline form (behaviour unchanged) and gains an **Edit** link next
to the dropdown when a credential is selected, opening
`<CredentialForm mode="edit">` inline; on `saved` the credentials store
refreshes.

### 4.2 Store (`frontend/src/stores/credentials.ts`)

Adds `get(id)`, `update(id, patch)`, `remove(id)`; `update`/`remove`
refresh the list. `CredentialSummary` gains `used_by: number`. `remove`
surfaces a 409 as an error carrying the workflow names.

### 4.3 Credentials page (`/credentials`, `CredentialsView.vue`)

- Route `{ path: '/credentials', name: 'credentials' }` (auth-guarded
  like `/workflows`); a "Credentials" link in the workflow list header
  and a "Workflows" link back.
- Table: name, type, "used by N workflow(s)", last updated; **New
  credential** button (form in create mode, all generic + known types
  offered) and per-row **Edit** / **Delete**.
- Delete asks for confirmation; on 409 shows "Can't delete: used by
  <names>. Remove it from those workflows first."

## 5. Security

- No endpoint returns password-type values or untyped data; the
  existing encryption-at-rest path is reused for updates.
- PATCH cannot change `credential_type` or `owner_id`.
- Logs never include credential data.

## 6. Testing

**Storage** (`sqlite.rs` tests): update round-trips decrypted data and
new name, keeps `created_at`/type, returns false for a missing id;
delete removes and returns false for a missing id.

**Usage** (`credentials.rs`): finds workflows by `auth.credential_id`,
ignores other ids and nodes without auth.

**API** (`tests/api_test.rs`):
- GET by id returns text fields and never a password field (checked on
  `apiKeyHeader`: `header_name` present, `value` absent).
- PATCH with only `name` keeps the secret, verified by reading the
  credential back through `Storage::get_credential` via
  `test_app_with_state()`.
- PATCH with `{ "bot_token": "" }` keeps the old token; with a new
  value replaces it; untyped PATCH replaces data.
- PATCH blank name → 400; missing id → 404.
- DELETE while used → 409 naming the workflow; after removing the
  reference → 204; missing → 404.
- List includes `used_by`.

**Frontend** (Vitest):
- `CredentialForm` edit mode: pre-fills text fields, password fields
  empty with the unchanged placeholder, submits only changed values.
- `CredentialsView`: lists rows with used-by counts; delete confirm →
  409 message names the workflows; 204 removes the row.
- `CredentialPicker`: create behaviour unchanged (existing tests stay
  green); Edit link appears only when a credential is selected.

## 7. Out of Scope / Deferred

- Per-user credential ownership / sharing rules (single shared
  workspace, unchanged).
- Changing a credential's type.
- Revealing stored secrets.
- Credential rotation history / audit log.
- Testing a credential against its service ("Test connection").
