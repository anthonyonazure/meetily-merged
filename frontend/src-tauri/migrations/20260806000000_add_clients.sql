-- Client Memory: client registry and meeting tagging.
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on re-run.

CREATE TABLE IF NOT EXISTS clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT,
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

-- One row per (meeting, client) link. The app treats this as "a meeting has at
-- most one client" today, but the pair PK keeps the door open for many-to-many.
CREATE TABLE IF NOT EXISTS meeting_clients (
    meeting_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    PRIMARY KEY (meeting_id, client_id),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE,
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_meeting_clients_client_id ON meeting_clients(client_id);
