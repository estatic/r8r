# r8r BDD Conformance Suite — Spec Review and Design

## 1. Summary

This document reviews *n8n in Rust — Analysis, Review & Reimplementation
Spec* (2026-09-25, `raw/2026-09-25-n8n-in-rust-reimplementation-spec.md`)
and describes the BDD suite built from it in `tests/bdd/`. The spec's own
advice (§3.3 lesson 5) is to build the conformance suite before the engine,
and the suite does exactly that. It has 392 cucumber scenarios in 14 areas,
driving the `r8r` binary black-box through n8n's own contracts.

The expectations were checked against **real n8n 2.35.7** (the newest
release that runs on Node 22), with the same suite and `R8R_BIN` pointed
at `n8n`. That found a dozen places where my first reading of n8n was
wrong, and several n8n 2.x behaviours the spec does not mention (§4). After
correcting them, **n8n passes all 358 scenarios it should**. The rest are
spec requirements beyond n8n, features n8n licenses, r8r-only commands,
or opt-in perf and infrastructure scenarios.

Against the current r8r code all 392 scenarios fail, because the binary has
no CLI subcommands and none of n8n's `/rest` auth routes, so nearly every
scenario stops at its first step. The suite is the progress bar for the
rewrite.

No production code was changed.

## 2. Review of the spec

### 2.1 What is strong

- **Parity is defined by behaviour, not code** (§1, §3.3). That makes the
  target testable: the frozen contracts in §4.3 are exactly what the suite
  checks.
- **Scope is realistic.** It keeps the Vue editor, bridges the long tail of
  integrations, and ports ~60 nodes natively. Those choices cut years.
- **It names the right weaknesses of n8n** (§3.2): memory model, event-loop
  latency, sandboxing, stalled jobs, config sprawl. Each has a concrete
  response, and most of those responses can be checked from outside.

### 2.2 What is under-specified

Turning the spec into executable scenarios surfaced these gaps. Each one
needs a decision; the suite records a default (see §5):

| Area | Gap | Default the suite uses |
| --- | --- | --- |
| CLI (§2.1, Phase 1) | "CLI `execute`" is named but not specified | n8n 2.x's contract: `import:workflow` + `execute --id --rawOutput` |
| Pinned n8n version (§9.4) | No version is pinned, so node `typeVersion`s and parameter shapes are unfixed | n8n 2.35.x shapes, versions listed in `support/workflow.rs` |
| SSRF guard (§5.3) | "blocks private ranges unless allowed", with no setting named | `R8R_SSRF_ALLOWED_HOSTS` |
| Queue backend (§7.3) | Redis Streams or Postgres, with no switch named | `R8R_QUEUE_BACKEND=postgres` |
| Rename propagation (§6.1 MUST) | n8n does this in the editor; there is no API surface to test | Not covered; needs an API decision |
| Timeouts (§6.2 step 8) | Status of a timed-out execution isn't stated | Accept `canceled` or `error` |
| n8n 2.x publishing | Not mentioned at all (see §4) | Scenarios publish sub-workflows and error workflows |
| Signed resume URLs | Not mentioned; spec lists `/webhook-waiting/:id` only | Resume through `$execution.resumeUrl` |
| Licensing of n8n features | Variables and team projects are licensed in n8n; spec lists them as GA features | Scenarios tagged `@n8n-licensed` |

### 2.3 Risks I would add

- **Moving target.** n8n shipped 2.0 → 2.40 in the period the spec
  describes, and changed the CLI (`execute --file` is gone), manual runs,
  invitations and publishing along the way. Pinning one version per r8r
  release (§9.2) is essential, and re-running the suite against that n8n
  version should be part of every pin bump.
- **Performance targets** (§8.1) are unmeasured proposals. The `@perf`
  scenarios encode them, but they need the Phase 0 baseline first.

## 3. Suite design

See `tests/bdd/README.md` for usage. Key points:

- **Black-box.** Steps spawn the binary as a CLI or as a server on a free
  port with its own `N8N_USER_FOLDER`; no step imports `r8r::*`. The suite
  keeps compiling through the planned workspace split, and it runs
  unchanged against n8n (`R8R_BIN=…/n8n`).
- **n8n JSON everywhere.** Workflows are built as n8n workflow JSON with
  real node types and versions, from Gherkin tables and `A -> B` connection
  lines.
