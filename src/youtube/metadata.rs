use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use rustypipe::client::RustyPipe;

use crate::config::BotConfig;
use crate::domain::MetadataProvider;
use crate::error::BotError;
use crate::track::Track;
use crate::youtube::setup::{default_cookies_path, resolve_paths, which, YoutubeSetupPaths};
use crate::youtube::types::{parse_youtube_ref, YouTubeRef, YouTubeTrack};

/// Opaque continuation for a playlist whose first page has been returned but
/// whose remaining pages are still on YouTube's side. Fed back into
/// `fetch_more_playlist` by the background loader.
pub struct YtPlaylistRest {
    paginator: rustypipe::model::paginator::Paginator<rustypipe::model::TrackItem>,
}

/// Result of `resolve_paged`.
pub enum YtResolved {
    /// Fully resolved (single track, album, search hit).
    Tracks(Vec<YouTubeTrack>),
    /// First playlist page; `rest` is Some when more pages exist.
    PlaylistFirstPage {
        tracks: Vec<YouTubeTrack>,
        rest: Option<YtPlaylistRest>,
    },
}

/// YouTube Music metadata service.
///
/// Search and track metadata go through rustypipe (fast, native).
/// Stream URL resolution goes through `yt-dlp` because rustypipe's
/// signature deobfuscator can't keep up with YouTube's player JS
/// changes.
pub struct YouTubeMetadata {
    client: Arc<RustyPipe>,
    /// Path passed to `yt-dlp --cookies <file>`. Empty = don't pass.
    /// Resolved at init: explicit config override → falls back to the
    /// default `<config_dir>/cookies.txt` if it exists → empty.
    cookies_file: String,
    /// Resolved paths for the bundled binaries + plugin dir.
    /// `Some` if the bot can find them; `None` falls back to PATH.
    bundle: Option<YoutubeSetupPaths>,
    /// Resolved yt-dlp executable path. PATH lookup happens once at
    /// construction; falls back to the bundled binary or the bare name.
    yt_dlp_exe: PathBuf,
}

impl YouTubeMetadata {
    pub fn new(config: &BotConfig, profile_name: &str) -> Result<Self, BotError> {
        // Keep rustypipe's cache (rustypipe_cache.json) in the config dir.
        // The default is the process working directory, which under systemd
        // may be unwritable (silently losing the cache) and during development
        // litters the repo root.
        let client = RustyPipe::builder()
            .no_botguard()
            .storage_dir(crate::config::config_dir())
            .build()
            .map_err(|e| BotError::Playback(format!("rustypipe init failed: {e}")))?;
        // Resolve bundled paths but don't require them — falling back to PATH
        // keeps the manual-install path working.
        let bundle = resolve_paths().ok().filter(|p| p.yt_dlp.is_file());

        // Cookies: explicit override wins; otherwise look for the default path.
        let cookies_file = if !config.youtube_cookies_file.is_empty() {
            config.youtube_cookies_file.clone()
        } else {
            let default = default_cookies_path(profile_name);
            if default.is_file() {
                tracing::info!("YouTube: auto-loaded cookies from {}", default.display());
                default.to_string_lossy().into_owned()
            } else {
                String::new()
            }
        };

        // Resolve yt-dlp once: prefer the bundled copy under <exe-dir>/lib since
        // its version is paired with the bundled bgutil plugin and kept current
        // by --update-tools. Fall back to a PATH install, then a bare `yt-dlp`
        // (NotFound at spawn time). A stale PATH yt-dlp otherwise wins and 403s
        // on YouTube's current PO-token requirements.
        let yt_dlp_exe = bundle.as_ref().map(|b| b.yt_dlp.clone())
            .or_else(|| which("yt-dlp"))
            .unwrap_or_else(|| PathBuf::from("yt-dlp"));

        Ok(Self {
            client: Arc::new(client),
            cookies_file,
            bundle,
            yt_dlp_exe,
        })
    }

    /// Like `resolve`, but playlists return only their first page plus a
    /// continuation so the caller can start playback immediately and pull the
    /// remaining pages in the background (mirrors Spotify bulk loading).
    pub async fn resolve_paged(&self, query: &str, search_limit: u8) -> Result<YtResolved, BotError> {
        if let Some(YouTubeRef::Playlist(id)) = parse_youtube_ref(query) {
            return self.fetch_playlist_first_page(&id).await;
        }
        self.resolve(query, search_limit).await.map(YtResolved::Tracks)
    }

