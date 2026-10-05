// SPDX-License-Identifier: AGPL-3.0-or-later

//! The canvas. Emits `Command`s and never mutates the graph itself.

pub mod about;
pub mod banner;
pub mod bridge;
pub mod browse;
pub mod cable;
pub mod canvas;
pub mod check;
pub mod color;
pub mod context;
pub mod history;
pub mod keycap;
pub mod layout;
pub mod loop_meter;
pub mod maininput;
pub mod menu;
pub mod midi;
pub mod mixer;
pub mod node_widget;
pub mod number;
pub mod panel;
pub mod placed;
pub mod player;
pub mod popup;
pub mod prefs;
pub mod press;
pub mod problems;
pub mod project;
pub mod project_name;
pub mod rail;
pub mod report;
pub mod scope;
pub mod shortcuts;
pub mod start;
pub mod status;
pub mod strip;
pub mod tabs;
pub mod text;
pub mod theme;
pub mod timecode;

use crate::command::Command;
use crate::graph::{Graph, LayoutMode, NodeId, PortRef, PortType, WorkspaceId};
use crate::render::Fit;
use canvas::Transform;
use eframe::egui::{Key, Modifiers, Pos2, Rect, Sense, Stroke, Ui, vec2};
use node_widget::PortSlot;

pub use context::{CanvasFrame, Effects, NodeCtx};

/// What a widget on the canvas is called: written into the accessibility tree and a tooltip
/// only when something reads it, and hashed into the widget's id every frame.
///
/// A control on a node is named by [`node_widget::ControlName`], which is its three parts
/// until it is read; anything else passes a string.
pub trait Name: std::fmt::Display + std::fmt::Debug + std::hash::Hash {}

impl<T: std::fmt::Display + std::fmt::Debug + std::hash::Hash + ?Sized> Name for T {}

/// Give `response` its accessible name, which is what a test and the egui MCP find it by.
///
/// `name` is formatted only when something reads the tree, so a `format_args!` builds no
/// string on a frame nobody is listening to. Its arguments are evaluated at the call, so a
/// name that allocates to compute keeps a `widget_info` closure of its own.
pub fn accessible(
    response: &eframe::egui::Response,
    kind: eframe::egui::WidgetType,
    name: impl std::fmt::Display,
) {
    response.widget_info(|| eframe::egui::WidgetInfo::labeled(kind, true, &name));
}

/// A framed field's chrome: a ground at `RADIUS_SM` and a one-point hairline inside its edge,
/// silvia's `background` and `1px solid` border. The press button, the text field, the
/// tick's box and the select all wear it. `level` fills the field from the left, a fraction of
/// its width in a tint of its own, between the ground and the hairline.
pub fn field(
    painter: &eframe::egui::Painter,
    rect: Rect,
    ground: eframe::egui::Color32,
    level: Option<(f32, eframe::egui::Color32)>,
    border: eframe::egui::Color32,
) {
    let radius = eframe::egui::CornerRadius::same(theme::RADIUS_SM);
    painter.rect_filled(rect, radius, ground);
    if let Some((fraction, tint)) = level {
        let mut done = rect;
        done.max.x = rect.min.x + rect.width() * fraction.clamp(0.0, 1.0);
        painter.rect_filled(done, radius, tint);
    }
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, border),
        eframe::egui::StrokeKind::Inside,
    );
}

/// What a control waiting for a MIDI message adds to its accessible name, which is how a test
/// and the egui MCP read that it is armed.
pub const LEARNING: &str = "(learning MIDI)";

/// What a control a fader is out of soft takeover's pick-up for adds to its accessible name,
/// before the fader's place: `frequency 8 (MIDI fader at 2)`.
pub const GHOST: &str = "MIDI fader at";

/// One breath of the learning ring, in seconds: the period of silvia's `midi-learning-pulse`.
const LEARN_SECONDS: f64 = 1.5;
/// The ring at its dimmest, as a fraction of the accent: it never goes out while armed.
const LEARN_FLOOR: f32 = 0.3;
/// The ring's width, in world units.
const LEARN_WIDTH: f32 = 2.0;

/// The ring a control waiting for a MIDI message wears: the accent, inside the control's own
/// edge at `RADIUS_SM` where its hairline and a narrowed range's accent border already sit,
/// breathing between [`LEARN_FLOOR`] and full once every [`LEARN_SECONDS`]. The number
/// control and the press button both draw it, over their own chrome.
///
/// Asks for the next frame, as the node throb does, so the breath is not one frame long.
pub fn learning_ring(ui: &Ui, rect: Rect, zoom: f32, theme: &theme::Theme) {
    let now = ui.input(|i| i.time);
    let breath = (1.0 - (now / LEARN_SECONDS * std::f64::consts::TAU).cos()) * 0.5;
    let brightness = LEARN_FLOOR + (1.0 - LEARN_FLOOR) * breath as f32;
    ui.painter().rect_stroke(
        rect,
        eframe::egui::CornerRadius::same(theme::RADIUS_SM),
        Stroke::new(
            (LEARN_WIDTH * zoom).max(1.5),
            theme.accent().gamma_multiply(brightness),
        ),
        eframe::egui::StrokeKind::Inside,
    );
    ui.ctx().request_repaint();
}

/// The bound mark's radius, in world units. Small: it says *this one answers to a knob* and
/// nothing more, and a control already carries a label, a value and often a port.
pub const MIDI_DOT: f32 = 2.5;

/// Where a bound control's dot sits, relative to what the control is drawn in.
#[derive(Debug, Clone, Copy)]
pub enum MarkAt {
    /// Just left of the control's own slot, on its middle: a row's number, swatch or press
    /// button, which has the row's own room to its left.
    Beside(Rect),
    /// On the slot's top-left corner, in the gutter: a number in a region's grid, whose cells
    /// stand a few points apart on every side, so the only room is where four rounded
    /// corners leave it.
    Corner(Rect),
    /// After a caption, on its middle: the mixer's fade, whose slot has `A` and `B` on its
    /// two sides and whose caption stands over it.
    After(Rect),
}

impl MarkAt {
    fn center(self, radius: f32) -> Pos2 {
        match self {
            Self::Beside(slot) => Pos2::new(slot.min.x - radius * 2.0, slot.center().y),
            Self::Corner(slot) => slot.min - vec2(radius, radius) * 0.8,
            Self::After(caption) => caption.right_center() + vec2(radius * 3.0, 0.0),
        }
    }
}

/// The mark a bound control wears: a dot in the accent, just outside the control — silvia's
/// `midi-mapped` indicator. Every bindable control draws it through here: a row's number,
/// swatch and press button, a number a region draws, and the Main Mixer's fade.
///
/// Outside the control rather than on it, because a control is drawn to its own edges — a
/// dot inside would sit on the trough, and on the one control that fills its row it would sit
/// on the caption. Painted over whatever is there rather than laid out, so nothing moves by a
/// point when a knob is bound.
///
/// A named widget and not only paint: what drives a control is a fact about it, and a fact
/// the accessibility tree cannot see is a fact a test and the agent-driven layer cannot read.
/// `control` is the control as that name reads it — `checkerboard1 frequency`,
/// `A / B balance` — and the name is `{control} bound to {trigger}`; `unbind` says where the
/// binding is forgotten, on the tooltip after the trigger.
pub fn midi_mark(
    ui: &Ui,
    at: MarkAt,
    zoom: f32,
    theme: &theme::Theme,
    control: &str,
    trigger: &str,
    unbind: &str,
) {
    let radius = MIDI_DOT * zoom;
    let center = at.center(radius);
    ui.painter().circle_filled(center, radius, theme.accent());
    let hit = Rect::from_center_size(center, vec2(radius * 4.0, radius * 4.0));
    let name = format!("{control} bound to {trigger}");
    let response = ui.interact(hit, ui.id().with(("midi-mark", &name)), Sense::hover());
    accessible(
        &node_widget::hover_with(response, || format!("{trigger} — {unbind}")),
        eframe::egui::WidgetType::Label,
        &name,
    );
}

/// What every uniform number output published this frame, as the canvas reads it.
///
/// The view is the synth's — it is a borrow of the maps a tick writes — and is re-exported
/// here because the canvas is the one thing that reads it.
pub use crate::synth::Uniforms;

/// A control whose popup is open. Recorded while nodes are drawn and rendered afterwards,
/// so a node painted later cannot cover the popup.
///
/// `at` is in **world** units. A screen position captured when the popup opened detaches
/// from its control the moment the canvas pans, zooms or the node moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenControl {
    Color {
        node: NodeId,
        key: &'static str,
        at: Pos2,
    },
    Select {
        node: NodeId,
        key: &'static str,
        at: Pos2,
        /// How wide the control it hangs from is, in world units. A list narrower than the
        /// button that opened it reads as a different object; this is what makes it the
        /// same one, opened.
        width: f32,
    },
    /// The three numbers that belong to this instance of a control rather than to its kind.
    Range {
        node: NodeId,
        key: &'static str,
        at: Pos2,
    },
    /// An `Asset` option's picker: the project's own media that this option accepts, with
    /// the file dialog under it.
    Asset {
        node: NodeId,
        key: &'static str,
        at: Pos2,
        width: f32,
    },
    /// An `Asset` option asked for a file. Not a popup: `ui::show` turns it into a
    /// `Effects::file_requests` entry and the app opens the dialog on its thread.
    File { node: NodeId, key: &'static str },
    /// An Output's `!`: the sources upstream of it that a render cannot step, each a way to
    /// go there.
    Live { node: NodeId, at: Pos2 },
}

/// One source upstream of an Output that can only answer *now* — a camera, a microphone,
/// a screen, or the Main Input panel pointed at one — so a render of that Output reads
/// whatever it gives. What the Output's `!` lists. `App::live_sources` is the walk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSource {
    pub node: NodeId,
    /// What the popup says: the node's name and what it is.
    pub label: String,
    /// Where to go: a workspace the node is shown on, or `None` for the panel's own source.
    pub workspace: Option<WorkspaceId>,
    /// The source is the Main Input panel's, so going there unfolds the panel.
    pub panel: bool,
}

/// Transient canvas state. None of this is document data.
#[derive(Debug, Default)]
pub struct CanvasState {
    pub transform: Transform,
    /// The port a cable is currently being dragged from.
    dragging: Option<PortRef>,
    /// Where that port reaches, walked once for the drag rather than once per candidate
    /// port per frame: see [`drag_reach`].
    reach: Option<crate::graph::Reach>,
    /// The nodes under selection, in id order.
    ///
    /// Session state, not document data: it is not in the project and not undoable, which
    /// is why `ui/` may write it directly where it may not write the graph.
    selected: std::collections::BTreeSet<NodeId>,
    /// A rubber band in progress: where it started, in **world** units so it survives a pan
    /// mid-drag, and the selection it started from.
    ///
    /// The base is kept because the band updates the selection on every frame rather than on
    /// release — you can see what it is catching while you drag it — so each frame has to
    /// start from what was selected before the band, not from last frame's answer.
    marquee: Option<(emath::Pos2, std::collections::BTreeSet<NodeId>)>,
    /// A node drag in flight: each dragged node's offset from the pointer, in **world**
    /// units, taken at the press.
    ///
    /// The drag places a node at the pointer plus its offset rather than adding the
    /// pointer's motion to where the node was. An offset in world units survives everything
    /// that happens to the view under a drag — a pan, an auto-scroll, a zoom — because the
    /// pointer's world position already carries it; an accumulated delta tracks the cursor
    /// only while the view is still. It also survives [`clamp_to_strip`], which decides
    /// where a node is *placed* without touching where the hand is holding it, so a node
    /// pushed against the strip's floor comes back to the same grip.
    ///
    /// Empty when no node is being dragged, which is also how the canvas knows one is.
    drag_offsets: Vec<(NodeId, eframe::egui::Vec2)>,
    /// The rectangle the pan is clamped to and the minimap is a map of, easing toward
    /// [`strip_bounds`] every frame.
    ///
    /// One answer for both, so a map of one length and a view held to another cannot drift
    /// apart, and one mechanism for every way the strip can change length: a node deleted,
    /// an undo, a node collapsing, an arrange, a drag letting go. Each of those shortens the
    /// live strip in a single frame, and the pan is clamped to it — so without the lag the
    /// camera is yanked by whatever happened, wherever the view was sitting.
    ///
    /// While a node drag is in flight this grows and never shrinks. A node dragged further
    /// out makes room to drop it into, which is wanted; a node dragged inward shortening the
    /// strip under the hand is the feedback loop that moves the thing being placed while it
    /// is being placed.
    ///
    /// `None` on a workspace with nothing on it, and on a plane, where nothing clamps.
    bounds: Option<Rect>,
    /// Which control popup is open, if any.
    open: Option<OpenControl>,
    /// The node browser, open. Where it draws and where a node chosen in it lands are both
    /// its own, so opening it is the whole of deciding those.
    browser: Option<browse::Browser>,
    /// A cable released on a port that can only take it through a node: the conversion menu,
    /// open, with the two ports it is choosing a bridge for.
    bridging: Option<bridge::Bridging>,
    /// Nodes drawn last frame, against the graph's total. Off-screen nodes are culled.
    drawn: usize,
    /// Height of the canvas area last frame. `Command::AutoArrange` carries a height, and
    /// the menu has to know which one to ask for.
    height: f32,
    /// Width of it, which with the height is what centring a node on the view needs.
    width: f32,
    /// Top-left of the canvas area last frame, which is the origin world coordinates are
    /// measured from. Recorded for the same reason `height` is: it is known only while
    /// drawing, and something outside needs it afterwards.
    origin: emath::Pos2,
    /// Scroll velocity in points per second, so a flick keeps going after the fingers stop.
    fling: eframe::egui::Vec2,
    /// A pan the view is gliding to, because something was put somewhere you cannot see.
    ///
    /// Dropping a node is the case it exists for: a drag can carry one out over a side
    /// panel or clean off the edge, and letting go there leaves it placed somewhere with no
    /// sign of it on screen. The view goes and finds it. A glide rather than a jump for the
    /// reason everything else on this canvas glides — a cut to a different part of the
    /// strip reads as the graph having moved, not the camera.
    ///
    /// Cleared by any hand on the view, because a glide the person is steering against is a
    /// fight neither wins, and cleared when it cannot move, as a fling against the clamp is.
    ///
    /// A whole view rather than a pan, because Frame All and Frame Selected glide the zoom as
    /// well. Pan and zoom ease at the one rate, and a world point's place on screen is affine in
    /// the two, so every point on the canvas travels in a straight line to where it is going.
    pan_target: Option<Transform>,
    /// Where the last right-click landed, in world units: the point a Paste from the menu it
    /// opened plants its clip at. Taken when the click happens rather than inside the menu,
    /// where the pointer is over the menu and no longer over the canvas.
    menu_at: emath::Pos2,
    /// Which step of the golden-angle hue walk each cable was given, by its two ends.
    ///
    /// Handed out in the order cables are first *seen*, and kept — so a cable holds its
    /// color for as long as the session does, and deleting one does not recolor the rest.
    /// silvia's counter reshuffles every patch on reload; this one is only reshuffled by
    /// closing the app, and undo puts a cable back under the color it had.
    ///
    /// Session state, and the reason it is a map rather than a field on the connection: a
    /// display preference has no business in the document.
    cable_hues: std::collections::HashMap<(PortRef, PortRef), u32>,
    /// The next step to hand out.
    next_cable_hue: u32,
    /// The step reserved for the cable currently being dragged out of a port.
    ///
    /// Taken when the drag arms, so the wire in flight is already wearing the color it will
    /// keep once it lands — silvia's `CursorWire` takes a color and hands that same one to
    /// the `Connection` it becomes. A cable that changed color at the moment it connected
    /// would be the one frame where the color said nothing.
    drag_hue: Option<u32>,
    /// A node a link was followed to, throbbing so the eye lands on it.
    ///
    /// Following a link jumps the view somewhere else entirely, and the node that was asked
    /// for arrives in the middle of however many others are around it. The throb is what
    /// says *this one* — it is the answer to the click, and without it the jump is a screen
    /// that changed and nothing pointing at why.
    ///
    /// The start is stamped on the first frame that draws it rather than when the link is
    /// clicked: the clock this animates against is egui's, and `App` is holding a different
    /// one. `None` inside the pair means *asked for, not yet drawn*.
    throb: Option<(NodeId, Option<f64>)>,
    /// Action ports that have fired and are still throbbing, each with the moment it was
    /// first drawn. `None` inside means *fired, not yet drawn*, exactly as `throb` does —
    /// and re-firing puts one back, so a port that fires again restarts rather than
    /// finishing the throb it was already in.
    fires: std::collections::HashMap<PortRef, Option<f64>>,
    /// How tall each node's value fields came out when they last drew: the one part of a
    /// node's height the document cannot say. See [`canvas::Measured`].
    measured: canvas::Measured,
    /// Every node on the workspace, laid out once a frame. Kept between frames only so its
    /// buffers are reused; nothing reads last frame's answer.
    layouts: canvas::Layouts,
    /// Something else had the keyboard when the last pass ended: a field had focus, or a
    /// popup was open. `Escape` takes focus away and closes a popup before the canvas runs,
    /// so the key that closed one is told from a key meant for the canvas by this.
    keyboard_elsewhere: bool,
    /// Whether the node in hand could go into the cable under the pointer, and through which
    /// of its ports: asked once per node and cable rather than once a frame, since the answer
    /// is put to a copy of the graph, and forgotten when the hand opens. A drag writes
    /// positions and nothing else, so the cables the answer was worked out over are the ones
    /// under the hand until then. See [`crate::nodes::attach::splice_ports`].
    splice: Option<SpliceAnswer>,
}

/// One answer of [`crate::nodes::attach::splice_ports`], kept with the question it answers.
#[derive(Debug, Clone, Copy)]
struct SpliceAnswer {
    node: NodeId,
    cable: crate::graph::Connection,
    ports: Option<(&'static str, &'static str)>,
}

/// How long a followed node throbs for, in seconds.
const THROB_SECONDS: f64 = 1.1;
/// How many times it brightens in that span. Three reads as a pulse; one reads as a flash
/// that might have been a redraw, and five is a node asking to be looked at for too long.
const THROB_PULSES: f64 = 3.0;
/// How far outside the node's own border the ring sits, in world units, and how thick it is.
const THROB_GAP: f32 = 3.0;
const THROB_WIDTH: f32 = 2.5;

