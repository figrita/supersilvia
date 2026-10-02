// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the project folder.
//!
//! Every test writes under its own directory in the system temp and hands the project an
//! explicit root, so nothing here can reach the real data directory. `App::headless` is the
//! same promise from the other side: its project is a scratch folder and its preferences are
//! in memory.

use emath::Pos2;
use std::collections::BTreeSet;
use std::path::PathBuf;
use supersilvia::graph::{Graph, NodeId, PortRef, WorkspaceId, WorkspaceKind};
use supersilvia::project::{self, Active, Project, View};
use supersilvia::workspace::{self, LoadError, LoadWarning};
use supersilvia::{App, Command, nodes};

/// A directory of this test's own, empty, named so two tests running at once cannot collide.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-project-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn add(g: &mut Graph, slug: &str, at: Pos2) -> NodeId {
    nodes::add_to_graph(g, slug, at).unwrap()
}

/// Three workspaces, one node shared by the first two, a cable inside the first and one
/// across to the third.
fn three_workspaces() -> (Graph, [WorkspaceId; 3], NodeId) {
    let mut g = Graph::new();
    let first = g.default_workspace();
    g.rename_workspace(first, "Tunnel".to_string());
    let second = g.add_workspace("Cameras".to_string(), WorkspaceKind::Video);
    let third = g.add_workspace("Set".to_string(), WorkspaceKind::Video);

    let shared = add(&mut g, "checkerboard", Pos2::new(10.0, 20.0));
    let zoom = add(&mut g, "zoom", Pos2::new(240.0, 20.0));
    let out = add(&mut g, "output", Pos2::new(480.0, 20.0));
    g.get_mut(shared).unwrap().workspaces.insert(second);
    g.get_mut(out).unwrap().workspaces = BTreeSet::from([third]);

    g.connect(PortRef::new(shared, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();
    (g, [first, second, third], shared)
}

fn save(root: &std::path::Path, graph: &Graph) -> Project {
    let mut project = Project::new(root.to_path_buf());
    project.save(graph).unwrap();
    project
}

fn workspace_files(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(project::WORKSPACES))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_project_round_trips_through_a_folder() {
    let root = dir("round-trip").join("friday");
    let (graph, [first, second, third], shared) = three_workspaces();
    save(&root, &graph);

    let (_, back, warnings) = Project::open(root.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");

    let names: Vec<&str> = back.workspaces().iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Tunnel", "Cameras", "Set"], "in project order");
    assert_eq!(back.len(), 3);
    assert_eq!(
        back.connections().len(),
        2,
        "the cable inside a file and the one across two came back"
    );
    assert_eq!(
        back.get(shared).unwrap().workspaces,
        BTreeSet::from([first, second]),
        "the rest of a shared node's set is glue, and the glue is read"
    );
    assert_eq!(
        back.iter()
            .find(|(_, n)| n.def.slug == "output")
            .unwrap()
            .1
            .workspaces,
        BTreeSet::from([third])
    );
}

/// The manifest as JSON, for a test that asks what is actually written where.
fn manifest(root: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(root.join(project::MANIFEST)).unwrap()).unwrap()
}

/// One workspace file as JSON.
fn workspace_file(root: &std::path::Path, name: &str) -> serde_json::Value {
    serde_json::from_str(
        &std::fs::read_to_string(root.join(project::WORKSPACES).join(name)).unwrap(),
    )
    .unwrap()
}

/// The two ends of every connection an object holds, as `(node, key)` pairs.
fn cables(value: &serde_json::Value) -> Vec<((u64, String), (u64, String))> {
    let end = |v: &serde_json::Value| {
        (
            v["node"].as_u64().unwrap(),
            v["key"].as_str().unwrap().to_string(),
        )
    };
    value["connections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (end(&c["from"]), end(&c["to"])))
        .collect()
}

/// A cable whose two ends are written by different files is glue: it goes in the manifest,
/// and no workspace file refers to a node it does not hold.
#[test]
fn a_cross_workspace_cable_is_in_the_manifest_and_in_neither_workspace_file() {
    let root = dir("cross-cable").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);

    assert_eq!(
        cables(&manifest(&root)),
        [((2, "output".to_string()), (3, "input".to_string()))],
        "the cable from Tunnel's zoom to Set's output is the project's"
    );
    assert_eq!(
        cables(&workspace_file(&root, "Tunnel.ssw")),
        [((1, "output".to_string()), (2, "input".to_string()))],
        "and the one with both ends in Tunnel is Tunnel's"
    );
    assert!(cables(&workspace_file(&root, "Cameras.ssw")).is_empty());
    assert!(cables(&workspace_file(&root, "Set.ssw")).is_empty());
}

/// The workspaces a shared node is on beyond the one that writes it are the other kind of
/// glue, and they are in the manifest for the same reason.
#[test]
fn the_rest_of_a_shared_nodes_set_is_in_the_manifest() {
    let root = dir("shared-glue").join("friday");
    let (graph, [_, second, _], shared) = three_workspaces();
    save(&root, &graph);

    let m = manifest(&root);
    let entries = m["shared"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "only the node on two workspaces: {m}");
    assert_eq!(entries[0]["node"].as_u64().unwrap(), u64::from(shared.0));
    assert_eq!(
        entries[0]["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w.as_u64().unwrap())
            .collect::<Vec<_>>(),
        [u64::from(second.0)],
        "the rest of the set, not the workspace that wrote the node"
    );
    // And nothing else claims it: the file that does not write it does not name it either.
    let cameras = workspace_file(&root, "Cameras.ssw");
    assert!(cameras["nodes"].as_array().unwrap().is_empty(), "{cameras}");
}

/// A workspace file that is gone takes its nodes with it, so a cross-workspace cable into
/// one of them cannot be made. It is reported and the rest of the project opens.
#[test]
fn a_cross_workspace_cable_into_a_missing_file_is_a_warning_and_the_rest_opens() {
    let root = dir("cross-missing").join("friday");
    let (graph, [first, second, _], shared) = three_workspaces();
    save(&root, &graph);
    std::fs::remove_file(root.join(project::WORKSPACES).join("Set.ssw")).unwrap();

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::MissingWorkspace { file, .. } if file == "Set.ssw")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| matches!(
            w,
            LoadWarning::DroppedConnection { reason } if reason.contains("no input 3.input")
        )),
        "the glue cable is reported rather than swallowed: {warnings:?}"
    );
    assert_eq!(back.len(), 2, "the two nodes whose file was there");
    assert_eq!(back.connections().len(), 1, "and the cable between them");
    assert_eq!(
        back.get(shared).unwrap().workspaces,
        BTreeSet::from([first, second]),
        "the other kind of glue is unaffected"
    );
}

/// The glue goes through `Graph::connect` like everything else, so a hand-edited manifest
/// cannot describe a graph the editor could not have produced.
#[test]
fn a_glue_cable_the_graph_refuses_is_reported_and_the_project_opens() {
    let root = dir("cross-refused").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);

    // A color out of Set's output into Tunnel's checkerboard frequency, which is a fragment
    // number.
    let mut m = manifest(&root);
    m["connections"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "from": { "node": 3, "key": "frame" },
            "to": { "node": 1, "key": "frequency" },
        }));
    std::fs::write(
        root.join(project::MANIFEST),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::DroppedConnection { .. })),
        "{warnings:?}"
    );
    assert_eq!(back.len(), 3, "and everything else opened");
    assert_eq!(back.connections().len(), 2);
}

