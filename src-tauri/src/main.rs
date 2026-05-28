#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod dlog;
mod audio;
mod config;
mod injector;
mod models;
mod permissions;
mod shortcut;
mod sounds;
mod stats;
mod transcriber;
mod updater;

use audio::{audio_stats, AudioCapture, TARGET_SAMPLE_RATE};
use config::{build_vocabulary_prompt, load_config, save_config, AppConfig, TypingSpeedPreset};
use stats::{MonthStatsPayload, UsageStats};
use models::ModelInfo;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;
use tokio::time::sleep;
use transcriber::WhisperTranscriber;

/// Wrapper for AudioCapture that asserts Send + Sync.
///
/// `cpal::Stream` is `!Send + !Sync` on macOS due to CoreAudio thread affinity
/// markers, but in our usage we always lock the surrounding Mutex before
/// touching the inner Stream and never call cpal APIs concurrently. A single
/// dedicated owner is fine, so we assert the bounds manually.
struct AudioGuard(AudioCapture);
unsafe impl Send for AudioGuard {}
unsafe impl Sync for AudioGuard {}

impl AudioGuard {
    fn new() -> Self {
        Self(AudioCapture::new())
    }
}

impl std::ops::Deref for AudioGuard {
    type Target = AudioCapture;
    fn deref(&self) -> &AudioCapture {
        &self.0
    }
}

impl std::ops::DerefMut for AudioGuard {
    fn deref_mut(&mut self) -> &mut AudioCapture {
        &mut self.0
    }
}

struct AppState {
    audio: Mutex<AudioGuard>,
    recording_start: Mutex<Option<Instant>>,
    auto_stop: Mutex<bool>,
    config: Mutex<AppConfig>,
    transcriber: Mutex<Option<WhisperTranscriber>>,
    // PID of the app that was frontmost BEFORE the overlay was shown.
    // Captured in on_shortcut_pressed so CGEventPostToPid targets the right window
    // even if showing the overlay briefly activates DM Voice itself.
    frontmost_pid: Mutex<Option<i32>>,
    // Mirrors `SharedUpdateState` (also exposed as a Tauri-managed state for
    // updater commands). Held here so `rebuild_tray_menu` can read it without
    // looking up the managed state.
    update: updater::SharedUpdateState,
    usage_stats: Arc<UsageStats>,
}

type SharedState = Arc<AppState>;

const OVERLAY_WIDTH: f64 = 220.0;
const OVERLAY_HEIGHT: f64 = 52.0;
const OVERLAY_BOTTOM_MARGIN: f64 = 60.0;

#[tauri::command]
fn get_config(state: State<'_, SharedState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_shortcut(shortcut: String, state: State<'_, SharedState>, app: AppHandle) {
    if !shortcut::is_valid_shortcut(&shortcut) {
        return;
    }
    let mut cfg = state.config.lock().unwrap();
    cfg.shortcut = shortcut.clone();
    let _ = save_config(&cfg);
    drop(cfg);
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let _ = app.global_shortcut().unregister_all();
    register_shortcut(&app, &shortcut, Arc::clone(&state));
}

#[tauri::command]
fn set_sounds_enabled(enabled: bool, state: State<'_, SharedState>) {
    let mut cfg = state.config.lock().unwrap();
    cfg.sounds_enabled = enabled;
    let _ = save_config(&cfg);
}

