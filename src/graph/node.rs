// SPDX-License-Identifier: AGPL-3.0-or-later

//! A node instance. Pure data: no GPU, no egui, no code generation.

use crate::graph::WorkspaceId;
use crate::graph::port::PortDef;
use crate::nodes::{Frame, NodeDef};
use emath::Pos2;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

/// A control's current value. Colors are stored as linear 0..1 floats; the `#rrggbbaa` form
/// exists only at the file and node-definition boundary.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ControlValue {
    Float(f32),
    Color([f32; 4]),
}

/// What a number control's ends and quantum are on **this** node.
///
/// A definition declares a range for its kind; an instance may want a different one — a
/// speed that only ever wants 0.9 to 1.1 is unscrubbable across 0.1 to 50, and a MIDI fader
/// or a timeline lane needs to know where a parameter's ends are *here*. So the range is
/// document data beside the value, absent until something changes it.
///
/// It goes where it is put. What the definition declares is advice — printed in the range
/// editor's header and binding nothing — because a control whose ends can only ever narrow
/// cannot be pushed, and pushing a parameter past where its author expected is most of what
/// this is for. A non-finite end is refused; NaN is not a decision.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControlRange {
    pub min: f32,
    pub max: f32,
    pub step: f32,
}

/// One piece of state a node keeps for itself: not a port's value, and not a choice.
///
/// A node stores three kinds of thing, and silvia stores the same three. `controls` is what
/// an input port shows when nothing is plugged into it. `options` is a choice out of a list
/// the node declares. Some things are neither, and those are values. silvia's own registry
/// puts it plainly: *"serializable, user-controlled state that doesn't fit the standard
/// `input` or `options` model"*.
///
/// **What separates a value from an option is who draws it.**
///
/// An option's row is drawn by code shared by every node in the library. That code knows four
/// shapes: a select, a file button, a tick, a one-line field. Those four are all there will
/// ever be, which is what makes an option portable between nodes.
///
/// A value is drawn by the node's own code, so the shapes stay open: a two-column range
/// editor, a box of prose, a recorded curve, a painting, and later a pad and a step grid.
///
/// **A node has to declare each value up front**, through
/// [`ValueDef`](crate::nodes::ValueDef), which silvia's do not.
///
/// Two reasons. `ui/` is not allowed to check which node it is drawing, and that rule is what
/// keeps node-specific drawing out of a module every node shares. And undo, addressing and
/// the save file all need to know what a node holds without running it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Value {
    /// Free-running text. `note`'s whole reason to exist.
    Text(String),
    /// A curve a hand performed, oldest first: `automation`'s recording. The transport that
    /// made it is runtime state and is not here — see docs/cpu.md.
    Points(Vec<Point>),
    /// A grid of cells a hand lit, one string per lane, `x` for a lit cell and `.` for an
    /// unlit one — `stepsequencer`'s pattern, in the notation every rhythm table uses. A lane
    /// shorter than the grid, or a character that is not `x`, is unlit.
    Cells(Vec<String>),
    /// A number control's ends and quantum on this instance, keyed by the input it belongs
    /// to rather than by a [`ValueDef`] — every number control can have one, so there is
    /// nothing for a definition to declare.
    Range(ControlRange),
    /// A picture a hand painted: `drawingcanvas`'s. The pixels in memory, and a picture file
    /// in the project's `assets/` on disk — see [`Painting`].
    Painting(Painting),
}

/// A picture a hand painted, `drawingcanvas`'s: RGBA8, rows top first.
///
/// **In memory it is the pixels, shared.** A snapshot in the undo ring, a pasted copy and the
/// synth's graph all hold the one `Arc` until one of them paints, and the frame the node
/// publishes is that same `Arc`, so the picture reaches the renderer without a copy.
///
/// **In a file it is the name of a PNG** in the project's `assets/`, named by what is in it:
/// `assets/painting-<print>.png`, sixteen hex digits of an FNV-1a over the size and the bytes.
/// Named by content, a file is never overwritten — a painting that changed is a new name —
/// so the project's save writes one before the manifest that names it commits, and a save cut
/// off anywhere opens as the last save whole. A `.ssw` read back holds the name alone,
/// [unread](Self::is_read), until the project that holds the file reads it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(into = "PaintingFile", from = "PaintingFile")]
pub struct Painting {
    /// `Pixels::Bytes`, always; a frame of no size for a painting that is still a name.
    frame: Arc<Frame>,
    /// Which stroke last finished on it: a number no earlier stroke in this run was given, or
    /// zero for a painting a file brought. Runtime, and not written: what makes the node's
    /// `strokeDone` fire once per stroke, and not on an undo back to an older picture.
    stroke: u64,
    /// The reference, worked out once from the pixels, or the one a file named.
    file: Arc<OnceLock<String>>,
}

/// The form a [`Painting`] takes in a `.ssw`: the reference to its picture file.
#[derive(serde::Serialize, serde::Deserialize)]
struct PaintingFile {
    file: String,
}

impl From<Painting> for PaintingFile {
    fn from(painting: Painting) -> Self {
        Self {
            file: painting.file().to_string(),
        }
    }
}

