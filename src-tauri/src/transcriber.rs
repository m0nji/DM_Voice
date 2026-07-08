use anyhow::Result;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

/// Bundled Silero-VAD model (ggml-silero-v5.1.2.bin), registered once at
/// startup. `None` or a missing file leaves VAD off and transcription behaves
/// exactly as before — the VAD is an additive pre-filter, never a hard
/// dependency.
static VAD_MODEL: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn init_vad_model(path: Option<PathBuf>) {
    let _ = VAD_MODEL.set(existing_vad_model(path));
}

fn existing_vad_model(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|p| p.exists())
}

fn vad_model_path() -> Option<&'static str> {
    VAD_MODEL
        .get()
        .and_then(|opt| opt.as_ref())
        .and_then(|p| p.to_str())
}

/// Extra breathing room around detected speech so word onsets/tails at the
/// clip edges don't get cut. Applied by Silero segment post-processing.
const VAD_SPEECH_PAD_MS: i32 = 100;

/// 16 kHz mono → 160 samples per centisecond (Silero timestamps are in cs).
const SAMPLES_PER_CS: f32 = 160.0;

enum VadVerdict {
    /// No VAD model available (or VAD errored) — transcribe the full clip.
    Disabled,
    /// Silero found no speech at all — skip Whisper entirely.
    NoSpeech,
    /// Padded speech bounds as sample indices; trim the clip to this range.
    Speech(usize, usize),
}

pub struct WhisperTranscriber {
    ctx: WhisperContext,
    /// Standalone Silero-VAD context. `state.full()` ignores the FullParams
    /// VAD flags (only `whisper_full` on the context's own state honors
    /// them), so we run the VAD ourselves before decoding.
    vad: Option<Mutex<WhisperVadContext>>,
}

/// Per-call diagnostics returned alongside the transcribed text. Used by
/// callers to decide whether the result is trustworthy (e.g. drop on high
/// no_speech_prob or very low avg_logprob — both classic Whisper-hallucination
/// signals).
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
    /// Mean across segments of whisper's per-segment no_speech probability.
    /// `None` when there were no segments.
    pub no_speech_prob: Option<f32>,
    /// Mean log-probability over all emitted (non-special) tokens.
    /// `None` when no tokens were emitted. Range roughly [-3.0, 0.0]; values
    /// below ~-1.0 typically indicate gibberish.
    pub avg_logprob: Option<f32>,
    /// True when the (otherwise valid-looking) output matched the known
    /// silence-hallucination list and was zeroed out by this transcriber.
    pub silence_hallucination: bool,
}

impl WhisperTranscriber {
    pub fn new(model_path: &PathBuf) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true); // Metal on Apple Silicon
        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid path"))?,
            params,
        )?;
        let vad = vad_model_path().and_then(|path| {
            match WhisperVadContext::new(path, WhisperVadContextParams::new()) {
                Ok(v) => Some(Mutex::new(v)),
                Err(e) => {
                    crate::dlog!("vad: init failed ({:?}), transcribing without VAD", e);
                    None
                }
            }
        });
        Ok(Self { ctx, vad })
    }

    /// Run Silero over the clip and reduce the result to one contiguous
    /// padded range from first to last speech segment. Deliberately no
    /// stitching of inner segments: cutting out mid-clip pauses risks
    /// clipping words, while the hallucination hotspots are the leading and
    /// trailing silence.
    fn detect_speech_bounds(&self, audio: &[f32]) -> VadVerdict {
        let Some(vad) = &self.vad else {
            return VadVerdict::Disabled;
        };
        let mut vad = vad.lock().unwrap();
        let mut params = WhisperVadParams::new();
        params.set_speech_pad(VAD_SPEECH_PAD_MS);
        let segments = match vad.segments_from_samples(params, audio) {
            Ok(s) => s,
            Err(e) => {
                // Fail open: a VAD hiccup must never cost the user a dictation.
                crate::dlog!("vad: segmentation failed ({:?}), using full clip", e);
                return VadVerdict::Disabled;
            }
        };
        if segments.num_segments() == 0 {
            return VadVerdict::NoSpeech;
        }
        let (mut start_cs, mut end_cs): (f32, f32) = (f32::MAX, 0.0);
        for seg in segments {
            start_cs = start_cs.min(seg.start);
            end_cs = end_cs.max(seg.end);
        }
        let start = ((start_cs * SAMPLES_PER_CS) as usize).min(audio.len());
        let end = ((end_cs * SAMPLES_PER_CS).ceil() as usize).clamp(start, audio.len());
        // Give whisper at least one second of audio; it degrades on
        // ultra-short inputs.
        let end = end.max((start + 16_000).min(audio.len()));
        VadVerdict::Speech(start, end)
    }

    pub fn transcribe(
        &self,
        audio: &[f32],
        initial_prompt: Option<&str>,
    ) -> Result<TranscriptionResult> {
        // Silero pre-filter (whisper.cpp ≥1.8): trimming leading/trailing
        // non-speech removes the main source of "Vielen Dank"-style silence
        // hallucinations; a clip with no speech at all skips Whisper entirely.
        // The text-side hallucination filter below stays as a second line of
        // defense.
        let audio = match self.detect_speech_bounds(audio) {
            VadVerdict::Disabled => audio,
            VadVerdict::Speech(start, end) => {
                crate::dlog!(
                    "vad: speech at samples {}..{} of {}",
                    start,
                    end,
                    audio.len()
                );
                &audio[start..end]
            }
            VadVerdict::NoSpeech => {
                crate::dlog!("vad: no speech detected, skipping transcription");
                return Ok(TranscriptionResult {
                    text: String::new(),
                    no_speech_prob: Some(1.0),
                    avg_logprob: None,
                    silence_hallucination: false,
                });
            }
        };
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("de"));
        params.set_detect_language(false);
        params.set_translate(false);
        params.set_no_context(true);
        // Must stay false: recordings may run up to MAX_RECORDING_SECS (90 s),
        // but whisper.cpp decodes only the first 30 s window when
        // single_segment is set — everything after that would be dropped.
        // The segment loop below aggregates the multi-segment output.
        params.set_single_segment(false);
        params.set_no_timestamps(true);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.0);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        if let Some(prompt) = initial_prompt {
            // whisper-rs panics on null bytes; callers should pre-filter via
            // config::build_vocabulary_prompt, but guard here too.
            if !prompt.is_empty() && !prompt.contains('\0') {
                params.set_initial_prompt(prompt);
            }
        }
        state.full(params, audio)?;
        let n = state.full_n_segments();
        let mut text = String::new();
        let mut no_speech_sum: f32 = 0.0;
        let mut no_speech_count: usize = 0;
        let mut logprob_sum: f32 = 0.0;
        let mut logprob_count: usize = 0;
        for i in 0..n {
            if let Some(seg) = state.get_segment(i) {
                let s = seg.to_str_lossy()?;
                text.push_str(s.trim());
                text.push(' ');
                no_speech_sum += seg.no_speech_probability();
                no_speech_count += 1;
                let n_tokens = seg.n_tokens();
                for ti in 0..n_tokens {
                    if let Some(tok) = seg.get_token(ti) {
                        let p = tok.token_probability();
                        if p > 0.0 {
                            logprob_sum += p.ln();
                            logprob_count += 1;
                        }
                    }
                }
            }
        }
        let text = text.trim().to_string();
        let no_speech_prob = if no_speech_count > 0 {
            Some(no_speech_sum / no_speech_count as f32)
        } else {
            None
        };
        let avg_logprob = if logprob_count > 0 {
            Some(logprob_sum / logprob_count as f32)
        } else {
            None
        };
        if is_known_silence_hallucination(&text) {
            return Ok(TranscriptionResult {
                text: String::new(),
                no_speech_prob,
                avg_logprob,
                silence_hallucination: true,
            });
        }
        Ok(TranscriptionResult {
            text,
            no_speech_prob,
            avg_logprob,
            silence_hallucination: false,
        })
    }
}

