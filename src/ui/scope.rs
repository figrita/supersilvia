// SPDX-License-Identifier: AGPL-3.0-or-later

//! The audio scope: what each band is hearing, and the two handles that tune it.
//!
//! Ported from silvia's `audioScope.js` and `audio-meters.css`, and it is worth saying what
//! is being copied, because both halves are ideas rather than decoration.
//!
//! **The spectrum**, on a logarithmic frequency axis, with the bins under each band tinted
//! that band's color and a **handle** drawn on it: X is where the band listens, Y is how
//! narrowly. Dragging one is the only way to set a band's center frequency and Q — they are
//! `NodeDef::hidden` controls with no rows, because one drag says both at once and the tint
//! underneath says what it did, which six number fields could not.
//!
//! **A meter per band** below it: a dot in the band's color, a bar that fills with what the
//! band is hearing, and the **threshold** on it as a twelve-pixel rounded square in the action
//! port's own color, dragged along the bar. That square is silvia's best idea in this whole
//! area — the control that sets the level and the port that emits the event are one object,
//! sitting on top of the data they measure, and then there is nothing left to explain. It
//! brightens while the gate it opens is open.
//!
//! The bar fills with what the band is hearing, which is the number the threshold beside it
//! is compared against.

use crate::audio::{BANDS, Scope, bands};
use crate::graph::NodeId;
use crate::nodes::audio_ports;
use crate::nodes::cpu::TraceRing;
use crate::ui::canvas;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2,
};

/// The threshold square, which is the action port's own 12 px rounded square.
const MARKER: f32 = 12.0;
/// The dot naming a band, at the left of its meter.
const DOT: f32 = 6.0;

/// What a drag on the scope asked to change: one control, or the pair a band handle carries.
///
/// A handle's two axes are one edit and not two, because they are one gesture: the caller
/// turns each of these into one command, and one command a frame is one undo step for the
/// whole drag.
pub type Edit = Vec<(&'static str, f32)>;

/// Whose scope this is: the egui ids its two handles need, and the name a test finds them by.
///
/// A scope belongs to a node almost everywhere, and to the Main Input panel in one place —
/// where there is no node, because the tuning is the rig's rather than any one reader's. Both
/// draw the identical picture and both drag the identical controls, so the difference is only
/// what the handles are called.
#[derive(Clone, Copy)]
pub struct Owner<'a> {
    /// Salts the egui ids, so two scopes on screen do not share a drag.
    pub salt: u64,
    /// What a handle is named: `"video3"`, or `"main input"`.
    pub name: &'a str,
}

impl<'a> Owner<'a> {
    pub fn node(id: NodeId, slug: &'a str) -> Self {
        Self {
            salt: u64::from(id.0),
            name: slug,
        }
    }

    /// The panel's. Salted past any node id, since the two id spaces share one namespace.
    pub const MAIN_INPUT: Self = Self {
        salt: u64::MAX,
        name: "main input",
    };
}

/// Draw one node's scope, and return whatever a hand moved.
pub fn show(
    ui: &mut Ui,
    rect: Rect,
    owner: Owner<'_>,
    scope: &Scope,
    theme: &Theme,
    zoom: f32,
) -> Vec<Edit> {
    let mut edits = Vec::new();
    if rect.height() < 8.0 || zoom < 0.4 {
        return edits;
    }
    // Split between the spectrum and the meters in silvia's proportion.
    let spectrum_h = rect.height() * (canvas::SCOPE_SPECTRUM / canvas::SCOPE_HEIGHT);
    let spectrum = Rect::from_min_size(rect.min, vec2(rect.width(), spectrum_h));
    let below = Rect::from_min_max(pos2(rect.min.x, spectrum.max.y), rect.max);

    // No ground of its own: the body is already filled with `bg_sunken`, with the node's own
    // rounded corners, and the border is painted before this. A square fill here in the same
    // color changed nothing except to cut the border and the corners off the band.
    draw_spectrum(ui, spectrum, scope, theme);

    edits.extend(meters(ui, below, owner, scope, theme, zoom, Hands::On));

    // After the meters, so a handle at the very bottom of the spectrum still takes the
    // pointer before the meter under it does.
    for band in 0..BANDS {
        if let Some((freq, q)) = handle(ui, spectrum, band, scope, theme, owner) {
            edits.push(vec![
                (audio_ports::FREQS[band], freq),
                (audio_ports::QS[band], q),
            ]);
        }
    }
    edits
}

