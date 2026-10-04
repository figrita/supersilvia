// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pan/zoom transform and node geometry. World units are what node positions are stored in;
//! screen units are what egui draws with.
//!
//! Layout reads a node's kind through `Node::def` — its rows, its regions, its width — and the
//! one thing it cannot know from the document, how tall a value's text came out, from
//! [`Measured`], which is the canvas's own view state. Every function that answers a height
//! takes the node's measured heights beside the node, so a frame and a hit test that ask
//! about the same node ask about the same body.
//!
//! A frame asks once: [`Layouts`] lays every node on the workspace out at the top of the pass,
//! and the canvas reads each node's [`NodeLayout`]. Whatever needs one node outside a frame
//! asks [`Layouts::one`], which is the same walk. See docs/ui.md, "Layout".

use crate::graph::{Graph, Node, NodeId};
use eframe::egui::{Pos2, Rect, Vec2, vec2};
use std::collections::HashMap;

/// Node body width in world units. The design system sets `min-width: 200px`, and an
/// Output grows to 240 wide to hold its render — the height of that render is not fixed,
/// see `widgets::picture::RENDER`, since it follows the Output's own `resolution` option.
pub const NODE_WIDTH: f32 = 200.0;
pub const OUTPUT_NODE_WIDTH: f32 = 240.0;
/// Header height: the taller of a 20px icon padded 0.25rem top and bottom, and a title padded
/// 0.5rem top and bottom — both about 26px, which is silvia's own header.
pub const HEADER_HEIGHT: f32 = 26.0;
/// Where the header's kind icon starts, from the body's left edge.
///
/// silvia's `.node-icon` is a 20px box at `margin-left: 0.25rem` holding a 16px glyph, so its
/// ink lands about five points in. This is the text origin rather than a box, so it is the
/// number that puts the ink the same distance from the left edge as the glyph's own cap sits
/// from the top of the header — six points, measured off a 6x crop rather than assumed.
pub const ICON_INSET: f32 = 6.0;

/// The header's `?` and `✕`: silvia's `.node-tooltip`/`.node-close`, 20px square.
pub const MARK_SIZE: f32 = 20.0;
/// `✕`'s own inset from the header's right edge — silvia's `.node-close { right: 0.25rem }`.
pub const MARK_MARGIN: f32 = 3.0;
/// Clear space between the `?` and `✕` marks — silvia's two 20px boxes sit one point apart.
pub const MARK_GAP: f32 = 1.0;
/// Vertical pitch of one port row: 0.4rem padding either side of a 14px port.
pub const PORT_PITCH: f32 = 24.0;
/// Port radius. Data ports are 1.2rem across, action ports 1rem and rounded-square.
pub const PORT_RADIUS: f32 = 7.2;
pub const ACTION_PORT_RADIUS: f32 = 6.0;
/// How much larger than the dot the pointer's target for it is. A port is easier to hit than
/// to miss: 3x `PORT_RADIUS` is a 21.6-point square against a 24-point row pitch, so the rows
/// above and below stay reachable. `node_widget::port_hit` is the square it makes, and it is
/// the one shape a port answers in: the hover, the click and a cable let go.
pub const PORT_HIT_SCALE: f32 = 3.0;
/// Spacing of the canvas dot grid.
pub const GRID_PITCH: f32 = 24.0;

pub const MIN_ZOOM: f32 = 0.25;
pub const MAX_ZOOM: f32 = 3.0;

/// The zoom at and above which a node's rows carry their detail: the controls, the port
/// labels and the uniform number readouts, which arrive and leave together. Below it a node
/// is its header, its bands and its ports, and above `TITLE_ZOOM` it still has its title.
pub const DETAIL_ZOOM: f32 = 0.5;

/// The zoom above which a node has a title, and with it the `?` and the `✕` beside it.
pub const TITLE_ZOOM: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Transform {
    /// The view as the project file carries it. Session state: switching to a tab brings
    /// back the view you left, and none of it is an edit.
    ///
    pub fn saved(self) -> crate::project::View {
        crate::project::View {
            pan: [self.pan.x, self.pan.y],
            zoom: self.zoom,
        }
    }

    /// A saved view, back as a transform.
    pub fn restored(view: crate::project::View) -> Self {
        Self {
            pan: Vec2::new(view.pan[0], view.pan[1]),
            zoom: view.zoom,
        }
    }

    pub fn to_screen(&self, origin: Pos2, world: Pos2) -> Pos2 {
        origin + (world.to_vec2() * self.zoom + self.pan)
    }

    /// A world rect on screen. The transform is a uniform scale and a translation, so both
    /// corners move the same way and the rect stays axis-aligned.
    pub fn to_screen_rect(&self, origin: Pos2, world: Rect) -> Rect {
        Rect::from_min_max(
            self.to_screen(origin, world.min),
            self.to_screen(origin, world.max),
        )
    }

    pub fn to_world(&self, origin: Pos2, screen: Pos2) -> Pos2 {
        (((screen - origin) - self.pan) / self.zoom).to_pos2()
    }

    /// Zoom about a fixed screen point, so the thing under the cursor stays put.
    pub fn zoom_about(&mut self, origin: Pos2, anchor: Pos2, factor: f32) {
        let before = self.to_world(origin, anchor);
        self.zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let after = self.to_world(origin, anchor);
        self.pan += (after - before) * self.zoom;
    }
}

/// How far below the deeper node a backward cable passes, in world units.
pub const CABLE_CLEARANCE: f32 = 24.0;

/// A row holding an s-number or s-color: 25px control plus 0.4rem either side.
pub const CONTROL_ROW_PITCH: f32 = 34.0;

/// The rows of an Output's Render section, under the **Render** heading that folds them: its
/// three selects, then one row per number, then the button.
pub const RENDER_SELECTS: usize = crate::nodes::output::RENDER_SELECTS.len();
pub const RENDER_CONTROLS: usize = crate::nodes::output::RENDER_CONTROLS.len();
pub const RENDER_ROWS: usize = RENDER_SELECTS + RENDER_CONTROLS + 1;

/// How tall the status line is. A line of `FONT_TINY` and the air around it, well under
/// `OPTION_ROW_PITCH`: it is a line of monospace, not a control, and giving it a control's
/// height would put twenty-five points of nothing on every Output in the graph.
pub const READOUT_ROW_PITCH: f32 = 15.0;

/// Does this node carry the Render heading — the bar and disclosure triangle over the render
/// rows, drawn open or closed on every Output and on nothing else.
pub fn render_heading(node: &Node) -> bool {
    node.def.is_output && !node.collapsed
}

/// Is the Output's render section open: its heading's option, which reads absent as closed.
pub fn render_shown(node: &Node) -> bool {
    node.def.is_output && tick(node, crate::nodes::output::OFFLINE).unwrap_or(false)
}

/// Where a moving node's Timing heading sits: the index of its first time row, on a node that
/// keeps time (`nodes::timing`). `None` elsewhere, and on a collapsed node.
pub fn timing_heading(node: &Node) -> Option<usize> {
    if node.collapsed || node.def.timing.is_none() {
        return None;
    }
    node.inputs
        .iter()
        .position(|p| crate::nodes::is_time_row(p.key))
}

/// How much further in a row under the Timing heading starts its label than a row outside
/// it: a few points, so the rows read as the heading's own rather than as the node's next.
pub const TIMING_INDENT: f32 = 6.0;

/// The one gap a heading keeps: above its bar, under a closed bar, and under the last row the
/// Timing heading folds, which parts those rows from the node's next input as the open bar
/// touching its first row joins them to it. The bar itself is the same height open or closed,
/// so a fold moves what is under it and never the bar.
pub const HEADING_GAP: f32 = 2.0;

/// Whether a heading row is open, or `None` for a row that is not a heading.
fn heading_open(node: &Node, row: Row) -> Option<bool> {
    match row {
        Row::TimingHeading => Some(timing_shown(node)),
        Row::RenderHeading => Some(render_shown(node)),
        Row::SendHeading => Some(send_shown(node)),
        _ => None,
    }
}

