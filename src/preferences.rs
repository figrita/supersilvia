// SPDX-License-Identifier: AGPL-3.0-or-later

//! Preferences: the saved state that is about the person rather than about the project.
//!
//! A project travels and a preference stays on the machine. Two people opening the same
//! project may disagree about the window size and the Status box, and the project
//! still means the same thing — that is the test for what belongs here.
//!
//! One file, in the XDG config directory. `App` owns the one [`Store`]; nothing else
//! constructs one and no preference reaches `graph/`, `compile/`, `nodes/`, `render/`,
//! `audio/` or `video/`, which `tests/rules.rs` checks.
//!
//! Nothing here is worth an error dialog. A file that cannot be read loads as defaults with
//! a warning, and is renamed `preferences.json.bad` before the first write over it, so a hand
//! edit gone wrong is kept rather than lost; a file that cannot be written leaves the
//! preferences in memory for this run.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Bumped only for a change this reader could not cope with. Adding a field does not need
/// it, because every field has a default.
const VERSION: u32 = 1;

/// How many files the Recent list holds.
const RECENT_CAP: usize = 10;

/// Overrides where preferences are read and written. Set to nothing for memory only.
pub const PATH_ENV: &str = "SUPERSILVIA_PREFERENCES";

/// The window as the last run left it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometry {
    /// Inner size, in points.
    pub size: [f32; 2],
    /// Outer top-left corner, in points. `None` where the compositor does not report a
    /// window's position, which on Wayland is always.
    pub position: Option<[f32; 2]>,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            size: [1280.0, 720.0],
            position: None,
            maximized: false,
        }
    }
}

/// Where one of the editor's own windows — the Status box, Preferences, MIDI, About, Licences —
/// was left, in points: its outer top-left corner, and its outer size where it can be resized.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub pos: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[f32; 2]>,
}

/// The editor's own windows by their titles, each where it was left.
pub type Placements = BTreeMap<String, Placement>;

