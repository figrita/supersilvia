// SPDX-License-Identifier: AGPL-3.0-or-later

//! Number arithmetic. Small nodes, each one output, defined by a macro because the only
//! thing that differs between them is one expression.
//!
//! **Every one of them is dual.** The output is declared `VaryingNumber` and carries an
//! `eval`: the same expression in Rust, which `tick` evaluates and publishes as a uniform
//! number on any instance whose inputs all resolve to uniform numbers. So one `add` is the
//! varying number arithmetic and the uniform number arithmetic both, and a chain of them fed
//! by knobs and diamonds stays on the CPU where it can drive a `UniformNumber` input. See
//! [Dual outputs](../../docs/nodes.md#dual-outputs); the two implementations of each formula
//! are held equal by a GPU test in `tests/gpu_nodes.rs`.

use crate::graph::PortType::VaryingNumber;
use crate::nodes::{Category, Control, InputDef, NodeDef, OutputDef, OutputKind};

/// A node taking `a` and `b` and returning one expression over them.
///
/// `$out` is the output row's label — silvia writes the operation across it, `A - B` rather
/// than `Output`, which is the only thing on the node that says which way round it goes —
/// and `$rest` is where both knobs rest: 0 for the ones a zero leaves alone, 1 for the two
/// whose identity is 1. `$body` is the WGSL expression and `$eval` the same one in Rust.
macro_rules! binary {
    ($name:ident, $slug:literal, $icon:literal, $label:literal, $tip:literal, $out:literal,
     $rest:literal, $body:literal, $eval:expr) => {
        pub static $name: NodeDef = NodeDef {
            slug: $slug,
            icon: $icon,
            label: $label,
            tooltip: $tip,
            inputs: &[
                InputDef {
                    key: "a",
                    label: "A",
                    ty: VaryingNumber,
                    control: Control::num($rest, -100.0, 100.0, 0.01, ""),
                },
                InputDef {
                    key: "b",
                    label: "B",
                    ty: VaryingNumber,
                    control: Control::num($rest, -100.0, 100.0, 0.01, ""),
                },
            ],
            outputs: &[OutputDef {
                key: "output",
                label: $out,
                ty: VaryingNumber,
                kind: OutputKind::Shader,
                wgsl: |node, ctx, _func| {
                    let a = ctx.input(node, "a", "uv");
                    let b = ctx.input(node, "b", "uv");
                    format!(concat!("    return ", $body, ";"), a = a, b = b)
                },
                eval: Some(|node, ctx| {
                    let f: fn(f32, f32) -> f32 = $eval;
                    f(ctx.input(node, "a"), ctx.input(node, "b"))
                }),
                ..OutputDef::EMPTY
            }],
            category: Category::Math,
            ..NodeDef::EMPTY
        };
    };
}

binary!(
    ADD,
    "add",
    "➕",
    "Add",
    "A plus B.",
    "Output",
    0.0,
    "({a}) + ({b})",
    |a, b| a + b
);
binary!(
    SUBTRACT,
    "subtract",
    "➖",
    "Subtract",
    "A minus B.",
    "A - B",
    0.0,
    "({a}) - ({b})",
    |a, b| a - b
);
// Both knobs rest at 1, as silvia's do: a Multiply or a Divide dropped into a running patch
// passes A through untouched, where resting at 0 made the node's first act be to erase.
binary!(
    MULTIPLY,
    "multiply",
    "✖",
    "Multiply",
    "A times B.",
    "A × B",
    1.0,
    "({a}) * ({b})",
    |a, b| a * b
);
// Guarded: a zero divisor is a black frame in a live set, not a crash, but it is still wrong.
// Resting at 1 over 1 is also what keeps a fresh Divide from tripping its own guard.
binary!(
    DIVIDE,
    "divide",
    "➗",
    "Divide",
    "A over B. Zero divisor yields zero.",
    "A ÷ B",
    1.0,
    "select(({a}) / ({b}), 0.0, abs({b}) < 1e-9)",
    |a, b| if b.abs() < 1e-9 { 0.0 } else { a / b }
);
// The shader's `min` and `max` and Rust's `f32::min` and `f32::max` part only on a NaN, and
// a NaN never reaches here, since `publish` drops one and a control cannot hold one.
binary!(
    MIN,
    "min",
    "⬇",
    "Min",
    "The smaller of A and B.",
    "Output",
    0.0,
    "min({a}, {b})",
    f32::min
);
binary!(
    MAX,
    "max",
    "⬆",
    "Max",
    "The larger of A and B.",
    "Output",
    0.0,
    "max({a}, {b})",
    f32::max
);

