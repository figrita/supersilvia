// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the graph's rules. No GPU, no egui, no window.

use emath::Pos2;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use supersilvia::graph::{ConnectError, Graph, NodeId, PortDef, PortRef, PortType, WorkspaceKind};
use supersilvia::nodes;

const F: PortType = PortType::VaryingNumber;
const C: PortType = PortType::VaryingColor;
const A: PortType = PortType::Action;

/// A node with one input and one output of the same type. The kind names what the node
/// stands for; the ports are the test's own.
fn add(g: &mut Graph, slug: &'static str, ty: PortType) -> NodeId {
    g.add_node(
        nodes::find(slug).expect("a kind in the registry"),
        Pos2::ZERO,
        vec![PortDef::new("in", ty)],
        vec![PortDef::new("out", ty)],
        BTreeSet::from([g.default_workspace()]),
    )
}

fn add_output(g: &mut Graph, ty: PortType) -> NodeId {
    g.add_node(
        nodes::find("output").expect("in the registry"),
        Pos2::ZERO,
        vec![PortDef::new("input", ty)],
        vec![PortDef::new("frame", C)],
        BTreeSet::from([g.default_workspace()]),
    )
}

fn out(n: NodeId) -> PortRef {
    PortRef::new(n, "out")
}
fn inp(n: NodeId) -> PortRef {
    PortRef::new(n, "in")
}

// ---------------------------------------------------------------- identity

#[test]
fn node_ids_are_stable_and_never_reused() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    assert_eq!(a, NodeId(1));

    g.remove_node(a);
    let b = add(&mut g, "checkerboard", C);

    // The counter keeps climbing over a deleted node, so a stale reference and a stale WGSL
    // function name can never collide with a live one.
    assert_ne!(a, b);
    assert_eq!(b, NodeId(2));
}

// ---------------------------------------------------------------- type checking

#[test]
fn types_must_match_exactly() {
    let mut g = Graph::new();
    let f = add(&mut g, "number", F);
    let c = add(&mut g, "checkerboard", C);

    assert_eq!(
        g.connect(out(f), inp(c)),
        Err(ConnectError::TypeMismatch { from: F, to: C }),
    );
    assert!(g.connections().is_empty());
}

#[test]
fn action_ports_only_connect_to_action_ports() {
    let mut g = Graph::new();
    let ev = add(&mut g, "button", A);
    let num = add(&mut g, "number", F);

    assert_eq!(
        g.connect(out(ev), inp(num)),
        Err(ConnectError::ActionMismatch { from: A, to: F }),
    );
    assert_eq!(
        g.connect(out(num), inp(ev)),
        Err(ConnectError::ActionMismatch { from: F, to: A }),
    );
}

#[test]
fn an_unknown_port_key_is_an_error_not_a_panic() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    let bogus = PortRef::new(a, "nope");

    assert_eq!(
        g.connect(bogus, inp(b)),
        Err(ConnectError::NoSuchPort(bogus))
    );
    // Connecting an input to an input is the same error: `out` is not an output on b.
    assert!(matches!(
        g.connect(inp(a), inp(b)),
        Err(ConnectError::NoSuchPort(_)),
    ));
}

// ---------------------------------------------------------------- arity

#[test]
fn a_data_input_takes_one_connection_and_the_newest_wins() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "circle", C);
    let sink = add(&mut g, "blur", C);

    g.connect(out(a), inp(sink)).unwrap();
    g.connect(out(b), inp(sink)).unwrap();

    // Dragging a second cable onto an occupied input replaces rather than stacks.
    assert_eq!(g.connections().len(), 1);
    assert_eq!(g.source_of(inp(sink)), Some(out(b)));
}

#[test]
fn action_connections_are_many_to_many() {
    let mut g = Graph::new();
    let src1 = add(&mut g, "button", A);
    let src2 = add(&mut g, "clock", A);
    let dst1 = add(&mut g, "counter", A);
    let dst2 = add(&mut g, "adsr", A);

    g.connect(out(src1), inp(dst1)).unwrap();
    g.connect(out(src1), inp(dst2)).unwrap();
    g.connect(out(src2), inp(dst1)).unwrap();

    assert_eq!(g.connections().len(), 3);
    assert_eq!(g.targets_of(out(src1)).count(), 2);

    // The same edge twice is still one edge.
    g.connect(out(src1), inp(dst1)).unwrap();
    assert_eq!(g.connections().len(), 3);
}

// ---------------------------------------------------------------- cycles

#[test]
fn a_node_cannot_feed_itself() {
    let mut g = Graph::new();
    let a = add(&mut g, "blur", C);
    assert_eq!(
        g.connect(out(a), inp(a)),
        Err(ConnectError::SelfConnection(a))
    );
}

#[test]
fn data_cycles_are_rejected() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    let c = add(&mut g, "invert", C);

    g.connect(out(a), inp(b)).unwrap();
    g.connect(out(b), inp(c)).unwrap();

    // Closing the loop c -> a would make the compiler recurse forever.
    assert_eq!(
        g.connect(out(c), inp(a)),
        Err(ConnectError::WouldCycle { from: c, to: a }),
    );
    assert_eq!(g.connections().len(), 2);
}

#[test]
fn action_cycles_are_allowed() {
    let mut g = Graph::new();
    let a = add(&mut g, "clock", A);
    let b = add(&mut g, "counter", A);

    g.connect(out(a), inp(b)).unwrap();
    // Action edges are CPU events, never compiled, so a loop is legal.
    assert!(g.connect(out(b), inp(a)).is_ok());
    assert_eq!(g.connections().len(), 2);
}

