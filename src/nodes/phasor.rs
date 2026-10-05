// SPDX-License-Identifier: AGPL-3.0-or-later

//! A rate integrated against how far the transport moved: what the stateful nodes with a start
//! of their own step — `animation`, `automation`'s playback, a clip's Main Input play — and
//! what a free-running node's Speed is integrated by (`nodes::timing::Pace`, which the synth
//! keeps). A gear integrates its own `f64` and reads this module's [`Step`] and [`fraction`]; a
//! node that moves with time reads `Time + Offset` each frame. See `proposals/time.md` and
//! `docs/cpu.md#stateful-nodes-step-on-dt`.
//!
//! **It integrates the advance it is handed** ([`Time::advance`]), never a frame's `dt`. So it
//! pauses with the show, follows a seek by `rate × the jump`, and catches up after a stall or
//! a closed tab by exactly the distance the playhead moved — the Main Input's clip, which is
//! on. `animation` and `automation` hand it [`Time::carried`], whose jump carries no
//! advance, so a seek leaves them where they stood.
//!
//! **State is `f64`, and so is every number between nodes.** A phase that knows its period
//! publishes its [`fraction`] of that period, where the wrap is invisible. A general clock — a
//! gear's Cycles, the Time node's Seconds — publishes its **count**, unbounded, and nothing on
//! the CPU wraps it: a Math node, a row and a Time read on a CPU node all get the `f64`
//! itself. A number becomes an `f32` only where it enters a shader. A shader's Time gets it
//! [`split`] into its whole part, wrapped at [`WHOLE_WRAP`], 80640, and the `f32` of its
//! fraction, which it reads at the same precision at every count forever. 80640 is twice the
//! least common multiple of 2520 — of 1 to 10 — and 128, so every period a shader reduces the
//! whole part by — a noise's Repeat, Static's to 128, the tunnel's 64, one cycle — divides it,
//! and an `f32` holds every whole number to 2²⁴ exactly. It is twice and not once so that a
//! picture that never repeats, which a shader can only take round the wrap, comes back no
//! sooner than 40 minutes at the Speed knob's top, 4: Static rolls six times a second, 24 at
//! Speed 4, and 80640 rolls is 56 minutes. Any other shader input takes the number's `f32`,
//! with an `f32`'s precision at its size (`docs/decisions.md`).
//!
//! **The glide is exact.** A rate follows its target with a first-order lag of time constant
//! `τ`, integrated in closed form over each step,
//!
//! ```text
//! ΔΦ = S·Δ + (s₀ − S)·τ·(1 − e^(−Δ/τ))
//! ```
//!
//! so two steps of `Δ` land exactly where one of `2Δ` does, and a stepped knob gives the same
//! phase at 30, 60 and 144 frames a second.
//!
//! **Birth** ([`Birth`]): a phase starts at zero, and one born on a jump has nothing to catch
//! up.

use crate::transport::Time;

/// The modulus a count's whole part wraps under when a shader reads it ([`split`]): 80640,
/// twice the least common multiple of 2520 — of 1 to 10 — and 128, so a noise's Repeat,
/// Static's 4 to 128, the tunnel's 64 and one cycle all divide it; and the longest a shader can
/// tell a count apart over, which a picture that never repeats comes back after — Static, the
/// fastest, 56 minutes on at Speed 4. The prelude's `WHOLE_WRAP` is this number.
pub const WHOLE_WRAP: f64 = 80640.0;

/// How near a boundary a phase must come to have reached it, in its own units, and how near a
/// whole period a phase published modulo that period must come to be published as zero.
///
/// A render steps the playhead to exact frame times, so a beat or a whole cycle that falls on
/// a frame — every one on a loop's first frame, since a looped speed makes whole cycles a loop
/// — is reached exactly in arithmetic and only to within `f64` rounding in the sum of
/// advances: a hair under on one loop and over on the next, and which one depends on the
/// travel the live show left behind. Rounding is some 10⁻¹⁴ of a cycle; this is far above it
/// and far below anything a person could time.
pub const REACH: f64 = 1e-9;

/// Where a phase starts, when it is made and when a render resets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Birth {
    /// At zero: something with a start of its own — an animation, an automation's playback,
    /// a clip.
    Zero,
}

