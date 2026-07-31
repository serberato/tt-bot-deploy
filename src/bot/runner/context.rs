use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8};
use std::sync::Arc;

use crate::bot::runner::{BotExit, RunnerEvent};
use crate::bot::state::SharedState;
use crate::config::BotConfig;
use crate::spotify::metadata::SpotifyMetadata;
use crate::spotify::player::SpotifyPlayer;

/// All shared context needed by the command processor, bundled to avoid parameter explosion.
pub(crate) struct CmdContext {
    pub player: SpotifyPlayer,
    pub metadata: SpotifyMetadata,
    pub youtube_metadata: Arc<crate::youtube::metadata::YouTubeMetadata>,
    pub youtube_player: crate::youtube::player::YouTubePlayer,
    pub session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    pub auth: Arc<crate::spotify::auth::SpotifyAuth>,
    pub spotify_connected: bool,
    /// Wakes the recovery supervisor when a command detects a dead session.
    pub recovery_notify: Arc<tokio::sync::Notify>,
    /// Cleared by a Spotify command to un-latch auto-recovery after a give-up.
    pub recovery_suspended: Arc<AtomicBool>,
    pub state: SharedState,
    pub client: Arc<::teamtalk::Client>,
    pub search_limit: u8,
    pub radio_batch_size: u8,
    pub radio_delay: f32,
    pub radio_cmd_tx: tokio::sync::mpsc::UnboundedSender<crate::bot::commands::BotCommand>,
    pub bot_gender: ::teamtalk::types::UserGender,
    pub config_store: Arc<crate::config::ConfigStore>,
    pub audio_reset: Arc<AtomicBool>,
    pub timing_reset: Arc<AtomicBool>,
    pub pause_flag: Arc<AtomicBool>,
    /// True while the audio pipeline has nothing buffered; natural track ends
    /// wait on this before advancing so the song's tail plays out.
    pub pipeline_drained: Arc<AtomicBool>,
    pub volume_for_save: Arc<AtomicU8>,
    pub exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>>,
    pub shutdown: Arc<AtomicBool>,
    pub event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    pub i18n: Arc<crate::i18n::I18n>,
    pub spotify_brake: Arc<parking_lot::Mutex<crate::bot::controller::StartFailureBrake>>,
    pub youtube_brake: Arc<parking_lot::Mutex<crate::bot::controller::StartFailureBrake>>,
}

/// Shared atomic flags for audio pipeline and synchronization.
#[derive(Clone)]
pub(crate) struct SharedFlags {
    pub audio_reset: Arc<AtomicBool>,
    pub timing_reset: Arc<AtomicBool>,
    pub pause_flag: Arc<AtomicBool>,
    pub stream_flush: Arc<AtomicBool>,
    pub pipeline_drained: Arc<AtomicBool>,
    pub pipeline_pos_ms: Arc<AtomicU32>,
    pub local_shutdown: Arc<AtomicBool>,
}

impl SharedFlags {
    pub fn new() -> Self {
        Self {
            audio_reset: Arc::new(AtomicBool::new(false)),
            timing_reset: Arc::new(AtomicBool::new(false)),
            pause_flag: Arc::new(AtomicBool::new(false)),
            stream_flush: Arc::new(AtomicBool::new(false)),
            pipeline_drained: Arc::new(AtomicBool::new(true)),
            pipeline_pos_ms: Arc::new(AtomicU32::new(0)),
            local_shutdown: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Tracking for channel joins across reconnects.
#[derive(Clone)]
pub(crate) struct ChannelTracker {
    pub last_channel_id: Arc<parking_lot::Mutex<::teamtalk::types::ChannelId>>,
    pub last_channel_pw: Arc<parking_lot::Mutex<String>>,
}

impl ChannelTracker {
    pub fn from_client_and_config(client: &::teamtalk::Client, config: &BotConfig) -> Self {
        Self {
            last_channel_id: Arc::new(parking_lot::Mutex::new(client.my_channel_id())),
            last_channel_pw: Arc::new(parking_lot::Mutex::new(config.channel_password.clone())),
        }
    }
}
