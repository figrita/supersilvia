// SPDX-License-Identifier: AGPL-3.0-or-later

//! NDI®: live video between machines on the local network, through GStreamer's NDI plugin
//! compiled into the binary.
//!
//! **The plugin is ours, the runtime is the user's.** `gst-plugin-ndi` is a dependency, and
//! [`register`] hands it to GStreamer as a static plugin, which replaces any `ndi` plugin the
//! system's GStreamer carries: every NDI element the process makes is the one built with it.
//! The plugin opens the proprietary NDI runtime, `libndi`, only when an element starts, so
//! nothing proprietary is linked and building needs no NDI SDK; the user installs the runtime
//! from ndi.video. **The app finds the runtime before the plugin does**: every path that starts
//! an NDI element first has `platform::ndi` open it — where the plugin would, then from the
//! system's library folders by its path — and hold it, so a runtime in a folder the loader
//! does not search still loads (`/usr/local/lib`, on Fedora). Where it is missing, [`missing`]
//! says so, naming where it looked, and where it is there and still will not load, its path and
//! why ([`absent`] tells the two apart); found once off every thread that draws and never asked
//! again: the plugin remembers its first answer for the life of the process, so a runtime
//! installed while the app runs is found on its next run. **The probe puts nothing on the
//! network**: it takes an `ndisrc` to Ready, where the plugin loads the runtime and does no more,
//! since a receiver connects only on the way to Paused. An `ndisink` there would announce a
//! sender to the local network at every launch, and supersilvia uses the network only for NDI
//! that someone is using (`docs/decisions.md`, *No internet, and the network only for NDI*).
//! `proposals/ndi.md` is the argument, and the licence's additional permission for the runtime
//! is at the head of `LICENSE`.
//!
//! **Sending is bytes in, a stream out.** A [`Sender`] is one `appsrc ! ndisink` pipeline under
//! one name, handed a picture's rows as BGRA — with its alpha — or BGRx — opaque — which
//! `ndisink` hands the runtime as they are. The GPU half, which draws the picture and reads it
//! back, is `render::ndi`'s; nothing here waits for it or for the network: `appsrc` keeps the
//! two newest frames and drops the oldest past that, and `ndisink` encodes on the pipeline's own
//! thread. **A frame is stamped with the pipeline's clock as it is pushed** (`do-timestamp`), so
//! its NDI timecode is the time it was read back, and the caps declare the rate the synth ticks
//! at, which a receiver reads as the stream's rate.
//!
//! **Receiving is a camera.** The network's sources are listed by the plugin's device
//! provider, started by name the first time a menu asks and read at most twice a second
//! ([`labels`], [`menu`]), each by the name NDI gives it, `MACHINE (Stream)`, which is what a
//! project saves. A [`Receiver`] opens `ndisrc ndi-name=… ! ndisrcdemux` as a [`Camera`]
//! (`Source::Ndi`): the demuxer's video into the camera's appsink, UYVY for an opaque source and
//! BGRA for one with alpha, both uploaded as they are. A source that goes away keeps its last
//! frame up, says so, and is opened again when it is listed again; one read opaque has its BGRA
//! read as BGRx. The demuxer's sound is the Main Input's to take, through [`audio_head`].
//!
//! NDI® is a registered trademark of Vizrt NDI AB.

use super::{Camera, Source};
use crate::nodes::{Choices, Frame, Layout, Pixels};
use crate::platform::ndi::Preload;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// What the app says wherever NDI is offered and the runtime is not there. On a Mac it names
/// the runtime's own download: NDI Tools carry private copies inside each app and install
/// nothing the plugin can open, so "install NDI Tools" is not enough.
#[cfg(target_os = "macos")]
pub const MISSING: &str =
    "The NDI® runtime is not installed — get the NDI 6 Runtime at ndi.link/NDIRedistV6Apple";
#[cfg(not(target_os = "macos"))]
pub const MISSING: &str = "The NDI® runtime is not installed — get it at ndi.video";

/// The page [`MISSING`] names, which an Output's **Get it** opens.
#[cfg(target_os = "macos")]
pub const RUNTIME_URL: &str = "https://ndi.link/NDIRedistV6Apple";
#[cfg(not(target_os = "macos"))]
pub const RUNTIME_URL: &str = "https://ndi.video";

/// The plugin's device provider, which lists the network's NDI sources.
const PROVIDER: &str = "ndideviceprovider";

