//! Tiny file-backed logger. Writes to `~/Library/Logs/dm-voice.log`,
//! rotating the previous runs' logs to `dm-voice.log.1` .. `dm-voice.log.5`
//! at startup (oldest dropped). The legacy `.old` name is still cleaned up
//! on first launch after upgrade.
//!
//! Used during debugging so that diagnostics survive even when the app is
//! launched outside a terminal (stderr → /dev/null in that case).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

const ROTATION_KEEP: usize = 5;

fn log_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Logs/dm-voice.log")
}

fn rotate(path: &PathBuf) {
    // Drop oldest, then shift .N → .N+1 down to .1, then current → .1.
    let nth = |n: usize| path.with_extension(format!("log.{}", n));
    let _ = std::fs::remove_file(nth(ROTATION_KEEP));
    for n in (1..ROTATION_KEEP).rev() {
        let _ = std::fs::rename(nth(n), nth(n + 1));
    }
    let _ = std::fs::rename(path, nth(1));
    // Legacy single-slot rotation from older versions; harmless if absent.
    let _ = std::fs::remove_file(path.with_extension("log.old"));
}

pub fn init() {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    rotate(&path);
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    *LOG_FILE.lock().unwrap() = f;
    log(&format!("=== dm-voice log opened: {} ===", path.display()));
    log(&format!("dm-voice version {}", env!("CARGO_PKG_VERSION")));
}

fn ts() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let ms = d.subsec_millis();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

pub fn log(msg: &str) {
    let line = format!("[{}] {}\n", ts(), msg);
    eprint!("{}", line);
    if let Ok(mut guard) = LOG_FILE.lock() {
        if let Some(ref mut f) = *guard {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {{
        $crate::dlog::log(&format!($($arg)*));
    }};
}