impl From<PaintingFile> for Painting {
    fn from(saved: PaintingFile) -> Self {
        Self::named(saved.file)
    }
}

/// Where a painting's file lives: the project's `assets/`, which is `project::ASSETS`.
/// `tests/painting.rs` holds the two to one name.
pub const PAINTING_FOLDER: &str = "assets";

impl Painting {
    /// Pixels a hand painted, `width × height × 4` bytes of straight RGBA8, rows top first.
    ///
    /// # Panics
    /// The bytes are not that many — a painting of the wrong length is a caller's bug and would
    /// otherwise reach the renderer as a garbled upload.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>, stroke: u64) -> Self {
        assert_eq!(
            rgba.len(),
            width as usize * height as usize * 4,
            "a {width}x{height} painting"
        );
        Self {
            frame: Arc::new(Frame {
                width,
                height,
                pixels: crate::nodes::Pixels::Bytes(rgba),
            }),
            stroke,
            file: Arc::new(OnceLock::new()),
        }
    }

    /// A painting a file names and nothing has read: what a `.ssw` holds until its project
    /// reads the picture. It draws as a blank canvas and saves as the name it was given.
    pub fn named(file: String) -> Self {
        Self {
            frame: Arc::new(Frame {
                width: 0,
                height: 0,
                pixels: crate::nodes::Pixels::Bytes(Vec::new()),
            }),
            stroke: 0,
            file: Arc::new(OnceLock::from(file)),
        }
    }

    /// Whether the pixels are here, rather than only a file's name.
    pub fn is_read(&self) -> bool {
        self.frame.width > 0 && self.frame.height > 0
    }

    /// The picture as the frame the node publishes, where it has been read.
    pub fn frame(&self) -> Option<&Arc<Frame>> {
        self.is_read().then_some(&self.frame)
    }

    /// Width, height and the bytes, where it has been read.
    pub fn pixels(&self) -> Option<(u32, u32, &[u8])> {
        let frame = self.frame()?;
        Some((frame.width, frame.height, frame.bytes()?))
    }

    /// Which stroke last finished on it. See the field.
    pub fn stroke(&self) -> u64 {
        self.stroke
    }

    /// The same pixels, finished by another stroke. Shares the pixels and the file name.
    #[must_use]
    pub fn with_stroke(&self, stroke: u64) -> Self {
        Self {
            stroke,
            ..self.clone()
        }
    }

    /// The reference its picture file goes by, `assets/painting-<print>.png`. Worked out once
    /// per picture: every clone shares the answer.
    pub fn file(&self) -> &str {
        self.file.get_or_init(|| {
            let mut hash = 0xcbf2_9ce4_8422_2325_u64;
            let mut eat = |bytes: &[u8]| {
                for b in bytes {
                    hash ^= u64::from(*b);
                    hash = hash.wrapping_mul(0x0100_0000_01b3);
                }
            };
            eat(&self.frame.width.to_le_bytes());
            eat(&self.frame.height.to_le_bytes());
            eat(self.frame.bytes().unwrap_or_default());
            format!("{PAINTING_FOLDER}/painting-{hash:016x}.png")
        })
    }

    /// The bytes it holds, for the undo ring's budget.
    pub fn bytes(&self) -> usize {
        self.frame.bytes().map_or(0, <[u8]>::len)
    }

    /// Whether two paintings are one picture in memory, which is what a snapshot beside the
    /// graph it came from has when nothing painted in between.
    pub fn shares(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
    }
}

/// Equal pictures, finished by the same stroke. Two read pictures compare their pixels — the
/// same `Arc` first, which is what almost every comparison is — and a picture that is still a
/// name compares by the name.
impl PartialEq for Painting {
    fn eq(&self, other: &Self) -> bool {
        if self.stroke != other.stroke {
            return false;
        }
        match (self.is_read(), other.is_read()) {
            (true, true) => self.shares(other) || self.frame == other.frame,
            _ => self.file() == other.file(),
        }
    }
}

/// A size and a name rather than a megabyte of bytes.
impl std::fmt::Debug for Painting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_read() {
            write!(
                f,
                "Painting({}x{}, stroke {})",
                self.frame.width, self.frame.height, self.stroke
            )
        } else {
            write!(f, "Painting(unread {})", self.file())
        }
    }
}

/// One point of a recorded curve: how far into the recording it was made, and what the value
/// was there.
///
/// Time in seconds from the start of the recording, value normalized to 0..1 — the ends the
/// curve is mapped into are knobs on the node, so moving them re-reads the same recording
/// rather than rewriting it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Point {
    pub time: f32,
    pub value: f32,
}

impl Value {
    /// The range, where this value is one. Saves every reader a `matches!`.
    pub fn range(&self) -> Option<ControlRange> {
        match self {
            Self::Range(r) => Some(*r),
            Self::Text(_) | Self::Points(_) | Self::Cells(_) | Self::Painting(_) => None,
        }
    }

