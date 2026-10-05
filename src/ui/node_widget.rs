// SPDX-License-Identifier: AGPL-3.0-or-later

//! One node on the canvas.

use crate::graph::{Node, NodeId, PortRef, PortType};

use crate::nodes;
use crate::ui::canvas::{self, Transform};
use crate::ui::context::{Effects, NodeCtx};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, WidgetType,
    epaint::{CircleShape, PathShape, RectShape},
    pos2, vec2,
};

pub use crate::ui::canvas::PortSlot;

/// The square a port answers the pointer in, centered on its dot.
///
/// `port` interacts with exactly this, and [`port_at`] asks the same question for everything
/// else — which cables light up, which port a cable lands on — so a port lights exactly when
/// it would take the click that clears it and the cable that is let go on it.
pub fn port_hit(center: Pos2, t: &Transform) -> Rect {
    let side = canvas::PORT_RADIUS * canvas::PORT_HIT_SCALE * t.zoom;
    Rect::from_center_size(center, vec2(side, side))
}

/// The port under a point: of those whose [`port_hit`] square holds it and `pick` accepts, the
/// nearest, and the first of equals — which is where a collapsed node's ports, gathered on
/// one point of its header, share a square.
///
/// Only a port nothing is painted over: `bodies` is every node's body on screen in the order
/// they are painted, and a port whose node comes before the last body holding the point is
/// under that body.
///
/// The one test of whether a point is on a port, for the hover and the drop alike.
pub fn port_at<'s>(
    slots: &'s [PortSlot],
    bodies: &[(NodeId, Rect)],
    p: Pos2,
    t: &Transform,
    pick: impl Fn(&PortSlot) -> bool,
) -> Option<&'s PortSlot> {
    let over = bodies.iter().rposition(|(_, body)| body.contains(p));
    let seen = |s: &PortSlot| {
        over.is_none_or(|over| bodies[over..].iter().any(|(id, _)| *id == s.port.node))
    };
    slots
        .iter()
        .filter(|s| port_hit(s.center, t).contains(p) && pick(s) && seen(s))
        .min_by(|a, b| (a.center - p).length().total_cmp(&(b.center - p).length()))
}

/// The body's corner radius on screen, in points, at its top: `RADIUS_LG`, the pillowy one
/// nothing else in the editor wears. Rounded to a whole point because `CornerRadius` is drawn
/// in whole points, so anything derived from it has to agree.
pub fn corner_radius(t: &Transform) -> f32 {
    (f32::from(crate::ui::theme::RADIUS_LG) * t.zoom).round()
}

/// The body's corner radius at its **foot**: `ROW_BLOCK_RADIUS`, the radius the input and
/// output slabs round their own outer corners by.
///
/// The bottom of a node is where the slabs, the pictures and the heading bars end, and a
/// 12 point arc under a 6 point one reads as two feet rather than one. The same number for
/// both is what makes the foot one shape — and it is what `canvas::FOOT_PAD` keeps clear
/// under a band that is not flush, and what the renderer rounds an Output picture's own two
/// bottom corners by, which is what `Thumbnail::corner` carries.
pub fn foot_radius(t: &Transform) -> f32 {
    (canvas::ROW_BLOCK_RADIUS * t.zoom).round()
}

/// The body's own `CornerRadius`, grown by `by` points: pillowy at the top, the slabs' radius
/// at the foot. Grown for a ring drawn around the body, which one number would square at the
/// bottom or balloon at the top.
pub fn body_corners(t: &Transform, by: f32) -> CornerRadius {
    let top = (corner_radius(t) + by) as u8;
    let foot = (foot_radius(t) + by) as u8;
    CornerRadius {
        nw: top,
        ne: top,
        sw: foot,
        se: foot,
    }
}

/// The shadow under one node body, painted before the body it belongs to.
///
/// `Visuals::window_shadow` and not a number of its own: it is the shadow the Status box and
/// the Preferences window already cast, so the canvas and the windows over it agree about
/// where the light is, and tuning one tunes both.
///
/// Scaled by the zoom, so the shadow stays fixed to the body rather than growing away from
/// it as the canvas moves. Every node paints its own immediately before its fill, so a node
/// drawn later casts over one drawn earlier and an overlap reads as a stack.
///
/// **Hollow where the body covers it.** The body's fill is opaque, so the part of the shadow
/// under the body's interior is never seen, and it is most of the shadow's area. It is cut
/// out before egui paints it — see [`hollow`] — which paints the same pixels for a fraction
/// of the blending.
pub fn shadow(ui: &Ui, layout: &canvas::NodeLayout<'_>, t: &Transform, origin: Pos2) {
    let base = ui.style().visuals.window_shadow;
    let scaled = |v: f32| (v * t.zoom).round().clamp(0.0, 127.0);
    // Its own clamp, because an offset may be negative and the other three may not.
    let offset = |v: i8| (f32::from(v) * t.zoom).round().clamp(-128.0, 127.0) as i8;
    let shadow = eframe::egui::Shadow {
        offset: [offset(base.offset[0]), offset(base.offset[1])],
        blur: scaled(f32::from(base.blur)) as u8,
        spread: scaled(f32::from(base.spread)) as u8,
        color: base.color,
    };
    let rect = t.to_screen_rect(origin, layout.rect);
    let corners = body_corners(t, 0.0);
    // Clear of the rounded corners and of the body edge's own anti-aliasing, both of which
    // the shadow shows through.
    let covered = rect.shrink(f32::from(corners.nw.max(corners.sw)) + 1.0);
    ui.painter()
        .add(hollow(ui.ctx(), shadow.as_shape(rect, corners), covered));
}

/// A blurred rect, tessellated here as egui would tessellate it, less its solid core inside
/// `covered`.
///
/// egui draws a blur as a feathered fill: a fan over the core, every vertex the shadow's color,
/// and a ring of quads fading from it to transparent. The ring is kept as it is. The core is
/// replaced by the parts of it outside `covered` — a band above, a band below, and a piece
/// either side between them — cut along `covered`'s own edges, so the pieces meet on
/// axis-aligned lines and every pixel outside `covered` is still covered once, in the one
/// color the core has everywhere. Where the mesh is not that shape, or `covered` is empty, the
/// rect is painted as it was.
pub fn hollow(
    ctx: &eframe::egui::Context,
    shape: eframe::egui::epaint::RectShape,
    covered: Rect,
) -> eframe::egui::Shape {
    use eframe::egui::Shape;
    use eframe::egui::epaint::{Mesh, Tessellator, Vertex};
    if !covered.is_positive() {
        return Shape::Rect(shape);
    }
    let mut mesh = Mesh::default();
    let mut tessellator = Tessellator::new(
        ctx.pixels_per_point(),
        ctx.tessellation_options(|o| *o),
        [1, 1],
        Vec::new(),
    );
    tessellator.tessellate_rect(&shape, &mut mesh);
    // `fill_closed_path`'s layout: an inner and an outer vertex per point of the outline, and
    // the core's fan first among the triangles.
    let n = mesh.vertices.len() / 2;
    let fan = 3 * n.saturating_sub(2);
    let inner: Vec<Vertex> = mesh.vertices.iter().step_by(2).copied().collect();
    let core = shape.fill;
    let laid_out = n >= 3
        && mesh.vertices.len() == 2 * n
        && mesh.indices.len() == fan + 6 * n
        && inner.iter().all(|v| v.color == core)
        && mesh.indices[..fan].iter().all(|i| i % 2 == 0);
    if !laid_out {
        return Shape::Rect(shape);
    }
    let outline: Vec<Pos2> = inner.iter().map(|v| v.pos).collect();
    let uv = inner[0].uv;
    let mid = cut(
        &cut(&outline, Cut::Below(covered.min.y)),
        Cut::Above(covered.max.y),
    );
    let pieces = [
        cut(&outline, Cut::Above(covered.min.y)),
        cut(&outline, Cut::Below(covered.max.y)),
        cut(&mid, Cut::LeftOf(covered.min.x)),
        cut(&mid, Cut::RightOf(covered.max.x)),
    ];
    let mut indices: Vec<u32> = mesh.indices[fan..].to_vec();
    for piece in pieces.iter().filter(|p| p.len() >= 3) {
        let first = mesh.vertices.len() as u32;
        mesh.vertices.extend(piece.iter().map(|&pos| Vertex {
            pos,
            uv,
            color: core,
        }));
        for i in 2..piece.len() as u32 {
            indices.extend([first, first + i - 1, first + i]);
        }
    }
    mesh.indices = indices;
    Shape::mesh(mesh)
}

/// Which side of an axis-aligned line a convex polygon is cut to: what is kept.
#[derive(Clone, Copy)]
enum Cut {
    /// Keep what is above the line: `y` at most this, on a screen where `y` grows down.
    Above(f32),
    /// Keep what is below it: `y` at least this.
    Below(f32),
    /// Keep what is left of it: `x` at most this.
    LeftOf(f32),
    /// Keep what is right of it: `x` at least this.
    RightOf(f32),
}

/// A convex polygon cut to one side of an axis-aligned line (Sutherland–Hodgman against one
/// edge). A point made on the line is put exactly on it, so two pieces cut from either side
/// share an edge exactly.
fn cut(polygon: &[Pos2], side: Cut) -> Vec<Pos2> {
    // How far outside the kept side a point is: kept at zero or below.
    let out = |p: Pos2| match side {
        Cut::Above(y) => p.y - y,
        Cut::Below(y) => y - p.y,
        Cut::LeftOf(x) => p.x - x,
        Cut::RightOf(x) => x - p.x,
    };
    let mut kept = Vec::with_capacity(polygon.len() + 2);
    for (i, &a) in polygon.iter().enumerate() {
        let b = polygon[(i + 1) % polygon.len()];
        let (da, db) = (out(a), out(b));
        if da <= 0.0 {
            kept.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            let mut on = a + (b - a) * (da / (da - db));
            match side {
                Cut::Above(y) | Cut::Below(y) => on.y = y,
                Cut::LeftOf(x) | Cut::RightOf(x) => on.x = x,
            }
            kept.push(on);
        }
    }
    kept
}

/// Space between a uniform output's readout and the label left of it.
const READOUT_GAP: f32 = 6.0;
/// How far the swatch a uniform color output draws in its readout slot stops short of its
/// row, top and bottom, at zoom 1. The swatch is otherwise the full height of the row: the
/// color *is* the reading, and a reading you have to squint at is a worse one than a number.
/// The inset is what keeps it a square sitting on the row rather than a band across it,
/// which is what the row block itself already is.
const READOUT_SWATCH_INSET: f32 = 2.0;
/// A port label's inset from the row block's port-side edge, and the clearance an output
/// row's label keeps at the block's far end — silvia's `.node-input` padding, both sides.
pub(crate) const LABEL_INSET: f32 = 8.0 + 4.0;
/// Where a control stops short of the block's far edge, which is also where a row with no
/// control lets its label run to.
const CONTROL_INSET: f32 = 8.0;
/// Clear space between a label and the control it stops short of.
const CONTROL_LABEL_GAP: f32 = 4.0;
/// Clear space between a node's title and the leftmost mark on its header.
const TITLE_CLEARANCE: f32 = 6.0;

/// One monospace advance, which is every advance.
///
/// Everything a node paints is monospace, so `fit` is division rather than a text layout per
/// row per frame — and the `.max` is what keeps a font that has not loaded from dividing by
/// zero.
pub fn advance(ctx: &eframe::egui::Context, font: &FontId) -> f32 {
    ctx.fonts_mut(|f| f.glyph_width(font, '0')).max(0.01)
}

/// How much room a port row's label has, from its own anchor, before it runs into whatever
/// ends the row: the control beside it, the dot at the far end, or (on an output) the
/// readout or declared range already drawn there, whose width is `taken`.
///
/// `block` is the row's own block — inset away from the port side — rather than the body,
/// because that is what the label is painted inside. One function rather than the same
/// arithmetic in the painter and in `no_registry_label_clips_its_declared_width`, which is
/// the only guard the registry's labels have.
fn label_room(block_width: f32, is_output: bool, has_control: bool, taken: f32, zoom: f32) -> f32 {
    let far = if is_output {
        LABEL_INSET * zoom + taken
    } else if has_control {
        (crate::ui::number::WIDTH + CONTROL_INSET + CONTROL_LABEL_GAP) * zoom
    } else {
        CONTROL_INSET * zoom
    };
    (block_width - LABEL_INSET * zoom - far).max(0.0)
}

/// A published uniform number as text, into a buffer the caller reuses: the `f64` as it was
/// published ([`crate::synth::Uniforms::get`]), so a gear's Cycles climb for as long as the
/// show runs and the label beside them gives up its room a character at a time.
///
/// **Two fixed places, not three significant figures.** Everything on the canvas is
/// monospace, so a fixed decimal count changes width only when the integer part gains a
/// digit — once a decade — where significant figures re-flow on every crossing of a power
/// of ten, and a value crossing 1.0 sixty times a second would jitter its own label. Two
/// places is also what the readout on the control beside it shows for the same step, so a
/// number does not gain precision by moving from an input to an output.
///
/// A negative that rounds to zero drops its sign: a value dithering either side of zero
/// would otherwise flicker a character.
///
/// `integral` is the one exception, an output that declared itself a count rather than a
/// measurement: it is written with no decimal places at all, because two zeroes after a
/// channel number say the number is between two others when it never is.
fn readout(buf: &mut String, value: f64, integral: bool) {
    use std::fmt::Write as _;
    buf.clear();
    if !value.is_finite() {
        buf.push_str(if value.is_nan() {
            "nan"
        } else if value.is_sign_positive() {
            "inf"
        } else {
            "-inf"
        });
        return;
    }
    if integral {
        let _ = write!(buf, "{:.0}", value.round());
        if buf == "-0" {
            buf.clear();
            buf.push('0');
        }
        return;
    }
    let _ = write!(buf, "{value:.2}");
    if buf == "-0.00" {
        buf.clear();
        buf.push_str("0.00");
    }
}

/// A node's own name, `{slug}{id}`: what its header is called, and what every other name on
/// it starts with.
///
/// Kept as its two parts and written only when something reads it — the accessibility tree,
/// a tooltip — rather than formatted for every node on every frame to be thrown away. It is
/// also what the node's widgets are keyed by, since the two parts are the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeName {
    pub slug: &'static str,
    pub node: NodeId,
}

impl std::fmt::Display for NodeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.slug, self.node)
    }
}

/// A control's name on a node, `{slug}{id}.{key}`: the accessible name every control on a
/// node carries, kept as its parts for the reason [`NodeName`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlName {
    pub slug: &'static str,
    pub node: NodeId,
    pub key: &'static str,
}

impl std::fmt::Display for ControlName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}.{}", self.slug, self.node, self.key)
    }
}

/// Text written only when something reads it: `Display` over a closure.
pub struct Lazy<F>(pub F);

impl<F: Fn(&mut std::fmt::Formatter<'_>) -> std::fmt::Result> std::fmt::Display for Lazy<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (self.0)(f)
    }
}

/// `Response::on_hover_text`, with the text written only on a frame the tip is shown.
///
/// The same tooltip, down to the width egui holds a text tip to: this is `on_hover_text`'s own
/// body with the string built inside it rather than handed to it.
pub fn hover_with(response: Response, text: impl FnOnce() -> String) -> Response {
    response.on_hover_ui(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.add(eframe::egui::Label::new(text()));
    })
}

/// The marks a header carries at its right-hand end, and where the title has to stop.
struct Marks {
    close: Option<Rect>,
    help: Option<Rect>,
    tooltip: Option<&'static str>,
    /// One square left of the others, for the flag a node at fault wears or the amber
    /// warning a node whose measured taps have multiplied wears. Reserved only when there is
    /// one to draw, so an ordinary node's title keeps the room `help`/`close` already give it
    /// — see docs/decisions.md#a-header-warns-on-what-the-probe-measures-not-on-the-view.
    warn: Option<Rect>,
}

impl Marks {
    fn of(cx: &NodeCtx<'_>, header: Rect, flagged: bool) -> Self {
        let close = close_rect(header, &cx.t);
        let tooltip = Some(cx.node.def.tooltip).filter(|t| !t.is_empty());
        let help = tooltip.and_then(|_| help_rect(header, &cx.t));
        // The leftmost mark the header already carries, which the title, the drag handle and
        // the warning all measure against.
        let leftmost = help.or(close);
        let warn = leftmost
            .filter(|_| flagged)
            .map(|r| r.translate(vec2(-r.width(), 0.0)));
        Self {
            close,
            help,
            tooltip,
            warn,
        }
    }

    /// Where the drag handle stops: short of the close button and the `?`. Taking them out of
    /// the handle's rect rather than letting it run under them is what stops a click on either
    /// also being a zero-distance drag, which would be an undo step for a node that no longer
    /// exists.
    fn handle(&self, header: Rect) -> Rect {
        Rect::from_min_max(
            header.min,
            Pos2::new(
                self.help.or(self.close).map_or(header.max.x, |r| r.min.x),
                header.max.y,
            ),
        )
    }
}

