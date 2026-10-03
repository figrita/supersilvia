// SPDX-License-Identifier: AGPL-3.0-or-later

//! The CPU half of a node: per-instance state that runs once a frame from `Synth::tick`.
//!
//! A node definition is data. What a definition cannot hold is a microphone or a camera,
//! which exist per instance and have a lifetime. `CpuDef::create` builds that state when
//! the node first ticks, and it is dropped when the node leaves the graph. Everything it
//! produces goes through `TickContext`: a uniform number per output port, a frame for a
//! texture output, or a tick of a world the renderer steps. Nothing here touches the GPU; a
//! frame is bytes or a descriptor, and `render/` uploads or imports it, and a simulation is
//! passes over kernels that are WGSL in a string.

use crate::graph::{ControlValue, Graph, Node, NodeId, PortRef};
use crate::nodes::action::{Edge, Event, Gate};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// One image, rows top to bottom. What a camera or a decoder publishes.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
}

/// Where a frame's pixels are.
#[derive(Debug, Clone, PartialEq)]
pub enum Pixels {
    /// Tightly packed RGBA8 in system memory. The renderer uploads it.
    Bytes(Vec<u8>),
    /// A source's own buffer in system memory, in the layout the source produced, held
    /// rather than copied. The renderer uploads it as it lies — stride, channel order and
    /// planes — and converts on the GPU.
    Mapped(Mapped),
    /// Already in GPU memory, named by a descriptor. The renderer imports it; nothing is
    /// copied.
    DmaBuf(DmaBuf),
    /// In an `IOSurface`, which the renderer samples where it lies, or uploads as the same
    /// memory mapped where it cannot.
    IoSurface(IoSurface),
    /// In a Direct3D 12 texture on the renderer's own device, or shared from another, which the
    /// renderer samples where it lies once the producer's fence says it is written.
    D3d12(D3d12Texture),
}

/// How the bytes of a [`Mapped`] frame are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Four bytes a pixel in the order named; an `x` byte is padding and reads as opaque.
    Rgba,
    Rgbx,
    Bgra,
    Bgrx,
    /// Three planes, Y then U then V, the chroma halved both ways. YV12 is this with its
    /// chroma planes stored the other way round, which the mapping swaps back.
    I420,
    /// Three planes, the chroma halved across only.
    Y42b,
    /// Three planes at full size.
    Y444,
    /// A Y plane and one plane of interleaved U and V, halved both ways.
    Nv12,
    /// One plane, Y0 U Y1 V per pair of pixels.
    Yuy2,
    /// One plane, U Y0 V Y1 per pair of pixels.
    Uyvy,
}

impl Layout {
    /// Whether the renderer draws this into RGB rather than uploading it as it is.
    pub fn is_yuv(self) -> bool {
        !matches!(self, Self::Rgba | Self::Rgbx | Self::Bgra | Self::Bgrx)
    }

    /// How many planes a frame of this layout carries.
    pub fn planes(self) -> usize {
        match self {
            Self::I420 | Self::Y42b | Self::Y444 => 3,
            Self::Nv12 => 2,
            _ => 1,
        }
    }

    /// Plane `i`'s size in texels of the texture it is uploaded into, for a frame of
    /// `width` by `height`: a byte for a plane of one component, a pair for NV12's chroma,
    /// four for a packed 4:2:2 pair of pixels or a packed RGB pixel.
    pub fn plane_size(self, i: usize, width: u32, height: u32) -> (u32, u32) {
        let half = |n: u32| n.div_ceil(2);
        match (self, i) {
            (Self::I420 | Self::Nv12, 1 | 2) => (half(width), half(height)),
            (Self::Y42b, 1 | 2) | (Self::Yuy2 | Self::Uyvy, 0) => (half(width), height),
            _ => (width, height),
        }
    }

    /// Bytes per texel of plane `i`, which is also how its texture is formatted.
    pub fn texel_bytes(self, i: usize) -> u32 {
        match (self, i) {
            (Self::Rgba | Self::Rgbx | Self::Bgra | Self::Bgrx | Self::Yuy2 | Self::Uyvy, _) => 4,
            (Self::Nv12, 1) => 2,
            _ => 1,
        }
    }
}

/// How a YUV frame's numbers become RGB: which matrix, and whether they use the whole byte
/// or the studio range of 16 to 235.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Yuv {
    pub matrix: YuvMatrix,
    pub full_range: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl Default for Yuv {
    fn default() -> Self {
        Self {
            matrix: YuvMatrix::Bt601,
            full_range: false,
        }
    }
}

/// The planes of a held buffer. Implemented by `video/` over a mapped GStreamer frame, so
/// `render/` reads the bytes without seeing a media type.
pub trait Planes: Send + Sync {
    /// Plane `i` from its first byte to the end of what is mapped.
    fn plane(&self, i: usize) -> &[u8];
}

/// A buffer in system memory, mapped and held for as long as the frame is.
#[derive(Clone)]
pub struct Mapped {
    pub layout: Layout,
    /// Bytes from one row of each plane to the next. At least a row's width, and often
    /// more: a decoder or a device pads its rows, and the upload honors that rather than
    /// repacking.
    pub strides: [u32; 3],
    pub yuv: Yuv,
    pub data: Arc<dyn Planes>,
}

impl Mapped {
    /// Plane `i` cut to exactly the bytes its rows cover, or `None` where the buffer is
    /// shorter than its stride and size claim.
    pub fn plane(&self, i: usize, width: u32, height: u32) -> Option<&[u8]> {
        let (w, h) = self.layout.plane_size(i, width, height);
        let row = (w * self.layout.texel_bytes(i)) as usize;
        let stride = *self.strides.get(i)? as usize;
        if stride < row || h == 0 {
            return None;
        }
        let len = stride * (h as usize - 1) + row;
        self.data.plane(i).get(..len)
    }
}

impl std::fmt::Debug for Mapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mapped")
            .field("layout", &self.layout)
            .field("strides", &self.strides)
            .field("yuv", &self.yuv)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Mapped {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout
            && self.strides == other.strides
            && self.yuv == other.yuv
            && Arc::ptr_eq(&self.data, &other.data)
    }
}

