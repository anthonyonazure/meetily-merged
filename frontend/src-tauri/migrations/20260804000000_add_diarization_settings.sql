-- Migration: Speaker diarization settings
-- Single-row table holding the on/off toggle for the post-recording speaker
-- diarization pass (default ON). Idempotent by construction: CREATE TABLE IF
-- NOT EXISTS + INSERT OR IGNORE, no ALTER on existing tables.

CREATE TABLE IF NOT EXISTS diarization_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 1
);

INSERT OR IGNORE INTO diarization_settings (id, enabled) VALUES (1, 1);
