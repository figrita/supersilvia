// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the shader compiler. Pure string generation, no GPU.

use emath::Pos2;
use supersilvia::compile::{self, UniformProvider, UniformType};
use supersilvia::graph::{Graph, NodeId, PortRef, PortType};
use supersilvia::nodes;

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is in the registry")
}

/// The node and port a `VaryingNumber` input is driven from, where a test wants the
/// connected path.
///
/// A vignette's falloff, because it is the smallest output in the library that is a *field*
/// and nothing else. The `math` family cannot stand here: every one of them is dual, so with
/// knobs on its own inputs it publishes a uniform number and a consumer resolves it to a
/// member of the uniform struct — which is the unconnected path, not the connected one.
const FIELD: (&str, &str) = ("vignette", "mask");

/// checkerboard -> output, the smallest graph that draws.
fn checkerboard_into_output() -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(out, "input"))
        .expect("color into color");
    (g, cb, out)
}

#[test]
fn an_unconnected_output_has_no_shader() {
    let mut g = Graph::new();
    let out = add(&mut g, "output");
    // Not an error. The node is inactive and the preview is black, as in silvia.
    assert!(compile::wgsl::build(&g, out).is_none());
}

#[test]
fn unconnected_controls_become_uniforms_with_providers() {
    let (g, cb, out) = checkerboard_into_output();
    let shader = compile::wgsl::build(&g, out).unwrap();

    let freq = shader
        .uniforms
        .get("u_control_checkerboard1_frequency")
        .expect("frequency is an unconnected control");
    assert_eq!(
        *freq,
        UniformProvider::Control {
            node: cb,
            key: "frequency",
            ty: UniformType::Float
        },
    );
    assert_eq!(
        shader.uniforms["u_control_checkerboard1_color1"].ty(),
        UniformType::Vec4,
    );

    // Three controls, and nothing else: a connected input is a call, not a uniform.
    assert_eq!(shader.uniforms.len(), 3);
}

#[test]
fn a_connected_input_is_a_call_not_a_uniform() {
    let (g, _, out) = checkerboard_into_output();
    let shader = compile::wgsl::build(&g, out).unwrap();

    assert!(shader.body.contains("return checkerboard1_output(uv);"));
    assert!(!shader.uniforms.contains_key("u_control_output2_input"));
}

#[test]
fn a_diamond_emits_each_function_once() {
    // One checkerboard feeding two inputs of the same downstream node would repeat its
    // function if `visited` were not consulted. Two Outputs sharing one source is the
    // reachable version of that with the nodes available now.
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(a, "input"))
        .unwrap();
    g.connect(PortRef::new(cb, "output"), PortRef::new(b, "input"))
        .unwrap();

    for out in [a, b] {
        let shader = compile::wgsl::build(&g, out).unwrap();
        assert_eq!(
            shader
                .body
                .matches("fn checkerboard1_output(uv: vec2f) -> vec4f")
                .count(),
            1,
        );
    }
}

#[test]
fn an_output_frame_feeding_another_output_becomes_a_texture_uniform() {
    // Output A's frame port is how a graph gets an intermediate buffer.
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(a, "input"))
        .unwrap();
    g.connect(PortRef::new(a, "frame"), PortRef::new(b, "input"))
        .unwrap();

    let shader = compile::wgsl::build(&g, b).expect("b's input is connected");

    // B samples A's texture; it does not inline A's whole tree.
    assert_eq!(
        shader.uniforms["u_texture_output2_frame"],
        UniformProvider::NodeTexture {
            node: a,
            port: "frame"
        },
    );
    assert!(
        shader
            .body
            .contains("var u_texture_output2_frame: texture_2d<f32>;")
    );
    assert!(
        !shader.body.contains("checkerboard1_output"),
        "B must sample A's frame, not recompile A's chain into its own shader",
    );
}

#[test]
fn uniform_order_is_deterministic() {
    let (g, _, out) = checkerboard_into_output();
    let a = compile::wgsl::build(&g, out).unwrap();
    let b = compile::wgsl::build(&g, out).unwrap();
    assert_eq!(a.body, b.body, "two builds of one patch must be identical");
}

/// The compiler produces a shader for a broken graph rather than refusing — silvia's rule,
/// and the right one for a live tool. What changed is that it no longer does so silently.
#[test]
fn a_well_formed_patch_produces_no_diagnostics() {
    let mut g = Graph::new();
    let cb = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    let zoom = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
    let out = nodes::add_to_graph(&mut g, "output", Pos2::ZERO).unwrap();
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();

    let shader = compile::wgsl::build(&g, out).expect("input is connected");
    assert!(
        shader.diagnostics.is_empty(),
        "diagnostics on a healthy patch: {:?}",
        shader.diagnostics
    );
}

/// A divisor can be driven by a connection, which no control range constrains. An
/// oscillator into `zoom` crosses zero twice a cycle, and `uv / 0.0` is a NaN frame on the
/// projector. `math::DIVIDE` guards the identical case.
#[test]
fn zoom_guards_its_divisor() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();

    let body = compile::wgsl::build(&g, out).expect("connected").body;
    assert!(
        body.contains("zoomAmount"),
        "the divisor is bound and checked before the divide:\n{body}"
    );
    assert!(
        !body.contains("/ (u.u_control_zoom"),
        "nothing divides by the raw input:\n{body}"
    );
}

/// The same for the fisheye's radius, which also feeds `pow`'s base.
#[test]
fn fisheye_guards_its_radius() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let lens = add(&mut g, "fisheye");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(lens, "input"))
        .unwrap();
    g.connect(PortRef::new(lens, "output"), PortRef::new(out, "input"))
        .unwrap();

    let body = compile::wgsl::build(&g, out).expect("connected").body;
    assert!(
        body.contains("lensRadius = max("),
        "the radius is clamped positive before the divide and the pow:\n{body}"
    );
}

// --------------------------------------------- varying color to varying number

/// The cheap half of "going down": a color becomes a number per pixel, with no readback and
/// no frame of latency. Every one of these is a candidate the conversion menu can offer for
/// a varying color output dragged onto a varying number input.
#[test]
fn every_color_to_fragment_number_node_compiles_from_a_color_source() {
    for slug in [
        "luminosity",
        "lightness",
        "value",
        "average",
        "hue",
        "saturation",
        "chroma",
        "red",
        "green",
        "blue",
        "alpha",
    ] {
        let mut g = Graph::new();
        let cb = add(&mut g, "checkerboard");
        let conv = add(&mut g, slug);
        let out = add(&mut g, "output");
        // Color in, varying number out, and it drives a color node's control input.
        g.connect(PortRef::new(cb, "output"), PortRef::new(conv, "input"))
            .unwrap_or_else(|e| panic!("{slug}: color into {slug}: {e}"));
        g.connect(PortRef::new(conv, "output"), PortRef::new(out, "input"))
            .expect_err("a varying number output cannot feed a color input");

        let mix = add(&mut g, "mix");
        g.connect(PortRef::new(conv, "output"), PortRef::new(mix, "amount"))
            .unwrap_or_else(|e| panic!("{slug}: varying number into varying number: {e}"));
        g.connect(PortRef::new(mix, "output"), PortRef::new(out, "input"))
            .unwrap();

        let shader = compile::wgsl::build(&g, out).unwrap_or_else(|| panic!("{slug}: no shader"));
        assert!(
            shader.diagnostics.is_empty(),
            "{slug}: {:?}",
            shader.diagnostics
        );
    }
}

