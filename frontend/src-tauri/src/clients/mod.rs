//! Client Memory: a per-client registry that turns isolated meetings into a
//! running relationship record.
//!
//! - `commands.rs` — Tauri command surface (registry CRUD, meeting tagging,
//!   suggestion, timeline, memory facts, follow-through)
//! - `suggest.rs` — pure suggestion logic (attendee domains, fuzzy titles)
//!
//! All data is local SQLite; the only network touchpoint is the existing
//! Microsoft Graph client (read-only calendar attendees for suggestions and
//! the draft-only chase email flow), both reusing `m365`'s plumbing.

pub mod commands;
pub mod suggest;