- **Mocks** stand in for the outside world: wiremock for HTTP APIs, OAuth2
  token endpoints and an OpenAI-compatible chat API.
- **Traceability.** Every scenario is tagged with its spec section
  (`@spec-6.2`) and roadmap phase (`@phase-1`…`@phase-5`); opt-in tags
  (`@perf`, `@slow`, `@requires-redis`, `@requires-postgres`,
  `@requires-python-runner`) keep the default run self-contained.
- **Validation tags.** `@beyond-n8n` marks where the spec deliberately goes
  further than n8n (so n8n fails these), `@n8n-licensed` marks features n8n
  gates behind a licence, and `@r8r-only` marks commands n8n doesn't have.

### 3.1 Coverage

| Area | Spec | Scenarios |
| --- | --- | --- |
| 01 Workflow model | §2.3, §6.1, §4.3 | import/export round trip, unknown fields, key order, cycles, unknown types |
| 02 Engine | §2.4, §6.2 | items and runData shape, pairedItem, v0/v1 order, disabled, retries, onError modes, alwaysOutputData, executeOnce, loops, merge, pin data, partial runs, sub-workflows + callerPolicy, error workflows, wait/resume, timeouts, cancellation |
| 03 Expressions | §6.4 | `=`/`{{ }}` splitting and native types, full data proxy, ~50 extension methods, Luxon, timezones, `$env` blocking, sandbox escapes, time and memory limits |
| 04 Nodes | §6.6, §6.7 | Set, If, Filter, Switch, Merge, Code, HTTP Request, Sort, Limit, Aggregate, Split Out, Remove Duplicates, Summarize, Crypto, Date & Time, Markdown, XML, Compare Datasets, Stop and Error |
| 05 Triggers | §6.3 | production/test webhooks, methods, path params, conflicts, restart survival, binary/form bodies, CORS, payload limit, response modes, Respond to Webhook, header/basic/JWT auth, schedules, forms |
| 06 Credentials | §6.5, G3 | decrypting n8n CryptoJS blobs, n8n-readable exports, wrong key, secrets never returned/logged, schema, OAuth2 client credentials + refresh on 401 |
| 07 API | §6.9–6.11 | public API workflows/executions/tags/variables/users/audit/docs and scopes; `/rest` owner setup, login cookie flags, settings, `/types/nodes.json` incl. the full native GA node list; push; health; metrics |
| 08 Persistence | §7.2, §8.3 | save policies, crash → `crashed`, graceful shutdown, restart |
| 09 Users/RBAC | §6.10 | invitations, member isolation, credential access, team projects |
| 10 AI | §6.8 | LLM chain, agent tool loop, max iterations, window memory, provider errors, token usage |
| 11 Security | §8.2, §2.5 | Execute Command/Local File Trigger off by default, SSRF, Code node isolation (network, modules, env, timeout, memory), login rate limit |
| 12 CLI/config | §5.1, G4 | subcommands, `config check`, validation naming the variable, default port 5678, key generation, `migrate-from-n8n` |
| 13 Scaling | §7.3 | queue mode with Redis and Postgres, worker death → lease re-queue |
| 14 NFR | §8.1, §8.4 | cold start, idle RSS, webhook p99 and throughput, 100k-item Set, JSON logs, metrics |

Not covered yet: rename propagation (§6.1, no API), external secrets,
source control, SSO/LDAP/MFA, log streaming, insights, the node bridge and
community packages, and Python code beyond one opt-in scenario. These are
mostly SHOULD-level or Phase 5 and can be added as the APIs are decided.

## 4. What running the suite against n8n 2.35.7 showed

Validation took several rounds. The first headless run passed 103 of 212,
mostly because of harness problems (n8n's task broker port colliding
across parallel processes, and the removed `execute --file`). The final
run passes all 358 n8n-applicable scenarios. The corrections to
expectations, all now in the suite, fall into two groups.

**My expectations were wrong about n8n:**
- A workflow *without* `settings.executionOrder` runs in **v0** order.
  New workflows are saved with `v1`; old ones keep legacy ordering.
- `$node['X'].json` follows the current item index; it doesn't always
  return the first item.
