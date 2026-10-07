// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Preferences window: the four theme anchors and silvia's sixteen looks, how the editor
//! behaves, the synth's rate, and where things are kept, on four tabs.
//!
//! **One height for every tab.** The window is as tall as its tallest tab, so a click on the
//! tab strip never moves the window's foot or anything under it. The heights are measured
//! when the window opens, and again when the interface size changes: every tab laid out in an
//! invisible child that takes no room, and the content area fixed at the greatest. On that one
//! frame the accessibility tree holds every tab's rows, the hidden ones disabled. The tab
//! strip is the Licences window's, a row of egui's selectable labels. The tab open last comes
//! back with the window for the rest of the run (`App`).
//!
//! An `egui::Window` rather than a modal overlay. The canvas is hand-painted because it is an
//! instrument; this is a settings surface, so it is ordinary egui widgets and is in the
//! accessibility tree by construction, with no hand-rolled `widget_info` anywhere in it.
//!
//! **Live preview, not OK/Cancel.** Dragging an anchor re-tints the editor in the same frame,
//! because the entire value of "a whole re-theme is four numbers" is watching what four
//! numbers do. There is nothing to confirm and nothing to roll back: the four numbers are a
//! preference, a preference is saved as it changes, and the way back to where you started is
//! the `vapor` preset that [`Theme::default`](super::theme::Theme::default) is.
//!
//! The swatches are the real `s-color` — [`super::color::swatch`] and its picker, the same
//! instrument a color port carries — rather than a settings-shaped lookalike. A person who
//! has learned to pick a color once has learned it everywhere.

use super::color;
use super::icon::{self, Icon};
use super::theme::{Hsl, PRESETS, Theme};
use crate::preferences::{Flag, Preferences};
use eframe::egui::{
    Align, Button, Context, FontId, Label, Layout, Rect, RichText, ScrollArea, Sense, TextStyle,
    Ui, UiBuilder, Window, vec2,
};
use std::path::Path;

/// Which of the four a control is editing. The window's own vocabulary: `Theme`'s fields are
/// named for what they anchor, and these are named for what a person is looking at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    Main,
    Number,
    Color,
    Event,
}

impl Anchor {
    pub const ALL: [Self; 4] = [Self::Main, Self::Number, Self::Color, Self::Event];

    /// What the swatch is labeled. "Ports" on three of them because that is where the hue
    /// is seen: a number port, a color port, an action port.
    fn label(self) -> &'static str {
        match self {
            Self::Main => "Main UI",
            Self::Number => "Number ports",
            Self::Color => "Color ports",
            Self::Event => "Event ports",
        }
    }

    fn of(self, theme: &Theme) -> Hsl {
        match self {
            Self::Main => theme.main,
            Self::Number => theme.number,
            Self::Color => theme.color,
            Self::Event => theme.event,
        }
    }

    fn set(self, theme: &mut Theme, hsl: Hsl) {
        let slot = match self {
            Self::Main => &mut theme.main,
            Self::Number => &mut theme.number,
            Self::Color => &mut theme.color,
            Self::Event => &mut theme.event,
        };
        *slot = hsl;
    }
}

/// What the window asks `App` to do. It never mutates anything itself, the same shape every
/// other surface in `ui/` has.
#[derive(Debug, Clone, PartialEq)]
pub enum PrefAction {
    /// The four anchors, as they now stand. Emitted while a picker is dragged, so it arrives
    /// many times a second and must stay cheap — it is four floats and a preference write.
    SetTheme(Theme),
    /// One of the checkboxes, ticked or unticked.
    SetFlag(Flag, bool),
    /// The mode a new workspace opens in.
    SetDefaultLayout(crate::graph::LayoutMode),
    /// The width and height a new Output is made at.
    SetOutputResolution(u32, u32),
    /// How far the canvas zooms out and in.
    SetZoomRange(f32, f32),
    /// How often the synth ticks: the display's rate, or a fixed one.
    SetTickRate(crate::preferences::TickRate),
    /// How large the whole editor is drawn.
    SetInterfaceSize(crate::preferences::InterfaceSize),
    /// The projects folder in the file manager.
    ShowProjects,
    /// A folder dialog for another projects folder.
    ChangeProjects,
    /// A folder dialog for a recordings folder of its own.
    ChooseRecordings,
    /// Recordings back into `recordings/` in the project.
    RecordingsInProject,
    /// Empty Project ▸ Recent.
    ClearRecent,
    /// `preferences.json` in the file manager.
    ShowPreferencesFile,
    /// `preferences.json` in the text editor.
    OpenPreferencesFile,
    /// The GPU the next start renders on, `None` for Automatic.
    SetGpu(Option<crate::render::adapter::AdapterId>),
    /// Whether the next start may render on a software GPU.
    SetSoftwareGpu(bool),
    /// Restart to apply: close as Quit does, asking what Quit asks, and start again.
    Restart,
    /// The window's own close button.
    Close,
}