/// How long an action port throbs for when it fires, in seconds.
///
/// silvia has no throb on a port at all: the only geometry it has for a firing is Random
/// Fire's own `_showFireIndicator`, which steps a pill's background up one token and puts it
/// back on a 50 ms timer. That is a jump and a snap back, which reads as a glitch at a port's
/// size, so what is taken from silvia is the *duration* of the one animation it gives an
/// action control — `.action-control-button { transition: transform 0.15s ease }` — with the
/// brightening decaying across it rather than being dropped at the end of it.
const FIRE_SECONDS: f64 = 0.15;
/// How far toward white the dot goes on the frame it fires. silvia's indicator steps one
/// token up, `bg-interactive` to `bg-hover`; this is the same size of step at full
/// brightness, and a port that is already the brightest thing on its row cannot afford more.
const FIRE_LIFT: f32 = 0.55;

impl CanvasState {
    /// What each node's value fields last measured, for anything outside the canvas that
    /// lays a node out: an arrange, a clamp to the strip, a test.
    pub fn measured(&self) -> &canvas::Measured {
        &self.measured
    }

    /// Record how tall one value's field says it needs to be, and forget every node the graph
    /// no longer holds. View state: nothing here is an edit, so nothing here is undone.
    pub fn measure(&mut self, graph: &Graph, grown: &[(NodeId, usize, f32)]) {
        for &(id, index, height) in grown {
            if let Some(node) = graph.get(id) {
                self.measured.set(id, node, index, height);
            }
        }
        self.measured.retain_in(graph);
    }

    /// The rectangle the view is clamped to and the minimap is a map of, easing toward the
    /// strip as the graph makes it. `None` on a plane and on an empty workspace.
    pub fn bounds(&self) -> Option<Rect> {
        self.bounds
    }

    /// The view as the project file carries it, which is the transform and nothing else:
    /// how long the strip is, the strip works out for itself every frame.
    pub fn saved_view(&self) -> crate::project::View {
        self.transform.saved()
    }

    /// Put a saved view back.
    ///
    /// The clamp's own rectangle is dropped with it: a view arriving from a file is where
    /// the strip *is*, not somewhere to glide to from whatever the last workspace happened
    /// to be. The next frame measures this workspace and takes the answer whole.
    pub fn restore_view(&mut self, view: crate::project::View) {
        self.transform = Transform::restored(view);
        self.bounds = None;
    }

    /// Which step of the hue walk a cable was given, if it has been seen.
    pub fn cable_hue(&self, from: PortRef, to: PortRef) -> Option<u32> {
        self.cable_hues.get(&(from, to)).copied()
    }

    /// The next step of the hue walk, and the walk moves on.
    fn take_cable_hue(&mut self) -> u32 {
        let step = self.next_cable_hue;
        self.next_cable_hue = self.next_cable_hue.wrapping_add(1);
        step
    }

    /// Make `node` throb, which is how following a link answers the click.
    ///
    /// Session state, like the pan it arrives with: following a link never enters the undo
    /// history, so neither does this.
    pub fn throb_on(&mut self, node: NodeId) {
        self.throb = Some((node, None));
    }

    /// Remember that these action ports fired, so the next frame draws the throb.
    ///
    /// Session state, like the node throb it is a sibling of: a firing is something that
    /// happened, never an edit.
    pub fn fired_on(&mut self, ports: impl IntoIterator<Item = PortRef>) {
        for port in ports {
            self.fires.insert(port, None);
        }
    }

    /// How brightly each throbbing port is lit, at `now` on egui's clock, stamping the ones
    /// that have not been drawn yet and dropping the ones that are spent.
    ///
    /// Brightest on the first frame and decaying from there, exactly as the node's throb is:
    /// what it says is *that happened*, and a mark that swells in says it late.
    fn fires_at(&mut self, now: f64) -> std::collections::HashMap<PortRef, f32> {
        let mut lit = std::collections::HashMap::new();
        self.fires.retain(|port, started| {
            let t = (now - *started.get_or_insert(now)) / FIRE_SECONDS;
            if t >= 1.0 {
                return false;
            }
            lit.insert(*port, (1.0 - t) as f32);
            true
        });
        lit
    }

    /// The node throbbing and how brightly, at `now` on egui's clock. `None` once the throb
    /// has run its course, which is also when it clears itself.
    ///
    /// Brightest on the first frame and decaying from there: the point is to catch the eye
    /// that has just been moved somewhere else, so it cannot afford to swell in.
    fn throb_at(&mut self, now: f64) -> Option<(NodeId, f32)> {
        let (node, started) = self.throb.as_mut()?;
        let (node, started) = (*node, *started.get_or_insert(now));
        let t = (now - started) / THROB_SECONDS;
        if t >= 1.0 {
            self.throb = None;
            return None;
        }
        let pulse = (t * THROB_PULSES * std::f64::consts::TAU).cos() * 0.5 + 0.5;
        Some((node, (pulse * (1.0 - t)) as f32))
    }

    /// How many nodes were drawn last frame. The rest were off screen.
    pub fn drawn(&self) -> usize {
        self.drawn
    }

    /// Height of the canvas area last frame, which is the strip an arrange fits nodes into.
    pub fn height(&self) -> f32 {
        self.height
    }

    /// Width of the canvas area last frame.
    pub fn width(&self) -> f32 {
        self.width
    }

    /// Top-left of the canvas area last frame, the origin for `Transform::to_screen`.
    pub fn origin(&self) -> emath::Pos2 {
        self.origin
    }

    /// Put a world point in the middle of the canvas area, at the zoom in force.
    ///
    /// What clicking a tag ends in: the workspace comes up showing the node the cable came
    /// from, rather than wherever that canvas was left.
    pub fn center_on(&mut self, world: emath::Pos2) {
        let half = vec2(self.width, self.height) * 0.5;
        self.transform.pan = half - world.to_vec2() * self.transform.zoom;
        // Being sent somewhere outranks being on the way somewhere.
        self.pan_target = None;
    }

    /// Whether nodes are in hand, as the last canvas pass left them. The tab bar is drawn
    /// before the canvas, so this is what tells it to light a tab up for a drop.
    pub fn dragging_nodes(&self) -> bool {
        !self.drag_offsets.is_empty()
    }

    pub fn transform(&self) -> Transform {
        self.transform
    }

    /// The nodes under selection, in id order.
    pub fn selected(&self) -> &std::collections::BTreeSet<NodeId> {
        &self.selected
    }

    /// The first selected node satisfying `pick` — the Output whose picture the preview
    /// shows, in practice.
    pub fn selected_where(&self, pick: impl Fn(NodeId) -> bool) -> Option<NodeId> {
        self.selected.iter().copied().find(|id| pick(*id))
    }

    /// Close the node browser, for `App` when the other menu opened.
    pub fn close_browser(&mut self) {
        self.browser = None;
    }

    /// Did the browser open since this was last asked? `App` closes the Nodes menu on it.
    pub fn take_browser_just_opened(&mut self) -> bool {
        self.browser
            .as_mut()
            .is_some_and(browse::Browser::take_just_opened)
    }

    /// Replace the selection.
    pub fn select_only(&mut self, ids: &[NodeId]) {
        self.selected = ids.iter().copied().collect();
    }

    /// Drop these from the selection, for a command that removed them.
    pub fn deselect(&mut self, ids: &[NodeId]) {
        self.selected.retain(|id| !ids.contains(id));
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
    }

    /// Which control's popup is showing. The picker is drawn after every node, so this is
    /// also what says whether anything is floating above the canvas.
    pub fn open_control(&self) -> Option<OpenControl> {
        self.open
    }

    /// Drop everything that points into the graph.
    ///
    /// Undo, redo and opening a file all replace the graph wholesale. A selection may name
    /// a node the new one does not have, and node ids restart at 1 in every project, so a
    /// popup left open would go on editing whatever now holds the id it captured.
    pub fn forget_graph_refs(&mut self, graph: &Graph) {
        self.selected.retain(|id| graph.get(*id).is_some());
        self.open = None;
        self.bridging = None;
    }
}

/// How close a click has to be to a cable, in screen points, to delete it.
const CABLE_HIT: f32 = 6.0;

/// What fraction of a fling's speed survives one second. A flick glides for something under
/// a second and stops rather than drifting.
const FLING_DECAY: f32 = 0.004;
/// Below this speed, in points per second, a glide is over.
const FLING_STOP: f32 = 12.0;
/// Ceiling on launch speed, so one violent flick cannot throw the strip across the graph.
const FLING_MAX: f32 = 3200.0;

/// How tall the minimap is, in screen points. Linear mode only.
const RAIL_HEIGHT: f32 = 120.0;

/// Space kept clear above and below a node in the strip. The same margin the arrange uses,
/// so a hand-placed node sits in the band an arranged one would.
const STRIP_MARGIN: f32 = layout::VERTICAL_MARGIN;

/// How close to the edge of the canvas the pointer has to be, in screen points, for a node
/// drag to start creeping the view that way.
///
/// A quarter of a node's width: wide enough to fall into without aiming at it, and narrow
/// enough that most of the canvas is still somewhere a node can be put down and left alone.
const EDGE_SCROLL_MARGIN: f32 = 48.0;

/// How fast the view creeps with the pointer held at the very edge, in points per second.
///
/// **This is how fast carrying a node somewhere far away goes**, and nothing else is. The
/// node is held under the cursor and the cursor is at the edge, so the node travels at
/// exactly the rate the view does: making room ahead of the view faster than this changes
/// how long the strip is and changes nothing anyone can feel. It was three hundred, which
/// was a push to sit through — a thousand points of strip took three and a half seconds.
///
/// The rate falls off linearly to nothing at the inner lip of the margin, so the hand still
/// chooses the speed by how far in it pushes, and the slow end of that ramp is where a node
/// is placed precisely rather than carried.
const EDGE_SCROLL_RATE: f32 = 900.0;

/// How far ahead of the creep the far end of the strip gives way, as a multiple of it.
///
/// A multiple rather than a rate of its own, so the room keeps leading by the same margin
/// whatever the creep is tuned to. Leading at all is worth a little: room that arrives
/// exactly as fast as it is used leaves the pan against its own clamp for the whole push.
/// Leading by a lot is worth nothing — the node cannot outrun the view carrying it — so
/// this is small. What is made and not used costs nothing either way: the strip eases back
/// to what its content needs the moment the hand opens.
const EDGE_GROWTH_AHEAD: f32 = 2.0;

/// How long anything on the canvas that eases takes to cover most of its distance.
///
/// One knob for every length the view is clamped to, moving to where the strip says it
/// should be. A **time constant** rather than a duration, so a second ask part-way
/// through retargets and carries on from where it is — no queue, no wait, no jump at the
/// handover. Short enough to read as one motion rather than an animation to sit through.
const EASE_TWEEN: f32 = 0.18;
/// Near enough to the target to stop easing, in world units. Below a pixel at any sane zoom,
/// and an exponential approach never actually arrives.
const EASE_SETTLED: f32 = 0.5;
/// The same for a zoom gliding to where a frame asked: a thousandth of a node's width is
/// below a pixel at any width a node is drawn.
const ZOOM_SETTLED: f32 = 0.001;

/// How far to move toward a target this frame, 0 to 1: an exponential approach at
/// [`EASE_TWEEN`], which is frame-rate independent where a fixed fraction per frame is not.
fn ease(dt: f32) -> f32 {
    1.0 - (-dt / EASE_TWEEN).exp()
}

/// This frame's length, bounded at both ends.
///
/// Clamped low so a frame reporting no time at all still advances an ease, and high so a
/// stall — a shader rebuild, a file dialog — does not teleport whatever was mid-motion when
/// the app comes back.
fn frame_dt(ui: &Ui) -> f32 {
    ui.input(|i| i.stable_dt).clamp(1.0 / 1000.0, 1.0 / 15.0)
}

/// Grid dots never get closer together than this on screen. Below it the world pitch steps
/// up, so zooming out cannot produce an unbounded number of dots.
const MIN_GRID_SCREEN_PITCH: f32 = 18.0;

/// The render in progress, as the canvas draws it: the Output's button reads *Cancel* with
/// the progress behind it, and every other number goes inert, since the document is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderView {
    pub node: NodeId,
    pub written: u32,
    pub frames: u32,
}

/// An Output's recording, as its Record row draws it: how long it has run on the show's
/// clock, and how many of its frames repeat the one before for lack of a picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordView {
    pub seconds: f64,
    pub dropped: u64,
}

/// An Output's Render button was clicked: start one, or cancel the one running.
///
/// A *request*, like `pop_outs`: a render is done to the instrument rather than to the
/// document, so `App` performs it and nothing enters the undo history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderRequest {
    pub node: NodeId,
    pub cancel: bool,
}

/// A picture that can be shown in a window of its own: the node, and which of its textures.
///
/// The window a picture can have, the list of the ones that do, and what a mark asked for.
///
/// All three live in [`crate::render::picture`], because the thread that owns the windows is
/// there and one definition is worth more than a mirror of it. `ui/` reads them: a picture's
/// marks say what its window is already doing.
pub use crate::render::picture::{Popped, Request as PopOutRequest, Shown as PopOut};

/// Where one Output node's own frame goes.
///
/// The rect is the picture's, held inside the node's rounded border and kept at full size
/// however little of it is on screen. The slot was reserved in the node's own paint order,
/// between the screen's black ground and the border, so whatever the caller puts in it is
/// painted under the node's chrome rather than over every node on the canvas.
#[derive(Debug, Clone, Copy)]
pub struct Thumbnail {
    pub node: NodeId,
    /// Which of the node's textures to blit: `None` is an Output's own render, `Some(key)` a
    /// CPU node's published frame, which the renderer keys by port rather than by node.
    pub port: Option<&'static str>,
    pub rect: eframe::egui::Rect,
    pub slot: eframe::egui::layers::ShapeIdx,
    /// How the frame is fitted to `rect`. A node's own slot already carries the Output's
    /// aspect and asks for `Cover`; a fixed-shape box an Output of any resolution is shown
    /// in, like the mixer panel's channel preview, asks for `Letterbox`.
    pub fit: Fit,
    /// The body's corner radius in points; the renderer rounds the picture's bottom corners
    /// by it so the picture can sit flush.
    pub corner: f32,
}

/// What the cost strip under a node says: a line of text and how full its bar is.
///
/// Built by `App` from the probe's counts and the Outputs' timers; `ui/` only draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Cost {
    /// Drawn from the strip's left edge.
    pub left: String,
    /// Drawn against its right edge, so the space between the two opens as the node widens
    /// and the two halves never have to agree about a separator. Empty where there is
    /// nothing to say — a drop count of zero is the strip's own silence.
    pub right: String,
    /// 0..1 of the bar. What it is a fraction *of* is the caller's: an Output's is its share
    /// of a vsync interval, a node's is its share of the busiest node's evaluations.
    pub fraction: f32,
    /// Over budget: the bar is drawn in the accent color.
    pub hot: bool,
}

/// What an Output says about itself: silvia's status line above its picture, and what its
/// Send rows report.
///
/// Live state rather than the node's own — every field is read from the graph, the mixer or
/// the publisher each frame — which is why it arrives as a lane beside [`Cost`] rather than as
/// a `Node::values` entry. Nothing here is saved and nothing here is an edit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputReadout {
    /// Something is cabled into its input. silvia's `● Input` / `○ No Input`.
    pub connected: bool,
    /// Claimed by deck A, by deck B, by both, or by neither.
    pub decks: (bool, bool),
    /// A render of *this* Output is running.
    pub rendering: bool,
    /// This Output's live recording, while one runs.
    pub recording: Option<RecordView>,
    /// Why the last recording asked of this Output failed, until the next press.
    pub record_error: Option<String>,
    /// Some Output is recording, so no render can start.
    pub recording_anywhere: bool,
    /// Why it is not going out over NDI, where it is asked to and the sender says why not.
    pub ndi_error: Option<String>,
    /// The same over Syphon.
    pub syphon_error: Option<String>,
}

impl OutputReadout {
    /// What the sender said about one way out, if anything.
    pub fn error(&self, way: crate::nodes::output::Way) -> Option<&str> {
        match way {
            crate::nodes::output::Way::Ndi => self.ndi_error.as_deref(),
            crate::nodes::output::Way::Syphon => self.syphon_error.as_deref(),
        }
    }
}

/// How tall a cost strip is, in world units.
const COST_HEIGHT: f32 = 14.0;

/// The slot the canvas reserved under itself for the mix.
#[derive(Debug, Clone, Copy)]
pub struct Background {
    pub rect: eframe::egui::Rect,
    pub slot: eframe::egui::layers::ShapeIdx,
}

/// A node drag, as the canvas sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeDrag {
    /// The selection being dragged, which the drag made the dragged node a member of.
    pub nodes: Vec<NodeId>,
    /// True on the frame the pointer let go.
    pub released: bool,
}

/// A copy, a cut or a paste, as a surface asked for it.
///
/// A *request*, like `pop_outs`: the clipboard is session state rather than document data —
/// a copy changes no graph state, so there is nothing about it to undo — and only the paste
/// becomes a command. One enum for every door, because the keyboard, the Edit menu and the
/// node menu ask for the same three things and `App` answers all of them in one place.
#[derive(Debug, Clone, PartialEq)]
pub enum ClipAction {
    Copy(Vec<NodeId>),
    /// The copy and the delete both, which `App` performs as one `RemoveNodes` and therefore
    /// as one undo step.
    Cut(Vec<NodeId>),
    Paste {
        /// Where the clip's anchor lands, in world units: the point a right-click named.
        /// `None` where the gesture named no point — the keyboard and the Edit menu — and the
        /// clip goes back to where it was in the window instead.
        at: Option<Pos2>,
    },
}

/// The golden angle, in degrees: a full turn divided by phi.
///
/// Walking the hue circle by this is what makes consecutive cables as unlike each other as
/// consecutive things can be — the step never lands where a previous one did and never
/// settles into a cycle, which is the whole of why silvia picked it.
const GOLDEN_ANGLE: f32 = 137.508;

