// SPDX-License-Identifier: AGPL-3.0-or-later

//! One frame, as eframe asks for it: the `eframe::App` body and what it paints.
//!
//! The Output previews and the mix reach the screen through paint callbacks, which
//! `render::viewer` builds: this module says which picture goes in which rect, and the
//! viewport geometry and the backend are the renderer's.

use super::{App, Pending, document::UNDO_DEPTH, files::FileAsk};
use crate::clock::Figure;
use crate::render::timing::PaintMark;
use crate::ui::menu::{self, MenuState};
use crate::ui::tabs::{self, TabBar};
use eframe::egui;

/// What the tab bar keeps free for the time readout and the frame-rate meter when they stand
/// at its right end.
const TAB_BAR_READOUTS: f32 = 240.0;

/// How long, in seconds, the canvas holds one size before a viewport-matched mix takes it. A
/// window or panel drag changes the canvas on every frame, and a mix that followed it would
/// reallocate on every frame.
pub(super) const MIX_SETTLE_S: f64 = 0.25;

/// How long the editor's own frames take, and the tick's as the editor reads it.
///
/// The **editor's own frame** is off egui's input clock: this frame, the mean of about a
/// second, and the worst of the last two. The synth's tick has its own three, which the
/// snapshot carries — see [`crate::synth::ClockReport`] — and the Status box shows both,
/// because a slow or minimized editor moves these and nothing about the world.
#[derive(Debug)]
pub(super) struct Meters {
    /// CPU time spent building the last frame's job and drawing the canvas, in
    /// milliseconds. If this regresses that is a bug, so it has to be visible.
    pub(super) cpu_ms: f32,
    /// The editor's interval, in milliseconds.
    pub(super) frame: Figure,
    /// The wall time of the previous frame, so this frame's interval can be measured.
    pub(super) frame_last: Option<f64>,
    /// The tick time as a running mean over about a second: a single tick's figure changes
    /// too fast to read.
    pub(super) tick_avg_ms: f32,
    pub(super) cpu_avg_ms: f32,
    /// One interval of the display the window is on, in milliseconds: the budget every
    /// per-frame figure is measured against. 60 Hz until a monitor has answered.
    pub(super) refresh_ms: f32,
    /// One interval of the tick rate the preference asks for, in milliseconds: the display's,
    /// or a fixed rate. The budget every per-tick figure is measured against.
    pub(super) tick_budget_ms: f32,
    /// The GPU time of the editor's own painting, from
    /// [`crate::render::timing::PaintTimer`],
    /// in milliseconds. `None` until a reading lands, and again whenever the Status box
    /// closes, since nothing is marked while it is.
    pub(super) paint_gpu: Option<Figure>,
}

impl Meters {
    /// Fold the paint timer's readings in, oldest first: each is a frame, and each moves the
    /// mean by the weight a frame of the editor's own interval does.
    fn fold_paint(&mut self, readings: &[f32]) {
        let blend = self.frame.now / 1000.0;
        for &ms in readings {
            self.paint_gpu.get_or_insert_default().record(ms, blend);
        }
    }
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            cpu_ms: 0.0,
            frame: Figure::default(),
            frame_last: None,
            tick_avg_ms: 0.0,
            cpu_avg_ms: 0.0,
            refresh_ms: 1000.0 / 60.0,
            tick_budget_ms: 1000.0 / 60.0,
            paint_gpu: None,
        }
    }
}

impl App {
    /// The tab bar as the session and the graph stand, lit for a drop while nodes are held.
    pub(super) fn tab_bar(&self) -> TabBar<'_> {
        TabBar {
            workspaces: self.doc.graph().workspaces(),
            open: &self.project.session().open,
            active: self.project.session().active,
            dragging: self.canvas.dragging_nodes(),
            // Where the menu bar is the operating system's, the readout and the meter stand at
            // the bar's right end.
            reserve: if self.native_menu.is_some()
                && (self.prefs.get().show_fps || self.prefs.get().show_time)
            {
                TAB_BAR_READOUTS
            } else {
                0.0
            },
        }
    }

    /// The MIDI window, while it is open.
    ///
    /// Everything it asks for is the rig rather than the document: rescanning, binding,
    /// unbinding, watching. None of it is a `Command`, so none of it reaches the undo
    /// history — going to a node is the one exception, and that is navigation, which is not
    /// an edit either.
    pub(super) fn show_midi(&mut self, ctx: &egui::Context) {
        if !self.midi.window_open() {
            return;
        }
        let actions = crate::ui::midi::show(
            ctx,
            &self.midi.view(self.doc.graph()),
            &self.theme,
            &self.prefs.get().windows,
        );
        let mut go_to = None;
        for action in actions {
            match self.midi.answer(action, &self.link, &mut self.project) {
                Some(crate::ui::midi::MidiAction::GoTo(node)) => go_to = Some(node),
                Some(crate::ui::midi::MidiAction::ShowMixer) => {
                    self.prefs.set_mixer_collapsed(false);
                }
                _ => {}
            }
        }
        // An unbind or a clear crosses here, before the next tick reads the map.
        self.publish_midi();
        // After the loop, which is where the Status box's own links go: the window is drawn
        // and done with before the view moves under it.
        if let Some(node) = go_to
            && let Some(workspace) = self.home_of(node)
        {
            self.navigate_to(workspace, node);
        }
    }

    /// Help ▸ About supersilvia, while it is open. Its Licences… button opens the Licences
    /// window beside it rather than in its place.
    pub(super) fn show_about(&mut self, ctx: &egui::Context) {
        if !self.help.about {
            return;
        }
        for action in crate::ui::about::show_about(ctx, &self.theme, &self.prefs.get().windows) {
            match action {
                crate::ui::about::AboutAction::OpenLicences => {
                    self.help.licences.get_or_insert_default();
                }
                crate::ui::about::AboutAction::Close => self.help.about = false,
            }
        }
    }

    /// Help ▸ Keyboard shortcuts…, while it is open.
    pub(super) fn show_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.help.shortcuts {
            return;
        }
        let actions = crate::ui::shortcuts::show(ctx, &self.theme, &self.prefs.get().windows);
        if actions.contains(&crate::ui::shortcuts::ShortcutsAction::Close) {
            self.help.shortcuts = false;
        }
    }

    /// Help ▸ Licences…, while it is open.
    pub(super) fn show_licences(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.help.licences.take() else {
            return;
        };
        let actions = crate::ui::about::show_licences(
            ctx,
            &mut state,
            &self.theme,
            &self.prefs.get().windows,
        );
        if !actions.contains(&crate::ui::about::LicencesAction::Close) {
            self.help.licences = Some(state);
        }
    }

    /// The Preferences window, while it is open.
    ///
    /// Applying a theme is three things in the same frame and no command at all: the editor
    /// draws with it, egui's own `Visuals` are rebuilt from it so a `Window` and a menu match
    /// the canvas, and the preference is written. There is no undo step, because a
    /// preference is not an edit to the document — the same rule the Status box follows.
    pub(super) fn show_preferences(&mut self, ctx: &egui::Context) {
        let Some(mut state) = self.prefs_window.take() else {
            return;
        };
        let mut open = true;
        let projects = self.projects_dir();
        let problem = projects
            .as_deref()
            .and_then(|dir| crate::project::projects_dir_problem(dir, false));
        let recordings = self.recordings_dir();
        let view = crate::ui::prefs::PrefsView {
            prefs: self.prefs.get(),
            projects: projects.as_deref(),
            projects_problem: problem.as_deref(),
            recordings: &recordings,
            file: self.prefs.path(),
            file_busy: self.media.file_busy(),
            gpu: self.gpu_choice.as_ref(),
        };
        let actions = crate::ui::prefs::show(ctx, &mut state, &self.theme, &view);
        for action in actions {
            match action {
                crate::ui::prefs::PrefAction::SetTheme(theme) => {
                    self.theme = theme;
                    if self.ground == super::Ground::Cleared {
                        crate::ui::theme::apply_cleared(ctx, &theme);
                    } else {
                        crate::ui::theme::apply(ctx, &theme);
                    }
                    self.prefs.set_theme(theme);
                }
                crate::ui::prefs::PrefAction::SetFlag(flag, on) => self.prefs.set_flag(flag, on),
                crate::ui::prefs::PrefAction::SetDefaultLayout(mode) => {
                    self.prefs.set_default_layout(mode);
                }
                crate::ui::prefs::PrefAction::SetTickRate(rate) => self.prefs.set_tick_rate(rate),
                crate::ui::prefs::PrefAction::SetInterfaceSize(size) => {
                    self.prefs.set_interface_size(size);
                }
                crate::ui::prefs::PrefAction::ShowProjects => self.show_projects_folder(),
                crate::ui::prefs::PrefAction::ChangeProjects => {
                    self.ask_for_file(FileAsk::ProjectsFolder);
                }
                crate::ui::prefs::PrefAction::ChooseRecordings => {
                    self.ask_for_file(FileAsk::RecordingsFolder);
                }
                crate::ui::prefs::PrefAction::RecordingsInProject => {
                    self.prefs.set_recordings_dir(None);
                }
                crate::ui::prefs::PrefAction::ClearRecent => self.prefs.clear_recent(),
                crate::ui::prefs::PrefAction::ShowPreferencesFile => {
                    self.prefs.write_if_missing();
                    if let Some(file) = self.prefs.path() {
                        crate::platform::files::reveal_file(file.to_path_buf());
                    }
                }
                crate::ui::prefs::PrefAction::OpenPreferencesFile => {
                    self.prefs.write_if_missing();
                    if let Some(file) = self.prefs.path() {
                        crate::platform::files::edit_text(file.to_path_buf());
                    }
                }
                crate::ui::prefs::PrefAction::Close => open = false,
            }
        }
        self.prefs_tab = state.tab();
        if open {
            self.prefs_window = Some(state);
        }
    }
}