/// A file's cables settle in dependency order over this frame's values, so a feedback loop
/// does not reorder what it feeds: here `max` settles to a number before the `modulo` it
/// feeds, although its id is higher and a measurement closes the ring, and `modulo`'s
/// number into `autoexposure`'s target comes back.
#[test]
fn a_cable_behind_a_feedback_loop_survives_a_reopen() {
    let root = dir("behind-a-loop").join("friday");
    let mut g = Graph::new();
    let modulo = add(&mut g, "modulo", Pos2::ZERO);
    let exposure = add(&mut g, "autoexposure", Pos2::ZERO);
    let max = add(&mut g, "max", Pos2::ZERO);
    for (from, to) in [
        ((exposure, "luma"), (max, "a")),
        ((max, "output"), (modulo, "a")),
        ((modulo, "output"), (exposure, "target")),
    ] {
        g.connect(PortRef::new(from.0, from.1), PortRef::new(to.0, to.1))
            .unwrap();
    }
    let number = |g: &Graph| g.get(modulo).unwrap().output("output").unwrap().ty;
    assert_eq!(number(&g), supersilvia::graph::PortType::UniformNumber);
    save(&root, &g);

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.connections().len(), 3);
    assert_eq!(number(&back), supersilvia::graph::PortType::UniformNumber);
}

/// A node on two workspaces is written by the first of them in project order, and by
/// nothing else: no workspace file refers to a node it does not hold.
#[test]
fn a_shared_node_is_in_exactly_one_file_and_moves_when_the_order_does() {
    let root = dir("shared").join("friday");
    let (mut graph, [_, second, _], shared) = three_workspaces();
    let mut project = save(&root, &graph);

    let holding: Vec<String> = workspace_files(&root)
        .into_iter()
        .filter(|f| {
            std::fs::read_to_string(root.join(project::WORKSPACES).join(f))
                .unwrap()
                .contains("\"id\": 1")
        })
        .collect();
    assert_eq!(holding, ["Tunnel.ssw"], "the first workspace of its set");

    // Reordering the tabs moves it, because nothing stores which file wrote it.
    graph.move_workspace(second, 0);
    project.save(&graph).unwrap();
    let tunnel =
        std::fs::read_to_string(root.join(project::WORKSPACES).join("Tunnel.ssw")).unwrap();
    let cameras =
        std::fs::read_to_string(root.join(project::WORKSPACES).join("Cameras.ssw")).unwrap();
    assert!(!tunnel.contains("\"id\": 1"), "it left: {tunnel}");
    assert!(cameras.contains("\"id\": 1"), "and arrived: {cameras}");

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.get(shared).unwrap().workspaces.len(), 2);
}

/// The counters are the project's, so a node added after a reload cannot take an id another
/// file still names.
#[test]
fn the_counters_survive_a_reload() {
    let root = dir("counters").join("friday");
    let (mut graph, _, _) = three_workspaces();
    // Deleting the highest-numbered node leaves the counter above it.
    let last = graph.iter().map(|(id, _)| id).last().unwrap();
    graph.remove_node(last);
    let doomed = graph.add_workspace("Scratch".to_string(), WorkspaceKind::Video);
    graph.remove_workspace(doomed);
    save(&root, &graph);

    let (_, mut back, _) = Project::open(root).unwrap();
    let fresh = add(&mut back, "zoom", Pos2::ZERO);
    assert!(fresh > last, "a fresh node id, not a reissued one: {fresh}");
    assert!(
        back.add_workspace("Next".to_string(), WorkspaceKind::Video) > doomed,
        "and a fresh workspace id"
    );
}

/// Save deletes the file of a workspace that has gone, and nothing else at all.
#[test]
fn save_removes_a_deleted_workspaces_file_and_leaves_everything_else() {
    let root = dir("delete").join("friday");
    let (mut graph, [_, second, _], _) = three_workspaces();
    let mut project = save(&root, &graph);
    assert_eq!(
        workspace_files(&root),
        ["Cameras.ssw", "Set.ssw", "Tunnel.ssw"]
    );
    std::fs::write(root.join("notes.txt"), "keep me").unwrap();

    graph.remove_workspace(second);
    project.save(&graph).unwrap();

    assert_eq!(workspace_files(&root), ["Set.ssw", "Tunnel.ssw"]);
    assert!(
        root.join("notes.txt").is_file(),
        "Save deletes nothing else"
    );
}

/// A workspace made where a removed one of the same name stood gets a file of its own, since
/// the save that writes it deletes the removed one's.
#[test]
fn a_new_workspace_never_takes_the_file_a_removed_one_leaves() {
    let root = dir("reused-name").join("friday");
    let (mut graph, [first, _, _], _) = three_workspaces();
    let mut project = save(&root, &graph);

    graph.remove_workspace(first);
    let again = graph.add_workspace("Tunnel".to_string(), WorkspaceKind::Video);
    let nodes: BTreeSet<NodeId> = ["checkerboard", "zoom"]
        .into_iter()
        .map(|slug| {
            let id = add(&mut graph, slug, Pos2::ZERO);
            graph.get_mut(id).unwrap().workspaces = BTreeSet::from([again]);
            id
        })
        .collect();
    project.save(&graph).unwrap();

    assert_eq!(
        workspace_files(&root),
        ["Cameras.ssw", "Set.ssw", "Tunnel-2.ssw"]
    );
    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let on_it: BTreeSet<NodeId> = back.on_workspace(again).map(|(id, _)| id).collect();
    assert_eq!(on_it, nodes, "every node on the new Tunnel came back");
}

/// What a reopened project holds, as one string two projects can be compared by: the
/// workspaces in order, every node with where it is and what it is on, and every cable.
fn shape(g: &Graph) -> String {
    let workspaces: Vec<_> = g.workspaces().iter().map(|w| (w.id, &w.name)).collect();
    let nodes: Vec<_> = g
        .iter()
        .map(|(id, n)| (id, n.def.slug, n.pos, &n.workspaces))
        .collect();
    let mut cables: Vec<String> = g.connections().iter().map(|c| format!("{c:?}")).collect();
    cables.sort();
    format!("{workspaces:?}\n{nodes:?}\n{cables:?}")
}

