// SPDX-License-Identifier: AGPL-3.0-or-later

//! The controls a picture carries: a scrubber, a mute and a volume, in the strip along the
//! bottom that every video player has put them in for thirty years.
//!
//! They are the conventional shape on purpose. A hand that has used a video player knows a
//! filled bar is where it is and that dragging it seeks; knows the speaker mutes and the
//! short slider beside it is loudness. Nothing here is a new idea, which is the point — the
//! new ideas on this node are the ports.
//!
//! **What it edits lives elsewhere.** Volume is the `monitor` input's control, so the strip
//! is a second editor for the same address the number row on the node already has, exactly as
//! a meter's threshold square is for its level. Position is not a control at all — where a
//! clip is playing from is the node's own state — so a drag goes back as a *seek*
//! ([`TickContext::seek`]), which repositions the clip and lets `speed` carry on from there.
//!
//! Drawn over a picture, so everything is painted on a scrim dark enough to read over a white
//! frame, and nothing is drawn at all until a hand is on the picture.

use crate::ui::theme::Theme;
use eframe::egui::{
    Color32, CornerRadius, CursorIcon, Rect, Sense, Stroke, Ui, WidgetInfo, WidgetType, pos2, vec2,
};

/// How tall the strip is, and the size of the speaker inside it.
pub const HEIGHT: f32 = 18.0;
/// The volume slider's track, beside the speaker.
const VOLUME_WIDTH: f32 = 40.0;
/// Clear space at the strip's ends and between its parts.
const PAD: f32 = 4.0;
/// How thick the scrub track is, and how much taller the part a hand can grab.
const TRACK: f32 = 4.0;

/// What a hand did to the strip. One at a time: they are different controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touched {
    /// Drag on the scrubber: 0 to 1 through the clip.
    Seek(f32),
    /// Drag on the volume: the `monitor` level, 0 to 1.
    Volume(f32),
    /// The speaker was clicked. Which way it goes is the caller's, because only the caller
    /// remembers what the level was before it was silenced.
    Mute,
}

/// What the strip shows: where the picture is, and how loud it is.
///
/// `at` is `None` for a picture that is not playing a length of anything — an Output's
/// render — and then there is no scrubber, only the volume. `volume` is `None` for a node
/// with no `monitor` input, and then there is no volume, only the scrubber. A picture with
/// neither gets no strip at all.
#[derive(Debug, Clone, Copy, Default)]
pub struct Playing {
    pub at: Option<f32>,
    pub volume: Option<f32>,
    /// Whether a hand may drag the scrubber. False while a cable answers the node's
    /// `position` input, where the bar is a readout of where the graph has put the clip — the
    /// same rule an option row follows when an input overrides it: it still says what the
    /// value is, and nothing invites a hand that cannot change it.
    pub seekable: bool,
}

impl Playing {
    fn empty(self) -> bool {
        self.at.is_none() && self.volume.is_none()
    }
}

/// Draw the strip along the bottom of `picture` and return what a hand did.
///
/// `scale` is the zoom on a node and 1 in a window. `name` is the node's own — `video1` — so
/// the parts report themselves as `video1 scrubber` and `video1 volume`.
pub fn show(
    ui: &mut Ui,
    picture: Rect,
    playing: Playing,
    name: &str,
    theme: &Theme,
    scale: f32,
) -> Option<Touched> {
    let height = HEIGHT * scale;
    let pad = PAD * scale;
    // Too small to hold controls without being the picture. The same judgement the marks in
    // the opposite corner make, so the two arrive and leave together.
    if playing.empty() || picture.height() < height * 3.0 || picture.width() < height * 6.0 {
        return None;
    }
    let strip = Rect::from_min_max(
        pos2(picture.min.x, picture.max.y - height - pad),
        pos2(picture.max.x, picture.max.y - pad),
    );
    // A scrim under the whole strip rather than a plate per control: a white frame under a
    // bare white icon is a control nobody can see, and the gradient every player uses is one
    // shape here because the strip is one row.
    ui.painter().rect_filled(
        strip.expand2(vec2(0.0, pad * 0.5)),
        CornerRadius::ZERO,
        Color32::from_black_alpha(140),
    );

    let mut touched = None;
    let mut left = strip.min.x + pad;
    if let Some(volume) = playing.volume {
        let speaker = Rect::from_min_size(pos2(left, strip.min.y), vec2(height, height));
        if mute(ui, speaker, volume, name, theme) {
            touched = Some(Touched::Mute);
        }
        left = speaker.max.x + pad * 0.5;
        let track = Rect::from_min_max(
            pos2(left, strip.min.y),
            pos2(left + VOLUME_WIDTH * scale, strip.max.y),
        );
        if let Some(v) = slider(ui, track, volume, &format!("{name} volume"), theme, scale) {
            touched = Some(Touched::Volume(v));
        }
        left = track.max.x + pad;
    }
    if let Some(at) = playing.at {
        let track = Rect::from_min_max(
            pos2(left, strip.min.y),
            pos2(strip.max.x - pad, strip.max.y),
        );
        let name = format!("{name} scrubber");
        if playing.seekable {
            if let Some(to) = slider(ui, track, at, &name, theme, scale) {
                touched = Some(Touched::Seek(to));
            }
        } else {
            readout(ui, track, at, &name, theme, scale);
        }
    }
    touched
}

