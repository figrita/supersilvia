// SPDX-License-Identifier: AGPL-3.0-or-later

//! The main thread's side of the picture windows on macOS and Windows: winit's event loop, with
//! eframe inside it, and every picture window's `Window`.
//!
//! **Why the loop is ours.** AppKit makes and drives every window from the main thread, Win32
//! from the thread that made it, and winit permits one event loop per process, which eframe
//! would otherwise own. So [`super::run`] builds the loop itself, hands it to
//! `eframe::create_native`, and runs [`Loop`] around what comes back. `Loop` passes eframe every
//! event that is not for a picture window, so the editor's life is the one `eframe::run_native`
//! gives it: on macOS winit's `run` *is* `run_on_demand`, and both paths wrap eframe with the
//! same `run_and_return` flag.
//!
//! **Every method is forwarded.** winit's `ApplicationHandler` gives each of its nine methods
//! a default, so one left out here would compile and silently drop that event for eframe. An
//! eframe or winit upgrade re-reads the trait.
//!
//! **A window is made here and drawn elsewhere.** Its wgpu surface is made here too, because
//! winit hands out a window handle on the main thread alone, and then moves to the window's
//! own thread in [`super::draw`]. What stays here is the window, its keys, its pointer and
//! its fullscreen.
//!
//! **The hand is ours.** winit on macOS has no resize at all, so a press is remembered and,
//! once it has travelled [`DRAG_SLOP`], every motion sets the window's frame from
//! [`super::super::dragged`]: the middle moves the window, a 12-point band resizes it from that
//! edge, with `Shift` or `Ctrl` holding the picture's aspect. The move is ours too, rather
//! than AppKit's `performWindowDragWithEvent:`, which Apple documents for a mouse-down and which
//! would take the second press of a double-click with it if it were called on one. Windows
//! takes the same gesture, so the aspect lock is the same on both.
//!
//! **Fullscreen differs by machine**: macOS's covers the screen in place with the menu bar and
//! the Dock hidden, Windows' is winit's borderless fullscreen on the window's monitor.
//! [`fullscreen`] has a body for each, and a Mac's window is made with two attributes of its
//! own, no shadow and the menu bar's hiding.
//!
//! **This thread holds every window until the thread drawing it has ended.** A winit window
//! dropped anywhere else closes itself by waiting on this thread, so a drawing thread that let
//! go of the last one while this thread waited for it would wait for ever. Closing a window
//! therefore ends its thread first and drops the window after.

use super::super::{Ask, BAND, Bounds, Edge, Escaped, Shown, Told, dragged, edge_at, escaped};
use super::{Start, ToLoop, draw};
use eframe::UserEvent;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{
    DeviceEvent, DeviceId, ElementState, KeyEvent, MouseButton, StartCause, WindowEvent,
};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesExtMacOS as _, WindowExtMacOS as _};
use winit::window::{CursorIcon, Window, WindowId, WindowLevel};

/// How wide a picture window opens when its picture has no size of its own yet, in points.
const FALLBACK_WIDTH: f64 = 640.0;

/// The smallest a picture window may be, in points.
const MIN_SIZE: (f64, f64) = (160.0, 90.0);

/// How long closing a window waits for its thread to end before hiding it and moving on. A
/// visible window's thread ends within a refresh or two; one that does not is waiting for a
/// drawable that an occluded window may never be given, and waiting longer would not help.
const CLOSE_WAIT: Duration = Duration::from_millis(100);

/// How far the pointer must travel with the button down, in points, before a press becomes a
/// move or a resize — so a click, and the first press of a double-click, moves nothing.
const DRAG_SLOP: f64 = 4.0;

/// Two presses closer together than this are a double-click, as on Linux.
const DOUBLE_CLICK: Duration = Duration::from_millis(300);

/// A left button held down on a picture window, waiting to become a move or a resize.
struct Press {
    window: WindowId,
    /// Where it landed, on screen, in points.
    at: (f64, f64),
    /// Where the window was when it landed.
    start: Bounds,
    /// The band it landed in; `None`, the middle, moves the window.
    edge: Option<Edge>,
    /// It has travelled past [`DRAG_SLOP`], and the window follows it.
    active: bool,
}

