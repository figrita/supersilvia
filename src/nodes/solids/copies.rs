// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which copy a repeat is drawing, as a field like any other.
//!
//! A repeat folds the point into one cell and evaluates its input there, so every copy is the
//! same solid asked about a different point. **The repeat says which copy it is asking for**:
//! round the call into its input it sets `sd_copy`, a private variable of the module, and puts
//! back what was there after. A node upstream that reads it — [`COPY`] or [`PER_COPY`] —
//! hands out an ordinary field, so it goes wherever a number goes: a sphere's
//! radius, a Multiply on the way there, an RGBA. A sphere's radius cabled from Per Copy is a
//! different radius in every copy, and the copies still cost what one does.
//!
//! **Nearest first.** A repeat inside a repeat sets its own copy and moves the one round it to
//! `sd_copy_outer`, so a reader asks for the nearest repeat or the one round that. A reader
//! outside every repeat — a picture, or a solid no repeat holds — reads a count of zero: Order
//! 0, and a Per Copy's From.
//!
//! **A repeat that nothing reads says nothing.** It asks the compiler whether a reader is
//! upstream of its input (`CompileContext::reads_copy`) and writes the variable only then.
//!
//! **Copies that differ are measured against their neighbor.** The fold looks at the nearest
//! cell, which is right while every copy is the same shape: no neighbor is nearer than the copy
//! in this cell. A copy taller than the one beside it is nearer to a point in that cell than
//! that cell's own copy, and a ray stepping by the shorter one's distance runs into it. So a
//! read Repeat and Repeat Around evaluate the nearest copy and the one on the near side of each
//! axis they repeat along, and keep the nearer: twice the cost for a row or a ring, four times
//! for a grid in three dimensions. Scatter on Sphere's cells are not a grid, and keep to the
//! nearest.

use super::{SDF, num, num_log};
use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::VaryingNumber;
use crate::nodes::{Category, Family, NodeDef, OptionDef, OutputDef, OutputKind};

/// The copy a repeat is drawing, emitted into any module with a repeat or a reader.
///
/// `index` counts from 0 and `count` is how many copies there are, 0 outside every repeat;
/// `random` is the repeat's own hash of the copy, so its Seed reshuffles it; `axes` is how far
/// along each axis of a grid the copy is, 0 to 1.
pub const COPY_STATE: &str = "// Which copy a repeat is drawing, nearest first.
struct SdCopy {
    index: f32,
    count: f32,
    random: f32,
    axes: vec3f,
}
var<private> sd_copy: SdCopy;
var<private> sd_copy_outer: SdCopy;
";

/// What a repeat writes round the call into its input: `before` works the copy out with the
/// statements in `prep` and says which it is with `value`, an `SdCopy` expression — both may
/// hold the repeat's own `{holes}` — and `after` puts the copy round it back. Both empty where
/// nothing upstream reads it.
pub(crate) fn enter(
    ctx: &CompileContext,
    node: NodeId,
    prep: &str,
    value: &str,
) -> (String, String) {
    if !ctx.reads_copy(node, "input") {
        return (String::new(), String::new());
    }
    (
        format!("{prep}{SAVE}    sd_copy = {value};\n"),
        RESTORE.to_string(),
    )
}

/// What a repeat writes before it sets its own copy: the copy round it kept to put back, and
/// moved to `sd_copy_outer` for a reader that asks for the outer one.
pub(crate) const SAVE: &str = "    let copy_was = sd_copy;
    let copy_outer_was = sd_copy_outer;
    sd_copy_outer = sd_copy;
";

/// What a repeat writes after its input has run: the copy round it, put back.
pub(crate) const RESTORE: &str = "    sd_copy = copy_was;
    sd_copy_outer = copy_outer_was;
";

/// The Repeat option every reader has: the nearest repeat, or the one round it.
const OF: OptionDef = OptionDef {
    key: "of",
    label: "Repeat",
    default: "nearest",
    choices: &[("nearest", "Nearest"), ("outer", "Outer")],
    ..OptionDef::EMPTY
};

/// The copy a reader reads, by its Repeat option.
fn state(node: NodeId, ctx: &CompileContext) -> &'static str {
    if ctx.option(node, "of") == "outer" {
        "sd_copy_outer"
    } else {
        "sd_copy"
    }
}

