// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lists: any number of values a frame, held on the CPU, at most [`CAP`] items.
//!
//! **A list is a rate** (`graph::Rate::List`), beside the per-pixel field and the per-frame
//! uniform, so a number list is the number kind at a third rate and no new kind. A list's
//! length may change every frame and may be zero. Items are `f64`, as a number between nodes
//! is.
//!
//! **A list feeds a list and nothing else.** A one-value input never reads a quiet first
//! item: a list dropped there is offered a node that picks a value out of it — Get, Length or
//! Reduce — through the conversion menu (`nodes::bridge`). The dual maths takes one and runs
//! per item instead, list out. The design is [docs/nodes.md](../../docs/nodes.md#lists).
//!
//! **One matching rule, stretch.** Wherever a list of `n` items is read over `m`, item `i`
//! reads item `floor(i · n / m)` of it ([`stretch`]): elementwise maths as long as its longest
//! input, a shorter input stretched across it, and an empty one making the answer empty. A
//! single number is a list of one, so it reaches every item alike. Wrapping and averaging are
//! nodes of their own, Cycle and Resample.
//!
//! The nodes are in four files: this one holds the rules, Range, Get and Length; [`make`]
//! the lists made from numbers; [`reshape`] the lists made from lists; [`read`] the numbers
//! read out of one.

pub mod make;
pub mod read;
pub mod reshape;

pub use make::{CIRCLE_POINTS, GRID_POINTS, LIST, RANDOM};
pub use read::{LOOKUP, REDUCE};
pub use reshape::{CYCLE, JOIN, RESAMPLE, REVERSE, SLICE, SORT};

use crate::graph::NodeId;
use crate::graph::PortType::{ListNumber, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

/// The most items a list holds. A producer's items past it are dropped where they are
/// published, so every reader can count on it.
pub const CAP: usize = 1024;

/// Which item of a list `n` long item `i` of `m` reads: `floor(i · n / m)`, the one matching
/// rule. A list read over its own length reads itself; a short one is stretched, each item
/// held over a run of the longer one.
pub fn stretch(n: usize, i: usize, m: usize) -> usize {
    (i * n).checked_div(m).unwrap_or(0)
}

/// A number list input: no knob, an empty list where nothing is cabled in.
const fn list_in(key: &'static str, label: &'static str) -> InputDef {
    InputDef {
        key,
        label,
        ty: ListNumber,
        control: Control::None,
    }
}

/// A number list output, published by the node's tick.
const fn list_out(key: &'static str, label: &'static str) -> OutputDef {
    OutputDef {
        key,
        label,
        ty: ListNumber,
        kind: OutputKind::List,
        ..OutputDef::EMPTY
    }
}

/// A uniform number input with a knob.
const fn number_in(key: &'static str, label: &'static str, control: Control) -> InputDef {
    InputDef {
        key,
        label,
        ty: UniformNumber,
        control,
    }
}

/// A count of items: a whole number from none to [`CAP`].
const fn count_in(key: &'static str, label: &'static str, default: f32) -> InputDef {
    number_in(key, label, Control::num(default, 0.0, CAP as f32, 1.0, ""))
}

/// A uniform number output, published by the node's tick.
const fn number_out(key: &'static str, label: &'static str) -> OutputDef {
    OutputDef {
        key,
        label,
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        ..OutputDef::EMPTY
    }
}

/// An option a tick alone reads.
const fn runtime_option(
    key: &'static str,
    label: &'static str,
    default: &'static str,
    choices: &'static [(&'static str, &'static str)],
) -> OptionDef {
    OptionDef {
        key,
        label,
        default,
        choices,
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }
}

const fn cpu(create: fn() -> Box<dyn CpuNode>) -> CpuDef {
    CpuDef {
        create,
        integrates: false,
        live: false,
    }
}

/// A count as a number of items: rounded and held to `0..=CAP`, none for a NaN.
pub fn whole(count: f64) -> usize {
    if count.is_finite() {
        count.round().clamp(0.0, CAP as f64) as usize
    } else {
        0
    }
}

pub static RANGE: NodeDef = NodeDef {
    slug: "range",
    category: Category::List,
    icon: "📶",
    label: "Range",
    tooltip: "Count numbers evenly spaced From one To the other. Ends set to From Only leaves \
              the last one out, so a Range of turns closes a ring without two copies on one spot.",
    inputs: &[
        count_in("count", "Count", 8.0),
        number_in("from", "From", Control::num(0.0, -100.0, 100.0, 0.01, "")),
        number_in("to", "To", Control::num(1.0, -100.0, 100.0, 0.01, "")),
    ],
    options: &[runtime_option(
        "ends",
        "Ends",
        "both",
        &[("both", "Both"), ("from", "From Only")],
    )],
    outputs: &[list_out("list", "List")],
    cpu: Some(cpu(|| Box::new(Range))),
    ..NodeDef::EMPTY
};

/// `count` evenly spaced values from `from` to `to`, the last one `to` itself unless `closed`
/// leaves it out. A count is rounded and held to `0..=CAP`.
pub fn range(count: f64, from: f64, to: f64, both: bool) -> impl Iterator<Item = f64> {
    let n = whole(count);
    let steps = if both {
        n.saturating_sub(1).max(1)
    } else {
        n.max(1)
    };
    let step = (to - from) / steps as f64;
    (0..n).map(move |i| from + step * i as f64)
}

struct Range;

impl CpuNode for Range {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let both = ctx.option(id, "ends") != "from";
        let items = range(
            ctx.input(id, "count"),
            ctx.input(id, "from"),
            ctx.input(id, "to"),
            both,
        );
        ctx.publish_list(id, "list", items);
    }
}

