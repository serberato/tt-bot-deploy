//! Core YouTube audio player struct and media player trait implementation.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::bot::commands::BotCommand;
use crate::bot::state::SharedState;
use crate::player::MediaPlayer;
use crate::youtube::metadata::YouTubeMetadata;
use crate::youtube::player::streamer::play_track;

/// Whether a track-end signal is stale compared to the current generation.
pub fn generation_is_stale(signal_gen: u64, current_gen: u64) -> bool {
    signal_gen != current_gen
}

/// Per-track control flags. Recreated on every `load`.
#[derive(Default)]
pub struct TrackControl {
    pub(crate) paused: AtomicBool,
    pub(crate) stopped: AtomicBool,
    pub(crate) position_ms: AtomicU32,
    pub(crate) seek_requested: AtomicBool,
    pub(crate) seek_to_ms: AtomicU32,
}

#[derive(Clone)]
pub struct YouTubePlayer {
    audio_tx: Sender<Vec<i16>>,
    metadata: Arc<YouTubeMetadata>,
    cmd_tx: UnboundedSender<BotCommand>,
    state: SharedState,
    pipeline_pos_ms: Arc<AtomicU32>,
    #[allow(clippy::type_complexity)]
    current: Arc<Mutex<Option<(JoinHandle<()>, Arc<TrackControl>)>>>,
    generation: Arc<AtomicU64>,
}

impl YouTubePlayer {
    pub fn new(
        audio_tx: Sender<Vec<i16>>,
        metadata: Arc<YouTubeMetadata>,
        cmd_tx: UnboundedSender<BotCommand>,
        state: SharedState,
        pipeline_pos_ms: Arc<AtomicU32>,
    ) -> Self {
        Self {
            audio_tx,
            metadata,
            cmd_tx,
            state,
            pipeline_pos_ms,
            current: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    fn spawn_track(&self, video_id: &str) {
        self.abort_current();
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;

        let audio_tx = self.audio_tx.clone();
        let metadata = self.metadata.clone();
        let cmd_tx = self.cmd_tx.clone();
        let state = self.state.clone();
        let pipeline_pos_ms = self.pipeline_pos_ms.clone();
        let video_id = video_id.to_string();
        let ctrl = Arc::new(TrackControl::default());
        let ctrl_for_task = ctrl.clone();

        let handle = tokio::spawn(async move {
            let error = match play_track(video_id.clone(), metadata, audio_tx, ctrl_for_task, state, pipeline_pos_ms).await {
                Ok(()) => None,
                Err(e) => {
                    tracing::error!("YouTube playback failed (video_id={video_id}): {e}");
                    Some(e)
                }
            };
            let _ = cmd_tx.send(BotCommand::TrackEnded { generation, error });
        });

        *self.current.lock() = Some((handle, ctrl));
    }

    pub fn is_stale_generation(&self, signal_gen: u64) -> bool {
        generation_is_stale(signal_gen, self.current_generation())
    }

    fn abort_current(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
        let mut cur = self.current.lock();
        if let Some((handle, ctrl)) = cur.take() {
            ctrl.stopped.store(true, Ordering::Relaxed);
            handle.abort();
        }
    }
}

impl MediaPlayer for YouTubePlayer {
    fn load(&self, video_id: &str) {
        self.spawn_track(video_id);
    }

    fn play(&self) {
        if let Some((_, ctrl)) = self.current.lock().as_ref() {
            ctrl.paused.store(false, Ordering::Relaxed);
        }
    }

    fn pause(&self) {
        if let Some((_, ctrl)) = self.current.lock().as_ref() {
            ctrl.paused.store(true, Ordering::Relaxed);
        }
    }

    fn stop(&self) {
        self.abort_current();
    }

    fn seek(&self, position_ms: u32) {
        if let Some((_, ctrl)) = self.current.lock().as_ref() {
            tracing::debug!("YouTube seek requested to {position_ms}ms");
            ctrl.seek_to_ms.store(position_ms, Ordering::Relaxed);
            ctrl.seek_requested.store(true, Ordering::Relaxed);
        } else {
            tracing::debug!("YouTube seek ignored: no track loaded");
        }
    }

    fn preload(&self, _video_id: &str) {}
}
