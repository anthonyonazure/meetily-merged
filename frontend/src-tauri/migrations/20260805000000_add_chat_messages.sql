-- Chat with meetings: stored question/answer history.
-- meeting_id NULL means the message belongs to the "all meetings" scope.
-- New table only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on re-run.

CREATE TABLE IF NOT EXISTS chat_messages (
    id TEXT PRIMARY KEY,
    meeting_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_meeting_id ON chat_messages(meeting_id);
