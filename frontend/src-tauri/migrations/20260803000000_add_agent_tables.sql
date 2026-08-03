-- Meeting Agents: agent run history, extracted action items, and per-agent settings.
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on re-run.

CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    status TEXT NOT NULL,
    output_md TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_agent_runs_meeting_id ON agent_runs(meeting_id);

CREATE TABLE IF NOT EXISTS action_items (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    agent_run_id TEXT,
    description TEXT NOT NULL,
    owner TEXT,
    due_hint TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_action_items_meeting_id ON action_items(meeting_id);
CREATE INDEX IF NOT EXISTS idx_action_items_status ON action_items(status);

CREATE TABLE IF NOT EXISTS agent_settings (
    agent_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    auto_run INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);
