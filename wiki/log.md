# Wiki Log

Append-only chronological record of operations on the wiki. Each entry begins with `## [YYYY-MM-DD] <op> | <description>` so it's parseable with `grep "^## \[" log.md | tail -N`.

Operations:
- `ingest` — a source was processed into the wiki.
- `query` — a question was answered against the wiki (typically only logged when the answer was filed back as synthesis).
- `lint` — a health check was run.
- `schema` — the schema was modified.
- `shard` — an index was sharded.

---

## [2026-09-25] ingest | n8n in Rust reimplementation spec -> sources/n8n-in-rust-reimplementation-spec, concepts/bdd-conformance-suite
## [2026-09-25] query | validated the BDD suite against n8n 2.35.7; updated concepts/bdd-conformance-suite and sources/n8n-in-rust-reimplementation-spec
## [2026-09-25] query | Phase 1 core implemented; BDD 258/392 (phase 1 222/223); updated concepts/bdd-conformance-suite

## [2026-09-26] compact | auto compaction (summary text unavailable)

## [2026-09-26] query | Phase 2 server implemented; BDD 387/392 (phase 1 223/223, phase 2 123/123); updated concepts/bdd-conformance-suite

## [2026-09-26] query | AI cluster nodes implemented; BDD 392/392 (default run); updated concepts/bdd-conformance-suite

## [2026-09-26] query | Queue mode, Python runner, performance work; BDD 397/397 with opt-ins, @perf 3/4; updated concepts/bdd-conformance-suite
