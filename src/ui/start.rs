// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Nodes menu: a start button in the bottom-left corner, and the library above it.
//!
//! This is a port of silvia's start menu, and the part worth porting is not the list — it is
//! *when* things happen. Read it as three rules:
//!
//! **The mouse speaks in transitions, not in positions.** A browser fires `mouseenter`,
//! `mouseleave` and `mouseover` when the pointer *crosses* something; a pointer sitting still
//! says nothing at all. Every timer and every selection change below hangs off a crossing,
//! which is what `Over` and `StartMenu::over` are for. Written against "is the pointer over
//! it now?" instead, the menu gets two faults that feel like one: a submenu opened by the
//! keyboard closes itself 300 ms later because the pointer is not over anything, and a
//! pointer parked anywhere over the menu drags the selection back under itself on the next
//! frame, so the arrow keys cannot move.
//!
//! **Two timers, and they are asymmetric on purpose.** A category the pointer enters opens
//! after 200 ms, so crossing three on the way to a fourth flashes nothing open. A submenu
//! survives the pointer *leaving* it, or leaving its category, by 300 ms — the pointer can
//! cut the diagonal to the far corner of a submenu instead of tracking along the narrow
//! valley a menu without that delay demands. Neither timer ever runs while the pointer is
//! still.
//!
//! **The keyboard is Windows's.** The menu opens with nothing selected, so `Down` is the
//! first entry and `Up` the last; both wrap; `Right`/`Enter` go in and `Left`/`Escape` come
//! back out; `Escape` outside a submenu closes the menu. Opening a submenu with the pointer
//! moves the keyboard into it with nothing selected, so `Down` then walks its entries — that
//! is silvia's `onSubmenuShown`, and it is why hovering and typing compose.
//!
//! **A category of one node is that node.** Output is the only one: a submenu holding a
//! single entry of the same name is a step that chooses nothing, so its row is the node
//! itself, with no ▶, and a click or `Enter` adds it.

use crate::nodes::{Category, NodeDef, REGISTRY};
use crate::ui::icon::{self, Icon};
use crate::ui::theme::{self, Theme};
use crate::ui::{browse, popup};
use eframe::egui::{
    Area, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Response, ScrollArea, Sense,
    Stroke, Ui, UiBuilder, vec2,
};

/// How long the pointer has to rest on a category before its submenu opens.
const OPEN_DELAY: f64 = 0.2;
/// How long an open submenu survives the pointer leaving it or its category.
const CLOSE_DELAY: f64 = 0.3;
const ROW: f32 = 20.0;
/// Width of the category column. silvia's 16rem.
const WIDTH: f32 = 220.0;
/// The button, and how far it floats off the canvas's bottom-left corner.
const BUTTON_HEIGHT: f32 = 18.0;
const BUTTON_PAD: f32 = 10.0;
/// The square the button's triangle is painted in, and the space between it and the word.
const BUTTON_ICON: f32 = 12.0;
const BUTTON_ICON_GAP: f32 = 4.0;
/// The square a category's submenu triangle is painted in, and its margin from the row's end.
const MORE_ICON: f32 = 12.0;
const MORE_MARGIN: f32 = 6.0;
const MARGIN: f32 = 8.0;
/// The popup frame's own padding, which the panel's height has to be known without measuring.
const FRAME_PAD: f32 = 4.0;

/// What the pointer was over at the end of last frame.
///
/// The whole point of this type: a crossing is *this* frame's answer differing from last
/// frame's. `Submenu(None)` is the submenu's padding and scrollbar — inside it, on no entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Over {
    #[default]
    Nothing,
    Category(usize),
    Submenu(Option<usize>),
}

impl Over {
    fn in_submenu(self) -> bool {
        matches!(self, Self::Submenu(_))
    }
}

