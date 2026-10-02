// SPDX-License-Identifier: AGPL-3.0-or-later

//! The transport: a playhead in seconds over the one clock, which plays, pauses and seeks.
//! It is **ambient time**, the one clock of the show: every node that moves with time reads
//! it at a rate of its own unless a gear is cabled into its Time.
//!
//! It is a coordinate system over [`crate::clock::Clock`], not a second timer. It keeps an
//! anchor — a playhead and the clock's `elapsed` at that playhead — and reads
//!
//! ```text
//! T = T_anchor + playing × (elapsed − elapsed_anchor)
//! ```
//!
//! re-anchoring on every play, pause and seek, the way Ableton Link, Tidal and
//! SuperCollider's TempoClock keep a position. Nothing else keeps a timer. There is no speed
//! and no loop: a rate is a gear's, and a loop is a property of a gear chain, which a Master
//! Gear's caption reads (`nodes::chain`).
//!
//! **Travel is what gears integrate.** Beside the playhead it keeps `travel`, the total
//! distance the playhead has moved, carrying a seek's jump as a jump. Two readings of it a
//! tick apart are how far the show moved in that tick, which is what a gear integrates
//! (`nodes::phasor`), and two readings a minute apart are how far it moved while a node slept
//! on a closed tab. A seek is also counted, so a node can tell a jump inside its advance from
//! a motion. See `docs/cpu.md#the-transport`.
//!
//! **A render drives it.** [`Transport::drive`] puts the playhead at a frame's own time with
//! the travel between, and [`Transport::resume`] hands it back to the clock afterwards as the
//! live show left it, travel and seeks and all, so a node handed back its own reading of it
//! sees no jump. `proposals/time.md` is the argument.

/// The longest step a stateful node takes in one tick, in seconds.
///
/// A simulation, a slew, an envelope or a pad's physics takes the transport's advance as its
/// `dt`, clamped to this, and zero on a jump: a 300 ms stall or a tab reopened after a minute
/// is at most this much of a step, where a gear integrates the whole of it. Only a live tick
/// clamps; a render's frame is a step of exactly its own length.
pub const MAX_DT: f32 = 0.1;

/// What the editor asks of the transport. Each is a hand on the instrument rather than an
/// edit: none enters the undo history, and none is saved — a project opens playing, at zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Play,
    Pause,
    /// The playhead to this many seconds. The time readout's reset is a seek to zero.
    Seek(f64),
}

/// The transport as the editor reads it, once a tick on the snapshot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    /// Where the playhead is, in seconds.
    pub playhead: f64,
    /// The show is playing rather than paused.
    pub playing: bool,
    /// A render owns the playhead.
    pub rendering: bool,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            playhead: 0.0,
            playing: true,
            rendering: false,
        }
    }
}

/// What one node sees of the transport on one tick. [`crate::nodes::TickContext::time`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Time {
    /// The playhead, in seconds: ambient time. It jumps on a seek.
    pub playhead: f64,
    /// How far the playhead moved since **this node** last ticked, in seconds: the jump's
    /// distance on a seek, zero while paused, and the whole gap for a node waking on a
    /// reopened tab.
    pub advance: f64,
    /// `advance` holds something nothing played through: a seek, a render's start or its
    /// return, or this node sleeping on a closed tab. Nothing inside it fires, a stateful
    /// node steps zero, and a gear is born again where the playhead puts it.
    pub jumped: bool,
    /// Where a seek inside `advance` put the playhead, before the motion played since it:
    /// the playhead less that motion. `None` where the advance holds no seek, or where this
    /// node slept through one on a closed tab.
    pub landed: Option<f64>,
    pub playing: bool,
}

impl Default for Time {
    fn default() -> Self {
        Self {
            playhead: 0.0,
            advance: 0.0,
            jumped: false,
            landed: None,
            playing: true,
        }
    }
}

impl Time {
    /// This tick with a jump's advance taken out: what a node with a start of its own
    /// integrates — `animation`, `automation`'s playback — so a seek carries it across where
    /// it stood rather than running it by the jump.
    #[must_use]
    pub fn carried(&self) -> Self {
        if self.jumped {
            Self {
                advance: 0.0,
                ..*self
            }
        } else {
            *self
        }
    }

