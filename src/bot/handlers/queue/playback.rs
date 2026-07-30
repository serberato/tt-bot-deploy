//! Enqueueing and playback start helpers for resolved search results.

use crate::bot::handlers::HandlerContext;
use crate::bot::state::{PlaybackStatus, SharedState};
use crate::i18n::Key;
use crate::services::Service;

pub(crate) struct EnqueueResult {
    pub should_start: bool,
    pub loader_gen: Option<u64>,
    pub count: usize,
    pub added_name: Option<String>,
}

pub(crate) fn enqueue_resolved_tracks(
    state: &SharedState,
    play_mode: crate::config::PlayMode,
    tracks_to_add: Vec<crate::track::Track>,
    user_name: &str,
    has_rest: bool,
    is_multi: bool,
) -> EnqueueResult {
    let mut s = state.lock();
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
    s.enqueue_all(fresh, user_name, !is_multi);
    let loader_gen = if has_rest {
        Some(s.begin_bulk_load())
    } else {
        None
    };
    EnqueueResult {
        should_start,
        loader_gen,
        count,
        added_name,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_should_start_playback(
    ctx: &mut HandlerContext,
    first_service: Service,
    first_uri: String,
    first_name: String,
    count: usize,
    more: String,
    is_multi: bool,
    user_id: i32,
) {
    if ctx.start_or_skip(first_service, &first_uri, user_id, &first_name) {
        if count > 1 {
            ctx.reply_t(
                user_id,
                Key::NowPlayingQueued,
                &[
                    ("track", first_name.clone()),
                    ("count", (count - 1).to_string()),
                    ("more", more),
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
                    &ctx.channel.radio_cmd_tx,
                    first_uri,
                    ctx.channel.radio_delay,
                    &ctx.channel.radio_prefetch_slot,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_queued_playback(
    ctx: &mut HandlerContext,
    count: usize,
    upcoming: String,
    more: String,
    added_name: Option<String>,
    first_name: String,
    user_id: i32,
    loader_gen: Option<u64>,
) {
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
                ("more", more),
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
                ("more", more),
            ],
        )
    };
    ctx.announcer.reply(user_id, &msg);
}
