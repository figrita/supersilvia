// SPDX-License-Identifier: AGPL-3.0-or-later

//! The machine: every service the app asks of the operating system that is not the same on
//! each one.
//!
//! Each service below is a module whose names are the whole of what the rest of the app may
//! call, and each name is re-exported from `linux/`, `macos/` or `windows/` by the build's own
//! target — the three backends side by side, so a name one of them lacks is a build failure on
//! that target rather than a surprise at run time. What a service hands back is the app's own
//! plain data: a MIDI [`Message`](crate::midi::Message), a [`Frame`](crate::nodes::Frame), a
//! path. Nothing here owns a node, a graph or a window.
//!
//! **`linux/` is the machine this app is built and tested on**: the ALSA sequencer,
//! xdg-desktop-portal through `ashpd` and `rfd`, PipeWire, PulseAudio, V4L2, VA-API and
//! DMA-BUF, fontconfig, the XDG base and user directories and the kernel's DRM counters. **`macos/`
//! answers every service natively**, each through the macOS API that replaces the Linux one:
//! the folders and the file manager, which need nothing but the standard library, the file
//! dialogs, which are `NSOpenPanel` through `rfd`, the cameras and hardware codecs, which are
//! AVFoundation's and VideoToolbox's through GStreamer, the named audio inputs, which Core
//! Audio lists and `osxaudiosrc` captures, the loopback, a Core Audio process tap, screen
//! capture, which is ScreenCaptureKit's, MIDI, which is CoreMIDI through `midir`, the fonts,
//! which are AppKit's font collection, the menu bar, which is AppKit's, and Syphon, which
//! Linux does not have. `proposals/platform.md` is the plan that got there, and what the
//! renderer needs from here beside it. **`windows/` answers every service too**, the media
//! through GStreamer's official MSVC release: Media Foundation's cameras, WASAPI's inputs and
//! loopbacks, the primary monitor through Direct3D 11, each GPU vendor's hardware codecs, the
//! shell's dialogs, Known Folders and Explorer, WinMM through `midir` and DirectWrite's fonts,
//! with no menu bar, Syphon or zero-copy path, as on Linux, and no GPU counter read.
//!
//! The picture windows and the GPU device are not here. They are `render/`'s and
//! `render/`'s, with Wayland beneath the windows on Linux and winit's on the other two.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "supersilvia's platform layer has a Linux backend, a macOS one and a Windows one, and no other"
);

/// MIDI in: every source the machine offers, wired to one input, read on a thread that is not
/// the frame's — the backend's own on Linux, CoreMIDI's on macOS and WinMM's on Windows.
///
/// `Midi::open` returns the handle and the queue its reader posts to; `Midi::sources` lists
/// what is there and whether it is wired in; `Midi::connect_all` wires in whatever is not.
/// The queue is the synth's, which drains it inside the tick. See [`crate::midi`].
pub mod midi {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::midi::Midi;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::midi::Midi;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::midi::Midi;
}

/// The calling thread's own CPU time, the clock that stands still while the thread is blocked,
/// for the Status box's line between work and waiting on the GPU (`synth::meter`):
/// `CLOCK_THREAD_CPUTIME_ID` on Linux and macOS, `GetThreadTimes` on Windows.
pub mod clock {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::clock::thread_cpu;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::clock::thread_cpu;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::clock::thread_cpu;
}

/// Screen capture: the desktop's own picker, and the session that must outlive whatever reads
/// its answer.
///
/// `ask` returns at once with a `Pending` a tick polls; a `Cast` is the answer, held for as
/// long as anything reads it, and dropping it ends the capture. `Cast::stream` is the
/// `Stream` a [`crate::video::Source::Screen`] names, and `Stream::head` is where its frames
/// come from: a GStreamer source element that reads it on Linux, and on macOS the slots the
/// capture writes each frame and its end into itself. Windows has no picker: its `Pending`
/// answers on the first poll, with the primary monitor, read by a GStreamer element.
pub mod screen {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::screen::{Cast, Pending, Stream, ask};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::screen::{Cast, Pending, Stream, ask};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::screen::{Cast, Pending, Stream, ask};

    use crate::nodes::Frame;
    use std::sync::{Arc, Mutex};

    /// One value a capture writes and a reader takes: the newest frame, or the reason it
    /// stopped.
    pub type Slot<T> = Arc<Mutex<Option<T>>>;