impl eframe::App for App {
    /// The ground every panel and the canvas stand on, so none of them paints it again.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        crate::ui::theme::clear_color(&self.theme)
    }

    /// Stop the two threads that draw before eframe tears its window down. Every GPU object is
    /// reference-counted and freed once nothing holds it and the GPU is done with it, so
    /// nothing here frees one by hand.
    fn on_exit(&mut self) {
        // **The pictures first, and before anything else.** On Linux their Wayland surfaces
        // are on a `wl_display` the thread *borrowed* from eframe, and each wgpu surface must
        // go before the `wl_surface` under it — so the thread must be joined while the display
        // is still alive. eframe tears the display down when this returns, so anything after
        // this order is a use-after-free. On macOS nothing is borrowed: this lets go of the
        // loop, which closes the windows itself once eframe has exited.
        self.pictures.stop();
        // The file drops' thread borrows the same display, so it goes while that is alive.
        self.filedrop.stop();
        // Then the publisher, which lets go of each Syphon server once its last frame is drawn.
        self.sending.publisher.stop();
        // Then the synth, whose renderer goes with its thread.
        self.link.host_mut().stop();
        self.viewer = None;
        self.paint_timer = None;
    }

    /// What winit does not hear, put into the frame's input before egui reads it.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.feed_file_drags(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let started = std::time::Instant::now();
        // The display's own interval, for every figure measured against a frame: the
        // monitor the window is on, read each frame since a window can be carried to
        // another. A window with no monitor yet keeps the last answer.
        if let Some(mhz) = frame
            .winit_window()
            .and_then(|w| w.current_monitor())
            .and_then(|m| m.refresh_rate_millihertz())
            .filter(|mhz| *mhz > 0)
        {
            self.meters.refresh_ms = 1_000_000.0 / mhz as f32;
        }
        // The rate the synth ticks at: the display's, or whatever the preference names
        // instead. Sent only when it changes — see `Host::set_interval`.
        let want = self
            .prefs
            .get()
            .tick_rate
            .interval_ms(self.meters.refresh_ms);
        self.meters.tick_budget_ms = want;
        self.link.host_mut().set_interval(want);
        // Everything the last tick left, and the editor's housekeeping around it. At the
        // top of the frame, so every panel below draws from one tick rather than from
        // whichever one landed while it was being laid out.
        self.push_inputs();
        // One step, **where the synth is on this thread** — egui_kittest, and any host with
        // no GPU to draw on. Under the thread this does nothing: it steps itself,
        // on its own timer, whether or not this frame ever happens. That is the whole point.
        self.link
            .host_mut()
            .step(crate::synth::Beat::Wall(ui.ctx().input(|i| i.time)));
        self.take_snapshot();
        self.link.latch_published();
        // The editor's own interval, off egui's input clock: this is the only place it is
        // measured, because the synth's clock now describes the tick and not the frame.
        {
            let now = ui.ctx().input(|i| i.time);
            let interval = self
                .meters
                .frame_last
                .map_or(0.0, |last| (now - last).max(0.0) as f32);
            self.meters.frame_last = Some(now);
            // A running mean with a time constant of a second: readable, and still honest
            // about a stall within a second of it. The worst is kept for two seconds, the
            // window an Output's GPU time uses, so the two read the same way.
            self.meters.frame.record(interval * 1000.0, interval);
            let blend = interval.clamp(0.0, 1.0);
            self.meters.tick_avg_ms +=
                (self.clock().dt() * 1000.0 - self.meters.tick_avg_ms) * blend;
            self.meters.cpu_avg_ms += (self.meters.cpu_ms - self.meters.cpu_avg_ms) * blend;
        }
        if let Some(viewer) = &self.viewer {
            viewer.open_frame(ui.ctx());
        }
        let timing_paint = self.mark_paint(ui.ctx(), PaintMark::Start);
        // The window's close button, held until the question is answered. `CancelClose` is
        // the only way to keep a window that eframe has already been told to close.
        if ui.ctx().input(|i| i.viewport().close_requested()) && !self.closing && self.must_ask() {
            ui.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.waiting.is_none() {
                self.confirm = Some((Pending::Quit, None));
            }
        }
        // A Quit, an Open or a New the question let go on, once the render it canceled has
        // ended.
        self.after_render(ui);
        // What the tick already did with MIDI, and the editor's half of it: the monitor, a
        // control being learned, and the writes a bound knob made going through the bus so
        // the document and the undo history catch up. The acting happened inside the tick.
        self.take_midi(ui.ctx().input(|i| i.time));

        // One picture a frame, wherever the frame is going: the project tab's cards and the
        // canvas's file picker draw the same textures, so neither can be the thing that
        // decides they get read.
        self.media
            .load_one_picture(ui.ctx(), self.doc.graph(), &self.project);

        // `Escape` while a control waits for a MIDI message stops the wait and binds nothing.
        // Consumed before any shortcut, popup, field or scrub reads the key, so it means only
        // this on the frame it lands.
        if self.midi.learning().is_some()
            && ui.input(|i| i.viewport().focused.unwrap_or(true))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.midi.cancel_learn(&self.link);
        }
        // What was chosen from a native menu bar since the last frame. **The clipboard three
        // come back as the chords they stand for**: AppKit took `⌘C` for the Copy entry
        // before egui-winit could turn it into `Event::Copy`, so a text field holding the
        // keyboard would otherwise lose its copy to the nodes. Replayed as that event, it
        // meets the guard below exactly as the key would have.
        let chosen: Vec<_> = self
            .native_menu
            .as_ref()
            .map(crate::platform::menu::Bar::take)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|tag| self.native_actions.get(tag).cloned())
            .collect();
        for action in chosen {
            let chord = match action {
                menu::MenuAction::Clip(crate::ui::ClipAction::Copy(_)) => egui::Event::Copy,
                menu::MenuAction::Clip(crate::ui::ClipAction::Cut(_)) => egui::Event::Cut,
                menu::MenuAction::Clip(crate::ui::ClipAction::Paste { .. }) => egui::Event::Paste(
                    self.native_menu
                        .as_ref()
                        .map(crate::platform::menu::Bar::pasteboard)
                        .unwrap_or_default(),
                ),
                action => {
                    self.handle_menu(ui, action);
                    continue;
                }
            };
            ui.input_mut(|i| i.events.push(chord));
        }
        // Consumed before anything else looks at input, so a shortcut is never also read as
        // a keystroke by a widget below. Each is the menu entry it stands for.
        for action in ui.input_mut(menu::shortcuts) {
            self.handle_menu(ui, action);
        }
        self.switch_tabs_by_key(ui);
        self.poll_file_dialog();
        self.take_dropped_files(ui);
        // Bare keys, so never while a text field has the keyboard: `H` in a search box is
        // a letter. `Modifiers::NONE` as the pattern rejects a press with `Ctrl` held, so
        // neither is also some shortcut's key. And never while one of our *other* windows has
        // the focus: `F` in a popped-out picture is that picture's fullscreen, and the editor
        // taking it as well would fullscreen the very thing the picture was popped out of.
        let editor_focused = ui.input(|i| i.viewport().focused.unwrap_or(true));
        // The clipboard three, **guarded where Ctrl+Z is not**. Undo is consumed above
        // whatever has the keyboard, because `Ctrl+Z` in a text field is an undo of the
        // graph either way. `Ctrl+C`, `Ctrl+X` and `Ctrl+V` are not: every `TextEdit` in the
        // app — a hex field, a typed number, a note, a rename, the browser's search — answers
        // them itself and none of them guards itself, so consuming them here would take the
        // clipboard away from every field there is. The editor's own focus is the second
        // condition, for the reason `H` and `F` have it: a popped-out picture holding the
        // keyboard is not the editor.
        if editor_focused && !ui.ctx().egui_wants_keyboard_input() {
            // **Not `consume_shortcut`.** `egui-winit` never delivers these three as key
            // presses: it recognizes the clipboard chords itself, pushes `Event::Copy`,
            // `Event::Cut` or `Event::Paste` and returns *before* pushing the `Event::Key`
            // every other shortcut is matched against. So a `KeyboardShortcut` for `Ctrl+C`
            // can never fire in a real window — only under a test harness that synthesizes
            // the key event directly, which is how this shipped broken with a passing test.
            // `keys::COPY` and friends survive as the text the menu prints beside the entry.
            //
            // Taken out of the queue rather than read, for the reason the guard above
            // exists: an `Event::Paste` left in place would also be answered by whatever
            // `TextEdit` comes next.
            let (copy, cut, paste) = ui.input_mut(|i| {
                let (mut copy, mut cut, mut paste) = (false, false, false);
                i.events.retain(|event| match event {
                    egui::Event::Copy => {
                        copy = true;
                        false
                    }
                    egui::Event::Cut => {
                        cut = true;
                        false
                    }
                    egui::Event::Paste(_) => {
                        paste = true;
                        false
                    }
                    _ => true,
                });
                (copy, cut, paste)
            });
            let selection = self.workspace_selection();
            if copy || cut {
                let action = if cut {
                    crate::ui::ClipAction::Cut(selection)
                } else {
                    crate::ui::ClipAction::Copy(selection)
                };
                self.clipboard_action(action);
                // The second half of the winit story: `Event::Paste` is pushed **only when
                // the system clipboard holds text**. A node clip lives in memory, so without
                // this the very next `Ctrl+V` would produce no event at all and the paste
                // would look broken. Saying what was taken also makes the chord mean
                // something outside the app, which is what a person pressing it expects.
                if let Some(said) = self.clipboard_summary() {
                    ui.ctx().copy_text(said);
                }
            } else if paste {
                // No point of its own: it lands where the copy was in the window.
                self.clipboard_action(crate::ui::ClipAction::Paste { at: None });
            }
        }
        if editor_focused && !ui.ctx().egui_wants_keyboard_input() {
            // Space pauses and plays, as it does in every player, `H` hides the editor and
            // `F` is fullscreen: bare keys, each the View entry it is printed beside.
            for action in ui.input_mut(menu::bare_keys) {
                self.handle_menu(ui, action);
            }
        }
        let selection = self.workspace_selection();
        let menu_state = MenuState {
            undo: self.undo_name(),
            redo: self.redo_name(),
            file_busy: self.media.file_busy(),
            playing: self.transport_state().playing,
            rendering: self.rendering(),
            editor_hidden: self.show.hidden,
            fullscreen: ui.input(|i| i.viewport().fullscreen.unwrap_or(false)),
            zoom: self.canvas.transform.zoom,
            // The readout's reading, off the last tick, while the preference shows it.
            time: self.prefs.get().show_time.then(|| menu::Time {
                report: self.transport_state(),
                enabled: !self.rendering(),
            }),
            show_costs: self.prefs.get().show_costs,
            // The two rates as a second's running mean, off the same figures the Status box
            // reads, so the meter and the box never disagree.
            fps: self.prefs.get().show_fps.then(|| {
                let synth = 1000.0 / self.meters.tick_avg_ms.max(0.001);
                menu::Fps {
                    synth,
                    editor: 1000.0 / self.meters.frame.avg.max(0.001),
                    short: crate::ui::status::short_of(
                        synth,
                        1000.0 / self.meters.tick_budget_ms.max(0.001),
                    ),
                }
            }),
            problems: self.problem_count(),
            // The selection, narrowed to the workspace being looked at. The canvas holds
            // one selection across the project, so a node selected on another workspace
            // would otherwise be deleted by a menu naming a node nobody can see.
            selection: selection.clone(),
            clipboard: self.can_paste(),
            any_expanded: selection
                .iter()
                .any(|id| self.doc.graph().get(*id).is_some_and(|n| !n.collapsed)),
            workspace: self
                .active_workspace()
                .and_then(|id| self.doc.graph().workspace(id))
                .map(|w| (w.name.as_str(), w.layout)),
            recent: &self.prefs.get().recent,
        };
        // The operating system's bar where there is one, answered at the top of the next
        // frame; the egui bar otherwise, answered below.
        let mut menu_actions = Vec::new();
        let fps = menu_state.fps;
        let problems = menu_state.problems;
        match &mut self.native_menu {
            Some(bar) => {
                self.native_actions.clear();
                bar.show(&native_menus(
                    &menu::model(&menu_state),
                    &mut self.native_actions,
                ));
            }
            None => {
                egui::Panel::top("menubar").show(ui, |ui| {
                    menu_actions = menu::show(ui, &menu_state, &self.theme);
                });
            }
        }
        let native_menu = self.native_menu.is_some();

        // A row above the canvas, below the menu. Drawn from the graph's workspace list and
        // the project's session; it returns actions and mutates neither.
        let mut tab_actions = Vec::new();
        let mut asked = None;
        let mut badge = false;
        let time = menu_state.time;
        // No margin under the tabs: a tab is joined to the canvas below it, and the panel's
        // default two points of air under the row read as a gap between them.
        let tabs_frame = egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 2,
            bottom: 0,
        });
        egui::Panel::top("tabs").frame(tabs_frame).show(ui, |ui| {
            // The bar's own state out of `self` while the bar borrows the rest of it.
            let mut state = std::mem::take(&mut self.tabs);
            ui.horizontal(|ui| {
                tab_actions = tabs::show(ui, &mut state, &self.tab_bar(), &self.theme);
                // The frame-rate meter's and the time readout's place when the bar they stand
                // at the end of is the operating system's, which draws nothing of ours.
                if native_menu {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(fps) = fps {
                            menu::meter(ui, fps, &self.theme);
                            ui.add_space(8.0);
                        }
                        if let Some(time) = time {
                            asked = crate::ui::timecode::show(
                                ui,
                                time.report,
                                time.enabled,
                                &self.theme,
                            );
                        }
                        ui.add_space(8.0);
                        badge = crate::ui::problems::badge(ui, problems, &self.theme);
                    });
                }
            });
            self.tabs = state;
        });
        if let Some(command) = asked {
            self.transport(command);
        }
        if badge {
            self.toggle_problems();
        }

        // The render, said loudly: a band under the tabs and over every workspace, since the
        // document is closed and the frame thread is rendering rather than performing.
        if let Some(view) = self.render_view() {
            let destination = self
                .media
                .render_destination()
                .and_then(|d| d.strip_prefix(self.project.root()).ok())
                .map_or_else(String::new, |d| d.display().to_string());
            let mut cancel = false;
            egui::Panel::top("render-banner")
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    cancel = crate::ui::banner::show(ui, &view, &destination, &self.theme);
                });
            if cancel {
                self.cancel_render();
            }
        }

        for action in menu_actions {
            self.handle_menu(ui, action);
        }
        // Answered fresh each frame: the pointer leaving the bar has to stop offering a drop.
        self.drop_target = None;
        for action in tab_actions {
            self.handle_tab(action);
        }

        // The canvas as of last frame, in pixels, for a viewport-matched mix. Only a frame
        // that had a canvas updates it, so the project tab leaves the mix its size. The first
        // size is taken at once; after that, a size is taken once the canvas has held it for
        // `MIX_SETTLE_S`, so a drag resizes the mix once, where it stops.
        let scale = ui.ctx().pixels_per_point();
        let canvas_px = (
            (self.canvas.width() * scale).round() as u32,
            (self.canvas.height() * scale).round() as u32,
        );
        if canvas_px.0 > 0 && canvas_px.1 > 0 {
            let now = ui.ctx().input(|i| i.time);
            let since = match self.mix_canvas {
                None => {
                    self.mix_viewport = canvas_px;
                    now
                }
                Some((seen, since)) if seen == canvas_px => since,
                Some(_) => now,
            };
            self.mix_canvas = Some((canvas_px, since));
            if canvas_px != self.mix_viewport {
                let left = MIX_SETTLE_S - (now - since);
                if left <= 0.0 {
                    self.mix_viewport = canvas_px;
                } else {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_secs_f64(left));
                }
            }
        }
        // After the menu bar, so a node added this frame is in the plan. Nothing is drawn
        // here: the renderer is the synth's, on a thread and a context of its own, and
        // every window below is a viewer blitting what its last tick published. See
        // docs/rendering.md.
        self.publish_plan();
        self.show_status_box(ui.ctx());

        // The Main Input, on the left: one video source and one audio source for the whole
        // rig. It draws its own fold, as the mixer does.
        let mut input_actions = Vec::new();
        let input_collapsed = self.prefs.get().main_input_collapsed;
        let input_width = self.prefs.get().main_input_width;
        let input_resized = side_panel(
            ui,
            crate::ui::panel::Side::Left,
            input_collapsed,
            ("main-input", "main-input-spine"),
            input_width.unwrap_or(300.0),
            |ui| {
                // Read before the view borrows `self`, which is what keeps the panel a pure
                // draw-and-return with nothing of the app's mutable in its hand.
                let lock_cursor = self.prefs.get().lock_cursor_while_scrubbing;
                let theme = self.theme;
                let view = self.main_input_view();
                let panel = crate::ui::maininput::show(ui, &view, &theme, lock_cursor);
                if let Some(thumb) = &panel.preview {
                    self.blit_into(ui, thumb);
                }
                input_actions = panel.actions;
            },
        );
        if let Some(width) = input_resized {
            self.prefs.set_main_input_width(width);
        }
        for action in input_actions {
            self.handle_main_input(action);
        }

        // The two surfaces the editor draws itself are answered fresh each frame, the way
        // the tab bar's drop target is: a panel that folded away or a canvas that closed
        // says nothing rather than holding the last place the hand was, and whichever of
        // them is drawn below reports again. See `crate::pointer`.
        self.set_pointer(crate::pointer::Surface::Preview, None);
        self.set_pointer(crate::pointer::Surface::Canvas, None);

        let mut mixer_actions = Vec::new();
        let mixer_collapsed = self.prefs.get().mixer_collapsed;
        let mixer_width = self.prefs.get().mixer_width;
        let mixer_resized = side_panel(
            ui,
            crate::ui::panel::Side::Right,
            mixer_collapsed,
            ("preview", "preview-spine"),
            mixer_width.unwrap_or(380.0),
            |ui| {
                // The Main Mixer: the decks, the fade and the method over the mix. It draws
                // and returns; the pictures are slots it reserved, filled here the way a
                // node's on-body render is.
                let view = self.mixer_view();
                let panel = crate::ui::mixer::show(
                    ui,
                    &view,
                    &self.theme,
                    self.prefs.get().lock_cursor_while_scrubbing,
                );
                for thumb in &panel.previews {
                    self.blit_into(ui, thumb);
                }
                mixer_actions = panel.actions;
                // No preview while folded: the frame is already drawn, so a folded panel is a
                // picture nobody is looking at and nothing else. No rule over it: it is the
                // Projection section's picture, what the rows above it project.
                if !mixer_collapsed {
                    self.preview(ui, self.link.plan().display);
                }
            },
        );
        if let Some(width) = mixer_resized {
            self.prefs.set_mixer_width(width);
        }
        for action in mixer_actions {
            match action {
                crate::ui::mixer::MixerAction::SetBalance(v) => self.set_balance(v),
                // The fade, Blackout and Freeze learn as a node's control does: the next
                // message binds to it, and it wears the learning ring until one does.
                crate::ui::mixer::MixerAction::LearnMidi(target) => self.learn_midi(target),
                crate::ui::mixer::MixerAction::SetBlackout(on) => self.set_blackout(on),
                crate::ui::mixer::MixerAction::SetFreeze(on) => self.set_freeze(on),
                crate::ui::mixer::MixerAction::SetMethod(m) => self.set_method(m),
                crate::ui::mixer::MixerAction::SetResolution(r) => self.set_mix_resolution(r),
                crate::ui::mixer::MixerAction::PopOut(req) => self.pop_out(req),
                crate::ui::mixer::MixerAction::SetBackground(on) => self.set_background(on),
                crate::ui::mixer::MixerAction::Syphon(on) => self.sending.mix.syphon = on,
                crate::ui::mixer::MixerAction::Ndi(on) => self.sending.mix.ndi = on,
                crate::ui::mixer::MixerAction::GoTo { workspace, node } => {
                    self.navigate_to(workspace, node);
                }
                crate::ui::mixer::MixerAction::SetCollapsed(on) => {
                    self.prefs.set_mixer_collapsed(on);
                }
            }
        }

        let central = egui::CentralPanel::default_margins().show(ui, |ui| {
            // The project tab replaces the canvas. The preview panel stays either way, so an
            // Output goes on rendering while the project is being looked at.
            let Some(workspace) = self.active_workspace() else {
                self.show_project_tab(ui);
                return;
            };
            // `H`: nothing but the mix. The buttons the canvas would have reported are
            // not on screen, so none is held.
            if self.show.hidden {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
                if let Some(callback) = self.mix_cover(rect) {
                    ui.painter().add(callback);
                }
                // The pointer's half only. `H` hides the editor to perform behind it, which
                // is exactly when a note held on a controller must not be let go of.
                self.set_pointer_held(std::collections::HashSet::new());
                return;
            }
            // Everything the canvas draws that came out of the tick rather than out of the
            // graph: the notes, the scopes, the playheads, the traces and what was published.
            // Read before the snapshot is taken out from under `faults`.
            let faults: std::collections::HashMap<_, _> = self.faults().into_iter().collect();
            // The Outputs' rows read the recordings out of the snapshot, so before it is taken.
            let readouts = self.output_readouts();
            let snapshot = std::mem::take(self.link.snapshot_mut());
            // One reading of the probes' counts, thresholded for the header warning and
            // formatted for the strip.
            let taps = self.link.taps_by_node();
            let costs = if self.prefs.get().show_costs {
                self.link
                    .cost_strips(self.doc.graph(), &taps, self.meters.refresh_ms)
            } else {
                std::collections::HashMap::new()
            };
            let sampling = super::link::SynthLink::sampling_warnings(self.doc.graph(), &taps);
            let render_view = self.render_view();
            let live = self.live_sources();
            let can_paste = self.can_paste();
            let prefs = self.prefs.get();
            // What a loop of each Master Gear on screen needs, read off the chains below it.
            let gear_captions = crate::nodes::chain::captions(self.doc.graph(), |id| {
                self.doc
                    .graph()
                    .get(id)
                    .is_some_and(|n| n.workspaces.contains(&workspace))
            });
            let frame = crate::ui::CanvasFrame {
                graph: self.doc.graph(),
                workspace,
                notes: &snapshot.notes,
                scopes: &snapshot.scopes,
                playheads: &snapshot.playheads,
                traces: &snapshot.traces,
                captions: &snapshot.captions,
                curves: &snapshot.curves,
                pucks: &snapshot.pucks,
                gears: &snapshot.gears,
                gear_captions: &gear_captions,
                // A borrowed view of what the last tick published, so a uniform number port
                // and a connected control show the value rather than the last one stored.
                uniforms: crate::synth::Uniforms::of(&snapshot),
                thumbs: &snapshot.thumbs,
                tabs: &self.project.session().open,
                assets: self.media.assets(),
                posters: self.media.asset_thumbnails(),
                theme: &self.theme,
                project_background: self.project.mixer().background && self.has_gpu,
                cleared: self.ground == super::Ground::Cleared,
                costs: &costs,
                readouts: &readouts,
                bindings: self.project.midi(),
                learning: self.midi.learning(),
                ghosts: &snapshot.midi_ghosts,
                sampling: &sampling,
                faults: &faults,
                live: &live,
                popped: self.wall.open(),
                render: render_view,
                clipboard: can_paste,
                nodes_menu_open: self.start.is_open(),
                prefs: crate::ui::CanvasPrefs {
                    lock_cursor: prefs.lock_cursor_while_scrubbing,
                    scroll_x_inverted: prefs.scroll_x_inverted,
                    port_hover_highlight: prefs.port_hover_highlight,
                    cable_droop: prefs.cable_droop,
                    phi_cables: prefs.phi_cables,
                    node_shadow: prefs.node_shadow,
                },
            };
            let effects = crate::ui::show(ui, &mut self.canvas, &frame);
            *self.link.snapshot_mut() = snapshot;
            // The canvas is the whole of its own rect rather than a picture inside one, so
            // its aspect is the rect's and there are no bars to fall on — which is what
            // silvia's one surface was.
            let rect = ui.max_rect();
            let aspect = rect.width() / rect.height().max(1.0);
            self.report_pointer(ui, crate::pointer::Surface::Canvas, rect, aspect);
            self.apply_canvas(ui, effects);

            // The start button, in the canvas's own bottom-left corner and floating over it.
            // Drawn after the canvas so it is above the nodes, and only where there is a
            // canvas: on the project tab there is nothing for a node to land on.
            let canvas = egui::Rect::from_min_size(
                self.canvas.origin(),
                egui::vec2(self.canvas.width(), self.canvas.height()),
            );
            if let Some(slug) = crate::ui::start::show(ui, &mut self.start, canvas, &self.theme) {
                self.add_node_at_drop(slug);
            }
            // One menu at a time: whichever opened last closes the other.
            if self.start.take_just_opened() {
                self.canvas.close_browser();
            } else if self.canvas.take_browser_just_opened() {
                self.start.close();
            }
            // No menu at all while a question is up. A modal's backdrop stops the pointer
            // reaching the canvas, but `n` and `/` are read inside it, so a list that opens
            // under the question is closed on the frame it opened.
            if self.dialog_up() {
                self.start.close();
                self.canvas.close_browser();
            }
        });
        // Over the canvas or the project tab, while files are held over the window.
        self.show_drop_hint(ui.ctx(), central.response.rect);

        if self.offer_arrange {
            let mut answered = None;
            egui::Modal::new(egui::Id::new("offer-arrange")).show(ui.ctx(), |ui| {
                ui.label("Arrange to fit the strip?");
                ui.label("Some nodes sit outside a linear workspace's height.");
                ui.horizontal(|ui| {
                    if ui.button("Arrange").clicked() {
                        answered = Some(true);
                    }
                    if ui.button("Leave them").clicked() {
                        answered = Some(false);
                    }
                });
            });
            match answered {
                // Workspace ▸ Auto-arrange: its own undo step, so it can be taken back without
                // undoing the switch, and it takes the offer down.
                Some(true) => self.handle_menu(ui, menu::MenuAction::AutoArrange),
                Some(false) => self.offer_arrange = false,
                None => {}
            }
        }

        self.confirm_window(ui);
        self.project_name_window(ui.ctx());
        self.crashlog_frame(ui);
        self.recovery_window(ui);
        self.show_preferences(ui.ctx());
        self.show_midi(ui.ctx());
        self.show_about(ui.ctx());
        self.show_licences(ui.ctx());
        self.show_shortcuts(ui.ctx());
        self.show_undo_history(ui.ctx());
        self.poll_pictures();
        self.send_out();
        self.show_toast(ui.ctx());
        self.show_problems(ui.ctx());

        // The project's folder name, with a marker while there are unsaved edits. Sent only
        // when it changes: a viewport command a frame is a command a frame.
        let title = format!(
            "{}{} — supersilvia",
            self.project.name(),
            if self.dirty() { " •" } else { "" }
        );
        if title != self.title {
            self.title.clone_from(&title);
            ui.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        self.record_window_geometry(ui.ctx());
        // The text size times the View zoom, as this frame's menus and Preferences left them:
        // egui takes it at the start of the next frame, as it would its own zoom keys.
        ui.ctx().set_zoom_factor(self.prefs.get().scale());
        // One write per frame at most, and only when something moved.
        self.prefs.flush();
        // After every edit this frame made.
        self.autosave_tick(ui.ctx().input(|i| i.time));

        if timing_paint {
            self.mark_paint(ui.ctx(), PaintMark::End);
        }
        if let Some(viewer) = &self.viewer {
            viewer.close_frame(ui.ctx(), self.link.published());
        }
        self.meters.cpu_ms = started.elapsed().as_secs_f32() * 1000.0;

        // **This window is a viewer, and what it must never do is block the thread its
        // other windows paint on.** A minimized window gets no frame callback, so asking its
        // surface for the next texture can wait on the compositor for one that never comes. So a minimized window
        // stops painting: it asks for its next repaint a quarter of a second out instead of
        // a display interval, eframe runs no pass for it, and nothing swaps or blocks. A
        // quarter of a second is short enough that restoring is not a visible wait and long
        // enough to cost nothing measurable.
        //
        // **`pre_present_notify` is not the answer, measured twice.** It gates this window's
        // redraw on its own frame callback, which is the right *shape* — but eframe polls the
        // event loop for the whole time a redraw is held, so asking for it every frame keeps
        // eframe in `ControlFlow::Poll` for the life of the run: **97.9% of a core against
        // 17.2% with the call out**, on Linux with an Intel UHD 770, same window, same graph.
        // Asking for it only while minimized does nothing, because the pass that would ask is
        // the pass that no longer runs. `proposals/pacing.md`'s second finding stands, and it
        // stands for a viewer too.
        //
        // **Nothing else is behind this window any more.** Every picture window is a Wayland
        // surface of our own, on the pictures thread, paced by its own frame callback — so a
        // minimized editor costs its own painting and nothing else's. That is what
        // `proposals/picture-windows.md` bought, and it is why the cost this comment used to
        // name is gone from docs/decisions.md.
        let minimized = ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false));
        // It's a synth: always redraw — but **after** an interval, not at once. A bare
        // `request_repaint` makes eframe poll rather than sleep between frames.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f32(if minimized {
                0.25
            } else {
                self.meters.refresh_ms / 1000.0
            }));
    }
}

