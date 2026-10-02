// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's simulations: every
//! `slimemold` kernel links on the real GPU, arrivals are counted exactly, the field keeps
//! every deposit less the decay, one seed and one clock make one world to the last bit, a
//! world that changes size keeps its pattern and never shows an empty picture, an Output
//! samples the picture the same tick drew, a tick whose kernels are still linking waits rather
//! than being skipped, and a closed project's world is born again rather than carried on.
//!
//! Here the worlds are published by hand, as the passes the node's CPU half would plan; the
//! app-level tests that tick that half through an `App` are `tests/gpu_app.rs`.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, module, rgba_of, tick};
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::compile::wgsl::Sampler;
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::nodes::slimemold::{
    CLEAR, DIFFUSE, FADE, INIT, KERNELS, MOTOR, PARAMS, PEAK, PICTURE, SCALE, SENSE, SETTLE, THROW,
};
use supersilvia::nodes::{Domain, Kernel, Pass, Simulation, TextureFilter, TextureWrap};
use supersilvia::render::{
    FrameJob, OutputJob, OutputMode, Renderer, SimJob, SimReadback, UniformValue,
};

/// How long a test waits for kernels to link beside six other lanes on one iGPU.
const LINK_WAIT: Duration = Duration::from_secs(30);

fn port() -> PortRef {
    PortRef::new(NodeId(1), "trail")
}

/// The numbers `slimemold`'s kernels read, at its defaults, with the decay and the jitter the
/// test asks for.
fn mold_params(decay: f32, jitter: f32) -> Vec<(&'static str, f32)> {
    let degrees = std::f32::consts::PI / 180.0;
    let values = [
        19.0 * degrees,
        151.8 * degrees,
        22.0,
        decay,
        jitter,
        0.0,
        0.2,
        0.3,
        0.02,
    ];
    PARAMS.into_iter().zip(values).collect()
}

fn world(size: u32, agents: u32, decay: f32, passes: Vec<Pass>) -> Simulation {
    Simulation {
        size,
        agents,
        params: mold_params(decay, 0.02),
        passes,
        steps: 0,
    }
}

/// A world's first moment, as the node's CPU half plans it: a starting maximum, a clean field
/// and every agent thrown.
fn birth(agents: u32, seed: u32) -> Vec<Pass> {
    vec![
        Pass::of(&INIT).with_arg(20.0),
        Pass::of(&CLEAR),
        Pass::of(&THROW).over(0, agents, 1).seeded(seed),
    ]
}

/// `n` steps of the full rules, seeded from `seed` up.
fn steps(n: u32, seed: u32) -> Vec<Pass> {
    (0..n)
        .flat_map(|k| {
            [
                Pass::of(&MOTOR).seeded(seed + 2 * k),
                Pass::of(&SENSE).seeded(seed + 2 * k + 1),
                Pass::of(&DIFFUSE),
            ]
        })
        .collect()
}

/// What every tick of the node ends with: the peak, the running maximum, the picture.
fn drawn(mut passes: Vec<Pass>) -> Vec<Pass> {
    passes.extend([Pass::of(&PEAK), Pass::of(&SETTLE), Pass::of(&PICTURE)]);
    passes
}

fn sim_job(sim: Simulation) -> SimJob {
    SimJob {
        port: port(),
        sim,
        wrap: TextureWrap::Repeat,
        filter: TextureFilter::Linear,
    }
}

fn sims_only(sim: Simulation) -> FrameJob {
    FrameJob {
        sims: vec![sim_job(sim)],
        ..FrameJob::default()
    }
}

/// The same shape with nothing to do: a tick that changes nothing.
fn idle(sim: &Simulation) -> Simulation {
    Simulation {
        passes: Vec::new(),
        ..sim.clone()
    }
}

