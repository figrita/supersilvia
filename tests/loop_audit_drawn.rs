// SPDX-License-Identifier: AGPL-3.0-or-later

//! The loop claims of every node that draws and moves with time, held on the GPU.
//!
//! A node's `Timing::period` says after how many of its own cycles its picture comes back, or
//! `None` for never. Two promises follow from it, and both are held here for every option a
//! node offers and across the range of every control it has:
//!
//! - **No false claim.** With a period `P`, the frame at Time `t + P` is the frame at `t`, at
//!   any `t` — zero, a fraction, a thousand cycles on, behind zero, across the 40320 the
//!   count's whole part wraps at — and with any Offset.
//! - **No early loop.** The frame does not come back before `P`: at `t + d` for every whole
//!   `d` under `P`, `P / k` for `k` up to 12, and a few fractions, it is visibly another
//!   picture. A setting where the node stands still — every wave at zero amplitude, a Mandelbrot
//!   zoomed into the set — is exempt, and printed. A picture that never comes back must not
//!   come back at a small whole number of cycles either.
//!
//! **The pictures.** A transform reads a picture of the coordinate itself — red is `x`, green
//! is `y` — which has no symmetry at all, so a frame that comes back means the transform's
//! map came back and not that a symmetric source hid a difference. The Cosine Gradient's
//! number is `x`; a field output is drawn into all three channels. Every frame is 48 pixels
//! square, read back as floats.
//!
//! **Same, and come back.** Two frames are the same where no more than 0.2% of their pixels
//! differ by over two levels in any channel: what a claim is held to. A frame has come back
//! early where it is the same frame and its largest difference from the start is under 2% of
//! the largest the node moves by across its period, or under half a level: a picture that is
//! still moving, however slowly, has not looped, and one whose every pixel is where it
//! started has — from two starts, since a picture that is one number's function passes back
//! through a value it held without having looped. Every true early loop found here is under
//! 0.5% of its motion; the nearest return that is not a loop is printed with each node.
//!
//! **What is swept.** Each Code option set is compiled and linked once. A node's controls are
//! uniforms, so each setting is written into the uniforms: the defaults; each control at its
//! two ends, at zero where its range crosses it and 37% of the way along; three random
//! settings moving every control at once; and settings a node's own WGSL makes worth naming.
//! Frames are drawn sixteen to a tick, one Output each, with the Time count of each written as
//! the synth writes it (`phasor::split`).

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, link_all, tick};
use std::fmt::Write as _;
use std::sync::Arc;
use supersilvia::compile::wgsl;
use supersilvia::compile::{Shader, UniformProvider, UniformType};
use supersilvia::graph::{ControlValue, Graph, NodeId, PortRef};
use supersilvia::nodes::{self, timing};
use supersilvia::render::{Gpu, OutputMode, Renderer, UniformValue};

/// The side of every frame, in pixels.
const SIZE: u32 = 48;

/// How many frames one tick draws, each on an Output of its own.
const SLOTS: usize = 16;

/// The `k`th Output a tick draws into: ids no graph here reaches.
fn slot(k: usize) -> NodeId {
    NodeId(1_000_000 + k as u32)
}

/// A difference of more than this, in any channel, is a pixel that differs: two levels.
const LEVEL: f32 = 2.0 / 255.0;

/// Two frames are the same with no more than this share of their pixels differing.
const SAME: f64 = 0.002;

/// A frame has come back where it is the same frame and its largest difference from the start
/// is under this share of how far the node moves at all — the largest difference across four
/// steps through its period —
const BACK_SHARE: f32 = 0.02;

/// — or under this, half a level, whatever the motion: a hair above a half float's rounding
/// at the values these pictures hold.
const BACK_FLOOR: f32 = 0.002;

// --------------------------------------------------------------------------- the harness

/// One moment to draw: a Time count per axis and an Offset per axis, in the node's cycles.
#[derive(Clone, Copy)]
struct Probe {
    time: [f64; 2],
    offset: [f32; 2],
}

/// One graph's module linked once, drawn at any Time and Offset.
struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    shader: Arc<Shader>,
    under: NodeId,
    g: Graph,
}

impl Rig {
    fn new(g: Graph, out: NodeId, under: NodeId) -> Self {
        let shader = wgsl::build(&g, out).expect("the Output is connected");
        assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
        let shader = Arc::new(shader);
        let gpu = gpu::gpu();
        let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
        let rest = Probe {
            time: [0.0; 2],
            offset: [0.0; 2],
        };
        let values = values(&g, &shader, under, rest);
        let outputs = |send: bool, mode| {
            (0..SLOTS)
                .map(|k| job(slot(k), (SIZE, SIZE), &shader, send, mode, values.clone()))
                .collect()
        };
        link_all(&mut renderer, &outputs);
        Self {
            gpu,
            renderer,
            shader,
            under,
            g,
        }
    }

    /// Whether the module reads the node under test's `key` as a uniform, so that writing the
    /// control changes the picture without a rebuild.
    fn reads(&self, key: &str) -> bool {
        self.shader.uniforms.values().any(|p| {
            matches!(p, UniformProvider::Control { node, key: k, .. }
                if *node == self.under && *k == key)
        })
    }

    fn set(&mut self, key: &'static str, value: f32) {
        self.g
            .get_mut(self.under)
            .expect("in the graph")
            .controls
            .insert(key, ControlValue::Float(value));
    }

    /// Every probe's frame as RGBA floats, rows bottom first, in order.
    fn frames(&mut self, probes: &[Probe]) -> Vec<Vec<f32>> {
        let mut all = Vec::with_capacity(probes.len());
        for chunk in probes.chunks(SLOTS) {
            // Every slot is drawn every tick, or the renderer drops the ones a tick leaves out.
            let jobs = (0..SLOTS)
                .map(|k| {
                    let probe = chunk[k.min(chunk.len() - 1)];
                    let values = values(&self.g, &self.shader, self.under, probe);
                    job(
                        slot(k),
                        (SIZE, SIZE),
                        &self.shader,
                        false,
                        OutputMode::Draw,
                        values,
                    )
                })
                .collect();
            self.renderer.draw(&tick(0.0, jobs));
            drain(&self.gpu);
            for k in 0..chunk.len() {
                let texture = self.renderer.texture_of(slot(k)).expect("drawn");
                all.push(gpu::floats_of(&self.gpu, &texture));
            }
        }
        all
    }
}