#[tauri::command]
fn set_custom_vocabulary(
    words: Vec<String>,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    // Normalize: trim, drop empties + duplicates (case-insensitive), keep order.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cleaned: Vec<String> = Vec::new();
    for w in words {
        let term = w.trim().to_string();
        if term.is_empty() {
            continue;
        }
        let key = term.to_lowercase();
        if seen.insert(key) {
            cleaned.push(term);
        }
    }
    let mut cfg = state.config.lock().unwrap();
    cfg.custom_vocabulary = cleaned;
    save_config(&cfg).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn set_typing_speed_preset(
    preset: String,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let parsed = TypingSpeedPreset::from_str(&preset)
        .ok_or_else(|| format!("unknown preset '{}'", preset))?;
    let mut cfg = state.config.lock().unwrap();
    cfg.typing_speed_preset = parsed;
    save_config(&cfg).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn get_current_month_stats(stats: State<'_, Arc<UsageStats>>) -> MonthStatsPayload {
    stats.current_month()
}

#[tauri::command]
fn list_models() -> Vec<ModelInfo> {
    models::list_models()
}

#[tauri::command]
fn delete_model(filename: String) -> Result<(), String> {
    models::delete_model(&filename).map_err(|e| e.to_string())
}

/// Switch the active transcription model. Persists to config, reloads the
/// Whisper transcriber in-place, and rebuilds the tray menu so the checkmark
/// follows the new selection.
#[tauri::command]
fn set_active_model(
    name: String,
    state: State<'_, SharedState>,
    app: AppHandle,
) -> Result<(), String> {
    let info = models::list_models()
        .into_iter()
        .find(|m| m.name == name && m.installed)
        .ok_or_else(|| format!("model '{}' not installed", name))?;
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.model_name = name.clone();
        save_config(&cfg).map_err(|e| e.to_string())?;
    }
    let path = models::model_path(&info.filename);
    let new_t = transcriber::WhisperTranscriber::new(&path).map_err(|e| e.to_string())?;
    *state.transcriber.lock().unwrap() = Some(new_t);
    dlog!("active model switched to {}", name);
    rebuild_tray_menu(&app, &state);
    Ok(())
}

#[tauri::command]
fn get_permissions() -> permissions::PermissionStatus {
    permissions::status()
}

#[tauri::command]
fn request_permissions() {
    permissions::request_all();
}

#[tauri::command]
fn open_privacy_pane(pane: String) {
    // Opens System Settings → Privacy & Security → <pane>
    // pane: "Microphone" or "Accessibility"
    let url = match pane.as_str() {
        "Microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone",
        "Accessibility" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        _ => return,
    };
    let _ = std::process::Command::new("open").arg(url).spawn();
}

#[tauri::command]
async fn download_model(filename: String, app: AppHandle) -> Result<(), String> {
    let name = filename
        .trim_start_matches("ggml-")
        .trim_end_matches(".bin")
        .trim_end_matches("-q5_0")
        .to_string();
    models::download_model(&filename, move |progress| {
        let _ = app.emit(
            "model-download-progress",
            serde_json::json!({"name": name, "progress": progress}),
        );
    })
    .await
    .map_err(|e| e.to_string())
}

fn show_overlay(app: &AppHandle, state_name: &str) {
    if let Some(w) = app.get_webview_window("overlay") {
        configure_overlay_window(&w);
        // Position at bottom-center of the primary monitor (like SuperWhisper mini)
        if let Ok(Some(monitor)) = w.primary_monitor() {
            let mw = monitor.size().width as f64;
            let mh = monitor.size().height as f64;
            let scale = monitor.scale_factor();
            let overlay_w = OVERLAY_WIDTH * scale;
            let overlay_h = OVERLAY_HEIGHT * scale;
            let x = ((mw - overlay_w) / 2.0) as i32;
            let y = (mh - overlay_h - OVERLAY_BOTTOM_MARGIN * scale) as i32;
            let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
        }
        let _ = w.show();
        let _ = app.emit("recording-state", state_name);
    }
}

fn hide_overlay(app: &AppHandle) {
    let _ = app.emit("recording-state", "idle");
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.hide();
    }
}

fn configure_overlay_window(w: &tauri::WebviewWindow) {
    // Do NOT call set_background_color here — it can re-enable isOpaque on the
    // WKWebView and break the transparency set by transparent:true in the config.
    let _ = w.set_focusable(false);
    let _ = w.set_ignore_cursor_events(true);
    let _ = w.set_shadow(false);
    set_webview_transparent(w);
}

