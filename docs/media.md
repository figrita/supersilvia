# Media sources

The microphone, cameras, video files and still pictures — found and painted. All four are CPU nodes, so read
[cpu.md](cpu.md) first: this describes only what is particular to getting real-world data
into the frame on time. MIDI is here too, at the foot, because it is the same problem — a
device on its own thread — with the opposite answer about what to drop.

One shape is common to all of them. **Capture happens on somebody else's thread**, cpal's audio
callback or GStreamer's streaming threads, and the synth thread never waits on it. The handoff
keeps only the newest value: the producer writes it, the consumer takes it, the consumer never
blocks, and a slow consumer drops intermediate values rather than accumulating them. A synth
that is late is worse than a synth that skipped. For audio, a clip and the letters it is a
`triple_buffer`; for a camera or a screen it is a one-frame slot, because those frames are the
source's own buffers — see [Cameras](#cameras).

## The Main Input

One video source and one audio source, chosen in the left panel and read by any number of
`maininput` nodes. It is the rig's, like the mixer, and it is split the way the mixer is:
[`crate::maininput`] is the *choice* and `synth/maininput.rs` is what the choice costs — the open
camera, the open capture, the held portal session, which are the synth's for the reason every
open device is. The panel that draws it is `app/maininput.rs`.

**The choice is saved with the project, as silvia's is**, and Open brings back a clip, a Syphon
server, an NDI® source, a sound file and the tuning through `MainInput::restored`. A camera, a capture device
and a screen come back as *None* and the monitor off, so a project opens having opened no
device; a Syphon server or an NDI source is another app's picture, which switches nothing on, so it is taken up
again as soon as it runs; see
[decisions.md](decisions.md#the-mixer-does-not-persist-and-the-main-input-does).

One rule runs it: **`Live::reconcile` is called once a frame with the choice and makes what is
open match it.** Nothing else opens or closes anything, so switching sources five times in a
second opens the fifth and only the fifth, and there is no path where the panel says one thing
and a device does another.

**What the machine has is asked on a thread named `devices`**, once at start and again on *Look
for devices again*, and the panel's two lists are empty until it answers, on the first tick
after it does (`Live::ask_devices`, `Live::take_devices`). The asking is slow and has no bound
of ours: on Linux GStreamer's ALSA provider opens each card to read its formats, and a USB
webcam's microphone that did not answer held that open through the kernel's control timeouts
for 10.4 s — which, asked in the tick, was the whole of the synth's first tick and so the first
picture of every project. `a_device_listing_that_does_not_answer_holds_no_tick` holds the tick
to not waiting for it.

The tuning and the thresholds are the panel's, not the node's, because there is one analysis:
see [decisions.md](decisions.md). **The meters are on the node as well**, read-only: three
band bars with the panel's threshold square drawn where the panel has it, because a level is
set while the band it measures is being watched. The node does not draw the rig's picture —
the panel is holding it a few inches to the left, and one picture drawn twice is once too
many.

**The *Video source* audio is the video source's soundtrack** — an NDI source's own sound ([NDI](#ndi)), or a clip's, decoded as a `video` node's is
(see [A clip's soundtrack](#a-clips-soundtrack)), so a new video source is a new soundtrack
even though the audio choice reads the same, and `reconcile` reopens the audio with the video.
The panel's levels reach a clip's or a sound file's analyzer each frame, as they reach a
capture's audio thread, so its events fire as a microphone's do.

**A clip and a sound file are read at one `position`**, integrated live from the transport's
advance at the file's own speed — a `nodes::phasor::Phasor` at one, so it pauses with the show
and follows a seek — and looped. The Main Input is a source that is *on*: it has no Time, no
Offset and no rate, and everything a hand or a gear does to a clip belongs to a `video` node
([cpu.md](cpu.md#stateful-nodes-step-on-dt)). The frame is chosen by `video`'s rule,
`round(position × frames)` over the clip's own length.
That is what makes offline audio a small thing: a render *drives* the position to each
frame's own time (`Live::drive`), so `Reader::advance_to` analyzes the file at `t = i / fps`
and the clip's frame is the one at `t`, exactly and repeatably — and the position is put back
where live play had it once the render hands the clock back. The render also waits for the
clip's frame, as it waits for a `video`'s: [Video files](#video-files). A device — a microphone, the
loopback, a camera — has no `t` to be read at and hands back whatever it has, which is what
the Output's `!` says about it.

### Audio inputs by name

The panel lists every input the machine has beneath the default microphone, and a named one is
captured through GStreamer and handed to the same analyzer. On Linux the list is PulseAudio's
and the element `pulsesrc`. **On a Mac the list is Core Audio's own properties** —
`kAudioHardwarePropertyDevices`, then each device's input streams, name and UID — and the
element is `osxaudiosrc unique-id=<UID>`, which picks the device whose UID matches before the
default. The UID persists across boots, so it is what a saved `Device::Pulse` holds there.
GStreamer's device monitor is never asked for audio on a Mac: its `osxaudio` provider binds an
AudioUnit to each microphone to read its formats, which opens it, and from a session that could
not show the permission prompt that probe never returned — and a listing that never returns
leaves the lists empty and nothing else, since it runs on a thread of its own. **On Windows the
list is WASAPI's**, through `wasapi2deviceprovider`, and the element `wasapi2src device=<ID>` on
the endpoint ID, which persists across boots; the other providers that list the same endpoints
again are left out.

### The loopback

*System audio (what you hear)* captures the default output's monitor — what OBS calls Desktop
Audio. cpal cannot: its Linux host is ALSA and ALSA does not expose a PipeWire monitor. So
`Capture` has a second backend, a `pulsesrc ! audioconvert ! appsink` pipeline pinned to mono
at `track::RATE`, handing blocks to the same analyzer the cpal callback does. The device name
`@DEFAULT_MONITOR@` is resolved by the server on connect, so the loopback **follows** the
default output rather than pinning whichever one was default when the panel was drawn.

**On a Mac the loopback is a Core Audio process tap**, which macOS 14.2 and later offer, and
there is no GStreamer in it. `platform::macos::audio::Tap` describes a stereo tap over every
process (`CATapDescription`'s global tap excluding none), private and unmuted, makes it
(`AudioHardwareCreateProcessTap`), and puts it in a private aggregate device of its own whose
only input is the tap. An IO proc on that device runs on Core Audio's real-time thread, mixes
each cycle's buffers to mono in a block grown once, and hands it to the same analyzer closure
cpal's callback feeds, at the device's own rate. Dropping the capture stops and destroys the
proc, then destroys the aggregate device and the tap. `platform::audio::Tap::open` answers the
tap for `@DEFAULT_MONITOR@` and nothing for any other name, which GStreamer opens, and on Linux
it answers nothing at all; `audio::Stream::Tap` holds it open.

**On Windows the loopback is WASAPI's**, an output endpoint read backwards, through GStreamer
as on Linux: `@DEFAULT_MONITOR@` is `wasapi2src loopback=true`, the default output, which
follows the default when it changes, and every output is listed beside the inputs as a
loopback of its own, named as PulseAudio names a monitor, its endpoint ID with `.monitor`
after it. `Tap::open` answers nothing there too.

Two things differ from Linux on a Mac. **The tap hears everything every process plays, to any output**,
where a PulseAudio monitor is what goes to the default output. **It includes supersilvia**, as
the monitor does on Linux, so monitoring the loopback can feed back on both. It needs only
macOS's audio-capture permission, asked with `NSAudioCaptureUsageDescription` the first time a
tap starts; ScreenCaptureKit's system audio would have needed a stream over a display, and so
the screen's permission or its picker. The aggregate device holds no sub-device, which would
add that device's own inputs beside the tap's, and does not set `tapautostart`, under which
starting the device waits for the first tapped sound on the synth's thread.

### Screen capture

On Wayland an application cannot read the screen; it asks xdg-desktop-portal, the desktop shows
its own picker, and back comes a PipeWire remote — one descriptor and one node. `platform/linux/screen.rs`
does that conversation on a thread with a small tokio runtime, the shape `app/files.rs` uses
for file dialogs, and `Cast` holds the session open for as long as the pipeline reads it:
dropping it is how a capture stops and how the desktop's sharing indicator goes down.

**Every portal call in the process shares one D-Bus connection**, because `ashpd` caches it in
a `OnceLock` — the file dialogs' and the screen cast's alike. Its socket is pumped by a task on
whichever tokio runtime made it, so that runtime has to outlive every later call and nothing
may block it. `platform/linux/portal.rs` is the one runtime that satisfies both, and getting this wrong
does not fail at the call site: the connection stops being serviced, a running capture dies a
minute later, the next portal call never answers, and everything downstream goes black.

**The portal's restore token is thrown away and it always asks.** The token would resume the
same pick with no dialog, which is the rig persisting by another route. It also must never be
written back into `VideoSource`: that enum is compared against the last one to decide whether
to reopen, so a token stored in it is a *change*, and the next frame tore the session down and
put the picker up again, for ever.

**On a Mac the picker is ScreenCaptureKit's own**, `SCContentSharingPicker`, and the capture
has no pipeline. `platform/macos/screen.rs` sets the shared picker up on the main queue and
returns a `Pending` at once, as Linux does; the person chooses a display or a window, and the
observer's answer — on whichever thread the system calls it — builds an `SCStream` over the
choice right there, since the filter it is handed cannot cross threads, and sends back a `Cast`
that holds only the two slots and the stream's stop. Each answer, a choice or a cancel, is the
oldest ask's. **The picker allows a stream for every ask waiting and every stream it started
that is still running**: its `maximumStreamCount` is one unless set, and while that many of its
streams run, `present` shows nothing until one stops, so the Main Input capturing a window left
a Screen Capture node waiting on a picker that never came. The count is set before each
`present` and falls as each `Cast` stops its stream, and the picker stays active while it is
above zero, since the streams it started are what it manages. The stream delivers
`32BGRA` at the content's own size in pixels, the pointer drawn in, on a serial queue of its
own; each sample's `CVPixelBuffer` is published as its `IOSurface`, which the renderer samples
where it lies (see [Frames without a copy](#frames-without-a-copy)), beside the same buffer
locked read-only as bytes, as a camera's buffer is held, and unlocked when the frame drops. A
sample with no picture — the screen did not change — is skipped. **The frame's fourth byte is
not alpha**: ScreenCaptureKit leaves what is outside a window clear, so the frame is a `Bgrx`
layout and the conversion pass writes it opaque. That is read in `SCStream.h`, where the
stream's background colour is clear by default, and not on a captured screen. **The menu
bar's *Stop sharing* ends the stream, and the end reaches the status line two ways**, each
writing *the source stopped* where the camera's `error` reads a pipeline's end of stream: the
delegate's `stream:didStopWithError:`, whose error is taken as optional because a nil one has
been seen despite the header, and a sample whose `SCStreamFrameInfoStatus` is
`SCFrameStatusStopped`. A stop the system caused rather than the person carries its reason
after the line. Dropping the `Cast`
stops the stream on the main queue and lets it go once its sample queue has drained. So
`Stream::head` answers an element on Linux and the slots on the Mac, and a `Camera` over the
slots builds no pipeline and only reads them; the panel and the node do not know which.

**On Windows there is no picker**: Windows lets any application read the screen and has no
desktop dialog for choosing one, so `ask`'s `Pending` answers on its first poll with the
primary monitor, `d3d11screencapturesrc show-cursor=true ! d3d11download` — Direct3D 11's
desktop duplication, the pointer drawn in as Linux asks the portal for it, copied into memory
for the same bytes chain a camera's frames take. Another monitor or a window is not offered.

## Audio

`src/audio/` owns the device. cpal calls back with each block, the block is mixed to mono,
pushed through the analyzer, and the result published. The callback never allocates after
start-up.

The analysis is a 1024-point real FFT, which is 21 ms at 48 kHz and 47 Hz a bin: enough to
tell a kick from a bass line, and short enough not to be a smoothing in itself. It runs every
`HOP` samples — a quarter of the window, 5.3 ms, about 187 times a second — and **at a fixed
hop rather than once per delivered block**, so the rate does not depend on how a device
chunks its callbacks, so a microphone and a decoded file analyze identically. A DC blocker with
a corner near 25 Hz removes the offset a cheap microphone carries, which would otherwise show
up as a constant level.

`Analysis` carries the time it was written, so a consumer can ask how old the number it is
looking at actually is rather than assuming.

### Three bands

A band is a **center frequency and a Q**, not a fixed span: bass at 100 Hz, mid at 1 kHz,
high at 8 kHz, each spanning `freq / q` around its center. The bins under it are averaged with
logarithmic weighting — a band an octave wide holds far more high bins than low ones, and a
flat mean would let the top of it speak for all of it — measured as a fraction of a seventy
decibel window, and shaped by a power curve.

That is the whole of it: what a band publishes is its own level. **Shaping the number is the
graph's job.** `slew` smooths any uniform number, and an exciter — a departure from a running
median, expanded and soft-clipped, which is what turns "the bass is loud" into "a kick just
landed" — is a node's worth of work that can then be metered, patched and put anywhere. Built
into the analyzer it could be none of those, and every source paid for it whether or not it was
wanted. See [decisions.md](decisions.md#band-shaping-belongs-to-the-graph).

The constants are silvia's, ported deliberately because they are tuned. The **decibel window
is not**: silvia's -100..-30 assumes Web Audio's bin scaling, and `Analyzer::magnitude`
normalizes so a full-scale sine on a bin reads exactly one. Keeping silvia's ceiling pinned
every loud band at one, and a band already at one cannot report that anything happened.

Bands are not isolated and cannot be: a window generous enough to hear quiet music hears a
full-scale tone's leakage two decades away. What a band owes is dominance in its own range.

### Thresholds are crossed where the samples are

Each band, and the volume, has a level it fires at. Crossing up fires `Down` after a hold-off;
falling back fires `Up` with none, because a gate that will not close is worse than one that
closes early — an envelope downstream would be stuck open.

This happens **on the audio thread**, in `Analyzer::cross`, and each crossing carries the
sample it happened on. A block is a few milliseconds and a frame is sixteen, so a crossing
found where the samples are can be placed inside the frame that collects it; one found where
the pictures are is rounded to the frame that noticed. Since [an event carries when it
happened](cpu.md#the-event-half), the first is worth having — and it is the one thing about
this that silvia, checking its thresholds in an animation frame, could not do.

Crossings reach the synth thread inside `Analysis`, in a fixed ring with a running total: the
consumer remembers the total and takes the difference, so each is delivered exactly once and a
consumer that fell far enough behind to lose one is told. The levels go the other way through
atomics, because the callback may not block.

**Why this shape rather than silvia's.** silvia used the Web Audio `AnalyzerNode`, which
smooths over several frames by design and was polled from one animation loop for another to
consume. A transient reached a shader 50 to 100 ms late, and nothing measured that. Here the
only smoothing is what the graph asks for, and the age is a number.

### The scope on the node

Every audio source draws one, and it is two things stacked.

**The spectrum**, on a logarithmic frequency axis, with the bins under each band tinted that
band's color, and a **handle** on it whose X is where the band listens and whose Y is how
narrowly. The spectrum is bucketed onto that same log axis before it is published — not the
FFT's own bins, which are a fixed number of hertz wide and would collapse the three decades
holding all the music into the first column or two.

One drag says both: **X is where the band listens and Y is how narrowly**, Q running down the
plot on a log axis — narrow at the top, wide at the bottom, which is how a filter is drawn.
The tint under the handle is the feedback, since the columns a band covers are colored as it
moves. Two axes and no third: a band's own gain and its shaping are the graph's, so the
handle has exactly the two values a band has.

The values behind it are `NodeDef::hidden` controls: stored on the node, saved with the file,
undoable, and with no port and no row. Six number fields would say the same thing without
showing any of it, and the node is already tall. Dragging one writes both keys as **one
command**, so the whole drag is one undo step rather than two a frame.

**The clip is drawn at the foot of the node behind its `Preview` tick**, with the player's
own strip along it — a scrubber that repositions the clip by writing its Offset, so its Time
plays on from where it landed, a speaker that mutes the `monitor` control, and a volume for it. See
[ui.md](ui.md#the-strip-on-a-picture).

**How much of all this is drawn is the node's own**, as silvia's `Uniforms`, `Events` and
`Scope`, with `Preview` on the end of `video`'s — the first two ticks sharing one row and the
last two the headings over the regions they open. `Uniforms` and `Events` hide the output rows
that are uniform numbers and events, which is what a graph wants once it is wired and tuned;
`Scope` closes the scope region and keeps every row, which is what it wants while it is being
wired; and they are independent, so "the events and the scope, but
not the uniforms" is sayable. Each is an ordinary option — saved, undoable, one click one undo
step. Layout reads them off the `Node`, so a node's height is still decided without consulting
the registry, and hiding an output row keeps its cables exactly as collapsing a node does — a
hidden output that carries a cable gathers on the header's right edge, and the cable is drawn
from there; one that carries none has no dot at all. Hiding uniforms and events needs no new
bit on a port, either: a uniform output is a `UniformNumber` and an event output is an
`Action`, and layout can already see both.

**A meter per band** below it: a dot in the band's color, a bar, and the threshold on it as a
twelve-pixel rounded square in the action port's own color. That square is silvia's best idea
in this area — the control that sets the level and the port that emits the event are one
object sitting on the data they measure, and then there is nothing left to explain. It is dim
while parked at one, where nothing can reach it, and bright while the gate it opens is open.

The bar fills with what the graph sees, which is the number the threshold on it is compared
against — the meter and the trigger read the same value, so where the square sits is exactly
where it fires. Like the band handle, the threshold is a `NodeDef::hidden` control — no port,
no row — because a number row beside the square would only say what the square already shows.
`volumeLevel`, the microphone's own fourth threshold, has no meter and stays a port.

Band colors are literal — `theme::Theme::band` is red, green and blue — and they are the one
place in the editor a color does not derive from the four anchors. Low-mid-high as
red-green-blue is a convention, and a convention is what makes a dot legible without a
label; a themed triple would rotate with the anchors and stop meaning anything.

### The oscilloscope

Every audio source publishes its waveform as a `VaryingColor` output: a 512x1 texture and eleven
lines of WGSL that sample it and draw a line, silvia's. The primitive underneath is worth more
than the waveform — **a CPU array as a one-dimensional lookup texture a color output samples**
— and a palette, an automation curve, a sequencer lane and an LFO table all want it.

It is also the node that made a texture output stop being addressed by its node: `video`
publishes its picture *and* its waveform, and until then no node had two. See
[nodes.md](nodes.md).

### Monitoring

Every audio source has a `monitor` control: **zero is off and anything above it is the level**.
One row rather than a switch and a fader, on nodes that are already tall, and at zero nothing
is queued and no output device is opened — a graph that is not monitoring costs nothing.

Until this existed a `video` node made **no sound at all**. The cache is a picture format, the
soundtrack is decoded for analysis and never played, and nothing opened an output. silvia is
not better designed here — its analyzer explicitly refuses to connect to the destination — it
simply gets sound free from the `<video>` element it analyzes, and we gave that up when we
replaced the element with a cache and a sample buffer.

**The monitor plays exactly the samples the analyzer consumed.** `Reader` already reads the
span of audio each frame covers; handing the same slice to the output is the whole
implementation, and the behavior falls out of it rather than being written: at 2x a frame
reads two seconds and plays them in one, so it pitches like tape; a clip played backwards
reads the span backwards; scrubbing reads scattered windows, which is the sound of scrubbing
tape because it is the same operation; and a jump reads one window, so a cut sounds like a
cut. A second playback pipeline chasing the picture would have to be argued into every one of
those.

One device, shared. Each source owns a channel holding samples at the device's rate, resampled
on the way in by linear interpolation carrying its phase across blocks. The mix sums every
channel at its volume and clamps, because two loud sources sum past full scale and wrapping is
a bang. The queue is a `Mutex<VecDeque>` rather than a lock-free ring, because a lock-free ring
means `unsafe` and `unsafe` lives in `render/`; the output callback uses `try_lock` and plays
silence rather than waiting, so contention is a click and never a stall.

**A channel lives exactly as long as its owner.** A `video` node opens one when it is made, an
`audioin` node's capture and the Main Input's own capture each open one with a clone handed to
the thread that delivers their samples, and the Main Input opens one for a sound file it plays.
The last handle onto a channel to be dropped — the node's, the capture's, or the audio
thread's clone, whichever goes last — takes it out of the mix, and the Main Input drops its
file channel whenever its audio source changes. So a node deleted, a capture closed or a source
switched leaves no channel behind, and the output callback walks only the channels something
still holds.

The microphone monitors too, and it is the one that can howl — which is why zero is the
default everywhere rather than only here. The device is the system default; by [the tier
test](architecture.md#three-tiers-of-saved-state) a chosen one would be project data, and
nothing chooses one.

### The bundle

Every audio-capable source publishes the same ports — `bass`, `mid`, `high` as uniform numbers
and `bassEvent`, `midEvent`, `highEvent` as actions, plus `volumeEvent` on the microphone — so
a graph built against the microphone plays against a video file unchanged. That
interchangeability is worth more than any one of the nodes that has it, and it is why `audioin`
publishes `bass` rather than `low`.

## Cameras

One GStreamer pipeline per camera, ending in an `appsink` with `sync=false`,
`max-buffers=1` and `drop=true`. Those three settings are the policy: do not pace to the
clock, keep one frame, and throw away anything the synth thread did not collect.

**A camera is opened once, however many read it.** A Camera node and the Main Input on the
same camera, or two Camera nodes, read the one pipeline running on it: the first reader opens
it, a later one joins it with a slot of its own that the sink writes each frame into, and the
last to let go stops it. `Auto` is resolved to the camera it means first, so *Auto* and the
same camera chosen by name are one camera. A reader that asks for no size — a Camera node at
Size *Auto*, and the Main Input — takes whatever size the camera was opened at; one that asks
for a different size is refused, and a Camera node
says so on its status line — *FaceTime HD Camera is open elsewhere at 1280x720: set Size to
1280x720 or Auto to share it* — rather than showing a size its menu does not say. Opening a
camera twice is wrong on both machines, differently: V4L2 refuses the second capture as busy,
and AVFoundation starts a second session that sets the device's format for both, so the first
pipeline goes on describing frames by caps that no longer fit them — a crop of a bigger frame,
or a frame that fails to map. A test pattern and a screen are each reader's own.

**Auto opens a camera at 1920x1080, then 1280x720, where the camera offers one**, and at the
first size the camera lists otherwise. What a camera offers is the caps the device monitor
lists for it over `Video/Source` — raw or MJPEG, in system memory — read without opening the
camera, on Linux and on a Mac alike. The size chosen is asked for in the pipeline's caps
filter, with format and framerate left to the camera, and it is the pipeline's size: a later
*Auto* reader shares it, and a reader asking for another is refused as above.

**A device listing on Linux can print `GStreamer-CRITICAL … gst_value_collect_int_range`**, a
burst of about 34 at once, and nothing is wrong with the app. PipeWire's own GStreamer plugin
(`libgstpipewire`), on PipeWire's thread, turns every node's formats into caps while any
listing — the microphones', the cameras' — is started, and a node that takes any size up to
`u32::MAX` becomes an int range of 1 to -1, which GStreamer refuses. On KDE that node is
Plasma's own consumer of a screen cast (`plasma-screencast-node-<n>`, a live window
thumbnail), so the burst comes and goes with Plasma, not with anything the app does. Seen with
PipeWire's plugin on Fedora 44, 28 September 2026.

**Nothing converts a camera's frame on the CPU, and nothing copies it before the upload.** The
sink asks for any of the formats the renderer uploads as they are — RGB in four byte orders,
and YUV as three planes (I420, YV12, Y42B, Y444), as NV12, or packed (YUY2, UYVY) — so the
`videoconvert` in front of it passes a webcam's YUY2 or NV12 through untouched and converts
only a format outside that list. The callback maps the buffer and wraps it, mapped, in an
`Arc` (`Pixels::Mapped`): the device's own memory at the device's own stride, held for as long
as the frame is. `tick` takes the newest and hands it to the renderer, which uploads it and
**skips an `Arc` it has already seen**, so holding the same frame across several rendered
frames costs one upload, not one per frame. How the upload goes is in
[rendering.md](rendering.md#source-textures): one copy into a pixel-unpack buffer on the
synth thread, and the YUV-to-RGB conversion as a draw.

**An MJPEG webcam decodes on the video engine.** A webcam sends 720p and up as JPEG far more
often than raw, and `jpegdec` decodes every frame on one CPU thread. `vajpegdec`'s rank is
none, so decodebin would never pick it; the camera's own decodebin puts it at the head of the
decoder list once `jpegparse` has described the stream (`prefer_va_jpeg`), with `jpegdec`
behind it. Raising the rank instead would reach every decodebin in the process, a poster of a
progressive JPEG the engine cannot decode among them. What comes out is the decoder's own
planes in system memory, which take the same path as a raw camera's.

**The handoff is a one-frame slot, not a triple buffer.** A held frame is a buffer the source
cannot reuse, and a triple buffer keeps two stale frames alive in its spare slots — a screen
cast whose compositor lends three buffers then stops for good, since the frame that would
release them can never be drawn. The slot holds only the newest unread frame. The sink writes
it under a lock held for a pointer's swap; `latest` only ever *tries* the lock, and a sink
mid-write is "nothing new" until the next tick. The renderer gives back a frame it imported
once the GPU has finished the draws that sampled it, whether or not another frame has
arrived, so what a source lends comes back at the pace of the GPU and never waits on the
source itself.

**Which camera is a platform question.** On Linux a camera is `v4l2src` on a V4L2 node, and
the node saves its path. The device monitor lists it through PipeWire's provider where that is
installed, which hides V4L2's own; the path is PipeWire's `api.v4l2.path` or V4L2's
`device.path`, and the name is the card's. On a Mac it is `avfvideosrc device-index=<n>`, and the node saves
AVFoundation's unique ID, so a project reopens on the same camera whatever else is plugged in
([decisions.md](decisions.md#on-a-mac-a-saved-camera-node-names-the-camera-itself)). The
device monitor over `Video/Source` lists the Mac's cameras through `avfdeviceprovider`, opening
none of them; `avfvideosrc` has no unique-ID property, so the ID is matched in the listing and
the index read off the element the device builds. `vtdec_hw` already outranks `jpegdec`, and
`avfvideosrc` only offers raw NV12, UYVY, YUY2, ARGB and BGRA, all but ARGB uploaded as they
are. **The camera is held to frames in memory**: `avfvideosrc` lists its UYVY and YUY2 as GL
rectangle textures ahead of everything in memory and fixates on the first structure its peer
accepts, and with no size asked for the peer is `decodebin`, which accepts anything — so left
to itself the camera settles on a texture no `videoconvert` can take and stops *not-linked*
before its first frame. The source is `avfvideosrc device-index=<n> ! video/x-raw`. The FaceTime
HD camera lists a 1552x1552 square first, then 1328x1760, 640x480, 1760x1328, 1080x1920,
1280x720 and 1920x1080, so *Auto* opens it at 1920x1080.
On Windows a camera is `mfvideosrc device-path=<path>`, Media Foundation's, and the node saves
the device's symbolic link, which Windows keeps across reboots; the device monitor lists it
through `mfdeviceprovider`, opening none, and the DirectShow and kernel-streaming providers that
list the same cameras again are left out.
**The Camera node's menu on a Mac and on Windows is the cameras by name**, between *Auto* and the test
pattern. It is the list the Main Input's listing made on its own thread, once at start and
again on *Look for devices again* — at the foot of this menu as of the panel's — so drawing
the menu never asks the machine.

The pipeline is opened on the first tick and rebuilt whenever the options change. Before the
first frame arrives the node publishes a 2x2 black placeholder, so a shader sampling it has
a real texture rather than a missing uniform. **A camera that delivers nothing for ten
seconds is not responding** — before its first frame or after its last, counted on the wall
clock from the open by `video::silence` — and its `error` says so, which is the flag on its
header ([ui.md](ui.md#problems)): a camera warms up in a second or two, and one wedged on its
first stream looked, until then, exactly like one warming up.

A camera's texture output is **delayed**, like an Output's `frame` port: a consumer samples
what the camera last published rather than descending into it. That is what makes a loop
through a camera legal.

### Screen capture as a node

The section above is the rig's one capture, on the panel. **`screencapture` is a node beside
`camera`**, and each instance holds a portal session of its own: it asks on the first tick,
which is the node opening, and gives the session up when the node is deleted. That is what a
second window costs and what it buys — two nodes are two pickers and two captures, and a mix
can hold one window in one corner and another somewhere else.

None of the portal code is new. [`crate::platform::screen`] is a module rather than a
method on the panel, so the node calls the same `ask`, polls the same `Pending` and holds the
same `Cast`; below the descriptor it is a `Camera` over `pipewiresrc`, exactly as the panel's
is — delivering DMA-BUFs where it can, as [Frames without a copy](#frames-without-a-copy)
describes — and on a Mac a `Camera` over the stream's own slots. Two nodes on a Mac are two
asks of the one system picker, answered in the order they asked: the picker comes back for
the second as soon as the first is answered, and it comes up while the panel's own capture
runs, since the picker allows a stream for each capture running and each ask. The rules above hold: one runtime for the process, the session outliving the
pipeline, and the restore token thrown away — **a node asks every run too**.

The node says which of three things it is doing on its status line: waiting for the picker,
capturing, or stopped. *Choose Screen* asks again and *Stop* ends the session, and both are
`Action` inputs with a button on them, so a sequencer can cut a window in or out. A refusal,
a missing `pipewiresrc` and a descriptor that went stale are all the status line and a black
frame; none of them is a panic.

`cargo test` never asks. The node opens the portal on its first tick, so `tests/reset.rs`
names it beside the camera and the microphone as a node it does not instantiate, and nothing
else in the suite ticks every node in the registry. `doctor.sh` asks the portal nothing.

## Syphon

On a Mac, a picture **another app publishes over Syphon** — Resolume's *Composition*, VDMX's
outputs, MadMapper, a Syphon test app — is a video source: the Main Input's *Syphon*, one for
the rig, and a **`syphon` node** per server a patch wants, as `screencapture` is beside the
panel's screen. Publishing is [rendering.md](rendering.md#syphon)'s; this is receiving.
`proposals/syphon.md` is the argument.

**A server is chosen by its label**, "App – Server", from the Mac's Syphon directory as it
stands: the node's `Server` menu and the panel's server menu both list what is running now,
read at most twice a second (`video::syphon::labels`, and `menu` for the node's `found`). The
label is what a project saves, because the directory's own ID is new on every run of the other
app. **A server that is not running is waited for**: a `video::syphon::Receiver` looks for its
label every second, connects when it appears, notices within a second when it stops — its
client's `isValid` — keeps the last frame up, says so, and connects again when it comes back.

**Frames need no pipeline.** A client (`platform::syphon::Inlet`, over the framework's
`SyphonClientBase`) is handed the server's `IOSurface` on each new frame, on a thread of its
own, and writes it as a `Pixels::IoSurface` into the one-frame slot a `Camera` adopts, as a
screen's frames are on a Mac (`Source::Syphon`). **Each frame is copied into a texture of
ours**: the server draws into the same surface again for its next frame, so sampling it where
it lies could show a frame half drawn. The frame is marked `nodes::Redrawn`, and the renderer
draws it through [the conversion pass](rendering.md#source-textures) into the source's own
texture — one GPU copy a frame, on the synth's prelude, never waiting. A surface whose pixel
format is zero, as frameworks built before October 2025 leave it, is read as BGRA.

**Two ticks**, on the node and beside the panel's server menu: **Flip** reads the surface top
row first, where Syphon's convention — and the default — is bottom row first; **Transparent**
keeps the surface's alpha, where the default reads it opaque, the fourth byte written one by
the copy. A server that publishes the other way up, or with alpha a patch wants, ticks its way
out.

**Linux has no Syphon, and shows none of it** (`platform::syphon::available`): the Main
Input's list does not offer it, the Nodes menu and the browser do not offer the node, and an
Output has no Syphon row ([ui.md](ui.md#syphon)). A Mac's project still opens whole: a panel
that chose a server reads *Syphon (macOS only)*, its line says *Syphon is macOS's*, and a
`syphon` node lists nothing and says the same. `tests/syphon.rs` receives a server of the process's
own through all of this — the directory, the client, the camera's slot and the renderer's copy
— the right way up, opaque and with alpha; `tests/gpu_iosurface.rs` holds the copy, the flip
and the pixel format of zero on a surface made by hand.

## NDI

**NDI®** sends live video and sound between machines on the local network — Vizrt's, at
[ndi.video](https://ndi.video) — where Syphon links apps on one Mac. `proposals/ndi.md` is the
argument and the decisions.

**The plugin is compiled in; the runtime is the user's.** GStreamer's NDI plugin,
`gst-plugin-ndi` (MPL-2.0, at our `gstreamer` 0.25), is a Cargo dependency, and `video::ndi::
register` hands it to GStreamer as a static plugin at start-up, before any device monitor
runs, so its elements replace any `ndi` plugin the system's GStreamer carries and no
distribution has to package it. The plugin opens the proprietary NDI runtime, `libndi`, with
`libloading` only when an element starts — `libndi.dylib` on a Mac, `libndi.so.6` or `.5` on
Linux, from `NDI_RUNTIME_DIR_V6` or `NDI_RUNTIME_DIR_V5` where set, else by bare name through
the system's loader — so nothing proprietary is linked, building needs no NDI SDK, and the user
installs NDI's free runtime from ndi.video and accepts its licence themselves. `LICENSE` opens
with the AGPL section 7 additional permission for linking with and loading it. Only plain NDI:
the plugin's `advanced-sdk` feature, NDI|HX's, is off.

**The app finds the runtime before the plugin does.** Every path that starts an NDI element —
the probe, a sender, a receiver, the source listing — first has `platform::ndi::preload` open
the runtime: in the variables' folders, then by bare name, as the plugin would, then by its
full path in the system's library folders (`/usr/lib`, `/usr/local/lib`, `/usr/lib64` and
Debian's two), holding the first that opens for the life of the process. On Linux that is what
makes a runtime the loader does not search load at all: Fedora's `ld.so` does not search
`/usr/local/lib`, where NDI's installer puts `libndi.so.6`, and glibc hands the plugin's later
bare-name open the object already loaded under that SONAME. So on Linux the runtime is
installed by putting `libndi.so.6` in `/usr/local/lib` or `/usr/lib64`, with no variable to
set; the app sets none either. `libloading`'s open, in `platform/linux/ndi.rs`, is the one
`unsafe` in `platform/linux/`. On a Mac nothing is opened ahead of the plugin: dyld's fallback
already searches `/usr/local/lib`, where the NDI 6 Runtime's installer puts `libndi.dylib`, nor
on Windows, where the plugin opens `Processing.NDI.Lib.x64.dll` from the folder
`NDI_RUNTIME_DIR_V6` names, which NDI's installer sets.

**Whether the runtime is there is asked once**, on a thread of its own at start-up
(`video::ndi::start`), by starting an `ndisink` in a pipeline of its own as far as the plugin's
own loading, after the preload. Where there is none, every place that offers NDI says *The
NDI® runtime is not installed — get it at ndi.video* (`video::ndi::MISSING`) and where it
looked — the folders the two variables name, where set, then the system's: `/usr/local/lib` and
`/usr/lib` on a Mac, where NDI's installers put `libndi.dylib`, NDI 6 Runtime's own folder in
`Program Files` on Windows, and the usual library folders on Linux. Where a runtime is in one of them and still would not load, it says so by its path, with
the loader's reason or the one the plugin posted, and `video::ndi::absent` tells that apart
from none at all, which an Output's row says as *runtime won't load*. Nothing asks again: the plugin keeps its first answer
for the life of the process, so a runtime installed while the app runs is found on the next
run. **The device provider is ranked out of every device monitor's reach**: a camera's
listing asks for `Video/Source`, which the provider's `Source/Audio/Video/Network` also
answers, and starting it would start NDI's discovery on the network for nobody.

**Receiving** is the Main Input's *NDI* video source, one for the rig, and an **`ndi` node**
per source a patch wants, as the `syphon` node is beside the panel's Syphon server; sending is
[rendering.md](rendering.md#ndi)'s. **A source is chosen by the name NDI gives it**,
`MACHINE (Stream)`, from a menu of what the network has now: the device provider, started by
name the first time a menu asks — the panel asks only while its source is *NDI*, so nobody who
never chose NDI starts discovery — and read at most twice a second (`video::ndi::labels`, and
`menu` for the node's `found`). The name is what a project saves, and **it is kept on Open**,
as a Syphon server is, since it opens no device.

**A received source is a camera** (`Source::Ndi`): `ndisrc ndi-name=… ! ndisrcdemux`, its
video pad into the camera's own appsink, capped to what the renderer uploads, with `sync=false`,
`max-buffers=1` and `drop=true`. `ndisrc` hands UYVY for a source with no alpha and BGRA for
one with it, both uploaded as they are as `Pixels::Mapped`, top row first, and a Main Input and
a node on the same source share one pipeline, keyed apart from every device. A
`video::ndi::Receiver` keeps it open: it opens a source when the listing has it, or every ten
seconds whether or not it does, asks each second whether its pipeline has ended — `ndisrc` ends
the stream after five seconds without a frame — and then **keeps the last frame up, says the
source has gone, and opens it again when it is listed again**. **Transparent** keeps a BGRA
source's alpha; by default it is read opaque, BGRA read as BGRx.

**Its sound reaches the Main Input's analyzer** through the *Video source* audio choice: a
receiver of its own on the same source that asks NDI for sound alone (`bandwidth=10`) and waits
for it however long it is gone, its demuxer's audio pad into the same GStreamer capture a named
microphone uses (`audio::Capture::open_ndi`).

`tests/ndi.rs` is the loopback: a four-quadrant picture sent through `appsrc ! ndisink` and
received with `ndisrc ! ndisrcdemux` in the same process, found by the plugin's device provider
under the name NDI gives it, `MACHINE (stream)`. It holds that a picture sent top row first
comes back top row first, that one sent as BGRA comes back as BGRA with its alpha and one sent
opaque comes back as UYVY — `ndisrc`'s default — and prints how long a frame takes to come
back. **Without the runtime it skips, saying so**, which is what it does on every machine
nobody has installed it on. With it, it needs NDI's discovery, which on Linux is Avahi over the
system bus, or the listing times out. A distrobox shares only the host's session bus, so
`distrobox.ini` links the box's `/run/dbus/system_bus_socket` to the host's.

NDI® is a registered trademark of Vizrt NDI AB.

## Words as a picture

`text` is the one node that draws a letter, and it draws it with what is already in the box:
**pango**, through GStreamer's own `textoverlay`. `nodes/` may take no graphical dependency
and a font rasterizer is one, so the letters are rendered by a pipeline like any other source's
— `videotestsrc pattern=black num-buffers=1 ! textoverlay ! videoconvert ! appsink` — and the
frame comes back through a triple buffer, as packed RGBA (`Delivery::Rgba`) because the tests
read its ink on the CPU and it is drawn once.

**One frame per string.** The pipeline carries `num-buffers=1`: it renders once and goes to
end of stream, the node keeps the `Arc` it produced, and every tick after that republishes the
same one — which the renderer skips, having seen it. A still string costs one upload in its
life. A keystroke, a font, a size or an alignment builds a new pipeline; nothing rebuilds a
shader, because the words are a value and the colors are ports.

**It is `textoverlay` rather than `textrender`,** which are one plugin and one rasterizer.
`textrender` sizes its output to the text it was handed, which would make Size a resolution
rather than a size — a word would fill the frame whatever it said. `textoverlay` draws into a
frame of a stated size, which is silvia's canvas with `fillText` on it, so Size, Align and
Baseline mean here what they mean there. Size goes into the pango description in **pixels**,
`64px`, for the same reason.

The fonts are the machine's. The Font menu is every family the machine lists, by its first
name, after pango's three generics — read once, the first time the menu is drawn, through the
same library pango finds a family through, so a name on the menu is one it will draw: the
`fontconfig` crate on Linux, on macOS AppKit's `NSFontCollection`, which reads Core Text, and
on Windows DirectWrite's system font collection, each family by its English name where it has
one.
silvia's twenty faces stay declared as the node's choices: the default's home, and the menu on
a machine that cannot list its fonts. The family goes into the pango description closed by a
comma, `Times New Roman, Normal 64px`, since without it pango reads a last word such as
`Roman` or `Condensed` as a style and draws the fallback. A family that is not installed
falls back to the nearest thing there is, which is what a browser did for silvia's own
comma-separated stacks, and a project naming one still loads. So a project opened on
another machine may set the words in a different face — true of silvia too.

## Video files

A delivery file has one keyframe every few seconds and every other frame is a diff, so
showing frame *n* means decoding from the last keyframe forward. That is why scrubbing,
reverse and speed changes fall apart in a browser, and it is a property of the file rather
than of the player.

So a clip is **transcoded once on import** into an all-intra stream: a group-of-pictures of
one, every frame a keyframe. From then on any frame is reachable at the same cost, and position becomes the
primitive. Speed, direction and scrubbing all fall out of "which frame do I want this tick".

**A clip is an oscillator whose shape is a frame lookup.** One cycle is one play of the clip,
and the node reads it as every node that moves with time does
([cpu.md](cpu.md#the-oscillator-the-sequencers-and-the-clips)). **Time** counts plays: unplugged
it is ambient time at the clip's native speed, `ctx.clock_at(id, 1 ÷ length)`, one play every
clip length, so a clip pauses with the show and a seek lands it where the playhead puts it; a gear cabled in
replaces it, so a Ratio Gear at ×2 plays it twice as fast, one at `-×1` backwards and one at ×0
holds it. **Offset** is added, 0 to 1 across the clip, and the Loop option wraps the sum or, at
Hold, clamps it to one play. The frame is `round(position × frames)` (`clip::frame_at`), so a
24 fps clip on a 60 Hz display judders 3:2, as any clip read at a position does, and the same
position is the same frame however it was reached: the node keeps no position of its own. A
slow wave on Offset scratches around the playing clip, and under a Ratio Gear at ×0 a cable on
Offset is the whole position.

**A render waits for its frame.** Live, the picture is the newest the worker delivered, so a
jump shows the old frame for a tick while the new one decodes. A render cannot afford that: it
asks `CpuNode::waiting` after each frame it steps, and while the frame a clip asked for has not
arrived it holds — the Output is not drawn, the transport does not move, and only the waiting
nodes tick again — until it has, or five seconds have passed. A clip rendered twice is the same
film to the byte, whatever live play left the decoder holding (`tests/gpu_app.rs`).

The transcode needs a hardware encoder and decoder pair. `src/video/clip.rs` probes for
the first installed triple of encoder, decoder and parser in the machine's `CODECS` — on Linux
H.264, HEVC, AV1 over VA-API, then the same three over NVENC, so an Intel part without H.264
encode lands on HEVC and an NVIDIA card on its own plugin; on a Mac H.264, then HEVC, over
VideoToolbox (`vtenc_h264_hw`, `vtenc_h265_hw`, and `vtdec_hw` for both); on Windows NVENC's
three under Linux's `nv-` names, then Quick Sync's, AMF's — decoded by Direct3D 11's decoders,
since AMF has none — and Direct3D 12's H.264, each registered by GStreamer only on a machine
whose GPU and driver offer it. `scripts/doctor.sh`
fails if no pair is present, because importing on the CPU is not a thing that finishes, and a
node's status line names what the machine lacks: VA-API or NVENC on Linux, VideoToolbox on a
Mac, NVENC, Quick Sync, AMF or Direct3D 12 on Windows.

**Every frame a keyframe, at constant quality.** VA-API is `key-int-max=1 rate-control=cqp`,
NVENC `gop-size=1 rc-mode=constqp`, VideoToolbox `max-keyframe-interval=1 quality=0.7`, and
Quick Sync, AMF and Direct3D 12 `gop-size=1 rate-control=cqp qp-i=26 qp-p=26`.
VideoToolbox has no constant-QP mode; its `quality`, with the bitrate left automatic, holds a
quality rather than a budget, and 0.7 lands where x264's QP 26 does. A test holds every
available codec's intra chain to a keyframe on every buffer.

**A Mac and a Linux box share cache entries.** The Mac's rows carry Linux's names, `h264` and
`h265`, and prefer H.264 as Linux does, so a project folder imported on one plays on the other
with no *Preparing clip…*. Both machines write standard streams: an all-intra High-profile
H.264 file of the kind Linux writes decodes on `vtdec_hw` bit for bit as ffmpeg decodes it, and
seeks exactly.

**The wait says how long it has been.** `CpuNode::status` reads
`Preparing clip… 40%  0:07` — the words for the wait, the encoder's own position in the file,
and a clock, because a first import of a long clip is minutes and a percentage cannot say
whether the rest is ten seconds or ten minutes away. `Transcode::elapsed` reads a start
recorded on the shared job rather than on the handle, so a second node joining an encode
already running reports the wait that is happening. The line is drawn across the middle of the
node's own **preview** band, which is where the eye already is; `progress` still fills the file
button's bar.

Transcoded clips live in the project's own `cache/`, beside `assets/` — in the folder so
that it plays on another machine without re-encoding, which
[decisions.md](decisions.md#assets-and-the-cache-live-inside-the-project) argues. It is
made on demand, Save never touches it, and it is listed nowhere: a cache, and deleting it
costs a re-import and nothing else.

**Everything derived from a source is named after that source.** A transcode's file name is
the source key *and* a hash of the size cap and the codec: importing the same file twice is
free, two machines that chose different codecs keep separate entries in one folder, and the
entries belonging to one asset can be found without knowing which codec made them — which is
what lets [removing an asset](architecture.md#assets) take its cache with it. An asset is
keyed by where it sits in the project rather than by an absolute path, which is what lets the
folder move; a file somebody typed a path to from outside is keyed by that path and its
modification time, since nothing here owns it.

Two nodes importing the same file share one encode; dropping the last of them cancels it and
removes the partial file.

**VideoToolbox loses frames at the edges of a stream, and the Mac works around it.**
GStreamer 1.28's `vtenc` and `vtdec` drain by waiting on VideoToolbox and then pausing their
output task from outside, so a frame handed back during the wait, or queued while the task has
not yet run, is stranded and thrown away; and a `vtdec` flush that lands mid-push leaves frames
from before a seek to come out after it. Under load that was a clip's, a delivery's or an
import's last frame missing, a seek to one of a clip's last frames coming back empty, and a seek
answered with another frame's picture under the wanted frame's timestamp. So on a Mac
`platform::video::settle_before_eos` holds the end of every codec pipeline's stream at each
VideoToolbox element until VideoToolbox has handed back all it was given, which is why the Mac's
delivery rows turn frame reordering off; `Player` names each decoded frame by a stamp put on
the compressed buffer it came from rather than by the decoder's output timestamp; and a seek or
a start that comes back short is tried again, the seek from 34 frames earlier so the output task
must run before the stream ends, after putting back the playing state a decoder's *No valid
frames* error leaves failed. Linux's codecs drain properly, and there none of this changes
anything.

### Rendering to a file

An offline render writes through `video/encode.rs`: the same hardware encoder the import
transcode probed, in a delivery's shape rather than the cache's — constant quality and the
encoder's own keyframe interval, since a render is a thing you send someone and nobody
scrubs it here. Frames go in one at a time as RGBA8 rows-top-first, each stamped with its
index over the frame rate, so the file's clock is the render's and not the wall's; the file
is written to a `.part` beside its name and renamed at the end, so a half-written render is
never mistaken for a clip. The Output's **Writer** select chooses this over the PNG sequence,
which stays the lossless hand-off to an editor. See
[rendering.md](rendering.md#the-render-job).

### A clip's soundtrack

**The cache holds no audio.** It is a picture format, and for a long time that was a bug
rather than a decision: `sink_other_streams` sent every non-video pad to a `fakesink` so
`decodebin` would not error, and nothing ever picked the audio back up, so a `video` node had
no soundtrack to analyze.

It is picked up now, from the **source** rather than from the cache. `audio::Track` decodes
the original once into mono `i16` at 48 kHz, written into the project's `cache/` beside the
transcode and read a few kilobytes at a time as the playhead needs them. It travels with the
folder for the same reason the transcode does, and is keyed the same way — on the file
alone, not on the size cap, since a resolution change does not change what a track sounds
like. Giving the cache an
audio track instead would mean re-encoding a soundtrack lossily on top of the source for
something only the analyzer reads.

The decode writes into a temporary named for itself and renames it over the track once whole,
so a `video` node and the Main Input decoding one clip at once both get the whole track. A
file with no audio stream ends the decode as soon as `decodebin` has exposed its last pad, as
an empty track, rather than leaving the pipeline waiting on an appsink nothing will reach; a
decode error ends it with no track written.

The samples are indexed by time rather than played, which is what makes the analysis follow
the picture wherever it goes: a scrub scrubs the sound, a clip played backwards reads
backwards, and
frame *n* analyzed twice gives the same numbers — which is what a render against an audio
track will need. A short forward move feeds every sample it passed over, so a crossing inside
it is dated by its own sample; a backwards or long move is a **jump**, which resets the
analyzer, reads one window at the destination, and reports no crossing, because arriving
somewhere is not the same as something happening there. The gates are closed by the jump too,
or a level already above its threshold at the destination would deliver an `Up` that nothing
had opened.

Measured on a 67-minute VHS transfer: 74 seconds to decode, 389 MB on disk, 0.016 ms to reopen
on the next launch, 64 µs for a random seek and 46 µs a frame playing forward — against a
frame budget of 16,700 µs. Holding those samples in memory instead would have cost 778 MB and
the 74 seconds *on every launch*, to save none of that.

## Images and GIFs

`imagegif` is the smallest source there is: an [`Asset` option](nodes.md#option-kinds) naming
a file, one texture output, a Time and an Offset. It takes png, jpg, jpeg, gif and webp — what
the `image` crate decodes with its default features — and everything about it is the shape
`video` already had, one size down.

**It draws the picture it is holding**, under the same **Preview** heading the clip node's
band is under and through the same [`nodes::Region::Preview`](ui.md#preview-and-on-node-render)
region — a fixed 16:9 box of the body's width, letterboxed. It is the node whose entire
content is one still picture, so a row of them reads as a contact sheet the way silvia's does.

**The decode is on a worker and the tick polls it.** A file is read the moment the option
points at one: a thread opens it, decodes it, and sends the frames back through an
`mpsc::channel` the tick drains with `try_recv`, exactly as a transcode's result arrives.
`status` says how many frames have landed while it is working, drawn across the middle of the
picture band, and it is a count rather than a fraction on purpose — **a GIF says nowhere in
its header how long it is**, so there is no total to be a fraction of. A decode
that fails reaches `error()` and the status line, and the node publishes a 2x2 black frame so
a shader sampling it has a real texture rather than a missing uniform.

**A GIF plays by its own delays.** Each frame carries the delay it was authored with, and the
node reads Time in plays of the whole animation, `video`'s rule: unplugged, ambient time at its
own pace, one play every length of its delays, so it pauses with the show and a seek lands it
where the playhead puts it; a gear cabled in replaces it, a Ratio Gear at ×2 twice as fast, one at `-×1`
backwards and one at ×0 holding it. **Offset is added**, in plays, 0 to 1 across the whole
animation laid over the delays, and the sum wraps as a GIF does. The frame shown is the one
whose delay the sum falls inside, so the same sum is the same frame however it was reached,
and the node keeps no playhead of its own. A frame may claim a delay of zero, which every
player clamps and so does this. A still is one frame with no delay, and Time and Offset do
nothing to it. `frame` and
`frames` are uniform numbers: where the playhead is, and how many there are.

**Its picture is bounded by count and by bytes**, 1024 frames or 256 MB, whichever comes
first, because a wall of frames is a wall of megabytes and nothing in the file says how many
are coming.

**Why an image crate, given `video/png.rs`.** PNG in and out is still GStreamer's, and
`decodebin` decodes a PNG, a JPEG and a WebP too. It cannot decode a GIF: no machine this has
run on has a gif loader behind `gdkpixbufdec`, whose caps list every other still format and
not that one, and there is no `avdec_gif` without the libav plugin — `decodebin` answers
`not-linked` and the pipeline never prerolls. So the one thing the crate is here for is the
one thing that cannot be got any other way. See
[decisions.md](decisions.md#the-image-crate-is-for-the-gif-and-nothing-else).

**Its poster is the asset machinery's, unchanged.** A PNG is its own picture and the card
draws the file; a JPEG and a WebP get a frame decoded out of them into `cache/` like any
other asset; a GIF gets no poster on a machine whose GStreamer cannot decode one, and keeps
the icon its extension gives it. See [Posters](#posters).

## A painting

`drawingcanvas` is the one source whose picture is made by a hand on the node, and the one whose
picture the project keeps: silvia forgot a painting on a reload, and here it is saved
([decisions.md](decisions.md#a-painting-is-saved-with-the-project)).

**In memory it is the node's own value**, `Value::Painting`: RGBA8, rows top first, behind an
`Arc`. The tick publishes that very frame as the node's `output` texture — nothing is copied,
and a tick that changed nothing publishes the same `Arc` and uploads nothing. A canvas nobody
has painted on publishes a blank of its background at the size Canvas Size names, and a painting
of another size is stretched to it, once, when the menu moves.

**On disk it is a PNG in the project's `assets/`**, beside the media a node was handed:

```text
friday/
  workspaces/
    tunnel.ssw         …"painting": { "Painting": { "file": "assets/painting-3f2a09d1c47be8c9.png" } }…
  assets/
    gumbasia.webm
    painting-3f2a09d1c47be8c9.png
```

The name is the picture's content — sixteen hex digits of an FNV-1a over its size and its bytes
— so a file is never written over: a stroke makes a new name, and a picture saved twice unchanged
is one file. The PNG is `video/png.rs`'s, GStreamer's `pngenc` and `pngdec`, and it carries the
alpha an eraser leaves and the color under it to the byte.

**The save writes it first.** `Project::save` writes every painting not already on disk into
`assets/` — beside itself, synced, renamed into place — before any workspace file, so the
manifest's rename commits a save whose pictures are already there, and a save cut off anywhere
opens as the last one whole. Once it has committed, it deletes the painting files the save
before it named and this one does not: the one thing under `assets/` a save deletes, because it
wrote them and the pixels are in memory — unless a node took one up as a file of its own, which
`project::asset_users` counts. Opening reads each one back into its node; one whose file is gone
opens blank with a warning and keeps its name.

**The autosave is the same save into `.autosave/`**, so an unsaved painting is
`.autosave/assets/painting-<print>.png`, read back when the autosave is recovered and written
into the project's own `assets/` by the next Save. An **export** writes the paintings into the
`assets/` beside the file, and an **import** reads them from there into the nodes, for the next
save to write into this project. The asset cards list a painting file like any picture, used by
the node that painted it.

## Frames without a copy

A decoder's frame is already in GPU memory. With bytes delivery the pipeline pulls it down,
converts it on the CPU, the sink copies it, and the synth thread uploads it: one download,
one conversion, one memcpy and one upload per frame, the last on the thread that must not
stall.

With DMA-BUF delivery `vapostproc` converts to RGBA on the video engine and exports the
result as a descriptor naming that memory. The sink publishes the descriptor with the
decoder's buffer held behind it; the renderer imports it on Vulkan as the source's texture,
through wgpu-hal, with a `dup` of the descriptor's fd; the shader samples the decoder's memory.
Nothing is copied, and the synth thread's cost per new frame is one import. `Player::open` asks for this where the machine
can export and import, and falls back to bytes where it cannot — NVIDIA's decoders export
through CUDA, not DMA-BUF — and says which in the overlay.

Two things make it work. The sink has to advertise `VideoMeta` in the allocation query,
because the stride and offset travel there and `vapostproc` refuses DMA-BUF caps to a sink
that has not; `advertise_video_meta` answers on appsink's behalf. And the export format is
read from `vapostproc`'s own pad template, because the modifier is the GPU's tiling and
asking for a linear layout would make the video engine untile every frame.

The descriptor is valid only while the decoder's buffer lives. The renderer holds the frame
its texture imported, and a frame it replaced until no viewer still holds a picture naming it
and the submission after that has finished — see [rendering.md](rendering.md#dma-buf-import). With a tick or
less on the GPU that is one or two buffers of a decoder pool of four; with the GPU further
behind the pool can run dry, and then the decoder waits for a buffer rather than writing into
one a draw is still reading. The renderer sees an opaque
keep-alive, never a GStreamer type.

**A screen cast is the compositor's own buffer.** The compositor already has the screen in
GPU memory, so there is nothing to convert and no `vapostproc`: `Camera::open` sets the
appsink's caps to `memory:DMABuf` in the RGB formats and tilings this machine's renderer
imports, then the bytes a camera takes, and `pipewiresrc` offers the compositor both. Which it
shares is the compositor's choice, and one that shares only memory is taken in memory with
nothing to retry. The list is asked of Vulkan on the renderer's device
(`render::dmabuf::importable_here`, through `platform::video::dmabuf_caps`): `XR24`,
`AR24`, `XB24` and `AB24`, each in the modifiers Vulkan reports as one plane and would import
for exactly the image the import makes. A compressed tiling carries a second plane the import
does not pass, so it is never offered. Before the synth has made its renderer there is no
device to ask, and nothing is offered but bytes. `XR24` and `XB24` carry a padding byte where
alpha would be, so their frames go through the renderer's conversion pass, which writes alpha
one.

**Either way it can fail, it falls back to bytes.** Negotiation can find nothing — a
compositor that shares memory only in a format outside the list — and the pipeline errors
before its first frame. Or it can agree on a format the import then refuses — a compositor drawing on
another GPU hands over memory this one cannot sample — and the renderer raises a flag the
frame carries (`DmaBuf::refused`). On either, the camera's next `latest` stops the pipeline
and opens the same source again on the bytes chain, `videoconvert` and all, keeping its last
frame up until the first of those arrives.

Cameras stay on bytes: a USB webcam's frames arrive in system memory whatever is asked. An
MJPEG one decoded by `vajpegdec` could stay on the GPU through `vapostproc`, but deciding
between that and a raw camera's bytes needs the decoder's output caps before the chain after
it is built; its planes are downloaded instead, which is one copy and no conversion.

**On a Mac a clip's frame is VideoToolbox's own `IOSurface`.** `vtdec_hw`'s plain output is
its `CVPixelBuffer`, NV12 on a two-plane `IOSurface`, and the bytes chain's `videoconvert`
passes it through untouched, so `platform::video::dmabuf_chain` is that chain and there is no
second one to fall back to. What `Player` asks for is only which frame a sample becomes:
`clip::frame_from` consults `platform::video::dmabuf_frame` for a `DmaBuf` delivery alone,
and on a Mac that finds the pixel buffer behind the sample through applemedia's
`GstCoreVideoMeta` and answers `Pixels::IoSurface`: the surface, beside the same buffer mapped
as bytes, which costs about a microsecond a frame. A bytes delivery keeps its frames mapped,
which a reader of pixels relies on. The meta's struct is not installed with GStreamer, so it
is mirrored, and read only after the meta is found by its API's name, its registered size
matches the mirror's and the pointer is a `CVPixelBuffer` by its CoreFoundation type; a buffer
that fails any of them, or a software decoder's with no meta at all, goes as bytes. The
renderer samples the planes where they lie, as [rendering.md](rendering.md#dma-buf-import)
says, and goes back to the bytes for a surface it cannot import. **The frame keeps its name**:
a decoded frame is named by the stamp on the compressed buffer it came from, which rides on
the sample whichever way the frame goes, so zero copy changes nothing about which frame is
shown. `Player::open` asks for it once the renderer's device can import one.

**A screen on a Mac is its `IOSurface` too**, one BGR plane, which ScreenCaptureKit's own
pixel buffer carries, so the capture publishes `Pixels::IoSurface` from its sample queue with
no pipeline to choose a delivery. Its fourth byte is padding, so the renderer draws the one
imported plane through the conversion pass rather than sampling it as it lies, and a surface
that does not import uploads the bytes locked beside it.

## The file button

An [`Asset` option](nodes.md#option-kinds) holds a path rather than a choice, and the canvas
draws it as [a button rather than a select](ui.md#options-and-the-file-button). Clicking it opens a picker of the media
this project already holds — filtered to what the option accepts, so a `video` node offers
clips and not the workspace file beside them — with `Import a file…` under it for a file
dialog. A file can also be dropped on the window, and **the file decides which node it
makes**: a picture — a png, a jpg, a jpeg, a webp, or a GIF, still or animated — makes an
`imagegif`, and everything else makes a `video`. The drop asks the Image/GIF node's own list,
and a `video` node's holds no picture, since the transcode has no clip to make of one; a video
node handed a picture anyway, from an older project, says so on its picture band and names the
node that shows it. Unless it is a `.ssw`, which is a workspace and is
[imported](architecture.md#export-and-import) instead.

**Every way in copies the file into the project.** The dialog's answer and the drop each call
`Project::import_asset`, and what lands in the option is the reference it returns —
`assets/gumbasia.webm` — not the path the file came from. A file already in this project's
`assets/` is not copied again. Picking one out of the picker copies nothing at all: it is
already a reference, and choosing it is one `SetOption`. A copy that fails says so on the status line and leaves the
option unset, because a node playing nothing is visible and a reference to a file the project
does not hold would not be. What that buys is in
[architecture.md](architecture.md#the-project): the folder is the whole show, and it plays
after it is moved.

A node never sees any of this. `ctx.option` hands it the reference and `ctx.path` resolves
it, so a path outside `assets/` — one somebody typed by hand — still means itself. See
[cpu.md](cpu.md#the-tick).

## Posters

Every file in `assets/` gets one: **a single frame, decoded out of the file itself, written
into `cache/` as a PNG at most `POSTER_WIDTH` across.** The file picker on a node draws it and
so does the project tab's asset card, which is what makes those two the same object seen twice.

**Out of the source, not out of the transcode.** A poster is wanted for every file in the
folder including the ones no node has played yet, and those have no cache entry — waiting for
one would mean the picture arriving only after somebody had already found the clip by hand.
`clip::Player` is a `filesrc ! decodebin`, so it opens whatever the machine can decode.

A second in, not frame zero: a cut usually opens on black, and a black card says nothing about
which clip it is. Box-filtered rather than sampled, because a poster is a whole frame shrunk by
a factor of six and nearest-neighbor at that ratio turns a face into noise.

`App` runs **one at a time, on a worker**, and remembers the files no poster could be made
from so it is not attempted again every frame — a font, a text file, a codec this machine
cannot decode. Those keep the icon their extension gives them. The poster is keyed on the
source alone, like the soundtrack, so it lands in `cache/` under the same prefix everything
derived from that file does — which is what makes removing an asset remove its poster with it,
without `remove_asset` knowing posters exist.

## What the tests can and cannot do

Nothing in `cargo test` opens a camera or a microphone: the camera tests run GStreamer's
`videotestsrc` through the real `appsink`, so the pipeline, the caps negotiation and the
handoff are all exercised and only the source is synthetic. `Source::Described` is how a test
picks the format: the pattern in I420 at a width whose rows pad, or encoded to MJPEG the way a
webcam sends it, which is how the `vajpegdec` preference is tested. `tests/gpu_upload.rs`
feeds one source the pattern in every layout in turn and reads back the same bars the pattern
paints in RGBA, and `tests/gpu_dmabuf.rs` imports real frames from `videotestsrc` through
`vapostproc`. Nothing tests a screen cast end to end, because that asks the portal; the
formats it would offer are asked of Vulkan by a test of their own. On a Mac nothing calls the
picker either: a test makes its own `CVPixelBuffer`, backed by an `IOSurface` as
ScreenCaptureKit's are, and reads it back as a frame at its own stride, and another writes a
`Camera`'s slots by hand where the stream's output would. `tests/gpu_iosurface.rs` encodes
the pattern through the H.264 row, decodes it into VideoToolbox's own surfaces, and imports
them.

The clip tests are the exception to "layer 1 is sub-second". They drive the real hardware
encoder, because the thing worth testing is that this machine's codec actually produces a
seekable all-intra file. That is why `doctor.sh` checks for a codec pair before `check.sh`
runs `cargo test` at all.

Nothing opens a controller either: `gamepad::bench` installs a pad that `Pad::open` takes in
place of a device, so `tests/actions.rs` drives a button down and up with nothing plugged
into the machine and `doctor.sh` never asks whether a controller is present. The same is
true of the pointer: `App::set_pointer` is the seam a picture window, the preview, the canvas
and a test all write through, so `tests/uniform.rs` feeds a synthetic one through the
context.

## The pointer

`pointer.rs`, and one slot. The pointer is a device like the others — what sees it writes a
reading, and the tick reads the newest and never waits — but it is the one with **two**
sources, because the surfaces a hand can be over are not all egui's.

A picture in a window of its own is a Wayland surface on the pictures thread, so its pointer
arrives through that thread's own `wl_pointer`; the mixer's preview and the canvas are
ordinary egui regions, so theirs arrives through egui, in the frame, and is read by the next
tick the way a press on a button is. Both write into one `pointer::Feed`, and `Synth::tick`
takes a copy of it before the walk, so every node reading the pointer that frame reads the
same hand.

**A position is in the picture's own world units.** The reading is placed against the
*picture*, not against the rect it is drawn in: a blit letterboxes, so a picture whose shape
is not its window's has bars down two sides, and a hand on a bar is off the picture and reads
nothing. What comes out is worldspace — height 2, width 2·aspect, up positive — which is what
a shape's center and a `sample`'s point are in, so `x` and `y` land where the hand is.

`mouseinput` is the node that reads it, and its **Measure against** option names the surface:
*Picture* — a popped-out one, else the mixer's preview — *Preview*, or *Canvas*, with
*Picture* the default. *Options for the position framing* is read as the surface the
position is framed in: a patch that wants the hand measured
against the canvas says so, and a patch that says nothing gets the one a performer means.
**Off the framed surface it publishes nothing**, so the rows go blank rather than freezing on
the last place the hand was, and a button held when the pointer leaves comes up — an envelope
it opened has to close. Each button is both a level and a gate.

## Game controllers

`gamepad/`, over `gilrs`, which on Linux is evdev with its own hotplug watch. The shape is
the camera's: a thread of its own polls the device and writes into one slot, and `tick` reads
the newest. What is different is that a controller has **edges** as well as levels, so the
slot carries a short queue of them beside the axes, each stamped with the instant the thread
read it — and the tick turns that age into a moment inside its own frame, exactly as
`audioin` turns a sample count into one. A press between two frames is therefore not
quantized to the frame that noticed it, which is what silvia's animation-frame poll could
only do.

`gamepad` publishes silvia's mapping unchanged: both sticks and both triggers as uniform
numbers in `[-1, 1]`, with silvia's tenth of a dead zone on the sticks and up positive on
both, and the four face buttons and the four on the d-pad as actions. They are **gates**, so
a held button opens an envelope directly rather than needing a press and a release wired
separately, and a controller unplugging releases whatever it was holding.

The controller's own name, or the reason there is none, is on the node's status line. The
device option names which pad — the first connected, or one of four — and *Rescan* is an
action input with a button on its row, so the thing silvia had a button for is a port a
sequencer can reach too.

## MIDI

`midi/`, and the same rule every other device keeps: **nothing waits on it from the frame
thread**. A reader thread posts each message into a queue — on Linux one of this app's,
blocked on the ALSA sequencer, and on macOS and Windows CoreMIDI's or WinMM's own, calling
back into `midir` — and
the synth empties that queue at the top of every tick, before anything reads the graph.

**Every message says which device sent it, and the queue says when a device goes.** What
crosses is a `midi::Wire`: a message with its `Device`, or `Gone(device)`. On Linux the
reader's port is subscribed to the sequencer's own announcements, and a device's port
exiting, its client exiting or its wire into ours being cut is that device gone; on macOS the
pass that drops a source no longer offered says so as it drops it. A note held on a device
that goes away is let go, since the note-off it owes will never arrive — see [the
map](#the-map).

**A queue, not a triple buffer — the one place this file's common shape does not apply.** The
audio analysis hands over its newest reading and drops the rest, because a reading one frame
stale is still a reading. A MIDI message is an *event*: a note-off dropped is a gate that
never closes. So every message is kept, in order.

**Two sequencer clients, on Linux.** `alsa::Seq` is `Send` and not `Sync`, so the reader thread owns the
handle it blocks on and the frame thread keeps a second for the two things it asks — what
ports exist, and wire that one to ours. Subscribing one client's port to another's is an
ordinary sequencer operation, so a device is connected without the reader waking to do it.

**One connection per source, on macOS.** CoreMIDI has no subscription for a second client to
make, and `midir`'s `connect` consumes the client it is called on, so each source gets a
client and an input port of its own, and one more client, never connected, lists what is
there. A source is known by its CoreMIDI unique ID, which a device keeps across being
unplugged: one pass closes the connection of every source no longer offered and connects every
one offered and not held, so a device that went away is dropped and one that came back is
wired in again.

**A watcher thread runs that pass every second on macOS, so a device plugged in after launch
is heard with no Rescan.** macOS holds a USB accessory back until it is allowed, the first time
it is plugged in, so a controller can arrive after launch. The window lists sources live, so
without the watcher a device plugged in late would be listed, with a hollow mark, and deliver
nothing. The watcher sleeps on a client of its own, holds the
connections' lock only for the pass, and ends when the handle is dropped; Rescan runs the same
pass at once. Linux wires in at open and on Rescan.

**Windows is macOS's shape over WinMM**: a connection and a client per source, known by its
device interface path, and the same watcher. A WinMM input is one application's at a time, so
a source another program holds is refused on every pass until it is let go, and the refusal is
logged once rather than once a second.

**Every source at once, and no device to choose.** Which box is on the table is the rig, and
[the rig is written nowhere](architecture.md#three-tiers-of-saved-state). A binding names a
channel and a number rather than a device, so a patch opens the same whichever controller is
plugged in. The cost is that two controllers sending the same channel and CC drive the same
control, which is what the channel is there to avoid.

**`midir` on macOS and Windows, the sequencer directly on Linux.** `midir` is the portable wrapper and the
obvious choice. Through 0.10 it pins `alsa` 0.9 while cpal 0.18 needs 0.11 — and `alsa-sys`
carries `links = "alsa"`, so only one of them may exist in a binary. 0.11 accepts `alsa` 0.11,
and on Linux `midir` is a thin shell over `alsa::seq` anyway, which is what
`platform/linux/midi.rs` uses directly. On macOS it is CoreMIDI and on Windows WinMM, declared
for those two alone ([proposals/platform.md](../proposals/platform.md)).

### What is read, and what is not

| on the wire | here |
| --- | --- |
| Control Change | `Kind::Control { cc, value }`, 0–127 |
| Note On, Note Off | `Kind::Note { note, on }` — a note-on at velocity zero arrives already turned into a note-off, which is what half the hardware in the world sends |
| pitch bend, aftertouch, clock, sysex | dropped at the reader |
| a device leaving | `Wire::Gone`, never in the monitor |

Each of the dropped ones is a real thing a controller sends and none of them has an address
in this editor yet. A message with nowhere to go is noise in a monitor, so it is not carried.

### The map

`midi/bind.rs`. Keyed by the **trigger** — the channel and the number — because that is the
direction a message travels: one arrives, and the map is asked what it drives.

**Channel is part of the address**, which is where silvia is loose: it keys its own map by
the CC number alone, which is sixteen times fewer addresses and two controllers colliding the
moment both send CC 7.

**The target is a `Target`: a `PortRef`, or one of the Main Mixer's three show controls.**
A number control is keyed by its node and key — a port's, or [a hidden one](nodes.md) a
region draws, which has the same address and no port — and an action input *is* a port, so
one address covers all of those. What may be bound is `NodeDef::bindable`: every control a
node has but the ones it lists as `unbindable`, which are an Output's render numbers. What is
bound and not on a node is the rig's — the fade, `Target::Balance`, and the two presses
under it, `Target::Blackout` and `Target::Freeze`, the controls a VJ most wants under a
hand — each an arm of the enum rather than a second map kept in step with the first. In the
file a port row is `{node, key}` and a mixer control is `{"mixer": "balance"}`,
`"blackout"` or `"freeze"`, a name rather than an id since the graph holds nothing for it; a
mixer row naming a control this build has no name for is dropped on load exactly as a row
naming a control a node no longer has, or one it does not let MIDI bind.

**One trigger drives one control, both ways.** Learning a control that is already bound moves
its binding rather than doubling it, and a trigger that drove something else stops driving it.
A hand that learns the same control twice meant the second one.

**A note's press is held apart from the pointer's.** The canvas replaces its own half of the
held set every frame, from the buttons a finger is on. A note lives in the other half, until
it is let go — `App::press` writes that one and `tick` reads the union. One set for both meant
whichever wrote last won, and the canvas writes last every frame: a note arrived mid-frame,
after the `tick` that would have read it, and was swept up before the next one. Hiding the
editor with `H` clears the pointer's half alone, for the same reason — that is exactly when
somebody is performing.

**A held note is keyed by its device and its trigger**, so two notes on one action input are
two hands on it and the button comes up when the last of them does. **A device that goes away
lets go of its own notes** and of nothing else: a key held on a second controller still
holds. **Release all**, in [the MIDI window](ui.md#the-midi-window), lets go of every note at
once, for the note-off lost from a device that is still there. Neither is quieted by
learning or by a render, for the reason a note-off is not: letting go is never what those
rules guard.

**A binding outlives its node.** Deleting a node takes its controls out of the graph but
leaves their bindings in the map, so undoing the delete brings the knob back with the node.
What reads the map reads only its live half, `Bindings::live` — the bindings whose control
the graph has: the synth is handed only those, the window lists only those, and a save
writes only those. A node id is never handed out twice, so a binding left behind can never
come to drive another node.

A CC lands across the control's own range through `nodes::control_range` — the instance's
where it has one, the definition's otherwise — so a knob narrowed to 0.9–1.1 gets the whole
fader across that. Scaling is linear; a `Binding` holds its target and nothing else, so
there is no curve to set. A note reaches an action input through `App::press`, the same call
a button on the node makes, so a hand and a controller are one source to the node underneath.

**A CC on the fade is −1 at 0 and +1 at 127**, written onto the fade the synth renders with
on the tick it arrived, so the mix moves with the fader whether or not the editor is
painting. It rides back on the snapshot as `midi_balance` and the editor writes it into the
mixer directly — not through the bus, because [moving the fade is playing, not
editing](decisions.md#the-mixer-is-a-render-target-with-two-decks-not-a-node) and the undo
history never sees it. The same two rules a control write keeps hold for it: the synth keeps
the fader's value across a `Msg::Fade` sent before the editor saw the write, measured against
what the last one said; and a hand on the fade is later and wins, barred until the snapshot's
`fade_seq` reaches the fade the hand's move went out on. A note bound to the fade does nothing, with its row still in
the window, as a CC bound to a button does.

**A note on Blackout or Freeze flips it, and its note-off does nothing**, since each is a
press that holds until pressed again. **A CC holds it on at 64 and above**, which reads a
toggling button's 127 and 0 as on and off and a momentary one's as held while the finger is
down. Both are written on the mix the synth draws on the tick they arrive and ride back as
`midi_blackout` and `midi_freeze`, by the fade's two rules. `Msg::Fade` carries which of the
three a hand moved since the last one, `mixer::Hands`, and a hand's value is taken over a
write the editor had not seen even where it puts the control back where the write found it —
which, for a press, a hand pressing it back always does.

**Soft takeover, a preference off by default.** On, a CC whose value disagrees with its
control's moves nothing until the fader passes the control's value, then drives it from
there (`synth::picks_up`). It takes over when the control is within one code of the fader, or
still within one code of where the fader last was — it is following — or when the move from
the last code to this one crossed it; a first message with no last code takes over only
near. Meanwhile the synth publishes where the fader is, in the control's own units, as
`midi_ghosts`, and the control wears [a ghost mark](ui.md#the-midi-window) there. A control
moved by a hand, an undo or a project opened leaves its fader behind again, by the same test:
the control is no longer where the fader last was. The synth keeps every fader's last code
whatever the preference, so turning it on knows where each one stands. The fade keeps it too;
Blackout and Freeze are presses, and take a message as it comes.

Where a knob lands in the undo history is
[architecture.md](architecture.md#a-gesture-ends-on-a-release-or-on-silence); the window and
the `Alt`-click that binds are [ui.md](ui.md#the-midi-window).

**What the map cannot address.** A target is a `PortRef` or one of the mixer's three, so it
reaches a node's controls and its action inputs and, of the **rig's** controls, the fade,
Blackout and Freeze alone. The Main Input's gain and thresholds, the crossfade method, the
mix's resolution and the deck claims have no address: which deck is on air is already a note
on an Output's `Show on A` or `Show on B`, the method and the resolution are selects nobody
turns mid-set, and the Main Input is tuned once for the room. The fade is the one a hand
rides during a set and Blackout and Freeze are the ones it reaches for when the set goes
wrong, and the enum grows an arm the day another proves it. A binding to any of them is
project data and saved; where the fade stands and whether a press holds are written to no
file. See
[decisions.md](decisions.md#the-fade-blackout-and-freeze-are-the-rig-controls-the-midi-map-can-address).