/// Draw the body, its header and its regions, and return the header and the body ground as
/// one response: both are the node's drag handle and click target, so the caller's select,
/// drag and menu rules are written once and read the same wherever the gesture landed. The
/// header's id leads, so the context menu stays keyed to it.
///
/// Every picture a region reserved a slot for goes to `fx.thumbnails` whole — where to blit,
/// which texture, how to fit it — because the body is the one place that has the node's
/// definition, its geometry and its corner radius in hand. The slot inside each is reserved
/// *within* the node's own paint order, between the screen's black ground and the border, so
/// whatever `App` blits into it is painted under the node's chrome rather than over every
/// node on the canvas.
pub fn body(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    // Where a region's own s-number opens its range editor.
    open: &mut Option<crate::ui::OpenControl>,
) -> Response {
    let theme = cx.theme();
    let rect = cx.body();
    let radius = body_corners(&cx.t, 0.0);

    // Body: sunken fill, pillowy 12px corners at the top and the slabs' own 6 at the foot. The
    // border comes last — see below.
    ui.painter().rect_filled(rect, radius, theme.bg_sunken());
    row_bands(ui, cx, rect);

    let header = Rect::from_min_max(
        rect.min,
        Pos2::new(rect.max.x, rect.min.y + canvas::HEADER_HEIGHT * cx.zoom()),
    );
    ui.painter().rect_filled(header, radius, theme.bg_header());
    // Square off the header's bottom corners so it meets the gap below it flush.
    ui.painter().rect_filled(
        Rect::from_min_max(Pos2::new(header.min.x, header.center().y), header.max),
        0.0,
        theme.bg_header(),
    );

    // The close button, the `?` beside it, and where the title has to stop clear of both.
    // Worked out ahead of drawing either, because the title needs it as much as the drag
    // handle built after it does.
    let sampling = cx.frame.sampling.get(&cx.id).copied();
    let fault = cx.frame.faults.get(&cx.id);
    let marks = Marks::of(cx, header, sampling.is_some() || fault.is_some());
    if cx.zoom() > canvas::TITLE_ZOOM {
        title(ui, cx, header, &marks);
    }
    // A fault outranks the warning: what does not work is said before what costs too much,
    // and the flag's hover says both.
    match (marks.warn, fault, sampling) {
        (Some(at), Some(why), _) => fault_flag(ui, cx, at, why, sampling),
        (Some(at), None, Some(taps)) => sampling_warning(ui, cx, at, taps),
        _ => {}
    }
    // Port row labels and uniform number readouts, at the zoom the controls beside them
    // arrive.
    if cx.zoom() >= canvas::DETAIL_ZOOM {
        row_labels(ui, cx);
    }

    // The body's own ground, before the regions: everything a node covers is a
    // select-and-drag handle, so the whitespace around a port's label acts as the header does.
    // Registered before the regions, the header, the close button, the controls and the ports,
    // all of which come later and so win the hit test on a tie — the ground only answers where
    // nothing else does, which is what lets a region claim the pointer by asking for it.
    let name = cx.name();
    let ground = ui.interact(
        rect,
        ui.id().with(("node-ground", name)),
        Sense::click_and_drag(),
    );
    crate::ui::accessible(&ground, WidgetType::Other, format_args!("{name} body"));

    let pictures = fx.thumbnails.len();
    regions(ui, cx, fx, open);

    // The border, last. The row bands and the header are full-width fills painted over the
    // body, so a border drawn before them is covered on three sides and a selected node
    // shows a couple of slivers instead of an outline.
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(
            border_width(cx.selected),
            if cx.selected {
                theme.primary()
            } else {
                theme.border_subtle()
            },
        ),
        eframe::egui::StrokeKind::Inside,
    );

    let response = header_marks(ui, cx, header, &marks, fx);
    picture_marks(ui, cx, fx, pictures);
    players(ui, cx, fx, pictures);
    response.union(ground)
}

/// How wide the node's border is, in screen points, drawn inside the body: two when selected,
/// one otherwise. A heading bar over rows is drawn after the border, so it stops this far short
/// of each side rather than covering it.
pub fn border_width(selected: bool) -> f32 {
    if selected { 2.0 } else { 1.0 }
}

/// Alternating row bands: inputs, then outputs, then options, each its own block rather than
/// one continuous strip — silvia's `.node-inputs`/`.node-outputs`/`.node-options` are separate
/// flex containers, so the alternation restarts at each one, and an input or output block
/// insets away from the port side and rounds its own first and last row on that far corner,
/// reading as a slab short of the edge rather than the full row. A groove divider marks the
/// seam between two non-empty sections — silvia's own `<hr>`, drawn full width regardless of
/// either block's inset, exactly as the CSS's `width: 100%` is.
///
/// One peekable walk of the laid-out rows: `is_last` is the only thing here that needs to see
/// past the current row, and the ordinal the alternation counts is a running total the
/// section boundary resets.
fn row_bands(ui: &Ui, cx: &NodeCtx<'_>, rect: Rect) {
    let theme = cx.theme();
    let world = cx.layout.rect;
    let mut rows = cx.layout.rows.iter().peekable();
    let block_radius = (canvas::ROW_BLOCK_RADIUS * cx.zoom()).round() as u8;
    let mut previous: Option<u8> = None;
    let mut ordinal_in_section = 0usize;
    let is_bar = |row: canvas::Row| {
        matches!(
            row,
            canvas::Row::RenderHeading | canvas::Row::SendHeading | canvas::Row::TimingHeading
        )
    };
    while let Some(&canvas::RowBox { row, top, height }) = rows.next() {
        let section = row.section();
        let opens_section = previous != Some(section);
        // A heading bar paints its own slab on the body's ground, exactly as a region's does,
        // so this row has no band of its own and does not take a turn in the alternation —
        // the rows under it keep the banding they had when the heading was a tick.
        let heading_bar = is_bar(row);
        // A heading parts a block with air, not with corners: the rows either side of a bar,
        // and the Timing heading's own rows, stay square, and only a section's own first and
        // last rows round.
        let is_first = opens_section;
        let is_last = rows.peek().is_none_or(|r| r.row.section() != section);
        if !heading_bar {
            ordinal_in_section = if opens_section {
                0
            } else {
                ordinal_in_section + 1
            };
        }
        let opens_a_seam = opens_section && previous.is_some();
        previous = Some(section);

        let band = cx.screen(canvas::row_block(world, row, top, height));
        let fill = if ordinal_in_section.is_multiple_of(2) {
            theme.bg_secondary()
        } else {
            theme.bg_tertiary()
        };
        // Rounded on the block's own outer corner — away from the port side — where this is
        // its first or last row; square everywhere else, including every option row.
        let radius = match row.port() {
            Some((_, true)) => CornerRadius {
                ne: if is_first { block_radius } else { 0 },
                se: if is_last { block_radius } else { 0 },
                ..Default::default()
            },
            Some((_, false)) => CornerRadius {
                nw: if is_first { block_radius } else { 0 },
                sw: if is_last { block_radius } else { 0 },
                ..Default::default()
            },
            None => CornerRadius::ZERO,
        };
        if !heading_bar {
            ui.painter().rect_filled(band, radius, fill);
        }

        if opens_a_seam {
            // The groove sits in the gap `rows_with_top` opened before this row rather than
            // flush against it — `canvas::groove_top` answers where, beside the constants
            // that gap is made of. Only a section boundary draws one: the header's own gap
            // before the first row carries no groove, since there is nothing above it for a
            // groove to separate.
            let groove_y = world.min.y + canvas::groove_top(top);
            let left = cx.t.to_screen(cx.origin, Pos2::new(world.min.x, groove_y));
            let right = cx.t.to_screen(cx.origin, Pos2::new(world.max.x, groove_y));
            // Clipped to the body: a groove is a bare `hline` rather than a filled shape
            // sharing the body's own rounded corners, so nothing stops it bleeding a sliver
            // past them without an explicit clip.
            let clipped = ui.painter().with_clip_rect(rect);
            groove(&clipped, left.x, right.x, left.y, theme);
        }
    }
}

/// The kind's icon and its label, elided against whatever mark the header carries.
fn title(ui: &Ui, cx: &NodeCtx<'_>, header: Rect, marks: &Marks) {
    let theme = cx.theme();
    let zoom = cx.zoom();
    // Two draws rather than one string: the icon is `theme::ICON_BUMP` larger than the
    // title beside it, and the title starts wherever the icon ended.
    let at = header.left_center() + vec2(canvas::ICON_INSET * zoom, 0.0);
    let drawn = ui.painter().text(
        at,
        Align2::LEFT_CENTER,
        cx.node.def.icon,
        crate::ui::theme::icon_font(crate::ui::theme::FONT_BASE, zoom),
        theme.text_primary(),
    );
    let title_at = Pos2::new(drawn.max.x + 5.0 * zoom, at.y);
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_BASE,
        zoom,
    ));
    // Elided against the `⚠`/`?`/`✕`, rather than left to run under them: a title the
    // width of `Sierpinski Triangle` draws its last letter under the `?` glyph on the
    // fixed 200 pt body otherwise.
    let boundary = marks
        .warn
        .or(marks.help.or(marks.close))
        .map_or(header.max.x, |r| r.min.x)
        - TITLE_CLEARANCE * zoom;
    let shown = fit(
        cx.node.def.label,
        advance(ui.ctx(), &font),
        (boundary - title_at.x).max(0.0),
        Keep::Start,
    );
    ui.painter().text(
        title_at,
        Align2::LEFT_CENTER,
        &shown,
        font,
        theme.text_primary(),
    );
}

/// The amber warning on the header of a node whose measured taps have multiplied, whatever
/// View ▸ Costs is set to: the person who most needs to know a patch has multiplied is the
/// one who has never opened the Costs view. silvia's own sentence, with the probe's measured
/// count in place of its declared one.
fn sampling_warning(ui: &mut Ui, cx: &NodeCtx<'_>, rect: Rect, taps: f64) {
    let theme = cx.theme();
    let name = cx.name();
    let w = ui.interact(rect, ui.id().with(("node-sampling", name)), Sense::hover());
    if w.hovered() {
        ui.painter()
            .circle_filled(rect.center(), rect.width() * 0.5, theme.bg_hover());
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "⚠",
        FontId::monospace(crate::ui::theme::font_size(
            crate::ui::theme::FONT_BASE,
            cx.zoom(),
        )),
        theme.accent(),
    );
    // Built only while the pointer is on it, as `port`'s own hover text is: the sentence
    // is 180 bytes that would otherwise be formatted and thrown away every frame.
    if w.hovered() {
        w.clone().on_hover_text(sampling_sentence(taps));
    }
    crate::ui::accessible(
        &w,
        WidgetType::Other,
        format_args!("{name} sampling {taps:.1}/px"),
    );
}

/// silvia's sentence for a node whose measured taps have multiplied, with the count.
fn sampling_sentence(taps: f64) -> String {
    format!(
        "Samples this input {taps:.1}x per pixel. Connect an Output node before this to \
         rasterize to a framebuffer and avoid multiplying the upstream graph."
    )
}

/// The flag on the header of a node at fault — an Output whose shader failed, a camera that
/// would not open or stopped answering, a microphone, a text that could not be drawn: a
/// disc in the accent with a `!` cut out of it, the palette having no red, and the reason on
/// its hover. Drawn whatever the Status box is doing, since the node is where the eye is.
fn fault_flag(ui: &mut Ui, cx: &NodeCtx<'_>, rect: Rect, why: &str, taps: Option<f64>) {
    let theme = cx.theme();
    let name = cx.name();
    let w = ui.interact(rect, ui.id().with(("node-fault", name)), Sense::hover());
    let painter = ui.painter();
    let radius = rect.width() * 0.3;
    painter.circle_filled(rect.center(), radius, theme.accent());
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "!",
        FontId::monospace(crate::ui::theme::font_size(
            crate::ui::theme::FONT_TINY,
            cx.zoom(),
        )),
        theme.bg_header(),
    );
    if w.hovered() {
        w.clone().on_hover_text(match taps {
            Some(taps) => format!("{why}\n\n{}", sampling_sentence(taps)),
            None => why.to_string(),
        });
    }
    crate::ui::accessible(&w, WidgetType::Other, format_args!("{name} fault {why}"));
}

/// Each port row's label, and what a uniform output published against its own dot.
///
/// The row's own block — inset away from the port side for an input or an output, same as
/// the fill — is what bounds a label's room; the port side itself is never inset, since that
/// is where the port hangs off the node's true edge.
fn row_labels(ui: &mut Ui, cx: &NodeCtx<'_>) {
    let (node, def, theme, zoom) = (cx.node, cx.node.def, cx.theme(), cx.zoom());
    let small = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    // One buffer for every readout this node draws. A readout's text changes on most frames,
    // so it is a fresh galley whatever happens; what it must not also be is a fresh
    // allocation per uniform number output per frame, and a node like `audioin` has six.
    let mut number = String::new();
    let advance = advance(ui.ctx(), &small);
    for (r, block) in cx.layout.blocks() {
        let band = cx.screen(block);
        let Some((index, is_input)) = r.row.port() else {
            continue;
        };
        let (ports, is_output, align, mut at) = if is_input {
            (
                &node.inputs,
                false,
                Align2::LEFT_CENTER,
                band.left_center()
                    + vec2(
                        (LABEL_INSET
                            + if canvas::under_timing(node, r.row) {
                                canvas::TIMING_INDENT
                            } else {
                                0.0
                            })
                            * zoom,
                        0.0,
                    ),
            )
        } else {
            (
                &node.outputs,
                true,
                Align2::RIGHT_CENTER,
                band.right_center() - vec2(LABEL_INSET * zoom, 0.0),
            )
        };
        let Some(port) = ports.get(index) else {
            continue;
        };
        let (in_def, out_def) = if is_output {
            (None, def.output(port.key))
        } else {
            (def.input(port.key), None)
        };

        // An action input with a press button captions the full-row button `controls`
        // draws over this row; the plain label here would only be painted under it.
        if let Some(i) = in_def
            && matches!(i.control, crate::nodes::Control::Press)
        {
            continue;
        }

        // A uniform output says what it published, against its own dot, with the label
        // pushed left of it. A port that has published nothing draws nothing: that is a
        // node which has not run, not a node reading zero. A varying color or varying
        // number output with no live number to print instead prints its declared range
        // there, where one is declared — a static fact standing in for the measurement a
        // uniform number has. One slot, so whichever fills it is painted and measured
        // the same way.
        //
        // A uniform **color** has no number to print, so it fills the same slot with a
        // swatch the height of the row: the color itself is the reading, and a hex string
        // beside a dot would be a worse one.
        let published_color = (is_output && port.ty == PortType::UniformColor)
            .then(|| cx.frame.uniforms.color(PortRef::new(cx.id, port.key)))
            .flatten();
        let mut taken = 0.0;
        if let Some(value) = published_color {
            let side = (band.height() - 2.0 * READOUT_SWATCH_INSET * zoom).max(0.0);
            let rect = Rect::from_min_size(pos2(at.x - side, at.y - side / 2.0), Vec2::splat(side));
            crate::ui::color::swatch(
                ui,
                rect,
                cx.control(port.key),
                value,
                theme,
                true,
                false,
                zoom,
            );
            taken = side + READOUT_GAP * zoom;
            at.x -= taken;
        }
        let thumb = (is_output && port.ty.is_varying())
            .then(|| cx.frame.thumbs.get(&PortRef::new(cx.id, port.key)))
            .flatten();
        if let Some(thumb) = thumb {
            let h = (band.height() - 2.0 * READOUT_SWATCH_INSET * zoom).max(0.0);
            let w = h * crate::compile::THUMB_W as f32 / crate::compile::THUMB_H as f32;
            let rect = Rect::from_min_size(pos2(at.x - w, at.y - h / 2.0), vec2(w, h));
            port_thumb(ui, cx, port.key, thumb, out_def.and_then(|o| o.range), rect);
            taken += w + READOUT_GAP * zoom;
            at.x -= w + READOUT_GAP * zoom;
        }
        let published = (is_output && port.ty == PortType::UniformNumber)
            .then(|| cx.frame.uniforms.get(PortRef::new(cx.id, port.key)))
            .flatten();
        let declared = (is_output && port.ty.is_varying() && thumb.is_none())
            .then(|| out_def.and_then(|o| o.range))
            .flatten();
        if let Some((text, color)) = match (published, declared) {
            (Some(value), _) => {
                readout(&mut number, value, out_def.is_some_and(|o| o.integral));
                Some((number.as_str(), theme.readout()))
            }
            (None, Some(range)) => Some((range, theme.text_muted())),
            (None, None) => None,
        } {
            let drawn = ui.painter().text(at, align, text, small.clone(), color);
            taken = at.x - (drawn.min.x - READOUT_GAP * zoom);
            at.x -= taken;
        }

        // The declared label, falling back to the key when that port is not on the
        // definition, so a port the instance carries and the kind does not still draws
        // something rather than nothing.
        let label = in_def
            .map(|i| i.label)
            .or_else(|| out_def.map(|o| o.label))
            .unwrap_or(port.key);
        let room = label_room(
            band.width(),
            is_output,
            node.controls.contains_key(port.key) || (!is_output && canvas::speed_tall(node, index)),
            taken,
            zoom,
        );
        let shown = fit(label, advance, room, Keep::Start);
        ui.painter()
            .text(at, align, &shown, small.clone(), theme.text_secondary());
    }
}

/// A color thumbnail's cell as it is drawn: over black, as the picture on an Output is. The
/// word is the cell's color packed four bytes, RGBA low byte first; the color is
/// premultiplied, so over black it is the rgb as it stands.
fn thumb_color(word: u32) -> Color32 {
    let [r, g, b, _] = word.to_le_bytes();
    Color32::from_rgb(r, g, b)
}

/// A varying output's thumbnail in its readout slot, and a larger one with its range on
/// hover. A number is shaded between its declared `[lo, hi]` where the output declares one,
/// and otherwise between the lowest and highest it reached on the grid.
fn port_thumb(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    key: &'static str,
    thumb: &std::sync::Arc<crate::synth::PortThumb>,
    declared: Option<&str>,
    rect: Rect,
) {
    use crate::compile::{THUMB_H, THUMB_W};
    use eframe::egui::{Color32, ColorImage, TextureOptions};
    let id = ui.id().with(("port_thumb", cx.id, key));
    let stamp = std::sync::Arc::as_ptr(thumb) as usize;
    let cached: Option<(usize, eframe::egui::TextureHandle, (f32, f32))> =
        ui.ctx().data(|d| d.get_temp(id));
    let (texture, range) = match cached {
        Some((was, texture, range)) if was == stamp => (texture, range),
        cached => {
            let mut range = (0.0, 1.0);
            let pixels: Vec<Color32> = if thumb.number {
                let values: Vec<f32> = thumb.words.iter().map(|w| f32::from_bits(*w)).collect();
                range = declared.and_then(parse_range).unwrap_or_else(|| {
                    values
                        .iter()
                        .filter(|v| v.is_finite())
                        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                            (lo.min(*v), hi.max(*v))
                        })
                });
                let span = range.1 - range.0;
                values
                    .iter()
                    .map(|v| {
                        if !v.is_finite() {
                            return Color32::from_rgb(255, 0, 255);
                        }
                        let t = if span > 1e-9 {
                            (v - range.0) / span
                        } else {
                            0.5
                        };
                        let g = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
                        Color32::from_gray(g)
                    })
                    .collect()
            } else {
                thumb.words.iter().map(|w| thumb_color(*w)).collect()
            };
            // Bottom row first, as an Output's frame is.
            let mut rows = Vec::with_capacity(pixels.len());
            for y in (0..THUMB_H).rev() {
                rows.extend_from_slice(&pixels[y * THUMB_W..(y + 1) * THUMB_W]);
            }
            let image = ColorImage::new([THUMB_W, THUMB_H], rows);
            let texture = match cached {
                Some((_, mut texture, _)) => {
                    texture.set(image, TextureOptions::LINEAR);
                    texture
                }
                None => {
                    ui.ctx()
                        .load_texture(format!("thumb {}", cx.id), image, TextureOptions::LINEAR)
                }
            };
            ui.ctx()
                .data_mut(|d| d.insert_temp(id, (stamp, texture.clone(), range)));
            (texture, range)
        }
    };
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
    let number = thumb.number;
    // By the pointer rather than by a widget's hover, since the row's own widgets sit on top.
    let Some(at) = ui.ctx().pointer_hover_pos().filter(|p| rect.contains(*p)) else {
        return;
    };
    eframe::egui::Area::new(id.with("big"))
        .order(eframe::egui::Order::Tooltip)
        .fixed_pos(at + vec2(16.0, 16.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            eframe::egui::Frame::popup(ui.style()).show(ui, |ui| {
                let size = vec2(THUMB_W as f32, THUMB_H as f32) * 5.0;
                ui.image((texture.id(), size));
                if number {
                    ui.label(format!("{:.3} … {:.3}", range.0, range.1));
                }
            });
        });
}

