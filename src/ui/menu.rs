// SPDX-License-Identifier: AGPL-3.0-or-later

//! The menu bar.
//!
//! Like [`crate::ui::show`], this draws and returns what the interaction asked for; it never
//! mutates. That matters more here than it looks: the menu's entries span graph edits, file
//! operations, undo and view state, and inlining it in `App::ui` meant that one closure
//! reached into fourteen different fields. As a list of `MenuAction`s, what the menu can ask
//! for is enumerable, and `App` has one place that answers.
//!
//! The menus themselves are data, [`model`], which [`show`] draws with egui — the bar on
//! Linux, and on any build that has no native one — and which `platform::menu` hands to
//! AppKit on macOS. Why Linux's is not native is in `docs/decisions.md`.

use crate::Command;
use crate::graph::{LayoutMode, NodeId};
use crate::ui::ClipAction;
use eframe::egui::{self, KeyboardShortcut, Ui};
use std::path::{Path, PathBuf};

/// What the menu asked for. `App::handle_menu` is the only thing that answers.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// Ask for a new project's name, and make it in the projects folder.
    NewProject,
    /// Open a project folder.
    OpenProject,
    /// Open a project from the Recent list.
    OpenRecent(PathBuf),
    Save,
    SaveAs,
    /// Open the project's folder in the desktop's file manager.
    ShowProjectFolder,
    Quit,
    Undo,
    Redo,
    /// A command the Edit menu built against the selection it is already displaying —
    /// duplicate, collapse, reset, disconnect, delete. One variant rather than five, because
    /// the menu knows the exact command and `App` has nothing to add to any of them; the
    /// entries that *do* need `App` to fill something in, like `AutoArrange`'s height, stay
    /// variants of their own.
    Nodes(Command),
    /// Copy, cut or paste. Not a `Nodes(Command)`, because two of the three are not commands
    /// at all: the clipboard is session state, and only the paste reaches the bus.
    Clip(ClipAction),
    /// The time readout beside the frame-rate meter. A preference, like the cost strip.
    SetTime(bool),
    /// A hand on the transport from the time readout: pause, play or back to zero. Never an
    /// edit.
    Transport(crate::transport::Command),
    /// The problems badge beside the time readout: its list opens, or closes.
    Problems,
    /// The cost strip under every node. A preference, like the time readout.
    SetCosts(bool),
    ResetView,
    /// Canvas or Linear for the active workspace. A graph edit, so `App` sends it to the
    /// bus; which workspace is `App`'s to fill in, as the arrange's height is.
    SetLayout(LayoutMode),
    /// Rank the active workspace into columns. The height comes from the canvas, which the
    /// menu does not measure, so `App` fills it in.
    AutoArrange,
    /// Start renaming the active workspace. The tab bar holds the editor, so the menu asks
    /// for it rather than putting up a dialog of its own.
    RenameWorkspace,
    /// Close the active workspace: it keeps its nodes and loses its tab.
    CloseWorkspace,
    /// Write the active workspace out on its own. Where it goes is a folder dialog's
    /// answer, and `App` opens that, because `ui/` never blocks a frame.
    ExportWorkspace,
    /// Open the Preferences window. App state: a window is not an edit, and the window
    /// closes by its own ✕ rather than by a second trip through the menu.
    OpenPreferences,
    /// The MIDI window: devices, the map, and a monitor. Its own window rather than a page
    /// of Preferences — a binding is project data and a preference is about the person.
    OpenMidi,
    /// A new video workspace, open and showing: Workspace ▸ New, `Ctrl+T` and the tab bar's
    /// `+`.
    NewWorkspace,
    /// The tab at this position in `TabBar::order`, the project tab first. `Ctrl+1` to
    /// `Ctrl+9`, which no entry carries: the tabs are their entries.
    GoToTab(usize),
    /// Help ▸ About supersilvia: the name, the version, the licence and the source.
    OpenAbout,
    /// Help ▸ Licences…: supersilvia's licence and every third-party notice.
    OpenLicences,
    /// Help ▸ Report a problem…: the form a bug report is written and copied from.
    ReportProblem,
    /// Help ▸ Keyboard shortcuts…: every key there is, from [`crate::ui::shortcuts::TABLE`].
    OpenShortcuts,
    /// Edit ▸ Undo History…: the ring's steps by name.
    OpenUndoHistory,
    /// View ▸ Pause or Play, and `F8`. Never an edit, and refused while a render owns the
    /// playhead.
    PlayPause,
    /// View ▸ Hide editor, and `H`: the canvas gives way to the mix.
    HideEditor,
    /// View ▸ Fullscreen, and `F`: the editor's window.
    Fullscreen,
    /// View ▸ Zoom in, Zoom out and Actual size: egui's zoom over the whole editor, which
    /// `ui_zoom` keeps.
    Zoom(Zoom),
}

/// A step of the editor's zoom, egui's `gui_zoom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zoom {
    In,
    Out,
    /// Back to 1, the display's own scale.
    Actual,
}

