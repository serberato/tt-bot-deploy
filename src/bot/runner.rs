//! Reusable bot runner.
//!
//! Contains the full bot lifecycle: TeamTalk setup, Spotify auth,
//! audio pipeline, command processor, and event loop.
//! Used by both the standalone binary and the Windows tray manager.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use librespot_playback::player::PlayerEvent;

use crate::bot::commands::BotCommand;
use crate::bot::state::{PlaybackStatus, PlayerState, SharedState};
use crate::config::BotConfig;
use crate::error::BotError;
use crate::spotify::metadata::SpotifyMetadata;
use crate::spotify::player::SpotifyPlayer;

use crate::bot::announcer::Announcer;
use crate::bot::handlers::{handle_command, HandlerContext};
pub(crate) use crate::bot::controller::{
    channel_move_needs_flush, spawn_drained_advance, startup_auth_plan,
    Controller, StartFailureBrake, StartupAuthPlan,
};
#[cfg(test)]
pub(crate) use crate::bot::controller::{auto_advance_is_stale, DrainWait};
#[cfg(test)]
pub(crate) use crate::bot::handlers::queue::queue_wait_info;
/// How the bot exited.
#[derive(Debug, Clone, PartialEq)]
pub enum BotExit {
    /// Clean quit (user sent quit command).
    Quit,
    /// Restart requested (user sent restart command).
    Restart,
    /// External shutdown signal (tray stop button, systemd stop).
    Shutdown,
}

/// Status events sent to the tray (or any observer).
#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum RunnerEvent {
    Connecting,
    Authenticating,
    Connected,
    Playing(String),
    Idle,
    Disconnected,
    Error(String),
}