/// Is this row one the Timing heading folds: a Time, a Speed or an Offset under an open
/// heading. Only drawn while the heading is open, so a row that is drawn is under one.
pub fn under_timing(node: &Node, row: Row) -> bool {
    let Row::Input(i) = row else {
        return false;
    };
    node.def.timing.is_some()
        && node
            .inputs
            .get(i)
            .is_some_and(|p| crate::nodes::is_time_row(p.key))
}

/// Is the Timing heading open: its option, which reads absent as closed.
pub fn timing_shown(node: &Node) -> bool {
    tick(node, crate::nodes::timing::HEADING.key).unwrap_or(false)
}

/// Is this input's row drawn: every input but the row the node's time mode puts away, and
/// every time row while their heading is closed (`nodes::is_time_row`).
fn input_shown(node: &Node, index: usize) -> bool {
    let Some(port) = node.inputs.get(index) else {
        return true;
    };
    if crate::nodes::timing::is_inactive(node, port.key) {
        return false;
    }
    timing_heading(node).is_none() || timing_shown(node) || !crate::nodes::is_time_row(port.key)
}

/// How many of an Output's selects are drawn in the render section rather than the option
/// block, whether or not that section is shown.
pub fn render_selects(node: &Node) -> usize {
    if node.def.is_output {
        RENDER_SELECTS
    } else {
        0
    }
}

/// Is the Output's Send section open: its heading's option, which reads absent as closed.
pub fn send_shown(node: &Node) -> bool {
    render_heading(node) && tick(node, crate::nodes::output::SEND).unwrap_or(false)
}

/// How many of an Output's options are drawn as its Send rows rather than as selects, whether
/// or not that section is shown.
pub fn send_options(node: &Node) -> usize {
    if node.def.is_output {
        crate::nodes::output::SEND_OPTIONS.len()
    } else {
        0
    }
}

/// The rows under an Output's Send heading while it is open, top to bottom; none otherwise.
pub fn send_rows(node: &Node) -> impl Iterator<Item = crate::nodes::output::SendRow> + use<> {
    send_shown(node)
        .then(|| crate::nodes::output::send_rows(node))
        .into_iter()
        .flatten()
}

/// A row holding a select: the control plus 3 points either side.
///
/// Tighter than `CONTROL_ROW_PITCH` because a select is not an s-number. An s-number is 25
/// points because it holds two stepper buttons and a field a hand scrubs along; a select
/// holds one line of text and hugs it. A node's options are the part of it that is set once
/// and then read, so the rows that carry them are the rows worth taking height out of — a
/// `video` node has four.
pub const OPTION_ROW_PITCH: f32 = 24.0;
/// What each line past the first adds to a multi-line value's row. The row's own padding is
/// already in `OPTION_ROW_PITCH`, so this is a line of text and nothing else — otherwise a
/// four-line note would carry four rows' worth of padding down its side.
pub const VALUE_LINE: f32 = 14.0;
/// A value's height before its field has ever drawn, from the lines the definition asked
/// for. An assumed line height, which is exactly what the measured one replaces — so it is
/// used for one frame and never again.
pub fn seed_value_height(rows: u8) -> f32 {
    f32::from(rows.max(1) - 1).mul_add(VALUE_LINE, OPTION_ROW_PITCH)
}

/// How tall each value a node declares came out the last time its field drew, in world units,
/// keyed by node.
///
/// **View state, not document data.** Wrapping needs the font and the width, so only the
/// field that draws a value knows how tall its text is, and it says so while it paints. That
/// answer is about this canvas rather than about the graph: it is not an edit, it is not in
/// the file or the undo history, and it never crosses to the synth. A node whose fields have
/// not drawn yet has no entry, and is laid out from its definition's line counts through
/// [`value_height`] until they have.
#[derive(Debug, Clone, Default)]
pub struct Measured(HashMap<NodeId, Vec<f32>>);

impl Measured {
    /// One node's heights, one per value its definition declares, in declaration order —
    /// empty for a node whose fields have not drawn.
    pub fn of(&self, id: NodeId) -> &[f32] {
        self.0.get(&id).map_or(&[], Vec::as_slice)
    }

    /// What one value's field says it needs. The rest of the node's values start at their
    /// seeds, so a node's list is always whole once it has one.
    pub fn set(&mut self, id: NodeId, node: &Node, index: usize, height: f32) {
        let heights = self.0.entry(id).or_insert_with(|| {
            (0..node.def.values.len())
                .map(|i| value_height(node, &[], i))
                .collect()
        });
        if let Some(slot) = heights.get_mut(index) {
            *slot = height;
        }
    }

    /// Forget every node the graph no longer holds.
    pub fn retain_in(&mut self, graph: &Graph) {
        self.0.retain(|id, _| graph.get(*id).is_some());
    }
}

/// How tall one value of a node is drawn, in world units: what its field last measured, or
/// the seed its definition's line count gives before it has drawn. Zero for a value a region
/// draws, which takes no row.
pub fn value_height(node: &Node, measured: &[f32], index: usize) -> f32 {
    match node
        .def
        .values
        .get(index)
        .map_or(0, crate::nodes::ValueDef::rows)
    {
        0 => 0.0,
        rows => measured
            .get(index)
            .copied()
            .unwrap_or_else(|| seed_value_height(rows)),
    }
}

/// How far a value's box pulls in from the node's edges, on every side. `ROW_BLOCK_INSET`'s
/// value, because it is the same gesture an input block makes on its far side — the width
/// this design system pulls a slab in by.
pub const VALUE_INSET: f32 = ROW_BLOCK_INSET;

/// How tall a select is: 4 points of padding around one `FONT_TINY` line, which is a chip's
/// height for the same reason `TAG_HEIGHT` is.
pub const SELECT_HEIGHT: f32 = 18.0;

/// An audio scope: a spectrum with the band handles over it, then a meter a band.
///
/// silvia's canvas is 160 px in a node 320 wide, and the size is load-bearing: at half that
/// the spectrum is a smear, the handles overlap, and there is nowhere to put a threshold. The
/// spectrum keeps silvia's 40%, leaving each meter 22 px — a little under a control row.
pub const SCOPE_HEIGHT: f32 = 130.0;
pub const SCOPE_SPECTRUM: f32 = 64.0;

/// The meters alone, with no spectrum over them: what `maininput` draws, since the handles
/// are the only part of a scope that is about tuning and its tuning is the panel's. The same
/// three rows at the same pitch, so the two nodes' meters are one element and not two.
pub const METERS_HEIGHT: f32 = SCOPE_HEIGHT - SCOPE_SPECTRUM;

/// How wide a node carrying a scope is. silvia's 320 for the same reason it is 160 tall.
pub const SCOPE_NODE_WIDTH: f32 = 300.0;

/// How tall a region's heading is in world units, its bar and the air around it: silvia's
/// `.section-toggle` min-height with its own `padding` around it. `widgets::HEADING_PAD` of that is clear above
/// and below the bar itself, so two stacked headings read as two strips rather than one block.
pub const HEADING_HEIGHT: f32 = 24.0;

/// A trace band: a picture of the shape a node like `adsr` or `oscillator` is editing,
/// flush to the body's bottom corners as the audio scope and an Output's own render are.
pub const TRACE_HEIGHT: f32 = 48.0;

/// The band a recorded curve is drawn in: silvia's own 120 pixel automation canvas, in the
/// world units a 300 wide body is measured in.
pub const CURVE_HEIGHT: f32 = 90.0;

/// The caption under a trace: silvia's two stacked lines, a tiny label over a reading, with
/// the air around them a row of the node has.
pub const CAPTION_HEIGHT: f32 = 26.0;