/// One picture window, as the main thread holds it.
struct Win {
    picture: Shown,
    window: Arc<Window>,
    draw: Sender<draw::Msg>,
    thread: std::thread::JoinHandle<()>,
    fullscreen: bool,
    /// True for a window the **fullscreen** mark opened. It has no pop-out to go back to, so
    /// one `Escape` closes it — see [`super::super::escaped`].
    opened_fullscreen: bool,
    /// The size the window opened at, in points. Its ratio is the aspect a locked resize
    /// keeps.
    opened_at: (f64, f64),
    /// Where the pointer is over the window, in points, while it is.
    at: Option<(f64, f64)>,
    /// The two buttons `mouseinput` reads, as this window last saw them.
    held: (bool, bool),
    /// The cursor last set, so a motion inside one band is not a request a motion.
    cursor: Option<CursorIcon>,
    /// Kept above every other app's windows, by `K`.
    on_top: bool,
}

/// eframe, run inside a winit loop of our own, and the picture windows beside it.
pub(super) struct Loop<'a> {
    eframe: eframe::EframeWinitApplication<'a>,
    inbox: Receiver<ToLoop>,
    told: Sender<Told>,
    /// What the windows draw with, once `App::new` has sent it.
    start: Option<Start>,
    viewers: Arc<draw::Viewers>,
    windows: Vec<Win>,
    /// Windows whose thread was asked to end and had not when the window was closed: hidden,
    /// and held here until it has.
    retiring: Vec<(Arc<Window>, std::thread::JoinHandle<()>)>,
    /// The left button down on a picture window, until it is let go.
    press: Option<Press>,
    /// The last left press: when, and on which window, for the double-click.
    last_press: Option<(Instant, WindowId)>,
    /// The modifiers held, for the aspect lock. winit reports them per window and clears them
    /// when the focus leaves.
    mods: ModifiersState,
}

impl<'a> Loop<'a> {
    pub(super) fn new(
        eframe: eframe::EframeWinitApplication<'a>,
        inbox: Receiver<ToLoop>,
        told: Sender<Told>,
    ) -> Self {
        Self {
            eframe,
            inbox,
            told,
            start: None,
            viewers: Arc::default(),
            windows: Vec::new(),
            retiring: Vec::new(),
            press: None,
            last_press: None,
            mods: ModifiersState::empty(),
        }
    }

    /// One of the editor's asks.
    fn ask(&mut self, event_loop: &ActiveEventLoop, ask: Ask) {
        match ask {
            Ask::Open {
                picture,
                size,
                title,
                fullscreen,
            } => {
                if let Err(why) = self.open(event_loop, picture, size, &title, fullscreen) {
                    log::error!("picture window: {why}");
                    let _ = self.told.send(Told::Failed(picture, why));
                }
            }
            Ask::Close(picture) => self.close(picture),
            Ask::Fullscreen(picture, to) => {
                if let Some(win) = self.windows.iter_mut().find(|w| w.picture == picture) {
                    set_fullscreen(win, to);
                    // Said whether or not it changed: the editor's list asked, so it hears
                    // what the window is actually doing.
                    let _ = self.told.send(Told::Fullscreen(picture, win.fullscreen));
                }
            }
        }
    }

