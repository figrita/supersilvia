// SPDX-License-Identifier: AGPL-3.0-or-later

//! Time, for every node that moves with it. The one place its rules live: the declaration a
//! node makes, the rows and options that declaration expands into, the Free integrator, what
//! a CPU node reads, the WGSL a body reads it through, and what `nodes::chain` reads to say
//! whether a loop closes. `docs/nodes.md#timing` is the spec and
//! `docs/decisions.md#time-is-an-input-port-not-an-ambient-global` the argument.
//!
//! **One clock.** The transport's playhead is the only clock (`transport.rs`); nothing here
//! keeps a timer. A node that moves with time declares a [`Timing`] and nothing else about
//! time: its pace, whether a new one stands still, the period its picture comes back after,
//! and whether it has one axis or two. Everything else is expanded from that, so no two nodes can disagree
//! about what a time row is.
//!
//! **Two modes, so the first time row means one thing at a time** ([`MODE`], key
//! `clockMode`):
//!
//! - **Free**, the default: the row is **Speed** ([`SPEED`]), a uniform number with a knob
//!   and a port, a multiple of the node's [`Timing::pace`] — 1 its normal pace, 0 still,
//!   negative backwards. The synth integrates it against the transport's advance into the
//!   node's own playhead ([`Pace`]): it pauses with the show, follows a seek by
//!   `speed × the jump`, wakes from a closed tab having moved by the gap, and is born where
//!   the playhead puts it, `speed × playhead`, so a render that starts it again is
//!   deterministic and Speed 1 reads exactly what Loop mode's ambient reading does. A cable
//!   into Speed — an LFO, an envelope — changes how fast the node runs, not where it is.
//! - **Loop**: the row is **Time** ([`TIME`]), a diamond with no knob. Unplugged it is
//!   ambient time, `playhead × pace`, so every node in Loop mode moves with the show with
//!   nothing cabled in; plugged, what arrives replaces it, usually a gear's Cycles, which
//!   drives the node exactly and closes a loop to the bit.
//!
//! Either way the synth publishes where the node is as a **count** under the Time key —
//! `u_count_{slug}{id}_clock`, split into a whole part and a fraction (`phasor::split`) — so
//! a body reads Time and never Speed, and a count is as precise a million cycles on as at the
//! first; a CPU node's own reading is published there by `TickContext::cycle`. Switching mode
//! drops the cable in the row that goes away in the same step
//! (`Graph::drop_inactive`), since a gear's Cycles, a growing count, would race off as a
//! speed; a cable can never land on the row a mode puts away ([`is_inactive`]).
//!
//! **Offset** ([`OFFSET`]) is added on top in both modes, in the node's own cycles: 1 is one
//! cycle. A varying number on a node that draws, so a field ripples it, and a uniform number on
//! a CPU node, whose tick has no pixel. Its knob reaches one whole period either way, `−P` to
//! `P`, from the period the node's options give it now, or one cycle either way where its
//! picture never comes back ([`range`], which `nodes::control_range` asks), stepping by about a
//! thousandth of it so a drag across ±1 and one across ±64 are as long. An edit that shrinks
//! the period takes a stored Offset round it ([`fit_offsets`]), which on a picture that comes
//! back every `P` is the same picture; a cable into Offset is added as it arrives.
//!
//! **The rows** are Time, Speed and Offset per axis, folded under one Timing heading
//! ([`HEADING`]) that starts closed; the mode is two segments on the heading's bar. The mode
//! shows one of Time and Speed and puts the other away in place, so the node keeps its height,
//! and where Speed's knob stands the Time row carries the loop meter (`ui::loop_meter`): where
//! the node is in its period, by [`Progress`].
//! `node!` writes them for a shader node from its `timing:` (`timing_xy:` for two axes), and
//! [`inputs!`] and [`options!`] for a hand-written one.
//!
//! **What a body reads.** One WGSL helper per kind of period, in the prelude: `time_periodic`
//! for a node whose picture comes back every cycle, `time_repeat` for one that repeats every
//! `n`, and `time_unbounded` for one that never does. Each reduces the count's whole part by
//! the period, adds its fraction, then adds Offset, which is the order that closes a loop to
//! the bit: a Time one period on draws exactly what a Time of zero drew, whatever the Offset.
//!
//! **What a CPU node reads**: [`TickContext::cycle`](crate::nodes::TickContext::cycle), where
//! the node is with its Offset added, or `cycle_at` for a clip, whose pace is one play over its
//! own length.

use crate::graph::{ControlRange, ControlValue, Node, PortType, Value};
use crate::nodes::{Control, InputDef, OptionDef, OptionKind, phasor};
use crate::transport::Time;

/// The key of a time-driven node's **Time**, the first time row in Loop mode: a diamond with
/// no knob, in the node's own cycles. Unplugged, ambient time at the node's pace, which
/// the synth writes under this key every tick; plugged, what arrives replaces it. In Free mode
/// the synth writes the node's own playhead here instead.
pub const TIME: &str = "clock";

/// The key of a second axis's Time: Shaky Cam's Y.
pub const TIME_Y: &str = "clockY";

/// The key of a time-driven node's **Speed**, the first time row in Free mode: a multiple of
/// the node's pace, from −4 to 4.
pub const SPEED: &str = "speed";

