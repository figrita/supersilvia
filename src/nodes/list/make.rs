// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lists made out of numbers: Random, Circle Points, Grid Points, and List, whose numbers a
//! hand draws on the node itself.
//!
//! **Every one is deterministic.** Random hashes the seed with the index rather than drawing
//! from a generator, so item `i` is the same number whatever Count is and a bigger Count keeps
//! the items already there. A point is two parallel lists, X and Y, never a pair: every
//! position in the library is two number ports, so the lists cable straight into them.

use super::{CAP, count_in, cpu, list_out, number_in, whole};
use crate::graph::{NodeId, Value};
use crate::nodes::{
    Category, Control, CpuNode, NodeDef, Region, TURNS, TickContext, ValueDef, ValueKind,
};

pub static RANDOM: NodeDef = NodeDef {
    slug: "randomlist",
    category: Category::List,
    icon: "🎲",
    label: "Random",
    tooltip: "Count numbers between Min and Max, each chosen by Seed and its own place in the \
              list. The same seed gives the same list every time, and a bigger Count keeps the \
              numbers already there.",
    inputs: &[
        count_in("count", "Count", 8.0),
        number_in("min", "Min", Control::num(0.0, -100.0, 100.0, 0.01, "")),
        number_in("max", "Max", Control::num(1.0, -100.0, 100.0, 0.01, "")),
        number_in("seed", "Seed", Control::num(0.0, 0.0, 1000.0, 1.0, "")),
    ],
    outputs: &[list_out("list", "List")],
    cpu: Some(cpu(|| Box::new(Random))),
    ..NodeDef::EMPTY
};

/// A number in `0..1` that depends on `seed` and `index` alone: SplitMix64's finalizer over
/// the seed's bits and the index, the top 53 bits as a fraction.
pub fn hash(seed: f64, index: usize) -> f64 {
    // Zero is one seed, whichever sign it has.
    let seed = if seed == 0.0 { 0.0 } else { seed };
    let mut z = seed
        .to_bits()
        .wrapping_add((index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1_u64 << 53) as f64
}

/// `count` numbers between `min` and `max`, item `i` by [`hash`] of `seed` and `i`.
pub fn random(count: f64, min: f64, max: f64, seed: f64) -> impl Iterator<Item = f64> {
    (0..whole(count)).map(move |i| min + hash(seed, i) * (max - min))
}

struct Random;

impl CpuNode for Random {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let items = random(
            ctx.input(id, "count"),
            ctx.input(id, "min"),
            ctx.input(id, "max"),
            ctx.input(id, "seed"),
        );
        ctx.publish_list(id, "list", items);
    }
}

pub static CIRCLE_POINTS: NodeDef = NodeDef {
    slug: "circlepoints",
    category: Category::List,
    icon: "⭕",
    label: "Circle Points",
    tooltip: "Count points round a circle, as an X list, a Y list and the Angle of each in \
              turns. Start turns the first point round; Spread is how much of a turn they \
              cover. A whole turn spaces them evenly with no point on top of the first, and \
              less than one runs from Start to the end of the arc.",
    inputs: &[
        count_in("count", "Count", 8.0),
        number_in("radius", "Radius", Control::num(0.5, 0.0, 4.0, 0.01, "")),
        number_in(
            "centerX",
            "Center X",
            Control::num(0.0, -2.0, 2.0, 0.01, ""),
        ),
        number_in(
            "centerY",
            "Center Y",
            Control::num(0.0, -2.0, 2.0, 0.01, ""),
        ),
        number_in("start", "Start", Control::num(0.0, -1.0, 1.0, 0.001, TURNS)),
        number_in(
            "spread",
            "Spread",
            Control::num(1.0, -1.0, 1.0, 0.001, TURNS),
        ),
    ],
    outputs: &[
        list_out("x", "X"),
        list_out("y", "Y"),
        list_out("angle", "Angle"),
    ],
    cpu: Some(cpu(|| Box::new(CirclePoints))),
    ..NodeDef::EMPTY
};

/// The angle of each of `count` points, in turns: from `start` across `spread`, a whole turn
/// or more divided into `count` so the last point does not land on the first, and an arc into
/// `count - 1` so it runs from end to end.
pub fn circle_angles(count: f64, start: f64, spread: f64) -> impl Iterator<Item = f64> {
    let n = whole(count);
    let gaps = if spread.abs() >= 1.0 {
        n
    } else {
        n.saturating_sub(1)
    }
    .max(1);
    let step = spread / gaps as f64;
    (0..n).map(move |i| start + step * i as f64)
}

