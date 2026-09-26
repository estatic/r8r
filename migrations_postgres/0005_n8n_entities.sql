-- n8n-compatible entities on PostgreSQL (spec §7.1). Same shape as the
-- SQLite schema: full JSON in text columns, timestamps as ISO-8601 text,
-- flags as integers, so the store runs the same SQL on both.
CREATE TABLE IF NOT EXISTS workflow_entity (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    active BIGINT NOT NULL DEFAULT 0,
    version_id TEXT,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    owner_id TEXT,
    project_id TEXT
);

CREATE TABLE IF NOT EXISTS credentials_entity (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    owner_id TEXT
);

CREATE TABLE IF NOT EXISTS execution_entity (
    id BIGSERIAL PRIMARY KEY,
    workflow_id TEXT,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    finished BIGINT NOT NULL DEFAULT 0,
    retry_of TEXT,
    started_at TEXT NOT NULL,
    stopped_at TEXT,
    wait_till TEXT,
    workflow_data TEXT,
    data TEXT,
    wait_state TEXT,
    parent_execution_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_execution_entity_workflow ON execution_entity (workflow_id, id);
CREATE INDEX IF NOT EXISTS idx_execution_entity_status ON execution_entity (status, wait_till);