    /// The step a stateful node takes: the advance, clamped to `0..=limit`, and zero on a
    /// jump. Zero while paused, so a simulation holds.
    pub fn step(&self, limit: f32) -> f32 {
        if self.jumped {
            return 0.0;
        }
        (self.advance.max(0.0) as f32).min(limit)
    }
}

/// Where one node was on the transport when it last ticked. The synth keeps one per node and
/// reads a node's [`Time`] against it with [`Transport::time_since`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Seen {
    travel: f64,
    seeks: u64,
    tick: u64,
}

impl Seen {
    /// A transport that has not ticked.
    const ZERO: Self = Self {
        travel: 0.0,
        seeks: 0,
        tick: 0,
    };
}

/// The playhead, and how it moves.
#[derive(Debug, Clone)]
pub struct Transport {
    playing: bool,
    /// The playhead at the anchor, and the clock's `elapsed` there.
    anchor: (f64, f64),
    /// The clock's `elapsed` at the last [`Self::follow`].
    elapsed: f64,
    playhead: f64,
    /// The total distance the playhead has moved, jumps included.
    travel: f64,
    /// Seeks since the transport was made.
    seeks: u64,
    /// Where the last seek put the playhead.
    landed: f64,
    /// Ticks since the transport was made: [`Self::follow`] and [`Self::drive`] count them.
    ticks: u64,
    /// The reading at the top of this tick, before it moved: what a node ticking for the
    /// first time was last "seen" at.
    last: Seen,
    /// A render owns the playhead: the clock is not followed.
    rendering: bool,
    /// The reading the tick before the last one ended on, a seek after it included in the
    /// last: what [`Self::stood_still`] compares with.
    before: Seen,
    /// The reading the last tick ended on.
    ended: Seen,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            playing: true,
            anchor: (0.0, 0.0),
            elapsed: 0.0,
            playhead: 0.0,
            travel: 0.0,
            seeks: 0,
            landed: 0.0,
            ticks: 0,
            last: Seen::ZERO,
            rendering: false,
            before: Seen::ZERO,
            ended: Seen::ZERO,
        }
    }
}

