//! Unified Liveness & Self-Healing Watchdog for Spotify and YouTube streaming engines.
//!
//! Monitors `SharedState.status`, track timestamps, and `pipeline_drained`:
//! - **Loading Stall Rule:** If `status == PlaybackStatus::Loading` continuously for **> 15 seconds**, declares a stream initialization freeze.
//! - **Playback Dry-Stall Rule:** If `status == PlaybackStatus::Playing` AND `pipeline_drained == true` continuously for **> 10 seconds** without a position advance, declares an audio decoder/socket stall.
//! - **Automatic Recovery Without Restarting Bot:**
//!   - For YouTube: stops the player (RAII process kill) and triggers auto-recovery.
//!   - For Spotify: triggers `recover_spotify` to rebuild session/player.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bot::commands::BotCommand;
use crate::bot::runner::spotify_recovery::{recover_spotify, SpotifyRecovery};
use crate::bot::state::{PlaybackStatus, SharedState};
use crate::player::MediaPlayer;
use crate::services::Service;
use crate::youtube::player::YouTubePlayer;

const LOADING_STALL_LIMIT: Duration = Duration::from_secs(15);
const PLAYING_STALL_LIMIT: Duration = Duration::from_secs(10);
const WATCHDOG_INTERVAL: Duration = Duration::from_millis(500);

/// Supervisor loop that continuously monitors stream liveness and triggers self-healing on stall.
pub(crate) async fn watchdog_loop(
    state: SharedState,
    pipeline_drained: Arc<AtomicBool>,
    youtube_player: YouTubePlayer,
    spotify_recovery: Arc<SpotifyRecovery>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    shutdown: Arc<AtomicBool>,
) {
    let mut loading_since: Option<Instant> = None;
    let mut playing_stalled_since: Option<Instant> = None;
    let mut last_position_ms: u32 = 0;

    while !shutdown.load(Ordering::Relaxed) {
        tokio::time::sleep(WATCHDOG_INTERVAL).await;
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let (status, position_ms, service, uri) = {
            let s = state.lock();
            let svc = s.current().map(|entry| entry.track.service());
            let u = s.current().map(|entry| entry.track.uri().to_string());
            (s.status, s.position_ms, svc, u)
        };

        match status {
            PlaybackStatus::Loading => {
                playing_stalled_since = None;
                match loading_since {
                    Some(start) if start.elapsed() > LOADING_STALL_LIMIT => {
                        tracing::warn!(
                            "Watchdog: Stream loading stall detected (> {:?}) for {:?} ({:?}); auto-recovering",
                            LOADING_STALL_LIMIT,
                            service,
                            uri
                        );
                        loading_since = None;
                        handle_recovery(&youtube_player, &spotify_recovery, service, &uri, &cmd_tx).await;
                    }
                    Some(_) => {}
                    None => loading_since = Some(Instant::now()),
                }
            }
            PlaybackStatus::Playing => {
                loading_since = None;
                let drained = pipeline_drained.load(Ordering::Relaxed);
                if drained && position_ms == last_position_ms {
                    match playing_stalled_since {
                        Some(start) if start.elapsed() > PLAYING_STALL_LIMIT => {
                            tracing::warn!(
                                "Watchdog: Playback dry-stall detected (> {:?}) for {:?} ({:?}); auto-recovering",
                                PLAYING_STALL_LIMIT,
                                service,
                                uri
                            );
                            playing_stalled_since = None;
                            handle_recovery(&youtube_player, &spotify_recovery, service, &uri, &cmd_tx).await;
                        }
                        Some(_) => {}
                        None => playing_stalled_since = Some(Instant::now()),
                    }
                } else {
                    playing_stalled_since = None;
                    last_position_ms = position_ms;
                }
            }
            _ => {
                loading_since = None;
                playing_stalled_since = None;
                last_position_ms = position_ms;
            }
        }
    }
}

async fn handle_recovery(
    youtube_player: &YouTubePlayer,
    spotify_recovery: &Arc<SpotifyRecovery>,
    service: Option<Service>,
    uri: &Option<String>,
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<BotCommand>,
) {
    match service {
        Some(Service::YouTube) => {
            tracing::warn!("Watchdog: YouTube stream stalled, auto-recovering");
            youtube_player.stop();
            if let Some(track_uri) = uri {
                let _ = cmd_tx.send(BotCommand::SearchAndPlay {
                    user_id: 0,
                    user_name: "Watchdog".to_string(),
                    query: track_uri.clone(),
                });
            }
        }
        Some(Service::Spotify) => {
            tracing::warn!("Watchdog: Spotify stream stalled, auto-recovering via recover_spotify");
            let _ = recover_spotify(spotify_recovery).await;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watchdog_constants_match_spec() {
        assert_eq!(LOADING_STALL_LIMIT, Duration::from_secs(15));
        assert_eq!(PLAYING_STALL_LIMIT, Duration::from_secs(10));
    }
}
