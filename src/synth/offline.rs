// SPDX-License-Identifier: AGPL-3.0-or-later

//! An offline render: the Output rendered to a PNG sequence or a video, one frame per frame,
//! with time a function of the frame index and nothing dropped.
//!
//! **It runs on the synth thread**, because everything it drives is the synth's: the clock,
//! every `CpuNode`, the renderer and the context the frames are drawn on. The editor asks for
//! one with [`super::Msg::StartRender`], watches it through the snapshot, and stops it with
//! [`super::Msg::CancelRender`]; `app/render.rs` is what reads the settings off the node and
//! what draws the banner.
//!
//! Per tick, in place of the live beat: [`Synth::render_frame`] takes the frames the renderer
//! has read back and hands them to the writer, then drives the clock and the transport's
//! playhead to the next frame, `T = (n − warm) / fps`, and ticks the graph — the open workspaces, exactly what live play ticks — and draws it. A step
//! happens only once the previous one has been drawn, since the count of captures the
//! renderer has issued is what tells a drawn frame from a stepped one, and only while the
//! writer has room, which is the back-pressure that slows the stepper rather than losing a
//! frame. The document is closed for the length of it: `App::apply` refuses every command.
//! See [docs/rendering.md](../../docs/rendering.md#the-render-job).

use super::Synth;
use crate::app::render::{Format, Outcome, RenderSettings};
use crate::clock::{Clock, Stepper, Warmup};
use crate::graph::NodeId;
use crate::video::png;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How many frames may wait for the writer before the stepper holds.
const WRITER_DEPTH: usize = 8;

/// How many ticks a render may go without the Output drawing before it is given up on: an
/// Output that never draws would otherwise hold the document closed for good.
const HOLD_LIMIT: u32 = 600;

/// How long a render holds one frame for a clip's decoder before it draws what has arrived.
/// A frame of an all-intra clip is milliseconds; this is a decoder that has stopped.
const WAIT_LIMIT: Duration = Duration::from_secs(5);

/// How long a render waits for the program it draws with — the Output's program to link —
/// before it is given up on.
const LINK_LIMIT: Duration = Duration::from_secs(60);

/// The render in progress.
pub struct Render {
    output: NodeId,
    /// The editor's number for this render, carried out on the outcome.
    seq: u64,
    stepper: Stepper,
    /// Where the frames are going, so the outcome can say so.
    destination: PathBuf,
    /// The Output's size at the start, which every captured frame must be. The film's size,
    /// not the size it is drawn at: a supersampled render draws larger and the renderer
    /// brings each frame back down to this before it is read back.
    size: (u32, u32),
    /// How much larger than `size` every frame is drawn: 1, 2 or 4.
    scale: u32,
    /// The live clock, put back when the render ends.
    live: Clock,
    /// The live show's transport, put back whole when the render ends.
    transport: crate::transport::Transport,
    /// Every CPU node's live instance the render runs a fresh one in place of, and where each
    /// node last read the transport, put back when the render ends: the live show carries on
    /// as the render found it.
    parked: HashMap<NodeId, (&'static str, Box<dyn crate::nodes::CpuNode>)>,
    seen: HashMap<NodeId, crate::transport::Seen>,
    /// Where the Main Input's file was, put back with it.
    live_position: f64,
    /// The frames the live show's CPU nodes had published, and what its taps last measured,
    /// put back with them.
    frames: HashMap<crate::graph::PortRef, std::sync::Arc<crate::nodes::Frame>>,
    readbacks: HashMap<NodeId, [u32; crate::compile::TAP_WORDS]>,
    /// Frames stepped, and captures the renderer had issued at the last collect.
    stepped: u32,
    issued: u64,
    /// Captured frames received, warm-up included.
    captured: u32,
    /// The Output's drops when the capture began: one more is a frame the film is missing.
    drops_at_start: u64,
    capturing: bool,
    /// Ticks in a row that neither stepped nor collected.
    holds: u32,
    /// The frame stepped last is held for a clip's decoder, since this moment: it is ticked
    /// again where it is, and drawn once every clip has the frame it asked for.
    waiting: Option<Instant>,
    /// No frame is stepped while the program it would be drawn with is still on its way,
    /// since this moment: see `Synth::program_ready`.
    linking: Option<Instant>,
    canceled: bool,
    writer: Writer,
}

impl Render {
    /// Does the next job blank the Output first: a black warm-up, before anything is drawn.
    pub fn wants_clear(&self) -> bool {
        self.stepper.warmup() == Warmup::Black && self.issued == 0
    }

