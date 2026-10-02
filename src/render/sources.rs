// SPDX-License-Identifier: AGPL-3.0-or-later

//! CPU nodes' frames on the GPU: a texture per published port, uploaded when a new `Arc`
//! arrives and dropped when its node has gone — the draw's **uploads** phase
//! (`proposals/wgpu.md`, 1.10).
//!
//! **Bytes go up as they lie, through `queue.write_texture`**, which copies them once into
//! wgpu's staging and queues the copy ahead of the next submission, so an upload is queued
//! like a draw. A padded row is `bytes_per_row` equal to the source's own
//! stride, never a repack; `write_texture` has no 256-byte row rule. BGR is a `Bgra8Unorm`
//! texture, which samples as RGBA.
//!
//! **An `x` layout and every YUV layout go through one conversion pass** ([`CONVERT`]),
//! recorded into the prelude's [`Recording`]: the planes are uploaded into textures of their
//! own — a byte, a pair or a quad a texel — and one fullscreen pass writes RGB with alpha one
//! into the source's `Rgba8Unorm` texture. wgpu has no `TEXTURE_SWIZZLE_A`, so a padding byte
//! is dropped by that pass rather than swizzled; it costs one pass per new frame, for `x`
//! layouts only. What a node samples is an RGBA texture like any other either way.
//!
//! **Nothing changes until a frame is known to fit.** Every plane's length and stride is
//! checked before a texture is made or written, so a frame that does not fit leaves the
//! texture holding the last one that did.
//!
//! **A new or resized texture is made and written in the same tick**, before the submission
//! that samples it, so no frame shows it empty — *zero flash*. A texture is replaced rather
//! than written where its size or format changes, or where it is an import's memory, so bytes
//! never go into a DMA-BUF. A `Published` naming the old one keeps it alive with the frame it
//! held; one naming a texture written in place sees the frame before or the one after, whole,
//! since on one queue the viewer's blit and the write are ordered by submission.
//!
//! **An `Arc` already uploaded is skipped, by pointer**, and remembered by a `Weak`, so the
//! buffer behind it goes back to its source as soon as the copy is queued while the
//! allocation the `Weak` keeps stops another frame arriving at the same address.
//!
//! **A DMA-BUF goes to [`Imports`]** (lane 5f's), which says whether it took it. A frame
//! whose texture is its import is held for as long as it is, and let go through
//! [`Imports::let_go`] when it is replaced or its node leaves; the prelude is then submitted
//! even with nothing else in it, so the frame is stamped with a serial and goes back. An
//! import that fails raises the frame's `refused` flag, so its source falls back to bytes, and
//! leaves the texture as it was.
//!
//! **An `IOSurface` goes to [`Imports`] too**, a texture per plane: BGRA is the source's
//! texture where it lies, as a DMA-BUF with an alpha byte is, and NV12 or BGR with a padding
//! byte is drawn by the conversion pass from the imported planes. **A surface its producer
//! draws into again** — a Syphon server's, [`crate::nodes::Redrawn`] — is always drawn by the
//! pass, BGRA included, into the source's own texture, which is the copy that keeps a frame
//! from being read half drawn; the pass reads it bottom row first where it is laid out so. It is held and let go as a
//! DMA-BUF is. One that does not import goes up as its own bytes, which on a Mac are the same
//! memory mapped, so there is nothing to fall back to and no flag to raise.
//!
//! What the renderer calls, and when, is fixed in `mod.rs`: [`Sources::sync`] in the prelude
//! after the `UploadsFrom` mark, [`Sources::views`] for every draw's texture bindings,
//! [`Sources::submitted`] with the prelude's submission, [`Sources::publish`] into a
//! `Published`.