/// The speaker, which is a mute button: three states, as every player's is — silent, quiet,
/// loud — so the icon says what the slider beside it says without being read.
fn mute(ui: &mut Ui, rect: Rect, volume: f32, name: &str, theme: &Theme) -> bool {
    let w = ui
        .interact(rect, ui.id().with(("mute", name)), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    crate::ui::accessible(
        &w,
        WidgetType::Button,
        format_args!("{name} {}", if volume > 0.0 { "mute" } else { "unmute" }),
    );
    w.clone().on_hover_text(if volume > 0.0 {
        "Mute this node's monitor"
    } else {
        "Unmute this node's monitor"
    });
    let color = if w.hovered() {
        theme.text_primary()
    } else {
        theme.text_secondary()
    };
    speaker(ui.painter(), rect, volume, color);
    w.clicked()
}

/// The speaker's own geometry: the cone, then an arc per step of loudness, or the cross that
/// every player draws over a silent one.
fn speaker(painter: &eframe::egui::Painter, rect: Rect, volume: f32, color: Color32) {
    let s = rect.width();
    let c = rect.center();
    let p = |x: f32, y: f32| pos2(c.x + s * x, c.y + s * y);
    let stroke = Stroke::new((s * 0.09).max(1.0), color);
    // The box and the cone, as one closed outline.
    for [a, b] in [
        [p(-0.30, -0.10), p(-0.16, -0.10)],
        [p(-0.16, -0.10), p(0.02, -0.28)],
        [p(0.02, -0.28), p(0.02, 0.28)],
        [p(0.02, 0.28), p(-0.16, 0.10)],
        [p(-0.16, 0.10), p(-0.30, 0.10)],
        [p(-0.30, 0.10), p(-0.30, -0.10)],
    ] {
        painter.line_segment([a, b], stroke);
    }
    if volume <= 0.0 {
        // Silent: the cross, not a missing arc — an absence is not a state a glance can see.
        painter.line_segment([p(0.14, -0.14), p(0.36, 0.14)], stroke);
        painter.line_segment([p(0.36, -0.14), p(0.14, 0.14)], stroke);
        return;
    }
    // One arc for a quiet monitor, two for a loud one. Drawn as short polylines, the way the
    // header's `?` hook is, rather than as font glyphs.
    let arcs = if volume > 0.5 { 2 } else { 1 };
    for arc in 0..arcs {
        let r = 0.14 + 0.12 * arc as f32;
        let points: Vec<_> = (0..=6i32)
            .map(|i| {
                let a = -0.9 + 1.8 * i as f32 / 6.0;
                p(0.10 + r * a.cos(), r * a.sin())
            })
            .collect();
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], stroke);
        }
    }
}

/// The bar a slider and a readout both draw, across `rect`: the track, and the part behind
/// `value` filled. Returns the two.
fn bar(
    ui: &Ui,
    rect: Rect,
    value: f32,
    ground: Color32,
    fill: Color32,
    scale: f32,
) -> (Rect, Rect) {
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), TRACK * scale));
    let radius = CornerRadius::same((TRACK * scale * 0.5).max(1.0) as u8);
    ui.painter().rect_filled(track, radius, ground);
    let filled = Rect::from_min_max(
        track.min,
        pos2(
            track.min.x + track.width() * value.clamp(0.0, 1.0),
            track.max.y,
        ),
    );
    ui.painter().rect_filled(filled, radius, fill);
    (track, filled)
}

/// The same bar with nothing to grab: where the graph has put the clip, while a cable is the
/// thing putting it there. Dimmer than a live one and with no head, because a head is a
/// handle and there is nothing here to handle.
fn readout(ui: &mut Ui, rect: Rect, value: f32, name: &str, theme: &Theme, scale: f32) {
    bar(
        ui,
        rect,
        value,
        Color32::from_white_alpha(40),
        theme.primary_muted(),
        scale,
    );
    let w = ui.interact(rect, ui.id().with(("readout", name)), Sense::hover());
    w.clone()
        .on_hover_text("A cable is driving this clip's position");
    w.widget_info(|| WidgetInfo::slider(false, f64::from(value), name));
}

/// A track with the part behind the head filled, and a head on it. Returns where a hand put
/// it, while it is putting it: a drag reports every frame, so the caller's command coalesces
/// into one undo step exactly as a number scrub does.
fn slider(
    ui: &mut Ui,
    rect: Rect,
    value: f32,
    name: &str,
    theme: &Theme,
    scale: f32,
) -> Option<f32> {
    // The whole strip-height row is the target, where the track drawn inside it is four
    // points: a four-point drag target is a miss, and every player makes the row the target.
    let w = ui
        .interact(
            rect,
            ui.id().with(("slider", name)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    let value = value.clamp(0.0, 1.0);
    let (track, filled) = bar(
        ui,
        rect,
        value,
        Color32::from_white_alpha(60),
        theme.primary(),
        scale,
    );
    // The head, as a player draws it: on the track at the value, and larger under a hand.
    let head = if w.hovered() || w.dragged() {
        TRACK * scale
    } else {
        TRACK * scale * 0.7
    };
    ui.painter()
        .circle_filled(pos2(filled.max.x, track.center().y), head, theme.primary());

    w.widget_info(|| WidgetInfo::slider(true, f64::from(value), name));
    let moved = (w.dragged() || w.clicked())
        .then(|| w.interact_pointer_pos())
        .flatten()
        .map(|at| ((at.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0));
    // A value that did not move is not an edit: a click that lands exactly where the head
    // already is should not enter the undo history as one.
    moved.filter(|to| (to - value).abs() > f32::EPSILON)
}
