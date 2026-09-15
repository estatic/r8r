# r8r Plan 4a — Credential Storage + HTTP Request Node — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give r8r encrypted-at-rest Credential storage (AES-256-GCM) and a generic HTTP Request node that can use those credentials for Bearer/API-key/Basic auth — the foundational pair the roadmap's Plan 4 needs before Telegram (a "thin wrapper over HTTP Request") can be built on top.

**Architecture:** A new `src/crypto.rs` module provides symmetric encrypt/decrypt helpers (AES-256-GCM, random nonce per call) and a startup key-loader; `SqliteStorage` owns the encryption key (passed at construction, mirroring how it already owns its connection pool) and encrypts a credential's `data` before writing it, decrypting only when the EXECUTION path needs it — the public HTTP API never returns decrypted (or even encrypted) secret material, only metadata (`CredentialSummary`). `NodeExecutionContext` gains a `credentials: HashMap<Uuid, serde_json::Value>` field (decrypted plaintext, pre-resolved once per workflow run by whichever caller starts the run — manual execute, webhook, or schedule — never decrypted inside a node's own `execute()`), defaulted via `#[derive(Default)]` so every existing node's test fixtures need only a mechanical one-line addition. `engine::execute_workflow_seeded` gains a `credentials: &HashMap<Uuid, serde_json::Value>` parameter; the simple `execute_workflow` wrapper stays exactly as it is today (2 arguments, empty credentials map) so its 15 existing test callers are completely unaffected. The new HTTP Request node (`core.httpRequest`) is an ordinary node — a non-2xx response becomes `Err(NodeError::ExecutionFailed(..))`, which Plan 2's existing per-node error-output routing already handles with zero new engine work.

**Tech Stack:** Rust, `aes-gcm` (RustCrypto, AES-256-GCM), `reqwest` (HTTP client, `rustls-tls` feature to match the rest of the stack's TLS choice), `wiremock` (dev-only, HTTP mocking per the spec's own testing strategy), the existing Axum/sqlx/Tokio stack (unchanged).

**Spec:** `docs/superpowers/specs/2026-09-13-r8r-workflow-automation-design.md` (§4 Data Model — Credential; §5.1 `NodeExecutionContext`; §5.3 HTTP Request; §11 Testing Strategy)
**Roadmap reference:** `docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`, Plan 4 §4.1–4.2 (§4.3 Telegram is explicitly deferred — see Explicitly Out of Scope)

## Global Constraints

- Credentials are encrypted at rest with AES-256-GCM (spec §4, verbatim requirement). The encryption key is a 32-byte key, base64-encoded in the `CREDENTIALS_KEY` environment variable — loaded once at startup, same fail-fast-if-missing pattern as the existing `JWT_SECRET` (`main.rs` already refuses to start without `JWT_SECRET`; `CREDENTIALS_KEY` follows the identical pattern).
- A random 12-byte nonce is generated per encryption call (never reused) and stored alongside the ciphertext, not derived from anything predictable.
- The public HTTP API (`/rest/credentials/*`) NEVER returns decrypted secret material, and never returns the ciphertext blob either — every response uses `CredentialSummary` (`id`, `name`, `credential_type`, `owner_id`, `created_at`, `updated_at`), never the `data` field. There is no `GET /rest/credentials/:id` endpoint in this plan (omitting it is the safe default; nothing in the spec requires one).
- Decryption happens in exactly one place in the request-handling flow: once per workflow run, before `execute_workflow_seeded` is called, building the `HashMap<Uuid, serde_json::Value>` that becomes `NodeExecutionContext.credentials`. A node's own `execute()` never talks to `Storage` or `crypto` directly — it only reads `ctx.credentials`, already-decrypted, already scoped to whatever credential id its own resolved parameters reference.
- `NodeExecutionContext` gains `#[derive(Default)]` alongside its existing derives, so the ~31 pre-existing test-fixture construction sites across `src/nodes/*.rs` need only a mechanical `..Default::default()` addition — no other change to their behavior or assertions. The ONE production construction site (`src/engine.rs`, inside `execute_workflow_seeded`) populates the field for real from the new `credentials` parameter.
- `execute_workflow` (the simple 2-argument wrapper around `execute_workflow_seeded`) is UNCHANGED in its public signature — it now internally passes an empty credentials map. Its 15 existing test callers across the crate need zero changes.
- `execute_workflow_seeded` gains a 4th parameter, `credentials: &HashMap<uuid::Uuid, serde_json::Value>`. Its 3 existing call sites (`src/triggers.rs`, `src/api/webhook.rs`, one test in `src/engine.rs`) are updated to pass `&HashMap::new()` in this plan — none of them gain real credential-resolution logic here; only `src/api/workflows.rs`'s manual-execute handler does (see Task 9), since it's the only one this plan's own end-to-end test exercises. Wiring credential resolution into the webhook/schedule paths too is a natural, small follow-up once this lands, not required by this plan's own scope.
- HTTP Request node parameters: `{"method": "GET"|"POST"|"PUT"|"PATCH"|"DELETE", "url": "<string, expression-resolved>", "headers": {<string:string>}, "query": {<string:string>}, "body": <JSON value, optional, sent as the request's JSON body when present>, "auth": {"type": "none"|"bearer"|"apiKey"|"basic", "credential_id": "<uuid string, required unless type is \"none\">"}}`. Credential `data` shapes: `bearer` → `{"token": "<string>"}`; `apiKey` → `{"header_name": "<string>", "value": "<string>"}`; `basic` → `{"username": "<string>", "password": "<string>"}`.
- A non-2xx HTTP response is `Err(NodeError::ExecutionFailed(..))` — no new engine mechanism; this flows through Plan 2's already-built error-output routing (a connected error output receives it, otherwise the run aborts) exactly like any other node's error today.
- No Telegram nodes, no AI Agent tool integration, no binary/file response handling (only JSON/text response bodies) — all explicitly out of scope, carried to a later plan.

---

## File Structure

- `Cargo.toml` — add `aes-gcm`, `base64`, `rand`, `reqwest` (`rustls-tls`, `json` features); add `wiremock` to `[dev-dependencies]`.
- `src/crypto.rs` — new. `encrypt`, `decrypt`, `load_key_from_env`.
- `src/domain.rs` — modify. `Credential`, `CredentialSummary` types.
- `migrations/0002_credentials.sql` — new. `credentials` table.
- `src/storage/mod.rs`, `src/storage/sqlite.rs` — modify. `create_credential`/`get_credential`/`list_credentials`; `SqliteStorage` gains an encryption-key field, `SqliteStorage::new`'s signature changes.
- `src/node.rs` — modify. `NodeExecutionContext` gains `credentials` field + `#[derive(Default)]`.
- `src/nodes/{code,filter,if_node,manual_trigger,merge,noop,schedule,set,switch,wait,webhook}.rs` — modify (mechanical). Every test-fixture `NodeExecutionContext { .. }` literal gains `..Default::default()`.
- `src/engine.rs` — modify. `execute_workflow_seeded` gains the `credentials` parameter; its one real `NodeExecutionContext` construction site populates it; its own test-fixture site and `execute_workflow`'s wrapper both get the mechanical/empty-map treatment.
- `src/api/webhook.rs`, `src/triggers.rs` — modify (mechanical). Pass `&HashMap::new()` at their `execute_workflow_seeded` call sites.
- `src/credentials.rs` — new. `resolve_credentials_for_workflow(storage, workflow) -> anyhow::Result<HashMap<Uuid, serde_json::Value>>`.
- `src/api/credentials.rs` — new. `POST /rest/credentials`, `GET /rest/credentials`.
- `src/api/workflows.rs` — modify. `execute_workflow` handler switches from the 2-arg wrapper to `execute_workflow_seeded` with resolved credentials.
- `src/api/mod.rs` — modify. Wire the new routes.
- `src/nodes/http_request.rs` — new. `HttpRequestNode`.
- `src/lib.rs` — modify. `pub mod crypto; pub mod credentials;`.
- `src/main.rs` — modify. Load `CREDENTIALS_KEY`, construct `SqliteStorage` with it.
- `tests/api_test.rs` — modify. One new end-to-end test: create a credential, create+execute a workflow (Manual Trigger → HTTP Request with Bearer auth against a `wiremock` server), assert the mocked server received the right `Authorization` header and the node captured the response.

