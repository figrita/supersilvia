// SPDX-License-Identifier: AGPL-3.0-or-later

//! A live recording's file: an Output's picture, written one frame per slot of the show's
//! clock by a thread of its own, while the show plays.
//!
//! **A slot is a frame of the file**, `1 / fps` of the show's clock long, the first at the
//! moment the recording began. The synth reads a picture back once per slot it enters and
//! offers it here stamped with that slot; this thread writes one frame per slot in order,
//! whatever arrives. **A slot no picture reached repeats the frame before it and counts as
//! dropped** — the show drew slower than the rate, the GPU had no read free, or this thread
//! had no room — and a slot the Output was not drawn in on purpose, a paused loop, repeats it
//! as [`Picture::Same`] and counts as nothing. A second picture for one slot replaces the
//! first. So the file is as long as the show was, to the nearest frame, however the show
//! went: [`frames_for`] is the length a stop gives it, and one picture is held back until the
//! next arrives or the stop says how long the file is, so a slot past the stop is never
//! written.
//!
//! **Nothing here waits for the hand that offers.** [`Recorder::offer`] is a `try_send` into a
//! queue a few frames deep and a picture with no room is refused and counted, and
//! [`Recorder::close`] ends the queue and returns: the synth polls [`Recorder::done`] and
//! takes the outcome once the thread has closed the file. Only dropping a recorder that was
//! never finished waits for it, which is quitting. The encoder's own queue is held to a few
//! frames ([`Sink::busy`]), so a slow encoder backs up into this thread and from it into
//! refusals rather than into memory.
//!
//! The file is [`super::encode::Encoder`]'s: the hardware encoder into an `.mp4`, written as a
//! `.part` and renamed when it closes, so a crash leaves only the `.part`. Picture only, and
//! premultiplied over black, as a video render is. See
//! [docs/rendering.md](../../docs/rendering.md#live-recording).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many pictures may wait for the writer before an offer is refused.
pub const QUEUE: usize = 4;

/// How long the writer waits on a sink that stays busy before it calls the sink stopped.
const STUCK: Duration = Duration::from_secs(10);

/// What a recording's frames are written into: the hardware encoder, or a test's stand-in.
pub trait Sink: Send + 'static {
    /// One frame, RGBA8 rows top first.
    ///
    /// # Errors
    /// The frame was refused.
    fn push(&mut self, rgba: &[u8]) -> Result<(), String>;
    /// The frame pushed last, once more.
    ///
    /// # Errors
    /// The frame was refused.
    fn repeat(&mut self) -> Result<(), String>;
    /// Whether the sink holds as many frames as it should before it is handed another.
    fn busy(&self) -> bool {
        false
    }
    /// Close the file and put it under its name.
    ///
    /// # Errors
    /// The file did not close.
    fn finish(self: Box<Self>) -> Result<PathBuf, String>;
}

impl Sink for super::encode::Encoder {
    fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
        super::encode::Encoder::push(self, rgba)
    }

    fn repeat(&mut self) -> Result<(), String> {
        super::encode::Encoder::repeat(self)
    }

    fn busy(&self) -> bool {
        self.queued() > QUEUE as u64
    }

    fn finish(self: Box<Self>) -> Result<PathBuf, String> {
        super::encode::Encoder::finish(*self)
    }
}

/// What a slot of the recording shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// A frame read back for it, RGBA8 rows top first.
    Frame(Vec<u8>),
    /// The frame before, again, on purpose: the Output was not drawn because nothing about it
    /// moved.
    Same,
}

/// The counts and the outcome the writer leaves for the synth to read.
#[derive(Default)]
struct Shared {
    written: AtomicU64,
    dropped: AtomicU64,
    /// How long the file is, once the recording has stopped.
    frames: Mutex<Option<u64>>,
    error: Mutex<Option<String>>,
    outcome: Mutex<Option<Result<PathBuf, String>>>,
}

fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A recording on its way to disk.
pub struct Recorder {
    tx: Option<SyncSender<(u64, Picture)>>,
    handle: Option<std::thread::JoinHandle<()>>,
    shared: Arc<Shared>,
    refused: u64,
    /// The latest slot offered, which a recorder dropped unclosed ends after.
    latest: Option<u64>,
}

impl Recorder {
    /// Open `dest`, `size` at `fps`, on the machine's hardware encoder. Opened before the first
    /// picture, so a machine with no encoder refuses the recording rather than failing it a
    /// frame in.
    ///
    /// # Errors
    /// No hardware encoder, a size of nothing, or a folder that could not be made.
    pub fn start(dest: &Path, size: (u32, u32), fps: f64) -> Result<Self, String> {
        let encoder = super::encode::Encoder::start(dest, size.0, size.1, fps)?;
        Self::with_sink(Box::new(encoder), size)
    }

    /// Write `size` frames into `sink`.
    ///
    /// # Errors
    /// The writer thread could not be started.
    pub fn with_sink(sink: Box<dyn Sink>, size: (u32, u32)) -> Result<Self, String> {
        let (tx, rx) = sync_channel(QUEUE);
        let shared = Arc::new(Shared::default());
        let theirs = Arc::clone(&shared);
        let black = vec![0; size.0 as usize * size.1 as usize * 4];
        let handle = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || write(sink, &rx, &theirs, &black))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            shared,
            refused: 0,
            latest: None,
        })
    }

    /// Hand over the picture for `slot`. Never waits: false where the queue had no room or
    /// the writer has stopped, and the slot is left to the frame before it.
    pub fn offer(&mut self, slot: u64, picture: Picture) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        if self.error().is_some() {
            self.refused += 1;
            return false;
        }
        self.latest = Some(self.latest.map_or(slot, |l| l.max(slot)));
        match tx.try_send((slot, picture)) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.refused += 1;
                false
            }
        }
    }

    /// Pictures refused because the writer was behind or had stopped.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Frames written.
    pub fn written(&self) -> u64 {
        self.shared.written.load(Ordering::Acquire)
    }

    /// Slots written as a repeat of the one before for lack of a picture.
    pub fn dropped(&self) -> u64 {
        self.shared.dropped.load(Ordering::Acquire)
    }

    /// Why the writer stopped, where it failed.
    pub fn error(&self) -> Option<String> {
        locked(&self.shared.error).clone()
    }

    /// No more pictures: the file is `frames` long. Never waits; the writer writes what is
    /// queued, fills the rest and closes the file on its own thread.
    pub fn close(&mut self, frames: u64) {
        if self.tx.is_none() {
            return;
        }
        *locked(&self.shared.frames) = Some(frames.max(1));
        self.tx = None;
    }

    /// Whether the writer has finished, the file closed or given up.
    pub fn done(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }

    /// Once [`Self::done`]: where the file went, or why it did not. Taken once.
    pub fn outcome(&mut self) -> Option<Result<PathBuf, String>> {
        if !self.done() {
            return None;
        }
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        locked(&self.shared.outcome).take()
    }
}

