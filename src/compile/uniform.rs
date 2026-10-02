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
    Sampler2D,
}

impl UniformType {
    /// Its type as a member of a WGSL module's uniform struct, or `None` for a texture,
    /// which is a binding of its own rather than a member.
    pub fn wgsl(self) -> Option<&'static str> {
        match self {
            Self::Float => Some("f32"),
            Self::Int => Some("i32"),
            Self::Vec2 => Some("vec2f"),
            Self::Vec4 => Some("vec4f"),
            Self::Sampler2D => None,
        }
    }
}

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
    /// `ty` says which of the two kinds it is — a `Float` for a uniform number, a `Vec4` for
    /// a uniform color.
    NodeUniform {
        node: NodeId,
        port: &'static str,
        ty: UniformType,
    },
    /// A count a node published, as a Time reads it: `vec2f(whole, fraction)`, the whole part
    /// wrapped at `phasor::WHOLE_WRAP` and the `f32` of the fraction
    /// (`nodes::phasor::split`). A gear's Cycles, the Time node's Seconds and an unplugged
    /// Time's ambient reading are counts published whole; anything else in a Time is its one
    /// `f32` split the same way.
    NodeCount { node: NodeId, port: &'static str },
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
            // A control and a published uniform each carry their own: both are one value a
            // frame, and the kind — a number or a color — is what decides the WGSL type.
            Self::Control { ty, .. } | Self::NodeUniform { ty, .. } => *ty,
        }
    }
}
