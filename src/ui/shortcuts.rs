// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help ▸ Keyboard shortcuts…: every key supersilvia answers, grouped by where it answers it,
//! drawn from one table.
//!
//! **One table, [`TABLE`], so the window cannot drift from the bindings.** A key the menu
//! prints and the app consumes is a constant in [`crate::ui::menu::keys`], and its row here
//! names that constant rather than spelling the key again; a test reads `menu::keys` and
//! fails on any constant no row names, and another fails on any key a menu entry prints
//! that no row lists. A key a widget consumes for itself — the number control's `D`, a
//! picture window's `Escape` — is written once here as the key it is.
//!
//! A fixed window like About: a frame that never changes size, whose rows scroll inside it.
//! docs/ui.md, "The keyboard shortcuts window", has the rest.

use crate::ui::keycap;
use crate::ui::menu::{keys, said};
use crate::ui::theme::Theme;
use eframe::egui::{
    self, Context, Key, KeyboardShortcut, Modifiers, RichText, ScrollArea, Ui, Window, vec2,
};

/// The keys a row stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    One(KeyboardShortcut),
    /// Any of these, said as `Ctrl+Shift+Z / Ctrl+Y`.
    Any(&'static [KeyboardShortcut]),
    /// Every key from the first to the last, said as `Ctrl+1 – Ctrl+9`.
    Run(KeyboardShortcut, KeyboardShortcut),
    /// A gesture of the pointer's, with whatever is held through it: `Alt` and `click`.
    Gesture(Modifiers, &'static str),
}

/// One row: the keys, and what they do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Binding {
    pub keys: Keys,
    pub what: &'static str,
}

/// A heading and its rows: where the keys are answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub rows: &'static [Binding],
}

const fn one(key: KeyboardShortcut, what: &'static str) -> Binding {
    Binding {
        keys: Keys::One(key),
        what,
    }
}

const fn any(keys: &'static [KeyboardShortcut], what: &'static str) -> Binding {
    Binding {
        keys: Keys::Any(keys),
        what,
    }
}

const fn gesture(held: Modifiers, gesture: &'static str, what: &'static str) -> Binding {
    Binding {
        keys: Keys::Gesture(held, gesture),
        what,
    }
}

/// A key with nothing held.
const fn bare(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::NONE, key)
}

const ESCAPE: KeyboardShortcut = bare(Key::Escape);
const BRACKETS: &[KeyboardShortcut] = &[bare(Key::OpenBracket), bare(Key::CloseBracket)];
const UP_DOWN: &[KeyboardShortcut] = &[bare(Key::ArrowUp), bare(Key::ArrowDown)];
const INTO: &[KeyboardShortcut] = &[bare(Key::ArrowRight), bare(Key::Enter)];