/// The Nodes menu, across frames. Every field is session state: nothing here is in the file.
// Five independent questions the menu asks itself, and none of them is a mode the others
// belong to.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct StartMenu {
    open: bool,
    /// The category whose submenu is up.
    submenu: Option<usize>,
    /// A category the pointer has entered, and when its submenu is due.
    pending: Option<(usize, f64)>,
    /// When the open submenu is due to close, unless the pointer comes back first.
    closing: Option<f64>,
    /// Which category is selected, `None` being the "nothing selected" a Windows menu opens
    /// in — the state that makes `Down` mean *first* and `Up` mean *last*.
    cursor: Option<usize>,
    /// Which entry of the open submenu is selected.
    item: Option<usize>,
    /// The keyboard is in the submenu rather than in the category column.
    inside: bool,
    /// The selection moved by key, so the list scrolls to it.
    follow: bool,
    /// The frame it opened on: the click that opened it is not also the click that shuts it.
    fresh: bool,
    /// It opened, and the canvas's browser has not been told yet. One menu at a time.
    just_opened: bool,
    /// What the pointer was over last frame, which is the whole mouse model.
    over: Over,
    /// The frame the keys were last read on. egui may run a frame's UI more than once — a
    /// discarded pass and then the real one — replaying the same input each time, and a menu
    /// that steps its selection on every pass moves two entries for one press.
    keys_at: Option<u64>,
}

impl StartMenu {
    pub fn close(&mut self) {
        *self = Self::default();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Did it open since this was last asked? Read by `App`, which closes the other menu.
    pub fn take_just_opened(&mut self) -> bool {
        std::mem::take(&mut self.just_opened)
    }

    fn show_menu(&mut self) {
        self.close();
        self.open = true;
        self.fresh = true;
        self.just_opened = true;
    }

    /// silvia's `hideSubmenu`, with the `onSubmenuHidden` that follows it: the submenu goes,
    /// the keyboard comes back to the categories, and the category selection stays where it
    /// was.
    fn hide_submenu(&mut self) {
        self.submenu = None;
        self.item = None;
        self.inside = false;
        self.pending = None;
        self.closing = None;
    }

    /// silvia's `showSubmenu` with its `onSubmenuShown`: opening a submenu with the pointer
    /// takes the keyboard into it with nothing selected, so `Down` walks its entries.
    fn show_submenu(&mut self, index: usize) {
        self.submenu = Some(index);
        self.pending = None;
        self.closing = None;
        self.inside = true;
        self.item = None;
    }
}

/// The categories that have members, with them. Driven by the registry, so a new node file
/// reaches the menu without this function knowing it exists — and only what this machine
/// offers (`NodeDef::offered`).
fn categories() -> Vec<(Category, Vec<&'static NodeDef>)> {
    Category::ALL
        .iter()
        .filter_map(|category| {
            let members: Vec<&'static NodeDef> = REGISTRY
                .iter()
                .filter(|d| d.category == *category && (d.offered)())
                .copied()
                .collect();
            (!members.is_empty()).then_some((*category, members))
        })
        .collect()
}

/// The node a category of one member stands for, which its row adds directly.
fn leaf(members: &[&'static NodeDef]) -> Option<&'static NodeDef> {
    match members {
        [only] => Some(*only),
        _ => None,
    }
}

/// Draw the start button in `within`'s bottom-left corner and, while it is open, the menu
/// above it. Returns a node kind to add.
pub fn show(
    ui: &mut Ui,
    menu: &mut StartMenu,
    within: Rect,
    theme: &Theme,
) -> Option<&'static str> {
    let (button, button_rect) = button(ui, menu, within, theme);
    let toggled = button.clicked()
        // `n`, which is silvia's Windows key. Not while anything is taking typing.
        || (!ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|i| i.consume_shortcut(&crate::ui::menu::keys::NODES)));
    if toggled {
        if menu.open {
            menu.close();
        } else {
            menu.show_menu();
        }
    }
    if !menu.open {
        return None;
    }

    let cats = categories();
    let now = ui.input(|i| i.time);
    let pointer = ui.ctx().pointer_latest_pos();
    let mut chosen = None;

    keys(ui, menu, &cats, &mut chosen);
    if !menu.open {
        return chosen;
    }

    // --- what the timers say, before anything is drawn -------------------------------------
    // Before, not after: a submenu whose delay expired during this frame belongs on screen in
    // this frame. Checked after the drawing it opens a frame late, which is exactly the
    // sixteen milliseconds these delays exist to spend well.
    if let Some((index, due)) = menu.pending {
        if now >= due {
            // A leaf has no submenu of its own, but resting on it still puts away the one
            // that is up, as resting on any other category would.
            if cats.get(index).is_some_and(|(_, m)| leaf(m).is_some()) {
                menu.hide_submenu();
            } else {
                menu.show_submenu(index);
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(due - now));
        }
    }
    if let Some(due) = menu.closing {
        if now >= due {
            menu.hide_submenu();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(due - now));
        }
    }

