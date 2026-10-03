// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: time, inverted. Live, time is a function of the wall clock; offline it is a
//! function of the frame index, and nothing may consult the wall. What is held here is that
//! a driven transport reaches everything that samples time — `u_time` through the frame job,
//! a CPU node through its advance — and that the same frame gives the same time however many
//! times it is asked.

use emath::Pos2;
use supersilvia::clock::{Stepper, Warmup};
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::transport;
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

/// A Ratio Gear at ×1 on ambient seconds: the playhead, as a gear counts it.
fn seconds(app: &mut App) -> NodeId {
    add(app, "ratiogear")
}

/// A counter counting a half-second Master Gear's beats: what a run through a warm-up leaves
/// behind, which a gear re-born at the first frame does not.
fn beats(app: &mut App) -> NodeId {
    use supersilvia::graph::ControlValue;
    let clock = add(app, "mastergear");
    let counter = add(app, "counter");
    for (node, key, value) in [
        (clock, "length", 0.5),
        (counter, "step", 1.0),
        (counter, "max", 100.0),
    ] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.apply(Command::Connect {
        from: PortRef::new(clock, "trigger"),
        to: PortRef::new(counter, "increment"),
    })
    .unwrap();
    counter
}

/// Run a stepper's frames the way a render does: a seek to the first frame's time, then the
/// transport driven to each frame's own.
fn render(app: &mut App, stepper: &Stepper) {
    app.transport(transport::Command::Seek(stepper.time_of(0)));
    for i in 0..stepper.len() {
        app.tick_at(stepper.time_of(i));
    }
}

/// `u_time` is the playhead's fraction of a second, and the playhead is wherever a render
/// drove it: the same `t` gives the same uniform twice, whatever was asked for in between. A
/// warm-up's negative time has its fraction too: −2.25 s is a quarter short of −2.
#[test]
fn the_same_t_gives_the_same_u_time_twice() {
    let mut app = App::headless();
    let out = add(&mut app, "output");
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    app.tick_at(4.5);
    let first = app.build_frame_job().time;
    assert_eq!(first, 0.5);

    // Somewhere else, then back — a warm-up starts before zero, a run starts over.
    app.tick_at(-2.25);
    assert_eq!(app.build_frame_job().time, 0.75);
    app.tick_at(4.5);
    assert_eq!(app.build_frame_job().time, first);
}

/// A run over a patch with a gear: a gear is born at the render's first frame where the
/// playhead puts it, and after the stepper has driven the playhead a frame per frame it
/// reads the playhead, at any frame rate, and two runs of the same patch read the same.
#[test]
fn a_stepped_run_reads_the_playhead_it_was_driven_to() {
    fn run(fps: f64, frames: u32, warmup: Warmup) -> f32 {
        let mut app = App::headless();
        let gear = seconds(&mut app);
        render(&mut app, &Stepper::new(fps, frames, warmup));
        app.uniform(PortRef::new(gear, "cycles")).unwrap()
    }

    // At 5 fps a frame is longer than the live clamp, and it must count whole.
    let seconds = run(5.0, 11, Warmup::Black);
    assert!(
        (seconds - 2.0).abs() < 1e-5,
        "eleven frames at 5 fps: {seconds}"
    );

    // The warm-up is the patch running from before the beginning, and the gear with it.
    let seconds = run(30.0, 31, Warmup::Run(30));
    assert!(
        (seconds - 1.0).abs() < 1e-5,
        "thirty frames at 30 fps after the warm-up: {seconds}"
    );

    assert_eq!(
        run(60.0, 100, Warmup::Run(10)),
        run(60.0, 100, Warmup::Run(10)),
        "a run is repeatable"
    );
}