impl Drop for Recorder {
    /// A recorder dropped unclosed is a run ending: the file is closed after the latest slot
    /// offered, and waited for, so quitting leaves a film rather than a `.part`.
    fn drop(&mut self) {
        self.close(self.latest.map_or(1, |l| l + 1));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// The writer thread: one frame per slot, in order, until the queue closes; then the rest of
/// the length to the stop, and the file closed.
fn write(mut sink: Box<dyn Sink>, rx: &Receiver<(u64, Picture)>, shared: &Shared, black: &[u8]) {
    let mut file = File {
        sink: &mut *sink,
        shared,
        black,
        next: 0,
        pushed: false,
    };
    // The newest picture, held until the next arrives or the stop says the file reaches it.
    let mut held: Option<(u64, Picture)> = None;
    let mut result = Ok(());
    for (slot, picture) in rx {
        match &held {
            Some((at, _)) if slot < *at => continue,
            Some((at, _)) if slot == *at => {}
            Some(_) => {
                if let Some((at, picture)) = held.take() {
                    result = file.slot(at, picture);
                }
            }
            None if slot < file.next => continue,
            None => {}
        }
        if result.is_err() {
            break;
        }
        held = Some((slot, picture));
    }
    let frames = locked(&shared.frames).unwrap_or(file.next);
    if result.is_ok()
        && let Some((at, picture)) = held.take()
        && at < frames
    {
        result = file.slot(at, picture);
    }
    if result.is_ok() {
        result = file.fill(frames);
    }
    let outcome = match result {
        Ok(()) => sink.finish(),
        Err(e) => {
            drop(sink);
            Err(e)
        }
    };
    if let Err(e) = &outcome {
        locked(&shared.error).get_or_insert_with(|| e.clone());
    }
    *locked(&shared.outcome) = Some(outcome);
}

/// The file as the writer has it: the slot it writes next, and whether any frame is in it.
struct File<'a> {
    sink: &'a mut dyn Sink,
    shared: &'a Shared,
    black: &'a [u8],
    next: u64,
    pushed: bool,
}

impl File<'_> {
    /// Write slot `at` showing `picture`, the slots before it that nothing reached first.
    fn slot(&mut self, at: u64, picture: Picture) -> Result<(), String> {
        self.fill(at)?;
        match picture {
            Picture::Frame(rgba) => self.frame(Some(&rgba)),
            Picture::Same => self.frame(None),
        }
    }

    /// Repeat the frame before into every slot up to `end`, each counted as dropped.
    fn fill(&mut self, end: u64) -> Result<(), String> {
        while self.next < end {
            self.frame(None)?;
            self.shared.dropped.fetch_add(1, Ordering::Release);
        }
        Ok(())
    }

    /// One frame: `rgba`, or the one before again — black where there was none.
    fn frame(&mut self, rgba: Option<&[u8]>) -> Result<(), String> {
        let since = Instant::now();
        while self.sink.busy() {
            if since.elapsed() > STUCK {
                return Err("the encoder stopped taking frames".to_string());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let result = match (rgba, self.pushed) {
            (Some(rgba), _) => self.sink.push(rgba),
            (None, true) => self.sink.repeat(),
            (None, false) => self.sink.push(self.black),
        };
        if let Err(e) = &result {
            locked(&self.shared.error).get_or_insert_with(|| e.clone());
        }
        result?;
        self.pushed = true;
        self.next += 1;
        self.shared.written.fetch_add(1, Ordering::Release);
        Ok(())
    }
}

/// The slot `seconds` into a recording falls in at `fps`; before the start is the first.
pub fn slot_at(seconds: f64, fps: f64) -> u64 {
    (seconds.max(0.0) * fps).floor() as u64
}

/// How many frames a recording `seconds` long is at `fps`: the nearest whole frame, and never
/// none.
pub fn frames_for(seconds: f64, fps: f64) -> u64 {
    ((seconds.max(0.0) * fps).round() as u64).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A frame as the tests tell them apart: its first byte.
    fn frame(n: u8) -> Vec<u8> {
        vec![n; 2 * 2 * 4]
    }

    /// What a sink was handed: the first byte of each frame, in order.
    #[derive(Clone, Default)]
    struct Seen(Arc<Mutex<Vec<u8>>>);

    impl Seen {
        fn frames(&self) -> Vec<u8> {
            self.0.lock().unwrap().clone()
        }
    }

    /// A sink that keeps what it is handed, taking `delay` over each frame.
    struct Fake {
        seen: Seen,
        last: Option<u8>,
        delay: Duration,
    }

    impl Fake {
        fn new(delay: Duration) -> (Box<Self>, Seen) {
            let seen = Seen::default();
            let sink = Box::new(Self {
                seen: seen.clone(),
                last: None,
                delay,
            });
            (sink, seen)
        }
    }

    impl Sink for Fake {
        fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
            std::thread::sleep(self.delay);
            self.last = Some(rgba[0]);
            self.seen.0.lock().unwrap().push(rgba[0]);
            Ok(())
        }

        fn repeat(&mut self) -> Result<(), String> {
            std::thread::sleep(self.delay);
            let last = self.last.ok_or("nothing to repeat")?;
            self.seen.0.lock().unwrap().push(last);
            Ok(())
        }

        fn finish(self: Box<Self>) -> Result<PathBuf, String> {
            Ok(PathBuf::from("film.mp4"))
        }
    }

    /// Wait for the writer, which a test may; the synth never does.
    fn finished(recorder: &mut Recorder) -> Result<PathBuf, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !recorder.done() {
            assert!(Instant::now() < deadline, "the writer never finished");
            std::thread::sleep(Duration::from_millis(1));
        }
        recorder
            .outcome()
            .expect("a finished writer has an outcome")
    }

