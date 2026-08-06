-- Meeting-type detection and the type -> summary-template mapping.
--
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on
-- re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- WHY A SIDE TABLE RATHER THAN A COLUMN ON `meetings`:
-- adding a column to `meetings` would mean the table-recreation dance, and
-- `meetings` is the parent of four ON DELETE CASCADE children (transcripts,
-- summary_processes, transcript_chunks, meeting_clients) plus
-- meeting_billing_overrides and this table. Recreating it requires
-- PRAGMA foreign_keys=off around a DROP of the parent, which is exactly the
-- shape that silently orphans or cascades away child rows if anything in the
-- copy step is wrong. `clients` could take that risk because it has no
-- children; `meetings` should not. A keyed side table gets the same result with
-- no DROP of a parent table, and it also gives the classification somewhere to
-- record its confidence and provenance.
CREATE TABLE IF NOT EXISTS meeting_types (
    meeting_id TEXT PRIMARY KEY,
    -- discovery | status | planning | incident | review | one_on_one | sales | other
    meeting_type TEXT NOT NULL,
    -- 0.0-1.0 as reported by the model; a manual correction stores 1.0.
    confidence REAL NOT NULL DEFAULT 0,
    -- model | manual. A manual correction is never overwritten by the model.
    source TEXT NOT NULL DEFAULT 'model',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);

-- Which summary template a meeting type should use.
--
-- `client_id` is NOT NULL with '' meaning "the workspace mapping". SQLite treats
-- NULLs in a unique index as distinct, so a nullable client_id would happily
-- accept several conflicting workspace rows for the same type; the empty-string
-- sentinel makes the primary key actually enforce one row per scope.
CREATE TABLE IF NOT EXISTS meeting_type_templates (
    meeting_type TEXT NOT NULL,
    client_id TEXT NOT NULL DEFAULT '',
    template_id TEXT NOT NULL,
    PRIMARY KEY (meeting_type, client_id)
);

CREATE INDEX IF NOT EXISTS idx_meeting_type_templates_client_id
    ON meeting_type_templates(client_id);