    /// Resolve a YouTube URL/ID/playlist/album/search query into a list of
    /// tracks. URLs and bare IDs become single-track or playlist/album
    /// fetches; anything else falls back to the top match for the search.
    pub async fn resolve(&self, query: &str, _search_limit: u8) -> Result<Vec<YouTubeTrack>, BotError> {
        match parse_youtube_ref(query) {
            Some(YouTubeRef::Video(id)) => self.fetch_video(&id).await.map(|t| vec![t]),
            // A bare 11-char token is probably an ID but might be an
            // 11-letter search word; if the ID lookup fails, search instead
            // of surfacing "video fetch failed" for a legitimate query.
            Some(YouTubeRef::BareVideo(id)) => match self.fetch_video(&id).await {
                Ok(t) => Ok(vec![t]),
                Err(e) => {
                    tracing::debug!("Bare token '{id}' is not a video id ({e}); searching instead");
                    self.search_tracks(query, 1).await
                }
            },
            Some(YouTubeRef::Playlist(id)) => self.fetch_playlist(&id).await,
            Some(YouTubeRef::Album(id)) => self.fetch_album(&id).await,
            // A free-form search returns just the top hit so play_and_queue
            // doesn't accidentally enqueue 5 tracks for a single song name.
            None => self.search_tracks(query, 1).await,
        }
    }

    async fn fetch_video(&self, video_id: &str) -> Result<YouTubeTrack, BotError> {
        let details = self.client.query()
            .music_details(video_id)
            .await
            .map_err(|e| BotError::Playback(format!("YouTube video fetch failed: {e}")))?;
        Ok(track_item_to_track(details.track))
    }

    async fn fetch_playlist(&self, playlist_id: &str) -> Result<Vec<YouTubeTrack>, BotError> {
        let mut playlist = self.client.query()
            .music_playlist(playlist_id)
            .await
            .map_err(|e| BotError::Playback(format!("YouTube playlist fetch failed: {e}")))?;
        // Pull all pages, not just the first. A paging failure truncates the
        // list; say so instead of silently returning a partial playlist.
        if let Err(e) = playlist.tracks.extend_all(&self.client.query()).await {
            tracing::warn!("YouTube playlist only partially loaded: {e}");
        }
        let tracks: Vec<YouTubeTrack> = playlist.tracks.items.into_iter().map(track_item_to_track).collect();
        if tracks.is_empty() {
            Err(BotError::NoResults)
        } else {
            Ok(tracks)
        }
    }

    /// First page of a playlist plus a continuation for background loading.
    async fn fetch_playlist_first_page(&self, playlist_id: &str) -> Result<YtResolved, BotError> {
        let playlist = self.client.query()
            .music_playlist(playlist_id)
            .await
            .map_err(|e| BotError::Playback(format!("YouTube playlist fetch failed: {e}")))?;
        let mut paginator = playlist.tracks;
        // Drain the page out of the paginator: each later extend() appends
        // only the next page, so fetch_more_playlist can drain again and get
        // exactly the new tracks.
        let tracks: Vec<YouTubeTrack> = std::mem::take(&mut paginator.items)
            .into_iter()
            .map(track_item_to_track)
            .collect();
        if tracks.is_empty() {
            return Err(BotError::NoResults);
        }
        let rest = paginator.ctoken.is_some().then_some(YtPlaylistRest { paginator });
        Ok(YtResolved::PlaylistFirstPage { tracks, rest })
    }

    /// Fetch the next page of a partially-loaded playlist. `Ok(None)` when the
    /// playlist is exhausted.
    pub async fn fetch_more_playlist(
        &self,
        rest: &mut YtPlaylistRest,
    ) -> Result<Option<Vec<YouTubeTrack>>, BotError> {
        let more = rest.paginator.extend(self.client.query())
            .await
            .map_err(|e| BotError::Playback(format!("YouTube playlist page fetch failed: {e}")))?;
        if !more {
            return Ok(None);
        }
        let tracks: Vec<YouTubeTrack> = std::mem::take(&mut rest.paginator.items)
            .into_iter()
            .map(track_item_to_track)
            .collect();
        Ok(Some(tracks))
    }

    async fn fetch_album(&self, album_id: &str) -> Result<Vec<YouTubeTrack>, BotError> {
        let album = self.client.query()
            .music_album(album_id)
            .await
            .map_err(|e| BotError::Playback(format!("YouTube album fetch failed: {e}")))?;
        let tracks: Vec<YouTubeTrack> = album.tracks.into_iter().map(track_item_to_track).collect();
        if tracks.is_empty() {
            Err(BotError::NoResults)
        } else {
            Ok(tracks)
        }
    }