pub static GET: NodeDef = NodeDef {
    slug: "get",
    category: Category::List,
    icon: "👉",
    label: "Get",
    tooltip: "One item of a List. Index 0 is the first; an Index past the end wraps round to \
              the start. A list of indices cabled into Index gathers the items they name, in \
              their order, so Sort's Order reorders a sibling list. An empty list gives nothing.",
    inputs: &[
        list_in("list", "List"),
        number_in(
            "index",
            "Index",
            Control::num(0.0, 0.0, (CAP - 1) as f32, 1.0, ""),
        ),
    ],
    outputs: &[OutputDef {
        key: "item",
        label: "Item",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        gathers: true,
        ..OutputDef::EMPTY
    }],
    cpu: Some(cpu(|| Box::new(Get))),
    ..NodeDef::EMPTY
};

/// The item at `index` of `list`, the index rounded and wrapped round its length; `None` for an
/// empty list.
pub fn get(list: &[f64], index: f64) -> Option<f64> {
    if list.is_empty() {
        return None;
    }
    let index = if index.is_finite() {
        index.round()
    } else {
        0.0
    };
    let at = (index as i64).rem_euclid(list.len() as i64) as usize;
    list.get(at).copied()
}

struct Get;

impl CpuNode for Get {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // A list on Index makes the graph give Item the list rate: one item per index.
        let gathers = ctx
            .node(id)
            .and_then(|n| n.output("item"))
            .is_some_and(|p| p.ty.is_list());
        if gathers {
            let list = ctx.list(id, "list");
            let items: Vec<f64> = ctx
                .list(id, "index")
                .iter()
                .filter_map(|i| get(&list, *i))
                .collect();
            ctx.publish_list(id, "item", items);
            ctx.withdraw(id, "item");
            return;
        }
        match get(&ctx.list(id, "list"), ctx.input(id, "index")) {
            Some(item) => ctx.publish(id, "item", item),
            None => ctx.withdraw(id, "item"),
        }
        ctx.withdraw_list(id, "item");
    }
}

pub static LENGTH: NodeDef = NodeDef {
    slug: "length",
    category: Category::List,
    icon: "📏",
    label: "Length",
    tooltip: "How many items a List holds.",
    inputs: &[list_in("list", "List")],
    outputs: &[OutputDef {
        key: "count",
        label: "Count",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
        integral: true,
        ..OutputDef::EMPTY
    }],
    cpu: Some(cpu(|| Box::new(Length))),
    ..NodeDef::EMPTY
};

struct Length;

impl CpuNode for Length {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let n = ctx.list(id, "list").len();
        ctx.publish(id, "count", n as f64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_spans_from_to_to_with_both_ends() {
        let items: Vec<f64> = range(5.0, 0.0, 1.0, true).collect();
        assert_eq!(items, [0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn a_range_from_only_leaves_the_last_out() {
        let items: Vec<f64> = range(4.0, 0.0, 1.0, false).collect();
        assert_eq!(items, [0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn a_range_of_one_is_from_and_of_none_is_empty() {
        assert_eq!(range(1.0, 3.0, 9.0, true).collect::<Vec<_>>(), [3.0]);
        assert_eq!(range(0.0, 3.0, 9.0, true).count(), 0);
        assert_eq!(range(-4.0, 3.0, 9.0, true).count(), 0);
        assert_eq!(range(f64::NAN, 3.0, 9.0, true).count(), 0);
    }

    #[test]
    fn a_range_stops_at_the_cap() {
        assert_eq!(range(1.0e9, 0.0, 1.0, true).count(), CAP);
    }

    #[test]
    fn get_wraps_its_index_and_gives_nothing_from_an_empty_list() {
        let list = [10.0, 20.0, 30.0];
        assert_eq!(get(&list, 0.0), Some(10.0));
        assert_eq!(get(&list, 2.0), Some(30.0));
        assert_eq!(get(&list, 3.0), Some(10.0));
        assert_eq!(get(&list, -1.0), Some(30.0));
        assert_eq!(get(&list, 1.4), Some(20.0));
        assert_eq!(get(&[], 0.0), None);
    }

    /// Three items over twelve is AAAA BBBB CCCC, and a list over its own length is itself.
    #[test]
    fn stretch_holds_each_item_over_a_run_of_the_longer_list() {
        let read: Vec<usize> = (0..12).map(|i| stretch(3, i, 12)).collect();
        assert_eq!(read, [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2]);
        assert!((0..7).all(|i| stretch(7, i, 7) == i));
        assert_eq!(stretch(1, 5, 9), 0);
        // A long list over a few skips: sixteen over four reads every fourth.
        let read: Vec<usize> = (0..4).map(|i| stretch(16, i, 4)).collect();
        assert_eq!(read, [0, 4, 8, 12]);
    }
}