/// The color of the `step`th cable under [`CanvasPrefs::phi_cables`].
///
/// silvia's `getGoldenRatioColor`, saturation and lightness included: three saturations
/// cycling every step and three lightnesses every third, so two cables close together on the
/// circle still differ in how strong and how deep they are. Ours are lifted from silvia's
/// 30/40/50% because a cable here is drawn over a dark canvas rather than a light editor,
/// and 30% lightness on this ground is a cable you cannot see.
fn phi_cable(step: u32) -> eframe::egui::Color32 {
    const SATURATION: [f32; 3] = [0.45, 0.55, 0.65];
    const LIGHTNESS: [f32; 3] = [0.55, 0.65, 0.75];
    let hue = (step as f32 * GOLDEN_ANGLE) % 360.0;
    let s = SATURATION[step as usize % SATURATION.len()];
    let l = LIGHTNESS[(step as usize / SATURATION.len()) % LIGHTNESS.len()];
    theme::Hsl::new(hue, s, l).color()
}

/// Where a duplicate lands relative to its original: far enough to see both, near enough
/// that the copy is obviously the same thing.
pub const DUPLICATE_OFFSET: eframe::egui::Vec2 = eframe::egui::vec2(24.0, 24.0);

/// The four verbs a selection is offered, as each reads and the command it sends: Duplicate,
/// Collapse or Expand, Reset controls, Disconnect all. The right-click menu and the Edit menu
/// both draw these, so the two doors to them cannot drift apart.
///
/// One expanded node means Collapse: the entry closes whatever is still open rather than
/// toggling each node into the opposite of what it was, which would scramble a mixed
/// selection. With nothing selected it reads Collapse too, since a greyed *Expand* suggests
/// something is collapsed when nothing is chosen at all.
pub fn node_verbs(nodes: &[NodeId], any_expanded: bool) -> [(&'static str, Command); 4] {
    let nodes = nodes.to_vec();
    [
        (
            "Duplicate",
            Command::Duplicate {
                nodes: nodes.clone(),
                offset: DUPLICATE_OFFSET,
            },
        ),
        (
            if any_expanded || nodes.is_empty() {
                "Collapse"
            } else {
                "Expand"
            },
            Command::SetCollapsed {
                nodes: nodes.clone(),
                collapsed: any_expanded,
            },
        ),
        ("Reset controls", Command::ResetControls(nodes.clone())),
        ("Disconnect all", Command::DisconnectAll(nodes)),
    ]
}

/// The menu a selection gets, whether it was reached from a node's header or from the
/// background.
///
/// One function, because there is no such thing as a single-node menu here: every entry is
/// already a command over a list, so the only difference between one node and twenty is the
/// length of `nodes`. Two copies of this drifted apart within a day — the header's kept
/// acting on its own node while a selection was live, which is not what right-clicking one
/// of several selected nodes means.
fn selection_menu(
    ui: &mut Ui,
    frame: &CanvasFrame<'_>,
    nodes: &[NodeId],
    // Where a Paste from this menu lands, in world units: the click that opened it.
    paste_at: Pos2,
    fx: &mut Effects,
) {
    let (graph, can_paste) = (frame.graph, frame.clipboard);
    let (out, clip) = (&mut fx.commands, &mut fx.clip);
    if nodes.len() > 1 {
        ui.label(format!("{} selected", nodes.len()));
        ui.separator();
    }
    // The clipboard three first, where every editor keeps them. Copy and Cut act on the
    // selection this menu is already displaying; Paste needs nothing selected at all, so it
    // is offered whenever the clipboard holds something and lands at the click that opened
    // the menu.
    let any = !nodes.is_empty();
    if ui
        .add_enabled(any, eframe::egui::Button::new("Copy"))
        .clicked()
    {
        *clip = Some(ClipAction::Copy(nodes.to_vec()));
        ui.close();
    }
    if ui
        .add_enabled(any, eframe::egui::Button::new("Cut"))
        .clicked()
    {
        *clip = Some(ClipAction::Cut(nodes.to_vec()));
        ui.close();
    }
    if ui
        .add_enabled(can_paste, eframe::egui::Button::new("Paste"))
        .on_disabled_hover_text(menu::why::EMPTY_CLIPBOARD)
        .clicked()
    {
        *clip = Some(ClipAction::Paste { at: Some(paste_at) });
        ui.close();
    }
    ui.separator();
    let any_open = nodes
        .iter()
        .any(|id| graph.get(*id).is_some_and(|n| !n.collapsed));
    for (label, command) in node_verbs(nodes, any_open) {
        if ui.button(label).clicked() {
            out.push(command);
            ui.close();
        }
    }
    workspaces_menu(ui, graph, nodes, out);
    if ui.button("Delete").clicked() {
        out.push(Command::RemoveNodes(nodes.to_vec()));
        ui.close();
    }
}

/// **Workspaces ▸** and **Move to ▸**: which views these nodes are on.
///
/// A box is ticked where *every* node in the selection is on that workspace, so a mixed
/// selection reads as unticked and one click puts all of it there. Unticking the last
/// workspace a node is on would leave it nowhere, which `HideFrom` refuses — so that box is
/// drawn disabled rather than offered and then refused.
fn workspaces_menu(ui: &mut Ui, graph: &Graph, nodes: &[NodeId], out: &mut Vec<Command>) {
    let on = |id: NodeId, w: WorkspaceId| graph.get(id).is_some_and(|n| n.workspaces.contains(&w));
    ui.menu_button("Workspaces", |ui| {
        for workspace in graph.workspaces() {
            let w = workspace.id;
            let all = nodes.iter().all(|id| on(*id, w));
            // The last one a node has: taking it away would strand that node.
            let only = nodes.iter().any(|id| {
                graph
                    .get(*id)
                    .is_some_and(|n| n.workspaces.len() == 1 && n.workspaces.contains(&w))
            });
            let mut ticked = all;
            let box_ = eframe::egui::Checkbox::new(&mut ticked, &workspace.name);
            if ui
                .add_enabled(!(all && only), box_)
                .on_disabled_hover_text(menu::why::LAST_WORKSPACE)
                .changed()
            {
                out.push(if ticked {
                    Command::ShowOn {
                        nodes: nodes.to_vec(),
                        workspace: w,
                    }
                } else {
                    Command::HideFrom {
                        nodes: nodes.to_vec(),
                        workspace: w,
                    }
                });
                ui.close();
            }
        }
    });
    ui.menu_button("Move to", |ui| {
        for workspace in graph.workspaces() {
            if ui.button(&workspace.name).clicked() {
                out.push(Command::MoveTo {
                    nodes: nodes.to_vec(),
                    workspace: workspace.id,
                });
                ui.close();
            }
        }
    });
}

/// The preferences the canvas reads while it draws.
///
/// A struct rather than four more arguments on `show`, which already takes twenty. They are
/// grouped because they arrive together and for no deeper reason: each is an independent
/// question, and `App` builds this once a frame from the store.
// Four independent answers a person gave about their own editor, not a state machine: any
// combination is meaningful, so grouping them to satisfy a count would only add a layer.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy)]
pub struct CanvasPrefs {
    /// Lock and hide the cursor while a number control is dragged. `number::scrub` reads it
    /// to know whether to grab the pointer on `drag_started` and release it on `drag_stopped`.
    pub lock_cursor: bool,
    /// Invert the wheel along a Linear workspace's strip.
    pub scroll_x_inverted: bool,
    /// Brighten a hovered port's cables and the ports at their far ends.
    pub port_hover_highlight: bool,
    /// Let a cable sag between its ports.
    pub cable_droop: bool,
    /// Color each cable by a golden-angle walk of the hue circle, and outline each connected
    /// port in its cable's color.
    pub phi_cables: bool,
    /// Cast a shadow under every node body.
    pub node_shadow: bool,
}

/// Draw the canvas and return everything the interaction asked for.
///
/// One pass in phases, each a function of its own, in the order a frame needs them:
///
/// - **the hand** — `Escape` puts back a node drag or drops a cable ([`escape_from_hand`]);
/// - **layout** — every node on the workspace, once, in world units ([`canvas::Layouts`]);
/// - **view** — the canvas's rect, the pan and its clamp, the ground and the grid;
/// - **hit test** — every port placed on screen, and what is under the pointer ([`Pass`]),
///   the cable a node in the hand would go into among it ([`insert_target`]);
/// - **cables** — painted behind the nodes, with their click handles held back;
/// - **nodes** — each one on screen, its body, its controls and its ports;
/// - **drag** — the node drag in flight, the cable handles, the rubber band and the
///   background's menu;
/// - **popups** — a control's popup, the browser and the conversion menu, above every node;
/// - **keys** — the canvas's own ([`keys`]), and a refused port's words ([`refusal`]);
/// - **drag end** — a cable let go, the wheel, the cost strips, the minimap and the
///   selection count.
///
/// Which widget wins a click is the order egui was told about them, which is the order of
/// the phases: the background first, then each node's ground, its regions, its header and its
/// marks, its controls and its ports, then every cable's handle last of all.
pub fn show(ui: &mut Ui, state: &mut CanvasState, frame: &CanvasFrame<'_>) -> Effects {
    let mut fx = Effects::default();
    state.drawn = 0;
    // Before anything moves or is drawn: what is in the hand goes back on this frame.
    escape_from_hand(ui, state, &mut fx);
    // Every node on the workspace, laid out once for the whole pass, in world units — so it
    // does not wait on the pan below, and everything after it reads it rather than walking a
    // node's rows again. Out of the state for the pass, so it can be read while the rest of
    // the state is written.
    let mut layouts = std::mem::take(&mut state.layouts);
    layouts.lay_out(frame.graph, frame.workspace, &state.measured);

    let view = view(ui, state, frame, &layouts, &mut fx);
    let mut pass = Pass::new(ui, state, frame, view, &layouts);
    pass.hover.insert = insert_target(state, &pass);
    let drawn = cables(state, &pass);

    // A click that opens a popup is the same click the dismissal check would see, so
    // remember what was open before this frame and only allow dismissal of a popup that
    // was already up.
    let previously = state.open;
    let mut open = state.open;
    nodes(ui, state, &pass, &drawn.outline, &mut fx, &mut open);
    drag_nodes(ui, state, &pass, &mut fx);
    cable_handles(ui, &pass, drawn.handles, &mut fx);
    marquee(ui, state, &pass);
    background_menu(state, &pass, &mut fx);

    popups(ui, state, &pass, previously, open, &mut fx);
    browser(ui, state, &pass, &mut fx);
    bridge_menu(ui, state, &pass, &mut fx);
    keys(ui, state, &pass, &mut fx);
    refusal(ui, state, &pass);

    drop_cable(ui, state, &pass, &mut fx);
    wheel(ui, state, &pass);
    cost_strips(ui, state, &pass);
    if pass.view.linear {
        rail(ui, state, &pass, &mut fx);
    }
    selection_count(ui, state, &pass);
    drop(pass);
    // Read by the next pass's keys: see `keyboard_elsewhere`.
    state.keyboard_elsewhere =
        ui.ctx().egui_wants_keyboard_input() || eframe::egui::Popup::is_any_open(ui.ctx());
    state.layouts = layouts;
    fx
}

/// Where the canvas is this frame and how it is shown, as the view phase settled it.
struct View {
    /// The canvas area: the panel less the minimap under a strip.
    rect: Rect,
    /// Its top-left, which world coordinates are measured from.
    origin: Pos2,
    /// The canvas's own response, registered before anything on it.
    background: eframe::egui::Response,
    /// The canvas's own painter, clipped to it.
    painter: eframe::egui::Painter,
    linear: bool,
    /// Shift held: a band adds to the selection, a click on a node toggles it.
    shift: bool,
    rail_height: f32,
}

/// The view: the canvas's rect, the hand on the background, the creep at an edge, the glide,
/// the strip's clamp, the ground and the grid — everything that decides where this frame is
/// drawn, settled before anything is, since a view that moved halfway down the frame would
/// tear.
fn view(
    ui: &mut Ui,
    state: &mut CanvasState,
    frame: &CanvasFrame<'_>,
    layouts: &canvas::Layouts,
    fx: &mut Effects,
) -> View {
    let linear = frame.graph.layout_of(frame.workspace) == LayoutMode::Linear;
    // The strip is drawn at one scale and nothing moves it off one. Every `t.zoom` multiply
    // below then takes its identity value, and no glyph size ever changes.
    if linear {
        state.transform.zoom = 1.0;
    }
    let available = ui.available_size();
    let rail_height = if linear { RAIL_HEIGHT } else { 0.0 };
    let (rect, background) = ui.allocate_exact_size(
        vec2(available.x, (available.y - rail_height).max(0.0)),
        Sense::click_and_drag(),
    );
    state.height = rect.height();
    state.width = rect.width();
    let origin = rect.min;
    state.origin = origin;
    let painter = ui.painter_at(rect);

    // --- background: pan, or a rubber band with Shift ------------------------------------
    let shift = ui.input(|i| i.modifiers.shift);
    if background.drag_started() && shift {
        // `press_origin`, not `interact_pointer_pos`: a drag starts on the frame the pointer
        // has already moved past the threshold, so the interact position is the far end of
        // the band by the time anyone asks. World units, so the band stays over the same
        // nodes if the view moves under it.
        state.marquee = ui
            .input(|i| i.pointer.press_origin())
            .map(|p| (state.transform.to_world(origin, p), state.selected.clone()));
    }
    if background.dragged() {
        // Putting a hand on it stops it, as it would anywhere else.
        state.fling = eframe::egui::Vec2::ZERO;
        if state.marquee.is_none() {
            state.transform.pan += background.drag_delta();
            // A hand on the view outranks a glide, exactly as it outranks a fling.
            state.pan_target = None;
        }
    }
    creep(ui, state, rect, linear);
    settle(ui, state, layouts, rect, linear);
    if background.clicked() {
        // Shift-click on nothing is an additive gesture that added nothing, not a clear.
        if !shift {
            state.clear_selection();
        }
    }
    // silvia's *Project to Background*: the mix under the nodes and the cables, the graph
    // floating on the show. The ground and the grid are what it replaces, so neither is
    // drawn over it; the mix is over the screen's black, as every picture is, and the slot is
    // filled by `App`, which has the GL.
    if frame.project_background {
        painter.rect_filled(rect, 0.0, frame.theme.screen_off());
        fx.background = Some(Background {
            rect,
            slot: painter.add(eframe::egui::Shape::Noop),
        });
    } else {
        if !frame.cleared {
            painter.rect_filled(rect, 0.0, frame.theme.bg_primary());
        }
        grid(state, frame.theme, &painter, rect);
    }
    View {
        rect,
        origin,
        background,
        painter,
        linear,
        shift,
        rail_height,
    }
}

/// A node drag in flight creeps the view at the edges.
///
/// Before anything is drawn, so the pan this settles on is the one every node, cable and
/// port is painted at. Where the held nodes go is [`drag_nodes`], after the loop that arms a
/// drag — it is a `Command` and takes effect on the next frame either way.
fn creep(ui: &Ui, state: &mut CanvasState, rect: Rect, linear: bool) {
    if state.drag_offsets.is_empty() || !ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let Some(p) = ui.ctx().pointer_latest_pos() else {
        return;
    };
    // Along the strip only in Linear: a node there is clamped into the viewport's own
    // height, so there is nothing above or below the view to reach.
    let mut creep = edge_creep(p, rect, frame_dt(ui));
    if linear {
        creep.y = 0.0;
    }
    if creep == eframe::egui::Vec2::ZERO {
        return;
    }
    // **The far end of the strip yields as the drag pushes at it, faster than the view
    // follows.** The room is made straight into the clamp's own rectangle, which grows and
    // never shrinks while a hand is closed — so it is still there on the next frame, and it
    // is given back by the ease once the hand opens.
    //
    // It leads the creep by `EDGE_GROWTH_AHEAD` rather than matching it, which is only worth
    // the small margin it is: room arriving exactly as fast as it is used leaves the pan
    // against its own clamp for the whole push. **How fast the strip grows is not how fast
    // the gesture feels** — the node is under the cursor and the cursor is at the edge, so
    // what the hand is waiting for is the creep, and that is the number to raise.
    //
    // Straight off the creep, so there is one falloff and not two, and so this is a function
    // of the pointer and the frame like the creep is. A rate read from the pan or the bounds
    // would accelerate into itself.
    if linear
        && creep.x < 0.0
        && let Some(bounds) = &mut state.bounds
    {
        bounds.max.x += -creep.x * EDGE_GROWTH_AHEAD;
    }
    state.transform.pan += creep;
    // A still canvas asks for no frames, and the hand holding a node at the edge sends no
    // events.
    ui.ctx().request_repaint();
}

/// The glide toward somewhere the view has been sent, and the rectangle the view is held to.
fn settle(ui: &Ui, state: &mut CanvasState, layouts: &canvas::Layouts, rect: Rect, linear: bool) {
    // A glide toward somewhere the view has been sent, at the one `EASE_TWEEN`. Before the
    // clamp, so the clamp has the last word on where this frame ends up.
    let before_glide = state.transform;
    if let Some(target) = state.pan_target {
        let k = ease(frame_dt(ui));
        let pan = state.transform.pan + (target.pan - state.transform.pan) * k;
        let zoom = state.transform.zoom + (target.zoom - state.transform.zoom) * k;
        if (target.pan - pan).length() < EASE_SETTLED && (target.zoom - zoom).abs() < ZOOM_SETTLED {
            state.transform = target;
            state.pan_target = None;
        } else {
            state.transform.pan = pan;
            state.transform.zoom = zoom;
            // A still canvas asks for no frames, and nothing else here is moving.
            ui.ctx().request_repaint();
        }
    }

    if !linear {
        // Nothing clamps a plane and there is no map under one, so the strip is not measured
        // where the answer has no reader.
        state.bounds = None;
        return;
    }
    // The rectangle the view is held to, easing toward the strip as it stands. The strip has
    // ends: panning stops where the content does, so a node cannot be lost off the side of
    // one — which is half of what a bounded canvas is for.
    let live = strip_bounds(layouts, rect);
    state.bounds = match (state.bounds, live) {
        // Grown, never shrunk, for as long as a drag lasts.
        (Some(held), Some(live)) if !state.drag_offsets.is_empty() => Some(held.union(live)),
        (Some(held), Some(live)) => {
            let next = held.lerp_towards(&live, ease(frame_dt(ui)));
            // Near enough that a further frame would move nothing anyone can see.
            let done = (next.min - live.min).length() < EASE_SETTLED
                && (next.max - live.max).length() < EASE_SETTLED;
            if done {
                Some(live)
            } else {
                // A settled canvas asks for no frames, and a strip that changed length sends
                // no events of its own.
                ui.ctx().request_repaint();
                Some(next)
            }
        }
        // A workspace that has just been emptied has no strip to ease toward, and one
        // arriving with content is where the view is rather than somewhere to glide to.
        (_, live) => live,
    };
    clamp_pan(&mut state.transform, rect, state.bounds);
    // **A glide that cannot move is over**, exactly as a fling that has reached the end of
    // the strip is. Left running it would ask for a frame a second forever against a clamp
    // that is never going to yield — which happens when what it was sent to fetch is wider
    // than the viewport, or sits past an end the pan cannot reach.
    if state.pan_target.is_some() && state.transform == before_glide {
        state.pan_target = None;
    }
    // Where a right-click landed, for whatever menu it opens. Read here, before any menu is
    // drawn: inside one the pointer is over the menu, and a Paste would land under its own
    // entry rather than under the click that asked for it.
    if ui.input(|i| i.pointer.secondary_clicked())
        && let Some(p) = ui.ctx().pointer_latest_pos()
    {
        state.menu_at = state.transform.to_world(state.origin, p);
    }
}

/// The editor's dot grid. The world pitch steps up as you zoom out, so the number of dots on
/// screen stays bounded: a fixed 24-unit pitch means ~25,000 circles at zoom 0.35 and the
/// count grows without limit as you keep zooming out.
fn grid(state: &CanvasState, theme: &theme::Theme, painter: &eframe::egui::Painter, rect: Rect) {
    let (t, origin) = (state.transform, state.origin);
    let mut world_pitch = canvas::GRID_PITCH;
    while world_pitch * t.zoom < MIN_GRID_SCREEN_PITCH {
        world_pitch *= 4.0;
    }
    // Collected and handed over in one call. `circle_filled` takes the Context's write lock
    // per dot, and there are thousands of dots.
    let dot = theme.grid_dot();
    let first = t.to_world(origin, rect.min);
    let start_x = (first.x / world_pitch).floor() * world_pitch;
    let start_y = (first.y / world_pitch).floor() * world_pitch;
    let mut dots: Vec<eframe::egui::Shape> = Vec::new();
    let mut wy = start_y;
    while t.to_screen(origin, Pos2::new(start_x, wy)).y <= rect.max.y {
        let mut wx = start_x;
        while t.to_screen(origin, Pos2::new(wx, wy)).x <= rect.max.x {
            dots.push(eframe::egui::Shape::circle_filled(
                t.to_screen(origin, Pos2::new(wx, wy)),
                1.0,
                dot,
            ));
            wx += world_pitch;
        }
        wy += world_pitch;
    }
    painter.extend(dots);
}

/// Everything the pass worked out before drawing a node, and read by every phase after it:
/// the frame, the view, every node's layout, every port on screen, and what is under the
/// pointer.
struct Pass<'a> {
    frame: &'a CanvasFrame<'a>,
    view: View,
    layouts: &'a canvas::Layouts,
    /// The pan and zoom every place on screen here was worked out for.
    transform: Transform,
    /// Every port on screen, each node's the span its layout says, in the layout's order.
    slots: Vec<PortSlot>,
    /// Every node's body on screen, in the layout's order, which is the order they are
    /// painted in: what [`node_widget::port_at`] asks whether a port is covered.
    bodies: Vec<(NodeId, Rect)>,
    /// Which data inputs already have something plugged in.
    fed: std::collections::HashSet<PortRef>,
    /// Every cable with exactly one end on this workspace, by the port that end is: an input
    /// gets a tag per source that is elsewhere, an output gets hover text naming where it
    /// goes.
    from_elsewhere: std::collections::HashMap<PortRef, Vec<PortRef>>,
    to_elsewhere: std::collections::HashMap<PortRef, Vec<PortRef>>,
    pointer: Option<Pos2>,
    /// The pointer, where it is on the canvas itself: [`on_canvas`].
    on_canvas: Option<Pos2>,
    /// What the pointer is on, as the one hit test answered it.
    hover: Hover,
    /// How brightly each action port is throbbing this frame.
    fires: std::collections::HashMap<PortRef, f32>,
    /// The node a followed link is throbbing, and how brightly.
    throb: Option<(NodeId, f32)>,
}