/// Everything remembered between runs. `Default` is the app's behavior with no file, so a
/// missing one changes nothing.
// Each bool is one independent question a person answered about their own editor. Grouping
// them into sub-structs to satisfy a count would put a layer between the file and what it
// says, and the file is meant to be read by a person.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// The file's own version, not the app's.
    pub version: u32,
    /// The Status box, the frame-pacing readout: Preferences ▸ Performance, or its own ✕.
    pub show_status_box: bool,
    /// The synth's frames per second, at the right-hand end of the menu bar: one figure,
    /// always in view, for the evening the Status box is too much.
    pub show_fps: bool,
    /// The time readout beside the meter: the playhead, pause and back to zero. On by
    /// default, since pause lives there.
    pub show_time: bool,
    /// The cost strip under every node: evaluations per pixel, and GPU time on an Output.
    pub show_costs: bool,
    /// While dragging a number control, lock the cursor to the point the drag started at and
    /// hide it, so the scrub is relative motion with no screen edge — pick the mouse up and
    /// keep moving it past where a monitor would otherwise stop it. Off by default: a locked,
    /// invisible cursor is a bigger behavior change than any other preference here, worth
    /// opting into rather than out of.
    pub lock_cursor_while_scrubbing: bool,
    /// **Soft takeover** for MIDI faders: a CC whose value disagrees with its control's does
    /// nothing until the fader passes the control's value, then takes over, and the control
    /// wears a ghost mark where the fader is meanwhile. Off by default:
    /// a fader that moves its control at once is what a first-time hand expects.
    pub midi_soft_takeover: bool,
    /// The four anchors every color in the editor derives from.
    ///
    /// A preference and not a project field, deliberately: the editor looks the way this
    /// person likes it whichever project is open, and a project carried to another machine
    /// does not re-tint that machine's editor. The look a performance has is the *picture*,
    /// not the chrome around it — and a project-level override beneath a preference is how
    /// two-tier settings become a question nobody can answer.
    pub theme: crate::ui::theme::Theme,
    /// Hovering a port brightens every cable it carries and the port at the far end of each.
    ///
    /// silvia's `glowOnHover` without the glow. On by default: it is how you trace where a
    /// port goes without dragging anything.
    pub port_hover_highlight: bool,
    /// Let a cable sag between its ports, as a real one would.
    ///
    /// silvia's `droopyCables`, on by default as it is there. One offset on each control
    /// point, no cost, and people are fond of it.
    pub cable_droop: bool,
    /// Color every cable by a golden-angle walk of the hue circle instead of by what it
    /// carries, and outline each connected port in its cable's color.
    ///
    /// silvia's `phiSpacedWires`, and **off** where silvia has it on. The port colors are
    /// this editor's type system drawn — a pink diamond says *uniform color* before you have
    /// read anything — and a cable in its port's hue is that system continued along the wire.
    /// What phi spacing buys instead is telling two cables apart in a bundle, which is worth
    /// more the denser a patch gets and nothing at all on a small one. So it is offered and
    /// not assumed.
    pub phi_cables: bool,
    /// Cast a shadow under every node body.
    ///
    /// The one the Status box and the Preferences window already cast — `window_shadow`,
    /// which is a preference rather than a fixed part of the chrome because a shadow under
    /// a node is a matter of whether the graph should float over the mix or sit in it.
    pub node_shadow: bool,
    /// Invert the wheel along a Linear workspace's strip.
    ///
    /// silvia's `reverseScrolling`, kept because Linear mode maps the wheel to x, and a
    /// direction that feels wrong there is unusable rather than merely annoying.
    pub scroll_x_inverted: bool,
    /// The mode a new workspace opens in. A workspace's own mode is document data and saved
    /// with it; this is only the one it is born with.
    pub default_layout: crate::graph::LayoutMode,
    /// The Main Input panel folded to the left edge, and the Main Mixer to the right.
    ///
    /// A preference and not project data: whether you can see a panel is about the editor in
    /// front of you, and a project carried to a smaller screen should not unfold two panels
    /// on it. silvia keeps both in `localStorage`, which is the same answer.
    ///
    /// The Main Input starts **folded** where the Mixer starts open, because a new project
    /// has no source chosen and an empty panel is not worth three hundred points of canvas.
    /// Its spine says what it is, and one click opens it.
    pub main_input_collapsed: bool,
    pub mixer_collapsed: bool,
    /// The width each side panel was last dragged to, in points. `None` is the panel's own,
    /// 300 for the Main Input and 380 for the Mixer: a preference for the reason the folds
    /// are, since how much of the window a panel takes is about this screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_input_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixer_width: Option<f32>,
    /// How often the synth ticks. The display's own rate by default, which is what the
    /// editor's monitor reports; the two fixed rates are for a box whose display is not the
    /// rate the set wants to run at — a 144 Hz panel with the show going to 60.
    ///
    /// A preference rather than project data: it is about this machine tonight. See
    /// `proposals/deterministic-loop.md`.
    pub tick_rate: TickRate,
    /// Which of the Status box's sections are folded, and whether it lists every Output.
    pub status_folds: StatusFolds,
    pub window: WindowGeometry,
    /// How large the whole editor is drawn. See [`InterfaceSize`].
    pub interface_size: InterfaceSize,
    /// Where each of the editor's own windows was left, by its title. A window not here opens
    /// where it opens the first time.
    pub windows: Placements,
    /// Where New project makes a project, Untitled is made and Open project starts, chosen in
    /// the Preferences window. Absent is the default: `supersilvia` in the documents folder,
    /// [`crate::project::default_projects_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects_dir: Option<PathBuf>,
    /// Project folders opened or saved, most recent first, capped at ten and deduplicated.
    pub recent: Vec<PathBuf>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: VERSION,
            show_status_box: false,
            show_fps: false,
            show_time: true,
            show_costs: false,
            lock_cursor_while_scrubbing: false,
            midi_soft_takeover: false,
            theme: crate::ui::theme::Theme::default(),
            port_hover_highlight: true,
            cable_droop: true,
            phi_cables: false,
            node_shadow: true,
            scroll_x_inverted: false,
            default_layout: crate::graph::LayoutMode::default(),
            main_input_collapsed: true,
            tick_rate: TickRate::default(),
            status_folds: StatusFolds::default(),
            mixer_collapsed: false,
            main_input_width: None,
            mixer_width: None,
            window: WindowGeometry::default(),
            interface_size: InterfaceSize::default(),
            windows: Placements::new(),
            projects_dir: None,
            recent: Vec::new(),
        }
    }
}

impl Preferences {
    /// egui's zoom factor: the interface size's, on top of the display's own scale.
    pub fn scale(&self) -> f32 {
        self.interface_size.factor()
    }

    /// The projects folder: the one chosen, or the default. `None` only where there is no
    /// default and none was chosen, which is a machine with no `$HOME`.
    pub fn projects_dir(&self) -> Option<PathBuf> {
        self.projects_dir
            .clone()
            .or_else(crate::project::default_projects_dir)
    }