    // --- the categories and the submenu, in one area ---------------------------------------
    // One `Area`, not two. An `Area` reports the rect it had when it was last laid out, so a
    // second one placed from the first lands a frame behind — visibly, on the frame a submenu
    // opens, which is the flash the timers exist to prevent. A child `Ui` placed beside the
    // category's own row is laid out now.
    //
    // The height is computed rather than measured for the same reason: the menu stands on the
    // button, so it is positioned by its bottom edge, and a height that is last frame's would
    // put this frame's menu in the wrong place.
    let height = (cats.len() as f32).mul_add(ROW, FRAME_PAD * 2.0 + 2.0);
    let top = (button_rect.min.y - height).max(within.min.y);
    let mut over = Over::Nothing;
    // The first time an `Area` is shown egui lays it out once to measure it — a *sizing
    // pass*, at a position it has not worked out yet, which it then discards and runs again.
    // The geometry of that pass is not where anything will be drawn, so the mouse model must
    // not read it: reacting to it selects whatever category the provisional layout happened
    // to put under the pointer.
    let mut sizing = false;
    popup::area(Id::new("nodes-menu"), Pos2::new(button_rect.min.x, top))
        // Unconstrained, because the geometry above is the geometry: a submenu taller than
        // the category column reaches above the area's own origin, and egui's constraint
        // answers that by sliding the whole area down — which moves the categories out from
        // under the pointer on the frame their submenu opens.
        .constrain(false)
        .show(ui.ctx(), |ui| {
            sizing = ui.is_sizing_pass();
            let mut rows: Vec<Rect> = Vec::with_capacity(cats.len());
            popup::frame(ui.style(), theme, Theme::border_strong)
                .inner_margin(FRAME_PAD)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // Flush, like every menu: the height above is computed from the rows, and
                    // a gap between them would put the panel's bottom edge over the button.
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, (category, members)) in cats.iter().enumerate() {
                        let leaf = leaf(members);
                        let (rect, clicked) = category_row(ui, *category, leaf, index, menu, theme);
                        rows.push(rect);
                        if clicked && let Some(def) = leaf {
                            chosen = Some(def.slug);
                        }
                    }
                });
            if let Some(p) = pointer
                && let Some(index) = rows.iter().position(|r| r.contains(p))
            {
                over = Over::Category(index);
            }

            // The submenu, level with the category that owns it and pushed up when it would
            // run off the bottom.
            if let Some(index) = menu.submenu
                && let (Some(anchor), Some((_, members))) = (rows.get(index), cats.get(index))
            {
                let screen = ui.ctx().content_rect();
                let tall = (members.len() as f32)
                    .mul_add(ROW, FRAME_PAD * 2.0 + 2.0)
                    .min(screen.height() - MARGIN * 2.0);
                let at = Rect::from_min_size(
                    Pos2::new(
                        anchor.max.x + FRAME_PAD,
                        anchor
                            .min
                            .y
                            .min(screen.max.y - tall - MARGIN)
                            .max(screen.min.y + MARGIN),
                    ),
                    vec2(browse::WIDTH + FRAME_PAD * 2.0, tall),
                );
                if pointer.is_some_and(|p| at.contains(p)) {
                    over = Over::Submenu(None);
                }
                ui.scope_builder(UiBuilder::new().max_rect(at), |ui| {
                    popup::frame(ui.style(), theme, Theme::border_strong)
                        .inner_margin(FRAME_PAD)
                        .show(ui, |ui| {
                            ui.set_width(browse::WIDTH);
                            ScrollArea::vertical()
                                .max_height(tall)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for (i, def) in members.iter().enumerate() {
                                        let selected = menu.inside && menu.item == Some(i);
                                        let row =
                                            browse::row(ui, def, selected, menu.follow, theme);
                                        if pointer.is_some_and(|p| row.rect.contains(p)) {
                                            over = Over::Submenu(Some(i));
                                        }
                                        if row.clicked {
                                            chosen = Some(def.slug);
                                        }
                                    }
                                });
                        });
                });
            }
        });

    if !sizing {
        crossings(menu, over, now);
    }

    // Clicked away, the rule every popup here follows.
    if !menu.fresh
        && ui.ctx().input(|i| i.pointer.any_click())
        && over == Over::Nothing
        && !button.contains_pointer()
    {
        menu.close();
    }
    if chosen.is_some() {
        menu.close();
    }
    menu.fresh = false;
    menu.follow = false;
    chosen
}

