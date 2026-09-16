# r8r Frontend — Plan 6a Design Spec

## 1. Summary

The first working slice of r8r's frontend: a Vue 3 SPA that lets a user log
in, create and edit workflows on a visual canvas, configure nodes, pick
credentials, and manually execute a workflow to see its result. It is
deliberately scoped to what's buildable against the REST API as it exists
today (plus three small, necessary additions — see §5), deferring the
features that have real backend dependencies not yet built: live execution
status (needs Plan 7's WebSocket push) and execution history/replay (needs
an executions-listing endpoint that doesn't exist).

This spec covers roadmap items §6.1 (Project Scaffold), §6.2 (Workflow
Editor), and §6.4 (Auth UI) from `docs/superpowers/plans/2026-09-14-r8r-roadmap-breakdown.md`.
§6.3 (Execution View — live status, execution log/replay) is out of scope
for this plan; it becomes its own future plan once its backend
dependencies exist.

## 2. Goals / Non-Goals

**Goals:**
- A user can register/log in, see their workflows, create a new one, edit
  it on a canvas (add/connect/configure/delete nodes, add/delete
  connections), save it, and execute it manually to see the result.
- A user can create and select credentials for nodes that need them,
  without ever seeing a previously-stored secret's plaintext value.
- Ships as part of the same single-binary deployment r8r already is — no
  separate frontend process or origin in production.

**Non-Goals (this plan):**
- Live execution status streaming (WebSocket push) — Plan 7 dependency.
- Execution history / replay of past runs — needs a new executions-listing
  API endpoint, out of scope here.
- Dynamic, schema-driven node parameter forms — the backend has no
  per-node parameter schema today (`Node::type_name()` is the only
  metadata a node exposes); building that out is its own project.
  Parameter editing in this plan is a raw, validated-JSON textarea.
- Multi-user concurrent-edit conflict handling — single-editor-at-a-time
  is an acceptable v1 assumption, consistent with the rest of this
  project's single-shared-workspace v1 model (spec §8's known,
  accepted trust model).
- End-to-end browser testing (Playwright etc.) — manual verification
  against the real running backend, same practice already used for
  backend feature work in this project.

## 3. High-Level Architecture

Vue 3 + TypeScript, built with Vite. State via Pinia (auth token, current
user, workflows list, credentials list, the workflow currently being
edited). Routing via Vue Router. Canvas rendering via Vue Flow. Styling via
Tailwind CSS, hand-built components (no component library) — this editor's
layout (canvas-first, slide-in panels) doesn't fit a general-purpose
component library's assumptions well, and Tailwind keeps the bundle small
and the team in full control of exactly how the canvas-adjacent UI behaves.

**Serving:** the Vite build output (`frontend/dist/`) is embedded into the
r8r binary via the `rust-embed` crate and served by the existing Axum
server — a catch-all route serves `index.html` for any path that isn't
`/rest/*`, `/webhook/*`, or a matched static asset, so Vue Router's
client-side routing (history mode) works correctly on a hard refresh or
direct link. One binary, one process, matching every other part of this
project's deployment story (no separate webhook process, no separate
frontend process). In development, `vite dev` serves the frontend with a
proxy to the Rust API (`/rest`, `/webhook`) running separately, for fast
iteration without a full binary rebuild per change.

**Auth:** JWT stored in `localStorage`, attached as `Authorization: Bearer
<token>` on every API call via an Axios (or `fetch`) interceptor. A 401
response clears the stored token and redirects to `/login`. r8r is a
self-hosted, single-tenant-per-instance tool, not a public multi-tenant
SaaS — this substantially lowers `localStorage`'s XSS blast radius
compared to a general web app, which is why it's an acceptable v1 choice
over the more defensive (but more disruptive, and needing new backend
work) in-memory-plus-httpOnly-refresh-cookie approach. Revisit if r8r's
threat model changes (e.g., a hosted multi-tenant offering).

## 4. Pages

- **`/login`, `/register`** — plain forms (email, password). Success
  stores the returned JWT and redirects to `/workflows`.
- **`/workflows`** — list of the current user's workflows: name, active
  toggle (calls the existing `PATCH /rest/workflows/:id/active`), created
  date, a delete action (calls the new `DELETE /rest/workflows/:id`, §5),
  and a "+ New workflow" button (calls the existing `POST /rest/workflows`
  with an empty/minimal graph, then navigates to its editor).
