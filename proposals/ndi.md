# Proposal: NDI

**Status: approved, every recommendation under *Decisions* taken; built but for
the `.app`** — the plugin, sending and receiving are in (`docs/media.md`, `docs/rendering.md` and
`docs/ui.md`, *NDI*), untried against a real NDI runtime; the entitlements and Info.plist are packaging's.
Users have asked for NDI. This says what it is, how it
differs from Syphon, what its licence asks of an open-source app, how other software integrates
it, what supersilvia already has, the routes with the one recommended, and the questions it
raised. Everything here was read in NDI's documentation, the GStreamer NDI plugin's
source (`gst-plugin-ndi` 0.15.4) and other projects' pages, and checked against Homebrew's
GStreamer on a Mac; nothing was built or run against a real NDI source. A claim that
could not be checked that way says **(unverified)**.

## What NDI is

**NDI sends live video and sound between machines over the local network.** One app *sends* a
stream; any machine on the same network can find it by name and *receive* it, with its sound.
It is by Vizrt (formerly NewTek), runs on macOS, Windows, Linux, iOS and Android, and is
royalty-free but proprietary: the library that does the work, `libndi`, is closed source.

**Why people want it.** A live rig often spans machines: supersilvia on a laptop, Resolume on
the projection PC, OBS on the streaming PC, TouchDesigner on a third. Syphon only links apps on
one Mac; NDI links the machines, whatever they run. It also works on one machine, less
efficiently than Syphon. Resolume, OBS (through the DistroAV plugin), TouchDesigner, vMix,
MadMapper, Millumin, VDMX and Isadora all speak it **(unverified for each app's current
version)**.

**The words.**

- A **sender** is one published stream, named `MACHINE (Stream)` on the network, such as
  `STUDIO-MAC (supersilvia Output 3)`. The machine part is added by NDI.
- A **receiver** takes one sender's picture and sound.
- **Discovery** finds senders by Bonjour (mDNS) on the local network, or through an NDI
  Discovery Server where multicast is blocked.

**What users expect of an app that "supports NDI"**: each output sendable by a toggle under a
clear name; a menu of the network's NDI sources to take any of them as an input; alpha when the
sender has it; sound with the picture.

## How it differs from Syphon

| | Syphon | NDI |
| --- | --- | --- |
| Reach | apps on one Mac | any machine on the network, any OS |
| Frames | shared on the GPU, no copy | read back to the CPU, compressed, sent |
| Delay | none | about one to three frames |
| Sound | no | yes, with the picture |
| Alpha | convention unspecified | part of the format |
| Orientation | bottom row first by convention | top row first, always |
| Precision here | 8-bit | 8-bit through the GStreamer plugin; NDI 6 itself has 16-bit formats |
| Licence | BSD, vendored | proprietary runtime the user installs |

They complement each other: Syphon for the next app on the same Mac, NDI for the next machine.

## The licence, and what it asks of an open-source app

This is the part that decides the design. Not legal advice; it is what the documents say and
what other open-source projects do.

- **`libndi` is proprietary.** supersilvia is AGPL-3.0. Distributing a copyleft program
  together with a closed library is the conflict that kept OpenLP from adding NDI.
- **The accepted route is to not ship it.** NDI's own distribution page says open-source
  projects may include the (MIT-licensed) headers and **load the library dynamically at run
  time**; the user installs NDI's free runtime from ndi.video and accepts its licence
  themselves. OBS's NDI plugin, DistroAV (GPL-2.0), works exactly this way: "the NDI Runtime
  must be installed separately".
- **The GStreamer NDI plugin already does it.** `gst-plugin-ndi` (MPL-2.0, compatible with the
  AGPL) carries its own bindings and opens `libndi` with `libloading` only when an NDI element
  starts: `libndi.dylib` on macOS, `libndi.so.6` then `.5` on Linux, from `NDI_RUNTIME_DIR_V6`
  or `NDI_RUNTIME_DIR_V5` if set, else the system's search path. With no runtime, the element
  fails to start with "Failed loading NDI SDK"; registering the plugin needs nothing. So
  supersilvia would link nothing proprietary, and **building it needs no NDI SDK**.
- **An explicit exception would make it airtight.** Every commit in the repository has one
  copyright holder, who alone can add an AGPL section 7 additional permission: linking with and
  loading the NDI runtime is allowed. One paragraph in `LICENSE`'s preamble or `README`.
  Recommended; it removes the "is loading at run time a combined work" question outright.
