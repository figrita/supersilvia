// SPDX-License-Identifier: AGPL-3.0-or-later

//! Node definitions: static data plus one generator per output. No GPU, no egui.
//!
//! A definition is a `const NodeDef`, not a trait implementation. Each output carries its
//! own generator, so a node with several outputs is several `OutputDef`s rather than a
//! dispatch on port key, and adding an output without a generator is a compile error.
//!
//! A generator returns **only the function body**. The signature is derived from the port's
//! `PortType`, which is why the declared type and the emitted return type cannot disagree.

pub mod action;
pub mod adjust;
pub mod adsr;
pub mod alpha;
pub mod animation;
pub mod area;
pub mod attach;
pub mod audio_ports;
pub mod audioin;
pub mod autoexposure;
pub mod autogain;
pub mod automation;
pub mod brickgame;
pub mod bridge;
pub mod button;
pub mod camcordercrt;
pub mod camera;
pub mod cellularautomata;
pub mod chain;
pub mod checkerboard;
pub mod chromakey;
pub mod chromaticaberration;
pub mod clock;
pub mod clockdivider;
pub mod color;
pub mod convolve;
pub mod cosinegradient;
pub mod counter;
pub mod cpu;
pub mod decompose;
pub mod distort;
pub mod drawingcanvas;
pub mod edgedetection;
pub mod euclideanrhythm;
pub mod fractals;
pub mod gamepad;
pub mod gear;
pub mod geissflow;
pub mod glitch;
pub mod gradients;
pub mod hsla;
pub mod imagegif;
pub mod macros;
pub mod maininput;
pub mod math;
pub mod mix;
pub mod mouseinput;
pub mod multisample;
pub mod muxevent;
pub mod muxnumber;
pub mod ndi;
pub mod noise;
pub mod note;
pub mod number;
pub mod oscillator;
pub mod output;
pub mod palette;
pub mod patterns;
pub mod phasor;
pub mod pixelsort;
pub mod random;
pub mod randomfire;
pub mod randomhurl;
pub mod recolor;
pub mod reframerange;
pub mod region;
pub mod rgba;
pub mod rng;
pub mod sample;
pub mod saturate;
pub mod screencapture;
pub mod screentone;
pub mod sequencer;
pub mod shapes;
pub mod sim;
pub mod slew;
pub mod sliderule;
pub mod slimemold;
pub mod smoothcounter;
pub mod stargate;
pub mod stepsequencer;
pub mod syphon;
pub mod tap;
#[cfg(test)]
pub mod test_support;
pub mod text;
pub mod time;
pub mod timing;
pub mod transform;
pub mod triggeredcolor;
pub mod triggeredrandom;
pub mod video;
pub mod wallpaper;
pub mod wavefold;
pub mod worldcoordinates;
pub mod xypad;

pub use action::{Edge, Event, Gate};
pub use area::{Buttons, Heading, Picture, Region, Settings};
pub use bridge::{Bridge, bridges, can_bridge, can_bridge_within};
pub use cpu::{
    Assets, CpuDef, CpuNode, D3d12Plane, D3d12Texture, DmaBuf, Frame, IoSurface, Layout,
    MainInputFeed, Mapped, NodeNote, Pixels, Planes, Redrawn, TickContext, Yuv, YuvMatrix,
};
pub use sim::{Domain, Kernel, Pass, Simulation};

use crate::compile::CompileContext;
use crate::graph::{
    ControlRange, ControlValue, Graph, Node, NodeId, PortDef, PortType, WorkspaceId,
};
use emath::Pos2;

/// Emits the body of one output's function, in WGSL.
/// `func` is the function's name, for nodes that need to reference it; most do not.
pub type GenFn = fn(node: NodeId, ctx: &mut CompileContext, func: &str) -> String;

/// The same formula as an output's WGSL, evaluated on the CPU for one frame.
///
/// What makes an output **dual**: its declared type is `VaryingNumber`, and on an instance
/// whose inputs all resolve to uniform numbers there is nothing per-pixel about it, so `tick`
/// computes it and publishes it as a uniform number. See
/// [Dual outputs](../../docs/nodes.md#dual-outputs).
pub type EvalFn = fn(node: NodeId, ctx: &TickContext<'_>) -> f32;

/// The generator every `OutputKind::Uniform` and `OutputKind::Action` output carries, and any
/// output nobody has written a body for. A uniform number is resolved to a uniform before the
/// compiler could ever call it and an action never reaches the compiler at all, so a call is a
/// placeholder of the function's type and a
/// [`Diagnostic::Untranslated`](crate::compile::Diagnostic::Untranslated) naming the node.
pub fn no_wgsl(node: NodeId, ctx: &mut CompileContext, _func: &str) -> String {
    ctx.untranslated(node)
}

/// The height in pixels that a size in pixels is measured against.
///
/// No node body reads the resolution, so a size in pixels is pixels of a frame this tall: a
/// node's field is the same in every Output, and each Output draws that field at its own
/// pixel pitch. Worldspace is exactly 2.0 units tall, so one world unit is half this many
/// pixels. See Worldspace in `docs/nodes.md`.
pub const REFERENCE_HEIGHT: f32 = 720.0;

/// What an input falls back to when nothing is connected.
pub enum Control {
    /// Nothing. A `color` input becomes `defaultUvMap(uv)`, a `float` becomes `0.0`.
    None,
    Number {
        default: f32,
        min: f32,
        max: f32,
        step: f32,
        unit: &'static str,
        /// Scrub in log space. For a range spanning decades — a zoom of 0.01 to 100 — a
        /// linear drag spends its whole travel below 1.
        log: bool,
        /// Another input of the same node whose value is this control's top end.
        ///
        /// A kaleidoscope's Source Segment picks one of the wedges Segments cut, so a knob
        /// running past the count is travel that picks the last wedge over and over. Editing
        /// the named control writes this one's range — the node's own `ControlRange`, the
        /// same document data the range editor writes — and pulls the value down with it.
        /// silvia does exactly this, from a listener that sets the `max` attribute.
        ///
        /// It follows the *control*, so a cable into the named input leaves the range where
        /// it was rather than tracking a number the node no longer reads.
        capped_by: Option<&'static str>,
    },
    /// Stored as `#rrggbbaa`; converted to a `vec4` at the uniform boundary.
    Color { default: &'static str },
    /// A momentary button, drawn in the input's row. The fallback for an **action** input:
    /// what it falls back to when nothing is connected is a hand.
    ///
    /// It stores no `ControlValue` — a press is not a value, it is a level the UI reports
    /// for one frame and `tick` turns into edges. That is also why it does not go inert when
    /// something is connected: an action input takes many sources and the hand is one more.
    Press,
}

impl Control {
    /// A number control. Most take this form, so it is worth being short.
    pub const fn num(default: f32, min: f32, max: f32, step: f32, unit: &'static str) -> Self {
        Self::Number {
            default,
            min,
            max,
            step,
            unit,
            log: false,
            capped_by: None,
        }
    }

    /// A number control whose top end follows another input's value. See
    /// [`Control::Number::capped_by`].
    pub const fn num_capped(
        default: f32,
        min: f32,
        max: f32,
        step: f32,
        unit: &'static str,
        capped_by: &'static str,
    ) -> Self {
        Self::Number {
            default,
            min,
            max,
            step,
            unit,
            log: false,
            capped_by: Some(capped_by),
        }
    }

    /// A number control scrubbed in log space, for ranges spanning decades.
    pub const fn num_log(default: f32, min: f32, max: f32, step: f32, unit: &'static str) -> Self {
        Self::Number {
            default,
            min,
            max,
            step,
            unit,
            log: true,
            capped_by: None,
        }
    }

    pub const fn color(default: &'static str) -> Self {
        Self::Color { default }
    }
}

pub use timing::{TIME, TIME_Y, Timing, is_time, is_time_row};

/// The unit every angle knob is read in: **turns**, so 1 is one full turn and a gear's Phase
/// cabled into a Rotation turns it once a cycle with no snap. A glyph rather than the word,
/// so it fits beside three decimals in silvia's 100-point number control as her `π` did.
pub const TURNS: &str = "↻";

/// Which group of the Nodes menu a definition belongs to.
///
/// A field on the definition rather than a list of slugs kept beside the menu. silvia keeps
/// that list, and it is the one thing in its registry that a new node has to be added to by
/// hand in a second place. Here the menu stays a pure registry query, and a node that
/// declares no category is a compile error rather than a node missing from the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Brings something in from outside the graph: a device, a file, a previous frame.
    Source,
    /// Makes a picture from nothing but `uv`.
    Generate,
    /// Builds, blends or maps color, one texel at a time. What comes out at a fragment is
    /// a function of what went in at that fragment, and of nothing else.
    Color,
    /// Reads one number per pixel out of a picture.
    Convert,
    /// Carries numbers across the boundary between the picture and the CPU: out of a
    /// picture where it flows, as a side effect and one frame late, or back into one as
    /// digits drawn on it.
    Tap,
    /// Moves the sampling coordinate.
    Transform,
    /// Changes a picture by reading it somewhere other than under the fragment: a
    /// neighborhood of texels, or a cell the fragment sits inside.
    Effect,
    /// Arithmetic on numbers.
    Math,
    /// Values that change over time on the CPU.
    Control,
    /// The clocks: the one place a rate is set, changed or divided, and ambient time as a
    /// number.
    Gear,
    /// Terminates a shader.
    Output,
}

impl Category {
    /// Menu order, which is roughly signal order: what comes in, what makes a picture, what
    /// changes it, what measures it, and what it ends in.
    pub const ALL: &'static [Self] = &[
        Self::Source,
        Self::Generate,
        Self::Color,
        Self::Transform,
        Self::Effect,
        Self::Convert,
        Self::Tap,
        Self::Math,
        Self::Control,
        Self::Gear,
        Self::Output,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Generate => "Generate",
            Self::Color => "Color",
            Self::Convert => "Convert",
            Self::Tap => "Tap",
            Self::Transform => "Transform",
            Self::Effect => "Effect",
            Self::Math => "Math",
            Self::Control => "Control",
            Self::Gear => "Gears",
            Self::Output => "Output",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Source => "📥",
            Self::Generate => "🎨",
            Self::Color => "🌈",
            // silvia's mark for the same heading: `Convert` kept its name across the fold
            // that renamed and split `Effects`, so it keeps silvia's icon too.
            Self::Convert => "🔀",
            Self::Tap => "🩺",
            Self::Transform => "🔄",
            Self::Effect => "✨",
            // silvia's mark for the same heading, restored for the same reason `Convert`'s
            // is: the name did not change, so neither should the mark a silvia hand scans
            // the menu for.
            Self::Math => "🧮",
            Self::Control => "🎛",
            Self::Gear => "⚙",
            Self::Output => "📺",
        }
    }
}

/// How an output's value reaches the shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    /// A WGSL function, inlined into the consumer's module.
    Shader,
    /// A texture, sampled through `u_texture_{slug}{id}_{key}`. An Output publishes its
    /// frame this way; a CPU node publishes what it captured or decoded.
    Texture,
    /// One value per frame, published by the node's CPU half from `tick` and reaching a
    /// consumer as the uniform `u_float_{slug}{id}_{key}`. The port type is a uniform one;
    /// the generator is never called.
    Uniform,
    /// An event, fired by the node's CPU half from `tick`. The port type is `Action`; it
    /// never reaches the compiler, never becomes a uniform, and the generator is never
    /// called.
    Action,
}

/// How a texture output is sampled outside its own bounds.
///
/// `Mirror` is the rule for every texture in supersilvia — see
/// [docs/rendering.md](../../docs/rendering.md#texture-wrapping). `Repeat` is the declared
/// exception: a simulation whose world is a torus publishes a picture of that world, and a
/// mirrored tiling would draw a seam the simulation does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureWrap {
    /// A sample past `[0,1]` reflects the picture back in.
    Mirror,
    /// A sample past `[0,1]` tiles the picture.
    Repeat,
}

/// How a texture output is filtered between its own texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureFilter {
    /// Blended, which is what a photograph, a video frame and a scent field want.
    Linear,
    /// The texel, unblended. For a picture whose texels are the thing itself — a grid of
    /// cells is not a smooth field and must not be drawn as one.
    Nearest,
}

pub struct InputDef {
    pub key: &'static str,
    pub label: &'static str,
    pub ty: PortType,
    pub control: Control,
}