/// What a cable drag asks of every candidate port, from either end, is what a cable made
/// then would be told — and a reach walked before the cables changed is not believed.
#[test]
fn a_drags_reach_answers_as_can_connect_does() {
    let mut g = Graph::new();
    let chain: Vec<NodeId> = (0..5).map(|_| add(&mut g, "blur", C)).collect();
    for w in chain.windows(2) {
        g.connect(out(w[0]), inp(w[1])).unwrap();
    }
    let loose = add(&mut g, "blur", C);
    let mut ends: Vec<PortRef> = chain.iter().map(|n| out(*n)).collect();
    ends.extend(chain.iter().map(|n| inp(*n)));
    ends.extend([out(loose), inp(loose)]);
    for end in &ends {
        let reach = g.reach(*end);
        assert!(reach.is_current(&g, *end));
        for other in &ends {
            for (from, to) in [(*end, *other), (*other, *end)] {
                assert_eq!(
                    g.can_connect_within(&reach, from, to),
                    g.can_connect(from, to),
                    "{from:?} -> {to:?} from the reach of {end:?}",
                );
            }
        }
    }

    // Walked down from the chain's head before the chain fed the loose node, so it does not
    // know the loose node is downstream; believed, it would let the loop close.
    let reach = g.reach(inp(chain[0]));
    g.connect(out(chain[4]), inp(loose)).unwrap();
    assert!(!reach.is_current(&g, inp(chain[0])));
    assert!(matches!(
        g.can_connect_within(&reach, out(loose), inp(chain[0])),
        Err(ConnectError::WouldCycle { .. })
    ));
}

#[test]
fn can_connect_does_not_mutate() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);

    assert!(g.can_connect(out(a), inp(b)).is_ok());
    assert!(
        g.connections().is_empty(),
        "can_connect must be a pure query"
    );
}

// ---------------------------------------------------------------- disconnection

#[test]
fn removing_a_node_removes_its_connections() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    let c = add(&mut g, "invert", C);
    g.connect(out(a), inp(b)).unwrap();
    g.connect(out(b), inp(c)).unwrap();

    g.remove_node(b);
    assert!(g.connections().is_empty(), "dangling edges left behind");
}

#[test]
fn disconnect_reports_what_it_removed() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    g.connect(out(a), inp(b)).unwrap();

    let removed = g.disconnect(inp(b));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].from, out(a));
    assert!(g.disconnect(inp(b)).is_empty());
}

/// What a right-click on a port dot does: `disconnect` only reaches a port from its input
/// side, and a port can just as well be an output feeding several inputs at once.
#[test]
fn disconnect_port_clears_an_input_or_an_output() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    let c = add(&mut g, "invert", C);
    g.connect(out(a), inp(b)).unwrap();
    g.connect(out(a), inp(c)).unwrap();

    // The output side: both edges leaving it go, in one call.
    let removed = g.disconnect_port(out(a));
    assert_eq!(removed.len(), 2);
    assert!(g.connections().is_empty());
    assert!(
        g.disconnect_port(out(a)).is_empty(),
        "nothing left to clear"
    );

    // The input side, same as `disconnect`.
    g.connect(out(a), inp(b)).unwrap();
    let removed = g.disconnect_port(inp(b));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].from, out(a));
}

// ---------------------------------------------------------------- ordering

#[test]
fn topological_order_puts_producers_before_consumers() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    let c = add(&mut g, "invert", C);

    // Connect out of order: c <- b <- a.
    g.connect(out(b), inp(c)).unwrap();
    g.connect(out(a), inp(b)).unwrap();

    let order = g.topological_order().to_vec();
    let at = |n| order.iter().position(|&x| x == n).unwrap();
    assert!(at(a) < at(b), "a must be ticked before b");
    assert!(at(b) < at(c), "b must be ticked before c");
    assert_eq!(order.len(), 3);
}

#[test]
fn topological_order_includes_isolated_nodes() {
    let mut g = Graph::new();
    add(&mut g, "checkerboard", C);
    add(&mut g, "circle", C);
    assert_eq!(g.topological_order().len(), 2);
}

#[test]
fn the_topo_cache_is_invalidated_by_structural_change() {
    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "blur", C);
    assert_eq!(g.topological_order().len(), 2);

    let c = add(&mut g, "invert", C);
    assert_eq!(g.topological_order().len(), 3, "cache survived add_node");

    g.connect(out(a), inp(b)).unwrap();
    g.connect(out(b), inp(c)).unwrap();
    let order = g.topological_order().to_vec();
    let at = |n| order.iter().position(|&x| x == n).unwrap();
    assert!(at(a) < at(c), "cache survived connect");

    g.remove_node(b);
    assert_eq!(g.topological_order().len(), 2, "cache survived remove_node");
}

/// A node with the ports the order tests need: two picture inputs and an event input; a
/// picture out this frame, the same picture a frame late, and an event.
fn add_loopable(g: &mut Graph) -> NodeId {
    g.add_node(
        nodes::find("mix").expect("in the registry"),
        Pos2::ZERO,
        vec![
            PortDef::new("a", C),
            PortDef::new("b", C),
            PortDef::new("act", A),
        ],
        vec![
            PortDef::new("out", C),
            PortDef::delayed("late", C),
            PortDef::new("fire", A),
        ],
        BTreeSet::from([g.default_workspace()]),
    )
}

fn at(order: &[NodeId], n: NodeId) -> usize {
    order.iter().position(|&x| x == n).unwrap()
}