/// What the window draws, beside the editor's own theme.
pub struct PrefsView<'a> {
    pub prefs: &'a Preferences,
    /// The projects folder, or `None` with nowhere to keep projects.
    pub projects: Option<&'a Path>,
    /// Why the projects folder cannot be read, where it cannot.
    pub projects_problem: Option<&'a str>,
    /// The folder a recording goes into now: the one chosen, or the project's `recordings/`.
    pub recordings: &'a Path,
    /// Where the preferences are written, or `None` where they live for this run only.
    pub file: Option<&'a Path>,
    /// A file dialog is already up, so Change… would open nothing.
    pub file_busy: bool,
    /// Every adapter the machine offered and the one in use, or `None` where the host chose.
    pub gpu: Option<&'a crate::render::adapter::Choice>,
    /// The GPU rows differ from what this run started on, so Restart to apply is offered.
    pub gpu_restart: bool,
}

/// The window's tabs, in the order the strip shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PrefsTab {
    /// The interface size, the theme and its presets, and how nodes and cables are drawn.
    #[default]
    Appearance,
    /// How the editor answers a hand and a controller.
    Editing,
    /// The synth's rate, the frame-pacing readouts and the GPU.
    Performance,
    /// The projects folder and the preferences file.
    Files,
}

impl PrefsTab {
    pub const ALL: [Self; 4] = [
        Self::Appearance,
        Self::Editing,
        Self::Performance,
        Self::Files,
    ];

    /// The tab's name on the strip.
    pub fn label(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Editing => "Editing",
            Self::Performance => "Performance",
            Self::Files => "Files",
        }
    }
}

/// The window's state across frames: the tab showing, which swatch has its picker open, and
/// the height every tab is drawn at.
#[derive(Debug, Default)]
pub struct PrefsState {
    picking: Option<Anchor>,
    /// The new Outputs' resolution picker has its popover up.
    sizing: bool,
    tab: PrefsTab,
    /// The tallest tab's height in points, and the pixels per point it was measured at.
    tallest: Option<(f32, f32)>,
}

impl PrefsState {
    /// A window opening on `tab`.
    pub fn on(tab: PrefsTab) -> Self {
        Self {
            tab,
            ..Self::default()
        }
    }

    /// The tab showing.
    pub fn tab(&self) -> PrefsTab {
        self.tab
    }
}

/// One swatch's height, and the picker's reach below it.
const SWATCH: f32 = 28.0;

/// Draw the window. `theme` is what the editor is drawing with *now*, which is what the
/// swatches show: there is no separate draft copy to fall out of step with the screen.
///
/// The tab strip, and under it the tab showing at the tallest tab's height, scrolling inside
/// the window where the screen is shorter than that.
pub fn show(
    ctx: &Context,
    state: &mut PrefsState,
    theme: &Theme,
    view: &PrefsView<'_>,
) -> Vec<PrefAction> {
    let mut actions = Vec::new();
    let mut open = true;
    let prefs = view.prefs;

    let window = Window::new(super::placed::PREFERENCES)
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .default_width(WIDTH);
    let room = (ctx.content_rect().height() - 160.0).max(200.0);
    super::placed::place(ctx, window, super::placed::PREFERENCES, &prefs.windows).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for tab in PrefsTab::ALL {
                if ui.selectable_label(state.tab == tab, tab.label()).clicked() {
                    state.tab = tab;
                    state.picking = None;
                    state.sizing = false;
                }
            }
        });
        ui.separator();
        let ppp = ui.ctx().pixels_per_point();
        let tallest = match state.tallest {
            Some((height, at)) if at == ppp => height,
            _ => {
                let height = measure(ui, theme, view);
                state.tallest = Some((height, ppp));
                height
            }
        };
        let height = tallest.min(room);
        let tab = state.tab;
        ScrollArea::vertical()
            .id_salt(("prefs", tab))
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| body(ui, state, theme, view, tab, &mut actions));
    });

    if !open {
        // Re-opening the window does not bring back a picker nobody asked for.
        state.picking = None;
        state.sizing = false;
        actions.push(PrefAction::Close);
    }
    actions
}

/// The window's width, which every row is laid out across.
const WIDTH: f32 = 480.0;

/// The tallest tab's height: each laid out in a child that is invisible, takes no room in the
/// window and answers no pointer, with a picker-less state of its own and its actions dropped.
fn measure(ui: &mut Ui, theme: &Theme, view: &PrefsView<'_>) -> f32 {
    let width = ui.available_width();
    let at = ui.cursor().min;
    PrefsTab::ALL
        .into_iter()
        .map(|tab| {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(("prefs-measure", tab))
                    .max_rect(Rect::from_min_size(at, vec2(width, f32::INFINITY)))
                    .layout(Layout::top_down(Align::Min))
                    .invisible(),
            );
            body(
                &mut child,
                &mut PrefsState::default(),
                theme,
                view,
                tab,
                &mut Vec::new(),
            );
            child.min_rect().height()
        })
        .fold(0.0, f32::max)
}

