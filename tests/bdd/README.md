# r8r BDD conformance suite

Executable scenarios (Gherkin + [cucumber-rs](https://github.com/cucumber-rs/cucumber))
for the n8n-compatible behaviour described in *n8n in Rust — Analysis, Review &
Reimplementation Spec* (2026-09-25). The review and gap analysis behind them
is in `docs/superpowers/specs/2026-09-25-r8r-bdd-conformance-suite-design.md`.

The suite is written **before** the implementation. Most scenarios fail today;
each failure says what is missing. Treat the pass count as the progress bar.

## Running

The crate embeds `frontend/dist`, so build the frontend once first:

```sh
(cd frontend && npm ci && npm run build)

cargo test --test bdd                                  # default set
cargo test --test bdd -- --tags @phase-1               # one roadmap phase
cargo test --test bdd -- --tags '@spec-6.4 and not @security'
cargo test --test bdd -- -i 'tests/bdd/features/03-expressions/*'
cargo test --test bdd -- -n 'Loop Over Items'         # scenario name regex
R8R_BDD_INCLUDE=perf,slow cargo test --test bdd        # add opt-in tags
R8R_BIN=/path/to/r8r cargo test --test bdd             # test another build
```

| Variable | Default | Meaning |
| --- | --- | --- |
| `R8R_BIN` | the `r8r` binary Cargo builds | Binary under test |
| `R8R_BDD_CONCURRENCY` | 8 | Scenarios run in parallel |
| `R8R_BDD_INCLUDE` | – | Opt-in tags to add: `perf`, `slow`, `requires-redis`, `requires-postgres`, `requires-python-runner` |
| `R8R_BDD_REDIS_HOST` / `_PORT` | 127.0.0.1 / 6379 | Redis for `@requires-redis` |
| `R8R_BDD_POSTGRES_URL` | `postgres://postgres:postgres@127.0.0.1:5432/postgres` | PostgreSQL for `@requires-postgres` |

`--tags` replaces the opt-in filter, so `--tags @perf` runs perf scenarios.

## How it works

The binary is driven **black-box**; no step imports `r8r::*`. The spec
restructures the crate into a workspace, and the suite must keep compiling
through that.

- **Headless**: `r8r execute --file=<workflow.json> --rawOutput` runs a
  workflow and prints n8n's `IRun` JSON. Engine, expression and node
  scenarios use this; no server or database is needed.
- **CLI**: `import:workflow`, `export:workflow`, `import:credentials`,
  `export:credentials`, `config check`, `migrate-from-n8n`.
- **Server**: `r8r start` on a free port, per scenario, with its own
  database in a temporary `N8N_USER_FOLDER`. Scenarios talk to it over
  `/rest`, `/api/v1`, `/webhook*`, `/form`, `/rest/push` and `/healthz`
  the way n8n clients do.
- **External services** are faked with wiremock: plain HTTP APIs, OAuth2
  token endpoints and an OpenAI-compatible chat API.

Each scenario gets a fresh scratch folder and processes, killed when it ends,
so scenarios can run in parallel.

## Layout

```
tests/bdd/
  main.rs              runner: tag filter, concurrency, fail on undefined steps
  world.rs             per-scenario state (processes, HTTP, placeholders)
  support/             process spawning, JSON matching, workflow builder,
                       n8n's CryptoJS credential encryption
  steps/               step definitions, one module per area
  features/
    01-workflow-model  n8n workflow JSON contract, import/export round trip
    02-engine          items, runData, v0/v1 order, node flags, loops, merge,
                       pin data, partial runs, sub-workflows, error workflows,
                       wait/resume, timeouts
    03-expressions     syntax, data proxy, extension methods, Luxon, sandbox
    04-nodes           Set, If/Filter/Switch, Merge, Code, HTTP Request,
                       transforms, utilities
    05-triggers        webhooks, response modes, webhook auth, schedule, forms
    06-credentials     n8n encryption compatibility, API, OAuth2
    07-api             public API, editor REST + auth, push, health, metrics
    08-persistence     save policies, crash recovery, graceful shutdown
    09-users-rbac      members, projects, credential access
    10-ai              LLM chain, agent + tools, memory, max iterations
    11-security        dangerous nodes, SSRF, Code node isolation
    12-cli-config      subcommands, typed config, default port
    13-scaling         queue mode (Redis / PostgreSQL)
    14-nfr             performance targets, structured logs, metrics
```

## Tags

| Tag | Meaning |
| --- | --- |
| `@phase-1` … `@phase-5` | Roadmap phase (spec §9.1) that should make the scenario pass |
| `@spec-6.2` etc. | Spec section the scenario checks |
| `@security` | Security requirement (spec §8.2, goal G6) |
| `@node-*` | Node the scenario is about |
| `@perf`, `@slow` | Opt-in: timing-sensitive or long |
| `@requires-*` | Opt-in: needs external infrastructure |

## Writing scenarios

Workflows are n8n JSON, built from tables:

```gherkin
Given a workflow with nodes:
  | name  | type          | onError             | parameters                                |
  | Start | manualTrigger |                     |                                           |
  | Call  | httpRequest   | continueErrorOutput | {"url": "%{MOCK_URL}/x", "options": {}}   |
And the connections:
  """
  Start -> Call
  Call:1 -> Handle errors
  Model -[ai_languageModel]-> Agent
  """
And the trigger outputs the items:
  """
  [{"id": 1}]
  """
```

- `type` without a dot means `n8n-nodes-base.<type>`; `lc.<type>` means
  `@n8n/n8n-nodes-langchain.<type>`. `typeVersion` defaults to the versions
  in `support/workflow.rs`.
- Other columns become node properties (`disabled`, `onError`,
  `retryOnFail`, `maxTries`, `credentials`, ...), parsed as JSON when they can be.
- `the node "X" sets the fields:` / `adds the fields:` build an Edit Fields
  (Set) node in raw mode, so values keep their JSON types.
- `%{NAME}` placeholders: `MOCK_URL`, `SERVER_URL`, `WORKFLOW_ID`,
  `WORKFLOW_ID:<name>`, `CREDENTIAL_ID:<name>`, `WEBHOOK_ID:<node>`,
  `EXECUTION_ID`, `USER_ID:<email>`, and anything saved with
  `I remember the response JSON at "<path>" as "<NAME>"`.
- Expected JSON may use wildcards: `$any`, `$string`, `$number`,
  `$boolean`, `$nonempty`, `$datetime`, `$regex:<re>`, `$contains:<text>`.
  "outputs:" and "is:" compare exactly; "matching:" allows extra keys.

Undefined steps fail the run (`fail_on_skipped`), so a typo shows up
immediately.

## Harness contracts

The spec fixes the n8n-facing contracts but leaves some details open. The
suite pins these down; change them here and in the steps together if the
team decides otherwise.

1. **`r8r execute --file=<f> --rawOutput`** prints the `IRun` JSON on stdout
   (status, mode, `data.resultData.runData`, `lastNodeExecuted`, `error`),
   exits 0 on success and non-zero on a failed run, and runs as a *manual*
   execution: pin data is used and `$execution.mode` is `"test"`.
2. **Run order** is read from `executionIndex` on each task (n8n ≥ 1.7x),
   falling back to `startTime`.
3. **`R8R_SSRF_ALLOWED_HOSTS`** (comma-separated) allows private hosts
   through the SSRF guard. The harness sets it to `127.0.0.1,localhost` so
   wiremock is reachable. The SSRF scenarios unset it.
4. **Queue backend** is chosen with `R8R_QUEUE_BACKEND=postgres` (default
   Redis Streams), with the database from `R8R_DATABASE_URL`.
5. **Environment**: every process starts from an empty environment plus
   n8n variable names (`N8N_USER_FOLDER`, `N8N_ENCRYPTION_KEY`, `N8N_PORT`,
   `WEBHOOK_URL`, `GENERIC_TIMEZONE`, ...). Servers also get the variables
   the current pre-spec binary needs to boot (`PORT`, `JWT_SECRET`,
   `CREDENTIALS_KEY`, `DATABASE_URL`, see `legacy_server_env`); delete them
   once `src/main.rs` reads n8n names.
6. **API keys**: `POST /rest/api-keys` with `label`, `scopes`, `expiresAt`
   returns `data.rawApiKey`, as n8n 1.8x+ does.

Parameter shapes follow the n8n node versions listed in
`support/workflow.rs`. The spec asks for these to be checked against the
pinned n8n version in Phase 0; the fastest way is to run these same
workflows through real n8n and diff the results.
