// SPDX-License-Identifier: AGPL-3.0-or-later

//! Beat-locked block displacement: the picture cut into blocks, a few of them shoved, and
//! red and blue pulled apart along the shove.
//!
//! Ported from silvia's `glitch.js`, which silvia files under a `Distort` category of its
//! own. There is no such category here, so it is `Category::Effect`: what a fragment gets
//! depends on which block it sits inside, which is the cell half of what that category is
//! defined as — the same reading that puts `mosaic` there.
//!
//! **The block hierarchy, the hashing and the channel split are silvia's arithmetic
//! unchanged.** A coarse block hashes itself into one of three sizes, so `blockSize` is the
//! finest unit rather than the only one; a second hash decides whether the block moves at
//! all; a third picks how far, and a fourth how far apart the channels go.
//!
//! **Step is a knob, where silvia had a BPM row, a subdivision menu and a button.** silvia
//! quantizes `u_time` into a beat count and adds a counter a button increments, all inside
//! the node. Here the beat is already a cable — a Master Gear fires it, `clockdivider` thins
//! it, `counter` counts it and `button` covers the hand — so the node reads one number and
//! floors it. The pattern is therefore still dead between whole numbers, which is what makes
//! this a glitch rather than noise, and two of these on one counter glitch in lockstep.
//!
//! There is no speed here and so no accumulator: the node's whole clock is the number on
//! that knob.
//!
//! **A block that is not picked returns early**, as silvia's does. The alternative is three
//! samples of everything upstream at every fragment of a picture that is mostly untouched,
//! which is the one place in this library where the branch is the cheap shape.
//!
//! `mask` is the picked blocks — coverage, 1 inside a block that moved — which is the field
//! the body computes on the way to the picture and silvia throws away.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// One axis' block index, from silvia's `varBlock`: a coarse block hashes itself into one of
/// three subdivisions, and the index is offset per subdivision so the three never collide.
const GLITCH_BLOCK_WGSL: &str =
    "fn glitchBlock(coord: f32, blockSize: f32, stepIndex: f32) -> f32 {
    let coarse = floor(coord / (blockSize * 4.0));
    let scale = fract(sin(coarse * 127.1 + stepIndex * 0.37) * 43758.5453);
    if (scale < 0.3) {
        return coarse;
    }
    if (scale < 0.65) {
        return floor(coord / (blockSize * 2.0)) + 10000.0;
    }
    return floor(coord / blockSize) + 20000.0;
}";

/// The knobs every direction reads, and the step the whole pattern is quantized to.
const SETUP_WGSL: &str = "    let blockSize = max({blockSize}, 1e-4);
    let stepIndex = floor({step});
    let intensity = {intensity};
";

/// The block index a direction hashes, and whether this fragment's block was picked.
///
/// Horizontal blocks run across the frame, so their index comes from `uv.y`; vertical ones
/// from `uv.x`. Both hashes the pair together, which is silvia's cell.
fn blocks_wgsl(direction: &str) -> &'static str {
    match direction {
        "vertical" => {
            "    let blockCoord = glitchBlock(uv.x, blockSize, stepIndex);
    let blockRand = fract(sin(dot(vec2f(blockCoord, stepIndex), vec2f(12.9898, 78.233))) * 43758.5453);
    let mask = 1.0 - step(intensity, blockRand);
"
        }
        "both" => {
            "    let blockH = glitchBlock(uv.y, blockSize, stepIndex);
    let blockV = glitchBlock(uv.x, blockSize, stepIndex);
    let cellRand = fract(sin(dot(vec2f(blockH, blockV), vec2f(12.9898, 78.233)) + stepIndex * 43.17) * 43758.5453);
    let mask = 1.0 - step(intensity, cellRand);
"
        }
        _ => {
            "    let blockCoord = glitchBlock(uv.y, blockSize, stepIndex);
    let blockRand = fract(sin(dot(vec2f(blockCoord, stepIndex), vec2f(12.9898, 78.233))) * 43758.5453);
    let mask = 1.0 - step(intensity, blockRand);
"
        }
    }
}

