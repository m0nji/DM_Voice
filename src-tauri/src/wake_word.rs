//! Wrapper around the vendored oww-rs wake-word engine (tract-onnx, pure Rust).
//! Exposes a minimal, stable API so the listener never depends on oww-rs
//! internals. Each wake word's classifier .onnx is loaded from `base_dir`
//! (the bundled resources/wakeword dir); the shared melspectrogram + embedding
//! models are compiled into oww-rs via rust-embed.

use anyhow::Result;
use std::path::Path;

/// Wake words offered in v1 → (config key, bundled classifier filename, label).
/// Keep the keys in sync with resources/wakeword/ and the frontend dropdown.
pub const AVAILABLE_MODELS: &[(&str, &str, &str)] = &[
    ("hey_jarvis", "hey_jarvis_v0.1.onnx", "Hey Jarvis"),
    ("alexa", "alexa_v0.1.onnx", "Alexa"),
    ("hey_mycroft", "hey_mycroft_v0.1.onnx", "Hey Mycroft"),
];

/// Samples per detection frame: 16 kHz, 80 ms. == oww_rs::oww::OWW_MODEL_CHUNK_SIZE.
pub const FRAME_LENGTH: usize = 1280;

/// Per-frame detector output. `probability` is oww-rs's debounced running
/// average (0.0 until a detection is building), useful for diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct Detection {
    pub detected: bool,
    pub probability: f32,
}

pub struct WakeWordDetector {
    model: oww_rs::oww::OwwModel,
}

impl WakeWordDetector {
    /// `model_name` is one of the `AVAILABLE_MODELS` keys. `base_dir` holds the
    /// classifier `.onnx` files (the resolved resource dir at runtime; the
    /// manifest `resources/wakeword` dir in tests).
    pub fn new(base_dir: &Path, model_name: &str, threshold: f32) -> Result<Self> {
        let (_, filename, label) = AVAILABLE_MODELS
            .iter()
            .find(|(k, _, _)| *k == model_name)
            .ok_or_else(|| anyhow::anyhow!("unknown wake word '{}'", model_name))?;
        let path = base_dir.join(filename);
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "wake-word model not found: {}",
                path.display()
            ));
        }
        let model = oww_rs::oww::OwwModel::from_file(&path, label.to_string(), threshold)
            .map_err(|e| anyhow::anyhow!("oww-rs from_file failed: {}", e))?;
        Ok(Self { model })
    }

    /// Number of f32 samples (16 kHz mono) expected per `detect` call.
    #[allow(dead_code)]
    pub fn frame_length(&self) -> usize {
        FRAME_LENGTH
    }

    /// Feed exactly `FRAME_LENGTH` samples. Returns the detection outcome.
    /// oww-rs debounces internally, so a single noisy frame won't fire.
    pub fn detect(&mut self, samples: &[f32]) -> Detection {
        let d = self.model.detection(samples.to_vec());
        Detection {
            detected: d.detected,
            probability: d.probability,
        }
    }

    /// Clear detector state between listening sessions, so a stale detection
    /// buffer from a prior utterance cannot immediately re-fire after a
    /// recording ends.
    pub fn reset(&mut self) {
        self.model.reset();
    }
}

/// Directory holding the bundled classifier models, for use in unit tests.
#[cfg(test)]
fn test_models_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/wakeword")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_default_model_and_reports_frame_length() {
        let d = WakeWordDetector::new(&test_models_dir(), "hey_jarvis", 0.5)
            .expect("load hey_jarvis");
        assert_eq!(d.frame_length(), FRAME_LENGTH);
    }

    #[test]
    fn silence_does_not_trigger() {
        let mut d = WakeWordDetector::new(&test_models_dir(), "hey_jarvis", 0.5).unwrap();
        let frame = d.frame_length();
        let mut fired = false;
        for _ in 0..50 {
            if d.detect(&vec![0.0f32; frame]).detected { fired = true; }
        }
        assert!(!fired, "silence triggered the wake word");
    }

    #[test]
    fn unknown_model_errors() {
        assert!(WakeWordDetector::new(&test_models_dir(), "nope", 0.5).is_err());
    }
}