/// Draw until every tick the world was handed has run, drawing `then` meanwhile — its kernels
/// link off the thread, and a tick waits for them — and read it.
fn settle(renderer: &mut Renderer, then: &dyn Fn() -> FrameJob) -> SimReadback {
    let deadline = Instant::now() + LINK_WAIT;
    loop {
        let read = renderer.read_simulation(port()).expect("a world");
        if read.queued == 0 {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "the kernels never linked: {:?}",
            renderer.errors
        );
        std::thread::sleep(Duration::from_millis(1));
        renderer.draw(&then());
    }
}

/// Draw one tick holding one world, and go on drawing it with no new work until it has run.
fn step_world(renderer: &mut Renderer, sim: Simulation) -> SimReadback {
    let rest = idle(&sim);
    renderer.draw(&sims_only(sim));
    settle(renderer, &|| sims_only(rest.clone()))
}

fn renderer() -> Renderer {
    Renderer::new(gpu::gpu()).expect("renderer")
}

fn texels(picture: &[u8]) -> &[[u8; 4]] {
    picture.as_chunks::<4>().0
}

/// Every kernel `slimemold` steps its world with links as a compute pipeline after the
/// renderer's prelude, on the real GPU, and one tick naming them all runs with no error.
#[test]
fn every_simulation_kernel_links_on_the_gpu() {
    let mut renderer = renderer();
    let passes = KERNELS
        .iter()
        .map(|k| Pass::of(k).over(0, 16, 1).with_arg(1.0))
        .collect();
    step_world(&mut renderer, world(32, 64, 0.05, passes));
    assert_eq!(renderer.errors.get(&port().node), None);
}

/// **Arrivals are counted exactly.** A hundred agents standing on one spot and facing one way
/// all land on one cell in one step, and the count there is a hundred — an atomic that lost
/// one would say ninety-nine. The diffusion then spreads their deposit over the nine cells
/// around it, and with no decay the field holds all of it.
#[test]
fn every_agent_that_lands_on_one_cell_is_counted_and_its_scent_is_kept() {
    /// Every agent at (5.5, 7.5), facing along +x.
    static GATHER: Kernel = Kernel {
        name: "gather",
        over: Domain::Agents,
        wgsl_common: "",
        wgsl: "fn run_agent(i: i32) { agents[i] = vec4f(5.5, 7.5, 0.0, 0.0); }",
        flips: false,
    };

    let mut renderer = renderer();
    let (size, agents) = (16u32, 100u32);
    let at = |passes| Simulation {
        params: mold_params(0.0, 0.0),
        ..world(size, agents, 0.0, passes)
    };

    let landed = step_world(
        &mut renderer,
        at(vec![
            Pass::of(&INIT).with_arg(20.0),
            Pass::of(&CLEAR),
            Pass::of(&GATHER),
            Pass::of(&MOTOR).seeded(1),
        ]),
    );
    let cell = |x: usize, y: usize| y * size as usize + x;
    assert_eq!(
        landed.arrivals[cell(6, 7)],
        agents,
        "one step east of (5.5, 7.5) is cell (6, 7), and every agent is counted there"
    );
    assert_eq!(
        landed.arrivals.iter().sum::<u32>(),
        agents,
        "and nowhere else"
    );

    let spread = step_world(&mut renderer, at(vec![Pass::of(&DIFFUSE)]));
    let deposit = 5.0 * agents as f32;
    for (x, y) in (5..=7).flat_map(|x| (6..=8).map(move |y| (x, y))) {
        assert!(
            (spread.field[cell(x, y)] - deposit / 9.0).abs() < 1e-3,
            "({x}, {y}) holds {} of {deposit}",
            spread.field[cell(x, y)]
        );
    }
    let total: f32 = spread.field.iter().sum();
    assert!((total - deposit).abs() < 1e-2, "{total} of {deposit}");
    assert_eq!(
        spread.arrivals.iter().sum::<u32>(),
        0,
        "and the next step's counts start at nothing"
    );
    assert_eq!(
        spread.arrivals_next[cell(6, 7)],
        agents,
        "while this step's stay behind as where the agents stand"
    );
}

