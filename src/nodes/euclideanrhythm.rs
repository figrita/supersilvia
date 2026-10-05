// SPDX-License-Identifier: AGPL-3.0-or-later

//! Four independent Euclidean sequencers behind one sixteenth-note clock. Each lane's own
//! `steps`, `pulses` and `rotation` are ports here — silvia fixes `steps` at 16 with no UI
//! for it at all, and this node's shape asks for it per lane, so an absolute step position
//! is shared and each lane reads its own pattern at that position modulo its own length.
//!
//! **The pattern is Bjorklund's, the textbook construction** — pair every pulse group with a
//! rest group, then pair what is left over, and again, until one group of remainder is left
//! — and **not** silvia's `_generateEuclideanPattern`, which rasterizes additively:
//! accumulate `pulses / steps` and mark a pulse each time the running error crosses one.
//! The two draw the same necklace for every count this node accepts; they disagree only
//! about where it starts. silvia's (8, 3) at `rotation` zero is `..x..x.x`, which reaches
//! the textbook tresillo `x..x..x.` only at rotation 1, because its accumulator needs more
//! than one step of slope before it first crosses.
//!
//! **This is a departure from silvia, made deliberately.** Rotation zero starts on a pulse
//! here, so E(3, 8) is the tresillo, E(5, 8) the cinquillo, E(5, 16) the bossa figure and
//! E(7, 16) the samba, each as published. The cost is that a lane imported from a silvia
//! patch reads one step early: silvia's figure at rotation `r` is this one at rotation
//! `r - 1`, so a rotation carried across wants decrementing by one to sound as it did there.
//!
//! **The clock is `nodes::sequencer`'s**, which `stepsequencer` runs behind too: Time and
//! Offset in bars, Step, a Gate, and four gate lanes. This node answers only what a lane
//! plays at a step.
//!
//! **The figure is drawn on the node, and the twelve numbers that shape it are the node's
//! own values.** silvia's four lanes of lit cells with the playhead walking across them are
//! [`Region::Steps`](super::Region::Steps), and Steps, Pulses and Rotation are hidden
//! controls — no port, no cable, no row — on a slab per lane under the grid. They were twelve `UniformNumber`
//! ports; with the grid there, twelve rows with ports on them were the wrong shape, and what
//! a patch drives is Time, Offset, Step and Gate, which stay ports. The grid is
//! read-only here; the node whose cells a hand lights one by one is `stepsequencer`, drawn by
//! the same grid.

use crate::graph::NodeId;
use crate::graph::PortType::UniformNumber;
use crate::nodes::sequencer::{self, Transport};
use crate::nodes::{Category, Control, CpuDef, CpuNode, InputDef, NodeDef, TickContext};