pub struct OutputDef {
    pub key: &'static str,
    pub label: &'static str,
    pub ty: PortType,
    pub kind: OutputKind,
    /// The function's body, in WGSL; see [docs/nodes.md](../../docs/nodes.md#wgsl) for the
    /// conventions. [`no_wgsl`] for an output that never reaches the compiler.
    pub wgsl: GenFn,
    /// What this output's maths bounds it to, drawn on the row as `[0, 1]` where a
    /// `VaryingColor` or `VaryingNumber` port has no live value to print instead — a
    /// `UniformNumber` output keeps its own readout in that slot, and never carries a
    /// declared range.
    pub range: Option<&'static str>,
    /// True for a `UniformNumber` output whose value is last frame's by construction: a
    /// tap's, a sample's and an `autoexposure`'s, whose CPU half reads a measurement back
    /// the frame after the shader made it. `port_defs` turns this into a delayed `PortDef`,
    /// beside the one it already makes of every `Texture` output, so `Graph::can_connect`
    /// treats a cable out of one as feedback rather than an immediate cycle.
    pub delayed: bool,
    /// How the texture this output publishes reads outside `[0,1]`, and how it filters
    /// between its texels. Every texture is `Mirror` and `Linear` — the rule, and
    /// `OutputDef::EMPTY`'s answer — but a `Texture` output may declare otherwise, and the
    /// app copies both into the frame job so `render/` applies them without reading the
    /// registry. A registry test holds either off its default to a `Texture` output, since a
    /// `Shader` output has no texture to parametrize.
    pub wrap: TextureWrap,
    pub filter: TextureFilter,
    /// The WGSL body above, in Rust, for an output whose type is decided by what feeds it.
    ///
    /// An output with one is **dual**: `ty` is `VaryingNumber` in the definition, and on an
    /// instance whose every input resolves to a uniform number the effective type is
    /// `UniformNumber` — `Graph` writes that into the instance's `PortDef`, `tick` evaluates
    /// this, and the compiler resolves the port to a uniform and emits no function.
    /// `None` for every output whose value genuinely depends on `uv`. A registry test holds
    /// one to a `VaryingNumber` output of a node with no `cpu` half whose inputs are all
    /// `VaryingNumber` with number controls.
    pub eval: Option<EvalFn>,
    /// True for a `UniformNumber` output that counts rather than measures, drawn on its row
    /// with no decimal places. The readout's two fixed places are right for a number that
    /// moves continuously and wrong for one that only ever lands on whole numbers:
    /// `muxevent`'s channel is Input 1 through Input 4, and `1.00` beside the row labelled
    /// Input 1 reads as a measurement of something.
    pub integral: bool,
    /// Where a `UniformNumber` output that is a clock wraps as one `f32`: 1 for a phase
    /// published 0 up to 1 — the two gears' Phase — and [`phasor::WRAP`] for everything else,
    /// a count's one `f32` published −1260 up to 1260. A clock cabled into a sequencer's Time
    /// is unwrapped at half of it, so a 0..1 Phase runs forward across its wrap, and a Ratio
    /// Gear's Clock In there to find the whole cycles a frame passed; a count published whole
    /// is read unwrapped and never wraps (`TickContext::wraps_at`).
    pub wraps_at: f64,
}

impl OutputDef {
    /// The fields an output rarely says anything about, so a definition writes only the ones
    /// it means: `OutputDef { key, label, ty, kind, wgsl, ..OutputDef::EMPTY }`, the way
    /// `OptionDef::EMPTY` and `NodeDef::EMPTY` are already used.
    pub const EMPTY: Self = Self {
        key: "",
        label: "",
        ty: PortType::VaryingColor,
        kind: OutputKind::Shader,
        wgsl: no_wgsl,
        range: None,
        delayed: false,
        wrap: TextureWrap::Mirror,
        filter: TextureFilter::Linear,
        eval: None,
        integral: false,
        wraps_at: phasor::WRAP,
    };
}

/// What an option costs to change, which is the only thing that separates the three.
///
/// The recompile boundary in `docs/architecture.md` is the whole argument: changing what the
/// graph *is* costs a shader rebuild and changing what it *does* costs a float. An option
/// that does not change the emitted WGSL has no business paying for a rebuild, and two kinds
/// of option do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    /// A choice the generator reads with `CompileContext::option`, so it changes the emitted
    /// WGSL and a change rebuilds the shader. Every option is this unless it says otherwise.
    Code,
    /// A choice the shader branches on at run time: the index of the chosen value among
    /// `choices`, uploaded as an `i32` uniform. A change costs one integer in the tick's uniform block.
    ///
    /// The cost is paid the other way, so this is not the default: every branch is in the
    /// compiled program whether or not it runs, and the driver allocates registers for the
    /// worst of them. It is right where a rebuild is the thing that must not happen — a
    /// crossfade method changed mid-performance — and wrong for a choice that changes the
    /// shape of the code rather than one term in it.
    Uniform,
    /// A path to a file in the project. It changes which texture is bound and no WGSL at
    /// all, so a change rebuilds nothing. Carries no `choices`; `accepts` says what it takes.
    Asset,
    /// A choice only the canvas reads, which reaches no shader at all — how much of an audio
    /// node to draw. The same answer `SetCollapsed` already gives: the ports and the cables
    /// are untouched, so the shader that was running still describes the graph.
    Presentation,
    /// A choice read while running rather than while compiling: by the node's `tick` — a
    /// camera's device, a clip's loop, a clock's division — or by the renderer, which reads
    /// an Output's resolution into every frame's job and resizes from that.
    ///
    /// It changes what the node or its targets *do*, and a `tick` reads the option fresh, so
    /// the change lands on the next frame with no shader involved. A node with only
    /// `UniformNumber` and `Action` outputs emits no WGSL at all, which puts every option it
    /// has here.
    Runtime,
}

/// How a value is drawn, and so what it holds.
///
/// This list is meant to grow. An [`OptionDef`] has four shapes and always will, because one
/// piece of shared code draws them all. A value is drawn by the node's own code, so a new
/// entry arrives here every time a node wants a hand on something new — a curve, a paint
/// surface, and later a pad and a step grid.
///
/// One rule holds: a kind may not make `ui/` special-case a particular node. The kind is what
/// `ui/` draws from, and that is what keeps node-specific drawing out of a module every node
/// shares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueKind {
    /// A box of free-running text, `rows` lines tall. A note's, and the words a `text` node
    /// sets. `default` is what a new node starts holding, empty where the placeholder is the
    /// whole of what a blank one should say.
    Text {
        rows: u8,
        placeholder: &'static str,
        default: &'static str,
    },
    /// A curve a hand performed, drawn in one of the node's own **regions** rather than in a
    /// row: `automation`'s recording. `min` and `max` are the ends the curve is drawn
    /// against, which is what the region reads to place a point on its band.
    ///
    /// The one kind so far that takes no row. A curve wants the width of the body and a band
    /// of its own, which is what a region is; the value is still the node's own state, saved
    /// and undone exactly as a note's prose is.
    Points { min: f32, max: f32 },
    /// A grid of cells a hand turns on and off, `lanes` rows of `steps`, drawn in one of the
    /// node's own regions: `stepsequencer`'s pattern. Takes no row, as a curve takes none. A
    /// new node holds no value here, which is every cell unlit.
    Cells { lanes: u8, steps: u8 },
    /// A picture a hand paints, drawn — and painted on — in one of the node's own regions:
    /// `drawingcanvas`'s. Held as a [`crate::graph::Painting`], and written into the project
    /// folder as a picture file of its own rather than into the `.ssw`.
    Painting,
}

/// One piece of state a node declares for itself: not a port's value, and not a choice out of
/// a list.
///
/// See [`crate::graph::Value`] for what separates this from an option.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueDef {
    pub key: &'static str,
    /// Drawn beside the control, or empty where the control is the whole row — a note is a
    /// box of text and a label saying "Text" above it would be a label saying nothing.
    pub label: &'static str,
    pub kind: ValueKind,
}

impl ValueDef {
    /// How many rows of the node's body this value takes before its field has drawn, which is
    /// what `canvas` sizes it from until the field says how tall its text came out.
    ///
    /// **Zero for a value a region draws**, which takes no row at all: the band it is drawn
    /// in is declared in `NodeDef::regions` and sized there.
    pub const fn rows(&self) -> u8 {
        match self.kind {
            ValueKind::Text { rows, .. } => rows,
            ValueKind::Points { .. } | ValueKind::Cells { .. } | ValueKind::Painting => 0,
        }
    }

    /// What a new node holds here before anybody types. Empty for almost every value, which
    /// is a box showing its placeholder.
    pub const fn default(&self) -> &'static str {
        match self.kind {
            ValueKind::Text { default, .. } => default,
            // A curve starts with no points; the default is `Value::Points(Vec::new())`,
            // built at the call site since it is not `&'static str`.
            ValueKind::Points { .. } | ValueKind::Cells { .. } | ValueKind::Painting => "",
        }
    }
}

/// The heading over a source's own picture, for the node kinds that declare a
/// [`Region::Preview`] to draw in.
///
/// `Presentation`: what it changes is how tall the node is, and nothing downstream of it.
pub const SHOW_PREVIEW: OptionDef =
    OptionDef::heading("preview", "Preview", true, OptionKind::Presentation);

/// The heading over a trace, and over Automation's recorded curve, which is the same band:
/// open on a new node, since the shape is what a person reads first.
pub const SHOW_TRACE: OptionDef =
    OptionDef::heading("trace", "Trace", true, OptionKind::Presentation);

/// A checkbox option's two values. Words rather than `true`/`false` so the file reads as
/// every other option does, and so a third state could be added to one without the file
/// format having to change what it means.
pub const ON: &str = "on";
pub const OFF: &str = "off";

/// The choices every checkbox declares, so `option_is_valid` holds a tick to its two values
/// exactly as it holds a select to its list.
pub const CHECK_CHOICES: &[(&str, &str)] = &[(ON, "On"), (OFF, "Off")];

/// An option's menu: `(value, display name)`, in order.
pub type Choices = &'static [(&'static str, &'static str)];

/// A number typed as text: what a plain number field admits, and what it commits.
///
/// The field has none of a number control's hands — no scrub, no steppers, no fill — so this
/// is all there is to say about it: its ends, whether it holds only whole numbers, and the
/// unit it is read in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberField {
    pub min: f32,
    pub max: f32,
    pub integer: bool,
    /// Drawn muted after the number, inside the field; empty for none. Never part of the
    /// stored text.
    pub unit: &'static str,
}

impl NumberField {
    /// Any finite number, fractions included: the range editor's ends.
    pub const ANY: Self = Self {
        min: f32::MIN,
        max: f32::MAX,
        integer: false,
        unit: "",
    };

    /// Whether `c` can be typed into the field: a digit, a minus where the field reaches
    /// below zero, and a point where it takes fractions.
    pub fn admits(&self, c: char) -> bool {
        c.is_ascii_digit() || (c == '-' && self.min < 0.0) || (c == '.' && !self.integer)
    }

    /// What the field commits for `text`: the number, rounded where the field is whole and
    /// clamped to its ends. `None` for anything that is not a number.
    pub fn read(&self, text: &str) -> Option<f32> {
        let v = text.trim().parse::<f32>().ok().filter(|v| v.is_finite())?;
        let v = if self.integer { v.round() } else { v };
        Some(v.clamp(self.min, self.max))
    }

    /// Whether `text` is a number the field would commit unchanged.
    pub fn holds(&self, text: &str) -> bool {
        text.trim()
            .parse::<f32>()
            .is_ok_and(|v| self.read(text) == Some(v))
    }

    /// `v` as the field writes it: no point on a whole number.
    pub fn written(&self, v: f32) -> String {
        if self.integer {
            format!("{}", v as i64)
        } else {
            format!("{v}")
        }
    }
}

