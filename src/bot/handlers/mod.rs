//! Bot command handlers module.
//!
//! Breaks down command processing by domain: playback, queue, radio, and settings.
//! Uses `HandlerContext` to encapsulate shared dependencies across handlers.

pub mod playback;
pub mod queue;
pub mod radio;
pub mod settings;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::bot::announcer::Announcer;
use crate::bot::commands::BotCommand;
use crate::bot::controller::{Controller, StartFailureBrake};
use crate::bot::runner::BotExit;
use crate::config::ConfigStore;
use crate::error::BotError;
use crate::i18n::Key;
use crate::services::Service;
use crate::spotify::auth::SpotifyAuth;
use crate::spotify::metadata::SpotifyMetadata;
use crate::youtube::metadata::YouTubeMetadata;

/// Shared context passed to domain handlers during command processing.
pub struct HandlerContext {
    pub controller: Controller,
    pub announcer: Announcer,
    pub metadata: SpotifyMetadata,
    pub youtube_metadata: Arc<YouTubeMetadata>,
    pub session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    pub auth: Arc<SpotifyAuth>,
    pub spotify_connected: bool,
    pub recovery_notify: Arc<tokio::sync::Notify>,
    pub recovery_suspended: Arc<AtomicBool>,
    pub search_limit: u8,
    pub radio_batch_size: u8,
    pub radio_delay: f32,
    pub radio_cmd_tx: UnboundedSender<BotCommand>,
    pub config_store: Arc<ConfigStore>,
    pub volume_for_save: Arc<AtomicU8>,
    pub exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>>,
    pub shutdown: Arc<AtomicBool>,
    pub start_brake: StartFailureBrake,
    pub radio_prefetch_slot: Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub pending_volume_save: Arc<AtomicBool>,
}

impl HandlerContext {
    /// Connects the Spotify session on first use.
    /// No-op once connected. For a YouTube-only user this is where OAuth flow triggers lazily.
    pub async fn ensure_spotify(&mut self) -> Result<(), BotError> {
        if self.session.lock().is_invalid() {
            self.recovery_suspended.store(false, Ordering::Relaxed);
            self.recovery_notify.notify_one();
            Err(BotError::Playback(
                "Spotify is reconnecting; try again in a moment".into(),
            ))
        } else if self.spotify_connected {
            Ok(())
        } else {
            self.announcer.send_event(crate::bot::runner::RunnerEvent::Authenticating);
            let s = self.session.lock().clone();
            let r = self.auth.connect_existing(&s).await;
            if r.is_ok() {
                self.spotify_connected = true;
            }
            r
        }
    }

    /// Checks if session has died after a Spotify metadata failure and wakes the recovery supervisor.
    pub fn notify_recovery_if_invalid(&self) {
        if self.session.lock().is_invalid() {
            self.recovery_suspended.store(false, Ordering::Relaxed);
            self.recovery_notify.notify_one();
        }
    }

    /// Attempts to start a track; on failure, notifies user and auto-skips or trips brake.
    pub fn start_or_skip(
        &mut self,
        service: Service,
        uri_str: &str,
        user_id: i32,
        track_name: &str,
    ) -> bool {
        if self.controller.start_track(service, uri_str) {
            if service == Service::Spotify {
                self.start_brake.on_success();
            }
            true
        } else {
            self.reply_t(user_id, Key::FailedToStart, &[("track", track_name.to_string())]);
            if self.start_brake.on_failure() {
                self.brake_stop();
            } else {
                let _ = self.radio_cmd_tx.send(BotCommand::Next {
                    user_id: 0,
                    after_track: Some(uri_str.to_string()),
                });
            }
            false
        }
    }

    /// Stops playback, clears the queue, and resets bot to an idle status.
    pub fn brake_stop(&mut self) {
        self.controller.stop_playback();
        {
            let mut s = self.controller.state.lock();
            s.clear();
            s.position_ms = 0;
        }
        self.announce_idle();
    }

