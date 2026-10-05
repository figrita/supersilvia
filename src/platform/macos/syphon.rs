// SPDX-License-Identifier: AGPL-3.0-or-later

//! Syphon on macOS: the vendored `Syphon.framework`'s base classes, bound through `objc2`.
//!
//! **Only the base classes.** A server is a `SyphonServerBase`, whose shared `IOSurface` the
//! renderer draws into itself (`newSurfaceForWidth:height:options:`, then `publish`), and a
//! client is a `SyphonClientBase`, whose `newSurface` hands over the server's surface on each
//! new frame. That is the framework's documented subclassing recipe
//! (`Syphon.docc/ExtendingSyphon.md`), used without subclassing: neither class needs a method
//! of ours, and neither the framework's Metal server nor its shader library is touched.
//! `proposals/syphon.md` is the argument.
//!
//! **Discovery needs the main thread's run loop.** The framework's server directory is made
//! when the framework loads, on the main thread, and hears announcements as distributed
//! notifications delivered there. eframe's event loop runs it in the app; a test runs it with
//! [`pump`]. Frames do not: they travel over Mach ports the framework services on queues of its
//! own, and a client's frame handler runs on one of them.
//!
//! **A surface is 8-bit BGRA**, recreated by the server only when its size changes, and the
//! same memory is drawn into again for the next frame: there is no second buffer.
//!
//! **The `unsafe` here** is the framework's classes and methods, declared by hand since no
//! binding crate carries them, the three description keys it exports, the surface's `+1`
//! reference taken over from a `new` method, the lock a surface's bytes are read under, and
//! `Send` on the objects that cross to the thread that uses them. Each block says why it holds.

use crate::nodes::{Frame, IoSurface, Layout, Mapped, Pixels, Planes, Redrawn, Yuv};
use crate::platform::screen::Slot;
use crate::platform::syphon::{Described, Look, Stream};
use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, extern_class, extern_methods};
use objc2_core_foundation::{CFRetained, CFRunLoop, kCFRunLoopDefaultMode};
use objc2_foundation::{NSArray, NSDictionary, NSString};
use objc2_io_surface::{IOSurfaceLockOptions, IOSurfaceRef};
use std::ptr::NonNull;
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::Duration;

// SAFETY: the three keys `SyphonServerDirectory.h` declares as `NSString * const`, exported by
// the framework `build.rs` links; each is a constant string for the life of the process.
unsafe extern "C" {
    static SyphonServerDescriptionUUIDKey: &'static NSString;
    static SyphonServerDescriptionNameKey: &'static NSString;
    static SyphonServerDescriptionAppNameKey: &'static NSString;
}