use super::Published;
use super::Texture;
use super::dmabuf::{Imports, alpha_is_padding};
use super::gpu::{Gpu, Ticket};
use super::queue::Recording;
use super::shared::{self, Shared};
use crate::compile::wgsl::Sampler;
use crate::graph::{NodeId, PortRef};
use crate::nodes::{
    Frame, IoSurface, Layout, Mapped, Pixels, TextureFilter, TextureWrap, Yuv, YuvMatrix,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

/// One frame a CPU node published, and how the texture holding it is sampled.
///
/// The sampling travels with the frame because `render/` may not read the registry: the app
/// copies `OutputDef::wrap` and `OutputDef::filter` off the node that published the frame,
/// and this is the whole of what the renderer knows about either.
pub struct SourceJob {
    pub port: PortRef,
    pub frame: Arc<Frame>,
    pub wrap: TextureWrap,
    pub filter: TextureFilter,
}

impl SourceJob {
    /// A frame sampled by the rule: mirrored outside its bounds and linearly filtered.
    pub fn new(port: PortRef, frame: Arc<Frame>) -> Self {
        Self {
            port,
            frame,
            wrap: TextureWrap::Mirror,
            filter: TextureFilter::Linear,
        }
    }
}

/// The renderer's own stage for the conversion pass: planes in, RGBA out, one output pixel per
/// fragment, read by `@builtin(position)` so row 0 of every plane lands in row 0 of the target
/// (`proposals/wgpu.md`, 1.15).
///
/// The layouts are numbered as [`layout_code`] numbers them: three planes, a luma plane and an
/// interleaved chroma plane, the two packed 4:2:2 orders whose texel is a pair of pixels, an
/// RGB quad whose fourth byte is padding, and an RGB quad whose fourth byte is alpha — a
/// surface copied as it lies, alpha and all. `flip` reads the planes bottom row first, for a
/// surface laid out that way, so the target is always top row first. Luma and packed texels are read by
/// `textureLoad`; chroma is sampled linearly between its texels, which is the upsampling, with
/// an explicit level as every stage the renderer writes.
pub const CONVERT: &str = "
@group(0) @binding(0) var plane0: texture_2d<f32>;
@group(0) @binding(1) var plane1: texture_2d<f32>;
@group(0) @binding(2) var plane2: texture_2d<f32>;
@group(0) @binding(3) var chroma: sampler;
struct Convert {
    matrix: mat3x3f,
    offset: vec3f,
    arrangement: u32,
    subsample: vec2f,
    flip: u32,
}
@group(0) @binding(4) var<uniform> c: Convert;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let rows = f32(textureDimensions(plane0).y);
    let here = select(frag_coord.xy, vec2f(frag_coord.x, rows - frag_coord.y), c.flip == 1u);
    let p = vec2i(here);
    if c.arrangement == 4u {
        return vec4f(textureLoad(plane0, p, 0).rgb, 1.0);
    }
    if c.arrangement == 5u {
        return textureLoad(plane0, p, 0);
    }
    var yuv: vec3f;
    if c.arrangement >= 2u {
        let pair = textureLoad(plane0, vec2i(p.x / 2, p.y), 0);
        let odd = (p.x & 1) == 1;
        if c.arrangement == 2u {
            yuv = vec3f(select(pair.r, pair.b, odd), pair.g, pair.a);
        } else {
            yuv = vec3f(select(pair.g, pair.a, odd), pair.r, pair.b);
        }
    } else {
        let at = here * c.subsample / vec2f(textureDimensions(plane1));
        let y = textureLoad(plane0, p, 0).r;
        let first = textureSampleLevel(plane1, chroma, at, 0.0);
        var uv = first.rg;
        if c.arrangement == 0u {
            uv = vec2f(first.r, textureSampleLevel(plane2, chroma, at, 0.0).r);
        }
        yuv = vec3f(y, uv);
    }
    return vec4f(clamp(c.matrix * (yuv - c.offset), vec3f(0.0), vec3f(1.0)), 1.0);
}
";

/// The size of [`CONVERT`]'s uniform struct: a `mat3x3f` of three padded columns, a `vec3f`
/// and a `u32` in its last four bytes, a `vec2f`, a `u32`, rounded up to the struct's
/// alignment.
const CONVERT_BLOCK: u64 = 80;

