// SPDX-License-Identifier: AGPL-3.0-or-later

//! The synth's own thread, its timer, and the two channels across it.
//!
//! **The loop.** One [`Synth::step`] per display interval, from a deadline carried forward
//! rather than from a sleep of the interval: a tick that costs four milliseconds is followed
//! by a sleep of twelve, so the rate is the display's and does not drift with the work. A
//! tick that ends past the next deadline re-anchors it on the wall: the next tick starts at
//! once and the one after an interval later, so no tick is ever run early to make up for a
//! late one. When the GPU is short the draw waits for it inside the tick (see
//! `render::TICKS_IN_FLIGHT`), and the ticks run back to back at the rate the GPU allows,
//! each integrating the wall time it actually took — `Clock::tick` measures `dt` between the
//! starts of two ticks, a wait included.
//!
//! **The rate is the editor's display, or the preference.** The frame thread reads the
//! monitor's refresh every frame and sends [`Msg::Interval`] when it changes; before the
//! first message the loop runs at [`DEFAULT_INTERVAL_MS`]. Nothing about the schedule comes
//! from a window's frame callback — that is the whole point of
//! `proposals/deterministic-loop.md`.
//!
//! **The clock is monotonic.** `Clock::tick` is fed from an `Instant` taken when the thread
//! started, not from egui's input time, because egui's belongs to a loop that stops when a
//! window is minimized.
//!
//! **Inline is the same code.** [`Host::Inline`] holds the synth on the caller's thread and
//! runs the identical drain-step-swap through the identical channel and buffer, so
//! `App::headless` and every layer-1 test execute the path a performance executes. There is
//! no second implementation to keep in step.

use super::{Plan, Snapshot, Synth, Work};
use crate::app::render::RenderSettings;
use crate::graph::{Graph, NodeId, PortRef};
use crate::maininput::MainInput;
use crate::project::AssetPaths;
use crate::render::{Gpu, Live};
use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The rate before the editor has said which display it is on. Sixty hertz.
pub const DEFAULT_INTERVAL_MS: f32 = 1000.0 / 60.0;

/// What the editor asks the synth. Every one of these was a `pub fn` on `Synth`.
pub enum Msg {
    /// The graph, shared: the editor and its undo steps hold the same one, and neither side
    /// writes a graph the other can see — a write copies it first, through `Arc::make_mut`.
    ///
    /// `project` counts the projects opened in this run: a new one drops everything held for
    /// the old one's nodes. See [`super`] on another project.
    Graph {
        graph: Arc<Graph>,
        project: u64,
    },
    /// The frame to draw, as the compiler described it.
    Plan(Box<Plan>),
    /// What the tick is given from the document, sent whenever any of it changes, because
    /// each can change without an edit: a tab opened, the project tab shown, a device chosen.
    Inputs {
        live: HashSet<NodeId>,
        report: bool,
        status: bool,
        assets: AssetPaths,
        main_input: MainInput,
        /// The Main Input panel is unfolded, the only reason its picture is uploaded.
        main_input_preview: bool,
    },
    /// The fade, the crossfade method, Blackout and Freeze, whenever any changes or a hand
    /// moves the fade. `seq` counts them, so the editor knows which one a snapshot has seen:
    /// see [`crate::synth::Snapshot::fade_seq`].
    Fade {
        balance: f32,
        method: crate::mixer::Method,
        blackout: bool,
        freeze: bool,
        /// Which of them a hand moved since the last: its value wins over a write this
        /// thread made that the editor had not seen.
        hands: crate::mixer::Hands,
        seq: u64,
    },
    /// A queue of MIDI messages, handed over once and drained at the top of every tick.
    ///
    /// Several of them: the reader thread's, and the editor's own seam — the one a test
    /// posts a message into with no device on the box. Only one of the two is ever live.
    MidiIn(Receiver<crate::midi::Wire>),
    /// The MIDI window's **Release all**: every action input a note is holding is let go.
    MidiReleaseAll,
    /// The soft takeover preference: a CC that disagrees with its control waits for the
    /// fader to pass it.
    SoftTakeover(bool),
    /// The map, whenever the editor changes a binding: learn, unbind, a range edit, a file
    /// opened. See `App::publish_midi`.
    MidiMap(Box<crate::midi::Bindings>),
    /// A control is waiting for the next message. While it is, no binding is driven — a knob
    /// being taught a control must not also move whatever it moved a moment ago.
    MidiLearn(bool),
    /// Hold or release the button on an action input from any source but the pointer.
    Press(PortRef, bool),
    /// The buttons the pointer is on, as the canvas reports them. Replaced every frame.
    PointerHeld(HashSet<PortRef>),
    /// Where a hand dragged a scrubber, for the next tick to act on.
    Seeks(Vec<(NodeId, f32)>),
    /// What a hand did on a node's own surface — a well dropped on a pad, a preset — for the
    /// next tick to act on.
    Touches(Vec<(NodeId, crate::nodes::cpu::Touch)>),
    /// Every CPU node back to the state it was created in.
    ResetCpu,
    /// Pause, play or seek. See [`crate::transport`].
    Transport(crate::transport::Command),
    /// One tick per this many milliseconds.
    Interval(f32),
    /// *Choose again* on the screen capture.
    ReopenVideo,
    /// *Look for devices*: the panel's audio and camera lists, asked for again.
    ForgetDevices,
    /// A picture of each of these Outputs, read back for a save.
    Thumbnails(Vec<NodeId>),
    /// Run an offline render. The synth owns the clock and the nodes, so it runs the loop.
    StartRender(Box<RenderRequest>),
    CancelRender,
    /// Record an Output live, as the show plays. See [`super::record`].
    StartRecord(Box<super::RecordRequest>),
    /// The Stop button on an Output's recording.
    StopRecord(NodeId),
}