/// One step of a phase: where it was, where it is, and whether it got there by a jump.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub from: f64,
    pub to: f64,
    /// Nothing between `from` and `to` was played through: see [`Time::jumped`].
    pub jumped: bool,
}

impl Step {
    /// How far through the step the phase was at `p`, 0 to 1, or `None` where the step does
    /// not reach it or jumped over it. Forwards `p` is in `(from, to]`, backwards
    /// `[to, from)`, each end moved [`REACH`] the way the phase travels: a boundary the step
    /// ends a hair short of is reached by this step, and not again by the next.
    pub fn when(&self, p: f64) -> Option<f64> {
        if self.jumped || self.to == self.from {
            return None;
        }
        let inside = if self.to > self.from {
            p > self.from + REACH && p <= self.to + REACH
        } else {
            p >= self.to - REACH && p < self.from - REACH
        };
        inside.then(|| ((p - self.from) / (self.to - self.from)).clamp(0.0, 1.0))
    }

    /// Every `n + offset` the phase passed, `n` whole, in the order it passed them, with how
    /// far through the step each was — reached as [`Step::when`] reaches it. None on a jump: a
    /// seek over forty beats fires none.
    pub fn crossings(&self, offset: f64) -> Crossings {
        let (next, last, dir) = if self.jumped || self.to == self.from {
            (0.0, -1.0, 1.0)
        } else if self.to > self.from {
            // The first `n + offset` strictly after `from`.
            (
                (self.from - offset + REACH).floor() + 1.0,
                (self.to - offset + REACH).floor(),
                1.0,
            )
        } else {
            (
                (self.from - offset - REACH).ceil() - 1.0,
                (self.to - offset - REACH).ceil(),
                -1.0,
            )
        };
        Crossings {
            step: *self,
            offset,
            next,
            last,
            dir,
        }
    }
}

/// `x` modulo `period`, never negative, and zero within [`REACH`] of a whole number of periods.
pub fn fraction(x: f64, period: f64) -> f64 {
    let r = x.rem_euclid(period);
    if r < REACH * period || r > period * (1.0 - REACH) {
        0.0
    } else {
        r
    }
}

/// A count as a shader reads it: `[whole, fraction]`, the whole part wrapped at
/// [`WHOLE_WRAP`] centered on zero, −40320 up to 40320, and the `f32` of the fraction, 0 up to
/// 1, zero within [`REACH`] of a whole number. A shader that needs the count modulo a period
/// dividing [`WHOLE_WRAP`] reduces the whole part first, which is exact in an `f32`, then adds
/// the fraction, so the reading is as precise at the millionth cycle as at the first.
pub fn split(x: f64) -> [f32; 2] {
    if !x.is_finite() {
        return [0.0, 0.0];
    }
    let nearest = x.round();
    let (mut whole, mut fraction) = if (x - nearest).abs() <= REACH {
        (nearest, 0.0)
    } else {
        let whole = x.floor();
        (whole, (x - whole) as f32)
    };
    if fraction >= 1.0 {
        whole += 1.0;
        fraction = 0.0;
    }
    let half = WHOLE_WRAP / 2.0;
    let wrapped = whole.rem_euclid(WHOLE_WRAP);
    let wrapped = if wrapped >= half {
        wrapped - WHOLE_WRAP
    } else {
        wrapped
    };
    [wrapped as f32, fraction]
}

/// The crossings of one [`Step`], oldest first: `(n, fraction)`, the whole number `n` whose
/// `n + offset` the phase passed and how far through the step it did.
#[derive(Debug, Clone)]
pub struct Crossings {
    step: Step,
    offset: f64,
    next: f64,
    last: f64,
    dir: f64,
}

impl Iterator for Crossings {
    type Item = (f64, f64);
    fn next(&mut self) -> Option<(f64, f64)> {
        if (self.dir > 0.0 && self.next > self.last) || (self.dir < 0.0 && self.next < self.last) {
            return None;
        }
        let n = self.next;
        self.next += self.dir;
        let p = n + self.offset;
        let at = (p - self.step.from) / (self.step.to - self.step.from);
        Some((n, at.clamp(0.0, 1.0)))
    }
}

