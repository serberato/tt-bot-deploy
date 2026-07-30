//! Audio decoding and resampling logic for YouTube tracks.

use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::Sender;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::bot::state::SharedState;
use crate::youtube::player::interleave::{flush_remaining, interleave_to_i16};
use crate::youtube::player::TrackControl;

const PIPELINE_RATE: u32 = 44_100;
const CHANNELS: usize = 2;

/// Decode + resample buffered compressed audio from memory.
pub fn decode_and_stream(
    bytes: Vec<u8>,
    audio_tx: Sender<Vec<i16>>,
    ctrl: Arc<TrackControl>,
    state: SharedState,
    pipeline_pos_ms: Arc<AtomicU32>,
) -> Result<(), String> {
    let (mut decoder, mut format, track_id, src_rate, src_channels) = setup_decoder(bytes)?;
    let chunk_in: usize = 1024;
    let mut resampler = setup_resampler(src_rate, chunk_in)?;
    let mut buf_l: Vec<f32> = Vec::with_capacity(chunk_in * 4);
    let mut buf_r: Vec<f32> = Vec::with_capacity(chunk_in * 4);
    let mut base_ms: u64 = 0;

    loop {
        if ctrl.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        handle_pause_loop(&ctrl);
        serve_seek(&mut *format, &mut *decoder, &ctrl, &state, &pipeline_pos_ms, track_id, &mut base_ms, &mut buf_l, &mut buf_r);

        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                flush_remaining(resampler.as_mut(), &mut buf_l, &mut buf_r, &audio_tx, chunk_in);
                return Ok(());
            }
            Err(e) => return Err(format!("next_packet: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        decode_packet_into_buffers(&mut *decoder, &packet, &mut buf_l, &mut buf_r, src_channels)?;
        resample_and_send_chunks(
            resampler.as_mut(),
            &mut buf_l,
            &mut buf_r,
            &audio_tx,
            &ctrl,
            &state,
            &pipeline_pos_ms,
            base_ms,
            chunk_in,
        )?;
    }
}

fn handle_pause_loop(ctrl: &TrackControl) {
    while ctrl.paused.load(Ordering::Relaxed) {
        if ctrl.stopped.load(Ordering::Relaxed) || ctrl.seek_requested.load(Ordering::Relaxed) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_seek(
    format: &mut dyn FormatReader,
    decoder: &mut dyn Decoder,
    ctrl: &TrackControl,
    state: &SharedState,
    pipeline_pos_ms: &AtomicU32,
    track_id: u32,
    base_ms: &mut u64,
    buf_l: &mut Vec<f32>,
    buf_r: &mut Vec<f32>,
) {
    if !ctrl.seek_requested.swap(false, Ordering::Relaxed) {
        return;
    }
    let target = ctrl.seek_to_ms.load(Ordering::Relaxed);
    let time = Time { seconds: (target / 1000) as u64, frac: (target % 1000) as f64 / 1000.0 };
    if let Ok(seeked) = format.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(track_id) }) {
        buf_l.clear();
        buf_r.clear();
        decoder.reset();
        *base_ms = target as u64;
        pipeline_pos_ms.store(0, Ordering::Relaxed);
        ctrl.position_ms.store(target, Ordering::Relaxed);
        state.lock().position_ms = target;
        tracing::debug!("YouTube native seek to {target}ms (actual_ts={})", seeked.actual_ts);
    }
}

#[allow(clippy::type_complexity)]
fn setup_decoder(
    bytes: Vec<u8>,
) -> Result<(Box<dyn Decoder>, Box<dyn FormatReader>, u32, u32, usize), String> {
    let source: Box<dyn MediaSource> = Box::new(Cursor::new(bytes));
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let format = probed.format;
    let track = format.default_track().ok_or_else(|| "no default track".to_string())?;
    let track_id = track.id;
    let codec_params = track.codec_params.clone();
    let src_rate = codec_params.sample_rate.ok_or_else(|| "missing sample_rate".to_string())?;
    let src_channels = codec_params.channels.map(|c| c.count()).unwrap_or(2);
    let decoder = symphonia::default::get_codecs()
        .make(&codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder make: {e}"))?;
    Ok((decoder, format, track_id, src_rate, src_channels))
}

fn setup_resampler(src_rate: u32, chunk_in: usize) -> Result<Option<SincFixedIn<f32>>, String> {
    if src_rate == PIPELINE_RATE {
        return Ok(None);
    }
    let params = SincInterpolationParameters {
        sinc_len: 128,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let rs = SincFixedIn::<f32>::new(
        PIPELINE_RATE as f64 / src_rate as f64,
        2.0,
        params,
        chunk_in,
        CHANNELS,
    )
    .map_err(|e| format!("resampler new: {e}"))?;
    Ok(Some(rs))
}

fn decode_packet_into_buffers(
    decoder: &mut dyn Decoder,
    packet: &Packet,
    buf_l: &mut Vec<f32>,
    buf_r: &mut Vec<f32>,
    src_channels: usize,
) -> Result<(), String> {
    let decoded = match decoder.decode(packet) {
        Ok(d) => d,
        Err(SymphoniaError::DecodeError(_)) => return Ok(()),
        Err(e) => return Err(format!("decode: {e}")),
    };
    match decoded {
        AudioBufferRef::F32(buf) => {
            let n = buf.frames();
            let l = buf.chan(0);
            let r = if src_channels >= 2 { buf.chan(1) } else { l };
            buf_l.extend_from_slice(&l[..n]);
            buf_r.extend_from_slice(&r[..n]);
        }
        AudioBufferRef::S16(buf) => {
            let n = buf.frames();
            let l = buf.chan(0);
            let r = if src_channels >= 2 { buf.chan(1) } else { l };
            buf_l.extend(l[..n].iter().map(|&s| s as f32 / 32768.0));
            buf_r.extend(r[..n].iter().map(|&s| s as f32 / 32768.0));
        }
        AudioBufferRef::S32(buf) => {
            let n = buf.frames();
            let l = buf.chan(0);
            let r = if src_channels >= 2 { buf.chan(1) } else { l };
            buf_l.extend(l[..n].iter().map(|&s| s as f32 / 2147483648.0));
            buf_r.extend(r[..n].iter().map(|&s| s as f32 / 2147483648.0));
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resample_and_send_chunks(
    mut resampler: Option<&mut SincFixedIn<f32>>,
    buf_l: &mut Vec<f32>,
    buf_r: &mut Vec<f32>,
    audio_tx: &Sender<Vec<i16>>,
    ctrl: &TrackControl,
    state: &SharedState,
    pipeline_pos_ms: &AtomicU32,
    base_ms: u64,
    chunk_in: usize,
) -> Result<(), String> {
    let mut in_l: Vec<f32> = Vec::with_capacity(chunk_in);
    let mut in_r: Vec<f32> = Vec::with_capacity(chunk_in);
    let out_cap = resampler.as_ref().map(|rs| rs.output_frames_max()).unwrap_or(0);
    let mut out_l: Vec<f32> = vec![0.0; out_cap];
    let mut out_r: Vec<f32> = vec![0.0; out_cap];

    while buf_l.len() >= chunk_in {
        in_l.clear();
        in_r.clear();
        in_l.extend_from_slice(&buf_l[..chunk_in]);
        in_r.extend_from_slice(&buf_r[..chunk_in]);
        buf_l.drain(..chunk_in);
        buf_r.drain(..chunk_in);

        let frame = if let Some(ref mut rs) = resampler {
            let (_, written) = rs
                .process_into_buffer(&[&in_l, &in_r], &mut [out_l.as_mut_slice(), out_r.as_mut_slice()], None)
                .map_err(|e| format!("resample: {e}"))?;
            interleave_to_i16(&out_l[0..written], &out_r[0..written])
        } else {
            interleave_to_i16(&in_l, &in_r)
        };

        if ctrl.seek_requested.load(Ordering::Relaxed) {
            buf_l.clear();
            buf_r.clear();
            break;
        }

        send_frame(audio_tx, ctrl, frame)?;

        let pos = (base_ms + pipeline_pos_ms.load(Ordering::Relaxed) as u64).min(u32::MAX as u64) as u32;
        ctrl.position_ms.store(pos, Ordering::Relaxed);
        state.lock().position_ms = pos;
    }
    Ok(())
}

fn send_frame(audio_tx: &Sender<Vec<i16>>, ctrl: &TrackControl, frame: Vec<i16>) -> Result<(), String> {
    let mut item = Some(frame);
    while let Some(samples) = item.take() {
        if ctrl.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        match audio_tx.send_timeout(samples, Duration::from_millis(100)) {
            Ok(()) => break,
            Err(crossbeam_channel::SendTimeoutError::Timeout(returned)) => item = Some(returned),
            Err(crossbeam_channel::SendTimeoutError::Disconnected(_)) => return Ok(()),
        }
    }
    Ok(())
}