impl<'a> Pass<'a> {
    /// Place every port on screen for the view the frame settled on, and ask what the
    /// pointer is on.
    fn new(
        ui: &Ui,
        state: &mut CanvasState,
        frame: &'a CanvasFrame<'a>,
        view: View,
        layouts: &'a canvas::Layouts,
    ) -> Self {
        let graph = frame.graph;
        let (t, origin) = (state.transform, view.origin);
        // Once per frame, into one buffer, from the layout: the layout's own world positions,
        // moved to where this frame's pan and zoom put them.
        let slots: Vec<PortSlot> = layouts
            .ports()
            .iter()
            .map(|s| PortSlot {
                center: t.to_screen(origin, s.center),
                ..*s
            })
            .collect();
        let bodies = layouts
            .iter()
            .map(|l| (l.id, t.to_screen_rect(origin, l.rect)))
            .collect();
        // Which data inputs already have something plugged in. Asked once per port per frame.
        let fed = graph.connections().iter().map(|c| c.to).collect();
        // Built in the same one pass `fed` is, because asking `sources_of` per port would
        // rescan every connection for every port — which is the cost rule the canvas is built
        // on. Both stay empty in a project whose cables are all local, which is most of them.
        // The workspace filter, where the culling already filters. A node on another
        // workspace is not drawn, not hit and not laid out, so it has no port geometry
        // either — which is what makes a cable with an end elsewhere simply absent rather
        // than drawn to nowhere.
        let visible: std::collections::HashSet<NodeId> = layouts.iter().map(|l| l.id).collect();
        let mut from_elsewhere: std::collections::HashMap<PortRef, Vec<PortRef>> =
            std::collections::HashMap::new();
        let mut to_elsewhere: std::collections::HashMap<PortRef, Vec<PortRef>> =
            std::collections::HashMap::new();
        for c in graph.connections() {
            match (visible.contains(&c.from.node), visible.contains(&c.to.node)) {
                (false, true) => from_elsewhere.entry(c.to).or_default().push(c.from),
                (true, false) => to_elsewhere.entry(c.from).or_default().push(c.to),
                _ => {}
            }
        }
        // Stamped once for the whole pass: each stamps its own start on the first frame that
        // draws it and drops itself on the last, and both should happen once.
        let now = ui.input(|i| i.time);
        let fires = state.fires_at(now);
        if !fires.is_empty() {
            ui.ctx().request_repaint();
        }
        let throb = state.throb_at(now);
        if throb.is_some() {
            // Nothing else on a still canvas asks for frames, so without this the throb is
            // one frame long and then whatever the next event happens to redraw.
            ui.ctx().request_repaint();
        }
        let pointer = ui.ctx().pointer_latest_pos();
        let on_canvas = pointer.filter(|p| on_canvas(ui, view.rect, *p));
        let mut pass = Self {
            frame,
            view,
            layouts,
            transform: t,
            slots,
            bodies,
            fed,
            from_elsewhere,
            to_elsewhere,
            pointer,
            on_canvas,
            hover: Hover::default(),
            fires,
            throb,
        };
        pass.hover = pass.hit_test(&t);
        pass
    }

    /// One port's place on screen.
    fn find(&self, port: PortRef) -> Option<PortSlot> {
        self.layouts.slot(port).map(|i| self.slots[i])
    }

    /// How far below both ends' bodies a cable between these two nodes passes, on screen.
    fn clearance(&self, a: NodeId, b: NodeId) -> f32 {
        self.layouts
            .clearance(a, b, &self.transform, self.view.origin)
    }

    /// The one hit test the canvas makes of its own: what the pointer is on.
    ///
    /// Made here, ahead of the node loop that draws and interacts with the ports themselves,
    /// because cables are drawn *behind* nodes and need the answer before that loop runs.
    fn hit_test(&self, t: &Transform) -> Hover {
        let graph = self.frame.graph;
        let on_canvas = self.on_canvas;
        let mut hover = Hover {
            // The port's own square, the one `node_widget::port` interacts in and the one a
            // cable let go lands by: a port lights exactly where it would take the click
            // that clears it and the cable that is dropped on it.
            port: on_canvas
                .and_then(|p| node_widget::port_at(&self.slots, &self.bodies, p, t, |_| true))
                .map(|s| s.port),
            // Only the nearest cable within the hit radius is hovered. Cables leaving one
            // output run side by side, and crossings are ordinary, so testing each
            // independently meant one click deleted every cable it happened to be near —
            // each as its own undo step.
            cable: on_canvas
                .and_then(|p| {
                    graph
                        .connections()
                        .iter()
                        .enumerate()
                        .filter_map(|(i, c)| {
                            let (from, to) = (self.find(c.from)?, self.find(c.to)?);
                            let d = cable::Curve::new(
                                from.center,
                                to.center,
                                from.ty,
                                self.clearance(c.from.node, c.to.node),
                                self.frame.prefs.cable_droop,
                            )
                            .distance_to(p);
                            (d < CABLE_HIT).then_some((i, d))
                        })
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                })
                .map(|(i, _)| i),
            lit: std::collections::HashSet::new(),
            insert: None,
        };

        // **What lights up with the pointer, in both directions.** From a port: every cable
        // it carries and the port at the far end of each — silvia's `glowOnHover` without the
        // glow, which is several extra strokes a cable a frame and the UI must never make the
        // render miss one. Brightened and thickened instead, which is what hover already
        // does.
        //
        // And from a *cable*: the two ports it runs between. The two halves answer the same
        // question from whichever end the hand happens to be on, and a cable that lit its
        // ports only one way round was a gap you noticed the moment you followed a wire with
        // the pointer rather than from its socket.
        //
        // Both gated by the preference. The nearest cable itself is not: that one is the
        // click-to-delete affordance and stays whatever anyone thinks of the highlight.
        if self.frame.prefs.port_hover_highlight {
            if let Some(p) = hover.port {
                hover.lit.insert(p);
                for c in graph.connections() {
                    if c.from == p {
                        hover.lit.insert(c.to);
                    }
                    if c.to == p {
                        hover.lit.insert(c.from);
                    }
                }
            }
            if let Some(c) = hover.cable.and_then(|i| graph.connections().get(i)) {
                hover.lit.insert(c.from);
                hover.lit.insert(c.to);
            }
        }
        hover
    }
}

/// Whether a point is on the canvas itself: inside its rect, which the tab bar, the panels and
/// the rail are outside, and on its layer rather than under a window, a menu or a popup.
///
/// `layer_id_at` is egui's own answer to which layer is on top at a point, the one
/// `contains_pointer` reads for [`wheel`]; the background layer, which the canvas is drawn on,
/// is registered over the whole window, so the answer is never `None` there.
fn on_canvas(ui: &Ui, rect: Rect, p: Pos2) -> bool {
    rect.contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
}

/// What the pointer is on this frame, as the canvas's own hit test answers it: the port under
/// it, the cable nearest it, and the ports that lights.
///
/// Worked out once, and read by everything that asks: the cables' paint and the ports' paint,
/// the double-click that deletes the cable, and — through the same square, `port_at` — the
/// cable a hand lets go. A struct rather than an enum because the pointer can be on a port and
/// beside a cable at once, and each of the two paints reads both.
#[derive(Debug, Default)]
struct Hover {
    port: Option<PortRef>,
    /// By its place in the graph's list.
    cable: Option<usize>,
    lit: std::collections::HashSet<PortRef>,
    /// The cable the node in hand would go into if it were let go now, by its place in the
    /// graph's list, with the node's input and output that would carry it.
    insert: Option<(usize, &'static str, &'static str)>,
}

/// What painting the cables left for the phases after it.
struct Cables {
    /// Each cable's own click widget, registered *after* every node and port rather than
    /// alongside the paint: the ports on the very node this cable ends at can sit close enough
    /// to the curve's midpoint that their own (larger) hit rect would otherwise cover it, and
    /// the port registered later in the very same frame is what a tied hit test hands the
    /// click to. Collected where the curve and its midpoint are already at hand, and
    /// interacted with in [`cable_handles`] once nothing left to draw can still out-rank it.
    handles: Vec<(crate::graph::Connection, Pos2, bool)>,
    /// Under `phi_cables`, the color each connected port is outlined in: its cable's. An
    /// input has exactly one cable, so its answer is that cable's; an **output** may have
    /// many, and the first in graph order wins rather than the last to be drawn — silvia lets
    /// whichever connection updated most recently overwrite the border, which means the
    /// outline on a fanned-out port changes for reasons that have nothing to do with it.
    ///
    /// Empty and untouched when the preference is off, so nothing here costs anything to the
    /// look that does not ask for it.
    outline: std::collections::HashMap<PortRef, eframe::egui::Color32>,
}

/// The cables, behind the nodes, and the one being dragged out of a port.
fn cables(state: &mut CanvasState, pass: &Pass<'_>) -> Cables {
    let (graph, prefs, theme) = (pass.frame.graph, pass.frame.prefs, pass.frame.theme);
    let zoom = state.transform.zoom;
    let mut drawn = Cables {
        handles: Vec::new(),
        outline: std::collections::HashMap::new(),
    };
    for (ordinal, connection) in graph.connections().iter().enumerate() {
        let (Some(from), Some(to)) = (pass.find(connection.from), pass.find(connection.to)) else {
            continue;
        };
        let hovered = pass.hover.cable == Some(ordinal);
        // A cable also lights up when the pointer is on either of its own ports — what a
        // right-click there is about to clear — even when it is not the nearest cable to the
        // pointer itself. Only `hovered` gates the double-click deletion: that gesture is
        // proximity to the *cable*, and a port lighting up several at once must not make all
        // of them one click from gone. Gated by the preference, unlike `hovered`.
        let touches_hovered_port = prefs.port_hover_highlight
            && pass
                .hover
                .port
                .is_some_and(|p| p == connection.from || p == connection.to);
        // The cable a node in hand would go into: lit in the selection's color, since what
        // it says is *this is where the node you are holding goes*.
        let inserting = pass.hover.insert.is_some_and(|(i, ..)| i == ordinal);
        let emphasized = hovered || touches_hovered_port || inserting;
        // 4px for data, 2px for action; 6px and brightened when emphasized, which is also
        // the click-to-delete affordance for the nearest cable.
        let base = if from.ty == PortType::Action {
            2.0
        } else {
            4.0
        };
        // What it carries, or its own place on the hue circle. The step is handed out the
        // first time a cable is seen and kept for the session, so a cable's color does not
        // depend on how many cables happen to exist beside it this frame.
        let wire = if prefs.phi_cables {
            let key = (connection.from, connection.to);
            let next = &mut state.next_cable_hue;
            let step = *state.cable_hues.entry(key).or_insert_with(|| {
                let step = *next;
                *next = next.wrapping_add(1);
                step
            });
            let color = phi_cable(step);
            // The two ends wear it too. `or_insert`, so the first cable in graph order is
            // the one a fanned-out output answers with.
            drawn.outline.entry(connection.from).or_insert(color);
            drawn.outline.entry(connection.to).or_insert(color);
            color
        } else {
            theme.wire(from.ty)
        };
        let stroke = Stroke::new(
            if emphasized { base * 1.5 } else { base } * zoom.max(0.4),
            if inserting {
                theme.primary()
            } else if emphasized {
                wire.gamma_multiply(1.4)
            } else {
                wire
            },
        );
        let clear = pass.clearance(connection.from.node, connection.to.node);
        let curve = cable::Curve::new(from.center, to.center, from.ty, clear, prefs.cable_droop);
        curve.paint(&pass.view.painter, stroke);

        // A cable is a real widget, sitting on the curve's midpoint — a hand-painted thing a
        // user can click and the accessibility tree cannot see is a bug by this project's own
        // rule, and it is what let the agent-driven layer reach every control and no cable.
        // The widget is what carries the name; the geometry is what carries the click.
        drawn.handles.push((*connection, curve.midpoint(), hovered));
    }

    // --- the cable being dragged --------------------------------------------------------
    let dragging = state.dragging.and_then(|p| pass.find(p));
    if let (Some(anchor), Some(p)) = (dragging, pass.pointer) {
        let clear = state
            .dragging
            .map_or(0.0, |src| pass.clearance(src.node, src.node));
        // The color it will keep. Reserved when the drag armed rather than picked now, so
        // the wire does not change color on the frame it lands — see `drag_hue`.
        let ink = match state.drag_hue.filter(|_| prefs.phi_cables) {
            Some(step) => phi_cable(step),
            None => theme.port(anchor.ty),
        };
        cable::Curve::new(anchor.center, p, anchor.ty, clear, prefs.cable_droop)
            .paint(&pass.view.painter, Stroke::new(2.0, ink));
    }
    // A cable let go in the open stays drawn to where it was let go while the browser asks
    // what it lands on, so the list reads as an answer about that cable.
    if let Some(loose) = state.browser.as_ref().and_then(browse::Browser::cable)
        && let Some(anchor) = pass.find(loose.end)
    {
        let at = state.transform.to_screen(pass.view.origin, loose.at);
        let clear = pass.clearance(loose.end.node, loose.end.node);
        cable::Curve::new(anchor.center, at, anchor.ty, clear, prefs.cable_droop)
            .paint(&pass.view.painter, Stroke::new(2.0, theme.port(anchor.ty)));
    }
    drawn
}

/// Every node on screen: its shadow, its body, its throb, its selection and drag handle, its
/// controls and its ports.
///
/// Cull off-screen nodes. Port geometry is placed for every node, because a cable may run
/// from off-screen to on-screen, but drawing and hit-testing cost nothing for a node nobody
/// can see. This is what keeps UI cost proportional to what is visible rather than to graph
/// size — the render must never pay for chrome. The rule has no exception: a node in hand is
/// culled like any other, because the drag that is carrying it is run from the pointer in
/// [`drag_nodes`] rather than from this widget.
fn nodes(
    ui: &mut Ui,
    state: &mut CanvasState,
    pass: &Pass<'_>,
    outline: &std::collections::HashMap<PortRef, eframe::egui::Color32>,
    fx: &mut Effects,
    open: &mut Option<OpenControl>,
) {
    for layout in pass.layouts.iter() {
        let id = layout.id;
        let Some(node) = pass.frame.graph.get(id) else {
            continue;
        };
        if !pass.view.rect.intersects(
            state
                .transform
                .to_screen_rect(pass.view.origin, layout.rect),
        ) {
            continue;
        }
        state.drawn += 1;
        let cx = NodeCtx {
            frame: pass.frame,
            id,
            node,
            layout,
            t: state.transform,
            origin: pass.view.origin,
            selected: state.selected.contains(&id),
            fires: &pass.fires,
        };
        node_on_screen(ui, state, pass, &cx, fx, open);
        ports(ui, state, pass, &cx, outline, fx);
    }
}