    /// Open a window for a picture, at the picture's own size, and start the thread that
    /// draws it.
    fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        picture: Shown,
        size: (u32, u32),
        title: &str,
        fullscreen: bool,
    ) -> Result<(), String> {
        if self.windows.iter().any(|w| w.picture == picture) {
            return Ok(());
        }
        let start = self
            .start
            .as_ref()
            .ok_or("the picture windows are not ready")?;
        let (width, height) = opening_size(event_loop, size);
        let attributes = Window::default_attributes()
            .with_title(title)
            // No decorations, which is the pop-out's whole shape: what is on screen is the
            // picture and nothing else. Not resizable by the window system, because the resize
            // band is ours; its own edge would take presses meant for it.
            .with_decorations(false)
            .with_resizable(false)
            .with_inner_size(LogicalSize::new(width, height))
            .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1));
        #[cfg(target_os = "macos")]
        let attributes = attributes
            // No shadow, as on Linux: the edge of the picture is the edge of the window.
            .with_has_shadow(false)
            // Fullscreen hides the menu bar and the Dock outright, rather than on hover.
            .with_borderless_game(true);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| format!("no window for the picture: {e}"))?,
        );
        // Here, on the main thread: winit gives a window handle nowhere else, and on a Mac the
        // layer under it is made from the view, which AppKit allows nowhere else either.
        let surface = start
            .gpu
            .instance()
            .create_surface(Arc::clone(&window))
            .map_err(|e| format!("no surface for the window: {e}"))?;
        if !start.gpu.adapter().is_surface_supported(&surface) {
            return Err("the GPU cannot present to this window".to_string());
        }
        let physical = window.inner_size();
        let (draw, thread) = draw::spawn(draw::Job {
            picture,
            surface,
            physical: (physical.width, physical.height),
            gpu: start.gpu.clone(),
            live: Arc::clone(&start.live),
            viewers: Arc::clone(&self.viewers),
            told: self.told.clone(),
        })
        .map_err(|e| format!("the picture window's thread could not be started: {e}"))?;
        let mut win = Win {
            picture,
            window,
            draw,
            thread,
            fullscreen: false,
            opened_fullscreen: fullscreen,
            opened_at: (width, height),
            at: None,
            held: (false, false),
            cursor: None,
            on_top: false,
        };
        if fullscreen {
            set_fullscreen(&mut win, true);
            let _ = self.told.send(Told::Fullscreen(picture, win.fullscreen));
        }
        self.windows.push(win);
        Ok(())
    }

    /// Close a picture's window, where it has one.
    fn close(&mut self, picture: Shown) {
        if let Some(index) = self.windows.iter().position(|w| w.picture == picture) {
            let win = self.windows.remove(index);
            // A window that goes sends no leave, so the pointer it may have been under is let
            // go of here. The next motion over any window reports again.
            if win.at.is_some() {
                self.unreport();
            }
            if self
                .press
                .as_ref()
                .is_some_and(|p| p.window == win.window.id())
            {
                self.press = None;
            }
            self.retire(vec![win]);
        }
    }

    /// End these windows' threads, then drop the windows.
    ///
    /// A window stays on screen until its thread has ended, because a visible window's thread
    /// is at most a refresh from its next present, and a hidden one's may wait for a drawable
    /// for ever. One whose thread has not ended within [`CLOSE_WAIT`] is hidden and held in
    /// `retiring`, which drops it once the thread ends.
    fn retire(&mut self, windows: Vec<Win>) {
        for win in &windows {
            // Out of fullscreen first, which on a Mac puts the menu bar and the Dock back.
            if win.fullscreen {
                fullscreen(&win.window, false);
            }
            let _ = win.draw.send(draw::Msg::Close);
        }
        let until = Instant::now() + CLOSE_WAIT;
        while Instant::now() < until && windows.iter().any(|w| !w.thread.is_finished()) {
            std::thread::sleep(Duration::from_millis(1));
        }
        for win in windows {
            if !win.thread.is_finished() {
                win.window.set_visible(false);
                self.retiring.push((win.window, win.thread));
            }
        }
    }

    /// Drop what has ended: retired windows whose thread is done, and any window whose thread
    /// ended on its own, which is one whose surface could not be configured and said so.
    fn reap(&mut self) {
        self.retiring.retain(|(_, thread)| !thread.is_finished());
        let (ended, open): (Vec<Win>, Vec<Win>) = std::mem::take(&mut self.windows)
            .into_iter()
            .partition(|w| w.thread.is_finished());
        self.windows = open;
        for win in ended {
            if win.fullscreen {
                fullscreen(&win.window, false);
            }
        }
    }

    /// A window event for one of the picture windows. `false` is an event for a window that
    /// is not one of them, which is eframe's.
    fn window_event_ours(&mut self, id: WindowId, event: &WindowEvent) -> bool {
        let Some(index) = self.windows.iter().position(|w| w.window.id() == id) else {
            // A closed window still waiting on its thread is ours too, and has nothing to say.
            return self.retiring.iter().any(|(window, _)| window.id() == id);
        };
        let win = &mut self.windows[index];
        match event {
            WindowEvent::Resized(size) => {
                let _ = win.draw.send(draw::Msg::Resized((size.width, size.height)));
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = win.window.inner_size();
                let _ = win.draw.send(draw::Msg::Resized((size.width, size.height)));
            }
            WindowEvent::Occluded(hidden) => {
                let _ = win.draw.send(draw::Msg::Occluded(*hidden));
            }
            WindowEvent::CloseRequested => {
                let picture = win.picture;
                let _ = self.told.send(Told::Closed(picture));
                self.close(picture);
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => self.keyed(index, *code),
            WindowEvent::ModifiersChanged(modifiers) => self.mods = modifiers.state(),
            // A modifier held as the focus leaves is not held here any more, and a stale one
            // would lock the aspect of the next resize nobody asked to lock.
            WindowEvent::Focused(false) => self.mods = ModifiersState::empty(),
            WindowEvent::CursorMoved { position, .. } => self.moved(index, *position),
            WindowEvent::MouseInput { state, button, .. } => {
                self.button(index, *button, *state == ElementState::Pressed);
            }
            WindowEvent::CursorLeft { .. } => {
                let win = &mut self.windows[index];
                win.at = None;
                win.cursor = None;
                self.unreport();
            }
            _ => {}
        }
        true
    }

    /// The pointer moved over a window, or anywhere with the button down on one: the gesture
    /// where there is one, the cursor where there is not, and the reading for the graph.
    fn moved(&mut self, index: usize, position: PhysicalPosition<f64>) {
        let win = &mut self.windows[index];
        let local: LogicalPosition<f64> = position.to_logical(win.window.scale_factor());
        let local = (local.x, local.y);
        win.at = Some(local);
        let id = win.window.id();
        let lock = self.mods.shift_key() || self.mods.control_key();
        let aspect = lock.then(|| win.opened_at.0 / win.opened_at.1.max(1.0));
        match self.press.as_mut().filter(|p| p.window == id) {
            Some(press) => {
                if let Some(now) = bounds(&win.window) {
                    let travel = (now.x + local.0 - press.at.0, now.y + local.1 - press.at.1);
                    if press.active || travel.0.hypot(travel.1) >= DRAG_SLOP {
                        press.active = true;
                        place(
                            &win.window,
                            dragged(press.start, travel, press.edge, aspect, MIN_SIZE),
                        );
                    }
                }
            }
            None => point(win, local),
        }
        self.report(index);
    }

    /// A button went down or up over a window.
    fn button(&mut self, index: usize, button: MouseButton, down: bool) {
        match button {
            MouseButton::Left => {
                self.windows[index].held.0 = down;
                if down {
                    self.pressed(index);
                } else {
                    self.press = None;
                }
            }
            MouseButton::Right => self.windows[index].held.1 = down,
            _ => {}
        }
        self.report(index);
    }

    /// A left press. Two in quick succession on one window are a double-click and so
    /// fullscreen; otherwise the press is remembered and becomes a move or a resize only once
    /// the pointer has travelled — see [`DRAG_SLOP`].
    fn pressed(&mut self, index: usize) {
        let now = Instant::now();
        let win = &mut self.windows[index];
        let id = win.window.id();
        let double = self
            .last_press
            .is_some_and(|(then, on)| on == id && now.duration_since(then) < DOUBLE_CLICK);
        if double {
            let (picture, to) = (win.picture, !win.fullscreen);
            set_fullscreen(win, to);
            let _ = self.told.send(Told::Fullscreen(picture, win.fullscreen));
            // Cleared, so a third press starts a fresh pair rather than toggling again.
            self.last_press = None;
            self.press = None;
            return;
        }
        self.last_press = Some((now, id));
        // Fullscreen, there is nothing to move a window to or resize it by.
        if win.fullscreen {
            return;
        }
        let (Some(local), Some(start)) = (win.at, bounds(&win.window)) else {
            return;
        };
        self.press = Some(Press {
            window: id,
            at: (start.x + local.0, start.y + local.1),
            start,
            edge: edge_at(local, (start.width, start.height), BAND),
            active: false,
        });
    }

    /// Where the pointer is over this window's picture, for the graph.
    ///
    /// **Against the picture, not against the window**, as on Linux: the blit letterboxes,
    /// so a hand on a bar is not on the picture and reads nothing, and a window whose picture
    /// has not been drawn yet has no aspect and reads nothing either.
    fn report(&self, index: usize) {
        let Some(start) = &self.start else { return };
        let win = &self.windows[index];
        let Some(at) = win.at else {
            return;
        };
        let published = start.live.get();
        let picture = match win.picture {
            Shown::Node { node, port } => published.picture(node, port),
            Shown::Mix => published.mixer.clone(),
        };
        let size = win
            .window
            .inner_size()
            .to_logical::<f64>(win.window.scale_factor());
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let reading = crate::pointer::place(
            (at.0 as f32, at.1 as f32),
            (size.width as f32, size.height as f32),
            picture.map_or(0.0, |p| p.width as f32 / p.height.max(1) as f32),
        );
        let (left, right) = win.held;
        let reading = reading.map(|r| crate::pointer::Reading { left, right, ..r });
        start.pointer.set(crate::pointer::Surface::Picture, reading);
    }

    /// The pointer has left every picture window, so the graph reads nothing rather than
    /// the last place the hand was.
    fn unreport(&self) {
        if let Some(start) = &self.start {
            start.pointer.set(crate::pointer::Surface::Picture, None);
        }
    }

    /// `F` fullscreens, `Escape` leaves fullscreen and a second `Escape` closes, and `K`
    /// keeps the window above every other app's or lets it go — which is what a picture with
    /// no title bar needs a key for. Read by position, as Linux reads evdev codes: `KeyCode`
    /// is the physical key on every layout.
    fn keyed(&mut self, index: usize, code: KeyCode) {
        let win = &mut self.windows[index];
        let picture = win.picture;
        match code {
            KeyCode::KeyF => {
                let to = !win.fullscreen;
                set_fullscreen(win, to);
                let _ = self.told.send(Told::Fullscreen(picture, win.fullscreen));
            }
            KeyCode::KeyK => {
                win.on_top = !win.on_top;
                // The floating level, AppKit's or Win32's topmost band, above every normal
                // window whichever app is in front, and kept through fullscreen.
                win.window.set_window_level(if win.on_top {
                    WindowLevel::AlwaysOnTop
                } else {
                    WindowLevel::Normal
                });
            }
            KeyCode::Escape => match escaped(win.fullscreen, win.opened_fullscreen) {
                Escaped::Windowed => {
                    set_fullscreen(win, false);
                    let _ = self.told.send(Told::Fullscreen(picture, win.fullscreen));
                }
                Escaped::Closed => {
                    let _ = self.told.send(Told::Closed(picture));
                    self.close(picture);
                }
            },
            _ => {}
        }
    }
}

