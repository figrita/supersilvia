// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: `CpuNode::reset` puts a node where `CpuDef::create` made it.
//!
//! A render starts every node from a known state, and the only way to know it is the same
//! state a fresh instance would be in. So for every node with a CPU half: run it for a
//! while, reset it, run it again from a fresh clock, and hold what it publishes on every frame
//! — each uniform, each picture, each edge on an action output, its playhead — equal to what a
//! new instance publishes over the same frames. A render's start is a seek, and a seek is a
//! jump that re-births a gear on its own, so each gear is put in a state a jump keeps before
//! the reset: Hold pressed. A node that only moves when something drives it is driven, by a
//! fast Master Gear on its Time or its trigger, so a sequencer is not two empty lists. The
//! nodes that read the world are the exception: `cargo test` opens no camera and no
//! microphone, and their reset keeps the device, while `clock` is two different instants on
//! two runs, which is the whole of what the node is.

use emath::Pos2;
use supersilvia::clock::{Stepper, Warmup};
use supersilvia::graph::{ControlValue, NodeId, PortRef, PortType, Value};
use supersilvia::nodes::{self, OutputKind};
use supersilvia::{App, Command};

/// Nodes whose tick reads the world rather than the graph.
///
/// `cargo test` opens no camera and no microphone, and it puts no screen picker on somebody's
/// desktop either — `screencapture` asks the portal on the first tick, which is the node being
/// opened. `text` is here for a different reason: its picture is drawn by a GStreamer
/// pipeline on GStreamer's own threads, so whether the letters have arrived by the twentieth
/// tick is the wall clock's answer and not the node's. `clock` reads the wall clock, so two
/// runs are two different instants — the whole of what the node is.
const READS_THE_WORLD: &[&str] = &["camera", "audioin", "screencapture", "text", "clock"];

/// The nodes that hold a Hold, pressed during the first run.
const GEARS: &[&str] = &["mastergear"];

/// Inputs a driving Master Gear is cabled into, by node, beside every node's Time: an action
/// input takes its Trigger, a number its Cycles.
const DRIVEN: &[(&str, &str)] = &[
    ("counter", "increment"),
    ("smoothcounter", "increment"),
    ("clockdivider", "input"),
    ("slew", "input"),
];

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// `slug`, and the Master Gear driving it where it has something to drive: a quarter of a
/// second a cycle, so twenty frames at 30 fps see beats and a sequencer's steps. A Step
/// Sequencer has cells lit in every lane.
fn build(app: &mut App, def: &'static nodes::NodeDef) -> NodeId {
    let id = add(app, def.slug);
    if def.slug == "stepsequencer" {
        let lanes = [
            "x...x...x...x...",
            "..x...x...x...x.",
            "x..x..x..x..x..x",
            "xxxx....xxxx....",
        ];
        app.apply(Command::SetValue {
            node: id,
            key: "pattern",
            value: Value::Cells(lanes.iter().map(ToString::to_string).collect()),
        })
        .unwrap();
    }
    let driven: Vec<&nodes::InputDef> = def
        .inputs
        .iter()
        .filter(|input| {
            (input.key == nodes::TIME && input.ty == PortType::UniformNumber)
                || DRIVEN.contains(&(def.slug, input.key))
        })
        .collect();
    if driven.is_empty() {
        return id;
    }
    let driver = add(app, "mastergear");
    app.apply(Command::SetControl {
        node: driver,
        key: "length",
        value: ControlValue::Float(0.25),
    })
    .unwrap();
    // On a gear, the node loops rather than running free.
    if def.timing.is_some() {
        app.apply(Command::SetOption {
            node: id,
            key: "clockMode",
            value: "loop".to_string(),
        })
        .unwrap();
    }
    for input in driven {
        let from = if input.ty == PortType::Action {
            "trigger"
        } else {
            "cycles"
        };
        app.apply(Command::Connect {
            from: PortRef::new(driver, from),
            to: PortRef::new(id, input.key),
        })
        .unwrap();
    }
    id
}

/// Tick `frames` frames on a fresh stepper, the way a render does: a seek to its first
/// frame, then the transport driven to each frame's own time. A gear's Hold is pressed on the
/// tenth frame when `hold` says so. What the node published on each frame comes back.
fn run(
    app: &mut App,
    id: NodeId,
    def: &nodes::NodeDef,
    frames: u32,
    hold: bool,
) -> Vec<Vec<(String, String)>> {
    let stepper = Stepper::new(30.0, frames, Warmup::Black);
    app.transport(supersilvia::transport::Command::Seek(stepper.time_of(0)));
    let button = PortRef::new(id, "hold");
    (0..stepper.len())
        .map(|i| {
            if hold && GEARS.contains(&def.slug) {
                if i == 10 {
                    app.press(button, true);
                } else if i == 11 {
                    app.press(button, false);
                }
            }
            app.tick_at(stepper.time_of(i));
            published(app, id, def)
        })
        .collect()
}

