# oww-rs Wake-Word Integration Spike — Confirmed Facts (2026-06-02)

De-risking SPIKE for the DM-Voice "Hey Jarvis" hands-free mode. Throwaway code; this
file is the deliverable. All facts below were verified by reading the vendored source
and running `examples/wake_spike.rs` on macOS (Apple Silicon, rustc/cargo 1.95).

Status: **DONE** — detection proven on a real "hey jarvis" clip; raw model peak 0.985.

---

## 1. Crate name + dependency line

- Real package name in `vendor/oww-rs/Cargo.toml`: **`oww-rs`** (version `0.3.1`,
  edition 2024). Source: https://github.com/skoky/oww_rs (the *repo* dir is `oww_rs`,
  but the crate is `oww-rs`; the import path in Rust is `oww_rs`, e.g. `use oww_rs::...`).
- Working dependency line (in `src-tauri/Cargo.toml`):

  ```toml
  oww-rs = { path = "vendor/oww-rs", default-features = false }
  ```

  `default-features = false` is **required** — see §7 (cpal coexistence).

## 2. Constructor + wake-word selection

Two constructors on `OwwModel` (in `vendor/oww-rs/src/oww/oww_model.rs`):

```rust
// (a) embedded model, selected by enum — ONLY Alexa + Hey Mycroft are wired:
pub fn new(model_type: SpeechUnlockType, threshold: f32) -> Result<OwwModel, String>

// (b) ARBITRARY model from a file path — THIS is what we use for Hey Jarvis:
pub fn from_file<P: AsRef<Path>>(
    path: P,
    model_unlock_word: String,
    threshold: f32,
) -> io::Result<OwwModel>
```

`SpeechUnlockType` enum (in `src/config.rs`) has only:
- `SpeechUnlockType::OpenWakeWordAlexa`
- `SpeechUnlockType::OpenWakeWordHeyMycroft`

There is **NO `HeyJarvis` enum variant** and the `new()` path's embedded set is
hardcoded to `alexa.onnx` / `hey_mycroft_v0.1.onnx`. **For Hey Jarvis we MUST use
`from_file(...)`** with an on-disk `.onnx`. (The free-text `model_unlock_word` string is
only stored for display — it does not affect inference.)

Wake words actually available to us (openWakeWord v0.5.1 `.onnx`, all confirmed
compatible with the bundled feature models — see §6):
- **hey_jarvis** (`hey_jarvis_v0.1.onnx`) — our target.
- alexa (`alexa_v0.1.onnx`).
- hey_mycroft (`hey_mycroft_v0.1.onnx`).
- (crate also ships a Czech `ahoj_hugo.onnx`; ignore.)
- Other openWakeWord models in the same release (hey_rhasspy, timer, weather, etc.) can
  be dropped in the same way via `from_file`.

## 3. Detection method + return type

Public per-frame call (in `src/oww/oww_model.rs`):

```rust
pub fn detection(&mut self, chunk_f32: Vec<f32>) -> Detection
```

`Detection` (in `src/model.rs`):

```rust
pub struct Detection {
    pub detected: bool,     // <-- the boolean hit signal
    pub probability: f32,   // <-- score field (see CAVEAT below)
    pub duration_ms: u128,  // inference time for the frame
}
```

- The hit flag is **`Detection.detected`**.
- **CAVEAT — `probability` is NOT the raw per-frame model score.** It is a *running
  average over only the above-threshold frames* in a 12-frame (~1s) rolling buffer, and
  is `0.0` until the debounce conditions are met. The raw per-frame model output (the
  thing that actually peaks at ~0.985 on "hey jarvis") is computed inside
  `OwwModel::detect()` but is **not exposed** through the public `detection()` API. It is
  only visible via `RUST_LOG=oww_rs=trace` ("Tract probability: ...").
  → **Next phase: if we want raw scores for our own gating/UI, add a getter that returns
  the raw probability, or fork `detect()`.**

`detected` latches only when ALL of these hold (constants in `src/oww/oww_model.rs`):
- raw frame prob < 0.1 (i.e. the phrase just *ended*), AND
- rolling average of above-threshold frames > `threshold`, AND
- `> MIN_POSITIVE_DETECTIONS` (= 3.0) frames in the 12-frame buffer were above threshold, AND
- `> NO_DETECTION_MS` (= 2000 ms) elapsed since the last hit (cooldown).

