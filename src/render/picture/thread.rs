// SPDX-License-Identifier: AGPL-3.0-or-later

//! The pictures thread: a Wayland connection, a window per picture, and one blit each.
//!
//! **The loop is calloop over two fds** — the Wayland socket and the editor's asks — so the
//! thread sleeps between events rather than polling either. Nothing here has a rate of its
//! own: a window paints when its `wl_surface.frame` callback arrives, which is the
//! compositor's pacing for *that* window, on *that* output. A window on a 60 Hz projector
//! beside a 100 Hz editor is paced at 60 and shows the newest published frame twice where
//! the synth placed two; that is correct, and is what a viewer is.
//!
//! **Exactly one frame callback is outstanding per window.** It is asked for in
//! [`Pictures::paint`], immediately before the present whose commit carries it, and cleared
//! when it arrives. Asking for a second while one is outstanding is the bug this is written
//! against: the window would then present once per callback per request, for ever. The
//! surface presents in `Mailbox` or `Immediate`, never waiting for the display, so a present
//! never holds one window while another's callback is due.
//!
//! **A window nobody can see stops painting**, which is the same rule the editor follows —
//! see docs/rendering.md. A compositor stops sending frame callbacks for a surface that is not
//! composited, so a minimized picture window simply has one outstanding callback and sleeps;
//! the callback arrives when it is composited again. The watchdog below is *not* for that
//! case and must not fire in it: it is for a window with **no** callback outstanding that
//! has somehow not painted, and it is skipped for a window on no output at all.
//!
//! **What it owns and what it borrows.** It owns the event queue, every window, the wgpu
//! surface on each and a [`Viewer`] per surface format, on a clone of the one device. It
//! borrows two things and frees neither: eframe's `wl_display`, and the synth's published
//! textures through the `Arc<Live>` every other viewer reads — each `Published` held until
//! after the submit that carries its blit.
//!
//! **No egui.** A picture window is a bare picture — see `proposals/picture-windows.md` for
//! why the player's strip stays in the editor. What is left to handle by hand is two
//! gestures and two keys, which is what the pointer and keyboard below are for.

use super::{Ask, BAND, Edge, Escaped, Shown, Told, edge_at, escaped, fit_aspect};
use crate::render::Fit;
use crate::render::viewer::Viewport;
use crate::render::{Gpu, Live, Viewer};
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_output, delegate_pointer, delegate_registry, delegate_seat,
    delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{
            CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
        },
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{DecorationMode, Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler},
};
use wayland_backend::client::Backend;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
};

/// evdev's keycode for `Escape`, which is the same on every layout because it is the key's
/// position and `Escape` has only ever had one.
const KEY_ESCAPE: u32 = 1;
/// evdev's keycode for the key `F` sits on. Read as a *position* rather than as a letter,
/// because reading it as a letter would mean an xkb keymap — and the `xkbcommon` crate —
/// for one shortcut. Named here so the trade is visible rather than a magic number.
const KEY_F: u32 = 33;
/// `BTN_LEFT`, from evdev, which is what `wl_pointer.button` carries.
const BTN_LEFT: u32 = 0x110;
/// `BTN_RIGHT`, the other button `mouseinput` publishes.
const BTN_RIGHT: u32 = 0x111;

/// The bits of `wl_keyboard.modifiers`' depressed mask that mean `Shift` and `Control`.
///
/// The mask is indexed by the **keymap's** own modifier order, so reading a bit as a named
/// modifier is only true of a keymap that lists them in the usual order. Every xkb keymap
/// does — `Shift` first, `Lock`, then `Control` — because that order is inherited from the
/// core X protocol; it is a convention rather than a guarantee, and the alternative is an
/// xkb keymap on this thread and the `xkbcommon` crate with it, which is the trade `KEY_F`
/// already names.
const MOD_SHIFT: u32 = 1 << 0;
/// See [`MOD_SHIFT`]: the third modifier in the standard keymap order.
const MOD_CTRL: u32 = 1 << 2;

/// How wide a picture window opens when its picture has no size of its own yet, in pixels.
const FALLBACK_WIDTH: u32 = 640;

/// How far the pointer must travel with the button down before the window is handed to the
/// compositor to move or resize.
///
/// **Not on the press**, which is the bug this is written against: `xdg_toplevel.move` takes
/// a pointer grab and the compositor sends `leave`, so the second press of a double-click
/// never arrives and `F`-by-double-click could not be done with the mouse.
const DRAG_SLOP: f64 = 4.0;

/// How long a window with **no callback outstanding** may go without painting before the
/// thread paints it anyway. A safety net for a paint that bailed before it could ask for its
/// next callback; it is never what paces a window, and never what wakes a hidden one.
const STALL: std::time::Duration = std::time::Duration::from_millis(50);

/// Two presses closer together than this are a double-click.
const DOUBLE_CLICK_MS: u32 = 300;

/// A window's wgpu surface and how it is configured.
struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
}