/// Everything the node published on the last frame: each uniform output — a number or a
/// color — each texture's pixels, each action output's edges and where they fell, and the
/// playhead it reports.
fn published(app: &App, id: NodeId, def: &nodes::NodeDef) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = def
        .outputs
        .iter()
        .filter_map(|out| {
            let port = PortRef::new(id, out.key);
            let value = match out.kind {
                OutputKind::Uniform if out.ty == PortType::UniformColor => {
                    format!("{:?}", app.uniform_color(port))
                }
                OutputKind::Uniform => format!("{:?}", app.uniform(port)),
                // A world stepped on the GPU is what its tick described: its shape, its numbers
                // and the steps it has taken. `tests/gpu_sims.rs` and
                // `tests/gpu_app.rs` hold the GPU's half.
                OutputKind::Texture if app.simulation(port).is_some() => {
                    let sim = app.simulation(port).unwrap();
                    format!("{:?}", (sim.size, sim.agents, sim.steps, &sim.params))
                }
                // A digest rather than the pixels, so a failure prints a line and not a grid.
                OutputKind::Texture => format!(
                    "{:?}",
                    app.frame(port).map(|f| {
                        let bytes = f.bytes().unwrap_or_default();
                        (
                            f.width,
                            f.height,
                            bytes.iter().map(|&b| u64::from(b)).sum::<u64>(),
                        )
                    })
                ),
                _ if out.ty == PortType::Action => format!("{:?}", app.edges(port)),
                _ => return None,
            };
            Some((out.key.to_string(), value))
        })
        .collect();
    out.push((
        "playhead".to_string(),
        format!("{:?}", app.snapshot().playheads.get(&id)),
    ));
    out
}

/// The first frame two trails differ on, and what each published there.
fn first_difference(
    a: &[Vec<(String, String)>],
    b: &[Vec<(String, String)>],
) -> Option<(usize, String)> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .map(|i| (i, format!("{:?}\n   vs {:?}", a[i], b[i])))
}

#[test]
fn a_reset_node_publishes_what_a_fresh_one_does() {
    for def in nodes::REGISTRY {
        if def.cpu.is_none() || READS_THE_WORLD.contains(&def.slug) {
            continue;
        }
        let mut fresh = App::headless();
        let id = build(&mut fresh, def);
        let expected = run(&mut fresh, id, def, 20, false);

        let mut reset = App::headless();
        let same = build(&mut reset, def);
        assert_eq!(same, id);
        run(&mut reset, id, def, 37, true);
        reset.reset_cpu();
        let after = run(&mut reset, id, def, 20, false);
        if let Some((frame, what)) = first_difference(&after, &expected) {
            panic!(
                "{}: after 37 frames and a reset, frame {frame} of 20 more differs from a fresh \
                 one's:\n   {what}",
                def.slug
            );
        }
    }
}

/// The reset test would pass vacuously on a node whose reset changes nothing, so hold that
/// for nodes with state — the gears held, an integrator, a counter, the slime mold — the same
/// run with no reset is not what a fresh node publishes, and that a driven sequencer fires.
#[test]
fn a_stateful_node_is_somewhere_else_without_its_reset() {
    for slug in GEARS
        .iter()
        .chain(&["slew", "counter", "smoothcounter", "slimemold"])
    {
        let def = nodes::find(slug).unwrap();
        let mut fresh = App::headless();
        let id = build(&mut fresh, def);
        let expected = run(&mut fresh, id, def, 20, false);

        let mut kept = App::headless();
        build(&mut kept, def);
        run(&mut kept, id, def, 37, true);
        let after = run(&mut kept, id, def, 20, false);
        assert!(
            first_difference(&after, &expected).is_some(),
            "{slug}: with no reset, 20 frames after 37 are what a fresh node publishes, so the \
             reset test cannot tell its reset from none"
        );
    }
    for slug in ["stepsequencer", "euclideanrhythm", "clockdivider"] {
        let def = nodes::find(slug).unwrap();
        let mut app = App::headless();
        let id = build(&mut app, def);
        let trail = run(&mut app, id, def, 20, false);
        assert!(
            trail
                .iter()
                .flatten()
                .any(|(key, edges)| key != "playhead" && edges.contains("Down")),
            "{slug}: driven for 20 frames, it fires nothing"
        );
    }
}