/// A save cut off after any one of its files opens as the last save whole or as this one
/// whole, never a mix: the manifest's rename is the commit, and what is left after it is
/// finished on open.
///
/// The second save moves the shared node between files, removes a workspace and makes a new
/// one under the removed one's name, moves a node and adds one — so a mix of the two saves
/// would load a node twice, lose one, adopt the removed file as a stray or put a node back.
#[test]
fn a_save_cut_off_after_any_file_opens_as_one_save_whole() {
    let (old, [first, second, third], _) = three_workspaces();
    let mut new = old.clone();
    new.move_workspace(second, 0);
    new.remove_workspace(third);
    let set = new.add_workspace("Set".to_string(), WorkspaceKind::Video);
    let dot = add(&mut new, "color", Pos2::ZERO);
    new.get_mut(dot).unwrap().workspaces = BTreeSet::from([set]);
    let moved = new.on_workspace(first).map(|(id, _)| id).last().unwrap();
    new.get_mut(moved).unwrap().pos = Pos2::new(-50.0, -50.0);

    let reopened = |root: &std::path::Path| {
        let (_, graph, warnings) = Project::open(root.to_path_buf()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        shape(&graph)
    };
    let whole = |name: &str, graph: &Graph| {
        let root = dir(name).join("friday");
        save(&root, graph);
        reopened(&root)
    };
    let (whole_old, whole_new) = (whole("cut-old", &old), whole("cut-new", &new));
    assert_ne!(whole_old, whole_new);

    let mut seen = Vec::new();
    for cut in 0.. {
        let root = dir(&format!("cut-{cut}")).join("friday");
        let mut project = save(&root, &old);
        let mut files = 0;
        let result = project.save_with(&new, &mut || {
            files += 1;
            if files > cut {
                Err(std::io::Error::other("cut off"))
            } else {
                Ok(())
            }
        });
        let back = reopened(&root);
        assert!(
            back == whole_old || back == whole_new,
            "cut after {cut} files opens as neither save: {back}"
        );
        seen.push(back == whole_new);
        assert!(
            workspace_files(&root).iter().all(|f| !f.contains(".tmp")),
            "and leaves nothing temporary: {:?}",
            workspace_files(&root)
        );
        if result.is_ok() {
            break;
        }
    }
    // Three workspace files, the manifest, and the three renamed into place: the manifest's
    // rename is the commit, and every cut from there on opens as the new save.
    assert_eq!(seen, [false, false, false, true, true, true, true, true]);
}

/// Someone drops a workspace into the folder from a file manager. Its ids come from another
/// counter, so it arrives with new ones and every cable inside it re-pointed.
#[test]
fn a_stray_file_is_adopted_with_remapped_ids_and_reported() {
    let root = dir("stray").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);

    // A file whose ids collide with every id the project already uses.
    let (other, _, _) = three_workspaces();
    let nodes: BTreeSet<NodeId> = other.iter().map(|(id, _)| id).collect();
    let file = workspace::WorkspaceFile::of(&other, &other.workspaces()[0], &nodes);
    workspace::write(
        &file,
        &root.join(project::WORKSPACES).join("dropped-in.ssw"),
    )
    .unwrap();

    let (project, back, warnings) = Project::open(root.clone()).unwrap();
    assert!(
        warnings.iter().any(
            |w| matches!(w, LoadWarning::AdoptedWorkspace { file } if file == "dropped-in.ssw")
        ),
        "{warnings:?}"
    );
    assert_eq!(back.workspaces().len(), 4, "it came in as a workspace");
    assert_eq!(
        back.len(),
        6,
        "its three nodes joined the three already there"
    );

    let adopted = back.workspaces().last().unwrap().id;
    let inside: Vec<NodeId> = back
        .iter()
        .filter(|(_, n)| n.workspaces.contains(&adopted))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(inside.len(), 3);
    assert_eq!(
        back.connections()
            .iter()
            .filter(|c| inside.contains(&c.from.node) && inside.contains(&c.to.node))
            .count(),
        2,
        "every cable inside the stray file survived the remap"
    );

    // And it is a workspace of the project from then on, under the file it arrived in.
    let mut project = project;
    project.save(&back).unwrap();
    let (_, again, warnings) = Project::open(root).unwrap();
    assert!(
        warnings.is_empty(),
        "adopted once, listed after: {warnings:?}"
    );
    assert_eq!(again.workspaces().len(), 4);
}

/// A listed file that is gone loses its workspace and says so; the rest of the project
/// opens.
#[test]
fn a_missing_workspace_file_is_a_warning_and_the_project_still_opens() {
    let root = dir("missing").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);
    std::fs::remove_file(root.join(project::WORKSPACES).join("Cameras.ssw")).unwrap();

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(
        warnings.iter().any(
            |w| matches!(w, LoadWarning::MissingWorkspace { file, .. } if file == "Cameras.ssw")
        ),
        "{warnings:?}"
    );
    let names: Vec<&str> = back.workspaces().iter().map(|w| w.name.as_str()).collect();
    assert_eq!(names, ["Tunnel", "Set"]);
    assert_eq!(back.len(), 3, "the shared node is still on Tunnel");
}

/// A loose workspace file becomes a project named after it.
#[test]
fn a_loose_workspace_file_opens_as_a_one_workspace_project() {
    let projects = dir("loose");

    let (graph, _, _) = three_workspaces();
    let nodes: BTreeSet<NodeId> = graph.iter().map(|(id, _)| id).collect();
    let today = projects.join("tunnel.ssw");
    workspace::write(
        &workspace::WorkspaceFile::of(&graph, &graph.workspaces()[0], &nodes),
        &today,
    )
    .unwrap();

    let (project, back, report) = project::import(&today, &projects).unwrap();
    assert!(report.warnings.is_empty(), "{report:?}");
    assert_eq!(back.workspaces().len(), 1);
    assert_eq!(
        back.len(),
        3,
        "every node in the file, on the one workspace"
    );
    assert!(Project::is_project(project.root()));
    assert_eq!(project.name(), "tunnel");

    let (_, reopened, _) = Project::open(project.root().to_path_buf()).unwrap();
    assert_eq!(reopened.len(), 3, "and it is a project from then on");
}

// ---------------------------------------------------------------- export and import

/// A file of the given bytes, somewhere outside any project.
fn media(dir: &std::path::Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

/// A project holding one workspace with a `video` node pointing at an imported asset.
fn project_with_media(root: &std::path::Path, dir: &std::path::Path) -> (Project, Graph, NodeId) {
    let mut project = Project::new(root.to_path_buf());
    let mut graph = Graph::new();
    let node = add(&mut graph, "video", Pos2::new(10.0, 10.0));
    let reference = project
        .import_asset(&media(dir, "elsewhere/gumbasia.webm", b"a clip"))
        .unwrap();
    graph
        .get_mut(node)
        .unwrap()
        .options
        .insert("file", reference);
    project.save(&graph).unwrap();
    (project, graph, node)
}

#[test]
fn export_writes_the_file_the_picture_and_the_assets_beside_it() {
    let dir = dir("export");
    let (project, graph, _) = project_with_media(&dir.join("friday"), &dir);
    let first = graph.workspaces()[0].id;
    // A picture, as a save would have left one.
    let thumb = project.thumbnail_path(first).unwrap();
    supersilvia::video::png::write(&thumb, &supersilvia::video::png::Image::new(8, 4)).unwrap();

    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let report = project.export_workspace(&graph, first, &out).unwrap();

    assert_eq!(report.file, out.join("Workspace 1.ssw"));
    assert!(report.file.is_file(), "the workspace file");
    assert!(out.join("Workspace 1.png").is_file(), "and its picture");
    assert_eq!(
        std::fs::read(out.join("assets/gumbasia.webm")).unwrap(),
        b"a clip",
        "and the media it references, beside it"
    );
    assert_eq!(report.assets, ["assets/gumbasia.webm"]);
    assert!(report.missing.is_empty());

    // The exported file means the same thing on its own: its reference resolves against the
    // folder it landed in.
    let file = workspace::read(&report.file).unwrap();
    assert_eq!(
        file.nodes[0].options["file"], "assets/gumbasia.webm",
        "resolved to the copy beside the file"
    );
}

#[test]
fn an_export_into_a_project_lands_in_its_folders_and_is_adopted_on_open() {
    let dir = dir("export-into");
    let (project, graph, _) = project_with_media(&dir.join("friday"), &dir);
    let first = graph.workspaces()[0].id;

    // A second project, saved so it is one.
    let other_root = dir.join("saturday");
    let mut other = Project::new(other_root.clone());
    other.save(&Graph::new()).unwrap();

    let report = project
        .export_workspace(&graph, first, &other_root)
        .unwrap();
    assert_eq!(
        report.file,
        other_root
            .join(project::WORKSPACES)
            .join("Workspace 1-2.ssw"),
        "into the other project's own workspaces/, name-deduplicated against what is there"
    );
    assert!(other_root.join("assets/gumbasia.webm").is_file());

    // Step 2's adoption is what makes exporting *into* a project work while it is closed.
    let (_, opened, warnings) = Project::open(other_root).unwrap();
    assert_eq!(opened.workspaces().len(), 2, "its own, and the stray");
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, LoadWarning::AdoptedWorkspace { .. })),
        "and it says so: {warnings:?}"
    );
    assert!(
        opened.iter().any(|(_, n)| n.def.slug == "video"),
        "with the node the file carried"
    );
}