impl App {
    /// Time the editor's own painting on the GPU, while the Status box is open: at
    /// [`PaintMark::Start`], fold in what has landed and have the timer mark the start ahead
    /// of every shape, and at [`PaintMark::End`] have it mark the end above every window.
    /// Returns whether a mark was placed, so the end is placed only after a start.
    ///
    /// Closed, nothing is marked, and the reading and every pair still unread are forgotten,
    /// as the synth's phases are: a reopened box's first reading is of a frame after it
    /// opened.
    fn mark_paint(&mut self, ctx: &egui::Context, mark: PaintMark) -> bool {
        let Some(timer) = self.paint_timer.clone() else {
            return false;
        };
        if matches!(mark, PaintMark::Start) {
            self.meters.fold_paint(&timer.take());
            if !self.prefs.get().show_status_box {
                self.meters.paint_gpu = None;
                timer.forget();
                return false;
            }
        }
        timer.mark_on_paint(ctx, mark);
        true
    }

    /// What the Status box shows: every Output with its cost, its state and what it shows,
    /// the file line, the frame, the graph and each CPU node's line.
    pub(super) fn status_view(&self) -> crate::ui::status::StatusView {
        use crate::synth::{Mode, Why};
        use crate::ui::status::{NodeLine, OutputRow, OutputState, StatusView};
        let report = &self.link.snapshot().render;
        let graph = self.doc.graph();
        let outputs = self
            .link
            .plan()
            .outputs
            .iter()
            .map(|o| {
                let id = o.node;
                let state = if !self.has_gpu {
                    OutputState::NoGpu
                } else if let Some(err) = report.errors.get(&id) {
                    OutputState::Error(err.lines().next().unwrap_or(err).to_string())
                } else if let Some(d) = self.link.shader(id).and_then(|s| s.diagnostics.first()) {
                    // A diagnostic outranks the rest: the picture is still on screen, so
                    // nothing else would say the compiler could not resolve part of the graph.
                    OutputState::Diagnostic(d.to_string())
                } else if o.mode == Mode::Suspended {
                    OutputState::Suspended
                } else if o.mode == Mode::Dark {
                    OutputState::NothingConnected
                } else if report.linking.contains(&id) {
                    OutputState::Linking
                } else if o.mode == Mode::Idle {
                    OutputState::Idle
                } else {
                    OutputState::Rendering
                };
                // The GPU's own figure, where one exists. An Output that is suspended or has
                // nothing connected is not drawn, so it has none, and the row says so with
                // a dash rather than a zero that would read as an idle GPU. An idle one keeps
                // the figure of the last frame it drew, which the row mutes.
                let gpu_ms = report
                    .gpu_times
                    .get(&id)
                    .filter(|_| !matches!(o.mode, Mode::Suspended | Mode::Dark))
                    .map(|g| (g.latest, g.worst));
                let name_of = |id: crate::graph::NodeId| {
                    format!("{}{id}", graph.get(id).map_or("output", |n| n.def.slug))
                };
                let why = match o.mode.why() {
                    None => String::new(),
                    Some(Why::Tab) => "it is on the workspace being looked at".to_string(),
                    Some(Why::Deck) => "it is on a mixer deck".to_string(),
                    Some(Why::Window) => "its picture has a window of its own".to_string(),
                    Some(Why::Sent) if crate::platform::syphon::available() => {
                        "it is sent out over Syphon or NDI".to_string()
                    }
                    Some(Why::Sent) => "it is sent out over NDI".to_string(),
                    Some(Why::Capture) => "the render is capturing it".to_string(),
                    Some(Why::Feedback) => "its frame feeds back into its own picture".to_string(),
                    Some(Why::Frame(by)) => format!("{} samples its frame", name_of(by)),
                    Some(Why::Tap(of)) => format!(
                        "{} measures its frame, and something live or stateful reads it",
                        name_of(of)
                    ),
                };
                let node = graph.get(id);
                // What it shows: the node on the cable into its picture, or the Output
                // itself with nothing there.
                let label = node
                    .and_then(|n| n.inputs.first())
                    .and_then(|port| graph.source_of(crate::graph::PortRef::new(id, port.key)))
                    .and_then(|from| graph.get(from.node))
                    .or(node)
                    .map_or("Output", |n| n.def.label)
                    .to_string();
                let workspace = self
                    .home_of(id)
                    .and_then(|w| graph.workspace(w))
                    .map(|w| w.name.clone())
                    .unwrap_or_default();
                OutputRow {
                    node: id,
                    name: format!("{}{id}", node.map_or("output", |n| n.def.slug)),
                    label,
                    workspace,
                    resolution: o.resolution,
                    gpu_ms,
                    dropped_per_s: self.link.drops_per_second(id),
                    state,
                    why,
                }
            })
            .collect();
        let cpu_lines = self
            .link
            .snapshot()
            .cpu_lines
            .iter()
            .map(|(node, name, text)| NodeLine {
                node: *node,
                name: name.clone(),
                kind: graph.get(*node).map_or("Node", |n| n.def.label).to_string(),
                text: text.clone(),
            })
            .collect();
        StatusView {
            file: self.media.status().to_string(),
            file_shows: self.media.status_shows().is_some(),
            file_failed: self.media.status_failed(),
            tick: self.clock().ticks(),
            frame_ms: self.meters.frame.now,
            frame_avg_ms: self.meters.frame.avg,
            frame_worst_ms: self.meters.frame.worst,
            refresh_ms: self.meters.refresh_ms,
            tick_budget_ms: self.meters.tick_budget_ms,
            cpu_ms: self.meters.cpu_ms,
            cpu_avg_ms: self.meters.cpu_avg_ms,
            paint_gpu: self.meters.paint_gpu.map(|g| (g.now, g.avg, g.worst)),
            drops: report.drops.listed(),
            gpu_waits: report.gpu_waits,
            gpu_waits_per_s: self.link.waits_per_second(),
            dropped_per_s: self.link.worst_drops_per_second(),
            uptime_s: self.clock().elapsed(),
            dropped: report.drops.total(),
            drawn: self.canvas.drawn(),
            nodes: graph.len(),
            shaders: self.link.shader_counts().0,
            uniforms: self.link.shader_counts().1,
            undo: self.doc.undo_len(),
            undo_depth: UNDO_DEPTH,
            redo: self.doc.redo_len(),
            outputs,
            outputs_drawn: report.drawn,
            cpu_lines,
            phases: self.link.snapshot().phases.clone(),
            gpu: self.has_gpu,
            busy_of: crate::ui::status::BusyOf::here(),
        }
    }

