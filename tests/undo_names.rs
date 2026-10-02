// SPDX-License-Identifier: AGPL-3.0-or-later

//! Undo by name: what each step is called, the walk to any step in the ring, and the toast
//! that says what an undo changed and offers to go there.

mod common;

use common::{add_on, connect, two_tabs};
use emath::Pos2;
use supersilvia::app::Go;
use supersilvia::graph::{ControlValue, PortRef};
use supersilvia::{App, Command};

fn add(app: &mut App, slug: &'static str) -> supersilvia::graph::NodeId {
    let workspace = app.graph().default_workspace();
    add_on(app, slug, workspace)
}

/// Each command is named for what a person did: the node by its kind's label, a control by
/// its row's, a selection by how many.
#[test]
fn a_step_is_named_for_what_was_done() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    assert_eq!(app.undo_name().as_deref(), Some("Add Checkerboard"));
    let b = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");

    connect(&mut app, (a, "output"), (out, "input"));
    assert_eq!(app.undo_name().as_deref(), Some("Connect"));

    app.apply(Command::SetControl {
        node: a,
        key: "frequency",
        value: ControlValue::Float(9.0),
    })
    .unwrap();
    assert_eq!(
        app.undo_name().as_deref(),
        Some("Change Checkerboard Frequency")
    );

    app.apply(Command::MoveNodes {
        moves: vec![(a, Pos2::new(10.0, 0.0)), (b, Pos2::new(20.0, 0.0))],
    })
    .unwrap();
    assert_eq!(app.undo_name().as_deref(), Some("Move 2 nodes"));

    app.apply(Command::DisconnectEdge {
        from: PortRef::new(a, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    assert_eq!(app.undo_name().as_deref(), Some("Disconnect"));

    // A node that is gone is named from the graph it was deleted out of.
    app.apply(Command::RemoveNodes(vec![out])).unwrap();
    assert_eq!(app.undo_name().as_deref(), Some("Delete Output"));
    app.apply(Command::RemoveNodes(vec![a, b])).unwrap();
    assert_eq!(app.undo_name().as_deref(), Some("Delete 2 nodes"));
    assert_eq!(app.redo_name(), None);

    // And a redo is named as the undo was, from the other side.
    assert!(app.undo());
    assert_eq!(app.redo_name().as_deref(), Some("Delete 2 nodes"));
    assert_eq!(app.undo_name().as_deref(), Some("Delete Output"));
}

/// The history lists what can be undone oldest first and what can be redone next first, and
/// the walk to a step lands where that step left the graph — in one step whatever the
/// distance, with one toast for it.
#[test]
fn the_history_walks_to_any_step() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    connect(&mut app, (a, "output"), (out, "input"));
    app.apply(Command::RemoveNodes(vec![a])).unwrap();

    let (undo, redo) = app.step_names();
    assert_eq!(
        undo,
        [
            "Add Checkerboard",
            "Add Output",
            "Connect",
            "Delete Checkerboard"
        ]
    );
    assert!(redo.is_empty());

    // Back to before the cable.
    assert!(app.travel_to(2));
    assert_eq!(app.graph().len(), 2);
    assert!(app.graph().connections().is_empty());
    let (undo, redo) = app.step_names();
    assert_eq!(undo, ["Add Checkerboard", "Add Output"]);
    assert_eq!(
        redo,
        ["Connect", "Delete Checkerboard"],
        "the next redo first"
    );
    assert_eq!(
        app.toast(),
        Some("Undid 2 steps, through Connect"),
        "one toast for the walk"
    );

    // As far back as undo goes, then all the way forward.
    assert!(app.travel_to(0));
    assert!(app.graph().is_empty());
    assert!(app.travel_to(4));
    assert_eq!(app.graph().len(), 1, "the delete is redone");
    assert_eq!(
        app.toast(),
        Some("Redid 4 steps, through Delete Checkerboard")
    );
    assert!(!app.travel_to(4), "already there");
}

/// An undo whose change is on another workspace offers to go there: to the node it touched,
/// or to the workspace where the node it named is gone.
#[test]
fn an_undo_elsewhere_offers_to_go_there() {
    let (mut app, first, second) = two_tabs();
    let node = add_on(&mut app, "checkerboard", second);
    app.apply(Command::SetControl {
        node,
        key: "frequency",
        value: ControlValue::Float(3.0),
    })
    .unwrap();
    assert_eq!(app.active_workspace(), Some(first));

    let kept = app.undo_len() - 1;
    assert!(app.travel_to(kept));
    assert_eq!(app.toast(), Some("Undid Change Checkerboard Frequency"));
    assert_eq!(app.toast_goes(), Some(Go::Node(second, node)));

    // Undoing the add takes the node away, so what is left to go to is its workspace.
    let kept = app.undo_len() - 1;
    assert!(app.travel_to(kept));
    assert!(app.graph().get(node).is_none());
    assert_eq!(app.toast(), Some("Undid Add Checkerboard"));
    assert_eq!(app.toast_goes(), Some(Go::Workspace(second)));
}
