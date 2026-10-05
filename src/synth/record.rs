// SPDX-License-Identifier: AGPL-3.0-or-later

//! A live recording, from the synth's side: an Output's picture written to a video file while
//! the show plays — hands, MIDI, a live camera — where the offline render steps time.
//!
//! **Slots of the show's clock.** A recording keeps the moment it began on [`crate::clock::Clock::elapsed`],
//! the one clock, wall time with every stall in it, and divides what follows into slots of
//! `1 / fps`, the Output's own Record FPS read when it begins. On the first tick in each slot
//! the Output is drawn — [`Synth::drawing`] adds it, as a deck — and its frame read back for
//! that slot, through the never-waiting read in `render::readback`; a tick whose slot was
//! already read draws it only for the reasons it always would. So a slot holds the first
//! frame drawn in it, a faster show's other frames are skipped, and a slot no tick reached in
//! time is filled by `video::record` with the frame before and counted as dropped. A paused
//! loop is not drawn ([`Synth::drawing`]'s freeze), and its slots repeat the frame before on
//! purpose, counted as nothing. The file is as long as the show was, to the frame.
//!
//! **Nothing here waits.** The read is a request and a later collect, the writer a queue that
//! refuses what it has no room for, and a stop a flag: the reads still on the GPU are taken as
//! they land, then the queue is closed and the writer finishes the file on its own thread,
//! which [`Synth::record_collect`] polls. Only [`Synth::finish_recordings`] waits, and only
//! the run's end calls it.
//!
//! **What ends one**: the Stop button; its Output deleted, its resolution changed — a file is
//! one size — or no longer awake, its tab closed; another project; the app quitting. Each ends
//! the file under its name with what was recorded, and says why where it was not the button.
//! A render and a recording never run at once: a render steps the clock a recording follows.
//! See [docs/rendering.md](../../docs/rendering.md#live-recording).

use super::Synth;
use crate::graph::NodeId;
use crate::render::readback::Recorded as Read;
use crate::video::record::{Picture, Recorder, frames_for, slot_at};
use std::path::PathBuf;

/// How long a stopped recording waits for the reads still on the GPU before it closes the file
/// without them, in seconds of the show's clock.
const DRAIN: f64 = 2.0;

/// A recording, as the editor asks for one.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    pub output: NodeId,
    pub fps: f64,
    pub destination: PathBuf,
    /// Which recording this is, as the editor counts them, carried back on its progress and
    /// its end.
    pub seq: u64,
}

/// A recording that ended with a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub destination: PathBuf,
    /// Frames in the file, and how many of them repeat the frame before for lack of a picture.
    pub frames: u64,
    pub dropped: u64,
    /// How long it ran on the show's clock.
    pub seconds: f64,
    /// Why it stopped, where it was not the Stop button.
    pub why: Option<String>,
}

/// How a recording ended: its file, or why there is none.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordEnd {
    pub output: NodeId,
    pub seq: u64,
    pub result: Result<Recorded, String>,
}

/// Where one recording is, for the row that draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordProgress {
    pub output: NodeId,
    pub seq: u64,
    /// How long it has run on the show's clock.
    pub seconds: f64,
    pub dropped: u64,
    pub destination: PathBuf,
}

/// A recording running, or stopped and waiting for its last reads.
pub(super) struct Recording {
    output: NodeId,
    seq: u64,
    destination: PathBuf,
    size: (u32, u32),
    fps: f64,
    /// The clock on the first tick that asked for a picture, its first slot's start. Not the
    /// tick the Record press arrived on: that tick's clock was read before the encoder took its
    /// time to open, so a recording timed from it would be past its first slot before it could
    /// ask, and open on a black frame.
    start: Option<f64>,
    /// The latest slot a read was asked for.
    entered: Option<u64>,
    /// The slot asked for on this tick, until the draw is over.
    asked: Option<u64>,
    /// Whether this tick's job draws the Output.
    drawn: bool,
    stop: Option<Stop>,
    recorder: Recorder,
}

/// Where a recording stopped: how long its file is, why, and when.
struct Stop {
    frames: u64,
    seconds: f64,
    why: Option<String>,
    at: f64,
}