/// Register the NDI plugin with GStreamer, once for the process. Every entry point here calls
/// it; the app calls it at start-up.
pub fn register() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            gst::init().map_err(|e| format!("gstreamer: {e}"))?;
            gstndi::plugin_register_static().map_err(|e| format!("the NDI plugin: {e}"))?;
            // Out of every device monitor's reach: a camera's listing asks for `Video/Source`,
            // which the NDI provider's `Source/Audio/Video/Network` also answers, and starting
            // it starts NDI's discovery on the network. A listing of NDI sources starts it by name.
            if let Some(provider) = gst::DeviceProviderFactory::find(PROVIDER) {
                provider.set_rank(gst::Rank::NONE);
            }
            Ok(())
        })
        .clone()
}

/// Where the runtime probe stands.
const UNKNOWN: u8 = 0;
const PRESENT: u8 = 1;
/// No runtime file anywhere it was looked for.
const ABSENT: u8 = 2;
/// A runtime file there that would not load.
const REFUSED: u8 = 3;

static RUNTIME: AtomicU8 = AtomicU8::new(UNKNOWN);

std::thread_local! {
    /// This thread reads the runtime as not installed, whatever the probe found.
    static PRETEND: Cell<bool> = const { Cell::new(false) };
}

/// **Test hook.** On the calling thread alone, NDI reads as not installed while `on`, whatever
/// this machine has: [`missing`] says [`MISSING`] and no more, [`runtime`] is false and
/// [`labels`] is empty. What `tests/ui.rs`' harness sets, whose editor and inline synth run on
/// the test's own thread, so an Output's NDI row is drawn the same on every machine.
#[doc(hidden)]
pub fn pretend_missing(on: bool) {
    PRETEND.with(|p| p.set(on));
}

/// Where the probe stands, as this thread is to read it.
fn state() -> u8 {
    if PRETEND.with(Cell::get) {
        ABSENT
    } else {
        RUNTIME.load(Ordering::Acquire)
    }
}

/// Register the plugin and find out, on a thread of its own, whether the NDI runtime loads.
/// Once for the process; the app calls it at start-up so the answer is in before anything asks.
pub fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        if let Err(e) = register() {
            log::warn!("ndi: {e}");
        }
        let spawned = std::thread::Builder::new()
            .name("ndi-probe".into())
            .spawn(|| RUNTIME.store(probe(), Ordering::Release));
        if let Err(e) = spawned {
            log::warn!("ndi: the runtime probe could not be started: {e}");
        }
    });
}

/// Whether the NDI runtime loads, found by taking an `ndisrc` to Ready, where the plugin loads
/// it, once [`preloaded`] has opened the runtime wherever it is. Blocks for as long as loading
/// the library takes, so it is [`start`]'s thread that asks.
fn probe() -> u8 {
    if let Err(e) = register() {
        log::warn!("ndi: {e}");
        return ABSENT;
    }
    let preload = preloaded();
    let Ok(src) = gst::ElementFactory::make("ndisrc").build() else {
        return ABSENT;
    };
    // In a pipeline of its own, for the bus the plugin posts its reason on.
    let pipeline = gst::Pipeline::new();
    if pipeline.add(&src).is_err() {
        return ABSENT;
    }
    let loads = pipeline.set_state(gst::State::Ready).is_ok();
    let error = pipeline
        .bus()
        .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]))
        .and_then(|msg| match msg.view() {
            gst::MessageView::Error(e) => Some(e.error().to_string()),
            _ => None,
        });
    let _ = pipeline.set_state(gst::State::Null);
    if loads {
        return PRESENT;
    }
    let (found, why) = why_missing(preload, error.as_deref());
    log::info!("ndi: {why}");
    let _ = WHY.set(why);
    if found { REFUSED } else { ABSENT }
}

/// The runtime's file names, in the order the plugin opens them.
#[cfg(target_os = "macos")]
const LIBRARY_NAMES: &[&str] = &["libndi.dylib"];
#[cfg(target_os = "windows")]
const LIBRARY_NAMES: &[&str] = &["Processing.NDI.Lib.x64.dll"];
#[cfg(target_os = "linux")]
const LIBRARY_NAMES: &[&str] = &["libndi.so.6", "libndi.so.5"];

