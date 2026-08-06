-- Privacy profiles: a named bundle of processing rules that can be attached to
-- a client, so the same choices do not have to be remembered per meeting.
--
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe
-- on re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- transcription_mode / llm_mode: local_only | cloud_allowed
-- consent_level:                 self_only | notify | affirmative | per_speaker
-- consent_enforcement:           flag_only | strict
-- retention_days:                NULL means kept indefinitely
CREATE TABLE IF NOT EXISTS privacy_profiles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    transcription_mode TEXT NOT NULL DEFAULT 'local_only',
    llm_mode TEXT NOT NULL DEFAULT 'local_only',
    consent_level TEXT NOT NULL DEFAULT 'self_only',
    consent_enforcement TEXT NOT NULL DEFAULT 'flag_only',
    retention_days INTEGER,
    redact_pii INTEGER NOT NULL DEFAULT 0,
    allow_sharing INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    is_builtin INTEGER NOT NULL DEFAULT 0
);

-- The three shipped profiles. INSERT OR IGNORE so a re-run never overwrites an
-- operator's renamed copy of a built-in.
INSERT OR IGNORE INTO privacy_profiles (
    id, name, description, transcription_mode, llm_mode, consent_level,
    consent_enforcement, retention_days, redact_pii, allow_sharing,
    created_at, updated_at, is_builtin
) VALUES (
    'profile-builtin-strict',
    'Strict',
    'Nothing leaves this machine: local transcription, local models, every speaker confirmed, recordings and notes removed after 90 days.',
    'local_only', 'local_only', 'per_speaker', 'strict', 90, 1, 0,
    '2026-08-09T00:00:00Z', '2026-08-09T00:00:00Z', 1
);

INSERT OR IGNORE INTO privacy_profiles (
    id, name, description, transcription_mode, llm_mode, consent_level,
    consent_enforcement, retention_days, redact_pii, allow_sharing,
    created_at, updated_at, is_builtin
) VALUES (
    'profile-builtin-standard',
    'Standard',
    'Local transcription and local models, the room is told a recording is running, and meetings are kept for a year.',
    'local_only', 'local_only', 'notify', 'flag_only', 365, 0, 1,
    '2026-08-09T00:00:00Z', '2026-08-09T00:00:00Z', 1
);

INSERT OR IGNORE INTO privacy_profiles (
    id, name, description, transcription_mode, llm_mode, consent_level,
    consent_enforcement, retention_days, redact_pii, allow_sharing,
    created_at, updated_at, is_builtin
) VALUES (
    'profile-builtin-open',
    'Open',
    'Cloud transcription and cloud models allowed for accuracy, no announcement, nothing removed on a schedule.',
    'cloud_allowed', 'cloud_allowed', 'self_only', 'flag_only', NULL, 0, 1,
    '2026-08-09T00:00:00Z', '2026-08-09T00:00:00Z', 1
);

-- Workspace-level privacy settings, mirroring the consent_settings shape.
--
-- default_profile_id is deliberately NULL on upgrade: an existing install keeps
-- behaving exactly as it did (global transcription, model, and consent settings
-- govern) until the operator picks a workspace default. A non-NULL default here
-- would silently change transcription and consent behaviour on first launch.
--
-- retention_dry_run defaults to 1 so the first release cannot delete anything,
-- and retention_armed_at stays NULL until the operator explicitly turns dry run
-- off — the background purge requires both.
CREATE TABLE IF NOT EXISTS privacy_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    default_profile_id TEXT,
    retention_dry_run INTEGER NOT NULL DEFAULT 1,
    retention_armed_at TEXT,
    retention_last_run_at TEXT
);

INSERT OR IGNORE INTO privacy_settings (
    id, default_profile_id, retention_dry_run, retention_armed_at, retention_last_run_at
) VALUES (1, NULL, 1, NULL, NULL);
