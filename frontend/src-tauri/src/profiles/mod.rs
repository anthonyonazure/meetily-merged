//! Per-client privacy profiles.
//!
//! An MSP runs meetings for clients with very different appetites: one wants
//! nothing to leave the machine and short retention, another wants the most
//! accurate cloud transcription available. Those choices used to be scattered
//! global settings that had to be remembered per meeting. Here they are a named
//! profile attached to a client, so the right policy applies by itself and the
//! consent log shows afterwards which one ran.
//!
//! - `rules.rs` — pure mode/provider/retention logic (the unit-tested core)
//! - `redaction.rs` — the secret matchers, and an honest account of their scope
//! - `resolver.rs` — the single answer to "which profile governs this?"
//! - `gate.rs` — the recording-start enforcement point, beside the consent gate
//! - `enforce.rs` — model, sharing, and redaction helpers for the other paths
//! - `commands.rs` — Tauri command surface
//!
//! Every enforcement point resolves through `resolver`, so transcription,
//! models, consent, and sharing cannot disagree about the policy in
//! force. A profile with nothing resolved is a real answer: on upgrade the
//! workspace default is unset and the app behaves exactly as it did before.

pub mod commands;
pub mod enforce;
pub mod gate;
pub mod redaction;
pub mod resolver;
pub mod rules;
