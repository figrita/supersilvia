// SPDX-License-Identifier: AGPL-3.0-or-later

//! Undo is a snapshot of the graph, not an inverse of a command.
//!
//! These tests lean on that: the hard cases for an inverse-command scheme — restoring a node
//! together with every edge that touched it, restoring a connection that was silently
//! replaced — are the ones that must work.

use emath::Pos2;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use supersilvia::graph::{ControlRange, ControlValue, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::{App, Command, CommandError};

const C: Pos2 = Pos2::ZERO;

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: C,
        workspace,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

fn wire(app: &mut App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    app.apply(Command::Connect {
        from: PortRef::new(from.0, from.1),
        to: PortRef::new(to.0, to.1),
    })
    .unwrap();
}

#[test]
fn undo_restores_a_deleted_node_with_its_edges_and_controls() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (out, "input"));
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(23.0),
    })
    .unwrap();

    app.apply(Command::RemoveNodes(vec![cb])).unwrap();
    assert!(app.graph().get(cb).is_none());
    assert_eq!(app.graph().connections().len(), 0);

    assert!(app.undo());

    // The node is back under the same id — which is what lets every command recorded before
    // the deletion still address it.
    let node = app.graph().get(cb).expect("node restored");
    assert_eq!(
        node.controls.get("frequency"),
        Some(&ControlValue::Float(23.0)),
        "the control value came back with the node"
    );
    assert_eq!(
        app.graph().connections().len(),
        1,
        "the edge into the Output came back too"
    );
}

/// **A time mode switched is one step, its dropped cable with it.** A Perlin looping on a
/// gear's Cycles switched to Free drops the gear's cable from its Time, since a growing count
/// would race off as a speed; one undo puts the mode and the cable back together, and one redo
/// takes them both again. Back in Loop mode, an LFO cabled into Speed is dropped the same way.
/// A cable cannot land on the row a mode puts away at all.
#[test]
fn a_time_mode_switched_drops_the_cable_it_puts_away_in_one_step() {
    let mut app = App::headless();
    let gear = add(&mut app, "mastergear");
    let lfo = add(&mut app, "oscillator");
    let perlin = add(&mut app, "perlin");
    let mode = |app: &App| app.graph().get(perlin).unwrap().options["clockMode"].clone();
    let time = PortRef::new(perlin, "clock");
    let speed = PortRef::new(perlin, "speed");
    assert_eq!(mode(&app), "free", "a new node runs free");
    assert!(matches!(
        app.apply(Command::Connect {
            from: PortRef::new(gear, "cycles"),
            to: time,
        }),
        Err(CommandError::Refused(
            supersilvia::graph::ConnectError::Inactive(_)
        ))
    ));

    let set = |app: &mut App, value: &str| {
        app.apply(Command::SetOption {
            node: perlin,
            key: "clockMode",
            value: value.to_string(),
        })
        .unwrap();
    };
    set(&mut app, "loop");
    wire(&mut app, (gear, "cycles"), (perlin, "clock"));
    set(&mut app, "free");
    assert_eq!(mode(&app), "free");
    assert_eq!(
        app.graph().source_of(time),
        None,
        "the switch dropped the gear"
    );

    assert!(app.undo());
    assert_eq!(mode(&app), "loop", "one undo is the mode");
    assert_eq!(
        app.graph().source_of(time),
        Some(PortRef::new(gear, "cycles")),
        "and the cable with it"
    );
    assert!(app.redo());
    assert_eq!(
        (mode(&app).as_str(), app.graph().source_of(time)),
        ("free", None)
    );

    wire(&mut app, (lfo, "output"), (perlin, "speed"));
    set(&mut app, "loop");
    assert_eq!(
        app.graph().source_of(speed),
        None,
        "Loop drops a Speed's cable"
    );
    assert!(app.undo());
    assert_eq!(
        app.graph().source_of(speed),
        Some(PortRef::new(lfo, "output"))
    );
}

#[test]
fn undo_restores_a_connection_that_was_replaced() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let b = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");

    wire(&mut app, (a, "output"), (out, "input"));
    // A data input takes one connection: this silently replaces the first.
    wire(&mut app, (b, "output"), (out, "input"));
    assert_eq!(
        app.graph().source_of(PortRef::new(out, "input")),
        Some(PortRef::new(b, "output"))
    );

    assert!(app.undo());
    assert_eq!(
        app.graph().source_of(PortRef::new(out, "input")),
        Some(PortRef::new(a, "output")),
        "the replaced connection is what undo has to bring back"
    );
}

#[test]
fn a_drag_is_one_undo_step() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let start = app.graph().get(cb).unwrap().pos;

    for i in 1..=60 {
        app.apply(Command::MoveNodes {
            moves: vec![(cb, Pos2::new(i as f32, 0.0))],
        })
        .unwrap();
    }
    assert_eq!(app.graph().get(cb).unwrap().pos, Pos2::new(60.0, 0.0));
    assert_eq!(app.history().len(), 2, "add, then one coalesced move");

    assert!(app.undo());
    assert_eq!(
        app.graph().get(cb).unwrap().pos,
        start,
        "one undo steps over the whole drag"
    );
}

/// The log names a drag by where it ended, and a redo of the drag logs it the same way
/// rather than as its first frame.
#[test]
fn a_redone_drag_is_logged_as_the_drag_that_ended() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    for i in 1..=10 {
        app.apply(Command::MoveNodes {
            moves: vec![(cb, Pos2::new(i as f32, 0.0))],
        })
        .unwrap();
    }
    let last = |app: &App| match app.history().last() {
        Some(Command::MoveNodes { moves }) => moves.clone(),
        other => panic!("the log ends in the drag, not {other:?}"),
    };
    let ended = vec![(cb, Pos2::new(10.0, 0.0))];
    assert_eq!(last(&app), ended);

    assert!(app.undo());
    assert!(app.redo());
    assert_eq!(app.history().len(), 2, "add, then one coalesced move");
    assert_eq!(last(&app), ended, "the redo logs the drag's last frame");
    assert_eq!(app.graph().get(cb).unwrap().pos, Pos2::new(10.0, 0.0));
}

#[test]
fn a_scrub_is_one_undo_step() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    for i in 1..=30 {
        app.apply(Command::SetControl {
            node: cb,
            key: "frequency",
            value: ControlValue::Float(i as f32),
        })
        .unwrap();
    }
    assert!(app.undo());
    assert_eq!(
        app.graph().get(cb).unwrap().controls.get("frequency"),
        Some(&ControlValue::Float(8.0)),
        "back to the definition's default, not to 29"
    );
}

#[test]
fn redo_replays_and_a_new_edit_discards_it() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    add(&mut app, "output");
    assert_eq!(app.graph().len(), 2);

    assert!(app.undo());
    assert_eq!(app.graph().len(), 1);
    assert!(app.can_redo());

    assert!(app.redo());
    assert_eq!(app.graph().len(), 2, "redo puts it back");

    assert!(app.undo());
    // A fresh edit from a rewound state makes the undone future unreachable.
    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(9.0, 9.0))],
    })
    .unwrap();
    assert!(!app.can_redo());
}

#[test]
fn undo_and_redo_bottom_out_rather_than_panicking() {
    let mut app = App::headless();
    assert!(!app.undo());
    assert!(!app.redo());
    add(&mut app, "output");
    assert!(app.undo());
    assert!(!app.undo());
}

#[test]
fn a_refused_command_leaves_no_undo_step() {
    let mut app = App::headless();
    add(&mut app, "output");
    let before = app.history().len();

    assert!(
        app.apply(Command::RemoveNodes(vec![NodeId(999)])).is_err(),
        "no such node"
    );
    assert_eq!(app.history().len(), before, "a failure is not an edit");
    assert!(app.undo());
    assert!(!app.can_undo(), "only the add was undoable");
}

#[test]
fn undo_marks_every_output_for_recompile() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (out, "input"));
    app.take_recompiles();

    assert!(app.undo());
    // The graph was replaced wholesale, so no shader that referred to the old one stands.
    assert!(
        app.needs_recompile(out),
        "a restored graph invalidates every Output"
    );
}

// ---------------------------------------------------------------- gesture bookkeeping