#[allow(clippy::struct_excessive_bools)] // the shapes an option can take, each its own
pub struct OptionDef {
    pub key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    /// `(value, display name)`, in menu order. **Empty on an `Asset`**, which holds any path
    /// it is given and draws a file button rather than a select.
    pub choices: Choices,
    /// What this option costs to change. `Code` unless the node says otherwise.
    pub kind: OptionKind,
    /// What an `Asset` option's file is. It says two things at once, which is why it is one
    /// field: what a file dialog filters to, and which of the project's assets the button's
    /// picker offers. Meaningless on an option that has choices.
    pub accepts: Accepts,
    /// The key of an input that answers this option's question better than the option does.
    /// While something is connected to it, the generator reads the cable and not the choice,
    /// and the canvas draws the select disabled showing that input's label.
    ///
    /// Data rather than a special case, so `ui/` can draw the state without knowing which
    /// node it belongs to. A registry test holds the key to a real input of the same node.
    pub overridden_by: Option<&'static str>,
    /// The row draws a typed field rather than a closed list, with `choices` offered beside
    /// it as presets instead of the only values `option_is_valid` accepts. `Some` is the
    /// field's placeholder text; `None`, every option but one — Lyapunov's `sequence`, which
    /// takes any string its grammar parses rather than eight named ones. The same idea as an
    /// `Asset` holding any path, for a `Code` option instead of a file.
    pub placeholder: Option<&'static str>,
    /// What a free-text option's typed value has to satisfy to be the one compiled in,
    /// rather than the row showing text the node has quietly fallen back from. Meaningless
    /// without `placeholder`; read by `ui/` for the row's border, not enforced here — a
    /// value that fails it is still stored, because the generator it feeds already answers
    /// for anything unparseable on its own.
    pub validate: Option<fn(&str) -> bool>,
    /// The row draws a plain number field: typed like text, taking only what a number is
    /// written with, committed on Enter or when the field is left, and clamped into its
    /// bounds. The Text node's `size`. Stored as its text, as every option is.
    pub number: Option<NumberField>,
    /// Drawn as a tick, sharing one row with every other checkbox the node declares —
    /// silvia's own `Numbers` / `Events` / `Scope` row at the foot of an audio node.
    ///
    /// Only the shape of the row changes: the value is [`ON`] or [`OFF`], which is an
    /// ordinary option value, so it is saved, undoable, validated and addressed exactly as a
    /// select's is. Three questions that are each yes or no were one three-way select here
    /// until the answers stopped being mutually exclusive — see
    /// docs/decisions.md#the-scope-a-two-axis-handle-and-a-row-of-ticks-for-the-rest.
    pub checkbox: bool,
    /// Drawn as the disclosure triangle on the heading of the region it governs, rather than
    /// as a row of the node.
    ///
    /// The storage is a checkbox's exactly — [`ON`] or [`OFF`] in `Node::options`, saved,
    /// undoable and validated the same way — and the region that carries it wears a
    /// [`Heading`] built from this option, which a registry test holds the node to declaring.
    /// Only the affordance differs: a tick in a shared row cannot say *this part of the node is
    /// here and closed*, and a heading is sixteen points that do.
    pub heading: bool,
    /// Drawn by one of the node's own regions rather than as a row: `drawingcanvas`'s tool,
    /// which is a row of buttons under the paint surface rather than a select above it.
    ///
    /// Stored, saved, undone and validated exactly as a select's value is — the region writes
    /// it through `RegionEvent::Option` — and only the row is missing, because the region is
    /// the control. A registry test holds such an option to a node that has a region.
    pub in_region: bool,
    /// Choices the machine has rather than ones the node declares — the Text node's fonts.
    /// While it returns any, the menu is this list and `choices` is only the fallback for a
    /// machine that could not say, and the default's home.
    ///
    /// A value off the list is still valid: a project carries a family from the machine it
    /// was made on, and one this machine does not have is the fallback it would draw, not a
    /// broken file.
    pub found: Option<fn() -> Choices>,
    /// The menu lists the machine's capture devices, so it ends in *Look for devices again*,
    /// which asks the machine for its list again: a camera plugged in after the app started
    /// is not in it until then.
    pub devices: bool,
    /// Drawn as a row of segments, one per choice, at the right end of the bar of the row
    /// heading this names, rather than as a row of its own: [`timing::MODE`] on the Time
    /// heading, where it shows whether the heading is open or closed. Stored, saved, undone
    /// and validated exactly as a select's value is.
    pub on_heading: Option<&'static str>,
}

impl OptionDef {
    pub const EMPTY: Self = Self {
        key: "",
        label: "",
        default: "",
        choices: &[],
        kind: OptionKind::Code,
        accepts: Accepts::ANY,
        overridden_by: None,
        placeholder: None,
        validate: None,
        number: None,
        checkbox: false,
        heading: false,
        in_region: false,
        found: None,
        devices: false,
        on_heading: None,
    };

    /// A checkbox option: a tick that is on or off, in the row the node's other ticks share.
    ///
    /// `Presentation` for every one there is so far — what a hand hides is how much of the
    /// node is drawn, never what it computes — but the kind is the caller's, because nothing
    /// about a tick says a node could not have one that rebuilds.
    pub const fn check(
        key: &'static str,
        label: &'static str,
        default_on: bool,
        kind: OptionKind,
    ) -> Self {
        Self {
            key,
            label,
            default: if default_on { ON } else { OFF },
            choices: CHECK_CHOICES,
            kind,
            checkbox: true,
            heading: false,
            ..Self::EMPTY
        }
    }

    /// A region's heading: the same on/off option a tick is, worn as a disclosure triangle on
    /// the region it opens and closes.
    pub const fn heading(
        key: &'static str,
        label: &'static str,
        default_on: bool,
        kind: OptionKind,
    ) -> Self {
        Self {
            heading: true,
            checkbox: false,
            ..Self::check(key, label, default_on, kind)
        }
    }

    /// What the select offers: what the machine has, where the option asks it, or else its
    /// declared choices.
    pub fn menu(&self) -> Choices {
        self.found
            .map(|found| found())
            .filter(|found| !found.is_empty())
            .unwrap_or(self.choices)
    }

    /// An asset reference holds whatever string it is given — a path into the project.
    pub fn is_asset(&self) -> bool {
        self.kind == OptionKind::Asset
    }

    /// Whether changing this option's value changes the WGSL, and so needs a rebuild.
    ///
    /// The one question `SetOption` asks. `Uniform` and `Asset` both answer no, for
    /// different reasons: one is a uniform the shader already reads, the other is a texture
    /// the renderer already binds.
    pub fn rebuilds(&self) -> bool {
        self.kind == OptionKind::Code
    }

    /// Where `value` sits among `choices`, which is what a `Uniform` option uploads.
    ///
    /// A value that is not a choice reads as the default's index, and a default that is not
    /// one either reads as zero — a registry test holds every `Uniform` option to having its
    /// default among its choices, so neither fallback is reachable from a well-formed
    /// registry. They exist because a project file is not one: an option value saved by a
    /// build that had a tenth choice must land somewhere rather than index past the branch.
    pub fn index_of(&self, value: &str) -> i32 {
        let at = |v: &str| self.choices.iter().position(|(c, _)| *c == v);
        at(value).or_else(|| at(self.default)).unwrap_or(0) as i32
    }
}

/// What kind of file an option holds a path to.
///
/// One value rather than a filter here and a predicate there, because the dialog and the
/// asset picker are two routes to the same answer and must not disagree about what the node
/// can play: a file the dialog would not offer is one the picker must not either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accepts {
    /// What the dialog calls this filter — "Video".
    pub label: &'static str,
    /// Lowercase, without the dot. **Empty is anything**, which is what an option that is
    /// not a file at all carries.
    pub extensions: &'static [&'static str],
}

impl Accepts {
    pub const ANY: Self = Self {
        label: "Any file",
        extensions: &[],
    };

    /// What an `imagegif` node will show. Everything `image` decodes with its default
    /// features that a performer would call a picture: the four still formats and the one
    /// that moves.
    pub const IMAGE: Self = Self {
        label: "Image",
        extensions: &["png", "jpg", "jpeg", "gif", "webp"],
    };

    /// What a `video` node will play. Everything here reaches the cache through one
    /// transcode, so the list is what the transcoder accepts rather than what a container
    /// could in principle hold. **No picture is here**, an animated GIF included: the
    /// transcode has no clip to make of one, and [`IMAGE`](Self::IMAGE)'s node plays it.
    pub const VIDEO: Self = Self {
        label: "Video",
        extensions: &["mp4", "mov", "mkv", "webm", "avi", "m4v", "mts", "ts"],
    };

    /// What the Main Input panel will play as a sound. Decoded by GStreamer into the
    /// project's cache the same way a clip's own soundtrack is, so the list is what
    /// `decodebin` handles rather than what an analyzer could in principle read.
    pub const AUDIO: Self = Self {
        label: "Audio",
        extensions: &[
            "wav", "flac", "mp3", "ogg", "opus", "m4a", "aac", "aiff", "wma",
        ],
    };

    /// Does this file name look like one of these? Case-insensitively, because a `.MP4` off
    /// a camera is an mp4.
    pub fn matches(&self, name: &str) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        let Some((_, extension)) = name.rsplit_once('.') else {
            return false;
        };
        let extension = extension.to_ascii_lowercase();
        self.extensions.contains(&extension.as_str())
    }
}

pub struct NodeDef {
    pub slug: &'static str,
    pub icon: &'static str,
    pub label: &'static str,
    pub tooltip: &'static str,
    pub inputs: &'static [InputDef],
    /// Controls with **no port and no row**: stored on the node, saved with the workspace, and
    /// edited somewhere other than the node body.
    ///
    /// A band's center frequency and Q are the case this exists for. They are document data —
    /// tuning a band to a track has to survive a save — and they go through `SetControl` like
    /// anything else, so undo has them. What they are not is six more number rows: their
    /// editor is the two-dimensional handle on the node's own scope, where one drag says both
    /// at once and the spectrum underneath says what it did.
    pub hidden: &'static [InputDef],
    /// Controls MIDI does not bind, though the map could address them: an Output's render
    /// numbers, which silvia marks `midi-disabled` because nobody turns a frame rate mid-set.
    /// Every other control — a port's, or a hidden one a region draws — is bindable. See
    /// [`NodeDef::bindable`].
    pub unbindable: &'static [&'static str],
    pub outputs: &'static [OutputDef],
    pub options: &'static [OptionDef],
    /// State this node keeps for itself, drawn by whatever `ValueKind` each one declares.
    /// Empty for every node but `note` so far.
    pub values: &'static [ValueDef],
    /// WGSL helpers this node's generators call, emitted once however many nodes ask. WGSL has
    /// no overloading, so a helper's name is unique across every node's, which a test over the
    /// registry holds.
    pub wgsl_utils: &'static [&'static str],
    /// True for nodes that own a renderer and terminate a shader.
    pub is_output: bool,
    /// What this node draws below its rows, in draw order: an audio scope, a trace, a picture
    /// of what it publishes, an Output's own render — and a pad, a grid or an XY pad in time.
    ///
    /// The node's own area, silvia's `customArea` with one function added: a region reports its
    /// height without drawing, because layout runs for every node in the graph before anything
    /// is painted. Each entry names a region rather than holding it, so the registry says what
    /// it wants drawn without naming the toolkit — `widgets/` keys its drawing on the name.
    /// The escape `has_scope`, `trace` and `preview` each were, now one list a node writes
    /// itself. See [`area`].
    pub regions: &'static [Region],
    /// A wider body than `canvas::NODE_WIDTH`, for a node whose longest *row* does not fit it.
    ///
    /// Rows are not regions, so this stays the body's own property and is honest about being
    /// about rows: a region that needs a wider body declares its own `width`. It is still a
    /// short, tested list rather than per-frame text measurement, and `None` for every node
    /// whose rows fit the default.
    pub width: Option<f32>,
    /// A hand may drag this kind's body wider, and the width it drags is saved with the
    /// patch. False for every node whose width is the rows' own answer, which is every node
    /// but the note: what a node is wide enough for is a fact about its rows, and only a box
    /// of prose has a width a person wants an opinion about. silvia's is the same one node.
    pub resizable: bool,
    /// Which group of the Nodes menu this belongs to.
    pub category: Category,
    /// How this node keeps time, for a node that moves with it: its pace, whether a new one
    /// stands still, its period and its axes, from which its time rows, its Timing heading and its mode are
    /// expanded. `None` for every node that does not move with time. See [`timing`].
    pub timing: Option<Timing>,
    /// The CPU half, for a node that computes something outside a shader: a uniform number
    /// per frame, or a captured texture. Created per instance on first tick.
    pub cpu: Option<CpuDef>,
    /// The measurement this node makes, for a node that turns a picture into numbers.
    ///
    /// Called once per node per shader, by the first of its outputs the compiler emits. It
    /// claims a tap slot, evaluates the node's inputs at the point it is handed through
    /// `ctx.input(node, key, "p")`, and registers itself with
    /// [`CompileContext::measure_grid`] or [`CompileContext::measure_once`] — so what it
    /// measures is the node's input over the unit square, and not whatever coordinates a
    /// consumer asked the node's own function at. A registry test holds a node with one to
    /// having a `cpu` half, which is what reads the slot back.
    pub measure_wgsl: Option<fn(NodeId, &mut CompileContext)>,
    /// The keys of the heading options whose triangles fold a run of the node's own **rows**
    /// rather than a region.
    ///
    /// A region carries its heading on itself, because a region is one band and the heading
    /// sits over it. A run of rows is not a band and has no such place to hang one, so the
    /// node names the option here and `ui/` draws the same bar over the rows it governs. The
    /// Output's Render and Send are the ones there are. A registry test holds each to naming
    /// a heading option of this node, exactly as it holds a region's heading to one.
    pub row_headings: &'static [&'static str],
    /// Whether the library offers this kind on this machine: the Nodes menu and the browser
    /// list only what answers true. A kind the machine cannot offer still loads from a file,
    /// so a project made elsewhere opens whole. `syphon` is the one that answers false, on
    /// Linux.
    pub offered: fn() -> bool,
}

