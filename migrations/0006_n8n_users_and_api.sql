-- Users, API keys, tags, variables and projects for the n8n-compatible
-- server (spec §6.9, §6.10, §7.1).
CREATE TABLE IF NOT EXISTS user (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    first_name TEXT,
    last_name TEXT,
    -- bcrypt, as n8n stores it; NULL while an invitation is pending.
    password TEXT,
    role TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_api_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    label TEXT NOT NULL,
    -- SHA-256 of the key; the key itself is shown once.
    api_key_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at INTEGER
);

CREATE TABLE IF NOT EXISTS tag_entity (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workflows_tags (
    workflow_id TEXT NOT NULL,
    tag_id TEXT NOT NULL,
    PRIMARY KEY (workflow_id, tag_id)
);

CREATE TABLE IF NOT EXISTS variables (
    id TEXT PRIMARY KEY,
    key TEXT NOT NULL UNIQUE,
    value TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'string'
);

CREATE TABLE IF NOT EXISTS project (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS project_relation (
    project_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (project_id, user_id)
);

ALTER TABLE workflow_entity ADD COLUMN owner_id TEXT;
ALTER TABLE workflow_entity ADD COLUMN project_id TEXT;
ALTER TABLE credentials_entity ADD COLUMN owner_id TEXT;

-- Paused executions keep where they continue (node, input, source) here.
ALTER TABLE execution_entity ADD COLUMN wait_state TEXT;
ALTER TABLE execution_entity ADD COLUMN parent_execution_id TEXT;