/// Fill every `{key}` hole in `src` naming one of `node`'s inputs with its value at `uv`: a
/// reader is a field, read wherever it is cabled.
fn fill_field(node: NodeId, ctx: &mut CompileContext, src: &str) -> String {
    let Some(def) = ctx.node(node).map(|n| n.def) else {
        return src.to_string();
    };
    let mut out = src.to_string();
    for input in def.inputs {
        let hole = ["{", input.key, "}"].concat();
        if out.contains(&hole) {
            let expr = ctx.input(node, input.key, "uv");
            out = out.replace(&hole, &expr);
        }
    }
    out
}

/// One of Copy's numbers, as a field.
const fn reading(key: &'static str, label: &'static str, range: &'static str) -> OutputDef {
    OutputDef {
        key,
        label,
        ty: VaryingNumber,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, func| {
            let c = state(node, ctx);
            let key = func.rsplit('_').next().unwrap_or("");
            let body = match key {
                "order" => format!("{c}.index / max({c}.count - 1.0, 1.0)"),
                "random" => format!("{c}.random"),
                "number" => format!("{c}.index"),
                "count" => format!("{c}.count"),
                "x" => format!("{c}.axes.x"),
                "y" => format!("{c}.axes.y"),
                _ => format!("{c}.axes.z"),
            };
            format!("    return {body};")
        },
        range: Some(range),
        ..OutputDef::EMPTY
    }
}

pub static COPY: NodeDef = NodeDef {
    slug: "copy",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "🔢",
    label: "Copy",
    tooltip: "Which copy a repeat is drawing, for any number inside it. Order runs \
              0 at the first copy to 1 at the last; Random is the repeat's own, reshuffled by \
              its Seed; Number counts 0, 1, 2…; X, Y and Z are how far along each axis of a \
              grid. Outside every repeat everything reads 0.",
    inputs: &[],
    options: &[OF],
    outputs: &[
        reading("order", "Order", "[0, 1]"),
        reading("random", "Random", "[0, 1]"),
        reading("number", "Number", "0, 1, 2…"),
        reading("count", "Count", "copies"),
        reading("x", "X", "[0, 1]"),
        reading("y", "Y", "[0, 1]"),
        reading("z", "Z", "[0, 1]"),
    ],
    wgsl_utils: &[SDF, COPY_STATE],
    ..NodeDef::EMPTY
};

pub static PER_COPY: NodeDef = NodeDef {
    slug: "percopy",
    category: Category::Solid,
    family: Some(Family::Repeat),
    icon: "🎚",
    label: "Per Copy",
    tooltip: "A number that runs From the first copy To the last, for any number inside a \
              repeat. By picks the walk: in Order, at Random, Alternating, out from \
              the Middle, or along one axis of a grid. Curve bends it; Seed reshuffles Random. \
              Outside every repeat it is From.",
    inputs: &[
        num("from", "From", 0.3, -20.0, 20.0, 0.01, ""),
        num("to", "To", 1.0, -20.0, 20.0, 0.01, ""),
        num_log("curve", "Curve", 1.0, 0.1, 10.0, 0.01, ""),
        num("seed", "Seed", 0.0, 0.0, 999.0, 1.0, ""),
    ],
    options: &[
        OptionDef {
            key: "by",
            label: "By",
            default: "order",
            choices: &[
                ("order", "Order"),
                ("random", "Random"),
                ("alternate", "Alternate"),
                ("middle", "Middle"),
                ("x", "X"),
                ("y", "Y"),
                ("z", "Z"),
            ],
            ..OptionDef::EMPTY
        },
        OF,
    ],
    outputs: &[OutputDef {
        key: "value",
        label: "Value",
        ty: VaryingNumber,
        kind: OutputKind::Shader,
        wgsl: |node, ctx, _| {
            let c = state(node, ctx);
            let t = match ctx.option(node, "by") {
                "random" => {
                    format!("sd_hash3(vec3f({c}.random * 4096.0, {c}.index, 0.0), {{seed}}).x")
                }
                "alternate" => format!("floor_mod({c}.index, 2.0)"),
                "middle" => format!("1.0 - abs(2.0 * {c}.index / max({c}.count - 1.0, 1.0) - 1.0)"),
                "x" => format!("{c}.axes.x"),
                "y" => format!("{c}.axes.y"),
                "z" => format!("{c}.axes.z"),
                _ => format!("{c}.index / max({c}.count - 1.0, 1.0)"),
            };
            fill_field(
                node,
                ctx,
                &format!(
                    "    let t = clamp({t}, 0.0, 1.0);
    return mix({{from}}, {{to}}, pow(t, max({{curve}}, 1.0e-3)));"
                ),
            )
        },
        ..OutputDef::EMPTY
    }],
    wgsl_utils: &[SDF, COPY_STATE],
    ..NodeDef::EMPTY
};
