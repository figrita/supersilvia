// SPDX-License-Identifier: AGPL-3.0-or-later

//! The List node's bars: one per item, from the floor of the band up to its share of the way
//! from Min to Max, dragged up and down by hand — Max's `multislider`.
//!
//! It is the list row's chart drawn tall and made to answer the pointer: the same hole for a
//! ground and the same ink for a bar, so a hand-made list looks like the list it publishes.
//!
//! **A drag is an edit.** A press sets the bar under it to the height of the pointer, and every
//! frame the pointer moves sets each bar it crossed, the run between the last place and this
//! one drawn as a straight line, so a quick sweep across leaves no gaps. Each frame hands the
//! whole list back as a `SetValue`, and every one of them lands in the gesture the press
//! opened, so a stroke is one undo step. Where the pointer was on the last frame is egui's
//! widget memory, the region's to keep and nobody's document.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::{Node, PortType};
use crate::nodes::list::{self, make};
use eframe::egui::{Align2, CursorIcon, Pos2, Rect, pos2};

/// The band's padding round the bars.
const PAD: f32 = 8.0;
/// The bars' own height.
const HEIGHT: f32 = 72.0;

pub const BARS: RegionDef = RegionDef {
    size,
    show,
    claims_pointer: true,
    ..RegionDef::EMPTY
};

fn size(_node: &Node, _: crate::nodes::Region) -> f32 {
    PAD * 2.0 + HEIGHT
}

/// Where the pointer was on the last frame of a drag: the bar, and its share.
#[derive(Clone, Copy)]
struct Stroke {
    bar: usize,
    share: f32,
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let area = r.rect.shrink(PAD * r.zoom);
    let count = list::whole(f64::from(r.number_now("count")));
    let value = r.node.values.get(make::VALUES);
    let painter = r.ui.painter();
    painter.rect_filled(area, 0.0, r.theme.port_hole(PortType::ListNumber));
    let ink = r.theme.readout();
    let step = area.width() / count.max(1) as f32;
    let gap = if step >= 4.0 * r.zoom { r.zoom } else { 0.0 };
    let bar_rect = |i: usize, share: f32| {
        let x = area.left() + step * i as f32;
        let top = area.bottom() - share * area.height();
        Rect::from_x_y_ranges(x..=x + step - gap, top..=area.bottom())
    };
    for i in 0..count {
        let share = make::bar(value, i) as f32;
        // A bar at none still shows a hair, so an empty list reads as bars at the floor.
        let rect = bar_rect(i, share);
        let rect = rect.with_min_y(rect.min.y.min(area.bottom() - r.zoom));
        painter.rect_filled(rect, 0.0, ink);
    }

    let mut out = Vec::new();
    let Some(grab) = r.grab.clone() else {
        return out;
    };
    let place = |at: Pos2| {
        let bar = ((at.x - area.left()) / step).floor().max(0.0) as usize;
        let share = ((area.bottom() - at.y) / area.height()).clamp(0.0, 1.0);
        Stroke {
            bar: bar.min(count.saturating_sub(1)),
            share,
        }
    };
    let pointer = grab
        .interact_pointer_pos()
        .or_else(|| r.ui.input(|i| i.pointer.latest_pos()));
    let hovered = pointer.filter(|p| grab.hovered() && area.contains(*p));
    if hovered.is_some() {
        r.ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
    }
    let memory = grab.id.with("bars");
    let last: Option<Stroke> = r.ui.data(|d| d.get_temp(memory));
    let held = grab.is_pointer_button_down_on();
    let pressed_here =
        r.ui.input(|i| i.pointer.press_origin())
            .is_some_and(|o| area.contains(o));
    match (count, held && pressed_here, pointer) {
        (1.., true, Some(at)) => {
            let here = place(at);
            let from = last.unwrap_or(here);
            let mut next = value.cloned();
            for (bar, share) in run(from, here) {
                next = Some(make::set_bar(next.as_ref(), bar, share));
            }
            r.ui.data_mut(|d| d.insert_temp(memory, here));
            if let Some(value) = next
                && Some(&value) != r.node.values.get(make::VALUES)
            {
                out.push(RegionEvent::Value {
                    key: make::VALUES,
                    value,
                });
            }
            caption(r, area, here.bar);
        }
        // A press and its release inside one frame, which a quick click can be.
        (1.., false, Some(at)) if grab.clicked() && area.contains(at) => {
            let here = place(at);
            out.push(RegionEvent::Value {
                key: make::VALUES,
                value: make::set_bar(value, here.bar, here.share),
            });
        }
        _ => {
            if last.is_some() {
                r.ui.data_mut(|d| d.remove::<Stroke>(memory));
            }
            if let Some(at) = hovered.filter(|_| count > 0) {
                caption(r, area, place(at).bar);
            }
        }
    }
    out
}

/// Every bar from `from` to `to`, each at the height a straight line between the two reaches
/// over it.
fn run(from: Stroke, to: Stroke) -> impl Iterator<Item = (usize, f32)> {
    let (lo, hi) = (from.bar.min(to.bar), from.bar.max(to.bar));
    let span = (to.bar as f32 - from.bar as f32).abs().max(1.0);
    (lo..=hi).map(move |bar| {
        let t = (bar as f32 - from.bar as f32).abs() / span;
        let share = if from.bar == to.bar {
            to.share
        } else {
            from.share + (to.share - from.share) * t
        };
        (bar, share)
    })
}

/// Which bar is under the hand and what it publishes, in the band's top corner.
fn caption(r: &RegionUi<'_>, area: Rect, bar: usize) {
    let (min, max) = (r.number_now("min"), r.number_now("max"));
    let share = make::bar(r.node.values.get(make::VALUES), bar) as f32;
    let text = format!("{}: {:.2}", bar + 1, min + share * (max - min));
    r.caption(
        pos2(area.right() - 2.0 * r.zoom, area.top() + 2.0 * r.zoom),
        Align2::RIGHT_TOP,
        &text,
        r.theme.text_secondary(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sweep_sets_every_bar_it_crossed_on_a_straight_line() {
        let from = Stroke { bar: 1, share: 0.0 };
        let to = Stroke { bar: 3, share: 1.0 };
        let run: Vec<(usize, f32)> = run(from, to).collect();
        assert_eq!(run, [(1, 0.0), (2, 0.5), (3, 1.0)]);
        let back: Vec<(usize, f32)> = super::run(to, from).collect();
        assert_eq!(back, [(1, 0.0), (2, 0.5), (3, 1.0)]);
        let still: Vec<(usize, f32)> = super::run(to, to).collect();
        assert_eq!(still, [(3, 1.0)]);
    }
}
