# Roadmap — meetily-merged

Owner: Anthony (anthonyonazure). Working doc; Oscar executes, this file tracks order and scope.

## Wave 1 — fork feature parity (in flight)

1. **Speaker diarization** (from mimi202605, audited CLEAN 2026-08-03): on-device CAM++ voiceprint separation, Speaker 1/2/3 labels refining the existing You/Others channel labels, color-coded transcript. Port scope = diarization engine + alignment + trigger + UI; strip Chinese proxy mirrors (official k2-fsa URLs + SHA256 pin), build sherpa libs from source (no bundled DLL blobs), no SenseVoice ASR, no i18n. Voiceprint naming across meetings = Wave 3.
2. **MaxwellJryao features** (audited CLEAN 2026-08-03): Qwen3-ASR engine (pin vendor lib commit; note community GGUF provenance), template management UI, ASR error correction prompts (English examples), detailed-discussion template, OpenAI API transcription, VAD timestamp fix + onboarding deadlock fix if not already covered. Dictation (macOS push-to-talk paste) ports WITH the debug keystroke buffer stripped.

## Wave 2 — PRO-gap features (build ourselves, no fork has them)

3. **Chat with meetings** — SHIPPED 2026-08-03: chat panel in meeting details with "This meeting" and "All meetings" scopes, grounded in transcript+summary via the configured summary LLM. Built as its own module, not a fourth agents-registry entry: the registry models one-shot runs with stored output, chat is interactive multi-turn (decision logged in src-tauri/src/chat/mod.rs).
4. **PDF / DOCX export** — SHIPPED 2026-08-03: DOCX via pure-Rust docx-rs; PDF as print-styled HTML opened in the browser for print-to-PDF (pure-Rust PDF crates rejected for Unicode reasons; rationale in src-tauri/src/export/html.rs).
5. **Calendar integration v1 (local-first)** — SHIPPED 2026-08-03 (macOS): EventKit read-only, Upcoming section in sidebar, meeting-name prefill on click, Join button for Zoom/Teams/Meet/Webex links. TODO not shipped: auto-start prompt (notification at event start) needs a scheduler loop plus dedupe on top of the notifications module and was cut from v1; Windows calendar remains Wave 4 territory.

## Wave 3 — integrations (explicit opt-in, each sends data OUT only on user action)

6. **M365/Outlook** — SHIPPED 2026-08-04: device-code Graph OAuth in Rust (tokens in the OS keychain via `keyring`, transparent refresh), next-24h calendarView merged into the sidebar Upcoming list with per-event source badges (Cal / M365) and title+start dedupe, and "email summary via Outlook" as an explicit share action that creates a DRAFT (never sends) and opens it in Outlook web. Ships with a default Entra registration; other tenants can override client id / tenant in Settings → Integrations. Side effect: M365 is calendar support on Windows, where EventKit doesn't exist — Wave 4's "Windows calendar" is now covered for M365 accounts.
7. **Google Workspace** — stubbed 2026-08-04: Settings → Integrations shows a "coming soon" card that accepts and persists an OAuth client ID, so shipping the integration later is config-plus-code with no settings migration. No Google network code exists yet. Needs a Google Cloud OAuth client (Anthony's action).
8. **Zoom / Teams / Slack**:
   - Detection: meeting auto-detect already catches them (mic-activity + process-name detection once Maxwell port lands).
   - Auto-join — SHIPPED 2026-08-04: a 30s scheduler watches both calendar sources; when an event with a meeting link starts within 2 minutes it fires a notification plus an in-app banner with a Join button (prompt-then-open only, no headless joining). Toggle "Prompt to join from calendar" in Settings → Integrations, default on. This also closes the Wave 2 calendar-v1 TODO (auto-start prompt scheduler + dedupe).
   - Share — SHIPPED 2026-08-04: per-meeting "Send summary to Slack/Teams" via user-supplied incoming webhooks (HTTPS-only, stored in the OS keychain), explicit action per post, nothing automatic.
9. **Voiceprint speaker naming** (mimi Layer B): remember known voices so "Speaker 2" becomes "Dana" automatically in future meetings.

## Wave 4 — platform

10. **Windows (PC) support**: CI build already exists; first dispatch running. Fix what fails; dictation and diarization need Windows-path work (CGEventTap and ScreenCaptureKit equivalents differ).
11. **UI redesign**: four directions mocked (Phosphor terminal, Ledger client-facing, Console Pro dark ops, Slate light dense) — awaiting Anthony's reactions; winner becomes a theme system, ideally shipping 2+ skins since variants share structure.

## Standing rules

- Any fork code merges only after a line-by-line security audit (five forks audited to date, all clean; every fork's auto-updater repoint stripped).
- Privacy contract: processing local or user-configured endpoints only; anything outbound (email, Slack, Teams) is an explicit per-use user action, never automatic.
- Update channel decision still open: own signed feed (key exists in 1Password) vs disabled updater.
