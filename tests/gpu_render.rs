// SPDX-License-Identifier: AGPL-3.0-or-later

//! Graphs drawn over many ticks by the renderer: a real feedback loop through the camcorder, an idle Output and one
//! switched to, and an Output whose program changes under it — the uniforms, textures and tap
//! slots a program draws with are its own source's, whatever the job names.
//!
//! The decisions the synth makes — which Outputs draw, a render's warm-up, a Snap's file — are
//! played here by hand, the way the synth plays them; the tests that need the app itself are
//! `tests/gpu_app.rs`.

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain, floats_of, job, link_all, module, rgba_of, tick};
use std::fmt::Write as _;
use std::sync::Arc;
use supersilvia::compile::wgsl;
use supersilvia::compile::{
    Shader, TAP_TEMPLATE, TAP_WORDS, TapKind, UniformProvider, UniformType,
};
use supersilvia::graph::{ControlValue, Graph, NodeId, PortRef};
use supersilvia::nodes::{self, Frame};
use supersilvia::render::{FrameJob, OutputJob, OutputMode, Renderer, SourceJob, UniformValue};

const DRAW: OutputMode = OutputMode::Draw;

/// Every uniform of `shader` as the synth resolves it: a control and an option off the graph,
/// a texture by the port that publishes it, and every published uniform number `published`.
fn resolve(g: &Graph, shader: &Shader, published: f32) -> Vec<(Arc<str>, UniformValue)> {
    shader
        .uniforms
        .iter()
        .filter_map(|(name, provider)| {
            let value = match provider {
                UniformProvider::Control { node, key, .. } => {
                    match g.get(*node)?.controls.get(key)? {
                        ControlValue::Float(v) => UniformValue::Float(*v),
                        ControlValue::Color(v) => {
                            UniformValue::Vec4(supersilvia::nodes::alpha::premultiply(*v))
                        }
                    }
                }
                UniformProvider::NodeTexture { node, port } => {
                    UniformValue::NodeTexture(PortRef::new(*node, port))
                }
                UniformProvider::NodeUniform { ty, .. } => match ty {
                    UniformType::Vec4 => UniformValue::Vec4([0.0; 4]),
                    _ => UniformValue::Float(published),
                },
                UniformProvider::NodeCount { .. } => {
                    UniformValue::Vec2(supersilvia::nodes::phasor::split(f64::from(published)))
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

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, emath::Pos2::ZERO).expect("in the registry")
}

// ------------------------------------------------------------------ the camcorder's loop

/// `camcordercrt`'s picture as silvia draws it and as this node drew it until its taps were
/// made conditional: all seven reads of `Input` a fragment, whatever Glow and Aberration are.
/// `$u_` is the node's control-uniform prefix, `$input` the function feeding `Input` and
/// `$last` the one feeding `Last Frame`, which is all that names a node in a module.
const SEVEN_TAP_CAMCORDER: &str = "    let curv = $u_curvature;
    let vig = $u_vignette;
    let r2 = dot(uv, uv);
    let crtUV = uv * (1.0 + r2 * curv * 0.3);
    let edgeFade = 1.0 - smoothstep(0.92, 1.0, abs(crtUV.y));
    let vigAmount = clamp(1.0 - r2 * vig * vig, 0.0, 1.0);
    let aber = $u_aberration;
    let scan = $u_scanlines;
    let glw = $u_glow;
    let abDist = length(crtUV);
    let abDir = select(vec2f(1.0, 0.0), crtUV / abDist, abDist > 0.001);
    let abOffset = aber * abDist * 0.04;
    let abLow = crtUV - abDir * abOffset;
    let abHigh = crtUV + abDir * abOffset;
    var rgb = vec3f($input(abLow).r, $input(crtUV).g, $input(abHigh).b);
    let glowRadius = 6.0 / 720.0;
    let bloomSample = ($input(crtUV + vec2f(glowRadius, 0.0)) + $input(crtUV - vec2f(glowRadius, 0.0)) + $input(crtUV + vec2f(0.0, glowRadius)) + $input(crtUV - vec2f(0.0, glowRadius))) * 0.25;
    rgb += max(bloomSample.rgb - rgb, vec3f(0.0)) * glw;
    let pixelY = (crtUV.y + 1.0) * 0.5 * 720.0;
    let scanLine = pow(sin(pixelY * PI) * 0.5 + 0.5, 1.5);
    rgb *= mix(vec3f(1.0), vec3f(scanLine), scan);
    let pixelX = (crtUV.x + 1.0) * 0.5 * 720.0;
    let subpixel = floor_mod(floor(pixelX), 3.0);
    let phosphorTint = vec3f(
        select(0.85, 1.0, subpixel == 0.0),
        select(0.85, 1.0, subpixel == 1.0),
        select(0.85, 1.0, subpixel == 2.0)
    );
    rgb *= mix(vec3f(1.0), phosphorTint, scan * 0.5);
    rgb *= $u_brightness;
    rgb *= vigAmount;
    rgb.r *= 1.05;
    rgb.b *= 0.92;
    rgb *= edgeFade;
    var camera = uv * 0.5;
    camera /= $u_fbZoom;
    let roll = $u_fbRotation;
    let cr = cos(roll);
    let sr = sin(roll);
    camera = vec2f(camera.x * cr - camera.y * sr, camera.x * sr + camera.y * cr);
    let pw = max(1.0 + camera.x * ($u_fbTiltX) + camera.y * ($u_fbTiltY), 0.001);
    camera /= pw;
    camera += vec2f($u_fbDriftX, $u_fbDriftY);
    let fbUV = camera * 2.0;
    let prevFrame = $last(fbUV);
    let prevRgb = (prevFrame.rgb - 0.5) * ($u_fbContrast) + 0.5;
    rgb += prevRgb * ($u_fbAmount);
    return vec4f(rgb, 1.0);
";

/// Frames of a feedback loop, read back as the half floats they were drawn in: `frames` draws
/// of the one Output of `g`, whose own `frame` is sampled as it draws, the Perlin under it
/// walking with the clock.
fn feedback_frames(
    g: &Graph,
    out: NodeId,
    shader: &Arc<Shader>,
    size: (u32, u32),
    frames: usize,
) -> Vec<Vec<f32>> {
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let values = resolve(g, shader, 0.0);
    let outputs = |send: bool, mode| vec![job(out, size, shader, send, mode, values.clone())];
    link_all(&mut renderer, &outputs);
    let mut drawn = Vec::new();
    for frame in 0..frames {
        let phase = frame as f32 * 0.05;
        let values = resolve(g, shader, phase);
        renderer.draw(&tick(
            phase,
            vec![job(out, size, shader, false, DRAW, values)],
        ));
        drain(&gpu);
        drawn.push(floats_of(&gpu, &renderer.texture_of(out).expect("drawn")));
    }
    drawn
}

/// **Skipping the taps that cannot change the picture changes nothing in it.** `camcordercrt`
/// reads `Input` seven times in silvia and only where the read shows: no glow taps at Glow 0,
/// one fringe tap at Aberration 0. This draws the demo's patch — a Perlin into the camcorder,
/// the Output's frame back into Last Frame, zoomed and rolled so the loop moves — once with
/// the node as it is and once with silvia's seven-tap body swapped into the same module, over
/// eight frames of feedback through the renderer's ring, at each corner of the two switches,
/// and holds the two to within a half float's rounding.
#[test]
fn the_camcorder_draws_the_seven_tap_picture_with_fewer_taps() {
    for (aberration, glow) in [(1.0, 0.0), (0.0, 0.0), (0.0, 0.8), (1.5, 0.8)] {
        let mut g = Graph::new();
        let perlin = add(&mut g, "perlin");
        let crt = add(&mut g, "camcordercrt");
        let out = add(&mut g, "output");
        g.connect(PortRef::new(perlin, "color"), PortRef::new(crt, "input"))
            .unwrap();
        g.connect(PortRef::new(crt, "color"), PortRef::new(out, "input"))
            .unwrap();
        g.connect(PortRef::new(out, "frame"), PortRef::new(crt, "lastFrame"))
            .unwrap();
        g.get_mut(perlin)
            .unwrap()
            .controls
            .insert("scale", ControlValue::Float(4.0));
        let camera = &mut g.get_mut(crt).unwrap().controls;
        for (key, value) in [
            ("aberration", aberration),
            ("glow", glow),
            ("curvature", 0.4),
            ("scanlines", 0.5),
            ("vignette", 0.6),
            ("fbAmount", 0.5),
            ("fbZoom", 1.02),
            ("fbRotation", 0.02),
            ("fbTiltX", 0.3),
            ("fbDriftY", 0.01),
        ] {
            camera.insert(key, ControlValue::Float(value));
        }

        let shader = wgsl::build(&g, out).expect("the Output is connected");
        assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
        let head = format!("fn camcordercrt{crt}_color(uv: vec2f) -> vec4f {{\n");
        let start = shader.body.find(&head).expect("the camcorder's function") + head.len();
        let end = start + shader.body[start..].find("\n}\n").expect("its end") + 1;
        let (prefix, input, last) = (
            format!("u.u_control_camcordercrt{crt}_"),
            format!("perlin{perlin}_color"),
            format!("output{out}_frame"),
        );
        for name in [&prefix, &input, &last] {
            assert!(
                shader.body.contains(name.as_str()),
                "{name} is in the module"
            );
        }
        let seven = SEVEN_TAP_CAMCORDER
            .replace("$u_", &prefix)
            .replace("$input", &input)
            .replace("$last", &last);
        let before = Shader {
            body: format!("{}{seven}{}", &shader.body[..start], &shader.body[end..]),
            ..shader.clone()
        };
        assert_ne!(before.body, shader.body, "the seven-tap body went in");

        let size = (320, 180);
        let now = feedback_frames(&g, out, &Arc::new(shader), size, 8);
        let then = feedback_frames(&g, out, &Arc::new(before), size, 8);
        for (frame, (now, then)) in now.iter().zip(&then).enumerate() {
            let worst = now
                .iter()
                .zip(then)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                worst <= 1.0 / 1024.0,
                "aberration {aberration}, glow {glow}: frame {frame} differs by {worst}"
            );
        }
        let last = now.last().unwrap();
        let lit = last.chunks(4).filter(|p| p[0] > 0.05).count();
        assert!(
            lit > last.len() / 32,
            "aberration {aberration}, glow {glow}: the loop drew a picture, not black"
        );
    }
}

// ------------------------------------------------------------------------ idle Outputs

/// A moving picture — a `perlin`, whose phase its CPU half integrates, into an Output.
fn moving_picture() -> (Graph, NodeId, Arc<Shader>) {
    let mut g = Graph::new();
    let noise = add(&mut g, "perlin");
    let out = add(&mut g, "output");
    g.connect(PortRef::new(noise, "color"), PortRef::new(out, "input"))
        .unwrap();
    let shader = wgsl::build(&g, out).expect("connected");
    assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
    (g, out, Arc::new(shader))
}

/// What a viewer of this Output is shown: the published frame, as it lies.
fn shown(gpu: &supersilvia::render::Gpu, renderer: &mut Renderer, out: NodeId) -> Vec<u8> {
    let published = renderer.publish();
    let picture = published.outputs.get(&out).expect("a picture is published");
    bytes_of(gpu, picture.texture.texture())
}

/// **An idle Output is shown its last frame, and switched to draws what continuous drawing
/// would have.** Two renderers holding the same patch tick in lockstep: one Output is drawn
/// throughout, as a tab looked at is; the other is idle until the switch, drawn only where
/// `Renderer::wants_picture` says a picture is owed — which is the synth's rule for an idle
/// Output, played here by hand — so it drew once, its first picture once its program had
/// landed, and nothing after. A viewer is shown that picture and never the blank ring it was
/// made with. The picture is a function of the clock, so the frame drawn on the tick after the
/// switch is the other's to the bit.
///
/// The synth's half — that the tab looked at is what decides — is `tests/gpu_app.rs`'s.
#[test]
fn a_tab_switched_to_draws_what_continuous_drawing_draws() {
    let (g, out, shader) = moving_picture();
    let (idle_gpu, seen_gpu) = (gpu::gpu(), gpu::gpu());
    let mut idle = Renderer::new(idle_gpu.clone()).expect("renderer");
    let mut seen = Renderer::new(seen_gpu.clone()).expect("renderer");
    let size = (160, 90);
    let at = |send: bool, mode, n: u32| {
        let phase = n as f32 * 0.05;
        vec![job(
            out,
            size,
            &shader,
            send,
            mode,
            resolve(&g, &shader, phase),
        )]
    };
    link_all(&mut idle, &|send, mode| at(send, mode, 0));
    link_all(&mut seen, &|send, mode| at(send, mode, 0));

    let mut n = 0u32;
    // One tick of each in lockstep, each finished, the idle one drawn only where it owes a
    // picture unless it is `looked_at`.
    let mut lockstep = |idle: &mut Renderer, seen: &mut Renderer, looked_at: bool| {
        n += 1;
        let mode = if looked_at || idle.wants_picture(out) {
            DRAW
        } else {
            OutputMode::Idle
        };
        let time = n as f32 / 60.0;
        idle.draw(&tick(time, at(false, mode, n)));
        drain(&idle_gpu);
        seen.draw(&tick(time, at(false, DRAW, n)));
        drain(&seen_gpu);
    };

    for _ in 0..5 {
        lockstep(&mut idle, &mut seen, false);
    }
    let first = shown(&idle_gpu, &mut idle, out);
    assert!(
        first.iter().any(|b| *b != 0),
        "a viewer of the idle Output is shown its first picture, not a blank ring"
    );
    let latest = idle.read_output(out).expect("an idle Output has its ring");
    for _ in 0..3 {
        lockstep(&mut idle, &mut seen, false);
        assert_eq!(
            shown(&idle_gpu, &mut idle, out),
            first,
            "and only that: idle, it draws nothing"
        );
    }
    assert_eq!(idle.read_output(out).as_ref(), Some(&latest));
    let before = seen.read_output(out).expect("drawn");
    assert_ne!(before.2, latest.2, "while the one looked at has moved on");

    lockstep(&mut idle, &mut seen, true);
    let now = seen.read_output(out).expect("drawn");
    assert_ne!(now.2, before.2, "the noise moves from tick to tick");
    assert_eq!(
        idle.read_output(out).expect("drawn"),
        now,
        "switched to, it draws the frame continuous drawing draws on the same tick"
    );
    for _ in 0..3 {
        assert!(
            shown(&idle_gpu, &mut idle, out).iter().any(|b| *b != 0),
            "and a viewer is shown a picture on every tick"
        );
        lockstep(&mut idle, &mut seen, true);
    }
}

// --------------------------------------------------------- a program's own source, drawn

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

/// A picture of two textures, `u_a` — node 1's frame — on the left and `u_b` — node 2's — on
/// the right, each scaled by `u_gain`; or, with `a` false, the same picture with `u_a` removed:
/// `u_b` across the whole frame.
fn two_sampler_shader(a: bool) -> Arc<Shader> {
    if a {
        module(
            &["u_gain"],
            &[("u_a", NodeId(1)), ("u_b", NodeId(2))],
            "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let uv = frag_coord.xy / u.u_resolution;
    let a = textureSampleLevel(u_a, sampler_mirror_linear, vec2f(0.5), 0.0);
    let b = textureSampleLevel(u_b, sampler_mirror_linear, vec2f(0.5), 0.0);
    return u.u_gain * select(b, a, uv.x < 0.5);
}
",
        )
    } else {
        module(
            &["u_gain"],
            &[("u_b", NodeId(2))],
            "
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    return u.u_gain * textureSampleLevel(u_b, sampler_mirror_linear, vec2f(0.5), 0.0);
}
",
        )
    }
}