// ---------------------------------------------------------------- many outputs, one node

/// A node has as many outputs as it declares, and several of them may be textures.
///
/// This was not true of the plumbing for a long time and nothing said so. The compiler was
/// always right — it names a texture uniform `u_texture_{slug}{id}_{key}` and records the port
/// beside it — but `publish_frame`, `App::frames` and the renderer's upload all keyed on the
/// **node**, because until `video` grew an oscilloscope beside its picture, no node had two.
/// Two texture outputs on one node therefore compiled to two uniforms that sampled the same
/// texture: whichever was published last. These are the tests that would have caught it.
#[test]
fn two_texture_outputs_on_one_node_are_two_uniforms() {
    let mut g = Graph::new();
    let video = add(&mut g, "video");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    g.connect(PortRef::new(video, "frame"), PortRef::new(a, "input"))
        .unwrap();
    g.connect(
        PortRef::new(video, "oscilloscope"),
        PortRef::new(b, "input"),
    )
    .unwrap();

    let picture = compile::wgsl::build(&g, a).expect("an output compiles");
    let waveform = compile::wgsl::build(&g, b).expect("and so does the other");
    let names = |s: &compile::Shader| -> Vec<String> {
        s.uniforms
            .iter()
            .filter(|(_, p)| matches!(p, compile::UniformProvider::NodeTexture { .. }))
            .map(|(n, _)| n.to_string())
            .collect()
    };
    let (p, w) = (names(&picture), names(&waveform));
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(w.len(), 1, "{w:?}");
    assert_ne!(
        p[0], w[0],
        "two texture outputs of one node must not share a uniform"
    );
}

/// And the provider carries the port, which is what the app and the renderer key on.
#[test]
fn a_texture_uniform_records_which_port_it_came_from() {
    let mut g = Graph::new();
    let video = add(&mut g, "video");
    let out = add(&mut g, "output");
    g.connect(
        PortRef::new(video, "oscilloscope"),
        PortRef::new(out, "input"),
    )
    .unwrap();

    let shader = compile::wgsl::build(&g, out).expect("an output compiles");
    let port = shader
        .uniforms
        .values()
        .find_map(|p| match p {
            compile::UniformProvider::NodeTexture { node, port } => Some((*node, *port)),
            _ => None,
        })
        .expect("a texture uniform");
    assert_eq!(
        port,
        (video, "oscilloscope"),
        "a provider that forgot the port would sample whichever texture the node published last"
    );
}

/// Every texture output in the registry names its own uniform, so a node added later cannot
/// quietly reintroduce the collision.
#[test]
fn every_texture_output_has_its_own_uniform_name() {
    for def in nodes::REGISTRY {
        let textures: Vec<&str> = def
            .outputs
            .iter()
            .filter(|o| o.kind == nodes::OutputKind::Texture)
            .map(|o| o.key)
            .collect();
        if textures.len() < 2 {
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        for key in &textures {
            let mut g = Graph::new();
            let id = add(&mut g, def.slug);
            let out = add(&mut g, "output");
            g.connect(PortRef::new(id, key), PortRef::new(out, "input"))
                .expect("a texture output is a color output");
            let shader = compile::wgsl::build(&g, out).expect("an output compiles");
            let name = shader
                .uniforms
                .iter()
                .find(|(_, p)| matches!(p, compile::UniformProvider::NodeTexture { .. }))
                .map(|(n, _)| n.to_string())
                .expect("a texture uniform");
            assert!(
                seen.insert(name.clone()),
                "{}.{key}: {name} is already another output's uniform",
                def.slug,
            );
        }
    }
}

/// The nodes whose picture is a `Shader` output reading a state its own tick published:
/// `(slug, state port, picture port)`.
const STATE_TEXTURES: &[(&str, &str, &str)] = &[
    ("cellularautomata", "cells", "output"),
    ("slimemold", "trail", "color"),
    ("brickgame", "field", "color"),
];

/// A picture colored in the shader out of a texture its **own node** publishes.
///
/// A simulation on the CPU publishes the state it computed, not a picture, and the color
/// inputs are mixed into it in the shader — so the generator asks for a texture uniform of a
/// *sibling* port rather than of the port being compiled. Nothing about `texture_uniform` is
/// specific to the output calling it, and this is what holds that: one texture uniform,
/// naming the state port, bound by the renderer off the same `PortRef` the tick published
/// under.
#[test]
fn a_picture_samples_the_state_its_own_node_published() {
    for (slug, state, picture) in STATE_TEXTURES {
        let def = nodes::find(slug).expect("named in the registry");
        let mut g = Graph::new();
        let under = add(&mut g, slug);
        let out = add(&mut g, "output");
        // Every field beside the picture goes in too, through an `rgba`: a field of one of
        // these reads the same texture, and a generator nothing compiles is a generator
        // nothing checks.
        let fields: Vec<&'static str> = def
            .outputs
            .iter()
            .filter(|p| p.ty == PortType::VaryingNumber)
            .map(|p| p.key)
            .collect();
        let mut root = PortRef::new(under, picture);
        if !fields.is_empty() {
            let rgba = add(&mut g, "rgba");
            for (field, channel) in fields.iter().zip(["r", "g", "b", "a"]) {
                g.connect(PortRef::new(under, field), PortRef::new(rgba, channel))
                    .unwrap_or_else(|e| panic!("{slug}.{field}: {e}"));
            }
            let blend = add(&mut g, "mix");
            g.connect(root, PortRef::new(blend, "a")).unwrap();
            g.connect(PortRef::new(rgba, "output"), PortRef::new(blend, "b"))
                .unwrap();
            root = PortRef::new(blend, "output");
        }
        g.connect(root, PortRef::new(out, "input"))
            .unwrap_or_else(|e| panic!("{slug}.{picture}: {e}"));
        let shader = compile::wgsl::build(&g, out).expect("connected to the Output");
        assert!(
            shader.diagnostics.is_empty(),
            "{slug}: {:?}",
            shader.diagnostics
        );
        let bound: Vec<(NodeId, &str)> = shader
            .uniforms
            .values()
            .filter_map(|p| match p {
                UniformProvider::NodeTexture { node, port } => Some((*node, *port)),
                _ => None,
            })
            .collect();
        assert_eq!(
            bound,
            vec![(under, *state)],
            "{slug}: the picture binds its own state port"
        );
        let wgsl = shader
            .body
            .strip_prefix(compile::wgsl::PRELUDE)
            .expect("every module starts with the prelude");
        insta::assert_snapshot!(format!("state_{slug}"), wgsl);
    }
}

// -------------------------------------------------------------------------------- the tap

/// The one batch of a pass that binds few enough textures to need one.
fn one(mut batches: Vec<compile::Shader>) -> compile::Shader {
    assert_eq!(batches.len(), 1, "one batch");
    batches.pop().expect("one batch")
}

/// The pass that measures `id` and nothing else: a measurement is its workspace's pass's,
/// never an Output's.
fn measuring(g: &Graph, id: NodeId) -> compile::Shader {
    one(compile::wgsl::build_pass(
        g,
        g.default_workspace(),
        &[id],
        false,
    ))
}

/// A tap measuring by its picker splices that conversion's expression, out of the same table
/// the `Convert` nodes read: the eleven quantities and the tap's `measure` cannot drift.
#[test]
fn a_tap_measures_the_quantity_its_picker_names() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let t = add(&mut g, "tap");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(t, "input"))
        .unwrap();
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.get_mut(t)
        .unwrap()
        .options
        .insert("measure", "red".to_string());

    let shader = measuring(&g, t);
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    insta::assert_snapshot!(shader.body);
}

