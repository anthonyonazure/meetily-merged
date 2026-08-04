-- Client Memory: structured facts (commitments, decisions, figures, notes)
-- extracted from meetings by the memory_extractor agent.
-- client_id is denormalized from the meeting's client tag at extraction time.
-- status lifecycle applies to commitments (open/done/dismissed); other kinds
-- use 'na'.
-- New table only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on re-run.

CREATE TABLE IF NOT EXISTS memory_facts (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    client_id TEXT,
    agent_run_id TEXT,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail TEXT NOT NULL,
    owner TEXT,
    due_hint TEXT,
    amount TEXT,
    status TEXT NOT NULL DEFAULT 'na',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_facts_meeting_id ON memory_facts(meeting_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_client_id ON memory_facts(client_id);
CREATE INDEX IF NOT EXISTS idx_memory_facts_status ON memory_facts(status);
