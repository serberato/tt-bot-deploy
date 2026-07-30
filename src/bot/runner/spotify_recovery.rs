use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::bot::commands::BotCommand;
use crate::bot::runner::RunnerEvent;
use crate::bot::state::{PlaybackStatus, SharedState};
use crate::config::BotConfig;
use crate::error::BotError;
use crate::spotify::player::SpotifyPlayer;
use crate::spotify::recovery::{
    delay_before_attempt, resume_seek_ms, RecoveryGuard, RecoveryOutcome, MAX_ATTEMPTS,
};

/// Everything the session-recovery supervisor needs to rebuild a dead Spotify
/// session and resume playback. All fields are cheap handles/clones.
pub(crate) struct SpotifyRecovery {
    pub session_holder: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    pub auth: Arc<crate::spotify::auth::SpotifyAuth>,
    pub config: BotConfig,
    pub audio_tx: crossbeam_channel::Sender<Vec<i16>>,
    pub player: SpotifyPlayer,
    pub state: SharedState,
    pub cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    pub pause_flag: Arc<AtomicBool>,
    pub audio_reset: Arc<AtomicBool>,
    pub guard: Arc<RecoveryGuard>,
    pub recovery_notify: Arc<tokio::sync::Notify>,
    pub local_shutdown: Arc<AtomicBool>,
    pub event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    pub pipeline_drained: Arc<AtomicBool>,
}

/// Build a brand-new Spotify session (cached credentials only — never opens a
/// browser) and rebuild the player from it, swapping both into the shared
/// holders. Returns the new player event channel for the caller to restart the
/// event loop on. librespot Sessions are single-use, so this is the only way to
/// recover a session whose connection has died.
pub(crate) async fn rebuild_spotify_engine(
    rec: &SpotifyRecovery,
) -> Result<librespot_playback::player::PlayerEventChannel, BotError> {
    if !rec.auth.has_cached_credentials() {
        return Err(BotError::Playback(
            "no cached Spotify credentials to rebuild the session".into(),
        ));
    }
    let session = rec.auth.new_session();
    rec.auth.connect_existing(&session).await?;
    *rec.session_holder.lock() = session.clone();
    let event_rx = rec
        .player
        .rebuild(session, &rec.config, rec.audio_tx.clone());
    Ok(event_rx)
}

/// Recover a dead Spotify session: pause, rebuild with bounded backoff, restart
/// the player event loop, and resume the interrupted track where it left off.
/// Single-flight via `rec.guard`.
pub(crate) async fn recover_spotify(rec: &SpotifyRecovery) -> RecoveryOutcome {
    if !rec.guard.try_begin() {
        return RecoveryOutcome::Recovered;
    }
    tracing::warn!("Spotify session died; starting bounded recovery");

    let resume = {
        let s = rec.state.lock();
        let was_paused = s.status == PlaybackStatus::Paused;
        s.current().and_then(|e| {
            if e.track.service() == crate::services::Service::Spotify {
                Some((e.track.uri().to_string(), s.position_ms, was_paused))
            } else {
                None
            }
        })
    };
    let pause_pipeline = resume.is_some();
    let resume_paused = resume.as_ref().map(|(_, _, p)| *p).unwrap_or(false);
    if pause_pipeline {
        rec.pause_flag.store(true, Ordering::Relaxed);
    }

    let mut attempt = 0usize;
    let outcome = loop {
        let Some(delay) = delay_before_attempt(attempt) else {
            break RecoveryOutcome::GaveUp;
        };
        tokio::time::sleep(delay).await;
        if rec.local_shutdown.load(Ordering::Relaxed) {
            break RecoveryOutcome::GaveUp;
        }
        match rebuild_spotify_engine(rec).await {
            Ok(event_rx) => {
                tracing::info!("Spotify session rebuilt on attempt {}", attempt + 1);
                let st = rec.state.clone();
                let tx = rec.cmd_tx.clone();
                let sh = rec.session_holder.clone();
                let notify = rec.recovery_notify.clone();
                let drained = rec.pipeline_drained.clone();
                let paused = rec.pause_flag.clone();
                tokio::spawn(async move {
                    crate::bot::runner::player_loop::player_event_loop(
                        event_rx, st, tx, sh, notify, drained, paused,
                    )
                    .await;
                });
                if let Some((uri, pos_ms, _)) = &resume {
                    if let Ok(parsed) = librespot_core::spotify_uri::SpotifyUri::from_uri(uri) {
                        rec.audio_reset.store(true, Ordering::Relaxed);
                        let seek = resume_seek_ms(*pos_ms);
                        rec.player.load_track_at(&parsed, seek);
                        if resume_paused {
                            rec.player.pause();
                        }
                        tracing::info!(
                            "Resumed {uri} at {seek}ms after recovery (paused={resume_paused})"
                        );
                    }
                }
                if pause_pipeline && !resume_paused {
                    rec.pause_flag.store(false, Ordering::Relaxed);
                }
                if let Some(tx) = &rec.event_tx {
                    let _ = tx.send(RunnerEvent::Connected);
                }
                break RecoveryOutcome::Recovered;
            }
            Err(e) => {
                tracing::error!("Spotify rebuild attempt {} failed: {e}", attempt + 1);
                attempt += 1;
            }
        }
    };

    if outcome == RecoveryOutcome::GaveUp {
        tracing::error!(
            "Spotify recovery gave up after {MAX_ATTEMPTS} attempts; playback stopped. \
             A Spotify command will retry."
        );
        if pause_pipeline {
            rec.pause_flag.store(false, Ordering::Relaxed);
        }
        if let Some(tx) = &rec.event_tx {
            let _ = tx.send(RunnerEvent::Error(
                "Spotify unreachable; playback stopped".to_string(),
            ));
        }
    }
    rec.guard.finish();
    outcome
}

/// Supervisor task: watch for a dead session and drive recovery. Polls the local
/// `session.is_invalid()` signal (free — no network) on a 1s tick, or wakes
/// immediately when notified by the event loop / a command. After a give-up it
/// stays suspended until a Spotify command clears the latch and re-notifies.
pub(crate) async fn spotify_supervisor(rec: SpotifyRecovery, recovery_suspended: Arc<AtomicBool>) {
    loop {
        tokio::select! {
            _ = rec.recovery_notify.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        if rec.local_shutdown.load(Ordering::Relaxed) {
            break;
        }
        let dead = rec.session_holder.lock().is_invalid();
        if dead
            && !recovery_suspended.load(Ordering::Relaxed)
            && recover_spotify(&rec).await == RecoveryOutcome::GaveUp
        {
            tracing::error!("Spotify recovery gave up. Exiting to allow systemd to restart the bot.");
            std::process::exit(1);
        }
    }
}