/// One picture window.
///
/// `surface` is declared before `window`, so a `Win` dropped whole drops its wgpu surface
/// before the `wl_surface` under it.
struct Win {
    picture: Shown,
    /// Made on the first configure, because a surface is configured at a size and the
    /// compositor is the one that says what it is.
    surface: Option<Surface>,
    window: Window,
    /// What the compositor configured, in logical points. What a pointer position is in, and
    /// so what the resize bands are measured against.
    logical: (u32, u32),
    /// `logical` times `scale`: what the surface is configured at.
    physical: (u32, u32),
    scale: i32,
    fullscreen: bool,
    /// Asked for before the window was mapped; applied on the first configure, where the
    /// compositor will act on it.
    want_fullscreen: Option<bool>,
    /// A `wl_surface.frame` callback has been asked for and has not arrived. While this is
    /// true the window is paced, and nothing else may ask for another.
    pending: bool,
    /// How many outputs this surface is on. Zero is a window nobody can see, which the
    /// watchdog leaves alone.
    on_outputs: u32,
    /// When this window last painted, for the watchdog.
    painted: std::time::Instant,
    /// The size the window asked to open at, in logical points. Its ratio is the aspect a
    /// locked resize keeps.
    opened_at: (u32, u32),
    /// True for a window the **fullscreen** mark opened. It has no pop-out to go back to, so
    /// one `Escape` closes it — see [`super::escaped`].
    opened_fullscreen: bool,
    /// The edge a compositor-driven resize is dragging, while one is running. Set when the
    /// gesture is handed over and cleared by the first configure that is no longer resizing.
    resizing: Option<Edge>,
}

impl Win {
    fn matches(&self, surface: &wl_surface::WlSurface) -> bool {
        self.window.wl_surface() == surface
    }

    /// Whether the watchdog should look at this window: it has a surface to paint into, no
    /// callback is outstanding, and somebody can see it.
    fn watchable(&self) -> bool {
        self.surface.is_some() && !self.pending && self.on_outputs > 0
    }
}

/// A left button held down, waiting to become a move or a resize.
struct Press {
    surface: wl_surface::WlSurface,
    serial: u32,
    at: (f64, f64),
    /// The edge the press landed on, where it landed on one: the gesture is a resize from
    /// that edge. `None` is a press in the middle, which moves the window.
    edge: Option<Edge>,
}

/// The `xdg_toplevel` edge to hand the compositor for a resize from this edge.
fn resize_edge(edge: Edge) -> ResizeEdge {
    match edge {
        Edge::Top => ResizeEdge::Top,
        Edge::Bottom => ResizeEdge::Bottom,
        Edge::Left => ResizeEdge::Left,
        Edge::Right => ResizeEdge::Right,
        Edge::TopLeft => ResizeEdge::TopLeft,
        Edge::TopRight => ResizeEdge::TopRight,
        Edge::BottomLeft => ResizeEdge::BottomLeft,
        Edge::BottomRight => ResizeEdge::BottomRight,
    }
}

/// The cursor that says what a press on this edge would do. `None` — the middle of the
/// window — is the ordinary arrow, because a press there moves the window and every
/// compositor draws a move with the arrow it already has.
fn cursor_for(edge: Option<Edge>) -> CursorIcon {
    match edge {
        None => CursorIcon::Default,
        Some(Edge::Top) => CursorIcon::NResize,
        Some(Edge::Bottom) => CursorIcon::SResize,
        Some(Edge::Left) => CursorIcon::WResize,
        Some(Edge::Right) => CursorIcon::EResize,
        Some(Edge::TopLeft) => CursorIcon::NwResize,
        Some(Edge::TopRight) => CursorIcon::NeResize,
        Some(Edge::BottomLeft) => CursorIcon::SwResize,
        Some(Edge::BottomRight) => CursorIcon::SeResize,
    }
}

/// Everything the thread holds. One `&mut` of this is what every handler is given.
struct Pictures {
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    shell: XdgShell,
    /// Bound for one reason: the cursor. `ThemedPointer` falls back to a `wl_cursor` theme
    /// where `cursor-shape-v1` is absent, and that fallback is an shm buffer.
    shm: Shm,
    windows: Vec<Win>,
    /// eframe's `wl_display`, borrowed: what each window's wgpu surface is made against.
    display: std::ptr::NonNull<std::ffi::c_void>,
    /// The one device, which every window blits on.
    gpu: Gpu,
    /// The blit pipeline for each surface format a window has been configured in, made on the
    /// first window of that format.
    viewers: Vec<Viewer>,
    live: std::sync::Arc<Live>,
    /// Where the pointer is, for the graph. A picture window is not an egui region, so this
    /// thread's own `wl_pointer` is the only thing that sees a hand over the picture the
    /// audience is looking at. See [`crate::pointer`].
    feed: std::sync::Arc<crate::pointer::Feed>,
    /// The two buttons, as this window last saw them, and where the pointer was — so a
    /// press reports the position it happened at rather than waiting for a motion.
    held: (bool, bool),
    at: Option<(f64, f64)>,
    told: std::sync::mpsc::Sender<Told>,
    /// The pointer, themed: `set_cursor` goes through `cursor-shape-v1` where the
    /// compositor has it — KWin does — and falls back to the `wl_cursor` theme where it does
    /// not. Both paths are sctk's, and neither is a dependency that was not already here.
    pointer: Option<ThemedPointer>,
    /// The cursor last set, so a motion inside one band is not a request a frame.
    cursor: Option<CursorIcon>,
    /// `wl_keyboard.modifiers`' depressed mask for the window that has the focus. Cleared
    /// when the focus leaves, because a modifier held into another window is not held here.
    mods: u32,
    /// The connection, which is what `ThemedPointer::set_cursor` loads a theme against.
    conn: Connection,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    seat: Option<wl_seat::WlSeat>,
    /// Which window has the keyboard, so `F` and `Escape` reach one window rather than all.
    focus: Option<wl_surface::WlSurface>,
    /// The button that is down, until it travels far enough to become a gesture.
    press: Option<Press>,
    /// The last left press: its time and surface, for the double-click.
    last_press: Option<(u32, wl_surface::WlSurface)>,
    /// True once the connection has gone, which ends the loop.
    stop: bool,
    /// Every sctk call wants one and none of them keeps one.
    qh: QueueHandle<Self>,
}

