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

## [2026-09-26] query | PostgreSQL storage (DB_TYPE=postgresdb); BDD 397/397 on SQLite and on PostgreSQL; updated concepts/bdd-conformance-suite

## [2026-09-26] ingest | migrate-from-n8n implemented and validated on real n8n 2.35.7 SQLite and PostgreSQL databases; n8n-style resume tokens; BDD 403/403 on both storages; updated concepts/bdd-conformance-suite

## [2026-09-27] compact | auto compaction (summary text unavailable)

## [2026-10-07] query | Google Gemini, Azure OpenAI and AWS Bedrock chat models (plan 2.3); 10-ai BDD 46/46 on r8r; new features green on n8n 2.35.7

## [2026-10-07] query | AI tools (plan 2.4): $fromAI, native nodes as tools, Code Tool, Workflow Tool; NODES_EXCLUDE covers tool variants; full suite 958/958; new features green on n8n 2.35.7

## [2026-10-07] query | Postgres and Redis chat memory (plan 2.5) in n8n's LangChain storage format, captured from n8n 2.35.7; chat_memory 4/4 on r8r and n8n; full suite 958/958

## [2026-10-07] query | AI chain root nodes (plan 2.6 part): Sentiment Analysis, Text Classifier, Information Extractor with LangChain structured output; prompts captured from n8n 2.35.7; 10-ai 69/69