/// The key of a second axis's Speed: Shaky Cam's Y.
pub const SPEED_Y: &str = "speedY";

/// The key of **Offset**, added to where the node is every frame, in its own cycles.
pub const OFFSET: &str = "phaseOffset";

/// The key of a second axis's Offset: Shaky Cam's Y.
pub const OFFSET_Y: &str = "phaseOffsetY";

/// [`MODE`]'s value for a node that runs free on its Speed. The default.
pub const FREE: &str = "free";

/// [`MODE`]'s value for a node on a clock: its Time, ambient or cabled.
pub const LOOP: &str = "loop";

/// The heading the time rows fold under: closed on a new node.
pub const HEADING: OptionDef =
    OptionDef::heading("timing", "Timing", false, OptionKind::Presentation);

/// The mode: **Free** or **Loop**, drawn as two segments on [`HEADING`]'s bar. A tick reads
/// it, so a change rebuilds nothing.
pub const MODE: OptionDef = OptionDef {
    key: "clockMode",
    label: "Time Mode",
    default: FREE,
    choices: &[(FREE, "Free"), (LOOP, "Loop")],
    kind: OptionKind::Runtime,
    on_heading: Some(HEADING.key),
    ..OptionDef::EMPTY
};

/// The row headings a time-driven node declares: the one [`HEADING`].
pub const ROW_HEADINGS: &[&str] = &[HEADING.key];

/// One axis or two: Shaky Cam moves X and Y on clocks of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axes {
    One,
    Two,
}

/// How a node keeps time: the one thing it says about it.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How many of the node's own cycles one second is: at a Speed of 1 in Free mode, and on
    /// ambient time in Loop mode. silvia's default speed, rounded to whole seconds where it was
    /// 20π, and a pace that looks natural where silvia sits still.
    pub pace: f64,
    /// Whether a new node stands still: its Speed starts at 0 rather than 1, as silvia's sits
    /// still at rest. Free mode alone; Loop mode always moves with its Time.
    pub still: bool,
    /// How long the picture takes to come back, in its own units, by what its options say:
    /// one for a periodic node, a noise's Repeat, the tunnel's 64 while its depth wraps, and
    /// `None` for a picture that never repeats.
    pub period: fn(&Node) -> Option<f64>,
    pub axes: Axes,
    /// Whether the node's pace is one play over a length of its own — a clip's, a GIF's — that
    /// the node reads off its file and hands to `TickContext::cycle_at`, so [`Self::pace`] is
    /// a play and nothing outside the node knows how long one is.
    pub clip: bool,
}

impl Timing {
    /// A node whose picture comes back after one of its own cycles, at `pace`.
    pub const fn periodic(pace: f64) -> Self {
        Self::repeating(pace, |_| Some(1.0))
    }

    /// A node whose picture comes back where `period` says, at `pace`.
    pub const fn repeating(pace: f64, period: fn(&Node) -> Option<f64>) -> Self {
        Self {
            pace,
            still: false,
            period,
            axes: Axes::One,
            clip: false,
        }
    }

    /// The same, standing still when it is new: its Speed starts at 0.
    #[must_use]
    pub const fn still(mut self) -> Self {
        self.still = true;
        self
    }

    /// The same on two axes, X and Y.
    #[must_use]
    pub const fn xy(mut self) -> Self {
        self.axes = Axes::Two;
        self
    }

    /// The same paced by a length of its own: a play a cycle ([`Self::clip`]).
    #[must_use]
    pub const fn clip(mut self) -> Self {
        self.clip = true;
        self
    }

    /// Where a new node's Speed starts: 1, or 0 on one that stands still.
    pub const fn speed_default(&self) -> f32 {
        if self.still { 0.0 } else { 1.0 }
    }

    /// The axes this node has, X first.
    pub const fn axes(&self) -> &'static [Axis] {
        match self.axes {
            Axes::One => &[Axis::X],
            Axes::Two => &[Axis::X, Axis::Y],
        }
    }

    /// What the editor reads `node`'s Time round: its period, or [`phasor::WRAP`] for a
    /// picture that never repeats.
    pub fn wrap(&self, node: &Node) -> f64 {
        (self.period)(node)
            .filter(|p| *p > 0.0)
            .unwrap_or(phasor::WRAP)
    }
}

/// One axis's three rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axis {
    pub index: usize,
    pub time: &'static str,
    pub speed: &'static str,
    pub offset: &'static str,
}

impl Axis {
    pub const X: Self = Self {
        index: 0,
        time: TIME,
        speed: SPEED,
        offset: OFFSET,
    };
    pub const Y: Self = Self {
        index: 1,
        time: TIME_Y,
        speed: SPEED_Y,
        offset: OFFSET_Y,
    };
}

/// A row's label: the word alone on one axis, with the axis's letter on two.
const fn label(t: Timing, axis: usize, one: &'static str, xy: [&'static str; 2]) -> &'static str {
    match t.axes {
        Axes::One => one,
        Axes::Two => xy[if axis == 0 { 0 } else { 1 }],
    }
}

