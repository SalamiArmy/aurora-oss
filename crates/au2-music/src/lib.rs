//! Aurora song format (`aurora-song/1`) and its deterministic renderer.
//!
//! A song is JSON: instruments plus nine or more stems (intro, beat, beathigh,
//! three pads, 1-4 fills), each a set of tracks of 16th-note events. Rendering is integer-timed
//! synthesis, so a stem is always exactly `bars * 128` game frames long and loops sample-exactly.
pub mod format;
pub mod render;
pub mod wav;

pub use format::{Song, StemKind, ValidationError};
pub use render::{render_song, render_stem, RenderedStem};

/// Game frames per second (the sim's fixed step).
pub const FPS: u32 = 60;
/// 16th-note steps per bar.
pub const STEPS_PER_BAR: u32 = 16;
/// Game frames per 16th-note step (`(frame & 0x7f) >> 3` is the step index).
pub const FRAMES_PER_STEP: u32 = 8;
/// Game frames per bar: 128 = 2.1333 s = 112.5 BPM.
pub const FRAMES_PER_BAR: u32 = STEPS_PER_BAR * FRAMES_PER_STEP;
