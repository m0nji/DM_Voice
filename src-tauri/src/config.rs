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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub shortcut: String,
    pub model_name: String,
    #[serde(default = "default_sounds_enabled")]
    pub sounds_enabled: bool,
    #[serde(default)]
    pub typing_speed_preset: TypingSpeedPreset,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            shortcut: default_shortcut().to_string(),
            model_name: "large-v3-turbo".to_string(),
            sounds_enabled: default_sounds_enabled(),
            typing_speed_preset: TypingSpeedPreset::default(),
        }
    }
}

fn default_sounds_enabled() -> bool {
    true
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
}
