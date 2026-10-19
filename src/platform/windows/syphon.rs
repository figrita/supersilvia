// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spout on Windows: what Syphon is on a Mac, a picture handed between apps on one machine on
//! the GPU, spoken natively — no Spout library is linked, and every object is made as Spout
//! 2.007 makes it, so Spout's own apps (OBS's Spout2 plugin, Resolume, TouchDesigner, Spout's
//! demo sender and receiver) and supersilvia read each other.
//!
//! **A sender** is a name in Spout's list, a map of that name describing a Direct3D 11 texture
//! shared by legacy handle, and the access mutex and frame count its receivers share
//! ([`registry`]). The renderer draws a picture into a **bridge**, a Direct3D 12 texture on its
//! own device that the process's Direct3D 11 device also holds ([`d3d11`]); [`Server::publish`],
//! once the GPU has drawn it, copies the bridge into the Spout texture under the access mutex,
//! flushes and counts the frame, as every Spout sender does. **The next draw into the bridge
//! waits for that copy on the GPU**: the copy signals a fence the renderer's queue is told to
//! wait on before its next submission, so no thread waits; where the driver shares no fence,
//! `publish` waits for the copy itself, on the publisher's thread. A sender is listed with its
//! first published frame, since Spout's entry names a texture, and a new size is named in the
//! map only once a frame has been copied into it, so a receiver never opens an empty texture.
//! The texture is 8-bit BGRA (DXGI format 87) and holds the picture top row first, Direct3D's
//! way up, as Spout's own senders do.
//!
//! **Names are the machine's.** A sender is named `supersilvia - {name}` in ASCII — any other
//! character written `_` — and where another sender has the name, ` 2`, ` 3` follow it.
//! [`Server::rename`] lists the sender again under its new name, keeping its texture. There is
//! no app name beside it and no count of receivers: [`Server::has_clients`] is always true.
//!
//! **A receiver** is a thread of its own that looks at its sender every few milliseconds — there
//! is no run loop to be told on — opening its map, reading it and closing it each time, since a
//! receiver holding it would keep a sender that has gone alive for everyone. A new share handle
//! is a new texture, opened anew; a map that no longer opens is a sender that has gone. Each
//! frame the sender counted — or every look, for a sender that does not count — is copied into
//! one of a few bridges of the receiver's own, and the frame handed on is that bridge, sampled
//! by the renderer's Direct3D 12 import once the queue has waited for the copy's fence. A
//! bridge is copied into again only once every frame that carries it has been let go.
//! 8-bit BGRA and RGBA senders are received; any other format is refused, saying which.
//!
//! **The `unsafe` here** is the Win32 and COM calls, the shared memory copied in and out of, and
//! the `Send` and `Sync` of the handles and interfaces that cross threads. Each block says why
//! it holds.

mod d3d11;
mod receive;
mod registry;

use crate::nodes::{D3d12Plane, D3d12Texture, Frame, Layout, Pixels, Yuv};
use crate::platform::screen::Slot;
use crate::platform::syphon::{BOTTOM_FIRST, Described, Look, PREFIX, Stream};
use d3d11::{Bridge, Device, Done, Fence};
use registry::{Access, Entry, Info};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use windows::Win32::Graphics::Direct3D11::{ID3D11Query, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};

/// Why there is no Spout here: never, on Windows.
pub fn unavailable() -> Option<&'static str> {
    None
}

/// Every sender Spout lists whose map still opens, in the list's order.
pub fn servers() -> Vec<Described> {
    let names = registry::names().unwrap_or_else(|e| {
        log::debug!("spout: {e}");
        Vec::new()
    });
    names
        .into_iter()
        .filter(|name| !matches!(registry::info(name), Ok(None)))
        .map(|name| described(&name))
        .collect()
}

/// A sender by its name: the name is its ID, unique on the machine while it lives.
fn described(name: &[u8]) -> Described {
    let text = registry::text_of(name);
    Described {
        id: text.clone(),
        app: String::new(),
        name: text,
    }
}

/// Nothing announces anything: Spout's directory is shared memory, read when it is asked.
pub fn pump(time: Duration) {
    std::thread::sleep(time);
}

/// A texture in Direct3D 12 and Direct3D 11 both, and the Spout texture beside it where it is
/// a sender's.
struct Held {
    device: Arc<Device>,
    bridge: Bridge,
    /// A sender's Spout texture and its legacy handle; `None` for a receiver's bridge.
    spout: Option<(ID3D11Texture2D, u32)>,
}

