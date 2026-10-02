// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `s-number` scrub control: decrement, a draggable value, increment, and a
//! value-proportional fill behind the text.
//!
//! Dragging the value is the primary gesture — a slider you can also type exact numbers
//! into, rather than a slider or a field. Shift is a fine step, Ctrl a coarse one.
//!
//! Every other gesture is here because a scrub alone cannot say certain things: clicking the
//! value types an exact one, `[` and `]` go to the ends, `D` returns the definition's default,
//! `R` returns the default *and* the definition's range, and right-click opens the range
//! editor. The last two differ only because a range belongs to the instance — which is the
//! whole reason `R` exists separately from `D`.
//!
//! **Right-click, where silvia uses `Ctrl` + hover.** Ctrl is already the coarse multiplier
//! here, so a hand holding it to scrub in tens would have the range editor open under the
//! pointer the whole time. Right-click is the gesture for *this object's context*, which is
//! exactly what a range is, and unlike a modifier held over a hover it can be found. An
//! ordinary hover still answers "how far can this go": the tooltip carries the range beside
//! the value, which is what silvia's `Ctrl` + hover actually bought.
//!
//! **The text is the value and the rest of the bar is the track.** A drag that begins on the
//! number scrubs from where the number stands; one that begins on the bar jumps to where it
//! landed and scrubs on from there. The split is a drag's, not a click's — clicking anywhere
//! but a stepper still opens the typed entry, because there is nowhere else to put it.

use crate::graph::ControlRange;
use crate::nodes::NumberField;
use crate::nodes::gear::ladder;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align2, Color32, CornerRadius, FontId, Key, Pos2, Rect, Response, Sense, Stroke, TextEdit, Ui,
    vec2,
};

/// Design system: `.s-number` is 100x25; `snumber.js`'s own markup insets the slider track
/// `left: 19px; right: 19px`, which is where a cap's own width comes from.
pub const WIDTH: f32 = 100.0;
pub const HEIGHT: f32 = 25.0;
const BUTTON_WIDTH: f32 = 19.0;

/// How far the pointer travels for one step.
const PIXELS_PER_STEP: f32 = 4.0;

/// The finest step `Ctrl` + `−` will dial in, unless the definition already declares a finer
/// one. `decimals` tops out at three places, so anything smaller changes a readout that
/// cannot show it.
const STEP_FLOOR: f32 = 0.001;

/// How far either side of the number counts as the number, so a one-digit value is still
/// something a drag can grab.
const VALUE_PAD: f32 = 4.0;

// Four independent answers about one control — log, varying, learning, ladder — each read in
// its own place; an enum of their combinations would name sixteen states nothing asks for.
#[allow(clippy::struct_excessive_bools)]
pub struct NumberSpec {
    pub value: f32,
    /// What the definition says this control is when nothing has been dialed: `D`.
    pub default: f32,
    /// The ends and quantum this control actually has — the instance's where it has one.
    pub range: ControlRange,
    /// What the definition declares, which is what `R` puts back and what the range editor
    /// may not exceed.
    pub declared: ControlRange,
    pub unit: &'static str,
    /// Scrub in log space. A zoom of 0.01 to 100 spends its whole linear travel below 1.
    pub log: bool,
    /// A **varying** value is arriving here, so there is no one value to show and `value` is
    /// a number nobody is reading. The control says [`VARYING`] in place of its digits and
    /// draws no fill — see [The value on the row](../../docs/ui.md#the-value-on-the-row).
    pub varying: bool,
    /// This control is waiting for a MIDI message: it wears [`crate::ui::learning_ring`] and
    /// its accessible name ends in [`crate::ui::LEARNING`].
    pub learning: bool,
    /// A Ratio Gear's **ratio**, drawn and walked on [`ladder`]: the value is the ratio
    /// itself, written `×3`, `÷4` or `3/2`; a drag, the steppers, the arrows and the wheel
    /// climb the ladder a rung at a time, through ×0 into reverse, `[` and `]` go to its ends,
    /// and a typed `×5`, `÷7`, `3/2`, `0.3` or `-×1` is taken as it is. It has no range
    /// editor. False for every other number.
    pub ladder: bool,
    /// Where a bound fader is, in this control's own units, while soft takeover holds it out
    /// of pick-up: a ghost mark across the trough at that place, inside the control's own
    /// size, says where to steer it. Its accessible name says it too.
    pub ghost: Option<f32>,
}

/// What a control fed by a value that varies per pixel says where its number would go.
///
/// The rate's own name, and the word the port under the cable wears: the dot beside such a
/// control reads *varying number input*.
pub const VARYING: &str = "varying";

/// What a gesture on the control asked for. One per frame: a control cannot be scrubbed and
/// reset in the same one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumberAction {
    /// A new value.
    Set(f32),
    /// `R`: the value *and* the range back to what the definition declares. Two commands, one
    /// gesture, and the reason it is not `D`.
    ResetAll,
    /// `Ctrl` + a stepper: this control's quantum, ten times coarser or finer. A range, so
    /// it is document data rather than a scrub.
    SetStep(f32),
    /// `Escape` mid-drag: the value the drag began at. Not another edit — the step the drag
    /// opened collapses back to it, which is why it is not a `Set`.
    Cancel(f32),
    /// Right-click: show the range editor, because the ends themselves are what needs
    /// changing.
    OpenRange,
    /// `Alt` + click: learn a MIDI binding for this control. silvia's own gesture, and the
    /// reason it is a modifier rather than a menu — a control is bound while looking at it.
    Learn,
}

/// How much drag traverses the whole range, for a log control.
const LOG_STEPS_ACROSS: f32 = 240.0;

impl NumberSpec {
    fn min(&self) -> f32 {
        self.range.min
    }

    fn max(&self) -> f32 {
        self.range.max
    }

    fn step(&self) -> f32 {
        self.range.step
    }

    fn quantize(&self, v: f32) -> f32 {
        if self.ladder {
            return v.clamp(self.min(), self.max());
        }
        let v = if self.step() > 0.0 {
            (v / self.step()).round() * self.step()
        } else {
            v
        };
        v.clamp(self.min(), self.max())
    }

    /// `ln min` and the span from it to `ln max`, where a log scrub is meaningful: the range
    /// must be positive.
    fn log_span(&self) -> Option<(f32, f32)> {
        (self.log && self.min() > 0.0 && self.max() > self.min())
            .then(|| (self.min().ln(), self.max().ln() - self.min().ln()))
    }

    /// 0..1 position within the range, for the fill behind the text.
    fn fraction(&self) -> f32 {
        // A value that varies has no position on the track. Leaving the stale number's fill
        // there would be the control lying a second time, under a word saying it is not a
        // number.
        if self.varying {
            return 0.0;
        }
        self.fraction_of(self.value)
    }

