//! Search query resolution for Spotify and YouTube queueing.

use crate::bot::handlers::HandlerContext;
use crate::error::BotError;
use crate::i18n::Key;
use crate::services::Service;
use crate::youtube::metadata::YtResolved;
use super::bulk::BulkRest;

type ResolveOk = (Vec<crate::track::Track>, Option<BulkRest>, bool);

async fn resolve_spotify(
    ctx: &mut HandlerContext,
    query: &str,
    user_id: i32,
) -> Result<Option<ResolveOk>, BotError> {
    if let Err(e) = ctx.ensure_spotify().await {
        ctx.reply_t(
            user_id,
            Key::SpotifyUnavailable,
            &[("error", crate::bot::commands::user_error(&e))],
        );
        return Ok(None);
    }
    let res = ctx.metadata.resolve(query, ctx.channel.search_limit).await;
    if res.is_err() {
        ctx.notify_recovery_if_invalid();
    }
    let r = res?;
    let rest = (!r.remaining.is_empty()).then_some(BulkRest::Spotify(r.remaining));
    Ok(Some((r.tracks.into_iter().map(Into::into).collect(), rest, r.bulk)))
}

async fn resolve_youtube(
    ctx: &mut HandlerContext,
    query: &str,
) -> Result<Option<ResolveOk>, BotError> {
    let resolved = ctx
        .youtube_metadata
        .resolve_paged(query, ctx.channel.search_limit)
        .await?;
    match resolved {
        YtResolved::Tracks(v) => Ok(Some((v.into_iter().map(Into::into).collect(), None, false))),
        YtResolved::PlaylistFirstPage { tracks, rest } => Ok(Some((
            tracks.into_iter().map(Into::into).collect(),
            rest.map(BulkRest::YouTube),
            true,
        ))),
    }
}

pub(crate) async fn resolve_search_query(
    ctx: &mut HandlerContext,
    query: &str,
    user_id: i32,
    active: Service,
) -> Option<ResolveOk> {
    let result = match active {
        Service::Spotify => resolve_spotify(ctx, query, user_id).await,
        Service::YouTube => resolve_youtube(ctx, query).await,
    };

    match result {
        Ok(res) => res,
        Err(e) => {
            ctx.reply_t(
                user_id,
                Key::SearchFailed,
                &[("error", crate::bot::commands::user_error(&e))],
            );
            None
        }
    }
}
