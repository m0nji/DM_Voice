//! Continuous wake-word listener. Runs a single 16 kHz mono capture stream
//! while hands-free mode is enabled. State machine:
//!   Listening -> (wake word) -> Recording -> (VAD stop) -> emit buffer -> Listening
//! Never runs concurrently with the push-to-talk capture stream (the app stops
//! this before any push-to-talk dictation begins).

use crate::audio::{downmix_to_mono, rms_amplitude, TARGET_SAMPLE_RATE};
use crate::limits::MAX_RECORDING_SECS;
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
use std::time::Instant;

const RESAMPLE_CHUNK: usize = 1024;

pub enum WakeEvent {
    Detected,
    /// RMS amplitude of the latest recorded chunk, for the waveform overlay.
    /// Emitted continuously between Detected and SpeechEnded so the hands-free
    /// path animates the waveform like the push-to-talk path does.
    Amplitude(f32),
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
                // Diagnostics (see DEBUG.md wake-word investigation 2026-06-02):
                let mut last_fire: Option<Instant> = None; // wall-clock of last FIRED
                let mut rec_started: Option<Instant> = None; // wall-clock recording start

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

                    // Diagnostic: if we're processing a large backlog, the worker
                    // fell behind realtime (CPU/GPU contention) — a lag source.
                    let backlog = frames.len() / FRAME_LENGTH;
                    if backlog > 3 {
                        crate::dlog!(
                            "wake-listener: behind realtime — backlog={} frames (~{}ms), raw_pending={}",
                            backlog,
                            backlog * 80,
                            raw_pending.len()
                        );
                    }

                    // Slice into FRAME_LENGTH (1280) chunks; drive the state machine.
                    while frames.len() >= FRAME_LENGTH {
                        let chunk: Vec<f32> = frames.drain(..FRAME_LENGTH).collect();
                        match &mut phase {
                            Phase::Listening => {
                                let det = detector.detect(&chunk);
                                if det.detected {
                                    let gap_ms =
                                        last_fire.map(|t| t.elapsed().as_millis()).unwrap_or(0);
                                    last_fire = Some(Instant::now());
                                    crate::dlog!(
                                        "wake-listener: FIRED prob={:.3} gap_since_last={}ms frames_buffered={} raw_pending={}",
                                        det.probability,
                                        gap_ms,
                                        frames.len(),
                                        raw_pending.len()
                                    );
                                    if events.send(WakeEvent::Detected).is_err() {
                                        return;
                                    }
                                    rec_started = Some(Instant::now());
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
                                // Feed the waveform overlay. A dropped amplitude
                                // event is harmless (just a skipped frame), so we
                                // ignore send errors here and only bail on the
                                // control events below.
                                let _ = events.send(WakeEvent::Amplitude(rms_amplitude(&chunk)));
                                if vad.push(&chunk) {
                                    let dur = buffer.len() as f32 / TARGET_SAMPLE_RATE as f32;
                                    let wall_ms =
                                        rec_started.map(|t| t.elapsed().as_millis()).unwrap_or(0);
                                    let hit_cap = dur >= (MAX_RECORDING_SECS as f32 - 0.5);
                                    crate::dlog!(
                                        "wake-listener: SpeechEnded buffer_dur={:.2}s wall={}ms hit_cap={} heard_speech={}",
                                        dur,
                                        wall_ms,
                                        hit_cap,
                                        vad.heard_speech()
                                    );
                                    let buf = std::mem::take(buffer);
                                    if events
                                        .send(WakeEvent::SpeechEnded { buffer: buf, duration_s: dur })
                                        .is_err()
                                    {
                                        return;
                                    }
                                    // Clear stale detection state so the buffer
                                    // that just triggered can't immediately
                                    // re-fire on the next listening frames.
                                    detector.reset();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_recording_cap_is_90_seconds() {
        assert_eq!(MAX_RECORDING_SECS, 90);
    }
}