impl NodeDef {
    pub const EMPTY: Self = Self {
        slug: "",
        icon: "",
        label: "",
        tooltip: "",
        inputs: &[],
        hidden: &[],
        unbindable: &[],
        outputs: &[],
        options: &[],
        values: &[],
        wgsl_utils: &[],
        is_output: false,
        regions: &[],
        width: None,
        resizable: false,
        // Overridden by every shipping node, and held to it by a registry test.
        category: Category::Effect,
        timing: None,
        cpu: None,
        measure_wgsl: None,
        row_headings: &[],
        offered: || true,
    };

    /// How many of this kind's options are ticks sharing one row of their own — silvia's
    /// `Numbers` / `Events` / `Scope` row. Zero for almost every node.
    ///
    /// A count rather than the keys, because the count is the whole of the layout fact:
    /// `canvas::rows` has to know that several options share one row, and how many of them
    /// are therefore not select rows.
    pub fn checks(&self) -> usize {
        self.options.iter().filter(|o| o.checkbox).count()
    }

    /// How many of this kind's options are a region's heading rather than a row of their own.
    /// Zero for almost every node.
    pub fn headings(&self) -> usize {
        self.options.iter().filter(|o| o.heading).count()
    }

    /// How many of this kind's options one of its regions draws rather than a row. Zero for
    /// every node but `drawingcanvas`, whose tool is its brush's row of buttons.
    pub fn in_regions(&self) -> usize {
        self.options.iter().filter(|o| o.in_region).count()
    }

    /// How many of this kind's options are segments on a heading's bar rather than a row:
    /// [`timing::MODE`] on every node that moves with time, and none elsewhere.
    pub fn on_headings(&self) -> usize {
        self.options
            .iter()
            .filter(|o| o.on_heading.is_some())
            .count()
    }

    /// An input by key, port or not.
    ///
    /// Hidden controls are searched too, so the command bus, the file loader and the range
    /// helpers all treat them as the controls they are. Nothing that resolves a *port* can
    /// reach one: they are not in `port_defs`, so no cable and no compiled input names one.
    /// The value this node declares under `key`, if it declares one.
    pub fn value(&self, key: &str) -> Option<&ValueDef> {
        self.values.iter().find(|v| v.key == key)
    }

    pub fn input(&self, key: &str) -> Option<&InputDef> {
        self.inputs.iter().chain(self.hidden).find(|p| p.key == key)
    }

    /// The control under `key` where MIDI may bind it: an input, port or not, that this kind
    /// does not list in [`NodeDef::unbindable`]. The key comes back `'static`, which is what
    /// a binding read from a file needs to become a `PortRef`.
    pub fn bindable(&self, key: &str) -> Option<&'static str> {
        let key = self.input(key)?.key;
        (!self.unbindable.contains(&key)).then_some(key)
    }

    pub fn output(&self, key: &str) -> Option<&OutputDef> {
        self.outputs.iter().find(|p| p.key == key)
    }

    pub fn option(&self, key: &str) -> Option<&OptionDef> {
        self.options.iter().find(|o| o.key == key)
    }

    /// The subset `graph/` needs: port identity, type, whether an output is delayed, and
    /// whether its type is its own or decided by what feeds it.
    ///
    /// A `Texture` output publishes a finished frame, so consuming it reads the previous one,
    /// and a `UniformNumber` output with `OutputDef::delayed` set reads a measurement the
    /// shader made a frame ago. Both make a loop through them feedback rather than infinite
    /// recursion, which is one of the two bits of node semantics `graph/` has to know. The
    /// other is `OutputDef::eval`: an output carrying one is dual, so the graph recomputes its
    /// effective type rather than reading the declared one, and does it without matching on a
    /// slug. The `VaryingNumber` inputs of that same node are dual too — the graph pins them
    /// to `UniformNumber` on an instance whose flip would demote what reads its number — which
    /// `a_dual_output_is_a_varying_number_a_tick_can_evaluate` holds to being all of them.
    pub fn port_defs(&self) -> (Vec<PortDef>, Vec<PortDef>) {
        let dual = self.outputs.iter().any(|o| o.eval.is_some());
        (
            self.inputs
                .iter()
                .map(|p| {
                    if dual && p.ty == PortType::VaryingNumber {
                        PortDef::dual(p.key, p.ty)
                    } else {
                        PortDef::new(p.key, p.ty)
                    }
                })
                .collect(),
            self.outputs
                .iter()
                .map(|p| {
                    if p.kind == OutputKind::Texture || p.delayed {
                        PortDef::delayed(p.key, p.ty)
                    } else if p.eval.is_some() {
                        PortDef::dual(p.key, p.ty)
                    } else {
                        PortDef::new(p.key, p.ty)
                    }
                })
                .collect(),
        )
    }
}

/// A definition is named by its slug, which a registry test holds unique; the rest of it is
/// functions and tables that say nothing a `Node`'s printout needs.
impl std::fmt::Debug for NodeDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NodeDef({:?})", self.slug)
    }
}

/// Two definitions are the same kind when they have the same slug.
///
/// Not the address: a `const` definition is a fresh value at every place it is named, so two
/// references to the same kind need not point at the same bytes.
impl PartialEq for NodeDef {
    fn eq(&self, other: &Self) -> bool {
        self.slug == other.slug
    }
}

impl Eq for NodeDef {}

/// Every node kind the app knows about.
///
/// One list, so the invariant tests below cover every shipping node. A unit build appends
/// `test_support`; nothing removes anything.
pub static REGISTRY: &[&NodeDef] = &[
    &color::DEF,
    &worldcoordinates::DEF,
    &checkerboard::DEF,
    &shapes::CIRCLE,
    &shapes::POLYGON,
    &shapes::STAR,
    &patterns::STRIPES,
    &patterns::GRID,
    &patterns::POLKADOT,
    &patterns::HOUNDSTOOTH,
    &shapes::SPIRAL,
    &shapes::PHYLLOTAXIS,
    &patterns::PRIDEFLAG,
    &gradients::LINEARGRADIENT,
    &gradients::RADIALGRADIENT,
    &fractals::MANDELBROT,
    &fractals::JULIASET,
    &fractals::LYAPUNOV,
    &fractals::SIERPINSKI,
    &noise::PERLIN,
    &noise::SIMPLEX,
    &noise::WORLEY,
    &noise::FRACTAL,
    &noise::STATIC,
    &randomhurl::DEF,
    &cellularautomata::DEF,
    &slimemold::DEF,
    &brickgame::DEF,
    &mix::DEF,
    &mix::LAYERBLEND,
    &muxevent::DEF,
    &muxnumber::DEF,
    &note::DEF,
    &cosinegradient::COSINEGRADIENT,
    &adjust::CONTRAST,
    &adjust::GAMMA,
    &adjust::LEVELS,
    &adjust::INVERT,
    &adjust::POSTERIZE,
    &adjust::VIGNETTE,
    &adjust::SIMPLELIGHT,
    &saturate::SATURATE,
    &saturate::VIBRANCE,
    &wavefold::WAVEFOLD,
    &palette::PALETTE,
    &recolor::COLORIZE,
    &recolor::COLORMAPPING,
    &recolor::COLORSHIFT,
    &chromakey::DEF,
    &output::DEF,
    &rgba::DEF,
    &hsla::DEF,
    &edgedetection::DEF,
    &chromaticaberration::DEF,
    &convolve::BLUR,
    &convolve::SHARPEN,
    &convolve::EMBOSS,
    &convolve::BLOOM,
    &convolve::DILATE,
    &convolve::ERODE,
    &convolve::HEIGHTTONORMAL,
    &screentone::HALFTONE,
    &screentone::MOSAIC,
    &screentone::DITHER,
    &multisample::KUWAHARA,
    &multisample::MOTIONBLUR,
    &multisample::RADIALBLUR,
    &multisample::SINCFILTER,
    &multisample::SUPERSAMPLING,
    &pixelsort::PIXELSORT,
    &stargate::DEF,
    &glitch::DEF,
    &camcordercrt::DEF,
    &decompose::LUMINOSITY,
    &decompose::LIGHTNESS,
    &decompose::VALUE,
    &decompose::AVERAGE,
    &decompose::HUE,
    &decompose::SATURATION,
    &decompose::CHROMA,
    &decompose::RED,
    &decompose::GREEN,
    &decompose::BLUE,
    &decompose::ALPHA,
    &decompose::CHANNELSPLITTER,
    &sliderule::SLIDERULE,
    &transform::ZOOM,
    &transform::ROTATE,
    &transform::FISHEYE,
    &transform::TRANSLATE,
    &transform::MIRROR,
    &transform::STRETCHSKEW,
    &transform::PERSPECTIVE,
    &transform::POLARCOORDS,
    &transform::ROTOZOOM,
    &transform::SHAKYCAM,
    &distort::WAVE,
    &distort::WHIRLANDPINCH,
    &distort::KALEIDOSCOPE,
    &distort::TILE,
    &distort::REPEATER,
    &distort::DOMAINWARP,
    &region::REGIONABSOLUTE,
    &region::REGIONSIZED,
    &distort::SCATTER,
    &distort::TUNNEL3D,
    &wallpaper::WALLPAPER,
    &geissflow::DEF,
    &math::ADD,
    &math::SUBTRACT,
    &math::MULTIPLY,
    &math::DIVIDE,
    &math::MIN,
    &math::MAX,
    &math::SINE,
    &math::COSINE,
    &math::ABS,
    &math::CEIL,
    &math::FLOOR,
    &math::ATAN2,
    &math::LERP,
    &math::MODULO,
    &math::POWER,
    &math::PYTHAGOREAN,
    &math::SMOOTHSTEP,
    &math::THRESHOLD,
    &random::DEF,
    &reframerange::REFRAMERANGE,
    &number::DEF,
    &slew::DEF,
    &oscillator::DEF,
    &animation::DEF,
    &button::DEF,
    &clockdivider::DEF,
    &counter::DEF,
    &adsr::DEF,
    &automation::DEF,
    &clock::DEF,
    &smoothcounter::DEF,
    &triggeredrandom::DEF,
    &triggeredcolor::DEF,
    &randomfire::DEF,
    &euclideanrhythm::DEF,
    &stepsequencer::DEF,
    &xypad::DEF,
    &gear::MASTER,
    &gear::RATIO,
    &time::DEF,
    &tap::DEF,
    &sample::DEF,
    &autogain::DEF,
    &autoexposure::DEF,
    &audioin::DEF,
    &camera::DEF,
    &gamepad::DEF,
    &mouseinput::DEF,
    &screencapture::DEF,
    &syphon::DEF,
    &ndi::DEF,
    &maininput::DEF,
    &video::DEF,
    &imagegif::DEF,
    &drawingcanvas::DEF,
    &text::DEF,
    #[cfg(test)]
    &test_support::DEF,
];

/// The definition a slug names: a string off disk or out of the library, into the kind a
/// `Node` holds. A scan of the registry, so it is for the edges where a slug arrives — a file
/// loading, a node being added — and never for a node already in the graph, which carries its
/// definition in `Node::def`.
pub fn find(slug: &str) -> Option<&'static NodeDef> {
    REGISTRY.iter().copied().find(|d| d.slug == slug)
}