/// Where the runtime is looked for past the loader's own search: on a Mac, dyld's default
/// fallback, which is where NDI's installers put `libndi.dylib`; on Windows, the folder NDI 6's
/// runtime installs to, which its installer also names in `NDI_RUNTIME_DIR_V6`; on Linux, the
/// usual library folders, which `ld.so` need not search — Fedora's does not search
/// `/usr/local/lib`.
#[cfg(target_os = "macos")]
const SYSTEM_DIRS: &[&str] = &["/usr/local/lib", "/usr/lib"];
#[cfg(target_os = "windows")]
const SYSTEM_DIRS: &[&str] = &[r"C:\Program Files\NDI\NDI 6 Runtime\v6"];
#[cfg(target_os = "linux")]
const SYSTEM_DIRS: &[&str] = &[
    "/usr/lib",
    "/usr/local/lib",
    "/usr/lib64",
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
];

/// The variables the plugin reads a runtime folder from, before the loader's own search.
const RUNTIME_DIRS: [&str; 2] = ["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"];

/// Why the runtime did not load, as [`start`]'s probe found it.
static WHY: OnceLock<String> = OnceLock::new();

/// The folders the variables name, where set.
fn runtime_dirs() -> Vec<PathBuf> {
    RUNTIME_DIRS
        .iter()
        .filter_map(|var| std::env::var_os(var).map(PathBuf::from))
        .collect()
}

/// Every runtime file there is in `dirs`, by each of its names, in order.
fn files_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter()
        .flat_map(|dir| LIBRARY_NAMES.iter().map(move |name| dir.join(name)))
        .filter(|path| path.exists())
        .collect()
}

/// What to open, in order: the runtime in the folders the variables name, as the plugin opens
/// it first, then each bare name, which the system's loader searches for as the plugin's
/// next try does — then the runtime in the system's own folders, by its path, for a loader that
/// does not search where it is.
fn preload_order() -> Vec<PathBuf> {
    let system: Vec<PathBuf> = SYSTEM_DIRS.iter().map(PathBuf::from).collect();
    files_in(&runtime_dirs())
        .into_iter()
        .chain(LIBRARY_NAMES.iter().map(PathBuf::from))
        .chain(files_in(&system))
        .collect()
}

/// The runtime opened ahead of the plugin, once for the process, where this machine does that
/// (`platform::ndi`). Every path that starts an NDI element asks first, so the plugin's first
/// open, which it keeps for the process, comes after it.
fn preloaded() -> &'static Preload {
    static PRELOAD: OnceLock<Preload> = OnceLock::new();
    PRELOAD.get_or_init(|| {
        let preload = crate::platform::ndi::preload(&preload_order());
        match &preload {
            Preload::Held(path) => log::info!("ndi: the runtime is {}", path.display()),
            Preload::Left => {}
            Preload::Refused(tried) => {
                for (path, why) in tried {
                    log::debug!("ndi: {} would not open: {why}", path.display());
                }
            }
        }
        preload
    })
}

/// A path in a folder, rather than a bare name the loader searches for.
fn in_a_folder(path: &std::path::Path) -> bool {
    path.parent().is_some_and(|dir| !dir.as_os_str().is_empty())
}

/// What to say where the runtime did not load, and whether a runtime file was there at all:
/// a file that was there and still would not load, by its path and the loader's or the
/// plugin's reason; otherwise [`MISSING`]'s words and where it was looked for — the folders
/// the plugin's variables name, where set, then the system's.
fn why_missing(preload: &Preload, plugin: Option<&str>) -> (bool, String) {
    let dirs: Vec<PathBuf> = runtime_dirs()
        .into_iter()
        .chain(SYSTEM_DIRS.iter().map(PathBuf::from))
        .collect();
    let first_file = || files_in(&dirs).into_iter().next();
    let refused = match preload {
        Preload::Held(path) if in_a_folder(path) => Some((path.clone(), plugin)),
        Preload::Held(name) => Some((first_file().unwrap_or_else(|| name.clone()), plugin)),
        Preload::Refused(tried) => tried
            .iter()
            .find(|(path, _)| in_a_folder(path))
            .map(|(path, why)| (path.clone(), Some(why.as_str()))),
        Preload::Left => None,
    }
    .or_else(|| first_file().map(|path| (path, plugin)));
    match refused {
        Some((path, why)) => (
            true,
            format!(
                "The NDI® runtime at {} would not load{} — reinstall it from ndi.video",
                path.display(),
                why.map(|why| format!(": {why}")).unwrap_or_default()
            ),
        ),
        None => (
            false,
            format!(
                "{MISSING}. Looked for {} in {}",
                LIBRARY_NAMES.join(" or "),
                dirs.iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
    }
}

/// Why NDI cannot be used here: the whole of what to say, and whether a runtime file is there
/// that would not load, rather than none at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub why: &'static str,
    pub found: bool,
}

/// Why NDI cannot be used here, where the runtime is known not to load; `None` where it loads
/// or the probe has not answered yet. Never waits.
pub fn absent() -> Option<Missing> {
    if PRETEND.with(Cell::get) {
        return Some(Missing {
            why: MISSING,
            found: false,
        });
    }
    start();
    let found = match RUNTIME.load(Ordering::Acquire) {
        ABSENT => false,
        REFUSED => true,
        _ => return None,
    };
    Some(Missing {
        why: WHY.get().map_or(MISSING, String::as_str),
        found,
    })
}

/// Why NDI cannot be used here, where the runtime is known not to load — [`MISSING`]'s words and
/// where it was looked for, or the runtime found and why it would not load; `None` where it
/// loads or the probe has not answered yet. Never waits.
pub fn missing() -> Option<&'static str> {
    absent().map(|m| m.why)
}