/// What a source's texture is made with: sampled, written by `write_texture` or the conversion
/// pass, and read back by a test.
const SOURCE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::COPY_DST)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT);

/// Every CPU node's texture, by the port that published it.
pub struct Sources {
    by_port: HashMap<PortRef, Source>,
    /// Frames imported as DMA-BUFs, and those on their way back to their producers.
    imports: Imports,
    /// Something was let go since the last submission, so the next prelude must be submitted
    /// to stamp it.
    letting_go: bool,
    convert: Convert,
}

/// One port's frame, resident on the GPU.
struct Source {
    /// `None` until a frame has gone up or been imported.
    target: Option<Target>,
    /// What is in the texture, so the next frame's job can be compared by pointer.
    uploaded: Weak<Frame>,
    /// The frame whose DMA-BUF this source samples — the texture's storage, or plane 0's where
    /// [`Source::planar_import`] says so — held for as long as it is.
    imported: Option<Arc<Frame>>,
    /// The imports are the planes and the conversion pass draws the texture, which is the
    /// source's own: a fourcc whose fourth byte is padding, so its alpha is written one, or an
    /// `IOSurface`'s NV12 or padded BGR.
    planar_import: bool,
    /// An `IOSurface` of this source's has been refused and its bytes uploaded, which is said
    /// once.
    surface_refused: bool,
    /// Where an `x` or YUV frame's planes go before the pass draws them into the texture.
    planes: [Option<Plane>; 3],
    /// The conversion pass's uniforms, made on the first frame that needs them.
    block: Option<wgpu::Buffer>,
    /// How a viewer samples it: what the node's texture output declared.
    wrap: TextureWrap,
    filter: TextureFilter,
}