/// Every key there is, by where it is answered. The window draws this and nothing else.
pub const TABLE: &[Group] = &[
    Group {
        title: "Global",
        rows: &[
            one(keys::OPEN, "Open a project"),
            one(keys::SAVE, "Save the project"),
            one(keys::SAVE_AS, "Save the project in a folder of its own"),
            one(keys::UNDO, "Undo, saying what it took back"),
            any(&[keys::REDO, keys::REDO_ALT], "Redo"),
            one(keys::NEW_TAB, "A new workspace"),
            Binding {
                keys: Keys::Run(keys::TAB[0], keys::TAB[8]),
                what: "The tab in that place, the project tab first",
            },
            one(keys::NEXT_TAB, "The next tab along the bar, wrapping round"),
            one(keys::PREVIOUS_TAB, "The tab before, wrapping round"),
            one(keys::PLAY_PAUSE, "Pause or play"),
            one(keys::HIDE_EDITOR, "Hide the editor, leaving the mix"),
            one(keys::FULLSCREEN, "The editor's window fullscreen, or back"),
            any(&[keys::ZOOM_IN, keys::ZOOM_IN_ALT], "Zoom the editor in"),
            one(keys::ZOOM_OUT, "Zoom the editor out"),
            one(keys::ZOOM_ACTUAL, "The editor at its actual size"),
            any(&[keys::SHORTCUTS, keys::SHORTCUTS_ALT], "This window"),
        ],
    },
    Group {
        title: "Canvas",
        rows: &[
            gesture(Modifiers::NONE, "click", "Select a node"),
            gesture(
                Modifiers::SHIFT,
                "click",
                "Add a node to the selection, or take it out",
            ),
            gesture(Modifiers::SHIFT, "drag", "Select with a band"),
            gesture(Modifiers::NONE, "drag the background", "Pan"),
            one(keys::SELECT_ALL, "Select every node on this workspace"),
            one(
                keys::CANCEL,
                "Put back a node drag or drop a held cable; otherwise clear the selection",
            ),
            one(keys::COPY, "Copy the selection"),
            one(keys::CUT, "Cut the selection"),
            one(keys::PASTE, "Paste where the copy was in the window"),
            one(keys::DUPLICATE, "Duplicate the selection beside itself"),
            any(&[keys::DELETE, keys::DELETE_BACK], "Delete the selection"),
            one(
                keys::FRAME_ALL,
                "Fit everything on this workspace in the window",
            ),
            one(keys::FRAME_SELECTED, "Fit the selection in the window"),
            gesture(
                Modifiers::NONE,
                "right-click",
                "Add a node here, or the selection's menu",
            ),
            gesture(Modifiers::NONE, "double-click a cable", "Delete it"),
            gesture(
                Modifiers::NONE,
                "let a cable go in the open",
                "The node browser, on the kinds that take it; the chosen one lands wired",
            ),
            gesture(
                Modifiers::NONE,
                "drop a free node on a cable",
                "Splice it in",
            ),
            any(
                &[
                    bare(Key::B),
                    bare(Key::E),
                    bare(Key::L),
                    bare(Key::R),
                    bare(Key::C),
                    bare(Key::F),
                ],
                "A Drawing Canvas's tools, once a press has given it the keys",
            ),
            any(
                BRACKETS,
                "A Drawing Canvas's brush a pixel smaller or larger, five with Shift",
            ),
        ],
    },
    Group {
        title: "Number control",
        rows: &[
            gesture(Modifiers::NONE, "drag", "Scrub"),
            gesture(Modifiers::SHIFT, "drag", "Scrub a fine step"),
            gesture(Modifiers::COMMAND, "drag", "Scrub a coarse step"),
            gesture(
                Modifiers::NONE,
                "wheel",
                "One step, with the same multipliers",
            ),
            any(
                UP_DOWN,
                "One step while hovered or typing, with the same multipliers",
            ),
            gesture(
                Modifiers::NONE,
                "click the value",
                "Type one: Enter commits, Escape cancels",
            ),
            one(ESCAPE, "While scrubbing: back to where the drag began"),
            any(BRACKETS, "To this control's own low or high end"),
            one(bare(Key::D), "Back to the default"),
            one(
                bare(Key::R),
                "Back to the default and the definition's range",
            ),
            gesture(Modifiers::NONE, "right-click", "The range editor"),
            gesture(
                Modifiers::ALT,
                "click",
                "Learn a MIDI binding; Escape stops waiting",
            ),
        ],
    },
    Group {
        title: "Nodes menu and browser",
        rows: &[
            one(keys::NODES, "Open or close the Nodes menu"),
            any(
                &[keys::BROWSER, bare(Key::Backtick)],
                "The node browser at the top of the canvas",
            ),
            any(UP_DOWN, "Walk the list"),
            any(INTO, "Into a category, or add the node"),
            one(bare(Key::ArrowLeft), "Back out of a category"),
            one(ESCAPE, "Back out, or close"),
            gesture(
                Modifiers::NONE,
                "type",
                "Search the browser; Enter takes the top of the list",
            ),
        ],
    },
    Group {
        title: "Picture windows",
        rows: &[
            gesture(Modifiers::NONE, "drag the picture", "Move the window"),
            one(bare(Key::F), "Fullscreen, or back"),
            gesture(Modifiers::NONE, "double-click", "Fullscreen, or back"),
            one(
                ESCAPE,
                "Out of fullscreen, then close; a window opened fullscreen closes",
            ),
            one(bare(Key::K), "Keep the window above every other, on macOS"),
        ],
    },
];