/// A connected `number` overrides the picker: the tap measures that field at the
/// measurement's own point, the conversion helpers are not emitted, and the choice — still
/// `red` here — is not read at all.
#[test]
fn a_connected_number_is_what_the_tap_measures() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let t = add(&mut g, "tap");
    let field = add(&mut g, FIELD.0);
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(t, "input"))
        .unwrap();
    g.connect(PortRef::new(field, FIELD.1), PortRef::new(t, "number"))
        .expect("a varying number into the sidechain");
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.get_mut(t)
        .unwrap()
        .options
        .insert("measure", "red".to_string());

    let shader = measuring(&g, t);
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert!(
        shader
            .body
            .contains(&format!("{}{field}_{}(p)", FIELD.0, FIELD.1)),
        "the sidechain is a call at the measured point, not the picker's expression",
    );
    assert!(
        !shader.body.contains("let delta ="),
        "the conversion helpers belong to the picker's path only",
    );
    insta::assert_snapshot!(shader.body);
}

/// The measurement is a function of a point that its pass's `fs_main` calls under a grid
/// guard, and the grid is the option's: nothing about the tap's reading comes from the
/// coordinate its pass-through was called at. The Output the tap is cabled into measures
/// nothing, and nor does its probe.
#[test]
fn a_tap_is_measured_by_main_over_its_grid() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let t = add(&mut g, "tap");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(t, "input"))
        .unwrap();
    g.connect(PortRef::new(t, "output"), PortRef::new(out, "input"))
        .unwrap();

    let picture = compile::wgsl::build(&g, out).expect("connected");
    assert!(
        !picture.body.contains("_measure") && picture.taps.is_empty(),
        "an Output's module measures nothing:\n{}",
        picture.body,
    );
    let shader = measuring(&g, t);
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert_eq!(
        shader.grid, 128,
        "the square the pass measures in is the grid"
    );
    assert!(
        shader
            .body
            .contains(&format!("fn tap{t}_measure(p: vec2f) {{")),
        "the measurement is a function of a point:\n{}",
        shader.body,
    );
    assert!(
        shader.body.contains("let cell = vec2i(frag_coord.xy);"),
        "and `fs_main` says which cell this fragment is",
    );
    assert!(
        shader.body.contains(&format!(
            "if (cell.x < 128 && cell.y < 128) {{ tap{t}_measure("
        )),
        "guarded by the default grid",
    );
    assert_eq!(
        shader.body.matches(&format!("tap{t}_measure(")).count(),
        2,
        "declared once and called once",
    );
    assert!(
        !shader.body.contains(&format!(
            "fn tap{t}_output(uv: vec2f) -> vec4f {{\n    let color"
        )),
        "and the pass-through is a pass-through",
    );

    g.get_mut(t)
        .unwrap()
        .options
        .insert("grid", "32".to_string());
    let coarse = measuring(&g, t);
    assert!(
        coarse.body.contains(&format!(
            "if (cell.x < 32 && cell.y < 32) {{ tap{t}_measure("
        )),
        "the option is the grid",
    );

    // The probe is drawn at a few pixels, so a grid guard over those would count every one
    // of them as an extra evaluation of the tap's input and misreport it.
    let probe = compile::wgsl::build_probe(&g, out).expect("connected");
    assert!(
        !probe.body.contains("_measure"),
        "a probe carries no measurement",
    );
    assert!(
        !probe.body.contains("let cell"),
        "and no cell to guard one with",
    );
}

/// A sample's measurement is one call, at the point its uniforms name, made by the one
/// fragment at the corner.
#[test]
fn a_sample_is_measured_once_at_its_own_point() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let s = add(&mut g, "sample");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(s, "input"))
        .unwrap();
    g.connect(PortRef::new(s, "output"), PortRef::new(out, "input"))
        .unwrap();

    let shader = measuring(&g, s);
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert_eq!(shader.grid, 1, "one point, at the corner");
    assert!(
        shader.body.contains(&format!("fn sample{s}_measure() {{")),
        "the point is the body's, not an argument:\n{}",
        shader.body,
    );
    assert!(
        shader.body.contains(&format!(
            "if (all(cell == vec2i(0))) {{ sample{s}_measure(); }}"
        )),
        "and one fragment makes the call",
    );
    assert!(
        shader.body.contains(&format!(
            "let c = checkerboard{cb}_output(vec2f((u.u_control_sample{s}_x), (u.u_control_sample{s}_y)))"
        )),
        "the input is evaluated at the point exactly:\n{}",
        shader.body,
    );
    assert!(
        !shader.body.contains("texel"),
        "no tolerance around the point, so no node body reads the resolution",
    );
}