/// Whether the runtime is known to load. Blocks until the probe has answered, so it is for
/// tests and for a thread that is about to start an NDI element anyway.
pub fn runtime() -> bool {
    if PRETEND.with(Cell::get) {
        return false;
    }
    start();
    loop {
        match RUNTIME.load(Ordering::Acquire) {
            UNKNOWN => std::thread::sleep(std::time::Duration::from_millis(5)),
            found => return found == PRESENT,
        }
    }
}

/// What a sender's frames are: their size, whether they carry alpha, and the rate the stream
/// declares, in frames a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    pub rate: u32,
}

impl Format {
    /// How many bytes one frame is, its rows packed.
    pub fn bytes(self) -> usize {
        (self.width * self.height * 4) as usize
    }

    fn caps(self) -> gst::Caps {
        gst::Caps::builder("video/x-raw")
            .field("format", if self.alpha { "BGRA" } else { "BGRx" })
            .field("width", self.width as i32)
            .field("height", self.height as i32)
            .field("framerate", gst::Fraction::new(self.rate.max(1) as i32, 1))
            .build()
    }
}

/// A string inside a quoted `gst-launch` property.
fn quoted(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One stream sent under one name: an `appsrc ! ndisink` pipeline of its own, announced on the
/// network from the moment it is made until it is dropped.
pub struct Sender {
    pipeline: gst::Pipeline,
    src: gst_app::AppSrc,
    /// What the caps say now, and the pool of buffers that size.
    format: Option<(Format, gst::BufferPool)>,
}

impl Sender {
    /// A sender announced as `name`, which NDI shows as `MACHINE (name)`. Refused with
    /// [`MISSING`]'s words where the runtime is missing.
    pub fn new(name: &str) -> Result<Self, String> {
        register()?;
        preloaded();
        if let Some(why) = missing() {
            return Err(why.to_string());
        }
        // `leaky-type=downstream`: past two queued frames the oldest goes, so a slow encoder
        // sends the newest. `sync=false`: sent as it arrives, stamped with the time it came.
        let description = format!(
            "appsrc name=src is-live=true format=time do-timestamp=true max-buffers=2 \
             leaky-type=downstream block=false \
             ! ndisink ndi-name=\"{}\" sync=false",
            quoted(name)
        );
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| format!("{description}: {e}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline".to_string())?;
        let src = pipeline
            .by_name("src")
            .ok_or("no appsrc")?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| "src is not an appsrc".to_string())?;
        if pipeline.set_state(gst::State::Playing).is_err() {
            let _ = pipeline.set_state(gst::State::Null);
            return Err(if runtime() {
                format!("{name}: the sender would not start")
            } else {
                missing().unwrap_or(MISSING).to_string()
            });
        }
        Ok(Self {
            pipeline,
            src,
            format: None,
        })
    }

    /// Send one frame as `format` says, `fill` writing its rows top first, packed, into the
    /// buffer it is handed. Never waits.
    pub fn send(&mut self, format: Format, fill: impl FnOnce(&mut [u8])) -> Result<(), String> {
        if self.format.as_ref().is_none_or(|(f, _)| *f != format) {
            if let Some((_, old)) = self.format.take() {
                let _ = old.set_active(false);
            }
            let caps = format.caps();
            let pool = gst::BufferPool::new();
            let mut config = pool.config();
            config.set_params(Some(&caps), format.bytes() as u32, 2, 0);
            pool.set_config(config)
                .map_err(|e| format!("the buffer pool: {e}"))?;
            pool.set_active(true)
                .map_err(|e| format!("the buffer pool: {e}"))?;
            self.src.set_caps(Some(&caps));
            self.format = Some((format, pool));
        }
        let (_, pool) = self.format.as_ref().expect("made above");
        let mut buffer = pool
            .acquire_buffer(None)
            .map_err(|e| format!("a buffer: {e:?}"))?;
        {
            let mut map = buffer
                .make_mut()
                .map_writable()
                .map_err(|_| "a buffer that would not map".to_string())?;
            fill(map.as_mut_slice());
        }
        self.src
            .push_buffer(buffer)
            .map(|_| ())
            .map_err(|e| format!("the sender: {e:?}"))
    }

    /// An error the pipeline reported since the last ask. Polled.
    pub fn error(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        let msg = bus.pop_filtered(&[gst::MessageType::Error])?;
        match msg.view() {
            gst::MessageView::Error(e) => Some(e.error().to_string()),
            _ => None,
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some((_, pool)) = self.format.take() {
            let _ = pool.set_active(false);
        }
    }
}

/// How old the listing may be before it is read again.
const LISTING: Duration = Duration::from_millis(500);

/// How often a receiver with no source looks for it, and one with a source asks it is there.
const RETRY: Duration = Duration::from_secs(1);

/// How often a receiver opens its source whether or not the listing has it, for a source the
/// provider cannot see and `ndisrc` can still reach.
const UNLISTED: Duration = Duration::from_secs(10);

/// What an NDI menu offers before anything is listed: nothing chosen.
pub const NONE: Choices = &[("", "None")];

/// The device provider, once started, and its last listing.
struct Listing {
    provider: Option<gst::DeviceProvider>,
    /// It would not start, which was said; it is not asked again.
    failed: bool,
    read: Option<(Instant, Vec<String>)>,
}

static LISTED: Mutex<Listing> = Mutex::new(Listing {
    provider: None,
    failed: false,
    read: None,
});

/// The NDI node's menu as the last listing made it.
static MENU: Mutex<Choices> = Mutex::new(NONE);

/// Every NDI source on the network by its name, `MACHINE (Stream)`, in the order the provider
/// found them, read again where the last reading is older than half a second. Empty where the
/// runtime is missing or has not been found yet. The first ask starts NDI's discovery, which
/// runs from then on. Never waits.
pub fn labels() -> Vec<String> {
    if state() != PRESENT {
        return Vec::new();
    }
    let mut listed = LISTED.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, names)) = &listed.read
        && at.elapsed() < LISTING
    {
        return names.clone();
    }
    if listed.provider.is_none() && !listed.failed {
        preloaded();
        let started = register().and_then(|()| {
            let provider = gst::DeviceProviderFactory::by_name(PROVIDER)
                .ok_or("the NDI device provider is not registered")?;
            provider
                .start()
                .map_err(|e| format!("the NDI device provider: {e}"))?;
            Ok(provider)
        });
        match started {
            Ok(provider) => listed.provider = Some(provider),
            Err(e) => {
                log::warn!("ndi: {e}");
                listed.failed = true;
            }
        }
    }
    let names: Vec<String> = listed.provider.as_ref().map_or_else(Vec::new, |p| {
        p.devices()
            .iter()
            .map(|d| d.display_name().to_string())
            .collect()
    });
    listed.read = Some((Instant::now(), names.clone()));
    names
}