/// An offline render, as the editor asks for one.
pub struct RenderRequest {
    pub output: NodeId,
    pub settings: RenderSettings,
    /// Which render this is. The editor counts them and only believes an outcome carrying
    /// the number it last asked for — an in-flight snapshot still holding the *previous*
    /// render's outcome would otherwise end this one on its first frame.
    pub seq: u64,
}

/// What a step is paced by.
#[derive(Debug, Clone, Copy)]
pub enum Beat {
    /// Live: the monotonic clock says it is this many seconds since the thread started.
    Wall(f64),
    /// Live, inline: this much wall time passed. The clock and the transport move by all of
    /// it, and a stateful node's step is clamped to `transport::MAX_DT` exactly as a live
    /// wall tick's is, so the inline path and the thread's are the same code.
    Delta(f32),
    /// A render's frame, at playhead `t`: the clock is set to `t` and the transport driven
    /// there with the travel between, as `Synth::render_frame` does, and no step is clamped —
    /// a stepped frame is not a stall, and a 5 fps run clamped to `MAX_DT` would integrate at
    /// half speed. What a test that renders without a GPU ticks on; a run starts with a
    /// [`crate::transport::Command::Seek`] to its first frame, as a render does.
    At(f64),
}

/// The double buffer the snapshot crosses in.
///
/// A mutex held for the length of a `mem::swap` and for nothing else: the synth swaps its
/// filled buffer in at the end of a tick and the editor swaps its spent one back at the top
/// of a frame, so neither ever waits on the other's work.
///
/// **The editor only ever moves forward.** The slot is one buffer, so an editor that paints
/// twice between two ticks would, on the second frame, find the spent buffer it left on the
/// first and take it back as if it were news — every reading a tick older than the one it
/// had, then the newer one again on the frame after, and so on until the synth caught up.
/// That was a phase that ran backwards every other frame and an Output preview that
/// alternated between a picture and the black frame before it, whenever the synth fell
/// behind the display. `take` compares [`Snapshot::seq`] and keeps the editor's own buffer
/// when the slot holds nothing newer.
///
/// **An unread buffer comes back to the synth.** `put` hands back whatever the slot held,
/// taken or not, so nothing that must arrive once rides in a buffer: see [`super::events`].
#[derive(Default)]
pub struct Mailbox(Mutex<Box<Snapshot>>);

