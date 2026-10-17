// SPDX-License-Identifier: AGPL-3.0-or-later

//! Numbers read out of a list: Reduce, one number from all of it, and Lookup, the list at a
//! fraction of the way across it.
//!
//! These are what a list dropped on a one-value input is converted by. The conversion menu's
//! Average, Highest, Lowest, Sum and Loudest Index rows each land a Reduce with its mode set
//! (`nodes::bridge`), since a list never lands on such an input by itself.

use super::{cpu, list_in, number_in, number_out, runtime_option};
use crate::graph::NodeId;
use crate::nodes::{Category, Control, CpuNode, NodeDef, TickContext};

pub static REDUCE: NodeDef = NodeDef {
    slug: "reduce",
    category: Category::List,
    icon: "∑",
    label: "Reduce",
    tooltip: "One number from a whole List: the Sum of its items, their Mean, the lowest, the \
              highest, or Argmax, the index of the highest — the loudest band of a spectrum. An \
              empty list sums to zero and has none of the others.",
    inputs: &[list_in("list", "List")],
    options: &[runtime_option(
        "mode",
        "Mode",
        "mean",
        &[
            ("sum", "Sum"),
            ("mean", "Mean"),
            ("min", "Min"),
            ("max", "Max"),
            ("argmax", "Argmax"),
        ],
    )],
    outputs: &[number_out("value", "Value")],
    cpu: Some(cpu(|| Box::new(Reduce))),
    ..NodeDef::EMPTY
};

/// One number out of `list` by `mode`: `sum`, `mean`, `min`, `max` or `argmax`, the index of
/// the first of the highest. `None` where an empty list has no such number.
pub fn reduce(list: &[f64], mode: &str) -> Option<f64> {
    if mode == "sum" {
        return Some(list.iter().sum());
    }
    if list.is_empty() {
        return None;
    }
    Some(match mode {
        "min" => list.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => list.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "argmax" => {
            let mut best = 0;
            for (i, v) in list.iter().enumerate() {
                if *v > list[best] {
                    best = i;
                }
            }
            best as f64
        }
        _ => list.iter().sum::<f64>() / list.len() as f64,
    })
}

struct Reduce;

impl CpuNode for Reduce {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        match reduce(&ctx.list(id, "list"), ctx.option(id, "mode")) {
            Some(v) => ctx.publish(id, "value", v),
            None => ctx.withdraw(id, "value"),
        }
    }
}

pub static LOOKUP: NodeDef = NodeDef {
    slug: "lookup",
    category: Category::List,
    icon: "🔍",
    label: "Lookup",
    tooltip: "The List read at Position, 0 the start and 1 the end coming round to the start \
              again, so a gear's Phase on Position steps through it once a cycle — a step \
              sequencer. Step holds each item for its share of the way; Linear glides from \
              each to the next, the last back to the first.",
    inputs: &[
        list_in("list", "List"),
        number_in(
            "position",
            "Position",
            Control::num(0.0, 0.0, 1.0, 0.001, ""),
        ),
    ],
    options: &[runtime_option(
        "mode",
        "Mode",
        "step",
        &[("step", "Step"), ("linear", "Linear")],
    )],
    outputs: &[number_out("value", "Value")],
    cpu: Some(cpu(|| Box::new(Lookup))),
    ..NodeDef::EMPTY
};

/// `list` at `position`, wrapped round `0..1`: the item whose share of the way it is in, or,
/// `linear`, a glide from that item to the next, the last to the first. `None` for an empty
/// list.
pub fn lookup(list: &[f64], position: f64, linear: bool) -> Option<f64> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    let position = if position.is_finite() {
        position.rem_euclid(1.0)
    } else {
        0.0
    };
    let at = position * n as f64;
    let i = (at.floor() as usize).min(n - 1);
    if !linear {
        return Some(list[i]);
    }
    let t = at - i as f64;
    Some(list[i] * (1.0 - t) + list[(i + 1) % n] * t)
}

struct Lookup;

impl CpuNode for Lookup {
    fn reset(&mut self) {}

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let linear = ctx.option(id, "mode") == "linear";
        match lookup(&ctx.list(id, "list"), ctx.input(id, "position"), linear) {
            Some(v) => ctx.publish(id, "value", v),
            None => ctx.withdraw(id, "value"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_reads_a_list_five_ways() {
        let list = [2.0, 8.0, -1.0, 8.0, 3.0];
        assert_eq!(reduce(&list, "sum"), Some(20.0));
        assert_eq!(reduce(&list, "mean"), Some(4.0));
        assert_eq!(reduce(&list, "min"), Some(-1.0));
        assert_eq!(reduce(&list, "max"), Some(8.0));
        assert_eq!(
            reduce(&list, "argmax"),
            Some(1.0),
            "the first of the highest"
        );
    }

    #[test]
    fn an_empty_list_sums_to_zero_and_has_nothing_else() {
        assert_eq!(reduce(&[], "sum"), Some(0.0));
        for mode in ["mean", "min", "max", "argmax"] {
            assert_eq!(reduce(&[], mode), None, "{mode}");
        }
    }

    #[test]
    fn lookup_steps_or_glides_round_the_list() {
        let list = [0.0, 10.0, 20.0, 30.0];
        assert_eq!(lookup(&list, 0.0, false), Some(0.0));
        assert_eq!(lookup(&list, 0.3, false), Some(10.0));
        assert_eq!(lookup(&list, 0.99, false), Some(30.0));
        assert_eq!(
            lookup(&list, 1.0, false),
            Some(0.0),
            "1 comes round to the start"
        );
        assert_eq!(lookup(&list, -0.25, false), Some(30.0));
        assert_eq!(lookup(&list, 0.125, true), Some(5.0));
        assert_eq!(
            lookup(&list, 0.875, true),
            Some(15.0),
            "the last glides to the first"
        );
        assert_eq!(lookup(&[], 0.5, true), None);
    }
}