/// One tab's sections, top to bottom.
fn body(
    ui: &mut Ui,
    state: &mut PrefsState,
    theme: &Theme,
    view: &PrefsView<'_>,
    tab: PrefsTab,
    actions: &mut Vec<PrefAction>,
) {
    let prefs = view.prefs;
    let rule = |ui: &mut Ui| {
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
    };
    match tab {
        PrefsTab::Appearance => {
            interface_size(ui, prefs, actions);
            rule(ui);
            ui.heading("Theme");
            ui.label("Every color in the editor derives from these four.");
            ui.add_space(8.0);
            anchors(ui, state, theme, actions);
            rule(ui);
            ui.heading("Presets");
            presets(ui, theme, actions);
            rule(ui);
            ui.heading("Nodes and cables");
            for row in CANVAS {
                flag(ui, prefs, row, actions);
            }
        }
        PrefsTab::Editing => editing(ui, state, theme, prefs, actions),
        PrefsTab::Performance => {
            performance(ui, prefs, actions);
            rule(ui);
            ui.heading("GPU");
            gpu(ui, view, theme, actions);
        }
        PrefsTab::Files => files(ui, view, theme, actions),
    }
}

/// The four swatches in a row, each under its label, each the real `s-color`.
fn anchors(ui: &mut Ui, state: &mut PrefsState, theme: &Theme, actions: &mut Vec<PrefAction>) {
    let previously = state.picking;
    let mut picking = state.picking;
    let mut popups: Vec<(Anchor, Rect)> = Vec::new();

    ui.columns(4, |cols| {
        for (col, anchor) in cols.iter_mut().zip(Anchor::ALL) {
            col.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.label(anchor.label());
                let width = ui.available_width().min(72.0);
                let (rect, _) = ui.allocate_exact_size(vec2(width, SWATCH), Sense::click());
                let hsl = anchor.of(theme);
                let value = rgba(hsl);
                // `swatch` appends the color to whatever it is handed, exactly as a port's
                // does, so the accessible name here reads "Main UI #3adceeff".
                let response =
                    color::swatch(ui, rect, anchor.label(), value, theme, true, false, 1.0);
                crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
                if response.clicked() {
                    picking = if picking == Some(anchor) {
                        None
                    } else {
                        Some(anchor)
                    };
                }
                if picking == Some(anchor) {
                    popups.push((anchor, rect));
                }
            });
        }
    });

    // The click that opens a picker is also a click outside the picker, which had not been
    // drawn yet when it happened — so the frame a popup goes up is the one frame its own
    // dismissal has to be ignored. The canvas carries the same guard for a node's picker,
    // and without it a swatch opens and shuts in the same frame and appears to do nothing.
    let allow_dismiss = previously == picking;

    // Drawn after every swatch, so a swatch laid out later cannot cover the popup — the same
    // ordering rule the canvas follows for a node's picker.
    for (anchor, rect) in popups {
        let (changed, dismissed) = color::picker(
            ui,
            rect.left_bottom(),
            ("prefs", anchor),
            rgba(anchor.of(theme)),
            theme,
        );
        if let Some(next) = changed {
            let mut next_theme = *theme;
            anchor.set(&mut next_theme, hsl_of(next, anchor.of(theme).h));
            actions.push(PrefAction::SetTheme(next_theme));
        }
        if dismissed && allow_dismiss {
            picking = None;
        }
    }
    state.picking = picking;
}

/// The checkboxes under Appearance ▸ Nodes and cables, in the order the tab lists them: what each
/// ticks, its label, and what hovering it says.
const CANVAS: [(Flag, &str, &str); 3] = [
    (
        Flag::NodeShadow,
        "Nodes cast a shadow",
        "Put the same shadow under every node that the Status box and this window cast, \
         so the graph floats over the mix rather than sitting in it.",
    ),
    (
        Flag::CableDroop,
        "Droopy cables",
        "Let a cable sag between its ports, as a real one would.",
    ),
    (
        Flag::PhiCables,
        "Phi-spaced cable colors",
        "Give every cable its own color, walked around the hue circle by the golden \
         angle so neighbours are as unlike as they can be, and outline each connected \
         port to match. Off, a cable wears the color of what it carries.",
    ),
];

