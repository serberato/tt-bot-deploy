//! Player controller module.
//!
//! Decouples player control (Spotify and YouTube playback engines), audio
//! pipeline control flags, and player state transitions from the command
//! processor and runner lifecycle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use librespot_core::spotify_uri::SpotifyUri;
use tokio::sync::mpsc::UnboundedSender;

use crate::bot::commands::BotCommand;
use crate::bot::state::{PlaybackStatus, SharedState};
use crate::config::ConfigStore;
use crate::player::MediaPlayer as _;
use crate::services::Service;
use crate::spotify::player::SpotifyPlayer;
use crate::youtube::player::YouTubePlayer;

/// Decoupled controller for Spotify and YouTube playback engines and shared state.
#[derive(Clone)]
pub struct Controller {
    pub player: SpotifyPlayer,
    pub youtube_player: YouTubePlayer,
    pub client: Arc<::teamtalk::Client>,
    pub state: SharedState,
    pub audio_reset: Arc<AtomicBool>,
    pub pause_flag: Arc<AtomicBool>,
    pub timing_reset: Arc<AtomicBool>,
    pub pipeline_drained: Arc<AtomicBool>,
    pub config_store: Arc<ConfigStore>,
}

impl Controller {
    pub fn new(
        player: SpotifyPlayer,
        youtube_player: YouTubePlayer,
        client: Arc<::teamtalk::Client>,
        state: SharedState,
        audio_reset: Arc<AtomicBool>,
        pause_flag: Arc<AtomicBool>,
        timing_reset: Arc<AtomicBool>,
        pipeline_drained: Arc<AtomicBool>,
        config_store: Arc<ConfigStore>,
    ) -> Self {
        Self {
            player,
            youtube_player,
            client,
            state,
            audio_reset,
            pause_flag,
            timing_reset,
            pipeline_drained,
            config_store,
        }
    }

    /// Stops playback on both engines, flushes injected audio, disables voice transmission,
    /// resets audio timing, and sets status to `PlaybackStatus::Idle`.
    pub fn stop_playback(&self) {
        self.pause_flag.store(false, Ordering::Relaxed);
        self.player.stop();
        self.youtube_player.stop();
        crate::tt::audio_inject::flush_audio(&self.client);
        let _ = self.client.enable_voice_transmission(false);
        self.audio_reset.store(true, Ordering::Relaxed);
        let mut s = self.state.lock();
        s.status = PlaybackStatus::Idle;
    }

    /// Starts a track on the specified service.
    ///
    /// Stops any existing playback, flushes audio buffers, and sets status to `PlaybackStatus::Loading`.
    /// Returns `true` if the load was successfully initiated, `false` otherwise (e.g. invalid URI).
    pub fn start_track(&self, service: Service, uri_str: &str) -> bool {
        match service {
            Service::Spotify => {
                if let Ok(uri) = SpotifyUri::from_uri(uri_str) {
                    self.pause_flag.store(false, Ordering::Relaxed);
                    self.player.stop();
                    self.youtube_player.stop();
                    crate::tt::audio_inject::flush_audio(&self.client);
                    let _ = self.client.enable_voice_transmission(false);
                    self.audio_reset.store(true, Ordering::Relaxed);
                    self.player.load_track(&uri);
                    self.player.play();
                    {
                        let mut s = self.state.lock();
                        s.status = PlaybackStatus::Loading;
                        s.tracks_played += 1;
                    }
                    true
                } else {
                    false
                }
            }
            Service::YouTube => {
                self.pause_flag.store(false, Ordering::Relaxed);
                self.player.stop();
                self.youtube_player.stop();
                crate::tt::audio_inject::flush_audio(&self.client);
                let _ = self.client.enable_voice_transmission(false);
                self.audio_reset.store(true, Ordering::Relaxed);
                self.youtube_player.load(uri_str);
                self.youtube_player.play();
                {
                    let mut s = self.state.lock();
                    s.status = PlaybackStatus::Loading;
                    s.tracks_played += 1;
                }
                true
            }
        }
    }

    /// Resumes or starts playback on both players, setting state status to `PlaybackStatus::Playing`.
    /// Returns the display name of the current track, if any.
    pub fn play(&self) -> Option<String> {
        self.pause_flag.store(false, Ordering::Relaxed);
        self.timing_reset.store(true, Ordering::Relaxed);
        self.player.play();
        self.youtube_player.play();
        let mut s = self.state.lock();
        s.status = PlaybackStatus::Playing;
        s.current().map(|entry| entry.track.display_name())
    }

    /// Pauses playback on both players, setting state status to `PlaybackStatus::Paused`.
    /// Returns the display name of the current track, if any.
    pub fn pause(&self) -> Option<String> {
        self.pause_flag.store(true, Ordering::Relaxed);
        self.player.pause();
        self.youtube_player.pause();
        crate::tt::audio_inject::flush_audio(&self.client);
        let mut s = self.state.lock();
        s.status = PlaybackStatus::Paused;
        s.current().map(|entry| entry.track.display_name())
    }

    /// Replays the current track from position 0.
    /// Returns the display name of the current track, if any.
    pub fn replay(&self) -> Option<String> {
        let service = {
            let mut s = self.state.lock();
            s.position_ms = 0;
            s.current().map(|e| e.track.service()).unwrap_or(s.active_service)
        };
        self.audio_reset.store(true, Ordering::Relaxed);
        self.pause_flag.store(false, Ordering::Relaxed);
        self.timing_reset.store(true, Ordering::Relaxed);
        match service {
            Service::Spotify => {
                self.player.seek(0);
                self.player.play();
            }
            Service::YouTube => {
                self.youtube_player.seek(0);
                self.youtube_player.play();
            }
        }
        let mut s = self.state.lock();
        s.status = PlaybackStatus::Playing;
        s.current().map(|entry| entry.track.display_name())
    }

