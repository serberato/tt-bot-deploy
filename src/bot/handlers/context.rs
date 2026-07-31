//! Handler context module.
//!
//! Deconstructs the shared command handler state into cohesive domain sub-contexts:
//! - `ClientCtx`: Core bot control, voice announcements, and metadata providers.
//! - `SpotifyCtx`: Spotify session, authentication, and connection recovery.
//! - `ChannelCtx`: Radio streaming parameters, background prefetching, and channel queues.
//! - `LifecycleCtx`: Shared configuration store, volume persistence, and shutdown signaling.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::bot::announcer::Announcer;
use crate::spotify::auth::SpotifyAuth;
use crate::bot::commands::BotCommand;
use crate::bot::controller::Controller;
use crate::bot::runner::BotExit;
use crate::config::ConfigStore;
use crate::error::BotError;
use crate::i18n::Key;
use crate::services::Service;
use crate::spotify::metadata::SpotifyMetadata;
use crate::youtube::metadata::YouTubeMetadata;

/// Core client services and metadata providers.
pub struct ClientCtx {
    pub controller: Controller,
    pub announcer: Announcer,
    pub metadata: SpotifyMetadata,
    pub youtube_metadata: Arc<YouTubeMetadata>,
}

/// Spotify session, authentication, and connection recovery state.
pub struct SpotifyCtx {
    pub session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    pub auth: Arc<SpotifyAuth>,
    pub connected: bool,
    pub recovery_notify: Arc<tokio::sync::Notify>,
    pub recovery_suspended: Arc<AtomicBool>,
}

/// Channel and radio streaming parameters and prefetch handles.
pub struct ChannelCtx {
    pub search_limit: u8,
    pub radio_batch_size: u8,
    pub radio_delay: f32,
    pub radio_cmd_tx: UnboundedSender<BotCommand>,
    pub radio_prefetch_slot: Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub debounce_slot: Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

/// Shared configuration store, volume persistence, and shutdown signaling.
pub struct LifecycleCtx {
    pub config_store: Arc<ConfigStore>,
    pub volume_for_save: Arc<AtomicU8>,
    pub pending_volume_save: Arc<AtomicBool>,
    pub exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>>,
    pub shutdown: Arc<AtomicBool>,
}

/// Root handler context aggregating cohesive sub-contexts.
pub struct HandlerContext {
    pub client: ClientCtx,
    pub spotify: SpotifyCtx,
    pub channel: ChannelCtx,
    pub lifecycle: LifecycleCtx,
}

impl std::ops::Deref for HandlerContext {
    type Target = ClientCtx;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl std::ops::DerefMut for HandlerContext {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

impl HandlerContext {
    /// Connects the Spotify session on first use.
    pub async fn ensure_spotify(&mut self) -> Result<(), BotError> {
        if self.spotify.session.lock().is_invalid() {
            self.spotify.recovery_suspended.store(false, Ordering::Relaxed);
            self.spotify.recovery_notify.notify_one();
            Err(BotError::Playback(
                "Spotify is reconnecting; try again in a moment".into(),
            ))
        } else if self.spotify.connected {
            Ok(())
        } else {
            self.client.announcer.send_event(crate::bot::runner::RunnerEvent::Authenticating);
            let s = self.spotify.session.lock().clone();
            let r = self.spotify.auth.connect_existing(&s).await;
            if r.is_ok() {
                self.spotify.connected = true;
            }
            r
        }
    }

    /// Checks if session has died after a Spotify metadata failure and wakes the recovery supervisor.
    pub fn notify_recovery_if_invalid(&self) {
        if self.spotify.session.lock().is_invalid() {
            self.spotify.recovery_suspended.store(false, Ordering::Relaxed);
            self.spotify.recovery_notify.notify_one();
        }
    }

    /// Debounces stream initialization by 350ms to coalesce rapid commands (`n` / `b`).
    pub fn schedule_start_track(
        &self,
        service: Service,
        uri_str: String,
        user_id: i32,
        name: String,
    ) {
        let mut guard = self.channel.debounce_slot.lock();
        if let Some(handle) = guard.take() {
            handle.abort();
        }
        let tx = self.channel.radio_cmd_tx.clone();
        *guard = Some(tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            let _ = tx.send(BotCommand::StartSettledTrack {
                service,
                uri: uri_str,
                user_id,
                name,
            });
        }));
    }

    /// Attempts to start a track; on failure, notifies user and auto-skips or trips brake.
    pub fn start_or_skip(
        &mut self,
        service: Service,
        uri_str: &str,
        user_id: i32,
        track_name: &str,
    ) -> bool {
        if self.client.controller.start_track(service, uri_str) {
            true
        } else {
            self.reply_t(user_id, Key::FailedToStart, &[("track", track_name.to_string())]);
            let mut guard = match service {
                Service::Spotify => self.client.controller.spotify_brake.lock(),
                Service::YouTube => self.client.controller.youtube_brake.lock(),
            };
            if guard.on_failure() {
                drop(guard);
                self.reply_t(user_id, Key::RateLimitCooldown, &[("service", format!("{service:?}"))]);
                crate::bot::handlers::playback::handle_circuit_breaker_trip(self, service);
            } else {
                let delay = guard.backoff_duration();
                drop(guard);
                let tx = self.channel.radio_cmd_tx.clone();
                let after_uri = Some(uri_str.to_string());
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let _ = tx.send(BotCommand::Next {
                        user_id: 0,
                        after_track: after_uri,
                    });
                });
            }
            false
        }
    }

    /// Stops playback, clears the queue, and resets bot to an idle status.
    pub fn brake_stop(&mut self) {
        self.client.controller.stop_playback();
        {
            let mut s = self.client.controller.state.lock();
            s.clear();
            s.position_ms = 0;
        }
        self.announce_idle();
    }

    /// Performs a clean exit or restart of the bot.
    pub fn do_exit(&mut self, reason: BotExit) {
        self.client.controller.stop_playback();
        self.announce_idle();
        {
            let s = self.client.controller.state.lock();
            let vol = self.lifecycle.volume_for_save.load(Ordering::Relaxed);
            let radio = s.radio_enabled;
            let repeat_track = s.repeat_track;
            let repeat_queue = s.repeat_queue;
            let shuffle = s.shuffle;
            drop(s);
            self.lifecycle.config_store.update(|cfg| {
                cfg.radio_enabled = radio;
                cfg.volume = vol;
                cfg.repeat_track = repeat_track;
                cfg.repeat_queue = repeat_queue;
                cfg.shuffle = shuffle;
            });
        }
        let _ = self.client.controller.client.disconnect();
        *self.lifecycle.exit_reason.lock() = Some(reason);
        self.lifecycle.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn reply_t(&self, user_id: i32, key: Key, args: &[(&str, String)]) {
        self.client.announcer.reply_t(user_id, key, args);
    }

    pub fn announce_playing(&self, track_name: &str) {
        self.client.announcer.announce_playing(track_name, &self.client.controller.state);
    }

    pub fn announce_idle(&self) {
        self.client.announcer.announce_idle(&self.lifecycle.config_store.get_idle_status());
    }
}
