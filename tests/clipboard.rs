// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the clipboard, and what a paste plants.
//!
//! The clipboard is session state — copying is not a command and never enters the history —
//! but a paste is, and it carries what the copy took inside it. That is what these tests
//! lean on: the payload is self-contained, so a paste replays against the graph rather than
//! against whatever the clipboard holds by the time it is applied.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::project::Active;
use supersilvia::ui::ClipAction;
use supersilvia::{App, Command, CommandError};

fn add(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
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

fn move_to(app: &mut App, node: NodeId, at: Pos2) {
    app.apply(Command::MoveNodes {
        moves: vec![(node, at)],
    })
    .unwrap();
}

/// A spiral into a zoom into an Output, with the spiral dialed away from its defaults and
/// the zoom collapsed. The pair is what gets copied; the Output is what stays behind.
fn patch(app: &mut App) -> (NodeId, NodeId, NodeId) {
    let spiral = add(app, "spiral");
    let zoom = add(app, "zoom");
    let out = add(app, "output");
    wire(app, (spiral, "color"), (zoom, "input"));
    wire(app, (zoom, "output"), (out, "input"));
    move_to(app, spiral, Pos2::new(10.0, 20.0));
    move_to(app, zoom, Pos2::new(60.0, 120.0));
    app.apply(Command::SetControl {
        node: spiral,
        key: "turns",
        value: ControlValue::Float(7.0),
    })
    .unwrap();
    app.apply(Command::SetOption {
        node: spiral,
        key: "type",
        value: "logarithmic".to_string(),
    })
    .unwrap();
    app.apply(Command::SetCollapsed {
        nodes: vec![zoom],
        collapsed: true,
    })
    .unwrap();
    (spiral, zoom, out)
}

/// The nodes made by the most recent paste, which is also the selection it left behind.
fn pasted(app: &App) -> Vec<NodeId> {
    app.canvas_selection()
}

#[test]
fn a_copy_and_a_paste_carry_the_controls_options_and_collapse_and_only_the_inside_cables() {
    let mut app = App::headless();
    let (spiral, zoom, out) = patch(&mut app);

    app.select_only(&[spiral, zoom]);
    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));
    assert!(app.can_paste(), "the copy left nothing on the clipboard");
    app.clipboard_action(ClipAction::Paste {
        at: Some(Pos2::new(200.0, 200.0)),
    });

    assert_eq!(app.graph().len(), 5, "two copies were made");
    let made = pasted(&app);
    let (new_spiral, new_zoom) = (made[0], made[1]);

    let copy = app.graph().get(new_spiral).expect("the spiral's copy");
    assert_eq!(
        copy.controls.get("turns"),
        Some(&ControlValue::Float(7.0)),
        "the copy lost its control value"
    );
    assert_eq!(
        copy.options.get("type").map(String::as_str),
        Some("logarithmic"),
        "the copy lost its option"
    );
    assert!(
        app.graph()
            .get(new_zoom)
            .expect("the zoom's copy")
            .collapsed,
        "the copy came back expanded"
    );

    // The anchor landed on the paste point and the second node kept its place relative to
    // it: a patch arrives as the patch it was.
    assert_eq!(copy.pos, Pos2::new(200.0, 200.0));
    assert_eq!(
        app.graph().get(new_zoom).expect("the zoom's copy").pos,
        Pos2::new(250.0, 300.0)
    );

    // The cable between the two copies exists...
    assert_eq!(
        app.graph().source_of(PortRef::new(new_zoom, "input")),
        Some(PortRef::new(new_spiral, "color")),
        "the internal cable was not carried"
    );
    // ...and the one that left the set did not follow, so the Output still has exactly one
    // source and it is still the original.
    assert_eq!(
        app.graph().source_of(PortRef::new(out, "input")),
        Some(PortRef::new(zoom, "output")),
        "pasting stole or doubled an outside connection"
    );
}

/// Copying is not an edit. It changes no graph state, so there is nothing about it to undo.
#[test]
fn a_copy_is_not_a_command() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);
    let history = app.history().len();

    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));

    assert_eq!(app.history().len(), history, "a copy reached the bus");
}

/// A pasted node belongs to the workspace being looked at, whatever the one it was copied
/// from was.
#[test]
fn a_paste_lands_on_the_workspace_being_looked_at() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);
    let first = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Workspace 2".into(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let second: WorkspaceId = app.graph().workspaces().last().expect("two of them").id;

    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));
    app.activate(Active::Workspace(second));
    app.clipboard_action(ClipAction::Paste { at: None });

    for id in pasted(&app) {
        let node = app.graph().get(id).expect("a pasted node");
        assert!(
            node.workspaces.contains(&second) && !node.workspaces.contains(&first),
            "a paste put its nodes on the workspace they were copied from"
        );
    }
}