/// A gesture is coalesced against the gesture in progress, not against the top of the log.
/// After an undo the top of the log belongs to a step that is no longer being extended, so
/// coalescing into it drops the new edit's undo step entirely.
#[test]
fn an_edit_after_an_undo_is_its_own_step() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(1.0, 0.0))],
    })
    .unwrap();
    add(&mut app, "output");
    assert!(app.undo(), "undo the Output away");

    // The top of the log is now the MoveNode from before the Output was added.
    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(5.0, 5.0))],
    })
    .unwrap();

    assert!(app.undo(), "the second drag has its own step");
    assert_eq!(
        app.graph().get(cb).unwrap().pos,
        Pos2::new(1.0, 0.0),
        "one undo goes back to where the first drag left it, not to the origin"
    );
}

/// The same shape, from the other side: an edit that coalesces must still clear the redo
/// stack, or redo restores a snapshot from before it and throws the edit away.
#[test]
fn an_edit_after_an_undo_leaves_nothing_to_redo() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(1.0, 0.0))],
    })
    .unwrap();
    add(&mut app, "output");
    assert!(app.undo());
    assert!(app.can_redo(), "the Output is redoable at this point");

    app.apply(Command::MoveNodes {
        moves: vec![(cb, Pos2::new(5.0, 5.0))],
    })
    .unwrap();

    assert!(!app.can_redo(), "a new edit discards the redo stack");
    assert_eq!(app.graph().len(), 1, "the Output did not come back");
    assert_eq!(app.graph().get(cb).unwrap().pos, Pos2::new(5.0, 5.0));
}

/// Collapsing is presentation, but the *state* is document data: it survives undo, and it
/// costs one step like any other edit.
#[test]
fn collapsing_a_node_is_one_undo_step_and_touches_nothing_else() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (a, "output"), (out, "input"));
    let before = app.graph().get(a).cloned().expect("the node");

    app.apply(Command::SetCollapsed {
        nodes: vec![a],
        collapsed: true,
    })
    .expect("collapse");
    assert!(app.graph().get(a).expect("the node").collapsed);
    // The cable is untouched: collapsing hides rows, it does not disconnect anything.
    assert_eq!(app.graph().connections().len(), 1);

    app.undo();
    assert_eq!(app.graph().get(a), Some(&before), "undo restores the node");
    assert_eq!(app.graph().connections().len(), 1);
}

/// Dragging a selection is one undo step, not one per node per frame.
#[test]
fn moving_several_nodes_is_one_undo_step() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let b = add(&mut app, "output");
    let before = app.history().len();

    for i in 1..=30 {
        app.apply(Command::MoveNodes {
            moves: vec![
                (a, Pos2::new(i as f32, 0.0)),
                (b, Pos2::new(i as f32, 50.0)),
            ],
        })
        .unwrap();
    }
    assert_eq!(app.history().len(), before + 1, "a drag is one step");

    // Grabbing a different set is a new gesture, so it opens its own step.
    app.apply(Command::MoveNodes {
        moves: vec![(a, Pos2::new(99.0, 0.0))],
    })
    .unwrap();
    assert_eq!(
        app.history().len(),
        before + 2,
        "a new set is a new gesture"
    );
}

/// A move naming a node that is gone leaves the rest of the selection alone.
#[test]
fn a_move_with_a_stale_node_moves_nothing() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let was = app.graph().get(a).expect("the node").pos;

    assert!(
        app.apply(Command::MoveNodes {
            moves: vec![(a, Pos2::new(5.0, 5.0)), (NodeId(999), Pos2::ZERO)],
        })
        .is_err()
    );
    assert_eq!(app.graph().get(a).expect("the node").pos, was);
}

// ---------------------------------------------------------------- reset and disconnect

/// `Reset controls` is the number control's `R` for a whole selection: the value and the
/// range together, on every control, in one undo step.
#[test]
fn resetting_controls_restores_defaults_and_clears_ranges() {
    let mut app = App::headless();
    let a = add(&mut app, "checkerboard");
    let b = add(&mut app, "oscillator");
    app.apply(Command::SetControl {
        node: a,
        key: "frequency",
        value: ControlValue::Float(23.0),
    })
    .unwrap();
    app.apply(Command::SetRange {
        node: a,
        key: "frequency",
        range: ControlRange {
            min: 1.0,
            max: 4.0,
            step: 0.5,
        },
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: b,
        key: "amplitude",
        value: ControlValue::Float(7.0),
    })
    .unwrap();
    let steps = app.undo_len();

    app.apply(Command::ResetControls(vec![a, b]))
        .expect("reset");

    let node = app.graph().get(a).expect("the node");
    assert_eq!(
        node.controls["frequency"],
        ControlValue::Float(8.0),
        "the definition's default"
    );
    assert!(node.values.is_empty(), "the node's own range went with it");
    assert_eq!(
        app.graph().get(b).expect("the node").controls["amplitude"],
        ControlValue::Float(1.0),
    );
    assert_eq!(
        app.undo_len(),
        steps + 1,
        "one step for the whole selection"
    );

    assert!(app.undo());
    let node = app.graph().get(a).expect("the node");
    assert_eq!(node.controls["frequency"], ControlValue::Float(4.0));
    assert_eq!(
        node.values["frequency"].range().expect("a range"),
        ControlRange {
            min: 1.0,
            max: 4.0,
            step: 0.5,
        },
        "the range came back with the value"
    );
    assert_eq!(
        app.graph().get(b).expect("the node").controls["amplitude"],
        ControlValue::Float(7.0),
    );
}

/// An option is structural, so a reset of the parameters may not carry it: rebuilding a
/// shader is a different command.
#[test]
fn resetting_controls_leaves_options_alone() {
    let mut app = App::headless();
    let layer = add(&mut app, "layerblend");
    app.apply(Command::SetOption {
        node: layer,
        key: "blend_mode",
        value: "multiply".to_string(),
    })
    .unwrap();

    app.apply(Command::ResetControls(vec![layer]))
        .expect("reset");
    assert_eq!(
        app.graph().get(layer).expect("the node").options["blend_mode"],
        "multiply"
    );
}

/// Every cable touching the selection goes, whichever end of it is in there.
#[test]
fn disconnecting_all_takes_the_cables_on_both_sides() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let zoom = add(&mut app, "zoom");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (zoom, "input"));
    wire(&mut app, (zoom, "output"), (out, "input"));
    let steps = app.undo_len();

    app.apply(Command::DisconnectAll(vec![zoom]))
        .expect("disconnect");
    assert_eq!(app.graph().connections().len(), 0, "in and out, both");
    assert_eq!(app.graph().len(), 3, "the nodes stayed");
    assert_eq!(app.undo_len(), steps + 1);

    assert!(app.undo());
    assert_eq!(app.graph().connections().len(), 2);
}

/// Both commands check every id before they write any, so one stale node leaves the rest of
/// the selection exactly as it was.
#[test]
fn a_reset_or_a_disconnect_with_a_stale_node_changes_nothing() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (out, "input"));
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(23.0),
    })
    .unwrap();
    let steps = app.undo_len();

    for command in [
        Command::ResetControls(vec![cb, NodeId(999)]),
        Command::DisconnectAll(vec![cb, NodeId(999)]),
    ] {
        assert_eq!(
            app.apply(command.clone()),
            Err(CommandError::NoSuchNode(NodeId(999))),
            "{command:?}"
        );
    }
    assert_eq!(
        app.graph().get(cb).expect("the node").controls["frequency"],
        ControlValue::Float(23.0),
    );
    assert_eq!(app.graph().connections().len(), 1);
    assert_eq!(app.undo_len(), steps, "and neither left an undo step");
}

/// Naming no nodes asks for nothing, so it is not an edit and there is nothing to undo.
#[test]
fn a_reset_or_a_disconnect_of_nothing_is_not_a_step() {
    let mut app = App::headless();
    add(&mut app, "checkerboard");
    let steps = app.undo_len();
    let entries = app.history().len();

    app.apply(Command::ResetControls(vec![])).expect("no-op");
    app.apply(Command::DisconnectAll(vec![])).expect("no-op");

    assert_eq!(app.undo_len(), steps);
    assert_eq!(app.history().len(), entries);
}

