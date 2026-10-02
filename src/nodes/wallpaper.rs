// SPDX-License-Identifier: AGPL-3.0-or-later

//! The seventeen plane symmetry groups, from silvia's `wallpaper.js`, itself Frank A.
//! Farris's *Creating Symmetry*.
//!
//! The construction is one idea: a point is carried onto a lattice, a short sum of plane
//! waves invariant under the chosen group is evaluated there, and the complex number that
//! comes back is the coordinate the input is sampled at. So the node is a `Transform` — its
//! picture is its input passed through — and the pattern repeats forever because the wave sum
//! does.
//!
//! Three departures from the source, and one thing kept.
//!
//! **The group is a `Code` option, not a uniform.** silvia writes `int sym_type` and a
//! seventeen-arm `if` chain into every shader; here the arm is chosen while the shader is
//! built, so a `p6m` program carries the hexagonal lattice map and three `W_nm` terms and
//! nothing else. The unused half of that is not only instructions: `latticeParam` is spliced
//! only into the branches that read it, so a square or hexagonal group declares no uniform
//! for it, and `coeff5` and `coeff6` reach only the two generic groups that use them.
//!
//! **The hint line is the choice's label.** silvia draws the select, a read-only textarea
//! under it holding a line about the chosen group, and a credit under that. An [`OptionDef`]
//! here carries no per-choice tooltip and no text under the row, and the two places a string
//! can go are the choice's display name and the node's tooltip — so each choice is
//! `p6m (Hexagonal) — 6-fold rotation and mirrors`, whole in the open list and truncated from
//! the right in the closed row, where `node_widget`'s `Keep::Start` keeps the
//! crystallographer's name that tells the seventeen apart. The credit is the tooltip's last
//! sentence.
//!
//! **`angle` is published.** The wave sum is a complex number and both its components become
//! the sampling coordinate, so the field beside the picture is its argument — a polar sweep,
//! 0 to 1, which is the pattern's own phase. Its modulus is the other half and is not
//! published: six coefficients of ±2 put no bound on it, and a field that is neither
//! coverage nor a 0-to-1 quantity has no name in the vocabulary.
//!
//! What is kept, deliberately: `pg`'s first term is `E(1,0) - E(1,-0)`, which is identically
//! zero. That is Farris's rule rather than a slip — a glide reflection flips the sign of a
//! term whose second index is its own negation — so `Coeff 1` does nothing on `pg`, exactly
//! as it does nothing in silvia.

use crate::graph::PortType::{VaryingColor, VaryingNumber};
use crate::nodes::macros::{node, varying};
use crate::nodes::{Category, Control, InputDef, NodeDef, OptionDef, OutputDef, OutputKind};

/// The lattice maps and the four wave summations, from silvia's `shaderUtils`.
///
/// `E_nm` is the plane wave itself; `T_nm`, `S_nm` and `W_nm` average it over a 2-, 4- and
/// 3-fold rotation, which is what makes a sum of them invariant under the group. silvia also
/// declares a complex multiply that nothing calls; it is not here.
const WALLPAPER_WAVES_WGSL: &str =
    "fn cexp_i(angle: f32) -> vec2f { return vec2f(cos(angle), sin(angle)); }
fn getXY_square(z: vec2f) -> vec2f { return z; }
fn getXY_hex(z: vec2f) -> vec2f { return vec2f(z.x + z.y / sqrt(3.0), 2.0 * z.y / sqrt(3.0)); }
fn getXY_rect(z: vec2f, L: f32) -> vec2f { return vec2f(z.x, z.y / max(0.001, L)); }
fn getXY_rhombic(z: vec2f, b: f32) -> vec2f {
    let b_safe = max(0.001, b);
    return vec2f(z.x + z.y / (2.0 * b_safe), z.x - z.y / (2.0 * b_safe));
}
fn getXY_generic(z: vec2f, L: f32) -> vec2f {
    let omega = vec2f(0.5, L);
    let eta = max(0.001, omega.y);
    return vec2f(z.x - (omega.x / eta) * z.y, z.y / eta);
}
fn E_nm(nm: vec2f, XY: vec2f) -> vec2f { return cexp_i(2.0 * PI * dot(nm, XY)); }
fn T_nm(nm: vec2f, XY: vec2f) -> vec2f { return (E_nm(nm, XY) + E_nm(-nm, XY)) * 0.5; }
fn S_nm(nm: vec2f, XY: vec2f) -> vec2f {
    let nm2 = vec2f(nm.y, -nm.x);
    let nm3 = vec2f(-nm.x, -nm.y);
    let nm4 = vec2f(-nm.y, nm.x);
    return (E_nm(nm, XY) + E_nm(nm2, XY) + E_nm(nm3, XY) + E_nm(nm4, XY)) / 4.0;
}
fn W_nm(nm: vec2f, XY: vec2f) -> vec2f {
    let nm2 = vec2f(nm.y, -nm.x - nm.y);
    let nm3 = vec2f(-nm.x - nm.y, nm.x);
    return (E_nm(nm, XY) + E_nm(nm2, XY) + E_nm(nm3, XY)) / 3.0;
}";

