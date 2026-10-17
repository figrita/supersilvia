// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lists: any number of values a frame, held on the CPU, at most [`CAP`] items.
//!
//! **A list is a rate** (`graph::Rate::List`), beside the per-pixel field and the per-frame
//! uniform, so a number list is the number kind at a third rate and no new kind. A list's
//! length may change every frame and may be zero. Items are `f64`, as a number between nodes
//! is.
//!
//! **A list feeds a list and nothing else.** A one-value input never reads a quiet first
//! item: a list dropped there is offered a node that picks a value out of it — Get or Length —
//! through the conversion menu (`nodes::bridge`). The design is
//! [docs/nodes.md](../../docs/nodes.md#lists).

use crate::graph::NodeId;
use crate::graph::PortType::{ListNumber, UniformNumber};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, InputDef, NodeDef, OptionDef, OptionKind, OutputDef,
    OutputKind, TickContext,
};

/// The most items a list holds. A producer's items past it are dropped where they are
/// published, so every reader can count on it.
pub const CAP: usize = 1024;

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

const fn cpu(create: fn() -> Box<dyn CpuNode>) -> CpuDef {
    CpuDef {
        create,
        integrates: false,
        live: false,
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
        InputDef {
            key: "count",
            label: "Count",
            ty: UniformNumber,
            control: Control::num(8.0, 0.0, CAP as f32, 1.0, ""),
        },
        InputDef {
            key: "from",
            label: "From",
            ty: UniformNumber,
            control: Control::num(0.0, -100.0, 100.0, 0.01, ""),
        },
        InputDef {
            key: "to",
            label: "To",
            ty: UniformNumber,
            control: Control::num(1.0, -100.0, 100.0, 0.01, ""),
        },
    ],
    options: &[OptionDef {
        key: "ends",
        label: "Ends",
        default: "both",
        choices: &[("both", "Both"), ("from", "From Only")],
        kind: OptionKind::Runtime,
        ..OptionDef::EMPTY
    }],
    outputs: &[list_out("list", "List")],
    cpu: Some(cpu(|| Box::new(Range))),
    ..NodeDef::EMPTY
};

/// `count` evenly spaced values from `from` to `to`, the last one `to` itself unless `closed`
/// leaves it out. A count is rounded and held to `0..=CAP`.
pub fn range(count: f64, from: f64, to: f64, both: bool) -> impl Iterator<Item = f64> {
    let n = if count.is_finite() {
        count.round().clamp(0.0, CAP as f64) as usize
    } else {
        0
    };
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
              the start. An empty list gives nothing.",
    inputs: &[
        list_in("list", "List"),
        InputDef {
            key: "index",
            label: "Index",
            ty: UniformNumber,
            control: Control::num(0.0, 0.0, (CAP - 1) as f32, 1.0, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "item",
        label: "Item",
        ty: UniformNumber,
        kind: OutputKind::Uniform,
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
        match get(&ctx.list(id, "list"), ctx.input(id, "index")) {
            Some(item) => ctx.publish(id, "item", item),
            None => ctx.withdraw(id, "item"),
        }
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
}