/// Duplicating a subgraph copies the values and the cables *inside* it, and nothing else.
#[test]
fn duplicate_keeps_internal_cables_and_control_values() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let zoom = add(&mut app, "zoom");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (zoom, "input"));
    wire(&mut app, (zoom, "output"), (out, "input"));
    app.apply(Command::SetControl {
        node: zoom,
        key: "zoom",
        value: ControlValue::Float(2.5),
    })
    .unwrap();

    // Copy the pair, not the Output they feed.
    app.apply(Command::Duplicate {
        nodes: vec![cb, zoom],
        offset: emath::vec2(40.0, 40.0),
    })
    .unwrap();

    let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 5, "two copies were made");
    let new_cb = ids[3];
    let new_zoom = ids[4];

    // The value came with it.
    assert_eq!(
        app.graph().get(new_zoom).unwrap().controls.get("zoom"),
        Some(&ControlValue::Float(2.5)),
        "the copy lost its control value"
    );
    // Offset, not stacked on the original.
    assert_eq!(
        app.graph().get(new_cb).unwrap().pos,
        app.graph().get(cb).unwrap().pos + emath::vec2(40.0, 40.0)
    );

    // The cable between the two copies exists...
    assert_eq!(
        app.graph().source_of(PortRef::new(new_zoom, "input")),
        Some(PortRef::new(new_cb, "output")),
        "the internal cable was not copied"
    );
    // ...and the one that left the set did not follow, so the Output still has exactly one
    // source and it is still the original.
    assert_eq!(
        app.graph().source_of(PortRef::new(out, "input")),
        Some(PortRef::new(zoom, "output")),
        "duplicating stole or doubled an outside connection"
    );

    // Snapshot undo covers it with no undo-specific code.
    assert!(app.undo());
    assert_eq!(app.graph().len(), 3);
}

/// A duplicate is what the next drag moves.
#[test]
fn duplicate_selects_the_copies() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::Duplicate {
        nodes: vec![cb],
        offset: emath::vec2(10.0, 10.0),
    })
    .unwrap();

    let made: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
    let copy = *made.last().unwrap();
    assert_eq!(
        app.canvas_selection(),
        vec![copy],
        "the copy is not the selection"
    );
}

// ---------------------------------------------------------------- workspaces

fn workspaces(app: &App) -> Vec<WorkspaceId> {
    app.graph().workspaces().iter().map(|w| w.id).collect()
}

fn on(app: &App, node: NodeId) -> BTreeSet<WorkspaceId> {
    app.graph().get(node).unwrap().workspaces.clone()
}

/// Add a second workspace and hand back both ids.
fn two_workspaces(app: &mut App) -> (WorkspaceId, WorkspaceId) {
    let first = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Workspace 2".into(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        // These tests want a bare workspace; the app seeds a video tab with a patch.
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let second = *workspaces(app).last().unwrap();
    (first, second)
}

/// Every one of the seven commands, applied, undone and redone. Snapshot undo is what buys
/// this with no undo-specific code, so what it needs is checking rather than writing.
#[test]
fn every_workspace_command_round_trips_through_undo_and_redo() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let (first, second) = two_workspaces(&mut app);
    app.apply(Command::AddWorkspace {
        name: "Workspace 3".into(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        // These tests want a bare workspace; the app seeds a video tab with a patch.
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let third = *workspaces(&app).last().unwrap();

    let cases: Vec<Command> = vec![
        Command::AddWorkspace {
            name: "Workspace 4".into(),
            kind: WorkspaceKind::Video,
            layout: supersilvia::graph::LayoutMode::default(),
            // These tests want a bare workspace; the app seeds a video tab with a patch.
            seed: supersilvia::command::Seed::Empty,
        },
        Command::RenameWorkspace {
            id: second,
            name: "renamed".into(),
        },
        Command::MoveWorkspace { id: second, to: 0 },
        Command::ShowOn {
            nodes: vec![cb],
            workspace: third,
        },
        Command::MoveTo {
            nodes: vec![cb],
            workspace: second,
        },
        Command::HideFrom {
            nodes: vec![cb],
            workspace: first,
        },
        Command::RemoveWorkspace(second),
    ];

    for command in cases {
        // Whatever the previous case left behind: the node is on both, so every command
        // below is legal from here.
        app.apply(Command::MoveTo {
            nodes: vec![cb],
            workspace: first,
        })
        .unwrap();
        app.apply(Command::ShowOn {
            nodes: vec![cb],
            workspace: second,
        })
        .unwrap();

        let before = describe(&app);
        app.apply(command.clone())
            .unwrap_or_else(|e| panic!("{command:?}: {e}"));
        let after = describe(&app);
        assert_ne!(before, after, "{command:?} changed nothing");

        assert!(app.undo(), "{command:?} left no undo step");
        assert_eq!(describe(&app), before, "undo of {command:?}");
        assert!(app.redo());
        assert_eq!(describe(&app), after, "redo of {command:?}");
        assert!(app.undo());
    }
}

/// Workspaces, node membership and the node set, as text: enough that any of the seven
/// commands changes it.
fn describe(app: &App) -> String {
    let mut out = String::new();
    for w in app.graph().workspaces() {
        writeln!(out, "{} {} {:?}", w.id, w.name, w.kind).unwrap();
    }
    for (id, node) in app.graph().iter() {
        writeln!(out, "{id} {:?}", node.workspaces).unwrap();
    }
    out
}

#[test]
fn hiding_a_node_from_its_last_workspace_is_refused_and_writes_nothing() {
    let mut app = App::headless();
    let shared = add(&mut app, "checkerboard");
    let lonely = add(&mut app, "output");
    let (first, second) = two_workspaces(&mut app);
    app.apply(Command::ShowOn {
        nodes: vec![shared],
        workspace: second,
    })
    .unwrap();

    let steps = app.undo_len();
    assert_eq!(
        app.apply(Command::HideFrom {
            nodes: vec![shared, lonely],
            workspace: first,
        }),
        Err(CommandError::NoWorkspaceLeft(lonely)),
    );
    // The whole command is checked before any of it is written, so the node that *could*
    // have been hidden was not.
    assert_eq!(on(&app, shared), BTreeSet::from([first, second]));
    assert_eq!(on(&app, lonely), BTreeSet::from([first]));
    assert_eq!(app.undo_len(), steps, "a refused command leaves no step");
}

#[test]
fn the_last_workspace_cannot_be_removed_through_the_bus() {
    let mut app = App::headless();
    add(&mut app, "checkerboard");
    let only = app.graph().default_workspace();
    assert_eq!(
        app.apply(Command::RemoveWorkspace(only)),
        Err(CommandError::LastWorkspace)
    );
    assert_eq!(app.graph().len(), 1);
}

#[test]
fn move_to_leaves_a_node_on_exactly_one_workspace() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let (first, second) = two_workspaces(&mut app);
    app.apply(Command::ShowOn {
        nodes: vec![cb],
        workspace: second,
    })
    .unwrap();
    assert_eq!(on(&app, cb), BTreeSet::from([first, second]));

    app.apply(Command::MoveTo {
        nodes: vec![cb],
        workspace: second,
    })
    .unwrap();
    assert_eq!(on(&app, cb), BTreeSet::from([second]));
}

#[test]
fn a_command_naming_a_missing_workspace_changes_nothing() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let ghost = WorkspaceId(9999);
    let steps = app.undo_len();
    for command in [
        Command::ShowOn {
            nodes: vec![cb],
            workspace: ghost,
        },
        Command::HideFrom {
            nodes: vec![cb],
            workspace: ghost,
        },
        Command::MoveTo {
            nodes: vec![cb],
            workspace: ghost,
        },
        Command::RemoveWorkspace(ghost),
        Command::RenameWorkspace {
            id: ghost,
            name: "nope".into(),
        },
        Command::MoveWorkspace { id: ghost, to: 0 },
    ] {
        assert_eq!(
            app.apply(command.clone()),
            Err(CommandError::NoSuchWorkspace(ghost)),
            "{command:?}"
        );
    }
    assert_eq!(app.undo_len(), steps);
}