- **`/workflows/:id`** — the editor. Layout:
  - Top toolbar: workflow name (editable inline), **Execute**, **Active**
    toggle, **Save** (calls the new `PUT /rest/workflows/:id`, §5).
  - Full-width Vue Flow canvas below the toolbar, rendering the
    workflow's `nodes`/`connections`. Dragging a node updates its
    `position` in local editor state (persisted on Save, not on every
    drag). Drawing a connection between two node handles updates
    `connections` in local editor state.
  - **"+ Add node"** button opens a searchable list of node types,
    sourced from the new `GET /rest/node-types` endpoint (§5) — never
    hardcoded in the frontend, so it can't drift from what's actually
    registered in the running backend.
  - Double-clicking a node slides in a right-side panel: the node's
    `id`/`node_type` (read-only), a `disabled` toggle, and a raw JSON
    textarea for its `parameters` (validated as JSON client-side before
    it's accepted back into local editor state — a parse error blocks
    closing the panel with a clear inline message, it does not silently
    discard the edit).
  - **Execute** calls the existing `POST /rest/workflows/:id/execute`
    and shows the result (overall status, per-node status/output) in a
    results panel — a snapshot after the run completes, not a live
    stream (that's Plan 7+6's job).
- **Credential picker** (used from a node's config panel, when that
  node's parameters reference `auth.credential_id`): a dropdown populated
  from the existing `GET /rest/credentials` (name + type only — the API
  never returns decrypted secret material, and the frontend must never
  attempt to display one), plus "+ New credential" opening a small form
  (name, `credential_type`, and a JSON textarea for `data` — same
  raw-JSON-for-v1 reasoning as node parameters, since there's no
  per-credential-type schema either) that calls the existing
  `POST /rest/credentials`.

## 5. Backend Additions (small, in this plan's scope)

Three small REST additions, all thin wrappers over capability the backend
already has internally:

- **`PUT /rest/workflows/:id`** — full update of `name`/`nodes`/`connections`
  (not `active`, which keeps its own dedicated `PATCH .../active` endpoint
  and its existing trigger-activation side effects). Calls the
  already-existing `Storage::update_workflow` (already used internally by
  the active-toggle handler). Without this endpoint, the canvas editor
  could create a workflow but never save a subsequent edit to it — a
  first-slice frontend with no working "Save" button is not a usable
  first slice.
- **`DELETE /rest/workflows/:id`** — no `Storage` method exists for this
  yet either; add `delete_workflow` to the `Storage` trait + SQLite impl,
  plus the REST route. Without it, the workflow list accumulates
  permanently with no way to clean it up — a real, immediate annoyance
  the first time someone uses the editor for real.
- **`GET /rest/node-types`** — returns the list of registered node
  `type_name` strings (e.g. `["core.manualTrigger", "core.httpRequest",
  "telegram.sendMessage", "telegram.trigger", ...]`). `NodeRegistry`
  already holds this as its internal map's keys; this is a thin,
  read-only projection, not new domain logic.

None of these three touch node execution, credential encryption, or
trigger activation/deactivation logic — they're additive surface area on
top of existing, already-tested internals.

## 6. Error Handling

- **Auth**: a 401 from any API call clears the stored JWT and redirects to
  `/login`. Login/register form validation errors show inline under the
  relevant field.
- **Other API errors** (4xx/5xx from workflow/credential/execution calls):
  shown as an inline toast/banner near the action that triggered the call
  (e.g., near the Save button, not a global full-page error state) —
  keeps the user's editor state and position intact so they can retry
  without losing work.
- **Client-side JSON validation** (node parameters, credential data): a
  parse error is caught before the value leaves the editing panel; the
  panel stays open with an inline error message rather than silently
  discarding the edit or sending malformed JSON to the API.
- **Workflow execution failure**: the existing `POST .../execute` response
  already carries per-node status/output including error nodes (this is
  existing engine behavior, unchanged by this plan) — the results panel
  renders whatever the API returns, including a failed node's error
  message, without needing new error-handling logic beyond rendering the
  response.

## 7. Testing Strategy

- Vitest + Vue Test Utils for component and Pinia store unit tests
  (auth flow, API client error handling, node-parameter JSON validation,
  canvas-to-API-payload serialization).
- No end-to-end browser test suite in this first slice — Playwright is
  available in this environment as a future option, but setting up a full
  E2E harness is more infrastructure than a first slice needs. Each piece
  gets manually verified against the real running backend before being
  considered done, matching this project's existing practice for backend
  feature work (live demos against a real server, not just unit tests).
- The three new backend endpoints (§5) get the same Rust-side test
  treatment as every other endpoint in this codebase: unit tests plus an
  end-to-end integration test in `tests/api_test.rs`, per this project's
  existing conventions — no different from how every prior plan in this
  project has tested new REST surface area.

## 8. Explicitly Out of Scope (this plan)

Carried forward as future work, per the roadmap breakdown's own structure:

- §6.3.1 Live execution status (WebSocket) — depends on Plan 7 §7.1.
- §6.3.2 Inline JSON data preview beyond the post-execution results panel
  described in §4 — a live per-node preview during a run needs §6.3.1's
  WebSocket first.
- §6.3.3 Execution log/replay — needs an executions-listing API endpoint
  (`Storage` has no "list executions" method at all today; deliberately
  not added here, since designing it well deserves its own scoped pass
  rather than being bolted onto this plan).
- Dynamic per-node parameter schema forms (see §2 Non-Goals) — a natural
  enhancement once/if node parameter schemas exist as backend metadata.
- Any workflow edit-conflict handling beyond "last save wins."
