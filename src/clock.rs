// SPDX-License-Identifier: AGPL-3.0-or-later

//! The single clock.
//!
//! silvia drives animation from 45 independent `requestAnimationFrame` loops, each with its
//! own `performance.now()`, so the order of work within a frame is undefined. supersilvia has
//! exactly one clock: `Clock::tick` runs once per tick, and the transport
//! ([`crate::transport`]) is a coordinate system over it — a playhead that plays, pauses,
//! seeks and loops, whose motion is what every node integrates. Nothing else keeps a timer.
//!
//! There is exactly one clock, and it does not lie about elapsed time: neither `elapsed` nor
//! `dt` is clamped. The bound a stateful node needs on a stall is its own, applied where its
//! step is taken (`transport::MAX_DT`).
//!
//! The clock can also be **driven**: an offline render makes time a function of the frame
//! index rather than of the wall clock, `t = (i - warmup) / fps`, and `Clock::set_elapsed` is
//! how that time reaches everything that samples `elapsed` — a trace's dates. A [`Stepper`]
//! owns a clock for the length of such a run, and the render drives the transport's playhead
//! to the same `t`.

/// Per-frame timing, advanced once at the top of the frame.
#[derive(Debug, Clone)]
pub struct Clock {
    /// Time of the first tick, in seconds, on egui's clock.
    start: Option<f64>,
    /// Time of the previous tick.
    last: Option<f64>,
    /// Seconds since the previous tick. Zero on the first frame.
    dt: f32,
    /// Seconds since the first tick. **True elapsed time, never clamped.**
    elapsed: f64,
    /// Number of ticks since startup.
    ticks: u64,
}

/// How many figures a worst is remembered for: two seconds at 60 Hz.
pub const WORST_WINDOW: u32 = 120;

/// A readout's figure: the latest, a running mean and the worst of the recent past.
///
/// The mean moves toward each figure by a weight, clamped to `0..=1` — the seconds since the
/// last figure gives it a time constant of about a second — and the first figure seeds it.
/// The worst is the largest of the last [`WORST_WINDOW`] figures, or a larger one since.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Figure {
    pub now: f32,
    pub avg: f32,
    pub worst: f32,
    /// How many figures the worst has left before it is forgotten.
    worst_age: u32,
    /// True once a figure has been recorded.
    seeded: bool,
}

impl Figure {
    /// Take one figure, moving the mean by `weight`.
    pub fn record(&mut self, value: f32, weight: f32) {
        self.now = value;
        if self.seeded {
            self.avg += (value - self.avg) * weight.clamp(0.0, 1.0);
        } else {
            self.avg = value;
            self.seeded = true;
        }
        if value >= self.worst || self.worst_age >= WORST_WINDOW {
            self.worst = value;
            self.worst_age = 0;
        } else {
            self.worst_age += 1;
        }
    }

