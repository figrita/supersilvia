// SPDX-License-Identifier: AGPL-3.0-or-later

//! The CPU nodes that publish a picture of their own, driven through a headless `App`.
//!
//! A texture output is tested the way `tests/video.rs` tests a clip: tick until the node
//! publishes, then read the pixels back. What is asserted here is the shape every one of them
//! shares — a frame the size of the world it simulated, the same `Arc` again while nothing
//! changed, and the ports a graph reads it through.

use emath::Pos2;
use std::sync::Arc;
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::nodes::Frame;
use supersilvia::{App, Command};

const FRAME: f32 = 1.0 / 60.0;

/// A headless app holding one node of the given kind, ticked once so its state exists.
fn app_with(slug: &'static str) -> (App, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .expect("slug is in the registry");
    let id = app
        .graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added");
    app.tick(FRAME);
    (app, id)
}

/// What a port published this frame.
fn frame(app: &App, id: NodeId, port: &'static str) -> Arc<Frame> {
    Arc::clone(
        app.frame(PortRef::new(id, port))
            .unwrap_or_else(|| panic!("{port} published nothing")),
    )
}

/// Press a button, tick, let go, and tick again, so the release reaches the node's own gate
/// and the next press is a fresh edge.
fn tap(app: &mut App, id: NodeId, key: &'static str) {
    app.press(PortRef::new(id, key), true);
    app.tick(FRAME);
    app.press(PortRef::new(id, key), false);
    app.tick(FRAME);
}

fn set(app: &mut App, id: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node: id,
        key,
        value: ControlValue::Float(value),
    })
    .expect("a number control");
}

// ---------------------------------------------------------------- cellular automata

#[test]
fn the_automaton_publishes_a_frame_of_its_grid() {
    let (app, id) = app_with("cellularautomata");
    let cells = frame(&app, id, "cells");
    // silvia's default: `gridScale` of four sixteens.
    assert_eq!((cells.width, cells.height), (64, 64));
    let bytes = cells.bytes().expect("a grid is bytes, not a descriptor");
    assert_eq!(bytes.len(), 64 * 64 * 4);
    // Randomized on the first tick, so some cells are alive and some are not.
    let alive = bytes.as_chunks::<4>().0.iter().filter(|p| p[0] > 0).count();
    assert!(alive > 0 && alive < 64 * 64, "{alive} of 4096 alive");
}

#[test]
fn a_step_advances_a_generation() {
    let (mut app, id) = app_with("cellularautomata");
    let before = frame(&app, id, "cells");
    app.tick(FRAME);
    assert!(
        Arc::ptr_eq(&before, &frame(&app, id, "cells")),
        "a still automaton republishes the same frame rather than uploading it again"
    );

    tap(&mut app, id, "step");
    let after = frame(&app, id, "cells");
    assert!(!Arc::ptr_eq(&before, &after), "a press is a generation");
    assert_ne!(*before, *after, "and the generation is a different grid");
}

#[test]
fn randomize_refills_the_grid_and_the_threshold_says_how_full() {
    let (mut app, id) = app_with("cellularautomata");
    // Above every random: nothing is born.
    set(&mut app, id, "initThreshold", 1.0);
    tap(&mut app, id, "randomize");
    let empty = frame(&app, id, "cells");
    let alive = |f: &Frame| {
        f.bytes()
            .unwrap()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 0)
            .count()
    };
    assert_eq!(alive(&empty), 0, "a threshold of one leaves an empty grid");

    // Below every random: every cell is born.
    set(&mut app, id, "initThreshold", 0.0);
    tap(&mut app, id, "randomize");
    assert_eq!(
        alive(&frame(&app, id, "cells")),
        64 * 64,
        "and zero fills it"
    );
}

#[test]
fn the_grid_scale_option_reallocates_the_world() {
    let (mut app, id) = app_with("cellularautomata");
    app.apply(Command::SetOption {
        node: id,
        key: "gridScale",
        value: "8".to_string(),
    })
    .expect("a choice of the option");
    app.tick(FRAME);
    let cells = frame(&app, id, "cells");
    assert_eq!((cells.width, cells.height), (128, 128));
}

