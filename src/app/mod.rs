// SPDX-License-Identifier: AGPL-3.0-or-later

//! The application: it owns the document, and drives one frame.
//!
//! What a node *computes* is not here — that is [`crate::synth::Synth`], which this owns one
//! of and hands the graph on every edit. The line between the two is `docs/cpu.md`'s: a saved
//! file's state against a running node's.
//!
//! **`App` keeps the view and the frame** — the canvas, the tabs, the start menu, the theme,
//! the preferences window, the confirm, the frame meters — and owns the rest as four parts,
//! each a struct whose fields only its own module can reach:
//!
//! - [`document::Document`] — the graph, the undo ring, the edit serials and the clipboard.
//!   `apply` is its one door.
//! - [`link::SynthLink`] — the synth's host, the snapshot, the plan and its counters, the
//!   shaders, the probes and the workspace passes.
//! - [`midi::MidiDesk`] — learning, the monitor, the map as last sent, and the barriers.
//! - [`media::Media`] — assets and their pictures, the file dialog, the status line, and the
//!   offline render's bookkeeping.
//!
//! A part is handed what it reads that is not its own — the graph, the project, another
//! part — and never reaches for it. Where two parts must move together, as an edit that
//! republishes the graph and bars a MIDI write, it is `App` that says so, at one call site.
//! The modules that `impl App` are where those call sites live, split by what they are for:
//!
//! - [`autosave`] — the unsaved edits written into `.autosave/` off the frame thread.
//! - [`crashlog`] — the log file, the last run's notice and Help ▸ Report a problem….
//! - [`edit`] — `App::apply`, which hands what an applied command did to whoever owns it.
//! - [`files`] — dialogs, projects, assets, import and export.
//! - [`frame`] — the `eframe::App` body and what it paints.
//! - [`gpu`] — the GPU a start asks for: the environment over the preference over the rule.
//! - [`restart`] — Restart to apply: Quit's close, and the same executable started again.
//! - [`maininput`] — the left panel's view, and what a gesture on it sets.
//! - [`show`] — the pop-outs and the toast.
//! - [`undo`] — undo and redo by name: the toast that says what changed, and the history.
//!
//! A sibling reaching one of these is why a method here is `pub(super)`: it marks exactly
//! what crosses a seam, and the rest stays private to the module that owns it.

mod autosave;
pub mod crashlog;
mod document;
mod edit;
mod files;
mod frame;
pub mod gpu;
mod link;
mod maininput;
mod media;
mod midi;
mod onair;
pub use onair::OnAir;
mod problems;
mod record;
pub mod render;
pub mod restart;
mod show;
mod tabs;
mod undo;

use document::Document;
use files::FileAsk;
use frame::Meters;
use link::SynthLink;
use media::Media;
use midi::MidiDesk;
pub use show::Go;
use show::Show;

use crate::clock::Clock;
use crate::command::Command;
use crate::compile;
use crate::graph::{Graph, NodeId, PortRef};
use crate::mixer::{Channel, Mixer};
use crate::nodes::Frame;
use crate::preferences;
use crate::project::{self, Active, Project};
use crate::render::{FrameJob, Viewer};
use crate::synth::{Host, Msg, Snapshot};
use crate::ui::CanvasState;
use crate::ui::menu::MenuAction;
use crate::ui::project::{ProjectAction, ProjectPage, ProjectState};
use crate::ui::tabs::{TabAction, TabState};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// What the confirm is standing in front of. It asks before unsaved edits are lost, and
/// before the show going out is stopped: see [`OnAir`].
#[derive(Debug, Clone)]
pub(super) enum Pending {
    Quit,
    /// Quit, then start again: Preferences ▸ Performance's Restart to apply.
    Restart,
    OpenDialog,
    NewDialog,
    /// A project from the Recent list, or the command line.
    Open(std::path::PathBuf),
}

/// Where the first node from the toolbar lands, in canvas world units.
pub(super) const FIRST_DROP: egui::Pos2 = egui::Pos2::new(48.0, 48.0);

/// The name the mix is published over Syphon under; Syphon puts the app's name beside it.
const MIX_SERVER: &str = "Mix";
/// The name the mix is sent over NDI under; NDI puts only the machine's name beside it. No
/// Output may take it.
const MIX_STREAM: &str = crate::nodes::output::MIX_NAME;
/// Offset between successive nodes added from the toolbar, so they land side by side
/// instead of on top of one another. Wider than a node body plus a gap.
pub(super) const DROP_STAGGER: egui::Vec2 = egui::Vec2::new(216.0, 32.0);

pub struct App {
    /// The graph, its undo ring and the clipboard. [`Document::apply`] is the one door
    /// into it.
    doc: Document,
    /// The editor's end of the synth: everything sent to it and everything read back.
    link: SynthLink,
    /// The editor's half of MIDI.
    midi: MidiDesk,
    /// Assets, pictures, the file dialog, the status line and the render's bookkeeping.
    media: Media,
    /// The folder everything is saved in. A project always has one, so there is no "not
    /// saved anywhere yet" state and `Save` never has to ask where.
    project: Project,
    /// What is remembered between runs: the window, the Status box, the files
    /// opened. Never a command, so none of it enters the undo history.
    prefs: preferences::Store,
    /// How long the editor's own frames take, and the tick's as the editor reads it.
    meters: Meters,
    /// True when eframe handed us a device and the synth draws on it. False under
    /// egui_kittest, which hands us none; the output preview draws a placeholder instead of a
    /// paint callback.
    has_gpu: bool,
    /// The blit pipeline every picture in the editor is painted with, for eframe's target
    /// format. `None` when there is no device. Shared with each paint callback, which must be
    /// `Send + Sync`; a viewer is read-only, so nothing here is locked.
    viewer: Option<Arc<Viewer>>,
    /// The two timestamps around the editor's own painting, placed while the Status box is
    /// open. `None` with no device or where it writes no timestamps inside a pass.
    paint_timer: Option<Arc<crate::render::timing::PaintTimer>>,
    /// Who paints the ground under the panels and the canvas.
    ground: Ground,
    /// Pan, zoom and drag state for the workspace on screen. Not part of the graph.
    ///
    /// One canvas, not one per workspace: switching stashes the transform in the project's
    /// session and restores the one the tab was left at.
    canvas: CanvasState,
    /// The rename in progress on the tab bar, if one is.
    tabs: TabState,
    /// The Nodes menu across frames: which submenu is up, and the timers holding it there.
    start: crate::ui::start::StartMenu,
    /// The menu bar the operating system draws, once [`App::use_native_menu`] has asked for
    /// one and the machine has one: AppKit's on macOS. `None` draws the egui bar, which is
    /// Linux's, and every test's.
    native_menu: Option<crate::platform::menu::Bar>,
    /// What each entry on the native bar sends, at its tag, as of the menus last shown.
    native_actions: Vec<MenuAction>,
    /// The tab a node drag is over, as the bar reported it **this** frame. Cleared every
    /// frame, so a pointer that left the bar stops being a drop target at once.
    drop_target: Option<crate::graph::WorkspaceId>,
    /// What the project tab has open: a rename, a delete asking, a card being dragged.
    project_tab: ProjectState,
    /// The workspace the last `ImportWorkspace` landed under, for the caller to open. Not
    /// session state that outlives a frame: it is taken the moment it is read.
    imported: Option<crate::graph::WorkspaceId>,
    /// The four anchors every color in `ui/` derives from. One per app, so a re-theme is
    /// four numbers and nothing below has a default of its own to disagree with.
    theme: crate::ui::theme::Theme,
    /// Where the next node from the toolbar lands.
    next_drop: egui::Pos2,
    /// A switch to Linear found nodes outside the strip, and the offer to arrange them is
    /// on screen. Not a command: the question is app state, and the answer is the command.
    offer_arrange: bool,
    /// The unsaved-edits and on-air question, while it is on screen: what it stands in front
    /// of, and why the Save it offered did not save, once one has not.
    confirm: Option<(Pending, Option<String>)>,
    /// A Quit, an Open or a New the question let go on, held while the render it canceled
    /// ends. See [`App::after_render`].
    waiting: Option<Pending>,
    /// An autosave newer than the project's last save, found when it opened, while the
    /// question whether to recover it is on screen.
    recovery: Option<project::Recovered>,
    /// Whether the project on screen has a folder of its own, or is the scratch one in the
    /// system temp folder because the launch had nowhere to make `Untitled`.
    home: Home,
    /// **Test hook.** Every file dialog answered at once with this, rather than put up.
    dialog_answer: Option<Canned>,
    /// Project ▸ New project…'s or Save as…'s window while it is up: the name being typed.
    project_name_ask: Option<crate::ui::project_name::ProjectNameState>,
    /// The unsaved edits written into `.autosave/`, off the frame thread.
    autosave: autosave::Autosave,
    /// The window is closing and the confirm has been answered, so the close guard lets the
    /// next `close_requested` through.
    closing: bool,
    /// The window title as last sent, so it is sent again only when it changes.
    title: String,
    /// The performance surface: the toast, and what `H` and `F` hold.
    show: Show,
    /// The Preferences window while it is open, holding its tab and which swatch has its picker up.
    /// `None` is closed — the window's own openness, not a separate flag beside it.
    prefs_window: Option<crate::ui::prefs::PrefsState>,
    /// The Preferences tab showing when the window last closed, which it opens on again.
    prefs_tab: crate::ui::prefs::PrefsTab,
    /// Help ▸ About supersilvia and Help ▸ Licences…, whichever are open.
    help: crate::ui::about::HelpWindows,
    /// The last run's notice, and Help ▸ Report a problem…'s report while it is gathered.
    crashlog: crashlog::Desk,
    /// Edit ▸ Undo History… while it is open. `None` is closed.
    undo_history: Option<crate::ui::history::HistoryState>,
    /// Every adapter the machine offered and the one the editor and the synth draw on, for the
    /// Preferences window. `None` where the host handed over a device it chose itself.
    gpu_choice: Option<crate::render::adapter::Choice>,
    /// Preferences ▸ Performance ▸ GPU as this run started on it, which Restart to apply is
    /// offered against.
    gpu_started: gpu::Setting,
    /// Whether the app starts again once it has closed: asked by Restart to apply, read by
    /// `main`.
    restart: restart::Restart,
    /// The canvas in physical pixels, as of the last frame, which is what a
    /// viewport-matched mix is sized to. Kept from the last frame that had a canvas, so
    /// the project tab does not shrink the mix to nothing.
    mix_viewport: (u32, u32),
    /// The canvas size seen last and the time it was first seen, which `mix_viewport` takes
    /// once it has held for `frame::MIX_SETTLE_S`. `None` until a frame has had a canvas.
    mix_canvas: Option<((u32, u32), f64)>,
    /// The connected display with the fewest pixels, its physical size, which is what a
    /// display-matched mix — the default — is sized to. As the pictures thread last reported
    /// the displays; `None` until it has, and in a run with no window system.
    mix_display: Option<(u32, u32)>,
    /// Which pictures should have windows of their own, and what those windows are doing.
    ///
    /// Session state: which picture is on which screen is decided on the night, so it is not
    /// saved and it is not undoable. The rule is [`crate::render::picture::Wall`]'s.
    wall: crate::render::picture::Wall,
    /// The pictures thread, which owns the windows themselves. See
    /// [`crate::render::picture`].
    pictures: crate::render::picture::Host,
    /// Files dragged onto the window where winit does not hear them, on Wayland, and the
    /// files held over it now. See [`App::feed_file_drags`].
    filedrop: crate::platform::filedrop::FileDrop,
    held: Option<Vec<std::path::PathBuf>>,
    /// Syphon and NDI, from the editor's side.
    sending: Sending,
    /// Which of the Main Input panel's two pickers is up: the clip's or the sound's.
    /// Session state, like the popup open on the canvas.
    main_input_picker: Option<bool>,
}