/// A texture and the view it is sampled and drawn through.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One plane's texture, kept from frame to frame and remade only when its shape changes.
struct Plane {
    target: Target,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

/// The conversion pass: its pipeline, its group's layout and the sampler chroma is read
/// through. Made with the renderer.
struct Convert {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    chroma: wgpu::Sampler,
}

/// What one frame's bytes are, checked against its size before anything is written.
enum Parts<'a> {
    /// Straight into the source's texture, in this format, rows `stride` bytes apart.
    Direct {
        format: wgpu::TextureFormat,
        bytes: &'a [u8],
        stride: u32,
    },
    /// Into plane textures, each `(bytes, stride, width, height, format)`, then through the
    /// conversion pass.
    Converted {
        m: &'a Mapped,
        planes: Vec<(&'a [u8], u32, u32, u32, wgpu::TextureFormat)>,
    },
}

impl Sources {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            by_port: HashMap::new(),
            imports: Imports::default(),
            letting_go: false,
            convert: Convert::new(gpu.device()),
        }
    }

    /// Upload what CPU nodes published, recording any conversion pass into `recording`, and
    /// drop the textures of nodes that are gone. `completed` is the newest finished serial,
    /// which says which let-go DMA-BUFs may go back.
    pub fn sync(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        jobs: &[SourceJob],
        completed: u64,
    ) {
        self.imports.sweep(completed);

        let live: HashSet<PortRef> = jobs.iter().map(|j| j.port).collect();
        let (imports, letting_go) = (&mut self.imports, &mut self.letting_go);
        self.by_port.retain(|port, source| {
            if live.contains(port) {
                return true;
            }
            if let Some(frame) = source.imported.take() {
                imports.let_go(frame);
                *letting_go = true;
            }
            false
        });

        for job in jobs {
            let source = self.by_port.entry(job.port).or_insert_with(|| Source {
                target: None,
                uploaded: Weak::new(),
                imported: None,
                planar_import: false,
                surface_refused: false,
                planes: [None, None, None],
                block: None,
                wrap: job.wrap,
                filter: job.filter,
            });
            // Asked of the viewer's sampler, never of the texture: nothing is reallocated.
            (source.wrap, source.filter) = (job.wrap, job.filter);
            let frame = &job.frame;
            if std::ptr::eq(source.uploaded.as_ptr(), Arc::as_ptr(frame)) {
                continue;
            }
            source.uploaded = Arc::downgrade(frame);
            let surface = match &frame.pixels {
                Pixels::IoSurface(s) => Some(
                    self.imports
                        .import_surface(gpu.device(), s, frame.width, frame.height)
                        .map(|planes| (s, planes)),
                ),
                _ => None,
            };
            let result = match (&frame.pixels, surface) {
                (_, Some(Ok((s, planes)))) => {
                    let size = (frame.width, frame.height);
                    // A surface its producer draws into again is copied, however it is
                    // laid out, so a frame is never sampled half drawn.
                    let old = if s.mapped.layout == Layout::Bgra && s.redrawn.is_none() {
                        let texture = planes.into_iter().next().expect("one BGR plane");
                        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                        source.target = Some(Target { texture, view });
                        source.planes = [None, None, None];
                        source.planar_import = false;
                        source.imported.take()
                    } else {
                        let bottom_first = s.redrawn.is_some_and(|r| r.bottom_first);
                        let block = convert_block(s.mapped.layout, s.mapped.yuv, bottom_first);
                        self.convert
                            .imported(gpu, shared, recording, source, planes, size, &block)
                    };
                    source.imported = Some(Arc::clone(frame));
                    if let Some(old) = old {
                        self.imports.let_go(old);
                        self.letting_go = true;
                    }
                    Ok(())
                }
                (Pixels::DmaBuf(buf), _) => {
                    match self
                        .imports
                        .import(gpu.device(), buf, frame.width, frame.height)
                    {
                        Ok(texture) => {
                            let old = if alpha_is_padding(buf.fourcc) {
                                // Any matrix: the pass reads the padded layout as RGB.
                                let any = Yuv {
                                    matrix: YuvMatrix::Bt709,
                                    full_range: true,
                                };
                                self.convert.imported(
                                    gpu,
                                    shared,
                                    recording,
                                    source,
                                    vec![texture],
                                    (frame.width, frame.height),
                                    &convert_block(Layout::Rgbx, any, false),
                                )
                            } else {
                                let view =
                                    texture.create_view(&wgpu::TextureViewDescriptor::default());
                                source.target = Some(Target { texture, view });
                                source.planes = [None, None, None];
                                source.planar_import = false;
                                source.imported.take()
                            };
                            source.imported = Some(Arc::clone(frame));
                            if let Some(old) = old {
                                self.imports.let_go(old);
                                self.letting_go = true;
                            }
                            Ok(())
                        }
                        Err(e) => {
                            if let Some(refused) = &buf.refused {
                                refused.store(true, Ordering::Relaxed);
                            }
                            Err(e)
                        }
                    }
                }
                (_, surface) => match parts(frame) {
                    Ok(parts) => {
                        // An IOSurface that does not import goes up as the same memory mapped,
                        // said once for the source.
                        if let Some(Err(e)) = surface
                            && !source.surface_refused
                        {
                            log::warn!(
                                "{}.{}: {e}; uploading its bytes",
                                job.port.node,
                                job.port.key
                            );
                            source.surface_refused = true;
                        }
                        if let Some(old) = self.convert.upload(
                            gpu,
                            shared,
                            recording,
                            source,
                            (frame.width, frame.height),
                            &parts,
                        ) {
                            self.imports.let_go(old);
                            self.letting_go = true;
                        }
                        Ok(())
                    }
                    Err(e) => Err(e),
                },
            };
            // The texture keeps the last frame that did go up or in.
            if let Err(e) = result {
                log::error!("{}.{}: {e}", job.port.node, job.port.key);
            }
        }

        // A frame let go goes back behind a serial, so the prelude is submitted even when
        // nothing else went into it — whether the renderer let go of it or the last viewer did.
        if self.letting_go || self.imports.waiting() {
            recording.encoder();
        }
    }

    /// Every source texture's view, by port: what a draw's texture binding looks up first.
    pub fn views(&self) -> impl Iterator<Item = (PortRef, wgpu::TextureView)> + '_ {
        self.by_port
            .iter()
            .filter_map(|(port, s)| Some((*port, s.target.as_ref()?.view.clone())))
    }

    /// The prelude, holding whatever [`Sources::sync`] recorded, went in `ticket`'s
    /// submission.
    pub fn submitted(&mut self, ticket: &Ticket) {
        self.imports.submitted(ticket.serial);
        self.letting_go = false;
    }

    /// Put every source texture into `out`, rows top first (`flip`), with the sampler its
    /// output declared, and a claim on every imported frame one of them samples, so the frame
    /// goes back to its producer only once `out` and every clone of it has been let go.
    pub fn publish(&mut self, out: &mut Published) {
        for (port, s) in &self.by_port {
            let Some(target) = &s.target else {
                continue;
            };
            if let Some(frame) = &s.imported {
                out.imports.push(self.imports.claim(frame));
            }
            out.sources.insert(
                *port,
                super::Picture {
                    texture: Texture::new(target.texture.clone(), target.view.clone()),
                    width: target.texture.width(),
                    height: target.texture.height(),
                    // A frame is uploaded top row first where an Output fills its target
                    // bottom row first.
                    flip: true,
                    sampler: Sampler::of(s.wrap, s.filter),
                    drawn_tick: None,
                },
            );
        }
    }

    /// The texture a node last uploaded. For a test.
    pub fn texture_of(&self, node: NodeId) -> Option<wgpu::Texture> {
        self.by_port
            .iter()
            .filter(|(port, _)| port.node == node)
            .find_map(|(_, s)| Some(s.target.as_ref()?.texture.clone()))
    }

    /// Let go of every texture: the project is closing. An imported frame goes back once the
    /// next prelude, which is then submitted whatever it holds, has finished.
    pub fn forget(&mut self, completed: u64) {
        self.imports.sweep(completed);
        for (_, mut source) in self.by_port.drain() {
            if let Some(frame) = source.imported.take() {
                self.imports.let_go(frame);
                self.letting_go = true;
            }
        }
    }
}

