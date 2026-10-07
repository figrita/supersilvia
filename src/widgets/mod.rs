// SPDX-License-Identifier: AGPL-3.0-or-later

//! What a node draws below its rows, and the contract every one of them keeps.
//!
//! silvia's `snode.js` hands a node one `customArea` div and calls the node's own `onCreate`;
//! the node imports its widgets and fills it, and the base class has never heard of a scope,
//! a preview or an output canvas. This is that hook, with the one function a browser gave
//! silvia for free added: a region reports its size without drawing, because every node on
//! the workspace is laid out at the top of a frame, long before anything is painted.
//!
//! A node declares `NodeDef::regions`, a list of [`Region`] names in draw order, which layout
//! reads through the `Node`'s own `def` without a search. The name is the registry's and the
//! drawing is here: [`def`] is the one table from a name to the widget that sizes and draws
//! it, so `nodes/` says what it wants below its rows without naming the toolkit. Nothing
//! above a region knows which kind it is: `ui/` walks the list, draws each band through
//! [`show`], and turns what comes back into commands.
//!
//! See docs/ui.md and proposals/node-body-controls.md.

pub mod buttons;
pub mod curve;
pub mod gear;
pub mod paint;
pub mod palette;
pub mod picture;
pub mod ranges;
pub mod scope;
pub mod status;
pub mod steps;
pub mod trace;
pub mod viewfinder;
pub mod xypad;

use crate::graph::{ControlValue, Node, NodeId};
use crate::nodes::{Heading, Region};
use crate::ui::canvas::{self, Transform};
use crate::ui::theme::Theme;
use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Ui, pos2, vec2};

/// How one kind of region is sized and drawn: the widget behind a [`Region`] name.
///
/// Every field is declaration, read from a `&'static` here. The two functions are the split
/// silvia does not need: `size` answers for layout without a `Ui`, `show` draws into the rect
/// layout gave it. What the region is called, the heading it wears and the picture it shows
/// are the name's, in `nodes::area`, because the registry and the file have to agree on them.
#[derive(Debug, Clone, Copy)]
pub struct RegionDef {
    /// How tall the region's own content is on a node of this shape, in world units, with its
    /// heading's height excluded — the heading is the frame's and is added by
    /// `canvas::region_height`. Pure and cheap: called for every node on the workspace once a
    /// frame, and by anything laying one node out. Never called on a closed region.
    pub size: fn(&Node) -> f32,
    /// Draw into the rect, and say what the hand did in it.
    pub show: fn(&mut RegionUi<'_>) -> Vec<RegionEvent>,
    /// Whether the pointer inside this region's rect is the region's rather than the node's.
    ///
    /// The node's own hit rect is not one size: a region that does not claim the pointer lets
    /// a drag through to the body, so a hand can still carry the node by its scope or its
    /// trace, and a region that claims it — a pad — owns every press inside its rect and
    /// never starts a node drag. The declaration is the region's, not a rule in `canvas`.
    /// A claiming region is handed the `Response` in [`RegionUi::grab`].
    pub claims_pointer: bool,
    /// The narrowest body this region can be drawn in, where it needs more than
    /// `canvas::NODE_WIDTH`. The widest region wins, and the rows have their own answer in
    /// `NodeDef::width` — see [`canvas::node_width`].
    pub width: Option<f32>,
}

impl RegionDef {
    pub const EMPTY: Self = Self {
        size: |_| 0.0,
        show: |_| Vec::new(),
        claims_pointer: false,
        width: None,
    };
}

/// The widget that sizes and draws a region the registry names.
///
/// The whole of the table from `nodes/`'s names to `ui/`'s drawing: a region added to the
/// library is a variant in `nodes::area` and an arm here, and the compiler asks for both.
pub fn def(region: Region) -> &'static RegionDef {
    match region {
        Region::Render => &picture::RENDER,
        Region::Preview(_) => &picture::PREVIEW,
        Region::Scope => &scope::SCOPE,
        Region::Meters => &scope::METERS,
        Region::Trace => &trace::TRACE,
        Region::Caption => &trace::CAPTION,
        Region::Status => &status::STATUS,
        Region::Curve => &curve::CURVE,
        Region::Palette => &palette::PALETTE,
        Region::Steps => &steps::STEPS,
        Region::Grid => &steps::GRID,
        Region::Ranges => &ranges::RANGES,
        Region::Viewfinder => &viewfinder::VIEWFINDER,
        Region::Buttons(_) => &buttons::BUTTONS,
        Region::XyPad => &xypad::XYPAD,
        Region::Paint(_) => &paint::PAINT,
        Region::Brush => &paint::BRUSH,
        Region::Gear => &gear::GEAR,
        Region::Teeth => &gear::TEETH,
    }
}

