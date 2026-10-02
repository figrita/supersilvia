// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a node can go on a cable: the kinds a loose cable could land on, and the ports a
//! node dropped on a cable would carry it through.
//!
//! Two gestures ask. A cable let go over empty canvas opens the node browser filtered to
//! [`takers`], and the kind chosen lands wired (`Command::AddConnected`). A node dragged
//! across a cable lights it where [`splice_ports`] answers, and the drop puts the node into
//! it (`Command::Splice`). See docs/ui.md, "Cables".
//!
//! Both are answered by the graph's own rule rather than a second one: the question is put
//! to `Graph::can_connect` on a copy that holds the node, so a dual node's effective types,
//! a pin and a loop are judged exactly as the cable itself would be. The copy shares every
//! node it does not write, so asking costs a node per kind and nothing per node already
//! there.

use crate::graph::{Graph, NodeId, PortRef, WorkspaceId};
use crate::nodes::{REGISTRY, TIME, phasor};
use emath::Pos2;

/// One kind a loose cable could land on, and the port on it the cable would take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Taker {
    pub slug: &'static str,
    /// The new node's input where `end` is an output, and its output where `end` is an input.
    pub key: &'static str,
}

/// Every kind this machine offers that a cable from `end` could land on, in registry order,
/// each with the first of its ports that takes it.
///
/// A fresh node of each kind is put on a copy of `graph` and asked [`Graph::can_connect`]:
/// a fresh node has no cables, so what can refuse it is the cable's own type against the
/// port's, and an action against data. **Time and Offset come last**, because a number let go
/// in the open means a parameter more often than a clock, and a node that moves with time
/// lists those two first.
pub fn takers(graph: &Graph, end: PortRef, workspace: WorkspaceId) -> Vec<Taker> {
    let Some(node) = graph.get(end.node) else {
        return Vec::new();
    };
    let from_output = node.output(end.key).is_some();
    let mut scratch = graph.clone();
    let mut out = Vec::new();
    for def in REGISTRY.iter().filter(|d| (d.offered)()) {
        let Some(id) = crate::nodes::add_to_graph_on(&mut scratch, def.slug, Pos2::ZERO, workspace)
        else {
            continue;
        };
        let Some(fresh) = scratch.get(id) else {
            continue;
        };
        let ports = if from_output {
            &fresh.inputs
        } else {
            &fresh.outputs
        };
        let folded = |key: &str| key == TIME || key == phasor::OFFSET;
        let takes = |key: &'static str| {
            let made = PortRef::new(id, key);
            let checked = if from_output {
                scratch.can_connect(end, made)
            } else {
                scratch.can_connect(made, end)
            };
            checked.is_ok()
        };
        let key = ports
            .iter()
            .map(|p| p.key)
            .filter(|k| !folded(k))
            .chain(ports.iter().map(|p| p.key).filter(|k| folded(k)))
            .find(|k| takes(k));
        if let Some(key) = key {
            out.push(Taker {
                slug: def.slug,
                key,
            });
        }
    }
    out
}

/// The input and the output that would carry the cable `from` → `to` through `node`, or
/// `None` where nothing would: the node is already cabled, is one of the two ends, or no
/// pairing of its ports passes once the old cable is gone.
///
/// Only a node with no cables of its own is offered, so a splice never quietly rewires
/// something that was already patched. Each pairing is tried on a copy with the old cable
/// taken out and both new ones landed, in declaration order with Time and Offset last, for the
/// reason [`takers`] gives — so the answer is the graph's, a dual node's flip included.
pub fn splice_ports(
    graph: &Graph,
    node: NodeId,
    from: PortRef,
    to: PortRef,
) -> Option<(&'static str, &'static str)> {
    if node == from.node || node == to.node {
        return None;
    }
    let n = graph.get(node)?;
    if graph
        .connections()
        .iter()
        .any(|c| c.from.node == node || c.to.node == node)
    {
        return None;
    }
    let folded = |key: &str| key == TIME || key == phasor::OFFSET;
    let ordered = |ports: &[crate::graph::PortDef]| -> Vec<&'static str> {
        ports
            .iter()
            .map(|p| p.key)
            .filter(|k| !folded(k))
            .chain(ports.iter().map(|p| p.key).filter(|k| folded(k)))
            .collect()
    };
    let (inputs, outputs) = (ordered(&n.inputs), ordered(&n.outputs));
    let mut cut = graph.clone();
    if !cut.disconnect_edge(from, to) {
        return None;
    }
    for input in &inputs {
        // The first half alone, before a copy is spent on the second.
        if cut.can_connect(from, PortRef::new(node, input)).is_err() {
            continue;
        }
        for output in &outputs {
            let mut trial = cut.clone();
            let landed = trial
                .connect(from, PortRef::new(node, input))
                .and_then(|_| trial.connect(PortRef::new(node, output), to));
            if landed.is_ok() {
                return Some((input, output));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::add_to_graph;

    fn wired() -> (Graph, NodeId, NodeId) {
        let mut g = Graph::new();
        let cb = add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
        let out = add_to_graph(&mut g, "output", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(cb, "output"), PortRef::new(out, "input"))
            .unwrap();
        (g, cb, out)
    }

    /// A picture let go in the open is offered what takes a picture and nothing that does
    /// not: an Output, by its input, and no Master Gear.
    #[test]
    fn a_picture_is_offered_what_takes_a_picture() {
        let (g, cb, _) = wired();
        let found = takers(&g, PortRef::new(cb, "output"), g.default_workspace());
        assert!(
            found.contains(&Taker {
                slug: "output",
                key: "input"
            }),
            "{found:?}"
        );
        assert!(!found.iter().any(|t| t.slug == "mastergear"), "{found:?}");
        assert!(!found.is_empty() && found.len() < REGISTRY.len());
    }

    /// From an input the question turns round: a kind is offered by an output that feeds it.
    #[test]
    fn an_input_is_offered_what_can_feed_it() {
        let (g, _, out) = wired();
        let found = takers(&g, PortRef::new(out, "input"), g.default_workspace());
        let checker = found.iter().find(|t| t.slug == "checkerboard");
        assert_eq!(checker.map(|t| t.key), Some("output"), "{found:?}");
        assert!(!found.iter().any(|t| t.slug == "mastergear"), "{found:?}");
    }

    /// The asking leaves the graph it was asked of as it was.
    #[test]
    fn asking_adds_nothing() {
        let (g, cb, _) = wired();
        let before = g.len();
        let _ = takers(&g, PortRef::new(cb, "output"), g.default_workspace());
        assert_eq!(g.len(), before);
    }

    /// A free node that carries the picture splices in; one already cabled does not.
    #[test]
    fn only_a_free_node_splices() {
        let (mut g, cb, out) = wired();
        let (from, to) = (PortRef::new(cb, "output"), PortRef::new(out, "input"));
        let free = add_to_graph(&mut g, "invert", Pos2::ZERO).unwrap();
        assert!(splice_ports(&g, free, from, to).is_some());
        let gear = add_to_graph(&mut g, "mastergear", Pos2::ZERO).unwrap();
        assert_eq!(
            splice_ports(&g, gear, from, to),
            None,
            "a gear carries no picture"
        );
        assert_eq!(
            splice_ports(&g, cb, from, to),
            None,
            "an end is not a middle"
        );
        let other = add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(other, "output"), PortRef::new(free, "input"))
            .unwrap();
        assert_eq!(splice_ports(&g, free, from, to), None, "already patched");
    }
}
