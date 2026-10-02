// SPDX-License-Identifier: AGPL-3.0-or-later

//! What puts a published picture on a screen: the blit, for an egui_wgpu paint callback and
//! for a picture window's surface.
//!
//! The synth draws; every window is a **viewer**, blitting a picture the synth has already
//! finished drawing. On one device there is nothing to fence: a `Published` holds views, a view
//! keeps its texture alive, and the queue orders the viewer's reads before the synth's next
//! draw into that slot, because the synth draws into a slot only once no `Published` names it.
//! **A viewer holds its `Published` until after the `queue.submit` that carries its reads**:
//! egui_wgpu's submit at the end of the editor's frame, or the submit before a picture window's
//! `present`. See `proposals/wgpu.md`, 1.5.
//!
//! **One pipeline per target format** ([`Viewer::new`]), blending as egui_wgpu's own does —
//! premultiplied — so a picture lands on a node body as egui's own images do. The editor's
//! viewer is made for egui_wgpu's `target_format`, a picture
//! window's for its surface's.
//!
//! **A blit fits the whole rect it was given, never the part a window clips.** The rect is
//! placed in the vertex stage against the whole target, and the pass's viewport is the whole
//! target, so a rect hanging past the window — or past `max_texture_dimension_2d` at a deep
//! zoom, which `set_viewport` would refuse — keeps its size, and the scissor egui set from the
//! clip rect is what cuts it off. egui_wgpu's own viewport, `viewport_in_pixels`, clamps to the
//! window; [`ViewerCallback`]'s `paint` replaces it.
//!
//! **Orientation.** A texture keeps GL's rows: an Output's frame and the mix have their bottom
//! row first, an upload its top row first ([`Picture::flip`]). NDC is y-up under both
//! backends, so the texture coordinate the vertex stage hands on is GL's. What differs is the
//! window: a target's row 0 is its top, so a [`Viewport`] is measured from the top and the
//! corner rounding measures its bottom band from the rect's bottom edge, `top + height`.
//!
//! **The editor paints through callbacks built here**: a node's picture and the mix are
//! [`Viewer::node_callback`] and [`Viewer::mixer_callback`], each an `egui::PaintCallback`
//! wrapping a [`ViewerCallback`] through `egui_wgpu::Callback::new_paint_callback`. Its
//! `CallbackTrait::prepare` makes the blit's bind group, and its `paint` draws it inside egui's
//! own render pass. [`Viewer::open_frame`] and [`Viewer::close_frame`] owe the device nothing
//! and are empty.

use super::gpu::Gpu;
use super::{Picture, Published};
use crate::compile::wgsl::Sampler;
use crate::graph::NodeId;
use eframe::egui;
use std::sync::{Arc, Mutex, PoisonError};

/// How a picture is fitted to a rect it does not share a shape with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The whole picture, with bars where the shapes differ.
    Letterbox,
    /// The whole rect, with the picture cropped where the shapes differ.
    Cover,
}

/// The blit. The rect is placed against the whole target in the vertex stage, and the picture
/// scaled about the rect's center by `scale`: below one letterboxes, above one covers.
///
/// `flip` is 1 for a texture whose first row is the **top** of the picture — everything a CPU
/// node uploads — and 0 for an Output's frame or the mix, whose first row is the bottom.
///
/// `radius` rounds the rect's two bottom corners. `p` is measured from the rect's bottom-left
/// corner, y up, so the band below `radius` is the bottom one although the target's row 0 is
/// its top. The clamp names the arc's center — the left corner's below `radius`, the right
/// corner's above `width - radius`, and between them `p.x` itself, whose distance is
/// `radius - p.y` and never discards. A radius of zero rounds nothing.
pub const BLIT: &str = "
struct Blit {
    rect: vec4f,
    size: vec2f,
    scale: vec2f,
    flip: f32,
    radius: f32,
    pad: vec2f,
}
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
@group(0) @binding(2) var<uniform> blit: Blit;

