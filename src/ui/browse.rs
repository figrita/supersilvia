// SPDX-License-Identifier: AGPL-3.0-or-later

//! The node browser: every node in the registry in one fuzzy-searched list.
//!
//! Two ways in and one list. A right-click on the canvas opens it at the pointer and the
//! node lands where the pointer was; `/`, `~` or `` ` `` opens it at the top of the canvas
//! and the node lands in the middle of the view. silvia's quick menu and its quake bar are
//! the same menu shown in two places, and they are one here too.
//!
//! A cable let go over empty canvas opens it a third way: at the pointer, holding only the
//! kinds that can take the cable ([`Cable`]), and the kind chosen lands wired.

use crate::graph::PortRef;
use crate::nodes::attach::Taker;
use crate::nodes::{NodeDef, REGISTRY};
use crate::ui::popup;
use crate::ui::theme::{self, Theme};
use eframe::egui::{
    Align, Align2, FontId, Id, Key, Modifiers, Pos2, Rect, ScrollArea, Sense, Stroke, TextEdit, Ui,
    vec2,
};

/// Panel width. Wide enough for the longest label with its ports beside it.
pub const WIDTH: f32 = 320.0;
/// How much list is shown before it scrolls.
const LIST_HEIGHT: f32 = 300.0;
/// One row.
const ROW_HEIGHT: f32 = 20.0;
/// A port dot in a row.
const DOT: f32 = 4.0;

/// The browser, open. `None` on `CanvasState` is closed.
#[derive(Debug, Clone)]
pub struct Browser {
    /// Top-left of the panel, in screen points.
    at: Pos2,
    /// Where the node lands, in **world** units.
    landing: Pos2,
    query: String,
    /// Index into the list as it is filtered and ordered *now*, which is why it is reset by
    /// every keystroke: the row under it is a different node once the order changed.
    selected: usize,
    /// The frame it opened on: the field takes focus, and the click that opened it is not
    /// also the click that dismisses it.
    fresh: bool,
    /// The selection moved by key, so the list scrolls to it. Not set by the mouse, which
    /// would fight the wheel.
    follow: bool,
    /// It opened, and the Nodes menu has not been told yet.
    just_opened: bool,
    /// The cable it was opened for, when a cable let go in the open opened it.
    cable: Option<Cable>,
}

/// A cable let go over empty canvas, waiting on a kind to land on.
#[derive(Debug, Clone)]
pub struct Cable {
    /// The port the cable was dragged out of.
    pub end: PortRef,
    /// Where it was let go, in **world** units: the loose end is drawn to here while the
    /// browser is up.
    pub at: Pos2,
    /// The kinds that can take it, each with the port it would land on: the list is these and
    /// nothing else. See [`crate::nodes::attach::takers`].
    pub takers: Vec<Taker>,
}

impl Cable {
    /// The port a node of this kind takes the cable on.
    pub fn key(&self, slug: &str) -> Option<&'static str> {
        self.takers.iter().find(|t| t.slug == slug).map(|t| t.key)
    }
}

impl Browser {
    pub fn new(at: Pos2, landing: Pos2) -> Self {
        Self {
            at,
            landing,
            query: String::new(),
            selected: 0,
            fresh: true,
            follow: false,
            just_opened: true,
            cable: None,
        }
    }

    /// The browser for a cable let go at `at`, on screen, holding only what takes it.
    pub fn for_cable(at: Pos2, landing: Pos2, cable: Cable) -> Self {
        Self {
            cable: Some(cable),
            ..Self::new(at, landing)
        }
    }

    /// The cable it was opened for, if it was.
    pub fn cable(&self) -> Option<&Cable> {
        self.cable.as_ref()
    }

    /// Where a node chosen now would land, in world units.
    pub fn landing(&self) -> Pos2 {
        self.landing
    }

    /// Did it open since this was last asked? `App` closes the Nodes menu on it, because two
    /// lists of the same nodes on screen at once is one too many.
    pub fn take_just_opened(&mut self) -> bool {
        std::mem::take(&mut self.just_opened)
    }
}

/// What the browser asked for this frame.
pub struct Outcome {
    /// A node kind to add, at the browser's landing point.
    pub chosen: Option<&'static str>,
    /// Escape, or a click outside it.
    pub dismissed: bool,
    /// The Paste row was clicked: the clip lands where a chosen node would have.
    pub paste: bool,
}