/// Force WKWebView (and any wrapping NSViews) to be non-opaque using raw Objective-C.
/// Tauri's `transparent: true` should do this, but in practice the rectangular border
/// (visible behind the rounded pill) shows that *some* layer in the view hierarchy
/// is still painting an opaque background. This walks the entire NSView tree from
/// `wv.inner()` and forces transparency on every layer, logging the class of each
/// view and its before/after isOpaque state so we can see exactly which layer was
/// the culprit.
#[cfg(target_os = "macos")]
fn set_webview_transparent(w: &tauri::WebviewWindow) {
    use std::ffi::c_void;
    extern "C" {
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
        fn objc_getClass(name: *const u8) -> *mut c_void;
    }
    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgSetBool = unsafe extern "C" fn(*mut c_void, *mut c_void, bool);
    type MsgGetBool = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
    type MsgRespondsTo = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> bool;
    type MsgSetId = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
    type MsgGetIsize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize;

    unsafe fn class_name(obj: *mut c_void) -> String {
        if obj.is_null() {
            return "<null>".into();
        }
        let class_sel = sel_registerName(b"class\0".as_ptr());
        let get_class: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let cls = get_class(obj, class_sel);
        if cls.is_null() {
            return "<no class>".into();
        }
        let name_sel = sel_registerName(b"className\0".as_ptr());
        let get_name: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let nsstr = get_name(obj, name_sel);
        if nsstr.is_null() {
            return "<no name>".into();
        }
        let utf8_sel = sel_registerName(b"UTF8String\0".as_ptr());
        type MsgPtr = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const i8;
        let utf8: MsgPtr = std::mem::transmute(objc_msgSend as *const ());
        let cstr = utf8(nsstr, utf8_sel);
        if cstr.is_null() {
            return "<no utf8>".into();
        }
        std::ffi::CStr::from_ptr(cstr).to_string_lossy().into_owned()
    }

    unsafe fn read_bool(obj: *mut c_void, selector: &[u8]) -> Option<bool> {
        let sel = sel_registerName(selector.as_ptr());
        let resp_sel = sel_registerName(b"respondsToSelector:\0".as_ptr());
        let resp: MsgRespondsTo = std::mem::transmute(objc_msgSend as *const ());
        if !resp(obj, resp_sel, sel) {
            return None;
        }
        let f: MsgGetBool = std::mem::transmute(objc_msgSend as *const ());
        Some(f(obj, sel))
    }

    unsafe fn try_set_bool(obj: *mut c_void, selector: &[u8], val: bool) -> bool {
        let sel = sel_registerName(selector.as_ptr());
        let resp_sel = sel_registerName(b"respondsToSelector:\0".as_ptr());
        let resp: MsgRespondsTo = std::mem::transmute(objc_msgSend as *const ());
        if !resp(obj, resp_sel, sel) {
            return false;
        }
        let f: MsgSetBool = std::mem::transmute(objc_msgSend as *const ());
        f(obj, sel, val);
        true
    }

    /// Walk an NSView tree, log each node, force isOpaque=NO + clear background.
    unsafe fn walk(view: *mut c_void, depth: usize) {
        if view.is_null() {
            return;
        }
        let pad = "  ".repeat(depth);
        let cls = class_name(view);
        let opaque_before = read_bool(view, b"isOpaque\0");
        let draws_before = read_bool(view, b"drawsBackground\0");
        let _ = try_set_bool(view, b"setOpaque:\0", false);
        let _ = try_set_bool(view, b"_setDrawsBackground:\0", false);
        let _ = try_set_bool(view, b"setDrawsBackground:\0", false);
        // Set layer.backgroundColor = clearColor (CGColor).
        // wantsLayer:YES then [view layer] -> setBackgroundColor:[NSColor.clearColor CGColor]
        let _ = try_set_bool(view, b"setWantsLayer:\0", true);
        let layer_sel = sel_registerName(b"layer\0".as_ptr());
        let layer_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let layer = layer_fn(view, layer_sel);
        if !layer.is_null() {
            let nscolor_cls = objc_getClass(b"NSColor\0".as_ptr());
            let clear_sel = sel_registerName(b"clearColor\0".as_ptr());
            let clear_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
            let clear = clear_fn(nscolor_cls, clear_sel);
            let cg_sel = sel_registerName(b"CGColor\0".as_ptr());
            let cg_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
            let cg_clear = cg_fn(clear, cg_sel);
            let setbg_sel = sel_registerName(b"setBackgroundColor:\0".as_ptr());
            let setbg: MsgSetId = std::mem::transmute(objc_msgSend as *const ());
            setbg(layer, setbg_sel, cg_clear);
        }
        let opaque_after = read_bool(view, b"isOpaque\0");
        let draws_after = read_bool(view, b"drawsBackground\0");
        dlog!(
            "{}view {:p} class={} isOpaque {:?}->{:?} drawsBg {:?}->{:?}",
            pad,
            view,
            cls,
            opaque_before,
            opaque_after,
            draws_before,
            draws_after
        );

        // Recurse into subviews
        let subviews_sel = sel_registerName(b"subviews\0".as_ptr());
        let subviews_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let subviews = subviews_fn(view, subviews_sel);
        if subviews.is_null() {
            return;
        }
        let count_sel = sel_registerName(b"count\0".as_ptr());
        let count_fn: MsgGetIsize = std::mem::transmute(objc_msgSend as *const ());
        let count = count_fn(subviews, count_sel);
        let obj_at_sel = sel_registerName(b"objectAtIndex:\0".as_ptr());
        type MsgIdIdx = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void;
        let obj_at: MsgIdIdx = std::mem::transmute(objc_msgSend as *const ());
        for i in 0..count as usize {
            let child = obj_at(subviews, obj_at_sel, i);
            walk(child, depth + 1);
        }
    }

    let r = w.with_webview(|wv| {
        let wk_view: *mut c_void = unsafe { std::mem::transmute(wv.inner()) };
        unsafe {
            dlog!(
                "set_webview_transparent: wv.inner()={:p} class={}",
                wk_view,
                class_name(wk_view)
            );
            if wk_view.is_null() {
                return;
            }

            // Climb up to the NSWindow contentView root and walk DOWN from there
            // so we touch every NSView between the window and the WKWebView.
            let win_sel = sel_registerName(b"window\0".as_ptr());
            let win_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
            let nswindow = win_fn(wk_view, win_sel);
            dlog!("set_webview_transparent: nswindow={:p} class={}", nswindow, class_name(nswindow));

            if !nswindow.is_null() {
                // Force the NSWindow itself to be non-opaque + clear background.
                let _ = try_set_bool(nswindow, b"setOpaque:\0", false);
                let _ = try_set_bool(nswindow, b"setHasShadow:\0", false);
                let nscolor_cls = objc_getClass(b"NSColor\0".as_ptr());
                let clear_sel = sel_registerName(b"clearColor\0".as_ptr());
                let clear_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
                let clear = clear_fn(nscolor_cls, clear_sel);
                let setbg_sel = sel_registerName(b"setBackgroundColor:\0".as_ptr());
                let setbg: MsgSetId = std::mem::transmute(objc_msgSend as *const ());
                setbg(nswindow, setbg_sel, clear);

                let win_opaque = read_bool(nswindow, b"isOpaque\0");
                dlog!("NSWindow isOpaque after force-clear: {:?}", win_opaque);

                // Walk from contentView (root NSView)
                let cv_sel = sel_registerName(b"contentView\0".as_ptr());
                let cv_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
                let content_view = cv_fn(nswindow, cv_sel);
                dlog!("contentView={:p} class={} -- walking subtree:", content_view, class_name(content_view));
                walk(content_view, 0);
            } else {
                // Fall back to walking from the WKWebView only
                walk(wk_view, 0);
            }
        }
    });
    if let Err(e) = r {
        dlog!("set_webview_transparent: with_webview failed: {}", e);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_webview_transparent(_w: &tauri::WebviewWindow) {}

/// Confidence thresholds for the post-transcription gate. Conservative
/// defaults (taken from whisper.cpp's own defaults) so we don't drop valid
/// quiet speech — but they catch the classic gibberish output that comes
/// from feeding Whisper silence or stale-buffer noise.
const NO_SPEECH_PROB_MAX: f32 = 0.6;
const AVG_LOGPROB_MIN: f32 = -1.0;
/// Pre-transcription audio-energy thresholds. Below these we treat the
/// recording as silence and skip Whisper entirely.
const PRE_GATE_RMS_MIN: f32 = 0.005;
const PRE_GATE_ACTIVE_RATIO_MIN: f32 = 0.05;
/// Sanity-check tolerance: how much longer than the shortcut-press the
/// audio buffer may be before we treat it as a stale-buffer leak (the
/// "Couterwell" bug). One second covers stream-stop latency + resampler
/// boundary; anything larger is the audio.rs buffer-clear bug recurring.
const BUFFER_DRIFT_TOLERANCE_S: f32 = 1.0;

fn trigger_transcription(app: AppHandle, state: SharedState, expected_duration: Duration) {
    tauri::async_runtime::spawn(async move {
        let expected_s = expected_duration.as_secs_f32();
        let buffer = {
            let mut audio = state.audio.lock().unwrap();
            audio.stop_and_get_buffer().unwrap_or_default()
        };
        let astats = audio_stats(&buffer, TARGET_SAMPLE_RATE);
        let recording_s = astats.duration_secs;
        dlog!(
            "Audio stats: duration={:.2}s rms={:.5} peak={:.5} active={:.3} samples={} expected={:.2}s",
            astats.duration_secs,
            astats.rms,
            astats.peak,
            astats.active_ratio,
            buffer.len(),
            expected_s
        );
        if buffer.is_empty() {
            dlog!("Empty audio buffer — nothing to transcribe (mic permission missing?)");
            hide_overlay(&app);
            return;
        }
        // Sanity check: buffer length must roughly match how long the user
        // actually held the shortcut. If it's much longer, stale samples
        // from earlier runs leaked through and Whisper will hallucinate on
        // the mixed input. See audio.rs::stop_and_get_buffer / commit
        // history for the original Couterwell bug.
        if recording_s > expected_s + BUFFER_DRIFT_TOLERANCE_S {
            dlog!(
                "Buffer drift detected: duration={:.2}s but shortcut elapsed={:.2}s — discarding to avoid hallucination",
                recording_s,
                expected_s
            );
            show_no_speech_overlay(&app);
            return;
        }
        // Pre-gate: if the recording is essentially silence, skip whisper.
        if astats.rms < PRE_GATE_RMS_MIN || astats.active_ratio < PRE_GATE_ACTIVE_RATIO_MIN {
            dlog!(
                "Pre-gate: audio below speech thresholds (rms={:.5}, active={:.3}) — no transcription",
                astats.rms,
                astats.active_ratio
            );
            show_no_speech_overlay(&app);
            return;
        }
        show_overlay(&app, "processing");
        let t_proc_start = Instant::now();
        let vocab_prompt = {
            let cfg = state.config.lock().unwrap();
            build_vocabulary_prompt(&cfg.custom_vocabulary)
        };
        if let Some(ref p) = vocab_prompt {
            dlog!("Using custom-vocabulary prompt ({} chars)", p.len());
        }
        let outcome = {
            let t = state.transcriber.lock().unwrap();
            if t.is_none() {
                dlog!("Transcriber not loaded — model missing?");
            }
            t.as_ref()
                .and_then(|t| t.transcribe(&buffer, vocab_prompt.as_deref()).ok())
        };
        let processing_s = t_proc_start.elapsed().as_secs_f32();
        let (text, no_speech_prob, avg_logprob, silence_hallucination) = match outcome {
            Some(r) => (r.text, r.no_speech_prob, r.avg_logprob, r.silence_hallucination),
            None => (String::new(), None, None, false),
        };
        dlog!(
            "Transcription result: {:?} ({} chars) no_speech={} avg_logprob={} silence_match={}",
            &text,
            text.len(),
            no_speech_prob
                .map(|v| format!("{:.3}", v))
                .unwrap_or_else(|| "n/a".into()),
            avg_logprob
                .map(|v| format!("{:.3}", v))
                .unwrap_or_else(|| "n/a".into()),
            silence_hallucination
        );
        // Post-gate: Whisper's own confidence signals. Threshold values
        // follow whisper.cpp defaults; tune via the constants above if real
        // speech is being dropped.
        let low_confidence = no_speech_prob.map(|p| p > NO_SPEECH_PROB_MAX).unwrap_or(false)
            || avg_logprob.map(|p| p < AVG_LOGPROB_MIN).unwrap_or(false);
        if !text.is_empty() && low_confidence {
            dlog!(
                "Post-gate: low-confidence transcription discarded (no_speech={:?}, avg_logprob={:?})",
                no_speech_prob,
                avg_logprob
            );
            show_no_speech_overlay(&app);
            return;
        }
        if silence_hallucination {
            show_no_speech_overlay(&app);
            return;
        }
        if text.is_empty() {
            dlog!("Empty transcription — nothing to inject");
            show_no_speech_overlay(&app);
            return;
        }
        {
            let target_pid = *state.frontmost_pid.lock().unwrap();
            dlog!("injecting into pid={:?}", target_pid);
            // NSPasteboard + CGEventPost both work best on the main thread.
            let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), ()>>();
            let t = text.clone();
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || {
                let result = injector::inject_text(&t, target_pid);
                dlog!("inject_text result: {:?}", result);
                let ok = result.is_ok();
                if !ok {
                    let _ = injector::copy_to_clipboard(&t);
                    use tauri_plugin_notification::NotificationExt;
                    let _ = app2
                        .notification()
                        .builder()
                        .title("DM Voice")
                        .body("No text field active — text copied to clipboard")
                        .show();
                }
                let _ = tx.send(if ok { Ok(()) } else { Err(()) });
            });
            let inject_result = rx.await.unwrap_or(Err(()));
            // Record stats only when text was successfully injected into the target.
            // Fallback-clipboard cases are still user-useful but we keep stats honest
            // about "time saved vs typing into the app I was in".
            if inject_result.is_ok() {
                let chars = text.chars().count() as u32;
                state
                    .usage_stats
                    .record(chars, recording_s, processing_s);
            }
        }
        show_overlay(&app, "done");
        sleep(Duration::from_millis(400)).await;
        hide_overlay(&app);
    });
}