/// A workspace's pass carries a measurement no picture contains: the node's `measure`
/// function, the chain that measurement evaluates, and the guarded call `fs_main` makes. Its
/// pass-through is not in there — nothing calls it — and the Output on the same workspace,
/// drawing a checkerboard, carries none of it.
#[test]
fn a_pass_carries_a_measurement_and_its_chain() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let out = add(&mut g, "output");
    let gradient = add(&mut g, "radialgradient");
    let t = add(&mut g, "tap");
    g.connect(PortRef::new(cb, "output"), PortRef::new(out, "input"))
        .unwrap();
    g.connect(PortRef::new(gradient, "output"), PortRef::new(t, "input"))
        .unwrap();

    let alone = compile::wgsl::build(&g, out).expect("connected");
    assert!(
        !alone.body.contains("_measure") && alone.taps.is_empty(),
        "the picture is a checkerboard: nothing in it measures anything",
    );

    let shader = measuring(&g, t);
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert!(
        shader
            .body
            .contains(&format!("fn tap{t}_measure(p: vec2f)")),
        "the measurement is emitted:\n{}",
        shader.body,
    );
    assert!(
        shader
            .body
            .contains(&format!("radialgradient{gradient}_output(p)")),
        "and it pulls its own chain in, evaluated at the measured point",
    );
    assert!(
        shader
            .body
            .contains(&format!("tap{t}_measure(((vec2f(cell) + 0.5)")),
        "and `fs_main` calls it over the grid",
    );
    assert!(
        !shader.body.contains(&format!("tap{t}_output")),
        "the pass-through is not emitted: nothing in a pass without thumbnails calls it",
    );
    assert_eq!(shader.taps.len(), 1, "one slot, the tap's");
    assert_eq!(shader.taps[0].0, t);
    assert!(shader.thumbs.is_empty());
    insta::assert_snapshot!(shader.body);

    // With thumbnails the slots still come first, and every tile lies right of the grid.
    let full = one(compile::wgsl::build_pass(
        &g,
        g.default_workspace(),
        &[t],
        true,
    ));
    assert_eq!(full.taps.len(), 1);
    assert_eq!(full.thumb_base(), compile::TAP_WORDS);
    assert!(
        full.body.contains("let tiled = cell - vec2i(128, 0);"),
        "the tiles begin where the grid ends:\n{}",
        full.body,
    );
    let (w, h) = full.pass_size();
    assert!(
        w > 128 && h >= 128,
        "{w}x{h} holds the grid and the tiles beside it"
    );
    assert_eq!(full.tap_region(), (128, 128));
}

/// The `measure` choices are the `Convert` family, by slug, and nothing else: one table, read
/// twice.
#[test]
fn the_measure_choices_are_exactly_the_convert_nodes() {
    let def = nodes::find("tap").expect("the tap is in the registry");
    let option = def.option("measure").expect("the tap has a picker");
    let choices: Vec<&str> = option.choices.iter().map(|(v, _)| *v).collect();
    assert_eq!(
        choices,
        vec![
            "red",
            "green",
            "blue",
            "alpha",
            "hue",
            "saturation",
            "lightness",
            "luminosity",
            "value",
            "average",
            "chroma",
        ],
    );
    for slug in &choices {
        let conv = nodes::find(slug).unwrap_or_else(|| panic!("{slug} is a node"));
        assert_eq!(conv.category, nodes::Category::Convert);
    }
    assert_eq!(option.default, "luminosity", "what a tap always measured");
    assert_eq!(option.overridden_by, Some("number"));
}

// ---------------------------------------------------------------------- the node library

/// Every node with a picture or a field, both ways round, under
/// `wgsl_{group}_{slug}_{connected|unconnected}` with the prelude stripped, so what is reviewed
/// is the node's own WGSL and not the preamble repeated per node.
///
/// Two snapshots each: with nothing plugged in, where every input is a uniform or a prelude
/// global, and with every port driven, where each is a call into another function. Those are
/// the two shapes `ctx.input` can return, and a body that reads an input twice or splices one
/// where the wrong `uv` is in scope reads wrong in exactly one of them.
///
/// `Transform` is held for the second of those: a transform's whole job is to hand its input
/// a coordinate of its own, and the snapshot is where the `at` expression and the body that
/// declares it are read against each other. `Effect` is held for the same reason one step on
/// — a neighborhood is that coordinate written a dozen times — and `Color`, `Convert` and
/// `Math` because a library reviewed by eye once is a library reviewed.
///
/// Those groups' nodes with no `cpu` half are held further: every cable lands, and a placeholder where their WGSL belongs fails rather than
/// being skipped. The groups' counts are asserted so that a node added to one without a
/// snapshot is a failure here rather than a silently unreviewed body. Any other node whose
/// WGSL is not written compiles to a placeholder and is skipped; `tests/shader_targets.rs`
/// prints how many have.
#[test]
fn every_wgsl_node_compiles_connected_and_unconnected() {
    use supersilvia::nodes::Category;

    // The nodes with no `cpu` half each group has, so adding a node to one is a deliberate
    // edit here. The ones that move with time have none: they read Time and Offset, and the
    // synth writes ambient time for them. `color` is a Generate node with a `cpu` half and no
    // function at all: it publishes a uniform color, and `tests/uniform.rs` holds what it
    // publishes.
    let groups = [
        (Category::Generate, 24),
        (Category::Color, 21),
        (Category::Transform, 22),
        (Category::Effect, 21),
        (Category::Convert, 14),
        (Category::Tap, 0),
        (Category::Math, 18),
    ];
    for (category, expected) in groups {
        let count = nodes::REGISTRY
            .iter()
            .filter(|d| d.category == category && d.cpu.is_none())
            .count();
        assert_eq!(count, expected, "the {} library", category.label());
    }
    let held =
        |def: &nodes::NodeDef| def.cpu.is_none() && groups.iter().any(|(c, _)| *c == def.category);

    let mut written = 0;
    for def in nodes::REGISTRY {
        let held = held(def);
        let has_picture = def
            .outputs
            .iter()
            .any(|p| matches!(p.ty, PortType::VaryingColor | PortType::VaryingNumber));
        if !has_picture {
            assert!(
                !held,
                "{}: a held node publishes a picture or a field",
                def.slug
            );
            continue;
        }
        let mut builds = Vec::new();
        for wired in [false, true] {
            if def.is_output && !wired {
                continue;
            }
            let (g, out) = node_graph(def, wired, held);
            let shader = compile::wgsl::build(&g, out).unwrap_or_else(|| panic!("{}", def.slug));
            builds.push((if wired { "connected" } else { "unconnected" }, shader));
        }
        let untranslated = builds.iter().any(|(_, s)| {
            s.diagnostics
                .iter()
                .any(|d| matches!(d, compile::Diagnostic::Untranslated { .. }))
        });
        if untranslated && !held {
            continue;
        }
        written += 1;
        for (wiring, shader) in builds {
            assert!(
                shader.diagnostics.is_empty(),
                "{}: {:?}",
                def.slug,
                shader.diagnostics
            );
            let wgsl = shader
                .body
                .strip_prefix(compile::wgsl::PRELUDE)
                .expect("every module starts with the prelude");
            let group = def.category.label().to_lowercase();
            insta::assert_snapshot!(format!("wgsl_{group}_{}_{wiring}", def.slug), wgsl);
        }
    }
    assert!(written > 0, "some node has WGSL");
}

