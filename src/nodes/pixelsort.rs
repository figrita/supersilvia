// SPDX-License-Identifier: AGPL-3.0-or-later

//! The glitch smear: a run of the picture cut into chunks and each chunk reordered.
//!
//! Ported from silvia's `pixelsort.js`. It is the one `Effect` that reads the picture along a
//! *run* rather than over a patch around the fragment: every pixel of the chunk it landed in
//! is sampled, sorted by brightness or by hue, and the one that sorts to this fragment's
//! place in the chunk is what comes back. **It is the most expensive node in the library** —
//! a chunk of 64 is 64 samples of everything upstream and a further insertion sort over them,
//! per fragment — and the tooltip says so.
//!
//! **The seed is a knob, and silvia's New Seed button is not ported.** silvia hides a counter
//! behind a press and re-rolls it into a uniform nobody ever sees; here the seed is a row, so
//! a Button or a Master Gear into a Triggered Random re-rolls it, a moving number animates it —
//! which is all silvia's Animate switch did — and two nodes can share one. See
//! [decisions.md](../../../docs/decisions.md).
//!
//! **The run is the pixel grid, not the window.** silvia indexes its scanlines off
//! `u_resolution`, so a chunk is a different length in an Output of another size. Here a
//! pixel is a pixel of a [`REFERENCE_HEIGHT`]-high frame, the same convention every kernel in
//! `convolve.rs` steps by, and the index is biased into the positives before it is divided:
//! worldspace runs either side of zero, and integer division truncates toward it, which would
//! fold two chunks into one along the picture's middle.
//!
//! `mask` is where it sorted: 1 inside a chunk whose contrast cleared Threshold, 0 where the
//! picture fell through untouched. The body worked that out on the way to the picture, so it
//! is a port rather than something to rebuild from the color.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::distort::HASH_RANDOM_WGSL;
use crate::nodes::macros::{node, varying};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind, REFERENCE_HEIGHT,
};

/// The two keys a chunk may be sorted by, as functions of a sampled color's own channels.
///
/// `pixelSortHue` is `decompose::CONVERSIONS`' own hue expression, which is silvia's
/// `rgb2hsv(...).x`: a gray pixel has no hue and reads 0 rather than dividing by zero. The
/// table's chain of `select`s is written out here as an `if`.
pub const SORT_KEYS_WGSL: &str = "fn pixelSortLuma(c: vec3f) -> f32 {
    return dot(c, vec3f(0.299, 0.587, 0.114));
}

fn pixelSortHue(c: vec3f) -> f32 {
    let maxc = max(max(c.r, c.g), c.b);
    let minc = min(min(c.r, c.g), c.b);
    let delta = maxc - minc;
    if (delta <= 0.0) {
        return 0.0;
    }
    var sector: f32;
    if (maxc == c.r) {
        sector = (c.g - c.b) / delta;
    } else if (maxc == c.g) {
        sector = (c.b - c.r) / delta + 2.0;
    } else {
        sector = (c.r - c.g) / delta + 4.0;
    }
    return fract(sector / 6.0);
}";

/// Half the pixels a worldspace coordinate can reach, added to a scanline index so the
/// division that finds its chunk never sees a negative.
const INDEX_BIAS: i32 = 16384;

/// The chunk length one choice of Chunk Size means. It is the loop's bound, so it is an
/// option and not a knob, and the value is the sample count outright.
fn chunk_size(size: &str) -> i32 {
    match size {
        "16" => 16,
        "64" => 64,
        _ => 32,
    }
}

/// Which way the run lies: the step along it, the coordinate it is indexed by, and the one
/// that names the run.
fn run_axes_wgsl(direction: &str) -> (&'static str, &'static str, &'static str) {
    if direction == "vertical" {
        ("vec2f(0.0, psStep)", "pixel.y", "pixel.x")
    } else {
        ("vec2f(psStep, 0.0)", "pixel.x", "pixel.y")
    }
}