/// Fullscreen, or not, as the window says it is after the ask.
fn set_fullscreen(win: &mut Win, to: bool) {
    win.fullscreen = fullscreen(&win.window, to);
}

/// Fullscreen as decided on a Mac: the window covers its screen at once, in place, with no
/// animation and no Space of its own, and the menu bar and the Dock hidden outright —
/// `set_simple_fullscreen`, on a window made with `with_borderless_game`.
/// `proposals/macos-windows.md`, decision 1. Whether the window is fullscreen after it.
#[cfg(target_os = "macos")]
fn fullscreen(window: &Window, to: bool) -> bool {
    window.set_simple_fullscreen(to);
    window.simple_fullscreen()
}

/// Fullscreen on Windows: a borderless window the size of the monitor it is on, which covers
/// the taskbar and changes no display mode. Whether the window is fullscreen after it.
#[cfg(target_os = "windows")]
fn fullscreen(window: &Window, to: bool) -> bool {
    window.set_fullscreen(to.then_some(winit::window::Fullscreen::Borderless(None)));
    window.fullscreen().is_some()
}

/// Where a window is on screen and how big, in points. A picture window has no frame, so its
/// outer and inner rectangles are the same.
fn bounds(window: &Window) -> Option<Bounds> {
    let scale = window.scale_factor();
    let at: LogicalPosition<f64> = window.outer_position().ok()?.to_logical(scale);
    let size: LogicalSize<f64> = window.inner_size().to_logical(scale);
    Some(Bounds {
        x: at.x,
        y: at.y,
        width: size.width,
        height: size.height,
    })
}

