//! Streaming and download coordination for YouTube tracks using yt-dlp.

use std::io::Read;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;

use crate::bot::state::SharedState;
use crate::youtube::metadata::YouTubeMetadata;
use crate::youtube::player::decoder::decode_and_stream;
use crate::youtube::player::TrackControl;

const MAX_TRACK_BYTES: usize = 512 * 1024 * 1024;

/// Download compressed audio via yt-dlp into memory, then spawn decoding worker.
pub async fn play_track(
    video_id: String,
    metadata: Arc<YouTubeMetadata>,
    audio_tx: Sender<Vec<i16>>,
    ctrl: Arc<TrackControl>,
    state: SharedState,
    pipeline_pos_ms: Arc<AtomicU32>,
) -> Result<(), String> {
    let mut child = metadata.spawn_ytdlp(&video_id).map_err(|e| format!("yt-dlp spawn: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| "yt-dlp stdout was not piped".to_string())?;
    let stderr = child.stderr.take().ok_or_else(|| "yt-dlp stderr was not piped".to_string())?;
    let (stderr_handle, watcher_handle) = spawn_watcher_and_stderr(child, stderr, ctrl.clone());

    let download = download_blocking(stdout, ctrl.clone())
        .await
        .map_err(|e| format!("download worker join: {e}"))?;

    let exit_status = watcher_handle.join().ok().flatten();
    let stderr_text = stderr_handle.join().unwrap_or_default();
    let bytes = verify_download(download, &stderr_text, exit_status)?;

    if ctrl.stopped.load(Ordering::Relaxed) || bytes.is_empty() {
        return Ok(());
    }

    tokio::task::spawn_blocking(move || decode_and_stream(bytes, audio_tx, ctrl, state, pipeline_pos_ms))
        .await
        .map_err(|e| format!("decode worker join: {e}"))?
}

struct KillOnDropChild(Option<std::process::Child>);

impl KillOnDropChild {
    fn new(child: std::process::Child) -> Self {
        Self(Some(child))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        if let Some(ref mut child) = self.0 {
            child.kill()
        } else {
            Ok(())
        }
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        if let Some(mut child) = self.0.take() {
            child.wait()
        } else {
            Err(std::io::Error::other("child already reaped"))
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        if let Some(ref mut child) = self.0 {
            let res = child.try_wait();
            if let Ok(Some(_)) = res {
                self.0 = None;
            }
            res
        } else {
            Ok(None)
        }
    }
}

impl Drop for KillOnDropChild {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_watcher_and_stderr(
    child: std::process::Child,
    stderr: std::process::ChildStderr,
    ctrl: Arc<TrackControl>,
) -> (
    std::thread::JoinHandle<String>,
    std::thread::JoinHandle<Option<std::process::ExitStatus>>,
) {
    let stderr_handle = std::thread::spawn(move || -> String {
        let mut buf = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::BufReader::new(stderr), &mut buf);
        buf
    });

    let ctrl_for_kill = ctrl;
    let watcher_handle = std::thread::spawn(move || -> Option<std::process::ExitStatus> {
        let mut child = KillOnDropChild::new(child);
        loop {
            if ctrl_for_kill.stopped.load(Ordering::Relaxed) {
                let _ = child.kill();
                return child.wait().ok();
            }
            match child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => std::thread::sleep(Duration::from_millis(100)),
                Err(_) => return None,
            }
        }
    });

    (stderr_handle, watcher_handle)
}

fn download_blocking(
    mut stdout: std::process::ChildStdout,
    ctrl: Arc<TrackControl>,
) -> tokio::task::JoinHandle<Result<Vec<u8>, String>> {
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            if ctrl.stopped.load(Ordering::Relaxed) {
                return Ok(Vec::new());
            }
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > MAX_TRACK_BYTES {
                        return Err("track exceeds maximum buffer size".to_string());
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) => return Err(format!("read yt-dlp output: {e}")),
            }
        }
        Ok(buf)
    })
}

fn verify_download(
    download: Result<Vec<u8>, String>,
    stderr_text: &str,
    exit_status: Option<std::process::ExitStatus>,
) -> Result<Vec<u8>, String> {
    let exit_code = exit_status.and_then(|s| s.code()).unwrap_or(-1);
    let is_error = exit_status.is_some_and(|s| !s.success());
    let trimmed_stderr = stderr_text.trim();

    if is_error || trimmed_stderr.contains("ERROR:") || trimmed_stderr.contains("HTTP Error 403") {
        tracing::error!(
            "yt-dlp process exited with code={}: {}",
            exit_code,
            trimmed_stderr
        );
    } else if !trimmed_stderr.is_empty() {
        tracing::warn!("yt-dlp stderr: {}", trimmed_stderr);
    }

    match download {
        Ok(b) if b.is_empty() && is_error => {
            let yt_err = stderr_text
                .lines()
                .find(|l| l.to_lowercase().contains("error"))
                .unwrap_or_else(|| stderr_text.lines().last().unwrap_or(""));
            Err(format!(
                "yt-dlp produced 0 bytes (exit={exit_code}): {}",
                yt_err.chars().take(300).collect::<String>()
            ))
        }
        Ok(b) => Ok(b),
        Err(e) => {
            let yt_err = stderr_text
                .lines()
                .find(|l| l.to_lowercase().contains("error"))
                .unwrap_or_else(|| stderr_text.lines().last().unwrap_or(""));
            Err(format!(
                "{e} (yt-dlp exit={exit_code}, stderr: {})",
                yt_err.chars().take(300).collect::<String>()
            ))
        }
    }
}