/// What the pointer crossed between last frame and this one, and what silvia does about it.
///
/// Every rule here is a DOM handler in `menu.js`, in the same order:
///
/// - `categoryItemEl.mouseenter` — cancel the close, cancel any *other* pending open, and
///   start this one's 200 ms unless its submenu is already up.
/// - `categoryItemEl.mouseleave` — cancel this one's pending open, and start the 300 ms close
///   if its submenu is up.
/// - `submenuEl.mouseenter` / `mouseleave` — cancel the close; start the close.
/// - `KeyboardNavigationManager.handleMouseOver` — the pointer moving onto a category moves
///   the selection there, but **only while the keyboard is in the category column**, and the
///   pointer moving onto an entry moves it there **only while the keyboard is in the
///   submenu**. Crossing, not resting: a still pointer never takes the selection back.
fn crossings(menu: &mut StartMenu, over: Over, now: f64) {
    let was = menu.over;
    if was == over {
        return;
    }
    menu.over = over;

    // Left a category.
    if let Over::Category(index) = was {
        if menu.pending.map(|(p, _)| p) == Some(index) {
            menu.pending = None;
        }
        if menu.submenu == Some(index) && menu.closing.is_none() {
            menu.closing = Some(now + CLOSE_DELAY);
        }
    }
    // Left the submenu.
    if was.in_submenu() && !over.in_submenu() && menu.closing.is_none() {
        menu.closing = Some(now + CLOSE_DELAY);
    }

    match over {
        Over::Category(index) => {
            menu.closing = None;
            if menu.pending.map(|(p, _)| p) != Some(index) {
                menu.pending = None;
            }
            if menu.submenu != Some(index) && menu.pending.is_none() {
                menu.pending = Some((index, now + OPEN_DELAY));
            }
            if !menu.inside {
                menu.cursor = Some(index);
            }
        }
        Over::Submenu(entry) => {
            menu.closing = None;
            if menu.inside
                && let Some(i) = entry
            {
                menu.item = Some(i);
            }
        }
        Over::Nothing => {}
    }
}