    /// Performs a clean exit or restart of the bot.
    pub fn do_exit(&mut self, reason: BotExit) {
        self.controller.stop_playback();
        self.announce_idle();
        {
            let s = self.controller.state.lock();
            let vol = self.volume_for_save.load(Ordering::Relaxed);
            let radio = s.radio_enabled;
            let repeat_track = s.repeat_track;
            let repeat_queue = s.repeat_queue;
            let shuffle = s.shuffle;
            drop(s);
            self.config_store.update(|cfg| {
                cfg.radio_enabled = radio;
                cfg.volume = vol;
                cfg.repeat_track = repeat_track;
                cfg.repeat_queue = repeat_queue;
                cfg.shuffle = shuffle;
            });
        }
        let _ = self.controller.client.disconnect();
        *self.exit_reason.lock() = Some(reason);
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Shorthand to send a translated reply to a user.
    pub fn reply_t(&self, user_id: i32, key: Key, args: &[(&str, String)]) {
        self.announcer.reply_t(user_id, key, args);
    }

    /// Shorthand to announce that a track is playing.
    pub fn announce_playing(&self, track_name: &str) {
        self.announcer.announce_playing(track_name, &self.controller.state);
    }

    /// Shorthand to announce that the bot is idle.
    pub fn announce_idle(&self) {
        self.announcer.announce_idle(&self.config_store.get_idle_status());
    }
}

/// Dispatch a `BotCommand` to the appropriate domain handler.
pub async fn handle_command(cmd: BotCommand, ctx: &mut HandlerContext) {
    match cmd {
        // Playback domain
        BotCommand::Play { user_id } => playback::handle_play(ctx, user_id),
        BotCommand::Pause { user_id } => playback::handle_pause(ctx, user_id),
        BotCommand::Stop { user_id } => playback::handle_stop(ctx, user_id),
        BotCommand::Next { user_id, after_track } => playback::handle_next(ctx, user_id, after_track).await,
        BotCommand::Prev { user_id } => playback::handle_prev(ctx, user_id),
        BotCommand::Seek { offset_ms, user_id } => playback::handle_seek(ctx, offset_ms, user_id),
        BotCommand::SetVolume { percent, user_id } => playback::handle_set_volume(ctx, percent, user_id),
        BotCommand::SetMode { mode, user_id } => playback::handle_set_mode(ctx, mode, user_id),
        BotCommand::Replay { user_id } => playback::handle_replay(ctx, user_id),
        BotCommand::PreloadNext => playback::handle_preload_next(ctx),
        BotCommand::TrackEnded { generation, error } => playback::handle_track_ended(ctx, generation, error),

        // Queue domain
        BotCommand::SearchAndPlay { query, user_id, user_name } => {
            queue::handle_search_and_play(ctx, query, user_id, user_name).await;
        }
        BotCommand::SearchOnly { query, user_id } => {
            queue::handle_search_only(ctx, query, user_id).await;
        }
        BotCommand::SearchPick { user_id, pick, user_name } => {
            queue::handle_search_pick(ctx, user_id, pick, user_name);
        }
        BotCommand::QueueClear { user_id } => queue::handle_queue_clear(ctx, user_id),
        BotCommand::QueueRemove { index, user_id } => queue::handle_queue_remove(ctx, index, user_id),

        // Radio domain
        BotCommand::RadioToggle { enable, user_id } => radio::handle_radio_toggle(ctx, enable, user_id),
        BotCommand::RadioPreFetch { seed_uri } => {
            radio::handle_radio_prefetch(ctx, seed_uri).await;
        }

        // Settings / lifecycle domain
        BotCommand::JoinChannel { path, user_id } => settings::handle_join_channel(ctx, path, user_id),
        BotCommand::ChangeNick { name, user_id } => settings::handle_change_nick(ctx, name, user_id),
        BotCommand::SetGender { gender, user_id } => settings::handle_set_gender(ctx, gender, user_id),
        BotCommand::SetStatus { status_text, user_id } => settings::handle_set_status(ctx, status_text, user_id),
        BotCommand::SetPlayMode { mode, user_id } => settings::handle_set_play_mode(ctx, mode, user_id),
        BotCommand::SetService { service, user_id } => settings::handle_set_service(ctx, service, user_id),
        BotCommand::SetDefaultLanguage { code, user_id } => settings::handle_set_default_language(ctx, code, user_id),
        BotCommand::Quit { user_id } => settings::handle_quit(ctx, user_id),
        BotCommand::Restart { user_id } => settings::handle_restart(ctx, user_id),
    }
}
