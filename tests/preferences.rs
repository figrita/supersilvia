// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the preferences file.
//!
//! Every test writes under its own directory in the system temp, and the store under test is
//! given that path explicitly, so nothing here can touch a real `preferences.json`.

use std::path::PathBuf;
use supersilvia::preferences::{
    Placement, Preferences, StatusFolds, Store, TextSize, WindowGeometry,
};

/// A directory of this test's own, named so two tests running at once cannot collide.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssw-prefs-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_defaults_are_what_the_app_did_before_there_was_a_file() {
    let prefs = Preferences::default();
    assert!(!prefs.show_status_box, "the readout is off");
    assert_eq!(prefs.window, WindowGeometry::default());
    assert_eq!(
        prefs.window.size,
        [1280.0, 720.0],
        "the size main.rs used to hardcode"
    );
    assert_eq!(prefs.window.position, None, "the window manager decides");
    assert!(!prefs.window.maximized);
    assert!(prefs.recent.is_empty());
}

#[test]
fn a_file_round_trips() {
    let path = dir("round-trip").join("preferences.json");

    let mut written = Preferences {
        show_status_box: true,
        show_fps: true,
        // Every field that is not a default, so a new one added without a thought for the
        // file shows up here rather than on somebody's disk.
        port_hover_highlight: false,
        cable_droop: false,
        phi_cables: true,
        node_shadow: false,
        scroll_x_inverted: true,
        default_layout: supersilvia::graph::LayoutMode::Linear,
        status_folds: StatusFolds {
            gpu: true,
            editor: false,
            all_outputs: true,
            ..StatusFolds::default()
        },
        window: WindowGeometry {
            size: [900.0, 600.0],
            position: Some([12.0, 34.0]),
            maximized: true,
        },
        ui_zoom: 1.5,
        text_size: TextSize::Larger,
        projects_dir: Some(PathBuf::from("/media/shows")),
        windows: [
            (
                "Status box".to_string(),
                Placement {
                    pos: [40.0, 80.0],
                    size: Some([500.0, 700.0]),
                },
            ),
            (
                "Preferences".to_string(),
                Placement {
                    pos: [300.0, 20.0],
                    size: None,
                },
            ),
        ]
        .into(),
        ..Preferences::default()
    };
    written.push_recent(PathBuf::from("/projects/one"));
    written.save(&path);

    assert_eq!(Preferences::load(&path), written);
}

/// A zoom the file cannot mean is not handed to egui: out of range is clamped to egui's own
/// keyboard limits, and a value that is not a number is 1.
#[test]
fn a_zoom_out_of_range_starts_inside_it() {
    let at = |ui_zoom| {
        Preferences {
            ui_zoom,
            ..Preferences::default()
        }
        .zoom()
    };
    assert_eq!(at(1.3), 1.3);
    assert_eq!(at(0.0), 0.2);
    assert_eq!(at(40.0), 5.0);
    assert_eq!(at(f32::NAN), 1.0);
}

/// The text size is written as it is picked and read back by the next run, and egui's zoom
/// factor is it times the View zoom: a View zoom step moves a tenth of the View zoom and
/// leaves the text size under it.
#[test]
fn the_text_size_is_kept_and_sits_under_the_view_zoom() {
    let path = dir("text-size").join("preferences.json");
    let mut store = Store::load(Some(path.clone()));
    assert_eq!(store.get().text_size, TextSize::Default);
    assert_eq!(
        store.get().scale(),
        1.0,
        "the default is the display's own scale"
    );

    store.set_text_size(TextSize::Large);
    store.flush();
    let back = Store::load(Some(path)).get().clone();
    assert_eq!(
        back.text_size,
        TextSize::Large,
        "the next run reads it back"
    );
    assert_eq!(back.scale(), 1.25);

    let zoomed = Preferences {
        ui_zoom: supersilvia::preferences::zoom_step(back.ui_zoom, 0.1),
        ..back
    };
    assert_eq!(zoomed.ui_zoom, 1.1, "a step is a tenth of the View zoom");
    assert!(
        (zoomed.scale() - 1.25 * 1.1).abs() < 1e-6,
        "{}",
        zoomed.scale()
    );
    assert!(
        TextSize::ALL
            .windows(2)
            .all(|pair| pair[0].0.factor() < pair[1].0.factor()),
        "each size is larger than the one before"
    );
}