---

### Task 1: Crypto module (AES-256-GCM encrypt/decrypt + key loading)

**Files:**
- Create: `src/crypto.rs`
- Modify: `Cargo.toml` (add `aes-gcm`, `base64`, `rand`)
- Modify: `src/lib.rs`

**Interfaces:**
- Produces: `r8r::crypto::encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String>` — encrypts `plaintext` with a fresh random 12-byte nonce, returns `base64(nonce || ciphertext)` as one string.
- Produces: `r8r::crypto::decrypt(key: &[u8; 32], blob: &str) -> anyhow::Result<String>` — reverses `encrypt`; `Err` on malformed base64, too-short data, or a failed AEAD tag check (wrong key/tampered data).
- Produces: `r8r::crypto::load_key_from_env(var_name: &str) -> anyhow::Result<[u8; 32]>` — reads the named env var, base64-decodes it, `Err` if unset or not exactly 32 bytes after decoding.

- [ ] **Step 1: Add dependencies**

```toml
# Cargo.toml, in [dependencies]
aes-gcm = "0.10"
base64 = "0.22"
rand = "0.8"
```

- [ ] **Step 2: Write failing tests**

```rust
// src/crypto.rs, tests module
#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn encrypt_then_decrypt_round_trips() {
        let key = test_key();
        let ciphertext = encrypt(&key, "super secret token").unwrap();
        let plaintext = decrypt(&key, &ciphertext).unwrap();
        assert_eq!(plaintext, "super secret token");
    }

    #[test]
    fn two_encryptions_of_the_same_plaintext_differ() {
        // Different random nonces each call -> different ciphertext blobs,
        // even for identical input. Proves the nonce isn't fixed/reused.
        let key = test_key();
        let a = encrypt(&key, "same input").unwrap();
        let b = encrypt(&key, "same input").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let ciphertext = encrypt(&test_key(), "secret").unwrap();
        let wrong_key = [9u8; 32];
        assert!(decrypt(&wrong_key, &ciphertext).is_err());
    }

    #[test]
    fn decrypt_malformed_blob_returns_err_not_panic() {
        assert!(decrypt(&test_key(), "not-valid-base64!!!").is_err());
        assert!(decrypt(&test_key(), "").is_err());
    }

    #[test]
    fn load_key_from_env_reads_and_decodes_base64() {
        let key_bytes = [3u8; 32];
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, key_bytes);
        std::env::set_var("R8R_TEST_CREDENTIALS_KEY", &encoded);
        let loaded = load_key_from_env("R8R_TEST_CREDENTIALS_KEY").unwrap();
        assert_eq!(loaded, key_bytes);
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY");
    }

    #[test]
    fn load_key_from_env_errors_when_unset() {
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY_MISSING");
        assert!(load_key_from_env("R8R_TEST_CREDENTIALS_KEY_MISSING").is_err());
    }

    #[test]
    fn load_key_from_env_errors_on_wrong_length() {
        let short = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8; 16]);
        std::env::set_var("R8R_TEST_CREDENTIALS_KEY_SHORT", &short);
        assert!(load_key_from_env("R8R_TEST_CREDENTIALS_KEY_SHORT").is_err());
        std::env::remove_var("R8R_TEST_CREDENTIALS_KEY_SHORT");
    }
}
```

- [ ] **Step 3: Run, confirm compile failure**

Run: `cargo test --lib crypto::tests`
Expected: compile error — `encrypt`/`decrypt`/`load_key_from_env` undefined (module not wired into `lib.rs` yet either).

