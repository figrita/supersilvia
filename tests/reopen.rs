// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: Open and New replace the running state, and undo does not.
//!
//! Node ids restart at 1 in every project, so a project opened in a running app holds the
//! very ids the one it replaces did. Everything the synth keeps by id — a transport, a count,
//! a world, a deck, a recording on its way to the document — would carry across onto
//! whichever node now holds that id. So a project opened in a running `App` must publish
//! what a fresh `App` opening it publishes, and an undo, which restores the same patch, must
//! keep all of it.

use emath::Pos2;
use std::path::PathBuf;
use supersilvia::graph::{ControlValue, NodeId, PortRef, Value};
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

/// A directory of this test's own, empty, named so two tests running at once cannot collide.
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-reopen-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap();
}

/// One press and release of an action input's button, a tick each.
fn tap(app: &mut App, port: PortRef) {
    app.press(port, true);
    app.tick(FRAME);
    app.press(port, false);
    app.tick(FRAME);
}

/// The three nodes whose running state is the point: an automation, a counter and a slime
/// mold.
struct Nodes {
    automation: NodeId,
    counter: NodeId,
    mold: NodeId,
}

/// A saved project whose automation holds a ramp, and an app on it with the automation
/// playing, the counter at three and the mold some way into its world.
fn performed(root: &std::path::Path) -> (App, Nodes) {
    let mut app = App::headless();
    app.new_project(root.to_path_buf());
    let nodes = Nodes {
        automation: add(&mut app, "automation"),
        counter: add(&mut app, "counter"),
        mold: add(&mut app, "slimemold"),
    };
    let a = nodes.automation;
    set(&mut app, a, "duration", 0.25);
    tap(&mut app, PortRef::new(a, "record"));
    for i in 0..=20u8 {
        set(&mut app, a, "input", f32::from(i) / 20.0);
        app.tick(FRAME);
    }
    assert!(recording(&app, a).is_some(), "the ramp was recorded");
    set(&mut app, nodes.counter, "step", 1.0);
    set(&mut app, nodes.counter, "max", 10.0);
    for _ in 0..3 {
        tap(&mut app, PortRef::new(nodes.counter, "increment"));
    }
    app.save_project().unwrap();

    tap(&mut app, PortRef::new(a, "play"));
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert!(playing(&mut app, a), "the automation plays");
    assert_eq!(counted(&app, &nodes), Some(3.0));
    assert!(steps(&app, &nodes) > 0, "the mold has stepped");
    (app, nodes)
}

fn recording(app: &App, node: NodeId) -> Option<Vec<supersilvia::graph::Point>> {
    app.graph()
        .get(node)?
        .values
        .get("recording")
        .and_then(Value::points)
        .map(<[_]>::to_vec)
}

/// Whether the automation's output moves over ten ticks with its knob standing still, which
/// only a playing transport does: a stopped one publishes the knob.
fn playing(app: &mut App, node: NodeId) -> bool {
    let mut seen = Vec::new();
    for _ in 0..10 {
        app.tick(FRAME);
        seen.push(app.uniform(PortRef::new(node, "output")).unwrap());
    }
    seen.windows(2).any(|w| w[0] != w[1])
}

fn counted(app: &App, nodes: &Nodes) -> Option<f64> {
    app.uniform(PortRef::new(nodes.counter, "value"))
}

fn steps(app: &App, nodes: &Nodes) -> u64 {
    app.simulation(PortRef::new(nodes.mold, "trail"))
        .map_or(0, |s| s.steps)
}

/// Everything the three publish, as one comparable line.
fn published(app: &App, nodes: &Nodes) -> String {
    let sim = app
        .simulation(PortRef::new(nodes.mold, "trail"))
        .map(|s| (s.size, s.agents, s.steps, s.params.clone()));
    format!(
        "{:?} {:?} {sim:?}",
        app.uniform(PortRef::new(nodes.automation, "output")),
        counted(app, nodes),
    )
}

