// speaker_diarization_engine/mod.rs
//
// Module entry for speaker diarization engine (post-processing speaker labeling).
// Ported from mimi202605/meeting-minutes (commits 3ef898b..934305a), adapted to
// this tree's runtime model-download flow and "You"/"Others" speaker fields.

pub mod commands;
pub mod engine;