    /// Seeks by the given offset in milliseconds relative to the current position.
    pub fn seek(&self, offset_ms: i32) {
        let (new_pos, service) = {
            let mut s = self.state.lock();
            let current = s.position_ms as i32;
            let pos = (current + offset_ms).max(0) as u32;
            let svc = s.current().map(|e| e.track.service()).unwrap_or(s.active_service);
            s.position_ms = pos;
            (pos, svc)
        };
        self.audio_reset.store(true, Ordering::Relaxed);
        match service {
            Service::Spotify => self.player.seek(new_pos),
            Service::YouTube => self.youtube_player.seek(new_pos),
        }
    }

    /// Preloads the next track on Spotify if `uri_str` is a valid Spotify URI.
    pub fn preload(&self, uri_str: &str) {
        if let Ok(uri) = SpotifyUri::from_uri(uri_str) {
            self.player.preload(&uri);
            tracing::debug!("Preloading next track: {uri_str}");
        }
    }

    /// Returns whether a YouTube end-of-track generation is stale compared to current active generation.
    pub fn is_stale_generation(&self, generation: u64) -> bool {
        self.youtube_player.is_stale_generation(generation)
    }
}

/// How the runner should handle Spotify auth at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAuthPlan {
    /// Connect eagerly; a failure aborts startup (interactive contexts, where
    /// the user is present to complete or fix the OAuth flow).
    ConnectFatal,
    /// Connect eagerly with cached credentials; on failure log, disable
    /// Spotify, and keep running. Used when OAuth is infeasible (systemd):
    /// dying here would loop TeamTalk login/logout via Restart=on-failure.
    ConnectBestEffort,
    /// Don't touch Spotify at startup (YouTube-only user, no cached creds);
    /// the connection happens lazily on the first Spotify command.
    Skip,
}

/// Decide the startup auth plan from what's cached, what the default service
/// is, and whether an interactive OAuth flow could succeed in this process.
pub fn startup_auth_plan(
    has_cached_credentials: bool,
    spotify_is_default: bool,
    oauth_feasible: bool,
) -> StartupAuthPlan {
    if !has_cached_credentials && !spotify_is_default {
        StartupAuthPlan::Skip
    } else if oauth_feasible {
        StartupAuthPlan::ConnectFatal
    } else {
        StartupAuthPlan::ConnectBestEffort
    }
}

/// Counts consecutive track-start failures so a queue of broken tracks (or a
/// broken repeat-mode track) stops instead of auto-skipping forever.
pub struct StartFailureBrake {
    consec: u32,
    cap: u32,
}

impl StartFailureBrake {
    pub fn new(cap: u32) -> Self {
        Self { consec: 0, cap }
    }

    /// A track started (Spotify) or finished (any service) cleanly.
    pub fn on_success(&mut self) {
        self.consec = 0;
    }

    /// A track failed to start or errored out. Returns true when the streak
    /// hit the cap: caller must stop playback and go idle (streak resets).
    pub fn on_failure(&mut self) -> bool {
        self.consec += 1;
        if self.consec >= self.cap {
            self.consec = 0;
            true
        } else {
            false
        }
    }
}

/// Settles when the audio pipeline has reported "nothing left to play" twice
/// in a row.
pub struct DrainWait {
    consecutive: u32,
}

impl DrainWait {
    pub fn new() -> Self {
        Self { consecutive: 0 }
    }

    /// Feed one drained-or-not observation; returns true once settled.
    pub fn observe(&mut self, drained: bool) -> bool {
        if drained {
            self.consecutive += 1;
        } else {
            self.consecutive = 0;
        }
        self.consecutive >= 2
    }
}

impl Default for DrainWait {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawns a background task that waits for the audio pipeline to run dry, then
/// sends an auto-advance `Next` command.
pub fn spawn_drained_advance(
    cmd_tx: UnboundedSender<BotCommand>,
    pipeline_drained: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    after_track: Option<String>,
) {
    tokio::spawn(async move {
        const MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
        let mut started = std::time::Instant::now();
        let mut wait = DrainWait::new();
        loop {
            if pause_flag.load(Ordering::Relaxed) {
                started = std::time::Instant::now();
            } else {
                if wait.observe(pipeline_drained.load(Ordering::Relaxed)) {
                    break;
                }
                if started.elapsed() > MAX_WAIT {
                    tracing::warn!("Track-end drain wait timed out; advancing anyway");
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let _ = cmd_tx.send(BotCommand::Next { user_id: 0, after_track });
    });
}

/// Whether an auto-advance (sent when a track ended or failed) is stale: the
/// queue has already moved past the track it was advancing from.
pub fn auto_advance_is_stale(after_track: Option<&str>, current: Option<&str>) -> bool {
    match after_track {
        None => false,
        Some(expected) => current != Some(expected),
    }
}

/// Whether a self channel-change requires flushing the injected audio stream.
pub fn channel_move_needs_flush(
    prev: ::teamtalk::types::ChannelId,
    new: ::teamtalk::types::ChannelId,
) -> bool {
    prev != ::teamtalk::types::ChannelId(0) && prev != new
}