    /// The Status box: a window of its own, so the mixer panel is the mixer and the canvas
    /// is the canvas. Open while the preference says so; closing it is unticking it. What it
    /// asks — a node shown, a fold, a copy — is answered here.
    fn show_status_box(&mut self, ctx: &egui::Context) {
        let mut open = self.prefs.get().show_status_box;
        if !open {
            return;
        }
        let view = self.status_view();
        let folds = self.prefs.get().status_folds;
        // The box is laid out at the window's inner width in whole characters, and the window
        // opens at a whole count of them and never narrows past the box's least: however it is
        // dragged, the frame's right edge is inside it.
        let (open_width, min_width) = {
            use crate::ui::status::{MIN_WIDTH, OPEN_WIDTH, window_width};
            (window_width(ctx, OPEN_WIDTH), window_width(ctx, MIN_WIDTH))
        };
        let window = egui::Window::new(crate::ui::placed::STATUS)
            .open(&mut open)
            .default_pos(egui::pos2(16.0, 80.0))
            .default_width(open_width)
            .min_width(min_width)
            // The body scrolls inside; see `status_height`.
            .default_height(status_height(ctx))
            .max_height(status_height(ctx))
            .resizable(true);
        // Taller than a screen with every section open, so its sections scroll inside it
        // rather than running off the foot of the window.
        let asked = crate::ui::placed::place(
            ctx,
            window,
            crate::ui::placed::STATUS,
            &self.prefs.get().windows,
        )
        .show(ctx, |ui| {
            crate::ui::status::show(ui, &view, &self.theme, folds)
        })
        .and_then(|r| r.inner);
        if !open {
            self.prefs.set_show_status_box(false);
        }
        let Some(asked) = asked else { return };
        self.prefs.set_status_folds(asked.folds);
        if let Some(text) = asked.copy {
            ctx.copy_text(text);
        }
        if asked.show {
            self.show_status_file();
        }
        // A name clicked is a node to be shown, centred on it, as every link to one is.
        if let Some(node) = asked.go
            && let Some(workspace) = self.home_of(node)
        {
            self.navigate_to(workspace, node);
        }
    }

