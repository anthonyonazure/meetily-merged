//! Client-branded deliverables.
//!
//! An exported meeting is the thing a client actually reads, and until now it
//! arrived with no indication of which firm produced it. This module puts the
//! firm's name, logo, footer, and accent colour on the two formats a client is
//! handed (print-styled HTML and DOCX) and removes the app's own marks from them.
//!
//! - `rules.rs` — colour handling, image sniffing, base64, and the `Branding`
//!   value the export paths take. Pure.
//! - `assets.rs` — the owned copy of the logo inside app data.
//! - `commands.rs` — Tauri command surface.
//!
//! Two deliberate boundaries:
//!
//! - **Markdown export stays unbranded.** It is a data format meant for a folder,
//!   a Git repo, or another tool; a firm banner in it is noise.
//! - **Branding is inert until a firm name exists.** With no firm name the export
//!   paths render byte-for-byte as they did before, so shipping this cannot change
//!   anyone's existing exports.

pub mod assets;
pub mod commands;
pub mod rules;
pub mod store;

pub use rules::Branding;
pub use store::{for_export, load};

/// The sample summary used by the Deliverables preview. Real markdown, so the
/// preview exercises the same walker the export does.
pub const SAMPLE_SUMMARY: &str = "## Summary\n\nQuarterly service review with the client's \
operations lead. Ticket volume is down, two escalations remain open, and the \
firewall replacement is scheduled.\n\n## Decisions\n\n- Replace the edge firewall \
in the first week of next month\n- Move backup verification to a weekly report\n\n\
## Action items\n\n1. Send the firewall quote by Friday\n2. Schedule the \
maintenance window with the client's team";

