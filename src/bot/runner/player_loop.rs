use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use librespot_playback::player::PlayerEvent;

use crate::bot::commands::BotCommand;
use crate::bot::controller::spawn_drained_advance;
use crate::bot::state::{PlaybackStatus, SharedState};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn player_event_loop(
    mut events: librespot_playback::player::PlayerEventChannel,
    state: SharedState,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    recovery_notify: Arc<tokio::sync::Notify>,
    pipeline_drained: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    spotify_brake: Arc<parking_lot::Mutex<crate::bot::controller::StartFailureBrake>>,
) {
    while let Some(event) = events.recv().await {
        match event {
            PlayerEvent::Playing { position_ms, .. } => {
                let mut s = state.lock();
                s.status = PlaybackStatus::Playing;
                s.position_ms = position_ms;
                spotify_brake.lock().on_success();
            }
            PlayerEvent::Paused { position_ms, .. } => {
                let mut s = state.lock();
                s.status = PlaybackStatus::Paused;
                s.position_ms = position_ms;
            }
            PlayerEvent::EndOfTrack { track_id, .. } => {
                if session.lock().is_invalid() {
                    tracing::warn!("EndOfTrack with dead Spotify session; triggering recovery instead of advancing");
                    recovery_notify.notify_one();
                    continue;
                }
                let is_current = {
                    let s = state.lock();
                    match (s.current().map(|e| e.track.uri().to_string()), track_id.to_uri()) {
                        (Some(cur_uri), Ok(ended_uri)) => cur_uri == ended_uri,
                        _ => true,
                    }
                };
                if is_current {
                    tracing::info!("Track ended (decode); waiting for the buffered tail to play out");
                    spawn_drained_advance(
                        cmd_tx.clone(),
                        pipeline_drained.clone(),
                        pause_flag.clone(),
                        track_id.to_uri().ok(),
                    );
                } else {
                    tracing::debug!("Ignoring stale Spotify EndOfTrack for {track_id:?}");
                }
            }
            PlayerEvent::Unavailable { track_id, .. } => {
                tracing::warn!("Track unavailable: {track_id:?}, skipping");
                let mut guard = spotify_brake.lock();
                if guard.on_failure() {
                    drop(guard);
                    tracing::warn!("Spotify circuit breaker tripped after 3 consecutive failures; stopping playback");
                    let _ = cmd_tx.send(BotCommand::CircuitBreakerTrip {
                        service: crate::services::Service::Spotify,
                    });
                } else {
                    let delay = guard.backoff_duration();
                    drop(guard);
                    let tx = cmd_tx.clone();
                    let after_uri = track_id.to_uri().ok();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = tx.send(BotCommand::Next {
                            user_id: 0,
                            after_track: after_uri,
                        });
                    });
                }
            }
            PlayerEvent::TimeToPreloadNextTrack { .. } => {
                let _ = cmd_tx.send(BotCommand::PreloadNext);
            }
            PlayerEvent::PositionChanged { position_ms, .. }
            | PlayerEvent::PositionCorrection { position_ms, .. }
            | PlayerEvent::Seeked { position_ms, .. } => {
                let mut s = state.lock();
                s.position_ms = position_ms;
            }
            _ => {}
        }
    }
}