/// `[lo, hi]` at the start of an output's declared range.
fn parse_range(text: &str) -> Option<(f32, f32)> {
    let inner = text.strip_prefix('[')?.split(']').next()?;
    let (lo, hi) = inner.split_once(',')?;
    Some((lo.trim().parse().ok()?, hi.trim().parse().ok()?))
}

/// The node's own area: every region it declared, in its order, each drawn in the band
/// layout reserved for it. `ui/` does not know which kind any of them is — a picture's black
/// ground and its slot are the region's, and the marks and the player's strip over a picture
/// are every picture's alike.
fn regions(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
) {
    use crate::command::Command;
    if cx.layout.regions.is_empty() {
        return;
    }
    let corner = foot_radius(&cx.t);
    let foot = cx.layout.foot();
    let live = cx.frame.live_of(cx.id);
    let learning = cx
        .frame
        .learning
        .and_then(crate::midi::Target::port)
        .filter(|p| p.node == cx.id)
        .map(|p| p.key);
    for laid in cx.layout.regions {
        let canvas::Region::Declared(i) = laid.region else {
            continue;
        };
        let declared = cx.node.def.regions[i];
        for event in crate::widgets::show(
            ui,
            declared,
            crate::widgets::def(declared),
            cx.node,
            cx.id,
            cx.screen(laid.rect),
            &cx.t,
            if foot == Some(laid.region) {
                corner
            } else {
                0.0
            },
            cx.theme(),
            live,
            cx.frame.prefs.lock_cursor,
            learning,
            cx.frame.bindings,
        ) {
            match event {
                crate::widgets::RegionEvent::Picture(picture) => fx.thumbnails.push(picture),
                crate::widgets::RegionEvent::Controls(values) => {
                    fx.commands.push(Command::SetControls {
                        node: cx.id,
                        values: values.into_iter().collect(),
                    });
                }
                crate::widgets::RegionEvent::Value { key, value } => {
                    fx.commands.push(Command::SetValue {
                        node: cx.id,
                        key,
                        value,
                    });
                }
                crate::widgets::RegionEvent::Option { key, value } => {
                    fx.commands.push(Command::SetOption {
                        node: cx.id,
                        key,
                        value: value.to_string(),
                    });
                }
                crate::widgets::RegionEvent::Settings(settings) => {
                    fx.commands.push(Command::SetSettings {
                        node: cx.id,
                        options: settings.options,
                        controls: settings
                            .controls
                            .into_iter()
                            .map(|(key, v)| (key, crate::graph::ControlValue::Float(v)))
                            .collect(),
                    });
                }
                crate::widgets::RegionEvent::Held(key) => {
                    fx.held.push(crate::graph::PortRef::new(cx.id, key));
                }
                // A number a node keeps for itself answers every gesture a knob on a row
                // does, through the one place that turns those gestures into edits.
                crate::widgets::RegionEvent::Number { key, action, at } => {
                    number_action(cx, key, action, cx.world(at), fx, open);
                }
                crate::widgets::RegionEvent::Touch(touch) => fx.touches.push((cx.id, touch)),
                // A swatch a node keeps for itself opens the picker a row's swatch opens.
                crate::widgets::RegionEvent::Color { key, at } => {
                    toggle(
                        open,
                        crate::ui::OpenControl::Color {
                            node: cx.id,
                            key,
                            at: cx.world(at),
                        },
                    );
                }
            }
        }
    }
}

/// The header as a drag handle, then the `?` and the `✕` over it: registered in that order,
/// after everything else on the body, so each mark wins the click over the handle and the
/// handle wins it over the ground.
fn header_marks(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    header: Rect,
    marks: &Marks,
    fx: &mut Effects,
) -> Response {
    let theme = cx.theme();
    let name = cx.name();
    // The header is the drag handle, and it is a real Response so the accessibility tree
    // and egui_kittest can both see it.
    let response = ui.interact(
        marks.handle(header),
        ui.id().with(("node", name)),
        Sense::click_and_drag(),
    );
    hover_with(response.clone(), || {
        format!("{name} — {}", cx.node.def.label)
    });
    crate::ui::accessible(&response, WidgetType::Button, name);

    if let (Some(help), Some(tooltip)) = (marks.help, marks.tooltip) {
        // Click and drag, where the close button senses only clicks: the body ground under
        // it senses drags, and egui hands a drag to the topmost widget that wants one. A
        // mark whose whole job is to be pointed at must not move the node when it is.
        let q = ui.interact(
            help,
            ui.id().with(("node-help", name)),
            Sense::click_and_drag(),
        );
        // silvia's `circle-help` (Feather's `help-circle`), drawn as the icon's own vector
        // geometry — not a hover-only ground under a font glyph, and not a glyph at all.
        // silvia never gives this mark a hover style, so neither does this: it stays
        // `text_muted` at rest and on hover alike, where the `✕` beside it brightens.
        help_mark(ui.painter(), help, theme.text_muted());
        q.clone().on_hover_text(tooltip);
        // `Button`, so the accessibility tree says it can be clicked, even though clicking it
        // does nothing: the mark's job is to be pointed at, and a screen reader or an agent
        // should be told that pointing is what it is for.
        crate::ui::accessible(&q, WidgetType::Button, format_args!("help {name}"));
    }

    if let Some(close) = marks.close {
        let x = ui.interact(close, ui.id().with(("node-close", name)), Sense::click());
        // silvia's `close.svg`, drawn as the icon's own vector geometry: always `text_primary`
        // — brighter than the `?` beside it at rest, which is silvia's own asymmetry
        // (`.node-tooltip` is muted, `.node-close` is not) — and `accent` on hover, standing
        // in for its `--accent-light`.
        let tint = if x.hovered() {
            theme.accent()
        } else {
            theme.text_primary()
        };
        close_mark(ui.painter(), close, tint);
        hover_with(x.clone(), || format!("delete {name}"));
        crate::ui::accessible(&x, WidgetType::Button, format_args!("close {name}"));
        if x.clicked() {
            fx.commands
                .push(crate::command::Command::RemoveNodes(vec![cx.id]));
        }
    }
    response
}

/// Two marks on each picture the node drew, in the corner a video player puts its controls:
/// the pop-out, and fullscreen at the end, which is the order every player has them in. They
/// are drawn after the border, so they sit over the picture the caller blits into the slot
/// below — and only while the pointer is on the picture, because the render is the thing
/// worth looking at and not the furniture on it.
///
/// `from` is where this node's pictures start in `fx.thumbnails`.
fn picture_marks(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects, from: usize) {
    let theme = cx.theme();
    let name = cx.name();
    for index in from..fx.thumbnails.len() {
        let shown = fx.thumbnails[index];
        let window = cx
            .frame
            .popped
            .iter()
            .find(|p| p.picture.is(cx.id, shown.port));
        let over = ui.rect_contains_pointer(shown.rect);
        // Fullscreen at the end, the pop-out inside it. `mark_rect` counts right to left, so
        // the indices run the other way.
        for (index, fullscreen) in [(0, true), (1, false)] {
            let Some(rect) = mark_rect(shown.rect, cx.zoom(), index) else {
                continue;
            };
            // Lit while the thing it asks for is already true, so a picture on a screen the
            // hand cannot see still says here that it is up, and fullscreen says it is
            // fullscreen. `None` is a mark that is named but not drawn: both go bright
            // together with a hand anywhere on the picture, so the tint is the picture's
            // state, not this mark's.
            let tint = over.then(|| {
                let lit = match (window, fullscreen) {
                    (Some(p), true) => p.fullscreen,
                    (Some(_), false) => true,
                    (None, _) => false,
                };
                if lit {
                    theme.primary()
                } else {
                    theme.text_primary()
                }
            });
            let verb = if fullscreen { "fullscreen" } else { "pop out" };
            let w = picture_mark(
                ui,
                rect,
                ui.id().with(("node-expand", name, shown.port, fullscreen)),
                Lazy(|f: &mut std::fmt::Formatter<'_>| write!(f, "{verb} {name}")),
                if fullscreen { expand_mark } else { popout_mark },
                tint,
                theme,
            );
            // A window showing a picture is session state, like the projector, so this is a
            // request rather than a `Command`.
            if w.clicked() {
                fx.pop_outs.push(crate::ui::PopOutRequest {
                    picture: crate::ui::PopOut::Node {
                        node: cx.id,
                        port: shown.port,
                    },
                    fullscreen,
                });
            }
            if over {
                w.on_hover_text(match (window.is_some(), fullscreen) {
                    (false, false) => "Pop out — this picture in a window of its own",
                    (false, true) => "Fullscreen — this picture, filling a screen",
                    (true, false) => "Close the window of this picture",
                    (true, true) => "Fullscreen this picture's window, or leave fullscreen",
                });
            }
        }
    }
}

/// What a node's picture-strip has to show: where the node has got to, and its monitor level.
///
/// The monitor is a *control*, so a cable into that input takes it away — the strip is a
/// second editor for the address the number row on the node already has, and neither may be
/// dragged while something else is answering it. A node with no `monitor` input has no
/// speaker at all, which is every node but the two audio sources.
fn playing<S: std::hash::BuildHasher>(
    graph: &crate::graph::Graph,
    id: NodeId,
    node: &Node,
    playheads: &std::collections::HashMap<NodeId, f32, S>,
) -> crate::ui::player::Playing {
    crate::ui::player::Playing {
        at: playheads.get(&id).copied(),
        volume: match node.controls.get("monitor") {
            Some(crate::graph::ControlValue::Float(v))
                if graph.source_of(PortRef::new(id, "monitor")).is_none() =>
            {
                Some(*v)
            }
            _ => None,
        },
        // A cable into `position` says where the clip is every frame, and a scrubber cannot
        // argue with it — the node would put the clip back on the next tick. So the bar
        // stays, saying where the graph has put it, and stops being a handle.
        seekable: graph
            .source_of(PortRef::new(id, crate::nodes::timing::OFFSET))
            .is_none(),
    }
}

/// The player's own strip along the foot of each picture — scrubber, mute, volume — on the
/// same hover rule as the marks above it, and over the picture for the same reason: it is the
/// caller's blit that lands underneath.
fn players(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects, from: usize) {
    use crate::command::Command;
    for index in from..fx.thumbnails.len() {
        let shown = fx.thumbnails[index];
        if !ui.rect_contains_pointer(shown.rect) {
            continue;
        }
        // Asked of the graph, which is the one place that can see whether a cable answers the
        // monitor input — a control a cable overrides is not one a hand may drag.
        let playing = playing(cx.frame.graph, cx.id, cx.node, cx.frame.playheads);
        let name = cx.name().to_string();
        match crate::ui::player::show(ui, shown.rect, playing, &name, cx.theme(), cx.zoom()) {
            Some(crate::ui::player::Touched::Seek(to)) => fx.seeks.push((cx.id, to)),
            Some(crate::ui::player::Touched::Volume(v)) => fx.commands.push(Command::SetControl {
                node: cx.id,
                key: "monitor",
                value: crate::graph::ControlValue::Float(v),
            }),
            Some(crate::ui::player::Touched::Mute) => fx.commands.push(Command::SetControl {
                node: cx.id,
                key: "monitor",
                // Unmuting goes back to half, which is where a hand that has never set a
                // level would have put it: the node does not carry what it was before, and
                // session state for one number is a memory nobody asked us to keep.
                value: crate::graph::ControlValue::Float(
                    if playing.volume.is_some_and(|v| v > 0.0) {
                        0.0
                    } else {
                        0.5
                    },
                ),
            }),
            None => {}
        }
    }
}

/// The heading a run of a node's own rows folds under, built from the option that is its
/// state.
///
/// `NodeDef::row_headings` names the option and the option carries the label and the default,
/// so the bar over the render rows and the bar over a region are the same declaration read
/// the same way — there is no second place for the words to live.
fn row_heading_of(def: &'static nodes::NodeDef, key: &'static str) -> Option<nodes::Heading> {
    let key = *def.row_headings.iter().find(|k| **k == key)?;
    let option = def.option(key)?;
    Some(nodes::Heading {
        key,
        label: option.label,
        open: option.default == nodes::ON,
    })
}

/// An Output's status line: silvia's three cells, above its own picture.
///
/// **Color says the same thing twice and never says it alone.** The palette here derives
/// from four anchors and carries no green and no red, so silvia's green/gray/amber cannot be
/// ported as color. The filled and hollow dots are what carry the state — `●` against `○`,
/// which is legible with the hue turned off — and the ink is the neutral ladder under them,
/// with the accent kept for the one cell that is a live capture.
fn readout_lines(ui: &Ui, cx: &NodeCtx<'_>, readout: &crate::ui::OutputReadout) {
    let (theme, zoom) = (cx.theme(), cx.zoom());
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    // Three cells across the body, each a dot and a word.
    let Some(line) = cx.block(canvas::Row::Readout) else {
        return;
    };
    let (a, b) = readout.decks;
    let cells = [
        (
            readout.connected,
            if readout.connected {
                "Input"
            } else {
                "No Input"
            },
        ),
        (
            a || b,
            match (a, b) {
                (true, true) => "A & B",
                (true, false) => "On A",
                (false, true) => "On B",
                (false, false) => "Hidden",
            },
        ),
        (
            readout.rendering,
            if readout.rendering {
                "Rendering"
            } else {
                "Ready"
            },
        ),
    ];
    let third = line.width() / 3.0;
    for (n, (lit, word)) in cells.into_iter().enumerate() {
        // The capture cell is the one that earns the accent: a render running is the only
        // state here a person needs to see from across a room.
        let ink = match (lit, n) {
            (true, 2) => theme.accent(),
            (true, _) => theme.text_primary(),
            (false, _) => theme.text_muted(),
        };
        let at = Pos2::new(
            line.min.x + third * n as f32 + READOUT_INSET * zoom,
            line.center().y,
        );
        ui.painter().text(
            at,
            Align2::LEFT_CENTER,
            format!("{} {word}", if lit { '\u{25cf}' } else { '\u{25cb}' }),
            font.clone(),
            ink,
        );
    }
    let name = cx.name();
    let w = ui.interact(line, ui.id().with(("status", cx.id)), Sense::hover());
    crate::ui::accessible(
        &w,
        WidgetType::Label,
        format_args!(
            "{name} status {} {} {}",
            if readout.connected {
                "input"
            } else {
                "no input"
            },
            match readout.decks {
                (true, true) => "on A and B",
                (true, false) => "on A",
                (false, true) => "on B",
                (false, false) => "hidden",
            },
            if readout.rendering {
                "rendering"
            } else {
                "ready"
            },
        ),
    );
}

/// How far a readout cell sits in from the body's edge, matching an option row's own label.
const READOUT_INSET: f32 = 8.0;

/// The mark a bound control on a row wears, beside its slot: see [`crate::ui::midi_mark`].
fn midi_mark(ui: &Ui, cx: &NodeCtx<'_>, slot: Rect, key: &'static str) {
    let Some(trigger) = cx.frame.bindings.trigger_of(PortRef::new(cx.id, key)) else {
        return;
    };
    crate::ui::midi_mark(
        ui,
        crate::ui::MarkAt::Beside(slot),
        cx.zoom(),
        cx.theme(),
        &format!("{}{} {key}", cx.node.def.slug, cx.id),
        &trigger.label(),
        UNBIND_ON_NODE,
    );
}

/// Where a node's bound control is unbound, as its mark's tooltip says.
pub const UNBIND_ON_NODE: &str = "right-click the control to unbind";

/// silvia's `<hr>` between two non-empty sections of a node — inputs to outputs, or either to
/// options — `border-top: 2px groove border-subtle`. Not drawn under the header: that gap is
/// `canvas::HEADER_GAP` alone, since the header is not a section for a groove to separate two
/// of. A browser paints a `groove` border as a line carved into the surface, darker on the
/// side facing the content above it and lighter on the side facing the content below; two
/// adjacent hairlines stand in for that here, `bg_sunken` (already the darkest neutral in the
/// ladder) above `border_subtle`. The caller clips to the body, since this is a bare pair of
/// `hline`s rather than a shape sharing the body's own rounded corners.
fn groove(
    painter: &eframe::egui::Painter,
    x0: f32,
    x1: f32,
    y: f32,
    theme: &crate::ui::theme::Theme,
) {
    painter.hline(x0..=x1, y - 0.5, Stroke::new(1.0, theme.bg_sunken()));
    painter.hline(x0..=x1, y + 0.5, Stroke::new(1.0, theme.border_subtle()));
}

/// silvia's `circle-help` hook — Feather's `help-circle`,
/// `M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3` — sampled into a polyline in the icon's own 24-unit
/// space: the elliptical arc as 8 points (endpoint-to-center conversion, `theta1 ≈ -160.6°`,
/// `dtheta ≈ 160.7°`, both sampled uniformly), the cubic bezier as 6, sharing their joint at
/// `(14.92, 10.0)`.
const HELP_HOOK_24: [(f32, f32); 15] = [
    (9.09, 9.0),
    (9.6041, 8.0886),
    (10.4, 7.4091),
    (11.3808, 7.0444),
    (12.4272, 7.0387),
    (13.4119, 7.3928),
    (14.2151, 8.0635),
    (14.739, 8.9693),
    (14.92, 10.0),
    (14.6978, 10.9167),
    (14.1422, 11.6667),
    (13.42, 12.25),
    (12.6978, 12.6667),
    (12.1422, 12.9167),
    (11.92, 13.0),
];

/// The hit box is 20x20 (`.node-tooltip`/`.node-close` width/height), but the icon drawn
/// inside it is smaller: `snode.js` calls `iconHtml('circle-help', 14)` and `node.css` sets
/// `.node-close`'s `mask-size: 14px` — both render the mark at 14 of the box's 20 pixels,
/// centerd by the box's own `flex`/`mask-position: center`. A mark drawn at the full 20px
/// read too heavy next to silvia's.
const ICON_RENDER_FRACTION: f32 = 14.0 / 20.0;

/// The icon's own square within the 20x20 hit box, centerd, at `ICON_RENDER_FRACTION` of it.
fn icon_rect(hit_box: Rect) -> Rect {
    let side = hit_box.width() * ICON_RENDER_FRACTION;
    Rect::from_center_size(hit_box.center(), vec2(side, side))
}

/// silvia's `circle-help` (Feather's `help-circle`), viewBox `0 0 24 24`, drawn as the icon's
/// own vector geometry rather than a font glyph in a ring: a circle (`cx 12 cy 12 r 10`), the
/// hook, and its dot (`M12 17h.01`, round-capped — a stroke too short to be anything but its
/// own cap, which is a filled circle the stroke's own radius). `rect` is the mark's own 20x20
/// hit box; the icon itself is drawn into `icon_rect(rect)`, the 14px square centerd in it,
/// and the icon's 24-unit space maps onto *that* square.
fn help_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    use eframe::egui::Shape;
    let icon = icon_rect(rect);
    let scale = icon.width() / 24.0;
    let stroke = Stroke::new(2.0 * scale, color);
    let to_screen = |(x, y): (f32, f32)| icon.min + vec2(x, y) * scale;
    // Each point mapped once, and the whole mark handed over in one call: thirty separate
    // `Painter` methods took the layer's lock thirty times per node per frame.
    let hook = HELP_HOOK_24.map(to_screen);

    let mut shapes = vec![Shape::circle_stroke(
        to_screen((12.0, 12.0)),
        10.0 * scale,
        stroke,
    )];
    shapes.extend(
        hook.windows(2)
            .map(|p| Shape::line_segment([p[0], p[1]], stroke)),
    );
    // A filled circle at each vertex fakes the path's own round joins and caps, which a
    // plain polyline of straight segments does not carry on its own.
    shapes.extend(
        hook.into_iter()
            .chain(std::iter::once(to_screen((12.0, 17.0))))
            .map(|p| Shape::circle_filled(p, stroke.width * 0.5, color)),
    );
    painter.extend(shapes);
}

