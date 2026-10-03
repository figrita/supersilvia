// SPDX-License-Identifier: AGPL-3.0-or-later

//! Port types and the shape of a connection. Pure data.

use crate::graph::NodeId;

/// The port types: four that carry data, on two axes, and the event.
///
/// A data port has a [`Kind`] — a number or a color — and a [`Rate`]. `Fragment` is one
/// value per pixel, `float f(vec2 uv)` or `vec4 f(vec2 uv)`, meaningful only inside a
/// shader. `Uniform` is one value per frame, held on the CPU and uploaded as a uniform.
/// `Action` is an event and carries no value. All four cells of kind × rate are filled, so
/// everything here reads a kind and a rate rather than matching a variant.
///
/// Connections match exactly, with one asymmetry that is physics rather than policy: a
/// uniform may feed a varying input of its own kind, because broadcasting one value across
/// every pixel costs nothing — it is a uniform, which is what an unconnected control already
/// is. The reverse needs a render pass and a readback, and is a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortType {
    VaryingNumber,
    VaryingColor,
    UniformNumber,
    UniformColor,
    Action,
}

/// What a port carries, whatever its rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Number,
    Color,
    Event,
}

/// How often a data port's value varies: once per pixel, or once per frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rate {
    Varying,
    Uniform,
}

impl PortType {
    pub fn kind(self) -> Kind {
        match self {
            Self::VaryingNumber | Self::UniformNumber => Kind::Number,
            Self::VaryingColor | Self::UniformColor => Kind::Color,
            Self::Action => Kind::Event,
        }
    }

    /// `None` for an event, which has no value to vary.
    pub fn rate(self) -> Option<Rate> {
        match self {
            Self::VaryingNumber | Self::VaryingColor => Some(Rate::Varying),
            Self::UniformNumber | Self::UniformColor => Some(Rate::Uniform),
            Self::Action => None,
        }
    }

    pub fn is_varying(self) -> bool {
        self.rate() == Some(Rate::Varying)
    }

    pub fn is_uniform(self) -> bool {
        self.rate() == Some(Rate::Uniform)
    }

    /// The same kind at the other rate. An event is itself, having no rate to move between.
    #[must_use]
    pub fn with_rate(self, rate: Rate) -> Self {
        match (self.kind(), rate) {
            (Kind::Number, Rate::Varying) => Self::VaryingNumber,
            (Kind::Number, Rate::Uniform) => Self::UniformNumber,
            (Kind::Color, Rate::Varying) => Self::VaryingColor,
            (Kind::Color, Rate::Uniform) => Self::UniformColor,
            (Kind::Event, _) => self,
        }
    }

    /// The type as a person reads it: on a port's hover, in a locator, in an error.
    pub fn name(self) -> &'static str {
        match self {
            Self::VaryingNumber => "varying number",
            Self::VaryingColor => "varying color",
            Self::UniformNumber => "uniform number",
            Self::UniformColor => "uniform color",
            Self::Action => "action",
        }
    }

    /// Data ports carry a value. Action ports carry an event on the CPU and obey different
    /// connection rules: many-to-many, and no cycle check.
    pub fn is_data(self) -> bool {
        !matches!(self, Self::Action)
    }

    /// Ports whose value lives on the CPU rather than in a shader.
    pub fn is_cpu(self) -> bool {
        self.is_uniform() || !self.is_data()
    }

    /// May an output of this type feed an input of `to`?
    pub fn feeds(self, to: Self) -> bool {
        self == to || (self.kind() == to.kind() && self.is_uniform() && to.is_varying())
    }
}

