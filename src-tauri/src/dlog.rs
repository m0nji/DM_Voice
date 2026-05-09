//! Tiny file-backed logger. Writes to `~/Library/Logs/dm-voice.log`,
//! rotating the previous run's log to `dm-voice.log.old` at startup.
//!
//! Used during debugging so that diagnostics survive even when the app is
//! launched outside a terminal (stderr → /dev/null in that case).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<File>> = Mutex::new(None);

fn log_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join("Library/Logs/dm-voice.log")
}

pub fn init() {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let old = path.with_extension("log.old");
    let _ = std::fs::rename(&path, &old);
    let f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    *LOG_FILE.lock().unwrap() = f;
    log(&format!("=== dm-voice log opened: {} ===", path.display()));
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
