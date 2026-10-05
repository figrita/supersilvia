// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: `drawingcanvas`'s painting, which is one of the node's own values and is saved
//! with the project as a picture file.
//!
//! What the tick publishes, what Clear writes back and when Stroke Done fires; a painted
//! stroke as one undo step and a copied node carrying its picture; and the picture's trip
//! through the project folder — saved as a PNG in `assets/` named by its content, opened back
//! pixel for pixel, replaced and tidied at the next save, carried by the autosave, and taken
//! along by an export and an import. The paint surface's own gestures are layer 2's, in
//! `tests/ui.rs`.

use emath::{Pos2, Vec2};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use supersilvia::graph::{ControlValue, Graph, NodeId, Painting, PortRef, Value};
use supersilvia::nodes::drawingcanvas::{self, Brush, Sheet, Symmetry, Tool};
use supersilvia::project::{ASSETS, AUTOSAVE, Project};
use supersilvia::{App, Command};

fn add(app: &mut App) -> NodeId {
    app.apply(Command::AddNode {
        slug: "drawingcanvas",
        at: Pos2::new(40.0, 40.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// A folder of this test's own, empty.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-painting-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A picture with something on it: a white stroke, a red fill beside it, and an erased hole
/// whose color stays under an alpha of zero — every kind of pixel a PNG has to carry back.
fn picture(width: u32, height: u32) -> Painting {
    let mut sheet = Sheet::blank(width, height, [0, 0, 0, 255]);
    let pen = Brush {
        size: 4.0,
        color: [255, 255, 255, 255],
        erase: false,
    };
    let (w, h) = (width as f32, height as f32);
    sheet.segment(&pen, Symmetry::None, (w * 0.5, 2.0), (w * 0.5, h - 2.0));
    sheet.fill([200, 30, 30, 255], (2.0, 2.0));
    let eraser = Brush { erase: true, ..pen };
    sheet.segment(&eraser, Symmetry::None, (w - 6.0, 6.0), (w - 6.0, 6.0));
    sheet.into_painting(0)
}

/// Write a picture onto the node, as a stroke's frame does: into whatever gesture is open.
fn paint(app: &mut App, node: NodeId, painting: &Painting) {
    app.apply(Command::SetValue {
        node,
        key: drawingcanvas::PAINTING,
        value: Value::Painting(painting.clone()),
    })
    .unwrap();
}

/// Write a picture and let go, as a whole stroke does: one step of its own.
fn stroke(app: &mut App, node: NodeId, painting: &Painting) {
    paint(app, node, painting);
    app.end_gesture();
}

fn held(graph: &Graph, node: NodeId) -> Option<Painting> {
    graph
        .get(node)?
        .values
        .get(drawingcanvas::PAINTING)?
        .painting()
        .cloned()
}

fn pixels(painting: &Painting) -> (u32, u32, Vec<u8>) {
    let (w, h, px) = painting.pixels().expect("read");
    (w, h, px.to_vec())
}

/// The painting files under a folder of assets.
fn painting_files(assets: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(assets)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("painting-"))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

// ------------------------------------------------------------------------------------ the tick

/// A canvas nobody has painted on publishes a blank of its background at the size Canvas
/// Size names, premultiplied; a painting is published premultiplied too, a copy made once
/// and published again while nothing changes, and the painting keeps its straight pixels.
#[test]
fn the_tick_publishes_the_painting_premultiplied() {
    let mut app = App::headless();
    let id = add(&mut app);
    app.tick(1.0 / 60.0);
    let port = PortRef::new(id, drawingcanvas::OUTPUT);
    let blank = app
        .frame(port)
        .expect("a picture before any stroke")
        .clone();
    assert_eq!((blank.width, blank.height), (512, 512));
    assert!(
        blank
            .bytes()
            .unwrap()
            .chunks(4)
            .all(|p| p == [0, 0, 0, 255]),
        "silvia's black background"
    );
    app.apply(Command::SetControl {
        node: id,
        key: drawingcanvas::BACKGROUND,
        value: ControlValue::Color([1.0, 0.0, 0.0, 0.5]),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    assert!(
        app.frame(port)
            .unwrap()
            .bytes()
            .unwrap()
            .chunks(4)
            .all(|p| p == [128, 0, 0, 128]),
        "a half-transparent red background, premultiplied"
    );

    // The picture, with a half-transparent texel and a red one under no alpha in a corner.
    let (_, _, mut straight) = pixels(&picture(512, 512));
    straight[..8].copy_from_slice(&[200, 100, 51, 128, 200, 30, 30, 0]);
    let painting = Painting::new(512, 512, straight.clone(), 0);
    paint(&mut app, id, &painting);
    app.tick(1.0 / 60.0);
    let shown = Arc::clone(app.frame(port).unwrap());
    let shown_bytes = shown.bytes().unwrap();
    assert_eq!(shown_bytes[..8], [100, 50, 26, 128, 0, 0, 0, 0]);
    for (s, p) in straight.chunks(4).zip(shown_bytes.chunks(4)) {
        let a = u32::from(s[3]);
        let want: Vec<u8> = s[..3]
            .iter()
            .map(|&c| ((f64::from(c) * f64::from(a) / 255.0).round()) as u8)
            .chain([s[3]])
            .collect();
        assert_eq!(p, want.as_slice(), "{s:?} premultiplied");
    }
    assert_eq!(
        pixels(&held(app.graph(), id).unwrap()).2,
        straight,
        "the painting itself is straight"
    );
    app.tick(1.0 / 60.0);
    assert!(
        Arc::ptr_eq(app.frame(port).unwrap(), &shown),
        "the same copy while nothing changed"
    );
}

/// Canvas Size moved under a painting stretches it to the new size rather than wiping it.
#[test]
fn a_new_canvas_size_stretches_the_painting() {
    let mut app = App::headless();
    let id = add(&mut app);
    paint(&mut app, id, &picture(256, 256));
    app.apply(Command::SetOption {
        node: id,
        key: drawingcanvas::CANVAS_SIZE,
        value: "1024x512".to_string(),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    let shown = app
        .frame(PortRef::new(id, drawingcanvas::OUTPUT))
        .unwrap()
        .clone();
    assert_eq!((shown.width, shown.height), (1024, 512));
    let bytes = shown.bytes().unwrap();
    let at = |x: usize, y: usize| &bytes[(y * 1024 + x) * 4..(y * 1024 + x) * 4 + 4];
    assert_eq!(at(512, 256), [255, 255, 255, 255], "the stroke, stretched");
    assert_eq!(at(10, 10), [200, 30, 30, 255], "the fill, stretched");
}

/// Clear, from the button or a cable, writes the background back into the document at the
/// canvas's size — one undo step — and a Clear of a canvas already blank writes nothing.
#[test]
fn clear_writes_the_background_and_is_one_undo_step() {
    let mut app = App::headless();
    let id = add(&mut app);
    let clear = PortRef::new(id, drawingcanvas::CLEAR);
    // Nothing painted: a Clear has nothing to say.
    let steps = app.undo_len();
    app.press(clear, true);
    app.tick(1.0 / 60.0);
    app.press(clear, false);
    app.tick(1.0 / 60.0);
    assert!(held(app.graph(), id).is_none());
    assert_eq!(app.undo_len(), steps, "a Clear of nothing is no edit");

    let painting = picture(512, 512);
    paint(&mut app, id, &painting);
    app.end_gesture();
    let steps = app.undo_len();
    app.press(clear, true);
    app.tick(1.0 / 60.0);
    app.press(clear, false);
    app.tick(1.0 / 60.0);
    let cleared = held(app.graph(), id).expect("a painting");
    let (w, h, px) = pixels(&cleared);
    assert_eq!((w, h), (512, 512));
    assert!(px.chunks(4).all(|p| p == [0, 0, 0, 255]), "the background");
    assert_eq!(app.undo_len(), steps + 1, "one step");

    // Again: already blank, so nothing moves.
    app.press(clear, true);
    app.tick(1.0 / 60.0);
    app.press(clear, false);
    app.tick(1.0 / 60.0);
    assert_eq!(app.undo_len(), steps + 1, "a second Clear is no edit");

    assert!(app.undo());
    assert_eq!(
        held(app.graph(), id).map(|p| pixels(&p)),
        Some(pixels(&painting)),
        "undo brings the painting back"
    );
}

/// Stroke Done fires on the tick a stroke finishes — the picture carries a stroke number
/// newer than any before it — for one frame, and not when an undo goes back to an older one.
#[test]
fn stroke_done_fires_once_per_stroke_and_not_on_undo() {
    let mut app = App::headless();
    let id = add(&mut app);
    let done = PortRef::new(id, drawingcanvas::STROKE_DONE);
    app.tick(1.0 / 60.0);
    assert!(app.edges(done).is_empty());

    let first = picture(512, 512).with_stroke(drawingcanvas::next_stroke());
    paint(&mut app, id, &first);
    app.end_gesture();
    app.tick(1.0 / 60.0);
    assert!(
        app.edges(done).iter().any(|e| e.is_down()),
        "a finished stroke fires"
    );
    app.tick(1.0 / 60.0);
    assert!(
        !app.edges(done).iter().any(|e| e.is_down()),
        "for one frame, and it comes up: {:?}",
        app.edges(done)
    );

    let second = first.with_stroke(drawingcanvas::next_stroke());
    paint(&mut app, id, &second);
    app.end_gesture();
    app.tick(1.0 / 60.0);
    assert!(app.edges(done).iter().any(|e| e.is_down()), "and the next");
    app.tick(1.0 / 60.0);

    assert!(app.undo());
    app.tick(1.0 / 60.0);
    assert!(
        !app.edges(done).iter().any(|e| e.is_down()),
        "an undo finishes no stroke"
    );
}

// --------------------------------------------------------------------- the document around it

/// A pen stroke is a `SetValue` a frame while it moves, and every one lands in the one step
/// the press opened: a stroke is one undo step, and the next stroke is another.
#[test]
fn a_stroke_is_one_undo_step() {
    let mut app = App::headless();
    let id = add(&mut app);
    let steps = app.undo_len();
    let brush = Brush {
        size: 6.0,
        color: [255, 255, 255, 255],
        erase: false,
    };
    let mut last = (100.0, 100.0);
    for i in 1..=5 {
        let next = (100.0 + 20.0 * i as f32, 100.0);
        let node = app.graph().get(id).unwrap().clone();
        let mut sheet = Sheet::of(&node);
        sheet.segment(&brush, Symmetry::None, last, next);
        last = next;
        paint(&mut app, id, &sheet.into_painting(0));
    }
    app.end_gesture();
    assert_eq!(app.undo_len(), steps + 1, "five segments, one stroke");

    let node = app.graph().get(id).unwrap().clone();
    let mut sheet = Sheet::of(&node);
    sheet.shape(
        Tool::Rect,
        &brush,
        Symmetry::None,
        (10.0, 10.0),
        (60.0, 60.0),
    );
    paint(&mut app, id, &sheet.into_painting(0));
    app.end_gesture();
    assert_eq!(app.undo_len(), steps + 2, "and the next stroke its own");

    assert!(app.undo());
    let back = held(app.graph(), id).unwrap();
    let (_, _, px) = pixels(&back);
    let at = |x: usize, y: usize| &px[(y * 512 + x) * 4..(y * 512 + x) * 4 + 4];
    assert_eq!(at(150, 100), [255, 255, 255, 255], "the pen stroke stays");
    assert_eq!(at(35, 10), [0, 0, 0, 255], "the rectangle went");
    assert!(app.undo());
    assert!(
        held(app.graph(), id).is_none(),
        "and the pen stroke with one more"
    );
}

/// A duplicate and a paste carry the picture, sharing its pixels until one of them paints —
/// and painting the copy leaves the original as it was.
#[test]
fn a_copied_canvas_carries_its_painting() {
    let mut app = App::headless();
    let id = add(&mut app);
    let painting = picture(512, 512);
    paint(&mut app, id, &painting);

    app.apply(Command::Duplicate {
        nodes: vec![id],
        offset: Vec2::new(40.0, 40.0),
    })
    .unwrap();
    let copy = app.graph().iter().map(|(id, _)| id).max().unwrap();
    assert_ne!(copy, id);
    let carried = held(app.graph(), copy).expect("the duplicate's painting");
    assert!(
        carried.shares(&painting),
        "the one picture, not a copy of it"
    );

    let clip = supersilvia::command::Clip {
        nodes: vec![app.graph().get(id).unwrap().clone()],
        edges: Vec::new(),
        from: None,
    };
    app.apply(Command::Paste {
        clip,
        at: Pos2::new(400.0, 40.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let pasted = app.graph().iter().map(|(id, _)| id).max().unwrap();
    assert_eq!(held(app.graph(), pasted), Some(painting.clone()));

    let mut sheet = Sheet::of(app.graph().get(copy).unwrap());
    sheet.fill([0, 255, 0, 255], (500.0, 500.0));
    paint(&mut app, copy, &sheet.into_painting(0));
    assert_ne!(
        held(app.graph(), copy).map(|p| pixels(&p)),
        Some(pixels(&painting))
    );
    assert_eq!(
        held(app.graph(), id).map(|p| pixels(&p)),
        Some(pixels(&painting)),
        "the original is untouched"
    );
}

// ------------------------------------------------------------------------ the project folder

/// The folder the painting's reference names is the project's own `assets/`.
#[test]
fn a_painting_lives_where_the_projects_assets_do() {
    assert_eq!(supersilvia::graph::PAINTING_FOLDER, ASSETS);
    assert!(picture(8, 8).file().starts_with("assets/painting-"));
}

/// Saved, the painting is a PNG in `assets/` named by what is in it, and the workspace file
/// names it and holds none of its pixels; opened, it is the same pixels to the byte —
/// transparent ones included.
#[test]
fn a_painting_round_trips_through_the_project_folder() {
    let root = dir("round-trip").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    let painting = picture(512, 256);
    app.apply(Command::SetOption {
        node: id,
        key: drawingcanvas::CANVAS_SIZE,
        value: "512x256".to_string(),
    })
    .unwrap();
    paint(&mut app, id, &painting);
    app.save_project().unwrap();

    let reference = painting.file().to_string();
    let files = painting_files(&root.join(ASSETS));
    assert_eq!(files, [reference.trim_start_matches("assets/")]);
    let ssw = std::fs::read_dir(root.join("workspaces"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.path().extension().is_some_and(|x| x == "ssw"))
        .unwrap();
    let text = std::fs::read_to_string(ssw.path()).unwrap();
    assert!(text.contains(&reference), "the file names the picture");
    assert!(text.len() < 8 * 1024, "and holds none of its pixels");

    let (_, graph, warnings) = Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let back = held(&graph, id).expect("the painting came back");
    assert!(back.is_read());
    assert_eq!(pixels(&back), pixels(&painting), "to the byte");
    assert_eq!(back.file(), reference, "and so under the same name");
}

/// The next save writes the new picture and deletes the one the last save wrote, since
/// nothing names it now — and leaves every other asset where it was.
#[test]
fn a_save_replaces_the_painting_it_supersedes() {
    let root = dir("replace").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    let first = picture(512, 512);
    stroke(&mut app, id, &first);
    app.save_project().unwrap();
    let assets = root.join(ASSETS);
    std::fs::write(assets.join("clip.webm"), b"not really a clip").unwrap();

    let mut sheet = Sheet::of(app.graph().get(id).unwrap());
    sheet.fill([0, 0, 255, 255], (500.0, 500.0));
    let second = sheet.into_painting(0);
    stroke(&mut app, id, &second);
    app.save_project().unwrap();
    assert_eq!(
        painting_files(&assets),
        [second.file().trim_start_matches("assets/")],
        "the new picture, and the old one gone"
    );
    assert!(
        assets.join("clip.webm").is_file(),
        "nothing else is touched"
    );

    // An undo back to the first picture is in memory, so the next save writes it again.
    assert!(app.undo());
    app.save_project().unwrap();
    let (_, graph, _) = Project::open(root).unwrap();
    assert_eq!(held(&graph, id).map(|p| pixels(&p)), Some(pixels(&first)));
}

/// A painting whose file is gone opens blank and says so, and keeps the name.
#[test]
fn a_painting_whose_file_is_gone_opens_blank_and_says_so() {
    let root = dir("gone").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    let painting = picture(512, 512);
    paint(&mut app, id, &painting);
    app.save_project().unwrap();
    for name in painting_files(&root.join(ASSETS)) {
        std::fs::remove_file(root.join(ASSETS).join(name)).unwrap();
    }

    let (_, graph, warnings) = Project::open(root).unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.to_string().contains("could not read its painting")),
        "{warnings:?}"
    );
    let unread = held(&graph, id).expect("the name is kept");
    assert!(!unread.is_read());
    assert_eq!(unread.file(), painting.file());
}

/// The autosave carries the painting: a stroke after the save is in `.autosave/`, as a
/// picture of its own there, and recovering the autosave brings it back.
#[test]
fn the_autosave_carries_the_painting() {
    let root = dir("autosave").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    app.save_project().unwrap();

    let painting = picture(512, 512);
    paint(&mut app, id, &painting);
    app.autosave_tick(0.0);
    app.wait_for_autosave();
    assert_eq!(
        painting_files(&root.join(AUTOSAVE).join(ASSETS)),
        [painting.file().trim_start_matches("assets/")],
        "the autosave's own picture"
    );
    assert!(
        painting_files(&root.join(ASSETS)).is_empty(),
        "and nothing written into the project's own assets"
    );
    drop(app);

    let mut app = App::headless();
    app.open_project(root.clone());
    assert!(app.recovery_offered().is_some());
    app.recover();
    assert_eq!(
        held(app.graph(), id).map(|p| pixels(&p)),
        Some(pixels(&painting)),
        "the stroke came back"
    );
    app.save_project().unwrap();
    assert!(!root.join(AUTOSAVE).exists());
    let (_, graph, _) = Project::open(root).unwrap();
    assert_eq!(
        held(&graph, id).map(|p| pixels(&p)),
        Some(pixels(&painting))
    );
}

/// An exported workspace takes its paintings with it, and importing it into another project
/// brings them in.
#[test]
fn an_export_and_an_import_carry_the_painting() {
    let base = dir("export");
    let root = base.join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    let painting = picture(256, 256);
    paint(&mut app, id, &painting);
    app.save_project().unwrap();
    let (project, graph, _) = Project::open(root).unwrap();
    let workspace = graph.default_workspace();

    let out = base.join("out");
    let report = project
        .export_workspace(&graph, workspace, &out)
        .expect("exported");
    assert!(
        report.assets.contains(&painting.file().to_string()),
        "{report:?}"
    );
    assert!(
        out.join(ASSETS)
            .join(&painting_files(&out.join(ASSETS))[0])
            .is_file()
    );

    let other = Project::new(base.join("saturday"));
    let mut into = Graph::new();
    let (_, imported) = other
        .import_workspace(&report.file, &mut into)
        .expect("imported");
    assert!(imported.missing.is_empty(), "{imported:?}");
    let arrived = into
        .iter()
        .find(|(_, n)| n.def.slug == "drawingcanvas")
        .map(|(id, _)| id)
        .unwrap();
    assert_eq!(
        held(&into, arrived).map(|p| pixels(&p)),
        Some(pixels(&painting))
    );
}

/// A workspace file that names somebody's media as a painting — edited by hand, or by a tool
/// that got it wrong — opens with a blank canvas, and no save afterwards deletes the media:
/// the only files a save deletes are the ones with a painting's own name.
#[test]
fn a_save_deletes_nothing_but_a_file_with_a_paintings_name() {
    let root = dir("not-a-painting").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let id = add(&mut app);
    app.save_project().unwrap();
    let clip = root.join(ASSETS).join("clip.webm");
    std::fs::create_dir_all(clip.parent().unwrap()).unwrap();
    std::fs::write(&clip, b"somebody's clip").unwrap();

    let ssw = std::fs::read_dir(root.join("workspaces"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "ssw"))
        .unwrap();
    let mut file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&ssw).unwrap()).unwrap();
    file["nodes"][0]["values"] = serde_json::json!({
        "painting": { "Painting": { "file": "assets/clip.webm" } }
    });
    std::fs::write(&ssw, serde_json::to_string_pretty(&file).unwrap()).unwrap();

    let mut app = App::headless();
    app.open_project(root.clone());
    let named = held(app.graph(), id).expect("the name is kept");
    assert!(!named.is_read(), "a clip is no picture");
    stroke(&mut app, id, &picture(512, 512));
    app.save_project().unwrap();
    assert!(
        clip.is_file(),
        "the clip outlives the save that stopped naming it"
    );
}