    /// Everything one canvas pass asked for, done: the one place the canvas's answers reach the
    /// app.
    ///
    /// The order is the frame's: the pictures into the slots the canvas reserved, the requests
    /// the instrument answers, the edits, and then whatever joins an edit to something the
    /// canvas cannot see — a drop on a tab after the drag's own moves, an abandoned scrub after
    /// the step it collapses is on top of the ring.
    fn apply_canvas(&mut self, ui: &mut egui::Ui, out: crate::ui::Effects) {
        // The mix under everything, covering the canvas: the graph floats on the show.
        if let Some(background) = out.background
            && let Some(callback) = self.mix_cover(background.rect)
        {
            ui.painter().set(background.slot, callback);
        }
        // One paint callback per picture on a node, blitting its own published frame into
        // the slot the canvas reserved for it. The same mechanism as the preview.
        for thumb in &out.thumbnails {
            self.blit_into(ui, thumb);
        }
        for request in out.pop_outs {
            self.pop_out(request);
        }
        for request in out.render_requests {
            if request.cancel {
                self.cancel_render();
            } else {
                self.start_render_on(request.node);
            }
        }
        for node in out.record_requests {
            self.toggle_recording(node);
        }
        // Read by the *next* tick, with the held buttons: the UI is drawn after the ticks, so
        // where a hand let the scrubber go lands a frame later, which is one sixteenth of a
        // second and is a scrub.
        if !out.seeks.is_empty() {
            self.link.send(crate::synth::Msg::Seeks(out.seeks));
        }
        if !out.touches.is_empty() {
            self.link.send(crate::synth::Msg::Touches(out.touches));
        }
        for (node, key) in out.file_requests {
            let accepts = self
                .doc
                .graph()
                .get(node)
                .and_then(|n| n.def.option(key))
                .map_or(crate::nodes::Accepts::ANY, |o| o.accepts);
            self.ask_for_file(FileAsk::Option { node, key, accepts });
        }
        // Read by the *next* tick, which runs at the top of the next frame. A press is one
        // frame late and a click is sixteen milliseconds long, so nothing is lost.
        self.set_pointer_held(out.held.into_iter().collect());
        for command in out.commands {
            if let Err(err) = self.apply(command) {
                log::debug!("command refused: {err}");
            }
        }
        // After the drag's own moves, so a drop on a tab is the step after them.
        self.handle_node_drag(out.node_drag);
        if let Some((workspace, node)) = out.navigate {
            self.navigate_to(workspace, node);
        }
        if out.reveal_main_input {
            self.prefs.set_main_input_collapsed(false);
        }
        if out.look_for_devices {
            self.link.send(crate::synth::Msg::ForgetDevices);
        }
        // `Alt` + click on a control: the next message binds to it, and the control wears the
        // learning ring until one does.
        if let Some(target) = out.learn_midi {
            self.learn_midi(target);
        }
        if let Some(action) = out.clip {
            self.clipboard_action(action);
        }
        if let Some(target) = out.unbind_midi {
            self.project.midi_mut().unbind_target(target);
            self.publish_midi();
        }
        // After the drag's own commands: the frame Escape lands on is the frame the step it
        // collapses is on top of the ring.
        if let Some((node, key, value)) = out.cancel_control {
            self.cancel_control_drag(node, key, value);
        }
        if let Some(nodes) = out.cancel_node_drag {
            self.cancel_node_drag(&nodes);
        }
        // Letting go ends the gesture, so two scrubs of one control are two undo steps rather
        // than one that walks back both. The **release**, not the button being up: a key
        // repeat holds its gesture the way a drag does, and has no release to end it. After
        // this frame's commands, because the release frame is still theirs.
        if ui.ctx().input(|i| i.pointer.any_released()) {
            self.end_gesture();
        }
    }