Implication: the engine is tuned for a **continuous live-mic stream**, not single short
clips. A lone 1.07 s clip produces only ONE high frame → `detected` never fires even
though raw prob hits 0.985. Feeding the phrase repeatedly (sustained stream) fires
reliably. **The real listener must stream frames continuously; do NOT expect a one-shot
clip to trip `.detected`.**

## 4. frame_length()

- `OwwModel::frame_length()` (via the `Model` trait) returns
  `OWW_MODEL_CHUNK_SIZE` = **1280 samples** (public const at
  `oww_rs::oww::OWW_MODEL_CHUNK_SIZE`).
- 1280 samples @ 16 kHz = **80 ms per frame**.
- The mel model input is hard-faceted to `[1, 1280]`, so chunks MUST be exactly 1280
  f32 samples. Drop trailing partial frames.

## 5. Input format

**Confirmed: 16 kHz, mono, f32 in [-1, 1].** `detection(Vec<f32>)`. There is a
`detect1_i16(Vec<i16>)` on the `Models` wrapper but the `OwwModel` `detect_i16` impl just
returns `None` — i16 is NOT supported on the OWW path; convert to f32 yourself
(`s as f32 / 32768.0`). The app already resamples mic audio to 16 kHz mono (cpal +
rubato 0.15), so we feed that straight in.

## 6. Model embedding mechanism

Two layers, both via **rust-embed (compiled INTO the binary)**:

1. **Shared feature models** — `melspectrogram.onnx` + `embedding_model.onnx` — are
   embedded with `#[derive(Embed)] #[folder = "models/"]` in `src/oww/audio.rs`. These
   are **always compiled into the crate** (hardcoded), loaded by
   `AudioFeaturesTract::create_default()`. They are NOT read from disk at runtime.
2. **Wake-word models** for the `new()` enum path (`alexa`, `hey_mycroft`) — embedded
   with `#[folder = "speech_models/"]` in `src/oww/oww_model.rs`. Also compiled in.
3. **`from_file()`** — reads the wake-word `.onnx` **from disk at runtime** via
   `std::fs::read`.

**Consequence for bundling (tauri.conf.json):**
- The shared mel + embedding models and the enum-selected models are baked into the
  Rust binary by rust-embed → **no Tauri resource bundling needed for those.**
- BUT since we load Hey Jarvis via `from_file`, the **`hey_jarvis_v0.1.onnx` file MUST
  be bundled as a Tauri resource** and resolved at runtime (e.g. via
  `app.path().resource_dir()`). It lives at
  `src-tauri/resources/wakeword/hey_jarvis_v0.1.onnx`.
- Alternative for next phase: add a `HeyJarvis` variant + `#[folder="speech_models/"]`
  entry so it's embedded too (the file is already copied into
  `vendor/oww-rs/speech_models/hey_jarvis_v0.1.onnx`). That would avoid resource
  bundling entirely. Decide in Phase 3/6.

Model provenance: the vendored `alexa.onnx` is **byte-identical** to openWakeWord
release v0.5.1 `alexa_v0.1.onnx` (`cmp` confirmed). So the v0.5.1 `hey_jarvis_v0.1.onnx`
(downloaded from
`https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/hey_jarvis_v0.1.onnx`)
is guaranteed compatible with the bundled mel/embedding feature models.

## 7. cpal coexistence (the key risk)

Vendored crate originally pulled **cpal 0.17 + rubato 2.0 + tokio + tokio-util** as hard
deps (only used by its `mic` module + the `create_unlock_task_sync` live-mic loop + its
examples). The app uses **cpal 0.15 + rubato 0.15** — different majors → two copies would
link.

Fix applied in the vendored crate (throwaway edits, all under our control):
- Made `cpal`, `tokio`, `tokio-util`, `rubato` **optional**, behind a new `mic` feature
  (default on for upstream users).
- Feature-gated `pub mod mic`, `create_unlock_task_sync`, and the i16→f32 mic import in
  `src/lib.rs` behind `#[cfg(feature = "mic")]`.
- The app consumes it with `default-features = false` → `mic` off → none of cpal 0.17 /
  rubato 2.0 / tokio(-util) are pulled by oww-rs.

