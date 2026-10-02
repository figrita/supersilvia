// SPDX-License-Identifier: AGPL-3.0-or-later

//! Small helpers shared by Layer 1 test files: a headless `App`, adding a node on a
//! workspace, cabling two ports, and two tabs with the first one showing.
//!
//! `#![allow(dead_code)]` because each `tests/*.rs` file is its own crate — a helper this
//! module exports but a given caller doesn't use is dead code in that one crate even though
//! another crate here uses it.

#![allow(dead_code)]

use emath::Pos2;
use supersilvia::graph::{NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::project::Active;
use supersilvia::{App, Command};

/// Add a node through the bus, on one workspace, and hand back its id.
pub fn add_on(app: &mut App, slug: &'static str, workspace: WorkspaceId) -> NodeId {
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

/// Cable one port to another.
pub fn connect(app: &mut App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    app.apply(Command::Connect {
        from: PortRef::new(from.0, from.1),
        to: PortRef::new(to.0, to.1),
    })
    .expect("a legal cable");
}

/// Two open workspaces, the first one showing.
pub fn two_tabs() -> (App, WorkspaceId, WorkspaceId) {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Second".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .expect("a workspace can always be added");
    let second = app.graph().workspaces().last().expect("just added").id;
    app.open_workspace(second);
    app.activate(Active::Workspace(first));
    (app, first, second)
}

/// A checkerboard into an Output, on one workspace.
pub fn picture_on(app: &mut App, workspace: WorkspaceId) -> NodeId {
    let out = add_on(app, "output", workspace);
    let source = add_on(app, "checkerboard", workspace);
    connect(app, (source, "output"), (out, "input"));
    out
}