/// One node on screen: the shadow under it, the body, the throb over it, what the body's
/// response says about the selection and the drag, and the controls on its rows.
fn node_on_screen(
    ui: &mut Ui,
    state: &mut CanvasState,
    pass: &Pass<'_>,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<OpenControl>,
) {
    let id = cx.id;
    // Before the body, which starts by filling the same rect: a shadow is under the node it
    // belongs to and over whatever the canvas has drawn so far, cables included.
    if pass.frame.prefs.node_shadow {
        node_widget::shadow(ui, &cx.layout, &cx.t, cx.origin);
    }
    let grab = node_widget::body(ui, cx, fx, open);
    // Over the node's own border and under everything drawn later, in the accent — not
    // `primary`, which is what a selected node's border already is: a throb that wore the
    // selection color would read as *this is now selected*, which it is not.
    if let Some((_, brightness)) = pass.throb.filter(|(n, _)| *n == id) {
        let gap = THROB_GAP * cx.zoom();
        ui.painter().rect_stroke(
            cx.body().expand(gap),
            node_widget::body_corners(&cx.t, gap),
            Stroke::new(
                THROB_WIDTH * cx.zoom(),
                cx.theme().accent().gamma_multiply(brightness),
            ),
            eframe::egui::StrokeKind::Outside,
        );
    }

    // Right-clicking a node outside the selection makes it the selection first, the same
    // rule dragging one follows: a menu that acted on nodes the pointer is nowhere near
    // would be a surprise.
    if grab.secondary_clicked() && !state.selected.contains(&id) {
        state.select_only(&[id]);
    }
    // A node inside a live selection opens that selection's menu. Right-clicking one of
    // several selected nodes means "these", not "this one".
    let targets: Vec<NodeId> = if state.selected.contains(&id) {
        state.selected.iter().copied().collect()
    } else {
        vec![id]
    };
    let menu_at = state.menu_at;
    grab.context_menu(|ui| selection_menu(ui, pass.frame, &targets, menu_at, fx));
    if grab.clicked() {
        if pass.view.shift {
            // Additive, and a second Shift-click takes it back out — the only way to drop one
            // node from a band selection without starting again.
            if !state.selected.remove(&id) {
                state.selected.insert(id);
            }
        } else {
            state.select_only(&[id]);
        }
    }
    // Grabbing a node outside the selection makes it the selection, as every canvas does:
    // otherwise the drag would move something the pointer is nowhere near.
    if grab.drag_started() && !state.selected.contains(&id) {
        state.select_only(&[id]);
    }
    // Arming the drag, and the whole of what the widget has to say about it: which node the
    // hand closed on, and where every selected node was relative to it. Everything after
    // this frame is the pointer and these grips, and runs in `drag_nodes`.
    //
    // `press_origin`, not the interact position, for the reason the marquee gives — a drag is
    // reported on the frame the pointer has already crossed the threshold, and the grip was
    // taken before that motion, not after it.
    if grab.dragged()
        && state.drag_offsets.is_empty()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        // A hand on a node stops a glide, as a hand on the background does: the grip is a
        // world position, and a strip still sliding under it is a strip the grip was taken
        // on a frame ago.
        state.fling = eframe::egui::Vec2::ZERO;
        state.pan_target = None;
        let held = state.transform.to_world(pass.view.origin, press);
        let workspace = pass.frame.workspace;
        state.drag_offsets = state
            .selected
            .iter()
            .filter_map(|sel| {
                let n = pass.frame.graph.get(*sel)?;
                // A selection can only hold nodes shown here, and a drag must not move one
                // that is not.
                n.workspaces
                    .contains(&workspace)
                    .then(|| (*sel, n.pos - held))
            })
            .collect();
    }

    node_widget::controls(ui, cx, fx, open);
}

/// One node's ports, and the tags and hover text that say where a cable goes when its far end
/// is on another workspace.
fn ports(
    ui: &mut Ui,
    state: &mut CanvasState,
    pass: &Pass<'_>,
    cx: &NodeCtx<'_>,
    outline: &std::collections::HashMap<PortRef, eframe::egui::Color32>,
    fx: &mut Effects,
) {
    let graph = pass.frame.graph;
    for world in cx.layout.ports {
        let slot = &PortSlot {
            center: cx.t.to_screen(cx.origin, world.center),
            ..*world
        };
        // The three states a port has while a cable is in flight: one it can take, one a node
        // between the two could carry it to, and one that is neither and is dimmed. An
        // illegal connection is not rejected, it is unofferable.
        let (dimmed, convertible) = match state.dragging {
            Some(src) => {
                let reach = drag_reach(&mut state.reach, graph, src);
                if legal(graph, reach, src, slot.port) {
                    (false, false)
                } else {
                    let bridged = bridgeable(graph, reach, src, slot.port);
                    (!bridged, bridged)
                }
            }
            None => (false, false),
        };
        // An input publishes nothing, so only an output has a value to carry.
        let (published, published_color) = if slot.is_input {
            (None, None)
        } else {
            (
                pass.frame.uniforms.get(slot.port),
                pass.frame.uniforms.color(slot.port),
            )
        };
        let look = node_widget::PortLook {
            dimmed,
            convertible,
            connected: slot.is_input && pass.fed.contains(&slot.port),
            lit: pass.hover.lit.contains(&slot.port),
            outline: outline.get(&slot.port).copied(),
            published,
            published_color,
            fire: pass.fires.get(&slot.port).copied().unwrap_or(0.0),
        };
        let response = node_widget::port(ui, slot, cx.node, &cx.t, &look, cx.theme());

        if response.drag_started() {
            state.dragging = Some(slot.port);
            // Reserved here and spent when it lands. A drag let go over nothing spends it
            // anyway and the walk simply moves on, which is what silvia does too — the step is
            // a position on a circle, not a scarce thing.
            state.drag_hue = pass.frame.prefs.phi_cables.then(|| state.take_cable_hue());
        }
        if response.clicked() && look.connected {
            fx.commands.push(Command::Disconnect { to: slot.port });
        }
        // Right-click clears every connection on the port, input or output alike — one undo
        // step, the way a group delete is. A no-op right-click on a bare port is left to the
        // command bus to refuse; nothing here has to know in advance whether there is
        // anything to clear.
        //
        // Ports gathered on one point of a header share a square, and egui hands the click to
        // whichever of them registered last, which is often one with nothing on it. What the
        // hand sees there is the cables leaving the point, so the first gathered port that
        // carries one is the one cleared, and a second right-click clears the next.
        if response.secondary_clicked() {
            let carries = |p: PortRef| graph.connections().iter().any(|c| c.from == p || c.to == p);
            let port = cx
                .layout
                .ports
                .iter()
                .filter(|w| w.center == world.center && w.is_input == world.is_input)
                .map(|w| w.port)
                .find(|p| carries(*p))
                .unwrap_or(slot.port);
            fx.commands.push(Command::DisconnectPort(port));
        }

        // The far end of a cable that cannot be drawn, because the node at the other end is
        // not on this workspace. The port keeps its connected border either way — it *is*
        // connected — and what is missing is where to.
        if slot.is_input {
            tags(ui, pass, slot, cx.zoom(), fx);
        } else if let Some(away) = pass.to_elsewhere.get(&slot.port) {
            // An output has no room for a pill — a cable leaves it rightwards into whatever
            // is there — so where it goes is hover text.
            let away: Vec<String> = away
                .iter()
                .filter_map(|to| {
                    let (home, _) = tag_home(pass.frame, to.node)?;
                    let target = graph.get(to.node)?;
                    Some(format!(
                        "{}{}.{} on {}",
                        target.def.slug, to.node, to.key, home.name
                    ))
                })
                .collect();
            if !away.is_empty() {
                response.on_hover_text(format!("to {}", away.join(", ")));
            }
        }
    }
}

/// Which workspace a tag names for a node that is not here, and whether it is open: the
/// node's [`Graph::home_of`], so clicking the tag goes where every other link to it goes.
fn tag_home<'a>(
    frame: &'a CanvasFrame<'_>,
    id: NodeId,
) -> Option<(&'a crate::graph::Workspace, bool)> {
    let home = frame.graph.home_of(id, None, frame.tabs)?;
    Some((frame.graph.workspace(home)?, frame.tabs.contains(&home)))
}

/// A pill per source that is elsewhere, stacked leftwards from an input port: an action input
/// takes many, and each one has its own somewhere.
fn tags(ui: &mut Ui, pass: &Pass<'_>, slot: &PortSlot, zoom: f32, fx: &mut Effects) {
    let Some(sources) = pass.from_elsewhere.get(&slot.port) else {
        return;
    };
    let theme = pass.frame.theme;
    let mut right = slot.center - vec2(theme::TAG_GAP * zoom, 0.0);
    for src in sources {
        let Some((home, is_open)) = tag_home(pass.frame, src.node) else {
            continue;
        };
        let Some(source) = pass.frame.graph.get(src.node) else {
            continue;
        };
        let (placed, response) = node_widget::tag(
            ui,
            right,
            &node_widget::Tag {
                icon: source.def.icon,
                label: &home.name,
                name: &format!(
                    "tag from {}{}.{} on {}",
                    source.def.slug, src.node, src.key, home.name
                ),
                ty: slot.ty,
                closed: !is_open,
            },
            theme,
            zoom,
        );
        if response.clicked() {
            fx.navigate = Some((home.id, src.node));
        }
        right.x = placed.min.x - theme::TAG_GAP * zoom;
    }
}

/// The node drag itself, once for however many nodes are in the hand.
///
/// The *start* of a node drag is a widget event — which node the hand closed on — and is
/// armed in [`node_on_screen`]. Nothing after that frame is: the gesture is `drag_offsets` and
/// the pointer, and nothing here asks a node's response for anything. So it runs once a frame
/// rather than once per dragged node, and a node culled off the edge of the view or deleted
/// under the hand cannot take the gesture with it.
///
/// After the node loop rather than with the creep, because the loop is where a drag is armed
/// and the first frame of one is the frame the hand notices. What it produces is a `Command`,
/// which lands on the next frame from anywhere in this one.
fn drag_nodes(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    if state.drag_offsets.is_empty() {
        return;
    }
    let (rect, origin) = (pass.view.rect, pass.view.origin);
    // A drag ends with the button rather than with the node's own response: what is being
    // dragged may have been culled or deleted under the hand, and a gesture must not end
    // because the thing it is moving stopped being drawn.
    let down = ui.input(|i| i.pointer.any_down());
    if let Some(p) = ui.ctx().pointer_latest_pos() {
        // Every node goes to the pointer plus the grip it was taken with, so the whole
        // selection keeps its layout and every node keeps the same place under the cursor
        // however the view moves beneath it. One command for all of them, so the whole drag
        // is one undo step.
        //
        // On the frame the button comes up as well, which is where a drag short enough to
        // press, move and release in three frames lands its only move.
        let at = state.transform.to_world(origin, p);
        let moves: Vec<_> = state
            .drag_offsets
            .iter()
            .filter_map(|(sel, offset)| {
                let laid = pass.layouts.find(*sel)?;
                let mut to = at + *offset;
                if pass.view.linear {
                    to = clamp_into(laid.height, to, rect.height());
                }
                Some((*sel, to))
            })
            .collect();
        if !down {
            // **Letting go somewhere you cannot see sends the view after it.** A drag can
            // carry a node out over a side panel or clean off the edge — the cull has no
            // exemption for the thing in hand — so the drop can put it down with nothing on
            // screen to show for it. The glide is set from `moves` rather than from the
            // graph, because the graph is still a frame behind: the `MoveNodes` below lands
            // next frame and these are the positions it lands.
            state.pan_target = reveal_pan(&moves, pass.layouts, &state.transform, origin, rect)
                .map(|pan| Transform {
                    pan,
                    ..state.transform
                });
        }
        if !moves.is_empty() {
            fx.commands.push(Command::MoveNodes { moves });
        }
        // Let go on a cable that lit: the node goes into it, after the move that put it
        // there, as a step of its own.
        if !down
            && let [(node, _)] = state.drag_offsets[..]
            && let Some((i, input, output)) = pass.hover.insert
            && let Some(cable) = pass.frame.graph.connections().get(i)
        {
            fx.commands.push(Command::Splice {
                node,
                from: cable.from,
                to: cable.to,
                input,
                output,
            });
        }
    }
    // A drag that ends on a tab shows the nodes there too. The canvas cannot see the tab bar,
    // so it reports the drag and `App` joins it to the tab under the pointer. The offsets are
    // cleared here and nowhere else, so one frame carries the release.
    fx.node_drag = Some(NodeDrag {
        nodes: state.selected.iter().copied().collect(),
        released: !down,
    });
    if !down {
        state.drag_offsets.clear();
        state.splice = None;
    }
}

/// Every cable's click handle, interacted with last so a port cannot outrank it.
///
/// Registered now rather than where the curve was painted: a cable's own 12-point click
/// target can sit close enough to one of its endpoint node's *other* ports — not the one it is
/// even plugged into — that the port's own hit rect would cover the same pixels. Within one
/// frame, egui hands a tied hit test to whichever widget was interacted with last, so a port
/// registered after the cable was simply always going to win one. Registering the cable after
/// every node and port instead — same rect, same id, only later — is what makes the cable win
/// instead, without changing anything about where a port itself is clickable.
fn cable_handles(
    ui: &mut Ui,
    pass: &Pass<'_>,
    handles: Vec<(crate::graph::Connection, Pos2, bool)>,
    fx: &mut Effects,
) {
    let graph = pass.frame.graph;
    for (connection, mid, hovered) in handles {
        let handle = ui.interact(
            eframe::egui::Rect::from_center_size(mid, eframe::egui::vec2(12.0, 12.0)),
            ui.id().with(("cable", connection.from, connection.to)),
            Sense::click(),
        );
        crate::ui::accessible(
            &handle,
            eframe::egui::WidgetType::Other,
            format_args!(
                "cable {}{}.{} to {}{}.{}",
                graph.get(connection.from.node).map_or("", |n| n.def.slug),
                connection.from.node,
                connection.from.key,
                graph.get(connection.to.node).map_or("", |n| n.def.slug),
                connection.to.node,
                connection.to.key,
            ),
        );

        // Double-click, not click. A cable is deleted by the pointer being near it, cables
        // leaving one output run side by side, and crossings are ordinary — so a single click
        // near a bundle destroys a connection on the way to doing something else. Removal is
        // the one gesture here with nothing to fall back on but undo, and it is worth the
        // second click. This one edge, not everything feeding that input: action inputs take
        // many.
        if handle.double_clicked() || (hovered && pass.view.background.double_clicked()) {
            fx.commands.push(Command::DisconnectEdge {
                from: connection.from,
                to: connection.to,
            });
        }
    }
}

/// The rubber band, and the selection it makes.
fn marquee(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>) {
    let Some((anchor, ref base)) = state.marquee else {
        return;
    };
    let (t, origin, theme) = (state.transform, pass.view.origin, pass.frame.theme);
    let now = ui
        .ctx()
        .pointer_latest_pos()
        .map_or(anchor, |p| t.to_world(origin, p));
    let band = Rect::from_two_pos(anchor, now);
    let on_screen =
        Rect::from_two_pos(t.to_screen(origin, band.min), t.to_screen(origin, band.max));
    let painter = &pass.view.painter;
    painter.rect_filled(on_screen, 0.0, theme.primary().gamma_multiply(0.12));
    painter.rect_stroke(
        on_screen,
        0.0,
        Stroke::new(1.0, theme.primary()),
        eframe::egui::StrokeKind::Inside,
    );

    // Touching, not containing: a band has to be dragged around a node either way, and
    // requiring the whole of a tall Output inside it makes the gesture fussy.
    //
    // Recomputed every frame from the selection the band started with, so shrinking the band
    // lets a node go again and the highlight tracks the pointer.
    let mut now_selected = base.clone();
    for laid in pass.layouts.iter() {
        if band.intersects(laid.rect) {
            now_selected.insert(laid.id);
        }
    }
    state.selected = now_selected;
    if !pass.view.background.dragged() {
        state.marquee = None;
    }
}

/// The background's right-click: the selection's menu, or the browser.
///
/// With several nodes selected, right-clicking the canvas opens their menu. That is the one
/// time the pointer has no single node to be over, and a menu that acts on all of them is
/// what a right-click anywhere in the editor means then. Otherwise the canvas asks for a
/// node, as it does in silvia: the background is where the pointer is when the answer is
/// "something new here", and the node lands under it. One node's menu is on that node.
fn background_menu(state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    let background = &pass.view.background;
    if state.selected.len() > 1 {
        let chosen: Vec<NodeId> = state.selected.iter().copied().collect();
        let menu_at = state.menu_at;
        background.context_menu(|ui| selection_menu(ui, pass.frame, &chosen, menu_at, fx));
    } else if background.secondary_clicked()
        && let Some(p) = background.interact_pointer_pos()
    {
        state.browser = Some(browse::Browser::new(
            p,
            state.transform.to_world(pass.view.origin, p),
        ));
    }
}