/// The checkboxes on the Editing tab, in the order it lists them.
const EDITING: [(Flag, &str, &str); 4] = [
    (
        Flag::CursorLock,
        "Lock the cursor while scrubbing",
        "While a number is dragged, hold the pointer where the drag started and hide it, \
         so the scrub has no screen edge to run into.",
    ),
    (
        Flag::PortHover,
        "Light cables and ports on hover",
        "Hovering a port brightens every cable it carries and the port at the far end \
         of each; hovering a cable brightens the ports at both of its ends. Either way \
         you can trace where something goes without dragging it.",
    ),
    (
        Flag::ScrollXInverted,
        "Invert scrolling along a strip",
        "A Linear workspace maps the wheel along its strip. This is which way it goes.",
    ),
    (
        Flag::SoftTakeover,
        "MIDI soft takeover",
        "A bound fader or knob whose position disagrees with its control moves nothing \
         until it passes the control's value, then takes over, so the picture never \
         jumps. Meanwhile a mark on the control shows where the fader is.",
    ),
];

/// The checkboxes on the Performance tab, after the tick rate.
const PERFORMANCE: [(Flag, &str, &str); 2] = [
    (
        Flag::Fps,
        "Frame rate in the menu bar",
        "The frames a second the synth is actually making, at the right-hand end of the \
         menu bar — the Tick rate above is what it is asked for.",
    ),
    (
        Flag::StatusBox,
        "Show the Status box",
        "The frame-pacing readout, in a window of its own: how fast the synth is running \
         and what is holding it back. Its own ✕ closes it as well.",
    ),
];

/// One checkbox from a table row.
fn flag(
    ui: &mut Ui,
    prefs: &Preferences,
    (flag, label, hint): (Flag, &str, &str),
    actions: &mut Vec<PrefAction>,
) {
    let mut on = prefs.flag(flag);
    let entry = ui.checkbox(&mut on, label).on_hover_text(hint);
    crate::ui::pointing(&entry);
    if entry.changed() {
        actions.push(PrefAction::SetFlag(flag, on));
    }
}

/// How large the whole editor is drawn: one row of five sizes.
fn interface_size(ui: &mut Ui, prefs: &Preferences, actions: &mut Vec<PrefAction>) {
    ui.horizontal(|ui| {
        ui.label("Interface size");
        for (size, name) in crate::preferences::InterfaceSize::ALL {
            if ui
                .selectable_label(prefs.interface_size == size, name)
                .clicked()
            {
                actions.push(PrefAction::SetInterfaceSize(size));
            }
        }
    })
    .response
    .on_hover_text(
        "How large the whole editor is drawn: text, node rows, controls and panels grow \
         together, so nothing sized for its text is cut off. Ctrl with + and − zoom the canvas.",
    );
}

/// The Editing tab: a handful of answers about how the editor behaves, each independent of
/// the others.
fn editing(
    ui: &mut Ui,
    state: &mut PrefsState,
    theme: &Theme,
    prefs: &Preferences,
    actions: &mut Vec<PrefAction>,
) {
    for row in EDITING {
        flag(ui, prefs, row, actions);
    }
    ui.horizontal(|ui| {
        ui.label("New workspaces open as");
        for (mode, name) in [
            (crate::graph::LayoutMode::Canvas, "Canvas"),
            (crate::graph::LayoutMode::Linear, "Linear"),
        ] {
            if ui
                .selectable_label(prefs.default_layout == mode, name)
                .clicked()
            {
                actions.push(PrefAction::SetDefaultLayout(mode));
            }
        }
    });
    output_resolution(ui, state, theme, prefs, actions);
    zoom_range(ui, theme, prefs, actions);
}

/// How far the canvas zooms out and in: two typed percentages, each held to its bounds.
fn zoom_range(ui: &mut Ui, theme: &Theme, prefs: &Preferences, actions: &mut Vec<PrefAction>) {
    use crate::preferences::{ZOOM_LEAST, ZOOM_MOST};
    let (least, most) = prefs.zoom_range();
    let field = |(min, max): (f32, f32)| crate::nodes::NumberField {
        min: min * 100.0,
        max: max * 100.0,
        integer: true,
        unit: "%",
    };
    let percent = |v: f32| format!("{}", (v * 100.0).round());
    // Each field is drawn in a child over the room taken for it, so the field's own layout of
    // its text leaves the row's cursor past the field rather than inside it.
    let typed = |ui: &mut Ui, name: &str, value: f32, bounds| {
        let size = vec2(64.0, ui.spacing().interact_size.y);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let mut child = ui.new_child(UiBuilder::new().id_salt(name).max_rect(rect));
        super::text::number(
            &mut child,
            rect,
            name,
            &percent(value),
            bounds,
            super::text::Chrome::Row,
            theme,
            1.0,
        )
    };
    ui.horizontal(|ui| {
        ui.label("Canvas zoom from");
        let out = typed(ui, "Least canvas zoom", least, field(ZOOM_LEAST));
        ui.label("to");
        let into = typed(ui, "Greatest canvas zoom", most, field(ZOOM_MOST));
        if let Some(v) = out {
            actions.push(PrefAction::SetZoomRange(v / 100.0, most));
        }
        if let Some(v) = into {
            actions.push(PrefAction::SetZoomRange(least, v / 100.0));
        }
    })
    .response
    .on_hover_text(
        "How far the wheel and the zoom keys take the canvas out and in: from 5% to 100% \
         at the far end, from 100% to 800% at the near one.",
    );
}