/// A DMA-BUF: a file descriptor naming memory a device owns, plus what is in it.
#[derive(Clone)]
pub struct DmaBuf {
    /// Valid for as long as `keep` is alive. Not owned here.
    pub fd: i32,
    /// DRM fourcc, such as `AB24` for RGBA8.
    pub fourcc: u32,
    /// DRM format modifier: the tiling the GPU chose. Passed through to the import.
    pub modifier: u64,
    pub stride: u32,
    pub offset: u32,
    /// Whatever owns the descriptor — a decoder's buffer — held so the memory stays valid
    /// while a texture samples it. Opaque, so `render/` sees no media type.
    pub keep: Arc<dyn std::any::Any + Send + Sync>,
    /// Raised by the renderer when EGL will not import the descriptor, so the source that
    /// made it can go back to bytes. `None` for a source with nothing to fall back to.
    pub refused: Option<Arc<AtomicBool>>,
}

impl std::fmt::Debug for DmaBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmaBuf")
            .field("fd", &self.fd)
            .field("fourcc", &self.fourcc)
            .field("modifier", &format_args!("{:#x}", self.modifier))
            .field("stride", &self.stride)
            .field("offset", &self.offset)
            .finish_non_exhaustive()
    }
}

impl PartialEq for DmaBuf {
    fn eq(&self, other: &Self) -> bool {
        self.fd == other.fd
            && self.fourcc == other.fourcc
            && self.modifier == other.modifier
            && self.stride == other.stride
            && self.offset == other.offset
            && Arc::ptr_eq(&self.keep, &other.keep)
    }
}

/// A frame in an `IOSurface`, macOS's memory shared between the CPU, the GPU and the media
/// engines.
#[derive(Debug, Clone, PartialEq)]
pub struct IoSurface {
    /// The `IOSurfaceRef`, as an integer, valid for as long as `mapped.data` is alive. Not
    /// owned here.
    pub surface: usize,
    /// The same memory as bytes, for a device that will not import it; its layout, strides
    /// and color matrix are the surface's. NV12 is two planes and BGR one.
    pub mapped: Mapped,
    /// `Some` for a surface its producer draws into again while it is being read — a Syphon
    /// server's — which the renderer copies into a texture of its own rather than sampling
    /// where it lies, so a frame is never seen half drawn. `None` for a frame that is its
    /// own until it drops: a decoder's, a screen's.
    pub redrawn: Option<Redrawn>,
}

/// A frame in Direct3D 12 textures: a decoder's, or an upload's, made on the renderer's device
/// or shared from another on its adapter. Integers and a layout, so `nodes/` names no graphics
/// API.
#[derive(Clone)]
pub struct D3d12Texture {
    /// The `ID3D12Device` the textures were made on, as an integer.
    pub device: usize,
    /// Each texture the frame is in: one where Direct3D 12 holds the format whole — NV12 as one
    /// texture of two planes, RGBA, BGRA — or one per plane, NV12's luma and chroma in textures
    /// of their own, where the producer's device holds no NV12 texture.
    pub textures: Vec<D3d12Plane>,
    /// `Nv12`, `Rgba` or `Bgra`.
    pub layout: Layout,
    pub yuv: Yuv,
    /// Whatever owns the textures and the fences — the producer's buffer — held so they stay
    /// valid while a texture samples them. Opaque, so `render/` sees no media type.
    pub keep: Arc<dyn std::any::Any + Send + Sync>,
    /// Raised by the renderer when it will not import the texture, so the source that made it
    /// can go back to bytes. `None` for a source with nothing to fall back to.
    pub refused: Option<Arc<AtomicBool>>,
}

/// One of a [`D3d12Texture`]'s textures. Every integer is valid while the frame's `keep` is
/// alive, and none is owned here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3d12Plane {
    /// The `ID3D12Resource`.
    pub resource: usize,
    /// An NT handle to it where the frame's device is not the renderer's, which the renderer
    /// opens the texture on its own device through; zero on the renderer's device.
    pub shared: usize,
    /// The array slice the frame is in: zero for a texture of its own, any slice of a decoder's
    /// texture array.
    pub slice: u32,
    /// The `ID3D12Fence` the producer signals once this texture is written, and the value it
    /// signals; zero for a texture already written.
    pub fence: usize,
    pub fence_value: u64,
}

impl std::fmt::Debug for D3d12Texture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("D3d12Texture")
            .field("textures", &self.textures)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl PartialEq for D3d12Texture {
    fn eq(&self, other: &Self) -> bool {
        self.device == other.device
            && self.textures == other.textures
            && self.layout == other.layout
            && self.yuv == other.yuv
            && Arc::ptr_eq(&self.keep, &other.keep)
    }
}

/// How a surface that is drawn into again is laid out, for the copy that takes each frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Redrawn {
    /// Its first row is the picture's bottom, as a Syphon surface's is by convention.
    pub bottom_first: bool,
}

impl Frame {
    /// A frame of one color, for tests and for a source that has nothing yet.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        Self {
            width,
            height,
            pixels: Pixels::Bytes(rgba.repeat((width * height) as usize)),
        }
    }

    /// The bytes, if the pixels are tightly packed RGBA8 in system memory — a CPU node's
    /// own, or a source's buffer that happens to be laid out that way.
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.pixels {
            Pixels::Bytes(b) => Some(b),
            Pixels::Mapped(m) | Pixels::IoSurface(IoSurface { mapped: m, .. })
                if m.layout == Layout::Rgba && m.strides[0] == self.width * 4 =>
            {
                m.plane(0, self.width, self.height)
            }
            Pixels::Mapped(_) | Pixels::IoSurface(_) | Pixels::DmaBuf(_) | Pixels::D3d12(_) => None,
        }
    }
}

/// The mark a node puts at the front of its status while something is happening now. A beat
/// that passed is one frame of synth time and several of the editor's, so the node holds the
/// mark for long enough that a frame cannot miss it.
pub const FLASH: char = '\u{25cf}';

