use std::collections::{BTreeSet, HashMap};

/// The language code of the embedded fallback catalog.
pub const ENGLISH: &str = "en";

/// Special `.lang` entry holding the language's own display name.
pub(crate) const LANGUAGE_NAME_KEY: &str = "language_name";

/// Defines `Key`, `Key::id()`, and `Key::ALL` from a single list so the enum,
/// the `.lang` file ids, and the completeness check can never drift apart.
macro_rules! keys {
    ($($variant:ident => $id:literal),* $(,)?) => {
        /// A translatable message. One variant per string the bot actually
        /// sends; the id is the key used in `.lang` files. Referencing a
        /// variant (not a raw string) makes a typo a compile error.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Key {
            $($variant),*
        }

        impl Key {
            /// The stable snake_case id used in `.lang` files.
            pub fn id(self) -> &'static str {
                match self {
                    $(Key::$variant => $id),*
                }
            }

            /// Every key, for completeness and validation checks.
            pub const ALL: &'static [Key] = &[$(Key::$variant),*];
        }
    };
}

keys! {
    // Language
    LangSet => "lang_set",
    // Playback
    Searching => "searching",
    LoadingTrack => "loading_track",
    Resuming => "resuming",
    Paused => "paused",
    NothingToPlay => "nothing_to_play",
    RestartingTrack => "restarting_track",
    LoadingLiked => "loading_liked",
    NothingPlaying => "nothing_playing",
    CurrentTrack => "current_track",
    SearchCancelled => "search_cancelled",
    // Volume and seek
    VolumeSet => "volume_set",
    VolumeShow => "volume_show",
    VolumeRange => "volume_range",
    CurrentVolume => "current_volume",
    SeekForward => "seek_forward",
    SeekBackward => "seek_backward",
    SeekUsage => "seek_usage",
    SeekExceedsTimeline => "seek_exceeds_timeline",
    // Queue
    QueueCleared => "queue_cleared",
    IndexStartsAtOne => "index_starts_at_one",
    NoTrackAtPosition => "no_track_at_position",
    Removed => "removed",
    QueueRmUsage => "queue_rm_usage",
    NoMoreItemsInQueue => "no_more_items_in_queue",
    // Modes
    ModeRepeatTrack => "mode_repeat_track",
    ModeRepeatQueue => "mode_repeat_queue",
    ModeShuffle => "mode_shuffle",
    ModeOff => "mode_off",
    ModeUsage => "mode_usage",
    // Search and pick
    SearchUsage => "search_usage",
    SearchResultsHeader => "search_results_header",
    SearchResultsFooter => "search_results_footer",
    PickUsage => "pick_usage",
    PickTooLow => "pick_too_low",
    // Radio
    RadioAlreadyOn => "radio_already_on",
    RadioEnabled => "radio_enabled",
    RadioAlreadyOff => "radio_already_off",
    RadioDisabled => "radio_disabled",
    RadioStatusOn => "radio_status_on",
    RadioStatusOff => "radio_status_off",
    // Service switching
    AlreadyOnService => "already_on_service",
    SwitchedService => "switched_service",
    // Bot management
    Nickname => "nickname",
    GenderSet => "gender_set",
    GenderUsage => "gender_usage",
    Info => "info",
    Stats => "stats",
    // Player events (command processor)
    SpotifyUnavailable => "spotify_unavailable",
    NoResults => "no_results",
    NowPlaying => "now_playing",
    NowPlayingQueued => "now_playing_queued",
    MoreLoading => "more_loading",
    QueuedMany => "queued_many",
    QueuedOne => "queued_one",
    AlreadyQueuedLoadingRest => "already_queued_loading_rest",
    AlreadyInQueue => "already_in_queue",
    SearchFailed => "search_failed",
    RadioFetching => "radio_fetching",
    RadioPlaying => "radio_playing",
    RadioNoRecs => "radio_no_recs",
    RadioFailed => "radio_failed",
    EndOfQueue => "end_of_queue",
    StartOfQueue => "start_of_queue",
    InvalidPick => "invalid_pick",
    ChannelNotFound => "channel_not_found",
    FailedToStart => "failed_to_start",
    RateLimitCooldown => "rate_limit_cooldown",
}

/// Parse `.lang` file text into a key -> template map.
///
/// Format: `key = value` per line; `#` comments and blank lines ignored;
/// everything after the first `=` is the value (so values may contain `=`);
/// key and value are trimmed; `\n` in a value becomes a newline. A malformed
/// line is skipped with a warning — it never invalidates the rest of the file.
pub fn parse_lang(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line.split_once('=') {
            Some((key, value)) => {
                let key = key.trim();
                if key.is_empty() {
                    tracing::warn!("Ignoring translation line {} with empty key", idx + 1);
                    continue;
                }
                map.insert(key.to_string(), value.trim().replace("\\n", "\n"));
            }
            None => {
                tracing::warn!("Ignoring malformed translation line {}: {line}", idx + 1);
            }
        }
    }
    map
}

/// Fill named `{slot}` placeholders in a template.
///
/// Single-pass by name: slots may appear in any order, an unknown slot is left
/// visible as-is, and substituted values are never re-scanned (a value that
/// happens to contain braces cannot trigger a second substitution).
pub fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let brace = &rest[start..];
        match brace.find('}') {
            Some(end) => {
                let name = &brace[1..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(&brace[..=end]),
                }
                rest = &brace[end + 1..];
            }
            None => {
                // Unmatched '{' — copy the remainder verbatim.
                out.push_str(brace);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Extract the set of `{slot}` names in a template (for validation).
pub(crate) fn slots_of(template: &str) -> BTreeSet<String> {
    let mut slots = BTreeSet::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let brace = &rest[start..];
        match brace.find('}') {
            Some(end) => {
                slots.insert(brace[1..end].to_string());
                rest = &brace[end + 1..];
            }
            None => break,
        }
    }
    slots
}
