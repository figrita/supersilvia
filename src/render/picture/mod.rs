// SPDX-License-Identifier: AGPL-3.0-or-later

//! Picture windows: a window of our own per picture, drawn off the editor's thread — a Wayland
//! surface on Linux, a winit window on macOS and Windows.
//!
//! **What this is for.** `proposals/deterministic-loop.md` left one cost open: a minimized
//! editor stopped the projector, because eframe runs one winit event loop and that loop
//! services every viewport in turn. winit permits exactly one `EventLoop` per process —
//! `EVENT_LOOP_CREATED` is a static, and a second is `EventLoopError::RecreationAttempt` — so
//! a second loop is not available on any thread. What is available is Wayland surfaces of our
//! own: an `xdg_toplevel` with no decorations and a wgpu surface on it, blitting on the one
//! device the synth draws on. Each window is then paced by its own frame callback, and what
//! the editor's window is doing is not a fact any of them can observe.
//!
//! **The connection is eframe's, borrowed.** The pictures thread runs an event queue of its
//! own on eframe's `wl_display`, through `Backend::from_foreign_display`. One display carrying
//! a queue per thread is libwayland's own design: a proxy created on our queue delivers to our
//! queue, and the read protocol keeps two threads off the socket. A Vulkan surface needs *a*
//! display, and the textures are the device's whatever connection a window is on, so a
//! connection of our own would draw as well; the borrow is kept because a window from a second
//! client connection is a client that does not hold focus, and the compositor's
//! focus-stealing prevention may open it behind the editor (`proposals/wgpu.md`, section 2).
//!
//! **Wayland only, on Linux.** On X11, or anywhere the display handle is not Wayland's, no
//! picture window opens and the mark that asked says so. There is no second path: an X11 one would be
//! a whole windowing backend — input, decorations, surfaces — for a case this instrument does
//! not have. The editor itself is unaffected; eframe keeps both backends.
//!
//! **Two halves.** This file is the portable one: which picture a window shows, the pure
//! geometry and keys a window reads, and the editor's list of windows ([`Wall`]), all tested
//! with no compositor. [`Host`], the thread from the editor's side, is `wayland.rs` on Linux,
//! with the thread itself in `thread.rs`; those two hold the module's `unsafe` and name its
//! Wayland crates. On macOS and Windows it is `winit/`, which answers the same names with no
//! `unsafe`: AppKit makes windows on the main thread alone, and Win32 delivers a window's
//! messages to the thread that made it, which must be the event loop's, so each is a winit
//! window made inside the event loop [`run`] starts around eframe, and drawn on a thread of its
//! own on the one device (`proposals/macos-windows.md`).
//!
//! `proposals/picture-windows.md` is the argument; [`docs/rendering.md`] carries the
//! threading contract.
//!
//! [`docs/rendering.md`]: ../../../docs/rendering.md

use crate::graph::NodeId;

#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod thread;
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::Host;

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod winit;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use self::winit::{Host, run};

/// Start eframe and run it until the app quits: `eframe::run_native`, with the same three
/// arguments and nothing else. macOS's and Windows' `run` builds the event loop itself, because
/// their picture windows are made inside it.
///
/// # Errors
/// What `eframe::run_native` returns.
#[cfg(target_os = "linux")]
pub fn run(
    app_name: &str,
    options: eframe::NativeOptions,
    creator: eframe::AppCreator<'_>,
) -> eframe::Result {
    eframe::run_native(app_name, options, creator)
}

/// Which picture a window shows. The window's **identity**: one window per picture, whatever
/// opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shown {
    /// A node's own render (`port: None`), or a CPU node's published frame on that port —
    /// the same pair a thumbnail names, because it is the same picture.
    Node {
        node: NodeId,
        port: Option<&'static str>,
    },
    /// The mix. What the projector was: the Main Mixer panel pops it out like any other
    /// picture, and there is no second kind of window for it.
    Mix,
}

impl Shown {
    /// Whether this is that node's picture on that port. What a node's own marks ask, to
    /// know whether to light.
    pub fn is(self, node: NodeId, port: Option<&'static str>) -> bool {
        self == Self::Node { node, port }
    }