/// The NDI node's menu: *None*, then every source listed now.
pub fn menu() -> Choices {
    let labels = labels();
    let mut menu = MENU.lock().unwrap_or_else(PoisonError::into_inner);
    let listed = &menu[1..];
    let same = listed.len() == labels.len()
        && listed
            .iter()
            .zip(&labels)
            .all(|(&(value, _), label)| value == label.as_str());
    if !same {
        // Leaked once a listing that differs, which is a source starting or stopping.
        let leak = |s: &str| &*Box::leak(s.to_owned().into_boxed_str());
        let choices: Vec<_> = NONE
            .iter()
            .copied()
            .chain(labels.iter().map(|l| {
                let l = leak(l);
                (l, l)
            }))
            .collect();
        *menu = Box::leak(choices.into_boxed_slice());
    }
    *menu
}

/// The head of a camera pipeline on the source NDI names `name`: its picture, off the
/// demuxer. `ndisrc` hands UYVY for a source with no alpha and BGRA for one with it, its
/// default, both in [`super::UPLOADABLE`].
pub(super) fn video_head(name: &str) -> String {
    preloaded();
    format!(
        "ndisrc ndi-name=\"{}\" ! ndisrcdemux name=ndidemux ndidemux.video",
        quoted(name)
    )
}

/// The head of an audio pipeline on the source NDI names `name`: its sound alone, off the
/// demuxer, from a receiver of its own that asks for no picture (`bandwidth=10`, NDI's
/// audio-only) and waits for the source for as long as it takes, since NDI's own receiver takes
/// a sender up again when it returns.
pub fn audio_head(name: &str) -> Result<String, String> {
    register()?;
    preloaded();
    if let Some(why) = missing() {
        return Err(why.to_string());
    }
    Ok(format!(
        "ndisrc ndi-name=\"{}\" bandwidth=10 timeout=0 connect-timeout=0 \
         ! ndisrcdemux name=ndidemux ndidemux.audio",
        quoted(name)
    ))
}

