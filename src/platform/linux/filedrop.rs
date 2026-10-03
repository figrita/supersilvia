// SPDX-License-Identifier: AGPL-3.0-or-later

//! Files dragged onto the editor's window from another app, on Wayland.
//!
//! **winit has no drag and drop on Wayland**: 0.30 sends `DroppedFile` on X11, Windows and
//! macOS and nothing here, so egui never hears of a drop. This is that half, on a thread of
//! its own: a `wl_data_device` for the seat, made on eframe's `wl_display` through a queue of
//! ours — `Backend::from_foreign_display`, as the picture windows borrow it — which hears the
//! compositor say a drag entered a surface, moved over it, left it or was dropped on it.
//!
//! **Only the editor's surface, and only files.** A drag over any other surface of ours — a
//! picture window — is refused, and so is one that offers no `text/uri-list`, which is how a
//! file manager says *these are files*. The list is read when the drag enters, so the hint
//! can say what the drop will make, and again on the drop if that read came back empty.
//! Reading is a pipe the source writes into, on a helper thread with a deadline: a source that
//! never writes costs a helper, not this thread.
//!
//! What crosses to the editor is plain data, a [`Drag`], with a wake so a drag over a window
//! that is not drawing gets a frame. The editor turns it into egui's hovered and dropped files.

use std::io::Read as _;
use std::os::fd::AsFd as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use wayland_backend::client::Backend;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

/// The one type a file manager offers files as: a URI per line.
const URI_LIST: &str = "text/uri-list";
/// How long a read of the list may take before it is given up on.
const READ_DEADLINE: Duration = Duration::from_secs(2);

/// What a drag did over the editor's window. Positions are the surface's, in logical pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Drag {
    /// Files came over the window. `files` is empty where the source would not say which.
    Entered { files: Vec<PathBuf>, at: (f32, f32) },
    Moved((f32, f32)),
    /// They left, or the drag was cancelled.
    Left,
    Dropped { files: Vec<PathBuf>, at: (f32, f32) },
}

/// The drop thread, from the editor's side. `None` behind everything where there is none: not
/// Wayland, which winit already serves, or a harness.
pub struct FileDrop {
    drags: Option<Receiver<Drag>>,
    stop: Option<calloop::channel::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FileDrop {
    pub fn none() -> Self {
        Self {
            drags: None,
            stop: None,
            handle: None,
        }
    }

    /// The thread for this window, if its display is Wayland's. `wake` is called after each
    /// [`Drag`] is sent, from the thread: the editor's request for a frame.
    ///
    /// The display and the surface are eframe's, borrowed: [`Self::stop`] must run before
    /// eframe tears its event loop down, which `App::on_exit` does.
    pub fn for_window(
        window: &(impl raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle),
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
        let Ok(RawDisplayHandle::Wayland(display)) = window.display_handle().map(|h| h.as_raw())
        else {
            return Self::none();
        };
        // Without the surface every surface of ours is taken for the editor's, which is
        // still right for a session with no picture window open.
        let surface = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Wayland(w)) => Some(w.surface.as_ptr() as usize),
            _ => None,
        };
        // Carried as an address: a raw pointer is not `Send`.
        let display = display.display.as_ptr() as usize;
        let (tx, rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = calloop::channel::channel();
        let wake: Box<dyn Fn() + Send> = Box::new(wake);
        let handle = std::thread::Builder::new()
            .name("file drops".into())
            .spawn(move || {
                // SAFETY: the display is eframe's, alive for as long as its event loop, and
                // this thread is joined in `FileDrop::stop` before that loop is torn down.
                // The backend borrows it and never frees it.
                let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
                if let Err(why) = run(backend, surface, tx, wake, stop_rx) {
                    log::warn!("file drops: {why}");
                }
            });
        match handle {
            Ok(handle) => Self {
                drags: Some(rx),
                stop: Some(stop_tx),
                handle: Some(handle),
            },
            Err(err) => {
                log::warn!("file drops: the thread could not be started: {err}");
                Self::none()
            }
        }
    }

