// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: an Output's live recording, from the editor's side, with no GPU. What is held
//! here is when one may start and what ends it: a recording and a render never run at once,
//! an Output with nothing in it is refused on its own row, and a resolution changed or a tab
//! closed ends the file under its name and says why. The pictures in it are the GPU's, and
//! `tests/gpu_app.rs` reads them back; here every slot is the black a recording starts on.

use emath::Pos2;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use supersilvia::app::render::{Format, RenderSettings};
use supersilvia::clock::Warmup;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::nodes::output::RECORD_FPS;
use supersilvia::{App, Command};

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// A project of its own holding a checkerboard into an Output.
fn patch(name: &str) -> (App, NodeId, PathBuf) {
    let root = std::env::temp_dir().join(format!("supersilvia-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::headless();
    app.new_project(root.clone());
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.publish_plan();
    app.tick(1.0 / 60.0);
    (app, out, root)
}

fn has_codec() -> bool {
    let here = supersilvia::video::clip::Codec::probe().is_some();
    if !here {
        eprintln!("no hardware codec pair here; skipping");
    }
    here
}

/// Tick until the recording of `out` has ended and say how.
fn ends(app: &mut App, out: NodeId) -> Result<supersilvia::synth::Recorded, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        if app.recording_of(out).is_none()
            && let Some(outcome) = app.record_outcome(out)
        {
            return outcome.clone();
        }
        assert!(Instant::now() < deadline, "the recording never ended");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn render_settings(root: &std::path::Path) -> RenderSettings {
    RenderSettings {
        fps: 10.0,
        frames: 3,
        warmup: Warmup::Black,
        supersample: 1,
        format: Format::PngSequence,
        destination: root.join("renders/film"),
    }
}

/// **A recording and a render never run at once.** A render steps the show's time a frame at
/// a time and a recording follows it as it plays, so whichever is running refuses the other,
/// from the moment it is asked for.
#[test]
fn a_recording_and_a_render_cannot_both_run() {
    let (mut app, out, root) = patch("record-render");
    app.start_recording(out)
        .expect("a connected Output records");
    assert!(app.recording(), "recording from the moment it is asked for");
    let refused = app
        .start_render(out, &render_settings(&root))
        .expect_err("no render while a recording runs");
    assert!(refused.contains("recording"), "{refused}");
    assert!(!app.rendering());
    app.stop_recording(out);
    let _ = ends(&mut app, out);
    assert!(!app.recording());

    app.start_render(out, &render_settings(&root))
        .expect("a render once the recording is over");
    let refused = app
        .start_recording(out)
        .expect_err("no recording while a render runs");
    assert!(refused.contains("render"), "{refused}");
    app.cancel_render();
    let _ = std::fs::remove_dir_all(&root);
}

/// **A recording runs at the Record section's FPS**, not the Render section's: a second of the
/// show with the render at 30 and the recording at 24 is twenty-four frames, in
/// `recordings/`.
#[test]
fn a_recording_runs_at_its_own_fps_not_the_renders() {
    if !has_codec() {
        return;
    }
    let (mut app, out, root) = patch("record-fps");
    app.apply(Command::SetControls {
        node: out,
        values: vec![
            ("fps", ControlValue::Float(30.0)),
            (RECORD_FPS, ControlValue::Float(24.0)),
        ],
    })
    .unwrap();
    app.start_recording(out)
        .expect("a connected Output records");
    for _ in 0..60 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    app.stop_recording(out);
    let recorded = ends(&mut app, out).expect("the file closes");
    assert_eq!(recorded.frames, 24, "a second at 24 fps: {recorded:?}");
    assert_eq!(
        recorded.destination.parent(),
        Some(root.join("recordings").as_path())
    );
    let info = supersilvia::video::clip::discover(&recorded.destination).expect("playable");
    assert!((info.fps - 24.0).abs() < 1e-3, "{}", info.fps);
    assert_eq!(info.frames, 24, "{info:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// An Output with nothing cabled into it is refused, and the reason is the row's.
#[test]
fn an_output_with_nothing_in_it_is_not_recorded() {
    let mut app = App::headless();
    let out = add(&mut app, "output");
    let refused = app.start_recording(out).expect_err("nothing to record");
    assert!(refused.contains("nothing connected"), "{refused}");
    app.toggle_recording(out);
    assert!(!app.recording());
    assert!(
        app.record_outcome(out)
            .is_some_and(|o| o.as_ref().is_err_and(|e| e.contains("nothing connected"))),
        "the row says why"
    );
}

/// **A resolution changed mid-recording ends it.** A file is one size from its first frame to
/// its last, so the recording closes at the frame before the change, under its name, and says
/// why.
#[test]
fn a_resolution_change_stops_the_recording_and_says_so() {
    if !has_codec() {
        return;
    }
    let (mut app, out, root) = patch("record-resize");
    app.start_recording(out)
        .expect("a connected Output records");
    for _ in 0..20 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    assert!(app.recording_of(out).is_some());
    app.apply(Command::SetOption {
        node: out,
        key: "resolution",
        value: "1024x768".to_string(),
    })
    .unwrap();
    let recorded = ends(&mut app, out).expect("the file closes");
    assert!(
        recorded
            .why
            .as_deref()
            .is_some_and(|w| w.contains("resolution")),
        "{:?}",
        recorded.why
    );
    assert!(recorded.destination.is_file());
    assert!(!recorded.destination.with_extension("part").exists());
    let info = supersilvia::video::clip::discover(&recorded.destination).expect("playable");
    assert_eq!(
        (info.width, info.height),
        (1280, 720),
        "the size it began at"
    );
    assert!(
        app.file_status().contains("resolution"),
        "the status line says why: {}",
        app.file_status()
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Closing the Output's tab ends its recording, as the tab's nodes stop running.
#[test]
fn closing_its_tab_stops_the_recording() {
    if !has_codec() {
        return;
    }
    let (mut app, out, root) = patch("record-close");
    app.start_recording(out)
        .expect("a connected Output records");
    for _ in 0..10 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    let workspace = app.graph().default_workspace();
    app.close_workspace(workspace);
    let recorded = ends(&mut app, out).expect("the file closes");
    assert!(recorded.destination.is_file());
    assert!(
        recorded
            .why
            .as_deref()
            .is_some_and(|w| w.contains("closed")),
        "{:?}",
        recorded.why
    );
    let _ = std::fs::remove_dir_all(&root);
}