/// A frame read opaque: BGRA and RGBA read with their fourth byte as padding, which reads as
/// one. Every other frame as it is.
fn opaque(frame: Arc<Frame>) -> Arc<Frame> {
    let Pixels::Mapped(mapped) = &frame.pixels else {
        return frame;
    };
    let layout = match mapped.layout {
        Layout::Bgra => Layout::Bgrx,
        Layout::Rgba => Layout::Rgbx,
        _ => return frame,
    };
    let mut mapped = mapped.clone();
    mapped.layout = layout;
    Arc::new(Frame {
        width: frame.width,
        height: frame.height,
        pixels: Pixels::Mapped(mapped),
    })
}

/// A camera kept open on the source a name names, for the Main Input and the NDI node alike.
pub struct Receiver {
    source: String,
    transparent: bool,
    camera: Option<Camera>,
    /// When the camera was last asked whether its source is still there.
    asked: Instant,
    /// When to look for the source next, while there is no camera.
    next_try: Instant,
    /// When to open it next whether or not it is listed.
    next_unlisted: Instant,
    /// The last frame the camera handed over, and what was made of it.
    taken: Option<Arc<Frame>>,
    last: Option<Arc<Frame>>,
    /// A frame has arrived since the camera was opened.
    receiving: bool,
    error: Option<String>,
}

impl Receiver {
    /// A receiver for the source NDI names `source`, its alpha kept where `transparent`. It
    /// looks for it on the first [`Self::latest`].
    pub fn new(source: &str, transparent: bool) -> Self {
        let now = Instant::now();
        Self {
            source: source.to_string(),
            transparent,
            camera: None,
            asked: now,
            next_try: now,
            next_unlisted: now,
            taken: None,
            last: None,
            receiving: false,
            error: None,
        }
    }

    /// Whether this receives from `source` as `transparent` says, which a caller whose choice
    /// moved asks before making another.
    pub fn is(&self, source: &str, transparent: bool) -> bool {
        self.source == source && self.transparent == transparent
    }

    /// The newest frame, or the last one received once the source has gone, or `None` before
    /// any. Opens the source, and opens it again, as it comes and goes. Never waits.
    pub fn latest(&mut self) -> Option<Arc<Frame>> {
        if let Some(why) = missing() {
            self.camera = None;
            self.error = Some(why.to_string());
            return self.last.clone();
        }
        let now = Instant::now();
        let gone = match &self.camera {
            Some(camera) if now.duration_since(self.asked) >= RETRY => {
                self.asked = now;
                camera.error()
            }
            _ => None,
        };
        if let Some(why) = gone {
            log::info!("ndi: {}: {why}", self.source);
            self.camera = None;
            self.error = Some(if self.last.is_some() {
                format!("{} has gone: keeping its last frame", self.source)
            } else {
                format!("{} did not answer: {why}", self.source)
            });
            self.next_try = now + RETRY;
        }
        if self.camera.is_none() && !self.source.is_empty() && now >= self.next_try {
            self.next_try = now + RETRY;
            if labels().contains(&self.source) || now >= self.next_unlisted {
                self.next_unlisted = now + UNLISTED;
                self.open(now);
            } else if self.last.is_none() {
                self.error = Some(format!("waiting for {} on the network", self.source));
            }
        }
        if let Some(camera) = &mut self.camera
            && let Some(frame) = camera.latest()
            && !self.taken.as_ref().is_some_and(|t| Arc::ptr_eq(t, &frame))
        {
            self.taken = Some(Arc::clone(&frame));
            self.last = Some(if self.transparent {
                frame
            } else {
                opaque(frame)
            });
            if !self.receiving {
                self.receiving = true;
                self.error = None;
            }
        }
        self.last.clone()
    }

