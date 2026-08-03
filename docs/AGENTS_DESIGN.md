# Meeting Agents — on-device AI agent library (v1 design)

Turns upstream's roadmap teaser ("a library of on-device AI agents — automating
follow-ups, action tracking, and more") into a shipped feature. Everything runs
locally through the LLM provider the user already configured for summaries
(built-in AI, Ollama, or their own API keys). No agent ever sends anything
anywhere: outputs are drafts and local records only.

## Architecture

**Registry pattern.** Built-in agents are declared in one Rust registry;
adding an agent means adding one registry entry (id, name, description,
prompt builder, output kind, auto-run default). The frontend renders whatever
the registry reports — no per-agent UI wiring.

**Data model** (sqlx migrations, table-recreation pattern per repo convention):

- `agent_runs`: id, agent_id, meeting_id, status (running/completed/error),
  output_md, error, created_at
- `action_items`: id, meeting_id, agent_run_id, description, owner (nullable),
  due_hint (nullable free text), status (open/done), created_at, updated_at

**Rust module** `frontend/src-tauri/src/agents/`:

- `registry.rs` — agent definitions
- `runner.rs` — builds transcript+summary context, calls the existing
  summary LLM plumbing (`summary::llm_client`), parses output, persists
- `commands.rs` — Tauri commands: `agents_list`, `agent_run`,
  `agent_runs_for_meeting`, `agents_get_settings`, `agents_set_enabled`,
  `actions_list`, `actions_for_meeting`, `action_set_status`, `action_delete`

Every new command is registered in `generate_handler!`, the generated
allowlist, and `permissions/main-window.toml` — the security contract test
enforces the three stay in sync.

## v1 agents

1. **Follow-up Drafter** (`followup_drafter`, manual): drafts a follow-up
   email in markdown from transcript + summary — recap, commitments made by
   "You", asks of others, proposed next steps. Output shown with a copy
   button. Never sends.
2. **Action Tracker** (`action_tracker`, auto after summary completes):
   extracts action items as structured JSON (description, owner, due hint),
   tolerant parsing (code-fence stripping; on parse failure stores raw output
   as an agent run instead of dropping it), inserts open items into
   `action_items`.
3. **Decision Log** (`decision_log`, manual): extracts decisions with a
   one-line context and who drove each; stored per meeting.

## Frontend

- **Meeting details → "Agents" panel**: list registry agents with Run button,
  status, and rendered markdown output per run; copy button on outputs.
- **Sidebar → "Actions" view**: all open action items across meetings,
  grouped by meeting, checkbox to mark done, link back to the meeting.
- **Settings**: per-agent enable/auto-run toggles (stored via existing store
  plugin patterns).

## Privacy contract

Agents inherit the app's privacy posture: local processing, user-configured
LLM endpoint only, no network calls of their own, no telemetry. A future
"send email" would require an explicit user action outside agent scope.