    #[test]
    fn a_slot_is_where_the_clock_falls_and_a_length_is_rounded() {
        assert_eq!(slot_at(0.0, 30.0), 0);
        assert_eq!(slot_at(0.999 / 30.0, 30.0), 0);
        assert_eq!(slot_at(1.0 / 30.0, 30.0), 1);
        assert_eq!(slot_at(2.0, 30.0), 60);
        assert_eq!(slot_at(-0.5, 30.0), 0, "before the start is the first slot");
        assert_eq!(frames_for(2.0, 30.0), 60, "two seconds at 30 fps");
        assert_eq!(frames_for(60.0, 60.0), 3600, "a minute is a minute");
        assert_eq!(
            frames_for(2.000_000_2, 30.0),
            60,
            "the clock's own rounding"
        );
        assert_eq!(frames_for(0.0, 30.0), 1, "a recording is never empty");
    }

    /// One picture a slot is one frame a slot, in order, and nothing dropped.
    #[test]
    fn a_picture_in_every_slot_is_a_frame_in_every_slot() {
        let (sink, seen) = Fake::new(Duration::ZERO);
        let mut recorder = Recorder::with_sink(sink, (2, 2)).unwrap();
        for slot in 0..5u8 {
            assert!(recorder.offer(u64::from(slot), Picture::Frame(frame(slot))));
            // A slot's length apart, as the show offers them, rather than all at once.
            std::thread::sleep(Duration::from_millis(5));
        }
        recorder.close(5);
        assert_eq!(finished(&mut recorder), Ok(PathBuf::from("film.mp4")));
        assert_eq!(seen.frames(), [0, 1, 2, 3, 4]);
        assert_eq!((recorder.written(), recorder.dropped()), (5, 0));
    }

    /// A slot no picture reached repeats the frame before it and counts as dropped; a slot
    /// the Output was not drawn in on purpose repeats it and does not.
    #[test]
    fn a_slot_with_no_picture_repeats_the_one_before_and_counts() {
        let (sink, seen) = Fake::new(Duration::ZERO);
        let mut recorder = Recorder::with_sink(sink, (2, 2)).unwrap();
        recorder.offer(0, Picture::Frame(frame(10)));
        recorder.offer(1, Picture::Frame(frame(11)));
        recorder.offer(4, Picture::Frame(frame(14)));
        recorder.offer(5, Picture::Same);
        recorder.close(8);
        finished(&mut recorder).unwrap();
        assert_eq!(seen.frames(), [10, 11, 11, 11, 14, 14, 14, 14]);
        assert_eq!(
            (recorder.written(), recorder.dropped()),
            (8, 4),
            "slots 2, 3, 6 and 7 had no picture; slot 5 said it was the same"
        );
    }