/// A frame's bytes cut to the rows its size covers, or why they do not fit.
fn parts(frame: &Frame) -> Result<Parts<'_>, String> {
    let (width, height) = (frame.width, frame.height);
    if width == 0 || height == 0 {
        return Err(format!("a frame of {width}x{height} has no pixels"));
    }
    match &frame.pixels {
        Pixels::Bytes(bytes) => {
            if bytes.len() != (width * height * 4) as usize {
                return Err(format!(
                    "frame of {width}x{height} carries {} bytes",
                    bytes.len()
                ));
            }
            Ok(Parts::Direct {
                format: wgpu::TextureFormat::Rgba8Unorm,
                bytes,
                stride: width * 4,
            })
        }
        Pixels::Mapped(m) | Pixels::IoSurface(IoSurface { mapped: m, .. })
            if matches!(m.layout, Layout::Rgba | Layout::Bgra) =>
        {
            let bytes = m
                .plane(0, width, height)
                .ok_or("buffer shorter than its caps claim")?;
            Ok(Parts::Direct {
                format: rgb_format(m.layout),
                bytes,
                stride: whole_texels(m.strides[0], 4)?,
            })
        }
        Pixels::Mapped(m) | Pixels::IoSurface(IoSurface { mapped: m, .. }) => {
            let mut planes = Vec::with_capacity(3);
            for i in 0..m.layout.planes() {
                let bytes = m
                    .plane(i, width, height)
                    .ok_or("buffer shorter than its caps claim")?;
                let texel = m.layout.texel_bytes(i);
                let (w, h) = m.layout.plane_size(i, width, height);
                let format = match (m.layout, texel) {
                    (Layout::Rgbx | Layout::Bgrx, _) => rgb_format(m.layout),
                    (_, 1) => wgpu::TextureFormat::R8Unorm,
                    (_, 2) => wgpu::TextureFormat::Rg8Unorm,
                    _ => wgpu::TextureFormat::Rgba8Unorm,
                };
                planes.push((bytes, whole_texels(m.strides[i], texel)?, w, h, format));
            }
            Ok(Parts::Converted { m, planes })
        }
        Pixels::DmaBuf(_) => Err("a DMA-BUF is imported, not uploaded".to_owned()),
    }
}