/// Every uniform of `shader` as the synth resolves it, with the node under test's Time counts
/// and Offsets from `probe`.
fn values(
    g: &Graph,
    shader: &Shader,
    under: NodeId,
    probe: Probe,
) -> Vec<(Arc<str>, UniformValue)> {
    shader
        .uniforms
        .iter()
        .filter_map(|(name, provider)| {
            let value = match provider {
                UniformProvider::Control { node, key, .. }
                    if *node == under && *key == timing::OFFSET =>
                {
                    UniformValue::Float(probe.offset[0])
                }
                UniformProvider::Control { node, key, .. }
                    if *node == under && *key == timing::OFFSET_Y =>
                {
                    UniformValue::Float(probe.offset[1])
                }
                UniformProvider::Control { node, key, .. } => {
                    match g.get(*node)?.controls.get(key)? {
                        ControlValue::Float(v) => UniformValue::Float(*v),
                        ControlValue::Color(v) => UniformValue::Vec4(nodes::alpha::premultiply(*v)),
                    }
                }
                UniformProvider::NodeTexture { node, port } => {
                    UniformValue::NodeTexture(PortRef::new(*node, port))
                }
                UniformProvider::NodeUniform { ty, .. } => match ty {
                    UniformType::Vec4 => UniformValue::Vec4([0.0; 4]),
                    _ => UniformValue::Float(0.0),
                },
                UniformProvider::NodeCount { node, port } => {
                    let count = if *node != under {
                        0.0
                    } else if *port == timing::TIME {
                        probe.time[0]
                    } else if *port == timing::TIME_Y {
                        probe.time[1]
                    } else {
                        0.0
                    };
                    UniformValue::Vec2(nodes::phasor::split(count))
                }
                UniformProvider::Option { node, key } => {
                    let n = g.get(*node)?;
                    let value = n.options.get(key).map_or("", |v| v.as_str());
                    UniformValue::Int(n.def.option(key)?.index_of(value))
                }
            };
            Some((Arc::clone(name), value))
        })
        .collect()
}

/// How two frames differ: the share of pixels off by more than [`LEVEL`] in some channel, and
/// the largest difference in any channel. A NaN matches a NaN.
fn compare(a: &[f32], b: &[f32]) -> (f64, f32) {
    let mut off = 0usize;
    let mut most = 0.0f32;
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let mut worst = 0.0f32;
        for (x, y) in p.iter().zip(q) {
            let d = if x == y || (x.is_nan() && y.is_nan()) {
                0.0
            } else if x.is_nan() || y.is_nan() {
                1.0
            } else {
                (x - y).abs()
            };
            worst = worst.max(if d.is_nan() { 1.0 } else { d });
        }
        if worst > LEVEL {
            off += 1;
        }
        most = most.max(worst);
    }
    (off as f64 / (a.len() / 4) as f64, most)
}

fn same(a: &[f32], b: &[f32]) -> bool {
    compare(a, b).0 <= SAME
}

/// Whether `b` is `a` come back, for a node whose largest difference over its period is
/// `motion`: the same frame, and no pixel off by more than a sliver of that motion.
fn back(a: &[f32], b: &[f32], motion: f32) -> bool {
    let (share, most) = compare(a, b);
    share <= SAME && most < BACK_FLOOR.max(BACK_SHARE * motion)
}

/// The mean absolute difference over every channel of two frames.
fn mean_diff(a: &[f32], b: &[f32]) -> f64 {
    let sum: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = f64::from((x - y).abs());
            if d.is_finite() { d } else { 0.0 }
        })
        .sum();
    sum / a.len() as f64
}

// ------------------------------------------------------------------------------ the nodes

