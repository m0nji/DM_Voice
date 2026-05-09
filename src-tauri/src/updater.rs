use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

/// Cached info about the most recent update check. Lives in `AppState` so the
/// tray menu and the settings UI can both read it without re-running the
/// network call. `latest_version` is set only when a NEWER version than the
/// currently running one is available.
#[derive(Default, Clone, Serialize)]
pub struct UpdateState {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub last_check_unix: Option<u64>,
    pub last_error: Option<String>,
    pub installing: bool,
}

impl UpdateState {
    pub fn new() -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        }
    }

    pub fn update_available(&self) -> bool {
        self.latest_version.is_some()
    }
}

pub type SharedUpdateState = Arc<Mutex<UpdateState>>;

fn now_unix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run a fresh update check. Updates the shared state, emits `update-checked`,
/// and returns the new snapshot. Safe to call from background tasks AND from
/// the UI button — both go through this single path.
pub async fn run_check(app: AppHandle, state: SharedUpdateState) -> UpdateState {
    let result = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };

    let mut s = state.lock().unwrap();
    s.last_check_unix = Some(now_unix());
    match result {
        Ok(Some(update)) => {
            s.latest_version = Some(update.version.clone());
            s.notes = update.body.clone();
            s.last_error = None;
            dlog!("[updater] update available: {}", update.version);
        }
        Ok(None) => {
            s.latest_version = None;
            s.notes = None;
            s.last_error = None;
            dlog!("[updater] no update available");
        }
        Err(e) => {
            s.last_error = Some(e.to_string());
            dlog!("[updater] check failed: {}", e);
        }
    }
    let snapshot = s.clone();
    drop(s);
    let _ = app.emit("update-checked", &snapshot);
    snapshot
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, SharedUpdateState>,
) -> Result<UpdateState, String> {
    let shared = Arc::clone(&state);
    Ok(run_check(app, shared).await)
}

#[tauri::command]
pub fn get_update_state(state: State<'_, SharedUpdateState>) -> UpdateState {
    state.lock().unwrap().clone()
}

/// Download + install the most recent update, then restart. Re-runs the
/// `check()` because `Update::download_and_install` consumes the value and we
/// don't keep it around in state. Emits `update-progress` events with
/// `{downloaded, total}` so the UI can render a bar.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, SharedUpdateState>,
) -> Result<(), String> {
    {
        let mut s = state.lock().unwrap();
        if s.installing {
            return Err("Update läuft bereits".into());
        }
        s.installing = true;
    }

    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Kein Update verfügbar".to_string())?;

    let app_for_progress = app.clone();
    let downloaded = Arc::new(Mutex::new(0u64));
    let downloaded_for_chunk = Arc::clone(&downloaded);

    let result = update
        .download_and_install(
            move |chunk_length, content_length| {
                let mut d = downloaded_for_chunk.lock().unwrap();
                *d += chunk_length as u64;
                let _ = app_for_progress.emit(
                    "update-progress",
                    serde_json::json!({
                        "downloaded": *d,
                        "total": content_length,
                    }),
                );
            },
            move || {
                dlog!("[updater] download finished, installing");
            },
        )
        .await;

    {
        let mut s = state.lock().unwrap();
        s.installing = false;
    }

    match result {
        Ok(()) => {
            dlog!("[updater] install ok, restarting");
            app.restart();
        }
        Err(e) => Err(e.to_string()),
    }
}

