// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the editor describes and the synth draws, once per edit rather than once per tick.
//!
//! The synth ticks on its own clock and the editor paints on the compositor's, so what crosses
//! is a **plan** — the shaders the compiler built, where each uniform's value comes from, the
//! decks and which Outputs draw — which the synth turns into a [`crate::render::FrameJob`] on
//! every tick. The fade and the crossfade method are not in it: a hand on the fade moves them
//! every frame and nothing a plan works out depends on them, so they cross on their own, as
//! [`super::Msg::Fade`].
//!
//! **The uniforms are providers, not values.** Resolving one reads the graph and what the last
//! tick published, both the synth's; resolved on the editor's frame, every uniform would be a
//! tick stale. So the plan carries [`crate::compile::UniformProvider`]s.
//!
//! **The shader is one-shot.** `shader: Some(module)` means *this was just compiled*; the
//! synth takes it out of the plan on the tick it sends it, so a plan reused for a thousand
//! ticks uploads a program once. It is sent only with a job that does not suspend its Output,
//! because the renderer ignores a suspended Output's source.

use crate::compile::UniformProvider;
use crate::graph::{NodeId, PortRef, PortType};
use crate::nodes::{TextureFilter, TextureWrap};
use crate::render::Display;
use std::collections::HashMap;
use std::sync::Arc;

/// Why an Output draws on every tick. The rule is
/// [docs/rendering.md](../../docs/rendering.md#which-outputs-draw).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// It is on the workspace being looked at.
    Tab,
    /// It is on a mixer deck.
    Deck,
    /// Its picture has a window of its own.
    Window,
    /// It is sent out, over Syphon to another app on the Mac or over NDI to the network.
    Sent,
    /// The offline render is capturing it.
    Capture,
    /// It is in a cycle of data edges, delayed ones included: its picture depends on its own
    /// earlier frames.
    Feedback,
    /// Its frame is read by this Output, which draws.
    Frame(NodeId),
    /// This node's measurement samples its frame, and something live or stateful reads the
    /// reading.
    Tap(NodeId),
}

/// What an Output does on every tick the plan stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Shown only on closed workspaces: not built and not drawn, its targets, program and
    /// last frame kept.
    Suspended,
    /// Awake with nothing connected: black, and nothing to draw.
    Dark,
    /// Awake with a shader, not drawn: it publishes its last frame until something asks a
    /// picture of it.
    Idle,
    /// Drawn on every tick, for `why`.
    Draw { why: Why },
}

impl Mode {
    /// Why it draws on every tick, or `None` where it does not.
    pub fn why(self) -> Option<Why> {
        match self {
            Self::Draw { why } => Some(why),
            Self::Suspended | Self::Dark | Self::Idle => None,
        }
    }
}

/// One Output, as the editor describes it.
#[derive(Debug, Clone)]
pub struct OutputPlan {
    pub node: NodeId,
    pub resolution: (u32, u32),
    /// The WGSL module, one-shot: see the module doc.
    pub shader: Option<Arc<crate::compile::Shader>>,
    /// The hash of the source `uniforms`, `taps` and `tap_nodes` were compiled for
    /// (`compile::source_hash`), sent or not; zero with no shader.
    pub source: u64,
    pub mode: Mode,
    /// The Outputs whose `frame` its shader samples, itself included where it feeds back.
    pub reads: Vec<NodeId>,
    /// Every other Output that draws on a tick this one draws: the frames and measurements it
    /// reads, closed over both, so a picture asked of it reads this tick's.
    pub needs: Vec<NodeId>,
    /// It is in a cycle of data edges, its picture made of its own earlier frames: it draws
    /// only on a tick the playhead moved, so pause freezes the loop.
    pub feeds_back: bool,
    pub uniforms: Vec<(Arc<str>, UniformProvider)>,
    pub taps: usize,
}

/// One workspace's pass, as the editor describes it: `compile::wgsl::build_pass`.
#[derive(Debug, Clone)]
pub struct PassPlan {
    /// Its workspace, and which of that workspace's batches.
    pub key: crate::render::PassKey,
    /// The module, one-shot, as an Output's is.
    pub shader: Option<Arc<crate::compile::Shader>>,
    /// As [`OutputPlan::source`].
    pub source: u64,
    /// The whole target: the measurements' square and the thumbnails beside it.
    pub resolution: (u32, u32),
    /// The measurements' square, from the corner: all that is drawn where only `measures`.
    pub region: (u32, u32),
    pub uniforms: Vec<(Arc<str>, UniformProvider)>,
    /// How many tap slots the buffer holds, the thumbnails' words included.
    pub taps: usize,
    /// Which node owns each tap slot, in slot order, so a reading is routed without the
    /// compiled shader crossing.
    pub tap_nodes: Vec<NodeId>,
    /// The word the first thumbnail starts at: the one after the last tap slot.
    pub thumb_base: usize,
    /// Which port each thumbnail is, in tile order.
    pub thumbs: Vec<(PortRef, PortType)>,
    /// Its thumbnails are drawn on every tick: the workspace being looked at.
    pub shown: bool,
    /// Its measurements are drawn on every tick: something reads one of them. See
    /// `docs/rendering.md` on which Outputs draw.
    pub measures: bool,
    /// The Outputs whose picture reads one of its measurements: it draws on a tick any of
    /// them does, as a deck the synth claims does before the editor replans.
    pub wanted_by: Vec<NodeId>,
}

/// One cost probe, on the same terms.
#[derive(Debug, Clone)]
pub struct ProbePlan {
    pub output: NodeId,
    pub shader: Option<Arc<crate::compile::Shader>>,
    /// As [`OutputPlan::source`].
    pub source: u64,
    pub uniforms: Vec<(Arc<str>, UniformProvider)>,
    pub taps: usize,
}

/// What a texture output declared about its own sampling, since `render/` may not read the
/// registry and the synth's frames are keyed by port rather than by node.
#[derive(Debug, Clone, Copy)]
pub struct Sampling {
    pub wrap: TextureWrap,
    pub filter: TextureFilter,
}

/// The mix as a plan states it: which Output is on each deck, and how large the mix is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mix {
    pub a: Option<NodeId>,
    pub b: Option<NodeId>,
    pub resolution: (u32, u32),
}

impl Default for Mix {
    fn default() -> Self {
        Self {
            a: None,
            b: None,
            resolution: crate::nodes::output::DEFAULT_RESOLUTION,
        }
    }
}

/// Everything the editor says about the frame, replaced whenever any of it changes.
#[derive(Debug, Default, Clone)]
pub struct Plan {
    /// Which plan this is, so a one-shot shader is not re-sent against a snapshot that
    /// predates it. See [`crate::synth::RenderReport::plan_generation`].
    pub generation: u64,
    pub outputs: Vec<OutputPlan>,
    pub probes: Vec<ProbePlan>,
    /// One per open workspace with something to draw, and per closed one measuring what a
    /// deck or a send keeps awake.
    pub passes: Vec<PassPlan>,
    pub mixer: Mix,
    pub display: Display,
    /// How each published texture wants to be sampled.
    pub sampling: HashMap<PortRef, Sampling>,
}
