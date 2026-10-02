// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: `assets/`, the project's media.
//!
//! Every file that reaches a node is copied in and referenced as `assets/<name>`, so the
//! folder can be moved, zipped and sent and every reference inside it still resolves. These
//! tests are about that promise: what import does, what a reference means, and who is using
//! one.
//!
//! Each test works in its own directory under the system temp, so nothing here reaches a
//! real project.

use emath::Pos2;
use std::path::{Path, PathBuf};
use supersilvia::graph::Graph;
use supersilvia::nodes;
use supersilvia::project::{self, Project};

/// A directory of this test's own, empty, named so two tests running at once cannot collide.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-assets-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A file of the given bytes, somewhere outside any project.
fn file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

fn names(project: &Project) -> Vec<String> {
    project.assets().into_iter().map(|a| a.name).collect()
}

#[test]
fn an_imported_file_is_copied_in_and_named_by_a_reference() {
    let dir = dir("import");
    let source = file(&dir, "elsewhere/gumbasia.webm", b"a clip");
    let project = Project::new(dir.join("friday"));

    let reference = project.import_asset(&source).unwrap();

    assert_eq!(reference, "assets/gumbasia.webm");
    let copied = dir.join("friday/assets/gumbasia.webm");
    assert_eq!(std::fs::read(&copied).unwrap(), b"a clip");
    // The original is never touched.
    assert!(source.is_file());
    assert_eq!(project.resolve(&reference), copied);

    let listed = project.assets();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "gumbasia.webm");
    assert_eq!(listed[0].size, 6);
    assert_eq!(listed[0].reference, reference);
}

#[test]
fn two_files_of_the_same_name_get_a_numeric_suffix() {
    let dir = dir("collision");
    let project = Project::new(dir.join("friday"));

    let one = project
        .import_asset(&file(&dir, "one/clip.webm", b"first"))
        .unwrap();
    let two = project
        .import_asset(&file(&dir, "two/clip.webm", b"second"))
        .unwrap();

    assert_eq!(one, "assets/clip.webm");
    assert_eq!(two, "assets/clip-2.webm");
    assert_eq!(names(&project), ["clip-2.webm", "clip.webm"]);
    assert_eq!(std::fs::read(project.resolve(&two)).unwrap(), b"second");
}

#[test]
fn the_same_content_imported_twice_is_one_asset() {
    let dir = dir("dedupe");
    let project = Project::new(dir.join("friday"));

    // Same bytes, different names, different folders: one file in the project.
    let one = project
        .import_asset(&file(&dir, "one/clip.webm", b"the same bytes"))
        .unwrap();
    let two = project
        .import_asset(&file(&dir, "two/copy-of-clip.webm", b"the same bytes"))
        .unwrap();

    assert_eq!(one, two);
    assert_eq!(names(&project), ["clip.webm"]);
}

#[test]
fn a_file_already_in_the_project_is_not_copied_again() {
    let dir = dir("already");
    let project = Project::new(dir.join("friday"));
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();

    let again = project.import_asset(&project.resolve(&reference)).unwrap();

    assert_eq!(again, reference);
    assert_eq!(names(&project), ["clip.webm"]);
}

#[test]
fn a_reference_resolves_after_the_folder_moves() {
    let dir = dir("moved");
    let here = dir.join("friday");
    let project = Project::new(here.clone());
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();

    // A project folder is moved, zipped, sent. Nothing inside it is rewritten.
    let there = dir.join("saturday");
    std::fs::rename(&here, &there).unwrap();
    let moved = Project::new(there.clone());

    assert!(!project.resolve(&reference).exists());
    assert_eq!(
        moved.resolve(&reference),
        there.join("assets").join("clip.webm")
    );
    assert_eq!(std::fs::read(moved.resolve(&reference)).unwrap(), b"a clip");
}

#[test]
fn a_reference_is_joined_natively_and_a_path_outside_is_its_own_answer() {
    let project = Project::new(PathBuf::from("/projects/friday"));

    // Forward slashes in the reference, the platform's separator in the path.
    assert_eq!(
        project.resolve("assets/sub/clip.webm"),
        Path::new("/projects/friday")
            .join("assets")
            .join("sub")
            .join("clip.webm")
    );
    // Somebody typed a path into the option by hand. It means what it says.
    assert_eq!(
        project.resolve("/mnt/media/clip.webm"),
        Path::new("/mnt/media/clip.webm")
    );
    assert_eq!(project.resolve(""), Path::new(""));
}

#[test]
fn the_cache_is_in_the_project_and_is_not_an_asset() {
    let dir = dir("cache");
    let project = Project::new(dir.join("friday"));
    project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();
    // A transcode lands beside the media it came from, not in $XDG_CACHE_HOME.
    assert_eq!(project.cache_dir(), dir.join("friday").join("cache"));
    std::fs::create_dir_all(project.cache_dir()).unwrap();
    std::fs::write(project.cache_dir().join("0123.mp4"), b"a transcode").unwrap();

    assert_eq!(names(&project), ["clip.webm"]);
}

