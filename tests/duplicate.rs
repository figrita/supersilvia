// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: a workspace duplicated whole.
//!
//! One command, so one undo step; the copy beside the original in project order, under
//! `<name> copy`; every node on it a plain node on the copy alone; the cables between them
//! with them, a cable arriving from elsewhere arriving at the copy too, and one leaving for
//! elsewhere left with the original.

use emath::Pos2;
use std::collections::BTreeSet;
use supersilvia::graph::{ControlValue, LayoutMode, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::project::Active;
use supersilvia::{App, Command};

fn add_on(app: &mut App, slug: &'static str, workspace: WorkspaceId) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::new(40.0, 60.0),
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

fn add_workspace(app: &mut App, name: &str) -> WorkspaceId {
    app.apply(Command::AddWorkspace {
        name: name.into(),
        kind: WorkspaceKind::Video,
        layout: LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    app.graph().workspaces().last().unwrap().id
}

fn names(app: &App) -> Vec<String> {
    app.graph()
        .workspaces()
        .iter()
        .map(|w| w.name.clone())
        .collect()
}

fn on(app: &App, workspace: WorkspaceId) -> Vec<NodeId> {
    app.graph()
        .on_workspace(workspace)
        .map(|(id, _)| id)
        .collect()
}

/// Whether a cable joins these two ports.
fn wired(app: &App, from: (NodeId, &str), to: (NodeId, &str)) -> bool {
    app.graph().connections().iter().any(|c| {
        c.from.node == from.0 && c.from.key == from.1 && c.to.node == to.0 && c.to.key == to.1
    })
}

/// **The copy is the workspace again, beside it, as one undo step**: every node with its
/// knobs, the cables between them, its layout and its blurb, and it is opened and shown.
#[test]
fn a_duplicate_copies_the_workspace_beside_it_as_one_step() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    app.apply(Command::RenameWorkspace {
        id: first,
        name: "Tunnel".into(),
    })
    .unwrap();
    app.apply(Command::SetBlurb {
        id: first,
        blurb: "the long one".into(),
    })
    .unwrap();
    app.apply(Command::SetLayout {
        workspace: first,
        mode: LayoutMode::Linear,
    })
    .unwrap();
    let spiral = add_on(&mut app, "spiral", first);
    let zoom = add_on(&mut app, "zoom", first);
    let out = add_on(&mut app, "output", first);
    wire(&mut app, (spiral, "color"), (zoom, "input"));
    wire(&mut app, (zoom, "output"), (out, "input"));
    app.apply(Command::SetControl {
        node: spiral,
        key: "turns",
        value: ControlValue::Float(7.0),
    })
    .unwrap();
    // A workspace after it, so "beside" is not the same answer as "last".
    add_workspace(&mut app, "Feedback");
    let undo = app.undo_len();
    let nodes = app.graph().len();

    app.duplicate_workspace(first);

    assert_eq!(names(&app), ["Tunnel", "Tunnel copy", "Feedback"]);
    assert_eq!(app.undo_len(), undo + 1, "one undo step");
    let copy = app.graph().workspaces()[1].id;
    let w = app.graph().workspace(copy).unwrap();
    assert_eq!(
        (w.layout, w.blurb.as_str()),
        (LayoutMode::Linear, "the long one")
    );
    assert_eq!(app.active(), Active::Workspace(copy), "opened and shown");
    assert!(app.open_workspaces().contains(&copy));

    let copies = on(&app, copy);
    assert_eq!(copies.len(), 3);
    assert_eq!(app.graph().len(), nodes + 3);
    for id in &copies {
        assert_eq!(
            app.graph().get(*id).unwrap().workspaces,
            BTreeSet::from([copy]),
            "a copy is on the copy alone"
        );
    }
    let slug = |id: NodeId| app.graph().get(id).unwrap().def.slug;
    let find = |s: &str| copies.iter().copied().find(|id| slug(*id) == s).unwrap();
    let (spiral2, zoom2, out2) = (find("spiral"), find("zoom"), find("output"));
    assert!(wired(&app, (spiral2, "color"), (zoom2, "input")));
    assert!(wired(&app, (zoom2, "output"), (out2, "input")));
    assert_eq!(
        app.graph().get(spiral2).unwrap().controls.get("turns"),
        Some(&ControlValue::Float(7.0))
    );
    assert_eq!(
        app.graph().get(spiral2).unwrap().pos,
        app.graph().get(spiral).unwrap().pos,
        "where the original's are, on a canvas of its own"
    );
    assert_eq!(
        on(&app, first),
        [spiral, zoom, out],
        "the original is untouched"
    );

    app.undo();
    assert_eq!(names(&app), ["Tunnel", "Feedback"]);
    assert_eq!(app.graph().len(), nodes);
}

/// **A cable from elsewhere reaches the copy; a cable to elsewhere stays the original's.** The
/// copy reads the clock or the source the original reads, so it works as the original does;
/// the input elsewhere already has the original's cable, and taking it would change the
/// original's patch.
#[test]
fn a_cable_from_elsewhere_reaches_the_copy_and_one_to_elsewhere_does_not() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = add_workspace(&mut app, "Other");
    let spiral = add_on(&mut app, "spiral", other);
    let zoom = add_on(&mut app, "zoom", first);
    let out = add_on(&mut app, "output", other);
    wire(&mut app, (spiral, "color"), (zoom, "input"));
    wire(&mut app, (zoom, "output"), (out, "input"));

    app.duplicate_workspace(first);
    let copy = app.graph().workspaces()[1].id;
    let [zoom2] = on(&app, copy)[..] else {
        panic!("one node copied");
    };
    assert!(
        wired(&app, (spiral, "color"), (zoom2, "input")),
        "the copy reads what the original reads"
    );
    assert!(
        wired(&app, (spiral, "color"), (zoom, "input")),
        "and so does the original"
    );
    assert!(
        !wired(&app, (zoom2, "output"), (out, "input")),
        "the Output elsewhere keeps the original's cable"
    );
    assert!(wired(&app, (zoom, "output"), (out, "input")));
}

/// A node shared with another workspace is copied as a plain node on the copy, as an export
/// writes it: a duplicate is a variation, and a node still shared would be the original.
#[test]
fn a_shared_node_is_copied_as_a_plain_node() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = add_workspace(&mut app, "Other");
    let gear = add_on(&mut app, "mastergear", first);
    app.apply(Command::ShowOn {
        nodes: vec![gear],
        workspace: other,
    })
    .unwrap();

    app.duplicate_workspace(first);
    let copy = app.graph().workspaces()[1].id;
    let [gear2] = on(&app, copy)[..] else {
        panic!("one node copied");
    };
    assert_ne!(gear2, gear);
    assert_eq!(
        app.graph().get(gear2).unwrap().workspaces,
        BTreeSet::from([copy])
    );
    assert_eq!(
        app.graph().get(gear).unwrap().workspaces,
        BTreeSet::from([first, other]),
        "the original stays shared"
    );
}

/// Each copy has a name of its own, counting up past the ones taken.
#[test]
fn a_second_duplicate_counts_up() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    app.duplicate_workspace(first);
    app.duplicate_workspace(first);
    assert_eq!(
        names(&app),
        ["Workspace 1", "Workspace 1 copy 2", "Workspace 1 copy"],
        "each beside the original, the newest nearest"
    );
}