    /// 0..1 position of any value within the range: the fill's for the value, the ghost
    /// mark's for a fader.
    fn fraction_of(&self, value: f32) -> f32 {
        if self.max() <= self.min() {
            return 0.0;
        }
        if self.ladder {
            let at = ladder::nearest(f64::from(value)) + ladder::TOP;
            return (at as f32 / (2 * ladder::TOP) as f32).clamp(0.0, 1.0);
        }
        if let Some((low, span)) = self.log_span() {
            return ((value.max(self.min()).ln() - low) / span).clamp(0.0, 1.0);
        }
        ((value - self.min()) / (self.max() - self.min())).clamp(0.0, 1.0)
    }

    /// Advance by `steps` of drag, linearly or in log space.
    fn advance(&self, steps: f32, multiplier: f32) -> f32 {
        if self.ladder {
            // A rung at a time, from wherever a typed ratio sits nearest.
            let at = ladder::nearest(f64::from(self.value));
            let by = (steps * multiplier).round() as i32;
            return ladder::at(at + by) as f32;
        }
        if self.log_span().is_some() {
            return self.at_fraction(self.fraction() + steps * multiplier / LOG_STEPS_ACROSS);
        }
        self.quantize(self.value + steps * self.step() * multiplier)
    }

    /// The value at a 0..1 position along the bar: the inverse of `fraction`, so a jump lands
    /// where the fill's edge is drawn.
    fn at_fraction(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if self.ladder {
            let rung = (t * (2 * ladder::TOP) as f32).round() as i32 - ladder::TOP;
            return ladder::at(rung) as f32;
        }
        if let Some((low, span)) = self.log_span() {
            return (low + t * span).exp().clamp(self.min(), self.max());
        }
        self.quantize(self.min() + t * (self.max() - self.min()))
    }

    /// The finest a quantum may be: what the readout can show, or what the definition itself
    /// declares if that is finer. Read by the steppers' `Ctrl` and by the range editor's own
    /// step field, which must agree about how small is too small.
    fn step_floor(&self) -> f32 {
        STEP_FLOOR.min(self.declared.step).max(f32::MIN_POSITIVE)
    }

    /// This control's quantum, scaled and kept usable: never below what the readout can show
    /// or what the definition itself declares, and never above the whole span, which would
    /// leave the control with one position.
    fn scaled_step(&self, factor: f32) -> f32 {
        let floor = self.step_floor();
        let ceiling = (self.max() - self.min()).max(floor);
        (self.step() * factor).clamp(floor, ceiling)
    }

    /// Enough places to show one step. A control stepping by 0.001 shown to two places has
    /// a readout that does not move when it changes.
    fn decimals(&self) -> usize {
        if self.step() >= 1.0 {
            0
        } else if self.step() >= 0.1 {
            1
        } else if self.step() >= 0.01 {
            2
        } else {
            3
        }
    }

    /// Any number at this control's own precision, so a readout, a typed buffer and a
    /// default shown beside a field cannot disagree about how many places to give it.
    fn shown(&self, v: f32) -> String {
        if self.ladder {
            return ladder::label(f64::from(v));
        }
        format!("{:.*}", self.decimals(), v)
    }

    /// The value as text, for the readout and for seeding a typed edit.
    fn text(&self) -> String {
        self.shown(self.value)
    }

    /// Where `[` and `]` go: the control's ends, or the ladder's.
    fn ends(&self) -> (f32, f32) {
        if self.ladder {
            (
                ladder::at(-ladder::TOP) as f32,
                ladder::at(ladder::TOP) as f32,
            )
        } else {
            (self.min(), self.max())
        }
    }

    /// A typed value as this control reads it: a number, or on the ladder `×n`, `÷n`, `p/q`
    /// and a sign. `None` leaves the value alone.
    fn typed(&self, text: &str) -> Option<f32> {
        let v = if self.ladder {
            ladder::parse(text)? as f32
        } else {
            text.trim().parse::<f32>().ok().filter(|v| v.is_finite())?
        };
        Some(self.quantize(v))
    }

    /// The value and its unit, for anywhere the readout is shown whole: `10.0 /⬓`. A space
    /// separates them — run together, `10.0/⬓` reads as a fraction rather than ten per
    /// half-height — and a control with no unit leaves nothing to add one to.
    fn with_unit(&self) -> String {
        if self.varying {
            return VARYING.to_string();
        }
        if self.unit.is_empty() {
            self.text()
        } else {
            format!("{} {}", self.text(), self.unit)
        }
    }
}

/// What `chrome` drew and measured, for the caller to paint the value into and to read for
/// its own hit-testing.
struct Chrome {
    /// The room between the two caps, where the value itself lives — a plain scrub's number
    /// is centerd in the whole control, but a typed entry's field sits here specifically, so
    /// it does not draw over them.
    trough: Rect,
    /// The caps' targets, or [`Rect::NOTHING`] where the control is too narrow to carry
    /// them — so "is there a cap here" and "is there a cap at all" are the same question.
    dec_rect: Rect,
    inc_rect: Rect,
}

/// Points sampled evenly around an arc, both ends included — the corner geometry
/// `bevel_border` strokes each side from. Angles are degrees in screen space (0° = +x/right,
/// 90° = +y/down), so `0..90` is the arc a bottom-right corner sweeps. `r <= 0` collapses to
/// the corner point itself, which is what a control too small for its own radius should draw.
/// Appended rather than returned, so one polyline is one allocation: the six arcs below are
/// two boundary walks, not six lines.
fn arc_points(out: &mut Vec<Pos2>, center: Pos2, r: f32, start_deg: f32, end_deg: f32) {
    if r <= 0.0 {
        out.push(center);
        return;
    }
    let step_deg = 15.0;
    let n = (((end_deg - start_deg).abs() / step_deg).ceil() as usize).max(1);
    out.extend((0..=n).map(|i| {
        let t = i as f32 / n as f32;
        let rad = (start_deg + (end_deg - start_deg) * t).to_radians();
        center + vec2(r * rad.cos(), r * rad.sin())
    }));
}

