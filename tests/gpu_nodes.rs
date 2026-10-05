// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the nodes draw and measure on the renderer: every node's module made into a pipeline on the real GPU, a
//! `mask` that reads 1 inside and 0 outside, a `value` that is a field, a tile's period, the
//! unglamorous four, the phyllotaxis against its plain definition, taps and samples through
//! real graphs, every dual node's two implementations held equal, and the probe's exact counts.
//!
//! Every graph is compiled by `compile::wgsl::build` and drawn through the `Renderer`, with its
//! uniforms resolved as the synth resolves them. An Output's texture holds GL's rows, bottom
//! first (`proposals/wgpu.md`, 1.15), and `gpu::rgba_of` rounds as GL's `read_pixels` into
//! bytes does, so the expected pixels are the ones the GL renderer was held to.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{drain, job, link_all, rgba_of, tick};
use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use supersilvia::compile::wgsl;
use supersilvia::compile::{
    self, Diagnostic, EVAL_WORD, Shader, TAP_WORDS, TapKind, UniformProvider, UniformType,
};
use supersilvia::graph::{ControlValue, Graph, NodeId, PortRef, PortType};
use supersilvia::nodes::{self, NodeDef, sample, tap};
use supersilvia::render::program::Program;
use supersilvia::render::{FrameJob, OutputMode, PROBE, ProbeJob, Renderer, UniformValue};
use supersilvia::{App, Command};

const DRAW: OutputMode = OutputMode::Draw;

// --------------------------------------------------------------------------- the harness

/// Every uniform of `shader` as the synth resolves it: a control and an option off the graph,
/// a texture by the port that publishes it, and a published uniform number from `published`.
fn resolve(
    g: &Graph,
    shader: &Shader,
    published: &dyn Fn(PortRef) -> f32,
) -> Vec<(Arc<str>, UniformValue)> {
    shader
        .uniforms
        .iter()
        .filter_map(|(name, provider)| {
            let value = match provider {
                UniformProvider::Control { node, key, .. } => {
                    match g.get(*node)?.controls.get(key)? {
                        ControlValue::Float(v) => UniformValue::Float(*v),
                        ControlValue::Color(v) => UniformValue::Vec4(*v),
                    }
                }
                UniformProvider::NodeTexture { node, port } => {
                    UniformValue::NodeTexture(PortRef::new(*node, port))
                }
                UniformProvider::NodeUniform { node, port, ty } => match ty {
                    UniformType::Vec4 => UniformValue::Vec4([0.0; 4]),
                    _ => UniformValue::Float(published(PortRef::new(*node, port))),
                },
                UniformProvider::NodeCount { node, port } => {
                    UniformValue::Vec2(supersilvia::nodes::phasor::split(f64::from(published(
                        PortRef::new(*node, port),
                    ))))
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

/// `out`'s WGSL module, with nothing but WGSL in it.
fn module_of(g: &Graph, out: NodeId) -> Arc<Shader> {
    let shader = wgsl::build(g, out).expect("the Output is connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    Arc::new(shader)
}

/// One graph drawn through the renderer at `size` square, and its pixels as RGBA8, rows
/// bottom first.
fn rendered(g: &Graph, out: NodeId, size: u32) -> Vec<[u8; 4]> {
    rendered_publishing(g, out, size, &|_| 0.0)
}

/// [`rendered`], with every uniform number a CPU half publishes read from `published`.
fn rendered_publishing(
    g: &Graph,
    out: NodeId,
    size: u32,
    published: &dyn Fn(PortRef) -> f32,
) -> Vec<[u8; 4]> {
    let shader = module_of(g, out);
    let values = resolve(g, &shader, published);
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs =
        |send: bool, mode| vec![job(out, (size, size), &shader, send, mode, values.clone())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"))
}

/// A generator's field port, driven into an `rgba`'s three channels so it can be read back
/// as a picture.
fn field_graph(slug: &str, field: &'static str) -> (Graph, NodeId) {
    let mut g = Graph::new();
    let under = nodes::add_to_graph(&mut g, slug, emath::Pos2::ZERO).unwrap();
    let rgba = nodes::add_to_graph(&mut g, "rgba", emath::Pos2::ZERO).unwrap();
    let out = nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO).unwrap();
    for channel in ["r", "g", "b"] {
        g.connect(PortRef::new(under, field), PortRef::new(rgba, channel))
            .expect("a field is a varying number and a channel takes one");
    }
    g.connect(PortRef::new(rgba, "output"), PortRef::new(out, "input"))
        .unwrap();
    (g, out)
}

/// The red channel at one point of a square render, `y` from the bottom.
fn red_at(pixels: &[[u8; 4]], size: usize, x: usize, y: usize) -> u8 {
    pixels[y * size + x][0]
}

fn set(g: &mut Graph, node: NodeId, key: &'static str, value: f32) {
    g.get_mut(node)
        .expect("in the graph")
        .controls
        .insert(key, ControlValue::Float(value));
}

fn set_color(g: &mut Graph, node: NodeId, key: &'static str, value: [f32; 4]) {
    g.get_mut(node)
        .expect("in the graph")
        .controls
        .insert(key, ControlValue::Color(value));
}

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, emath::Pos2::ZERO).expect("in the registry")
}

// ------------------------------------------------------------------ every node on the GPU

/// A pipeline made from `shader`'s module the way the renderer makes one, or the driver's
/// refusal.
fn pipeline(gpu: &supersilvia::render::Gpu, shader: &Shader) -> Result<Program, String> {
    Program::create(gpu, shader, wgpu::TextureFormat::Rgba16Float)
}

/// The generated module is only correct if the driver accepts it. A snapshot proves the text
/// is what we meant to write; this proves it is WGSL a pipeline is made from, and that every
/// uniform the compiler promised is a member of the struct the module declares.
#[test]
fn the_generated_checkerboard_shader_compiles_on_the_gpu() {
    let (g, _, out) = gpu::one_node("checkerboard");
    let shader = module_of(&g, out);
    let gpu = gpu::gpu();
    if let Err(e) = pipeline(&gpu, &shader) {
        panic!(
            "the checkerboard did not link:\n{e}\n--- source ---\n{}",
            shader.body
        );
    }
    for name in shader.uniforms.keys() {
        assert!(
            shader.body.contains(&format!("{name}: ")),
            "{name} was registered as a provider but never declared"
        );
    }
}

/// A loop in the graph is the case most likely to emit something that reads well and does not
/// compile: a `mix` of a checkerboard and the Output's own frame.
#[test]
fn the_feedback_shader_compiles_on_the_gpu() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let mix = add(&mut g, "mix");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(mix, "a"))
        .unwrap();
    g.connect(PortRef::new(mix, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.connect(PortRef::new(out, "frame"), PortRef::new(mix, "b"))
        .unwrap();
    let shader = module_of(&g, out);
    let gpu = gpu::gpu();
    if let Err(e) = pipeline(&gpu, &shader) {
        panic!(
            "the feedback module did not link:\n{e}\n--- source ---\n{}",
            shader.body
        );
    }
}

/// The slugs a module holds no WGSL for.
fn untranslated(shader: &Shader) -> BTreeSet<&'static str> {
    shader
        .diagnostics
        .iter()
        .filter_map(|d| match d {
            Diagnostic::Untranslated { slug, .. } => Some(*slug),
            _ => None,
        })
        .collect()
}

/// `def` into an Output as `every_node_compiles_on_the_gpu` wires it: every color input fed
/// from a checkerboard, every varying number from a vignette's falloff and every uniform color
/// from a `color` when `wired`, with `combo`'s options set, and every picture it draws — a
/// node with none reaching the Output through an `rgba` — as the roots to cable in turn.
fn wired_graph(
    def: &'static NodeDef,
    combo: &[(&'static str, &'static str)],
    wired: bool,
) -> (Graph, NodeId, Vec<PortRef>) {
    let mut g = Graph::new();
    let under = add(&mut g, def.slug);
    let out = add(&mut g, "output");
    for (key, value) in combo {
        g.get_mut(under)
            .unwrap()
            .options
            .insert(key, (*value).to_string());
    }
    if wired {
        for port in def.inputs {
            let (source, from) = match port.ty {
                PortType::VaryingColor => ("checkerboard", "output"),
                PortType::VaryingNumber => ("vignette", "mask"),
                PortType::UniformColor => ("color", "output"),
                PortType::UniformNumber | PortType::Action => continue,
            };
            let src = add(&mut g, source);
            g.connect(PortRef::new(src, from), PortRef::new(under, port.key))
                .unwrap_or_else(|e| panic!("{}.{}: {e}", def.slug, port.key));
        }
    }
    let mut roots: Vec<PortRef> = def
        .outputs
        .iter()
        .filter(|p| p.ty == PortType::VaryingColor)
        .map(|p| PortRef::new(under, p.key))
        .collect();
    if roots.is_empty() {
        let rgba = add(&mut g, "rgba");
        for (field, channel) in def
            .outputs
            .iter()
            .filter(|p| p.ty == PortType::VaryingNumber)
            .zip(["r", "g", "b", "a"])
        {
            g.connect(PortRef::new(under, field.key), PortRef::new(rgba, channel))
                .unwrap_or_else(|e| panic!("{}.{}: {e}", def.slug, field.key));
        }
        if g.source_of(PortRef::new(rgba, "r")).is_some() {
            roots.push(PortRef::new(rgba, "output"));
        }
    }
    (g, out, roots)
}

/// Every option combination of `def`, since options change generated code; free text changes
/// none and its default stands in for it.
fn combos(def: &'static NodeDef) -> Vec<Vec<(&'static str, &'static str)>> {
    let mut combos: Vec<Vec<(&'static str, &'static str)>> = vec![vec![]];
    for option in def.options {
        if option.is_asset() {
            continue;
        }
        combos = combos
            .into_iter()
            .flat_map(|base| {
                option.choices.iter().map(move |(value, _)| {
                    let mut next = base.clone();
                    next.push((option.key, *value));
                    next
                })
            })
            .collect();
    }
    combos
}

/// **Every node in the registry makes a pipeline on the real GPU**, in every option
/// combination, with its inputs both connected and unconnected, and every picture it draws
/// reached: `compile::wgsl::build`'s module through `Program::create`, the renderer's own
/// creation, whose error scope holds the driver's message. A typo in a body is a failure with
/// naga's or the driver's own words rather than a black frame discovered mid-set.
///
/// A node whose WGSL is not written yet is skipped and named — `slimemold`, lane 5d's — so this
/// holds every other node while that lane lands. Two builds with one source make one pipeline.
#[test]
fn every_node_compiles_on_the_gpu() {
    let gpu = gpu::gpu();
    let mut checked = 0;
    let mut seen = HashSet::new();
    let mut not_yet = BTreeSet::new();
    for def in nodes::REGISTRY {
        for wired in [false, true] {
            for combo in &combos(def) {
                let (mut g, out, roots) = wired_graph(def, combo, wired);
                for root in roots {
                    g.connect(root, PortRef::new(out, "input")).unwrap();
                    let shader =
                        wgsl::build(&g, out).unwrap_or_else(|| panic!("{}: no module", def.slug));
                    let missing = untranslated(&shader);
                    if !missing.is_empty() {
                        not_yet.extend(missing);
                        continue;
                    }
                    assert!(
                        shader.diagnostics.is_empty(),
                        "{}.{} (options {combo:?}, wired {wired}): {:?}",
                        def.slug,
                        root.key,
                        shader.diagnostics
                    );
                    checked += 1;
                    if !seen.insert(shader.body.clone()) {
                        continue;
                    }
                    if let Err(e) = pipeline(&gpu, &shader) {
                        panic!(
                            "{}.{} (options {combo:?}, wired {wired}) did not link:\n{e}\n\
                             --- source ---\n{}",
                            def.slug, root.key, shader.body
                        );
                    }
                }
            }
        }
    }
    println!(
        "made {} pipelines for {checked} modules across {} nodes; no WGSL yet: {not_yet:?}",
        seen.len(),
        nodes::REGISTRY.len()
    );
    assert!(
        checked > 20,
        "expected to have exercised rather more than {checked}"
    );
    assert!(
        not_yet.iter().all(|s| *s == "slimemold"),
        "only lane 5d's node may be missing its WGSL: {not_yet:?}"
    );
}

/// **And every node draws a frame through the renderer**, unconnected and connected, as an
/// Output of its own on one renderer: bound by `wgsl::bindings` alone, a texture nothing
/// publishes reading black, one tick submitted and finished with no validation error — which
/// the harness's device turns into a panic — and every Output with a picture to show.
#[test]
fn every_node_draws_a_frame_through_the_renderer() {
    let mut graphs = Vec::new();
    for def in nodes::REGISTRY {
        for wired in [false, true] {
            let (mut g, out, roots) = wired_graph(def, &[], wired);
            let Some(root) = roots.first() else {
                continue;
            };
            g.connect(*root, PortRef::new(out, "input")).unwrap();
            let shader = wgsl::build(&g, out).expect("connected");
            if !untranslated(&shader).is_empty() {
                continue;
            }
            let values = resolve(&g, &shader, &|_| 0.5);
            graphs.push((def.slug, Arc::new(shader), values));
        }
    }
    // One renderer, every graph an Output of its own at an id nothing else uses.
    let ids: Vec<NodeId> = (0..graphs.len())
        .map(|i| NodeId(u32::try_from(10_000 + i).unwrap()))
        .collect();
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode: OutputMode| {
        graphs
            .iter()
            .zip(&ids)
            .map(|((_, shader, values), id)| job(*id, (32, 32), shader, send, mode, values.clone()))
            .collect::<Vec<_>>()
    };
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.25, outputs(false, DRAW)));
    drain(&gpu);
    let published = renderer.publish();
    for ((slug, _, _), id) in graphs.iter().zip(&ids) {
        assert!(
            published.outputs.contains_key(id),
            "{slug}: drawn and published"
        );
    }
    println!("drew {} graphs through one renderer", graphs.len());
}

// -------------------------------------------------------------- a mask is what it says

/// A circle's `mask` is 1 where the circle is and 0 where it is not: at the default radius of
/// half a unit in a two-unit-tall world, the center is inside and every corner well outside.
#[test]
fn a_circles_mask_is_one_at_its_center_and_zero_at_the_corners() {
    const SIZE: usize = 64;
    let (g, out) = field_graph("circle", "mask");
    let pixels = rendered(&g, out, SIZE as u32);
    assert_eq!(
        red_at(&pixels, SIZE, SIZE / 2, SIZE / 2),
        255,
        "the center of a circle is inside it"
    );
    for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
        assert_eq!(
            red_at(&pixels, SIZE, x, y),
            0,
            "the corner at ({x}, {y}) is outside a circle of radius 0.5"
        );
    }
}