    /// The node whose picture this is, where there is one. `None` for the mix, which belongs
    /// to no node.
    pub fn node(self) -> Option<NodeId> {
        match self {
            Self::Node { node, .. } => Some(node),
            Self::Mix => None,
        }
    }
}

/// How many **logical** points in from an edge of a picture window a press resizes rather
/// than moves.
///
/// A window with no decorations has no frame to take hold of, so the band *is* the frame.
/// Twelve points is what a hand finds without looking for it — a compositor's own borders
/// are thinner, and they have a visible frame to aim at.
pub const BAND: f64 = 12.0;

/// Which edge or corner of a window a position is on.
///
/// The window's own name for it, so the classification stays pure and testable;
/// `render::picture::thread` maps one onto `xdg_toplevel`'s `ResizeEdge` and onto the cursor
/// that says what a press there would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Which edge a pointer at `pos` is on, in a window `size` big, with bands `band` wide.
///
/// Both are in logical points, which is the frame a `wl_pointer` position arrives in and the
/// frame the compositor configures a window in. A corner is where two bands meet, and
/// anything further in than a band is `None` — a press there moves the window.
///
/// A `band` of zero is a window with **no bands at all**, which is what a fullscreen window
/// passes: there is nothing left to resize it to. The band is never more than half the
/// window either, so the two sides of a window narrower than two bands cannot overlap.
pub fn edge_at(pos: (f64, f64), size: (f64, f64), band: f64) -> Option<Edge> {
    if band <= 0.0 || pos.0 < 0.0 || pos.1 < 0.0 || pos.0 > size.0 || pos.1 > size.1 {
        return None;
    }
    let (bx, by) = (band.min(size.0 / 2.0), band.min(size.1 / 2.0));
    let (left, right) = (pos.0 < bx, pos.0 > size.0 - bx);
    let (top, bottom) = (pos.1 < by, pos.1 > size.1 - by);
    Some(match (top, bottom, left, right) {
        (true, _, true, _) => Edge::TopLeft,
        (true, _, _, true) => Edge::TopRight,
        (_, true, true, _) => Edge::BottomLeft,
        (_, true, _, true) => Edge::BottomRight,
        (true, ..) => Edge::Top,
        (_, true, ..) => Edge::Bottom,
        (_, _, true, _) => Edge::Left,
        (_, _, _, true) => Edge::Right,
        _ => return None,
    })
}

/// The largest size at `aspect` that fits inside `proposed`, anchored on the edge a hand is
/// dragging.
///
/// A compositor *proposes* a size through `xdg_toplevel.configure` and a client may answer
/// with one of its own, which is what makes an aspect lock possible at all. The anchor is the
/// gesture: dragging a **vertical** edge is a width, so the height follows it; dragging a
/// **horizontal** edge is a height, so the width follows; a **corner** is both, so the result
/// is the larger of the two that still fits inside what was proposed.
pub fn fit_aspect(proposed: (u32, u32), aspect: f64, edge: Edge) -> (u32, u32) {
    let (w, h) = (f64::from(proposed.0.max(1)), f64::from(proposed.1.max(1)));
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        w / h
    };
    let from_width = (w, w / aspect);
    let from_height = (h * aspect, h);
    let (w, h) = match edge {
        Edge::Left | Edge::Right => from_width,
        Edge::Top | Edge::Bottom => from_height,
        // A corner: whichever of the two fits inside the proposal, which is the smaller.
        _ if from_width.1 <= h => from_width,
        _ => from_height,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    ((w.round() as u32).max(1), (h.round() as u32).max(1))
}