/// silvia's `close.svg`, viewBox `0 0 20 20`: a filled ring — the annulus between radius 10
/// and radius 8, a 2-unit (10% of the icon) band — and a cross of two bars the same width,
/// each stopping well short of the ring: the path's own corner points put a bar's centerline
/// end about 0.18 of the icon's size out from the center, so the cross spans roughly half the
/// icon's diameter with a clear gap to the ring's inner edge, not a thin X nearly touching it.
/// `rect` is the mark's own 20x20 hit box; `node.css` masks it in at `mask-size: 14px`, so the
/// icon itself is drawn into `icon_rect(rect)`, and every measurement below is a fraction of
/// *that* square's own size rather than the 20-unit viewBox, which is simpler once the ring
/// and the cross are both being read as ratios of the icon rather than as raw path points.
pub fn close_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let icon = icon_rect(rect);
    let s = icon.width();
    let center = icon.center();
    let stroke = Stroke::new(0.10 * s, color);

    painter.circle_stroke(center, 0.45 * s, stroke);
    let d = 0.18 * s;
    painter.line_segment([center - vec2(d, d), center + vec2(d, d)], stroke);
    painter.line_segment([center + vec2(d, -d), center - vec2(d, -d)], stroke);
}

/// Where a picture's `index`-th mark sits, counting right to left from the picture's own
/// top-right corner, at the size and margin the header's marks keep — so the two read as the
/// same family of mark.
///
/// `None` where the picture is too small to carry them without covering what it is a picture
/// of: a node zoomed out, or an Output whose render is a sliver.
pub fn mark_rect(band: Rect, zoom: f32, index: u8) -> Option<Rect> {
    let side = canvas::MARK_SIZE * zoom;
    let margin = canvas::MARK_MARGIN * zoom;
    let step = (side + canvas::MARK_GAP * zoom) * f32::from(index);
    (band.width() > side * 4.0 && band.height() > side * 2.0).then(|| {
        Rect::from_min_size(
            Pos2::new(band.max.x - margin - side - step, band.min.y + margin),
            vec2(side, side),
        )
    })
}

/// One mark over a picture: the hover plate, the icon, the cursor and the accessibility name.
///
/// Both places a picture is drawn call this — the node's own band, and the window that
/// picture is popped out into — because the marks on the two are meant to read as one family
/// of mark, and a family is only one if there is a single place that draws it. `App` reaches
/// for this the way it reaches for `render::` to blit: the chrome stays `ui/`'s.
///
/// `tint` is `None` for a mark that is **named but not drawn** — the hover-reveal rule both
/// callers follow, as one shape rather than two `continue`s. The widget is registered and
/// named either way, so a screen reader and the agent that drives this app by name find it
/// whether or not it is painted, and nothing invisible is in anybody's way: the corner cannot
/// be reached without being over the picture, which is what reveals it.
pub fn picture_mark(
    ui: &mut Ui,
    rect: Rect,
    id: eframe::egui::Id,
    name: impl std::fmt::Display,
    draw: fn(&eframe::egui::Painter, Rect, Color32),
    tint: Option<Color32>,
    theme: &crate::ui::theme::Theme,
) -> Response {
    let w = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
    crate::ui::accessible(&w, WidgetType::Button, &name);
    if let Some(tint) = tint {
        if w.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(crate::ui::theme::RADIUS_SM),
                theme.bg_hover(),
            );
        }
        draw(ui.painter(), rect, tint);
    }
    w
}

/// The mark for a picture in a window of its own: a small frame with an arrow leaving it,
/// which is what "open this elsewhere" has looked like since the first `target="_blank"`.
pub fn popout_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let icon = icon_rect(rect);
    let s = icon.width();
    let stroke = Stroke::new(0.12 * s, color);
    let p = |x: f32, y: f32| Pos2::new(icon.min.x + s * x, icon.min.y + s * y);
    // The frame, open at its top-right corner where the arrow leaves it.
    for [a, b] in [
        [p(0.62, 0.14), p(0.14, 0.14)],
        [p(0.14, 0.14), p(0.14, 0.86)],
        [p(0.14, 0.86), p(0.86, 0.86)],
        [p(0.86, 0.86), p(0.86, 0.42)],
    ] {
        painter.line_segment([a, b], stroke);
    }
    // The arrow: out through the corner, with its head on the outside.
    painter.line_segment([p(0.46, 0.54), p(0.94, 0.06)], stroke);
    painter.line_segment([p(0.62, 0.06), p(0.94, 0.06)], stroke);
    painter.line_segment([p(0.94, 0.06), p(0.94, 0.38)], stroke);
}

/// The Syphon mark: a picture sent out to other apps, drawn as a dot with two arcs spreading
/// from it, the shape a cast or a broadcast has. In the icon's own geometry, like the others.
pub fn syphon_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let icon = icon_rect(rect);
    let s = icon.width();
    let stroke = Stroke::new(0.12 * s, color);
    let origin = Pos2::new(icon.min.x + 0.22 * s, icon.max.y - 0.22 * s);
    painter.circle_filled(origin, 0.1 * s, color);
    for radius in [0.36 * s, 0.66 * s] {
        let arc: Vec<Pos2> = (0..=12)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 * i as f32 / 12.0;
                origin + vec2(a.cos(), a.sin()) * radius
            })
            .collect();
        painter.add(eframe::egui::Shape::line(arc, stroke));
    }
}

/// The NDI mark: a picture sent across the network, drawn as three linked nodes — one sending
/// at the top, two receiving below. In the icon's own geometry, like the others.
pub fn ndi_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let icon = icon_rect(rect);
    let s = icon.width();
    let stroke = Stroke::new(0.12 * s, color);
    let p = |x: f32, y: f32| Pos2::new(icon.min.x + s * x, icon.min.y + s * y);
    let (top, left, right) = (p(0.5, 0.2), p(0.18, 0.8), p(0.82, 0.8));
    painter.line_segment([top, left], stroke);
    painter.line_segment([top, right], stroke);
    painter.line_segment([left, right], stroke);
    for at in [top, left, right] {
        painter.circle_filled(at, 0.14 * s, color);
    }
}

