//! Audio framing and jitter buffer gating structures.

use std::collections::VecDeque;

use crate::audio::pipeline::resampler::{CHANNELS, IDLE_POLLS_BEFORE_FLUSH, SAMPLE_RATE};

/// Accumulates incoming PCM and hands out fixed-size frames. Backed by a `VecDeque`.
pub(crate) struct Framer {
    buf: VecDeque<i16>,
}

impl Framer {
    pub(crate) fn new(capacity: usize) -> Self {
        Self { buf: VecDeque::with_capacity(capacity) }
    }

    pub(crate) fn push(&mut self, samples: &[i16]) {
        self.buf.extend(samples.iter().copied());
    }

    pub(crate) fn len(&self) -> usize {
        self.buf.len()
    }

    pub(crate) fn clear(&mut self) {
        self.buf.clear();
    }

    /// Pop exactly `out.len()` samples into `out`. Returns false if fewer are buffered.
    pub(crate) fn pop_frame(&mut self, out: &mut [i16]) -> bool {
        if self.buf.len() < out.len() {
            return false;
        }
        for slot in out.iter_mut() {
            if let Some(sample) = self.buf.pop_front() {
                *slot = sample;
            } else {
                break;
            }
        }
        true
    }
}

/// Holds back injection after a (re)start until `jitter_buffer_ms` worth of
/// audio is buffered, absorbing bursty producer starts.
pub(crate) struct PrebufferGate {
    target_samples: usize,
    open: bool,
    idle_polls: u32,
}

impl PrebufferGate {
    pub(crate) fn new(jitter_buffer_ms: u32) -> Self {
        let target_samples =
            (SAMPLE_RATE as u64 * CHANNELS as u64 * jitter_buffer_ms as u64 / 1000) as usize;
        Self { target_samples, open: target_samples == 0, idle_polls: 0 }
    }

    pub(crate) fn rearm(&mut self) {
        self.open = self.target_samples == 0;
        self.idle_polls = 0;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn on_data(&mut self, buffered: usize) -> bool {
        self.idle_polls = 0;
        if buffered >= self.target_samples {
            self.open = true;
        }
        self.open
    }

    pub(crate) fn on_idle(&mut self, buffered: usize) -> bool {
        if !self.open && buffered > 0 {
            self.idle_polls += 1;
            if self.idle_polls >= IDLE_POLLS_BEFORE_FLUSH {
                self.open = true;
            }
        }
        self.open
    }
}