/// The inset bevel a recessed field wears: silvia's own `2px inset border-normal`, which CSS
/// resolves per *side* rather than as a blend — the whole top and left edges are one dark
/// tone, the whole bottom and right are one light tone, and the two only meet at the 45° apex
/// of the two corners where a dark side turns into a light one (top-right, bottom-left); the
/// other two corners (top-left, bottom-right) are a single color all the way round, because
/// both of *their* sides already agree. `border_normal`, darkened further so the two sides
/// read apart at a glance, is the dark tone; `text_muted` — already brighter than any border
/// token on the ladder — is the light one. Both are existing theme tokens; nothing here is new
/// color.
///
/// **Two open paths, not one closed stroke wearing a clip.** A first version stroked the whole
/// rounded rect twice and clipped the light pass to an axis-aligned quadrant through the
/// center — which put the color change halfway along the bottom and right edges instead of
/// at the corners, since egui has no rotated or arbitrary clip to cut a rounded rect on its
/// own diagonal. This instead walks the rounded rect's own boundary as two arcs-and-edges
/// polylines, split at the exact 45° point of the top-right and bottom-left corner arcs, and
/// strokes each polyline once in its own color — so the geometry itself carries the miter,
/// nothing is drawn past where a side should stop, and there is no seam to align by hand.
/// Shared by the s-number's own chrome and the range editor's fields, so a value reads as the
/// same recessed thing wherever it is edited.
pub(crate) fn bevel_border(painter: &eframe::egui::Painter, rect: Rect, zoom: f32, theme: &Theme) {
    // Neither the radius nor the width is the caller's to choose: a bevel that is not this
    // bevel is not the same recessed thing, which is the whole point of one function.
    let radius = f32::from(theme::RADIUS_SM);
    let width = (1.5 * zoom).max(1.0);
    let dark = theme.border_normal().gamma_multiply(0.4);
    let light = theme.text_muted();

    // The stroke's own centerline: inset half a width from the rect silvia's `border-radius`
    // describes, same as `StrokeKind::Inside` would place it, with the corner radius shrunk to
    // match so the arc still meets the straight edges tangent.
    let half = width * 0.5;
    let cl = rect.shrink(half);
    let cl_r = (radius - half)
        .max(0.0)
        .min(cl.width().min(cl.height()) * 0.5);

    let tl = cl.min + vec2(cl_r, cl_r);
    let tr = Pos2::new(cl.max.x - cl_r, cl.min.y + cl_r);
    let br = cl.max - vec2(cl_r, cl_r);
    let bl = Pos2::new(cl.min.x + cl_r, cl.max.y - cl_r);

    // Light: from the top-right corner's 45° apex, down the right edge, around the
    // bottom-right corner (whole, since both its sides are light), along the bottom edge, to
    // the bottom-left corner's own 45° apex.
    let mut light_pts = Vec::new();
    arc_points(&mut light_pts, tr, cl_r, 315.0, 360.0);
    arc_points(&mut light_pts, br, cl_r, 0.0, 90.0);
    arc_points(&mut light_pts, bl, cl_r, 90.0, 135.0);

    // Dark: the other half of the same boundary, starting where light left off.
    let mut dark_pts = Vec::new();
    arc_points(&mut dark_pts, bl, cl_r, 135.0, 180.0);
    arc_points(&mut dark_pts, tl, cl_r, 180.0, 270.0);
    arc_points(&mut dark_pts, tr, cl_r, 270.0, 315.0);

    painter.line(dark_pts, Stroke::new(width, dark));
    painter.line(light_pts, Stroke::new(width, light));
}

/// The font the value is set in, wherever it is drawn: the readout, the caps' glyphs, and the
/// field a typed edit opens. One function, because the readout and the field must sit on the
/// same baseline — a field whose text is a hair larger makes the number jump the moment it is
/// clicked, which is the one frame the eye is already on it.
fn value_font(zoom: f32) -> FontId {
    FontId::monospace(theme::font_size(theme::FONT_BASE * 0.9, zoom))
}

/// The control's own chrome — background, trough fill, caps, bevel, and the accent border a
/// narrowed range wears — everything but the value itself.
///
/// Shared by `scrub`, which paints the value as text, and `typed_entry`, which paints it as a
/// field: the whole point of splitting this out is that the two must draw *identically* here,
/// so clicking a control to type into it does not change its shape.
fn chrome(
    ui: &Ui,
    rect: Rect,
    spec: &NumberSpec,
    theme: &Theme,
    enabled: bool,
    zoom: f32,
) -> Chrome {
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme.bg_interactive());

    // Whether the steppers have room, decided before either the fill or the caps are drawn:
    // the fill's trough is confined between them exactly as silvia's `left:19px; right:19px`
    // wrapper confines `.s-number-slider`, so this has to be known first.
    let steppers = rect.width() > BUTTON_WIDTH * 3.0 * zoom;
    let inset = if steppers { BUTTON_WIDTH * zoom } else { 0.0 };
    // The caps' own rects say whether there are caps: too narrow, and they are `NOTHING`,
    // which contains no point, so a hit test needs no second condition beside it.
    let (dec_rect, inc_rect) = if steppers {
        (
            rect.with_max_x(rect.min.x + inset),
            rect.with_min_x(rect.max.x - inset),
        )
    } else {
        (Rect::NOTHING, Rect::NOTHING)
    };
    let trough = rect.shrink2(vec2(inset, 0.0));

    // The fill sits behind the text and shows where in its range the value is, confined to
    // the trough between the two caps and square-cornered on every side of its own — the
    // caps carry the control's rounding, and a fill rounded to match them reads as a second,
    // smaller pill floating inside the first one rather than a flat trough filling up.
    let fraction = spec.fraction();
    if fraction > 0.0 {
        let mut fill = trough;
        fill.set_width(trough.width() * fraction);
        // A connected control's is the uniform number hue and quieter, so the ground says
        // which of
        // the two things this bar is before the number is read: cyan is a knob, violet is a
        // meter.
        let ground = if enabled {
            theme.primary().gamma_multiply(0.2)
        } else {
            theme.readout().gamma_multiply(0.15)
        };
        painter.rect_filled(fill, 0, ground);
    }

    if steppers {
        let font = value_font(zoom);
        // The steppers carry the disabled state. They are the parts a hand reaches for, so
        // an inert control says so where the pressing would happen rather than by making
        // its number hard to read.
        let (button, glyph_color) = if enabled {
            (theme.bg_tertiary(), theme.text_secondary())
        } else {
            (
                theme.bg_tertiary().gamma_multiply(0.4),
                theme.text_disabled(),
            )
        };
        // Rounded on the outer corner only, square where each meets the trough: the caps are
        // part of one pill-shaped control, not two small buttons floating beside it.
        let cap = theme::RADIUS_SM;
        for (r, cap_radius, glyph) in [
            (
                dec_rect,
                CornerRadius {
                    nw: cap,
                    ne: 0,
                    sw: cap,
                    se: 0,
                },
                "-",
            ),
            (
                inc_rect,
                CornerRadius {
                    nw: 0,
                    ne: cap,
                    sw: 0,
                    se: cap,
                },
                "+",
            ),
        ] {
            painter.rect_filled(r, cap_radius, button);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                glyph,
                font.clone(),
                glyph_color,
            );
        }
    }

    // Where a fader out of pick-up is: a hairline across the trough in the accent, short of
    // its top and bottom so the number's own digits stay readable over it.
    if let Some(ghost) = spec.ghost.filter(|_| !spec.varying) {
        let x = trough.min.x + trough.width() * spec.fraction_of(ghost);
        let inset = 3.0 * zoom;
        painter.line_segment(
            [
                Pos2::new(x, trough.min.y + inset),
                Pos2::new(x, trough.max.y - inset),
            ],
            Stroke::new((1.5 * zoom).max(1.0), theme.accent()),
        );
    }

    bevel_border(painter, rect, zoom, theme);

    // A range this instance chose is said, not merely felt: the track is not the one the
    // definition draws, and nothing else on the node would show that.
    if spec.range != spec.declared {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme.accent().gamma_multiply(0.7)),
            eframe::egui::StrokeKind::Inside,
        );
    }
    if spec.learning {
        crate::ui::learning_ring(ui, rect, zoom, theme);
    }

    Chrome {
        trough,
        dec_rect,
        inc_rect,
    }
}

