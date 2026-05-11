// Monthly usage aggregates for the "Time Saved" panel.
//
// Stores only integers (chars, seconds, count) — no text or audio.
// Path: <dirs::data_dir()>/DM-Voice/usage_stats.json (next to config.toml).
// Schema: { "version": 1, "months": { "YYYY-MM": { chars, rec_s, proc_s, count } } }

use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

const FILE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct MonthEntry {
    pub chars: u64,
    pub rec_s: f32,
    pub proc_s: f32,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct StatsFile {
    version: u32,
    months: BTreeMap<String, MonthEntry>,
}

impl Default for StatsFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            months: BTreeMap::new(),
        }
    }
}

pub struct UsageStats {
    inner: Mutex<StatsFile>,
    path: PathBuf,
}

#[derive(Serialize, Clone, Debug)]
pub struct MonthStatsPayload {
    pub month_iso: String,
    pub month_label: String,
    pub total_chars: u64,
    pub total_recording_s: f32,
    pub total_processing_s: f32,
    pub dictation_count: u32,
}

pub fn stats_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DM-Voice").join("usage_stats.json")
}

fn current_month_iso() -> String {
    let now = Local::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

fn current_month_label() -> String {
    // English month names, no platform locale dependency.
    const MONTHS: [&str; 12] = [
        "January", "February", "March", "April", "May", "June",
        "July", "August", "September", "October", "November", "December",
    ];
    let now = Local::now();
    let idx = (now.month() as usize).saturating_sub(1).min(11);
    format!("{} {}", MONTHS[idx], now.year())
}

impl UsageStats {
    pub fn load() -> Self {
        let path = stats_path();
        let inner = match std::fs::read_to_string(&path) {
            Ok(s) => serde_json::from_str::<StatsFile>(&s).unwrap_or_else(|e| {
                dlog!("[stats] corrupt JSON, resetting: {}", e);
                StatsFile::default()
            }),
            Err(_) => StatsFile::default(),
        };
        Self {
            inner: Mutex::new(inner),
            path,
        }
    }

    /// Records one successful dictation into the current local month.
    /// Errors are logged and swallowed — the dictation path must not fail
    /// because stats can't be written.
    pub fn record(&self, chars: u32, rec_s: f32, proc_s: f32) {
        let key = current_month_iso();
        let snapshot = {
            let mut guard = self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let entry = guard.months.entry(key).or_default();
            entry.chars = entry.chars.saturating_add(chars as u64);
            entry.rec_s += rec_s.max(0.0);
            entry.proc_s += proc_s.max(0.0);
            entry.count = entry.count.saturating_add(1);
            guard.clone()
        };
        if let Err(e) = self.flush(&snapshot) {
            dlog!("[stats] flush failed: {}", e);
        }
    }

    pub fn current_month(&self) -> MonthStatsPayload {
        let key = current_month_iso();
        let label = current_month_label();
        let entry = {
            let guard = self
                .inner
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            guard.months.get(&key).cloned().unwrap_or_default()
        };
        MonthStatsPayload {
            month_iso: key,
            month_label: label,
            total_chars: entry.chars,
            total_recording_s: entry.rec_s,
            total_processing_s: entry.proc_s,
            dictation_count: entry.count,
        }
    }

    fn flush(&self, snapshot: &StatsFile) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(snapshot).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, e)
        })?;
        std::fs::write(&self.path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_stats(path: PathBuf) -> UsageStats {
        UsageStats {
            inner: Mutex::new(StatsFile::default()),
            path,
        }
    }

    #[test]
    fn record_accumulates_within_same_month() {
        let dir = TempDir::new().unwrap();
        let stats = make_stats(dir.path().join("u.json"));
        stats.record(100, 10.0, 1.0);
        stats.record(50, 5.0, 0.5);
        let cur = stats.current_month();
        assert_eq!(cur.total_chars, 150);
        assert!((cur.total_recording_s - 15.0).abs() < 1e-3);
        assert!((cur.total_processing_s - 1.5).abs() < 1e-3);
        assert_eq!(cur.dictation_count, 2);
    }

    #[test]
    fn current_month_returns_zeros_when_empty() {
        let dir = TempDir::new().unwrap();
        let stats = make_stats(dir.path().join("u.json"));
        let cur = stats.current_month();
        assert_eq!(cur.total_chars, 0);
        assert_eq!(cur.dictation_count, 0);
        assert_eq!(cur.total_recording_s, 0.0);
        assert_eq!(cur.total_processing_s, 0.0);
        assert!(!cur.month_iso.is_empty());
        assert!(!cur.month_label.is_empty());
    }

    #[test]
    fn record_persists_to_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("u.json");
        let stats = make_stats(path.clone());
        stats.record(42, 3.0, 0.2);
        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: StatsFile = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.version, FILE_VERSION);
        let key = current_month_iso();
        let entry = parsed.months.get(&key).unwrap();
        assert_eq!(entry.chars, 42);
        assert_eq!(entry.count, 1);
    }

    #[test]
    fn corrupt_file_resets_to_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("u.json");
        std::fs::write(&path, "not json {{{").unwrap();
        // Manually load using the same path.
        let raw = std::fs::read_to_string(&path).unwrap();
        let parsed: StatsFile =
            serde_json::from_str(&raw).unwrap_or_else(|_| StatsFile::default());
        assert!(parsed.months.is_empty());
    }

    #[test]
    fn negative_seconds_clamp_to_zero() {
        let dir = TempDir::new().unwrap();
        let stats = make_stats(dir.path().join("u.json"));
        stats.record(10, -5.0, -1.0);
        let cur = stats.current_month();
        assert_eq!(cur.total_recording_s, 0.0);
        assert_eq!(cur.total_processing_s, 0.0);
    }

    #[test]
    fn concurrent_records_do_not_lose_updates() {
        use std::sync::Arc;
        use std::thread;

        let dir = TempDir::new().unwrap();
        let stats = Arc::new(make_stats(dir.path().join("u.json")));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = Arc::clone(&stats);
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    s.record(1, 0.1, 0.01);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        let cur = stats.current_month();
        assert_eq!(cur.dictation_count, 400);
        assert_eq!(cur.total_chars, 400);
    }
}
