#!/usr/bin/env bash
set -euo pipefail
cd "/home/estatic/Documents/Projects/rust/r8r"

PROMPT=$(cat <<'PROMPT_EOF'
You are running as an unattended daily cron job for this project's LLM Wiki (the Karpathy "LLM Wiki" pattern) at the current working directory. Read wiki/SCHEMA.md first for conventions.

Steps:
1. Determine today's date (YYYY-MM-DD).
2. Scan wiki/log.md for all `## [<today's date>] ...` entries (any operation type -- compact, ingest, query, etc.).
3. If there are no entries for today, do nothing further and stop -- do not create an empty page or log entry.
4. If there are entries, write ONE dated rollup page at wiki/synthesis/<today's date>-daily-rollup.md, following wiki/SCHEMA.md's frontmatter and page conventions (type: synthesis, title, tags, created, updated, sources -- sources should note this covers today's log.md entries). Keep it concise: what happened, key content/decisions added, not a verbatim copy of the log entries.
5. Update wiki/index.md with a one-line entry for the new page, matching its existing format.
6. Append a `## [<today's date>] digest | Daily rollup filed` entry to wiki/log.md's chronological record. If wiki/log.md's Operations list at the top doesn't yet document a `digest` operation type, add a one-line description of it there too (co-evolving the schema, per the wiki's own convention).

Constraints: only touch files under wiki/. Do not run git commands, do not make commits, do not touch anything under src/, docs/, or other project directories.
PROMPT_EOF
)

exec /home/estatic/.local/bin/claude -p "$PROMPT" \
  --permission-mode acceptEdits \
  --allowedTools "Read,Write,Edit,Glob,Grep"