/// Draw the control. Returns the new value when the user changed it.
///
/// `enabled` is false when the input is connected, and the caller passes the arriving value
/// in `spec`: the control is a meter of what the node sees rather than a knob, so the number
/// is drawn at a readout's weight and the fill and the steppers are what go quiet.
// Every argument is used; splitting it would only move the list into a struct built at the
// single call site.
#[allow(clippy::too_many_arguments)]
pub fn scrub(
    ui: &mut Ui,
    rect: Rect,
    label: impl crate::ui::Name,
    spec: &NumberSpec,
    theme: &Theme,
    enabled: bool,
    // The `lock_cursor_while_scrubbing` preference: grab and hide the pointer on
    // `drag_started`, release it on `drag_stopped`, and read the raw motion `MouseMoved`
    // delivers instead of `Response::drag_delta`, which stops moving once the OS pins the
    // cursor in place.
    lock_cursor: bool,
    zoom: f32,
) -> Option<NumberAction> {
    let edit_id = ui.id().with(("num-edit", &label));
    // `Alt` is the MIDI learn gesture, and click-to-type is on the same pixels. Held, the
    // typed editor is not offered at all — otherwise an `Alt` click on the value opens the
    // keyboard entry and the binding never happens.
    let alt = ui.ctx().input(|i| i.modifiers.alt);
    // Typing takes over the value only: the chrome — background, trough, caps, bevel — is
    // the same `chrome` call this draws below, so clicking to type does not swap the control
    // for a bare text box. No scrub, key or wheel besides the value's own is read while it
    // has the keyboard.
    if enabled && !alt {
        match typed_entry(ui, rect, edit_id, spec, theme, zoom) {
            Typing::Editing(action) => return action,
            Typing::Idle => {}
        }
    }

    let Chrome {
        dec_rect, inc_rect, ..
    } = chrome(ui, rect, spec, theme, enabled, zoom);
    let painter = ui.painter();

    // A connected control's number is the value arriving down the cable, so it is drawn as
    // the readouts on the output rows are: the uniform number hue, at full weight. Dimming
    // it is
    // what a stale number deserved; this one is the only place the arriving value is
    // written, and the inertness is said by the fill and the steppers instead.
    let text_color = if enabled {
        theme.text_primary()
    } else {
        theme.readout()
    };
    // Where the number sits is where the value is, and everything else on the bar is track.
    // The rect is measured from the shown text, so a unit widens it along with the number —
    // there is no separate fit to fall out of step and clip the unit off, as silvia's does.
    let value_rect = painter
        .text(
            rect.center(),
            Align2::CENTER_CENTER,
            spec.with_unit(),
            value_font(zoom),
            text_color,
        )
        .expand2(vec2(VALUE_PAD * zoom, 0.0));

    // Inert when connected, but still a widget in the tree, so tests and the agent can read
    // the value.
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response: Response = ui.interact(rect, ui.id().with(("num", &label)), sense);
    describe(&response, &label, spec);
    // The range, reachable from a hover: silvia's `Ctrl` + hover bought the same answer to
    // "how far can this go", and a tooltip closes that gap without spending a click on the
    // range editor. Built inside the closure, not handed to `on_hover_text`, which would
    // format three strings every frame for every control to show one of them on hover.
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.add(eframe::egui::Label::new(hover_text(&label, spec)));
    });
    if !enabled {
        return None;
    }
    if response.hovered() {
        mark_hovered(ui.ctx());
    }

    let modifiers = ui.ctx().input(|i| i.modifiers);
    let multiplier = multiplier(modifiers);

    // `Alt` + click learns a MIDI binding, and consumes the click. Checked before the drag
    // and the steppers: an `Alt` drag would otherwise scrub the value on the way to binding
    // it, and the control would end up somewhere nobody asked for.
    if alt && (response.clicked() || response.drag_started()) {
        return Some(NumberAction::Learn);
    }

    if response.drag_started() {
        ui.data_mut(|d| {
            d.insert_temp(
                response.id,
                Drag {
                    start: spec.value,
                    carried: 0.0,
                    fine: modifiers.shift,
                },
            );
        });
        // Grabbed and hidden, so the scrub is the pointer's raw motion rather than its
        // position — there is no edge of the screen to run out of. `send_viewport_cmd` never
        // panics on a platform that refuses the grab; it logs a warning and the drag falls
        // back to reading the absolute position below, same as with the preference off.
        if lock_cursor {
            let ctx = ui.ctx();
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CursorGrab(
                eframe::egui::viewport::CursorGrab::Locked,
            ));
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CursorVisible(false));
        }
        // A drag that began on the track jumps to where it landed, and scrubs on from there.
        // The press is what the split is read from, not the pointer where the drag was
        // decided: a scrub that started on the number stays a scrub once it leaves it.
        if let Some(pos) = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos())
            && !value_rect.contains(pos)
            && !dec_rect.contains(pos)
            && !inc_rect.contains(pos)
        {
            let next = spec.at_fraction((pos.x - rect.left()) / rect.width());
            if next != spec.value {
                return Some(NumberAction::Set(next));
            }
        }
    }

    // Read from the marker rather than from `Response::dragged`: egui ends a drag of its own
    // accord on Escape, so by the time the widget runs the response says only that the drag
    // stopped. That also means the rest of the press scrubs nothing, which is what it should
    // do — a drag put back is not a drag to resume.
    let dragging: Option<Drag> = ui.data_mut(|d| d.get_temp(response.id));
    if let Some(drag) = dragging
        // Consumed like the other keys, so abandoning a scrub does not also close a popup.
        && ui
            .ctx()
            .input_mut(|i| i.consume_key(eframe::egui::Modifiers::NONE, Key::Escape))
    {
        ui.data_mut(|d| d.remove::<Drag>(response.id));
        release_cursor(ui.ctx(), lock_cursor);
        return Some(NumberAction::Cancel(drag.start));
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<Drag>(response.id));
        release_cursor(ui.ctx(), lock_cursor);
    }

    if response.dragged()
        && let Some(mut drag) = dragging
    {
        // The carried remainder is dropped when Shift is taken or released mid-drag. Without
        // this, a tenth-of-a-step remainder accumulated in fine mode is spent at ten times the
        // size the moment Shift comes off, and the value jumps exactly when the point of fine
        // mode was that it should not.
        if drag.fine != modifiers.shift {
            drag.carried = 0.0;
            drag.fine = modifiers.shift;
        }

        // Locked, the cursor's absolute position stops changing — the OS pins it where the
        // grab engaged — so `Response::drag_delta`, which is a position delta, reads zero.
        // `i.motion()` is the raw relative motion `DeviceEvent::MouseMotion` reports, which
        // eframe forwards regardless of grab state: with a button held or the window
        // focused, it arrives every frame whether or not the cursor itself is locked.
        let raw_x = if lock_cursor {
            ui.ctx().input(|i| i.pointer.motion()).map_or(0.0, |m| m.x)
        } else {
            response.drag_delta().x
        };
        let delta = raw_x / (PIXELS_PER_STEP * zoom) * multiplier;
        let mut next = None;
        if delta != 0.0 {
            // Carried across frames. A step smaller than the control's quantum rounds away
            // in `advance`, so without this a Shift drag — a tenth of a step per 4 px — moves
            // nothing at all rather than moving finely.
            let steps = drag.carried + delta;
            let to = spec.advance(steps, 1.0);
            if to == spec.value {
                // Bounded, so that dragging past a clamp does not have to be dragged all the
                // way back before the value moves again.
                drag.carried = steps.clamp(-1.0, 1.0);
            } else {
                drag.carried = 0.0;
                next = Some(NumberAction::Set(to));
            }
        }
        ui.data_mut(|d| d.insert_temp(response.id, drag));
        if next.is_some() {
            return next;
        }
    }

    // Right-click asks about the control rather than about the value: where its ends are. A
    // ratio's ends are its ladder's, which nothing edits.
    if response.secondary_clicked() && !spec.ladder {
        return Some(NumberAction::OpenRange);
    }

    if response.clicked() {
        // A stepper if the click landed on one, and the value otherwise. One block, because
        // the position is not always known — kittest reports a click with no pointer position
        // — and "somewhere on the control that is not a stepper" is what typing means.
        if let Some(pos) = response.interact_pointer_pos() {
            // Held on a stepper, Ctrl changes the quantum rather than multiplying the move:
            // the two live on the same button because coarser steps and a coarser step size
            // are the same wish.
            let quantum = (modifiers.command || modifiers.ctrl) && !spec.ladder;
            if dec_rect.contains(pos) {
                return Some(if quantum {
                    NumberAction::SetStep(spec.scaled_step(0.1))
                } else {
                    NumberAction::Set(spec.advance(-1.0, multiplier))
                });
            }
            if inc_rect.contains(pos) {
                return Some(if quantum {
                    NumberAction::SetStep(spec.scaled_step(10.0))
                } else {
                    NumberAction::Set(spec.advance(1.0, multiplier))
                });
            }
        }

        // Clicking the value itself types it.
        // The buffer is the whole state: the field exists next frame, and *that* is where
        // focus is asked for. Requesting it here names an id no widget has yet, which
        // AccessKit reports as a focused node missing from the tree.
        ui.data_mut(|d| {
            d.insert_temp(edit_id, spec.text());
            d.insert_temp(edit_id.with("fresh"), true);
        });
        return None;
    }

    if response.hovered() {
        // Taken, not read. The canvas zooms on whatever wheel input is left at the end of
        // the frame, so a notch that scrubs this control must not also zoom the canvas.
        let scroll = ui.ctx().input_mut(|i| {
            let v = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta.y = 0.0;
            v
        });
        if scroll != 0.0 {
            let next = spec.advance(scroll.signum(), multiplier);
            if next != spec.value {
                return Some(NumberAction::Set(next));
            }
        }

        // Keys, while the pointer is over the control. Hover rather than focus, because the
        // canvas has no focus to give: a node is not a widget tree.
        if let Some(action) = keys(ui, spec, multiplier) {
            return Some(action);
        }
    }

    None
}