#[test]
fn the_report_lists_exactly_what_stayed_behind() {
    let dir = dir("report");
    let (graph, [first, second, third], shared) = three_workspaces();
    let project = save(&dir.join("friday"), &graph);

    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let report = project.export_workspace(&graph, first, &out).unwrap();

    // `zoom → out` leaves the workspace, because `out` is on the third.
    assert_eq!(report.dropped_cables.len(), 1, "{report:?}");
    assert!(
        report.dropped_cables[0].ends_with("to 3.input"),
        "{report:?}"
    );
    assert_eq!(
        report.shared,
        [(shared, vec!["Cameras".to_string()])],
        "the other workspaces a shared node is on are the project's, not the file's"
    );
    assert_eq!(report.midi, 0, "nothing binds anything yet");

    // The node itself went, plainly: an export is about making the file work on its own.
    let file = workspace::read(&report.file).unwrap();
    assert_eq!(file.nodes.len(), 2, "the shared node and the zoom");
    assert_eq!(file.connections.len(), 1, "and the cable between them");
    let _ = (second, third);
}

#[test]
fn a_timeline_kind_would_be_refused_and_video_is_not() {
    // Only `Video` exists, so this is the assertion that the refusal is keyed on the kind
    // rather than on nothing: the day a timeline lands it takes the other branch.
    assert!(WorkspaceKind::Video.is_portable());
}

#[test]
fn import_with_colliding_ids_remaps_every_one_and_keeps_every_connection() {
    let dir = dir("import-ids");
    let (from, _, _) = three_workspaces();
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let source = save(&dir.join("friday"), &from);
    let first = from.workspaces()[0].id;
    let report = source.export_workspace(&from, first, &out).unwrap();

    // A project whose ids are exactly the ones in the file.
    let mut graph = Graph::new();
    let a = add(&mut graph, "checkerboard", Pos2::ZERO);
    let b = add(&mut graph, "zoom", Pos2::new(200.0, 0.0));
    graph
        .connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
        .unwrap();
    let before: Vec<NodeId> = graph.iter().map(|(id, _)| id).collect();
    let project = Project::new(dir.join("saturday"));

    let (workspace, into) = project.import_workspace(&report.file, &mut graph).unwrap();

    assert!(into.warnings.is_empty(), "{into:?}");
    assert_eq!(graph.len(), 4, "two of ours and two of theirs");
    let arrived: Vec<NodeId> = graph.on_workspace(workspace).map(|(id, _)| id).collect();
    assert_eq!(arrived.len(), 2);
    for id in &arrived {
        assert!(!before.contains(id), "every id was remapped: {id}");
    }
    // The cable inside the file came with it, re-pointed at the ids it landed under.
    let inside = graph
        .connections()
        .iter()
        .filter(|c| arrived.contains(&c.from.node) && arrived.contains(&c.to.node))
        .count();
    assert_eq!(
        inside, 1,
        "the connection inside the file survived the remap"
    );
    assert_eq!(graph.connections().len(), 2, "and ours is untouched");
}

#[test]
fn import_copies_the_assets_and_rewrites_the_references() {
    let dir = dir("import-assets");
    let (project, graph, _) = project_with_media(&dir.join("friday"), &dir);
    let first = graph.workspaces()[0].id;
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let exported = project.export_workspace(&graph, first, &out).unwrap();

    let other = Project::new(dir.join("saturday"));
    let mut into = Graph::new();
    let (workspace, report) = other.import_workspace(&exported.file, &mut into).unwrap();

    assert_eq!(report.assets, ["assets/gumbasia.webm"]);
    assert!(report.missing.is_empty(), "{report:?}");
    assert_eq!(
        std::fs::read(dir.join("saturday/assets/gumbasia.webm")).unwrap(),
        b"a clip",
        "copied into this project"
    );
    let (_, node) = into.on_workspace(workspace).next().unwrap();
    assert_eq!(
        node.options["file"], "assets/gumbasia.webm",
        "and the reference points at this project's copy"
    );
    assert_eq!(
        other.resolve(&node.options["file"]),
        dir.join("saturday/assets/gumbasia.webm")
    );
}

#[test]
fn an_asset_that_cannot_be_found_is_reported_and_the_rest_imports() {
    let dir = dir("import-missing");
    let (project, graph, _) = project_with_media(&dir.join("friday"), &dir);
    let first = graph.workspaces()[0].id;
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let exported = project.export_workspace(&graph, first, &out).unwrap();
    // Somebody sent the workspace file and forgot the folder beside it.
    std::fs::remove_dir_all(out.join("assets")).unwrap();

    let other = Project::new(dir.join("saturday"));
    let mut into = Graph::new();
    let (workspace, report) = other.import_workspace(&exported.file, &mut into).unwrap();

    assert_eq!(report.missing, ["assets/gumbasia.webm"]);
    assert!(report.assets.is_empty());
    let (_, node) = into.on_workspace(workspace).next().unwrap();
    assert_eq!(
        node.options["file"], "assets/gumbasia.webm",
        "the option is left pointing at what it could not find, and the node shows its error"
    );
}

#[test]
fn export_then_import_round_trips_a_workspace_between_two_projects() {
    let dir = dir("round-trip-workspace");
    let mut project = Project::new(dir.join("friday"));
    let mut graph = Graph::new();
    let first = graph.workspaces()[0].id;
    graph.rename_workspace(first, "Tunnel".to_string());
    graph.set_blurb(first, "the one with the corridor".to_string());
    let source = add(&mut graph, "checkerboard", Pos2::new(10.0, 20.0));
    let out_node = add(&mut graph, "output", Pos2::new(400.0, 20.0));
    graph
        .connect(
            PortRef::new(source, "output"),
            PortRef::new(out_node, "input"),
        )
        .unwrap();
    project.save(&graph).unwrap();

    let elsewhere = dir.join("out");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let exported = project.export_workspace(&graph, first, &elsewhere).unwrap();

    let other = Project::new(dir.join("saturday"));
    let mut into = Graph::new();
    let (workspace, _) = other.import_workspace(&exported.file, &mut into).unwrap();

    assert_eq!(into.workspace(workspace).unwrap().name, "Tunnel");
    assert_eq!(
        into.workspace(workspace).unwrap().blurb,
        "the one with the corridor",
        "the blurb travels with the workspace, because it is the workspace's"
    );
    let slugs: Vec<&str> = into
        .on_workspace(workspace)
        .map(|(_, n)| n.def.slug)
        .collect();
    assert_eq!(slugs, ["checkerboard", "output"]);
    assert_eq!(
        into.connections().len(),
        1,
        "and the cable between them came too"
    );
}

/// Importing is a graph edit — nodes appear — so it is one undo step, and the command log is
/// honest about where those nodes came from.
#[test]
fn importing_a_workspace_is_one_undo_step_and_the_log_says_so() {
    let dir = dir("import-undo");
    let (project, graph, _) = project_with_media(&dir.join("friday"), &dir);
    let first = graph.workspaces()[0].id;
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let exported = project.export_workspace(&graph, first, &out).unwrap();

    let mut app = App::headless();
    let before = app.graph().len();
    app.import_workspace_file(&exported.file);

    assert_eq!(
        app.graph().workspaces().len(),
        2,
        "the imported one is there"
    );
    assert_eq!(app.graph().len(), before + 1, "with the node the file held");
    assert_eq!(
        app.history().len(),
        1,
        "one step, not one per node: {:?}",
        app.history()
    );
    assert!(
        matches!(
            app.history().last(),
            Some(Command::ImportWorkspace { file }) if *file == exported.file
        ),
        "and the log names the file: {:?}",
        app.history()
    );

    assert!(app.undo(), "one undo takes the whole import back");
    assert_eq!(app.graph().workspaces().len(), 1);
    assert_eq!(app.graph().len(), before);
    // The copy it made under `assets/` stays, for the same reason Save deletes nothing
    // there: undoing an edit is not a license to remove somebody's media.
    assert!(
        app.project().resolve("assets/gumbasia.webm").is_file(),
        "the asset it copied in is still in the folder"
    );
}

