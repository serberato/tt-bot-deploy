//! Help text generation and topic detail lookups.

use crate::services::Service;

/// Build help text for the currently active service.
pub fn help_text(active: Service, is_admin: bool) -> String {
    let mut out = String::from(
        "Playback:\n\
         \x20 p <query>      Search and play a track, playlist, or album\n\
         \x20 p               Toggle play/pause\n\
         \x20 s               Stop playback and clear queue\n\
         \x20 n               Next track\n\
         \x20 b               Previous track\n\
         \x20 replay          Restart current track\n\
         \x20 c               Show current track info\n\
         \n\
         Queue:\n\
         \x20 queue           Show the queue\n\
         \x20 queue clear     Clear upcoming tracks\n\
         \x20 queue rm <N>    Remove Nth upcoming track\n\
         \n\
         Modes:\n\
         \x20 mode [direct|queue] Set play mode for searches\n\
         \x20 mode [r|rq|s|off]   Set repeat/shuffle mode\n",
    );
    if active == Service::Spotify {
        out.push_str("  radio [on|off]      Toggle radio (auto-recommendations)\n");
        out.push_str("  liked               Play your Liked Songs (also: fav)\n");
    }
    append_general_help(&mut out, active, is_admin);
    out
}

fn append_general_help(out: &mut String, active: Service, is_admin: bool) {
    out.push_str(
        "\n\
         Audio:\n\
         \x20 v [0-100]       Get or set volume\n\
         \x20 sf/sb [N]       Seek forward/backward N seconds\n\
         \n\
         Search:\n\
         \x20 search <query>  Search and pick from results\n\
         \x20 <number>        Pick a search result\n\
         \x20 a / cancel      Cancel search\n\
         \n\
         Service:\n\
         \x20 /sp             Switch to Spotify\n\
         \x20 /yt             Switch to YouTube\n\
         \n\
         Bot:\n\
         \x20 link         Get URL for current track\n\
         \x20 stats        Show bot uptime and session stats\n\
         \x20 lang [code]  Set personal language\n\
         \x20 status <text> Set idle status text\n\
         \x20 cn <name>    Change nickname\n\
         \x20 gender <g>   Change gender\n\
         \x20 info         Bot info\n",
    );
    if is_admin {
        out.push_str(
            "\x20 jc <path>    Join channel\n\
             \x20 glang        Set the server default language\n\
             \x20 rs           Restart\n\
             \x20 q            Quit\n",
        );
    }
    out.push_str("\n\x20 Active service: ");
    out.push_str(active.name());
    out.push_str("\nType h <command> for detailed help (e.g. h queue)");
}

pub(crate) fn help_topic_detail(topic: &str, active: Service) -> Option<&'static str> {
    let detail = match topic {
        "p" | "play" => HELP_PLAY,
        "s" | "stop" => "s / stop\nStop playback and clear the queue.",
        "n" | "next" => {
            "n / next\nSkip to the next track in the queue.\nIf radio is on and queue is empty, fetches recommendations."
        }
        "b" | "prev" => "b / prev\nGo back to the previous track in the queue.",
        "replay" | "rp" => "replay / rp\nRestart the current track from the beginning.",
        "c" | "current" => {
            "c / current\nShow the currently playing track with position, duration, and active modes."
        }
        "queue" => HELP_QUEUE,
        "mode" => HELP_MODE,
        "v" | "volume" => HELP_VOLUME,
        "sf" | "sb" | "seek" => HELP_SEEK,
        "search" => HELP_SEARCH,
        "radio" if active == Service::Spotify => HELP_RADIO,
        "radio" => return None,
        "link" | "url" => {
            "link / url\nGet the URL for the currently playing track.\nOpen it in the service's app or share it with others."
        }
        "stats" => "stats\nShow bot uptime, tracks played this session, queue length, and volume.",
        "jc" => "jc <path>\nJoin a TeamTalk channel by path.\nExample: jc /Music Room",
        "lang" => {
            "lang [code]\nShow available languages, or set your own.\nYour choice is remembered by username.\nlang clear removes your choice (follow the server default).\nExample: lang de"
        }
        "glang" => {
            "glang <code>\nSet the server default language (admin).\nUsers who picked their own language with lang keep it."
        }
        "cn" => "cn <name>\nChange the bot's nickname.\nExample: cn DJ Bot",
        "gender" => {
            "gender <male|female|neutral>\nSet the bot's gender (affects TT avatar).\nAliases: m, f, n, man, woman, nb"
        }
        "sp" | "spotify" | "yt" | "youtube" => HELP_SERVICE,
        "rs" | "restart" => "rs / restart\nRestart the bot. Saves config before exit.",
        "q" | "quit" => "q / quit\nShut down the bot. Saves config before exit.",
        _ => return None,
    };
    Some(detail)
}

const HELP_SERVICE: &str = "\
/sp / /yt
  /sp     Switch active service to Spotify.
  /yt     Switch active service to YouTube.
Commands like p, search, n, b target the active service.
Switching does not interrupt playback. Use s to stop.";

const HELP_PLAY: &str = "\
p / play
  p <query>   Search Spotify and play the first result.
              If already playing, queues the track instead.
              Accepts track names, Spotify URLs, playlist URLs, album URLs.
  p           Toggle play/pause when no query given.
              If paused: resumes. If playing: pauses.
              If paused: resumes. If playing: pauses.
Examples:
  p photograph
  p spotify:track:6rqhFgbbKwnb9MLmUQDhG6
  p https://open.spotify.com/playlist/...";

const HELP_QUEUE: &str = "\
queue
  queue          Show all tracks in the queue with positions.
  queue clear    Remove all upcoming tracks (keeps current).
  queue rm <N>   Remove the Nth upcoming track.
                 N=1 is the next track after the current one.
Examples:
  queue rm 1     Remove the next upcoming track
  queue rm 3     Remove the 3rd upcoming track
  queue clear    Clear everything after current track";

const HELP_MODE: &str = "\
mode [direct|queue|r|rq|s|off]
  mode direct  Searches interrupt the current track
  mode queue   Searches are added to the queue
  mode r       Repeat current track
  mode rq      Repeat entire queue
  mode s       Shuffle
  mode off     Turn off repeat and shuffle";

const HELP_VOLUME: &str = "\
v / volume [0-100]
  v          Show current volume
  v 50       Set volume to 50%
  v50        Set volume to 50% (no space)
  volume 30  Set volume to 30%
Volume is capped by the configured max volume.";

const HELP_SEEK: &str = "\
sf / sb [seconds]
  sf         Seek forward 10 seconds (default)
  sb         Seek backward 10 seconds (default)
  sf30       Seek forward 30 seconds
  sb 5       Seek backward 5 seconds";

const HELP_SEARCH: &str = "\
search <query>
  Search Spotify and show results. Then:
  <number>   Pick a result to play/queue
  a / cancel Dismiss search results
Example:
  search photograph
  2          Play the 2nd result";

const HELP_RADIO: &str = "\
radio [on|off]
  radio on   Enable radio mode. When a single track finishes
             and the queue is empty, automatically fetches
             Spotify recommendations based on the last track.
             Does not trigger for playlists or albums.
  radio off  Disable radio mode.
  radio      Show current radio status.";