    /// Whether anything has been recorded.
    pub fn seeded(&self) -> bool {
        self.seeded
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    pub fn new() -> Self {
        Self {
            start: None,
            last: None,
            dt: 0.0,
            elapsed: 0.0,
            ticks: 0,
        }
    }

    /// Advance to `now` (seconds, from `egui::Context::input(|i| i.time)`) and return `dt`.
    pub fn tick(&mut self, now: f64) -> f32 {
        // The origin is placed so that `elapsed` continues from wherever it is: zero on a new
        // clock, and the last driven time on one that `set_elapsed` has moved.
        let start = *self.start.get_or_insert(now - self.elapsed);
        // Unclamped, and monotonic: a clock that can run backwards is worse than one that
        // stalls.
        self.elapsed = (now - start).max(self.elapsed);

        self.dt = match self.last {
            Some(previous) => (now - previous).max(0.0) as f32,
            None => 0.0,
        };
        self.last = Some(now);
        self.ticks += 1;
        self.dt
    }

    /// Forget where the wall was, keeping `elapsed`: the next [`Self::tick`] measures its
    /// interval from *that* tick and not from whenever this clock was last touched.
    ///
    /// What a clock put aside needs before it is used again. Without it the first tick after
    /// an offline render measures the whole render as one interval, and the transport would
    /// play the show on by the length of the render.
    pub fn reanchor(&mut self) {
        self.start = None;
        self.last = None;
        self.dt = 0.0;
    }

    /// Drive the clock to virtual time `t`, in seconds, and return `dt`.
    ///
    /// `dt` is the distance from the previous time, never negative. It is zero on a clock
    /// that has never ticked, as a live first tick is.
    /// `t` may be negative — a warm-up runs from before the beginning — and may go backwards,
    /// since a run starts over where a wall clock cannot. The wall-clock origin is forgotten:
    /// a later `tick` continues from `t`, so a driven clock never sticks waiting for the wall
    /// to catch up with it.
    pub fn set_elapsed(&mut self, t: f64) -> f32 {
        self.dt = if self.ticks == 0 {
            0.0
        } else {
            (t - self.elapsed).max(0.0) as f32
        };
        self.elapsed = t;
        self.start = None;
        self.last = None;
        self.ticks += 1;
        self.dt
    }

    /// Move on by `dt` seconds, as a host with no wall clock of its own does — a test, the
    /// inline host — and return it. A negative or non-finite `dt` moves nothing.
    pub fn advance(&mut self, dt: f32) -> f32 {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.elapsed += f64::from(dt);
        self.dt = dt;
        self.start = None;
        self.last = None;
        self.ticks += 1;
        self.dt
    }

    /// Seconds since the previous tick. What happened, not what a node integrates: a node
    /// reads the transport's advance.
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// True seconds since the first tick. Stalls are included, because they happened.
    ///
    /// This is the one clock. A musical grid, a clip's local time and a scrub are coordinate
    /// systems over it, not timers of their own.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Ticks since startup.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// What was on screen before frame zero: the question a live renderer never has to answer
/// and a render with feedback in it cannot avoid. silvia's three answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Warmup {
    /// Clear the buffers and advance nothing: the first kept frame is the first frame run.
    Black,
    /// Render the scene at `t = 0` this many times before keeping one. Feedback settles,
    /// and nothing that integrates time moves, because `dt` is zero through the hold.
    Hold(u32),
    /// Run this many frames at negative virtual time, `(i − warmup) / fps`, so the patch
    /// arrives at `t = 0` the way it would have live.
    Run(u32),
}

impl Warmup {
    /// Frames run and not kept.
    pub fn frames(self) -> u32 {
        match self {
            Self::Black => 0,
            Self::Hold(n) | Self::Run(n) => n,
        }
    }
}

/// Virtual time for an offline render: a [`Warmup`], then `frames` that are kept, at `fps`,
/// on a clock of its own.
///
/// Frame `i` counts from zero over the whole run, warm-up included. The first kept frame is
/// `t = 0` exactly; what the warm-up frames before it are at is the mode's.
/// The clock is the stepper's for the run: swap it in for a tick, and the live clock is
/// untouched by however long the render took.
#[derive(Debug, Clone)]
pub struct Stepper {
    fps: f64,
    warmup: Warmup,
    frames: u32,
    clock: Clock,
}

impl Stepper {
    /// A run of `frames` kept frames after a `warmup`, at `fps`.
    pub fn new(fps: f64, frames: u32, warmup: Warmup) -> Self {
        assert!(fps > 0.0, "a render at {fps} fps has no frame length");
        Self {
            fps,
            warmup,
            frames,
            clock: Clock::new(),
        }
    }

    /// Frames in the run, warm-up included.
    pub fn len(&self) -> u32 {
        self.warmup.frames() + self.frames
    }

