-- Local semantic search: on-device embeddings for transcript chunks, summaries,
-- and client memory facts.
--
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on
-- re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- `vector` holds the embedding as raw little-endian f32 bytes (dim * 4 bytes).
-- SQLite has no vector index and this tree adds no extension, so retrieval is a
-- brute-force cosine scan over a candidate set narrowed first by client, meeting,
-- and date in SQL. At a single technician's meeting volume (thousands of chunks,
-- not millions) a full scan of the narrowed set is a few milliseconds, so the
-- honest design is a linear scan rather than pretending an approximate index
-- exists. If a workspace ever outgrows that, the fix is a real vector extension,
-- not a cleverer query here.
--
-- source_kind is one of: transcript_chunk | summary | memory_fact
-- source_id points at the row the text came from (transcripts.id, the meeting id
-- for a summary, memory_facts.id) and is unique per (kind, model) so re-indexing
-- replaces instead of duplicating.
CREATE TABLE IF NOT EXISTS embeddings (
    id TEXT PRIMARY KEY,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    meeting_id TEXT NOT NULL,
    client_id TEXT,
    chunk_text TEXT NOT NULL,
    vector BLOB NOT NULL,
    dim INTEGER NOT NULL,
    model TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_embeddings_source
    ON embeddings(source_kind, source_id, model);
CREATE INDEX IF NOT EXISTS idx_embeddings_meeting_id ON embeddings(meeting_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_client_id ON embeddings(client_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_created_at ON embeddings(created_at);
CREATE INDEX IF NOT EXISTS idx_embeddings_model ON embeddings(model);

-- Single-row settings, mirroring the consent_settings / diarization_settings
-- shape. `enabled` defaults to 0: semantic search needs a ~90 MB model download,
-- so an upgrade never starts one without the operator asking for it. Keyword
-- search keeps working untouched in the meantime.
CREATE TABLE IF NOT EXISTS embeddings_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    enabled INTEGER NOT NULL DEFAULT 0,
    model TEXT NOT NULL DEFAULT 'all-MiniLM-L6-v2',
    top_k INTEGER NOT NULL DEFAULT 12
);

INSERT OR IGNORE INTO embeddings_settings (id, enabled, model, top_k)
VALUES (1, 0, 'all-MiniLM-L6-v2', 12);