/// A Mandelbrot's `mask` is the set: 1 at the origin, 0 out where the orbit escapes. The
/// default center of -0.5 puts the origin deep inside the main cardioid.
/// The graph a time-driven node is drawn through: `slug` into an Output by `root`, a
/// checkerboard in its picture input where it has one, and its Offset knob at `offset` — each
/// axis's, on a node with a Time per axis.
fn timed(slug: &'static str, root: &'static str, offset: f32) -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let under = add(&mut g, slug);
    let out = add(&mut g, "output");
    if nodes::find(slug).unwrap().input("input").is_some() {
        let cb = add(&mut g, "checkerboard");
        g.connect(PortRef::new(cb, "output"), PortRef::new(under, "input"))
            .unwrap();
    }
    g.connect(PortRef::new(under, root), PortRef::new(out, "input"))
        .unwrap();
    for key in [nodes::timing::OFFSET, nodes::timing::OFFSET_Y] {
        if nodes::find(slug).unwrap().input(key).is_some() {
            g.get_mut(under)
                .unwrap()
                .controls
                .insert(key, supersilvia::graph::ControlValue::Float(offset));
        }
    }
    (g, out, under)
}

/// How many pixels of two pictures differ by more than two levels in some channel.
fn differ(a: &[[u8; 4]], b: &[[u8; 4]]) -> usize {
    a.iter()
        .zip(b)
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p.abs_diff(*q) > 2))
        .count()
}

/// **Offset adds, either way.** A node whose Time reads 0.3 with its Offset at 0.4 draws what
/// one whose Time reads 0.7 draws with no Offset, and not what 0.3 alone draws: the two are one
/// sum, in the node's own cycles. An Offset of −0.4 on 0.7 is 0.3 the same way. Held on a
/// noise, a palette, the two transforms that keep time and the tunnel.
#[test]
fn offset_adds_to_time() {
    const SIZE: u32 = 64;
    for (slug, root) in [
        ("perlin", "color"),
        ("cosinegradient", "output"),
        ("rotozoom", "output"),
        ("shakycam", "output"),
        ("tunnel3d", "output"),
    ] {
        let draw = |time: f32, offset: f32| {
            let (g, out, under) = timed(slug, root, offset);
            rendered_publishing(&g, out, SIZE, &move |port| {
                if port.node == under && nodes::is_time(port.key) {
                    time
                } else {
                    0.0
                }
            })
        };
        let summed = draw(0.3, 0.4);
        let whole = draw(0.7, 0.0);
        let alone = draw(0.3, 0.0);
        let pixels = summed.len();
        assert!(
            differ(&summed, &whole) * 1000 <= pixels,
            "{slug}: 0.3 and an Offset of 0.4 is 0.7, but {} of {pixels} pixels differ",
            differ(&summed, &whole)
        );
        assert!(
            differ(&summed, &alone) * 20 > pixels,
            "{slug}: and the Offset moved it"
        );
        let back = draw(0.7, -0.4);
        assert!(
            differ(&back, &alone) * 1000 <= pixels,
            "{slug}: 0.7 and an Offset of −0.4 is 0.3, but {} of {pixels} pixels differ",
            differ(&back, &alone)
        );
    }
}

/// **A field into Offset is a ripple, with one cable.** The distance from the middle cabled
/// into a Cosine Gradient's Offset puts each ring of the picture at its own place in the
/// gradient's cycle: two points the same distance out are the same colour, and the middle
/// and a point a quarter of a unit out — a quarter of the gradient's cycle — are not. One
/// number in the same place colours the whole frame one colour.
#[test]
fn a_field_into_offset_is_a_ripple() {
    const SIZE: usize = 64;
    let draw = |field: bool| {
        let mut g = Graph::new();
        let grad = add(&mut g, "cosinegradient");
        let out = add(&mut g, "output");
        if field {
            let world = add(&mut g, "worldcoordinates");
            let radius = add(&mut g, "pythagorean");
            g.connect(PortRef::new(world, "x"), PortRef::new(radius, "a"))
                .unwrap();
            g.connect(PortRef::new(world, "y"), PortRef::new(radius, "b"))
                .unwrap();
            g.connect(
                PortRef::new(radius, "output"),
                PortRef::new(grad, nodes::timing::OFFSET),
            )
            .expect("a field feeds Offset");
        } else {
            let number = add(&mut g, "number");
            g.connect(
                PortRef::new(number, "output"),
                PortRef::new(grad, nodes::timing::OFFSET),
            )
            .unwrap();
        }
        g.connect(PortRef::new(grad, "output"), PortRef::new(out, "input"))
            .unwrap();
        rendered_publishing(&g, out, SIZE as u32, &|_| 0.35)
    };
    let pixels = draw(true);
    let at = |x: usize, y: usize| pixels[y * SIZE + x];
    let c = SIZE / 2;
    // The world is two units tall, so a pixel is 2 / 64 of a unit.
    let q = SIZE / 4;
    // A pixel's center is half a pixel past its corner, so the mirror of `c + q` is `c - 1 - q`.
    assert_eq!(
        at(c + q, c),
        at(c - 1 - q, c),
        "the same distance, the same colour"
    );
    assert_eq!(at(c + q, c), at(c, c + q), "whichever way out");
    assert_ne!(
        at(c, c),
        at(c + q / 2, c),
        "a quarter of a cycle out is not the middle"
    );
    let flat = draw(false);
    assert!(
        flat.iter().all(|p| *p == flat[0]),
        "one number is one colour"
    );
    assert!(
        pixels.iter().any(|p| *p != pixels[0]),
        "and the field is not"
    );
}

