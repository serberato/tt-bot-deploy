//! Reusable bot runner.
//!
//! Contains the full bot lifecycle: TeamTalk setup, Spotify auth,
//! audio pipeline, command processor, and event loop.
//! Used by both the standalone binary and the Windows tray manager.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::BotConfig;
use crate::error::BotError;

pub mod command_loop;
pub mod context;
pub mod lifecycle;
pub mod player_loop;
pub mod spotify_recovery;
pub mod setup;

use context::ChannelTracker;
use lifecycle::{
    log_startup_versions, resolve_exit_reason, run_teamtalk_event_loop,
    setup_spotify_connection, setup_teamtalk_connection,
};

#[cfg(test)]
pub(crate) use crate::bot::controller::{
    auto_advance_is_stale, channel_move_needs_flush, startup_auth_plan, DrainWait,
    StartFailureBrake, StartupAuthPlan,
};
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

    let setup = setup::initialize_bot_runtime(
        config.clone(),
        config_path,
        shutdown.clone(),
        event_tx.clone(),
        last_channel.clone(),
    )
    .await?;

    let dispatcher = Arc::new(crate::bot::commands::CommandDispatcher {
        state: setup.state.clone(),
        volume: setup.volume.clone(),
        cmd_tx: setup.cmd_tx.clone(),
        max_volume: config.max_volume,
        start_time: std::time::Instant::now(),
        auth: crate::bot::auth::AdminAuth::from_config(&config),
        i18n: setup.i18n.clone(),
    });

    run_main_event_loop(&config, setup, dispatcher, send_event, shutdown, event_tx, last_channel).await
}

async fn run_main_event_loop(
    config: &BotConfig,
    setup: setup::BotRuntimeSetup,
    dispatcher: Arc<crate::bot::commands::CommandDispatcher>,
    send_event: impl Fn(RunnerEvent),
    shutdown: Arc<AtomicBool>,
    event_tx: Option<crossbeam_channel::Sender<RunnerEvent>>,
    last_channel: Arc<parking_lot::Mutex<Option<String>>>,
) -> Result<BotExit, BotError> {
    tracing::info!("Bot is ready! Listening for commands...");

    if crate::settings::load().check_updates_on_startup {
        tokio::spawn(async {
            if let Ok(Some(info)) = crate::update::check().await {
                tracing::info!("Update {} available - run: ttspotify --update", info.tag);
            }
        });
    }

    {
        let mut status = ::teamtalk::types::UserStatus::default();
        status.gender = setup.bot_gender;
        let _ = setup
            .client
            .set_status(status, &setup.config_store.get_idle_status());
    }
    send_event(RunnerEvent::Idle);

    let channel_tracker = ChannelTracker::from_client_and_config(&setup.client, config);

    let reconnect_exhausted = run_teamtalk_event_loop(
        setup.client.clone(),
        shutdown.clone(),
        setup.exit_reason.clone(),
        event_tx,
        last_channel,
        setup.flags.stream_flush.clone(),
        channel_tracker,
        dispatcher,
    )
    .await?;

    setup.flags.local_shutdown.store(true, Ordering::Relaxed);

    if reconnect_exhausted {
        setup.processor_handle.abort();
        setup.event_loop_handle.abort();
        let _ = setup.client.disconnect();
        return Err(BotError::TeamTalk(
            "Lost connection to the TeamTalk server and auto-reconnect was exhausted".into(),
        ));
    }

    let exit = resolve_exit_reason(
        &setup.client,
        &setup.exit_reason,
        &shutdown,
        setup.processor_handle,
        setup.event_loop_handle,
    )
    .await;
    Ok(exit)
}

#[cfg(test)]
mod tests;
