// SPDX-License-Identifier: AGPL-3.0-or-later

//! A repeat's per-copy rows: where each copy sits, how it turns, how big it is on each axis,
//! and its color.
//!
//! **Copies vary on the repeat alone.** Whatever is upstream of a repeat is the same in every
//! copy; the repeat moves, turns, sizes and tints each copy by its own rows, under a **Per
//! copy** heading that folds. A copy number a node upstream could read was tried and lost: the
//! link that decided its value was drawn nowhere (docs/decisions.md, "Copies vary on the
//! repeat alone").
//!
//! **Each row is a list port with a knob.** A knob, or a single number cabled in, is a list of
//! one and reaches every copy alike. A list gives copy i an item of its own: on a repeat with a
//! count, item `floor(i · n / m)` for a list of n over m copies, so a list stretches across the
//! copies whatever its length; on an endless repeat, item `(x + y + z) mod n` of the cell's
//! coordinates, so the list cycles and each row starts one further along.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::graph::PortType::{ListColor, ListNumber};
use crate::nodes::{Control, InputDef, OptionDef, OptionKind};

/// The heading the per-copy rows fold under: closed on a new node.
pub const HEADING: OptionDef =
    OptionDef::heading("perCopy", "Per copy", false, OptionKind::Presentation);

/// The row headings a repeat declares: the one [`HEADING`].
pub const ROW_HEADINGS: &[&str] = &[HEADING.key];

const fn row(key: &'static str, label: &'static str, control: Control) -> InputDef {
    InputDef {
        key,
        label,
        ty: ListNumber,
        control,
    }
}

pub const OFFSET_X: InputDef = row(
    "copyX",
    "Offset X",
    Control::num(0.0, -10.0, 10.0, 0.01, "⬓"),
);
pub const OFFSET_Y: InputDef = row(
    "copyY",
    "Offset Y",
    Control::num(0.0, -10.0, 10.0, 0.01, "⬓"),
);
pub const OFFSET_Z: InputDef = row(
    "copyZ",
    "Offset Z",
    Control::num(0.0, -10.0, 10.0, 0.01, "⬓"),
);
pub const TURN: InputDef = row("copyTurn", "Turn", Control::turn(0.0, -1.0, 1.0, 0.001));
pub const SCALE_X: InputDef = row(
    "copyScaleX",
    "Scale X",
    Control::num(1.0, 0.0, 4.0, 0.01, "x"),
);
pub const SCALE_Y: InputDef = row(
    "copyScaleY",
    "Scale Y",
    Control::num(1.0, 0.0, 4.0, 0.01, "x"),
);
pub const SCALE_Z: InputDef = row(
    "copyScaleZ",
    "Scale Z",
    Control::num(1.0, 0.0, 4.0, 0.01, "x"),
);
pub const COLOR: InputDef = InputDef {
    key: "copyColor",
    label: "Color",
    ty: ListColor,
    control: Control::color("#ffffffff"),
};

/// A solid repeat's rows, in the order the node draws them.
pub const ROWS_3D: [InputDef; 8] = [
    OFFSET_X, OFFSET_Y, OFFSET_Z, TURN, SCALE_X, SCALE_Y, SCALE_Z, COLOR,
];

/// The rows that move, turn or size a copy.
const MOVING: [&str; 7] = [
    OFFSET_X.key,
    OFFSET_Y.key,
    OFFSET_Z.key,
    TURN.key,
    SCALE_X.key,
    SCALE_Y.key,
    SCALE_Z.key,
];

/// Whether an input folds under the Per copy heading.
pub fn is_row(key: &str) -> bool {
    ROWS_3D.iter().any(|r| r.key == key)
}

/// A count row: how many copies, a knob, or a list whose length is the count.
pub const fn count(key: &'static str, label: &'static str, default: f32, max: f32) -> InputDef {
    row(key, label, Control::num(default, 1.0, max, 1.0, ""))
}

/// The count on a count row, as an `f32` expression: a list's length where one is cabled in,
/// and otherwise the knob or the number cabled in.
pub fn count_of(ctx: &mut CompileContext, node: NodeId, key: &'static str) -> String {
    match ctx.list_input(node, key) {
        Some(stem) => format!("{stem}_len()"),
        None => ctx.input(node, key, "p"),
    }
}

/// Which item of a list a copy reads.
#[derive(Clone, Copy)]
pub enum Item<'a> {
    /// Copy `index` of `count`, both `f32` expressions: the list stretched over the copies.
    Counted { index: &'a str, count: &'a str },
    /// A copy of an endless repeat, `sum` the `f32` sum of its cell's coordinates: the list
    /// cycled, each row one item further along than the last.
    Endless { sum: &'a str },
}

/// One copy's rows as WGSL expressions: `offset` and `scale` a `vec3f`, `turn` an `f32` in
/// turns, `color` the `vec3f` its color is multiplied by.
pub struct Copy {
    pub offset: String,
    pub turn: String,
    pub scale: String,
    pub color: String,
}

/// Whether any row that moves, turns or sizes a copy has a list cabled in, so copies differ
/// in place or size and a repeat measures the near neighbor as well as the nearest.
pub fn moves_differ(ctx: &mut CompileContext, node: NodeId) -> bool {
    MOVING.iter().any(|key| ctx.list_input(node, key).is_some())
}

/// One row's value for the copy `item` names, `fallback` where its list is empty.
fn value(
    ctx: &mut CompileContext,
    node: NodeId,
    key: &'static str,
    item: Item<'_>,
    fallback: &str,
) -> String {
    let Some(stem) = ctx.list_input(node, key) else {
        return ctx.input(node, key, "p");
    };
    let at = match item {
        Item::Counted { index, count } => {
            format!("floor(({index}) * {stem}_len() / max({count}, 1.0))")
        }
        Item::Endless { sum } => format!("floor_mod({sum}, max({stem}_len(), 1.0))"),
    };
    format!("{stem}_at({at}, {fallback})")
}

/// Every row's value for one copy.
pub fn read(ctx: &mut CompileContext, node: NodeId, item: Item<'_>) -> Copy {
    let mut v = |key, fallback| value(ctx, node, key, item, fallback);
    let offset = format!(
        "vec3f({}, {}, {})",
        v(OFFSET_X.key, "0.0"),
        v(OFFSET_Y.key, "0.0"),
        v(OFFSET_Z.key, "0.0")
    );
    let turn = v(TURN.key, "0.0");
    let scale = format!(
        "max(vec3f({}, {}, {}), vec3f(1.0e-4))",
        v(SCALE_X.key, "1.0"),
        v(SCALE_Y.key, "1.0"),
        v(SCALE_Z.key, "1.0")
    );
    let color = format!("{}.rgb", ctx.input(node, COLOR.key, "p"));
    Copy {
        offset,
        turn,
        scale,
        color,
    }
}

/// The statements that take a copy's own point `q` into its rows' frame — moved by the
/// offset, turned about y, divided by the scale — as `m`, and the factor its distance is
/// multiplied back by as `k`.
pub fn place(copy: &Copy) -> String {
    format!(
        "        let sc = {scale};
        var m = q - {offset};
        let mt = sd_rot2(m.xz, -({turn}) * 6.2831853);
        m = vec3f(mt.x, m.y, mt.y) / sc;
        let k = min(sc.x, min(sc.y, sc.z));
",
        scale = copy.scale,
        offset = copy.offset,
        turn = copy.turn,
    )
}
