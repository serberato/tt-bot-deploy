//! Playback command handler module.
//!
//! Handles `Play`, `Pause`, `Stop`, `Next`, `Prev`, `Seek`, `SetVolume`,
//! `SetMode`, `Replay`, `PreloadNext`, and `TrackEnded`.

use std::sync::atomic::Ordering;

use librespot_core::spotify_uri::SpotifyUri;

use crate::bot::commands::{BotCommand, PlaybackMode};
use crate::bot::controller::{auto_advance_is_stale, spawn_drained_advance};
use crate::bot::handlers::HandlerContext;
use crate::bot::runner::RunnerEvent;
use crate::bot::state::PlaybackStatus;
use crate::i18n::Key;
use crate::services::Service;

pub fn handle_play(ctx: &mut HandlerContext, _user_id: i32) {
    if let Some(name) = ctx.controller.play() {
        ctx.announce_playing(&name);
    }
}

pub fn handle_pause(ctx: &mut HandlerContext, _user_id: i32) {
    if let Some(name) = ctx.controller.pause() {
        ctx.announce_playing(&name);
    }
    ctx.announcer.send_event(RunnerEvent::Idle);
}

pub fn handle_stop(ctx: &mut HandlerContext, _user_id: i32) {
    ctx.controller.stop_playback();
    {
        let mut s = ctx.controller.state.lock();
        s.clear();
    }
    ctx.announce_idle();
}

async fn try_radio_fallback(
    ctx: &mut HandlerContext,
    user_id: i32,
    seed_uri: Option<String>,
    played_ids: &[String],
) -> bool {
    let seed = match seed_uri.and_then(|s| SpotifyUri::from_uri(&s).ok()) {
        Some(s) => s,
        None => return false,
    };
    ctx.reply_t(user_id, Key::RadioFetching, &[]);
    let radio_res = ctx
        .metadata
        .get_radio_tracks(&seed, ctx.channel.radio_batch_size as usize, played_ids)
        .await;
    if radio_res.is_err() {
        ctx.notify_recovery_if_invalid();
    }
    match radio_res {
        Ok(tracks) if !tracks.is_empty() => {
            let tracks: Vec<crate::track::Track> = tracks.into_iter().map(Into::into).collect();
            let first_uri = tracks[0].uri().to_string();
            let first_name = tracks[0].display_name();
            {
                let mut s = ctx.controller.state.lock();
                s.enqueue_all(tracks, "Radio", true);
            }
            if ctx.start_or_skip(Service::Spotify, &first_uri, user_id, &first_name) {
                ctx.reply_t(user_id, Key::RadioPlaying, &[("track", first_name.clone())]);
                ctx.announce_playing(&first_name);
                return true;
            }
            false
        }
        Ok(_) => {
            ctx.reply_t(user_id, Key::RadioNoRecs, &[]);
            false
        }
        Err(e) => {
            ctx.reply_t(
                user_id,
                Key::RadioFailed,
                &[("error", crate::bot::commands::user_error(&e))],
            );
            false
        }
    }
}

fn finish_next_playback(
    ctx: &mut HandlerContext,
    user_id: i32,
    prev_index: Option<usize>,
    resumed: bool,
) {
    if !resumed {
        let was_playing = {
            let s = ctx.controller.state.lock();
            s.status == PlaybackStatus::Playing || s.status == PlaybackStatus::Paused
        };
        if user_id > 0 && was_playing {
            ctx.controller.state.lock().current_index = prev_index;
        } else {
            ctx.controller.stop_playback();
            {
                let mut s = ctx.controller.state.lock();
                s.position_ms = 0;
            }
            ctx.announce_idle();
        }
    }
}

pub async fn handle_next(ctx: &mut HandlerContext, user_id: i32, after_track: Option<String>) {
    {
        let current = ctx.controller.state.lock().current().map(|e| e.track.uri().to_string());
        if auto_advance_is_stale(after_track.as_deref(), current.as_deref()) {
            tracing::debug!("Dropping stale auto-advance");
            return;
        }
    }

    let (pre_seed_uri, pre_allow_rec, pre_played_ids) = {
        let s = ctx.controller.state.lock();
        let seed = s.current().map(|e| e.track.uri().to_string());
        let allow = s.current().map(|e| e.allow_recommend).unwrap_or(false);
        let played: Vec<String> = s.queue.iter().map(|e| e.track.id().to_string()).collect();
        (seed, allow, played)
    };

    let (next, prev_index) = {
        let mut s = ctx.controller.state.lock();
        let prev_index = s.current_index;
        let next = s.advance().map(|e| (
            e.track.service(),
            e.track.uri().to_string(),
            e.track.display_name(),
        ));
        (next, prev_index)
    };

    if let Some((service, uri_str, name)) = next {
        if ctx.start_or_skip(service, &uri_str, user_id, &name) {
            ctx.reply_t(user_id, Key::NowPlaying, &[("track", name.clone())]);
            ctx.announce_playing(&name);

            let (radio_on, at_end, allow_rec) = {
                let s = ctx.controller.state.lock();
                let at_end = s.current_index.map(|i| i + 3 >= s.queue.len()).unwrap_or(true);
                let allow = s.current().map(|e| e.allow_recommend).unwrap_or(false);
                (s.radio_enabled, at_end, allow)
            };
            if radio_on && at_end && allow_rec {
                crate::bot::handlers::radio::schedule_radio_prefetch(
                    &ctx.channel.radio_cmd_tx,
                    uri_str,
                    ctx.channel.radio_delay,
                    &ctx.channel.radio_prefetch_slot,
                );
            }
        }
    } else {
        let radio_on = ctx.controller.state.lock().radio_enabled;
        let mut resumed = false;
        if radio_on && pre_allow_rec {
            resumed = try_radio_fallback(ctx, user_id, pre_seed_uri, &pre_played_ids).await;
        } else if user_id > 0 {
            ctx.reply_t(user_id, Key::EndOfQueue, &[]);
        }
        finish_next_playback(ctx, user_id, prev_index, resumed);
    }
}