/// What feeds the node under test.
#[derive(Clone, Copy)]
enum Feed {
    /// Nothing: a generator.
    Nothing,
    /// The coordinate picture, into this input.
    Picture(&'static str),
    /// The world's `x`, into this number input.
    Number(&'static str),
    /// A picture of two waves, red along `x` and green along `y`, into this input: bounded
    /// wherever it is read, for the tunnel at Depth Wrap None, which reads its input
    /// thousands of units out, where the coordinate picture outgrows a half float.
    Waves(&'static str),
}

/// One node as the audit draws it.
struct Spec {
    slug: &'static str,
    /// The output drawn.
    root: &'static str,
    /// Whether the root is a field, drawn into all three channels of an `rgba`.
    field: bool,
    feed: Feed,
    /// Settings swept beyond each control's ends, zero and a point inside.
    specials: Vec<Vec<(&'static str, f32)>>,
}

impl Spec {
    fn new(slug: &'static str, root: &'static str, feed: Feed) -> Self {
        Self {
            slug,
            root,
            field: false,
            feed,
            specials: Vec::new(),
        }
    }

    fn field(mut self) -> Self {
        self.field = true;
        self
    }

    fn special(mut self, setting: &[(&'static str, f32)]) -> Self {
        self.specials.push(setting.to_vec());
        self
    }
}

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, emath::Pos2::ZERO).expect("in the registry")
}

fn set(g: &mut Graph, node: NodeId, key: &'static str, value: f32) {
    g.get_mut(node)
        .expect("in the graph")
        .controls
        .insert(key, ControlValue::Float(value));
}

/// The coordinate picture: red the world's `x`, green its `y`, blue a constant.
fn coordinates(g: &mut Graph) -> PortRef {
    let world = add(g, "worldcoordinates");
    let rgba = add(g, "rgba");
    g.connect(PortRef::new(world, "x"), PortRef::new(rgba, "r"))
        .unwrap();
    g.connect(PortRef::new(world, "y"), PortRef::new(rgba, "g"))
        .unwrap();
    set(g, rgba, "b", 0.25);
    PortRef::new(rgba, "output")
}

/// `spec`'s node under these options, drawn into an Output: the graph, the Output, the node.
fn build(spec: &Spec, options: &[(&'static str, &'static str)]) -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let under = add(&mut g, spec.slug);
    let out = add(&mut g, "output");
    for (key, value) in options {
        g.get_mut(under)
            .unwrap()
            .options
            .insert(key, (*value).to_string());
    }
    match spec.feed {
        Feed::Nothing => {}
        Feed::Picture(input) => {
            let picture = coordinates(&mut g);
            g.connect(picture, PortRef::new(under, input)).unwrap();
        }
        Feed::Waves(input) => {
            let world = add(&mut g, "worldcoordinates");
            let rgba = add(&mut g, "rgba");
            for (axis, channel, frequency) in [("x", "r", 0.37), ("y", "g", 0.53)] {
                let wave = add(&mut g, "sine");
                set(&mut g, wave, "frequency", frequency);
                g.connect(PortRef::new(world, axis), PortRef::new(wave, "input"))
                    .unwrap();
                g.connect(PortRef::new(wave, "output"), PortRef::new(rgba, channel))
                    .unwrap();
            }
            set(&mut g, rgba, "b", 0.25);
            g.connect(PortRef::new(rgba, "output"), PortRef::new(under, input))
                .unwrap();
        }
        Feed::Number(input) => {
            let world = add(&mut g, "worldcoordinates");
            g.connect(PortRef::new(world, "x"), PortRef::new(under, input))
                .unwrap();
        }
    }
    if spec.field {
        let rgba = add(&mut g, "rgba");
        for channel in ["r", "g", "b"] {
            g.connect(PortRef::new(under, spec.root), PortRef::new(rgba, channel))
                .unwrap();
        }
        g.connect(PortRef::new(rgba, "output"), PortRef::new(out, "input"))
            .unwrap();
    } else {
        g.connect(PortRef::new(under, spec.root), PortRef::new(out, "input"))
            .unwrap();
    }
    (g, out, under)
}

/// A setting: the controls it moves from their defaults, and a name for a failure message.
#[derive(Clone)]
struct Setting {
    name: String,
    values: Vec<(&'static str, f32)>,
}

/// A small deterministic generator, so a failure names the same setting every run.
struct Lcg(u64);

impl Lcg {
    fn unit(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

/// The node's own number controls, hidden ones included: every one but the time rows, by key,
/// with its default and its declared ends.
fn swept(g: &Graph, under: NodeId) -> Vec<(&'static str, f32, f32, f32)> {
    let node = g.get(under).unwrap();
    let mut keys: Vec<_> = node
        .controls
        .iter()
        .filter(|(key, _)| !timing::is_time_row(key))
        .filter_map(|(key, value)| {
            let ControlValue::Float(default) = value else {
                return None;
            };
            let range = nodes::declared_range(node.def, key)?;
            Some((*key, *default, range.min, range.max))
        })
        .collect();
    keys.sort_by_key(|k| k.0);
    keys
}

/// The settings swept for a node: its defaults; with `full`, each control at its two ends, at
/// zero where its range crosses it, and 37% of the way along; `random` settings moving every
/// control at once; and the spec's own.
fn settings(spec: &Spec, g: &Graph, under: NodeId, full: bool, random: usize) -> Vec<Setting> {
    let keys = swept(g, under);
    let mut out = vec![Setting {
        name: "defaults".into(),
        values: Vec::new(),
    }];
    if full {
        for (key, default, min, max) in &keys {
            let mut points = vec![*min, *max, min + 0.37 * (max - min)];
            if *min < 0.0 && *max > 0.0 {
                points.push(0.0);
            }
            for v in points {
                if v != *default {
                    out.push(Setting {
                        name: format!("{key}={v}"),
                        values: vec![(*key, v)],
                    });
                }
            }
        }
    }
    let def = g.get(under).unwrap().def;
    let mut rng = Lcg(spec
        .slug
        .bytes()
        .fold(7u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b))));
    for i in 0..random {
        let values: Vec<_> = keys
            .iter()
            .map(|(key, _, min, max)| {
                let mut v = min + rng.unit() * (max - min);
                // A whole-number control — Turns, Seed, Octaves — stays whole.
                if def.input(key).is_some_and(
                    |d| matches!(d.control, nodes::Control::Number { step, .. } if step >= 1.0),
                ) {
                    v = v.round();
                }
                (*key, v)
            })
            .collect();
        out.push(Setting {
            name: format!("random#{i} {values:?}"),
            values,
        });
    }
    for values in &spec.specials {
        out.push(Setting {
            name: format!("{values:?}"),
            values: values.clone(),
        });
    }
    out
}

/// Which Times a probe moves: all of them together, as ambient time does, or one of Shaky
/// Cam's two with the other held.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    Both,
    X,
    Y,
}

/// A probe at Time `t` with Offset `offset` on `axis`; an axis held stands at 0.37 with no
/// Offset.
fn at(axis: Axis, t: f64, offset: f32) -> Probe {
    const HELD: f64 = 0.37;
    match axis {
        Axis::Both => Probe {
            time: [t, t],
            offset: [offset, offset],
        },
        Axis::X => Probe {
            time: [t, HELD],
            offset: [offset, 0.0],
        },
        Axis::Y => Probe {
            time: [HELD, t],
            offset: [0.0, offset],
        },
    }
}

/// Where a claim is held: a Time, and an Offset as a share of the period.
const CLAIMS: [(f64, f64); 6] = [
    (0.0, 0.0),
    (0.37, 0.0),
    (1000.37, 0.3),
    (-3.61, -0.71),
    // Across the 40320 the count's whole part wraps at, a period on.
    (20159.37, 0.13),
    (7.5, 0.999),
];

/// Every way the claim of a period `period` fails on `rig` as it is set: a frame one and three
/// periods on that is not the frame at the start.
fn claim_failures(rig: &mut Rig, period: f64, axis: Axis) -> Vec<String> {
    let mut probes = Vec::new();
    for (t, share) in CLAIMS {
        let offset = (share * period) as f32;
        for k in [0.0, 1.0, 3.0] {
            probes.push(at(axis, t + k * period, offset));
        }
    }
    let frames = rig.frames(&probes);
    let mut failures = Vec::new();
    for (i, (t, share)) in CLAIMS.iter().enumerate() {
        let base = &frames[3 * i];
        for (j, k) in [1.0, 3.0].iter().enumerate() {
            let (frac, most) = compare(base, &frames[3 * i + 1 + j]);
            if frac > SAME {
                failures.push(format!(
                    "Time {t} Offset {:.3}: {k}×{period} on differs in {:.1}% of pixels, by up to {most:.3}",
                    share * period,
                    frac * 100.0
                ));
            }
        }
    }
    failures
}

/// What the early-loop sweep found on one setting.
struct Early {
    /// The node stands still: every probe draws the start.
    still: bool,
    failures: Vec<String>,
    /// The step that came nearest to the start, and its largest difference as a share of
    /// the node's motion.
    nearest: (f64, f32),
}

/// The steps a picture with period `period` must not have come back after: every whole
/// number under it, `period / k` for `k` from 2 to 12, and a few fractions; with no period,
/// 1 to 16 and two fractions. `whole` keeps only whole steps, for Static, which holds each
/// roll for a whole cycle.
fn steps(period: Option<f64>, whole: bool) -> Vec<f64> {
    let mut d = Vec::new();
    if let Some(p) = period {
        let mut n = 1.0;
        while n < p - 1e-9 {
            d.push(n);
            n += 1.0;
        }
        if !whole {
            for k in 2..=12 {
                d.push(p / f64::from(k));
            }
            for f in [0.25, 0.37, 0.5, 0.75] {
                if f < p {
                    d.push(f);
                }
            }
        }
    } else {
        d.extend((1..=16).map(f64::from));
        if !whole {
            d.extend([0.5, 2.5]);
        }
    }
    d.sort_by(f64::total_cmp);
    d.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    d
}

/// The early-loop sweep on `rig` as it is set, from Time 0.37 with no Offset.
fn early(rig: &mut Rig, period: Option<f64>, axis: Axis, whole: bool) -> Early {
    const START: f64 = 0.37;
    const AGAIN: f64 = 0.61;
    let reach = period.unwrap_or(1.0);
    let still: Vec<f64> = if whole {
        vec![1.0, 2.0, 3.0]
    } else {
        vec![0.0731 * reach, 0.137 * reach, 0.5 * reach, 0.77 * reach]
    };
    let d = steps(period, whole);
    let mut probes = vec![at(axis, START, 0.0)];
    probes.extend(still.iter().map(|s| at(axis, START + s, 0.0)));
    probes.extend(d.iter().map(|s| at(axis, START + s, 0.0)));
    let frames = rig.frames(&probes);
    let base = &frames[0];
    if frames[1..=still.len()].iter().all(|f| same(base, f)) {
        return Early {
            still: true,
            failures: Vec::new(),
            nearest: (0.0, 0.0),
        };
    }
    let motion = frames[1..=still.len()]
        .iter()
        .map(|f| compare(base, f).1)
        .fold(0.0, f32::max);
    let mut candidates = Vec::new();
    let mut nearest = (f64::NAN, f32::INFINITY);
    for (s, frame) in d.iter().zip(&frames[1 + still.len()..]) {
        let (frac, most) = compare(base, frame);
        if back(base, frame, motion) {
            candidates.push((*s, frac, most));
        } else if most / motion < nearest.1 {
            nearest = (*s, most / motion);
        }
    }
    // A loop comes back from wherever it starts; a picture that is one number's function —
    // Shaky Cam's offset on one axis — passes back through a value it held without having
    // looped. So a step that came back is held again from a second start.
    let mut probes = vec![at(axis, AGAIN, 0.0)];
    probes.extend(candidates.iter().map(|(s, _, _)| at(axis, AGAIN + s, 0.0)));
    let again = rig.frames(&probes);
    let mut failures = Vec::new();
    for ((s, frac, most), frame) in candidates.iter().zip(&again[1..]) {
        if back(&again[0], frame, motion) {
            failures.push(format!(
                "back {s:.4} on (claims {period:?}): {:.1}% of pixels differ, by up to {most:.4} \
                 where it moves by up to {motion:.4}",
                frac * 100.0
            ));
        } else {
            let ratio = compare(base, &frames[0]).1.max(*most) / motion;
            if ratio < nearest.1 {
                nearest = (*s, ratio);
            }
        }
    }
    Early {
        still: false,
        failures,
        nearest,
    }
}

/// The option sets a node is drawn under: the product of every option listed.
fn product(options: &[(&'static str, &[&'static str])]) -> Vec<Vec<(&'static str, &'static str)>> {
    let mut out = vec![Vec::new()];
    for (key, values) in options {
        out = out
            .into_iter()
            .flat_map(|set: Vec<_>| {
                values.iter().map(move |v| {
                    let mut s = set.clone();
                    s.push((*key, *v));
                    s
                })
            })
            .collect();
    }
    out
}

/// A node over option sets: each one built, linked once and handed every setting.
struct Audit {
    spec: Spec,
    /// The option sets swept with every setting…
    full: Vec<Vec<(&'static str, &'static str)>>,
    /// …and those swept with the defaults, two random settings and the spec's own.
    light: Vec<Vec<(&'static str, &'static str)>>,
}

impl Audit {
    fn new(spec: Spec) -> Self {
        Self {
            spec,
            full: vec![Vec::new()],
            light: Vec::new(),
        }
    }

    fn full(mut self, options: &[(&'static str, &[&'static str])]) -> Self {
        self.full = product(options);
        self
    }

    fn light(mut self, options: &[(&'static str, &[&'static str])]) -> Self {
        self.light = product(options);
        self
    }