    /// Where a stream's frames come from.
    #[derive(Debug, Clone)]
    pub enum Head {
        /// A GStreamer source element, written as a pipeline fragment, whose output a pipeline
        /// delivers.
        Element(String),
        /// Frames the capture delivers itself: the newest, which a reader takes, and the
        /// reason the capture ended, once it has, with a line saying what it captures.
        Slots {
            frame: Slot<Arc<Frame>>,
            error: Slot<String>,
            description: String,
        },
    }
}

/// Syphon: pictures handed between apps on one Mac, on the GPU.
///
/// A [`Server`] publishes one picture under a name: `Server::surface` is its shared surface at
/// a size, which the renderer draws into, and `Server::publish` tells its clients a frame is
/// there. `servers` is every server the machine's directory knows of, which on a Mac is kept
/// by the main thread's run loop and on a test's by `pump`. A [`Client`] is handed one server's
/// [`Surface`] on each of its frames. Linux and Windows have no Syphon: the list is empty and a
/// server or a client is refused, `unavailable` says why, and
/// [`available`](syphon::available) is false, so nothing that offers Syphon is drawn there. An [`Inlet`](syphon::Inlet) is a client
/// whose frames land in the slots a [`crate::video::Camera`] reads, through its
/// [`Stream`](syphon::Stream), as a screen's do. See `proposals/syphon.md`.
///
/// [`pretend_unavailable`](syphon::pretend_unavailable) is the UI tests' hook: a thread that
/// sets it is told what Linux tells every thread, so a Mac draws the Linux snapshots.
pub mod syphon {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::syphon::{Client, Inlet, Server, Surface, pump, servers};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::syphon::{Client, Inlet, Server, Surface, pump, servers};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::syphon::{Client, Inlet, Server, Surface, pump, servers};

    use crate::nodes::Frame;
    #[cfg(target_os = "linux")]
    use crate::platform::linux::syphon as here;
    #[cfg(target_os = "macos")]
    use crate::platform::macos::syphon as here;
    use crate::platform::screen::{Head, Slot};
    #[cfg(target_os = "windows")]
    use crate::platform::windows::syphon as here;
    use std::cell::Cell;
    use std::sync::Arc;

    /// What a machine without Syphon says: Linux's and Windows' answer, and a pretending
    /// thread's.
    pub(crate) const MACOS_ONLY: &str = "Syphon is macOS's";

    std::thread_local! {
        static PRETEND: Cell<bool> = const { Cell::new(false) };
    }

    /// Why there is no Syphon here, or `None` where there is.
    pub fn unavailable() -> Option<&'static str> {
        if PRETEND.with(Cell::get) {
            return Some(MACOS_ONLY);
        }
        here::unavailable()
    }

    /// Whether this machine has Syphon at all: everything that offers it — a menu entry, a row,
    /// a mark — is drawn only where this is true.
    pub fn available() -> bool {
        unavailable().is_none()
    }

    /// **Test hook.** On the calling thread alone, Syphon reads as absent while `on`, as it
    /// does on Linux: [`unavailable`] says Linux's words and [`available`] is false, so nothing
    /// that offers Syphon is drawn. What `tests/ui.rs`' harness sets, whose editor and inline
    /// synth run on the test's own thread.
    #[doc(hidden)]
    pub fn pretend_unavailable(on: bool) {
        PRETEND.with(|p| p.set(on));
    }

    /// One server, as the directory describes it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Described {
        /// Unique to this server's run, and never shown.
        pub id: String,
        /// The application publishing it, by its own name.
        pub app: String,
        /// The server's own name, which need not be unique; empty for an app's one server.
        pub name: String,
    }

    impl Described {
        /// How a menu names it: "App – Server", or the app's name alone for a server with none.
        pub fn label(&self) -> String {
            match (self.app.is_empty(), self.name.is_empty()) {
                (_, true) => self.app.clone(),
                (true, false) => self.name.clone(),
                (false, false) => format!("{} – {}", self.app, self.name),
            }
        }
    }

    /// How a received surface is read: Syphon's bottom row first unless `flip`, and opaque
    /// unless `transparent`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Look {
        pub flip: bool,
        pub transparent: bool,
    }

    /// An [`Inlet`] as a [`crate::video::Camera`] reads it: the slots its client writes each
    /// frame and its end into. The inlet must outlive the camera — dropping it stops the
    /// client.
    #[derive(Clone)]
    pub struct Stream {
        pub(crate) frame: Slot<Arc<Frame>>,
        pub(crate) error: Slot<String>,
        pub(crate) description: String,
    }

    impl Stream {
        /// The slots this stream's frames and its end are written into.
        pub fn head(&self) -> Head {
            Head::Slots {
                frame: Arc::clone(&self.frame),
                error: Arc::clone(&self.error),
                description: self.description.clone(),
            }
        }
    }

    impl PartialEq for Stream {
        fn eq(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.frame, &other.frame)
        }
    }

    impl Eq for Stream {}

    impl std::fmt::Debug for Stream {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_tuple("Stream").field(&self.description).finish()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A pretending thread is told what Linux tells every thread; letting go gives it this
        /// machine's own answer back.
        #[test]
        fn a_pretending_thread_has_no_syphon() {
            let here = here::unavailable();
            pretend_unavailable(true);
            assert_eq!(unavailable(), Some(MACOS_ONLY));
            assert!(!available());
            pretend_unavailable(false);
            assert_eq!(unavailable(), here);
        }
    }
}