Verified with `cargo tree` (no-dev): the only audio crates in the app tree are
**`cpal v0.15.3`** and **`rubato v0.15.0`** — NO cpal 0.17, NO rubato 2.0. The `oww-rs`
subtree shows neither. Single cpal major confirmed.

`tract-onnx` is **pure Rust** — `otool -L` on the built example shows NO onnxruntime /
ort / tract dylib, and `cargo tree` shows no `ort`/`onnxruntime` crate. **No native
dylib to link or ship.** (Compile time for the tract stack is ~30 s cold.)

## 8. Threshold mapping (Low / Medium / High)

Measured on the bundled 16 kHz "hey jarvis" clip (raw peak ≈ 0.985; repeated-phrase
stream to satisfy the debounce). Detection fired at 0.3/0.5/0.7/0.99; silence (2 s zeros)
never fired at any threshold (avg 0.000).

Because the engine's gate also requires >3 high frames + 2 s cooldown, a single short
clip will NOT fire regardless of threshold — threshold tunes *sensitivity vs.
false-positive rejection on a live stream*, not single-clip behaviour. With only one
positive clip + silence available, we could not measure a real false-positive rate, so
the mapping below mirrors openWakeWord's published defaults and must be re-tuned in
Phase 4 with live mic + real background noise.

| Sensitivity | threshold | Rationale |
|-------------|-----------|-----------|
| **Low** (fewest false positives) | **0.7** | strict; needs a clean, confident utterance |
| **Medium** (default)            | **0.5** | openWakeWord's recommended default |
| **High** (most responsive)      | **0.3** | catches quieter/farther speech; more false positives in noise |

**Action for Phase 4:** validate these against a real mic in a noisy room; the
false-positive floor is the binding constraint, and we have no FP data yet.

## 9. Quirks / gotchas for the next phases

1. **First-sample bug in the upstream cpal example.** `examples/cpal_mycroft_test.rs`
   line 84 feeds `chunk.data_f32.first()` — i.e. only the FIRST sample of each 1280-sample
   chunk — into `model.detection()`. That is a bug; **always feed the full 1280-sample
   `Vec<f32>`** (our `wake_spike.rs` does). Do not copy that example's wiring.
2. **`probability` field is a debounced average, not the raw score** (see §3). It is 0.0
   until the gate trips. For a real-time confidence meter / our own gating we need the raw
   per-frame value — add a getter or fork `detect()`.
3. **Detection requires a sustained stream**, not a one-shot clip (>3 high frames in ~1s +
   2 s cooldown). The listener must push frames continuously.
4. **`from_file` uses `.unwrap()` on the tract pipeline** — a malformed/incompatible
   `.onnx` will PANIC, not return `Err`. Validate the model file presence/integrity before
   calling, or catch the panic, in production code.
5. **`OwwModel::new()`'s embedded set does NOT include Hey Jarvis.** Use `from_file`, or add
   an enum variant + `speech_models/` entry if we want it embedded (file already vendored
   at `vendor/oww-rs/speech_models/hey_jarvis_v0.1.onnx`).
6. **`multithread::set_default_executor(Executor::SingleThread)` is set on every
   `detect()`** — global tract state. Fine for our single-detector use; note if we ever run
   tract elsewhere concurrently.
7. The crate is **edition 2024**; our toolchain (1.95) supports it. Fine.

## Repro

```bash
cd src-tauri
cargo build --example wake_spike            # compiles oww-rs (tract) + app, no onnx dylib
cargo run   --example wake_spike            # threshold sweep: positive fires, silence doesn't
# raw per-frame scores:
RUST_LOG=oww_rs=trace cargo run --example wake_spike -- \
  vendor/oww-rs/speech_models/hey_jarvis_v0.1.onnx vendor/oww-rs/hey_jarvis.wav 0.5
```

Files touched by the spike:
- `src-tauri/vendor/oww-rs/` — vendored crate (Cargo.toml + lib.rs feature-gated).
- `src-tauri/vendor/oww-rs/speech_models/hey_jarvis_v0.1.onnx` — added.
- `src-tauri/resources/wakeword/` — on-disk models for the `from_file`/bundling path.
- `src-tauri/Cargo.toml` — path dep (default-features=false) + spike dev-deps.
- `src-tauri/examples/wake_spike.rs` — throwaway proof harness.
