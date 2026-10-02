# r8r: plan and task breakdown for the remaining work

Source: `docs/remaining-work.md` (handoff brief, status 2026-09-27: BDD
403/403 on SQLite and PostgreSQL). This plan orders the brief's gaps into
phases with dependencies, decisions and acceptance criteria, sized so each
task fits one working session (≤ 20 min of agent time per chunk; larger
tasks list their chunks).

## Review of the brief

The brief is accurate and unusually complete (paths, commands, pitfalls,
per-area gaps). Corrections and gaps found while reviewing it on
2026-09-27:

1. **Stale frontend build breaks the legacy editor after every pull.**
   `frontend/dist` is gitignored and embedded at compile time (rust-embed),
   so pulling the `/rest` → `/rest/r8r` route move left the built UI calling
   `/rest/auth/login` (no longer served): login failed. Rebuilding fixed it.
   The brief says to build the frontend "once"; it must be rebuilt whenever
   `frontend/src` changes. → Task 0.1.
2. **Remotes.** The live history is on the `github` remote; `origin/main`
   and local `main` are stale (both before PR #2). → Task 0.2.
3. **PR #4 is merged** (`45f4224` on `github/main`), not open.
4. **Two stacks, one decision missing.** Recent product work on the legacy
   path (node retry/timeout/continue-on-fail, background execution, credential
   editing, the tool library, the AI Agent settings form, logging) lives in
   `src/api` + `frontend/` under `/rest/r8r`. The brief lists only
   n8n-path gaps and calls the legacy path "not the n8n-compatible path",
   without saying whether those features are ported, kept, or retired.
   Together with the editor licensing question (brief §2) this decides what
   the user-facing product is. → Decision D2.
5. **Value delivery.** The brief recommends native nodes next (§1), but no
   UI talks to the n8n-compatible API yet (§2 blocked on licensing). New
   nodes are usable only through imported workflows, webhooks and the API
   until D1/D2 are settled. Fine to proceed, but worth doing D1/D2 first.
6. **Orphaned BDD work on branch `bdd-suite`.** `tests/features/`
   (60 `@r8r` + 47 `@n8n-compat` scenarios) and `tests/bdd/steps/{common,n8n}.rs`
   from 2026-09-26 are not run: the upstream runner reads
   `tests/bdd/features/` and its `steps/mod.rs` doesn't include those files.
   The upstream suite supersedes the `@n8n-compat` part. → Task 0.3.
7. **Suite runtime vs. session length.** The full BDD run takes ~15 min;
   run per feature directory (`-i 'features/04-nodes/*'`) during tasks and
   the full suite only at phase exits.

## Decisions (block the phases noted)

| ID | Decision | Outcome (2026-09-27) |
| --- | --- | --- |
| D1 | May r8r ship the n8n editor (Sustainable Use License)? | **Moot for now** — D2 keeps r8r's own editor, so the n8n editor is not shipped; revisit only if D2 changes |
| D2 | Fate of the legacy stack (`/rest/r8r`, `frontend/`) | **Keep r8r's Vue editor as the product UI and point it at the n8n-compatible API**; legacy features (tool library, credential editing, agent form, node settings) move onto the n8n path; `src/api` + the legacy engine retire once nothing uses them |
| D3 | Priority between nodes, editor and AI | **Native nodes first** (Phase 1), then editor migration (Phase 3) and AI (Phase 2) |
| D4 | Orphaned BDD work on `bdd-suite` | **Drop `@n8n-compat`** (superseded by the 403-scenario suite); **port the `@r8r` scenarios** into `tests/bdd/features/` as `@r8r-only` |

## Phase 0 — hygiene (now, small)

| # | Task | Acceptance |
| --- | --- | --- |
| 0.1 ✅ | Guard against a stale embedded frontend: `build.rs` fails (or rebuilds) when any file in `frontend/src` is newer than `frontend/dist/index.html`, with a message naming `npm run build` | Touching a `.vue` file and running `cargo build` without rebuilding the frontend errors with that message; normal builds unaffected |
| 0.2 ✅ | Sync branches: fast-forward local `main` to `github/main`, set upstream to `github`, fix or remove the stale `origin` | `git status` on `main` shows up to date with `github/main` |
| 0.3 ✅ | Resolve orphaned BDD work (D4): delete `tests/features/n8n_compat` and `steps/n8n.rs`; port the 60 `@r8r` scenarios to `/rest/r8r` paths as `@r8r-only` features under `tests/bdd/features/` using the upstream harness's steps | No unreferenced step files; every feature file is executed by the runner; ported scenarios green |
| 0.4 ✅ | Update the brief: PR #4 merged, remotes, frontend rebuild rule, pointer to this plan | Brief matches reality |

## Phase 1 — native nodes (brief §1; Phase 4 "60 native nodes")

22 nodes, one task each, in the brief's order. Every node task is:
(a) run the node on n8n 2.35.7 and record parameters + `runData` for the
typeVersions the editor creates; (b) BDD scenarios in
`tests/bdd/features/04-nodes/` (wiremock for SaaS; local Postgres/Redis with
`@requires-*`), red first; (c) implement `NodeType` + register + full
description in `node_types.rs` + credential type; (d) scenarios green on r8r
and on n8n; `codex.r8r.native = false` removed.

| Order | Node(s) | Notes / chunks |
| --- | --- | --- |
| 1.1 ✅ | Postgres | operations: executeQuery, insert, update, upsert, delete, select; query params; chunks: read ops · write ops |
| 1.2 ✅ | MySQL | reuses sql_common.rs; v2 uses dataMode/valuesToSend/columnToMatchOn (no resourceMapper); BLOB as hex, multi-statement queries unsupported |
| 1.3 ✅ | Redis | get/set/delete/incr/keys/publish/push/pop |
| 1.4 ✅ | Send Email (SMTP) | lettre; attachments from binary; `sendAndWait` (v2.1) not supported natively yet |
| 1.5 ✅ | Slack | message/channel/user/reaction/file upload; not yet: scheduled messages, star, userGroup, profile ops; OAuth2 without refresh |
| 1.6 ✅ | Google Sheets | append/appendOrUpdate/clear/create/delete/read/remove/update + spreadsheet create/deleteSpreadsheet; `googleApi` (service-account JWT) + `googleSheetsOAuth2Api` (with refresh); wiremock |
| 1.7 ✅ | HTML | extract (CSS selectors), generate table, convert |
| 1.8 ✅ | JWT | sign/decode/verify (c2e660e) |
| 1.9 ✅ | Extract from File / Convert to File | CSV, XLSX, JSON, text, HTML, PDF text |
| 1.10 ✅ | Compression | zip/gzip in/out |
| 1.11 ✅ | Read/Write Files | behind the same allow-list as Execute Command |
| 1.12 ✅ | Microsoft SQL ✅, MongoDB ✅, Data Tables ✅ | MongoDB: aggregate/delete/find/findOneAndReplace/findOneAndUpdate/insert/update + search-index ops (Atlas-only, fail against community `mongod` same as real n8n); `mongodb` crate (tokio-native, no spawn-split strictly required but kept for consistency); EJSON via `bson::Bson::try_from` (superset of the pinned reference, which has no `parseJsonToEjson` at all — only a hardcoded top-level `_id` string→ObjectId coercion, also reproduced). Data Tables: storage (`data_table`/`data_table_column` metadata + dynamic `data_table_user_<id>` physical tables, `src/n8n/data_table.rs`), REST API (`/rest/projects/:projectId/data-tables`, `/rest/data-tables-global`, `src/n8n/server/data_tables.rs`), node (`n8n-nodes-base.dataTable`, `src/n8n/nodes/data_table.rs`); r8r simplifies n8n's always-on personal project into a lazily-created one (`Store::personal_project`) and column value types onto TEXT/REAL-or-DOUBLE-PRECISION/INTEGER rather than native TIMESTAMPTZ/DOUBLE/BOOLEAN, for one cross-backend shape through sqlx's `Any` driver; CSV import/export and the public-API variant of the controller are not implemented |
| 1.13 | RabbitMQ, Kafka, MQTT | incl. their trigger variants (leader-only, Phase 4.1) |
| 1.14 ✅ | Gmail ✅, Google Drive ✅, Notion ✅, Airtable ✅, GitHub ✅, Telegram ✅, Discord ✅ | one task each; GitHub: file/issue/release/repository/review/user/organization (pullRequest, workflow, GitHub App auth out of scope); Gmail: Gmail Trigger and sendAndWait out of scope; Google Drive: Google Drive Trigger out of scope |
| 1.15 | Email Trigger (IMAP), FTP/SFTP ✅, SSH ✅ | FTP/SFTP and SSH: `russh`/`russh-sftp` (SSH exec + SFTP, pure Rust) and `suppaftp` (FTP, tokio feature); SSH resources command (execute) and file (upload/download, SFTP-backed); FTP operations delete/download/list/rename/upload across both protocols. Email Trigger (IMAP) out of scope for this task. |
| 1.16 ✅ | OpenAI (non-chat) | text (response/message/classify), image (generate/analyze), audio (generate/transcribe/translate), file (upload/list/deleteFile); not yet: assistant, conversation, video, built-in/connected tools |

**Exit:** all 22 green on r8r and n8n; full BDD suite green on SQLite and
PostgreSQL.

## Phase 2 — AI nodes (brief §3; spec §6.8)

| # | Task | Notes |
| --- | --- | --- |
| 2.1 | Ollama chat model | first: the user runs Ollama; OpenAI-compatible HTTP |
| 2.2 | Anthropic, OpenRouter, Groq, Mistral chat models | one task each; share the OpenAI-compatible client where possible |
| 2.3 | Google, Azure OpenAI, Bedrock chat models | auth-specific |
| 2.4 | Tools: HTTP Request Tool, Code Tool, Workflow Tool, `$fromAI` on any node | `$fromAI` touches `expr` + agent loop |
| 2.5 | Memory: Postgres, Redis | persist per session key |
| 2.6 | Root nodes: Q&A Chain, Summarization, Information Extractor, Text Classifier, Sentiment Analysis; Agent structured output + streaming | one task per node |
| 2.7 | Embeddings + vector stores (in-memory, PGVector, Qdrant, Pinecone, Supabase), document loaders, text splitters, output parsers | chunk per store |
| 2.8 | MCP Client Tool, MCP Server Trigger (`rmcp`) | |

**Exit:** scenarios in `features/10-ai/` for every node, green; agent sub-runs
render in `runData` like n8n.

## Phase 3 — r8r's editor onto the n8n-compatible API (brief §2, per D2)

The Vue editor in `frontend/` stays the product UI; it stops using
`/rest/r8r` and talks to the n8n-compatible API. Needs a spec + plan of its
own before starting (brainstorm first): data model changes are large
(name-keyed connections, `type`/`typeVersion`, n8n parameter shapes,
cookie session instead of JWT, n8n push messages).

| # | Task | Notes |
| --- | --- | --- |
| 3.1 | Spec: map every editor screen/call to its n8n API equivalent; list gaps in the n8n API | output: design doc + gap list |
| 3.2 | Auth: n8n owner setup/login (`/rest/login`, `n8n-auth` cookie) instead of `/rest/r8r/auth` JWT | |
| 3.3 | Workflow model in the editor: n8n workflow JSON (name-keyed connections, `typeVersion`, node descriptions from `/types/nodes.json`) | largest; canvas + node panel |
| 3.4 | Parameter forms generated from n8n node descriptions (`properties`, `displayOptions`) instead of raw JSON | enables every native node in the UI |
| 3.5 | Runs and results: `/rest/workflows/:id/run`, n8n push (`executionStarted`, `nodeExecuteAfter`, …), `runData` in the results panel | |
| 3.6 | Credentials on the n8n API (`/rest/credentials`, credential type descriptions) incl. the editing UX from the legacy work | |
| 3.7 | Port legacy-only features onto the n8n path: node retry/timeout/continue-on-fail UI (n8n node flags), agent settings + tools (n8n AI sub-nodes / `$fromAI` from Phase 2), background-run UX | |
| 3.8 | Playwright smoke: build, run with live push, inspect a workflow end to end | exit gate |
| 3.9 | Retire `src/api`, the legacy engine and `/rest/r8r` once nothing calls them | |

## Phase 4 — scale and operations (brief §4)

| # | Task | Acceptance |
| --- | --- | --- |
| 4.1 ✅ | Leader election (`src/n8n/election.rs`): Redis `SET NX EX` + a compare-and-expire Lua renew script, matching n8n's `multi-main-setup.ee.js`/`leader-election-client.js` (key `{prefix}:main_instance_leader`, `N8N_MULTI_MAIN_SETUP_ENABLED`/`_KEY_TTL`/`_CHECK_INTERVAL`, defaults 10 s/3 s). A `LeaderGate` (tokio `watch`) is the reusable hook: `n8n.leader.is_leader()` / `.subscribe()`; single-main (default) uses `LeaderGate::always_leader()`, unchanged from before. `src/n8n/server/activation.rs` gates schedule-timer spawning on `schedules_enabled && leader.is_leader()`, with `activate_schedules_for_all`/`deactivate_schedules_for_all` wired to takeover/stepdown (webhooks/forms stay registered on every main, including a new `sync_registrations` poll on `r8r start` too, not just `r8r webhook`, so activations made through one main reach the others). Leadership is surfaced on `/healthz/readiness` and `/rest/settings` (`isLeader`), since n8n does not expose instance role on either in the pinned version. PostgreSQL advisory-lock backend skipped as out of scope (Redis is required for n8n multi-main anyway; `ElectionBackend` is the seam for adding one later). No OpenTelemetry/push relay (4.2) between mains — that's its own task. | two mains fire a schedule once: `tests/bdd/features/13-scaling/multi_main.feature` |
| 4.2 | Push relay between mains via pub/sub | editor on main B sees a run started on main A |
| 4.3 | Queue gaps: stop an execution on a worker; relay worker push to the editor | BDD scenarios in queue mode |
| 4.4 | Binary data modes: filesystem, S3 (`N8N_DEFAULT_BINARY_DATA_MODE`), streamed | large file runs without inline base64 |
| 4.5 | OpenTelemetry: trace per execution, span per node, HTTP spans | spans visible in a local collector |
| 4.6 | Re-measure `@perf` on reference hardware with a separate load generator; optimise only if still over target | 4/4 perf scenarios |

## Phase 5 — runtime and security (brief §5)

| # | Task |
| --- | --- |
| 5.1 | `r8r runner`: Code node out of process over a local socket; no network, memory/time limits, module allow-lists (`NODE_FUNCTION_ALLOW_BUILTIN/EXTERNAL`, `N8N_RUNNERS_STDLIB_ALLOW`) |
| 5.2 | Bridge node host (Node.js) for non-native and community nodes; `/rest/community-packages` |
| 5.3 | Versioned credential envelope `r8n:v1:` (AES-256-GCM, per-record key), still reading CryptoJS; key rotation command |

## Phase 6 — enterprise (brief §6)

SSO (SAML, OIDC, LDAP) · source control (git push/pull) · external secrets ·
custom roles / full RBAC · folders. One task each; start after D1/D2 since
most surface in the editor.

## Phase 7 — conformance corpus (brief §7)

| # | Task |
| --- | --- |
| 7.1 | Recorder: run a workflow on n8n 2.35.7 with VCR-style HTTP fixtures, store `runData` |
| 7.2 | Replayer: run the same workflow + fixtures on r8r, diff `runData` item by item |
| 7.3 | Seed the corpus (public templates, n8n's own workflow tests) toward ~500 workflows |
| 7.4 | Expression differential fuzzing against n8n |

## Suggested order

Phase 0 → Phase 1 (native nodes, per D3) → Phase 3 (editor onto the n8n
API, per D2; start with its spec 3.1) → Phase 2 (AI) → Phases 4–7.
Phase 3.4 (forms from node descriptions) makes every Phase 1 node usable
in the UI, so Phase 3 can start in parallel once Phase 1 is under way.
Phase 4.1 should land before Phase 1.13's triggers.