/// Control popups, above every node: the one `open` names, drawn now so no node painted
/// later can cover it.
fn popups(
    ui: &mut Ui,
    state: &mut CanvasState,
    pass: &Pass<'_>,
    previously: Option<OpenControl>,
    mut open: Option<OpenControl>,
    fx: &mut Effects,
) {
    let (graph, theme) = (pass.frame.graph, pass.frame.theme);
    let (t, origin) = (state.transform, pass.view.origin);
    // A picker with nothing in it is not a choice: an option whose kind of file the project
    // holds none of goes straight to the dialog, so the first import is the gesture it
    // always was rather than that gesture behind an empty menu.
    if let Some(OpenControl::Asset { node, key, .. }) = open
        && offered(graph, node, key, pass.frame.assets)
            .next()
            .is_none()
    {
        open = Some(OpenControl::File { node, key });
    }
    // A file request is not a popup: hand it to the app and leave nothing open.
    if let Some(OpenControl::File { node, key }) = open {
        fx.file_requests.push((node, key));
        open = None;
    }
    state.open = open;
    let allow_dismiss = previously == state.open;
    // Each popup answers whether it is done — chosen from, or its control gone — and whether
    // a click landed away from it, which closes it on any frame but the one it opened on.
    let (done, dismissed) = match state.open {
        Some(OpenControl::Color { node, key, at }) => {
            let at = t.to_screen(origin, at);
            let value = graph
                .get(node)
                .and_then(|n| n.controls.get(key))
                .and_then(|v| match v {
                    crate::graph::ControlValue::Color(c) => Some(*c),
                    crate::graph::ControlValue::Float(_) => None,
                });
            match value {
                Some(value) => {
                    let (changed, dismissed) = color::picker(ui, at, (node, key), value, theme);
                    if let Some(next) = changed {
                        fx.commands.push(Command::SetControl {
                            node,
                            key,
                            value: crate::graph::ControlValue::Color(next),
                        });
                    }
                    (false, dismissed)
                }
                None => (true, false),
            }
        }
        Some(OpenControl::Select {
            node,
            key,
            at,
            width,
        }) => {
            let at = t.to_screen(origin, at);
            let width = width * t.zoom;
            let option = graph.get(node).and_then(|n| n.def.option(key));
            let choices = option.map_or(&[][..], crate::nodes::OptionDef::menu);
            let devices = option.is_some_and(|o| o.devices);
            match graph.get(node).and_then(|n| n.options.get(key)).cloned() {
                Some(current) => {
                    let picked =
                        node_widget::select_popup(ui, at, width, &current, choices, devices, theme);
                    if let Some(value) = picked.chosen {
                        fx.commands.push(Command::SetOption { node, key, value });
                    }
                    fx.look_for_devices |= picked.look_again;
                    (false, picked.dismissed)
                }
                None => (true, false),
            }
        }
        Some(OpenControl::Asset {
            node,
            key,
            at,
            width,
        }) => {
            let at = t.to_screen(origin, at);
            let width = width * t.zoom;
            let current = graph
                .get(node)
                .and_then(|n| n.options.get(key))
                .cloned()
                .unwrap_or_default();
            let offered: Vec<node_widget::AssetChoice<'_>> =
                offered(graph, node, key, pass.frame.assets)
                    .map(|info| node_widget::AssetChoice {
                        picture: pass
                            .frame
                            .posters
                            .get(&info.reference)
                            .and_then(Option::as_ref),
                        info,
                    })
                    .collect();
            let picked = node_widget::asset_popup(ui, at, width, &current, &offered, theme);
            match picked.chosen {
                Some(node_widget::Pick::Asset(reference)) => {
                    fx.commands.push(Command::SetOption {
                        node,
                        key,
                        value: reference,
                    });
                    (true, false)
                }
                // The dialog, on the frame after this one: the same request the button made
                // before there was anything to put in front of it.
                Some(node_widget::Pick::File) => {
                    state.open = Some(OpenControl::File { node, key });
                    (false, false)
                }
                None => (false, picked.dismissed),
            }
        }
        Some(OpenControl::Live { node, at }) => {
            let at = t.to_screen(origin, at);
            let sources = pass
                .frame
                .live
                .get(&node)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let picked = node_widget::live_popup(ui, at, sources, theme);
            let chosen = picked.chosen.and_then(|i| sources.get(i));
            if let Some(source) = chosen {
                if source.panel {
                    fx.reveal_main_input = true;
                } else if let Some(workspace) = source.workspace {
                    fx.navigate = Some((workspace, source.node));
                }
            }
            (chosen.is_some(), picked.dismissed)
        }
        // The range editor answers the first-frame rule itself.
        Some(OpenControl::Range { node, key, at }) => {
            let at = t.to_screen(origin, at);
            (
                !range_editor(ui, pass, node, key, at, allow_dismiss, fx),
                false,
            )
        }
        Some(OpenControl::File { .. }) | None => (false, false),
    };
    if done || (dismissed && allow_dismiss) {
        state.open = None;
    }
}

/// The range editor a number control's right-click opens. False when it is to close: dismissed,
/// reset, learning, or the control is gone.
fn range_editor(
    ui: &mut Ui,
    pass: &Pass<'_>,
    node: NodeId,
    key: &'static str,
    at: Pos2,
    allow_dismiss: bool,
    fx: &mut Effects,
) -> bool {
    let graph = pass.frame.graph;
    let spec = graph.get(node).and_then(|n| {
        let def = n.def;
        let declared = crate::nodes::default_range(n, key)?;
        let value = match n.controls.get(key)? {
            crate::graph::ControlValue::Float(v) => *v,
            crate::graph::ControlValue::Color(_) => return None,
        };
        let (default, unit, log) = match &def.input(key)?.control {
            crate::nodes::Control::Number {
                default, unit, log, ..
            } => (*default, *unit, *log),
            _ => return None,
        };
        Some(number::NumberSpec {
            value,
            default,
            range: n
                .values
                .get(key)
                .and_then(crate::graph::Value::range)
                .unwrap_or(declared),
            declared,
            unit,
            log,
            // The mixer's own readout of a bound control, not a port on a node: a cable
            // never lands here.
            varying: false,
            learning: false,
            ghost: None,
        })
    });
    let Some(spec) = spec else {
        return false;
    };
    // The same name the control itself carries, so a test or the agent finds the editor by
    // the control it belongs to.
    let slug = graph.get(node).map_or("", |n| n.def.slug);
    let name = format!("{slug}{node}.{key}");
    let bound = pass
        .frame
        .bindings
        .trigger_of(PortRef::new(node, key))
        .map(crate::midi::Trigger::label);
    let bindable = graph
        .get(node)
        .is_some_and(|n| n.def.bindable(key).is_some());
    let midi = match &bound {
        Some(trigger) => number::MidiRow::Bound(trigger),
        None if bindable => number::MidiRow::Unbound,
        None => number::MidiRow::Unbindable,
    };
    let edit = number::range_popup(ui, at, &name, &spec, midi, pass.frame.theme);
    let mut keep = true;
    if edit.learn {
        fx.learn_midi = Some(PortRef::new(node, key));
        keep = false;
    }
    if edit.unbind {
        fx.unbind_midi = Some(PortRef::new(node, key));
    }
    if let Some(range) = edit.range {
        fx.commands.push(Command::SetRange { node, key, range });
    }
    if let Some(value) = edit.value {
        fx.commands.push(Command::SetControl {
            node,
            key,
            value: crate::graph::ControlValue::Float(value),
        });
    }
    if edit.reset {
        if let Some(n) = graph.get(node) {
            fx.commands
                .extend(node_widget::reset_control(n, node, key, spec.default));
        }
        keep = false;
    }
    if edit.dismissed && allow_dismiss {
        keep = false;
    }
    keep
}

/// The browser, and the key that opens it.
///
/// `/`, `~` and `` ` `` are silvia's quake bar: the list at the top of the canvas, and the
/// node lands in the middle of the view rather than under a pointer that was never asked to
/// be anywhere. Not while something is taking typing, which includes the browser's own
/// search field — a `/` typed into it is a search for a slash. Not while the conversion menu
/// is up either: it is a list of nodes on the canvas, and two of those at once is one too
/// many — and both read the arrows and `Enter`.
fn browser(ui: &mut Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    let rect = pass.view.rect;
    if state.browser.is_none() && state.bridging.is_none() && !ui.ctx().egui_wants_keyboard_input()
    {
        let asked = ui.input_mut(|i| {
            [
                (Modifiers::NONE, Key::Backtick),
                (Modifiers::SHIFT, Key::Backtick),
                (Modifiers::NONE, Key::Slash),
                (Modifiers::NONE, Key::Questionmark),
                (Modifiers::SHIFT, Key::Questionmark),
            ]
            .into_iter()
            .any(|(m, k)| i.consume_key(m, k))
        });
        if asked {
            let at = Pos2::new(rect.center().x - browse::WIDTH * 0.5, rect.min.y + 16.0);
            let middle = state.transform.to_world(pass.view.origin, rect.center());
            let landing = middle
                - vec2(
                    canvas::NODE_WIDTH * 0.5,
                    canvas::HEADER_HEIGHT + canvas::PORT_PITCH,
                );
            state.browser = Some(browse::Browser::new(at, landing));
        }
    }
    let Some(browser) = state.browser.as_mut() else {
        return;
    };
    let outcome = browse::show(ui, browser, rect, pass.frame.theme, pass.frame.clipboard);
    if let Some(slug) = outcome.chosen {
        let (at, workspace) = (browser.landing(), pass.frame.workspace);
        // A loose cable's browser lands the node wired, in the one step a bridge is.
        fx.commands.push(
            match browser.cable().and_then(|c| Some((c.end, c.key(slug)?))) {
                Some((end, key)) => Command::AddConnected {
                    slug,
                    at,
                    workspace,
                    end,
                    key,
                },
                None => Command::AddNode {
                    slug,
                    at,
                    workspace,
                },
            },
        );
    }
    // The browser's Paste row lands the clip where a node chosen from the same panel would
    // land: the background right-click means *something here*, and a paste is one of the two
    // things that can be.
    if outcome.paste {
        fx.clip = Some(ClipAction::Paste {
            at: Some(browser.landing()),
        });
    }
    if outcome.chosen.is_some() || outcome.dismissed || outcome.paste {
        state.browser = None;
    }
}

/// The conversion menu, for a cable released where a node has to carry it.
///
/// Beside the port it was released on, which is where the cable ended: the panel hangs off
/// the port's own screen position, recomputed every frame from the world point, so a pan or a
/// zoom under it does not detach it from the port it is about.
fn bridge_menu(ui: &mut Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    let graph = pass.frame.graph;
    let Some(bridging) = state.bridging.as_mut() else {
        return;
    };
    let at = state.transform.to_screen(pass.view.origin, bridging.at());
    let outcome = bridge::show(ui, bridging, graph, pass.view.rect, at, pass.frame.theme);
    if let Some(cast) = outcome.chosen {
        // The midpoint of the two nodes' own positions, which is silvia's: the converter
        // stands between the two it was made for.
        let midpoint = |a: NodeId, b: NodeId| {
            let pos = |id: NodeId| graph.get(id).map(|n| n.pos);
            match (pos(a), pos(b)) {
                (Some(a), Some(b)) => a + (b - a) * 0.5,
                _ => bridging.at(),
            }
        };
        fx.commands.push(Command::Bridge {
            from: bridging.from,
            to: bridging.to,
            slug: cast.def.slug,
            inputs: cast.inputs,
            output: cast.output,
            at: midpoint(bridging.from.node, bridging.to.node),
            workspace: pass.frame.workspace,
        });
    }
    if outcome.chosen.is_some() || outcome.dismissed {
        state.bridging = None;
    }
}

/// `Escape` with something in the hand puts it back: a node drag goes back where it started,
/// and a cable being dragged is dropped where it is, landing on nothing.
///
/// Read at the top of the pass, before the view creeps or a node is placed, so the frame the
/// key lands on moves nothing. egui abandons every widget drag on `Escape` by itself, so the
/// grip that started either one does not take it up again while the button is still down;
/// what is left is the canvas's own half, which is this.
fn escape_from_hand(ui: &Ui, state: &mut CanvasState, fx: &mut Effects) {
    if state.drag_offsets.is_empty() && state.dragging.is_none() {
        return;
    }
    if !ui.input_mut(|i| i.consume_shortcut(&menu::keys::CANCEL)) {
        return;
    }
    if !state.drag_offsets.is_empty() {
        // In the order the drag moved them, which is the order the step it opened names them.
        fx.cancel_node_drag = Some(state.drag_offsets.drain(..).map(|(id, _)| id).collect());
        state.splice = None;
    }
    state.dragging = None;
    state.reach = None;
    state.drag_hue = None;
}

/// How near the pointer has to be to a cable, in screen points, for a node carried over it to
/// light it. Twice a click's reach: the node's own body is over the cable at that moment, so
/// the hand is aiming at something it cannot see.
const INSERT_HIT: f32 = 12.0;

/// The cable the one node in hand would go into, and through which of its ports.
///
/// Only one node, and one with no cables of its own ([`crate::nodes::attach::splice_ports`]):
/// a selection has no one input and one output to put into a cable, and a node already
/// patched would be rewired by a drop meant only to move it.
fn insert_target(
    state: &mut CanvasState,
    pass: &Pass<'_>,
) -> Option<(usize, &'static str, &'static str)> {
    let [(node, _)] = state.drag_offsets[..] else {
        return None;
    };
    let p = pass.on_canvas?;
    let graph = pass.frame.graph;
    let (index, _) = graph
        .connections()
        .iter()
        .enumerate()
        .filter(|(_, c)| c.from.node != node && c.to.node != node)
        .filter_map(|(i, c)| {
            let (from, to) = (pass.find(c.from)?, pass.find(c.to)?);
            let d = cable::Curve::new(
                from.center,
                to.center,
                from.ty,
                pass.clearance(c.from.node, c.to.node),
                pass.frame.prefs.cable_droop,
            )
            .distance_to(p);
            (d < INSERT_HIT).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let cable = graph.connections()[index];
    let ports = match state.splice {
        Some(answer) if answer.node == node && answer.cable == cable => answer.ports,
        _ => {
            let ports = crate::nodes::attach::splice_ports(graph, node, cable.from, cable.to);
            state.splice = Some(SpliceAnswer { node, cable, ports });
            ports
        }
    };
    ports.map(|(input, output)| (index, input, output))
}

/// The canvas's own keys: delete, duplicate, select all, clear, and the two frames.
///
/// Read after every node, popup and menu on the canvas has had its turn, so a key a control
/// consumed never reaches here — and then only where [`keys_free`] says nothing else holds the
/// keyboard. Each is a constant in [`menu::keys`], the one table of keys there is.
fn keys(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    use menu::keys::{
        CANCEL, DELETE, DELETE_BACK, DUPLICATE, FRAME_ALL, FRAME_SELECTED, SELECT_ALL,
    };
    if !keys_free(ui, state, pass) {
        return;
    }
    let [delete, duplicate, all, cancel, frame_all, frame_selected] = ui.input_mut(|i| {
        [
            i.consume_shortcut(&DELETE) | i.consume_shortcut(&DELETE_BACK),
            i.consume_shortcut(&DUPLICATE),
            i.consume_shortcut(&SELECT_ALL),
            i.consume_shortcut(&CANCEL),
            i.consume_shortcut(&FRAME_ALL),
            i.consume_shortcut(&FRAME_SELECTED),
        ]
    });
    // The selection on this workspace: one selection is held across the project, and a key
    // must not act on a node nobody can see.
    let here: Vec<NodeId> = state
        .selected
        .iter()
        .copied()
        .filter(|id| pass.layouts.find(*id).is_some())
        .collect();
    if delete && !here.is_empty() {
        fx.commands.push(Command::RemoveNodes(here.clone()));
    }
    if duplicate && !here.is_empty() {
        fx.commands.push(Command::Duplicate {
            nodes: here.clone(),
            offset: DUPLICATE_OFFSET,
        });
    }
    if all {
        state.selected = pass.layouts.iter().map(|l| l.id).collect();
    }
    if cancel {
        state.clear_selection();
    }
    let rect = pass.view.rect;
    if frame_all && let Some(world) = content_bounds(pass.layouts) {
        state.pan_target = Some(framed(world, rect, pass.view.linear, state.bounds));
    }
    if frame_selected
        && let Some(world) = here
            .iter()
            .filter_map(|id| pass.layouts.find(*id).map(|l| l.rect))
            .reduce(Rect::union)
    {
        let world = world.expand(STRIP_MARGIN);
        state.pan_target = Some(framed(world, rect, pass.view.linear, state.bounds));
    }
}

/// Whether the canvas's keys are the canvas's to read this pass.
///
/// Not while a field has the keyboard, or had it when the last pass ended — `Escape` takes
/// focus away before anything is drawn, so the key that left a field would otherwise also
/// clear the selection. Not while a popup, a control's popup, the browser, the conversion
/// menu, the Nodes menu or a modal question is up, each of which answers keys of its own. Not
/// with anything in the hand. And not while a number control is under the pointer, whose
/// keys are read on hover: a `Delete` aimed at a parameter is not aimed at its node.
fn keys_free(ui: &Ui, state: &CanvasState, pass: &Pass<'_>) -> bool {
    let ctx = ui.ctx();
    ui.input(|i| i.viewport().focused.unwrap_or(true))
        && !state.keyboard_elsewhere
        && !ctx.egui_wants_keyboard_input()
        && !eframe::egui::Popup::is_any_open(ctx)
        && ctx.memory(|m| m.is_above_modal_layer(ui.layer_id()))
        && state.open.is_none()
        && state.browser.is_none()
        && state.bridging.is_none()
        && state.drag_offsets.is_empty()
        && state.dragging.is_none()
        && !pass.frame.nodes_menu_open
        && !number::hovered_this_pass(ctx)
}

/// The view that fits `world` into `rect`: centred, at the zoom that shows all of it, and
/// never past actual size, since a frame is for seeing where things are rather than for
/// reading one up close.
///
/// A strip has one zoom and one height, so there it is the pan along the strip alone, held to
/// the strip's ends as any pan is; and where `world` is longer than the view, its start, for
/// the reason a drop's reveal shows the near edge of something too big to show.
fn framed(world: Rect, rect: Rect, linear: bool, bounds: Option<Rect>) -> Transform {
    let size = rect.size();
    let zoom = if linear {
        1.0
    } else {
        (size.x / world.width())
            .min(size.y / world.height())
            .clamp(canvas::MIN_ZOOM, 1.0)
    };
    let mut t = Transform {
        pan: size * 0.5 - world.center().to_vec2() * zoom,
        zoom,
    };
    if linear {
        if world.width() > size.x {
            t.pan.x = -world.min.x;
        }
        clamp_pan(&mut t, rect, bounds);
    }
    t
}

/// While a cable is dragged, a port under the pointer that cannot take it says why, beside the
/// pointer: the dim says *no*, and this says what kind of no.
///
/// Only for the dimmed state. A port the cable fits needs no words, and a convertible one has
/// its ring and its menu.
fn refusal(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>) {
    let (Some(src), Some(port), Some(p)) = (state.dragging, pass.hover.port, pass.pointer) else {
        return;
    };
    if port == src {
        return;
    }
    let graph = pass.frame.graph;
    let reach = drag_reach(&mut state.reach, graph, src);
    if legal(graph, reach, src, port) || bridgeable(graph, reach, src, port) {
        return;
    }
    let Some(words) = refusal_words(graph, reach, src, port) else {
        return;
    };
    // Below and right of the pointer, where a tooltip goes, and letting the pointer through:
    // the canvas under it is still where the cable lands.
    eframe::egui::Area::new(ui.id().with("cable-refusal"))
        .order(eframe::egui::Order::Tooltip)
        .interactable(false)
        .fixed_pos(p + vec2(14.0, 18.0))
        .show(ui.ctx(), |ui| {
            eframe::egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(words);
            });
        });
}