/// A loop through a delayed port is placed whole before what it feeds, whatever the ids: a
/// consumer made before its producer, both downstream of a loop, still follows it.
#[test]
fn everything_downstream_of_a_loop_follows_it() {
    let mut g = Graph::new();
    let q = add_loopable(&mut g);
    let p = add_loopable(&mut g);
    let l = add_loopable(&mut g);
    let m = add_loopable(&mut g);
    // The loop: m into l this frame, l into m a frame late.
    g.connect(PortRef::new(m, "out"), PortRef::new(l, "a"))
        .unwrap();
    g.connect(PortRef::new(l, "late"), PortRef::new(m, "a"))
        .unwrap();
    // Downstream of it, the consumer's id lower than its producer's.
    g.connect(PortRef::new(l, "late"), PortRef::new(p, "a"))
        .unwrap();
    g.connect(PortRef::new(p, "late"), PortRef::new(q, "a"))
        .unwrap();

    for order in [g.topological_order().to_vec(), g.tick_order().to_vec()] {
        assert!(
            at(&order, m) < at(&order, l),
            "inside the loop, this frame's edge holds"
        );
        assert!(at(&order, l) < at(&order, p), "{order:?}");
        assert!(at(&order, p) < at(&order, q), "{order:?}");
    }
    assert_eq!(
        g.in_cycles(&g.iter().map(|(id, _)| id).collect()),
        HashSet::from([l, m]),
        "the loop is its two members, and not what it feeds"
    );
    assert_eq!(
        g.in_cycles(&HashSet::from([l, p, q])),
        HashSet::new(),
        "a loop through a node left out is no loop"
    );
}

/// The tick's order breaks an action loop and nothing downstream of it: a consumer with the
/// lowest id still runs after the loop that fires it.
#[test]
fn a_tick_runs_what_an_action_loop_fires_after_it() {
    let mut g = Graph::new();
    let c = add_loopable(&mut g);
    let a = add_loopable(&mut g);
    let b = add_loopable(&mut g);
    g.connect(PortRef::new(a, "fire"), PortRef::new(b, "act"))
        .unwrap();
    g.connect(PortRef::new(b, "fire"), PortRef::new(a, "act"))
        .unwrap();
    g.connect(PortRef::new(b, "fire"), PortRef::new(c, "act"))
        .unwrap();
    assert_eq!(
        g.tick_order(),
        [a, b, c],
        "the loop breaks at its lowest id"
    );
    assert_eq!(
        g.topological_order(),
        [c, a, b],
        "and data order sees no edge"
    );
}

/// A CPU chain behind a tap loop ticks producer first, where the loop elsewhere upstream
/// once put the whole of it in id order.
#[test]
fn a_cpu_chain_after_a_tap_loop_ticks_in_order() {
    let mut g = Graph::new();
    let add = |g: &mut Graph, slug| nodes::add_to_graph(g, slug, Pos2::ZERO).unwrap();
    let consumer = add(&mut g, "slew");
    let producer = add(&mut g, "slew");
    let out = add(&mut g, "output");
    let cb = add(&mut g, "checkerboard");
    let mix = add(&mut g, "mix");
    let tap = add(&mut g, "tap");
    let wire = |g: &mut Graph, from: (NodeId, &'static str), to: (NodeId, &'static str)| {
        g.connect(PortRef::new(from.0, from.1), PortRef::new(to.0, to.1))
            .unwrap();
    };
    wire(&mut g, (cb, "output"), (mix, "a"));
    wire(&mut g, (mix, "output"), (tap, "input"));
    wire(&mut g, (tap, "output"), (out, "input"));
    wire(&mut g, (producer, "output"), (consumer, "input"));
    wire(&mut g, (tap, "mean"), (producer, "input"));
    // The reading drives what it measures.
    wire(&mut g, (tap, "mean"), (mix, "amount"));

    let order = g.tick_order();
    assert!(at(order, tap) < at(order, producer), "{order:?}");
    assert!(at(order, producer) < at(order, consumer), "{order:?}");
}

type Keep = dyn Fn(&Graph, &PortRef) -> bool;

fn is_data(g: &Graph, from: &PortRef) -> bool {
    g.get(from.node)
        .unwrap()
        .output(from.key)
        .unwrap()
        .ty
        .is_data()
}

fn is_delayed(g: &Graph, from: &PortRef) -> bool {
    g.get(from.node).unwrap().output(from.key).unwrap().delayed
}

/// Kahn's order, seeded in id order and walking each node's cables in the order they were
/// made: what the order is on a graph without a loop.
fn kahn(g: &Graph, keep: &Keep) -> Vec<NodeId> {
    let kept: Vec<_> = g
        .connections()
        .iter()
        .filter(|c| keep(g, &c.from))
        .collect();
    let mut indegree: HashMap<NodeId, usize> = g.iter().map(|(id, _)| (id, 0)).collect();
    for c in &kept {
        *indegree.get_mut(&c.to.node).unwrap() += 1;
    }
    let mut queue: VecDeque<NodeId> = g
        .iter()
        .map(|(id, _)| id)
        .filter(|id| indegree[id] == 0)
        .collect();
    let mut order = Vec::new();
    while let Some(n) = queue.pop_front() {
        order.push(n);
        for c in kept.iter().filter(|c| c.from.node == n) {
            let d = indegree.get_mut(&c.to.node).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push_back(c.to.node);
            }
        }
    }
    order
}

