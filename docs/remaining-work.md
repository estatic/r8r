# r8r: remaining work (handoff brief)

This file is a work brief for an agent (e.g. Claude Code CLI) continuing
the r8r project. It lists what is still missing relative to the
reimplementation spec, in priority order, with the context needed to start
each task without re-discovering the codebase.

**Plan and task breakdown:** `docs/superpowers/plans/2026-09-27-remaining-work-plan.md`
(decisions D1–D4, phases 0–7). Phase 0 (hygiene) is done.

Status as of 2026-09-27: the BDD conformance suite passes **403/403 on
SQLite and 403/403 on PostgreSQL storage**, plus 60 `@r8r-only`
scenarios for r8r's own editor API in `tests/bdd/features/15-r8r-legacy/` (opt-in Redis, PostgreSQL
queue, Python and slow scenarios included). That covers the roadmap's
Phases 1–2, most of 3, queue mode from 4 and parts of 5. Everything below
is *not* covered by the suite yet.

---

## 0. Before you start

### Where things are

| Path | What |
| --- | --- |
| `raw/2026-09-25-n8n-in-rust-reimplementation-spec.md` | The spec (source of truth; roadmap in §9.1, nodes in §6.6, AI in §6.8, APIs in §6.9) |
| `src/n8n/` | The n8n-compatible core: `workflow.rs` (model), `engine.rs` (execution), `expr.rs` + `vm.rs` + `js/prelude.js` (expressions in QuickJS), `node.rs` (node contract, `ExecCtx`) |
| `src/n8n/nodes/` | Native nodes; `mod.rs` holds the `Registry` |
| `src/n8n/node_types.rs` | Editor descriptions (`/types/nodes.json`); non-native catalog entries are marked `codex.r8r.native = false` |
| `src/n8n/credential_types.rs` | Credential types: fields, validation, redaction, JSON schema |
| `src/n8n/server/` | HTTP server: `rest.rs` (editor API), `public_api.rs` (`/api/v1`), `webhooks.rs`, `runner.rs` (runs/persists executions), `activation.rs` (webhooks + schedules), `auth.rs`, `push.rs`, `worker.rs` (queue mode) |
| `src/n8n/store*.rs` | Storage on sqlx `Any` (SQLite or PostgreSQL); `store_batch.rs` = group commit |
| `src/n8n/queue.rs` | Queue mode backends (Redis, PostgreSQL) |
| `src/n8n/migrate.rs` | `r8r migrate-from-n8n` |
| `src/cli.rs`, `src/main.rs` | CLI and server entry |
| `src/api/`, `src/engine/`, `frontend/` | The **legacy** r8r engine + Vue editor, served under `/rest/r8r` and `/webhook-r8r`. Not the n8n-compatible path. Per decision D2 the Vue editor stays the product UI and moves onto the n8n-compatible API (plan Phase 3); `src/api` and the legacy engine retire afterwards. |
| `tests/bdd/` | Cucumber suite (black-box: drives the `r8r` binary). `README.md` explains the harness |
| `docs/superpowers/specs/2026-09-25-r8r-bdd-conformance-suite-design.md` | Suite design + status table |
| `docs/migrating-from-n8n.md` | User guide for the migration |
| `wiki/` | LLM wiki (see `CLAUDE.md`): update `wiki/concepts/bdd-conformance-suite.md` and append to `wiki/log.md` when status changes |

### How to build and test

```sh
cargo build
cargo test --lib                                   # unit tests
cargo test --test api_test --test health_test --test logging_test
cargo test --test bdd                              # full BDD suite (~15 min)
cargo test --test bdd -- --tags @phase-2           # one phase
cargo test --test bdd -- --name "some scenario"    # by name (regex)
cargo clippy --all-targets
```

- Opt-in scenarios: `R8R_BDD_INCLUDE=requires-redis,requires-postgres,requires-python-runner,slow`.
  They need local Redis (6379) and PostgreSQL (`postgres://postgres:postgres@127.0.0.1:5432/postgres`),
  and `python3`.
- Run everything on PostgreSQL storage: `R8R_BDD_STORAGE=postgres`.
- `@perf` scenarios: release build, one at a time, release-built harness
  (see `tests/bdd/README.md`).
- The frontend is embedded at compile time (`rust-embed`) and `frontend/dist`
  is gitignored, so **rebuild it whenever `frontend/src` changes** (including
  after a `git pull`): `cd frontend && npm ci && npm run build`. `build.rs`
  enforces this: `cargo build` fails when `frontend/dist` is missing or older
  than its sources (override: `R8R_SKIP_FRONTEND_CHECK=1`). A stale build
  once served a UI calling `/rest/auth/login` after the routes moved to
  `/rest/r8r`, which broke login.