/// Put a window where a gesture says. The size first: AppKit keeps a window's bottom-left
/// corner where it was when its size changes, and setting the top-left after it puts it back.
fn place(window: &Window, to: Bounds) {
    let _ = window.request_inner_size(LogicalSize::new(to.width, to.height));
    window.set_outer_position(LogicalPosition::new(to.x, to.y));
}

/// The cursor for wherever the pointer is: a resize shape over a band, the arrow elsewhere.
/// Set only when it changes. The system's own two-headed resize shapes, which winit names.
fn point(win: &mut Win, local: (f64, f64)) {
    // A fullscreen window has no bands, so it has no resize cursor either.
    let band = if win.fullscreen { 0.0 } else { BAND };
    let Some(size) = bounds(&win.window).map(|b| (b.width, b.height)) else {
        return;
    };
    let icon = match edge_at(local, size, band) {
        None => CursorIcon::Default,
        Some(Edge::Top | Edge::Bottom) => CursorIcon::NsResize,
        Some(Edge::Left | Edge::Right) => CursorIcon::EwResize,
        Some(Edge::TopLeft | Edge::BottomRight) => CursorIcon::NwseResize,
        Some(Edge::TopRight | Edge::BottomLeft) => CursorIcon::NeswResize,
    };
    if win.cursor != Some(icon) {
        win.window.set_cursor(icon);
        win.cursor = Some(icon);
    }
}

