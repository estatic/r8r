# r8r

r8r is a reimplementation of [n8n](https://n8n.io) in Rust: a workflow automation server that stays behaviourally compatible with n8n 2.x. Workflows, credentials, expressions, webhooks and the REST APIs work the way n8n's do, so an n8n instance can be moved to r8r and back.

Compatibility is defined by tests, not by porting n8n's code line by line. The BDD conformance suite in `tests/bdd/` runs the same scenarios against r8r and against real n8n 2.35.7.

## Status

- **The default BDD suite passes in full: 1001/1001 scenarios.** The opt-in scenarios (Redis, PostgreSQL, Python runner, slow) pass too.
- **The expectations are checked against real n8n:** the scenarios that apply to n8n pass on n8n 2.35.7. The rest are tagged `@beyond-n8n`, `@n8n-licensed` or `@r8r-only`.
- **The same suite passes on PostgreSQL storage** (`R8R_BDD_STORAGE=postgres`).
- **3 of 4 performance scenarios pass.** The miss is the webhook p99 under load (about 18–25 ms against a 15 ms target), measured with the load generator on the same machine.

The target specification is `raw/2026-09-25-n8n-in-rust-reimplementation-spec.md`; a summary is in [`wiki/sources/n8n-in-rust-reimplementation-spec.md`](wiki/sources/n8n-in-rust-reimplementation-spec.md). Work still open is listed in [`docs/remaining-work.md`](docs/remaining-work.md).

## What is compatible with n8n

These contracts are frozen to n8n's behaviour; everything behind them is r8r's own:

- **Workflow JSON**, with n8n's node types and `typeVersion`s.
- **Node type descriptions** at `/types/nodes.json`.
- **Expressions:** results of `=` values and `{{ }}` blocks, with n8n's data proxy (`$json`, `$node`, `$()`, `$input` …), extension methods and Luxon, run sandboxed in QuickJS.
- **Execution data:** the `runData` shape, `pairedItem`, per-node retry and `onError`, execution order v1 and v0, pin data, partial runs, Wait/resume, and sub-workflows.
- **Webhook URLs:** `/webhook`, `/webhook-test`, `/webhook-waiting` (signed resume URLs) and `/form`.
- **Credential encryption:** n8n's AES format with the same `N8N_ENCRYPTION_KEY`, so n8n's encrypted credentials load unchanged. OAuth1/2 refresh on 401.
- **APIs:** the editor's `/rest` API with `n8n-auth` sessions, the public `/api/v1` with scoped API keys, and `/rest/push`.

## Features

- **Nodes.** About 75 node types:
  - **Triggers and core:** manual, schedule, webhook, form, error and Execute Workflow triggers; HTTP Request, Code (JavaScript and Python), Set, If, Switch, Filter, Merge, Loop Over Items, and data transforms (Split Out, Aggregate, Summarize, Sort, Limit, Remove Duplicates, Compare Datasets).
  - **Files and formats:** Date & Time, Crypto, XML, HTML, Markdown, JWT, Compression, files.
  - **Services:** email and IMAP, FTP/SFTP, SSH, Postgres, MySQL, MSSQL, MongoDB, Redis, Data Tables, RabbitMQ, Kafka, MQTT, Slack, Telegram, Discord, GitHub, Notion, Airtable, Google Sheets, Gmail and Google Drive.
- **AI cluster.**
  - **Agents and chains:** AI Agent (tool calling, structured output), Basic LLM Chain, Q&A, Summarization, Information Extractor, Text Classifier and Sentiment Analysis.
  - **Chat models:** OpenAI, Ollama, Anthropic, OpenRouter, Groq, Mistral, Google Gemini, Azure OpenAI and AWS Bedrock.
  - **Memory:** window buffer, Postgres and Redis chat memory.
  - **Tools:** any native node as a tool via `$fromAI`, plus the Code Tool, Workflow Tool and Calculator.
  - **Retrieval:** embeddings, document loaders, text splitters, and the in-memory, PGVector, Qdrant, Pinecone and Supabase vector stores, with a Cohere reranker.
- **Storage:** SQLite by default; PostgreSQL with `DB_TYPE=postgresdb`.
- **Queue mode:** `r8r worker` and `r8r webhook` processes on a Redis or PostgreSQL job queue, with leases and heartbeats so a dead worker's job is re-queued.
- **Migration from n8n:** `r8r migrate-from-n8n` imports an n8n 1.x/2.x database (SQLite or PostgreSQL) read-only: users, API keys, projects, workflows, credentials, variables and executions. See [`docs/migrating-from-n8n.md`](docs/migrating-from-n8n.md).
- **Security:**
  - an SSRF guard for outgoing requests (`R8R_SSRF_ALLOWED_HOSTS`);
  - sandboxed expressions and Code (memory and time limits; Python imports allow-listed);
  - credential keys and bot tokens are sent only to the address configured on the credential.

## Quick start

You need a current stable Rust toolchain and Node.js (to build the editor). Python 3 is optional; the Python Code node needs it.

```sh
# 1. Build the editor (embedded into the binary; the Rust build checks it is current)
cd frontend && npm ci && npm run build && cd ..

# 2. Build and start r8r
cargo build --release
./target/release/r8r start
```

Open <http://localhost:5678> and register the first user, who becomes the owner. After that, registration is closed unless `R8R_ALLOW_OPEN_REGISTRATION=true`.

Data lives in `~/.n8n/` by default: `database.sqlite`, plus a generated encryption key. Set `N8N_ENCRYPTION_KEY` yourself to share credentials with an n8n instance or with other r8r processes.

For a ready-made example, `demo/rag/run.sh` starts r8r with a local retrieval-augmented chat setup on Ollama. No API keys needed; see [`demo/rag/README.md`](demo/rag/README.md).

## Configuration

r8r reads n8n's environment variables where n8n has one. The most used:

| Variable | Default | Meaning |
| --- | --- | --- |
| `N8N_PORT` / `PORT` | `5678` | HTTP port |
| `N8N_LISTEN_ADDRESS` | `0.0.0.0` | Listen address |
| `N8N_USER_FOLDER` | `$HOME` | Where `.n8n/` (database, key) lives |
| `N8N_ENCRYPTION_KEY` | generated | Credential encryption key (n8n-compatible) |
| `DB_TYPE` | `sqlite` | `sqlite` or `postgresdb` |
| `DB_SQLITE_DATABASE` | `~/.n8n/database.sqlite` | SQLite file |
| `DB_POSTGRESDB_HOST`, `_PORT`, `_DATABASE`, `_USER`, `_PASSWORD`, `_SCHEMA`, `_SSL_ENABLED` | | PostgreSQL connection (or `R8R_DATABASE_URL`) |
| `EXECUTIONS_MODE` | `regular` | `queue` to run executions on workers |
| `R8R_QUEUE_BACKEND` | `redis` | Queue backend: `redis` or `postgres` |
| `QUEUE_BULL_REDIS_HOST`, `_PORT`, `_PASSWORD` … | | Redis for queue mode |
| `WEBHOOK_URL` | | Public base URL for webhooks |
| `GENERIC_TIMEZONE` | | Default time zone for schedules and dates |
| `N8N_LOG_LEVEL`, `N8N_LOG_FORMAT` | `info`, `text` | Logging; `R8R_LOG_FILE` also writes to a file |
| `R8R_SSRF_ALLOWED_HOSTS` | empty | Private hosts outgoing requests may reach (e.g. `localhost,127.0.0.1` for a local Ollama) |
| `N8N_RUNNERS_TASK_TIMEOUT` | `60` | Code node time limit (seconds) |
| `NODE_FUNCTION_ALLOW_BUILTIN`, `N8N_RUNNERS_STDLIB_ALLOW` | | Modules Code may import |
| `R8R_PYTHON_PATH` | `python3` | Python for the Python Code node |
| `R8R_ALLOW_OPEN_REGISTRATION` | `false` | Allow sign-ups after the first user |

`r8r config check` validates the configuration and prints the effective values.

## Command line

The commands mirror n8n's CLI:

| Command | What it does |
| --- | --- |
| `r8r start` (default) | Editor, APIs, webhooks and triggers |
| `r8r worker [--concurrency N]` | Queue-mode worker |
| `r8r webhook` | Webhook processor (production URLs only) |
| `r8r execute --id <id> [--rawOutput]` | Run a stored workflow once and print the result |
| `r8r import:workflow --input <file>` / `export:workflow --all` | Import or export workflows (n8n JSON) |
| `r8r import:credentials --input <file>` / `export:credentials [--decrypted]` | Import or export credentials |
| `r8r migrate-from-n8n --db <sqlite:path or postgres://…>` | Import an n8n database |
| `r8r config check` | Validate configuration |

## The editor

`frontend/` is r8r's own Vue editor. It is not n8n's, whose reuse would need a license review (spec §9).

The editor currently runs workflows on r8r's original, smaller engine (`src/nodes`, 16 node types). Moving it onto the n8n-compatible engine (`src/n8n`), and with it every node above, is in progress. Plan: [`docs/superpowers/plans/2026-10-08-editor-on-n8n-engine-roadmap.md`](docs/superpowers/plans/2026-10-08-editor-on-n8n-engine-roadmap.md).

## Testing

```sh
cargo test                                   # unit tests, API tests, and the default BDD suite
cd frontend && npx vitest run                # editor tests
```

- **BDD scenarios** are tagged with the spec section (`@spec-6.2`) and roadmap phase. Heavy ones are opt-in (`@requires-redis`, `@requires-postgres`, `@requires-python-runner`, `@slow`, `@perf`); enable them with `R8R_BDD_INCLUDE`.
- **Running the same scenarios against real n8n**, to check an expectation, is described in [`tests/bdd/README.md`](tests/bdd/README.md). Design notes: [`wiki/concepts/bdd-conformance-suite.md`](wiki/concepts/bdd-conformance-suite.md).

## Project layout

| Path | Contents |
| --- | --- |
| `src/n8n/` | The n8n-compatible engine: workflow model, expressions, nodes, credentials, server (`/rest`, `/api/v1`, webhooks, push), queue mode, storage |
| `src/nodes/`, `src/api/` | The original engine and the API the editor uses today |
| `frontend/` | The Vue editor |
| `tests/bdd/` | Conformance suite (cucumber) |
| `docs/` | Guides (`adding-a-node.md`, `migrating-from-n8n.md`) and plans |
| `wiki/` | Project knowledge base (start at [`wiki/index.md`](wiki/index.md)) |
| `raw/` | Source documents, including the reimplementation spec |
| `demo/` | Runnable demos |

## License

No license has been chosen yet. n8n itself is fair-code under the Sustainable Use License. r8r reimplements its behaviour and public interfaces without copying its code, editor or node descriptions (spec §9). This is not legal advice.
