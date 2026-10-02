// SPDX-License-Identifier: AGPL-3.0-or-later

//! A tap: measure a color where it flows, without changing it.
//!
//! The picture passes through untouched. Beside the pass-through, the node declares a
//! **measurement**: a function of a point, which its workspace's pass calls at every cell
//! of an `N`x`N` grid over the unit square, and which emits atomic writes into the pass's tap
//! buffer — a count, a sum of the measured quantity, the extremes, and coordinate sums
//! weighted by it. What it measures is therefore the node's own input over the frame, one
//! evaluation per point per frame, whoever calls the pass-through and at whatever
//! coordinates, and whether anything calls it at all. The CPU half reads the slot back a frame later and publishes uniform
//! numbers.
//!
//! `grid` is the `N`, and `jitter` moves each point inside its own cell by a hash of the
//! cell and the clock, so a pattern finer than the grid stops biasing the mean in one
//! direction. See docs/decisions.md#a-tap-measures-its-input-over-the-unit-square.
//!
//! Which quantity it measures is a picker — the eleven `Convert` reductions, out of the one
//! table in `decompose` — or, while something is plugged into `number`, that field instead. The
//! sidechain wins because it is the more specific statement: a cable is there or it is not.
//!
//! Fixed point, because a fragment shader's atomic add is integer. A sample is stored at
//! 1/65536 of a unit and biased by 32768, so the unsigned words carry sign: the measured
//! range is -32768 to 32767 and a sample outside it is clamped there. Each sum is 64 bits
//! across two words — `atomicAdd` returns the word it replaced, so a wrap is visible and
//! costs one further atomic on the high word — which is why no frame size overflows a sum
//! and no fraction is thrown away to make room.
//!
//! Extremes are exact and signed, ordered by a monotonic map of the float's bits: a negative
//! float's bits are inverted, a non-negative float's sign bit is set, and `atomicMax` and
//! `atomicMin` then order the whole line. The centroid weights by the positive part,
//! `max(l, 0)`, so `x` and `y` say where the quantity is positive.
//!
//! Beside all that, the **mean color**: three one-word sums of the input's own channels,
//! which is a different question from the picked quantity — *what color is this picture on
//! average*, rather than *how much of this quantity is in it*. Three words rather than six,
//! because a channel needs neither the range nor the precision a measured quantity does: it
//! is clamped to [`COLOR_MAX`] and kept to 1/[`COLOR_SCALE`], and the two constants are
//! chosen so the largest grid cannot overflow one word. Alpha is not summed — the mean color
//! is opaque, so a mostly-transparent picture still shows the hue it is being asked about
//! rather than reporting itself invisible.
//!
//! The slot, in words: count, the sum of the measured quantity (low, high), the maximum's
//! key, the minimum's key, the sum of `x` times the weight (low, high), the same for `y`,
//! the sum of the weight (low, high), and the three color sums.

use crate::compile::{CompileContext, TAP_WORDS, TapKind};
use crate::graph::NodeId;
use crate::graph::PortType::{UniformColor, UniformNumber, VaryingColor, VaryingNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext, decompose,
};

/// What a tap measures when nothing is plugged into `number`: the reading it published
/// before there was anything to choose.
pub const DEFAULT_MEASURE: &str = "luminosity";

/// The grids a tap offers, as the option's values and its labels.
pub const GRID_CHOICES: &[(&str, &str)] =
    &[("32", "32"), ("64", "64"), ("128", "128"), ("256", "256")];

/// Points per axis where a node measures on a grid and offers no choice about it: 16,384
/// evaluations of its input a frame.
pub const DEFAULT_GRID: u32 = 128;

/// The grid an option value names, or [`DEFAULT_GRID`] where it names none.
fn grid_of(value: &str) -> u32 {
    value.parse().unwrap_or(DEFAULT_GRID)
}

/// Fixed-point units per unit of the measured quantity: a sample is kept to 1/65536.
pub const SCALE: f32 = 65536.0;
/// Added to a sample before it is summed, so an unsigned sum carries sign. The measured
/// range is `-BIAS` to `BIAS - 1`.
pub const BIAS: f32 = 32768.0;
/// `BIAS * SCALE`: what a sample of zero stores, and what decoding subtracts per sample.
pub const BIAS_UNITS: u32 = 1 << 31;

