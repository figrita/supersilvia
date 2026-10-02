# Proposal: a native macOS app, and the platform layer under it

**Status: the layer is built; the macOS backend is not.** `src/platform/` holds every service
the app asks of the operating system that is not the same on Linux and macOS, outside
`render/`. `platform/linux/` is the code the app runs on, moved there without a change in
behavior; `platform/macos/` compiles every service to a refusal naming the API it needs. This
file is what is left: the macOS implementation of each service, and what `render/`'s own
move off OpenGL needs from here. [architecture.md](../docs/architecture.md#module-layering)
has the module, and [decisions.md](../docs/decisions.md) why it is one module per service
rather than a trait.

The target is Apple Silicon, one binary, as a `.app` bundle.

## The interface

One inline module per service in `platform/mod.rs`, each re-exporting the same names from
`linux/` or `macos/` under `#[cfg(target_os)]`. The names are the ones the callers use and no
more.

| Service | Names | Called from | Linux | macOS needs |
| --- | --- | --- | --- | --- |
| `midi` | `Midi::{open, sources, connect_all}` | `app/mod.rs`, `app/midi.rs` (as `midi::Midi`) | ALSA sequencer, two clients, a reader thread | CoreMIDI through `midir`, one connection per source (**written**) |
| `screen` | `ask`, `Pending::poll`, `Cast::stream`, `Stream::element` | `synth/maininput.rs`, `nodes/screencapture.rs`, `video::Source::Screen` | xdg-desktop-portal through `ashpd`, a PipeWire remote read by `pipewiresrc`, on the one portal runtime | ScreenCaptureKit |
| `files` | `pick`, `reveal`, `Pick` | `app/files.rs` | `rfd`'s xdg-portal backend under the portal runtime; `xdg-open` | `rfd`'s native backend (`NSOpenPanel`), handed to the main thread (**written**); `open` (**written**) |
| `dirs` | `config`, `data`, `DATA_UNSET` | `preferences.rs`, `project.rs`, `app/files.rs` | XDG base directories | `~/Library/Application Support` (**written**) |
| `fonts` | `installed` | `video/text.rs` (the Text node's Font menu) | fontconfig | AppKit's `NSFontCollection` (**written**) |
| `gpu` | `Clients::{scan, read}`, `render_engine_ns` | `synth/meter.rs`, `examples/*_bench.rs` | DRM fdinfo in `/proc/self` | IORegistry `IOAccelerator` statistics, or the Metal renderer's own timestamps |
| `audio` | `sources`, `element`, `Source` | `audio/device.rs`, `audio/mod.rs` | PulseAudio (PipeWire answers it) through `pulsesrc` and the device monitor | `osxaudiosrc` for a named input; a Core Audio process tap for the loopback |
| `video` | `capture_devices`, `first_capture_device`, `device_element`, `prefer_hardware_jpeg`, `CODECS`, `dmabuf_format`, `clip_dmabuf`, `dmabuf_imports`, `dmabuf_chain`, `dmabuf_caps`, `dmabuf_frame` | `video/mod.rs`, `video/clip.rs` | V4L2, VA-API and NVENC, `vapostproc` and DMA-BUF | `applemedia`: `avfvideosrc`, VideoToolbox, `IOSurface` |

What a service hands back is the app's own plain data — a `midi::Message`, a `nodes::Frame`, a
path — and a service owns no node, graph or window. `tests/rules.rs` holds that the Linux
crates (`alsa`, `ashpd`, `fontconfig`, `tokio`, `gstreamer_allocators`) are named in
`platform/linux/` alone, the macOS ones (`midir`) in `platform/macos/` alone, and `rfd`, both machines'
file dialogs, in those two; `Cargo.toml` declares each for its own machine alone, so neither
build can pick up the other's by accident.

## What moved

| Was | Is |
| --- | --- |
| `src/portal.rs` | `src/platform/linux/portal.rs`, private to the backend |
| `src/video/screen.rs` | `src/platform/linux/screen.rs`; `Cast`'s fields became `Cast::stream()`, and `video::Source::Screen { fd, node }` became `Source::Screen(Stream)` |
| the device half of `src/midi/mod.rs` | `src/platform/linux/midi.rs`; `midi/` keeps `Message`, `Kind`, `Source` and the map, and `midi::Source` lost its sequencer address, which only the backend reads |
| the dialog and `xdg-open` in `src/app/files.rs` | `src/platform/linux/files.rs`; `Pick` is `platform::files::Pick` |
| the XDG lookups in `preferences.rs` and `project.rs` | `src/platform/linux/dirs.rs` |
| `installed()` in `src/video/text.rs` | `src/platform/linux/fonts.rs` |
| the DRM fdinfo reader in `src/synth/meter.rs` | `src/platform/linux/gpu.rs`, with its test |
| the device monitor in `src/audio/device.rs` and the `pulsesrc` head in `audio/mod.rs` | `src/platform/linux/audio.rs` |
| V4L2 enumeration and probing, `vajpegdec`, `vapostproc`, the DMA-BUF caps and the DMA-BUF branch of `frame_from`, from `src/video/mod.rs` and `clip.rs`; the codec table from `clip.rs` | `src/platform/linux/video.rs`, which is also where `render/dmabuf.rs` is asked what this GL imports |

## What stayed where it was, and why

- **`render/`** — `dmabuf.rs` and `picture/`, which gate themselves by `#[cfg(target_os)]`
  rather than going through `platform/`: `dmabuf.rs`'s import is Vulkan's, with a twin that
  refuses off Linux, and `picture/`'s `Host` is `wayland.rs` and `thread.rs` on Linux and
  `macos.rs` elsewhere, which refuses every window. The Wayland, sctk and calloop crates the
  Linux half names are declared for Linux alone.
- **The Camera node's device choices**, `/dev/video0` to `/dev/video3` in `nodes/camera.rs`.
  They are option values saved in projects, so the macOS spelling — an AVFoundation index or
  unique ID — is a file-format decision made with the macOS camera, not guessed now.
- **`audio::device::Device::Pulse` and `DEFAULT_MONITOR`** are the saved file's words for a
  named source and for the loopback. Renaming them breaks every saved Main Input; the macOS
  backend reads them as meaning "a named source" and "the loopback".
- **The VA-API fixtures** in `tests/video.rs` and `video/clip.rs`'s tests encode a test clip
  with `vah264enc` directly. They are the Linux suite, and a macOS suite would write its own.
- **The status line that asks for "VA-API (Mesa) or NVENC"** when no codec probes, in
  `synth/maininput.rs`, the `video` node and `video/encode.rs`. It is text a person reads on
  Linux; on macOS the codec table is empty until `vtenc` is measured, and the line should then
  name VideoToolbox.
- **Already portable, and left alone:** `cpal` (Core Audio), `gilrs` (IOKit), `rustix`'s
  thread CPU clock (`CLOCK_THREAD_CPUTIME_ID` exists on macOS), GStreamer core,
  `textoverlay`, `videotestsrc`, eframe's `wayland` and `x11` features (no-ops off Linux).
- **The development environment** — `scripts/doctor.sh`, `distrobox.ini`, `check.sh` — is a
  Fedora box's. A Mac needs its own doctor: Xcode's command-line tools, the GStreamer
  framework from gstreamer.freedesktop.org, and `PKG_CONFIG_PATH` at its `.pc` files.

The Main Input files (`maininput.rs`, `synth/maininput.rs`, `ui/maininput.rs`,
`app/maininput.rs`) have nothing Linux-specific of their own: their Linux reach is the
loopback, the screen and the camera paths, each now through a service.

## The macOS backend, service by service

In the order a person on a Mac would notice them missing.

1. **Folders and the file manager — written.** `~/Library/Application Support` for both
   preferences and projects, `open` for Reveal. Open question below on where projects go.
2. **File dialogs — written.** `rfd` 0.15 with `default-features = false` for macOS, which is
   `NSOpenPanel`. AppKit wants the panel on the main thread and `pick` runs on a thread of its
   own: rfd's blocking `FileDialog` hands the panel to the main queue with `dispatch_sync` and
   waits, and the main thread drains that queue because it is in `NSApp`'s `run` under our
   winit loop. `AsyncFileDialog` lost: it reads the main window from the calling thread and
   hangs the panel on it as a sheet. See `platform/macos/files.rs` and
   [decisions.md](../docs/decisions.md#the-file-dialog-runs-on-a-thread).
3. **Codecs.** Without one, no clip plays. `vtenc_h264_hw` / `vtenc_h265_hw` with `vtdec_hw`
   and `h264parse` / `h265parse`: every frame a keyframe through `max-keyframe-interval=1`,
   constant quality through `quality`. Measure the properties before writing the table, since
   the cache's file name carries the codec's `name` and a wrong entry is a re-encode on every
   machine that shares the folder.
4. **Cameras.** `avfvideosrc device-index=<n>`; the device monitor over `Video/Source` answers
   through AVFoundation's provider. Settle what a project stores (see *What stayed*). The
   bundle needs `NSCameraUsageDescription`, and the first open asks the person.
5. **MIDI — written.** `midir` 0.11 for macOS only, with its default features, which are
   CoreMIDI. `MidiInput::connect` calls back on CoreMIDI's own thread, which is the reader
   thread: the callback parses the three bytes into a `midi::Message` and sends it down the
   queue. `ports()` is `sources`; `connect_all` opens one connection per port not yet held,
   each with a client of its own since `connect` consumes one and CoreMIDI has no third-party
   subscription for a second client to make, and closes the connection of every port no
   longer offered. A port is known by its CoreMIDI unique ID, which a device keeps across
   being unplugged, so Rescan drops a device that went away and wires it in again when it is
   back. See `platform/macos/midi.rs`.
6. **The loopback and named inputs.** A named input is `osxaudiosrc device=<AudioDeviceID>`
   from the device monitor over `Audio/Source`. The loopback has no device on macOS: from
   14.2, a Core Audio process tap (`AudioHardwareCreateProcessTap` over a `CATapDescription`
   of every process, in an aggregate device) reads what is playing, and needs
   `NSAudioCaptureUsageDescription`. It feeds the analyzer the way the cpal callback does
   rather than through GStreamer, which means `audio::element` grows a sibling for sources
   that are not a pipeline, or the tap is wrapped in an `appsrc`.
7. **Screen capture.** `SCContentSharingPicker` (macOS 14) is the system's own picker, the
   portal's counterpart, answering with an `SCContentFilter`; an `SCStream` over it delivers
   `CMSampleBuffer`s backed by `IOSurface`s. `ask`/`Pending`/`Cast` keep their shape — the
   picker off the frame thread, the answer through the polled channel, the stream stopped on
   drop. `Stream` does not: there is no GStreamer element that reads an `SCStream`, so either
   the cast pushes into an `appsrc` and `Stream::element` names it, or a screen stops being a
   `video::Camera` and publishes `IOSurface` frames straight to the renderer, which is the
   zero-copy path Metal imports anyway. The second is better and waits for `render/`. Screen
   Recording permission is asked by the picker's first use.
8. **Fonts — written.** The family of every descriptor in AppKit's
   `NSFontCollection::fontCollectionWithAllAvailableDescriptors`, through `objc2-app-kit`,
   which the lock already holds, sorted and deduplicated as fontconfig's list is.
   `NSFontManager`'s `availableFontFamilies` is the same list, but objc2 holds the manager to
   the main thread, and a test asks for the menu on a thread of its own; Core Text's
   `CTFontManagerCopyAvailableFontFamilyNames` needs `objc2-core-text`, a package the lock
   does not have. pango on macOS finds a family through Core Text, so a family on the menu
   draws as itself, which `video/text.rs` tests. A family whose script has no Latin letters
   draws Latin text in the fallback, as a browser does.
9. **GPU counters.** No per-process engine counter exists. The IORegistry's `IOAccelerator`
   `PerformanceStatistics` carries the whole GPU's `Device Utilization %`, which the Status
   box should label as the GPU's rather than the process's; the Metal renderer's command
   buffers carry `GPUStartTime`/`GPUEndTime` for the synth's own share.

## What `render/`'s rewrite needs from here

`render/` is out of this layer's scope, and it is the larger half of a Mac port. What it will
ask the platform for, so that the wgpu design can shape the questions:

- **Windows, per operating system.** A picture window is a borderless surface dragged by its
  picture, resized from an edge band, fullscreen on `F` or a double-click, closed on `Escape`,
  paced by its own display. On Linux that is `render/picture/`: a Wayland surface of our own
  on a thread of its own, sharing eframe's `wl_display` because an EGL share group cannot
  cross two `EGLDisplay`s. **None of that constraint survives wgpu** — one `wgpu::Device` is
  shared by any number of surfaces without a share group — and **none of the threading
  survives macOS**: AppKit creates and drives every window from the main thread, so a
  pictures thread owning windows is Linux-only. **On macOS this is built**
  ([macos-windows.md](macos-windows.md)): winit windows made inside eframe's own event loop,
  each with a `wgpu::Surface` drawn on a thread of its own in `PresentMode::Fifo`, with no
  `unsafe`. That is the portable shape; Linux keeps its Wayland code, and could adopt it later.
- **Keys by position.** `picture/thread.rs` reads `Escape` and `F` as evdev keycodes, which is
  what Wayland's `wl_keyboard` carries. Under winit that is `KeyCode::Escape` and
  `KeyCode::KeyF` — physical keys on every platform, with no evdev table.
- **Frames without a copy.** A frame reaches the renderer as `nodes::Pixels`: `Mapped` bytes
  everywhere, or `DmaBuf { fd, fourcc, modifier, stride, offset, keep, refused }` on Linux.
  macOS needs a sibling variant carrying a retained `IOSurface` (or `CVPixelBuffer`) and its
  plane layout, imported through `CVMetalTextureCache` or `MTLDevice::newTextureWithDescriptor:
  iosurface:plane:`. wgpu has no portable external-memory import, so the import is
  backend-specific `unsafe` in `render/` either way (`wgpu-hal`'s Vulkan or Metal texture
  from raw); the platform side is producing the handle. Keep the variant opaque to everything
  outside `render/`, as `keep` is now.
- **The two questions `platform::video` asks the renderer.** `dmabuf_imports()` and
  `dmabuf_caps()` ask `render/dmabuf.rs` whether this GPU imports DMA-BUFs and in which
  fourcc/modifier pairs, before a screen cast or a clip asks its source for them. The wgpu
  renderer must answer the same two questions — on Linux from Vulkan's
  `VK_EXT_image_drm_format_modifier` query, on macOS trivially (`IOSurface` always imports) —
  from a context-free place, since the question is asked from the synth thread before any
  frame. That edge is the one `tests/rules.rs` steps over, and it moves with the answer.
- **GPU time.** `render/timing.rs` places `GL_TIME_ELAPSED` queries; wgpu has timestamp
  queries (`Features::TIMESTAMP_QUERY`) on both backends. The whole-process figure is
  `platform::gpu`'s, above.
- **Identity.** `supersilvia`, `supersilvia-popout` and `supersilvia-fullscreen` are Wayland
  app ids matched to desktop entries. On macOS the identity is the bundle's
  `CFBundleIdentifier` and the icon is the `.icns` `scripts/make-icons.py` already renders; the
  usage descriptions (camera, microphone, audio capture, screen recording) live in the
  bundle's `Info.plist`. `packaging/` grows a `macos/`.

## The cross-check

`rustup target add aarch64-apple-darwin` and `cargo check --target aarch64-apple-darwin` on the
Linux box stop in build scripts before the crate compiles: `glib-sys`, `gobject-sys`,
`gio-sys` and the `gstreamer-*-sys` crates find no pkg-config for the target, which on a Mac is
the GStreamer framework's; and `wayland-sys`, which is `render/`'s. `alsa-sys`,
`yeslogic-fontconfig-sys` and `gstreamer-allocators-sys` no longer appear — they are Linux's
alone. With `PKG_CONFIG_ALLOW_CROSS=1` the build scripts pass and `smithay-client-toolkit`
fails to compile against `rustix` on Apple, which is `render/picture/`. `platform/mod.rs` and
`platform/macos/` type-check and pass clippy for the Apple target in isolation, against
stand-ins for the app types they name.

## Open

- **Where projects live on a Mac.** `~/Library/Application Support/supersilvia/projects`
  mirrors Linux's hidden data folder; `~/Documents/supersilvia` is where a Mac user would look
  for work they made. Since decided: `~/Documents/supersilvia`.
- **What a camera is in a saved project** on macOS: an index is unstable across plugging, a
  unique ID is stable and unreadable.
- **Whether a screen stays a GStreamer source** on macOS, or becomes the first source that
  hands the renderer `IOSurface`s directly. The second waits for `render/` on Metal.