/// The size a new Output is made at: a caption and the resolution picker an Output's own row
/// draws, its popover under the closed row while it is up.
fn output_resolution(
    ui: &mut Ui,
    state: &mut PrefsState,
    theme: &Theme,
    prefs: &Preferences,
    actions: &mut Vec<PrefAction>,
) {
    use super::resolution;
    const NAME: &str = "New Outputs";
    let (w, h) = prefs
        .output_resolution
        .unwrap_or(crate::nodes::output::DEFAULT_RESOLUTION);
    let shown = resolution::Shown::Size(w, h);
    let row = ui.horizontal(|ui| {
        ui.label("New Outputs render at");
        let width = resolution::closed_width(ui.ctx(), shown, 1.0) + 12.0;
        let (rect, _) =
            ui.allocate_exact_size(vec2(width, ui.spacing().interact_size.y), Sense::hover());
        let response = resolution::closed(ui, rect, NAME, shown, state.sizing, theme, 1.0);
        if response.clicked() {
            state.sizing = !state.sizing;
        }
        (rect, response.clicked())
    });
    let (rect, clicked) = row.inner;
    row.response.on_hover_text(
        "The resolution an Output is made at, from the Nodes menu, the browser or a new \
         workspace. Outputs already made, and every project opened, keep their own.",
    );
    if state.sizing {
        let popped = resolution::popover(ui, rect.left_bottom(), NAME, Some((w, h)), &[], theme);
        if let Some(resolution::Pick::Size(w, h)) = popped.picked {
            actions.push(PrefAction::SetOutputResolution(w, h));
        }
        if popped.dismissed && !clicked {
            state.sizing = false;
        }
    }
}

/// The synth's rate. One row: the display it is on, or a number.
///
/// The synth keeps its own time — see `proposals/deterministic-loop.md` — so this is the
/// rate the world runs at, not the rate anything is drawn at. A window shows the newest
/// frame whenever its own compositor asks it to.
fn performance(ui: &mut Ui, prefs: &Preferences, actions: &mut Vec<PrefAction>) {
    ui.horizontal(|ui| {
        ui.label("Tick rate");
        for (rate, name) in crate::preferences::TickRate::ALL {
            if ui.selectable_label(prefs.tick_rate == rate, name).clicked() {
                actions.push(PrefAction::SetTickRate(rate));
            }
        }
    })
    .response
    .on_hover_text(
        "How often the graph advances, in ticks a second. Display follows the monitor the \
         editor is on.",
    );
    for row in PERFORMANCE {
        flag(ui, prefs, row, actions);
    }
}

/// Where things are kept: the projects folder, the recordings folder, the recent projects and
/// the preferences file, each a caption with its buttons on the right and its path, or what it
/// holds, on the line under it.
///
/// **A path is one line whatever it is.** It is drawn in monospace, cut in the middle with `…`
/// where it is longer than the line — the start says which disk and the end which folder, and
/// the middle is what can go — and whole on its hover, so no path moves a row.
fn files(ui: &mut Ui, view: &PrefsView<'_>, theme: &Theme, actions: &mut Vec<PrefAction>) {
    let show = format!("Show in {}", crate::platform::files::MANAGER);
    file_row(ui, "Projects folder", |ui| {
        if ui.add(Button::new("Change…")).clicked() && !view.file_busy {
            actions.push(PrefAction::ChangeProjects);
        }
        let entry = ui.add_enabled(view.projects.is_some(), Button::new(show.as_str()));
        crate::ui::pointing(&entry);
        if entry.clicked() {
            actions.push(PrefAction::ShowProjects);
        }
    })
    .on_hover_text("Where New project makes a project and Open project… starts.");
    path_line(
        ui,
        view.projects,
        &format!(
            "nowhere to keep projects: {}",
            crate::platform::dirs::DOCUMENTS_UNSET
        ),
        theme,
    );
    if let Some(problem) = view.projects_problem {
        ui.add(Label::new(RichText::new(problem).color(ui.visuals().error_fg_color)).truncate());
    }
    ui.add_space(6.0);
    let chosen = view.prefs.recordings_dir.is_some();
    file_row(ui, "Recordings folder", |ui| {
        if ui.add(Button::new("Choose…")).clicked() && !view.file_busy {
            actions.push(PrefAction::ChooseRecordings);
        }
        let entry = ui.add_enabled(chosen, Button::new("In the project"));
        crate::ui::pointing(&entry);
        if entry.clicked() {
            actions.push(PrefAction::RecordingsInProject);
        }
    })
    .on_hover_text(
        "Where an Output's Record row writes. In the project is recordings/ in the project's \
         folder; a folder chosen here takes every project's recordings, each file's name \
         starting with its project's.",
    );
    path_line(ui, Some(view.recordings), "", theme);
    ui.add_space(6.0);
    let recent = view.prefs.recent.len();
    file_row(ui, "Recent projects", |ui| {
        let entry = ui.add_enabled(recent > 0, Button::new("Clear"));
        crate::ui::pointing(&entry);
        if entry.clicked() {
            actions.push(PrefAction::ClearRecent);
        }
    });
    let held = match recent {
        0 => "Project ▸ Recent is empty".to_string(),
        1 => "1 project in Project ▸ Recent".to_string(),
        n => format!("{n} projects in Project ▸ Recent"),
    };
    ui.label(RichText::new(held).color(theme.text_muted()));
    ui.add_space(6.0);
    file_row(ui, "Preferences file", |ui| {
        let there = view.file.is_some();
        let entry = ui.add_enabled(there, Button::new("Open"));
        crate::ui::pointing(&entry);
        if entry.clicked() {
            actions.push(PrefAction::OpenPreferencesFile);
        }
        let entry = ui.add_enabled(there, Button::new("Show"));
        crate::ui::pointing(&entry);
        if entry.clicked() {
            actions.push(PrefAction::ShowPreferencesFile);
        }
    });
    path_line(ui, view.file, "not saved: kept for this run only", theme);
    ui.label(
        RichText::new(
            "Edits take effect the next time supersilvia starts: while it runs, it writes its \
             own copy.",
        )
        .color(theme.text_muted()),
    );
}