/// **A project reopened in the running app starts where a fresh app opening it starts**: the
/// automation stopped, the counter at zero and the mold's world born again — tick for tick
/// the same as an app that never ran anything.
#[test]
fn a_project_reopened_in_a_running_app_starts_as_a_fresh_app_opening_it() {
    let root = dir("fresh").join("friday");
    let (mut running, nodes) = performed(&root);
    running.open_project(root.clone());
    let mut fresh = App::headless();
    fresh.open_project(root);
    for i in 0..20 {
        running.tick(FRAME);
        fresh.tick(FRAME);
        assert_eq!(
            published(&running, &nodes),
            published(&fresh, &nodes),
            "tick {i} after the reopen differs from a fresh app's"
        );
    }
    assert_eq!(
        counted(&running, &nodes),
        Some(0.0),
        "the counter starts at zero"
    );
    assert!(
        !playing(&mut running, nodes.automation),
        "and the transport is stopped"
    );
    assert!(
        recording(&running, nodes.automation).is_some(),
        "with the recording kept, since it is the patch's"
    );
}

/// New is the same replacement: a node the new project's first `AddNode` makes holds the id
/// the old project's counter held, and it counts from zero.
#[test]
fn a_new_project_in_a_running_app_starts_from_nothing() {
    let root = dir("new");
    let (mut app, nodes) = performed(&root.join("friday"));
    app.new_project(root.join("saturday"));
    let counter = add(&mut app, "automation");
    assert_eq!(counter, nodes.automation, "ids start again at 1");
    let counter = add(&mut app, "counter");
    assert_eq!(counter, nodes.counter);
    app.tick(FRAME);
    assert_eq!(counted(&app, &nodes), Some(0.0));
    assert!(!playing(&mut app, nodes.automation));
}

/// **Undo keeps the running state**, because the graph it restores is the same patch: the
/// automation goes on playing, the counter stays at three and the mold carries on its world.
#[test]
fn undo_keeps_the_running_state() {
    let root = dir("undo").join("friday");
    let (mut app, nodes) = performed(&root);
    let before = steps(&app, &nodes);
    add(&mut app, "number");
    app.tick(FRAME);
    assert!(app.undo());
    app.tick(FRAME);
    assert!(playing(&mut app, nodes.automation), "still playing");
    assert_eq!(counted(&app, &nodes), Some(3.0), "still at three");
    assert!(
        steps(&app, &nodes) > before,
        "and the same world, further on"
    );
}

/// Two projects, each holding one `automation` at id 1 and an Output at id 2: what one of
/// them does on a tick the editor never read must not land on the other.
fn two_projects(name: &str) -> (App, PathBuf, NodeId, NodeId) {
    let root = dir(name);
    let other = root.join("saturday");
    let mut b = App::headless();
    b.new_project(other.clone());
    add(&mut b, "automation");
    add(&mut b, "output");
    b.save_project().unwrap();

    let mut app = App::headless();
    app.new_project(root.join("friday"));
    let automation = add(&mut app, "automation");
    let out = add(&mut app, "output");
    set(&mut app, automation, "duration", 0.25);
    tap(&mut app, PortRef::new(automation, "record"));
    for i in 0..10u8 {
        set(&mut app, automation, "input", f32::from(i) / 10.0);
        app.tick(FRAME);
    }
    // The recording stops on a tick the editor does not read, and a hand puts the Output on
    // deck A on another: both are on their way to the document, in a snapshot not taken.
    for _ in 0..20 {
        app.step_synth(FRAME);
    }
    app.press(PortRef::new(out, "show_a"), true);
    app.step_synth(FRAME);
    app.press(PortRef::new(out, "show_a"), false);
    app.step_synth(FRAME);
    (app, other, automation, out)
}

/// The control for the test below: read in the same project, both land.
#[test]
fn a_recording_and_a_deck_claim_the_editor_has_not_read_land_when_it_reads_them() {
    let (mut app, _, automation, out) = two_projects("landed");
    app.tick(FRAME);
    assert!(
        recording(&app, automation).is_some(),
        "the recording landed"
    );
    assert_eq!(app.mixer().a, Some(out), "and the claim did");
}

/// **What the old project did on a tick the editor never read is dropped by the project it
/// was for.** A recording and a deck claim still in an untaken snapshot when another project
/// opens would land on that project's node of the same id — a curve written into a file that
/// never recorded one, and an Output on air that nobody put there.
#[test]
fn a_recording_and_a_deck_claim_from_the_old_project_never_land_in_the_new_one() {
    let (mut app, other, automation, out) = two_projects("dropped");
    app.open_project(other);
    assert_eq!(app.graph().get(automation).unwrap().def.slug, "automation");
    assert_eq!(app.graph().get(out).unwrap().def.slug, "output");
    for _ in 0..3 {
        app.tick(FRAME);
    }
    assert!(
        recording(&app, automation).is_none(),
        "no curve was written"
    );
    assert!(!app.dirty(), "nothing was edited at all");
    assert_eq!(app.mixer().a, None, "no deck is claimed");
    assert_eq!(
        app.ticked().unwrap().decks(),
        (None, None),
        "on either side"
    );
}

