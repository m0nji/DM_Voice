//! THROWAWAY SPIKE (feature/wake-word-handsfree).
//!
//! Proves that the vendored `oww-rs` (openWakeWord inference via tract-onnx)
//! integrates and detects "Hey Jarvis" from a 16 kHz mono WAV — with no native
//! ONNX-runtime dylib and without dragging a second cpal into the app.
//!
//! Run:  cargo run --example wake_spike -- [path/to/model.onnx] [path/to/clip.wav] [threshold]
//! Defaults: bundled hey_jarvis_v0.1.onnx + hey_jarvis.wav, threshold sweep.
//!
//! NOTE: the OwwModel detection path needs the model fed in `frame_length()`
//! (= OWW_MODEL_CHUNK_SIZE = 1280) sample f32 chunks. The crate's cpal example
//! has a bug where it passes only `chunk.data_f32.first()` — feed the FULL chunk.

use hound::{SampleFormat, WavReader};
use oww_rs::oww::{OwwModel, OWW_MODEL_CHUNK_SIZE};

const VENDOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/oww-rs");

/// Load a WAV as 16 kHz mono f32 in [-1, 1]. Panics if not already 16 kHz mono.
fn load_wav_16k_mono(path: &str) -> Vec<f32> {
    let mut reader = WavReader::open(path).expect("open wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1, "spike expects mono wav: {path}");
    assert_eq!(spec.sample_rate, 16_000, "spike expects 16 kHz wav: {path}");
    match spec.sample_format {
        SampleFormat::Int => reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / 32768.0)
            .collect(),
        SampleFormat::Float => reader.samples::<f32>().map(|s| s.unwrap()).collect(),
    }
}

/// Feed `samples` to a fresh model in 1280-sample chunks; return (max_avg_score, fired).
fn run_clip(model: &mut OwwModel, samples: &[f32], label: &str, verbose: bool) -> (f32, bool) {
    let frame = OWW_MODEL_CHUNK_SIZE; // 1280
    let mut max_score = 0.0f32;
    let mut fired = false;
    for (i, chunk) in samples.chunks(frame).enumerate() {
        if chunk.len() < frame {
            break; // drop trailing partial frame
        }
        let d = model.detection(chunk.to_vec());
        max_score = max_score.max(d.probability);
        if d.detected {
            fired = true;
        }
        if verbose && (d.probability > 0.05 || d.detected) {
            println!(
                "  [{label}] frame {i:>3} t={:>5}ms  prob/avg={:.3}  detected={}  ({}ms)",
                i * frame * 1000 / 16000,
                d.probability,
                d.detected,
                d.duration_ms
            );
        }
    }
    (max_score, fired)
}

fn main() {
    // Honor RUST_LOG (e.g. RUST_LOG=oww_rs=trace) for raw per-frame probability;
    // default to Warn when unset.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn"),
    )
    .init();

    let args: Vec<String> = std::env::args().collect();
    let model_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("{VENDOR}/speech_models/hey_jarvis_v0.1.onnx"));
    let wav_path = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{VENDOR}/hey_jarvis.wav"));

    println!("model : {model_path}");
    println!("clip  : {wav_path}");
    println!("frame_length() = {OWW_MODEL_CHUNK_SIZE} samples (16kHz mono f32)\n");

    let one = load_wav_16k_mono(&wav_path);
    // The crate's `.detected` gate needs >3 frames above threshold inside a 12-frame
    // (~1s) rolling buffer AND a 2s cooldown since the last hit. A single 1.07s clip
    // yields only ONE high frame, so we repeat the phrase a few times back-to-back to
    // emulate a sustained live-mic stream and prove `.detected` actually latches.
    let repeats: usize = std::env::var("REPEATS").ok().and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut positive = Vec::new();
    for _ in 0..repeats {
        positive.extend_from_slice(&one);
    }
    println!(
        "positive clip: {} samples ({:.2}s, phrase x{repeats})\n",
        positive.len(),
        positive.len() as f32 / 16000.0
    );

    // Silence: 2s of zeros — must never fire.
    let silence = vec![0.0f32; 16_000 * 2];

    // If a single threshold was passed, run verbose once; otherwise sweep.
    if let Some(t) = args.get(3).and_then(|s| s.parse::<f32>().ok()) {
        let mut m = OwwModel::from_file(&model_path, "Hey Jarvis".into(), t).expect("load model");
        let (ps, pf) = run_clip(&mut m, &positive, "POS", true);
        let mut ms = OwwModel::from_file(&model_path, "Hey Jarvis".into(), t).expect("load model");
        let (ss, sf) = run_clip(&mut ms, &silence, "SIL", true);
        println!("\nthreshold {t}: positive fired={pf} (max avg {ps:.3}) | silence fired={sf} (max avg {ss:.3})");
        return;
    }

    println!("=== THRESHOLD SWEEP ===");
    println!("thr  | positive fired (max avg) | silence fired (max avg)");
    for t in [0.3f32, 0.5, 0.7] {
        let mut mp = OwwModel::from_file(&model_path, "Hey Jarvis".into(), t).expect("load model");
        let (ps, pf) = run_clip(&mut mp, &positive, "POS", false);
        let mut msn = OwwModel::from_file(&model_path, "Hey Jarvis".into(), t).expect("load model");
        let (ss, sf) = run_clip(&mut msn, &silence, "SIL", false);
        println!("{t:<4} | {:<5} ({ps:.3})            | {:<5} ({ss:.3})", pf, sf);
    }
}