/// Show a brief "no speech detected" overlay state, then hide. Used by all
/// gates (pre-, sanity-, post-, empty result) so the user always gets the
/// same feedback when nothing is injected.
fn show_no_speech_overlay(app: &AppHandle) {
    show_overlay(app, "no-speech");
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        sleep(Duration::from_millis(900)).await;
        hide_overlay(&app2);
    });
}

fn on_shortcut_pressed(app: &AppHandle, state: &SharedState) {
    if state.recording_start.lock().unwrap().is_some() {
        return;
    }
    dlog!("on_shortcut_pressed");
    // Capture before show_overlay() — showing a Tauri window can briefly activate
    // DM Voice, causing CGEventPost to paste into the wrong app.
    let pid = injector::frontmost_app_pid();
    *state.frontmost_pid.lock().unwrap() = pid;
    dlog!("on_shortcut_pressed: captured frontmost_pid={:?}", pid);

    let (sounds_enabled, preferred_device) = {
        let cfg = state.config.lock().unwrap();
        (cfg.sounds_enabled, cfg.input_device.clone())
    };
    sounds::play_start(sounds_enabled);

    let mut audio = state.audio.lock().unwrap();
    if let Err(e) = audio.start_with_device(preferred_device.as_deref()) {
        dlog!(
            "on_shortcut_pressed: audio.start() failed (preferred={:?}): {}",
            preferred_device,
            e
        );
        return;
    }
    if let Some(name) = preferred_device.as_deref() {
        dlog!("on_shortcut_pressed: requested input device={}", name);
    }
    *state.recording_start.lock().unwrap() = Some(Instant::now());
    *state.auto_stop.lock().unwrap() = false;
    show_overlay(app, "recording");
    dlog!("on_shortcut_pressed: overlay shown");
    drop(audio);

    let app2 = app.clone();
    let state2 = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        loop {
            sleep(Duration::from_millis(50)).await;
            let elapsed = {
                let start = state2.recording_start.lock().unwrap();
                start.map(|s| s.elapsed())
            };
            match elapsed {
                None => break,
                Some(d) if d > Duration::from_secs(60) => {
                    *state2.recording_start.lock().unwrap() = None;
                    *state2.auto_stop.lock().unwrap() = true;
                    trigger_transcription(app2.clone(), Arc::clone(&state2), d);
                    break;
                }
                _ => {}
            }
            let amp = state2.audio.lock().unwrap().current_amplitude();
            let _ = app2.emit("amplitude", amp);
        }
    });
}

