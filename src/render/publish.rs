// SPDX-License-Identifier: AGPL-3.0-or-later

//! The publisher: a thread of its own that hands pictures to other apps — over Syphon, to apps
//! on this Mac ([`super::syphon`]), and over NDI®, to machines on the network ([`super::ndi`]).
//!
//! **The publisher is a thread of its own, named `publisher`**, shaped like a macOS picture
//! window's ([`Publisher`]). It reads the newest `Published` the synth set in [`Live`], draws
//! every outlet whose picture is newer than the one it last drew and that can take one — a
//! Syphon server some client reads, an NDI sender with a staging buffer free — and finishes each
//! once its submission has: a Syphon server is told of the frame, an NDI sender handed its rows.
//! It never waits on the synth or on the GPU: the synth wakes it through a [`Live`] watcher
//! each time it publishes, and the GPU through a completion callback on the one queue, which
//! runs on whatever thread next polls — and while a frame of its own is on the GPU it polls
//! every few milliseconds itself, which never waits. Its blit pipeline is made on it, never on
//! the synth or the editor's frame thread, and both kinds draw through the one `Bgra8Unorm`
//! [`Viewer`].
//!
//! **A Syphon server goes only once its last frame has been drawn.** An outlet no longer wanted
//! is kept until the submission that last drew into its surface has finished, then dropped,
//! which stops its server; a stopped run lets go of every one the same way, waiting a second at
//! most. An NDI sender goes at once: what it has on the GPU is a copy into a buffer of its own,
//! and the frame is simply never sent.
//!
//! **What fails is said back.** An outlet that could not be made, or whose last frame failed,
//! is a [`Failure`] the thread writes into a list the editor reads once a frame, only when the
//! list changes; the row that asked for the outlet shows it.

use super::picture::Shown;
use super::{Gpu, Live, Viewer};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How a picture is written for another app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    /// Top row first, rather than Syphon's bottom row first. Syphon's alone: NDI has one
    /// orientation, top row first.
    pub flip: bool,
    /// The picture's own alpha, rather than opaque over black.
    pub transparent: bool,
}

/// Which way a picture leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// A Syphon server, for apps on this Mac.
    Syphon,
    /// An NDI sender, for machines on the network, declaring `rate` frames a second.
    Ndi { rate: u32 },
}

impl Via {
    /// Whether `self` and `other` are the same way out, whatever either declares.
    fn same_way(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Syphon, Self::Syphon) | (Self::Ndi { .. }, Self::Ndi { .. })
        )
    }
}

/// One picture the editor wants sent out: which, which way, under what name, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wanted {
    pub picture: Shown,
    pub via: Via,
    pub name: String,
    pub look: Look,
}

/// One wanted picture that is not going out, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub picture: Shown,
    pub via: Via,
    pub why: String,
}

/// Every failure the thread last said, which it writes and the editor reads.
type Failures = Arc<Mutex<Vec<Failure>>>;

/// What reaches the publisher's thread.
enum Msg {
    /// Everything that should be sent out now.
    Want(Vec<Wanted>),
    /// The synth set a new `Published`, or a submission of ours finished: look again.
    Look,
    Stop,
}

/// How long the thread waits for a frame's submission to be said finished before it polls the
/// device itself.
const GPU_POLL: Duration = Duration::from_millis(4);

/// How long a stopping publisher waits for its last frames to be drawn before it lets go of
/// the servers anyway.
const LAST_FRAMES: Duration = Duration::from_secs(1);

/// The publisher, from the editor's side: what it is asked to send out, and the thread that
/// does, started the first time anything is.
pub struct Publisher {
    /// The one device and the synth's `Live`, until the thread takes them. `None` with no GPU.
    start: Option<(Gpu, Arc<Live>)>,
    tx: Option<Sender<Msg>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// What was last asked for, so an unchanged ask is not sent again.
    sent: Vec<Wanted>,
    failures: Failures,
}

impl Publisher {
    /// A publisher drawing on `gpu` from `live`, or one that sends nothing with no GPU.
    pub fn new(gpu: Option<&Gpu>, live: Arc<Live>) -> Self {
        Self {
            start: gpu.map(|gpu| (gpu.clone(), live)),
            tx: None,
            thread: None,
            sent: Vec::new(),
            failures: Failures::default(),
        }
    }

    /// A publisher with nothing to draw with: every ask is kept and nothing is sent.
    pub fn detached() -> Self {
        Self {
            start: None,
            tx: None,
            thread: None,
            sent: Vec::new(),
            failures: Failures::default(),
        }
    }

    /// What should be sent out now, handed over only where it differs from the last ask.
    pub fn want(&mut self, wanted: Vec<Wanted>) {
        if wanted == self.sent {
            return;
        }
        if self.tx.is_none()
            && !wanted.is_empty()
            && let Some((gpu, live)) = self.start.take()
        {
            self.spawn(gpu, &live);
        }
        if let Some(tx) = &self.tx {
            // A closed channel is a thread that has stopped, which is a run that is ending.
            let _ = tx.send(Msg::Want(wanted.clone()));
        }
        self.sent = wanted;
    }

