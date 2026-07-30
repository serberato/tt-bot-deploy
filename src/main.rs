//! Entry point for tt-spotify-bot CLI daemon.

use tt_spotify_bot::cli::run_cli;
use tt_spotify_bot::error::BotError;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), BotError> {
    run_cli().await
}
