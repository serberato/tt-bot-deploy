//! Audio pipeline processing, timing, and TeamTalk injection loop.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use teamtalk::Client;

use crate::audio::pipeline::buffer::{Framer, PrebufferGate};
use crate::audio::pipeline::resampler::{
    BLOCK_DURATION_US, CHANNELS, FRAME_SAMPLES, FRAME_SIZE, SAMPLE_RATE,
};
use crate::audio::volume::VolumeController;
use crate::config::BotConfig;
use crate::tt::audio_inject;

pub fn new_stream_id() -> i32 {
    static NEXT_STREAM_ID: AtomicI32 = AtomicI32::new(1);
    let id = NEXT_STREAM_ID.fetch_add(1, Ordering::Relaxed);
    if id > 0 {
        id
    } else {
        NEXT_STREAM_ID.store(2, Ordering::Relaxed);
        1
    }
}

pub struct AudioPipeline {
    audio_rx: Receiver<Vec<i16>>,
    client: Arc<Client>,
    volume: Arc<AtomicU8>,
    max_volume: u8,
    reset_flag: Arc<AtomicBool>,
    timing_reset_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    stream_flush_flag: Arc<AtomicBool>,
    drained_flag: Arc<AtomicBool>,
    shutdown_flag: Arc<AtomicBool>,
    volume_controller: VolumeController,
    framer: Framer,
    prebuffer: PrebufferGate,
    frame_buf: Vec<i16>,
    stream_id: i32,
    sample_index: u32,
    pos_ms: Arc<AtomicU32>,
    next_block_time: Option<Instant>,
}

