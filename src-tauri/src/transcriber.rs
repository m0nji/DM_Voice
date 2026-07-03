use anyhow::Result;
use std::path::PathBuf;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperTranscriber {
    ctx: WhisperContext,
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
        Ok(Self { ctx })
    }

    pub fn transcribe(
        &self,
        audio: &[f32],
        initial_prompt: Option<&str>,
    ) -> Result<TranscriptionResult> {
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
        let model = std::env::var("WHISPER_MODEL").expect("set WHISPER_MODEL");
        let t = WhisperTranscriber::new(&PathBuf::from(model)).unwrap();
        let silence = vec![0.0f32; 16_000];
        let result = t.transcribe(&silence, None).unwrap();
        assert!(result.text.is_empty() || result.text.len() < 10);
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
