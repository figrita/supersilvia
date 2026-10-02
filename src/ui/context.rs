// SPDX-License-Identifier: AGPL-3.0-or-later

//! What one canvas pass reads, what it asks for, and what one node is drawn with.
//!
//! `ui::show` is handed a [`CanvasFrame`], borrowed and never written, and hands back
//! [`Effects`], which `App` applies in one place. Each node on screen is drawn from a
//! [`NodeCtx`]. Nothing here holds state: what outlives a frame is `CanvasState`'s.
//!
//! Three types rather than thirty arguments, so which widget wins a click and which answer
//! reaches the app are not spread across parameter order and out-parameters. See docs/ui.md,
//! "One pass".

use crate::command::Command;
use crate::graph::{Graph, Node, NodeId, PortRef, WorkspaceId};
use crate::ui::canvas::{NodeLayout, Transform};
use crate::ui::{
    Background, CanvasPrefs, ClipAction, Cost, LiveSource, NodeDrag, OutputReadout, PopOutRequest,
    Popped, RenderRequest, RenderView, Thumbnail, Uniforms,
};
use eframe::egui::{Pos2, Rect};
use std::collections::{BTreeSet, HashMap};

/// Everything the canvas reads this frame. Built by `App` once a frame; `ui/` never writes it.
// Independent facts about the frame, each read in its own place — the ground, the clipboard,
// the Nodes menu — not the states of one machine.
#[allow(clippy::struct_excessive_bools)]
pub struct CanvasFrame<'a> {
    pub graph: &'a Graph,
    /// The workspace on screen.
    pub workspace: WorkspaceId,
    /// What each CPU node said about itself on the last tick.
    pub notes: &'a HashMap<NodeId, crate::nodes::NodeNote>,
    pub scopes: &'a HashMap<NodeId, crate::audio::Scope>,
    /// Where each playing node has got to, for the scrubber on its picture: only a node whose
    /// `CpuNode::playhead` answers.
    pub playheads: &'a HashMap<NodeId, f32>,
    /// What a node like `adsr` published recently, for the trace on its body: only a node
    /// whose `CpuNode::trace` answers.
    pub traces: &'a HashMap<NodeId, crate::nodes::cpu::TraceRing>,
    /// The cells under that trace, for a node that captions it: `adsr`'s gate and stage.
    pub captions: &'a HashMap<NodeId, Vec<(&'static str, String)>>,
    /// A transport's curve as it is performed, like `automation`'s: only a node whose
    /// `CpuNode::curve` answers.
    pub curves: &'a HashMap<NodeId, crate::nodes::cpu::Curve>,
    /// A pad's puck and what pulls on it, like `xypad`'s: only a node whose `CpuNode::puck`
    /// answers.
    pub pucks: &'a HashMap<NodeId, crate::nodes::cpu::Puck>,
    /// What each gear is doing: only a node whose `CpuNode::gear` answers.
    pub gears: &'a HashMap<NodeId, crate::nodes::gear::Reading>,
    /// What a loop of each Master Gear on screen needs, in words: `nodes::chain::caption`.
    pub gear_captions: &'a HashMap<NodeId, String>,
    /// What every uniform number output published on the last tick.
    pub uniforms: Uniforms<'a>,
    /// Each varying output's thumbnail, from the last frame the GPU finished.
    pub thumbs: &'a HashMap<PortRef, std::sync::Arc<crate::synth::PortThumb>>,
    /// The workspaces with a tab, which is where a tag can go without opening one.
    pub tabs: &'a BTreeSet<WorkspaceId>,
    pub assets: &'a [crate::project::AssetInfo],
    /// Each asset's poster, by reference, as `App` has decoded them so far.
    pub posters: &'a HashMap<String, Option<eframe::egui::TextureHandle>>,
    pub theme: &'a crate::ui::theme::Theme,
    /// The mix is painted behind the canvas: reserve it a slot instead of painting a ground.
    pub project_background: bool,
    /// The window is cleared to the canvas's ground color before anything is painted, so the
    /// ground is already there and painting it again would be a screenful of blending for
    /// nothing. See [`crate::ui::theme::clear_color`].
    pub cleared: bool,
    /// The cost strip under each node, while the cost view is on. Empty otherwise.
    pub costs: &'a HashMap<NodeId, Cost>,
    /// Every Output's status line, every frame, whatever the Costs preference: it is not a
    /// measurement.
    pub readouts: &'a HashMap<NodeId, OutputReadout>,
    /// What drives each control, so a bound one wears a mark and the range editor can offer
    /// to forget it.
    pub bindings: &'a crate::midi::Bindings,
    /// The control waiting for a MIDI message, if one is: it wears the learning ring.
    pub learning: Option<crate::midi::Target>,
    /// Every control whose fader soft takeover holds out of pick-up, and where that fader
    /// is: it wears a ghost mark there.
    pub ghosts: &'a [(crate::midi::Target, f32)],
    /// How many times a node measurably samples its own input, only where it earns a header
    /// warning, whatever the Costs preference: the probe always runs.
    pub sampling: &'a HashMap<NodeId, f64>,
    /// Every node at fault and why — a shader that failed, a device that would not open or
    /// stopped answering: the flag on its header and the flag's hover.
    pub faults: &'a HashMap<NodeId, String>,
    /// The sources upstream of each Output that a render cannot step, for its `!`.
    pub live: &'a HashMap<NodeId, Vec<LiveSource>>,
    /// The pictures in windows of their own, so a node can light the mark that put one there.
    pub popped: &'a [Popped],
    /// The render in progress, if one is: its Output's button says so, and every number
    /// control goes inert with the document.
    pub render: Option<RenderView>,
    /// Whether the clipboard holds anything, which is all a Paste entry needs; the clip is
    /// `App`'s.
    pub clipboard: bool,
    /// The Nodes menu is open. It is drawn after the canvas and answers `Escape` itself, so the
    /// canvas's keys leave the keyboard to it.
    pub nodes_menu_open: bool,
    pub prefs: CanvasPrefs,
}

