//! Chat with meetings: ask questions about one meeting (or, in "all meetings"
//! mode, across recent meetings) and get answers grounded in the stored
//! transcript and summary.
//!
//! The feature reuses the summary LLM plumbing (`summary::llm_client`) with
//! provider settings resolved exactly like the Meeting Agents runner does. It
//! adds no network endpoints of its own; every request goes to the provider
//! the user already configured for summaries.
//!
//! Chat is deliberately NOT an entry in the agents registry: the registry
//! models one-shot runs with a stored markdown output per meeting, while chat
//! is an interactive, multi-turn conversation with its own history table.

pub mod commands;
pub mod service;