/// Does `from` reach `to`, in one step or more, over the cables `keep` passes?
fn reaches(g: &Graph, from: NodeId, to: NodeId, keep: &Keep) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![from];
    while let Some(v) = stack.pop() {
        for c in g.connections() {
            if c.from.node == v && keep(g, &c.from) {
                if c.to.node == to {
                    return true;
                }
                if seen.insert(c.to.node) {
                    stack.push(c.to.node);
                }
            }
        }
    }
    false
}

/// Over seeded random graphs of loops, chains and action edges: every cable outside a loop
/// points forward in the order that sees it, every immediate cable does wherever it is, the
/// loops found are exactly the nodes that reach themselves, and a graph with no loop is
/// ordered exactly as Kahn orders it.
#[test]
fn every_cable_outside_a_loop_points_forward_in_random_graphs() {
    let mut state: u64 = 0x5EED_0F0D_E125;
    let mut next = move |n: usize| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % n as u64) as usize
    };
    let data: &Keep = &|g, p| is_data(g, p);
    let all: &Keep = &|_, _| true;
    let (mut looped, mut acyclic) = (0, 0);
    for _ in 0..400 {
        let mut g = Graph::new();
        let ids: Vec<NodeId> = (0..3 + next(10)).map(|_| add_loopable(&mut g)).collect();
        for _ in 0..ids.len() + next(ids.len() * 3) {
            let (from, to) = (ids[next(ids.len())], ids[next(ids.len())]);
            let (out, input) = [
                ("out", "a"),
                ("late", "b"),
                ("fire", "act"),
                ("out", "b"),
                ("late", "a"),
            ][next(5)];
            // A refused cable is part of the test: an immediate loop never lands.
            let _ = g.connect(PortRef::new(from, out), PortRef::new(to, input));
        }

        let everything: HashSet<NodeId> = ids.iter().copied().collect();
        let cyclic: HashSet<NodeId> = ids
            .iter()
            .copied()
            .filter(|n| reaches(&g, *n, *n, data))
            .collect();
        assert_eq!(g.in_cycles(&everything), cyclic);
        if cyclic.is_empty() {
            acyclic += 1;
            assert_eq!(
                g.topological_order(),
                kahn(&g, data),
                "no loop, so Kahn's order"
            );
        } else {
            looped += 1;
        }

        for (order, keep) in [(g.topological_order(), data), (g.tick_order(), all)] {
            assert_eq!(order.len(), ids.len(), "every node, once");
            for c in g.connections().iter().filter(|c| keep(&g, &c.from)) {
                let immediate = is_data(&g, &c.from) && !is_delayed(&g, &c.from);
                let in_a_loop = reaches(&g, c.to.node, c.from.node, keep);
                if immediate || !in_a_loop {
                    assert!(
                        at(order, c.from.node) < at(order, c.to.node),
                        "{}.{} -> {}.{} points back in {order:?}\n{:?}",
                        c.from.node,
                        c.from.key,
                        c.to.node,
                        c.to.key,
                        g.connections()
                    );
                }
            }
        }
    }
    assert!(
        looped > 100 && acyclic > 50,
        "{looped} with a loop, {acyclic} without"
    );
}

// ---------------------------------------------------------------- downstream outputs

#[test]
fn downstream_outputs_finds_the_shaders_to_rebuild() {
    let mut g = Graph::new();
    let src = add(&mut g, "checkerboard", C);
    let mid = add(&mut g, "blur", C);
    let out_a = add_output(&mut g, C);
    let out_b = add_output(&mut g, C);
    let unrelated = add_output(&mut g, C);

    g.connect(out(src), inp(mid)).unwrap();
    g.connect(out(mid), PortRef::new(out_a, "input")).unwrap();
    g.connect(out(mid), PortRef::new(out_b, "input")).unwrap();

    let mut hit = g.downstream_outputs(src);
    hit.sort();
    let mut want = vec![out_a, out_b];
    want.sort();
    assert_eq!(hit, want);
    assert!(!hit.contains(&unrelated));
}

#[test]
fn downstream_outputs_follows_frame_edges_between_outputs() {
    let mut g = Graph::new();
    let src = add(&mut g, "checkerboard", C);
    let a = add_output(&mut g, C);
    let b = add_output(&mut g, C);

    g.connect(out(src), PortRef::new(a, "input")).unwrap();
    // §2b: Output A's `frame` port feeds Output B, so B is downstream of A.
    g.connect(PortRef::new(a, "frame"), PortRef::new(b, "input"))
        .unwrap();

    let mut hit = g.downstream_outputs(src);
    hit.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(hit, want, "recompile must propagate across a frame edge");
}

#[test]
fn an_output_is_downstream_of_itself() {
    let mut g = Graph::new();
    let a = add_output(&mut g, C);
    // Changing an Output's own option rebuilds its own shader.
    assert_eq!(g.downstream_outputs(a), vec![a]);
}

// ---------------------------------------------------------------- workspaces

/// Every node's set is non-empty and every id in it names a live workspace.
fn assert_invariants(g: &Graph) {
    for (id, node) in g.iter() {
        assert!(!node.workspaces.is_empty(), "node {id} is on no workspace");
        for w in &node.workspaces {
            assert!(
                g.workspace(*w).is_some(),
                "node {id} names workspace {w}, which is not in the graph"
            );
        }
    }
}

#[test]
fn a_new_graph_has_one_workspace_and_a_node_lands_on_it() {
    let mut g = Graph::new();
    assert_eq!(g.workspaces().len(), 1);
    assert_eq!(g.workspaces()[0].name, "Workspace 1");
    assert_eq!(g.workspaces()[0].kind, WorkspaceKind::Video);

    let n = add(&mut g, "zoom", F);
    assert_eq!(
        g.get(n).unwrap().workspaces,
        BTreeSet::from([g.default_workspace()])
    );
    assert_invariants(&g);
}