/// A recording whose writer is closing the file.
pub(super) struct Finishing {
    output: NodeId,
    seq: u64,
    seconds: f64,
    why: Option<String>,
    recorder: Recorder,
}

impl Recording {
    /// The Output this is a recording of, while the job should draw it this tick: a slot was
    /// asked for.
    pub(super) fn asking(&self) -> Option<NodeId> {
        self.asked.map(|_| self.output)
    }

    pub(super) fn output(&self) -> NodeId {
        self.output
    }

    /// The job this tick draws the Output, or not.
    pub(super) fn set_drawn(&mut self, drawn: bool) {
        self.drawn = drawn;
    }

    /// End it at `now` on the show's clock, `why` where it was not the Stop button.
    /// How long it has run at `now`: nothing before its first slot.
    fn seconds(&self, now: f64) -> f64 {
        self.start.map_or(0.0, |start| (now - start).max(0.0))
    }

    fn stop(&mut self, now: f64, why: Option<String>) {
        if self.stop.is_some() {
            return;
        }
        let seconds = self.seconds(now);
        self.stop = Some(Stop {
            frames: frames_for(seconds, self.fps),
            seconds,
            why,
            at: now,
        });
    }

    /// Close the file at its stop's length and hand the writer over to finish it.
    fn close(mut self) -> Finishing {
        let stop = self.stop.take().unwrap_or(Stop {
            frames: 1,
            seconds: 0.0,
            why: None,
            at: 0.0,
        });
        self.recorder.close(stop.frames);
        Finishing {
            output: self.output,
            seq: self.seq,
            seconds: stop.seconds,
            why: stop.why,
            recorder: self.recorder,
        }
    }
}

impl Synth {
    /// Begin recording an Output. A refusal — a render running, no hardware encoder, a folder
    /// that cannot be made — ends it on the spot, said as its end.
    pub(super) fn start_recording(&mut self, request: RecordRequest) {
        let refuse = |synth: &mut Self, why: String| {
            synth.events.recorded.push(RecordEnd {
                output: request.output,
                seq: request.seq,
                result: Err(why),
            });
        };
        if self.offline.is_some() {
            refuse(self, "a render is running".to_string());
            return;
        }
        if self.recordings.iter().any(|r| r.output == request.output) {
            return;
        }
        let Some(node) = self.graph.get(request.output) else {
            refuse(self, "no such Output".to_string());
            return;
        };
        let size = crate::nodes::output::resolution_of(node);
        let fps = request.fps.max(1.0);
        match Recorder::start(&request.destination, size, fps) {
            Ok(recorder) => self.recordings.push(Recording {
                output: request.output,
                seq: request.seq,
                destination: request.destination,
                size,
                fps,
                start: None,
                entered: None,
                asked: None,
                drawn: false,
                stop: None,
                recorder,
            }),
            Err(e) => refuse(self, e),
        }
    }

    /// The Stop button.
    pub(super) fn stop_recording(&mut self, output: NodeId) {
        let now = self.clock.elapsed();
        for r in &mut self.recordings {
            if r.output == output {
                r.stop(now, None);
            }
        }
    }

    /// Before the job: end every recording whose Output can no longer be recorded, and ask
    /// for a read of each one's frame on the first tick of a slot.
    pub(super) fn record_ask(&mut self) {
        let now = self.clock.elapsed();
        for r in &mut self.recordings {
            if r.stop.is_some() {
                continue;
            }
            let why = match self.graph.get(r.output) {
                None => Some("the Output was deleted"),
                Some(node) if crate::nodes::output::resolution_of(node) != r.size => {
                    Some("the resolution changed")
                }
                Some(_)
                    if self
                        .plan
                        .outputs
                        .iter()
                        .any(|o| o.node == r.output && o.mode == super::Mode::Suspended) =>
                {
                    Some("its workspace was closed")
                }
                Some(_) => None,
            };
            if let Some(why) = why {
                r.stop(now, Some(why.to_string()));
                continue;
            }
            let start = *r.start.get_or_insert(now);
            let slot = slot_at(now - start, r.fps);
            if r.entered.is_none_or(|e| slot > e) {
                r.entered = Some(slot);
                r.asked = Some(slot);
                if let Some(renderer) = &mut self.renderer {
                    renderer.request_record(r.output, slot);
                }
            }
        }
    }