- [ ] **Step 4: Implement (reference implementation — verify method names against the installed `aes-gcm`/`base64` crate versions; both are widely-used, stable crates but this plan's author could not check live docs while writing this)**

```rust
// top of src/crypto.rs, above the tests module
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, AeadCore, Key, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

pub fn encrypt(key: &[u8; 32], plaintext: &str) -> anyhow::Result<String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(combined))
}

pub fn decrypt(key: &[u8; 32], blob: &str) -> anyhow::Result<String> {
    let combined = BASE64
        .decode(blob)
        .map_err(|e| anyhow::anyhow!("invalid base64: {e}"))?;
    if combined.len() < 12 {
        return Err(anyhow::anyhow!("ciphertext blob too short"));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| anyhow::anyhow!("decrypted data is not valid UTF-8: {e}"))
}

pub fn load_key_from_env(var_name: &str) -> anyhow::Result<[u8; 32]> {
    let encoded = std::env::var(var_name)
        .map_err(|_| anyhow::anyhow!("{var_name} environment variable must be set"))?;
    let decoded = BASE64
        .decode(&encoded)
        .map_err(|e| anyhow::anyhow!("{var_name} is not valid base64: {e}"))?;
    decoded
        .try_into()
        .map_err(|v: Vec<u8>| anyhow::anyhow!("{var_name} must decode to exactly 32 bytes, got {}", v.len()))
}
```

Add `pub mod crypto;` to `src/lib.rs`.

- [ ] **Step 5: Run tests, adapting to the installed crate APIs as needed**

Run: `cargo build` first to surface any API mismatches, fix them, then:

Run: `cargo test --lib crypto::tests`
Expected: all seven tests PASS.

Note: these tests use `std::env::set_var`/`remove_var` on distinctly-named test-only variables — if `cargo test` runs this file's tests in parallel with other tests that also touch process-wide env vars under the SAME names, there could be flakiness; the chosen names (`R8R_TEST_CREDENTIALS_KEY*`) are unique to this file specifically to avoid that. Don't rename them without preserving uniqueness.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/crypto.rs src/lib.rs
git commit -m "feat: add AES-256-GCM crypto module for credential encryption"
```

---

### Task 2: `Credential`/`CredentialSummary` domain types + migration

**Files:**
- Modify: `src/domain.rs`
- Create: `migrations/0002_credentials.sql`

**Interfaces:**
- Produces: `r8r::domain::Credential { id: Uuid, name: String, credential_type: String, data: serde_json::Value, owner_id: Uuid, created_at: DateTime<Utc>, updated_at: DateTime<Utc> }` — `data` is the PLAINTEXT representation in Rust; encryption/decryption happens entirely inside the storage layer (Task 4), never inside this type itself.
- Produces: `r8r::domain::CredentialSummary { id: Uuid, name: String, credential_type: String, owner_id: Uuid, created_at: DateTime<Utc>, updated_at: DateTime<Utc> }` — everything about a `Credential` except `data`; this is the only shape the public API ever serializes.

- [ ] **Step 1: Write the failing test**

```rust
// add to the tests module in src/domain.rs
#[test]
fn credential_summary_never_serializes_a_data_field() {
    let cred = Credential {
        id: Uuid::new_v4(),
        name: "my-api".into(),
        credential_type: "bearer".into(),
        data: serde_json::json!({"token": "super-secret-value"}),
        owner_id: Uuid::new_v4(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let summary = CredentialSummary::from(&cred);
    let json = serde_json::to_value(&summary).unwrap();
    assert!(json.get("data").is_none());
    assert_eq!(json["name"], "my-api");
    // The actual secret value must never appear anywhere in the serialized summary.
    let serialized = serde_json::to_string(&summary).unwrap();
    assert!(!serialized.contains("super-secret-value"));
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib domain::tests::credential_summary_never_serializes_a_data_field`
Expected: compile error — `Credential`/`CredentialSummary` undefined.

- [ ] **Step 3: Implement the types**

```rust
// src/domain.rs — add alongside the existing domain types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Credential {
    pub id: Uuid,
    pub name: String,
    pub credential_type: String,
    pub data: serde_json::Value,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CredentialSummary {
    pub id: Uuid,
    pub name: String,
    pub credential_type: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&Credential> for CredentialSummary {
    fn from(c: &Credential) -> Self {
        Self {
            id: c.id,
            name: c.name.clone(),
            credential_type: c.credential_type.clone(),
            owner_id: c.owner_id,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}
```

- [ ] **Step 4: Add the migration**

```sql
-- migrations/0002_credentials.sql
CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    data TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

(`data` stores the base64 `nonce || ciphertext` blob produced by `crypto::encrypt` — plain `TEXT`, no different in kind from `workflows.definition`.)

- [ ] **Step 5: Run tests**

Run: `cargo test --lib domain::tests`
Expected: all pass. Migration correctness is verified in Task 4 once `SqliteStorage` actually runs it.

- [ ] **Step 6: Commit**

```bash
git add src/domain.rs migrations/0002_credentials.sql
git commit -m "feat: add Credential/CredentialSummary domain types and migration"
```

---

### Task 3: `Storage::create_credential`/`get_credential`/`list_credentials`

**Files:**
- Modify: `src/storage/mod.rs`
- Modify: `src/storage/sqlite.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Produces: `Storage::create_credential(&self, credential: &Credential) -> anyhow::Result<()>` — encrypts `credential.data` (via `crypto::encrypt` with the storage's own key) before persisting.
- Produces: `Storage::get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>>` — decrypts `data` before returning; this is the ONLY function in the whole codebase that ever reconstructs a `Credential` with real plaintext `data` — callers outside the execution-credential-resolution path (Task 8) must never expose its result via an API response.
- Produces: `Storage::list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>>` — never decrypts; queries only the non-`data` columns.
- Changes: `SqliteStorage::new(db_url: &str, encryption_key: [u8; 32]) -> anyhow::Result<Self>` — gains the `encryption_key` parameter, stored as a field, used by `create_credential`/`get_credential`.
- Changes: `main.rs` — loads `CREDENTIALS_KEY` via `crypto::load_key_from_env("CREDENTIALS_KEY")` (fail-fast, same pattern as `JWT_SECRET`) and passes it to `SqliteStorage::new`.

- [ ] **Step 1: Write failing tests**

```rust
// add to the tests module in src/storage/sqlite.rs
use crate::domain::Credential;

fn test_key() -> [u8; 32] {
    [5u8; 32]
}

async fn storage_with_test_key() -> SqliteStorage {
    SqliteStorage::new("sqlite::memory:", test_key()).await.unwrap()
}

fn sample_credential(owner_id: Uuid) -> Credential {
    Credential {
        id: Uuid::new_v4(),
        name: "test-api".into(),
        credential_type: "bearer".into(),
        data: serde_json::json!({"token": "abc123secret"}),
        owner_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn create_and_get_credential_round_trips_decrypted() {
    let storage = storage_with_test_key().await;
    let user = sample_user();
    storage.create_user(&user).await.unwrap();
    let cred = sample_credential(user.id);
    storage.create_credential(&cred).await.unwrap();

    let fetched = storage.get_credential(cred.id).await.unwrap().unwrap();
    assert_eq!(fetched.data, serde_json::json!({"token": "abc123secret"}));
    assert_eq!(fetched.name, "test-api");
}

#[tokio::test]
async fn get_credential_returns_none_when_missing() {
    let storage = storage_with_test_key().await;
    assert!(storage.get_credential(Uuid::new_v4()).await.unwrap().is_none());
}

#[tokio::test]
async fn list_credentials_never_includes_decrypted_data() {
    let storage = storage_with_test_key().await;
    let user = sample_user();
    storage.create_user(&user).await.unwrap();
    storage.create_credential(&sample_credential(user.id)).await.unwrap();

    let summaries = storage.list_credentials().await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "test-api");
    // CredentialSummary has no `data` field at the type level -- this test
    // exists to document that guarantee at the storage layer, not just the
    // API-serialization layer (Task 2's test covers serialization).
}

#[tokio::test]
async fn stored_data_is_actually_encrypted_at_rest() {
    // Read the raw column value directly (bypassing get_credential's decrypt
    // step) and confirm the plaintext secret never appears in it.
    let storage = storage_with_test_key().await;
    let user = sample_user();
    storage.create_user(&user).await.unwrap();
    let cred = sample_credential(user.id);
    storage.create_credential(&cred).await.unwrap();

    let raw: (String,) = sqlx::query_as("SELECT data FROM credentials WHERE id = ?")
        .bind(cred.id.to_string())
        .fetch_one(&storage.pool)
        .await
        .unwrap();
    assert!(!raw.0.contains("abc123secret"));
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib storage::sqlite::tests::create_and_get_credential`
Expected: compile error — `create_credential`/`get_credential`/`list_credentials` not on the `Storage` trait; `SqliteStorage::new` doesn't take an encryption key yet.

- [ ] **Step 3: Add to the trait**

```rust
// src/storage/mod.rs — add inside the Storage trait
async fn create_credential(&self, credential: &Credential) -> anyhow::Result<()>;
async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>>;
async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>>;
```

Update the `use crate::domain::{...}` import at the top of the file to include `Credential, CredentialSummary`.

- [ ] **Step 4: Implement in `SqliteStorage`**

```rust
// src/storage/sqlite.rs — SqliteStorage struct and constructor
pub struct SqliteStorage {
    pool: SqlitePool,
    encryption_key: [u8; 32],
}

impl SqliteStorage {
    pub async fn new(db_url: &str, encryption_key: [u8; 32]) -> anyhow::Result<Self> {
        // ... existing body unchanged down to `sqlx::migrate!(...)`, then:
        Ok(Self { pool, encryption_key })
    }
}
```

```rust
// src/storage/sqlite.rs — inside impl Storage for SqliteStorage
async fn create_credential(&self, credential: &Credential) -> anyhow::Result<()> {
    let plaintext = serde_json::to_string(&credential.data)?;
    let encrypted = crate::crypto::encrypt(&self.encryption_key, &plaintext)?;
    sqlx::query(
        "INSERT INTO credentials (id, name, credential_type, data, owner_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(credential.id.to_string())
    .bind(&credential.name)
    .bind(&credential.credential_type)
    .bind(encrypted)
    .bind(credential.owner_id.to_string())
    .bind(credential.created_at.to_rfc3339())
    .bind(credential.updated_at.to_rfc3339())
    .execute(&self.pool)
    .await?;
    Ok(())
}

async fn get_credential(&self, id: Uuid) -> anyhow::Result<Option<Credential>> {
    let row = sqlx::query_as::<_, (String, String, String, String, String, String, String)>(
        "SELECT id, name, credential_type, data, owner_id, created_at, updated_at FROM credentials WHERE id = ?"
    )
    .bind(id.to_string())
    .fetch_optional(&self.pool)
    .await?;
    row.map(|r| row_to_credential(r, &self.encryption_key)).transpose()
}

async fn list_credentials(&self) -> anyhow::Result<Vec<CredentialSummary>> {
    let rows = sqlx::query_as::<_, (String, String, String, String, String, String)>(
        "SELECT id, name, credential_type, owner_id, created_at, updated_at FROM credentials"
    )
    .fetch_all(&self.pool)
    .await?;
    rows.into_iter().map(row_to_credential_summary).collect()
}
```

```rust
// src/storage/sqlite.rs — free functions, alongside row_to_workflow/row_to_execution
fn row_to_credential(
    row: (String, String, String, String, String, String, String),
    key: &[u8; 32],
) -> anyhow::Result<Credential> {
    let (id, name, credential_type, encrypted_data, owner_id, created_at, updated_at) = row;
    let plaintext = crate::crypto::decrypt(key, &encrypted_data)?;
    Ok(Credential {
        id: Uuid::parse_str(&id)?,
        name,
        credential_type,
        data: serde_json::from_str(&plaintext)?,
        owner_id: Uuid::parse_str(&owner_id)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}

fn row_to_credential_summary(
    row: (String, String, String, String, String, String),
) -> anyhow::Result<CredentialSummary> {
    let (id, name, credential_type, owner_id, created_at, updated_at) = row;
    Ok(CredentialSummary {
        id: Uuid::parse_str(&id)?,
        name,
        credential_type,
        owner_id: Uuid::parse_str(&owner_id)?,
        created_at: chrono::DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&chrono::Utc),
        updated_at: chrono::DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&chrono::Utc),
    })
}
```

Add `use crate::domain::{Credential, CredentialSummary, ...}` to this file's existing import line.

- [ ] **Step 5: Update every other `SqliteStorage::new` call site**

`SqliteStorage::new`'s signature changed (gained `encryption_key`). Grep the whole crate for `SqliteStorage::new(` and update every call site (test helpers in `src/storage/sqlite.rs`'s own tests module, `src/main.rs`, and `tests/api_test.rs`/`tests/health_test.rs`'s `test_app()` helpers) to pass a key. Test call sites can use any fixed 32-byte array (e.g. `[0u8; 32]`) since they don't exercise real secret material. `src/main.rs` must load a REAL key via `crypto::load_key_from_env("CREDENTIALS_KEY")`:

```rust
// src/main.rs — add near the existing jwt_secret load
let credentials_key = r8r::crypto::load_key_from_env("CREDENTIALS_KEY")?;
// ... and update the SqliteStorage::new call:
let storage = SqliteStorage::new(&database_url, credentials_key).await?;
```

- [ ] **Step 6: Run tests**

Run: `cargo test --lib storage::sqlite::tests`
Expected: all pass, including the four new tests.

- [ ] **Step 7: Run the whole crate**

Run: `cargo build` — confirm it compiles (every `SqliteStorage::new` call site updated). `.env.example` should also gain a `CREDENTIALS_KEY=` line with a comment noting it must be 32 raw bytes, base64-encoded (e.g. generate one with `openssl rand -base64 32`) — add this now since `main.rs` will refuse to start without it, same as `JWT_SECRET`.

Run: `cargo test`
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/storage/mod.rs src/storage/sqlite.rs src/main.rs .env.example
git commit -m "feat: add Storage::create_credential/get_credential/list_credentials"
```

---

### Task 4: `POST /rest/credentials` and `GET /rest/credentials`

**Files:**
- Create: `src/api/credentials.rs`
- Modify: `src/api/mod.rs`

**Interfaces:**
- Produces: `create_credential(State(state), AuthUser(user_id), Json(payload)) -> impl IntoResponse` for `POST /rest/credentials`, body `{"name": "<string>", "credential_type": "<string>", "data": <JSON object>}`. Creates a `Credential` owned by the authenticated user, persists it (encrypted, via `Storage::create_credential`), responds `201` with its `CredentialSummary` (never echoes `data` back, even though the caller obviously already has it).
- Produces: `list_credentials(State(state), AuthUser(user_id)) -> impl IntoResponse` for `GET /rest/credentials`. Returns every credential's `CredentialSummary` as a JSON array. (v1 scope: all authenticated users see all credentials, matching the existing workflow-listing endpoint's same-shared-workspace model — no per-owner filtering, consistent with spec §8's stated v1 access model.)

- [ ] **Step 1: Write failing tests**

```rust
// add to tests/api_test.rs
#[tokio::test]
async fn create_credential_never_returns_data_field() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred1@example.com").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({
                        "name": "my-bearer-cred",
                        "credential_type": "bearer",
                        "data": {"token": "super-secret-123"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["name"], "my-bearer-cred");
    assert!(body.get("data").is_none());
    let raw = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(!raw.contains("super-secret-123"));
}

#[tokio::test]
async fn list_credentials_returns_created_ones_without_data() {
    let app = test_app().await;
    let token = register_and_get_token(&app, "cred2@example.com").await;

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "cred-a", "credential_type": "bearer", "data": {"token": "x"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/rest/credentials")
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
    assert_eq!(list[0]["name"], "cred-a");
    assert!(list[0].get("data").is_none());
}

#[tokio::test]
async fn create_credential_requires_auth() {
    let app = test_app().await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "x", "credential_type": "bearer", "data": {}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Run, confirm compile failure/404**

Run: `cargo test --test api_test credential`
Expected: fails — the routes don't exist yet.

- [ ] **Step 3: Implement**

```rust
// src/api/credentials.rs
use crate::api::workflows::AuthUser;
use crate::domain::{Credential, CredentialSummary};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct CreateCredentialRequest {
    pub name: String,
    pub credential_type: String,
    pub data: serde_json::Value,
}

pub async fn create_credential(
    State(state): State<AppState>,
    AuthUser(user_id): AuthUser,
    Json(payload): Json<CreateCredentialRequest>,
) -> impl IntoResponse {
    let now = chrono::Utc::now();
    let credential = Credential {
        id: Uuid::new_v4(),
        name: payload.name,
        credential_type: payload.credential_type,
        data: payload.data,
        owner_id: user_id,
        created_at: now,
        updated_at: now,
    };
    match state.storage.create_credential(&credential).await {
        Ok(()) => (StatusCode::CREATED, Json(CredentialSummary::from(&credential))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to create credential");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn list_credentials(
    State(state): State<AppState>,
    AuthUser(_user_id): AuthUser,
) -> impl IntoResponse {
    match state.storage.list_credentials().await {
        Ok(summaries) => Json(summaries).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to list credentials");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
```

`AuthUser` is already `pub` in `src/api/workflows.rs` (used by every existing authenticated handler) — reuse it exactly as shown, don't redefine it.

- [ ] **Step 4: Wire the module and routes**

```rust
// src/api/mod.rs
pub mod credentials;
// ... existing pub mod lines ...

// inside build_router, add:
.route("/rest/credentials", post(credentials::create_credential).get(credentials::list_credentials))
```

- [ ] **Step 5: Run tests**

Run: `cargo test --test api_test credential`
Expected: all three PASS.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/api/credentials.rs src/api/mod.rs tests/api_test.rs
git commit -m "feat: add POST/GET /rest/credentials endpoints"
```

---

### Task 5: `NodeExecutionContext` gains `credentials` + `Default`

**Files:**
- Modify: `src/node.rs`

**Interfaces:**
- Changes: `r8r::node::NodeExecutionContext` — adds `pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>`; the struct gains `#[derive(Default)]` alongside its existing `#[derive(Debug, Clone)]`.

- [ ] **Step 1: Write the failing test**

```rust
// add to the tests module in src/node.rs
#[test]
fn node_execution_context_default_has_empty_credentials() {
    let ctx = NodeExecutionContext {
        parameters: serde_json::json!({}),
        input_items: vec![],
        ..Default::default()
    };
    assert!(ctx.credentials.is_empty());
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib node::tests::node_execution_context_default_has_empty_credentials`
Expected: compile error — `credentials` field doesn't exist, `Default` isn't derived.

- [ ] **Step 3: Implement**

```rust
// src/node.rs — replace the NodeExecutionContext struct
#[derive(Debug, Clone, Default)]
pub struct NodeExecutionContext {
    pub parameters: serde_json::Value,
    pub input_items: Vec<Item>,
    pub credentials: std::collections::HashMap<uuid::Uuid, serde_json::Value>,
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --lib node::tests`
Expected: the new test passes. The pre-existing `registry_dispatches_to_registered_node` test in this same file constructs a `NodeExecutionContext { parameters: ..., input_items: ... }` WITHOUT `..Default::default()` — it will now fail to compile. Add `..Default::default()` to that one construction site too, in this same task (it lives in the file you're already editing).

- [ ] **Step 5: Run the whole crate**

Run: `cargo build`
Expected: fails to compile — every OTHER `NodeExecutionContext { .. }` literal across `src/nodes/*.rs` and `src/engine.rs` (31 more sites) is now missing the new field too. **Do not fix them in this task** — that is Task 6's job entirely (a dedicated mechanical pass) and Task 7's job for `src/engine.rs`'s one real, non-test construction site. Confirm via the compiler's error list that every remaining error is exactly this "missing field `credentials`" shape, and note the count in your report.

- [ ] **Step 6: Commit**

```bash
git add src/node.rs
git commit -m "feat: add credentials field to NodeExecutionContext"
```

---

### Task 6: Mechanical migration — add `..Default::default()` to every test-fixture `NodeExecutionContext` literal

**Files:**
- Modify: `src/nodes/code.rs`, `src/nodes/filter.rs`, `src/nodes/if_node.rs`, `src/nodes/manual_trigger.rs`, `src/nodes/merge.rs`, `src/nodes/noop.rs`, `src/nodes/schedule.rs`, `src/nodes/set.rs`, `src/nodes/switch.rs`, `src/nodes/wait.rs`, `src/nodes/webhook.rs`

**Interfaces:**
- Changes: nothing behavioral. Every `NodeExecutionContext { parameters: ..., input_items: ... }` literal in these eleven files' `#[cfg(test)]` modules gains `, ..Default::default()` before the closing `}`, so it compiles against Task 5's now-three-field struct. `credentials` defaults to an empty `HashMap` in every one of these — none of these pre-existing tests exercise credential-consuming behavior, so an empty map is exactly correct and changes no test's outcome.

- [ ] **Step 1: Find every site**

Run: `grep -rn "NodeExecutionContext {" src/nodes/*.rs`
Expected output: roughly 30 matches across the eleven files listed above (exact count may differ slightly from when this plan was written if intervening work touched these files — trust the grep, not this number).

- [ ] **Step 2: Apply the transform to each site**

For every match, the literal currently looks like one of these two shapes:

```rust
// Shape A — multi-line
let ctx = NodeExecutionContext {
    parameters: serde_json::json!({"condition": true}),
    input_items: items(),
};
```

becomes:

```rust
let ctx = NodeExecutionContext {
    parameters: serde_json::json!({"condition": true}),
    input_items: items(),
    ..Default::default()
};
```

```rust
// Shape B — single-line
let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![] };
```

becomes:

```rust
let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![], ..Default::default() };
```

Apply this exact transform — add `..Default::default()` as the last item before the closing brace, changing nothing else about the literal's existing fields — to every site the Step 1 grep found. This is mechanical: every site takes the identical transform regardless of what its `parameters`/`input_items` values are.

- [ ] **Step 3: Run tests**

Run: `cargo test --lib nodes::`
Expected: every pre-existing test across all eleven files still passes, with identical assertions to before — this task changes zero behavior, only makes the code compile against the new field.

- [ ] **Step 4: Run the whole crate**

Run: `cargo build`
Expected: the only remaining compile errors are in `src/engine.rs` (its one production site plus its own test-fixture sites) — that's Task 7's job. Confirm and note the exact remaining error count/locations in your report.

- [ ] **Step 5: Commit**

```bash
git add src/nodes/code.rs src/nodes/filter.rs src/nodes/if_node.rs src/nodes/manual_trigger.rs src/nodes/merge.rs src/nodes/noop.rs src/nodes/schedule.rs src/nodes/set.rs src/nodes/switch.rs src/nodes/wait.rs src/nodes/webhook.rs
git commit -m "chore: migrate node test fixtures to NodeExecutionContext's new credentials field"
```

---

### Task 7: `execute_workflow_seeded` gains a `credentials` parameter

**Files:**
- Modify: `src/engine.rs`

**Interfaces:**
- Changes: `pub async fn execute_workflow_seeded(workflow: &Workflow, registry: &NodeRegistry, trigger_items: Option<Vec<Item>>, credentials: &std::collections::HashMap<uuid::Uuid, serde_json::Value>) -> anyhow::Result<HashMap<String, Vec<Item>>>` — the one real `NodeExecutionContext` construction site inside this function now sets `credentials: credentials.clone()`.
- Changes: `pub async fn execute_workflow(workflow: &Workflow, registry: &NodeRegistry) -> anyhow::Result<HashMap<String, Vec<Item>>>` — UNCHANGED public signature; its one-line body now passes `&std::collections::HashMap::new()` as the new argument. Every one of its 15 existing test callers needs zero changes.

- [ ] **Step 1: Update the wrapper**

```rust
// src/engine.rs
pub async fn execute_workflow(
    workflow: &Workflow,
    registry: &NodeRegistry,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    execute_workflow_seeded(workflow, registry, None, &HashMap::new()).await
}
```

- [ ] **Step 2: Add the parameter to `execute_workflow_seeded` and populate the real construction site**

```rust
// src/engine.rs — signature
pub async fn execute_workflow_seeded(
    workflow: &Workflow,
    registry: &NodeRegistry,
    trigger_items: Option<Vec<Item>>,
    credentials: &HashMap<uuid::Uuid, serde_json::Value>,
) -> anyhow::Result<HashMap<String, Vec<Item>>> {
    // ... existing body unchanged, EXCEPT the one real NodeExecutionContext
    // construction site (the only non-test one in this file):
```

```rust
// src/engine.rs — the one real construction site, inside the loop
let ctx = NodeExecutionContext {
    parameters,
    input_items,
    credentials: credentials.clone(),
};
```

Everything else in the function body (input aggregation, disabled check, the seeding branch, registry lookup, empty-input skip, parameter resolution, the `execute()` match/error-routing, flattening) is unchanged.

- [ ] **Step 3: Fix this file's own test-fixture `NodeExecutionContext` sites**

Apply Task 6's identical mechanical transform (`..Default::default()`) to whichever `NodeExecutionContext { .. }` literals exist in THIS file's own `#[cfg(test)] mod tests` (there should be none besides the one real site already fixed in Step 2 — engine tests construct `Workflow`/`NodeInstance` fixtures and call `execute_workflow`/`execute_workflow_seeded`, they don't typically build `NodeExecutionContext` directly; confirm this by re-running the Step 1 grep from Task 6 scoped to this file and handle any that exist).

- [ ] **Step 4: Update `execute_workflow_seeded`'s three call sites**

```rust
// src/engine.rs — the test at (approximately) line 309:
let outputs = execute_workflow_seeded(&wf, &registry(), Some(seeded_items.clone()), &HashMap::new()).await.unwrap();
```

```rust
// src/triggers.rs — inside fire_schedule
match crate::engine::execute_workflow_seeded(&workflow, &registry, Some(trigger_items), &HashMap::new()).await {
```

```rust
// src/api/webhook.rs — inside handle_webhook
let response_status = match crate::engine::execute_workflow_seeded(
    &workflow,
    &state.registry,
    Some(vec![trigger_item]),
    &HashMap::new(),
)
.await
{
```

(Both production call sites pass an empty map deliberately, per this plan's Global Constraints — wiring real credential resolution into the webhook/schedule paths is explicitly out of this plan's scope, left for a natural small follow-up.)

- [ ] **Step 5: Run tests**

Run: `cargo test --lib engine::tests`
Expected: all pass, including every pre-existing test (they all call the unchanged 2-arg `execute_workflow` wrapper).

- [ ] **Step 6: Run the whole crate**

Run: `cargo build`
Expected: compiles cleanly now — this was the last remaining error class from Tasks 5-6.

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add src/engine.rs src/triggers.rs src/api/webhook.rs
git commit -m "feat: thread credentials through execute_workflow_seeded"
```

---

### Task 8: `resolve_credentials_for_workflow` + wire into manual execution

**Files:**
- Create: `src/credentials.rs`
- Modify: `src/lib.rs`
- Modify: `src/api/workflows.rs`

**Interfaces:**
- Produces: `r8r::credentials::resolve_credentials_for_workflow(storage: &dyn Storage, workflow: &Workflow) -> anyhow::Result<HashMap<Uuid, serde_json::Value>>` — scans every node in `workflow.nodes` for a `parameters.auth.credential_id` string field (the shape this plan's `core.httpRequest` node uses; a node with no such field is simply skipped), collects the distinct set of referenced credential ids, calls `storage.get_credential` once per unique id (returning `Err` if any referenced credential doesn't exist — a workflow referencing a deleted/nonexistent credential should fail loudly at execute time, not silently proceed with no auth), and returns a map of `id -> decrypted data`.
- Changes: `src/api/workflows.rs`'s `execute_workflow` handler — instead of calling the 2-arg `engine::execute_workflow`, it now calls `resolve_credentials_for_workflow` first, then `engine::execute_workflow_seeded(&workflow, &state.registry, None, &credentials)`.

- [ ] **Step 1: Write failing tests**

```rust
// src/credentials.rs
use crate::domain::Workflow;
use crate::storage::Storage;
use std::collections::HashMap;
use uuid::Uuid;

pub async fn resolve_credentials_for_workflow(
    storage: &dyn Storage,
    workflow: &Workflow,
) -> anyhow::Result<HashMap<Uuid, serde_json::Value>> {
    let mut ids = std::collections::HashSet::new();
    for node in &workflow.nodes {
        if let Some(id_str) = node.parameters.get("auth").and_then(|a| a.get("credential_id")).and_then(|v| v.as_str()) {
            let id = Uuid::parse_str(id_str)
                .map_err(|e| anyhow::anyhow!("node {} has an invalid credential_id: {e}", node.id))?;
            ids.insert(id);
        }
    }

    let mut resolved = HashMap::new();
    for id in ids {
        let credential = storage
            .get_credential(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("referenced credential {id} does not exist"))?;
        resolved.insert(id, credential.data);
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Credential, NodeInstance, User, UserRole};
    use crate::storage::sqlite::SqliteStorage;
    use chrono::Utc;

    async fn test_storage() -> SqliteStorage {
        SqliteStorage::new("sqlite::memory:", [0u8; 32]).await.unwrap()
    }

    async fn create_test_user(storage: &SqliteStorage, id: Uuid) {
        let user = User {
            id,
            email: format!("{id}@example.com"),
            password_hash: "irrelevant-for-this-test".into(),
            role: UserRole::Owner,
            created_at: Utc::now(),
        };
        storage.create_user(&user).await.unwrap();
    }

    fn node_with_credential(id: Uuid) -> NodeInstance {
        NodeInstance {
            id: "http1".into(),
            node_type: "core.httpRequest".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({"auth": {"type": "bearer", "credential_id": id.to_string()}}),
            disabled: false,
        }
    }

    fn workflow_with_nodes(nodes: Vec<NodeInstance>) -> Workflow {
        let now = Utc::now();
        Workflow { id: Uuid::new_v4(), name: "wf".into(), active: false, nodes, connections: vec![], created_at: now, updated_at: now }
    }

    #[tokio::test]
    async fn resolves_a_referenced_credential() {
        let storage = test_storage().await;
        let owner_id = Uuid::new_v4();
        create_test_user(&storage, owner_id).await;

        let cred = Credential {
            id: Uuid::new_v4(),
            name: "c1".into(),
            credential_type: "bearer".into(),
            data: serde_json::json!({"token": "secret-value"}),
            owner_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        storage.create_credential(&cred).await.unwrap();

        let wf = workflow_with_nodes(vec![node_with_credential(cred.id)]);
        let resolved = resolve_credentials_for_workflow(&storage, &wf).await.unwrap();
        assert_eq!(resolved.get(&cred.id), Some(&serde_json::json!({"token": "secret-value"})));
    }

    #[tokio::test]
    async fn workflow_with_no_credential_references_resolves_to_empty_map() {
        let storage = test_storage().await;
        let wf = workflow_with_nodes(vec![NodeInstance {
            id: "set1".into(),
            node_type: "core.set".into(),
            position: (0.0, 0.0),
            parameters: serde_json::json!({}),
            disabled: false,
        }]);
        let resolved = resolve_credentials_for_workflow(&storage, &wf).await.unwrap();
        assert!(resolved.is_empty());
    }

    #[tokio::test]
    async fn referencing_a_nonexistent_credential_returns_error() {
        let storage = test_storage().await;
        let wf = workflow_with_nodes(vec![node_with_credential(Uuid::new_v4())]);
        let result = resolve_credentials_for_workflow(&storage, &wf).await;
        assert!(result.is_err());
    }
}
```

- [ ] **Step 2: Run, confirm compile failure**

Run: `cargo test --lib credentials::tests`
Expected: compile error — `pub mod credentials;` not yet in `src/lib.rs`.

- [ ] **Step 3: Wire into `src/lib.rs`**

Add `pub mod credentials;` alongside the existing `pub mod` lines.

- [ ] **Step 4: Run tests**

Run: `cargo test --lib credentials::tests`
Expected: all three PASS.

- [ ] **Step 5: Wire into the manual-execute handler**

```rust
// src/api/workflows.rs — inside execute_workflow, replace the engine call
let credentials = match crate::credentials::resolve_credentials_for_workflow(state.storage.as_ref(), &workflow).await {
    Ok(c) => c,
    Err(e) => {
        tracing::warn!(error = %e, workflow_id = %workflow.id, "failed to resolve workflow credentials");
        return (StatusCode::BAD_REQUEST, format!("credential resolution failed: {e}")).into_response();
    }
};

match crate::engine::execute_workflow_seeded(&workflow, &state.registry, None, &credentials).await {
    // ... rest of the match arms unchanged from the existing engine::execute_workflow call ...
```

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: all pass, including every pre-existing `tests/api_test.rs` test that calls `POST /rest/workflows/:id/execute` on a workflow with no credential references (they hit the new `resolve_credentials_for_workflow` code path, which correctly resolves to an empty map for them and changes nothing about their existing assertions).

- [ ] **Step 7: Commit**

```bash
git add src/credentials.rs src/lib.rs src/api/workflows.rs
git commit -m "feat: resolve and inject credentials for manual workflow execution"
```

---

### Task 9: HTTP Request node

**Files:**
- Create: `src/nodes/http_request.rs`
- Modify: `src/nodes/mod.rs`
- Modify: `Cargo.toml` (add `reqwest`)

**Interfaces:**
- Produces: `r8r::nodes::http_request::HttpRequestNode`, `type_name() == "core.httpRequest"`. Parameters per Global Constraints. Single output port (index 0) on success. On a non-2xx response, or a request-level failure (DNS/connect/timeout), returns `Err(NodeError::ExecutionFailed(..))`.

- [ ] **Step 1: Add the dependency**

```toml
# Cargo.toml, in [dependencies]
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
```

```toml
# Cargo.toml, in [dev-dependencies]
wiremock = "0.6"
```

- [ ] **Step 2: Write failing tests**

```rust
// src/nodes/http_request.rs
use crate::domain::Item;
use crate::node::{Node, NodeError, NodeExecutionContext, NodeOutput};
use async_trait::async_trait;
use uuid::Uuid;

pub struct HttpRequestNode;

#[async_trait]
impl Node for HttpRequestNode {
    fn type_name(&self) -> &'static str {
        "core.httpRequest"
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError> {
        let method = ctx.parameters.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let url = ctx
            .parameters
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| NodeError::ExecutionFailed("core.httpRequest requires a \"url\" parameter".into()))?;

        let client = reqwest::Client::new();
        let method: reqwest::Method = method
            .parse()
            .map_err(|_| NodeError::ExecutionFailed(format!("invalid HTTP method: {method}")))?;
        let mut request = client.request(method, url);

        if let Some(headers) = ctx.parameters.get("headers").and_then(|v| v.as_object()) {
            for (k, v) in headers {
                if let Some(v_str) = v.as_str() {
                    request = request.header(k, v_str);
                }
            }
        }
        if let Some(query) = ctx.parameters.get("query").and_then(|v| v.as_object()) {
            let pairs: Vec<(String, String)> = query
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect();
            request = request.query(&pairs);
        }
        if let Some(body) = ctx.parameters.get("body") {
            if !body.is_null() {
                request = request.json(body);
            }
        }

        request = apply_auth(request, &ctx.parameters, &ctx.credentials)?;

        let response = request
            .send()
            .await
            .map_err(|e| NodeError::ExecutionFailed(format!("request failed: {e}")))?;

        let status = response.status();
        let response_json: serde_json::Value = response
            .json()
            .await
            .unwrap_or(serde_json::Value::Null);

        if !status.is_success() {
            return Err(NodeError::ExecutionFailed(format!(
                "HTTP {status}: {response_json}"
            )));
        }

        Ok(vec![vec![Item { json: response_json, binary: serde_json::json!({}) }]])
    }
}

fn apply_auth(
    mut request: reqwest::RequestBuilder,
    parameters: &serde_json::Value,
    credentials: &std::collections::HashMap<Uuid, serde_json::Value>,
) -> Result<reqwest::RequestBuilder, NodeError> {
    let auth = match parameters.get("auth") {
        Some(a) => a,
        None => return Ok(request),
    };
    let auth_type = auth.get("type").and_then(|v| v.as_str()).unwrap_or("none");
    if auth_type == "none" {
        return Ok(request);
    }

    let credential_id_str = auth
        .get("credential_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| NodeError::ExecutionFailed(format!("auth.type \"{auth_type}\" requires a credential_id")))?;
    let credential_id = Uuid::parse_str(credential_id_str)
        .map_err(|e| NodeError::ExecutionFailed(format!("invalid credential_id: {e}")))?;
    let data = credentials
        .get(&credential_id)
        .ok_or_else(|| NodeError::ExecutionFailed(format!("credential {credential_id} was not resolved for this run")))?;

    match auth_type {
        "bearer" => {
            let token = data
                .get("token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("bearer credential missing \"token\"".into()))?;
            request = request.bearer_auth(token);
        }
        "apiKey" => {
            let header_name = data
                .get("header_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("apiKey credential missing \"header_name\"".into()))?;
            let value = data
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("apiKey credential missing \"value\"".into()))?;
            request = request.header(header_name, value);
        }
        "basic" => {
            let username = data
                .get("username")
                .and_then(|v| v.as_str())
                .ok_or_else(|| NodeError::ExecutionFailed("basic credential missing \"username\"".into()))?;
            let password = data.get("password").and_then(|v| v.as_str());
            request = request.basic_auth(username, password);
        }
        other => {
            return Err(NodeError::ExecutionFailed(format!("unknown auth.type: {other}")));
        }
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn get_request_returns_json_body_as_item() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/data"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"method": "GET", "url": format!("{}/data", server.uri())}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"ok": true}));
    }

    #[tokio::test]
    async fn bearer_auth_sends_authorization_header_from_resolved_credential() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secure"))
            .and(header("authorization", "Bearer secret-token-xyz"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"authed": true})))
            .mount(&server)
            .await;

        let credential_id = Uuid::new_v4();
        let mut credentials = std::collections::HashMap::new();
        credentials.insert(credential_id, serde_json::json!({"token": "secret-token-xyz"}));

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": format!("{}/secure", server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id.to_string()}
            }),
            input_items: vec![],
            credentials,
        };
        let result = node.execute(&ctx).await.unwrap();
        assert_eq!(result[0][0].json, serde_json::json!({"authed": true}));
    }

    #[tokio::test]
    async fn non_2xx_response_returns_execution_failed_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/broken"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({"method": "GET", "url": format!("{}/broken", server.uri())}),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn missing_url_returns_error() {
        let node = HttpRequestNode;
        let ctx = NodeExecutionContext { parameters: serde_json::json!({}), input_items: vec![], ..Default::default() };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }

    #[tokio::test]
    async fn auth_referencing_unresolved_credential_returns_error() {
        // credential_id points at something never put into ctx.credentials --
        // must fail loudly, not silently send an unauthenticated request.
        let node = HttpRequestNode;
        let ctx = NodeExecutionContext {
            parameters: serde_json::json!({
                "method": "GET",
                "url": "http://example.invalid/",
                "auth": {"type": "bearer", "credential_id": Uuid::new_v4().to_string()}
            }),
            input_items: vec![],
            ..Default::default()
        };
        let result = node.execute(&ctx).await;
        assert!(matches!(result, Err(NodeError::ExecutionFailed(_))));
    }
}
```

- [ ] **Step 3: Run, confirm compile failure**

Run: `cargo test --lib nodes::http_request`
Expected: compile error (module not wired into `mod.rs` yet, `reqwest`/`wiremock` not yet dependencies until Step 1's edits land).

- [ ] **Step 4: Wire into `src/nodes/mod.rs`**

```rust
pub mod http_request;
```

In `register_all`, add:

```rust
registry.register(Box::new(http_request::HttpRequestNode));
```

- [ ] **Step 5: Run tests, adapting to the installed `reqwest`/`wiremock` APIs as needed**

Run: `cargo build` first to surface any API mismatches (both crates are widely-used and stable, but this plan's author could not check live docs while writing this — treat the request-building chain and `wiremock`'s matcher/mount API as a light reference, adapt method names if the installed versions differ), then:

Run: `cargo test --lib nodes::http_request`
Expected: all five PASS.

- [ ] **Step 6: Run the whole crate**

Run: `cargo test`
Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/nodes/http_request.rs src/nodes/mod.rs
git commit -m "feat: add HTTP Request node (core.httpRequest)"
```

---

### Task 10: End-to-end integration test — credential-authenticated HTTP request through the real API

**Files:**
- Modify: `tests/api_test.rs`
- Modify: `Cargo.toml` (`wiremock` needs to be available to the integration-test binary too, not just unit tests — confirm whether `[dev-dependencies]` already covers `tests/*.rs`, which it does by default in Cargo; no change needed if Task 9's `Cargo.toml` edit already added it there)

**Interfaces:**
- Consumes: everything above, plus the existing `test_app()`/`register_and_get_token()` helpers.
- Produces: one new test proving the whole plan works end-to-end: create a credential via `POST /rest/credentials`, create a workflow (Manual Trigger → HTTP Request node referencing that credential via `auth.credential_id`, targeting a `wiremock` server that asserts the `Authorization` header it receives), execute the workflow via `POST /rest/workflows/:id/execute`, assert the execution succeeded and the HTTP Request node's output reflects the mocked response.

- [ ] **Step 1: Write the failing test**

```rust
// add to tests/api_test.rs
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn credential_authenticated_http_request_executes_end_to_end() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/orders"))
        .and(header("authorization", "Bearer e2e-secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"orders": [1, 2, 3]})))
        .mount(&mock_server)
        .await;

    let app = test_app().await;
    let token = register_and_get_token(&app, "http-e2e@example.com").await;

    let cred_response = app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rest/credentials")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(
                    serde_json::json!({"name": "orders-api", "credential_type": "bearer", "data": {"token": "e2e-secret-token"}}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cred_response.status(), StatusCode::CREATED);
    let bytes = cred_response.into_body().collect().await.unwrap().to_bytes();
    let credential: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let credential_id = credential["id"].as_str().unwrap();

    let workflow_body = serde_json::json!({
        "name": "http-e2e-wf",
        "nodes": [
            {"id": "trigger", "node_type": "core.manualTrigger", "position": [0.0, 0.0], "parameters": {}, "disabled": false},
            {"id": "http1", "node_type": "core.httpRequest", "position": [1.0, 0.0], "parameters": {
                "method": "GET",
                "url": format!("{}/orders", mock_server.uri()),
                "auth": {"type": "bearer", "credential_id": credential_id}
            }, "disabled": false}
        ],
        "connections": [
            {"from_node": "trigger", "from_output": 0, "to_node": "http1", "to_input": 0}
        ]
    });
    let wf_response = app.clone()
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
    assert_eq!(wf_response.status(), StatusCode::CREATED);
    let bytes = wf_response.into_body().collect().await.unwrap().to_bytes();
    let workflow: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let workflow_id = workflow["id"].as_str().unwrap();

    let exec_response = app
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
    assert_eq!(exec_response.status(), StatusCode::OK);
    let bytes = exec_response.into_body().collect().await.unwrap().to_bytes();
    let execution: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(execution["status"], "Success");
    assert_eq!(execution["node_outputs"]["http1"][0]["json"], serde_json::json!({"orders": [1, 2, 3]}));
}
```

- [ ] **Step 2: Run, confirm it passes**

Run: `cargo test --test api_test credential_authenticated_http_request_executes_end_to_end`
Expected: PASS if Tasks 1-9 are correctly implemented and wired together (no new production code should be needed for this task — it is purely an integration-proof test). If it fails, the failure identifies exactly which earlier task's implementation has a bug; fix that task's code, not this test — the mock server's `header("authorization", "Bearer e2e-secret-token")` matcher is exactly the assertion that proves the credential actually flowed end-to-end (resolved from storage, decrypted, applied as a Bearer header) rather than the request merely succeeding for an unrelated reason.

- [ ] **Step 3: Run the full suite**

Run: `cargo test`
Expected: all tests across the whole crate PASS.

- [ ] **Step 4: Commit**

```bash
git add tests/api_test.rs
git commit -m "test: add end-to-end credential-authenticated HTTP request integration test"
```

---

## Explicitly Out of Scope (this plan)

Carried forward as future work per the roadmap breakdown:
- Telegram Trigger and Telegram (action node) — roadmap §4.3, explicitly deferred; both depend on this plan's Credential + HTTP Request foundation and are described in the spec as "a thin wrapper over the HTTP Request pattern."
- Credential resolution wired into the webhook and schedule execution paths (`src/api/webhook.rs`, `src/triggers.rs`) — both pass an empty credentials map in this plan; a workflow whose webhook/schedule-triggered path uses an HTTP Request node with real auth won't have it resolved until a small follow-up wires `resolve_credentials_for_workflow` into those two call sites the same way Task 8 wired it into manual execution.
- `PUT`/`DELETE /rest/credentials/:id` (update/delete) and any `GET /rest/credentials/:id` — only create+list exist in this plan; nothing in the spec requires more for v1, and omitting a single-credential fetch endpoint is the safer default given the whole point of encryption-at-rest.
- Per-owner credential visibility/ACL — list returns every credential to every authenticated user, matching the existing workflow-listing endpoint's same shared-workspace v1 model (spec §8).
- Binary/non-JSON HTTP response bodies (file downloads, images) — the HTTP Request node only handles JSON (or JSON-parse-failure-as-null) response bodies in this plan.
- AI Agent tool integration with HTTP Request/Credentials — Plan 5's job.