/// Where the pass a control was last under the pointer in is kept.
const HOVERED: &str = "number-hovered";

/// Remember that a control is under the pointer in this pass.
fn mark_hovered(ctx: &eframe::egui::Context) {
    let frame = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(eframe::egui::Id::new(HOVERED), frame));
}

/// Whether an enabled number control has been under the pointer in this pass: the canvas's
/// own keys yield to it, since its keys are read while it is hovered.
pub fn hovered_this_pass(ctx: &eframe::egui::Context) -> bool {
    let frame = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(eframe::egui::Id::new(HOVERED))) == Some(frame)
}

/// A scrub in progress, from the frame it starts to the frame it stops or is abandoned.
#[derive(Clone, Copy)]
struct Drag {
    /// The value it began at, which `Escape` restores.
    start: f32,
    /// The part of a step dragged and not yet spent, bounded to one either way.
    carried: f32,
    /// Whether `Shift` was held, the last frame it was read.
    fine: bool,
}

/// Shift is a fine step, Ctrl a coarse one — the module's own rule, in one place, because a
/// hovered control and a focused field both answer to it.
fn multiplier(modifiers: eframe::egui::Modifiers) -> f32 {
    if modifiers.shift {
        0.1
    } else if modifiers.command || modifiers.ctrl {
        10.0
    } else {
        1.0
    }
}

/// Up and down step the value: the gesture a hovered control and a focused typing field
/// share, so neither can drift from the other.
///
/// Consumed, not read, so a key that moved a control does not also reach the canvas. The
/// arrows carry the wheel's multipliers — one key press is one notch — and the pattern is
/// whatever is held, because egui's `NONE` rejects a held Ctrl outright and Ctrl is one of
/// the multipliers.
fn arrow_step(ui: &Ui, spec: &NumberSpec, multiplier: f32) -> Option<f32> {
    ui.ctx().input_mut(|i| {
        let held = i.modifiers;
        if i.consume_key(held, Key::ArrowUp) {
            return Some(spec.advance(1.0, multiplier));
        }
        if i.consume_key(held, Key::ArrowDown) {
            return Some(spec.advance(-1.0, multiplier));
        }
        None
    })
}