    /// After the draw: hand what came back to each writer in slot order, close the files of
    /// those stopped once their last reads are in, and say how each finished file ended.
    pub(super) fn record_collect(&mut self) {
        let now = self.clock.elapsed();
        for r in &mut self.recordings {
            if let Some(slot) = r.asked.take()
                && let Some(renderer) = &mut self.renderer
            {
                renderer.record_done(r.output, slot, !r.drawn);
            }
            let back = self
                .renderer
                .as_mut()
                .map(|renderer| renderer.take_recorded(r.output))
                .unwrap_or_default();
            for (slot, read) in back {
                let picture = match read {
                    Read::Frame(bytes) => Picture::Frame(bytes),
                    Read::Same => Picture::Same,
                };
                r.recorder.offer(slot, picture);
            }
            if let Some(e) = r.recorder.error() {
                r.stop(now, Some(e));
            }
        }
        let mut i = 0;
        while i < self.recordings.len() {
            let r = &self.recordings[i];
            let drained = r.stop.as_ref().is_some_and(|s| {
                now - s.at > DRAIN
                    || !self
                        .renderer
                        .as_ref()
                        .is_some_and(|renderer| renderer.record_pending(r.output))
            });
            if drained {
                let r = self.recordings.swap_remove(i);
                if let Some(renderer) = &mut self.renderer {
                    renderer.forget_record(r.output);
                }
                self.finishing.push(r.close());
            } else {
                i += 1;
            }
        }
        let mut i = 0;
        while i < self.finishing.len() {
            if self.finishing[i].recorder.done() {
                let mut f = self.finishing.swap_remove(i);
                let end = ended(&mut f);
                self.events.recorded.push(end);
            } else {
                i += 1;
            }
        }
    }

    /// Another project: every recording ends now, without the reads still on the GPU, since
    /// the ids it was kept by are about to be the new project's nodes.
    pub(super) fn close_recordings(&mut self, why: &str) {
        let now = self.clock.elapsed();
        for mut r in std::mem::take(&mut self.recordings) {
            if let Some(renderer) = &mut self.renderer {
                renderer.forget_record(r.output);
            }
            r.stop(now, Some(why.to_string()));
            self.finishing.push(r.close());
        }
    }

    /// The run is ending: every recording is closed where it stands and every file waited
    /// for, so quitting leaves films rather than `.part`s.
    pub fn finish_recordings(&mut self) {
        self.close_recordings("the app quit");
        for mut f in std::mem::take(&mut self.finishing) {
            while !f.recorder.done() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let end = ended(&mut f);
            log::info!("{end:?}");
        }
    }

    /// Where each recording is, for the snapshot.
    pub(super) fn record_report(&self) -> Vec<RecordProgress> {
        let now = self.clock.elapsed();
        self.recordings
            .iter()
            .map(|r| RecordProgress {
                output: r.output,
                seq: r.seq,
                seconds: r
                    .stop
                    .as_ref()
                    .map_or_else(|| r.seconds(now), |s| s.seconds),
                dropped: r.recorder.dropped(),
                destination: r.destination.clone(),
            })
            .collect()
    }

    /// Whether anything is recording, which a render refuses to start beside.
    pub(super) fn recording(&self) -> bool {
        !self.recordings.is_empty()
    }
}

/// How a finished writer's file came out.
fn ended(f: &mut Finishing) -> RecordEnd {
    let result = match f.recorder.outcome() {
        Some(Ok(destination)) => Ok(Recorded {
            destination,
            frames: f.recorder.written(),
            dropped: f.recorder.dropped(),
            seconds: f.seconds,
            why: f.why.clone(),
        }),
        Some(Err(e)) => Err(e),
        None => Err("the writer stopped without a word".to_string()),
    };
    RecordEnd {
        output: f.output,
        seq: f.seq,
        result,
    }
}
