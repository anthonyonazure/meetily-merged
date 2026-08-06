-- Attach a privacy profile to a client.
--
-- Table-recreation pattern (never a bare ALTER TABLE ADD COLUMN, per CLAUDE.md):
-- CREATE IF NOT EXISTS _new -> INSERT OR IGNORE -> DROP -> RENAME. Re-running
-- the file is safe because the copy is INSERT OR IGNORE and the drop/rename pair
-- always leaves exactly one `clients` table.
--
-- meeting_clients and memory_facts reference clients(id); foreign keys are
-- switched off for the swap so the DROP cannot cascade those rows away.
PRAGMA foreign_keys=off;

CREATE TABLE IF NOT EXISTS clients_new (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT,
    notes TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    privacy_profile_id TEXT
);

INSERT OR IGNORE INTO clients_new (id, name, domain, notes, created_at, privacy_profile_id)
SELECT id, name, domain, notes, created_at, NULL FROM clients;

DROP TABLE clients;

ALTER TABLE clients_new RENAME TO clients;

CREATE INDEX IF NOT EXISTS idx_clients_privacy_profile_id ON clients(privacy_profile_id);

PRAGMA foreign_keys=on;
