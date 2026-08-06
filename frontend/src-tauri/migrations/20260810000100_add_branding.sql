-- Client-branded deliverables: the firm's name, logo, footer, and accent colour
-- applied to the exports a client actually sees (print-HTML and DOCX).
--
-- New table only, so plain CREATE TABLE IF NOT EXISTS is idempotent and safe on
-- re-run (no ALTER TABLE, per the migration rules in CLAUDE.md).
--
-- firm_name defaults to empty, which is what makes this feature inert on
-- upgrade: with no firm name configured the export paths render exactly as they
-- did before. `logo_path` points at a copy inside the app data directory, never
-- at the file the user originally picked (that path can move or vanish).
CREATE TABLE IF NOT EXISTS branding (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    firm_name TEXT NOT NULL DEFAULT '',
    logo_path TEXT,
    footer_text TEXT NOT NULL DEFAULT '',
    accent_hex TEXT NOT NULL DEFAULT '#23252b',
    include_logo INTEGER NOT NULL DEFAULT 1,
    include_footer INTEGER NOT NULL DEFAULT 1
);

INSERT OR IGNORE INTO branding (
    id, firm_name, logo_path, footer_text, accent_hex, include_logo, include_footer
) VALUES (1, '', NULL, '', '#23252b', 1, 1);
