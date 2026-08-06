-- Network transparency: the app's own record of every outbound HTTP request it
-- made, so privacy can be checked rather than taken on trust.
--
-- New table only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on
-- re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- This table is written by the app's own instrumentation at each call site. It is
-- therefore a record of what the app believes it sent, not an independent capture:
-- the panel that reads it says so, and points the operator at their own network
-- monitor for independent confirmation.
--
-- purpose:  model_download | llm_call | transcription | graph_api | share_webhook
--         | update_check | provider_metadata
-- outcome:  ok | error
CREATE TABLE IF NOT EXISTS network_events (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    session_id TEXT NOT NULL,
    host TEXT NOT NULL,
    -- Scheme, host, and path only. Query strings and request bodies are never
    -- stored: they are exactly where API keys and meeting content would leak into
    -- a log that the operator can export.
    url TEXT NOT NULL,
    method TEXT NOT NULL,
    purpose TEXT NOT NULL,
    outcome TEXT NOT NULL,
    bytes_out INTEGER NOT NULL DEFAULT 0,
    bytes_in INTEGER NOT NULL DEFAULT 0,
    meeting_id TEXT,
    profile_name TEXT,
    -- Whether this request carried recorded audio or transcript text off the
    -- device. This is what a per-meeting answer to "did anything leave?" reads.
    carried_audio INTEGER NOT NULL DEFAULT 0,
    carried_transcript INTEGER NOT NULL DEFAULT 0,
    detail TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_network_events_created_at ON network_events(created_at);
CREATE INDEX IF NOT EXISTS idx_network_events_session_id ON network_events(session_id);
CREATE INDEX IF NOT EXISTS idx_network_events_host ON network_events(host);
CREATE INDEX IF NOT EXISTS idx_network_events_meeting_id ON network_events(meeting_id);