/// One horizontal band inside a node body. Everything about node layout derives from the
/// row list, so heights, port centers and hit rects cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Input(usize),
    /// The bar a moving node's Time and Offset fold under, in the input block where Time's
    /// row is: the same bar a region's heading is, over two port rows. On every such node
    /// whether it is open or closed.
    TimingHeading,
    Output(usize),
    Option(usize),
    /// One value the node declares, drawn by its own `ValueKind` — a box of text on a
    /// `note`, and later a pad or a step grid. Its height is the lines it asked for.
    Value(usize),
    /// One row of an Output's Render section: its three numbers, then the button that
    /// starts a render with its progress behind it. `RENDER_ROWS` of them, on every Output.
    Render(usize),
    /// The bar over those rows: a region's heading, over rows rather than over a band. On
    /// every Output whether the section is open or closed — a closed section still says it
    /// is there, which is the whole of why it is a heading and not a tick.
    RenderHeading,
    /// The bar over an Output's Send rows, the same bar the Render section wears, on every
    /// Output whether they are open or closed.
    SendHeading,
    /// One row under it: a way out with its status and its button, Syphon's Flip, or the
    /// alpha both ways share — `nodes::output::send_rows`.
    Send(crate::nodes::output::SendRow),
    /// An Output's status line: whether anything is cabled in, which deck claimed it, and
    /// whether it is rendering. silvia's status line: one row, on every Output and on nothing
    /// else.
    Readout,
    /// Every tick the node declares, side by side in one row of their own — silvia's
    /// `Numbers` / `Events` / `Scope` band at the foot of an audio node. One row however many
    /// there are, which is the whole point of them: three questions that each hide part of the
    /// node, in the space one select used to take.
    Checks,
}

impl Row {
    /// The port this row hangs off: its index in the node's inputs or outputs, and whether it
    /// is an input. `None` for every row that is the node's own state rather than a port.
    pub fn port(self) -> Option<(usize, bool)> {
        match self {
            Row::Input(i) => Some((i, true)),
            Row::Output(i) => Some((i, false)),
            Row::TimingHeading
            | Row::Option(_)
            | Row::Checks
            | Row::Value(_)
            | Row::Render(_)
            | Row::RenderHeading
            | Row::SendHeading
            | Row::Send(_)
            | Row::Readout => None,
        }
    }

    /// Which of the three stacked blocks this row belongs to — `.node-inputs`,
    /// `.node-outputs` or `.node-options` — for anything that has to know where one block
    /// ends and the next begins: the alternating fill restarts there, and so does the
    /// block's own inset and corner rounding.
    pub fn section(self) -> u8 {
        match self {
            // The Timing heading is in the input block, over the two inputs it folds.
            Row::Input(_) | Row::TimingHeading => 0,
            Row::Output(_) => 1,
            // The ticks are options, and they sit in the option block: one groove above the
            // whole of it, not a second seam inside it.
            // A value sits in the option block too: both are the node's own state rather
            // than a port, and a second groove between them would say they were unrelated.
            Row::Option(_)
            | Row::Checks
            | Row::Value(_)
            | Row::Render(_)
            | Row::RenderHeading
            | Row::SendHeading
            | Row::Send(_)
            | Row::Readout => 2,
        }
    }
}

/// How far an input or output block sits from the node's far edge — silvia's
/// `.node-inputs { padding-right: 0.5rem }` and `.node-outputs { padding-left: 0.5rem }`.
/// The near edge, where the ports hang off it, stays flush; only the far side pulls in,
/// which is what makes each block read as a slab short of the edge rather than the full row.
pub const ROW_BLOCK_INSET: f32 = 6.0;

/// The corner an input or output block rounds on its own first and last row, on the side
/// away from the ports — silvia's `.node-input:first-child { border-top-right-radius: 6px }`
/// and `:last-child { border-bottom-right-radius: 6px }` (mirrored for outputs, top/bottom
/// left). Options carry no such rounding of their own.
pub const ROW_BLOCK_RADIUS: f32 = 6.0;

/// Room a section boundary — inputs to outputs, outputs to options — takes beyond its two
/// rows' own heights: silvia's own `<hr>` sets no margin of its own, so it keeps the browser's
/// unstyled default, `0.5em` above and below, resolved against this design system's 12px
/// root — 6px, 6px — plus the rule's own `border-top: 2px groove`. Measured against a 2x crop
/// of silvia's `blur.png` between "Blur Y" and "Output" (28 device px = 14 CSS px, split
/// 6/2/6) rather than assumed from the CSS alone.
pub const SECTION_GAP_ABOVE: f32 = 6.0;
/// The groove's own thickness: silvia's `hr { border-top: 2px groove ... }`.
pub const GROOVE_HEIGHT: f32 = 2.0;
pub const SECTION_GAP_BELOW: f32 = 6.0;
/// Total vertical room one section boundary takes, gap included on both sides of the groove.
pub const SECTION_GAP: f32 = SECTION_GAP_ABOVE + GROOVE_HEIGHT + SECTION_GAP_BELOW;
/// Plain gap under the header before the first row — no groove, just the room silvia's own
/// `margin-bottom: 0.5rem` leaves before content begins. A section boundary carries a groove
/// because it separates two *sections*; the header is not one, so it gets one gap, not a
/// gap-groove-gap.
pub const HEADER_GAP: f32 = SECTION_GAP_ABOVE;

/// Where the groove that opens a section sits, as an offset from the body's top: centerd in
/// the gap `rows_with_top` left in front of that section's first row — `SECTION_GAP_BELOW`
/// clear beneath it, `GROOVE_HEIGHT / 2` of it above that. Beside the constants it is made
/// of, so re-splitting the 6/2/6 moves the rule and the groove together.
pub fn groove_top(row_top: f32) -> f32 {
    row_top - SECTION_GAP_BELOW - GROOVE_HEIGHT * 0.5
}

/// The row's own background block, in world units: a slice of `body`, inset away from the
/// port side for an input or an output — silvia's `.node-inputs`/`.node-outputs` padding —
/// and full width for an option, which silvia gives no such inset.
///
/// Takes the body rather than the node: this is called once per row, and the body's height
/// is the whole row list's to answer, which would make node layout quadratic in its rows
/// again — the very thing `rows_with_top` exists to avoid. Every caller already has the rect.
pub fn row_block(body: Rect, row: Row, top: f32, height: f32) -> Rect {
    let full = Rect::from_min_size(
        Pos2::new(body.min.x, body.min.y + top),
        vec2(body.width(), height),
    );
    match row {
        // The Timing heading stands among the inputs it folds, so it stops short of the far
        // edge where they do.
        Row::Input(_) | Row::TimingHeading => Rect::from_min_max(
            full.min,
            Pos2::new(full.max.x - ROW_BLOCK_INSET, full.max.y),
        ),
        Row::Output(_) => Rect::from_min_max(
            Pos2::new(full.min.x + ROW_BLOCK_INSET, full.min.y),
            full.max,
        ),
        // A readout is a line of text across the body, like an option row: full width, no
        // inset away from a port side.
        Row::Option(_)
        | Row::Checks
        | Row::Render(_)
        | Row::RenderHeading
        | Row::SendHeading
        | Row::Send(_)
        | Row::Readout => full,
        // A value's box is a slab sitting *on* the node rather than a band across it, so it
        // pulls in on all four sides. An option row runs edge to edge because it is a label
        // with a control at the end of it; a box of text that touches the node's own border
        // reads as a hole in the node.
        Row::Value(_) => full.shrink(VALUE_INSET),
    }
}

/// The narrowest this node's own contents can be drawn in: the widest thing in it.
///
/// `NodeDef::width` is the rows' own answer — a node whose longest label does not fit the
/// default 200 — and each region declares the narrowest body it can be drawn in. So an Output
/// is 240 because its render says so and an audio node is 300 because its scope does, rather
/// than because layout knows what kind of node it is. A closed region still asks: folding a
/// scope away must not reflow the node under the hand that closed it.
pub fn natural_width(node: &Node) -> f32 {
    node.def
        .regions
        .iter()
        .filter_map(|r| crate::widgets::def(*r).width)
        .fold(node.def.width.unwrap_or(NODE_WIDTH), f32::max)
}

/// The widest a body may be dragged, in world units.
///
/// silvia's textarea has no ceiling at all, but silvia's note is a box inside a node where
/// this *is* the node: a width nobody can see the far side of is a node whose header, and so
/// the handle it is dragged by, is off the screen. Six default bodies is a paragraph a good
/// deal wider than anything else on the canvas and still one glance across.
pub const MAX_NODE_WIDTH: f32 = NODE_WIDTH * 6.0;