struct Varying {
    @builtin(position) position: vec4f,
    @location(0) uv: vec2f,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> Varying {
    var corners = array<vec2f, 6>(
        vec2f(0.0, 0.0), vec2f(1.0, 0.0), vec2f(0.0, 1.0),
        vec2f(0.0, 1.0), vec2f(1.0, 0.0), vec2f(1.0, 1.0),
    );
    let c = corners[i];
    let center = blit.rect.xy + 0.5 * blit.rect.zw;
    let half = 0.5 * blit.rect.zw * blit.scale;
    let px = vec2f(center.x + (c.x * 2.0 - 1.0) * half.x, center.y - (c.y * 2.0 - 1.0) * half.y);
    var out: Varying;
    out.position = vec4f(px.x / blit.size.x * 2.0 - 1.0, 1.0 - px.y / blit.size.y * 2.0, 0.0, 1.0);
    out.uv = vec2f(c.x, mix(c.y, 1.0 - c.y, blit.flip));
    return out;
}

@fragment
fn fs_main(v: Varying) -> @location(0) vec4f {
    let r = blit.radius;
    let p = vec2f(v.position.x - blit.rect.x, blit.rect.y + blit.rect.w - v.position.y);
    if (p.y < r && distance(p, vec2f(clamp(p.x, r, blit.rect.z - r), r)) > r) {
        discard;
    }
    return textureSampleLevel(picture, picture_sampler, v.uv, 0.0);
}
";

/// The size of `Blit` in [`BLIT`], in bytes.
const BLOCK: u64 = 48;

/// A rect in a target's pixels, row 0 at the top. It may reach past the target on any side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Viewport {
    /// The whole of a target this size.
    pub fn whole(size: (u32, u32)) -> Self {
        Self {
            left: 0,
            top: 0,
            width: size.0 as i32,
            height: size.1 as i32,
        }
    }
}

/// A rect in points as whole pixels, each edge rounded, with nothing clamped to the window.
///
/// Not `PaintCallbackInfo::viewport_in_pixels`, which clamps the rect to the window. That
/// hands the blit a rect the size of a node's visible part, and the fit is computed from
/// whatever the blit is handed, so a node at the edge of the canvas would shrink its render
/// rather than have it cut off.
pub fn viewport_of(rect: egui::Rect, pixels_per_point: f32) -> Viewport {
    let left = (pixels_per_point * rect.min.x).round() as i32;
    let top = (pixels_per_point * rect.min.y).round() as i32;
    let right = (pixels_per_point * rect.max.x).round() as i32;
    let bottom = (pixels_per_point * rect.max.y).round() as i32;
    Viewport {
        left,
        top,
        width: right - left,
        height: bottom - top,
    }
}

/// A picture's size over a rect's, about its center, for `fit`.
fn scale(picture: (u32, u32), viewport: Viewport, fit: Fit) -> [f32; 2] {
    let (vw, vh) = (viewport.width.max(1) as f32, viewport.height.max(1) as f32);
    let source = picture.0 as f32 / picture.1.max(1) as f32;
    let target = vw / vh;
    match fit {
        Fit::Letterbox if source > target => [1.0, target / source],
        Fit::Letterbox => [source / target, 1.0],
        Fit::Cover if source > target => [source / target, 1.0],
        Fit::Cover => [1.0, target / source],
    }
}

/// One blit made ready: the bind group naming the picture, its sampler and its block.
pub struct Prepared {
    group: wgpu::BindGroup,
    target: (u32, u32),
}

/// The blit pipeline for one target format, and what every blit binds.
pub struct Viewer {
    gpu: Gpu,
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// In `Sampler::ALL` order: a picture is read through the one its [`Picture::sampler`]
    /// names, so a `cellularautomata` grid is blitted nearest.
    samplers: [wgpu::Sampler; 4],
}

impl Viewer {
    /// The blit pipeline on `gpu` for targets of `format`: egui_wgpu's `target_format` for the
    /// editor, a surface's format for a picture window. Made once, synchronously.
    pub fn new(gpu: &Gpu, format: wgpu::TextureFormat) -> Result<Self, String> {
        let device = gpu.device();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(BLOCK),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(crate::compile::wgsl::VERTEX_ENTRY),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(crate::compile::wgsl::FRAGMENT_ENTRY),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(PREMULTIPLIED),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = crate::render::adapter::block_on(scope.pop()) {
            return Err(error.to_string());
        }
        Ok(Self {
            gpu: gpu.clone(),
            format,
            samplers: Sampler::ALL.map(|kind| super::shared::sampler(device, kind)),
            pipeline,
            layout,
        })
    }

    /// The editor's: the blit pipeline for the format eframe's window is painted in.
    pub fn for_eframe(cc: &eframe::CreationContext<'_>, gpu: &Gpu) -> Result<Self, String> {
        let state = cc
            .wgpu_render_state
            .as_ref()
            .ok_or("eframe paints with no wgpu device")?;
        Self::new(gpu, state.target_format)
    }