    /// What was last asked for.
    pub fn wanted(&self) -> &[Wanted] {
        &self.sent
    }

    /// Why each wanted picture that is not going out is not, as the thread last said.
    pub fn failures(&self) -> Vec<Failure> {
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn spawn(&mut self, gpu: Gpu, live: &Arc<Live>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = tx.clone();
        live.watch(move || {
            let _ = wake.send(Msg::Look);
        });
        let (theirs, theirs_live) = (tx.clone(), Arc::clone(live));
        let failures = Arc::clone(&self.failures);
        match std::thread::Builder::new()
            .name("publisher".into())
            .spawn(move || run(&gpu, &theirs_live, &rx, &theirs, &failures))
        {
            Ok(thread) => {
                self.tx = Some(tx);
                self.thread = Some(thread);
            }
            Err(e) => log::error!("publisher: the thread could not be started: {e}"),
        }
    }

    /// Stop sending and wait for the thread, which lets go of every Syphon server once its
    /// last frame is drawn and of every NDI sender at once.
    pub fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.failures
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Where one wanted picture goes.
enum Outlet {
    Syphon(super::syphon::Outlet),
    Ndi(super::ndi::Outlet),
}

impl Outlet {
    fn new(wanted: &Wanted) -> Result<Self, String> {
        Ok(match wanted.via {
            Via::Syphon => Self::Syphon(super::syphon::Outlet::new(&wanted.name)?),
            Via::Ndi { .. } => Self::Ndi(super::ndi::Outlet::new(&wanted.name)?),
        })
    }
}

/// One outlet the thread keeps, and where it stands.
struct Entry {
    wanted: Wanted,
    outlet: Outlet,
    /// The tick that drew the picture last drawn for it.
    shown: Option<u64>,
    /// A Syphon server's submission drawing into its surface, until it has finished and been
    /// published. An NDI sender keeps its own.
    pending: Option<u64>,
    /// Why the last draw failed, which was said; the next failure is said only if it differs.
    failing: Option<String>,
}

impl Entry {
    /// Whether a frame of its own is still on the GPU.
    fn busy(&self) -> bool {
        self.pending.is_some() || matches!(&self.outlet, Outlet::Ndi(o) if o.busy())
    }

    /// Say `e` once for as long as it keeps happening.
    fn failed(&mut self, e: &str) {
        if self.failing.as_deref() != Some(e) {
            log::warn!("publisher: {}: {e}", self.wanted.name);
            self.failing = Some(e.to_string());
        }
    }
}

/// A Syphon outlet no longer wanted, kept until the submission that last drew into it has
/// finished.
struct Retiring {
    _outlet: super::syphon::Outlet,
    serial: u64,
}

fn run(gpu: &Gpu, live: &Live, rx: &Receiver<Msg>, tx: &Sender<Msg>, failures: &Failures) {
    let viewer = match Viewer::new(gpu, wgpu::TextureFormat::Bgra8Unorm) {
        Ok(viewer) => viewer,
        Err(e) => {
            log::error!("publisher: the blit: {e}");
            return;
        }
    };
    let mut entries: Vec<Entry> = Vec::new();
    let mut retiring: Vec<Retiring> = Vec::new();
    // The outlets that could not be made, from the last ask, and what was last said.
    let mut refused: Vec<Failure> = Vec::new();
    let mut said: Vec<Failure> = Vec::new();
    'run: loop {
        // With a frame on the GPU, woken by its callback or, where nothing else polls the
        // device, by a poll of its own a moment later.
        let waiting = !retiring.is_empty() || entries.iter().any(Entry::busy);
        let first = if waiting {
            match rx.recv_timeout(GPU_POLL) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            }
        };
        for msg in first.into_iter().chain(rx.try_iter()) {
            match msg {
                Msg::Want(wanted) => refused = want(&mut entries, &mut retiring, wanted),
                Msg::Look => {}
                Msg::Stop => break 'run,
            }
        }
        gpu.poll();
        let completed = gpu.completed();
        for entry in &mut entries {
            match &mut entry.outlet {
                Outlet::Syphon(outlet) => {
                    if entry.pending.is_some_and(|serial| serial <= completed) {
                        outlet.publish();
                        entry.pending = None;
                    }
                }
                Outlet::Ndi(outlet) => {
                    if let Err(e) = outlet.collect() {
                        entry.failed(&e);
                    }
                }
            }
        }
        retiring.retain(|r| r.serial > completed);
        if draw(gpu, &viewer, live, &mut entries) {
            let wake = tx.clone();
            gpu.queue().on_submitted_work_done(move || {
                let _ = wake.send(Msg::Look);
            });
        }
        let now: Vec<Failure> = refused
            .iter()
            .cloned()
            .chain(entries.iter().filter_map(|e| {
                e.failing.as_ref().map(|why| Failure {
                    picture: e.wanted.picture,
                    via: e.wanted.via,
                    why: why.clone(),
                })
            }))
            .collect();
        if now != said {
            failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone_from(&now);
            said = now;
        }
    }
    // The last frames drawn, then every server let go.
    retiring.extend(retire(entries));
    let since = Instant::now();
    while !retiring.is_empty() && since.elapsed() < LAST_FRAMES {
        gpu.poll();
        let completed = gpu.completed();
        retiring.retain(|r| r.serial > completed);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The Syphon outlets among `entries`, each kept until its last submission; the NDI ones go.
fn retire(entries: impl IntoIterator<Item = Entry>) -> Vec<Retiring> {
    entries
        .into_iter()
        .filter_map(|e| match e.outlet {
            Outlet::Syphon(outlet) => Some(Retiring {
                serial: e.pending.unwrap_or(0),
                _outlet: outlet,
            }),
            Outlet::Ndi(_) => None,
        })
        .collect()
}

/// Make the outlets match `wanted`: new ones started, changed ones renamed or redrawn, and the
/// rest retired. Returns the ones that could not be made, and why.
fn want(
    entries: &mut Vec<Entry>,
    retiring: &mut Vec<Retiring>,
    wanted: Vec<Wanted>,
) -> Vec<Failure> {
    let mut kept = Vec::with_capacity(wanted.len());
    let mut refused = Vec::new();
    for w in wanted {
        let at = entries
            .iter()
            .position(|e| e.wanted.picture == w.picture && e.wanted.via.same_way(w.via));
        match at {
            Some(at) => {
                let mut entry = entries.swap_remove(at);
                if entry.wanted.name != w.name {
                    match &entry.outlet {
                        Outlet::Syphon(outlet) => outlet.server().rename(&w.name),
                        // A sender's name is fixed when it is announced: a new one is
                        // announced and the old one dropped, which takes its stream off the
                        // network. The new one is drawn the picture it has, not the next.
                        Outlet::Ndi(_) => match super::ndi::Outlet::new(&w.name) {
                            Ok(outlet) => {
                                entry.outlet = Outlet::Ndi(outlet);
                                entry.shown = None;
                                entry.failing = None;
                            }
                            Err(e) => {
                                log::warn!("publisher: {}: {e}", w.name);
                                refused.push(Failure {
                                    picture: w.picture,
                                    via: w.via,
                                    why: e,
                                });
                                continue;
                            }
                        },
                    }
                }
                if entry.wanted.look != w.look {
                    entry.shown = None;
                }
                entry.wanted = w;
                kept.push(entry);
            }
            None => match Outlet::new(&w) {
                Ok(outlet) => kept.push(Entry {
                    wanted: w,
                    outlet,
                    shown: None,
                    pending: None,
                    failing: None,
                }),
                // The runtime's absence is said where NDI is offered, not once per ask here.
                Err(_) if crate::video::ndi::missing().is_some() => {}
                Err(e) => {
                    log::warn!("publisher: {}: {e}", w.name);
                    refused.push(Failure {
                        picture: w.picture,
                        via: w.via,
                        why: e,
                    });
                }
            },
        }
    }
    retiring.extend(retire(std::mem::take(entries)));
    *entries = kept;
    refused
}

/// Draw every outlet whose picture is newer than the one last drawn for it and that can take
/// one: a Syphon server with nothing being drawn into it that some client reads, an NDI sender
/// with a staging buffer free. Whether anything was submitted.
fn draw(gpu: &Gpu, viewer: &Viewer, live: &Live, entries: &mut [Entry]) -> bool {
    let published = live.get();
    let mut submitted = false;
    for entry in entries.iter_mut() {
        let picture = match entry.wanted.picture {
            Shown::Node { node, port } => published.picture(node, port),
            Shown::Mix => published.mixer.clone(),
        };
        let Some(picture) = picture else { continue };
        let tick = picture.drawn_tick.unwrap_or(published.tick);
        if entry.shown == Some(tick) {
            continue;
        }
        let look = entry.wanted.look;
        let drawn = match (&mut entry.outlet, entry.wanted.via) {
            (Outlet::Syphon(outlet), _) => {
                if entry.pending.is_some() || !outlet.server().has_clients() {
                    continue;
                }
                outlet.draw(gpu, viewer, &picture, look).map(|ticket| {
                    entry.pending = Some(ticket.serial);
                })
            }
            (Outlet::Ndi(outlet), via) => {
                if !outlet.can_draw() {
                    continue;
                }
                let rate = match via {
                    Via::Ndi { rate } => rate,
                    Via::Syphon => 0,
                };
                outlet
                    .draw(gpu, viewer, &picture, look, rate, tick)
                    .map(|_| ())
            }
        };
        // Tried again on the next picture either way, and a failure said only the first time
        // in a row.
        entry.shown = Some(tick);
        match drawn {
            Ok(()) => {
                entry.failing = None;
                submitted = true;
            }
            Err(e) => entry.failed(&e),
        }
    }
    // Only now: every submission carrying a blit of what it names is made.
    drop(published);
    submitted
}