/// The read-only slice of app state the menu needs to draw itself: what to enable, what to
/// tick, and what Delete would act on.
// A flat read-only slice of app state, not a configuration object: each bool is one
// independent question the menu asks, and grouping them would only add indirection.
#[allow(clippy::struct_excessive_bools)]
pub struct MenuState<'a> {
    /// What an undo would take back, by name, or `None` with nothing to undo.
    pub undo: Option<String>,
    /// What a redo would put back, by name, or `None` with nothing to redo.
    pub redo: Option<String>,
    /// A file dialog is already open. Opening a second behind it helps nobody.
    pub file_busy: bool,
    /// The show is playing rather than paused, which is what View's Pause or Play says.
    pub playing: bool,
    /// A render owns the playhead, so a hand cannot pause it.
    pub rendering: bool,
    /// `H` is in force: the canvas has given way to the mix.
    pub editor_hidden: bool,
    /// The editor's window is fullscreen.
    pub fullscreen: bool,
    /// egui's zoom factor, which Zoom in and out stop at the ends of.
    pub zoom: f32,
    /// The time readout's reading, or `None` while View ▸ Time is off.
    pub time: Option<Time>,
    pub show_costs: bool,
    /// The frame-rate meter's two figures, or `None` while the preference is off.
    pub fps: Option<Fps>,
    /// What the problems badge counts. Drawn whatever the preferences say.
    pub problems: crate::ui::problems::Count,
    /// The selection on the active workspace, which is what every verb in the Edit menu
    /// acts on. Empty disables all of them: a verb with no object is disabled, not one that
    /// guesses an object.
    pub selection: Vec<NodeId>,
    /// Whether the clipboard holds anything. Paste's enable, and the one thing in the Edit
    /// menu that does not read the selection: a paste wants a workspace, not an object.
    pub clipboard: bool,
    /// Whether any of `selection` is expanded, which is the only thing those verbs need to
    /// know beyond the ids — it decides whether the entry reads Collapse or Expand.
    pub any_expanded: bool,
    /// The active workspace's name and layout mode, or `None` while the project tab is
    /// showing. The Workspace menu exists only when this does: on the project tab there is
    /// nothing for it to act on.
    pub workspace: Option<(&'a str, LayoutMode)>,
    /// Project folders opened or saved, most recent first. A borrow rather than a clone:
    /// it is read to draw a submenu and nothing here owns it.
    pub recent: &'a [PathBuf],
}

/// What the menu bar's frame-rate meter reads, in frames a second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fps {
    /// The synth's ticks, which is the picture's rate and the figure shown.
    pub synth: f32,
    /// The editor's redraws, which is only how often the canvas is painted: the hover text.
    pub editor: f32,
    /// The synth is five per cent short of the rate it is asked for, by the Status box's
    /// own rule: the figure is in the accent.
    pub short: bool,
}

/// What the time readout draws: the transport as the last tick left it, and whether a hand
/// may move it — not while a render owns the playhead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Time {
    pub report: crate::transport::Report,
    pub enabled: bool,
}

/// Keyboard shortcuts, shown in the menu and consumed by `App`.
///
/// **Consume the most specific first.** egui's `Modifiers::matches_logically` rejects a
/// shortcut only when the *pattern* needs a modifier that is not held; extra modifiers that
/// are held do not disqualify it. So `Ctrl+Z` matches a `Ctrl+Shift+Z` press, and `Ctrl+S` a
/// `Ctrl+Shift+S` one.
///
/// **Every constant here is a row of [`crate::ui::shortcuts::TABLE`]**, the Keyboard shortcuts
/// window, and a test there fails naming any that is not.
pub mod keys {
    use eframe::egui::{Key, KeyboardShortcut, Modifiers, gui_zoom::kb_shortcuts};

    /// `F8`: pause or play, the key a Mac's keyboard prints ⏯ on. Not `Space`, which a hand
    /// brushes and which would stop the whole show. A bare key, read under the guard `H` and
    /// `F` share.
    pub const PLAY_PAUSE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F8);
    /// `H`: hide the editor, leaving the mix.
    pub const HIDE_EDITOR: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::H);
    /// `F`: the editor's window fullscreen.
    pub const FULLSCREEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F);
    /// `F1`: the Keyboard shortcuts window.
    pub const SHORTCUTS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F1);
    /// `Ctrl+/`, the same window: `?` is the node browser's.
    pub const SHORTCUTS_ALT: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Slash);
    /// egui's own zoom keys, consumed by egui at the end of every frame. Here for the menu
    /// to print and the window to list.
    pub const ZOOM_IN: KeyboardShortcut = kb_shortcuts::ZOOM_IN;
    pub const ZOOM_IN_ALT: KeyboardShortcut = kb_shortcuts::ZOOM_IN_SECONDARY;
    pub const ZOOM_OUT: KeyboardShortcut = kb_shortcuts::ZOOM_OUT;
    pub const ZOOM_ACTUAL: KeyboardShortcut = kb_shortcuts::ZOOM_RESET;
    /// `N`: the Nodes menu, read by `ui::start`.
    pub const NODES: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::N);
    /// `/`: the node browser at the top of the canvas. `` ` `` and `?` open it too.
    pub const BROWSER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Slash);

    pub const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    pub const REDO: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    /// The other convention, for people who learned it in a different decade.
    pub const REDO_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
    pub const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
    pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    pub const SAVE_AS: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
    /// The clipboard three. Consumed in `App` under a guard the other shortcuts do not need
    /// — see `app::frame`.
    pub const COPY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::C);
    pub const CUT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::X);
    pub const PASTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::V);
    /// A new video workspace, open and showing.
    pub const NEW_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::T);

    /// `Ctrl+1` to `Ctrl+9`: the tab in that position, the project tab first.
    pub const TAB: [KeyboardShortcut; 9] = [
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num1),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num2),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num3),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num4),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num5),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num6),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num7),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num8),
        KeyboardShortcut::new(Modifiers::COMMAND, Key::Num9),
    ];
    /// `Ctrl+Tab`: the next tab along the bar, read by `App::switch_tabs_by_key`. `Ctrl` on a
    /// Mac too, as every browser and editor binds it.
    pub const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);
    /// `Ctrl+Shift+Tab`: the one before.
    pub const PREVIOUS_TAB: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab);

    // The canvas's own keys, read by `ui::show` after every control on it has had its turn,
    // and never while a field, a popup, the Nodes menu or a hovered number control has the
    // keyboard.

    /// Delete the selection on this workspace, one undo step.
    pub const DELETE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Delete);
    /// Delete the selection, as `DELETE` does: the key a laptop has.
    pub const DELETE_BACK: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::NONE, Key::Backspace);
    /// Duplicate the selection beside itself; the copies become the selection.
    pub const DUPLICATE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::D);
    /// Select every node on this workspace.
    pub const SELECT_ALL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::A);
    /// Put back a node drag, or drop a held cable; with nothing in hand, clear the selection.
    pub const CANCEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Escape);
    /// Glide the view to fit everything on this workspace.
    pub const FRAME_ALL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::Home);
    /// Glide the view to fit the selection.
    pub const FRAME_SELECTED: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::NONE, Key::Period);
}