struct CirclePoints;

impl CpuNode for CirclePoints {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let radius = ctx.input(id, "radius");
        let (cx, cy) = (ctx.input(id, "centerX"), ctx.input(id, "centerY"));
        let angles: Vec<f64> = circle_angles(
            ctx.input(id, "count"),
            ctx.input(id, "start"),
            ctx.input(id, "spread"),
        )
        .collect();
        let at = |a: f64| a * std::f64::consts::TAU;
        ctx.publish_list(id, "x", angles.iter().map(|a| cx + radius * at(*a).cos()));
        ctx.publish_list(id, "y", angles.iter().map(|a| cy + radius * at(*a).sin()));
        ctx.publish_list(id, "angle", angles);
    }
}

pub static GRID_POINTS: NodeDef = NodeDef {
    slug: "gridpoints",
    category: Category::List,
    icon: "🔳",
    label: "Grid Points",
    tooltip: "A lattice of Columns by Rows points over a Width by Height rectangle round the \
              center, row after row from the bottom left: the middle of each cell as an X list \
              and a Y list, with the Column and Row each point is in.",
    inputs: &[
        number_in("columns", "Columns", Control::num(4.0, 0.0, 32.0, 1.0, "")),
        number_in("rows", "Rows", Control::num(4.0, 0.0, 32.0, 1.0, "")),
        number_in("width", "Width", Control::num(1.0, 0.0, 4.0, 0.01, "")),
        number_in("height", "Height", Control::num(1.0, 0.0, 4.0, 0.01, "")),
        number_in(
            "centerX",
            "Center X",
            Control::num(0.0, -2.0, 2.0, 0.01, ""),
        ),
        number_in(
            "centerY",
            "Center Y",
            Control::num(0.0, -2.0, 2.0, 0.01, ""),
        ),
    ],
    outputs: &[
        list_out("x", "X"),
        list_out("y", "Y"),
        list_out("column", "Column"),
        list_out("row", "Row"),
    ],
    cpu: Some(cpu(|| Box::new(GridPoints))),
    ..NodeDef::EMPTY
};

/// Each point of a `columns` by `rows` lattice in the order the lists hold them, row after
/// row: its column and its row. No more than [`CAP`] of them.
pub fn grid(columns: f64, rows: f64) -> impl Iterator<Item = (usize, usize)> {
    let (columns, rows) = (whole(columns), whole(rows));
    (0..rows)
        .flat_map(move |r| (0..columns).map(move |c| (c, r)))
        .take(CAP)
}

/// Where the middle of cell `i` of `n` is across a span `size` wide round `center`.
pub fn cell_middle(i: usize, n: usize, size: f64, center: f64) -> f64 {
    center + ((i as f64 + 0.5) / n.max(1) as f64 - 0.5) * size
}

struct GridPoints;

impl CpuNode for GridPoints {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let (columns, rows) = (ctx.input(id, "columns"), ctx.input(id, "rows"));
        let (w, h) = (ctx.input(id, "width"), ctx.input(id, "height"));
        let (cx, cy) = (ctx.input(id, "centerX"), ctx.input(id, "centerY"));
        let (nc, nr) = (whole(columns), whole(rows));
        let cells: Vec<(usize, usize)> = grid(columns, rows).collect();
        ctx.publish_list(
            id,
            "x",
            cells.iter().map(|(c, _)| cell_middle(*c, nc, w, cx)),
        );
        ctx.publish_list(
            id,
            "y",
            cells.iter().map(|(_, r)| cell_middle(*r, nr, h, cy)),
        );
        ctx.publish_list(id, "column", cells.iter().map(|(c, _)| *c as f64));
        ctx.publish_list(id, "row", cells.iter().map(|(_, r)| *r as f64));
    }
}

/// What the hand-made numbers are stored under, which is also what the region that draws
/// them finds.
pub const VALUES: &str = "values";

/// The most bars the List node's own editor draws: a Count past it still publishes, with
/// the items past the stored ones at Min.
pub const BARS: usize = 64;

pub static LIST: NodeDef = NodeDef {
    slug: "list",
    category: Category::List,
    icon: "📊",
    label: "List",
    tooltip: "Numbers drawn by hand: drag a bar up or down to set it, or across several to \
              draw them in one stroke. Count is how many there are, and each bar's height is \
              its share of the way from Min to Max.",
    inputs: &[
        number_in(
            "count",
            "Count",
            Control::num(8.0, 0.0, BARS as f32, 1.0, ""),
        ),
        number_in("min", "Min", Control::num(0.0, -100.0, 100.0, 0.01, "")),
        number_in("max", "Max", Control::num(1.0, -100.0, 100.0, 0.01, "")),
    ],
    values: &[ValueDef {
        key: VALUES,
        label: "",
        kind: ValueKind::Numbers,
    }],
    regions: &[Region::Bars],
    outputs: &[list_out("list", "List")],
    cpu: Some(cpu(|| Box::new(List))),
    ..NodeDef::EMPTY
};