fn on_shortcut_released(app: &AppHandle, state: &SharedState) {
    let start = state.recording_start.lock().unwrap().take();
    if *state.auto_stop.lock().unwrap() {
        dlog!("on_shortcut_released: auto_stop already triggered");
        return;
    }
    let elapsed = start.map(|s| s.elapsed()).unwrap_or_default();
    dlog!("on_shortcut_released: elapsed={:?}", elapsed);
    if elapsed < Duration::from_millis(300) {
        let mut audio = state.audio.lock().unwrap();
        let _ = audio.stop_and_get_buffer();
        drop(audio);
        hide_overlay(app);
        return;
    }
    let sounds_enabled = state.config.lock().unwrap().sounds_enabled;
    sounds::play_end(sounds_enabled);
    trigger_transcription(app.clone(), Arc::clone(state), elapsed);
}

fn register_shortcut(app: &AppHandle, shortcut: &str, state: SharedState) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let app_clone = app.clone();
    let _ = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| match event.state() {
            ShortcutState::Pressed => on_shortcut_pressed(&app_clone, &state),
            ShortcutState::Released => on_shortcut_released(&app_clone, &state),
        });
}

/// Build the tray dropdown: app name + version (disabled), separator, one
/// CheckMenuItem per model (only installed ones are clickable, the active one
/// is checked), separator, quit.
fn build_tray_menu(
    app: &AppHandle,
    cfg: &AppConfig,
    update: &updater::UpdateState,
) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem};

    let header = MenuItem::with_id(
        app,
        "header",
        format!("DM Voice {}", env!("CARGO_PKG_VERSION")),
        false,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;

    // Update items: only the "install" variant is shown when an update is
    // pending. The "check" variant is always shown so the user can poll
    // manually. Both live near the top so they are easy to spot.
    let update_install: Option<MenuItem<tauri::Wry>> =
        if let Some(v) = &update.latest_version {
            Some(MenuItem::with_id(
                app,
                "update_install",
                format!("Install update v{}", v),
                !update.installing,
                None::<&str>,
            )?)
        } else {
            None
        };
    let update_check = MenuItem::with_id(
        app,
        "update_check",
        "Check for updates…",
        !update.installing,
        None::<&str>,
    )?;
    let sep_update = PredefinedMenuItem::separator(app)?;

    let model_header = MenuItem::with_id(app, "model_header", "Model", false, None::<&str>)?;

    let mut model_items: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
    for m in models::list_models() {
        let id = format!("model:{}", m.name);
        let label = if m.installed {
            format!("  {}", m.name)
        } else {
            format!("  {}  (not downloaded)", m.name)
        };
        let checked = m.installed && m.name == cfg.model_name;
        let item = CheckMenuItem::with_id(app, &id, &label, m.installed, checked, None::<&str>)?;
        model_items.push(item);
    }
    let sep_mic = PredefinedMenuItem::separator(app)?;

    // Microphone section — flat layout mirroring the Model section above.
    // We previously rendered this as a Submenu, but on macOS muda 0.16's
    // submenu items hit an AppKit cache-miss path on first hover that
    // dismisses the parent NSMenu without warning (looks to the user like
    // a crash + relaunch). Flat layout side-steps that path entirely.
    let mic_header = MenuItem::with_id(app, "mic_header", "Microphone", false, None::<&str>)?;
    let mic_default = CheckMenuItem::with_id(
        app,
        "mic:default",
        "  System default",
        true,
        cfg.input_device.is_none(),
        None::<&str>,
    )?;
    let devices = audio::list_input_devices();
    let mut mic_device_items: Vec<CheckMenuItem<tauri::Wry>> = Vec::new();
    for name in &devices {
        let id = format!("mic:{}", name);
        let checked = cfg.input_device.as_deref() == Some(name.as_str());
        let item = CheckMenuItem::with_id(
            app,
            &id,
            format!("  {}", name),
            true,
            checked,
            None::<&str>,
        )?;
        mic_device_items.push(item);
    }
    // If the saved device isn't currently present, show it as a disabled
    // row so the user can see which preference will reactivate when the
    // device comes back.
    let missing_item: Option<CheckMenuItem<tauri::Wry>> = match cfg.input_device.as_deref() {
        Some(name) if !devices.iter().any(|d| d == name) => Some(CheckMenuItem::with_id(
            app,
            format!("mic:{}", name),
            format!("  {}  (not connected)", name),
            false,
            true,
            None::<&str>,
        )?),
        _ => None,
    };

    let sep2 = PredefinedMenuItem::separator(app)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let autostart_enabled = autostart_is_enabled(app);
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start at login",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit DM Voice", true, None::<&str>)?;

    let mut refs: Vec<&dyn IsMenuItem<tauri::Wry>> = vec![&header, &sep1];
    if let Some(it) = update_install.as_ref() {
        refs.push(it);
    }
    refs.push(&update_check);
    refs.push(&sep_update);
    refs.push(&model_header);
    for it in &model_items {
        refs.push(it);
    }
    refs.push(&sep_mic);
    refs.push(&mic_header);
    refs.push(&mic_default);
    for it in &mic_device_items {
        refs.push(it);
    }
    if let Some(it) = missing_item.as_ref() {
        refs.push(it);
    }
    refs.push(&sep2);
    refs.push(&settings_item);
    refs.push(&autostart_item);
    refs.push(&sep3);
    refs.push(&quit_item);

    Menu::with_items(app, &refs)
}