/// One port on a node kind, as far as the graph is concerned. Labels, controls and code
/// generation live with the node definition; the graph only needs identity, type, and
/// whether reading it crosses a frame boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortDef {
    pub key: &'static str,
    pub ty: PortType,
    /// True when consuming this output samples a texture rather than recursing into the
    /// producer, or reads a uniform number the node's CPU half already published a frame
    /// after the shader measured it. Every `OutputKind::Texture` output is one: an Output's
    /// published `frame`, and a camera's or a clip's published capture. So is a tap's, a
    /// sample's and an `autoexposure`'s `UniformNumber` output, by `OutputDef::delayed`.
    ///
    /// The name says *previous frame*, literally true of an Output's `frame` and of a
    /// measurement's reading, and figuratively true of a camera's, whose texture is the
    /// newest frame it has. For a `Texture` port the flag decides termination: the compiler
    /// does not descend through one, it samples a texture binding instead, which is why a
    /// loop through one terminates and is legal where an immediate loop is not. A
    /// `UniformNumber` output is never descended into either way — the compiler resolves it
    /// to a uniform before a generator could run — so there the flag decides only the
    /// cycle check: a loop through one is feedback rather than recursion. §2b: cycles through
    /// a frame port *are* feedback, and a delayed uniform number's read the same way.
    pub delayed: bool,
    /// True for a port whose declared type is `VaryingNumber` and whose **effective** type
    /// is whatever the graph around it makes it. `ty` on this instance is that answer,
    /// recomputed by `Graph` on every structural change.
    ///
    /// On an **output** it is `UniformNumber` while every input of its node resolves to a
    /// uniform number and `VaryingNumber` otherwise. On an **input** of that same node it is
    /// `UniformNumber` while the node is *pinned* — in `UniformNumber` mode with a flipped
    /// output that would feed a `UniformNumber` input — because a `VaryingNumber` landing
    /// there is refused, and the type a port draws is the type a cable may bring.
    ///
    /// The second bit `nodes/` hands the graph beside `delayed`, and it comes from
    /// `OutputDef::eval` — the formula a `tick` can evaluate — so nothing here matches on a
    /// slug to find one. `graph/` needs it for three questions: which outputs to recompute,
    /// which inputs to pin, and which refusal a file may settle rather than obey. See
    /// `docs/nodes.md#dual-outputs`.
    pub dual: bool,
}

impl PortDef {
    pub const fn new(key: &'static str, ty: PortType) -> Self {
        Self {
            key,
            ty,
            delayed: false,
            dual: false,
        }
    }

    /// A port whose value is last frame's.
    pub const fn delayed(key: &'static str, ty: PortType) -> Self {
        Self {
            key,
            ty,
            delayed: true,
            dual: false,
        }
    }

    /// A port whose type is decided by the graph rather than by its own declaration. `ty` is
    /// the declared one until `Graph` works the effective one out.
    pub const fn dual(key: &'static str, ty: PortType) -> Self {
        Self {
            key,
            ty,
            delayed: false,
            dual: true,
        }
    }
}

/// A specific port on a specific node instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortRef {
    pub node: NodeId,
    pub key: &'static str,
}

impl PortRef {
    pub const fn new(node: NodeId, key: &'static str) -> Self {
        Self { node, key }
    }
}

/// A directed edge: an output port feeding an input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connection {
    pub from: PortRef,
    pub to: PortRef,
}

/// Why a connection was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    NoSuchNode(NodeId),
    /// The port key does not exist on that node, or exists in the other direction.
    NoSuchPort(PortRef),
    /// Both ports are data but the output cannot feed the input. `UniformNumber` into
    /// `VaryingNumber` is the one pairing of different types that can.
    ///
    /// A pinned dual node's input *is* `UniformNumber`, so a field dropped on one is this
    /// and not a rule of its own: the port says what it takes before a hand drags anything
    /// at it.
    TypeMismatch {
        from: PortType,
        to: PortType,
    },
    /// One side is an action port and the other is not.
    ActionMismatch {
        from: PortType,
        to: PortType,
    },
    /// A node cannot feed itself.
    SelfConnection(NodeId),
    /// The edge would close a loop in the data graph.
    WouldCycle {
        from: NodeId,
        to: NodeId,
    },
    /// The input is the time row its node's mode puts away — a Time while the node runs free,
    /// a Speed while it loops — which has no row to land on (`nodes::is_inactive`).
    Inactive(PortRef),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchNode(id) => write!(f, "no such node: {id:?}"),
            Self::NoSuchPort(p) => write!(f, "no such port: {:?}.{}", p.node, p.key),
            Self::TypeMismatch { from, to } => {
                write!(f, "type mismatch: {from:?} output into {to:?} input")
            }
            Self::ActionMismatch { from, to } => {
                write!(
                    f,
                    "action ports only connect to action ports: {from:?} into {to:?}"
                )
            }
            Self::SelfConnection(id) => write!(f, "node {id:?} cannot connect to itself"),
            Self::WouldCycle { from, to } => {
                write!(f, "connecting {from:?} into {to:?} would create a cycle")
            }
            Self::Inactive(p) => write!(
                f,
                "{:?}.{} is put away by the node's time mode",
                p.node, p.key
            ),
        }
    }
}

impl std::error::Error for ConnectError {}