/// The keyboard's shortcuts this frame, as the menu actions they stand for, consumed so no
/// widget below also reads one as a keystroke.
///
/// **Redo before undo and Save as before Save**, and the order is load-bearing: see [`keys`].
/// Consuming undo first made redo step backwards. The clipboard three are not here: `App`
/// takes those as events under a guard of their own, see `app::frame`.
pub fn shortcuts(input: &mut egui::InputState) -> Vec<MenuAction> {
    let mut actions = Vec::new();
    if input.consume_shortcut(&keys::REDO) || input.consume_shortcut(&keys::REDO_ALT) {
        actions.push(MenuAction::Redo);
    }
    if input.consume_shortcut(&keys::UNDO) {
        actions.push(MenuAction::Undo);
    }
    if input.consume_shortcut(&keys::OPEN) {
        actions.push(MenuAction::OpenProject);
    }
    let save_as = input.consume_shortcut(&keys::SAVE_AS);
    if input.consume_shortcut(&keys::SAVE) && !save_as {
        actions.push(MenuAction::Save);
    }
    if save_as {
        actions.push(MenuAction::SaveAs);
    }
    if input.consume_shortcut(&keys::NEW_TAB) {
        actions.push(MenuAction::NewWorkspace);
    }
    if let Some(index) = keys::TAB.iter().position(|s| input.consume_shortcut(s)) {
        actions.push(MenuAction::GoToTab(index));
    }
    if input.consume_shortcut(&keys::SHORTCUTS) || input.consume_shortcut(&keys::SHORTCUTS_ALT) {
        actions.push(MenuAction::OpenShortcuts);
    }
    actions
}

/// A key as a person reads it: `Ctrl++`, `Ctrl+−`, `/`, `[`, `↑` — the key's own symbol where
/// it has one. egui spells a key out by name on Linux, which prints `Ctrl+Plus` and `Slash`;
/// its modifiers are kept as egui says them, `Ctrl` here and `⌘` on a Mac.
pub fn said(ctx: &egui::Context, shortcut: &KeyboardShortcut) -> String {
    use egui::Key;
    let symbol = match shortcut.logical_key {
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        key => key.symbol_or_name(),
    };
    let full = ctx.format_shortcut(shortcut);
    match full.strip_suffix(shortcut.logical_key.name()) {
        Some(held) => format!("{held}{symbol}"),
        None => full,
    }
}

/// The editor's bare keys this frame — `F8`, `H` and `F` — as the View entries they stand
/// for, consumed.
///
/// Apart from [`shortcuts`] because two of them are letters: `App` reads them only while no field
/// has the keyboard and the editor's own window has the focus, since `H` in a search box is a
/// letter and `F` in a picture window is that picture's fullscreen. A bare pattern rejects a
/// press with `Ctrl` held, so none of them is also some shortcut's key.
pub fn bare_keys(input: &mut egui::InputState) -> Vec<MenuAction> {
    [
        (keys::PLAY_PAUSE, MenuAction::PlayPause),
        (keys::HIDE_EDITOR, MenuAction::HideEditor),
        (keys::FULLSCREEN, MenuAction::Fullscreen),
    ]
    .into_iter()
    .filter_map(|(key, action)| input.consume_shortcut(&key).then_some(action))
    .collect()
}

/// One menu on the bar: its title, and what is in it.
///
/// **The menus are data, and each bar draws them.** [`show`] draws them with egui, which is
/// the bar on Linux and any build without a native one; on macOS `platform::menu` hands the
/// same list to AppKit. So an entry, its enable, its tick, its shortcut and the action it
/// sends are decided once, in [`model`], and neither bar can offer what the other does not.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    pub title: &'static str,
    /// Hover text on the title itself. The Workspace menu's is the workspace's name.
    pub hint: Option<String>,
    pub items: Vec<Item>,
}

/// One row of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Entry(Entry),
    Submenu {
        title: &'static str,
        items: Vec<Item>,
    },
    Separator,
}