/// The keys a hovered control answers to.
fn keys(ui: &Ui, spec: &NumberSpec, multiplier: f32) -> Option<NumberAction> {
    if let Some(next) = arrow_step(ui, spec, multiplier) {
        return Some(NumberAction::Set(next));
    }
    ui.ctx().input_mut(|i| {
        // Consumed, not read, so a key that moved a control does not also reach the canvas.
        if i.consume_key(eframe::egui::Modifiers::NONE, Key::D) {
            return Some(NumberAction::Set(spec.default));
        }
        if i.consume_key(eframe::egui::Modifiers::NONE, Key::R) {
            return Some(NumberAction::ResetAll);
        }
        let (low, high) = spec.ends();
        if i.consume_key(eframe::egui::Modifiers::NONE, Key::OpenBracket) {
            return Some(NumberAction::Set(low));
        }
        if i.consume_key(eframe::egui::Modifiers::NONE, Key::CloseBracket) {
            return Some(NumberAction::Set(high));
        }
        None
    })
}

/// Whether an edit took the frame, and what it committed if it ended.
enum Typing {
    /// No edit is open; the caller draws the scrub as usual.
    Idle,
    /// The field owns this frame. `Some` when it committed a value.
    Editing(Option<NumberAction>),
}

/// The control as a text field, while one is open on it.
///
/// **The chrome stays.** Only the value, in the trough between the caps, becomes an editable
/// field — the background, the caps and the bevel are the same `chrome` call `scrub` makes,
/// so clicking a control to type into it does not swap it for a bare text box.
///
/// **Up and down step, rather than moving a cursor there is nowhere to move it to.** silvia's
/// own `keydown` handler treats them as the stepper's gesture even while the field has focus;
/// a single line has no second line for them to move the cursor between anyway. Left and
/// right are untouched, and still move the cursor through the digits.
fn typed_entry(
    ui: &mut Ui,
    rect: Rect,
    edit_id: eframe::egui::Id,
    spec: &NumberSpec,
    theme: &Theme,
    zoom: f32,
) -> Typing {
    let Some(mut text) = ui.data_mut(|d| d.get_temp::<String>(edit_id)) else {
        return Typing::Idle;
    };

    let trough = chrome(ui, rect, spec, theme, true, zoom).trough;

    let multiplier = multiplier(ui.ctx().input(|i| i.modifiers));
    if let Some(next) = arrow_step(ui, spec, multiplier) {
        // The field stays open and focused — only the buffer changes, to the new value's own
        // text, so the next press steps from what is now showing rather than from what was
        // last typed and not yet committed.
        ui.data_mut(|d| d.insert_temp(edit_id, spec.shown(next)));
        return Typing::Editing(Some(NumberAction::Set(next)));
    }

    let font = value_font(zoom);
    // Padded to put the text where the scrub draws it. A field whose baseline sits above the
    // readout's makes the number jump the moment it is clicked, which is the one frame the
    // eye is already on it.
    let row = ui.ctx().fonts_mut(|f| f.row_height(&font));
    let pad = ((trough.height() - row) * 0.5).max(0.0);
    let output = ui
        .scope_builder(eframe::egui::UiBuilder::new().max_rect(trough), |ui| {
            TextEdit::singleline(&mut text)
                .id(edit_id)
                .font(font)
                .horizontal_align(eframe::egui::Align::Center)
                .text_color(theme.text_primary())
                .background_color(Color32::TRANSPARENT)
                .margin(vec2(2.0, pad))
                .desired_width(trough.width())
                .show(ui)
        })
        .inner;
    let response = output.response;

    // The frame the field first appears on is the frame it takes the keyboard — and the frame
    // it selects what is in it, so typing replaces the number rather than appending to it.
    if ui.data_mut(|d| d.get_temp::<bool>(edit_id.with("fresh")).unwrap_or(false)) {
        response.request_focus();
        ui.data_mut(|d| d.remove::<bool>(edit_id.with("fresh")));
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(eframe::egui::text::CCursorRange::two(
                eframe::egui::text::CCursor::new(0),
                eframe::egui::text::CCursor::new(text.chars().count()),
            )));
        state.store(ui.ctx(), edit_id);
    }

    let committed = response.lost_focus() && ui.ctx().input(|i| i.key_pressed(Key::Enter));
    let canceled = ui.ctx().input(|i| i.key_pressed(Key::Escape));
    if canceled {
        // Handed back before the field disappears, or the keyboard would belong to a widget
        // that is no longer drawn.
        response.surrender_focus();
        ui.data_mut(|d| d.remove::<String>(edit_id));
        return Typing::Editing(None);
    }
    // Blur commits, as Enter does: a click elsewhere means the number that is showing.
    if committed || response.lost_focus() {
        response.surrender_focus();
        ui.data_mut(|d| d.remove::<String>(edit_id));
        // Anything unparseable leaves the value alone rather than snapping it to zero.
        return Typing::Editing(spec.typed(&text).map(NumberAction::Set));
    }
    ui.data_mut(|d| d.insert_temp(edit_id, text));
    Typing::Editing(None)
}

/// What the range editor asked for. `range` and `value` are independent — dragging Min does
/// not also touch the stored value, and typing a new Value does not touch the range — so the
/// caller issues whichever commands the popup actually reports this frame. `reset` is its own
/// gesture rather than a row: it clears the instance's range entirely, where a row's own
/// default only sets that one field to it.
// Four independent answers the popup can give in one frame — a range moved, a value typed,
// a reset, a binding asked for or forgotten — not a state machine with one outcome.
#[allow(clippy::struct_excessive_bools)]
pub struct RangeEdit {
    pub range: Option<ControlRange>,
    pub value: Option<f32>,
    pub reset: bool,
    /// Bind this control to the next MIDI message. The same thing `Alt` + click does, from
    /// the menu a right-click already opens — which is where a gesture nobody has been told
    /// about is findable.
    pub learn: bool,
    /// Forget whatever drives it.
    pub unbind: bool,
    pub dismissed: bool,
}

/// One row's worth of edit, folded into `out` by the caller: the field the user can change,
/// and its default beside it — a plain label, clicking it copies the default into the field.
struct Row<'a> {
    name: &'static str,
    field: &'a mut f32,
    default: f32,
    bounds: NumberField,
}

/// What the range editor's MIDI row offers.
#[derive(Debug, Clone, Copy)]
pub enum MidiRow<'a> {
    /// The trigger that drives this control, beside an **Unbind**.
    Bound(&'a str),
    /// **Bind MIDI…**.
    Unbound,
    /// No row at all: a control MIDI does not bind, an Output's render numbers.
    Unbindable,
}