/// A node taking `input`, `frequency`, `phase` and `amplitude`, for the trig pair.
macro_rules! wave {
    ($name:ident, $slug:literal, $icon:literal, $label:literal, $fun:literal, $eval:expr) => {
        pub static $name: NodeDef = NodeDef {
            slug: $slug,
            icon: $icon,
            label: $label,
            tooltip: concat!("A ", $slug, " of its input. Connect time for an oscillator."),
            inputs: &[
                InputDef { key: "input", label: "Input", ty: VaryingNumber, control: Control::num(0.0, -10.0, 10.0, 0.01, "") },
                InputDef { key: "frequency", label: "Frequency", ty: VaryingNumber, control: Control::num(1.0, 0.01, 10.0, 0.01, "x") },
                InputDef { key: "phase", label: "Phase", ty: VaryingNumber, control: Control::num(0.0, -2.0, 2.0, 0.001, crate::nodes::TURNS) },
                InputDef { key: "amplitude", label: "Amplitude", ty: VaryingNumber, control: Control::num(1.0, 0.0, 10.0, 0.01, "") },
            ],
            outputs: &[OutputDef {
                key: "output",
                label: "Output",
                ty: VaryingNumber,
                kind: OutputKind::Shader,
                // `$fun` is the WGSL builtin, `sin` or `cos`; `PI` is the prelude's.
                wgsl: |node, ctx, _func| {
                    let input = ctx.input(node, "input", "uv");
                    let frequency = ctx.input(node, "frequency", "uv");
                    let phase = ctx.input(node, "phase", "uv");
                    let amplitude = ctx.input(node, "amplitude", "uv");
                    format!(
                        "    return {}((({input}) * ({frequency}) * 2.0 * PI) + ({phase}) * 2.0 * PI) * ({amplitude});",
                        $fun
                    )
                },
                eval: Some(|node, ctx| {
                    let f: fn(f32) -> f32 = $eval;
                    let angle = ctx.input(node, "input")
                        * ctx.input(node, "frequency")
                        * 2.0
                        * std::f32::consts::PI
                        + ctx.input(node, "phase") * std::f32::consts::TAU;
                    f(angle) * ctx.input(node, "amplitude")
                }),
                ..OutputDef::EMPTY
            }],
            category: Category::Math,
            ..NodeDef::EMPTY
        };
    };
}

wave!(SINE, "sine", "∿", "Sine", "sin", f32::sin);
wave!(COSINE, "cosine", "∼", "Cosine", "cos", f32::cos);

// ---------------------------------------------------------------------------------------
// silvia's other ten, ported from the parity review. Each is one expression, written with
// `node!` because two of them declare an output range and three have knobs that are not the
// family's -100 to 100. Every one is dual, like the six above, and `tests/gpu_nodes.rs`
// holds each `eval` to its WGSL.
// ---------------------------------------------------------------------------------------

use crate::graph::NodeId;
use crate::nodes::TickContext;
use crate::nodes::macros::node;

node! {
    /// `abs(input)`: a ramp folded back on itself, a signed distance made a distance.
    ABS,
    slug: "abs",
    icon: "🏓",
    label: "Absolute",
    category: Math,
    tooltip: "The input without its sign.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Output" eval(|node, ctx| ctx.input(node, "input").abs())
            = "    return abs({input});",
    ],
}

node! {
    /// The next whole number up. silvia's up arrow is `max`'s icon here, so the ceiling
    /// bracket stands in.
    CEIL,
    slug: "ceil",
    icon: "⌈",
    label: "Ceil",
    category: Math,
    tooltip: "Rounds the input up to the next whole number.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Output" eval(|node, ctx| ctx.input(node, "input").ceil())
            = "    return ceil({input});",
    ],
}

node! {
    /// The whole number below. The piece a stepped, tiled or posterized number is built on:
    /// multiply by the step count, floor, divide back.
    FLOOR,
    slug: "floor",
    icon: "⌊",
    label: "Floor",
    category: Math,
    tooltip: "Rounds the input down to the whole number below it.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Output" eval(|node, ctx| ctx.input(node, "input").floor())
            = "    return floor({input});",
    ],
}