/// Per-instance state with a `tick`. Not `Send`: it lives with `App` on the frame thread,
/// and a cpal stream is not `Send` either.
pub trait CpuNode {
    /// Once per frame, in topological order, so an upstream uniform number is already
    /// published.
    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>);

    /// Back to the state `CpuDef::create` made, keeping what was expensive to acquire — an
    /// open device, a decoder, a decoded file — and forgetting everything the ticks since
    /// then accumulated. A render starts every node here rather than from whatever the last
    /// twenty minutes of live play left behind; `tests/reset.rs` holds that a reset instance
    /// and a fresh one publish the same thing.
    fn reset(&mut self);

    /// Whether a render resets this instance where it is, rather than setting it aside for
    /// the live show and running a fresh one in its place: `true` for an instance holding what
    /// is expensive to acquire and nothing a render disturbs but where it is in time — a
    /// decoder, a decoded file, a drawn string. Every other instance is handed back when the
    /// render ends, as the render found it. A device's node (`CpuDef::live`) is always reset
    /// where it is.
    fn reset_in_place(&self) -> bool {
        false
    }

    /// Something wrong with this instance — a device that would not open. Shown on the
    /// status line rather than logged and forgotten.
    fn error(&self) -> Option<String> {
        None
    }

    /// One line for the Status box, such as an audio age.
    fn debug(&self) -> Option<String> {
        None
    }

    /// What this instance is doing that the performer should see — a transcode's progress.
    /// Shown beside the status line and on the node. A line beginning with [`FLASH`] is drawn
    /// in the accent.
    fn status(&self) -> Option<String> {
        None
    }

    /// What this node's audio looks like right now, for the scope on its body.
    ///
    /// `None` for everything that is not an audio source, which is almost everything.
    fn scope(&self) -> Option<crate::audio::Scope> {
        None
    }

    /// A recent history of what this node published, for the trace on its body — `adsr`'s
    /// envelope, most of the way through the segment it is in and the ones before it, or an
    /// `oscillator`'s waveform as it goes past. Each sample is dated, so the band can draw
    /// it by time.
    ///
    /// `None` for everything whose body reserves no trace; a
    /// [`Region::Trace`](super::Region::Trace) among the node's regions is what says one is
    /// reserved.
    fn trace(&self) -> Option<&TraceRing> {
        None
    }

    /// The readings under this node's trace, as `(key, value)` in the order they are drawn —
    /// `adsr`'s gate and its stage, which is silvia's own pair of captions under its envelope
    /// graph.
    ///
    /// Empty for everything whose body reserves no caption; a
    /// [`Region::Caption`](super::Region::Caption) among the node's regions is what says one is
    /// reserved. The key names the cell, and is what the caption's label on screen is built
    /// from.
    fn caption(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    /// How far along `status` is, 0 to 1, when it is the kind of thing that has a far.
    /// Drawn as a bar on the node.
    fn progress(&self) -> Option<f32> {
        None
    }

    /// The curve this node is recording or playing right now, for the region that draws it —
    /// `automation`'s, as the hand is performing it.
    ///
    /// The *saved* curve is one of the node's own values and the region reads it off the
    /// `Node`; this is what the tick has that the document does not yet, which is a recording
    /// in progress and where the playhead is in one. `None` for everything else.
    fn curve(&self) -> Option<Curve> {
        None
    }

    /// Where this node is in whatever it is playing, 0 to 1, for the scrubber on its picture.
    ///
    /// `None` for everything that is not playing a length of something — which is everything
    /// but `video` so far. A *reading*, not a control: what a hand does to it goes back the
    /// other way, through [`TickContext::seek`], because where a clip is playing from is the
    /// node's own state rather than a value the document holds.
    fn playhead(&self) -> Option<f32> {
        None
    }

    /// Is the picture this tick published not yet the one it asked for: a clip whose decoder
    /// has not delivered the frame its position names. A render holds its frame until no
    /// node is waiting, which is what makes a render of a clip the same film twice; live
    /// play never asks, and shows whatever has arrived.
    fn waiting(&self) -> bool {
        false
    }

    /// What a gear is doing, for the region that draws it: its cycles, its ratio and any
    /// ratio waiting to land. `None` for everything but the two gears.
    fn gear(&self) -> Option<crate::nodes::gear::Reading> {
        None
    }

    /// Where a pad's puck is and what is pulling on it this frame, for the region that draws
    /// the pad — `xypad`'s. `None` for everything else.
    ///
    /// A *reading*, like [`Self::playhead`]: the flight is the node's own state, and what a
    /// hand does on the pad goes back the other way, as controls through the bus and as
    /// [`Touch`]es through [`TickContext::touches`].
    fn puck(&self) -> Option<Puck> {
        None
    }
}

/// What a hand did on a node's own surface since the last tick: a seek's sibling, for a
/// gesture that is neither a place in a clip nor a value the document holds.
///
/// One-shot and runtime, never an edit. `xypad`'s wells are silvia's `runtimeState` and are
/// not saved, so dropping one and loading a preset's are said to the tick directly, beside
/// the controls the same press writes through the bus. Points are in the pad's own units, -1
/// to 1 across it and up positive, which is what the node integrates in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    /// A well dropped at `at`, pulled out to `reach` in pad units: how strong a gravity well
    /// is or how long a tether, by the node's own Place Mode. `None` for a click, which is
    /// silvia's default well.
    Well { at: [f32; 2], reach: Option<f32> },
    /// The well at this index in the list the node last reported, taken away.
    Unwell(usize),
    /// One of the node's own presets, started from the top: its puck, its velocity and its
    /// wells.
    Preset(usize),
}

/// One well on a pad: where it is, and whether it attracts or holds the puck on a string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Well {
    pub at: [f32; 2],
    pub pull: Pull,
}

/// What a well does to the puck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pull {
    /// Attracts it, `strength / (distance + 0.05)`: silvia's gravity well.
    Gravity(f32),
    /// Keeps it exactly this far away, as a pendulum's string does.
    Tether(f32),
}

/// A pad's puck this frame, in the pad's own units: what [`CpuNode::puck`] reports.
///
/// Owned, as a [`Curve`] is: it crosses to the editor on the snapshot, and the trail is a
/// few hundred points.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Puck {
    pub at: [f32; 2],
    pub velocity: [f32; 2],
    pub wells: Vec<Well>,
    /// Where the puck has been, oldest first.
    pub path: Vec<[f32; 2]>,
}

/// What a node's transport is doing with a curve this frame: the points as they stand, and
/// where the playhead is among them.
///
/// Owned, the way [`CpuNode::caption`]'s cells are: it crosses to the editor on the snapshot,
/// and a recording is a few hundred points rather than a picture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curve {
    /// Oldest first. While a recording is in progress these are ahead of the node's own
    /// saved value, which is what makes the curve draw as it is performed.
    pub points: Vec<crate::graph::Point>,
    /// Seconds into the recording, where a transport is running. `None` when it is stopped.
    pub head: Option<f32>,
    /// Whether what is being drawn is being recorded rather than played back.
    pub recording: bool,
    /// How many seconds across the band is, so the region draws a point where its time puts
    /// it rather than stretching whatever has been recorded so far to the full width.
    pub span: f32,
}

/// One reading in a trace: what the node published, and the clock's `elapsed` when it did.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceSample {
    /// Seconds on the one clock — [`TickContext::elapsed`] at the tick that published it.
    pub at: f64,
    /// What the node published.
    pub value: f32,
}