// ---------------------------------------------------------------- slime mold

/// What the mold published on its `trail` port: the world it steps on the GPU, as its tick
/// last described it. The pixels are the GPU's, and `tests/gpu_app.rs` reads them.
fn world(app: &App, id: NodeId) -> supersilvia::nodes::Simulation {
    app.simulation(PortRef::new(id, "trail"))
        .expect("the mold published its world")
        .clone()
}

#[test]
fn the_mold_publishes_a_world_for_the_gpu_and_no_frame() {
    let (app, id) = app_with("slimemold");
    let sim = world(&app, id);
    // silvia's default: `gridScale` of twelve sixteens, a fifth of it alive.
    assert_eq!((sim.size, sim.agents), (192, 7372));
    assert!(
        sim.params.contains(&("u_decay", 0.05)),
        "the knobs ride with it: {:?}",
        sim.params
    );
    assert!(
        app.frame(PortRef::new(id, "trail")).is_none(),
        "nothing is simulated on the CPU, so nothing is uploaded"
    );
}

/// silvia's rate — `Speed` steps thirty times a second — owed to the clock and paid on the
/// tick it falls due, so a second is 180 steps at the default whatever the ticks were.
#[test]
fn the_mold_steps_at_silvias_rate_off_the_one_clock() {
    let (mut app, id) = app_with("slimemold");
    let born = world(&app, id).steps;
    assert_eq!(born, 3, "a sixtieth of a second at 180 steps a second");
    for _ in 0..30 {
        app.tick(1.0 / 30.0);
    }
    assert_eq!(world(&app, id).steps - born, 180);
    for _ in 0..100 {
        app.tick(0.01);
    }
    let steps = world(&app, id).steps - born - 180;
    assert!(
        (179..=180).contains(&steps),
        "{steps} steps in a second of 100 Hz"
    );
    set(&mut app, id, "stepsPerFrame", 20.0);
    let before = world(&app, id).steps;
    app.tick(0.1);
    assert_eq!(
        world(&app, id).steps - before,
        60,
        "Speed 20 is 600 a second"
    );
}

#[test]
fn the_grid_and_the_population_are_reallocations() {
    let (mut app, id) = app_with("slimemold");
    for (key, value, side) in [("gridScale", "2", 32), ("gridScale", "4", 64)] {
        app.apply(Command::SetOption {
            node: id,
            key,
            value: value.to_string(),
        })
        .expect("a choice of the option");
        app.tick(FRAME);
        let sim = world(&app, id);
        assert_eq!(sim.size, side);
        assert_eq!(sim.agents, side * side / 5);
    }
    // A population is the other reallocation: at 1% of a 64x64 grid, forty agents.
    app.apply(Command::SetOption {
        node: id,
        key: "population",
        value: "1".to_string(),
    })
    .expect("a choice of the option");
    app.tick(FRAME);
    assert_eq!(world(&app, id).agents, 40);
}

/// The sensing knobs as the document holds them.
fn sensing(app: &App, id: NodeId) -> [f32; 3] {
    ["sensorAngle", "rotationAngle", "sensorOffset"].map(|key| {
        match app.graph().get(id).unwrap().controls.get(key) {
            Some(ControlValue::Float(v)) => *v,
            other => panic!("{key} is a number, not {other:?}"),
        }
    })
}