    /// Search YouTube Music and YouTube videos with query sanitization, multi-tier fallback, and scoring ranking.
    pub async fn search_tracks(&self, query: &str, limit: u8) -> Result<Vec<YouTubeTrack>, BotError> {
        let sanitized = sanitize_query(query);
        let query_lower = sanitized.to_lowercase();
        let limit_usize = limit as usize;

        let force_video_fallback = has_non_catalogue_keywords(&query_lower);

        let mut tracks = Vec::new();

        // Tier 1: Try YouTube Music tracks if not forced to video search
        if !force_video_fallback {
            if let Ok(result) = self.client.query().music_search_tracks(&sanitized).await {
                tracks = result.items.items
                    .into_iter()
                    .map(track_item_to_track)
                    .collect();
            }
        }

        // Tier 2: If Tier 1 returned 0 results or force_video_fallback is true, search YouTube videos
        if tracks.is_empty() {
            let result = self.client.query()
                .search::<rustypipe::model::VideoItem, _>(&sanitized)
                .await
                .map_err(|e| BotError::Playback(format!("YouTube search failed: {e}")))?;

            tracks = result.items.items
                .into_iter()
                .map(video_item_to_track)
                .collect();
        }

        if tracks.is_empty() {
            return Err(BotError::NoResults);
        }

        // Tier 3: Rank and sort candidates by weighted score DESC
        tracks.sort_by_cached_key(|track| -score_track(&query_lower, track));
        tracks.truncate(limit_usize);

        Ok(tracks)
    }

    /// Spawn yt-dlp as a child process that streams M4A audio bytes to its
    /// stdout. The caller owns the `Child` — drop or kill it to stop the
    /// download (and free the pipe). yt-dlp handles all of YouTube's
    /// header/cookie/fragment requirements.
    pub fn spawn_ytdlp(&self, video_id: &str) -> Result<std::process::Child, BotError> {
        use std::process::{Command, Stdio};
        let url = format!("https://www.youtube.com/watch?v={video_id}");

        let mut cmd = Command::new(&self.yt_dlp_exe);
        cmd.args([
            "--no-warnings",
            "--no-playlist",
            "-f", "bestaudio[ext=m4a]/bestaudio",
            "-o", "-",
        ]);

        // Wire the bgutil-pot plugin and binary if bundled.
        if let Some(b) = &self.bundle {
            if b.plugin_dir.is_dir() {
                // yt-dlp searches <plugin-dir>/*/yt_dlp_plugins, one level down,
                // so point it at lib_dir (which contains the yt-dlp-plugins
                // package), not at the package dir itself.
                cmd.arg("--plugin-dirs");
                cmd.arg(&b.lib_dir);
            }
            if b.bgutil_pot.is_file() {
                cmd.arg("--extractor-args");
                cmd.arg(format!(
                    "youtubepot-bgutilscript:script_path={}",
                    b.bgutil_pot.display()
                ));
            }
        }

        // Cookies (optional, helps with rate-limited / age-restricted videos).
        if !self.cookies_file.is_empty() {
            cmd.arg("--cookies");
            cmd.arg(&self.cookies_file);
        }

        // The tray is a GUI process with no console, so a child console app
        // flashes a command window on each spawn. CREATE_NO_WINDOW suppresses it
        // for yt-dlp and the bgutil-pot child it launches.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        cmd.arg("--").arg(&url)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => BotError::Playback(
                    "yt-dlp not found. Run: tt-spotify-bot --setup-yt".to_string()
                ),
                _ => BotError::Playback(format!("yt-dlp spawn: {e}")),
            })
    }
}

impl MetadataProvider for YouTubeMetadata {
    fn search_tracks<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Track>, BotError>> + Send + 'a>> {
        Box::pin(async move {
            let tracks = YouTubeMetadata::search_tracks(self, query, limit).await?;
            Ok(tracks.into_iter().map(Track::from).collect())
        })
    }

    fn resolve<'a>(
        &'a self,
        query: &'a str,
        limit: u8,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Track>, BotError>> + Send + 'a>> {
        Box::pin(async move {
            let tracks = YouTubeMetadata::resolve(self, query, limit).await?;
            Ok(tracks.into_iter().map(Track::from).collect())
        })
    }
}

