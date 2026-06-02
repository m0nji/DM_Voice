//! Pure silence-tracking logic for hands-free auto-stop. No audio I/O — it
//! consumes already-captured 16 kHz mono f32 chunks and reports when an
//! utterance has ended (enough trailing silence after speech) or when the
//! hard recording cap is hit.

use crate::audio::rms_amplitude;

/// RMS above this is treated as speech. Aligned with the existing pre-gate
/// energy thresholds in main.rs so the VAD and the post-hoc gate agree on
/// what "silence" means.
const SPEECH_RMS_THRESHOLD: f32 = 0.01;

pub struct SilenceTracker {
    silence_stop_samples: usize,
    max_samples: usize,
    trailing_silence: usize,
    total: usize,
    spoke: bool,
}

impl SilenceTracker {
    pub fn new(sample_rate: u32, silence_timeout_ms: u32, max_secs: u32) -> Self {
        let silence_stop_samples =
            (sample_rate as u64 * silence_timeout_ms as u64 / 1000) as usize;
        let max_samples = sample_rate as usize * max_secs as usize;
        Self {
            silence_stop_samples,
            max_samples,
            trailing_silence: 0,
            total: 0,
            spoke: false,
        }
    }

    /// Feed one chunk. Returns true once recording should stop.
    pub fn push(&mut self, chunk: &[f32]) -> bool {
        self.total += chunk.len();
        if rms_amplitude(chunk) >= SPEECH_RMS_THRESHOLD {
            self.spoke = true;
            self.trailing_silence = 0;
        } else {
            self.trailing_silence += chunk.len();
        }
        if self.total >= self.max_samples {
            return true;
        }
        self.spoke && self.trailing_silence >= self.silence_stop_samples
    }

    /// Whether any speech was observed (used to skip empty transcriptions).
    #[allow(dead_code)]
    pub fn heard_speech(&self) -> bool {
        self.spoke
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speech(n: usize) -> Vec<f32> { vec![0.2f32; n] }
    fn silence(n: usize) -> Vec<f32> { vec![0.0f32; n] }

    #[test]
    fn does_not_stop_on_leading_silence() {
        // 16 kHz, 2 s timeout. Pure silence before any speech must NOT stop.
        let mut t = SilenceTracker::new(16_000, 2000, 60);
        for _ in 0..100 {
            assert!(!t.push(&silence(1600)), "stopped during leading silence");
        }
        assert!(!t.heard_speech());
    }

    #[test]
    fn stops_after_silence_following_speech() {
        let mut t = SilenceTracker::new(16_000, 2000, 60);
        assert!(!t.push(&speech(8000))); // 0.5 s speech
        // 2 s silence = 32000 samples; feed in 1600-sample (0.1 s) chunks.
        let mut stopped = false;
        for _ in 0..40 {
            if t.push(&silence(1600)) { stopped = true; break; }
        }
        assert!(stopped, "did not stop after 2 s trailing silence");
        assert!(t.heard_speech());
    }

    #[test]
    fn silence_resets_when_speech_resumes() {
        let mut t = SilenceTracker::new(16_000, 2000, 60);
        t.push(&speech(1600));
        for _ in 0..10 { t.push(&silence(1600)); } // 1 s silence (< 2 s)
        t.push(&speech(1600));                       // resume → reset
        // Now only 1 s more silence should NOT stop.
        let mut stopped = false;
        for _ in 0..10 { if t.push(&silence(1600)) { stopped = true; } }
        assert!(!stopped, "stopped too early after speech resumed");
    }

    #[test]
    fn stops_at_max_cap_even_with_continuous_speech() {
        let mut t = SilenceTracker::new(16_000, 2000, 1); // 1 s cap
        let mut stopped = false;
        for _ in 0..20 { if t.push(&speech(1600)) { stopped = true; break; } }
        assert!(stopped, "did not stop at hard cap");
    }
}
