---
type: concept
title: "BDD conformance suite"
tags: [testing, n8n-compatibility, spec]
sources: [n8n-in-rust-reimplementation-spec]
created: 2026-09-25
updated: 2026-09-25
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
listed in `tests/bdd/README.md`: the `execute --rawOutput` output format
and manual-mode semantics, reading run order from `executionIndex`, the
`R8R_SSRF_ALLOWED_HOSTS` and `R8R_QUEUE_BACKEND` settings, and the n8n 1.8x
API-key response shape.

## Status

On 2026-09-25, before any implementation work, 1 of 395 scenarios passed
against the existing code. The existing binary has no CLI subcommands and
none of n8n's `/rest` auth routes, so nearly every scenario stops at its
first step. That is the starting point of the progress bar, not a harness
defect.

## Where this fits

- [[n8n-in-rust-reimplementation-spec]]