/// The range editor: the numbers that belong to *this* control rather than to its kind, laid
/// out as a small table — a row per number, a plain number field and its default side by
/// side. The fields are typed, not dragged: an end is a number somebody means, and a drag
/// on a field this small is a number nobody meant.
///
/// Drawn as a popup through the same path the color picker and the select use, so it lands
/// above every node however late that node was painted, in the node chrome's own font and
/// the theme's own inset-field colors, at the width its content asks for rather than a
/// fixed one — the same discipline a select's row already keeps.
pub fn range_popup(
    ui: &mut Ui,
    at: eframe::egui::Pos2,
    label: &str,
    spec: &NumberSpec,
    midi: MidiRow<'_>,
    theme: &Theme,
) -> RangeEdit {
    use eframe::egui::{Grid, RichText, Sense};

    let mut out = RangeEdit {
        range: None,
        value: None,
        reset: false,
        learn: false,
        unbind: false,
        dismissed: false,
    };
    let mut range = spec.range;
    let mut value = spec.value;

    let shown = crate::ui::popup::Popup::new(ui.id().with(("range", label)), at)
        .edge(Theme::border_strong)
        .radius(theme::RADIUS_MD)
        .margin(10.0)
        .show(ui.ctx(), theme, |ui| {
            ui.spacing_mut().item_spacing = vec2(10.0, 6.0);
            // Every label here is monospace, and the headings are tiny.
            let mono = |text: &str, color| RichText::new(text).monospace().color(color);
            let tiny = |text: &str, color| mono(text, color).size(theme::FONT_TINY);
            ui.label(mono(label, theme.text_primary()).strong());
            // The definition's own ends, so "how far can this go" is answered from
            // the header alone rather than something to go find in the node source.
            let declared = format!("declared {} .. {}", spec.declared.min, spec.declared.max);
            ui.label(tiny(&declared, theme.text_muted()));

            // A step is bounded by what a step can be, not by where the range sits:
            // a quantum of 0.1 on a control running 1 to 64 is an ordinary thing to
            // want, and the ends' own bounds would round it away on sight.
            let floor = spec.step_floor();
            // **The ends are not bounded at all.** The declared pair is printed above
            // as advice; binding the fields to it would mean a control could only
            // ever be narrowed, which is half of what this table is for.
            let ends = NumberField::ANY;
            let quanta = NumberField {
                min: floor,
                max: (range.max - range.min).max(floor),
                ..NumberField::ANY
            };
            let value_bounds = NumberField {
                min: range.min,
                max: range.max,
                ..NumberField::ANY
            };

            let rows = [
                Row {
                    name: "min",
                    field: &mut range.min,
                    default: spec.declared.min,
                    bounds: ends,
                },
                Row {
                    name: "step",
                    field: &mut range.step,
                    default: spec.declared.step,
                    bounds: quanta,
                },
                Row {
                    name: "max",
                    field: &mut range.max,
                    default: spec.declared.max,
                    bounds: ends,
                },
                Row {
                    name: "value",
                    field: &mut value,
                    default: spec.default,
                    bounds: value_bounds,
                },
            ];

            Grid::new(ui.id().with("grid"))
                .num_columns(3)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(tiny("value", theme.text_muted()));
                    ui.label(tiny("default", theme.text_muted()));
                    ui.end_row();

                    for row in rows {
                        ui.label(mono(row.name, theme.text_secondary()));
                        // The same recessed field the control itself draws — the
                        // popup is the control's own instrument for these numbers,
                        // and should read as the same family rather than a bare
                        // egui default wearing the theme's flat colors.
                        let (rect, _) = ui.allocate_exact_size(vec2(64.0, 20.0), Sense::hover());
                        let shown = spec.shown(*row.field);
                        let name = format!("{label}.{}", row.name);
                        if let Some(v) = crate::ui::text::number(
                            ui,
                            rect,
                            name,
                            &shown,
                            row.bounds,
                            crate::ui::text::Chrome::Inset,
                            theme,
                            1.0,
                        ) {
                            *row.field = v;
                        }
                        // The default, dimmer and beside it: clicking it is the reset
                        // for this one row, rather than the whole control.
                        let shown = spec.shown(row.default);
                        let default = ui
                            .add(
                                eframe::egui::Label::new(mono(&shown, theme.text_muted()))
                                    .sense(Sense::click()),
                            )
                            .on_hover_text("click to reset this field");
                        if default.clicked() {
                            *row.field = row.default;
                        }
                        ui.end_row();
                    }

                    // A unit is the definition's, not the instance's — there is
                    // nothing to commit an edit of it to — so it is shown rather
                    // than offered, in the same row shape as the numbers above it.
                    if !spec.unit.is_empty() {
                        ui.label(mono("unit", theme.text_secondary()));
                        ui.label(mono(spec.unit, theme.text_secondary()));
                        ui.label(mono(spec.unit, theme.text_muted()));
                        ui.end_row();
                    }
                });

            // MIDI, under its own rule: what drives this control, and the two things
            // that can be done about it.
            match midi {
                MidiRow::Bound(trigger) => {
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(tiny(trigger, theme.accent()));
                        if ui.button("Unbind").clicked() {
                            out.unbind = true;
                        }
                    });
                }
                MidiRow::Unbound => {
                    ui.separator();
                    if ui.button("Bind MIDI…").clicked() {
                        out.learn = true;
                    }
                }
                MidiRow::Unbindable => {}
            }
            ui.separator();
            if ui.button("Reset").clicked() {
                out.reset = true;
            }
        });

    if range != spec.range {
        out.range = Some(range);
    }
    if value != spec.value {
        out.value = Some(spec.quantize(value));
    }

    out.dismissed = shown.clicked_away;
    out
}

/// Undo `drag_started`'s grab: released on every drag end, canceled or ordinary, so a
/// scrub never leaves the pointer locked and invisible. A no-op where the preference never
/// engaged it, rather than the caller having to remember whether it did.
fn release_cursor(ctx: &eframe::egui::Context, lock_cursor: bool) {
    if lock_cursor {
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CursorGrab(
            eframe::egui::viewport::CursorGrab::None,
        ));
        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::CursorVisible(true));
    }
}

/// Built inside the closure, which egui calls only when something is listening — a name
/// formatted every frame for every control is three strings nobody reads.
fn describe(response: &Response, label: &impl std::fmt::Display, spec: &NumberSpec) {
    response.widget_info(|| {
        let name = if spec.learning {
            format!("{label} {} {}", spec.with_unit(), crate::ui::LEARNING)
        } else if let Some(ghost) = spec.ghost {
            format!(
                "{label} {} ({} {})",
                spec.with_unit(),
                crate::ui::GHOST,
                spec.shown(ghost)
            )
        } else {
            format!("{label} {}", spec.with_unit())
        };
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::DragValue, true, name)
    });
}