/// Run a single bot instance. Returns when the bot exits.
///
/// - `config`: Bot configuration.
/// - `config_path`: Path to config file (for saving runtime changes).
/// - `shutdown`: External shutdown signal. Set to true to stop the bot.
/// - `event_tx`: Optional channel for status updates (used by tray).
/// - `last_channel`: In-memory carry of the current channel across a restart.
///   Applied only to the TT-connection config copy (never to `config` itself,
///   so ConfigStore/the config file keep the configured default). On a `rs`
///   restart it holds the channel the bot was in, so it rejoins there; `None`
///   (fresh process start) joins the configured default.
pub async fn run_bot(
    config: BotConfig,
    config_path: String,
    shutdown: Arc<AtomicBool>,
    event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    last_channel: Arc<parking_lot::Mutex<Option<String>>>,
) -> Result<BotExit, BotError> {
    let send_event = {
        let tx = event_tx.clone();
        move |evt: RunnerEvent| {
            if let Some(ref tx) = tx {
                let _ = tx.send(evt);
            }
        }
    };

    tracing::info!("TeamTalk Spotify Bot starting...");
    tracing::info!("Config loaded from {}", config_path);
    log_startup_versions();

    let mut initial_state = PlayerState::new();
    initial_state.radio_enabled = config.radio_enabled;
    initial_state.repeat_track = config.repeat_track;
    initial_state.repeat_queue = config.repeat_queue;
    initial_state.shuffle = config.shuffle;
    initial_state.active_service = config.default_service;
    let state: SharedState = Arc::new(parking_lot::Mutex::new(initial_state));
    let volume = Arc::new(AtomicU8::new(config.volume.min(config.max_volume)));

    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<i16>>(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<BotCommand>();

    send_event(RunnerEvent::Connecting);
    // Only the TT connection copy gets the restart channel override; the real
    // `config` (and thus ConfigStore) keeps the configured default channel, so
    // the config file's channel_name is never rewritten.
    let tt_config = {
        let mut c = config.clone();
        if let Some(ch) = last_channel.lock().clone() {
            if ch != c.channel_name {
                tracing::info!("Restart: rejoining last channel {ch} (default is {})", c.channel_name);
                c.channel_name = ch;
            }
        }
        c
    };
    let client = tokio::task::spawn_blocking(move || {
        crate::tt::connection::setup_teamtalk(&tt_config)
    }).await.map_err(|e| BotError::TeamTalk(format!("TT setup task failed: {e}")))??;
    let client = Arc::new(client);

    send_event(RunnerEvent::Connected);

    // Spawn audio pipeline thread
    let pipeline_client = client.clone();
    let pipeline_volume = volume.clone();
    let pipeline_config = config.clone();
    let audio_reset = Arc::new(AtomicBool::new(false));
    let timing_reset = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    // Set on a self channel-move: the pipeline ends and restarts the injected
    // stream (like a manual pause/play) without touching position counters.
    let stream_flush = Arc::new(AtomicBool::new(false));
    // True while the pipeline has nothing left to play; end-of-track advances
    // wait on this so the buffered tail of a song reaches listeners first.
    let pipeline_drained = Arc::new(AtomicBool::new(true));
    // Realtime playback position (ms injected since last reset), written by the
    // pipeline and read by the YouTube player for accurate `c`/seek positions.
    let pipeline_pos_ms = Arc::new(AtomicU32::new(0));
    let pipeline_reset = audio_reset.clone();
    let pipeline_timing_reset = timing_reset.clone();
    let pipeline_pause = pause_flag.clone();
    let pipeline_stream_flush = stream_flush.clone();
    let pipeline_drained_flag = pipeline_drained.clone();
    // Internal teardown signal set on EVERY run_bot exit (including the
    // reconnect-exhausted Err path, which must not touch the shared `shutdown`
    // — that would stop the supervisor from retrying). Keeps the pipeline
    // thread from leaking across tray restart-retries.
    let local_shutdown = Arc::new(AtomicBool::new(false));
    let pipeline_shutdown = local_shutdown.clone();
    let pipeline_pos = pipeline_pos_ms.clone();
    std::thread::spawn(move || {
        let mut pipeline = crate::audio::pipeline::AudioPipeline::new(
            audio_rx,
            pipeline_client,
            pipeline_volume,
            pipeline_reset,
            pipeline_timing_reset,
            pipeline_pause,
            pipeline_stream_flush,
            pipeline_drained_flag,
            pipeline_shutdown,
            pipeline_pos,
            &pipeline_config,
        );
        pipeline.run();
    });

    let profile_name = std::path::Path::new(&config_path).file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let auth = crate::spotify::auth::SpotifyAuth::new(&profile_name);
    let session = auth.new_session();

    // Connect Spotify eagerly only if credentials are already cached or Spotify
    // is the default service. A YouTube-only user with no cached credentials is
    // never sent to the browser at startup; the connection happens lazily on
    // their first Spotify command instead (see `ensure_spotify!`). When OAuth
    // is infeasible (systemd: no browser, no stdin) a failure must NOT abort —
    // we've already logged into TeamTalk, so exiting turns Restart=on-failure
    // into a nonstop login/logout loop on the server.
    let spotify_connected = match startup_auth_plan(
        auth.has_cached_credentials(),
        config.default_service == crate::services::Service::Spotify,
        auth.oauth_feasible(),
    ) {
        StartupAuthPlan::ConnectFatal => {
            send_event(RunnerEvent::Authenticating);
            auth.connect_existing(&session).await?;
            true
        }
        StartupAuthPlan::ConnectBestEffort => {
            send_event(RunnerEvent::Authenticating);
            match auth.connect_existing(&session).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!(
                        "Spotify is unavailable and interactive login is impossible here: {e}. \
                         Continuing without Spotify; run `tt-spotify-bot --auth`, then restart."
                    );
                    false
                }
            }
        }
        StartupAuthPlan::Skip => {
            tracing::info!("Skipping Spotify auth at startup; no cached credentials and default service is YouTube");
            false
        }
    };

    // Wrap the (possibly-connected) session in a shared holder so the recovery
    // routine can swap in a freshly-rebuilt session after a session death. The
    // player is rebuilt on recovery; the metadata client reads the holder live.
    let session_holder = Arc::new(parking_lot::Mutex::new(session));
    let (player, event_rx) = {
        let s = session_holder.lock().clone();
        SpotifyPlayer::new(s, &config, audio_tx.clone())
    };
    let metadata = SpotifyMetadata::new(session_holder.clone());
    // Shared with the recovery supervisor for rebuilding a dead session.
    let auth = Arc::new(auth);
    let youtube_metadata = Arc::new(crate::youtube::metadata::YouTubeMetadata::new(&config, &profile_name)?);
    let youtube_player = crate::youtube::player::YouTubePlayer::new(
        audio_tx.clone(),
        youtube_metadata.clone(),
        cmd_tx.clone(),
        state.clone(),
        pipeline_pos_ms.clone(),
    );

    // Session-recovery coordination (see `spotify_supervisor`). `recovery_notify`
    // wakes the supervisor immediately; `recovery_suspended` latches after a
    // give-up so it stops auto-retrying until a Spotify command clears it.
    let recovery_notify = Arc::new(tokio::sync::Notify::new());
    let recovery_suspended = Arc::new(AtomicBool::new(false));
    let recovery_guard = Arc::new(crate::spotify::recovery::RecoveryGuard::new());

    // Exit signal: command_processor sets this instead of process::exit
    let exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>> =
        Arc::new(parking_lot::Mutex::new(None));

    // Single writer for all runtime config persistence.
    let config_store = Arc::new(crate::config::ConfigStore::new(
        config_path.clone(),
        config.clone(),
    ));

    // Shared i18n runtime: embedded English + any <config_dir>/lang/*.lang
    // files, and per-user language prefs. Shared by the dispatcher (which
    // seeds the per-user language at dispatch) and the command processor.
    let i18n = std::sync::Arc::new(crate::i18n::I18n::load(
        &crate::config::config_dir(),
        &config.default_language,
    ));

    // Session-recovery supervisor: rebuilds a dead Spotify session and resumes
    // playback with no user action. Uses cheap clones of the shared handles.
    let recovery = SpotifyRecovery {
        session_holder: session_holder.clone(),
        auth: auth.clone(),
        config: config.clone(),
        audio_tx: audio_tx.clone(),
        player: player.clone(),
        state: state.clone(),
        cmd_tx: cmd_tx.clone(),
        pause_flag: pause_flag.clone(),
        audio_reset: audio_reset.clone(),
        guard: recovery_guard.clone(),
        recovery_notify: recovery_notify.clone(),
        local_shutdown: local_shutdown.clone(),
        event_tx: event_tx.clone(),
        pipeline_drained: pipeline_drained.clone(),
    };
    tokio::spawn(spotify_supervisor(recovery, recovery_suspended.clone()));

    // Spawn command processor
    let bot_gender = crate::config::parse_gender(&config.bot_gender);
    let cmd_ctx = CmdContext {
        player,
        metadata,
        youtube_metadata,
        youtube_player,
        session: session_holder.clone(),
        auth,
        spotify_connected,
        recovery_notify: recovery_notify.clone(),
        recovery_suspended: recovery_suspended.clone(),
        state: state.clone(),
        client: client.clone(),
        search_limit: config.search_limit,
        radio_batch_size: config.radio_batch_size,
        radio_delay: config.radio_delay,
        radio_cmd_tx: cmd_tx.clone(),
        bot_gender,
        config_store: config_store.clone(),
        audio_reset: audio_reset.clone(),
        timing_reset: timing_reset.clone(),
        pause_flag: pause_flag.clone(),
        pipeline_drained: pipeline_drained.clone(),
        volume_for_save: volume.clone(),
        exit_reason: exit_reason.clone(),
        shutdown: shutdown.clone(),
        event_tx: event_tx.clone(),
        i18n: i18n.clone(),
    };
    let processor_handle = tokio::spawn(async move {
        command_processor(cmd_rx, cmd_ctx).await;
    });

    // Spawn player event loop
    let event_state = state.clone();
    let event_cmd_tx = cmd_tx.clone();
    let event_session = session_holder.clone();
    let event_notify = recovery_notify.clone();
    let event_drained = pipeline_drained.clone();
    let event_pause = pause_flag.clone();
    let event_loop_handle = tokio::spawn(async move {
        player_event_loop(event_rx, event_state, event_cmd_tx, event_session, event_notify, event_drained, event_pause).await;
    });

    let dispatcher = crate::bot::commands::CommandDispatcher {
        state: state.clone(),
        volume: volume.clone(),
        cmd_tx,
        max_volume: config.max_volume,
        start_time: std::time::Instant::now(),
        auth: crate::bot::auth::AdminAuth::from_config(&config),
        i18n: i18n.clone(),
    };

    tracing::info!("Bot is ready! Listening for commands...");

    // One-shot, non-blocking update check. Logs a breadcrumb if a newer release
    // exists; never blocks startup and never self-updates a running service.
    if crate::settings::load().check_updates_on_startup {
        tokio::spawn(async {
            if let Ok(Some(info)) = crate::update::check().await {
                tracing::info!("Update {} available - run: ttspotify --update", info.tag);
            }
        });
    }

    {
        let mut status = ::teamtalk::types::UserStatus::default();
        status.gender = bot_gender;
        let _ = client.set_status(status, &config_store.get_idle_status());
    }
    send_event(RunnerEvent::Idle);

    // Track current channel for manual rejoin after reconnects.
    // SDK auto-join is disabled so admin moves are respected.
    let last_channel_id = Arc::new(parking_lot::Mutex::new(client.my_channel_id()));
    let last_channel_pw = Arc::new(parking_lot::Mutex::new(config.channel_password.clone()));

    // Event loop runs on a blocking thread.
    // Connection + login reconnect is handled by the SDK; channel rejoin is manual.
    let event_client = client.clone();
    let event_shutdown = shutdown.clone();
    let event_exit = exit_reason.clone();
    let event_event_tx = event_tx.clone();
    let event_last_channel = last_channel.clone();
    let event_stream_flush = stream_flush.clone();
    // If the SDK's auto-reconnect can't restore the session within this window,
    // stop spinning and return an error so the supervisor (tray restart /
    // systemd Restart=) can recover with a fresh client instead of the bot
    // becoming a silent zombie polling a dead connection forever.
    const RECONNECT_DEADLINE: Duration = Duration::from_secs(360);
    let reconnect_exhausted = tokio::task::spawn_blocking(move || -> bool {
        // `Some(instant)` while disconnected, cleared on successful re-login.
        let mut disconnected_since: Option<Instant> = None;
        loop {
            if event_shutdown.load(Ordering::Relaxed) {
                break false;
            }
            if event_exit.lock().is_some() {
                break false;
            }
            // Give up if we've been disconnected past the deadline.
            if let Some(since) = disconnected_since {
                if since.elapsed() > RECONNECT_DEADLINE {
                    tracing::error!(
                        "Auto-reconnect exhausted after {}s, giving up so the supervisor can restart",
                        RECONNECT_DEADLINE.as_secs()
                    );
                    break true;
                }
            }

            if let Some((event, message)) = event_client.poll(100) {
                match event {
                    ::teamtalk::Event::ConnectionLost => {
                        tracing::warn!("Connection lost, SDK auto-reconnect will handle recovery");
                        if disconnected_since.is_none() {
                            disconnected_since = Some(Instant::now());
                        }
                        if let Some(ref tx) = event_event_tx {
                            let _ = tx.send(RunnerEvent::Disconnected);
                        }
                    }
                    ::teamtalk::Event::ConnectSuccess => {
                        tracing::info!("Reconnected to server");
                    }
                    ::teamtalk::Event::MySelfLoggedIn => {
                        tracing::info!("Re-logged in after reconnect");
                        // Session restored: reset the disconnect watchdog.
                        disconnected_since = None;
                        // Rejoin our last channel whenever the reconnect didn't
                        // land us back in it (root, a different channel, or 0).
                        // Admin moves during a live session are still respected
                        // because UserJoined keeps last_channel_id current.
                        let ch = event_client.my_channel_id();
                        let rejoin_ch = *last_channel_id.lock();
                        if rejoin_ch != ::teamtalk::types::ChannelId(0) && ch != rejoin_ch {
                            let pw = last_channel_pw.lock().clone();
                            match event_client.join_channel_and_wait(rejoin_ch, &pw, 5_000) {
                                Ok(_) => tracing::info!("Rejoined channel {} after reconnect", rejoin_ch.0),
                                Err(e) => tracing::warn!("Failed to rejoin channel after reconnect: {e}"),
                            }
                        }
                        if let Some(ref tx) = event_event_tx {
                            let _ = tx.send(RunnerEvent::Connected);
                        }
                    }
                    ::teamtalk::Event::UserJoined => {
                        if let Some(user) = message.user() {
                            if user.id == event_client.my_id() && user.channel_id != ::teamtalk::types::ChannelId(0) {
                                let prev = {
                                    let mut ch = last_channel_id.lock();
                                    let prev = *ch;
                                    *ch = user.channel_id;
                                    prev
                                };
                                tracing::info!("Now in channel {}", user.channel_id.0);
                                if channel_move_needs_flush(prev, user.channel_id) {
                                    // The SDK restarted the voice stream for the
                                    // new channel; restart injection cleanly or
                                    // the audio comes out garbled until a manual
                                    // pause/play.
                                    event_stream_flush.store(true, Ordering::Relaxed);
                                }
                                // Remember the current channel (in memory only) so a
                                // restart rejoins here instead of the configured
                                // default. The config file is never modified.
                                if let Some(path) = event_client.get_channel_path(user.channel_id) {
                                    *event_last_channel.lock() = Some(path);
                                }
                            }
                        }
                    }
                    ::teamtalk::Event::TextMessage => {
                        if let Some(text_msg) = message.text() {
                            if (text_msg.msg_type as i32) != 1 {
                                continue;
                            }
                            let sender_id = text_msg.from_id.0;
                            let my_id = event_client.my_id().0;
                            if sender_id != my_id && !text_msg.text.is_empty()
                                && !dispatcher.dispatch(
                                    &event_client,
                                    &text_msg.text,
                                    sender_id,
                                    &text_msg.from_username,
                                ) {
                                    break false;
                                }
                        }
                    }
                    _ => {}
                }
            }
        }
    }).await.map_err(|e| BotError::TeamTalk(format!("Event loop failed: {e}")))?;

    // Tear down the pipeline thread on every exit path (the shared `shutdown`
    // may be untouched — e.g. reconnect-exhausted, where the supervisor still
    // needs it clear to retry).
    local_shutdown.store(true, Ordering::Relaxed);

    if reconnect_exhausted {
        processor_handle.abort();
        event_loop_handle.abort();
        let _ = client.disconnect();
        return Err(BotError::TeamTalk(
            "Lost connection to the TeamTalk server and auto-reconnect was exhausted".into(),
        ));
    }

    // Give the command processor a moment to finish do_exit() if it's
    // still running (event loop may break before the async command handler
    // has set exit_reason).
    for _ in 0..20 {
        if exit_reason.lock().is_some()
            || shutdown.load(Ordering::Relaxed)
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Determine exit reason: check explicit exit_reason first (quit/restart
    // command), then fall back to external shutdown signal (tray/systemd).
    // do_exit() sets both exit_reason AND shutdown=true, so we must check
    // exit_reason first to avoid masking quit/restart as Shutdown.
    let exit = exit_reason.lock().take();
    let reason = match exit {
        Some(reason) => reason,
        None if shutdown.load(Ordering::Relaxed) => BotExit::Shutdown,
        None => BotExit::Quit,
    };
    // do_exit() has run by now (we waited for exit_reason), so config is saved;
    // abort the spawned tasks so they don't linger across a restart.
    processor_handle.abort();
    event_loop_handle.abort();
    let _ = client.disconnect();
    Ok(reason)
}

/// Log the app version plus the versions of the tools we depend on (TeamTalk
/// SDK, yt-dlp, bgutil-pot). Written to each instance's log at startup so a bug
/// report's log self-identifies exactly what was running.
fn log_startup_versions() {
    let app = env!("CARGO_PKG_VERSION");
    let sdk = std::fs::read_to_string("TEAMTALK_DLL/TEAMTALK_SDK_VERSION.txt")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let tools = crate::youtube::setup::installed_tool_versions();
    let yt = tools.yt_dlp.as_deref().unwrap_or("not installed");
    let bg = tools.bgutil.as_deref().unwrap_or("not installed");
    tracing::info!(
        "Versions — app: v{app}, TeamTalk SDK: {sdk}, yt-dlp: {yt}, bgutil-pot: {bg}"
    );
}


/// All shared context needed by the command processor, bundled to avoid parameter explosion.
struct CmdContext {
    player: SpotifyPlayer,
    metadata: SpotifyMetadata,
    youtube_metadata: Arc<crate::youtube::metadata::YouTubeMetadata>,
    youtube_player: crate::youtube::player::YouTubePlayer,
    session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    auth: Arc<crate::spotify::auth::SpotifyAuth>,
    spotify_connected: bool,
    /// Wakes the recovery supervisor when a command detects a dead session.
    recovery_notify: Arc<tokio::sync::Notify>,
    /// Cleared by a Spotify command to un-latch auto-recovery after a give-up.
    recovery_suspended: Arc<AtomicBool>,
    state: SharedState,
    client: Arc<::teamtalk::Client>,
    search_limit: u8,
    radio_batch_size: u8,
    radio_delay: f32,
    radio_cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    bot_gender: ::teamtalk::types::UserGender,
    config_store: Arc<crate::config::ConfigStore>,
    audio_reset: Arc<AtomicBool>,
    timing_reset: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    /// True while the audio pipeline has nothing buffered; natural track ends
    /// wait on this before advancing so the song's tail plays out.
    pipeline_drained: Arc<AtomicBool>,
    volume_for_save: Arc<AtomicU8>,
    exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>>,
    shutdown: Arc<AtomicBool>,
    event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    i18n: Arc<crate::i18n::I18n>,
}

/// Everything the session-recovery supervisor needs to rebuild a dead Spotify
/// session and resume playback. All fields are cheap handles/clones.
struct SpotifyRecovery {
    session_holder: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    auth: Arc<crate::spotify::auth::SpotifyAuth>,
    config: BotConfig,
    audio_tx: crossbeam_channel::Sender<Vec<i16>>,
    player: SpotifyPlayer,
    state: SharedState,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    pause_flag: Arc<AtomicBool>,
    audio_reset: Arc<AtomicBool>,
    guard: Arc<crate::spotify::recovery::RecoveryGuard>,
    recovery_notify: Arc<tokio::sync::Notify>,
    local_shutdown: Arc<AtomicBool>,
    event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    pipeline_drained: Arc<AtomicBool>,
}

/// Build a brand-new Spotify session (cached credentials only — never opens a
/// browser) and rebuild the player from it, swapping both into the shared
/// holders. Returns the new player event channel for the caller to restart the
/// event loop on. librespot Sessions are single-use, so this is the only way to
/// recover a session whose connection has died.
async fn rebuild_spotify_engine(
    rec: &SpotifyRecovery,
) -> Result<librespot_playback::player::PlayerEventChannel, BotError> {
    if !rec.auth.has_cached_credentials() {
        return Err(BotError::Playback(
            "no cached Spotify credentials to rebuild the session".into(),
        ));
    }
    let session = rec.auth.new_session();
    rec.auth.connect_existing(&session).await?;
    // Publish the new session to metadata (shared holder) and rebuild the player.
    *rec.session_holder.lock() = session.clone();
    let event_rx = rec
        .player
        .rebuild(session, &rec.config, rec.audio_tx.clone());
    Ok(event_rx)
}

/// Recover a dead Spotify session: pause, rebuild with bounded backoff, restart
/// the player event loop, and resume the interrupted track where it left off.
/// Single-flight via `rec.guard`.
async fn recover_spotify(rec: &SpotifyRecovery) -> crate::spotify::recovery::RecoveryOutcome {
    use crate::spotify::recovery::{delay_before_attempt, resume_seek_ms, RecoveryOutcome, MAX_ATTEMPTS};

    if !rec.guard.try_begin() {
        // Another recovery cycle is already running.
        return RecoveryOutcome::Recovered;
    }
    tracing::warn!("Spotify session died; starting bounded recovery");

    // Capture the resume point: only a currently-playing Spotify track. When a
    // Spotify track was playing we pause the pipeline so its decrypt-garbage
    // stops; if YouTube is playing (or nothing), leave the pipeline alone so an
    // idle Spotify session death never interrupts YouTube audio.
    let resume = {
        let s = rec.state.lock();
        let was_paused = s.status == PlaybackStatus::Paused;
        s.current().and_then(|e| {
            if e.track.service() == crate::services::Service::Spotify {
                Some((e.track.uri().to_string(), s.position_ms, was_paused))
            } else {
                None
            }
        })
    };
    let pause_pipeline = resume.is_some();
    // If the user had the track paused when the session died, resume it paused
    // rather than suddenly playing.
    let resume_paused = resume.as_ref().map(|(_, _, p)| *p).unwrap_or(false);
    if pause_pipeline {
        rec.pause_flag.store(true, Ordering::Relaxed);
    }

    let mut attempt = 0usize;
    let outcome = loop {
        let Some(delay) = delay_before_attempt(attempt) else {
            break RecoveryOutcome::GaveUp;
        };
        tokio::time::sleep(delay).await;
        if rec.local_shutdown.load(Ordering::Relaxed) {
            break RecoveryOutcome::GaveUp;
        }
        match rebuild_spotify_engine(rec).await {
            Ok(event_rx) => {
                tracing::info!("Spotify session rebuilt on attempt {}", attempt + 1);
                // Restart the player event loop on the new channel; the old loop
                // ends when the old player (and its channel) drops.
                let st = rec.state.clone();
                let tx = rec.cmd_tx.clone();
                let sh = rec.session_holder.clone();
                let notify = rec.recovery_notify.clone();
                let drained = rec.pipeline_drained.clone();
                let paused = rec.pause_flag.clone();
                tokio::spawn(async move {
                    player_event_loop(event_rx, st, tx, sh, notify, drained, paused).await;
                });
                // Resume the interrupted track slightly before where it died.
                if let Some((uri, pos_ms, _)) = &resume {
                    if let Ok(parsed) = librespot_core::spotify_uri::SpotifyUri::from_uri(uri) {
                        rec.audio_reset.store(true, Ordering::Relaxed);
                        let seek = resume_seek_ms(*pos_ms);
                        rec.player.load_track_at(&parsed, seek);
                        if resume_paused {
                            // Keep it paused: pause the freshly-loaded track and
                            // leave the pipeline paused (don't unpause below).
                            rec.player.pause();
                        }
                        tracing::info!(
                            "Resumed {uri} at {seek}ms after recovery (paused={resume_paused})"
                        );
                    }
                }
                // Unpause the pipeline only when actually resuming playback.
                if pause_pipeline && !resume_paused {
                    rec.pause_flag.store(false, Ordering::Relaxed);
                }
                if let Some(tx) = &rec.event_tx {
                    let _ = tx.send(RunnerEvent::Connected);
                }
                break RecoveryOutcome::Recovered;
            }
            Err(e) => {
                tracing::error!("Spotify rebuild attempt {} failed: {e}", attempt + 1);
                attempt += 1;
            }
        }
    };

    if outcome == RecoveryOutcome::GaveUp {
        tracing::error!(
            "Spotify recovery gave up after {MAX_ATTEMPTS} attempts; playback stopped. \
             A Spotify command will retry."
        );
        if pause_pipeline {
            rec.pause_flag.store(false, Ordering::Relaxed);
        }
        if let Some(tx) = &rec.event_tx {
            let _ = tx.send(RunnerEvent::Error(
                "Spotify unreachable; playback stopped".to_string(),
            ));
        }
    }
    rec.guard.finish();
    outcome
}

/// Supervisor task: watch for a dead session and drive recovery. Polls the local
/// `session.is_invalid()` signal (free — no network) on a 1s tick, or wakes
/// immediately when notified by the event loop / a command. After a give-up it
/// stays suspended until a Spotify command clears the latch and re-notifies.
async fn spotify_supervisor(rec: SpotifyRecovery, recovery_suspended: Arc<AtomicBool>) {
    loop {
        tokio::select! {
            _ = rec.recovery_notify.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }
        if rec.local_shutdown.load(Ordering::Relaxed) {
            break;
        }
        let dead = rec.session_holder.lock().is_invalid();
        if dead
            && !recovery_suspended.load(Ordering::Relaxed)
            && recover_spotify(&rec).await == crate::spotify::recovery::RecoveryOutcome::GaveUp
        {
            tracing::error!("Spotify recovery gave up. Exiting to allow systemd to restart the bot.");
            std::process::exit(1);
        }
    }
}

async fn command_processor(
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<BotCommand>,
    ctx: CmdContext,
) {
    let CmdContext {
        player,
        metadata,
        youtube_metadata,
        youtube_player,
        session,
        auth,
        spotify_connected,
        recovery_notify,
        recovery_suspended,
        state,
        client,
        search_limit,
        radio_batch_size,
        radio_delay,
        radio_cmd_tx,
        bot_gender,
        config_store,
        audio_reset,
        timing_reset,
        pause_flag,
        pipeline_drained,
        volume_for_save,
        exit_reason,
        shutdown,
        event_tx,
        i18n,
    } = ctx;

    let controller = Controller::new(
        player,
        youtube_player,
        client.clone(),
        state.clone(),
        audio_reset,
        pause_flag,
        timing_reset,
        pipeline_drained,
        config_store.clone(),
    );

    let announcer = Announcer::new(client, i18n, bot_gender, event_tx);
    let start_brake = StartFailureBrake::new(3);

    let mut handler_ctx = HandlerContext {
        controller,
        announcer,
        metadata,
        youtube_metadata,
        session,
        auth,
        spotify_connected,
        recovery_notify,
        recovery_suspended,
        search_limit,
        radio_batch_size,
        radio_delay,
        radio_cmd_tx,
        config_store,
        volume_for_save,
        exit_reason,
        shutdown,
        start_brake,
        radio_prefetch_slot: Arc::new(parking_lot::Mutex::new(None)),
        pending_volume_save: Arc::new(AtomicBool::new(false)),
    };

    while let Some(cmd) = cmd_rx.recv().await {
        handle_command(cmd, &mut handler_ctx).await;
    }
}

async fn player_event_loop(
    mut events: librespot_playback::player::PlayerEventChannel,
    state: SharedState,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    session: Arc<parking_lot::Mutex<librespot_core::session::Session>>,
    recovery_notify: Arc<tokio::sync::Notify>,
    pipeline_drained: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
) {
    while let Some(event) = events.recv().await {
        match event {
            PlayerEvent::Playing { position_ms, .. } => {
                let mut s = state.lock();
                s.status = PlaybackStatus::Playing;
                s.position_ms = position_ms;
            }
            PlayerEvent::Paused { position_ms, .. } => {
                let mut s = state.lock();
                s.status = PlaybackStatus::Paused;
                s.position_ms = position_ms;
            }
            PlayerEvent::EndOfTrack { track_id, .. } => {
                // A dead session surfaces a decrypt failure as a normal
                // EndOfTrack (librespot plays the still-encrypted bytes, the
                // decoder chokes, and it "ends" the track). Advancing here would
                // skip-storm through the whole queue in seconds. If the session
                // is invalid, this is a fake end: don't advance; wake the
                // recovery supervisor to rebuild the session instead.
                if session.lock().is_invalid() {
                    tracing::warn!("EndOfTrack with dead Spotify session; triggering recovery instead of advancing");
                    recovery_notify.notify_one();
                    continue;
                }
                // Guard against a stale EndOfTrack for a track we've already
                // moved past (e.g. the user skipped just as it ended), which
                // would otherwise double-advance the queue. Only advance if the
                // ended track is still the current one.
                let is_current = {
                    let s = state.lock();
                    match (s.current().map(|e| e.track.uri().to_string()), track_id.to_uri()) {
                        (Some(cur_uri), Ok(ended_uri)) => cur_uri == ended_uri,
                        // If we can't compare, fall back to advancing (old behavior).
                        _ => true,
                    }
                };
                if is_current {
                    tracing::info!("Track ended (decode); waiting for the buffered tail to play out");
                    // EndOfTrack means "finished decoding into the buffer",
                    // several seconds before the listener hears the end.
                    // Advance only after the pipeline runs dry, or the last
                    // seconds of every song get wiped by the track start.
                    // The after_track tag still guards against a manual `n`
                    // racing in during (or after) the wait.
                    spawn_drained_advance(
                        cmd_tx.clone(),
                        pipeline_drained.clone(),
                        pause_flag.clone(),
                        track_id.to_uri().ok(),
                    );
                } else {
                    tracing::debug!("Ignoring stale Spotify EndOfTrack for {track_id:?}");
                }
            }
            PlayerEvent::Unavailable { track_id, .. } => {
                tracing::warn!("Track unavailable: {track_id:?}, skipping");
                let _ = cmd_tx.send(BotCommand::Next {
                    user_id: 0,
                    after_track: track_id.to_uri().ok(),
                });
            }
            PlayerEvent::TimeToPreloadNextTrack { .. } => {
                let _ = cmd_tx.send(BotCommand::PreloadNext);
            }
            PlayerEvent::PositionChanged { position_ms, .. }
            | PlayerEvent::PositionCorrection { position_ms, .. }
            | PlayerEvent::Seeked { position_ms, .. } => {
                let mut s = state.lock();
                s.position_ms = position_ms;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::state::PlayerState;
    use crate::spotify::types::SpotifyTrack;
    use crate::track::Track;

    // -- startup_auth_plan --

    #[test]
    fn youtube_only_user_without_creds_skips_eager_connect() {
        assert_eq!(startup_auth_plan(false, false, true), StartupAuthPlan::Skip);
        assert_eq!(startup_auth_plan(false, false, false), StartupAuthPlan::Skip);
    }

    #[test]
    fn interactive_contexts_keep_fatal_eager_connect() {
        // Cached creds present, spotify default, or both — with OAuth feasible
        // a startup failure should still abort (user is there to see/fix it).
        assert_eq!(startup_auth_plan(true, false, true), StartupAuthPlan::ConnectFatal);
        assert_eq!(startup_auth_plan(false, true, true), StartupAuthPlan::ConnectFatal);
        assert_eq!(startup_auth_plan(true, true, true), StartupAuthPlan::ConnectFatal);
    }

    #[test]
    fn noninteractive_contexts_never_die_on_spotify_failure() {
        // systemd: OAuth infeasible. Failure must disable Spotify, not kill the
        // bot — a fatal exit here becomes a TT login/logout crash-restart loop.
        assert_eq!(startup_auth_plan(true, false, false), StartupAuthPlan::ConnectBestEffort);
        assert_eq!(startup_auth_plan(false, true, false), StartupAuthPlan::ConnectBestEffort);
        assert_eq!(startup_auth_plan(true, true, false), StartupAuthPlan::ConnectBestEffort);
    }

    // -- DrainWait --

    #[test]
    fn drain_wait_needs_two_consecutive_drained_polls() {
        let mut w = DrainWait::new();
        assert!(!w.observe(true));
        assert!(w.observe(true));
    }

    #[test]
    fn drain_wait_resets_on_a_busy_poll() {
        // A chunk can be in flight between the channel and the framer: one
        // empty poll isn't proof. A busy poll restarts the count.
        let mut w = DrainWait::new();
        assert!(!w.observe(true));
        assert!(!w.observe(false));
        assert!(!w.observe(true));
        assert!(w.observe(true));
    }

    // -- auto_advance_is_stale --

    #[test]
    fn manual_next_is_never_stale() {
        assert!(!auto_advance_is_stale(None, Some("spotify:track:a")));
        assert!(!auto_advance_is_stale(None, None));
    }

    #[test]
    fn auto_advance_runs_when_ended_track_is_still_current() {
        assert!(!auto_advance_is_stale(Some("spotify:track:a"), Some("spotify:track:a")));
    }

    #[test]
    fn auto_advance_is_stale_after_queue_moved() {
        // Track A ended naturally, but a manual `n` (processed first) already
        // advanced the queue to B — the auto-advance must not fire again.
        assert!(auto_advance_is_stale(Some("spotify:track:a"), Some("spotify:track:b")));
        assert!(auto_advance_is_stale(Some("spotify:track:a"), None));
    }

    // -- StartFailureBrake --

    #[test]
    fn brake_trips_after_cap_consecutive_failures() {
        let mut brake = StartFailureBrake::new(3);
        assert!(!brake.on_failure());
        assert!(!brake.on_failure());
        assert!(brake.on_failure());
        // Tripping resets the streak.
        assert!(!brake.on_failure());
    }

    #[test]
    fn brake_resets_on_immediate_success() {
        let mut brake = StartFailureBrake::new(3);
        assert!(!brake.on_failure());
        assert!(!brake.on_failure());
        brake.on_success();
        assert!(!brake.on_failure());
        assert!(!brake.on_failure());
        assert!(brake.on_failure());
    }

    // -- channel_move_needs_flush --

    #[test]
    fn initial_join_does_not_flush() {
        use ::teamtalk::types::ChannelId;
        // prev == 0 means we had no channel yet (first join after login).
        assert!(!channel_move_needs_flush(ChannelId(0), ChannelId(5)));
    }

    #[test]
    fn rejoining_same_channel_does_not_flush() {
        use ::teamtalk::types::ChannelId;
        assert!(!channel_move_needs_flush(ChannelId(3), ChannelId(3)));
    }

    #[test]
    fn moving_between_channels_flushes() {
        use ::teamtalk::types::ChannelId;
        assert!(channel_move_needs_flush(ChannelId(1), ChannelId(5)));
        assert!(channel_move_needs_flush(ChannelId(5), ChannelId(1)));
    }

    fn track(id: &str, duration_ms: u32) -> Track {
        Track::Spotify(SpotifyTrack {
            id: id.to_string(),
            name: format!("T{id}"),
            artists: vec!["A".to_string()],
            album: "Album".to_string(),
            duration_ms,
            uri: format!("spotify:track:{id}"),
        })
    }

    fn enqueue(state: &mut PlayerState, durations_ms: &[u32]) {
        for (i, d) in durations_ms.iter().enumerate() {
            state.enqueue(track(&i.to_string(), *d), "u".into(), true);
        }
    }

    // -- empty / not-applicable cases --

    #[test]
    fn queue_wait_info_empty_when_no_current() {
        let state = PlayerState::new();
        assert_eq!(queue_wait_info(&state), "");
    }

    #[test]
    fn queue_wait_info_empty_when_only_current_track() {
        let mut state = PlayerState::new();
        enqueue(&mut state, &[180_000]);
        assert_eq!(queue_wait_info(&state), "");
    }

    // -- "next" position (1 upcoming) --

    #[test]
    fn queue_wait_info_one_upcoming_zero_position_says_next() {
        let mut state = PlayerState::new();
        // Two tracks: current full duration unplayed, one upcoming.
        // Wait = 60s remaining on current → rounds to 1 min.
        enqueue(&mut state, &[60_000, 120_000]);
        // position_ms=0 (default) → wait = 60_000 - 0 = 60_000ms → 1 min.
        assert_eq!(queue_wait_info(&state), " (next, ~1 min)");
    }

    #[test]
    fn queue_wait_info_subtracts_position_from_current_track_wait() {
        let mut state = PlayerState::new();
        enqueue(&mut state, &[180_000, 60_000]);
        state.position_ms = 150_000; // 30s left on current
        // Wait = 30s → (30000+30000)/60000 = 1 min.
        assert_eq!(queue_wait_info(&state), " (next, ~1 min)");
    }

    #[test]
    fn queue_wait_info_under_thirty_seconds_drops_minute_suffix() {
        let mut state = PlayerState::new();
        enqueue(&mut state, &[20_000, 60_000]);
        // Wait = 20s → (20000+30000)/60000 = 0 min → no "~N min".
        assert_eq!(queue_wait_info(&state), " (next)");
    }

    // -- multi-upcoming --

    #[test]
    fn queue_wait_info_multi_upcoming_uses_ahead_form() {
        let mut state = PlayerState::new();
        // queue [A=120s, B=60s, C=60s, D=60s], current=A, asking about D's wait.
        // upcoming_pos = total(4) - current_idx(0) - 1 = 3.
        // Wait = remaining(A=120s) + B(60s) + C(60s) = 240s = 4 min.
        // (D itself is not summed — wait is "until D starts".)
        enqueue(&mut state, &[120_000, 60_000, 60_000, 60_000]);
        assert_eq!(queue_wait_info(&state), " (3 ahead, ~4 min)");
    }

    #[test]
    fn queue_wait_info_does_not_count_last_upcoming_track_duration() {
        // Defensive test for the "wait until the newly-queued (last) track starts"
        // semantic: skip(current+1).take(upcoming_pos - 1) excludes the final entry.
        let mut state = PlayerState::new();
        // queue [A=60s, B=60s, C=999_999_000ms (huge)], current=A.
        // wait = 60s (remaining A) + 60s (B). C is excluded.
        enqueue(&mut state, &[60_000, 60_000, 999_999_000]);
        // Wait = 120s → (120000+30000)/60000 = 2 min.
        assert_eq!(queue_wait_info(&state), " (2 ahead, ~2 min)");
    }

    #[test]
    fn queue_wait_info_position_past_current_duration_saturates_to_zero() {
        // Edge: position_ms > current.duration_ms (shouldn't happen but
        // saturating_sub guards it). With upcoming_pos=1, only the (saturated)
        // remainder of the current track is summed → wait_ms=0 → "(next)".
        let mut state = PlayerState::new();
        enqueue(&mut state, &[10_000, 60_000]);
        state.position_ms = 99_999_999;
        assert_eq!(queue_wait_info(&state), " (next)");
    }
}
