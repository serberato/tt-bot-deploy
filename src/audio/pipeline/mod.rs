//! Audio pipeline processing module.

pub mod buffer;
pub mod encoder;
pub mod resampler;

#[cfg(test)]
mod tests;

pub use encoder::AudioPipeline;