impl Mailbox {
    /// Leave `filled` for the editor and take back whatever it left.
    fn put(&self, filled: Box<Snapshot>) -> Box<Snapshot> {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *slot, filled)
    }

    /// Take the newest and leave `spent` to be written over — or hand `spent` straight
    /// back when the slot holds nothing newer than it, so the editor never reads backwards.
    ///
    /// **Handing a buffer back acknowledges its events.** The editor applied everything
    /// `spent` carries, so it is marked seen for the synth to forget, and the newest is marked
    /// as seen up to it, so the editor applies only what is past. See [`super::events`].
    fn take(&self, mut spent: Box<Snapshot>) -> Box<Snapshot> {
        spent.events.see_all();
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.seq <= spent.seq {
            return spent;
        }
        let mut newest = std::mem::replace(&mut *slot, spent);
        newest.events.seen_after(&slot.events);
        // The spent buffer waits here until the synth's next tick writes over it, and the
        // pictures it names would hold their targets that whole time. The newest names what
        // the editor now holds anyway.
        slot.published = Arc::clone(&newest.published);
        newest
    }
}

/// Where the synth is running.
pub enum Host {
    /// On the caller's thread: `App::headless`, every layer-1 test, and egui_kittest. The
    /// same channel and the same buffer, drained and swapped by whoever calls `step`.
    Inline {
        synth: Box<Synth>,
        tx: Sender<Msg>,
        rx: Receiver<Msg>,
        mailbox: Arc<Mailbox>,
        live: Arc<Live>,
        pointer: Arc<crate::pointer::Feed>,
    },
    /// On a thread of its own, ticking whether or not anything is painting.
    Thread {
        tx: Sender<Msg>,
        mailbox: Arc<Mailbox>,
        handle: Option<std::thread::JoinHandle<()>>,
        /// The interval last sent, so the rate is sent when it changes and not per frame.
        interval_ms: f32,
        live: Arc<Live>,
        pointer: Arc<crate::pointer::Feed>,
    },
}

impl Default for Host {
    fn default() -> Self {
        Self::inline()
    }
}

impl Host {
    /// A synth on the caller's thread.
    pub fn inline() -> Self {
        let (tx, rx) = channel();
        let live: Arc<Live> = Arc::default();
        let pointer: Arc<crate::pointer::Feed> = Arc::default();
        let mut synth = Box::new(Synth::default());
        synth.publish_into(Arc::clone(&live));
        synth.point_from(Arc::clone(&pointer));
        Self::Inline {
            synth,
            tx,
            rx,
            mailbox: Arc::default(),
            live,
            pointer,
        }
    }

    /// A synth on a thread of its own, drawing on `gpu`.
    ///
    /// The handle is the editor's device, asked of it with [`Gpu::for_synth`], and the
    /// renderer is made on it **on the synth thread**, which draws on it for the life of the
    /// run. `Synth` itself is built there too, so every `CpuNode` is born on the thread that
    /// ticks it and nothing that is not `Send` ever crosses.
    pub fn spawn(gpu: Option<Gpu>) -> Self {
        let (tx, rx) = channel();
        let mailbox: Arc<Mailbox> = Arc::default();
        let live: Arc<Live> = Arc::default();
        let pointer: Arc<crate::pointer::Feed> = Arc::default();
        let (theirs, theirs_live) = (Arc::clone(&mailbox), Arc::clone(&live));
        let theirs_pointer = Arc::clone(&pointer);
        let handle = std::thread::Builder::new()
            .name("synth".into())
            .spawn(move || run(gpu, &rx, &theirs, theirs_live, theirs_pointer))
            .map_err(|e| log::error!("the synth thread could not be started: {e}"))
            .ok();
        Self::Thread {
            tx,
            mailbox,
            handle,
            interval_ms: DEFAULT_INTERVAL_MS,
            live,
            pointer,
        }
    }

    /// The slot the synth writes its newest [`crate::render::Published`] into, for a window
    /// painted outside the editor's own pass.
    ///
    /// **Written by the synth, not by the frame.** The projector is a deferred viewport and
    /// keeps painting while the editor is minimized, which is exactly when no frame runs; a
    /// slot the frame filled would freeze at whatever it last held.
    pub fn live(&self) -> Arc<Live> {
        match self {
            Self::Inline { live, .. } | Self::Thread { live, .. } => Arc::clone(live),
        }
    }

    /// The slot the pointer's sources write into and the tick reads.
    ///
    /// Handed to the pictures thread, which sees a picture window's own `wl_pointer`, and
    /// held by the editor, which sees the preview's and the canvas's through egui.
    pub fn pointer(&self) -> Arc<crate::pointer::Feed> {
        match self {
            Self::Inline { pointer, .. } | Self::Thread { pointer, .. } => Arc::clone(pointer),
        }
    }

