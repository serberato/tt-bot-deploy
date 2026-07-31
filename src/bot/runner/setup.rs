//! Initialization helpers and subsystem builders for `run_bot`.

use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use librespot_playback::player::PlayerEventChannel;
use parking_lot::Mutex;
use teamtalk::Client;

use crate::bot::commands::BotCommand;
use crate::bot::runner::command_loop::command_processor;
use crate::bot::runner::context::CmdContext;
use crate::bot::runner::player_loop::player_event_loop;
use crate::bot::runner::spotify_recovery::{
    spotify_supervisor, SpotifyRecovery,
};
use crate::bot::runner::{
    setup_spotify_connection, setup_teamtalk_connection, BotExit, RunnerEvent,
};
use crate::bot::state::{PlayerState, SharedState};
use crate::config::BotConfig;
use crate::error::BotError;
use crate::spotify::auth::SpotifyAuth;
use crate::spotify::metadata::SpotifyMetadata;
use crate::spotify::player::SpotifyPlayer;

pub(crate) type SpotifySessionHolder = Arc<parking_lot::Mutex<librespot_core::session::Session>>;

use crate::bot::runner::context::SharedFlags;

pub(crate) struct MetadataBundle {
    pub player: SpotifyPlayer,
    pub player_event_rx: PlayerEventChannel,
    pub audio_tx: Sender<Vec<i16>>,
    pub metadata: SpotifyMetadata,
    pub youtube_metadata: Arc<crate::youtube::metadata::YouTubeMetadata>,
    pub youtube_player: crate::youtube::player::YouTubePlayer,
    pub session_holder: SpotifySessionHolder,
    pub auth: Arc<SpotifyAuth>,
    pub spotify_connected: bool,
}

pub(crate) struct BotRuntimeSetup {
    pub state: SharedState,
    pub volume: Arc<AtomicU8>,
    pub cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    pub client: Arc<Client>,
    pub i18n: Arc<crate::i18n::I18n>,
    pub config_store: Arc<crate::config::ConfigStore>,
    pub exit_reason: Arc<Mutex<Option<BotExit>>>,
    pub flags: SharedFlags,
    pub processor_handle: tokio::task::JoinHandle<()>,
    pub event_loop_handle: tokio::task::JoinHandle<()>,
    pub bot_gender: teamtalk::types::UserGender,
}

pub(crate) fn init_player_state(config: &BotConfig) -> (SharedState, Arc<AtomicU8>) {
    let mut initial_state = PlayerState::new();
    initial_state.radio_enabled = config.radio_enabled;
    initial_state.repeat_track = config.repeat_track;
    initial_state.repeat_queue = config.repeat_queue;
    initial_state.shuffle = config.shuffle;
    initial_state.active_service = config.default_service;
    let state: SharedState = Arc::new(Mutex::new(initial_state));
    let volume = Arc::new(AtomicU8::new(config.volume.min(config.max_volume)));
    (state, volume)
}

pub(crate) fn spawn_audio_pipeline(
    audio_rx: Receiver<Vec<i16>>,
    client: Arc<Client>,
    volume: Arc<AtomicU8>,
    config: BotConfig,
    flags: &SharedFlags,
) {
    let pipeline_client = client;
    let pipeline_volume = volume;
    let pipeline_config = config;
    let audio_reset = flags.audio_reset.clone();
    let timing_reset = flags.timing_reset.clone();
    let pause_flag = flags.pause_flag.clone();
    let stream_flush = flags.stream_flush.clone();
    let pipeline_drained = flags.pipeline_drained.clone();
    let local_shutdown = flags.local_shutdown.clone();
    let pipeline_pos_ms = flags.pipeline_pos_ms.clone();

    std::thread::spawn(move || {
        let mut pipeline = crate::audio::pipeline::AudioPipeline::new(
            audio_rx,
            pipeline_client,
            pipeline_volume,
            audio_reset,
            timing_reset,
            pause_flag,
            stream_flush,
            pipeline_drained,
            local_shutdown,
            pipeline_pos_ms,
            &pipeline_config,
        );
        pipeline.run();
    });
}

