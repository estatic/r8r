# r8r Node-Type Metadata System — Design Spec

## 1. Summary

`/rest/node-types` currently returns a bare `Vec<&'static str>` of type
names (`src/api/node_types.rs`), and every frontend surface that lists,
renders, or configures a node is built against that void: the canvas
labels nodes as `${id}\n${node_type}` (raw UUID + dotted type string),
the add-node menu is a flat searchable list of the same strings, and the
credential picker is a free-text type field plus a raw-JSON data
textarea with no relationship to the node being configured.

This spec adds a small, compiler-enforced metadata surface to the `Node`
trait — display name, icon, category, description, accepted credential
type(s), and output port labels — and threads it through a richer
`/rest/node-types` response and one new endpoint, into the canvas, the
add-node menu, and the credential picker. It is the first of six
sub-projects identified from a round of hands-on UI review (see
`docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md` for the
others: connection arrows, multi-port canvas wiring, structured
credential forms, per-node retry/timeout, and AI Agent UX) — chosen
first because most of the others build on the metadata this establishes.

## 2. Goals / Non-Goals

**Goals:**
- Every node type declares a human-readable `display_name`, an `icon`
  (emoji), a `category`, and a one-sentence `description` — compiler
  enforced, so a new node type cannot ship without them (matching this
  project's existing `type_name()` discipline).
- A node type declares which credential type string(s) it accepts (or
  none). The credential picker uses this to filter/default its dropdown
  for a given node, without changing backend credential validation.
- A node type's actual *output port list* is computable for a given node
  *instance* (not just the type in the abstract) — critical for
  `core.switch`, whose port count is `cases.len() + 1`, data the type
  alone doesn't have. `core.if` stays fixed at `["true", "false"]`
  regardless of instance data.
- The canvas renders icon + display name instead of id + raw type
  string, and (groundwork for the separate multi-port sub-project) has
  a live, per-instance-accurate port list available to render from.
- The add-node menu shows icon, name, and description, still searchable.

**Non-Goals:**
- Rendering multiple output handles on the canvas, or exposing
  `ERROR_OUTPUT` as a connectable port — that's the next sub-project
  ("multi-port canvas + error routing"), which *consumes* the
  `output_ports` this spec produces but doesn't implement the
  handle-rendering itself.
- Connection arrows — trivial and independent, tracked as its own quick
  bounded fix, not bundled here.
- Any change to the raw-JSON "Parameters" textarea, or a parameter
  field/form schema — explicitly deferred per user decision during
  brainstorming; this pass covers node *identity* and *port* metadata
  only, not parameter *content* schema.
- Backend enforcement of credential-type matching (rejecting a
  mismatched `credential_id` at execution time). The picker's filtering
  is a UI convenience; `apply_auth` and friends are unchanged.
- Input ports beyond the existing implicit single, unlabeled port. The
  engine has no real multi-input-port concept today — `to_input` is
  never branched on; every incoming connection's items are flattened
  into one `input_items` list regardless of its declared `to_input`
  index (confirmed by reading `execute_workflow_seeded` in
  `src/engine.rs`). `core.merge`'s "combine multiple things" behavior
  already works purely from multiple connections feeding that single
  list — not from a port distinction. Nothing here changes that.

## 3. `Node` Trait Additions

`src/node.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NodeCategory {
    Trigger,
    Action,
    FlowControl,
    Ai,
}

#[async_trait]
pub trait Node: Send + Sync {
    fn type_name(&self) -> &'static str;
    fn resolves_parameters(&self) -> bool { true }

    /// Human-readable name shown in the canvas and add-node menu, e.g.
    /// "Telegram Trigger" for `telegram.trigger`. No default: every node
    /// type must declare one.
    fn display_name(&self) -> &'static str;

    /// One sentence describing what the node does, shown in the add-node
    /// menu and (later) the config panel. No default.
    fn description(&self) -> &'static str;

    /// Groups nodes in the add-node menu and drives canvas styling. No
    /// default.
    fn category(&self) -> NodeCategory;

    /// A single emoji shown next to the node's name. Defaults to a
    /// generic gear so a node type that hasn't picked one yet still
    /// renders something, though every node in this migration gets a
    /// deliberate one (see §6).
    fn icon(&self) -> &'static str { "⚙️" }

    /// Credential type string(s) this node's `auth.credential_id`
    /// accepts, e.g. `&["telegramApi"]`. Empty (the default) means "no
    /// credential" for nodes with no `auth` parameter at all, OR "any
    /// credential" for a node like `core.httpRequest` that accepts
    /// whatever shape its own `auth.type` parameter calls for — the
    /// credential picker treats an empty list as "don't filter" rather
    /// than "don't allow", since the two cases aren't distinguishable
    /// from this list alone and neither needs to be for the picker's
    /// purpose (soft UI filtering, not validation).
    fn credential_types(&self) -> &'static [&'static str] { &[] }

    /// This node instance's current output ports, labeled. Takes
    /// `parameters` because a port *count* can be data-dependent
    /// (`core.switch`); most node types ignore the argument and return a
    /// fixed list. Defaults to a single `"main"` port, correct for the
    /// majority of node types without any override.
    fn output_ports(&self, parameters: &serde_json::Value) -> Vec<String> {
        vec!["main".to_string()]
    }

    async fn execute(&self, ctx: &NodeExecutionContext) -> Result<NodeOutput, NodeError>;
}
```

`core.if` overrides `output_ports` to always return
`vec!["true".into(), "false".into()]`, ignoring `parameters`.

`core.switch` overrides it to compute from its own `cases` parameter,
mirroring `execute()`'s own reading of that field:

```rust
fn output_ports(&self, parameters: &serde_json::Value) -> Vec<String> {
    let case_count = parameters.get("cases").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    (0..case_count).map(|i| format!("case {i}")).chain(std::iter::once("default".to_string())).collect()
}
```

Every other node type keeps the trait's default `output_ports` — zero
change to their `execute()` logic or existing tests, since this is a
new, independent method.

## 4. Metadata Per Node Type

| `type_name` | `display_name` | `icon` | `category` | `credential_types` |
|---|---|---|---|---|
| `core.manualTrigger` | Manual Trigger | 🖱️ | Trigger | — |
| `core.webhook` | Webhook | 🌍 | Trigger | — |
| `core.schedule` | Schedule | ⏰ | Trigger | — |
| `telegram.trigger` | Telegram Trigger | 📨 | Trigger | `telegramApi` |
| `core.set` | Set | 📝 | Action | — |
| `core.code` | Code | 💻 | Action | — |
| `core.httpRequest` | HTTP Request | 🌐 | Action | — (accepts any, see §3) |
| `core.wait` | Wait | ⏳ | Action | — |
| `core.noop` | No-Op | ⚪ | Action | — |
| `telegram.sendMessage` | Send Telegram Message | 📤 | Action | `telegramApi` |
| `core.if` | If | ❓ | FlowControl | — |
| `core.switch` | Switch | 🔀 | FlowControl | — |
| `core.merge` | Merge | 🔗 | FlowControl | — |
| `core.filter` | Filter | 🚦 | FlowControl | — |
| `ai.agent` | AI Agent | 🤖 | Ai | `anthropicApi`, `openaiApi` |

Descriptions are one sentence each, written per node at implementation
time from its existing doc comments (every node file already has a
short comment describing its behavior — this is a transcription, not
new research).

`docs/adding-a-node.md` (the existing "how to add a node type" guide)
gets a new step for these five methods, so the checklist stays the
single source of truth for what a new node must implement.

## 5. API Changes

**`GET /rest/node-types`** — response shape changes from `string[]` to:

```json
[
  {
    "type_name": "telegram.trigger",
    "display_name": "Telegram Trigger",
    "icon": "📨",
    "category": "trigger",
    "description": "Fires when a message arrives for the configured Telegram bot.",
    "credential_types": ["telegramApi"],
    "output_ports": ["main"]
  }
]
```

`output_ports` here is each type's *default* (computed by calling
`output_ports(&serde_json::json!({}))`) — correct for every type except
`core.switch` when used with the empty-parameters default (0 cases → a
single `"default"` port), which is a reasonable placeholder for the
add-node menu (a freshly-added switch node has no cases yet anyway).