    /// A slot that comes back twice keeps the newer picture, and a picture at or past the
    /// length the recording was stopped at is not in the file.
    #[test]
    fn the_length_is_the_stops_and_a_slot_is_one_frame() {
        let (sink, seen) = Fake::new(Duration::ZERO);
        let mut recorder = Recorder::with_sink(sink, (2, 2)).unwrap();
        recorder.offer(0, Picture::Frame(frame(1)));
        recorder.offer(1, Picture::Frame(frame(2)));
        recorder.offer(1, Picture::Frame(frame(3)));
        recorder.offer(2, Picture::Frame(frame(4)));
        recorder.close(2);
        finished(&mut recorder).unwrap();
        assert_eq!(seen.frames(), [1, 3]);
    }

    /// A first slot nothing reached is black, not nothing.
    #[test]
    fn a_recording_starts_black_where_no_picture_arrived() {
        let (sink, seen) = Fake::new(Duration::ZERO);
        let mut recorder = Recorder::with_sink(sink, (2, 2)).unwrap();
        recorder.offer(2, Picture::Frame(frame(7)));
        recorder.close(3);
        finished(&mut recorder).unwrap();
        assert_eq!(seen.frames(), [0, 0, 7]);
        assert_eq!(recorder.dropped(), 2);
    }

    /// **A slow writer drops and counts, and never holds the hand that offers.** Sixty
    /// pictures offered at once to a sink taking twenty milliseconds a frame: every offer
    /// returns at once, the ones the queue had no room for are refused, and the file is still
    /// one frame a slot, the gaps repeats counted as dropped.
    #[test]
    fn a_slow_writer_drops_and_counts_frames_instead_of_stalling() {
        let (sink, seen) = Fake::new(Duration::from_millis(20));
        let mut recorder = Recorder::with_sink(sink, (2, 2)).unwrap();
        let mut slowest = Duration::ZERO;
        let mut taken = 0;
        for slot in 0..60u8 {
            let at = Instant::now();
            if recorder.offer(u64::from(slot), Picture::Frame(frame(slot))) {
                taken += 1;
            }
            slowest = slowest.max(at.elapsed());
        }
        assert!(
            slowest < Duration::from_millis(10),
            "an offer never waits for the writer: {slowest:?}"
        );
        assert!(taken < 60, "a queue with no room refuses");
        assert_eq!(recorder.refused(), 60 - taken);
        let at = Instant::now();
        recorder.close(60);
        assert!(at.elapsed() < Duration::from_millis(10), "nor does a close");
        finished(&mut recorder).unwrap();
        assert_eq!(seen.frames().len(), 60, "one frame a slot all the same");
        assert_eq!(recorder.written(), 60);
        assert!(
            recorder.dropped() >= 60 - taken,
            "every slot a refused picture left empty is counted: {} of {}",
            recorder.dropped(),
            60 - taken
        );
    }

    /// A sink that fails says so, and an offer after it is refused rather than queued.
    #[test]
    fn a_failing_sink_is_said_and_stops_the_recording() {
        struct Full;
        impl Sink for Full {
            fn push(&mut self, _: &[u8]) -> Result<(), String> {
                Err("No space left on device".to_string())
            }
            fn repeat(&mut self) -> Result<(), String> {
                Err("No space left on device".to_string())
            }
            fn finish(self: Box<Self>) -> Result<PathBuf, String> {
                Ok(PathBuf::new())
            }
        }
        let mut recorder = Recorder::with_sink(Box::new(Full), (2, 2)).unwrap();
        // The second picture is what lets the first be written: one is held back until the
        // next says the file reaches it.
        recorder.offer(0, Picture::Frame(frame(1)));
        recorder.offer(1, Picture::Frame(frame(1)));
        let deadline = Instant::now() + Duration::from_secs(10);
        while recorder.error().is_none() {
            assert!(Instant::now() < deadline, "the failure is never said");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorder.error().as_deref(), Some("No space left on device"));
        assert!(!recorder.offer(2, Picture::Frame(frame(2))));
        recorder.close(2);
        assert_eq!(
            finished(&mut recorder),
            Err("No space left on device".to_string())
        );
    }
}