    pub fn send(&self, msg: Msg) {
        let tx = match self {
            Self::Inline { tx, .. } | Self::Thread { tx, .. } => tx,
        };
        // A closed channel is a synth thread that stopped, which is a run that is ending.
        let _ = tx.send(msg);
    }

    /// The rate, sent only when it changes: a message a frame is a message a frame.
    pub fn set_interval(&mut self, ms: f32) {
        match self {
            Self::Inline { synth, .. } => synth.handle(Msg::Interval(ms)),
            Self::Thread { interval_ms, .. } => {
                if (*interval_ms - ms).abs() > 0.01 {
                    *interval_ms = ms;
                    self.send(Msg::Interval(ms));
                }
            }
        }
    }

    /// Take the newest snapshot, handing back the one the editor is done with.
    pub fn take(&self, spent: Box<Snapshot>) -> Box<Snapshot> {
        match self {
            Self::Inline { mailbox, .. } | Self::Thread { mailbox, .. } => mailbox.take(spent),
        }
    }

    /// The synth itself, where it is on this thread. `None` under the thread, which is what
    /// makes every other path go through a message.
    pub fn inline_synth(&self) -> Option<&Synth> {
        match self {
            Self::Inline { synth, .. } => Some(synth),
            Self::Thread { .. } => None,
        }
    }

    pub fn inline_synth_mut(&mut self) -> Option<&mut Synth> {
        match self {
            Self::Inline { synth, .. } => Some(synth),
            Self::Thread { .. } => None,
        }
    }

    /// Apply everything waiting, without stepping. Inline only; a no-op under the thread,
    /// which drains at the top of every tick of its own.
    pub fn pump(&mut self) {
        if let Self::Inline { synth, rx, .. } = self {
            drain(synth, rx);
        }
    }

    /// One step on the caller's thread. Does nothing under the thread, which steps itself.
    pub fn step(&mut self, beat: Beat) {
        let Self::Inline {
            synth, rx, mailbox, ..
        } = self
        else {
            return;
        };
        synth.begin_tick();
        drain(synth, rx);
        synth.lap(Work::Inbox);
        synth.step(beat);
        let filled = synth.take_snapshot();
        let spent = mailbox.put(filled);
        synth.put_buffer(spent);
        synth.lap(Work::Snapshot);
    }

