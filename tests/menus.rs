// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 2: the menu bar's own help — undo by name, the Undo History, the keyboard shortcuts
//! window, a greyed entry's reason, the editor's zoom and the Nodes button's keys — driven
//! headless through the accessibility tree, as `tests/ui.rs` drives the rest.
//!
//! No test here takes a snapshot, so no harness reaches the GPU: kittest's lazy renderer is
//! never asked for a frame. `step()` only, never `run()`, for the reason `tests/ui.rs` gives.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use emath::Pos2;
use supersilvia::graph::{ControlValue, WorkspaceKind};
use supersilvia::project::Active;
use supersilvia::{App, Command};

fn app(cc: &mut eframe::CreationContext<'_>) -> App {
    supersilvia::video::ndi::pretend_missing(true);
    supersilvia::platform::syphon::pretend_unavailable(true);
    App::new(cc, supersilvia::preferences::Store::in_memory())
}

fn harness<'a>() -> Harness<'a, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(app);
    h.step();
    h
}

/// Open a menu on the bar, past its sizing pass.
fn open(h: &mut Harness<'_, App>, menu: &str) {
    h.get_by_label(menu).click();
    h.run_steps(2);
}

/// A key with these modifiers held, and a frame to answer it.
fn chord(h: &mut Harness<'_, App>, modifiers: egui::Modifiers, key: egui::Key) {
    h.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    h.run_steps(2);
}

