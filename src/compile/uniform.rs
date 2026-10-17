// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where a uniform's value comes from each frame.

use crate::graph::NodeId;

/// The type of a generated uniform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UniformType {
    Float,
    Int,
    Vec2,
    Vec4,
    /// A number list: `array<vec4f, LIST_VEC4S>`, its length in the first element's `.x` and
    /// its items packed four to an element from the second on.
    List,
    Sampler2D,
}

/// How many `vec4f`s a list uniform holds: one for the length, and [`crate::nodes::list::CAP`]
/// items packed four to each after it.
pub const LIST_VEC4S: usize = 1 + crate::nodes::list::CAP / 4;

impl UniformType {
    /// Its type as a member of a WGSL module's uniform struct, or `None` for a texture,
    /// which is a binding of its own rather than a member.
    pub fn wgsl(self) -> Option<&'static str> {
        match self {
            Self::Float => Some("f32"),
            Self::Int => Some("i32"),
            Self::Vec2 => Some("vec2f"),
            Self::Vec4 => Some("vec4f"),
            Self::List => Some(LIST_WGSL),
            Self::Sampler2D => None,
        }
    }
}

/// The most colors of a color list a shader sees: one `vec4f` each after the length, in the
/// same array a number list packs four to an element.
pub const COLOR_LIST_CAP: usize = LIST_VEC4S - 1;

/// A list uniform's WGSL type. Kept in step with [`LIST_VEC4S`] by a test.
pub const LIST_WGSL: &str = "array<vec4f, 257>";

/// Who supplies a uniform's value. The renderer walks these once per frame.
///
/// This is the whole reason a parameter edit costs a few bytes of the tick's uniform block
/// rather than a shader rebuild: an unconnected control becomes a uniform with a pointer back to the control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniformProvider {
    /// An unconnected input's control value, read from the node each frame.
    Control {
        node: NodeId,
        key: &'static str,
        ty: UniformType,
    },
    /// A texture published by a node's output port, such as an Output's `frame`.
    NodeTexture { node: NodeId, port: &'static str },
    /// A uniform published by a CPU node's output port, read from `Synth::tick`'s results
    /// each frame. The same shape as `Control`: one value, written into the uniform block, and
    /// `ty` says which of the two kinds it is — a `Float` for a uniform number, the `f32` of
    /// the `f64` it was published as, a `Vec4` for a uniform color.
    NodeUniform {
        node: NodeId,
        port: &'static str,
        ty: UniformType,
    },
    /// A number a node published, as a Time reads it: `vec2f(whole, fraction)`, the `f64`'s
    /// whole part wrapped at `phasor::WHOLE_WRAP` and the `f32` of its fraction
    /// (`nodes::phasor::split`) — a gear's Cycles, the Time node's Seconds, an unplugged
    /// Time's ambient reading, or any other number cabled in.
    NodeCount { node: NodeId, port: &'static str },
    /// A number list a CPU node published, carried whole in the uniform block: a shader reads
    /// an item by index, and a length change rebuilds nothing.
    NodeList { node: NodeId, port: &'static str },
    /// The index of an `OptionKind::Uniform` option's chosen value among its choices.
    ///
    /// What makes a discrete choice cost one integer in the uniform block rather than a
    /// rebuild: the branches
    /// are all in the program and this says which one runs.
    Option { node: NodeId, key: &'static str },
}

impl UniformProvider {
    pub fn ty(&self) -> UniformType {
        match self {
            Self::NodeTexture { .. } => UniformType::Sampler2D,
            Self::Option { .. } => UniformType::Int,
            Self::NodeCount { .. } => UniformType::Vec2,
            Self::NodeList { .. } => UniformType::List,
            // A control and a published uniform each carry their own: both are one value a
            // frame, and the kind — a number or a color — is what decides the WGSL type.
            Self::Control { ty, .. } | Self::NodeUniform { ty, .. } => *ty,
        }
    }
}