impl Keys {
    /// The keys as the window prints them, in the machine's own spelling — `Ctrl` here,
    /// `⌘` on a Mac.
    pub fn said(&self, ctx: &Context) -> String {
        match self {
            Self::One(key) => said(ctx, key),
            Self::Any(keys) => keys
                .iter()
                .map(|k| said(ctx, k))
                .collect::<Vec<_>>()
                .join(" / "),
            Self::Run(first, last) => format!("{} – {}", said(ctx, first), said(ctx, last)),
            Self::Gesture(held, gesture) if held.is_none() => (*gesture).to_owned(),
            Self::Gesture(held, gesture) => format!("{}+{gesture}", ctx.format_modifiers(*held)),
        }
    }

    /// The keys drawn as they are pressed: a cap for every key with a legend on it, joined
    /// by a muted `+` (a Mac writes its chords with none), alternatives by a `/` and a run by a
    /// `–`; the space bar and a gesture's word stay words. `said` stays the row's name for
    /// the accessibility tree, so nothing that reads the window by its text changes.
    pub fn draw(&self, ui: &mut Ui, theme: &Theme) {
        let muted = theme.text_muted();
        let mac = ui.ctx().os().is_mac();
        let chord = |ui: &mut Ui, held: Modifiers, last: &dyn Fn(&mut Ui)| {
            for legend in keycap::modifier_legends(ui, held) {
                keycap::keycap(ui, &legend, theme);
                if !mac {
                    keycap::word(ui, "+", muted);
                }
            }
            last(ui);
        };
        let key = |ui: &mut Ui, shortcut: &KeyboardShortcut| {
            let bare_key = shortcut.logical_key;
            chord(ui, shortcut.modifiers, &|ui: &mut Ui| {
                let ctx = ui.ctx().clone();
                match keycap::legend(bare_key, || said(&ctx, &bare(bare_key))) {
                    Some(legend) => keycap::keycap(ui, &legend, theme),
                    None => keycap::word(ui, &said(&ctx, &bare(bare_key)), theme.text_primary()),
                }
            });
        };
        match self {
            Self::One(shortcut) => key(ui, shortcut),
            Self::Any(shortcuts) => {
                for (i, shortcut) in shortcuts.iter().enumerate() {
                    if i > 0 {
                        keycap::word(ui, "/", muted);
                    }
                    key(ui, shortcut);
                }
            }
            Self::Run(first, last) => {
                key(ui, first);
                keycap::word(ui, "–", muted);
                key(ui, last);
            }
            Self::Gesture(held, gesture) => chord(ui, *held, &|ui: &mut Ui| {
                keycap::word(ui, gesture, theme.text_primary());
            }),
        }
    }

    /// Every key this row names. A gesture names none.
    pub fn shortcuts(&self) -> Vec<KeyboardShortcut> {
        match self {
            Self::One(key) => vec![*key],
            Self::Any(keys) => keys.to_vec(),
            Self::Run(first, last) => vec![*first, *last],
            Self::Gesture(..) => Vec::new(),
        }
    }
}

/// What the window asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutsAction {
    /// The window's close button.
    Close,
}

/// How wide the keys' column is, so every group's descriptions start on one line.
const KEYS_WIDTH: f32 = 230.0;

/// A group's heading, a quarter above the rows' size and a whole pixel, as the canvas's
/// fonts are.
const HEADING_SIZE: f32 = 15.0;