impl CanvasFrame<'_> {
    /// What one node published this frame, for the regions that draw it.
    pub fn live_of(&self, id: NodeId) -> crate::widgets::Live<'_> {
        crate::widgets::Live {
            scope: self.scopes.get(&id),
            trace: self.traces.get(&id),
            caption: self.captions.get(&id).map(Vec::as_slice),
            curve: self.curves.get(&id),
            puck: self.pucks.get(&id),
            gear: self.gears.get(&id),
            gear_caption: self.gear_captions.get(&id).map(String::as_str),
            ghosts: self.ghosts,
            status: self.notes.get(&id).map(|n| n.text.as_str()),
            playhead: self
                .playheads
                .get(&id)
                .copied()
                .or_else(|| self.time_of(id)),
        }
    }

    /// Where a node that moves with time is this frame, in its own cycles: what its Time read
    /// — the cable's, or the ambient reading published under the input's own key — taken
    /// modulo the node's period in `f64` where it arrived as a count, so it is as precise at
    /// any count, with its Offset knob added. `None` for a node that does not move with time,
    /// and before the first tick. A field in Offset has no one value, and adds nothing here.
    ///
    /// The palette strip is what reads it. Time X stands for a node with a Time per axis:
    /// Shaky Cam draws no region that reads where it is.
    fn time_of(&self, id: NodeId) -> Option<f32> {
        let node = self.graph.get(id)?;
        let ambient = node.def.ambient?;
        let time = PortRef::new(id, crate::nodes::TIME);
        let at = self.graph.source_of(time).unwrap_or(time);
        let clock = self
            .uniforms
            .count(at)
            .or_else(|| self.uniforms.get(at).map(f64::from))?;
        let clock = crate::nodes::phasor::fraction(clock, ambient.wrap(node)) as f32;
        let offset = PortRef::new(id, crate::nodes::phasor::OFFSET);
        let knob = match (self.graph.source_of(offset), node.controls.get(offset.key)) {
            (None, Some(crate::graph::ControlValue::Float(v))) => *v,
            _ => 0.0,
        };
        Some(clock + knob)
    }
}