    /// The format of the targets this viewer draws into.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Make `picture` ready to blit into `viewport` of a target `target` pixels in size,
    /// fitted as `fit` says, its two bottom corners rounded to `corner` pixels. `None` for a
    /// rect with no area.
    pub fn prepare(
        &self,
        picture: &Picture,
        viewport: Viewport,
        target: (u32, u32),
        fit: Fit,
        corner: f32,
    ) -> Option<Prepared> {
        if viewport.width <= 0 || viewport.height <= 0 || target.0 == 0 || target.1 == 0 {
            return None;
        }
        let [sx, sy] = scale((picture.width, picture.height), viewport, fit);
        let values: [f32; 12] = [
            viewport.left as f32,
            viewport.top as f32,
            viewport.width as f32,
            viewport.height as f32,
            target.0 as f32,
            target.1 as f32,
            sx,
            sy,
            if picture.flip { 1.0 } else { 0.0 },
            corner.max(0.0),
            0.0,
            0.0,
        ];
        let mut bytes = [0u8; BLOCK as usize];
        for (at, v) in values.iter().enumerate() {
            bytes[at * 4..at * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let device = self.gpu.device();
        let block = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("blit"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        let sampler = Sampler::ALL
            .iter()
            .position(|s| *s == picture.sampler)
            .map_or(&self.samplers[0], |at| &self.samplers[at]);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(picture.texture.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: block.as_entire_binding(),
                },
            ],
        });
        Some(Prepared { group, target })
    }

    /// Draw a prepared blit into `pass`, whose target is the one it was prepared for. The
    /// viewport is set to the whole target, since the rect was placed against it; the scissor
    /// the pass has is left as it is.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>, prepared: &Prepared) {
        let (w, h) = prepared.target;
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &prepared.group, &[]);
        pass.draw(0..6, 0..1);
    }

    /// Blit `picture` into `viewport` of `target`, which is `size` pixels, in a pass of its
    /// own that keeps what the target holds: a picture window's surface, after its clear.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        picture: &Picture,
        viewport: Viewport,
        fit: Fit,
        corner: f32,
    ) {
        let Some(prepared) = self.prepare(picture, viewport, size, fit, corner) else {
            return;
        };
        let mut pass = super::shared::begin(encoder, target, wgpu::LoadOp::Load, "blit");
        self.paint(&mut pass, &prepared);
    }

    /// One node's picture, where there is one: `None` is an Output's own frame, `Some(key)` a
    /// CPU node's texture on that port.
    #[allow(clippy::too_many_arguments)]
    pub fn show_node(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        published: &Published,
        node: NodeId,
        port: Option<&'static str>,
        viewport: Viewport,
        fit: Fit,
        corner: f32,
    ) {
        if let Some(picture) = published.picture(node, port) {
            self.show(encoder, target, size, &picture, viewport, fit, corner);
        }
    }

    /// The mix, where there is one.
    pub fn show_mixer(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        published: &Published,
        viewport: Viewport,
        fit: Fit,
    ) {
        if let Some(picture) = &published.mixer {
            self.show(encoder, target, size, picture, viewport, fit, 0.0);
        }
    }

    /// A paint callback that blits one node's picture — `None` is an Output's own frame,
    /// `Some(key)` a CPU node's texture on that port — into `rect`, fitted to the whole of
    /// it. `corner` is the radius of its two bottom corners in points.
    #[allow(clippy::too_many_arguments)]
    pub fn node_callback(
        self: &Arc<Self>,
        published: &Arc<Published>,
        rect: egui::Rect,
        node: NodeId,
        port: Option<&'static str>,
        fit: Fit,
        corner: f32,
    ) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            ViewerCallback {
                viewer: Arc::clone(self),
                published: Arc::clone(published),
                shown: Shown::Node { node, port },
                rect,
                fit,
                corner,
                prepared: Mutex::new(None),
            },
        )
    }

    /// A paint callback that blits the mix into `rect`, fitted to the whole of it.
    pub fn mixer_callback(
        self: &Arc<Self>,
        published: &Arc<Published>,
        rect: egui::Rect,
        fit: Fit,
    ) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            rect,
            ViewerCallback {
                viewer: Arc::clone(self),
                published: Arc::clone(published),
                shown: Shown::Mix,
                rect,
                fit,
                corner: 0.0,
                prepared: Mutex::new(None),
            },
        )
    }

    /// What the editor's frame owes the device before it paints: nothing. There is no
    /// threaded-GL parking to drain on one device.
    pub fn open_frame(&self, _ctx: &egui::Context) {}

    /// What the editor's frame owes the synth after it has painted every picture: nothing
    /// but holding `published` until egui_wgpu's submit, which the caller does by keeping it
    /// until the next frame's snapshot replaces it.
    pub fn close_frame(self: &Arc<Self>, _ctx: &egui::Context, _published: &Arc<Published>) {}
}

/// egui_wgpu's own blend: premultiplied color over what is there.
const PREMULTIPLIED: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Which picture a callback blits.
#[derive(Debug, Clone, Copy)]
enum Shown {
    Node {
        node: NodeId,
        port: Option<&'static str>,
    },
    Mix,
}