/// An RGB layout's texture format: BGR orders sample as RGBA through `Bgra8Unorm`.
fn rgb_format(layout: Layout) -> wgpu::TextureFormat {
    match layout {
        Layout::Bgra | Layout::Bgrx => wgpu::TextureFormat::Bgra8Unorm,
        _ => wgpu::TextureFormat::Rgba8Unorm,
    }
}

/// A stride, where it is a whole number of texels: a row the GPU copies texel by texel cannot
/// start part way into one.
fn whole_texels(stride: u32, texel_bytes: u32) -> Result<u32, String> {
    if stride.is_multiple_of(texel_bytes) {
        Ok(stride)
    } else {
        Err(format!(
            "a stride of {stride} bytes is not a whole number of {texel_bytes}-byte texels"
        ))
    }
}

/// Which branch of [`CONVERT`] a layout takes.
fn layout_code(layout: Layout) -> u32 {
    match layout {
        Layout::Nv12 => 1,
        Layout::Yuy2 => 2,
        Layout::Uyvy => 3,
        Layout::Rgbx | Layout::Bgrx => 4,
        Layout::Rgba | Layout::Bgra => 5,
        _ => 0,
    }
}

/// A texture of `size` and `format` a source is sampled from and written into.
fn texture(
    device: &wgpu::Device,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    label: &str,
) -> Target {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Target { texture, view }
}

/// Queue `bytes`, rows `stride` apart, into the whole of `texture`.
fn write(gpu: &Gpu, texture: &wgpu::Texture, bytes: &[u8], stride: u32) {
    gpu.queue().write_texture(
        texture.as_image_copy(),
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: None,
        },
        texture.size(),
    );
}