/// Snapshot undo covers a paste with no paste-specific code, and the whole of it goes.
#[test]
fn a_paste_is_exactly_one_undo_step() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);
    let before = app.graph().len();
    let cables = app.graph().connections().len();

    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));
    app.clipboard_action(ClipAction::Paste { at: None });
    assert_eq!(app.graph().len(), before + 2);

    assert!(app.undo());
    assert_eq!(app.graph().len(), before, "undo left a node behind");
    assert_eq!(
        app.graph().connections().len(),
        cables,
        "undo left a cable behind"
    );
}

/// A cut is the copy and the delete, and the delete is one `RemoveNodes` — so it is one undo
/// step, and what it took is still there to be planted again.
#[test]
fn a_cut_removes_the_nodes_and_leaves_them_pasteable() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);
    let steps = app.undo_len();

    app.clipboard_action(ClipAction::Cut(vec![spiral, zoom]));

    assert!(app.graph().get(spiral).is_none(), "the cut left the spiral");
    assert!(app.graph().get(zoom).is_none(), "the cut left the zoom");
    assert_eq!(
        app.graph().len(),
        1,
        "the Output should be all that is left"
    );
    assert_eq!(app.undo_len(), steps + 1, "a cut is one undo step");

    assert!(app.can_paste());
    app.clipboard_action(ClipAction::Paste { at: None });
    assert_eq!(app.graph().len(), 3, "the cut nodes did not come back");
    assert_eq!(
        app.graph().connections().len(),
        1,
        "the cable between them did not come back"
    );
}

/// Nothing on the clipboard is nothing to paste: no node, no command, no undo step. The
/// menus draw the entry disabled from the same answer.
#[test]
fn a_paste_with_an_empty_clipboard_does_nothing() {
    let mut app = App::headless();
    let spiral = add(&mut app, "spiral");
    let history = app.history().len();

    assert!(!app.can_paste(), "a fresh app has an empty clipboard");
    app.clipboard_action(ClipAction::Paste { at: None });

    assert_eq!(app.graph().len(), 1);
    assert_eq!(
        app.history().len(),
        history,
        "an empty paste reached the bus"
    );

    // And the command itself refuses an empty clip, so nothing can plant one behind the
    // menu's back either.
    let workspace = app.graph().default_workspace();
    assert_eq!(
        app.apply(Command::Paste {
            clip: supersilvia::command::Clip::default(),
            at: Pos2::ZERO,
            workspace,
        }),
        Err(CommandError::EmptyClip)
    );
    assert_eq!(app.graph().len(), 1);
    assert!(app.graph().get(spiral).is_some());
}

/// The payload travels inside the command, so a paste plants what was copied whatever has
/// happened to the originals since — including their deletion.
#[test]
fn a_paste_survives_the_originals_being_deleted() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);

    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));
    app.apply(Command::RemoveNodes(vec![spiral, zoom])).unwrap();
    app.clipboard_action(ClipAction::Paste { at: None });

    assert_eq!(app.graph().len(), 3, "the clip did not plant");
    assert_eq!(
        app.graph().connections().len(),
        1,
        "the cable between the copies did not plant"
    );
}

/// A paste that named no point goes back to where the copy was: the same place in the
/// window. With the view unmoved, that is exactly on top of the original — which is the
/// promise, not a miss.
#[test]
fn an_unaimed_paste_lands_where_the_copy_was_in_the_window() {
    let mut app = App::headless();
    let (spiral, zoom, _) = patch(&mut app);

    app.clipboard_action(ClipAction::Copy(vec![spiral, zoom]));
    app.clipboard_action(ClipAction::Paste { at: None });

    let made = pasted(&app);
    assert_eq!(
        app.graph().get(made[0]).expect("the copy").pos,
        app.graph().get(spiral).expect("the original").pos,
        "an unaimed paste into an unmoved view should land on the original"
    );
}

/// An Output copied with a name of its own lands under that name and ` copy`, once more for
/// each copy already there; one with the default takes its own number. A duplicate is the
/// same, and undo takes the copy and its name back together.
#[test]
fn a_copied_output_goes_out_under_a_name_of_its_own() {
    use supersilvia::nodes::output::{SEND_NAME, send_name};
    let mut app = App::headless();
    let named = add(&mut app, "output");
    let plain = add(&mut app, "output");
    app.apply(Command::SetOption {
        node: named,
        key: SEND_NAME,
        value: "warpzone".to_string(),
    })
    .unwrap();
    let name = |app: &App, id: NodeId| send_name(id, app.graph().get(id).unwrap());
    let newest = |app: &App| app.graph().iter().map(|(id, _)| id).max().unwrap();

    app.clipboard_action(ClipAction::Copy(vec![named, plain]));
    app.clipboard_action(ClipAction::Paste { at: None });
    let names: Vec<String> = app.graph().iter().map(|(id, _)| name(&app, id)).collect();
    assert_eq!(names.len(), 4);
    assert!(names.contains(&"warpzone copy".to_string()), "{names:?}");
    assert_eq!(
        names
            .iter()
            .filter(|n| n.starts_with("supersilvia Output"))
            .count(),
        2,
        "each default its own number: {names:?}"
    );

    app.apply(Command::Duplicate {
        nodes: vec![named],
        offset: emath::vec2(20.0, 20.0),
    })
    .unwrap();
    let copy = newest(&app);
    assert_eq!(name(&app, copy), "warpzone copy copy");

    app.undo();
    assert!(app.graph().get(copy).is_none());
    assert_eq!(name(&app, named), "warpzone", "the original is untouched");
}