    pub fn warmup(&self) -> Warmup {
        self.warmup
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The frames that are kept: those after the warm-up.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn fps(&self) -> f64 {
        self.fps
    }

    /// The virtual time of frame `i`: zero on the first kept frame, and through the warm-up
    /// either negative or held at zero, as the mode says.
    pub fn time_of(&self, i: u32) -> f64 {
        let warm = self.warmup.frames();
        match self.warmup {
            Warmup::Hold(_) if i < warm => 0.0,
            _ => (f64::from(i) - f64::from(warm)) / self.fps,
        }
    }

    /// Is frame `i` one of the warm-up, run for its side effects and never written?
    pub fn is_warmup(&self, i: u32) -> bool {
        i < self.warmup.frames()
    }

    /// The kept frame `i` is, for a frame that is not warm-up.
    pub fn kept_index(&self, i: u32) -> Option<u32> {
        i.checked_sub(self.warmup.frames())
    }

    /// Drive the clock to frame `i` and return `dt`.
    pub fn step(&mut self, i: u32) -> f32 {
        let t = self.time_of(i);
        self.clock.set_elapsed(t)
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// The clock itself, for swapping into the place a tick reads from.
    pub fn clock_mut(&mut self) -> &mut Clock {
        &mut self.clock
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_driven_clock_is_where_it_was_put() {
        let mut c = Clock::new();
        assert_eq!(
            c.set_elapsed(-0.5),
            0.0,
            "a first step is an origin, as a first tick is"
        );
        assert_eq!(
            c.elapsed(),
            -0.5,
            "a warm-up runs from before the beginning"
        );
        assert!((c.set_elapsed(0.0) - 0.5).abs() < 1e-6);
        assert_eq!(c.elapsed(), 0.0);
        assert_eq!(c.ticks(), 2);
    }

    /// A stepped frame is as long as it is: a low frame rate integrates at its own speed.
    #[test]
    fn a_driven_dt_is_not_clamped() {
        let mut c = Clock::new();
        c.set_elapsed(0.0);
        assert_eq!(c.set_elapsed(1.0), 1.0);
        assert_eq!(c.dt(), 1.0);
    }

    #[test]
    fn a_driven_clock_may_start_over() {
        let mut c = Clock::new();
        c.set_elapsed(10.0);
        assert_eq!(
            c.set_elapsed(-1.0),
            0.0,
            "backwards is a new start, not a negative dt"
        );
        assert_eq!(c.elapsed(), -1.0);
    }

    /// Driving the live clock and then ticking it must not leave it stuck at the driven time
    /// until the wall catches up: it continues from there.
    #[test]
    fn a_live_tick_after_driving_continues_from_the_driven_time() {
        let mut c = Clock::new();
        c.tick(100.0);
        c.tick(101.0);
        c.set_elapsed(60.0);
        assert_eq!(
            c.tick(102.0),
            0.0,
            "the wall-clock origin was forgotten with the drive"
        );
        assert!((c.elapsed() - 60.0).abs() < 1e-6);
        assert!((c.tick(102.05) - 0.05).abs() < 1e-6);
        assert!((c.elapsed() - 60.05).abs() < 1e-6, "{}", c.elapsed());
    }

    #[test]
    fn the_stepper_places_the_first_kept_frame_at_zero() {
        let s = Stepper::new(30.0, 90, Warmup::Run(15));
        assert_eq!(s.len(), 105);
        assert!(s.is_warmup(14));
        assert!(!s.is_warmup(15));
        assert!((s.time_of(0) + 0.5).abs() < 1e-9, "{}", s.time_of(0));
        assert_eq!(s.time_of(15), 0.0);
        assert!((s.time_of(45) - 1.0).abs() < 1e-9);
        assert!(
            (s.time_of(s.len()) - 3.0).abs() < 1e-9,
            "one past the end is the duration"
        );
    }

    #[test]
    fn stepping_hands_out_exactly_one_frame_of_dt() {
        let mut s = Stepper::new(24.0, 48, Warmup::Black);
        assert_eq!(s.step(0), 0.0);
        for i in 1..s.len() {
            let dt = s.step(i);
            assert!((dt - 1.0 / 24.0).abs() < 1e-6, "frame {i}: dt {dt}");
            assert!((s.clock().elapsed() - f64::from(i) / 24.0).abs() < 1e-9);
        }
    }

    /// A hold is `t = 0` for every warm-up frame, so `dt` is zero through it: feedback
    /// settles and nothing that integrates moves.
    #[test]
    fn a_hold_stands_at_zero_until_the_first_kept_frame() {
        let mut s = Stepper::new(30.0, 30, Warmup::Hold(10));
        for i in 0..10 {
            assert_eq!(s.time_of(i), 0.0);
            assert_eq!(s.step(i), 0.0, "frame {i}");
            assert!(s.is_warmup(i));
            assert_eq!(s.kept_index(i), None);
        }
        assert_eq!(s.time_of(10), 0.0);
        assert_eq!(s.kept_index(10), Some(0));
        assert!((s.time_of(11) - 1.0 / 30.0).abs() < 1e-9);
        assert_eq!(Stepper::new(30.0, 30, Warmup::Black).len(), 30);
    }

    /// The property a render rests on: the time of a frame is a function of its index and of
    /// nothing else, so two runs, or one run twice, agree bit for bit.
    #[test]
    fn the_same_frame_has_the_same_time_twice() {
        let mut a = Stepper::new(60.0, 600, Warmup::Run(120));
        let mut b = a.clone();
        for i in 0..a.len() {
            a.step(i);
        }
        for i in (0..b.len()).rev() {
            b.step(i);
        }
        a.step(333);
        b.step(333);
        assert_eq!(a.clock().elapsed(), b.clock().elapsed());
        assert_eq!(
            a.clock().elapsed() as f32,
            b.clock().elapsed() as f32,
            "as u_time sees it"
        );
    }

    /// The first figure seeds the mean, a weight moves it, and a worst is kept for its window
    /// and forgotten after.
    #[test]
    fn a_figure_keeps_its_mean_and_its_worst() {
        let mut f = Figure::default();
        assert!(!f.seeded());
        f.record(10.0, 0.5);
        assert_eq!((f.now, f.avg, f.worst), (10.0, 10.0, 10.0));
        f.record(20.0, 0.5);
        assert_eq!((f.now, f.avg, f.worst), (20.0, 15.0, 20.0));
        f.record(0.0, 2.0);
        assert_eq!(f.avg, 0.0, "a weight past one is one");
        for _ in 0..WORST_WINDOW - 1 {
            f.record(1.0, 0.0);
        }
        assert_eq!(f.worst, 20.0, "held for its window");
        f.record(1.0, 0.0);
        f.record(1.0, 0.0);
        assert_eq!(f.worst, 1.0, "and forgotten after it");
    }

    /// A host with no wall moves the clock by what it says passed.
    #[test]
    fn advancing_moves_elapsed_by_dt() {
        let mut c = Clock::new();
        c.advance(0.25);
        c.advance(0.5);
        assert_eq!(c.elapsed(), 0.75);
        assert_eq!(c.dt(), 0.5);
        c.advance(-1.0);
        assert_eq!(c.elapsed(), 0.75, "never backwards");
        assert!(
            (c.tick(9.0) - 0.0).abs() < 1e-9,
            "a wall after it starts from here"
        );
        assert_eq!(c.elapsed(), 0.75);
    }

    #[test]
    fn first_tick_has_zero_dt() {
        let mut c = Clock::new();
        assert_eq!(c.tick(123.0), 0.0);
        assert_eq!(c.elapsed(), 0.0);
        assert_eq!(c.ticks(), 1);
    }

    #[test]
    fn dt_is_the_gap_between_ticks() {
        let mut c = Clock::new();
        c.tick(10.0);
        assert!((c.tick(10.05) - 0.05).abs() < 1e-6);
        assert!((c.elapsed() - 0.05).abs() < 1e-6);
    }

    /// Neither `dt` nor `elapsed` is clamped: a stall is thirty seconds long, and the bound a
    /// stateful node needs is its own (`transport::MAX_DT`).
    #[test]
    fn a_stall_is_the_whole_of_it() {
        let mut c = Clock::new();
        c.tick(0.0);
        assert_eq!(c.tick(30.0), 30.0, "thirty seconds passed");
        assert!(
            (c.elapsed() - 30.0).abs() < 1e-6,
            "thirty seconds passed, so elapsed is thirty seconds: {}",
            c.elapsed()
        );
    }

    #[test]
    fn elapsed_never_runs_backwards() {
        let mut c = Clock::new();
        c.tick(5.0);
        assert_eq!(c.elapsed(), 0.0, "the first tick is the origin");
        c.tick(8.0);
        assert!((c.elapsed() - 3.0).abs() < 1e-6);

        // A source that jumps backwards must not drag the clock with it.
        assert_eq!(c.tick(6.0), 0.0, "no negative dt");
        assert!(
            (c.elapsed() - 3.0).abs() < 1e-6,
            "elapsed holds rather than rewinding: {}",
            c.elapsed()
        );
    }
}
