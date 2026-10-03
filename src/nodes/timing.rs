// SPDX-License-Identifier: AGPL-3.0-or-later

//! Time, for every node that moves with it. The one place its rules live: the declaration a
//! node makes, the rows and options that declaration expands into, the Free integrator, what
//! a CPU node reads, the WGSL a body reads it through, and what `nodes::chain` reads to say
//! whether a loop closes. `docs/nodes.md#timing` is the spec and
//! `docs/decisions.md#time-is-an-input-port-not-an-ambient-global` the argument.
//!
//! **One clock.** The transport's playhead is the only clock (`transport.rs`); nothing here
//! keeps a timer. A node that moves with time declares a [`Timing`] and nothing else about
//! time: its rate at rest, its pace, the period its picture comes back after, and whether it
//! has one axis or two. Everything else is expanded from that, so no two nodes can disagree
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
//!   ambient time, `playhead × rate`; plugged, what arrives replaces it, usually a gear's
//!   Cycles, which drives the node exactly and closes a loop to the bit.
//!
//! Either way the synth publishes where the node is as a **count** under the Time key —
//! `u_count_{slug}{id}_clock`, split into a whole part and a fraction (`phasor::split`) — so
//! a body reads Time and never Speed, and a count is as precise a million cycles on as at the
//! first. Switching mode drops the cable in the row that goes away in the same step
//! (`Graph::drop_inactive`), since a gear's Cycles, a growing count, would race off as a
//! speed; a cable can never land on the row a mode puts away ([`is_inactive`]).
//!
//! **Offset** ([`OFFSET`]) is added on top in both modes: 0 to 1 is one of the node's cycles,
//! a varying number on a node that draws, so a field ripples it, and a uniform number on a CPU
//! node, whose tick has no pixel.
//!
//! **The rows** are Time, Speed and Offset per axis, folded under one Timing heading
//! ([`HEADING`]) that starts closed; the mode is two segments on the heading's bar. The mode
//! shows one of Time and Speed and puts the other away in place, so the node keeps its height.
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

use crate::graph::{Node, PortType};
use crate::nodes::{Control, InputDef, OptionDef, OptionKind, phasor};
use crate::transport::Time;

/// The key of a time-driven node's **Time**, the first time row in Loop mode: a diamond with
/// no knob, in the node's own cycles. Unplugged, ambient time at the node's rest rate, which
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
    /// How many of the node's own cycles one ambient second is at rest in Loop mode: silvia's
    /// default speed, rounded to whole seconds where it was 20π, and zero where silvia is
    /// still.
    pub rate: f64,
    /// How many of its cycles one second is at a Speed of 1 in Free mode: the rate where that
    /// is not zero, and a pace that looks natural where silvia sits still — whose Speed then
    /// starts at zero, so a new one still sits still.
    pub pace: f64,
    /// How long the picture takes to come back, in its own units, by what its options say:
    /// one for a periodic node, a noise's Repeat, the tunnel's 64 while its depth wraps, and
    /// `None` for a picture that never repeats.
    pub period: fn(&Node) -> Option<f64>,
    pub axes: Axes,
}

impl Timing {
    /// A node whose picture comes back after one of its own cycles, at `rate` at rest and as
    /// its pace.
    pub const fn periodic(rate: f64) -> Self {
        Self::repeating(rate, |_| Some(1.0))
    }

    /// A node whose picture comes back where `period` says, at `rate` at rest and as its pace.
    pub const fn repeating(rate: f64, period: fn(&Node) -> Option<f64>) -> Self {
        Self {
            rate,
            pace: rate,
            period,
            axes: Axes::One,
        }
    }

    /// The same with a pace of its own: a node whose rate at rest is zero.
    #[must_use]
    pub const fn paced(mut self, pace: f64) -> Self {
        self.pace = pace;
        self
    }

    /// The same on two axes, X and Y.
    #[must_use]
    pub const fn xy(mut self) -> Self {
        self.axes = Axes::Two;
        self
    }

    /// Where a new node's Speed starts: 1 on a node that moves at rest, 0 on one that sits
    /// still.
    pub const fn speed_default(&self) -> f32 {
        if self.rate == 0.0 { 0.0 } else { 1.0 }
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

/// Axis `axis`'s **Offset**: a knob at zero that adds nothing, 0 to 1 one cycle — `ty` a
/// varying number on a node that draws and a uniform number on a CPU node.
pub const fn offset_row(t: Timing, axis: usize, ty: PortType) -> InputDef {
    InputDef {
        key: if axis == 0 { OFFSET } else { OFFSET_Y },
        label: label(t, axis, "Offset", ["Offset X", "Offset Y"]),
        ty,
        control: Control::num(0.0, 0.0, 1.0, 0.001, ""),
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

    /// The rows a timing expands into: Time with no knob, Speed from 1 or 0, Offset one cycle
    /// from zero, labelled by axis where there are two.
    #[test]
    fn a_timing_expands_into_its_rows() {
        let moving = Timing::periodic(0.5);
        let still = Timing::periodic(0.0).paced(0.1);
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
            Control::Number { default, min, max, .. } if default == 0.0 && min == 0.0 && max == 1.0
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
}