#[test]
fn saving_touches_nothing_under_assets_and_a_fork_carries_them_but_not_the_cache() {
    let dir = dir("save");
    let root = dir.join("friday");
    let mut project = Project::new(root.clone());
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();
    std::fs::create_dir_all(project.cache_dir()).unwrap();
    std::fs::write(project.cache_dir().join("0123.mp4"), b"a transcode").unwrap();
    let unreferenced = file(&root, "assets/spare.png", b"nothing points at me");

    let graph = Graph::new();
    project.save(&graph).unwrap();
    project.save(&graph).unwrap();

    // Save deletes nothing under assets/: removing one is a gesture, not a side effect.
    assert!(unreferenced.is_file());
    assert!(project.resolve(&reference).is_file());
    assert!(project.cache_dir().join("0123.mp4").is_file());

    // Save as: the folder is copied, and the media goes with it; the transcodes are made
    // again in the copy if a clip there plays.
    let forked = project.fork_to(dir.join("saturday")).unwrap();
    assert_eq!(
        std::fs::read(forked.resolve(&reference)).unwrap(),
        b"a clip"
    );
    assert!(!forked.cache_dir().exists());
}

#[test]
fn usage_is_every_free_text_option_holding_a_reference() {
    let dir = dir("users");
    let root = dir.join("friday");
    let project = Project::new(root.clone());
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();

    let mut graph = Graph::new();
    let video = nodes::add_to_graph(&mut graph, "video", Pos2::ZERO).unwrap();
    let other = nodes::add_to_graph(&mut graph, "video", Pos2::new(300.0, 0.0)).unwrap();
    // A node with nothing chosen, and one pointed at a file outside the project.
    let empty = nodes::add_to_graph(&mut graph, "video", Pos2::new(600.0, 0.0)).unwrap();
    graph
        .get_mut(video)
        .unwrap()
        .options
        .insert("file", reference.clone());
    graph
        .get_mut(other)
        .unwrap()
        .options
        .insert("file", "/mnt/media/loose.webm".to_string());
    graph
        .get_mut(empty)
        .unwrap()
        .options
        .insert("file", String::new());

    let users = project::asset_users(&graph);

    assert_eq!(users[&reference], [(video, "file")]);
    assert_eq!(users["/mnt/media/loose.webm"], [(other, "file")]);
    assert_eq!(users.len(), 2, "an empty option is not a use");
    // `size` is a select, not free text, so it is never a reference however it is set.
    assert!(!users.contains_key("1080"));
}

#[test]
fn removing_an_asset_is_refused_while_something_uses_it() {
    let dir = dir("remove-used");
    let project = Project::new(dir.join("friday"));
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();

    let mut graph = Graph::new();
    let video = nodes::add_to_graph(&mut graph, "video", Pos2::ZERO).unwrap();
    graph
        .get_mut(video)
        .unwrap()
        .options
        .insert("file", reference.clone());

    let refused = project.remove_asset(&graph, &reference).unwrap_err();
    assert!(
        refused.contains("video1.file"),
        "and it says by what: {refused}"
    );
    assert!(
        project.resolve(&reference).is_file(),
        "the file is still there"
    );

    // Point the node somewhere else, and it goes.
    graph
        .get_mut(video)
        .unwrap()
        .options
        .insert("file", String::new());
    project.remove_asset(&graph, &reference).unwrap();
    assert!(!project.resolve(&reference).is_file());
    assert!(names(&project).is_empty());
}

/// What was derived from an asset goes with it: a transcode of something that is gone is
/// bytes nobody will ever ask for.
#[test]
fn removing_an_asset_takes_its_cache_entries_with_it() {
    let dir = dir("remove-cache");
    let project = Project::new(dir.join("friday"));
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();
    let source = project.resolve(&reference);

    // The two things a clip leaves behind, made where `clip` would make them.
    let cache = project.cache_dir();
    std::fs::create_dir_all(&cache).unwrap();
    let Some(codec) = supersilvia::video::clip::Codec::probe() else {
        eprintln!("no hardware codec pair here; skipping");
        return;
    };
    let settings = supersilvia::video::clip::Settings {
        max_height: 1080,
        codec,
    };
    let transcode = supersilvia::video::clip::cache_path(&cache, &source, settings).unwrap();
    let audio = supersilvia::video::clip::audio_cache_path(&cache, &source).unwrap();
    std::fs::write(&transcode, b"transcoded").unwrap();
    std::fs::write(&audio, b"decoded").unwrap();
    // And something belonging to another asset, which must be left alone.
    let other = cache.join("ffffffffffffffff-0000000000000000.mp4");
    std::fs::write(&other, b"somebody else's").unwrap();

    project.remove_asset(&Graph::new(), &reference).unwrap();

    assert!(!transcode.is_file(), "the transcode went with it");
    assert!(!audio.is_file(), "and the decoded soundtrack");
    assert!(other.is_file(), "and nothing else did");
}

#[test]
fn removing_something_that_is_not_an_asset_is_refused() {
    let dir = dir("remove-outside");
    let project = Project::new(dir.join("friday"));
    let refused = project
        .remove_asset(&Graph::new(), "/mnt/media/loose.webm")
        .unwrap_err();
    assert!(
        refused.contains("not one of this project's assets"),
        "{refused}"
    );
}

#[test]
fn a_saved_workspace_writes_the_reference_with_forward_slashes() {
    let dir = dir("saved-reference");
    let root = dir.join("friday");
    let mut project = Project::new(root.clone());
    let reference = project
        .import_asset(&file(&dir, "elsewhere/clip.webm", b"a clip"))
        .unwrap();

    let mut graph = Graph::new();
    let video = nodes::add_to_graph(&mut graph, "video", Pos2::ZERO).unwrap();
    graph
        .get_mut(video)
        .unwrap()
        .options
        .insert("file", reference.clone());
    project.save(&graph).unwrap();

    let written = std::fs::read_dir(root.join(project::WORKSPACES))
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect::<String>();
    assert!(
        written.contains("assets/clip.webm"),
        "the file holds the reference, not a path into this machine: {written}"
    );
    assert!(!written.contains(root.to_str().unwrap()));
}