/// The six frequency pairs every group draws its terms from, silvia's `freqPairs`.
const FREQUENCY_PAIRS_WGSL: &str = "    const nm1 = vec2f(1.0, 0.0);
    const nm2 = vec2f(0.0, 1.0);
    const nm3 = vec2f(1.0, 1.0);
    const nm4 = vec2f(1.0, -1.0);
    const nm5 = vec2f(2.0, 1.0);
    const nm6 = vec2f(1.0, 2.0);
";

/// The lattice a group's point is carried onto, as the line that computes `XY`.
///
/// Five maps over seventeen groups: generic and rhombic read `Lattice Param`, rectangular
/// reads it as the cell's aspect, and square and hexagonal have no parameter to read.
fn lattice_wgsl(group: &str) -> &'static str {
    match group {
        "p1" | "p2" => "    let XY = getXY_generic(z, {latticeParam});\n",
        "cm" | "cmm" => "    let XY = getXY_rhombic(z, {latticeParam});\n",
        "p4" | "p4m" | "p4g" => "    let XY = getXY_square(z);\n",
        "p3" | "p31m" | "p3m1" | "p6" | "p6m" => "    let XY = getXY_hex(z);\n",
        _ => "    let XY = getXY_rect(z, {latticeParam});\n",
    }
}

