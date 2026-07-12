-- Index the transcripts lookup that every meeting-open performs.
--
-- `SELECT * FROM transcripts WHERE meeting_id = ?` (meeting.rs:78, :146, :154) had no
-- supporting index, so opening any meeting scanned the ENTIRE transcripts table — the
-- cost grew with the total number of transcript segments ever recorded, not with the
-- size of the meeting being opened. A one-hour meeting produces roughly 700-1500
-- VAD segments, so this degrades steadily with use.
CREATE INDEX IF NOT EXISTS idx_transcripts_meeting_id ON transcripts(meeting_id);

-- The meeting list orders by recency; index it so the sidebar does not sort the whole
-- table on every render.
CREATE INDEX IF NOT EXISTS idx_meetings_created_at ON meetings(created_at DESC);