/// The Keyboard shortcuts window.
pub fn show(
    ctx: &Context,
    theme: &Theme,
    saved: &crate::preferences::Placements,
) -> Vec<ShortcutsAction> {
    let mut actions = Vec::new();
    let mut open = true;
    let window = Window::new(super::placed::SHORTCUTS)
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .fixed_size(vec2(660.0, 540.0))
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    super::placed::place(ctx, window, super::placed::SHORTCUTS, saved).show(ctx, |ui| {
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, group) in TABLE.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(16.0);
                    }
                    // A heading a size above the rows and in the primary ink, with a rule
                    // under it, so each group starts where the eye can find it.
                    ui.label(
                        RichText::new(group.title)
                            .font(crate::ui::theme::strong_font(HEADING_SIZE))
                            .color(theme.text_primary())
                            .strong(),
                    );
                    ui.separator();
                    for row in group.rows {
                        // One column of keys at a fixed width in every group, so the
                        // descriptions start on one line down the whole window; a
                        // description wraps inside the window rather than widening it.
                        ui.horizontal_top(|ui| {
                            let said = row.keys.said(ctx);
                            let keys = ui.allocate_ui_with_layout(
                                vec2(KEYS_WIDTH, 0.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_width(KEYS_WIDTH);
                                    ui.spacing_mut().item_spacing.x = 3.0;
                                    row.keys.draw(ui, theme);
                                },
                            );
                            // The caps are paint; the row keeps its keys' text as its name.
                            keys.response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &said)
                            });
                            ui.add_space(12.0);
                            // Down by the cap's lift, so the words sit on the caps' line.
                            ui.vertical(|ui| {
                                ui.add_space(keycap::LIFT);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(row.what).color(theme.text_secondary()),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                        ui.add_space(2.0);
                    }
                }
            });
    });
    if !open {
        actions.push(ShortcutsAction::Close);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::menu::{self, Item, MenuState};

    /// Every key the table lists.
    fn listed() -> Vec<KeyboardShortcut> {
        TABLE
            .iter()
            .flat_map(|g| g.rows)
            .flat_map(|r| r.keys.shortcuts())
            .collect()
    }

    /// **Every constant in `menu::keys` is a row here.** Read off the source, since a module's
    /// constants cannot be listed at run time: a key added there with no row fails this, by
    /// name, before it can be a key the window does not mention.
    #[test]
    fn every_key_constant_has_a_row() {
        let menu = include_str!("menu.rs");
        let start = menu.find("pub mod keys {").expect("menu::keys");
        let body = &menu[start..];
        let body = &body[..body.find("\n}\n").expect("the end of menu::keys")];
        let table = include_str!("shortcuts.rs");
        let table = &table[..table.find("#[cfg(test)]").expect("the tests")];
        let names: Vec<&str> = body
            .lines()
            .filter_map(|line| line.trim().strip_prefix("pub const "))
            .filter_map(|rest| rest.split(':').next())
            .collect();
        assert!(names.len() > 10, "read the constants: {names:?}");
        let missing: Vec<&&str> = names
            .iter()
            .filter(|name| {
                let wanted = format!("keys::{name}");
                !table.match_indices(&wanted).any(|(at, _)| {
                    table[at + wanted.len()..]
                        .chars()
                        .next()
                        .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
                })
            })
            .collect();
        assert!(
            missing.is_empty(),
            "menu::keys has no row in ui::shortcuts::TABLE for {missing:?}: add one to the group \
             the key is answered in"
        );
    }

    /// Every key a menu entry prints is a key the window lists, so the two cannot say
    /// different things.
    #[test]
    fn every_key_a_menu_prints_is_listed() {
        let state = MenuState {
            workspace: Some(("Main", crate::graph::LayoutMode::Canvas)),
            ..menu::tests::quiet()
        };
        let listed = listed();
        let menus = menu::model(&state);
        for m in &menus {
            for entry in menu::tests::entries(&menus, m.title) {
                if let Some(key) = entry.shortcut {
                    assert!(
                        listed.contains(&key),
                        "{} ▸ {} prints {key:?}, which the shortcuts window does not list",
                        m.title,
                        entry.label
                    );
                }
            }
            // Every entry with a key is a plain one: a checkbox draws none.
            assert!(m.items.iter().all(|i| !matches!(
                i,
                Item::Entry(e) if e.shortcut.is_some() && e.mark != menu::Mark::None
            )));
        }
    }

    /// The five groups, in the order the window shows them, and every row says something.
    #[test]
    fn the_groups_are_where_keys_are_answered() {
        let titles: Vec<&str> = TABLE.iter().map(|g| g.title).collect();
        assert_eq!(
            titles,
            [
                "Global",
                "Canvas",
                "Number control",
                "Nodes menu and browser",
                "Picture windows"
            ]
        );
        assert!(
            TABLE
                .iter()
                .flat_map(|g| g.rows)
                .all(|r| !r.what.is_empty())
        );
    }

    /// The keys are said in the machine's spelling: a run as its two ends, a gesture with
    /// what is held through it.
    #[test]
    fn keys_are_said_as_the_machine_spells_them() {
        let ctx = Context::default();
        ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
        ctx.set_os(egui::os::OperatingSystem::Nix);
        assert_eq!(Keys::One(keys::UNDO).said(&ctx), "Ctrl+Z");
        assert_eq!(
            Keys::Any(&[keys::REDO, keys::REDO_ALT]).said(&ctx),
            "Ctrl+Shift+Z / Ctrl+Y"
        );
        assert_eq!(
            Keys::Run(keys::TAB[0], keys::TAB[8]).said(&ctx),
            "Ctrl+1 – Ctrl+9"
        );
        assert_eq!(
            Keys::Gesture(Modifiers::ALT, "click").said(&ctx),
            "Alt+click"
        );
        assert_eq!(Keys::One(keys::SHORTCUTS).said(&ctx), "F1");
        // A key with a symbol is said by it, where egui would spell it out.
        assert_eq!(
            Keys::Any(&[keys::ZOOM_IN, keys::ZOOM_OUT, keys::SHORTCUTS_ALT]).said(&ctx),
            "Ctrl++ / Ctrl+− / Ctrl+/"
        );
        assert_eq!(Keys::Any(BRACKETS).said(&ctx), "[ / ]");
        assert_eq!(Keys::One(keys::BROWSER).said(&ctx), "/");
    }

    /// On a Mac the window says the command key and Option as a Mac does: by their
    /// symbols where the app's font has them, as Cmd and Option where it does not, a
    /// gesture's included. Only a key held with Control itself, as Ctrl+Tab is on a Mac
    /// too, may say Ctrl.
    #[test]
    fn a_mac_is_told_cmd_and_option() {
        let ctx = Context::default();
        ctx.set_os(egui::os::OperatingSystem::Mac);
        // The spelling asks the app's fonts for the symbols, and they load on a first pass.
        let mut first = ctx.run_ui(egui::RawInput::default(), |ctx| {
            crate::ui::theme::apply(ctx, &Theme::default());
        });
        first.textures_delta.clear();
        let coarse = Keys::Gesture(Modifiers::COMMAND, "drag").said(&ctx);
        assert!(coarse.contains('⌘') || coarse.contains("Cmd"), "{coarse:?}");
        let learn = Keys::Gesture(Modifiers::ALT, "click").said(&ctx);
        assert!(learn.contains('⌥') || learn.contains("Option"), "{learn:?}");
        for group in TABLE {
            for row in group.rows {
                let said = row.keys.said(&ctx);
                let control = row
                    .keys
                    .shortcuts()
                    .iter()
                    .any(|k| k.modifiers.ctrl && !k.modifiers.command);
                assert!(!said.contains("Alt"), "{said:?} in {}", group.title);
                assert!(
                    control || !said.contains("Ctrl"),
                    "{said:?} in {}",
                    group.title
                );
                assert!(
                    !row.what.contains("Ctrl") && !row.what.contains("Alt"),
                    "{:?} in {}",
                    row.what,
                    group.title
                );
            }
        }
    }
}
