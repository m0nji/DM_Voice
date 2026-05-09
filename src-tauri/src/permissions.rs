//! Explicit permission prompts for macOS.
//!
//! Without explicit calls, the underlying APIs we use fail silently:
//! - cpal opens a CoreAudio AudioUnit and just receives all-zero samples when
//!   microphone access is denied (no error, no prompt).
//! - CGEventPost is a no-op when Accessibility access is denied (no error,
//!   no prompt).
//!
//! Both APIs need an explicit "request access" call up-front so macOS shows
//! the standard TCC dialog. We do that on app startup.

#[cfg(target_os = "macos")]
pub fn request_all() {
    request_microphone();
    request_accessibility();
}

#[cfg(not(target_os = "macos"))]
pub fn request_all() {}

/// Status snapshot exposed to the settings UI.
#[derive(serde::Serialize, Clone)]
pub struct PermissionStatus {
    /// "granted", "denied", "not_determined", "restricted", "unknown"
    pub microphone: &'static str,
    /// "granted", "denied" — Accessibility has no "not determined" state
    /// (it's always either trusted or not trusted).
    pub accessibility: &'static str,
}

#[cfg(target_os = "macos")]
pub fn status() -> PermissionStatus {
    PermissionStatus {
        microphone: mic_status(),
        accessibility: if accessibility_trusted() {
            "granted"
        } else {
            "denied"
        },
    }
}

#[cfg(not(target_os = "macos"))]
pub fn status() -> PermissionStatus {
    PermissionStatus {
        microphone: "granted",
        accessibility: "granted",
    }
}

#[cfg(target_os = "macos")]
fn mic_status() -> &'static str {
    use std::ffi::c_void;
    extern "C" {
        fn objc_getClass(name: *const u8) -> *mut c_void;
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }
    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgIdCStr =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const i8) -> *mut c_void;
    type MsgGetIsize = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> isize;

    unsafe {
        let cls = objc_getClass(b"AVCaptureDevice\0".as_ptr());
        if cls.is_null() {
            return "unknown";
        }
        let nsstr_cls = objc_getClass(b"NSString\0".as_ptr());
        let alloc: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let raw = alloc(nsstr_cls, sel_registerName(b"alloc\0".as_ptr()));
        let with_utf8: MsgIdCStr = std::mem::transmute(objc_msgSend as *const ());
        let media_type = with_utf8(
            raw,
            sel_registerName(b"initWithUTF8String:\0".as_ptr()),
            b"soun\0".as_ptr() as *const i8,
        );
        let sel = sel_registerName(b"authorizationStatusForMediaType:\0".as_ptr());
        let f: MsgGetIsize = std::mem::transmute(objc_msgSend as *const ());
        let s = f(cls, sel, media_type);
        // 0=NotDetermined, 1=Restricted, 2=Denied, 3=Authorized
        match s {
            0 => "not_determined",
            1 => "restricted",
            2 => "denied",
            3 => "granted",
            _ => "unknown",
        }
    }
}

#[cfg(target_os = "macos")]
fn accessibility_trusted() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

