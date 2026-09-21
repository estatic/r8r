# r8r Structured Credential Forms (Plan 8.4) — Design Spec

## 1. Summary

`CredentialPicker.vue`'s "+ New credential" form has a free-text Type
field and a raw-JSON Data textarea, with no relationship between the two
— a user must already know (from documentation or trial and error) that
a `telegramApi` credential needs `{"bot_token": "..."}` and an
`anthropicApi` one needs `{"api_key": "..."}`. This spec adds a small
backend-declared schema per known credential type and uses it to render
real input fields instead of raw JSON, for both the node-restricted
types from Plan 8.2 (`telegramApi`, `anthropicApi`, `openaiApi`) and
`core.httpRequest`'s three `auth.type` shapes, given new standardized
type names (`bearerToken`, `apiKeyHeader`, `basicAuth`) since that node's
accepted shape is chosen by its own `auth.type` parameter, not by
whatever free-text `credential_type` string a credential happens to
carry.

## 2. Goals / Non-Goals

**Goals:**
- Every currently-consumed credential shape (`telegramApi`,
  `anthropicApi`, `openaiApi`, plus `core.httpRequest`'s bearer/apiKey/
  basic) has a declared field schema: name, human label, whether it's a
  secret (rendered masked), whether it's required.
- The "+ New credential" form renders real input fields instead of raw
  JSON whenever the selected type matches a known schema; unrecognized/
  custom types keep today's raw-JSON textarea as an escape hatch.
- `core.httpRequest`, which declares no fixed `credential_types` (Plan
  8.2), gets a Type dropdown offering the three generic shapes plus
  "Custom…" for anything else — replacing today's immediate fallback to
  free text.
- No runtime/backend node behavior changes anywhere — `apply_auth`,
  `telegram_send_message.rs`, `agent.rs` already just read whatever's in
  a credential's `data`, regardless of what `credential_type` string was
  used to create it. This spec only changes how that `data` gets
  authored in the UI.

**Non-Goals:**
- Editing an already-created credential — confirmed no such capability
  exists anywhere in the app today (create/list/select only). Nothing
  here needs to handle "existing data doesn't match the new schema."
- Backend validation of credential data against its schema at creation
  or execution time — this is a UI authoring aid, not enforcement (same
  posture as Plan 8.2's credential-type filtering).
- Any change to `core.httpRequest`'s `apply_auth` logic, or to any other
  node's credential-reading code — the six schemas below are chosen to
  exactly match what already exists, not to change it.
- A schema for credential types not currently consumed by any node
  (there are none beyond the six below as of this plan).

## 3. Credential-Type Schema Registry

New file `src/credential_types.rs`:

```rust
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldType {
    Text,
    Password,
}

#[derive(serde::Serialize)]
pub struct CredentialField {
    pub name: &'static str,
    pub label: &'static str,
    pub field_type: FieldType,
    pub required: bool,
}

#[derive(serde::Serialize)]
pub struct CredentialTypeSchema {
    pub credential_type: &'static str,
    pub display_name: &'static str,
    pub fields: &'static [CredentialField],
}

pub fn known_credential_types() -> &'static [CredentialTypeSchema] {
    &[
        CredentialTypeSchema {
            credential_type: "telegramApi",
            display_name: "Telegram Bot",
            fields: &[CredentialField { name: "bot_token", label: "Bot Token", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "anthropicApi",
            display_name: "Anthropic API",
            fields: &[CredentialField { name: "api_key", label: "API Key", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "openaiApi",
            display_name: "OpenAI API",
            fields: &[
                CredentialField { name: "api_key", label: "API Key", field_type: FieldType::Password, required: true },
                CredentialField { name: "base_url", label: "Base URL (optional)", field_type: FieldType::Text, required: false },
            ],
        },
        CredentialTypeSchema {
            credential_type: "bearerToken",
            display_name: "Bearer Token",
            fields: &[CredentialField { name: "token", label: "Token", field_type: FieldType::Password, required: true }],
        },
        CredentialTypeSchema {
            credential_type: "apiKeyHeader",
            display_name: "API Key (Header)",
            fields: &[
                CredentialField { name: "header_name", label: "Header Name", field_type: FieldType::Text, required: true },
                CredentialField { name: "value", label: "Value", field_type: FieldType::Password, required: true },
            ],
        },
        CredentialTypeSchema {
            credential_type: "basicAuth",
            display_name: "Basic Auth",
            fields: &[
                CredentialField { name: "username", label: "Username", field_type: FieldType::Text, required: true },
                CredentialField { name: "password", label: "Password (optional)", field_type: FieldType::Password, required: false },
            ],
        },
    ]
}
```

Field names/requiredness verified against actual current consumers:
`telegramApi.bot_token` (`src/nodes/telegram_send_message.rs`,
`src/telegram_poller.rs`), `anthropicApi.api_key`/`openaiApi.api_key`+
`base_url` (`src/nodes/agent.rs`), and the three generic ones against
`src/nodes/http_request.rs`'s `apply_auth` (`bearer` → `token`
required; `apiKey` → `header_name` + `value` both required; `basic` →
`username` required, `password` optional — matches `apply_auth`'s
`data.get("password").and_then(|v| v.as_str())` returning `Option<&str>`,
passed straight to `request.basic_auth(username, password)`).