    /// `check` on every option set and every setting, the rig set to it: every failure,
    /// named. `check` returns failures; the summary line is printed for the audit's report.
    fn run(
        &self,
        what: &str,
        mut check: impl FnMut(&mut Rig, &Setting, Option<f64>) -> Vec<String>,
    ) -> Vec<String> {
        let mut failures = Vec::new();
        let mut cases = 0;
        let sets = self
            .full
            .iter()
            .map(|s| (s, true))
            .chain(self.light.iter().map(|s| (s, false)));
        for (options, full) in sets {
            let (g, out, under) = build(&self.spec, options);
            let node = g.get(under).unwrap();
            let period = (node.def.timing.expect("moves with time").period)(node);
            let all = settings(&self.spec, &g, under, full, if full { 3 } else { 2 });
            let defaults: Vec<_> = swept(&g, under)
                .into_iter()
                .map(|(k, d, _, _)| (k, d))
                .collect();
            let mut rig = Rig::new(g, out, under);
            // A control the drawn output never reads changes nothing, and is not swept.
            let all: Vec<_> = all
                .into_iter()
                .filter_map(|mut s| {
                    let moved = !s.values.is_empty();
                    s.values.retain(|(k, _)| rig.reads(k));
                    (!moved || !s.values.is_empty()).then_some(s)
                })
                .collect();
            for setting in &all {
                for (key, value) in &defaults {
                    rig.set(key, *value);
                }
                for (key, value) in &setting.values {
                    rig.set(key, *value);
                }
                cases += 1;
                for f in check(&mut rig, setting, period) {
                    failures.push(format!("{options:?} {}: {f}", setting.name));
                }
            }
        }
        eprintln!(
            "{} {} {what}: {cases} settings, {} failures",
            self.spec.slug,
            self.spec.root,
            failures.len()
        );
        failures
    }