/// Trigger the microphone TCC prompt.
///
/// First logs the current authorization status, then:
/// - if `notDetermined`: calls `+[AVCaptureDevice requestAccessForMediaType:]`
///   to show the standard prompt (async, completion block).
/// - regardless of status: spins up a real `AVCaptureSession` with a mic input
///   and calls `startRunning`. This is a stronger probe than `requestAccess`
///   and is what some apps need to force `tccd` to revisit a stale cache and
///   show the prompt when the DB says `notDetermined` but the daemon still
///   has a stale `denied` decision in RAM.
#[cfg(target_os = "macos")]
fn request_microphone() {
    use std::ffi::c_void;

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    extern "C" {
        fn objc_getClass(name: *const u8) -> *mut c_void;
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
        // Imported globals that supply the proper isa pointer for blocks.
        static _NSConcreteGlobalBlock: *const c_void;
    }

    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgIdCStr =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const i8) -> *mut c_void;
    type MsgIdId =
        unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    type MsgIdIdPP = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> *mut c_void;
    type MsgVoid = unsafe extern "C" fn(*mut c_void, *mut c_void);
    type MsgRequestAccess = unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    );

    // Block ABI types (libclosure / clang Block_layout)
    #[repr(C)]
    struct BlockDescriptor {
        reserved: usize,
        size: usize,
    }
    #[repr(C)]
    struct Block {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: extern "C" fn(*mut c_void, bool),
        descriptor: *const BlockDescriptor,
    }

    extern "C" fn callback(_block: *mut c_void, granted: bool) {
        crate::dlog::log(&format!(
            "[permissions] AVCaptureDevice.requestAccess callback granted={}",
            granted
        ));
    }

    let status_before = mic_status();
    crate::dlog::log(&format!(
        "[permissions] mic status before request = {}",
        status_before
    ));

    unsafe {
        let cls = objc_getClass(b"AVCaptureDevice\0".as_ptr());
        if cls.is_null() {
            crate::dlog::log("[permissions] AVCaptureDevice class missing");
            return;
        }
        let nsstr_cls = objc_getClass(b"NSString\0".as_ptr());
        let alloc: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let raw = alloc(nsstr_cls, sel_registerName(b"alloc\0".as_ptr()));
        let with_utf8: MsgIdCStr = std::mem::transmute(objc_msgSend as *const ());
        let media_type = with_utf8(
            raw,
            sel_registerName(b"initWithUTF8String:\0".as_ptr()),
            b"soun\0".as_ptr() as *const i8,
        );

        // 1) AVCaptureSession probe FIRST. This is what triggers a fresh
        //    permission check in tccd: alloc the session, add the default
        //    audio device as input, startRunning. Some setups silently ignore
        //    `requestAccess` (callback returns false in milliseconds with no
        //    prompt) but show the prompt for a real capture session.
        let session_cls = objc_getClass(b"AVCaptureSession\0".as_ptr());
        let dev_cls = objc_getClass(b"AVCaptureDevice\0".as_ptr());
        let input_cls = objc_getClass(b"AVCaptureDeviceInput\0".as_ptr());
        if session_cls.is_null() || dev_cls.is_null() || input_cls.is_null() {
            crate::dlog::log("[permissions] AVCaptureSession classes missing");
        } else {
            // AVCaptureDevice* dev = [AVCaptureDevice defaultDeviceWithMediaType:@"soun"];
            let default_dev_sel =
                sel_registerName(b"defaultDeviceWithMediaType:\0".as_ptr());
            let default_dev: MsgIdId = std::mem::transmute(objc_msgSend as *const ());
            let dev = default_dev(dev_cls, default_dev_sel, media_type);
            if dev.is_null() {
                crate::dlog::log(
                    "[permissions] AVCaptureSession probe: no default audio device",
                );
            } else {
                // AVCaptureDeviceInput* in = [[AVCaptureDeviceInput alloc] initWithDevice:dev error:&err];
                let input_alloc: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
                let input_raw = input_alloc(input_cls, sel_registerName(b"alloc\0".as_ptr()));
                let init_sel = sel_registerName(b"initWithDevice:error:\0".as_ptr());
                let mut err: *mut c_void = std::ptr::null_mut();
                let init_with_dev: MsgIdIdPP = std::mem::transmute(objc_msgSend as *const ());
                let input = init_with_dev(input_raw, init_sel, dev, &mut err);
                if input.is_null() {
                    log_nserror("AVCaptureDeviceInput init", err);
                } else {
                    let sess_alloc: MsgIdNoArg =
                        std::mem::transmute(objc_msgSend as *const ());
                    let sess_raw =
                        sess_alloc(session_cls, sel_registerName(b"alloc\0".as_ptr()));
                    let sess_init: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
                    let sess = sess_init(sess_raw, sel_registerName(b"init\0".as_ptr()));
                    if !sess.is_null() {
                        let add_input: MsgIdId =
                            std::mem::transmute(objc_msgSend as *const ());
                        add_input(sess, sel_registerName(b"addInput:\0".as_ptr()), input);

                        let start: MsgVoid = std::mem::transmute(objc_msgSend as *const ());
                        crate::dlog::log("[permissions] AVCaptureSession.startRunning");
                        start(sess, sel_registerName(b"startRunning\0".as_ptr()));

                        // Block briefly so the prompt has time to surface.
                        std::thread::sleep(std::time::Duration::from_millis(800));

                        let stop: MsgVoid = std::mem::transmute(objc_msgSend as *const ());
                        stop(sess, sel_registerName(b"stopRunning\0".as_ptr()));

                        let release: MsgVoid =
                            std::mem::transmute(objc_msgSend as *const ());
                        release(sess, sel_registerName(b"release\0".as_ptr()));
                    }
                    let release: MsgVoid = std::mem::transmute(objc_msgSend as *const ());
                    release(input, sel_registerName(b"release\0".as_ptr()));
                }
            }
        }

        let status_mid = mic_status();
        crate::dlog::log(&format!(
            "[permissions] mic status after AVCaptureSession probe = {}",
            status_mid
        ));

        // 2) Standard async prompt. macOS only shows a UI when the current
        //    status is notDetermined; for granted/denied/restricted it
        //    invokes the block synchronously with the existing answer.
        let descriptor = Box::leak(Box::new(BlockDescriptor {
            reserved: 0,
            size: std::mem::size_of::<Block>(),
        })) as *const BlockDescriptor;
        let block = Box::leak(Box::new(Block {
            isa: _NSConcreteGlobalBlock,
            flags: 1 << 28, // BLOCK_IS_GLOBAL
            reserved: 0,
            invoke: callback,
            descriptor,
        })) as *mut Block as *mut c_void;

        let req_sel =
            sel_registerName(b"requestAccessForMediaType:completionHandler:\0".as_ptr());
        let req: MsgRequestAccess = std::mem::transmute(objc_msgSend as *const ());
        crate::dlog::log(
            "[permissions] calling AVCaptureDevice.requestAccessForMediaType",
        );
        req(cls, req_sel, media_type, block);

        let status_after = mic_status();
        crate::dlog::log(&format!(
            "[permissions] mic status after requestAccess = {}",
            status_after
        ));
    }
}

