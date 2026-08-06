//! Billable time and meeting cost.
//!
//! An MSP bills clients by the hour, and the recording of the meeting is already
//! the most accurate record of how long that meeting took. This module turns that
//! record into an invoice line, and separately into an honest estimate of what
//! the meeting cost the firm internally.
//!
//! - `rules.rs` — the money arithmetic: rate resolution, rounding, minimums,
//!   amounts, and the cost estimate. Pure and exhaustively tested.
//! - `duration.rs` — where a meeting's length actually comes from, with the
//!   answer labelled by source.
//! - `report.rs` — rows, the date-range filter, and totals.
//! - `export.rs` — CSV and invoice-ready Markdown.
//! - `commands.rs` — Tauri command surface and input validation.
//!
//! The one invariant worth stating twice: a missing rate is never a zero. It
//! travels as `None` from the column, through the arithmetic, into the row, and
//! out to the UI and the exports as "no rate set", and those rows are counted and
//! named rather than silently summed into a smaller total.

pub mod commands;
pub mod duration;
pub mod export;
pub mod report;
pub mod rules;