/// Fixed-point units per unit of a color channel, for the mean color's three sums.
///
/// Finer than eight-bit color by a factor of thirty-two, which is all a mean of a picture
/// needs. Paired with [`COLOR_MAX`] so one word cannot overflow: the largest grid is 256×256,
/// and 65536 samples of `COLOR_MAX * COLOR_SCALE` is exactly 2^31.
pub const COLOR_SCALE: f32 = 8192.0;
/// The top of the range a channel is summed over. Above HDR white by two stops, and a
/// channel outside `[0, COLOR_MAX)` is clamped there — a mean *color* is a color, so a
/// negative channel contributes nothing rather than pulling the mean below black.
pub const COLOR_MAX: f32 = 4.0;

/// Word offsets inside a `Stats` slot, from its base.
const COUNT: usize = 0;
const SUM: usize = 1;
const MAX: usize = 3;
const MIN: usize = 4;
const XSUM: usize = 5;
const YSUM: usize = 7;
const WSUM: usize = 9;
const RSUM: usize = 11;
const GSUM: usize = 12;
const BSUM: usize = 13;

/// One sample of `expr` as biased fixed point. `expr` must already sit inside the measured
/// range, which is what `clamped` writes.
fn biased(expr: &str) -> String {
    format!("{BIAS_UNITS}u + u32(i32(({expr}) * {SCALE:.1}))")
}

/// `expr` held inside the measured range, `-BIAS` to `BIAS - 1`.
fn clamped(expr: &str) -> String {
    format!("clamp({expr}, {:.1}, {:.1})", -BIAS, BIAS - 1.0)
}

/// Add `value` to the 64-bit sum whose low word is `lo`, carrying into the word after it.
fn add64(lo: usize, value: &str) -> String {
    let hi = lo + 1;
    format!(
        "    ta = {value};
    to = atomicAdd(&tap[{lo}u], ta);
    if (to + ta < to) {{ atomicAdd(&tap[{hi}u], 1u); }}"
    )
}

/// The atomic writes that measure `luma` into slot `base`, at the point named `at`. Shared
/// with any node that measures as part of doing something else.
///
/// `luma` is an `f32` expression in the caller's scope; the caller declares whatever it reads.
pub fn stats_wgsl(base: usize, luma: &str, at: &str) -> String {
    let (count, max, min) = (base + COUNT, base + MAX, base + MIN);
    format!(
        "    let l = {};
    let w = max(l, 0.0);
    var tk = bitcast<u32>(l);
    tk = select(tk | 0x80000000u, ~tk, (tk & 0x80000000u) != 0u);
    var ta: u32;
    var to: u32;
    atomicAdd(&tap[{count}u], 1u);
    atomicMax(&tap[{max}u], tk);
    atomicMin(&tap[{min}u], tk);
{}
{}
{}
{}",
        clamped(luma),
        add64(base + SUM, &biased("l")),
        add64(base + XSUM, &biased(&clamped(&format!("{at}.x * w")))),
        add64(base + YSUM, &biased(&clamped(&format!("{at}.y * w")))),
        add64(base + WSUM, &format!("u32(w * {SCALE:.1})")),
    )
}

/// The atomic writes that sum `color`'s three channels into slot `base`.
///
/// Beside [`stats_wgsl`] rather than inside it, because a node that measures one quantity
/// wants the quantity and nothing else: `autoexposure` would otherwise pay three atomics a
/// grid point for a color it never reads.
///
/// `color` is a `vec4f` expression in the caller's scope, as `luma` is for `stats_wgsl`.
pub fn color_wgsl(base: usize, color: &str) -> String {
    let channel = |c: char, word: usize| {
        format!(
            "    atomicAdd(&tap[{word}u], u32(clamp({color}.{c}, 0.0, {:.4}) * {COLOR_SCALE:.1}));",
            COLOR_MAX - 1.0 / COLOR_SCALE
        )
    };
    format!(
        "{}\n{}\n{}",
        channel('r', base + RSUM),
        channel('g', base + GSUM),
        channel('b', base + BSUM),
    )
}

