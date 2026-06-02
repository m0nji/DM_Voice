//! Continuous wake-word listener. Runs a single 16 kHz mono capture stream
//! while hands-free mode is enabled. State machine:
//!   Listening -> (wake word) -> Recording -> (VAD stop) -> emit buffer -> Listening
//! Never runs concurrently with the push-to-talk capture stream (the app stops
//! this before any push-to-talk dictation begins).

use crate::audio::{downmix_to_mono, TARGET_SAMPLE_RATE};
use crate::vad::SilenceTracker;
use crate::wake_word::{WakeWordDetector, FRAME_LENGTH};
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

const MAX_RECORDING_SECS: u32 = 60;
const RESAMPLE_CHUNK: usize = 1024;

pub enum WakeEvent {
    Detected,
    SpeechEnded { buffer: Vec<f32>, duration_s: f32 },
}

enum Phase {
    Listening,
    Recording { buffer: Vec<f32>, vad: SilenceTracker },
}

pub struct WakeListener {
    stream: Option<cpal::Stream>,
    running: Arc<AtomicBool>,
}

// Same rationale as AudioGuard in main.rs: cpal::Stream is !Send/!Sync on macOS,
// but we only ever touch it behind AppState's Mutex from one owner at a time.
unsafe impl Send for WakeListener {}
unsafe impl Sync for WakeListener {}

impl WakeListener {
    pub fn new() -> Self {
        Self {
            stream: None,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        self.stream.is_some()
    }

    /// Start listening. `events` receives WakeEvent::Detected / SpeechEnded.
    /// `models_dir` holds the classifier `.onnx` files.
    pub fn start(
        &mut self,
        preferred: Option<&str>,
        models_dir: PathBuf,
        model_name: &str,
        threshold: f32,
        silence_timeout_ms: u32,
        events: Sender<WakeEvent>,
    ) -> Result<()> {
        self.stop();

        // Validate the model loads before opening a stream / spawning a thread,
        // so a bad model name or missing file surfaces as an error to the caller.
        let _ = WakeWordDetector::new(&models_dir, model_name, threshold)?;

        let host = cpal::default_host();
        let device = preferred
            .and_then(|name| {
                host.input_devices().ok().and_then(|mut it| {
                    it.find(|d| d.name().ok().as_deref() == Some(name))
                })
            })
            .or_else(|| host.default_input_device())
            .ok_or_else(|| anyhow::anyhow!("No microphone found"))?;
        let supported = device.default_input_config()?;
        let in_rate = supported.sample_rate().0;
        let channels = supported.channels();
        let config: cpal::StreamConfig = supported.into();
        crate::dlog!(
            "wake-listener: device sample_rate={} channels={}",
            in_rate,
            channels
        );

        let staging: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let staging_cb = Arc::clone(&staging);
        let stream = device.build_input_stream(
            &config,
            move |data: &[f32], _| {
                let mono = downmix_to_mono(data, channels);
                staging_cb.lock().unwrap().extend_from_slice(&mono);
            },
            |err| crate::dlog!("wake-listener: stream error: {}", err),
            None,
        )?;
        stream.play()?;
        self.stream = Some(stream);

        let running = Arc::new(AtomicBool::new(true));
        self.running = Arc::clone(&running);

        let model_name = model_name.to_string();
        let timeout = silence_timeout_ms;
        std::thread::Builder::new()
            .name("dm-voice-wake".into())
            .spawn(move || {
                // Build the detector ON the worker thread (avoids Send bounds on
                // the tract model). Already validated above, so this rarely fails.
                let mut detector = match WakeWordDetector::new(&models_dir, &model_name, threshold) {
                    Ok(d) => d,
                    Err(e) => {
                        crate::dlog!("wake-listener: detector init failed: {}", e);
                        return;
                    }
                };

                // Persistent resampler (native rate -> 16 kHz). None when rates match.
                let mut resampler: Option<SincFixedIn<f32>> = if in_rate != TARGET_SAMPLE_RATE {
                    let params = SincInterpolationParameters {
                        sinc_len: 256,
                        f_cutoff: 0.95,
                        interpolation: SincInterpolationType::Linear,
                        oversampling_factor: 256,
                        window: WindowFunction::BlackmanHarris2,
                    };
                    match SincFixedIn::<f32>::new(
                        TARGET_SAMPLE_RATE as f64 / in_rate as f64,
                        2.0,
                        params,
                        RESAMPLE_CHUNK,
                        1,
                    ) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            crate::dlog!("wake-listener: resampler init failed: {}", e);
                            return;
                        }
                    }
                } else {
                    None
                };

                let mut phase = Phase::Listening;
                let mut raw_pending: Vec<f32> = Vec::new(); // native-rate carry
                let mut frames: Vec<f32> = Vec::new(); // 16 kHz carry

                while running.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    {
                        let mut s = staging.lock().unwrap();
                        if s.is_empty() {
                            continue;
                        }
                        raw_pending.extend(s.drain(..));
                    }

                    // Native -> 16 kHz, carrying the < RESAMPLE_CHUNK remainder.
                    match &mut resampler {
                        None => frames.append(&mut raw_pending),
                        Some(rs) => {
                            while raw_pending.len() >= RESAMPLE_CHUNK {
                                let inp: Vec<f32> = raw_pending.drain(..RESAMPLE_CHUNK).collect();
                                match rs.process(&[inp], None) {
                                    Ok(out) => frames.extend_from_slice(&out[0]),
                                    Err(e) => crate::dlog!("wake-listener: resample error: {}", e),
                                }
                            }
                        }
                    }

                    // Slice into FRAME_LENGTH (1280) chunks; drive the state machine.
                    while frames.len() >= FRAME_LENGTH {
                        let chunk: Vec<f32> = frames.drain(..FRAME_LENGTH).collect();
                        match &mut phase {
                            Phase::Listening => {
                                if detector.detect(&chunk) {
                                    if events.send(WakeEvent::Detected).is_err() {
                                        return;
                                    }
                                    phase = Phase::Recording {
                                        buffer: Vec::new(),
                                        vad: SilenceTracker::new(
                                            TARGET_SAMPLE_RATE,
                                            timeout,
                                            MAX_RECORDING_SECS,
                                        ),
                                    };
                                }
                            }
                            Phase::Recording { buffer, vad } => {
                                buffer.extend_from_slice(&chunk);
                                if vad.push(&chunk) {
                                    let dur = buffer.len() as f32 / TARGET_SAMPLE_RATE as f32;
                                    let buf = std::mem::take(buffer);
                                    if events
                                        .send(WakeEvent::SpeechEnded { buffer: buf, duration_s: dur })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    phase = Phase::Listening;
                                }
                            }
                        }
                    }
                }
                crate::dlog!("wake-listener: worker stopped");
            })?;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(ref s) = self.stream {
            let _ = s.pause();
        }
        self.stream.take();
    }
}