    /// No false claim, on `axis`.
    fn claims(&self, axis: Axis) -> Vec<String> {
        self.run(&format!("claim {axis:?}"), |rig, _, period| match period {
            Some(p) => claim_failures(rig, p, axis),
            None => Vec::new(),
        })
    }

    /// No early loop, on `axis`, printing the settings that stood still and the step that
    /// came nearest to the start.
    fn early(&self, axis: Axis, whole: bool) -> Vec<String> {
        let mut stood = Vec::new();
        let mut nearest = (f64::NAN, f32::INFINITY, String::new());
        let failures = self.run(&format!("early {axis:?}"), |rig, setting, period| {
            let e = early(rig, period, axis, whole);
            if e.still {
                stood.push(setting.name.clone());
            } else if e.nearest.1 < nearest.1 {
                nearest = (e.nearest.0, e.nearest.1, setting.name.clone());
            }
            e.failures
        });
        eprintln!(
            "{} {} early {axis:?}: stood still under {stood:?}; nearest return {nearest:?}",
            self.spec.slug, self.spec.root
        );
        failures
    }
}

/// Fail with every failure, the first forty spelled out and every one printed.
fn assert_none(what: &str, failures: &[String]) {
    for f in failures {
        eprintln!("FAIL {what}: {f}");
    }
    let mut message = String::new();
    for f in failures.iter().take(40) {
        writeln!(message, "  {f}").unwrap();
    }
    assert!(
        failures.is_empty(),
        "{what}: {} failures, the first {}:\n{message}",
        failures.len(),
        failures.len().min(40)
    );
}

// ------------------------------------------------------------------------------ the specs

const REPEATS: &[&str] = &["1", "2", "4", "8", "16"];
const STATIC_REPEATS: &[&str] = &["4", "8", "16", "32", "64", "128"];
const FBM_TYPES: &[&str] = &["standard", "turbulence", "ridged"];

fn cosinegradient() -> Audit {
    Audit::new(
        Spec::new("cosinegradient", "output", Feed::Number("t"))
            .special(&[("freqR", 0.0), ("freqG", 0.0), ("freqB", 0.0)])
            .special(&[("freqR", 2.37), ("freqG", 0.61), ("freqB", 3.99)])
            .special(&[("freqR", 4.0), ("freqG", 4.0), ("freqB", 4.0)])
            .special(&[("freqR", 0.5), ("freqG", 0.25), ("freqB", 1.5)]),
    )
}

fn escape(slug: &'static str) -> Audit {
    Audit::new(Spec::new(slug, "map", Feed::Picture("input")))
}

fn geissflow() -> Audit {
    Audit::new(
        Spec::new("geissflow", "output", Feed::Picture("lastFrame"))
            .special(&[("swirl", 0.0), ("feedbackAmount", 1.0)])
            .special(&[("feedbackAmount", 1.0), ("distortionAmount", 0.1)]),
    )
}

fn geissflow_angle() -> Audit {
    Audit::new(
        Spec::new("geissflow", "angle", Feed::Nothing)
            .field()
            .special(&[("swirl", 0.0)]),
    )
}

fn noise(slug: &'static str, repeats: &'static [&'static str]) -> Audit {
    Audit::new(Spec::new(slug, "color", Feed::Nothing)).full(&[("repeat", repeats)])
}

fn fractal(repeats: &'static [&'static str]) -> Audit {
    Audit::new(Spec::new("fractal", "color", Feed::Nothing))
        .full(&[("type", &["standard"]), ("repeat", repeats)])
        .light(&[("type", &["turbulence", "ridged"]), ("repeat", repeats)])
}

fn domainwarp(repeats: &'static [&'static str]) -> Audit {
    Audit::new(Spec::new("domainwarp", "output", Feed::Picture("input")))
        .full(&[
            ("type", &["standard"]),
            ("iterations", &["1"]),
            ("repeat", repeats),
        ])
        .light(&[
            ("type", FBM_TYPES),
            ("iterations", &["2", "3"]),
            ("repeat", repeats),
        ])
}

fn tunnel(wraps: &'static [&'static str]) -> Audit {
    Audit::new(Spec::new("tunnel3d", "output", Feed::Picture("input")))
        .full(&[("path", &["sine", "helix", "lissajous"]), ("wrap", wraps)])
        .light(&[
            ("path", &["sine"]),
            ("wrap", wraps),
            ("mapping", &["cartesian_mirror", "polar"]),
            ("shading", &["light", "heavy"]),
        ])
}

fn rotozoom() -> Audit {
    Audit::new(
        Spec::new("rotozoom", "output", Feed::Picture("input"))
            .special(&[("cosCoeff", 0.0), ("turns", 0.0)])
            .special(&[("cosCoeff", 0.0), ("turns", 2.0)])
            .special(&[("cosCoeff", 0.0), ("turns", 4.0)])
            .special(&[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 3.0)])
            .special(&[("sinCoeff", 0.0), ("cosCoeff", 0.0), ("turns", 1.0)]),
    )
}

fn shakycam() -> Audit {
    Audit::new(Spec::new("shakycam", "output", Feed::Picture("input")))
}

// ---------------------------------------------------------------- property 1: no false claim

#[test]
fn cosinegradient_comes_back_after_its_period() {
    assert_none("cosinegradient", &cosinegradient().claims(Axis::Both));
}

#[test]
fn mandelbrot_comes_back_after_its_period() {
    assert_none("mandelbrot", &escape("mandelbrot").claims(Axis::Both));
}

#[test]
fn juliaset_comes_back_after_its_period() {
    assert_none("juliaset", &escape("juliaset").claims(Axis::Both));
}

#[test]
fn geissflow_comes_back_after_its_period() {
    let mut failures = geissflow().claims(Axis::Both);
    failures.extend(geissflow_angle().claims(Axis::Both));
    assert_none("geissflow", &failures);
}

