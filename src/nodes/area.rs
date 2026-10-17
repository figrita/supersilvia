// SPDX-License-Identifier: AGPL-3.0-or-later

//! The node's own area, described: which regions a node draws below its rows, what heading
//! each one wears and which picture it shows.
//!
//! A description and nothing else. How tall a region is and how it is drawn are `widgets/`,
//! keyed by the [`Region`] a node declares here, so the registry names what it wants drawn
//! without naming the toolkit that draws it — `nodes/` takes no graphical dependency, and
//! `tests/rules.rs` walks every path it names to hold it to that. What stays here is what
//! the registry and the file both have to agree on: the heading's option key, and the
//! texture output a picture shows. See docs/ui.md.

use crate::nodes::{SHOW_PREVIEW, SHOW_TRACE, audio_ports::SHOW_SCOPE, reframerange};

/// One region of a node's body: a band below the rows that the node fills itself.
///
/// silvia's `customArea`, as a name: each variant is one widget in `widgets/`, and a node
/// lists the ones it has in draw order in `NodeDef::regions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// An Output's own render, flush with the body's bottom corners. No heading: it is the
    /// node, and a triangle that closed it would turn an Output into a header with ports.
    Render,
    /// A source's own picture of one of its texture outputs, under the Preview heading.
    Preview(&'static str),
    /// An audio source's spectrum and band handles, under the Scope heading.
    Scope,
    /// The three band meters alone, read-only: what a `maininput` node draws.
    Meters,
    /// The ring a `CpuNode::trace` keeps, drawn as a curve.
    Trace,
    /// The cells under a trace: what `CpuNode::caption` says, a label over a reading.
    Caption,
    /// One line of what `CpuNode::status` says.
    Status,
    /// A recorded curve, as it is performed and as it was saved.
    Curve,
    /// `cosinegradient`'s strip and its three channel curves over a grid of numbers.
    Palette,
    /// `euclideanrhythm`'s four lanes of steps.
    Steps,
    /// The List node's bars, one per item, which a hand drags up and down.
    Bars,
    /// `stepsequencer`'s four lanes of cells a hand lights, drawn in the cells `Steps` is.
    Grid,
    /// Reframe Range's named ranges, under a heading of their own.
    Ranges,
    /// The camcorder's viewfinder, which a hand aims a feedback loop in.
    Viewfinder,
    /// A row of buttons, each writing some of the node's own settings as one edit:
    /// `lyapunov`'s Random Seq and `slimemold`'s nine presets.
    Buttons(&'static Buttons),
    /// `xypad`'s square, which a hand throws a puck around, with its two numbers and its
    /// presets under it.
    XyPad,
    /// A picture a hand paints on: `drawingcanvas`'s surface, which is also the picture of
    /// what the node publishes on the port it names.
    Paint(&'static str),
    /// The brush under a paint surface: the tool buttons, the size, the two colors and the
    /// keys.
    Brush,
    /// A gear's own picture — a still rosette or two meshing gears, by its Display option —
    /// turning at the gear's real rate, with what it is set to beside it.
    Gear,
    /// A Ratio Gear's Teeth: its two whole numbers on one row, `p : q`.
    Teeth,
    /// A folding section of a node's own controls, laid out as its [`Section`] says: Render
    /// 3D's Camera, Light, Look and Quality.
    Section(&'static Section),
}

/// A row of buttons on a node's body, each of which writes some of its node's own options and
/// controls — silvia's `Random Seq` and its slime mold's preset bar.
///
/// **A press is an edit, not a gate.** What it writes is document data, so it goes through
/// the command bus as one `SetSettings` — one undo step, saved with the patch, and a `Code`
/// option among it rebuilds the shader exactly as a pick from the option's own row does. It
/// is therefore not an action input: nothing downstream can fire it, because a write that
/// rebuilds a shader is not something a cable should do at frame rate.
#[derive(Debug)]
pub struct Buttons {
    /// Each button's key — the last part of its accessible name, `{slug}{id}.{key}` — and
    /// its caption, left to right.
    pub buttons: &'static [(&'static str, &'static str)],
    /// What the button at `index` writes. `seed` is a fresh number every press, for a button
    /// that rolls dice; a preset ignores it.
    pub press: fn(index: usize, seed: u32) -> Settings,
    /// A key the node's own `tick` reads as pressed on the tick after any of these buttons
    /// is, through `TickContext::downs`: the half of a press that is the running world's
    /// rather than the document's. `slimemold` knocks its trail back on a preset.
    pub pulse: Option<&'static str>,
}

/// Declared once as a `static`, so one row is one row: identity is the address.
impl PartialEq for Buttons {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Buttons {}

/// A section of a node's body: a heading that folds it, and the node's own controls laid out
/// under it as [`Part`]s — a dial a hand drags, a segmented switch for an option, a number, a
/// swatch, and a button that opens a panel of more of them.
///
/// **Not everything a node keeps needs a port.** A control a section draws is usually a hidden
/// one, `NodeDef::hidden`: saved, undone through the bus, bindable to MIDI, resolved by the
/// compiler as the uniform a row's control is, and with no row and no cable. An option a
/// section draws is `OptionDef::in_region`, so it has no row either. See docs/ui.md.
pub struct Section {
    /// The heading's option, which the node also declares in its own list.
    pub heading: &'static crate::nodes::OptionDef,
    /// What the section draws, top to bottom.
    pub parts: &'static [Part],
}

/// Declared once as a `static`, so one section is one section: identity is the address.
impl PartialEq for Section {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl Eq for Section {}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Section")
            .field("heading", &self.heading.key)
            .field("parts", &self.parts)
            .finish()
    }
}

/// One piece of a [`Section`], or of the panel a [`Popout`] opens. Every key names one of the
/// node's own controls or options; `caption` is the words drawn beside it.
#[derive(Debug, Clone, Copy)]
pub enum Part {
    /// A dial on the left, with a column of parts beside it.
    Beside(Dial, &'static [Part]),
    /// An option as a segmented switch, one segment per choice.
    Choice {
        key: &'static str,
        caption: &'static str,
    },
    /// A number control, as the inset s-number.
    Number {
        key: &'static str,
        caption: &'static str,
    },
    /// A color control, as the swatch.
    Swatch {
        key: &'static str,
        caption: &'static str,
    },
    /// A button that opens a panel of more parts beside the node.
    Popout(&'static Popout),
    /// Parts that have an effect only while the node is set a certain way: drawn as any
    /// others are while it holds, and dimmed and deaf to the hand while it does not.
    When(Applies, &'static [Part]),
    /// A few words in the tiny muted type, one entry a line: how a panel's settings take
    /// effect where that is decided somewhere other than the panel.
    Note(&'static [&'static str]),
}

/// What gives some parts of a section an effect, read off the node and its cables: the
/// condition a [`Part::When`] draws its parts dimmed without.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    /// While an option holds this value.
    Is {
        key: &'static str,
        value: &'static str,
    },
    /// While an option holds any value but this one.
    IsNot {
        key: &'static str,
        value: &'static str,
    },
    /// While a cable runs into this input.
    Cabled(&'static str),
}

impl Applies {
    /// Whether it holds on `node`, with `cabled` saying whether a cable runs into one of its
    /// inputs, by key.
    pub fn holds(self, node: &crate::graph::Node, cabled: impl Fn(&str) -> bool) -> bool {
        let option = |key: &str| {
            node.options
                .get(key)
                .map(String::as_str)
                .or_else(|| node.def.option(key).map(|o| o.default))
                .unwrap_or_default()
        };
        match self {
            Self::Is { key, value } => option(key) == value,
            Self::IsNot { key, value } => option(key) != value,
            Self::Cabled(key) => cabled(key),
        }
    }
}

/// A button in a section and the panel it opens: hung under the button, closed by a click
/// outside it, by `Escape` or by the button again. Which panel is open is the editor's, not
/// the document's.
#[derive(Debug)]
pub struct Popout {
    /// The last part of the button's accessible name, `{slug}{id}.{key}`.
    pub key: &'static str,
    /// The button's words, with the ellipsis that says it opens something.
    pub caption: &'static str,
    pub parts: &'static [Part],
    /// The controls a Reset button at the panel's foot puts back to their defaults, as one
    /// edit. Empty for a panel with no Reset.
    pub reset: &'static [&'static str],
}

/// A picture of where something is, which a hand drags to move it.
#[derive(Debug, Clone, Copy)]
pub enum Dial {
    /// A camera on a globe round its target.
    Globe(&'static Globe),
    /// A light on the sky seen from above: the rim is the horizon and the middle straight
    /// up. Both keys are in turns, the height from 0 to a quarter.
    Sun {
        azimuth: &'static str,
        height: &'static str,
        /// The globe whose camera the disk marks on its rim.
        camera: &'static Globe,
    },
}

/// The controls a globe reads and writes: an orbit and a tilt in turns and a distance, or,
/// while `mode` is set to `look_at`, a From point round a Target.
#[derive(Debug)]
pub struct Globe {
    pub orbit: &'static str,
    pub tilt: &'static str,
    pub distance: &'static str,
    /// The light's azimuth and height, in turns, drawn as a sun beside the globe.
    pub light: [&'static str; 2],
    /// The option that says how the camera is placed, and the value that stands it at From.
    pub mode: &'static str,
    pub look_at: &'static str,
    pub from: [&'static str; 3],
    pub target: [&'static str; 3],
}

/// What one press of a node's button writes on its own node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    /// Options, by key, and the value each is set to.
    pub options: Vec<(&'static str, String)>,
    /// Number controls, by key, and the value each is set to — fitted to the control's own
    /// range by the command bus, as any other write is.
    pub controls: Vec<(&'static str, f32)>,
}

/// The heading over a region: what it is called, and whether a node nobody has asked has it
/// open.
///
/// `key` names an option in `Node::options` — `OptionDef::heading`,
/// `OptionKind::Presentation` — so the state is document data, undoable, and read off the
/// `Node` by layout. `open` is the answer for a file written before the option existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heading {
    pub key: &'static str,
    pub label: &'static str,
    pub open: bool,
}

/// Which texture a region shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Picture {
    /// An Output's own render, which the renderer keys by node rather than by port.
    Render,
    /// A texture the node publishes on one of its outputs.
    Port(&'static str),
}

impl Picture {
    /// The port a thumbnail names: `None` is an Output's own render.
    pub fn port(self) -> Option<&'static str> {
        match self {
            Self::Render => None,
            Self::Port(key) => Some(key),
        }
    }
}

impl Region {
    /// The heading this region wears, where it can close. Built from the option that keeps
    /// its state, so the key cannot name one option here and another on the node.
    pub fn heading(self) -> Option<Heading> {
        let option = match self {
            Self::Preview(_) => &SHOW_PREVIEW,
            Self::Scope => &SHOW_SCOPE,
            Self::Ranges => &reframerange::SHOW_NAMED,
            Self::Trace | Self::Curve => &SHOW_TRACE,
            Self::Section(section) => section.heading,
            Self::Render
            | Self::Meters
            | Self::Caption
            | Self::Status
            | Self::Palette
            | Self::Steps
            | Self::Bars
            | Self::Grid
            | Self::Viewfinder
            | Self::Buttons(_)
            | Self::XyPad
            | Self::Paint(_)
            | Self::Brush
            | Self::Gear
            | Self::Teeth => return None,
        };
        Some(Heading {
            key: option.key,
            label: option.label,
            open: option.default == crate::nodes::ON,
        })
    }

    /// The picture this region shows, where it shows one. `None` for a region that draws
    /// itself — a scope, a trace.
    pub const fn picture(self) -> Option<Picture> {
        match self {
            Self::Render => Some(Picture::Render),
            Self::Preview(port) | Self::Paint(port) => Some(Picture::Port(port)),
            Self::Scope
            | Self::Meters
            | Self::Trace
            | Self::Caption
            | Self::Status
            | Self::Curve
            | Self::Palette
            | Self::Steps
            | Self::Bars
            | Self::Grid
            | Self::Ranges
            | Self::Viewfinder
            | Self::Buttons(_)
            | Self::XyPad
            | Self::Brush
            | Self::Gear
            | Self::Teeth
            | Self::Section(_) => None,
        }
    }
}
