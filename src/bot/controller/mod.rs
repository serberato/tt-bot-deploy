//! Player controller module.
//!
//! Decouples player control (Spotify and YouTube playback engines), audio
//! pipeline control flags, and player state transitions from the command
//! processor and runner lifecycle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use librespot_core::spotify_uri::SpotifyUri;

use crate::bot::state::{PlaybackStatus, SharedState};
use crate::config::ConfigStore;
use crate::player::MediaPlayer as _;
use crate::services::Service;
use crate::spotify::player::SpotifyPlayer;
use crate::youtube::player::YouTubePlayer;

mod helpers;
pub use helpers::*;

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
    pub spotify_brake: Arc<parking_lot::Mutex<StartFailureBrake>>,
    pub youtube_brake: Arc<parking_lot::Mutex<StartFailureBrake>>,
}

impl Controller {
    #[allow(clippy::too_many_arguments)] // Dependency injection constructor
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
        spotify_brake: Arc<parking_lot::Mutex<StartFailureBrake>>,
        youtube_brake: Arc<parking_lot::Mutex<StartFailureBrake>>,
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
            spotify_brake,
            youtube_brake,
        }
    }

    /// Stops playback on both engines, flushes injected audio, disables voice transmission,
    /// resets audio timing, and sets status to `PlaybackStatus::Idle`.
    pub fn stop_playback(&self) {
        self.spotify_brake.lock().on_success();
        self.youtube_brake.lock().on_success();
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