/// A trace: a node's own shape, drawn as a line. What `adsr`, `oscillator` and `animation`
/// show on their body instead of a Status box number, because a shape is the thing four
/// digits cannot say.
///
/// Unrelated to the audio scope above — it shares this file because both are a picture drawn
/// in a region a node reserves rather than in a row, and both are painter, not egui widgets.
/// There is nothing to interact with: no ground of its own, the same as the audio scope,
/// since the body is already filled with `bg_sunken`.
///
/// **The x axis is time.** The band is the ring's span of seconds, the newest sample sits at
/// the right edge and every other one where its own date puts it, so a frame that arrived
/// late is a wider step on a line whose shape is still the shape — see `TraceRing`. A node
/// that has less than a span of history draws from partway across: the left is *nothing
/// yet*, not zero.
pub fn trace(ui: &Ui, rect: Rect, ring: &TraceRing, color: Color32, theme: &Theme, zoom: f32) {
    if rect.height() < 8.0 || zoom < 0.4 {
        return;
    }
    // A line needs two points; a ring with one has a newest and nothing to draw to it.
    let Some(newest) = ring.newest().filter(|_| ring.len() >= 2) else {
        return;
    };
    baseline(ui, rect, theme);

    // Fit the line to what it actually did, not to a fixed range: an envelope sits in 0..1
    // and a waveform's amplitude is a control, so neither has one range worth assuming.
    // Margin top and bottom so a value that touches an end is still a whole line, not one
    // clipped by the band's own edge.
    let band = rect.shrink2(vec2(0.0, 4.0 * zoom));
    let (lo, hi) = ring.iter().fold((f32::MAX, f32::MIN), |(lo, hi), s| {
        (lo.min(s.value), hi.max(s.value))
    });
    // A flat line is still a line: a span too small to divide by is padded rather than
    // collapsed to the band's center, which would draw nothing at all.
    let (lo, hi) = if hi - lo < 1e-4 {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    };
    let span = f64::from(ring.span());
    let points: Vec<Pos2> = ring
        .iter()
        .map(|s| {
            let age = ((newest.at - s.at) / span).clamp(0.0, 1.0) as f32;
            let x = band.max.x - band.width() * age;
            let y = band.max.y - band.height() * ((s.value - lo) / (hi - lo)).clamp(0.0, 1.0);
            pos2(x, y)
        })
        .collect();
    ui.painter()
        .add(Shape::line(points, Stroke::new(1.5 * zoom, color)));
}

/// The rule a picture in a node's own band sits on: the same line under the spectrum and under
/// a trace, so the two read as the same kind of thing.
fn baseline(ui: &Ui, rect: Rect, theme: &Theme) {
    ui.painter().hline(
        rect.x_range(),
        rect.max.y - 0.5,
        Stroke::new(1.0, theme.border_subtle()),
    );
}

// ------------------------------------------------------------------ the two axes

/// Where `v` sits between `lo` and `hi` on a logarithmic axis, 0 to 1, clamped to it.
fn log_axis(v: f32, lo: f32, hi: f32) -> f32 {
    ((v.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln())).clamp(0.0, 1.0)
}

/// The value at `t` of a logarithmic axis from `lo` to `hi`: [`log_axis`] undone.
fn log_value(t: f32, lo: f32, hi: f32) -> f32 {
    (lo.ln() + t.clamp(0.0, 1.0) * (hi.ln() - lo.ln())).exp()
}

/// Where a frequency sits across the plot, 0 to 1. Logarithmic, because music is.
fn freq_to_x(freq: f32) -> f32 {
    log_axis(freq, audio_ports::FREQ_MIN, audio_ports::FREQ_MAX)
}

fn x_to_freq(x: f32) -> f32 {
    log_value(x, audio_ports::FREQ_MIN, audio_ports::FREQ_MAX)
}

/// Q down the plot: narrow at the top, wide at the bottom, which is how a filter is drawn.
fn q_to_y(q: f32) -> f32 {
    1.0 - log_axis(q, audio_ports::Q_MIN, audio_ports::Q_MAX)
}

fn y_to_q(y: f32) -> f32 {
    log_value(
        1.0 - y.clamp(0.0, 1.0),
        audio_ports::Q_MIN,
        audio_ports::Q_MAX,
    )
}

// ------------------------------------------------------------------ the spectrum

