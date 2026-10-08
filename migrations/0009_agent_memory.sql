-- AI Agent chat memory: the last exchanges of each conversation, by key
-- ("<workflow>:<node>:<session>").
CREATE TABLE agent_memory (
    key TEXT PRIMARY KEY,
    messages TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