    /// Blit one Output's frame into the slot the canvas reserved for it.
    ///
    /// `Painter::set` rather than `add`: the slot was reserved inside the node's own paint
    /// order, between the screen's black ground and the border, so the border is painted over
    /// the picture rather than under it. `Thumbnail::corner` is in points, which is why it is
    /// scaled by `pixels_per_point` here; the arc itself is the renderer's.
    fn blit_into(&self, ui: &mut egui::Ui, thumb: &crate::ui::Thumbnail) {
        let Some(viewer) = &self.viewer else {
            return;
        };
        ui.painter().set(
            thumb.slot,
            viewer.node_callback(
                self.link.published(),
                thumb.rect,
                thumb.node,
                thumb.port,
                thumb.fit,
                thumb.corner,
            ),
        );
    }

    /// Where the pointer is over one of the editor's own picture surfaces, for the graph.
    ///
    /// egui's pointer, against `rect`, placed inside a picture of `aspect` letterboxed into
    /// it — the same placement a picture window's `wl_pointer` goes through, so the two
    /// sources say the same thing about the same hand. Read by the *next* tick, like a
    /// press: the UI is drawn after the ticks, which is one sixteenth of a second.
    fn report_pointer(
        &self,
        ui: &egui::Ui,
        surface: crate::pointer::Surface,
        rect: egui::Rect,
        aspect: f32,
    ) {
        let reading = ui.ctx().input(|i| {
            // A pointer that has left the window entirely: `latest_pos` remembers where it
            // went out, and where it went out is not where it is.
            if !i.pointer.has_pointer() {
                return None;
            }
            let pos = i.pointer.latest_pos()?;
            let placed = crate::pointer::place(
                (pos.x - rect.left(), pos.y - rect.top()),
                (rect.width(), rect.height()),
                aspect,
            )?;
            Some(crate::pointer::Reading {
                left: i.pointer.button_down(egui::PointerButton::Primary),
                right: i.pointer.button_down(egui::PointerButton::Secondary),
                ..placed
            })
        });
        self.set_pointer(surface, reading);
    }

