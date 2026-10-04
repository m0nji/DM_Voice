//! Cross-platform sound feedback for the recording hotkey.
//!
//! Sounds are bundled into the binary via `include_bytes!` and decoded with
//! `rodio` (which uses `cpal` underneath, the same backend the recording
//! pipeline already depends on). A single dedicated audio thread owns the
//! `OutputStream` only while sounds play. Idle playback releases the device
//! instead of leaving a silent stream that prevents automatic system sleep.
//!
//! Calls are silent no-ops when `enabled` is false, when the audio device
//! cannot be opened, or when decoding fails.

use std::io::Cursor;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};

#[path = "sound_worker.rs"]
mod sound_worker;
use sound_worker::Playback;

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
            // CoreAudio's stream is !Send; creation, playback and disposal
            // all stay on the same owner thread on every supported platform.
            thread::Builder::new()
                .name("dm-voice-sounds".into())
                .spawn(move || sound_worker::run(rx, SoundOutput::open))
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

struct SoundOutput {
    // Drop sinks and the handle before releasing the underlying device.
    sinks: Vec<Sink>,
    handle: OutputStreamHandle,
    _stream: OutputStream,
}

impl SoundOutput {
    fn open() -> Option<Self> {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Some(Self {
                sinks: Vec::new(),
                handle,
                _stream: stream,
            }),
            Err(e) => {
                crate::dlog::log(&format!("[sounds] OutputStream::try_default failed: {}", e));
                None
            }
        }
    }
}

impl Playback for SoundOutput {
    fn play(&mut self, data: &'static [u8]) {
        // One sink per feedback sound preserves overlapping start/end cues.
        // Keep the device until ALL sinks finish, then release it in the worker.
        let decoder = match Decoder::new(Cursor::new(data)) {
            Ok(decoder) => decoder,
            Err(e) => {
                crate::dlog::log(&format!("[sounds] decode failed: {}", e));
                return;
            }
        };
        match Sink::try_new(&self.handle) {
            Ok(sink) => {
                sink.append(decoder);
                self.sinks.push(sink);
            }
            Err(e) => crate::dlog::log(&format!("[sounds] playback failed: {}", e)),
        }
    }

    fn is_playing(&mut self) -> bool {
        self.sinks.retain(|sink| !sink.empty());
        !self.sinks.is_empty()
    }
}