/// **Going to a node prefers an open tab**: the one showing where the node is on it, then the
/// first open one in project order, then the first at all — project order, not id order.
#[test]
fn a_nodes_home_is_the_tab_showing_then_an_open_one_then_the_first() {
    let mut g = Graph::new();
    let first = g.default_workspace();
    let second = g.add_workspace("second".into(), WorkspaceKind::Video);
    let third = g.add_workspace("third".into(), WorkspaceKind::Video);
    // Project order third, second, first: the ids say otherwise.
    assert!(g.move_workspace(third, 0));
    assert!(g.move_workspace(second, 1));
    let n = add(&mut g, "zoom", F);
    g.get_mut(n).unwrap().workspaces = BTreeSet::from([first, second, third]);

    let none = BTreeSet::new();
    assert_eq!(
        g.home_of(n, None, &none),
        Some(third),
        "the first in project order"
    );
    let open = BTreeSet::from([first, second]);
    assert_eq!(
        g.home_of(n, None, &open),
        Some(second),
        "the first open one"
    );
    assert_eq!(
        g.home_of(n, Some(first), &open),
        Some(first),
        "the one showing"
    );

    g.get_mut(n).unwrap().workspaces = BTreeSet::from([first, third]);
    assert_eq!(
        g.home_of(n, Some(second), &open),
        Some(first),
        "a tab the node is not on is not its home"
    );
    assert_eq!(g.home_of(NodeId(999), None, &open), None);
}

#[test]
fn a_workspace_id_is_not_reused() {
    let mut g = Graph::new();
    let first = g.add_workspace("second".into(), WorkspaceKind::Video);
    g.remove_workspace(first);
    let second = g.add_workspace("third".into(), WorkspaceKind::Video);
    assert_ne!(first, second);
    assert!(g.workspace(first).is_none());
    assert!(g.workspace(second).is_some());
}

/// Removing a workspace is silvia's `_onBeforeDelete`: a node visible elsewhere stays, a
/// node visible only there goes, and its cables go with it.
#[test]
fn removing_a_workspace_keeps_a_shared_node_and_removes_an_unshared_one() {
    let mut g = Graph::new();
    let first = g.default_workspace();
    let second = g.add_workspace("Workspace 2".into(), WorkspaceKind::Video);

    let shared = add(&mut g, "zoom", F);
    let only_there = add(&mut g, "zoom", F);
    g.get_mut(shared).unwrap().workspaces.insert(second);
    g.get_mut(only_there).unwrap().workspaces = BTreeSet::from([second]);
    g.connect(out(shared), inp(only_there)).unwrap();
    assert_eq!(g.connections().len(), 1);

    assert_eq!(g.remove_workspace(second), Some(vec![only_there]));
    assert!(
        g.get(shared).is_some(),
        "it is still on the first workspace"
    );
    assert!(g.get(only_there).is_none(), "it was on nothing else");
    assert_eq!(g.connections().len(), 0, "the cable went with the node");
    assert_eq!(g.get(shared).unwrap().workspaces, BTreeSet::from([first]));
    assert_invariants(&g);
}

#[test]
fn the_last_workspace_cannot_be_removed() {
    let mut g = Graph::new();
    let only = g.default_workspace();
    add(&mut g, "zoom", F);
    assert_eq!(g.remove_workspace(only), None);
    assert_eq!(g.workspaces().len(), 1);
    assert_eq!(g.len(), 1);
}

#[test]
fn a_workspace_moves_in_project_order() {
    let mut g = Graph::new();
    let a = g.default_workspace();
    let b = g.add_workspace("b".into(), WorkspaceKind::Video);
    let c = g.add_workspace("c".into(), WorkspaceKind::Video);

    assert!(g.move_workspace(c, 0));
    let order: Vec<_> = g.workspaces().iter().map(|w| w.id).collect();
    assert_eq!(order, vec![c, a, b]);

    // Past the end lands at the end rather than failing.
    assert!(g.move_workspace(c, 99));
    let order: Vec<_> = g.workspaces().iter().map(|w| w.id).collect();
    assert_eq!(order, vec![a, b, c]);
}

/// The undo ring is capped by bytes, and this is the estimate it counts. 150 KB for 200
/// nodes is the measured figure `docs/architecture.md` quotes.
#[test]
fn a_two_hundred_node_graph_estimates_near_a_hundred_and_fifty_kilobytes() {
    let mut g = Graph::new();
    let mut prev = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    for _ in 0..199 {
        let z = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(prev, "output"), PortRef::new(z, "input"))
            .unwrap();
        prev = z;
    }
    assert_eq!(g.len(), 200);

    let bytes = g.footprint();
    assert!(
        (75_000..300_000).contains(&bytes),
        "{bytes} is not within a factor of two of 150 KB"
    );
}

/// The editor, its undo ring and the synth thread all hold one graph, so it has to be
/// readable from two threads at once.
#[test]
fn a_graph_can_be_shared_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<Graph>();
}

/// A clone is the same nodes, not copies of them, and a write to one node copies that node
/// alone.
#[test]
fn a_clone_shares_every_node_until_one_is_written() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    let b = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();
    let before = g.clone();
    assert!(g.shares_node(&before, a) && g.shares_node(&before, b));
    assert!(g.shares_connections(&before));

    g.get_mut(b).unwrap().pos = Pos2::new(10.0, 0.0);
    assert!(
        g.shares_node(&before, a),
        "the node nobody wrote is still the same one"
    );
    assert!(!g.shares_node(&before, b), "the one written is a copy");
    assert_eq!(
        before.get(b).unwrap().pos,
        Pos2::ZERO,
        "and the original is untouched"
    );
    assert!(g.shares_connections(&before), "a move is not a cable");
}

