// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 2: the canvas's keys and the two cable gestures, headless.
//!
//! Delete, Backspace, `Ctrl`+D, `Ctrl`+A, `Escape`, Home and `.` on the canvas; a cable let go
//! in the open, which opens the browser on what takes it; a node dropped on a cable, which
//! splices it in; the words a refused port says; and the selection count. See docs/ui.md,
//! "The canvas's keys" and "Cables".
//!
//! A file of its own rather than more of `tests/ui.rs`, with the few helpers it needs written
//! out again here. No test takes a snapshot, so no harness reaches a GPU.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use emath::Pos2;
use supersilvia::graph::{LayoutMode, NodeId, PortRef};
use supersilvia::{App, Command};

/// The app with preferences that live for the test only, NDI and Syphon pinned absent as
/// `tests/ui.rs` pins them.
fn app(cc: &mut eframe::CreationContext<'_>) -> App {
    supersilvia::video::ndi::pretend_missing(true);
    supersilvia::platform::syphon::pretend_unavailable(true);
    App::new(cc, supersilvia::preferences::Store::in_memory())
}

fn harness<'a>() -> Harness<'a, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_eframe(app);
    h.step();
    h
}

/// A node added through the bus on the workspace showing, settled; answers its id.
fn add_at(h: &mut Harness<'_, App>, slug: &'static str, at: Pos2) -> NodeId {
    let workspace = h
        .state()
        .active_workspace()
        .expect("a workspace is showing");
    h.state_mut()
        .apply(Command::AddNode {
            slug,
            at,
            workspace,
        })
        .expect("in the registry");
    let id = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    h.run_steps(2);
    id
}

fn connect(h: &mut Harness<'_, App>, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(from.0, from.1),
            to: PortRef::new(to.0, to.1),
        })
        .expect("a legal cable");
    h.run_steps(2);
}

/// The workspace showing, laid out as an unbounded plane.
fn on_a_plane(h: &mut Harness<'_, App>) {
    let workspace = h.state().active_workspace().unwrap();
    if h.state().graph().layout_of(workspace) != LayoutMode::Canvas {
        h.state_mut()
            .apply(Command::SetLayout {
                workspace,
                mode: LayoutMode::Canvas,
            })
            .unwrap();
    }
    h.run_steps(2);
}

fn press_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

fn move_to(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
}

fn release_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
}

/// Press, travel there over a few frames, and hold: a drag is reported on the frame the
/// pointer crosses the threshold, and one jump is a click to egui.
fn hold_and_move(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    press_at(h, from);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        move_to(h, from + (to - from) * t);
    }
}

fn port(h: &Harness<'_, App>, name: &str) -> Pos2 {
    h.get_by_label_contains(name).rect().center()
}

fn header(h: &Harness<'_, App>, slug: &str, id: NodeId) -> Pos2 {
    h.get_by_label(&format!("{slug}{id}")).rect().center()
}

fn key(h: &mut Harness<'_, App>, key: egui::Key) {
    h.key_press(key);
    h.run_steps(2);
}

fn command_key(h: &mut Harness<'_, App>, key: egui::Key) {
    h.key_press_modifiers(egui::Modifiers::COMMAND, key);
    h.run_steps(2);
}

fn selection(h: &Harness<'_, App>) -> Vec<NodeId> {
    h.state().canvas_selection()
}

/// Delete and Backspace each take the selection, one undo step a press, and only the
/// selection.
#[test]
fn delete_and_backspace_each_delete_the_selection_in_one_step() {
    let mut h = harness();
    let a = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    let b = add_at(&mut h, "checkerboard", Pos2::new(300.0, 60.0));
    let c = add_at(&mut h, "checkerboard", Pos2::new(560.0, 60.0));
    let steps = h.state().history().len();

    h.state_mut().select_only(&[a, b]);
    key(&mut h, egui::Key::Delete);
    assert!(h.state().graph().get(a).is_none() && h.state().graph().get(b).is_none());
    assert!(
        h.state().graph().get(c).is_some(),
        "only the selection goes"
    );
    assert_eq!(
        h.state().history().len(),
        steps + 1,
        "one step for two nodes"
    );

    h.state_mut().select_only(&[c]);
    key(&mut h, egui::Key::Backspace);
    assert!(
        h.state().graph().is_empty(),
        "Backspace deletes as Delete does"
    );

    // With nothing selected the key does nothing, and opens no step.
    key(&mut h, egui::Key::Delete);
    assert_eq!(h.state().history().len(), steps + 2);
}

