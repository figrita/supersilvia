// SPDX-License-Identifier: AGPL-3.0-or-later

//! An Output's programs: the one it draws with, the one linking to replace it, and those it
//! kept, by the source each was linked from, over [`Program`].
//!
//! **Links are made on threads of ours, named `linker`.** wgpu has no asynchronous pipeline
//! creation on native: `create_shader_module` and `create_render_pipeline` return once the
//! driver has compiled to machine code, which is tens to hundreds of milliseconds the synth
//! thread must never spend. The device is `Send + Sync`, so [`Programs::set_shader`] sends the
//! module to the linkers and returns, and [`Programs::poll`] is a `try_recv` on the Output's
//! own channel. The program on screen draws until the new one lands, and a link that fails
//! leaves it drawing, its error for the status line: [`Program::create`]'s validation error
//! scope is thread-local, so on a linker it catches that link's errors and nothing the synth
//! is doing on the same device. A few linkers take from one queue, so a project opening links
//! its Outputs side by side, as GL's driver threads did, rather than one after another.
//!
//! **A newer source supersedes one in flight, and the superseded one is never shown.** Every
//! request is numbered, and the Output keeps the number it wants in an atomic the linkers
//! read: a linker drops a request no longer wanted rather than build it — a control drag sends
//! one a frame — and [`Programs::poll`] drops any result that is not the one pending.
//!
//! **A program owns what it draws with.** A job's uniform list and tap slots are compiled for
//! the source it names, which moves the moment an edit is compiled, while the program changes
//! only once the new one has landed. So each program keeps the list and slots of the last job
//! that named its own source ([`Programs::adopt`]) and draws with those.
//!
//! **Programs are kept by their source**, [`KEPT_PROGRAMS`] deep, so a source that comes back —
//! an undo, a cable put back — draws on the frame it arrives. A source already drawing or
//! linking is not linked again. A probe's programs are an `OutputRenderer`'s like any other,
//! so `compile::wgsl::build_probe`'s modules link the same way. See `proposals/wgpu.md`, 1.13.

use super::UniformValue;
use super::gpu::Gpu;
use super::program::Program;
use crate::compile::Shader;
use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

/// How many replaced programs an Output keeps aside.
pub const KEPT_PROGRAMS: usize = 4;

/// The most linker threads there are.
const MAX_LINKERS: usize = 4;

/// What one job compiled for a source draws with: its uniforms and its tap slots.
#[derive(Default)]
pub struct Setup {
    pub uniforms: Vec<(Arc<str>, UniformValue)>,
    pub taps: usize,
}

/// A program, and what the jobs naming its source left.
struct Kept {
    program: Program,
    setup: Option<Setup>,
}

/// A replacement sent to the linkers and not yet landed.
struct Pending {
    source: u64,
    /// The number it was requested under.
    request: u64,
    setup: Option<Setup>,
}

/// A module for a linker to make a pipeline of, for one Output's [`Programs`].
struct Request {
    gpu: Gpu,
    shader: Shader,
    format: wgpu::TextureFormat,
    number: u64,
    /// The number the Output wants; gone with the Output.
    wanted: Weak<AtomicU64>,
    reply: Sender<Linked>,
}

impl Request {
    /// Whether the Output is still there and still wants this one.
    fn wanted(&self) -> bool {
        self.wanted
            .upgrade()
            .is_some_and(|w| w.load(Ordering::Acquire) == self.number)
    }
}

/// A link made, or refused.
struct Linked {
    number: u64,
    program: Result<Program, String>,
}

/// The queue every linker takes from, and the linkers, made with the first request: half the
/// machine's threads, between one and [`MAX_LINKERS`].
fn linkers() -> &'static Sender<Request> {
    static QUEUE: OnceLock<Sender<Request>> = OnceLock::new();
    QUEUE.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Request>();
        let receive = Arc::new(Mutex::new(receive));
        let count = std::thread::available_parallelism()
            .map_or(1, |n| n.get() / 2)
            .clamp(1, MAX_LINKERS);
        for _ in 0..count {
            let receive = Arc::clone(&receive);
            let spawned = std::thread::Builder::new()
                .name("linker".into())
                .spawn(move || link(&receive));
            if let Err(err) = spawned {
                log::error!("could not start a linker thread: {err}");
            }
        }
        send
    })
}

/// One linker: take requests one at a time, and make each that is still wanted when taken.
fn link(queue: &Mutex<Receiver<Request>>) {
    loop {
        let next = queue.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(request) = next else {
            return;
        };
        if !request.wanted() {
            continue;
        }
        let made = panic::catch_unwind(AssertUnwindSafe(|| {
            Program::create(&request.gpu, &request.shader, request.format)
        }));
        let program = made.unwrap_or_else(|panicked| {
            let why = panicked
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panicked.downcast_ref::<&str>().copied())
                .unwrap_or("no message");
            Err(format!("the link panicked: {why}"))
        });
        // An Output gone since has dropped its end, and the result goes nowhere.
        let _ = request.reply.send(Linked {
            number: request.number,
            program,
        });
    }
}

pub struct Programs {
    format: wgpu::TextureFormat,
    current: Option<Kept>,
    pending: Option<Pending>,
    kept: VecDeque<Kept>,
    /// The number of the request wanted, read by the linkers: that of `pending`, or one no
    /// request carries.
    wanted: Arc<AtomicU64>,
    /// The number the last request was sent under.
    requests: u64,
    reply: Sender<Linked>,
    linked: Receiver<Linked>,
    /// A test's hold on `pending`: [`Programs::poll`] leaves it in flight.
    pub held: bool,
    /// The last link error, for the node's status line.
    pub error: Option<String>,
}

