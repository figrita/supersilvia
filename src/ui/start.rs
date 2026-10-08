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
//! **A category of one node is that node.** A submenu holding a single entry of the same name
//! is a step that chooses nothing, so the Output's row is the node itself, with no ▶, and a
//! click or `Enter` adds it.
//!
//! **The menu is a tree, and its shape is the workspace's kind** ([`library::menu`]): on a
//! video workspace Solids is a category whose submenu holds Render 3D and the solid families,
//! each a submenu of its own, and on a solids workspace the families stand at the top and
//! Video holds the video categories. Every column past the first is a submenu, and the rules
//! above hold in each: an entry that opens one opens it on the same timer, and the keyboard is
//! in one column at a time.

use crate::graph::WorkspaceKind;
use crate::nodes::library::{self, Entry};
use crate::ui::icon::{self, Icon};
use crate::ui::theme::{self, Theme};
use crate::ui::{browse, popup};
use eframe::egui::{
    Align, Area, CornerRadius, FontId, Id, Key, Modifiers, Order, Pos2, Rect, Response, ScrollArea,
    Sense, Stroke, Ui, UiBuilder, vec2,
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
/// frame's. A column is the categories at 0 and each submenu one further; an entry of `None`
/// is a submenu's padding and scrollbar — inside it, on no entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Over {
    #[default]
    Nothing,
    Entry(usize, Option<usize>),
}

/// The Nodes menu, across frames. Every field is session state: nothing here is in the file.
// Four independent questions the menu asks itself, and none of them is a mode the others
// belong to.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Default)]
pub struct StartMenu {
    open: bool,
    /// The open submenus as a chain: in each column from the categories on, the entry whose
    /// submenu is up.
    path: Vec<usize>,
    /// An entry the pointer has entered, by column and index, and when its submenu is due.
    pending: Option<(usize, usize, f64)>,
    /// When every submenu past the first `keep` is due to close, unless the pointer comes back
    /// first.
    closing: Option<(usize, f64)>,
    /// The column the keyboard is in: 0 is the categories, 1 the submenu they opened.
    depth: usize,
    /// Which entry is selected in each column up to the keyboard's, `None` being the "nothing
    /// selected" a Windows menu opens in — the state that makes `Down` mean *first* and `Up`
    /// mean *last*.
    cursor: Vec<Option<usize>>,
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

    /// The entry selected in `column`.
    fn selected(&self, column: usize) -> Option<usize> {
        self.cursor.get(column).copied().flatten()
    }

    fn select(&mut self, column: usize, entry: Option<usize>) {
        if self.cursor.len() <= column {
            self.cursor.resize(column + 1, None);
        }
        self.cursor[column] = entry;
    }

    /// silvia's `hideSubmenu`, with the `onSubmenuHidden` that follows it, for every submenu
    /// past the first `keep`: they go, the keyboard comes back to the deepest column left, and
    /// the selection there stays where it was.
    fn hide_from(&mut self, keep: usize) {
        self.path.truncate(keep);
        self.depth = self.depth.min(keep);
        self.cursor.truncate(self.depth + 1);
        self.pending = None;
        self.closing = None;
    }

    /// silvia's `showSubmenu` with its `onSubmenuShown`, for the entry `index` of `column`:
    /// whatever that column had open goes, and opening a submenu with the pointer takes the
    /// keyboard into it with nothing selected, so `Down` walks its entries.
    fn show_submenu(&mut self, column: usize, index: usize) {
        self.path.truncate(column);
        self.path.push(index);
        self.pending = None;
        self.closing = None;
        self.depth = column + 1;
        self.cursor.resize(column + 1, None);
        self.cursor.push(None);
    }
}

/// The entries of the column `path` leads to: the categories for an empty path, and each index
/// one submenu further.
fn column<'a>(tree: &'a [Entry], path: &[usize]) -> Option<&'a [Entry]> {
    path.iter()
        .try_fold(tree, |entries, i| entries.get(*i)?.entries())
}

