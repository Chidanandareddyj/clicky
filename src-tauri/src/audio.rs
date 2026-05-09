//! Live microphone PCM streaming for Deepgram (callback → std channel).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::sync::mpsc::Sender;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MicError {
    #[error("no input device")]
    NoDevice,
    #[error("cpal: {0}")]
    Cpal(String),
}

fn mono_downmix_linear(samples_interleaved_f32: &[f32], channel_count: usize) -> Vec<f32> {
    if channel_count <= 1 {
        return samples_interleaved_f32.to_vec();
    }
    let frame_quantity = samples_interleaved_f32.len() / channel_count;
    let mut mono = Vec::with_capacity(frame_quantity);
    for frame_position in 0..frame_quantity {
        let mut acc = 0.0_f32;
        for ch in 0..channel_count {
            acc += samples_interleaved_f32[frame_position * channel_count + ch];
        }
        mono.push(acc / channel_count as f32);
    }
    mono
}

fn linear_resampled_mono_pcm16(mono: &[f32], incoming_hz: u32, outgoing_hz: u32) -> Vec<i16> {
    if incoming_hz == 0 || outgoing_hz == 0 {
        return Vec::new();
    }
    if incoming_hz == outgoing_hz {
        return mono
            .iter()
            .map(|s| ((*s).clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .collect();
    }
    let seconds = mono.len() as f64 / incoming_hz as f64;
    let out_frames = (seconds * outgoing_hz as f64).floor().max(0.0) as usize;
    let ratio = outgoing_hz as f64 / incoming_hz as f64;
    let mut out = Vec::with_capacity(out_frames);
    for out_idx in 0..out_frames {
        let pos = out_idx as f64 / ratio;
        let left = pos.floor() as usize;
        let right = (left + 1).min(mono.len().saturating_sub(1));
        let frac = pos - left as f64;
        let interpolated = mono[left] as f64 * (1.0 - frac) + mono[right] as f64 * frac;
        let v = interpolated as f32;
        out.push((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
    }
    out
}

pub struct MicCaptureStreaming {
    _stream: cpal::Stream,
}

impl MicCaptureStreaming {
    /// Targeted 16 kHz mono little-endian PCM for Deepgram `linear16`.
    pub fn start(
        pcm_out: Sender<Vec<i16>>,
        target_hz: u32,
        keep_recording_flag: Arc<AtomicBool>,
    ) -> Result<Self, MicError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(MicError::NoDevice)?;
        let default_cfg = device
            .default_input_config()
            .map_err(|e| MicError::Cpal(e.to_string()))?;
        if default_cfg.sample_format() != SampleFormat::F32 {
            return Err(MicError::Cpal(
                "input stream must expose f32 samples (select another input device)".into(),
            ));
        }

        let stream_cfg: StreamConfig = default_cfg.clone().into();
        let channels = usize::from(stream_cfg.channels);
        let in_hz = stream_cfg.sample_rate.0;

        let flag = Arc::clone(&keep_recording_flag);

        let stream = device
            .build_input_stream(
                &stream_cfg,
                move |data: &[f32], _: &_| {
                    if !flag.load(Ordering::Acquire) {
                        return;
                    }
                    let mono = mono_downmix_linear(data, channels);
                    let pcm16 = linear_resampled_mono_pcm16(&mono, in_hz, target_hz);
                    if pcm16.is_empty() {
                        return;
                    }
                    let _discard_capacity_full = pcm_out.send(pcm16);
                },
                |_err| {},
                None,
            )
            .map_err(|e| MicError::Cpal(e.to_string()))?;

        stream.play().map_err(|e| MicError::Cpal(e.to_string()))?;

        Ok(Self { _stream: stream })
    }
}
