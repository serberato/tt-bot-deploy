//! Channel interleaving and final buffer flushing utilities for audio decoding.

use crossbeam_channel::Sender;
use rubato::{Resampler, SincFixedIn};

/// Interleave planar f32 left and right channels into signed 16-bit PCM samples.
pub fn interleave_to_i16(l: &[f32], r: &[f32]) -> Vec<i16> {
    let n = l.len().min(r.len());
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        out.push((l[i].clamp(-1.0, 1.0) * 32767.0) as i16);
        out.push((r[i].clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    out
}

/// Flush any remaining samples in the left/right planar buffers through the resampler
/// or directly to the audio output channel.
pub fn flush_remaining(
    resampler: Option<&mut SincFixedIn<f32>>,
    buf_l: &mut Vec<f32>,
    buf_r: &mut Vec<f32>,
    audio_tx: &Sender<Vec<i16>>,
    chunk_in: usize,
) {
    if buf_l.is_empty() {
        return;
    }
    if let Some(rs) = resampler {
        if buf_l.len() < chunk_in {
            buf_l.resize(chunk_in, 0.0);
            buf_r.resize(chunk_in, 0.0);
        }
        let in_l: Vec<f32> = buf_l.drain(..chunk_in).collect();
        let in_r: Vec<f32> = buf_r.drain(..chunk_in).collect();
        if let Ok(out) = rs.process(&[in_l, in_r], None) {
            let _ = audio_tx.send(interleave_to_i16(&out[0], &out[1]));
        }
    } else {
        let _ = audio_tx.send(interleave_to_i16(buf_l, buf_r));
        buf_l.clear();
        buf_r.clear();
    }
}