// SAFETY: Direct3D 11 textures are free-threaded, and the context that copies them is used
// under its lock.
unsafe impl Send for Held {}
// SAFETY: as above.
unsafe impl Sync for Held {}

/// A bridge, held: a sender's, drawn into, or a frame a receiver copied.
pub struct Surface {
    held: Arc<Held>,
    /// The receiver's fence, kept for as long as a frame names it.
    fence: Option<Arc<Fence>>,
    /// The value the fence reaches once the copy into the bridge is done.
    value: u64,
}

impl Surface {
    /// The bridge's `ID3D12Resource` as an integer, valid for as long as this value is alive:
    /// what [`crate::render::dmabuf`] makes a texture over.
    pub fn raw(&self) -> usize {
        self.held.bridge.resource_raw()
    }

    /// Its size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.held.bridge.width, self.held.bridge.height)
    }

    /// The legacy handle a sender's map names its Spout texture by; zero for a received frame.
    pub fn id(&self) -> u32 {
        self.held.spout.as_ref().map_or(0, |(_, handle)| *handle)
    }

    /// Its `DXGI_FORMAT`: 87, B8G8R8A8, or 28, R8G8B8A8.
    pub fn pixel_format(&self) -> u32 {
        self.held.bridge.format.0 as u32
    }

    /// Its rows, top first, each `width × 4` bytes in the order its format names — what a
    /// sender's receivers read, or what a receiver copied. Waits for the GPU; for a test.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        let texture = match &self.held.spout {
            Some((spout, _)) => spout,
            None => &self.held.bridge.texture,
        };
        self.held.device.read(texture)
    }
}

/// How one sender's or receiver's copies are marked done and waited for: a fence shared with
/// the renderer's queue, or an event query where there is none.
enum Waiter {
    Fence { fence: Arc<Fence>, value: u64 },
    Query(ID3D11Query),
}

// SAFETY: a query is used only with the immediate context, under its lock.
unsafe impl Send for Waiter {}

impl Waiter {
    fn new(device: &Device) -> Result<Self, String> {
        match device.fence() {
            Some(fence) => Ok(Self::Fence {
                fence: Arc::new(fence),
                value: 0,
            }),
            None => device.query().map(Self::Query),
        }
    }

    /// The mark for the next copy.
    fn next(&mut self) -> Done<'_> {
        match self {
            Self::Fence { fence, value } => {
                *value += 1;
                Done::Fence(fence, *value)
            }
            Self::Query(query) => Done::Query(query),
        }
    }

    /// Wait on this thread for the last copy marked.
    fn wait(&self, device: &Device) -> Result<(), String> {
        match self {
            Self::Fence { fence, value } => fence.wait(*value),
            Self::Query(query) => device.wait_query(query),
        }
    }
}

/// A sender this process publishes: listed from its first published frame until it is
/// dropped.
///
/// Used from one thread at a time: the one that draws into its surface.
pub struct Server {
    sending: Mutex<Sending>,
}

struct Sending {
    /// What its names begin with: [`PREFIX`], or nothing for a name taken as given.
    prefix: &'static str,
    /// The name asked for, before a suffix makes it unique.
    wanted: String,
    /// Its place in the list, once a frame has been published.
    entry: Option<Entry>,
    access: Option<Access>,
    /// The texture its map names.
    shown: Option<Arc<Held>>,
    /// A texture of a new size, named at the next publish.
    next: Option<Arc<Held>>,
    waiter: Option<Waiter>,
    /// Whether the last publish failed, so a failure is logged once in a row.
    failing: bool,
}

impl Server {
    /// A sender named `supersilvia - {name}`, listed once its first frame is published.
    pub fn new(name: &str) -> Result<Self, String> {
        Ok(Self::named(PREFIX, name))
    }

    /// A sender named `name` itself, ASCII as Spout's names are, rather than after the app: for
    /// a peer that another Spout app finds by the name it was given.
    pub fn named_exactly(name: &str) -> Result<Self, String> {
        Ok(Self::named("", name))
    }

    fn named(prefix: &'static str, name: &str) -> Self {
        Self {
            sending: Mutex::new(Sending {
                prefix,
                wanted: registry::sanitized(&format!("{prefix}{name}")),
                entry: None,
                access: None,
                shown: None,
                next: None,
                waiter: None,
                failing: false,
            }),
        }
    }