/// A key meant for a number control under the pointer, or for a field with the keyboard, is
/// not the canvas's: the node stays.
#[test]
fn a_hovered_number_and_a_typing_field_keep_the_keys() {
    let mut h = harness();
    let cb = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    h.state_mut().select_only(&[cb]);
    let number = h
        .get_by_label_contains(&format!("checkerboard{cb}.frequency 8"))
        .rect()
        .center();

    // Hovered.
    move_to(&mut h, number);
    key(&mut h, egui::Key::Delete);
    assert!(h.state().graph().get(cb).is_some(), "Delete over a number");
    command_key(&mut h, egui::Key::D);
    assert_eq!(h.state().graph().len(), 1, "Ctrl+D over a number");

    // Typing: the click opens the field, and Backspace is a character taken off it.
    press_at(&mut h, number);
    release_at(&mut h, number);
    h.run_steps(2);
    assert!(h.ctx.egui_wants_keyboard_input(), "the typed entry is open");
    move_to(&mut h, Pos2::new(900.0, 600.0));
    key(&mut h, egui::Key::Backspace);
    assert!(h.state().graph().get(cb).is_some(), "Backspace in a field");
    // And the Escape that closes the field closes only the field.
    key(&mut h, egui::Key::Escape);
    assert_eq!(selection(&h), vec![cb], "the Escape that left a field");
}

/// `Ctrl`+D duplicates the selection beside itself, in one step, and the copies become the
/// selection.
#[test]
fn ctrl_d_duplicates_the_selection_and_the_copies_become_it() {
    let mut h = harness();
    let a = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    let b = add_at(&mut h, "invert", Pos2::new(300.0, 60.0));
    connect(&mut h, (a, "output"), (b, "input"));
    let steps = h.state().history().len();
    h.state_mut().select_only(&[a, b]);
    command_key(&mut h, egui::Key::D);

    assert_eq!(h.state().graph().len(), 4);
    assert_eq!(
        h.state().graph().connections().len(),
        2,
        "the cable between came too"
    );
    assert_eq!(h.state().history().len(), steps + 1);
    let copies = selection(&h);
    assert_eq!(copies.len(), 2);
    assert!(
        !copies.contains(&a) && !copies.contains(&b),
        "the copies are selected"
    );
}

/// `Ctrl`+A selects every node on the workspace being looked at, and none elsewhere.
#[test]
fn ctrl_a_selects_every_node_on_this_workspace() {
    let mut h = harness();
    let a = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    let b = add_at(&mut h, "invert", Pos2::new(300.0, 60.0));
    h.state_mut()
        .apply(Command::AddWorkspace {
            name: "Elsewhere".to_owned(),
            kind: supersilvia::graph::WorkspaceKind::Video,
            layout: LayoutMode::Canvas,
            seed: supersilvia::command::Seed::Empty,
        })
        .unwrap();
    let elsewhere = h.state().graph().workspaces().last().unwrap().id;
    h.state_mut()
        .apply(Command::AddNode {
            slug: "checkerboard",
            at: Pos2::ZERO,
            workspace: elsewhere,
        })
        .unwrap();
    h.run_steps(2);

    command_key(&mut h, egui::Key::A);
    assert_eq!(selection(&h), vec![a, b]);
}

/// `Escape` with nothing in hand clears the selection.
#[test]
fn escape_clears_the_selection() {
    let mut h = harness();
    let a = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    h.state_mut().select_only(&[a]);
    h.run_steps(2);
    key(&mut h, egui::Key::Escape);
    assert!(selection(&h).is_empty());
}

