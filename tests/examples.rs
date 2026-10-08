// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the Example Library, `assets/examples/` compiled into the binary.
//!
//! Every example is held to what `scripts/examples/make.py` promises of it — it is saved as a
//! strip, loads with no warning, every cable it saved connects, it holds a note and an Output,
//! it opens no device and names no file — through the same import Help ▸ Example Library runs,
//! and to what adding one does: a strip where Auto-arrange puts it, its note first. A node changed
//! under an example fails here by the example's category and title. That every Output in one
//! compiles and reaches each shader target is `tests/shader_targets.rs`'s.

use std::collections::BTreeSet;
use std::path::PathBuf;
use supersilvia::examples::{EXAMPLES, Example};
use supersilvia::graph::{Graph, LayoutMode, NodeId, WorkspaceId, WorkspaceKind};
use supersilvia::project::Project;
use supersilvia::ui::layout::{HORIZONTAL_MARGIN, VERTICAL_MARGIN};
use supersilvia::{App, Command, workspace};

/// A strip's height, as the canvas would measure one in an ordinary window.
const STRIP: f32 = 700.0;

/// Nodes an example never holds: each opens a device or a file the moment it loads.
const FORBIDDEN: &[&str] = &[
    "camera",
    "screencapture",
    "audioin",
    "video",
    "syphon",
    "ndi",
    "imagegif",
];

fn named(example: &Example) -> String {
    format!("{}/{}", example.category, example.title)
}

/// Everything wrong with one example, imported into a project of its own.
fn problems(example: &Example) -> Vec<String> {
    let file = match workspace::parse(example.file) {
        Ok(file) => file,
        Err(e) => return vec![format!("is not a workspace file: {e}")],
    };
    let mut out = Vec::new();
    if file.layout != LayoutMode::Linear {
        out.push("is not saved as a strip".to_owned());
    }
    let project = Project::scratch();
    let mut graph = Graph::new();
    let (id, report) =
        match project.import_workspace_text(example.file, example.title, None, &mut graph) {
            Ok(imported) => imported,
            Err(e) => return vec![format!("does not import: {e}")],
        };
    out.extend(report.warnings.iter().map(|w| format!("warns: {w}")));
    out.extend(report.missing.iter().map(|m| format!("names a file: {m}")));
    // A dropped cable is a warning too; this says it in the file's own count.
    let kept = graph.connections().len();
    if kept != file.connections.len() {
        out.push(format!(
            "saved {} cables and {kept} connect",
            file.connections.len()
        ));
    }
    let slugs: BTreeSet<&str> = graph.on_workspace(id).map(|(_, n)| n.def.slug).collect();
    for wanted in ["note", "output"] {
        if !slugs.contains(wanted) {
            out.push(format!("holds no {wanted}"));
        }
    }
    for slug in FORBIDDEN.iter().filter(|s| slugs.contains(**s)) {
        out.push(format!("holds {slug}, which opens a device or a file"));
    }
    for (node, n) in graph.on_workspace(id) {
        for option in n.def.options.iter().filter(|o| o.is_asset()) {
            if n.options.get(option.key).is_some_and(|v| !v.is_empty()) {
                out.push(format!("{}{node}.{} names a file", n.def.slug, option.key));
            }
        }
    }
    out
}

/// Every bundled example imports cleanly and holds what an example holds.
#[test]
fn every_example_imports_with_no_warning_and_holds_a_note_and_an_output() {
    let failures: Vec<String> = EXAMPLES
        .iter()
        .filter_map(|e| {
            let found = problems(e);
            (!found.is_empty()).then(|| format!("{}: {}", named(e), found.join("; ")))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} examples fail:\n{}",
        failures.len(),
        EXAMPLES.len(),
        failures.join("\n")
    );
}

/// The `NN-` prefix and the extension off a name, or `None` without a prefix.
fn unnumbered(name: &str) -> Option<(u32, String)> {
    let (n, rest) = name.split_once('-')?;
    Some((n.parse().ok()?, rest.trim_end_matches(".ssw").to_owned()))
}

/// The library as the folder holds it, in the order of its prefixes.
fn on_disk() -> Vec<(String, String)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/examples");
    let sorted = |dir: &std::path::Path, files: bool| {
        let mut found: Vec<(u32, String, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                if files {
                    p.extension().is_some_and(|x| x == "ssw")
                } else {
                    p.is_dir()
                }
            })
            .filter_map(|p| {
                let (n, name) = unnumbered(&p.file_name()?.to_string_lossy())?;
                Some((n, name, p))
            })
            .collect();
        found.sort();
        found
    };
    sorted(&root, false)
        .into_iter()
        .flat_map(|(_, category, dir)| {
            sorted(&dir, true)
                .into_iter()
                .map(move |(_, title, _)| (category.clone(), title))
        })
        .collect()
}

