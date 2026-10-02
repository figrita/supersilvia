// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the autosave. While there are unsaved edits the project is written into its
//! `.autosave/`, never over its own files; opening a project whose autosave is newer and
//! different offers it back, and Recover, Discard and Save each leave the folder as they say.
//!
//! The frame's clock is handed to `App::autosave_tick` by hand, and each write is waited for,
//! so nothing here depends on how long a test takes.

use emath::Pos2;
use std::path::{Path, PathBuf};
use supersilvia::graph::{ControlValue, Graph, NodeId};
use supersilvia::project::{AUTOSAVE, MANIFEST, Project};
use supersilvia::{App, Command};

/// A project folder of this test's own, not yet made.
fn root(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-autosave-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("friday")
}

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::new(40.0, 40.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// One frame's autosave at `now` seconds, landed.
fn tick(app: &mut App, now: f64) {
    app.autosave_tick(now);
    app.wait_for_autosave();
}

fn autosave(root: &Path) -> PathBuf {
    root.join(AUTOSAVE)
}

/// The graph the autosave holds.
fn autosaved(root: &Path) -> Graph {
    Project::open(autosave(root)).unwrap().1
}

/// Every file under the folder but the autosave, with its bytes.
fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path == autosave(root) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_timer_writes_only_while_there_are_unsaved_edits() {
    let root = root("timer");
    let mut app = App::headless();
    app.new_project(root.clone());
    tick(&mut app, 0.0);
    assert!(
        !autosave(&root).exists(),
        "nothing unsaved, nothing written"
    );

    add(&mut app, "checkerboard");
    tick(&mut app, 1.0);
    assert_eq!(
        autosaved(&root).len(),
        1,
        "the first unsaved edit is written"
    );

    add(&mut app, "circle");
    tick(&mut app, 20.0);
    assert_eq!(
        autosaved(&root).len(),
        1,
        "no second write inside the interval"
    );
    tick(&mut app, 31.0);
    assert_eq!(autosaved(&root).len(), 2, "and the next once it has passed");

    app.undo();
    app.undo();
    assert!(!app.dirty());
    tick(&mut app, 62.0);
    assert!(
        !autosave(&root).exists(),
        "undone back to the save, the autosave goes"
    );
}

#[test]
fn an_autosave_never_touches_the_projects_own_files() {
    let root = root("untouched");
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    app.save_project().unwrap();
    let before = files(&root);

    let node = add(&mut app, "circle");
    app.apply(Command::SetControl {
        node,
        key: "radius",
        value: ControlValue::Float(0.3),
    })
    .unwrap();
    tick(&mut app, 0.0);
    assert!(autosave(&root).join(MANIFEST).is_file());
    assert_eq!(
        files(&root),
        before,
        "every file of the project is as saved"
    );
}

/// Saved, edited, autosaved, and gone without a save: what a crash leaves.
fn crashed(name: &str) -> (PathBuf, Graph, Graph) {
    let root = root(name);
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    app.save_project().unwrap();
    let saved = app.graph().clone();

    let node = add(&mut app, "circle");
    app.apply(Command::SetControl {
        node,
        key: "radius",
        value: ControlValue::Float(0.3),
    })
    .unwrap();
    app.apply(Command::AddWorkspace {
        name: "Second".to_string(),
        kind: supersilvia::graph::WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    tick(&mut app, 0.0);
    let edited = app.graph().clone();
    drop(app);
    (root, saved, edited)
}

#[test]
fn recover_restores_the_autosave_exactly_and_leaves_it_unsaved() {
    let (root, saved, edited) = crashed("recover");
    let mut app = App::headless();
    app.open_project(root.clone());
    assert!(app.recovery_offered().is_some(), "the question is up");
    assert!(
        app.graph().same_document(&saved),
        "under it, the project as saved"
    );

    app.recover();
    assert!(app.recovery_offered().is_none());
    assert!(
        app.graph().same_document(&edited),
        "the recovered document is the one that was on screen"
    );
    assert_eq!(app.graph().workspaces().len(), 2);
    assert!(app.dirty(), "and it is not saved until it is saved");
    assert!(autosave(&root).exists(), "the autosave stays until then");

    // The next autosave writes over the one recovered, and a workspace removed since has
    // its file go with it rather than come back as a stray.
    let second = app.graph().workspaces()[1].id;
    app.apply(Command::RemoveWorkspace(second)).unwrap();
    tick(&mut app, 0.0);
    assert_eq!(autosaved(&root).workspaces().len(), 1);

    app.save_project().unwrap();
    assert!(!autosave(&root).exists(), "a save clears it");
    let mut again = App::headless();
    again.open_project(root);
    assert!(again.recovery_offered().is_none());
    assert_eq!(again.graph().workspaces().len(), 1);
}

#[test]
fn discard_deletes_the_autosave() {
    let (root, saved, _) = crashed("discard");
    let mut app = App::headless();
    app.open_project(root.clone());
    assert!(app.recovery_offered().is_some());

    app.discard_recovery();
    assert!(app.recovery_offered().is_none());
    assert!(!autosave(&root).exists());
    assert!(app.graph().same_document(&saved));
    assert!(!app.dirty());
}

/// Discard in the unsaved-edits confirm, on the way to Quit: the autosave goes and stays
/// gone for the frames before the app exits, though the edits are still on screen, so the
/// next launch asks nothing.
#[test]
fn discarding_the_edits_on_quit_leaves_no_recovery() {
    let root = root("discard-on-quit");
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    tick(&mut app, 0.0);
    assert!(autosave(&root).exists());

    app.discard_edits();
    assert!(!autosave(&root).exists());
    assert!(
        app.dirty(),
        "the edits are still on screen until the app goes"
    );
    tick(&mut app, 100.0);
    tick(&mut app, 200.0);
    assert!(
        !autosave(&root).exists(),
        "and no later frame writes them back"
    );

    let mut again = App::headless();
    again.open_project(root.clone());
    assert!(again.recovery_offered().is_none());
}

#[test]
fn a_save_clears_the_autosave() {
    let root = root("save");
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    tick(&mut app, 0.0);
    assert!(autosave(&root).exists());

    app.save_project().unwrap();
    assert!(!autosave(&root).exists());
    tick(&mut app, 100.0);
    assert!(!autosave(&root).exists(), "and nothing is unsaved to write");
}

/// An autosave that holds what the project already holds is no question: it is deleted on
/// open and nothing is asked.
#[test]
fn an_autosave_the_same_as_the_save_is_not_offered() {
    let (root, _, edited) = crashed("same");
    // The edits reach the project's own files some other way, and the autosave is still
    // the newer of the two.
    let (mut project, _, _) = Project::open(root.clone()).unwrap();
    project.save(&edited).unwrap();
    std::fs::File::options()
        .write(true)
        .open(autosave(&root).join(MANIFEST))
        .unwrap()
        .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
        .unwrap();

    let mut app = App::headless();
    app.open_project(root.clone());
    assert!(app.recovery_offered().is_none());
    assert!(!autosave(&root).exists());
}

#[test]
fn save_as_leaves_no_autosave_behind_in_either_folder() {
    let root = root("save-as");
    let elsewhere = root.with_file_name("saturday");
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    tick(&mut app, 0.0);
    assert!(autosave(&root).exists());

    app.save_project_as(elsewhere.clone());
    assert_eq!(app.project().root(), elsewhere);
    assert!(
        !autosave(&root).exists(),
        "the edits are saved, if elsewhere"
    );
    assert!(
        !autosave(&elsewhere).exists(),
        "and the copy took none with it"
    );
}

/// A lost device ends the run: what the frame last recorded is written at once, however
/// recently the timer last wrote, and the person is told it was.
#[test]
fn a_lost_device_writes_the_newest_edits_at_once() {
    let root = root("lost");
    let mut app = App::headless();
    app.new_project(root.clone());
    add(&mut app, "checkerboard");
    tick(&mut app, 0.0);
    add(&mut app, "circle");
    tick(&mut app, 1.0);
    assert_eq!(
        autosaved(&root).len(),
        1,
        "the timer has not written the second"
    );

    let said = app.save_for_lost_device("Unknown: test");
    assert!(
        said.starts_with("The GPU stopped responding. Your work was saved; restart supersilvia."),
        "{said}"
    );
    assert_eq!(autosaved(&root).len(), 2, "the newest edits are on disk");
}