/// What a step costs the undo ring: a two-hundred-node graph with one node written holds
/// that node and its map of pointers beside the graph before, not a whole graph.
#[test]
fn a_graph_one_edit_apart_costs_one_node() {
    let mut g = Graph::new();
    let mut prev = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    for _ in 0..199 {
        let z = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(prev, "output"), PortRef::new(z, "input"))
            .unwrap();
        prev = z;
    }
    let before = g.clone();
    assert!(
        before.footprint_beside(&g) < 200 * 32,
        "an unedited clone holds nothing but its map: {}",
        before.footprint_beside(&g)
    );

    g.get_mut(prev).unwrap().pos = Pos2::new(1.0, 1.0);
    let alone = before.footprint_beside(&g);
    assert!(
        alone < g.footprint() / 10,
        "{alone} bytes of {} is more than one node and a map",
        g.footprint()
    );
}

/// What feeds a port and what a port feeds are answered from an index the graph keeps beside
/// its cable list, and the two never disagree: after every kind of cable change, each answer
/// is the one a scan of the list gives — and a clone taken before a change keeps its own.
#[test]
fn the_cable_index_agrees_with_the_cable_list() {
    fn agrees(g: &Graph) {
        for (id, node) in g.iter() {
            for p in &node.inputs {
                let at = PortRef::new(id, p.key);
                let scanned: Vec<PortRef> = g
                    .connections()
                    .iter()
                    .filter(|c| c.to == at)
                    .map(|c| c.from)
                    .collect();
                assert_eq!(g.sources_of(at).collect::<Vec<_>>(), scanned, "{at:?}");
                assert_eq!(g.source_of(at), scanned.first().copied(), "{at:?}");
            }
            for p in &node.outputs {
                let at = PortRef::new(id, p.key);
                let scanned: Vec<PortRef> = g
                    .connections()
                    .iter()
                    .filter(|c| c.from == at)
                    .map(|c| c.to)
                    .collect();
                assert_eq!(g.targets_of(at).collect::<Vec<_>>(), scanned, "{at:?}");
            }
        }
    }

    let mut g = Graph::new();
    let a = add(&mut g, "checkerboard", C);
    let b = add(&mut g, "invert", C);
    let c = add(&mut g, "blur", C);
    let x = add(&mut g, "button", A);
    let y = add(&mut g, "counter", A);
    let z = add(&mut g, "adsr", A);

    g.connect(out(a), inp(b)).unwrap();
    g.connect(out(b), inp(c)).unwrap();
    // Onto an occupied data input: the cable it replaces leaves the index with it.
    g.connect(out(a), inp(c)).unwrap();
    agrees(&g);
    assert_eq!(g.targets_of(out(b)).count(), 0);

    g.connect(out(x), inp(y)).unwrap();
    g.connect(out(x), inp(z)).unwrap();
    g.connect(out(y), inp(z)).unwrap();
    // The same edge twice is one edge, in the index as in the list.
    g.connect(out(x), inp(y)).unwrap();
    agrees(&g);
    assert_eq!(g.sources_of(inp(z)).count(), 2);

    let before = g.clone();
    assert!(g.disconnect_edge(out(x), inp(z)));
    agrees(&g);
    agrees(&before);
    assert_eq!(
        before.targets_of(out(x)).count(),
        2,
        "the clone kept the index it had"
    );

    assert_eq!(g.disconnect(inp(c)).len(), 1);
    agrees(&g);
    assert_eq!(g.disconnect_port(out(x)).len(), 1);
    agrees(&g);
    g.connect(out(a), inp(c)).unwrap();
    assert_eq!(g.disconnect_node(a).len(), 2);
    agrees(&g);
    g.connect(out(b), inp(c)).unwrap();
    g.remove_node(b);
    agrees(&g);
    assert_eq!(g.source_of(inp(c)), None, "a removed node feeds nothing");
    assert_eq!(g.connections().len(), 1, "y into z is all that is left");
}

/// A bulk change settles to the same types a gesture at a time does: a chain of dual
/// nodes into an input that reads a number, cabled consumer first so no cable arrives with
/// its source's type already known.
#[test]
fn a_bulk_change_settles_to_what_gestures_would_have_made() {
    let build = |bulk: bool| {
        let mut g = Graph::new();
        if bulk {
            g.begin_bulk();
        }
        let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
        let b = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
        let slew = nodes::add_to_graph(&mut g, "slew", Pos2::ZERO).unwrap();
        g.connect_loading(PortRef::new(b, "output"), PortRef::new(slew, "input"))
            .unwrap();
        g.connect_loading(PortRef::new(a, "output"), PortRef::new(b, "a"))
            .unwrap();
        let dropped = g.settle_effective_types();
        assert!(dropped.is_empty(), "nothing here reads a field as a number");
        assert!(!g.in_bulk(), "settling closes the change");
        let types = |id: NodeId| {
            let n = g.get(id).unwrap();
            (
                n.inputs.iter().map(|p| p.ty).collect::<Vec<_>>(),
                n.outputs.iter().map(|p| p.ty).collect::<Vec<_>>(),
            )
        };
        (types(a), types(b), g.connections().len())
    };
    assert_eq!(build(true), build(false));
}

// ---------------------------------------------------------------- dual outputs