node! {
    /// A run of the picture cut into chunks, each chunk sorted by brightness or hue.
    PIXELSORT,
    slug: "pixelsort",
    icon: "🧮",
    label: "Pixel Sort",
    category: Effect,
    tooltip: "Walks each row or column, cuts it into chunks at hashed boundaries, and sorts \
              the pixels inside a chunk by brightness or hue, leaving a chunk whose contrast \
              is under Threshold alone. The classic glitch smear, and the most expensive node \
              here: Chunk Size is how many samples of everything upstream each fragment \
              takes, and it sorts them on top of that. Its mask is where it sorted.",
    inputs: [
        VaryingColor "input" "Input" at "psUV" = Control::None,
        VaryingNumber "threshold" "Threshold" = Control::num(0.3, 0.0, 1.0, 0.01, ""),
        VaryingNumber "seed" "Seed" = Control::num(0.0, 0.0, 1.0, 0.001, ""),
    ],
    options: [
        "direction" "Direction" = "horizontal" [
            "horizontal" => "Horizontal",
            "vertical" => "Vertical",
        ],
        "sortBy" "Sort By" = "brightness" [
            "brightness" => "Brightness",
            "hue" => "Hue",
        ],
        "sortOrder" "Sort" = "ascending" [
            "ascending" => "Ascending",
            "descending" => "Descending",
        ],
        "chunkSize" "Chunk Size" = "32" [
            "16" => "16 samples",
            "32" => "32 samples",
            "64" => "64 samples",
        ],
    ],
    wgsl_utils: [HASH_RANDOM_WGSL, SORT_KEYS_WGSL],
    wgsl_common: varying(|node, ctx| {
        let n = chunk_size(ctx.option(node, "chunkSize"));
        let (step, scan, run) = run_axes_wgsl(ctx.option(node, "direction"));
        let key = if ctx.option(node, "sortBy") == "hue" {
            "pixelSortHue"
        } else {
            "pixelSortLuma"
        };
        format!(
            "    let thresh = {{threshold}};
    let salt = hashBits(bitcast<u32>({{seed}}));
    let pixel = uv * ({REFERENCE_HEIGHT:?} * 0.5);
    let psStep = 2.0 / {REFERENCE_HEIGHT:?};
    let dir = {step};
    let scanIdx = i32(floor({scan})) + {INDEX_BIAS};
    let runIdx = i32(floor({run})) + {INDEX_BIAS};

    let runHash = hashBits(u32(runIdx) ^ salt);
    let shifted = scanIdx + i32(runHash % {n}u);
    let superIdx = shifted / {n};
    let posInSuper = shifted - superIdx * {n};

    let splitAt = i32(hashBits(u32(superIdx) ^ runHash) % {n}u);
    let subStart = select(splitAt, 0, posInSuper < splitAt);
    let subLen = select({n} - splitAt, splitAt, posInSuper < splitAt);
    let myPos = posInSuper - subStart;
    let chunkStart = uv - dir * f32(myPos);

    var keys: array<f32, {n}>;
    var order: array<i32, {n}>;
    var minKey = 1.0;
    var maxKey = 0.0;
    var psUV = uv;
    for (var i = 0; i < {n}; i++) {{
        if (i >= subLen) {{ break; }}
        psUV = chunkStart + dir * f32(i);
        let k = {key}(unpremultiply({{input}}).rgb);
        keys[i] = k;
        order[i] = i;
        minKey = min(minKey, k);
        maxKey = max(maxKey, k);
    }}
    let mask = select(0.0, 1.0, subLen > 1 && thresh < 1.0 && maxKey - minKey >= thresh);
    psUV = uv;
"
        )
    }),
    outputs: [
        VaryingColor "color" "Color" = varying(|node, ctx| {
            let n = chunk_size(ctx.option(node, "chunkSize"));
            let cmp = if ctx.option(node, "sortOrder") == "descending" { "<" } else { ">" };
            format!(
                "    if (mask == 0.0) {{ return {{input}}; }}
    for (var i = 1; i < {n}; i++) {{
        if (i >= subLen) {{ break; }}
        let k = keys[i];
        let idx = order[i];
        var insertAt = i;
        for (var j = i - 1; j >= 0; j--) {{
            if (!(keys[j] {cmp} k)) {{ break; }}
            keys[j + 1] = keys[j];
            order[j + 1] = order[j];
            insertAt = j;
        }}
        keys[insertAt] = k;
        order[insertAt] = idx;
    }}
    psUV = chunkStart + dir * f32(order[myPos]);
    return {{input}};"
            )
        }),
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
    ],
}