/// Run the thread. Returns when the editor drops its sender or the connection goes.
///
/// # Safety
/// `display` must be a live `wl_display` — eframe's — that outlives this call. `App::on_exit`
/// stops and joins this thread before eframe tears its event loop down, which is what makes
/// that true.
pub unsafe fn run(
    display: *mut std::ffi::c_void,
    gpu: &Gpu,
    live: &std::sync::Arc<Live>,
    feed: &std::sync::Arc<crate::pointer::Feed>,
    asks: calloop::channel::Channel<Ask>,
    told: &std::sync::mpsc::Sender<Told>,
) {
    // SAFETY: the caller's contract. The backend borrows the display and never frees it —
    // `from_foreign_display` is wayland-backend's own door for an existing connection, and
    // one `wl_display` carrying a queue per thread is libwayland's design rather than
    // something being got away with.
    let backend = unsafe { Backend::from_foreign_display(display.cast()) };
    let conn = Connection::from_backend(backend);
    let (globals, queue) = match registry_queue_init::<Pictures>(&conn) {
        Ok(pair) => pair,
        Err(err) => return fail(told, format!("no Wayland registry: {err}")),
    };
    let qh = queue.handle();

    let compositor = match CompositorState::bind(&globals, &qh) {
        Ok(c) => c,
        Err(err) => return fail(told, format!("no wl_compositor: {err}")),
    };
    let shell = match XdgShell::bind(&globals, &qh) {
        Ok(s) => s,
        Err(err) => return fail(told, format!("no xdg-shell: {err}")),
    };
    let shm = match Shm::bind(&globals, &qh) {
        Ok(s) => s,
        Err(err) => return fail(told, format!("no wl_shm: {err}")),
    };

    let Some(display) = std::ptr::NonNull::new(display) else {
        return fail(told, "no Wayland display".to_string());
    };
    let mut state = Pictures {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seats: SeatState::new(&globals, &qh),
        compositor,
        shell,
        shm,
        windows: Vec::new(),
        display,
        gpu: gpu.clone(),
        viewers: Vec::new(),
        live: std::sync::Arc::clone(live),
        feed: std::sync::Arc::clone(feed),
        held: (false, false),
        at: None,
        told: told.clone(),
        pointer: None,
        cursor: None,
        mods: 0,
        conn: conn.clone(),
        keyboard: None,
        seat: None,
        focus: None,
        press: None,
        last_press: None,
        stop: false,
        qh: qh.clone(),
    };

    let mut loop_: calloop::EventLoop<Pictures> = match calloop::EventLoop::try_new() {
        Ok(l) => l,
        Err(err) => return fail(told, format!("no event loop: {err}")),
    };
    let handle = loop_.handle();
    if let Err(err) = calloop_wayland_source::WaylandSource::new(conn.clone(), queue).insert(handle)
    {
        return fail(
            told,
            format!("the Wayland source could not be added: {err}"),
        );
    }
    let asked = loop_.handle().insert_source(asks, |event, (), state| {
        match event {
            calloop::channel::Event::Msg(ask) => state.ask(ask),
            // The editor dropped its sender: the run is ending.
            calloop::channel::Event::Closed => state.stop = true,
        }
    });
    if let Err(err) = asked {
        return fail(told, format!("the asks could not be listened for: {err}"));
    }

    while !state.stop {
        // **Block with no timeout when nothing is waiting on the watchdog**, which is every
        // window either paced by a callback or on no output at all. A timeout of its own
        // would be a wakeup a second a hidden window did not need, which is exactly the cost
        // this whole thread exists to avoid.
        let timeout = state.windows.iter().any(Win::watchable).then_some(STALL);
        if loop_.dispatch(timeout, &mut state).is_err() {
            break;
        }
        state.nudge();
    }
    state.shut_down();
    // The editor is told before the sender drops, so a `Host` that has not yet noticed a
    // dead thread learns why rather than discovering it on the next silent send.
    let _ = told.send(Told::Gone("the pictures thread stopped".to_string()));
}