/// The wave sum that is invariant under `group`, as the line that computes `f`.
///
/// Only the two generic groups spend all six coefficients; the rest spend four or three, and
/// a coefficient a group does not name reaches neither the shader nor its uniforms.
fn waves_wgsl(group: &str) -> &'static str {
    match group {
        "p1" => {
            "    let f = ({coeff1}) * E_nm(nm1, XY)
           + ({coeff2}) * E_nm(nm2, XY)
           + ({coeff3}) * E_nm(nm3, XY)
           + ({coeff4}) * E_nm(nm4, XY)
           + ({coeff5}) * E_nm(nm5, XY)
           + ({coeff6}) * E_nm(nm6, XY);
"
        }
        "p2" => {
            "    let f = ({coeff1}) * T_nm(nm1, XY)
           + ({coeff2}) * T_nm(nm2, XY)
           + ({coeff3}) * T_nm(nm3, XY)
           + ({coeff4}) * T_nm(nm4, XY)
           + ({coeff5}) * T_nm(nm5, XY)
           + ({coeff6}) * T_nm(nm6, XY);
"
        }
        "pm" => {
            "    let f = ({coeff1}) * (E_nm(nm1, XY) + E_nm(vec2f(1.0, 0.0), XY))
           + ({coeff2}) * (E_nm(nm2, XY) + E_nm(vec2f(0.0, -1.0), XY))
           + ({coeff3}) * (E_nm(nm3, XY) + E_nm(vec2f(1.0, -1.0), XY))
           + ({coeff4}) * (E_nm(nm5, XY) + E_nm(vec2f(2.0, -1.0), XY));
"
        }
        "pg" => {
            "    let f = ({coeff1}) * (E_nm(nm1, XY) - E_nm(vec2f(1.0, 0.0), XY))
           + ({coeff2}) * (E_nm(nm2, XY) + E_nm(vec2f(0.0, -1.0), XY))
           + ({coeff3}) * (E_nm(nm3, XY) - E_nm(vec2f(1.0, -1.0), XY))
           + ({coeff4}) * (E_nm(vec2f(2.0, 1.0), XY) + E_nm(vec2f(2.0, -1.0), XY));
"
        }
        "cm" => {
            "    let f = ({coeff1}) * (E_nm(nm1, XY) + E_nm(nm2, XY))
           + ({coeff2}) * E_nm(nm3, XY)
           + ({coeff3}) * (E_nm(nm5, XY) + E_nm(nm6, XY))
           + ({coeff4}) * E_nm(vec2f(2.0, 2.0), XY);
"
        }
        "pmm" => {
            "    let f = ({coeff1}) * T_nm(nm1, XY)
           + ({coeff2}) * T_nm(nm2, XY)
           + ({coeff3}) * T_nm(nm3, XY);
"
        }
        "pmg" => {
            "    let f = ({coeff1}) * T_nm(nm2, XY)
           + ({coeff2}) * (T_nm(nm1, XY) - T_nm(vec2f(1.0, 0.0), XY))
           + ({coeff3}) * (T_nm(nm3, XY) - T_nm(vec2f(1.0, -1.0), XY));
"
        }
        "pgg" => {
            "    let f = ({coeff1}) * T_nm(nm3, XY)
           + ({coeff2}) * (T_nm(nm1, XY) - T_nm(vec2f(1.0, 0.0), XY))
           + ({coeff3}) * (T_nm(nm2, XY) - T_nm(vec2f(0.0, -1.0), XY));
"
        }
        "cmm" => {
            "    let f = ({coeff1}) * T_nm(nm3, XY)
           + ({coeff2}) * (T_nm(nm1, XY) + T_nm(nm2, XY))
           + ({coeff3}) * (T_nm(nm5, XY) + T_nm(nm6, XY));
"
        }
        "p4" => {
            "    let f = ({coeff1}) * S_nm(nm1, XY)
           + ({coeff2}) * S_nm(nm2, XY)
           + ({coeff3}) * S_nm(nm3, XY)
           + ({coeff4}) * S_nm(nm5, XY);
"
        }
        "p4m" => {
            "    let f = ({coeff1}) * S_nm(nm1, XY)
           + ({coeff2}) * S_nm(nm3, XY)
           + ({coeff3}) * (S_nm(nm5, XY) + S_nm(nm6, XY))
           + ({coeff4}) * S_nm(vec2f(2.0, 0.0), XY);
"
        }
        "p4g" => {
            "    let f = ({coeff1}) * S_nm(nm3, XY)
           + ({coeff2}) * (S_nm(nm1, XY) - S_nm(nm2, XY))
           + ({coeff3}) * (S_nm(nm5, XY) - S_nm(nm6, XY))
           + ({coeff4}) * S_nm(vec2f(2.0, 2.0), XY);
"
        }
        "p3" => {
            "    let f = ({coeff1}) * W_nm(nm1, XY)
           + ({coeff2}) * W_nm(nm2, XY)
           + ({coeff3}) * W_nm(nm3, XY)
           + ({coeff4}) * W_nm(nm5, XY);
"
        }
        "p31m" => {
            "    let f = ({coeff1}) * (W_nm(nm1, XY) + W_nm(nm2, XY))
           + ({coeff2}) * W_nm(nm3, XY)
           + ({coeff3}) * (W_nm(nm5, XY) + W_nm(nm6, XY))
           + ({coeff4}) * W_nm(vec2f(2.0, 0.0), XY);
"
        }
        "p3m1" => {
            "    let f = ({coeff1}) * (W_nm(nm1, XY) + W_nm(vec2f(0.0, -1.0), XY))
           + ({coeff2}) * (W_nm(nm2, XY) + W_nm(vec2f(-1.0, 0.0), XY))
           + ({coeff3}) * W_nm(nm3, XY);
"
        }
        "p6" => {
            "    let f = ({coeff1}) * (W_nm(nm1, XY) + W_nm(-nm1, XY))
           + ({coeff2}) * (W_nm(nm2, XY) + W_nm(-nm2, XY))
           + ({coeff3}) * (W_nm(nm3, XY) + W_nm(-nm3, XY));
"
        }
        _ => {
            "    let f = ({coeff1}) * W_nm(nm1, XY)
           + ({coeff2}) * W_nm(nm3, XY)
           + ({coeff3}) * (W_nm(nm5, XY) + W_nm(nm6, XY));
"
        }
    }
}