/// One tick of an Output at node 3 drawing `shader` with the uniforms it names — node 1's frame
/// as `u_a`, node 2's as `u_b`, and `u_gain` — beside the sources `published` holds.
fn two_sampler_tick(
    shader: &Arc<Shader>,
    send: bool,
    names: &[&str],
    gain: f32,
    published: &[(u32, [u8; 4])],
) -> FrameJob {
    let uniforms = names
        .iter()
        .map(|name| {
            let value = match *name {
                "u_a" => UniformValue::NodeTexture(PortRef::new(NodeId(1), "frame")),
                "u_b" => UniformValue::NodeTexture(PortRef::new(NodeId(2), "frame")),
                _ => UniformValue::Float(gain),
            };
            (Arc::<str>::from(*name), value)
        })
        .collect();
    FrameJob {
        sources: published
            .iter()
            .map(|(node, rgba)| {
                SourceJob::new(
                    PortRef::new(NodeId(*node), "frame"),
                    Arc::new(Frame::solid(4, 4, *rgba)),
                )
            })
            .collect(),
        ..tick(
            0.0,
            vec![job(NodeId(3), (64, 64), shader, send, DRAW, uniforms)],
        )
    }
}

/// Draw `job`, waiting for each tick, until none of `nodes` is linking, and none failed.
fn draw_until_linked(r: &mut Renderer, nodes: &[u32], job: impl Fn() -> FrameJob) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        r.draw(&job());
        drain(r.gpu());
        if nodes.iter().all(|n| !r.is_linking(NodeId(*n))) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "never linked");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    for n in nodes {
        assert_eq!(r.errors.get(&NodeId(*n)), None, "a link error");
    }
}

