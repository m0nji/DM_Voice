use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TypingSpeedPreset {
    Beginner, // 24 WPM = 120 CPM
    Average,  // 40 WPM = 200 CPM
    Fast,     // 60 WPM = 300 CPM
}

impl Default for TypingSpeedPreset {
    fn default() -> Self {
        Self::Average
    }
}

impl TypingSpeedPreset {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "beginner" => Some(Self::Beginner),
            "average" => Some(Self::Average),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WakeWordSensitivity {
    Low,
    Medium,
    High,
}

impl Default for WakeWordSensitivity {
    fn default() -> Self {
        Self::Medium
    }
}

impl WakeWordSensitivity {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// Detection threshold fed to the wake-word model. Lower threshold = more
    /// sensitive = "High". Raised from the initial Phase 0 values after live-mic
    /// testing showed too many false triggers on ambient noise at 0.5 — real
    /// "Hey Jarvis" hits average ~0.99, so these leave ample margin.
    pub fn threshold(self) -> f32 {
        match self {
            Self::Low => 0.85,
            Self::Medium => 0.7,
            Self::High => 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub shortcut: String,
    pub model_name: String,
    #[serde(default = "default_sounds_enabled")]
    pub sounds_enabled: bool,
    #[serde(default)]
    pub typing_speed_preset: TypingSpeedPreset,
    #[serde(default)]
    pub custom_vocabulary: Vec<String>,
    /// Preferred input device name (as reported by cpal). `None` means
    /// "follow the system default". If the saved name is no longer available
    /// at capture time, capture falls back to the system default but the
    /// preference is kept so the device gets picked up automatically when it
    /// reappears.
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default)]
    pub wake_word_enabled: bool,
    #[serde(default = "default_wake_word_model")]
    pub wake_word_model: String,
    #[serde(default)]
    pub wake_word_sensitivity: WakeWordSensitivity,
    #[serde(default = "default_silence_timeout_ms")]
    pub silence_timeout_ms: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut().to_string(),
            model_name: "large-v3-turbo".to_string(),
            sounds_enabled: default_sounds_enabled(),
            typing_speed_preset: TypingSpeedPreset::default(),
            custom_vocabulary: Vec::new(),
            input_device: None,
            wake_word_enabled: false,
            wake_word_model: default_wake_word_model(),
            wake_word_sensitivity: WakeWordSensitivity::default(),
            silence_timeout_ms: default_silence_timeout_ms(),
        }
    }
}

/// Build the initial-prompt string fed to Whisper for vocabulary biasing.
///
/// Whisper's `initial_prompt` is capped at ~224 tokens; going over silently
/// truncates from the FRONT, dropping the user's terms. We conservatively cap
/// at ~600 characters (Whisper's BPE tokenizer averages ~3–4 chars/token across
/// German/English), so even worst-case single-char tokens (e.g. CJK) stay
/// within budget.
///
/// Returns `None` when the resulting prompt would be empty.
pub fn build_vocabulary_prompt(words: &[String]) -> Option<String> {
    const MAX_PROMPT_CHARS: usize = 600;
    let mut buf = String::new();
    for raw in words {
        let term = raw.trim();
        if term.is_empty() || term.contains('\0') {
            continue;
        }
        let sep = if buf.is_empty() { "" } else { ", " };
        if buf.len() + sep.len() + term.len() > MAX_PROMPT_CHARS {
            break;
        }
        buf.push_str(sep);
        buf.push_str(term);
    }
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

fn default_sounds_enabled() -> bool {
    true
}

fn default_wake_word_model() -> String {
    "hey_jarvis".to_string()
}

/// Clamp range is enforced in the setter command (1000..=8000).
fn default_silence_timeout_ms() -> u32 {
    2000
}

// Plattform-spezifischer Default-Hotkey:
//   macOS  → Alt+Space (Original-Default, im OS frei).
//   Windows → Ctrl+Space (Alt+Space ist dort System-Shortcut für Fenstermenü).
fn default_shortcut() -> &'static str {
    #[cfg(target_os = "macos")]
    { "Alt+Space" }
    #[cfg(not(target_os = "macos"))]
    { "Ctrl+Space" }
}

pub fn config_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("DM-Voice").join("config.toml")
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let contents = toml::to_string(config)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn with_temp_config<F: FnOnce(PathBuf)>(f: F) {
        let dir = TempDir::new().unwrap();
        f(dir.path().join("config.toml"));
    }

    #[test]
    fn roundtrip_saves_and_loads() {
        with_temp_config(|path| {
            let cfg = AppConfig {
                shortcut: "Ctrl+D".to_string(),
                model_name: "small".to_string(),
                sounds_enabled: false,
                typing_speed_preset: TypingSpeedPreset::Fast,
                custom_vocabulary: vec!["Tauri".into(), "whisper-rs".into()],
                input_device: Some("MacBook Pro Microphone".into()),
                wake_word_enabled: true,
                wake_word_model: "alexa".into(),
                wake_word_sensitivity: WakeWordSensitivity::High,
                silence_timeout_ms: 3500,
            };
            let contents = toml::to_string(&cfg).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            let loaded: AppConfig = toml::from_str(
                &std::fs::read_to_string(&path).unwrap()
            ).unwrap();
            assert_eq!(loaded, cfg);
        });
    }

    #[test]
    fn load_returns_default_when_missing() {
        let result: AppConfig = toml::from_str("").unwrap_or_default();
        assert_eq!(result.shortcut, AppConfig::default().shortcut);
    }

    #[test]
    fn build_vocab_prompt_empty_list_returns_none() {
        assert!(build_vocabulary_prompt(&[]).is_none());
    }

    #[test]
    fn build_vocab_prompt_skips_blanks_and_nulls() {
        let words = vec![
            "  ".into(),
            "Tauri".into(),
            "".into(),
            "with\0null".into(),
            "whisper-rs".into(),
        ];
        assert_eq!(
            build_vocabulary_prompt(&words),
            Some("Tauri, whisper-rs".into())
        );
    }

    #[test]
    fn build_vocab_prompt_trims_terms() {
        let words = vec!["  Hello  ".into(), "\tWorld\n".into()];
        assert_eq!(
            build_vocabulary_prompt(&words),
            Some("Hello, World".into())
        );
    }

    #[test]
    fn wake_word_defaults_are_off() {
        let cfg = AppConfig::default();
        assert!(!cfg.wake_word_enabled);
        assert_eq!(cfg.wake_word_model, "hey_jarvis");
        assert_eq!(cfg.wake_word_sensitivity, WakeWordSensitivity::Medium);
        assert_eq!(cfg.silence_timeout_ms, 2000);
    }

    #[test]
    fn missing_wake_word_fields_deserialize_to_defaults() {
        // An old config.toml without the new keys must still load.
        let old = "shortcut = \"Alt+Space\"\nmodel_name = \"large-v3-turbo\"\n";
        let cfg: AppConfig = toml::from_str(old).unwrap();
        assert!(!cfg.wake_word_enabled);
        assert_eq!(cfg.silence_timeout_ms, 2000);
    }

    #[test]
    fn build_vocab_prompt_caps_at_max_chars() {
        // 100 words × ~10 chars ≈ 1100 chars; cap is 600 → should truncate.
        let words: Vec<String> = (0..100).map(|i| format!("word{:04}", i)).collect();
        let prompt = build_vocabulary_prompt(&words).unwrap();
        assert!(prompt.len() <= 600, "prompt was {} chars", prompt.len());
        assert!(prompt.starts_with("word0000"));
    }
}
