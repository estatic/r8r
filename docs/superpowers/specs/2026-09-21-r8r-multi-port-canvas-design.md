# r8r Multi-Port Canvas + Error Routing (Plan 8.3) — Design Spec

## 1. Summary

The canvas hardcodes exactly one output handle per node (`Handle id="0"`),
even though the engine has fully supported multi-output nodes since Plan 2
(`core.if` returns `[true_items, false_items]`; `core.switch` returns
`cases.len() + 1` ports) and a working error-output routing mechanism
(`ERROR_OUTPUT`) since the same plan. Neither has ever been reachable from
the UI. This spec makes the canvas render one handle per output port a
node type actually has (computed live, per instance, via the
`output_ports`/`portsFor` mechanism Plan 8.2 built for exactly this), plus
one universal, visually distinct "error" handle on every node, and fixes a
real bug this would otherwise expose: `ERROR_OUTPUT`'s current
representation (`usize::MAX`) cannot survive a JSON round-trip through
JavaScript without corruption.

## 2. Goals / Non-Goals

**Goals:**
- Every node on the canvas renders one source handle per entry in its live
  `output_ports` list (`core.if` → 2 handles labeled "true"/"false";
  `core.switch` → `cases.len() + 1` handles; every other node type → 1
  handle labeled "main", visually unchanged from today).
- Every node also renders one additional, always-present, visually
  distinct "error" handle — connecting from it produces a connection that
  routes that node's execution failures downstream, using the engine's
  existing (but currently UI-unreachable) error-routing mechanism.
- Port labels are visible on the canvas by default (not hover-only) — this
  is the whole point of the sub-project: making branching legible at a
  glance.
- Error-routed edges render in a visually distinct color.
- Fix the `ERROR_OUTPUT`/JSON precision bug (§3) as a prerequisite, not an
  afterthought — this sub-project is what actually exercises the risk.

**Non-Goals:**
- Any change to input ports — still exactly one, unlabeled, per node (the
  engine has no real multi-input concept; unchanged from 8.2's own
  non-goals).
- A visual editor for `core.switch`'s `cases` parameter — cases are still
  edited via the raw JSON "Parameters" textarea; this spec only makes the
  *resulting* ports render and connect correctly once cases exist.
- Retry/timeout/continue-on-fail configuration (Plan 8.5) — the error
  *output port* this spec adds is a routing mechanism (send the error
  item somewhere), not a policy mechanism (retry N times, wait, etc.).
  Unrelated, already-separately-tracked concern.
- Structured credential forms (Plan 8.4) — unrelated.

## 3. Prerequisite Fix: `ERROR_OUTPUT` Representation

**The bug:** `ERROR_OUTPUT` (`src/node.rs`) is `usize::MAX` =
`18446744073709551615`. `serde_json` serializes this as a bare JSON
number. JavaScript's `JSON.parse` represents all numbers as IEEE-754
float64, whose largest exactly-representable integer is
`2^53 - 1 = 9007199254740991` — far below `usize::MAX`. Verified by
direct simulation: `18446744073709551615` parses to the float64
`1.8446744073709552e+19`, which converts back to the integer
`18446744073709551616` — one more than `u64::MAX`, so re-serializing it
and sending it back to the backend would fail to deserialize into a
`usize` field entirely (integer overflow), not merely produce a wrong-but
valid port index. This has never mattered because the UI has never been
able to construct an error-routed connection — exactly what this spec
changes.

**The fix:** `Connection` (`src/domain.rs`) gains a new field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub from_node: String,
    pub from_output: usize,
    pub to_node: String,
    pub to_input: usize,
    #[serde(default)]
    pub error: bool,
}
```

`#[serde(default)]` makes every already-stored workflow (which has no
`error` field in its JSON) deserialize with `error: false` automatically —
no migration needed, since workflows are stored as a JSON blob, not typed
SQL columns. When `error` is `true`, `from_output`'s value is unused by
convention (always `0` at construction sites) — the engine keys off
`error`, not `from_output`, once this change lands.

`Connection` does **not** derive `Default` (no field has a semantically
meaningful default — an empty `from_node`/`to_node` string isn't a
sensible "mostly-default" connection), so every existing `Connection { .. }`
struct literal in the Rust codebase needs the new field added explicitly.
Confirmed via search: **24 literal sites** across `src/domain.rs`,
`src/engine.rs`, `src/execution_runner.rs`, `src/telegram_poller.rs`,
`src/triggers.rs` — 22 become `error: false`, the 2 error-routing test
connections (`src/engine.rs`, currently `from_output:
crate::node::ERROR_OUTPUT`) become `error: true, from_output: 0`. This
mirrors this codebase's own established precedent: `NodeInstance.disabled`
also has `#[serde(default)]` for JSON back-compat while every Rust literal
sets it explicitly — same pattern, not a new one.

`src/engine.rs`'s two routing checks change:
- `if conn.from_output == crate::node::ERROR_OUTPUT` → `if conn.error`
- `c.from_output == crate::node::ERROR_OUTPUT` → `c.error`

`ERROR_OUTPUT` (`src/node.rs`, the constant and its own dedicated test)
is removed — fully superseded, nothing references it once the two sites
above change, and it would otherwise be dead code.

`Connection` (`frontend/src/types/domain.ts`) gains `error: boolean`.

## 4. Port Computation on the Canvas

`WorkflowCanvas.vue` needs each node's **live, per-instance** port list —
critical for `core.switch`, whose count depends on its own `cases`
parameter (data the node *type* alone doesn't have; Plan 8.2 built
`useNodeTypesStore().portsFor(typeName, parameters)` for exactly this,
calling `POST /rest/node-types/:type_name/output-ports`).