/// What a region says the hand did in it.
pub enum RegionEvent {
    /// Controls the hand moved, as one edit: the scope's two-axis band handles.
    Controls(Vec<(&'static str, ControlValue)>),
    /// An option the region set — a heading opened or closed.
    Option {
        key: &'static str,
        value: &'static str,
    },
    /// Some of the node's own options and controls, written by one of its buttons as one
    /// edit: a `SetSettings`.
    Settings(crate::nodes::Settings),
    /// A key part of this node is held down this frame, as a finger on an action input's
    /// button is: a level the node's `tick` reads, never an edit. A press button reads it
    /// through `TickContext::downs`; `xypad`'s puck, which does not fly while a hand has it,
    /// reads it through `TickContext::pressed`.
    Held(&'static str),
    /// A picture whose slot this region reserved, for the caller to blit into.
    Picture(crate::ui::Thumbnail),
    /// What an s-number inside a region asked for, named by the control it stands over.
    ///
    /// The same [`crate::ui::number::NumberAction`] a port row's control returns, carried out
    /// of the region rather than turned into a `Controls` edit here: a reset puts the range
    /// back too, a right-click opens the range editor at `at`, and `Alt` + click learns a
    /// MIDI binding — three answers that are not a new value and cannot be spelled as one.
    /// `at` is where the cell is on screen, which is where its editor hangs.
    Number {
        key: &'static str,
        action: crate::ui::number::NumberAction,
        at: Pos2,
    },
    /// A swatch inside a region was clicked: open the color picker on the control it stands
    /// over, hanging from `at`, as a row's swatch does.
    Color { key: &'static str, at: Pos2 },
    /// One of the node's own values the hand rewrote, as one edit: a cell of a grid turned
    /// over, the whole grid cleared, or the picture a paint surface painted on.
    Value {
        key: &'static str,
        value: crate::graph::Value,
    },
    /// What a hand did on the node's own surface that is runtime rather than document — a
    /// well dropped on a pad, a preset's wells. See [`crate::nodes::cpu::Touch`].
    Touch(crate::nodes::cpu::Touch),
}

/// What the node published this frame, for the regions that draw it.
///
/// Gathered by `App` per node and handed down: a region never reaches for it.
#[derive(Default, Clone, Copy)]
pub struct Live<'a> {
    /// An audio source's bands and spectrum.
    pub scope: Option<&'a crate::audio::Scope>,
    /// The ring a `CpuNode::trace` pushes into.
    pub trace: Option<&'a crate::nodes::cpu::TraceRing>,
    /// What `CpuNode::caption` said about itself this frame, as `(key, value)` cells.
    pub caption: Option<&'a [(&'static str, String)]>,
    /// The curve a transport is recording or playing this frame, for the region that draws
    /// one. The node's own saved value is what it falls back to.
    pub curve: Option<&'a crate::nodes::cpu::Curve>,
    /// What `CpuNode::status` said this frame, for the node that draws a status line.
    pub status: Option<&'a str>,
    /// Where this node is in whatever it is playing — `CpuNode::playhead`, which is a clip's
    /// 0 to 1 for the strip under a picture and a sequencer's own step for the grid that
    /// lights it. `None` for a node that is playing nothing, and for a step grid that means
    /// nothing is lit.
    pub playhead: Option<f32>,
    /// Where a pad's puck is and what pulls on it — `CpuNode::puck`. `None` before the node
    /// has ticked, and for every node without a pad.
    pub puck: Option<&'a crate::nodes::cpu::Puck>,
    /// What a gear is doing — `CpuNode::gear`. `None` before the node has ticked, and for
    /// everything but the two gears.
    pub gear: Option<&'a crate::nodes::gear::Reading>,
    /// What a loop of a Master Gear needs, read from the chains below it, for the line under
    /// its picture.
    pub gear_caption: Option<&'a str>,
    /// Every control whose fader soft takeover holds out of pick-up, and where the fader is,
    /// so a region's own number wears the ghost mark a row's does.
    pub ghosts: &'a [(crate::midi::Target, f32)],
}

/// Where the fader bound to `port` is, among `ghosts`, while it is out of pick-up.
pub fn ghost_of(ghosts: &[(crate::midi::Target, f32)], port: crate::graph::PortRef) -> Option<f32> {
    ghosts
        .iter()
        .find(|(t, _)| *t == crate::midi::Target::Port(port))
        .map(|(_, at)| *at)
}

/// One region's drawing context: where it is, how big, and what the node published.
pub struct RegionUi<'a> {
    pub ui: &'a mut Ui,
    /// Which region this is, so one that shows a picture can read back which one rather than
    /// being written twice.
    pub region: Region,
    pub node: &'a Node,
    pub id: NodeId,
    /// The region's own content, below its heading — the band layout reserved for it.
    pub rect: Rect,
    /// The canvas zoom, for anything sized in points rather than in world units.
    pub zoom: f32,
    /// The body's corner radius where this region takes the foot of the body, zero above
    /// another region: what rounds a picture's bottom corners with the node's own.
    pub corner: f32,
    pub theme: &'a Theme,
    pub live: Live<'a>,
    /// The pointer inside this region's rect, where the region claimed it. `None` for a
    /// region that let the drag through to the node's body.
    pub grab: Option<Response>,
    /// The `lock_cursor_while_scrubbing` preference, for a region drawing an s-number: the
    /// control grabs the pointer while it is dragged, and a region cannot read a preference
    /// of its own.
    pub lock_cursor: bool,
    /// The key of this node's own number waiting for a MIDI message, if one is: that
    /// s-number wears the learning ring.
    pub learning: Option<&'static str>,
    /// The project's MIDI map, so a bound number wears the dot a bound row does.
    pub bindings: &'a crate::midi::Bindings,
}