fn draw_spectrum(ui: &Ui, rect: Rect, scope: &Scope, theme: &Theme) {
    let painter = ui.painter();
    let n = scope.spectrum.len();
    // The buckets are already on this plot's own logarithmic axis, so a bucket is a column
    // and the width is uniform. Which band owns a column is asked in hertz, so the tint says
    // what each band is listening to rather than leaving it to be inferred from a handle.
    let width = rect.width() / n as f32;
    for (i, level) in scope.spectrum.iter().enumerate() {
        let hz = Scope::bucket_hz(i);
        let color = match (0..BANDS).find(|b| {
            let cfg = scope.config[*b];
            let half = cfg.freq / cfg.q.max(0.01) * 0.5;
            (cfg.freq - half..=cfg.freq + half).contains(&hz)
        }) {
            Some(b) => theme.band(b),
            None => theme.text_muted().gamma_multiply(0.5),
        };
        let h = level.clamp(0.0, 1.0) * rect.height();
        if h > 0.5 {
            let x = rect.min.x + width * i as f32;
            painter.rect_filled(
                Rect::from_min_max(pos2(x, rect.max.y - h), pos2(x + width, rect.max.y)),
                0.0,
                color,
            );
        }
    }
    baseline(ui, rect, theme);
}

/// The band handle: one drag, a frequency and a Q.
fn handle(
    ui: &mut Ui,
    spectrum: Rect,
    band: usize,
    scope: &Scope,
    theme: &Theme,
    owner: Owner<'_>,
) -> Option<(f32, f32)> {
    let cfg = scope.config[band];
    let at = pos2(
        spectrum.min.x + freq_to_x(cfg.freq) * spectrum.width(),
        spectrum.min.y + q_to_y(cfg.q) * spectrum.height(),
    );
    let response = ui.interact(
        Rect::from_center_size(at, vec2(MARKER + 4.0, MARKER + 4.0)),
        ui.id().with(("band", owner.salt, band)),
        Sense::click_and_drag(),
    );

    let radius = if response.dragged() || response.hovered() {
        6.0
    } else {
        5.0
    };
    ui.painter().circle_filled(at, radius, theme.band(band));
    ui.painter().circle_stroke(
        at,
        radius,
        Stroke::new(
            1.5,
            if response.dragged() {
                theme.text_primary()
            } else {
                theme.bg_primary()
            },
        ),
    );

    let name = format!("{}.{} band", owner.name, bands::NAMES[band]);
    crate::ui::accessible(
        &response,
        eframe::egui::WidgetType::Other,
        format_args!("{name} {:.0} Hz Q {:.2}", cfg.freq, cfg.q),
    );

    if response.dragged()
        && let Some(to) = response.interact_pointer_pos()
    {
        return Some((
            x_to_freq((to.x - spectrum.min.x) / spectrum.width().max(1.0)),
            y_to_q((to.y - spectrum.min.y) / spectrum.height().max(1.0)),
        ));
    }
    None
}

// ------------------------------------------------------------------ the meters

/// Whether a threshold square on these meters may be dragged.
///
/// The same picture either way — that is the point. A `maininput` node draws the rig's three
/// bands with the panel's threshold on each, because silvia draws them on the node and a
/// level is set where the band is being watched; but the threshold itself is the **panel's**,
/// one number for every reader, so the square on a node says where it is and does not offer
/// to move it. Eight nodes each dragging one copy of it would be eight answers to a question
/// that has one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Hands {
    /// The scope's own: drag a square and the level moves.
    On,
    /// Read-only. No interact of its own, so the drag reaches the node's body and a hand
    /// still carries the node by its meters.
    Off,
}

/// The three band meters, in a band of their own.
///
/// Split out of [`show`] because the `maininput` node draws these and not the spectrum: it
/// has no tuning to place a handle for, and the handles are the only part of the scope that
/// is about tuning. Same geometry, same colors, same threshold square.
pub fn meters(
    ui: &mut Ui,
    rect: Rect,
    owner: Owner<'_>,
    scope: &Scope,
    theme: &Theme,
    zoom: f32,
    hands: Hands,
) -> Vec<Edit> {
    let mut edits = Vec::new();
    if rect.height() < 8.0 || zoom < 0.4 {
        return edits;
    }
    let pitch = rect.height() / BANDS as f32;
    for band in 0..BANDS {
        let row = Rect::from_min_size(
            pos2(rect.min.x, rect.min.y + pitch * band as f32),
            vec2(rect.width(), pitch),
        );
        if let Some(level) = meter(ui, row, band, scope, theme, owner, zoom, hands) {
            edits.push(vec![(audio_ports::LEVELS[band], level)]);
        }
    }
    edits
}