/// Why a cable from `a` cannot land on `b`, in the words a hand reads: `ConnectError`, said
/// as what the two ports carry rather than what the graph calls them.
fn refusal_words(
    graph: &Graph,
    reach: &crate::graph::Reach,
    a: PortRef,
    b: PortRef,
) -> Option<String> {
    use crate::graph::ConnectError;
    let Some((from, to)) = orient(graph, a, b) else {
        let output = graph.get(a.node)?.output(a.key).is_some();
        return Some(
            if output {
                "an output can't take a cable"
            } else {
                "an input can't feed an input"
            }
            .to_owned(),
        );
    };
    match graph.can_connect_within(reach, from, to) {
        Err(ConnectError::SelfConnection(_)) => Some("a node can't feed itself".to_owned()),
        Err(ConnectError::WouldCycle { .. }) => Some("would make a loop".to_owned()),
        Err(ConnectError::Inactive(_)) => Some("the time mode puts this row away".to_owned()),
        Err(
            ConnectError::TypeMismatch { from, to } | ConnectError::ActionMismatch { from, to },
        ) => Some(format!("{} can't take {}", carried(to), carried(from))),
        Ok(()) | Err(ConnectError::NoSuchNode(_) | ConnectError::NoSuchPort(_)) => None,
    }
}

/// What a port carries, as a refusal says it: a picture, a field, a number, a color or an
/// event.
fn carried(ty: PortType) -> &'static str {
    match ty {
        PortType::VaryingColor => "a picture",
        PortType::VaryingNumber => "a field",
        PortType::UniformNumber => "a number",
        PortType::UniformColor => "a color",
        PortType::Action => "an event",
    }
}

/// A cable let go over empty canvas: the browser at the pointer, holding only the kinds that
/// can take it, with the cable drawn to where it was let go. Nothing at all where no kind can.
///
/// The node lands with the port that takes the cable near the pointer: to the right of it for
/// a cable out of an output, which runs on rightwards into the new node, and to the left for
/// one dragged back out of an input.
fn loose_cable(state: &mut CanvasState, pass: &Pass<'_>, end: PortRef, p: Pos2) {
    let graph = pass.frame.graph;
    let takers = crate::nodes::attach::takers(graph, end, pass.frame.workspace);
    if takers.is_empty() {
        return;
    }
    let world = state.transform.to_world(pass.view.origin, p);
    let row = vec2(0.0, canvas::HEADER_HEIGHT + canvas::PORT_PITCH * 0.5);
    let out_of_output = graph
        .get(end.node)
        .is_some_and(|n| n.output(end.key).is_some());
    let landing = if out_of_output {
        world - row
    } else {
        world - row - vec2(canvas::NODE_WIDTH, 0.0)
    };
    state.browser = Some(browse::Browser::for_cable(
        p,
        landing,
        browse::Cable {
            end,
            at: world,
            takers,
        },
    ));
}

/// How many nodes are selected on this workspace, beside the Nodes button while there are
/// any: a paste on top of its originals, a select-all, a band — each of which is otherwise a
/// change nobody can see.
///
/// **Fixed-width**, as wide as four digits, so the count changing moves nothing and the label
/// never grows into what is beside it.
fn selection_count(ui: &Ui, state: &CanvasState, pass: &Pass<'_>) {
    let count = state
        .selected
        .iter()
        .filter(|id| pass.layouts.find(**id).is_some())
        .count();
    if count == 0 {
        return;
    }
    let theme = pass.frame.theme;
    let rect = pass.view.rect;
    let font = eframe::egui::FontId::monospace(theme::FONT_TINY);
    let widest = ui
        .painter()
        .layout_no_wrap("0000 selected".to_owned(), font.clone(), theme.text_muted())
        .size();
    let button = start::button_rect(ui.ctx());
    let left = button.map_or(rect.min.x, |b| b.max.x) + SELECTION_COUNT_GAP;
    let middle = button.map_or(
        rect.max.y - SELECTION_COUNT_GAP - SELECTION_COUNT_HEIGHT,
        |b| b.center().y,
    );
    let at = Rect::from_min_size(
        Pos2::new(left, middle - SELECTION_COUNT_HEIGHT * 0.5),
        vec2(widest.x + SELECTION_COUNT_PAD * 2.0, SELECTION_COUNT_HEIGHT),
    );
    let text = format!("{count} selected");
    let painter = &pass.view.painter;
    painter.rect_filled(
        at,
        eframe::egui::CornerRadius::same(theme::RADIUS_SM),
        theme.bg_secondary(),
    );
    painter.text(
        at.left_center() + vec2(SELECTION_COUNT_PAD, 0.0),
        eframe::egui::Align2::LEFT_CENTER,
        &text,
        font,
        theme.text_muted(),
    );
    // Named apart from the text it shows: the selection's own menu is headed `3 selected`, a
    // name is unique on screen, and a test asks whether that menu has a count by the word.
    let w = ui.interact(at, ui.id().with("selection-count"), Sense::hover());
    let noun = if count == 1 { "node" } else { "nodes" };
    accessible(
        &w,
        eframe::egui::WidgetType::Label,
        format_args!("selection: {count} {noun}"),
    );
}

/// The selection count's height, its gap from the Nodes button and its padding inside.
const SELECTION_COUNT_HEIGHT: f32 = 18.0;
const SELECTION_COUNT_GAP: f32 = 8.0;
const SELECTION_COUNT_PAD: f32 = 6.0;

/// A cable let go: connected to the port it was let go on, or offered through a node.
fn drop_cable(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    let graph = pass.frame.graph;
    let Some(src) = state.dragging else {
        return;
    };
    if !ui.ctx().input(|i| i.pointer.any_released()) {
        return;
    }
    // The port under the pointer by the same test the hover makes, so a cable lands on the
    // port that lit under it and on no other: the nearest whose square holds the pointer, of
    // those it can land on, and only then those a node between the two could carry it to.
    // Only on the canvas itself, as the hover is, so a cable let go over a window, a panel or
    // the tab bar lands on nothing under it.
    let t = state.transform;
    let near = |pick: &dyn Fn(&PortSlot) -> bool, p: Pos2| {
        node_widget::port_at(&pass.slots, &pass.bodies, p, &t, pick)
    };
    let reach = drag_reach(&mut state.reach, graph, src);
    if let Some(p) = pass.on_canvas {
        if let Some(target) = near(&|s| legal(graph, reach, src, s.port), p)
            && let Some((from, to)) = orient(graph, src, target.port)
        {
            // The wire in flight keeps the color it has been wearing: the command lands next
            // frame, and the cable it makes finds its step already here rather than taking a
            // fresh one and changing color as it connects.
            if let Some(step) = state.drag_hue {
                state.cable_hues.insert((from, to), step);
            }
            fx.commands.push(Command::Connect { from, to });
        } else if let Some(target) = near(&|s| bridgeable(graph, reach, src, s.port), p)
            && let Some((from, to)) = orient(graph, src, target.port)
        {
            // A cable released on a convertible port is not dropped: the menu opens at the
            // port and the cable is waiting on an answer.
            state.bridging = Some(bridge::Bridging::new(
                from,
                to,
                state.transform.to_world(pass.view.origin, target.center),
            ));
        } else if near(&|_| true, p).is_none() && !pass.bodies.iter().any(|(_, r)| r.contains(p)) {
            loose_cable(state, pass, src, p);
        }
    }
    state.dragging = None;
    state.reach = None;
    // Spent either way. A drag that landed has handed its step to the cable; one that did not
    // has nothing to hand it to, and a bridge makes *two* cables, neither of which is the one
    // this step was reserved for — both take their own.
    state.drag_hue = None;
}

/// The wheel, after every control has had its chance at it.
///
/// A number control under the pointer takes the scroll and clears it. Read at the top of the
/// frame this ran first, so one notch over a control both scrubbed the parameter and zoomed
/// the canvas out from under it.
///
/// The one gate on every reading of the wheel below. `contains_pointer` is egui's own hit
/// test: it drops this response wherever a widget on a higher layer covers the point, and its
/// rect is the canvas's own rather than the window's, so a menu, a popup and a panel are all
/// outside the canvas. A glide in flight is not wheel input, so it passes the gate. See
/// docs/ui.md, "Interaction rules".
fn wheel(ui: &Ui, state: &mut CanvasState, pass: &Pass<'_>) {
    let over = pass.view.background.contains_pointer();
    if pass.view.linear {
        scroll_the_strip(
            ui,
            state,
            pass.view.rect,
            over,
            pass.frame.prefs.scroll_x_inverted,
        );
    } else if let Some(p) = pass.pointer
        && over
    {
        let scroll = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            state
                .transform
                .zoom_about(pass.view.origin, p, (scroll * 0.002).exp());
            // A hand on the view outranks a glide, the zoom of a frame included.
            state.pan_target = None;
        }
    }
}

/// A slim bar under each node, painted after every node so it is never covered and so turning
/// it on moves nothing: the strip hangs off the body rather than being a row of it. Each is a
/// named widget, so a test and the agent-driven layer read it as text.
fn cost_strips(ui: &mut Ui, state: &CanvasState, pass: &Pass<'_>) {
    let costs = pass.frame.costs;
    if costs.is_empty() {
        return;
    }
    let (t, origin, theme) = (state.transform, pass.view.origin, pass.frame.theme);
    let painter = &pass.view.painter;
    for laid in pass.layouts.iter() {
        let id = laid.id;
        let Some(node) = pass.frame.graph.get(id) else {
            continue;
        };
        let Some(cost) = costs.get(&id) else {
            continue;
        };
        let body = laid.rect;
        let strip = Rect::from_min_size(
            t.to_screen(origin, Pos2::new(body.min.x, body.max.y + 2.0)),
            vec2(body.width() * t.zoom, COST_HEIGHT * t.zoom),
        );
        if !strip.intersects(pass.view.rect) {
            continue;
        }
        painter.rect_filled(strip, theme::RADIUS_SHARP, theme.bg_secondary());
        let mut bar = strip;
        bar.set_width(strip.width() * cost.fraction.clamp(0.0, 1.0));
        let ink = if cost.hot {
            theme.accent()
        } else {
            theme.primary()
        };
        painter.rect_filled(bar, theme::RADIUS_SHARP, ink.gamma_multiply(0.35));
        let font = eframe::egui::FontId::monospace(theme::font_size(theme::FONT_TINY, t.zoom));
        painter.text(
            strip.left_center() + vec2(4.0 * t.zoom, 0.0),
            eframe::egui::Align2::LEFT_CENTER,
            &cost.left,
            font.clone(),
            theme.text_primary(),
        );
        if !cost.right.is_empty() {
            // The accent where the bar is hot: the right half of an Output's strip is the
            // drop count, and a drop count exists only when there is one to report.
            painter.text(
                strip.right_center() - vec2(4.0 * t.zoom, 0.0),
                eframe::egui::Align2::RIGHT_CENTER,
                &cost.right,
                font,
                if cost.hot {
                    theme.accent()
                } else {
                    theme.text_primary()
                },
            );
        }
        let slug = node.def.slug;
        let w = ui.interact(strip, ui.id().with(("cost", id)), Sense::hover());
        // The two halves joined by the one space they are read with, with neither an empty
        // half nor a double space: the name is the line, not the layout.
        let gap = if cost.left.is_empty() || cost.right.is_empty() {
            ""
        } else {
            " "
        };
        accessible(
            &w,
            eframe::egui::WidgetType::Label,
            format_args!("{slug}{id} cost {}{gap}{}", cost.left, cost.right),
        );
    }
}

/// The minimap under a strip, and the strip's own controls over its corner.
fn rail(ui: &mut Ui, state: &mut CanvasState, pass: &Pass<'_>, fx: &mut Effects) {
    let rect = pass.view.rect;
    let rail_rect = Rect::from_min_size(
        Pos2::new(rect.min.x, rect.max.y),
        vec2(rect.width(), pass.view.rail_height),
    );
    // **Along the strip, the rectangle the clamp is already holding the view to** — a map of
    // one length and a view held to another is how the two drift apart.
    //
    // **Across it, a whole viewport, always.** Not the nodes' own top and bottom: the band a
    // node can be in is `clamp_to_strip`'s, which is the viewport's height, so framing the map
    // to whatever part of it happens to be occupied makes a node's place on the map mean
    // something different from one frame to the next. A node collapsing would slide every
    // other node up. The frame is the band, and where a node sits in it is where it sits on
    // the strip.
    //
    // The map's one scale then has to fit both, so a short strip leaves the map smaller than
    // the rail with a gutter at each end. That is the price of a frame that holds still, and
    // it is the right way round: the alternative is a map that fills the bar by lying about
    // the shape of the thing it maps.
    //
    // Both of those are `strip_bounds` itself, so there is one rectangle again: the map and
    // the clamp cannot disagree about either axis.
    let held = state.bounds;
    // Being sent somewhere by the map outranks being on the way somewhere, as `center_on`'s
    // own send does: a glide left running pulls the view back to where it was going.
    if rail::show(
        ui,
        rail_rect,
        rect,
        pass.frame,
        pass.layouts,
        &mut state.transform,
        held,
    ) {
        state.pan_target = None;
    }
    clamp_pan(&mut state.transform, rect, held);
    // silvia's `#workspace-controls`, over the canvas and above the rail. Arrange is an edit
    // and goes to the bus, which is all that is left here: Extend and Crop were buttons for a
    // length the strip now decides for itself.
    if strip::controls(ui, rect, pass.frame.theme) {
        fx.commands.push(Command::AutoArrange {
            workspace: pass.frame.workspace,
            height: rect.height(),
        });
    }
}

/// The project's media this option would accept, in the order `assets/` lists them.
///
/// The node's own definition answers it: what a dialog would filter to is what the picker
/// offers, so the two routes to a file cannot disagree about what the node can play.
fn offered<'a>(
    graph: &Graph,
    node: NodeId,
    key: &str,
    assets: &'a [crate::project::AssetInfo],
) -> impl Iterator<Item = &'a crate::project::AssetInfo> {
    let accepts = graph
        .get(node)
        .and_then(|n| n.def.option(key))
        .filter(|o| o.is_asset())
        .map(|o| o.accepts);
    assets
        .iter()
        .filter(move |a| accepts.is_some_and(|k| k.matches(&a.name)))
}

/// Where the dragged port reaches: the one kept for the drag, walked again only where the
/// port or the graph's cables are not the ones it was walked for.
fn drag_reach<'a>(
    kept: &'a mut Option<crate::graph::Reach>,
    graph: &Graph,
    end: PortRef,
) -> &'a crate::graph::Reach {
    if !kept.as_ref().is_some_and(|r| r.is_current(graph, end)) {
        *kept = Some(graph.reach(end));
    }
    kept.as_ref().expect("just kept")
}

/// Can these two ports be joined, in whichever direction makes sense?
fn legal(graph: &Graph, reach: &crate::graph::Reach, a: PortRef, b: PortRef) -> bool {
    orient(graph, a, b).is_some_and(|(from, to)| graph.can_connect_within(reach, from, to).is_ok())
}

/// Can they be joined *through a node*? The third port state during a drag.
///
/// One question, asked of `nodes::can_bridge`: whether the refusal is a type and nothing
/// else, whether the registry holds a conversion for the pair, and whether a node between the
/// two would close a loop are one rule, and it is not the canvas's.
fn bridgeable(graph: &Graph, reach: &crate::graph::Reach, a: PortRef, b: PortRef) -> bool {
    orient(graph, a, b)
        .is_some_and(|(from, to)| crate::nodes::can_bridge_within(graph, Some(reach), from, to))
}

/// Put the output first. Returns `None` if both are inputs or both are outputs.
fn orient(graph: &Graph, a: PortRef, b: PortRef) -> Option<(PortRef, PortRef)> {
    let a_out = graph.get(a.node)?.output(a.key).is_some();
    let b_out = graph.get(b.node)?.output(b.key).is_some();
    match (a_out, b_out) {
        (true, false) => Some((a, b)),
        (false, true) => Some((b, a)),
        _ => None,
    }
}

/// Scroll the strip, with momentum.
///
/// **The raw delta, not egui's smoothed one.** egui spreads one flick across many frames,
/// which would keep re-arming the fling and never let it decay; the momentum here replaces
/// that smoothing rather than compounding with it.
///
/// **A wheel and a trackpad are different devices and are treated as such.** A mouse wheel
/// reports in lines and only ever on one axis, so its notches move the view *along* the
/// strip — that is the whole reason the mode exists. A trackpad reports in points and has
/// two axes of its own, so it pans in both, and the vertical goes as far as there is content
/// to reach.
fn scroll_the_strip(
    ui: &Ui,
    state: &mut CanvasState,
    rect: eframe::egui::Rect,
    over: bool,
    inverted: bool,
) {
    use eframe::egui::{Event, MouseWheelUnit, Vec2};

    let per_line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let (raw, by_wheel, dt) = ui.ctx().input(|i| {
        let mut raw = Vec2::ZERO;
        let mut by_wheel = false;
        for event in &i.events {
            if let Event::MouseWheel { unit, delta, .. } = event {
                raw += match unit {
                    MouseWheelUnit::Point => *delta,
                    MouseWheelUnit::Line => per_line * *delta,
                    MouseWheelUnit::Page => rect.height() * *delta,
                };
                by_wheel |= *unit != MouseWheelUnit::Point;
            }
        }
        (raw, by_wheel, i.stable_dt)
    });
    let dt = dt.clamp(1.0 / 1000.0, 1.0 / 15.0);

    let delta = if raw == Vec2::ZERO || !over {
        Vec2::ZERO
    } else if by_wheel {
        Vec2::new(raw.x + raw.y, 0.0)
    } else {
        raw
    };
    // Along the strip only. A wheel that scrolls the wrong way here is unusable rather than
    // merely annoying, because Linear mode is the one place the wheel means *x* — and which
    // way that should go is a property of the hand, not of the workspace.
    let delta = if inverted {
        Vec2::new(-delta.x, delta.y)
    } else {
        delta
    };

    if delta != Vec2::ZERO {
        // A hand on the view outranks a glide, as it outranks everything else here.
        state.pan_target = None;
    }
    if delta == Vec2::ZERO {
        state.transform.pan += state.fling * dt;
        state.fling *= FLING_DECAY.powf(dt);
        if state.fling.length() < FLING_STOP {
            state.fling = Vec2::ZERO;
        }
    } else {
        state.transform.pan += delta;
        // Blended rather than replaced, so the launch speed comes from the flick and not
        // from whichever single frame the fingers happened to leave on.
        let launch = delta / dt;
        let mut next = state.fling * 0.4 + launch * 0.6;
        if next.length() > FLING_MAX {
            next = next.normalized() * FLING_MAX;
        }
        state.fling = next;
    }

    // A glide that has reached the end of the strip is over. Left running, it would keep
    // pushing against the clamp and spring back the moment the view could move again.
    let before = state.transform.pan;
    clamp_pan(&mut state.transform, rect, state.bounds);
    if state.transform.pan.x != before.x {
        state.fling.x = 0.0;
    }
    if state.transform.pan.y != before.y {
        state.fling.y = 0.0;
    }
}

