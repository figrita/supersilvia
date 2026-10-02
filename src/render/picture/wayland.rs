// SPDX-License-Identifier: AGPL-3.0-or-later

//! The editor's side of the pictures thread on Linux: eframe's `wl_display`, borrowed, and
//! the thread in [`super::thread`] that draws every window on it.
//!
//! This file and `thread.rs` hold all of `render::picture`'s `unsafe`, and are the only two
//! that name the Wayland, sctk and calloop crates; `Cargo.toml` declares those for Linux
//! alone. `macos.rs` answers the same names everywhere else.

use super::thread;
use super::{Ask, Told};
use std::sync::mpsc::{Receiver, TryRecvError};

/// The pictures thread, from the editor's side.
///
/// `None` behind everything where there is no thread — no Wayland, no GPU, or a
/// headless run — in which case an `Ask` goes nowhere and [`Self::why_not`] is the sentence
/// saying so.
pub struct Host {
    tx: Option<calloop::channel::Sender<Ask>>,
    told: Option<Receiver<Told>>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Why there are no picture windows. **Latched**: once the thread is gone — it said so,
    /// or a send failed — this is set and every later ask is answered from it rather than
    /// dropped into a channel nobody reads.
    why_not: Option<String>,
}

impl Default for Host {
    fn default() -> Self {
        Self::detached()
    }
}

impl Host {
    /// No thread, and no window will ever open: every ask is refused with this reason, so
    /// the editor never keeps a window that cannot exist. A real session with no Wayland.
    pub fn none(why: impl Into<String>) -> Self {
        Self {
            tx: None,
            told: None,
            handle: None,
            why_not: Some(why.into()),
        }
    }

    /// The thread for an eframe session, or the reason there is none.
    ///
    /// The **safe** door, and the only one the app uses: the `wl_display` is eframe's own,
    /// read off its display handle here so that `unsafe` stays inside `render/`. `gpu` is
    /// the editor's device, which every picture window blits on; `None` is a session with no
    /// GPU, and a display handle that is not Wayland's is a session with no picture windows —
    /// there is no second path.
    pub fn for_eframe(
        cc: &eframe::CreationContext<'_>,
        gpu: Option<&crate::render::Gpu>,
        live: std::sync::Arc<crate::render::Live>,
        pointer: std::sync::Arc<crate::pointer::Feed>,
    ) -> Self {
        use raw_window_handle::{HasDisplayHandle as _, RawDisplayHandle};
        // No GPU at all is a harness — egui_kittest hands the app none — and a harness is
        // detached rather than refused: the whole graphical half is absent there, and what a
        // test asserts on is the editor's own list.
        if cc.wgpu_render_state.is_none() {
            return Self::detached();
        }
        let Some(gpu) = gpu else {
            return Self::none("picture windows need a GPU");
        };
        let display = match cc.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(w)) => w.display,
            _ => return Self::none("picture windows need Wayland; this session is not one"),
        };
        // SAFETY: the display is eframe's, which owns it for the life of its event loop, and
        // the thread is joined in `Host::stop` before eframe tears that loop down. The
        // backend below borrows it and never frees it.
        unsafe { Self::spawn(display, gpu.clone(), live, pointer) }
    }

    /// Start the thread on eframe's own `wl_display`, blitting on `gpu`.
    ///
    /// `display` is the `wl_display` out of eframe's raw display handle — **borrowed**, never
    /// freed here, and valid for as long as eframe's event loop is, which is why
    /// [`Self::stop`] runs before eframe tears down.
    ///
    /// # Safety
    /// `display` must be a live `wl_display` that outlives this thread.
    pub unsafe fn spawn(
        display: std::ptr::NonNull<std::ffi::c_void>,
        gpu: crate::render::Gpu,
        live: std::sync::Arc<crate::render::Live>,
        pointer: std::sync::Arc<crate::pointer::Feed>,
    ) -> Self {
        let (tx, rx) = calloop::channel::channel();
        let (told_tx, told_rx) = std::sync::mpsc::channel();
        // Carried to the thread as an address, because a raw pointer is not `Send` and
        // there is nothing to assert about an integer.
        let display = display.as_ptr() as usize;
        let handle = std::thread::Builder::new()
            .name("pictures".into())
            .spawn(move || {
                // SAFETY: as above — the pointer is eframe's `wl_display` and outlives this.
                unsafe { thread::run(display as *mut _, &gpu, &live, &pointer, rx, &told_tx) };
            });
        match handle {
            Ok(handle) => Self {
                tx: Some(tx),
                told: Some(told_rx),
                handle: Some(handle),
                why_not: None,
            },
            Err(err) => {
                log::error!("the pictures thread could not be started: {err}");
                Self::none("the pictures thread could not be started")
            }
        }
    }

    /// No thread, and nothing refused: the asks go nowhere and the editor's list behaves as
    /// though the windows opened.
    ///
    /// What a run with no windowing at all gets — `App::headless`, every layer-1 test and
    /// egui_kittest — for the reason `has_gpu` already makes a picture a placeholder there:
    /// the whole graphical half is absent, and a test asserting on the marks is asserting
    /// about the editor's list, which is the half that is present.
    pub fn detached() -> Self {
        Self {
            tx: None,
            told: None,
            handle: None,
            why_not: None,
        }
    }

    /// Why there are no picture windows, where there are none.
    pub fn why_not(&self) -> Option<&str> {
        self.why_not.as_deref()
    }

    /// Send an ask. Where there is a thread it goes to it; where there is a *reason* there
    /// is none, the ask comes straight back as a failure, so the editor's own list never
    /// keeps a window that will not exist. A detached host swallows it.
    pub fn send(&self, ask: Ask) -> Option<Told> {
        let Some(tx) = &self.tx else {
            let why = self.why_not.clone()?;
            let picture = match &ask {
                Ask::Open { picture, .. } | Ask::Close(picture) | Ask::Fullscreen(picture, _) => {
                    *picture
                }
            };
            return Some(Told::Failed(picture, why));
        };
        // A closed channel is a thread that stopped, which is a run that is ending.
        let _ = tx.send(ask);
        None
    }

    /// Everything the thread has said since the last frame.
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

    /// Close every window and wait for the thread, so its surfaces are gone
    /// before eframe tears down the `wl_display` they were made on.
    pub fn stop(&mut self) {
        self.tx = None;
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop();
    }
}
