use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::bot::announcer::Announcer;
use crate::bot::commands::BotCommand;
use crate::bot::controller::{Controller, StartFailureBrake};
use crate::bot::handlers::{handle_command, ClientCtx, SpotifyCtx, ChannelCtx, LifecycleCtx, HandlerContext};
use crate::bot::runner::context::CmdContext;

pub(crate) async fn command_processor(
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
        client: ClientCtx {
            controller,
            announcer,
            metadata,
            youtube_metadata,
        },
        spotify: SpotifyCtx {
            session,
            auth,
            connected: spotify_connected,
            recovery_notify,
            recovery_suspended,
            start_brake,
        },
        channel: ChannelCtx {
            search_limit,
            radio_batch_size,
            radio_delay,
            radio_cmd_tx,
            radio_prefetch_slot: Arc::new(parking_lot::Mutex::new(None)),
        },
        lifecycle: LifecycleCtx {
            config_store,
            volume_for_save,
            exit_reason,
            shutdown,
            pending_volume_save: Arc::new(AtomicBool::new(false)),
        },
    };

    while let Some(cmd) = cmd_rx.recv().await {
        handle_command(cmd, &mut handler_ctx).await;
    }
}