    /// The text, where this value is some.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            Self::Range(_) | Self::Points(_) | Self::Cells(_) | Self::Painting(_) => None,
        }
    }

    /// The recorded curve, where this value is one.
    pub fn points(&self) -> Option<&[Point]> {
        match self {
            Self::Points(p) => Some(p),
            Self::Text(_) | Self::Range(_) | Self::Cells(_) | Self::Painting(_) => None,
        }
    }

    /// The painted picture, where this value is one.
    pub fn painting(&self) -> Option<&Painting> {
        match self {
            Self::Painting(p) => Some(p),
            Self::Text(_) | Self::Range(_) | Self::Points(_) | Self::Cells(_) => None,
        }
    }

    /// The grid's lanes, where this value is one.
    pub fn cells(&self) -> Option<&[String]> {
        match self {
            Self::Cells(c) => Some(c),
            Self::Text(_) | Self::Range(_) | Self::Points(_) | Self::Painting(_) => None,
        }
    }

    /// Whether one cell of a grid is lit: an `x` at `step` of `lane`. Anything else, a cell
    /// past the end of its lane and a value that is not a grid are unlit.
    pub fn lit(&self, lane: usize, step: usize) -> bool {
        self.cells()
            .and_then(|c| c.get(lane))
            .and_then(|l| l.as_bytes().get(step))
            == Some(&b'x')
    }

    /// A grid of `lanes` by `steps`, each cell lit where `lit(lane, step)` says.
    pub fn grid(lanes: usize, steps: usize, lit: impl Fn(usize, usize) -> bool) -> Self {
        Self::Cells(
            (0..lanes)
                .map(|l| {
                    (0..steps)
                        .map(|s| if lit(l, s) { 'x' } else { '.' })
                        .collect()
                })
                .collect(),
        )
    }
}

/// One node in the graph.
///
/// The port lists are copied from the node definition at insertion, because a dual port's
/// effective type is this instance's rather than its kind's. Everything else about the kind is
/// read through `def`, which is a pointer and not a copy.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Which kind of node this is: its definition in the registry, held rather than looked up,
    /// so every reader has the kind without a search. A slug becomes one only where it
    /// arrives as a string, and `def.is_output` is read here so nothing outside `nodes/`
    /// matches on a slug.
    pub def: &'static NodeDef,
    /// Position on the canvas. One position, whichever workspace it is seen on.
    pub pos: Pos2,
    /// The workspaces this node is on. Never empty, and every id names a live workspace.
    ///
    /// A workspace is a view, so visibility is layout in the same sense `pos` is: it is
    /// saved with the project and it is undoable. It is not a control and not an option, so it
    /// stays outside `controls` and the address space.
    pub workspaces: BTreeSet<WorkspaceId>,
    pub inputs: Vec<PortDef>,
    pub outputs: Vec<PortDef>,
    /// A body width a hand set, for the node kinds that let one be dragged — the note, so
    /// far, which is silvia's one resizable node too.
    ///
    /// Document data, like a position: it describes the graph rather than the session, so it
    /// goes through the bus, rides in the file and is undoable. `None` is the width the kind
    /// asks for, which is what every node that was never dragged has.
    ///
    /// Distinct from `NodeDef::width`, which is the same on every node of a kind. This is the
    /// one node's own, and it never goes below the kind's.
    pub dragged_width: Option<f32>,
    /// The height a hand set for the node's text box, for the kinds that hold one — the note
    /// and the Text node. Document data for the same reasons a width is. `None` is the box
    /// its kind declares, in lines; either way the box keeps its height whatever is typed,
    /// and its text scrolls inside it.
    pub dragged_height: Option<f32>,
    /// Current value of each unconnected control, keyed by input port.
    ///
    /// This is the single source of truth for a parameter. The renderer reads it each frame
    /// to set a uniform; the UI writes it. There is no second copy to keep in sync.
    pub controls: HashMap<&'static str, ControlValue>,
    /// What this node keeps for itself. Two things live here, under one map.
    ///
    /// A control's range, where this node's differs from its definition's, stored under the
    /// name of the input port it belongs to. And every value the definition declares, stored
    /// under its own name.
    ///
    /// One map rather than two because they are the same idea — see [`Value`]. A registry
    /// test stops a node declaring a value named after one of its own ports, which is the
    /// only way the two could collide.
    ///
    /// Usually empty. Almost no control in almost any graph moves its range, and almost no
    /// node declares a value at all.
    pub values: BTreeMap<&'static str, Value>,
    /// Current value of each select option, keyed by option key.
    ///
    /// Ordered: the canvas indexes option rows by iteration order, and `HashMap`'s is
    /// unspecified, so rows could swap between runs.
    pub options: BTreeMap<&'static str, String>,
    /// Drawn as a header and nothing else. Document data, like a position: it describes the
    /// graph rather than the session, so it rides in the file and through undo.
    ///
    /// Nothing about the graph changes — the ports still exist, the cables still land, and
    /// the shader is untouched. Only `canvas::rows` yields nothing.
    pub collapsed: bool,
}

impl Node {
    pub fn input(&self, key: &str) -> Option<&PortDef> {
        self.inputs.iter().find(|p| p.key == key)
    }

    pub fn output(&self, key: &str) -> Option<&PortDef> {
        self.outputs.iter().find(|p| p.key == key)
    }
}