#[test]
fn a_blurb_rides_in_the_workspace_file() {
    let root = dir("blurb").join("friday");
    let mut graph = Graph::new();
    let first = graph.default_workspace();
    graph.set_blurb(first, "kick and a corridor".to_string());
    save(&root, &graph);

    let (_, back, _) = Project::open(root).unwrap();
    assert_eq!(back.workspace(first).unwrap().blurb, "kick and a corridor");
}

/// A workspace name is a person's text, and a file name is not. Two workspaces with one
/// name get two files.
#[test]
fn file_names_are_sanitized_and_deduplicated() {
    let root = dir("names").join("friday");
    let mut graph = Graph::new();
    graph.rename_workspace(graph.default_workspace(), "../etc/passwd".to_string());
    graph.add_workspace("Tunnel".to_string(), WorkspaceKind::Video);
    graph.add_workspace("Tunnel".to_string(), WorkspaceKind::Video);
    graph.add_workspace(String::new(), WorkspaceKind::Video);
    save(&root, &graph);

    assert_eq!(
        workspace_files(&root),
        [
            "-etc-passwd.ssw",
            "Tunnel-2.ssw",
            "Tunnel.ssw",
            "workspace.ssw"
        ]
    );
    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.workspaces().len(), 4);
}

/// A renamed workspace keeps its file, so the mapping in the project file is what is read
/// rather than the name being turned into a file name twice.
#[test]
fn renaming_a_workspace_keeps_its_file() {
    let root = dir("rename").join("friday");
    let (mut graph, [first, _, _], _) = three_workspaces();
    let mut project = save(&root, &graph);

    graph.rename_workspace(first, "Gumby".to_string());
    project.save(&graph).unwrap();

    assert!(
        workspace_files(&root).contains(&"Tunnel.ssw".to_string()),
        "the file the nodes were in: {:?}",
        workspace_files(&root)
    );
    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.workspaces()[0].name, "Gumby");
}

// ---------------------------------------------------------------------------- the session