/// Nothing can be opened, so say so once and end. The editor latches the reason and answers
/// every later ask from it, rather than dropping asks into a thread that is gone.
fn fail(told: &std::sync::mpsc::Sender<Told>, why: String) {
    log::error!("picture windows: {why}");
    let _ = told.send(Told::Gone(why));
}

impl Pictures {
    fn find(&mut self, picture: Shown) -> Option<&mut Win> {
        self.windows.iter_mut().find(|w| w.picture == picture)
    }

    fn at(&mut self, surface: &wl_surface::WlSurface) -> Option<&mut Win> {
        self.windows.iter_mut().find(|w| w.matches(surface))
    }

    /// One of the editor's asks.
    fn ask(&mut self, ask: Ask) {
        match ask {
            Ask::Open {
                picture,
                size,
                title,
                fullscreen,
            } => self.open(picture, size, &title, fullscreen),
            Ask::Close(picture) => self.close(picture),
            Ask::Fullscreen(picture, to) => {
                if let Some(win) = self.find(picture) {
                    if win.surface.is_some() {
                        set_fullscreen(&win.window, to);
                    } else {
                        // Not mapped yet: the compositor would drop it. Held for the first
                        // configure, which is the one path to fullscreen.
                        win.want_fullscreen = Some(to);
                    }
                }
            }
        }
    }