/// A checkerboard on the workspace showing, through the bus.
fn checkerboard(h: &mut Harness<'_, App>) -> supersilvia::graph::NodeId {
    let workspace = h
        .state()
        .active_workspace()
        .expect("a workspace is showing");
    h.state_mut()
        .apply(Command::AddNode {
            slug: "checkerboard",
            at: Pos2::new(40.0, 40.0),
            workspace,
        })
        .unwrap();
    h.run_steps(2);
    h.state()
        .graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

/// Edit's Undo says what it would take back, and `Ctrl+Z` says what it took back in a toast
/// with nothing to go to when the change is in view.
#[test]
fn undo_says_what_it_takes_back() {
    let mut h = harness();
    checkerboard(&mut h);
    open(&mut h, "Edit");
    assert!(
        h.query_by_label_contains("Undo Add Checkerboard").is_some(),
        "Edit ▸ Undo names the step"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    chord(&mut h, egui::Modifiers::COMMAND, egui::Key::Z);
    assert!(h.state().graph().is_empty());
    assert_eq!(h.state().toast(), Some("Undid Add Checkerboard"));
    assert_eq!(
        h.state().toast_goes(),
        None,
        "the workspace is the one showing"
    );
    assert!(h.query_by_label("go").is_none());

    chord(
        &mut h,
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    assert_eq!(h.state().graph().len(), 1);
    assert_eq!(h.state().toast(), Some("Redid Add Checkerboard"));
}

/// An undo whose change is on another workspace says so with a ▸ go, and the go opens that
/// workspace on the node.
#[test]
fn an_undo_on_another_workspace_goes_there() {
    let mut h = harness();
    let node = checkerboard(&mut h);
    let home = h.state().active_workspace().unwrap();
    h.state_mut()
        .apply(Command::SetControl {
            node,
            key: "frequency",
            value: ControlValue::Float(4.0),
        })
        .unwrap();
    h.state_mut().end_gesture();
    h.state_mut()
        .apply(Command::AddWorkspace {
            name: "Elsewhere".to_owned(),
            kind: WorkspaceKind::Video,
            layout: supersilvia::graph::LayoutMode::default(),
            seed: supersilvia::command::Seed::Empty,
        })
        .unwrap();
    let elsewhere = h.state().graph().workspaces().last().unwrap().id;
    h.state_mut().open_workspace(elsewhere);
    h.run_steps(2);
    // Undo the new workspace, then the knob on the one left behind.
    chord(&mut h, egui::Modifiers::COMMAND, egui::Key::Z);
    h.state_mut().activate(Active::Project);
    h.run_steps(2);
    chord(&mut h, egui::Modifiers::COMMAND, egui::Key::Z);
    assert_eq!(
        h.state().toast(),
        Some("Undid Change Checkerboard Frequency")
    );
    h.run_steps(2);
    h.get_by_label("go").click();
    h.run_steps(2);
    assert_eq!(h.state().active(), Active::Workspace(home));
    assert_eq!(
        h.state().toasts(),
        ["Undid New workspace Elsewhere"],
        "the toast went with the go, and the undo before it still stands"
    );
}

/// Edit ▸ Undo History… lists the steps by name, and a click on one walks the ring to it.
#[test]
fn the_undo_history_goes_to_a_step() {
    let mut h = harness();
    let node = checkerboard(&mut h);
    h.state_mut()
        .apply(Command::RemoveNodes(vec![node]))
        .unwrap();
    h.run_steps(2);
    open(&mut h, "Edit");
    h.get_by_label_contains("Undo History").click();
    h.run_steps(3);
    assert!(h.query_by_label("Undo History").is_some(), "the window");
    assert!(h.query_by_label_contains("1  Add Checkerboard").is_some());
    assert!(
        h.query_by_label_contains("2  Delete Checkerboard")
            .is_some()
    );

    h.get_by_label_contains("0  Start").click();
    h.run_steps(2);
    assert!(h.state().graph().is_empty(), "back before the add");
    assert_eq!(h.state().undo_len(), 0);
    assert_eq!(
        h.state().toast(),
        Some("Undid 2 steps, through Add Checkerboard")
    );

    h.get_by_label_contains("1  Add Checkerboard").click();
    h.run_steps(2);
    assert_eq!(h.state().graph().len(), 1, "forward to the add");
    assert_eq!(h.state().undo_len(), 1);
}

/// Help ▸ Keyboard shortcuts… and `F1` open the one window, grouped where the keys are
/// answered, and its ✕ closes it.
#[test]
fn the_shortcuts_window_opens_from_help_and_f1() {
    let mut h = harness();
    open(&mut h, "Help");
    h.get_by_label_contains("Keyboard shortcuts…").click();
    h.run_steps(3);
    assert!(h.query_by_label("Keyboard shortcuts").is_some());
    for group in supersilvia::ui::shortcuts::TABLE {
        assert!(
            h.query_by_label(group.title).is_some(),
            "{} is a group in the window",
            group.title
        );
    }
    assert!(
        h.query_by_label("Hide the editor, leaving the mix")
            .is_some()
    );
    h.get_by_label("Close window").click();
    h.run_steps(2);
    assert!(h.query_by_label("Keyboard shortcuts").is_none(), "closed");

    chord(&mut h, egui::Modifiers::NONE, egui::Key::F1);
    h.step();
    assert!(
        h.query_by_label("Keyboard shortcuts").is_some(),
        "F1 opens it"
    );
}

/// A greyed-out entry says why on its hover.
#[test]
fn a_greyed_entry_says_why_on_hover() {
    let mut h = harness();
    open(&mut h, "Edit");
    h.get_by_label_contains("Paste").hover();
    h.run_steps(4);
    assert!(
        h.query_by_label_contains(supersilvia::ui::menu::why::EMPTY_CLIPBOARD)
            .is_some(),
        "Paste says the clipboard is empty"
    );
}

/// View's zoom entries zoom the canvas and leave the editor's own size, egui's zoom factor,
/// alone: that is the interface size's.
#[test]
fn view_zooms_the_canvas() {
    let mut h = harness();
    open(&mut h, "View");
    h.get_by_label_contains("Zoom in").click();
    h.run_steps(2);
    let zoom = h.state().canvas_transform().zoom;
    assert!((zoom - 1.25).abs() < 1e-3, "{zoom}");
    assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-3);
    open(&mut h, "View");
    h.get_by_label_contains("Actual size").click();
    h.run_steps(2);
    assert!((h.state().canvas_transform().zoom - 1.0).abs() < 1e-3);
}

/// The Nodes button's hover gives the two keys that reach the library without it.
#[test]
fn the_nodes_button_names_its_keys() {
    let mut h = harness();
    h.get_by_label("Nodes").hover();
    h.run_steps(4);
    assert!(
        h.query_by_label_contains("N opens this menu; / searches it")
            .is_some()
    );
}
