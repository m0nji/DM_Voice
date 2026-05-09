use anyhow::Result;
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Inject text into the focused field.
///
/// macOS: copies to clipboard then simulates Cmd+V via AppleScript
/// (osascript). This is the most reliable approach — no threading
/// constraints, no Accessibility permission edge cases with enigo.
pub fn inject_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    copy_to_clipboard(text)?;
    thread::sleep(Duration::from_millis(50));

    #[cfg(target_os = "macos")]
    {
        let script = r#"tell application "System Events" to keystroke "v" using command down"#;
        let output = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("osascript failed: {}", err);
        }
    }

    #[cfg(target_os = "windows")]
    {
        use enigo::{Enigo, Key, Keyboard, Settings};
        let mut enigo = Enigo::new(&Settings::default())?;
        enigo.key(Key::Control, enigo::Direction::Press)?;
        enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
        enigo.key(Key::Control, enigo::Direction::Release)?;
        thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new()?;
    clipboard.set_text(text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_empty_text_is_noop() {
        assert!(inject_text("").is_ok());
    }
}