/// **A gear in Time is the node at the gear's reading.** A Ratio Gear's Cycles cabled into a
/// node's Time draw the frame the node draws at that reading unplugged: Time is one number,
/// the moment the node is at, wherever it comes from.
#[test]
fn a_gear_in_time_is_the_node_at_the_gears_reading() {
    const SIZE: u32 = 64;
    for (slug, root) in [("cosinegradient", "output"), ("rotozoom", "output")] {
        let reading: f32 = 0.37;
        let (mut g, out, under) = timed(slug, root, 0.0);
        g.get_mut(under)
            .unwrap()
            .options
            .insert(nodes::timing::MODE.key, nodes::timing::LOOP.to_string());
        let gear = add(&mut g, "ratiogear");
        g.connect(
            PortRef::new(gear, "cycles"),
            PortRef::new(under, nodes::TIME),
        )
        .expect("a gear's Cycles feed Time");
        let driven = rendered_publishing(&g, out, SIZE, &move |port| {
            if port.node == gear && port.key == "cycles" {
                reading
            } else {
                0.0
            }
        });
        let (g, out, under) = timed(slug, root, 0.0);
        let own = rendered_publishing(&g, out, SIZE, &move |port| {
            if port.node == under && port.key == nodes::TIME {
                reading
            } else {
                0.0
            }
        });
        assert_eq!(
            differ(&driven, &own),
            0,
            "{slug}: a gear reading {reading} is its Time at {reading}"
        );
    }
}

/// **The tunnel comes back every 64 units.** Its path is retuned so Sine, Lissajous and the
/// Helix repeat inside 64 units of depth, a whole number of both depth wraps, so its Time at
/// 64 draws what its Time at zero draws, and five units on does not.
#[test]
fn the_tunnel_comes_back_after_64() {
    const SIZE: u32 = 64;
    for path in ["sine", "helix", "lissajous"] {
        for wrap in ["mirror", "repeat"] {
            let draw = |time: f32| {
                let (mut g, out, under) = timed("tunnel3d", "output", 0.0);
                let node = g.get_mut(under).unwrap();
                node.options.insert("path", path.to_string());
                node.options.insert("wrap", wrap.to_string());
                rendered_publishing(&g, out, SIZE, &move |port| {
                    if port.node == under && port.key == nodes::TIME {
                        time
                    } else {
                        0.0
                    }
                })
            };
            let (start, round, half) = (draw(0.0), draw(64.0), draw(5.0));
            let pixels = start.len();
            assert!(
                differ(&start, &round) * 1000 <= pixels,
                "{path} {wrap}: 64 units on is the start, but {} of {pixels} differ",
                differ(&start, &round)
            );
            assert!(
                differ(&start, &half) * 20 > pixels,
                "{path} {wrap}: and five units on is not"
            );
        }
    }
}

/// **Static under Repeat comes back to the bit with a field in its Offset.** It takes its Time
/// round the Repeat before it adds Offset, as every node with a period does, so a Time of
/// 1024.5 draws what a Time of 0.5 draws at Repeat 4 with Offsets a hair under a half in it,
/// where adding first rounds `1024.5 + Offset` up to the next roll; a roll on is another
/// picture.
#[test]
fn static_under_repeat_comes_back_with_a_field_in_its_offset() {
    const SIZE: u32 = 64;
    let draw = |time: f32| {
        let (mut g, out, under) = timed("static", "color", 0.0);
        g.get_mut(under)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        // Offset 0.5 + x / 10⁴: across the frame, within a ten-thousandth of a half.
        let world = add(&mut g, "worldcoordinates");
        let scale = add(&mut g, "multiply");
        let shift = add(&mut g, "add");
        set(&mut g, scale, "b", 1e-4);
        set(&mut g, shift, "b", 0.5);
        for (from, to) in [
            (PortRef::new(world, "x"), PortRef::new(scale, "a")),
            (PortRef::new(scale, "output"), PortRef::new(shift, "a")),
            (
                PortRef::new(shift, "output"),
                PortRef::new(under, nodes::timing::OFFSET),
            ),
        ] {
            g.connect(from, to).unwrap();
        }
        rendered_publishing(&g, out, SIZE, &move |port| {
            if port.node == under && port.key == nodes::TIME {
                time
            } else {
                0.0
            }
        })
    };
    let (start, round, next) = (draw(0.5), draw(1024.5), draw(1.5));
    assert_eq!(
        differ(&start, &round),
        0,
        "256 Repeats on is the same picture"
    );
    assert!(differ(&start, &next) > 0, "and a roll on is not");
}

/// **A negative Offset comes back with the period too.** Time is taken round the period
/// before Offset is added, whatever its sign, so a Perlin at Repeat 4 with its Offset at −2.7
/// draws at a Time of 1024.5 exactly what it draws at 0.5, as does the tunnel at −5.3 a flight
/// of 64 on, and Static at Repeat 4 with a field a hair over −0.5 in its Offset. An Offset of
/// −1.5 at Repeat 4 is 2.5 there: one period apart, the same picture.
#[test]
fn a_negative_offset_comes_back_with_the_period() {
    const SIZE: u32 = 64;
    let draw = |slug: &'static str, root, options: &[(&'static str, &str)], offset, time| {
        let (mut g, out, under) = timed(slug, root, offset);
        for (key, value) in options {
            g.get_mut(under)
                .unwrap()
                .options
                .insert(key, (*value).to_string());
        }
        rendered_publishing(&g, out, SIZE, &move |port| {
            if port.node == under && port.key == nodes::TIME {
                time
            } else {
                0.0
            }
        })
    };
    let four = [("repeat", "4")];
    let start = draw("perlin", "color", &four, -2.7, 0.5);
    assert_eq!(
        differ(&start, &draw("perlin", "color", &four, -2.7, 1024.5)),
        0,
        "perlin: 256 Repeats on is the same picture"
    );
    assert!(
        differ(&start, &draw("perlin", "color", &four, -2.7, 1.5)) > 0,
        "perlin: and a cell on is not"
    );
    let tunnel = draw("tunnel3d", "output", &[], -5.3, 0.0);
    assert_eq!(
        differ(&tunnel, &draw("tunnel3d", "output", &[], -5.3, 64.0)),
        0,
        "tunnel: a flight on is the same picture"
    );
    let below = draw("perlin", "color", &four, -1.5, 0.25);
    let above = draw("perlin", "color", &four, 2.5, 0.25);
    assert!(
        differ(&below, &above) * 1000 <= below.len(),
        "−1.5 is 2.5 round 4, but {} pixels differ",
        differ(&below, &above)
    );

    let field = |time: f32| {
        let (mut g, out, under) = timed("static", "color", 0.0);
        g.get_mut(under)
            .unwrap()
            .options
            .insert("repeat", "4".to_string());
        // Offset −0.5 + x / 10⁴: across the frame, within a ten-thousandth of a half back.
        let world = add(&mut g, "worldcoordinates");
        let scale = add(&mut g, "multiply");
        let shift = add(&mut g, "add");
        set(&mut g, scale, "b", 1e-4);
        set(&mut g, shift, "b", -0.5);
        for (from, to) in [
            (PortRef::new(world, "x"), PortRef::new(scale, "a")),
            (PortRef::new(scale, "output"), PortRef::new(shift, "a")),
            (
                PortRef::new(shift, "output"),
                PortRef::new(under, nodes::timing::OFFSET),
            ),
        ] {
            g.connect(from, to).unwrap();
        }
        rendered_publishing(&g, out, SIZE, &move |port| {
            if port.node == under && port.key == nodes::TIME {
                time
            } else {
                0.0
            }
        })
    };
    let (start, round, next) = (field(0.5), field(1024.5), field(1.5));
    assert_eq!(
        differ(&start, &round),
        0,
        "static: 256 Repeats on is the same"
    );
    assert!(differ(&start, &next) > 0, "static: and a roll on is not");
}

#[test]
fn a_mandelbrots_mask_is_one_inside_the_set() {
    const SIZE: usize = 64;
    let (g, out) = field_graph("mandelbrot", "mask");
    let pixels = rendered(&g, out, SIZE as u32);
    assert_eq!(
        red_at(&pixels, SIZE, SIZE / 2, SIZE / 2),
        255,
        "the origin maps to c = -0.5, which never escapes"
    );
    for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
        assert_eq!(
            red_at(&pixels, SIZE, x, y),
            0,
            "the corner at ({x}, {y}) escapes"
        );
    }
}