/// The list is every file in the folder, in the order of its prefixes, each category in one
/// run and no title twice in one.
#[test]
fn the_list_is_the_folder_in_order_with_titles_unique_in_a_category() {
    assert!(!EXAMPLES.is_empty(), "the library holds something");
    let listed: Vec<(String, String)> = EXAMPLES
        .iter()
        .map(|e| (e.category.to_owned(), e.title.to_owned()))
        .collect();
    assert_eq!(
        listed,
        on_disk(),
        "the binary's list is the folder's, in order"
    );
    let mut seen = BTreeSet::new();
    let mut categories = Vec::new();
    for (category, run) in supersilvia::examples::categories() {
        assert!(
            !categories.contains(&category),
            "{category} is one run of the list"
        );
        categories.push(category);
        for example in run {
            assert!(seen.insert(named(example)), "{} twice", named(example));
        }
    }
}

/// An example keeps the kind it was written as: every Solids example is a solids workspace,
/// and adding one makes a solids tab.
#[test]
fn a_solids_example_is_a_solids_workspace_and_stays_one_when_added() {
    for e in EXAMPLES.iter().filter(|e| e.category == "Solids") {
        let file = workspace::parse(e.file).expect("a workspace file");
        assert_eq!(file.kind, WorkspaceKind::Solids, "{}", named(e));
    }
    let index = EXAMPLES
        .iter()
        .position(|e| e.category == "Solids")
        .expect("a Solids example");
    let mut app = App::headless();
    app.add_example(index);
    let id = app.active_workspace().expect("a workspace is showing");
    assert_eq!(
        app.graph().workspace(id).map(|w| w.kind),
        Some(WorkspaceKind::Solids)
    );
}

/// Adding an example puts it on a tab of its own, shown, in the project that was open, as one
/// undo step that takes all of it back.
#[test]
fn adding_an_example_is_a_tab_of_its_own_and_one_undo_step() {
    let mut app = App::headless();
    let root = app.project().root().to_path_buf();
    let (workspaces, nodes) = (app.graph().workspaces().len(), app.graph().len());
    let example = &EXAMPLES[0];

    app.add_example(0);
    let id = app.active_workspace().expect("a workspace is showing");
    assert_eq!(
        app.graph().workspace(id).map(|w| w.name.as_str()),
        Some(example.title)
    );
    assert!(app.open_workspaces().contains(&id), "its tab is open");
    assert_eq!(app.graph().workspaces().len(), workspaces + 1);
    assert_eq!(app.project().root(), root, "the same project");
    assert!(
        matches!(
            app.history().last(),
            Some(&Command::AddExample { index: 0, .. })
        ),
        "one step: {:?}",
        app.history()
    );
    assert_eq!(
        app.undo_name(),
        Some(format!("Add example {}", example.title))
    );

    assert!(app.undo());
    assert_eq!(app.graph().workspaces().len(), workspaces);
    assert_eq!(app.graph().len(), nodes);
}

/// The workspace added last, which is the last in project order.
fn newest(app: &App) -> WorkspaceId {
    app.graph().workspaces().last().expect("a workspace").id
}

/// Every node on one workspace, by id, with where it is.
fn positions(app: &App, ws: WorkspaceId) -> Vec<(NodeId, &'static str, (f32, f32))> {
    app.graph()
        .on_workspace(ws)
        .map(|(id, n)| (id, n.def.slug, (n.pos.x, n.pos.y)))
        .collect()
}

/// Every example arrives as a strip already where Auto-arrange at that height would put it,
/// so arranging it again moves nothing, with a note first in the first column, at its top.
#[test]
fn every_example_arrives_as_a_strip_arranged_with_its_note_first() {
    let mut app = App::headless();
    let mut failures = Vec::new();
    for (index, example) in EXAMPLES.iter().enumerate() {
        app.apply(Command::AddExample {
            index,
            height: STRIP,
        })
        .expect("an example adds");
        let ws = newest(&app);
        if app.graph().layout_of(ws) != LayoutMode::Linear {
            failures.push(format!("{}: is not a strip", named(example)));
        }
        let arrived = positions(&app, ws);
        app.apply(Command::AutoArrange {
            workspace: ws,
            height: STRIP,
        })
        .expect("an arrange");
        if positions(&app, ws) != arrived {
            failures.push(format!("{}: arranging again moves it", named(example)));
        }
        let first = arrived
            .iter()
            .find(|(_, slug, _)| *slug == "note")
            .map(|(_, _, at)| *at);
        if first != Some((HORIZONTAL_MARGIN, VERTICAL_MARGIN)) {
            failures.push(format!(
                "{}: its first note is at {first:?}, not the first column's top",
                named(example)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Adding an example is one undo step, and a redo puts every node back where the first run
/// did, since the height rides in the command.
#[test]
fn a_redone_example_lands_where_it_first_did() {
    let mut app = App::headless();
    let index = EXAMPLES.len() - 1;
    app.apply(Command::AddExample {
        index,
        height: STRIP,
    })
    .expect("an example adds");
    let ws = newest(&app);
    let first: Vec<_> = positions(&app, ws)
        .into_iter()
        .map(|(_, slug, at)| (slug, at))
        .collect();
    let before = app.graph().workspaces().len();

    assert!(app.undo());
    assert_eq!(
        app.graph().workspaces().len(),
        before - 1,
        "one step takes it all"
    );
    assert!(app.redo());
    let again: Vec<_> = positions(&app, newest(&app))
        .into_iter()
        .map(|(_, slug, at)| (slug, at))
        .collect();
    assert_eq!(again, first);
    assert_eq!(app.graph().layout_of(newest(&app)), LayoutMode::Linear);
}