// SAFETY: IOSurface's own lock and unlock, as `IOSurfaceRef.h` declares them; the binding's
// methods for them are behind a `libc` feature this crate does not turn on for two calls.
unsafe extern "C-unwind" {
    fn IOSurfaceLock(buffer: &IOSurfaceRef, options: IOSurfaceLockOptions, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(buffer: &IOSurfaceRef, options: IOSurfaceLockOptions, seed: *mut u32)
    -> i32;
}

/// A server description, as the framework hands one out and takes one back.
type Description = NSDictionary<NSString, AnyObject>;

extern_class!(
    /// `SyphonServerBase`: a named server, its announcements, and its one shared surface.
    #[unsafe(super(NSObject))]
    #[name = "SyphonServerBase"]
    struct SyphonServerBase;
);

impl SyphonServerBase {
    extern_methods!(
        /// The designated initializer: announced at once, or `nil` when it could not start.
        #[unsafe(method(initWithName:options:))]
        unsafe fn init_with_name(
            this: Allocated<Self>,
            name: Option<&NSString>,
            options: Option<&Description>,
        ) -> Option<Retained<Self>>;

        /// The shared surface at this size, made anew where the last one was another size; `+1`,
        /// released by the caller.
        #[unsafe(method(newSurfaceForWidth:height:options:))]
        #[unsafe(method_family = none)]
        unsafe fn new_surface(
            &self,
            width: usize,
            height: usize,
            options: Option<&Description>,
        ) -> *mut IOSurfaceRef;

        #[unsafe(method(publish))]
        unsafe fn publish(&self);

        #[unsafe(method(stop))]
        unsafe fn stop(&self);

        #[unsafe(method(hasClients))]
        unsafe fn has_clients(&self) -> bool;

        #[unsafe(method(setName:))]
        unsafe fn set_name(&self, name: &NSString);

        #[unsafe(method(serverDescription))]
        unsafe fn server_description(&self) -> Retained<Description>;
    );
}

extern_class!(
    /// `SyphonClientBase`: one server's frames, as its surface.
    #[unsafe(super(NSObject))]
    #[name = "SyphonClientBase"]
    struct SyphonClientBase;
);

impl SyphonClientBase {
    extern_methods!(
        /// The designated initializer; `handler` is called with the client on each new frame,
        /// on a queue of the framework's.
        #[unsafe(method(initWithServerDescription:options:newFrameHandler:))]
        unsafe fn init_with_description(
            this: Allocated<Self>,
            description: &Description,
            options: Option<&Description>,
            handler: Option<&block2::DynBlock<dyn Fn(NonNull<AnyObject>)>>,
        ) -> Option<Retained<Self>>;

        /// The server's current surface, `+1`, released by the caller; null before the server
        /// has published one.
        #[unsafe(method(newSurface))]
        #[unsafe(method_family = none)]
        unsafe fn new_surface(&self) -> *mut IOSurfaceRef;

        #[unsafe(method(isValid))]
        unsafe fn is_valid(&self) -> bool;

        #[unsafe(method(stop))]
        unsafe fn stop(&self);
    );
}

extern_class!(
    /// `SyphonServerDirectory`: every server on the Mac, as the notifications describe them.
    #[unsafe(super(NSObject))]
    #[name = "SyphonServerDirectory"]
    struct SyphonServerDirectory;
);

impl SyphonServerDirectory {
    extern_methods!(
        #[unsafe(method(sharedDirectory))]
        unsafe fn shared() -> Retained<Self>;

        #[unsafe(method(servers))]
        unsafe fn servers(&self) -> Retained<NSArray<Description>>;
    );
}

/// A string a description holds under `key`, or empty where it holds none.
fn text(description: &Description, key: &NSString) -> String {
    description
        .objectForKey(key)
        .and_then(|value| value.downcast::<NSString>().ok())
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// What a description says about its server.
fn described(description: &Description) -> Described {
    // SAFETY: the framework's exported constants, valid for the life of the process.
    let (id, name, app) = unsafe {
        (
            SyphonServerDescriptionUUIDKey,
            SyphonServerDescriptionNameKey,
            SyphonServerDescriptionAppNameKey,
        )
    };
    Described {
        id: text(description, id),
        app: text(description, app),
        name: text(description, name),
    }
}

/// The directory's descriptions, as it holds them now.
fn descriptions() -> Retained<NSArray<Description>> {
    // SAFETY: the shared directory is made when the framework loads and never released; its
    // `servers` is a copy taken under its own lock.
    unsafe { SyphonServerDirectory::shared().servers() }
}

/// Why there is no Syphon here: never, on a Mac.
pub fn unavailable() -> Option<&'static str> {
    None
}

/// Every server the directory knows of, in the order it heard of them.
pub fn servers() -> Vec<Described> {
    descriptions().iter().map(|d| described(&d)).collect()
}

/// Run the calling thread's run loop for `time`, which on the main thread is where the
/// directory hears servers announce themselves. What a test does in place of AppKit's loop.
pub fn pump(time: Duration) {
    // SAFETY: CoreFoundation's own default mode, a constant for the life of the process.
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, time.as_secs_f64(), false);
}

