// SPDX-License-Identifier: AGPL-3.0-or-later

//! The WGSL module the renderer draws: one module per Output, and per cost probe, with
//! both entry points in it. The conventions a node's WGSL is written to are in
//! [docs/nodes.md](../../docs/nodes.md#wgsl); what the renderer needs from a module is here.
//!
//! **Every binding is computable from the [`Shader`] alone**, with no reflection, all in
//! group 0: the uniform struct at [`UNIFORMS`], the tap buffer at [`TAPS`] where the shader
//! has taps, the four shared samplers at [`SAMPLERS`] where it has any texture, and one
//! texture per `NodeTexture` uniform from [`TEXTURES`] up, in the uniforms' name order.
//! [`bindings`] lists them and [`uniform_layout`] gives the struct's byte offsets, which are
//! the WGSL uniform address space's own rules over the struct's fields in the order this
//! module writes them.
//!
//! **Texture memory keeps GL's rows.** `fs_main` builds `uv` from `@builtin(position)` with
//! no flip: wgpu's framebuffer row 0 is the first row in memory with `y = 0.5`, as GL's is,
//! so an Output's texture has its bottom row first, as GL's did, and every readback, upload
//! flip and expected pixel written against GL's rows holds (`proposals/wgpu.md`,
//! "Orientation: nothing moves in memory").

use super::{CompileContext, Graph, NodeId, Shader, UniformProvider, UniformType};
use crate::nodes::{TextureFilter, TextureWrap};
use std::fmt::Write as _;
use std::sync::Arc;

/// Every module's constants, its vertex stage, `floor_mod`, and the default color map.
pub const PRELUDE: &str = include_str!("prelude.wgsl");

/// The vertex entry point, in [`PRELUDE`].
pub const VERTEX_ENTRY: &str = "vs_main";
/// The fragment entry point, written by [`assemble`].
pub const FRAGMENT_ENTRY: &str = "fs_main";

/// The uniform struct `u`'s binding.
pub const UNIFORMS: u32 = 0;
/// The tap buffer's binding, `tap: array<atomic<u32>>`, declared only where there are taps.
pub const TAPS: u32 = 1;
/// The first of the four samplers, in [`Sampler::ALL`] order.
pub const SAMPLERS: u32 = 2;
/// The first texture's binding.
pub const TEXTURES: u32 = SAMPLERS + Sampler::ALL.len() as u32;

/// The two members every uniform struct starts with, the prelude's standard uniforms.
const STANDARD: [(&str, &str); 2] = [("u_resolution", "vec2f"), ("u_time", "f32")];

/// One of the samplers a module shares among its textures: the wrap and the filter a texture
/// output declares, which in wgpu belong to a sampler rather than to the texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampler {
    MirrorLinear,
    MirrorNearest,
    RepeatLinear,
    RepeatNearest,
}

impl Sampler {
    /// In binding order, from [`SAMPLERS`].
    pub const ALL: [Self; 4] = [
        Self::MirrorLinear,
        Self::MirrorNearest,
        Self::RepeatLinear,
        Self::RepeatNearest,
    ];

    /// The sampler a texture output with this wrap and filter is read through.
    pub fn of(wrap: TextureWrap, filter: TextureFilter) -> Self {
        match (wrap, filter) {
            (TextureWrap::Mirror, TextureFilter::Linear) => Self::MirrorLinear,
            (TextureWrap::Mirror, TextureFilter::Nearest) => Self::MirrorNearest,
            (TextureWrap::Repeat, TextureFilter::Linear) => Self::RepeatLinear,
            (TextureWrap::Repeat, TextureFilter::Nearest) => Self::RepeatNearest,
        }
    }

    /// Its name in a module.
    pub fn name(self) -> &'static str {
        match self {
            Self::MirrorLinear => "sampler_mirror_linear",
            Self::MirrorNearest => "sampler_mirror_nearest",
            Self::RepeatLinear => "sampler_repeat_linear",
            Self::RepeatNearest => "sampler_repeat_nearest",
        }
    }
}

/// What the renderer puts at one binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// The uniform struct, laid out by [`uniform_layout`].
    Uniforms,
    /// The tap buffer: `shader.taps.len() * TAP_WORDS` words, read and written.
    Taps,
    Sampler(Sampler),
    /// The texture a `NodeTexture` uniform of this name provides.
    Texture(Arc<str>),
}

/// Every binding of a shader, in group 0, in binding order.
pub fn bindings(shader: &Shader) -> Vec<(u32, Resource)> {
    let mut out = vec![(UNIFORMS, Resource::Uniforms)];
    if shader.slots() > 0 {
        out.push((TAPS, Resource::Taps));
    }
    let textures = textures(shader);
    if !textures.is_empty() {
        out.extend(
            Sampler::ALL
                .iter()
                .zip(SAMPLERS..)
                .map(|(s, b)| (b, Resource::Sampler(*s))),
        );
    }
    out.extend(
        textures
            .into_iter()
            .zip(TEXTURES..)
            .map(|(name, b)| (b, Resource::Texture(name))),
    );
    out
}

