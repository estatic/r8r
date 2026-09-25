---
type: source
title: "n8n in Rust — Analysis, Review & Reimplementation Spec"
tags: [spec, n8n-compatibility, architecture]
authors: [Ivan Grunev]
url: ""
raw: raw/2026-09-25-n8n-in-rust-reimplementation-spec.md
ingested: 2026-09-25
created: 2026-09-25
updated: 2026-09-25
---

# n8n in Rust — Analysis, Review & Reimplementation Spec

The target specification for r8r (called "r8n" in the document): a Rust
reimplementation of n8n that stays behaviourally compatible with it. Parity
is defined by a conformance corpus, not by translating n8n's code line by
line. r8r's executable version of that corpus is the
[[bdd-conformance-suite]].

## Key claims

- **Frozen contracts** (§3.3, §4.3): workflow JSON, node type descriptions
  (`/types/nodes.json`), expression results, `runData` shape, webhook URLs
  (`/webhook`, `/webhook-test`, `/webhook-waiting`, `/form`), credential
  encryption, and the `/rest` + `/api/v1` APIs. Everything behind them may
  change.
- **Goals G1–G6** (§4.1): workflow, API, data and deployment compatibility
  with n8n 2.x; performance targets; no user code or expression reaching the
  host except through declared capabilities.
- **Execution semantics** (§2.4, §6.2): items with `pairedItem`, per-node
  retry and `onError`, execution order v1 (depth-first, canvas order) and
  legacy v0, pin data, partial execution, Wait/resume, sub-workflows with
  `callerPolicy`, cancellation and `executionTimeout`.
- **Expressions** (§6.4): `=` prefix plus `{{ }}` blocks in QuickJS with
  n8n's data proxy, extension methods and Luxon, a Function-constructor-proof
  sandbox and a 1 s default limit.
- **Credentials** (§6.5): read n8n's CryptoJS AES format with the same
  `N8N_ENCRYPTION_KEY`; OAuth1/2 with refresh on 401.
- **Scaling** (§7.3): queue mode on Redis Streams or Postgres `SKIP
  LOCKED`, with leased jobs so a dead worker's job is re-queued (n8n 2.0
  dropped stalled-job retry).
- **Non-functional targets** (§8.1): ≤ 60 MB idle RSS, ≤ 1 s cold start,
  webhook p99 ≤ 15 ms at 500 rps. The spec labels these proposed, to confirm
  against an n8n baseline in Phase 0.
- **Roadmap** (§9.1): Phase 0 baseline, then 1 core engine, 2 server, 3 beta
  (runner, bridge, Postgres, migration), 4 scale, 5 GA.

## Gaps and ambiguities found while turning it into tests

The spec says *what* must match n8n but leaves several contracts open. The
[[bdd-conformance-suite]] pins these down, and they need a team decision:

- The headless CLI shape (`execute --file`, output format, whether pin data
  applies) is only named ("CLI `execute`"), not specified.
- The SSRF guard's allow-list setting has no name, and neither does the
  switch between the Redis and Postgres queue backends.
- The "rename propagation into expressions" MUST (§6.1) has no API surface
  in n8n (the editor does it), so it can't be tested black-box yet.
- Node parameter shapes and `typeVersion`s are implied by "n8n 2.x" but no
  version is pinned (open question §9.4).
- The timeout status (`canceled` vs `error`) and several n8n error
  messages are not stated.

## Where this fits

- [[bdd-conformance-suite]]