/// `def` into an Output: every input driven when `wired`, and every output it has gathered
/// into one picture — the pictures folded together through `mix`, the fields through the
/// channels of an `rgba`. A cable that does not land panics where `strict` says so and is
/// left out where it does not.
fn node_graph(def: &'static nodes::NodeDef, wired: bool, strict: bool) -> (Graph, NodeId) {
    let connect = |g: &mut Graph, from: PortRef, to: PortRef| {
        let result = g.connect(from, to);
        if strict {
            result.unwrap_or_else(|e| panic!("{}.{}: {e}", def.slug, to.key));
        }
    };
    let mut g = Graph::new();
    let under = add(&mut g, def.slug);
    let out = add(&mut g, "output");
    if wired {
        for port in def.inputs {
            let (source, from) = match port.ty {
                PortType::VaryingColor => ("checkerboard", "output"),
                PortType::VaryingNumber => FIELD,
                PortType::UniformColor => ("color", "output"),
                PortType::UniformNumber | PortType::Action => continue,
            };
            let src = add(&mut g, source);
            connect(
                &mut g,
                PortRef::new(src, from),
                PortRef::new(under, port.key),
            );
        }
    }
    // Every output the node has, gathered into one shader: the pictures folded together
    // through `mix`, the fields through the channels of an `rgba`. A generator that is never
    // reached is a generator that is never checked, and a node with a second picture — a
    // fractal's `map` — has one that the first color port alone would leave out.
    let mut roots: Vec<PortRef> = def
        .outputs
        .iter()
        .filter(|p| p.ty == PortType::VaryingColor)
        .map(|p| PortRef::new(under, p.key))
        .collect();
    let fields: Vec<&str> = def
        .outputs
        .iter()
        .filter(|p| p.ty == PortType::VaryingNumber)
        .map(|p| p.key)
        .collect();
    assert!(
        !roots.is_empty() || !fields.is_empty(),
        "{}: the node publishes a picture or a field",
        def.slug
    );
    if !fields.is_empty() {
        let rgba = add(&mut g, "rgba");
        for (field, channel) in fields.iter().zip(["r", "g", "b", "a"]) {
            connect(
                &mut g,
                PortRef::new(under, field),
                PortRef::new(rgba, channel),
            );
        }
        roots.push(PortRef::new(rgba, "output"));
    }
    let mut root = roots[0];
    for next in &roots[1..] {
        let blend = add(&mut g, "mix");
        g.connect(root, PortRef::new(blend, "a")).unwrap();
        g.connect(*next, PortRef::new(blend, "b")).unwrap();
        root = PortRef::new(blend, "output");
    }
    g.connect(root, PortRef::new(out, "input")).unwrap();
    (g, out)
}

/// Whether `body` adds `offset` to `time`, a count `vec2f(whole, fraction)`, through the
/// prelude's helper for its period: to its fraction on a periodic node, to the two parts on a
/// line, or to its whole part taken modulo the tunnel's 64 and then its fraction.
fn added(body: &str, time: &str, offset: &str) -> bool {
    [
        format!("time_periodic({time}, {offset})"),
        format!("time_unbounded({time}, {offset})"),
        format!("time_repeat({time}, 64.0, {offset})"),
    ]
    .iter()
    .any(|sum| body.contains(sum.as_str()))
}

/// **Time and Offset are ordinary holes.** Time is a count, a whole part and a fraction.
/// Unconnected, a node's Time is what the synth publishes under the node's own key — the
/// ambient reading, or a free-running node's own playhead — and Offset its knob, added; a gear
/// cabled into a looping node's Time is the gear's Cycles in its place; a field cabled into
/// Offset is a call, added per pixel. Speed is never asked for: the synth folds it into Time.
#[test]
fn a_time_driven_node_reads_time_and_adds_offset() {
    for (slug, root_key) in [
        ("perlin", "color"),
        ("cosinegradient", "output"),
        ("shakycam", "output"),
        ("rotozoom", "output"),
        ("tunnel3d", "output"),
    ] {
        // Unconnected: the ambient reading and the knob.
        let mut g = Graph::new();
        let under = add(&mut g, slug);
        let out = add(&mut g, "output");
        g.connect(PortRef::new(under, root_key), PortRef::new(out, "input"))
            .unwrap();
        let shader = compile::wgsl::build(&g, out).expect("connected to the Output");
        let time = format!("u_count_{slug}{under}_clock");
        assert_eq!(
            shader.uniforms.get(time.as_str()),
            Some(&UniformProvider::NodeCount {
                node: under,
                port: "clock",
            }),
            "{slug} unconnected reads the ambient reading published under its own Time"
        );
        let offset = format!("u.u_control_{slug}{under}_phaseOffset");
        assert!(
            added(&shader.body, &format!("u.{time}"), &offset),
            "{slug} adds its Offset knob to its Time, u.{time} and {offset}: {}",
            shader.body
        );

        // A gear in Time, a field in Offset.
        let mut g = Graph::new();
        let under = add(&mut g, slug);
        g.get_mut(under).unwrap().options.insert(
            supersilvia::nodes::timing::MODE.key,
            supersilvia::nodes::timing::LOOP.to_string(),
        );
        let gear = add(&mut g, "ratiogear");
        let field = add(&mut g, FIELD.0);
        g.connect(
            PortRef::new(gear, "cycles"),
            PortRef::new(under, supersilvia::nodes::TIME),
        )
        .expect("a gear's Cycles feed Time");
        g.connect(
            PortRef::new(field, FIELD.1),
            PortRef::new(under, supersilvia::nodes::timing::OFFSET),
        )
        .expect("Offset is a varying number input");
        let out = add(&mut g, "output");
        g.connect(PortRef::new(under, root_key), PortRef::new(out, "input"))
            .unwrap();
        let shader = compile::wgsl::build(&g, out).expect("connected to the Output");
        let (cycles, field) = (
            format!("u.u_count_ratiogear{gear}_cycles"),
            format!("{}{field}_{}(uv)", FIELD.0, FIELD.1),
        );
        assert!(
            added(&shader.body, &cycles, &field),
            "{slug} reads the gear in place of its Time and adds the field, {cycles} and \
             {field}: {}",
            shader.body
        );
        assert!(
            !shader
                .uniforms
                .contains_key(format!("u_count_{slug}{under}_clock").as_str()),
            "{slug}: a cabled Time reads no ambient reading"
        );
    }
}

/// **A field into Time is a type mismatch**: Time is one number a frame, the moment the node is
/// at, and a per-pixel lag is Offset's job.
#[test]
fn a_field_cannot_drive_time() {
    let mut g = Graph::new();
    let perlin = add(&mut g, "perlin");
    let field = add(&mut g, FIELD.0);
    assert!(
        g.connect(
            PortRef::new(field, FIELD.1),
            PortRef::new(perlin, supersilvia::nodes::TIME),
        )
        .is_err()
    );
}

// ---------------------------------------------------------------------------------------
// Option kinds against the shader they do or do not change.
//
// `OptionDef::kind` decides whether `SetOption` rebuilds, and both ways of getting it wrong
// have a cost. Marking a code-changing option as anything else leaves the *running shader
// describing a graph that no longer exists*, which is a wrong picture and the reason `Code`
// is the default. Marking an inert one `Code` is a driver recompile in the middle of a set
// for a shader that comes out identical. So both directions are asserted here rather than
// left to whoever adds the next node.