    fn sending(&self) -> std::sync::MutexGuard<'_, Sending> {
        self.sending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The bridge at `width` by `height`, which the renderer draws into, with a Spout texture of
    /// that size beside it, named in the sender's map at the next [`Self::publish`]; until
    /// then receivers keep reading the last.
    pub fn surface(&self, width: u32, height: u32) -> Result<Surface, String> {
        if width == 0 || height == 0 {
            return Err(format!("a {width}×{height} Spout texture has no pixels"));
        }
        let device = d3d11::here()?;
        let bridge = device.bridge(width, height, DXGI_FORMAT_B8G8R8A8_UNORM)?;
        let spout = device.spout_texture(width, height)?;
        let held = Arc::new(Held {
            device,
            bridge,
            spout: Some(spout),
        });
        self.sending().next = Some(Arc::clone(&held));
        Ok(Surface {
            held,
            fence: None,
            value: 0,
        })
    }

    /// Copy the frame drawn into the bridge into the Spout texture and count it. Call it once
    /// the GPU has finished drawing.
    pub fn publish(&self) {
        let mut sending = self.sending();
        match sending.publish() {
            Ok(()) => sending.failing = false,
            Err(e) => {
                if !sending.failing {
                    log::warn!("spout: {}: {e}", sending.name());
                }
                sending.failing = true;
            }
        }
    }

    /// Always: Spout counts no receivers, so a sender is always drawn.
    pub fn has_clients(&self) -> bool {
        true
    }

    /// List it under another name, keeping its texture.
    pub fn rename(&self, name: &str) {
        let mut sending = self.sending();
        sending.wanted = registry::sanitized(&format!("{}{name}", sending.prefix));
        if sending.entry.take().is_some() {
            sending.access = None;
            if let Err(e) = sending.announce() {
                log::warn!("spout: {}: {e}", sending.wanted);
            }
        }
    }

    /// What the list describes it as: its name, unique once it is listed.
    pub fn described(&self) -> Described {
        let sending = self.sending();
        described(sending.name().as_bytes())
    }
}

impl Sending {
    /// The name it is listed under, or the one it will ask for.
    fn name(&self) -> String {
        self.entry
            .as_ref()
            .map_or_else(|| self.wanted.clone(), |e| registry::text_of(e.name()))
    }

    /// What its map says of the texture it names.
    fn info(&self) -> Option<Info> {
        let held = self.shown.as_ref()?;
        let (_, handle) = held.spout.as_ref()?;
        Some(Info {
            handle: *handle,
            width: held.bridge.width,
            height: held.bridge.height,
            format: DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
        })
    }

    /// List it, with the texture it shows, and count that texture's frame.
    fn announce(&mut self) -> Result<(), String> {
        let info = self.info().ok_or("no frame to list a sender with")?;
        let entry = registry::announce(self.wanted.as_bytes(), info)?;
        let access = Access::new(entry.name())?;
        if let Some(_locked) = access.lock() {
            access.sent();
        }
        log::info!(
            "spout: sending {} at {}×{}",
            registry::text_of(entry.name()),
            info.width,
            info.height
        );
        self.entry = Some(entry);
        self.access = Some(access);
        Ok(())
    }

    fn publish(&mut self) -> Result<(), String> {
        let fresh = self.next.is_some();
        let Some(held) = self.next.clone().or_else(|| self.shown.clone()) else {
            return Ok(());
        };
        let Some((spout, _)) = &held.spout else {
            return Ok(());
        };
        if self.waiter.is_none() {
            self.waiter = Some(Waiter::new(&held.device)?);
        }
        let waiter = self.waiter.as_mut().expect("made above");
        let done = waiter.next();
        {
            let locked = match &self.access {
                Some(access) => Some(
                    access
                        .lock()
                        .ok_or("a receiver held the texture past Spout's wait")?,
                ),
                None => None,
            };
            held.device.copy(spout, &held.bridge.texture, &done)?;
            if let Some(access) = &self.access {
                access.sent();
            }
            drop(locked);
        }
        let waiter = self.waiter.as_ref().expect("made above");
        match waiter {
            // The next draw into the bridge waits on the renderer's queue, and a texture about
            // to be named waits here, so no receiver opens it before the copy lands.
            Waiter::Fence { fence, value }
                if !fresh && crate::render::dmabuf::d3d12_wait_here(fence.raw(), *value) => {}
            _ => waiter.wait(&held.device)?,
        }
        if fresh {
            self.shown = self.next.take();
            match &self.entry {
                Some(entry) => entry.update(self.info().expect("shown above"))?,
                None => self.announce()?,
            }
        }
        Ok(())
    }
}

