//! Queue command handler module.
//!
//! Handles `SearchAndPlay`, `SearchOnly`, `SearchPick`, `QueueClear`, and
//! `QueueRemove`, delegating resolution and background bulk loading to
//! submodules.

pub mod bulk;
pub mod playback;
pub mod resolve;

use crate::bot::commands::format_search_results;
use crate::bot::handlers::HandlerContext;
use crate::bot::state::{PlaybackStatus, PlayerState};
use crate::error::BotError;
use crate::i18n::Key;
use crate::services::Service;

use bulk::spawn_bulk_loader_for_rest;
use playback::{enqueue_resolved_tracks, handle_queued_playback, handle_should_start_playback};
use resolve::resolve_search_query;

/// Computes the user-facing queue wait string (e.g. `(next, ~1 min)` or `(3 ahead, ~4 min)`).
pub fn queue_wait_info(state: &PlayerState) -> String {
    let current_idx = match state.current_index {
        Some(i) => i,
        None => return String::new(),
    };
    let total = state.queue.len();
    if total <= current_idx + 1 {
        return String::new();
    }
    let upcoming_pos = total - current_idx - 1;
    let mut wait_ms: u64 = 0;
    if let Some(current) = state.queue.get(current_idx) {
        wait_ms += current.track.duration_ms().saturating_sub(state.position_ms) as u64;
    }
    for entry in state.queue.iter().skip(current_idx + 1).take(upcoming_pos - 1) {
        wait_ms += entry.track.duration_ms() as u64;
    }
    let wait_min = (wait_ms + 30_000) / 60_000;
    let pos_str = match upcoming_pos {
        1 => "next".to_string(),
        _ => format!("{upcoming_pos} ahead"),
    };
    if wait_min > 0 {
        format!(" ({pos_str}, ~{wait_min} min)")
    } else {
        format!(" ({pos_str})")
    }
}

pub async fn handle_search_and_play(
    ctx: &mut HandlerContext,
    query: String,
    user_id: i32,
    user_name: String,
) {
    let active = ctx.controller.state.lock().active_service;
    let (tracks, bulk_rest, is_bulk) =
        match resolve_search_query(ctx, &query, user_id, active).await {
            Some(res) => res,
            None => return,
        };
    if tracks.is_empty() {
        ctx.reply_t(user_id, Key::NoResults, &[]);
        return;
    }

    let is_multi = tracks.len() > 1 || is_bulk;
    let first_name = tracks[0].display_name();
    let first_uri = tracks[0].uri().to_string();
    let first_service = tracks[0].service();
    let has_rest = bulk_rest.is_some();
    let play_mode = ctx.lifecycle.config_store.get().play_mode;

    let res = enqueue_resolved_tracks(
        &ctx.controller.state,
        play_mode,
        tracks,
        &user_name,
        has_rest,
        is_multi,
    );

    if let (Some(gen), Some(rest)) = (res.loader_gen, bulk_rest) {
        spawn_bulk_loader_for_rest(ctx, rest, user_name, gen);
    }

    let more = if res.loader_gen.is_some() {
        ctx.announcer.i18n.tr(user_id, Key::MoreLoading, &[])
    } else {
        String::new()
    };

    if res.should_start {
        handle_should_start_playback(
            ctx,
            first_service,
            first_uri,
            first_name,
            res.count,
            more,
            is_multi,
            user_id,
        );
    } else {
        let upcoming = queue_wait_info(&ctx.controller.state.lock());
        handle_queued_playback(
            ctx,
            res.count,
            upcoming,
            more,
            res.added_name,
            first_name,
            user_id,
            res.loader_gen,
        );
    }
}

pub async fn handle_search_only(ctx: &mut HandlerContext, query: String, user_id: i32) {
    let active = ctx.controller.state.lock().active_service;
    type SearchOk = Vec<crate::track::Track>;
    let result: Result<SearchOk, BotError> = match active {
        Service::Spotify => {
            if let Err(e) = ctx.ensure_spotify().await {
                ctx.reply_t(
                    user_id,
                    Key::SpotifyUnavailable,
                    &[("error", crate::bot::commands::user_error(&e))],
                );
                return;
            }
            let res = ctx.metadata.search_tracks(&query, ctx.channel.search_limit).await;
            if res.is_err() {
                ctx.notify_recovery_if_invalid();
            }
            res.map(|tracks| tracks.into_iter().map(Into::into).collect())
        }
        Service::YouTube => {
            ctx.youtube_metadata
                .search_tracks(&query, ctx.channel.search_limit)
                .await
                .map(|tracks| tracks.into_iter().map(Into::into).collect())
        }
    };

    match result {
        Ok(tracks) => {
            if tracks.is_empty() {
                ctx.reply_t(user_id, Key::NoResults, &[]);
            } else {
                let header = ctx.announcer.i18n.tr(user_id, Key::SearchResultsHeader, &[]);
                let footer = ctx.announcer.i18n.tr(user_id, Key::SearchResultsFooter, &[]);
                let formatted = format_search_results(&tracks, &header, &footer);
                ctx.controller.state.lock().insert_search_results(user_id, tracks);
                ctx.announcer.reply(user_id, &formatted);
            }
        }
        Err(e) => {
            ctx.reply_t(
                user_id,
                Key::SearchFailed,
                &[("error", crate::bot::commands::user_error(&e))],
            );
        }
    }
}

pub fn handle_search_pick(ctx: &mut HandlerContext, user_id: i32, pick: usize, user_name: String) {
    let picked = {
        let mut s = ctx.controller.state.lock();
        let track = s.pick_search_result(user_id, pick);
        track.map(|track| {
            s.remove_search_results(user_id);
            let idle = s.status == PlaybackStatus::Idle;
            if idle {
                s.clear();
            }
            let service = track.service();
            let uri_str = track.uri().to_string();
            let track_name = track.display_name();
            s.enqueue(track, user_name, true);
            (service, uri_str, track_name, idle)
        })
    };
    if let Some((service, uri_str, track_name, is_idle)) = picked {
        if is_idle {
            if ctx.start_or_skip(service, &uri_str, user_id, &track_name) {
                ctx.reply_t(user_id, Key::NowPlaying, &[("track", track_name.clone())]);
                ctx.announce_playing(&track_name);

                let radio_on = ctx.controller.state.lock().radio_enabled;
                if radio_on {
                    crate::bot::handlers::radio::schedule_radio_prefetch(
                        &ctx.channel.radio_cmd_tx,
                        uri_str.clone(),
                        ctx.channel.radio_delay,
                        &ctx.channel.radio_prefetch_slot,
                    );
                }
            }
        } else {
            let upcoming = queue_wait_info(&ctx.controller.state.lock());
            ctx.reply_t(
                user_id,
                Key::QueuedOne,
                &[
                    ("track", track_name),
                    ("upcoming", upcoming),
                    ("more", String::new()),
                ],
            );
        }
    } else {
        ctx.reply_t(user_id, Key::InvalidPick, &[]);
    }
}

pub fn handle_queue_clear(ctx: &mut HandlerContext, _user_id: i32) {
    ctx.controller.state.lock().clear_upcoming();
}

pub fn handle_queue_remove(ctx: &mut HandlerContext, index: usize, _user_id: i32) {
    let mut s = ctx.controller.state.lock();
    s.remove(index);
}