/// The NDI® runtime, opened ahead of the plugin that uses it.
///
/// `gst-plugin-ndi` opens `libndi` itself, the first time an element starts, and keeps that
/// answer for the process. `preload` is handed what the plugin would try, in its order, with
/// every runtime file in the system's library folders after it, and opens the first of them
/// that opens, holding it open for the life of the process. On Linux that is what makes a
/// runtime the loader does not search for load at all: Fedora's `ld.so` does not search
/// `/usr/local/lib`, where NDI's installer puts `libndi.so.6`, and glibc hands the plugin's
/// later open of that bare name the object already loaded under it as its SONAME. On macOS it
/// opens nothing and answers [`Preload::Left`](ndi::Preload::Left): dyld's fallback already
/// searches `/usr/local/lib`, where NDI's installer puts `libndi.dylib`. Nor on Windows, where
/// the plugin opens `Processing.NDI.Lib.x64.dll` from the folder `NDI_RUNTIME_DIR_V6` names,
/// which NDI's installer sets.
pub mod ndi {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::ndi::preload;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::ndi::preload;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::ndi::preload;

    use std::path::PathBuf;

    /// What `preload` did.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Preload {
        /// This one opened, and is held open for the life of the process.
        Held(PathBuf),
        /// Nothing was opened: the plugin's own open is left to find the runtime.
        Left,
        /// Nothing opened: each one tried, with the loader's reason.
        Refused(Vec<(PathBuf, String)>),
    }
}

/// File dialogs, the file manager and the text editor.
///
/// `pick` blocks until a person answers, so it is only ever called off the frame thread;
/// `reveal` opens the desktop's file manager on a folder, `reveal_file` on the folder holding a
/// file with the file selected where the file manager can, and `edit_text` a text file in the
/// editor the desktop opens it with, each returning at once. `MANAGER` is the file manager's
/// name, for a *Show in …* button.
pub mod files {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::files::{MANAGER, edit_text, pick, reveal, reveal_file};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::files::{MANAGER, edit_text, pick, reveal, reveal_file};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::files::{MANAGER, edit_text, pick, reveal, reveal_file};

    /// What a dialog is asked for.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Pick {
        Folder,
        /// A file, with a filter named for what it holds. An empty extension list is any file.
        File(&'static str, &'static [&'static str]),
    }
}

/// Where this user's files live: `config` for preferences, `data` for what the app keeps for
/// itself — its log — and `documents` for projects, the person's documents folder, in their
/// own language where the desktop names it so. Each is the base directory; the app's own
/// folder under it is the caller's to name.
///
/// `DOCUMENTS_UNSET` is what a person would have to set for `documents` to answer, for a
/// status line, `NOT_IN_NAMES` the characters a folder's name cannot hold here, and
/// `DENIED_HINT` what to do about a folder this user is not allowed into, where there is
/// something to say.
pub mod dirs {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::dirs::{
        DENIED_HINT, DOCUMENTS_UNSET, NOT_IN_NAMES, config, data, documents,
    };
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::dirs::{
        DENIED_HINT, DOCUMENTS_UNSET, NOT_IN_NAMES, config, data, documents,
    };
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::dirs::{
        DENIED_HINT, DOCUMENTS_UNSET, NOT_IN_NAMES, config, data, documents,
    };
}

/// A box on the desktop saying why the app cannot go on, for an exit before any window of its
/// own is up: `fatal` puts it up and waits for it to be closed. `rfd`'s message dialog on
/// every machine — `zenity` on Linux, or `kdialog` where that is what the desktop has, a
/// `CFUserNotification` alert on macOS and a `MessageBoxW` on Windows. Called on the main
/// thread.
pub mod alert {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::alert::fatal;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::alert::fatal;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::alert::fatal;
}

/// The font families this machine has, by the names the Text node's letters are drawn with.
///
/// `installed` is every family's first name, sorted without regard to case, each once — or
/// `None` where the machine cannot be asked.
pub mod fonts {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::fonts::installed;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::fonts::installed;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::fonts::installed;
}

/// The menu bar, where the operating system draws one: AppKit's on macOS. Linux and Windows
/// have none, so their `Bar::install` answers `None` and `App` draws the egui bar instead — as
/// it does on any build with no native bar.
///
/// The menus arrive as the plain data below, which `App` makes from `ui::menu`'s model, and
/// what was chosen comes back as the [`Entry::tag`]s it gave them. `Bar::show` puts this
/// frame's menus up, `Bar::take` hands back what was chosen since the last frame, and
/// `Bar::pasteboard` is the system clipboard's text, for a Paste the bar took the key of.
pub mod menu {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::menu::Bar;
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::menu::Bar;
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::menu::Bar;