/// Removing a workspace takes the nodes left on none, and one `Ctrl+Z` brings all of it
/// back — the case an inverse-command scheme would have to write by hand.
#[test]
fn removing_a_workspace_is_one_undo_step() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (out, "input"));
    let (_, second) = two_workspaces(&mut app);
    app.apply(Command::MoveTo {
        nodes: vec![cb],
        workspace: second,
    })
    .unwrap();

    app.apply(Command::RemoveWorkspace(second)).unwrap();
    assert!(app.graph().get(cb).is_none());
    assert_eq!(app.graph().connections().len(), 0);

    assert!(app.undo());
    assert!(app.graph().get(cb).is_some(), "the node came back");
    assert_eq!(app.graph().connections().len(), 1, "and so did its cable");
    assert_eq!(app.graph().workspaces().len(), 2);
}

/// A project of `n` zooms, opened through the folder so the history starts empty.
fn opened_with_zooms(n: usize, dir: &str) -> (App, std::path::PathBuf) {
    let mut app = App::headless();
    let mut graph = supersilvia::Graph::new();
    for _ in 0..n {
        supersilvia::nodes::add_to_graph(&mut graph, "zoom", Pos2::ZERO).unwrap();
    }
    let root = std::env::temp_dir().join(dir);
    std::fs::remove_dir_all(&root).ok();
    let mut project = supersilvia::project::Project::new(root.clone());
    project.save(&graph).unwrap();
    app.open_project(root.clone());
    assert_eq!(app.graph().len(), n);
    (app, root)
}

/// The ring's first cap is bytes, counted as what a step holds that its neighbour does not.
/// An edit that writes every node of a large graph holds a whole graph of its own, and 64 MB
/// of those runs out well before the 256-step depth cap.
#[test]
fn the_ring_trims_by_bytes_before_it_reaches_the_depth_cap() {
    let (mut app, root) = opened_with_zooms(900, "supersilvia-undo-bytes");

    // 250 edits of every node, none of which coalesce: the gesture ends between them.
    let ids: Vec<NodeId> = app.graph().iter().map(|(id, _)| id).collect();
    for n in 0..250 {
        #[allow(clippy::cast_precision_loss)]
        let to = Pos2::new(n as f32, 0.0);
        app.apply(Command::MoveNodes {
            moves: ids.iter().map(|id| (*id, to)).collect(),
        })
        .unwrap();
        app.end_gesture();
    }

    let steps = app.undo_len();
    assert!(
        steps < 200,
        "the bytes ran out well before the 256-step depth cap, so the byte budget is what \
         bound the ring — not this: {steps} steps"
    );
    assert!(app.can_undo());
    while app.undo() {}
    std::fs::remove_dir_all(&root).ok();
}

/// The same graph, one node an edit: each step shares the other 899 with its neighbour and
/// costs one node and a map of pointers, so it is the depth cap that binds.
#[test]
fn a_step_that_writes_one_node_costs_one_node() {
    let (mut app, root) = opened_with_zooms(900, "supersilvia-undo-shared");

    let ids: Vec<NodeId> = app.graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().cycle().take(300).enumerate() {
        #[allow(clippy::cast_precision_loss)]
        app.apply(Command::MoveNodes {
            moves: vec![(*id, Pos2::new(n as f32, 0.0))],
        })
        .unwrap();
    }

    assert_eq!(
        app.undo_len(),
        256,
        "a step is one node, so 256 of them are far inside the byte budget"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// A scrub writes one node, and the graph after it is the graph before it in every other:
/// the same allocation, not an equal copy.
#[test]
fn a_scrub_step_shares_every_untouched_node_with_the_graph_before() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let zoom = add(&mut app, "zoom");
    let other = add(&mut app, "zoom");
    wire(&mut app, (cb, "output"), (zoom, "input"));
    let before = app.graph().clone();

    app.apply(Command::SetControl {
        node: zoom,
        key: "zoom",
        value: ControlValue::Float(2.0),
    })
    .unwrap();

    let after = app.graph();
    assert!(
        !after.shares_node(&before, zoom),
        "the scrubbed node is a copy"
    );
    assert!(after.shares_node(&before, cb));
    assert!(after.shares_node(&before, other));
    assert!(
        after.shares_connections(&before),
        "and the cables are the same list"
    );

    // Undo puts the very graph back, not a copy of it.
    assert!(app.undo());
    for (id, _) in before.iter() {
        assert!(
            app.graph().shares_node(&before, id),
            "{id} came back as itself"
        );
    }
}

/// Add, undo, add: the second node is a new id. The first is named by a command in the log
/// and may be named by a redo, so handing it out again would make two nodes one.
#[test]
fn add_undo_add_hands_out_a_fresh_node_id() {
    let mut app = App::headless();
    let first = add(&mut app, "checkerboard");
    assert!(app.undo());
    assert!(app.graph().get(first).is_none());

    let second = add(&mut app, "checkerboard");
    assert!(
        second > first,
        "{second} came after {first}, not instead of it"
    );
}

/// The same for a workspace, whose id names a file and a picture on disk.
#[test]
fn add_undo_add_hands_out_a_fresh_workspace_id() {
    let mut app = App::headless();
    let (_, first) = two_workspaces(&mut app);
    assert!(app.undo());
    assert!(!app.graph().has_workspace(first));

    let (_, second) = two_workspaces(&mut app);
    assert!(second.0 > first.0, "{second:?} came after {first:?}");
}

/// Redo keeps the counter too: undo two nodes, redo one, and the next is after both.
#[test]
fn a_redo_does_not_rewind_the_counter_either() {
    let mut app = App::headless();
    let first = add(&mut app, "checkerboard");
    let second = add(&mut app, "checkerboard");
    assert!(app.undo());
    assert!(app.undo());
    assert!(app.redo());
    let third = add(&mut app, "checkerboard");
    assert!(third > second && second > first);
}

/// Undoing the connect that made a field of a number brings the diamond back.
///
/// Nothing computes the inverse: a step is a snapshot of the graph, and the effective type
/// is on the instance's own port, so it is restored with everything else.
#[test]
fn undoing_a_connect_that_made_a_field_restores_the_diamond() {
    use supersilvia::graph::PortType;

    let mut app = App::headless();
    let sum = add(&mut app, "add");
    let luma = add(&mut app, "luminosity");
    let ty = |app: &App| app.graph().get(sum).unwrap().output("output").unwrap().ty;
    assert_eq!(ty(&app), PortType::UniformNumber);

    wire(&mut app, (luma, "output"), (sum, "a"));
    assert_eq!(ty(&app), PortType::VaryingNumber);

    assert!(app.undo());
    assert_eq!(ty(&app), PortType::UniformNumber);
}

/// A conversion is one node and two cables, and therefore one step.
///
/// The gesture is a single drag that ended in a menu, so one undo has to leave the graph as
/// it was before the cable — not a node with one cable, and not three steps to walk back.
#[test]
fn a_bridge_is_one_step_for_a_node_and_two_cables() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let zoom = add(&mut app, "zoom");
    let workspace = app.graph().default_workspace();

    app.apply(Command::Bridge {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(zoom, "zoom"),
        slug: "luminosity",
        inputs: &["input"],
        output: "output",
        at: Pos2::new(120.0, 40.0),
        workspace,
    })
    .expect("a color bridges to a number through a luminosity");

    assert_eq!(app.graph().len(), 3);
    assert_eq!(app.graph().connections().len(), 2);
    let luma = app.graph().iter().map(|(id, _)| id).max().unwrap();
    assert_eq!(
        app.graph().get(luma).map(|n| n.def.slug),
        Some("luminosity")
    );
    assert_eq!(
        app.graph().get(luma).map(|n| n.pos),
        Some(Pos2::new(120.0, 40.0))
    );
    assert_eq!(app.history().len(), 3, "one command, not three");

    assert!(app.undo());
    assert_eq!(app.graph().len(), 2, "the node went with its cables");
    assert!(app.graph().connections().is_empty());

    assert!(app.redo());
    assert_eq!(app.graph().len(), 3);
    assert_eq!(app.graph().connections().len(), 2);
}