/// The tap's measurement: the quantity its picker names, or the field in its sidechain,
/// summed over the grid, with the mean color beside it.
fn measure_wgsl(node: NodeId, ctx: &mut CompileContext) {
    // A connected `number` is the more specific statement, so it wins over the picker: the
    // tap measures that field and the choice is not read.
    let sidechain = ctx
        .connected(node, "number")
        .then(|| ctx.input(node, "number", "p"));
    let picked = decompose::find(ctx.option(node, "measure"))
        .unwrap_or_else(|| decompose::find(DEFAULT_MEASURE).expect("the default is in the table"));
    let input = ctx.input(node, "input", "p");
    let base = ctx.tap_slot(node, TapKind::Stats) * TAP_WORDS;
    // The helpers are what a conversion expression is written against, so they are emitted
    // only where one is spliced.
    let (helpers, luma) = match &sidechain {
        Some(field) => (String::new(), field.as_str()),
        None => (format!("{}\n", decompose::HELPERS_WGSL), picked.wgsl),
    };
    let body = format!(
        "    let color = {input};\n{helpers}{}\n{}",
        stats_wgsl(base, luma, "p"),
        color_wgsl(base, "color"),
    );
    let grid = grid_of(ctx.option(node, "grid"));
    let jitter = ctx.option_uniform(node, "jitter");
    ctx.measure_grid(node, grid, Some(&jitter), &body);
}

pub static DEF: NodeDef = NodeDef {
    slug: "tap",
    category: Category::Tap,
    icon: "🩺",
    label: "Tap",
    tooltip: "Passes its input through and measures it: its mean color, and the mean, peak, \
              floor and centroid of the quantity Measure names, one frame later. A varying \
              number plugged into Number is measured instead of that quantity.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            control: Control::None,
        },
        InputDef {
            key: "number",
            label: "Number",
            ty: VaryingNumber,
            control: Control::None,
        },
    ],
    options: &[
        OptionDef {
            key: "measure",
            label: "Measure",
            default: DEFAULT_MEASURE,
            choices: &decompose::CHOICES,
            overridden_by: Some("number"),
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "grid",
            label: "Grid",
            default: "128",
            choices: GRID_CHOICES,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "jitter",
            label: "Jitter",
            default: crate::nodes::OFF,
            // Off first, so the uniform reads 1 exactly when the jitter is on.
            choices: &[(crate::nodes::OFF, "Off"), (crate::nodes::ON, "On")],
            kind: OptionKind::Uniform,
            ..OptionDef::EMPTY
        },
    ],
    outputs: &[
        OutputDef {
            key: "output",
            // Not "Output": the row below it is a color too, and this one is the *input*,
            // untouched. A tap is the only shape where the picture leaving is the picture
            // arriving, so the label says that rather than which way the port points.
            label: "Pass-Through",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            wgsl: |node, ctx, _func| {
                let input = ctx.input(node, "input", "uv");
                format!("    return {input};")
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "color",
            // "Mean Color", not "Color", so it reads as one of the summaries beside it: the
            // mean of the channels, where `mean` is the mean of the picked quantity.
            label: "Mean Color",
            ty: UniformColor,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "mean",
            label: "Mean",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "max",
            label: "Max",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "min",
            label: "Min",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "x",
            label: "X",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "y",
            label: "Y",
            ty: UniformNumber,
            kind: OutputKind::Uniform,
            delayed: true,
            ..OutputDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || {
            Box::new(Tap {
                last: decode(&crate::compile::TAP_TEMPLATE),
                status: DORMANT.to_string(),
            })
        },
        integrates: false,
        live: false,
    }),
    measure_wgsl: Some(measure_wgsl),
    ..NodeDef::EMPTY
};

/// What a node whose measurement no pass runs says about itself.
pub const DORMANT: &str = "not measuring";

/// What a node its workspace's pass measures says about itself.
pub const MEASURING: &str = "measuring";