    /// Open a window for a picture, at the picture's own size.
    fn open(&mut self, picture: Shown, size: (u32, u32), title: &str, fullscreen: bool) {
        if self.find(picture).is_some() {
            return;
        }
        // Capped to the largest output there is, so a 4K picture does not open a window
        // bigger than the screen it lands on. The compositor has the last word either way —
        // its configure is what the surface is made at.
        let largest = self
            .outputs
            .outputs()
            .filter_map(|o| self.outputs.info(&o))
            .filter_map(|i| i.logical_size.or(i.modes.first().map(|m| m.dimensions)))
            .fold((0, 0), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        let (mut w, mut h) = (size.0, size.1);
        if w < 2 || h < 2 {
            (w, h) = (FALLBACK_WIDTH, FALLBACK_WIDTH * 9 / 16);
        }
        if largest.0 > 0 && largest.1 > 0 {
            let cap = (largest.0 as u32 * 3 / 4).max(160);
            let capv = (largest.1 as u32 * 3 / 4).max(90);
            let shrink = (cap as f32 / w as f32).min(capv as f32 / h as f32).min(1.0);
            w = ((w as f32 * shrink) as u32).max(160);
            h = ((h as f32 * shrink) as u32).max(90);
        }

        let qh = self.qh.clone();
        let surface = self.compositor.create_surface(&qh);
        // No decorations, which is the pop-out's whole shape: what is on screen is the
        // picture and nothing else. `RequestClient` and then drawing none is what a
        // compositor that honours xdg-decoration does with it; KWin does.
        let window = self
            .shell
            .create_window(surface, WindowDecorations::RequestClient, &qh);
        window.set_title(title);
        window.set_app_id(app_id(fullscreen));
        window.set_min_size(Some((160, 90)));
        window.request_decoration_mode(Some(DecorationMode::Client));
        // The initial commit with no buffer: the compositor answers with a configure, and
        // that configure is where the wgpu surface is made.
        window.commit();
        self.windows.push(Win {
            picture,
            window,
            surface: None,
            logical: (w, h),
            physical: (w, h),
            scale: 1,
            fullscreen: false,
            want_fullscreen: fullscreen.then_some(true),
            pending: false,
            on_outputs: 0,
            painted: std::time::Instant::now(),
            opened_at: (w, h),
            opened_fullscreen: fullscreen,
            resizing: None,
        });
    }

    fn close(&mut self, picture: Shown) {
        let Some(index) = self.windows.iter().position(|w| w.picture == picture) else {
            return;
        };
        let win = self.windows.remove(index);
        Self::drop_window(win);
        // A surface that is gone sends no leave, so the pointer it may have been over is
        // let go of here. The next motion over any window reports again.
        self.unreport();
    }

    /// Free one window's surfaces in the order Wayland wants: the wgpu surface, then the
    /// `wl_surface` it was made on, with the window.
    fn drop_window(win: Win) {
        drop(win.surface);
        drop(win.window);
    }

    /// The wgpu surface for a window's `wl_surface`, configured at `physical` pixels, and the
    /// viewer for its format made where there is none yet. `Err` says why not.
    fn surface_for(
        &mut self,
        wl: &wl_surface::WlSurface,
        physical: (u32, u32),
    ) -> Result<Surface, String> {
        use raw_window_handle::{
            RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
        };
        let window = std::ptr::NonNull::new(wl.id().as_ptr().cast())
            .ok_or("the window has no wl_surface")?;
        // SAFETY: the display is eframe's, alive until this thread is joined, and the
        // `wl_surface` is this window's, which is dropped only after the surface made on it —
        // `drop_window` drops them in that order, and `Win` declares them in it.
        let surface = unsafe {
            self.gpu
                .instance()
                .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                        self.display,
                    ))),
                    raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
                })
        }
        .map_err(|e| format!("no surface for the window: {e}"))?;
        let adapter = self.gpu.adapter();
        if !adapter.is_surface_supported(&surface) {
            return Err("the GPU cannot present to this window".to_string());
        }
        let caps = surface.get_capabilities(adapter);
        // Not sRGB, so the values written are the values the synth drew, as the editor's are.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or("the window offers no format")?;
        // Never waiting on the display: the frame callback is the window's clock.
        let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
            .into_iter()
            .find(|m| caps.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);
        // Opaque, so the compositor never blends the desktop under a picture whose alpha is
        // below one; the opaque region set on each configure says the same.
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto)
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::default(),
            width: physical.0.max(1),
            height: physical.1.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: Vec::new(),
        };
        surface.configure(self.gpu.device(), &config);
        if !self.viewers.iter().any(|v| v.format() == format) {
            self.viewers.push(
                Viewer::new(&self.gpu, format).map_err(|e| format!("the pictures' viewer: {e}"))?,
            );
        }
        Ok(Surface { surface, config })
    }

    /// Paint one window: ask for the next callback, blit, present.
    ///
    /// What is blitted is the newest picture the synth has finished, so there is nothing to
    /// wait for. The `Published` it names is held until after the submit that carries the
    /// blit, so the synth draws nothing into what it names before the GPU has read it.
    ///
    /// **The callback is asked for immediately before the present**, because
    /// `wl_surface.frame` must reach the compositor on the same commit as the buffer, and the
    /// present is that commit. Everything that could fail is resolved first, so a window never
    /// ends up with a request it will not commit.
    fn paint(&mut self, surface: &wl_surface::WlSurface) {
        let published = self.live.get();
        let qh = self.qh.clone();
        let Some(win) = self.windows.iter_mut().find(|w| w.matches(surface)) else {
            return;
        };
        let Some(target) = win.surface.as_mut() else {
            return;
        };
        let frame = match target.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                // Configured again, and painted by the watchdog once it has a callback-free
                // window to look at.
                target.surface.configure(self.gpu.device(), &target.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => return,
        };
        let Some(viewer) = self
            .viewers
            .iter()
            .find(|v| v.format() == target.config.format)
        else {
            return;
        };
        let size = (target.config.width, target.config.height);
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("picture window"),
                });
        crate::render::shared::clear(&mut encoder, &view, wgpu::Color::BLACK);
        let whole = Viewport::whole(size);
        match win.picture {
            Shown::Node { node, port } => viewer.show_node(
                &mut encoder,
                &view,
                size,
                &published,
                node,
                port,
                whole,
                Fit::Letterbox,
                0.0,
            ),
            Shown::Mix => {
                viewer.show_mixer(&mut encoder, &view, size, &published, whole, Fit::Letterbox);
            }
        }
        self.gpu.submit([encoder.finish()]);
        // The request, then the present that commits it. One outstanding at a time, always.
        surface.frame(&qh, surface.clone());
        win.pending = true;
        win.painted = std::time::Instant::now();
        self.gpu.queue().present(frame);
        // Only now: the submit carrying the blit is made.
        drop(published);
    }

    /// The safety net: a window with a surface, nobody's callback outstanding, somebody able
    /// to see it, and no paint for [`STALL`].
    ///
    /// It does **not** wake a hidden window. A compositor stops sending callbacks for a
    /// surface it is not compositing, and that window's one outstanding callback is what it
    /// waits on — arriving when the surface is composited again. Nudging it instead would be
    /// a picture nobody can see, repainted for ever.
    fn nudge(&mut self) {
        let stale: Vec<wl_surface::WlSurface> = self
            .windows
            .iter()
            .filter(|w| w.watchable() && w.painted.elapsed() > STALL)
            .map(|w| w.window.wl_surface().clone())
            .collect();
        for surface in stale {
            self.paint(&surface);
        }
    }

    /// A press inside a window. Two in quick succession are a double-click and so
    /// fullscreen; otherwise the press is *remembered* and becomes a move or a resize only
    /// once the pointer has travelled — see [`DRAG_SLOP`].
    fn pressed(&mut self, surface: &wl_surface::WlSurface, at: (f64, f64), serial: u32, time: u32) {
        let double = self.last_press.as_ref().is_some_and(|(then, what)| {
            what == surface && time.saturating_sub(*then) < DOUBLE_CLICK_MS
        });
        let Some(win) = self.at(surface) else {
            self.last_press = None;
            return;
        };
        let (fullscreen, logical) = (win.fullscreen, win.logical);
        if double {
            set_fullscreen(&win.window, !fullscreen);
            // Cleared, so a third press starts a fresh pair rather than toggling again.
            self.last_press = None;
            self.press = None;
            return;
        }
        self.last_press = Some((time, surface.clone()));
        // Fullscreen there is nothing to drag a window to, and a compositor asked to move a
        // fullscreen surface does something nobody wanted.
        if fullscreen {
            return;
        }
        let size = (f64::from(logical.0), f64::from(logical.1));
        self.press = Some(Press {
            surface: surface.clone(),
            serial,
            at,
            edge: edge_at(at, size, BAND),
        });
    }

    /// The pointer moved with the button down: past the slop, the window is handed to the
    /// compositor and the gesture is over as far as this thread is concerned.
    fn dragged(&mut self, surface: &wl_surface::WlSurface, at: (f64, f64)) {
        let Some(press) = self.press.as_ref() else {
            return;
        };
        if &press.surface != surface {
            return;
        }
        if (at.0 - press.at.0).hypot(at.1 - press.at.1) < DRAG_SLOP {
            return;
        }
        let Some(press) = self.press.take() else {
            return;
        };
        let Some(seat) = self.seat.clone() else {
            return;
        };
        let Some(win) = self.at(&press.surface) else {
            return;
        };
        if let Some(edge) = press.edge {
            win.resizing = Some(edge);
            win.window.resize(&seat, press.serial, resize_edge(edge));
        } else {
            win.window.move_(&seat, press.serial);
        }
    }

    /// The cursor for wherever the pointer is: a resize shape over a band, the arrow
    /// elsewhere. Set only when it changes, since a motion inside one band would otherwise
    /// be a request per event.
    fn point(&mut self, surface: &wl_surface::WlSurface, at: (f64, f64)) {
        let Some(win) = self.at(surface) else {
            return;
        };
        // A fullscreen window has no bands, so it has no resize cursor either.
        let band = if win.fullscreen { 0.0 } else { BAND };
        let size = (f64::from(win.logical.0), f64::from(win.logical.1));
        let icon = cursor_for(edge_at(at, size, band));
        if self.cursor == Some(icon) {
            return;
        }
        if let Some(pointer) = self.pointer.as_ref()
            && pointer.set_cursor(&self.conn, icon).is_ok()
        {
            self.cursor = Some(icon);
        }
    }

    /// Where the pointer is over this window's picture, for the graph.
    ///
    /// **Against the picture, not against the window.** The blit letterboxes, so a
    /// fullscreen window whose picture is a different shape has bars down two of its sides;
    /// a hand on a bar is not on the picture and reads nothing, exactly as a hand outside
    /// the window does. A window whose picture has not been drawn yet has no aspect and so
    /// reads nothing either.
    fn report(&mut self, surface: &wl_surface::WlSurface) {
        let Some(at) = self.at else { return };
        let published = self.live.get();
        let Some(win) = self.windows.iter().find(|w| w.matches(surface)) else {
            return;
        };
        let picture = match win.picture {
            Shown::Node { node, port } => published.picture(node, port),
            Shown::Mix => published.mixer.clone(),
        };
        let aspect = picture.map_or(0.0, |p| p.width as f32 / p.height.max(1) as f32);
        let size = (win.logical.0 as f32, win.logical.1 as f32);
        let (left, right) = self.held;
        let reading = crate::pointer::place((at.0 as f32, at.1 as f32), size, aspect)
            .map(|r| crate::pointer::Reading { left, right, ..r });
        self.feed.set(crate::pointer::Surface::Picture, reading);
    }

    /// The pointer has left every picture window, so the graph reads nothing rather than
    /// the last place the hand was.
    fn unreport(&mut self) {
        self.at = None;
        self.held = (false, false);
        self.feed.set(crate::pointer::Surface::Picture, None);
    }

    /// `F` fullscreens, `Escape` leaves fullscreen and a second `Escape` closes — which is
    /// what a picture with no title bar needs a key for.
    fn keyed(&mut self, key: u32) {
        let Some(surface) = self.focus.clone() else {
            return;
        };
        let Some(win) = self.at(&surface) else {
            return;
        };
        match key {
            KEY_F => set_fullscreen(&win.window, !win.fullscreen),
            KEY_ESCAPE => match escaped(win.fullscreen, win.opened_fullscreen) {
                Escaped::Windowed => set_fullscreen(&win.window, false),
                Escaped::Closed => {
                    let picture = win.picture;
                    let _ = self.told.send(Told::Closed(picture));
                    self.close(picture);
                }
            },
            _ => {}
        }
    }

    /// Every window freed, surface before `wl_surface`, before eframe tears down the display
    /// they were made on.
    fn shut_down(&mut self) {
        self.unreport();
        for win in std::mem::take(&mut self.windows) {
            Self::drop_window(win);
        }
        self.viewers.clear();
    }
}

