//! Radio command handler module.
//!
//! Handles `RadioToggle` and `RadioPreFetch` commands, as well as background
//! scheduling for radio recommendation fetching.

use std::sync::Arc;
use std::time::Duration;

use librespot_core::spotify_uri::SpotifyUri;
use tokio::sync::mpsc::UnboundedSender;

use crate::bot::commands::BotCommand;
use crate::bot::handlers::HandlerContext;
use crate::i18n::Key;

pub fn handle_radio_toggle(ctx: &mut HandlerContext, enable: bool, user_id: i32) {
    ctx.controller.state.lock().radio_enabled = enable;
    if enable {
        ctx.reply_t(user_id, Key::RadioEnabled, &[]);
    } else {
        ctx.reply_t(user_id, Key::RadioDisabled, &[]);
    }
}

pub async fn handle_radio_prefetch(ctx: &mut HandlerContext, seed_uri: String) {
    if !ctx.controller.state.lock().radio_enabled {
        return;
    }
    if let Ok(seed_parsed) = SpotifyUri::from_uri(&seed_uri) {
        let played_ids: Vec<String> = {
            let s = ctx.controller.state.lock();
            s.queue.iter().map(|e| e.track.id().to_string()).collect()
        };
        let radio_res = ctx
            .metadata
            .get_radio_tracks(
                &seed_parsed,
                ctx.channel.radio_batch_size as usize,
                &played_ids,
            )
            .await;
        if radio_res.is_err() {
            ctx.notify_recovery_if_invalid();
        }
        match radio_res {
            Ok(tracks) if !tracks.is_empty() => {
                let tracks: Vec<crate::track::Track> = tracks.into_iter().map(Into::into).collect();
                let mut s = ctx.controller.state.lock();
                if !s.radio_enabled {
                    return;
                }
                s.enqueue_all(tracks, "Radio", true);
                tracing::debug!("Pre-fetched radio recommendations from {seed_uri}");
            }
            Ok(_) => {
                tracing::debug!("No radio recommendations found for {seed_uri}");
            }
            Err(e) => {
                tracing::warn!("Failed to pre-fetch radio tracks: {e}");
            }
        }
    }
}

/// Schedules a delayed radio pre-fetch task, cancelling any previously scheduled one.
pub(crate) fn schedule_radio_prefetch(
    radio_cmd_tx: &UnboundedSender<BotCommand>,
    seed_uri: String,
    radio_delay: f32,
    slot: &Arc<parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>>,
) {
    if radio_delay <= 0.0 {
        let _ = radio_cmd_tx.send(BotCommand::RadioPreFetch { seed_uri });
    } else {
        let tx = radio_cmd_tx.clone();
        let dur = Duration::from_secs_f32(radio_delay);
        let mut guard = slot.lock();
        if let Some(prev) = guard.take() {
            prev.abort();
        }
        *guard = Some(tokio::spawn(async move {
            tokio::time::sleep(dur).await;
            let _ = tx.send(BotCommand::RadioPreFetch { seed_uri });
        }));
    }
}