/// Which workspaces are open, which tab is showing and where each view was left are the
/// project's, so they ride in `project.ssp` — and none of it is an edit.
#[test]
fn the_session_round_trips_through_the_project_file() {
    let root = dir("session").join("friday");
    let (graph, [first, second, third], _) = three_workspaces();
    let mut project = Project::new(root.clone());
    {
        let session = project.session_mut();
        session.open = [first, third].into_iter().collect();
        session.active = Active::Workspace(third);
        session.views.insert(
            third,
            View {
                pan: [-120.0, 48.0],
                zoom: 1.75,
            },
        );
    }
    project.save(&graph).unwrap();

    let (back, _, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let session = back.session();
    assert_eq!(
        session.open.iter().copied().collect::<Vec<_>>(),
        [first, third],
        "the closed one has no tab and is still in the project"
    );
    assert_eq!(session.active, Active::Workspace(third));
    assert_eq!(session.views[&third].pan, [-120.0, 48.0]);
    assert_eq!(session.views[&third].zoom, 1.75);
    // Never recorded, so it opens at the origin rather than at somebody else's view.
    assert_eq!(session.views[&second], View::default());
}

/// A manifest naming a workspace that is gone leaves the app with a tab all the same.
#[test]
fn a_session_naming_a_workspace_that_is_gone_still_leaves_a_tab() {
    let root = dir("session-stale").join("friday");
    let (mut graph, [first, second, _], _) = three_workspaces();
    let mut project = Project::new(root.clone());
    {
        let session = project.session_mut();
        session.open = [second].into_iter().collect();
        session.active = Active::Workspace(second);
    }
    project.save(&graph).unwrap();

    graph.remove_workspace(second).expect("not the last one");
    project.save(&graph).unwrap();

    let (back, _, _) = Project::open(root).unwrap();
    assert_eq!(
        back.session().open.iter().copied().collect::<Vec<_>>(),
        [first],
        "something is always open"
    );
    assert_eq!(
        back.session().active,
        Active::Project,
        "and the tab that is always there is what shows"
    );
}

/// The layout mode is the workspace's own, so two of them can disagree across a save.
#[test]
fn each_workspace_keeps_its_own_layout_mode() {
    use supersilvia::graph::LayoutMode;

    let root = dir("layout").join("friday");
    let (mut graph, [first, second, _], _) = three_workspaces();
    assert!(graph.set_layout(second, LayoutMode::Linear));
    save(&root, &graph);

    let (_, back, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(back.layout_of(first), LayoutMode::Canvas);
    assert_eq!(back.layout_of(second), LayoutMode::Linear);
}

// ---------------------------------------------------------------------------- through App

#[test]
fn opening_a_project_replaces_everything_and_clears_the_history() {
    let root = dir("open").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);

    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "oscillator",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    assert!(app.can_undo());

    app.open_project(root);
    assert_eq!(app.graph().len(), 3, "the project replaced what was there");
    assert!(
        app.file_status().starts_with("opened friday"),
        "{}",
        app.file_status()
    );
    assert!(
        !app.can_undo(),
        "opening is not an undo step: there is no way back into the old project"
    );
    assert!(app.history().is_empty());
    assert!(!app.dirty(), "a project just opened has nothing unsaved");
}

#[test]
fn saving_and_reopening_through_the_app_keeps_the_graph_and_the_recent_list() {
    let root = dir("through-app").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    assert!(Project::is_project(&root), "{}", app.file_status());

    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    assert!(app.dirty(), "an edit since the save");
    app.save_project().unwrap();
    assert_eq!(app.file_status(), "saved friday");
    assert!(!app.dirty(), "and none after it");
    assert_eq!(app.preferences().recent.first(), Some(&root));

    let mut other = App::headless();
    other.open_project(root);
    assert_eq!(other.graph().len(), 1);
}

/// Undo is what makes the dirty marker honest: stepping back to what was written is not a
/// change to it.
#[test]
fn undoing_back_to_what_was_saved_is_not_dirty() {
    let root = dir("dirty").join("friday");
    let mut app = App::headless();
    app.new_project(root);
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.save_project().unwrap();
    assert!(!app.dirty());

    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    assert!(app.dirty());
    app.undo();
    assert!(!app.dirty(), "back at the state that was written");
    app.redo();
    assert!(app.dirty(), "and forward off it again");
}

/// And so is abandoning a scrub. `Escape` mid-drag drops the step the drag opened rather
/// than setting the value back, precisely so the serial goes back with it: a no-op step
/// would leave the title claiming unsaved work over a graph identical to the file.
#[test]
fn an_abandoned_scrub_is_not_unsaved_work() {
    let root = dir("abandoned").join("friday");
    let mut app = App::headless();
    app.new_project(root);
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.save_project().unwrap();
    assert!(!app.dirty());
    let id = app.graph().iter().next().expect("one node").0;
    let steps = app.undo_len();

    // A scrub: one command a frame, all of them one gesture and one step.
    for value in [9.0, 11.0, 14.0] {
        app.apply(Command::SetControl {
            node: id,
            key: "frequency",
            value: supersilvia::graph::ControlValue::Float(value),
        })
        .unwrap();
    }
    assert!(app.dirty());
    assert_eq!(app.undo_len(), steps + 1, "a scrub is one step");

    assert!(app.cancel_control_drag(id, "frequency", 8.0));
    assert_eq!(app.undo_len(), steps, "the step collapsed");
    assert!(!app.dirty(), "the graph is the one on disk again");
}

#[test]
fn save_as_copies_the_folder_and_switches_to_it() {
    let base = dir("save-as");
    let root = base.join("friday");
    let elsewhere = base.join("saturday");

    let mut app = App::headless();
    app.new_project(root.clone());
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.save_project_as(elsewhere.clone());

    assert_eq!(app.project().root(), elsewhere, "{}", app.file_status());
    assert!(!app.dirty(), "and what was on screen is in it");
    assert!(Project::is_project(&elsewhere));

    let (_, copy, _) = Project::open(elsewhere).unwrap();
    assert_eq!(copy.len(), 1);
    let (_, original, _) = Project::open(root).unwrap();
    assert!(original.is_empty(), "last week's set is untouched");
}

/// An app whose projects folder is `projects`, and whose preferences touch no disk.
fn app_keeping_projects_in(projects: &std::path::Path) -> App {
    let mut app = App::headless();
    app.use_preferences(supersilvia::preferences::Store::of(
        supersilvia::preferences::Preferences {
            projects_dir: Some(projects.to_path_buf()),
            ..Default::default()
        },
    ));
    app
}

/// **The first launch makes `Untitled` in the projects folder and says where, in full**,
/// since nobody chose the place. The second opens the same one.
#[test]
fn the_first_launch_makes_untitled_in_the_projects_folder_and_says_where() {
    let projects = dir("first-launch").join("Documents").join("supersilvia");
    let mut app = app_keeping_projects_in(&projects);
    app.open_last_or_untitled();

    let untitled = projects.join("Untitled");
    assert_eq!(app.project().root(), untitled);
    assert!(Project::is_project(&untitled));
    assert_eq!(
        app.file_status(),
        format!("new project at {}", untitled.display())
    );

    let mut again = app_keeping_projects_in(&projects);
    again.open_last_or_untitled();
    assert_eq!(again.project().root(), untitled, "{}", again.file_status());
}

/// A folder nobody may read or enter, for as long as this lives, and readable again after so
/// the test can clean up. `None` where the permissions do not bind, as for root.
struct Locked(PathBuf);

impl Locked {
    fn new(name: &str) -> Option<Self> {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir(name).join("locked");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let locked = Self(path);
        if std::fs::read_dir(&locked.0).is_ok() {
            eprintln!("permissions do not bind here; skipping");
            return None;
        }
        Some(locked)
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// **A projects folder that cannot be read or made says so**, with its whole path and the
/// system's reason, and nothing falls back to another folder. One that is not there yet is
/// no problem until it has to be made.
#[test]
fn a_projects_folder_that_cannot_be_read_or_made_says_so() {
    let Some(locked) = Locked::new("unreadable-projects") else {
        return;
    };
    let read = project::projects_dir_problem(&locked.0, false).expect("a problem");
    assert!(
        read.starts_with(&format!(
            "could not read the projects folder {}: ",
            locked.0.display()
        )) && read.contains("ermission denied"),
        "{read}"
    );
    let inside = locked.0.join("supersilvia");
    let made = project::projects_dir_problem(&inside, true).expect("a problem");
    assert!(
        made.starts_with(&format!(
            "could not make the projects folder {}: ",
            inside.display()
        )),
        "{made}"
    );

    let missing = dir("projects-not-there-yet").join("supersilvia");
    assert_eq!(project::projects_dir_problem(&missing, false), None);
    assert!(!missing.exists(), "looking makes nothing");
    assert_eq!(project::projects_dir_problem(&missing, true), None);
    assert!(missing.is_dir(), "and making makes it");
}

/// **A launch whose projects folder cannot be made still comes up**, on the empty document it
/// started with, unsaved, and the status line says what failed rather than making `Untitled`
/// somewhere nobody chose.
#[test]
fn a_launch_whose_projects_folder_cannot_be_made_comes_up_and_says_so() {
    let Some(locked) = Locked::new("launch-unreadable") else {
        return;
    };
    for (projects, verb) in [
        (locked.0.join("supersilvia"), "make"),
        (locked.0.clone(), "read"),
    ] {
        let mut app = app_keeping_projects_in(&projects);
        let started = app.project().root().to_path_buf();
        app.open_last_or_untitled();
        assert!(
            app.file_status()
                .starts_with(&format!("could not {verb} the projects folder")),
            "{}",
            app.file_status()
        );
        assert_eq!(app.project().root(), started, "no Untitled anywhere else");
        app.apply(Command::AddNode {
            slug: "checkerboard",
            at: Pos2::ZERO,
            workspace: app.graph().default_workspace(),
        })
        .expect("and the editor is usable");
    }
}

/// **New project's name rules**: a name, holding nothing a folder's name cannot, and not
/// already in the folder — each refused with the reason the window shows. Spaces at either end
/// are not part of it.
#[test]
fn a_new_projects_name_is_refused_with_its_reason() {
    let projects = dir("names");
    std::fs::create_dir_all(projects.join("Friday")).unwrap();
    let refused = |name: &str| project::new_project_path(&projects, name).unwrap_err();

    assert_eq!(refused(""), "Give the project a name.");
    assert_eq!(refused("   "), "Give the project a name.");
    assert_eq!(refused("Friday/Saturday"), "A name cannot hold /");
    assert_eq!(refused(".."), "A name cannot be ..");
    assert_eq!(
        refused("Friday"),
        "There is already a folder called Friday here."
    );
    assert_eq!(
        refused(" Friday "),
        "There is already a folder called Friday here."
    );
    if supersilvia::platform::dirs::NOT_IN_NAMES.contains(&':') {
        assert_eq!(refused("10:30"), "A name cannot hold :");
    } else {
        assert_eq!(
            project::new_project_path(&projects, "10:30"),
            Ok(projects.join("10:30"))
        );
    }
    assert_eq!(
        project::new_project_path(&projects, " Saturday night "),
        Ok(projects.join("Saturday night"))
    );
}

/// New project offers `Untitled`, then the first `Untitled N` that is free.
#[test]
fn new_project_offers_the_next_free_untitled() {
    let projects = dir("untitled-names");
    assert_eq!(project::next_untitled(&projects), "Untitled");
    std::fs::create_dir_all(projects.join("Untitled")).unwrap();
    assert_eq!(project::next_untitled(&projects), "Untitled 2");
    std::fs::create_dir_all(projects.join("Untitled 3")).unwrap();
    assert_eq!(project::next_untitled(&projects), "Untitled 2");
    std::fs::create_dir_all(projects.join("Untitled 2")).unwrap();
    assert_eq!(project::next_untitled(&projects), "Untitled 4");
}

/// **Save as copies the person's work and not the cache**: `renders/` and `snaps/` go with the
/// copy, `cache/` and `.autosave/` stay behind, and the original keeps all of it.
#[test]
fn save_as_copies_renders_and_snaps_and_leaves_the_cache() {
    let base = dir("save-as-cache");
    let root = base.join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    for (folder, file) in [
        ("renders", "output3-001.mp4"),
        ("snaps", "output3-20261001-120000.png"),
        (project::CACHE, "clip.mp4"),
    ] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
        std::fs::write(root.join(folder).join(file), folder).unwrap();
    }

    let elsewhere = base.join("saturday");
    app.save_project_as(elsewhere.clone());
    assert_eq!(app.project().root(), elsewhere, "{}", app.file_status());
    assert!(elsewhere.join("renders/output3-001.mp4").is_file());
    assert!(
        elsewhere
            .join("snaps/output3-20261001-120000.png")
            .is_file()
    );
    assert!(
        !elsewhere.join(project::CACHE).exists(),
        "the cache stays behind"
    );
    assert!(
        root.join(project::CACHE).join("clip.mp4").is_file(),
        "and the original keeps it"
    );
}

#[test]
fn a_new_project_is_refused_over_a_folder_with_anything_in_it() {
    let root = dir("not-empty");
    std::fs::write(root.join("something.txt"), "hello").unwrap();

    let mut app = App::headless();
    app.new_project(root.clone());
    assert!(
        app.file_status().contains("not empty"),
        "{}",
        app.file_status()
    );
    assert!(!Project::is_project(&root));
}

#[test]
fn opening_something_that_is_not_a_project_leaves_the_editor_alone() {
    let root = dir("junk");
    std::fs::write(root.join(project::MANIFEST), "{ not a project").unwrap();

    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();

    app.open_project(root.clone());
    assert_eq!(app.graph().len(), 1, "what was on screen survives");
    assert!(
        app.file_status().starts_with("open failed"),
        "{}",
        app.file_status()
    );

    let err = Project::open(root).unwrap_err();
    assert!(matches!(err, LoadError::Json(_)), "{err:?}");
}

/// A project that opened with something dropped is open, and says what it lost.
#[test]
fn a_project_with_a_warning_still_opens() {
    let root = dir("warned").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);
    std::fs::remove_file(root.join(project::WORKSPACES).join("Set.ssw")).unwrap();

    let mut app = App::headless();
    app.open_project(root);
    assert_eq!(app.graph().workspaces().len(), 2);
    assert!(
        app.file_status()
            .starts_with("opened friday with 2 warning(s) — "),
        "the missing file, and the cable that went across to it: {}",
        app.file_status()
    );
}

/// A file inside a project opens the project it is inside, from the command line.
#[test]
fn a_path_inside_a_project_opens_that_project() {
    let root = dir("argument").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);

    let mut app = App::headless();
    app.open_argument(&root.join(project::WORKSPACES).join("Tunnel.ssw"));
    assert_eq!(app.project().root(), root, "{}", app.file_status());
    assert_eq!(app.graph().len(), 3);
}