/// Axis `axis`'s **Time**: a diamond with no knob.
pub const fn time_row(t: Timing, axis: usize) -> InputDef {
    InputDef {
        key: if axis == 0 { TIME } else { TIME_Y },
        label: label(t, axis, "Time", ["Time X", "Time Y"]),
        ty: PortType::UniformNumber,
        control: Control::None,
    }
}

/// Axis `axis`'s **Speed**: −4 to 4 times the pace, from [`Timing::speed_default`].
pub const fn speed_row(t: Timing, axis: usize) -> InputDef {
    InputDef {
        key: if axis == 0 { SPEED } else { SPEED_Y },
        label: label(t, axis, "Speed", ["Speed X", "Speed Y"]),
        ty: PortType::UniformNumber,
        control: Control::num(t.speed_default(), -4.0, 4.0, 0.01, "×"),
    }
}

/// Axis `axis`'s **Offset**: a knob at zero that adds nothing, in cycles — `ty` a varying
/// number on a node that draws and a uniform number on a CPU node. The range declared here is
/// the one a node with no period has, −1 to 1; a node's own is [`range`].
pub const fn offset_row(t: Timing, axis: usize, ty: PortType) -> InputDef {
    let reach = offset_range(None);
    InputDef {
        key: if axis == 0 { OFFSET } else { OFFSET_Y },
        label: label(t, axis, "Offset", ["Offset X", "Offset Y"]),
        ty,
        control: Control::num(0.0, reach.min, reach.max, reach.step, ""),
    }
}

/// The steps an Offset knob takes, finest first: what the number control's readout shows, up
/// to a whole cycle.
const OFFSET_STEPS: [f32; 4] = [0.001, 0.01, 0.1, 1.0];

/// Offset's range on a node whose picture comes back every `period` of its cycles: one whole
/// period either way, or one cycle either way where `period` is `None` or not above zero. The
/// step is the finest of [`OFFSET_STEPS`] at least a thousandth of the period.
pub const fn offset_range(period: Option<f64>) -> ControlRange {
    let reach = match period {
        Some(p) if p.is_finite() && p > 0.0 => p,
        _ => 1.0,
    };
    let mut i = 0;
    while i + 1 < OFFSET_STEPS.len() && (OFFSET_STEPS[i] as f64) * 1000.0 < reach * (1.0 - 1e-6) {
        i += 1;
    }
    ControlRange {
        min: -reach as f32,
        max: reach as f32,
        step: OFFSET_STEPS[i],
    }
}

/// The range `node`'s input `key` has before a hand changed it, where its timing decides it:
/// an Offset's, by the period the node's options give it now ([`offset_range`]). `None` for
/// every other input, and on a node that does not move with time.
pub fn range(node: &Node, key: &str) -> Option<ControlRange> {
    let t = node.def.timing?;
    (key == OFFSET || key == OFFSET_Y).then(|| offset_range((t.period)(node)))
}

/// An Offset of `v` fitted to the range a `period` gives it: as it is inside, taken round the
/// period outside — the remainder keeps its sign, so it lands inside `−P` to `P` — and clamped
/// to one cycle either way where there is no period.
pub fn fit_offset(v: f32, period: Option<f64>) -> f32 {
    let r = offset_range(period);
    if (r.min..=r.max).contains(&v) {
        return v;
    }
    match period.filter(|p| p.is_finite() && *p > 0.0) {
        Some(p) => ((f64::from(v) % p) as f32).clamp(r.min, r.max),
        None => v.clamp(r.min, r.max),
    }
}

/// Bring each of `node`'s Offsets back inside its range after an edit that may have moved its
/// period — a Repeat, a depth wrap, a clip's Loop, a lane's length — by [`fit_offset`]. An
/// Offset whose range is the node's own, set by a hand, is left alone: that range did not move.
pub fn fit_offsets(node: &mut Node) {
    let Some(t) = node.def.timing else {
        return;
    };
    let period = (t.period)(node);
    for axis in t.axes() {
        if node
            .values
            .get(axis.offset)
            .and_then(Value::range)
            .is_some()
        {
            continue;
        }
        if let Some(ControlValue::Float(v)) = node.controls.get(axis.offset).copied() {
            let fitted = fit_offset(v, period);
            if fitted.to_bits() != v.to_bits() {
                node.controls
                    .insert(axis.offset, ControlValue::Float(fitted));
            }
        }
    }
}

/// A CPU node's inputs: its time rows on one axis — Time, Speed, and a uniform Offset — then
/// its own. A macro because `&'static [InputDef]` cannot be concatenated in a `const`.
macro_rules! timing_inputs {
    ($timing:expr; $($own:expr),* $(,)?) => {
        &[
            $crate::nodes::timing::time_row($timing, 0),
            $crate::nodes::timing::speed_row($timing, 0),
            $crate::nodes::timing::offset_row(
                $timing,
                0,
                $crate::graph::PortType::UniformNumber,
            ),
            $($own,)*
        ]
    };
}

pub(crate) use timing_inputs as inputs;

/// A hand-written node's options: its own, then [`HEADING`] and [`MODE`].
macro_rules! timing_options {
    ($($own:expr),* $(,)?) => {
        &[
            $($own,)*
            $crate::nodes::timing::HEADING,
            $crate::nodes::timing::MODE,
        ]
    };
}