impl Source {
    /// The source's own texture of `size` and `format`, made where it has none of that shape
    /// or where its storage is an import's. Returns the import that storage was, now let go
    /// of by the texture.
    fn own_target(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Option<Arc<Frame>> {
        let fits = (self.imported.is_none() || self.planar_import)
            && self.target.as_ref().is_some_and(|t| {
                (t.texture.width(), t.texture.height()) == size && t.texture.format() == format
            });
        if !fits {
            self.target = Some(texture(device, size, format, SOURCE_USAGE, "source"));
        }
        self.planar_import = false;
        self.imported.take()
    }

    /// Plane `i`'s texture, made or remade where its shape is not `(width, height, format)`.
    fn plane(
        &mut self,
        device: &wgpu::Device,
        i: usize,
        (width, height): (u32, u32),
        format: wgpu::TextureFormat,
    ) -> &Plane {
        let fits = self.planes[i]
            .as_ref()
            .is_some_and(|p| (p.width, p.height, p.format) == (width, height, format));
        if !fits {
            self.planes[i] = Some(Plane {
                target: texture(
                    device,
                    (width, height),
                    format,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    "plane",
                ),
                width,
                height,
                format,
            });
        }
        self.planes[i].as_ref().expect("made above")
    }
}

impl Convert {
    fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("convert"),
            source: wgpu::ShaderSource::Wgsl(CONVERT.into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("convert"),
            entries: &[
                plane(0),
                plane(1),
                plane(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(CONVERT_BLOCK),
                    },
                    count: None,
                },
            ],
        });
        let pipeline = shared::fullscreen(
            device,
            &module,
            &layout,
            wgpu::TextureFormat::Rgba8Unorm,
            "convert",
        );
        // Clamped at the edges and linear between texels, as the GL planes were
        // parametrized.
        let chroma = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("chroma"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            chroma,
        }
    }

    /// Put a frame whose `parts` fit into `source`'s texture: written in place where it has
    /// one of the frame's shape, into a new one where it does not. Returns the import the
    /// texture's storage was, which the caller lets go.
    fn upload(
        &self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        source: &mut Source,
        size: (u32, u32),
        parts: &Parts<'_>,
    ) -> Option<Arc<Frame>> {
        let device = gpu.device();
        // The planes are an import's memory, which bytes never go into.
        if source.planar_import {
            source.planes = [None, None, None];
        }
        match parts {
            Parts::Direct {
                format,
                bytes,
                stride,
            } => {
                let old = source.own_target(device, size, *format);
                source.planes = [None, None, None];
                let target = source.target.as_ref().expect("made above");
                write(gpu, &target.texture, bytes, *stride);
                old
            }
            Parts::Converted { m, planes } => {
                for (i, (bytes, stride, w, h, format)) in planes.iter().enumerate() {
                    let plane = source.plane(device, i, (*w, *h), *format);
                    write(gpu, &plane.target.texture, bytes, *stride);
                }
                // A camera that renegotiates from three planes to one leaves nothing behind.
                for plane in source.planes.iter_mut().skip(planes.len()) {
                    *plane = None;
                }
                let old = source.own_target(device, size, wgpu::TextureFormat::Rgba8Unorm);
                self.draw(
                    gpu,
                    shared,
                    recording,
                    source,
                    &convert_block(m.layout, m.yuv, false),
                );
                old
            }
        }
    }

    /// Put a frame imported as `imports`, one texture per plane, into `source`'s own texture
    /// through the pass with `block`: a padded fourcc's one plane, whose alpha the pass writes
    /// one, or an `IOSurface`'s NV12 or padded BGR. The imports are the planes, kept for as
    /// long as the frame is. Returns the import the source held before, which the caller lets
    /// go.
    #[allow(clippy::too_many_arguments)]
    fn imported(
        &self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        source: &mut Source,
        imports: Vec<wgpu::Texture>,
        size: (u32, u32),
        block: &[u8; CONVERT_BLOCK as usize],
    ) -> Option<Arc<Frame>> {
        let old = source.own_target(gpu.device(), size, wgpu::TextureFormat::Rgba8Unorm);
        let mut planes = [None, None, None];
        for (plane, import) in planes.iter_mut().zip(imports) {
            let view = import.create_view(&wgpu::TextureViewDescriptor::default());
            *plane = Some(Plane {
                width: import.width(),
                height: import.height(),
                format: import.format(),
                target: Target {
                    texture: import,
                    view,
                },
            });
        }
        source.planes = planes;
        source.planar_import = true;
        self.draw(gpu, shared, recording, source, block);
        old
    }

    /// Record the pass drawing `source`'s planes into its texture, with `block` for its
    /// uniforms.
    fn draw(
        &self,
        gpu: &Gpu,
        shared: &Shared,
        recording: &mut Recording,
        source: &mut Source,
        block: &[u8; CONVERT_BLOCK as usize],
    ) {
        let device = gpu.device();
        let buffer = source.block.get_or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("convert"),
                size: CONVERT_BLOCK,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        gpu.queue().write_buffer(buffer, 0, block);
        let buffer = buffer.clone();
        let view = |i: usize| {
            source.planes[i]
                .as_ref()
                .map_or(shared.black(), |p| &p.target.view)
        };
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("convert"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view(0)),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view(1)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view(2)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.chroma),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let target = source.target.as_ref().expect("made above");
        // Every fragment is written, so what the target held is never read.
        let mut pass = shared::begin(
            recording.encoder(),
            &target.view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            "convert",
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// [`CONVERT`]'s uniform struct for a frame of `layout` and `yuv`: `rgb = matrix * (yuv -
/// offset)`, the matrix's columns each padded to sixteen bytes, the planes read bottom row
/// first where `bottom_first` says so.
fn convert_block(layout: Layout, yuv: Yuv, bottom_first: bool) -> [u8; CONVERT_BLOCK as usize] {
    let (matrix, offset) = coefficients(yuv);
    let subsample = match layout {
        Layout::I420 | Layout::Nv12 => [0.5f32, 0.5],
        Layout::Y42b => [0.5, 1.0],
        _ => [1.0, 1.0],
    };
    let mut block = [0u8; CONVERT_BLOCK as usize];
    let mut put = |at: usize, v: f32| block[at..at + 4].copy_from_slice(&v.to_le_bytes());
    for column in 0..3 {
        for row in 0..3 {
            put(column * 16 + row * 4, matrix[column * 3 + row]);
        }
    }
    for (i, v) in offset.iter().enumerate() {
        put(48 + i * 4, *v);
    }
    put(64, subsample[0]);
    put(68, subsample[1]);
    block[60..64].copy_from_slice(&layout_code(layout).to_le_bytes());
    block[72..76].copy_from_slice(&u32::from(bottom_first).to_le_bytes());
    block
}

/// `rgb = matrix * (yuv - offset)`, the matrix column-major, with the studio range's scale
/// folded into it.
pub fn coefficients(yuv: Yuv) -> ([f32; 9], [f32; 3]) {
    let (kr, kb) = match yuv.matrix {
        YuvMatrix::Bt601 => (0.299, 0.114),
        YuvMatrix::Bt709 => (0.2126, 0.0722),
        YuvMatrix::Bt2020 => (0.2627, 0.0593),
    };
    let kg = 1.0 - kr - kb;
    let (sy, sc, black) = if yuv.full_range {
        (1.0, 1.0, 0.0)
    } else {
        (255.0 / 219.0, 255.0 / 224.0, 16.0 / 255.0)
    };
    let rv = sc * 2.0 * (1.0 - kr);
    let bu = sc * 2.0 * (1.0 - kb);
    let gu = -sc * 2.0 * kb * (1.0 - kb) / kg;
    let gv = -sc * 2.0 * kr * (1.0 - kr) / kg;
    (
        [sy, sy, sy, 0.0, gu, bu, rv, gv, 0.0],
        [black, 128.0 / 255.0, 128.0 / 255.0],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &[f32; 9], o: &[f32; 3], yuv: [f32; 3]) -> [f32; 3] {
        let v = [yuv[0] - o[0], yuv[1] - o[1], yuv[2] - o[2]];
        std::array::from_fn(|r| m[r] * v[0] + m[3 + r] * v[1] + m[6 + r] * v[2])
    }

    /// Studio black and white are black and white, and grey has no color, in every matrix.
    #[test]
    fn studio_range_spans_black_to_white() {
        for matrix in [YuvMatrix::Bt601, YuvMatrix::Bt709, YuvMatrix::Bt2020] {
            let (m, o) = coefficients(Yuv {
                matrix,
                full_range: false,
            });
            let black = apply(&m, &o, [16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0]);
            let white = apply(&m, &o, [235.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0]);
            for c in 0..3 {
                assert!(black[c].abs() < 0.01, "{matrix:?} black: {black:?}");
                assert!((white[c] - 1.0).abs() < 1e-5, "{matrix:?} white: {white:?}");
            }
        }
    }

    /// BT.601's red: full-range Y 76, U 85, V 255 is (254, 0, 0) to within a step.
    #[test]
    fn full_range_red_is_red() {
        let (m, o) = coefficients(Yuv {
            matrix: YuvMatrix::Bt601,
            full_range: true,
        });
        let rgb = apply(&m, &o, [76.0 / 255.0, 85.0 / 255.0, 255.0 / 255.0]);
        assert!((rgb[0] * 255.0 - 254.0).abs() < 2.0, "{rgb:?}");
        assert!((rgb[1] * 255.0).abs() < 2.0, "{rgb:?}");
        assert!((rgb[2] * 255.0).abs() < 2.0, "{rgb:?}");
    }
}