    /// The field a flag names.
    pub fn flag(&self, flag: Flag) -> bool {
        match flag {
            Flag::CursorLock => self.lock_cursor_while_scrubbing,
            Flag::PortHover => self.port_hover_highlight,
            Flag::CableDroop => self.cable_droop,
            Flag::PhiCables => self.phi_cables,
            Flag::NodeShadow => self.node_shadow,
            Flag::ScrollXInverted => self.scroll_x_inverted,
            Flag::Fps => self.show_fps,
            Flag::StatusBox => self.show_status_box,
            Flag::SoftTakeover => self.midi_soft_takeover,
        }
    }

    /// Read a file. A missing one, a truncated one and one from a newer supersilvia all
    /// give the defaults; the only trace is a warning in the log.
    pub fn load(path: &Path) -> Self {
        Self::read(path).unwrap_or_default()
    }

    /// Read a file: `Some` when it was there and readable, and `None` with a warning when it
    /// was missing — or there and not readable, which [`Store::flush`] keeps aside.
    fn read(path: &Path) -> Option<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                log::warn!("preferences {}: {e}; using defaults", path.display());
                return None;
            }
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(prefs) if prefs.version > VERSION => {
                log::warn!(
                    "preferences {} are version {}, this build reads {VERSION}; using defaults",
                    path.display(),
                    prefs.version,
                );
                None
            }
            Ok(prefs) => Some(prefs),
            Err(e) => {
                log::warn!(
                    "preferences {} are not readable: {e}; using defaults",
                    path.display(),
                );
                None
            }
        }
    }

    /// Write a file, or log why not. An unwritable path is not an error the user has to
    /// answer: the run continues with the preferences it has.
    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!(
                "could not make {}: {e}; preferences not saved",
                dir.display()
            );
            return;
        }
        match serde_json::to_string_pretty(self) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json + "\n") {
                    log::warn!("could not write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("could not serialize preferences: {e}"),
        }
    }

    /// Put a file at the front of the Recent list, removing it from wherever it was.
    pub fn push_recent(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(RECENT_CAP);
    }
}

/// Where preferences live: `supersilvia/preferences.json` in the machine's configuration
/// folder — `$XDG_CONFIG_HOME`, falling back to `$HOME/.config`, on Linux. See
/// [`crate::platform::dirs`].
///
/// [`PATH_ENV`] overrides it, which is how a test keeps its hands off the real file. `None`
/// means nowhere to write, and the preferences last for this run only.
pub fn path() -> Option<PathBuf> {
    if let Some(overridden) = std::env::var_os(PATH_ENV) {
        return (!overridden.is_empty()).then(|| PathBuf::from(overridden));
    }
    let config = crate::platform::dirs::config()?;
    Some(config.join("supersilvia").join("preferences.json"))
}

/// How often the synth ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TickRate {
    /// One tick per interval of the display the editor is on.
    #[default]
    Display,
    Hz60,
    Hz30,
}

impl TickRate {
    /// What the select shows, in order.
    pub const ALL: [(Self, &'static str); 3] = [
        (Self::Display, "Display"),
        (Self::Hz60, "60"),
        (Self::Hz30, "30"),
    ];

    /// One tick's interval in milliseconds, against the display's own.
    pub fn interval_ms(self, display_ms: f32) -> f32 {
        match self {
            Self::Display => display_ms,
            Self::Hz60 => 1000.0 / 60.0,
            Self::Hz30 => 1000.0 / 30.0,
        }
    }

    pub fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(r, _)| *r == self)
            .map_or("Display", |(_, l)| *l)
    }
}

/// How large the whole editor is drawn: Preferences ▸ Appearance ▸ Interface size, the one
/// control that scales it — for a display whose own scale is not the one wanted, and for
/// eyes that want everything larger.
///
/// **egui's zoom factor, not the fonts.** A node's rows, a number field and a panel's width
/// are fixed sizes in points, each sized for the text it holds, so a larger font alone would
/// clip in all of them. The zoom factor scales the points themselves: text, rows, controls and
/// panels grow together, on the display's own scale. It is set here and nowhere else: the
/// zoom keys, `Ctrl` with `+`, `-` and `0`, are the canvas's, as they are in every node editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InterfaceSize {
    Percent90,
    #[default]
    Percent100,
    Percent110,
    Percent125,
    Percent150,
}