/// A casting that fans out is still one step: the `Grayscale` row lands an `rgba` and *four*
/// cables, three of them from the same output.
///
/// silvia's `float-to-color` closure, which is the reason `Command::Bridge` carries a list of
/// inputs rather than one: a hand that changes its mind about a gray presses undo once.
#[test]
fn a_fanned_out_bridge_is_one_step_for_all_of_its_cables() {
    let mut app = App::headless();
    let luma = add(&mut app, "luminosity");
    let zoom = add(&mut app, "zoom");
    let workspace = app.graph().default_workspace();

    app.apply(Command::Bridge {
        from: PortRef::new(luma, "output"),
        to: PortRef::new(zoom, "input"),
        slug: "rgba",
        inputs: &["r", "g", "b"],
        output: "output",
        at: Pos2::ZERO,
        workspace,
    })
    .expect("a field bridges to a color through an rgba");

    assert_eq!(app.graph().len(), 3);
    assert_eq!(app.graph().connections().len(), 4, "three in, one out");

    assert!(app.undo());
    assert_eq!(
        app.graph().len(),
        2,
        "one undo, the node and all four cables"
    );
    assert!(app.graph().connections().is_empty());
}

/// Undoing the connect that pinned an `add` unpins it.
///
/// Nothing computes the inverse here either: a step is a snapshot of the graph, so the
/// restored instance carries the port types the graph had worked out before the cable landed.
#[test]
fn undoing_a_connect_that_pinned_a_node_restores_its_circles() {
    use supersilvia::graph::PortType;

    let mut app = App::headless();
    let sum = add(&mut app, "add");
    let slew = add(&mut app, "slew");
    let tys = |app: &App| -> Vec<PortType> {
        app.graph()
            .get(sum)
            .unwrap()
            .inputs
            .iter()
            .map(|p| p.ty)
            .collect()
    };
    assert_eq!(
        tys(&app),
        [PortType::VaryingNumber, PortType::VaryingNumber]
    );

    wire(&mut app, (sum, "output"), (slew, "input"));
    assert_eq!(
        tys(&app),
        [PortType::UniformNumber, PortType::UniformNumber]
    );

    assert!(app.undo());
    assert_eq!(
        tys(&app),
        [PortType::VaryingNumber, PortType::VaryingNumber]
    );

    assert!(app.redo());
    assert_eq!(
        tys(&app),
        [PortType::UniformNumber, PortType::UniformNumber]
    );
}

/// A removal naming one node twice — a selection sent twice — removes it once, and is one
/// step like any other removal.
#[test]
fn removing_one_node_named_twice_removes_it_once() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    wire(&mut app, (cb, "output"), (out, "input"));
    let before = app.undo_len();

    app.apply(Command::RemoveNodes(vec![cb, cb]))
        .expect("a node that is there, twice");
    assert!(app.graph().get(cb).is_none());
    assert_eq!(app.undo_len(), before + 1);
    assert_eq!(
        app.history().last(),
        Some(&Command::RemoveNodes(vec![cb])),
        "the step names it once"
    );

    assert!(app.undo());
    assert!(app.graph().get(cb).is_some());
    assert_eq!(app.graph().connections().len(), 1, "with its cable");
}

/// The history is the ring's own list: one command per step, the one that opened it or the
/// last one its gesture wrote. A label for each step rather than a log that replays — a value
/// written and its range cleared in one gesture shows only the clearing — and it is trimmed
/// with the ring, because it is the ring.
#[test]
fn the_history_is_one_command_per_undo_step() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::SetRange {
        node: cb,
        key: "frequency",
        range: ControlRange {
            min: 0.0,
            max: 5.0,
            step: 0.1,
        },
    })
    .unwrap();
    app.end_gesture();
    app.apply(Command::SetControl {
        node: cb,
        key: "frequency",
        value: ControlValue::Float(4.0),
    })
    .unwrap();
    app.apply(Command::ClearRange {
        node: cb,
        key: "frequency",
    })
    .unwrap();
    assert_eq!(
        app.undo_len(),
        3,
        "add, range, and the value with its reset"
    );
    assert_eq!(app.history().len(), app.undo_len());
    assert!(matches!(
        app.history().last(),
        Some(Command::ClearRange { .. })
    ));

    for i in 0..300 {
        app.apply(Command::AddNode {
            slug: "number",
            at: Pos2::new(i as f32, 0.0),
            workspace: app.graph().default_workspace(),
        })
        .unwrap();
    }
    assert!(app.undo_len() < 300, "the depth cap trimmed the ring");
    assert_eq!(app.history().len(), app.undo_len());
}

/// A value the tick writes while a scrub is in progress joins the scrub's step rather than
/// splitting it: an automation's recording that stops mid-drag lands in the drag's step, and
/// one undo takes back both the drag and the recording.
#[test]
fn a_recording_landing_mid_scrub_does_not_split_it() {
    let mut app = App::headless();
    let node = add(&mut app, "automation");
    app.apply(Command::SetControl {
        node,
        key: "duration",
        value: ControlValue::Float(0.2),
    })
    .unwrap();
    app.end_gesture();
    app.press(PortRef::new(node, "record"), true);
    app.tick(1.0 / 60.0);
    app.press(PortRef::new(node, "record"), false);
    let before = app.undo_len();
    let recorded = |app: &App| {
        app.graph()
            .get(node)
            .unwrap()
            .values
            .get("recording")
            .cloned()
    };
    let had = recorded(&app);

    // A scrub that outlasts the fifth of a second the recording takes.
    for i in 0..30 {
        app.apply(Command::SetControl {
            node,
            key: "input",
            value: ControlValue::Float(i as f32 / 30.0),
        })
        .unwrap();
        app.tick(1.0 / 60.0);
    }
    assert_ne!(recorded(&app), had, "the recording landed mid-scrub");
    assert_eq!(app.undo_len(), before + 1, "one scrub, recording and all");

    // With nothing open, the next recording is a step of its own.
    app.end_gesture();
    app.press(PortRef::new(node, "record"), true);
    app.tick(1.0 / 60.0);
    app.press(PortRef::new(node, "record"), false);
    for _ in 0..30 {
        app.tick(1.0 / 60.0);
    }
    assert_eq!(
        app.undo_len(),
        before + 2,
        "a recording alone is its own step"
    );

    assert!(app.undo());
    assert!(app.undo());
    assert_eq!(
        recorded(&app),
        had,
        "one undo took the recording with the scrub"
    );
}

/// A seeded random walk of edits over the whole command bus, checking after every step what
/// a snapshot undo promises: a refused command changes nothing and leaves no step, undo and
/// redo give back exactly the graph a model of the ring predicts, an id is never handed out
/// twice, a graph an older step holds is never written through, the cable index agrees with
/// the cable list, and a save and an open give the same graph back.
///
/// Deterministic — the same seeds make the same edits on every run — and small enough to run
/// in the suite. A failure names the seed and the step, which is enough to replay it.
mod fuzz {
    use emath::{Pos2, Vec2};
    use std::collections::{BTreeMap, BTreeSet, HashSet};
    use std::fmt::Write as _;
    use std::path::PathBuf;
    use supersilvia::command::{Clip, ClipEdge, Seed};
    use supersilvia::graph::{
        ControlRange, ControlValue, Graph, LayoutMode, Node, NodeId, PortRef, Value, WorkspaceId,
        WorkspaceKind,
    };
    use supersilvia::nodes::{Control, NodeDef, REGISTRY};
    use supersilvia::project::Project;
    use supersilvia::{App, Command};