/// Draw the start button in `within`'s bottom-left corner and, while it is open, the menu
/// above it, shaped for a workspace of `kind`. Returns a node kind to add.
pub fn show(
    ui: &mut Ui,
    menu: &mut StartMenu,
    within: Rect,
    theme: &Theme,
    kind: WorkspaceKind,
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

    let tree = library::menu(kind);
    let now = ui.input(|i| i.time);
    let pointer = ui.ctx().pointer_latest_pos();
    let mut chosen = None;

    keys(ui, menu, &tree, &mut chosen);
    if !menu.open {
        return chosen;
    }

    // --- what the timers say, before anything is drawn -------------------------------------
    // Before, not after: a submenu whose delay expired during this frame belongs on screen in
    // this frame. Checked after the drawing it opens a frame late, which is exactly the
    // sixteen milliseconds these delays exist to spend well.
    if let Some((at, index, due)) = menu.pending {
        if now >= due {
            // A node has no submenu of its own, but resting on it still puts away the one
            // its column has up, as resting on any other entry would.
            let opens = at <= menu.path.len()
                && column(&tree, &menu.path[..at])
                    .and_then(|c| c.get(index))
                    .is_some_and(|e| e.entries().is_some());
            if opens {
                menu.show_submenu(at, index);
            } else {
                menu.hide_from(at.min(menu.path.len()));
            }
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(due - now));
        }
    }
    if let Some((keep, due)) = menu.closing {
        if now >= due {
            menu.hide_from(keep);
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(due - now));
        }
    }

    // --- the categories and the submenus, in one area --------------------------------------
    // One `Area`, not one each. An `Area` reports the rect it had when it was last laid out, so
    // a second one placed from the first lands a frame behind — visibly, on the frame a submenu
    // opens, which is the flash the timers exist to prevent. A child `Ui` placed beside the
    // row that owns it is laid out now.
    //
    // The height is computed rather than measured for the same reason: the menu stands on the
    // button, so it is positioned by its bottom edge, and a height that is last frame's would
    // put this frame's menu in the wrong place.
    let height = (tree.len() as f32).mul_add(ROW, FRAME_PAD * 2.0 + 2.0);
    let top = (button_rect.min.y - height).max(within.min.y);
    let mut over = Over::Nothing;
    // An entry with a submenu that was clicked: it opens at once rather than on the timer.
    let mut opened = None;
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
            let mut rows: Vec<Vec<Rect>> = Vec::with_capacity(menu.path.len() + 1);
            let mut first: Vec<Rect> = Vec::with_capacity(tree.len());
            popup::frame(ui.style(), theme, Theme::border_strong)
                .inner_margin(FRAME_PAD)
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    // Flush, like every menu: the height above is computed from the rows, and
                    // a gap between them would put the panel's bottom edge over the button.
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (index, entry) in tree.iter().enumerate() {
                        let selected = menu.depth == 0 && menu.selected(0) == Some(index);
                        let (rect, clicked) =
                            category_row(ui, entry, WIDTH, selected, false, theme);
                        first.push(rect);
                        if clicked {
                            match entry {
                                Entry::Node(def) => chosen = Some(def.slug),
                                Entry::Group(_) => opened = Some((0, index)),
                            }
                        }
                    }
                });
            if let Some(p) = pointer
                && let Some(index) = first.iter().position(|r| r.contains(p))
            {
                over = Over::Entry(0, Some(index));
            }
            rows.push(first);

            // Each submenu, level with the row that owns it and pushed up when it would run
            // off the bottom.
            let screen = ui.ctx().content_rect();
            for at in 1..=menu.path.len() {
                let (Some(anchor), Some(entries)) = (
                    rows[at - 1].get(menu.path[at - 1]).copied(),
                    column(&tree, &menu.path[..at]),
                ) else {
                    break;
                };
                let tall = (entries.len() as f32)
                    .mul_add(ROW, FRAME_PAD * 2.0 + 2.0)
                    .min(screen.height() - MARGIN * 2.0);
                let wide = browse::WIDTH + FRAME_PAD * 2.0;
                // Beside its row, or as far right as the window lets it be: a submenu two deep
                // in a narrow window covers part of the one it came from rather than running
                // off the edge.
                let place = Rect::from_min_size(
                    Pos2::new(
                        (anchor.max.x + FRAME_PAD).min(screen.max.x - wide - MARGIN),
                        anchor
                            .min
                            .y
                            .min(screen.max.y - tall - MARGIN)
                            .max(screen.min.y + MARGIN),
                    ),
                    vec2(wide, tall),
                );
                if pointer.is_some_and(|p| place.contains(p)) {
                    over = Over::Entry(at, None);
                }
                let mut these: Vec<Rect> = Vec::with_capacity(entries.len());
                ui.scope_builder(UiBuilder::new().max_rect(place), |ui| {
                    popup::frame(ui.style(), theme, Theme::border_strong)
                        .inner_margin(FRAME_PAD)
                        .show(ui, |ui| {
                            ui.set_width(browse::WIDTH);
                            ScrollArea::vertical()
                                .id_salt(("nodes-menu-submenu", at))
                                .max_height(tall)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for (i, entry) in entries.iter().enumerate() {
                                        let selected =
                                            menu.depth == at && menu.selected(at) == Some(i);
                                        let (rect, clicked) = match entry {
                                            Entry::Node(def) => {
                                                let row = browse::row(
                                                    ui,
                                                    def,
                                                    selected,
                                                    menu.follow,
                                                    theme,
                                                );
                                                (row.rect, row.clicked)
                                            }
                                            Entry::Group(_) => category_row(
                                                ui,
                                                entry,
                                                ui.available_width(),
                                                selected,
                                                menu.follow,
                                                theme,
                                            ),
                                        };
                                        if pointer.is_some_and(|p| rect.contains(p)) {
                                            over = Over::Entry(at, Some(i));
                                        }
                                        these.push(rect);
                                        if clicked {
                                            match entry {
                                                Entry::Node(def) => chosen = Some(def.slug),
                                                Entry::Group(_) => opened = Some((at, i)),
                                            }
                                        }
                                    }
                                });
                        });
                });
                rows.push(these);
            }
        });

    if !sizing {
        crossings(menu, over, now);
    }
    if let Some((at, index)) = opened {
        menu.show_submenu(at, index);
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
/// Every rule here is a DOM handler in `menu.js`, in the same order, and each holds in every
/// column:
///
/// - `categoryItemEl.mouseenter` — cancel the close, cancel any *other* pending open, and
///   start this one's 200 ms unless its submenu is already up.
/// - `categoryItemEl.mouseleave` — cancel this one's pending open, and start the 300 ms close
///   if its submenu is up.
/// - `submenuEl.mouseenter` / `mouseleave` — cancel the close; start the close.
/// - `KeyboardNavigationManager.handleMouseOver` — the pointer moving onto an entry moves the
///   selection there, but **only in the column the keyboard is in**. Crossing, not resting: a
///   still pointer never takes the selection back.
fn crossings(menu: &mut StartMenu, over: Over, now: f64) {
    let was = menu.over;
    if was == over {
        return;
    }
    menu.over = over;

    // Left an entry.
    if let Over::Entry(at, Some(index)) = was {
        if menu.pending.is_some_and(|(c, i, _)| (c, i) == (at, index)) {
            menu.pending = None;
        }
        if menu.path.get(at) == Some(&index) && menu.closing.is_none() {
            menu.closing = Some((at, now + CLOSE_DELAY));
        }
    }
    // Left a submenu for outside the menu.
    if let Over::Entry(at, _) = was
        && at > 0
        && over == Over::Nothing
        && menu.closing.is_none()
    {
        menu.closing = Some((0, now + CLOSE_DELAY));
    }

    if let Over::Entry(at, entry) = over {
        menu.closing = None;
        if let Some(index) = entry {
            if menu.pending.is_some_and(|(c, i, _)| (c, i) != (at, index)) {
                menu.pending = None;
            }
            if menu.path.get(at) != Some(&index) && menu.pending.is_none() {
                menu.pending = Some((at, index, now + OPEN_DELAY));
            }
            if menu.depth == at {
                menu.select(at, Some(index));
            }
        }
    }
}

/// One row that is not a node's shape: an icon, a name, and the ▶ that says it has a submenu
/// — a category, a family — or, for a node that stands among them, the node with no ▶.
/// Returns its rect, and whether it was clicked.
fn category_row(
    ui: &mut Ui,
    entry: &Entry,
    width: f32,
    selected: bool,
    follow: bool,
    theme: &Theme,
) -> (Rect, bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click());
    crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
    if selected && follow {
        ui.scroll_to_rect(rect, Some(Align::Center));
    }
    let (icon, label, more) = match entry {
        Entry::Node(def) => (def.icon, def.label, false),
        Entry::Group(group) => (group.icon, group.label, true),
    };
    browse::paint_row(ui, rect, selected, response.hovered(), icon, label, theme);
    if more {
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
    let name = format!("{icon} {label}");
    response.widget_info(|| {
        eframe::egui::WidgetInfo::labeled(eframe::egui::WidgetType::Button, true, &name)
    });
    (rect, response.clicked())
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
fn keys(ui: &Ui, menu: &mut StartMenu, tree: &[Entry], chosen: &mut Option<&'static str>) {
    let frame = ui.ctx().cumulative_frame_nr();
    if menu.keys_at == Some(frame) {
        return;
    }
    menu.keys_at = Some(frame);
    let len = |menu: &StartMenu| column(tree, &menu.path[..menu.depth]).map_or(0, <[Entry]>::len);
    let mut enter = false;
    let mut right = false;
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            if menu.depth > 0 {
                menu.hide_from(menu.depth - 1);
            } else {
                menu.close();
            }
        }
        for (key, by) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
            if i.consume_key(Modifiers::NONE, key) {
                menu.follow = true;
                let at = menu.depth;
                let to = step(menu.selected(at), len(menu), by);
                menu.select(at, Some(to));
            }
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowLeft) && menu.depth > 0 {
            menu.hide_from(menu.depth - 1);
        }
        enter = i.consume_key(Modifiers::NONE, Key::Enter);
        right = i.consume_key(Modifiers::NONE, Key::ArrowRight);
    });
    if !(enter || right) || !menu.open {
        return;
    }
    // Nothing selected takes nothing: a submenu opened by hovering has no entry under the
    // keyboard until `Down` puts one there.
    let at = menu.depth;
    let Some(index) = menu.selected(at) else {
        return;
    };
    match column(tree, &menu.path[..at]).and_then(|c| c.get(index)) {
        // A node among the categories has nothing to go into: `Enter` takes it, and `Right`
        // does nothing, as on a Windows menu entry with no submenu. In a submenu either takes
        // it.
        Some(Entry::Node(def)) => {
            if enter || at > 0 {
                *chosen = Some(def.slug);
            }
        }
        // Going in by key selects the first entry, where going in by pointer selects none.
        Some(Entry::Group(_)) => {
            menu.show_submenu(at, index);
            menu.select(at + 1, Some(0));
            menu.follow = true;
        }
        None => {}
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

    /// The one node among the categories on a video workspace is the Output; on a solids one,
    /// Render 3D beside it.
    #[test]
    fn the_nodes_at_the_top_are_the_output_and_on_solids_the_render() {
        for (kind, want) in [
            (WorkspaceKind::Video, vec!["output"]),
            (WorkspaceKind::Solids, vec!["solidrender", "output"]),
        ] {
            let leaves: Vec<_> = library::menu(kind)
                .iter()
                .filter_map(|e| match e {
                    Entry::Node(def) => Some(def.slug),
                    Entry::Group(_) => None,
                })
                .collect();
            assert_eq!(leaves, want, "{kind:?}");
        }
    }

    /// A path names a column whatever its depth, and a path through a node names none.
    #[test]
    fn a_path_leads_to_its_column() {
        let tree = library::menu(WorkspaceKind::Video);
        let solids = tree
            .iter()
            .position(|e| matches!(e, Entry::Group(g) if g.label == "Solids"))
            .expect("Solids");
        let shapes = column(&tree, &[solids, 1]).expect("the first family");
        assert!(
            shapes
                .iter()
                .any(|e| matches!(e, Entry::Node(d) if d.slug == "sphere"))
        );
        assert_eq!(column(&tree, &[solids, 0]), None, "Render 3D opens nothing");
    }

    /// A pointer that has not moved says nothing, so no timer starts and no selection moves.
    /// This is the whole difference between a menu the keyboard can drive and one it cannot.
    #[test]
    fn a_still_pointer_crosses_nothing() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Entry(0, Some(2)),
            cursor: vec![Some(4)],
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Entry(0, Some(2)), 10.0);
        assert_eq!(
            menu.selected(0),
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
            cursor: vec![Some(1)],
            ..StartMenu::default()
        };
        menu.show_submenu(0, 1);
        menu.select(1, Some(0));
        for t in 0..10 {
            crossings(&mut menu, Over::Nothing, f64::from(t));
        }
        assert_eq!(menu.closing, None, "the keyboard armed the close timer");
        assert_eq!(menu.path, [1], "the submenu closed itself");
    }

    /// Crossing onto another category does not close the open submenu: entering one cancels
    /// the close, and the new one waits its 200 ms before it replaces what is there. That is
    /// what stops a menu flashing on the way past.
    #[test]
    fn crossing_another_category_does_not_close_the_open_one() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Entry(0, Some(1)),
            path: vec![1],
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Entry(0, Some(2)), 1.0);
        assert_eq!(menu.closing, None, "passing over one closed the other");
        assert_eq!(menu.path, [1], "it closed on the way past");
        assert_eq!(
            menu.pending,
            Some((0, 2, 1.0 + OPEN_DELAY)),
            "the category crossed on the way did not wait its turn"
        );
    }

    /// Leaving the menu arms the close; reaching the submenu disarms it. That pair is the
    /// diagonal, and it is the whole reason the delay is 300 ms rather than nothing.
    #[test]
    fn the_diagonal_to_a_submenu_keeps_it_open() {
        let mut menu = StartMenu {
            open: true,
            over: Over::Entry(0, Some(1)),
            path: vec![1],
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Nothing, 1.0);
        assert_eq!(
            menu.closing,
            Some((0, 1.0 + CLOSE_DELAY)),
            "leaving the menu armed nothing"
        );

        crossings(&mut menu, Over::Entry(1, Some(0)), 1.1);
        assert_eq!(menu.closing, None, "reaching the submenu did not disarm it");
        assert_eq!(menu.path, [1], "it closed on the way");
    }

    /// The pointer moves the selection only in the half the keyboard is in.
    #[test]
    fn the_pointer_moves_the_selection_only_where_the_keyboard_is() {
        let mut menu = StartMenu {
            open: true,
            path: vec![0],
            depth: 1,
            cursor: vec![Some(0), Some(3)],
            ..StartMenu::default()
        };
        // In the submenu, a category the pointer crosses does not take the selection.
        crossings(&mut menu, Over::Entry(0, Some(2)), 1.0);
        assert_eq!(menu.selected(1), Some(3));
        assert_eq!(menu.selected(0), Some(0), "the pointer stole the selection");

        // Out of it, one does.
        menu.hide_from(0);
        menu.over = Over::Nothing;
        crossings(&mut menu, Over::Entry(0, Some(2)), 2.0);
        assert_eq!(menu.selected(0), Some(2));
    }

    /// A submenu's entry that opens one of its own waits its turn as a category does, and
    /// leaving the menu from the deepest column closes every submenu, not only that one.
    #[test]
    fn a_submenu_inside_a_submenu_keeps_the_same_timers() {
        let mut menu = StartMenu {
            open: true,
            path: vec![10],
            depth: 1,
            cursor: vec![Some(10), None],
            ..StartMenu::default()
        };
        crossings(&mut menu, Over::Entry(1, Some(3)), 1.0);
        assert_eq!(menu.pending, Some((1, 3, 1.0 + OPEN_DELAY)));
        assert_eq!(menu.selected(1), Some(3), "the keyboard is in that column");
        menu.show_submenu(1, 3);
        assert_eq!((menu.path.as_slice(), menu.depth), (&[10, 3][..], 2));

        crossings(&mut menu, Over::Entry(2, Some(0)), 1.5);
        crossings(&mut menu, Over::Nothing, 2.0);
        assert_eq!(menu.closing, Some((0, 2.0 + CLOSE_DELAY)));
        menu.hide_from(0);
        assert!(menu.path.is_empty());
        assert_eq!((menu.depth, menu.selected(0)), (0, Some(10)));
    }
}