/// The node and port a field comes from: a luminosity of the default hue wheel.
fn field(g: &mut Graph) -> PortRef {
    PortRef::new(
        nodes::add_to_graph(g, "luminosity", Pos2::ZERO).unwrap(),
        "output",
    )
}

fn out_ty(g: &Graph, node: NodeId, key: &str) -> PortType {
    g.get(node).unwrap().output(key).unwrap().ty
}

/// A `math` node with nothing but knobs on it is a constant number, so its output is a
/// `UniformNumber` from the frame it is added — not from the frame something is connected
/// to it.
#[test]
fn a_math_node_with_knobs_publishes_a_number() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
    // And its declaration still says `VaryingNumber`: the effective type is the instance's.
    assert_eq!(
        nodes::find("add").unwrap().output("output").unwrap().ty,
        PortType::VaryingNumber,
    );
}

/// A field on any input is a field on the output: there is no `uv` on the CPU, so a node
/// reading one has to be evaluated per pixel.
#[test]
fn a_field_into_a_math_node_makes_it_a_field() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let f = field(&mut g);

    g.connect(f, PortRef::new(a, "a")).unwrap();
    assert_eq!(out_ty(&g, a, "output"), PortType::VaryingNumber);

    // And the promotion is the same rule read backwards.
    g.disconnect(PortRef::new(a, "a"));
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
}

/// A number that arrives as a number keeps the node a number: two gear outputs into an `add`
/// is uniform number arithmetic on the CPU.
#[test]
fn uniforms_into_a_math_node_leave_it_uniform() {
    let mut g = Graph::new();
    let p = nodes::add_to_graph(&mut g, "ratiogear", Pos2::ZERO).unwrap();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(p, "cycles"), PortRef::new(a, "a"))
        .unwrap();
    g.connect(PortRef::new(p, "wrapped"), PortRef::new(a, "b"))
        .unwrap();
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
}

/// A chain of duals flips as one, in either direction, because each link's answer is its
/// producer's answer.
#[test]
fn a_chain_of_math_nodes_flips_together() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let m = nodes::add_to_graph(&mut g, "multiply", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(m, "a"))
        .unwrap();
    assert_eq!(
        out_ty(&g, m, "output"),
        PortType::UniformNumber,
        "two knobs deep"
    );

    let f = field(&mut g);
    g.connect(f, PortRef::new(a, "a")).unwrap();
    assert_eq!(out_ty(&g, a, "output"), PortType::VaryingNumber);
    assert_eq!(
        out_ty(&g, m, "output"),
        PortType::VaryingNumber,
        "the flip traveled down the chain in one edit"
    );

    g.disconnect(PortRef::new(a, "a"));
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
    assert_eq!(
        out_ty(&g, m, "output"),
        PortType::UniformNumber,
        "and back again"
    );
}

/// The effective type of every input of a node, in row order.
fn in_tys(g: &Graph, node: NodeId) -> Vec<PortType> {
    g.get(node).unwrap().inputs.iter().map(|p| p.ty).collect()
}

/// **A pinned dual node's inputs are `UniformNumber`.** A `slew` reads the `add`'s number,
/// so a field cannot land on either of its inputs — and the inputs say so rather than
/// drawing a circle that promises a field may go there.
#[test]
fn a_slew_on_a_math_nodes_output_pins_its_inputs() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    assert_eq!(
        in_tys(&g, a),
        [PortType::VaryingNumber, PortType::VaryingNumber],
        "unpinned by construction: nothing reads its number yet"
    );

    let slew = nodes::add_to_graph(&mut g, "slew", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(slew, "input"))
        .expect("a diamond feeds a diamond");
    assert_eq!(
        in_tys(&g, a),
        [PortType::UniformNumber, PortType::UniformNumber]
    );

    // And the pin goes with the reader, which needs no permission of any kind.
    g.remove_node(slew);
    assert_eq!(
        in_tys(&g, a),
        [PortType::VaryingNumber, PortType::VaryingNumber]
    );
}

/// The pin travels the chain: every dual node between the reader and the cable would flip
/// with it, so every one of them is pinned.
#[test]
fn a_pin_travels_the_whole_chain_of_duals() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let m = nodes::add_to_graph(&mut g, "multiply", Pos2::ZERO).unwrap();
    let slew = nodes::add_to_graph(&mut g, "slew", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(m, "a"))
        .unwrap();
    g.connect(PortRef::new(m, "output"), PortRef::new(slew, "input"))
        .unwrap();

    assert_eq!(
        in_tys(&g, m),
        [PortType::UniformNumber, PortType::UniformNumber]
    );
    assert_eq!(
        in_tys(&g, a),
        [PortType::UniformNumber, PortType::UniformNumber],
        "the pin reached upstream through the multiply"
    );

    // Cut the middle and the far end is nobody's business: only the `multiply` still feeds
    // the number the `slew` reads.
    g.disconnect(PortRef::new(m, "a"));
    assert_eq!(
        in_tys(&g, a),
        [PortType::VaryingNumber, PortType::VaryingNumber]
    );
    assert_eq!(
        in_tys(&g, m),
        [PortType::UniformNumber, PortType::UniformNumber]
    );
}