/// The app id a picture window carries, which is the only thing a Wayland desktop has to
/// find its icon with: it matches this against the `.desktop` files it knows and takes the
/// `Icon=` from the one that matches.
///
/// Two, so the two kinds of window are two entries with two icons — the pop-out mark and the
/// fullscreen mark, the same marks that opened them. Neither is the editor's `supersilvia`,
/// which is the point: a picture window is not the app, and a taskbar that groups it with
/// the app hides it behind the thing it was opened out of. The entries and their icons are
/// in `packaging/linux/`; a session with neither installed falls back to the desktop's own
/// unknown-window icon, which is what happens today for every window here.
///
/// **Set once, when the window is made, and never again** — so a pop-out that `F` makes
/// fullscreen keeps the pop-out icon for as long as it lives. The protocol allows a later
/// `set_app_id`, and it was tried: KWin re-resolves it, fails to find the new entry on a
/// window it has already mapped, and falls back to the editor's own icon. A mark that is
/// merely out of date beats one that turns into the wrong thing entirely.
fn app_id(fullscreen: bool) -> &'static str {
    if fullscreen {
        "supersilvia-fullscreen"
    } else {
        "supersilvia-popout"
    }
}

/// Configure a window's surface again at `physical` pixels, where it is not already.
fn resize(device: &wgpu::Device, target: &mut Surface, physical: (u32, u32)) {
    let (width, height) = (physical.0.max(1), physical.1.max(1));
    if (target.config.width, target.config.height) != (width, height) {
        target.config.width = width;
        target.config.height = height;
        target.surface.configure(device, &target.config);
    }
}