/// A region's background is behind the input as well as around it: a half-transparent red
/// over opaque blue is half red and half blue, opaque.
/// Over the default transparent background the input is untouched, which is what silvia's
/// `mix(bg, input, mask)` drew and what every patch already relies on.
#[test]
fn a_regions_background_shows_through_a_transparent_input() {
    const SIZE: usize = 64;
    let draw = |background: Option<[f32; 4]>| {
        let mut g = Graph::new();
        let color = add(&mut g, "rgba");
        let region = add(&mut g, "regionabsolute");
        let out = add(&mut g, "output");
        for (channel, value) in [("r", 1.0), ("g", 0.0), ("b", 0.0), ("a", 0.5)] {
            set(&mut g, color, channel, value);
        }
        set(&mut g, region, "left", -0.5);
        set(&mut g, region, "right", 0.5);
        if let Some(bg) = background {
            set_color(&mut g, region, "bgColor", bg);
        }
        g.connect(PortRef::new(color, "output"), PortRef::new(region, "input"))
            .unwrap();
        g.connect(PortRef::new(region, "output"), PortRef::new(out, "input"))
            .unwrap();
        let pixels = rendered(&g, out, SIZE as u32);
        let at = |x: usize| pixels[(SIZE / 2) * SIZE + x];
        (at(SIZE / 2), at(0))
    };

    let (inside, outside) = draw(Some([0.0, 0.0, 1.0, 1.0]));
    assert_eq!(
        inside,
        [128, 0, 128, 255],
        "the blue shows through the red's other half"
    );
    assert_eq!(
        outside,
        [0, 0, 255, 255],
        "outside the rectangle is the background"
    );

    let (inside, outside) = draw(None);
    assert_eq!(
        inside,
        [128, 0, 0, 128],
        "nothing behind the input leaves it as it was"
    );
    assert_eq!(outside, [0, 0, 0, 0], "and outside is transparent");
}

// --------------------------------------------------- a distortion moves, a noise is a field

/// A tile's picture repeats at exactly the period its width says: a width of 0.75 in a
/// two-unit-tall world is three eighths of the frame, over a checkerboard whose own period of
/// half a unit is deliberately not a multiple of it.
#[test]
fn a_tile_repeats_its_period() {
    const SIZE: usize = 64;
    const WIDTH: f32 = 0.75;
    const PERIOD: usize = 24;

    let mut g = Graph::new();
    let source = add(&mut g, "checkerboard");
    let tile = add(&mut g, "tile");
    let out = add(&mut g, "output");
    set(&mut g, tile, "width", WIDTH);
    set(&mut g, tile, "height", WIDTH);
    g.connect(PortRef::new(source, "output"), PortRef::new(tile, "input"))
        .unwrap();
    g.connect(PortRef::new(tile, "output"), PortRef::new(out, "input"))
        .unwrap();

    let pixels = rendered(&g, out, SIZE as u32);
    let row = SIZE / 2;
    let mut seen = HashSet::new();
    for x in 0..SIZE - PERIOD {
        let here = red_at(&pixels, SIZE, x, row);
        let there = red_at(&pixels, SIZE, x + PERIOD, row);
        assert_eq!(
            here,
            there,
            "the pixel at {x} and the one at {} are one period apart",
            x + PERIOD
        );
        seen.insert(here);
    }
    assert!(
        seen.len() > 1,
        "a row of one color would satisfy any period"
    );
}

/// A Perlin noise's `value` is a field: many levels across the frame, most of them between
/// its ends, and two points a lattice cell apart differ.
#[test]
fn a_perlins_value_is_a_field_between_zero_and_one() {
    const SIZE: usize = 64;
    let (g, out) = field_graph("perlin", "value");
    let pixels = rendered(&g, out, SIZE as u32);
    let mut seen = HashSet::new();
    for y in (0..SIZE).step_by(4) {
        for x in (0..SIZE).step_by(4) {
            seen.insert(red_at(&pixels, SIZE, x, y));
        }
    }
    assert!(
        seen.len() > 8,
        "a noise field takes many values across the frame, not {}",
        seen.len()
    );
    assert!(
        seen.iter().any(|v| *v > 0 && *v < 255),
        "and most of them are between its ends"
    );
    let a = red_at(&pixels, SIZE, SIZE / 4, SIZE / 2);
    let b = red_at(&pixels, SIZE, SIZE / 2, SIZE / 2);
    assert_ne!(a, b, "two points of a noise field differ");
}

// ------------------------------------------------- the unglamorous four, on the GPU

/// A field that is `value` at every point: an `rgba`'s red channel, read back out by a `red`.
/// Two nodes because every `math` node is dual, so a knob on one publishes a uniform number;
/// this is genuinely a field.
fn constant_field(g: &mut Graph, value: f32) -> PortRef {
    let rgba = add(g, "rgba");
    set(g, rgba, "r", value);
    let red = add(g, "red");
    g.connect(PortRef::new(rgba, "output"), PortRef::new(red, "input"))
        .expect("a color into a conversion");
    PortRef::new(red, "output")
}

/// `invert` of white is black, and of black is white.
#[test]
fn inverting_white_reads_black() {
    const SIZE: usize = 8;
    for (input, expected) in [(1.0_f32, 0_u8), (0.0, 255)] {
        let mut g = Graph::new();
        let flat = add(&mut g, "checkerboard");
        let invert = add(&mut g, "invert");
        let out = add(&mut g, "output");
        for key in ["color1", "color2"] {
            set_color(&mut g, flat, key, [input, input, input, 1.0]);
        }
        g.connect(PortRef::new(flat, "output"), PortRef::new(invert, "input"))
            .unwrap();
        g.connect(PortRef::new(invert, "output"), PortRef::new(out, "input"))
            .unwrap();
        let pixels = rendered(&g, out, SIZE as u32);
        for (x, y) in [(0, 0), (SIZE / 2, SIZE / 2), (SIZE - 1, SIZE - 1)] {
            assert_eq!(
                red_at(&pixels, SIZE, x, y),
                expected,
                "inverting {input} at ({x}, {y})"
            );
        }
    }
}

/// `reframerange` maps 0.5 in [0, 1] onto [0, 10] and reads 5, brought back inside a color
/// channel by a `divide` by ten that is not the node under test.
#[test]
fn reframerange_maps_a_half_onto_the_middle_of_its_output_range() {
    const SIZE: usize = 8;
    let mut g = Graph::new();
    let under = add(&mut g, "reframerange");
    let scale = add(&mut g, "divide");
    let rgba = add(&mut g, "rgba");
    let out = add(&mut g, "output");
    for (key, value) in [
        ("inMin", 0.0),
        ("inMax", 1.0),
        ("outMin", 0.0),
        ("outMax", 10.0),
    ] {
        set(&mut g, under, key, value);
    }
    set(&mut g, scale, "b", 10.0);
    let half = constant_field(&mut g, 0.5);
    g.connect(half, PortRef::new(under, "input")).unwrap();
    g.connect(PortRef::new(under, "output"), PortRef::new(scale, "a"))
        .unwrap();
    for channel in ["r", "g", "b"] {
        g.connect(PortRef::new(scale, "output"), PortRef::new(rgba, channel))
            .unwrap();
    }
    g.connect(PortRef::new(rgba, "output"), PortRef::new(out, "input"))
        .unwrap();
    let pixels = rendered(&g, out, SIZE as u32);
    let mapped = f32::from(red_at(&pixels, SIZE, SIZE / 2, SIZE / 2)) / 255.0 * 10.0;
    assert!(
        (mapped - 5.0).abs() < 0.05,
        "0.5 of [0, 1] onto [0, 10] is 5, not {mapped}"
    );
}

/// `channelsplitter`'s four ports are the four channels of a color asymmetric in every one,
/// built by an `rgba` from the same four numbers.
#[test]
fn a_channel_splitter_reads_the_four_channels_it_was_given() {
    const SIZE: usize = 8;
    const GIVEN: [f32; 4] = [0.8, 0.6, 0.4, 0.2];
    for (i, expected) in GIVEN.iter().enumerate() {
        let channel = ["r", "g", "b", "a"][i];
        let mut g = Graph::new();
        let flat = add(&mut g, "rgba");
        let split = add(&mut g, "channelsplitter");
        let rgba = add(&mut g, "rgba");
        let out = add(&mut g, "output");
        for (key, value) in ["r", "g", "b", "a"].into_iter().zip(GIVEN) {
            set(&mut g, flat, key, value);
        }
        g.connect(PortRef::new(flat, "output"), PortRef::new(split, "input"))
            .unwrap();
        g.connect(PortRef::new(split, channel), PortRef::new(rgba, "r"))
            .unwrap();
        g.connect(PortRef::new(rgba, "output"), PortRef::new(out, "input"))
            .unwrap();
        let pixels = rendered(&g, out, SIZE as u32);
        let read = f32::from(red_at(&pixels, SIZE, SIZE / 2, SIZE / 2)) / 255.0;
        assert!(
            (read - expected).abs() < 0.01,
            "the {channel} port of {GIVEN:?} read {read}"
        );
    }
}