/// Where the project on screen lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Home {
    /// A folder of its own, which Save writes.
    Folder,
    /// The scratch project in the system temp folder, the launch having had nowhere to make
    /// `Untitled`: a Save asks for a folder instead, so nothing is ever saved into the temp
    /// folder.
    Scratch,
}

/// A file dialog's answer, given by a test rather than a person: a folder, or `None` for a
/// dialog closed.
#[derive(Debug, Clone)]
struct Canned(Option<std::path::PathBuf>);

/// Syphon and NDI from the editor's side: the publisher, and which ways the mix leaves.
struct Sending {
    /// Every Output sent out over Syphon or NDI, and the mix while a mark is lit. See
    /// [`crate::render::publish`].
    publisher: crate::render::publish::Publisher,
    /// The mix's two marks. Session state, as the mixer is: not saved.
    mix: MixOut,
}

/// Which ways the mix is sent out.
#[derive(Debug, Clone, Copy, Default)]
struct MixOut {
    /// Published over Syphon.
    syphon: bool,
    /// Sent over NDI.
    ndi: bool,
}

/// Who paints the ground every panel and the canvas stand on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ground {
    /// Each panel and the canvas paint their own, over a clear of any color: egui_kittest's
    /// renderer, which clears to its own.
    Painted,
    /// eframe clears the window to [`crate::ui::theme::clear_color`] before each frame, so no
    /// panel and no canvas paints it again. Wherever eframe hands the app a GPU.
    Cleared,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, prefs: preferences::Store) -> Self {
        // The editor's GPU, the one device eframe paints through; `None` under a harness
        // that hands the app none, which is egui_kittest.
        let gpu = crate::render::Gpu::for_eframe(cc);
        // The synth's is the same device and its one queue (`Gpu::for_synth`), handed to the
        // synth thread, which makes its renderer on it. Without it there is no renderer and
        // no picture, which is the state a machine with no GPU was already in.
        let synth_gpu = gpu.as_ref().map(crate::render::Gpu::for_synth);
        // **A thread only where there is a GPU to draw on.** With none there is nothing for
        // a thread to do that the frame cannot do inline, and a test harness that drives
        // frames by hand must have the tick land inside the frame it drove — egui_kittest is
        // exactly that. One `Synth::step`, two hosts. See [`crate::synth::thread`].
        let drawing = synth_gpu.is_some();
        let synth = match synth_gpu {
            Some(synth_gpu) => Host::spawn(Some(synth_gpu)),
            None => Host::inline(),
        };
        // The pictures thread, on eframe's own `wl_display`, blitting on the same device.
        // Wayland only: see `render::picture`.
        let pictures = crate::render::picture::Host::for_eframe(
            cc,
            gpu.as_ref(),
            synth.live(),
            synth.pointer(),
        );
        // Wayland's file drops, which winit does not hear, on eframe's display too. Not under
        // a harness, which has no window to drag onto.
        let filedrop = if cc.wgpu_render_state.is_some() {
            let ctx = cc.egui_ctx.clone();
            crate::platform::filedrop::FileDrop::for_window(cc, move || ctx.request_repaint())
        } else {
            crate::platform::filedrop::FileDrop::none()
        };
        // The publisher reads the same `Live` on a thread of its own, started the first time
        // anything is sent out, over Syphon or NDI.
        let publisher = crate::render::publish::Publisher::new(gpu.as_ref(), synth.live());
        let viewer = gpu
            .as_ref()
            .and_then(|gpu| match Viewer::for_eframe(cc, gpu) {
                Ok(v) => Some(Arc::new(v)),
                Err(err) => {
                    log::error!("could not create the viewer: {err}");
                    None
                }
            });
        let paint_timer = gpu
            .as_ref()
            .and_then(crate::render::timing::PaintTimer::new)
            .map(Arc::new);
        // The person's four numbers, not the built-in default: the editor opens wearing
        // whatever it was last left wearing — and at the scale it was left at.
        let theme = prefs.get().theme;
        let gpu_started = gpu::Setting::of(prefs.get());
        cc.egui_ctx.set_zoom_factor(prefs.get().scale());
        // The zoom keys are the canvas's, read with the other shortcuts: egui's own would scale
        // the whole editor, which is the interface size's alone.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let ground = if gpu.is_some() {
            Ground::Cleared
        } else {
            Ground::Painted
        };
        if ground == Ground::Cleared {
            crate::ui::theme::apply_cleared(&cc.egui_ctx, &theme);
        } else {
            crate::ui::theme::apply(&cc.egui_ctx, &theme);
        }
        // The one place a sequencer is opened. A box with none is a box with no MIDI, which
        // is a line in the log and a window that says so — not a failure to start.
        let (midi, midi_rx) = match crate::midi::Midi::open() {
            Ok((midi, rx)) => (Some(midi), Some(rx)),
            Err(err) => {
                log::warn!("midi: {err}");
                (None, None)
            }
        };
        let app = Self {
            // A device eframe handed over *and* the synth drawing on it: without the second
            // there is nothing drawing, and an Output row that said `Rendering` over a black
            // screen is the worst of the two ways to be wrong.
            has_gpu: gpu.is_some() && drawing,
            link: SynthLink::new(synth),
            pictures,
            filedrop,
            held: None,
            sending: Sending {
                publisher,
                mix: MixOut::default(),
            },
            viewer,
            paint_timer,
            ground,
            theme,
            prefs,
            gpu_started,
            midi: MidiDesk::new(midi),
            ..Self::headless()
        };
        // Handed to the synth to drain at the top of its ticks.
        if let Some(rx) = midi_rx {
            app.link.send(Msg::MidiIn(rx));
        }
        // **A lost device ends the run, with the work saved.** The editor paints on the same
        // device, so nothing is left to draw a question with: the newest unsaved document is
        // written into `.autosave/`, the line is said on stderr and in the log, written as the
        // reason the next launch gives, and the process exits, from whichever thread wgpu
        // reported it on.
        if let Some(gpu) = &gpu {
            let shared = app.autosave.shared();
            gpu.device()
                .set_device_lost_callback(move |reason, message| {
                    let report = autosave::device_lost(&shared, &format!("{reason:?}: {message}"));
                    log::error!("{report}");
                    eprintln!("supersilvia: {report}");
                    crashlog::why(&report);
                    std::process::exit(1);
                });
        }
        app
    }

    /// An app with no GPU, for tests and for egui_kittest, which hands the app none.
    /// `render/` is simply absent in this state and the output preview draws a placeholder.
    ///
    /// Its project is a scratch folder under the system temp directory and its preferences
    /// are in memory, so nothing a test does reaches the real ones.
    pub fn headless() -> Self {
        let mut app = Self {
            doc: Document::default(),
            link: SynthLink::new(Host::inline()),
            // Headless: no sequencer is opened, so no thread is started and no device is
            // touched by `cargo test`. The map and the applier are reachable without one.
            midi: MidiDesk::default(),
            media: Media::default(),
            project: Project::scratch(),
            prefs: preferences::Store::in_memory(),
            meters: Meters::default(),
            has_gpu: false,
            viewer: None,
            paint_timer: None,
            ground: Ground::Painted,
            canvas: CanvasState::default(),
            tabs: TabState::default(),
            start: crate::ui::start::StartMenu::default(),
            native_menu: None,
            native_actions: Vec::new(),
            drop_target: None,
            project_tab: ProjectState::default(),
            imported: None,
            theme: crate::ui::theme::Theme::default(),
            next_drop: FIRST_DROP,
            offer_arrange: false,
            confirm: None,
            waiting: None,
            recovery: None,
            project_name_ask: None,
            home: Home::Folder,
            dialog_answer: None,
            autosave: autosave::Autosave::default(),
            closing: false,
            title: String::new(),
            show: Show::default(),
            prefs_window: None,
            prefs_tab: crate::ui::prefs::PrefsTab::default(),
            help: crate::ui::about::HelpWindows::default(),
            crashlog: crashlog::Desk::default(),
            undo_history: None,
            gpu_choice: None,
            gpu_started: gpu::Setting::default(),
            restart: restart::Restart::default(),
            mix_viewport: crate::nodes::output::DEFAULT_RESOLUTION,
            mix_canvas: None,
            mix_display: None,
            wall: crate::render::picture::Wall::default(),
            pictures: crate::render::picture::Host::detached(),
            filedrop: crate::platform::filedrop::FileDrop::none(),
            held: None,
            sending: Sending {
                publisher: crate::render::publish::Publisher::detached(),
                mix: MixOut::default(),
            },
            main_input_picker: None,
        };
        // A project always shows a tab, and a scratch one has never recorded a session.
        app.project.reconcile(app.doc.graph());
        app.media.refresh_assets(&app.project);
        app.publish_graph();
        app
    }

    /// Hand the synth the graph to tick over: the one place it crosses. See
    /// [`SynthLink::publish_graph`].
    pub(super) fn publish_graph(&mut self) {
        self.link.publish_graph(self.doc.shared());
    }

    /// Hand the synth the MIDI map's live bindings, where they differ from what it already
    /// has: the one place the map crosses. See [`MidiDesk::publish`].
    pub(super) fn publish_midi(&mut self) {
        self.midi
            .publish(self.project.midi(), self.doc.graph(), &self.link);
    }

    // The tick's own state is the synth's, and what the editor knows of it is the snapshot
    // the last tick left: each of these is a read of that, kept because a test and the
    // canvas both ask the app.

    /// What the synth's last tick left. The whole of what the editor knows about running
    /// state.
    pub fn snapshot(&self) -> &Snapshot {
        self.link.snapshot()
    }

    /// See [`Snapshot::uniform`].
    pub fn uniform(&self, port: PortRef) -> Option<f64> {
        self.link.snapshot().uniform(port)
    }

    /// What one node's tick says about itself this frame: its
    /// [`CpuNode::status`](crate::nodes::CpuNode::status), which is what a status line on
    /// that node's body draws. `None` where the node has nothing to say, or where nothing is
    /// asking for a report.
    pub fn node_status(&self, node: NodeId) -> Option<&str> {
        self.link
            .snapshot()
            .notes
            .get(&node)
            .map(|n| n.text.as_str())
    }

    /// See [`Snapshot::uniform_color`].
    pub fn uniform_color(&self, port: PortRef) -> Option<[f32; 4]> {
        self.link.snapshot().uniform_color(port)
    }

    /// Where the pointer is over one surface, as that surface's own source reports it.
    ///
    /// The seam all three write through — a picture window from the pictures thread, the
    /// preview and the canvas from the frame, and a test feeding a synthetic pointer — and
    /// the one the tick reads. See [`crate::pointer`].
    pub fn set_pointer(
        &self,
        surface: crate::pointer::Surface,
        reading: Option<crate::pointer::Reading>,
    ) {
        self.link.host().pointer().set(surface, reading);
    }

    /// See [`Snapshot::edges`].
    pub fn edges(&self, port: PortRef) -> &[crate::nodes::Event] {
        self.link.snapshot().edges(port)
    }

    /// Hold or release the button on an action input.
    ///
    /// The seam a test and the accessibility tree both press through. Not a command: a hand
    /// on a button plays the graph rather than editing it, so nothing enters the history.
    /// The snapshot's own copy is set here too, so a test that presses and asks in the same
    /// breath reads what it just did rather than what the last tick saw.
    pub fn press(&mut self, port: PortRef, down: bool) {
        self.link.press(port, down);
    }

    /// See [`Snapshot::is_held`], plus a press this frame that no tick has answered yet.
    pub fn is_held(&self, port: PortRef) -> bool {
        self.link.is_held(port)
    }

    /// The buttons the pointer is on, as the canvas reports them. Replaced every frame.
    pub(super) fn set_pointer_held(&mut self, held: HashSet<PortRef>) {
        self.link.send(Msg::PointerHeld(held));
    }

    /// See [`Snapshot::frame`].
    pub fn frame(&self, port: PortRef) -> Option<&Arc<Frame>> {
        self.link.snapshot().frame(port)
    }

    /// See [`Snapshot::trace`].
    pub fn trace(&self, node: NodeId) -> Option<&crate::nodes::cpu::TraceRing> {
        self.link.snapshot().trace(node)
    }

    /// See [`Snapshot::puck`].
    pub fn puck(&self, node: NodeId) -> Option<&crate::nodes::cpu::Puck> {
        self.link.snapshot().puck(node)
    }

    /// What a hand did on a node's own surface, for the next tick: the seam the canvas's own
    /// pad crosses, which a test drops a well or starts a preset through. Not a command, for
    /// the reason a press is not one.
    pub fn touch(&mut self, node: NodeId, touch: crate::nodes::cpu::Touch) {
        self.link.send(Msg::Touches(vec![(node, touch)]));
    }

    pub fn graph(&self) -> &Graph {
        self.doc.graph()
    }

    /// **Test accessor.** How many graphs have been handed to the synth. The same number
    /// over a frame is a frame that crossed nothing.
    #[doc(hidden)]
    pub fn graph_generation(&self) -> u64 {
        self.link.graph_generation()
    }

    /// The clock, as it stands. See [`SynthLink::clock`].
    pub fn clock(&self) -> crate::synth::ClockReport {
        self.link.clock()
    }

    /// A hand on the transport: pause, play or a seek — the time readout's reset is a seek to
    /// zero.
    ///
    /// Not a command, and never an edit: it moves the show rather than the patch, as a deck
    /// claim does, and nothing of it is saved. The synth takes it at the top of its next tick.
    pub fn transport(&mut self, command: crate::transport::Command) {
        self.link.transport(command);
    }

    /// Pause a playing show, or play a paused one: `F8` and the readout's button.
    pub fn toggle_pause(&mut self) {
        let playing = self.transport_state().playing;
        self.transport(if playing {
            crate::transport::Command::Pause
        } else {
            crate::transport::Command::Play
        });
    }

    /// The transport as the last tick left it: where the playhead is and whether it plays.
    pub fn transport_state(&self) -> crate::transport::Report {
        self.link.transport_report()
    }

    /// **Inline only, and a test's.** One tick of the synth with the editor not running:
    /// nothing is taken off the mailbox and nothing is reconciled, which is what a minimized
    /// editor looks like from here. `App::tick` is this and everything the editor does
    /// around it.
    #[doc(hidden)]
    pub fn step_synth(&mut self, dt: f32) {
        self.link.host_mut().step(crate::synth::Beat::Delta(dt));
    }

    /// **Inline only, and a test's.** The synth itself, for the two things only it knows:
    /// what its own copy of the graph says a control is, and which Output is on each deck of
    /// the mix it is rendering.
    #[doc(hidden)]
    pub fn ticked(&self) -> Option<&crate::synth::Synth> {
        self.link.host().inline_synth()
    }

    /// **Inline only, and a test's.** The simulation a node last published on one of its
    /// ports — its shape, its numbers and the steps it has taken. See
    /// [`crate::synth::Synth::simulation`].
    #[doc(hidden)]
    pub fn simulation(&self, port: PortRef) -> Option<&crate::nodes::Simulation> {
        self.link.host().inline_synth()?.simulation(port)
    }

    /// **Inline only, and a test's.** What a simulation holds on the GPU, read back and
    /// waited for. `None` without a renderer.
    #[doc(hidden)]
    pub fn read_simulation(&mut self, port: PortRef) -> Option<crate::render::SimReadback> {
        self.link
            .host_mut()
            .inline_synth_mut()?
            .read_simulation(port)
    }

    /// **Inline only, and a test's.** A port's thumbnail as though its pass had drawn it,
    /// which is how a test with no renderer shows the editor a port's picture. See
    /// [`crate::synth::Synth::put_thumb`].
    #[doc(hidden)]
    pub fn put_thumb(&mut self, port: PortRef, thumb: crate::synth::PortThumb) {
        if let Some(synth) = self.link.host_mut().inline_synth_mut() {
            synth.put_thumb(port, thumb);
        }
    }

    /// **Inline only, and a test's.** Leave this Output's link in flight unfinished until
    /// let go.
    #[doc(hidden)]
    pub fn hold_link(&mut self, node: NodeId, hold: bool) {
        if let Some(synth) = self.link.host_mut().inline_synth_mut() {
            synth.hold_link(node, hold);
        }
    }

    /// **Inline only, and a test's.** An Output's latest frame as RGBA8, rows bottom first,
    /// read back and waited for. `None` without a renderer.
    #[doc(hidden)]
    pub fn read_output(&self, node: NodeId) -> Option<(u32, u32, Vec<u8>)> {
        self.link.host().inline_synth()?.read_output(node)
    }

    /// **Test accessor.** Why the last plan draws this Output on every tick, or `None` where
    /// it is idle — and where it is suspended, unplugged or not in the plan at all.
    #[doc(hidden)]
    pub fn draws(&self, node: NodeId) -> Option<crate::synth::Why> {
        self.link
            .plan()
            .outputs
            .iter()
            .find(|o| o.node == node)
            .and_then(|o| o.mode.why())
    }

    /// **Inline only.** The Outputs the synth draws on its next tick: the plan's, and what it
    /// has been asked a picture of since. Empty under the synth thread.
    #[doc(hidden)]
    pub fn synth_drawing(&self) -> std::collections::HashSet<NodeId> {
        self.link
            .host()
            .inline_synth()
            .map(crate::synth::Synth::drawing)
            .unwrap_or_default()
    }

    /// **Inline only, and a test's.** The job the synth would draw now, from the plan it
    /// holds, with no plan built first: what a tick draws before the editor replans.
    #[doc(hidden)]
    pub fn synth_job(&mut self) -> FrameJob {
        self.link
            .host_mut()
            .inline_synth_mut()
            .map(crate::synth::Synth::job)
            .unwrap_or_default()
    }

    /// **Inline only.** The clock itself, for a test and for the stepper that drives it.
    /// `None` under the synth thread, where the clock is the synth's alone.
    pub fn clock_mut(&mut self) -> &mut Clock {
        self.link
            .host_mut()
            .inline_synth_mut()
            .expect("the clock is only reachable directly while the synth is inline")
            .clock_mut()
    }

    /// The command each undo step carries, oldest first. See `Document::history` for what it
    /// is and what it is not.
    pub fn history(&self) -> Vec<Command> {
        self.doc.history()
    }

    /// How many nodes the canvas drew last frame; the rest were off screen.
    /// Pan and zoom, for a test that wants to know what the wheel did.
    pub fn canvas_transform(&self) -> crate::ui::canvas::Transform {
        self.canvas.transform()
    }

    pub fn canvas_drawn(&self) -> usize {
        self.canvas.drawn()
    }

    /// Height of the canvas area last frame.
    pub fn canvas_height(&self) -> f32 {
        self.canvas.height()
    }

    /// Width of it, which with the height is what a centerd view is measured against.
    pub fn canvas_width(&self) -> f32 {
        self.canvas.width()
    }

    /// Which step of the golden-angle walk a cable wears, under `phi_cables`.
    ///
    /// Session state like the pan: it is a display preference's working, not the document's.
    pub fn cable_hue(
        &self,
        from: NodeId,
        from_key: &'static str,
        to: NodeId,
        to_key: &'static str,
    ) -> Option<u32> {
        self.canvas.cable_hue(
            crate::graph::PortRef::new(from, from_key),
            crate::graph::PortRef::new(to, to_key),
        )
    }

    /// The strip the view can actually reach: the rectangle the pan is clamped to and the
    /// minimap is drawn from, which eases toward the strip the graph makes rather than
    /// being it. How much room a drag at the far edge has made, and how much of it has been
    /// given back, are both read here.
    pub fn strip_reach(&self) -> Option<emath::Rect> {
        self.canvas.bounds()
    }

    /// Top-left of the canvas area last frame, for turning world into screen.
    pub fn canvas_origin(&self) -> emath::Pos2 {
        self.canvas.origin()
    }

    /// The selected nodes, in id order. Session state, so it is not in the file.
    pub fn canvas_selection(&self) -> Vec<NodeId> {
        self.canvas.selected().iter().copied().collect()
    }

    /// The selection, narrowed to the workspace being looked at.
    ///
    /// What every verb that acts on a selection is handed. The canvas holds one selection
    /// across the project, so a node selected on another workspace would otherwise be
    /// deleted, copied or cut by a gesture naming a node nobody can see.
    pub fn workspace_selection(&self) -> Vec<NodeId> {
        let shown = self.active_workspace();
        self.canvas
            .selected()
            .iter()
            .copied()
            .filter(|id| {
                shown.is_some_and(|w| {
                    self.doc
                        .graph()
                        .get(*id)
                        .is_some_and(|n| n.workspaces.contains(&w))
                })
            })
            .collect()
    }

    /// Replace the selection — for a test, and for a keyboard shortcut that acts on one.
    pub fn select_only(&mut self, ids: &[NodeId]) {
        self.canvas.select_only(ids);
    }

    // ---------------------------------------------------------------- tabs

    /// Which tab is showing.
    pub fn active(&self) -> Active {
        self.project.session().active
    }

    /// The workspace whose canvas is showing, if a workspace is showing at all.
    pub fn active_workspace(&self) -> Option<crate::graph::WorkspaceId> {
        self.project.session().active_workspace()
    }

    /// Every workspace with a tab, in id order.
    pub fn open_workspaces(&self) -> &std::collections::BTreeSet<crate::graph::WorkspaceId> {
        &self.project.session().open
    }

    /// Where a new node lands: the active workspace, or the first open one while the project
    /// tab is showing.
    fn landing_workspace(&self) -> crate::graph::WorkspaceId {
        self.active_workspace()
            .or_else(|| self.open_workspaces().iter().copied().next())
            .unwrap_or_else(|| self.doc.graph().default_workspace())
    }

    /// Put the view on screen back where the session keeps it, before the tab changes.
    fn stash_view(&mut self) {
        let Some(id) = self.active_workspace() else {
            return;
        };
        let view = self.canvas.saved_view();
        self.project.session_mut().views.insert(id, view);
    }

    /// Show a tab. **Not a command**: switching is session state, like a pan, so it never
    /// enters the undo history.
    ///
    /// The view follows the tab — the one you left comes back — and the selection does not:
    /// a node not on the workspace on screen is never selected, so switching clears it.
    pub fn activate(&mut self, to: Active) {
        if self.project.session().active == to {
            return;
        }
        self.stash_view();
        self.project.session_mut().active = to;
        self.canvas.restore_view(match to {
            Active::Workspace(id) => self
                .project
                .session()
                .views
                .get(&id)
                .copied()
                .unwrap_or_default(),
            Active::Project => crate::project::View::default(),
        });
        self.canvas.clear_selection();
    }

    /// Give a workspace a tab and show it.
    pub fn open_workspace(&mut self, id: crate::graph::WorkspaceId) {
        if !self.doc.graph().has_workspace(id) {
            return;
        }
        self.project.session_mut().open.insert(id);
        self.activate(Active::Workspace(id));
    }

    /// Take a workspace's tab away. Its nodes stay in the graph and stop running.
    ///
    /// Nothing is lost by closing, which is why it needs no confirm: a closed workspace is
    /// still in the project, and Save writes it whether or not it has a tab.
    pub fn close_workspace(&mut self, id: crate::graph::WorkspaceId) {
        self.stash_view();
        self.project.session_mut().open.remove(&id);
        if self.active() == Active::Workspace(id) {
            // The neighbor, so closing the middle of a row does not throw you to the front.
            let next = self
                .doc
                .graph()
                .workspaces()
                .iter()
                .map(|w| w.id)
                .find(|w| self.open_workspaces().contains(w));
            self.activate(next.map_or(Active::Project, Active::Workspace));
        }
    }

    /// Answer one thing the tab bar asked for.
    // A message answered once, spent here.
    #[allow(clippy::needless_pass_by_value)]
    fn handle_tab(&mut self, action: TabAction) {
        match action {
            TabAction::Activate(to) => self.activate(to),
            TabAction::Add(kind) => self.add_workspace(kind),
            TabAction::Close(id) => self.close_workspace(id),
            TabAction::Duplicate(id) => self.duplicate_workspace(id),
            TabAction::Rename { id, name } => {
                let _ = self.apply(Command::RenameWorkspace { id, name });
            }
            TabAction::DropTarget(id) => self.drop_target = Some(id),
        }
    }

    /// Where a link to this node goes: [`crate::graph::Graph::home_of`], against the tab
    /// showing and the tabs open.
    fn home_of(&self, node: NodeId) -> Option<crate::graph::WorkspaceId> {
        self.doc
            .graph()
            .home_of(node, self.active_workspace(), &self.project.session().open)
    }

    /// Show the workspace a tag names, and put its view on the node the cable came from.
    ///
    /// **Not a command.** Opening a tab, switching to it and panning are all session state,
    /// so following a tag never enters the undo history.
    fn navigate_to(&mut self, workspace: crate::graph::WorkspaceId, node: NodeId) {
        self.open_workspace(workspace);
        let graph = self.doc.graph();
        if let Some(at) = crate::ui::canvas::Layouts::one(graph, node)
            .find(node)
            .map(|l| l.rect.center())
        {
            self.canvas.center_on(at);
            // Centring alone lands the view on a node that looks like every other node
            // around it. The throb is the answer to the click.
            self.canvas.throb_on(node);
        }
    }

    /// Answer a node drag the canvas reported, against the tab the bar reported under it.
    ///
    /// Dropping onto the tab of a workspace the nodes are already on does nothing, rather
    /// than pushing an undo step that changes nothing.
    fn handle_node_drag(&mut self, drag: Option<crate::ui::NodeDrag>) {
        let Some(drag) = drag.filter(|d| d.released) else {
            return;
        };
        let Some(workspace) = self.drop_target else {
            return;
        };
        let nodes: Vec<NodeId> = drag
            .nodes
            .into_iter()
            .filter(|id| {
                self.doc
                    .graph()
                    .get(*id)
                    .is_some_and(|n| !n.workspaces.contains(&workspace))
            })
            .collect();
        if !nodes.is_empty() {
            let _ = self.apply(Command::ShowOn { nodes, workspace });
        }
    }

    /// The project tab, which replaces the canvas while it is showing.
    fn show_project_tab(&mut self, ui: &mut egui::Ui) {
        let counts = |id: crate::graph::WorkspaceId| self.doc.graph().on_workspace(id).count();
        let name = self.project.name();
        let assets = self.asset_cards();
        let page = ProjectPage {
            name: &name,
            workspaces: self.doc.graph().workspaces(),
            open: &self.project.session().open,
            counts: &counts,
            thumbnails: self.media.thumbnails(),
            assets: &assets,
        };
        let actions = crate::ui::project::show(ui, &mut self.project_tab, &page, &self.theme);
        drop(assets);
        for action in actions {
            self.handle_project_tab(action);
        }
    }

    /// Every file in `assets/`, with who is using it and a picture where there is one.
    ///
    /// Usage is `project::asset_users`, which walks every node's `Asset` options: an asset
    /// reference *is* an option, and that is the honest question to ask.
    fn asset_cards(&self) -> Vec<crate::ui::project::AssetCard> {
        let users = project::asset_users(self.doc.graph());
        self.project
            .assets()
            .into_iter()
            .map(|asset| {
                let using = users
                    .get(&asset.reference)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                crate::ui::project::AssetCard {
                    users: using
                        .iter()
                        .filter_map(|(node, key)| self.asset_user(*node, key))
                        .collect(),
                    thumbnail: self
                        .media
                        .asset_thumbnails()
                        .get(&asset.reference)
                        .cloned()
                        .flatten(),
                    icon: crate::ui::project::icon_for(&asset.name),
                    name: asset.name,
                    reference: asset.reference,
                    size: asset.size,
                }
            })
            .collect()
    }

    /// One node that references an asset, named the way a cross-workspace tag is, and
    /// pointed at the workspace clicking it would go to.
    fn asset_user(&self, node: NodeId, key: &str) -> Option<crate::ui::project::AssetUser> {
        let n = self.doc.graph().get(node)?;
        let workspace = self.home_of(node)?;
        let where_ = self.doc.graph().workspace(workspace)?;
        Some(crate::ui::project::AssetUser {
            node,
            workspace,
            label: format!("{}{node}.{key} on {}", n.def.slug, where_.name),
        })
    }

    /// Answer one thing the project tab asked for.
    #[allow(clippy::needless_pass_by_value)]
    fn handle_project_tab(&mut self, action: ProjectAction) {
        match action {
            ProjectAction::Open(id) => self.open_workspace(id),
            ProjectAction::Close(id) => self.close_workspace(id),
            ProjectAction::Add(kind) => self.add_workspace(kind),
            ProjectAction::Rename { id, name } => {
                let _ = self.apply(Command::RenameWorkspace { id, name });
            }
            ProjectAction::SetBlurb { id, blurb } => {
                let _ = self.apply(Command::SetBlurb { id, blurb });
            }
            ProjectAction::Delete(id) => {
                let _ = self.apply(Command::RemoveWorkspace(id));
            }
            ProjectAction::Move { id, to } => {
                let _ = self.apply(Command::MoveWorkspace { id, to });
            }
            ProjectAction::Duplicate(id) => self.duplicate_workspace(id),
            ProjectAction::Export(id) => self.ask_for_file(FileAsk::ExportWorkspace(id)),
            ProjectAction::ImportWorkspace => self.ask_for_file(FileAsk::ImportWorkspace),
            ProjectAction::ImportAsset => self.ask_for_file(FileAsk::ImportAsset),
            ProjectAction::ExportAsset(reference) => {
                self.ask_for_file(FileAsk::ExportAsset(reference));
            }
            ProjectAction::RevealAsset(reference) => self.reveal_asset(&reference),
            ProjectAction::RemoveAsset(reference) => self.remove_asset(&reference),
            ProjectAction::Navigate { workspace, node } => self.navigate_to(workspace, node),
        }
    }

    /// Add a workspace, open it and show it. The edit goes through the bus; opening and
    /// showing do not, because neither is one.
    pub fn add_workspace(&mut self, kind: crate::graph::WorkspaceKind) {
        // **Count up until the name is free.** How many workspaces there are is the number
        // to *start* from and not the answer: close Workspace 2 of three and the count says
        // three, which is a tab that already exists. Renaming one to `Workspace 7` sets the
        // same trap further out. Two tabs with one name is a name that names neither, and
        // the one thing a person does with a tab is say which one they mean.
        //
        // Counting up from the count rather than searching from one: the count is where a
        // new tab naturally belongs, and the search exists only to get out of the way of a
        // name that is taken. So this is not a promise that the number is higher than every
        // other tab's — rename one to `Workspace 9` and the next is still `Workspace 4` if
        // that is free. The rule is about collisions and nothing else.
        //
        // The name is chosen here and travels inside the command, so redo puts back the
        // name that was given rather than recomputing one against a graph that has moved on.
        let name = {
            let taken: std::collections::HashSet<&str> = self
                .doc
                .graph()
                .workspaces()
                .iter()
                .map(|w| w.name.as_str())
                .collect();
            let mut n = self.doc.graph().workspaces().len() + 1;
            while taken.contains(format!("Workspace {n}").as_str()) {
                n += 1;
            }
            format!("Workspace {n}")
        };
        let layout = self.prefs.get().default_layout;
        // A video tab is born with the shortest patch that shows a picture: the Main Input on
        // the left, an Output on the right, cabled. An empty plane is a worse first thing to
        // meet than two nodes you can delete. The canvas as of last frame is what the Output
        // is placed against, and it rides in the command so redo puts it back there.
        let seed = match kind {
            crate::graph::WorkspaceKind::Video => crate::command::Seed::SourceToOutput {
                width: self.canvas.width(),
            },
            #[allow(unreachable_patterns)]
            _ => crate::command::Seed::Empty,
        };
        if self
            .apply(Command::AddWorkspace {
                name,
                kind,
                layout,
                seed,
            })
            .is_err()
        {
            return;
        }
        let Some(added) = self.doc.graph().workspaces().last().map(|w| w.id) else {
            return;
        };
        self.open_workspace(added);
    }

    /// Every node shown on a workspace that is open, and everything feeding a deck, worked
    /// out once per change to what it depends on. See [`SynthLink::live`].
    fn live_nodes(&mut self) -> HashSet<NodeId> {
        self.with_link(|link, session| link.live(session).clone())
    }

    /// **Test accessor.** The nodes one workspace's pass measures, in slot order across its
    /// batches, as the last plan built it: empty for a workspace with no pass.
    #[doc(hidden)]
    pub fn pass_measures(&self, workspace: crate::graph::WorkspaceId) -> Vec<NodeId> {
        self.link
            .plan()
            .passes
            .iter()
            .filter(|p| p.key.workspace == workspace)
            .flat_map(|p| p.tap_nodes.iter().copied())
            .collect()
    }

    /// Hand the link the session as the editor stands, with the link itself: every call that
    /// reads the plan's inputs comes through here.
    fn with_link<R>(&mut self, f: impl FnOnce(&mut SynthLink, &link::Session<'_>) -> R) -> R {
        let seen = self.seen();
        let session = session(
            &self.doc,
            &self.project,
            &self.prefs,
            self.mix_viewport,
            self.mix_display,
            seen,
        );
        f(&mut self.link, &session)
    }

    /// The decks, the fade, the method and the projection.
    pub fn mixer(&self) -> &Mixer {
        self.project.mixer()
    }

    /// The mixer as the panel draws it: each deck with the name of the workspace its link
    /// goes to.
    fn mixer_view(&self) -> crate::ui::mixer::MixerView<'_> {
        let channel = |id: Option<NodeId>| {
            let id = id?;
            self.doc.graph().get(id)?;
            let workspace = self
                .home_of(id)
                .and_then(|w| Some((w, self.doc.graph().workspace(w)?.name.as_str())));
            Some(crate::ui::mixer::ChannelView {
                node: id,
                workspace,
            })
        };
        let m = self.project.mixer();
        crate::ui::mixer::MixerView {
            a: channel(m.a),
            b: channel(m.b),
            balance: m.balance,
            bound: self
                .project
                .midi()
                .trigger_of(crate::midi::Target::Balance)
                .map(crate::midi::Trigger::label),
            learning: self.midi.learning() == Some(crate::midi::Target::Balance),
            blackout: crate::ui::mixer::SwitchView {
                on: m.blackout,
                bound: self
                    .project
                    .midi()
                    .trigger_of(crate::midi::Target::Blackout)
                    .map(crate::midi::Trigger::label),
                learning: self.midi.learning() == Some(crate::midi::Target::Blackout),
            },
            freeze: crate::ui::mixer::SwitchView {
                on: m.freeze,
                bound: self
                    .project
                    .midi()
                    .trigger_of(crate::midi::Target::Freeze)
                    .map(crate::midi::Trigger::label),
                learning: self.midi.learning() == Some(crate::midi::Target::Freeze),
            },
            ghost: self.midi_ghost(crate::midi::Target::Balance),
            method: m.method,
            resolution: m.resolution,
            display: self.mix_display,
            popped: self
                .wall
                .open()
                .iter()
                .find(|p| p.picture == crate::ui::PopOut::Mix)
                .map(|p| p.fullscreen),
            no_windows: self.pictures.why_not(),
            background: m.background,
            syphon: crate::platform::syphon::available().then_some(self.sending.mix.syphon),
            ndi: crate::video::ndi::missing().map_or(Ok(self.sending.mix.ndi), Err),
            collapsed: self.prefs.get().mixer_collapsed,
        }
    }

    /// Whether `H` has hidden the canvas.
    pub fn editor_hidden(&self) -> bool {
        self.show.hidden
    }

    /// Put an Output on a deck, as `Show on A` and `Show on B` do. Not a command: it is
    /// playing the instrument, and it never enters the undo history.
    pub fn show_on(&mut self, channel: Channel, node: NodeId) {
        if self.doc.graph().get(node).is_some_and(|n| n.def.is_output) {
            self.project.mixer_mut().claim(channel, node);
        }
    }

    pub fn set_balance(&mut self, balance: f32) {
        self.project.mixer_mut().set_balance(balance);
        // A hand is later and wins: a fader's write still on a snapshot published before
        // this move is not applied until the synth has seen the fade this goes out on.
        self.midi
            .bar_rig(crate::midi::Target::Balance, self.link.fade_seq() + 1);
        self.link.touch(crate::midi::Target::Balance);
    }

    /// Blackout on or off, as its press in the Main Mixer does: the mix black wherever it is
    /// shown until it is let go. Playing, not editing, so not a command; and a hand on it
    /// wins over a note's switch the synth made before it saw the press.
    pub fn set_blackout(&mut self, on: bool) {
        self.project.mixer_mut().blackout = on;
        self.midi
            .bar_rig(crate::midi::Target::Blackout, self.link.fade_seq() + 1);
        self.link.touch(crate::midi::Target::Blackout);
    }

    /// Freeze on or off, as its press does: the mix's last frame, wherever it is shown, until
    /// it is let go. As [`Self::set_blackout`].
    pub fn set_freeze(&mut self, on: bool) {
        self.project.mixer_mut().freeze = on;
        self.midi
            .bar_rig(crate::midi::Target::Freeze, self.link.fade_seq() + 1);
        self.link.touch(crate::midi::Target::Freeze);
    }

    pub fn set_method(&mut self, method: crate::mixer::Method) {
        self.project.mixer_mut().method = method;
    }

    pub fn set_mix_resolution(&mut self, resolution: crate::mixer::Resolution) {
        self.project.mixer_mut().resolution = resolution;
    }

    pub fn set_background(&mut self, on: bool) {
        self.project.mixer_mut().background = on;
    }

    /// A hand on `Show on A` or `Show on B`, or an event arriving down its port, puts that
    /// Output on the deck. The reading of both is the tick's, because an Output's press is
    /// an edge against the previous tick; the deck is the project's, so the claiming is
    /// here.
    fn claim_decks(&mut self, claims: &[(NodeId, Channel)]) {
        for (id, channel) in claims {
            self.project.mixer_mut().claim(*channel, *id);
        }
    }

    /// What the last save or load did, as the status line shows it.
    pub fn file_status(&self) -> &str {
        self.media.status()
    }

    /// The file the status line offers to show — a Snap's picture, a render's film or folder —
    /// if it offers one.
    pub fn file_status_shows(&self) -> Option<&std::path::Path> {
        self.media.status_shows()
    }

    /// What is remembered between runs, for the menu and for tests.
    pub fn preferences(&self) -> &preferences::Preferences {
        self.prefs.get()
    }

    /// Which control popup is open on the canvas, if any.
    pub fn open_control(&self) -> Option<crate::ui::OpenControl> {
        self.canvas.open_control()
    }

    /// Do it, or put the confirm up first when there are unsaved edits, or when the show is
    /// going out — a picture window, the mix over NDI or Syphon, a render.
    fn guarded(&mut self, ui: &egui::Ui, pending: Pending) {
        if self.must_ask() {
            self.confirm = Some((pending, None));
        } else {
            self.do_pending(ui, pending);
        }
    }

    /// What the confirm was standing in front of, once it is answered or was never needed.
    /// A render running is canceled first, and this waits for it to end.
    fn do_pending(&mut self, ui: &egui::Ui, pending: Pending) {
        if self.rendering() {
            self.cancel_render();
            self.waiting = Some(pending);
            return;
        }
        match pending {
            Pending::Quit => {
                self.closing = true;
                ui.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::Restart => {
                let root = self.project.root();
                let reopen =
                    (self.home == Home::Folder && Project::is_project(root)).then_some(root);
                self.restart.ask(restart::arguments(reopen));
                self.closing = true;
                ui.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::OpenDialog => self.ask_for_file(FileAsk::OpenProject),
            Pending::NewDialog => self.ask_project_name(crate::ui::project_name::Purpose::New),
            Pending::Open(root) => self.open_project(root),
        }
    }

    /// Whether a modal question stands over the canvas.
    fn dialog_up(&self) -> bool {
        self.confirm.is_some()
            || self.crashlog.notice_up()
            || self.recovery.is_some()
            || self.offer_arrange
            || self.project_name_ask.is_some()
    }

    /// Put Project ▸ New project…'s or Save as…'s window up: its name the next free *Untitled
    /// N* in the projects folder for a new project, and for a copy this project's own name, or
    /// the next free one after it.
    fn ask_project_name(&mut self, purpose: crate::ui::project_name::Purpose) {
        use crate::ui::project_name::{ProjectNameState, Purpose};
        // Made now if it is not there, so the window says at once if it cannot be.
        if let Some(dir) = self.projects_dir() {
            let _ = project::projects_dir_problem(&dir, true);
        }
        let dir = self.projects_dir();
        let name = match (purpose, dir) {
            (Purpose::New, Some(dir)) => project::next_untitled(&dir),
            (Purpose::New, None) => "Untitled".to_owned(),
            (Purpose::SaveAs, Some(dir)) => project::next_copy(&dir, &self.project.name()),
            (Purpose::SaveAs, None) => self.project.name(),
        };
        self.project_name_ask = Some(ProjectNameState::new(purpose, name));
    }

    /// Project ▸ New project…'s or Save as…'s window, while it is up: the name checked against
    /// the projects folder every frame, so the reason it cannot be had is on screen as it is
    /// typed.
    fn project_name_window(&mut self, ctx: &egui::Context) {
        use crate::ui::project_name::{self, ProjectNameAction, ProjectNameView, Purpose};
        let Some(mut state) = self.project_name_ask.take() else {
            return;
        };
        let dir = self.projects_dir();
        let verdict = match &dir {
            Some(dir) => match project::projects_dir_problem(dir, false) {
                Some(problem) => Err(format!("{}. Choose a location.", capitalized(&problem))),
                None => project::new_project_path(dir, &state.name),
            },
            None => Err(format!(
                "Nowhere to keep projects: {}. Choose a location.",
                crate::platform::dirs::DOCUMENTS_UNSET
            )),
        };
        let view = ProjectNameView {
            dir: dir.as_deref(),
            verdict: &verdict,
        };
        let purpose = state.purpose;
        match project_name::show(ctx, &mut state, &view, &self.theme) {
            None => self.project_name_ask = Some(state),
            Some(ProjectNameAction::Chosen(root)) => match purpose {
                Purpose::New => self.new_project(root),
                Purpose::SaveAs => self.save_project_as(root),
            },
            Some(ProjectNameAction::ChooseLocation) => self.ask_for_file(match purpose {
                Purpose::New => FileAsk::NewProject,
                Purpose::SaveAs => FileAsk::SaveAs,
            }),
            Some(ProjectNameAction::Cancel) => {}
        }
    }

    /// **Test hook.** Answer every file dialog at once with `answer` — a folder, or `None` for
    /// a dialog closed — rather than putting one up on the desktop.
    #[doc(hidden)]
    pub fn answer_file_dialogs(&mut self, answer: Option<std::path::PathBuf>) {
        self.dialog_answer = Some(Canned(answer));
    }

    /// Whether a Save asks for a folder: the project on screen is the launch's scratch one in
    /// the temp folder, there having been nowhere to make `Untitled`.
    pub fn save_asks_for_a_folder(&self) -> bool {
        self.home == Home::Scratch
    }

    /// **Test accessor.** Whether Project ▸ New project…'s or Save as…'s window is up.
    #[doc(hidden)]
    pub fn asking_project_name(&self) -> bool {
        self.project_name_ask.is_some()
    }

    /// The unsaved-edits confirm.
    ///
    /// Save writes the folder and carries on; Discard goes on without it; Cancel leaves
    /// everything as it was. A Save that fails carries on with nothing: the question stays
    /// up with the reason under it, and the answer is asked again. Drawn inside the frame,
    /// which is why it never stops the projector, and as an `egui::Modal` — the start menu
    /// and the browser are `Order::Foreground` areas, so a question at any lesser order can
    /// be covered by a list of nodes, and a question the canvas underneath still answers is
    /// not a question.
    fn confirm_window(&mut self, ui: &egui::Ui) {
        let Some((pending, failed)) = self.confirm.clone() else {
            return;
        };
        let mut answered = None;
        let air = self.on_air();
        let dirty = self.dirty();
        egui::Modal::new(egui::Id::new("unsaved-changes")).show(ui.ctx(), |ui| {
            // With no unsaved edits, the show is the whole question.
            if !dirty {
                for line in air.lines() {
                    ui.label(line);
                }
                ui.label(air.question(&pending));
                ui.horizontal(|ui| {
                    if ui.button(onair::go_on_label(&pending)).clicked() {
                        answered = Some(false);
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm = None;
                    }
                });
                return;
            }
            ui.label("Unsaved changes");
            ui.label(format!(
                "{} has edits that are not saved.",
                self.project.name()
            ));
            for line in air.lines() {
                ui.label(line);
            }
            if let Some(going_on) = air.going_on() {
                ui.label(going_on);
            }
            if let Some(failed) = &failed {
                ui.colored_label(ui.visuals().error_fg_color, failed);
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answered = Some(true);
                }
                if ui.button("Discard").clicked() {
                    answered = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.confirm = None;
                }
            });
        });
        if let Some(save) = answered {
            if save && let Err(failed) = self.save_project() {
                self.confirm = Some((pending, Some(failed)));
                return;
            }
            if !save && dirty {
                self.discard_edits();
            }
            self.confirm = None;
            self.do_pending(ui, pending);
        }
    }

    /// The recovery question: an autosave newer than the last save was found when the
    /// project opened. Recover puts it on screen, unsaved; Discard deletes it. Modal, like
    /// the confirm, and with no Cancel, since there is no third thing to do with it. It waits
    /// while *supersilvia closed unexpectedly* is up, which says first why there is anything
    /// to recover.
    fn recovery_window(&mut self, ui: &egui::Ui) {
        let Some(when) = self.recovery_offered() else {
            return;
        };
        if self.crashlog.notice_up() {
            return;
        }
        let mut answered = None;
        egui::Modal::new(egui::Id::new("recover-autosave")).show(ui.ctx(), |ui| {
            ui.label(format!(
                "Recover unsaved changes from {}?",
                ago(std::time::SystemTime::now()
                    .duration_since(when)
                    .unwrap_or_default())
            ));
            ui.label(format!(
                "{} was left with edits that were never saved.",
                self.project.name()
            ));
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    answered = Some(true);
                }
                if ui.button("Discard").clicked() {
                    answered = Some(false);
                }
            });
        });
        match answered {
            Some(true) => self.recover(),
            Some(false) => self.discard_recovery(),
            None => {}
        }
    }

    /// When the autosave on offer was written, while the recovery question is up.
    pub fn recovery_offered(&self) -> Option<std::time::SystemTime> {
        self.recovery.as_ref().map(|r| r.when)
    }

    /// Put the autosave on offer on screen as the document, unsaved: the person saves to
    /// keep it. The autosave stays on disk until then.
    pub fn recover(&mut self) {
        let Some(recovered) = self.recovery.take() else {
            return;
        };
        let mut project = self.project.clone();
        let graph = project.recover(recovered);
        self.replace_project(project, graph);
        self.doc.mark_unsaved();
        self.media.set_status(format!(
            "recovered unsaved changes to {}",
            self.project.name()
        ));
    }

    /// Delete the autosave on offer and carry on with the project as it was saved.
    pub fn discard_recovery(&mut self) {
        if self.recovery.take().is_some() {
            self.autosave.clear(&self.project);
        }
    }

    /// The unsaved edits are thrown away: the autosave holding them is deleted and nothing
    /// more of this document is written, so no recovery is offered for them.
    pub fn discard_edits(&mut self) {
        self.autosave.forget(&self.project);
    }

    /// Record the document for the autosave, and start a write where one is due: once a
    /// frame, with the frame's clock in seconds. See [`autosave`].
    pub fn autosave_tick(&mut self, now: f64) {
        self.autosave.update(
            now,
            self.doc.dirty(),
            self.doc.edit(),
            self.doc.shared(),
            &self.project,
        );
    }

    /// Wait for an autosave in flight to land.
    pub fn wait_for_autosave(&mut self) {
        self.autosave.wait();
    }

    /// What a lost device does before the process ends: write the newest unsaved document at
    /// once, and hand back what the person is told. `App::new` calls it from wgpu's
    /// lost-device callback; a test calls it here.
    pub fn save_for_lost_device(&self, why: &str) -> String {
        autosave::device_lost(&self.autosave.shared(), why)
    }

    /// Add a node of this kind, as the start menu asks: at the next staggered drop point on
    /// the workspace a node lands on.
    fn add_node_at_drop(&mut self, slug: &'static str) {
        let at = self.next_drop;
        self.next_drop += DROP_STAGGER;
        if self.next_drop.x > FIRST_DROP.x + DROP_STAGGER.x * 3.0 {
            self.next_drop.x = FIRST_DROP.x;
        }
        let workspace = self.landing_workspace();
        let _ = self.apply(Command::AddNode {
            slug,
            at,
            workspace,
        });
    }

    /// What the GPU was chosen from: the adapters `main` enumerated to pick the one it handed
    /// eframe, for the Preferences window to list. A test hands a list of its own.
    ///
    /// Where the adapter Preferences ▸ Performance ▸ Use GPU names was passed over for the
    /// strongest, the run says so as it says any failure: the status line, a toast and the
    /// problems list.
    pub fn use_gpu_choice(&mut self, choice: Option<crate::render::adapter::Choice>) {
        if let Some(note) = choice.as_ref().and_then(gpu::fallback) {
            log::warn!("{note}");
            self.fail(note);
        }
        self.gpu_choice = choice;
    }

    /// Where a Restart to apply is recorded for `main` to act on once the run has closed: a
    /// clone of the one `main` holds.
    pub fn use_restart(&mut self, restart: restart::Restart) {
        self.restart = restart;
    }

    /// **Test accessor.** Whether the app is to start again once it has closed.
    #[doc(hidden)]
    pub fn restart_asked(&self) -> bool {
        self.restart.asked()
    }

    /// Put the menus in the operating system's own menu bar, where it has one — AppKit's on
    /// macOS — rather than in the window. Asked for by `main` alone, so a test harness keeps
    /// the egui bar it clicks through on every machine.
    pub fn use_native_menu(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.native_menu = crate::platform::menu::Bar::install(move || ctx.request_repaint());
    }

    /// Answer one thing the menu asked for.
    ///
    /// The single place menu intent turns into app state. Graph edits go through `apply` like
    /// everything else; the rest are app-level and deliberately not commands — undoing a
    /// window resize or a View toggle is not a thing anyone wants.
    // `MenuAction` is a message, and this is the one place that answers it. Taking it by
    // value says the message is spent here and cannot be answered twice.
    #[allow(clippy::needless_pass_by_value)]
    fn handle_menu(&mut self, ui: &egui::Ui, action: MenuAction) {
        match action {
            MenuAction::NewProject => self.guarded(ui, Pending::NewDialog),
            MenuAction::OpenProject => self.guarded(ui, Pending::OpenDialog),
            MenuAction::OpenRecent(root) => self.guarded(ui, Pending::Open(root)),
            MenuAction::Save => {
                let _ = self.save_project();
            }
            MenuAction::SaveAs => {
                self.ask_project_name(crate::ui::project_name::Purpose::SaveAs);
            }
            MenuAction::ShowProjectFolder => self.show_project_folder(),
            MenuAction::Quit => self.guarded(ui, Pending::Quit),
            MenuAction::OpenPreferences => {
                let tab = self.prefs_tab;
                self.prefs_window
                    .get_or_insert_with(|| crate::ui::prefs::PrefsState::on(tab));
            }
            MenuAction::OpenMidi => self.midi.open_window(),
            MenuAction::OpenAbout => self.help.about = true,
            MenuAction::OpenLicences => {
                self.help.licences.get_or_insert_default();
            }
            MenuAction::ReportProblem => self.report_problem(),
            MenuAction::Undo => self.undo_and_say(),
            MenuAction::Redo => self.redo_and_say(),
            MenuAction::OpenUndoHistory => self.open_undo_history(),
            MenuAction::OpenShortcuts => self.help.shortcuts = true,
            MenuAction::PlayPause => {
                // The readout's own rule: a render owns the playhead.
                if !self.rendering() {
                    self.toggle_pause();
                }
            }
            MenuAction::HideEditor => {
                self.show.hidden = !self.show.hidden;
                self.say_state(if self.show.hidden {
                    "Editor hidden — press H to show"
                } else {
                    "Editor visible — press H to hide"
                });
            }
            MenuAction::Fullscreen => {
                let is = ui.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ui.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is));
            }
            MenuAction::Zoom(step) => {
                // About the middle of the canvas, as last drawn: a key has no pointer to keep
                // still under, and the middle is what a person is looking at.
                let origin = self.canvas.origin();
                let middle = origin + emath::vec2(self.canvas.width(), self.canvas.height()) * 0.5;
                let zoom = self.canvas.transform.zoom;
                let factor = match step {
                    crate::ui::menu::Zoom::In => crate::ui::menu::ZOOM_STEP,
                    crate::ui::menu::Zoom::Out => crate::ui::menu::ZOOM_STEP.recip(),
                    crate::ui::menu::Zoom::Actual => zoom.max(f32::EPSILON).recip(),
                };
                let range = self.prefs.get().zoom_range();
                self.canvas
                    .transform
                    .zoom_about(origin, middle, factor, range);
            }
            MenuAction::Nodes(ref command) => {
                let _ = self.apply(command.clone());
            }
            MenuAction::Clip(action) => self.clipboard_action(action),
            MenuAction::SetTime(on) => self.prefs.set_show_time(on),
            MenuAction::Problems => self.toggle_problems(),
            MenuAction::Transport(command) => {
                if !self.rendering() {
                    self.transport(command);
                }
            }
            MenuAction::SetCosts(on) => self.set_show_costs(on),
            MenuAction::SetLayout(mode) => {
                let Some(workspace) = self.active_workspace() else {
                    return;
                };
                let _ = self.apply(Command::SetLayout { workspace, mode });
                // Switching to a strip on a workspace laid out by hand leaves nodes piled at
                // the clamp. Offer the arrange rather than performing it: rearranging
                // someone's work without asking is worse than asking.
                // A row's worth of slack: a node two points above the margin is not a
                // graph that needs rearranging, and a dialog for one is noise.
                let slack = crate::ui::canvas::PORT_PITCH;
                self.offer_arrange = mode == crate::graph::LayoutMode::Linear
                    && self.doc.graph().on_workspace(workspace).any(|(id, n)| {
                        let clamped = crate::ui::clamp_to_strip(
                            self.doc.graph(),
                            id,
                            n.pos,
                            self.canvas.height(),
                        );
                        (clamped.y - n.pos.y).abs() > slack
                    });
            }
            MenuAction::AutoArrange => {
                self.offer_arrange = false;
                let Some(workspace) = self.active_workspace() else {
                    return;
                };
                let height = self.canvas.height();
                let _ = self.apply(Command::AutoArrange { workspace, height });
            }
            MenuAction::RenameWorkspace => {
                if let Some(id) = self.active_workspace()
                    && let Some(workspace) = self.doc.graph().workspace(id)
                {
                    let name = workspace.name.clone();
                    self.tabs.rename(id, &name);
                }
            }
            MenuAction::CloseWorkspace => {
                if let Some(id) = self.active_workspace() {
                    self.close_workspace(id);
                }
            }
            MenuAction::ExportWorkspace => {
                if let Some(id) = self.active_workspace() {
                    self.ask_for_file(FileAsk::ExportWorkspace(id));
                }
            }
            MenuAction::ResetView => {
                self.canvas.transform = crate::ui::canvas::Transform::default();
            }
            MenuAction::NewWorkspace => self.add_workspace(crate::graph::WorkspaceKind::Video),
            MenuAction::GoToTab(index) => {
                if let Some(to) = self.tab_bar().order().get(index).copied() {
                    self.activate(to);
                }
            }
        }
    }

    /// **Test accessor.** True when this Output's shader will be rebuilt for the next plan.
    #[doc(hidden)]
    pub fn needs_recompile(&mut self, node: NodeId) -> bool {
        self.link.needs_recompile(node)
    }

    /// **Test accessor.** Take the pending rebuilds, as the next plan would. For tests that
    /// want to assert what a later command does or does not dirty.
    #[doc(hidden)]
    pub fn take_recompiles(&mut self) -> Vec<NodeId> {
        self.link.take_recompiles()
    }

    /// Build the plan, keep it for the Status box, and hand it over — where anything it is
    /// built from changed since the last one.
    pub fn publish_plan(&mut self) {
        self.with_link(SynthLink::publish_plan);
    }

    /// **Test and inline only.** The frame the plan describes, resolved by the synth on
    /// this thread. What `build_frame_job` was, kept for the tests that read `job.time`.
    #[doc(hidden)]
    pub fn build_frame_job(&mut self) -> FrameJob {
        self.with_link(SynthLink::build_frame_job)
    }

    /// **Test accessor.** How many plans have been built and handed over. The same number
    /// over a frame is a frame that rebuilt and sent none.
    #[doc(hidden)]
    pub fn plans_built(&self) -> u64 {
        self.link.plan_seq()
    }

    /// **Test accessor.** How many shader and probe sources have been handed to the
    /// renderer. Each is a link there, unless it is one the renderer kept.
    #[doc(hidden)]
    pub fn sources_sent(&self) -> u64 {
        self.link.sources_sent()
    }

    /// **Test accessor.** Open or close the Status box, which is what times each tick by
    /// phase on both processors — the readings a measurement with no window reads.
    #[doc(hidden)]
    /// Soft takeover on or off, as its row in Preferences ▸ Editing does.
    pub fn set_soft_takeover(&mut self, on: bool) {
        self.prefs
            .set_flag(crate::preferences::Flag::SoftTakeover, on);
    }

    pub fn set_show_status_box(&mut self, on: bool) {
        self.prefs.set_show_status_box(on);
    }

    /// **Test accessor.** A renderer on a device a test or a bench made, so the offline
    /// render can be driven with a real GPU and no eframe.
    #[doc(hidden)]
    pub fn attach_gpu_on(&mut self, gpu: crate::render::Gpu) {
        if let Some(synth) = self.link.host_mut().inline_synth_mut()
            && let Err(err) = synth.attach_gpu(gpu)
        {
            log::error!("could not make the test renderer: {err}");
            return;
        }
        self.has_gpu = true;
    }

    /// **Test accessor.** The blit pipeline for targets of `format` and the paint timer on a
    /// device a test or a bench made, as `App::new` makes them on eframe's: what lets a
    /// headless host paint the editor's own frame, pictures and all, with egui_wgpu.
    #[doc(hidden)]
    pub fn attach_viewer_on(&mut self, gpu: &crate::render::Gpu, format: wgpu::TextureFormat) {
        match Viewer::new(gpu, format) {
            Ok(v) => self.viewer = Some(Arc::new(v)),
            Err(err) => log::error!("could not create the viewer: {err}"),
        }
        self.paint_timer = crate::render::timing::PaintTimer::new(gpu).map(Arc::new);
    }

    /// **Test accessor.** Paint as a window eframe clears to the ground does — no panel's or
    /// canvas's ground of its own — or as one that does not. The caller clears with
    /// [`crate::ui::theme::clear_color`] and applies the theme to its own context with
    /// [`crate::ui::theme::apply_cleared`].
    #[doc(hidden)]
    pub fn set_cleared(&mut self, on: bool) {
        self.ground = if on { Ground::Cleared } else { Ground::Painted };
    }

    /// **Test accessor.** Preferences read from somewhere of the caller's choosing, the theme
    /// with them — what a headless host does to wear a person's layout rather than the
    /// defaults. The caller applies the theme to its own `egui::Context`.
    #[doc(hidden)]
    pub fn use_preferences(&mut self, prefs: preferences::Store) {
        self.theme = prefs.get().theme;
        self.gpu_started = gpu::Setting::of(prefs.get());
        self.prefs = prefs;
    }

    /// **Test accessor.** The editor's painting on the GPU as the Status box reads it: the last
    /// frame, the mean and the worst, in milliseconds.
    #[doc(hidden)]
    pub fn paint_gpu(&self) -> Option<(f32, f32, f32)> {
        self.meters.paint_gpu.map(|g| (g.now, g.avg, g.worst))
    }

    /// **Test accessor.** One shader's uniforms as the synth resolves them.
    #[doc(hidden)]
    pub fn resolve_uniforms(
        &mut self,
        shader: &compile::Shader,
    ) -> Vec<(Arc<str>, crate::render::UniformValue)> {
        self.with_link(|link, session| link.resolve_uniforms(session, shader))
    }

    /// Whether anything will read the per-node half of a tick: a canvas on screen, or a
    /// picture in a window of its own. The project tab and `H` both leave nothing that
    /// reads a node's status, scope or playhead.
    fn wants_report(&self) -> bool {
        (self.active_workspace().is_some() && !self.show.hidden) || !self.wall.open().is_empty()
    }

    /// What the editor is showing that a plan reads: whether anything reads the per-node
    /// report, the nodes whose picture has a window of its own, and the Output a render is
    /// capturing. Each of the last two is a reason an Output draws.
    fn seen(&self) -> Seen {
        Seen {
            report: self.wants_report(),
            windows: self
                .wall
                .open()
                .iter()
                .filter_map(|p| match p.picture {
                    crate::render::picture::Shown::Node { node, port: None } => Some(node),
                    _ => None,
                })
                .collect(),
            capture: self
                .link
                .snapshot()
                .offline
                .running
                .as_ref()
                .map(|r| r.output),
            sent: self.sent_outputs(),
        }
    }

    /// Every Output sent out over Syphon or NDI.
    fn sent_outputs(&self) -> std::collections::BTreeSet<NodeId> {
        self.doc
            .graph()
            .iter()
            .filter(|(_, n)| n.def.is_output && crate::nodes::output::sent_of(n).any())
            .map(|(id, _)| id)
            .collect()
    }

    /// Hand the publisher what should be sent out: each Output sent out over Syphon or NDI,
    /// under its name for each and with its looks, and the mix while a mark is lit. An NDI
    /// stream declares the rate the synth ticks at. It sends only a change.
    fn send_out(&mut self) {
        use crate::render::publish::{Look, Via, Wanted};
        let rate = (1000.0 / self.meters.tick_budget_ms.max(1.0)).round() as u32;
        let ndi = Via::Ndi { rate };
        let graph = self.doc.graph();
        let mut wanted: Vec<Wanted> = Vec::new();
        for (id, n) in graph.iter().filter(|(_, n)| n.def.is_output) {
            let sent = crate::nodes::output::sent_of(n);
            let picture = crate::render::picture::Shown::Node {
                node: id,
                port: None,
            };
            // One name, both ways.
            let name = crate::nodes::output::send_name(id, n);
            if sent.syphon {
                wanted.push(Wanted {
                    picture,
                    via: Via::Syphon,
                    name: name.clone(),
                    look: Look {
                        flip: sent.flip,
                        transparent: sent.transparent,
                    },
                });
            }
            if sent.ndi {
                wanted.push(Wanted {
                    picture,
                    via: ndi,
                    name,
                    look: Look {
                        flip: false,
                        transparent: sent.transparent,
                    },
                });
            }
        }
        let mix = crate::render::picture::Shown::Mix;
        if self.sending.mix.syphon {
            wanted.push(Wanted {
                picture: mix,
                via: Via::Syphon,
                name: MIX_SERVER.to_string(),
                look: Look::default(),
            });
        }
        if self.sending.mix.ndi {
            wanted.push(Wanted {
                picture: mix,
                via: ndi,
                name: MIX_STREAM.to_string(),
                look: Look::default(),
            });
        }
        self.sending.publisher.want(wanted);
    }

    /// **Test accessor.** What the publisher is asked to send out, as a frame asks it.
    #[doc(hidden)]
    pub fn sent_wanted(&mut self) -> Vec<crate::render::publish::Wanted> {
        self.send_out();
        self.sending.publisher.wanted().to_vec()
    }

    /// **Test accessor.** Publish the mix over Syphon, or stop, as the Main Mixer's mark does.
    #[doc(hidden)]
    pub fn set_syphon_mix(&mut self, on: bool) {
        self.sending.mix.syphon = on;
    }

    /// **Test accessor.** Send the mix over NDI, or stop, as the Main Mixer's NDI mark does.
    #[doc(hidden)]
    pub fn set_ndi_mix(&mut self, on: bool) {
        self.sending.mix.ndi = on;
    }

    /// What the tick is given beside the plan, where it changed. See
    /// [`SynthLink::push_inputs`].
    pub(super) fn push_inputs(&mut self) {
        self.midi
            .soft_takeover(self.prefs.get().midi_soft_takeover, &self.link);
        let assets = self.project.asset_paths();
        let main_input = self.project.main_input().clone();
        self.with_link(|link, session| link.push_inputs(session, assets, main_input));
    }

    /// Take the newest snapshot and do the editor's housekeeping around it.
    ///
    /// Everything that was done *before* the tick when the tick was on this thread is done
    /// *after* it now, because what it reads is what the tick left: the probes' counts, the
    /// drop rates, a picture a save asked for, the decks a press claimed. The one that moved
    /// with the tick rather than around it is the tap readings, which the synth routes where
    /// it drew them. Each event is applied once and in the order it happened, however many
    /// snapshots the editor missed: see [`crate::synth::events`].
    pub(super) fn take_snapshot(&mut self) {
        self.link.take();
        // A deck whose Output was deleted is empty, on the frame it went. A binding onto a
        // control that went with the node stays in the map, so undoing the delete brings it
        // back, and stops being published: the synth and the MIDI window see only the live
        // ones, and a save writes only those.
        self.project.mixer_mut().reconcile(self.doc.graph());
        self.publish_midi();
        self.link.collect_probes(self.doc.graph());
        let prefs = self.prefs.get();
        self.link
            .track_drops(self.doc.graph(), prefs.show_costs || prefs.show_status_box);
        self.media
            .collect_thumbnails(self.link.snapshot(), &self.project);
        self.collect_snaps();
        self.collect_recordings();
        // A finished render says where it went where a person sees it, with a Show.
        if self.media.observe_render(self.link.snapshot())
            && let Some(file) = self.media.status_shows().map(std::path::Path::to_path_buf)
        {
            let said = self.media.status().to_owned();
            self.say_written(said, file);
        }
        let decks: Vec<_> = (self.link.snapshot().events.decks)
            .unseen("deck claims")
            .map(|(_, claim)| *claim)
            .collect();
        self.claim_decks(&decks);
        self.canvas.fired_on(
            (self.link.snapshot().events.fired)
                .unseen("ticks of firings")
                .flat_map(|(_, ports)| ports.iter().copied()),
        );
        self.take_value_writes();
    }

    /// Put every value a tick wrote onto its own node back through the bus, so the document
    /// agrees and the file carries it.
    ///
    /// The `SetValue` a hand would have sent, had there been an editor for a performance: one
    /// undo step per recording, because a tick writes one when a transport stops rather than
    /// while it runs — or none of its own where it lands while a gesture is open, which it
    /// joins rather than splits. A write the editor's own graph already agrees with has
    /// landed and is skipped.
    fn take_value_writes(&mut self) {
        let writes: Vec<_> = self
            .link
            .snapshot()
            .events
            .values
            .unseen("value writes")
            .map(|(_, write)| write.clone())
            .collect();
        for (node, key, value) in writes {
            let landed = self
                .doc
                .graph()
                .get(node)
                .and_then(|n| n.values.get(key))
                .is_some_and(|had| *had == value);
            if landed {
                continue;
            }
            let _ = self.apply_from(
                Command::SetValue { node, key, value },
                document::Origin::Tick,
            );
        }
    }

    /// **Inline only.** One tick on this thread: send what the tick is given, step, and
    /// take what it left.
    ///
    /// The path `App::headless` and every layer-1 test drive. It runs the same
    /// `Synth::step` through the same channel and the same buffer swap the synth thread
    /// runs, so there is one implementation and not two.
    pub fn tick(&mut self, dt: f32) {
        self.step_inline(crate::synth::Beat::Delta(dt));
    }

    /// **Inline only.** One render frame at playhead `t`, as `Synth::render_frame` steps one:
    /// the transport driven to `t` with the travel between, no step clamped. A run begins
    /// with `transport(Command::Seek(first))`, as a render does. For the tests that render
    /// with no GPU.
    #[doc(hidden)]
    pub fn tick_at(&mut self, t: f64) {
        self.step_inline(crate::synth::Beat::At(t));
    }

    /// **Inline only, and a test's.** The editor's half of a frame with no tick: take the
    /// newest snapshot and apply what it carries, as a frame painted between two ticks does.
    /// `App::step_synth` is the other half.
    #[doc(hidden)]
    pub fn read_synth(&mut self) {
        self.take_snapshot();
        let now = self.midi.advance(crate::synth::Beat::Delta(0.0));
        self.take_midi(now);
    }

    fn step_inline(&mut self, beat: crate::synth::Beat) {
        self.push_inputs();
        self.link.host_mut().step(beat);
        self.take_snapshot();
        // What the frame does once a frame, where there is no frame. The clock is the
        // editor's own, and what this time is measured for is the silence between two turns
        // of a knob.
        let now = self.midi.advance(beat);
        self.take_midi(now);
    }

    /// See [`crate::synth::Synth::reset_cpu`].
    pub fn reset_cpu(&mut self) {
        self.link.send(Msg::ResetCpu);
    }

    /// Everything one CPU node reports, for diagnostics.
    ///
    /// Asked of the synth where it is on this thread, and read off the snapshot where it is
    /// not — and there it is gathered only while the Status box is open, since that is the
    /// only thing in the app that shows it.
    pub fn cpu_report(&self, node: NodeId) -> String {
        self.link.cpu_report(node)
    }

    // ---------------------------------------------------------------- midi

    /// Read what the tick left of MIDI and do the editor's half of it, then put every write
    /// a bound knob made through the bus as the `SetControl` it would have been, so the
    /// document, the file and the undo history catch up. Once a frame, after the snapshot is
    /// taken.
    ///
    /// Every write joins the open gesture, whichever control it lands on, and holds it open
    /// until the knobs fall silent: see `document::history`.
    pub(super) fn take_midi(&mut self, now: f64) {
        let writes = self.midi.take(&mut self.link, &mut self.project);
        // A binding learned just now, so the next tick drives it.
        self.publish_midi();
        for (target, value) in writes {
            if !self.midi.lands(self.doc.graph(), target, value, now) {
                continue;
            }
            let _ = self.apply_from(
                Command::SetControl {
                    node: target.node,
                    key: target.key,
                    value: crate::graph::ControlValue::Float(value),
                },
                document::Origin::Midi,
            );
        }
        self.settle_midi_at(now);
    }

    /// Post a message as though a device had sent it.
    ///
    /// Public because the device is not the only thing that can produce one — a test drives
    /// this, which is what lets the whole map and every scaling rule be checked on a box with
    /// nothing plugged into it. It goes into the synth's own queue, so it is applied on the
    /// next tick and comes back on the snapshot exactly as a device's does.
    #[doc(hidden)]
    pub fn apply_midi(&mut self, message: crate::midi::Message) {
        self.apply_midi_wire(crate::midi::Wire::Message(
            crate::midi::Device::EDITOR,
            message,
        ));
    }

    /// [`Self::apply_midi`], from a device of the test's choosing, or that device going away.
    #[doc(hidden)]
    pub fn apply_midi_wire(&mut self, wire: crate::midi::Wire) {
        self.midi.post(wire, &self.link);
    }

    /// The MIDI window's **Release all**: every action input a note is holding is let go, on
    /// the next tick.
    pub fn release_midi_notes(&mut self) {
        self.link.send(Msg::MidiReleaseAll);
    }

    /// Where the fader bound to `target` is, in the control's own units, while soft takeover
    /// holds it out of pick-up: the ghost mark the control wears. `None` once it has picked
    /// up, and always with the preference off.
    pub fn midi_ghost(&self, target: impl Into<crate::midi::Target>) -> Option<f32> {
        let target = target.into();
        self.link
            .snapshot()
            .midi_ghosts
            .iter()
            .find(|(t, _)| *t == target)
            .map(|(_, at)| *at)
    }

    /// Let the knobs stop holding the undo step open where they have been still long enough;
    /// it closes once the pointer is up too. Public for the same reason `apply_midi` is: a
    /// test has to be able to let the silence happen.
    pub fn settle_midi_at(&mut self, now: f64) {
        if self.midi.settled(now) {
            self.doc.end_midi_gesture();
        }
    }

    /// Start learning: the next message binds to this control.
    ///
    /// Not a command. Binding a knob is setting the rig up rather than editing the patch —
    /// even though the map it writes rides in the project file — so it never enters the undo
    /// history, exactly as claiming a deck does not.
    pub fn learn_midi(&mut self, target: impl Into<crate::midi::Target>) {
        let target = target.into();
        // Learning replaces what drove the control: the old binding goes as the learn is armed,
        // so an `Escape` leaves the control unbound rather than quietly keeping it.
        if self.project.midi().trigger_of(target).is_some() {
            self.project.midi_mut().unbind_target(target);
            self.publish_midi();
        }
        self.midi.learn(target, &self.link);
    }

    /// **A test's.** Put a binding in the map that the learn gesture cannot make, because a
    /// file can: two triggers on one control. See [`crate::midi::Bindings::bind_unchecked`].
    #[doc(hidden)]
    pub fn bind_midi_unchecked(
        &mut self,
        trigger: crate::midi::Trigger,
        target: impl Into<crate::midi::Target>,
    ) {
        self.project.midi_mut().bind_unchecked(trigger, target);
        self.publish_midi();
    }

    /// The control currently waiting for a message, if one is.
    pub fn midi_learning(&self) -> Option<crate::midi::Target> {
        self.midi.learning()
    }

    /// What drives this control, for the dot a bound one wears.
    pub fn midi_trigger_of(
        &self,
        target: impl Into<crate::midi::Target>,
    ) -> Option<crate::midi::Trigger> {
        self.project.midi().trigger_of(target)
    }

    /// Bar a control this editor moved itself from being written back over by a MIDI write
    /// the synth made before it saw the move: after the graph the move went out on.
    pub(super) fn bar_midi(&mut self, cmd: &Command) {
        self.midi.bar(cmd, self.link.graph_generation());
    }

    // ---------------------------------------------------------------- measurement

    /// See [`crate::synth::Snapshot::readback`]: a measured node's slot from the last pass
    /// that measured it, its workspace's.
    pub fn readback(&self, node: NodeId) -> Option<&[u32; compile::TAP_WORDS]> {
        self.link.snapshot().readback(node)
    }

    /// One probe's words, as the renderer read them back: each slot's count, over the
    /// probe's few pixels, scaled to the Output's own. Public so the arithmetic is tested
    /// without a GPU.
    pub fn ingest_probe(&mut self, output: NodeId, words: &[u32]) {
        self.link.ingest_probe(self.doc.graph(), output, words);
    }

    /// How many times one node runs an input per run of its own. `None` for a node with no
    /// connected function input, or one no probe has counted.
    pub fn taps(&self, node: NodeId) -> Option<f64> {
        self.link.taps_by_node().get(&node).copied()
    }

    /// How often a node's function runs: per pixel of the Outputs it is compiled into, and
    /// per frame in all. `None` until a probe has counted it.
    pub fn evaluations(&self, node: NodeId) -> Option<(f64, f64)> {
        self.link.evaluations(self.doc.graph(), node)
    }

    /// The cost view, as the View menu sets it. A preference: how the person likes to look
    /// at a graph, not anything about the graph.
    pub fn set_show_costs(&mut self, on: bool) {
        self.prefs.set_show_costs(on);
    }

    /// What each Output says about itself on its own body: whether anything is cabled in,
    /// which deck it is on, whether it is the one rendering, and why a way out is failing.
    ///
    /// Every Output in the graph and not only the ones on screen, because the map is built
    /// before the canvas culls and a node's readout is cheap: three lookups and no GPU call.
    /// Unlike the cost strips this is not behind a preference — a status line is not a
    /// measurement, and silvia shows its own always.
    pub(super) fn output_readouts(&self) -> HashMap<NodeId, crate::ui::OutputReadout> {
        use crate::render::publish::Via;
        let rendering = self.render_progress().map(|p| p.output);
        let recording_anywhere = self.recording();
        let mixer = self.project.mixer();
        let graph = self.doc.graph();
        // Empty, and so no allocation, on every frame nothing is failing.
        let failures = self.sending.publisher.failures();
        let failure = |id: NodeId, ndi: bool| {
            failures
                .iter()
                .find(|f| {
                    f.picture
                        == (crate::render::picture::Shown::Node {
                            node: id,
                            port: None,
                        })
                        && matches!(f.via, Via::Ndi { .. }) == ndi
                })
                .map(|f| f.why.clone())
        };
        graph
            .iter()
            .filter(|(_, node)| node.def.is_output)
            .map(|(id, _)| {
                let readout = crate::ui::OutputReadout {
                    // Any cable landing on it: an Output has one input, so the port it lands
                    // on is that one and asking which would be asking `nodes/` twice.
                    connected: !graph.cables_into(id).is_empty(),
                    decks: mixer.decks_of(id),
                    rendering: rendering == Some(id),
                    recording: self.recording_of(id),
                    record_error: match self.record_outcome(id) {
                        Some(Err(e)) => Some(e.clone()),
                        _ => None,
                    },
                    recording_anywhere,
                    ndi_error: failure(id, true),
                    syphon_error: failure(id, false),
                };
                (id, readout)
            })
            .collect()
    }
}