/// A speed integrated into a phase.
#[derive(Debug, Clone)]
pub struct Phasor {
    /// Unbounded, in the node's own units of `speed × seconds`.
    phase: f64,
    /// The glided speed. `None` until the first step, which takes its target as given.
    speed: Option<f64>,
    /// Started: by its [`Birth`] on the first step, or by a [`Phasor::set`] before it.
    born: bool,
    /// The glide's time constant in seconds of travel. Zero follows the target at once.
    glide: f64,
    held: bool,
}

impl Default for Phasor {
    /// A phase with a start of its own, at zero, with no glide.
    fn default() -> Self {
        Self::new(Birth::Zero)
    }
}

impl Phasor {
    pub const fn new(birth: Birth) -> Self {
        match birth {
            Birth::Zero => Self {
                phase: 0.0,
                speed: None,
                born: false,
                glide: 0.0,
                held: false,
            },
        }
    }

    /// The same, with a glide of `seconds`.
    #[must_use]
    pub const fn gliding(mut self, seconds: f64) -> Self {
        self.glide = seconds;
        self
    }

    /// The phase, unbounded.
    pub fn phase(&self) -> f64 {
        self.phase
    }

    /// The speed it is running at, glide included. Zero before it is born.
    pub fn speed(&self) -> f64 {
        self.speed.unwrap_or(0.0)
    }

    pub fn is_born(&self) -> bool {
        self.born
    }

    /// Back to unborn: the next step starts it again by its [`Birth`].
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.speed = None;
        self.born = false;
        self.held = false;
    }

    /// Put the phase somewhere: a restart, a scrub, a wrap the node keeps itself. A phasor
    /// set before its first step starts from there rather than by its [`Birth`].
    pub fn set(&mut self, phase: f64) {
        self.phase = phase;
        self.born = true;
    }

    /// Hold the phase where it is. The speed keeps following its target while held, so a
    /// release picks up at the speed asked for rather than gliding to it.
    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Integrate `by` seconds of travel toward `target`, by the exact glide. Held, the phase
    /// stays and the speed still glides.
    pub fn advance(&mut self, target: f64, by: f64) {
        let s0 = *self.speed.get_or_insert(target);
        if by == 0.0 || !by.is_finite() || !target.is_finite() {
            return;
        }
        let (moved, speed) = if self.glide > 0.0 {
            let decay = (-by.abs() / self.glide).exp();
            (
                target * by + (s0 - target) * self.glide * (1.0 - decay) * by.signum(),
                target + (s0 - target) * decay,
            )
        } else {
            (target * by, target)
        };
        self.speed = Some(speed);
        if !self.held {
            self.phase += moved;
        }
    }

    /// Start at zero if unborn, for a tick whose advance is about to be integrated.
    fn born(&mut self, target: f64) {
        if self.born {
            return;
        }
        self.born = true;
        self.speed = Some(target);
        self.phase = 0.0;
    }

    /// One tick at `target`, integrating the node's whole advance. A jump settles the glide
    /// first and moves the phase by `target × the jump`; a phasor born on a jump stays at
    /// zero.
    pub fn step(&mut self, target: f64, time: &Time) -> Step {
        self.walk(target, time).finish()
    }

    /// One tick walked in parts, for a node with moments inside the frame — a reset, a hold.
    pub fn walk(&mut self, target: f64, time: &Time) -> Walk<'_> {
        let unborn = !self.is_born();
        self.born(target);
        if time.jumped {
            self.speed = Some(target);
        }
        // A phasor born on a jump has nothing to catch up: it did not exist before it.
        let advance = if unborn && time.jumped {
            0.0
        } else {
            time.advance
        };
        Walk {
            from: self.phase,
            phasor: self,
            target,
            advance,
            done: 0.0,
            jumped: time.jumped,
        }
    }
}

/// A tick in progress: [`Walk::to`] integrates up to a moment, anything may happen there, and
/// [`Walk::finish`] integrates the rest.
pub struct Walk<'a> {
    phasor: &'a mut Phasor,
    target: f64,
    advance: f64,
    /// How far through the tick the walk has integrated, 0 to 1.
    done: f64,
    from: f64,
    jumped: bool,
}