/// The names of a shader's texture uniforms, in binding order.
fn textures(shader: &Shader) -> Vec<Arc<str>> {
    shader
        .uniforms
        .iter()
        .filter(|(_, p)| matches!(p, UniformProvider::NodeTexture { .. }))
        .map(|(name, _)| name.clone())
        .collect()
}

/// One member of the uniform struct and where it sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: Arc<str>,
    pub offset: u32,
}

/// The uniform struct's members in declaration order, `u_resolution` and `u_time` first,
/// and its size in bytes by WGSL's rule — the end of the last member rounded up to the
/// largest alignment — which a buffer slice bound to it must cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UniformLayout {
    pub fields: Vec<Field>,
    pub size: u32,
}

/// The size and alignment of a uniform member, by its WGSL type.
fn size_align(ty: &str) -> (u32, u32) {
    match ty {
        "vec4f" => (16, 16),
        "vec2f" => (8, 8),
        _ => (4, 4),
    }
}

/// The struct's members as `(name, WGSL type)`, in the order the module declares them.
fn members(
    uniforms: &std::collections::BTreeMap<impl AsRef<str>, UniformProvider>,
) -> Vec<(String, &'static str)> {
    STANDARD
        .iter()
        .map(|(n, t)| ((*n).to_string(), *t))
        .chain(uniforms.iter().filter_map(|(name, p)| {
            let ty = p.ty().wgsl()?;
            Some((name.as_ref().to_string(), ty))
        }))
        .collect()
}

/// Where each member of a shader's uniform struct sits.
pub fn uniform_layout(shader: &Shader) -> UniformLayout {
    let mut offset: u32 = 0;
    let mut largest: u32 = 4;
    let mut fields = Vec::new();
    for (name, ty) in members(&shader.uniforms) {
        let (size, align) = size_align(ty);
        largest = largest.max(align);
        offset = offset.next_multiple_of(align);
        fields.push(Field {
            name: Arc::from(name),
            offset,
        });
        offset += size;
    }
    UniformLayout {
        fields,
        size: offset.next_multiple_of(largest),
    }
}

/// The module around the functions the walk emitted: the prelude, the bindings, the utils,
/// the functions and `fs_main`.
pub(super) fn assemble(ctx: &CompileContext, root: &str) -> String {
    let mut body = head(ctx);
    let measurements = if ctx.measurements.is_empty() {
        String::new()
    } else {
        format!(
            "    let cell = vec2i(frag_coord.xy);\n{}\n",
            ctx.measurements.join("\n")
        )
    };
    write!(
        body,
        "
@fragment
fn {FRAGMENT_ENTRY}(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    let screenUV = frag_coord.xy / u.u_resolution;
    let aspectRatio = u.u_resolution.x / u.u_resolution.y;
    let uv = vec2f((2.0 * screenUV.x - 1.0) * aspectRatio, 2.0 * screenUV.y - 1.0);
{measurements}    return {root};
}}
"
    )
    .unwrap();
    body
}