    fn open(&mut self, now: Instant) {
        match Camera::open(&Source::Ndi(self.source.clone()), None) {
            Ok(camera) => {
                self.camera = Some(camera);
                self.asked = now;
                self.receiving = false;
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Whether frames are arriving.
    pub fn receiving(&self) -> bool {
        self.camera.is_some() && self.receiving
    }

    /// Why nothing is being received, where nothing is.
    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// The name of the source it receives from.
    pub fn source(&self) -> &str {
        &self.source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runtime that was there and would not open is named by its path with the loader's
    /// reason, and said to be there, not missing; one opened that the plugin still would not
    /// start is named with the plugin's reason.
    #[test]
    fn a_runtime_that_is_there_and_will_not_load_says_where_and_why() {
        let tried = Preload::Refused(vec![
            (
                PathBuf::from("libndi.so.6"),
                "libndi.so.6: cannot open shared object file".to_string(),
            ),
            (
                PathBuf::from("/opt/ndi/libndi.so.6"),
                "libavahi-client.so.3: cannot open shared object file".to_string(),
            ),
        ]);
        let (found, why) = why_missing(&tried, None);
        assert!(found, "{why}");
        assert!(
            why.starts_with(
                "The NDI® runtime at /opt/ndi/libndi.so.6 would not load: \
                 libavahi-client.so.3: cannot open shared object file"
            ),
            "{why}"
        );

        let held = Preload::Held(PathBuf::from("/opt/ndi/libndi.so.6"));
        let (found, why) = why_missing(&held, Some("Failed to load function 'NDIlib_v5'"));
        assert!(found, "{why}");
        assert!(
            why.contains("/opt/ndi/libndi.so.6 would not load: Failed to load function"),
            "{why}"
        );
    }

    /// A thread that pretends reads the runtime as not installed, in [`MISSING`]'s words alone,
    /// whatever this machine has; another thread is not told.
    #[test]
    fn a_pretending_thread_reads_the_runtime_as_not_installed() {
        pretend_missing(true);
        assert_eq!(
            absent(),
            Some(Missing {
                why: MISSING,
                found: false
            })
        );
        assert_eq!(missing(), Some(MISSING));
        assert!(!runtime());
        assert!(labels().is_empty());
        let elsewhere = std::thread::spawn(|| PRETEND.with(Cell::get))
            .join()
            .expect("joined");
        assert!(!elsewhere, "this thread alone");
        pretend_missing(false);
        assert!(!PRETEND.with(Cell::get));
    }

    /// The plugin's elements and its device provider are the ones compiled in, whatever the
    /// system's GStreamer holds — a static plugin has no file, and no file is loaded — and the
    /// provider is ranked where no device monitor starts it.
    #[test]
    fn the_plugin_registered_is_the_one_compiled_in() {
        register().expect("registered");
        for name in ["ndisrc", "ndisrcdemux", "ndisink"] {
            let factory = gst::ElementFactory::find(name).expect(name);
            let plugin = factory.plugin().expect("a plugin");
            assert_eq!(plugin.filename(), None, "{name} is from {plugin:?}");
        }
        // A provider registered over a system's keeps the system's feature and takes our type,
        // so what is held is that making one loads no plugin file.
        let provider = gst::DeviceProviderFactory::find(PROVIDER).expect("the provider");
        drop(provider.get().expect("a provider"));
        let plugin = provider.plugin().expect("a plugin");
        assert!(
            plugin.filename().is_none() || !plugin.is_loaded(),
            "the provider loaded {plugin:?}"
        );
        assert_eq!(
            provider.rank(),
            gst::Rank::NONE,
            "out of a camera listing's reach"
        );
    }
}
