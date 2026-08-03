//! Meeting Agents: an on-device library of AI agents (follow-up drafting,
//! action tracking, decision logging) that run against a meeting's transcript
//! and summary through the user's already-configured summary LLM provider.
//!
//! - `registry` declares the built-in agents (one entry per agent).
//! - `runner` builds context, calls `summary::llm_client`, parses, persists.
//! - `commands` exposes the Tauri command surface.
//!
//! Privacy contract: agents reuse the summary LLM plumbing only. They add no
//! network endpoints and no telemetry; outputs are drafts and local records.

pub mod commands;
pub mod registry;
pub mod runner;