/// A workspace pass's module: the same head, and an `fs_main` that is one fragment per cell:
/// the measurements' square of [`super::Shader::grid`] at the corner, each measurement called
/// under its own grid's guard as it always was, and to its right a grid of thumbnails,
/// [`super::PASS_COLS`] across, each tile one port's.
///
/// A fragment right of the square finds its tile and its cell in it, and the `switch` calls that tile's function
/// once at the cell's center over the thumbnail's frame. The word it writes is the cell's
/// own, after the tap slots, so no two fragments write one word and nothing is atomic but
/// the declaration: a number's `f32` bits, or a color clamped and packed four bytes to a
/// word. A tile is one port throughout, so the `switch` is the same case across a
/// workgroup but at a tile's edge. The picture is thrown away.
pub(super) fn assemble_pass(ctx: &CompileContext) -> String {
    use super::{PASS_COLS, PortType, TAP_WORDS, THUMB_CELLS, THUMB_H, THUMB_W};
    let mut body = head(ctx);
    let cols = ctx.thumbs.len().clamp(1, PASS_COLS);
    let base = ctx.taps.len() * TAP_WORDS;
    let grid = ctx.grid;
    let measurements = ctx.measurements.join("\n");
    let mut cases = String::new();
    for (i, (_, ty, func)) in ctx.thumbs.iter().enumerate() {
        let word = if *ty == PortType::VaryingNumber {
            format!("bitcast<u32>({func}(tp))")
        } else {
            format!("pack4x8unorm(clamp({func}(tp), vec4f(0.0), vec4f(1.0)))")
        };
        writeln!(
            cases,
            "        case {i}: {{ atomicStore(&tap[at], {word}); }}"
        )
        .unwrap();
    }
    let tiles = if ctx.thumbs.is_empty() {
        String::new()
    } else {
        format!(
            "    let tiled = cell - vec2i({grid}, 0);
    if (tiled.x < 0) {{
        return vec4f(0.0);
    }}
    let tile = tiled / vec2i({THUMB_W}, {THUMB_H});
    let local = tiled - tile * vec2i({THUMB_W}, {THUMB_H});
    let index = tile.y * {cols} + tile.x;
    let tp = vec2f(
        ((f32(local.x) + 0.5) / f32({THUMB_W}) * 2.0 - 1.0) * (f32({THUMB_W}) / f32({THUMB_H})),
        (f32(local.y) + 0.5) / f32({THUMB_H}) * 2.0 - 1.0,
    );
    let at = {base}u + u32(index) * {THUMB_CELLS}u + u32(local.y * {THUMB_W} + local.x);
    switch index {{
{cases}        default: {{}}
    }}
"
        )
    };
    write!(
        body,
        "
@fragment
fn {FRAGMENT_ENTRY}(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {{
    let cell = vec2i(frag_coord.xy);
{measurements}
{tiles}    return vec4f(0.0);
}}
"
    )
    .unwrap();
    body
}

/// Everything a module holds before its `fs_main`: the prelude, the uniform struct, the
/// bindings, the utils and the functions.
fn head(ctx: &CompileContext) -> String {
    let mut body = String::from(PRELUDE);

    body.push_str("\nstruct Uniforms {\n");
    for (name, ty) in members(&ctx.uniforms) {
        writeln!(body, "    {name}: {ty},").unwrap();
    }
    writeln!(
        body,
        "}}\n@group(0) @binding({UNIFORMS}) var<uniform> u: Uniforms;"
    )
    .unwrap();
    if !ctx.taps.is_empty() || !ctx.thumbs.is_empty() {
        writeln!(
            body,
            "@group(0) @binding({TAPS}) var<storage, read_write> tap: array<atomic<u32>>;"
        )
        .unwrap();
    }
    let textures: Vec<&String> = ctx
        .uniforms
        .iter()
        .filter(|(_, p)| p.ty() == UniformType::Sampler2D)
        .map(|(name, _)| name)
        .collect();
    if !textures.is_empty() {
        for (sampler, binding) in Sampler::ALL.iter().zip(SAMPLERS..) {
            writeln!(
                body,
                "@group(0) @binding({binding}) var {}: sampler;",
                sampler.name()
            )
            .unwrap();
        }
    }
    for (name, binding) in textures.into_iter().zip(TEXTURES..) {
        writeln!(
            body,
            "@group(0) @binding({binding}) var {name}: texture_2d<f32>;"
        )
        .unwrap();
    }

    if !ctx.utils.is_empty() {
        write!(body, "\n{}\n", ctx.utils.join("\n\n")).unwrap();
    }
    write!(body, "\n{}\n", ctx.functions.join("\n\n")).unwrap();
    body
}

/// The module for one Output node, or `None` if its input is unconnected.
///
/// An Output with nothing plugged in has no shader and renders black; that is not an error,
/// which is why `None` says only that and nothing else. Anything that *is* wrong comes back
/// in `Shader::diagnostics` alongside a shader that still renders.
pub fn build(graph: &Graph, output: NodeId) -> Option<Shader> {
    super::build_with(graph, output, false)
}

/// The same module with every node function counting its evaluations, and with no
/// measurement in it.
///
/// **Measured, not declared.** How many times a node runs per pixel depends on loops whose
/// trip counts are options and controls — a blur's size, a bloom's rings, a phyllotaxis's
/// seed count — so no table of taps per node kind could stay right. The probe is the real
/// program plus one `atomicAdd` at the top of each function, drawn at a few pixels by the
/// renderer and read back the way a tap is. See
/// [docs/rendering.md](../../docs/rendering.md#the-cost-probe).
pub fn build_probe(graph: &Graph, output: NodeId) -> Option<Shader> {
    super::build_with(graph, output, true)
}

/// One workspace's **pass**: modules of its own, drawn after every Output, that run each of
/// `measured`'s measurements — a tap's grid over its input, a sample's one point — and, where
/// `thumbs`, evaluate every varying output of every node on the workspace, connected or not,
/// on a [`super::THUMB_W`]x[`super::THUMB_H`] grid over the frame for the thumbnail beside
/// the port.
///
/// **Context-free.** A measurement runs over its node's input on the unit square and a
/// thumbnail is its port's function over the frame's own coordinates, whoever reads either
/// at whatever coordinates: what the node makes, not what a Zoom downstream asks of it.
///
/// **In batches, where it must.** One module, unless that would bind more than
/// [`super::PASS_TEXTURES`] textures: then as many as keep each under it, the measurements
/// and the ports in their order, each in exactly one. Empty with nothing to measure and no
/// thumbnail. See [docs/rendering.md](../../docs/rendering.md#the-workspace-pass).
pub fn build_pass(
    graph: &Graph,
    workspace: crate::graph::WorkspaceId,
    measured: &[NodeId],
    thumbs: bool,
) -> Vec<Shader> {
    super::build_pass(graph, workspace, measured, thumbs)
}

/// One node's measurement alone, as a pass would run it: what the draw rule reads to know
/// which frames and which readings a measurement samples. `None` for a node that measures
/// nothing.
pub fn build_measure(graph: &Graph, id: NodeId) -> Option<Shader> {
    super::build_measure(graph, id)
}
