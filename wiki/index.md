# Wiki Index

The catalog of all pages in this wiki. Each entry: a wikilink to the page and a one-line summary. The LLM reads this first when answering queries to identify candidate pages.

Keep summaries tight — one line each. The index is engineered to be cheap to read; a fat index defeats its purpose.

When this file exceeds ~300 lines or the wiki passes ~150 pages, shard into `wiki/indexes/<type>.md` and replace this file with a directory of shards. See the `scaling-playbook.md` reference in the `llm-wiki` skill for the migration procedure.

---

## Sources

- [[n8n-in-rust-reimplementation-spec]] — target spec for r8r: n8n-compatible Rust rewrite, frozen contracts, goals G1–G6, roadmap.

## Entities

(populated as entity pages are created)

## Concepts

- [[bdd-conformance-suite]] — black-box cucumber scenarios in `tests/bdd/` that define "behaves like n8n"; harness contracts and status.

## Synthesis

(populated as query answers are filed back)
