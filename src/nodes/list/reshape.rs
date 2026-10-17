// SPDX-License-Identifier: AGPL-3.0-or-later

//! Lists made out of lists: Resample, Slice, Join, Reverse, Sort and Cycle.
//!
//! **Reordering is explicit and shared.** Sort publishes the sorted list and its **Order**,
//! the source indices in sorted order, and Get with that Order on its Index gathers any sibling
//! list into the same order — one Order reorders X, Y and a size alike, which is what keeps
//! parallel lists parallel. **Stretching is the rule and the rest are nodes**: Resample is a
//! stretch made explicit, with a linear and an averaging reading beside it, and Cycle repeats
//! a list round to a count where stretch would hold each item over a run.

use super::{count_in, cpu, list_in, list_out, number_in, runtime_option, stretch, whole};
use crate::graph::NodeId;
use crate::nodes::{Category, Control, CpuNode, NodeDef, TickContext};

pub static RESAMPLE: NodeDef = NodeDef {
    slug: "resample",
    category: Category::List,
    icon: "📐",
    label: "Resample",
    tooltip: "The List read over Count items. Nearest holds each item over a run, as every \
              list is stretched; Linear glides between neighbours, first to last; Average gives \
              each item the mean of the items under it, so a long list over a few keeps every \
              one of them.",
    inputs: &[list_in("list", "List"), count_in("count", "Count", 16.0)],
    options: &[runtime_option(
        "mode",
        "Mode",
        "nearest",
        &[
            ("nearest", "Nearest"),
            ("linear", "Linear"),
            ("average", "Average"),
        ],
    )],
    outputs: &[list_out("output", "List")],
    cpu: Some(cpu(|| Box::new(Resample))),
    ..NodeDef::EMPTY
};

/// How Resample reads a list over another length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Nearest,
    Linear,
    Average,
}

/// `list` read over `count` items. An empty list resamples to nothing.
pub fn resample(list: &[f64], count: usize, reading: Reading) -> Vec<f64> {
    let n = list.len();
    if n == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|i| match reading {
            Reading::Nearest => list[stretch(n, i, count)],
            Reading::Linear => {
                // First onto first and last onto last.
                let at = if count > 1 {
                    i as f64 * (n - 1) as f64 / (count - 1) as f64
                } else {
                    0.0
                };
                let lo = (at.floor() as usize).min(n - 1);
                let hi = (lo + 1).min(n - 1);
                let t = at - lo as f64;
                list[lo] * (1.0 - t) + list[hi] * t
            }
            Reading::Average => {
                let start = stretch(n, i, count);
                let end = stretch(n, i + 1, count).clamp(start + 1, n);
                let under = &list[start..end];
                under.iter().sum::<f64>() / under.len() as f64
            }
        })
        .collect()
}

struct Resample;

impl CpuNode for Resample {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let reading = match ctx.option(id, "mode") {
            "linear" => Reading::Linear,
            "average" => Reading::Average,
            _ => Reading::Nearest,
        };
        let items = resample(
            &ctx.list(id, "list"),
            whole(ctx.input(id, "count")),
            reading,
        );
        ctx.publish_list(id, "output", items);
    }
}

pub static SLICE: NodeDef = NodeDef {
    slug: "slice",
    category: Category::List,
    icon: "🔪",
    label: "Slice",
    tooltip: "Count items of the List from Start on, 0 being the first. A slice that runs off \
              the end stops there.",
    inputs: &[
        list_in("list", "List"),
        number_in(
            "start",
            "Start",
            Control::num(0.0, 0.0, (super::CAP - 1) as f32, 1.0, ""),
        ),
        count_in("count", "Count", 4.0),
    ],
    outputs: &[list_out("output", "List")],
    cpu: Some(cpu(|| Box::new(Slice))),
    ..NodeDef::EMPTY
};

/// `count` items of `list` from `start` on, as many as there are.
pub fn slice(list: &[f64], start: f64, count: f64) -> &[f64] {
    let start = whole(start).min(list.len());
    let end = start.saturating_add(whole(count)).min(list.len());
    &list[start..end]
}

struct Slice;

impl CpuNode for Slice {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let list = ctx.list(id, "list");
        let items = slice(&list, ctx.input(id, "start"), ctx.input(id, "count")).to_vec();
        ctx.publish_list(id, "output", items);
    }
}

pub static JOIN: NodeDef = NodeDef {
    slug: "join",
    category: Category::List,
    icon: "🔗",
    label: "Join",
    tooltip: "List A, then List B after it.",
    inputs: &[list_in("a", "A"), list_in("b", "B")],
    outputs: &[list_out("output", "List")],
    cpu: Some(cpu(|| Box::new(Join))),
    ..NodeDef::EMPTY
};

struct Join;

impl CpuNode for Join {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let items: Vec<f64> = ctx
            .list(id, "a")
            .iter()
            .chain(ctx.list(id, "b").iter())
            .copied()
            .collect();
        ctx.publish_list(id, "output", items);
    }
}

pub static REVERSE: NodeDef = NodeDef {
    slug: "reverse",
    category: Category::List,
    icon: "🔃",
    label: "Reverse",
    tooltip: "The List, last item first.",
    inputs: &[list_in("list", "List")],
    outputs: &[list_out("output", "List")],
    cpu: Some(cpu(|| Box::new(Reverse))),
    ..NodeDef::EMPTY
};

struct Reverse;