/// The three warm-ups, told apart by the beats a counter took from a Master Gear by the
/// first kept frame. Each render is born on a whole cycle of the half-second master and fires
/// that beat: a hold and black count it alone, at zero, and a run is born at −2 s and counts
/// it and the four beats of the warm-up's two seconds. Every gear is at zero on the first
/// kept frame whatever came before it, and so is the Time node, which reads the playhead.
#[test]
fn a_counter_tells_the_warm_ups_apart() {
    fn first_kept(warmup: Warmup) -> (f32, f32, f32) {
        let mut app = App::headless();
        let gear = seconds(&mut app);
        let counted = beats(&mut app);
        let time = add(&mut app, "time");
        render(&mut app, &Stepper::new(10.0, 1, warmup));
        (
            app.uniform(PortRef::new(gear, "cycles")).unwrap(),
            app.uniform(PortRef::new(counted, "value")).unwrap(),
            app.uniform(PortRef::new(time, "seconds")).unwrap(),
        )
    }
    assert_eq!(first_kept(Warmup::Black), (0.0, 1.0, 0.0));
    assert_eq!(first_kept(Warmup::Hold(20)), (0.0, 1.0, 0.0));
    let (gear, counted, playhead) = first_kept(Warmup::Run(20));
    assert!(gear.abs() < 1e-5, "{gear}");
    assert_eq!(
        counted, 5.0,
        "the beat it was born on and four half-second beats in two seconds of warm-up"
    );
    assert_eq!(playhead, 0.0);
}

/// **A render's first frame is a ÷4 gear's downbeat whatever the warm-up.** A Ratio Gear at ÷4
/// on a two-second Master Gear's Cycles, rendered at 30 fps after thirty frames of run: the
/// warm-up's first frame is a seek to −1 s, where the master is half a cycle before zero, and
/// the ÷4 is born at a quarter of that, an eighth before zero, so a second of warm-up brings
/// both to zero on the first kept frame. With a black warm-up both are born there, and each
/// fires that one beat.
#[test]
fn a_ratio_gear_is_at_zero_on_a_renders_first_frame_after_a_warm_up() {
    use supersilvia::graph::ControlValue;
    let mut app = App::headless();
    let clock = add(&mut app, "mastergear");
    let quarter = add(&mut app, "ratiogear");
    for (node, key, value) in [(clock, "length", 2.0), (quarter, "ratio", 0.25)] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.apply(Command::Connect {
        from: PortRef::new(clock, "cycles"),
        to: PortRef::new(quarter, "clock"),
    })
    .unwrap();
    for _ in 0..100 {
        app.tick(1.0 / 60.0);
    }
    app.reset_cpu();
    render(&mut app, &Stepper::new(30.0, 1, Warmup::Run(30)));
    let (m, q) = (
        app.uniform(PortRef::new(clock, "cycles")).unwrap(),
        app.uniform(PortRef::new(quarter, "cycles")).unwrap(),
    );
    assert!(m.abs() < 1e-5, "the master is at zero: {m}");
    assert!(q.abs() < 1e-5, "and so is the ÷4: {q}");

    // A black warm-up, after the live show ran some seconds back: both are born at zero on
    // the first frame, a downbeat of each, and the ÷4 has none for the distance back.
    for _ in 0..200 {
        app.tick(1.0 / 60.0);
    }
    app.reset_cpu();
    render(&mut app, &Stepper::new(30.0, 1, Warmup::Black));
    for (id, name) in [(clock, "the master"), (quarter, "the ÷4")] {
        let downs = app
            .edges(PortRef::new(id, "trigger"))
            .iter()
            .filter(|e| e.is_down())
            .count();
        assert_eq!(downs, 1, "{name} fires the beat it is born on");
    }
}

/// A `slug` sequencer with only step 0 of lane 1 lit, its Time on a one-second Master Gear's
/// Cycles, after some live play and every CPU node reset, as a render resets them.
fn step_zero_on_a_master(slug: &'static str) -> (App, NodeId) {
    use supersilvia::graph::{ControlValue, Value};
    let mut app = App::headless();
    let clock = add(&mut app, "mastergear");
    app.apply(Command::SetControl {
        node: clock,
        key: "length",
        value: ControlValue::Float(1.0),
    })
    .unwrap();
    let seq = add(&mut app, slug);
    if slug == "stepsequencer" {
        app.apply(Command::SetValue {
            node: seq,
            key: "pattern",
            value: Value::Cells(vec!["x...............".to_string()]),
        })
        .unwrap();
    } else {
        for (key, value) in [("lane1steps", 16.0), ("lane1pulses", 1.0)] {
            app.apply(Command::SetControl {
                node: seq,
                key,
                value: ControlValue::Float(value),
            })
            .unwrap();
        }
    }
    app.apply(Command::SetOption {
        node: seq,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(clock, "cycles"),
        to: PortRef::new(seq, supersilvia::nodes::TIME),
    })
    .unwrap();
    for _ in 0..100 {
        app.tick(1.0 / 60.0);
    }
    app.reset_cpu();
    (app, seq)
}