/// Where a window is on screen, in logical points: its top-left corner and its size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a window a hand is dragging goes: the bounds it had when the press landed, after the
/// pointer has travelled `travel` points since.
///
/// Where no window system drags or resizes a borderless window for us — winit on macOS has no
/// resize at all, and Windows shares macOS's windows — this is the whole gesture, recomputed
/// on every motion from where it began, so nothing accumulates. `edge` is the band the press
/// landed in ([`edge_at`]): `None`, the middle, moves the window whole; an edge or a corner
/// follows the pointer while the opposite side stays where it was. `aspect`, where there is
/// one, is the lock `Shift` or `Ctrl` holds, by [`fit_aspect`]'s rule anchored on the same
/// edge. The size never goes below `min`, and a locked one grows back to it at its aspect
/// rather than giving the aspect up. Sizes are whole points.
pub fn dragged(
    start: Bounds,
    travel: (f64, f64),
    edge: Option<Edge>,
    aspect: Option<f64>,
    min: (f64, f64),
) -> Bounds {
    let Some(edge) = edge else {
        return Bounds {
            x: start.x + travel.0,
            y: start.y + travel.1,
            ..start
        };
    };
    let left = matches!(edge, Edge::Left | Edge::TopLeft | Edge::BottomLeft);
    let right = matches!(edge, Edge::Right | Edge::TopRight | Edge::BottomRight);
    let top = matches!(edge, Edge::Top | Edge::TopLeft | Edge::TopRight);
    let bottom = matches!(edge, Edge::Bottom | Edge::BottomLeft | Edge::BottomRight);
    let along = |grow: bool, shrink: bool, by: f64| {
        if grow {
            by
        } else if shrink {
            -by
        } else {
            0.0
        }
    };
    let mut width = (start.width + along(right, left, travel.0))
        .max(min.0)
        .round();
    let mut height = (start.height + along(bottom, top, travel.1))
        .max(min.1)
        .round();
    if let Some(aspect) = aspect {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (w, h) = fit_aspect((width as u32, height as u32), aspect, edge);
        (width, height) = (f64::from(w), f64::from(h));
        let grow = (min.0 / width).max(min.1 / height).max(1.0);
        (width, height) = ((width * grow).round(), (height * grow).round());
    }
    Bounds {
        x: if left {
            start.x + start.width - width
        } else {
            start.x
        },
        y: if top {
            start.y + start.height - height
        } else {
            start.y
        },
        width,
        height,
    }
}

/// What `Escape` does in a picture window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escaped {
    /// Leave fullscreen and stay: the window goes back to the pop-out it was.
    Windowed,
    /// The window goes.
    Closed,
}

/// `Escape` in a window that is `fullscreen`, and that was or was not **opened** fullscreen.
///
/// A window opened by the fullscreen mark has no pop-out to go back to — it was never one —
/// so one `Escape` closes it. A pop-out that was *made* fullscreen by `F` or a double-click
/// goes back to being that pop-out, and a second `Escape` closes it: `Escape` undoes what the
/// hand did, one step at a time, and the window is gone when there is nothing left to undo.
pub fn escaped(fullscreen: bool, opened_fullscreen: bool) -> Escaped {
    if fullscreen && !opened_fullscreen {
        Escaped::Windowed
    } else {
        Escaped::Closed
    }
}

/// A picture that has a window, and whether that window is fullscreen.
///
/// `App` owns these and `ui/` reads them, so a picture's marks say what its window is
/// already doing rather than being two buttons that look the same whatever happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Popped {
    pub picture: Shown,
    pub fullscreen: bool,
}

/// What one of a picture's two marks asked for: its window, or its window fullscreen.
///
/// Two marks rather than one, because they are two different things to want — a picture
/// beside the graph while the patch is built, and a picture filling a screen for the show —
/// and a single button could only guess which. The player's convention: the pop-out mark
/// first, fullscreen at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub picture: Shown,
    pub fullscreen: bool,
}

/// What the editor asks the pictures thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Open {
        picture: Shown,
        /// The picture's own size in pixels, which the window opens at — capped by the
        /// thread to the output it lands on.
        size: (u32, u32),
        title: String,
        /// Open already fullscreen. One path to fullscreen rather than one for opening and
        /// another for later.
        fullscreen: bool,
    },
    Close(Shown),
    Fullscreen(Shown, bool),
}