impl RegionUi<'_> {
    /// A name for a widget inside this region, unique on screen: the node's own name and the
    /// region's key, the way every other control on a node is named.
    pub fn name(&self, part: &str) -> String {
        format!("{}{}.{part}", self.node.def.slug, self.id)
    }

    /// One of this node's own numbers, drawn as the inset s-number the library already draws
    /// on a port row.
    ///
    /// **The control is the library's, the layout is the region's.** A value a node keeps for
    /// itself — a hidden control, with no port and no cable — is still a number a hand dials,
    /// so it wears the same chrome, answers the same gestures and carries the same
    /// accessible name `{slug}{id}.{key}` it would have had on a row. What the region decides
    /// is where it sits: a grid three across under R, G and B, rather than one row each.
    ///
    /// `None` where this node has no such control, and where the hand did nothing this frame.
    pub fn number(&mut self, rect: Rect, key: &'static str) -> Option<RegionEvent> {
        let def = self.node.def;
        let input = def.input(key)?;
        let crate::nodes::Control::Number {
            default, unit, log, ..
        } = input.control
        else {
            return None;
        };
        let &ControlValue::Float(value) = self.node.controls.get(key)? else {
            return None;
        };
        let declared = crate::nodes::default_range(self.node, key)?;
        let range = crate::nodes::control_range(def, self.node, key)?;
        let spec = crate::ui::number::NumberSpec {
            value,
            default,
            range,
            declared,
            unit,
            log,
            // A value with no port has nothing arriving at it: there is one number and the
            // control is it.
            varying: false,
            picture: None,
            learning: self.learning == Some(key),
            ghost: ghost_of(self.live.ghosts, crate::graph::PortRef::new(self.id, key)),
        };
        let name = self.name(key);
        let action = crate::ui::number::scrub(
            self.ui,
            rect,
            &name,
            &spec,
            self.theme,
            true,
            self.lock_cursor,
            self.zoom,
        );
        // After the control, so the dot is painted over a neighbour's bevel rather than
        // under it where a grid's gutter is narrow.
        if let Some(trigger) = self
            .bindings
            .trigger_of(crate::graph::PortRef::new(self.id, key))
        {
            crate::ui::midi_mark(
                self.ui,
                crate::ui::MarkAt::Corner(rect),
                self.zoom,
                self.theme,
                &format!("{}{} {key}", def.slug, self.id),
                &trigger.label(),
                crate::ui::node_widget::UNBIND_ON_NODE,
            );
        }
        Some(RegionEvent::Number {
            key,
            action: action?,
            at: rect.left_bottom(),
        })
    }

    /// One of this node's own colors, drawn as the swatch a color control on a row is: the
    /// same chrome, the same accessible name `{slug}{id}.{key}`, and a click that opens the
    /// same picker. `None` where this node has no such color, and where nobody clicked.
    pub fn swatch(&mut self, rect: Rect, key: &'static str) -> Option<RegionEvent> {
        let &ControlValue::Color(value) = self.node.controls.get(key)? else {
            return None;
        };
        let name = self.name(key);
        let response = crate::ui::color::swatch(
            self.ui,
            rect,
            name.as_str(),
            value,
            self.theme,
            true,
            false,
            self.zoom,
        );
        crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
        response.clicked().then(|| RegionEvent::Color {
            key,
            at: rect.left_bottom(),
        })
    }

    /// A tiny caption inside the region — a grid's row and column headings.
    pub fn caption(&self, at: Pos2, align: Align2, text: &str, tint: Color32) {
        self.ui.painter().text(
            at,
            align,
            text,
            FontId::proportional(crate::ui::theme::font_size(
                crate::ui::theme::FONT_TINY,
                self.zoom,
            )),
            tint,
        );
    }
}