/// **The field keeps what is deposited in it, less the decay.** A world scattered at random
/// steps under the full rules; with no decay the field grows by exactly one deposit per agent
/// per step, and with a decay each step keeps `1 − decay` of what it held and what landed.
#[test]
fn a_step_deposits_one_scent_per_agent_and_the_decay_takes_its_share() {
    let mut renderer = renderer();
    let (size, agents) = (64u32, 819u32);

    let mut passes = birth(agents, 7);
    passes.extend(steps(10, 100));
    let kept = step_world(&mut renderer, world(size, agents, 0.0, passes));
    let total: f64 = kept.field.iter().map(|v| f64::from(*v)).sum();
    let expected = 5.0 * f64::from(agents) * 10.0;
    assert!(
        (total - expected).abs() / expected < 1e-4,
        "ten steps of no decay hold {total}, not {expected}"
    );

    let decay = 0.05;
    let faded = step_world(&mut renderer, world(size, agents, decay, steps(10, 200)));
    let mut expected = total;
    for _ in 0..10 {
        expected = (expected + 5.0 * f64::from(agents)) * (1.0 - f64::from(decay));
    }
    let total: f64 = faded.field.iter().map(|v| f64::from(*v)).sum();
    assert!(
        (total - expected).abs() / expected < 1e-4,
        "ten steps at a decay of {decay} hold {total}, not {expected}"
    );
}

/// Silvia's default world: 192 on a side, a fifth of it alive, born and stepped over `ticks`
/// ticks of `per_tick` steps each, every tick drawn.
fn default_world(renderer: &mut Renderer, ticks: u32, per_tick: u32) -> SimReadback {
    let (size, agents) = (192u32, 7372u32);
    let mut last = None;
    for t in 0..ticks {
        let mut passes = if t == 0 {
            birth(agents, 11)
        } else {
            Vec::new()
        };
        passes.extend(steps(per_tick, 1000 + 2 * per_tick * t));
        let sim = world(size, agents, 0.05, drawn(passes));
        renderer.draw(&sims_only(sim.clone()));
        last = Some(sim);
    }
    let rest = idle(&last.expect("a tick"));
    settle(renderer, &|| sims_only(rest.clone()))
}

/// **The same seed and the same clock make the same world.** Two renderers, each on a device
/// of its own, handed the same ticks hold the same agents, the same field, the same state and
/// the same picture to the last bit — which is what lets an offline render, stepping virtual
/// time at whatever speed the GPU manages, make the world live play made.
#[test]
fn a_slime_mold_is_the_same_world_twice_for_one_seed_and_one_clock() {
    let first = default_world(&mut renderer(), 12, 3);
    let second = default_world(&mut renderer(), 12, 3);

    assert_eq!(first.size, 192, "silvia's default grid");
    assert_eq!(first.agents.len(), 7372, "and a fifth of it alive");
    assert!(
        first.field.iter().any(|v| *v > 0.0),
        "the agents left scent"
    );
    assert!(
        texels(&first.picture).iter().any(|p| p[0] > 0),
        "and the picture shows it"
    );
    assert!(
        first.agents == second.agents,
        "the agents went somewhere else the second time"
    );
    assert!(first.field == second.field, "the field differs");
    assert_eq!(first.state, second.state, "the running maximum differs");
    assert_eq!(first.picture, second.picture, "the picture differs");
}

/// **A tick whose kernels are still linking waits; it is never skipped.** Ticks published one
/// after another from the very first draw, before anything can have linked, step the world
/// exactly as one tick holding all of their passes does.
#[test]
fn ticks_published_while_the_kernels_link_all_run_in_order() {
    let (size, agents) = (64u32, 819u32);
    let mut early = renderer();
    let mut ticks = vec![birth(agents, 3)];
    ticks.extend((0..8).map(|t| drawn(steps(2, 50 + 4 * t))));
    for passes in &ticks {
        early.draw(&sims_only(world(size, agents, 0.05, passes.clone())));
    }
    let rest = world(size, agents, 0.05, Vec::new());
    let staggered = settle(&mut early, &|| sims_only(rest.clone()));

    let all: Vec<Pass> = ticks.concat();
    let whole = step_world(&mut renderer(), world(size, agents, 0.05, all));
    assert!(staggered.agents == whole.agents, "the agents differ");
    assert!(staggered.field == whole.field, "the field differs");
    assert_eq!(staggered.picture, whole.picture, "the picture differs");
}