/// The fullscreen mark every video player draws: four corner brackets pointing outward, in
/// the icon's own geometry like the `?` and the `✕`. Its box is `icon_rect(rect)`, the same
/// 14-of-20 square the header's marks are drawn into.
pub fn expand_mark(painter: &eframe::egui::Painter, rect: Rect, color: Color32) {
    let icon = icon_rect(rect);
    let s = icon.width();
    let stroke = Stroke::new(0.12 * s, color);
    // A bracket is two segments from a corner, each running a third of the way along its
    // edge — the shape a player's fullscreen button has had since QuickTime.
    let arm = 0.32 * s;
    for (corner, dx, dy) in [
        (icon.left_top(), 1.0, 1.0),
        (icon.right_top(), -1.0, 1.0),
        (icon.left_bottom(), 1.0, -1.0),
        (icon.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment([corner, corner + vec2(arm * dx, 0.0)], stroke);
        painter.line_segment([corner, corner + vec2(0.0, arm * dy)], stroke);
    }
}

/// Where the header's close button sits, or `None` when the header is too small to carry one.
///
/// Zoomed out far enough that the title is not drawn, an `✕` would be an unlabeled few
/// pixels next to a node you cannot read — easier to hit by accident than on purpose.
fn close_rect(header: Rect, t: &Transform) -> Option<Rect> {
    (t.zoom > canvas::TITLE_ZOOM).then(|| {
        let side = canvas::MARK_SIZE * t.zoom;
        let margin = canvas::MARK_MARGIN * t.zoom;
        Rect::from_center_size(
            Pos2::new(header.max.x - margin - side * 0.5, header.center().y),
            vec2(side, side),
        )
    })
}

/// Where the header's `?` sits: silvia's own gap left of the close button, and gone whenever
/// that is.
///
/// Derived from `close_rect` so the two share their size and their zoom threshold. A `?`
/// with no title beside it names nothing, and the title goes at the same zoom.
fn help_rect(header: Rect, t: &Transform) -> Option<Rect> {
    close_rect(header, t)
        .map(|close| close.translate(vec2(-(close.width() + canvas::MARK_GAP * t.zoom), 0.0)))
}

/// What resetting one number control means: its declared range back, and its declared
/// default back.
///
/// Two commands, because the range and the value are two pieces of state; they coalesce into
/// one undo step the way a drag's moves do. `ClearRange` is only sent for a node that carries
/// an override of its own — the bus refuses it otherwise, and a refused command is still an
/// entry in the frame's list. One function, because the control's own right-click and the
/// range editor's Reset button both mean this and had drifted apart.
pub fn reset_control(
    node: &Node,
    id: NodeId,
    key: &'static str,
    default: f32,
) -> impl Iterator<Item = crate::command::Command> {
    node.values
        .get(key)
        .and_then(crate::graph::Value::range)
        .is_some()
        .then_some(crate::command::Command::ClearRange { node: id, key })
        .into_iter()
        .chain(std::iter::once(crate::command::Command::SetControl {
            node: id,
            key,
            value: crate::graph::ControlValue::Float(default),
        }))
}

/// What one s-number asked for, turned into commands and into the three answers that are not
/// commands.
///
/// **One place, three callers.** A control on a port row, a control inside a region — a
/// coefficient in the palette's grid, a lane's own step count — and an Output's render
/// settings are the same widget over the same kind of value, so a reset puts the range back
/// on each, a right-click opens the same editor on each, and `Alt` + click learns a binding
/// wherever the caller lets it. `at` is where the editor hangs, in world units.
pub fn number_action(
    cx: &NodeCtx<'_>,
    key: &'static str,
    action: crate::ui::number::NumberAction,
    at: Pos2,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
) {
    use crate::command::Command;
    use crate::ui::number;
    let (node, id) = (cx.node, cx.id);
    let number::NumberSpec { default, range, .. } = number_spec(node, key, 0.0, false);
    match action {
        number::NumberAction::Set(next) => fx.commands.push(Command::SetControl {
            node: id,
            key,
            value: crate::graph::ControlValue::Float(next),
        }),
        number::NumberAction::ResetAll => fx.commands.extend(reset_control(node, id, key, default)),
        // A step is part of the range, so it is document data and its own undo step: the
        // range editor opens on the number the stepper just dialed.
        number::NumberAction::SetStep(step) => fx.commands.push(Command::SetRange {
            node: id,
            key,
            range: crate::graph::ControlRange { step, ..range },
        }),
        // Not a command: putting the value back is the collapse of the step the drag opened,
        // and only `App` holds the undo ring.
        number::NumberAction::Cancel(value) => fx.cancel_control = Some((id, key, value)),
        // `Alt` + click: bind this control to the next MIDI message. Not a command — see
        // `app/midi.rs` — so it leaves by its own lane. Nothing, on a control MIDI does not
        // bind.
        number::NumberAction::Learn => {
            if node.def.bindable(key).is_some() {
                fx.learn_midi = Some(PortRef::new(id, key));
            }
        }
        number::NumberAction::OpenRange => {
            let want = crate::ui::OpenControl::Range { node: id, key, at };
            if *open != Some(want) {
                *open = Some(want);
            }
        }
    }
}

/// What an s-number for one of this node's controls shows: `value`, against the control's
/// declared default, unit, curve and range, and the node's own range where it has one.
///
/// Both ranges come from `nodes`, which is the one place that answers what a control's ends
/// are; the widget never derives them from the definition itself. The one place here too:
/// [`number_action`] reads its default and range off this.
fn number_spec(node: &Node, key: &str, value: f32, varying: bool) -> crate::ui::number::NumberSpec {
    let def = node.def;
    let (default, unit, log) = def
        .input(key)
        .and_then(|i| match &i.control {
            crate::nodes::Control::Number {
                default, unit, log, ..
            } => Some((*default, *unit, *log)),
            _ => None,
        })
        .unwrap_or((0.0, "", false));
    let declared = crate::nodes::default_range(node, key).unwrap_or(crate::graph::ControlRange {
        min: f32::MIN,
        max: f32::MAX,
        step: 1.0,
    });
    let range = crate::nodes::control_range(def, node, key).unwrap_or(declared);
    crate::ui::number::NumberSpec {
        value,
        default,
        range,
        declared,
        unit,
        log,
        varying,
        learning: false,
        ghost: None,
    }
}

/// Where a control sits within its row: right-aligned, vertically centered.
fn control_slot(band: Rect, zoom: f32) -> Rect {
    let w = crate::ui::number::WIDTH * zoom;
    let h = crate::ui::number::HEIGHT * zoom;
    Rect::from_min_size(
        Pos2::new(
            band.max.x - w - CONTROL_INSET * zoom,
            band.center().y - h * 0.5,
        ),
        vec2(w, h),
    )
}

/// Toggle one popup: open it, or close it where it is already the one open.
fn toggle(open: &mut Option<crate::ui::OpenControl>, want: crate::ui::OpenControl) {
    *open = if *open == Some(want) {
        None
    } else {
        Some(want)
    };
}

/// Draw the controls that belong to this node's rows, and say what they asked for.
///
/// A control is drawn for an input that has one; when something is connected to that input
/// the control is shown disabled and draws the **arriving** value rather than the stored
/// one, fill included, because a connection overrides the stored number and a control
/// showing it would be saying something untrue. Popups are not opened here — clicking
/// records a request in `open`, and `ui::show` draws it after every node, so a node painted
/// later cannot cover it.
///
/// Each part of a node registers its widgets in the order it is drawn, top to bottom, and the
/// width grip last of all: a widget egui sees later wins a tie, and the grip has to outrank
/// both the box it sits in the corner of and the body ground under it.
pub fn controls(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
) {
    if cx.zoom() < canvas::DETAIL_ZOOM {
        return;
    }
    input_controls(ui, cx, fx, open);
    // The heading a moving node's time rows fold under, the same bar again, with its mode on
    // the bar's right end, open or closed.
    if canvas::timing_heading(cx.node).is_some() {
        row_heading(
            ui,
            cx,
            fx,
            crate::nodes::timing::HEADING.key,
            canvas::Row::TimingHeading,
            canvas::timing_shown(cx.node),
        );
        heading_options(
            ui,
            cx,
            fx,
            crate::nodes::timing::HEADING.key,
            canvas::Row::TimingHeading,
        );
    }
    // The heading over the Render section: the same bar, triangle and turn a region's heading
    // is, drawn over rows instead of over a band. It is on every Output whether the section
    // is open or closed, because a closed section still has to say it is there — which a tick
    // in a shared row at the foot of the node could not.
    if canvas::render_heading(cx.node) {
        row_heading(
            ui,
            cx,
            fx,
            crate::nodes::output::OFFLINE,
            canvas::Row::RenderHeading,
            canvas::render_shown(cx.node),
        );
        // And the one over its Send rows, the same bar again.
        row_heading(
            ui,
            cx,
            fx,
            crate::nodes::output::SEND,
            canvas::Row::SendHeading,
            canvas::send_shown(cx.node),
        );
    }
    // The `!` beside the Render button, clicked: its popup hangs under it.
    let mut live_at = None;
    if canvas::render_shown(cx.node) {
        render_section(ui, cx, fx, open, &mut live_at);
    }
    if canvas::send_shown(cx.node) {
        send_section(ui, cx, fx);
    }
    // The status line, directly above the Output's own picture, where silvia's own sits.
    if let Some(r) = cx.frame.readouts.get(&cx.id) {
        readout_lines(ui, cx, r);
    }
    option_rows(ui, cx, fx, open);
    value_rows(ui, cx, fx);
    check_row(ui, cx, fx);
    // The grip that sets the body's width, for the one kind that has one.
    if cx.node.def.resizable {
        let corner = cx
            .layout
            .blocks()
            .filter(|(r, _)| matches!(r.row, canvas::Row::Value(_)))
            .last()
            .map_or(cx.layout.rect, |(_, band)| band);
        fx.commands.extend(width_grip(ui, cx, corner));
    }
    if let Some(at) = live_at {
        toggle(
            open,
            crate::ui::OpenControl::Live {
                node: cx.id,
                at: cx.world(at),
            },
        );
    }
}

/// The control on each input row: an s-number, an s-color, or for an action input the press
/// button that spans the row.
fn input_controls(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
) {
    use crate::graph::ControlValue;
    use crate::ui::{OpenControl, color, number};
    let (node, id, theme, zoom) = (cx.node, cx.id, cx.theme(), cx.zoom());
    for (index, port) in node.inputs.iter().enumerate() {
        let Some(current) = node.controls.get(port.key) else {
            press_row(ui, cx, fx, index, port.key);
            loop_meter(ui, cx, index, port.key);
            continue;
        };
        let Some(band) = cx.block(canvas::Row::Input(index)) else {
            continue;
        };
        let slot = control_slot(band, zoom);
        midi_mark(ui, cx, slot, port.key);
        let source = cx.frame.graph.source_of(PortRef::new(id, port.key));
        let enabled = source.is_none();
        // A **varying** value arriving here is the case with no value to meter at all: a
        // uniform publishes one number or one color a frame and the control shows it, but a
        // varying one is a value per pixel and there is nothing to put on a row. Asked of
        // the source's own type rather than of whether anything was published, because those
        // are two different silences — a uniform that has not run yet still has one coming.
        let varying = source.is_some_and(|src| {
            cx.frame
                .graph
                .get(src.node)
                .and_then(|n| n.output(src.key))
                .is_some_and(|p| p.ty.is_varying())
        });
        let name = cx.control(port.key);

        match current {
            ControlValue::Float(value) => {
                // What the node actually sees. A connected input's stored number stopped
                // being the value the moment the cable landed, so the disabled control is a
                // meter of what arrives — text, fill and accessible name together. Where
                // the producer has published nothing there is nothing to meter, and the
                // stored number is what the control has to say.
                //
                // A value outside the control's range keeps its digits and clamps its fill:
                // `NumberSpec::fraction` is a position on a track and a position off the
                // track's end is the end, but rounding the number to the range would be the
                // control lying about the arriving value a second time.
                let shown = cx
                    .frame
                    .uniforms
                    .arriving(source)
                    .map_or(*value, |v| v as f32);
                let spec = crate::ui::number::NumberSpec {
                    learning: cx.learning(port.key),
                    ghost: cx.ghost(port.key),
                    ..number_spec(node, port.key, shown, varying)
                };
                let lock = cx.frame.prefs.lock_cursor;
                if let Some(action) =
                    number::scrub(ui, slot, name, &spec, theme, enabled, lock, zoom)
                {
                    number_action(cx, port.key, action, cx.world(slot.left_bottom()), fx, open);
                }
            }
            ControlValue::Color(value) => {
                // What the node actually sees, exactly as a connected number control meters
                // the number arriving down its cable: a swatch whose cable carries a uniform
                // color shows that color. Where the producer has published nothing there is
                // nothing to meter, and the stored color is what the swatch has to say.
                let shown = cx.frame.uniforms.arriving_color(source).unwrap_or(*value);
                let response = color::swatch(ui, slot, name, shown, theme, enabled, varying, zoom);
                if response.clicked() {
                    toggle(
                        open,
                        OpenControl::Color {
                            node: id,
                            key: port.key,
                            at: cx.world(slot.left_bottom()),
                        },
                    );
                }
            }
        }
    }
}

/// The loop meter on a Time row, where the Speed knob stands in the other mode: the node's
/// Time as the node reads it, through its axis's period (`timing::period_in`). The row is
/// drawn only in Loop mode under an open Timing heading, so the meter is too.
fn loop_meter(ui: &mut Ui, cx: &NodeCtx<'_>, index: usize, key: &'static str) {
    if cx.node.def.timing.is_none() || !canvas::speed_tall(cx.node, index) {
        return;
    }
    let Some(band) = cx.block(canvas::Row::Input(index)) else {
        return;
    };
    let at = PortRef::new(cx.id, key);
    // The axis's period as the graph gives it: a control a cable drives read as anything.
    let period = crate::nodes::timing::axis_of(key)
        .and_then(|axis| crate::nodes::timing::period_in(cx.frame.graph, cx.id, axis));
    let progress = cx
        .frame
        .uniforms
        .time(at, cx.frame.graph.source_of(at))
        .map(|time| crate::nodes::timing::Progress::of(time, period));
    let zoom = cx.zoom();
    crate::ui::loop_meter::meter(
        ui,
        control_slot(band, zoom),
        cx.control(key),
        progress,
        cx.theme(),
        zoom,
    );
}

/// An action input holds no value, so it has no control of its own. What it may have is a
/// button, drawn spanning the row rather than a control hugging one end of it, and reporting
/// a level rather than an edit.
fn press_row(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects, index: usize, key: &'static str) {
    use crate::ui::press;
    let input_def = cx.node.def.input(key);
    if !matches!(
        input_def.map(|i| &i.control),
        Some(crate::nodes::Control::Press)
    ) {
        return;
    }
    let Some(band) = cx.block(canvas::Row::Input(index)) else {
        return;
    };
    let zoom = cx.zoom();
    let height = press::SIZE * zoom;
    let pad = 8.0 * zoom;
    // The row's left edge sits right at the port dot, so the button needs the same
    // clearance there that a port label gets from the same edge.
    let left_pad = LABEL_INSET * zoom;
    let slot = Rect::from_min_size(
        Pos2::new(band.min.x + left_pad, band.center().y - height * 0.5),
        vec2((band.width() - left_pad - pad).max(0.0), height),
    );
    midi_mark(ui, cx, slot, key);
    let caption = input_def.map_or(key, |i| i.label);
    let port = PortRef::new(cx.id, key);
    // Never disabled by a connection. An action input takes many sources and the hand is one
    // more of them, which is what makes a button beside a sequencer lane an override rather
    // than a conflict.
    let fire = cx.fires.get(&port).copied().unwrap_or(0.0);
    let learning = cx.learning(key);
    match press::button(
        ui,
        slot,
        caption,
        cx.control(key),
        cx.theme(),
        zoom,
        fire,
        learning,
    ) {
        press::Press::Held => fx.held.push(port),
        press::Press::Learn => fx.learn_midi = Some(port),
        press::Press::Idle => {}
    }
}

/// The bar over a run of an Output's rows — its Render section or its Send rows — and the
/// option it turns.
fn row_heading(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    key: &'static str,
    row: canvas::Row,
    open: bool,
) {
    let Some(heading) = row_heading_of(cx.node.def, key) else {
        return;
    };
    let Some(strip) = cx.block(row) else {
        return;
    };
    // Drawn after the node's border, so its bar stops short of the border on each side that
    // reaches it: both on a full-width row, the port side alone on the Timing heading, whose
    // far end stops short where the inputs do.
    let border = border_width(cx.selected);
    let flush = strip.max.x >= cx.screen(cx.layout.rect).max.x - 0.5;
    if crate::widgets::heading_row(
        ui,
        strip,
        [border, if flush { border } else { 0.0 }],
        heading,
        open,
        cx.node,
        cx.id,
        cx.zoom(),
        cx.theme(),
    ) {
        fx.commands.push(crate::command::Command::SetOption {
            node: cx.id,
            key: heading.key,
            value: if open { nodes::OFF } else { nodes::ON }.to_string(),
        });
    }
}

/// The options a heading's bar carries at its right end (`OptionDef::on_heading`), each a row
/// of segments: a time-driven node's Free and Loop. Drawn after the bar, so a click on a
/// segment is the segment's and not the fold's.
fn heading_options(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    heading: &'static str,
    row: canvas::Row,
) {
    let Some(strip) = cx.block(row) else {
        return;
    };
    let (node, def) = (cx.node, cx.node.def);
    for option in def.options.iter().filter(|o| o.on_heading == Some(heading)) {
        let chosen = node
            .options
            .get(option.key)
            .map_or(option.default, String::as_str);
        let name = format!("{}{}.{}", def.slug, cx.id, option.key);
        if let Some(i) = crate::widgets::heading_segments(
            ui,
            strip,
            option.choices,
            chosen,
            &name,
            cx.zoom(),
            cx.theme(),
        ) {
            fx.commands.push(crate::command::Command::SetOption {
                node: cx.id,
                key: option.key,
                value: option.choices[i].0.to_string(),
            });
        }
    }
}

/// An Output's Render section, under its heading: silvia's offline output's three selects and
/// three numbers — fps, duration, warm-up frames — on rows of their own, and the button that
/// starts a render or cancels the one running, with its progress behind it. Every number here
/// goes inert while any render runs, since the document is closed.
fn render_section(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
    live_at: &mut Option<Pos2>,
) {
    use crate::graph::ControlValue;
    use crate::ui::number;
    let (node, def, theme, zoom) = (cx.node, cx.node.def, cx.theme(), cx.zoom());
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    // The selects, drawn by the row the option block draws its own with.
    for (i, key) in crate::nodes::output::RENDER_SELECTS.iter().enumerate() {
        if let (Some(band), Some(value)) = (cx.block(canvas::Row::Render(i)), node.options.get(key))
        {
            option_row(ui, cx, fx, open, band, key, value);
        }
    }
    for (i, key) in crate::nodes::output::RENDER_CONTROLS.iter().enumerate() {
        let Some(band) = cx.block(canvas::Row::Render(canvas::RENDER_SELECTS + i)) else {
            continue;
        };
        let Some(input) = def.input(key) else {
            continue;
        };
        let Some(ControlValue::Float(value)) = node.controls.get(key) else {
            continue;
        };
        ui.painter().text(
            band.left_center() + vec2(12.0 * zoom, 0.0),
            Align2::LEFT_CENTER,
            input.label,
            font.clone(),
            theme.text_secondary(),
        );
        let slot = control_slot(band, zoom);
        // An Output's render settings are the node's own numbers; no port, so no cable and
        // nothing to arrive.
        let spec = number_spec(node, key, *value, false);
        let enabled = cx.frame.render.is_none();
        let lock = cx.frame.prefs.lock_cursor;
        // Not learnable: the Output lists these as `NodeDef::unbindable`, so `Alt` + click
        // does nothing here and the range editor offers no binding.
        if let Some(action) =
            number::scrub(ui, slot, cx.control(key), &spec, theme, enabled, lock, zoom)
        {
            number_action(cx, key, action, cx.world(slot.left_bottom()), fx, open);
        }
    }
    if let Some(band) = cx.block(canvas::Row::Render(canvas::RENDER_ROWS - 1)) {
        render_button(ui, cx, fx, band, live_at);
    }
}

/// The button that starts a render of this Output or cancels the one running, with its
/// progress behind it, and the `!` beside it where a source upstream can only answer now.
fn render_button(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    band: Rect,
    live_at: &mut Option<Pos2>,
) {
    use crate::ui::press;
    let (id, theme, zoom, render) = (cx.id, cx.theme(), cx.zoom(), cx.frame.render);
    let live = cx.frame.live.get(&id).is_some_and(|l| !l.is_empty());
    let height = press::SIZE * zoom;
    let pad = 8.0 * zoom;
    // The `!` takes a square off the button's right end, only when there is one.
    let bang_room = if live { height + pad } else { 0.0 };
    let slot = Rect::from_min_size(
        Pos2::new(band.min.x + pad, band.center().y - height * 0.5),
        vec2((band.width() - pad * 2.0 - bang_room).max(0.0), height),
    );
    // A source upstream that can only answer now — a camera, a screen, a microphone — means a
    // render reads whatever it gives. The mark says so beside the button that would start
    // one, and its popup names each source and goes there. It warns and nothing more: the
    // render runs.
    if live {
        let name = cx.name();
        let rect = Rect::from_min_size(
            Pos2::new(slot.max.x + pad, slot.min.y),
            vec2(height, height),
        );
        let w = ui.interact(rect, ui.id().with(("node-live", name)), Sense::click());
        if w.hovered() {
            ui.painter()
                .circle_filled(rect.center(), rect.width() * 0.5, theme.bg_hover());
        }
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "!",
            FontId::monospace(crate::ui::theme::font_size(
                crate::ui::theme::FONT_BASE,
                zoom,
            )),
            theme.accent(),
        );
        if w.hovered() {
            w.clone().on_hover_text(
                "A source upstream can only answer now — a camera, a screen, a \
                 microphone — so a render reads whatever it gives. Click to see which.",
            );
        }
        if w.clicked() {
            *live_at = Some(rect.left_bottom());
        }
        crate::ui::accessible(&w, WidgetType::Button, format_args!("{name}.live"));
    }
    let mine = render.filter(|r| r.node == id);
    let (caption, progress) = match mine {
        Some(r) => (
            format!("Cancel  {} / {}", r.written, r.frames),
            Some(r.written as f32 / r.frames.max(1) as f32),
        ),
        None => ("Render".to_string(), None),
    };
    // Another Output's render closes this one's button with the rest of the document.
    let enabled = mine.is_some() || render.is_none();
    if press::click(
        ui,
        slot,
        &caption,
        cx.control("render"),
        theme,
        zoom,
        progress,
    ) && enabled
    {
        fx.render_requests.push(crate::ui::RenderRequest {
            node: id,
            cancel: mine.is_some(),
        });
    }
}

/// How wide a way's button is, in world units: room for its longest caption, *Get it*.
const SEND_BUTTON: f32 = 44.0;
/// The air between a way's status and its button, in world units.
const SEND_GAP: f32 = 2.0;

/// Where a Send row's status starts, from the row's left edge: a space past the longest name
/// of the ways this Output shows, so their lines start together. Flip's label starts here too,
/// under the Syphon row it belongs to.
fn send_column(node: &Node, band: Rect, zoom: f32, advance: f32) -> f32 {
    let widest = canvas::send_rows(node)
        .filter_map(|row| match row {
            nodes::output::SendRow::Way(way) => Some(way.label().chars().count()),
            nodes::output::SendRow::Name => Some(NAME_LABEL.chars().count()),
            nodes::output::SendRow::Flip | nodes::output::SendRow::Alpha => None,
        })
        .max()
        .unwrap_or(0);
    band.min.x + 12.0 * zoom + (widest + 1) as f32 * advance
}

/// An Output's Send rows, under their heading: the name it goes out under, then one row per
/// way out — its name, a dot and one
/// line saying what it is doing, and the one button that changes it — then Syphon's Flip
/// under Syphon while it publishes, and the alpha both ways share while either sends.
///
/// Every row is a fixed height and its line is cut short rather than wrapped: what a sender
/// reports changes what a row says, never where anything is.
fn send_section(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects) {
    use crate::nodes::output::SendRow;
    for row in canvas::send_rows(cx.node) {
        let Some(band) = cx.block(canvas::Row::Send(row)) else {
            continue;
        };
        match row {
            SendRow::Name => name_row(ui, cx, fx, band),
            SendRow::Way(way) => way_row(ui, cx, fx, band, way),
            SendRow::Flip => choice_row(ui, cx, fx, band, row.key(), true),
            SendRow::Alpha => choice_row(ui, cx, fx, band, row.key(), false),
        }
    }
}

/// What the name row is called.
const NAME_LABEL: &str = "Name";

/// The name an Output goes out under, every way: its label, and a field from the status
/// column to the row's end holding the name — the default where it has none of its own —
/// committed on Enter or when the field is left, never a keystroke at a time, since a
/// commit restarts whatever is on air.
fn name_row(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects, band: Rect) {
    use crate::nodes::output;
    let (node, theme, zoom) = (cx.node, cx.theme(), cx.zoom());
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    ui.painter().text(
        band.left_center() + vec2(12.0 * zoom, 0.0),
        Align2::LEFT_CENTER,
        NAME_LABEL,
        font.clone(),
        theme.text_secondary(),
    );
    let column = send_column(node, band, zoom, advance(ui.ctx(), &font));
    let h = canvas::SELECT_HEIGHT * zoom;
    let field = Rect::from_min_max(
        Pos2::new(column, band.center().y - h * 0.5),
        Pos2::new(band.max.x - CONTROL_INSET * zoom, band.center().y + h * 0.5),
    );
    let name = output::send_name(cx.id, node);
    let default = output::default_name(cx.id);
    let typed = crate::ui::text::line(
        ui,
        field,
        cx.control(output::SEND_NAME),
        &name,
        &default,
        &output::name_hover(&default),
        theme,
        zoom,
    );
    // Only a change is an edit: leaving the field as it was opens no undo step and restarts
    // nothing. Empty is the default, and the bus makes the name its own.
    if let Some(typed) = typed.filter(|t| t.trim() != name) {
        fx.commands.push(crate::command::Command::SetOption {
            node: cx.id,
            key: output::SEND_NAME,
            value: typed,
        });
    }
}

/// One way out: its name, its status with the whole of it under the pointer, and its button.
///
/// The dot says the state as the status line's do — `●` on air, `○` anything else — with the
/// neutral ladder under it, and an error in the brighter of the two unlit inks.
fn way_row(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects, band: Rect, way: nodes::output::Way) {
    use crate::nodes::output::{self, Press as Asks, Way};
    use crate::ui::press;
    let (node, theme, zoom) = (cx.node, cx.theme(), cx.zoom());
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    let advance = advance(ui.ctx(), &font);
    ui.painter().text(
        band.left_center() + vec2(12.0 * zoom, 0.0),
        Align2::LEFT_CENTER,
        way.label(),
        font.clone(),
        theme.text_secondary(),
    );
    let on = canvas::tick(node, way.key()).unwrap_or(false);
    // The name is needed only to say it is on air, so an Output sending nothing builds none.
    let name = if on {
        output::send_name(cx.id, node)
    } else {
        String::new()
    };
    let error = cx.frame.readouts.get(&cx.id).and_then(|r| r.error(way));
    let missing = match way {
        Way::Ndi => crate::video::ndi::absent(),
        Way::Syphon => None,
    };
    let status = output::status(on, &name, error, missing);

    let (w, h) = (SEND_BUTTON * zoom, press::SIZE * zoom);
    let button = Rect::from_min_size(
        Pos2::new(
            band.max.x - CONTROL_INSET * zoom - w,
            band.center().y - h * 0.5,
        ),
        vec2(w, h),
    );
    let column = send_column(node, band, zoom, advance);
    let ink = if status.on_air {
        theme.text_primary()
    } else if on && error.is_some() && missing.is_none() {
        theme.text_secondary()
    } else {
        theme.text_muted()
    };
    ui.painter().text(
        Pos2::new(column, band.center().y),
        Align2::LEFT_CENTER,
        if status.on_air {
            "\u{25cf}"
        } else {
            "\u{25cb}"
        },
        font.clone(),
        ink,
    );
    let text_at = column + 2.0 * advance;
    let room = (button.min.x - SEND_GAP * zoom - text_at).max(0.0);
    ui.painter().text(
        Pos2::new(text_at, band.center().y),
        Align2::LEFT_CENTER,
        fit(&status.line, advance, room, Keep::Start),
        font,
        ink,
    );
    let line = Rect::from_min_max(
        Pos2::new(column, band.min.y),
        Pos2::new(button.min.x, band.max.y),
    );
    let hover = ui.interact(
        line,
        ui.id().with(("send-status", cx.id, way.key())),
        Sense::hover(),
    );
    crate::ui::accessible(
        &hover,
        WidgetType::Label,
        format_args!("{}.status {}", cx.control(way.key()), status.line),
    );
    if hover.hovered() {
        hover.on_hover_text(output::status_hover(
            way,
            &output::send_name(cx.id, node),
            error,
            missing.map(|m| m.why),
        ));
    }

    let asks = status.press;
    if press::click(
        ui,
        button,
        asks.caption(),
        cx.control(way.key()),
        theme,
        zoom,
        None,
    ) {
        match asks {
            // The page the missing-runtime message names, as a link anywhere in egui opens.
            Asks::GetIt => ui.ctx().open_url(eframe::egui::OpenUrl::new_tab(
                crate::video::ndi::RUNTIME_URL,
            )),
            Asks::Send | Asks::Stop => fx.commands.push(crate::command::Command::SetOption {
                node: cx.id,
                key: way.key(),
                value: if on { nodes::OFF } else { nodes::ON }.to_string(),
            }),
        }
    }
}

