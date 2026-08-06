//! Local semantic search.
//!
//! Cross-meeting recall used to be literal: "when did we agree that deadline"
//! found nothing if the client said "let's lock the go-live". This module adds
//! meaning-based retrieval that runs entirely on this machine — a small sentence
//! encoder (see `model`) turns passages into vectors, `store` keeps them in
//! SQLite, and `search` merges vector hits with the existing word matches.
//!
//! Layout:
//! * `tokenizer`, `vector`, `chunk` — pure logic, no I/O, unit tested on their own.
//! * `model` — model download, verification, and ONNX inference.
//! * `store`, `settings` — persistence.
//! * `index` — building the index (after a summary, or on demand).
//! * `search` — hybrid retrieval and prompt-context retrieval.
//! * `commands` — the Tauri surface.

pub mod chunk;
pub mod commands;
pub mod index;
pub mod model;
pub mod search;
pub mod settings;
pub mod store;
pub mod tokenizer;
pub mod vector;
