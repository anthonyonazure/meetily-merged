//! Recording Consent.
//!
//! The app captures audio locally with no bot in the meeting, so nothing in the
//! call itself signals that a recording is running. This module makes consent an
//! explicit, operator-configurable, permanently-logged step of every recording
//! instead of an assumption.
//!
//! - `rules.rs` — pure level and blocking-rule logic (the unit-tested core)
//! - `settings.rs` — typed view over the single settings row
//! - `gate.rs` — the one enforcement point on the recording start path
//! - `filter.rs` — strict-mode withholding for unconsented speakers
//! - `announce.rs` — spoken announcement through the current output device
//! - `export.rs` — CSV and Markdown rendering of the log
//! - `commands.rs` — Tauri command surface
//!
//! The log (`consent_events`) is append-only: rows are never updated or
//! deleted, and a correction is a new row, so the record reflects what the
//! operator actually did at the time.

pub mod announce;
pub mod commands;
pub mod export;
pub mod filter;
pub mod gate;
pub mod rules;
pub mod settings;