/// The picture: an untouched block returns at once, and a picked one is read three times —
/// the center, the red one way along the shove and the blue the other.
fn picture_wgsl(direction: &str) -> &'static str {
    match direction {
        "vertical" => {
            "    var glitchUV = uv;
    if (mask < 0.5) {
        return {input};
    }
    let shiftAmt = (fract(sin(dot(vec2f(blockCoord * 3.17, stepIndex * 1.71), vec2f(45.233, 94.117))) * 23421.631) - 0.5) * {shift};
    let shiftedUV = uv + vec2f(0.0, shiftAmt);
    let splitRand = fract(sin(dot(vec2f(blockCoord * 7.31, stepIndex * 2.13), vec2f(67.891, 12.345))) * 54321.987);
    let splitOffset = vec2f(0.0, splitRand * ({rgbSplit}));
    glitchUV = shiftedUV;
    let center = {input};
    glitchUV = shiftedUV + splitOffset;
    let r = ({input}).r;
    glitchUV = shiftedUV - splitOffset;
    let b = ({input}).b;
    return vec4f(r, center.g, b, center.a);"
        }
        "both" => {
            "    var glitchUV = uv;
    if (mask < 0.5) {
        return {input};
    }
    let shiftAngle = fract(sin(dot(vec2f(blockH * 3.17, blockV * 7.23), vec2f(45.233, 94.117)) + stepIndex * 1.71) * 23421.631) * 2.0 * PI;
    let shiftMag = fract(sin(dot(vec2f(blockH * 5.71, blockV * 2.93), vec2f(31.727, 58.913)) + stepIndex * 2.31) * 61283.419) * ({shift});
    let shiftDir = vec2f(cos(shiftAngle), sin(shiftAngle));
    let shiftedUV = uv + shiftDir * shiftMag;
    let splitRand = fract(sin(dot(vec2f(blockH * 7.31 + blockV * 2.17, stepIndex * 2.13), vec2f(67.891, 12.345))) * 54321.987);
    let splitOffset = shiftDir * (splitRand * ({rgbSplit}));
    glitchUV = shiftedUV;
    let center = {input};
    glitchUV = shiftedUV + splitOffset;
    let r = ({input}).r;
    glitchUV = shiftedUV - splitOffset;
    let b = ({input}).b;
    return vec4f(r, center.g, b, center.a);"
        }
        _ => {
            "    var glitchUV = uv;
    if (mask < 0.5) {
        return {input};
    }
    let shiftAmt = (fract(sin(dot(vec2f(blockCoord * 3.17, stepIndex * 1.71), vec2f(45.233, 94.117))) * 23421.631) - 0.5) * {shift};
    let shiftedUV = uv + vec2f(shiftAmt, 0.0);
    let splitRand = fract(sin(dot(vec2f(blockCoord * 7.31, stepIndex * 2.13), vec2f(67.891, 12.345))) * 54321.987);
    let splitOffset = vec2f(splitRand * ({rgbSplit}), 0.0);
    glitchUV = shiftedUV;
    let center = {input};
    glitchUV = shiftedUV + splitOffset;
    let r = ({input}).r;
    glitchUV = shiftedUV - splitOffset;
    let b = ({input}).b;
    return vec4f(r, center.g, b, center.a);"
        }
    }
}

node! {
    /// Blocks picked by a hash, shoved sideways with the channels pulled apart.
    DEF,
    slug: "glitch",
    icon: "📳",
    label: "Glitch",
    category: Effect,
    tooltip: "Cuts the picture into blocks that subdivide themselves, shoves the ones a hash \
              picks and pulls red and blue apart along the shove. Step freezes the pattern \
              and advances it by one, so a Counter on a Master Gear glitches on the beat.",
    inputs: [
        VaryingColor "input" "Input" at "glitchUV" = Control::None,
        VaryingNumber "intensity" "Intensity" = Control::num(0.15, 0.0, 1.0, 0.01, ""),
        VaryingNumber "shift" "Shift" = Control::num(0.15, 0.0, 1.0, 0.01, "⬓"),
        VaryingNumber "blockSize" "Block Size" = Control::num(0.05, 0.005, 0.3, 0.005, "⬓"),
        VaryingNumber "rgbSplit" "RGB Split" = Control::num(0.02, 0.0, 0.1, 0.001, "⬓"),
        VaryingNumber "step" "Step" = Control::num(0.0, 0.0, 64.0, 1.0, ""),
    ],
    options: [
        "direction" "Direction" = "horizontal" [
            "horizontal" => "Horizontal",
            "vertical" => "Vertical",
            "both" => "Both",
        ],
    ],
    wgsl_utils: [GLITCH_BLOCK_WGSL],
    wgsl_common: varying(|node, ctx| {
        [SETUP_WGSL, blocks_wgsl(ctx.option(node, "direction"))].concat()
    }),
    outputs: [
        VaryingColor "color" "Color" = varying(|node, ctx| {
            picture_wgsl(ctx.option(node, "direction")).to_string()
        }),
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
    ],
}