/// **Clear is silvia's**: the scent and the stamp of where anyone stands go, the agents stay
/// where they are, and the picture says so on the tick it was pressed. Before it, the picture
/// is a fraction of the field's own maximum and never a saturated sheet.
#[test]
fn clear_empties_the_field_and_leaves_the_agents() {
    let mut renderer = renderer();
    let before = default_world(&mut renderer, 20, 3);
    let pixels = texels(&before.picture);
    assert!(pixels.iter().any(|p| p[0] > 0), "the field never got wet");
    let full = pixels.iter().filter(|p| p[0] == 255).count();
    assert!(full < pixels.len() / 2, "{full} cells are saturated");
    assert!(
        pixels.iter().any(|p| p[3] > 0),
        "somebody is standing somewhere"
    );

    let after = step_world(
        &mut renderer,
        world(192, 7372, 0.05, drawn(vec![Pass::of(&CLEAR)])),
    );
    assert!(after.field.iter().all(|v| *v == 0.0), "scent survived");
    assert!(
        texels(&after.picture)
            .iter()
            .all(|p| p[0] == 0 && p[3] == 0),
        "the picture still shows scent or somebody standing"
    );
    assert_eq!(after.agents, before.agents, "the agents went with it");
}

/// **Randomize knocks the scent back.** A fade by nine tenths leaves every cell nine tenths
/// of what it held, which is the first half of silvia's nudge.
#[test]
fn a_fade_keeps_its_share_of_every_cell() {
    let mut renderer = renderer();
    let mut passes = birth(819, 5);
    passes.extend(steps(6, 20));
    let before = step_world(&mut renderer, world(64, 819, 0.05, passes));
    let after = step_world(
        &mut renderer,
        world(64, 819, 0.05, vec![Pass::of(&FADE).with_arg(0.9)]),
    );
    for (a, b) in after.field.iter().zip(&before.field) {
        assert!((a - b * 0.9).abs() <= b.abs() * 1e-6, "{b} faded to {a}");
    }
}

