//! Keep the playback device open only while feedback sounds are playing.
//! The idle worker blocks on its channel with no stream and no polling timer.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

pub(crate) trait Playback {
    fn play(&mut self, data: &'static [u8]);
    fn is_playing(&mut self) -> bool;
}

pub(crate) fn run<P: Playback>(rx: Receiver<&'static [u8]>, mut open: impl FnMut() -> Option<P>) {
    let mut output: Option<P> = None;
    loop {
        let data = if let Some(device) = output.as_mut() {
            if !device.is_playing() {
                // Release the device BEFORE blocking for the next sound. Keeping
                // even a silent stream alive prevents idle sleep on macOS.
                output = None;
                continue;
            }
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(data) => data,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(data) => data,
                Err(_) => break,
            }
        };
        if output.is_none() {
            output = open();
        }
        if let Some(device) = output.as_mut() {
            device.play(data);
        }
        // Opening can fail while an output device is disconnected. Keep the
        // worker alive and retry on the next sound, not in a busy retry loop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Arc;
    use std::thread;

    struct FakePlayback {
        events: Sender<&'static str>,
        playing: Arc<AtomicBool>,
    }
    impl Playback for FakePlayback {
        fn play(&mut self, _: &'static [u8]) {
            self.playing.store(true, Ordering::SeqCst);
            self.events.send("played").unwrap();
        }
        fn is_playing(&mut self) -> bool {
            self.playing.load(Ordering::SeqCst)
        }
    }
    impl Drop for FakePlayback {
        fn drop(&mut self) {
            let _ = self.events.send("closed");
        }
    }

    fn event(rx: &Receiver<&'static str>) -> &'static str {
        rx.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    #[test]
    fn releases_idle_device_and_reopens_for_next_sound() {
        let (tx, rx) = mpsc::channel();
        let (events, observed) = mpsc::channel();
        let playing = Arc::new(AtomicBool::new(false));
        let state = Arc::clone(&playing);
        let worker = thread::spawn(move || {
            run(rx, || {
                events.send("opened").unwrap();
                Some(FakePlayback {
                    events: events.clone(),
                    playing: Arc::clone(&state),
                })
            })
        });
        // No device is acquired before the first requested sound.
        assert!(observed.try_recv().is_err());
        tx.send(b"start").unwrap();
        assert_eq!(event(&observed), "opened");
        assert_eq!(event(&observed), "played");
        playing.store(false, Ordering::SeqCst);
        assert_eq!(event(&observed), "closed");
        tx.send(b"end").unwrap();
        assert_eq!(event(&observed), "opened");
        assert_eq!(event(&observed), "played");
        drop(tx);
        worker.join().unwrap();
        assert_eq!(event(&observed), "closed");
    }

    #[test]
    fn overlapping_sounds_share_device_without_cutting_playback() {
        let (tx, rx) = mpsc::channel();
        let (events, observed) = mpsc::channel();
        let playing = Arc::new(AtomicBool::new(true));
        let worker = thread::spawn(move || {
            run(rx, || {
                events.send("opened").unwrap();
                Some(FakePlayback {
                    events: events.clone(),
                    playing: Arc::clone(&playing),
                })
            })
        });
        tx.send(b"start").unwrap();
        assert_eq!(event(&observed), "opened");
        assert_eq!(event(&observed), "played");
        tx.send(b"end").unwrap();
        assert_eq!(event(&observed), "played");
        drop(tx);
        worker.join().unwrap();
        assert_eq!(event(&observed), "closed");
    }

    #[test]
    fn retries_unavailable_device_on_next_request() {
        let (tx, rx) = mpsc::channel();
        let (events, observed) = mpsc::channel();
        let mut available = false;
        let worker = thread::spawn(move || {
            run(rx, || {
                if !available {
                    available = true;
                    events.send("unavailable").unwrap();
                    return None;
                }
                events.send("opened").unwrap();
                Some(FakePlayback {
                    events: events.clone(),
                    playing: Arc::new(AtomicBool::new(true)),
                })
            })
        });
        tx.send(b"start").unwrap();
        assert_eq!(event(&observed), "unavailable");
        tx.send(b"end").unwrap();
        assert_eq!(event(&observed), "opened");
        assert_eq!(event(&observed), "played");
        drop(tx);
        worker.join().unwrap();
        assert_eq!(event(&observed), "closed");
    }
}