/// The share of the way from Min to Max bar `i` stands at: what the node's value holds, `0`
/// to `1`, and none for a bar past the stored ones.
pub fn bar(value: Option<&Value>, i: usize) -> f64 {
    value
        .and_then(Value::numbers)
        .and_then(|bars| bars.get(i))
        .map_or(0.0, |v| f64::from(v.clamp(0.0, 1.0)))
}

/// The bars as they are after a hand set bar `at` to `share`: the rest kept, any between the
/// stored ones and `at` filled at none.
pub fn set_bar(value: Option<&Value>, at: usize, share: f32) -> Value {
    let mut bars = value
        .and_then(Value::numbers)
        .map(<[f32]>::to_vec)
        .unwrap_or_default();
    if bars.len() <= at {
        bars.resize(at + 1, 0.0);
    }
    bars[at] = share.clamp(0.0, 1.0);
    Value::Numbers(bars)
}

struct List;

impl CpuNode for List {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let (min, max) = (ctx.input(id, "min"), ctx.input(id, "max"));
        let n = whole(ctx.input(id, "count"));
        let items: Vec<f64> = {
            let value = ctx.node(id).and_then(|node| node.values.get(VALUES));
            (0..n).map(|i| min + bar(value, i) * (max - min)).collect()
        };
        ctx.publish_list(id, "list", items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_random_item_depends_on_its_seed_and_place_alone() {
        let short: Vec<f64> = random(4.0, 0.0, 1.0, 7.0).collect();
        let long: Vec<f64> = random(9.0, 0.0, 1.0, 7.0).collect();
        assert_eq!(
            short,
            long[..4],
            "a bigger Count keeps the items already there"
        );
        assert_ne!(short, random(4.0, 0.0, 1.0, 8.0).collect::<Vec<_>>());
        assert!(long.iter().all(|v| (0.0..1.0).contains(v)));
        let spread: Vec<f64> = random(64.0, -2.0, 2.0, 0.0).collect();
        assert!(spread.iter().all(|v| (-2.0..2.0).contains(v)));
        assert!(spread.iter().any(|v| *v < -1.0) && spread.iter().any(|v| *v > 1.0));
        assert_eq!(hash(0.0, 3), hash(-0.0, 3));
    }

    #[test]
    fn a_whole_turn_of_circle_points_never_lands_on_the_first() {
        let angles: Vec<f64> = circle_angles(4.0, 0.0, 1.0).collect();
        assert_eq!(angles, [0.0, 0.25, 0.5, 0.75]);
        let angles: Vec<f64> = circle_angles(3.0, 0.25, 0.5).collect();
        assert_eq!(angles, [0.25, 0.5, 0.75], "an arc runs end to end");
        assert_eq!(circle_angles(1.0, 0.1, 0.5).collect::<Vec<_>>(), [0.1]);
        assert_eq!(circle_angles(0.0, 0.0, 1.0).count(), 0);
    }

    #[test]
    fn grid_points_run_row_after_row_through_the_middle_of_each_cell() {
        let cells: Vec<(usize, usize)> = grid(3.0, 2.0).collect();
        assert_eq!(cells, [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]);
        let xs: Vec<f64> = (0..4).map(|c| cell_middle(c, 4, 1.0, 0.0)).collect();
        assert_eq!(xs, [-0.375, -0.125, 0.125, 0.375]);
        assert_eq!(cell_middle(0, 1, 2.0, 0.5), 0.5);
        assert_eq!(grid(64.0, 64.0).count(), CAP);
    }

    #[test]
    fn a_bar_is_set_alone_and_a_missing_one_is_none() {
        let set = set_bar(None, 2, 0.5);
        assert_eq!(set.numbers(), Some(&[0.0, 0.0, 0.5][..]));
        let again = set_bar(Some(&set), 0, 2.0);
        assert_eq!(again.numbers(), Some(&[1.0, 0.0, 0.5][..]));
        assert_eq!(bar(Some(&again), 2), 0.5);
        assert_eq!(bar(Some(&again), 7), 0.0);
        assert_eq!(bar(None, 0), 0.0);
    }
}