- **Branding rules come with it** (NDI's licensing page):
  - "NDI®" with the mark on first mention in the UI and docs;
  - "NDI® is a registered trademark of Vizrt NDI AB" on the same page or in a footnote, and in
    the About box;
  - a link to ndi.video near where NDI is used, on the website and in the docs;
  - "NDI" never in the product's own name without asking them.
- **Only plain NDI.** NDI|HX (H.264/H.265) may need separate codec licensing and the paid
  Advanced SDK; the plugin's `advanced-sdk` feature stays off.

## How other software does it

- **OBS / DistroAV**: a plugin, GPL-2.0, loading the user-installed NDI 6 runtime at run time;
  a *NDI Source* in the source list, and an output setting that sends the program.
- **GStreamer** (`gst-plugins-rs`, `net/ndi`): `ndisrc` + `ndisrcdemux` receive, `ndisink`
  sends picture or sound, `ndisinkcombiner` puts both into one stream, and a device provider
  lists the network's sources as devices of class `Source/Audio/Video/Network`. Homebrew's
  GStreamer 1.28.7 on macOS already ships it (`gst-inspect-1.0 ndi`), plugin 0.15.3.
- **Resolume, TouchDesigner, vMix**: built in, with the runtime bundled by the vendor under
  their commercial licences, which an open-source app cannot copy.
- **Rust crates binding the SDK** (`grafton-ndi`, `ndi-sdk-sys` and older `ndi` crates): most
  link against the SDK at build time, which needs the SDK on every build machine and puts us
  back in the licence question.

## What supersilvia already has

More than for Syphon, because NDI lives where GStreamer does:

- **GStreamer, at the plugin's version.** `gst-plugin-ndi` 0.15.4 is on crates.io and depends on
  `gstreamer` 0.25, which supersilvia uses. It can be a Cargo dependency registered statically
  in our binary, so neither Homebrew nor a Linux distribution has to package it, and the `.app`
  carries it like any other crate.
- **A camera's shape for received frames.** A camera is one pipeline ending in an `appsink`
  with `sync=false`, `max-buffers=1`, `drop=true`, whose frames are held as `Pixels::Mapped` in
  the device's own layout and converted on the GPU. NDI hands out **UYVY** when a source is
  opaque and **BGRA** when it has alpha (`ndisrc`'s default `color-format`), and both are in
  `video::UPLOADABLE` already: no `videoconvert`, no CPU conversion.
- **A Main Input whose audio can follow its video.** The *Video source* audio choice follows
  whatever the video source is, so an NDI source's sound reaching the analyzer is that choice
  doing what it already does.
- **A publisher thread and its ticks.** Syphon's publisher (`render::syphon`) is a thread off
  the synth that blits each published picture into 8-bit BGRA with a picture window's `Viewer`,
  woken by the synth and by its own GPU work. The Output already has a tick row, the mixer a
  mark, and the render plan a reason to draw a published Output (`Why::Syphon`).
- **Readbacks that never wait.** The thumbnail and Snap readbacks copy a frame into a staging
  buffer and collect the map a tick later. An NDI sender needs the same, every frame, latest
  wins.

## The routes, and the one recommended

### Sending

- **A. The Syphon publisher, with a readback, into `ndisink` (recommended).** The publisher
  thread draws the picture into a BGRA texture the way it does for Syphon (top row first, no
  flip), copies it to a staging buffer, and, when the map lands, pushes the bytes to an
  `appsrc ! ndisink` pipeline of that sender's own. Nothing waits: a frame whose map has not
  landed when the next is ready is dropped, as a camera's is. One code path on both systems.
- **B. On a Mac, the IOSurface as the buffer.** Draw into an `IOSurface` as Syphon does and hand
  its memory to GStreamer, skipping the staging copy on Apple Silicon's unified memory. Less
  copying, but Mac-only, and it needs a pool of surfaces while the NDI encoder holds one. A
  later optimisation of A if the measurements ask for it.
- **C. Our own bindings to `libndi`** with `libloading`, no GStreamer. Full control, including
  NDI's asynchronous send, but it is the plugin rewritten, `unsafe` FFI of our own, and
  discovery, timing and sound to do again.
- **D. A crate that links the SDK.** Rejected: the SDK at build time and the licence question.

### Receiving

**One route stands out:** a camera-shaped pipeline, `ndisrc ndi-name=… ! ndisrcdemux`, its video
pad into an `appsink` capped to `UPLOADABLE`, its audio pad into the Main Input's audio when that
follows the video. Frames land as `Pixels::Mapped` in the one-frame slot a `Camera` adopts, as
`Source::Ndi`. Sources are listed by a `gst::DeviceMonitor` filtered to
`Source/Audio/Video/Network`, which is how the plugin's device provider reports them.

### Linking the plugin

- **The crate, registered statically (recommended).** `gst-plugin-ndi` as a dependency with
  `default-features` plus `static`, and `gstndi::plugin_register_static()` at start-up. Built
  with everything else, the same on both systems, nothing to install but NDI's runtime. It is
  a crate not yet on the approved list, with `libloading`, `quick-xml` and a few small ones
  behind it.
- **The system's plugin.** Homebrew has it; Linux distributions may not **(unverified which
  do)**, and the `.app`'s bundled GStreamer would need it too. Less to compile, more to
  explain to every user.

## How it would look in the app

- **Sending an Output**: an **NDI** tick in the Output's tick row beside **Syphon**, saved with
  the project; the stream is named `supersilvia Output <id>` until the Output is given a
  name of its own, so the network shows `MACHINE (supersilvia Output 3)` — the name is one
  field under Send, the same for Syphon ([docs/ui.md](../docs/ui.md#sending-an-output-out)). A sent Output is drawn with its tab closed, as a Syphon one
  is.
- **Sending the mix**: an NDI mark in Main Mixer ▸ Window beside the Syphon mark, as
  `supersilvia Mix`, not saved.
- **The ticks it shares**: NDI has one orientation, so **Flip** stays Syphon's alone.
  **Transparent** means the same thing for both and can be shared.
- **Receiving**: *NDI* among the Main Input's video sources with a menu of the network's
  sources, and an **`ndi`** node like the `syphon` node, one source each, with a **Transparent**
  tick. A source that goes away keeps its last frame and is taken up again when it returns.
- **Without the runtime**: the ticks and menus still show; the node's status and the tick's
  tooltip say "The NDI® runtime is not installed — get it at ndi.video", once, and nothing
  retries until the user asks.
- **Branding**: "NDI®" on first mention in each place, the trademark line and link in the About
  box and in `docs/`.

## Costs and risks

- **A readback and a CPU encode per sent picture per frame.** 1080p BGRA is about 8 MB a frame,
  about 500 MB/s at 60 fps, which unified memory takes easily; the SpeedHQ encode is `libndi`'s
  own, on the CPU **(unverified how many 1080p60 streams an M-series Mac sustains)**. Measured
  in release before anything is promised.
- **Bandwidth.** Plain NDI at 1080p60 is on the order of 100–150 Mb/s a stream. Gigabit wired
  is fine; Wi-Fi is a risk the docs should name.
- **Delay.** One to three frames, the network's and the codec's; nothing to do about it here.
- **8-bit.** The plugin's sink takes 8-bit formats only; values above 1 clip, as over Syphon.
- **The frame rate declared.** An NDI stream announces a rate; ours is the synth's, which
  varies. Declaring the display's rate and time-stamping each frame by the clock is the likely
  answer **(unverified how receivers treat an uneven stream)**.
- **macOS's Local Network permission.** Bonjour discovery triggers macOS 15's local-network
  prompt; the `.app` needs `NSLocalNetworkUsageDescription` and the NDI Bonjour service in
  `NSBonjourServices` **(unverified: the service name, and whether an unbundled binary is
  asked at all)**.
- **Library validation in the `.app`.** A hardened, notarised app loading a library signed by
  another team needs the `com.apple.security.cs.disable-library-validation` entitlement, as OBS
  has **(unverified for this case)**.
- **Where the runtime lands.** NDI's Mac installers put `libndi.dylib` where the system search
  path finds it **(unverified: the exact path, likely `/usr/local/lib`)**; `NDI_RUNTIME_DIR_V6`
  overrides.
- **Firewalls and multicast.** Discovery fails on networks that block mDNS; NDI's Discovery
  Server is the answer, and a setting for it is later work.

## Linux

**The same code works there**, which Syphon's never will: GStreamer and the plugin are the
same, and NDI's Linux runtime is `libndi.so.6`. So NDI is also the first way to send
supersilvia's picture out of a Linux machine. Trying it needs NDI's Linux runtime installed
**(unverified: which distributions package it; NDI's own installer is a shell archive)**.

**Where the runtime lands on Linux.** NDI's SDK installs `libndi.so.6` into `/usr/local/lib`,
which Fedora's `ld.so` does not search, so the plugin's own open of the bare name fails there.
The app opens the runtime itself, ahead of the plugin — where the plugin would, then by its full
path in the system's library folders — and holds it, so the plugin's open of `libndi.so.6`
finds it loaded under that SONAME (`platform/linux/ndi.rs`, `docs/media.md`). Putting
`libndi.so.6` in `/usr/local/lib` or `/usr/lib64` is the whole install, with no variable to
set. The loopback ran on Fedora on 28 September 2026 that way, with the NDI
6.3.2 runtime in `/usr/local/lib` and no `NDI_RUNTIME_DIR_V6`.

## Decisions

Each has a recommendation.

1. **Build it at all, after Syphon?** Recommend yes: it is the one link that crosses machines
   and operating systems, and GStreamer carries most of it.
2. **The AGPL exception for the NDI runtime.** Recommend adding it.
3. **The plugin as a crate, statically registered.** Recommend yes, over relying on the
   system's.
4. **Sound in what is sent.** The mix has no audio output of its own today: the monitor plays
   each source's slice to one device, off by default. Recommend picture only first, and a later
   decision on whether the monitor's mix goes out with the mix's picture.
5. **Transparent shared with Syphon, Flip Syphon's alone.** Recommend yes.
6. **Receiving through both** the Main Input and an `ndi` node, as for Syphon. Recommend yes.

## The build, in commits

1. **The plugin and one loopback.** `gst-plugin-ndi` registered statically; a test that sends a
   four-quadrant picture through `ndisink` and receives it with `ndisrc` in the same process,
   skipped with a message where the runtime is missing, which settles the byte order, alpha
   and delay on a real machine once the runtime is installed.
2. **Sending Outputs and the mix.** The readback on the publisher thread, a pipeline per sender,
   the tick, the mark, the render plan's reason to draw, the missing-runtime message, docs.
3. **Receiving.** The source listing, the Main Input source and the `ndi` node, the camera
   slot, the *Video source* audio following it.
4. **In the `.app`.** The entitlements and Info.plist entries above, the About box's trademark
   line, with the rest of packaging.

**Testing by hand needs no second machine**: NDI's free Tools for Mac include a monitor to
watch what supersilvia sends and a test-pattern sender to receive **(unverified: the current
tool names)**, and supersilvia can receive its own stream.

**What the loopback showed** (`tests/ndi.rs`, run on a Mac on 28 September 2026 with
the NDI 6 Runtime installed): a frame comes back 12 to 36 ms after it is sent over the machine's
own network stack; a picture sent top row first arrives top row first; sent opaque it arrives as
UYVY, with alpha as BGRA with its alpha; an Output sent by the publisher arrives the right way up
with its alpha; and the app's listing and receiver take it back as a camera, opaque or with its
alpha. **NDI Tools does not install the runtime**: each of its apps carries a private copy, and
only the NDI 6 Runtime (`ndi.link/NDIRedistV6Apple`, package `com.newtek.libndi`) puts
`libndi.dylib` in `/usr/local/lib`, which is what the `.app`'s `NDI_RUNTIME_DIR_V6` names. The
`.app` was tried from its disk image: an Output sent, the mix sent, and the Main Input and
an `ndi` node receiving. **`gst-plugin-ndi` 0.15's device provider cannot be restarted**: its
`stop` leaves the find instance in place, so a second `start` spawns a thread that finds it and
returns without polling; the app never stops it, and the test no longer does.

## Sources

- NDI: https://docs.ndi.video/all/developing-with-ndi/sdk/licensing ·
  https://docs.ndi.video/all/developing-with-ndi/sdk/software-distribution ·
  https://downloads.ndi.tv/SDK/NDI_SDK/NDI%20License%20Agreement.pdf
- The GStreamer plugin: https://gitlab.freedesktop.org/gstreamer/gst-plugins-rs (`net/ndi`,
  `src/ndisys.rs` for the loading, `src/device_provider/imp.rs` for the class) ·
  https://crates.io/crates/gst-plugin-ndi · https://gstreamer.freedesktop.org/documentation/ndi/
- Open-source integrations: https://github.com/DistroAV/DistroAV ·
  https://github.com/obs-ndi/obs-ndi/issues/230 ·
  https://forums.openlp.org/discussion/4594/openlp-support-for-ndi-output ·
  https://docs.ndi.video/all/using-ndi/using-ndi-with-software/getting-started-with-ndi-in-obs-for-windows-or-mac
- Syphon, for comparison: `proposals/syphon.md`
