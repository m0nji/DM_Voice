use anyhow::Result;
use arboard::Clipboard;
use std::thread;
use std::time::Duration;

/// Returns the (PID, localizedName) of the frontmost application via NSWorkspace.
#[cfg(target_os = "macos")]
fn frontmost_app_info() -> Option<(i32, String)> {
    use std::ffi::c_void;

    #[link(name = "AppKit", kind = "framework")]
    extern "C" {}

    extern "C" {
        fn objc_getClass(name: *const u8) -> *mut c_void;
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }

    type MsgSendI32 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgPtr = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const i8;

    unsafe {
        let ws_class = objc_getClass(b"NSWorkspace\0".as_ptr());
        if ws_class.is_null() {
            return None;
        }
        let shared_sel = sel_registerName(b"sharedWorkspace\0".as_ptr());
        let shared = objc_msgSend(ws_class, shared_sel);
        if shared.is_null() {
            return None;
        }
        let front_sel = sel_registerName(b"frontmostApplication\0".as_ptr());
        let app = objc_msgSend(shared, front_sel);
        if app.is_null() {
            return None;
        }
        let pid_sel = sel_registerName(b"processIdentifier\0".as_ptr());
        let pid_fn: MsgSendI32 = std::mem::transmute(objc_msgSend as *const ());
        let pid = pid_fn(app, pid_sel);

        let name_sel = sel_registerName(b"localizedName\0".as_ptr());
        let name_fn: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let nsstr = name_fn(app, name_sel);
        let name = if nsstr.is_null() {
            String::new()
        } else {
            let utf8_sel = sel_registerName(b"UTF8String\0".as_ptr());
            let utf8: MsgPtr = std::mem::transmute(objc_msgSend as *const ());
            let cstr = utf8(nsstr, utf8_sel);
            if cstr.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(cstr)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        if pid <= 0 {
            return None;
        }
        Some((pid, name))
    }
}

/// Returns the PID of the frontmost application (macOS: NSWorkspace).
/// Call this BEFORE showing the overlay — showing the window can briefly
/// activate DM Voice and make `NSWorkspace.frontmostApplication` return
/// the wrong app. Returns `None` if it would point to DM Voice itself.
#[cfg(target_os = "macos")]
pub fn frontmost_app_pid() -> Option<i32> {
    let info = frontmost_app_info();
    let my_pid = std::process::id() as i32;
    match info {
        Some((pid, ref name)) => {
            dlog!(
                "frontmost_app_pid: pid={} name={:?} my_pid={}",
                pid,
                name,
                my_pid
            );
            if pid == my_pid {
                dlog!("  -> filtered (DM Voice itself)");
                None
            } else {
                Some(pid)
            }
        }
        None => {
            dlog!("frontmost_app_pid: no frontmost app");
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[cfg(not(target_os = "windows"))]
pub fn frontmost_app_pid() -> Option<i32> {
    None
}

#[cfg(target_os = "windows")]
fn foreground_window_pid() -> Option<i32> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            None
        } else {
            Some(pid as i32)
        }
    }
}

#[cfg(target_os = "windows")]
pub fn frontmost_app_pid() -> Option<i32> {
    let pid = foreground_window_pid()?;
    let my_pid = std::process::id() as i32;
    dlog!("frontmost_app_pid: pid={} my_pid={}", pid, my_pid);
    if pid == my_pid {
        dlog!("  -> filtered (DM Voice itself)");
        None
    } else {
        Some(pid)
    }
}

/// Bring the app with the given PID to the foreground via the Accessibility API.
///
/// Uses `AXUIElementSetAttributeValue(_, kAXFrontmostAttribute, true)` directly,
/// which only requires the Accessibility permission (already granted to DM Voice).
///
/// We deliberately AVOID osascript / AppleScript here: that goes via Apple Events
/// to System Events, which requires the *Automation* TCC permission ("DM Voice
/// wants to control System Events"). When DM Voice is launched from Finder there
/// is no responsible-process inheritance, so the Automation prompt either never
/// appears or is silently denied — and `osascript` returns success 0 even when
/// the Apple Event was rejected. The AX path has no such gotcha.
///
/// Returns true on AX success, false on any failure (logged).
#[cfg(target_os = "macos")]
fn activate_pid(pid: i32) -> bool {
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> *mut c_void;
        fn AXUIElementSetAttributeValue(
            element: *mut c_void,
            attribute: *const c_void,
            value: *const c_void,
        ) -> i32;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
    }

    extern "C" {
        fn objc_getClass(name: *const u8) -> *mut c_void;
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }

    // Build a CFString for "AXFrontmost" via NSString → toll-free bridge.
    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgIdCStr = unsafe extern "C" fn(*mut c_void, *mut c_void, *const i8) -> *mut c_void;

    unsafe {
        let app_el = AXUIElementCreateApplication(pid);
        if app_el.is_null() {
            dlog!(
                "activate_pid({}): AXUIElementCreateApplication returned null",
                pid
            );
            return false;
        }

        let nsstr_cls = objc_getClass(b"NSString\0".as_ptr());
        let with_utf8: MsgIdCStr = std::mem::transmute(objc_msgSend as *const ());
        let alloc: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let raw = alloc(nsstr_cls, sel_registerName(b"alloc\0".as_ptr()));
        let attr_str = with_utf8(
            raw,
            sel_registerName(b"initWithUTF8String:\0".as_ptr()),
            b"AXFrontmost\0".as_ptr() as *const i8,
        );

        let err = AXUIElementSetAttributeValue(app_el, attr_str, kCFBooleanTrue);
        // Release attr_str; AX element we drop too.
        let release: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let _ = release(attr_str, sel_registerName(b"release\0".as_ptr()));
        CFRelease(app_el);

        // AXError 0 = kAXErrorSuccess, -25204 = kAXErrorAPIDisabled (no AX permission)
        // -25211 = kAXErrorCannotComplete, -25205 = kAXErrorInvalidUIElement
        if err == 0 {
            dlog!("activate_pid({}): AX setFrontmost OK", pid);
            true
        } else {
            dlog!("activate_pid({}): AX error={}", pid, err);
            false
        }
    }
}