pub(crate) use timing_options as options;

/// Whether an input is a Time, of either axis.
pub fn is_time(key: &str) -> bool {
    key == TIME || key == TIME_Y
}

/// Whether an input is a Speed, of either axis.
pub fn is_speed(key: &str) -> bool {
    key == SPEED || key == SPEED_Y
}

/// Whether an input folds under the Timing heading: a Time, a Speed or an Offset.
pub fn is_time_row(key: &str) -> bool {
    is_time(key) || is_speed(key) || key == OFFSET || key == OFFSET_Y
}

/// The axis a time row belongs to, by its key.
pub fn axis_of(key: &str) -> Option<Axis> {
    [Axis::X, Axis::Y]
        .into_iter()
        .find(|a| a.time == key || a.speed == key || a.offset == key)
}

/// Whether `node` runs free: it keeps time, and its mode is not [`LOOP`].
pub fn runs_free(node: &Node) -> bool {
    node.def.timing.is_some() && node.options.get(MODE.key).is_none_or(|mode| mode != LOOP)
}

/// Whether `node`'s input `key` is the row its mode puts away — a Time while it runs free, a
/// Speed while it loops — which draws no row and takes no cable.
pub fn is_inactive(node: &Node, key: &str) -> bool {
    if node.def.timing.is_none() {
        false
    } else if runs_free(node) {
        is_time(key)
    } else {
        is_speed(key)
    }
}

/// The most cycles a loop is cut into on its loop meter: a longer loop draws its fill and no
/// dividers.
pub const MAX_DIVIDED: u64 = 16;

/// The span a loop meter draws: the node's whole loop, or the one cycle it is in where its
/// picture never comes back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Span {
    /// A loop `period` of the node's cycles long, `cycles` of them counted whole — a period
    /// that is not whole rounded up, and one under a cycle counted as one — and the cycle within
    /// it the node is in, from zero.
    Loop {
        cycle: u64,
        cycles: u64,
        period: f64,
    },
    /// No loop: `count` whole cycles behind the node, and the span's right end open.
    Open { count: f64 },
}

/// Where a node is in its loop, from its Time reading and its period: what the loop meter on a
/// Time row draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// How far across the span, 0 up to 1, left to right.
    pub fill: f64,
    pub span: Span,
}

impl Progress {
    /// Where a Time reading of `time` is in a loop of `period` of the node's cycles, or in its
    /// current cycle where `period` is `None` or not above zero. Wrapped as a body takes Time
    /// round ([`phasor::fraction`]), so a Time a whole loop on reads as zero. A period that is
    /// a small fraction `p/q` — half a cycle, two and a half, a tenth — is read as that fraction
    /// (`chain::Fraction::near`): the Time is taken round it as `q × Time` round the whole `p`,
    /// so no rounding of `p/q` gathers over a long show.
    pub fn of(time: f64, period: Option<f64>) -> Self {
        let time = if time.is_finite() { time } else { 0.0 };
        let Some(period) = period.filter(|p| p.is_finite() && *p > 0.0) else {
            let fill = phasor::fraction(time, 1.0);
            return Self {
                fill,
                span: Span::Open {
                    count: (time - fill).round(),
                },
            };
        };
        let (at, period) = match crate::nodes::chain::Fraction::near(period) {
            Some(f) if f.q > 1 => {
                let (p, q) = (f.p as f64, f.q as f64);
                (phasor::fraction(time * q, p) / q, p / q)
            }
            _ => (phasor::fraction(time, period), period),
        };
        let cycles = (period - phasor::REACH).ceil().max(1.0) as u64;
        let cycle = ((at + phasor::REACH).floor().max(0.0) as u64).min(cycles - 1);
        Self {
            fill: at / period,
            span: Span::Loop {
                cycle,
                cycles,
                period,
            },
        }
    }

    /// Whether the span has no end to come back to.
    pub fn open(&self) -> bool {
        matches!(self.span, Span::Open { .. })
    }

    /// Where the hairlines between a loop's cycles go, as fractions of its width: one at each
    /// whole cycle inside the loop, and none on a loop of more than [`MAX_DIVIDED`] cycles or
    /// on an open span.
    pub fn dividers(&self) -> impl Iterator<Item = f64> {
        let (cycles, period) = match self.span {
            Span::Loop { cycles, period, .. } if cycles <= MAX_DIVIDED => (cycles, period),
            _ => (0, 1.0),
        };
        (1..cycles).map(move |k| k as f64 / period)
    }
}

/// How long a turned Speed takes to glide to its new value, in seconds of travel: short enough
/// that a knob answers at once, long enough that a stepped one — a MIDI knob's 128 values, a
/// drag a frame — bends the motion rather than kinking it.
pub const SPEED_GLIDE: f64 = 0.05;

/// A free-running node's own playheads, one per axis, in seconds of its pace: its Speed
/// integrated against the transport's advance ([`Time::advance`]), never a frame's `dt`. The
/// synth keeps one per node; the node's Time is this times its pace, or a clip's own rate.
///
/// Born where the playhead puts it, `speed × playhead`, as a Master Gear is born at
/// `playhead ÷ length`; a seek moves it by `speed × the jump`; a Speed turned glides by
/// [`SPEED_GLIDE`] in closed form (`phasor::Phasor`), so a stepped knob lands on the same
/// phase at 30, 60 or 144 frames a second.
#[derive(Debug, Clone)]
pub struct Pace {
    axes: [phasor::Phasor; 2],
}