/// The ring a [`CpuNode::trace`] pushes into: the last [`span`](Self::span) seconds of what
/// the node published, oldest first, each sample dated.
///
/// **Dated, because the band draws it by time and not by sample.** A frame is not a unit of
/// time: when the compositor holds a frame back, the next `dt` is two refreshes long, the
/// accumulator correctly advances twice as far, and a plot that puts every sample one step
/// from the last draws that as a kink in a wave that has none. Plotted against `at`, a late
/// frame is a wider gap on a line whose shape is still the shape. So the window is a span of
/// seconds rather than a count of samples, and the count is only a bound on memory.
#[derive(Debug, Clone, PartialEq)]
pub struct TraceRing {
    span: f32,
    samples: std::collections::VecDeque<TraceSample>,
}

/// A bound on memory, not the window: the window is the span. The longest span in the
/// library is five seconds, which reaches this only past 1600 frames a second.
const TRACE_MAX_SAMPLES: usize = 8192;

impl TraceRing {
    /// A ring holding `span` seconds.
    pub const fn new(span: f32) -> Self {
        Self {
            span,
            samples: std::collections::VecDeque::new(),
        }
    }

    /// Record `value` as published at `at`, dropping what has aged out of the span.
    ///
    /// A time before the newest sample's is a clock that started over — a driven render's
    /// warm-up, or a reset — and nothing recorded before it is on this timeline, so the ring
    /// starts again rather than drawing two runs on one axis.
    pub fn push(&mut self, at: f64, value: f32) {
        if self.samples.back().is_some_and(|s| at < s.at) {
            self.samples.clear();
        }
        self.samples.push_back(TraceSample { at, value });
        let horizon = at - f64::from(self.span);
        while self.samples.front().is_some_and(|s| s.at < horizon) {
            self.samples.pop_front();
        }
        while self.samples.len() > TRACE_MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// How many seconds the ring holds, which is the width of the band in time.
    pub fn span(&self) -> f32 {
        self.span
    }

    /// The samples, oldest first.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &TraceSample> {
        self.samples.iter()
    }

    /// How many samples the span holds right now.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Nothing published yet — a node that has not ticked.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The most recent sample: the right edge of the band.
    pub fn newest(&self) -> Option<TraceSample> {
        self.samples.back().copied()
    }
}

/// What the canvas shows on a node about its CPU half, gathered by the app each frame.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeNote {
    pub text: String,
    pub progress: Option<f32>,
}

/// How a definition makes its CPU state, and the two facts about it that an offline render
/// needs: whether its state is a sum over every frame before this one, and whether it reads
/// something only the wall clock can answer.
pub struct CpuDef {
    pub create: fn() -> Box<dyn CpuNode>,
    /// Does `tick` integrate: is the state at frame *n* a function of every frame before it —
    /// a gear, a slew, an envelope, a game, a sequencer's open gates — so that it can be run
    /// but never seeked. A registry test holds this to the source: a tick that reads `ctx.dt`
    /// or `ctx.elapsed` either integrates or is `live`, and one that integrates reads the
    /// transport.
    pub integrates: bool,
    /// Does `tick` read a device — a camera, a microphone, the Main Input's capture — so that
    /// stepped to an arbitrary `t` it can only hand back whatever the wall clock gave it.
    /// This is what an Output's `!` walks the graph for.
    pub live: bool,
}

/// The project's media, from a node's side: where a reference names a file, and where the
/// files derived from one — a transcode, a decoded soundtrack — belong.
///
/// The project implements it, and a node reaches it only through `TickContext::path` and
/// `TickContext::cache_dir`. That is what keeps the project root out of `nodes/`: a node
/// holds a reference, asks for a path, and never learns where the project is.
pub trait Assets {
    /// Where a reference names a file on this machine.
    fn resolve(&self, reference: &str) -> PathBuf;

    /// Where derived files go. Inside the project, so a folder carries its own transcodes.
    fn cache_dir(&self) -> PathBuf;
}

/// The Main Input, as a node reads it: one frame's worth, borrowed.
///
/// Assembled once by the app and handed to every `maininput` node in the graph, which is what
/// makes them all say the same thing about the same signal. See [`crate::maininput`].
#[derive(Clone, Copy)]
pub struct MainInputFeed<'a> {
    /// The newest picture. `None` before the first one arrives, or with no video source.
    pub frame: Option<&'a Arc<Frame>>,
    pub analysis: crate::audio::Analysis,
    /// The rate the analysis was made at, for placing a crossing inside this frame.
    pub sample_rate: f32,
    /// The panel's one gain, applied to every published level.
    pub gain: f32,
    /// Thresholds crossed since the last frame, collected once and fired by every node.
    pub crossings: &'a [crate::audio::Crossing],
    /// Where each band listens, as the panel has it tuned — for the meters a reader draws.
    pub config: [crate::audio::bands::BandConfig; crate::audio::BANDS],
    /// The level each band fires at, as the panel has it. Drawn on a reader's meters and not
    /// settable there: there is one capture, so there is one answer.
    pub thresholds: [f32; crate::audio::BANDS],
}

/// A feed with nothing on it: silence, no picture, and the panel's own defaults for the
/// tuning and the thresholds.
///
/// Written out rather than derived because a derived `[f32; BANDS]` is zeros, and a threshold
/// of zero is a band that fires on silence. One is the top of the range and the default
/// everywhere else, which is a rig that opens quiet.
impl Default for MainInputFeed<'_> {
    fn default() -> Self {
        Self {
            frame: None,
            analysis: crate::audio::Analysis::default(),
            sample_rate: 0.0,
            gain: 1.0,
            crossings: &[],
            config: crate::audio::bands::DEFAULT,
            thresholds: [1.0; crate::audio::BANDS],
        }
    }
}

/// The value on a uniform number input this frame: what its source published where it is
/// connected, its control where it is not, and zero for anything else.
pub fn number<S: std::hash::BuildHasher>(
    graph: &Graph,
    uniforms: &HashMap<PortRef, f32, S>,
    input: PortRef,
) -> f32 {
    if let Some(src) = graph.source_of(input) {
        return uniforms.get(&src).copied().unwrap_or(0.0);
    }
    match graph
        .get(input.node)
        .and_then(|n| n.controls.get(input.key))
    {
        Some(ControlValue::Float(v)) => *v,
        _ => 0.0,
    }
}