/// How wide a node's body is drawn: what its kind asks for, or the width a hand dragged it
/// to, which may be wider but never narrower.
///
/// The floor is the whole of why this is the one place that decides: a width from the file,
/// from undo or from a grip is a number that arrived from outside, and a body narrower than
/// its own rows is one with the rows drawn outside it.
pub fn node_width(node: &Node) -> f32 {
    let natural = natural_width(node);
    node.dragged_width
        .filter(|w| w.is_finite())
        .map_or(natural, |w| w.clamp(natural, MAX_NODE_WIDTH))
}

/// True when this input draws a control: it has one, and nothing is plugged into it.
pub fn input_has_control(node: &Node, index: usize) -> bool {
    node.inputs
        .get(index)
        .is_some_and(|p| node.controls.contains_key(p.key))
}

/// Height of one row. A row carrying a control is taller, and a value's row is as tall as its
/// text: `measured` is the node's own heights out of [`Measured`].
pub fn row_height(node: &Node, measured: &[f32], row: Row) -> f32 {
    match row {
        Row::Input(i) if input_has_control(node, i) || speed_tall(node, i) => CONTROL_ROW_PITCH,
        Row::Input(_) | Row::Output(_) => PORT_PITCH,
        // A render's numbers are control rows; its selects and its button are option rows.
        Row::Render(i) if (RENDER_SELECTS..RENDER_SELECTS + RENDER_CONTROLS).contains(&i) => {
            CONTROL_ROW_PITCH
        }
        Row::Option(_) | Row::Checks | Row::Render(_) | Row::Send(_) => OPTION_ROW_PITCH,
        // The same bar a region's heading is, so the two read as one affordance.
        Row::RenderHeading | Row::SendHeading | Row::TimingHeading => HEADING_HEIGHT,
        Row::Readout => READOUT_ROW_PITCH,
        // A value's height is the lines it declares: one line is an option row, and each
        // line after that adds a line of text rather than a whole row's padding.
        // Whatever the field last said it needed, plus the gap around it. The inset is part
        // of the row rather than taken out of it, so the box keeps the whole height it asked
        // for and the node grows by the margin.
        Row::Value(i) => value_height(node, measured, i) + 2.0 * VALUE_INSET,
    }
}

/// Whether this input is a Time standing where its Speed stands in the other mode, and so as
/// tall as the Speed's knob makes it: a mode switched swaps one row for the other in place,
/// and the node keeps its height. A Speed in Loop mode takes no cable, so its knob is always
/// there to measure by. The loop meter stands where the knob would (`ui::loop_meter`).
pub fn speed_tall(node: &Node, index: usize) -> bool {
    node.def.timing.is_some()
        && node
            .inputs
            .get(index)
            .is_some_and(|p| crate::nodes::is_time(p.key))
}

/// What one of a node's show/hide ticks says, or `None` where the node carries no such
/// option — which is almost every node, since almost none declares a tick at all.
///
/// The single reader of a tick's value, and of a heading's. The polarity belongs to whoever
/// asks: the ticks row's own `shown` reads absent as shown, and a region reads absent as its
/// heading's declared default.
pub fn tick(node: &Node, key: &str) -> Option<bool> {
    node.options.get(key).map(|v| v != crate::nodes::OFF)
}

/// Whether a run of port rows a tick can hide is drawn. Absent is shown.
pub fn shown(node: &Node, key: &str) -> bool {
    tick(node, key).unwrap_or(true)
}

/// Is this output row drawn?
///
/// The `uniforms` and `events` ticks hide the outputs that are **uniform numbers and
/// events**, which needs no new bit on a port: such a number output is a `UniformNumber` and
/// an event output is an `Action`, and both are already on the `PortDef` layout reads. A
/// hidden port keeps its cables: with no row to hang on, a hidden output that carries one
/// gathers on the header's right edge, exactly as a collapsed node's outputs do.
fn output_shown(node: &Node, index: usize) -> bool {
    node.outputs.get(index).is_none_or(|p| {
        if p.ty == crate::graph::PortType::Action {
            shown(node, "events")
        } else if p.ty.is_uniform() {
            shown(node, "uniforms")
        } else {
            true
        }
    })
}

/// Every row in a node, top to bottom, with its height.
///
/// An iterator, not a `Vec`: it is walked for every node on the workspace every frame, and at
/// 60 Hz an allocation per node was once the largest single cost in the canvas.
pub fn rows<'a>(node: &'a Node, measured: &'a [f32]) -> impl Iterator<Item = (Row, f32)> + 'a {
    let n = if node.collapsed { 0 } else { usize::MAX };
    // The options that are still selects. A tick is an option too, and so is a region's
    // heading: the ticks share the single `Checks` row after them and a heading is drawn on its
    // own region, so neither is counted here — which is also what keeps `Row::Option(i)` the
    // i-th *select*, the index `ui/` walks by.
    let checks = node.def.checks();
    let selects = node.options.len().saturating_sub(
        checks
            + node.def.headings()
            + node.def.in_regions()
            + node.def.on_headings()
            + render_selects(node)
            + send_options(node),
    );
    let render = if render_shown(node) { RENDER_ROWS } else { 0 };
    let time = timing_heading(node);
    (0..node.inputs.len().min(n))
        .flat_map(move |i| {
            (time == Some(i))
                .then_some(Row::TimingHeading)
                .into_iter()
                .chain(input_shown(node, i).then_some(Row::Input(i)))
        })
        .chain(
            (0..node.outputs.len().min(n))
                .filter(|i| output_shown(node, *i))
                .map(Row::Output),
        )
        .chain((0..selects.min(n)).map(Row::Option))
        .chain(render_heading(node).then_some(Row::RenderHeading))
        .chain((0..render.min(n)).map(Row::Render))
        // A value a region draws has no row of its own, and says so by declaring no lines.
        .chain(
            (0..node.def.values.len().min(n))
                .filter(|i| node.def.values[*i].rows() > 0)
                .map(Row::Value),
        )
        // How the Output leaves, under a heading of its own.
        .chain(render_heading(node).then_some(Row::SendHeading))
        .chain(send_rows(node).map(Row::Send))
        .chain((checks > 0 && !node.collapsed).then_some(Row::Checks))
        // Last, so the line sits directly above the Output's own picture — where silvia's
        // own status line sits, immediately above its preview canvas.
        .chain((node.def.is_output && !node.collapsed).then_some(Row::Readout))
        .map(move |row| (row, row_height(node, measured, row)))
}

/// Every row with its top offset from the body's top, in one pass.
///
/// `row_top` re-walks the row list per call, so asking it for each row in turn makes node
/// layout quadratic in the number of rows — paid for every node in the graph, before
/// culling. Anything that wants more than one row's position walks this instead.
///
/// A row that opens a new section — inputs to outputs, outputs to options — sits
/// `SECTION_GAP` below the last, the room silvia's own `<hr>` takes with its unstyled margin
/// either side of the groove itself. The very first row instead gets `HEADER_GAP` alone: the
/// header is not a section, so what sits below it is the one plain gap `margin-bottom`
/// describes, not a groove with nothing above it to separate.
pub fn rows_with_top<'a>(
    node: &'a Node,
    measured: &'a [f32],
) -> impl Iterator<Item = (Row, f32, f32)> + 'a {
    let mut top = HEADER_HEIGHT;
    let mut section: Option<u8> = None;
    let mut timed = false;
    let mut closed = false;
    rows(node, measured).map(move |(row, h)| {
        top += match section {
            None => HEADER_GAP,
            Some(s) if s != row.section() => SECTION_GAP,
            Some(_) => 0.0,
        };
        // A heading's gap, where the next row goes on in the same section: under the Timing
        // heading's last row, and under a closed bar unless another bar follows, whose own
        // air above it is the gap.
        let under = under_timing(node, row);
        let open = heading_open(node, row);
        if section == Some(row.section()) && ((timed && !under) || (closed && open.is_none())) {
            top += HEADING_GAP;
        }
        timed = under;
        closed = open == Some(false);
        section = Some(row.section());
        let this = top;
        top += h;
        (row, this, h)
    })
}

/// The body's bottom padding, which is what a node with no region of its own has.
pub const BODY_PAD: f32 = 6.0;