/// Strip Unicode emojis, typographic quotes, and punctuation that break tokenization.
pub fn sanitize_query(query: &str) -> String {
    let mut cleaned = String::with_capacity(query.len());
    for c in query.chars() {
        match c {
            '|' | '[' | ']' | '{' | '}' | '“' | '”' | '‘' | '’' | '"' | '\'' => {
                cleaned.push(' ');
            }
            c if c.is_alphanumeric() || c.is_whitespace() || c.is_ascii_punctuation() => {
                cleaned.push(c);
            }
            _ => {
                if c.is_alphabetic() {
                    cleaned.push(c);
                } else {
                    cleaned.push(' ');
                }
            }
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_non_catalogue_keywords(query_lower: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "live", "desfile", "detenido", "circo", "tiny desk", "noticias",
        "entrevista", "parada", "militar", "vlog", "podcast", "npr",
    ];
    KEYWORDS.iter().any(|&kw| query_lower.contains(kw))
}

fn score_track(query_lower: &str, track: &YouTubeTrack) -> i32 {
    let mut score = 0i32;
    let name_lower = track.name.to_lowercase();
    let artist_lower = track.artists.join(" ").to_lowercase();
    let target = format!("{} {}", name_lower, artist_lower);

    for token in query_lower.split_whitespace() {
        if target.contains(token) {
            score += 15;
        }
    }

    let dur_secs = track.duration_ms / 1000;
    if (60..=720).contains(&dur_secs) {
        score += 20;
    } else if dur_secs < 15 || dur_secs > 7200 {
        score -= 20;
    }

    const PENALTIES: &[&str] = &["karaoke", "cover", "reaction", "8d"];
    for &pen in PENALTIES {
        if target.contains(pen) && !query_lower.contains(pen) {
            score -= 40;
        }
    }

    score
}

fn track_item_to_track(item: rustypipe::model::TrackItem) -> YouTubeTrack {
    YouTubeTrack {
        id: item.id,
        name: item.name,
        artists: item.artists.into_iter().map(|a| a.name).collect(),
        album: item.album.map(|a| a.name).unwrap_or_default(),
        duration_ms: item.duration.unwrap_or(0).saturating_mul(1000),
    }
}

fn video_item_to_track(item: rustypipe::model::VideoItem) -> YouTubeTrack {
    let artist = item.channel.map(|c| c.name).unwrap_or_else(|| "YouTube".to_string());
    YouTubeTrack {
        id: item.id,
        name: item.name,
        artists: vec![artist],
        album: "YouTube Video".to_string(),
        duration_ms: item.duration.unwrap_or(0).saturating_mul(1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_metadata_implements_metadata_provider() {
        fn assert_provider<T: crate::domain::MetadataProvider>() {}
        assert_provider::<YouTubeMetadata>();
    }

    #[allow(dead_code)]
    async fn test_search(client: &RustyPipe, query: &str, limit: u8) -> Result<Vec<YouTubeTrack>, BotError> {
        let res = client.query().search::<rustypipe::model::VideoItem, _>(query).await
            .map_err(|e| BotError::Playback(format!("YouTube search failed: {e}")))?;
        let tracks: Vec<YouTubeTrack> = res.items.items
            .into_iter()
            .take(limit as usize)
            .map(video_item_to_track)
            .collect();
        Ok(tracks)
    }

    #[test]
    fn test_sanitize_query_strips_emojis_and_symbols() {
        let input = "QUERIAN ENFRIAR A KEIKO EN EL DESFILE ☠️ | PARADA MILITAR";
        let cleaned = sanitize_query(input);
        assert_eq!(cleaned, "QUERIAN ENFRIAR A KEIKO EN EL DESFILE PARADA MILITAR");
    }

    #[test]
    fn test_has_non_catalogue_keywords() {
        assert!(has_non_catalogue_keywords("tiny desk concert"));
        assert!(has_non_catalogue_keywords("circo sarita"));
        assert!(!has_non_catalogue_keywords("bad bunny la cancion"));
    }

    #[test]
    fn test_score_track_duration_and_penalties() {
        let track = YouTubeTrack {
            id: "test".to_string(),
            name: "Bad Bunny - La Cancion (Karaoke Version)".to_string(),
            artists: vec!["Cover Artist".to_string()],
            album: "Single".to_string(),
            duration_ms: 180_000,
        };
        let score = score_track("bad bunny", &track);
        // Has token matches but gets penalized for "karaoke" and "cover"
        assert!(score < 0);
    }
}