#[cfg(target_os = "macos")]
fn log_nserror(label: &str, err: *mut std::ffi::c_void) {
    use std::ffi::c_void;
    if err.is_null() {
        crate::dlog::log(&format!("[permissions] {}: null NSError", label));
        return;
    }
    extern "C" {
        fn sel_registerName(name: *const u8) -> *mut c_void;
        fn objc_msgSend(recv: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
    }
    type MsgIdNoArg = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
    type MsgIsize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize;
    type MsgCStr = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *const i8;
    unsafe {
        let code: MsgIsize = std::mem::transmute(objc_msgSend as *const ());
        let c = code(err, sel_registerName(b"code\0".as_ptr()));
        let domain_get: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let domain = domain_get(err, sel_registerName(b"domain\0".as_ptr()));
        let utf8: MsgCStr = std::mem::transmute(objc_msgSend as *const ());
        let dom_cstr = if domain.is_null() {
            std::ptr::null()
        } else {
            utf8(domain, sel_registerName(b"UTF8String\0".as_ptr()))
        };
        let dom_str = if dom_cstr.is_null() {
            "<null>".to_string()
        } else {
            std::ffi::CStr::from_ptr(dom_cstr)
                .to_string_lossy()
                .into_owned()
        };
        let desc: MsgIdNoArg = std::mem::transmute(objc_msgSend as *const ());
        let descr = desc(err, sel_registerName(b"localizedDescription\0".as_ptr()));
        let descr_cstr = if descr.is_null() {
            std::ptr::null()
        } else {
            utf8(descr, sel_registerName(b"UTF8String\0".as_ptr()))
        };
        let descr_str = if descr_cstr.is_null() {
            "<null>".to_string()
        } else {
            std::ffi::CStr::from_ptr(descr_cstr)
                .to_string_lossy()
                .into_owned()
        };
        crate::dlog::log(&format!(
            "[permissions] {}: NSError domain={} code={} desc={}",
            label, dom_str, c, descr_str
        ));
    }
}

/// Trigger the Accessibility TCC prompt by calling AXIsProcessTrustedWithOptions
/// with kAXTrustedCheckOptionPrompt = YES.
#[cfg(target_os = "macos")]
fn request_accessibility() {
    use std::ffi::c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        // CFDictionary helpers
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        // CFString helpers
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            cstr: *const i8,
            encoding: u32,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    // kCFStringEncodingUTF8 = 0x08000100
    const UTF8: u32 = 0x0800_0100;

    unsafe {
        let key = CFStringCreateWithCString(
            std::ptr::null(),
            b"AXTrustedCheckOptionPrompt\0".as_ptr() as *const i8,
            UTF8,
        );
        if key.is_null() {
            crate::dlog::log("[permissions] CFString create failed");
            return;
        }
        let keys: [*const c_void; 1] = [key];
        let values: [*const c_void; 1] = [kCFBooleanTrue];
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        if dict.is_null() {
            CFRelease(key);
            crate::dlog::log("[permissions] CFDictionary create failed");
            return;
        }
        let trusted = AXIsProcessTrustedWithOptions(dict);
        crate::dlog::log(&format!(
            "[permissions] AXIsProcessTrustedWithOptions(prompt=YES) -> trusted={}",
            trusted
        ));
        CFRelease(dict);
        CFRelease(key);
    }
}
