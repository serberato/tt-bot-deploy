//! YouTube audio player module.

pub mod core;
pub mod decoder;
pub mod interleave;
pub mod streamer;

#[cfg(test)]
mod tests;

pub use core::{generation_is_stale, TrackControl, YouTubePlayer};