fn is_known_silence_hallucination(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
        .to_lowercase();
    matches!(
        normalized.as_str(),
        "thank you"
            | "thanks for watching"
            | "thanks"
            | "danke"
            | "danke schön"
            | "dankeschön"
            | "vielen dank"
            | "vielen dank für ihre aufmerksamkeit"
            | "bitte"
            | "ja"
            | "nein"
            | "tschüss"
            | "auf wiedersehen"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn transcribes_german_audio() {
        // With the repo-bundled VAD model registered, this exercises the full
        // whisper.cpp-1.8 VAD path (Silero pre-filter + decode) end to end.
        let model = std::env::var("WHISPER_MODEL").expect("set WHISPER_MODEL");
        init_vad_model(Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/vad/ggml-silero-v5.1.2.bin"),
        ));
        let t = WhisperTranscriber::new(&PathBuf::from(model)).unwrap();
        let silence = vec![0.0f32; 16_000];
        let result = t.transcribe(&silence, None).unwrap();
        assert!(result.text.is_empty() || result.text.len() < 10);
    }

    #[test]
    #[ignore]
    fn transcribes_speech_wav_through_vad() {
        // Positive VAD path: SPEECH_WAV must point at a 16 kHz mono s16 WAV
        // with real speech (padded with silence to exercise the trim).
        let model = std::env::var("WHISPER_MODEL").expect("set WHISPER_MODEL");
        let wav = std::env::var("SPEECH_WAV").expect("set SPEECH_WAV");
        init_vad_model(Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("resources/vad/ggml-silero-v5.1.2.bin"),
        ));
        let mut reader = hound::WavReader::open(&wav).unwrap();
        let mut audio: Vec<f32> = vec![0.0; 16_000 * 2]; // 2 s leading silence
        audio.extend(
            reader
                .samples::<i16>()
                .map(|s| s.unwrap() as f32 / 32768.0),
        );
        audio.extend(vec![0.0f32; 16_000 * 3]); // 3 s trailing silence
        let t = WhisperTranscriber::new(&PathBuf::from(model)).unwrap();
        let result = t.transcribe(&audio, None).unwrap();
        println!("transcribed: {:?}", result.text);
        assert!(result.text.to_lowercase().contains("spracherkennung"));
    }

    #[test]
    fn missing_vad_model_leaves_vad_off() {
        assert!(existing_vad_model(Some(PathBuf::from("/nonexistent/ggml-silero.bin"))).is_none());
        assert!(existing_vad_model(None).is_none());
        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources/vad/ggml-silero-v5.1.2.bin");
        assert!(existing_vad_model(Some(bundled)).is_some());
    }

    #[test]
    fn transcriber_struct_exists() {
        let _: fn(&PathBuf) -> Result<WhisperTranscriber> = WhisperTranscriber::new;
    }

    #[test]
    fn filters_common_silence_hallucinations() {
        assert!(is_known_silence_hallucination("Thank you."));
        assert!(is_known_silence_hallucination("Danke."));
        assert!(is_known_silence_hallucination("Vielen Dank."));
        assert!(is_known_silence_hallucination("Vielen Dank!"));
        assert!(!is_known_silence_hallucination("Das ist ein echter Satz."));
    }
}