fn set_fullscreen(window: &Window, to: bool) {
    if to {
        window.set_fullscreen(None);
    } else {
        window.unset_fullscreen();
    }
}

impl CompositorHandler for Pictures {
    /// A window carried to a display of another scale. The surface keeps its logical size
    /// and gains pixels: the buffer scale tells the compositor, and the wgpu surface is
    /// configured again to match, because nothing else will.
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        let device = self.gpu.device().clone();
        let Some(win) = self.at(surface) else {
            return;
        };
        win.scale = new_factor.max(1);
        let scale = win.scale as u32;
        win.physical = (win.logical.0 * scale, win.logical.1 * scale);
        let physical = win.physical;
        if let Some(target) = win.surface.as_mut() {
            resize(&device, target, physical);
        }
        surface.set_buffer_scale(new_factor.max(1));
        surface.commit();
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    /// The window's clock: the callback it asked for has arrived, so another may be asked
    /// for, which `paint` does.
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if let Some(win) = self.at(surface) {
            win.pending = false;
        }
        self.paint(surface);
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
        if let Some(win) = self.at(surface) {
            win.on_outputs += 1;
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
        if let Some(win) = self.at(surface) {
            win.on_outputs = win.on_outputs.saturating_sub(1);
        }
    }
}

impl WindowHandler for Pictures {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        let Some(win) = self.windows.iter().find(|w| &w.window == window) else {
            return;
        };
        let picture = win.picture;
        let _ = self.told.send(Told::Closed(picture));
        self.close(picture);
    }

    /// The compositor says how big the window is, and this is where the wgpu surface is made
    /// or resized to match. Nothing else decides a picture window's size.
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.windows.iter().position(|w| &w.window == window) else {
            return;
        };
        let fullscreen = configure.is_fullscreen();
        let opened_at = self.windows[index].opened_at;
        let scale = self.windows[index].scale.max(1) as u32;
        // A configure with no size is "you choose", which is the size the window asked for.
        let mut logical = (
            configure
                .new_size
                .0
                .map_or(opened_at.0, std::num::NonZero::get),
            configure
                .new_size
                .1
                .map_or(opened_at.1, std::num::NonZero::get),
        );
        // **The aspect lock.** A configure is a proposal, and a client may answer with a
        // size of its own; while a resize is running with `Shift` or `Ctrl` held, the answer
        // is the largest size at the picture's aspect that fits inside it. The buffer
        // committed at that size is the answer — sctk has already acked the configure.
        let dragging = self.windows[index].resizing;
        if let Some(edge) = dragging
            && !fullscreen
            && self.mods & (MOD_SHIFT | MOD_CTRL) != 0
        {
            let aspect = f64::from(opened_at.0) / f64::from(opened_at.1.max(1));
            logical = fit_aspect(logical, aspect, edge);
        }
        if dragging.is_some() && !configure.is_resizing() {
            self.windows[index].resizing = None;
        }
        let physical = (logical.0 * scale, logical.1 * scale);

        // **The surface is opaque**, which saves the compositor blending the desktop under
        // every pixel of a picture. It is true because the surface is configured opaque and
        // the blit over its black clear leaves alpha at one; the two say the same thing to two
        // different readers.
        let region = Region::new(&self.compositor).ok();
        if let Some(region) = region.as_ref() {
            region.add(0, 0, logical.0 as i32, logical.1 as i32);
            window
                .wl_surface()
                .set_opaque_region(Some(region.wl_region()));
        }

        let first = self.windows[index].surface.is_none();
        let made = if first {
            match self.surface_for(window.wl_surface(), physical) {
                Ok(surface) => Some(surface),
                Err(err) => {
                    let picture = self.windows[index].picture;
                    let _ = self.told.send(Told::Failed(picture, err));
                    self.close(picture);
                    return;
                }
            }
        } else {
            None
        };
        let device = self.gpu.device().clone();
        let win = &mut self.windows[index];
        let told = win.fullscreen != fullscreen;
        win.fullscreen = fullscreen;
        win.logical = logical;
        win.physical = physical;
        match (&mut win.surface, made) {
            (Some(target), _) => resize(&device, target, physical),
            (slot @ None, made) => *slot = made,
        }
        let picture = self.windows[index].picture;
        if told {
            let _ = self.told.send(Told::Fullscreen(picture, fullscreen));
        }
        if first {
            // The one path to fullscreen, opening included: a command from here, where the
            // window is mapped and the compositor will act on it.
            if let Some(want) = self.windows[index].want_fullscreen.take() {
                set_fullscreen(window, want);
            }
            // **Paint now**, rather than leaving the first frame to the watchdog: that would
            // put up to a `STALL` between the window appearing and anything being in it.
            // A surface that has had no `enter` yet still paints, because painting is what
            // makes the compositor map it.
            let surface = window.wl_surface().clone();
            self.paint(&surface);
        }
    }
}