- `export:workflow --id` writes an array; `keepFieldsContaining` only
  matches strings; Split Out with included fields keeps the split field
  under its own name; there is no `toCamelCase()`; the agent's tool is
  named after its node (`Calculator`); hitting max iterations returns an
  output instead of failing; wrong basic-auth credentials get 401, not 403.
- Stop and Error ignores `onError`; a sub-workflow refused by its
  `callerPolicy` says it "limits which workflows it can be called by";
  execution context appears in JSON logs only at debug level, under
  `metadata`; `/api/v1/users` returns roles only with `includeRole=true`;
  API keys for members may only carry member scopes.
- Webhook v2 options: `options.responseCode.values.responseCode`, and
  dynamic paths are registered under the node's webhook id
  (`/webhook/<webhookId>/users/:id`).

**n8n 2.x behaviour the spec does not mention, which r8r must match:**
- `execute --file` is gone; `execute --id` runs in `cli` mode, where pin
  data does not apply and a trigger node is required.
- **Publishing.** Sub-workflows and error workflows must be published
  (active) before others can use them; an unpublished error workflow
  silently never runs.
- **Signed resume URLs.** `/webhook-waiting/:id` without its signature
  answers 401.
- Manual runs need `triggerToStartFrom` or `destinationNode {nodeName,
  mode}`; partial runs send `runData` + `dirtyNodeNames`. Pin data comes
  from the *saved* workflow, not the request.
- Invitations are accepted with a signed token (`POST
  /rest/invitations/accept`).
- The public API omits running executions; only `/rest/executions` shows
  them.
- n8n answers `200 "n8n is starting up"` before it is ready, so readiness
  must come from `/healthz/readiness`, not an open port.
- The `n8n-auth` cookie is `Secure` even on plain HTTP.

**Where the spec asks for more than n8n does** (tagged `@beyond-n8n`, 10
scenarios): unknown top-level workflow fields survive export; expressions
have no `setInterval`/`fetch` stubs and a hard time limit; credential values
sent to a webhook are redacted from execution data; an SSRF guard; a 413 for
oversized bodies (n8n answers 500); login rate limiting; invalid
configuration stops the server at boot; an execution counter metric.

## 5. Harness contracts to confirm

These are the suite's defaults where the spec leaves a choice. They are
listed in `tests/bdd/README.md` so the team can confirm or change them:

1. Headless runs use n8n's contract: `import:workflow` then `execute
   --id=<id> --rawOutput`, printing `IRun` JSON after any log lines.
2. Run order comes from `executionIndex`.
3. `R8R_SSRF_ALLOWED_HOSTS` allows private hosts through the SSRF guard.
4. `R8R_QUEUE_BACKEND=postgres` selects the Postgres queue.
5. Processes start from an empty environment plus n8n variable names; the
   current binary's own variables (`PORT`, `JWT_SECRET`, `CREDENTIALS_KEY`,
   `DATABASE_URL`) are passed too until `src/main.rs` reads n8n names.

## 6. Status

| Target | Scenarios run | Passed |
| --- | --- | --- |
| Current r8r (`claude/elegant-archimedes-u5hrus`, 2026-09-25, before Phase 1) | 393 | 0 |
| r8r after the Phase 1 core (`src/n8n`, CLI) | 392 | 258 (phase 1: 222/223) |
| n8n 2.35.7, all n8n-applicable scenarios (excl. `@beyond-n8n`, `@n8n-licensed`, `@r8r-only`, `@perf`, `@slow`, `@requires-*`) | 358 | 358 |

Not run anywhere yet: the opt-in `@perf` (4), `@slow` (1),
`@requires-redis` (3), `@requires-postgres` (1) and
`@requires-python-runner` (1) scenarios, which need reference hardware or
extra services. Their steps are defined and statically checked.

What makes the r8r column move, in order:

1. **CLI subcommands** (`start`, `execute --id`, `import:*`, `export:*`)
   with the n8n workflow JSON model. This unblocks every headless scenario.
2. **n8n auth routes** (`/rest/owner/setup`, `/rest/login`,
   `/rest/api-keys`) and the public API's workflow and execution
   endpoints. This unblocks every server scenario.
3. Then roadmap order: phase 1 engine, expressions and nodes; phase 2
   webhooks, credentials and push; and so on.

`cargo test --test bdd -- --tags @phase-1` is the Phase 1 exit gate in
executable form.
