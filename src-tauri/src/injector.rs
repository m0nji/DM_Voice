use anyhow::Result;
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Inject text into the focused field via clipboard + Cmd+V.
///
/// macOS: copies to clipboard, then sends Cmd+V using raw CGEvents.
/// Raw keycode 9 (physical V) bypasses TSM entirely — no main-thread
/// constraint, no Accessibility child-process issues.
pub fn inject_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    copy_to_clipboard(text)?;
    thread::sleep(Duration::from_millis(100));
    paste_via_cgevent()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn paste_via_cgevent() -> Result<()> {
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: *const c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut c_void);
        fn CFRelease(cf: *const c_void);
    }

    // kCGHIDEventTap = 0
    const HID_TAP: u32 = 0;
    // Physical keycode 9 = V on ANSI keyboard (layout-independent, no TSM needed)
    const V_KEY: u16 = 9;
    // kCGEventFlagMaskCommand = 1 << 20
    const CMD: u64 = 1 << 20;

    unsafe {
        // Cmd+V down
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), V_KEY, true);
        if down.is_null() {
            anyhow::bail!("CGEventCreateKeyboardEvent failed");
        }
        CGEventSetFlags(down, CMD);
        CGEventPost(HID_TAP, down);
        CFRelease(down);

        thread::sleep(Duration::from_millis(20));

        // V up (no modifiers)
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), V_KEY, false);
        if !up.is_null() {
            CGEventSetFlags(up, 0);
            CGEventPost(HID_TAP, up);
            CFRelease(up);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn paste_via_cgevent() -> Result<()> {
    use enigo::{Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Control, enigo::Direction::Press)?;
    enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
    enigo.key(Key::Control, enigo::Direction::Release)?;
    thread::sleep(Duration::from_millis(100));
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