/// What a `tick` can see and say. Inputs resolve exactly as the compiler resolves them: a
/// connected port reads the producer's published value, an unconnected one reads its
/// control, and anything else is zero.
pub struct TickContext<'a> {
    /// The step a stateful node takes this tick, in seconds: the transport's advance for this
    /// node, clamped to [`crate::transport::MAX_DT`] live, and zero on a jump and while
    /// paused. What a simulation, a slew, an envelope and a
    /// pad's physics integrate, and what an event's moment is measured in: an [`Event::at`]
    /// is in `0..dt`.
    pub dt: f32,
    /// The one clock's `elapsed`, in seconds: the wall's, for dating a trace or timing a tap,
    /// never for integrating.
    pub elapsed: f64,
    /// What this node sees of the transport: the playhead, the advance since this node last
    /// ticked, whether a jump is inside it, and whether it plays. What a gear integrates,
    /// through [`crate::nodes::phasor`]; a node on ambient time reads [`Self::clock`].
    pub time: crate::transport::Time,
    graph: &'a Graph,
    uniforms: &'a mut HashMap<PortRef, f32>,
    /// Every count published so far, in `f64` and unbounded: what a Time reads whole. See
    /// [`Self::publish_count`].
    counts: &'a mut HashMap<PortRef, f64>,
    uniform_colors: &'a mut HashMap<PortRef, [f32; 4]>,
    frames: &'a mut HashMap<PortRef, Arc<Frame>>,
    /// Every simulation published so far, with the passes the renderer has not run yet.
    sims: &'a mut HashMap<PortRef, crate::nodes::Simulation>,
    /// Every event fired so far this frame, by the output port that fired it. Cleared at the
    /// top of each tick: an event lives for one frame, and a consumer that wants to remember
    /// one remembers it itself.
    ///
    /// Producers tick before consumers, so an entry is complete by the time anything
    /// downstream reads it.
    actions: &'a mut HashMap<PortRef, Vec<Event>>,
    /// The number outputs put somewhere this frame rather than moved there. See
    /// [`Self::jump`].
    jumps: &'a mut HashSet<PortRef>,
    /// Action ports whose hand-button was held at the end of the last frame. The UI is
    /// drawn after `tick`, so a press arrives on the next one — sixteen milliseconds, which
    /// is a click.
    held: &'a HashSet<PortRef>,
    /// What each measured node's slot held after the last frame its workspace's pass
    /// finished. Absent for a node no pass measures.
    readbacks: &'a HashMap<NodeId, [u32; crate::compile::TAP_WORDS]>,
    /// Turns an asset reference into a path. The project, in the running app.
    assets: &'a dyn Assets,
    /// Where a hand dragged a node's scrubber, 0 to 1, since the last tick. One-shot, unlike
    /// `held`: a seek is a place asked for once, not a level held down.
    seeks: &'a HashMap<NodeId, f32>,
    /// What a hand did on a node's own surface since the last tick, oldest first. One-shot,
    /// as a seek is.
    touches: &'a HashMap<NodeId, Vec<Touch>>,
    /// What the Main Input panel is holding open, for the nodes that read it.
    main_input: MainInputFeed<'a>,
    /// Where the pointer is over each surface, read once at the top of the tick. See
    /// [`crate::pointer`].
    pointer: crate::pointer::Frame,
    /// Number controls a tick asked to move, for the synth to write once the walk is over.
    ///
    /// The one thing a node says about the *document* rather than about this frame: a tap
    /// tempo is a number a hand could have typed, so it belongs on the knob where it can be
    /// seen and nudged. See [`Self::write_control`].
    control_writes: &'a mut Vec<(PortRef, f32)>,
    /// Values a tick asked to write onto its own node, for the synth to apply once the walk
    /// is over. `automation`'s recording, and nothing else so far. See
    /// [`Self::write_value`].
    value_writes: &'a mut Vec<(NodeId, &'static str, crate::graph::Value)>,
    /// The ticking node's own playhead while it runs free, in seconds of its pace: what the
    /// synth's integrator made of its Speed this tick (`nodes::timing::Pace`). `None` for a
    /// node that loops. See [`Self::cycle`].
    free: Option<(NodeId, f64)>,
}