/// The Output at node 3's latest frame at the middle of its left and right halves, RGB.
fn halves(r: &Renderer) -> [[u8; 3]; 2] {
    drain(r.gpu());
    let texture = r.texture_of(NodeId(3)).expect("drawn");
    let px = rgba_of(r.gpu(), &texture);
    let w = texture.width() as usize;
    let at = |x: usize| {
        let p = px[32 * w + x];
        [p[0], p[1], p[2]]
    };
    [at(16), at(48)]
}

/// **An input removed while its replacement links leaves the old picture.** A module sampling
/// A and B loses A: the job's uniforms name only B from that tick, while the old program,
/// sampling both, draws until the new one has landed. It draws with the uniforms its own source
/// was compiled for — A's texture looked up fresh, never B's in A's place, and `u_gain` holding
/// its value — so every frame while the link is held is the old picture, with no validation
/// error, which the harness's device would panic on. Then the replacement fails, and the old
/// picture still holds; and a module that links replaces it.
#[test]
fn an_input_removed_while_its_replacement_links_leaves_the_old_picture() {
    let mut r = Renderer::new(gpu::gpu()).expect("renderer");
    let out = NodeId(3);
    let sources = [(1, RED), (2, GREEN)];
    let (both, one) = (two_sampler_shader(true), two_sampler_shader(false));
    let old = |send| two_sampler_tick(&both, send, &["u_a", "u_b", "u_gain"], 1.0, &sources);
    r.draw(&old(true));
    draw_until_linked(&mut r, &[3], || old(false));
    r.draw(&old(false));
    let picture = [[255, 0, 0], [0, 255, 0]];
    assert_eq!(halves(&r), picture, "A on the left, B on the right");

    r.hold_link(out, true);
    let new = |shader: &Arc<Shader>, send| {
        two_sampler_tick(shader, send, &["u_b", "u_gain"], 0.5, &sources)
    };
    r.draw(&new(&one, true));
    for tick in 0..6 {
        r.draw(&new(&one, false));
        assert!(r.is_linking(out), "held");
        assert_eq!(halves(&r), picture, "tick {tick} of the held link");
    }

    let broken = Arc::new(Shader {
        body: format!("{}\nthis does not compile\n", one.body),
        ..(*one).clone()
    });
    r.draw(&new(&broken, true));
    r.hold_link(out, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while r.is_linking(out) {
        assert!(std::time::Instant::now() < deadline, "never linked");
        r.draw(&new(&broken, false));
    }
    assert!(r.errors.contains_key(&out), "the link failed");
    for tick in 0..4 {
        r.draw(&new(&broken, false));
        assert_eq!(halves(&r), picture, "tick {tick} after the failed link");
    }

    r.draw(&new(&one, true));
    draw_until_linked(&mut r, &[3], || new(&one, false));
    r.draw(&new(&one, false));
    assert_eq!(
        halves(&r),
        [[0, 128, 0]; 2],
        "the new program draws B across the frame at the new gain"
    );
}

/// **A sampler whose texture is not there reads black.** A's node stops publishing — it was
/// deleted, say — while the program sampling it still draws: its binding is the black texture
/// rather than B's, so the left half goes black, not green.
#[test]
fn a_sampler_whose_texture_is_not_there_reads_black() {
    let mut r = Renderer::new(gpu::gpu()).expect("renderer");
    let both = two_sampler_shader(true);
    let at = |send, sources: &[(u32, [u8; 4])]| {
        two_sampler_tick(&both, send, &["u_a", "u_b", "u_gain"], 1.0, sources)
    };
    let all = [(1, RED), (2, GREEN)];
    r.draw(&at(true, &all));
    draw_until_linked(&mut r, &[3], || at(false, &all));
    r.draw(&at(false, &all));
    assert_eq!(halves(&r), [[255, 0, 0], [0, 255, 0]]);
    r.draw(&at(false, &[(2, GREEN)]));
    assert_eq!(halves(&r), [[0, 0, 0], [0, 255, 0]]);
}

// ------------------------------------------------------------- tap readings under change

/// A module that adds `adds[i]` into slot `i`'s first word from pixel (0, 0) alone, so a whole
/// frame's reading is the template's word plus exactly that; with none, a module that writes
/// no tap. `tag` keeps two sources apart.
fn tap_writer(tag: &str, adds: &[u32]) -> Arc<Shader> {
    let mut writes = String::new();
    for (i, add) in adds.iter().enumerate() {
        writeln!(
            writes,
            "        atomicAdd(&tap[{}], {add}u);",
            i * TAP_WORDS
        )
        .unwrap();
    }
    let decl = if adds.is_empty() {
        String::new()
    } else {
        format!(
            "@group(0) @binding({}) var<storage, read_write> tap: array<atomic<u32>>;\n",
            wgsl::TAPS
        )
    };
    let mut shader = (*module(
        &[],
        &[],
        &format!(
            "{decl}
// {tag}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    if (all(vec2i(frag_coord.xy) == vec2i(0))) {{
{writes}    }}
    return vec4f(1.0);
}}
"
        ),
    ))
    .clone();
    shader.taps = vec![(NodeId(0), TapKind::Stats); adds.len()];
    Arc::new(shader)
}