impl Transport {
    pub fn playhead(&self) -> f64 {
        self.playhead
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn is_rendering(&self) -> bool {
        self.rendering
    }

    /// The total distance moved, jumps included.
    pub fn travel(&self) -> f64 {
        self.travel
    }

    pub fn report(&self) -> Report {
        Report {
            playhead: self.playhead,
            playing: self.playing,
            rendering: self.rendering,
        }
    }

    /// Carry out one of the editor's commands, anchored at the clock's last reading.
    ///
    /// A command that lands while a render owns the playhead changes what the live show
    /// resumes with; a seek then is ignored, since the render's frames are where they are.
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play => self.playing = true,
            Command::Pause => self.playing = false,
            Command::Seek(t) => {
                if !self.rendering {
                    self.seek(t);
                }
            }
        }
        self.anchor = (self.playhead, self.elapsed);
    }

    /// Put the playhead at `t`: a jump, counted.
    pub fn seek(&mut self, t: f64) {
        if !t.is_finite() {
            return;
        }
        self.travel += t - self.playhead;
        self.playhead = t;
        self.seeks += 1;
        self.landed = t;
        self.anchor = (t, self.elapsed);
    }

    /// One live tick: the clock reads `elapsed`, and the playhead moves by the anchor's rule.
    /// A clock that ran backwards — only a driven one does — moves nothing and re-anchors.
    pub fn follow(&mut self, elapsed: f64) {
        self.begin_tick();
        if self.rendering {
            // A render owns the playhead; the clock is not followed.
        } else if elapsed < self.elapsed {
            self.elapsed = elapsed;
            self.anchor = (self.playhead, elapsed);
        } else {
            self.elapsed = elapsed;
            let rate = if self.playing { 1.0 } else { 0.0 };
            let raw = self.anchor.0 + rate * (elapsed - self.anchor.1);
            self.travel += raw - self.playhead;
            self.playhead = raw;
        }
        self.ended = self.seen();
    }

    /// One render frame: the playhead to `t`, the travel continuous, with the clock at
    /// `elapsed` there. A render's first frame is a [`Self::seek`] of its own, made by
    /// [`Self::start_render`].
    pub fn drive(&mut self, t: f64, elapsed: f64) {
        self.begin_tick();
        if t.is_finite() {
            self.travel += t - self.playhead;
            self.playhead = t;
            self.elapsed = elapsed;
            self.anchor = (t, elapsed);
        }
        self.ended = self.seen();
    }

    /// A render takes the playhead: the live transport is handed back for [`Self::resume`],
    /// and the playhead jumps to the render's first frame at `t`.
    #[must_use]
    pub fn start_render(&mut self, t: f64) -> Self {
        let live = self.clone();
        self.rendering = true;
        self.seek(t);
        live
    }

    /// The render is over: the transport is the live one again as the render found it — its
    /// playhead, its travel, its seeks and its ticks, so a node whose last reading of it is
    /// put back with it sees no jump — playing or paused as a hand last asked, and following
    /// the clock again from `elapsed`.
    pub fn resume(&mut self, live: Self, elapsed: f64) {
        let playing = self.playing;
        *self = live;
        self.playing = playing;
        self.elapsed = elapsed;
        self.anchor = (self.playhead, elapsed);
    }

    /// Where a node that ticks now was last seen, for the node's next [`Self::time_since`].
    pub fn seen(&self) -> Seen {
        Seen {
            travel: self.travel,
            seeks: self.seeks,
            tick: self.ticks,
        }
    }

    /// What a node sees this tick, given where it was last seen: `None` for a node ticking
    /// for the first time, which is read as seen at the top of this tick — it takes this
    /// tick's own motion, as a node made a frame ago would.
    pub fn time_since(&self, seen: Option<Seen>) -> Time {
        let seen = seen.unwrap_or(self.last);
        let slept = seen.tick + 1 < self.ticks;
        let sought = seen.seeks != self.seeks;
        Time {
            playhead: self.playhead,
            advance: self.travel - seen.travel,
            jumped: slept || sought,
            landed: (sought && !slept).then_some(self.landed),
            playing: self.rendering || self.playing,
        }
    }

    /// Whether the playhead stood still over the last tick: paused, with no seek since the
    /// tick before it. On such a tick nothing in a feedback loop advances.
    pub fn stood_still(&self) -> bool {
        !self.playing
            && !self.rendering
            && self.travel == self.before.travel
            && self.seeks == self.before.seeks
    }

    /// Every tick starts here: the reading a first-time node is taken to have been seen at
    /// is the one the last tick ended on.
    fn begin_tick(&mut self) {
        self.before = self.ended;
        self.last = self.seen();
        self.ticks += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: f64 = 1.0 / 60.0;

    /// A transport followed from zero, a frame at a time, for `seconds`.
    fn run(t: &mut Transport, from: f64, seconds: f64) -> f64 {
        let frames = (seconds / FRAME).round() as u32;
        let mut now = from;
        for _ in 0..frames {
            now += FRAME;
            t.follow(now);
        }
        now
    }

    #[test]
    fn it_plays_at_the_clocks_pace() {
        let mut t = Transport::default();
        t.follow(0.0);
        run(&mut t, 0.0, 2.0);
        assert!((t.playhead() - 2.0).abs() < 1e-9, "{}", t.playhead());
        assert!((t.travel() - 2.0).abs() < 1e-9);
    }

    /// Pause holds the playhead and the travel; play picks up from there, however long the
    /// pause was.
    #[test]
    fn pause_holds() {
        let mut t = Transport::default();
        let now = run(&mut t, 0.0, 1.0);
        t.apply(Command::Pause);
        let now = run(&mut t, now, 5.0);
        assert!((t.playhead() - 1.0).abs() < 1e-9, "{}", t.playhead());
        let before = t.seen();
        t.follow(now + FRAME);
        let time = t.time_since(Some(before));
        assert_eq!(time.advance, 0.0);
        assert!(!time.playing);
        t.apply(Command::Play);
        run(&mut t, now + FRAME, 1.0);
        assert!((t.playhead() - 2.0).abs() < 1e-9, "{}", t.playhead());
    }

    /// A seek is a jump, counted: the advance carries its distance and says it was one, and
    /// where it landed.
    #[test]
    fn a_seek_is_a_jump_of_its_own_distance() {
        let mut t = Transport::default();
        let now = run(&mut t, 0.0, 1.0);
        let before = t.seen();
        t.apply(Command::Seek(41.0));
        t.follow(now + FRAME);
        let time = t.time_since(Some(before));
        assert!(time.jumped);
        assert!(
            (time.advance - (40.0 + FRAME)).abs() < 1e-9,
            "{}",
            time.advance
        );
        assert!((t.playhead() - (41.0 + FRAME)).abs() < 1e-9);
        assert_eq!(
            time.landed,
            Some(41.0),
            "where it landed, before the frame ran on"
        );
        assert_eq!(time.step(MAX_DT), 0.0, "a stateful node steps nothing");
    }

    /// A node that slept takes the whole gap, flagged: nothing inside it was played through.
    #[test]
    fn a_node_that_slept_takes_the_whole_gap_as_a_jump() {
        let mut t = Transport::default();
        t.follow(0.0);
        let before = t.seen();
        run(&mut t, 0.0, 60.0);
        let time = t.time_since(Some(before));
        assert!((time.advance - 60.0).abs() < 1e-6);
        assert!(time.jumped);
        assert_eq!(time.landed, None, "no seek lands it anywhere");

        let every = t.seen();
        t.follow(60.0 + FRAME);
        let time = t.time_since(Some(every));
        assert!(!time.jumped, "one tick later is not a sleep");
        assert!((time.advance - FRAME).abs() < 1e-9);
    }

    /// A stall is not a jump: the whole of it is advance, and a stateful node takes at most
    /// `MAX_DT` of it.
    #[test]
    fn a_stall_is_advance_and_a_clamped_step() {
        let mut t = Transport::default();
        t.follow(0.0);
        let before = t.seen();
        t.follow(0.3);
        let time = t.time_since(Some(before));
        assert!(!time.jumped);
        assert!((time.advance - 0.3).abs() < 1e-12);
        assert_eq!(time.step(MAX_DT), MAX_DT);
        assert!((time.step(f32::INFINITY) - 0.3).abs() < 1e-6);
    }

    /// A first-time node takes this tick's own motion, as a node made a frame ago would.
    #[test]
    fn a_new_node_takes_this_ticks_motion() {
        let mut t = Transport::default();
        t.follow(0.0);
        t.follow(FRAME);
        let time = t.time_since(None);
        assert!((time.advance - FRAME).abs() < 1e-12);
        assert!(!time.jumped);
    }

    /// A render drives the playhead to each frame's time, and hands the live show back its
    /// transport as it found it: a node seen before the render sees no jump.
    #[test]
    fn a_render_drives_and_hands_back() {
        let mut t = Transport::default();
        let now = run(&mut t, 0.0, 3.0);
        let before_render = t.seen();
        let live = t.start_render(-0.5);
        assert!(t.is_rendering());
        t.follow(now + 10.0);
        assert_eq!(
            t.playhead(),
            -0.5,
            "the clock is not followed while rendering"
        );
        for i in 0..30 {
            let before = t.seen();
            t.drive(-0.5 + f64::from(i + 1) / 30.0, 0.0);
            let time = t.time_since(Some(before));
            assert!((time.advance - 1.0 / 30.0).abs() < 1e-12);
            assert!(!time.jumped);
        }
        t.resume(live, now + 20.0);
        t.follow(now + 20.0);
        let time = t.time_since(Some(before_render));
        assert!(
            !time.jumped,
            "the return is no jump to what the live show saw"
        );
        assert!(time.advance.abs() < 1e-9, "{}", time.advance);
        assert!((t.playhead() - 3.0).abs() < 1e-9, "{}", t.playhead());
        t.follow(now + 21.0);
        assert!((t.playhead() - 4.0).abs() < 1e-9);
    }
}
