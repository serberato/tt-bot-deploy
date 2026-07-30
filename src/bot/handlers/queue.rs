//! Queue command handler module.
//!
//! Handles `SearchAndPlay`, `SearchOnly`, `SearchPick`, `QueueClear`, and
//! `QueueRemove`, as well as background bulk loading and wait time calculations.

use std::sync::Arc;
use std::time::Duration;

use librespot_core::spotify_uri::SpotifyUri;

use crate::bot::commands::format_search_results;
use crate::bot::handlers::HandlerContext;
use crate::bot::state::{PlaybackStatus, PlayerState, SharedState};
use crate::error::BotError;
use crate::i18n::Key;
use crate::services::Service;
use crate::spotify::metadata::SpotifyMetadata;
use crate::youtube::metadata::{YouTubeMetadata, YtPlaylistRest, YtResolved};

const BULK_BG_BATCH: usize = 25;
const BULK_BG_DELAY: Duration = Duration::from_secs(1);

/// The not-yet-loaded remainder of a bulk source, per service: Spotify tracks
/// resolve from a URI list, YouTube playlists from a page continuation.
pub(crate) enum BulkRest {
    Spotify(Vec<SpotifyUri>),
    YouTube(YtPlaylistRest),
}

/// Computes the user-facing queue wait string (e.g. `(next, ~1 min)` or `(3 ahead, ~4 min)`).
pub(crate) fn queue_wait_info(state: &PlayerState) -> String {
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
    type ResolveOk = (Vec<crate::track::Track>, Option<BulkRest>, bool);

    let result: Result<ResolveOk, BotError> = match active {
        Service::Spotify => {
            if let Err(e) = ctx.ensure_spotify().await {
                ctx.reply_t(
                    user_id,
                    Key::SpotifyUnavailable,
                    &[("error", crate::bot::commands::user_error(&e))],
                );
                return;
            }
            let res = ctx.metadata.resolve(&query, ctx.search_limit).await;
            if res.is_err() {
                ctx.notify_recovery_if_invalid();
            }
            res.map(|r| {
                let rest = (!r.remaining.is_empty()).then_some(BulkRest::Spotify(r.remaining));
                (r.tracks.into_iter().map(Into::into).collect(), rest, r.bulk)
            })
        }
        Service::YouTube => {
            ctx.youtube_metadata
                .resolve_paged(&query, ctx.search_limit)
                .await
                .map(|resolved| match resolved {
                    YtResolved::Tracks(v) => (v.into_iter().map(Into::into).collect(), None, false),
                    YtResolved::PlaylistFirstPage { tracks, rest } => (
                        tracks.into_iter().map(Into::into).collect(),
                        rest.map(BulkRest::YouTube),
                        true,
                    ),
                })
        }
    };

    match result {
        Ok((tracks, bulk_rest, is_bulk)) => {
            if tracks.is_empty() {
                ctx.reply_t(user_id, Key::NoResults, &[]);
                return;
            }

            let is_multi = tracks.len() > 1 || is_bulk;
            let tracks_to_add = tracks;

            let first_name = tracks_to_add[0].display_name();
            let first_uri = tracks_to_add[0].uri().to_string();
            let first_service = tracks_to_add[0].service();

            let (should_start, loader_gen, count, added_name) = {
                let mut s = ctx.controller.state.lock();
                let play_mode = ctx.config_store.get().play_mode;
                let idle = s.status == PlaybackStatus::Idle;
                let is_direct = play_mode == crate::config::PlayMode::Direct;
                let should_start = idle || is_direct;
                if should_start {
                    s.clear();
                }
                let fresh = if is_multi && !is_direct {
                    s.filter_unqueued(tracks_to_add)
                } else {
                    tracks_to_add
                };
                let count = fresh.len();
                let added_name = fresh.first().map(|t| t.display_name());
                s.enqueue_all(fresh, &user_name, !is_multi);
                let generation = if bulk_rest.is_some() {
                    Some(s.begin_bulk_load())
                } else {
                    None
                };
                (should_start, generation, count, added_name)
            };

            if let (Some(gen), Some(rest)) = (loader_gen, bulk_rest) {
                match rest {
                    BulkRest::Spotify(uris) => spawn_bulk_loader(
                        ctx.metadata.clone(),
                        ctx.controller.state.clone(),
                        uris,
                        user_name.clone(),
                        gen,
                    ),
                    BulkRest::YouTube(rest) => spawn_youtube_bulk_loader(
                        ctx.youtube_metadata.clone(),
                        ctx.controller.state.clone(),
                        rest,
                        user_name.clone(),
                        gen,
                    ),
                }
            }

            let more = if loader_gen.is_some() {
                ctx.announcer.i18n.tr(user_id, Key::MoreLoading, &[])
            } else {
                String::new()
            };

            if should_start {
                if ctx.start_or_skip(first_service, &first_uri, user_id, &first_name) {
                    if count > 1 {
                        ctx.reply_t(
                            user_id,
                            Key::NowPlayingQueued,
                            &[
                                ("track", first_name.clone()),
                                ("count", (count - 1).to_string()),
                                ("more", more.clone()),
                            ],
                        );
                    } else {
                        ctx.reply_t(user_id, Key::NowPlaying, &[("track", first_name.clone())]);
                    }
                    ctx.announce_playing(&first_name);

                    if !is_multi {
                        let radio_on = ctx.controller.state.lock().radio_enabled;
                        if radio_on {
                            crate::bot::handlers::radio::schedule_radio_prefetch(
                                &ctx.radio_cmd_tx,
                                first_uri.clone(),
                                ctx.radio_delay,
                                &ctx.radio_prefetch_slot,
                            );
                        }
                    }
                }
            } else {
                let upcoming = queue_wait_info(&ctx.controller.state.lock());
                let msg = if count == 0 {
                    if loader_gen.is_some() {
                        ctx.announcer.i18n.tr(user_id, Key::AlreadyQueuedLoadingRest, &[])
                    } else {
                        ctx.announcer.i18n.tr(user_id, Key::AlreadyInQueue, &[])
                    }
                } else if count > 1 {
                    ctx.announcer.i18n.tr(
                        user_id,
                        Key::QueuedMany,
                        &[
                            ("count", count.to_string()),
                            ("upcoming", upcoming),
                            ("more", more.clone()),
                        ],
                    )
                } else {
                    let name = added_name.as_deref().unwrap_or(&first_name);
                    ctx.announcer.i18n.tr(
                        user_id,
                        Key::QueuedOne,
                        &[
                            ("track", name.to_string()),
                            ("upcoming", upcoming),
                            ("more", more.clone()),
                        ],
                    )
                };
                ctx.announcer.reply(user_id, &msg);
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
            let res = ctx.metadata.search_tracks(&query, ctx.search_limit).await;
            if res.is_err() {
                ctx.notify_recovery_if_invalid();
            }
            res.map(|tracks| tracks.into_iter().map(Into::into).collect())
        }
        Service::YouTube => {
            ctx.youtube_metadata
                .search_tracks(&query, ctx.search_limit)
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
                        &ctx.radio_cmd_tx,
                        uri_str.clone(),
                        ctx.radio_delay,
                        &ctx.radio_prefetch_slot,
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

fn spawn_youtube_bulk_loader(
    metadata: Arc<YouTubeMetadata>,
    state: SharedState,
    mut rest: YtPlaylistRest,
    requester: String,
    generation: u64,
) {
    tokio::spawn(async move {
        loop {
            if state.lock().bulk_load_generation != generation {
                return;
            }
            let page = match metadata.fetch_more_playlist(&mut rest).await {
                Ok(Some(tracks)) => tracks,
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!("YouTube background playlist load stopped early: {e}");
                    break;
                }
            };
            let batch: Vec<crate::track::Track> = page.into_iter().map(Into::into).collect();
            {
                let mut s = state.lock();
                if s.bulk_load_generation != generation {
                    return;
                }
                let fresh = s.filter_unqueued(batch);
                if !fresh.is_empty() {
                    s.enqueue_all(fresh, &requester, false);
                }
            }
            tokio::time::sleep(BULK_BG_DELAY).await;
        }
        tracing::info!("Background YouTube playlist load complete");
    });
}

fn spawn_bulk_loader(
    metadata: SpotifyMetadata,
    state: SharedState,
    uris: Vec<SpotifyUri>,
    requester: String,
    generation: u64,
) {
    tokio::spawn(async move {
        for chunk in uris.chunks(BULK_BG_BATCH) {
            if state.lock().bulk_load_generation != generation {
                return;
            }
            let tracks = metadata.fetch_tracks_meta(chunk).await;
            let batch: Vec<crate::track::Track> = tracks.into_iter().map(Into::into).collect();
            {
                let mut s = state.lock();
                if s.bulk_load_generation != generation {
                    return;
                }
                let fresh = s.filter_unqueued(batch);
                if !fresh.is_empty() {
                    s.enqueue_all(fresh, &requester, false);
                }
            }
            tokio::time::sleep(BULK_BG_DELAY).await;
        }
        tracing::info!("Background bulk load complete");
    });
}