/// The world rectangle one workspace occupies, with the strip's margin around it.
///
/// `None` for an empty workspace: there is nothing to clamp against.
pub fn content_bounds(layouts: &canvas::Layouts) -> Option<Rect> {
    let mut bounds: Option<Rect> = None;
    for laid in layouts.iter() {
        let r = laid.rect;
        bounds = Some(bounds.map_or(r, |b: Rect| b.union(r)));
    }
    bounds.map(|b| b.expand(STRIP_MARGIN))
}

/// The world rectangle a linear workspace's strip covers: its content, out to a whole
/// viewport in each direction at the least.
///
/// **The whole of how long a strip is.** The far end is the last node or a window's width
/// from the first, whichever is further out — silvia's
/// `Math.max(getMinWorkspaceWidth(), editorWidth)` — because a strip shorter than the window
/// is a strip with a gap at the end of it that nothing can scroll to. There is nothing
/// stored: no extent, no target, nothing in the file. A node moved or deleted changes this
/// answer in the frame it happens.
///
/// **Across the strip it is a whole viewport, not the content's own top and bottom.** That
/// band is where `clamp_to_strip` keeps every node, so normally it *is* the strip and the
/// clamp pins the view to it — a strip has no up and down and a hand should not be able to
/// push one off the top. A node taller than the viewport is the one thing that overflows
/// it, and then this grows and there is somewhere real to scroll to.
///
/// Which is why nothing reads it directly. What the view is clamped to and what the minimap
/// draws is `CanvasState::bounds`, which eases toward this — that ease is what makes a strip
/// that trims itself bearable, and it is also the only room a drag at the edge is given.
pub fn strip_bounds(layouts: &canvas::Layouts, rect: Rect) -> Option<Rect> {
    content_bounds(layouts).map(|b| {
        Rect::from_min_max(
            Pos2::new(b.min.x, b.min.y.min(0.0)),
            Pos2::new(
                b.max.x.max(b.min.x + rect.width()),
                b.max.y.max(rect.height()),
            ),
        )
    })
}

/// How far the view moves this frame with the pointer held near an edge of the canvas.
///
/// The rate is a function of how deep into the margin the pointer is and of nothing else —
/// not of the pan, the bounds or the extent, all of which the creep itself changes. A rate
/// read from anything it moves accelerates into itself; this one holds the same speed for as
/// long as the hand holds the same place.
///
/// Positive on an axis means the view moves that way to show what is behind the near edge.
fn edge_creep(pointer: Pos2, rect: Rect, dt: f32) -> eframe::egui::Vec2 {
    let axis = |p: f32, min: f32, max: f32| {
        let near = ((min + EDGE_SCROLL_MARGIN - p) / EDGE_SCROLL_MARGIN).clamp(0.0, 1.0);
        let far = ((p - (max - EDGE_SCROLL_MARGIN)) / EDGE_SCROLL_MARGIN).clamp(0.0, 1.0);
        (near - far) * EDGE_SCROLL_RATE * dt
    };
    eframe::egui::Vec2::new(
        axis(pointer.x, rect.min.x, rect.max.x),
        axis(pointer.y, rect.min.y, rect.max.y),
    )
}

/// Keep the view over the content. Where the content is smaller than the viewport on an
/// axis, it is pinned to that edge rather than floating.
///
/// `extent` is how far right the strip has been extended to, which is the one thing here that
/// is not the content: an extended strip has room past its last node, which is what makes
/// room to drag a node into. `frozen` is the strip a drag in flight is holding still.
fn clamp_pan(t: &mut Transform, rect: Rect, bounds: Option<Rect>) {
    let Some(bounds) = bounds else {
        t.pan = eframe::egui::Vec2::ZERO;
        return;
    };
    // Panning by `p` puts world `w` at screen `w + p`, so the near end is reachable while
    // `p <= -lo` and the far end while `p >= size - hi`. A strip narrower than the viewport
    // makes that range empty, and the near end wins.
    let (min, max) = (rect.width() - bounds.max.x, -bounds.min.x);
    t.pan.x = if min > max {
        max
    } else {
        t.pan.x.clamp(min, max)
    };
    // **Across the strip the view is pinned to the band, because a strip has no up and
    // down.** `clamp_to_strip` holds every node inside the viewport's own height and
    // `strip_bounds` is that band whatever the content does, so normally this range is a
    // single point and the pan sits on it: dragging the background up or down moves nothing,
    // where it used to push the whole strip off the top with nothing to bring it back.
    //
    // The band being the *viewport's* and not the content's is what makes that safe. Clamped
    // to the content, this would recentre the whole strip every time a node grew, collapsed
    // or was dragged — motion nobody asked for, and the reason this arm used to be skipped
    // wherever the content fit. A node taller than the viewport is the one thing that
    // overflows the band, and then the range is real and there is somewhere to scroll to.
    let (ymin, ymax) = (rect.height() - bounds.max.y, -bounds.min.y);
    if ymin <= ymax {
        t.pan.y = t.pan.y.clamp(ymin, ymax);
    }
}

/// The pan that would bring a just-dropped node into view, or `None` if it is already in
/// one.
///
/// **The smallest move that does it**, rather than centring on the node. Centring is what
/// following a tag does, where the person asked to be taken somewhere and the node is the
/// answer to a question. A drop is not a question: the hand knows where it put the thing,
/// and a view that swings to put it in the middle throws away the rest of the graph the
/// person was working against. Nudging the node inside the near edge keeps what was already
/// on screen on screen.
///
/// Inside `EDGE_SCROLL_MARGIN` of the edge, because that band is where a drag *creeps*: a
/// node revealed into it is a node sitting where the view would run away from it the moment
/// it was picked up again.
///
/// Positions come from the drop's own `moves` rather than from the graph, which is still a
/// frame behind — the `MoveNodes` they came from has not been applied yet.
fn reveal_pan(
    moves: &[(NodeId, Pos2)],
    layouts: &canvas::Layouts,
    transform: &Transform,
    origin: Pos2,
    rect: Rect,
) -> Option<eframe::egui::Vec2> {
    let mut shown: Option<Rect> = None;
    for (id, to) in moves {
        let Some(laid) = layouts.find(*id) else {
            continue;
        };
        let world = laid.rect.translate(*to - laid.rect.min);
        let r = Rect::from_min_max(
            transform.to_screen(origin, world.min),
            transform.to_screen(origin, world.max),
        );
        shown = Some(shown.map_or(r, |b: Rect| b.union(r)));
    }
    let shown = shown?;
    let room = rect.shrink(EDGE_SCROLL_MARGIN);
    // One axis at a time, and the near edge wins where the node is wider than the room:
    // showing the start of something too big to show is better than showing its middle.
    let axis = |lo: f32, hi: f32, min: f32, max: f32| {
        if lo < min {
            min - lo
        } else if hi > max {
            (max - hi).max(min - lo)
        } else {
            0.0
        }
    };
    let by = eframe::egui::Vec2::new(
        axis(shown.min.x, shown.max.x, room.min.x, room.max.x),
        axis(shown.min.y, shown.max.y, room.min.y, room.max.y),
    );
    (by != eframe::egui::Vec2::ZERO).then(|| transform.pan + by)
}

/// Clamp a node's position to the strip: anywhere along it, within the viewport across it.
///
/// A node taller than the viewport is pinned to the top margin rather than pushed off it.
pub fn clamp_to_strip(graph: &Graph, id: NodeId, measured: &[f32], to: Pos2, height: f32) -> Pos2 {
    let laid = canvas::Layouts::one(graph, id, measured);
    clamp_into(laid.find(id).map_or(0.0, |l| l.height), to, height)
}

/// [`clamp_to_strip`] for a node whose height is already laid out.
fn clamp_into(node_height: f32, to: Pos2, height: f32) -> Pos2 {
    let lowest = height - node_height - STRIP_MARGIN;
    Pos2::new(to.x, to.y.clamp(STRIP_MARGIN, lowest.max(STRIP_MARGIN)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeId;

    /// A point is on a port exactly where the port's own widget answers it — its square, with
    /// the painted dot inside it — and where two squares hold it, the nearer port has it, the
    /// first of equals where a collapsed node's ports share one point.
    #[test]
    fn a_point_is_on_the_port_whose_square_holds_it() {
        let slot = |key: &'static str, y: f32| PortSlot {
            port: PortRef::new(NodeId(1), key),
            ty: PortType::VaryingNumber,
            is_input: true,
            center: Pos2::new(100.0, y),
        };
        for zoom in [0.5_f32, 1.0, 2.5] {
            let t = Transform {
                zoom,
                ..Transform::default()
            };
            // Two rows a pitch apart, and two ports gathered on one point of a header.
            let pitch = canvas::PORT_PITCH * zoom;
            let slots = [
                slot("a", 100.0),
                slot("b", 100.0 + pitch),
                slot("c", 400.0),
                slot("d", 400.0),
            ];
            let at =
                |p: Pos2| node_widget::port_at(&slots, &[], p, &t, |_| true).map(|s| s.port.key);
            let half = canvas::PORT_RADIUS * canvas::PORT_HIT_SCALE * zoom * 0.5;
            // The whole painted dot is on its port.
            let r = canvas::PORT_RADIUS * zoom;
            for angle in 0..16 {
                let a = angle as f32 * std::f32::consts::TAU / 16.0;
                let edge = Pos2::new(100.0, 100.0) + vec2(a.cos(), a.sin()) * r * 0.99;
                assert_eq!(at(edge), Some("a"), "zoom {zoom}: the dot's edge at {a}");
            }
            // Inside the square's corner, and just past its edge.
            assert_eq!(
                at(Pos2::new(100.0 + half * 0.9, 100.0 - half * 0.9)),
                Some("a")
            );
            assert_eq!(
                at(Pos2::new(100.0 + half * 1.1, 100.0)),
                None,
                "zoom {zoom}"
            );
            // Between the two rows it is the nearer one's, never the other's.
            assert_eq!(at(Pos2::new(100.0, 100.0 + pitch * 0.4)), Some("a"));
            assert_eq!(at(Pos2::new(100.0, 100.0 + pitch * 0.6)), Some("b"));
            // Gathered: the first.
            assert_eq!(at(Pos2::new(100.0, 400.0)), Some("c"));
            // And a port the caller will not take gives way to one under the point it will.
            assert_eq!(
                node_widget::port_at(&slots, &[], Pos2::new(100.0, 400.0), &t, |s| s.port.key
                    == "d")
                .map(|s| s.port.key),
                Some("d")
            );
        }
    }

    /// A port under a body painted after its own node's is under that body, not under the
    /// pointer; one painted after the body over it is on top, and so is its own node's body.
    #[test]
    fn a_port_under_a_body_painted_later_is_not_under_the_pointer() {
        let t = Transform::default();
        let port = PortSlot {
            port: PortRef::new(NodeId(2), "input"),
            ty: PortType::VaryingColor,
            is_input: true,
            center: Pos2::new(100.0, 100.0),
        };
        let own = (
            NodeId(2),
            Rect::from_min_max(Pos2::new(100.0, 50.0), Pos2::new(300.0, 200.0)),
        );
        let over = (
            NodeId(3),
            Rect::from_min_max(Pos2::new(50.0, 80.0), Pos2::new(250.0, 120.0)),
        );
        let under = (
            NodeId(1),
            Rect::from_min_max(Pos2::new(50.0, 80.0), Pos2::new(250.0, 120.0)),
        );
        let p = Pos2::new(101.0, 100.0);
        let key = |bodies: &[(NodeId, Rect)]| {
            node_widget::port_at(&[port], bodies, p, &t, |_| true).map(|s| s.port.key)
        };
        assert_eq!(key(&[own]), Some("input"), "its own body");
        assert_eq!(
            key(&[under, own]),
            Some("input"),
            "a body painted before it"
        );
        assert_eq!(key(&[own, over]), None, "a body painted over it");
        assert_eq!(
            key(&[under, own, over]),
            None,
            "a body painted over it, whatever is under"
        );
    }

    /// The throb is brightest on the frame it first appears, decays, and takes itself off the
    /// canvas once it is spent.
    ///
    /// The clock is egui's, so the start is stamped on the first frame that draws rather than
    /// when the link is clicked — which means a link followed while the window is not
    /// painting still throbs for its full span once it is.
    #[test]
    fn a_throb_starts_bright_decays_and_clears_itself() {
        let node = NodeId(7);
        let mut state = CanvasState::default();
        assert_eq!(
            state.throb_at(100.0),
            None,
            "nothing asked for, nothing drawn"
        );

        state.throb_on(node);
        // Whatever the clock reads when the first frame lands is the start: the brightest
        // point, because the eye has just been moved somewhere else and has to be caught.
        let (first, bright) = state.throb_at(500.0).expect("asked for");
        assert_eq!(first, node);
        assert!(bright > 0.99, "brightest on arrival: {bright}");

        // Partway through it is dimmer than it began, and still the same node.
        let (_, later) = state
            .throb_at(500.0 + THROB_SECONDS * 0.5)
            .expect("running");
        assert!(later < bright, "{later} is not below {bright}");

        // Spent, and gone from the state rather than merely drawing nothing: a throb nobody
        // can see should not keep asking the canvas for frames.
        assert_eq!(state.throb_at(500.0 + THROB_SECONDS), None);
        assert!(state.throb.is_none(), "it clears itself");
    }

    /// The golden-angle walk keeps consecutive cables far apart on the hue circle, which is
    /// the whole reason for choosing that angle over any other.
    ///
    /// The weak claim — that no two of the first many land on the same hue — is true of
    /// plenty of angles. The one worth asserting is that **neighbours are never close**: a
    /// cable and the next one out of the same port are the two anyone has to tell apart.
    #[test]
    fn the_golden_angle_keeps_one_cable_clear_of_the_next() {
        let hue = |step: u32| (step as f32 * GOLDEN_ANGLE) % 360.0;
        let apart = |a: f32, b: f32| {
            let d = (a - b).abs() % 360.0;
            d.min(360.0 - d)
        };
        for step in 0..64 {
            let gap = apart(hue(step), hue(step + 1));
            assert!(
                gap > 100.0,
                "steps {step} and {} are {gap} degrees apart",
                step + 1
            );
        }
        // And nothing in a long run doubles back onto an earlier hue.
        for a in 0..64u32 {
            for b in (a + 1)..64 {
                assert!(
                    apart(hue(a), hue(b)) > 1.0,
                    "steps {a} and {b} landed on the same hue"
                );
            }
        }
    }

    /// Two cables close together on the circle still differ in saturation and lightness, so
    /// the walk has more than one axis to separate them on.
    #[test]
    fn the_hue_walk_varies_more_than_the_hue() {
        let colors: Vec<_> = (0..9).map(phi_cable).collect();
        for (a, first) in colors.iter().enumerate() {
            for (b, second) in colors.iter().enumerate().skip(a + 1) {
                assert_ne!(first, second, "steps {a} and {b} are the same color");
            }
        }
    }

    /// The creep at an edge is a function of the pointer and the frame, and of nothing the
    /// creep itself moves. Two frames of a hand holding still are two equal steps rather
    /// than a step that grows, which is what a rate read from the pan or the bounds would
    /// give.
    #[test]
    fn the_creep_at_an_edge_is_read_from_the_pointer_and_from_nothing_it_moves() {
        let rect = Rect::from_min_size(Pos2::new(40.0, 20.0), vec2(800.0, 600.0));
        let dt = 1.0 / 60.0;
        let full = EDGE_SCROLL_RATE * dt;
        let mid = |x: f32| Pos2::new(x, rect.center().y);

        assert_eq!(
            edge_creep(rect.center(), rect, dt),
            eframe::egui::Vec2::ZERO,
            "a pointer nowhere near an edge moved the view"
        );

        // Positive at the near edge: the view goes that way to show what is behind it.
        let near = edge_creep(mid(rect.min.x), rect, dt);
        assert!(
            (near.x - full).abs() < 1e-3,
            "{near:?} is not the full rate"
        );
        assert_eq!(near.y, 0.0, "one axis at a time on a pointer at one edge");
        assert_eq!(
            edge_creep(mid(rect.max.x), rect, dt).x,
            -near.x,
            "the far edge is the same speed the other way"
        );

        // Falling off to nothing at the inner lip, so how fast it goes is the hand's.
        let lip = edge_creep(mid(rect.min.x + EDGE_SCROLL_MARGIN * 0.5), rect, dt);
        assert!(
            lip.x > 0.0 && lip.x < near.x,
            "{lip:?} is not between still and full"
        );
        assert_eq!(
            edge_creep(mid(rect.min.x + EDGE_SCROLL_MARGIN), rect, dt).x,
            0.0,
            "the margin has an inside"
        );

        // A pointer dragged clean out of the window is held at the edge's own rate rather
        // than running away with however far outside it went.
        assert_eq!(edge_creep(mid(rect.min.x - 900.0), rect, dt).x, near.x);

        // And the same hand in the same place is the same step, every frame of a hold.
        let held = mid(rect.min.x + 6.0);
        assert_eq!(edge_creep(held, rect, dt), edge_creep(held, rect, dt));
    }
}