pub fn handle_prev(ctx: &mut HandlerContext, user_id: i32) {
    let prev = {
        let mut s = ctx.controller.state.lock();
        s.go_prev().map(|e| (e.track.service(), e.track.uri().to_string(), e.track.display_name()))
    };
    if let Some((service, uri_str, name)) = prev {
        if ctx.start_or_skip(service, &uri_str, user_id, &name) {
            ctx.reply_t(user_id, Key::NowPlaying, &[("track", name.clone())]);
            ctx.announce_playing(&name);
        }
    } else if user_id > 0 {
        ctx.reply_t(user_id, Key::StartOfQueue, &[]);
    }
}

pub fn handle_seek(ctx: &mut HandlerContext, offset_ms: i32, _user_id: i32) {
    ctx.controller.seek(offset_ms);
}

pub fn handle_set_volume(ctx: &mut HandlerContext, _percent: u8, _user_id: i32) {
    if !ctx.lifecycle.pending_volume_save.load(Ordering::Relaxed) {
        ctx.lifecycle.pending_volume_save.store(true, Ordering::Relaxed);
        let save_flag = ctx.lifecycle.pending_volume_save.clone();
        let vol_ref = ctx.lifecycle.volume_for_save.clone();
        let store = ctx.lifecycle.config_store.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let vol = vol_ref.load(Ordering::Relaxed);
            store.update(|cfg| {
                cfg.volume = vol;
            });
            save_flag.store(false, Ordering::Relaxed);
        });
    }
}

pub fn handle_set_mode(ctx: &mut HandlerContext, mode: PlaybackMode, _user_id: i32) {
    let mut s = ctx.controller.state.lock();
    match mode {
        PlaybackMode::RepeatTrack => {
            s.repeat_track = true;
            s.repeat_queue = false;
            s.shuffle = false;
        }
        PlaybackMode::RepeatQueue => {
            s.repeat_track = false;
            s.repeat_queue = true;
            s.shuffle = false;
        }
        PlaybackMode::Shuffle => {
            s.repeat_track = false;
            s.repeat_queue = false;
            s.shuffle = true;
        }
        PlaybackMode::Off => {
            s.repeat_track = false;
            s.repeat_queue = false;
            s.shuffle = false;
        }
    }
}

pub fn handle_replay(ctx: &mut HandlerContext, _user_id: i32) {
    if let Some(name) = ctx.controller.replay() {
        ctx.announce_playing(&name);
    }
}

pub fn handle_preload_next(ctx: &mut HandlerContext) {
    let next_uri = {
        let s = ctx.controller.state.lock();
        if s.repeat_track {
            s.current().map(|e| e.track.uri().to_string())
        } else if let Some(idx) = s.current_index {
            let next = idx + 1;
            if next < s.queue.len() {
                Some(s.queue[next].track.uri().to_string())
            } else if s.repeat_queue && !s.queue.is_empty() {
                Some(s.queue[0].track.uri().to_string())
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(uri_str) = next_uri {
        ctx.controller.preload(&uri_str);
    }
}

pub fn handle_track_ended(ctx: &mut HandlerContext, generation: u64, error: Option<String>) {
    if ctx.controller.is_stale_generation(generation) {
        tracing::debug!("Ignoring stale YouTube TrackEnded (gen {generation})");
        return;
    }
    if let Some(ref e) = error {
        tracing::warn!("YouTube track ended with error: {e}");
        if ctx.spotify.start_brake.on_failure() {
            tracing::warn!("Consecutive YouTube track failures, stopping playback");
            ctx.brake_stop();
            return;
        }
    } else {
        ctx.spotify.start_brake.on_success();
    }
    let ended_uri = ctx.controller.state.lock().current().map(|e| e.track.uri().to_string());
    if error.is_some() {
        let _ = ctx.channel.radio_cmd_tx.send(BotCommand::Next {
            user_id: 0,
            after_track: ended_uri,
        });
    } else {
        spawn_drained_advance(
            ctx.channel.radio_cmd_tx.clone(),
            ctx.controller.pipeline_drained.clone(),
            ctx.controller.pause_flag.clone(),
            ended_uri,
        );
    }
}
