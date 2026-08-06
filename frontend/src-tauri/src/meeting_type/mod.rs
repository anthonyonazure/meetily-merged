//! Meeting-type detection, and the template it chooses.
//!
//! A discovery call and an incident post-mortem want differently shaped notes, and
//! until now the operator had to remember to pick the right template before every
//! summary. This module classifies the meeting from its own transcript and lets
//! that classification pick the template.
//!
//! - `rules.rs` — the type vocabulary, tolerant parsing of the model's reply, and
//!   the mapping precedence. Pure.
//! - `classify.rs` — the single tight prompt to the already-configured model.
//! - `commands.rs` — Tauri command surface.
//!
//! Three properties worth stating, because they are what make an automatic choice
//! acceptable:
//!
//! - **It never silently overrides a person.** A manual correction is stored with
//!   source `manual` and neither the model nor the repository will overwrite it.
//! - **It never applies a guess.** Below `MIN_CONFIDENCE_FOR_TEMPLATE` the type is
//!   recorded but the caller's requested template stands, and the reason is
//!   reported as `low_confidence` rather than hidden.
//! - **The choice is always visible.** Every path returns a `TemplateChoice`
//!   carrying which template was used and which of the five reasons picked it, so
//!   the UI can say so and offer a regeneration with a different one.

pub mod classify;
pub mod commands;
pub mod rules;

pub use rules::{Classification, MeetingType, TemplateChoice, TemplateChoiceSource};