/// A button inside a region: silvia's own `.euc-controls button`, in the editor's tokens.
///
/// The affordance an action input already has through `ui::press`, for the thing that is not
/// an action input — a Clear that zeroes four lanes is one edit a hand asks for, not a gate
/// something downstream can hold. A click, not a level, for the same reason.
pub fn button(r: &mut RegionUi<'_>, rect: Rect, caption: &str, part: &str) -> bool {
    let name = r.name(part);
    let w = r.ui.interact(
        rect,
        r.ui.id().with(("region-button", &name)),
        Sense::click(),
    );
    crate::ui::cursor(&w, eframe::egui::CursorIcon::PointingHand);
    let radius = eframe::egui::CornerRadius::same(crate::ui::theme::RADIUS_SM);
    let painter = r.ui.painter();
    painter.rect(
        rect,
        radius,
        if w.hovered() {
            r.theme.bg_hover()
        } else {
            r.theme.bg_interactive()
        },
        eframe::egui::Stroke::new((1.0 * r.zoom).max(1.0), r.theme.border_normal()),
        eframe::egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(crate::ui::theme::font_size(
            crate::ui::theme::FONT_TINY,
            r.zoom,
        )),
        r.theme.text_primary(),
    );
    crate::ui::accessible(&w, eframe::egui::WidgetType::Button, &name);
    w.clicked()
}

/// The heading's own geometry, silvia's `.section-toggle` in world units: a 20 point header
/// (`canvas::HEADING_HEIGHT`) whose bar pulls in by `HEADING_PAD` at the top, a disclosure
/// triangle in a fixed box so the label cannot shift as it turns, and a gap a little under
/// silvia's `gap: 6px` between the two.
const TRIANGLE: f32 = 9.0;
const TRIANGLE_GAP: f32 = 4.0;
/// How far the triangle's corners are rounded, in world units.
const TRIANGLE_ROUND: f32 = 0.9;
/// Air above the bar: `canvas::HEADING_GAP`, the one gap a heading keeps. What goes under a
/// closed bar is the layout's (`canvas::rows_with_top`), so the bar is the same height open or
/// closed.
const HEADING_PAD: f32 = canvas::HEADING_GAP;
/// How long the triangle takes to turn, and how long the bar takes to brighten under the
/// pointer — silvia's `transition: transform 0.15s ease` on the same arrow.
const TURN_TIME: f32 = 0.15;
const HOVER_TIME: f32 = 0.10;