    /// Everything the drag did since the last frame, in order.
    pub fn take(&self) -> Vec<Drag> {
        let Some(rx) = &self.drags else {
            return Vec::new();
        };
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(d) => out.push(d),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// End the thread and wait for it, while eframe's display is still alive.
    pub fn stop(&mut self) {
        self.stop = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for FileDrop {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The thread's state, which every Wayland event is dispatched into.
struct State {
    conn: Connection,
    /// The editor's `wl_surface`, by address.
    surface: Option<usize>,
    /// The drag over the editor now: its offer, and the files it said it carries.
    current: Option<(WlDataOffer, Vec<PathBuf>)>,
    /// Where it is over the surface.
    at: (f32, f32),
    tx: Sender<Drag>,
    wake: Box<dyn Fn() + Send>,
    stop: bool,
}

impl State {
    fn send(&self, drag: Drag) {
        let _ = self.tx.send(drag);
        (self.wake)();
    }

    /// Let go of the drag's offer, if there is one.
    fn forget(&mut self) {
        if let Some((offer, _)) = self.current.take() {
            offer.destroy();
        }
    }
}

fn run(
    backend: Backend,
    surface: Option<usize>,
    tx: Sender<Drag>,
    wake: Box<dyn Fn() + Send>,
    stop: calloop::channel::Channel<()>,
) -> Result<(), String> {
    let conn = Connection::from_backend(backend);
    let (globals, queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("no Wayland registry: {e}"))?;
    let qh = queue.handle();
    let manager: WlDataDeviceManager = globals
        .bind(&qh, 1..=3, ())
        .map_err(|e| format!("no wl_data_device_manager: {e}"))?;
    let seat: WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("no wl_seat: {e}"))?;
    let device = manager.get_data_device(&seat, &qh, ());

    let mut state = State {
        conn: conn.clone(),
        surface,
        current: None,
        at: (0.0, 0.0),
        tx,
        wake,
        stop: false,
    };
    let mut event_loop: calloop::EventLoop<State> =
        calloop::EventLoop::try_new().map_err(|e| format!("no event loop: {e}"))?;
    calloop_wayland_source::WaylandSource::new(conn, queue)
        .insert(event_loop.handle())
        .map_err(|e| format!("the Wayland source could not be added: {e}"))?;
    event_loop
        .handle()
        .insert_source(stop, |event, (), state| {
            if let calloop::channel::Event::Closed = event {
                state.stop = true;
            }
        })
        .map_err(|e| format!("the stop could not be listened for: {e}"))?;
    while !state.stop {
        if event_loop.dispatch(None, &mut state).is_err() {
            break;
        }
    }
    state.forget();
    // `release` is version 2's; a version 1 device is left, and goes with the connection.
    if device.version() >= 2 {
        device.release();
    }
    let _ = state.conn.flush();
    Ok(())
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

/// An offer's types, as it announces them before the drag enters.
type Types = Mutex<Vec<String>>;

impl Dispatch<WlDataOffer, Types> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        types: &Types,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Ok(mut types) = types.lock()
        {
            types.push(mime_type);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                state.forget();
                let Some(offer) = id else {
                    return;
                };
                let ours = state
                    .surface
                    .is_none_or(|s| surface.id().as_ptr() as usize == s);
                let files = offer
                    .data::<Types>()
                    .and_then(|t| t.lock().ok().map(|t| t.iter().any(|m| m == URI_LIST)))
                    .unwrap_or(false);
                if !(ours && files) {
                    offer.accept(serial, None);
                    offer.destroy();
                    return;
                }
                offer.accept(serial, Some(URI_LIST.to_owned()));
                // Version 3 negotiates an action, and a drag none was agreed for is refused
                // at the drop. A file brought in is copied into the project.
                if offer.version() >= 3 {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
                let files = read(&state.conn, &offer).unwrap_or_default();
                state.at = (x as f32, y as f32);
                state.send(Drag::Entered {
                    files: files.clone(),
                    at: state.at,
                });
                state.current = Some((offer, files));
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if state.current.is_some() {
                    state.at = (x as f32, y as f32);
                    state.send(Drag::Moved(state.at));
                }
            }
            wl_data_device::Event::Leave => {
                if state.current.is_some() {
                    state.forget();
                    state.send(Drag::Left);
                }
            }
            wl_data_device::Event::Drop => {
                let Some((offer, mut files)) = state.current.take() else {
                    return;
                };
                if files.is_empty() {
                    files = read(&state.conn, &offer).unwrap_or_default();
                }
                if offer.version() >= 3 {
                    offer.finish();
                }
                offer.destroy();
                state.send(Drag::Dropped {
                    files,
                    at: state.at,
                });
            }
            // The clipboard's offer, which is smithay-clipboard's business on its own device.
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Types::default()),
    ]);
}

/// The files an offer carries, read through a pipe the source writes its list into.
fn read(conn: &Connection, offer: &WlDataOffer) -> Option<Vec<PathBuf>> {
    let (mut reader, writer) = std::io::pipe().ok()?;
    offer.receive(URI_LIST.to_owned(), writer.as_fd());
    // The request carries its own copy of the descriptor, so ours goes now: the read below
    // ends when the source closes the last write end.
    drop(writer);
    conn.flush().ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("file drop read".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let _ = tx.send(reader.read_to_end(&mut bytes).map(|_| bytes));
        })
        .ok()?;
    let bytes = rx.recv_timeout(READ_DEADLINE).ok()?.ok()?;
    Some(parse_uri_list(&bytes))
}

/// The local files in a `text/uri-list`: one URI a line, `#` lines comments, `file:` the only
/// scheme that is a file here, and a host that is empty or `localhost`.
fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    bytes
        .split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && !line.starts_with(b"#"))
        .filter_map(|line| line.strip_prefix(b"file://"))
        .filter_map(|rest| {
            let slash = rest.iter().position(|&b| b == b'/')?;
            let (host, path) = rest.split_at(slash);
            (host.is_empty() || host == b"localhost").then_some(path)
        })
        .map(|path| PathBuf::from(std::ffi::OsString::from_vec(percent_decode(path))))
        .collect()
}

/// `%2F` and its kind back to their bytes. A `%` not followed by two hex digits is itself.
fn percent_decode(text: &[u8]) -> Vec<u8> {
    let hex = |b: u8| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                text.get(i + 1).copied().and_then(hex),
                text.get(i + 2).copied().and_then(hex),
            )
        {
            out.push(hi << 4 | lo);
            i += 3;
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_list_is_its_local_files() {
        let list = b"# from Dolphin\r\nfile:///home/a/clip.mp4\r\nfile://localhost/tmp/b.png\r\n\
                     https://example.com/c.png\r\nfile://elsewhere/d.png\r\n";
        assert_eq!(
            parse_uri_list(list),
            [
                PathBuf::from("/home/a/clip.mp4"),
                PathBuf::from("/tmp/b.png")
            ]
        );
    }

    #[test]
    fn a_path_is_percent_decoded_to_its_bytes() {
        assert_eq!(
            parse_uri_list(b"file:///home/a/my%20clip%E2%80%94final.mp4\n"),
            [PathBuf::from("/home/a/my clip\u{2014}final.mp4")]
        );
        assert_eq!(percent_decode(b"100%"), b"100%");
        assert_eq!(percent_decode(b"%zz"), b"%zz");
    }
}