impl Walk<'_> {
    /// Integrate up to `fraction` of the tick. A moment before one already reached is
    /// reached already.
    pub fn to(&mut self, fraction: f64) {
        let fraction = fraction.clamp(self.done, 1.0);
        self.phasor
            .advance(self.target, self.advance * (fraction - self.done));
        self.done = fraction;
    }

    /// Put the phase somewhere at the moment reached: what is left of the tick runs on from
    /// there.
    pub fn set(&mut self, phase: f64) {
        self.phasor.set(phase);
    }

    pub fn hold(&mut self, held: bool) {
        self.phasor.hold(held);
    }

    pub fn phasor(&self) -> &Phasor {
        self.phasor
    }

    /// Integrate what is left of the tick.
    pub fn finish(mut self) -> Step {
        self.to(1.0);
        Step {
            from: self.from,
            to: self.phasor.phase,
            jumped: self.jumped,
        }
    }
}

/// How far a cabled clock published modulo `wrap` moved from `from` to `to`: a Phase, 0 up to
/// 1, wraps at one, so a step of more than half a cycle is its wrap and not a motion. A clock
/// that never wraps, `wrap` infinite, moved by the difference.
pub fn unwrap_at(from: f64, to: f64, wrap: f64) -> f64 {
    let d = to - from;
    let half = wrap / 2.0;
    if d > half {
        d - wrap
    } else if d < -half {
        d + wrap
    } else {
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(advance: f64) -> Time {
        Time {
            advance,
            ..Time::default()
        }
    }

    /// Run a phasor at `fps` for `seconds`, stepping the target from `a` to `b` at `at`.
    fn run(fps: f64, seconds: f64, a: f64, b: f64, at: f64) -> f64 {
        let mut p = Phasor::new(Birth::Zero).gliding(0.1);
        let frames = (seconds * fps).round() as u32;
        let dt = 1.0 / fps;
        let switch = (at * fps).round() as u32;
        for i in 0..frames {
            let target = if i < switch { a } else { b };
            p.step(target, &time(dt));
        }
        p.phase()
    }

    /// The trigger for the whole design: an `f32` phase at a million seconds stops moving.
    /// This one advances by one second of ticks, to the nanosecond.
    #[test]
    fn a_phase_at_a_million_seconds_advances_by_exactly_one_second_of_ticks() {
        let mut p = Phasor::new(Birth::Zero);
        p.step(1.0, &time(0.0));
        p.set(1.0e6);
        for _ in 0..144 {
            p.step(1.0, &time(1.0 / 144.0));
        }
        assert!(
            (p.phase() - (1.0e6 + 1.0)).abs() < 1e-8,
            "{}",
            p.phase() - 1.0e6
        );
    }

    /// The exact glide: a knob stepped at one moment gives the same phase whatever the frame
    /// rate, where a per-frame lag loses a different amount at each.
    #[test]
    fn the_exact_glide_gives_the_same_phase_at_30_60_and_144_fps() {
        let at30 = run(30.0, 3.0, 1.0, 2.0, 1.0);
        let at60 = run(60.0, 3.0, 1.0, 2.0, 1.0);
        let at144 = run(144.0, 3.0, 1.0, 2.0, 1.0);
        assert!((at30 - at60).abs() < 1e-9, "{at30} {at60}");
        assert!((at60 - at144).abs() < 1e-9, "{at60} {at144}");
        // And the closed form's own answer: one second at 1, then two at 2 with a glide from 1.
        let expected = 1.0 + 2.0 * 2.0 + (1.0 - 2.0) * 0.1 * (1.0 - (-2.0f64 / 0.1).exp());
        assert!((at60 - expected).abs() < 1e-9, "{at60} {expected}");
    }

    /// A count split for a shader: the whole part wrapped at 80640 centered on zero, the
    /// fraction its `f32`, and the pair as precise far from zero as near it — the fraction of
    /// a count a million cycles on is the fraction near zero, to the bit, and the whole part
    /// reduced by any period dividing 80640 is the count's own.
    #[test]
    fn a_count_splits_into_a_whole_part_and_its_fraction() {
        assert_eq!(split(0.0), [0.0, 0.0]);
        assert_eq!(split(-0.25), [-1.0, 0.75]);
        assert_eq!(split(3.5), [3.0, 0.5]);
        assert_eq!(split(20160.0), [20160.0, 0.0]);
        assert_eq!(split(40320.0), [-40320.0, 0.0]);
        assert_eq!(split(80640.0 + 7.25), [7.0, 0.25]);
        assert_eq!(
            split(5.0 - 1e-12),
            [5.0, 0.0],
            "a hair short of a whole number is it"
        );
        assert_eq!(split(5.0 + 1e-12), [5.0, 0.0]);
        assert_eq!(split(f64::NAN), [0.0, 0.0]);
        assert_eq!(split(1.0 - 1e-9 * 0.5)[1], 0.0);
        for k in [1.0, 1260.0, 2520.0, 40320.0, 1.0e6] {
            for f in [0.1, 1.0 / 3.0, 0.999] {
                assert_eq!(
                    split(k + f)[1],
                    split(f)[1],
                    "the fraction {k} cycles on is the fraction near zero"
                );
            }
        }
        for x in [
            -3000.7,
            -0.4,
            0.4,
            1259.9,
            1260.3,
            20159.5,
            20160.5,
            40319.5,
            40320.5,
            4.0e5 + 0.6,
        ] {
            let [whole, fraction] = split(x);
            for period in [1.0, 2.0, 16.0, 64.0, 128.0, 9.0, 7.0, 3.0, WHOLE_WRAP] {
                let ours = f64::from(whole).rem_euclid(period) + f64::from(fraction);
                let truth = x.rem_euclid(period);
                assert!(
                    (ours - truth).abs() < 1e-6,
                    "{x} over {period}: {ours} and {truth}"
                );
            }
        }
    }

    /// The prelude's `WHOLE_WRAP`, which a noise at Repeat Never takes its time cell round,
    /// is where [`split`] wraps the whole part.
    #[test]
    fn the_prelude_wraps_where_a_count_is_split() {
        let line = format!("const WHOLE_WRAP: f32 = {WHOLE_WRAP:?};");
        assert!(crate::compile::wgsl::PRELUDE.contains(&line), "{line}");
    }

    /// Held, the phase stays and the speed keeps following.
    #[test]
    fn hold_stops_the_phase_and_not_the_glide() {
        let mut p = Phasor::new(Birth::Zero).gliding(0.1);
        p.step(1.0, &time(0.5));
        p.hold(true);
        for _ in 0..60 {
            p.step(3.0, &time(1.0 / 60.0));
        }
        assert!((p.phase() - 0.5).abs() < 1e-12);
        assert!((p.speed() - 3.0).abs() < 1e-3, "{}", p.speed());
    }

    /// A seek moves a phase by speed × the jump, with the glide settled.
    #[test]
    fn a_jump_moves_by_speed_times_the_jump() {
        let mut p = Phasor::new(Birth::Zero).gliding(0.1);
        p.step(1.0, &time(0.0));
        let step = p.step(
            2.0,
            &Time {
                advance: 10.0,
                jumped: true,
                ..Time::default()
            },
        );
        assert!((p.phase() - 20.0).abs() < 1e-12);
        assert!(step.jumped);
        assert_eq!(step.crossings(0.0).count(), 0, "a jump fires no crossings");
    }

    /// A phase is born at zero wherever the playhead is, and takes the tick's advance; born on
    /// a jump, it has nothing to catch up.
    #[test]
    fn a_phase_is_born_at_zero_and_on_a_jump_stays_there() {
        let t = Time {
            playhead: 100.0,
            advance: 1.0 / 60.0,
            ..Time::default()
        };
        let mut z = Phasor::new(Birth::Zero);
        z.step(0.5, &t);
        assert!((z.phase() - 0.5 / 60.0).abs() < 1e-12, "{}", z.phase());
        let mut z = Phasor::new(Birth::Zero);
        z.step(0.5, &Time { jumped: true, ..t });
        assert_eq!(z.phase(), 0.0, "born on a jump, it has nothing to catch up");
    }

    /// Crossings of whole numbers, and of whole numbers plus an offset, with their moments.
    #[test]
    fn crossings_are_whole_numbers_with_their_moments() {
        let step = Step {
            from: 0.5,
            to: 2.5,
            jumped: false,
        };
        let got: Vec<_> = step.crossings(0.0).collect();
        assert_eq!(got, vec![(1.0, 0.25), (2.0, 0.75)]);
        let got: Vec<_> = step.crossings(0.25).collect();
        assert_eq!(
            got,
            vec![(0.0, 0.0), (1.0, 0.375), (2.0, 0.875)].split_off(1)
        );
        let back = Step {
            from: 2.5,
            to: 0.5,
            jumped: false,
        };
        let got: Vec<_> = back.crossings(0.0).collect();
        assert_eq!(got, vec![(2.0, 0.25), (1.0, 0.75)]);
        let exact = Step {
            from: 0.0,
            to: 1.0,
            jumped: false,
        };
        assert_eq!(
            exact.crossings(0.0).collect::<Vec<_>>(),
            vec![(1.0, 1.0)],
            "the start is not crossed, the end is"
        );
        assert_eq!(exact.when(0.5), Some(0.5));
        assert_eq!(exact.when(0.0), None);
    }

    /// A boundary a step ends a hair short of is reached by that step and not again by the
    /// next, and a phase a hair short of a whole period publishes zero: a sum of a hundred
    /// advances of a twenty-fifth of a second is four seconds only to within rounding.
    #[test]
    fn a_boundary_a_hair_short_is_reached() {
        let mut at = 0.0_f64;
        for _ in 0..100 {
            at += 0.32;
        }
        assert_ne!(at, 32.0, "the sum is not exact, which is the point");
        let short = Step {
            from: 31.68,
            to: 32.0 - 1e-12,
            jumped: false,
        };
        assert_eq!(short.when(32.0), Some(1.0));
        assert_eq!(short.crossings(0.0).collect::<Vec<_>>(), vec![(32.0, 1.0)]);
        let next = Step {
            from: short.to,
            to: short.to + 0.32,
            jumped: false,
        };
        assert_eq!(next.when(32.0), None, "and not again");
        assert_eq!(next.crossings(0.0).count(), 0);
        let back = Step {
            from: 32.32,
            to: 32.0 + 1e-12,
            jumped: false,
        };
        assert_eq!(back.crossings(0.0).collect::<Vec<_>>(), vec![(32.0, 1.0)]);
        assert_eq!(fraction(32.0 - 1e-12, 1.0), 0.0);
        assert_eq!(fraction(32.0 + 1e-12, 1.0), 0.0);
        assert_eq!(
            fraction(
                8.0 * std::f64::consts::PI - 1e-12,
                4.0 * std::f64::consts::PI
            ),
            0.0
        );
        assert!((fraction(32.25, 1.0) - 0.25).abs() < 1e-12);
        assert!((fraction(-0.25, 1.0) - 0.75).abs() < 1e-12);
    }

    /// A Phase's step over its wrap at one is the wrap, and a clock that never wraps moves by
    /// the difference however far apart: a count a million cycles on, and one going back.
    #[test]
    fn a_step_over_half_the_wrap_is_the_wrap() {
        assert!((unwrap_at(0.95, 0.05, 1.0) - 0.1).abs() < 1e-9);
        assert!((unwrap_at(0.05, 0.95, 1.0) + 0.1).abs() < 1e-9);
        assert!((unwrap_at(0.2, 0.5, 1.0) - 0.3).abs() < 1e-9);
        assert_eq!(unwrap_at(2519.9, 2520.1, f64::INFINITY), 2520.1 - 2519.9);
        assert_eq!(unwrap_at(0.1, 2519.9, f64::INFINITY), 2519.8);
        assert_eq!(unwrap_at(1.0e6, 1.0e6 + 0.5, f64::INFINITY), 0.5);
        assert_eq!(unwrap_at(-2519.9, -2520.4, f64::INFINITY), -2520.4 + 2519.9);
    }

    /// A walk integrates in parts exactly what a step integrates whole.
    #[test]
    fn a_walk_in_parts_is_a_step_whole() {
        let mut a = Phasor::new(Birth::Zero).gliding(0.1);
        let mut b = a.clone();
        a.step(1.0, &time(0.0));
        b.step(1.0, &time(0.0));
        a.step(3.0, &time(0.2));
        let mut walk = b.walk(3.0, &time(0.2));
        walk.to(0.25);
        walk.to(0.6);
        let step = walk.finish();
        assert!((a.phase() - b.phase()).abs() < 1e-12);
        assert!((step.to - b.phase()).abs() < 1e-15);
    }
}