/// What the editor is showing that a plan reads, beside the document and the project: see
/// [`App::seen`].
struct Seen {
    report: bool,
    windows: std::collections::BTreeSet<NodeId>,
    capture: Option<NodeId>,
    sent: std::collections::BTreeSet<NodeId>,
}

/// What the plan is built from, as the editor stands: the document's graph, the project's
/// tabs and mixer, the two preferences it reads and what the editor is showing. A function of
/// the fields rather than a method, so the link can be borrowed beside it.
fn session<'a>(
    doc: &'a Document,
    project: &'a Project,
    prefs: &'a preferences::Store,
    mix_viewport: (u32, u32),
    mix_display: Option<(u32, u32)>,
    seen: Seen,
) -> link::Session<'a> {
    link::Session {
        graph: doc.graph(),
        shape: doc.shape(),
        open: &project.session().open,
        active: project.session().active_workspace(),
        mixer: project.mixer(),
        mix_viewport,
        mix_display,
        report: seen.report,
        status: prefs.get().show_status_box,
        preview: !prefs.get().main_input_collapsed,
        windows: seen.windows,
        sent: seen.sent,
        capture: seen.capture,
    }
}

/// A status line as a sentence in a window: its first letter a capital.
fn capitalized(line: &str) -> String {
    let mut chars = line.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// How long ago, as the recovery question says it: `a moment ago`, `4 minutes ago`,
/// `2 hours ago`, `3 days ago`. Relative, because there is no timezone database in this binary
/// to say a clock time in.
fn ago(elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs();
    let (n, unit) = match secs {
        0..60 => return "a moment ago".to_string(),
        60..3600 => (secs / 60, "minute"),
        3600..86_400 => (secs / 3600, "hour"),
        _ => (secs / 86_400, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recovery question's clock: minutes, hours and days, and one of each is singular.
    #[test]
    fn ago_says_how_long_in_the_largest_whole_unit() {
        let at = |secs| ago(std::time::Duration::from_secs(secs));
        assert_eq!(at(0), "a moment ago");
        assert_eq!(at(59), "a moment ago");
        assert_eq!(at(60), "1 minute ago");
        assert_eq!(at(3599), "59 minutes ago");
        assert_eq!(at(7200), "2 hours ago");
        assert_eq!(at(86_400), "1 day ago");
    }

    /// A patch of `probe` into an Output, which is the only node carrying an option of
    /// every kind. The fixture is `cfg(test)`-only, which is why this half of the recompile
    /// boundary is here rather than beside the rest of it in `tests/controls.rs`.
    fn probe_patch() -> (App, NodeId, NodeId) {
        let mut app = App::headless();
        let workspace = app.graph().default_workspace();
        // Through an `rgba`, because the probe publishes a varying number and an Output
        // takes a color. It also puts a node between the two, so what is asserted is that the
        // rebuild reaches the Output downstream rather than only the node that changed.
        for (slug, x) in [("probe", 0.0), ("rgba", 200.0), ("output", 400.0)] {
            app.apply(Command::AddNode {
                slug,
                at: egui::pos2(x, 0.0),
                workspace,
            })
            .unwrap();
        }
        let ids: Vec<_> = app.graph().iter().map(|(id, _)| id).collect();
        let (probe, rgba, out) = (ids[0], ids[1], ids[2]);
        app.apply(Command::Connect {
            from: PortRef::new(probe, "output"),
            to: PortRef::new(rgba, "r"),
        })
        .unwrap();
        app.apply(Command::Connect {
            from: PortRef::new(rgba, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
        let _ = app.take_recompiles();
        (app, probe, out)
    }

    /// The reason the kind exists. A `Uniform` option's branches are all in the compiled
    /// program, so choosing between them is an `int` — and a crossfade method changed in the
    /// middle of a set must not rebuild a shader.
    #[test]
    fn a_uniform_option_never_recompiles() {
        let (mut app, probe, out) = probe_patch();

        app.apply(Command::SetOption {
            node: probe,
            key: "scale",
            value: "two".to_string(),
        })
        .unwrap();
        assert!(
            !app.needs_recompile(out),
            "a uniform option is an int the running program already reads",
        );

        // The `Code` option on the same node still rebuilds, so the exemption is carried by
        // the kind rather than by the node.
        app.apply(Command::SetOption {
            node: probe,
            key: "op",
            value: "sub".to_string(),
        })
        .unwrap();
        assert!(app.needs_recompile(out));
    }

    /// What the shader is handed: the index of the chosen value among the option's choices,
    /// resolved fresh from the graph each frame like every other uniform.
    #[test]
    fn a_uniform_option_uploads_the_index_of_its_choice() {
        let (mut app, probe, out) = probe_patch();
        let name: std::sync::Arc<str> =
            std::sync::Arc::from(format!("u_opt_probe{probe}_scale").as_str());

        let index = |app: &mut App| {
            let shader = compile::wgsl::build(app.graph(), out).expect("connected");
            // The branch is in the source and the choice is not: a uniform option is never
            // baked, exactly as a control value is never baked.
            assert!(shader.body.contains(&*name));
            assert!(!shader.body.contains("\"two\""));
            app.resolve_uniforms(&shader)
                .into_iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| v)
                .expect("the option's uniform is resolved")
        };

        assert_eq!(index(&mut app), crate::render::UniformValue::Int(0));
        app.apply(Command::SetOption {
            node: probe,
            key: "scale",
            value: "two".to_string(),
        })
        .unwrap();
        assert_eq!(index(&mut app), crate::render::UniformValue::Int(1));
    }
}