/// How well `pattern` describes this node, `0` for no match at all.
///
/// silvia's scoring, kept whole because it is tuned: a subsequence match over the label and
/// the slug together, scoring a run of consecutive characters by its length so `chkr` beats
/// the same letters scattered, and then the bonuses that decide the order anyone actually
/// notices — what *starts* with what you typed comes first.
pub fn score(pattern: &str, label: &str, slug: &str) -> u32 {
    let pattern = pattern.to_lowercase();
    let label = label.to_lowercase();
    let slug = slug.to_lowercase();
    if pattern.is_empty() {
        return 0;
    }
    let base = subsequence(&pattern, &format!("{label} {slug}"));
    if base == 0 {
        return 0;
    }
    let bonus = if label.starts_with(&pattern) || slug.starts_with(&pattern) {
        10_000
    } else if label
        .split_whitespace()
        .any(|word| word.starts_with(&pattern))
    {
        5_000
    } else if label.starts_with(first(&pattern)) || slug.starts_with(first(&pattern)) {
        1_000
    } else {
        0
    };
    base + bonus
}

/// The pattern's first character as a string, for the weakest of the three bonuses.
fn first(pattern: &str) -> &str {
    pattern
        .char_indices()
        .nth(1)
        .map_or(pattern, |(i, _)| &pattern[..i])
}

/// Is `pattern` a subsequence of `text`, and how well does it sit in it?
///
/// Each matched character scores the length of the run it is in, so consecutive characters
/// are worth more than the same characters spread out. `0` means it is not a subsequence.
fn subsequence(pattern: &str, text: &str) -> u32 {
    let mut chars = text.chars();
    let mut score = 0;
    let mut run = 0;
    for want in pattern.chars() {
        loop {
            match chars.next() {
                Some(c) if c == want => {
                    run += 1;
                    score += run;
                    break;
                }
                Some(_) => run = 0,
                None => return 0,
            }
        }
    }
    score
}

/// Every node this query describes, best first; every node in label order when it is empty.
/// Only what this machine offers (`NodeDef::offered`).
///
/// The sort is stable and the list starts in label order, so nodes that score the same come
/// out alphabetically rather than in registry order.
pub fn matches(query: &str) -> Vec<&'static NodeDef> {
    matches_within(query, None)
}

/// [`matches`], narrowed to the kinds in `only` where it is given: a cable's takers.
pub fn matches_within(query: &str, only: Option<&[Taker]>) -> Vec<&'static NodeDef> {
    let mut all: Vec<&'static NodeDef> = REGISTRY
        .iter()
        .copied()
        .filter(|d| (d.offered)())
        .filter(|d| only.is_none_or(|takers| takers.iter().any(|t| t.slug == d.slug)))
        .collect();
    all.sort_by_key(|d| d.label);
    let query = query.trim();
    if query.is_empty() {
        return all;
    }
    let mut scored: Vec<(u32, &'static NodeDef)> = all
        .into_iter()
        .filter_map(|d| {
            let s = score(query, d.label, d.slug);
            (s > 0).then_some((s, d))
        })
        .collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, d)| d).collect()
}

