// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: a closed workspace is suspended, not unloaded.
//!
//! The nodes stay in the graph and keep their state; what stops is the work. A node is
//! suspended when **every** workspace showing it is closed: its `tick` is not called and its
//! Output is not built or drawn. A node shown on a closed workspace and an open one does
//! both, which is what one node means.

mod common;

use common::add_on;
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::project::Active;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

/// A second workspace, open and showing.
fn second(app: &mut App) -> WorkspaceId {
    app.apply(Command::AddWorkspace {
        name: "Second".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        // These tests want a bare workspace; the app seeds a video tab with a patch.
        seed: supersilvia::command::Seed::Empty,
    })
    .expect("a workspace can always be added");
    let id = app.graph().workspaces().last().expect("just added").id;
    app.open_workspace(id);
    id
}

/// Whether this Output is in the frame the renderer is asked to draw.
fn drawn(app: &mut App, node: NodeId) -> bool {
    app.build_frame_job()
        .outputs
        .iter()
        .any(|o| o.node == node && o.mode != supersilvia::render::OutputMode::Suspended)
}

#[test]
fn a_node_on_a_closed_workspace_alone_does_not_tick() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = second(&mut app);

    let slew = add_on(&mut app, "slew", other);
    let out = PortRef::new(slew, "output");
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(4.0),
    })
    .unwrap();

    app.tick(FRAME);
    assert_eq!(app.uniform(out), Some(4.0), "open, so it runs");

    // Closing takes the tab, not the node.
    app.close_workspace(other);
    assert_eq!(app.active(), Active::Workspace(first));
    assert!(
        app.graph().get(slew).is_some(),
        "the node stays in the graph"
    );

    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(9.0),
    })
    .unwrap();
    for _ in 0..10 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(out),
        Some(4.0),
        "suspended: it did not tick, and what it last published still stands"
    );

    // The tick it wakes on takes the gap it slept through as a jump, which a slew steps
    // nothing across; the tick after runs.
    app.open_workspace(other);
    app.tick(FRAME);
    assert_eq!(
        app.uniform(out),
        Some(4.0),
        "woken, it carries across the gap"
    );
    app.tick(FRAME);
    assert!(
        app.uniform(out).is_some_and(|v| v > 4.0),
        "reopened, it runs again"
    );
}

/// The rule this design rests on: a node shown anywhere open is running.
#[test]
fn a_node_on_a_closed_and_an_open_workspace_ticks() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = second(&mut app);

    let slew = add_on(&mut app, "slew", other);
    app.apply(Command::ShowOn {
        nodes: vec![slew],
        workspace: first,
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(2.0),
    })
    .unwrap();

    app.close_workspace(other);
    app.tick(FRAME);
    assert_eq!(
        app.uniform(PortRef::new(slew, "output")),
        Some(2.0),
        "one tab that shows it is open, so it runs"
    );
}

/// A suspended producer holds its last value for a consumer that is still running.
///
/// Not a zero and not a gap: the number is still in the graph's published table, because
/// the node is still in the graph. It is the same hold a dropped frame gets.
#[test]
fn a_suspended_producer_holds_its_last_value_for_a_running_consumer() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = second(&mut app);

    let producer = add_on(&mut app, "slew", other);
    let consumer = add_on(&mut app, "slew", first);
    app.apply(Command::Connect {
        from: PortRef::new(producer, "output"),
        to: PortRef::new(consumer, "input"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: producer,
        key: "input",
        value: ControlValue::Float(6.0),
    })
    .unwrap();
    app.tick(FRAME);
    assert_eq!(app.uniform(PortRef::new(producer, "output")), Some(6.0));

    app.close_workspace(other);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(PortRef::new(producer, "output")),
        Some(6.0),
        "held, not cleared"
    );
    let seen = app
        .uniform(PortRef::new(consumer, "output"))
        .expect("the consumer is on an open workspace and runs");
    assert!(
        seen > 0.0,
        "the consumer slewed towards the held value: {seen}"
    );
}