impl Default for Pace {
    fn default() -> Self {
        let axis = phasor::Phasor::new(phasor::Birth::Zero).gliding(SPEED_GLIDE);
        Self {
            axes: [axis.clone(), axis],
        }
    }
}

impl Pace {
    /// Axis `axis`'s step this tick at `speed`: jumped where the tick holds a jump, or where
    /// the axis is born on it, which fires nothing.
    pub fn step(&mut self, axis: usize, speed: f64, time: &Time) -> phasor::Step {
        let phasor = &mut self.axes[axis.min(1)];
        let speed = if speed.is_finite() { speed } else { 0.0 };
        if !phasor.is_born() {
            phasor.set(speed * time.playhead);
            // The glide starts at the speed it is born at.
            phasor.advance(speed, 0.0);
            return phasor::Step {
                from: phasor.phase(),
                to: phasor.phase(),
                jumped: true,
            };
        }
        phasor.step(speed, time)
    }

    /// Axis `axis`'s playhead, in seconds of the node's pace. Zero before it is born.
    pub fn at(&self, axis: usize) -> f64 {
        self.axes[axis.min(1)].phase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(advance: f64) -> Time {
        Time {
            advance,
            ..Time::default()
        }
    }

    /// Run a pace at `fps` for `seconds`, its Speed stepped from `a` to `b` at `at` seconds.
    fn run(fps: f64, seconds: f64, a: f64, b: f64, at: f64) -> f64 {
        let mut p = Pace::default();
        p.step(0, a, &tick(0.0));
        let frames = (seconds * fps).round() as u32;
        let switch = (at * fps).round() as u32;
        for i in 0..frames {
            p.step(0, if i < switch { a } else { b }, &tick(1.0 / fps));
        }
        p.at(0)
    }

    /// A Speed stepped at one moment lands on the same phase at 30, 60 and 144 frames a
    /// second: the glide is integrated in closed form, not frame by frame.
    #[test]
    fn a_stepped_speed_lands_on_the_same_phase_at_any_frame_rate() {
        let at30 = run(30.0, 3.0, 1.0, -2.0, 1.0);
        let at60 = run(60.0, 3.0, 1.0, -2.0, 1.0);
        let at144 = run(144.0, 3.0, 1.0, -2.0, 1.0);
        assert!((at30 - at60).abs() < 1e-9, "{at30} {at60}");
        assert!((at60 - at144).abs() < 1e-9, "{at60} {at144}");
        let glide = (1.0 + 2.0) * SPEED_GLIDE * (1.0 - (-2.0 / SPEED_GLIDE).exp());
        assert!((at60 - (1.0 - 4.0 + glide)).abs() < 1e-9, "{at60}");
    }

    /// Paused, the transport advances nothing, and a pace holds where it stands.
    #[test]
    fn a_pause_holds_it() {
        let mut p = Pace::default();
        p.step(0, 2.0, &tick(0.0));
        for _ in 0..30 {
            p.step(0, 2.0, &tick(1.0 / 60.0));
        }
        let at = p.at(0);
        for _ in 0..120 {
            p.step(
                0,
                2.0,
                &Time {
                    playing: false,
                    ..tick(0.0)
                },
            );
        }
        assert_eq!(p.at(0), at, "paused, it holds");
        assert!((at - 1.0).abs() < 1e-9, "{at}");
    }

    /// A seek moves it by speed times the jump, and fires nothing.
    #[test]
    fn a_seek_moves_it_by_speed_times_the_jump() {
        let mut p = Pace::default();
        p.step(0, 3.0, &tick(0.0));
        let step = p.step(
            0,
            3.0,
            &Time {
                advance: -4.0,
                jumped: true,
                ..Time::default()
            },
        );
        assert!((p.at(0) + 12.0).abs() < 1e-12, "{}", p.at(0));
        assert!(step.jumped, "a seek is a jump");
        assert_eq!(step.crossings(0.0).count(), 0, "and crosses nothing");
    }

    /// Born where the playhead puts it, at speed times the playhead: what Loop mode's ambient
    /// reading is at Speed 1, and the same however it was reached — so a render, which starts
    /// every pace again, is deterministic.
    #[test]
    fn it_is_born_where_the_playhead_puts_it() {
        let at = |speed: f64, playhead: f64| {
            let mut p = Pace::default();
            let step = p.step(
                1,
                speed,
                &Time {
                    playhead,
                    advance: 0.25,
                    ..Time::default()
                },
            );
            assert!(step.jumped, "a birth fires nothing");
            p.at(1)
        };
        assert_eq!(at(1.0, 12.5), 12.5);
        assert_eq!(at(-2.0, 3.0), -6.0);
        assert_eq!(at(0.0, 99.0), 0.0, "a still node is born at zero");
        assert_eq!(at(1.5, -2.0), at(1.5, -2.0));
    }

    fn loop_of(p: Progress) -> (u64, u64) {
        match p.span {
            Span::Loop { cycle, cycles, .. } => (cycle, cycles),
            Span::Open { .. } => panic!("a loop, not {p:?}"),
        }
    }

    /// A Repeat of 4: four cycles, the one the node is in counted from zero, filled by how far
    /// it is through all four, and three hairlines at the quarters.
    #[test]
    fn a_loop_is_cut_into_its_cycles_and_filled_across_them() {
        let p = Progress::of(2.5, Some(4.0));
        assert_eq!(loop_of(p), (2, 4));
        assert!((p.fill - 0.625).abs() < 1e-12, "{}", p.fill);
        assert!(!p.open());
        assert_eq!(p.dividers().collect::<Vec<_>>(), [0.25, 0.5, 0.75]);
        let start = Progress::of(0.0, Some(4.0));
        assert_eq!((loop_of(start), start.fill), ((0, 4), 0.0));
        let last = Progress::of(3.99, Some(4.0));
        assert_eq!(loop_of(last), (3, 4));
    }

    /// It wraps where the node comes back: a loop on, and before zero, read as a body reads
    /// them, and a whole loop on a hair either side is the loop's start.
    #[test]
    fn a_loop_wraps_where_the_node_comes_back() {
        let a = Progress::of(1.25, Some(4.0));
        assert_eq!(Progress::of(1.25 + 4.0 * 1000.0, Some(4.0)), a);
        let back = Progress::of(-0.5, Some(4.0));
        assert_eq!(loop_of(back), (3, 4));
        assert!((back.fill - 0.875).abs() < 1e-12);
        for hair in [8.0 - 1e-12, 8.0, 8.0 + 1e-12] {
            let p = Progress::of(hair, Some(4.0));
            assert_eq!((loop_of(p), p.fill), ((0, 4), 0.0), "{hair}");
        }
        let edge = Progress::of(2.0 - 1e-12, Some(4.0));
        assert_eq!(
            loop_of(edge),
            (2, 4),
            "a hair under a whole cycle is that cycle"
        );
    }

    /// A periodic node's loop is its one cycle: no hairline, filled by the cycle.
    #[test]
    fn a_periodic_loop_is_one_cycle() {
        let p = Progress::of(7.3, Some(1.0));
        assert_eq!(loop_of(p), (0, 1));
        assert!((p.fill - 0.3).abs() < 1e-9);
        assert_eq!(p.dividers().count(), 0);
    }

    /// Up to sixteen cycles take hairlines; the tunnel's 64 is a fill alone, still counted.
    #[test]
    fn no_dividers_above_the_cap() {
        let sixteen = Progress::of(0.0, Some(MAX_DIVIDED as f64));
        assert_eq!(sixteen.dividers().count(), 15);
        let tunnel = Progress::of(40.5, Some(64.0));
        assert_eq!(loop_of(tunnel), (40, 64));
        assert_eq!(tunnel.dividers().count(), 0);
    }

    /// A period that is not whole counts its last, short cycle and puts a hairline at each
    /// whole cycle inside it.
    #[test]
    fn a_period_that_is_not_whole_rounds_its_cycles_up() {
        let p = Progress::of(2.2, Some(2.5));
        assert_eq!(loop_of(p), (2, 3));
        assert_eq!(p.dividers().collect::<Vec<_>>(), [0.4, 0.8]);
    }

    /// A loop shorter than a cycle — four on the floor's quarter of a bar, a tenth — is one
    /// cycle counted, no hairline, and filled across the loop, wrapping as often as it comes
    /// back in a cycle; a third, which an `f64` cannot hold, is read as a third, so a Time
    /// a million loops on reads where it read at the start.
    #[test]
    fn a_loop_shorter_than_a_cycle_fills_as_often_as_it_comes_back() {
        let p = Progress::of(0.3, Some(0.25));
        assert_eq!(loop_of(p), (0, 1));
        assert!((p.fill - 0.2).abs() < 1e-9, "{}", p.fill);
        assert_eq!(p.dividers().count(), 0);
        assert_eq!(Progress::of(7.0, Some(0.25)).fill, 0.0);
        let tenth = Progress::of(2.37, Some(0.1));
        assert!((tenth.fill - 0.7).abs() < 1e-6, "{}", tenth.fill);
        let third = 1.0 / 3.0;
        let a = Progress::of(0.25, Some(third));
        let b = Progress::of(0.25 + 1e6, Some(third));
        assert!((a.fill - 0.75).abs() < 1e-9, "{}", a.fill);
        assert!((a.fill - b.fill).abs() < 1e-6, "{} {}", a.fill, b.fill);
        assert_eq!(
            Progress::of(1e6, Some(third)).fill,
            0.0,
            "a whole number of thirds"
        );
        // Two and a half cycles, written as the fraction a sequencer's lanes give.
        let p = Progress::of(2.2 + 2.5 * 4000.0, Some(40.0 / 16.0));
        assert_eq!(loop_of(p), (2, 3));
        assert!((p.fill - 0.88).abs() < 1e-9, "{}", p.fill);
    }

    /// A picture that never comes back has no loop: the cycle it is in, filled, with the
    /// whole cycles behind it, and nothing to divide.
    #[test]
    fn no_period_is_an_open_span_over_the_current_cycle() {
        for period in [None, Some(0.0), Some(-3.0), Some(f64::NAN)] {
            let p = Progress::of(12.25, period);
            assert!(p.open(), "{period:?}");
            assert_eq!(p.span, Span::Open { count: 12.0 });
            assert!((p.fill - 0.25).abs() < 1e-12);
            assert_eq!(p.dividers().count(), 0);
        }
        assert_eq!(
            Progress::of(-0.25, None).span,
            Span::Open { count: -1.0 },
            "before zero, the whole cycle under it"
        );
        let hair = Progress::of(3.0 - 1e-12, None);
        assert_eq!((hair.span, hair.fill), (Span::Open { count: 3.0 }, 0.0));
        assert_eq!(
            Progress::of(f64::INFINITY, None).span,
            Span::Open { count: 0.0 }
        );
    }

    /// The rows a timing expands into: Time with no knob, Speed from 1 or 0, Offset from zero
    /// declared one cycle either way, labelled by axis where there are two.
    #[test]
    fn a_timing_expands_into_its_rows() {
        let moving = Timing::periodic(0.5);
        let still = Timing::periodic(0.1).still();
        let shaky = Timing::periodic(1.0 / 60.0).xy();
        let time = time_row(moving, 0);
        assert_eq!((time.key, time.label), (TIME, "Time"));
        assert!(time.ty == PortType::UniformNumber && matches!(time.control, Control::None));
        for (t, want) in [(moving, 1.0), (still, 0.0)] {
            let speed = speed_row(t, 0);
            assert_eq!((speed.key, speed.label), (SPEED, "Speed"));
            assert!(matches!(
                speed.control,
                Control::Number { default, min, max, .. }
                    if default == want && min == -4.0 && max == 4.0
            ));
        }
        let offset = offset_row(moving, 0, PortType::VaryingNumber);
        assert_eq!((offset.key, offset.ty), (OFFSET, PortType::VaryingNumber));
        assert!(matches!(
            offset.control,
            Control::Number { default, min, max, step, .. }
                if default == 0.0 && min == -1.0 && max == 1.0 && step == 0.001
        ));
        let labels: Vec<_> = (0..2)
            .flat_map(|a| {
                [
                    time_row(shaky, a),
                    speed_row(shaky, a),
                    offset_row(shaky, a, PortType::VaryingNumber),
                ]
            })
            .map(|i| (i.key, i.label))
            .collect();
        assert_eq!(
            labels,
            [
                (TIME, "Time X"),
                (SPEED, "Speed X"),
                (OFFSET, "Offset X"),
                (TIME_Y, "Time Y"),
                (SPEED_Y, "Speed Y"),
                (OFFSET_Y, "Offset Y"),
            ]
        );
        assert_eq!(shaky.axes(), [Axis::X, Axis::Y]);
        assert_eq!(MODE.default, FREE);
        assert_eq!(
            HEADING.default,
            crate::nodes::OFF,
            "the heading starts closed"
        );
    }
    /// Offset reaches one whole period either way: ±1 on a periodic node, ±4 at Repeat 4, ±64
    /// on the tunnel, and one cycle either way with no period. Its step is the finest that is a
    /// thousandth of the period or more, so both ends take a drag of about the same length.
    #[test]
    fn offset_reaches_a_period_either_way() {
        let r = |period| {
            let r = offset_range(period);
            (r.min, r.max, r.step)
        };
        assert_eq!(r(Some(1.0)), (-1.0, 1.0, 0.001));
        assert_eq!(r(Some(2.0)), (-2.0, 2.0, 0.01));
        assert_eq!(r(Some(4.0)), (-4.0, 4.0, 0.01));
        assert_eq!(r(Some(16.0)), (-16.0, 16.0, 0.1));
        assert_eq!(r(Some(64.0)), (-64.0, 64.0, 0.1));
        assert_eq!(r(Some(128.0)), (-128.0, 128.0, 1.0));
        assert_eq!(r(Some(3.0)), (-3.0, 3.0, 0.01), "a sequencer's three bars");
        for none in [
            None,
            Some(0.0),
            Some(-4.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(r(none), (-1.0, 1.0, 0.001), "{none:?}");
        }
    }

    /// An Offset outside its range is taken round the period, keeping its sign, so a picture
    /// that comes back every period is the same picture; with no period it is clamped to one
    /// cycle. Inside, ends included, it is left as it is.
    #[test]
    fn an_offset_outside_is_taken_round_its_period() {
        assert_eq!(fit_offset(10.0, Some(4.0)), 2.0);
        assert_eq!(fit_offset(-10.0, Some(4.0)), -2.0);
        assert_eq!(fit_offset(-9.5, Some(4.0)), -1.5);
        assert_eq!(fit_offset(12.0, Some(4.0)), 0.0);
        for inside in [-4.0, -3.25, 0.0, 2.5, 4.0] {
            assert_eq!(fit_offset(inside, Some(4.0)), inside);
        }
        assert_eq!(fit_offset(2.5, Some(1.0)), 0.5);
        assert_eq!(fit_offset(3.0, None), 1.0);
        assert_eq!(fit_offset(-3.0, None), -1.0);
        assert_eq!(fit_offset(-0.4, None), -0.4);
    }

    /// Every node that moves with time has an Offset whose range is its period either way,
    /// read from its options and controls as they stand: a Perlin by its Repeat, the tunnel by
    /// its depth wrap, a clip by its Loop, a Euclidean Rhythm by where its lanes meet again.
    #[test]
    fn every_moving_nodes_offset_reaches_its_own_period() {
        let mut g = crate::graph::Graph::new();
        for def in crate::nodes::REGISTRY {
            let Some(t) = def.timing else { continue };
            let id = crate::nodes::add_to_graph(&mut g, def.slug, emath::Pos2::ZERO).unwrap();
            let node = g.get(id).unwrap();
            for axis in t.axes() {
                assert_eq!(
                    crate::nodes::control_range(def, node, axis.offset),
                    Some(offset_range((t.period)(node))),
                    "{} {}",
                    def.slug,
                    axis.offset
                );
            }
        }
        let reach = |g: &crate::graph::Graph, id| {
            let node = g.get(id).unwrap();
            crate::nodes::control_range(node.def, node, OFFSET)
                .unwrap()
                .max
        };
        let set = |g: &mut crate::graph::Graph, id, key: &'static str, value: &str| {
            g.get_mut(id)
                .unwrap()
                .options
                .insert(key, value.to_string());
        };
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        assert_eq!(reach(&g, perlin), 1.0, "Repeat Never: one cycle either way");
        for (repeat, want) in [("4", 4.0), ("16", 16.0), ("1", 1.0)] {
            set(&mut g, perlin, "repeat", repeat);
            assert_eq!(reach(&g, perlin), want, "Repeat {repeat}");
        }
        let tunnel = crate::nodes::add_to_graph(&mut g, "tunnel3d", emath::Pos2::ZERO).unwrap();
        assert_eq!(reach(&g, tunnel), 64.0);
        set(&mut g, tunnel, "wrap", "none");
        assert_eq!(reach(&g, tunnel), 1.0);
        let clip = crate::nodes::add_to_graph(&mut g, "video", emath::Pos2::ZERO).unwrap();
        assert_eq!(reach(&g, clip), 1.0);
        set(&mut g, clip, "loop", "hold");
        assert_eq!(reach(&g, clip), 1.0);
        let euclid =
            crate::nodes::add_to_graph(&mut g, "euclideanrhythm", emath::Pos2::ZERO).unwrap();
        assert_eq!(reach(&g, euclid), 1.0, "four figures of 16 meet every bar");
        let controls = &mut g.get_mut(euclid).unwrap().controls;
        controls.insert("lane2steps", ControlValue::Float(12.0));
        controls.insert("lane2pulses", ControlValue::Float(5.0));
        assert_eq!(
            reach(&g, euclid),
            3.0,
            "16 and E(5, 12) meet every three bars"
        );
        g.get_mut(euclid)
            .unwrap()
            .controls
            .insert("lane2pulses", ControlValue::Float(3.0));
        assert_eq!(
            reach(&g, euclid),
            1.0,
            "E(3, 12) comes back every four steps"
        );
    }

    /// Fitting a node's Offsets after its period shrank: taken round the new period on each
    /// axis, and left alone where a hand gave the Offset a range of its own.
    #[test]
    fn fitting_a_nodes_offsets_takes_them_round_its_period() {
        let mut g = crate::graph::Graph::new();
        let perlin = crate::nodes::add_to_graph(&mut g, "perlin", emath::Pos2::ZERO).unwrap();
        let node = g.get_mut(perlin).unwrap();
        node.options.insert("repeat", "16".to_string());
        node.controls.insert(OFFSET, ControlValue::Float(10.0));
        fit_offsets(node);
        assert_eq!(
            node.controls[OFFSET],
            ControlValue::Float(10.0),
            "inside ±16"
        );
        node.options.insert("repeat", "4".to_string());
        fit_offsets(node);
        assert_eq!(
            node.controls[OFFSET],
            ControlValue::Float(2.0),
            "10 is 2 round 4"
        );
        node.options.insert("repeat", "never".to_string());
        node.controls.insert(OFFSET, ControlValue::Float(-3.5));
        fit_offsets(node);
        assert_eq!(
            node.controls[OFFSET],
            ControlValue::Float(-1.0),
            "no period: clamped"
        );

        let shaky = crate::nodes::add_to_graph(&mut g, "shakycam", emath::Pos2::ZERO).unwrap();
        let node = g.get_mut(shaky).unwrap();
        node.controls.insert(OFFSET, ControlValue::Float(1.5));
        node.controls.insert(OFFSET_Y, ControlValue::Float(-2.25));
        fit_offsets(node);
        assert_eq!(node.controls[OFFSET], ControlValue::Float(0.5));
        assert_eq!(node.controls[OFFSET_Y], ControlValue::Float(-0.25), "Y too");

        node.values.insert(
            OFFSET,
            Value::Range(ControlRange {
                min: -8.0,
                max: 8.0,
                step: 0.01,
            }),
        );
        node.controls.insert(OFFSET, ControlValue::Float(6.0));
        fit_offsets(node);
        assert_eq!(
            node.controls[OFFSET],
            ControlValue::Float(6.0),
            "a hand's own range did not move"
        );
    }
}