/// One category: icon, name, and the ▶ that says it has a submenu — or, for a category of
/// one, the node it stands for, named by the node and with no ▶. Returns its rect, and
/// whether it was clicked, which only a leaf answers.
fn category_row(
    ui: &mut Ui,
    category: Category,
    leaf: Option<&'static NodeDef>,
    index: usize,
    menu: &StartMenu,
    theme: &Theme,
) -> (Rect, bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::click());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
    let selected = menu.cursor == Some(index) && !menu.inside;
    let label = leaf.map_or(category.label(), |def| def.label);
    browse::paint_row(
        ui,
        rect,
        selected,
        response.hovered(),
        category.icon(),
        label,
        theme,
    );
    if leaf.is_none() {
        icon::paint(
            ui.painter(),
            Rect::from_center_size(
                rect.right_center() - vec2(MORE_MARGIN + MORE_ICON * 0.5, 0.0),
                vec2(MORE_ICON, MORE_ICON),
            ),
            Icon::TriangleRight,
            theme.primary_muted(),
        );
    }
    let name = format!("{} {label}", category.icon());
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, &name)
    });
    (rect, leaf.is_some() && response.clicked())
}

/// The start button's area.
const BUTTON: &str = "nodes-button";

/// The start button: bottom-left, floating over the canvas, and square on top while the menu
/// stands on it — silvia's `#nodes-menu-btn.menu-open`, which is what makes the two read as
/// one object.
///
/// Returns its rect as well as its response, and the menu is placed from *that* rather than
/// from `Response::rect`: the button lives in an `Area`, an `Area` reports the rect it had
/// when it was last laid out, and a menu standing on last frame's button lands in the wrong
/// place on the frame the window changes size.
fn button(ui: &mut Ui, menu: &StartMenu, within: Rect, theme: &Theme) -> (Response, Rect) {
    let font = FontId::proportional(theme::FONT_BASE);
    let mark = if menu.open {
        Icon::TriangleDown
    } else {
        Icon::TriangleUp
    };
    let galley = ui
        .painter()
        .layout_no_wrap("Nodes".to_owned(), font, theme.text_primary());
    let lead = BUTTON_ICON + BUTTON_ICON_GAP;
    let size = vec2(lead + galley.size().x + BUTTON_PAD * 2.0, BUTTON_HEIGHT);
    let at = Pos2::new(within.min.x + MARGIN, within.max.y - MARGIN - BUTTON_HEIGHT);
    let response = Area::new(Id::new(BUTTON))
        // **`Background`: above the canvas this is furniture for, below anything floating
        // over it.** At `Foreground` the button painted over the Status box and the MIDI
        // window, which are `egui::Window` and therefore `Order::Middle`. `Middle` is not
        // the fix — same-order layers fall back to insertion order and this one still won.
        //
        // An `Area` at `Background` is still above the central panel the canvas paints into,
        // because it is a later layer at the same order, so the button keeps floating over
        // the nodes. The *menu* stays `Foreground`: it is open only while a hand is in it,
        // and a window over the list of nodes is worse than one over the thing that opened
        // it.
        .order(Order::Background)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
            let lit = response.hovered() || menu.open;
            let radius = if menu.open {
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: theme::RADIUS_SM,
                    se: theme::RADIUS_SM,
                }
            } else {
                CornerRadius::same(theme::RADIUS_SM)
            };
            ui.painter().rect_filled(
                rect,
                radius,
                if lit {
                    theme.bg_hover()
                } else {
                    theme.bg_secondary()
                },
            );
            ui.painter().rect_stroke(
                rect,
                radius,
                Stroke::new(
                    1.0,
                    if lit {
                        theme.primary_muted()
                    } else {
                        theme.border_subtle()
                    },
                ),
                eframe::egui::StrokeKind::Inside,
            );
            let content = lead + galley.size().x;
            let left = rect.center().x - content * 0.5;
            icon::paint(
                ui.painter(),
                Rect::from_center_size(
                    Pos2::new(left + BUTTON_ICON * 0.5, rect.center().y),
                    vec2(BUTTON_ICON, BUTTON_ICON),
                ),
                mark,
                theme.text_primary(),
            );
            ui.painter().galley(
                Pos2::new(left + lead, rect.center().y - galley.size().y * 0.5),
                galley,
                theme.text_primary(),
            );
            crate::ui::accessible(&response, eframe::egui::WidgetType::Button, "Nodes");
            response
        })
        .inner;
    // The two keys that reach the library without the pointer, said where the pointer
    // reaches it. Not while the menu stands on the button: it would cover the first row.
    let response = if menu.open {
        response
    } else {
        let key = |k| crate::ui::menu::said(ui.ctx(), &k);
        response.on_hover_text(format!(
            "The node library. {} opens this menu; {} searches it at the top of the canvas.",
            key(crate::ui::menu::keys::NODES),
            key(crate::ui::menu::keys::BROWSER),
        ))
    };
    (response, Rect::from_min_size(at, size))
}