- Remotes: the live history is on `github` (`git@github.com:estatic/r8r.git`);
  `origin` is a self-hosted Gitea mirror that may lag behind. Local `main`
  tracks `github/main`.
- A real n8n 2.35.7 can be installed with `npm i n8n@2.35.7` to check
  behaviour (start with `N8N_LISTEN_ADDRESS=127.0.0.1`, a free
  `N8N_RUNNERS_BROKER_PORT`, and wait for `/healthz/readiness`).

### Conventions and pitfalls

- **Behaviour is checked against real n8n.** New scenarios should pass on
  n8n 2.35.7 unless tagged `@beyond-n8n` (spec asks for more than n8n),
  `@n8n-licensed` (licensed n8n feature) or `@r8r-only`.
- **Never** skip, disable or loosen a scenario to get green; fix the code or,
  if the scenario is wrong, fix it against real n8n and say why.
- sqlx `Any` driver quirks on PostgreSQL (already worked around, keep it so):
  NULL parameters are sent as INT4 (statement cache is off; avoid
  `COALESCE(?, col)`), nullable columns must be read with
  `try_get_unchecked`/`opt_text`, and `INSERT … RETURNING` returns no rows
  (use `store_batch::new_id`). Write SQL with `?` placeholders through
  `Store::sql()`.
- r8r refuses to open an n8n database in place (both default to
  `~/.n8n/database.sqlite`).
- Commit messages: describe what and why; keep the wiki and the status table
  in the design doc current.

---

## 1. Native nodes (Phase 4: "60 native nodes") — recommended next

**Now native (38 incl. sub-nodes):** manual/schedule/webhook/form/error/
execute-workflow triggers, HTTP Request, Code (JS + Python), Set, If,
Filter, Switch, Merge, Loop Over Items, Split Out, Aggregate, Summarize,
Sort, Limit, Remove Duplicates, Compare Datasets, Wait, Respond to Webhook,
Execute Workflow, No-Op, Stop and Error, Date & Time, Crypto, XML,
Markdown, Execute Command, AI Agent, Basic LLM Chain, OpenAI Chat Model,
Calculator tool, Simple Memory.

**Missing from the spec's GA list (§6.6)** — described in the editor but
fail with "not supported natively":

| Group | Nodes |
| --- | --- |
| Databases | Postgres, MySQL, Microsoft SQL, MongoDB, Redis, Data Tables |
| Messaging | RabbitMQ, Kafka, MQTT |
| SaaS | Slack, Google Sheets, Gmail, Google Drive, Notion, Airtable, GitHub, Telegram, Discord |
| Files/protocols | Send Email (SMTP), Email Trigger (IMAP), FTP/SFTP, SSH, Read/Write Files, Extract from File, Convert to File, Compression |
| Utilities | HTML, JWT |
| AI | OpenAI (the non-chat "OpenAI" node) |

Suggested order: Postgres, MySQL, Redis, Send Email, Slack, Google Sheets,
HTML, JWT, Extract/Convert to File, then the rest.

For each node:
- Implement `NodeType` in `src/n8n/nodes/` (see `http.rs` for credentials,
  `transform.rs` for item handling), register it in `nodes/mod.rs`, give it
  full parameter descriptions in `node_types.rs` and any credential type in
  `credential_types.rs`.
- Match n8n's parameters and output shape exactly for the typeVersions the
  editor creates (check with real n8n: run the node there, compare
  `runData`).
- Add BDD scenarios in `tests/bdd/features/04-nodes/` (use wiremock for
  SaaS APIs; use the local Postgres/Redis for databases, tagged
  `@requires-postgres`/`@requires-redis`).

**Acceptance:** each node's scenarios pass on r8r and on n8n 2.35.7; the
node no longer carries `codex.r8r.native = false`.

## 2. The n8n editor (Phase 2 exit gate) — decided: r8r's own editor

- **Decided (D1/D2, 2026-09-27):** r8r keeps its own Vue editor and moves it
  onto the n8n-compatible API instead of shipping n8n's editor (which would
  need a licensing answer). See plan Phase 3. The notes below about the n8n
  editor's route inventory still help: they list what the n8n API must
  offer an editor.
- r8r serves only its legacy Vue editor (`frontend/`, on `/rest/r8r`).
  None of the UI talks to the n8n-compatible API yet.
