// SPDX-License-Identifier: AGPL-3.0-or-later

//! The performance surface: the picture windows, and the toasts.
//!
//! None of it is saved and none of it is undoable — which picture is on which screen is
//! decided on the night.
//!
//! **There is no projector.** Every picture window is one kind of thing: a node's render, a
//! source's frame and the mix alike are a [`crate::ui::PopOut`] with a window of its own,
//! opened by the same pair of marks and closed the same way. What opens and paints them is
//! [`crate::render::picture`], on a thread of its own — so a minimized editor is not
//! something a picture window can observe. `proposals/picture-windows.md` is the argument.
//!
//! What `App` keeps here is the *wanting*: which pictures should have windows, which the
//! marks read, and the asks that cross to the thread.

use super::App;
use crate::render::picture::Told;
use eframe::egui;

/// How big a picture window opens when its picture has published nothing yet: a 16:9 window
/// half a 1080p row wide, which is what the projector opened at.
const DEFAULT_SIZE: (u32, u32) = (960, 540);

/// The performance surface's own state. App state, none of it saved: a window is not an
/// edit, and which display a rig is plugged into is decided on the night.
#[derive(Debug, Default)]
pub(super) struct Show {
    /// `H`: the canvas is hidden and the mix is all the central panel shows, so a laptop
    /// can be turned to face an audience. The side panels stay, as silvia's do.
    pub(super) hidden: bool,
    /// The lines at the bottom of the window, oldest first, each with the clock time it
    /// leaves at. At most [`TOASTS`].
    toasts: Vec<Toast>,
    /// What the problems badge counts and its list holds, beyond what is read fresh.
    pub(super) problems: super::problems::Problems,
}

/// What a toast is saying, which decides how long it stays and what replaces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// News of the editor itself — *Editor hidden*, *Undo Delete 3 nodes*. The next one
    /// replaces it.
    News,
    /// A file was written, with a **Show**.
    Written,
    /// Something failed, marked by the `⚠` and the accent.
    Failed,
}

/// One toast on screen.
#[derive(Debug, Clone)]
struct Toast {
    text: String,
    until: f64,
    shows: Option<std::path::PathBuf>,
    kind: Kind,
    /// Where its **▸ go** takes the view: what an undo or a redo changed, where that is not
    /// on screen.
    go: Option<Go>,
}

impl Toast {
    /// Whether it stays while the pointer is on it: one with something to press, or a
    /// failure to read.
    fn lingers(&self) -> bool {
        self.kind != Kind::News || self.go.is_some()
    }
}

/// Where a toast's ▸ go takes the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Go {
    /// That node, on that workspace, centred and throbbing as a tag's link puts it.
    Node(crate::graph::WorkspaceId, crate::graph::NodeId),
    /// That workspace, open and showing.
    Workspace(crate::graph::WorkspaceId),
}

/// What was pressed on a toast's line.
enum Pressed {
    Show(std::path::PathBuf),
    Go(Go),
}

/// How long a toast stays, in seconds: silvia's two.
const TOAST_SECONDS: f64 = 2.0;

/// How long a toast with a Show button, a ▸ go or a failure stays, in seconds: long enough
/// to read and to reach. Under the pointer it stays until the pointer leaves.
const SHOW_SECONDS: f64 = 6.0;

/// How many toasts stand at once. A fourth pushes the oldest off.
pub const TOASTS: usize = 3;

impl App {
    /// The newest toast on screen, if one is.
    pub fn toast(&self) -> Option<&str> {
        self.live_toasts().last().map(|t| t.text.as_str())
    }

    /// Every toast on screen, oldest first.
    pub fn toasts(&self) -> Vec<&str> {
        self.live_toasts().map(|t| t.text.as_str()).collect()
    }

    /// The file the newest toast offering one offers to show.
    pub fn toast_shows(&self) -> Option<&std::path::Path> {
        self.live_toasts()
            .filter_map(|t| t.shows.as_deref())
            .next_back()
    }

    /// Where the newest toast offering to go offers to go.
    pub fn toast_goes(&self) -> Option<Go> {
        self.live_toasts().filter_map(|t| t.go).next_back()
    }

    fn live_toasts(&self) -> impl DoubleEndedIterator<Item = &Toast> {
        let now = self.clock().elapsed();
        self.show.toasts.iter().filter(move |t| now < t.until)
    }

    /// Put a toast up. The ones gone are dropped first; one saying the same thing again is
    /// moved to the foot rather than doubled, and a piece of news replaces the last news.
    fn toast_up(&mut self, toast: Toast) {
        let now = self.clock().elapsed();
        let toasts = &mut self.show.toasts;
        toasts.retain(|t| {
            now < t.until
                && t.text != toast.text
                && !(toast.kind == Kind::News && t.kind == Kind::News)
        });
        toasts.push(toast);
        while toasts.len() > TOASTS {
            toasts.remove(0);
        }
    }

    pub(super) fn say(&mut self, text: impl Into<String>) {
        self.say_with_go(text, None);
    }