/// Draw one region of a node: its heading where it declares one, then the region itself in
/// what is left of the band.
///
/// `band` is the whole region on screen, heading included — `canvas::NodeLayout::region`, which
/// is what layout reserved. `def` is the widget that draws `region`, which is [`def`]'s answer
/// everywhere but a test standing a probe in for one. Returns what the hand did, heading and
/// region together.
// One region's worth of context, and every argument is used; a struct would only move the list
// to the single call site.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    region: Region,
    def: &RegionDef,
    node: &Node,
    id: NodeId,
    band: Rect,
    t: &Transform,
    corner: f32,
    theme: &Theme,
    live: Live<'_>,
    lock_cursor: bool,
    learning: Option<&'static str>,
    bindings: &crate::midi::Bindings,
) -> Vec<RegionEvent> {
    let mut out = Vec::new();
    let mut rect = band;
    if let Some(heading) = region.heading() {
        let split = band.min.y + canvas::HEADING_HEIGHT * t.zoom;
        let strip = Rect::from_min_max(band.min, pos2(band.max.x, split));
        rect = Rect::from_min_max(pos2(band.min.x, split), band.max);
        let open = canvas::region_open(node, region);
        // Drawn before the node's border, which covers the bar's ends, so it needs no inset.
        if heading_row(ui, strip, [0.0; 2], heading, open, node, id, t.zoom, theme) {
            out.push(RegionEvent::Option {
                key: heading.key,
                value: if open {
                    crate::nodes::OFF
                } else {
                    crate::nodes::ON
                },
            });
        }
        if !open {
            return out;
        }
    }
    // Registered after the body's own ground, which is what makes the claim work: egui hands
    // a drag to the last widget that asked for it, so a claiming region takes every press
    // inside its rect and the node stays put.
    let grab = def.claims_pointer.then(|| {
        let w = ui.interact(
            rect,
            ui.id().with(("region", node.def.slug, id)),
            Sense::click_and_drag(),
        );
        let name = format!("{}{id}.region", node.def.slug);
        crate::ui::accessible(&w, eframe::egui::WidgetType::Other, &name);
        w
    });
    let mut drawing = RegionUi {
        ui,
        region,
        node,
        id,
        rect,
        zoom: t.zoom,
        corner,
        theme,
        live,
        grab,
        lock_cursor,
        learning,
        bindings,
    };
    out.extend((def.show)(&mut drawing));
    out
}

/// The heading strip: a bar carrying a disclosure triangle and a tiny label, and whether it
/// was clicked.
///
/// Public because a heading is not only a region's: an Output's render folds a run of *rows*,
/// which is not a band and has nowhere to hang one of its own, so `ui/node_widget` draws the
/// same bar over them. One function, so the two cannot drift apart by a point.
///
/// The affordance a tick in a shared row cannot be — **a closed region still says it is
/// there** — which is the convention every properties pane has for the same reason. Its state
/// is still the option the tick was.
///
/// silvia's own is `.mixer-section h4.section-toggle`: a 20 px header with `gap: 6px` between
/// a 12 px arrow and the label, the arrow rotating 90 degrees over `0.15s ease`, sitting on a
/// section whose ground is one inset hairline of `border-subtle`. This is that, in supersilvia's tokens:
/// `bg_secondary` — silvia's own `.mixer-section` ground, a step *above* the body's
/// `bg_sunken` rather than below it — across the body's full width, as wide as the rows or
/// region it folds, under a dimmed `border_subtle` hairline with a brighter one along the
/// bottom edge, so a heading reads as a slab sitting in the body rather than as a hole cut in
/// it. `edge` pulls the bar's left and right ends in, in screen points, for a caller that
/// draws it over the node's border. **No tooltip.** A triangle beside a label is the one control that
/// needs no words, and the tip it used to show landed over the heading below it.
#[allow(clippy::too_many_arguments)]
pub fn heading_row(
    ui: &mut Ui,
    strip: Rect,
    edge: [f32; 2],
    heading: Heading,
    open: bool,
    node: &Node,
    id: NodeId,
    zoom: f32,
    theme: &Theme,
) -> bool {
    let name = format!("{}{id}.{}", node.def.slug, heading.key);
    let w = ui.interact(strip, ui.id().with(("heading", &name)), Sense::click());
    crate::ui::cursor(&w, eframe::egui::CursorIcon::PointingHand);
    // Both animations are the context's, keyed by this heading's own name: the turn survives
    // the frame the option changes on, which is what makes it a turn rather than a swap.
    let turn =
        ui.ctx()
            .animate_bool_with_time(ui.id().with(("heading-open", &name)), open, TURN_TIME);
    let hot = ui.ctx().animate_bool_with_time(
        ui.id().with(("heading-hot", &name)),
        w.hovered(),
        HOVER_TIME,
    );

    // The bar spans the body edge to edge, as the rows and regions it folds do: a bar pulled in
    // from both sides sat narrower than what opened under it. The air above it stays, so two
    // stacked headings still read as two strips, and it reaches down to the strip's foot, so
    // open it touches what it folds and the two read as one.
    let bar = Rect::from_min_max(
        strip.min + vec2(edge[0], HEADING_PAD * zoom),
        strip.max - vec2(edge[1], 0.0),
    );
    let painter = ui.painter();
    painter.rect_filled(
        bar,
        0.0,
        theme
            .bg_secondary()
            .lerp_to_gamma(theme.bg_hover(), hot * 0.7),
    );
    // No sides, since the node's own edge is there: a dim hairline along the top and a brighter
    // one along the bottom, which catches the light the top does not and makes the bar a slab
    // in the body rather than a hole cut in it.
    let hair = (1.0 * zoom).max(1.0);
    painter.line_segment(
        [
            pos2(bar.min.x, bar.min.y + hair * 0.5),
            pos2(bar.max.x, bar.min.y + hair * 0.5),
        ],
        eframe::egui::Stroke::new(
            hair,
            theme
                .border_subtle()
                .gamma_multiply(0.45)
                .lerp_to_gamma(theme.border_normal(), hot * 0.7),
        ),
    );
    painter.line_segment(
        [
            pos2(bar.min.x, bar.max.y - hair * 0.5),
            pos2(bar.max.x, bar.max.y - hair * 0.5),
        ],
        eframe::egui::Stroke::new(
            hair,
            theme
                .border_subtle()
                .lerp_to_gamma(theme.border_normal(), hot),
        ),
    );

    let tint = theme
        .text_secondary()
        .lerp_to_gamma(theme.text_primary(), hot);
    // The triangle's box is fixed and its label's x is a constant off the strip, so neither
    // moves as the triangle turns or as `edge` changes with the selection. The box's left
    // edge lines up with a port row's own label, `node_widget::LABEL_INSET` in from the
    // body's edge.
    let inset = crate::ui::node_widget::LABEL_INSET;
    let center = pos2(
        strip.min.x + (inset + TRIANGLE * 0.5) * zoom,
        bar.center().y,
    );
    triangle(
        painter,
        center,
        TRIANGLE * 0.58 * zoom,
        TRIANGLE_ROUND * zoom,
        turn,
        tint,
    );
    painter.text(
        pos2(
            strip.min.x + (inset + TRIANGLE + TRIANGLE_GAP) * zoom,
            bar.center().y,
        ),
        Align2::LEFT_CENTER,
        heading.label,
        FontId::proportional(crate::ui::theme::font_size(
            crate::ui::theme::FONT_TINY,
            zoom,
        )),
        tint,
    );
    // The same name the tick in the ticks row had, so the thing a hand or an agent asks for
    // is the part of the node it hides rather than the affordance it happens to wear.
    crate::ui::accessible(&w, eframe::egui::WidgetType::Button, &name);
    w.clicked()
}