/// **A gear on a closed tab keeps running while an open tab reads it.** A one-second Master
/// Gear on a workspace whose tab is closed, its Cycles in the Time of an oscillator on an open
/// one, counts on with the show — a second of ticks is a cycle — and so does the ×2 Ratio
/// Gear between them, as a deck wakes what it reads. With nothing open reading it, a closed
/// gear sleeps.
#[test]
fn a_gear_on_a_closed_tab_keeps_running_while_an_open_tab_reads_it() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = second(&mut app);
    let master = add_on(&mut app, "mastergear", other);
    let double = add_on(&mut app, "ratiogear", other);
    let idle = add_on(&mut app, "mastergear", other);
    let osc = add_on(&mut app, "oscillator", first);
    for (node, key, value) in [(master, "length", 1.0), (double, "p", 2.0)] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.apply(Command::SetOption {
        node: osc,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    for (from, to) in [
        (
            PortRef::new(master, "cycles"),
            PortRef::new(double, "clock"),
        ),
        (
            PortRef::new(double, "cycles"),
            PortRef::new(osc, supersilvia::nodes::TIME),
        ),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.tick(FRAME);
    app.close_workspace(other);
    app.tick(FRAME);
    let cycles = |app: &App, node| app.uniform(PortRef::new(node, "cycles")).unwrap();
    let (m, d, i) = (
        cycles(&app, master),
        cycles(&app, double),
        cycles(&app, idle),
    );
    for _ in 0..60 {
        app.tick(FRAME);
    }
    let counted = cycles(&app, master) - m;
    assert!(
        (counted - 1.0).abs() < 1e-3,
        "the master counts a cycle in a second: {counted}"
    );
    let doubled = cycles(&app, double) - d;
    assert!(
        (doubled - 2.0).abs() < 1e-3,
        "and the gear between counts two: {doubled}"
    );
    assert_eq!(cycles(&app, idle), i, "a gear nothing open reads sleeps");
}

#[test]
fn an_output_shown_only_on_a_closed_workspace_is_not_in_the_render_set() {
    let mut app = App::headless();
    let first = app.graph().default_workspace();
    let other = second(&mut app);

    let here = add_on(&mut app, "output", first);
    let there = add_on(&mut app, "output", other);
    let source = add_on(&mut app, "checkerboard", other);
    app.apply(Command::Connect {
        from: PortRef::new(source, "output"),
        to: PortRef::new(there, "input"),
    })
    .unwrap();

    assert!(drawn(&mut app, there), "open, so it draws");

    app.close_workspace(other);
    assert!(drawn(&mut app, here), "the open one still draws");
    assert!(!drawn(&mut app, there), "the closed one does not");

    // Still in the job, so the renderer keeps the targets it allocated: closing and
    // reopening a workspace must not reallocate anything.
    let job = app.build_frame_job();
    assert!(
        job.outputs.iter().any(|o| o.node == there),
        "a suspended Output is described, not dropped"
    );
    assert_eq!(
        job.display,
        supersilvia::render::Display::Mixer,
        "and the preview shows the mix, not any one Output"
    );

    app.open_workspace(other);
    assert!(drawn(&mut app, there), "reopened, it draws again");
}

/// An edit made while a workspace is closed is built on the frame its tab comes back.
#[test]
fn a_suspended_output_is_not_in_the_shader_build_set_and_rebuilds_on_reopen() {
    let mut app = App::headless();
    let other = second(&mut app);
    let out = add_on(&mut app, "output", other);
    let source = add_on(&mut app, "checkerboard", other);

    app.build_frame_job();
    assert!(!app.needs_recompile(out), "built while it was open");

    app.close_workspace(other);
    app.apply(Command::Connect {
        from: PortRef::new(source, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    assert!(app.needs_recompile(out));

    app.build_frame_job();
    assert!(
        app.needs_recompile(out),
        "suspended, so the rebuild waits rather than being spent on a frame nobody sees"
    );

    app.open_workspace(other);
    app.build_frame_job();
    assert!(!app.needs_recompile(out), "and it is built on the way back");
}

/// A tap on a closed workspace is measured by nothing: a suspended node does not tick, so
/// there would be nobody to read the slot back, and the measurement would be work the pass
/// paid for and nothing collected. The tab coming back puts it in its pass again.
#[test]
fn a_tap_on_a_closed_workspace_is_measured_by_nothing() {
    let mut app = App::headless();
    let other = second(&mut app);
    let out = add_on(&mut app, "output", other);
    let source = add_on(&mut app, "checkerboard", other);
    let tap = add_on(&mut app, "tap", other);
    app.apply(Command::Connect {
        from: PortRef::new(source, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.publish_plan();
    assert_eq!(
        app.pass_measures(other),
        [tap],
        "open, the workspace's pass measures its tap"
    );

    app.close_workspace(other);
    app.publish_plan();
    assert!(
        app.pass_measures(other).is_empty(),
        "a closed workspace keeps no pass for what nothing awake reads"
    );

    app.open_workspace(other);
    app.publish_plan();
    assert_eq!(
        app.pass_measures(other),
        [tap],
        "and the tab coming back starts it"
    );
}