/// **Randomize is silvia's**: it rolls Sense Angle, Turn Angle and Sense Dist onto the knobs,
/// where they read and save and undo like a hand's turn — one step for all three. It is still
/// an action input, so the hand here is one source of it and a cable would be another. The
/// nudge it ends in is the world's, and `tests/gpu_app.rs` reads it back off the GPU.
#[test]
fn randomize_rolls_the_sensing_knobs_onto_the_document() {
    let (mut app, id) = app_with("slimemold");
    assert_eq!(sensing(&app, id), [19.0, 151.8, 22.0]);
    let steps = app.history().len();

    let randomize = PortRef::new(id, "randomSensors");
    app.press(randomize, true);
    app.tick(FRAME);
    app.press(randomize, false);
    app.tick(FRAME);

    let [sensor, rotation, offset] = sensing(&app, id);
    assert_ne!([sensor, rotation, offset], [19.0, 151.8, 22.0], "rolled");
    for angle in [sensor, rotation] {
        assert!((1.0..=180.0).contains(&angle), "{angle}");
    }
    assert!(
        (1.0..=39.0).contains(&offset) && offset.fract() == 0.0,
        "{offset}"
    );
    let degrees = std::f32::consts::PI / 180.0;
    assert!(
        world(&app, id)
            .params
            .contains(&("u_sensor", sensor * degrees)),
        "and the world steps with what was rolled"
    );
    assert_eq!(app.history().len(), steps + 1, "three knobs, one step");
    assert!(app.undo());
    assert_eq!(sensing(&app, id), [19.0, 151.8, 22.0]);
}

// ---------------------------------------------------------------- how each one is sampled

/// How the sampling a texture output declared reaches the renderer: the app copies it off the
/// definition into the frame job, per published texture. `render/` never reads the registry,
/// so the job is the whole of what it knows — and a simulated world that wraps in its own
/// simulation asks for `Repeat` so its picture wraps with it.
#[test]
fn a_wrapping_world_publishes_a_wrapping_texture() {
    use supersilvia::nodes::{TextureFilter, TextureWrap};

    let declared = |slug: &'static str, port: &'static str| {
        let (mut app, id) = app_with(slug);
        let job = app.build_frame_job();
        // A frame the renderer uploads, or a world it steps: either is sampled as declared.
        job.sources
            .iter()
            .find(|s| s.port == PortRef::new(id, port))
            .map(|s| (s.wrap, s.filter))
            .or_else(|| {
                job.sims
                    .iter()
                    .find(|s| s.port == PortRef::new(id, port))
                    .map(|s| (s.wrap, s.filter))
            })
            .unwrap_or_else(|| panic!("{slug}.{port} published nothing"))
    };

    // Cells are cells: they tile, and they stay crisp rather than being blended into a
    // smooth field they are not.
    assert_eq!(
        declared("cellularautomata", "cells"),
        (TextureWrap::Repeat, TextureFilter::Nearest)
    );
    // A scent field tiles too, and is smooth, which is the whole of what a scent field is.
    assert_eq!(
        declared("slimemold", "trail"),
        (TextureWrap::Repeat, TextureFilter::Linear)
    );
    // Everything else is the rule: mirrored outside its own bounds, linearly filtered.
    assert_eq!(
        declared("brickgame", "field"),
        (TextureWrap::Mirror, TextureFilter::Linear)
    );
}

// ---------------------------------------------------------------- brick game

#[test]
fn the_game_draws_its_field_and_starts_at_nothing() {
    let (app, id) = app_with("brickgame");
    let field = frame(&app, id, "field");
    assert_eq!((field.width, field.height), (300, 300));
    assert_eq!(app.uniform(PortRef::new(id, "score")), Some(0.0));
    assert_eq!(app.uniform(PortRef::new(id, "bricksLeft")), Some(48.0));
    let speed = app.uniform(PortRef::new(id, "ballVelocity")).unwrap();
    assert!(speed > 0.0, "the ball has a velocity before it is launched");

    // The wall is drawn: the top of the field has ink in it and the middle has none.
    let bytes = field.bytes().unwrap();
    let ink = |x: f32, y: f32| {
        let px = (f32::midpoint(x, 1.0) * 300.0) as usize;
        let py = ((1.0 - y) * 0.5 * 300.0) as usize;
        bytes[(py * 300 + px) * 4]
    };
    assert_eq!(ink(-0.875, 0.8), 255, "the first brick");
    assert_eq!(
        ink(0.0, 0.0),
        0,
        "and open field between the wall and the paddle"
    );
}