/// One band's meter, with the threshold square on it. Returns a new level when dragged.
#[allow(clippy::too_many_arguments)]
fn meter(
    ui: &mut Ui,
    row: Rect,
    band: usize,
    scope: &Scope,
    theme: &Theme,
    owner: Owner<'_>,
    zoom: f32,
    hands: Hands,
) -> Option<f32> {
    let color = theme.band(band);
    let pad = 4.0 * zoom;
    let dot = DOT * zoom;
    let center = row.center().y;
    ui.painter()
        .circle_filled(pos2(row.min.x + pad + dot * 0.5, center), dot * 0.5, color);

    let bar = Rect::from_min_max(
        pos2(row.min.x + pad * 2.0 + dot, center - row.height() * 0.3),
        pos2(row.max.x - pad, center + row.height() * 0.3),
    );
    let radius = CornerRadius::same(theme::RADIUS_SM);
    let painter = ui.painter();
    painter.rect_filled(bar, radius, theme.bg_primary());

    let level = scope.levels[band];
    let w = bar.width() * level.clamp(0.0, 1.0);
    if w > 0.5 {
        painter.rect_filled(
            Rect::from_min_size(bar.min, vec2(w, bar.height())),
            radius,
            color,
        );
    }

    painter.rect_stroke(
        bar,
        radius,
        Stroke::new(1.0, theme.border_subtle()),
        StrokeKind::Inside,
    );

    let threshold = scope.thresholds[band];
    let armed = threshold < 1.0;
    let at = pos2(bar.min.x + bar.width() * threshold.clamp(0.0, 1.0), center);
    let size = MARKER * zoom;
    let square = Rect::from_center_size(at, vec2(size, size));
    // A read-only meter senses hover only: it still names itself for a test and for the
    // agent-driven layer, and it still lets a press through to the node under it.
    let response = ui.interact(
        square.expand(3.0),
        ui.id().with(("threshold", owner.salt, band)),
        match hands {
            Hands::On => Sense::click_and_drag(),
            Hands::Off => Sense::hover(),
        },
    );

    // The action port's own color and shape, because that is what it is. Bright while the
    // gate it opens is open, which is the feedback silvia gets by flashing it.
    let action = theme.port(crate::graph::PortType::Action);
    let open = armed && level >= threshold;
    ui.painter().rect_filled(
        square,
        CornerRadius::same(2),
        match (armed, open) {
            // Parked at the top: nothing reaches one, so nothing fires, and it says so.
            (false, _) => action.gamma_multiply(0.3),
            (true, false) => action,
            (true, true) => theme.text_primary(),
        },
    );

    // The level as well as the threshold: on a read-only meter the fill is the whole of what
    // the band is saying, and a picture of a bar is not something a test can read.
    let name = format!("{}.{} threshold", owner.name, bands::NAMES[band]);
    crate::ui::accessible(
        &response,
        match hands {
            Hands::On => eframe::egui::WidgetType::Slider,
            Hands::Off => eframe::egui::WidgetType::Label,
        },
        format_args!("{name} {threshold:.2}, level {level:.2}"),
    );
    if response.dragged()
        && let Some(to) = response.interact_pointer_pos()
    {
        let next = ((to.x - bar.min.x) / bar.width().max(1.0)).clamp(0.0, 1.0);
        if (next - threshold).abs() > f32::EPSILON {
            return Some(next);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frequency_axis_round_trips_and_is_logarithmic() {
        for hz in [20.0, 100.0, 1000.0, 8000.0, 20_000.0] {
            let back = x_to_freq(freq_to_x(hz));
            assert!((back / hz - 1.0).abs() < 0.01, "{hz} came back as {back}");
        }
        // A decade is the same distance wherever it is, which linear would not give.
        let a = freq_to_x(1000.0) - freq_to_x(100.0);
        let b = freq_to_x(10_000.0) - freq_to_x(1000.0);
        assert!((a - b).abs() < 0.001, "{a} against {b}");
    }

    /// Narrow at the top, wide at the bottom, and a round trip either way: the handle is put
    /// where the value says and the value comes back from where the handle was left.
    #[test]
    fn the_q_axis_runs_down_the_plot_and_round_trips() {
        assert!(q_to_y(audio_ports::Q_MAX) < q_to_y(audio_ports::Q_MIN));
        for q in [audio_ports::Q_MIN, 1.0, 4.0, audio_ports::Q_MAX] {
            let back = y_to_q(q_to_y(q));
            assert!((back / q - 1.0).abs() < 0.01, "{q} came back as {back}");
        }
    }

    /// The defaults have to land inside the plot, or a handle opens off the edge of it.
    #[test]
    fn every_default_band_sits_inside_the_plot() {
        for cfg in bands::DEFAULT {
            let (x, y) = (freq_to_x(cfg.freq), q_to_y(cfg.q));
            assert!((0.02..=0.98).contains(&x), "{} Hz is at {x}", cfg.freq);
            assert!((0.02..=0.98).contains(&y), "Q {} is at {y}", cfg.q);
        }
    }
}