impl InterfaceSize {
    /// What the row shows, in order.
    pub const ALL: [(Self, &'static str); 5] = [
        (Self::Percent90, "90%"),
        (Self::Percent100, "100%"),
        (Self::Percent110, "110%"),
        (Self::Percent125, "125%"),
        (Self::Percent150, "150%"),
    ];

    /// egui's zoom factor.
    pub fn factor(self) -> f32 {
        match self {
            Self::Percent90 => 0.9,
            Self::Percent100 => 1.0,
            Self::Percent110 => 1.1,
            Self::Percent125 => 1.25,
            Self::Percent150 => 1.5,
        }
    }
}

/// One of the answers the Preferences window ticks, each a `bool` on [`Preferences`] read
/// through [`Preferences::flag`] and written through [`Store::set_flag`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flag {
    CursorLock,
    PortHover,
    CableDroop,
    PhiCables,
    NodeShadow,
    ScrollXInverted,
    Fps,
    StatusBox,
    SoftTakeover,
}

/// The Status box's folds: `true` is folded to its one-line summary. What a person reads first
/// is open — the GPU, the CPU and the costliest Outputs — and the rest is a click away.
// One bool per section, each answered on its own; see `Preferences`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusFolds {
    pub gpu: bool,
    pub cpu: bool,
    pub editor: bool,
    pub outputs: bool,
    pub nodes: bool,
    pub project: bool,
    /// Every Output listed, rather than the costliest eight.
    pub all_outputs: bool,
}

impl Default for StatusFolds {
    fn default() -> Self {
        Self {
            gpu: false,
            cpu: false,
            editor: true,
            outputs: false,
            nodes: true,
            project: true,
            all_outputs: false,
        }
    }
}

impl StatusFolds {
    /// Every section open and every Output listed: what the copy writes.
    pub fn unfolded() -> Self {
        Self {
            gpu: false,
            cpu: false,
            editor: false,
            outputs: false,
            nodes: false,
            project: false,
            all_outputs: true,
        }
    }
}

/// The preferences, the file they came from, and whether this frame changed them.
///
/// Every setter compares before it marks, so reading the window's geometry back every frame
/// costs one comparison and no writes. [`Store::flush`] is called once at the end of a
/// frame, which is what caps this at one write per frame.
pub struct Store {
    prefs: Preferences,
    /// `None` writes nowhere.
    path: Option<PathBuf>,
    dirty: bool,
    /// The file was there and could not be read, and has not been put aside yet.
    unreadable: bool,
}

/// Write one preference, marking the store dirty only where the value moved. The single
/// spelling of the rule [`Store`]'s own doc states, so a new setter cannot forget it.
fn set<T: PartialEq>(slot: &mut T, value: T, dirty: &mut bool) {
    if *slot != value {
        *slot = value;
        *dirty = true;
    }
}

impl Store {
    /// Read the file at `path`, or start from the defaults when there is nowhere to read.
    pub fn load(path: Option<PathBuf>) -> Self {
        let read = path.as_deref().map(Preferences::read);
        let unreadable = path.as_deref().is_some_and(Path::exists) && matches!(read, Some(None));
        Self {
            prefs: read.flatten().unwrap_or_default(),
            path,
            dirty: false,
            unreadable,
        }
    }

    /// A store that touches no disk, for tests and for `App::headless`.
    pub fn in_memory() -> Self {
        Self::of(Preferences::default())
    }

    /// A store holding these preferences and touching no disk: what a test starts an app on
    /// when it wants something other than the defaults.
    pub fn of(prefs: Preferences) -> Self {
        Self {
            prefs,
            path: None,
            dirty: false,
            unreadable: false,
        }
    }

    pub fn get(&self) -> &Preferences {
        &self.prefs
    }

    /// The file this store reads and writes, or `None` for memory only.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Write the file now if it is not there yet, so it can be opened in an editor before
    /// anything has changed.
    pub fn write_if_missing(&mut self) {
        if self.path.as_deref().is_some_and(|p| !p.exists()) && !self.unreadable {
            self.dirty = true;
            self.flush();
        }
    }

    pub fn set_tick_rate(&mut self, rate: TickRate) {
        set(&mut self.prefs.tick_rate, rate, &mut self.dirty);
    }

    pub fn set_show_status_box(&mut self, on: bool) {
        set(&mut self.prefs.show_status_box, on, &mut self.dirty);
    }

    pub fn set_status_folds(&mut self, folds: StatusFolds) {
        set(&mut self.prefs.status_folds, folds, &mut self.dirty);
    }