/// A press on Start fires `gameStarted` as a gate that opens for one frame: `Down` on the
/// tick it happened and `Up` on the next, because the event half has no pulse.
#[test]
fn starting_the_game_fires_a_one_frame_gate() {
    let (mut app, id) = app_with("brickgame");
    let started = PortRef::new(id, "gameStarted");
    assert!(app.edges(started).is_empty(), "nothing until a hand");

    app.press(PortRef::new(id, "startGame"), true);
    app.tick(FRAME);
    let kinds: Vec<_> = app.edges(started).iter().map(|e| e.edge).collect();
    assert_eq!(kinds, [supersilvia::nodes::Edge::Down]);

    app.press(PortRef::new(id, "startGame"), false);
    app.tick(FRAME);
    let kinds: Vec<_> = app.edges(started).iter().map(|e| e.edge).collect();
    assert_eq!(kinds, [supersilvia::nodes::Edge::Up], "the gate closes");

    app.tick(FRAME);
    assert!(app.edges(started).is_empty(), "and stays closed");
}

/// A game left running plays: the ball moves, so the picture is a new frame every tick, and a
/// paused game is the same frame again.
#[test]
fn a_running_game_moves_and_a_paused_one_does_not() {
    let (mut app, id) = app_with("brickgame");
    app.press(PortRef::new(id, "startGame"), true);
    app.tick(FRAME);
    app.press(PortRef::new(id, "startGame"), false);
    let before = frame(&app, id, "field");
    app.tick(FRAME);
    assert!(
        !Arc::ptr_eq(&before, &frame(&app, id, "field")),
        "the ball is still"
    );

    tap(&mut app, id, "pauseGame");
    let paused = frame(&app, id, "field");
    app.tick(FRAME);
    assert!(
        Arc::ptr_eq(&paused, &frame(&app, id, "field")),
        "a paused game republishes the same frame rather than uploading it again"
    );
}

/// Auto Play and Auto Reset between them make the node play itself, which is what a performer
/// patching it into a mix wants: a few seconds of it and the score has gone up.
#[test]
fn auto_play_scores_on_its_own() {
    let (mut app, id) = app_with("brickgame");
    set(&mut app, id, "autoPlay", 1.0);
    set(&mut app, id, "autoReset", 1.0);
    for _ in 0..600 {
        app.tick(FRAME);
    }
    let score = app.uniform(PortRef::new(id, "score")).unwrap();
    assert!(score > 0.0, "ten seconds of auto play scored nothing");
    let left = app.uniform(PortRef::new(id, "bricksLeft")).unwrap();
    assert!(left < 48.0, "and the wall is untouched");
}

// ---------------------------------------------------------------- image and gif

/// A scratch directory of this test's own, outside any project, for the file it writes.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-pictures-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Point a node's `file` at a picture, by way of the project — which copies it into `assets/`
/// and hands back the reference the option holds, exactly as the file button does.
fn show(app: &mut App, id: NodeId, file: &std::path::Path) {
    let reference = app
        .project()
        .import_asset(file)
        .expect("the project copied it in");
    app.apply(Command::SetOption {
        node: id,
        key: "file",
        value: reference,
    })
    .expect("an asset option holds any path");
}

/// Tick at 60 Hz until the decode lands, or give up. The decode is on a worker, as a
/// transcode is, so this is `tests/video.rs`'s own wait.
fn tick_until_decoded(app: &mut App, id: NodeId) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        app.tick(FRAME);
        let frames = app.uniform(PortRef::new(id, "frames")).unwrap_or(0.0);
        if frames > 0.0 {
            return frames as usize;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no frames in twenty seconds"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn a_still_image_decodes_into_one_frame() {
    let dir = scratch("still");
    let path = dir.join("swatch.png");
    let mut picture = image::RgbaImage::new(7, 3);
    for pixel in picture.pixels_mut() {
        *pixel = image::Rgba([10, 200, 30, 255]);
    }
    picture.save(&path).expect("a png of our own");

    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    assert_eq!(tick_until_decoded(&mut app, id), 1, "a still is one frame");

    let shown = frame(&app, id, "output");
    assert_eq!((shown.width, shown.height), (7, 3));
    assert_eq!(
        shown.bytes().unwrap()[..4],
        [10, 200, 30, 255],
        "the pixels are the file's, in RGBA"
    );
    // A still has nowhere to go, whatever the speed.
    for _ in 0..30 {
        app.tick(FRAME);
    }
    assert_eq!(app.uniform(PortRef::new(id, "frame")), Some(0.0));
}