impl SeatHandler for Pictures {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seat.get_or_insert(seat);
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        self.seat.get_or_insert(seat.clone());
        match capability {
            Capability::Pointer if self.pointer.is_none() => {
                // A themed pointer, for the cursor alone: its surface is the cursor's own,
                // which is why one is made here and never painted into by this thread.
                let cursor = self.compositor.create_surface(qh);
                self.pointer = self
                    .seats
                    .get_pointer_with_theme(
                        qh,
                        &seat,
                        self.shm.wl_shm(),
                        cursor,
                        ThemeSpec::default(),
                    )
                    .ok();
                self.cursor = None;
            }
            // Bound by hand rather than through sctk, whose keyboard needs an xkb keymap and
            // so the `xkbcommon` crate. The two keys a picture window reads are read as evdev
            // positions instead — see `KEY_F`.
            Capability::Keyboard if self.keyboard.is_none() => {
                self.keyboard = Some(seat.get_keyboard(qh, ()));
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            // `ThemedPointer` releases the pointer and destroys its cursor surface in
            // its own `Drop`, which is what taking it here does.
            Capability::Pointer => {
                self.pointer = None;
                self.cursor = None;
            }
            Capability::Keyboard => {
                if let Some(k) = self.keyboard.take() {
                    k.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {
        self.seat = None;
    }
}

impl PointerHandler for Pictures {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                // The cursor is reloaded on every enter, which is what a `wl_pointer` asks
                // of a client: whatever the last surface set is not this surface's.
                PointerEventKind::Enter { .. } => {
                    self.cursor = None;
                    self.at = Some(event.position);
                    self.point(&event.surface, event.position);
                    self.report(&event.surface);
                }
                PointerEventKind::Motion { .. } => {
                    self.at = Some(event.position);
                    self.dragged(&event.surface, event.position);
                    self.point(&event.surface, event.position);
                    self.report(&event.surface);
                }
                PointerEventKind::Press {
                    button,
                    serial,
                    time,
                } => {
                    self.at = Some(event.position);
                    match button {
                        BTN_LEFT => {
                            self.held.0 = true;
                            self.pressed(&event.surface, event.position, serial, time);
                        }
                        BTN_RIGHT => self.held.1 = true,
                        _ => {}
                    }
                    self.report(&event.surface);
                }
                PointerEventKind::Release { button, .. } => {
                    self.at = Some(event.position);
                    match button {
                        BTN_LEFT => {
                            self.held.0 = false;
                            self.press = None;
                        }
                        BTN_RIGHT => self.held.1 = false,
                        _ => {}
                    }
                    self.report(&event.surface);
                }
                // A compositor grab — the move or resize just asked for — takes the
                // pointer away with a leave, and so does the hand. Either way the gesture is
                // over and there is nothing left to hold.
                PointerEventKind::Leave { .. } => {
                    self.press = None;
                    self.cursor = None;
                    self.unreport();
                }
                PointerEventKind::Axis { .. } => {}
            }
        }
    }
}

impl OutputHandler for Pictures {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

/// The keyboard, raw: which window has the focus, and the two keys it reads.
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Pictures {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => {
                if state.windows.iter().any(|w| w.matches(&surface)) {
                    state.focus = Some(surface);
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                if state.focus.as_ref() == Some(&surface) {
                    state.focus = None;
                }
                // A modifier held as the focus leaves is not held here any more, and a
                // stale bit would lock the aspect of the next resize nobody asked to lock.
                state.mods = 0;
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.mods = mods_depressed;
            }
            wl_keyboard::Event::Key {
                key,
                state: wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } => state.keyed(key),
            _ => {}
        }
    }
}

impl ProvidesRegistryState for Pictures {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

impl ShmHandler for Pictures {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Pictures);
delegate_shm!(Pictures);
delegate_output!(Pictures);
delegate_seat!(Pictures);
delegate_pointer!(Pictures);
delegate_xdg_shell!(Pictures);
delegate_xdg_window!(Pictures);
delegate_registry!(Pictures);
