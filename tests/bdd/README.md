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
| `R8R_BDD_START_TIMEOUT` | 30 | Seconds a server may take to become ready |

`--tags` replaces the opt-in filter, so `--tags @perf` runs perf scenarios.

## How it works

The binary is driven **black-box**; no step imports `r8r::*`. The spec
restructures the crate into a workspace, and the suite must keep compiling
through that.

- **Headless**: `r8r import:workflow` + `r8r execute --id --rawOutput` run
  a workflow and print n8n's `IRun` JSON. Engine, expression and node
  scenarios use this; no server is needed.
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
| `@beyond-n8n` | The spec asks for more than n8n 2.35 does; n8n fails these |
| `@n8n-licensed` | n8n gates the feature behind a licence (variables, team projects) |
| `@r8r-only` | r8r commands n8n doesn't have (`config check`, `migrate-from-n8n`) |

## Checking the expectations against n8n

The scenarios are n8n's own contracts, so real n8n should pass them. That
is the fastest way to check an expectation (spec §9.1, Phase 0):

```sh
npm install n8n@2.35.7            # newest release that runs on Node 22
R8R_BIN=$PWD/node_modules/.bin/n8n R8R_BDD_START_TIMEOUT=120 \
  cargo test --test bdd -- --tags 'not @beyond-n8n and not @n8n-licensed and not @r8r-only
    and not @perf and not @slow and not @requires-redis and not @requires-postgres
    and not @requires-python-runner'
```

On 2026-09-25 n8n 2.35.7 passed all 358 of these scenarios. When n8n
fails a scenario, either the expectation is wrong (fix it) or the spec
asks for more than n8n does (tag it `@beyond-n8n` and say why in a
comment above the scenario).

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

1. **Headless runs follow n8n 2.x**: `r8r import:workflow --input=<f>` then
   `r8r execute --id=<id> --rawOutput`, which prints the `IRun` JSON
   (status, mode, `data.resultData.runData`, `lastNodeExecuted`, `error`)
   after any log lines. It runs in `cli` mode, where pin data does not
   apply, so `the trigger outputs the items:` renames the trigger to
   "<name> (trigger)" and inserts a Code node under the original name that
   returns the items (`support::workflow::prepare_for_cli`).
2. **Run order** is read from `executionIndex` on each task (n8n ≥ 1.7x),
   falling back to `startTime`.
3. **`R8R_SSRF_ALLOWED_HOSTS`** (comma-separated) allows private hosts
   through the SSRF guard. The harness sets it to `127.0.0.1,localhost` so
   wiremock is reachable. The SSRF scenarios unset it.
4. **Queue backend** is chosen with `R8R_QUEUE_BACKEND=postgres` (default
   Redis Streams), with the database from `R8R_DATABASE_URL`.
5. **Environment**: every process starts from an empty environment plus
   n8n variable names (`N8N_USER_FOLDER`, `N8N_ENCRYPTION_KEY`, `N8N_PORT`,
   `WEBHOOK_URL`, `GENERIC_TIMEZONE`, ...). Servers also get `PORT` (see
   `legacy_server_env`). Server and CLI share
   `<user folder>/.n8n/database.sqlite`, as n8n's do.
6. **API keys**: `POST /rest/api-keys` with `label`, `scopes`, `expiresAt`
   returns `data.rawApiKey`, as n8n does.
7. **Readiness**: a server is used once its port is open *and*
   `/healthz/readiness` stops answering "starting up".
8. **Each process gets its own `N8N_RUNNERS_BROKER_PORT`**, so parallel
   processes don't collide on n8n's default 5679.

Parameter shapes follow the n8n node versions listed in
`support/workflow.rs`. The spec asks for these to be checked against the
pinned n8n version in Phase 0; the fastest way is to run these same
workflows through real n8n and diff the results.
