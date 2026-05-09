use anyhow::Result;
use arboard::Clipboard;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::thread;
use std::time::Duration;

pub fn inject_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let mut clipboard = Clipboard::new()?;
    let previous = clipboard.get_text().unwrap_or_default();
    clipboard.set_text(text)?;
    thread::sleep(Duration::from_millis(50));
    let mut enigo = Enigo::new(&Settings::default())?;
    #[cfg(target_os = "macos")]
    {
        enigo.key(Key::Meta, enigo::Direction::Press)?;
        enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
        enigo.key(Key::Meta, enigo::Direction::Release)?;
    }
    #[cfg(target_os = "windows")]
    {
        enigo.key(Key::Control, enigo::Direction::Press)?;
        enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
        enigo.key(Key::Control, enigo::Direction::Release)?;
    }
    thread::sleep(Duration::from_millis(100));
    if !previous.is_empty() {
        let _ = clipboard.set_text(&previous);
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
        let result = inject_text("");
        assert!(result.is_ok());
    }
}