/// Every WGSL module this node produces with `key` set to `value`, over the wired and
/// unwired shapes and every color root — the same construction
/// `every_node_compiles_on_the_gpu` uses, without a driver.
fn sources_with(def: &'static nodes::NodeDef, key: &'static str, value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for wired in [false, true] {
        let mut g = Graph::new();
        let Some(under) = nodes::add_to_graph(&mut g, def.slug, Pos2::ZERO) else {
            continue;
        };
        let Some(sink) = nodes::add_to_graph(&mut g, "output", Pos2::ZERO) else {
            continue;
        };
        g.get_mut(under)
            .unwrap()
            .options
            .insert(key, value.to_string());

        if wired {
            for port in def.inputs {
                let (source, from) = match port.ty {
                    PortType::VaryingColor => ("checkerboard", "output"),
                    PortType::VaryingNumber => FIELD,
                    PortType::UniformColor => ("color", "output"),
                    PortType::UniformNumber | PortType::Action => continue,
                };
                let src = nodes::add_to_graph(&mut g, source, Pos2::ZERO).unwrap();
                let _ = g.connect(PortRef::new(src, from), PortRef::new(under, port.key));
            }
        }

        let mut roots: Vec<PortRef> = def
            .outputs
            .iter()
            .filter(|p| p.ty == PortType::VaryingColor)
            .map(|p| PortRef::new(under, p.key))
            .collect();
        if roots.is_empty() {
            let rgba = nodes::add_to_graph(&mut g, "rgba", Pos2::ZERO).unwrap();
            for (field, channel) in def
                .outputs
                .iter()
                .filter(|p| p.ty == PortType::VaryingNumber)
                .zip(["r", "g", "b", "a"])
            {
                let _ = g.connect(PortRef::new(under, field.key), PortRef::new(rgba, channel));
            }
            if g.source_of(PortRef::new(rgba, "r")).is_some() {
                roots.push(PortRef::new(rgba, "output"));
            }
        }

        for root in roots {
            if g.connect(root, PortRef::new(sink, "input")).is_err() {
                continue;
            }
            if let Some(shader) = compile::wgsl::build(&g, sink) {
                out.push(shader.source());
            }
        }
        // A measurement is its pass's, so an option it reads rebuilds the pass.
        if def.measure_wgsl.is_some() {
            out.extend(
                compile::wgsl::build_pass(&g, g.default_workspace(), &[under], false)
                    .iter()
                    .map(compile::Shader::source),
            );
        }
    }
    out
}

/// Whether any pair of `option`'s values produces a different module.
fn changes_the_shader(def: &'static nodes::NodeDef, option: &'static nodes::OptionDef) -> bool {
    // An asset holds a path rather than a choice, so two paths stand in for two choices.
    let values: Vec<&str> = if option.choices.is_empty() {
        vec!["assets/one.webm", "assets/two.webm"]
    } else {
        option.choices.iter().map(|(v, _)| *v).collect()
    };
    let baseline = sources_with(def, option.key, values[0]);
    values[1..]
        .iter()
        .any(|v| sources_with(def, option.key, v) != baseline)
}

/// A `Code` option earns its rebuild. One that does not is a recompile for a shader that
/// comes out character for character the same, and belongs in another kind.
#[test]
fn every_code_option_changes_the_shader() {
    for def in nodes::REGISTRY {
        for option in def.options {
            if !option.rebuilds() || option.choices.len() < 2 {
                continue;
            }
            assert!(
                changes_the_shader(def, option),
                "{}.{} is `Code`, so changing it rebuilds every Output downstream — but its \
                 choices all generate the same WGSL. It wants `Uniform`, `Runtime`, \
                 `Presentation` or `Asset`.",
                def.slug,
                option.key,
            );
        }
    }
}

/// The half that is about correctness rather than speed: an option that is *not* `Code` does
/// not rebuild, so if its value reached the generated WGSL the shader on screen would go
/// stale the moment it changed.
#[test]
fn no_option_outside_code_reaches_the_shader() {
    for def in nodes::REGISTRY {
        for option in def.options {
            if option.rebuilds() {
                continue;
            }
            assert!(
                !changes_the_shader(def, option),
                "{}.{} is {:?}, so `SetOption` does not rebuild — but changing it changes the \
                 generated WGSL, which would leave the old shader running. Make it `Code`, or \
                 read it through `ctx.option_uniform` instead of `ctx.option`.",
                def.slug,
                option.key,
                option.kind,
            );
        }
    }
}

/// The probe is the real shader with one count at the top of every node function, and a
/// slot for every node. Neither measures anything — a measurement is its pass's — so a
/// measuring node's slot in a probe is its evaluations like anybody else's.
#[test]
fn a_probe_counts_every_function_once_into_its_own_slot() {
    use supersilvia::compile::{EVAL_WORD, TAP_WORDS, TapKind};

    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let blur = add(&mut g, "blur");
    let tap = add(&mut g, "tap");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(blur, "input"))
        .unwrap();
    g.connect(PortRef::new(blur, "output"), PortRef::new(tap, "input"))
        .unwrap();
    g.connect(PortRef::new(tap, "output"), PortRef::new(out, "input"))
        .unwrap();

    let plain = compile::wgsl::build(&g, out).unwrap();
    assert!(
        !plain.body.contains(&format!("+ {EVAL_WORD}], 1u)")),
        "the real shader counts nothing"
    );
    assert!(plain.taps.is_empty(), "the tap's measurement is its pass's");

    let probe = compile::wgsl::build_probe(&g, out).unwrap();
    assert!(probe.diagnostics.is_empty(), "{:?}", probe.diagnostics);
    // Three functions, and three call sites: the blur's, the tap's, and the Output's own.
    let functions = probe
        .taps
        .iter()
        .filter(|(_, k)| !matches!(k, TapKind::Taps(_)))
        .count();
    assert_eq!(functions, 3, "a slot per node: {:?}", probe.taps);
    assert_eq!(
        probe.taps.len(),
        6,
        "and one per call site: {:?}",
        probe.taps
    );
    let kind_of = |id| {
        probe
            .taps
            .iter()
            .find(|(n, k)| *n == id && !matches!(k, TapKind::Taps(_)))
            .map(|(_, k)| *k)
    };
    assert_eq!(
        kind_of(tap),
        Some(TapKind::Evaluations),
        "a probe measures nothing, so the tap's slot counts its function",
    );
    assert_eq!(kind_of(cb), Some(TapKind::Evaluations));
    assert_eq!(kind_of(blur), Some(TapKind::Evaluations));
    let counts = probe.body.matches(&format!("+ {EVAL_WORD}], 1u)")).count();
    assert_eq!(counts, 6, "one count per function and per call site");
    for (i, _) in probe.taps.iter().enumerate() {
        assert!(
            probe.body.contains(&format!(
                "atomicAdd(&tap[{} + {EVAL_WORD}], 1u)",
                i * TAP_WORDS
            )),
            "slot {i} is counted"
        );
    }
    assert!(
        probe
            .body
            .contains("var<storage, read_write> tap: array<atomic<u32>>;"),
        "the probe declares the buffer"
    );
}