/// **A render's first frame has step 0's gate open, whatever the warm-up.** The sequencers of
/// [`step_zero_on_a_master`], rendered at 30 fps: after a black warm-up the first kept frame
/// lands on step 0 and opens lane 1; after a run the show crosses step 0 on it; and a hold,
/// which lands on step 0 on its first frame and stands there to the first kept frame, keeps the
/// gate that landing opened open across it — a Time standing still closes a gate a step opened
/// as the Time passed it, and leaves one a landing opened until the Time moves on. A bar on,
/// the gate opens again under every warm-up.
#[test]
fn a_renders_first_frame_has_step_zeros_gate_open_under_every_warm_up() {
    for slug in ["stepsequencer", "euclideanrhythm"] {
        for warmup in [Warmup::Black, Warmup::Run(30), Warmup::Hold(30)] {
            let (mut app, seq) = step_zero_on_a_master(slug);
            let stepper = Stepper::new(30.0, 31, warmup);
            app.transport(transport::Command::Seek(stepper.time_of(0)));
            let mut open = false;
            for i in 0..stepper.len() {
                app.tick_at(stepper.time_of(i));
                let was_open = open;
                let mut opened = false;
                for event in app.edges(PortRef::new(seq, "lane1")) {
                    opened |= event.is_down();
                    open = event.is_down();
                }
                if matches!(stepper.kept_index(i), Some(0 | 30)) {
                    assert!(
                        was_open || opened,
                        "{slug} at {warmup:?}: lane 1 is open on kept frame {:?}",
                        stepper.kept_index(i)
                    );
                }
            }
        }
    }
}

/// **A render plays the step the grid stands on at its first frame.** A Step Sequencer and a
/// Euclidean Rhythm with only step 0 lit, on a one-second Master Gear's Cycles, rendered at
/// 30 fps after some live play and every CPU node reset, as a render resets them: with a
/// black warm-up, or a run of none, the first frame is the bar's downbeat and lane 1 opens on
/// it, then on every bar after, frames 0, 30, 60 and 90.
#[test]
fn a_sequencer_plays_its_first_step_on_a_renders_first_frame() {
    for slug in ["stepsequencer", "euclideanrhythm"] {
        for warmup in [Warmup::Black, Warmup::Run(0)] {
            let (mut app, seq) = step_zero_on_a_master(slug);
            let stepper = Stepper::new(30.0, 91, warmup);
            app.transport(transport::Command::Seek(stepper.time_of(0)));
            let mut opens = Vec::new();
            for i in 0..stepper.len() {
                app.tick_at(stepper.time_of(i));
                if app
                    .edges(PortRef::new(seq, "lane1"))
                    .iter()
                    .any(|e| e.is_down())
                {
                    opens.push(i);
                }
            }
            assert_eq!(
                opens,
                [0, 30, 60, 90],
                "{slug} at {warmup:?}: lane 1 opens on every bar, the first frame's too"
            );
        }
    }
}

