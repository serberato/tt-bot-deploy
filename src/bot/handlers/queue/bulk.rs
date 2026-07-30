//! Background bulk loaders for Spotify playlists/albums and YouTube playlists.

use std::sync::Arc;
use std::time::Duration;

use librespot_core::spotify_uri::SpotifyUri;

use crate::bot::handlers::HandlerContext;
use crate::bot::state::SharedState;
use crate::spotify::metadata::SpotifyMetadata;
use crate::youtube::metadata::{YouTubeMetadata, YtPlaylistRest};

pub(crate) const BULK_BG_BATCH: usize = 25;
pub(crate) const BULK_BG_DELAY: Duration = Duration::from_secs(1);

/// The not-yet-loaded remainder of a bulk source, per service.
pub(crate) enum BulkRest {
    Spotify(Vec<SpotifyUri>),
    YouTube(YtPlaylistRest),
}

pub(crate) fn spawn_bulk_loader_for_rest(
    ctx: &mut HandlerContext,
    rest: BulkRest,
    user_name: String,
    gen: u64,
) {
    match rest {
        BulkRest::Spotify(uris) => spawn_bulk_loader(
            ctx.metadata.clone(),
            ctx.controller.state.clone(),
            uris,
            user_name,
            gen,
        ),
        BulkRest::YouTube(rest) => spawn_youtube_bulk_loader(
            ctx.youtube_metadata.clone(),
            ctx.controller.state.clone(),
            rest,
            user_name,
            gen,
        ),
    }
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