/// The label of the kind a slug names, or the slug where none does: what a command carrying a
/// slug — an add, a conversion — is called in the Edit menu and the Undo History.
pub fn label_of(slug: &'static str) -> &'static str {
    find(slug).map_or(slug, |d| d.label)
}

/// Insert a node of the named kind on the graph's default workspace.
pub fn add_to_graph(graph: &mut Graph, slug: &str, pos: Pos2) -> Option<NodeId> {
    let workspace = graph.default_workspace();
    add_to_graph_on(graph, slug, pos, workspace)
}

/// Insert a node of the named kind on one workspace, taking its ports, control defaults and
/// option defaults from the definition.
pub fn add_to_graph_on(
    graph: &mut Graph,
    slug: &str,
    pos: Pos2,
    workspace: WorkspaceId,
) -> Option<NodeId> {
    let def = find(slug)?;
    let (inputs, outputs) = def.port_defs();
    let id = graph.add_node(
        def,
        pos,
        inputs,
        outputs,
        std::collections::BTreeSet::from([workspace]),
    );
    apply_defaults(graph, id);
    Some(id)
}

/// Seed a node's controls, options and values from its definition.
///
/// Separate from `add_to_graph` because loading a file needs it too: a file written before a
/// control existed should open with that control at its default rather than absent.
pub fn apply_defaults(graph: &mut Graph, id: NodeId) {
    reset_controls(graph, id);
    let Some(node) = graph.get_mut(id) else {
        return;
    };
    let def = node.def;
    // A value the definition starts full — the words a `text` node says before anybody types
    // — is stamped here beside the options, so it is ordinary document data from the first
    // frame: saved, undoable, and editable to nothing.
    for value in def.values {
        if !value.default().is_empty() {
            node.values.insert(
                value.key,
                crate::graph::Value::Text(value.default().to_string()),
            );
        }
    }
    for option in def.options {
        node.options.insert(option.key, option.default.to_string());
    }
    // An unconnected input's control is half of what decides a dual output's effective type,
    // and the controls only exist now. So a node with two knobs and nothing plugged in is a
    // diamond on the first frame it is drawn, rather than on the first frame something is
    // connected to it.
    graph.recompute_effective_types(&[id]);
}

/// Seed a node's controls from its definition, leaving its options alone.
///
/// The half of `apply_defaults` that `Command::ResetControls` wants: an option is structural
/// and rebuilds a shader, so a reset of the parameters may not carry it.
pub fn reset_controls(graph: &mut Graph, id: NodeId) {
    let Some(node) = graph.get_mut(id) else {
        return;
    };
    let def = node.def;
    for input in def.inputs.iter().chain(def.hidden) {
        match &input.control {
            Control::Number { default, .. } => {
                node.controls
                    .insert(input.key, ControlValue::Float(*default));
            }
            Control::Color { default } => {
                node.controls
                    .insert(input.key, ControlValue::Color(parse_hex_rgba(default)));
            }
            Control::None | Control::Press => {}
        }
    }
}

/// What a number control's range is on a definition, before any instance changed it.
///
/// `None` for an input that is not a number: a color has no ends and a press has no value.
pub fn declared_range(def: &NodeDef, key: &str) -> Option<ControlRange> {
    match def.input(key)?.control {
        Control::Number { min, max, step, .. } => Some(ControlRange { min, max, step }),
        _ => None,
    }
}

/// What a number control's range is on `node` before a hand changed it: its definition's, or
/// where the node's own settings decide its ends — a time-driven node's Offset, one period
/// either way — what they make it now ([`timing::range`]). What a range editor's Default column
/// shows and what clearing a node's own range hands back.
///
/// `None` for an input that is not a number.
pub fn default_range(node: &Node, key: &str) -> Option<ControlRange> {
    let declared = declared_range(node.def, key)?;
    Some(timing::range(node, key).unwrap_or(declared))
}

/// The range a control actually has: the node's own if it has one, else its
/// [default](default_range).
///
/// **The one place this question is answered.** The scrub control, the command bus, the file
/// loader and anything that later maps a fader into a knob all ask here, so a range cannot be
/// two different things depending on who looked.
pub fn control_range(def: &NodeDef, node: &Node, key: &str) -> Option<ControlRange> {
    declared_range(def, key)?;
    node.values
        .get(key)
        .and_then(crate::graph::Value::range)
        .or_else(|| default_range(node, key))
}

/// The ranges that follow a control that just changed: each dependent input's key, and the
/// range its top end has become.
///
/// A control declaring [`Control::Number::capped_by`] takes the named control's value as its
/// maximum — Source Segment cannot pick a wedge Segments did not cut — so the node's own
/// range is rewritten inside the same command that moved the cap, and the bus refits the
/// value the way it refits one after a range editor narrows a track. Empty for every node
/// whose controls cap nothing, which is every node but `kaleidoscope` and
/// `euclideanrhythm` — whose four lanes each cap their own Pulses by their own Steps.
///
/// The cap reads the control and not the port, so a cable into the capping input leaves the
/// range alone: a number arriving from upstream is not this node's to bound by. A hidden
/// control caps and is capped like any other: it is a control, and only its port is missing.
pub fn capped_ranges(
    def: &NodeDef,
    node: &Node,
    changed: &str,
) -> Vec<(&'static str, ControlRange)> {
    let Some(ControlValue::Float(cap)) = node.controls.get(changed) else {
        return Vec::new();
    };
    // Hidden controls too: a lane's Pulses is capped by that lane's own Steps and neither has
    // a port, so a cap that only looked at the port list would never move.
    def.inputs
        .iter()
        .chain(def.hidden)
        .filter(|input| {
            matches!(input.control, Control::Number { capped_by: Some(by), .. } if by == changed)
        })
        .filter_map(|input| {
            let range = control_range(def, node, input.key)?;
            Some((
                input.key,
                ControlRange {
                    max: cap.max(range.min),
                    ..range
                },
            ))
        })
        .collect()
}

/// Fit a range to what the definition allows, or reject it.
///
/// A range narrows or moves within the definition's; it does not escape it. The definition
/// says where the node's maths is defined, and a step of zero or a max below its min is not a
/// range at all.
pub fn coerce_range(
    def: &NodeDef,
    key: &str,
    range: ControlRange,
) -> Option<(&'static str, ControlRange)> {
    let input = def.input(key)?;
    let declared = declared_range(def, key)?;
    // **The declared range does not bound the instance's.** It says where the node's own
    // maths is comfortable and the popup prints it as advice, but a control whose ends could
    // only ever narrow is a control that cannot be pushed — which is half of what a range
    // editor is for. Non-finite is still refused: an end that is NaN is not a decision.
    let min = if range.min.is_finite() {
        range.min
    } else {
        declared.min
    };
    let max = if range.max.is_finite() {
        range.max
    } else {
        declared.max
    };
    // An inside-out range is straightened rather than refused: it is what a half-typed pair of
    // numbers looks like, and refusing would mean the field could not be edited left to right.
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    let span = max - min;
    let step = if range.step.is_finite() && range.step > 0.0 {
        // A step coarser than the range itself leaves a control with one position.
        range.step.min(span.max(f32::MIN_POSITIVE))
    } else {
        declared.step
    };
    Some((input.key, ControlRange { min, max, step }))
}

/// Fit a control value to what the definition declares, or reject it.
///
/// A `Number` input holds a `Float` within its range — the instance's, when it has one — and
/// a `Color` input holds a `Color`.
/// Nothing downstream re-checks: the compiler takes the WGSL type from the definition while
/// the renderer writes the uniform block from the stored variant, so a value of the wrong
/// variant lands as another type's bytes and no error path reports it. Range matters for the
/// same reason a divisor does — a zoom of zero is a NaN frame.
///
/// `None` means the value cannot be stored under that key at all.
pub fn coerce_control(
    def: &NodeDef,
    key: &str,
    value: ControlValue,
    range: Option<ControlRange>,
) -> Option<(&'static str, ControlValue)> {
    let input = def.input(key)?;
    let fitted = match (&input.control, value) {
        (Control::Number { min, max, .. }, ControlValue::Float(v)) => {
            // The instance's range where it has one, so a value cannot sit outside the track
            // it is drawn on. It is itself inside the definition's, so this is still the
            // clamp that keeps a zoom of zero out of a uniform.
            let (min, max) = range.map_or((*min, *max), |r| (r.min, r.max));
            // NaN would propagate into a uniform and paint an undefined frame.
            let v = if v.is_nan() { min } else { v };
            ControlValue::Float(v.clamp(min, max))
        }
        (Control::Color { .. }, ControlValue::Color(c)) => ControlValue::Color(c),
        _ => return None,
    };
    Some((input.key, fitted))
}

/// Is this a value the definition offers for that option?
///
/// `choices` is not the whole answer for a free-text option: `placeholder` marks one whose
/// `choices` are presets rather than the only reachable values, the way an `Asset`'s empty
/// `choices` already says any path is fine.
pub fn option_is_valid(def: &NodeDef, key: &str, value: &str) -> bool {
    def.option(key).is_some_and(|o| {
        o.choices.is_empty()
            || o.placeholder.is_some()
            || o.number.is_some()
            || o.found.is_some()
            || o.choices.iter().any(|(v, _)| *v == value)
    })
}

/// An option's value as it is stored, where the rest of the project decides it: an Output's
/// send name, made unique among every Output ([`output::fit_name`]). Every other value is
/// stored as it is given.
pub fn fit_option(graph: &Graph, id: NodeId, key: &str, value: String) -> String {
    match graph.get(id) {
        Some(node) if node.def.is_output && key == output::SEND_NAME => {
            output::fit_name(graph, id, &value)
        }
        _ => value,
    }
}

/// `#rrggbbaa` to linear 0..1. Anything unparseable is opaque magenta, which is visible
/// rather than silently black.
pub fn parse_hex_rgba(hex: &str) -> [f32; 4] {
    let s = hex.strip_prefix('#').unwrap_or(hex);
    // Byte `i` is characters 2i..2i+2. Indexing by `i` instead happens to work for
    // #ffffffff, where every two-character window is "ff", and for nothing else.
    let byte = |i: usize| u8::from_str_radix(s.get(i * 2..i * 2 + 2).unwrap_or("zz"), 16).ok();
    match (byte(0), byte(1), byte(2)) {
        (Some(r), Some(g), Some(b)) => {
            let a = byte(3).unwrap_or(255);
            [
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                f32::from(a) / 255.0,
            ]
        }
        _ => [1.0, 0.0, 1.0, 1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors_parse_and_bad_ones_are_visible() {
        assert_eq!(parse_hex_rgba("#ffffffff"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(parse_hex_rgba("#000000ff"), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(parse_hex_rgba("#00000000"), [0.0, 0.0, 0.0, 0.0]);
        // Missing alpha defaults to opaque.
        assert_eq!(parse_hex_rgba("#ff0000"), [1.0, 0.0, 0.0, 1.0]);
        // Rubbish is magenta, not black, so a mistake is obvious on screen.
        assert_eq!(parse_hex_rgba("banana"), [1.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn every_control_default_is_representable() {
        for def in REGISTRY {
            for input in def.inputs {
                if let Control::Color { default } = &input.control {
                    // Magenta is what `parse_hex_rgba` answers a string it cannot read, so a
                    // default that means magenta — `colorize`'s tint, silvia's — is held to
                    // its text instead: that text parses, by being the color's own spelling.
                    let magenta = default
                        .trim_start_matches('#')
                        .eq_ignore_ascii_case("ff00ffff")
                        || default
                            .trim_start_matches('#')
                            .eq_ignore_ascii_case("ff00ff");
                    assert!(
                        magenta || parse_hex_rgba(default) != [1.0, 0.0, 1.0, 1.0],
                        "{}.{}: default {default:?} did not parse",
                        def.slug,
                        input.key,
                    );
                }
            }
        }
    }

    /// A press button is the fallback for an action input and means nothing anywhere else:
    /// a number input already has a control, and a color input already has a swatch.
    #[test]
    fn a_press_control_belongs_to_an_action_input() {
        for def in REGISTRY {
            for input in def.inputs {
                if matches!(input.control, Control::Press) {
                    assert_eq!(
                        input.ty,
                        PortType::Action,
                        "{}.{}: a press button on a {:?} input",
                        def.slug,
                        input.key,
                        input.ty,
                    );
                }
            }
        }
    }

    /// A capped control names another number control of the same node, and never itself: the
    /// bus writes the named control's value into this one's range, so a name that resolves to
    /// nothing is a cap that silently never happens.
    #[test]
    fn a_capped_control_names_a_number_of_its_own_node() {
        for def in REGISTRY {
            for input in def.inputs {
                let Control::Number {
                    capped_by: Some(by),
                    ..
                } = input.control
                else {
                    continue;
                };
                assert_ne!(
                    by, input.key,
                    "{}.{}: capped by itself",
                    def.slug, input.key
                );
                assert!(
                    matches!(
                        def.input(by).map(|i| &i.control),
                        Some(Control::Number { .. })
                    ),
                    "{}.{}: capped by {by:?}, which is not a number control of this node",
                    def.slug,
                    input.key,
                );
            }
        }
    }

    /// A hidden control has no port, which is the whole of what makes it hidden. If one ever
    /// reached `port_defs` it would draw a row, take a cable, and be resolvable by the
    /// compiler — three ways for it to stop being what it says it is.
    #[test]
    fn a_hidden_control_has_no_port() {
        for def in REGISTRY {
            let (inputs, _) = def.port_defs();
            for h in def.hidden {
                assert!(
                    !inputs.iter().any(|p| p.key == h.key),
                    "{}.{}: a hidden control with a port",
                    def.slug,
                    h.key,
                );
                assert!(
                    matches!(h.control, Control::Number { .. } | Control::Color { .. }),
                    "{}.{}: a hidden control has to hold a value",
                    def.slug,
                    h.key,
                );
            }
        }
    }

    /// An unbindable key names a control of its own node, so a typo cannot quietly leave a
    /// control bindable that was meant not to be.
    #[test]
    fn an_unbindable_key_is_a_control_of_its_node() {
        for def in REGISTRY {
            for key in def.unbindable {
                assert!(
                    def.input(key).is_some(),
                    "{}.{key}: not a control of this node",
                    def.slug
                );
                assert_eq!(def.bindable(key), None, "{}.{key}", def.slug);
            }
        }
    }

    /// A tick is on or off and nothing else, so the file, the command bus and
    /// `option_is_valid` all agree about what may be stored in one.
    #[test]
    fn every_checkbox_option_is_on_or_off() {
        for def in REGISTRY {
            for o in def.options.iter().filter(|o| o.checkbox) {
                assert_eq!(o.choices, CHECK_CHOICES, "{}.{}", def.slug, o.key);
                assert!(
                    o.default == ON || o.default == OFF,
                    "{}.{} defaults to {:?}",
                    def.slug,
                    o.key,
                    o.default
                );
                assert!(
                    o.placeholder.is_none() && !o.is_asset(),
                    "{}.{} is a tick and something else",
                    def.slug,
                    o.key
                );
            }
        }
    }

    /// A region's heading and the option it keeps its state in travel together.
    ///
    /// The heading is built from a shared option constant and the node declares that option
    /// in its own list, so the two are named in two places. A node that wears a heading without
    /// declaring its option has a triangle that toggles nothing, and one that declares a
    /// heading option with no region under it has an option nothing draws.
    #[test]
    fn a_region_heading_and_its_option_agree() {
        for def in REGISTRY {
            let headings: Vec<_> = def.regions.iter().filter_map(|r| r.heading()).collect();
            for heading in &headings {
                let option = def
                    .option(heading.key)
                    .unwrap_or_else(|| panic!("{}: no option {}", def.slug, heading.key));
                assert!(
                    option.heading,
                    "{}: {} is a heading's option and is not declared as one",
                    def.slug, heading.key
                );
                assert_eq!(option.label, heading.label, "{}", def.slug);
                assert_eq!(
                    option.default == ON,
                    heading.open,
                    "{}: {} opens by one default in the option and another on the region",
                    def.slug,
                    heading.key
                );
            }
            for option in def.options.iter().filter(|o| o.heading) {
                assert!(
                    headings.iter().any(|h| h.key == option.key)
                        || def.row_headings.contains(&option.key),
                    "{}: {} is a heading with nothing to sit on",
                    def.slug,
                    option.key
                );
            }
            for key in def.row_headings {
                let option = def
                    .option(key)
                    .unwrap_or_else(|| panic!("{}: no option {key}", def.slug));
                assert!(
                    option.heading,
                    "{}: {key} folds a run of rows and is not declared a heading",
                    def.slug
                );
            }
        }
    }

    /// An option a region draws belongs to a node that has a region to draw it, and is none of
    /// the other shapes an option can be: a row nothing draws is a choice nobody can make.
    #[test]
    fn an_option_a_region_draws_has_a_region() {
        for def in REGISTRY {
            for o in def.options.iter().filter(|o| o.in_region) {
                assert!(
                    !def.regions.is_empty(),
                    "{}.{}: drawn by a region the node does not have",
                    def.slug,
                    o.key
                );
                assert!(
                    !o.checkbox && !o.heading && !o.is_asset() && o.placeholder.is_none(),
                    "{}.{}: a region's option and a row's shape at once",
                    def.slug,
                    o.key
                );
                assert!(
                    !o.choices.is_empty(),
                    "{}.{}: a region's buttons pick out of a list",
                    def.slug,
                    o.key
                );
            }
        }
    }

    /// A picture region names a texture output of its own node: either half alone is a band
    /// nobody can fill, or a port nothing shows.
    #[test]
    fn a_picture_region_names_a_texture_of_its_own_node() {
        for def in REGISTRY {
            for region in def.regions {
                let Some(Picture::Port(key)) = region.picture() else {
                    continue;
                };
                let port = def
                    .output(key)
                    .unwrap_or_else(|| panic!("{}: a picture names no output {key}", def.slug));
                assert_eq!(
                    port.kind,
                    OutputKind::Texture,
                    "{}: {key} is not a picture",
                    def.slug
                );
            }
        }
    }

    #[test]
    fn slugs_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for def in REGISTRY {
            assert!(seen.insert(def.slug), "duplicate slug {}", def.slug);
        }
    }

    #[test]
    fn every_port_key_is_unique_within_its_node() {
        for def in REGISTRY {
            let mut seen = std::collections::HashSet::new();
            for p in def.inputs.iter().chain(def.hidden) {
                assert!(
                    seen.insert(p.key),
                    "{}: duplicate input {}",
                    def.slug,
                    p.key
                );
            }
            let mut seen = std::collections::HashSet::new();
            for p in def.outputs {
                assert!(
                    seen.insert(p.key),
                    "{}: duplicate output {}",
                    def.slug,
                    p.key
                );
            }
        }
    }

    /// Each kind carries the shape it is read through, because the three are told apart by
    /// the field and not by what they happen to hold.
    ///
    /// `Asset` is the one that holds a path, so it is the one with no choices: the canvas
    /// draws a select for anything with choices and a file button for anything without, and
    /// `project::asset_users` walks the assets looking for paths. A `Code` or `Uniform`
    /// option with no choices would be a select with nothing in it and a file that is not
    /// one; an `Asset` with choices would be a path nobody can type. A free-text option is
    /// typed rather than chosen, so it may have none — an Output's send name.
    #[test]
    fn an_option_carries_the_shape_of_its_kind() {
        for def in REGISTRY {
            for o in def.options {
                match o.kind {
                    OptionKind::Code
                    | OptionKind::Uniform
                    | OptionKind::Presentation
                    | OptionKind::Runtime => assert!(
                        !o.choices.is_empty() || o.placeholder.is_some(),
                        "{}.{}: a {:?} option with no choices",
                        def.slug,
                        o.key,
                        o.kind,
                    ),
                    OptionKind::Asset => assert!(
                        o.choices.is_empty(),
                        "{}.{}: an asset reference with choices",
                        def.slug,
                        o.key,
                    ),
                }
            }
        }
    }

    /// A `Uniform` option's index is what the shader branches on, and it is only ever an
    /// index into `choices`. Anything else — a value from a build with a tenth choice, an
    /// empty string — lands on the default rather than past the last branch.
    #[test]
    fn a_uniform_option_indexes_only_its_own_choices() {
        let def = OptionDef {
            key: "mode",
            label: "Mode",
            default: "b",
            choices: &[("a", "A"), ("b", "B"), ("c", "C")],
            kind: OptionKind::Uniform,
            ..OptionDef::EMPTY
        };
        assert_eq!(def.index_of("a"), 0);
        assert_eq!(def.index_of("c"), 2);
        // Not a choice: the default's index, which is in range by the test above.
        assert_eq!(def.index_of("nonesuch"), 1);
        assert_eq!(def.index_of(""), 1);
    }

    /// An option a connection overrides names an input of the node it is on. The canvas
    /// draws that select disabled while the input is connected and puts the input's label in
    /// place of the chosen value, so a key naming nothing would be a select that never goes
    /// inert and a label nobody can find.
    #[test]
    fn an_option_is_overridden_by_an_input_of_its_own_node() {
        for def in REGISTRY {
            for o in def.options {
                let Some(key) = o.overridden_by else { continue };
                assert!(
                    def.input(key).is_some(),
                    "{}.{}: overridden_by names {key:?}, which is not an input",
                    def.slug,
                    o.key,
                );
            }
        }
    }

    /// An asset holds a path and has no choices to be among, nor has a free-text option that
    /// offers none; everything else must be able to find its own default, and a `Uniform`
    /// option must doubly so — `index_of` falls back to the default's index, so a default that
    /// is not a choice would silently read as zero.
    #[test]
    fn option_defaults_are_among_their_choices() {
        for def in REGISTRY {
            for o in def.options {
                if o.is_asset() || (o.placeholder.is_some() && o.choices.is_empty()) {
                    continue;
                }
                assert!(
                    o.choices.iter().any(|(v, _)| *v == o.default),
                    "{}.{}: default {:?} is not a choice",
                    def.slug,
                    o.key,
                    o.default,
                );
            }
        }
    }

    /// A node's values and its ports share one map on the instance — a range is keyed by the
    /// input it narrows — so a `ValueDef` keyed like one of the node's own ports would be two
    /// different things under one key.
    #[test]
    fn a_declared_value_does_not_collide_with_a_port() {
        for def in REGISTRY {
            for v in def.values {
                assert!(
                    def.input(v.key).is_none(),
                    "{}: value {:?} is also an input",
                    def.slug,
                    v.key,
                );
                assert!(
                    def.output(v.key).is_none(),
                    "{}: value {:?} is also an output",
                    def.slug,
                    v.key,
                );
                assert!(
                    def.option(v.key).is_none(),
                    "{}: value {:?} is also an option",
                    def.slug,
                    v.key,
                );
            }
        }
    }

    /// A free-text option's validator, where it has one, has to accept the option's own
    /// default — otherwise the row it opens in would show the default already marked
    /// invalid, and `validate` is meaningless on anything else.
    #[test]
    fn a_free_text_default_passes_its_own_validator() {
        for def in REGISTRY {
            for o in def.options {
                if let Some(validate) = o.validate {
                    assert!(
                        o.placeholder.is_some(),
                        "{}.{}: validate with no placeholder",
                        def.slug,
                        o.key,
                    );
                    assert!(
                        validate(o.default),
                        "{}.{}: default {:?} fails its own validator",
                        def.slug,
                        o.key,
                        o.default,
                    );
                }
            }
        }
    }

    /// An asset option is a file, and it has to say which kind.
    ///
    /// Two things read it and both are wrong without it: the dialog filters to everything,
    /// and the picker beside it offers every file in the project — a `.ssw` and a font
    /// included — as something the node could play.
    #[test]
    fn an_asset_option_says_what_it_accepts() {
        for def in REGISTRY {
            for o in def.options {
                if !o.is_asset() {
                    continue;
                }
                assert!(
                    !o.accepts.extensions.is_empty(),
                    "{}.{}: a file option with no extensions",
                    def.slug,
                    o.key,
                );
                for extension in o.accepts.extensions {
                    assert_eq!(
                        *extension,
                        extension.to_ascii_lowercase(),
                        "{}.{}: {extension:?} is matched lowercased, so it can never hit",
                        def.slug,
                        o.key,
                    );
                    assert!(
                        !extension.starts_with('.'),
                        "{}.{}: {extension:?} carries its dot",
                        def.slug,
                        o.key,
                    );
                }
            }
        }
    }

    #[test]
    fn a_file_is_accepted_by_its_extension_whatever_its_case() {
        let video = Accepts::VIDEO;
        assert!(video.matches("clip.mp4"));
        assert!(video.matches("CLIP.MP4"), "a camera writes .MP4");
        assert!(video.matches("holiday.2019.mkv"), "the last dot is the one");
        assert!(!video.matches("notes.txt"));
        assert!(!video.matches("mp4"), "an extension is not a name");
        assert!(!video.matches("README"), "and a file may have none");
        // A GIF is a picture, and a picture is the Image/GIF node's alone.
        assert!(!video.matches("loop.gif"), "a GIF is not a clip");
        assert!(Accepts::IMAGE.matches("LOOP.GIF"));
        // Anything at all, which is what an option that is not media carries.
        assert!(Accepts::ANY.matches("README"));
        assert!(Accepts::ANY.matches("clip.mp4"));
    }

    /// Every node declares its own category.
    ///
    /// `NodeDef::EMPTY` has to name one, so a node that forgets is not a compile error but a
    /// node filed under whatever EMPTY happens to say. This is what makes it visible.
    #[test]
    fn every_node_declares_a_category() {
        for def in REGISTRY {
            assert!(
                Category::ALL.contains(&def.category),
                "{}: category is not in Category::ALL",
                def.slug,
            );
        }
        // And every group in the menu has something in it, so the menu never opens a
        // submenu onto nothing.
        for category in Category::ALL {
            assert!(
                REGISTRY.iter().any(|d| d.category == *category),
                "{}: a menu group with no nodes in it",
                category.label(),
            );
        }
    }

    /// Every node that draws a picture names its ports out of one small vocabulary.
    ///
    /// The point of the rule is not the words but that there is a fixed set of them: a
    /// library this size written without one would be one idea under seventy names, and that
    /// is the cost `decisions.md` says retrofitting cannot pay. `mask` **means** coverage or
    /// an inside/outside test, 0 to 1 with 1 inside; every other name is the quantity it is.
    /// Widening the list is a deliberate edit here, which is the whole mechanism.
    ///
    /// Four categories draw: a `Generate` makes a picture, a `Color` maps one, a `Transform`
    /// resamples one and an `Effect` reads a neighborhood or a cell of one. All four are
    /// held to the field names. Three of them are also held to the **picture** names — one
    /// picture is `output`, and a picture with a field beside it is `color`, so a port list
    /// reads the same way whichever node it belongs to. `Transform` is not: its picture is
    /// the input passed through, and that port is `output` whether or not a mask sits beside
    /// it. `Convert` and `Math` are outside this entirely, since they publish no picture and
    /// their port *is* the quantity — `r`, `g`, `b`, `a`.
    ///
    /// `x` and `y` are the one pair that is not a field the body computed on the way to a
    /// picture: they are the picture's own argument, published by `worldcoordinates`, and a
    /// coordinate has exactly those two names.
    ///
    /// A `Texture` output of one of the four is a third thing and has its own list. What a
    /// node whose picture is computed on the CPU publishes there is the **state** it
    /// simulated, which its own WGSL then colors: a `cells`, a `trail`, a `field`. Naming
    /// one `output` would claim it was the picture, and it is the thing the picture is made
    /// from.
    #[test]
    fn a_node_names_its_outputs_from_the_vocabulary() {
        /// The picture, and the second pictures a node may also draw.
        const PICTURES: &[&str] = &["output", "color", "map"];
        /// The fields published beside a picture, and the two a coordinate is.
        const FIELDS: &[&str] = &["mask", "smooth", "angle", "value", "x", "y"];
        /// The state a CPU half publishes as a texture for its own WGSL to read.
        const STATES: &[&str] = &["cells", "trail", "field"];
        /// A **set**: several pictures of one kind, distinguished only by their place in the
        /// set. `palette`'s eight variations of its input are the one there is, and nothing
        /// in the vocabulary names one of eight, so the set is its own naming: a letter each,
        /// in order, and the picture rule does not apply to a node that has no single
        /// picture.
        const SET: &[&str] = &["a", "b", "c", "d", "e", "f", "g", "h"];
        /// The categories whose nodes draw one.
        const DRAWS: &[Category] = &[
            Category::Generate,
            Category::Color,
            Category::Transform,
            Category::Effect,
        ];

        for def in REGISTRY.iter().filter(|d| DRAWS.contains(&d.category)) {
            let has_field = def.outputs.iter().any(|p| p.ty == PortType::VaryingNumber);
            let pictures: Vec<_> = def
                .outputs
                .iter()
                .filter(|p| p.ty == PortType::VaryingColor && p.kind != OutputKind::Texture)
                .collect();
            let is_set = pictures.len() > 1 && pictures.iter().all(|p| SET.contains(&p.key));
            for p in def.outputs {
                match p.ty {
                    PortType::VaryingNumber => assert!(
                        FIELDS.contains(&p.key),
                        "{}.{}: a field is one of {FIELDS:?}",
                        def.slug,
                        p.key,
                    ),
                    PortType::VaryingColor if p.kind == OutputKind::Texture => assert!(
                        STATES.contains(&p.key),
                        "{}.{}: a state is one of {STATES:?}",
                        def.slug,
                        p.key,
                    ),
                    PortType::VaryingColor if is_set => {}
                    PortType::VaryingColor => {
                        assert!(
                            PICTURES.contains(&p.key),
                            "{}.{}: a picture is one of {PICTURES:?}",
                            def.slug,
                            p.key,
                        );
                        if def.category == Category::Transform {
                            continue;
                        }
                        let expected = if has_field { "color" } else { "output" };
                        assert!(
                            p.key == expected || p.key == "map",
                            "{}.{}: expected {expected:?}",
                            def.slug,
                            p.key,
                        );
                    }
                    // A uniform port or an action is a CPU port; it is named for what
                    // `tick` publishes, not for a field in a shader.
                    PortType::UniformNumber | PortType::UniformColor | PortType::Action => {}
                }
            }
        }
    }

    /// A uniform number is produced by `tick`, so a node publishing one has a CPU half, and
    /// a node with a CPU half has something to publish. Either alone is a definition that
    /// can never carry a value.
    #[test]
    fn uniform_outputs_and_cpu_state_come_together() {
        for def in REGISTRY {
            let publishes = def
                .outputs
                .iter()
                .any(|p| matches!(p.kind, OutputKind::Uniform | OutputKind::Action))
                || def
                    .outputs
                    .iter()
                    .any(|p| p.kind == OutputKind::Texture && !def.is_output);
            assert_eq!(
                publishes,
                def.cpu.is_some(),
                "{}: a CPU output needs a `cpu` half and vice versa",
                def.slug,
            );
        }
    }

    /// The two bits an offline render reads, held to the source that would contradict them.
    /// A tick reading `ctx.dt` or `ctx.elapsed` is either summing time into its state or
    /// placing a device's sample inside the frame, so it is `integrates` or `live`; one that
    /// reads only where it is in time — `ctx.time`, `ctx.cycle` — may be neither, since
    /// the playhead is the frame's own; and a node that says it integrates reads one of them,
    /// or the claim is idle.
    /// A node is read from the file named for it, and the two gears from `gear.rs`; a
    /// sequencer hands its tick to `sequencer.rs`'s transport, so that file is read beside its
    /// own.
    #[test]
    fn a_tick_that_reads_time_says_so() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/nodes");
        for def in REGISTRY {
            let Some(cpu) = &def.cpu else { continue };
            let file = match def.slug {
                "mastergear" | "ratiogear" => "gear",
                slug => slug,
            };
            let mut source =
                std::fs::read_to_string(root.join(format!("{file}.rs"))).unwrap_or_default();
            if source.contains("nodes::sequencer") {
                source += &std::fs::read_to_string(root.join("sequencer.rs")).unwrap_or_default();
            }
            if !source.contains("impl CpuNode for") {
                // A node declared in a file of another name — the test fixture's, a dual
                // math node's — has no tick of its own to hold.
                continue;
            }
            let steps = source.contains("ctx.dt") || source.contains("ctx.elapsed");
            let reads_time = steps || source.contains("ctx.time") || source.contains("ctx.cycle");
            assert!(
                !steps || cpu.integrates || cpu.live,
                "{}: steps on the clock and claims neither `integrates` nor `live`",
                def.slug,
            );
            assert!(
                !cpu.integrates || reads_time,
                "{}: claims to integrate and never reads `dt` or the transport",
                def.slug,
            );
        }
    }

    /// A measurement is read back by a `tick`, so a node that makes one has a CPU half.
    /// Without it the slot would be written every frame and nobody would ever decode it.
    #[test]
    fn a_measurement_belongs_to_a_node_with_a_cpu_half() {
        for def in REGISTRY {
            if def.measure_wgsl.is_some() {
                assert!(
                    def.cpu.is_some(),
                    "{}: a measurement needs a `cpu` half to read it back",
                    def.slug,
                );
            }
        }
    }

    /// A delayed output is a uniform reading that a measurement produced — a number, or the
    /// color a `sample` measured: the node has a `cpu` half to publish it, a `measure` that
    /// made it, and — true of `tap`, `sample` and `autoexposure` alike — a `Shader` output for
    /// its pass-through. A `Texture` output is never flagged this way, because `port_defs`
    /// already delays it by its kind.
    #[test]
    fn a_delayed_output_is_a_uniform_a_measurement_made() {
        for def in REGISTRY {
            for p in def.outputs {
                if p.kind == OutputKind::Texture {
                    assert!(
                        !p.delayed,
                        "{}.{}: a texture is delayed by kind; the flag is redundant",
                        def.slug, p.key,
                    );
                }
                if !p.delayed {
                    continue;
                }
                assert!(
                    p.ty.is_uniform(),
                    "{}.{}: a delayed output is a uniform, not {:?}",
                    def.slug,
                    p.key,
                    p.ty,
                );
                assert!(
                    def.cpu.is_some() && def.measure_wgsl.is_some(),
                    "{}.{}: a delayed uniform needs a cpu half and the measurement \
                     that reads it back",
                    def.slug,
                    p.key,
                );
                assert!(
                    def.outputs.iter().any(|o| o.kind == OutputKind::Shader),
                    "{}.{}: every node with a delayed uniform also passes its input \
                     through",
                    def.slug,
                    p.key,
                );
            }
        }
    }

    /// `wrap` and `filter` parametrize a texture, so only a `Texture` output may move either
    /// off the rule — a `Shader` output has no texture to parametrize, and a declaration there
    /// would be read by nothing.
    ///
    /// The two that ask are the two simulated worlds that wrap in their own simulation, and
    /// the list is spelled out because the exception is the argument: the picture of a torus
    /// tiles, and a mirrored tiling would fold a seam through a world that has none. See
    /// [decisions.md](../../docs/decisions.md#textures-mirror-wrap-outside-their-bounds).
    #[test]
    fn only_a_texture_output_parametrizes_its_sampling() {
        let mut declared = Vec::new();
        for def in REGISTRY {
            for p in def.outputs {
                let rule = (p.wrap, p.filter) == (TextureWrap::Mirror, TextureFilter::Linear);
                assert!(
                    rule || p.kind == OutputKind::Texture,
                    "{}.{}: a {:?} output has no texture to sample",
                    def.slug,
                    p.key,
                    p.kind,
                );
                if !rule {
                    declared.push((def.slug, p.key, p.wrap, p.filter));
                }
            }
        }
        assert_eq!(
            declared,
            vec![
                (
                    "cellularautomata",
                    "cells",
                    TextureWrap::Repeat,
                    TextureFilter::Nearest
                ),
                (
                    "slimemold",
                    "trail",
                    TextureWrap::Repeat,
                    TextureFilter::Linear
                ),
            ],
        );
    }

    /// A dual output is a `VaryingNumber` whose formula a `tick` can actually run.
    ///
    /// Four things have to hold for the two implementations to be the same function. The
    /// declared type is `VaryingNumber`, since the effective one is what `Graph` computes
    /// and `UniformNumber` is the answer rather than the declaration. The node has no `cpu`
    /// half, so the tick's dual branch is the only thing publishing that port and the two
    /// cannot disagree about it. It has no `measure`, since a measurement is a reading of a
    /// picture and there is no picture in diamond mode. And every input is a `VaryingNumber`
    /// with a number control — anything else could never resolve to a uniform number, so the
    /// node would be a field forever and the `eval` would be dead code.
    #[test]
    fn a_dual_output_is_a_varying_number_a_tick_can_evaluate() {
        for def in REGISTRY {
            for p in def.outputs.iter().filter(|p| p.eval.is_some()) {
                assert_eq!(
                    p.ty,
                    PortType::VaryingNumber,
                    "{}.{}: a dual output declares VaryingNumber",
                    def.slug,
                    p.key,
                );
                assert_eq!(
                    p.kind,
                    OutputKind::Shader,
                    "{}.{}: a dual output is a shader function in circle mode",
                    def.slug,
                    p.key,
                );
                assert!(
                    def.cpu.is_none(),
                    "{}: a dual output and a `cpu` half would both publish {}",
                    def.slug,
                    p.key,
                );
                assert!(
                    def.measure_wgsl.is_none(),
                    "{}: a measurement reads a picture, which a dual node in diamond mode \
                     does not have",
                    def.slug,
                );
                for input in def.inputs {
                    assert_eq!(
                        input.ty,
                        PortType::VaryingNumber,
                        "{}.{}: a dual node's input has to be able to carry a number",
                        def.slug,
                        input.key,
                    );
                    assert!(
                        matches!(input.control, Control::Number { .. }),
                        "{}.{}: an unconnected input of a dual node needs a number control",
                        def.slug,
                        input.key,
                    );
                }
            }
        }
    }

    /// **A node that moves with time declares its timing, and its time rows are the
    /// declaration's.** Every node with a [`Timing`] has exactly the rows `nodes::timing`
    /// expands it into — Time, Speed and Offset per axis, in one run, an Offset varying on a
    /// node that draws and uniform on a CPU node — its Timing heading, closed, on those rows,
    /// and its mode on the heading. No other node has a time row or the heading. The expansion
    /// itself is tested once, in `timing`.
    #[test]
    fn a_moving_node_has_the_rows_its_timing_expands_into() {
        for def in REGISTRY {
            let Some(t) = def.timing else {
                assert!(
                    def.option(timing::HEADING.key).is_none()
                        && def.option(timing::MODE.key).is_none(),
                    "{}: a Timing heading or a mode with no timing",
                    def.slug
                );
                assert!(
                    !def.inputs
                        .iter()
                        .any(|i| is_time(i.key) || i.key == timing::OFFSET)
                        || def.slug == "ratiogear",
                    "{}: a Time or an Offset with no timing",
                    def.slug
                );
                continue;
            };
            let offset = if def.cpu.is_none() {
                PortType::VaryingNumber
            } else {
                PortType::UniformNumber
            };
            // An input as text, since a definition compares nothing itself.
            let row = |i: &InputDef| match i.control {
                Control::Number {
                    default, min, max, ..
                } => format!("{} {} {:?} {default} {min} {max}", i.key, i.label, i.ty),
                _ => format!("{} {} {:?}", i.key, i.label, i.ty),
            };
            let want: Vec<String> = t
                .axes()
                .iter()
                .flat_map(|a| {
                    [
                        timing::time_row(t, a.index),
                        timing::speed_row(t, a.index),
                        timing::offset_row(t, a.index, offset),
                    ]
                })
                .map(|i| row(&i))
                .collect();
            let have: Vec<String> = def
                .inputs
                .iter()
                .skip_while(|i| !is_time_row(i.key))
                .take(want.len())
                .map(row)
                .collect();
            assert_eq!(have, want, "{}: the rows its timing expands into", def.slug);
            assert_eq!(
                def.inputs.iter().filter(|i| is_time_row(i.key)).count(),
                want.len(),
                "{}: every time row is in the one run under the heading",
                def.slug
            );
            let heading = def.option(timing::HEADING.key);
            assert!(
                heading.is_some_and(|o| o.heading && o.default == OFF && o.label == "Timing"),
                "{}: a closed Timing heading",
                def.slug
            );
            let mode = def.option(timing::MODE.key);
            assert!(
                mode.is_some_and(
                    |o| o.default == timing::FREE && o.on_heading == Some(timing::HEADING.key)
                ),
                "{}: its mode, Free, on the heading",
                def.slug
            );
            assert!(
                def.row_headings.contains(&timing::HEADING.key),
                "{}: the heading sits on the rows it folds",
                def.slug
            );
        }
    }

    /// **Angles are in turns**: 1 is one full turn on every angle knob, so a gear's Phase cabled
    /// into a Rotation turns it once a cycle with no snap. No knob is in π any more, and the
    /// ranges and defaults keep their meaning: what was ±4π is ±2 turns.
    #[test]
    fn every_angle_knob_is_in_turns() {
        let angles = [
            ("rotate", "angle", 0.0),
            ("polygon", "rotation", 0.0),
            ("star", "rotation", 0.0),
            ("spiral", "rotation", 0.0),
            ("phyllotaxis", "angle", 0.382),
            ("stripes", "rotation", 0.0),
            ("lineargradient", "angle", 0.25),
            ("stargate", "angle", 0.25),
            ("sine", "phase", 0.0),
            ("cosine", "phase", 0.0),
            ("wave", "phase", 0.0),
            ("wave", "rotation", 0.0),
            ("whirlandpinch", "whirl", 0.25),
            ("emboss", "angle", 0.125),
            ("chromaticaberration", "angle", 0.0),
            ("motionblur", "angle", 0.0),
            ("halftone", "angle", 0.125),
            ("colorshift", "hue", 0.0),
        ];
        for def in REGISTRY {
            for input in def.inputs.iter().chain(def.hidden) {
                if let Control::Number { unit, .. } = input.control {
                    assert_ne!(unit, "π", "{}.{} is still in π", def.slug, input.key);
                }
            }
        }
        for (slug, key, want) in angles {
            let def = find(slug).unwrap_or_else(|| panic!("no {slug}"));
            let input = def
                .input(key)
                .unwrap_or_else(|| panic!("{slug} has no {key}"));
            let Control::Number { default, unit, .. } = input.control else {
                panic!("{slug}.{key} is a number");
            };
            assert_eq!(unit, TURNS, "{slug}.{key}");
            assert!(
                (default - want).abs() < 1e-6,
                "{slug}.{key} opens at {default}"
            );
        }
    }

    /// **Which nodes move with time, and how fast**: the twelve that draw and the five on the
    /// CPU, each at one pace in both modes — silvia's speed, or for a node silvia keeps still a
    /// pace that looks natural, with its Speed starting at zero so a new one still sits still
    /// in Free mode. No node keeps a speed of its own beside its time rows.
    #[test]
    fn the_moving_nodes_and_their_paces() {
        let timed = |slug: &str| {
            find(slug)
                .and_then(|d| d.timing)
                .unwrap_or_else(|| panic!("{slug} keeps time"))
        };
        let mut drawn: Vec<&str> = REGISTRY
            .iter()
            .filter(|d| d.timing.is_some() && d.cpu.is_none())
            .map(|d| d.slug)
            .collect();
        drawn.sort_unstable();
        assert_eq!(
            drawn,
            [
                "cosinegradient",
                "domainwarp",
                "fractal",
                "geissflow",
                "juliaset",
                "mandelbrot",
                "perlin",
                "rotozoom",
                "shakycam",
                "simplex",
                "static",
                "tunnel3d",
            ],
            "every generator and transform that moves with time"
        );
        let mut cpu: Vec<&str> = REGISTRY
            .iter()
            .filter(|d| d.timing.is_some() && d.cpu.is_some())
            .map(|d| d.slug)
            .collect();
        cpu.sort_unstable();
        assert_eq!(
            cpu,
            [
                "euclideanrhythm",
                "imagegif",
                "oscillator",
                "stepsequencer",
                "video"
            ],
            "and every node on the CPU that does"
        );
        for (slug, pace, still) in [
            ("mandelbrot", 0.5, false),
            ("juliaset", 0.5, false),
            ("rotozoom", 1.0 / 60.0, false),
            ("shakycam", 1.0 / 60.0, false),
            ("geissflow", 1.0 / 160.0, false),
            ("perlin", 0.5, false),
            ("tunnel3d", 0.5 / 64.0, false),
            ("oscillator", 1.0, false),
            ("video", 1.0, false),
            ("imagegif", 1.0, false),
            ("cosinegradient", 0.1, true),
            ("simplex", 0.5, true),
            ("fractal", 0.5, true),
            ("domainwarp", 0.5, true),
            ("static", 6.0, true),
            ("stepsequencer", 0.5, true),
            ("euclideanrhythm", 0.5, true),
        ] {
            let t = timed(slug);
            assert_eq!((t.pace, t.still), (pace, still), "{slug}");
            assert_eq!(t.speed_default(), if still { 0.0 } else { 1.0 }, "{slug}");
        }
        assert_eq!(timed("shakycam").axes, timing::Axes::Two, "a Time per axis");
        assert_eq!(
            timed("shakycam").pace_of(timing::Axis::Y),
            1.0 / 15.0,
            "and Y's cycle of its own, four a minute"
        );
        assert!(find("oscillator").unwrap().input("frequency").is_none());
        for def in REGISTRY.iter().filter(|d| d.timing.is_some()) {
            for speed in [
                "timeSpeed",
                "cycle",
                "flowSpeed",
                "rotSpeed",
                "zoomSpeed",
                "xSpeed",
                "ySpeed",
                "bpm",
                "time",
                "position",
            ] {
                assert!(
                    def.input(speed).is_none(),
                    "{}: a {speed} of its own",
                    def.slug
                );
            }
        }
    }

    /// **No emitted node function names `u_resolution`, `frag_coord` or the `position`
    /// builtin**, nor GL's `gl_FragCoord`, which a body ported from silvia would reach for.
    ///
    /// A node body that reads any of them is a different field in Outputs of different size,
    /// and a tap behind one measures a different picture per Output — which is what breaks the
    /// rule that a uniform number is one number per frame. A size in pixels is measured against
    /// [`REFERENCE_HEIGHT`] instead. The uniform struct keeps `u_resolution` and `fs_main`
    /// keeps `frag_coord`, because `fs_main` is where `uv` is built and where a measurement's
    /// cell comes from; this holds the node bodies alone. See Worldspace in `docs/nodes.md`.
    ///
    /// Every choice of every option is emitted, since an option is read while generating and
    /// a branch nothing selects is a branch nothing reads.
    #[test]
    fn no_node_function_reads_the_resolution() {
        use crate::compile::CompileContext;
        use crate::graph::Graph;

        /// What a node body may not name.
        const FORBIDDEN: &[&str] = &[
            "u_resolution",
            "gl_FragCoord",
            "frag_coord",
            "@builtin(position)",
        ];

        for def in REGISTRY {
            for util in def.wgsl_utils {
                for name in FORBIDDEN {
                    assert!(
                        !util.contains(name),
                        "{}: a shader util names {name}",
                        def.slug,
                    );
                }
            }

            let mut combos: Vec<Vec<(&'static str, &'static str)>> = vec![Vec::new()];
            for option in def.options.iter().filter(|o| !o.is_asset()) {
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

            for combo in &combos {
                let mut graph = Graph::new();
                let id = add_to_graph(&mut graph, def.slug, Pos2::ZERO).expect("in the registry");
                for (key, value) in combo {
                    graph
                        .get_mut(id)
                        .expect("just added")
                        .options
                        .insert(key, (*value).to_string());
                }
                for out in def.outputs {
                    // A uniform number and an action have no generator at all: `tick`
                    // publishes them.
                    if !matches!(out.kind, OutputKind::Shader | OutputKind::Texture) {
                        continue;
                    }
                    let func = format!("{}{id}_{}", def.slug, out.key);
                    let mut ctx = CompileContext::new_for_test(&graph);
                    let body = (out.wgsl)(id, &mut ctx, &func);
                    for name in FORBIDDEN {
                        assert!(
                            !body.contains(name),
                            "{}.{} names {name} with options {combo:?}:\n{body}",
                            def.slug,
                            out.key,
                        );
                    }
                }
            }
        }
    }

    /// The port type and the output kind say the same thing twice, and must agree: a
    /// uniform port with a `Shader` kind would be asked for a function it cannot write, and an
    /// `Action` port with one would be asked for a function that has no meaning at all.
    #[test]
    fn uniform_ports_are_uniform_output_kinds() {
        for def in REGISTRY {
            for p in def.outputs {
                assert_eq!(
                    p.ty.is_uniform(),
                    p.kind == OutputKind::Uniform,
                    "{}.{}: type {:?} against kind {:?}",
                    def.slug,
                    p.key,
                    p.ty,
                    p.kind,
                );
                assert_eq!(
                    p.ty == PortType::Action,
                    p.kind == OutputKind::Action,
                    "{}.{}: type {:?} against kind {:?}",
                    def.slug,
                    p.key,
                    p.ty,
                    p.kind,
                );
            }
            for p in def.inputs {
                // A CPU node reads its inputs in `tick`, which can hold a uniform number
                // and not a field. A GPU node takes a varying number, which a uniform
                // number feeds for free. A node with both halves — a tap — may take a field,
                // because its WGSL reads it.
                let has_shader = def.outputs.iter().any(|o| o.kind == OutputKind::Shader);
                if def.cpu.is_some() && !has_shader {
                    assert!(
                        p.ty.is_cpu(),
                        "{}.{}: a CPU node cannot read a field; declare it UniformNumber",
                        def.slug,
                        p.key,
                    );
                }
            }
        }
    }
}