/// The phyllotaxis loop visits only the seeds whose ring passes under the pixel, so the render
/// is held to the plain definition — every seed, nearest wins — computed on the CPU.
#[test]
fn a_phyllotaxis_banded_by_ring_matches_every_seed_tested() {
    const SIZE: usize = 128;
    let (count, angle, max_r, dot) = (137.0_f32, 0.382_f32, 0.9_f32, 0.06_f32);
    let mut g = Graph::new();
    let p = add(&mut g, "phyllotaxis");
    let out = add(&mut g, "output");
    set(&mut g, p, "count", count);
    set(&mut g, p, "angle", angle);
    set(&mut g, p, "radius", max_r);
    set(&mut g, p, "dotSize", dot);
    set_color(&mut g, p, "fg", [0.0, 0.0, 0.0, 1.0]);
    set_color(&mut g, p, "bg", [1.0, 1.0, 1.0, 1.0]);
    g.connect(PortRef::new(p, "output"), PortRef::new(out, "input"))
        .unwrap();
    let px = rendered(&g, out, SIZE as u32);

    let seeds: Vec<(f32, f32)> = (0..count as usize)
        .map(|i| {
            let t = i as f32 / (count - 1.0);
            let r = max_r * t.sqrt();
            let a = i as f32 * angle * std::f32::consts::TAU;
            (r * a.cos(), r * a.sin())
        })
        .collect();
    let smoothstep = |e0: f32, e1: f32, x: f32| {
        let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut worst = 0i32;
    let mut lit = 0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let u = (x as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / SIZE as f32 * 2.0 - 1.0;
            let min = seeds
                .iter()
                .map(|(sx, sy)| ((u - sx).powi(2) + (v - sy).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            let expect = (1.0 - smoothstep(dot, dot * 0.5, min)) * 255.0;
            let got = f32::from(red_at(&px, SIZE, x, y));
            worst = worst.max((got - expect).abs() as i32);
            if got < 128.0 {
                lit += 1;
            }
        }
    }
    assert!(
        worst <= 8,
        "a pixel is {worst} levels off the every-seed answer"
    );
    assert!(lit > 200, "the dots are there: {lit} dark pixels");
}

// ------------------------------------------------------------------------------------ taps

/// What the pass measuring `g`'s taps read back after `frames` ticks, `out` drawn at `size`
/// beside it. See `gpu::pass_words`.
fn tap_words_at(g: &Graph, out: NodeId, frames: usize, size: (u32, u32)) -> Vec<u32> {
    gpu::pass_words(g, out, frames, size)
}

fn tap_words(g: &Graph, out: NodeId, frames: usize) -> Vec<u32> {
    tap_words_at(g, out, frames, (64, 64))
}

/// A grid of 64 keeps the pass these draw small; the unit square is measured whatever it is.
fn set_grid(g: &mut Graph, t: NodeId, grid: &str) {
    g.get_mut(t)
        .expect("the tap is in the graph")
        .options
        .insert("grid", grid.to_string());
}

fn slot(words: Vec<u32>) -> [u32; TAP_WORDS] {
    assert_eq!(words.len(), TAP_WORDS, "one slot");
    words.try_into().unwrap()
}

/// `input` into a tap on a grid of 64, the tap into an Output: the tap and the Output.
fn tapped(g: &mut Graph, input: PortRef) -> (NodeId, NodeId) {
    let t = add(g, "tap");
    set_grid(g, t, "64");
    let out = add(g, "output");
    g.connect(input, PortRef::new(t, "input")).unwrap();
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();
    (t, out)
}

/// A float plugged into `number` is what the tap measures, whatever the picker says: a `blue`
/// node reading a solid color, sidechained into a tap set to `red`, reads the blue.
#[test]
fn a_sidechained_tap_measures_the_field_and_not_its_picker() {
    let mut g = Graph::new();
    let solid = add(&mut g, "rgba");
    for (key, value) in [("r", 0.25f32), ("g", 0.5), ("b", 0.75)] {
        set(&mut g, solid, key, value);
    }
    let blue = add(&mut g, "blue");
    let (t, out) = tapped(&mut g, PortRef::new(solid, "output"));
    g.connect(PortRef::new(solid, "output"), PortRef::new(blue, "input"))
        .unwrap();
    g.connect(PortRef::new(blue, "output"), PortRef::new(t, "number"))
        .expect("a varying number into the sidechain");
    g.get_mut(t)
        .unwrap()
        .options
        .insert("measure", "red".to_string());

    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64);
    assert!((s.mean - 0.75).abs() < 1e-3, "mean {}", s.mean);
    assert_eq!(s.max, 0.75, "the cable won, not the picker's red");
    assert_eq!(s.min, 0.75);
}

/// A level that is not a multiple of the old 1/32 reads back as itself: the sums keep 1/65536
/// of a unit, so 0.51 is 0.51.
#[test]
fn a_tap_reads_a_level_between_the_coarse_steps() {
    let mut g = Graph::new();
    let gray = add(&mut g, "rgba");
    for key in ["r", "g", "b"] {
        set(&mut g, gray, key, 0.51);
    }
    let (_, out) = tapped(&mut g, PortRef::new(gray, "output"));
    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64);
    assert!(
        (s.mean - 0.51).abs() <= 1.0 / tap::SCALE,
        "mean {} is 0.51 to 1/{}",
        s.mean,
        tap::SCALE
    );
}

/// A constant negative field measures negative: the mean, the floor and the peak are all below
/// zero, and with nothing positive the centroid has no weight and sits at the origin.
#[test]
fn a_tap_reads_a_negative_field() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let sub = add(&mut g, "subtract");
    let zero = constant_field(&mut g, 0.0);
    g.connect(zero, PortRef::new(sub, "a"))
        .expect("a field into a varying number input");
    set(&mut g, sub, "b", 0.25);
    let (t, out) = tapped(&mut g, PortRef::new(cb, "output"));
    g.connect(PortRef::new(sub, "output"), PortRef::new(t, "number"))
        .expect("a varying number into the sidechain");

    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64);
    assert!((s.mean + 0.25).abs() < 1e-4, "mean {}", s.mean);
    assert_eq!(s.max, -0.25, "the peak of a negative field is negative");
    assert_eq!(s.min, -0.25);
    assert_eq!((s.x, s.y), (0.0, 0.0), "nothing was positive to weight by");
}

/// A field that crosses zero keeps both signs: a checkerboard's luminance less a half is minus
/// a half on one cell and plus a half on the next.
#[test]
fn a_tap_reads_both_signs_of_a_field_that_crosses_zero() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let luma = add(&mut g, "luminosity");
    let sub = add(&mut g, "subtract");
    set(&mut g, sub, "b", 0.5);
    let (t, out) = tapped(&mut g, PortRef::new(cb, "output"));
    g.connect(PortRef::new(cb, "output"), PortRef::new(luma, "input"))
        .unwrap();
    g.connect(PortRef::new(luma, "output"), PortRef::new(sub, "a"))
        .unwrap();
    g.connect(PortRef::new(sub, "output"), PortRef::new(t, "number"))
        .expect("a varying number into the sidechain");

    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64);
    assert_eq!(s.max, 0.5);
    assert_eq!(s.min, -0.5);
    assert!(s.mean.abs() < 1e-3, "the two halves cancel: {}", s.mean);
}