    /// Is there nothing to draw: the last frame stepped has been drawn, or none has. A job
    /// built now suspends every Output, so a tick that stepped nothing — the writer was
    /// full, the run is over — draws nothing, and a feedback ring advances once per frame of
    /// the film rather than once per tick.
    pub fn holding(&self) -> bool {
        self.issued >= u64::from(self.stepped)
    }

    pub fn cancel(&mut self) {
        self.canceled = true;
    }

    pub fn output(&self) -> NodeId {
        self.output
    }

    /// The supersampling multiplier, which is what the Output is drawn at for the length of
    /// the render and what its capture divides back out.
    pub fn scale(&self) -> u32 {
        self.scale
    }
}

/// A thread writing the frames — PNGs, a video through the hardware encoder, or a GIF — so
/// no encode is ever on the synth thread either.
struct Writer {
    tx: Option<SyncSender<(u32, Vec<u8>)>>,
    handle: Option<std::thread::JoinHandle<()>>,
    written: Arc<AtomicU32>,
    error: Arc<Mutex<Option<String>>>,
    /// Frames sent and not yet written.
    sent: u32,
}

impl Writer {
    fn spawn(
        destination: PathBuf,
        size: (u32, u32),
        fps: f64,
        format: Format,
    ) -> Result<Self, String> {
        // Opened before the first frame, so a machine with no encoder refuses the render
        // rather than failing it a frame in.
        let mut encoder = match format {
            Format::PngSequence => {
                std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
                None
            }
            Format::Video => Some(File::Video(crate::video::encode::Encoder::start(
                &destination,
                size.0,
                size.1,
                fps,
            )?)),
            Format::Gif => Some(File::Gif(crate::video::gif::Writer::start(
                &destination,
                size.0,
                size.1,
                fps,
            )?)),
        };
        let (tx, rx) = sync_channel::<(u32, Vec<u8>)>(WRITER_DEPTH);
        let written = Arc::new(AtomicU32::new(0));
        let error = Arc::new(Mutex::new(None));
        let (count, failure) = (Arc::clone(&written), Arc::clone(&error));
        let handle = std::thread::Builder::new()
            .name("render-writer".into())
            .spawn(move || {
                let fail = |what: String| {
                    *failure
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(what);
                };
                for (index, rgba) in rx {
                    let result = if let Some(encoder) = &mut encoder {
                        encoder.push(&rgba)
                    } else {
                        let image = png::Image {
                            width: size.0,
                            height: size.1,
                            rgba,
                        };
                        let path = destination.join(format!("{index:05}.png"));
                        png::write(&path, &image).map_err(|e| format!("{}: {e}", path.display()))
                    };
                    if let Err(e) = result {
                        fail(e);
                        return;
                    }
                    count.fetch_add(1, Ordering::Release);
                }
                // The channel closed: the render is over, or canceled, and what was pushed
                // is closed into a file either way.
                if let Some(encoder) = encoder.take()
                    && let Err(e) = encoder.finish()
                {
                    fail(e);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx: Some(tx),
            handle: Some(handle),
            written,
            error,
            sent: 0,
        })
    }

    fn written(&self) -> u32 {
        self.written.load(Ordering::Acquire)
    }

    fn error(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Frames handed over and not yet on disk.
    fn backlog(&self) -> usize {
        (self.sent - self.written()) as usize
    }

    fn send(&mut self, index: u32, rgba: Vec<u8>) -> Result<(), String> {
        let Some(tx) = &self.tx else {
            return Err("the writer is closed".to_string());
        };
        match tx.try_send((index, rgba)) {
            Ok(()) => {
                self.sent += 1;
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err("the writer fell behind its backlog".to_string()),
            Err(TrySendError::Disconnected(_)) => Err(self
                .error()
                .unwrap_or_else(|| "the writer stopped".to_string())),
        }
    }

    /// Close the channel and wait for what is queued: at most `WRITER_DEPTH` frames.
    fn finish(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// A single-file writer: a video or a GIF.
enum File {
    Video(crate::video::encode::Encoder),
    Gif(crate::video::gif::Writer),
}

impl File {
    fn push(&mut self, rgba: &[u8]) -> Result<(), String> {
        match self {
            Self::Video(e) => e.push(rgba),
            Self::Gif(g) => g.push(rgba),
        }
    }

    fn finish(self) -> Result<PathBuf, String> {
        match self {
            Self::Video(e) => e.finish(),
            Self::Gif(g) => g.finish(),
        }
    }
}

impl Synth {
    /// Begin a render. Every CPU node is reset and the live clock is put aside.
    pub(super) fn start_render(&mut self, output: NodeId, settings: &RenderSettings, seq: u64) {
        if self.offline.is_some() {
            return;
        }
        let size = self
            .graph
            .get(output)
            .map_or(crate::nodes::output::DEFAULT_RESOLUTION, |n| {
                crate::nodes::output::resolution_of(n)
            });
        let writer = match Writer::spawn(
            settings.destination.clone(),
            size,
            settings.fps,
            settings.format,
        ) {
            Ok(w) => w,
            Err(e) => {
                self.outcome = Some((seq, Outcome::Failed(e)));
                return;
            }
        };
        // 1, 2 or 4, and never a size the Output's own resolution does not divide by: the
        // film is `size` and the capture brings the drawn frame down to exactly that.
        let scale = match settings.supersample {
            2 if size.0.is_multiple_of(2) && size.1.is_multiple_of(2) => 2,
            4 if size.0.is_multiple_of(4) && size.1.is_multiple_of(4) => 4,
            _ => 1,
        };
        let parked = self.set_aside_cpu();
        // And what the live show reads next off the GPU: every world a node steps there, which
        // the render grows afresh from each node's seed as it does the nodes' own state, and
        // every Output's latest frame, which feedback reads and the render draws over. With
        // them, the frames the CPU nodes published and what the taps last measured.
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.park_live();
        }
        let frames = self.frames.clone();
        let readbacks = self.readbacks.clone();
        let seen = self.seen.clone();
        self.reset_cpu();
        let live = std::mem::take(&mut self.clock);
        let live_position = self.main_input.position();
        let stepper = Stepper::new(settings.fps, settings.frames, settings.warmup);
        // The first frame's time, as a seek: every gear is born again on the tick it lands,
        // where the playhead puts it, so each one is at the start of its cycle on the first
        // kept frame, at playhead zero.
        let transport = self.transport.start_render(stepper.time_of(0));
        self.offline = Some(Render {
            output,
            seq,
            stepper,
            destination: settings.destination.clone(),
            size,
            scale,
            live,
            transport,
            parked,
            seen,
            live_position,
            frames,
            readbacks,
            stepped: 0,
            issued: 0,
            captured: 0,
            drops_at_start: 0,
            capturing: false,
            holds: 0,
            waiting: None,
            linking: None,
            canceled: false,
            writer,
        });
        self.outcome = None;
    }

    /// One tick of a render: collect, step, draw. Called by [`Synth::step`] in place of the
    /// live beat for as long as one is running.
    pub(super) fn render_frame(&mut self) {
        self.render_collect();
        self.render_step();
        // Always, whether or not a frame was stepped: a job built while the render holds
        // suspends every Output, so a tick that stepped nothing draws nothing — but the
        // renderer still allocates the Output's targets from the job, and `set_capturing`
        // is what waits on those. A render that never drew would otherwise never start.
        self.render();
        self.publish();
    }

    /// Take what the renderer has read back, hand it to the writer, and end the render when
    /// everything is on disk. Before the first step, this is also what turns the capture on.
    fn render_collect(&mut self) {
        let Some(r) = &mut self.offline else { return };
        let output = r.output;
        if self.renderer.is_none() {
            // Nothing will ever draw it. A cancel still lands, and a render nothing can draw
            // is given up on rather than holding the document closed for good.
            let Some(r) = &mut self.offline else { return };
            if r.canceled {
                self.finish_render(Outcome::Canceled);
                return;
            }
            r.holds += 1;
            if r.holds > HOLD_LIMIT {
                self.finish_render(Outcome::Failed("no GPU to render with".to_string()));
            }
            return;
        }
        if r.canceled {
            self.set_capturing(output, false);
            self.finish_render(Outcome::Canceled);
            return;
        }
        if let Some(e) = r.writer.error() {
            self.set_capturing(output, false);
            self.finish_render(Outcome::Failed(e));
            return;
        }
        if !r.capturing && r.stepped < r.stepper.len() {
            if self.set_capturing(output, true) {
                let dropped = self.dropped(output);
                let Some(r) = &mut self.offline else { return };
                r.capturing = true;
                r.drops_at_start = dropped;
            } else {
                let Some(r) = &mut self.offline else { return };
                r.holds += 1;
                if r.holds > HOLD_LIMIT {
                    self.finish_render(Outcome::Failed("the Output never drew".to_string()));
                }
                return;
            }
        }
        let issued = self
            .renderer
            .as_ref()
            .map_or(0, |rr| rr.captures_issued(output));
        let dropped = self.dropped(output);
        let Some(r) = &mut self.offline else { return };
        // Only a frame stepped and not drawn is the Output holding the render up. A tick
        // that stepped nothing because the writer is full — a GIF quantizing, a slow disk —
        // or because every frame is drawn and the writer is finishing is the writer's time,
        // and it has no limit; a frame held for a clip is not stepped until it is drawn, and
        // its own limit is `WAIT_LIMIT`.
        if issued == r.issued
            && r.stepped > 0
            && issued < u64::from(r.stepped)
            && r.waiting.is_none()
        {
            r.holds += 1;
        } else {
            r.holds = 0;
        }
        r.issued = issued;
        if r.capturing && dropped > r.drops_at_start {
            self.set_capturing(output, false);
            self.finish_render(Outcome::Failed(
                "the GPU did not finish a frame in time".to_string(),
            ));
            return;
        }
        if r.holds > HOLD_LIMIT {
            self.set_capturing(output, false);
            self.finish_render(Outcome::Failed("the Output stopped drawing".to_string()));
            return;
        }
        // Every frame has been drawn: nothing more is wanted from the GPU, and the frames
        // any further draws would read are not the film's.
        if r.capturing && r.issued >= u64::from(r.stepper.len()) {
            self.set_capturing(output, false);
            if let Some(r) = &mut self.offline {
                r.capturing = false;
            }
        }
        let captured = self
            .renderer
            .as_mut()
            .map(|rr| rr.take_captured(output))
            .unwrap_or_default();
        for bytes in captured {
            let Some(r) = &mut self.offline else { return };
            let i = r.captured;
            r.captured += 1;
            if i >= r.stepper.len() {
                continue;
            }
            let Some(kept) = r.stepper.kept_index(i) else {
                continue;
            };
            if bytes.len() != (r.size.0 * r.size.1 * 4) as usize {
                self.finish_render(Outcome::Failed(format!(
                    "frame {kept} came back the wrong size"
                )));
                return;
            }
            if let Err(e) = r.writer.send(kept, bytes) {
                self.set_capturing(output, false);
                self.finish_render(Outcome::Failed(e));
                return;
            }
        }
        let Some(r) = &self.offline else { return };
        let film = r.stepper.frames();
        if r.captured >= r.stepper.len() && r.writer.written() >= film {
            let destination = r.destination.clone();
            self.finish_render(Outcome::Done {
                frames: film,
                destination,
            });
        }
    }

    fn set_capturing(&mut self, output: NodeId, on: bool) -> bool {
        let scale = self.offline.as_ref().map_or(1, Render::scale);
        self.renderer
            .as_mut()
            .is_some_and(|r| r.set_capturing(output, on, scale))
    }

    fn dropped(&self, output: NodeId) -> u64 {
        self.renderer
            .as_ref()
            .map_or(0, |r| r.dropped_frames(output))
    }

    /// Drive the clock to the next frame and tick the graph, if the last frame stepped has
    /// been drawn and the writer has room. Returns whether a frame was stepped.
    fn render_step(&mut self) -> bool {
        let Some(r) = &mut self.offline else {
            return false;
        };
        if !r.capturing
            || r.canceled
            || r.stepped >= r.stepper.len()
            || r.issued < u64::from(r.stepped)
        {
            return false;
        }
        // The writer's room gates a new frame and the drawing of a held one, never the
        // re-tick that lets a held clip's decoder deliver: the wait's five seconds are the
        // decoder's, not the writer's.
        let full = r.writer.backlog() >= WRITER_DEPTH;
        let t = r.stepper.time_of(r.stepped);
        let held = r.waiting;
        if let Some(since) = held {
            // The frame is held for a clip: only what is waiting ticks again, with the
            // transport where it was, so it sees no advance and asks for the same frame, and
            // nothing else takes a second step.
            let waiting: HashSet<NodeId> = self.waiting().into_iter().collect();
            self.tick_only(f32::INFINITY, Some(&waiting));
            if (!self.waiting().is_empty() || self.main_input.waiting())
                && since.elapsed() < WAIT_LIMIT
            {
                return false;
            }
            if full {
                return false;
            }
        } else {
            if full {
                return false;
            }
            if !self.program_ready() {
                let Some(r) = &mut self.offline else {
                    return false;
                };
                if r.linking.get_or_insert_with(Instant::now).elapsed() > LINK_LIMIT {
                    let output = r.output;
                    self.set_capturing(output, false);
                    self.finish_render(Outcome::Failed(
                        "the Output's program never linked".to_string(),
                    ));
                }
                return false;
            }
            if let Some(r) = &mut self.offline {
                r.linking = None;
            }
            self.clock.set_elapsed(t);
            // The playhead at the frame's own time and each frame's advance exactly the step
            // between: a constant speed telescopes to `speed × T` at any frame rate.
            self.transport.drive(t, t);
            // The Main Input's clip and sound file sit at the frame's own time, not where live
            // play left them: an audio-reactive patch renders against the file, exactly.
            self.main_input.drive(Some(t));
            // A stepped frame is not a stall: no step is clamped.
            self.tick(f32::INFINITY);
            if !self.waiting().is_empty() || self.main_input.waiting() {
                // A clip asked for a frame its decoder has not delivered: live play would
                // show what it has, and a render waits for the frame the position names.
                if let Some(r) = &mut self.offline {
                    r.waiting = Some(Instant::now());
                }
                return false;
            }
        }
        let Some(r) = &mut self.offline else {
            return false;
        };
        r.waiting = None;
        r.stepped += 1;
        true
    }

    /// Whether the next frame would be drawn with the program the plan names: no program
    /// still crossing to, or linking for, the Output or any Output it reads. A render's hold
    /// hands a waiting program over without drawing (`Synth::job`), and a link in flight
    /// would otherwise draw the program it replaces into the film.
    fn program_ready(&self) -> bool {
        let Some(r) = &self.offline else {
            return true;
        };
        let Some(plan) = self.plan.outputs.iter().find(|o| o.node == r.output) else {
            return true;
        };
        std::iter::once(r.output)
            .chain(plan.needs.iter().copied())
            .all(|id| {
                let crossing = self.plan.outputs.iter().any(|o| {
                    o.node == id
                        && o.shader.is_some()
                        && matches!(o.mode, super::Mode::Idle | super::Mode::Draw { .. })
                });
                let linking = self.renderer.as_ref().is_some_and(|rr| rr.is_linking(id));
                !crossing && !linking
            })
    }

    /// Set every CPU node's live instance aside and put a fresh one in its place, for a render
    /// to run: all but a device's and those that reset where they are
    /// ([`crate::nodes::CpuNode::reset_in_place`]), which [`Synth::reset_cpu`] resets.
    fn set_aside_cpu(&mut self) -> HashMap<NodeId, (&'static str, Box<dyn crate::nodes::CpuNode>)> {
        let mut parked = HashMap::new();
        for (id, (slug, state)) in &mut self.cpu {
            let Some(cpu) = self.graph.get(*id).and_then(|n| n.def.cpu.as_ref()) else {
                continue;
            };
            if cpu.live || state.reset_in_place() {
                continue;
            }
            parked.insert(*id, (*slug, std::mem::replace(state, (cpu.create)())));
        }
        parked
    }

    fn finish_render(&mut self, outcome: Outcome) {
        let Some(mut r) = self.offline.take() else {
            return;
        };
        let seq = r.seq;
        r.writer.finish();
        // The live clock was put aside at the start of the render and its `last` is still
        // the wall time it was put aside at. Restoring it as it stands would hand the first
        // live tick an interval the length of the whole render, and the transport would play
        // the show on by that much. So it is re-anchored to now, which is what `set_elapsed`
        // already does for the other direction.
        let mut live = r.live;
        live.reanchor();
        // The live show back as the render found it: every node it set aside, where each node
        // last read the transport, and the transport itself, so nothing sees a jump — a held
        // gear is held where it was, a pad has its wells, an animation runs on.
        for (id, (slug, state)) in r.parked {
            if let Some(entry) = self.cpu.get_mut(&id)
                && entry.0 == slug
            {
                entry.1 = state;
            }
        }
        // And what it read off the GPU: each simulation's world, so a slime mold grows on from
        // where it was, and each Output's frame, so feedback builds on the frame it left; the
        // frames the nodes published and what the taps last measured.
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.restore_live();
        }
        self.frames = r.frames;
        self.readbacks = r.readbacks;
        self.seen = r.seen;
        self.transport.resume(r.transport, live.elapsed());
        self.clock = live;
        self.main_input.drive(None);
        self.main_input.set_position(r.live_position);
        // Put back where live play had it, so the return's jump is not added to it as well.
        self.main_input_seen = None;
        self.outcome = Some((seq, outcome));
    }

    /// Where the render is, for the snapshot.
    pub(super) fn offline_report(&self) -> super::snapshot::RenderState {
        super::snapshot::RenderState {
            running: self
                .offline
                .as_ref()
                .map(|r| super::snapshot::RenderProgress {
                    output: r.output,
                    written: r.writer.written(),
                    frames: r.stepper.frames(),
                    destination: r.destination.clone(),
                }),
            outcome: self.outcome.clone(),
        }
    }
}

/// Keeps the module's one public type reachable from the snapshot's `Debug`.
impl std::fmt::Debug for Render {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Render")
            .field("output", &self.output)
            .field("stepped", &self.stepped)
            .field("captured", &self.captured)
            .finish_non_exhaustive()
    }
}