    /// One menu, on the bar or inside another.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Menu {
        pub title: String,
        pub hint: Option<String>,
        pub items: Vec<Item>,
    }

    /// One row of a menu.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Item {
        Entry(Entry),
        Submenu(Menu),
        Separator,
    }

    /// A row that is chosen.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Entry {
        pub label: String,
        pub enabled: bool,
        pub key: Option<Key>,
        /// A tick, on or off, for an entry that wears one.
        pub ticked: Option<bool>,
        pub hint: Option<String>,
        /// What `Bar::take` hands back when it is chosen. `None` for a row that only says
        /// something, and is never enabled.
        pub tag: Option<usize>,
        pub role: Role,
    }

    /// A key that chooses an entry: its character and the modifiers held with it.
    // Four independent keys held or not, as egui's own `Modifiers` has them.
    #[allow(clippy::struct_excessive_bools)]
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Key {
        /// Lower case: Shift is a modifier, not a capital.
        pub character: String,
        pub command: bool,
        pub shift: bool,
        pub alt: bool,
        pub ctrl: bool,
    }

    /// Where the operating system keeps an entry, when it keeps it somewhere of its own. A
    /// Mac puts Settings and Quit in the application menu, and answers About and Licences
    /// with the standard About panel that menu already holds.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Role {
        Plain,
        Settings,
        Quit,
        About,
        Licences,
    }
}

/// The third-party notices that differ by machine, for Help ▸ Licences: `RUST_CRATES`, every
/// Rust crate compiled in with its licence, and `GSTREAMER`, where the machine's GStreamer
/// comes from and under what terms. On Linux and Windows the crates are a committed file
/// compiled in; a Mac's are in its bundle, and it keeps AppKit's About panel instead of the
/// window.
pub mod notices {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::notices::{GSTREAMER, RUST_CRATES};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::notices::{GSTREAMER, RUST_CRATES};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::notices::{GSTREAMER, RUST_CRATES};
}

/// Files dragged onto the editor's window from another app, where winit does not hear them:
/// on Linux, a `wl_data_device` of its own on eframe's Wayland display, on a thread, whose
/// `FileDrop::take` hands the frame each [`filedrop::Drag`]. On macOS and Windows winit hears
/// drops itself, and `FileDrop` is there and says nothing.
pub mod filedrop {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::filedrop::{Drag, FileDrop};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::filedrop::{Drag, FileDrop};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::filedrop::{Drag, FileDrop};
}

/// What `--check` asks of the machine ([`crate::check`]): the GStreamer elements the app makes,
/// in `GROUPS` by what each serves and the plugin set it ships in, and `machine`, the report's
/// lines about everything past GStreamer and the GPU — on Linux the session, the libraries
/// opened at run time, the audio server and the MIDI sequencer, on Windows the Vulkan loader.
/// `os` names the operating system, its version and the desktop in one line, for Help ▸
/// Report a problem….
pub mod check {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::check::{GROUPS, machine, os};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::check::{GROUPS, machine, os};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::check::{GROUPS, machine, os};
}