/// A two-valued send option — Syphon's Flip, the shared alpha — as its label and a choice
/// between its two values by their own names. `under` sets the label in the status column,
/// under the way it belongs to.
fn choice_row(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    band: Rect,
    key: &'static str,
    under: bool,
) {
    use crate::ui::press;
    let (node, theme, zoom) = (cx.node, cx.theme(), cx.zoom());
    let Some(option) = node.def.option(key) else {
        return;
    };
    let &[(first, first_caption), (second, second_caption)] = option.choices else {
        return;
    };
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    let x = if under {
        send_column(node, band, zoom, advance(ui.ctx(), &font))
    } else {
        band.min.x + 12.0 * zoom
    };
    ui.painter().text(
        Pos2::new(x, band.center().y),
        Align2::LEFT_CENTER,
        option.label,
        font,
        theme.text_secondary(),
    );
    let captions = [first_caption, second_caption];
    let w = press::choice_width(ui.ctx(), &captions, zoom);
    let h = canvas::SELECT_HEIGHT * zoom;
    let rect = Rect::from_min_size(
        Pos2::new(
            band.max.x - CONTROL_INSET * zoom - w,
            band.center().y - h * 0.5,
        ),
        vec2(w, h),
    );
    let chosen = usize::from(node.options.get(key).is_some_and(|v| v == second));
    if let Some(i) = press::choice(ui, rect, &captions, chosen, cx.control(key), theme, zoom) {
        fx.commands.push(crate::command::Command::SetOption {
            node: cx.id,
            key,
            value: [first, second][i].to_string(),
        });
    }
}

/// Select options, such as an Output's resolution.
///
/// Drawn in the definition's declaration order rather than `Node::options`' own —
/// `BTreeMap` stays the storage, for a deterministic file and command bus, but the order an
/// option is *authored* in is part of how the node explains itself, and alphabetising it by
/// key scrambles that for free.
fn option_rows(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
) {
    let (node, def) = (cx.node, cx.node.def);
    let ordered = def
        .options
        .iter()
        // The ticks are not here: they share the one `Checks` row below, which is also why
        // `Row::Option(i)` counts only the selects. Nor is an option a region draws, whose
        // region is its control.
        .filter(|o| !o.checkbox && !o.in_region && o.on_heading.is_none())
        // The render's own selects are drawn in its section, under its heading, and the send
        // options are the Send rows.
        .filter(|o| {
            !(def.is_output
                && (crate::nodes::output::RENDER_SELECTS.contains(&o.key)
                    || crate::nodes::output::SEND_OPTIONS.contains(&o.key)))
        })
        .filter_map(|o| node.options.get(o.key).map(|v| (o.key, v)));
    for (index, (key, value)) in ordered.enumerate() {
        let Some(band) = cx.block(canvas::Row::Option(index)) else {
            continue;
        };
        option_row(ui, cx, fx, open, band, key, value);
    }
}

/// One option's row: its label, and the select, the file button or the typed field it is.
fn option_row(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    fx: &mut Effects,
    open: &mut Option<crate::ui::OpenControl>,
    band: Rect,
    key: &'static str,
    value: &String,
) {
    use crate::command::Command;
    use crate::ui::OpenControl;
    let (node, id, def, theme, zoom) = (cx.node, cx.id, cx.node.def, cx.theme(), cx.zoom());
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    let option_def = def.option(key);
    // The declared label, falling back to the key when the option is not in the registry.
    // Measured, not assumed: what is left of the row after it is how wide the select may
    // grow, which is what lets a long path show more of itself on a wide node than on a
    // narrow one.
    let label = ui.painter().text(
        band.left_center() + vec2(12.0 * zoom, 0.0),
        Align2::LEFT_CENTER,
        option_def.map_or(key, |o| o.label),
        font.clone(),
        theme.text_secondary(),
    );
    // What this choice costs, beside the choice that drives it — silvia prints a memory
    // figure under its own resolution so a second Output at 4K can be thought about before
    // it is made. Dimmer than the label and measured like it, so the select gets whatever is
    // left rather than growing under it.
    let mut taken = label.max.x;
    if def.is_output && key == "resolution" {
        let figure = ui.painter().text(
            Pos2::new(label.max.x + READOUT_GAP * zoom, band.center().y),
            Align2::LEFT_CENTER,
            crate::nodes::output::memory_label(node),
            font,
            theme.text_muted(),
        );
        taken = figure.max.x;
    }
    let room = (band.max.x - 8.0 * zoom) - (taken + 8.0 * zoom);
    let name = cx.control(key);
    // A number typed as text, such as the Text node's size: committed once, clamped, and
    // written back the way the field writes it. Placed as a select is — right-aligned at the
    // select's height, beside the label — and wide enough for its longest number and its
    // unit.
    if let Some(bounds) = option_def.and_then(|o| o.number) {
        let font = FontId::monospace(crate::ui::theme::font_size(
            crate::ui::theme::FONT_TINY,
            zoom,
        ));
        let digits = [bounds.min, bounds.max]
            .map(|v| bounds.written(v).chars().count())
            .into_iter()
            .max()
            .unwrap_or(1);
        let unit = if bounds.unit.is_empty() {
            0
        } else {
            bounds.unit.chars().count() + 1
        };
        let width = (advance(ui.ctx(), &font) * (digits + unit) as f32 + 2.0 * SELECT_PAD * zoom)
            .min(room.max(0.0));
        let height = canvas::SELECT_HEIGHT * zoom;
        let rect = Rect::from_min_size(
            Pos2::new(
                band.max.x - 8.0 * zoom - width,
                band.center().y - height * 0.5,
            ),
            vec2(width, height),
        );
        let chrome = crate::ui::text::Chrome::Row;
        if let Some(next) =
            crate::ui::text::number(ui, rect, name, value, bounds, chrome, theme, zoom)
        {
            let next = bounds.written(next);
            if next != *value {
                fx.commands.push(Command::SetOption {
                    node: id,
                    key,
                    value: next,
                });
            }
        }
        return;
    }
    // A typed option, such as Lyapunov's `sequence`: an always-open field rather than a
    // list, with `choices` reachable by typing them rather than by picking them.
    if let Some(opt) = option_def
        && let Some(placeholder) = opt.placeholder
    {
        let field = crate::ui::text::Field {
            value,
            placeholder,
            valid: opt.validate.is_none_or(|v| v(value)),
            rows: 1,
        };
        if let Some(next) = crate::ui::text::edit(ui, band, name, field, theme, zoom).text {
            fx.commands.push(Command::SetOption {
                node: id,
                key,
                value: next,
            });
        }
        return;
    }
    if option_def.is_some_and(nodes::OptionDef::is_asset) {
        file_button(ui, cx, open, band, room, key, value);
        return;
    }
    // An option a connected input answers instead: the select goes inert and reads that
    // input's label, so the row says where the value comes from rather than what was last
    // chosen. Which option and which input is `OptionDef::overridden_by`, so nothing here
    // knows what node it is drawing.
    let overriding = option_def.and_then(|o| {
        let by = o.overridden_by?;
        cx.frame.graph.source_of(PortRef::new(id, by))?;
        def.input(by).map(|i| i.label)
    });
    let is_open = matches!(
        *open,
        Some(OpenControl::Select { node, key: k, .. }) if node == id && k == key
    );
    // The choice's display name, where the stored value is one of the declared choices — a
    // native select shows the name closed as well as open, and the value is for the machine.
    // A value that names no choice (a hand-edited file) falls back to itself, same
    // as an unknown option falls back to its key above.
    let shown_value = option_def
        .and_then(|o| o.menu().iter().find(|(v, _)| *v == value.as_str()))
        .map_or(value.as_str(), |(_, name)| name);
    let select = Select {
        value: overriding.unwrap_or(shown_value),
        keep: Keep::Start,
        state: match (overriding.is_some(), is_open) {
            (true, _) => SelectState::Overridden,
            (false, true) => SelectState::Open,
            (false, false) => SelectState::Idle,
        },
        fill: None,
    };
    let response = select_button(ui, band, room, name, &select, theme, zoom);
    // A disabled select senses hover only, so this never fires while a cable overrides it.
    if response.clicked() {
        *open = (!is_open).then(|| OpenControl::Select {
            node: id,
            key,
            at: cx.world(response.rect.left_bottom()),
            width: response.rect.width() / zoom,
        });
    }
}

/// An `Asset` option's select: the file it names, and the picker it opens.
fn file_button(
    ui: &mut Ui,
    cx: &NodeCtx<'_>,
    open: &mut Option<crate::ui::OpenControl>,
    band: Rect,
    room: f32,
    key: &'static str,
    value: &String,
) {
    use crate::ui::OpenControl;
    let (id, zoom) = (cx.id, cx.zoom());
    let note = cx.frame.notes.get(&id);
    // While the CPU half is working on this file, the select is its progress bar. Otherwise
    // a path is shown by its file name — the reference's directory is always `assets/` and
    // says nothing — with the whole thing as the hover text.
    let shown = match note {
        Some(n) => n.text.clone(),
        None if value.is_empty() => "choose…".to_string(),
        None => std::path::Path::new(value)
            .file_name()
            .map_or_else(|| value.clone(), |n| n.to_string_lossy().into_owned()),
    };
    // Matched on which control it belongs to, not on the whole variant: where the list hangs
    // from is measured from the select that was drawn, and that is not known until after it
    // is.
    let is_open = matches!(
        *open,
        Some(OpenControl::Asset { node, key: k, .. }) if node == id && k == key
    );
    let select = Select {
        value: &shown,
        // A file name's end is the part that tells files apart: the episode number, the
        // extension.
        keep: Keep::End,
        state: if is_open {
            SelectState::Open
        } else {
            SelectState::Idle
        },
        fill: note.and_then(|n| n.progress),
    };
    let response = select_button(ui, band, room, cx.control(key), &select, cx.theme(), zoom);
    if !value.is_empty() {
        response.clone().on_hover_text(value);
    }
    if response.clicked() {
        *open = (!is_open).then(|| OpenControl::Asset {
            node: id,
            key,
            at: cx.world(response.rect.left_bottom()),
            width: response.rect.width() / zoom,
        });
    }
}

/// The node's own values, each drawn by the kind it declares. Unlike an option's row, which
/// one renderer paints for every node in the library, this is a match on the kind — and the
/// kind is what keeps that match here rather than on a slug, which `ui/` may never do.
fn value_rows(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects) {
    use crate::command::Command;
    let (node, id) = (cx.node, cx.id);
    for (index, value_def) in node.def.values.iter().enumerate() {
        let Some(band) = cx.block(canvas::Row::Value(index)) else {
            continue;
        };
        match value_def.kind {
            nodes::ValueKind::Text {
                rows, placeholder, ..
            } => {
                let text = node
                    .values
                    .get(value_def.key)
                    .and_then(crate::graph::Value::text)
                    .unwrap_or_default();
                let field = crate::ui::text::Field {
                    value: text,
                    placeholder,
                    // Nothing a note can hold is invalid: it is prose, and the border says
                    // "this will not compile" about things that compile.
                    valid: true,
                    // The floor under an empty box, in lines. How tall it actually ends up
                    // is the text's business, and the field reports that back below.
                    rows,
                };
                let name = cx.control(value_def.key);
                let edited = crate::ui::text::edit(ui, band, name, field, cx.theme(), cx.zoom());
                if let Some(next) = edited.text {
                    fx.commands.push(Command::SetValue {
                        node: id,
                        key: value_def.key,
                        value: crate::graph::Value::Text(next),
                    });
                }
                // No cap. One was tried, at 320 points, and a node that stops growing while
                // its text does not is a node with text lying on the canvas underneath it.
                // A note tall enough to be a nuisance has the same answer every node has:
                // collapse it to its header.
                let wants = edited.height;
                // Compared loosely: a height is a float that comes back through a galley and
                // a zoom, and reporting a change every frame over a rounding error would keep
                // the node resizing forever.
                if (canvas::value_height(node, cx.layout.measured, index) - wants).abs() > 0.5 {
                    fx.grown.push((id, index, wants));
                }
            }
            // Drawn in a region, which has no row to find here — see `widgets::curve`,
            // `widgets::steps` and `widgets::paint`.
            nodes::ValueKind::Points { .. }
            | nodes::ValueKind::Cells { .. }
            | nodes::ValueKind::Painting => {}
        }
    }
}

/// The row of ticks, every one the node declares side by side — silvia's
/// `.audio-visibility-toggles`, where an option that is only ever yes or no costs a share of
/// one row rather than a row of its own.
fn check_row(ui: &mut Ui, cx: &NodeCtx<'_>, fx: &mut Effects) {
    let (node, def) = (cx.node, cx.node.def);
    // Only for a node that has ticks: almost every node has none, so the common case is a
    // length check rather than a look through every row the node has.
    let checks = def.checks();
    if checks == 0 {
        return;
    }
    let Some(band) = cx.block(canvas::Row::Checks) else {
        return;
    };
    let pad = CONTROL_INSET * cx.zoom();
    let room = (band.width() - pad * 2.0).max(0.0);
    // The keys come from the definition, in its declaration order — the same walk the
    // selects take, and the one place a tick's key and its label live.
    let ticks: Vec<_> = def.options.iter().filter(|o| o.checkbox).collect();
    let captions: Vec<&str> = ticks.iter().map(|o| o.label).collect();
    let shares = crate::ui::check::shares(ui, &captions, room, cx.zoom());
    let mut x = band.min.x + pad;
    for (check, share) in ticks.into_iter().zip(shares) {
        let slot = Rect::from_min_size(Pos2::new(x, band.min.y), vec2(share, band.height()));
        x += share;
        // What the box shows is what layout believes, down to a value that is not there: the
        // definition's own default, which is the value layout would have used.
        let on = canvas::tick(node, check.key).unwrap_or(check.default == nodes::ON);
        let name = cx.control(check.key);
        let response =
            crate::ui::check::tick(ui, slot, check.label, on, name, cx.theme(), cx.zoom());
        if response.clicked() {
            fx.commands.push(crate::command::Command::SetOption {
                node: cx.id,
                key: check.key,
                value: if on { nodes::OFF } else { nodes::ON }.to_string(),
            });
        }
    }
}

/// How wide the grip is, in world units: silvia's note is a `textarea` with `resize: both`,
/// and the corner a browser draws for one is 16 CSS pixels square. The mark inside it is
/// smaller, the way the header's own marks are drawn smaller than their hit boxes.
const GRIP: f32 = 16.0;
/// The mark's own square within the grip: three hairlines across the corner, the outermost
/// of them this long.
const GRIP_MARK: f32 = 11.0;

/// The corner a hand drags a body wider by, in the bottom-right of the box it belongs to.
///
/// **Width only.** silvia's corner takes both axes, because its note holds a fixed box with
/// its own scrollbar; here the height is the text's own — the box grows as it is typed into
/// and shrinks back — so there is no height for a hand to set that the next keystroke would
/// not overrule. What is left is the one thing a person sizes a comment box for: how wide the
/// prose runs.
///
/// The grip is grabbed where it is taken, not where its corner is: the width follows the
/// pointer plus the distance from it to the body's edge at the press, so the body does not
/// jump to the cursor on the first frame of the drag.
fn width_grip(ui: &mut Ui, cx: &NodeCtx<'_>, box_world: Rect) -> Option<crate::command::Command> {
    let (node, id, t, origin, theme) = (cx.node, cx.id, &cx.t, cx.origin, cx.theme());
    let size = GRIP * t.zoom;
    let corner = t.to_screen_rect(origin, box_world).max;
    let rect = Rect::from_min_max(Pos2::new(corner.x - size, corner.y - size), corner);
    let widget = ui.id().with(("width-grip", id));
    let response = ui.interact(rect, widget, Sense::drag());
    let name = cx.control("width");
    let width = canvas::node_width(node);
    crate::ui::accessible(
        &response,
        WidgetType::Other,
        format_args!("{name} {width:.0}"),
    );
    if response.hovered() || response.dragged() {
        ui.ctx()
            .set_cursor_icon(eframe::egui::CursorIcon::ResizeHorizontal);
    }

    // Three hairlines across the corner, shortest at the outside: the mark a browser draws
    // on a resizable box, which is the mark silvia's note wears.
    let ink = if response.hovered() || response.dragged() {
        theme.accent()
    } else {
        theme.border_normal()
    };
    let mark = GRIP_MARK * t.zoom;
    let foot = Pos2::new(corner.x, corner.y);
    for step in 1..=3 {
        let along = mark * (step as f32) / 3.0;
        ui.painter().line_segment(
            [
                Pos2::new(foot.x - along, foot.y),
                Pos2::new(foot.x, foot.y - along),
            ],
            Stroke::new(t.zoom.max(0.5), ink),
        );
    }

    if response.drag_started()
        && let Some(press) = ui.input(|i| i.pointer.press_origin())
    {
        // How far the body's own edge was from the pointer when it closed. Held in egui's
        // temporary store rather than in a field, the way `ui::text`'s draft is: it belongs
        // to this gesture and to nothing that outlives it.
        let at = t.to_world(origin, press).x;
        ui.data_mut(|d| d.insert_temp(widget, node.pos.x + width - at));
    }
    if response.dragged()
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        let grab: f32 = ui.data(|d| d.get_temp(widget)).unwrap_or(0.0);
        let want = (t.to_world(origin, at).x + grab - node.pos.x)
            .clamp(canvas::natural_width(node), canvas::MAX_NODE_WIDTH);
        // Rounded to the point: a width is a number a file carries and a test reads, and a
        // drag that wrote 261.0837 would put the pointer's own sub-pixel noise in both.
        let want = want.round();
        if (want - width).abs() >= 0.5 {
            return Some(crate::command::Command::SetNodeWidth {
                node: id,
                width: Some(want),
            });
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<f32>(widget));
    }
    None
}