    /// Stop the thread and wait for it, so its renderer and every device it holds are freed
    /// before eframe tears its window down.
    pub fn stop(&mut self) {
        match self {
            Self::Inline { synth, .. } => {
                synth.finish_recordings();
                synth.destroy_gpu();
            }
            Self::Thread { tx, handle, .. } => {
                // Dropping the sender is what ends the loop: the receiver disconnects and
                // the thread drops its renderer with everything it holds.
                let (dead, _) = channel();
                *tx = dead;
                if let Some(h) = handle.take() {
                    let _ = h.join();
                }
            }
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Everything waiting, applied in order.
fn drain(synth: &mut Synth, rx: &Receiver<Msg>) -> bool {
    loop {
        match rx.try_recv() {
            Ok(msg) => synth.handle(msg),
            Err(TryRecvError::Empty) => return true,
            Err(TryRecvError::Disconnected) => return false,
        }
    }
}

/// The synth thread: make the renderer on its device, then tick to a deadline for ever.
fn run(
    gpu: Option<Gpu>,
    rx: &Receiver<Msg>,
    mailbox: &Mailbox,
    live: Arc<Live>,
    pointer: Arc<crate::pointer::Feed>,
) {
    let mut synth = Box::new(Synth::default());
    synth.publish_into(live);
    synth.point_from(pointer);
    if let Some(gpu) = gpu
        && let Err(err) = synth.attach_gpu(gpu)
    {
        log::error!("the synth's renderer could not be made: {err}");
    }
    let started = Instant::now();
    let mut deadline = Instant::now();
    loop {
        synth.begin_tick();
        if !drain(&mut synth, rx) {
            break;
        }
        synth.lap(Work::Inbox);
        synth.step(Beat::Wall(started.elapsed().as_secs_f64()));
        let filled = synth.take_snapshot();
        synth.put_buffer(mailbox.put(filled));
        synth.lap(Work::Snapshot);

        let interval = Duration::from_secs_f32(synth.interval_ms() / 1000.0);
        deadline += interval;
        let now = Instant::now();
        if now >= deadline {
            // Late: the next tick starts now, and the one after an interval from it. Nothing
            // is caught up — `Clock::elapsed` is unclamped, so the world's time loses nothing
            // by a tick not run.
            deadline = now;
            continue;
        }
        std::thread::sleep(deadline - now);
        synth.slept(now.elapsed());
    }
    // The run is over, so its recordings may be waited for: each file is closed and renamed.
    synth.finish_recordings();
    synth.destroy_gpu();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(seq: u64) -> Box<Snapshot> {
        Box::new(Snapshot {
            seq,
            ..Snapshot::default()
        })
    }

    /// The editor paints twice between two ticks: the second frame reads the same snapshot
    /// as the first, never the one before it.
    #[test]
    fn the_editor_never_reads_backwards() {
        let mailbox = Mailbox::default();
        mailbox.put(snapshot(1));
        let held = mailbox.take(snapshot(0));
        assert_eq!(held.seq, 1);
        // Nothing new has been put: the editor keeps what it has, and the slot is untouched.
        let held = mailbox.take(held);
        assert_eq!(held.seq, 1);
        let held = mailbox.take(held);
        assert_eq!(held.seq, 1);
        // The synth ticks twice before the editor looks again: it reads the newest.
        let spare = mailbox.put(snapshot(2));
        assert_eq!(
            spare.seq, 0,
            "the synth takes back the buffer the editor left"
        );
        mailbox.put(snapshot(3));
        let held = mailbox.take(held);
        assert_eq!(held.seq, 3);
    }

    /// One event of every kind, each carrying `n`, pushed as a tick would push it.
    fn make_events(host: &mut Host, n: u32) {
        use crate::graph::Value;
        use crate::midi::{Kind, Message};
        let events = &mut host.inline_synth_mut().expect("inline").events;
        events.decks.push((NodeId(n), crate::mixer::Channel::A));
        events.fired.push(vec![PortRef::new(NodeId(n), "trigger")]);
        events
            .values
            .push((NodeId(1), "recording", Value::Text(n.to_string())));
        events.probes.push(vec![(NodeId(1), vec![n])]);
        events.thumbnails.push((NodeId(n), Vec::new()));
        events.snaps.push((NodeId(n), 1, 1, Vec::new()));
        events.midi.push(Message {
            channel: 0,
            kind: Kind::Control {
                cc: 1,
                value: u8::try_from(n).expect("under 128"),
            },
        });
    }

    /// What the editor applies from one snapshot: every kind's fresh entries, by their `n`.
    fn applied(snapshot: &Snapshot) -> [Vec<u32>; 7] {
        use crate::graph::Value;
        use crate::midi::Kind;
        let e = &snapshot.events;
        let n = |id: &NodeId| id.0;
        [
            e.decks.fresh().unwrap().map(|(_, d)| n(&d.0)).collect(),
            e.fired
                .fresh()
                .unwrap()
                .map(|(_, f)| n(&f[0].node))
                .collect(),
            (e.values.fresh().unwrap())
                .map(|(_, v)| match &v.2 {
                    Value::Text(t) => t.parse().unwrap(),
                    _ => unreachable!(),
                })
                .collect(),
            e.probes.fresh().unwrap().map(|(_, p)| p[0].1[0]).collect(),
            e.thumbnails
                .fresh()
                .unwrap()
                .map(|(_, t)| n(&t.0))
                .collect(),
            e.snaps.fresh().unwrap().map(|(_, s)| n(&s.0)).collect(),
            (e.midi.fresh().unwrap())
                .map(|(_, m)| match m.kind {
                    Kind::Control { value, .. } => u32::from(value),
                    Kind::Note { .. } => unreachable!(),
                })
                .collect(),
        ]
    }

    /// **Every event reaches the editor once, in the order the synth made it,** however many
    /// ticks it missed — an odd number or an even one. The review's case: the synth ticks
    /// twice unread, the editor takes, the synth ticks, the editor takes; a buffer the editor
    /// never took comes back to the synth and goes out again under a newer `seq`, and must
    /// not carry the older events behind the newer ones.
    #[test]
    fn every_event_arrives_once_and_in_order() {
        for missed in 1..=5 {
            let mut host = Host::inline();
            let mut held = Box::<Snapshot>::default();
            let mut got: [Vec<u32>; 7] = Default::default();
            let mut made = 0;
            let step = |host: &mut Host, made: &mut u32| {
                *made += 1;
                make_events(host, *made);
                host.step(Beat::Delta(1.0 / 60.0));
            };
            for _ in 0..missed {
                step(&mut host, &mut made);
            }
            for _ in 0..3 {
                held = host.take(held);
                for (kind, fresh) in got.iter_mut().zip(applied(&held)) {
                    kind.extend(fresh);
                }
                held = host.take(held);
                assert!(
                    applied(&held).iter().all(Vec::is_empty),
                    "a snapshot read twice yields nothing twice"
                );
                step(&mut host, &mut made);
            }
            // The last tick is still unread; this take is the one that sees it.
            held = host.take(held);
            for (kind, fresh) in got.iter_mut().zip(applied(&held)) {
                kind.extend(fresh);
            }
            let all: Vec<u32> = (1..=made).collect();
            for (i, kind) in got.iter().enumerate() {
                assert_eq!(kind, &all, "kind {i}, {missed} ticks missed");
            }
            // What the editor handed back is forgotten, so the synth's logs stay short.
            step(&mut host, &mut made);
            host.take(held);
            step(&mut host, &mut made);
            let events = &host.inline_synth().expect("inline").events;
            assert!(
                events.decks.held() <= 2,
                "the synth keeps only what the editor has not handed back: {}",
                events.decks.held()
            );
        }
    }

    /// An editor away for longer than a log keeps is told how many it missed, rather than
    /// handed the part that is left.
    #[test]
    fn an_overrun_says_how_many_were_missed() {
        let mut host = Host::inline();
        for n in 0..=super::super::events::SNAPS {
            host.inline_synth_mut().expect("inline").events.snaps.push((
                NodeId(u32::try_from(n).expect("small")),
                1,
                1,
                Vec::new(),
            ));
            host.step(Beat::Delta(1.0 / 60.0));
        }
        let held = host.take(Box::default());
        assert_eq!(
            held.events.snaps.fresh().err(),
            Some(super::super::events::SNAPS as u64 + 1)
        );
        assert_eq!(held.events.snaps.unseen("snaps").count(), 0);
        // And once said, the editor is level with the synth again.
        host.inline_synth_mut()
            .expect("inline")
            .events
            .snaps
            .push((NodeId(99), 1, 1, Vec::new()));
        host.step(Beat::Delta(1.0 / 60.0));
        let held = host.take(held);
        let fresh: Vec<u32> = (held.events.snaps.fresh().expect("level"))
            .map(|(_, s)| s.0.0)
            .collect();
        assert_eq!(fresh, [99]);
    }

    fn inputs(status: bool) -> Msg {
        Msg::Inputs {
            live: HashSet::new(),
            report: false,
            status,
            assets: AssetPaths::default(),
            main_input: MainInput::default(),
            main_input_preview: false,
        }
    }

    /// With the Status box open, a snapshot carries the last whole tick by phase, and the
    /// phases add up to it; closed, it carries none.
    #[test]
    fn a_ticks_phases_add_up_to_the_tick() {
        let mut host = Host::inline();
        host.send(inputs(true));
        let mut held = Box::<Snapshot>::default();
        for _ in 0..4 {
            host.step(Beat::Delta(1.0 / 60.0));
            std::thread::sleep(Duration::from_millis(2));
            held = host.take(held);
        }
        let phases = held.phases.as_ref().expect("the box is open");
        assert!(phases.tick.now >= 2.0, "the sleep is in the tick");
        // On the thread's own CPU clock the sleep is not work, so it is waiting: the work,
        // the waiting and the sleep to the deadline add up to the tick all the same.
        let split =
            phases.waiting.now + phases.asleep.now + phases.work.iter().map(|r| r.now).sum::<f32>();
        assert!(
            (split - phases.tick.now).abs() < 1e-3,
            "work, waiting and sleep add up to the tick: {split} against {}",
            phases.tick.now
        );
        assert!(
            phases.waiting.now >= 1.5,
            "the test's own sleep is off the CPU"
        );

        host.send(inputs(false));
        host.step(Beat::Delta(1.0 / 60.0));
        host.step(Beat::Delta(1.0 / 60.0));
        let held = host.take(held);
        assert!(held.phases.is_none(), "closed, nothing is measured");
    }
}
