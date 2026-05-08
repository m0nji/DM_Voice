use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::sync::{Arc, Mutex};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

pub fn rms_amplitude(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_sq = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    mean_sq.sqrt().min(1.0)
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

pub struct AudioCapture {
    buffer: Arc<Mutex<Vec<f32>>>,
    amplitude: Arc<Mutex<f32>>,
    stream: Option<cpal::Stream>,
    sample_rate: u32,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(Mutex::new(Vec::new())),
            amplitude: Arc::new(Mutex::new(0.0)),
            stream: None,
            sample_rate: TARGET_SAMPLE_RATE,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("Kein Mikrofon gefunden"))?;
        let supported = device.default_input_config()?;
        let sample_rate = supported.sample_rate().0;
        self.sample_rate = sample_rate;
        let buffer = Arc::clone(&self.buffer);
        let amplitude = Arc::clone(&self.amplitude);
        let config: cpal::StreamConfig = supported.into();
        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let amp = rms_amplitude(data);
                *amplitude.lock().unwrap() = amp;
                buffer.lock().unwrap().extend_from_slice(data);
            },
            |err| eprintln!("Audio stream error: {}", err),
            None,
        )?;
        stream.play()?;
        self.stream = Some(stream);
        *self.buffer.lock().unwrap() = Vec::new();
        Ok(())
    }

    pub fn current_amplitude(&self) -> f32 {
        *self.amplitude.lock().unwrap()
    }

    pub fn stop_and_get_buffer(&mut self) -> Result<Vec<f32>> {
        self.stream.take();
        let raw = self.buffer.lock().unwrap().clone();
        let resampled = resample(&raw, self.sample_rate, TARGET_SAMPLE_RATE)?;
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
