use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bot::commands::CommandDispatcher;
use crate::bot::controller::{channel_move_needs_flush, startup_auth_plan, StartupAuthPlan};
use crate::bot::runner::context::ChannelTracker;
use crate::bot::runner::{BotExit, RunnerEvent};
use crate::config::BotConfig;
use crate::error::BotError;
use crate::services::Service;

/// Log the app version plus the versions of the tools we depend on (TeamTalk
/// SDK, yt-dlp, bgutil-pot). Written to each instance's log at startup so a bug
/// report's log self-identifies exactly what was running.
pub(crate) fn log_startup_versions() {
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

/// Setup TeamTalk connection in a blocking task, respecting any stored channel
/// override from a previous restart.
pub(crate) async fn setup_teamtalk_connection(
    config: &BotConfig,
    last_channel: Arc<parking_lot::Mutex<Option<String>>>,
) -> Result<Arc<::teamtalk::Client>, BotError> {
    let tt_config = {
        let mut c = config.clone();
        if let Some(ch) = last_channel.lock().clone() {
            if ch != c.channel_name {
                tracing::info!(
                    "Restart: rejoining last channel {ch} (default is {})",
                    c.channel_name
                );
                c.channel_name = ch;
            }
        }
        c
    };
    let client = tokio::task::spawn_blocking(move || {
        crate::tt::connection::setup_teamtalk(&tt_config)
    })
    .await
    .map_err(|e| BotError::TeamTalk(format!("TT setup task failed: {e}")))??;
    Ok(Arc::new(client))
}

/// Perform eager startup auth for Spotify if feasible and required.
pub(crate) async fn setup_spotify_connection(
    auth: &crate::spotify::auth::SpotifyAuth,
    session: &librespot_core::session::Session,
    default_service: Service,
    event_tx: &Option<crossbeam_channel::Sender<RunnerEvent>>,
) -> Result<bool, BotError> {
    match startup_auth_plan(
        auth.has_cached_credentials(),
        default_service == Service::Spotify,
        auth.oauth_feasible(),
    ) {
        StartupAuthPlan::ConnectFatal => {
            if let Some(ref tx) = event_tx {
                let _ = tx.send(RunnerEvent::Authenticating);
            }
            auth.connect_existing(session).await?;
            Ok(true)
        }
        StartupAuthPlan::ConnectBestEffort => {
            if let Some(ref tx) = event_tx {
                let _ = tx.send(RunnerEvent::Authenticating);
            }
            match auth.connect_existing(session).await {
                Ok(()) => Ok(true),
                Err(e) => {
                    tracing::error!(
                        "Spotify is unavailable and interactive login is impossible here: {e}. \
                         Continuing without Spotify; run `tt-spotify-bot --auth`, then restart."
                    );
                    Ok(false)
                }
            }
        }
        StartupAuthPlan::Skip => {
            tracing::info!(
                "Skipping Spotify auth at startup; no cached credentials and default service is YouTube"
            );
            Ok(false)
        }
    }
}

/// Run the TeamTalk polling loop in a blocking task, handling reconnects,
/// channel moves, and command dispatching. Returns `true` if auto-reconnect
/// was exhausted.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_teamtalk_event_loop(
    client: Arc<::teamtalk::Client>,
    shutdown: Arc<AtomicBool>,
    exit_reason: Arc<parking_lot::Mutex<Option<BotExit>>>,
    event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    last_channel: Arc<parking_lot::Mutex<Option<String>>>,
    stream_flush: Arc<AtomicBool>,
    channel_tracker: ChannelTracker,
    dispatcher: Arc<CommandDispatcher>,
) -> Result<bool, BotError> {
    const RECONNECT_DEADLINE: Duration = Duration::from_secs(360);
    let event_client = client.clone();
    let last_channel_id = channel_tracker.last_channel_id;
    let last_channel_pw = channel_tracker.last_channel_pw;

    tokio::task::spawn_blocking(move || -> bool {
        let mut disconnected_since: Option<Instant> = None;
        loop {
            if shutdown.load(Ordering::Relaxed) {
                break false;
            }
            if exit_reason.lock().is_some() {
                break false;
            }
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
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(RunnerEvent::Disconnected);
                        }
                    }
                    ::teamtalk::Event::ConnectSuccess => {
                        tracing::info!("Reconnected to server");
                    }
                    ::teamtalk::Event::MySelfLoggedIn => {
                        tracing::info!("Re-logged in after reconnect");
                        disconnected_since = None;
                        let ch = event_client.my_channel_id();
                        let rejoin_ch = *last_channel_id.lock();
                        if rejoin_ch != ::teamtalk::types::ChannelId(0) && ch != rejoin_ch {
                            let pw = last_channel_pw.lock().clone();
                            match event_client.join_channel_and_wait(rejoin_ch, &pw, 5_000) {
                                Ok(_) => tracing::info!("Rejoined channel {} after reconnect", rejoin_ch.0),
                                Err(e) => tracing::warn!("Failed to rejoin channel after reconnect: {e}"),
                            }
                        }
                        if let Some(ref tx) = event_tx {
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
                                    stream_flush.store(true, Ordering::Relaxed);
                                }
                                if let Some(path) = event_client.get_channel_path(user.channel_id) {
                                    *last_channel.lock() = Some(path);
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
    })
    .await
    .map_err(|e| BotError::TeamTalk(format!("Event loop failed: {e}")))
}

/// Resolve exit reason after the polling loop finishes, waiting briefly for
/// any in-flight asynchronous command handler to set exit_reason.
pub(crate) async fn resolve_exit_reason(
    client: &::teamtalk::Client,
    exit_reason: &parking_lot::Mutex<Option<BotExit>>,
    shutdown: &AtomicBool,
    processor_handle: tokio::task::JoinHandle<()>,
    event_loop_handle: tokio::task::JoinHandle<()>,
) -> BotExit {
    for _ in 0..20 {
        if exit_reason.lock().is_some() || shutdown.load(Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let exit = exit_reason.lock().take();
    let reason = match exit {
        Some(reason) => reason,
        None if shutdown.load(Ordering::Relaxed) => BotExit::Shutdown,
        None => BotExit::Quit,
    };

    processor_handle.abort();
    event_loop_handle.abort();
    let _ = client.disconnect();
    reason
}
