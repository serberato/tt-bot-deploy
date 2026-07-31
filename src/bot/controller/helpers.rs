//! Playback and lifecycle helper utilities for the controller and runner.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use crate::bot::commands::BotCommand;

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

    /// Current consecutive failure count.
    pub fn consec(&self) -> u32 {
        self.consec
    }

    /// Progressive exponential backoff duration: `500ms * 2^consec` (capped around 8s).
    pub fn backoff_duration(&self) -> std::time::Duration {
        let ms = 500_u64.saturating_mul(1_u64 << self.consec.min(4));
        std::time::Duration::from_millis(ms)
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
