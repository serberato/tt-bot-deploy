//! Audio pipeline sample rate, framing, and timing constants.

/// Use librespot's native sample rate - no resampling needed
pub const SAMPLE_RATE: i32 = 44100;
pub const CHANNELS: i32 = 2;
/// 20ms frames at 44100Hz stereo = 882 samples/channel × 2 channels = 1764 i16 values
pub const FRAME_SAMPLES: usize = 882;
pub const FRAME_SIZE: usize = FRAME_SAMPLES * CHANNELS as usize; // 1764

/// Block duration in microseconds (~20ms)
pub const BLOCK_DURATION_US: u64 = (FRAME_SAMPLES as u64 * 1_000_000) / SAMPLE_RATE as u64;

/// Number of consecutive empty 50ms channel polls after which a closed gate
/// opens anyway: the producer has gone quiet mid-fill (end of stream or a hard
/// stall), so play out whatever is buffered instead of holding it hostage.
pub const IDLE_POLLS_BEFORE_FLUSH: u32 = 6;
