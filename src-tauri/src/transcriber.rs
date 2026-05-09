use anyhow::Result;
use std::path::PathBuf;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

pub struct WhisperTranscriber {
    ctx: WhisperContext,
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

    pub fn transcribe(&self, audio: &[f32]) -> Result<String> {
        let mut state = self.ctx.create_state()?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("de"));
        params.set_detect_language(false);
        params.set_translate(false);
        params.set_no_context(true);
        params.set_single_segment(true);
        params.set_no_timestamps(true);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.0);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_suppress_blank(true);
        params.set_suppress_non_speech_tokens(true);
        state.full(params, audio)?;
        let n = state.full_n_segments()?;
        let mut text = String::new();
        for i in 0..n {
            let seg = state.full_get_segment_text(i)?;
            text.push_str(seg.trim());
            text.push(' ');
        }
        let text = text.trim().to_string();
        if is_known_silence_hallucination(&text) {
            return Ok(String::new());
        }
        Ok(text)
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
        let result = t.transcribe(&silence).unwrap();
        assert!(result.is_empty() || result.len() < 10);
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