    /// Say what changed, with a **▸ go** beside it where there is somewhere to go, which
    /// stays as long as a Show does so a hand can reach it. News, so the next undo's toast
    /// replaces it rather than stacking.
    pub(super) fn say_with_go(&mut self, text: impl Into<String>, go: Option<Go>) {
        let seconds = if go.is_some() {
            SHOW_SECONDS
        } else {
            TOAST_SECONDS
        };
        let until = self.clock().elapsed() + seconds;
        self.toast_up(Toast {
            text: text.into(),
            until,
            shows: None,
            kind: Kind::News,
            go,
        });
    }

    /// Say that something failed, marked as a failure. Through [`App::fail`], which also
    /// writes the status line and keeps it for the problems list.
    pub(super) fn say_failed(&mut self, text: impl Into<String>) {
        let until = self.clock().elapsed() + SHOW_SECONDS;
        self.toast_up(Toast {
            text: text.into(),
            until,
            shows: None,
            kind: Kind::Failed,
            go: None,
        });
    }

    /// Say that a file was written — a Snap, a render — with a **Show** beside it that opens
    /// the folder holding it with the file selected. What a Snap and a finished render say,
    /// where a person sees it with the Status box closed.
    pub fn say_written(&mut self, text: impl Into<String>, file: std::path::PathBuf) {
        let until = self.clock().elapsed() + SHOW_SECONDS;
        self.toast_up(Toast {
            text: text.into(),
            until,
            shows: Some(file),
            kind: Kind::Written,
            go: None,
        });
    }

    /// The paint callback that puts the mix in a rect, covering it: the background.
    pub(super) fn mix_cover(&self, rect: egui::Rect) -> Option<egui::PaintCallback> {
        let viewer = self.viewer.as_ref()?;
        Some(viewer.mixer_callback(self.link.published(), rect, crate::render::Fit::Cover))
    }