/// Inject text into the focused field via clipboard + Cmd+V.
///
/// `target_pid`: the PID captured before the overlay was shown (via
/// `frontmost_app_pid()`). When provided the target app is explicitly
/// re-activated so that Cmd+V lands in its text field even if the overlay
/// briefly stole focus.
pub fn inject_text(text: &str, target_pid: Option<i32>) -> Result<()> {
    dlog!(
        "inject_text: len={} target_pid={:?}",
        text.len(),
        target_pid
    );
    if text.is_empty() {
        return Ok(());
    }
    let mut clipboard = Clipboard::new()?;
    let previous_clipboard = capture_clipboard_snapshot(&mut clipboard);
    clipboard.set_text(text)?;
    thread::sleep(Duration::from_millis(80));
    let paste_result = paste_via_cgevent(target_pid);
    if let Err(err) = restore_clipboard_snapshot(&mut clipboard, previous_clipboard) {
        dlog!("clipboard restore failed after paste attempt: {:?}", err);
    }
    paste_result
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClipboardSnapshot {
    Text(String),
    NotTextOrUnavailable,
}

fn capture_clipboard_snapshot(clipboard: &mut Clipboard) -> ClipboardSnapshot {
    match clipboard.get_text() {
        Ok(text) => ClipboardSnapshot::Text(text),
        Err(err) => {
            dlog!("clipboard text snapshot unavailable: {:?}", err);
            ClipboardSnapshot::NotTextOrUnavailable
        }
    }
}

fn restore_clipboard_snapshot(
    clipboard: &mut Clipboard,
    snapshot: ClipboardSnapshot,
) -> Result<()> {
    match snapshot {
        ClipboardSnapshot::Text(text) => clipboard.set_text(text)?,
        ClipboardSnapshot::NotTextOrUnavailable => clipboard.clear()?,
    }
    Ok(())
}

fn frontmost_matches_target_pid(target_pid: Option<i32>, actual_pid: Option<i32>) -> bool {
    matches!((target_pid, actual_pid), (Some(target), Some(actual)) if target == actual)
}

#[cfg(target_os = "macos")]
fn paste_via_cgevent(target_pid: Option<i32>) -> Result<()> {
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
    // Physical keycode 9 = V (layout-independent, bypasses TSM)
    const V_KEY: u16 = 9;
    // Physical keycode 55 = left Command
    const CMD_KEY: u16 = 55;
    // kCGEventFlagMaskCommand = 1 << 20
    const CMD: u64 = 1 << 20;

    let pre_front = frontmost_app_info();
    dlog!("paste_via_cgevent: pre-activate frontmost={:?}", pre_front);

    unsafe {
        if let Some(pid) = target_pid {
            let ax_ok = activate_pid(pid);
            thread::sleep(Duration::from_millis(120));
            let post_front = frontmost_app_info();
            dlog!(
                "paste_via_cgevent: post-activate frontmost={:?} (target was {}, ax_ok={})",
                post_front,
                pid,
                ax_ok
            );
            // If AX didn't actually make the target frontmost, retry once with a
            // longer settling delay — some apps need a tick to bring up their key
            // window in response to AXFrontmost.
            let mut actual_pid = post_front.as_ref().map(|(p, _)| *p);
            if !frontmost_matches_target_pid(Some(pid), actual_pid) {
                dlog!("  retrying activation after 80ms");
                let _ = activate_pid(pid);
                thread::sleep(Duration::from_millis(80));
                let retry_front = frontmost_app_info();
                dlog!("  retry frontmost={:?}", retry_front);
                actual_pid = retry_front.as_ref().map(|(p, _)| *p);
            }
            if !frontmost_matches_target_pid(Some(pid), actual_pid) {
                anyhow::bail!("target application was not frontmost after activation");
            }
        } else {
            dlog!(
                "paste_via_cgevent: no target_pid — frontmost is {:?}",
                pre_front
            );
            anyhow::bail!("no target application captured for paste");
        }

        dlog!("paste_via_cgevent: posting Cmd+V");

        let cmd_down = CGEventCreateKeyboardEvent(std::ptr::null(), CMD_KEY, true);
        if cmd_down.is_null() {
            anyhow::bail!("CGEventCreateKeyboardEvent failed");
        }
        CGEventSetFlags(cmd_down, CMD);
        CGEventPost(HID_TAP, cmd_down);
        CFRelease(cmd_down);

        thread::sleep(Duration::from_millis(15));

        let v_down = CGEventCreateKeyboardEvent(std::ptr::null(), V_KEY, true);
        if v_down.is_null() {
            anyhow::bail!("CGEventCreateKeyboardEvent failed");
        }
        CGEventSetFlags(v_down, CMD);
        CGEventPost(HID_TAP, v_down);
        CFRelease(v_down);

        thread::sleep(Duration::from_millis(20));

        let v_up = CGEventCreateKeyboardEvent(std::ptr::null(), V_KEY, false);
        if !v_up.is_null() {
            CGEventSetFlags(v_up, CMD);
            CGEventPost(HID_TAP, v_up);
            CFRelease(v_up);
        }

        thread::sleep(Duration::from_millis(15));

        let cmd_up = CGEventCreateKeyboardEvent(std::ptr::null(), CMD_KEY, false);
        if !cmd_up.is_null() {
            CGEventSetFlags(cmd_up, 0);
            CGEventPost(HID_TAP, cmd_up);
            CFRelease(cmd_up);
        }

        thread::sleep(Duration::from_millis(40));
        let final_front = frontmost_app_info();
        dlog!("paste_via_cgevent: post-post frontmost={:?}", final_front);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn paste_via_cgevent(_target_pid: Option<i32>) -> Result<()> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow,
    };

    unsafe extern "system" fn enum_windows_for_pid(hwnd: HWND, lparam: LPARAM) -> i32 {
        let search = &mut *(lparam as *mut WindowSearch);
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid {
            search.hwnd = hwnd;
            return 0;
        }
        1
    }

    struct WindowSearch {
        pid: u32,
        hwnd: HWND,
    }

    fn find_window_for_pid(pid: i32) -> Option<HWND> {
        unsafe {
            let mut search = WindowSearch {
                pid: pid as u32,
                hwnd: std::ptr::null_mut(),
            };
            EnumWindows(
                Some(enum_windows_for_pid),
                &mut search as *mut WindowSearch as LPARAM,
            );
            if search.hwnd.is_null() {
                None
            } else {
                Some(search.hwnd)
            }
        }
    }

    fn activate_pid(pid: i32) -> bool {
        let Some(hwnd) = find_window_for_pid(pid) else {
            dlog!("activate_pid({}): no visible window found", pid);
            return false;
        };
        let ok = unsafe { SetForegroundWindow(hwnd) != 0 };
        dlog!("activate_pid({}): SetForegroundWindow={}", pid, ok);
        ok
    }

    let target_pid =
        _target_pid.ok_or_else(|| anyhow::anyhow!("no target application captured for paste"))?;
    let _ = activate_pid(target_pid);
    thread::sleep(Duration::from_millis(120));
    let actual_pid = foreground_window_pid();
    dlog!(
        "paste_via_cgevent: windows frontmost={:?} target={}",
        actual_pid,
        target_pid
    );
    if !frontmost_matches_target_pid(Some(target_pid), actual_pid) {
        anyhow::bail!("target application was not frontmost after activation");
    }

    use enigo::{Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default())?;
    enigo.key(Key::Control, enigo::Direction::Press)?;
    enigo.key(Key::Unicode('v'), enigo::Direction::Click)?;
    enigo.key(Key::Control, enigo::Direction::Release)?;
    thread::sleep(Duration::from_millis(100));
    Ok(())
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn paste_via_cgevent(_target_pid: Option<i32>) -> Result<()> {
    anyhow::bail!("text injection is only supported on macOS and Windows")
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
        assert!(inject_text("", None).is_ok());
    }

    #[test]
    fn target_frontmost_check_requires_exact_pid_match() {
        assert!(frontmost_matches_target_pid(Some(42), Some(42)));
        assert!(!frontmost_matches_target_pid(Some(42), Some(7)));
        assert!(!frontmost_matches_target_pid(Some(42), None));
        assert!(!frontmost_matches_target_pid(None, Some(42)));
    }
}