/// One server's shared surface, held: 8-bit BGRA, at the size it was asked for.
pub struct Surface(CFRetained<IOSurfaceRef>);

// SAFETY: an `IOSurface` is reference counted atomically and may be used and released on any
// thread; nothing here writes through it.
unsafe impl Send for Surface {}
// SAFETY: as above; every access through `&Surface` reads.
unsafe impl Sync for Surface {}

impl Surface {
    /// Take over a `+1` reference a `new` method returned, or `None` for a null one.
    fn adopt(raw: *mut IOSurfaceRef) -> Option<Self> {
        let raw = NonNull::new(raw)?;
        // SAFETY: a non-null surface the framework returned retained, whose one reference is
        // now this value's.
        Some(Self(unsafe { CFRetained::from_raw(raw) }))
    }

    /// The `IOSurfaceRef` as an integer, valid for as long as this value is alive: what
    /// [`crate::render::dmabuf`] makes a texture over.
    pub fn raw(&self) -> usize {
        CFRetained::as_ptr(&self.0).as_ptr() as usize
    }

    /// Its size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.0.width() as u32, self.0.height() as u32)
    }

    /// The system-wide ID a client finds it by.
    pub fn id(&self) -> u32 {
        self.0.id()
    }

    /// Bytes from one row to the next.
    pub fn bytes_per_row(&self) -> usize {
        self.0.bytes_per_row()
    }

    /// Its pixel format, as CoreVideo names them: `BGRA`, or zero where the server that made it
    /// left it unset, as frameworks built before October 2025 do.
    pub fn pixel_format(&self) -> u32 {
        self.0.pixel_format()
    }

    /// Its rows, top of memory first, each `width × 4` bytes of B, G, R, A, read under a
    /// read-only lock. For a test, and never on a thread that must not wait: the lock waits for
    /// the GPU's writes to land.
    pub fn read(&self) -> Result<Vec<u8>, String> {
        let surface = &*self.0;
        // SAFETY: locked read-only, with no seed asked back, and unlocked below with the same
        // option.
        let locked = unsafe {
            IOSurfaceLock(
                surface,
                IOSurfaceLockOptions::ReadOnly,
                std::ptr::null_mut(),
            )
        };
        if locked != 0 {
            return Err(format!("the surface could not be locked ({locked})"));
        }
        let (width, height) = (surface.width(), surface.height());
        let stride = surface.bytes_per_row();
        let base = surface.base_address().as_ptr().cast::<u8>().cast_const();
        let mut out = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            // SAFETY: the surface is locked, and each row is `width × 4` bytes inside its
            // `bytes_per_row`, `height` rows from its base address.
            let bytes = unsafe { std::slice::from_raw_parts(base.add(row * stride), width * 4) };
            out.extend_from_slice(bytes);
        }
        // SAFETY: locked above with the same option.
        unsafe {
            IOSurfaceUnlock(
                surface,
                IOSurfaceLockOptions::ReadOnly,
                std::ptr::null_mut(),
            )
        };
        Ok(out)
    }
}

/// A server this process publishes: announced from the moment it is made until it is dropped,
/// which stops it.
///
/// Used from one thread at a time: the one that draws into its surface.
pub struct Server {
    object: Retained<SyphonServerBase>,
}

// SAFETY: Objective-C reference counting is atomic, and the server's own state is behind its
// lock; a `Server` is used from whichever one thread holds it.
unsafe impl Send for Server {}

impl Server {
    /// A server named `name`, announced at once, whose application is this process's name.
    pub fn new(name: &str) -> Result<Self, String> {
        let name = NSString::from_str(name);
        // SAFETY: the designated initializer on a fresh allocation, with a name and no options.
        let object = unsafe {
            SyphonServerBase::init_with_name(SyphonServerBase::alloc(), Some(&name), None)
        }
        .ok_or("the Syphon server could not be started")?;
        Ok(Self { object })
    }