impl Programs {
    /// Programs drawing into targets of `format`.
    pub fn new(format: wgpu::TextureFormat) -> Self {
        let (reply, linked) = mpsc::channel();
        Self {
            format,
            current: None,
            pending: None,
            kept: VecDeque::new(),
            wanted: Arc::new(AtomicU64::new(0)),
            requests: 0,
            reply,
            linked,
            held: false,
            error: None,
        }
    }

    /// Replace the program with one linked from `shader`, on a linker; it lands on a
    /// [`Self::poll`] after the link has finished. On failure the one drawing is kept, so a
    /// typo does not blank the screen. A source already drawing or linking links nothing, and
    /// a kept one comes back on this call. Never waits.
    pub fn set_shader(&mut self, gpu: &Gpu, shader: &Shader) {
        let source = crate::compile::source_hash(&shader.body);
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.program.source == source)
        {
            // Back to what is on screen: whatever was linking to replace it is not wanted,
            // and neither is a failure it left.
            self.drop_pending();
            self.error = None;
            return;
        }
        if self.pending.as_ref().is_some_and(|p| p.source == source) {
            return;
        }
        if let Some(at) = self.kept.iter().position(|k| k.program.source == source) {
            let kept = self.kept.remove(at).expect("found above");
            self.drop_pending();
            self.keep_current();
            self.current = Some(kept);
            self.error = None;
            return;
        }
        self.requests += 1;
        let number = self.requests;
        self.wanted.store(number, Ordering::Release);
        let request = Request {
            gpu: gpu.clone(),
            shader: shader.clone(),
            format: self.format,
            number,
            wanted: Arc::downgrade(&self.wanted),
            reply: self.reply.clone(),
        };
        if linkers().send(request).is_err() {
            self.drop_pending();
            self.error = Some("no linker thread is running".into());
            return;
        }
        self.pending = Some(Pending {
            source,
            request: number,
            setup: None,
        });
    }

    /// Land a program that has finished linking, keeping the one it replaces; or take its
    /// error. Results of requests superseded since are dropped. Called once a tick before
    /// drawing. Never waits.
    pub fn poll(&mut self) {
        if self.held {
            return;
        }
        while let Ok(linked) = self.linked.try_recv() {
            let Some(pending) = self.pending.take_if(|p| p.request == linked.number) else {
                continue;
            };
            match linked.program {
                Ok(program) => {
                    self.keep_current();
                    self.current = Some(Kept {
                        program,
                        setup: pending.setup,
                    });
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    /// Take a job's uniforms and tap slots for the program linked from the source it names,
    /// the one drawing or the one linking. Whether the program drawing has something to draw
    /// with: one that landed from a link no job has named since does not.
    pub fn adopt(
        &mut self,
        source: u64,
        uniforms: &[(Arc<str>, UniformValue)],
        taps: usize,
    ) -> bool {
        let setup = match (&mut self.current, &mut self.pending) {
            (Some(c), _) if c.program.source == source => Some(&mut c.setup),
            (_, Some(p)) if p.source == source => Some(&mut p.setup),
            _ => None,
        };
        if let Some(setup) = setup {
            let setup = setup.get_or_insert_with(Setup::default);
            setup.uniforms.clear();
            setup.uniforms.extend_from_slice(uniforms);
            setup.taps = taps;
        }
        self.current.as_ref().is_some_and(|c| c.setup.is_some())
    }

    /// The program drawing and what it draws with, where it has both.
    pub fn current(&self) -> Option<(&Program, &Setup)> {
        let c = self.current.as_ref()?;
        Some((&c.program, c.setup.as_ref()?))
    }

    /// `compile::source_hash` of the module the program drawing was linked from.
    pub fn source(&self) -> Option<u64> {
        self.current.as_ref().map(|c| c.program.source)
    }

    pub fn has_program(&self) -> bool {
        self.current.is_some()
    }

    /// True while a replacement has not landed.
    pub fn is_linking(&self) -> bool {
        self.pending.is_some()
    }

    /// Stop drawing with any program: the current one is kept aside and a link in flight is
    /// dropped, with the error it belonged to.
    pub fn clear(&mut self) {
        self.keep_current();
        self.drop_pending();
        self.error = None;
    }

    /// How many programs are kept aside. For a test.
    pub fn kept(&self) -> usize {
        self.kept.len()
    }

    /// Whether a program linked from `source` is kept aside. For a test.
    pub fn keeps(&self, source: u64) -> bool {
        self.kept.iter().any(|k| k.program.source == source)
    }

    /// Want no request in flight: a linker that has not started one drops it, and its result,
    /// where there is one, is dropped on arrival.
    fn drop_pending(&mut self) {
        self.pending = None;
        self.requests += 1;
        self.wanted.store(self.requests, Ordering::Release);
    }

    /// Put the program drawing aside, newest first, dropping the oldest beyond
    /// [`KEPT_PROGRAMS`].
    fn keep_current(&mut self) {
        if let Some(current) = self.current.take() {
            self.kept.push_front(current);
            self.kept.truncate(KEPT_PROGRAMS);
        }
    }
}