pub(crate) async fn init_metadata_and_players(
    config: &BotConfig,
    config_path: &str,
    audio_tx: Sender<Vec<i16>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    state: SharedState,
    flags: &SharedFlags,
    event_tx: &Option<Sender<RunnerEvent>>,
) -> Result<MetadataBundle, BotError> {
    let profile_name = std::path::Path::new(config_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let auth = SpotifyAuth::new(&profile_name);
    let session = auth.new_session();
    let spotify_connected =
        setup_spotify_connection(&auth, &session, config.default_service, event_tx).await?;

    let session_holder = Arc::new(Mutex::new(session));
    let (player, player_event_rx) = {
        let s = session_holder.lock().clone();
        SpotifyPlayer::new(s, config, audio_tx.clone())
    };
    let metadata = SpotifyMetadata::new(session_holder.clone());
    let auth = Arc::new(auth);
    let youtube_metadata =
        Arc::new(crate::youtube::metadata::YouTubeMetadata::new(config, &profile_name)?);
    let youtube_player = crate::youtube::player::YouTubePlayer::new(
        audio_tx.clone(),
        youtube_metadata.clone(),
        cmd_tx,
        state,
        flags.pipeline_pos_ms.clone(),
    );

    Ok(MetadataBundle {
        player,
        player_event_rx,
        audio_tx,
        metadata,
        youtube_metadata,
        youtube_player,
        session_holder,
        auth,
        spotify_connected,
    })
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_supervisors(
    bundle: MetadataBundle,
    config: &BotConfig,
    config_path: &str,
    state: SharedState,
    volume: Arc<AtomicU8>,
    client: Arc<Client>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<BotCommand>,
    cmd_rx: tokio::sync::mpsc::UnboundedReceiver<BotCommand>,
    flags: &SharedFlags,
    event_tx: Option<Sender<RunnerEvent>>,
    shutdown: Arc<AtomicBool>,
    exit_reason: Arc<Mutex<Option<BotExit>>>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
    Arc<tokio::sync::Notify>,
    Arc<crate::config::ConfigStore>,
    Arc<crate::i18n::I18n>,
) {
    let recovery_notify = Arc::new(tokio::sync::Notify::new());
    let recovery_suspended = Arc::new(AtomicBool::new(false));
    let recovery_guard = Arc::new(crate::spotify::recovery::RecoveryGuard::new());
    let spotify_brake = Arc::new(parking_lot::Mutex::new(crate::bot::controller::StartFailureBrake::new(3)));

    let recovery = SpotifyRecovery {
        session_holder: bundle.session_holder.clone(),
        auth: bundle.auth.clone(),
        config: config.clone(),
        audio_tx: bundle.audio_tx.clone(),
        player: bundle.player.clone(),
        state: state.clone(),
        cmd_tx: cmd_tx.clone(),
        pause_flag: flags.pause_flag.clone(),
        audio_reset: flags.audio_reset.clone(),
        guard: recovery_guard,
        recovery_notify: recovery_notify.clone(),
        local_shutdown: flags.local_shutdown.clone(),
        event_tx: event_tx.clone(),
        pipeline_drained: flags.pipeline_drained.clone(),
        spotify_brake: spotify_brake.clone(),
    };
    let recovery_arc = Arc::new(recovery);
    tokio::spawn(spotify_supervisor(
        (*recovery_arc).clone(),
        recovery_suspended.clone(),
    ));

    tokio::spawn(crate::bot::runner::watchdog::watchdog_loop(
        state.clone(),
        flags.pipeline_drained.clone(),
        bundle.youtube_player.clone(),
        recovery_arc,
        cmd_tx.clone(),
        shutdown.clone(),
    ));

    let config_store = Arc::new(crate::config::ConfigStore::new(
        config_path.to_string(),
        config.clone(),
    ));
    let i18n = Arc::new(crate::i18n::I18n::load(
        &crate::config::config_dir(),
        &config.default_language,
    ));
    let bot_gender = crate::config::parse_gender(&config.bot_gender);

    let cmd_ctx = CmdContext {
        player: bundle.player.clone(),
        metadata: bundle.metadata.clone(),
        youtube_metadata: bundle.youtube_metadata.clone(),
        youtube_player: bundle.youtube_player.clone(),
        session: bundle.session_holder.clone(),
        auth: bundle.auth.clone(),
        spotify_connected: bundle.spotify_connected,
        recovery_notify: recovery_notify.clone(),
        recovery_suspended,
        state: state.clone(),
        client: client.clone(),
        search_limit: config.search_limit,
        radio_batch_size: config.radio_batch_size,
        radio_delay: config.radio_delay,
        radio_cmd_tx: cmd_tx.clone(),
        bot_gender,
        config_store: config_store.clone(),
        audio_reset: flags.audio_reset.clone(),
        timing_reset: flags.timing_reset.clone(),
        pause_flag: flags.pause_flag.clone(),
        pipeline_drained: flags.pipeline_drained.clone(),
        volume_for_save: volume,
        exit_reason,
        shutdown,
        event_tx: event_tx.clone(),
        i18n: i18n.clone(),
        spotify_brake: spotify_brake.clone(),
        youtube_brake: Arc::new(parking_lot::Mutex::new(crate::bot::controller::StartFailureBrake::new(3))),
    };
    let processor_handle = tokio::spawn(async move {
        command_processor(cmd_rx, cmd_ctx).await;
    });

    let event_loop_handle = tokio::spawn(player_event_loop(
        bundle.player_event_rx,
        state,
        cmd_tx,
        bundle.session_holder.clone(),
        recovery_notify.clone(),
        flags.pipeline_drained.clone(),
        flags.pause_flag.clone(),
        spotify_brake.clone(),
    ));

    (
        processor_handle,
        event_loop_handle,
        recovery_notify,
        config_store,
        i18n,
    )
}

pub(crate) async fn initialize_bot_runtime(
    config: BotConfig,
    config_path: String,
    shutdown: Arc<AtomicBool>,
    event_tx: Option<Sender<RunnerEvent>>,
    last_channel: Arc<Mutex<Option<String>>>,
) -> Result<BotRuntimeSetup, BotError> {
    let (state, volume) = init_player_state(&config);
    let (audio_tx, audio_rx) = crossbeam_channel::bounded::<Vec<i16>>(256);
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel::<BotCommand>();

    if let Some(ref tx) = event_tx {
        let _ = tx.send(RunnerEvent::Connecting);
    }
    let client = setup_teamtalk_connection(&config, last_channel).await?;
    if let Some(ref tx) = event_tx {
        let _ = tx.send(RunnerEvent::Connected);
    }

    let flags = SharedFlags::new();
    spawn_audio_pipeline(
        audio_rx,
        client.clone(),
        volume.clone(),
        config.clone(),
        &flags,
    );

    let bundle = init_metadata_and_players(
        &config,
        &config_path,
        audio_tx,
        cmd_tx.clone(),
        state.clone(),
        &flags,
        &event_tx,
    )
    .await?;

    let exit_reason = Arc::new(Mutex::new(None));
    let (proc_h, evt_h, _recovery_notify, config_store, i18n) = spawn_supervisors(
        bundle,
        &config,
        &config_path,
        state.clone(),
        volume.clone(),
        client.clone(),
        cmd_tx.clone(),
        cmd_rx,
        &flags,
        event_tx,
        shutdown,
        exit_reason.clone(),
    );

    let bot_gender = crate::config::parse_gender(&config.bot_gender);
    Ok(BotRuntimeSetup {
        state,
        volume,
        cmd_tx,
        client,
        i18n,
        config_store,
        exit_reason,
        flags,
        processor_handle: proc_h,
        event_loop_handle: evt_h,
        bot_gender,
    })
}