    /// The shared surface at `width` by `height`: the one clients read, remade where the last
    /// was another size. A new one reaches clients at the next [`Self::publish`], so the old
    /// one stays on their screens until then.
    pub fn surface(&self, width: u32, height: u32) -> Result<Surface, String> {
        // SAFETY: a running server, asked for a surface with no options; the result is `+1`.
        let raw = unsafe {
            self.object
                .new_surface(width as usize, height as usize, None)
        };
        Surface::adopt(raw).ok_or_else(|| format!("no {width}x{height} Syphon surface"))
    }

    /// Tell every client the surface holds a new frame. Call it once the GPU has finished
    /// drawing it.
    pub fn publish(&self) {
        // SAFETY: a running server with a surface; after `stop` it only drops the message.
        unsafe { self.object.publish() };
    }

    /// Whether any client is reading it.
    pub fn has_clients(&self) -> bool {
        // SAFETY: a plain property.
        unsafe { self.object.has_clients() }
    }

    /// Give it another name, announced to every other app.
    pub fn rename(&self, name: &str) {
        let name = NSString::from_str(name);
        // SAFETY: the name is copied by the server.
        unsafe { self.object.set_name(&name) };
    }

    /// What the directory describes it as.
    pub fn described(&self) -> Described {
        // SAFETY: a plain property, built fresh on each read.
        described(&*unsafe { self.object.server_description() })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // SAFETY: stopping retires the announcement and closes the ports; the surface is let go
        // with the object.
        unsafe { self.object.stop() };
    }
}

/// A client of one server: `frames` is called with the server's surface on each new frame, on
/// a thread of the client's own. Dropping it stops it.
///
/// **The framework's frame handler only wakes that thread.** A client's `stop` holds the
/// client's lock while it waits for its frame queue, and `newSurface` takes the same lock, so a
/// handler that asked for the surface itself would deadlock against a `stop` on another thread
/// — which is how the first version of this hung. The thread asks instead, with nothing of the
/// framework's held.
pub struct Client {
    object: Retained<SyphonClientBase>,
    wake: std::sync::mpsc::Sender<bool>,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// A client, for the thread that reads its surfaces.
struct Held(Retained<SyphonClientBase>);

// SAFETY: Objective-C reference counting is atomic, and the client's own state is behind its
// lock; `newSurface` and `stop` may be called from any thread.
unsafe impl Send for Held {}

// SAFETY: as `Held`; `stop` may be called from any thread.
unsafe impl Send for Client {}

impl Client {
    /// Connect to the server `server` names, as the directory describes it now.
    pub fn new(
        server: &Described,
        frames: impl Fn(Surface) + Send + 'static,
    ) -> Result<Self, String> {
        let description = descriptions()
            .iter()
            .find(|d| described(d).id == server.id)
            .ok_or_else(|| format!("no Syphon server {}", server.label()))?;
        // `true` is a frame; `false`, or the sender gone, is the end.
        let (wake, woken) = std::sync::mpsc::channel::<bool>();
        let from_handler = wake.clone();
        let handler = RcBlock::new(move |_client: NonNull<AnyObject>| {
            let _ = from_handler.send(true);
        });
        // SAFETY: a description the directory handed out, no options, and a handler the client
        // copies and keeps.
        let object = unsafe {
            SyphonClientBase::init_with_description(
                SyphonClientBase::alloc(),
                &description,
                None,
                Some(&handler),
            )
        }
        .ok_or_else(|| format!("could not connect to {}", server.label()))?;
        let held = Held(object.clone());
        let reader = std::thread::Builder::new()
            .name("syphon client".into())
            .spawn(move || {
                let held = held;
                while let Ok(true) = woken.recv() {
                    // Frames that arrived while the last was handed over are one frame now.
                    if woken.try_iter().any(|frame| !frame) {
                        return;
                    }
                    // SAFETY: a valid client; the result is `+1`, or null before any surface.
                    if let Some(surface) = Surface::adopt(unsafe { held.0.new_surface() }) {
                        frames(surface);
                    }
                }
            })
            .map_err(|e| format!("the Syphon client's thread: {e}"))?;
        Ok(Self {
            object,
            wake,
            reader: Some(reader),
        })
    }