/// A receiver of one sender: `frames` is called with a copy of each new frame, on a thread of
/// the client's own. Dropping it stops it.
pub struct Client {
    state: Arc<receive::State>,
    reader: Option<std::thread::JoinHandle<()>>,
}

impl Client {
    /// Receive from the sender `server` names, as the list describes it now.
    pub fn new(
        server: &Described,
        frames: impl Fn(Surface) + Send + 'static,
    ) -> Result<Self, String> {
        let missing = || format!("no Spout sender {}", server.label());
        let name = registry::bytes_of(&server.id).ok_or_else(missing)?;
        if registry::info(&name)?.is_none() {
            return Err(missing());
        }
        let device = d3d11::here()?;
        let waiter = Waiter::new(&device)?;
        let state = Arc::new(receive::State::default());
        let receiving = receive::Receiving::new(name, device, waiter, Arc::clone(&state));
        let reader = std::thread::Builder::new()
            .name("spout client".into())
            .spawn(move || receiving.run(&frames))
            .map_err(|e| format!("the Spout receiver's thread: {e}"))?;
        Ok(Self {
            state,
            reader: Some(reader),
        })
    }

    /// Whether the sender is still there.
    pub fn is_valid(&self) -> bool {
        !self.state.gone.load(Ordering::Acquire)
    }

    /// Why its frames are refused, where they are: a format, a texture on another GPU.
    pub fn problem(&self) -> Option<String> {
        self.state
            .problem
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.state.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// A received frame as a renderer's frame: the bridge it was copied into, on the renderer's
/// own device, behind the fence its copy signals, read as `look` says.
fn frame_of(surface: Surface, look: Look) -> Frame {
    let (width, height) = surface.size();
    let rgba = surface.held.bridge.format == DXGI_FORMAT_R8G8B8A8_UNORM;
    let layout = match (rgba, look.transparent) {
        (false, true) => Layout::Bgra,
        (false, false) => Layout::Bgrx,
        (true, true) => Layout::Rgba,
        (true, false) => Layout::Rgbx,
    };
    let plane = D3d12Plane {
        resource: surface.raw(),
        shared: 0,
        slice: 0,
        fence: surface.fence.as_ref().map_or(0, |f| f.raw()),
        fence_value: surface.value,
    };
    Frame {
        width,
        height,
        pixels: Pixels::D3d12(D3d12Texture {
            device: surface.held.device.d3d12_raw(),
            textures: vec![plane],
            layout,
            yuv: Yuv::default(),
            // Spout's texture is top row first, the convention; `flip` reads it the other way up.
            bottom_first: BOTTOM_FIRST != look.flip,
            keep: Arc::new(surface),
            refused: None,
        }),
    }
}

/// A receiver whose frames land in slots a [`crate::video::Camera`] reads: the newest frame,
/// and a line once the sender has gone or its frames are refused. Dropping it stops it.
pub struct Inlet {
    client: Client,
    frame: Slot<Arc<Frame>>,
    error: Slot<String>,
    description: String,
}

impl Inlet {
    /// Receive from `server`, each frame read as `look` says.
    pub fn open(server: &Described, look: Look) -> Result<Self, String> {
        let frame: Slot<Arc<Frame>> = Arc::default();
        let into = Arc::clone(&frame);
        let client = Client::new(server, move |surface| {
            let received = Arc::new(frame_of(surface, look));
            let displaced = into
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .replace(received);
            drop(displaced);
        })?;
        Ok(Self {
            client,
            frame,
            error: Arc::default(),
            description: format!("Spout {}", server.label()),
        })
    }

    /// What a [`crate::video::Camera`] reads this inlet by, valid for as long as it is alive.
    pub fn stream(&self) -> Stream {
        Stream {
            frame: Arc::clone(&self.frame),
            error: Arc::clone(&self.error),
            description: self.description.clone(),
        }
    }

    /// Whether the sender is still there and its frames are received; once not, the stream's
    /// error says why.
    pub fn is_valid(&self) -> bool {
        let problem = self.client.problem();
        let valid = self.client.is_valid() && problem.is_none();
        if !valid {
            let mut error = self.error.lock().unwrap_or_else(PoisonError::into_inner);
            if error.is_none() {
                *error = Some(match problem {
                    Some(why) => format!("{}: {why}", self.description),
                    None => format!("{} has stopped", self.description),
                });
            }
        }
        valid
    }
}