#[test]
fn perlin_comes_back_after_its_repeat() {
    assert_none("perlin", &noise("perlin", REPEATS).claims(Axis::Both));
}

#[test]
fn simplex_comes_back_after_its_repeat() {
    assert_none("simplex", &noise("simplex", REPEATS).claims(Axis::Both));
}

#[test]
fn fractal_comes_back_after_its_repeat() {
    assert_none("fractal", &fractal(REPEATS).claims(Axis::Both));
}

#[test]
fn domainwarp_comes_back_after_its_repeat() {
    assert_none("domainwarp", &domainwarp(REPEATS).claims(Axis::Both));
}

#[test]
fn static_comes_back_after_its_repeat() {
    assert_none(
        "static",
        &noise("static", STATIC_REPEATS).claims(Axis::Both),
    );
}

#[test]
fn tunnel3d_comes_back_after_64() {
    assert_none(
        "tunnel3d",
        &tunnel(&["mirror", "repeat"]).claims(Axis::Both),
    );
}

#[test]
fn rotozoom_comes_back_after_its_period() {
    assert_none("rotozoom", &rotozoom().claims(Axis::Both));
}

#[test]
fn shakycam_comes_back_after_its_period_on_each_axis() {
    let mut failures = shakycam().claims(Axis::X);
    failures.extend(shakycam().claims(Axis::Y));
    failures.extend(shakycam().claims(Axis::Both));
    assert_none("shakycam", &failures);
}

// ---------------------------------------------------------------- property 2: no early loop

#[test]
fn cosinegradient_does_not_come_back_early() {
    assert_none("cosinegradient", &cosinegradient().early(Axis::Both, false));
}

#[test]
fn mandelbrot_does_not_come_back_early() {
    assert_none("mandelbrot", &escape("mandelbrot").early(Axis::Both, false));
}

#[test]
fn juliaset_does_not_come_back_early() {
    assert_none("juliaset", &escape("juliaset").early(Axis::Both, false));
}

#[test]
fn geissflow_does_not_come_back_early() {
    let mut failures = geissflow().early(Axis::Both, false);
    failures.extend(geissflow_angle().early(Axis::Both, false));
    assert_none("geissflow", &failures);
}

#[test]
fn perlin_does_not_come_back_before_its_repeat() {
    assert_none("perlin", &noise("perlin", REPEATS).early(Axis::Both, false));
}

#[test]
fn simplex_does_not_come_back_before_its_repeat() {
    assert_none(
        "simplex",
        &noise("simplex", REPEATS).early(Axis::Both, false),
    );
}

#[test]
fn fractal_does_not_come_back_before_its_repeat() {
    assert_none("fractal", &fractal(REPEATS).early(Axis::Both, false));
}

#[test]
fn domainwarp_does_not_come_back_before_its_repeat() {
    assert_none("domainwarp", &domainwarp(REPEATS).early(Axis::Both, false));
}

#[test]
fn static_does_not_come_back_before_its_repeat() {
    assert_none(
        "static",
        &noise("static", STATIC_REPEATS).early(Axis::Both, true),
    );
}

#[test]
fn tunnel3d_does_not_come_back_before_64() {
    assert_none(
        "tunnel3d",
        &tunnel(&["mirror", "repeat"]).early(Axis::Both, false),
    );
}

#[test]
fn rotozoom_does_not_come_back_early() {
    assert_none("rotozoom", &rotozoom().early(Axis::Both, false));
}

#[test]
fn shakycam_does_not_come_back_early_on_x() {
    assert_none("shakycam X", &shakycam().early(Axis::X, false));
}

#[test]
fn shakycam_does_not_come_back_early_on_y() {
    assert_none("shakycam Y", &shakycam().early(Axis::Y, false));
}

#[test]
fn shakycam_does_not_come_back_early_on_both() {
    assert_none("shakycam both", &shakycam().early(Axis::Both, false));
}

// ----------------------------------------------------- a picture that says it never repeats

#[test]
fn a_noise_at_repeat_never_does_not_come_back_within_sixteen_cells() {
    let mut failures = Vec::new();
    for slug in ["perlin", "simplex"] {
        failures.extend(noise(slug, &["never"]).early(Axis::Both, false));
    }
    failures.extend(fractal(&["never"]).early(Axis::Both, false));
    failures.extend(domainwarp(&["never"]).early(Axis::Both, false));
    failures.extend(noise("static", &["never"]).early(Axis::Both, true));
    assert_none("Repeat Never", &failures);
}

#[test]
fn the_tunnel_at_depth_wrap_none_does_not_come_back_within_sixteen_units() {
    assert_none("tunnel3d none", &tunnel(&["none"]).early(Axis::Both, false));
}