/// **A warm-up before zero reads what a loop later reads.** A four-second Master Gear, a
/// Ratio Gear at ×1 counting its Cycles and one at ×2 counting that — "Reverse the show"'s
/// Show and Breathe gears — the Time node and `u_time`, over a render with three loops of
/// warm-up, as `examples/loop_gifs` runs one: each frame of the last warm-up loop reads the
/// fraction a shader reads of each count, and `u_time`, to the bit, as the frame a loop later
/// does. A count is published whole and split for a shader into its whole part and the `f32`
/// of its fraction, so a count before zero has the precision a loop later has.
#[test]
fn a_warm_up_before_zero_reads_what_a_loop_later_reads() {
    use supersilvia::graph::ControlValue;
    let mut app = App::headless();
    let clock = add(&mut app, "mastergear");
    let show = add(&mut app, "ratiogear");
    let breathe = add(&mut app, "ratiogear");
    for (node, key, value) in [(clock, "length", 4.0), (breathe, "ratio", 2.0)] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    for (from, to) in [(clock, show), (show, breathe)] {
        app.apply(Command::Connect {
            from: PortRef::new(from, "cycles"),
            to: PortRef::new(to, "clock"),
        })
        .unwrap();
    }
    let time = add(&mut app, "time");
    let out = add(&mut app, "output");
    let cb = add(&mut app, "checkerboard");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let (fps, loop_frames) = (30.0, 120);
    let stepper = Stepper::new(fps, loop_frames + 1, Warmup::Run(3 * loop_frames));
    app.transport(transport::Command::Seek(stepper.time_of(0)));
    let names = ["master", "show", "breathe", "seconds", "u_time"];
    let mut counts = Vec::new();
    let mut read = Vec::new();
    for i in 0..stepper.len() {
        app.tick_at(stepper.time_of(i));
        let count = |port| app.count(port).unwrap();
        let at = [
            count(PortRef::new(clock, "cycles")),
            count(PortRef::new(show, "cycles")),
            count(PortRef::new(breathe, "cycles")),
            count(PortRef::new(time, "seconds")),
        ];
        let fraction = |x: f64| supersilvia::nodes::phasor::split(x)[1].to_bits();
        read.push([
            fraction(at[0]),
            fraction(at[1]),
            fraction(at[2]),
            fraction(at[3]),
            app.build_frame_job().time.to_bits(),
        ]);
        counts.push(at);
    }
    let first = 3 * loop_frames as usize;
    for i in first - loop_frames as usize..first {
        assert!(
            counts[i][0] <= 0.0 && counts[i][3] <= 0.0,
            "{i} is before zero"
        );
        let later = i + loop_frames as usize;
        for (k, name) in names.iter().enumerate() {
            assert_eq!(
                read[i][k],
                read[later][k],
                "frame {i}'s {name}: {:?} before zero and {:?} a loop later read one fraction",
                counts[i].get(k),
                counts[later].get(k),
            );
        }
    }
    assert_eq!(
        [counts[first][0], counts[first][3]],
        [0.0; 2],
        "and the first kept frame is zero"
    );
    assert_eq!(read[first][4], 0.0f32.to_bits(), "u_time too");
}

/// The live clock is not what a render runs on. Swapping the stepper's clock in and out
/// leaves the live one where it was, so a set carries on from the same second after a render
/// as before it.
#[test]
fn a_run_leaves_the_live_clock_alone() {
    let mut app = App::headless();
    app.clock_mut().tick(100.0);
    app.clock_mut().tick(101.0);
    let before = app.clock().elapsed();

    let mut stepper = Stepper::new(30.0, 30, Warmup::Black);
    for i in 0..stepper.len() {
        std::mem::swap(app.clock_mut(), stepper.clock_mut());
        app.tick_at(stepper.time_of(i));
        std::mem::swap(app.clock_mut(), stepper.clock_mut());
    }

    assert_eq!(app.clock().elapsed(), before);
    assert!((app.clock_mut().tick(101.05) - 0.05).abs() < 1e-6);
}