/// The probe also counts each call site, so a consumer's taps can be read off it: the blur
/// calling the checkerboard is a slot of its own, separate from either node's evaluations.
#[test]
fn a_probe_counts_each_call_site_in_its_own_slot() {
    use supersilvia::compile::TapKind;

    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let blur = add(&mut g, "blur");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(blur, "input"))
        .unwrap();
    g.connect(PortRef::new(blur, "output"), PortRef::new(out, "input"))
        .unwrap();
    let probe = compile::wgsl::build_probe(&g, out).unwrap();
    assert!(
        probe.taps.contains(&(blur, TapKind::Taps("input"))),
        "{:?}",
        probe.taps
    );
    assert!(probe.taps.contains(&(blur, TapKind::Evaluations)));
    assert!(probe.taps.contains(&(cb, TapKind::Evaluations)));
    assert!(
        probe.taps.contains(&(out, TapKind::Taps("input"))),
        "the Output's own call of its input is a call site too"
    );
    assert_eq!(probe.taps.len(), 4);
    // The call is still the call: counted, then made.
    assert!(
        probe
            .body
            .contains(&format!("1u);\n    return checkerboard{cb}_output(uv);")),
        "the call site is wrapped, not replaced:\n{}",
        probe.body
    );
    let plain = compile::wgsl::build(&g, out).unwrap();
    assert!(
        !plain.body.contains("_via"),
        "the real shader wraps no call"
    );
}

// ---------------------------------------------------------------------------------------
// Dual outputs: one node, two ways of compiling.

/// An `add` feeding a `zoom`, and the `zoom` feeding an Output.
fn add_into_a_zoom() -> (Graph, NodeId, NodeId) {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let zoom = add(&mut g, "zoom");
    let sum = add(&mut g, "add");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(zoom, "input"))
        .unwrap();
    g.connect(PortRef::new(sum, "output"), PortRef::new(zoom, "zoom"))
        .unwrap();
    g.connect(PortRef::new(zoom, "output"), PortRef::new(out, "input"))
        .unwrap();
    (g, sum, out)
}

/// In diamond mode the `add` is the uniform number branch of `CompileContext::input`: a
/// bare member of the uniform struct, and no function at all — the same shape a number control already
/// compiles to.
#[test]
fn a_diamond_add_is_a_uniform_and_emits_no_function() {
    let (g, sum, out) = add_into_a_zoom();
    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert!(
        shader
            .body
            .contains(&format!("    u_float_add{sum}_output: f32,\n")),
        "{}",
        shader.body
    );
    assert!(
        !shader.body.contains(&format!("fn add{sum}_output(")),
        "no function is emitted for a node the tick evaluated"
    );
}

/// In circle mode it is an ordinary field node: one function, called by its consumer.
#[test]
fn a_circle_add_emits_its_function() {
    let (mut g, sum, out) = add_into_a_zoom();
    let luma = add(&mut g, "luminosity");
    g.connect(PortRef::new(luma, "output"), PortRef::new(sum, "a"))
        .expect("a field into A");

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert!(
        shader
            .body
            .contains(&format!("fn add{sum}_output(uv: vec2f) -> f32")),
        "{}",
        shader.body
    );
    assert!(
        !shader.body.contains(&format!("u_float_add{sum}_output")),
        "and no uniform beside it"
    );
    // The knob on the other input is a control uniform again, as it is on any field node.
    assert!(
        shader
            .uniforms
            .contains_key(format!("u_control_add{sum}_b").as_str())
    );
}

/// `triggeredcolor`'s `color` is one `vec4f` uniform, not four floats assembled per fragment
/// — `autoexposure`'s pattern, a `VaryingColor` body reading the node's own `r`, `g`, `b` and
/// `a` back through `own_uniform`. The cable lands, because a uniform color feeds a varying
/// color input for free, and none of the four floats is named, because nothing asks for them.
#[test]
fn triggeredcolors_color_is_one_vec4_uniform() {
    let mut g = Graph::new();
    let tc = add(&mut g, "triggeredcolor");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(tc, "color"), PortRef::new(out, "input"))
        .expect("a uniform color into a varying color input");

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    let name = format!("u_color_triggeredcolor{tc}_color");
    assert_eq!(
        shader.uniforms.get(name.as_str()),
        Some(&UniformProvider::NodeUniform {
            node: tc,
            port: "color",
            ty: UniformType::Vec4,
        }),
    );
    for channel in ["r", "g", "b", "a"] {
        assert!(
            !shader
                .uniforms
                .contains_key(format!("u_float_triggeredcolor{tc}_{channel}").as_str()),
            "{channel}: nothing reads it, so nothing declares it"
        );
    }
    insta::assert_snapshot!(shader.body);
}

/// `method` and `curve` are `OptionKind::Uniform`, so every branch of `mix` is in the
/// compiled program whatever is chosen, and a change costs a uniform write rather than a
/// rebuild. That is the whole point of the kind, and this is where it is held: both options
/// reach the uniform table under their own names, and the body branches on them rather than
/// on a constant the generator folded in.
#[test]
fn a_uniform_option_reaches_the_shader_as_a_uniform_the_body_branches_on() {
    let mut g = Graph::new();
    let mix = add(&mut g, "mix");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(mix, "output"), PortRef::new(out, "input"))
        .expect("color into color");

    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    for key in ["method", "curve"] {
        let name = format!("u_opt_mix{mix}_{key}");
        assert_eq!(
            shader.uniforms.get(name.as_str()),
            Some(&UniformProvider::Option { node: mix, key }),
            "`{key}` is not in the uniform table",
        );
        assert!(
            shader.body.contains(&name),
            "the body does not name `{key}`'s uniform: {}",
            shader.body
        );
    }
}

// ------------------------------------------------------------------------------ the module

/// The smallest graph that draws, as the whole WGSL module the wgpu renderer is handed.
#[test]
fn checkerboard_into_output_compiles_to_wgsl() {
    let (g, _, out) = checkerboard_into_output();
    let shader = compile::wgsl::build(&g, out).expect("input is connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    assert_eq!(
        shader.source(),
        shader.body,
        "the source is the module, with nothing prepended"
    );
    insta::assert_snapshot!(shader.body);
}