`WorkflowCanvas.vue` maintains its own reactive map (node id → port list),
populated by watching `props.nodes` and calling `portsFor` for any node
whose `(node_type, parameters)` pair has changed since the last watch
tick — covers both the initial load (every node fetched once) and any
later edit (`NodeConfigPanel`'s Apply mutates `props.nodes`, which
`WorkflowEditorView` already re-renders through — no new event wiring
needed in `WorkflowEditorView.vue`). This is NOT triggered per-keystroke:
`NodeConfigPanel`'s "Parameters (JSON)" textarea only propagates on
Apply, an explicit user action, matching how it already works today.

Since this is `portsFor`'s first real caller (flagged as pending during
Plan 8.2's final review), two fixes land in `stores/nodeTypes.ts` as part
of this work rather than staying deferred:
- **In-flight de-duplication:** two calls for the same uncached
  `(typeName, parameters)` key currently both hit the network before
  either resolves. Add a `Map<string, Promise<string[]>>` of pending
  requests, checked before issuing a new one.
- **URL encoding:** the type name is currently interpolated directly into
  the request path (`` `/rest/node-types/${typeName}/output-ports` ``);
  wrap it in `encodeURIComponent`.

Cache-size growth (one entry per distinct `(type, JSON.stringify(params))`
pair ever seen) stays an accepted non-issue — bounded in practice by how
many distinct parameter configurations a user actually creates in one
session, not worth the complexity of an eviction policy.

## 5. Canvas Rendering

`WorkflowCanvas.vue`'s `#node-default` slot currently renders exactly one
target `Handle id="0"` and one source `Handle id="0"`. This becomes:

- **Target:** unchanged — one handle, `id="0"`, per the non-goal above.
- **Source, success ports:** one `Handle` per entry in that node's live
  port list from §4, `id` equal to its index (`"0"`, `"1"`, ...),
  positioned evenly spaced along the bottom edge (e.g. `left:
  ${(i + 1) * 100 / (n + 1)}%` for `n` ports), each with a small
  always-visible text label directly beneath it showing that port's name
  (`"true"`, `"false"`, `"case 0"`, `"default"`, `"main"`, ...).
- **Source, error port:** one additional `Handle id="error"`, present on
  every node regardless of type, visually distinct (e.g. a red border
  instead of the gray success-port styling), positioned separately from
  the success-port row (e.g. offset further down/right) so it's never
  confused with a numbered port, labeled "error".

`WorkflowCanvas.vue`'s `onConnect` handler changes from always setting
`from_output: Number(connection.sourceHandle ?? 0)` to:

```typescript
onConnect((connection) => {
  const isError = connection.sourceHandle === 'error'
  emit('connect', {
    from_node: connection.source,
    from_output: isError ? 0 : Number(connection.sourceHandle ?? 0),
    to_node: connection.target,
    to_input: Number(connection.targetHandle ?? 0),
    error: isError,
  })
})
```

`flowEdges`'s existing mapping (`sourceHandle: String(c.from_output)`)
changes to `sourceHandle: c.error ? 'error' : String(c.from_output)`, so a
loaded error connection re-attaches to the `id="error"` handle instead of
a numbered one. Its `markerEnd` styling gains a conditional: error edges
get a distinct stroke color (e.g. red) via VueFlow's per-edge `style`
field, keyed off `c.error`.

## 6. Error Handling

- A node type whose `portsFor` call fails (network error, unknown type)
  falls back to a single `"main"` port — the same graceful-degradation
  pattern already used for a node's icon/display name when metadata
  hasn't loaded (Plan 8.2). Never blocks rendering the node itself.
- No backend validation is added for a connection's `error` field beyond
  what already exists for `from_output`/`to_input` (no such validation
  exists today either — out of scope, matching Plan 8.2's non-goal of not
  adding backend enforcement for UI-convenience metadata).

## 7. Testing

- `src/domain.rs`: a deserialize test confirming a JSON connection object
  with no `"error"` key deserializes with `error: false` (proving the
  `#[serde(default)]` back-compat path for already-stored workflows).
- `src/engine.rs`: existing error-routing tests (`error_with_connected_
  error_route_continues_and_routes_error_item` and its
  `execute_workflow_seeded` counterpart) updated to construct their test
  connection with `error: true, from_output: 0` instead of the old
  sentinel — same assertions, same behavior, proving the routing logic
  change is behavior-preserving.
- `frontend/src/stores/nodeTypes.spec.ts` (new, none exists today): a test
  proving two concurrent `portsFor` calls for the same uncached key
  produce exactly one network call (in-flight de-dup), and a test that
  the request URL is properly encoded for a type name containing a
  character `encodeURIComponent` would escape (none currently do, but the
  test should use a synthetic case to prove the mechanism, not rely on a
  real node type name happening to need it).
- `frontend/src/components/WorkflowCanvas.spec.ts`: extended for a node
  whose mocked `portsFor` returns 2 ports (asserting 2 source handles +
  1 error handle = 3 total source handles, each with the right `id` and
  visible label text), the `onConnect` mapping for a normal numbered
  source handle vs. the `"error"` handle (asserting the emitted
  connection's `error` field), and a loaded connection with `error: true`
  correctly attaching to the `id="error"` handle (via `sourceHandle`).

## 8. Out of Scope / Deferred

- A visual `cases` editor (§2).
- Retry/timeout/continue-on-fail policy (§2, Plan 8.5).
- Structured credential forms (Plan 8.4).
- Backend validation of `error`/`from_output`/`to_input` combinations.
- `portsFor`'s cache-size growth (§4) — accepted, not fixed.