/// How a popup stacks its entries: full width, left-aligned.
///
/// `top_down_justified` is what makes a row the width of the list rather than the width of
/// its own text, so the whole line is the target. Without it a menu's hit area is the word,
/// and the space beside it — which reads as part of the same row — does nothing.
fn rows() -> eframe::egui::Layout {
    eframe::egui::Layout::top_down_justified(eframe::egui::Align::LEFT)
}

/// Which end of a value survives when it does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Start,
    End,
}

/// What a select is doing, which is what its border says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectState {
    Idle,
    /// Its list is up, so the border is the primary anchor and the chevron points at it.
    Open,
    /// A cable answers this option. There is no list to open, so there is no chevron: the
    /// row still says what the value is and where it came from, and nothing invites a click.
    Overridden,
}

/// Everything a select draws that is not its geometry.
pub struct Select<'a> {
    pub value: &'a str,
    pub keep: Keep,
    pub state: SelectState,
    /// A fraction painted behind the label, for a select that is also a progress bar while
    /// the node's CPU half works on what it names.
    pub fill: Option<f32>,
}

/// Horizontal padding inside a select, which is `.node-option select`'s own `padding: 4px 6px`.
const SELECT_PAD: f32 = 6.0;
/// The chevron's half-width, and the gap between it and the label.
const CHEVRON: f32 = 3.5;
const CHEVRON_GAP: f32 = 6.0;

/// A select, drawn right-aligned in its row and grown to fit what it holds.
///
/// **It hugs its value.** The design system's select is a native `<select>` with `padding: 4px
/// 6px`, which is a box the width of its longest option and never a fixed one — so `loop`
/// takes the room `loop` needs and a file path takes whatever is left of the row. That is
/// both halves of the same fix: a short value stops reserving a hundred points it does not
/// use, and a long one stops being cut to a length chosen before anyone knew how wide the
/// node was.
///
/// **The chevron is painted, not typed.** A glyph would be at the mercy of the font stack —
/// the one thing every fallback face is allowed to render as `◻` — and this is chrome, not
/// content.
fn select_button(
    ui: &mut Ui,
    band: Rect,
    room: f32,
    name: impl crate::ui::Name,
    select: &Select<'_>,
    theme: &crate::ui::theme::Theme,
    zoom: f32,
) -> Response {
    let font = FontId::monospace(crate::ui::theme::font_size(
        crate::ui::theme::FONT_TINY,
        zoom,
    ));
    let chevron = select.state != SelectState::Overridden;
    let pad = SELECT_PAD * zoom;
    // Everything the box holds besides the label.
    let furniture = pad * 2.0
        + if chevron {
            (CHEVRON * 2.0 + CHEVRON_GAP) * zoom
        } else {
            0.0
        };
    let advance = advance(ui.ctx(), &font);
    let shown = fit(
        select.value,
        advance,
        (room - furniture).max(0.0),
        select.keep,
    );
    let text_w = advance * shown.chars().count() as f32;

    let height = canvas::SELECT_HEIGHT * zoom;
    let width = (text_w + furniture).min(room.max(0.0));
    let rect = Rect::from_min_size(
        Pos2::new(
            band.max.x - 8.0 * zoom - width,
            band.center().y - height * 0.5,
        ),
        vec2(width, height),
    );

    let sense = if chevron {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, ui.id().with(("opt", &name)), sense);

    // The handoff's three states, and they are all the border: `border_normal` at rest,
    // `primary_muted` under the pointer, `primary` with a ring around it while the list is
    // up — which is what a native select's focus ring is. A ring rather than a thicker
    // stroke, so nothing moves when it opens.
    let (border, ground) = match select.state {
        SelectState::Overridden => (theme.border_subtle(), theme.bg_interactive()),
        SelectState::Open => (theme.primary(), theme.bg_interactive()),
        SelectState::Idle if response.hovered() => (theme.primary_muted(), theme.bg_interactive()),
        SelectState::Idle => (theme.border_normal(), theme.bg_interactive()),
    };
    if select.state == SelectState::Open {
        ui.painter().rect_stroke(
            rect.expand(2.0 * zoom),
            CornerRadius::same(crate::ui::theme::RADIUS_MD),
            Stroke::new(2.0 * zoom, theme.primary().gamma_multiply(0.2)),
            eframe::egui::StrokeKind::Outside,
        );
    }
    crate::ui::field(
        ui.painter(),
        rect,
        ground,
        select
            .fill
            .map(|p| (p, theme.primary().gamma_multiply(0.35))),
        border,
    );

    let text = if select.state == SelectState::Overridden {
        theme.readout()
    } else {
        theme.text_primary()
    };
    ui.painter().text(
        Pos2::new(rect.min.x + pad, rect.center().y),
        Align2::LEFT_CENTER,
        &shown,
        font,
        text,
    );
    if chevron {
        let x = rect.max.x - pad - CHEVRON * zoom;
        let y = rect.center().y;
        let (w, h) = (CHEVRON * zoom, CHEVRON * 0.75 * zoom);
        ui.painter().add(eframe::egui::Shape::convex_polygon(
            vec![
                Pos2::new(x - w, y - h * 0.5),
                Pos2::new(x + w, y - h * 0.5),
                Pos2::new(x, y + h),
            ],
            if select.state == SelectState::Open {
                theme.primary()
            } else {
                theme.text_muted()
            },
            Stroke::NONE,
        ));
    }

    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(
            eframe::egui::WidgetType::ComboBox,
            chevron,
            format!("{name} {}", select.value),
        )
    });
    response
}

/// Cut `value` to what fits in `room`, keeping the end that tells values apart.
///
/// Monospace everywhere, so this is division rather than a text layout. An ellipsis marks
/// the cut and costs one of the characters it was going to save.
///
/// Borrowed unless something is actually cut: every port row and every node title fits this
/// through on every frame, and the registry's own labels are `&'static str` that were never
/// going to need an allocation to be drawn.
pub fn fit(value: &str, advance: f32, room: f32, keep: Keep) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    let n = value.chars().count();
    let fits = (room / advance).floor().max(0.0) as usize;
    if n <= fits {
        return Cow::Borrowed(value);
    }
    if fits <= 1 {
        return Cow::Borrowed("…");
    }
    Cow::Owned(match keep {
        Keep::End => format!(
            "…{}",
            value.chars().skip(n - (fits - 1)).collect::<String>()
        ),
        Keep::Start => format!("{}…", value.chars().take(fits - 1).collect::<String>()),
    })
}

/// How tall an open select grows before it scrolls, in points.
const SELECT_HEIGHT: f32 = 640.0;

/// The open list of a select, drawn after every node. Returns the chosen value and whether
/// the user clicked away.
pub fn select_popup(
    ui: &mut Ui,
    at: Pos2,
    width: f32,
    value: &str,
    choices: &'static [(&'static str, &'static str)],
    devices: bool,
    theme: &crate::ui::theme::Theme,
) -> Selected {
    let mut chosen = None;
    let mut look_again = false;
    let id = ui.id().with("optpicker");
    // Whether this is the frame the list opened on, which is the one frame it scrolls the
    // chosen row into view: after that the wheel is the hand's.
    let frame = ui.ctx().cumulative_frame_nr();
    let opened = ui.ctx().data_mut(|d| {
        let last = d.get_temp::<u64>(id);
        d.insert_temp(id, frame);
        last != Some(frame.wrapping_sub(1))
    });
    let shown = crate::ui::popup::Popup::new(id, at).show(ui.ctx(), theme, |ui| {
        // Never narrower than the select it hangs from.
        ui.set_min_width(width - 2.0 * ui.style().spacing.window_margin.left as f32);
        // One line a choice, and the list as wide as its widest. Wrapped, a
        // label that exactly filled its row broke in two the moment it was
        // hovered, because the hover's outline takes a point of the row's width.
        ui.style_mut().wrap_mode = Some(eframe::egui::TextWrapMode::Extend);
        // Justified, so a row is the width of the list and not the width of its
        // word: the target is the line you are reading, which is what every menu
        // in the app already behaves like.
        // Scrolled past a screenful: a font menu is every family the machine
        // has, which is a hundred and more. The floor is the same height as the
        // ceiling because a scroll area is otherwise only as tall as the room
        // between the select and the window's foot, which is two rows under a
        // select near the bottom; with it, the list keeps its height and
        // `constrain` lifts the popup onto the screen instead.
        let tall = SELECT_HEIGHT.min(ui.ctx().content_rect().height() - 32.0);
        eframe::egui::ScrollArea::vertical()
            .max_height(tall)
            .min_scrolled_height(tall)
            .show(ui, |ui| {
                ui.with_layout(rows(), |ui| {
                    for (v, name) in choices {
                        let row = ui.selectable_label(*v == value, *name);
                        if opened && *v == value {
                            row.scroll_to_me(Some(eframe::egui::Align::Center));
                        }
                        if row.clicked() {
                            chosen = Some((*v).to_string());
                        }
                    }
                    // Under the list, as in the Main Input's own device menus.
                    if devices {
                        ui.separator();
                        look_again = ui.button("Look for devices again").clicked();
                    }
                });
            });
    });
    let dismissed = chosen.is_some() || look_again || shown.clicked_away;
    Selected {
        chosen,
        look_again,
        dismissed,
    }
}

/// What one frame of an option's list did.
pub struct Selected {
    /// The value of the choice clicked.
    pub chosen: Option<String>,
    /// *Look for devices again* was clicked.
    pub look_again: bool,
    /// The list is to close: something was clicked, or a click landed outside it.
    pub dismissed: bool,
}

/// What an asset picker's click asked for.
pub enum Pick {
    /// One of the project's own files, as the reference an option holds.
    Asset(String),
    /// The file dialog, for something the project does not have yet.
    File,
}

/// What one frame of the asset picker did.
pub struct Picked {
    pub chosen: Option<Pick>,
    /// A click landed outside it.
    pub dismissed: bool,
}

/// One of the project's files, as the picker draws it.
pub struct AssetChoice<'a> {
    pub info: &'a crate::project::AssetInfo,
    /// Its poster, once one has been decoded. `None` while it is being made, or where it
    /// could not be.
    pub picture: Option<&'a eframe::egui::TextureHandle>,
}

/// How big a card's picture is in the picker: half the project tab's, because this hangs off
/// a node rather than filling a page, and the job here is *which clip is this* rather than
/// *what is in it*.
const CARD_THUMB: eframe::egui::Vec2 = eframe::egui::vec2(80.0, 45.0);

/// The picker behind a file button: the project's own media of this kind, and the dialog.
///
/// The media comes first because it is the common case — a rig is built out of clips already
/// imported, and reaching a file that is already in `assets/` through a file dialog means
/// navigating to a folder the project owns. Importing something new is one entry at the
/// bottom, where a menu's escape hatch belongs.
///
/// **A card, not a line of text.** A clip is recognized by what it looks like, and a folder
/// of episodes named by their numbering is exactly the case a list of names cannot answer.
/// The picture is the poster `App` decodes out of the file; until one exists the card shows
/// the same icon its card on the project tab does, so the two read as one thing.
pub fn asset_popup(
    ui: &mut Ui,
    at: Pos2,
    width: f32,
    value: &str,
    assets: &[AssetChoice<'_>],
    theme: &crate::ui::theme::Theme,
) -> Picked {
    let mut chosen = None;
    let shown =
        crate::ui::popup::Popup::new(ui.id().with("assetpicker"), at).show(ui.ctx(), theme, |ui| {
            let margin = 2.0 * ui.style().spacing.window_margin.left as f32;
            ui.set_min_width((width - margin).max(CARD_THUMB.x * 3.0));
            ui.set_max_width(320.0);
            eframe::egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for asset in assets {
                        if card(ui, asset, asset.info.reference == value, theme).clicked() {
                            chosen = Some(Pick::Asset(asset.info.reference.clone()));
                        }
                    }
                });
            ui.separator();
            ui.with_layout(rows(), |ui| {
                if ui.selectable_label(false, "Import a file…").clicked() {
                    chosen = Some(Pick::File);
                }
            });
        });
    Picked {
        dismissed: shown.clicked_away,
        chosen,
    }
}

/// What one frame of the `!` popup did.
pub struct LivePicked {
    /// Which source was clicked, by index into the list drawn.
    pub chosen: Option<usize>,
    /// A click landed outside it.
    pub dismissed: bool,
}

/// The popup under an Output's `!`: each source upstream that a render cannot step, as a row
/// that goes there — the node on its workspace, by the tag's own navigation, or the Main
/// Input panel when the source is the panel's.
pub fn live_popup(
    ui: &mut Ui,
    at: Pos2,
    sources: &[crate::ui::LiveSource],
    theme: &crate::ui::theme::Theme,
) -> LivePicked {
    let mut chosen = None;
    let shown =
        crate::ui::popup::Popup::new(ui.id().with("livepicker"), at).show(ui.ctx(), theme, |ui| {
            ui.set_min_width(220.0);
            ui.label(
                eframe::egui::RichText::new("A render reads these as they are now:")
                    .color(theme.text_muted())
                    .font(FontId::monospace(crate::ui::theme::FONT_TINY)),
            );
            ui.with_layout(rows(), |ui| {
                for (i, source) in sources.iter().enumerate() {
                    if ui
                        .selectable_label(false, &source.label)
                        .on_hover_text("Go there")
                        .clicked()
                    {
                        chosen = Some(i);
                    }
                }
            });
        });
    LivePicked {
        chosen,
        dismissed: shown.clicked_away,
    }
}

/// One asset's card: its poster or its icon, its name and its size.
fn card(
    ui: &mut Ui,
    asset: &AssetChoice<'_>,
    picked: bool,
    theme: &crate::ui::theme::Theme,
) -> Response {
    let frame = eframe::egui::Frame::new()
        .fill(if picked {
            theme.bg_tertiary()
        } else {
            theme.bg_sunken()
        })
        .stroke(Stroke::new(
            1.0,
            if picked {
                theme.primary()
            } else {
                theme.border_subtle()
            },
        ))
        .corner_radius(crate::ui::theme::RADIUS_SM)
        .inner_margin(4);
    let response = ui
        .scope_builder(eframe::egui::UiBuilder::new().sense(Sense::click()), |ui| {
            frame.show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    crate::ui::project::picture(
                        ui,
                        theme,
                        asset.picture,
                        CARD_THUMB,
                        crate::ui::project::icon_for(&asset.info.name),
                    );
                    ui.vertical(|ui| {
                        // Neither label may select its text, or it takes the press meant for
                        // the card under it. That is `theme::apply`'s doing, for every label
                        // at once, and it is what keeps a click on this name a click on this
                        // card.
                        ui.add(
                            eframe::egui::Label::new(asset.info.name.as_str())
                                .wrap_mode(eframe::egui::TextWrapMode::Truncate),
                        );
                        ui.label(
                            eframe::egui::RichText::new(crate::ui::project::size_of(
                                asset.info.size,
                            ))
                            .color(theme.text_muted()),
                        );
                    });
                });
            });
        })
        .response;
    // The whole card is the target, so it is what carries the name — the labels inside it
    // are decoration, and a click on one has to be a click on the card.
    let name = format!("asset {}", asset.info.name);
    response.widget_info(|| {
        eframe::egui::WidgetInfo::selected(
            eframe::egui::WidgetType::Button,
            true,
            picked,
            name.clone(),
        )
    });
    response.clone().on_hover_text(asset.info.name.as_str());
    response
}

/// A cable's far end, where the node it comes from is not on this workspace.
///
/// The pill stands in for the cable that cannot be drawn: the source node's icon and the
/// workspace it is on, in the port's own color, dimmed while that workspace has no tab.
pub struct Tag<'a> {
    /// The source node's `NodeDef::icon`, drawn `theme::ICON_BUMP` larger than the name
    /// beside it — the pill grows to hold it rather than clipping it.
    pub icon: &'a str,
    /// The workspace's name.
    pub label: &'a str,
    /// What the accessibility tree carries:
    /// `tag from {slug}{id}.{key} on {workspace name}`.
    pub name: &'a str,
    /// The cable's type, which is where the color comes from.
    pub ty: PortType,
    /// The workspace named has no tab.
    pub closed: bool,
}

/// Draw one tag, right-aligned to `right`, and return where it landed.
///
/// The rect comes back so a second tag on the same port stacks leftwards from the first: an
/// action input takes many sources, and each one that is elsewhere gets a pill.
pub fn tag(
    ui: &mut Ui,
    right: Pos2,
    tag: &Tag<'_>,
    theme: &crate::ui::theme::Theme,
    zoom: f32,
) -> (Rect, Response) {
    use crate::ui::theme;

    let font = FontId::monospace(theme::font_size(theme::FONT_TINY, zoom));
    let dim = |c: eframe::egui::Color32| {
        if tag.closed {
            c.gamma_multiply(theme::TAG_CLOSED)
        } else {
            c
        }
    };
    // Two galleys, so the icon can be the larger of the pair. The name is wrapped in what is
    // left of the pill's width once the icon has had its share.
    let icon = ui.painter().layout_no_wrap(
        tag.icon.to_string(),
        theme::icon_font(theme::FONT_TINY, zoom),
        dim(theme.tag_text(tag.ty)),
    );
    let gap = if tag.icon.is_empty() { 0.0 } else { 3.0 * zoom };
    let galley = ui.painter().layout(
        tag.label.to_string(),
        font,
        dim(theme.tag_text(tag.ty)),
        (theme::TAG_MAX_WIDTH * zoom - theme::TAG_PAD * 2.0 * zoom - icon.size().x - gap).max(1.0),
    );
    // The pill takes its height from what is in it, floored at the row-sized minimum: an
    // icon four points larger than the name is also taller than it, and a fixed height would
    // clip it.
    let size = vec2(
        icon.size().x + gap + galley.size().x + theme::TAG_PAD * 2.0 * zoom,
        (theme::TAG_HEIGHT * zoom).max(icon.size().y.max(galley.size().y) + 2.0 * zoom),
    );
    let rect = Rect::from_min_size(Pos2::new(right.x - size.x, right.y - size.y * 0.5), size);

    let radius = CornerRadius::same((f32::from(theme::RADIUS_SM) * zoom).round() as u8);
    ui.painter()
        .rect_filled(rect, radius, dim(theme.tag_fill(tag.ty)));
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, dim(theme.tag_border(tag.ty))),
        eframe::egui::StrokeKind::Inside,
    );
    // Icon then name, the pair centerd in the pill.
    let left = rect.center().x - (icon.size().x + gap + galley.size().x) * 0.5;
    ui.painter().galley(
        Pos2::new(left, rect.center().y - icon.size().y * 0.5),
        icon.clone(),
        dim(theme.tag_text(tag.ty)),
    );
    ui.painter().galley(
        Pos2::new(
            left + icon.size().x + gap,
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        dim(theme.tag_text(tag.ty)),
    );

    // The widget carries the name; the geometry carries the click — the same split cables
    // are drawn under.
    let response = ui.interact(rect, ui.id().with(("tag", tag.name)), Sense::click());
    crate::ui::accessible(&response, WidgetType::Button, tag.name);
    response.clone().on_hover_text(if tag.closed {
        format!("{} — closed; click to open it", tag.name)
    } else {
        format!("{} — click to go there", tag.name)
    });
    (rect, response)
}

