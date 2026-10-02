// SPDX-License-Identifier: AGPL-3.0-or-later

//! Screen capture: asking the desktop for a screen, and holding the answer open.
//!
//! On Wayland an application cannot read the screen. It asks **xdg-desktop-portal**, the
//! desktop puts up its own picker — KDE's on this machine, GNOME's on another — and what comes
//! back is a PipeWire remote: one file descriptor and one node number. `pipewiresrc` reads
//! that node, and from there it is the same pipeline a camera is.
//!
//! Three things about this shape are load-bearing.
//!
//! **The session must outlive the pipeline.** The portal stops the stream when the session
//! ends, and the session ends when the D-Bus object is dropped. So the negotiation cannot be
//! a function that returns a number: [`Cast`] holds the session open for as long as anything
//! is reading it, and dropping it is how a capture is stopped.
//!
//! **It has to happen off the frame thread.** A picker is up until a hand answers it, which is
//! unbounded, and the frame thread is not allowed to wait for anything. So the conversation is
//! a task on [`super::portal`]'s shared runtime and the answer arrives through a channel a
//! tick polls — the shape [`super::files`] uses for file dialogs.
//!
//! **And it must never block that runtime.** Read [`super::portal`] before touching this: one
//! blocking wait in here is a screen capture that runs for a minute and then dies, and a
//! portal that never answers again.
//!
//! **It always asks.** The portal offers a restore token that would resume the same pick with
//! no dialog, and it is thrown away. Which window is being captured is the rig's, the rig does
//! not persist, and a source you chose by hand that silently hands you last week's window is
//! worse than one more click.

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use std::os::fd::{AsRawFd as _, OwnedFd, RawFd};
use std::sync::mpsc;
use tokio::sync::oneshot;

/// A running screen capture, as the rest of the app holds it.
///
/// The fields are what a pipeline needs; the value's *existence* is what keeps the portal
/// session alive. Dropping it tells the negotiating thread to close the session and stops the
/// desktop's "sharing your screen" indicator.
pub struct Cast {
    /// The PipeWire node the portal chose, for `pipewiresrc path=`.
    node: u32,
    /// The open PipeWire remote, for `pipewiresrc fd=`. GStreamer borrows it; this owns it,
    /// so it stays valid exactly as long as the capture does.
    fd: OwnedFd,
    /// Dropped to end the session: the task is awaiting the other end.
    _stop: oneshot::Sender<()>,
}

impl Cast {
    /// What a pipeline reads this cast by, valid for as long as this value is alive.
    pub fn stream(&self) -> Stream {
        Stream {
            fd: self.fd.as_raw_fd(),
            node: self.node,
        }
    }
}

/// A cast as a pipeline's source names it: the PipeWire remote's descriptor and the node on
/// it. The [`Cast`] it came from must outlive the pipeline — it is what holds the portal
/// session open, and the descriptor is only valid while it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stream {
    fd: RawFd,
    node: u32,
}

impl Stream {
    /// The pipeline's source element for this stream.
    pub fn head(&self) -> crate::platform::screen::Head {
        crate::platform::screen::Head::Element(format!(
            "pipewiresrc fd={} path={}",
            self.fd, self.node
        ))
    }
}

/// A negotiation in flight: the picker may be on screen, or the person may be ignoring it.
pub struct Pending(mpsc::Receiver<Result<Cast, String>>);

impl Pending {
    /// The answer, if there is one yet. Never waits.
    ///
    /// `None` means still asking. `Some(Err)` covers both a refusal and a failure, because
    /// there is nothing useful for a caller to do differently about the two: either way there
    /// is no screen and the reason belongs in the status line.
    pub fn poll(&self) -> Option<Result<Cast, String>> {
        match self.0.try_recv() {
            Ok(answer) => Some(answer),
            Err(mpsc::TryRecvError::Empty) => None,
            // The thread died without answering. A cancel, as far as anything here is
            // concerned.
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("screen capture was canceled".to_string()))
            }
        }
    }
}

/// Ask the desktop for a screen. Returns at once; the picker appears on a task of its own.
///
/// **Always asks.** The portal will hand back a restore token that would resume the same pick
/// with no dialog, and it is deliberately thrown away: which window you are capturing is the
/// rig's, the rig does not persist, and being silently handed last week's window when you
/// chose *Screen or window* is worse than one click.
pub fn ask() -> Pending {
    let (answer_tx, answer_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    // Spawned onto the shared runtime rather than onto a runtime of its own, and **nothing
    // here blocks it**. Both halves of that are load-bearing: `ashpd` caches one D-Bus
    // connection for the process, its socket is pumped by a task on whichever runtime made
    // it, and a runtime that goes away — or that some future is sitting on a blocking `recv`
    // inside — stops pumping. The session then dies quietly a minute later and every portal
    // call after it reuses a connection nobody is reading. See [`super::portal`].
    super::portal::runtime().spawn(async move {
        match negotiate(stop_tx).await {
            Ok((cast, session)) => {
                if answer_tx.send(Ok(cast)).is_err() {
                    return;
                }
                // Awaited, not blocked on: this task yields, so the connection's own task goes
                // on running underneath it. The `Cast` on the other end owns the sending half,
                // so this resolves when the capture is dropped — which is what closes the
                // session and takes the desktop's screen-sharing indicator down.
                let _ = stop_rx.await;
                let _ = session.close().await;
            }
            Err(e) => {
                let _ = answer_tx.send(Err(e));
            }
        }
    });
    Pending(answer_rx)
}

/// The portal conversation itself. Returns the cast and the session that must outlive it.
async fn negotiate(
    stop: oneshot::Sender<()>,
) -> Result<(Cast, ashpd::desktop::Session<'static, Screencast<'static>>), String> {
    let proxy = Screencast::new().await.map_err(portal)?;
    let session = proxy.create_session().await.map_err(portal)?;
    proxy
        .select_sources(
            &session,
            // The pointer is part of what is on screen, and a VJ feeding a screen into a
            // synth wants what is on it. Embedded rather than metadata: the frame is going
            // straight into a texture, and nothing downstream would draw a separate cursor.
            CursorMode::Embedded,
            // A monitor, a window, or a region, and let the desktop offer whichever of the
            // three it implements.
            SourceType::Monitor | SourceType::Window | SourceType::Virtual,
            false,
            None,
            // Nothing is persisted, so the portal is asked not to remember anything either:
            // a permission it kept would be the same silent resume by another route.
            PersistMode::DoNot,
        )
        .await
        .map_err(portal)?;
    let streams = proxy
        .start(&session, None)
        .await
        .map_err(portal)?
        .response()
        .map_err(portal)?;
    let stream = streams
        .streams()
        .first()
        .ok_or_else(|| "no screen was chosen".to_string())?;
    let node = stream.pipe_wire_node_id();
    let fd = proxy
        .open_pipe_wire_remote(&session)
        .await
        .map_err(portal)?;
    Ok((
        Cast {
            node,
            fd,
            _stop: stop,
        },
        session,
    ))
}

/// A portal error as a line for the status bar.
///
/// A canceled dialog arrives here as an error like any other, and it says so plainly rather
/// than in D-Bus's words: a person who closed the picker did not encounter a fault.
fn portal(e: ashpd::Error) -> String {
    match e {
        ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled) => {
            "no screen was chosen".to_string()
        }
        other => format!("screen capture: {other}"),
    }
}