    /// Whether the server is still there.
    pub fn is_valid(&self) -> bool {
        // SAFETY: a plain property.
        unsafe { self.object.is_valid() }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.wake.send(false);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        // SAFETY: stops the frames, waiting for a handler already running on the framework's
        // queue, which only sends on a channel.
        unsafe { self.object.stop() };
    }
}

/// A received surface's bytes, for a device that will not import it: locked read-only the
/// first time they are asked for, which a device that imports never does, and unlocked when
/// the frame goes.
struct Mapping {
    surface: Surface,
    /// The base address and the length, once locked; `(0, 0)` where the lock failed.
    locked: OnceLock<(usize, usize)>,
}

impl Planes for Mapping {
    fn plane(&self, i: usize) -> &[u8] {
        if i != 0 {
            return &[];
        }
        let (base, len) = *self.locked.get_or_init(|| {
            let surface = &*self.surface.0;
            // SAFETY: locked read-only, with no seed asked back; unlocked in `drop`.
            let locked = unsafe {
                IOSurfaceLock(
                    surface,
                    IOSurfaceLockOptions::ReadOnly,
                    std::ptr::null_mut(),
                )
            };
            if locked != 0 {
                return (0, 0);
            }
            let base = surface.base_address().as_ptr() as usize;
            (base, surface.bytes_per_row() * surface.height())
        });
        if base == 0 {
            return &[];
        }
        // SAFETY: the surface stays locked while `self` lives, and `len` bytes from `base` are
        // its rows as it described them: the rows times the bytes between them.
        unsafe { std::slice::from_raw_parts(base as *const u8, len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        if self.locked.get().is_some_and(|(base, _)| *base != 0) {
            // SAFETY: locked read-only in `plane`, once, and unlocked here with the same option.
            unsafe {
                IOSurfaceUnlock(
                    &self.surface.0,
                    IOSurfaceLockOptions::ReadOnly,
                    std::ptr::null_mut(),
                )
            };
        }
    }
}

/// A received surface as a frame: the surface itself, which the renderer copies into a texture
/// of its own since the server draws into it again, read as `look` says.
fn frame_of(surface: Surface, look: Look) -> Frame {
    let (width, height) = surface.size();
    let stride = surface.bytes_per_row() as u32;
    let raw = surface.raw();
    Frame {
        width,
        height,
        pixels: Pixels::IoSurface(IoSurface {
            surface: raw,
            mapped: Mapped {
                // Opaque is the padded layout, whose fourth byte the copy writes one.
                layout: if look.transparent {
                    Layout::Bgra
                } else {
                    Layout::Bgrx
                },
                strides: [stride, 0, 0],
                yuv: Yuv::default(),
                // Syphon documents no convention, and the Mac's servers draw premultiplied,
                // as Core Animation and Metal do, so its alpha is sampled as it lies.
                straight_alpha: false,
                data: Arc::new(Mapping {
                    surface,
                    locked: OnceLock::new(),
                }),
            },
            redrawn: Some(Redrawn {
                bottom_first: !look.flip,
            }),
        }),
    }
}

/// A client whose frames land in slots a [`crate::video::Camera`] reads: the newest frame, and
/// a line once the server has gone. Dropping it stops the client.
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
            description: format!("Syphon {}", server.label()),
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

    /// Whether the server is still there; once it is not, the stream's error says so.
    pub fn is_valid(&self) -> bool {
        let valid = self.client.is_valid();
        if !valid {
            let mut error = self.error.lock().unwrap_or_else(PoisonError::into_inner);
            if error.is_none() {
                *error = Some(format!("{} has stopped", self.description));
            }
        }
        valid
    }
}