impl AudioPipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        audio_rx: Receiver<Vec<i16>>,
        client: Arc<Client>,
        volume: Arc<AtomicU8>,
        reset_flag: Arc<AtomicBool>,
        timing_reset_flag: Arc<AtomicBool>,
        pause_flag: Arc<AtomicBool>,
        stream_flush_flag: Arc<AtomicBool>,
        drained_flag: Arc<AtomicBool>,
        shutdown_flag: Arc<AtomicBool>,
        pos_ms: Arc<AtomicU32>,
        config: &BotConfig,
    ) -> Self {
        let mut volume_controller = VolumeController::new(config.volume_ramp_step);
        volume_controller.set_target(config.volume, config.max_volume);

        Self {
            audio_rx,
            client,
            volume,
            max_volume: config.max_volume,
            reset_flag,
            timing_reset_flag,
            pause_flag,
            stream_flush_flag,
            drained_flag,
            shutdown_flag,
            pos_ms,
            volume_controller,
            framer: Framer::new(FRAME_SIZE * 4),
            prebuffer: PrebufferGate::new(config.jitter_buffer_ms),
            frame_buf: vec![0i16; FRAME_SIZE],
            stream_id: new_stream_id(),
            sample_index: 0,
            next_block_time: None,
        }
    }

    pub fn run(&mut self) {
        tracing::info!("Audio pipeline started");
        loop {
            if self.shutdown_flag.load(Ordering::Relaxed) {
                tracing::info!("Audio pipeline shutting down");
                break;
            }
            self.update_drained_state();
            self.check_resets();
            if self.handle_pause_or_flush() {
                continue;
            }
            if !self.receive_and_frame() {
                break;
            }
            self.inject_ready_frames();
        }
    }

    fn update_drained_state(&self) {
        self.drained_flag.store(
            self.audio_rx.is_empty() && self.framer.len() < FRAME_SIZE,
            Ordering::Relaxed,
        );
    }

    fn check_resets(&mut self) {
        if self.reset_flag.swap(false, Ordering::Relaxed) {
            while self.audio_rx.try_recv().is_ok() {}
            audio_inject::flush_audio(&self.client);
            if !self.client.enable_voice_transmission(false) {
                tracing::debug!("Failed to disable voice transmission on reset");
            }
            self.stream_id = new_stream_id();
            self.framer.clear();
            self.prebuffer.rearm();
            self.next_block_time = None;
            self.sample_index = 0;
            self.pos_ms.store(0, Ordering::Relaxed);
            tracing::info!("Audio pipeline reset for new track (stream_id={})", self.stream_id);
        }
        if self.timing_reset_flag.swap(false, Ordering::Relaxed) {
            self.next_block_time = None;
            tracing::debug!("Audio pipeline timing reset (resume)");
        }
    }

    fn handle_pause_or_flush(&mut self) -> bool {
        if self.pause_flag.load(Ordering::Relaxed) {
            self.next_block_time = None;
            std::thread::sleep(Duration::from_millis(250));
            return true;
        }
        if self.stream_flush_flag.swap(false, Ordering::Relaxed) {
            audio_inject::flush_audio(&self.client);
            self.prebuffer.rearm();
            self.next_block_time = None;
            tracing::info!(
                "Audio stream flushed after channel move (stream_id={} continues, buffer kept)",
                self.stream_id
            );
        }
        false
    }

    fn receive_and_frame(&mut self) -> bool {
        let pcm_res = if self.prebuffer.is_open() && self.framer.len() == 0 {
            self.audio_rx
                .recv()
                .map_err(|_| crossbeam_channel::RecvTimeoutError::Disconnected)
        } else {
            self.audio_rx.recv_timeout(Duration::from_millis(50))
        };

        match pcm_res {
            Ok(pcm_data) => self.framer.push(&pcm_data),
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                if !self.prebuffer.on_idle(self.framer.len()) || self.framer.len() < FRAME_SIZE {
                    return true;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                tracing::info!("Audio pipeline channel closed, exiting");
                return false;
            }
        }

        while let Ok(pcm_data) = self.audio_rx.try_recv() {
            self.framer.push(&pcm_data);
        }
        true
    }

    fn inject_ready_frames(&mut self) {
        if !self.prebuffer.on_data(self.framer.len()) {
            return;
        }
        while self.framer.len() >= FRAME_SIZE {
            if self.reset_flag.load(Ordering::Relaxed)
                || self.pause_flag.load(Ordering::Relaxed)
                || self.stream_flush_flag.load(Ordering::Relaxed)
            {
                break;
            }
            if !self.framer.pop_frame(&mut self.frame_buf) {
                break;
            }
            if self.sample_index == 0 {
                tracing::info!("First audio frame ready, injecting (stream_id={})", self.stream_id);
            }
            let vol = self.volume.load(Ordering::Relaxed);
            self.volume_controller.set_target(vol, self.max_volume);
            self.volume_controller.apply(&mut self.frame_buf);
            self.wait_for_next_block();
            self.inject_frame_with_retry();
            self.sample_index = self.sample_index.wrapping_add(FRAME_SAMPLES as u32);
            self.pos_ms.store(
                (self.sample_index as u64 * 1000 / SAMPLE_RATE as u64) as u32,
                Ordering::Relaxed,
            );
        }
    }

    fn inject_frame_with_retry(&self) {
        const MAX_INJECT_RETRIES: u32 = 20;
        let mut retries = 0u32;
        while !audio_inject::inject_audio_block(
            &self.client,
            &self.frame_buf,
            SAMPLE_RATE,
            CHANNELS,
            self.stream_id,
            self.sample_index,
        ) {
            retries += 1;
            if self.shutdown_flag.load(Ordering::Relaxed) {
                break;
            }
            if retries == 1 {
                tracing::warn!("insert_audio_block failed, retrying...");
            }
            if retries > MAX_INJECT_RETRIES {
                tracing::error!("insert_audio_block failed {MAX_INJECT_RETRIES} times, skipping frame");
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_next_block(&mut self) {
        let now = Instant::now();
        let block_duration = Duration::from_micros(BLOCK_DURATION_US);
        if self.next_block_time.is_none() {
            self.next_block_time = Some(now);
        }
        let next_time = self.next_block_time.unwrap_or(now);
        if next_time > now {
            std::thread::sleep(next_time - now);
        } else if now.duration_since(next_time) > Duration::from_millis(200) {
            tracing::debug!("Audio timing drift, resetting");
            self.next_block_time = Some(now);
        }
        let base_time = self.next_block_time.unwrap_or(now);
        self.next_block_time = Some(base_time + block_duration);
    }
}