/// The GPU's use, from the operating system's own counters rather than from a query the
/// renderer placed: this whole process's share of the render engine on Linux, the whole GPU's
/// on a Mac, which keeps no count per process, and nothing on Windows, whose counters are not
/// read.
///
/// `Clients::scan` finds what to read, which is worth doing now and again rather than every
/// time; `Clients::read` is a [`Read`], or `None` where there is nothing to read. `WHOLE_GPU`
/// says which of the two the machine gives, before anything is read. `render_engine_ns` is the
/// process's engine nanoseconds at once, for a benchmark, and zero on a Mac and on Windows.
pub mod gpu {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::gpu::{Clients, WHOLE_GPU, render_engine_ns};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::gpu::{Clients, WHOLE_GPU, render_engine_ns};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::gpu::{Clients, WHOLE_GPU, render_engine_ns};

    /// What `Clients::read` found.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Read {
        /// The render engine's nanoseconds, cumulative, summed over this process's clients,
        /// and the clients, since a total over another set is not comparable: a count to
        /// difference across a second.
        EngineNs(u64, Vec<u64>),
        /// The share of the whole GPU busy now, `0..=1`, every process's work together.
        WholeBusy(f32),
    }
}

/// Audio capture by name, beside the default input cpal already opens on every machine.
///
/// `sources` lists what can be listened to, microphones and loopbacks together; `element`
/// is the GStreamer source that captures one of them by the name `sources` gave, or by
/// [`crate::audio::device::DEFAULT_MONITOR`], which is the loopback on every machine. `Tap`
/// is a name read without GStreamer: `Tap::open` answers `None` for a name `element` opens,
/// and a tap, not yet running, for one it does not — the loopback on a Mac, a Core Audio
/// process tap. Linux's and Windows' loopbacks are GStreamer's, so their `Tap` is never made. `Tap::start` hands its blocks, mono at `Tap::rate`, to the analyzer's closure,
/// and dropping it stops it.
pub mod audio {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::audio::{Tap, element, sources};
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::audio::{Tap, element, sources};
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::audio::{Tap, element, sources};

    /// One thing that can be listened to.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Source {
        /// What [`element`] opens it by.
        pub name: String,
        /// What the panel shows: the system's own description of it.
        pub label: String,
        /// True for a loopback: what is being played rather than what is being said.
        pub monitor: bool,
    }
}

/// Which GStreamer elements this machine captures, decodes and encodes video with, and the
/// memory a frame can reach the GPU in without a copy.
///
/// The pipelines themselves are [`crate::video`]'s and the same everywhere; what differs by
/// operating system is the source at the head of a camera's (`device_element`, the devices
/// `capture_devices` lists and the first `first_capture_device` finds, the formats and sizes
/// a listed device offers, `device_caps`, and whether a menu shows a camera by its name
/// alone, `NAMED_CAMERAS`), the hardware
/// decoder a webcam's JPEG prefers (`prefer_hardware_jpeg`), the hardware encoders a clip's
/// cache is written with (`CODECS`), the name a status line gives them where none is
/// installed (`CODEC_HINT`) and what a pipeline of them needs before its stream ends
/// (`settle_before_eos`), and the zero-copy path: whether there is one for a
/// clip (`clip_dmabuf`) or a screen (`dmabuf_imports`, `dmabuf_caps`), the chain and format
/// that export it (`dmabuf_chain`, `dmabuf_format`), and the frame a sample in that memory
/// becomes (`dmabuf_frame`).
pub mod video {
    #[cfg(target_os = "linux")]
    pub use crate::platform::linux::video::{
        CODEC_HINT, CODECS, NAMED_CAMERAS, capture_devices, clip_dmabuf, device_caps,
        device_element, dmabuf_caps, dmabuf_chain, dmabuf_format, dmabuf_frame, dmabuf_imports,
        first_capture_device, prefer_hardware_jpeg, settle_before_eos,
    };
    #[cfg(target_os = "macos")]
    pub use crate::platform::macos::video::{
        CODEC_HINT, CODECS, NAMED_CAMERAS, capture_devices, clip_dmabuf, device_caps,
        device_element, dmabuf_caps, dmabuf_chain, dmabuf_format, dmabuf_frame, dmabuf_imports,
        first_capture_device, prefer_hardware_jpeg, settle_before_eos,
    };
    #[cfg(target_os = "windows")]
    pub use crate::platform::windows::video::{
        CODEC_HINT, CODECS, NAMED_CAMERAS, capture_devices, clip_dmabuf, device_caps,
        device_element, dmabuf_caps, dmabuf_chain, dmabuf_format, dmabuf_frame, dmabuf_imports,
        first_capture_device, prefer_hardware_jpeg, settle_before_eos,
    };
}