/// Windows's keys: nothing selected to begin with, `Down` for the first entry and `Up` for
/// the last, wrapping at both ends, `Right`/`Enter` in and `Left`/`Escape` out.
///
/// Nothing here reads the pointer, and nothing the pointer does while it is still can undo
/// what this decided — that is the half of the menu people notice when it is missing.
fn keys(
    ui: &Ui,
    menu: &mut StartMenu,
    cats: &[(Category, Vec<&'static NodeDef>)],
    chosen: &mut Option<&'static str>,
) {
    let frame = ui.ctx().cumulative_frame_nr();
    if menu.keys_at == Some(frame) {
        return;
    }
    menu.keys_at = Some(frame);
    let members = menu
        .submenu
        .and_then(|i| cats.get(i))
        .map_or(0, |(_, m)| m.len());
    let mut enter = false;
    let mut right = false;
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            if menu.inside {
                menu.hide_submenu();
            } else {
                menu.close();
            }
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            menu.follow = true;
            if menu.inside {
                menu.item = Some(step(menu.item, members, 1));
            } else {
                menu.cursor = Some(step(menu.cursor, cats.len(), 1));
            }
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            menu.follow = true;
            if menu.inside {
                menu.item = Some(step(menu.item, members, -1));
            } else {
                menu.cursor = Some(step(menu.cursor, cats.len(), -1));
            }
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) && menu.inside {
            menu.hide_submenu();
        }
        enter = i.consume_key(Modifiers::NONE, Key::Enter);
        right = i.consume_key(Modifiers::NONE, Key::ArrowRight);
    });
    if !(enter || right) || !menu.open {
        return;
    }
    if menu.inside {
        // Nothing selected takes nothing: a submenu opened by hovering has no entry under the
        // keyboard until `Down` puts one there.
        if let Some(def) = menu
            .submenu
            .and_then(|i| cats.get(i))
            .and_then(|(_, m)| menu.item.and_then(|k| m.get(k)))
        {
            *chosen = Some(def.slug);
        }
    } else if let Some(index) = menu.cursor {
        // A leaf has nothing to go into: `Enter` takes it, and `Right` does nothing, as on
        // a Windows menu entry with no submenu.
        if let Some(def) = cats.get(index).and_then(|(_, m)| leaf(m)) {
            if enter {
                *chosen = Some(def.slug);
            }
            return;
        }
        // Going in by key selects the first entry, where going in by pointer selects none.
        menu.show_submenu(index);
        menu.item = Some(0);
        menu.follow = true;
    }
}