/// What the pictures thread reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    /// The window has gone — `Escape` inside it, or the compositor closed it.
    Closed(Shown),
    /// The window is now fullscreen, or is not. What it is *doing*, which is what the node's
    /// mark shows.
    Fullscreen(Shown, bool),
    /// The window could not be opened, with a sentence for the toast.
    Failed(Shown, String),
    /// Every display connected now, each its physical size in pixels as it is turned: sent
    /// once the window system has said what there is, and again whenever that changes. What
    /// the mix's default resolution is read from ([`crate::mixer::smallest_display`]).
    Displays(Vec<(u32, u32)>),
    /// **The thread itself is gone**, and no window will open again this run. Sent once, on
    /// the way out or instead of ever starting, so the editor latches the reason rather than
    /// discovering it one silently-dropped ask at a time.
    Gone(String),
}

/// The editor's side of it: which pictures should have windows, and what the thread says
/// they are doing.
///
/// Pure, and tested as such — the whole open/close/fullscreen state machine runs with no
/// compositor and no GPU. The rule it holds is the pop-out mark's: **a toggle**, as *Open
/// projector* was. The same click that opened the window puts it away, which matters most
/// when the window is on a screen the hand cannot see.
#[derive(Debug, Default)]
pub struct Wall {
    open: Vec<Popped>,
}

impl Wall {
    /// Act on one of a picture's marks, and say what to tell the thread.
    ///
    /// The fullscreen mark opens the window already fullscreen, or flips a window that is
    /// already open. Not a command: a window showing a picture is where the picture is being
    /// looked at, not an edit to the graph.
    pub fn mark(&mut self, req: Request, size: (u32, u32), title: String) -> Option<Ask> {
        match self.open.iter_mut().find(|p| p.picture == req.picture) {
            Some(open) if req.fullscreen => {
                // The thread answers with what the window is actually doing; this is only
                // the ask.
                Some(Ask::Fullscreen(req.picture, !open.fullscreen))
            }
            // The pop-out mark on a picture that has a window puts the window away. Dropped
            // here rather than on the answer, so the mark goes dark on the click.
            Some(_) => {
                self.open.retain(|p| p.picture != req.picture);
                Some(Ask::Close(req.picture))
            }
            None => {
                self.open.push(Popped {
                    picture: req.picture,
                    fullscreen: false,
                });
                Some(Ask::Open {
                    picture: req.picture,
                    size,
                    title,
                    fullscreen: req.fullscreen,
                })
            }
        }
    }

    /// A picture whose node has gone takes its window with it: there is no picture left to
    /// show, and a black window nobody asked to keep is litter.
    pub fn forget(&mut self, picture: Shown) -> Option<Ask> {
        let had = self.open.iter().any(|p| p.picture == picture);
        self.open.retain(|p| p.picture != picture);
        had.then_some(Ask::Close(picture))
    }

    /// Another project replaced this one: every window on a node goes, and the mix's stays.
    pub fn forget_nodes(&mut self) -> Vec<Ask> {
        let (nodes, kept) = self.open.iter().partition(|p| p.picture.node().is_some());
        self.open = kept;
        nodes
            .into_iter()
            .map(|p: Popped| Ask::Close(p.picture))
            .collect()
    }

    /// What the thread reported. Returns a line for the toast where there is one to say.
    pub fn told(&mut self, told: Told) -> Option<String> {
        match told {
            Told::Closed(picture) => {
                self.open.retain(|p| p.picture != picture);
                None
            }
            Told::Fullscreen(picture, to) => {
                if let Some(p) = self.open.iter_mut().find(|p| p.picture == picture) {
                    p.fullscreen = to;
                }
                None
            }
            Told::Failed(picture, why) => {
                self.open.retain(|p| p.picture != picture);
                Some(why)
            }
            // About the screens, not about any window.
            Told::Displays(_) => None,
            // Every window is gone with the thread that held it.
            Told::Gone(why) => {
                let had = !self.open.is_empty();
                self.open.clear();
                had.then_some(why)
            }
        }
    }

    /// Every picture that has a window. What the marks read.
    pub fn open(&self) -> &[Popped] {
        &self.open
    }

    /// Whether this picture has a window.
    pub fn has(&self, picture: Shown) -> bool {
        self.open.iter().any(|p| p.picture == picture)
    }
}