/// Draw the browser and return what it asked for. `within` is the canvas, which the panel
/// is kept inside.
pub fn show(
    ui: &mut Ui,
    browser: &mut Browser,
    within: Rect,
    theme: &Theme,
    // Whether the clipboard holds anything, which is the whole of what the Paste row needs.
    clipboard: bool,
) -> Outcome {
    let hits = matches_within(
        &browser.query,
        browser.cable.as_ref().map(|c| &c.takers[..]),
    );
    // A loose cable asks for a node to land on, and a paste is not one.
    let clipboard = clipboard && browser.cable.is_none();
    let mut out = Outcome {
        chosen: None,
        dismissed: false,
        paste: false,
    };

    // The keys first, so the search field never sees them: it is focused, and a single-line
    // field answers Enter by giving up focus, which would close the browser on the gesture
    // that is supposed to use it.
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            out.dismissed = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !hits.is_empty() {
            browser.selected = (browser.selected + 1).min(hits.len() - 1);
            browser.follow = true;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            browser.selected = browser.selected.saturating_sub(1);
            browser.follow = true;
        }
        if i.consume_key(Modifiers::NONE, Key::Enter) {
            out.chosen = hits.get(browser.selected).map(|d| d.slug);
        }
    });

    // Kept inside the canvas: opened near the right edge or the bottom, a panel that ran off
    // is a list you cannot read and a search field you cannot see.
    let height = LIST_HEIGHT.min(within.height() - 24.0);
    let at = Pos2::new(
        browser.at.x.min(within.max.x - WIDTH).max(within.min.x),
        browser.at.y.min(within.max.y - height).max(within.min.y),
    );

    let shown = popup::Popup::new(Id::new("node-browser"), at)
        .edge(Theme::border_strong)
        .show(ui.ctx(), theme, |ui| {
            ui.set_width(WIDTH);
            // An `Area` hands its content last frame's size as the room it has, so a
            // list that shrank to a two-hit search opened two rows tall ever after:
            // the scroll area below could never be taller than the area it was
            // measured in. The room is the height this panel may be, every frame.
            ui.set_max_height(height);
            // Above the field rather than in the list: the list is the registry,
            // fuzzy-searched and walked with the arrows, and a row that is not a node
            // in it would be caught by a search for its letters and chosen by Enter.
            // A right-click on the canvas means *something here*, and what is on the
            // clipboard is one of the things it can be.
            if clipboard {
                if ui
                    .add(eframe::egui::Button::new("Paste").min_size(vec2(WIDTH, 0.0)))
                    .clicked()
                {
                    out.paste = true;
                }
                ui.add_space(4.0);
            }
            let field = ui.add(
                TextEdit::singleline(&mut browser.query)
                    .id(Id::new("node-browser-search"))
                    .hint_text(if browser.cable.is_some() {
                        "Search what takes this cable…"
                    } else {
                        "Search nodes…"
                    })
                    .font(FontId::proportional(theme::FONT_BASE))
                    .desired_width(f32::INFINITY),
            );
            if browser.fresh {
                field.request_focus();
            }
            // A keystroke changes the order, so the row the selection names is a
            // different node: back to the best match, which is what was wanted.
            if field.changed() {
                browser.selected = 0;
                browser.follow = true;
            }
            ui.add_space(4.0);
            if hits.is_empty() {
                ui.label(
                    eframe::egui::RichText::new("no node by that name")
                        .size(theme::FONT_TINY)
                        .color(theme.text_muted()),
                );
            }
            // Shrinks vertically: two hits are a two-row panel, not a tall one with
            // a hole under it.
            ScrollArea::vertical()
                .max_height(height - 48.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, def) in hits.iter().enumerate() {
                        let hit = row(ui, def, index == browser.selected, browser.follow, theme);
                        if hit.clicked {
                            out.chosen = Some(def.slug);
                        }
                    }
                });
        });

    // Clicked away, as every popup is. Not on the frame it opened: that click is the
    // right-click that asked for it.
    if !browser.fresh && shown.clicked_away {
        out.dismissed = true;
    }
    browser.fresh = false;
    browser.follow = false;
    out
}

/// What a row answered: the click, and where it was drawn. The Nodes menu decides what the
/// pointer is over from the rect rather than from a hover, because what moves its keyboard
/// selection is what the pointer *crossed*.
pub struct Row {
    pub clicked: bool,
    pub rect: Rect,
}

/// One node in the list: icon, label, and the ports it would arrive with.
///
/// The ports are the reason this is a list of rows rather than a list of labels — silvia's
/// menu shows what a node's shape is before you place it, and choosing by shape is how
/// anyone patching at speed reads a menu.
pub fn row(ui: &mut Ui, def: &NodeDef, selected: bool, follow: bool, theme: &Theme) -> Row {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::click());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
    if selected && follow {
        ui.scroll_to_rect(rect, Some(Align::Center));
    }
    paint_row(
        ui,
        rect,
        selected,
        response.hovered(),
        def.icon,
        def.label,
        theme,
    );
    let painter = ui.painter();

    // Outputs at the right edge, inputs left of them across a divider — the node's own
    // left-to-right, in miniature.
    let mut x = rect.max.x - 8.0;
    for port in def.outputs.iter().rev() {
        glyph(painter, Pos2::new(x, rect.center().y), port.ty, theme);
        x -= DOT * 2.5;
    }
    if !def.outputs.is_empty() && !def.inputs.is_empty() {
        x -= 2.0;
        painter.line_segment(
            [
                Pos2::new(x, rect.center().y - 6.0),
                Pos2::new(x, rect.center().y + 6.0),
            ],
            Stroke::new(1.0, theme.border_strong()),
        );
        x -= 6.0;
    }
    for port in def.inputs.iter().rev() {
        glyph(painter, Pos2::new(x, rect.center().y), port.ty, theme);
        x -= DOT * 2.5;
    }

    response.clone().on_hover_text(def.tooltip);
    let name = format!("add {}", def.slug);
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, &name)
    });
    Row {
        clicked: response.clicked(),
        rect,
    }
}

