-- Recording Consent: operator-configurable consent level plus a permanent,
-- append-only record of what was done before and during each recording.
--
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe
-- on re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).

-- Single-row global settings, mirroring the diarization_settings shape.
-- `consent_level` is one of self_only | notify | affirmative | per_speaker.
-- `per_speaker_enforcement` is flag_only | strict.
-- Keyword and domain lists are comma-separated; an empty string means "none".
CREATE TABLE IF NOT EXISTS consent_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    consent_level TEXT NOT NULL DEFAULT 'self_only',
    per_speaker_enforcement TEXT NOT NULL DEFAULT 'flag_only',
    spoken_announcement_enabled INTEGER NOT NULL DEFAULT 0,
    announcement_text TEXT NOT NULL DEFAULT '',
    disclaimer_text TEXT NOT NULL DEFAULT '',
    blocked_title_keywords TEXT NOT NULL DEFAULT '',
    blocked_domains TEXT NOT NULL DEFAULT ''
);

INSERT OR IGNORE INTO consent_settings (
    id,
    consent_level,
    per_speaker_enforcement,
    spoken_announcement_enabled,
    announcement_text,
    disclaimer_text,
    blocked_title_keywords,
    blocked_domains
) VALUES (
    1,
    'self_only',
    'flag_only',
    0,
    'This meeting is being transcribed for notes. Please say so now if you object.',
    'Heads up: I am transcribing this meeting so I have accurate notes. Let me know if you would rather I did not.',
    'HR,legal,board,review,termination,disciplinary,therapy,medical,privileged',
    ''
);

-- The consent log. APPEND-ONLY by contract: rows are never UPDATEd or DELETEd,
-- and a correction is a new row. `meeting_id` holds either a real meetings.id
-- or, for anything logged before the meeting row exists, the pre-recording
-- consent session id (see consent_session_meetings below).
--
-- event_type: self | notice_given | attendee_confirmed | attendee_declined
--           | speaker_confirmed | speaker_declined | recording_blocked
--           | level_overridden
-- method:    chat_paste | spoken_announcement | verbal | in_person | other
CREATE TABLE IF NOT EXISTS consent_events (
    id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    level TEXT NOT NULL,
    event_type TEXT NOT NULL,
    subject TEXT,
    method TEXT,
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_consent_events_meeting_id ON consent_events(meeting_id);
CREATE INDEX IF NOT EXISTS idx_consent_events_created_at ON consent_events(created_at);

-- Bridge between a pre-recording consent session and the meeting row that the
-- recording eventually produced (meetings.id is only minted at save time, well
-- after consent is collected). Insert-only, one row per session, so the
-- consent_events append-only invariant is never violated to re-key a log.
CREATE TABLE IF NOT EXISTS consent_session_meetings (
    session_id TEXT PRIMARY KEY,
    meeting_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_consent_session_meetings_meeting_id
    ON consent_session_meetings(meeting_id);
