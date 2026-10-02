// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: pure logic. No GPU, no egui, no window. Runs in milliseconds.
//!
//! Tests build graphs with `apply` calls rather than simulated drags, and the WGSL
//! snapshots of what they compile to live here.

use std::fmt::Write as _;

use supersilvia::graph::Node;
use supersilvia::{App, Command, CommandError, Graph, NodeId};

/// Deterministic text rendering of a graph, for snapshotting.
fn describe(graph: &Graph) -> String {
    let mut out = format!("nodes: {}\n", graph.len());
    for (id, Node { def, pos, .. }) in graph.iter() {
        writeln!(out, "  {}{id} @ ({}, {})", def.slug, pos.x, pos.y).unwrap();
    }
    out
}

/// A graph built only through the command bus, snapshotted. Three `apply` calls instead of
/// thirty simulated drags.
#[test]
fn add_nodes_through_the_command_bus() {
    let mut app = App::headless();
    for (slug, x, y) in [
        ("checkerboard", 0.0, 0.0),
        ("output", 240.0, 0.0),
        ("checkerboard", 0.0, 160.0),
    ] {
        app.apply(Command::AddNode {
            slug,
            at: emath::pos2(x, y),
            workspace: app.graph().default_workspace(),
        })
        .expect("AddNode cannot fail");
    }

    assert_eq!(app.history().len(), 3);
    insta::assert_snapshot!(describe(app.graph()));
}

#[test]
fn removing_a_node_that_is_gone_is_an_error_not_a_panic() {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "output",
        at: emath::Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let id = app.graph().iter().map(|(id, _)| id).next().unwrap();

    assert_eq!(app.apply(Command::RemoveNodes(vec![id])), Ok(()));
    assert_eq!(
        app.apply(Command::RemoveNodes(vec![id])),
        Err(CommandError::NoSuchNode(id)),
    );

    // A failed command must not reach the history, or undo would replay it.
    assert_eq!(app.history().len(), 2);
    assert!(app.graph().is_empty());
}

#[test]
fn node_id_is_not_reused_after_removal() {
    let mut graph = Graph::new();
    let workspaces = std::collections::BTreeSet::from([graph.default_workspace()]);
    let checkerboard = supersilvia::nodes::find("checkerboard").expect("in the registry");
    let first = graph.add_node(
        checkerboard,
        emath::Pos2::ZERO,
        vec![],
        vec![],
        workspaces.clone(),
    );
    graph.remove_node(first);
    let second = graph.add_node(checkerboard, emath::Pos2::ZERO, vec![], vec![], workspaces);

    // The id counter never rewinds, so a stale id can never address a live node.
    // Commands and save files depend on this.
    assert_ne!(first, second);
    assert!(graph.get(first).is_none());
    assert!(graph.get(second).is_some());
    let _: NodeId = second;
}

/// A duplicate keeps the layout its definition declares.
///
/// Regions and width are read through `Node::def`, so a copy has them by holding the same
/// definition. When they were stamped onto the `Node` instead, a `Duplicate` that built its
/// copy with `add_node` alone skipped the stamp, and a duplicated adsr came back 200 wide
/// with no trace band under it — the same node, drawn as a different one. Both nodes here are
/// asserted to declare something first, or the test would pass on two nodes that declare
/// nothing.
#[test]
fn a_duplicate_keeps_the_region_its_definition_declares() {
    let mut app = App::headless();
    for slug in ["adsr", "audioin"] {
        let id = add(&mut app, slug);
        let before = app.graph().get(id).expect("added").clone();
        assert!(
            !before.def.regions.is_empty(),
            "{slug} is here because it declares a region"
        );
        app.apply(Command::Duplicate {
            nodes: vec![id],
            offset: emath::vec2(24.0, 24.0),
        })
        .unwrap();
        let copy = app.graph().iter().map(|(i, _)| i).max().expect("a copy");
        assert_ne!(copy, id);
        let copy = app.graph().get(copy).expect("a copy");
        assert_eq!(
            copy.def.regions.len(),
            before.def.regions.len(),
            "{slug}: the regions it declares"
        );
        assert_eq!(
            copy.def.width, before.def.width,
            "{slug}: the declared width"
        );
    }
}

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: emath::Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// A `Bridge` whose second cable cannot land leaves nothing behind, not even an id.
///
/// The conversion menu offers only what `nodes::can_bridge` allows, so this is the bus
/// holding its own line rather than a state a hand can reach: a command naming a port the
/// bridge does not have is refused whole, leaving no half-wired node, no undo step, and a
/// counter that hands the next node the id this one would have had.
#[test]
fn a_bridge_that_cannot_be_wired_leaves_nothing_behind() {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    for slug in ["checkerboard", "zoom"] {
        app.apply(Command::AddNode {
            slug,
            at: emath::Pos2::ZERO,
            workspace,
        })
        .unwrap();
    }
    let ids: Vec<NodeId> = app.graph().iter().map(|(id, _)| id).collect();
    let (cb, zoom) = (ids[0], ids[1]);
    let counter = app.graph().next_node_id();

    let refused = app.apply(Command::Bridge {
        from: supersilvia::graph::PortRef::new(cb, "output"),
        to: supersilvia::graph::PortRef::new(zoom, "zoom"),
        slug: "luminosity",
        inputs: &["input"],
        output: "nope",
        at: emath::Pos2::ZERO,
        workspace,
    });

    assert!(
        matches!(refused, Err(CommandError::Refused(_))),
        "{refused:?}"
    );
    assert_eq!(app.graph().len(), 2, "the node it added is gone");
    assert!(
        app.graph().connections().is_empty(),
        "so is its first cable"
    );
    assert_eq!(app.history().len(), 2, "a failed command is not an edit");
    assert_eq!(app.graph().next_node_id(), counter, "and spent no id");
    assert_eq!(add(&mut app, "checkerboard"), NodeId(counter + 1));
}

