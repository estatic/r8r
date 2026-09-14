#!/usr/bin/env python3
"""PostCompact hook: append the compaction summary to wiki/log.md.

Reads the hook's stdin JSON (session_id, transcript_path, trigger, ...),
locates the compaction-summary message in the transcript (flagged
isCompactSummary: true), and appends a `## [DATE] compact | ...` entry to
wiki/log.md following this project's llm-wiki log convention.

Defensive by design: never raises, never blocks the session. If the wiki
doesn't exist, or the transcript can't be parsed, it no-ops or falls back
to a metadata-only entry rather than failing loudly.
"""
import datetime
import json
import os
import sys

MAX_SUMMARY_CHARS = 4000
TAIL_LINES_TO_SCAN = 500


def find_project_root(payload):
    for key in ("cwd", "project_dir"):
        value = payload.get(key)
        if value and os.path.isdir(value):
            return value
    env_root = os.environ.get("CLAUDE_PROJECT_DIR")
    if env_root and os.path.isdir(env_root):
        return env_root
    return os.getcwd()


def extract_summary_and_trigger(transcript_path):
    trigger = None
    summary_text = None
    if not transcript_path or not os.path.isfile(transcript_path):
        return trigger, summary_text
    try:
        with open(transcript_path, "r", encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
    except OSError:
        return trigger, summary_text

    for line in reversed(lines[-TAIL_LINES_TO_SCAN:]):
        line = line.strip()
        if not line:
            continue
        try:
            entry = json.loads(line)
        except (json.JSONDecodeError, ValueError):
            continue

        if trigger is None and entry.get("type") == "system" and entry.get("subtype") == "compact_boundary":
            meta = entry.get("compact_metadata") or entry.get("compactMetadata") or {}
            trigger = meta.get("trigger")

        is_summary = entry.get("isCompactSummary") is True
        message = entry.get("message")
        if not is_summary and isinstance(message, dict):
            is_summary = message.get("isCompactSummary") is True

        if is_summary and summary_text is None and isinstance(message, dict):
            content = message.get("content")
            parts = []
            if isinstance(content, str):
                parts.append(content)
            elif isinstance(content, list):
                for chunk in content:
                    if isinstance(chunk, dict) and chunk.get("type") == "text":
                        text = chunk.get("text", "")
                        if text:
                            parts.append(text)
            if parts:
                summary_text = "\n".join(parts).strip()

        if trigger is not None and summary_text is not None:
            break

    return trigger, summary_text


def main():
    try:
        raw = sys.stdin.read()
        payload = json.loads(raw) if raw.strip() else {}
    except (json.JSONDecodeError, ValueError):
        payload = {}

    project_root = find_project_root(payload)
    log_path = os.path.join(project_root, "wiki", "log.md")
    if not os.path.isfile(log_path):
        return  # no wiki in this project (or wrong cwd) -- silently do nothing

    trigger, summary_text = extract_summary_and_trigger(payload.get("transcript_path"))
    trigger = trigger or payload.get("trigger") or "unknown"

    date_str = datetime.date.today().isoformat()

    if summary_text:
        body = summary_text
        if len(body) > MAX_SUMMARY_CHARS:
            body = body[:MAX_SUMMARY_CHARS].rstrip() + "\n\n…(truncated)"
        entry_md = f"\n## [{date_str}] compact | {trigger} compaction\n\n{body}\n"
    else:
        entry_md = f"\n## [{date_str}] compact | {trigger} compaction (summary text unavailable)\n"

    try:
        with open(log_path, "a", encoding="utf-8") as f:
            f.write(entry_md)
    except OSError:
        pass


if __name__ == "__main__":
    main()