/// Everything one canvas pass asks the app to do.
///
/// `commands` are edits and go to the bus; every other field is a *request*, done to the
/// instrument or the session rather than the document, which `App` performs and which never
/// enters the undo history. `ui/` has no GPU, opens no file dialog, cannot see the tab
/// bar and does not hold the undo ring or the clipboard.
#[derive(Default)]
pub struct Effects {
    /// What the interaction asked for.
    pub commands: Vec<Command>,
    /// Where each picture on a node wants its frame drawn; the caller fills them with paint
    /// callbacks.
    pub thumbnails: Vec<Thumbnail>,
    /// Where a hand dragged a picture's scrubber this frame, 0 to 1 through the clip. The
    /// playhead is the node's state, not the document's, so it goes through
    /// `TickContext::seek`.
    pub seeks: Vec<(NodeId, f32)>,
    /// What a hand did on a node's own surface this frame — a well dropped on a pad, a
    /// preset. A pad's wells are its runtime state, not the document's, so they go through
    /// `TickContext::touches`.
    pub touches: Vec<(NodeId, crate::nodes::cpu::Touch)>,
    /// Pictures whose pop-out or fullscreen mark was clicked this frame.
    pub pop_outs: Vec<PopOutRequest>,
    /// Render buttons clicked this frame.
    pub render_requests: Vec<RenderRequest>,
    /// The `!` popup asked for the Main Input panel: unfold it.
    pub reveal_main_input: bool,
    /// *Look for devices again* in a node's device select: ask the machine for its capture
    /// devices again.
    pub look_for_devices: bool,
    /// Free-text options whose button was clicked, each for a file dialog.
    pub file_requests: Vec<(NodeId, &'static str)>,
    /// Action inputs whose button is under a finger right now. A level rather than a click,
    /// because an action is a gate: the next `tick` turns held-and-then-not into a down and
    /// an up.
    pub held: Vec<PortRef>,
    /// The slot under everything where the mix goes, when it is projected to the background;
    /// `App` fills it with a paint callback. `None` when the canvas painted its own ground.
    pub background: Option<Background>,
    /// A tag was clicked: show this workspace and center it on this node.
    pub navigate: Option<(WorkspaceId, NodeId)>,
    /// A node drag in flight, so a drop onto a tab can be answered: the tab bar is drawn
    /// before the canvas, so the bar reports the tab under the pointer and `App` joins them.
    pub node_drag: Option<NodeDrag>,
    /// A control scrub abandoned with `Escape`: the control, and the value the drag began at.
    /// Putting it back collapses the edit the drag opened rather than making a second.
    pub cancel_control: Option<(NodeId, &'static str, f32)>,
    /// A node drag put back with `Escape`: the nodes it was moving, in the order it moved
    /// them. Dropping the step the drag opened puts them where they were.
    pub cancel_node_drag: Option<Vec<NodeId>>,
    /// A control `Alt`-clicked, waiting for the next MIDI message.
    pub learn_midi: Option<PortRef>,
    /// A bound control whose binding the range editor was asked to forget.
    pub unbind_midi: Option<PortRef>,
    /// A copy, a cut or a paste asked for by a node's menu or by the browser's own row.
    pub clip: Option<ClipAction>,
    /// `(node, which value, height)`: a value's box wants to be as tall as what it drew, in
    /// world units. Layout, not an edit.
    pub grown: Vec<(NodeId, usize, f32)>,
}

/// One node on screen: the frame, the node, the layout the pass made of it, and where the
/// view puts it. Built once per drawn node and handed to every part that draws it, so no part
/// lays the node out again.
pub struct NodeCtx<'a> {
    pub frame: &'a CanvasFrame<'a>,
    pub id: NodeId,
    pub node: &'a Node,
    pub layout: NodeLayout<'a>,
    pub t: Transform,
    /// Top-left of the canvas, which world positions are measured from.
    pub origin: Pos2,
    pub selected: bool,
    /// How brightly each action port is throbbing this frame, for its dot and its button.
    pub fires: &'a HashMap<PortRef, f32>,
}

impl NodeCtx<'_> {
    pub fn zoom(&self) -> f32 {
        self.t.zoom
    }

    pub fn theme(&self) -> &crate::ui::theme::Theme {
        self.frame.theme
    }

    /// A world rect on screen.
    pub fn screen(&self, world: Rect) -> Rect {
        self.t.to_screen_rect(self.origin, world)
    }

    /// A screen point in world units, which is what a popup hangs from.
    pub fn world(&self, screen: Pos2) -> Pos2 {
        self.t.to_world(self.origin, screen)
    }

    /// The body on screen.
    pub fn body(&self) -> Rect {
        self.screen(self.layout.rect)
    }

    /// One row's own block on screen, where the node has that row.
    pub fn block(&self, row: crate::ui::canvas::Row) -> Option<Rect> {
        self.layout.block(row).map(|world| self.screen(world))
    }

    /// The node's own name, `{slug}{id}`.
    pub fn name(&self) -> crate::ui::node_widget::NodeName {
        crate::ui::node_widget::NodeName {
            slug: self.node.def.slug,
            node: self.id,
        }
    }

    /// Whether this node's control under `key` is waiting for a MIDI message.
    pub fn learning(&self, key: &'static str) -> bool {
        self.frame.learning == Some(crate::midi::Target::Port(PortRef::new(self.id, key)))
    }

    /// Where the fader bound to this node's control under `key` is, while soft takeover
    /// holds it out of pick-up.
    pub fn ghost(&self, key: &'static str) -> Option<f32> {
        crate::widgets::ghost_of(self.frame.ghosts, PortRef::new(self.id, key))
    }

    /// A control's name on this node, `{slug}{id}.{key}`.
    pub fn control(&self, key: &'static str) -> crate::ui::node_widget::ControlName {
        crate::ui::node_widget::ControlName {
            slug: self.node.def.slug,
            node: self.id,
            key,
        }
    }
}