/// **A new grid reallocates the world and keeps its pattern, with no frame of nothing.** A
/// world of 64 halved to 32 is a new field resampled nearest from the old one — each new cell
/// the old cell its center falls in — counts that start at zero, and a new picture that is the
/// old one scaled in on the very tick, before any pass has drawn it; a smaller population keeps
/// the first of its agents where they stood, and the node's `scale` carries them onto the new
/// grid. The picture a viewer still holds is the old one, untouched. Grown back, each cell is
/// the one it came from.
#[test]
fn a_new_grid_reallocates_the_world_and_keeps_its_pattern() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let mut passes = birth(819, 9);
    passes.extend(steps(20, 300));
    let before = step_world(&mut renderer, world(64, 819, 0.05, drawn(passes)));
    let held = renderer.publish();
    let old = held.sources.get(&port()).expect("published").clone();
    assert!(
        texels(&before.picture).iter().any(|p| p[0] > 0),
        "a picture to keep"
    );

    // The shape alone: nothing runs but the reallocation.
    let halved = step_world(&mut renderer, world(32, 204, 0.05, Vec::new()));
    assert_eq!(halved.size, 32);
    assert_eq!(halved.field.len(), 32 * 32);
    for (x, y) in (0..32usize).flat_map(|x| (0..32usize).map(move |y| (x, y))) {
        assert_eq!(
            halved.field[y * 32 + x],
            before.field[(2 * y + 1) * 64 + 2 * x + 1],
            "cell ({x}, {y}) is not the old cell its center fell in"
        );
    }
    assert!(
        halved
            .arrivals
            .iter()
            .chain(&halved.arrivals_next)
            .all(|a| *a == 0)
    );
    assert_eq!(halved.state, before.state, "the state is kept");
    assert_eq!(halved.agents[..], before.agents[..204], "the first of them");
    // A linear scale by one half samples between four texels: their mean, to within a unit.
    let old_px = texels(&before.picture);
    let mut lit = 0;
    for (x, y) in (0..32usize).flat_map(|x| (0..32usize).map(move |y| (x, y))) {
        let got = texels(&halved.picture)[y * 32 + x];
        for ch in 0..4 {
            let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| u32::from(old_px[(2 * y + dy) * 64 + 2 * x + dx][ch]))
                .sum();
            let mean = sum as f32 / 4.0;
            assert!(
                (f32::from(got[ch]) - mean).abs() <= 1.0,
                "({x}, {y}) channel {ch}: {} where the old picture's mean is {mean}",
                got[ch]
            );
        }
        lit += usize::from(got[0] > 0);
    }
    assert!(lit > 0, "the scaled picture is empty");
    assert_eq!(
        gpu::bytes_of(&gpu, old.texture.texture()),
        before.picture,
        "a viewer holding the old picture still sees it whole"
    );
    assert_eq!((old.width, old.height), (64, 64));

    // The node's `scale` carries the agents onto the grid half the size.
    let scaled = step_world(
        &mut renderer,
        world(
            32,
            204,
            0.05,
            vec![Pass::of(&SCALE).over(0, 204, 1).with_arg(0.5)],
        ),
    );
    for (a, b) in scaled.agents.iter().zip(&before.agents) {
        assert_eq!(
            [a[0], a[1], a[2]],
            [b[0] * 0.5, b[1] * 0.5, b[2]],
            "an agent stayed where it stood on the old grid"
        );
    }

    let grown = step_world(&mut renderer, world(64, 204, 0.05, Vec::new()));
    for (x, y) in (0..64usize).flat_map(|x| (0..64usize).map(move |y| (x, y))) {
        assert_eq!(
            grown.field[y * 64 + x],
            halved.field[(y / 2) * 32 + x / 2],
            "grown cell ({x}, {y}) is not the cell it came from"
        );
    }
    let published = renderer.publish();
    let picture = published.sources.get(&port()).expect("published");
    assert_eq!((picture.width, picture.height), (64, 64));
}

/// **An Output samples the world the tick left.** An Output reading the simulation's picture
/// texel for texel, drawn in the same tick as passes that change it, holds exactly the picture
/// those passes drew — the simulation runs in the prelude, before any Output — and what the
/// renderer publishes for the port is that picture, rows top first, through the sampler its
/// output declared.
#[test]
fn an_output_samples_the_picture_the_simulation_drew() {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let out = NodeId(2);
    let size = 64u32;
    let shader = module(
        &[],
        &[("t", port().node)],
        "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return textureSampleLevel(t, sampler_repeat_nearest, frag_coord.xy / u.u_resolution, 0.0);
}
",
    );
    let values = vec![(Arc::<str>::from("t"), UniformValue::NodeTexture(port()))];
    let output = |send: bool, mode| -> Vec<OutputJob> {
        vec![job(out, (size, size), &shader, send, mode, values.clone())]
    };
    let both = |mode: OutputMode, sim: Simulation| FrameJob {
        outputs: output(false, mode),
        sims: vec![sim_job(sim)],
        ..FrameJob::default()
    };
    let mut first = birth(819, 13);
    first.extend(steps(10, 400));
    let first = world(size, 819, 0.05, drawn(first));
    // Sent on a drawing tick, since a suspended Output takes no module.
    renderer.draw(&FrameJob {
        outputs: output(true, OutputMode::Draw),
        sims: vec![sim_job(first.clone())],
        ..FrameJob::default()
    });
    let rest = idle(&first);
    let before = settle(&mut renderer, &|| both(OutputMode::Suspended, rest.clone()));
    let deadline = Instant::now() + LINK_WAIT;
    while renderer.awaiting_shader(out) || renderer.is_linking(out) {
        assert!(Instant::now() < deadline, "the Output never linked");
        renderer.draw(&both(OutputMode::Suspended, rest.clone()));
    }

    renderer.draw(&both(
        OutputMode::Draw,
        world(size, 819, 0.05, drawn(steps(4, 500))),
    ));
    drain(&gpu);
    let now = renderer.read_simulation(port()).expect("a world");
    assert_eq!(
        now.queued, 0,
        "linked kernels run on the tick they are handed"
    );
    assert_ne!(now.picture, before.picture, "the tick moved the world");
    let frame = rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"));
    assert_eq!(
        frame,
        texels(&now.picture).to_vec(),
        "the Output holds the picture this tick drew, row for row"
    );
    assert!(frame.iter().any(|p| p[0] > 0), "and it is not empty");

    let published = renderer.publish();
    let picture = published.sources.get(&port()).expect("the picture");
    assert!(picture.flip, "row zero of the world is the first row");
    assert_eq!(picture.sampler, Sampler::RepeatLinear);
    assert_eq!((picture.width, picture.height), (size, size));
    assert_eq!(
        gpu::bytes_of(&gpu, picture.texture.texture()),
        now.picture,
        "what a viewer is shown"
    );
}