node! {
    /// The input read through one of the seventeen plane symmetry groups.
    WALLPAPER,
    slug: "wallpaper",
    icon: "💠",
    label: "Wallpaper",
    category: Transform,
    tooltip: "Samples the input along a sum of plane waves invariant under one of the \
              seventeen plane symmetry groups, so the picture repeats forever on that \
              group's lattice. Which coefficients bite depends on the group; only the two \
              generic ones use all six. After Frank A. Farris's Creating Symmetry.",
    // The symmetry row is a long name and a longer hint, and "Texture Scale" does not fit
    // the default 200 either.
    width: 260.0,
    inputs: [
        VaryingColor "input" "Input" at "finalUV" = Control::None,
        VaryingNumber "scale" "Frequency" = Control::num(1.0, 0.1, 5.0, 0.01, "/⬓"),
        VaryingNumber "texScale" "Texture Scale" = Control::num(0.5, 0.01, 5.0, 0.01, ""),
        VaryingNumber "latticeParam" "Lattice Param" = Control::num(1.0, 0.1, 5.0, 0.01, ""),
        VaryingNumber "coeff1" "Coeff 1" = Control::num(1.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "coeff2" "Coeff 2" = Control::num(0.5, -2.0, 2.0, 0.01, ""),
        VaryingNumber "coeff3" "Coeff 3" = Control::num(0.2, -2.0, 2.0, 0.01, ""),
        VaryingNumber "coeff4" "Coeff 4" = Control::num(0.1, -2.0, 2.0, 0.01, ""),
        VaryingNumber "coeff5" "Coeff 5" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
        VaryingNumber "coeff6" "Coeff 6" = Control::num(0.0, -2.0, 2.0, 0.01, ""),
    ],
    options: [
        "symmetry" "Symmetry Type" = "p6m" [
            "p1" => "p1 (Generic) — uses Lattice Param; all six coefficients are independent",
            "p2" => "p2 (Generic) — 2-fold rotation; uses Lattice Param",
            "pm" => "pm (Rectangular) — horizontal mirrors; uses Lattice Param",
            "pg" => "pg (Rectangular) — horizontal glides; uses Lattice Param",
            "cm" => "cm (Rhombic) — centered, with diagonal mirrors; uses Lattice Param",
            "pmm" => "pmm (Rectangular) — horizontal and vertical mirrors",
            "pmg" => "pmg (Rectangular) — vertical mirrors and horizontal glides",
            "pgg" => "pgg (Rectangular) — horizontal and vertical glides",
            "cmm" => "cmm (Rhombic) — centered, with full mirror symmetry",
            "p4" => "p4 (Square) — 4-fold rotation; Lattice Param is ignored",
            "p4m" => "p4m (Square) — 4-fold rotation and diagonal mirrors",
            "p4g" => "p4g (Square) — 4-fold rotation and diagonal glides",
            "p3" => "p3 (Hexagonal) — 3-fold rotation; Lattice Param is ignored",
            "p31m" => "p31m (Hexagonal) — 3-fold rotation and one mirror set",
            "p3m1" => "p3m1 (Hexagonal) — 3-fold rotation and another mirror set",
            "p6" => "p6 (Hexagonal) — 6-fold rotation",
            "p6m" => "p6m (Hexagonal) — 6-fold rotation and mirrors",
        ],
    ],
    wgsl_utils: [WALLPAPER_WAVES_WGSL],
    wgsl_common: varying(|node, ctx| {
        let group = ctx.option(node, "symmetry");
        [
            "    let z = uv * ({scale});\n",
            FREQUENCY_PAIRS_WGSL,
            lattice_wgsl(group),
            waves_wgsl(group),
        ]
        .concat()
    }),
    // `finalUV` belongs to the picture alone, so the angle's function neither declares the
    // texture scale's uniform nor builds a coordinate nothing samples at.
    outputs: [
        VaryingColor "output" "Output" = "    let finalUV = f * ({texScale}) + 0.5;
    return {input};",
        VaryingNumber "angle" "Angle" in "[0, 1]" = "    return atan2(f.y, f.x) / (2.0 * PI) + 0.5;",
    ],
}
