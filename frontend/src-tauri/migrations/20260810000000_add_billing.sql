-- Billable time and meeting cost.
--
-- New tables only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe
-- on re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- Money is deliberately nullable everywhere a rate could be guessed:
-- `default_hourly_rate` and `client_billing.hourly_rate` are both NULL-able so
-- "no rate configured" is a state the app can see and say out loud. A DEFAULT 0
-- here would turn an unanswered question into a $0.00 invoice line.
CREATE TABLE IF NOT EXISTS billing_settings (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    -- NULL means no workspace rate has been set. Never defaulted to 0.
    default_hourly_rate REAL,
    currency TEXT NOT NULL DEFAULT 'USD',
    -- 0 means no rounding. Otherwise minutes are rounded UP to a multiple of N
    -- (15 is the common MSP increment).
    rounding_minutes INTEGER NOT NULL DEFAULT 0,
    -- 0 means no floor. Otherwise any billable meeting bills at least N minutes.
    min_billable_minutes INTEGER NOT NULL DEFAULT 0,
    -- Whether meetings with no client tag ("internal") appear in the report.
    include_internal INTEGER NOT NULL DEFAULT 0
);

INSERT OR IGNORE INTO billing_settings (
    id, default_hourly_rate, currency, rounding_minutes, min_billable_minutes, include_internal
) VALUES (1, NULL, 'USD', 0, 0, 0);

-- Per-client rate and billable flag. A row exists only once the operator has
-- said something about that client; absence means "fall back to the workspace
-- rate, billable".
CREATE TABLE IF NOT EXISTS client_billing (
    client_id TEXT PRIMARY KEY,
    -- NULL means "use the workspace default rate".
    hourly_rate REAL,
    billable INTEGER NOT NULL DEFAULT 1,
    FOREIGN KEY (client_id) REFERENCES clients(id) ON DELETE CASCADE
);

-- Per-meeting corrections. Both fields are nullable, and NULL means "inherit":
-- marking one meeting non-billable or trimming its minutes never touches the
-- transcript, the client's rate, or any other meeting.
CREATE TABLE IF NOT EXISTS meeting_billing_overrides (
    meeting_id TEXT PRIMARY KEY,
    -- NULL inherits the client's billable flag.
    billable INTEGER,
    -- NULL uses the recorded length. A number replaces it entirely (before
    -- rounding), which is how "we only bill 30 of those 50 minutes" is said.
    minutes_override INTEGER,
    note TEXT NOT NULL DEFAULT '',
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