/// **The mixer is not written to the project file, and the Main Input is**, as silvia's
/// are. Which Output is on air and where the fade is are a hand on the instrument tonight,
/// so a project opens with nothing on air; what the Main Input plays and how it is tuned
/// comes back, except a device, which is switched on by a hand and not by Open.
#[test]
fn the_mixer_is_not_in_the_project_file_and_the_main_input_is() {
    use supersilvia::maininput::{AudioSource, VideoSource};
    use supersilvia::mixer::{Channel, Method, Resolution};

    let root = dir("mixer").join("friday");
    let (graph, _, _) = three_workspaces();
    let out = graph
        .iter()
        .find(|(_, n)| n.def.slug == "output")
        .unwrap()
        .0;

    let mut project = Project::new(root.clone());
    project.mixer_mut().claim(Channel::A, out);
    project.mixer_mut().set_balance(0.5);
    project.mixer_mut().method = Method::Checkerboard;
    project.mixer_mut().resolution = Resolution::Fixed(1920, 1080);
    project.mixer_mut().background = true;
    let clip = VideoSource::File {
        asset: "assets/gumbasia.webm".to_string(),
    };
    project.main_input_mut().video = clip.clone();
    project.main_input_mut().audio = AudioSource::Video;
    project.main_input_mut().gain = 2.5;
    project.main_input_mut().levels[1] = 0.25;
    project.save(&graph).unwrap();

    let file = manifest(&root);
    assert!(
        file["mixer"].is_null(),
        "the mixer is not written: {file:?}"
    );
    assert_eq!(
        file["main_input"]["gain"], 2.5,
        "the Main Input is: {file:?}"
    );

    let (mut back, _, warnings) = Project::open(root.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mixer = back.mixer();
    assert_eq!(mixer.a, None, "no deck is claimed on opening");
    assert_eq!(mixer.balance, -1.0);
    assert_eq!(mixer.method, Method::Blend);
    assert_eq!(mixer.resolution, Resolution::Viewport);
    assert!(!mixer.background);
    let input = back.main_input();
    assert_eq!(input.video, clip, "the clip comes back");
    assert_eq!(input.audio, AudioSource::Video);
    assert_eq!(input.gain, 2.5);
    assert_eq!(input.levels[1], 0.25);

    // A screen and a microphone are written and not opened again.
    back.main_input_mut().video = VideoSource::Screen;
    back.main_input_mut().audio = AudioSource::system();
    back.save(&graph).unwrap();
    let (again, _, _) = Project::open(root).unwrap();
    assert!(
        again.main_input().is_idle(),
        "no device is opened on opening"
    );
    assert_eq!(again.main_input().gain, 2.5, "and the tuning stays");
}

/// A project file from before the Main Input rode in it opens with the Main Input at its
/// default.
#[test]
fn a_manifest_without_a_main_input_opens_with_the_default_one() {
    let root = dir("no-main-input").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);
    let path = root.join(project::MANIFEST);
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(file.as_object_mut().unwrap().remove("main_input").is_some());
    std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

    let (back, _, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        *back.main_input(),
        supersilvia::maininput::MainInput::default()
    );
}

/// Save as carries the Main Input, and a clip it names resolves inside the copy.
#[test]
fn save_as_carries_the_main_input_and_its_clip_resolves_in_the_copy() {
    use supersilvia::maininput::VideoSource;
    let base = dir("fork-main-input");
    let root = base.join("friday");
    let (graph, _, _) = three_workspaces();
    let mut project = Project::new(root.clone());
    project.save(&graph).unwrap();
    let source = base.join("clip.webm");
    std::fs::write(&source, b"not really a clip").unwrap();
    let asset = project.import_asset(&source).unwrap();
    project.main_input_mut().video = VideoSource::File {
        asset: asset.clone(),
    };
    project.main_input_mut().gain = 0.5;
    project.save(&graph).unwrap();

    let copy = base.join("saturday");
    let mut fork = project.fork_to(copy.clone()).unwrap();
    assert_eq!(
        fork.main_input(),
        project.main_input(),
        "the fork carries it"
    );
    fork.save(&graph).unwrap();
    let (back, _, _) = Project::open(copy.clone()).unwrap();
    assert_eq!(back.main_input().gain, 0.5);
    let VideoSource::File { asset: named } = &back.main_input().video else {
        panic!("the clip did not come back: {:?}", back.main_input().video);
    };
    let path = back.resolve(named);
    assert!(
        path.starts_with(&copy),
        "{} is outside the copy",
        path.display()
    );
    assert!(path.is_file(), "the clip was copied with the folder");
}

/// A project file from before the mixer opens with the mixer at its defaults.
#[test]
fn a_manifest_without_a_mixer_opens_with_the_default_one() {
    let root = dir("no-mixer").join("friday");
    let (graph, _, _) = three_workspaces();
    save(&root, &graph);
    let path = root.join(project::MANIFEST);
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file.as_object_mut().unwrap().remove("mixer");
    std::fs::write(&path, serde_json::to_string_pretty(&file).unwrap()).unwrap();

    let (back, _, _) = Project::open(root).unwrap();
    assert_eq!(*back.mixer(), supersilvia::mixer::Mixer::default());
}