/// **`Escape` mid-drag puts the node back** where the drag found it, and leaves no step: the
/// drag's own step is dropped rather than answered with a move back, so the title does not
/// read unsaved work. The rest of the press moves nothing.
#[test]
fn escape_puts_a_node_drag_back() {
    let mut h = harness();
    let a = add_at(&mut h, "checkerboard", Pos2::new(40.0, 60.0));
    let (steps, was, dirty) = (
        h.state().history().len(),
        h.state().graph().get(a).unwrap().pos,
        h.state().dirty(),
    );
    let grip = header(&h, "checkerboard", a);
    hold_and_move(&mut h, grip, grip + egui::vec2(200.0, 120.0));
    assert_ne!(
        h.state().graph().get(a).unwrap().pos,
        was,
        "the drag moved it"
    );

    key(&mut h, egui::Key::Escape);
    assert_eq!(h.state().graph().get(a).unwrap().pos, was, "put back");
    move_to(&mut h, grip + egui::vec2(300.0, 200.0));
    release_at(&mut h, grip + egui::vec2(300.0, 200.0));
    assert_eq!(
        h.state().graph().get(a).unwrap().pos,
        was,
        "the rest of the press"
    );
    assert_eq!(h.state().history().len(), steps, "no step");
    assert_eq!(h.state().dirty(), dirty);
    assert_eq!(selection(&h), vec![a], "and the selection is the drag's");
}

/// **`Escape` drops a held cable**: let go on the port it was headed for, it lands on
/// nothing, and no browser opens.
#[test]
fn escape_drops_a_held_cable() {
    let mut h = harness();
    let cb = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let out = add_at(&mut h, "output", Pos2::new(500.0, 120.0));
    let from = port(
        &h,
        &format!("checkerboard{cb}.output (varying color output)"),
    );
    let to = port(&h, &format!("output{out}.input (varying color input)"));
    hold_and_move(&mut h, from, from + (to - from) * 0.5);
    key(&mut h, egui::Key::Escape);
    move_to(&mut h, to);
    release_at(&mut h, to);
    assert!(h.state().graph().connections().is_empty());
    assert!(h.query_by_label_contains("Search").is_none(), "no browser");
}

/// Home glides the view to fit every node, and `.` to fit the selection.
#[test]
fn home_frames_everything_and_period_frames_the_selection() {
    let mut h = harness();
    on_a_plane(&mut h);
    let near = add_at(&mut h, "checkerboard", Pos2::new(0.0, 0.0));
    let far = add_at(&mut h, "checkerboard", Pos2::new(1500.0, 900.0));
    let canvas = egui::Rect::from_min_size(
        h.state().canvas_origin(),
        egui::vec2(h.state().canvas_width(), h.state().canvas_height()),
    );
    assert!(
        h.query_by_label(&format!("checkerboard{far}")).is_none(),
        "far is off screen"
    );

    key(&mut h, egui::Key::Home);
    h.run_steps(60);
    let zoom = h.state().canvas_transform().zoom;
    assert!(zoom < 1.0, "zoomed out to fit: {zoom}");
    for id in [near, far] {
        let body = h.get_by_label(&format!("checkerboard{id}")).rect();
        assert!(
            canvas.contains_rect(body),
            "{id} is in view: {body:?} in {canvas:?}"
        );
    }

    h.state_mut().select_only(&[far]);
    key(&mut h, egui::Key::Period);
    h.run_steps(60);
    let t = h.state().canvas_transform();
    assert!(
        (t.zoom - 1.0).abs() < 0.01,
        "never past actual size: {}",
        t.zoom
    );
    let body = h.get_by_label(&format!("checkerboard{far}")).rect();
    assert!(
        (body.center().x - canvas.center().x).abs() < 40.0,
        "the selection is in the middle: {body:?} in {canvas:?}"
    );
}