/// An entry: a row that sends one action when it is chosen.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub label: String,
    pub enabled: bool,
    pub shortcut: Option<KeyboardShortcut>,
    pub mark: Mark,
    pub hint: Option<String>,
    /// Why it is greyed out, said on its hover in place of the hint while it is. A disabled
    /// entry with no reason says nothing new by hovering.
    pub why: Option<&'static str>,
    /// What choosing it asks for. `None` only for a row that is there to say something, like
    /// an empty Recent list's "Nothing yet", and is never enabled.
    pub action: Option<MenuAction>,
}

/// What an entry wears beside its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    None,
    /// A preference that is on or off. The entry's action is the other state.
    Check(bool),
    /// One of a set, of which exactly one is ticked.
    Radio(bool),
}

impl Entry {
    fn new(label: impl Into<String>, action: MenuAction) -> Self {
        Self {
            label: label.into(),
            enabled: true,
            shortcut: None,
            mark: Mark::None,
            hint: None,
            why: None,
            action: Some(action),
        }
    }

    /// A row that only says something, and so is never enabled and sends nothing.
    fn note(label: &str) -> Self {
        Self {
            enabled: false,
            action: None,
            ..Self::new(label, MenuAction::Quit)
        }
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Enabled where `enabled` is, and greyed out otherwise with `why` on its hover.
    fn unless(mut self, enabled: bool, why: &'static str) -> Self {
        self.enabled = enabled;
        self.why = Some(why);
        self
    }

    /// What its hover says: the reason while it is greyed out, where it has one, and its
    /// hint otherwise.
    pub fn hover(&self) -> Option<&str> {
        if self.enabled {
            self.hint.as_deref()
        } else {
            self.why.or(self.hint.as_deref())
        }
    }

    fn shortcut(mut self, shortcut: KeyboardShortcut) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    fn mark(mut self, mark: Mark) -> Self {
        self.mark = mark;
        self
    }

    fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl From<Entry> for Item {
    fn from(entry: Entry) -> Self {
        Self::Entry(entry)
    }
}

/// The Layout submenu: which of the two modes this workspace's canvas is.
///
/// Under Workspace rather than View because a layout mode is that workspace's data, saved in
/// its file — View was only ever holding it because there was one workspace.
fn layout_menu(layout: LayoutMode) -> Item {
    Item::Submenu {
        title: "Layout",
        items: [
            (
                LayoutMode::Canvas,
                "Canvas",
                "An unbounded plane. The wheel zooms.",
            ),
            (
                LayoutMode::Linear,
                "Linear",
                "A strip at one scale. The wheel scrolls along the graph.",
            ),
        ]
        .into_iter()
        .map(|(mode, label, hint)| {
            Entry::new(label, MenuAction::SetLayout(mode))
                .mark(Mark::Radio(layout == mode))
                .hint(hint)
                .into()
        })
        .collect(),
    }
}

/// One project in the Recent list: its folder name, with the whole path as hover text.
///
/// A folder that is no longer there is shown disabled rather than dropped. The list is what
/// was opened, and a missing entry says so more usefully than an absence does.
fn recent_entry(path: &Path) -> Item {
    let label = path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let whole = path.display().to_string();
    let exists = path.is_dir();
    Entry::new(label, MenuAction::OpenRecent(path.to_path_buf()))
        .enabled(exists)
        .hint(if exists {
            whole
        } else {
            format!("{whole} — not there any more")
        })
        .into()
}

/// Why a greyed-out entry is greyed out, in the words its hover says it in.
pub mod why {
    pub const FILE_DIALOG: &str = "A file dialog is open.";
    pub const NO_SELECTION: &str = "Select a node on this workspace first.";
    pub const EMPTY_CLIPBOARD: &str = "Copy or cut a node first.";
    pub const NOTHING_TO_UNDO: &str = "Nothing to undo.";
    pub const NOTHING_TO_REDO: &str = "Nothing to redo.";
    pub const RENDERING: &str = "A render owns the playhead until it is done.";
    pub const LARGEST: &str = "The editor is as large as it goes.";
    pub const SMALLEST: &str = "The editor is as small as it goes.";
    pub const ACTUAL: &str = "The editor is at its actual size.";
    /// The node menu's Workspaces ▸ box that would leave a node on none.
    pub const LAST_WORKSPACE: &str = "A node has to stay on at least one workspace.";
}

/// The menus, as data. Every bar draws exactly this.
pub fn model(state: &MenuState<'_>) -> Vec<Menu> {
    let free = !state.file_busy;
    let mut menus = Vec::new();

    let recent = if state.recent.is_empty() {
        vec![Entry::note("Nothing yet").into()]
    } else {
        state.recent.iter().map(|p| recent_entry(p)).collect()
    };
    menus.push(Menu {
        title: "Project",
        hint: None,
        items: vec![
            Entry::new("New project…", MenuAction::NewProject)
                .unless(free, why::FILE_DIALOG)
                .into(),
            Entry::new("Open project…", MenuAction::OpenProject)
                .unless(free, why::FILE_DIALOG)
                .shortcut(keys::OPEN)
                .into(),
            Item::Submenu {
                title: "Recent",
                items: recent,
            },
            Entry::new("Save", MenuAction::Save)
                .unless(free, why::FILE_DIALOG)
                .shortcut(keys::SAVE)
                .into(),
            Entry::new("Save as…", MenuAction::SaveAs)
                .unless(free, why::FILE_DIALOG)
                .shortcut(keys::SAVE_AS)
                .into(),
            Entry::new("Show project folder", MenuAction::ShowProjectFolder)
                .hint(format!(
                    "This project's folder, in {}",
                    crate::platform::files::MANAGER
                ))
                .into(),
            Item::Separator,
            // A binding names a control on a node and is saved in the project manifest, so
            // it belongs to the project the way its nodes do rather than to the editor.
            // Below the file entries, because it is the project's *contents* and they are
            // what is done to the file.
            Entry::new("MIDI…", MenuAction::OpenMidi).into(),
            Item::Separator,
            Entry::new("Quit", MenuAction::Quit).into(),
        ],
    });

    if let Some((name, layout)) = state.workspace {
        menus.push(Menu {
            title: "Workspace",
            hint: Some(name.to_owned()),
            items: vec![
                // The tab bar's `+`, with the key beside it. `Ctrl+1` to `Ctrl+9` have no
                // entry: the tabs are their entries, and the shortcuts window lists them.
                Entry::new("New", MenuAction::NewWorkspace)
                    .shortcut(keys::NEW_TAB)
                    .hint("A new video workspace, holding a Main Input and an Output")
                    .into(),
                Item::Separator,
                Entry::new("Rename…", MenuAction::RenameWorkspace).into(),
                Entry::new("Close", MenuAction::CloseWorkspace).into(),
                // Export leaves from the thing, and the thing is the active workspace.
                Entry::new("Export…", MenuAction::ExportWorkspace)
                    .unless(free, why::FILE_DIALOG)
                    .into(),
                Item::Separator,
                layout_menu(layout),
                Entry::new("Auto-arrange", MenuAction::AutoArrange).into(),
            ],
        });
    }

    // The node context menu's verbs, here as well, for the hand that looks in a menu bar
    // before it thinks to right-click. Same commands, same undo steps; this is a second
    // door, not a second implementation.
    let nodes = &state.selection;
    let any = !nodes.is_empty();
    // Undo and Redo say what they would do, so the step is never a surprise.
    let named = |verb: &str, name: &Option<String>| match name {
        Some(name) => format!("{verb} {name}"),
        None => verb.to_owned(),
    };
    let mut edit: Vec<Item> = vec![
        Entry::new(named("Undo", &state.undo), MenuAction::Undo)
            .unless(state.undo.is_some(), why::NOTHING_TO_UNDO)
            .shortcut(keys::UNDO)
            .into(),
        Entry::new(named("Redo", &state.redo), MenuAction::Redo)
            .unless(state.redo.is_some(), why::NOTHING_TO_REDO)
            .shortcut(keys::REDO)
            .into(),
        Entry::new("Undo History…", MenuAction::OpenUndoHistory)
            .hint("Every step undo can go back to, by name. A click goes to that step.")
            .into(),
        Item::Separator,
        // Copy and Cut act on the selection, so they are disabled without one. Paste does
        // not: it needs something on the clipboard and a workspace to put it on, and a paste
        // from here lands the clip back where it was in the window.
        Entry::new("Copy", MenuAction::Clip(ClipAction::Copy(nodes.clone())))
            .unless(any, why::NO_SELECTION)
            .shortcut(keys::COPY)
            .into(),
        Entry::new("Cut", MenuAction::Clip(ClipAction::Cut(nodes.clone())))
            .unless(any, why::NO_SELECTION)
            .shortcut(keys::CUT)
            .into(),
        Entry::new("Paste", MenuAction::Clip(ClipAction::Paste { at: None }))
            .unless(state.clipboard, why::EMPTY_CLIPBOARD)
            .shortcut(keys::PASTE)
            .into(),
        Item::Separator,
    ];
    edit.extend(
        crate::ui::node_verbs(nodes, state.any_expanded)
            .into_iter()
            .map(|(label, command)| {
                // The canvas's `Ctrl+D` sends this same command, so the entry prints it — and
                // on a Mac, where the bar takes `⌘D` before the window sees it, is that key.
                let key = matches!(command, Command::Duplicate { .. }).then_some(keys::DUPLICATE);
                let entry =
                    Entry::new(label, MenuAction::Nodes(command)).unless(any, why::NO_SELECTION);
                match key {
                    Some(key) => entry.shortcut(key),
                    None => entry,
                }
                .into()
            }),
    );
    let delete = if nodes.len() > 1 {
        "Delete nodes"
    } else {
        "Delete node"
    };
    edit.extend([
        Item::Separator,
        Entry::new(
            delete,
            MenuAction::Nodes(Command::RemoveNodes(nodes.clone())),
        )
        .unless(any, why::NO_SELECTION)
        .shortcut(keys::DELETE)
        .into(),
        Item::Separator,
        // Last, under a rule, where every desktop editor keeps it. The one dialog left in
        // Edit, and the only app-wide thing here: a preference outlives every project, where
        // a MIDI binding is saved with one.
        Entry::new("Preferences…", MenuAction::OpenPreferences).into(),
    ]);
    menus.push(Menu {
        title: "Edit",
        hint: None,
        items: edit,
    });

    let (least, most) = (
        *crate::preferences::UI_ZOOM.start(),
        *crate::preferences::UI_ZOOM.end(),
    );
    menus.push(Menu {
        title: "View",
        hint: None,
        items: vec![
            Entry::new("Time", MenuAction::SetTime(state.time.is_none()))
                .mark(Mark::Check(state.time.is_some()))
                .hint("The playhead beside the frame rate, with pause and back to zero.")
                .into(),
            Entry::new("Costs", MenuAction::SetCosts(!state.show_costs))
                .mark(Mark::Check(state.show_costs))
                .hint(
                    "A strip under every node: how many times it runs per pixel, and on an \
                     Output the GPU time its frame takes.",
                )
                .into(),
            Item::Separator,
            // What the press would do, as the time readout's own button says it.
            Entry::new(
                if state.playing { "Pause" } else { "Play" },
                MenuAction::PlayPause,
            )
            .unless(!state.rendering, why::RENDERING)
            .shortcut(keys::PLAY_PAUSE)
            .into(),
            Item::Separator,
            Entry::new(
                if state.editor_hidden {
                    "Show editor"
                } else {
                    "Hide editor"
                },
                MenuAction::HideEditor,
            )
            .shortcut(keys::HIDE_EDITOR)
            .hint("The canvas gives way to the mix; the side panels stay.")
            .into(),
            Entry::new(
                if state.fullscreen {
                    "Leave fullscreen"
                } else {
                    "Fullscreen"
                },
                MenuAction::Fullscreen,
            )
            .shortcut(keys::FULLSCREEN)
            .into(),
            Item::Separator,
            // egui's zoom over the whole editor, kept in `ui_zoom`; not the canvas's own.
            Entry::new("Zoom in", MenuAction::Zoom(Zoom::In))
                .unless(state.zoom < most, why::LARGEST)
                .shortcut(keys::ZOOM_IN)
                .into(),
            Entry::new("Zoom out", MenuAction::Zoom(Zoom::Out))
                .unless(state.zoom > least, why::SMALLEST)
                .shortcut(keys::ZOOM_OUT)
                .into(),
            Entry::new("Actual size", MenuAction::Zoom(Zoom::Actual))
                .unless((state.zoom - 1.0).abs() > f32::EPSILON, why::ACTUAL)
                .shortcut(keys::ZOOM_ACTUAL)
                .into(),
            Item::Separator,
            Entry::new("Reset view", MenuAction::ResetView)
                .hint("The canvas back to where it starts: no pan, no zoom.")
                .into(),
        ],
    });

    // Last on the bar, where every desktop keeps it. A Mac answers About and Licences with the
    // About panel its application menu already has, and its Help menu holds the rest.
    menus.push(Menu {
        title: "Help",
        hint: None,
        items: vec![
            Entry::new("Keyboard shortcuts…", MenuAction::OpenShortcuts)
                .shortcut(keys::SHORTCUTS)
                .hint("Every key supersilvia answers, and where")
                .into(),
            Item::Separator,
            Entry::new("About supersilvia", MenuAction::OpenAbout).into(),
            Entry::new("Licences…", MenuAction::OpenLicences)
                .hint("supersilvia's licence, and every crate, font and library in it")
                .into(),
            Item::Separator,
            Entry::new("Report a problem…", MenuAction::ReportProblem)
                .hint(
                    "A short form, and one block to paste into the Discord's bug channel or a \
                     GitHub issue",
                )
                .into(),
        ],
    });

    menus
}

/// Draw one menu's rows with egui, pushing whatever was chosen.
fn draw_items(ui: &mut Ui, items: &[Item], actions: &mut Vec<MenuAction>) {
    for item in items {
        match item {
            Item::Separator => {
                ui.separator();
            }
            Item::Submenu { title, items } => {
                ui.menu_button(*title, |ui| draw_items(ui, items, actions));
            }
            Item::Entry(entry) => {
                if draw_entry(ui, entry) {
                    actions.extend(entry.action.clone());
                }
            }
        }
    }
}

/// One entry with egui. Returns true when it was chosen. A checkbox leaves its menu open, so
/// a second preference can be flipped beside the first; everything else closes it.
fn draw_entry(ui: &mut Ui, entry: &Entry) -> bool {
    let (response, close) = match entry.mark {
        Mark::Check(on) => {
            let mut on = on;
            let response = ui.add_enabled(
                entry.enabled,
                egui::Checkbox::new(&mut on, entry.label.as_str()),
            );
            (response, false)
        }
        Mark::Radio(on) => (
            ui.add_enabled(
                entry.enabled,
                egui::RadioButton::new(on, entry.label.as_str()),
            ),
            true,
        ),
        Mark::None => {
            let mut button = egui::Button::new(entry.label.as_str());
            if let Some(s) = &entry.shortcut {
                button = button.shortcut_text(said(ui.ctx(), s));
            }
            (ui.add_enabled(entry.enabled, button), true)
        }
    };
    let response = match entry.hover() {
        Some(said) if entry.enabled => response.on_hover_text(said),
        Some(said) => response.on_disabled_hover_text(said),
        None => response,
    };
    let chosen = response.clicked();
    if chosen && close {
        ui.close();
    }
    chosen
}

/// The frame-rate meter. Monospace and padded to three digits, so the figure changing does
/// not move it. On the egui bar it is at the far right; where the bar is the operating
/// system's, `App` puts it at the end of the tab row instead. In the accent while the synth
/// is short of its rate.
pub fn meter(ui: &mut Ui, fps: Fps, theme: &crate::ui::theme::Theme) {
    let mut figure = egui::RichText::new(format!("{:>3.0} fps", fps.synth)).monospace();
    if fps.short {
        figure = figure.color(theme.accent());
    }
    ui.label(figure).on_hover_text(format!(
        "The synth is making {:.0} frames a second{}. The editor is redrawing {:.0} times a \
         second, which is only how often this window is painted.",
        fps.synth,
        if fps.short {
            ", short of the rate it is asked for"
        } else {
            ""
        },
        fps.editor,
    ));
}

/// Draw the menu bar with egui. Returns every action the user asked for this frame.
pub fn show(
    ui: &mut Ui,
    state: &MenuState<'_>,
    theme: &crate::ui::theme::Theme,
) -> Vec<MenuAction> {
    let mut actions = Vec::new();
    let menus = model(state);

    egui::MenuBar::new().ui(ui, |ui| {
        for menu in &menus {
            let response = ui
                .menu_button(menu.title, |ui| draw_items(ui, &menu.items, &mut actions))
                .response;
            if let Some(hint) = &menu.hint {
                response.on_hover_text(hint);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(fps) = state.fps {
                meter(ui, fps, theme);
                ui.add_space(8.0);
            }
            // Immediately left of the meter, or at the far right without it.
            if let Some(time) = state.time
                && let Some(asked) = crate::ui::timecode::show(ui, time.report, time.enabled, theme)
            {
                actions.push(MenuAction::Transport(asked));
            }
            // Left of both, whatever is showing: a slot of fixed width, empty while there is
            // nothing to count, so nothing beside it moves when a count arrives.
            ui.add_space(8.0);
            if crate::ui::problems::badge(ui, state.problems, theme) {
                actions.push(MenuAction::Problems);
            }
        });
    });

    actions
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use eframe::egui::{Event, InputState, Key, Modifiers};

    /// What a frame holding these key presses asks for.
    fn asked(presses: &[(Modifiers, Key)]) -> Vec<MenuAction> {
        let mut input = InputState::default();
        for (modifiers, key) in presses {
            input.events.push(Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: *modifiers,
            });
        }
        shortcuts(&mut input)
    }

    /// The plainest state a menu is drawn from: nothing to undo, nothing selected, no
    /// workspace showing.
    pub(crate) fn quiet() -> MenuState<'static> {
        MenuState {
            undo: None,
            redo: None,
            file_busy: false,
            playing: true,
            rendering: false,
            editor_hidden: false,
            fullscreen: false,
            zoom: 1.0,
            time: None,
            show_costs: false,
            fps: None,
            problems: crate::ui::problems::Count::default(),
            selection: Vec::new(),
            clipboard: false,
            any_expanded: false,
            workspace: None,
            recent: &[],
        }
    }

