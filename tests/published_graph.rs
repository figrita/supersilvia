// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the graph the synth ticks is the editor's, handed over on an edit and on nothing
//! else.
//!
//! The synth holds the graph as it stood after the last edit, and the editor never writes a
//! graph the synth holds — an edit copies it first — so what a tick sees cannot change under
//! it. `App::publish_graph` is the one place it crosses, and these hold both halves of what
//! that has to be true of: an edit a tick could see is across by the next tick, an undo puts
//! the old graph back for the tick after it, a refused command leaves both graphs where they
//! were, and a frame that edits nothing — or edits only layout — crosses nothing. See
//! `proposals/deterministic-loop.md`.
//!
//! **What "crossed nothing" is counted with.** `App::graph_generation`: one per graph handed
//! over, so an unchanged count is a frame that sent nothing.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .expect("slug is in the registry");
    app.graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

#[test]
fn a_node_added_is_ticked_on_the_next_frame() {
    let mut app = App::headless();
    let clock = add(&mut app, "time");
    let seconds = PortRef::new(clock, "seconds");
    assert_eq!(
        app.uniform(seconds),
        None,
        "nothing has ticked over the new graph yet"
    );
    app.tick(FRAME);
    assert!(
        app.uniform(seconds).is_some(),
        "the tick ran over the graph the command left, not the one before it"
    );
}

#[test]
fn undo_puts_the_old_graph_back_for_the_next_tick() {
    let mut app = App::headless();
    let clock = add(&mut app, "time");
    let seconds = PortRef::new(clock, "seconds");
    for _ in 0..10 {
        app.tick(FRAME);
    }
    assert!(
        app.uniform(seconds).is_some(),
        "it published while it was there"
    );

    assert!(app.undo(), "adding a node is an undo step");
    app.tick(FRAME);
    assert_eq!(
        app.uniform(seconds),
        None,
        "the node is out of the synth's graph, so the tick dropped what it published"
    );

    assert!(app.redo(), "and back");
    app.tick(FRAME);
    assert!(
        app.uniform(seconds).is_some(),
        "the restored graph is the one being ticked"
    );
}

#[test]
fn a_control_scrub_crosses_on_every_command() {
    let mut app = App::headless();
    let clock = add(&mut app, "mastergear");
    let cycles = PortRef::new(clock, "cycles");
    // A scrub is a command a frame while the hand moves; each one publishes a graph.
    for step in 1..=10i16 {
        app.apply(Command::SetControl {
            node: clock,
            key: "length",
            value: ControlValue::Float(1.0 / f32::from(step)),
        })
        .expect("length is the node's own control");
        app.tick(FRAME);
    }
    let fast = app.uniform(cycles).expect("it has been ticking");
    assert!(
        fast > 10.0 * FRAME,
        "the later lengths reached the tick rather than the first one: {fast}"
    );
}

/// A command that fails leaves the editor's graph alone, so there is nothing to hand over
/// — and nothing half-applied on either side of the seam. `ClearRange` is the one that has
/// two refusals with a write between them.
#[test]
fn a_refused_command_leaves_the_two_graphs_identical() {
    let mut app = App::headless();
    let note = add(&mut app, "ratiogear");
    let published = app.graph_generation();
    let before = format!("{:?}", app.graph());

    let err = app
        .apply(Command::ClearRange {
            node: note,
            key: "p",
        })
        .expect_err("nothing has set a range on that control");
    assert!(matches!(
        err,
        supersilvia::command::CommandError::NoSuchKey(..)
    ));
    assert_eq!(
        before,
        format!("{:?}", app.graph()),
        "a refused command wrote nothing"
    );
    assert_eq!(published, app.graph_generation(), "and handed nothing over");
}

/// A node drag is a command a frame and the tick reads no position, so it crosses nothing.
#[test]
fn a_move_crosses_nothing_and_a_control_crosses() {
    let mut app = App::headless();
    let clock = add(&mut app, "ratiogear");
    let published = app.graph_generation();

    app.apply(Command::MoveNodes {
        moves: vec![(clock, Pos2::new(40.0, 40.0))],
    })
    .expect("the node is there to move");
    assert_eq!(
        published,
        app.graph_generation(),
        "a position is not something a tick can see"
    );

    app.apply(Command::SetControl {
        node: clock,
        key: "p",
        value: ControlValue::Float(3.0),
    })
    .expect("p is the node's own control");
    assert_ne!(
        published,
        app.graph_generation(),
        "a control is read by the tick, so it crosses"
    );
}

#[test]
fn a_frame_with_no_edit_copies_no_graph() {
    let mut app = App::headless();
    add(&mut app, "time");
    let published = app.graph_generation();
    for _ in 0..100 {
        app.tick(FRAME);
    }
    assert_eq!(
        published,
        app.graph_generation(),
        "the graph crosses per edit, not per frame"
    );

    add(&mut app, "ratiogear");
    assert_ne!(published, app.graph_generation(), "and an edit makes one");
}