impl CpuNode for Reverse {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let items: Vec<f64> = ctx.list(id, "list").iter().rev().copied().collect();
        ctx.publish_list(id, "output", items);
    }
}

pub static SORT: NodeDef = NodeDef {
    slug: "sort",
    category: Category::List,
    icon: "🔢",
    label: "Sort",
    tooltip: "The List from lowest to highest, and its Order: where each sorted item was in \
              the List. Cable Order into a Get's Index to put another list in the same order. \
              Equal items keep the order they came in.",
    inputs: &[list_in("list", "List")],
    outputs: &[list_out("sorted", "Sorted"), list_out("order", "Order")],
    cpu: Some(cpu(|| Box::new(Sort))),
    ..NodeDef::EMPTY
};

/// The indices of `list` in the order that sorts it, lowest first, equal items in the order
/// they came.
pub fn order(list: &[f64]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..list.len()).collect();
    order.sort_by(|a, b| list[*a].total_cmp(&list[*b]));
    order
}

struct Sort;

impl CpuNode for Sort {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let list = ctx.list(id, "list").into_owned();
        let order = order(&list);
        ctx.publish_list(id, "sorted", order.iter().map(|i| list[*i]));
        ctx.publish_list(id, "order", order.iter().map(|i| *i as f64));
    }
}

pub static CYCLE: NodeDef = NodeDef {
    slug: "cycle",
    category: Category::List,
    icon: "🔁",
    label: "Cycle",
    tooltip: "The List repeated round to Count items, moved Shift places on, the end coming \
              round to the start: three colors striped over twelve copies, or with a Counter on \
              Shift, a chase.",
    inputs: &[
        list_in("list", "List"),
        count_in("count", "Count", 16.0),
        number_in("shift", "Shift", Control::num(0.0, -64.0, 64.0, 1.0, "")),
    ],
    outputs: &[list_out("output", "List")],
    cpu: Some(cpu(|| Box::new(Cycle))),
    ..NodeDef::EMPTY
};

/// `list` repeated round to `count` items, item `i` the list's item `i - shift` round its
/// length, so a larger Shift moves the pattern towards the end. Nothing from an empty list.
pub fn cycle(list: &[f64], count: usize, shift: f64) -> impl Iterator<Item = f64> + '_ {
    let n = list.len() as i64;
    let shift = if shift.is_finite() {
        shift.round() as i64
    } else {
        0
    };
    let count = if n == 0 { 0 } else { count };
    (0..count).map(move |i| list[(i as i64 - shift).rem_euclid(n) as usize])
}

struct Cycle;

impl CpuNode for Cycle {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let list = ctx.list(id, "list").into_owned();
        let items = cycle(&list, whole(ctx.input(id, "count")), ctx.input(id, "shift"));
        ctx.publish_list(id, "output", items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_stretches_glides_or_averages() {
        let list = [0.0, 10.0, 20.0, 30.0];
        assert_eq!(
            resample(&list, 8, Reading::Nearest),
            [0.0, 0.0, 10.0, 10.0, 20.0, 20.0, 30.0, 30.0]
        );
        assert_eq!(
            resample(&list, 7, Reading::Linear),
            [0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0]
        );
        assert_eq!(resample(&list, 2, Reading::Average), [5.0, 25.0]);
        assert_eq!(resample(&list, 2, Reading::Nearest), [0.0, 20.0]);
        assert_eq!(resample(&list, 1, Reading::Linear), [0.0]);
        assert_eq!(resample(&[4.0], 3, Reading::Linear), [4.0, 4.0, 4.0]);
        assert!(resample(&[], 5, Reading::Average).is_empty());
        assert!(resample(&list, 0, Reading::Average).is_empty());
        // Up is a stretch, whichever way it is read.
        assert_eq!(
            resample(&[1.0, 2.0], 4, Reading::Average),
            [1.0, 1.0, 2.0, 2.0]
        );
    }

    #[test]
    fn a_slice_stops_at_the_end() {
        let list = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(slice(&list, 1.0, 2.0), [2.0, 3.0]);
        assert_eq!(slice(&list, 3.0, 9.0), [4.0, 5.0]);
        assert!(slice(&list, 7.0, 2.0).is_empty());
        assert_eq!(slice(&list, -2.0, 1.0), [1.0]);
    }

    #[test]
    fn sort_keeps_equal_items_in_order_and_says_where_each_came_from() {
        let list = [3.0, 1.0, 2.0, 1.0];
        let order = order(&list);
        assert_eq!(order, [1, 3, 2, 0]);
        let sorted: Vec<f64> = order.iter().map(|i| list[*i]).collect();
        assert_eq!(sorted, [1.0, 1.0, 2.0, 3.0]);
        assert!(super::order(&[]).is_empty());
    }

    #[test]
    fn cycle_repeats_round_and_shifts_towards_the_end() {
        let list = [1.0, 2.0, 3.0];
        assert_eq!(
            cycle(&list, 7, 0.0).collect::<Vec<_>>(),
            [1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 1.0]
        );
        assert_eq!(
            cycle(&list, 4, 1.0).collect::<Vec<_>>(),
            [3.0, 1.0, 2.0, 3.0]
        );
        assert_eq!(cycle(&list, 3, -1.0).collect::<Vec<_>>(), [2.0, 3.0, 1.0]);
        assert_eq!(cycle(&[], 4, 0.0).count(), 0);
    }
}
