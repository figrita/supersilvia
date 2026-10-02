# Proposal: media on macOS

**Status: approved; every step built.** This is the media lane of step 9 in
[wgpu.md](wgpu.md#the-steps). Every decision below is decided and the dependency list
approved. Clips play on the Mac, and so do cameras, audio inputs, the loopback, the
screen and zero copy.

On the Mac today a decoded frame already reaches the GPU. `nodes::Pixels::Mapped` is uploaded
with `queue.write_texture` and converted in a shader (`render/sources.rs`), and nothing there is
Linux's. What is missing is where frames come from: past the codec rows,
`src/platform/macos/{video,audio,screen}.rs` compile to refusals. This proposal fills them in,
in the order a person would miss them, and then removes the one copy left.

Everything below was measured on an M2 Mac (macOS 15.7.4, Homebrew GStreamer 1.28.7) or
read in the crates the lock holds. No camera, microphone or screen was captured. A claim that
could not be checked that way says **(unverified)**.

## What you can do

The first is built; the rest are not.

1. **Clips play.** Importing a clip shows the *Preparing clip…* clock, then it plays, scrubs and
   runs backwards as on Linux. A render to a video file works. A project folder last used on
   Linux plays its clips at once, with no second import (decision 3).
2. **Cameras.** The Camera node's *Device* menu lists the Mac's cameras by name. The Main
   Input's camera list does too. A saved Camera node reopens on the same camera after you plug
   another one in (decision 1).
3. **Audio inputs and the loopback.** The Main Input lists every microphone by name, beside
   the default one cpal already opens. *System audio* hears whatever the Mac is playing.
4. **The screen.** *Screen or window* puts up macOS's own picker. Choosing a window or a
   display shows it, with the pointer drawn in. The menu bar's *Stop sharing* stops it, and the
   status line says so. The Screen Capture node does the same.
5. **No copy.** A clip's decoded frame is sampled straight out of the decoder's memory, as on
   Linux. Then a screen's frame is too. You see no difference except in the Debug overlay's
   delivery line and its CPU and GPU figures.

## 1. Clips: the codec rows

### The rows

```rust
pub const CODECS: &[Codec] = &[
    Codec {
        name: "h264",
        encoder: "vtenc_h264_hw",
        decoder: "vtdec_hw",
        parser: "h264parse",
        intra: "max-keyframe-interval=1 quality=0.7",
        delivery: "quality=0.7 allow-frame-reordering=false",
    },
    Codec {
        name: "h265",
        encoder: "vtenc_h265_hw",
        decoder: "vtdec_hw",
        parser: "h265parse",
        intra: "max-keyframe-interval=1 quality=0.7",
        delivery: "quality=0.7 allow-frame-reordering=false",
    },
];
```

The names are Linux's VA-API names (decision 3). H.264 comes first so that both machines
choose the same codec. There is no AV1 row: no Apple chip encodes AV1.

### What was measured

**The properties** (`gst-inspect-1.0`). Both encoders take `max-keyframe-interval` (0 is
automatic), `quality` (0 to 1, default 0.5), `bitrate` (0 is automatic), and `rate-control`
with only `abr` and `cbr`. There is no constant-QP mode. `vtdec_hw` decodes H.264, H.265, AV1,
MPEG-2, JPEG, ProRes and VP9, at rank `primary + 1` (257).

**Every frame a keyframe.** With `max-keyframe-interval=1`, a 60-frame `videotestsrc` encode is
60 keyframes of 60, in both codecs. Every H.264 slice is an IDR (NAL type 5, slice type I, 60 of
60) and every H.265 picture is `IDR_N_LP` (type 20, 60 of 60), per ffmpeg's `trace_headers`.
Without the property the same encode is 2 I, 15 P and 42 B frames. The exact import chain from
`clip.rs` (`decodebin ! videoconvert ! videoscale ! … ! vtenc`) on a 120-frame 1080p delivery
file writes 120 keyframes of 120, in 1.4 s for H.264 and 1.0 s for H.265. Frames of 128×128,
192×144 and 320×240 encode too, which covers `clip.rs`'s 128-pixel floor and the tests' 144-line
cap. The streams are H.264 Main at level 4.0 (`avc1`) and H.265 Main at level 4.0 (`hvc1`),
8-bit 4:2:0, BT.709 studio range.

**`quality` is honoured with `bitrate=0`, and behaves as constant quality.** On 120 frames of
natural 1080p footage (macOS's own *Sequoia Sunrise* wallpaper movie, scaled), PSNR against
the source:

| `quality` | 0.5 | 0.6 | 0.7 | 0.8 | 0.9 |
| --- | --- | --- | --- | --- | --- |
| H.264: dB, MB a second | 36.3, 4.7 | 39.3, 8.2 | 42.1, 13.3 | 44.5, 20.4 | 46.7, 36.4 |
| H.265: dB, MB a second | 35.9, 3.0 | 38.6, 5.3 | 41.2, 8.9 | 43.5, 13.9 | 46.2, 28.1 |

For comparison, x264 all-intra at constant QP 22, 26 and 30 gives 43.4, 41.4 and 39.2 dB.
**0.7 is chosen because it lands where QP 26 does**, and VA-API's rows are constant-QP at the
element's default. That VA default is 26 is **(unverified)**; `gst-inspect-1.0 vah264enc` on
Linux says. The price is disk: a minute of 1080p is about 0.8 GB in H.264 and 0.5 GB in
H.265 at 0.7.

**The decoder and seeking.** decodebin picks `vtdec_hw` for both codecs on its own. Each
decode is bit-exact: every frame `vtdec_hw` gives equals ffmpeg's software decode of the same
file, for H.264, for H.265, and for an x264 all-intra High-profile file standing in for a Linux
cache. A spike seeks the way `Player` does (`FLUSH | ACCURATE` into `decodebin ! videoconvert !
appsink`) to frames 15, 3, 40, 0, 59, 31, 2, 58, 30 and 29. Every seek lands on its frame, and
every picture equals the one a straight decode gave at that index, on all three files.

**The delivery row.** `quality=0.7 allow-frame-reordering=false`, from RGBA through
`videoconvert` as `video/encode.rs` feeds it, writes a keyframe about every 30 frames with only
P frames between: an encoder told not to reorder has no frame it is keeping back, so it hands
each one back within milliseconds, which is what `settle_before_eos`'s wait at the end of a
stream relies on.

### The status line

The line a machine without a codec pair shows is written three times: `synth/maininput.rs`,
the `video` node and `video/encode.rs`. Each names `platform::video::CODEC_HINT`: "VA-API
(Mesa) or NVENC" on Linux and "VideoToolbox" on the Mac, as in "no hardware video encoder:
VideoToolbox is needed to import".

**Files:** `src/platform/macos/video.rs`, `src/platform/macos/mod.rs`,
`src/platform/linux/video.rs`, `src/platform/mod.rs`, `src/synth/maininput.rs`,
`src/nodes/video.rs`, `src/video/encode.rs`, `src/video/clip.rs` (one new test),
`tests/video.rs`, `tests/assets.rs`, `tests/gpu_app.rs` (their macOS comments),
`scripts/doctor.sh` (a VideoToolbox pair is a failure, not a warning), `DEVSETUP.md`,
`docs/media.md`, `docs/decisions.md`, `docs/testing.md`, `CONTRIBUTING.md` (the testing line that
said macOS skips).

## 2. Cameras

**A camera is `avfvideosrc device-index=<n>`.** The device monitor over `Video/Source` answers
through applemedia's `avfdeviceprovider` in 0.23 s, the whole `gst-device-monitor-1.0` run
included. Each device carries `device.api = avf`, `avf.unique_id`, `avf.model_id`,
`avf.manufacturer`, `avf.has_flash` and `avf.has_torch`. It carries **no `device.path`**, which
is what Linux's `capture_devices` reads. `Device::create_element` gives an `avfvideosrc` with
`device-index` already set (0 for the FaceTime camera here), and reading it needs no state
change.

**`avfvideosrc` has no unique-ID property.** It takes `device-index` (default −1, the system's
choice), `device-type` and `position`. `device-name` is read-only. So a camera saved by its
unique ID is found again by listing, matching `avf.unique_id`, and reading the index off the
element the device builds.

**What the Mac's backend answers:**

- `capture_devices()` is `(avf.unique_id, display name)` for each device.
- `first_capture_device()` is the first listed.
- `device_element(id)` looks the ID up in the last listing, and lists again if it is not
  there. It answers `avfvideosrc device-index=<n> ! video/x-raw`, or an error naming the
  camera. The caps keep it out of the GL textures `avfvideosrc` lists first, which a pipeline
  with no size asked for would otherwise settle on and fail to link.
- `prefer_hardware_jpeg` stays a no-op. `vtdec_hw` (257) already outranks `jpegdec` (256), and
  the existing test `mjpeg_decodes_on_the_video_engine_where_it_can` expects exactly that. A
  camera's JPEG never reaches decodebin on a Mac anyway: `avfvideosrc` only offers raw NV12,
  UYVY, YUY2, ARGB and BGRA, all but ARGB in `UPLOADABLE`.

**The Camera node's menu.** Its choices are `/dev/video0` to `/dev/video3` today, a static list.
`OptionDef::found` already exists for "choices the machine has" (the Text node's fonts), and a
value off that list still loads. On the Mac the node's `found` lists the cameras by name and
stores the ID (decision 1). It reads the list the Main Input's refresh already made, so opening
the menu never enumerates on the editor's thread. *Look for devices again* refreshes it.

The Main Input drops a camera when a project opens (`MainInput::restored`), so only the Camera
node's file format is at stake.

**Files:** `src/platform/macos/video.rs`, `src/platform/mod.rs`, `src/video/mod.rs`,
`src/nodes/camera.rs`, `docs/media.md` ("Cameras"), `docs/decisions.md`.

## 3. Named audio inputs and the loopback

### Named inputs

**The device monitor must not be used for audio on the Mac.** Over `Audio/Source`,
`osxaudiodeviceprovider` does not only list. It creates a HAL AudioUnit and binds it to the
built-in microphone to probe its formats (`gst_core_audio_open_device`, then
`gst_core_audio_bind_device`, in `GST_DEBUG`). Run from a terminal that could not
show a permission prompt, that probe never returned, in three tries of 10 s to two minutes.
Nothing was streamed. The likely cause is a microphone permission that could not be shown. `platform::audio::sources()`
runs inside the synth's tick, so a stall there stops the picture.

**Core Audio's properties list the inputs instead.** `kAudioHardwarePropertyDevices`, then per
device the input-scope `kAudioDevicePropertyStreams`, `kAudioObjectPropertyName` and
`kAudioDevicePropertyDeviceUID`, through `objc2-core-audio`. A spike listed the microphone and
the speakers this way in 57 ms, opening nothing. The built-in microphone's UID is
`BuiltInMicrophoneDevice`.

**A named input is `osxaudiosrc unique-id="<UID>"`.** `osxaudiosrc` has both `device` (an
`AudioDeviceID`, which can change between boots) and `unique-id` ("Unique persistent ID for the
input device"). The UID is what `Device::Pulse { name }` holds on a Mac. `provide-clock` and
`do-timestamp` exist on it as on `pulsesrc`, so the element line keeps Linux's
`provide-clock=false do-timestamp=true`. That `osxaudiosrc` opens by `unique-id` is
**(unverified)**: checking would open the microphone.

### The loopback

**A Core Audio process tap, not ScreenCaptureKit.** ScreenCaptureKit's system audio needs a
stream over a display, which needs either the picker or the Screen Recording grant, for a
feature that has nothing to do with the screen. The tap needs only the audio-capture grant.

**The shape**, every name read in `objc2-core-audio` 0.3.2, which the lock already compiles
under cpal with `AudioHardware`, `objc2` and `objc2-foundation` on:

1. `CATapDescription::initStereoGlobalTapButExcludeProcesses` with an empty list: every
   process, supersilvia included, as a PulseAudio monitor includes it on Linux (decision 4).
   `setPrivate(true)`, `setMuteBehavior(Unmuted)`.
2. `AudioHardwareCreateProcessTap` gives the tap's ID.
3. `AudioHardwareCreateAggregateDevice` over a dictionary with `private`, `tapautostart` and
   `taps = [{uid: <the description's UUID>}]`. Whether it also needs the default output as its
   main sub-device, for a clock, is **(unverified)**.
4. `kAudioTapPropertyFormat` gives the rate and the channels.
5. `AudioDeviceCreateIOProcID` and `AudioDeviceStart`. The IO proc runs on Core Audio's
   real-time thread and mixes to mono into a buffer grown once.
6. Dropping it stops the proc, then destroys the aggregate device and the tap.

**It feeds the analyzer the way cpal's callback does.** `audio::feed(rate, channels)` already
builds the closure a backend hands blocks to, and it allocates nothing after the first blocks.
So `platform::audio` gains a sibling of `element`: `Tap::open(name)`, which answers `None` for a
name GStreamer opens (every name on Linux, every UID on the Mac) and the tap for
`DEFAULT_MONITOR` on the Mac. `Tap::start(analyze)` runs the closure. `audio::Stream`, which is
`Cpal` or `Gst` today, gains a `Tap` variant. The saved words `Device::Pulse` and
`DEFAULT_MONITOR` stay.

*What lost:* wrapping the tap in an `appsrc` so `element` could name it. It adds a pipeline, a
caps negotiation and a resampler between an IO proc and a closure that already takes blocks.

**One difference from Linux.** `@DEFAULT_MONITOR@` is what goes to the default output. A global
tap is what every process plays, whichever output it goes to.

**Files:** `src/platform/macos/audio.rs`, `src/platform/linux/audio.rs`, `src/platform/mod.rs`,
`src/audio/mod.rs`, `packaging/macos/Info.plist`, `docs/media.md` ("The loopback").

## 4. Screen capture

**The picker.** `SCContentSharingPicker` (macOS 14.0 in the SDK) is macOS's own picker, the
portal's counterpart. It answers through an observer object: `contentSharingPicker:
didUpdateWithFilter:forStream:`, `…didCancelForStream:` and `…StartDidFailWithError:`.

**`ask`, `Pending` and `Cast` keep their shape.**

- `ask` dispatches to the main queue (`dispatch2`), where it makes the observer, sets the
  shared picker active, adds the observer and calls `present`. It returns a `Pending` at once.
- The observer's answer builds the stream where the answer arrives, then sends plain data back
  through the channel `Pending::poll` reads. The stream cannot cross threads:
  `Retained<SCContentFilter>` is not `Send`, which the spike's compiler said. A cancel sends
  "no screen was chosen", as Linux's does.
- `Cast` holds only `Send` things: the frame slot, the error slot, and a stop token. Dropping it
  dispatches to the main queue, which stops the stream and lets go of it. This is Linux's
  `Cast`: the session lives elsewhere and dropping the token ends it.

**The stream.** `SCStreamConfiguration` asks for `kCVPixelFormatType_32BGRA` at the content's
own pixel size (`contentRect` × `pointPixelScale` from the filter), with `showsCursor` on,
because Linux embeds the pointer. An output object implements `stream:didOutputSampleBuffer:
ofType:` on a serial queue of its own. It takes the sample's `CVPixelBuffer`, skips a sample with
none (an idle frame), and writes a `Frame` into the slot. The stream delegate's
`stream:didStopWithError:`, which the menu bar's *Stop sharing* causes
(`SCStreamErrorUserStopped` in `SCError.h`), writes "the source stopped" into the error slot,
where `Camera::error` already reads Linux's end-of-stream.

**`Stream` changes, and `Camera` gains a form without a pipeline.** No GStreamer element reads
an `SCStream`. `Stream::element` becomes `Stream::head`, answering either an element (Linux,
`pipewiresrc` as today) or the frame and error slots (the Mac). `Camera::open_with` builds a
pipeline for the first, and for the second adopts the slots, which are the same
`Arc<Mutex<Option<Arc<Frame>>>>` its appsink callback writes today. `latest`, `error` and the
two consumers, `synth/maininput.rs` and `nodes/screencapture.rs`, do not change.

**Frames skip GStreamer. This is the choice between two routes.**

- *Chosen:* the output handler maps the `CVPixelBuffer` (`CVPixelBufferLockBaseAddress`,
  read-only) and publishes it as `Pixels::Mapped`, BGRA, one plane, unlocked when the frame
  drops. That is bytes from the first commit, with no `videoconvert`, no caps and no pipeline.
  Section 5 then imports the same buffer's `IOSurface`.
- *What lost:* an `appsrc` bridge. `Stream::element` would name an `appsrc` the cast pushes into,
  each `CVPixelBuffer` wrapped in a `gst::Buffer`. It keeps `Camera` as it is, but adds a
  pipeline whose only job is to hand a buffer from one thread to another, and the `IOSurface`
  is lost inside the wrapped buffer unless a custom meta carries it back out.

**Files:** `src/platform/macos/screen.rs`, a new `src/platform/macos/pixels.rs` (a
`CVPixelBuffer` as a frame), `src/platform/linux/screen.rs` (`Stream::head`),
`src/platform/mod.rs`, `src/video/mod.rs`, `docs/media.md` ("Screen capture"),
`docs/decisions.md`.

## 5. Zero copy: IOSurface in place of DMA-BUF

### What the decoder hands over

**`vtdec_hw` offers two outputs** on its source pad: `video/x-raw(memory:GLMemory)`, NV12 in a
rectangle texture, and plain `video/x-raw` in NV12, AYUV64, ARGB64_BE, P010_10LE and RGBA64_LE.
There is no IOSurface or Metal caps feature. The app's chain takes the plain one.

**The plain one is the decoder's own buffer, not a copy.** A spike pulled a sample from
`decodebin ! videoconvert ! appsink` on a `vtenc` file:

- two memories, one per plane, both from `GstAppleCoreVideoAllocator`;
- three metas: `GstCoreVideoMetaAPI`, `GstVideoMetaAPI` and the SEI user-data meta;
- behind the CoreVideo meta, a `CVPixelBuffer` in `'420v'` (NV12, studio range), two planes,
  backed by an `IOSurface` of two planes.

**Mapping it costs 1.2 µs** a 1080p frame (a thousand maps and unmaps, release build). So a
zero-copy frame can carry its own bytes as a fallback at no cost.

**Reading the `CVPixelBuffer` needs a mirrored struct.** `GstCoreVideoMeta` is
`{ GstMeta meta; CVBufferRef cvbuf; CVPixelBufferRef pixbuf; }` in applemedia's
`corevideobuffer.h`, which Homebrew does not install. The spike mirrors it, finds the meta by its
API name, and checks the pointer with `CFGetTypeID` against `CVPixelBufferGetTypeID` before
using it. That check passed on every frame tried. A GStreamer that changed the struct would
fail it or worse; the struct being unchanged across GStreamer versions is **(unverified)**.

### The import

**It works.** The spike took the `IOSurface` (`CVPixelBufferGetIOSurface`, a safe function in
`objc2-core-video`), made one `MTLTexture` per plane with
`newTextureWithDescriptor:iosurface:plane:` (`R8Unorm` for Y, `RG8Unorm` for CbCr, shader-read,
shared storage), wrapped each with `wgpu::hal::metal::Device::texture_from_raw` and
`Device::create_texture_from_hal` (`initial_state: UNINITIALIZED`), and copied each back to a
buffer. Both planes read back byte for byte equal to the mapped frame, at 1920×1080 and at
640×360.

**The variant.**

```rust
/// A frame in an IOSurface, sampled where it lies. `surface` is an `IOSurfaceRef`, valid for
/// as long as `mapped.data` is alive; `mapped` is the same memory as bytes, for a device that
/// will not import it.
pub struct IoSurface {
    pub surface: usize,
    pub mapped: Mapped,
}
```

It sits beside `Pixels::DmaBuf` and is opaque in the same way. The handle is an integer, as
`DmaBuf::fd` is, so `nodes/` names no Apple crate and needs no `unsafe`. The keep-alive is
`mapped.data`, which holds the GStreamer buffer or the `CVPixelBuffer`. The layout, strides and
colour matrix are `mapped`'s. NV12 is two planes; a screen's BGRA is one.

**An import that fails uploads the bytes.** It needs no `refused` flag and no reopen, because on
a Mac the bytes are the same memory. That is simpler than Linux, where DMA-BUF and bytes are
different chains.

**The renderer.** `render/dmabuf.rs`'s non-Linux twin, which refuses every import today and
carries the `TODO(macOS media)`, becomes the Metal import:

- `supported(device)` is true on a Metal device, and `imports()` is true once the renderer has
  served one;
- `Imports::import_surface(device, &IoSurface, width, height)` returns one texture per plane.

`render/sources.rs` gains an arm. BGRA is the source's texture directly, as a DMA-BUF in `AR24`
is. NV12 becomes the two plane textures of the conversion pass, drawn with
`convert_block(Nv12, yuv)`. That is `Convert::padded` taking a list of imported planes rather
than one. The file keeps its name; its module doc names both imports.

**The return rule stays, unchanged.** A frame goes back to the decoder's pool only once the
renderer has let go of it, the submission after has finished, and no viewer's `Published` names
it: `Imports::claim`, `let_go`, `submitted` and `sweep`, as they are. The frame holds the
GStreamer buffer, which holds the `CVPixelBuffer`, which holds the `IOSurface`.
`texture_from_raw`'s drop callback is `None`.

**Where it switches on.** `platform::macos::video` answers the names Linux answers:

- `dmabuf_imports()` asks `render::dmabuf::imports()`, which is a second edge from
  `platform/` into `render/`. `tests/rules.rs`'s `KNOWN` list names it.
- `clip_dmabuf()` is `dmabuf_imports()`.
- `dmabuf_chain()` is the bytes chain, since NV12 passes `videoconvert` untouched.
- `dmabuf_frame()` reads the CoreVideo meta and answers `Pixels::IoSurface`. It answers `None`
  for a buffer with no meta, such as a software decoder's output.
- `dmabuf_caps()` and `dmabuf_format()` stay `None`, since a screen is not a pipeline.

**`frame_from` gains the delivery.** It consults `dmabuf_frame` only for `Delivery::DmaBuf`.
On Linux that changes nothing, because a bytes chain never carries DMA-BUF caps. On the Mac it
keeps a bytes pipeline's frames `Mapped`, which the tests that read pixels rely on.

**Screens second.** Once clips import, the screen's output handler publishes
`Pixels::IoSurface` with the one BGRA plane. Its alpha is assumed opaque **(unverified)**; if it
is not, the frame goes through the conversion pass as `Bgrx` does.

**Cameras stay on bytes**, as on Linux. Whether `avfvideosrc`'s buffers carry an `IOSurface` the
same import could take is **(unverified)**: checking means opening a camera.

**Files:** `src/nodes/cpu.rs`, `src/platform/macos/pixels.rs`, `src/platform/macos/video.rs`,
`src/video/clip.rs`, `src/video/mod.rs`, `src/render/dmabuf.rs`, `src/render/sources.rs`, a new
`tests/gpu_iosurface.rs`, `tests/rules.rs`, `docs/rendering.md` ("DMA-BUF import"),
`docs/media.md` ("Frames without a copy"), `docs/architecture.md` (the layering exception).

## Where the `unsafe` lives

The crate root denies `unsafe`, and CONTRIBUTING.md allows it in `render::picture`'s Linux half and
`render::dmabuf`. This lane needs it in four places:

- **`render::dmabuf`**, already allowed: the Metal import (`as_hal`, `texture_from_raw`,
  `create_texture_from_hal`, and the `IOSurfaceRef` rebuilt from its integer).
- **`platform::macos::pixels`**: the mirrored `GstCoreVideoMeta` read, the `CVPixelBuffer` lock
  and the plane slices it gives, and `Send`/`Sync` on the holder.
- **`platform::macos::screen`**: the observer and output classes (`define_class!` with `unsafe
  impl`), and ScreenCaptureKit's calls, which the bindings mark `unsafe fn` almost throughout
  (`sharedPicker`, `setActive`, `addObserver`, `present`, `initWithFilter…`,
  `addStreamOutput…`, `startCapture…`, `CMSampleBuffer::image_buffer`).
- **`platform::macos::audio`**: `AudioObjectGetPropertyData`, the tap, the aggregate device and
  the IO proc.

Each is re-allowed where `platform/macos/mod.rs` declares it, and every block carries a
`// SAFETY:` line.

Four shapes were weighed:

- **Per file, as above: chosen.** The `unsafe` sits beside the one service that needs it, and
  the allowance names three files.
- **One `platform::macos::sys` module** with every Apple call behind a safe wrapper, the three
  services safe. One allowance, but a layer of wrappers, and the Objective-C classes have to live
  in it too.
- **Third-party safe wrappers**, such as the `screencapturekit` crate. They are not in the lock,
  they cover only part of this, and they are still `unsafe` inside.
- **A Swift or Objective-C shim** built by a build script. It adds a second language.

The invariants row, CONTRIBUTING.md's hard rule and its "why" cell change in the first commit that
needs each file. `tests/rules.rs` gains the macOS half of the OS-crate rule: the Apple crates
(`objc2*`, `dispatch2`) are named in `platform/macos/` and `render/` alone.

## New dependencies

For macOS alone, approved as one list. Each is declared in the first commit that uses it; step
1 uses none. **Three are new packages. The rest are in the lock and
compiled already, or in the lock and not compiled today, and declaring them adds no package.**
The three new ones were resolved against a copy of the repo's lock: nothing else moved.

New to the lock:

- **`objc2-core-video` 0.3.2**, features `CVPixelBuffer`, `CVPixelBufferIOSurface` and their
  bases: a `CVPixelBuffer`'s planes, format and `IOSurface`.
- **`objc2-core-media` 0.3.2**, feature `CMSampleBuffer`: a screen sample's image buffer.
- **`objc2-screen-capture-kit` 0.3.2**, features `SCContentSharingPicker`, `SCStream`,
  `SCShareableContent`: the picker and the stream. Its default features would also pull
  `objc2-av-foundation`, which this does not need.

In the lock, declared directly:

- **`objc2` 0.6** (0.6.4): `Retained`, `define_class!`, `msg_send!`.
- **`objc2-foundation` 0.3** (0.3.2): `NSArray`, `NSString`, `NSError`, `NSDictionary`.
- **`objc2-core-foundation` 0.3** (0.3.2): `CFString`, `CFDictionary`.
- **`objc2-core-audio` 0.3** (0.3.2) and **`objc2-core-audio-types` 0.3** (0.3.2): the device
  list, the tap and the IO proc. Already compiled under cpal with the features this needs.
- **`objc2-metal` 0.3** (0.3.2), adding the feature `objc2-io-surface`: wgpu-hal compiles it
  without that feature, which gates `newTextureWithDescriptor:iosurface:plane:`.
- **`objc2-io-surface` 0.3** (0.3.2): in the lock, not compiled today.
- **`dispatch2` 0.3** (0.3.1): the main queue for the picker, and the screen's sample queue.

Nothing changes on Linux.

## Decisions

Every recommendation was taken.

**1. A saved Camera node names the camera itself** (decision 3 of the first Mac plan): AVFoundation's unique
ID. You plug a USB camera into the Mac, and a project that used the built-in camera reopens on
the built-in camera, whatever you plugged in, and after a reboot. The menu shows names; the
file stores the ID. A USB camera's ID may change when it moves to another port
**(unverified)**. A rig that changes cameras when a cable is plugged in is the failure a
performer notices on stage.

What the saved file stores:

| | |
| --- | --- |
| on Linux | `"device": "/dev/video0"`, unchanged |
| on the Mac | `"device": "<AVFoundation unique ID>"`, a 36-character UUID for the built-in camera |

The default, `auto`, is saved as `auto` and means the first camera on both machines. Most saved
nodes hold it. A camera a machine does not have shows black with a status line naming it, like
a missing clip. Choosing from the menu fixes it, and the next save writes this machine's camera.

*What lost:* **the position** (the index). It reopens on whatever is now in that place. The Mac
could even save Linux's words (`/dev/video0` meaning the first camera), so a file moves between
machines unchanged. It is unstable, and Linux's numbers do not match the Mac's anyway: a Linux
webcam often takes `/dev/video0` and `/dev/video1`.

**2. The oldest macOS it opens on is 14.2** (decision 7 of the first Mac plan). `LSMinimumSystemVersion` is
14.2, and the deployment target matches it. Every Apple Silicon Mac can run macOS 14. Finder
refuses to open it on an older system, with its own message.

The picker needs 14.0 and the tap 14.2, per the SDK headers. Picture windows do not need 14:
they present in `Fifo` with no display link. The app is built for 11.0 until the loopback's
commit moves it (`minos 11.0` in both of the repo's binaries). At that target,
`AudioHardwareCreateProcessTap` links as a lazy bind that is not weak, which the spike's
`dyld_info` shows. On an older Mac the app would open, and the first loopback would kill the
process **(unverified on an older Mac)**.

*What lost:* **opening on older Macs**, without the loopback before 14.2 and without screen
capture before 14.0. It means checking the OS version before every such call.

**3. Mac and Linux clip caches share names** (`h264`, `h265`). A project folder last used on
Linux plays its clips on the Mac at once, and the reverse. The folder holds one copy of each
clip. The cache lives in the folder so that "a folder that carries its own plays on another
machine at once" ([decisions.md](../docs/decisions.md#assets-and-the-cache-live-inside-the-project)),
and that holds across the two systems only if the names match. So the Mac prefers H.264, to
match Linux. [decisions.md](../docs/decisions.md#on-a-mac-the-cache-is-written-by-videotoolbox-under-linuxs-names)
records it.

The measured half: an all-intra H.264 file of the kind Linux writes decodes bit-exactly on
`vtdec_hw` and seeks to every frame. The Mac's own files are standard Main-profile streams that
ffmpeg's software decoder reads bit-exactly. The unmeasured half: a file from the real
`vah264enc` on the Mac, and a Mac file on Linux's `vah264dec` **(unverified)**. A copy
made by either machine looks slightly different, since the encoders differ at the same quality.

*What lost:* **names of its own** (`vt-h264`, `vt-h265`). Each machine would prepare every clip
once, and a synced folder would carry two copies and sync both. The Mac could then prefer
H.265, which is a third smaller at the same quality and encodes faster.

**4. The loopback hears supersilvia itself.** On Linux it does, since a PulseAudio monitor is
everything going to the output, and the Mac matches it. You would notice it only when
monitoring the loopback, where it can feed back, as it already can on Linux.

*What lost:* **excluding the app's own process from the tap**, which is one line.

**5. The dependency list above** is approved.

## Permissions

| | Info.plist | unbundled binary | the `.app` |
| --- | --- | --- | --- |
| camera | `NSCameraUsageDescription`, present | the prompt and the grant belong to the app that launched it, the terminal **(unverified per terminal)** | hardened runtime: `com.apple.security.device.camera` |
| microphone, named inputs | `NSMicrophoneUsageDescription`, present | as above | `com.apple.security.device.audio-input` |
| system audio, the tap | `NSAudioCaptureUsageDescription`, **missing**: add "supersilvia listens to what your Mac is playing to drive the graph." | as above; what a tap delivers with no usage string at all is **(unverified)** | no entitlement known outside the sandbox **(unverified)** |
| screen | no key exists | whether the system picker needs the Screen Recording grant at all is **(unverified)**; the headers do not say | nothing outside the sandbox |

**Packaging must** carry these strings in the bundle's `Info.plist`, sign with the two
entitlements, and set `LSMinimumSystemVersion` to 14.2 (decision 2). Until it runs from the
`.app`, the prompts name the terminal, not supersilvia.

One thing seen here matters for development. GStreamer's audio device probe stalled when run
from a terminal host that could not show a prompt. So checks by hand should run from
Terminal.app, where a
permission prompt can appear.

## Tests

### What `cargo test` covers, with no device

- **The codec rows run the clip tests on the Mac.** A machine with no codec pair prints "no
  hardware codec pair here; skipping" in each of these; the Mac runs them all:
  - in `video/clip.rs`: `every_available_codec_encodes_intra_and_seeks`,
    `frames_arrive_in_any_order_and_the_same_index_decodes_the_same`,
    `a_transcode_lands_in_the_cache_and_plays`,
    `the_cache_name_follows_the_source_the_settings_and_the_codec` (it needs a second row, the
    H.265 one), `the_probe_prefers_the_first_available`,
    `dropping_a_transcode_cancels_it_and_removes_the_partial`,
    `a_poster_is_one_frame_of_the_file_itself`, `discover_reads_size_rate_and_length` and
    `a_request_past_the_end_is_clamped`;
  - in `tests/video.rs`: the clip tests and `an_encoder_writes_a_clip_frame_by_frame`, which is
    the delivery row;
  - in `tests/assets.rs`: `removing_an_asset_takes_its_cache_entries_with_it`;
  - the codec half of `tests/gpu_app.rs`, the render to a video file.

  One test is new and runs on Linux too, `every_frame_out_of_the_encode_chain_is_a_keyframe`:
  every buffer out of each available codec's `encode_chain` is a keyframe, which is the
  property the cache exists for, and the same encoder's delivery chain has frames between
  keyframes, which says the parser's flag is being read.
- **Cameras through `videotestsrc`.** `Camera::open_with(&Source::Test, …)` already runs the
  whole pipeline with no camera. New on the Mac: `device_element` of an ID no camera has is an
  error naming it, and `capture_devices` only prints, like Linux's test. Listing opens nothing
  (checked above).
- **Audio.** The analyzer is already tested pure. New: the IO proc's mono mixdown is a pure
  function with a test, and `element` builds its `osxaudiosrc` line. `sources()` only prints.
  No test starts a tap.
- **Screens.** A `CVPixelBuffer` made in the test (`CVPixelBufferCreate` with IOSurface
  properties) becomes a `Mapped` frame with the right strides and bytes. No test calls the picker.
- **Zero copy: `tests/gpu_iosurface.rs`**, mirroring `tests/gpu_dmabuf.rs`. It encodes
  `videotestsrc` through the H.264 row, decodes it with `Delivery::DmaBuf`, and checks:
  - the frame is `Pixels::IoSurface`;
  - it is imported as two planes and drawn through the conversion pass, and it samples within a
    step of the same frame uploaded as bytes;
  - a surface that cannot be imported uploads its bytes instead;
  - the return rule's two tests hold for this variant: a frame let go is held until the next
    submission has finished, and a viewer's claim holds it past the serial.
- **The rules.** `tests/rules.rs`: the new `KNOWN` edge, and the Apple crates named only in
  `platform/macos/` and `render/`.

**No camera, microphone or screen is opened by `cargo test`**, as CONTRIBUTING.md requires.

### By hand

Run from Terminal.app, so each permission prompt can appear.

1. Import a clip into a `video` node. The *Preparing clip…* clock runs, then the clip plays.
   Scrub it, set a negative speed, drive its position from a cable.
2. Render an Output to a video file. It opens in QuickTime.
3. Open a project folder whose clips were imported on Linux. They play with no *Preparing*
   (decision 3). Then the reverse on Linux.
4. Camera node: the menu lists the Mac's cameras by name. Choosing one shows it, and the camera's
   light comes on. Deleting the node puts the light out.
5. Plug in a USB camera, *Look for devices again*, choose it, save. Unplug it, plug it into
   another port, reopen: the node shows the same camera (decision 1).
6. Main Input: the audio list names each microphone. Choosing one moves the scope.
7. Main Input *System audio*: a prompt about system audio appears once. Play music in another
   app: the scope moves. Choose another source: the menu bar's recording indicator goes away.
8. Main Input *Screen or window*: macOS's picker appears. Choose a window: it shows, with the
   pointer. Note whether macOS also asked for Screen Recording. *Stop sharing* in the menu
   bar: the status line says the source stopped. Choose again and cancel: "no screen was chosen".
9. Two Screen Capture nodes at once, each picking a different window.
10. The Debug overlay shows a clip's delivery as zero-copy, and eight clips at once still play.
11. Quit with a clip, a camera, the loopback and a screen all running. It exits at once, and no
    new report appears in `~/Library/Logs/DiagnosticReports/`.

## The build, in commits

Seven commits, or nine if two of them split, and about sixty file changes in all, with overlaps.
Each is green on `./check.sh` on the Mac and on Linux, and each carries its own doc change and
the dependency lines it uses.

**First, alone:**

1. **Clips play. Built.** The rows, `CODEC_HINT`, the keyframe test, doctor's failure. Every
   later lane's clip tests run from here.

**Then two lanes in parallel**, each on its own branch:

*Devices*, in order:

2. **Cameras.** Listing, the ID lookup, the Camera node's `found` menu, decision 1. About 6 files.
3. **Named audio inputs.** The Core Audio listing, `osxaudiosrc unique-id=`, the first `unsafe`
   allowance and the macOS OS-crate rule. About 7 files.
4. **The loopback.** `Tap`, `audio::Stream::Tap`, `NSAudioCaptureUsageDescription`,
   `LSMinimumSystemVersion`. About 7 files.

*Pictures*, in order:

5. **Screen capture as bytes.** `pixels.rs`, the picker, the stream, `Stream::head`, `Camera`
   without a pipeline. About 11 files. It may split in two: the picker and stream, then
   `Camera`.
6. **Zero copy for clips.** `Pixels::IoSurface`, the Metal import, the conversion arm,
   `frame_from`'s delivery, `tests/gpu_iosurface.rs`. About 13 files. It may split in two: the
   import and its test, then the clip path.
7. **Zero copy for screens.** The BGRA plane as an `IoSurface`. About 4 files.

The two lanes touch different files, except for `Cargo.toml`, `Cargo.lock`, the invariants row,
CONTRIBUTING.md's rule and `docs/media.md`, which one merge resolves. *Pictures* keeps 5 before
6, because both write `pixels.rs`. That is two branches at most besides `main`.

## Risks and open questions

- **The mirrored `GstCoreVideoMeta`.** A GStreamer that changes applemedia's private struct
  breaks the zero-copy read. When the meta's API name or the pointer's type ID does not match,
  the frame goes as bytes. A pointer into unmapped memory would crash before the check could
  answer. *What lost:* decoding with VideoToolbox directly
  (`objc2-video-toolbox`), with GStreamer only demuxing. All-intra frames decode independently,
  so it is feasible. It is another binding, another session to manage, and more `unsafe`, to
  avoid one struct.
- **GL on the decoder.** Every `vtdec_hw` run printed GStreamer-GL's warning that "an
  NSApplication needs to be running on the main thread". In the app one is (winit's). Decoding
  worked in the spike without one. Whether the GL context `vtdec_hw` makes while negotiating
  ever matters is **(unverified)**.
- **An audio input that stalls on open. (unverified)** The device probe stalled binding an
  AudioUnit to the microphone. If `osxaudiosrc` stalls the same way when a named input opens, the
  open has to move off the synth thread. cpal's default microphone binds the same kind of unit
  and works, which suggests the stall was a missing permission.
- **The picker's thread and state. (unverified)** Which thread the observer is called on, and
  whether leaving the picker `active` keeps supersilvia in Control Center's sharing menu. The
  design sets it active only while asking.
- **A camera index that moves between listing and opening.** Plugging in a camera in that moment
  opens the wrong one. The window is one listing long.
- **Many clips at once. (unmeasured)** Each `Player` is a `vtdec_hw` session. How many an M2
  decodes at once at 1080p before frames arrive late.
- **Cache size.** About 0.8 GB a minute of 1080p at `quality=0.7` in H.264. That is VA-API's
  cost too, if its QP is 26.
- **The level.** The Mac's H.264 at 1080p reads level 4.0 at about 100 Mbit/s, past that level's
  bitrate. x264's all-intra files do the same. Whether any decoder on Linux enforces it is
  **(unverified)**.

## What the source settled

| question | answer | where |
| --- | --- | --- |
| `vtenc_*_hw` properties | `max-keyframe-interval`, `quality` 0–1, `bitrate` (0 automatic), `rate-control` `abr`/`cbr` only, `allow-frame-reordering`, `realtime` | `gst-inspect-1.0` |
| all-intra | 60 of 60 IDR in both codecs with `max-keyframe-interval=1`; 120 of 120 through `clip.rs`'s chain | ffprobe, `trace_headers` |
| constant quality | `quality` changes PSNR and size with `bitrate=0`; 0.7 ≈ x264 QP 26 | the table above |
| `vtdec_hw` decodes and seeks | bit-exact against ffmpeg, both codecs and an x264 stand-in; ten out-of-order seeks exact on each | framemd5; the spike's `seek` |
| `vtdec_hw` output | GLMemory or plain NV12/AYUV64/ARGB64_BE/P010/RGBA64; no Metal or IOSurface caps feature | `gst-inspect-1.0 vtdec_hw` |
| what the plain output is | `GstAppleCoreVideoAllocator` memory, `GstCoreVideoMeta`, a `'420v'` `CVPixelBuffer` on a two-plane `IOSurface` | the spike's `meta` |
| the Metal import | `newTextureWithDescriptor:iosurface:plane:` → `texture_from_raw` → `create_texture_from_hal`; both planes read back equal | spike's `import`; `wgpu-hal-30.0.1/src/metal/device.rs:415`; `wgpu-30.0.1/src/api/device.rs:341`; `objc2-metal-0.3.2/src/generated/MTLDevice.rs:997`, gated on `objc2-io-surface` |
| `DeviceMonitor` over `Video/Source` | `avfdeviceprovider`; `avf.unique_id`, no `device.path`; element `avfvideosrc device-index=N` | `gst-device-monitor-1.0`; spike's `cams` |
| `avfvideosrc` properties | `device-index`, `device-type`, `position`; `device-name` read-only; no unique ID | `gst-inspect-1.0` |
| `prefer_hardware_jpeg` | a no-op: `vtdec_hw` 257 outranks `jpegdec` 256, and `avfvideosrc` never offers JPEG | `gst-inspect-1.0` |
| `DeviceMonitor` over `Audio/Source` | binds a HAL AudioUnit to the microphone and stalled here; not usable | `GST_DEBUG=osxaudio*:6` |
| `osxaudiosrc` | `device` (an ID) and `unique-id` (persistent); `provide-clock`, `do-timestamp` | `gst-inspect-1.0` |
| listing inputs without GStreamer | Core Audio properties, 57 ms, nothing opened | spike's `audios` |
| the process tap in the lock | `AudioHardwareCreateProcessTap`, `CATapDescription`, `AudioHardwareCreateAggregateDevice`, `AudioDeviceCreateIOProcID`, already compiled under cpal | `objc2-core-audio-0.3.2/src/generated/AudioHardware.rs:1794`, `:1852`, `:1114`, `:1465` |
| availability | the tap 14.2; the picker 14.0; SCStream audio 13.0 | the SDK's `AudioHardwareTapping.h:44`, `SCContentSharingPicker.h:62`, `SCStream.h:30` |
| the tap at today's deployment target | `minos 11.0`; `_AudioHardwareCreateProcessTap` a lazy bind, not weak | `otool -l`, `dyld_info` |
| ScreenCaptureKit in Rust | `objc2-screen-capture-kit` 0.3.2 on crates.io, against `objc2` 0.6; almost every method `unsafe fn`; the filter is not `Send` | the spike compiles an observer and an output with `define_class!` |
| what the three new crates add | exactly those three packages | the spike's lock against the repo's |

**The spike** was `media-spike`, a scratch crate outside the repository. It decodes, seeks,
reads the CoreVideo meta, and imports into wgpu on Metal. It lists cameras and builds their element
without starting it, and lists audio devices through Core Audio. It compiles, and never runs:
the tap with its aggregate device and IO proc, and the picker's observer and the stream's output
classes. It never opened a camera, a microphone or a screen.