fn words_of(taken: &[(NodeId, Vec<u32>)], node: u32) -> Option<Vec<u32>> {
    taken
        .iter()
        .find(|(id, _)| *id == NodeId(node))
        .map(|(_, w)| w.clone())
}

/// One tick of Output 1 drawing `a` and Output 2 drawing `b`, each sent where `send` says.
fn two_tapped(a: &Arc<Shader>, b: &Arc<Shader>, send: bool) -> FrameJob {
    let at = |node, shader: &Arc<Shader>| -> OutputJob {
        job(NodeId(node), (64, 64), shader, send, DRAW, Vec::new())
    };
    tick(0.0, vec![at(1, a), at(2, b)])
}

/// **A tapped Output relinking hands over nothing its old program measured.** B's source
/// changes to one with no taps, and its old program — which still writes one — draws while
/// the new one is held linking. Nothing B's old program wrote is delivered, since B's job names
/// the new layout, and A's readings, drawn just before it, are A's alone.
///
/// The GL test's other half, that binding 0 holds B's own buffer under it, is structural here:
/// the tap buffer is in the draw's own bind group (`proposals/wgpu.md`, 1.7).
#[test]
fn a_tapped_output_relinking_keeps_its_own_buffer_bound_and_delivers_nothing_old() {
    let gpu = gpu::gpu();
    let mut r = Renderer::new(gpu.clone()).expect("renderer");
    let a = tap_writer("A", &[1]);
    let (before, after) = (tap_writer("B before", &[1000]), tap_writer("B after", &[]));
    r.draw(&two_tapped(&a, &before, true));
    draw_until_linked(&mut r, &[1, 2], || two_tapped(&a, &before, false));
    let mut taken = Vec::new();
    for _ in 0..4 {
        r.draw(&two_tapped(&a, &before, false));
        drain(&gpu);
        taken.extend(r.take_taps());
    }
    assert_eq!(
        words_of(&taken, 2).map(|w| w[0]),
        Some(TAP_TEMPLATE[0] + 1000),
        "B's old program writes its tap"
    );

    r.hold_link(NodeId(2), true);
    r.draw(&two_tapped(&a, &after, true));
    let mut seen = Vec::new();
    for tick in 0..6 {
        r.draw(&two_tapped(&a, &after, false));
        assert!(r.is_linking(NodeId(2)), "held");
        drain(&gpu);
        let taken = r.take_taps();
        assert_eq!(words_of(&taken, 2), None, "tick {tick}: B's old words");
        seen.push(words_of(&taken, 1).map(|w| w[0]));
    }
    r.hold_link(NodeId(2), false);
    draw_until_linked(&mut r, &[2], || two_tapped(&a, &after, false));
    for _ in 0..3 {
        r.draw(&two_tapped(&a, &after, false));
        drain(&gpu);
        assert_eq!(words_of(&r.take_taps(), 2), None, "B writes no taps now");
    }
    assert!(
        seen.iter().all(|w| *w == Some(TAP_TEMPLATE[0] + 1)),
        "A's reading is A's alone: {seen:?}"
    );
}