/// The label, the value and the range, for the tooltip: `frequency 8/⬓ (0.1-64, step 0.1)`.
/// `f32`'s `Display` drops a trailing `.0`, so a whole-number end reads as one.
fn hover_text(label: &impl std::fmt::Display, spec: &NumberSpec) -> String {
    // The ends are still this control's ends, but they are not what somebody hovering a
    // varying value wants told: what it says is why there is no number.
    if spec.varying {
        return format!("{label} — a varying value arrives here: one per pixel, not a number");
    }
    if spec.ladder {
        return format!(
            "{label} {} — drag ÷16 to ×16 and through ×0 to reverse, or type ×n, ÷n, p/q or a \
             number",
            spec.text()
        );
    }
    format!(
        "{label} {} ({}-{}, step {})",
        spec.with_unit(),
        spec.min(),
        spec.max(),
        spec.step()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranged(value: f32, min: f32, max: f32, step: f32, log: bool) -> NumberSpec {
        let range = ControlRange { min, max, step };
        NumberSpec {
            value,
            default: min,
            range,
            declared: range,
            unit: "",
            log,
            varying: false,
            learning: false,
            ladder: false,
            ghost: None,
        }
    }

    /// The same spec with a varying value arriving at it, which is what a connected control
    /// that cannot be metered looks like.
    fn fed_by_a_varying(value: f32) -> NumberSpec {
        NumberSpec {
            varying: true,
            ..spec(value)
        }
    }

    #[test]
    fn a_varying_value_has_no_number_and_no_fill() {
        let s = fed_by_a_varying(32.0);
        assert_eq!(s.with_unit(), VARYING, "the word, not the stored number");
        assert_eq!(
            s.fraction(),
            0.0,
            "and no position on the track: 32 of 64 would draw a half-full bar under a word \
             saying there is no number"
        );
        // The number itself is untouched — it is still the node's control, and it is what
        // comes back the moment the cable is pulled.
        assert_eq!(s.value, 32.0);
        assert!(
            (spec(32.0).fraction() - 0.5).abs() < 0.02,
            "the same spec unfed"
        );
    }

    fn spec(value: f32) -> NumberSpec {
        ranged(value, 1.0, 64.0, 1.0, false)
    }

    fn log_spec(value: f32) -> NumberSpec {
        ranged(value, 0.01, 100.0, 0.01, true)
    }

    #[test]
    fn quantize_snaps_to_the_step_and_clamps_to_the_range() {
        let s = spec(8.0);
        assert_eq!(s.quantize(8.4), 8.0);
        assert_eq!(s.quantize(8.6), 9.0);
        assert_eq!(s.quantize(-5.0), 1.0);
        assert_eq!(s.quantize(1000.0), 64.0);
    }

    #[test]
    fn the_fill_tracks_the_position_within_the_range() {
        assert_eq!(spec(1.0).fraction(), 0.0);
        assert_eq!(spec(64.0).fraction(), 1.0);
        assert!((spec(32.5).fraction() - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_degenerate_range_does_not_divide_by_zero() {
        assert_eq!(ranged(5.0, 5.0, 5.0, 1.0, false).fraction(), 0.0);
    }

    #[test]
    fn a_log_control_puts_the_geometric_midpoint_halfway_along() {
        // 1.0 is the midpoint of 0.01..100 in log space, and nowhere near it linearly.
        assert!((log_spec(1.0).fraction() - 0.5).abs() < 0.01);
        assert!(
            log_spec(1.0).fraction() > 0.4,
            "linear would put this at 0.01"
        );
    }

    #[test]
    fn a_log_scrub_moves_by_ratio_not_by_step() {
        let one = log_spec(1.0);
        let up = one.advance(24.0, 1.0);
        // A tenth of the travel across four decades is about one decade.
        assert!(up > 2.0 && up < 20.0, "got {up}");
        // And the same drag from a smaller value moves by a similar *ratio*, not amount.
        let small = log_spec(0.1).advance(24.0, 1.0);
        assert!(
            (small / 0.1 - up / 1.0).abs() < 1.0,
            "ratios should match: {small} {up}"
        );
    }

    #[test]
    fn a_log_control_with_a_non_positive_range_falls_back_to_linear() {
        let s = ranged(0.0, -1.0, 1.0, 0.1, true);
        assert!(s.log_span().is_none());
        assert!((s.advance(1.0, 1.0) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn a_position_along_the_bar_is_the_value_the_fill_would_show() {
        let s = spec(8.0);
        assert_eq!(s.at_fraction(0.0), 1.0);
        assert_eq!(s.at_fraction(1.0), 64.0);
        // Round trip: the jump lands where the fill's edge is drawn.
        assert!((s.at_fraction(spec(48.0).fraction()) - 48.0).abs() < 1e-3);
        assert_eq!(s.at_fraction(-2.0), 1.0, "off the left end is the left end");
    }

    #[test]
    fn a_position_on_a_log_bar_is_read_in_log_space() {
        let s = log_spec(1.0);
        assert!(
            (s.at_fraction(0.5) - 1.0).abs() < 0.05,
            "the geometric middle"
        );
        assert!(
            s.at_fraction(0.25) < 0.2,
            "a quarter along is a decade down"
        );
    }

    #[test]
    fn a_stepper_scales_the_step_but_keeps_it_usable() {
        assert_eq!(spec(8.0).scaled_step(10.0), 10.0);
        assert_eq!(spec(8.0).scaled_step(0.1), 0.1);
        // The floor is what the three-place readout can show.
        let fine = ranged(1.0, 0.0, 1.0, 0.001, false);
        assert_eq!(
            fine.scaled_step(0.1),
            0.001,
            "no step the readout cannot show"
        );
        // A definition that declares a finer one keeps it.
        let finer = ranged(1.0, 0.0, 1.0, 0.0001, false);
        assert_eq!(finer.scaled_step(0.1), 0.0001);
        // The ceiling is the span: a coarser step leaves the control with one position.
        let narrow = ranged(1.0, 0.0, 2.0, 1.0, false);
        assert_eq!(narrow.scaled_step(10.0), 2.0);
    }

    #[test]
    fn decimals_follow_the_step() {
        assert_eq!(spec(1.0).decimals(), 0);
        assert_eq!(ranged(1.0, 1.0, 64.0, 0.5, false).decimals(), 1);
        assert_eq!(ranged(1.0, 1.0, 64.0, 0.01, false).decimals(), 2);
    }
}
