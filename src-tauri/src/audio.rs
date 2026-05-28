use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::{Arc, Mutex};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, Copy)]
pub struct AudioStats {
    pub duration_secs: f32,
    pub rms: f32,
    pub peak: f32,
    pub active_ratio: f32,
}

pub fn rms_amplitude(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    mean_sq.sqrt().min(1.0)
}

pub fn peak_amplitude(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|s| s.abs())
        .fold(0.0, f32::max)
        .min(1.0)
}

pub fn active_ratio(samples: &[f32], threshold: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let active = samples.iter().filter(|s| s.abs() >= threshold).count();
    active as f32 / samples.len() as f32
}

pub fn audio_stats(samples: &[f32], sample_rate: u32) -> AudioStats {
    AudioStats {
        duration_secs: samples.len() as f32 / sample_rate as f32,
        rms: rms_amplitude(samples),
        peak: peak_amplitude(samples),
        active_ratio: active_ratio(samples, 0.012),
    }
}

pub fn downmix_to_mono(input: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    if channels == 1 {
        return input.to_vec();
    }
    input
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

pub fn resample(input: &[f32], from_rate: u32, to_rate: u32) -> Result<Vec<f32>> {
    if from_rate == to_rate {
        return Ok(input.to_vec());
    }
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = to_rate as f64 / from_rate as f64;
    let chunk_size = 1024;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, chunk_size, 1)?;
    let mut output = Vec::new();
    let mut pos = 0;
    while pos + chunk_size <= input.len() {
        let chunk = vec![input[pos..pos + chunk_size].to_vec()];
        let out = resampler.process(&chunk, None)?;
        output.extend_from_slice(&out[0]);
        pos += chunk_size;
    }
    Ok(output)
}

/// Enumerate input devices known to cpal's default host. Returns just the
/// names — cpal does not expose CoreAudio UIDs directly, but on macOS the
/// device names are stable enough for re-selection across restarts. Devices
/// that fail to report a name are silently skipped.
pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    match host.input_devices() {
        Ok(iter) => iter.filter_map(|d| d.name().ok()).collect(),
        Err(_) => Vec::new(),
    }
}

pub struct AudioCapture {
    buffer: Arc<Mutex<Vec<f32>>>,
    amplitude: Arc<Mutex<f32>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
    channels: u16,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            amplitude: Arc::new(Mutex::new(0.0)),
            stream: None,
            sample_rate: TARGET_SAMPLE_RATE,
            channels: 1,
        }
    }

    /// Start capture, preferring the named input device. Falls back to the
    /// system default when `preferred` is `None`, when the named device is no
    /// longer present, or when it fails to expose an input config.
    pub fn start_with_device(&mut self, preferred: Option<&str>) -> Result<()> {
        let host = cpal::default_host();
        let device = preferred
            .and_then(|name| {
                host.input_devices().ok().and_then(|mut iter| {
                    iter.find(|d| d.name().ok().as_deref() == Some(name))
                })
            })
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow::anyhow!("No microphone found"))?;
        let resolved_name = device.name().unwrap_or_else(|_| "<unknown>".into());
        let supported = device.default_input_config()?;
        let sample_rate = supported.sample_rate().0;
        self.sample_rate = sample_rate;
        self.channels = supported.channels();
        crate::dlog!(
            "audio: resolved device={:?} (requested={:?}) sample_rate={} channels={} format={:?}",
            resolved_name,
            preferred,
            sample_rate,
            self.channels,
            supported.sample_format()
        );
        let buffer = Arc::clone(&self.buffer);
        let amplitude = Arc::clone(&self.amplitude);
        let config: cpal::StreamConfig = supported.into();
        // Clear before play() so the input callback can't write samples that
        // get wiped immediately after — and so stale samples from prior runs
        // (including streams that may have been started outside this method)
        // are guaranteed gone before capture begins.
        self.buffer.lock().unwrap().clear();
        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let amp = rms_amplitude(data);
                *amplitude.lock().unwrap() = amp;
                buffer.lock().unwrap().extend_from_slice(data);
            },
            |err| crate::dlog!("audio: stream error: {}", err),
            None,
        )?;
        stream.play()?;
        crate::dlog!("audio: stream built and playing on {:?}", resolved_name);
        self.stream = Some(stream);
        Ok(())
    }

    pub fn current_amplitude(&self) -> f32 {
        *self.amplitude.lock().unwrap()
    }

    pub fn stop_and_get_buffer(&mut self) -> Result<Vec<f32>> {
        // Drop the stream first so no further callback can append samples
        // while we're draining. Then take the buffer by std::mem::take so
        // the next recording starts empty even if start_with_device's clear
        // races with a still-firing callback (CoreAudio can post one more
        // buffer after the stream drop on macOS).
        // Explicitly stop the CoreAudio AudioUnit BEFORE dropping the stream.
        // Dropping alone is not enough on macOS: the stream is built on the
        // shortcut-handler thread but dropped here on a tokio worker (this runs
        // inside trigger_transcription's spawned task), and across that thread
        // boundary CoreAudio can leave the input AudioUnit running. That "ghost"
        // stream keeps appending to the shared buffer in parallel with the next
        // recording — observed as a consistent 2x buffer duration that trips the
        // "Buffer drift detected" guard (so nothing is transcribed) and keeps the
        // macOS mic indicator lit. pause() maps to AudioOutputUnitStop, which is
        // thread-safe and synchronous, so stop the unit while we still hold the
        // handle. After take() the handle is gone and the ghost is unreachable.
        let had_stream = self.stream.is_some();
        if let Some(ref stream) = self.stream {
            if let Err(e) = stream.pause() {
                crate::dlog!("audio: stream.pause() on stop failed: {}", e);
            }
        }
        self.stream.take();
        let raw = std::mem::take(&mut *self.buffer.lock().unwrap());
        crate::dlog!(
            "audio: stream paused+dropped (had_stream={}), drained {} raw samples",
            had_stream,
            raw.len()
        );
        let mono = downmix_to_mono(&raw, self.channels);
        let resampled = resample(&mono, self.sample_rate, TARGET_SAMPLE_RATE)?;
        Ok(resampled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_of_silence_is_zero() {
        let silence = vec![0.0f32; 1000];
        assert_eq!(rms_amplitude(&silence), 0.0);
    }

    #[test]
    fn rms_of_full_scale_is_one() {
        let full = vec![1.0f32; 1000];
        assert_eq!(rms_amplitude(&full), 1.0);
    }

    #[test]
    fn rms_of_empty_is_zero() {
        assert_eq!(rms_amplitude(&[]), 0.0);
    }

    #[test]
    fn downmix_stereo_to_mono_averages_frames() {
        let input = vec![0.2, 0.6, -0.4, 0.2];
        assert_eq!(downmix_to_mono(&input, 2), vec![0.4, -0.1]);
    }

    #[test]
    fn resample_passthrough_when_same_rate() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let output = resample(&input, 16_000, 16_000).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn resample_48k_to_16k_reduces_length() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32).sin()).collect();
        let output = resample(&input, 48_000, 16_000).unwrap();
        let ratio = output.len() as f32 / input.len() as f32;
        assert!((ratio - 0.333).abs() < 0.05, "ratio was {}", ratio);
    }
}