/// How wide one segment of [`heading_segments`] is, and how far the row stands in from the
/// strip's right end, in world units at zoom 1.
const SEGMENT_WIDTH: f32 = 32.0;
const SEGMENT_INSET: f32 = 4.0;

/// A row of segments at the right end of a heading's bar, one per choice, the chosen one lit:
/// an option drawn on the heading rather than as a row (`OptionDef::on_heading`), such as a
/// time-driven node's Free and Loop. Which segment was clicked, if one was.
///
/// Registered after the heading's own strip, so a click here is the segment's and never the
/// fold's. Drawn by [`segments`], whether the heading is open or closed.
pub fn heading_segments(
    ui: &mut Ui,
    strip: Rect,
    choices: crate::nodes::Choices,
    chosen: &str,
    name: &str,
    zoom: f32,
    theme: &Theme,
) -> Option<usize> {
    let height = SEGMENTS_HEIGHT * zoom;
    let width = SEGMENT_WIDTH * zoom;
    let right = strip.max.x - SEGMENT_INSET * zoom;
    // Centred on the bar, which starts `HEADING_PAD` below the strip's top.
    let middle = (strip.min.y + HEADING_PAD * zoom + strip.max.y) * 0.5;
    let frame = Rect::from_min_max(
        pos2(right - width * choices.len() as f32, middle - height * 0.5),
        pos2(right, middle + height * 0.5),
    );
    segments(ui, frame, choices, chosen, name, zoom, theme)
}

/// The height of a [`segments`] control, in world units at zoom 1: the heading's bar, less
/// its padding.
pub const SEGMENTS_HEIGHT: f32 = canvas::HEADING_HEIGHT - 2.0 * HEADING_PAD - 4.0;