/// A picture window on one of the old project's nodes closes, since the node that holds its
/// id now is another node; the mix's window is the rig's and stays.
#[test]
fn opening_a_project_closes_a_nodes_picture_window_and_keeps_the_mixs() {
    use supersilvia::render::picture::{Popped, Request, Shown};
    let root = dir("windows").join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let out = add(&mut app, "output");
    app.save_project().unwrap();
    for picture in [
        Shown::Node {
            node: out,
            port: None,
        },
        Shown::Mix,
    ] {
        app.pop_out(Request {
            picture,
            fullscreen: false,
        });
    }
    assert_eq!(app.popped_out().len(), 2);
    app.open_project(root);
    assert_eq!(
        app.popped_out(),
        [Popped {
            picture: Shown::Mix,
            fullscreen: false
        }]
    );
}

/// A snapshot of another project forgets every node it names, and keeps what is the run's:
/// the clock and the mix. Its events are the log's to forget; see `synth::events`.
#[test]
fn a_snapshot_of_another_project_keeps_only_the_runs_state() {
    use supersilvia::synth::Snapshot;
    let node = NodeId(1);
    let port = PortRef::new(node, "output");
    let mut s = Snapshot::default();
    s.uniforms.insert(port, 1.0);
    s.midi_writes.push((port, 0.5));
    s.render.awaiting_shader.insert(node);
    s.clock.ticks = 7;
    s.forget_project();
    assert!(s.uniforms.is_empty() && s.midi_writes.is_empty());
    assert!(s.render.awaiting_shader.is_empty());
    assert_eq!(s.clock.ticks, 7);
}

/// A render is reading the document, so Open and New wait for it: the project on screen is
/// the one being rendered until the render ends.
#[test]
fn open_and_new_are_refused_while_a_render_runs() {
    let root = dir("rendering");
    let other = root.join("saturday");
    let mut b = App::headless();
    b.new_project(other.clone());
    b.save_project().unwrap();

    let mut app = App::headless();
    app.new_project(root.join("friday"));
    let checker = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(checker, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let mut settings = app.render_settings_of(out).expect("an Output");
    settings.destination = root.join("film");
    app.start_render(out, &settings)
        .expect("a connected Output renders");
    assert!(app.rendering());

    app.open_project(other);
    app.new_project(root.join("sunday"));
    assert_eq!(
        app.project().root(),
        root.join("friday"),
        "still the project rendering"
    );
    assert!(app.graph().get(out).is_some());
    assert!(!root.join("sunday").exists(), "and no project was made");
}

/// **The Main Input is the project's**: set in the panel, saved, and back when the project is
/// opened again, in a fresh app and in the one that saved it after another project was open.
/// The project in between has its own, at the default.
#[test]
fn the_main_input_comes_back_with_its_project() {
    use supersilvia::maininput::{MainInput, VideoSource};
    use supersilvia::ui::maininput::MainInputAction;

    let base = dir("main-input");
    let (friday, saturday) = (base.join("friday"), base.join("saturday"));
    let clip = VideoSource::File {
        asset: "assets/gumbasia.webm".to_string(),
    };
    let mut app = App::headless();
    app.new_project(saturday.clone());
    app.new_project(friday.clone());
    app.handle_main_input(MainInputAction::SetVideo(clip.clone()));
    app.handle_main_input(MainInputAction::SetGain(2.0));
    assert!(!app.dirty(), "choosing a source is not an edit");
    app.save_project().unwrap();

    let chosen = app.project().main_input().clone();
    app.open_project(saturday);
    assert_eq!(
        *app.project().main_input(),
        MainInput::default(),
        "saturday has its own"
    );
    app.open_project(friday.clone());
    assert_eq!(
        *app.project().main_input(),
        chosen,
        "the same app gets it back"
    );

    let mut fresh = App::headless();
    fresh.open_project(friday);
    assert_eq!(fresh.project().main_input().video, clip, "a fresh app too");
    assert_eq!(fresh.project().main_input().gain, 2.0);
}