/// A PNG is straight and the picture the node publishes is premultiplied, rounded to the
/// nearest byte: a half-transparent texel's color is scaled by its alpha, an opaque one is
/// the file's, and a transparent one is transparent black whatever color it was saved with.
#[test]
fn a_still_image_is_published_premultiplied() {
    let dir = scratch("straight");
    let path = dir.join("straight.png");
    let mut picture = image::RgbaImage::new(3, 1);
    picture.put_pixel(0, 0, image::Rgba([200, 100, 51, 128]));
    picture.put_pixel(1, 0, image::Rgba([10, 200, 30, 255]));
    picture.put_pixel(2, 0, image::Rgba([255, 255, 255, 0]));
    picture.save(&path).expect("a png of our own");

    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    tick_until_decoded(&mut app, id);
    let shown = frame(&app, id, "output");
    assert_eq!(
        shown.bytes().unwrap(),
        [100, 50, 26, 128, 10, 200, 30, 255, 0, 0, 0, 0],
        "200, 100 and 51 times 128/255, rounded"
    );
}

#[test]
fn a_two_frame_gif_advances_at_its_own_delays() {
    let dir = scratch("gif");
    let path = dir.join("flip.gif");
    let colors = [[220, 20, 20, 255], [20, 20, 220, 255]];
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for rgba in colors {
            let mut picture = image::RgbaImage::new(4, 2);
            for pixel in picture.pixels_mut() {
                *pixel = image::Rgba(rgba);
            }
            encoder
                .encode_frame(image::Frame::from_parts(
                    picture,
                    0,
                    0,
                    // A tenth of a second a frame, which is what a hand-authored gif says.
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .expect("a gif of our own");
        }
    }

    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    assert_eq!(tick_until_decoded(&mut app, id), 2, "both frames arrived");
    // Back to zero, so the animation is at its start on ambient time.
    app.transport(supersilvia::transport::Command::Seek(0.0));
    app.tick(FRAME);

    let first = frame(&app, id, "output");
    assert_eq!((first.width, first.height), (4, 2));
    assert_eq!(app.uniform(PortRef::new(id, "frame")), Some(0.0));

    // On ambient time the animation runs at its own pace: a tenth of a second is a frame on.
    for _ in 0..6 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(PortRef::new(id, "frame")),
        Some(1.0),
        "a tenth of a second is the second frame"
    );
    let second = frame(&app, id, "output");
    assert!(!Arc::ptr_eq(&first, &second), "and a different picture");
    assert_ne!(
        first.bytes().unwrap()[..4],
        second.bytes().unwrap()[..4],
        "the two frames were authored different colors"
    );

    // Back to zero puts it back on the first frame and the picture with it.
    app.transport(supersilvia::transport::Command::Seek(0.0));
    app.tick(FRAME);
    assert_eq!(app.uniform(PortRef::new(id, "frame")), Some(0.0));
}

/// Offset is added to Time, 1 across the GIF's frames' own delays: with Time at zero it is the
/// whole position, and past either end it wraps as a GIF does, so −0.5 is half of it.
#[test]
fn a_gifs_position_is_added_over_its_own_delays() {
    let dir = scratch("position");
    let path = dir.join("position.gif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        // Four frames, the second held three times as long as each of the others: six
        // tenths in all, so a half is three tenths in, which is the second frame.
        for delay in [100, 300, 100, 100] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::new(2, 2),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }
    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    assert_eq!(tick_until_decoded(&mut app, id), 4);
    // Time at zero and standing: the show paused at its start.
    app.transport(supersilvia::transport::Command::Pause);
    app.transport(supersilvia::transport::Command::Seek(0.0));
    let frame = |app: &mut App, position: f32| {
        set(app, id, "phaseOffset", position);
        app.tick(FRAME);
        app.uniform(PortRef::new(id, "frame")).unwrap()
    };
    assert_eq!(frame(&mut app, 0.0), 0.0);
    assert_eq!(frame(&mut app, 0.1), 0.0, "six hundredths in is the first");
    assert_eq!(
        frame(&mut app, 0.5),
        1.0,
        "half is inside the long second frame"
    );
    assert_eq!(frame(&mut app, 0.75), 2.0);
    assert_eq!(frame(&mut app, 0.95), 3.0);
    assert_eq!(frame(&mut app, -0.5), 1.0, "half back is half on");
    assert_eq!(frame(&mut app, -0.05), 3.0, "a hair back is the last");
    assert_eq!(frame(&mut app, -1.0), 0.0, "a whole play back is the start");

    // Played on top of a half: three tenths in, and fifteen hundredths of a second on.
    set(&mut app, id, "phaseOffset", 0.5);
    app.transport(supersilvia::transport::Command::Play);
    for _ in 0..9 {
        app.tick(FRAME);
    }
    assert_eq!(
        app.uniform(PortRef::new(id, "frame")),
        Some(2.0),
        "played on from where Offset put it, into the third frame"
    );
}

