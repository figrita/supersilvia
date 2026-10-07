// SPDX-License-Identifier: AGPL-3.0-or-later

//! Escape-time and iterated fractals: Mandelbrot, Julia, Sierpiński, Lyapunov.
//!
//! Ported from silvia's `mandelbrot.js`, `juliaset.js`, `sierpinski.js` and `lyapunov.js`.
//!
//! Three departures from the source, each following from a rule written down here.
//!
//! **`smooth` is a `VaryingNumber`, not a picture.** silvia's `smooth` output is the
//! smooth escape count already wrapped in a four-stop palette nothing can reach, and its
//! `color` output bands on the raw integer count. Here one loop with a large escape radius
//! produces the smooth count once, `smooth` publishes it, and `color` ramps the node's own
//! foreground and background along it — so the field is patchable and the fixed palette is
//! a `mix` away rather than baked in.
//!
//! **`mask` is the set, not the escape fraction.** silvia's mask returns `i / maxIter`, which
//! is the escape count under a name that promises coverage — and it samples at `uv - 0.5`
//! while every other output of the same node samples at `uv`. `mask` here is 1 inside the
//! set and 0 outside, which is what the name means everywhere else in the library.
//!
//! **Time and Offset, in drifts of the map.** silvia's orbit-trap mapping reads `u_time *
//! timeSpeed`; here the escape-time pair move at half a drift a second — silvia's Time Speed
//! of 0.5 — at Speed 1, or in Loop mode on ambient time or a gear cabled into **Time**, and
//! **Offset** is added to it, so a Speed or a Ratio Gear turns the drift and nothing jumps.
//!
//! `lyapunov` runs a fixed [`ITERATIONS`] steps of its map rather than silvia's Iterations
//! control: ten are enough for the picture, and a count that is a field is a loop no compiler
//! can bound, which made it the one pass the editor waited behind. It is the exception to
//! [a number staying varying](../../docs/decisions.md#a-number-stays-varying).
//!
//! `sequence` is typed free text, like silvia's own field — `OptionDef::placeholder`,
//! documented in `docs/nodes.md`'s option kinds and `docs/decisions.md` — and silvia's Random
//! Seq writes it: a [`Region::Buttons`] row of one, whose press is a `SetSettings` and so a
//! rebuild, one undo step and a saved sequence. It is a button and not silvia's action input,
//! because a cable that rewrote a `Code` option would rebuild the shader every time it fired.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Buttons, Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, Region,
    Settings, Timing,
};

// --------------------------------------------------------------------- escape-time pair