/// The node describes its own render: silvia's three numbers and its warm-up mode, read off
/// the Output into the settings a render starts from. Duration times fps is the frame count,
/// rounded up so a fraction of a frame is a frame; a black warm-up has no frames whatever the
/// number says.
#[test]
fn an_output_describes_its_own_render() {
    use supersilvia::graph::ControlValue;
    let mut app = App::headless();
    let out = add(&mut app, "output");
    let defaults = app.render_settings_of(out).unwrap();
    assert_eq!(
        (defaults.fps, defaults.frames),
        (30.0, 300),
        "30 fps for 10 s"
    );
    assert_eq!(defaults.warmup, Warmup::Run(0));
    assert!(defaults.destination.ends_with("renders/output1-001"));

    for (key, value) in [("fps", 24.0), ("duration", 2.05), ("warmup", 5.0)] {
        app.apply(Command::SetControl {
            node: out,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.apply(Command::SetOption {
        node: out,
        key: "warmupMode",
        value: "hold".to_string(),
    })
    .unwrap();
    let settings = app.render_settings_of(out).unwrap();
    assert_eq!(
        (settings.fps, settings.frames),
        (24.0, 50),
        "49.2 frames is 50"
    );
    assert_eq!(settings.warmup, Warmup::Hold(5));

    app.apply(Command::SetOption {
        node: out,
        key: "warmupMode",
        value: "black".to_string(),
    })
    .unwrap();
    assert_eq!(app.render_settings_of(out).unwrap().warmup, Warmup::Black);

    app.apply(Command::SetOption {
        node: out,
        key: "writer",
        value: "video".to_string(),
    })
    .unwrap();
    let video = app.render_settings_of(out).unwrap();
    assert_eq!(video.format, supersilvia::app::render::Format::Video);
    assert!(video.destination.ends_with("renders/output1-001.mp4"));
}

/// Offline audio, and the Main Input's clip with it: the one `position` both are read at is
/// integrated from the transport's advance live, and placed at the frame's own time while a
/// render drives it —
/// so an audio-reactive patch renders against the file, and the live playhead is where it was
/// once the render hands the clock back.
#[test]
fn a_render_drives_the_main_inputs_file_and_hands_it_back() {
    use supersilvia::maininput::MainInput;
    use supersilvia::synth::maininput::Live;
    let mut live = Live::default();
    let want = MainInput::default();
    let cache = std::env::temp_dir().join("supersilvia-drive-test");
    let resolve = |r: &str| std::path::PathBuf::from(r);
    let tick = supersilvia::transport::Time {
        advance: 0.1,
        ..Default::default()
    };
    for _ in 0..3 {
        live.reconcile(&want, &resolve, &cache, &tick);
    }
    assert!(
        (live.position() - 0.3).abs() < 1e-6,
        "live, it integrates: {}",
        live.position()
    );

    live.drive(Some(4.5));
    live.reconcile(&want, &resolve, &cache, &tick);
    assert_eq!(live.position(), 4.5, "driven, it is where the frame is");
    live.reconcile(&want, &resolve, &cache, &tick);
    assert_eq!(live.position(), 4.5, "and does not run on between frames");
    live.drive(Some(-0.5));
    live.reconcile(&want, &resolve, &cache, &tick);
    assert_eq!(
        live.position(),
        -0.5,
        "a warm-up runs from before the beginning"
    );

    live.drive(None);
    live.set_position(0.3);
    live.reconcile(&want, &resolve, &cache, &tick);
    assert!(
        (live.position() - 0.4).abs() < 1e-6,
        "handed back, it runs on from where it was"
    );
}

/// The `!`: a walk back from each Output through the cables for every source that can only
/// answer now. A camera anywhere upstream counts, a checkerboard does not, and a `maininput`
/// node counts only while the panel is pointed at a device rather than a file or nothing.
#[test]
fn an_output_knows_which_of_its_sources_cannot_be_stepped() {
    use supersilvia::maininput::VideoSource;
    use supersilvia::ui::maininput::MainInputAction;
    let mut app = App::headless();
    let camera = add(&mut app, "camera");
    let rotate = add(&mut app, "rotate");
    let live_out = add(&mut app, "output");
    let cb = add(&mut app, "checkerboard");
    let pure_out = add(&mut app, "output");
    let input = add(&mut app, "maininput");
    let panel_out = add(&mut app, "output");
    for (from, to) in [
        (PortRef::new(camera, "frame"), PortRef::new(rotate, "input")),
        (
            PortRef::new(rotate, "output"),
            PortRef::new(live_out, "input"),
        ),
        (PortRef::new(cb, "output"), PortRef::new(pure_out, "input")),
        (
            PortRef::new(input, "frame"),
            PortRef::new(panel_out, "input"),
        ),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }

    let live = app.live_sources();
    let through_rotate = live.get(&live_out).expect("a camera upstream");
    assert_eq!(through_rotate.len(), 1);
    assert_eq!(through_rotate[0].node, camera);
    assert!(!through_rotate[0].panel);
    assert!(
        through_rotate[0].workspace.is_some(),
        "and a way to go there"
    );
    assert!(
        !live.contains_key(&pure_out),
        "a checkerboard answers at any t"
    );
    assert!(
        !live.contains_key(&panel_out),
        "the panel points at nothing, so its node reads nothing live"
    );

    app.handle_main_input(MainInputAction::SetVideo(VideoSource::Camera {
        device: "test".to_string(),
    }));
    let live = app.live_sources();
    let panel = live.get(&panel_out).expect("the panel is a camera now");
    assert_eq!(panel.len(), 1);
    assert!(panel[0].panel, "and going there means the panel: {panel:?}");
    assert!(panel[0].label.contains("camera"), "{}", panel[0].label);

    app.handle_main_input(MainInputAction::SetVideo(VideoSource::File {
        asset: "assets/clip.webm".to_string(),
    }));
    assert!(
        !app.live_sources().contains_key(&panel_out),
        "a file is exact at any t"
    );
}