/// What a slot decodes to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub count: u32,
    pub mean: f32,
    pub max: f32,
    pub min: f32,
    pub x: f32,
    pub y: f32,
    /// The mean of the input's own channels, opaque. Not the picked quantity: a tap says
    /// both what color its input is and how much of the chosen quantity is in it.
    pub color: [f32; 4],
}

impl Stats {
    /// A tap that measured nothing.
    const NONE: Self = Self {
        count: 0,
        mean: 0.0,
        max: 0.0,
        min: 0.0,
        x: 0.0,
        y: 0.0,
        color: [0.0, 0.0, 0.0, 1.0],
    };
}

/// The 64-bit sum whose low word is `w[lo]`.
fn sum64(w: &[u32; TAP_WORDS], lo: usize) -> u64 {
    u64::from(w[lo]) | (u64::from(w[lo + 1]) << 32)
}

/// The same sum with the per-sample bias taken back out.
fn signed64(w: &[u32; TAP_WORDS], lo: usize, count: u32) -> i64 {
    sum64(w, lo) as i64 - i64::from(BIAS_UNITS) * i64::from(count)
}

/// The float a monotonic extreme key came from. The two seeds — an all-zero maximum and an
/// all-ones minimum, which no finite sample produces — read as zero.
fn from_key(k: u32) -> f32 {
    if k == 0 || k == u32::MAX {
        return 0.0;
    }
    f32::from_bits(if k & 0x8000_0000 != 0 {
        k & 0x7fff_ffff
    } else {
        !k
    })
}

/// Decode one slot: the count, the mean, the extremes and the centroid. A slot nothing wrote
/// — no fragment ran — has a count of zero and reads as zeros, which is also what an
/// unwritten extreme reads as.
pub fn decode(w: &[u32; TAP_WORDS]) -> Stats {
    let count = w[COUNT];
    if count == 0 {
        return Stats::NONE;
    }
    let weight = sum64(w, WSUM);
    let centroid = |lo: usize| {
        if weight > 0 {
            (signed64(w, lo, count) as f64 / weight as f64) as f32
        } else {
            0.0
        }
    };
    let channel =
        |word: usize| (f64::from(w[word]) / f64::from(COLOR_SCALE) / f64::from(count)) as f32;
    Stats {
        count,
        mean: (signed64(w, SUM, count) as f64 / f64::from(SCALE) / f64::from(count)) as f32,
        max: from_key(w[MAX]),
        min: from_key(w[MIN]),
        x: centroid(XSUM),
        y: centroid(YSUM),
        color: [channel(RSUM), channel(GSUM), channel(BSUM), 1.0],
    }
}

struct Tap {
    last: Stats,
    status: String,
}

/// The five numbers a tap publishes beside its mean color.
const PORTS: [&str; 5] = ["mean", "max", "min", "x", "y"];

impl CpuNode for Tap {
    fn reset(&mut self) {
        self.last = decode(&crate::compile::TAP_TEMPLATE);
        self.status = DORMANT.to_string();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        self.status = if ctx.readback(id).is_some() {
            MEASURING
        } else {
            DORMANT
        }
        .to_string();
        // A reading of nothing is not a reading of zero: with no slot to decode, every port
        // is withdrawn and its row draws nothing.
        let stats = ctx.readback(id).map(decode).filter(|s| s.count > 0);
        self.last = stats.unwrap_or(Stats::NONE);
        let Some(stats) = stats else {
            for port in PORTS {
                ctx.withdraw(id, port);
            }
            ctx.withdraw_color(id, "color");
            return;
        };
        ctx.publish_color(id, "color", stats.color);
        for (port, value) in PORTS
            .iter()
            .zip([stats.mean, stats.max, stats.min, stats.x, stats.y])
        {
            ctx.publish(id, port, value);
        }
    }

    fn status(&self) -> Option<String> {
        Some(self.status.clone())
    }