/// The clearance the body's own rounded corner needs under a band that is not flush.
///
/// **The rule at the foot of a node**: the last band either paints its own ground edge to
/// edge, in which case it is handed the body's bottom radius and rounds its two bottom
/// corners inside the arc — an Output's render, an open preview, a scope, a trace — or it
/// does not, in which case it stands `FOOT_PAD` of body clear of the curve so no square
/// corner crosses it. A closed region is a heading bar and nothing else, which is the second
/// case; a node with no band at all gets `BODY_PAD`, which is the same distance. It is
/// `ROW_BLOCK_RADIUS`, the radius the body's own bottom corners are drawn with, because a
/// corner cannot reach further in than its own radius — and the same reason the output slab's
/// corner is that radius. silvia gets the shape from `overflow: hidden` on `.node`, which
/// egui has no equivalent of: a band there is clipped by the node's rounding rather than
/// asked to respect it.
pub const FOOT_PAD: f32 = ROW_BLOCK_RADIUS;

/// One band of a node's body below its rows: a region the node declared, or the padding a
/// node with nothing below its rows has at its foot.
///
/// The order is the node's own — `NodeDef::regions`, through `Node::def` — and a node has
/// **every** region it declares rather than the one a ladder picked, which is why `video`
/// keeps its meters *and* its clip. Which regions a node has is a property of the node rather
/// than of the caller asking, so the height and every band drawn into one are answered from
/// the same place: [`Layouts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// The i-th of `node.def.regions`, in declaration order.
    Declared(usize),
    /// The body's bottom padding, and only where no region takes the foot.
    Pad,
}

/// Whether a region is open: its heading's option, falling back to the heading's own default
/// for a node — or a file — nobody has asked. A region with no heading is always open.
///
/// The one polarity rule there is. It used to be the caller's on purpose, because a tick that
/// hides part of a node reads absent as shown while a tick that adds a picture reads absent as
/// off; on a heading the same fact is one field per region.
pub fn region_open(node: &Node, region: crate::nodes::Region) -> bool {
    region
        .heading()
        .is_none_or(|h| tick(node, h.key).unwrap_or(h.open))
}

/// How tall one band is on this node, or `None` where the node does not have it — which is
/// every band of a collapsed node.
///
/// A region's heading plus its content where it is open, and the heading alone where it is
/// closed: that is the whole of hiding, and nothing above the region carries a notion of
/// visible. A closed region still says it is there.
fn region_height(node: &Node, region: Region) -> Option<f32> {
    if node.collapsed {
        return None;
    }
    let Region::Declared(i) = region else {
        // The pad is what is left over, so it is answered by `Layouts::push` alone: a node whose
        // foot is a picture has no six points of nothing under it.
        return None;
    };
    let region = *node.def.regions.get(i)?;
    let heading = if region.heading().is_some() {
        HEADING_HEIGHT
    } else {
        0.0
    };
    let content = if region_open(node, region) {
        (crate::widgets::def(region).size)(node)
    } else {
        0.0
    };
    (heading + content > 0.0).then_some(heading + content)
}

/// The empty band at the foot of a body: what a rect for something this node does not have is.
fn foot(body: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(body.min.x, body.max.y), body.max)
}

/// A port's dot and what it is, laid out with its node so cables and drop targets can be
/// resolved after every node has been.
///
/// In world units inside a [`NodeLayout`], and on screen once the canvas has placed it for
/// the frame's transform — the same four fields either way.
#[derive(Debug, Clone, Copy)]
pub struct PortSlot {
    pub port: crate::graph::PortRef,
    pub ty: crate::graph::PortType,
    pub is_input: bool,
    pub center: Pos2,
}

/// One row as layout placed it: which row, its top as an offset from the body's top, and
/// its height — what [`rows_with_top`] yields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowBox {
    pub row: Row,
    pub top: f32,
    pub height: f32,
}

/// One band below the rows as layout placed it: which, how tall, and where, heading included.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionBox {
    pub region: Region,
    pub height: f32,
    pub rect: Rect,
}

/// Where everything on one node is, in world units: the body, every row, every band below
/// the rows and every port's dot.
///
/// Worked out once a frame by [`Layouts`] and read by everything that draws or hit-tests the
/// node — the cull, the shadow, the body, the controls, the ports, the marquee, the cost
/// strip, the minimap and the strip's bounds — so none of them walks the rows again, and a
/// region's size, which for an Output is a parse of its `resolution`, is asked once.
#[derive(Debug, Clone, Copy)]
pub struct NodeLayout<'a> {
    pub id: NodeId,
    /// The body, header to foot.
    pub rect: Rect,
    /// How tall the body is, before it became the difference of two edges.
    pub height: f32,
    pub rows: &'a [RowBox],
    /// Every band, top to bottom, with a collapsed node's list empty.
    pub regions: &'a [RegionBox],
    /// Every port's dot in world units, inputs first: where a cable ends.
    pub ports: &'a [PortSlot],
    /// What the node's value fields measured, as it was laid out from: [`Measured::of`].
    pub measured: &'a [f32],
}

impl NodeLayout<'_> {
    /// One row's own block, in world units — [`row_block`] for the row where it is laid out,
    /// or `None` for a row this node does not have.
    pub fn block(&self, row: Row) -> Option<Rect> {
        self.rows
            .iter()
            .find(|r| r.row == row)
            .map(|r| row_block(self.rect, r.row, r.top, r.height))
    }

    /// Every row with its own block, top to bottom.
    pub fn blocks(&self) -> impl Iterator<Item = (RowBox, Rect)> + '_ {
        self.rows
            .iter()
            .map(|r| (*r, row_block(self.rect, r.row, r.top, r.height)))
    }

    /// The band one region occupies, heading included, or an empty rect at the body's foot for
    /// a band this node does not have.
    pub fn region(&self, region: Region) -> Rect {
        self.regions
            .iter()
            .find(|b| b.region == region)
            .map_or_else(|| foot(self.rect), |b| b.rect)
    }

    /// Which band is flush with the foot of the body, where there is one at all: what a region
    /// asks when it has to know whether the body's bottom corners are its own to round.
    pub fn foot(&self) -> Option<Region> {
        self.regions.last().map(|b| b.region)
    }
}

/// One node's spans in [`Layouts`]' shared buffers.
#[derive(Debug, Clone)]
struct Laid {
    id: NodeId,
    rect: Rect,
    height: f32,
    rows: std::ops::Range<usize>,
    regions: std::ops::Range<usize>,
    ports: std::ops::Range<usize>,
    measured: std::ops::Range<usize>,
}

/// Every node on one workspace, laid out once for the frame.
///
/// Three buffers shared by every node, each node a span of them, and kept by the canvas from
/// one frame to the next so laying a workspace out allocates nothing once it has been done:
/// an allocation per node per frame is the cost the row walk was made lazy to avoid.
#[derive(Debug, Default)]
pub struct Layouts {
    nodes: Vec<Laid>,
    rows: Vec<RowBox>,
    regions: Vec<RegionBox>,
    ports: Vec<PortSlot>,
    /// Which of `ports` a port is, so a cable resolves both ends without a scan.
    index: HashMap<crate::graph::PortRef, usize>,
    measured: Vec<f32>,
}

impl Layouts {
    /// Lay out every node shown on `workspace`, in `Graph::on_workspace` order — which is id
    /// order, and what [`Layouts::find`] searches by.
    pub fn lay_out(
        &mut self,
        graph: &Graph,
        workspace: crate::graph::WorkspaceId,
        measured: &Measured,
    ) {
        self.nodes.clear();
        self.rows.clear();
        self.regions.clear();
        self.ports.clear();
        self.index.clear();
        self.measured.clear();
        for (id, node) in graph.on_workspace(workspace) {
            self.push(graph, id, node, measured.of(id));
        }
    }

    /// One node laid out alone, where the graph holds it: what an arrange, a clamp to the strip
    /// and a test ask about a node outside a frame. The same walk a frame makes, so there is
    /// one layout and not two that have to agree. Empty where the graph does not hold `id`.
    pub fn one(graph: &Graph, id: NodeId, measured: &[f32]) -> Self {
        let mut laid = Self::default();
        if let Some(node) = graph.get(id) {
            laid.push(graph, id, node, measured);
        }
        laid
    }