/// **A transform between a tap and its Output does not change the reading.** The tap measures
/// its input over the unit square, so a zoom downstream moves the picture and leaves the
/// numbers alone — on a radial gradient, where measuring at the caller's coordinate would have
/// read the bright middle four times over.
#[test]
fn a_zoom_between_a_tap_and_its_output_does_not_change_the_reading() {
    let mut g = Graph::new();
    let ramp = add(&mut g, "radialgradient");
    let zoom = add(&mut g, "zoom");
    set(&mut g, zoom, "zoom", 4.0);
    let (t, out) = tapped(&mut g, PortRef::new(ramp, "output"));
    let plain = tap_words(&g, out, 2);

    g.disconnect(PortRef::new(out, "input"));
    g.connect(PortRef::new(t, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();
    let zoomed = tap_words(&g, out, 2);

    assert_eq!(plain, zoomed, "the zoom is downstream of the measurement");
    let s = tap::decode(&slot(plain));
    assert_eq!(s.count, 64 * 64);
    assert!(
        (0.1..0.4).contains(&s.mean),
        "the mean of the whole square: {}",
        s.mean
    );
    assert!(
        s.max > 0.95 && s.min < 0.01,
        "both ends are in it: {} to {}",
        s.min,
        s.max
    );
}

/// **Two Outputs of different resolution and aspect read one tap the same.**
#[test]
fn two_outputs_of_different_size_and_aspect_read_one_tap_the_same() {
    let mut g = Graph::new();
    let ramp = add(&mut g, "radialgradient");
    let (t, square) = tapped(&mut g, PortRef::new(ramp, "output"));
    let wide = add(&mut g, "output");
    g.connect(PortRef::new(t, "output"), PortRef::new(wide, "input"))
        .unwrap();
    let a = tap_words_at(&g, square, 2, (256, 256));
    let b = tap_words_at(&g, wide, 2, (320, 180));
    assert_eq!(a, b, "one input, one reading");
    assert_eq!(tap::decode(&slot(a)).count, 64 * 64);
}

/// The sample's `x` and `y` at (0.125, 0.125).
fn point(g: &mut Graph, s: NodeId) {
    set(g, s, "x", 0.125);
    set(g, s, "y", 0.125);
}

/// A checkerboard's control color, exactly.
fn a_control_color(c: [f32; 4]) -> bool {
    c == [1.0, 1.0, 1.0, 1.0] || c == [0.0, 0.0, 0.0, 1.0]
}

/// A sample reads the color at its point with a zoom of sixteen downstream, which hands its
/// input coordinates inside a sixteenth of the frame: the measurement evaluates the input at
/// the point itself, so there is no fragment for it to miss.
#[test]
fn a_sample_reads_its_point_under_a_zoom_that_would_have_missed_it() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let s = add(&mut g, "sample");
    point(&mut g, s);
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(s, "input"))
        .unwrap();
    g.connect(PortRef::new(s, "output"), PortRef::new(out, "input"))
        .unwrap();
    let before = sample::decode(&slot(tap_words(&g, out, 2))).expect("the point was measured");

    let zoom = add(&mut g, "zoom");
    set(&mut g, zoom, "zoom", 16.0);
    g.disconnect(PortRef::new(out, "input"));
    g.connect(PortRef::new(s, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();
    let after = sample::decode(&slot(tap_words(&g, out, 2))).expect("the point is still measured");
    assert_eq!(before, after, "the point is the sample's, not the screen's");
    assert!(
        a_control_color(after),
        "a control color exactly, got {after:?}"
    );
}

/// **A tap on a workspace is measured whether or not it reaches an Output.** A gradient into a
/// tap into nothing, on a workspace whose only Output draws a checkerboard: the workspace's
/// pass measures it, and what comes back is what the same tap cabled into an Output reads, to
/// the bit.
#[test]
fn a_tap_reaching_no_output_reads_its_chain_as_if_it_were_cabled_in() {
    let mut loose = Graph::new();
    let ramp = add(&mut loose, "radialgradient");
    let t = add(&mut loose, "tap");
    set_grid(&mut loose, t, "64");
    let cb = add(&mut loose, "checkerboard");
    let out = add(&mut loose, "output");
    loose
        .connect(PortRef::new(ramp, "output"), PortRef::new(t, "input"))
        .unwrap();
    loose
        .connect(PortRef::new(cb, "output"), PortRef::new(out, "input"))
        .unwrap();

    let mut cabled = Graph::new();
    let ramp2 = add(&mut cabled, "radialgradient");
    let (_, out2) = tapped(&mut cabled, PortRef::new(ramp2, "output"));

    let loose = tap_words(&loose, out, 2);
    let direct = tap_words(&cabled, out2, 2);
    assert_eq!(
        loose, direct,
        "the reading is the input's, whatever the Output beside it draws"
    );
    assert_eq!(t, NodeId(2), "the same id in both, so the slot is the same");
    let s = tap::decode(&slot(loose));
    assert_eq!(s.count, 64 * 64, "every cell of the grid measured once");
    assert!(
        s.mean > 0.2 && s.mean < 0.35 && s.max < 1.0 && s.min == 0.0,
        "the gradient, not the checkerboard the Output drew: {s:?}"
    );
}

/// And a sample the same way: one call at the corner fragment, evaluating its own input at its
/// own point, on a workspace whose Output draws a flat color.
#[test]
fn a_sample_reaching_no_output_reads_its_point_as_if_it_were_cabled_in() {
    let mut loose = Graph::new();
    let cb = add(&mut loose, "checkerboard");
    let s = add(&mut loose, "sample");
    point(&mut loose, s);
    let solid = add(&mut loose, "rgba");
    let out = add(&mut loose, "output");
    loose
        .connect(PortRef::new(cb, "output"), PortRef::new(s, "input"))
        .unwrap();
    loose
        .connect(PortRef::new(solid, "output"), PortRef::new(out, "input"))
        .unwrap();

    let mut cabled = Graph::new();
    let cb2 = add(&mut cabled, "checkerboard");
    let s2 = add(&mut cabled, "sample");
    point(&mut cabled, s2);
    let out2 = add(&mut cabled, "output");
    cabled
        .connect(PortRef::new(cb2, "output"), PortRef::new(s2, "input"))
        .unwrap();
    cabled
        .connect(PortRef::new(s2, "output"), PortRef::new(out2, "input"))
        .unwrap();

    let loose = tap_words(&loose, out, 2);
    let direct = tap_words(&cabled, out2, 2);
    assert_eq!(loose, direct, "one point, one reading");
    let color = sample::decode(&slot(loose)).expect("the point was measured");
    assert!(
        a_control_color(color),
        "a checkerboard color exactly, and not the solid the Output drew: {color:?}"
    );
}

/// The tap's mean color is what its input looks like: a checkerboard of pure red and pure blue
/// over an even grid averages to half of each.
#[test]
fn a_tap_measures_the_mean_color_of_its_input() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let t = add(&mut g, "tap");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(t, "input"))
        .unwrap();
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();
    set_color(&mut g, cb, "color1", [1.0, 0.0, 0.0, 1.0]);
    set_color(&mut g, cb, "color2", [0.0, 0.0, 1.0, 1.0]);

    let stats = tap::decode(&slot(tap_words(&g, out, 2)));
    assert!(stats.count > 0, "the measurement ran");
    let [r, g_, b, a] = stats.color;
    assert!((r - 0.5).abs() < 0.02, "red {r}, color {:?}", stats.color);
    assert!(g_.abs() < 0.02, "green {g_}, color {:?}", stats.color);
    assert!((b - 0.5).abs() < 0.02, "blue {b}, color {:?}", stats.color);
    assert_eq!(a, 1.0, "the mean color is opaque");
}

// -------------------------------------------------------------------------- dual outputs

/// The dual family, and three sets of control values each. The first input listed is the one
/// a constant field drives, so its value is a color channel, 0 to 1; everything else is a
/// knob, inside its declared range.
type DualCase = (&'static str, &'static [&'static [(&'static str, f32)]]);

const DUAL_CASES: &[DualCase] = &[
    (
        "add",
        &[
            &[("a", 0.25), ("b", 0.5)],
            &[("a", 0.75), ("b", -2.5)],
            &[("a", 0.0), ("b", 0.0)],
        ],
    ),
    (
        "subtract",
        &[
            &[("a", 0.25), ("b", 0.75)],
            &[("a", 1.0), ("b", -4.0)],
            &[("a", 0.5), ("b", 0.5)],
        ],
    ),
    (
        "multiply",
        &[
            &[("a", 0.5), ("b", 3.0)],
            &[("a", 0.25), ("b", -8.0)],
            &[("a", 0.125), ("b", 0.0)],
        ],
    ),
    (
        "divide",
        &[
            &[("a", 0.5), ("b", 4.0)],
            &[("a", 0.75), ("b", -0.5)],
            &[("a", 0.25), ("b", 0.0)],
        ],
    ),
    (
        "min",
        &[
            &[("a", 0.5), ("b", 3.0)],
            &[("a", 0.5), ("b", -3.0)],
            &[("a", 0.0), ("b", 0.0)],
        ],
    ),
    (
        "max",
        &[
            &[("a", 0.5), ("b", 3.0)],
            &[("a", 0.5), ("b", -3.0)],
            &[("a", 1.0), ("b", 1.0)],
        ],
    ),
    (
        "abs",
        &[&[("input", 0.25)], &[("input", 0.75)], &[("input", 0.0)]],
    ),
    (
        "ceil",
        &[&[("input", 0.25)], &[("input", 1.0)], &[("input", 0.0)]],
    ),
    (
        "floor",
        &[&[("input", 0.25)], &[("input", 1.0)], &[("input", 0.0)]],
    ),
    (
        "atan2",
        &[
            &[("x", 0.5), ("y", 0.5)],
            &[("x", 0.25), ("y", -3.0)],
            &[("x", 0.0), ("y", 1.0)],
        ],
    ),
    (
        "lerp",
        &[
            &[("a", 0.25), ("b", 4.0), ("t", 0.5)],
            &[("a", 0.75), ("b", -2.0), ("t", 0.0)],
            &[("a", 0.5), ("b", 1.0), ("t", 1.0)],
        ],
    ),
    (
        "modulo",
        &[
            &[("a", 0.75), ("b", 0.5)],
            &[("a", 0.75), ("b", -0.5)],
            &[("a", 0.25), ("b", 0.0)],
        ],
    ),
    (
        "power",
        &[
            &[("base", 0.5), ("exponent", 2.0)],
            &[("base", 0.25), ("exponent", 0.5)],
            &[("base", 0.0), ("exponent", 3.0)],
        ],
    ),
    (
        "pythagorean",
        &[
            &[("a", 0.75), ("b", 1.0)],
            &[("a", 0.5), ("b", -12.0)],
            &[("a", 0.0), ("b", 0.0)],
        ],
    ),
    (
        "smoothstep",
        &[
            &[("input", 0.5), ("edgeA", 0.0), ("edgeB", 1.0)],
            &[("input", 0.25), ("edgeA", 1.0), ("edgeB", -1.0)],
            &[("input", 0.75), ("edgeA", 0.5), ("edgeB", 0.5)],
        ],
    ),
    (
        "threshold",
        &[
            &[("input", 0.5), ("threshold", 0.5), ("smooth", 0.01)],
            &[("input", 0.75), ("threshold", 0.5), ("smooth", 0.5)],
            &[("input", 0.25), ("threshold", 0.5), ("smooth", 0.0)],
        ],
    ),
    (
        "sine",
        &[
            &[
                ("input", 0.25),
                ("frequency", 1.0),
                ("phase", 0.0),
                ("amplitude", 2.0),
            ],
            &[
                ("input", 0.75),
                ("frequency", 2.0),
                ("phase", -0.5),
                ("amplitude", 1.0),
            ],
            &[
                ("input", 0.0),
                ("frequency", 0.5),
                ("phase", 1.5),
                ("amplitude", 3.0),
            ],
        ],
    ),
    (
        "cosine",
        &[
            &[
                ("input", 0.25),
                ("frequency", 1.0),
                ("phase", 0.0),
                ("amplitude", 2.0),
            ],
            &[
                ("input", 0.5),
                ("frequency", 3.0),
                ("phase", -0.75),
                ("amplitude", 2.5),
            ],
            &[
                ("input", 1.0),
                ("frequency", 0.01),
                ("phase", -2.0),
                ("amplitude", 4.0),
            ],
        ],
    ),
    (
        "reframerange",
        &[
            &[
                ("input", 0.5),
                ("inMin", 0.0),
                ("inMax", 1.0),
                ("outMin", 0.0),
                ("outMax", 10.0),
            ],
            &[
                ("input", 0.25),
                ("inMin", 0.0),
                ("inMax", 0.5),
                ("outMin", 4.0),
                ("outMax", -4.0),
            ],
            &[
                ("input", 1.0),
                ("inMin", 0.0),
                ("inMax", 1.0),
                ("outMin", -6.0),
                ("outMax", 6.0),
            ],
        ],
    ),
    (
        "sliderule",
        &[&[("input", 0.5)], &[("input", 0.0)], &[("input", 1.0)]],
    ),
];