/// **A cable let go in the open asks what it lands on.** The browser opens at the pointer
/// holding only what can take the cable; the kind chosen lands wired, and one undo takes the
/// node and its cable both.
#[test]
fn a_cable_let_go_in_the_open_lands_on_what_is_chosen() {
    let mut h = harness();
    on_a_plane(&mut h);
    let cb = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let steps = h.state().history().len();
    let from = port(
        &h,
        &format!("checkerboard{cb}.output (varying color output)"),
    );
    let open = from + egui::vec2(300.0, 40.0);
    hold_and_move(&mut h, from, open);
    release_at(&mut h, open);
    h.run_steps(2);

    assert!(
        h.query_by_label("add output").is_some(),
        "an Output takes a picture"
    );
    assert!(
        h.query_by_label("add mastergear").is_none(),
        "a Master Gear takes no picture"
    );
    // The search field has the keyboard, and Enter takes the best match.
    h.input_mut()
        .events
        .push(egui::Event::Text("output".to_owned()));
    h.step();
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);

    assert_eq!(h.state().graph().len(), 2);
    let out = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    assert_eq!(
        h.state().graph().connections(),
        &[supersilvia::graph::Connection {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        }]
    );
    assert_eq!(h.state().history().len(), steps + 1, "one step");
    h.state_mut().undo();
    h.run_steps(2);
    assert_eq!(h.state().graph().len(), 1, "undone whole");
    assert!(h.state().graph().connections().is_empty());
}

/// **A node dropped on a cable goes into it.** Carried over the cable it lights it, and let
/// go there the cable is replaced by two through the node — one undo step for the splice.
#[test]
fn a_node_dropped_on_a_cable_splices_in() {
    let mut h = harness();
    on_a_plane(&mut h);
    let cb = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let out = add_at(&mut h, "output", Pos2::new(700.0, 200.0));
    connect(&mut h, (cb, "output"), (out, "input"));
    let inv = add_at(&mut h, "invert", Pos2::new(300.0, 500.0));
    let cable = h
        .get_by_label(&format!(
            "cable checkerboard{cb}.output to output{out}.input"
        ))
        .rect()
        .center();

    let grip = header(&h, "invert", inv);
    hold_and_move(&mut h, grip, cable);
    move_to(&mut h, cable);
    release_at(&mut h, cable);

    let mut cables = h.state().graph().connections().to_vec();
    cables.sort_by_key(|c| c.from.node);
    assert_eq!(
        cables,
        [
            supersilvia::graph::Connection {
                from: PortRef::new(cb, "output"),
                to: PortRef::new(inv, "input"),
            },
            supersilvia::graph::Connection {
                from: PortRef::new(inv, "output"),
                to: PortRef::new(out, "input"),
            },
        ]
    );
    assert!(matches!(
        h.state().history().last(),
        Some(Command::Splice { .. })
    ));
    h.state_mut().undo();
    h.run_steps(2);
    assert_eq!(
        h.state().graph().connections(),
        &[supersilvia::graph::Connection {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        }],
        "one undo puts the cable back"
    );
}

/// **A refused port says why.** Held over a port it cannot land on, a cable is told in words:
/// a loop, a node feeding itself.
#[test]
fn a_dimmed_port_says_why_it_will_not_take_the_cable() {
    let mut h = harness();
    on_a_plane(&mut h);
    let a = add_at(&mut h, "invert", Pos2::new(40.0, 200.0));
    let b = add_at(&mut h, "invert", Pos2::new(400.0, 200.0));
    connect(&mut h, (a, "output"), (b, "input"));
    let from = port(&h, &format!("invert{b}.output (varying color output)"));
    let back = port(&h, &format!("invert{a}.input (varying color input)"));
    hold_and_move(&mut h, from, back);
    h.run_steps(2);
    assert!(
        h.query_by_label("would make a loop").is_some(),
        "the loop is named"
    );

    let own = port(&h, &format!("invert{b}.input (varying color input)"));
    move_to(&mut h, own);
    h.run_steps(2);
    assert!(h.query_by_label("would make a loop").is_none());
    assert!(h.query_by_label("a node can't feed itself").is_some());
    release_at(&mut h, own);
    assert_eq!(h.state().graph().connections().len(), 1, "nothing landed");
}