/// Show (or create) the settings window. Called from the tray menu's
/// "Settings…" item — the tray-icon click itself just opens the menu,
/// matching macOS status-bar conventions.
///
/// Agent apps (LSUIElement=true) don't auto-activate when a window is shown —
/// `[NSApp activateIgnoringOtherApps:YES]` is required before show/focus or
/// the window stays behind whatever app currently has the keyboard. We do not
/// touch the dictation/injection path here; paste targeting still resolves
/// via `frontmost_app_pid` captured at shortcut-press time, which already
/// excludes DM Voice's own PID.
fn show_settings_window(app: &AppHandle) {
    activate_app_macos();
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    } else if let Ok(w) = tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings/index.html".into()),
    )
    .title("DM Voice")
    .inner_size(640.0, 600.0)
    .resizable(false)
    .build()
    {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Bring the agent app (LSUIElement) to the foreground so a newly shown
/// window can actually receive keyboard focus and z-order. Calls
/// `[[NSApplication sharedApplication] activateIgnoringOtherApps:YES]`.
#[cfg(target_os = "macos")]
fn activate_app_macos() {
    use std::ffi::c_void;
    extern "C" {
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
        fn objc_getClass(name: *const u8) -> *mut c_void;
    }
    unsafe {
        let cls = objc_getClass(b"NSApplication\0".as_ptr());
        if cls.is_null() {
            return;
        }
        let shared_sel = sel_registerName(b"sharedApplication\0".as_ptr());
        type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
        let shared: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let ns_app = shared(cls, shared_sel);
        if ns_app.is_null() {
            return;
        }
        let activate_sel = sel_registerName(b"activateIgnoringOtherApps:\0".as_ptr());
        type MsgSetBool = unsafe extern "C" fn(*mut c_void, *mut c_void, bool);
        let activate: MsgSetBool = std::mem::transmute(objc_msgSend as *const ());
        activate(ns_app, activate_sel, true);
    }
}

#[cfg(not(target_os = "macos"))]
fn activate_app_macos() {}

/// Read the current autostart state. Plugin returns false on error, which is
/// the right default for the checkmark.
fn autostart_is_enabled(app: &AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Toggle login-launch on/off. Logs the result so we see in dlog.log whether
/// the LaunchAgent plist was actually written.
fn autostart_toggle(app: &AppHandle) {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let currently = mgr.is_enabled().unwrap_or(false);
    let res = if currently { mgr.disable() } else { mgr.enable() };
    match res {
        Ok(()) => dlog!("autostart toggled: {} -> {}", currently, !currently),
        Err(e) => dlog!("autostart toggle failed: {}", e),
    }
}

fn rebuild_tray_menu(app: &AppHandle, state: &SharedState) {
    let cfg = state.config.lock().unwrap().clone();
    let update = state.update.lock().unwrap().clone();
    if let Ok(menu) = build_tray_menu(app, &cfg, &update) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn main() {
    dlog::init();
    dlog!("dm-voice starting; pid={}", std::process::id());
    permissions::request_all();
    let cfg = load_config();
    let update_state: updater::SharedUpdateState =
        Arc::new(Mutex::new(updater::UpdateState::new()));
    let usage_stats = Arc::new(UsageStats::load());
    let state: SharedState = Arc::new(AppState {
        audio: Mutex::new(AudioGuard::new()),
        recording_start: Mutex::new(None),
        auto_stop: Mutex::new(false),
        config: Mutex::new(cfg.clone()),
        transcriber: Mutex::new(None),
        frontmost_pid: Mutex::new(None),
        update: Arc::clone(&update_state),
        usage_stats: Arc::clone(&usage_stats),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Arc::clone(&state))
        .manage(Arc::clone(&update_state))
        .manage(Arc::clone(&usage_stats))
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_shortcut,
            set_sounds_enabled,
            set_typing_speed_preset,
            set_custom_vocabulary,
            get_current_month_stats,
            list_models,
            delete_model,
            download_model,
            set_active_model,
            get_permissions,
            request_permissions,
            open_privacy_pane,
            updater::check_for_updates,
            updater::get_update_state,
            updater::install_update,
        ])
        .setup(move |app| {
            if let Some(w) = app.get_webview_window("overlay") {
                configure_overlay_window(&w);
            }

            // Point whisper's Metal backend to the bundled ggml-metal.metal shader.
            // ggml-metal.m checks GGML_METAL_PATH_RESOURCES before falling back to
            // NSBundle lookup, which won't find files in subdirectories.
            if let Ok(res_dir) = app.path().resource_dir() {
                let metal_dir = res_dir.join("resources");
                std::env::set_var(
                    "GGML_METAL_PATH_RESOURCES",
                    metal_dir.to_string_lossy().as_ref(),
                );
            }

            // Load transcriber if model is installed
            let model_name = state.config.lock().unwrap().model_name.clone();
            let model_info = models::list_models()
                .into_iter()
                .find(|m| m.name == model_name && m.installed);
            if let Some(info) = model_info {
                let path = models::model_path(&info.filename);
                if let Ok(t) = transcriber::WhisperTranscriber::new(&path) {
                    *state.transcriber.lock().unwrap() = Some(t);
                }
            }

            // System tray
            use tauri::tray::TrayIconBuilder;
            let state_for_menu = Arc::clone(&state);

            let tray_menu = build_tray_menu(
                app.handle(),
                &state.config.lock().unwrap(),
                &state.update.lock().unwrap(),
            )?;

            // Bake the tray icon into the binary at compile time. Loading via
            // env!("CARGO_MANIFEST_DIR") only works on the build machine; in CI
            // builds the path doesn't exist on the user's Mac, the load fails,
            // and the template-mode fallback to default_window_icon shows as
            // a white square in the menu bar.
            let tray_icon_img = tauri::image::Image::from_bytes(include_bytes!(
                "../icons/tray-icon.png"
            ))
            .unwrap_or_else(|_| app.default_window_icon().unwrap().clone());

            TrayIconBuilder::with_id("main")
                .icon(tray_icon_img)
                .icon_as_template(true)
                .tooltip("DM Voice")
                .menu(&tray_menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, event| {
                    let id = event.id().as_ref().to_string();
                    if id == "quit" {
                        app.exit(0);
                    } else if id == "settings" {
                        show_settings_window(app);
                    } else if id == "autostart" {
                        autostart_toggle(app);
                        rebuild_tray_menu(app, &state_for_menu);
                    } else if id == "update_check" {
                        let app2 = app.clone();
                        let state2 = Arc::clone(&state_for_menu);
                        tauri::async_runtime::spawn(async move {
                            let _ = updater::run_check(
                                app2.clone(),
                                Arc::clone(&state2.update),
                            )
                            .await;
                            rebuild_tray_menu(&app2, &state2);
                        });
                    } else if id == "update_install" {
                        let app2 = app.clone();
                        let state2 = Arc::clone(&state_for_menu);
                        tauri::async_runtime::spawn(async move {
                            let updater_inst = match app2.updater() {
                                Ok(u) => u,
                                Err(e) => {
                                    dlog!("[updater] menu: {}", e);
                                    return;
                                }
                            };
                            let update = match updater_inst.check().await {
                                Ok(Some(u)) => u,
                                Ok(None) => {
                                    dlog!("[updater] menu: no update");
                                    return;
                                }
                                Err(e) => {
                                    dlog!("[updater] menu check err: {}", e);
                                    return;
                                }
                            };
                            {
                                let mut s = state2.update.lock().unwrap();
                                s.installing = true;
                            }
                            rebuild_tray_menu(&app2, &state2);
                            let downloaded = Arc::new(Mutex::new(0u64));
                            let dl_chunk = Arc::clone(&downloaded);
                            let app3 = app2.clone();
                            let result = update
                                .download_and_install(
                                    move |chunk_length, content_length| {
                                        let mut d = dl_chunk.lock().unwrap();
                                        *d += chunk_length as u64;
                                        let _ = app3.emit(
                                            "update-progress",
                                            serde_json::json!({
                                                "downloaded": *d,
                                                "total": content_length,
                                            }),
                                        );
                                    },
                                    || {
                                        dlog!("[updater] menu: download finished");
                                    },
                                )
                                .await;
                            {
                                let mut s = state2.update.lock().unwrap();
                                s.installing = false;
                            }
                            match result {
                                Ok(()) => app2.restart(),
                                Err(e) => dlog!("[updater] menu install err: {}", e),
                            }
                        });
                    } else if let Some(rest) = id.strip_prefix("mic:") {
                        let chosen: Option<String> = if rest == "default" {
                            None
                        } else {
                            Some(rest.to_string())
                        };
                        {
                            let mut cfg = state_for_menu.config.lock().unwrap();
                            cfg.input_device = chosen.clone();
                            let _ = save_config(&cfg);
                        }
                        dlog!("tray: input device set to {:?}", chosen);
                        rebuild_tray_menu(app, &state_for_menu);
                    } else if let Some(model_name) = id.strip_prefix("model:") {
                        let app2 = app.clone();
                        let state2 = Arc::clone(&state_for_menu);
                        let name = model_name.to_string();
                        tauri::async_runtime::spawn(async move {
                            // Switch model on a worker thread — Whisper init can take
                            // several hundred ms and we don't want to block the menu.
                            let info = models::list_models()
                                .into_iter()
                                .find(|m| m.name == name && m.installed);
                            if let Some(info) = info {
                                {
                                    let mut cfg = state2.config.lock().unwrap();
                                    cfg.model_name = name.clone();
                                    let _ = save_config(&cfg);
                                }
                                let path = models::model_path(&info.filename);
                                if let Ok(t) = transcriber::WhisperTranscriber::new(&path) {
                                    *state2.transcriber.lock().unwrap() = Some(t);
                                    dlog!("tray: active model switched to {}", name);
                                }
                                rebuild_tray_menu(&app2, &state2);
                            }
                        });
                    }
                })
                .build(app)?;

            // Note: we intentionally do not auto-refresh the tray menu for
            // newly-connected input devices. Tauri 2 has no "menu will open"
            // hook on macOS, and rebuilding the menu from a background thread
            // calls `set_menu()` which dismisses the in-flight NSMenu if the
            // user happens to be opening it at that moment — observed as the
            // menu flickering shut on first launch. The device list is rebuilt
            // every time the menu is reconstructed for another reason (mic /
            // model / autostart toggle, update check), which covers the
            // typical workflow. If a device is hot-plugged with no other
            // interaction, the user sees it after the next menu action.

            // Register global shortcut
            let shortcut = state.config.lock().unwrap().shortcut.clone();
            register_shortcut(app.handle(), &shortcut, Arc::clone(&state));

            // Background update check ~30s after start. When an update is
            // found, the tray menu rebuild adds the "Install update" item and
            // a notification fires.
            let app_for_update = app.handle().clone();
            let state_for_update = Arc::clone(&state);
            let update_state_for_check = Arc::clone(&state.update);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let snap = updater::run_check(
                    app_for_update.clone(),
                    update_state_for_check,
                )
                .await;
                if snap.update_available() {
                    rebuild_tray_menu(&app_for_update, &state_for_update);
                    use tauri_plugin_notification::NotificationExt;
                    let v = snap.latest_version.unwrap_or_default();
                    let _ = app_for_update
                        .notification()
                        .builder()
                        .title("DM Voice — Update available")
                        .body(format!("Version {} is ready to install.", v))
                        .show();
                }
            });

            // Auto-download default model if not installed
            let default_model = models::MODELS
                .iter()
                .find(|(name, _, _, _)| *name == "large-v3-turbo")
                .unwrap();
            if !models::model_path(default_model.1).exists() {
                let app_handle2 = app.handle().clone();
                let app_handle3 = app.handle().clone();
                let filename = default_model.1.to_string();
                let state2 = Arc::clone(&state);
                tauri::async_runtime::spawn(async move {
                    let name = "large-v3-turbo".to_string();
                    let _ = models::download_model(&filename, move |p| {
                        let _ = app_handle2.emit(
                            "model-download-progress",
                            serde_json::json!({"name": name, "progress": p}),
                        );
                    })
                    .await;
                    let path = models::model_path(&filename);
                    if let Ok(t) = transcriber::WhisperTranscriber::new(&path) {
                        *state2.transcriber.lock().unwrap() = Some(t);
                    }
                    rebuild_tray_menu(&app_handle3, &state2);
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