// ---------------------------------------------------------------- across projects

/// A folder of this test's own, empty, so two tests running at once cannot collide.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-clip-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `video` node playing `bytes`, imported into the project on screen as `name`.
fn video_of(app: &mut App, dir: &std::path::Path, name: &str, bytes: &[u8]) -> NodeId {
    let source = dir.join(name);
    std::fs::write(&source, bytes).unwrap();
    let video = add(app, "video");
    let reference = app.import_asset(&source).expect("imported");
    app.apply(Command::SetOption {
        node: video,
        key: "file",
        value: reference,
    })
    .unwrap();
    video
}

fn file_of(app: &App, node: NodeId) -> String {
    app.graph().get(node).unwrap().options["file"].clone()
}

/// How many files a project's `assets/` holds.
fn assets_in(app: &App) -> usize {
    std::fs::read_dir(app.project().root().join("assets")).map_or(0, Iterator::count)
}

/// **A paste into another project brings its media.** The clipboard outlives Open, and the
/// node it carries names its file by the project it was copied in. Here the second project
/// already holds a *different* file under the same name, which is exactly what the reference
/// copied verbatim would have named: the paste copies the right bytes in beside it, under a
/// name of their own, and a second paste of the same bytes is the same asset.
#[test]
fn a_paste_into_another_project_copies_its_files_into_that_projects_assets() {
    let base = scratch("across");
    let mut app = App::headless();
    app.new_project(base.join("one"));
    let video = video_of(&mut app, &base, "gumbasia.webm", b"the one copied");
    assert_eq!(file_of(&app, video), "assets/gumbasia.webm");
    app.clipboard_action(ClipAction::Copy(vec![video]));

    app.new_project(base.join("two"));
    let elsewhere = base.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("gumbasia.webm"), b"somebody else's").unwrap();
    assert_eq!(
        app.import_asset(&elsewhere.join("gumbasia.webm")).unwrap(),
        "assets/gumbasia.webm"
    );

    app.clipboard_action(ClipAction::Paste { at: None });
    let first = pasted(&app)[0];
    assert_eq!(file_of(&app, first), "assets/gumbasia-2.webm");
    assert_eq!(
        std::fs::read(app.project().resolve("assets/gumbasia-2.webm")).unwrap(),
        b"the one copied"
    );
    assert!(
        app.file_status().contains("gumbasia-2.webm"),
        "the status says what came: {}",
        app.file_status()
    );

    app.clipboard_action(ClipAction::Paste { at: None });
    assert_eq!(file_of(&app, pasted(&app)[0]), "assets/gumbasia-2.webm");
    assert_eq!(assets_in(&app), 2, "the same bytes twice are one asset");

    // Undo takes the nodes back and leaves the file, as an import's undo does.
    app.undo();
    app.undo();
    assert_eq!(assets_in(&app), 2);
    std::fs::remove_dir_all(&base).ok();
}

/// A paste in the project the copy was taken in copies nothing: the reference already names
/// this project's own file.
#[test]
fn a_paste_in_the_project_it_was_copied_in_copies_nothing() {
    let base = scratch("same");
    let mut app = App::headless();
    app.new_project(base.join("one"));
    let video = video_of(&mut app, &base, "gumbasia.webm", b"a clip");
    app.clipboard_action(ClipAction::Copy(vec![video]));
    app.clipboard_action(ClipAction::Paste { at: None });
    assert_eq!(file_of(&app, pasted(&app)[0]), "assets/gumbasia.webm");
    assert_eq!(assets_in(&app), 1);
    assert!(
        !app.file_status().contains("pasted"),
        "{}",
        app.file_status()
    );
    std::fs::remove_dir_all(&base).ok();
}

/// A file gone from the project it was copied in before the paste is said, and the node is
/// left pointing at where it was looked for — never at a file of this project's that
/// happens to share its name.
#[test]
fn a_file_gone_before_the_paste_is_said_and_left_pointing_where_it_was() {
    let base = scratch("gone");
    let mut app = App::headless();
    app.new_project(base.join("one"));
    let video = video_of(&mut app, &base, "gumbasia.webm", b"a clip");
    app.clipboard_action(ClipAction::Copy(vec![video]));
    let gone = app.project().resolve("assets/gumbasia.webm");
    std::fs::remove_file(&gone).unwrap();

    app.new_project(base.join("two"));
    app.clipboard_action(ClipAction::Paste { at: None });
    assert_eq!(
        std::path::PathBuf::from(file_of(&app, pasted(&app)[0])),
        gone,
        "pointing at where it was"
    );
    assert!(
        app.file_status()
            .contains("could not find assets/gumbasia.webm"),
        "{}",
        app.file_status()
    );
    std::fs::remove_dir_all(&base).ok();
}