    /// The preview: the mix, 16:9 across the panel.
    ///
    /// A viewer like every other picture in the app: the frame was drawn on the synth's
    /// context before this panel was laid out and has finished there, so what is here is a
    /// blit over the screen's black and nothing to wait for. Folding the panel away hides a
    /// picture and nothing else.
    fn preview(&mut self, ui: &mut egui::Ui, display: crate::render::Display) {
        let width = ui.available_width().max(1.0);
        let size = egui::vec2(width, width * 9.0 / 16.0);
        let rect = ui.allocate_exact_size(size, egui::Sense::hover()).0;
        // Against the picture the panel is showing, which is letterboxed into that rect:
        // the mix, or the one Output a solo preview is on.
        let shown = match display {
            crate::render::Display::Nothing => None,
            crate::render::Display::Output(node) => self.link.published().picture(node, None),
            crate::render::Display::Mixer => self.link.published().mixer.clone(),
        };
        let aspect = shown.map_or(0.0, |p| p.width as f32 / p.height.max(1) as f32);
        self.report_pointer(ui, crate::pointer::Surface::Preview, rect, aspect);

        let Some(viewer) = &self.viewer else {
            ui.painter()
                .rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
            return;
        };
        // The screen's black ground under the picture, as every picture box has: the picture
        // is premultiplied and shown over black, its bars black with it.
        if display != crate::render::Display::Nothing {
            ui.painter().rect_filled(rect, 0.0, self.theme.screen_off());
        }
        let published = self.link.published();
        let fit = crate::render::Fit::Letterbox;
        match display {
            crate::render::Display::Nothing => {}
            crate::render::Display::Output(node) => {
                ui.painter()
                    .add(viewer.node_callback(published, rect, node, None, fit, 0.0));
            }
            crate::render::Display::Mixer => {
                ui.painter()
                    .add(viewer.mixer_callback(published, rect, fit));
            }
        }
    }
}