/// How far outside a port's own radius its convertible ring sits, in world units.
const CONVERTIBLE_GAP: f32 = 3.0;
/// How far outside it the cable-color outline sits, and how thick that ring is.
///
/// Inside `CONVERTIBLE_GAP`, so the two marks read as two rings rather than one thick one on
/// the rare frame a port is both connected and convertible — which is only ever while a
/// cable is in flight.
const OUTLINE_GAP: f32 = 1.5;
const OUTLINE_WIDTH: f32 = 1.5;
/// Dashes in that ring. A dozen reads as dotted at every zoom a port is legible at, and each
/// is two points, so the whole mark is 24 vertices on the frames a drag is in flight.
const CONVERTIBLE_DASHES: usize = 12;

/// How one port dot is drawn this frame: what the canvas worked out about it.
// Four independent questions about the same dot — dimmed, convertible, connected, lit — and
// no two of them imply each other, so a state enum would need a variant per combination.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default)]
pub struct PortLook {
    /// A cable is in flight that cannot land here, even through a node.
    pub dimmed: bool,
    /// A cable is in flight that cannot land here but a node between the two could carry.
    pub convertible: bool,
    /// An input with something plugged into it.
    pub connected: bool,
    /// This port is on a cable that reaches the port under the pointer, or is that port.
    pub lit: bool,
    /// The color of the cable on this port, under `phi_cables`: an outline just outside the
    /// dot, so a port says which wire leaves it without the wire having to be followed.
    /// `None` when the preference is off or the port carries nothing.
    pub outline: Option<Color32>,
    /// What an output published this frame, as a number — the `f64` as it was published
    /// ([`crate::synth::Uniforms::get`]) — or as a color.
    pub published: Option<f64>,
    pub published_color: Option<[f32; 4]>,
    /// How brightly this port is throbbing because it just fired: 1 on the frame it fired and
    /// decaying to 0. Zero on every port that is not an action, and on every frame nothing
    /// has happened on.
    pub fire: f32,
}

/// Draw one port dot and return its response.
pub fn port(
    ui: &mut Ui,
    slot: &PortSlot,
    node: &Node,
    t: &Transform,
    look: &PortLook,
    theme: &crate::ui::theme::Theme,
) -> Response {
    let PortLook {
        dimmed,
        convertible,
        connected,
        lit,
        outline,
        published,
        published_color,
        fire,
    } = *look;
    let r = canvas::PORT_RADIUS * t.zoom;
    let hit = port_hit(slot.center, t);
    // Built on demand. Every port of every drawn node came through here building four
    // Strings a frame, for a label egui reads only when the pointer is on the port or an
    // accessibility client is attached.
    //
    // A published value goes on the end of the name, where the s-number carries its own:
    // the tree is the agent's API, so a number a person can read off the row is a number a
    // test and an agent can read too.
    let describe = || {
        let kind = if slot.is_input { "input" } else { "output" };
        let mut text = format!(
            "{}{}.{} ({} {kind})",
            node.def.slug,
            slot.port.node,
            slot.port.key,
            slot.ty.name()
        );
        if let Some(value) = published {
            let mut number = String::new();
            let integral = node.def.output(slot.port.key).is_some_and(|o| o.integral);
            readout(&mut number, value, integral);
            text.push(' ');
            text.push_str(&number);
        }
        // A uniform color's reading is its hex, which is how a color is written down — and
        // the same string the picker's own field takes.
        if let Some(value) = published_color {
            text.push(' ');
            text.push_str(&crate::ui::color::to_hex(value));
        }
        if convertible {
            text.push_str(" (convertible)");
        }
        text
    };

    let color = theme.port(slot.ty);
    let color = if dimmed {
        color.gamma_multiply(0.25)
    } else if lit {
        // The same brightening the cable between them takes, so a port and its wire light
        // as one thing rather than two that happen to agree.
        color.gamma_multiply(1.4)
    } else {
        color
    };
    // **The throb**: an action fired here, and the dot says so for a sixth of a second. It is
    // over the hover brightening rather than instead of it, because the two answer different
    // questions — *this is the port you are pointing at* and *this just happened* — and a
    // firing on a port under the pointer must not be the one that cannot be seen.
    let color = if fire > 0.0 {
        color.lerp_to_gamma(Color32::WHITE, fire * crate::ui::FIRE_LIFT)
    } else {
        color
    };
    let painter = ui.painter();

    // The third state a port has while a cable is in flight: the cable cannot land here, but
    // a node between the two can carry it. A ring of dashes outside the dot, in the port's
    // own hue — egui has no dashed circle, so it is a dozen short segments on one radius.
    if convertible {
        let ring = r + CONVERTIBLE_GAP * t.zoom;
        let step = std::f32::consts::TAU / CONVERTIBLE_DASHES as f32;
        let half = step * 0.25;
        let on = |angle: f32| slot.center + vec2(angle.cos(), angle.sin()) * ring;
        for i in 0..CONVERTIBLE_DASHES {
            let angle = step * i as f32;
            painter.line_segment(
                [on(angle - half), on(angle + half)],
                Stroke::new(1.0 * t.zoom, color),
            );
        }
    }

    // Action ports are rounded squares and uniforms diamonds, not circles: shape carries the
    // type as well as color, so the distinction survives a colorblind viewer and a grayscale
    // theme. A data input with nothing in it is a hole, not a gap: the whole interior is the
    // port's own hue sunk to a shadow, not empty canvas showing through a ring with a second,
    // smaller dot floating inside it. An action port is solid either way.
    let dot = |fill, stroke| port_shape(slot, t.zoom, 0.0, fill, stroke);
    let ring = Stroke::new(1.5 * t.zoom, color);
    if connected || !slot.is_input || slot.ty == PortType::Action {
        painter.add(dot(color, Stroke::NONE));
    } else if slot.ty.is_uniform() {
        painter.add(dot(theme.port_hole(slot.ty), ring));
    } else {
        painter.add(dot(theme.port_hole(slot.ty), Stroke::NONE));
        painter.add(dot(Color32::TRANSPARENT, ring));
    }

    // **The cable's own color, ringed just outside the dot.** Outside rather than on it, so
    // the port keeps saying what it *is* — the hue and the shape are this editor's type
    // system drawn — while the ring says what it is wired to. silvia puts a 3px border on
    // the element, which is the same ring: CSS grows a border outward too.
    //
    // It follows the shape rather than always being a circle, for the reason the shapes
    // exist: an action port is a square and a uniform is a diamond so the type survives a
    // colorblind viewer, and a round ring around a square would undo half of that.
    if let Some(ink) = outline {
        let stroke = Stroke::new(OUTLINE_WIDTH * t.zoom, ink);
        painter.add(port_shape(
            slot,
            t.zoom,
            OUTLINE_GAP,
            Color32::TRANSPARENT,
            stroke,
        ));
    }

    let response = ui.interact(
        hit,
        ui.id()
            .with(("port", slot.port.node, slot.port.key, slot.is_input)),
        Sense::click_and_drag(),
    );
    if response.hovered() {
        // `describe()` is also the accessibility name, unchanged, so a locator built on it
        // keeps working; the second line is only for the hover, since the row now paints the
        // label and the key it patches by is otherwise nowhere on the node.
        response
            .clone()
            .on_hover_text(format!("{}\nkey: {}", describe(), slot.port.key));
    }
    // Labeled so the agent clicks `checkerboard1.frequency (varying number input)` by name
    // rather than guessing coordinates. Layers 2 and 4 both read this.
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Other, true, describe())
    });
    response
}

/// A port's own shape on screen, `gap` world units outside the dot: a rounded square for an
/// action, a diamond for a uniform and a circle for everything else. The dot is a gap of zero
/// and the cable-color ring is `OUTLINE_GAP`, so the two cannot disagree about the shape.
fn port_shape(slot: &PortSlot, zoom: f32, gap: f32, fill: Color32, stroke: Stroke) -> Shape {
    let c = slot.center;
    if slot.ty == PortType::Action {
        let side = (canvas::ACTION_PORT_RADIUS + gap) * 2.0 * zoom;
        let square = Rect::from_center_size(c, vec2(side, side));
        let corner = CornerRadius::same((side * 0.25) as u8);
        return Shape::Rect(RectShape::new(
            square,
            corner,
            fill,
            stroke,
            StrokeKind::Middle,
        ));
    }
    let r = canvas::PORT_RADIUS * zoom + gap * zoom;
    if slot.ty.is_uniform() {
        let points = vec![
            Pos2::new(c.x, c.y - r),
            Pos2::new(c.x + r, c.y),
            Pos2::new(c.x, c.y + r),
            Pos2::new(c.x - r, c.y),
        ];
        return Shape::Path(PathShape {
            points,
            closed: true,
            fill,
            stroke: stroke.into(),
        });
    }
    Shape::Circle(CircleShape {
        center: c,
        radius: r,
        fill,
        stroke,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use eframe::egui::vec2;

    /// A select that lists the machine's devices ends in *Look for devices again*, and a click
    /// on it asks again rather than choosing anything; a select of the node's own choices does
    /// not offer it.
    #[test]
    fn a_device_select_offers_to_look_for_devices_again() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        use std::sync::atomic::{AtomicBool, Ordering};
        assert!(
            crate::nodes::find("camera")
                .and_then(|d| d.option("device"))
                .is_some_and(|o| o.devices),
            "the Camera node's Device lists devices"
        );
        let theme = crate::ui::theme::Theme::default();
        let asked = AtomicBool::new(false);
        let chose = AtomicBool::new(false);
        let mut h = Harness::new_ui(|ui| {
            let picked = select_popup(
                ui,
                Pos2::new(10.0, 10.0),
                160.0,
                "auto",
                &[("auto", "Auto"), ("test", "Test pattern")],
                true,
                &theme,
            );
            asked.fetch_or(picked.look_again, Ordering::SeqCst);
            chose.fetch_or(picked.chosen.is_some(), Ordering::SeqCst);
        });
        h.run_steps(2);
        h.get_by_label("Look for devices again").click();
        h.run_steps(2);
        assert!(asked.load(Ordering::SeqCst), "the click asks again");
        assert!(!chose.load(Ordering::SeqCst), "and chooses nothing");
        drop(h);

        let mut h = Harness::new_ui(|ui| {
            select_popup(
                ui,
                Pos2::new(10.0, 10.0),
                160.0,
                "a",
                &[("a", "A")],
                false,
                &theme,
            );
        });
        h.run_steps(2);
        assert!(h.query_by_label("Look for devices again").is_none());
    }

    /// The value has to survive the cut in the direction that tells values apart, and the
    /// result has to actually fit — an ellipsis that pushes it back over the edge is not a fit.
    #[test]
    fn a_value_too_wide_is_cut_from_the_end_that_says_least() {
        // One point a character, so `room` is a character count.
        let fits = |room: f32, keep| fit("gumbasia.webm", 1.0, room, keep);
        assert_eq!(fits(13.0, Keep::End), "gumbasia.webm", "it fits whole");
        assert_eq!(fits(99.0, Keep::Start), "gumbasia.webm");
        // A file name keeps its tail: the extension is what says what it is.
        assert_eq!(fits(6.0, Keep::End), "….webm");
        // A choice keeps its head, which is where a menu's words differ.
        assert_eq!(fits(6.0, Keep::Start), "gumba…");
        for room in [0.0, 1.0, 2.0] {
            let cut = fits(room, Keep::End);
            assert!(
                cut.chars().count() <= room.max(1.0) as usize,
                "{room} points held {cut:?}"
            );
        }
    }

    #[test]
    fn a_value_that_fits_is_left_alone() {
        assert_eq!(fit("all", 1.0, 3.0, Keep::Start), "all");
        assert_eq!(fit("", 1.0, 0.0, Keep::End), "");
    }

    /// One Output node at the world origin, laid out alone: its body and the band its render
    /// is blitted into, in world units.
    fn output() -> (Rect, Rect) {
        let mut g = Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "output", Pos2::ZERO).expect("in registry");
        let laid = canvas::Layouts::one(&g, id, &[]);
        let l = laid.find(id).expect("just added");
        (l.rect, l.region(canvas::Region::Declared(0)))
    }

    /// The picture's rect is the node's, not the view's.
    ///
    /// A node hanging off the edge of the canvas keeps its size, and the callback's clip rect
    /// is what cuts the picture off — as it cuts off the body around it. A rect fitted to the
    /// visible part instead shrinks the render of an Output at the edge.
    #[test]
    fn a_node_off_the_edge_keeps_its_pictures_size() {
        let (_, picture) = output();
        let origin = Pos2::new(40.0, 60.0);
        let here = Transform::default();
        let size = here.to_screen_rect(origin, picture).size();

        for pan in [
            vec2(-4000.0, 0.0),
            vec2(4000.0, 0.0),
            vec2(0.0, -4000.0),
            vec2(0.0, 4000.0),
        ] {
            let away = Transform { pan, ..here };
            assert_eq!(
                away.to_screen_rect(origin, picture).size(),
                size,
                "the picture was resized by a pan of {pan:?}"
            );
        }
    }

    /// The picture is the slot: flush with the body's sides and bottom at every zoom, since
    /// the renderer rounds its bottom corners itself rather than the picture being held off
    /// the arc.
    #[test]
    fn the_picture_is_flush_with_the_body() {
        let (body, picture) = output();
        let origin = Pos2::ZERO;
        for zoom in [0.4_f32, 0.5, 0.75, 1.0, 1.5, 2.5] {
            let t = Transform {
                zoom,
                ..Transform::default()
            };
            let body = t.to_screen_rect(origin, body);
            let picture = t.to_screen_rect(origin, picture);
            assert!(picture.height() > 0.0, "an Output draws its render");
            assert_eq!(picture.min.x, body.min.x, "left, at zoom {zoom}");
            assert_eq!(picture.max.x, body.max.x, "right, at zoom {zoom}");
            assert_eq!(picture.max.y, body.max.y, "bottom, at zoom {zoom}");
        }
    }

    /// No port row's label runs into its control, its dot or (on an output) a declared range,
    /// at the width the node actually draws at zoom 1.
    ///
    /// Calls `label_room`, the same rule `body` paints by, so a margin that moves moves for
    /// both. `NodeDef::width` is the fix: a node this catches gets one, rather than the
    /// default growing for everyone.
    ///
    /// Measured against the row's own **block**, which is `canvas::ROW_BLOCK_INSET` narrower
    /// than the body and is what `body` bounds a label with. Against the body it read six
    /// points wider than the screen ever gives a label, and passed ten registry labels —
    /// `star`'s `Inner Radius` among them — that lost their last character to `fit`.
    #[test]
    fn no_registry_label_clips_its_declared_width() {
        let ctx = eframe::egui::Context::default();
        // Fonts are not available until the first frame runs; a bare `set_fonts` is not
        // enough to ask `glyph_width` afterwards.
        let mut output = ctx.run_ui(eframe::egui::RawInput::default(), |ctx| {
            crate::ui::theme::apply(ctx, &crate::ui::theme::Theme::default());
        });
        output.textures_delta.clear();
        let small = FontId::monospace(crate::ui::theme::font_size(
            crate::ui::theme::FONT_TINY,
            1.0,
        ));
        let advance = advance(&ctx, &small);
        let capacity = |room: f32| (room / advance).floor() as usize;

        let mut failures = Vec::new();
        for def in crate::nodes::REGISTRY {
            let mut g = Graph::new();
            let id = crate::nodes::add_to_graph(&mut g, def.slug, Pos2::ZERO)
                .expect("the registry's own slug");
            let node = g.get(id).unwrap();
            // The label is painted inside the row's block, not the body: the same width the
            // painter hands `label_room` at zoom 1.
            let width = canvas::node_width(node) - canvas::ROW_BLOCK_INSET;

            for (index, input) in def.inputs.iter().enumerate() {
                // An action input with a press button captions the button that is drawn over
                // this row instead; the row label is never painted under it.
                if matches!(input.control, crate::nodes::Control::Press) {
                    continue;
                }
                let room = label_room(
                    width,
                    false,
                    node.controls.contains_key(input.key) || canvas::speed_tall(node, index),
                    0.0,
                    1.0,
                );
                if input.label.chars().count() > capacity(room) {
                    failures.push(format!(
                        "{} input {:?} at width {width}",
                        def.slug, input.label
                    ));
                }
            }

            for output in def.outputs {
                // A declared range is static text drawn ahead of the label, on the same slot
                // a live uniform number readout would take — the live number itself is not
                // part of
                // the definition, so it plays no part in what this checks.
                let taken = output.range.map_or(0.0, |range| {
                    range.chars().count() as f32 * advance + READOUT_GAP
                });
                let room = label_room(width, true, false, taken, 1.0);
                if output.label.chars().count() > capacity(room) {
                    failures.push(format!(
                        "{} output {:?} at width {width}",
                        def.slug, output.label
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "these rows clip their node's declared width — give the definition a wider \
             `NodeDef::width`:\n  {}",
            failures.join("\n  ")
        );
    }

    /// A color thumbnail's cell is the packed premultiplied color, and over black that is its
    /// rgb as it stands: half-transparent red is half red, transparent black is black.
    #[test]
    fn a_color_thumbnail_cell_is_its_premultiplied_rgb_over_black() {
        let word = |rgba: [u8; 4]| u32::from_le_bytes(rgba);
        assert_eq!(
            thumb_color(word([128, 0, 0, 128])),
            Color32::from_rgb(128, 0, 0)
        );
        assert_eq!(thumb_color(word([0, 0, 0, 0])), Color32::BLACK);
        assert_eq!(thumb_color(word([255, 255, 255, 255])), Color32::WHITE);
        assert_eq!(
            thumb_color(word([10, 20, 30, 64])),
            Color32::from_rgb(10, 20, 30)
        );
    }
}