/// One picture blitted inside the editor's egui pass. It holds its `Published` until
/// egui_wgpu's submit has carried its reads, since egui drops a frame's callbacks only after
/// that.
pub struct ViewerCallback {
    viewer: Arc<Viewer>,
    published: Arc<Published>,
    shown: Shown,
    rect: egui::Rect,
    fit: Fit,
    /// Points.
    corner: f32,
    /// Made by [`Self::prepare`], drawn by [`Self::paint`].
    prepared: Mutex<Option<Prepared>>,
}

impl egui_wgpu::CallbackTrait for ViewerCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        self.ready(screen.size_in_pixels, screen.pixels_per_point);
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        self.draw(&info, pass);
    }
}

impl ViewerCallback {
    /// `CallbackTrait::prepare`: the picture, the fit and the corner, made ready for a window
    /// `size_in_pixels` at `pixels_per_point` — `ScreenDescriptor`'s two fields.
    fn ready(&self, size_in_pixels: [u32; 2], pixels_per_point: f32) {
        let picture = match self.shown {
            Shown::Node { node, port } => self.published.picture(node, port),
            Shown::Mix => self.published.mixer.clone(),
        };
        let prepared = picture.and_then(|picture| {
            self.viewer.prepare(
                &picture,
                viewport_of(self.rect, pixels_per_point),
                (size_in_pixels[0], size_in_pixels[1]),
                self.fit,
                self.corner * pixels_per_point,
            )
        });
        *self.prepared.lock().unwrap_or_else(PoisonError::into_inner) = prepared;
    }

    /// `CallbackTrait::paint`: the blit, over egui_wgpu's clamped viewport, under the scissor
    /// egui_wgpu set from the clip rect.
    fn draw(&self, info: &egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'_>) {
        let prepared = self.prepared.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(prepared) = prepared.as_ref()
            && prepared.target == (info.screen_size_px[0], info.screen_size_px[1])
        {
            self.viewer.paint(pass, prepared);
        }
    }
}

/// The newest [`Published`], readable from a window that is painted outside the editor's own
/// pass: a picture window, on a thread of its own, or the Syphon publisher. The lock is held for
/// an `Arc` clone and for nothing else, and the clone is kept until the submit that carries the
/// blit.
///
/// **A watcher is told each time the synth sets it**, from the synth's own thread: the Syphon
/// publisher's, which sends itself a message and returns. A watcher must never wait.
#[derive(Default)]
pub struct Live {
    published: Mutex<Arc<Published>>,
    watchers: Mutex<Vec<Box<dyn Fn() + Send + Sync>>>,
}

impl Live {
    pub fn set(&self, published: Arc<Published>) {
        *self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = published;
        for watcher in self
            .watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            watcher();
        }
    }

    pub fn get(&self) -> Arc<Published> {
        Arc::clone(
            &self
                .published
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Call `watcher` each time a new `Published` is set, on the thread that sets it.
    pub fn watch(&self, watcher: impl Fn() + Send + Sync + 'static) {
        self.watchers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Box::new(watcher));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A callback rect that hangs off the window keeps its size and its place.
    #[test]
    fn a_callback_rect_off_the_window_is_not_clamped_to_it() {
        for at in [
            egui::pos2(100.0, 100.0),
            egui::pos2(-120.0, 100.0),
            egui::pos2(300.0, 100.0),
            egui::pos2(100.0, -60.0),
            egui::pos2(100.0, 250.0),
        ] {
            let v = viewport_of(egui::Rect::from_min_size(at, egui::vec2(240.0, 135.0)), 1.0);
            assert_eq!(
                v,
                Viewport {
                    left: at.x as i32,
                    top: at.y as i32,
                    width: 240,
                    height: 135,
                },
                "the rect at {at:?} was moved or resized"
            );
        }
        let v = viewport_of(
            egui::Rect::from_min_size(egui::pos2(10.3, 20.6), egui::vec2(100.0, 50.0)),
            2.0,
        );
        assert_eq!(
            v,
            Viewport {
                left: 21,
                top: 41,
                width: 200,
                height: 100,
            },
            "each edge is rounded in pixels"
        );
    }

    #[test]
    fn a_letterbox_shrinks_and_a_cover_grows_the_longer_side() {
        let square = Viewport::whole((64, 64));
        assert_eq!(scale((64, 32), square, Fit::Letterbox), [1.0, 0.5]);
        assert_eq!(scale((64, 32), square, Fit::Cover), [2.0, 1.0]);
        assert_eq!(scale((32, 64), square, Fit::Letterbox), [0.5, 1.0]);
        assert_eq!(scale((32, 64), square, Fit::Cover), [1.0, 2.0]);
    }
}