    /// The toasts: a line each at the bottom of the window, above everything, the newest at
    /// the foot. Drawn where they are asked for, so the clock decides rather than a timer.
    ///
    /// A piece of news is gone in two seconds. One that says a file was written carries a
    /// **Show** after its text, one that says what an undo changed off screen a **▸ go**, and
    /// one that says something failed wears the `⚠` and an edge in the accent; those stay six
    /// seconds, and for as long as the pointer is on them. They float over the canvas, so
    /// they move nothing under them, and each is one line whatever it says.
    pub(super) fn show_toast(&mut self, ctx: &egui::Context) {
        self.hear_failures();
        let toasts: Vec<Toast> = self.live_toasts().cloned().collect();
        if toasts.is_empty() {
            return;
        }
        let mut pressed = None;
        let shown = egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -40.0))
            .interactable(toasts.iter().any(Toast::lingers))
            .show(ctx, |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    for toast in &toasts {
                        if let Some(p) = self.toast_line(ui, toast) {
                            pressed = Some(p);
                        }
                    }
                });
            });
        let now = self.clock().elapsed();
        if shown.response.contains_pointer() {
            for live in self.show.toasts.iter_mut().filter(|t| t.lingers()) {
                if now < live.until {
                    live.until = live.until.max(now + 1.0);
                }
            }
        }
        match pressed {
            Some(Pressed::Show(file)) => {
                crate::platform::files::reveal_file(file.clone());
                self.show.toasts.retain(|t| t.shows.as_ref() != Some(&file));
            }
            Some(Pressed::Go(to)) => {
                self.go(to);
                self.show.toasts.retain(|t| t.go != Some(to));
            }
            None => {}
        }
    }

    /// One toast's line, and what was pressed on it: its **Show** or its **▸ go**.
    fn toast_line(&self, ui: &mut egui::Ui, toast: &Toast) -> Option<Pressed> {
        let failed = toast.kind == Kind::Failed;
        let edge = if failed {
            self.theme.accent()
        } else {
            self.theme.border_normal()
        };
        let mut pressed = None;
        egui::Frame::popup(ui.style())
            .fill(self.theme.bg_secondary())
            .stroke(egui::Stroke::new(1.0, edge))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if failed {
                        ui.label(egui::RichText::new("⚠").color(self.theme.accent()));
                    }
                    ui.add(
                        egui::Label::new(toast.text.as_str()).wrap_mode(egui::TextWrapMode::Extend),
                    );
                    if let Some(file) = &toast.shows
                        && ui
                            .button("Show")
                            .on_hover_text(format!(
                                "The file, in {}",
                                crate::platform::files::MANAGER
                            ))
                            .clicked()
                    {
                        pressed = Some(Pressed::Show(file.clone()));
                    }
                    if let Some(to) = toast.go
                        && ui
                            .button("▸ go")
                            .on_hover_text("Show where it changed")
                            .clicked()
                    {
                        pressed = Some(Pressed::Go(to));
                    }
                });
            });
        pressed
    }

    /// The pictures showing in windows of their own, and which of those are fullscreen. For
    /// tests, and for the marks on a picture, which say what its window is already doing.
    pub fn popped_out(&self) -> &[crate::ui::Popped] {
        self.wall.open()
    }

    /// Why there are no picture windows, where there are none — no Wayland, or no GPU.
    pub fn no_picture_windows(&self) -> Option<&str> {
        self.pictures.why_not()
    }

    /// Act on one of a picture's two marks: its window, or its window fullscreen.
    ///
    /// The pop-out mark is a toggle, as **Open projector** was: the same click that opened
    /// the window puts it away — which matters most when the window is on a screen the hand
    /// cannot see, and is why the mark is lit while it is up. The fullscreen mark opens the
    /// window already fullscreen, or flips a window that is already open. Not a command: a
    /// window showing a picture is where the picture is being looked at, not an edit to the
    /// graph.
    ///
    /// The rule itself is [`crate::render::picture::Wall`]'s, which is where it is tested
    /// without a compositor; this is the size and the name the ask carries.
    pub fn pop_out(&mut self, req: crate::ui::PopOutRequest) {
        let size = self.picture_size(req.picture);
        let title = self.picture_title(req.picture);
        if let Some(ask) = self.wall.mark(req, size, title)
            && let Some(told) = self.pictures.send(ask)
        {
            self.picture_told(told);
        }
    }

    /// What the pictures thread has said since the last frame, and the windows whose picture
    /// has gone.
    ///
    /// A deleted node takes its window with it: there is no picture left to show, and a
    /// black window nobody asked to keep is litter.
    pub(super) fn poll_pictures(&mut self) {
        for told in self.pictures.told() {
            self.picture_told(told);
        }
        let orphans: Vec<crate::ui::PopOut> = self
            .wall
            .open()
            .iter()
            .map(|p| p.picture)
            .filter(|p| p.node().is_some_and(|n| self.doc.graph().get(n).is_none()))
            .collect();
        for picture in orphans {
            if let Some(ask) = self.wall.forget(picture) {
                self.pictures.send(ask);
            }
        }
    }

    fn picture_told(&mut self, told: Told) {
        if let Some(why) = self.wall.told(told) {
            self.say(why);
        }
    }

    /// The size a picture's window opens at: the picture's own, in pixels.
    ///
    /// The mix's is the mixer's resolution; a node's is whatever it last published, which is
    /// the same texture the window will blit. A picture that has published nothing yet opens
    /// at [`DEFAULT_SIZE`] — the thread caps either against the output the window lands on.
    fn picture_size(&self, picture: crate::ui::PopOut) -> (u32, u32) {
        match picture {
            crate::ui::PopOut::Mix => self.project.mixer().resolution.pixels(self.mix_viewport),
            crate::ui::PopOut::Node { node, port } => self
                .link
                .published()
                .picture(node, port)
                .map_or(DEFAULT_SIZE, |p| (p.width, p.height)),
        }
    }

    /// What a picture window is called in the task switcher, which is the only place a
    /// window with no title bar shows a name.
    ///
    /// **Not `… — supersilvia`**, which is the editor's own suffix: a window rule or a
    /// script that matches the editor by its title must not also catch the pictures.
    fn picture_title(&self, picture: crate::ui::PopOut) -> String {
        match picture {
            crate::ui::PopOut::Mix => "Mix".to_string(),
            crate::ui::PopOut::Node { node, port } => {
                let label = self
                    .doc
                    .graph()
                    .get(node)
                    .map_or("picture", |n| n.def.label);
                match port {
                    None => format!("{label} {node}"),
                    Some(key) => format!("{label} {node} {key}"),
                }
            }
        }
    }

    /// Note where the window is and how big it is, the UI zoom, and where each of the
    /// editor's own windows stands, so the next launch opens like this one.
    ///
    /// A maximized window records only that it is maximized: the size and position kept are
    /// the ones unmaximizing restores. Whole points, because a fractional wobble between
    /// frames is a difference nobody asked to save.
    pub(super) fn record_window_geometry(&mut self, ctx: &egui::Context) {
        let (inner, outer, maximized) = ctx.input(|i| {
            let viewport = i.viewport();
            (viewport.inner_rect, viewport.outer_rect, viewport.maximized)
        });
        let mut window = self.prefs.get().window;
        window.maximized = maximized.unwrap_or(false);
        if !window.maximized {
            if let Some(rect) = inner {
                window.size = [rect.width().round(), rect.height().round()];
            }
            if let Some(rect) = outer {
                window.position = Some([rect.min.x.round(), rect.min.y.round()]);
            }
        }
        self.prefs.set_window(window);
        // The UI zoom, in hundredths: egui's keyboard steps are tenths, and a float's last
        // digits are not a change worth saving.
        self.prefs
            .set_ui_zoom((ctx.zoom_factor() * 100.0).round() / 100.0);
        for (title, placement) in crate::ui::placed::read(ctx) {
            self.prefs.set_placement(title, placement);
        }
    }
}
