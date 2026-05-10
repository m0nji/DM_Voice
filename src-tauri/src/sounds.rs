//! Cross-platform sound feedback for the recording hotkey.
//!
//! Sounds are bundled into the binary via `include_bytes!` and decoded with
//! `rodio` (which uses `cpal` underneath, the same backend the recording
//! pipeline already depends on). A single dedicated audio thread owns the
//! `OutputStream` so subsequent plays don't pay the device-open latency.
//!
//! Calls are silent no-ops when `enabled` is false, when the audio device
//! cannot be opened, or when decoding fails.

use std::io::Cursor;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use rodio::{Decoder, OutputStream, Source};

const PURR: &[u8] = include_bytes!("../sounds/Purr.wav");
const BOTTLE: &[u8] = include_bytes!("../sounds/Bottle.wav");

static SENDER: OnceLock<Option<Sender<&'static [u8]>>> = OnceLock::new();

pub fn play_start(enabled: bool) {
    if !enabled {
        return;
    }
    send(PURR);
}

pub fn play_end(enabled: bool) {
    if !enabled {
        return;
    }
    send(BOTTLE);
}

fn send(data: &'static [u8]) {
    let Some(tx) = sender() else {
        return;
    };
    if let Err(e) = tx.send(data) {
        crate::dlog::log(&format!("[sounds] send failed: {}", e));
    }
}

fn sender() -> Option<&'static Sender<&'static [u8]>> {
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<&'static [u8]>();
            // OutputStream is !Send on some platforms, so it lives entirely on
            // this thread. We block-receive forever; the thread exits when the
            // sender is dropped at process shutdown.
            thread::Builder::new()
                .name("dm-voice-sounds".into())
                .spawn(move || {
                    let (_stream, handle) = match OutputStream::try_default() {
                        Ok(s) => s,
                        Err(e) => {
                            crate::dlog::log(&format!(
                                "[sounds] OutputStream::try_default failed: {}",
                                e
                            ));
                            return;
                        }
                    };
                    while let Ok(data) = rx.recv() {
                        match Decoder::new(Cursor::new(data)) {
                            Ok(d) => {
                                if let Err(e) = handle.play_raw(d.convert_samples()) {
                                    crate::dlog::log(&format!(
                                        "[sounds] play_raw failed: {}",
                                        e
                                    ));
                                }
                            }
                            Err(e) => {
                                crate::dlog::log(&format!(
                                    "[sounds] decode failed: {}",
                                    e
                                ));
                            }
                        }
                    }
                })
                .ok()?;
            Some(tx)
        })
        .as_ref()
}