    /// SplitMix64: a whole generator in four lines, and the same numbers on every machine.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: usize) -> usize {
            if n == 0 {
                0
            } else {
                (self.next() % n as u64) as usize
            }
        }

        fn chance(&mut self, p: f64) -> bool {
            (self.next() % 10_000) as f64 / 10_000.0 < p
        }

        fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
            if v.is_empty() {
                None
            } else {
                Some(&v[self.below(v.len())])
            }
        }

        fn f(&mut self, lo: f32, hi: f32) -> f32 {
            lo + (self.next() % 1_000_000) as f32 / 1_000_000.0 * (hi - lo)
        }
    }

    /// Everything a node holds, on one line.
    fn node_line(id: NodeId, n: &Node) -> String {
        let mut s = String::new();
        let _ = write!(s, "#{id} {} {:?} ws={:?}", n.def.slug, n.pos, n.workspaces);
        for p in &n.inputs {
            let _ = write!(s, " in:{}:{:?}:{}:{}", p.key, p.ty, p.delayed, p.dual);
        }
        for p in &n.outputs {
            let _ = write!(s, " out:{}:{:?}:{}:{}", p.key, p.ty, p.delayed, p.dual);
        }
        let controls: BTreeMap<_, _> = n.controls.iter().collect();
        let _ = write!(
            s,
            " w={:?} h={:?} c={controls:?} v={:?} o={:?} col={}",
            n.dragged_width, n.dragged_height, n.values, n.options, n.collapsed
        );
        s
    }

    /// Everything in the graph. Cables in list order, or as a set where `ordered` is false,
    /// which is what a file is compared by: it keeps the cables and not the order they were
    /// made in.
    fn dump(g: &Graph, ordered: bool) -> String {
        let mut s = String::new();
        for w in g.workspaces() {
            let _ = writeln!(
                s,
                "ws {} {:?} {:?} {:?} {:?}",
                w.id, w.name, w.kind, w.blurb, w.layout
            );
        }
        for (id, n) in g.iter() {
            let _ = writeln!(s, "{}", node_line(id, n));
        }
        let mut cables: Vec<String> = g
            .connections()
            .iter()
            .map(|c| {
                format!(
                    "cable {}.{} -> {}.{}",
                    c.from.node, c.from.key, c.to.node, c.to.key
                )
            })
            .collect();
        if !ordered {
            cables.sort();
        }
        for c in cables {
            let _ = writeln!(s, "{c}");
        }
        s
    }

    fn counters(g: &Graph) -> (u32, u32) {
        (g.next_node_id(), g.next_workspace_id())
    }

    /// What the graph promises about its cables, checked by brute force from the list.
    fn check_graph(g: &Graph) -> Result<(), String> {
        let conns = g.connections();
        let mut data_into = HashSet::new();
        let mut seen = HashSet::new();
        for c in conns {
            let from = g.get(c.from.node).ok_or(format!("dangling {c:?}"))?;
            let to = g.get(c.to.node).ok_or(format!("dangling {c:?}"))?;
            let fp = from
                .output(c.from.key)
                .ok_or(format!("no output for {c:?}"))?;
            let tp = to.input(c.to.key).ok_or(format!("no input for {c:?}"))?;
            if c.from.node == c.to.node {
                return Err(format!("a node feeds itself: {c:?}"));
            }
            if fp.ty.is_data() != tp.ty.is_data() || !fp.ty.feeds(tp.ty) {
                return Err(format!("illegal cable {c:?}: {:?} into {:?}", fp.ty, tp.ty));
            }
            if tp.ty.is_data() && !data_into.insert(c.to) {
                return Err(format!("two sources into {:?}", c.to));
            }
            if !seen.insert((c.from, c.to)) {
                return Err(format!("the same cable twice: {c:?}"));
            }
        }
        for (id, n) in g.iter() {
            let into: Vec<(PortRef, PortRef)> =
                g.cables_into(id).iter().map(|c| (c.from, c.to)).collect();
            let brute: Vec<(PortRef, PortRef)> = conns
                .iter()
                .filter(|c| c.to.node == id)
                .map(|c| (c.from, c.to))
                .collect();
            if into != brute {
                return Err(format!(
                    "the index into {id}: {into:?}, the list: {brute:?}"
                ));
            }
            for p in &n.outputs {
                let from = PortRef::new(id, p.key);
                let t: Vec<PortRef> = g.targets_of(from).collect();
                let b: Vec<PortRef> = conns
                    .iter()
                    .filter(|c| c.from == from)
                    .map(|c| c.to)
                    .collect();
                if t != b {
                    return Err(format!("targets_of {from:?}: {t:?}, the list: {b:?}"));
                }
            }
            for p in &n.inputs {
                let to = PortRef::new(id, p.key);
                let s: Vec<PortRef> = g.sources_of(to).collect();
                let b: Vec<PortRef> = conns
                    .iter()
                    .filter(|c| c.to == to)
                    .map(|c| c.from)
                    .collect();
                if s != b || g.source_of(to) != b.first().copied() {
                    return Err(format!("sources_of {to:?}: {s:?}, the list: {b:?}"));
                }
            }
            if n.workspaces.is_empty() || n.workspaces.iter().any(|w| !g.has_workspace(*w)) {
                return Err(format!("node {id} on {:?}", n.workspaces));
            }
            for (k, v) in &n.controls {
                if let (ControlValue::Float(f), Some(r)) =
                    (v, supersilvia::nodes::control_range(n.def, n, k))
                    && !(r.min..=r.max).contains(f)
                {
                    return Err(format!("{id}.{k} is {f}, outside {r:?}"));
                }
            }
            if id.0 > g.next_node_id() {
                return Err(format!("the counter {} is below {id}", g.next_node_id()));
            }
        }
        if g.workspaces()
            .iter()
            .any(|w| w.id.0 > g.next_workspace_id())
        {
            return Err("the workspace counter is below a workspace".into());
        }
        Ok(())
    }

    #[derive(Debug, Clone)]
    enum Op {
        Apply(Command),
        Undo,
        Redo,
        EndGesture,
        RoundTrip,
        Export(WorkspaceId, PathBuf),
    }

    /// Kinds that open a device, a file or a window when ticked.
    const SKIP: &[&str] = &[
        "audioin",
        "camera",
        "gamepad",
        "screencapture",
        "video",
        "imagegif",
        "mouseinput",
        "test_support",
        "sample",
    ];

    fn number_keys(n: &Node) -> Vec<&'static str> {
        n.def
            .inputs
            .iter()
            .chain(n.def.hidden)
            .filter(|i| matches!(i.control, Control::Number { .. }))
            .map(|i| i.key)
            .collect()
    }

    fn clip_of(g: &Graph, ids: &[NodeId]) -> Clip {
        let nodes = ids
            .iter()
            .map(|id| g.get(*id).expect("picked").clone())
            .collect();
        let index = |id: NodeId| ids.iter().position(|o| *o == id);
        let edges = g
            .connections()
            .iter()
            .filter_map(|c| {
                Some(ClipEdge {
                    from: (index(c.from.node)?, c.from.key),
                    to: (index(c.to.node)?, c.to.key),
                })
            })
            .collect();
        Clip {
            nodes,
            edges,
            from: None,
        }
    }

    /// What the next edit is, from the graph as it stands.
    struct Gen {
        rng: Rng,
        defs: Vec<&'static NodeDef>,
        duals: Vec<&'static NodeDef>,
        last: Option<Command>,
        tmp: PathBuf,
        exports: Vec<PathBuf>,
    }

    impl Gen {
        fn subset(&mut self, ids: &[NodeId], max: usize) -> Vec<NodeId> {
            let mut out = Vec::new();
            for _ in 0..=self.rng.below(max.min(ids.len())) {
                if let Some(id) = self.rng.pick(ids)
                    && !out.contains(id)
                {
                    out.push(*id);
                }
            }
            out
        }

        fn ws(&mut self, g: &Graph) -> WorkspaceId {
            let ws: Vec<WorkspaceId> = g.workspaces().iter().map(|w| w.id).collect();
            *self.rng.pick(&ws).expect("there is always one")
        }

        fn port(&mut self, g: &Graph, output: bool) -> Option<PortRef> {
            let ids: Vec<NodeId> = g.iter().map(|(i, _)| i).collect();
            for _ in 0..8 {
                let id = *self.rng.pick(&ids)?;
                let n = g.get(id)?;
                let ports = if output { &n.outputs } else { &n.inputs };
                if let Some(p) = self.rng.pick(ports) {
                    return Some(PortRef::new(id, p.key));
                }
            }
            None
        }

        /// A cable the rules are likely to take: an output, and an input its type feeds.
        fn good_pair(&mut self, g: &Graph) -> Option<(PortRef, PortRef)> {
            for _ in 0..20 {
                let from = self.port(g, true)?;
                let ty = g.get(from.node)?.output(from.key)?.ty;
                let fits: Vec<PortRef> = g
                    .iter()
                    .filter(|(id, _)| *id != from.node)
                    .flat_map(|(id, n)| {
                        n.inputs
                            .iter()
                            .filter(|p| ty.feeds(p.ty))
                            .map(move |p| PortRef::new(id, p.key))
                    })
                    .collect();
                if let Some(to) = self.rng.pick(&fits) {
                    return Some((from, *to));
                }
            }
            None
        }

        /// The next frame of the gesture in progress, a third of the time.
        fn continuation(&mut self, g: &Graph) -> Option<Command> {
            if !self.rng.chance(0.35) {
                return None;
            }
            match self.last.clone()? {
                Command::MoveNodes { moves } if moves.iter().all(|(i, _)| g.get(*i).is_some()) => {
                    let moves = moves
                        .iter()
                        .map(|(i, p)| {
                            (
                                *i,
                                *p + Vec2::new(self.rng.f(-9.0, 9.0), self.rng.f(-9.0, 9.0)),
                            )
                        })
                        .collect();
                    Some(Command::MoveNodes { moves })
                }
                Command::SetControl { node, key, .. } if g.get(node).is_some() => {
                    Some(Command::SetControl {
                        node,
                        key,
                        value: ControlValue::Float(self.rng.f(-5.0, 50.0)),
                    })
                }
                _ => None,
            }
        }

        #[allow(clippy::too_many_lines)]
        fn op(&mut self, g: &Graph) -> Op {
            if let Some(cmd) = self.continuation(g) {
                return Op::Apply(cmd);
            }
            let ids: Vec<NodeId> = g.iter().map(|(i, _)| i).collect();
            let some = !ids.is_empty();
            let cmd = match self.rng.below(100) {
                0..=13 => {
                    let def = if self.rng.chance(0.4) {
                        *self.rng.pick(&self.duals).expect("there are dual kinds")
                    } else {
                        *self.rng.pick(&self.defs).expect("there are kinds")
                    };
                    Command::AddNode {
                        slug: def.slug,
                        at: Pos2::new(self.rng.f(0.0, 900.0), self.rng.f(0.0, 900.0)),
                        workspace: self.ws(g),
                    }
                }
                14..=30 => {
                    let pair = if self.rng.chance(0.85) {
                        self.good_pair(g)
                    } else {
                        None
                    };
                    match pair.or_else(|| Some((self.port(g, true)?, self.port(g, false)?))) {
                        Some((from, to)) => Command::Connect { from, to },
                        None => return Op::EndGesture,
                    }
                }
                31..=34 => match self.rng.pick(g.connections()).copied() {
                    Some(c) => match self.rng.below(3) {
                        0 => Command::Disconnect { to: c.to },
                        1 => Command::DisconnectEdge {
                            from: c.from,
                            to: c.to,
                        },
                        _ => Command::DisconnectPort(if self.rng.chance(0.5) {
                            c.from
                        } else {
                            c.to
                        }),
                    },
                    None => return Op::Undo,
                },
                35..=36 if some => Command::DisconnectAll(self.subset(&ids, 2)),
                37..=40 if some => {
                    // Now and then the same node twice, as a selection sent twice would.
                    let mut doomed = self.subset(&ids, 3);
                    if self.rng.chance(0.2) {
                        doomed.push(doomed[0]);
                    }
                    Command::RemoveNodes(doomed)
                }
                41..=44 if some => {
                    let s = self.subset(&ids, 3);
                    Command::MoveNodes {
                        moves: s
                            .iter()
                            .map(|i| {
                                (
                                    *i,
                                    Pos2::new(self.rng.f(0.0, 900.0), self.rng.f(0.0, 900.0)),
                                )
                            })
                            .collect(),
                    }
                }
                45..=50 if some => {
                    let id = *self.rng.pick(&ids).expect("some");
                    let keys = number_keys(g.get(id).expect("listed"));
                    let Some(key) = self.rng.pick(&keys).copied() else {
                        return Op::Redo;
                    };
                    let v = if self.rng.chance(0.05) {
                        f32::NAN
                    } else {
                        self.rng.f(-100.0, 100.0)
                    };
                    Command::SetControl {
                        node: id,
                        key,
                        value: ControlValue::Float(v),
                    }
                }
                51..=52 if some => {
                    let id = *self.rng.pick(&ids).expect("some");
                    let keys = number_keys(g.get(id).expect("listed"));
                    if keys.len() < 2 {
                        return Op::EndGesture;
                    }
                    Command::SetControls {
                        node: id,
                        values: keys
                            .iter()
                            .take(2)
                            .map(|k| (*k, ControlValue::Float(self.rng.f(-3.0, 30.0))))
                            .collect(),
                    }
                }
                53..=55 if some => {
                    let id = *self.rng.pick(&ids).expect("some");
                    let keys = number_keys(g.get(id).expect("listed"));
                    match self.rng.pick(&keys).copied() {
                        Some(key) if self.rng.chance(0.6) => {
                            let min = self.rng.f(-10.0, 10.0);
                            let max = min + self.rng.f(0.01, 10.0);
                            Command::SetRange {
                                node: id,
                                key,
                                range: ControlRange {
                                    min,
                                    max,
                                    step: 0.01,
                                },
                            }
                        }
                        Some(key) => Command::ClearRange { node: id, key },
                        None => return Op::EndGesture,
                    }
                }
                56..=59 if some => {
                    let id = *self.rng.pick(&ids).expect("some");
                    let def = g.get(id).expect("listed").def;
                    let Some(o) = self.rng.pick(def.options) else {
                        return Op::EndGesture;
                    };
                    let value = match self.rng.pick(o.choices) {
                        Some((v, _)) => (*v).to_string(),
                        None => o.default.to_string(),
                    };
                    Command::SetOption {
                        node: id,
                        key: o.key,
                        value,
                    }
                }
                60 if some => {
                    let id = *self.rng.pick(&ids).expect("some");
                    let Some(v) = self.rng.pick(g.get(id).expect("listed").def.values) else {
                        return Op::EndGesture;
                    };
                    Command::SetValue {
                        node: id,
                        key: v.key,
                        value: Value::Text(format!("t{}", self.rng.below(99))),
                    }
                }
                61 if some => Command::SetCollapsed {
                    nodes: self.subset(&ids, 3),
                    collapsed: self.rng.chance(0.5),
                },
                62 if some => Command::SetNodeSize {
                    node: *self.rng.pick(&ids).expect("some"),
                    width: Some(self.rng.f(-10.0, 400.0)),
                    height: Some(self.rng.f(-10.0, 400.0)),
                },
                63..=65 if some => Command::Duplicate {
                    nodes: self.subset(&ids, 4),
                    offset: Vec2::new(30.0, 30.0),
                },
                66..=69 if some => {
                    let s = self.subset(&ids, 4);
                    Command::Paste {
                        clip: clip_of(g, &s),
                        at: Pos2::new(self.rng.f(0.0, 500.0), 10.0),
                        workspace: self.ws(g),
                    }
                }
                70..=72 => {
                    let Some(c) = self.rng.pick(g.connections()).copied() else {
                        return Op::EndGesture;
                    };
                    let ty = g
                        .get(c.from.node)
                        .and_then(|n| n.output(c.from.key))
                        .expect("a cable")
                        .ty;
                    let def = *self.rng.pick(&self.duals).expect("there are dual kinds");
                    let (ins, outs) = def.port_defs();
                    let inputs: Vec<&'static str> = ins
                        .iter()
                        .filter(|p| ty.feeds(p.ty))
                        .map(|p| p.key)
                        .take(1)
                        .collect();
                    let Some(out) = self.rng.pick(&outs) else {
                        return Op::EndGesture;
                    };
                    Command::Bridge {
                        from: c.from,
                        to: c.to,
                        slug: def.slug,
                        inputs: Box::leak(inputs.into_boxed_slice()),
                        output: out.key,
                        at: Pos2::new(1.0, 2.0),
                        workspace: self.ws(g),
                    }
                }
                73..=74 => Command::AddWorkspace {
                    name: format!("W{}", self.rng.below(1000)),
                    kind: WorkspaceKind::Video,
                    layout: if self.rng.chance(0.5) {
                        LayoutMode::Canvas
                    } else {
                        LayoutMode::Linear
                    },
                    seed: if self.rng.chance(0.5) {
                        Seed::Empty
                    } else {
                        Seed::SourceToOutput { width: 1200.0 }
                    },
                },
                75 => Command::RemoveWorkspace(self.ws(g)),
                76 => Command::RenameWorkspace {
                    id: self.ws(g),
                    name: format!("R{}", self.rng.below(50)),
                },
                77 => Command::SetBlurb {
                    id: self.ws(g),
                    blurb: format!("b{}", self.rng.below(50)),
                },
                78 => Command::MoveWorkspace {
                    id: self.ws(g),
                    to: self.rng.below(4),
                },
                79 => Command::SetLayout {
                    workspace: self.ws(g),
                    mode: if self.rng.chance(0.5) {
                        LayoutMode::Canvas
                    } else {
                        LayoutMode::Linear
                    },
                },
                80..=82 if some => {
                    let nodes = self.subset(&ids, 3);
                    let workspace = self.ws(g);
                    match self.rng.below(3) {
                        0 => Command::ShowOn { nodes, workspace },
                        1 => Command::HideFrom { nodes, workspace },
                        _ => Command::MoveTo { nodes, workspace },
                    }
                }
                83 if some => Command::ResetControls(self.subset(&ids, 3)),
                84 => {
                    let ws = self.ws(g);
                    let path = self
                        .tmp
                        .join(format!("export-{}.ssw", self.rng.next() % 1_000_000));
                    return Op::Export(ws, path);
                }
                85 => match self.rng.pick(&self.exports).cloned() {
                    Some(file) => Command::ImportWorkspace { file },
                    None => return Op::EndGesture,
                },
                86 => Command::AutoArrange {
                    workspace: self.ws(g),
                    height: 800.0,
                },
                87..=90 => return Op::Undo,
                91..=93 => return Op::Redo,
                97..=99 => return Op::RoundTrip,
                _ => return Op::EndGesture,
            };
            Op::Apply(cmd)
        }
    }

    /// The app under test, and a model of its ring: the dump of the graph each step holds.
    struct Model {
        app: App,
        undo: Vec<String>,
        redo: Vec<String>,
        /// Every node id ever seen, and the kind it was.
        ever: BTreeMap<NodeId, &'static str>,
        ever_ws: BTreeSet<WorkspaceId>,
        counters: (u32, u32),
        /// Older graphs, held as clones, with the dump taken when they were held.
        held: Vec<(Graph, String)>,
        tmp: PathBuf,
    }

    impl Model {
        /// Ids after an op: new ones only from an edit, and only above everything seen before.
        fn check_ids(&mut self, restoring: bool) -> Result<(), String> {
            let g = self.app.graph();
            let highest = self.ever.keys().next_back().copied();
            let mut fresh = Vec::new();
            for (id, n) in g.iter() {
                match self.ever.get(&id) {
                    Some(slug) if *slug != n.def.slug => {
                        return Err(format!("{id} was a {slug} and is a {}", n.def.slug));
                    }
                    Some(_) => {}
                    None => fresh.push((id, n.def.slug)),
                }
            }
            if restoring && !fresh.is_empty() {
                return Err(format!("an undo or redo made {fresh:?}"));
            }
            for (id, slug) in fresh {
                if highest.is_some_and(|m| id <= m) {
                    return Err(format!("{id} is not above {highest:?}"));
                }
                self.ever.insert(id, slug);
            }
            for w in g.workspaces() {
                if self.ever_ws.insert(w.id) && restoring {
                    return Err(format!("an undo or redo made workspace {}", w.id));
                }
            }
            let now = counters(g);
            if now.0 < self.counters.0 || now.1 < self.counters.1 {
                return Err(format!(
                    "a counter went back: {:?} to {now:?}",
                    self.counters
                ));
            }
            self.counters = now;
            Ok(())
        }

        fn round_trip(&self, step: usize) -> Result<(), String> {
            let g = self.app.graph();
            let dir = self.tmp.join(format!("round-trip-{step}"));
            Project::new(dir.clone())
                .save(g)
                .map_err(|e| e.to_string())?;
            let (_, back, warnings) = Project::open(dir.clone()).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_dir_all(&dir);
            if !warnings.is_empty() {
                return Err(format!("a round trip warned: {warnings:?}"));
            }
            if dump(&back, false) != dump(g, false) || counters(&back) != counters(g) {
                return Err("a round trip changed the graph".into());
            }
            check_graph(&back)
        }

        fn step(&mut self, step: usize, op: &Op) -> Result<(), String> {
            let before = dump(self.app.graph(), true);
            let (u0, dirty0) = (self.app.undo_len(), self.app.dirty());
            let mut restoring = false;
            match op {
                Op::Apply(cmd) => match self.app.apply(cmd.clone()) {
                    Err(e) => {
                        if dump(self.app.graph(), true) != before {
                            return Err(format!("{cmd:?} was refused ({e}) and changed the graph"));
                        }
                        if self.app.undo_len() != u0 || self.app.dirty() != dirty0 {
                            return Err(format!("{cmd:?} was refused ({e}) and left a step"));
                        }
                    }
                    Ok(()) => match self.app.undo_len() {
                        // A new step, and nothing undone is reachable any more.
                        u1 if u1 == u0 + 1 => {
                            self.undo.push(before);
                            self.redo.clear();
                        }
                        // It joined the gesture in progress, and the top stays.
                        u1 if u1 == u0 => {}
                        // A walk this short never reaches either cap.
                        u1 => return Err(format!("the ring went from {u0} to {u1}")),
                    },
                },
                Op::Undo => {
                    if self.app.undo() {
                        restoring = true;
                        let expect = self.undo.pop().ok_or("an undo the model has no step for")?;
                        if dump(self.app.graph(), true) != expect {
                            return Err("an undo gave back another graph".into());
                        }
                        self.redo.push(before);
                    } else if !self.undo.is_empty() {
                        return Err("an undo refused with steps to take".into());
                    }
                }
                Op::Redo => {
                    if self.app.redo() {
                        restoring = true;
                        let expect = self.redo.pop().ok_or("a redo the model has no step for")?;
                        if dump(self.app.graph(), true) != expect {
                            return Err("a redo gave back another graph".into());
                        }
                        self.undo.push(before);
                    } else if !self.redo.is_empty() {
                        return Err("a redo refused with steps to take".into());
                    }
                }
                Op::EndGesture => self.app.end_gesture(),
                Op::RoundTrip => self.round_trip(step)?,
                Op::Export(ws, path) => {
                    let g = self.app.graph();
                    if let Some(w) = g.workspace(*ws) {
                        let nodes: BTreeSet<NodeId> = g.on_workspace(*ws).map(|(i, _)| i).collect();
                        let file = supersilvia::workspace::WorkspaceFile::of(g, w, &nodes);
                        supersilvia::workspace::write(&file, path).map_err(|e| e.to_string())?;
                    }
                }
            }
            check_graph(self.app.graph())?;
            self.check_ids(restoring)?;
            if step.is_multiple_of(5) {
                self.held
                    .push((self.app.graph().clone(), dump(self.app.graph(), true)));
            }
            if step.is_multiple_of(20) && self.held.iter().any(|(g, d)| dump(g, true) != *d) {
                return Err("a graph an older step holds was written through".into());
            }
            Ok(())
        }
    }

    fn run(seed: u64, steps: usize) -> Result<(), String> {
        let tmp = std::env::temp_dir().join(format!("ssv-undo-fuzz-{}-{seed}", std::process::id()));
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let defs: Vec<&'static NodeDef> = REGISTRY
            .iter()
            .copied()
            .filter(|d| !SKIP.contains(&d.slug))
            .collect();
        let duals = defs
            .iter()
            .copied()
            .filter(|d| d.port_defs().1.iter().any(|p| p.dual))
            .collect();
        let mut generator = Gen {
            rng: Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) ^ 0xABCD),
            defs,
            duals,
            last: None,
            tmp: tmp.clone(),
            exports: Vec::new(),
        };
        let app = App::headless();
        let mut model = Model {
            counters: counters(app.graph()),
            ever_ws: app.graph().workspaces().iter().map(|w| w.id).collect(),
            app,
            undo: Vec::new(),
            redo: Vec::new(),
            ever: BTreeMap::new(),
            held: Vec::new(),
            tmp: tmp.clone(),
        };
        for step in 0..steps {
            let op = generator.op(model.app.graph());
            model
                .step(step, &op)
                .map_err(|e| format!("seed {seed}, step {step}, {op:?}: {e}"))?;
            match op {
                Op::Apply(c) => generator.last = Some(c),
                Op::Export(_, p) => generator.exports.push(p),
                _ => generator.last = None,
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(())
    }

    #[test]
    fn a_random_walk_of_edits_keeps_every_promise_undo_makes() {
        for seed in 0..SEEDS {
            if let Err(failure) = run(seed, STEPS) {
                panic!("{failure}");
            }
        }
    }

    const SEEDS: u64 = 12;
    const STEPS: usize = 160;
}