pub static DEF: NodeDef = NodeDef {
    slug: "euclideanrhythm",
    category: Category::Control,
    icon: "⚪",
    label: "Euclidean Rhythm",
    tooltip: "Generate Euclidean rhythms with pulse count and rotation controls for each lane; Speed 1 plays them a bar every two seconds, or a Master Gear a bar long cabled into Time does, a bar a cycle.",
    inputs: sequencer::INPUTS,
    timing: Some(crate::nodes::Timing {
        period: |node| Some(bars(node)),
        ..sequencer::TIMING
    }),
    options: crate::nodes::timing::options![],
    row_headings: crate::nodes::timing::ROW_HEADINGS,
    // The twelve a lane is shaped by: values the node keeps, drawn three across on a slab
    // per lane under the grid rather than on twelve rows with ports nobody cabled. Pulses is
    // capped by that lane's own Steps, so a 32-step lane can be filled the whole way rather
    // than stopping half empty at silvia's sixteen.
    hidden: &[
        InputDef {
            key: "lane1steps",
            label: "Lane 1 Steps",
            ty: UniformNumber,
            control: Control::num(16.0, 1.0, 64.0, 1.0, ""),
        },
        InputDef {
            key: "lane1pulses",
            label: "Lane 1 Pulses",
            ty: UniformNumber,
            control: Control::num_capped(4.0, 0.0, 64.0, 1.0, "", "lane1steps"),
        },
        InputDef {
            key: "lane1rotation",
            label: "Lane 1 Rotation",
            ty: UniformNumber,
            control: Control::num(0.0, -8.0, 8.0, 1.0, ""),
        },
        InputDef {
            key: "lane2steps",
            label: "Lane 2 Steps",
            ty: UniformNumber,
            control: Control::num(16.0, 1.0, 64.0, 1.0, ""),
        },
        InputDef {
            key: "lane2pulses",
            label: "Lane 2 Pulses",
            ty: UniformNumber,
            control: Control::num_capped(3.0, 0.0, 64.0, 1.0, "", "lane2steps"),
        },
        InputDef {
            key: "lane2rotation",
            label: "Lane 2 Rotation",
            ty: UniformNumber,
            control: Control::num(0.0, -8.0, 8.0, 1.0, ""),
        },
        InputDef {
            key: "lane3steps",
            label: "Lane 3 Steps",
            ty: UniformNumber,
            control: Control::num(16.0, 1.0, 64.0, 1.0, ""),
        },
        InputDef {
            key: "lane3pulses",
            label: "Lane 3 Pulses",
            ty: UniformNumber,
            control: Control::num_capped(5.0, 0.0, 64.0, 1.0, "", "lane3steps"),
        },
        InputDef {
            key: "lane3rotation",
            label: "Lane 3 Rotation",
            ty: UniformNumber,
            control: Control::num(0.0, -8.0, 8.0, 1.0, ""),
        },
        InputDef {
            key: "lane4steps",
            label: "Lane 4 Steps",
            ty: UniformNumber,
            control: Control::num(16.0, 1.0, 64.0, 1.0, ""),
        },
        InputDef {
            key: "lane4pulses",
            label: "Lane 4 Pulses",
            ty: UniformNumber,
            control: Control::num_capped(2.0, 0.0, 64.0, 1.0, "", "lane4steps"),
        },
        InputDef {
            key: "lane4rotation",
            label: "Lane 4 Rotation",
            ty: UniformNumber,
            control: Control::num(0.0, -8.0, 8.0, 1.0, ""),
        },
    ],
    regions: &[crate::nodes::Region::Steps],
    outputs: sequencer::OUTPUTS,
    cpu: Some(CpuDef {
        create: || Box::new(EuclideanRhythm::default()),
        integrates: true,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// How many bars a Euclidean Rhythm takes to come back: until every lane's figure comes round
/// again, each at the least shift that leaves it as it is (`sequencer::bars`). A lane's figure
/// is E(k, n) repeated, `n ÷ gcd(n, k)` steps long, and one step where the lane is silent or
/// full; Rotation moves a figure and not its length.
fn bars(node: &crate::graph::Node) -> f64 {
    sequencer::bars((0..4).map(|lane| {
        let lane = LaneParams::of(node, lane);
        (lane.pattern, lane.steps)
    }))
}

/// The three keys each lane is shaped by, which the grid on the node reads as well as the
/// tick that runs it.
pub const STEPS_KEYS: [&str; 4] = ["lane1steps", "lane2steps", "lane3steps", "lane4steps"];
pub const PULSES_KEYS: [&str; 4] = ["lane1pulses", "lane2pulses", "lane3pulses", "lane4pulses"];
pub const ROTATION_KEYS: [&str; 4] = [
    "lane1rotation",
    "lane2rotation",
    "lane3rotation",
    "lane4rotation",
];

/// One lane's shape: how long it is, where it starts, and the figure itself.
pub struct LaneParams {
    pub steps: u32,
    pub rotation: i32,
    /// The lane's unrotated pattern, built once a tick rather than per step boundary: a
    /// frame can cross sixty-four of them, and the pattern only depends on the inputs.
    pattern: u64,
}

impl LaneParams {
    /// One lane as the document has it, for the grid on the node.
    ///
    /// The tick asks `TickContext::input` instead, which resolves a cable where there is one;
    /// these three have no port, so the control *is* the value and the picture and the
    /// sequencer read the same figure out of the same numbers.
    pub fn of(node: &crate::graph::Node, lane: usize) -> Self {
        let read = |keys: [&str; 4]| match node.controls.get(keys[lane]) {
            Some(&crate::graph::ControlValue::Float(v)) => v,
            _ => 0.0,
        };
        let steps = read(STEPS_KEYS).round().clamp(1.0, 64.0) as u32;
        Self {
            steps,
            rotation: read(ROTATION_KEYS).round() as i32,
            pattern: euclidean_pattern(read(PULSES_KEYS).round().clamp(0.0, 64.0) as u32, steps),
        }
    }
}

/// The Euclidean rhythm of `pulses` pulses over `steps` steps, as a bitmask with bit `i` set
/// when step `i` is a pulse. Bjorklund's construction: start with `pulses` groups of one
/// pulse and `steps - pulses` groups of one rest, repeatedly append a rest group to a pulse
/// group until fewer than two groups of remainder are left, then read the groups out in
/// order. Step zero is always a pulse, so E(3, 8) is the tresillo `x..x..x.`.
///
/// `steps` is at most 64 — the node's own clamp — so the whole pattern is one `u64`, every
/// intermediate group fits one too, and the construction runs entirely on the stack: a
/// `tick` allocates nothing to ask for a rhythm.
fn euclidean_pattern(pulses: u32, steps: u32) -> u64 {
    debug_assert!(
        steps <= 64,
        "the caller clamps `steps` to the width of the mask"
    );
    if steps == 0 || pulses == 0 {
        return 0;
    }
    if pulses >= steps {
        return if steps >= 64 {
            u64::MAX
        } else {
            (1 << steps) - 1
        };
    }

    // A group is a bit pattern read from bit 0 up, beside its own length, so appending `h`
    // to `g` is `g | (h << g_len)`. `a` holds the groups being built, `b` the remainder.
    let mut a_bits = [0u64; 64];
    let mut a_len = [0u8; 64];
    let mut b_bits = [0u64; 64];
    let mut b_len = [0u8; 64];
    let mut na = pulses as usize;
    let mut nb = (steps - pulses) as usize;
    a_bits[..na].fill(1);
    a_len[..na].fill(1);
    b_len[..nb].fill(1);

    while nb > 1 {
        let pairs = na.min(nb);
        let mut next_bits = [0u64; 64];
        let mut next_len = [0u8; 64];
        for i in 0..pairs {
            next_bits[i] = a_bits[i] | (b_bits[i] << u32::from(a_len[i]));
            next_len[i] = a_len[i] + b_len[i];
        }
        // Whichever list the pairing did not exhaust is the new remainder. Both arrays are
        // `Copy`, so this reads the old list even while `b` is written back into.
        let (rest_bits, rest_len, rest) = if na > pairs {
            (a_bits, a_len, na - pairs)
        } else {
            (b_bits, b_len, nb - pairs)
        };
        b_bits[..rest].copy_from_slice(&rest_bits[pairs..pairs + rest]);
        b_len[..rest].copy_from_slice(&rest_len[pairs..pairs + rest]);
        a_bits = next_bits;
        a_len = next_len;
        na = pairs;
        nb = rest;
    }

    let mut pattern = 0;
    let mut at = 0;
    for i in 0..na + nb {
        let (bits, len) = if i < na {
            (a_bits[i], a_len[i])
        } else {
            (b_bits[i - na], b_len[i - na])
        };
        pattern |= bits << at;
        at += u32::from(len);
    }
    pattern
}

/// Is the given absolute step a pulse for one lane, wrapped to its own step count and
/// rotated by its own `rotation`? silvia's `_rotatePattern` places `src[i]` at `dest[(i +
/// rotation) mod steps]`, so reading `dest[j]` back is `src[(j - rotation) mod steps]`.
pub fn is_pulse(absolute_step: i64, lane: &LaneParams) -> bool {
    if lane.steps == 0 {
        return false;
    }
    let steps = i64::from(lane.steps);
    let j = absolute_step.rem_euclid(steps);
    let src = (j - i64::from(lane.rotation)).rem_euclid(steps);
    lane.pattern >> src & 1 == 1
}

#[derive(Default)]
struct EuclideanRhythm {
    transport: Transport,
    /// The least common multiple of the four lanes' lengths on the last tick: what the grid's
    /// playhead is taken modulo.
    period: i64,
}

impl CpuNode for EuclideanRhythm {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn debug(&self) -> Option<String> {
        Some(self.transport.debug())
    }

    /// Which cell the grid on the node lights: the absolute step, not a fraction of a clip.
    /// A reading, like every playhead — what moves it is its Time, or Step while that is
    /// cabled.
    fn playhead(&self) -> Option<f32> {
        self.transport.playhead(self.period)
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let lanes: [LaneParams; 4] = std::array::from_fn(|i| {
            let steps = ctx.input(id, STEPS_KEYS[i]).round().clamp(1.0, 64.0) as u32;
            let pulses = ctx.input(id, PULSES_KEYS[i]).round().clamp(0.0, 64.0) as u32;
            LaneParams {
                steps,
                rotation: ctx.input(id, ROTATION_KEYS[i]).round() as i32,
                pattern: euclidean_pattern(pulses, steps),
            }
        });
        self.period = lanes
            .iter()
            .fold(1, |m, lane| sequencer::lcm(m, i64::from(lane.steps)));
        self.transport
            .tick(id, ctx, |lane, step| is_pulse(step, &lanes[lane]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One lane's pattern as `x` and `.`, the notation every published table of these
    /// rhythms uses, so a failure reads as the figure it drew rather than as eight booleans.
    fn figure(pulses: u32, steps: u32, rotation: i32) -> String {
        let lane = LaneParams {
            steps,
            rotation,
            pattern: euclidean_pattern(pulses, steps),
        };
        (0..i64::from(steps))
            .map(|i| if is_pulse(i, &lane) { 'x' } else { '.' })
            .collect()
    }

    /// The reason the node departs from silvia at all: at rotation zero the lane draws the
    /// figure the literature publishes under that name, starting on a pulse. Toussaint's
    /// table, five of its entries.
    #[test]
    fn rotation_zero_draws_the_published_figure() {
        for (pulses, steps, want, name) in [
            (3, 8, "x..x..x.", "tresillo"),
            (5, 8, "x.xx.xx.", "cinquillo"),
            (5, 12, "x..x.x..x.x.", "venda"),
            (5, 16, "x..x..x..x..x...", "bossa"),
            (7, 16, "x..x.x.x..x.x.x.", "samba"),
        ] {
            assert_eq!(
                figure(pulses, steps, 0),
                want,
                "E({pulses}, {steps}), the {name}"
            );
        }
    }

    /// Rotation still rotates, and one step backwards is exactly silvia's own figure —
    /// which is the whole of what a patch carried over from silvia has to correct for.
    #[test]
    fn rotation_moves_the_figure_and_minus_one_is_silvias() {
        assert_eq!(figure(3, 8, 1), ".x..x..x", "one step later");
        assert_eq!(
            figure(3, 8, -1),
            "..x..x.x",
            "silvia's `_generateEuclideanPattern` at its own rotation zero"
        );
    }

    /// Every pattern holds as many pulses as were asked for, spends none of them past its
    /// own last step, and always starts on one. Across the node's whole clamped range,
    /// because the construction pairs and re-pairs and the edges are where it would slip.
    #[test]
    fn every_pattern_in_range_is_well_formed() {
        for steps in 1..=64u32 {
            for pulses in 0..=steps {
                let pattern = euclidean_pattern(pulses, steps);
                assert_eq!(
                    pattern.count_ones(),
                    pulses,
                    "E({pulses}, {steps}) holds the wrong number of pulses"
                );
                if steps < 64 {
                    assert_eq!(
                        pattern >> steps,
                        0,
                        "E({pulses}, {steps}) set a bit past its own last step"
                    );
                }
                if pulses > 0 {
                    assert!(
                        pattern & 1 == 1,
                        "E({pulses}, {steps}) does not start on a pulse"
                    );
                }
            }
        }
    }

    /// The two edges the construction never enters: nothing to place, and nothing but
    /// pulses to place. A count past `steps` is clamped by the caller and saturates here.
    #[test]
    fn zero_pulses_is_silent_and_a_full_count_is_every_step() {
        for steps in [1, 2, 3, 4, 5, 64] {
            assert_eq!(euclidean_pattern(0, steps), 0, "zero pulses: {steps} steps");
            let full = euclidean_pattern(steps, steps);
            assert_eq!(
                full.count_ones(),
                steps,
                "every step is a pulse at {steps} of {steps}"
            );
            assert_eq!(euclidean_pattern(steps + 1, steps), full, "and past it");
        }
    }

    /// A rhythm comes back when every lane's figure does, not when the lanes' lengths meet:
    /// E(2, 32) every sixteen steps, E(6, 30) every five, a silent or full lane every step, at
    /// any rotation; the lanes then meet at the least common multiple, in bars.
    #[test]
    fn a_rhythm_comes_back_when_its_figures_do() {
        let lane = |pulses: u32, steps: u32, rotation: i32| {
            let lane = LaneParams {
                steps,
                rotation,
                pattern: euclidean_pattern(pulses, steps),
            };
            sequencer::repeat(lane.pattern, lane.steps)
        };
        assert_eq!(lane(2, 32, 0), 16);
        assert_eq!(lane(6, 30, 0), 5);
        assert_eq!(
            lane(6, 30, 7),
            5,
            "rotation moves the figure, not its length"
        );
        assert_eq!(lane(0, 5, 0), 1);
        assert_eq!(lane(5, 5, 0), 1);
        assert_eq!(lane(4, 16, 0), 4);
        assert_eq!(lane(3, 16, 0), 16);
        assert_eq!(lane(13, 64, 5), 64);
        for steps in 1..=64u32 {
            for pulses in 0..=steps {
                let want = if pulses == 0 || pulses == steps {
                    1
                } else {
                    steps / crate::nodes::chain::gcd(u64::from(steps), u64::from(pulses)) as u32
                };
                assert_eq!(lane(pulses, steps, 0), want, "E({pulses}, {steps})");
            }
        }
    }
}