/// The iteration, shared by both escape-time fractals.
///
/// Reads `z`, `c` and `maxIter`; declares `i`, `inside` and `smoothT`. The escape radius is
/// large because the smooth count needs it: at a radius of 2 the fractional part is dominated
/// by how far past the boundary the last step landed.
///
/// `z` is a `var` its caller declares, and `i` outlives the loop, so it is declared before it
/// rather than in the loop's own header.
const ESCAPE_WGSL: &str = "    var i = 0;
    for (; i < maxIter; i++) {
        z = vec2f(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        if (dot(z, z) > 10000.0) { break; }
    }
    let inside = select(0.0, 1.0, i >= maxIter);
    var smoothT = 0.0;
    if (i < maxIter) {
        let nu = log(log(dot(z, z)) * 0.5 / log(2.0)) / log(2.0);
        smoothT = clamp((f32(i) + 1.0 - nu) / f32(maxIter), 0.0, 1.0);
    }
";

/// The color ramp both escape-time fractals draw: background inside the set, foreground at
/// the outside of the halo.
const ESCAPE_COLOR: &str = "    return mix({background}, {foreground}, smoothT);";

const ESCAPE_MASK: &str = "    return inside;";

const ESCAPE_SMOOTH: &str = "    return smoothT;";

/// Orbit-trap texture mapping: an escaped point reads the input at its final angle and
/// radius, a captured one at its own coordinate pushed by where the orbit ended up.
const ESCAPE_MAP_WGSL: &str = "    var finalUV: vec2f;
    if (i < maxIter) {
        let tu = floor_mod(atan2(z.y, z.x) / (2.0 * PI) + time_periodic({clock}, {phaseOffset}), 1.0);
        let tv = log2(log(dot(z, z)) / log(10000.0));
        finalUV = vec2f(tu, mix(tv, 1.0 - tv, floor_mod(f32(i), 2.0)));
    } else {
        finalUV = uv + z * ({strength});
    }
    return {input};";

node! {
    /// z ← z² + c from zero, over the complex plane.
    MANDELBROT,
    slug: "mandelbrot",
    icon: "🗺",
    label: "Mandelbrot",
    category: Generate,
    tooltip: "The Mandelbrot set. Its mask is the set itself, its smooth output the escape \
              count as a field, and its map reads the input picture along each orbit.",
    // "Map Strength" does not fit the default 200.
    width: 216.0,
    timing: Timing::periodic(0.5),
    inputs: [
        VaryingColor "input" "Map Texture" at "finalUV" = Control::None,
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "centerX" "Center X" = Control::num(-0.5, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "zoom" "Zoom" = Control::num_log(1.0, 0.01, 100_000.0, 0.01, "x"),
        VaryingNumber "iterations" "Iterations" = Control::num_log(50.0, 10.0, 500.0, 1.0, ""),
        VaryingNumber "strength" "Map Strength" = Control::num(1.3, -5.0, 5.0, 0.01, ""),
    ],
    wgsl_common: {
        [
            "    let c = vec2f({centerX}, {centerY}) + uv / max(abs({zoom}), 1e-6);
    var z = vec2f(0.0);
    let maxIter = i32(clamp({iterations}, 1.0, 500.0));
",
            ESCAPE_WGSL,
        ]
        .concat()
    },
    outputs: [
        VaryingColor "color" "Color" = ESCAPE_COLOR,
        VaryingNumber "smooth" "Smooth" = ESCAPE_SMOOTH,
        VaryingColor "map" "Map" = ESCAPE_MAP_WGSL,
        VaryingNumber "mask" "Mask" = ESCAPE_MASK,
    ],
}

node! {
    /// z ← z² + c from the pixel, with c a constant.
    JULIASET,
    slug: "juliaset",
    icon: "🌀",
    label: "Julia Set",
    category: Generate,
    tooltip: "The Julia set of one complex constant. Move c and the whole picture changes \
              shape; its mask, smooth and map are the Mandelbrot's.",
    // "Map Strength" does not fit the default 200.
    width: 216.0,
    timing: Timing::periodic(0.5),
    inputs: [
        VaryingColor "input" "Map Texture" at "finalUV" = Control::None,
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "centerX" "Center X" = Control::num(0.0, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "centerY" "Center Y" = Control::num(0.0, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "zoom" "Zoom" = Control::num_log(1.0, 0.01, 100_000.0, 0.01, "x"),
        VaryingNumber "cReal" "C Real" = Control::num(-0.7, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "cImag" "C Imaginary" = Control::num(0.27015, -2.0, 2.0, 0.001, "⬓"),
        VaryingNumber "iterations" "Iterations" = Control::num_log(50.0, 10.0, 500.0, 1.0, ""),
        VaryingNumber "strength" "Map Strength" = Control::num(1.3, -5.0, 5.0, 0.01, ""),
    ],
    wgsl_common: {
        [
            "    var z = vec2f({centerX}, {centerY}) + uv / max(abs({zoom}), 1e-6);
    let c = vec2f({cReal}, {cImag});
    let maxIter = i32(clamp({iterations}, 1.0, 500.0));
",
            ESCAPE_WGSL,
        ]
        .concat()
    },
    outputs: [
        VaryingColor "color" "Color" = ESCAPE_COLOR,
        VaryingNumber "smooth" "Smooth" = ESCAPE_SMOOTH,
        VaryingColor "map" "Map" = ESCAPE_MAP_WGSL,
        VaryingNumber "mask" "Mask" = ESCAPE_MASK,
    ],
}

// -------------------------------------------------------------------------- Sierpiński

node! {
    /// The Sierpiński gasket, as a bitwise test on a sheared triangular lattice.
    SIERPINSKI,
    slug: "sierpinski",
    icon: "🔺",
    label: "Sierpinski Triangle",
    category: Generate,
    tooltip: "The Sierpinski gasket. Zoom sets the lattice size and detail the depth of the \
              recursion; a cell is filled where its row and column share no bit.",
    inputs: [
        VaryingColor "colorIn" "Color In" = Control::color("#ffffffff"),
        VaryingColor "colorOut" "Color Out" = Control::color("#000000ff"),
        VaryingNumber "zoom" "Zoom" = Control::num_log(100.0, 1.0, 1000.0, 1.0, "x"),
        VaryingNumber "detail" "Detail" = Control::num(4.0, 1.0, 10.0, 1.0, ""),
    ],
    // The square lattice is sheared into an equilateral one before the bitwise test, which
    // is what makes the holes triangles rather than right angles.
    outputs: [
        VaryingColor "output" "Output" = "    const SQRT3 = 1.7320508;
    var p = uv * ({zoom});
    p.y *= 2.0 / SQRT3;
    p.x -= p.y * 0.5;
    let size = pow(2.0, clamp({detail}, 1.0, 10.0));
    let cell = vec2i(floor_mod2(p, vec2f(size)));
    let inSet = select(0.0, 1.0, (cell.x & cell.y) == 0);
    return mix({colorOut}, {colorIn}, inSet);",
    ],
}

// --------------------------------------------------------------------------- Lyapunov

/// How many steps of the map one exponent averages over: a count of the node's own, not a
/// control. Ten is enough for the picture, and a constant is what lets
/// the loop run a fixed number of times with no exit test of its own. See
/// docs/loop-bounding.md.
pub const ITERATIONS: u32 = 10;

/// One Lyapunov exponent, inlined.
///
/// A block rather than a function, because a generator returns one function body and nothing
/// else. `AAA` and `BBB` are the two rates, `OUT` the `f32` it writes, `SEQLEN` the length of
/// the `seq` array in scope, `STEPS` the count. Reads `amp`.
///
/// `OUT` is a `var` in scope, and `seq` a `var` array indexed by the loop's count.
const EXPONENT_WGSL: &str = "    {
        var x = 0.0;
        var sum = 0.0;
        for (var n = 0; n < STEPS; n++) {
            let r = select(BBB, AAA, seq[n % SEQLEN] == 0);
            let s = sin(x + r);
            x = amp * s * s;
            sum += log(max(abs(amp * sin(2.0 * (x + r))), 1e-12));
        }
        OUT = sum / f32(STEPS);
    }
";

/// The exponent block, with its two rates and its destination filled in.
fn exponent_wgsl(a: &str, b: &str, out: &str, len: usize) -> String {
    fill_exponent(EXPONENT_WGSL, a, b, out, len)
}

/// Either language's exponent block with its holes filled.
fn fill_exponent(block: &str, a: &str, b: &str, out: &str, len: usize) -> String {
    block
        .replace("AAA", a)
        .replace("BBB", b)
        .replace("OUT", out)
        .replace("SEQLEN", &len.to_string())
        .replace("STEPS", &ITERATIONS.to_string())
}

/// The one reading of a sequence string: `emit` is handed 0 for each `A` and 1 for each `B`,
/// and the whole length comes back where it parses.
///
/// `A6B6` is `AAAAAABBBBBB`; a run count follows its letter, optionally after `^`. `None` for
/// anything that does not parse — empty text, no `A` or `B` in it, a run over 128 long. One
/// scan of the text and no allocation of its own, because [`sequence_is_valid`] asks the same
/// question every frame the option row is drawn and wants only the answer.
fn scan_sequence(text: &str, mut emit: impl FnMut(i32)) -> Option<usize> {
    let mut bytes = text
        .bytes()
        .map(|b| b.to_ascii_uppercase())
        .filter(|b| matches!(b, b'A' | b'B' | b'0'..=b'9' | b'^'))
        .peekable();
    let mut total = 0;
    while let Some(byte) = bytes.next() {
        let value = match byte {
            b'A' => 0,
            b'B' => 1,
            _ => return None,
        };
        if bytes.peek() == Some(&b'^') {
            bytes.next();
        }
        // A run count too long to hold reads as one, which is what parsing its digits and
        // failing did.
        let mut digits = false;
        let mut count: Option<usize> = Some(0);
        while let Some(digit) = bytes.peek().copied().filter(u8::is_ascii_digit) {
            bytes.next();
            digits = true;
            count = count
                .and_then(|n| n.checked_mul(10))
                .and_then(|n| n.checked_add((digit - b'0') as usize));
        }
        let count = if digits {
            count.map_or(1, |n| n.clamp(1, 64))
        } else {
            1
        };
        total += count;
        if total > 128 {
            return None;
        }
        for _ in 0..count {
            emit(value);
        }
    }
    (total > 0).then_some(total)
}

/// A sequence string as the 0-for-A, 1-for-B array the shader indexes. Anything that does not
/// parse falls back to `A6B6`, so a shader is always emitted.
fn parse_sequence(text: &str) -> Vec<i32> {
    let mut out = Vec::new();
    match scan_sequence(text, |value| out.push(value)) {
        Some(_) => out,
        None => default_sequence(),
    }
}

/// Whether typed text parses to a sequence rather than silently becoming `A6B6` — the option
/// row's live validity, never enforced: what fails it is still stored, because
/// [`parse_sequence`] already answers for it.
fn sequence_is_valid(text: &str) -> bool {
    scan_sequence(text, |_| {}).is_some()
}

/// `A6B6`, silvia's default, as the array.
fn default_sequence() -> Vec<i32> {
    vec![0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1]
}

/// silvia's Random Seq: the one button under the rows, writing a fresh `sequence`.
pub static RANDOM_SEQ: Buttons = Buttons {
    buttons: &[("randomize", "Random Seq")],
    press: |_, seed| Settings {
        options: vec![("sequence", random_sequence(seed))],
        controls: Vec::new(),
    },
    pulse: None,
};

/// silvia's `randomSequence`, draw for draw: two to eleven letters, each an `A` or a `B` at
/// even odds, and where every letter came up the same the first is dropped and the other
/// letter put on the end, so the length stays and both are in it.
pub fn random_sequence(seed: u32) -> String {
    let mut rng = crate::nodes::rng::Rng::from_word(seed);
    let len = 2 + (rng.next_f32() * 10.0) as usize;
    let text: String = (0..len)
        .map(|_| if rng.next_f32() < 0.5 { 'A' } else { 'B' })
        .collect();
    if !text.contains('A') {
        format!("{}A", &text[1..])
    } else if !text.contains('B') {
        format!("{}B", &text[1..])
    } else {
        text
    }
}

/// The sequence array and the radial mapping every Lyapunov output starts from.
fn lyapunov_common_wgsl(sequence: &str) -> String {
    let seq = parse_sequence(sequence);
    let len = seq.len();
    let values: Vec<String> = seq.iter().map(i32::to_string).collect();
    [
        format!("    var seq = array<i32, {len}>({});\n", values.join(", ")),
        "    let scale = {scale};
    let amp = {amplitude};
    let contrast = {contrast};
    let depth = {depth};
    let a = {centerA} + length(uv) * scale;
    let b = {centerB} + sin(atan2(uv.y, uv.x)) * scale;
    var h = 0.0;
"
        .to_string(),
        exponent_wgsl("a", "b", "h", len),
        "    let t = clamp(0.5 + 0.5 * tanh(h * contrast), 0.0, 1.0);\n".to_string(),
    ]
    .concat()
}

/// The exponent field lit as a height map, which is what makes the ridges read as ridges.
///
/// The specular highlight is white scaled by the base's alpha, which keeps the lit color
/// premultiplied over a half-transparent ramp.
fn lyapunov_color_wgsl(sequence: &str) -> String {
    let len = parse_sequence(sequence).len();
    [
        "    let base = mix({foreground}, {background}, t);
    if (depth <= 0.001) { return base; }
    let eps = 0.005 / max(scale, 1e-4);
    var hx = 0.0;
    var hy = 0.0;
"
        .to_string(),
        exponent_wgsl("a + eps", "b", "hx", len),
        exponent_wgsl("a", "b + eps", "hy", len),
        "    let normal = normalize(vec3f((h - hx) * depth, (h - hy) * depth, eps));
    let light = normalize(vec3f(0.4, 0.4, 1.0));
    let diffuse = max(dot(normal, light), 0.0);
    let specular = pow(max(reflect(-light, normal).z, 0.0), 20.0);
    return vec4f(base.rgb * (0.12 + 0.88 * diffuse) + specular * 0.25 * base.a, base.a);"
            .to_string(),
    ]
    .concat()
}

node! {
    /// The Markus–Lyapunov fractal of a sin² map, mapped radially.
    LYAPUNOV,
    slug: "lyapunov",
    icon: "🦋",
    label: "Lyapunov",
    category: Generate,
    tooltip: "The Lyapunov fractal of a sin² map, with radius driving A and angle driving B. \
              Its mask is the stable region; depth lights it as a height map.",
    // "Stable Color" does not fit the default 200.
    width: 216.0,
    inputs: [
        VaryingColor "foreground" "Stable Color" = Control::color("#ffffffff"),
        VaryingColor "background" "Chaos Color" = Control::color("#000000ff"),
        VaryingNumber "centerA" "Center A" = Control::num(2.0, -2.0, 8.0, 0.01, ""),
        VaryingNumber "centerB" "Center B" = Control::num(2.0, -2.0, 8.0, 0.01, ""),
        VaryingNumber "scale" "Scale" = Control::num_log(1.5, 0.05, 10.0, 0.01, ""),
        VaryingNumber "contrast" "Contrast" = Control::num(3.0, 0.1, 20.0, 0.1, ""),
        VaryingNumber "amplitude" "Amplitude" = Control::num(1.95, 0.1, 4.0, 0.01, ""),
        VaryingNumber "depth" "Depth" = Control::num(1.0, 0.0, 10.0, 0.1, ""),
    ],
    regions: &[Region::Buttons(&RANDOM_SEQ)],
    options: [
        // Typed free text, as silvia's own field is: the eight named sequences are presets
        // beside it rather than the only ones reachable.
        "sequence" "Sequence" = "A6B6" [
            "AB" => "AB",
            "AAB" => "AAB",
            "ABB" => "ABB",
            "AABB" => "AABB",
            "AABAB" => "AABAB",
            "ABBB" => "ABBB",
            "A6B6" => "A6B6",
            "B6A6" => "B6A6",
        ] free "e.g. AB, A4B2, AABAB" checked by sequence_is_valid,
    ],
    wgsl_common: varying(|node, ctx| lyapunov_common_wgsl(ctx.option(node, "sequence"))),
    outputs: [
        VaryingColor "color" "Color" = varying(|node, ctx| lyapunov_color_wgsl(ctx.option(node, "sequence"))),
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return 1.0 - t;",
    ],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_count_expands_and_rubbish_falls_back() {
        assert_eq!(parse_sequence("AB"), vec![0, 1]);
        assert_eq!(parse_sequence("A2B3"), vec![0, 0, 1, 1, 1]);
        assert_eq!(parse_sequence("A^2B"), vec![0, 0, 1]);
        assert_eq!(parse_sequence("A6B6"), default_sequence());
        // Nothing usable, and a run longer than the array may be, both give the default.
        assert_eq!(parse_sequence(""), default_sequence());
        assert_eq!(parse_sequence("zzz"), default_sequence());
        assert_eq!(parse_sequence("A64B64A64"), default_sequence());
        assert_eq!(
            parse_sequence("A999999999999999999999999"),
            vec![0],
            "a run count too long to hold reads as one"
        );
    }

    /// The exponent runs a fixed count with no exit of its own, and nothing on the node sets
    /// it: the count is a constant in the WGSL rather than a uniform or a field.
    #[test]
    fn the_exponent_runs_a_fixed_count() {
        let wgsl = lyapunov_common_wgsl("AB") + &lyapunov_color_wgsl("AB");
        assert_eq!(
            wgsl.matches(&format!("n < {ITERATIONS}; n++")).count(),
            3,
            "{wgsl}"
        );
        assert!(!wgsl.contains("break"), "{wgsl}");
        assert!(!wgsl.contains("iterations"), "{wgsl}");
        assert!(LYAPUNOV.input("iterations").is_none());
    }

    /// The sequence row is silvia's typed field, not a closed list: the macro carried the
    /// placeholder and the validator through, and the eight named sequences are presets.
    #[test]
    fn the_sequence_option_is_free_text_with_its_validator() {
        let option = LYAPUNOV.option("sequence").expect("lyapunov has one");
        assert!(option.placeholder.is_some());
        assert!(option.validate.is_some_and(|v| v("A4B2")));
        assert_eq!(option.choices.len(), 8, "the presets are still there");
    }

    /// Validity is exactly whether `parse_sequence` would fall back — the same grammar,
    /// checked rather than defaulted, which is what the option row's border reads.
    #[test]
    fn sequence_validity_is_whether_it_would_fall_back() {
        assert!(sequence_is_valid("AB"));
        assert!(sequence_is_valid("A6B6"));
        assert!(sequence_is_valid("a4b2"), "lowercase, as silvia accepts");
        assert!(!sequence_is_valid(""));
        assert!(!sequence_is_valid("zzz"));
        assert!(!sequence_is_valid("A64B64A64"), "over 128 entries");
    }

    /// Random Seq writes what silvia's does: two to eleven letters, A and B both in it, and
    /// every one a sequence the node compiles rather than falls back from. Over enough seeds
    /// every length comes up, and the sequences are not all one.
    #[test]
    fn random_seq_rolls_silvias_sequences() {
        let mut lengths = std::collections::BTreeSet::new();
        let mut seen = std::collections::HashSet::new();
        for seed in 0..4000 {
            let text = random_sequence(seed);
            assert!((2..=11).contains(&text.len()), "{text:?}");
            assert!(text.chars().all(|c| c == 'A' || c == 'B'), "{text:?}");
            assert!(text.contains('A') && text.contains('B'), "{text:?}");
            assert!(sequence_is_valid(&text), "{text:?}");
            assert!(crate::nodes::option_is_valid(&LYAPUNOV, "sequence", &text));
            lengths.insert(text.len());
            seen.insert(text);
        }
        assert_eq!(
            lengths,
            (2..=11).collect(),
            "every length silvia's can roll"
        );
        assert!(seen.len() > 1000, "{} distinct in 4000", seen.len());

        // The button is the sequence and nothing else, and it rolls on what it is handed.
        let Settings { options, controls } = (RANDOM_SEQ.press)(0, 7);
        assert_eq!(options, vec![("sequence", random_sequence(7))]);
        assert!(controls.is_empty());
        assert_eq!(LYAPUNOV.regions, &[Region::Buttons(&RANDOM_SEQ)]);
    }
}
