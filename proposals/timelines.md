# Proposal: timelines

**Status: proposed, not agreed.** Nothing here exists. The principles it stands on are
already agreed — one address space with four writers, keyframes are document and values are
derived, connection > lane > manual, Read/Off/Write — and live in
[docs/decisions.md](../docs/decisions.md#a-timeline-and-a-viewport-over-one-parameter-address-space);
this is the shape built on them. When it is agreed, its decisions join that entry as *Agreed,
not built*; when it is built, the behavior moves to `docs/`. Everything it stands on is built:
the tab bar and the workspace kind ([docs/ui.md](../docs/ui.md#the-tab-bar)), the project
folder a timeline is saved in ([docs/architecture.md](../docs/architecture.md#the-project)),
and the event half for cues ([docs/cpu.md](../docs/cpu.md#the-event-half)).
[timelines-mockups.html](timelines-mockups.html) is the page this was drawn on: the object
model, the tab, the jump rule, and today's A/B practice mapped onto it.

## What this is for

Three things a performer cannot do today, and one object that gives all three:

- **Compose.** Arrange several Outputs on one frame with position, rotation, scale, opacity
  and a blend mode, by dragging handles on a picture rather than typing numbers.
- **Make videos.** Render a show against an audio file, frame-accurate and repeatable, for a
  music video or a visualizer. silvia's offline renderer does this today with `_prepareForTime`
  driven in topological order; supersilvia already runs that model live, with the wall clock
  as its transport.
- **Orchestrate a set.** Automate parameters across a whole performance, jump between cues,
  and see the big picture while still turning knobs.

The object is a **timeline**, and it is a second `WorkspaceKind` beside `Video`, following
silvia's audio branch where a workspace's kind decides what it shows. A timeline owns no
nodes and no compiler, so it is the kind `WorkspaceKind::is_portable` refuses — every
reference in it points into other workspaces, and it travels with the project instead. It
holds **references** into the one graph: an
Output referenced by a timeline is a **clip** and is composited; any other node referenced by
a timeline opens a **track group** of tracks over its parameters; and a **free track**,
owned by no node, can be bound to inputs on any number of nodes in any workspace.

Two rules are the spine of everything below.

> **A track owns exactly the addresses bound to it. Nothing else is timeline-controlled.**

> **An Output on a timeline is a clip. Nothing nests, nothing is instanced.**

## The model

### The tab

```rust
WorkspaceKind::Timeline

pub struct Timeline {                 // stored on the Graph beside the workspace it is
    pub clips: Vec<Clip>,             // compositing region, front first: order is z
    pub tracks: Vec<Track>,           // every lane on the timeline, owned or free
    pub audio: Option<AudioTrack>,
    pub cues: Vec<Cue>,
    pub length: f64,                  // seconds
}
```

Everything in it is graph data: saved in its workspace file, undoable by snapshot, edited through
commands. A timeline referring to a node that has since been deleted keeps the reference and
reports it, the way a dropped connection on load is reported rather than hidden.

**What is document and what is derived.** A clip's extent and controls, a track's curve and
its bindings, and a cue are document: edited through commands, undoable, saved. The value a
track produces each frame is derived, written during `tick` and never a command — a lane
playing back must not fill the undo history. The transport's position is app state, like a
pan. That split is the agreed rule and it decides what every struct below is for.

### The compositing region

```rust
pub struct Clip {
    pub output: NodeId,               // an Output, in any workspace
    pub start: f64, pub end: f64,     // seconds on this timeline
    pub controls: BTreeMap<&'static str, ControlValue>,   // x y rot scale opacity blend
}
```

**The transform is on the clip, not the Output.** Two timelines can place one Output in two
corners. The cost is a second owner of addresses: the graph owns `Node.controls`, a timeline
owns `Clip.controls`, and a lane, a MIDI fader and a viewport gizmo write both through one
`Address` enum. silvia made the same call for its mixer, where which channel an Output is on
is mixer state. The alternative — a layer as a node, whose position and opacity are controls
on it — would make every placement a node in the graph and every Output placeable exactly
once; the clip is what lets *a meter drive a layer's rotation* without that, because a clip
control is an address like any other.

**Z-order is the clip list.** Layers are ordered and a DAG is not, and expressing order
through connection order or a `z` control was the one part of this that did not extend the
graph; a front-first `Vec<Clip>` says it plainly, and reordering is a list edit.

An Output whose clip is not under the playhead **does not render**. Cost is bounded by what
is on stage now. An Output on no timeline renders as it does today, a buffer other graphs may
sample.

### Tracks

```rust
pub struct TrackId(u32);

pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub owner: Option<NodeId>,        // Some: drawn in that node's group. None: a free track
    pub bindings: Vec<Address>,       // the inputs this track writes; one for owned, any for free
    pub kind: TrackKind,
    pub mode: LaneMode,               // Read, Off, Write: the console model; Off is not delete
}

pub enum TrackKind {
    Automation { curve: Vec<(f64, f32)> },       // → a number control
    Cycle      (Cycle),                          // → a number control, seamless by construction
    Color      { clips: Vec<ColorClip> },        // → a color control: picker clips
    Media      { clips: Vec<MediaClip> },        // → a varying color input: file, image or gif clips
}
```

`Cycle` is [its own section](#loops-and-being-seamless-by-construction): it is the kind a
loop is made of, where `Automation` is the kind a show is made of.

**A track is the source. An input binds to it.** The timeline has two lane regions below
the compositing region, and both are made of the same `Track`:

- **Node groups.** A track with an owner is drawn under that node's header, one lane per
  bound input. A group exists the moment one of the node's parameters has a track, and is
  grouped per node for display only, as `decisions.md` already says.
- **Free tracks.** A track with no owner is drawn in its own region and can be bound to
  **any number of inputs on any nodes in any workspace**. One media track can feed three
  effect chains; one color track can tint five things. It is the sharing a fan-out cable
  gives, from a source that lives on the timeline rather than in a graph.

A **media clip** on a track has in and out points, speed and direction, and a file, image
or gif. The track owns the decoder: a `video::Player` opened on the timeline's tick, asked
each frame for the frame the playhead implies, publishing an `Arc<Frame>` the way a CPU node
does, keyed by `TrackId` rather than `NodeId`. Scrubbing a media track scrubs the picture by
construction. The `video` node stays for the other case, a file whose position a cable
drives.

Outside every clip on a track, and when the track is `Off`, the input is what it is
unplugged: its picker, or the hue wheel. **A track is a fourth writer, present or absent,
not a replacement for the port.**

### What a bound input compiles to

`CompileContext::input` has five cases today. An input bound to a track is a sixth, and it
is the only place this proposal reaches the compiler:

| track kind | the input yields |
| --- | --- |
| `Media` | `(u_track_7_on > 0.5 ? texture(u_track_7, aspect(uv)) : <what the port would be unplugged>)` |
| `Color` | `(u_track_7_on > 0.5 ? u_track_7 : <what the port would be unplugged>)` |
| `Automation` | `u_track_7`, no gap: a curve always has a value |

A track's kind is fixed when it is made, so its compiled shape is fixed: binding or
unbinding a track is a structural edit and recompiles, as `Connect` does; a clip moving
under the playhead is a uniform and recompiles nothing. That keeps the recompile boundary
where it is: **changing what the graph is costs a shader rebuild, changing what it does
costs a float**. The `_on` uniform is the "present or absent" bit, so a gap never rebuilds.

Several inputs bound to one free track all name the same uniform, so an Output program
holds one sampler per track it uses however many nodes read it, and the renderer uploads a
track's frame once per program, skipping an `Arc` it has already seen as it does for a
camera.

### The audio track

One audio file per timeline. It is a source: bands, level and beats reach the graph as
uniform numbers and cues the way `audioin` publishes them, so a graph built against the
microphone plays against the file unchanged. In render mode it is also the clock's
authority, which is where `decisions.md` already puts a running audio device.

### Cues

A cue is a time and an action: jump to a time, start or stop the transport, or fire an
`Action` output into the graph. A cue that fires is an event with `at`, the way a
`bpmclock` beat is, so a cue between two frames lands where it was placed rather than on the
frame that noticed it.

## Precedence, and the tag

An address can be written by hand, by a cable, by a track, or by MIDI. `decisions.md` sets
the order, connection > lane > manual, and says the winner must be visible. The mechanism is
silvia's cross-workspace tag, generalised: **a control whose address is owned by something
off screen shows a pill saying what, and clicking it goes there.** A cable from another tab
already needs one; a track on a timeline tab is the same case.

The pill on a bound input names **the track**: its icon by kind, its name, and the timeline
it is on. Clicking it activates that timeline tab and scrolls to the track. The track's
`Read`, `Off` and `Write` live on the pill, so a knob is taken back or recorded without
leaving the node editor. On the timeline the track header carries the reverse tags, one per
binding: the node, its input, and its workspace, each clickable. A free track bound five
ways shows five.

## The transport

The transport sets `Clock::elapsed`. Nothing else changes about the clock: there is one, and
a timeline is a coordinate system over it, as `architecture.md` says a clip's local time is.

Two modes, and they are modes of the transport, not two kinds of show:

| | jump means | for |
| --- | --- | --- |
| **Live** | lane-driven values take the lane's value at the target; **all other state carries across** | a set: trails keep decaying, slews keep sliding, the camera is the camera |
| **Render** | state is reset and run forward from a warm-up point before the frame | a file: frame 1200 renders the same twice |

A third bit, orthogonal to both: **`looping`**, which wraps the playhead at `length` instead
of stopping. Live, it is how a timeline runs unattended behind a set. In render, it is what
[a seamless loop](#loops-and-being-seamless-by-construction) is exported from.

Render mode needs one bit per node the registry does not yet carry: **does your tick
integrate `dt`**. GLSL, `video`, and a sequencer's which-step arithmetic are pure in time and
seek for free. `slew`, `autogain`, `autoexposure` and a free-running `video` integrate and
need reset-then-run. silvia found the same bit building Phase 5 of its offline renderer and
gave it a warm-up of *black*, *hold* or *run the sequence*; render mode here has the same
three.

Sources that can only answer *now*, a camera or a microphone, are not refused in render
mode. They are what they are, the frame is still deterministic given what they gave, and a
render with a camera in it is a choice someone made. silvia's `offlineBlocked` taint is not
ported; the timeline is not an offline renderer that happens to run live, it is the live
transport that can also step.

## Loops, and being seamless by construction

A **seamless loop** — a clip whose last frame flows into its first with no visible seam — is
the most shareable artifact this program can make. It is the native form of every place
moving images travel, it needs no player controls and no context, and it is produced as a
byproduct of ordinary use rather than as a separate export chore. It should be a first-class
thing the timeline makes, not something a careful person can achieve.

The prior art is ours: **torquigen** (`js/animator.js`) made every animation seamless by
refusing to represent a curve that was not. Two ideas carry over, and one is worth
tightening.

### A cycle, not a curve

torquigen gives each animated control a **start value, an inflection value and an end value**,
an **inflection point** in normalized time, **two tween functions** — one per segment — and an
integer **repeat count**. Time is normalized and wrapped, `adjustedT = (t * nT) % 1`, and the
two segments are evaluated either side of the inflection:

```js
if (adjustedT <= inflectionPoint) {
    adjustedT /= inflectionPoint;
    value = firstTweenFunction(startValue, inflectionValue, adjustedT);
} else {
    adjustedT = (adjustedT - inflectionPoint) / (1 - inflectionPoint);
    value = secondTweenFunction(inflectionValue, endValue, adjustedT);
}
```

The shape is right and one field is wrong. **A separate end value is the only way to break the
loop**, so it should not exist: the value at the end of a cycle *is* the value at its start,
and there is nothing to get wrong because there is nothing to type.

```rust
pub struct Cycle {
    pub rest: f32,              // the value at phase 0, and therefore at phase 1
    pub peak: f32,              // the value at the inflection
    pub inflection: f32,        // 0..1, where the turn happens
    pub out: Tween,             // rest → peak
    pub back: Tween,            // peak → rest
    pub cycles: u32,            // whole cycles per loop. never fractional
    pub phase: f32,             // 0..1, where in its cycle this track starts
}

pub enum Tween { Constant, Linear, EaseIn, EaseOut, EaseInOut }
```

Two guarantees fall out of the type, not out of care:

> **A `Cycle` returns to `rest` at the end of every cycle, because `rest` is both ends.**

> **`cycles` is a `u32`, so every track completes a whole number of cycles in one loop
> length, and every track wraps at the same instant.**

`phase` is what lets two tracks on the same period run against each other — a scale swelling
while a rotation lags a quarter turn behind it — without either leaving the loop.

Five tweens is the right number. torquigen shipped exactly these and never wanted more, and a
longer list is a worse control: what makes a loop feel alive is `cycles` and `phase` disagreeing
between tracks, not a sixteenth easing curve.

### Both kinds, for both jobs

`Automation` stays. The two kinds are for different work and neither replaces the other:

| kind | for | seamless |
| --- | --- | --- |
| `Cycle` | a loop: something that breathes, forever | **by construction** |
| `Automation` | a show: a music video, a set, a thing that goes somewhere | only if authored so |

A timeline with `looping` set and every number track a `Cycle` is seamless without anyone
checking. That is the case worth making effortless, because it is the case that produces
something to share.

### What still breaks a loop, and saying so

Wrapping the curves is not sufficient, because the graph carries state the timeline does not
own. The good news is that **the bit needed to detect this is already in this proposal**:
render mode requires each node to declare *does your tick integrate `dt`*, and that same bit
answers *can this loop seamlessly*.

| what breaks it | why |
| --- | --- |
| `slew`, `autogain`, `autoexposure` | they integrate `dt`; their state at the wrap is not their state at zero |
| an Output with frame history | feedback carries a picture across the wrap |
| `video` on a free-running clip | its position at the wrap is not its position at zero unless the clip's length divides the loop's |
| `counter`, `adsr`, a free-running `bpmclock` | event state that does not reset |

The answer is not to forbid any of them — a trail decaying across the wrap is often exactly
what makes a loop good, and a feedback loop that has *converged* is seamless in practice even
though its state is not identical. So the timeline **reports** rather than restricts:

> **The loop badge.** A timeline with `looping` set shows one indicator: *seamless*, or
> *seamless after warm-up*, or a list of what carries across. Each entry names the node and
> is clickable, the way a dropped connection on load is reported rather than hidden.

*Seamless after warm-up* is the common and interesting case, and it is already solved by
machinery this proposal needs anyway: run the transport from zero for one full loop length
before recording, so integrators have converged and feedback has settled, then record the
second pass. That is exactly the **run the sequence** warm-up mode render mode already
carries.

### Exporting one

Given render mode and the frame writer, a loop export is not new machinery — it is render
mode with three constraints:

- the range is exactly `0..length`, not a user-typed in and out
- the warm-up is *run the sequence* for one full length, always
- the last frame is `length - 1/fps`, never `length`, because the frame at `length` **is**
  the frame at zero and writing both is the one way to put a stutter in an otherwise perfect
  loop

The formats are the ones already in the box: H.264 or AV1 in MP4 through the encoders
`video/clip.rs` already probes, WebM, and a GIF through `imagegif`'s own decoder. torquigen
exported WebM, GIF and PNG for the same reason.

`Export ▸ Loop…` on a looping timeline, with a length, a resolution and a format, and the
progress reaching the tab the way a transcode reaches a `video` node's button.

## The viewport

The composite of the timeline's clips under the playhead, drawn in the preview panel when a
timeline tab is active. Which display it goes to belongs to the mixer and projector work and
is out of scope here.

The viewport is a **second editor**, per `decisions.md`: dragging a clip's handle emits
`SetClipControl`, the same shape as a knob scrub, coalesced the same way into one undo step.
Corners scale, a ring rotates, the body moves, the wheel sets opacity. Nothing about the
gizmo is stored; it is drawn from the clip's controls each frame.

## The compositor

One render pass after every Output has rendered: for each clip under the playhead, back to
front, sample the Output's published texture with the clip's transform and blend it. It is
silvia's main mixer with n channels and z-order instead of two channels and a crossfader, and
its eight wipes are blend modes here. It samples published textures, so it is **one frame
late**, which is what a delayed port already means and what the mixer already accepted.

It lives in `render/` beside `OutputRenderer`, is not a node, and needs no compile step: a
clip added or reordered changes a list the pass walks, not a shader.

## Rendering to a file

**The engine for this is [offline-render.md](offline-render.md)'s, not this proposal's.** The
inverted clock, the *integrates* bit, `CpuNode::reset`, the warm-up modes, the
full-resolution readback and both frame writers are built there, on the Output node, and a
timeline drives them from its transport instead of from a button.

What is left here is the driving: the transport steps `1/fps` per frame, the audio track is
analyzed at virtual time, every Output on stage renders, the compositor runs, and progress
reaches the tab the way a transcode reaches a `video` node's button.

## What changes elsewhere

| where | change |
| --- | --- |
| `graph/` | `Timeline`, `Clip`, `Track`, `Cycle`, `Cue`; `Address` as the one thing all four writers target |
| `compile/` | the sixth input case: an input bound to a track |
| `command.rs` | clip and lane edits; `SetClipControl`; transport commands are app state, not edits |
| `clock.rs` | *(from [offline-render.md](offline-render.md))* `set_elapsed` and the render stepper |
| `nodes/mod.rs`, `nodes/cpu.rs` | *(from [offline-render.md](offline-render.md))* the *integrates* bit and `CpuNode::reset` |
| `render/composite.rs` | new; the only new `unsafe` |
| `render/` | track textures, uploaded once per program, keyed by `TrackId` beside the node-keyed source textures |
| `ui/timeline.rs`, `ui/viewport.rs` | new; return actions, never mutate |
| `ui/node_widget.rs` | the tag on a track-bound control, naming the track, with its mode |
| `workspace.rs` | the timeline fields, `#[serde(default)]` |
| `audio/` | `Analyzer` over a decoded file at a given time |
| `video/` | a `Player` owned by a media track rather than a node; the same type |
| `docs/decisions.md` | the timeline-and-viewport entry gains what this settles |

## Tests

- **Layer 1.** Timeline data round-trips through the file and undo. A lane's value at a
  time. A clip under and not under the playhead. Live jump: an automated control takes the
  lane value, a slew's state is unchanged. Render jump: a slew is reset and re-run. The
  *integrates* bit agrees with a tick that reads `dt` (a registry test, as
  `reads_frame_history` is checked today). GLSL snapshots of an input bound to each track
  kind, and of two inputs bound to one free track naming one uniform. Binding recompiles,
  a clip moving does not, asserted the way `tests/controls.rs` asserts the boundary today.
- **Layer 2.** Add a timeline tab; drag an Output onto it by name; the clip is in the tree;
  the Output's control shows a lane tag after a lane is drawn; clicking the tag switches
  tabs. The viewport gizmo emits one coalesced command per drag.
- **Layer 3.** Two Outputs composited at known transforms and blend; read the pixels back.
- **Layer 4.** A rendered file of a checkerboard against a click track has the frame count
  the length and fps say, and frame *n* rendered twice is identical.

## Order of work

1. `WorkspaceKind::Timeline` and an empty timeline tab with a transport that sets
   `elapsed`. Play, pause, scrub. Nothing on it yet; every Output still renders.
2. Clips. Drag an Output onto the tab; extent gates its render; the compositor; the
   viewport with gizmos. This alone is the composition goal.
3. Owned tracks: automation and color, the `Address` enum, the sixth compile case, the tag
   with Read/Off/Write. This is the automation goal in its live form.
4. The audio track as a source, from a file.
5. Render mode: the integrates bit, warm-up, the stepper, the frame writer. This is the
   music-video goal.
5b. **Cycles and the loop.** `TrackKind::Cycle`, the `looping` transport bit, the loop badge
   from the integrates bit already landed in step 5, and `Export ▸ Loop…`. Small on top of
   step 5 and the step that makes something worth showing anyone, so do not let it slide
   behind steps 6 and 7.
6. Media tracks, with the track-owned player and the track texture path. Then free tracks,
   which are the same `Track` with `owner: None` and more than one binding.
7. Cues.

Steps 1 and 2 are the smallest thing that is useful, and they touch neither the compiler nor
any node.

## Open questions

- **Is `Cycle` a track kind, or a mode any number control can be put into?** As a track it
  fits the model with no new concept, and it is proposed that way. But a cycle is small,
  needs no lane to draw, and a performer may want one on a knob without a timeline existing
  at all — which would make it a property of the control, and a different proposal.
- **Should `cycles` be allowed to divide as well as multiply?** A track at one cycle per
  four loops is still seamless if the *loop* is four lengths long, but it is not seamless at
  one length, and a `u32` says no to the whole question. A rational `n/d` with `d` dividing
  the loop count is the general answer and may be more rope than anyone wants.
- **Which timeline is live, and what the projector shows** when several exist. Mixer and
  projector work; deferred.
- **Does an Output keep rendering off stage** between its clips, so its trails are warm at
  the next one? Same scope; deferred. Until then, an Output between clips goes cold.
- **Per-pixel time.** A lane on a `time` port is a uniform number per frame. Whether a node
  should ever receive a different `t` per pixel, for slit-scan and retimed subtrees, is the
  one question the meditation this grew from left open, and this proposal does not touch it.
- **Should uniform number tracks be free too?** Nothing above stops an automation track with
  no owner driving five knobs; the shape is identical. The proposal states it for color and
  media because that is what was asked for.
- **A bound input and a cable at once.** Precedence says the cable wins. Does binding a
  track to a connected input refuse, or bind and wait?
- **Nesting.** The second spine rule rules it out: a timeline is not a clip on another
  timeline, and an Output is not instanced. If it ever comes back it is a compiler-naming
  problem rather than a UI one — two uses of one group collide on `{slug}{id}_{key}`, so
  either a group is copied on use or names become path-qualified — and a nested timeline's
  zero would have to be passed down like `time` rather than read from a global.