/// What the node's `eval` publishes on these values: a headless app, one tick, one read.
fn diamond_value(slug: &'static str, values: &[(&str, f32)]) -> f32 {
    let mut app = App::headless();
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: emath::Pos2::ZERO,
        workspace,
    })
    .expect("in the registry");
    let id = app
        .graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added");
    for (key, value) in values {
        let key = nodes::find(slug)
            .and_then(|d| d.input(key))
            .map(|i| i.key)
            .expect("a key of this node");
        app.apply(Command::SetControl {
            node: id,
            key,
            value: ControlValue::Float(*value),
        })
        .expect("a number control");
    }
    app.tick(1.0 / 60.0);
    app.uniform(PortRef::new(id, "output"))
        .expect("a dual node with knobs only publishes its formula")
}

/// What the node's WGSL computes on the same values, measured on the GPU: the first value
/// arrives as a constant field, which keeps the node in circle mode, the rest are knobs, and
/// the node's field goes into a tap's `number` sidechain, so its mean over the grid is its
/// value.
fn circle_reading(slug: &'static str, values: &[(&str, f32)]) -> f32 {
    let mut g = Graph::new();
    let under = add(&mut g, slug);
    let key_of = |key: &str| {
        nodes::find(slug)
            .and_then(|d| d.input(key))
            .map(|i| i.key)
            .expect("a key of this node")
    };
    let (driven, value) = values[0];
    let field = constant_field(&mut g, value);
    g.connect(field, PortRef::new(under, key_of(driven)))
        .expect("a field into the first input");
    let def = nodes::find(slug).expect("in the registry");
    for (key, value) in &values[1..] {
        let (key, value) =
            nodes::coerce_control(def, key, ControlValue::Float(*value), None).expect("a number");
        g.get_mut(under)
            .expect("just added")
            .controls
            .insert(key, value);
    }
    let cb = add(&mut g, "checkerboard");
    let (t, out) = tapped(&mut g, PortRef::new(cb, "output"));
    g.connect(PortRef::new(under, "output"), PortRef::new(t, "number"))
        .expect("the node under test is a field, so it can be measured");
    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64, "{slug}: every cell measured");
    s.mean
}

/// **One formula, two implementations, held equal by the GPU.** Every dual node carries its
/// body twice — once as a shader's and once as Rust for the tick — and nothing but this stops
/// the two drifting. The tolerance is 1e-3: the tap's sums are fixed point at 1/65536, so what
/// is left is the difference between two `sin` implementations.
#[test]
fn a_dual_nodes_two_implementations_agree() {
    for (slug, sets) in DUAL_CASES {
        for values in *sets {
            let cpu = diamond_value(slug, values);
            let gpu = circle_reading(slug, values);
            assert!(
                (cpu - gpu).abs() < 1e-3,
                "{slug} {values:?}: the tick says {cpu}, the shader says {gpu}"
            );
        }
    }
}

/// And the family is the whole family: a dual node nobody added to `DUAL_CASES` would go
/// unchecked.
#[test]
fn every_dual_output_in_the_registry_is_covered() {
    for def in nodes::REGISTRY {
        for p in def.outputs.iter().filter(|p| p.eval.is_some()) {
            assert!(
                DUAL_CASES.iter().any(|(slug, _)| *slug == def.slug),
                "{}.{}: a dual output with no equivalence case",
                def.slug,
                p.key,
            );
        }
    }
}

// --------------------------------------------------------------- published values, drawn

