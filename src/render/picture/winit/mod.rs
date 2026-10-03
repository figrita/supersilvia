// SPDX-License-Identifier: AGPL-3.0-or-later

//! The picture windows on macOS and Windows: the editor's side of them, and the event loop they
//! live in.
//!
//! **Where each half runs.** AppKit makes and drives windows from the main thread only, and
//! Win32 delivers a window's messages to the thread that made it, which winit allows to be the
//! event loop's alone, so a thread that owns windows, as Linux's does, is not possible on
//! either. Each picture window is a winit window made on the main thread, by
//! [`event_loop::Loop`], which sits around eframe inside the one event loop [`run`] starts.
//! Each is drawn on a thread of its own, by [`draw`], on the one device.
//! `proposals/macos-windows.md` is the argument, written for the Mac.
//!
//! **How an ask crosses.** Every [`Ask`] comes from a mark, so it is sent from inside
//! `App::ui`, which runs inside winit's `RedrawRequested` on the main thread; winit calls
//! `about_to_wait` in the same turn of the loop, and `Loop` reads the ask there. So nothing
//! needs waking. [`Told`] goes back on a plain channel the editor drains once a frame.
//!
//! **How the two halves meet.** [`run`] makes both channels before eframe starts, keeps the
//! loop's ends in `Loop` and parks the editor's in a slot on the main thread. `Host::for_eframe`
//! takes them from there inside `App::new`, which eframe runs on the main thread, and sends the
//! loop the device, the synth's `Live` and the pointer feed. A harness never runs `run`, finds
//! no slot, and is detached, as it is on Linux.
//!
//! No `unsafe` and no Wayland crate: winit's safe API is the whole of it.

mod draw;
mod event_loop;

use super::{Ask, Told};
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

/// What reaches the main thread's `Loop` from the editor.
enum ToLoop {
    /// Sent once, from `App::new`: what every window draws with.
    Start(Start),
    Ask(Ask),
}

/// What a window's drawing needs, handed to the loop once the editor has it.
struct Start {
    gpu: crate::render::Gpu,
    live: Arc<crate::render::Live>,
    /// Where the pointer is, for the graph: a picture window is not an egui region, so the
    /// main thread's own pointer events are the only thing that sees a hand over the picture.
    pointer: Arc<crate::pointer::Feed>,
}

/// The editor's ends of the two channels, waiting for `App::new` to take them.
struct Door {
    to_loop: Sender<ToLoop>,
    told: Receiver<Told>,
}

thread_local! {
    /// Filled by [`run`] before eframe makes the app, and emptied by `Host::for_eframe` on the
    /// same thread. Empty wherever `run` did not start the loop, which is every harness.
    static DOOR: RefCell<Option<Door>> = const { RefCell::new(None) };
}

/// Start eframe inside a winit event loop of our own, and run it until the app quits.
///
/// `eframe::run_native`'s three arguments. The loop is ours so that [`event_loop::Loop`] can
/// sit around eframe on the main thread, which is where AppKit and winit's Win32 backend make
/// windows.
///
/// # Errors
/// What `eframe::run_native` returns: the event loop could not be made or run.
pub fn run(
    app_name: &str,
    options: eframe::NativeOptions,
    creator: eframe::AppCreator<'_>,
) -> eframe::Result {
    let event_loop =
        winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build()?;
    let (to_loop, inbox) = std::sync::mpsc::channel();
    let (told_tx, told) = std::sync::mpsc::channel();
    DOOR.with(|door| *door.borrow_mut() = Some(Door { to_loop, told }));
    let eframe = eframe::create_native(app_name, options, creator, &event_loop);
    event_loop.run_app(&mut event_loop::Loop::new(eframe, inbox, told_tx))?;
    Ok(())
}

/// The picture windows, from the editor's side.
///
/// `None` behind everything where there are none — no GPU, or a harness — in which case an
/// `Ask` goes nowhere and [`Self::why_not`] is the sentence saying so.
pub struct Host {
    to_loop: Option<Sender<ToLoop>>,
    told: Option<Receiver<Told>>,
    why_not: Option<String>,
}

impl Default for Host {
    fn default() -> Self {
        Self::detached()
    }
}

impl Host {
    /// No windows, and none will ever open: every ask is refused with this reason.
    pub fn none(why: impl Into<String>) -> Self {
        Self {
            to_loop: None,
            told: None,
            why_not: Some(why.into()),
        }
    }

    /// The windows for an eframe session, or the reason there are none. Read in the same
    /// order as Linux's: no GPU at all is a harness and is detached, and a session with no
    /// device of its own is refused for that.
    pub fn for_eframe(
        cc: &eframe::CreationContext<'_>,
        gpu: Option<&crate::render::Gpu>,
        live: Arc<crate::render::Live>,
        pointer: Arc<crate::pointer::Feed>,
    ) -> Self {
        if cc.wgpu_render_state.is_none() {
            return Self::detached();
        }
        let Some(gpu) = gpu else {
            return Self::none("picture windows need a GPU");
        };
        let Some(door) = DOOR.with(|door| door.borrow_mut().take()) else {
            return Self::none("picture windows need the event loop `render::picture::run` starts");
        };
        let start = Start {
            gpu: gpu.clone(),
            live,
            pointer,
        };
        if door.to_loop.send(ToLoop::Start(start)).is_err() {
            return Self::none("the picture windows' event loop has stopped");
        }
        Self {
            to_loop: Some(door.to_loop),
            told: Some(door.told),
            why_not: None,
        }
    }

    /// No windows, and nothing refused: the asks go nowhere and the editor's list behaves as
    /// though the windows opened. What a run with no windowing at all gets.
    pub fn detached() -> Self {
        Self {
            to_loop: None,
            told: None,
            why_not: None,
        }
    }

    /// Why there are no picture windows, where there are none.
    pub fn why_not(&self) -> Option<&str> {
        self.why_not.as_deref()
    }

    /// Send an ask to the main thread's loop, which acts on it later in this same turn of the
    /// loop. Where there is a reason there are no windows, the ask comes straight back as a
    /// failure, so the editor's own list never keeps a window that will not exist; a detached
    /// host swallows it.
    pub fn send(&self, ask: Ask) -> Option<Told> {
        let picture = match &ask {
            Ask::Open { picture, .. } | Ask::Close(picture) | Ask::Fullscreen(picture, _) => {
                *picture
            }
        };
        let Some(to_loop) = &self.to_loop else {
            let why = self.why_not.clone()?;
            return Some(Told::Failed(picture, why));
        };
        // A closed channel is a loop that has stopped, which is a run that is ending.
        to_loop
            .send(ToLoop::Ask(ask))
            .err()
            .map(|_| Told::Failed(picture, "the picture windows have stopped".to_string()))
    }

    /// Everything the windows have said since the last frame.
    pub fn told(&self) -> Vec<Told> {
        let Some(rx) = &self.told else {
            return Vec::new();
        };
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(t) => out.push(t),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Let go of the loop. The windows themselves are closed by `Loop` on its way out, after
    /// eframe's own exit, and nothing here waits: no window is made on anything the editor
    /// tears down.
    pub fn stop(&mut self) {
        self.to_loop = None;
    }
}