/// A segmented switch filling `frame`, one equal segment per choice, the chosen one lit: the
/// two-choice control the design system draws wherever an option is a switch rather than a
/// menu — Free | Loop on a Timing heading, Forward | Reverse under a Ratio Gear's Teeth.
/// Which segment was clicked, if one was; a click on the lit one is nothing.
///
/// One rounded frame in the press button's chrome, split by a hairline, with each choice's
/// name in the heading's tiny type. Each segment is a radio button named `{name}.{value}`.
pub fn segments(
    ui: &mut Ui,
    frame: Rect,
    choices: crate::nodes::Choices,
    chosen: &str,
    name: &str,
    zoom: f32,
    theme: &Theme,
) -> Option<usize> {
    use eframe::egui::{CornerRadius, Stroke, StrokeKind};
    let width = frame.width() / choices.len().max(1) as f32;
    let height = frame.height();
    let radius = CornerRadius::same(crate::ui::theme::RADIUS_SM);
    let hair = (1.0 * zoom).max(1.0);
    let font = FontId::proportional(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    ui.painter()
        .rect_filled(frame, radius, theme.bg_interactive());
    let mut clicked = None;
    for (i, (value, label)) in choices.iter().enumerate() {
        let rect = Rect::from_min_size(
            pos2(frame.min.x + width * i as f32, frame.min.y),
            vec2(width, height),
        );
        let segment = format!("{name}.{value}");
        let w = ui.interact(rect, ui.id().with(("segment", &segment)), Sense::click());
        crate::ui::cursor(&w, eframe::egui::CursorIcon::PointingHand);
        let lit = *value == chosen;
        let corners = CornerRadius {
            nw: if i == 0 { radius.nw } else { 0 },
            sw: if i == 0 { radius.sw } else { 0 },
            ne: if i + 1 == choices.len() { radius.ne } else { 0 },
            se: if i + 1 == choices.len() { radius.se } else { 0 },
        };
        let painter = ui.painter();
        if lit {
            painter.rect_filled(rect, corners, theme.bg_active());
        } else if w.hovered() {
            painter.rect_filled(rect, corners, theme.bg_hover());
        }
        if i > 0 {
            painter.line_segment(
                [rect.left_top(), rect.left_bottom()],
                Stroke::new(hair, theme.border_normal()),
            );
        }
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            if lit {
                theme.text_primary()
            } else {
                theme.text_secondary()
            },
        );
        crate::ui::accessible(&w, eframe::egui::WidgetType::RadioButton, &segment);
        if w.clicked() && !lit {
            clicked = Some(i);
        }
    }
    ui.painter().rect_stroke(
        frame,
        radius,
        Stroke::new(hair, theme.border_normal()),
        StrokeKind::Inside,
    );
    clicked
}

/// A rounded equilateral triangle, pointing right at `turn` 0 and down at `turn` 1.
///
/// Geometry rather than a glyph, for the reason the `?` and the `✕` are: the font is subset.
/// Equilateral off one circumradius `r`, so the shape is the same weight whichever way it
/// points, and rotated by `turn` of a quarter turn rather than swapped for a second set of
/// points — a disclosure triangle that jumps between two shapes reads as two icons.
///
/// The corners are rounded by walking the outline: each vertex becomes an arc of radius
/// `round`, whose own center sits `2·round` along the corner's bisector and whose tangents
/// meet the two edges `√3·round` from the vertex — the 30 degree half-angle of an equilateral
/// corner. Stroking a triangle instead would not do it: epaint miters a closed path's joins,
/// and a 60 degree miter is a spike.
///
/// `STEPS` segments an arc: four is smooth at the size a heading draws one.
const STEPS: usize = 4;