**New endpoint `POST /rest/node-types/:type_name/output-ports`** — body
`{"parameters": <the node instance's current parameters object>}`,
response `{"output_ports": ["case 0", "case 1", "default"]}`. Looks up
the type in `NodeRegistry`, calls `.output_ports(&parameters)`, returns
the result. This is the mechanism that makes `core.switch`'s port list
accurate for a *specific* node instance without any node-type-specific
logic in the frontend — the frontend never needs to know which types are
data-dependent, it can call this uniformly.

Both endpoints require auth (existing `AuthUser` extractor, matching
`list_node_types`'s current behavior).

## 6. Frontend Changes

- `frontend/src/stores/nodeTypes.ts` — `types: string[]` becomes
  `types: NodeTypeMeta[]` (the shape from §5); add a `portsFor(type_name,
  parameters)` action calling the new endpoint, with a simple
  in-memory cache keyed on `(type_name, JSON.stringify(parameters))` so
  repeated calls for an unchanged node don't re-hit the network.
- `AddNodeMenu.vue` — renders `icon + display_name` as the primary line
  and `description` as a secondary line per entry; search matches
  against both `display_name` and `type_name` (so muscle memory typing
  `telegram.` still works).
- `WorkflowCanvas.vue` — node label becomes `icon + display_name`
  instead of `${id}\n${node_type}`; the raw `node_type` moves to a
  `title` attribute (hover tooltip) for anyone who wants it. Handle
  rendering itself (multiple source handles) is explicitly out of scope
  here (§2) — this pass only makes the *data* available via
  `portsFor`, consumed by the next sub-project.
- `CredentialPicker.vue` — gains a `nodeType: string` prop. Looks up
  that type's `credential_types` from the store; if non-empty, the
  "+ New credential" Type field becomes a `<select>` of those options
  (pre-selected if there's exactly one) instead of free text; if empty,
  the field stays free text exactly as today. The existing dropdown of
  already-created credentials is unfiltered by this list — showing all
  existing credentials and only restricting what a *new* one defaults
  to keeps a user from being blocked if they, e.g., already created a
  credential under a slightly different type string before this change
  shipped.

## 7. Error Handling

- An unknown `type_name` on the new `output-ports` endpoint returns 404
  (matches this project's existing not-found convention elsewhere, e.g.
  `get_workflow`).
- A malformed `parameters` body (not an object) is treated the same way
  `execute()` already treats missing/malformed parameters for that node
  type — `output_ports` implementations only ever call `.get()`/`.as_*()`
  with `unwrap_or` fallbacks (see `core.switch`'s implementation in §3),
  never panicking on unexpected shapes.

## 8. Testing

- Per-node unit tests (colocated in each node's existing `#[cfg(test)]`
  module) asserting `display_name()`, `category()`, and default
  `output_ports()` — a single new test per file for the 13 nodes using
  the trait default, plus dedicated tests for `core.if`'s fixed
  `["true", "false"]` and `core.switch`'s case-count-dependent list
  (empty cases → `["default"]`; 2 cases → `["case 0", "case 1",
  "default"]`).
- `src/api/node_types.rs`: integration test confirming the enriched
  `/rest/node-types` response shape.
- New integration test for `POST /rest/node-types/:type_name/output-ports`
  in `tests/api_test.rs`: a `core.switch` request with 3 cases returns 4
  ports; an unknown type returns 404.
- Frontend: `AddNodeMenu.spec.ts` updated for the new metadata shape;
  new assertions in `WorkflowCanvas.spec.ts` for the icon+name label; a
  new `CredentialPicker.spec.ts` (none exists today) covering the
  type-dropdown-vs-free-text branching for a node with and without
  declared `credential_types`.

## 9. Out of Scope / Deferred

- Multi-port canvas rendering (next sub-project).
- Connection arrows (separate quick bounded fix).
- Structured credential data forms — the "+ New credential" Data field
  stays raw JSON; only the Type field changes in this pass.
- Parameter field schema / form generation for the "Parameters (JSON)"
  textarea.
- Backend-side credential-type validation at execution time.
- Any change to `NodeInstance`, `Connection`, or stored workflow data —
  this is purely additive metadata on the `Node` trait and API
  responses; no migration needed.
