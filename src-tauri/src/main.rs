#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod injector;
mod models;
mod shortcut;
mod transcriber;

use audio::AudioCapture;
use config::{load_config, save_config, AppConfig};
use models::ModelInfo;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
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
}

type SharedState = Arc<AppState>;

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
fn list_models() -> Vec<ModelInfo> {
    models::list_models()
}

#[tauri::command]
fn delete_model(filename: String) -> Result<(), String> {
    models::delete_model(&filename).map_err(|e| e.to_string())
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

fn trigger_transcription(app: AppHandle, state: SharedState) {
    tokio::spawn(async move {
        let buffer = {
            let mut audio = state.audio.lock().unwrap();
            audio.stop_and_get_buffer().unwrap_or_default()
        };
        show_overlay(&app, "processing");
        let text = {
            let t = state.transcriber.lock().unwrap();
            t.as_ref()
                .and_then(|t| t.transcribe(&buffer).ok())
                .unwrap_or_default()
        };
        if !text.is_empty() {
            if injector::inject_text(&text).is_err() {
                let _ = injector::copy_to_clipboard(&text);
                use tauri_plugin_notification::NotificationExt;
                let _ = app
                    .notification()
                    .builder()
                    .title("DM Voice")
                    .body("Kein Textfeld aktiv — Text kopiert")
                    .show();
            }
        }
        show_overlay(&app, "done");
        sleep(Duration::from_millis(400)).await;
        hide_overlay(&app);
    });
}

fn register_shortcut(app: &AppHandle, shortcut: &str, state: SharedState) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    let app_clone = app.clone();
    let _ = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_, _, event| match event.state() {
            ShortcutState::Pressed => {
                if state.recording_start.lock().unwrap().is_some() {
                    return;
                }
                let mut audio = state.audio.lock().unwrap();
                if audio.start().is_err() {
                    return;
                }
                *state.recording_start.lock().unwrap() = Some(Instant::now());
                *state.auto_stop.lock().unwrap() = false;
                show_overlay(&app_clone, "recording");
                drop(audio);

                let app2 = app_clone.clone();
                let state2 = Arc::clone(&state);
                tokio::spawn(async move {
                    loop {
                        sleep(Duration::from_millis(50)).await;
                        let elapsed = {
                            let start = state2.recording_start.lock().unwrap();
                            start.map(|s| s.elapsed())
                        };
                        match elapsed {
                            None => break,
                            Some(d) if d > Duration::from_secs(30) => {
                                *state2.recording_start.lock().unwrap() = None;
                                *state2.auto_stop.lock().unwrap() = true;
                                trigger_transcription(app2.clone(), Arc::clone(&state2));
                                break;
                            }
                            _ => {}
                        }
                        let amp = state2.audio.lock().unwrap().current_amplitude();
                        let _ = app2.emit("amplitude", amp);
                    }
                });
            }
            ShortcutState::Released => {
                let start = state.recording_start.lock().unwrap().take();
                if *state.auto_stop.lock().unwrap() {
                    return;
                }
                let elapsed = start.map(|s| s.elapsed()).unwrap_or_default();
                if elapsed < Duration::from_millis(300) {
                    let mut audio = state.audio.lock().unwrap();
                    let _ = audio.stop_and_get_buffer();
                    drop(audio);
                    hide_overlay(&app_clone);
                    return;
                }
                trigger_transcription(app_clone.clone(), Arc::clone(&state));
            }
        });
}

fn main() {
    let cfg = load_config();
    let state: SharedState = Arc::new(AppState {
        audio: Mutex::new(AudioGuard::new()),
        recording_start: Mutex::new(None),
        auto_stop: Mutex::new(false),
        config: Mutex::new(cfg.clone()),
        transcriber: Mutex::new(None),
    });

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .manage(Arc::clone(&state))
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_shortcut,
            list_models,
            delete_model,
            download_model,
        ])
        .setup(move |app| {
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
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::{TrayIconBuilder, TrayIconEvent};
            let app_handle = app.handle().clone();

            let quit_item =
                MenuItem::with_id(app, "quit", "DM Voice beenden", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&tray_menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    if event.id() == "quit" {
                        app.exit(0);
                    }
                })
                .on_tray_icon_event(move |_, event| {
                    if let TrayIconEvent::Click { .. } = event {
                        if let Some(w) = app_handle.get_webview_window("settings") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        } else {
                            let _ = tauri::WebviewWindowBuilder::new(
                                &app_handle,
                                "settings",
                                tauri::WebviewUrl::App("settings/index.html".into()),
                            )
                            .title("DM Voice")
                            .inner_size(300.0, 420.0)
                            .resizable(false)
                            .build();
                        }
                    }
                })
                .build(app)?;

            // Register global shortcut
            let shortcut = state.config.lock().unwrap().shortcut.clone();
            register_shortcut(app.handle(), &shortcut, Arc::clone(&state));

            // Auto-download default model if not installed
            let default_model = models::MODELS
                .iter()
                .find(|(name, _, _, _)| *name == "large-v3-turbo")
                .unwrap();
            if !models::model_path(default_model.1).exists() {
                let app_handle2 = app.handle().clone();
                let filename = default_model.1.to_string();
                let state2 = Arc::clone(&state);
                tokio::spawn(async move {
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
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
