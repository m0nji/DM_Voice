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

/// One spoken-word → symbol mapping. Matched as a whole utterance only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolReplacement {
    pub spoken: String,
    pub symbol: String,
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
    /// When true, the overlay pill stays visible at all times (in a dimmed
    /// "Ready" idle state) and can be dragged to a custom position. When false
    /// (default), the overlay only appears during recording at bottom-center.
    #[serde(default)]
    pub pill_always_visible: bool,
    /// Custom physical screen position of the pinned pill (top-left of the
    /// overlay window). `None` means "not placed yet" → falls back to the
    /// default bottom-center position. Only honored while `pill_always_visible`.
    #[serde(default)]
    pub pill_position: Option<(i32, i32)>,
    /// When true, all dictated text is lowercased before injection. Default
    /// false keeps Whisper's original capitalization.
    #[serde(default)]
    pub lowercase_output: bool,
    /// Master switch for spoken-symbol replacement. Default on.
    #[serde(default = "default_symbol_replacements_enabled")]
    pub symbol_replacements_enabled: bool,
    /// Spoken-word → symbol table. Replaces the whole utterance on exact match.
    #[serde(default = "default_symbol_replacements")]
    pub symbol_replacements: Vec<SymbolReplacement>,
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
            pill_always_visible: false,
            pill_position: None,
            lowercase_output: false,
            symbol_replacements_enabled: default_symbol_replacements_enabled(),
            symbol_replacements: default_symbol_replacements(),
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

/// Applies the chosen output casing to transcribed text. `lowercase == true`
/// returns the text fully lowercased (Rust's `to_lowercase` handles German
/// umlauts and ß correctly: Ä→ä, Ö→ö, Ü→ü, ß stays ß). Otherwise the text is
/// returned unchanged.
pub fn apply_output_casing(text: &str, lowercase: bool) -> String {
    if lowercase {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

pub fn default_symbol_replacements_enabled() -> bool {
    true
}

pub fn default_symbol_replacements() -> Vec<SymbolReplacement> {
    [
        ("pipe", "|"),
        ("backslash", "\\"),
        ("ampersand", "&"),
        ("tilde", "~"),
        ("caret", "^"),
        ("backtick", "`"),
        ("at", "@"),
        ("hash", "#"),
    ]
    .iter()
    .map(|(spoken, symbol)| SymbolReplacement {
        spoken: spoken.to_string(),
        symbol: symbol.to_string(),
    })
    .collect()
}

/// Builds the word list fed to `build_vocabulary_prompt`: the user's custom
/// vocabulary first, then the active spoken-symbol terms (when enabled) so
/// Whisper recognizes short symbol names more reliably. The 600-char cap in
/// `build_vocabulary_prompt` still applies, keeping user terms prioritized.
pub fn combined_vocabulary(
    custom: &[String],
    replacements: &[SymbolReplacement],
    symbols_enabled: bool,
) -> Vec<String> {
    let mut words: Vec<String> = custom.to_vec();
    if symbols_enabled {
        for r in replacements {
            let term = r.spoken.trim();
            if !term.is_empty() {
                words.push(term.to_string());
            }
        }
    }
    words
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
                pill_always_visible: true,
                pill_position: Some((120, 880)),
                lowercase_output: true,
                symbol_replacements_enabled: false,
                symbol_replacements: vec![SymbolReplacement {
                    spoken: "pipe".into(),
                    symbol: "|".into(),
                }],
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
        assert!(!cfg.pill_always_visible);
        assert_eq!(cfg.pill_position, None);
        assert!(!cfg.lowercase_output);
    }

    #[test]
    fn apply_output_casing_off_is_unchanged() {
        assert_eq!(apply_output_casing("Hallo Welt", false), "Hallo Welt");
    }

    #[test]
    fn apply_output_casing_on_lowercases() {
        assert_eq!(apply_output_casing("Hallo Welt", true), "hallo welt");
    }

    #[test]
    fn apply_output_casing_handles_umlauts_and_eszett() {
        assert_eq!(apply_output_casing("Ärger Über Öl", true), "ärger über öl");
        assert_eq!(apply_output_casing("STRAßE", true), "straße");
    }

    #[test]
    fn build_vocab_prompt_caps_at_max_chars() {
        // 100 words × ~10 chars ≈ 1100 chars; cap is 600 → should truncate.
        let words: Vec<String> = (0..100).map(|i| format!("word{:04}", i)).collect();
        let prompt = build_vocabulary_prompt(&words).unwrap();
        assert!(prompt.len() <= 600, "prompt was {} chars", prompt.len());
        assert!(prompt.starts_with("word0000"));
    }

    #[test]
    fn symbol_defaults_are_on_with_prefilled_list() {
        let cfg = AppConfig::default();
        assert!(cfg.symbol_replacements_enabled);
        let pipe = cfg
            .symbol_replacements
            .iter()
            .find(|r| r.spoken == "pipe")
            .expect("default list must contain 'pipe'");
        assert_eq!(pipe.symbol, "|");
        assert!(cfg.symbol_replacements.iter().any(|r| r.spoken == "backslash" && r.symbol == "\\"));
    }

    #[test]
    fn missing_symbol_fields_deserialize_to_defaults() {
        // An old config.toml without the new keys must still load with defaults.
        let old = "shortcut = \"Alt+Space\"\nmodel_name = \"large-v3-turbo\"\n";
        let cfg: AppConfig = toml::from_str(old).unwrap();
        assert!(cfg.symbol_replacements_enabled);
        assert!(!cfg.symbol_replacements.is_empty());
    }

    #[test]
    fn combined_vocabulary_appends_spoken_terms_when_enabled() {
        let custom = vec!["Tauri".to_string()];
        let repl = vec![
            SymbolReplacement { spoken: "pipe".into(), symbol: "|".into() },
            SymbolReplacement { spoken: "  ".into(), symbol: "x".into() }, // skipped (blank spoken)
        ];
        assert_eq!(
            combined_vocabulary(&custom, &repl, true),
            vec!["Tauri".to_string(), "pipe".to_string()]
        );
    }

    #[test]
    fn combined_vocabulary_omits_symbols_when_disabled() {
        let custom = vec!["Tauri".to_string()];
        let repl = vec![SymbolReplacement { spoken: "pipe".into(), symbol: "|".into() }];
        assert_eq!(combined_vocabulary(&custom, &repl, false), vec!["Tauri".to_string()]);
    }
}