    /// Lay one node out on the end of the buffers: one walk of its rows and one of its
    /// regions, each region's size asked once.
    fn push(&mut self, graph: &Graph, id: NodeId, node: &Node, measured: &[f32]) {
        let rows = self.rows.len();
        let mut last = None;
        for (row, top, height) in rows_with_top(node, measured) {
            self.rows.push(RowBox { row, top, height });
            last = Some((top, height));
        }
        let body = last.map_or(0.0, |(top, h)| top + h - HEADER_HEIGHT);

        let regions = self.regions.len();
        let declared = node.def.regions;
        for i in 0..declared.len() {
            if let Some(height) = region_height(node, Region::Declared(i)) {
                self.regions.push(RegionBox {
                    region: Region::Declared(i),
                    height,
                    rect: Rect::NOTHING,
                });
            }
        }
        // The foot: the body's own padding under a node with no band of its own, and the
        // corner's clearance under a closed one.
        let pad = match self.regions[regions..].last().map(|b| b.region) {
            None => (!node.collapsed).then_some(BODY_PAD),
            Some(Region::Declared(i)) => (!region_open(node, declared[i])).then_some(FOOT_PAD),
            Some(Region::Pad) => None,
        };
        if let Some(height) = pad {
            self.regions.push(RegionBox {
                region: Region::Pad,
                height,
                rect: Rect::NOTHING,
            });
        }
        let below: f32 = self.regions[regions..].iter().map(|b| b.height).sum();
        let height = HEADER_HEIGHT + body + below;
        let rect = Rect::from_min_size(node.pos, vec2(node_width(node), height));
        let mut top = rect.max.y - below;
        for band in &mut self.regions[regions..] {
            band.rect = Rect::from_min_max(
                Pos2::new(rect.min.x, top),
                Pos2::new(rect.max.x, top + band.height),
            );
            top += band.height;
        }

        let ports = self.ports.len();
        self.ports.reserve(node.inputs.len() + node.outputs.len());
        // A port on a row hangs off that row's middle. Rows come out inputs first, then
        // outputs, and a collapsed node has none.
        for r in &self.rows[rows..] {
            let Some((index, is_input)) = r.row.port() else {
                continue;
            };
            let (x, list) = if is_input {
                (rect.min.x, &node.inputs)
            } else {
                (rect.max.x, &node.outputs)
            };
            let Some(p) = list.get(index) else { continue };
            self.ports.push(PortSlot {
                port: crate::graph::PortRef::new(id, p.key),
                ty: p.ty,
                is_input,
                center: Pos2::new(x, rect.min.y + r.top + r.height * 0.5),
            });
        }
        // A port with no row still has its cables, and a cable whose endpoint has no slot is
        // dropped by the canvas rather than drawn somewhere wrong. So such a port gathers on
        // the header's edge, off the header's middle: every port of a collapsed node, inputs
        // at one point and outputs at the other, and every output a tick hides that carries a
        // cable. A hidden output with nothing on it has no slot, so a header with nothing
        // wired from its hidden rows wears no dot.
        let y = rect.min.y + HEADER_HEIGHT * 0.5;
        for (is_input, list) in [(true, &node.inputs), (false, &node.outputs)] {
            let x = if is_input { rect.min.x } else { rect.max.x };
            for (i, p) in list.iter().enumerate() {
                let port = crate::graph::PortRef::new(id, p.key);
                let hidden = || !is_input && !output_shown(node, i);
                if node.collapsed || (hidden() && graph.targets_of(port).next().is_some()) {
                    self.ports.push(PortSlot {
                        port,
                        ty: p.ty,
                        is_input,
                        center: Pos2::new(x, y),
                    });
                }
            }
        }
        // A Time or an Offset folded under its heading gathers on the heading's own edge where
        // a cable comes into it, so the cable is drawn into the bar that says the rows are
        // there; with nothing on it, it has no dot, as a hidden output has none.
        let heading = self.rows[rows..]
            .iter()
            .find(|r| r.row == Row::TimingHeading)
            .map(|r| rect.min.y + r.top + r.height * 0.5);
        if let Some(heading) = heading {
            for (i, p) in node.inputs.iter().enumerate() {
                let port = crate::graph::PortRef::new(id, p.key);
                if !input_shown(node, i) && graph.source_of(port).is_some() {
                    self.ports.push(PortSlot {
                        port,
                        ty: p.ty,
                        is_input: true,
                        center: Pos2::new(rect.min.x, heading),
                    });
                }
            }
        }

        for (i, slot) in self.ports.iter().enumerate().skip(ports) {
            self.index.insert(slot.port, i);
        }
        let from = self.measured.len();
        self.measured.extend_from_slice(measured);
        self.nodes.push(Laid {
            id,
            rect,
            height,
            rows: rows..self.rows.len(),
            regions: regions..self.regions.len(),
            ports: ports..self.ports.len(),
            measured: from..self.measured.len(),
        });
    }