/// An option takes only a value its definition offers, on the bus as in a file: the loader
/// drops one it does not know, so the bus refusing it is what keeps a save and an open from
/// disagreeing about a node.
#[test]
fn an_option_refuses_a_value_its_definition_does_not_offer() {
    let mut app = App::headless();
    let out = add(&mut app, "output");
    let key = "supersampling";
    let offered = supersilvia::nodes::find("output")
        .and_then(|d| d.option(key))
        .expect("an Output has a supersampling");
    assert!(
        !offered.choices.is_empty() && offered.placeholder.is_none() && offered.found.is_none(),
        "a closed list, so there is a value it does not offer"
    );
    let had = app.graph().get(out).expect("added").options[key].clone();
    let steps = app.undo_len();

    assert_eq!(
        app.apply(Command::SetOption {
            node: out,
            key,
            value: "not-a-choice".to_string(),
        }),
        Err(CommandError::NoSuchChoice(out, key)),
    );
    assert_eq!(app.graph().get(out).expect("added").options[key], had);
    assert_eq!(app.undo_len(), steps, "a refusal is not an edit");

    let (choice, _) = offered.choices[offered.choices.len() - 1];
    app.apply(Command::SetOption {
        node: out,
        key,
        value: choice.to_string(),
    })
    .expect("one it does offer");
}

/// Making a cable that is already there changes nothing, so it is no edit: refused, with no
/// step, no graph handed to the synth and no Output rebuilt. The same for an action cable,
/// which may fan in, and a data one, which the same source already feeds.
#[test]
fn a_cable_made_twice_is_refused_and_changes_nothing() {
    use supersilvia::graph::{PortRef, PortType};

    let action_port = |def: &supersilvia::nodes::NodeDef, output: bool| {
        let (ins, outs) = def.port_defs();
        (if output { outs } else { ins })
            .iter()
            .find(|p| p.ty == PortType::Action)
            .map(|p| p.key)
    };
    let sends = supersilvia::nodes::REGISTRY
        .iter()
        .find(|d| action_port(d, true).is_some())
        .expect("a kind with an action output");
    let takes = supersilvia::nodes::REGISTRY
        .iter()
        .find(|d| d.slug != sends.slug && action_port(d, false).is_some())
        .expect("another kind with an action input");

    let mut app = App::headless();
    let a = add(&mut app, sends.slug);
    let b = add(&mut app, takes.slug);
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    let action = (
        PortRef::new(a, action_port(sends, true).expect("found above")),
        PortRef::new(b, action_port(takes, false).expect("found above")),
    );
    let data = (PortRef::new(cb, "output"), PortRef::new(out, "input"));

    for (from, to) in [action, data] {
        app.apply(Command::Connect { from, to })
            .expect("the first time");
        let _ = app.take_recompiles();
        let cables = app.graph().connections().to_vec();
        let (steps, generation) = (app.undo_len(), app.graph_generation());

        assert_eq!(
            app.apply(Command::Connect { from, to }),
            Err(CommandError::AlreadyConnected { from, to }),
        );
        assert_eq!(app.graph().connections(), cables.as_slice());
        assert_eq!(app.undo_len(), steps, "no step");
        assert_eq!(app.graph_generation(), generation, "nothing published");
        assert!(!app.needs_recompile(to.node), "nothing rebuilt");
    }
}