/// **Demotion is unofferable, and the port is the one that says so.** A field onto a pinned
/// input is an ordinary type mismatch: the input *is* a `UniformNumber`.
#[test]
fn a_field_into_a_pinned_input_is_a_type_mismatch() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let slew = nodes::add_to_graph(&mut g, "slew", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(slew, "input"))
        .unwrap();
    let f = field(&mut g);

    assert_eq!(
        g.connect(f, PortRef::new(a, "a")),
        Err(ConnectError::TypeMismatch {
            from: PortType::VaryingNumber,
            to: PortType::UniformNumber,
        }),
    );
    assert_eq!(g.connections().len(), 1, "and nothing was changed");
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);

    // A uniform number still lands there, which is what the diamond promises.
    let p = nodes::add_to_graph(&mut g, "ratiogear", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(p, "cycles"), PortRef::new(a, "a"))
        .expect("a number feeds a pinned input");
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
    assert_eq!(
        in_tys(&g, a),
        [PortType::UniformNumber, PortType::UniformNumber]
    );
}

/// And the pin is only about what reads the number. A `zoom` takes a field as happily as a
/// number, so the `add` feeding one is unpinned and the same cable is an ordinary edit.
#[test]
fn a_dual_node_feeding_only_a_field_input_stays_unpinned() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();
    assert_eq!(
        in_tys(&g, a),
        [PortType::VaryingNumber, PortType::VaryingNumber]
    );
    let f = field(&mut g);

    g.connect(f, PortRef::new(a, "a"))
        .expect("a zoom takes a field as happily as a number");
    assert_eq!(out_ty(&g, a, "output"), PortType::VaryingNumber);
    assert_eq!(
        in_tys(&g, a),
        [PortType::VaryingNumber, PortType::VaryingNumber],
        "a field node's inputs are fields"
    );
}

/// Removing the producer of a field promotes what it fed, which needs no permission: a
/// number is what every `UniformNumber` input wanted in the first place.
#[test]
fn removing_a_field_promotes_what_it_fed() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let f = field(&mut g);
    g.connect(f, PortRef::new(a, "a")).unwrap();
    assert_eq!(out_ty(&g, a, "output"), PortType::VaryingNumber);

    g.remove_node(f.node);
    assert_eq!(out_ty(&g, a, "output"), PortType::UniformNumber);
}

// ---------------------------------------------------------------- bridges

/// A color onto a number is the conversion menu's whole reason: the cable is refused on type
/// alone, the table holds castings that carry it, and a node between the two closes no loop.
#[test]
fn a_color_onto_a_number_can_be_bridged() {
    let mut g = Graph::new();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();

    let from = PortRef::new(cb, "output");
    let to = PortRef::new(zoom, "zoom");
    assert!(matches!(
        g.can_connect(from, to),
        Err(ConnectError::TypeMismatch { .. })
    ));
    assert!(nodes::can_bridge(&g, from, to));
}

/// A pair a cable already joins is a plain connect, not a conversion. There is nothing to
/// choose, so there is no menu and no ring.
#[test]
fn a_legal_cable_is_not_a_bridge() {
    let mut g = Graph::new();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();

    assert!(!nodes::can_bridge(
        &g,
        PortRef::new(cb, "output"),
        PortRef::new(zoom, "input"),
    ));
}

/// An action has nothing to convert to or from, and the refusal is an `ActionMismatch` rather
/// than a type: a square port is dimmed during a color's drag, never ringed.
#[test]
fn an_action_is_never_bridged() {
    let mut g = Graph::new();
    let button = nodes::add_to_graph(&mut g, "button", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
    let counter = nodes::add_to_graph(&mut g, "counter", Pos2::ZERO).unwrap();

    assert!(!nodes::can_bridge(
        &g,
        PortRef::new(button, "trigger"),
        PortRef::new(zoom, "zoom"),
    ));
    assert!(!nodes::can_bridge(
        &g,
        PortRef::new(zoom, "output"),
        PortRef::new(counter, "increment"),
    ));
}

/// A bridge is a node in the path, so it closes the loop a plain cable would have closed.
/// The target's node reaching the source's is the whole of the question.
#[test]
fn a_bridge_that_would_cycle_is_not_offered() {
    let mut g = Graph::new();
    let luma = nodes::add_to_graph(&mut g, "luminosity", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();

    // A number out of the luminosity into a color input of the zoom: refused on type, and
    // `rgba` would carry it.
    let from = PortRef::new(luma, "output");
    let to = PortRef::new(zoom, "input");
    assert!(nodes::can_bridge(&g, from, to));

    // With the zoom feeding the luminosity, a bridge between them is a loop.
    g.connect(PortRef::new(zoom, "output"), PortRef::new(luma, "input"))
        .unwrap();
    assert!(!nodes::can_bridge(&g, from, to));
}

/// A pinned input is a number, so what is offered there is what is offered at any number: a
/// reading of the picture, through a `tap`. The rows a `luminosity` would have been in are
/// not offered, because a field is not what that port takes.
#[test]
fn a_pinned_input_is_offered_the_taps_rows() {
    let mut g = Graph::new();
    let a = nodes::add_to_graph(&mut g, "add", Pos2::ZERO).unwrap();
    let slew = nodes::add_to_graph(&mut g, "slew", Pos2::ZERO).unwrap();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(a, "output"), PortRef::new(slew, "input"))
        .unwrap();

    let from = PortRef::new(cb, "output");
    let to = PortRef::new(a, "a");
    assert_eq!(
        g.can_connect(from, to),
        Err(ConnectError::TypeMismatch {
            from: PortType::VaryingColor,
            to: PortType::UniformNumber,
        }),
        "the pinned input is a number, and a color is not one"
    );
    assert!(nodes::can_bridge(&g, from, to));
    assert!(
        nodes::bridges(PortType::VaryingColor, PortType::UniformNumber)
            .iter()
            .all(|b| b.def.slug == "tap"),
        "a color becomes a number by being measured"
    );
}