impl<'a> TickContext<'a> {
    // Eleven borrows of the frame's state and two numbers, all of them separate fields of
    // `App` that a tick reads or writes. Bundling them into a struct would build that struct
    // at the one call site and pass the same eleven things through it.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        graph: &'a Graph,
        uniforms: &'a mut HashMap<PortRef, f32>,
        counts: &'a mut HashMap<PortRef, f64>,
        uniform_colors: &'a mut HashMap<PortRef, [f32; 4]>,
        frames: &'a mut HashMap<PortRef, Arc<Frame>>,
        sims: &'a mut HashMap<PortRef, crate::nodes::Simulation>,
        actions: &'a mut HashMap<PortRef, Vec<Event>>,
        jumps: &'a mut HashSet<PortRef>,
        held: &'a HashSet<PortRef>,
        readbacks: &'a HashMap<NodeId, [u32; crate::compile::TAP_WORDS]>,
        assets: &'a dyn Assets,
        seeks: &'a HashMap<NodeId, f32>,
        touches: &'a HashMap<NodeId, Vec<Touch>>,
        main_input: MainInputFeed<'a>,
        pointer: crate::pointer::Frame,
        control_writes: &'a mut Vec<(PortRef, f32)>,
        value_writes: &'a mut Vec<(NodeId, &'static str, crate::graph::Value)>,
        dt: f32,
        elapsed: f64,
        time: crate::transport::Time,
    ) -> Self {
        Self {
            dt,
            elapsed,
            time,
            graph,
            uniforms,
            counts,
            uniform_colors,
            frames,
            sims,
            actions,
            jumps,
            held,
            readbacks,
            assets,
            seeks,
            touches,
            main_input,
            pointer,
            control_writes,
            value_writes,
            free: None,
        }
    }

    /// The same, for `id` running free with its own playhead at `at`, in seconds of its pace.
    #[must_use]
    pub fn running_free(mut self, id: NodeId, at: Option<f64>) -> Self {
        self.free = at.map(|at| (id, at));
        self
    }

    /// Whether `id` runs free this tick: Speed in its Time's place.
    pub fn runs_free(&self, id: NodeId) -> bool {
        self.free.is_some_and(|(node, _)| node == id)
    }

    /// How far through this tick's advance a moment `at` seconds into the frame is, 0 to 1:
    /// what a phasor walks to for an event, `at / dt`.
    pub fn fraction(&self, at: f32) -> f64 {
        if self.dt > 0.0 {
            f64::from((at / self.dt).clamp(0.0, 1.0))
        } else {
            0.0
        }
    }

    /// The moment inside the frame, in seconds, that is `fraction` of the way through this
    /// tick's advance: what an event is stamped with, `fraction × dt`.
    pub fn moment(&self, fraction: f64) -> f32 {
        (fraction.clamp(0.0, 1.0) as f32) * self.dt
    }

    /// What the Main Input panel is holding open this frame.
    pub fn main_input(&self) -> MainInputFeed<'a> {
        self.main_input
    }

    /// Where the pointer is over one surface this frame, or `None` where it is not over it.
    ///
    /// The pointer is a device like any other: the reading is taken once at the top of the
    /// tick and handed to every node that asks, so two `mouseinput` nodes framed the same
    /// way agree about where the hand is. See [`crate::pointer`].
    pub fn pointer(&self, surface: crate::pointer::Surface) -> Option<crate::pointer::Reading> {
        self.pointer.of(surface)
    }

    /// What `id`'s tap slot held after the last frame its workspace's pass finished on the
    /// GPU. `None` for a node no pass measures, before the first readback, and always in a
    /// headless app — which is what a tap not measuring says about itself.
    pub fn readback(&self, id: NodeId) -> Option<&[u32; crate::compile::TAP_WORDS]> {
        self.readbacks.get(&id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.graph.get(id)
    }

    /// The value on one of `id`'s uniform number inputs this frame.
    pub fn input(&self, id: NodeId, key: &'static str) -> f32 {
        number(self.graph, self.uniforms, PortRef::new(id, key))
    }

    /// The color on one of `id`'s uniform color inputs this frame.
    ///
    /// `input`'s shape for the other kind: a connected port reads the producer's published
    /// color, an unconnected one reads its swatch, and anything else is transparent black.
    pub fn color(&self, id: NodeId, key: &'static str) -> [f32; 4] {
        if let Some(src) = self.graph.source_of(PortRef::new(id, key)) {
            return self.uniform_colors.get(&src).copied().unwrap_or([0.0; 4]);
        }
        match self.graph.get(id).and_then(|n| n.controls.get(key)) {
            Some(ControlValue::Color(v)) => *v,
            _ => [0.0; 4],
        }
    }

    /// What arrives at one of `id`'s uniform number inputs read as a count, in `f64`: a count
    /// its source published whole ([`Self::publish_count`]), unbounded and to `f64`'s
    /// precision, or else the one `f32` [`Self::input`] reads.
    pub fn count(&self, id: NodeId, key: &'static str) -> f64 {
        self.graph
            .source_of(PortRef::new(id, key))
            .and_then(|src| self.counts.get(&src).copied())
            .unwrap_or_else(|| f64::from(self.input(id, key)))
    }

    /// Whether what arrives at one of `id`'s inputs is a count published whole, which
    /// [`Self::count`] reads to `f64`'s precision and never wrapped; anything else is one
    /// `f32`.
    pub fn counted(&self, id: NodeId, key: &'static str) -> bool {
        self.graph
            .source_of(PortRef::new(id, key))
            .is_some_and(|src| self.counts.contains_key(&src))
    }

    /// Where `id` is this tick, in its own cycles and unwrapped, with its Offset added: in
    /// Loop mode its Time — what is cabled in, read as a count ([`Self::count`]), or ambient
    /// time at its rest rate, from the `f64` playhead — and in Free mode its own playhead at
    /// its pace. See [`crate::nodes::timing`].
    pub fn cycle(&self, id: NodeId) -> f64 {
        let timing = self.graph.get(id).and_then(|n| n.def.timing);
        let rate = timing.map_or(0.0, |t| if self.runs_free(id) { t.pace } else { t.rate });
        self.cycle_at(id, rate)
    }

    /// The same at a rate the node works out itself — a clip's one play over its length, which
    /// is both its rate at rest and its pace.
    pub fn cycle_at(&self, id: NodeId, rate: f64) -> f64 {
        let time = match self.free {
            Some((node, at)) if node == id => at * rate,
            _ if self.connected(id, crate::nodes::TIME) => self.count(id, crate::nodes::TIME),
            _ => self.time.playhead * rate,
        };
        time + f64::from(self.input(id, crate::nodes::timing::OFFSET))
    }

    /// Where the clock cabled into one of `id`'s inputs wraps as [`Self::count`] reads it:
    /// never for a count published whole, and otherwise [`OutputDef::wraps_at`] of the output
    /// feeding it — one for a Phase, 2520 for one `f32` of a count through a Math node
    /// (−1260 up to 1260), or anything unplugged.
    ///
    /// [`OutputDef::wraps_at`]: crate::nodes::OutputDef::wraps_at
    pub fn wraps_at(&self, id: NodeId, key: &'static str) -> f64 {
        if self.counted(id, key) {
            return f64::INFINITY;
        }
        self.graph
            .source_of(PortRef::new(id, key))
            .and_then(|src| self.graph.get(src.node)?.def.output(src.key))
            .map_or(crate::nodes::phasor::WRAP, |out| out.wraps_at)
    }

    /// The output plugged into one of `id`'s inputs, if any: what a node that keeps a reading
    /// of a cable asks, so a cable moved onto another source is a new reading.
    pub fn source(&self, id: NodeId, key: &'static str) -> Option<PortRef> {
        self.graph.source_of(PortRef::new(id, key))
    }

    /// Is anything plugged into one of `id`'s inputs? A CPU node whose behavior depends on
    /// whether an input is driven — free-running against a connected position — asks here.
    pub fn connected(&self, id: NodeId, key: &'static str) -> bool {
        self.graph.source_of(PortRef::new(id, key)).is_some()
    }

    /// The current value of one of `id`'s select options.
    pub fn option(&self, id: NodeId, key: &str) -> &str {
        self.graph
            .get(id)
            .and_then(|n| n.options.get(key))
            .map_or("", String::as_str)
    }

    /// The text one of `id`'s own [values](crate::nodes::ValueKind::Text) holds.
    ///
    /// A value is document data like an option, so this reads it the same way; what differs
    /// is who drew it, which is nothing a tick has to know. `text`'s words are the one there
    /// is.
    pub fn text(&self, id: NodeId, key: &str) -> &str {
        self.graph
            .get(id)
            .and_then(|n| n.values.get(key))
            .and_then(crate::graph::Value::text)
            .unwrap_or("")
    }

    /// Where one of `id`'s `Asset` options points, as a path.
    ///
    /// The option holds a reference — `assets/gumbasia.webm` for a file in the project,
    /// an absolute path for one somebody typed by hand — and this is what turns it into
    /// something openable. `None` where the option is empty, which is a node with no file
    /// chosen.
    pub fn path(&self, id: NodeId, key: &str) -> Option<PathBuf> {
        let reference = self.option(id, key);
        if reference.is_empty() {
            return None;
        }
        Some(self.assets.resolve(reference))
    }

    /// Where this project keeps the files derived from its media: a transcode, a decoded
    /// soundtrack. Created by whoever writes into it.
    pub fn cache_dir(&self) -> PathBuf {
        self.assets.cache_dir()
    }

    /// Publish one of `id`'s uniform number outputs for this frame.
    pub fn publish(&mut self, id: NodeId, port: &'static str, value: f32) {
        // A NaN would travel into a uniform and paint an undefined frame.
        let value = if value.is_finite() { value } else { 0.0 };
        self.uniforms.insert(PortRef::new(id, port), value);
    }

    /// Publish one of `id`'s outputs as a **count**: a clock's reading in its own cycles,
    /// `count`, unbounded, which a Time reads whole — a CPU node in `f64` through
    /// [`Self::count`], a shader as its whole part and the `f32` of its fraction
    /// ([`crate::nodes::phasor::split`]) — and `one`, the one `f32` everything else reads: a
    /// Math node, an input that is not a Time, the row's number.
    pub fn publish_count(&mut self, id: NodeId, port: &'static str, count: f64, one: f32) {
        let count = if count.is_finite() { count } else { 0.0 };
        self.counts.insert(PortRef::new(id, port), count);
        self.publish(id, port, one);
    }

    /// Publish one of `id`'s uniform color outputs for this frame.
    pub fn publish_color(&mut self, id: NodeId, port: &'static str, color: [f32; 4]) {
        // A NaN would travel into a uniform and paint an undefined frame, exactly as
        // one on a number would.
        let color = color.map(|c| if c.is_finite() { c } else { 0.0 });
        self.uniform_colors.insert(PortRef::new(id, port), color);
    }

    /// Take one of `id`'s uniform number outputs out of this frame.
    ///
    /// The opposite of `publish`, for a port that has nothing to say: a tap whose
    /// measurement no pass ran has no reading, and a reading of nothing is not a reading
    /// of zero. The row then draws nothing, under the rule an unpublished port follows.
    pub fn withdraw(&mut self, id: NodeId, port: &'static str) {
        self.uniforms.remove(&PortRef::new(id, port));
        self.counts.remove(&PortRef::new(id, port));
    }

    /// Take one of `id`'s uniform color outputs out of this frame. [`Self::withdraw`] for the
    /// other kind: a measurement that did not run has no color, and a color of nothing is not
    /// transparent black.
    pub fn withdraw_color(&mut self, id: NodeId, port: &'static str) {
        self.uniform_colors.remove(&PortRef::new(id, port));
    }

    /// Fire an edge on one of `id`'s action outputs, at a moment this source could not place
    /// more precisely than the frame.
    ///
    /// Nothing is queued and nothing waits: the event is in the map, and every node
    /// downstream ticks later this same frame.
    pub fn fire(&mut self, id: NodeId, port: &'static str, edge: Edge) {
        self.fire_at(id, port, edge, 0.0);
    }

    /// Fire an edge that happened `at` seconds into this frame.
    ///
    /// For a source with a phase of its own. A clock knows where between two frames its beat
    /// actually fell, and saying so is the difference between an envelope that is exact and
    /// one quantized to the display.
    pub fn fire_at(&mut self, id: NodeId, port: &'static str, edge: Edge, at: f32) {
        self.actions
            .entry(PortRef::new(id, port))
            .or_default()
            .push(Event::at(edge, at.clamp(0.0, self.dt)));
    }

    /// Say that one of `id`'s number outputs was put where it is this frame rather than moved
    /// there: a gear's Reset, which sends its count back less than a cycle as readily as more.
    /// A reader keeping last frame's reading takes the step as a jump, never as motion
    /// backwards — a step back alone cannot tell a Reset from a clock that runs backwards.
    pub fn jump(&mut self, id: NodeId, port: &'static str) {
        self.jumps.insert(PortRef::new(id, port));
    }

    /// Whether the output cabled into one of `id`'s inputs was put where it is this frame
    /// rather than moved there: [`Self::jump`] on its source, which ticked first.
    pub fn jumped(&self, id: NodeId, key: &'static str) -> bool {
        self.graph
            .source_of(PortRef::new(id, key))
            .is_some_and(|src| self.jumps.contains(&src))
    }

    /// Every event that arrived on one of `id`'s action inputs this frame, oldest first.
    ///
    /// An action input may have several sources, so this is a gather rather than a lookup,
    /// and it is sorted: a consumer integrating between events needs one timeline, not one
    /// per cable. Two sources firing `Down` in one frame are two events — collapsing them
    /// would drop a beat, which is the whole reason a counter can trust this.
    pub fn edges(&self, id: NodeId, key: &'static str) -> Vec<Event> {
        let mut out = Vec::new();
        for src in self.graph.sources_of(PortRef::new(id, key)) {
            if let Some(events) = self.actions.get(&src) {
                out.extend_from_slice(events);
            }
        }
        // Stable, so two sources firing at the same moment stay in graph order.
        out.sort_by(|a, b| a.at.total_cmp(&b.at));
        out
    }

    /// Is a hand holding the button on one of `id`'s action inputs?
    ///
    /// Unlike a number control, this does not go quiet when something is connected: an
    /// action input takes many sources, and a hand is one more of them. That is what makes
    /// a button beside a sequencer lane an override rather than a conflict.
    pub fn pressed(&self, id: NodeId, key: &'static str) -> bool {
        self.held.contains(&PortRef::new(id, key))
    }

    /// How many `Down` edges arrived on one of `id`'s action inputs this frame, counting the
    /// hand on its button as one more source.
    ///
    /// A count rather than a bool, because two sources firing in one frame are two events: a
    /// counter that collapsed them drifts against the clock driving it, and a toggle that did
    /// would flip once where it was told twice. `hand` is the node's own [`Gate`] — the level
    /// a button reports becomes an edge in exactly one place.
    pub fn downs(&self, id: NodeId, key: &'static str, hand: &mut Gate) -> u32 {
        let mut n = 0;
        for event in self.edges(id, key) {
            if event.is_down() {
                n += 1;
            }
        }
        if hand.set(self.pressed(id, key)) == Some(Edge::Down) {
            n += 1;
        }
        n
    }

    /// The moment inside this frame an action input last went down, in seconds, counting the
    /// hand on its button as a down at the top of the frame, or `None` if nothing did: where a
    /// node with a start of its own restarts. The events are oldest first, so the last wins.
    pub fn last_down(&self, id: NodeId, key: &'static str, hand: &mut Gate) -> Option<f32> {
        let mut at = None;
        if hand.set(self.pressed(id, key)) == Some(Edge::Down) {
            at = Some(0.0);
        }
        for event in self.edges(id, key) {
            if event.is_down() {
                at = Some(event.at);
            }
        }
        at
    }

    /// Where a hand dragged this node's scrubber since the last tick, 0 to 1 through whatever
    /// it is playing, or `None` if nobody did.
    ///
    /// One-shot, where `pressed` is a level: a seek is a place asked for once. What it does
    /// with it is the node's — a clip writes it into its Offset and goes on playing from there
    /// on its Time, which is what makes a scrubber a scrubber rather than a transport.
    pub fn seek(&self, id: NodeId) -> Option<f32> {
        self.seeks.get(&id).copied()
    }

    /// What a hand did on this node's own surface since the last tick, oldest first: empty
    /// where nobody did. One-shot, as a seek is.
    pub fn touches(&self, id: NodeId) -> &[Touch] {
        self.touches.get(&id).map_or(&[], Vec::as_slice)
    }

    /// Move one of `id`'s own number controls, as a hand on the knob would.
    ///
    /// **The one write a tick makes to the document.** A number a node sets for a person —
    /// a roll of `slimemold`'s knobs, a clip's scrubber on Offset — has to land on the knob,
    /// where it reads, nudges and saves like any other number, rather than in a private field
    /// the node shows nobody. The write takes the path a bound MIDI
    /// knob's does — the synth's own graph now, and the editor's through the command bus a
    /// frame later — so it is one undo step and the file says what the desk says. See
    /// [`crate::synth::Synth::write_control`].
    ///
    /// A node writes only its own control, and only a number: everything else a tick has to
    /// say goes out through a port.
    pub fn write_control(&mut self, id: NodeId, key: &'static str, value: f32) {
        if value.is_finite() {
            self.control_writes.push((PortRef::new(id, key), value));
        }
    }

    /// Write one of `id`'s own declared values, as an editor that could draw it would.
    ///
    /// The second write a tick makes to the document, and it exists for the same reason the
    /// first does: a curve a hand performed has to be saved, undone and carried with the
    /// patch, which a field the node keeps to itself would not be. It takes the seam
    /// `write_control` takes — the synth's own graph on the tick it was asked for, and the
    /// document through the command bus as the `SetValue` a hand would have sent.
    ///
    /// **Not once a frame.** A recording lands when it stops, not while it runs, because
    /// every write is one step of the undo history.
    pub fn write_value(&mut self, id: NodeId, key: &'static str, value: crate::graph::Value) {
        self.value_writes.push((id, key, value));
    }

    /// What `id` has stored under one of its own declared values — a recording a file
    /// carried in, read back by the tick that plays it.
    pub fn value(&self, id: NodeId, key: &str) -> Option<&crate::graph::Value> {
        self.graph.get(id).and_then(|n| n.values.get(key))
    }

    /// Publish one of `id`'s texture outputs for this frame. The renderer uploads it once;
    /// publishing the same `Arc` again costs nothing.
    ///
    /// Keyed by port, not by node: a node declares as many outputs as it likes and a
    /// `VaryingColor` output is an expression — inlined WGSL, a texture sample, a lookup — so
    /// nothing stops one node publishing several. A video's picture and its oscilloscope are
    /// two.
    pub fn publish_frame(&mut self, id: NodeId, port: &'static str, frame: Arc<Frame>) {
        self.frames.insert(PortRef::new(id, port), frame);
    }

    /// Publish this tick of one of `id`'s simulated texture outputs: the world's shape, the
    /// numbers its kernels read, and the passes to run. See [`crate::nodes::sim`].
    ///
    /// The passes are **appended** to any the renderer has not run yet, so no step is lost
    /// between two frame jobs; the shape and the numbers replace what was there.
    pub fn publish_sim(&mut self, id: NodeId, port: &'static str, sim: crate::nodes::Simulation) {
        match self.sims.entry(PortRef::new(id, port)) {
            std::collections::hash_map::Entry::Occupied(mut held) => {
                let held = held.get_mut();
                held.size = sim.size;
                held.agents = sim.agents;
                held.params = sim.params;
                held.steps = sim.steps;
                held.passes.extend(sim.passes);
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(sim);
            }
        }
    }
}