    fn debug(&self) -> Option<String> {
        let s = self.last;
        Some(format!(
            "tap {} samples | mean {:.3} max {:.3} min {:.3} | centroid ({:.2}, {:.2}) | \
             color {:.2} {:.2} {:.2}",
            s.count, s.mean, s.max, s.min, s.x, s.y, s.color[0], s.color[1], s.color[2]
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One sample of `l` as the shader would store it.
    fn sample(l: f32) -> u64 {
        (i64::from(BIAS_UNITS) + (l * SCALE) as i64) as u64
    }

    /// A slot built the way runs of `(count, l, x, y)` — identical measurements each —
    /// would leave it.
    fn slot_of(runs: &[(u32, f32, f32, f32)]) -> [u32; TAP_WORDS] {
        let mut w = crate::compile::TAP_TEMPLATE;
        let key = |v: f32| {
            let k = v.to_bits();
            if k & 0x8000_0000 != 0 {
                !k
            } else {
                k | 0x8000_0000
            }
        };
        for (count, l, x, y) in runs.iter().copied() {
            w[COUNT] = w[COUNT].wrapping_add(count);
            w[MAX] = w[MAX].max(key(l));
            w[MIN] = w[MIN].min(key(l));
            let weight = l.max(0.0);
            for (lo, total) in [
                (SUM, sample(l).wrapping_mul(u64::from(count))),
                (XSUM, sample(x * weight).wrapping_mul(u64::from(count))),
                (YSUM, sample(y * weight).wrapping_mul(u64::from(count))),
                (WSUM, ((weight * SCALE) as u64) * u64::from(count)),
            ] {
                let sum = sum64(&w, lo).wrapping_add(total);
                w[lo] = sum as u32;
                w[lo + 1] = (sum >> 32) as u32;
            }
        }
        w
    }

    /// One run of identical measurements.
    fn slot(count: u32, l: f32, x: f32, y: f32) -> [u32; TAP_WORDS] {
        slot_of(&[(count, l, x, y)])
    }

    /// The three color sums `count` samples of `rgb` would leave, added into `w`.
    fn with_color(mut w: [u32; TAP_WORDS], count: u32, rgb: [f32; 3]) -> [u32; TAP_WORDS] {
        for (word, c) in [RSUM, GSUM, BSUM].into_iter().zip(rgb) {
            let per = (c.clamp(0.0, COLOR_MAX - 1.0 / COLOR_SCALE) * COLOR_SCALE) as u32;
            w[word] = w[word].wrapping_add(per * count);
        }
        w
    }

    #[test]
    fn an_unwritten_slot_decodes_to_zeros() {
        let s = decode(&crate::compile::TAP_TEMPLATE);
        assert_eq!(s.count, 0);
        assert_eq!((s.mean, s.max, s.min, s.x, s.y), (0.0, 0.0, 0.0, 0.0, 0.0));
        // Black, opaque — a color has an alpha and nothing measured means nothing was there.
        assert_eq!(s.color, [0.0, 0.0, 0.0, 1.0]);
    }

    /// The mean color is the mean of the channels, and it is a different question from the
    /// picked quantity: the same slot carries a luminosity of one and a mean color that is
    /// pure blue.
    #[test]
    fn the_mean_color_is_the_mean_of_the_channels() {
        let w = with_color(slot(8, 1.0, 0.0, 0.0), 8, [0.0, 0.0, 1.0]);
        let s = decode(&w);
        assert_eq!(s.mean, 1.0, "the picked quantity is untouched");
        let [r, g, b, a] = s.color;
        assert!(r.abs() < 1e-3 && g.abs() < 1e-3, "{:?}", s.color);
        assert!((b - 1.0).abs() < 1e-3, "{:?}", s.color);
        assert_eq!(a, 1.0, "the mean color is opaque");
    }

    /// Two runs of different colors average, rather than either winning.
    #[test]
    fn two_colors_average() {
        let w = with_color(with_color(slot(8, 0.0, 0.0, 0.0), 4, [1.0; 3]), 4, [0.0; 3]);
        let s = decode(&w);
        for c in &s.color[..3] {
            assert!((c - 0.5).abs() < 1e-3, "{:?}", s.color);
        }
    }

    /// The largest grid at the top of the range is exactly what one word holds: 65536 samples
    /// of `COLOR_MAX * COLOR_SCALE` is 2^31, which is why three words are enough where the
    /// measured quantity needs six.
    #[test]
    fn the_biggest_grid_at_the_brightest_channel_does_not_overflow_a_word() {
        let samples = u64::from(GRID_CHOICES.last().unwrap().0.parse::<u32>().unwrap()).pow(2);
        let per = (COLOR_MAX * COLOR_SCALE) as u64;
        assert!(
            u32::try_from(samples * per).is_ok(),
            "{samples} x {per} overflows a word"
        );
    }

    #[test]
    fn the_bias_cancels_at_a_count_of_zero_and_nothing_is_divided() {
        // Every word set as if fragments had run, but a count of zero: the bias has nothing
        // to cancel against, so the slot reads as nothing rather than as a huge mean.
        let mut w = slot(4, 0.5, 0.0, 0.0);
        w[COUNT] = 0;
        assert_eq!(decode(&w), Stats::NONE);
    }

    #[test]
    fn a_gray_frame_decodes_to_its_level_and_a_centerd_centroid() {
        // 200 samples of luminance 0.5, half at x = -1 and half at x = +1.
        let n = 200u32;
        let s = decode(&slot_of(&[(n / 2, 0.5, -1.0, 0.0), (n / 2, 0.5, 1.0, 0.0)]));
        assert_eq!(s.count, n);
        assert!((s.mean - 0.5).abs() < 1e-6, "mean {}", s.mean);
        assert_eq!(s.max, 0.5);
        assert_eq!(s.min, 0.5);
        assert_eq!((s.x, s.y), (0.0, 0.0));
    }

    #[test]
    fn a_sum_that_wraps_the_low_word_carries_into_the_high_one() {
        // The bias alone is half the low word, so two samples already wrap it. Sixty
        // thousand of them wrap it thirty thousand times and the mean is still the level.
        let n = 60_000u32;
        let w = slot(n, 0.75, 0.0, 0.0);
        assert_ne!(w[SUM + 1], 0, "the high word carries");
        let s = decode(&w);
        assert_eq!(s.count, n);
        assert!((s.mean - 0.75).abs() < 1e-6, "mean {}", s.mean);
    }

    #[test]
    fn a_negative_field_decodes_to_a_negative_mean_and_negative_extremes() {
        let s = decode(&slot(4096, -0.25, 0.0, 0.0));
        assert!((s.mean + 0.25).abs() < 1e-6, "mean {}", s.mean);
        assert_eq!(s.max, -0.25);
        assert_eq!(s.min, -0.25);
        // Nothing was positive, so the centroid has no weight and stays at the origin.
        assert_eq!((s.x, s.y), (0.0, 0.0));
    }

    #[test]
    fn the_extreme_keys_order_across_zero() {
        let s = decode(&slot_of(&[(1, -2.0, 0.0, 0.0), (1, 0.5, 0.0, 0.0)]));
        assert_eq!(s.max, 0.5);
        assert_eq!(s.min, -2.0);
    }

    #[test]
    fn a_negative_coordinate_sum_survives_the_bias_round_trip() {
        let s = decode(&slot(10, 1.0, -0.75, 0.25));
        assert!((s.x + 0.75).abs() < 1e-4, "x {}", s.x);
        assert!((s.y - 0.25).abs() < 1e-4, "y {}", s.y);
    }

    /// Two levels over one grid: the mean is weighted by how many points read each, and the
    /// extremes are the two levels.
    #[test]
    fn two_levels_over_one_grid_decode_to_the_weighted_mean() {
        let s = decode(&slot_of(&[(10, 1.0, 0.0, 0.0), (20, 0.1, 0.0, 0.0)]));
        assert_eq!(s.count, 30);
        assert_eq!(s.max, 1.0);
        assert_eq!(s.min, 0.1);
        let mean = (10.0 * 1.0 + 20.0 * 0.1) / 30.0;
        assert!((s.mean - mean).abs() < 1e-4, "mean {}", s.mean);
    }
}