/// A menu row's ground, icon and label. Two highlights, as silvia has: the keyboard's
/// selection, and `:hover` under the pointer — which is why a pointer resting on one row while
/// the keys walk another reads correctly. Every node row and the Nodes menu's categories draw
/// it, so the two lists are one list to the eye.
pub fn paint_row(
    ui: &Ui,
    rect: Rect,
    selected: bool,
    hovered: bool,
    icon: &str,
    label: &str,
    theme: &Theme,
) {
    let painter = ui.painter();
    if selected || hovered {
        painter.rect_filled(
            rect,
            eframe::egui::CornerRadius::same(theme::RADIUS_SM),
            if selected {
                theme.primary().gamma_multiply(0.25)
            } else {
                theme.bg_hover()
            },
        );
    }
    painter.text(
        rect.left_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        icon,
        theme::icon_font(theme::FONT_BASE, 1.0),
        theme.text_primary(),
    );
    painter.text(
        rect.left_center() + vec2(28.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme::FONT_BASE),
        theme.text_primary(),
    );
}

/// One port of a row's signature, in the shape the node draws it: a circle for a varying value,
/// a diamond for a uniform, a rounded square for an action. The signature is the node in
/// miniature, so it tells the rates apart the way the node does, by shape and not by shade.
fn glyph(painter: &eframe::egui::Painter, c: Pos2, ty: crate::graph::PortType, theme: &Theme) {
    let color = theme.port(ty);
    if ty == crate::graph::PortType::Action {
        let side = DOT * 2.0;
        painter.rect_filled(
            Rect::from_center_size(c, vec2(side, side)),
            eframe::egui::CornerRadius::same((side * 0.25) as u8),
            color,
        );
    } else if ty.is_uniform() {
        let r = DOT * 1.4;
        painter.add(eframe::egui::Shape::convex_polygon(
            vec![
                Pos2::new(c.x, c.y - r),
                Pos2::new(c.x + r, c.y),
                Pos2::new(c.x, c.y + r),
                Pos2::new(c.x - r, c.y),
            ],
            color,
            Stroke::NONE,
        ));
    } else {
        painter.circle_filled(c, DOT, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three tiers, in order: what starts with the query, what has a word starting with
    /// it, and what merely contains its letters in order.
    #[test]
    fn a_prefix_beats_a_word_start_beats_a_scattered_match() {
        let prefix = score("che", "Checkerboard", "checkerboard");
        let word = score("che", "Edge checker", "edgechecker");
        let scattered = score("che", "Color hue edges", "colorhueedges");
        assert!(scattered > 0, "c-h-e in order is a match");
        assert!(word > scattered, "{word} must beat {scattered}");
        assert!(prefix > word, "{prefix} must beat {word}");
    }

    #[test]
    fn a_run_beats_the_same_letters_spread_out() {
        let run = subsequence("abc", "xabc");
        let spread = subsequence("abc", "axbxc");
        assert!(run > spread, "{run} must beat {spread}");
    }

    #[test]
    fn what_is_not_a_subsequence_does_not_match() {
        assert_eq!(score("zzz", "Checkerboard", "checkerboard"), 0);
        assert_eq!(score("", "Checkerboard", "checkerboard"), 0);
    }

    #[test]
    fn the_slug_is_searched_as_well_as_the_label() {
        assert!(score("audioin", "Audio in", "audioin") > 0);
    }

    #[test]
    fn an_empty_query_is_every_node_in_label_order() {
        let all = matches("");
        assert_eq!(all.len(), REGISTRY.iter().filter(|d| (d.offered)()).count());
        let labels: Vec<_> = all.iter().map(|d| d.label).collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        assert_eq!(labels, sorted);
    }

    /// A kind this machine cannot offer is never listed, however it is asked for: Syphon on
    /// Linux.
    #[test]
    fn what_the_machine_cannot_offer_is_not_listed() {
        let listed = matches("syphon").iter().any(|d| d.slug == "syphon");
        assert_eq!(listed, crate::platform::syphon::available());
        assert_eq!(
            matches("").iter().any(|d| d.slug == "syphon"),
            crate::platform::syphon::available()
        );
    }

    /// A cable's browser lists its takers and nothing else, searched as the full list is.
    #[test]
    fn a_cables_list_is_its_takers() {
        let only = [
            Taker {
                slug: "output",
                key: "input",
            },
            Taker {
                slug: "invert",
                key: "input",
            },
        ];
        let all: Vec<_> = matches_within("", Some(&only))
            .iter()
            .map(|d| d.slug)
            .collect();
        assert_eq!(all, ["invert", "output"], "label order, and only these");
        let hits: Vec<_> = matches_within("out", Some(&only))
            .iter()
            .map(|d| d.slug)
            .collect();
        assert_eq!(hits, ["output"]);
    }

    #[test]
    fn a_query_puts_what_starts_with_it_first() {
        let hits = matches("out");
        assert_eq!(
            hits.first().map(|d| d.slug),
            Some("output"),
            "the node whose name starts with the query is not first"
        );
    }
}