## 4. API

New endpoint `GET /rest/credential-types`, `AuthUser`-gated (same
pattern as `list_node_types`), returns `Vec<CredentialTypeSchema>` as
JSON — the full registry, unfiltered (filtering to what a specific node
accepts already happens client-side via Plan 8.2's `credential_types`
metadata, same as today).

## 5. Frontend

`frontend/src/stores/credentialTypes.ts` (new store, mirrors
`nodeTypes.ts`'s shape): `fetchAll()` loads `GET /rest/credential-types`
once into `.types`.

`CredentialPicker.vue`'s "+ New credential" form:

- **Type field**, three cases:
  1. Node declares `credential_types` (Plan 8.2) — dropdown restricted to
     exactly those, unchanged from today.
  2. Node declares none — dropdown offers the three generic schemas
     (`bearerToken`, `apiKeyHeader`, `basicAuth`) plus a **"Custom…"**
     option; choosing "Custom…" reveals today's free-text input in its
     place.
  3. "Custom…" selected, or a typed-in value matches no known schema —
     Data field stays the raw-JSON textarea (today's behavior,
     unconditionally available as a fallback).
- **Data field:** once the selected type matches an entry in
  `credentialTypesStore.types`, render one input per field instead of
  the textarea — `type="password"` for `FieldType::Password`, `type="text"`
  for `FieldType::Text` — client-side-required for fields marked
  `required: true` before the Create button submits. On submit, the
  individual field values are assembled into the same `data: Record<string,
  unknown>` object `store.create()` already accepts — no change to
  `useCredentialsStore().create()`'s signature or the `POST
  /rest/credentials` request shape.

## 6. Testing

- `src/credential_types.rs`: a test per schema entry asserting its exact
  field names/types/requiredness (6 tests, one per type) — cheap and
  catches a typo before it ships, given nothing else in the codebase
  cross-checks this against the real consumers.
- `src/api/*`: integration test confirming `GET /rest/credential-types`
  returns all 6 known types with the right shape.
- Frontend: `CredentialPicker.spec.ts` gains cases for (a) a
  `credential_types`-declaring node (e.g. `telegram.trigger`) rendering
  the `telegramApi` schema's structured field once selected, (b) a
  generic node (`core.httpRequest`) showing the dropdown with
  `bearerToken`/`apiKeyHeader`/`basicAuth`/"Custom…", (c) selecting
  "Custom…" reveals the free-text field, (d) submitting a structured
  form assembles the expected `data` object.

## 7. Out of Scope / Deferred

- Credential editing (§2).
- Backend schema validation (§2).
- Any node-side credential-reading logic changes (§2).