    /// Fold the Main Input panel to the left edge, or unfold it.
    pub fn set_main_input_collapsed(&mut self, on: bool) {
        set(&mut self.prefs.main_input_collapsed, on, &mut self.dirty);
    }

    /// Fold the Main Mixer panel to the right edge, or unfold it.
    pub fn set_mixer_collapsed(&mut self, on: bool) {
        set(&mut self.prefs.mixer_collapsed, on, &mut self.dirty);
    }

    /// The width the Main Input panel was dragged to.
    pub fn set_main_input_width(&mut self, width: f32) {
        set(
            &mut self.prefs.main_input_width,
            Some(width),
            &mut self.dirty,
        );
    }

    /// The width the Main Mixer panel was dragged to.
    pub fn set_mixer_width(&mut self, width: f32) {
        set(&mut self.prefs.mixer_width, Some(width), &mut self.dirty);
    }

    pub fn set_show_time(&mut self, on: bool) {
        set(&mut self.prefs.show_time, on, &mut self.dirty);
    }

    pub fn set_show_costs(&mut self, on: bool) {
        set(&mut self.prefs.show_costs, on, &mut self.dirty);
    }

    /// Write the field a flag names.
    pub fn set_flag(&mut self, flag: Flag, on: bool) {
        let p = &mut self.prefs;
        let slot = match flag {
            Flag::CursorLock => &mut p.lock_cursor_while_scrubbing,
            Flag::PortHover => &mut p.port_hover_highlight,
            Flag::CableDroop => &mut p.cable_droop,
            Flag::PhiCables => &mut p.phi_cables,
            Flag::NodeShadow => &mut p.node_shadow,
            Flag::ScrollXInverted => &mut p.scroll_x_inverted,
            Flag::Fps => &mut p.show_fps,
            Flag::StatusBox => &mut p.show_status_box,
            Flag::SoftTakeover => &mut p.midi_soft_takeover,
        };
        set(slot, on, &mut self.dirty);
    }

    /// Write the four anchors. Called as a picker is dragged, so it goes through the same
    /// change-and-save path every other preference does and no theme edit is a command.
    pub fn set_theme(&mut self, theme: crate::ui::theme::Theme) {
        set(&mut self.prefs.theme, theme, &mut self.dirty);
    }

    pub fn set_default_layout(&mut self, mode: crate::graph::LayoutMode) {
        set(&mut self.prefs.default_layout, mode, &mut self.dirty);
    }

    pub fn set_window(&mut self, window: WindowGeometry) {
        set(&mut self.prefs.window, window, &mut self.dirty);
    }

    pub fn set_interface_size(&mut self, size: InterfaceSize) {
        set(&mut self.prefs.interface_size, size, &mut self.dirty);
    }

    /// Where the window of this title was left.
    pub fn set_placement(&mut self, title: &str, placement: Placement) {
        if self.prefs.windows.get(title) != Some(&placement) {
            self.prefs.windows.insert(title.to_owned(), placement);
            self.dirty = true;
        }
    }

    /// Choose the projects folder. The default's own path is stored as no choice, so the file
    /// keeps following the documents folder.
    pub fn set_projects_dir(&mut self, dir: PathBuf) {
        let chosen = (Some(&dir) != crate::project::default_projects_dir().as_ref()).then_some(dir);
        set(&mut self.prefs.projects_dir, chosen, &mut self.dirty);
    }

    pub fn push_recent(&mut self, path: PathBuf) {
        if self
            .prefs
            .recent
            .first()
            .is_some_and(|first| *first == path)
        {
            return;
        }
        self.prefs.push_recent(path);
        self.dirty = true;
    }

    /// Write, if something changed. Called once per frame.
    ///
    /// A file that was there and could not be read is first renamed beside itself, with
    /// `.bad` on its name, so the write does not lose whatever a hand put in it.
    pub fn flush(&mut self) {
        if !std::mem::take(&mut self.dirty) {
            return;
        }
        let Some(path) = self.path.as_deref() else {
            return;
        };
        if std::mem::take(&mut self.unreadable) {
            let bad = bad_path(path);
            match std::fs::rename(path, &bad) {
                Ok(()) => log::warn!(
                    "kept the unreadable preferences as {} and wrote new ones",
                    bad.display()
                ),
                Err(e) => log::warn!("could not keep {} aside: {e}", path.display()),
            }
        }
        self.prefs.save(path);
    }
}

/// Where an unreadable preferences file is kept: beside itself, `.bad` added to its name.
pub fn bad_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bad");
    path.with_file_name(name)
}