node! {
    /// The angle from the X axis to the point (X, Y), in the half-turn unit the Phase knobs
    /// use: a full turn runs -1 to 1.
    ATAN2,
    slug: "atan2",
    icon: "🧭",
    label: "ATan2",
    category: Math,
    tooltip: "The angle from the X axis to the point X, Y, in half turns: a full turn runs \
              -1 to 1, the unit the Phase knobs use.",
    inputs: [
        VaryingNumber "x" "X" = Control::num(1.0, -100.0, 100.0, 0.01, ""),
        VaryingNumber "y" "Y" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Angle" in "[-1, 1]"
            eval(|node, ctx| ctx.input(node, "y").atan2(ctx.input(node, "x")) / std::f32::consts::PI)
            = "    return atan2({y}, {x}) / PI;",
    ],
}

node! {
    /// A at 0, B at 1, and past both ends when T is: the short form of the map Reframe Range
    /// does with four bounds and a clamp.
    LERP,
    slug: "lerp",
    icon: "∝",
    label: "Lerp",
    category: Math,
    tooltip: "A number between A and B: A when T is 0, B when T is 1, and past them when T \
              runs past its ends.",
    inputs: [
        VaryingNumber "a" "A" = Control::num(0.0, -10.0, 10.0, 0.01, ""),
        VaryingNumber "b" "B" = Control::num(1.0, -10.0, 10.0, 0.01, ""),
        VaryingNumber "t" "T" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
    ],
    outputs: [
        // `mix`, term for term, rather than the algebraically equal `a + (b - a) * t`.
        VaryingNumber "output" "Output" eval(|node, ctx| {
            let t = ctx.input(node, "t");
            ctx.input(node, "a") * (1.0 - t) + ctx.input(node, "b") * t
        }) = "    return mix({a}, {b}, {t});",
    ],
}

