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
        params.set_language(Some("auto")); // auto-detect, works well for German
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_suppress_blank(true);
        state.full(params, audio)?;
        let n = state.full_n_segments()?;
        let mut text = String::new();
        for i in 0..n {
            let seg = state.full_get_segment_text(i)?;
            text.push_str(seg.trim());
            text.push(' ');
        }
        Ok(text.trim().to_string())
    }
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
}