#[cfg(test)]
mod trace_ring_tests {
    use super::TraceRing;

    /// The window is seconds, not samples: at a steady frame rate the ring holds the span
    /// and no more, oldest first, and what fell off is what is older than the span.
    #[test]
    fn the_ring_keeps_its_span_of_seconds_in_order() {
        let mut ring = TraceRing::new(1.0);
        assert!(ring.is_empty(), "nothing published yet");
        for i in 0..200 {
            ring.push(f64::from(i) / 100.0, i as f32);
        }
        let newest = ring.newest().unwrap();
        assert_eq!(newest.value, 199.0, "newest last");
        let oldest = ring.iter().next().unwrap();
        assert!(
            oldest.at >= newest.at - 1.0,
            "nothing older than the span: {} against {}",
            oldest.at,
            newest.at
        );
        assert_eq!(ring.len(), 101, "a second at 100 Hz, both ends included");
    }

    /// A late frame is a wider gap, not a lost sample: the ring records when, and a plot by
    /// time is what keeps a correct wave from looking like a wrong one.
    #[test]
    fn a_late_frame_is_dated_where_it_happened() {
        let mut ring = TraceRing::new(1.0);
        ring.push(0.00, 0.0);
        ring.push(0.01, 1.0);
        ring.push(0.03, 2.0);
        let at: Vec<f64> = ring.iter().map(|s| s.at).collect();
        assert_eq!(at, vec![0.0, 0.01, 0.03]);
    }

    /// A clock that starts over — a warm-up, a reset — starts the ring over: two runs never
    /// share one axis.
    #[test]
    fn time_going_backwards_starts_the_ring_over() {
        let mut ring = TraceRing::new(5.0);
        ring.push(10.0, 1.0);
        ring.push(10.1, 2.0);
        ring.push(-0.5, 3.0);
        let values: Vec<f32> = ring.iter().map(|s| s.value).collect();
        assert_eq!(values, vec![3.0]);
    }

    /// The count cap is a bound on memory and never the window: pushing far past it at an
    /// absurd rate keeps the newest and no more than the cap.
    #[test]
    fn the_cap_bounds_memory_not_the_window() {
        let mut ring = TraceRing::new(1000.0);
        for i in 0..(super::TRACE_MAX_SAMPLES + 100) {
            ring.push(i as f64 * 1e-6, i as f32);
        }
        assert_eq!(ring.len(), super::TRACE_MAX_SAMPLES);
        assert_eq!(
            ring.newest().unwrap().value,
            (super::TRACE_MAX_SAMPLES + 99) as f32
        );
    }
}