/// A headless app with no GPU, and a way to add a node to its workspace.
fn app_adding() -> (App, impl FnMut(&mut App, &'static str) -> NodeId) {
    let app = App::headless();
    let workspace = app.graph().default_workspace();
    let add = move |app: &mut App, slug: &'static str| {
        app.apply(Command::AddNode {
            slug,
            at: emath::Pos2::ZERO,
            workspace,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    (app, add)
}

/// `out` of `app`'s graph drawn at `size` with the uniforms the app's own frame job resolved,
/// on a renderer of its own, and its pixels as RGBA8. Every value the WGSL module reads is one
/// the job names: the two targets agree on uniform names.
fn drawn_with_the_apps_uniforms(app: &mut App, out: NodeId, size: (u32, u32)) -> Vec<[u8; 4]> {
    let job_of = app.build_frame_job();
    let uniforms = job_of
        .outputs
        .iter()
        .find(|o| o.node == out)
        .expect("an Output")
        .uniforms
        .clone();
    let shader = module_of(app.graph(), out);
    for (name, provider) in &shader.uniforms {
        if provider.ty() != UniformType::Sampler2D {
            assert!(
                uniforms.iter().any(|(n, _)| n == name),
                "{name} is resolved by the app's job"
            );
        }
    }
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let outputs = |send: bool, mode| vec![job(out, size, &shader, send, mode, uniforms.clone())];
    link_all(&mut renderer, &outputs);
    renderer.draw(&tick(0.0, outputs(false, DRAW)));
    drain(&gpu);
    rgba_of(&gpu, &renderer.texture_of(out).expect("drawn"))
}

/// A `color` node's published color reaches the shader as the `vec4f` it is: a checkerboard
/// whose two squares are both fed from one `color` renders in that one color, every pixel. The
/// uniform is resolved through `App`, whose tick is what puts the color in the map at all.
#[test]
fn a_published_color_paints_the_frame() {
    const PINK: [f32; 4] = [1.0, 0.0, 0.5, 1.0];
    let (mut app, mut add) = app_adding();
    let color = add(&mut app, "color");
    let cb = add(&mut app, "checkerboard");
    let out = add(&mut app, "output");
    for key in ["color1", "color2"] {
        app.apply(Command::Connect {
            from: PortRef::new(color, "output"),
            to: PortRef::new(cb, key),
        })
        .unwrap();
    }
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: color,
        key: "color",
        value: ControlValue::Color(PINK),
    })
    .unwrap();
    app.tick(1.0 / 60.0);
    let body = module_of(app.graph(), out).body.clone();
    assert!(
        body.contains(&format!("u_color_color{color}_output: vec4f,")),
        "the published color is a vec4f in the module:\n{body}"
    );
    for p in drawn_with_the_apps_uniforms(&mut app, out, (32, 32)) {
        assert_eq!(p, [255, 0, 128, 255], "every pixel is the published color");
    }
}

/// `decompose::hsl` is a transcription of three rows of the conversion table, held to the
/// shader's: one `color` read through `hue`, `saturation` and `lightness` into an `rgba`'s
/// three channels, and the pixel compared with what the Rust returns for the same color.
#[test]
fn the_cpu_hsl_is_the_same_formula_the_shader_runs() {
    const COLORS: [[f32; 4]; 5] = [
        [1.0, 0.25, 0.1, 1.0],
        [0.2, 0.9, 0.3, 1.0],
        [0.1, 0.3, 0.95, 1.0],
        [0.6, 0.6, 0.6, 1.0],
        [0.95, 0.1, 0.8, 1.0],
    ];
    let (mut app, mut add) = app_adding();
    let color = add(&mut app, "color");
    let hue = add(&mut app, "hue");
    let saturation = add(&mut app, "saturation");
    let lightness = add(&mut app, "lightness");
    let rgba = add(&mut app, "rgba");
    let out = add(&mut app, "output");
    for node in [hue, saturation, lightness] {
        app.apply(Command::Connect {
            from: PortRef::new(color, "output"),
            to: PortRef::new(node, "input"),
        })
        .unwrap();
    }
    for (node, key) in [(hue, "r"), (saturation, "g"), (lightness, "b")] {
        app.apply(Command::Connect {
            from: PortRef::new(node, "output"),
            to: PortRef::new(rgba, key),
        })
        .unwrap();
    }
    app.apply(Command::Connect {
        from: PortRef::new(rgba, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    for want in COLORS {
        app.apply(Command::SetControl {
            node: color,
            key: "color",
            value: ControlValue::Color(want),
        })
        .unwrap();
        app.tick(1.0 / 60.0);
        let pixel = drawn_with_the_apps_uniforms(&mut app, out, (4, 4))[0];
        let shaded = [pixel[0], pixel[1], pixel[2]].map(|c| f32::from(c) / 255.0);
        let cpu = nodes::decompose::hsl(want);
        for (i, name) in ["hue", "saturation", "lightness"].into_iter().enumerate() {
            assert!(
                (shaded[i] - cpu[i]).abs() < 1.0 / 255.0 + 1e-4,
                "{name} of {want:?}: shader {} against cpu {}",
                shaded[i],
                cpu[i],
            );
        }
    }
}

// ----------------------------------------------------------------------------- the probe

/// The probe counts exactly: a 3x3 blur runs its input nine times per pixel, so over the
/// probe's pixels the checkerboard is counted nine times as often as the blur, and the blur's
/// call site counts every one of its nine calls.
#[test]
fn a_probe_counts_a_blurs_nine_taps_exactly() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let blur = add(&mut g, "blur");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(blur, "input"))
        .unwrap();
    g.connect(PortRef::new(blur, "output"), PortRef::new(out, "input"))
        .unwrap();
    let shader = module_of(&g, out);
    let probe = Arc::new(wgsl::build_probe(&g, out).expect("connected"));
    assert!(probe.diagnostics.is_empty(), "{:?}", probe.diagnostics);
    let values = resolve(&g, &shader, &|_| 0.0);
    let probe_values = resolve(&g, &probe, &|_| 0.0);

    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let frame = |first: bool| FrameJob {
        probes: vec![ProbeJob {
            output: out,
            shader: first.then(|| Arc::clone(&probe)),
            source: compile::source_hash(&probe.body),
            uniforms: probe_values.clone(),
            taps: probe.taps.len(),
        }],
        ..tick(
            0.0,
            vec![job(out, (64, 64), &shader, first, DRAW, values.clone())],
        )
    };
    // The probe links on a linker of its own and is read back a tick after it draws, so the
    // first reading is waited for by the clock rather than counted in ticks.
    renderer.draw(&frame(true));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let words = loop {
        drain(&gpu);
        renderer.draw(&frame(false));
        assert_eq!(renderer.errors.get(&out), None);
        if let Some((_, w)) = renderer.take_probes().pop() {
            break w;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the probe read nothing back"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let count = |id| {
        let slot = probe
            .taps
            .iter()
            .position(|(n, k)| *n == id && *k == TapKind::Evaluations)
            .unwrap();
        words[slot * TAP_WORDS + EVAL_WORD]
    };
    let pixels = PROBE.0 * PROBE.1;
    assert_eq!(count(blur), pixels, "the blur runs once per probe pixel");
    assert_eq!(count(cb), 9 * pixels, "and its input nine times");
    let site = probe
        .taps
        .iter()
        .position(|(n, k)| *n == blur && *k == TapKind::Taps("input"))
        .expect("the blur's call site has a slot");
    assert_eq!(
        words[site * TAP_WORDS + EVAL_WORD],
        9 * pixels,
        "the call site counts every one of the blur's nine calls"
    );
}

// ------------------------------------------------------------------- premultiplied colors

/// An `rgba` of `rgb` at alpha `a`: a color the shader builds, so nothing on the CPU converts
/// it on the way in.
fn solid(g: &mut Graph, rgb: [f32; 3], a: f32) -> NodeId {
    let n = add(g, "rgba");
    for (key, value) in [("r", rgb[0]), ("g", rgb[1]), ("b", rgb[2]), ("a", a)] {
        set(g, n, key, value);
    }
    n
}

/// `node`'s `port` into a new Output: the Output.
fn shown(g: &mut Graph, node: NodeId, port: &'static str) -> NodeId {
    let out = add(g, "output");
    g.connect(PortRef::new(node, port), PortRef::new(out, "input"))
        .unwrap();
    out
}

/// **RGBA and HSLA premultiply what they build**: half-transparent red is half red at half
/// alpha, and an Alpha of zero is transparent black whatever the channels say.
#[test]
fn a_color_built_from_numbers_is_premultiplied() {
    for (rgb, a, expected) in [
        ([1.0, 0.0, 0.0], 0.5, [128, 0, 0, 128]),
        ([1.0, 1.0, 1.0], 0.0, [0, 0, 0, 0]),
        ([0.5, 1.0, 0.25], 1.0, [128, 255, 64, 255]),
    ] {
        let mut g = Graph::new();
        let color = solid(&mut g, rgb, a);
        let out = shown(&mut g, color, "output");
        for p in rendered(&g, out, 4) {
            assert_eq!(p, expected, "rgba {rgb:?} at alpha {a}");
        }
    }
    let mut g = Graph::new();
    let color = add(&mut g, "hsla");
    for (key, value) in [("h", 0.0), ("s", 1.0), ("l", 0.5), ("a", 0.5)] {
        set(&mut g, color, key, value);
    }
    let out = shown(&mut g, color, "output");
    for p in rendered(&g, out, 4) {
        assert_eq!(p, [128, 0, 0, 128], "hsla red at half alpha");
    }
}

/// **A number read out of a color is the color's own**: every conversion of a
/// half-transparent color reads what it reads of the same color opaque, and `alpha` reads
/// the alpha. Each is drawn through an opaque `rgba`'s red.
#[test]
fn a_conversion_reads_the_colors_own_channels() {
    const RGB: [f32; 3] = [1.0, 0.5, 0.25];
    let read = |slug: &str, port: &'static str, a: f32| {
        let mut g = Graph::new();
        let color = solid(&mut g, RGB, a);
        let under = add(&mut g, slug);
        let carrier = add(&mut g, "rgba");
        let out = add(&mut g, "output");
        g.connect(PortRef::new(color, "output"), PortRef::new(under, "input"))
            .unwrap();
        g.connect(PortRef::new(under, port), PortRef::new(carrier, "r"))
            .unwrap();
        g.connect(PortRef::new(carrier, "output"), PortRef::new(out, "input"))
            .unwrap();
        red_at(&rendered(&g, out, 4), 4, 2, 2)
    };
    assert_eq!(
        read("red", "output", 0.5),
        255,
        "red of half-transparent red"
    );
    assert_eq!(read("channelsplitter", "r", 0.5), 255);
    assert_eq!(read("alpha", "output", 0.5), 128);
    for conversion in nodes::decompose::CONVERSIONS {
        if conversion.slug == "alpha" {
            continue;
        }
        let opaque = read(conversion.slug, "output", 1.0);
        let half = read(conversion.slug, "output", 0.5);
        assert!(
            opaque.abs_diff(half) <= 1,
            "{}: {half} at half alpha, {opaque} opaque",
            conversion.slug
        );
    }
}

/// A sample publishes the color's own channels, and its alpha beside them.
#[test]
fn a_sample_reads_the_colors_own_channels() {
    let mut g = Graph::new();
    let color = solid(&mut g, [1.0, 0.5, 0.25], 0.5);
    let s = add(&mut g, "sample");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(color, "output"), PortRef::new(s, "input"))
        .unwrap();
    g.connect(PortRef::new(s, "output"), PortRef::new(out, "input"))
        .unwrap();
    let read = sample::decode(&slot(tap_words(&g, out, 2))).expect("the point was measured");
    assert_eq!(read, [1.0, 0.5, 0.25, 0.5]);
}

/// A tap measures the color's own channels: its picked quantity and its mean color are those
/// of the same color opaque.
#[test]
fn a_tap_reads_the_colors_own_channels() {
    let mut g = Graph::new();
    let color = solid(&mut g, [1.0, 0.5, 0.25], 0.5);
    let (t, out) = tapped(&mut g, PortRef::new(color, "output"));
    g.get_mut(t)
        .unwrap()
        .options
        .insert("measure", "red".to_string());
    let s = tap::decode(&slot(tap_words(&g, out, 2)));
    assert_eq!(s.count, 64 * 64);
    assert_eq!(s.mean, 1.0, "the red of half-transparent red is one");
    let [r, g_, b, a] = s.color;
    assert!(
        (r - 1.0).abs() < 1e-3 && (g_ - 0.5).abs() < 1e-3 && (b - 0.25).abs() < 1e-3,
        "the mean color is the color's own: {:?}",
        s.color
    );
    assert_eq!(a, 1.0, "and opaque");
}

/// A node that reads its input more than once: the slug, the port drawn, its controls and
/// its options.
type SampledCase = (
    &'static str,
    &'static str,
    &'static [(&'static str, f32)],
    &'static [(&'static str, &'static str)],
);

/// **A color assembled from several samples stays premultiplied.** A checkerboard of opaque
/// red and transparent black puts an edge of alpha under every node that takes a channel
/// from one sample and the alpha from another, or measures an edge and paints it with the
/// center's alpha: no channel of any pixel may exceed its alpha.
#[test]
fn a_color_assembled_from_several_samples_never_exceeds_its_alpha() {
    const SIZE: u32 = 32;
    let cases: &[SampledCase] = &[
        (
            "glitch",
            "color",
            &[("intensity", 1.0), ("rgbSplit", 0.1)],
            &[],
        ),
        ("chromaticaberration", "output", &[("offset", 0.1)], &[]),
        (
            "chromaticaberration",
            "output",
            &[("offset", 0.1)],
            &[("mode", "linear")],
        ),
        ("edgedetection", "output", &[], &[]),
        (
            "edgedetection",
            "output",
            &[],
            &[("mode", "laplacian_gray")],
        ),
        ("kuwahara", "color", &[], &[("size", "5x5")]),
        ("emboss", "output", &[], &[]),
        ("sharpen", "color", &[("amount", 4.0)], &[]),
        ("dilate", "color", &[("radius", 0.05)], &[]),
    ];
    for (slug, output, controls, options) in cases {
        let mut g = Graph::new();
        let cb = add(&mut g, "checkerboard");
        set_color(&mut g, cb, "color1", [1.0, 0.0, 0.0, 1.0]);
        set_color(&mut g, cb, "color2", [0.0, 0.0, 0.0, 0.0]);
        let under = add(&mut g, slug);
        g.connect(PortRef::new(cb, "output"), PortRef::new(under, "input"))
            .unwrap();
        for (key, value) in *controls {
            set(&mut g, under, key, *value);
        }
        for (key, value) in *options {
            g.get_mut(under)
                .unwrap()
                .options
                .insert(key, (*value).to_string());
        }
        let out = shown(&mut g, under, output);
        for (i, p) in rendered(&g, out, SIZE).iter().enumerate() {
            assert!(
                p[..3].iter().all(|c| *c <= p[3]),
                "{slug} {options:?}, pixel {i}: {p:?} has a channel above its alpha"
            );
        }
    }
}