/// The projects folder is the default until one is chosen, and choosing the default's own
/// path stores no choice, so the file keeps following the documents folder.
#[test]
fn the_projects_folder_is_the_default_until_another_is_chosen() {
    let default = supersilvia::project::default_projects_dir();
    assert_eq!(Preferences::default().projects_dir(), default);

    let mut store = Store::in_memory();
    store.set_projects_dir(PathBuf::from("/media/shows"));
    assert_eq!(
        store.get().projects_dir(),
        Some(PathBuf::from("/media/shows"))
    );
    if let Some(default) = default {
        store.set_projects_dir(default.clone());
        assert_eq!(store.get().projects_dir, None, "the default is no choice");
        assert_eq!(store.get().projects_dir(), Some(default));
    }
}

#[test]
fn a_missing_file_is_the_defaults() {
    let path = dir("missing").join("nothing-here.json");
    assert_eq!(Preferences::load(&path), Preferences::default());
}

#[test]
fn a_truncated_file_is_the_defaults() {
    let path = dir("truncated").join("preferences.json");
    std::fs::write(&path, r#"{"show_status_box": tr"#).unwrap();
    assert_eq!(Preferences::load(&path), Preferences::default());
}

#[test]
fn a_file_from_a_newer_build_is_the_defaults() {
    let path = dir("newer").join("preferences.json");
    std::fs::write(&path, r#"{"version": 99, "show_status_box": true}"#).unwrap();
    assert_eq!(
        Preferences::load(&path),
        Preferences::default(),
        "a version this build cannot read is not half-applied"
    );
}

#[test]
fn a_field_this_build_does_not_know_is_ignored() {
    let path = dir("unknown-field").join("preferences.json");
    // `theme` used to stand in for the unknown field here and is now a real one, so the
    // example has to be something this build genuinely has never heard of. That is the
    // point of the test: a file written by a later supersilvia still opens in this one.
    std::fs::write(
        &path,
        r#"{"show_status_box": true, "a_field_from_a_later_build": {"nested": [1, 2]}}"#,
    )
    .unwrap();
    let prefs = Preferences::load(&path);
    assert!(prefs.show_status_box, "the fields it knows still load");
    assert_eq!(prefs.window, WindowGeometry::default());
}

#[test]
fn a_file_with_one_field_defaults_the_rest() {
    let path = dir("one-field").join("preferences.json");
    std::fs::write(&path, r#"{"show_status_box": true}"#).unwrap();
    let prefs = Preferences::load(&path);
    assert!(prefs.show_status_box);
    assert_eq!(prefs.window, Preferences::default().window);
    assert!(prefs.recent.is_empty());
}

#[test]
fn a_geometry_with_one_field_defaults_the_rest() {
    let path = dir("part-geometry").join("preferences.json");
    std::fs::write(&path, r#"{"window": {"maximized": true}}"#).unwrap();
    let prefs = Preferences::load(&path);
    assert!(prefs.window.maximized);
    assert_eq!(prefs.window.size, [1280.0, 720.0]);
}

#[test]
fn push_recent_moves_a_repeat_to_the_front_rather_than_repeating_it() {
    let mut prefs = Preferences::default();
    for name in ["a", "b", "c"] {
        prefs.push_recent(PathBuf::from(format!("/projects/{name}")));
    }
    assert_eq!(
        prefs.recent,
        ["/projects/c", "/projects/b", "/projects/a"].map(PathBuf::from),
        "most recent first"
    );

    prefs.push_recent(PathBuf::from("/projects/a"));
    assert_eq!(
        prefs.recent,
        ["/projects/a", "/projects/c", "/projects/b"].map(PathBuf::from),
        "reopening a project moves it up and does not list it twice"
    );
}

#[test]
fn recent_is_capped_at_ten() {
    let mut prefs = Preferences::default();
    for n in 0..25 {
        prefs.push_recent(PathBuf::from(format!("/projects/{n}")));
    }
    assert_eq!(prefs.recent.len(), 10);
    assert_eq!(prefs.recent[0], PathBuf::from("/projects/24"));
    assert_eq!(prefs.recent[9], PathBuf::from("/projects/15"));
}

#[test]
fn a_store_writes_what_changed_and_only_when_something_did() {
    let path = dir("store").join("preferences.json");

    let mut store = Store::load(Some(path.clone()));
    store.flush();
    assert!(!path.exists(), "an untouched store writes nothing");

    store.set_show_status_box(false);
    store.flush();
    assert!(
        !path.exists(),
        "setting a value to what it already is is not a change"
    );

    store.set_show_status_box(true);
    store.flush();
    assert!(Store::load(Some(path.clone())).get().show_status_box);

    store.set_window(WindowGeometry {
        size: [640.0, 480.0],
        position: Some([1.0, 2.0]),
        maximized: false,
    });
    store.flush();
    let back = Store::load(Some(path)).get().clone();
    assert_eq!(back.window.size, [640.0, 480.0]);
    assert!(back.show_status_box, "the earlier change is still there");
}

/// What the Status box has folded survives a relaunch: the store that folded it writes it,
/// and the next run's store reads it back.
#[test]
fn the_status_boxs_folds_survive_a_relaunch() {
    let path = dir("folds").join("preferences.json");
    let defaults = StatusFolds::default();
    assert!(
        !defaults.gpu && !defaults.cpu && !defaults.outputs,
        "what is read first is open"
    );
    assert!(
        defaults.editor && defaults.nodes && defaults.project,
        "and the rest folded"
    );

    let mut store = Store::load(Some(path.clone()));
    let folds = StatusFolds {
        cpu: true,
        nodes: false,
        all_outputs: true,
        ..defaults
    };
    store.set_status_folds(folds);
    store.flush();
    assert_eq!(Store::load(Some(path)).get().status_folds, folds);
}

/// **A hand edit gone wrong is kept, not overwritten.** A file that is there and cannot be
/// read loads as the defaults and stays as it is until something changes; the first write
/// renames it `preferences.json.bad` and writes the new file beside it.
#[test]
fn an_unreadable_file_is_kept_aside_before_the_first_write() {
    let path = dir("bad").join("preferences.json");
    let edited = "{\"show_status_box\": tru, \"cable_droop\": false}";
    std::fs::write(&path, edited).unwrap();

    let mut store = Store::load(Some(path.clone()));
    assert_eq!(
        store.get(),
        &Preferences::default(),
        "the defaults, with a warning"
    );
    store.flush();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        edited,
        "nothing changed, so nothing is written over it"
    );

    store.set_show_costs(true);
    store.flush();
    let bad = supersilvia::preferences::bad_path(&path);
    assert_eq!(bad.file_name().unwrap(), "preferences.json.bad");
    assert_eq!(
        std::fs::read_to_string(&bad).unwrap(),
        edited,
        "the edit is kept"
    );
    assert!(
        Preferences::load(&path).show_costs,
        "and the new file is written"
    );

    // Once kept, it is not kept again over the next write.
    std::fs::write(&bad, "older").unwrap();
    store.set_show_costs(false);
    store.flush();
    assert_eq!(std::fs::read_to_string(&bad).unwrap(), "older");
}

/// The file can be opened in an editor before anything has changed: asking for it writes the
/// defaults where there was nothing, and leaves a file that is there alone.
#[test]
fn a_missing_file_is_written_when_it_is_asked_for() {
    let path = dir("write-if-missing").join("preferences.json");
    let mut store = Store::load(Some(path.clone()));
    store.write_if_missing();
    assert_eq!(Preferences::load(&path), Preferences::default());

    std::fs::write(&path, "{\"show_costs\": true}").unwrap();
    store.write_if_missing();
    assert!(
        Preferences::load(&path).show_costs,
        "a file that is there is not rewritten"
    );
}

#[test]
fn a_store_with_nowhere_to_write_keeps_the_preferences_in_memory() {
    let mut store = Store::in_memory();
    store.set_show_status_box(true);
    store.flush();
    assert!(store.get().show_status_box, "the run keeps going");
}

/// A preference is not an edit: it goes nowhere near the command bus, so it is not in the
/// history and there is nothing to undo.
#[test]
fn saving_and_opening_record_a_recent_project_without_touching_the_undo_history() {
    use emath::Pos2;
    use supersilvia::{App, Command};

    let root = dir("through-app").join("friday");

    let mut app = App::headless();
    app.new_project(root.clone());
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let commands = app.history().len();

    app.save_project().unwrap();
    assert_eq!(
        app.preferences().recent,
        std::slice::from_ref(&root),
        "saving remembers the folder"
    );
    assert_eq!(app.history().len(), commands, "and issues no command");

    // Undo goes back past the node that was added, and no further: saving left no step.
    assert!(app.undo());
    assert!(!app.can_undo());

    let mut other = App::headless();
    other.open_project(root.clone());
    assert_eq!(other.preferences().recent, std::slice::from_ref(&root));
    assert!(
        other.history().is_empty(),
        "opening a project is not a command either"
    );
}
