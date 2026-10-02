-- Data Tables (n8n's Data Tables feature): metadata for user-defined
-- tables plus their columns. Each table's rows live in a real SQL table
-- named `data_table_user_<id>` (see src/n8n/data_table.rs), created and
-- dropped dynamically -- there is no migration for those, since their
-- shape is user-defined.
CREATE TABLE IF NOT EXISTS data_table (
    id TEXT PRIMARY KEY,
    -- NULL = not scoped to a team project (personal/no-project scope).
    project_id TEXT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS data_table_project_name ON data_table (COALESCE(project_id, ''), name);

CREATE TABLE IF NOT EXISTS data_table_column (
    id TEXT PRIMARY KEY,
    data_table_id TEXT NOT NULL,
    name TEXT NOT NULL,
    type TEXT NOT NULL,
    col_index INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS data_table_column_table ON data_table_column (data_table_id);
