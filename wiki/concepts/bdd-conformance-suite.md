---
type: concept
title: "BDD conformance suite"
tags: [testing, n8n-compatibility, spec]
sources: [n8n-in-rust-reimplementation-spec]
created: 2026-09-25
updated: 2026-09-26
---

# BDD conformance suite

r8r's executable definition of "behaves like n8n": Gherkin scenarios run by
cucumber-rs under `tests/bdd/`, written from the
[[n8n-in-rust-reimplementation-spec]] before the engine is rebuilt. It plays
the role of the spec's "conformance corpus" (§3.3 lesson 5, §8.5).

## Design choices

- **Black-box.** Steps spawn the `r8r` binary as a CLI (`execute --file`,
  `import:*`, `export:*`, `config check`) or as a server (`start`), and
  talk to it over n8n's own contracts: workflow JSON, `/rest`, `/api/v1`,
  `/webhook*`, `/form`, `/rest/push`, `/healthz`. No step imports crate
  internals, so the suite survives the planned split into a Cargo workspace.
- **n8n JSON everywhere.** Workflows are built as n8n workflow JSON with
  real n8n node types and `typeVersion`s, so the same scenarios can be run
  against n8n itself (`R8R_BIN`) to check the expectations.
- **Isolation.** Each scenario gets its own temporary `N8N_USER_FOLDER`,
  free port and processes; external services are wiremock fakes (HTTP APIs,
  OAuth2 token endpoints, an OpenAI-compatible chat API).
- **Traceability.** Tags name the spec section (`@spec-6.2`), roadmap phase
  (`@phase-1`…`@phase-5`) and node; infrastructure-heavy or timing-sensitive
  scenarios are opt-in (`@perf`, `@slow`, `@requires-redis`, ...).

## Harness contracts it pins down

Where the spec is silent the suite had to choose, and the choices are
listed in `tests/bdd/README.md`: headless runs use n8n 2.x's `import:workflow`
+ `execute --id --rawOutput`, run order comes from `executionIndex`, the
`R8R_SSRF_ALLOWED_HOSTS` and `R8R_QUEUE_BACKEND` settings, and servers
count as ready only once `/healthz/readiness` settles.

## Validation against n8n

The same scenarios were run against real n8n 2.35.7 (`R8R_BIN`). After
correcting the expectations n8n disproved, n8n passes all 358 scenarios it
should. The rest are tagged `@beyond-n8n` (the spec asks for more than n8n
does), `@n8n-licensed` or `@r8r-only`, or are opt-in. The run also
surfaced n8n 2.x behaviour the
[[n8n-in-rust-reimplementation-spec]] does not mention: publishing
sub-workflows and error workflows, signed resume URLs, the 2.x manual-run
payload, token-based invitations, v0 order for workflows without
`executionOrder`, and `execute --id` in cli mode without pin data. Details
are in `docs/superpowers/specs/2026-09-25-r8r-bdd-conformance-suite-design.md`.

## Status

On 2026-09-25, before any implementation work, 0 of 392 scenarios passed
against the existing code. The existing binary has no CLI subcommands and
none of n8n's `/rest` auth routes, so nearly every scenario stops at its
first step. That is the starting point of the progress bar, not a harness
defect.

After the Phase 1 core landed (`src/n8n`: engine, expressions, nodes,
credentials, CLI), 258 of 392 pass, including 222 of 223 phase-1
scenarios. The rest need the Phase 2 server work.

After the Phase 2 server landed (`src/n8n/server`: editor REST API and
`n8n-auth` sessions, public API `/api/v1` with scoped keys, webhooks with
response modes and auth, forms, signed resume URLs, schedules, push,
error workflows, sub-workflows, save policies, crash recovery, graceful
shutdown, health and metrics), 387 of 392 pass: every phase-1 and
phase-2 scenario. The five left are the AI-node scenarios.

## Where this fits

- [[n8n-in-rust-reimplementation-spec]]
