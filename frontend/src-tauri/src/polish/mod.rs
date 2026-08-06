//! Output polish: making the transcript copy that leaves the app read like
//! writing instead of like a recording.
//!
//! Research on the local-transcription tier found the most-cited complaint is
//! formatting, not accuracy. Whisper hears correctly and then writes down every
//! "um", every restarted word, and every number as it was spoken. That is right
//! for an archive and wrong for a summary or a client deliverable.
//!
//! - `filler.rs` — fillers, discourse phrases, and stutters
//! - `numbers.rs` — spoken quantities to digits
//! - `datetime.rs` — spoken and inconsistent dates and times to one form
//!
//! # What this does NOT do
//!
//! **It never modifies the stored transcript.** `transcripts` in SQLite keeps the
//! original words. Polish is applied at the two consumption boundaries (the copy
//! handed to the summary model, and the copy written into an export), exactly like
//! the consent filter and the profile redaction that already sit there. Three
//! consequences worth stating:
//!
//! - Playback stays aligned: audio sync uses the stored segments, which are
//!   untouched.
//! - Improving the polish rules improves every past meeting on the next export,
//!   because each export re-derives from the original words.
//! - Nothing is lost. Any transformation this module gets wrong is recoverable by
//!   reading the transcript in the app.
//!
//! It also does not paraphrase, reorder, correct grammar, or change wording. Every
//! transformation is either a deletion of a filler or a rewrite of a number or
//! time into digits, and each one is only applied where the alternative reading is
//! ruled out (see the module docs for the specific anchors).

pub mod datetime;
pub mod filler;
pub mod numbers;

pub mod pass;

pub use pass::{polish_block, polish_transcript};