/// The size a picture window opens at, in points: the picture's own size, capped to three
/// quarters of the largest display, as Linux caps it to the largest output.
fn opening_size(event_loop: &ActiveEventLoop, size: (u32, u32)) -> (f64, f64) {
    let largest = event_loop
        .available_monitors()
        .map(|m| m.size().to_logical::<f64>(m.scale_factor()))
        .fold((0.0_f64, 0.0_f64), |a, s| {
            (a.0.max(s.width), a.1.max(s.height))
        });
    let (mut w, mut h) = (f64::from(size.0), f64::from(size.1));
    if w < 2.0 || h < 2.0 {
        (w, h) = (FALLBACK_WIDTH, FALLBACK_WIDTH * 9.0 / 16.0);
    }
    if largest.0 > 0.0 && largest.1 > 0.0 {
        let cap = (
            (largest.0 * 0.75).max(MIN_SIZE.0),
            (largest.1 * 0.75).max(MIN_SIZE.1),
        );
        let shrink = (cap.0 / w).min(cap.1 / h).min(1.0);
        w = (w * shrink).max(MIN_SIZE.0);
        h = (h * shrink).max(MIN_SIZE.1);
    }
    (w.round(), h.round())
}

impl ApplicationHandler<UserEvent> for Loop<'_> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.eframe.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.resumed(event_loop);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.eframe.user_event(event_loop, event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if !self.window_event_ours(id, &event) {
            self.eframe.window_event(event_loop, id, event);
        }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.eframe.device_event(event_loop, id, event);
    }

    /// eframe's, then the editor's asks — sent from `App::ui` earlier in this same turn of the
    /// loop, so a window opens on the turn its mark was clicked.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.about_to_wait(event_loop);
        while let Ok(msg) = self.inbox.try_recv() {
            match msg {
                ToLoop::Start(start) => self.start = Some(start),
                ToLoop::Ask(ask) => self.ask(event_loop, ask),
            }
        }
        self.reap();
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.suspended(event_loop);
    }

    /// eframe's exit first, which is `App::on_exit`; then every picture window. A window whose
    /// thread has not ended by then is let go of without being dropped, so that thread never
    /// holds the last of it — the process ending takes both.
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.exiting(event_loop);
        let windows = std::mem::take(&mut self.windows);
        self.retire(windows);
        for (window, _) in std::mem::take(&mut self.retiring) {
            std::mem::forget(window);
        }
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.memory_warning(event_loop);
    }
}