/// One step through a list of `len`, wrapping. From nothing selected, forwards is the first
/// entry and backwards is the last — Windows's answer, and the reason this is not `+ 1`.
fn step(from: Option<usize>, len: usize, by: isize) -> usize {
    if len == 0 {
        return 0;
    }
    match from {
        None if by > 0 => 0,
        None => len - 1,
        Some(at) if by > 0 => (at + 1) % len,
        Some(at) => (at + len - 1) % len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_selected_goes_to_the_first_or_the_last() {
        assert_eq!(step(None, 5, 1), 0);
        assert_eq!(step(None, 5, -1), 4);
    }

    #[test]
    fn it_wraps_at_both_ends() {
        assert_eq!(step(Some(4), 5, 1), 0);
        assert_eq!(step(Some(0), 5, -1), 4);
    }

    #[test]
    fn an_empty_list_stays_put() {
        assert_eq!(step(None, 0, 1), 0);
        assert_eq!(step(Some(0), 0, -1), 0);
    }

    #[test]
    fn every_category_with_members_is_offered() {
        let cats = categories();
        assert_eq!(cats.len(), Category::ALL.len(), "a category has no nodes");
        assert!(cats.iter().all(|(_, m)| !m.is_empty()));
    }

    /// Output, a category of one, is offered as the node; every other category opens.
    #[test]
    fn only_output_is_a_leaf() {
        for (category, members) in categories() {
            assert_eq!(
                leaf(&members).map(|d| d.slug),
                (category == Category::Output).then_some("output"),
                "{}",
                category.label()
            );
        }
    }

    /// A pointer that has not moved says nothing, so no timer starts and no selection moves.
    /// This is the whole difference between a menu the keyboard can drive and one it cannot.
    #[test]
    fn a_still_pointer_crosses_nothing() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Category(2),
            cursor: Some(4),
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Category(2), 10.0);
        assert_eq!(
            menu.cursor,
            Some(4),
            "a still pointer took the selection back"
        );
        assert_eq!(menu.pending, None, "a still pointer started an open timer");
    }

    /// Nothing the keyboard does starts a close timer, so a submenu opened with `Right` stays
    /// up however long the pointer sits elsewhere.
    #[test]
    fn the_keyboard_never_starts_a_close() {
        let mut menu = StartMenu {
            open: true,
            cursor: Some(1),
            ..StartMenu::default()
        };
        menu.show_submenu(1);
        menu.item = Some(0);
        for t in 0..10 {
            crossings(&mut menu, Over::Nothing, f64::from(t));
        }
        assert_eq!(menu.closing, None, "the keyboard armed the close timer");
        assert_eq!(menu.submenu, Some(1), "the submenu closed itself");
    }

    /// Crossing onto another category does not close the open submenu: entering one cancels
    /// the close, and the new one waits its 200 ms before it replaces what is there. That is
    /// what stops a menu flashing on the way past.
    #[test]
    fn crossing_another_category_does_not_close_the_open_one() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Category(1),
            submenu: Some(1),
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Category(2), 1.0);
        assert_eq!(menu.closing, None, "passing over one closed the other");
        assert_eq!(menu.submenu, Some(1), "it closed on the way past");
        assert_eq!(
            menu.pending,
            Some((2, 1.0 + OPEN_DELAY)),
            "the category crossed on the way did not wait its turn"
        );
    }

    /// Leaving the menu arms the close; reaching the submenu disarms it. That pair is the
    /// diagonal, and it is the whole reason the delay is 300 ms rather than nothing.
    #[test]
    fn the_diagonal_to_a_submenu_keeps_it_open() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Category(1),
            submenu: Some(1),
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Nothing, 1.0);
        assert_eq!(
            menu.closing,
            Some(1.0 + CLOSE_DELAY),
            "leaving the menu armed nothing"
        );

        crossings(&mut menu, Over::Submenu(Some(0)), 1.1);
        assert_eq!(menu.closing, None, "reaching the submenu did not disarm it");
        assert_eq!(menu.submenu, Some(1), "it closed on the way");
    }

    /// The pointer moves the selection only in the half the keyboard is in.
    #[test]
    fn the_pointer_moves_the_selection_only_where_the_keyboard_is() {
        let mut menu = StartMenu {
            open: true,
            submenu: Some(0),
            inside: true,
            item: Some(3),
            cursor: Some(0),
            ..StartMenu::default()
        };
        // In the submenu, a category the pointer crosses does not take the selection.
        crossings(&mut menu, Over::Category(2), 1.0);
        assert_eq!(menu.item, Some(3));
        assert_eq!(menu.cursor, Some(0), "the pointer stole the selection");

        // Out of it, one does.
        menu.inside = false;
        menu.over = Over::Nothing;
        crossings(&mut menu, Over::Category(2), 2.0);
        assert_eq!(menu.cursor, Some(2));
    }
}