fn triangle(
    painter: &eframe::egui::Painter,
    center: Pos2,
    r: f32,
    round: f32,
    turn: f32,
    tint: Color32,
) {
    use std::f32::consts::{FRAC_PI_2, TAU};
    let angle = turn * FRAC_PI_2;
    let v: Vec<Pos2> = (0..3)
        .map(|i| {
            let a = angle + i as f32 * TAU / 3.0;
            center + vec2(r * a.cos(), r * a.sin())
        })
        .collect();
    let round = round.min(r * 0.4);
    let mut points = Vec::with_capacity(3 * 5);
    for i in 0..3 {
        let p = v[i];
        let to_prev = (v[(i + 2) % 3] - p).normalized();
        let to_next = (v[(i + 1) % 3] - p).normalized();
        let bisector = (to_prev + to_next).normalized();
        let arc = p + bisector * (2.0 * round);
        let from = p + to_prev * (round * 3.0_f32.sqrt()) - arc;
        let to = p + to_next * (round * 3.0_f32.sqrt()) - arc;
        let (a0, a1) = (from.y.atan2(from.x), to.y.atan2(to.x));
        // The short way round, which is the 120 degrees outside the corner.
        let mut sweep = a1 - a0;
        while sweep > std::f32::consts::PI {
            sweep -= TAU;
        }
        while sweep < -std::f32::consts::PI {
            sweep += TAU;
        }
        points.extend((0..=STEPS).map(|s| {
            let a = a0 + sweep * s as f32 / STEPS as f32;
            arc + vec2(round * a.cos(), round * a.sin())
        }));
    }
    painter.add(eframe::egui::Shape::convex_polygon(
        points,
        tint,
        eframe::egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{ControlValue, Graph};
    use eframe::egui::{Event, Id, Modifiers, PointerButton, vec2};
    use egui_kittest::Harness;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};

    thread_local! {
        /// What the probe region below saw, since a `show` is a plain function and can capture
        /// nothing. Per thread, because the two tests run at once and share their statics.
        static GRABBED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    fn probe(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
        let Some(grab) = r.grab.clone() else {
            return Vec::new();
        };
        if !grab.dragged() {
            return Vec::new();
        }
        GRABBED.with(|g| g.set(true));
        vec![RegionEvent::Controls(vec![(
            "x",
            ControlValue::Float(grab.drag_delta().x),
        )])]
    }

    /// A region that claims the pointer, standing in for the pad: nothing in the registry
    /// claims yet, and the declaration is the thing under test rather than any node's.
    const CLAIMS: RegionDef = RegionDef {
        size: |_| 60.0,
        show: probe,
        claims_pointer: true,
        ..RegionDef::EMPTY
    };

    /// The same region, letting the pointer through — a read-only trace, a thumb.
    const LETS_THROUGH: RegionDef = RegionDef {
        claims_pointer: false,
        ..CLAIMS
    };

    /// Drag inside a region and say who got it: the region's own event, and whether the body
    /// ground under it started a node drag.
    fn drag_in(def: &'static RegionDef) -> (bool, bool) {
        GRABBED.with(|g| g.set(false));
        let mut g = Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "adsr", Pos2::ZERO).expect("in registry");
        let node = g.get(id).expect("just added").clone();
        let theme = Theme::default();
        // Through atomics rather than captured locals: the harness holds the closure for as
        // long as it lives, so a `&mut` into it cannot be read back while it is alive.
        let dragged_node = AtomicBool::new(false);
        let edited = AtomicBool::new(false);
        let band = Rect::from_min_size(pos2(0.0, 100.0), vec2(200.0, 60.0));
        let mut harness = Harness::new_ui(|ui| {
            // The node's body ground, registered first, exactly as `node_widget::body` does:
            // what wins over it is what asked after it.
            let ground = ui.interact(
                Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 160.0)),
                Id::new("ground"),
                Sense::click_and_drag(),
            );
            // A region with no heading, so the probe's band is all its own.
            let out = show(
                ui,
                Region::Caption,
                def,
                &node,
                id,
                band,
                &Transform::default(),
                0.0,
                &theme,
                Live::default(),
                false,
                None,
                &crate::midi::Bindings::default(),
            );
            if ground.dragged() {
                dragged_node.store(true, Ordering::SeqCst);
            }
            if out
                .iter()
                .any(|e| matches!(e, RegionEvent::Controls(values) if !values.is_empty()))
            {
                edited.store(true, Ordering::SeqCst);
            }
        });
        let at = band.center();
        harness.input_mut().events.push(Event::PointerMoved(at));
        harness.step();
        harness.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        harness.step();
        harness
            .input_mut()
            .events
            .push(Event::PointerMoved(at + vec2(24.0, 0.0)));
        harness.step();
        harness.step();
        drop(harness);
        (
            edited.load(Ordering::SeqCst),
            dragged_node.load(Ordering::SeqCst),
        )
    }

    /// A region that claims the pointer owns every press inside its rect: the drag is the
    /// region's, it comes back as the region's own event, and the node stays put.
    #[test]
    fn a_claiming_region_takes_the_drag_and_the_node_stays_put() {
        let (edited, dragged_node) = drag_in(&CLAIMS);
        assert!(GRABBED.with(Cell::get), "the region saw the drag");
        assert!(edited, "and said so, as an event of its own");
        assert!(!dragged_node, "and the body under it never started a drag");
    }

    /// A region that does not claim it lets the drag through to the body, so a hand carries
    /// the node by its trace exactly as by any other part of it. The hit rect is the node's
    /// business, not one size.
    #[test]
    fn a_region_that_does_not_claim_lets_the_drag_reach_the_node() {
        let (edited, dragged_node) = drag_in(&LETS_THROUGH);
        assert!(!edited, "the region was handed nothing");
        assert!(!GRABBED.with(Cell::get));
        assert!(dragged_node, "the body under it took the drag");
    }
}