/// A caption on the left and its buttons on the right, laid right to left so the first button
/// added is the rightmost.
fn file_row(ui: &mut Ui, caption: &str, buttons: impl FnOnce(&mut Ui)) -> eframe::egui::Response {
    ui.horizontal(|ui| {
        ui.label(caption);
        ui.with_layout(Layout::right_to_left(Align::Center), buttons);
    })
    .response
}

/// A path on one line in monospace, cut in the middle to fit the window's width, and whole on
/// its hover; `missing` in its place where there is none.
fn path_line(ui: &mut Ui, path: Option<&Path>, missing: &str, theme: &Theme) {
    let Some(path) = path else {
        ui.label(RichText::new(missing).color(theme.text_muted()));
        return;
    };
    let whole = path.display().to_string();
    let font = TextStyle::Monospace.resolve(ui.style());
    let fits = fits_in(ui, &font, ui.available_width());
    ui.add(Label::new(RichText::new(middle_cut(&whole, fits)).monospace()).truncate())
        .on_hover_text(whole);
}

/// How many characters of a monospace face fit across `width`.
fn fits_in(ui: &Ui, font: &FontId, width: f32) -> usize {
    let glyph = ui.fonts_mut(|f| f.glyph_width(font, 'M')).max(1.0);
    (width / glyph).floor().max(3.0) as usize
}

/// `text` in at most `fits` characters: whole when it fits, and otherwise its start and its
/// end with `…` between them, the end given the extra character.
pub fn middle_cut(text: &str, fits: usize) -> String {
    let count = text.chars().count();
    if count <= fits {
        return text.to_string();
    }
    let keep = fits.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start}…{end}")
}

/// What the editor and the synth draw on: the adapter in use, how it was picked, and every
/// adapter the machine offered with the one in use marked; then which of them the next start
/// renders on, whether it may take a software one, and Restart to apply.
///
/// A grid of a caption and its value. Every value is one line, cut at its end and whole on its
/// hover: a driver's version string is as long as its vendor likes. **The two settings take
/// effect at the next start**, since the one device is made before the window, and a variable
/// in the environment wins over each — the row says which and is not editable. The last row is
/// a note, with Restart to apply before it while either differs from the start, and is as tall
/// as the button either way, so the button arriving moves nothing.
fn gpu(ui: &mut Ui, view: &PrefsView<'_>, theme: &Theme, actions: &mut Vec<PrefAction>) {
    let Some(choice) = view.gpu else {
        ui.label(RichText::new("Not reported by this host.").color(theme.text_muted()));
        return;
    };
    eframe::egui::Grid::new("prefs-gpu")
        .num_columns(2)
        .spacing(vec2(12.0, 6.0))
        .show(ui, |ui| {
            caption(ui, "In use");
            ui.vertical(|ui| adapter_lines(ui, choice.in_use(), None, theme));
            ui.end_row();
            caption(ui, "Picked by");
            one_line(ui, RichText::new(choice.how()));
            ui.end_row();
            caption(ui, "Offered");
            ui.vertical(|ui| {
                for (i, info) in choice.offered.iter().enumerate() {
                    adapter_lines(ui, info, Some(i == choice.chosen), theme);
                }
            });
            ui.end_row();
            let label = ui
                .vertical(|ui| ui.add(Label::new("Use GPU").extend()))
                .inner;
            ui.vertical(|ui| use_gpu(ui, view.prefs, choice, label.id, theme, actions));
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| software_gpu(ui, view.prefs, choice, theme, actions));
            ui.end_row();
        });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        // As tall as the button whether or not it is there.
        let button = ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
        ui.set_min_height(button.max(ui.spacing().interact_size.y));
        let note = if view.gpu_restart {
            let restart = ui.add(Button::new("Restart to apply")).on_hover_text(
                "Close supersilvia as Quit does, asking first about unsaved changes, and \
                     start it again on this project.",
            );
            crate::ui::pointing(&restart);
            if restart.clicked() {
                actions.push(PrefAction::Restart);
            }
            "The GPU rows take effect when supersilvia starts again."
        } else {
            "Takes effect the next time supersilvia starts."
        };
        one_line(ui, RichText::new(note).color(theme.text_muted()));
    });
}

