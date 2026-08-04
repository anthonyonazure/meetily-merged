-- Chat client scope: add client_id to chat_messages so a conversation can be
-- scoped to one client (meeting_id NULL + client_id NULL remains the
-- "all meetings" thread; exactly one of the two is set otherwise).
-- Table-recreation pattern per repo convention (idempotent, safe on re-run).

PRAGMA foreign_keys=off;

CREATE TABLE IF NOT EXISTS chat_messages_new (
    id TEXT PRIMARY KEY,
    meeting_id TEXT,
    client_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO chat_messages_new (id, meeting_id, role, content, created_at)
SELECT id, meeting_id, role, content, created_at
FROM chat_messages;

DROP TABLE chat_messages;

ALTER TABLE chat_messages_new RENAME TO chat_messages;

CREATE INDEX IF NOT EXISTS idx_chat_messages_meeting_id ON chat_messages(meeting_id);
CREATE INDEX IF NOT EXISTS idx_chat_messages_client_id ON chat_messages(client_id);

PRAGMA foreign_keys=on;