/// WGSL has no sequence operator, so the probe counts a call site through a wrapper that
/// counts and calls through, one per call site. The wrapper is named by its slot, and the
/// counted word is the slot's [`compile::EVAL_WORD`].
#[test]
fn a_wgsl_probe_counts_each_call_site_through_a_wrapper() {
    let (g, cb, out) = checkerboard_into_output();
    let wgsl = compile::wgsl::build_probe(&g, out).expect("connected");
    let call = wgsl
        .taps
        .iter()
        .position(|(_, k)| matches!(k, compile::TapKind::Taps(_)))
        .expect("the Output's call into the checkerboard is counted");
    let base = call * compile::TAP_WORDS;
    let via = format!("checkerboard{cb}_output_via{call}");
    assert!(
        wgsl.body.contains(&format!(
            "fn {via}(uv: vec2f) -> vec4f {{\n    atomicAdd(&tap[{base} + {}], 1u);\n    return checkerboard{cb}_output(uv);\n}}",
            compile::EVAL_WORD
        )),
        "{}",
        wgsl.body
    );
    assert!(
        wgsl.body.contains(&format!("return {via}(uv);")),
        "{}",
        wgsl.body
    );
}

/// A module's bindings and its uniform struct are computable from the `Shader` alone: the
/// struct at 0, the four samplers where there is a texture, the textures from `TEXTURES` up
/// in name order, each uniform at its WGSL offset — a `vec4f` after a lone `f32` starts on
/// the next sixteen.
#[test]
fn a_wgsl_module_says_where_everything_is_bound() {
    use compile::wgsl::{self, Resource, Sampler};
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    g.connect(PortRef::new(cb, "output"), PortRef::new(a, "input"))
        .unwrap();
    g.connect(PortRef::new(a, "frame"), PortRef::new(b, "input"))
        .unwrap();
    let shader = wgsl::build(&g, b).expect("b's input is connected");
    let texture = format!("u_texture_output{a}_frame");
    assert_eq!(
        wgsl::bindings(&shader),
        vec![
            (0, Resource::Uniforms),
            (2, Resource::Sampler(Sampler::MirrorLinear)),
            (3, Resource::Sampler(Sampler::MirrorNearest)),
            (4, Resource::Sampler(Sampler::RepeatLinear)),
            (5, Resource::Sampler(Sampler::RepeatNearest)),
            (wgsl::TEXTURES, Resource::Texture(texture.as_str().into())),
        ]
    );
    assert!(shader.body.contains(&format!(
        "textureSampleLevel({texture}, sampler_mirror_linear, t, 0.0)"
    )));

    let (g, _, out) = checkerboard_into_output();
    let shader = wgsl::build(&g, out).expect("connected");
    let layout = wgsl::uniform_layout(&shader);
    let at: Vec<(&str, u32)> = layout.fields.iter().map(|f| (&*f.name, f.offset)).collect();
    assert_eq!(
        at,
        [
            ("u_resolution", 0),
            ("u_time", 8),
            ("u_control_checkerboard1_color1", 16),
            ("u_control_checkerboard1_color2", 32),
            ("u_control_checkerboard1_frequency", 48),
        ]
    );
    assert_eq!(layout.size, 64, "rounded up to the vec4f's alignment");
}

/// A node with no WGSL still compiles: its function is a placeholder of the right type and
/// the module says which node it stands in for.
#[test]
fn a_node_without_wgsl_compiles_to_a_placeholder_that_says_so() {
    let untranslated = nodes::REGISTRY.iter().find(|d| {
        d.cpu.is_none() && d.outputs.iter().any(|o| o.ty == PortType::VaryingColor) && {
            let (g, out) = node_graph(d, false, false);
            compile::wgsl::build(&g, out).is_some_and(|s| !s.diagnostics.is_empty())
        }
    });
    let Some(def) = untranslated else {
        // Every node has WGSL, and there is nothing left to stand in for.
        return;
    };
    let (g, out) = node_graph(def, false, false);
    let shader = compile::wgsl::build(&g, out).expect("connected");
    assert!(
        shader.diagnostics.iter().any(|d| matches!(
            d,
            compile::Diagnostic::Untranslated { slug, .. } if *slug == def.slug
        )),
        "{:?}",
        shader.diagnostics
    );
    assert!(shader.body.contains("return vec4f(1.0, 0.0, 1.0, 1.0);"));
    assert!(
        compile::wgsl::build(&g, out)
            .expect("connected")
            .diagnostics
            .is_empty()
    );
}

/// **A crowded workspace is drawn in batches.** Twenty cellular automata each bind their own
/// state texture, more than one pass may: the pass comes back in batches, each binding no more
/// than `PASS_TEXTURES`, every thumbnail and every measurement in exactly one, the
/// measurements' slots first in whichever batch holds them.
#[test]
fn a_workspace_binding_more_textures_than_a_pass_may_is_split_into_batches() {
    use std::collections::BTreeSet;
    let mut g = Graph::new();
    let cells: Vec<NodeId> = (0..20).map(|_| add(&mut g, "cellularautomata")).collect();
    let cb = add(&mut g, "checkerboard");
    let near = add(&mut g, "tap");
    let far = add(&mut g, "tap");
    g.connect(PortRef::new(cb, "output"), PortRef::new(near, "input"))
        .unwrap();
    g.connect(
        PortRef::new(cells[19], "output"),
        PortRef::new(far, "input"),
    )
    .unwrap();

    let batches = compile::wgsl::build_pass(&g, g.default_workspace(), &[near, far], true);
    assert!(batches.len() >= 2, "{} batches", batches.len());
    let mut ports = Vec::new();
    let mut measured = Vec::new();
    for batch in &batches {
        assert!(batch.diagnostics.is_empty(), "{:?}", batch.diagnostics);
        let textures = compile::wgsl::bindings(batch)
            .iter()
            .filter(|(_, r)| matches!(r, compile::wgsl::Resource::Texture(_)))
            .count();
        assert!(
            textures <= compile::PASS_TEXTURES,
            "a batch binds {textures} textures"
        );
        assert_eq!(batch.thumb_base(), batch.taps.len() * compile::TAP_WORDS);
        ports.extend(batch.thumbs.iter().map(|(p, _)| *p));
        measured.extend(batch.taps.iter().map(|(n, _)| *n));
    }
    let every: BTreeSet<PortRef> = g
        .iter()
        .flat_map(|(id, n)| {
            n.outputs
                .iter()
                .filter(|p| p.ty.is_varying())
                .map(move |p| PortRef::new(id, p.key))
        })
        .collect();
    assert_eq!(ports.len(), every.len(), "each port once");
    assert_eq!(ports.iter().copied().collect::<BTreeSet<_>>(), every);
    assert_eq!(measured, [near, far], "each measurement once, in order");

    // A workspace that binds few enough stays one module.
    let mut sparse = Graph::new();
    for _ in 0..4 {
        add(&mut sparse, "cellularautomata");
    }
    assert_eq!(
        compile::wgsl::build_pass(&sparse, sparse.default_workspace(), &[], true).len(),
        1
    );
}