/// The five pictures that never repeat and move smoothly, at their defaults.
fn unbounded() -> [(Spec, &'static [(&'static str, &'static str)]); 5] {
    [
        (
            Spec::new("perlin", "color", Feed::Nothing),
            &[("repeat", "never")],
        ),
        (
            Spec::new("simplex", "color", Feed::Nothing),
            &[("repeat", "never")],
        ),
        (
            Spec::new("fractal", "color", Feed::Nothing),
            &[("repeat", "never")],
        ),
        (
            Spec::new("domainwarp", "output", Feed::Picture("input")),
            &[("repeat", "never")],
        ),
        (
            Spec::new("tunnel3d", "output", Feed::Waves("input")),
            &[("wrap", "none")],
        ),
    ]
}

/// A picture that never repeats must not come back after a long show either. Perlin's
/// lattice is taken modulo 289 on every axis, time included; the simplex lattice's skew makes
/// that 867 along time; and the count's whole part wraps at 40320.
#[test]
fn a_picture_that_never_repeats_does_not_come_back_after_a_long_show() {
    let mut failures = Vec::new();
    for (spec, options) in unbounded() {
        let (g, out, under) = build(&spec, options);
        let mut rig = Rig::new(g, out, under);
        let long = [289.0, 578.0, 867.0, 40320.0];
        let start = 0.37;
        let mut probes = vec![at(Axis::Both, start, 0.0), at(Axis::Both, start + 1.0, 0.0)];
        probes.extend(long.iter().map(|d| at(Axis::Both, start + d, 0.0)));
        let frames = rig.frames(&probes);
        let motion = compare(&frames[0], &frames[1]).1;
        for (d, frame) in long.iter().zip(&frames[2..]) {
            let (frac, most) = compare(&frames[0], frame);
            eprintln!(
                "{}: {d} on, {:.1}% differ, by up to {most:.4}; a cycle on, {motion:.4}",
                spec.slug,
                frac * 100.0
            );
            if back(&frames[0], frame, motion) {
                failures.push(format!(
                    "{}: back {d} cycles on ({:.1}% of pixels differ, by up to {most:.3})",
                    spec.slug,
                    frac * 100.0
                ));
            }
        }
    }
    assert_none("a long show", &failures);
}

/// A picture that never repeats has no seam where the count's whole part wraps, at 20160
/// cycles: a step across it moves the picture no further than a step as long anywhere else.
#[test]
fn a_picture_that_never_repeats_has_no_seam_where_the_count_wraps() {
    let mut failures = Vec::new();
    for (spec, options) in unbounded() {
        let (g, out, under) = build(&spec, options);
        let mut rig = Rig::new(g, out, under);
        let step = 0.05;
        let frames = rig.frames(&[
            at(Axis::Both, 1000.0 - step, 0.0),
            at(Axis::Both, 1000.0 + step, 0.0),
            at(Axis::Both, 20160.0 - step, 0.0),
            at(Axis::Both, 20160.0 + step, 0.0),
        ]);
        let ordinary = mean_diff(&frames[0], &frames[1]);
        let seam = mean_diff(&frames[2], &frames[3]);
        eprintln!(
            "{}: a step of {} moves {ordinary:.4} at 1000, {seam:.4} across 20160",
            spec.slug,
            2.0 * step
        );
        if seam > 3.0 * ordinary + 0.005 {
            failures.push(format!(
                "{}: a step of {} across 20160 moves {seam:.4} on average, one at 1000 {ordinary:.4}",
                spec.slug,
                2.0 * step
            ));
        }
    }
    assert_none("the count's wrap", &failures);
}

/// A picture that never repeats moves as smoothly at the far end of its count as at the
/// start: a frame at 60 a second, at half a cell a second, moves about as far at 20159 cells
/// as at the first, rather than standing still between float steps.
#[test]
fn a_picture_that_never_repeats_moves_as_smoothly_late_as_early() {
    let mut failures = Vec::new();
    for (spec, options) in unbounded() {
        let (g, out, under) = build(&spec, options);
        let mut rig = Rig::new(g, out, under);
        let frame = 0.5 / 60.0;
        let mut probes = Vec::new();
        for start in [0.37, 20159.37] {
            for i in 0..5 {
                probes.push(at(Axis::Both, start + f64::from(i) * frame, 0.0));
            }
        }
        let frames = rig.frames(&probes);
        let moves = |from: usize| -> Vec<f64> {
            (0..4)
                .map(|i| mean_diff(&frames[from + i], &frames[from + i + 1]))
                .collect()
        };
        let (early_moves, late_moves) = (moves(0), moves(5));
        eprintln!(
            "{}: a frame moves {early_moves:.5?} early, {late_moves:.5?} at 20159",
            spec.slug
        );
        let typical = early_moves.iter().sum::<f64>() / 4.0;
        for m in &late_moves {
            if *m < 0.25 * typical || *m > 4.0 * typical {
                failures.push(format!(
                    "{}: at 20159 cells a frame moves {m:.5}, at the start {typical:.5}",
                    spec.slug
                ));
            }
        }
    }
    assert_none("late precision", &failures);
}

// ------------------------------------------------------------- the Cosine Gradient's Frequency

/// **Frequency never moves the Cosine Gradient's period.** Time is added to each channel's
/// phase, not multiplied by its frequency, so whatever the three frequencies — whole, not
/// whole, below one, mixed across R, G and B — and whatever range the number spans, the frame
/// a cycle on is the frame now and the frame half a cycle on is not. Its input is the world's
/// `x` scaled into 0 to 1, −1 to 1 and −3 to 3.
#[test]
fn cosinegradient_frequency_never_moves_its_period() {
    let frequencies: [[f32; 3]; 5] = [
        [1.0, 1.0, 1.0],
        [0.5, 0.37, 1.7],
        [2.3, 4.0, 0.5],
        [0.37, 1.7, 2.3],
        [4.0, 0.5, 1.0],
    ];
    let phases = [0.13, 0.41, 0.77];
    let mut failures = Vec::new();
    for (scale, shift) in [(0.5, 0.5), (1.0, 0.0), (3.0, 0.0)] {
        let mut g = Graph::new();
        let under = add(&mut g, "cosinegradient");
        let out = add(&mut g, "output");
        g.get_mut(under)
            .unwrap()
            .options
            .insert(timing::MODE.key, timing::LOOP.to_string());
        let world = add(&mut g, "worldcoordinates");
        let times = add(&mut g, "multiply");
        let plus = add(&mut g, "add");
        set(&mut g, times, "b", scale);
        set(&mut g, plus, "b", shift);
        g.connect(PortRef::new(world, "x"), PortRef::new(times, "a"))
            .unwrap();
        g.connect(PortRef::new(times, "output"), PortRef::new(plus, "a"))
            .unwrap();
        g.connect(PortRef::new(plus, "output"), PortRef::new(under, "t"))
            .unwrap();
        g.connect(PortRef::new(under, "output"), PortRef::new(out, "input"))
            .unwrap();
        let mut rig = Rig::new(g, out, under);
        for (key, phase) in ["phaseR", "phaseG", "phaseB"].into_iter().zip(phases) {
            rig.set(key, phase);
        }
        for freq in frequencies {
            for (key, f) in ["freqR", "freqG", "freqB"].into_iter().zip(freq) {
                rig.set(key, f);
            }
            for t in [0.0, 0.25, 7.6, 1000.37] {
                for offset in [0.0, 0.3] {
                    let frames = rig.frames(&[
                        at(Axis::Both, t, offset),
                        at(Axis::Both, t + 1.0, offset),
                        at(Axis::Both, t + 0.5, offset),
                    ]);
                    let (_, cycle) = compare(&frames[0], &frames[1]);
                    let (_, half) = compare(&frames[0], &frames[2]);
                    eprintln!(
                        "input {:+}..{:+} freq {freq:?} t {t} offset {offset}: \
                         max |Δ| a cycle on {cycle:.4}, half a cycle on {half:.4}",
                        shift - scale,
                        shift + scale
                    );
                    if !same(&frames[0], &frames[1]) || half < 8.0 / 255.0 {
                        failures.push(format!(
                            "range ±{scale}+{shift} freq {freq:?} t {t} offset {offset}: \
                             a cycle on {cycle:.4}, half a cycle on {half:.4}"
                        ));
                    }
                }
            }
        }
    }
    assert_none("cosinegradient frequency", &failures);
}

/// **The palette strip draws what the shader draws.** `cosinegradient::eval`, which the strip
/// and the curves on the node are painted from, at the drift the editor hands it — Time's
/// fraction plus Offset — agrees with the rendered frame at every pixel, at a Time and Offset
/// well away from zero and with frequencies that are not whole.
#[test]
fn the_palette_strip_draws_what_the_shader_draws() {
    let spec = Spec::new("cosinegradient", "output", Feed::Number("t"));
    let (g, out, under) = build(&spec, &[]);
    let mut rig = Rig::new(g, out, under);
    let coefficients = [
        [0.5, 0.45, 0.55],
        [0.5, 0.4, 0.45],
        [0.37, 1.7, 2.3],
        [0.13, 0.41, 0.77],
    ];
    for (row, keys) in nodes::cosinegradient::ROWS.iter().enumerate() {
        for (c, key) in keys.1.iter().enumerate() {
            rig.set(key, coefficients[row][c]);
        }
    }
    let (t, offset) = (1000.37, 0.3f32);
    let frame = &rig.frames(&[at(Axis::Both, t, offset)])[0];
    // What the editor hands the strip: `phasor::fraction` of the count, plus the Offset knob.
    let drift = nodes::phasor::fraction(t, 1.0) as f32 + offset;
    let mut worst = 0.0f32;
    for px in 0..SIZE as usize {
        let x = (2.0 * (px as f32 + 0.5) / SIZE as f32) - 1.0;
        let want = nodes::cosinegradient::eval(&coefficients, drift, x);
        let got = &frame[px * 4..px * 4 + 3];
        for c in 0..3 {
            worst = worst.max((want[c] - got[c]).abs());
        }
    }
    eprintln!("the strip and the shader differ by up to {worst:.4}");
    assert!(worst < 2.0 / 255.0, "the strip is off by {worst}");
}

// ------------------------------------------------- a scroll through a window, seen whole

/// **The palette meets itself at the ends of its domain, at every drift.** Time scrolls each
/// channel's wave along the number's 0 to 1, a cycle of Time moving it `1 / freq` of the way.
/// The frame comes back after a cycle at any Frequency, but only a whole Frequency puts whole
/// waves in the window, so that the picture scrolls through the cycle like a belt: at a
/// Frequency that is not whole the channel's value at 0 and at 1 differ, the strip on the node
/// and any picture fed a number that wraps show a seam there, and the shape in the window
/// changes through the cycle and only matches at whole cycles. Drawn on the GPU at Input 0 and
/// Input 1, at four drifts, over the Frequency sweep; a channel with no amplitude is exempt.
#[test]
fn cosinegradient_palette_meets_itself_at_the_ends_of_its_domain() {
    let spec = Spec::new("cosinegradient", "output", Feed::Nothing)
        .special(&[("freqR", 2.37), ("freqG", 0.61), ("freqB", 3.99)])
        .special(&[("freqR", 0.5), ("freqG", 0.25), ("freqB", 1.5)])
        .special(&[("freqR", 2.0), ("freqG", 3.0), ("freqB", 4.0)]);
    let (g, out, under) = build(&spec, &[]);
    let all = settings(&spec, &g, under, true, 3);
    let defaults: Vec<_> = swept(&g, under)
        .into_iter()
        .map(|(k, d, _, _)| (k, d))
        .collect();
    let mut rig = Rig::new(g, out, under);
    let mut failures = Vec::new();
    for setting in &all {
        for (key, value) in defaults.iter().chain(&setting.values) {
            rig.set(key, *value);
        }
        let control = |key: &str| {
            setting
                .values
                .iter()
                .chain(&defaults)
                .find(|(k, _)| *k == key)
                .map_or(0.0, |(_, v)| *v)
        };
        for drift in [0.0, 0.25, 0.5, 0.77] {
            rig.set("t", 0.0);
            let low = rig.frames(&[at(Axis::Both, drift, 0.0)]).remove(0);
            rig.set("t", 1.0);
            let high = rig.frames(&[at(Axis::Both, drift, 0.0)]).remove(0);
            for (c, name) in ["R", "G", "B"].into_iter().enumerate() {
                let gap = (low[c] - high[c]).abs();
                if gap > LEVEL && control(["ampR", "ampG", "ampB"][c]) != 0.0 {
                    failures.push(format!(
                        "{}: {name} at drift {drift}, Freq {}: Input 0 draws {:.3}, Input 1 {:.3}",
                        setting.name,
                        control(["freqR", "freqG", "freqB"][c]),
                        low[c],
                        high[c]
                    ));
                }
            }
        }
    }
    eprintln!(
        "cosinegradient seam: {} settings, {} failures",
        all.len(),
        failures.len()
    );
    assert_none("cosinegradient seam", &failures);
}

/// **The strip on the node meets itself too.** `cosinegradient::eval`, which the strip and
/// the curves are painted from over Input 0 to 1, gives each channel the same value at both
/// ends at every drift through a cycle, for every Frequency the knob reaches in its own steps
/// of 0.01 from 0 to 4 — which holds only where the Frequency is whole.
#[test]
fn the_palette_strip_meets_itself_at_its_ends() {
    let mut failing = Vec::new();
    for step in 0..=400 {
        let freq = step as f32 / 100.0;
        let coefficients = [[0.5; 3], [0.5; 3], [freq; 3], [0.0, 0.33, 0.67]];
        let worst = (0..20)
            .map(|i| {
                let drift = i as f32 / 20.0;
                let a = nodes::cosinegradient::eval(&coefficients, drift, 0.0);
                let b = nodes::cosinegradient::eval(&coefficients, drift, 1.0);
                (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0, f32::max)
            })
            .fold(0.0, f32::max);
        if worst > LEVEL {
            failing.push(freq);
        }
    }
    assert!(
        failing.is_empty(),
        "the strip's ends differ at {} of the 401 Frequencies from 0 to 4, every one that is not \
         whole: {:?}…",
        failing.len(),
        &failing[..failing.len().min(12)]
    );
}
