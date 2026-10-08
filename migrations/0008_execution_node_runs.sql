-- Per-node run status and item counts per output, for the editor's canvas.
ALTER TABLE executions ADD COLUMN node_runs TEXT NOT NULL DEFAULT '{}';
