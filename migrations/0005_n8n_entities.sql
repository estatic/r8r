-- n8n-compatible entities (spec §7.1). The full workflow JSON is stored as
-- received so import/export is lossless; the columns are for lookups.
CREATE TABLE IF NOT EXISTS workflow_entity (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 0,
    version_id TEXT,
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS credentials_entity (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    -- CryptoJS AES blob, readable by n8n with the same N8N_ENCRYPTION_KEY.
    data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS execution_entity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workflow_id TEXT,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    finished INTEGER NOT NULL DEFAULT 0,
    retry_of TEXT,
    started_at TEXT NOT NULL,
    stopped_at TEXT,
    wait_till TEXT,
    workflow_data TEXT,
    data TEXT
);

CREATE INDEX IF NOT EXISTS idx_execution_entity_workflow ON execution_entity (workflow_id, id);