node! {
    /// The remainder of A over B, on the positive side for a negative A, which is what makes
    /// a rising number wrap rather than reflect. Rests at 1 and 1: a zero divisor has no
    /// answer, and reads 0 the way Divide's does.
    MODULO,
    slug: "modulo",
    icon: "🪙",
    label: "Modulo",
    category: Math,
    tooltip: "The remainder of A over B, which is how a rising number wraps. Zero divisor \
              yields zero.",
    inputs: [
        VaryingNumber "a" "A" = Control::num(1.0, -100.0, 100.0, 0.01, ""),
        VaryingNumber "b" "B" = Control::num(1.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        // `floor_mod` is `a - b * floor(a / b)`, written out so the two sides agree on a
        // negative A.
        VaryingNumber "output" "A mod B" eval(|node, ctx| {
            let a = ctx.input(node, "a");
            let b = ctx.input(node, "b");
            if b.abs() < 1e-5 { 0.0 } else { a - b * (a / b).floor() }
        }) = "    let b = {b};
    if (abs(b) < 1e-5) { return 0.0; }
    return floor_mod({a}, b);",
    ],
}

node! {
    /// Base to the Exponent: the one way to put a curve on a number. silvia's guard is kept:
    /// a negative base under a fractional exponent reads 0 rather than nothing at all, which
    /// only arrives down a cable since the Base knob stops at zero.
    POWER,
    slug: "power",
    icon: "🏒",
    label: "Power",
    category: Math,
    tooltip: "Base raised to Exponent. Square a 0 to 1 number to ease it in, take its root \
              to ease it out. A negative base with a fraction in the exponent reads 0.",
    inputs: [
        VaryingNumber "base" "Base" = Control::num(2.0, 0.0, 10.0, 0.01, ""),
        VaryingNumber "exponent" "Exponent" = Control::num(2.0, -10.0, 10.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Base ^ Exp" eval(power) = "    let base = {base};
    let exponent = {exponent};
    if (base < 0.0 && floor_mod(exponent, 1.0) != 0.0) { return 0.0; }
    return pow(abs(base), exponent) * sign(base);",
    ],
}

/// `power`'s body in Rust. WGSL's `sign(0.0)` is 0 where Rust's `signum` is 1, and
/// `floor_mod(exp, 1.0)` is `exp - floor(exp)`, which is `rem_euclid`.
fn power(node: NodeId, ctx: &TickContext<'_>) -> f32 {
    let base = ctx.input(node, "base");
    let exp = ctx.input(node, "exponent");
    if base < 0.0 && exp.rem_euclid(1.0) != 0.0 {
        return 0.0;
    }
    let sign = if base > 0.0 {
        1.0
    } else if base < 0.0 {
        -1.0
    } else {
        0.0
    };
    base.abs().powf(exp) * sign
}

node! {
    /// The long side of A and B: a coordinate pair made a distance. Its knobs use the
    /// family's -100 to 100 rather than silvia's -10 to 10, so the arithmetic nodes all
    /// scrub alike.
    PYTHAGOREAN,
    slug: "pythagorean",
    icon: "📐",
    label: "Pythagorean",
    category: Math,
    tooltip: "The hypotenuse of A and B: the distance a coordinate pair is from the origin.",
    inputs: [
        VaryingNumber "a" "A" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
        VaryingNumber "b" "B" = Control::num(0.0, -100.0, 100.0, 0.01, ""),
    ],
    outputs: [
        VaryingNumber "output" "Output" in "[0, ∞)" eval(|node, ctx| {
            let a = ctx.input(node, "a");
            let b = ctx.input(node, "b");
            (a * a + b * b).sqrt()
        }) = "    let a = {a};
    let b = {b};
    return sqrt(a * a + b * b);",
    ],
}

node! {
    /// 0 below Edge A, 1 above Edge B, eased between. The edges are put in order first, so
    /// Edge A above Edge B is the same ramp the other way up rather than `smoothstep`'s
    /// undefined answer, and two equal edges are a hard step.
    SMOOTHSTEP,
    slug: "smoothstep",
    icon: "🪜",
    label: "Smoothstep",
    category: Math,
    tooltip: "0 below Edge A, 1 above Edge B, and an eased curve between them: the soft \
              edge on a mask, the ease on a fade.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.5, -1.0, 1.0, 0.01, ""),
        VaryingNumber "edgeA" "Edge A" = Control::num(0.0, -1.0, 1.0, 0.01, ""),
        VaryingNumber "edgeB" "Edge B" = Control::num(1.0, -1.0, 1.0, 0.01, ""),
    ],
    wgsl_utils: [EASED_WGSL],
    outputs: [
        VaryingNumber "output" "Output" in "[0, 1]" eval(|node, ctx| {
            let a = ctx.input(node, "edgeA");
            let b = ctx.input(node, "edgeB");
            eased(ctx.input(node, "input"), a.min(b), a.max(b))
        }) = "    let a = {edgeA};
    let b = {edgeB};
    return mathEased({input}, min(a, b), max(a, b));",
    ],
}

node! {
    /// Smoothstep said as a middle and a width: 0 below the level, 1 above it, with
    /// Smoothing as the half width of the crossing, so a saved silvia patch reads the same.
    THRESHOLD,
    slug: "threshold",
    icon: "⚖",
    label: "Threshold",
    category: Math,
    tooltip: "0 below Threshold and 1 above it. Smoothing is the half width of the crossing; \
              at 0 the cut is hard.",
    inputs: [
        VaryingNumber "input" "Input" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "threshold" "Threshold" = Control::num(0.5, 0.0, 1.0, 0.01, ""),
        VaryingNumber "smooth" "Smoothing" = Control::num(0.01, 0.0, 1.0, 0.001, ""),
    ],
    wgsl_utils: [EASED_WGSL],
    outputs: [
        VaryingNumber "output" "Output" in "[0, 1]" eval(|node, ctx| {
            let t = ctx.input(node, "threshold");
            let s = ctx.input(node, "smooth").abs();
            eased(ctx.input(node, "input"), t - s, t + s)
        }) = "    let t = {threshold};
    let s = abs({smooth});
    return mathEased({input}, t - s, t + s);",
    ],
}

/// `smoothstep` with its one undefined case decided: two edges closer than a
/// hundred-thousandth are a hard step at the lower one. `e0 <= e1` is the caller's promise.
/// The WGSL twin is [`EASED_WGSL`], emitted once into any shader that reaches either node.
fn eased(x: f32, e0: f32, e1: f32) -> f32 {
    if e1 - e0 < 1e-5 {
        return if x < e0 { 0.0 } else { 1.0 };
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// [`eased`], in WGSL. Prefixed with its file, since a WGSL helper's name is unique across the
/// whole registry.
const EASED_WGSL: &str = "fn mathEased(x: f32, e0: f32, e1: f32) -> f32 {
    if (e1 - e0 < 1e-5) { return select(1.0, 0.0, x < e0); }
    return smoothstep(e0, e1, x);
}";