    /// Every entry of one menu, its submenus' included.
    pub(crate) fn entries(menus: &[Menu], title: &str) -> Vec<Entry> {
        fn walk(items: &[Item], out: &mut Vec<Entry>) {
            for item in items {
                match item {
                    Item::Entry(e) => out.push(e.clone()),
                    Item::Submenu { items, .. } => walk(items, out),
                    Item::Separator => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(
            &menus
                .iter()
                .find(|m| m.title == title)
                .expect("the menu")
                .items,
            &mut out,
        );
        out
    }

    /// The Project menu shows the project's folder, and the View menu no longer holds the
    /// Status box, which is Preferences'.
    #[test]
    fn the_project_menu_shows_the_folder_and_view_has_no_status_box() {
        let menus = model(&quiet());
        let entries = |title: &str| entries(&menus, title);
        assert!(
            entries("Project")
                .iter()
                .any(|e| e.action == Some(MenuAction::ShowProjectFolder)
                    && e.label == "Show project folder")
        );
        assert!(entries("View").iter().all(|e| !e.label.contains("Status")));
    }

    /// Help ends on Report a problem…, under a rule of its own.
    #[test]
    fn help_ends_on_report_a_problem() {
        let state = quiet();
        let menus = model(&state);
        let help = menus.iter().find(|m| m.title == "Help").expect("Help");
        let [.., Item::Separator, Item::Entry(last)] = help.items.as_slice() else {
            panic!("Help ends on an entry under a rule: {:?}", help.items);
        };
        assert_eq!(last.label, "Report a problem…");
        assert_eq!(last.action, Some(MenuAction::ReportProblem));
        assert!(last.enabled);
    }

    /// Every shortcut is the entry it stands for, and the one with Shift held is never also
    /// the one without.
    #[test]
    fn every_shortcut_is_its_menu_action() {
        let ctrl = Modifiers::COMMAND;
        let shift = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        for (press, want) in [
            ((ctrl, Key::S), MenuAction::Save),
            ((shift, Key::S), MenuAction::SaveAs),
            ((ctrl, Key::O), MenuAction::OpenProject),
            ((ctrl, Key::Z), MenuAction::Undo),
            ((shift, Key::Z), MenuAction::Redo),
            ((ctrl, Key::Y), MenuAction::Redo),
            ((ctrl, Key::T), MenuAction::NewWorkspace),
            ((ctrl, Key::Num1), MenuAction::GoToTab(0)),
            ((ctrl, Key::Num9), MenuAction::GoToTab(8)),
        ] {
            assert_eq!(asked(&[press]), vec![want], "{press:?}");
        }
        assert!(
            asked(&[(Modifiers::NONE, Key::S)]).is_empty(),
            "a bare S is a letter"
        );
    }

    /// `F1` and `Ctrl+/` open the shortcuts window, wherever the keyboard is; a bare `/` is
    /// the browser's and is left for it.
    #[test]
    fn f1_and_ctrl_slash_open_the_shortcuts_window() {
        assert_eq!(
            asked(&[(Modifiers::NONE, Key::F1)]),
            vec![MenuAction::OpenShortcuts]
        );
        assert_eq!(
            asked(&[(Modifiers::COMMAND, Key::Slash)]),
            vec![MenuAction::OpenShortcuts]
        );
        assert!(asked(&[(Modifiers::NONE, Key::Slash)]).is_empty());
    }

    /// `F8`, `H` and `F` are the View entries they are printed beside, and with `Ctrl`
    /// held none of them is.
    #[test]
    fn the_bare_keys_are_the_view_entries() {
        for (key, want) in [
            (Key::F8, MenuAction::PlayPause),
            (Key::H, MenuAction::HideEditor),
            (Key::F, MenuAction::Fullscreen),
        ] {
            for (modifiers, expect) in [
                (Modifiers::NONE, vec![want.clone()]),
                (Modifiers::COMMAND, vec![]),
            ] {
                let mut input = InputState::default();
                input.events.push(Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                });
                assert_eq!(bare_keys(&mut input), expect, "{key:?} {modifiers:?}");
            }
        }
        let view = entries(&model(&quiet()), "View");
        for (action, key) in [
            (MenuAction::PlayPause, keys::PLAY_PAUSE),
            (MenuAction::HideEditor, keys::HIDE_EDITOR),
            (MenuAction::Fullscreen, keys::FULLSCREEN),
            (MenuAction::Zoom(Zoom::In), keys::ZOOM_IN),
            (MenuAction::Zoom(Zoom::Out), keys::ZOOM_OUT),
            (MenuAction::Zoom(Zoom::Actual), keys::ZOOM_ACTUAL),
        ] {
            let entry = view
                .iter()
                .find(|e| e.action == Some(action.clone()))
                .unwrap_or_else(|| panic!("View has {action:?}"));
            assert_eq!(entry.shortcut, Some(key), "{}", entry.label);
        }
    }

    /// Undo and Redo say what they would do, and say why when there is nothing to.
    #[test]
    fn undo_and_redo_say_what_they_would_do() {
        let edit = entries(&model(&quiet()), "Edit");
        let undo = edit
            .iter()
            .find(|e| e.action == Some(MenuAction::Undo))
            .expect("Edit ▸ Undo");
        assert_eq!(undo.label, "Undo");
        assert!(!undo.enabled);
        assert_eq!(undo.hover(), Some(why::NOTHING_TO_UNDO));

        let state = MenuState {
            undo: Some("Delete 3 nodes".to_owned()),
            redo: Some("Move Checkerboard".to_owned()),
            ..quiet()
        };
        let edit = entries(&model(&state), "Edit");
        let label = |action: &MenuAction| {
            edit.iter()
                .find(|e| e.action.as_ref() == Some(action))
                .map(|e| (e.label.clone(), e.enabled))
        };
        assert_eq!(
            label(&MenuAction::Undo),
            Some(("Undo Delete 3 nodes".to_owned(), true))
        );
        assert_eq!(
            label(&MenuAction::Redo),
            Some(("Redo Move Checkerboard".to_owned(), true))
        );
    }

    /// A greyed-out entry says why on its hover: a file dialog up, nothing selected, nothing
    /// to paste, a render holding the playhead, the zoom at an end.
    #[test]
    fn a_greyed_entry_says_why() {
        let state = MenuState {
            file_busy: true,
            rendering: true,
            zoom: *crate::preferences::UI_ZOOM.end(),
            workspace: Some(("Main", LayoutMode::Canvas)),
            ..quiet()
        };
        let menus = model(&state);
        for (title, label, want) in [
            ("Project", "Open project…", why::FILE_DIALOG),
            ("Project", "Save", why::FILE_DIALOG),
            ("Workspace", "Export…", why::FILE_DIALOG),
            ("Edit", "Copy", why::NO_SELECTION),
            ("Edit", "Delete node", why::NO_SELECTION),
            ("Edit", "Paste", why::EMPTY_CLIPBOARD),
            ("View", "Pause", why::RENDERING),
            ("View", "Zoom in", why::LARGEST),
        ] {
            let entry = entries(&menus, title)
                .into_iter()
                .find(|e| e.label == label)
                .unwrap_or_else(|| panic!("{title} has {label}"));
            assert!(!entry.enabled, "{label} is greyed out");
            assert_eq!(entry.hover(), Some(want), "{label}");
        }
        // Enabled, an entry's hover is its hint and never the reason.
        let view = entries(&model(&quiet()), "View");
        let zoom_in = view
            .iter()
            .find(|e| e.label == "Zoom in")
            .expect("View ▸ Zoom in");
        assert!(zoom_in.enabled);
        assert_eq!(zoom_in.hover(), None);
        // At the display's own scale there is no actual size to go back to.
        let actual = view
            .iter()
            .find(|e| e.label == "Actual size")
            .expect("View ▸ Actual size");
        assert!(!actual.enabled);
        assert_eq!(actual.hover(), Some(why::ACTUAL));
    }

    /// The Workspace menu's New is the tab bar's `+`, with `Ctrl+T` beside it.
    #[test]
    fn workspace_new_carries_ctrl_t() {
        let state = MenuState {
            workspace: Some(("Main", LayoutMode::Canvas)),
            ..quiet()
        };
        let new = entries(&model(&state), "Workspace")
            .into_iter()
            .find(|e| e.action == Some(MenuAction::NewWorkspace))
            .expect("Workspace ▸ New");
        assert_eq!(new.shortcut, Some(keys::NEW_TAB));
    }
}