/// What an adapter is called in the Use GPU select: its name and its backend, which tells two
/// backends' views of one card apart and is never the name the Offered list gives it.
fn adapter_choice(info: &eframe::wgpu::AdapterInfo) -> String {
    format!("{} · {:?}", info.name, info.backend)
}

/// What the Use GPU select says for Automatic.
const AUTOMATIC: &str = "Automatic (the strongest)";

/// The Use GPU select: Automatic, or one of the adapters offered, a software one only where a
/// software GPU is allowed. Disabled, with the variable named under it, while
/// `SUPERSILVIA_ADAPTER` decides. Named by the caption beside it, `label`.
fn use_gpu(
    ui: &mut Ui,
    prefs: &Preferences,
    choice: &crate::render::adapter::Choice,
    label: eframe::egui::Id,
    theme: &Theme,
    actions: &mut Vec<PrefAction>,
) {
    use crate::render::adapter::{ADAPTER_ENV, AdapterId};
    let software = choice
        .asked
        .software_env
        .unwrap_or(prefs.allow_software_gpu);
    let selected = match &prefs.gpu {
        None => AUTOMATIC.to_owned(),
        Some(pin) => choice
            .offered
            .iter()
            .find(|info| pin.is(info))
            .map_or_else(|| format!("{} (not offered)", pin.name), adapter_choice),
    };
    let mut picked = None;
    ui.add_enabled_ui(choice.asked.adapter.is_none(), |ui| {
        let select = eframe::egui::ComboBox::from_id_salt("prefs-use-gpu")
            .selected_text(selected)
            .width(ui.available_width().min(320.0))
            .truncate()
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(prefs.gpu.is_none(), AUTOMATIC)
                    .clicked()
                {
                    picked = Some(None);
                }
                for info in &choice.offered {
                    let id = AdapterId::of(info);
                    let usable = software || info.device_type != eframe::wgpu::DeviceType::Cpu;
                    let entry = ui
                        .add_enabled(
                            usable,
                            Button::selectable(
                                prefs.gpu.as_ref() == Some(&id),
                                adapter_choice(info),
                            ),
                        )
                        .on_disabled_hover_text("A software GPU needs Allow a software GPU.");
                    if entry.clicked() {
                        picked = Some(Some(id));
                    }
                }
            })
            .response
            .labelled_by(label);
        crate::ui::pointing(&select);
        select.on_hover_text(
            "The GPU supersilvia renders on from its next start. Automatic takes the strongest: \
             a discrete GPU before an integrated one.",
        );
    });
    if let Some(wanted) = &choice.asked.adapter {
        one_line(
            ui,
            RichText::new(format!("Overridden by {ADAPTER_ENV}={wanted}"))
                .color(theme.text_muted()),
        );
    }
    if let Some(gpu) = picked
        && gpu != prefs.gpu
    {
        actions.push(PrefAction::SetGpu(gpu));
    }
}

/// Allow a software GPU: disabled, with the variable named under it, while
/// `SUPERSILVIA_SOFTWARE_GPU` decides.
fn software_gpu(
    ui: &mut Ui,
    prefs: &Preferences,
    choice: &crate::render::adapter::Choice,
    theme: &Theme,
    actions: &mut Vec<PrefAction>,
) {
    use crate::render::adapter::SOFTWARE_ENV;
    // One line, as every value in the grid is.
    ui.style_mut().wrap_mode = Some(eframe::egui::TextWrapMode::Extend);
    let mut on = prefs.allow_software_gpu;
    let entry = ui
        .add_enabled(
            choice.asked.software_env.is_none(),
            eframe::egui::Checkbox::new(&mut on, "Allow a software GPU"),
        )
        .on_hover_text(
            "Let supersilvia render on the CPU, from its next start, where the machine has no GPU \
             it will take — a virtual machine, most often. Slow, and no measure of a real GPU.",
        );
    crate::ui::pointing(&entry);
    if entry.changed() {
        actions.push(PrefAction::SetSoftwareGpu(on));
    }
    if let Some(set) = choice.asked.software_env {
        let by = if set {
            format!("Overridden by {SOFTWARE_ENV}=1")
        } else {
            format!("Overridden by {SOFTWARE_ENV}, set and not 1")
        };
        one_line(ui, RichText::new(by).color(theme.text_muted()));
    }
}