    /// How many nodes are laid out.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The `index`th node laid out, in the order they were.
    pub fn get(&self, index: usize) -> NodeLayout<'_> {
        let laid = &self.nodes[index];
        NodeLayout {
            id: laid.id,
            rect: laid.rect,
            height: laid.height,
            rows: &self.rows[laid.rows.clone()],
            regions: &self.regions[laid.regions.clone()],
            ports: &self.ports[laid.ports.clone()],
            measured: &self.measured[laid.measured.clone()],
        }
    }

    /// Every node laid out, in the order they were.
    pub fn iter(&self) -> impl Iterator<Item = NodeLayout<'_>> + '_ {
        (0..self.nodes.len()).map(|i| self.get(i))
    }

    /// One node's layout, where it is on this workspace.
    pub fn find(&self, id: NodeId) -> Option<NodeLayout<'_>> {
        self.nodes
            .binary_search_by_key(&id, |l| l.id)
            .ok()
            .map(|i| self.get(i))
    }

    /// Every port's dot on the workspace, in world units: each node's own `NodeLayout::ports`,
    /// one after another in the order the nodes were laid out.
    pub fn ports(&self) -> &[PortSlot] {
        &self.ports
    }

    /// Where a port is in [`Self::ports`], where it is on this workspace.
    pub fn slot(&self, port: crate::graph::PortRef) -> Option<usize> {
        self.index.get(&port).copied()
    }

    /// How far down the screen a cable between two nodes has to bow to pass under both bodies
    /// rather than behind them: the lower of their bottom edges under `t`, plus
    /// [`CABLE_CLEARANCE`] so it reads as passing under rather than grazing. The canvas and the
    /// minimap each ask it under their own transform.
    pub fn clearance(&self, a: NodeId, b: NodeId, t: &Transform, origin: Pos2) -> f32 {
        let edge = |id| self.find(id).map(|l| l.rect.max.y);
        let bottom = match (edge(a), edge(b)) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        let bottom = bottom.map_or(0.0, |y| t.to_screen(origin, Pos2::new(0.0, y)).y);
        bottom + CABLE_CLEARANCE * t.zoom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::nodes::Picture;

    /// One node of `g` laid out alone, from its definition's line counts.
    fn laid(g: &Graph, id: NodeId) -> Layouts {
        Layouts::one(g, id, &[])
    }

    /// How tall one node of `g` is.
    fn height(g: &Graph, id: NodeId) -> f32 {
        laid(g, id).find(id).expect("the node").height
    }

    /// How much of one node's height is its bands below the rows.
    fn below(g: &Graph, id: NodeId) -> f32 {
        let laid = laid(g, id);
        laid.find(id)
            .expect("the node")
            .regions
            .iter()
            .map(|b| b.height)
            .sum()
    }

    /// A band's own content, below whatever heading its region declares: what the region is
    /// handed to draw in.
    fn content(node: &Node, band: &RegionBox) -> Rect {
        let heading = match band.region {
            Region::Declared(i) => node.def.regions[i].heading().is_some(),
            Region::Pad => false,
        };
        if heading {
            Rect::from_min_max(
                Pos2::new(
                    band.rect.min.x,
                    (band.rect.min.y + HEADING_HEIGHT).min(band.rect.max.y),
                ),
                band.rect.max,
            )
        } else {
            band.rect
        }
    }

    /// The content of the first band whose declared region `pick` accepts, or the empty rect at
    /// the body's foot where the node has no such band.
    fn band_where(g: &Graph, id: NodeId, pick: impl Fn(crate::nodes::Region) -> bool) -> Rect {
        let node = g.get(id).expect("the node");
        let laid = laid(g, id);
        let l = laid.find(id).expect("the node");
        l.regions
            .iter()
            .find(|b| matches!(b.region, Region::Declared(i) if pick(node.def.regions[i])))
            .map_or_else(|| foot(l.rect), |b| content(node, b))
    }

    /// What a node's scope region has to draw in, which is the band below its heading.
    fn scope_band(g: &Graph, id: NodeId) -> Rect {
        let key = crate::nodes::audio_ports::SHOW_SCOPE.key;
        band_where(g, id, |r| r.heading().is_some_and(|h| h.key == key))
    }

    /// Where a node's own picture of what it publishes is drawn.
    fn preview_band(g: &Graph, id: NodeId) -> Rect {
        band_where(g, id, |r| matches!(r.picture(), Some(Picture::Port(_))))
    }

    /// Where an Output draws its own frame, as `.output-canvas` is.
    fn thumb_band(g: &Graph, id: NodeId) -> Rect {
        band_where(g, id, |r| r.picture() == Some(Picture::Render))
    }

    /// The band a node's trace is in: the first region a trace node declares.
    fn trace_band(g: &Graph, id: NodeId) -> Rect {
        band_where(g, id, |_| true)
    }

    /// Set one of a node's ticks or headings.
    fn set(g: &mut Graph, id: NodeId, key: &'static str, on: bool) {
        g.get_mut(id).unwrap().options.insert(
            key,
            if on {
                crate::nodes::ON
            } else {
                crate::nodes::OFF
            }
            .to_string(),
        );
    }

    /// Every node kind as it is laid out open, with every heading turned the other way and the
    /// Output's render section open, and collapsed, with a name for each.
    fn every_kind_every_way(mut check: impl FnMut(&Graph, NodeId, &str)) {
        for def in crate::nodes::REGISTRY {
            let mut g = Graph::new();
            let id = crate::nodes::add_to_graph(&mut g, def.slug, Pos2::new(7.25, 13.5))
                .expect("the registry's own slug");
            check(&g, id, def.slug);
            for region in def.regions {
                if let Some(h) = region.heading() {
                    let open = region_open(g.get(id).unwrap(), *region);
                    set(&mut g, id, h.key, !open);
                }
            }
            if def.is_output {
                set(&mut g, id, crate::nodes::output::OFFLINE, true);
            }
            check(&g, id, &format!("{} turned", def.slug));
            g.get_mut(id).unwrap().collapsed = true;
            check(&g, id, &format!("{} collapsed", def.slug));
        }
    }

    /// The ticks are read off the node, so a node that hides half of itself is still laid out
    /// without consulting the registry — and a hidden row is not a removed port: wired, its
    /// slot gathers on the header, as a collapsed node's do.
    #[test]
    fn a_tick_hides_its_own_part_and_nothing_else() {
        let mut g = Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "audioin", Pos2::ZERO).expect("in registry");
        let full = height(&g, id);
        let node = g.get(id).expect("just added");
        let outputs = node.outputs.len();
        assert!(outputs > 4, "the microphone publishes a bundle");
        assert_eq!(
            node.def.checks(),
            2,
            "the two port groups; the scope has a heading"
        );

        let output_rows = |g: &Graph| {
            laid(g, id)
                .find(id)
                .unwrap()
                .rows
                .iter()
                .filter(|r| matches!(r.row, Row::Output(_)))
                .count()
        };

        set(&mut g, id, "scope", false);
        assert!(
            height(&g, id) < full,
            "the scope's heading closes the scope"
        );
        assert_eq!(scope_band(&g, id).height(), 0.0, "and the band with it");
        assert_eq!(output_rows(&g), outputs, "and keeps every output row");

        set(&mut g, id, "scope", true);
        set(&mut g, id, "uniforms", false);
        assert!(scope_band(&g, id).height() > 0.0, "the scope came back");
        let without_numbers = output_rows(&g);
        assert!(
            without_numbers < outputs,
            "the uniform numbers went: {without_numbers} of {outputs}"
        );
        // The combination the one `show` select could not say: the events hidden, the
        // uniform numbers back.
        set(&mut g, id, "uniforms", true);
        set(&mut g, id, "events", false);
        let without_events = output_rows(&g);
        assert!(without_events < outputs, "the events went");
        assert_ne!(
            without_events, without_numbers,
            "hiding events is not hiding uniform numbers"
        );
        assert_eq!(
            g.get(id).unwrap().outputs.len(),
            outputs,
            "a hidden row is not a removed port"
        );
    }

    /// A source's own picture: the band at the foot of the body, under the scope, with a
    /// heading of its own — and nothing but that heading when it is closed.
    #[test]
    fn the_preview_heading_puts_a_picture_under_the_scope() {
        let mut g = Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "video", Pos2::ZERO).expect("in registry");
        let body = |g: &Graph| laid(g, id).find(id).unwrap().rect;

        let picture = preview_band(&g, id);
        assert!(picture.height() > 0.0, "the clip is shown by default");
        let with = height(&g, id);
        {
            let laid = laid(&g, id);
            let l = laid.find(id).unwrap();
            assert_eq!(
                l.foot(),
                Some(Region::Declared(1)),
                "the picture takes the foot, as an Output's render does"
            );
            assert_eq!(
                l.regions.iter().map(|b| b.region).collect::<Vec<_>>(),
                vec![Region::Declared(0), Region::Declared(1)],
                "a node with both keeps both, the meters over the picture"
            );
        }
        let width = node_width(g.get(id).unwrap());
        assert_eq!(picture.width(), width, "flush to the sides");
        assert_eq!(picture.max.y, body(&g).max.y, "and flush to the bottom");
        assert!(
            (picture.height() - width * 9.0 / 16.0).abs() < 1e-3,
            "a fixed 16:9 band"
        );
        // The scope is the band above it, not underneath: meters over the picture they are
        // measuring.
        let meters = scope_band(&g, id);
        assert_eq!(meters.height(), SCOPE_HEIGHT);
        assert!(
            (meters.max.y + HEADING_HEIGHT - picture.min.y).abs() < 1e-3,
            "the scope sits on the picture's own heading"
        );

        set(&mut g, id, "scope", false);
        assert_eq!(
            below(&g, id),
            HEADING_HEIGHT * 2.0 + preview_band(&g, id).height(),
            "the closed scope is its heading and nothing else"
        );
        assert_eq!(scope_band(&g, id).height(), 0.0, "and no band at all");
        assert_eq!(
            preview_band(&g, id).max.y,
            body(&g).max.y,
            "the picture is still the foot"
        );

        set(&mut g, id, "preview", false);
        set(&mut g, id, "scope", true);
        assert_eq!(preview_band(&g, id).height(), 0.0, "no picture, no band");
        assert!(height(&g, id) < with, "and the node is shorter for it");
    }

    /// A collapsed node is a header and nothing else — for every node kind in the registry.
    ///
    /// This is the test of the claim `rows` makes about itself. Everything about node layout
    /// derives from the row list, so yielding no rows should take the body, the port bands
    /// and the Output's own thumbnail with it, with nothing else edited. The regions are the
    /// one term that is not a row, which is why they are checked here beside them.
    #[test]
    fn collapsing_leaves_a_header_and_nothing_else() {
        for def in crate::nodes::REGISTRY {
            let mut g = Graph::new();
            let id = crate::nodes::add_to_graph(&mut g, def.slug, Pos2::new(7.0, 13.0))
                .expect("the registry's own slug");

            let expanded = height(&g, id);
            g.get_mut(id).unwrap().collapsed = true;
            let laid = laid(&g, id);
            let l = laid.find(id).unwrap();

            assert!(l.rows.is_empty(), "{}: rows survived", def.slug);
            assert!(l.regions.is_empty(), "{}: bands survived", def.slug);
            assert_eq!(
                l.height, HEADER_HEIGHT,
                "{}: height is not just the header",
                def.slug
            );
            assert!(
                expanded >= HEADER_HEIGHT,
                "{}: collapsing grew the node",
                def.slug
            );
            // The width is a property of the kind, not of the rows, and must not move.
            assert_eq!(
                l.rect.width(),
                node_width(g.get(id).unwrap()),
                "{}",
                def.slug
            );
            // An Output's own render is a region. Collapsed, there is nowhere to draw it,
            // and a rect derived from the resolution's aspect alone, ignoring `collapsed`,
            // would put it above the header. A picture the node draws of itself and a trace
            // are held to the same rule.
            for (band, what) in [
                (thumb_band(&g, id), "thumbnail"),
                (preview_band(&g, id), "picture band"),
                (trace_band(&g, id), "trace rect"),
            ] {
                assert!(
                    band.height() <= 0.0,
                    "{}: a collapsed node still has a {what}",
                    def.slug
                );
            }
        }
    }

    /// `adsr`, `oscillator` and `animation` reserve a fixed 48 pt region for their trace under
    /// its Trace heading and widen to the audio scope's 300 to hold it — the width rule turned
    /// to a second purpose rather than a fourth number. `adsr` captions its trace with silvia's
    /// gate and stage, which is a second region of its own under it.
    #[test]
    fn a_trace_node_reserves_a_fixed_region_and_the_scope_width() {
        for slug in ["adsr", "oscillator", "animation"] {
            let mut g = Graph::new();
            let id = crate::nodes::add_to_graph(&mut g, slug, Pos2::ZERO).expect("in registry");
            let node = g.get(id).unwrap();
            let caption = if slug == "adsr" { CAPTION_HEIGHT } else { 0.0 };
            assert_eq!(
                node.def.regions.len(),
                if slug == "adsr" { 2 } else { 1 },
                "{slug}: declares a trace, and a caption where it has one"
            );
            assert_eq!(node_width(node), SCOPE_NODE_WIDTH, "{slug}: widened for it");
            assert_eq!(
                below(&g, id),
                HEADING_HEIGHT + TRACE_HEIGHT + caption,
                "{slug}: a heading over a fixed region and nothing else below the rows"
            );
            let trace = trace_band(&g, id);
            assert_eq!(trace.height(), TRACE_HEIGHT, "{slug}: flush to the body");
            assert_eq!(trace.width(), SCOPE_NODE_WIDTH, "{slug}: flush to the body");
        }
    }

    /// The one walk accounts for every row of every node kind, in order, inside the body, and
    /// every port hangs off its own row — for every kind open, turned and collapsed.
    ///
    /// The row walk is the single derivation of row geometry — ports, bands, labels and the
    /// body's own height all come from it — so what is left to check is that it misses no
    /// row, that the rows stack without overlapping, that the last of them ends where the
    /// regions begin, and that each port is on the middle of its row. A drift in any of them
    /// puts a port dot off its row.
    #[test]
    fn the_row_walk_accounts_for_every_row_and_port_in_order() {
        every_kind_every_way(|g, id, what| {
            let node = g.get(id).unwrap();
            let laid = laid(g, id);
            let l = laid.find(id).unwrap();

            let mut bottom = HEADER_HEIGHT;
            for r in l.rows {
                assert!(
                    r.top >= bottom - 1e-3,
                    "{what}: {:?} starts at {}, over the row that ends at {bottom}",
                    r.row,
                    r.top
                );
                bottom = r.top + r.height;
            }
            // Every option is a row, except that the ticks share one between them and a
            // heading is drawn on the region or the run of rows it folds, an option a region
            // draws is drawn there, every value the node declares that is not drawn in a
            // region is a row of its own however many lines tall it is, and an Output carries
            // the bars over its Render and Send sections, each section's own rows while it is
            // open, and its status line. No tick hides a row here, since every node starts
            // with every tick shown; a moving node's Timing heading is a row, and closed, its
            // Time and Offset are not.
            let timed = timing_heading(node).is_some();
            let folded = if timed && !timing_shown(node) {
                node.inputs
                    .iter()
                    .filter(|p| crate::nodes::is_time_row(p.key))
                    .count()
            } else if timed {
                // Open, the row its time mode puts away is still not drawn.
                node.inputs
                    .iter()
                    .filter(|p| crate::nodes::timing::is_inactive(node, p.key))
                    .count()
            } else {
                0
            };
            if !node.collapsed {
                let selects = node.options.len()
                    - node.def.checks()
                    - node.def.headings()
                    - node.def.in_regions()
                    - node.def.on_headings()
                    - render_selects(node)
                    - send_options(node);
                assert_eq!(
                    l.rows.len(),
                    node.inputs.len() - folded
                        + usize::from(timed)
                        + node.outputs.len()
                        + selects
                        + node.def.values.iter().filter(|v| v.rows() > 0).count()
                        + usize::from(node.def.checks() > 0)
                        + 2 * usize::from(render_heading(node))
                        + if render_shown(node) { RENDER_ROWS } else { 0 }
                        + send_rows(node).count()
                        + usize::from(node.def.is_output),
                    "{what}: every row is walked"
                );
            }
            assert!(
                (bottom + below(g, id) - l.height).abs() < 1e-3,
                "{what}: the rows end at {bottom}, not where the regions begin"
            );

            // A port hangs off its own row's middle, on the body's edge; with no row — every
            // port of a collapsed node — off the header's middle.
            for slot in l.ports {
                let x = if slot.is_input {
                    l.rect.min.x
                } else {
                    l.rect.max.x
                };
                let list = if slot.is_input {
                    &node.inputs
                } else {
                    &node.outputs
                };
                let row = l.rows.iter().find(|r| {
                    r.row.port().is_some_and(|(i, input)| {
                        input == slot.is_input && list[i].key == slot.port.key
                    })
                });
                let y = row.map_or(HEADER_HEIGHT * 0.5, |r| r.top + r.height * 0.5);
                assert_eq!(
                    slot.center,
                    Pos2::new(x, l.rect.min.y + y),
                    "{what}: {:?}",
                    slot.port
                );
            }
            // Nothing here is wired, so a hidden output and a folded Time or Offset have no
            // slot; a collapsed node's every port has one.
            let shown = (0..node.outputs.len())
                .filter(|i| node.collapsed || l.rows.iter().any(|r| r.row == Row::Output(*i)))
                .count();
            assert_eq!(
                l.ports.len(),
                node.inputs.len() - folded + shown,
                "{what}: ports"
            );
        });
    }

    /// The region list is the whole of a node's height that is not a row, for every node kind.
    ///
    /// The regions tile the foot of the body in the list's order — no gap, no overlap, each
    /// flush to both sides — and the last of them ends exactly at the body's bottom edge, so a
    /// band can never be drawn at a height the body was not laid out with. An Output's own
    /// render is the one that takes the foot wherever there is one, which is the ordering
    /// claim `Region` makes about itself.
    #[test]
    fn the_regions_tile_the_foot_of_every_node() {
        every_kind_every_way(|g, id, what| {
            let node = g.get(id).unwrap();
            let laid = laid(g, id);
            let l = laid.find(id).unwrap();

            let mut top = l.rect.max.y - below(g, id);
            for band in l.regions {
                assert!(
                    (band.rect.min.y - top).abs() < 1e-3,
                    "{what}: {:?} is drawn at {}, laid out at {top}",
                    band.region,
                    band.rect.min.y
                );
                assert!(
                    (band.rect.height() - band.height).abs() < 1e-3,
                    "{what}: {:?}",
                    band.region
                );
                assert_eq!(
                    band.rect.width(),
                    l.rect.width(),
                    "{what}: {:?}",
                    band.region
                );
                assert_eq!(
                    l.region(band.region),
                    band.rect,
                    "{what}: {:?}",
                    band.region
                );
                top += band.height;
            }
            assert!(
                (top - l.rect.max.y).abs() < 1e-3,
                "{what}: the regions end at {top}, not at the body's foot {}",
                l.rect.max.y
            );
            assert_eq!(l.foot(), l.regions.last().map(|b| b.region), "{what}");
            assert!(
                node.collapsed || l.foot().is_some(),
                "{what}: every node has something at its foot, a pad if nothing else"
            );
            if node.def.is_output && !node.collapsed {
                assert_eq!(
                    thumb_band(g, id).max.y,
                    l.rect.max.y,
                    "{what}: an Output's render stays flush at the foot"
                );
            }
        });
    }
}