- `/rest` implements what the scenarios need (auth, settings, workflows,
  runs, executions, credentials, push, types). Missing endpoints the n8n
  editor calls include folders, data tables, community packages, insights,
  evaluations, source control, `/rest/node-types` details
  (`loadOptions`, `listSearch`, `resourceMapping`), credential tests, and
  many smaller ones. The spec asks for a recorded route inventory of the
  pinned editor build (§6.9) — generate it by running the n8n 2.35.7
  editor against n8n and logging requests.
- **Acceptance:** the pinned editor builds, creates, runs (with live push)
  and debugs a workflow end to end against r8r; a Playwright smoke test is
  green.

## 3. AI nodes (Phase 5, §6.8)

Done: AI Agent (tools loop, max iterations), Basic LLM Chain, OpenAI Chat
Model (token usage in run data), Calculator tool, Simple Memory; chat
models for Ollama, Anthropic, OpenRouter, Groq, Mistral, Google Gemini,
Azure OpenAI (API key) and AWS Bedrock (IAM keys, SigV4) — plan tasks
2.1–2.3.

Missing:
- Chat models: Google Vertex, Azure Entra ID (OAuth2) auth, Bedrock
  assume-role auth, and the rest of n8n's list (DeepSeek, xAI, Cohere, …).
- Root nodes: Question and Answer Chain, Summarization, Information
  Extractor, Text Classifier, Sentiment Analysis; Agent structured output
  and streaming.
- Embeddings, vector stores (in-memory, PGVector, Qdrant, Pinecone,
  Supabase), document loaders, text splitters, output parsers.
- Tools: done for native nodes as tools (`<type>Tool`, e.g. HTTP Request
  Tool) with `$fromAI`, the JavaScript Code Tool and the Workflow Tool v2
  (plan 2.4); missing: Python Code Tool, Workflow Tool v1 / inline
  workflow source, the legacy `toolHttpRequest` node.
- Memory backed by Postgres/Redis (current memory is in-process only).
- MCP Client Tool and MCP Server Trigger (spec suggests the `rmcp` crate).

Code: `src/n8n/nodes/ai.rs` (sub-node calls are recorded in run data via
`RunState.sub_runs`). Scenarios: `tests/bdd/features/10-ai/`.

## 4. Scale and operations (Phase 4)

- **Multi-main / leader election:** schedules and timers would fire twice
  with two main processes. Needs a leader lease (Redis or PostgreSQL) and
  push relay between mains via pub/sub.
- **Queue mode gaps:** stopping an execution running on a worker, and push
  messages from worker executions to the editor, are not relayed.
- **Binary data:** kept inline (base64) in execution data. Spec asks for
  filesystem and S3 binary modes (`N8N_DEFAULT_BINARY_DATA_MODE`).
- **Observability:** JSON logs and Prometheus `/metrics` exist; OpenTelemetry
  tracing does not.
- **Performance targets (§8.1):** `@perf` passes 3/4. Webhook p99 under
  load measured 18–25 ms (target 15 ms) with the load generator on the same
  4 cores. Re-measure on reference hardware (4 vCPU, 8 GB, separate load
  generator) before optimising further.

## 5. Runtime and security

- **Code node sandbox:** JavaScript runs in-process in QuickJS; Python runs
  as a `python3` subprocess with an import allow-list. The spec (§6.7) asks
  for a separate `r8n runner` process behind a local broker socket, with
  no network, memory/time limits and module allow-lists
  (`NODE_FUNCTION_ALLOW_EXTERNAL` is not supported yet).
- **Bridge node host (Phase 3):** run any other n8n/community node in a
  Node.js host; required for community packages (`/rest/community-packages`).
- **Credential format:** credentials are written in n8n's CryptoJS format.
  The spec suggests a versioned AES-256-GCM envelope (`r8n:v1:`) once
  migration is confirmed, while still reading the n8n format.

## 6. Enterprise features (Phase 5)

SSO (SAML, OIDC, LDAP), source control (git sync), external secrets,
custom roles and the rest of RBAC beyond global owner/admin/member and
project admin/editor/viewer, folders.

## 7. Conformance corpus

The spec's Phase 0 asks for a recorded corpus of ~500 real workflows and
differential fuzzing of expressions against n8n. The suite has 403
hand-written scenarios validated on n8n. Building the corpus (record
n8n runs, replay on r8r, diff `runData`) would catch many edge cases the
scenarios miss.

---

## Open items outside the code

- PR [estatic/r8r#4](https://github.com/estatic/r8r/pull/4)
  (`migrate-from-n8n`) is merged (`45f4224`).
- n8n editor licensing: not needed while r8r ships its own editor (D2).