/// A recorded curve is one of the node's own values, so it goes into the `.ssw` and comes
/// back out of it — and the transport that made it does not, because a transport is runtime
/// state. Reopening gets the performance back, stopped at the start.
#[test]
fn a_recording_survives_a_save_and_reopen_with_the_transport_stopped() {
    let root = dir("recording").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    app.apply(Command::AddNode {
        slug: "automation",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let node = app.graph().iter().map(|(id, _)| id).max().unwrap();
    let frame = 1.0 / 60.0;
    app.apply(Command::SetControl {
        node,
        key: "duration",
        value: supersilvia::graph::ControlValue::Float(0.25),
    })
    .unwrap();

    // Perform a ramp into it, which is one recording and so one edit.
    app.press(PortRef::new(node, "record"), true);
    app.tick(frame);
    app.press(PortRef::new(node, "record"), false);
    for i in 0..=20u8 {
        app.apply(Command::SetControl {
            node,
            key: "input",
            value: supersilvia::graph::ControlValue::Float(i as f32 / 20.0),
        })
        .unwrap();
        app.tick(frame);
    }
    let recorded = app
        .graph()
        .get(node)
        .and_then(|n| n.values.get("recording"))
        .and_then(supersilvia::graph::Value::points)
        .expect("recorded")
        .to_vec();
    assert!(recorded.len() > 5, "{} points", recorded.len());
    app.save_project().unwrap();
    // Played, so the transport is running when the same app opens the project again.
    app.press(PortRef::new(node, "play"), true);
    app.tick(frame);
    app.press(PortRef::new(node, "play"), false);
    for _ in 0..5 {
        app.tick(frame);
    }

    let (_, back, warnings) = Project::open(root.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let reopened = back
        .get(node)
        .and_then(|n| n.values.get("recording"))
        .and_then(supersilvia::graph::Value::points)
        .expect("the curve came back");
    assert_eq!(reopened, recorded.as_slice(), "point for point");

    // And the transport is stopped: an app that opens the project publishes its knob rather
    // than the curve, which never reaches 0.9 — a fresh one, and the one that was playing it.
    let mut fresh = App::headless();
    for opened in [&mut fresh, &mut app] {
        opened.open_project(root.clone());
        opened
            .apply(Command::SetControl {
                node,
                key: "input",
                value: supersilvia::graph::ControlValue::Float(0.9),
            })
            .unwrap();
        opened.tick(frame);
        assert_eq!(
            opened.uniform(PortRef::new(node, "output")),
            Some(0.9),
            "stopped at the start, so the knob is what is published"
        );
    }
}

/// Two files that both claim one id: the node read first keeps it, and the other takes a fresh
/// one with a warning rather than landing on top of it. The cables inside each file follow
/// their own node.
#[test]
fn an_id_two_files_claim_stays_with_the_first_and_the_second_node_takes_a_fresh_one() {
    let root = dir("duplicate-id");
    let mut g = Graph::new();
    let second = g.add_workspace("Two".to_string(), WorkspaceKind::Video);
    let cb = add(&mut g, "checkerboard", Pos2::ZERO);
    let out = add(&mut g, "output", Pos2::new(240.0, 0.0));
    g.connect(PortRef::new(cb, "output"), PortRef::new(out, "input"))
        .unwrap();
    let number = add(&mut g, "number", Pos2::ZERO);
    let zoom = add(&mut g, "zoom", Pos2::new(240.0, 0.0));
    for id in [number, zoom] {
        g.get_mut(id).unwrap().workspaces = BTreeSet::from([second]);
    }
    g.connect(PortRef::new(number, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();
    save(&root, &g);

    // The second workspace's file renumbers its number node onto the Output's id.
    let name = manifest(&root)["workspaces"][1]["file"]
        .as_str()
        .unwrap()
        .to_string();
    let mut file = workspace_file(&root, &name);
    let claimed = u64::from(out.0);
    for node in file["nodes"].as_array_mut().unwrap() {
        if node["id"] == u64::from(number.0) {
            node["id"] = claimed.into();
        }
    }
    for cable in file["connections"].as_array_mut().unwrap() {
        if cable["from"]["node"] == u64::from(number.0) {
            cable["from"]["node"] = claimed.into();
        }
    }
    std::fs::write(
        root.join(project::WORKSPACES).join(&name),
        serde_json::to_string(&file).unwrap(),
    )
    .unwrap();

    let (_, back, warnings) = Project::open(root).unwrap();
    let fresh = match warnings.as_slice() {
        [LoadWarning::DuplicateId { id, fresh, .. }] if *id == out => *fresh,
        other => panic!("one warning, naming the id: {other:?}"),
    };
    assert_eq!(back.get(out).map(|n| n.def.slug), Some("output"));
    assert_eq!(back.get(fresh).map(|n| n.def.slug), Some("number"));
    assert!(fresh.0 > g.next_node_id(), "above every id the project had");
    assert_eq!(
        back.source_of(PortRef::new(out, "input")),
        Some(PortRef::new(cb, "output")),
        "the first file's cable is into its own node"
    );
    assert_eq!(
        back.source_of(PortRef::new(zoom, "zoom")),
        Some(PortRef::new(fresh, "output")),
        "and the second's followed the renumbered one"
    );
}

/// A file sending two Outputs under one name opens with the later one taking a ` copy`, and
/// opens clean: the rename is the graph's, and the file changes only if the project is saved.
#[test]
fn two_outputs_a_file_sends_under_one_name_open_apart() {
    use supersilvia::nodes::output::{SEND_NAME, send_name};
    let root = dir("send-names").join("friday");
    let mut g = Graph::new();
    let first = add(&mut g, "output", Pos2::ZERO);
    let second = add(&mut g, "output", Pos2::new(300.0, 0.0));
    for id in [first, second] {
        g.get_mut(id)
            .unwrap()
            .options
            .insert(SEND_NAME, "warpzone".to_string());
    }
    save(&root, &g);

    let (_, back, warnings) = Project::open(root.clone()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(send_name(first, back.get(first).unwrap()), "warpzone");
    assert_eq!(
        send_name(second, back.get(second).unwrap()),
        "warpzone copy"
    );

    let mut app = App::headless();
    app.open_project(root);
    assert!(!app.dirty(), "settling the names is not an edit");
    assert_eq!(
        send_name(second, app.graph().get(second).unwrap()),
        "warpzone copy"
    );
}

/// An XY Pad's hand controls, knobs and edges are its values and go into the `.ssw`; its
/// wells and where the flight has got to are runtime and do not. Reopening puts the puck back
/// where the hand last put it, with nothing pulling on it.
#[test]
fn an_xy_pad_reopens_with_the_puck_where_the_hand_left_it_and_no_wells() {
    use supersilvia::graph::ControlValue;
    use supersilvia::nodes::cpu::Touch;

    let root = dir("xypad").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    app.apply(Command::AddNode {
        slug: "xypad",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let pad = app.graph().iter().map(|(id, _)| id).max().unwrap();
    app.apply(Command::SetControls {
        node: pad,
        values: vec![
            ("padX", ControlValue::Float(-0.25)),
            ("padY", ControlValue::Float(0.75)),
            ("vx", ControlValue::Float(0.0)),
            ("vy", ControlValue::Float(0.0)),
        ],
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: pad,
        key: "maxY",
        value: ControlValue::Float(3.0),
    })
    .unwrap();
    app.apply(Command::SetOption {
        node: pad,
        key: "edgeX",
        value: "wrap".to_string(),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    app.touch(
        pad,
        Touch::Well {
            at: [0.0, 0.0],
            reach: None,
        },
    );
    app.tick(1.0 / 60.0);
    assert_eq!(app.puck(pad).map(|p| p.wells.len()), Some(1));
    app.save_project().unwrap();

    let mut fresh = App::headless();
    fresh.open_project(root);
    let node = fresh.graph().get(pad).expect("the pad came back").clone();
    assert_eq!(node.controls.get("padX"), Some(&ControlValue::Float(-0.25)));
    assert_eq!(node.controls.get("padY"), Some(&ControlValue::Float(0.75)));
    assert_eq!(node.controls.get("maxY"), Some(&ControlValue::Float(3.0)));
    assert_eq!(node.options.get("edgeX").map(String::as_str), Some("wrap"));
    fresh.tick(1.0 / 60.0);
    let puck = fresh.puck(pad).expect("the pad reports its puck");
    assert_eq!(puck.at, [-0.25, 0.75], "where the hand left it");
    assert!(puck.wells.is_empty(), "a well is runtime and is not saved");
    assert_eq!(
        fresh.uniform(PortRef::new(pad, "y")),
        Some(2.5),
        "three quarters of the way up -1 to 3"
    );
}
