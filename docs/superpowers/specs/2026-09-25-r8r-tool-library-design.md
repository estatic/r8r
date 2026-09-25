# r8r Tool Library and AI Agent Settings Form (B1) — Design Spec

## 1. Summary

An `ai.agent` node's tools are hand-written JSON inside its own
`parameters.tools` array (Plan 5): each entry repeats `name`,
`description`, `node_type`, `argument_schema` and `base_parameters`, and
the model's arguments are merged over `base_parameters` by key. Nothing
is reusable across agents, arguments can't be placed inside a value (a
URL, a message), and credentials referenced inside a tool are never
resolved for the run. The agent's own settings (`provider`, `model`,
prompts) are also raw JSON with no form.

This spec adds a **tool library**: tools are stored once, referenced by
id from any number of agents (live references), and their fixed
parameters use `{{ $args.<name> }}` templates. It also adds an **AI
Agent settings form**. Workflows-as-tools are a separate follow-up (B2).

## 2. Goals / Non-Goals

**Goals:**
- Define a tool once (HTTP request, Telegram message, or code) and
  attach it to many agents; editing it changes every agent's next run.
- Place the model's arguments anywhere in a tool's parameters via
  `{{ $args.x }}`.
- Credentials referenced by a tool work at run time, in every run mode.
- Configure an agent (provider, model, prompts, iterations, tools) with
  a form instead of JSON.
- Existing workflows with inline `parameters.tools` keep working
  unchanged.

**Non-Goals:** see §9.

## 3. Tool Entity

### 3.1 Domain (`src/domain.rs`)