/// One of the two side panels: folded to a spine, or open at `open_size` and resizable, and
/// scrolled either way, because a panel is taller than a short window and a select under the
/// bottom edge is one nobody can open.
///
/// **Returns the width a hand just left it at**, on the frame a drag of its edge lets go, for
/// the preference that opens it there next run: egui remembers it within a run and keeps
/// nothing across runs, since eframe's own persistence is off. Only a release that moved it
/// counts, so a window too narrow for the panel squeezes it without losing the width it is
/// kept at.
///
/// **Two ids, one per state**, because egui remembers a panel's width by id: one id used both
/// ways comes back from its folded turn remembering the spine's width, and `default_size` is
/// only consulted the first time an id is seen — which is how a panel once unfolded to
/// twenty-four points with every control crammed into it. `resizable(false)` on the spine: a
/// panel is resizable by default, and `exact_size` only pins the range it may take, so the
/// edge would still light and offer a resize cursor for a drag that can move nothing.
fn side_panel(
    ui: &mut egui::Ui,
    side: crate::ui::panel::Side,
    collapsed: bool,
    (open_id, spine_id): (&'static str, &'static str),
    open_size: f32,
    add: impl FnOnce(&mut egui::Ui),
) -> Option<f32> {
    use egui::containers::panel::PanelState;
    let id = if collapsed { spine_id } else { open_id };
    // A width that is not one opens at the panel's own.
    let open_size = if open_size.is_finite() && open_size > 0.0 {
        open_size
    } else {
        300.0
    };
    let panel = match side {
        crate::ui::panel::Side::Left => egui::Panel::left(id),
        crate::ui::panel::Side::Right => egui::Panel::right(id),
    };
    let panel = if collapsed {
        panel.exact_size(crate::ui::panel::SPINE).resizable(false)
    } else {
        panel.default_size(open_size).resizable(true)
    };
    let width = |ui: &egui::Ui| PanelState::load(ui.ctx(), egui::Id::new(id)).map(|p| p.size().x);
    let before = width(ui);
    panel.show(ui, |ui| {
        egui::ScrollArea::vertical().show(ui, add);
    });
    let after = width(ui)?;
    let moved = before.is_some_and(|b| (b - after).abs() > 0.5);
    (!collapsed && moved && ui.input(|i| i.pointer.any_released())).then_some(after)
}

/// How tall the Status box opens, and the most it may be dragged to: two thirds of the
/// screen, so it never reaches the foot of the window however many rows it holds.
fn status_height(ctx: &egui::Context) -> f32 {
    (ctx.content_rect().height() * 2.0 / 3.0).max(200.0)
}

/// The menus as the operating system's bar takes them: plain data, with each entry's action
/// pushed onto `actions` at the tag it is given.
fn native_menus(
    menus: &[menu::Menu],
    actions: &mut Vec<menu::MenuAction>,
) -> Vec<crate::platform::menu::Menu> {
    menus
        .iter()
        .map(|m| native_menu(m.title, m.hint.clone(), &m.items, actions))
        .collect()
}

fn native_menu(
    title: &str,
    hint: Option<String>,
    items: &[menu::Item],
    actions: &mut Vec<menu::MenuAction>,
) -> crate::platform::menu::Menu {
    use crate::platform::menu as native;
    let items = items
        .iter()
        .map(|item| match item {
            menu::Item::Separator => native::Item::Separator,
            menu::Item::Submenu { title, items } => {
                native::Item::Submenu(native_menu(title, None, items, actions))
            }
            menu::Item::Entry(entry) => native::Item::Entry(native::Entry {
                label: entry.label.clone(),
                enabled: entry.enabled,
                // Only a chord with Command held is a key equivalent: AppKit takes a menu's
                // key before the window sees it, so a bare `H` there would be no letter in
                // any field, and `F1` and `F8` are no character it can match.
                key: entry
                    .shortcut
                    .filter(|s| s.modifiers.command || s.modifiers.mac_cmd)
                    .and_then(|s| {
                        let character = match s.logical_key {
                            egui::Key::Plus => "+".to_owned(),
                            egui::Key::Minus => "-".to_owned(),
                            egui::Key::Slash => "/".to_owned(),
                            key => key.name().to_lowercase(),
                        };
                        (character.chars().count() == 1).then_some(native::Key {
                            character,
                            command: true,
                            shift: s.modifiers.shift,
                            alt: s.modifiers.alt,
                            ctrl: s.modifiers.ctrl,
                        })
                    }),
                ticked: match entry.mark {
                    menu::Mark::None => None,
                    menu::Mark::Check(on) | menu::Mark::Radio(on) => Some(on),
                },
                // While it is greyed out, its hover says why.
                hint: entry.hover().map(str::to_owned),
                role: match entry.action {
                    Some(menu::MenuAction::OpenPreferences) => native::Role::Settings,
                    Some(menu::MenuAction::Quit) => native::Role::Quit,
                    Some(menu::MenuAction::OpenAbout) => native::Role::About,
                    Some(menu::MenuAction::OpenLicences) => native::Role::Licences,
                    _ => native::Role::Plain,
                },
                tag: entry.action.clone().map(|action| {
                    actions.push(action);
                    actions.len() - 1
                }),
            }),
        })
        .collect();
    native::Menu {
        title: title.to_owned(),
        hint,
        items,
    }
}