/// **A kernel that does not compile is its node's status line**, and the world's queue is
/// emptied rather than waiting for it forever; the other kernels are untouched.
#[test]
fn a_kernel_that_fails_to_link_is_its_nodes_error() {
    static BROKEN: Kernel = Kernel {
        name: "broken",
        over: Domain::Once,
        wgsl_common: "",
        wgsl: "fn run_once() { this is not wgsl }",
        flips: false,
    };
    let mut renderer = renderer();
    let sim = world(16, 16, 0.05, vec![Pass::of(&INIT), Pass::of(&BROKEN)]);
    renderer.draw(&sims_only(sim.clone()));
    let deadline = Instant::now() + LINK_WAIT;
    let error = loop {
        if let Some(error) = renderer.errors.get(&port().node) {
            break error.clone();
        }
        assert!(Instant::now() < deadline, "the failure never landed");
        std::thread::sleep(Duration::from_millis(1));
        renderer.draw(&sims_only(idle(&sim)));
    };
    assert!(error.contains("broken"), "{error}");
    assert_eq!(
        renderer.read_simulation(port()).expect("a world").queued,
        0,
        "nothing is left waiting for it"
    );
    step_world(&mut renderer, world(16, 16, 0.05, birth(16, 1)));
    assert_eq!(
        renderer.errors.get(&port().node),
        None,
        "and a tick that does not name it runs"
    );
}

/// **A closed project's world is not carried into the next.** After `forget_project` the
/// port has no world and no picture, and the same ticks handed to it again make exactly the
/// world a fresh renderer makes — born again, not carried on.
#[test]
fn a_forgotten_world_is_born_again() {
    let mut renderer = renderer();
    let mut passes = birth(819, 21);
    passes.extend(steps(8, 600));
    let sim = world(64, 819, 0.05, drawn(passes));
    step_world(&mut renderer, world(64, 819, 0.05, drawn(steps(5, 900))));
    renderer.forget_project();
    assert!(renderer.read_simulation(port()).is_none());
    assert!(renderer.texture_of(port().node).is_none());
    assert!(!renderer.publish().sources.contains_key(&port()));

    let reborn = step_world(&mut renderer, sim.clone());
    let fresh = step_world(&mut self::renderer(), sim);
    assert!(reborn.agents == fresh.agents, "the agents were carried on");
    assert!(reborn.field == fresh.field, "the field was carried on");
    assert_eq!(reborn.picture, fresh.picture, "and so was the picture");
}

/// A world is dropped with the job that named it: a tick without it frees it.
#[test]
fn a_world_no_job_names_is_let_go() {
    let mut renderer = renderer();
    step_world(&mut renderer, world(16, 16, 0.05, birth(16, 1)));
    assert!(renderer.texture_of(port().node).is_some());
    renderer.draw(&tick(0.0, Vec::new()));
    assert!(renderer.read_simulation(port()).is_none());
    assert!(!renderer.publish().sources.contains_key(&port()));
}