/// **A gear in a GIF's Time plays it.** A Ratio Gear at 1 : 2 on ambient seconds is a play
/// every two seconds, so a second and a half in the GIF is three quarters of the way through
/// its four equal frames, and two and a half seconds in, a play and a quarter, it is a quarter
/// through: the frame follows the gear and wraps with it.
#[test]
fn a_gear_in_a_gifs_time_plays_it() {
    let dir = scratch("gear");
    let path = dir.join("gear.gif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for _ in 0..4 {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::new(2, 2),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .unwrap();
        }
    }
    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    assert_eq!(tick_until_decoded(&mut app, id), 4);
    app.apply(Command::AddNode {
        slug: "ratiogear",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let clock = app.graph().iter().map(|(id, _)| id).max().unwrap();
    set(&mut app, clock, "q", 2.0);
    app.apply(Command::SetOption {
        node: id,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(clock, "cycles"),
        to: PortRef::new(id, supersilvia::nodes::TIME),
    })
    .unwrap();
    app.transport(supersilvia::transport::Command::Seek(0.0));
    app.tick_at(0.0);
    let start = app.uniform(PortRef::new(clock, "cycles")).unwrap();
    let at = |app: &mut App, seconds: f64| {
        app.tick_at(seconds);
        let cycles = app.uniform(PortRef::new(clock, "cycles")).unwrap() - start;
        (cycles, app.uniform(PortRef::new(id, "frame")).unwrap())
    };
    let (cycles, frame) = at(&mut app, 1.5);
    assert!((cycles - 0.75).abs() < 1e-3, "{cycles}");
    assert_eq!(frame, 3.0, "three quarters through is the last frame");
    let (cycles, frame) = at(&mut app, 2.5);
    assert!((cycles - 1.25).abs() < 1e-3, "{cycles}");
    assert_eq!(frame, 1.0, "a play and a quarter is a quarter through");
}

/// A number standing still in a GIF's Time holds it where it is, on every tick.
#[test]
fn a_still_clock_holds_a_gif() {
    let dir = scratch("hold");
    let path = dir.join("hold.gif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for _ in 0..3 {
            let picture = image::RgbaImage::new(2, 2);
            encoder
                .encode_frame(image::Frame::from_parts(
                    picture,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(20, 1),
                ))
                .unwrap();
        }
    }
    let (mut app, id) = app_with("imagegif");
    show(&mut app, id, &path);
    assert_eq!(tick_until_decoded(&mut app, id), 3);

    app.apply(Command::AddNode {
        slug: "number",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let still = app.graph().iter().map(|(id, _)| id).max().unwrap();
    app.apply(Command::SetOption {
        node: id,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(still, "output"),
        to: PortRef::new(id, supersilvia::nodes::TIME),
    })
    .unwrap();
    // Three frames of 20 ms: on ambient time the GIF would pass through all three every
    // 60 ms, so a cable ignored shows on one tick in three or fewer.
    for tick in 0..60 {
        app.tick(FRAME);
        assert_eq!(
            app.uniform(PortRef::new(id, "frame")),
            Some(0.0),
            "a still clock is a hold, not a slow play, on tick {tick}"
        );
    }
}