/// One adapter in up to three lines: its name; its kind and backend; and its driver and the
/// driver's own version, muted. `in_use` marks it in a list with a painted dot before the
/// lines, as every status dot is: filled, named *in use*, and *(in use)* after the name for the
/// one rendered on; hollow, named *not in use*, for the rest.
fn adapter_lines(
    ui: &mut Ui,
    info: &eframe::wgpu::AdapterInfo,
    in_use: Option<bool>,
    theme: &Theme,
) {
    match in_use {
        Some(on) => {
            ui.horizontal_top(|ui| {
                let (mark, ink, state) = if on {
                    (Icon::Dot, theme.text_primary(), "in use")
                } else {
                    (Icon::Ring, theme.text_muted(), "not in use")
                };
                icon::label(ui, mark, ink, state);
                ui.vertical(|ui| {
                    let name = if on {
                        format!("{}  (in use)", info.name)
                    } else {
                        info.name.clone()
                    };
                    adapter_text(ui, info, name, theme);
                });
            });
        }
        None => adapter_text(ui, info, info.name.clone(), theme),
    }
}

/// An adapter's three lines under `name`: its kind and backend, then its driver, muted.
fn adapter_text(ui: &mut Ui, info: &eframe::wgpu::AdapterInfo, name: String, theme: &Theme) {
    one_line(ui, RichText::new(name));
    let kind = format!(
        "{} · {:?}",
        crate::render::adapter::kind(info.device_type),
        info.backend
    );
    one_line(ui, RichText::new(kind).color(theme.text_muted()));
    let driver = [info.driver.as_str(), info.driver_info.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !driver.is_empty() {
        one_line(ui, RichText::new(driver).color(theme.text_muted()));
    }
}

/// A grid row's caption, at the top of its row rather than beside the middle of a tall value.
fn caption(ui: &mut Ui, text: &str) {
    ui.vertical(|ui| ui.add(Label::new(text).extend()));
}

/// A label that is one line however long: cut at its end, and whole on its hover, which
/// egui gives a truncated label of its own accord.
fn one_line(ui: &mut Ui, text: RichText) {
    ui.add(Label::new(text).truncate());
}

/// The sixteen looks, four to a row, each its emoji and its name.
fn presets(ui: &mut Ui, theme: &Theme, actions: &mut Vec<PrefAction>) {
    for row in PRESETS.chunks(4) {
        ui.columns(4, |cols| {
            for (col, preset) in cols.iter_mut().zip(row) {
                // The look the editor is wearing is marked, and it stays marked only while
                // the four numbers are still that preset's: drag an anchor and no preset is
                // selected, because none of them is what is on screen any more.
                let showing = preset.theme == *theme;
                let label = format!("{} {}", preset.icon, preset.label);
                if col
                    .selectable_label(showing, label)
                    .on_hover_text(preset.key)
                    .clicked()
                {
                    actions.push(PrefAction::SetTheme(preset.theme));
                }
            }
        });
    }
}

/// `Hsl` and the picker's `[f32; 4]` are the two shapes a color has here: the anchors are
/// HSL because the design system is, and the picker speaks premultiplied-free RGBA because
/// that is what a color control carries everywhere else. An anchor is always opaque.
fn rgba(hsl: Hsl) -> [f32; 4] {
    let c = hsl.color();
    [
        f32::from(c.r()) / 255.0,
        f32::from(c.g()) / 255.0,
        f32::from(c.b()) / 255.0,
        1.0,
    ]
}

/// `keep_hue` is the anchor's current hue, so dragging its saturation to zero and back does
/// not come home a different color.
fn hsl_of(value: [f32; 4], keep_hue: f32) -> Hsl {
    Hsl::from_rgb(value[0], value[1], value[2], keep_hue)
}

#[cfg(test)]
mod tests {
    use super::middle_cut;

    #[test]
    fn a_long_path_is_cut_in_the_middle_and_a_short_one_is_whole() {
        let path = "/home/ana/Dokumente/supersilvia";
        assert_eq!(middle_cut(path, 40), path);
        assert_eq!(middle_cut(path, path.len()), path);
        let cut = middle_cut(path, 15);
        assert_eq!(cut.chars().count(), 15);
        assert_eq!(cut, "/home/a…rsilvia");
    }
}