```rust
pub struct Tool {
    pub id: Uuid,
    pub name: String,                     // what the model calls
    pub description: String,              // what the model reads
    pub node_type: String,                // core.httpRequest | telegram.sendMessage | core.code
    pub argument_schema: serde_json::Value, // JSON Schema object: {type:"object", properties, required}
    pub parameters: serde_json::Value,    // the node's fixed parameters; may contain {{ $args.x }}
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

### 3.2 Validation (`src/tools.rs`, pure; used by create and update)

- `name`: matches `^[A-Za-z0-9_-]{1,64}$` (the character set LLM
  function-calling APIs accept) and is unique (case-sensitive) among
  tools → otherwise 400 / 409.
- `node_type` ∈ `ALLOWED_TOOL_NODE_TYPES = ["core.httpRequest",
  "telegram.sendMessage", "core.code"]` → otherwise 400.
- `argument_schema`: an object whose `type` is `"object"`; each
  `properties.<k>.type` ∈ `string | number | integer | boolean`; every
  `required` entry names a property → otherwise 400.
- `parameters`: a JSON object → otherwise 400.

### 3.3 Storage (migration `migrations/0004_tools.sql`)

```sql
CREATE TABLE tools (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    node_type TEXT NOT NULL,
    argument_schema TEXT NOT NULL,
    parameters TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

`Storage` gains `create_tool`, `get_tool`, `list_tools`, `update_tool`
(→ `bool`), `delete_tool` (→ `bool`). Tool rows hold no secrets (a tool
references a credential by id; the secret stays in `credentials`), so
they are stored as plain JSON text. Test `Storage` wrappers delegate.

## 4. API (`src/api/tools.rs`)

All require `AuthUser`.

- `GET /rest/tools` → list, each item `Tool` + `used_by` (number of
  workflows with an `ai.agent` node whose `parameters.tool_ids`
  contains the tool id).
- `POST /rest/tools` → 201 `Tool`; 400/409 per §3.2.
- `GET /rest/tools/:id` → `Tool` + `used_by`; 404.
- `PATCH /rest/tools/:id` → any subset of `name, description,
  node_type, argument_schema, parameters`; re-validated as a whole;
  200 / 400 / 404 / 409 (name clash).
- `DELETE /rest/tools/:id` → 204; 404; **409** `{ "error": "tool is in
  use", "workflows": [{id, name}] }`.

Info logs: `tool created|updated|deleted` with id and name.

**Credential usage** (`workflows_using_credential` and the credential
API's `used_by` / delete guard) also counts tools: a credential whose
id is a tool's `parameters.auth.credential_id` is in use. The DELETE
409 body for credentials gains `tools: [{id, name}]` alongside
`workflows`; `used_by` counts workflows plus tools.

## 5. Run-Time Resolution

### 5.1 `RunResources` (`src/credentials.rs`)

```rust
pub struct RunResources {
    pub credentials: HashMap<Uuid, serde_json::Value>,
    pub tools: HashMap<Uuid, Tool>,
}
pub async fn resolve_run_resources(storage: &dyn Storage, workflow: &Workflow) -> anyhow::Result<RunResources>
```

Loads (1) every tool id listed in any `ai.agent` node's
`parameters.tool_ids`, (2) every credential referenced by a node's
`parameters.auth.credential_id` **or** by a loaded tool's
`parameters.auth.credential_id`. A missing tool or credential is an
error naming it (same as today's missing-credential error).
`resolve_credentials_for_workflow` is replaced by this.

### 5.2 Threading

`execute_workflow_seeded`, `start_execution` and
`run_and_track_execution` take `RunResources` (owned or `&`, matching
how they take credentials today) instead of the credentials map.
`NodeExecutionContext` gains `tools: HashMap<Uuid, Tool>` (default
empty); the engine gives every node the run's credentials (as today)
and tools.

**All four run paths resolve resources:** manual execute (already
resolves credentials), Telegram poller (already, per batch), and —
new — schedule firings (`fire_schedule`) and webhook calls, which
today pass an empty credential map. A resolution failure there is
logged and the run is not started (schedule) / answered 500 with the
error (webhook), matching the manual path's 400-before-start.

### 5.3 `$args` in the expression engine (`src/expr.rs`)

`EvalContext` gains `args: Option<&serde_json::Value>`, bound as the
global `$args` (data, never code — same binding mechanism as `$json`).
Everywhere else passes `None`, so `$args` is `undefined` outside tool
calls.

### 5.4 Agent tool dispatch (`src/nodes/agent.rs`)

- Tools offered to the model = inline `parameters.tools` (unchanged
  parsing and merge semantics) **plus** each `parameters.tool_ids` entry
  looked up in `ctx.tools`. A duplicate tool name across the two sets is
  an execution error naming it. A `tool_ids` entry missing from
  `ctx.tools` is an error (cannot happen after §5.1 succeeds).
- For a library tool whose node type does **not** resolve its own
  parameters (`core.code`, whose `script` runs verbatim), the parameters
  are passed unchanged and the arguments travel as data: the node binds
  them as the `$args` global (`NodeExecutionContext.tool_args`). Model
  text is never spliced into a script. (Added after the final review,
  which showed templating a script let model input run as code.)
- For a **library** tool call on any other node: validate the arguments against
  `argument_schema` (existing shallow validation, including the
  denied-key list), then compute the node parameters as
  `expr::resolve_parameters(&tool.parameters, &EvalContext { json: {},
  items: [], node_json: {}, workflow_name, args: Some(&arguments) })`.
  Arguments are **not** merged by key. An expression error becomes a
  `ToolResult { is_error: true }` for the model, like any tool failure.
- The called node receives the run's credentials (so a tool's
  `auth.credential_id` resolves) — `EngineToolExecutor` already carries
  them.
- The engine's own parameter resolution of the `ai.agent` node is
  unaffected: `{{ $args }}` templates live in the tool rows, not in the
  agent's parameters.

## 6. Frontend

### 6.1 Tools page (`/tools`, `ToolsView.vue` + `ToolForm.vue`)

- Header links: Workflows · Credentials · Tools (on all three list
  pages).
- List: name, node type, description, "used by N workflows"; New / Edit
  / Delete (confirm; 409 names the workflows).
- `ToolForm`:
  - name (validated live against §3.2's pattern), description
    (textarea, with the hint "The model reads this to decide when to
    call the tool").
  - node type select (HTTP Request, Telegram: Send Message, Code).
  - credential picker (`CredentialPicker` restricted to the node type's
    accepted credential types; HTTP shows the generic types) — stored
    as `parameters.auth.credential_id` (and `auth.type` for HTTP from
    the credential type: `bearerToken`→`bearer`, `apiKeyHeader`→
    `apiKey`, `basicAuth`→`basic`).
  - **Arguments** table: rows of name, type (string / number /
    integer / boolean), description, required; add/remove row.
    Generates `argument_schema`. Loading an existing tool fills the
    table from its schema.
  - **Parameters** JSON editor (everything except `auth`), pre-filled
    per node type when the type is chosen on a new tool:
    - HTTP: `{"method": "GET", "url": "https://api.example.com/search?q={{ encodeURIComponent($args.query) }}"}`
      (encoded so model text can't add query params or path segments —
      changed after the final review)
    - Telegram: `{"chat_id": "", "text": "{{ $args.message }}"}`
    - Code: `{"script": "return [{ json: { result: $args } }]"}`
    with the hint "Use {{ $args.<name> }} to insert an argument."
  - Client-side checks mirror §3.2; the backend 400/409 text is shown
    as returned.

### 6.2 AI Agent settings form (`AgentSettings.vue`, in `NodeConfigPanel` for `ai.agent`)

Fields bound to the node's parameters (the raw JSON textarea remains,
under an "Advanced (JSON)" disclosure, and stays in sync on Apply):

- **Provider**: select `openai` / `anthropic`; defaults from the
  selected credential's type (the rule shipped in the quick fix).
- **Model**: text input (required).
- **System prompt**: textarea.
- **User message**: text input (required), hint "Expressions like
  {{ $json.message.text }} work here."
- **Max iterations**: number, 1–50, default 10.
- **Tools**: checkbox list of library tools (name + description),
  writing `tool_ids`; a "Manage tools" link to `/tools`; if inline
  `parameters.tools` exist, a note "N inline tools (edit in Advanced)".

Apply validates the required fields (model, user message, credential)
before emitting, showing the same style of error as the JSON check.

### 6.3 Stores

`stores/tools.ts`: `fetchAll`, `get`, `create`, `update`, `remove`,
`inUseWorkflowNames` (same 409 shape handling as credentials).

## 7. Security

- `$args` values are bound as JSON data; they cannot inject code into
  the expression engine.
- The model still cannot choose `auth`/credentials/headers/script:
  those are fixed in the tool row, and argument validation keeps the
  denied-key list. A template author who writes
  `url: "{{ $args.url }}"` deliberately gives the model the host — the
  form's hint recommends a fixed scheme and host.
- Tool rows contain no secrets; credentials stay encrypted in
  `credentials` and are referenced by id.
- Tool names are validated to the function-calling charset.

## 8. Testing

**Expression engine:** `$args.x` resolves inside a string template;
`$args` is undefined when `args: None`; an `$args` value containing
`"}}"` or JS source stays literal data.

**Tool validation** (unit): name pattern, allowed node types, schema
shape, required-names-exist.

**Storage** (sqlite): tool CRUD round-trip; unique name enforced;
update/delete missing → false.

**API** (`tests/api_test.rs`): create/list/get/patch/delete; 400 on bad
name/type/schema; 409 on duplicate name; delete refused while an agent
in a workflow references it; credential `used_by` counts a tool and
credential delete 409 lists the tool.

**Run resources** (unit): tool ids collected from agent nodes; tool
credentials resolved; missing tool → error naming it.

**Agent** (unit, with the existing mock provider): a library tool call
resolves `{{ $args.q }}` into the called node's parameters and the node
receives the tool's credential; arguments are not key-merged; a
template error becomes an `is_error` tool result; inline tools still
work; duplicate names error.

**Run paths:** a schedule-fired and a webhook-fired workflow now get
their credentials (API/trigger tests).

**Frontend** (Vitest): ToolForm arguments table ↔ schema round-trip,
per-type parameter template prefill, credential → `auth` mapping;
ToolsView list/delete-409; AgentSettings fields ↔ parameters, provider
default, tools checkboxes → `tool_ids`, required-field validation.

## 9. Out of Scope / Deferred

- Workflows as tools (B2).
- Tool test-run button, per-tool rate limits, tool versioning.
- Migrating existing inline `parameters.tools` into the library
  (they keep working; users can recreate them as library tools).
- Model name suggestions / listing a provider's models.
- Nested-object argument types.
