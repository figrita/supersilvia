# Proposal: one Output node, and offline rendering

**Status: built.** The eight steps of the order of work are in; what is left below is the argument that shaped them and the open questions still open. Six questions were answered
before and during the work and they are settled; everything else below is argument, and the
open questions at the end are open.

silvia has two terminal nodes: `output`, which renders live, and `offlineoutput`, which renders
a PNG sequence at any resolution and fps. **Here they are one node.** An Output is already the
one primitive that owns memory — a render target whose `frame` port republishes the result —
and rendering that target to a file is a thing you ask it to do, not a different kind of node.
A second node would mean building the patch twice, or cabling the same graph into two
terminals and keeping them in step by hand.

## What is settled

1. **This owns the render engine; timelines reuse it.** `timelines.md`'s *Rendering to a file*
   becomes "drives what the Output already does". The stepping, the warm-up and the frame
   writer are built here, once.
2. **Both writers, chosen on the node.** A PNG sequence *and* a video file, because they are
   different jobs: one is a lossless hand-off to an editor, the other is a thing you send
   someone.
3. **A source that cannot be stepped warns and does not block.** silvia's `offlineBlocked`
   taint is not ported. A render with a camera in it is a choice someone made.
4. **Audio-reactive works offline**, by analyzing a file at virtual time rather than reading a
   device.
5. **A render is modal, and says so loudly.** The frame thread is rendering, not performing,
   so the UI says it in a way nobody can miss — a banner across the editor with the progress
   and the one button that cancels. **The command bus is closed for the run**: `App::apply`
   refuses every `Command`, which blocks a new node, a cable, a control drag and an option
   change with one rule instead of a widget-by-widget disable. Undo is closed with it, since
   it is a command too. Starting and canceling are not commands — a render is done to the
   instrument, not to the document — so nothing in the bus has to be let through.
6. **A render ticks the open workspaces, exactly as live play does.** `App::tick` skips a
   node on no open workspace and holds its last value, and a render keeps that rule: a
   camera on a closed tab is not woken by a render that never looks at it. The Output being
   rendered has to be on an open workspace, and the `!` walk crosses workspaces the way
   cables do.
7. **A render goes into the project**, a numbered folder under `renders/` beside `assets/`,
   so it travels with the set and never overwrites the last one. A sequence is many files and
   wants a folder of its own either way.
8. **A render drives the Main Input's file.** Its clip and its sound are read at one
   position, and a render places that position at each frame's time rather than letting it
   run, then puts it back where live play had it. The same call the `video` node's soundtrack
   already makes, `Reader::advance_to`, is what analyzes the file at `t = i / fps`.

## The one node

The Output keeps everything it has — the input, `Show on A` / `Show on B`, `resolution`,
`frame` out — and gains a **Render** section: `fps`, `duration`, `warm-up`, `supersampling`,
the writer and where it writes. A render is an action on a node that is otherwise live, so the
live picture on its body is what you are about to render, at the resolution you are about to
render it.

**`resolution` is shared.** silvia's offline node has its own, which means the thing you
previewed is not the thing you got. One resolution, used by both, and `supersampling` at 1x,
2x or 4x multiplies it for the render only — the same trick silvia has, and the reason its
offline frames are clean where the live ones alias.

## Time, inverted

This is the whole of the interesting part. Live, time is a function of the wall clock:
`Clock::tick(now)` hands out a `dt` and an `elapsed`, `elapsed` reaches every shader as
`u_time`, and twenty CPU nodes read `ctx.dt` or `ctx.elapsed`. **Offline, time is a function of
the frame index** — `t = i / fps` — and nothing may consult the wall clock.

So the clock gains a way to be driven: `Clock::set_elapsed`, or a stepper that owns it for the
run. Every shader then gets the right `u_time` for free, because `u_time` already comes from
`elapsed` and nowhere else.

The CPU half is the part that does not come free, and it splits in two:

- **Pure in `t`.** GLSL generators (which take `phase` as an *input* rather than accumulating
  — see [decisions.md](../docs/decisions.md)), `video` driven by `position`, a sequencer's
  which-step arithmetic. These seek for nothing.
- **Integrating.** `slew`, `autogain`, `autoexposure`, `accumulator`, `adsr`,
  `smoothcounter`, `cellularautomata`, `slimemold`, `brickgame`, `randomfire`, a free-running
  `video`. Their state at frame *n* depends on every frame before it, so they cannot be seeked,
  only run.

**The registry needs one bit per node it does not carry: does your tick integrate `dt`.** And
`CpuNode` needs `reset()`, so a render can start them from a known state rather than from
whatever the last twenty minutes of live play left behind.

### Warm-up

Feedback raises a question a live renderer never has to answer: *what was on screen before
frame zero*. silvia answers it with three modes and they are the right three:

| | |
| --- | --- |
| **Black** | clear the buffers, advance no time-driven state |
| **Hold first frame** | render the scene at `t = 0`, repeatedly |
| **Run sequence** | run `warmup` frames at **negative** virtual time, `(i − warmup)/fps` |

*Run sequence* is the one that matters. A feedback ring needs to reach its steady state before
the first frame anybody keeps, and the honest way is to run the patch from before the
beginning.

## The `!`

Some sources can only answer *now*: a camera, a screen cast, a microphone, and a `maininput`
pointed at any of those. Stepped to `t = 4.5` they hand back whatever they have, which is
whatever the wall clock gave them.

The Output shows a **`!`** when any source upstream of it is one of these. Clicking it opens a
small popup naming each one and offering to go there — the node, by the same navigation the
[cross-workspace tag](../docs/ui.md#the-tag-a-cable-whose-far-end-is-elsewhere) already uses,
or the Main Input panel when the source is the panel's.

**It warns and nothing more.** The render runs, the camera contributes what it gives, and the
result is deterministic given what it gave. This is already the position
[timelines.md](timelines.md) argues, and the reason silvia's taint is not ported: the taint
refuses a thing you might well have meant, and it propagates transitively, so one webcam in a
corner of a large patch can refuse an Output that barely depends on it.

The bit this needs is the inverse of the *integrates* bit and is per node: **can you answer at
an arbitrary `t`**. Both are registry data, both default to the safe answer, and the `!` is a
walk back through the graph exactly as `live_nodes` walks it.

## Offline audio

A band that reads zero makes an audio-reactive patch render dead, which is most of the point
of rendering one. So the analysis is driven by virtual time instead of by a device.

**The machinery is already here.** `audio::Track` decodes a file to a seekable buffer beside the
transcode, and `audio::Reader::advance_to(t)` returns the analysis of the window ending at `t` —
that is precisely what a `video` node's soundtrack already does, and why scrubbing a clip
scrubs its bands with it. An offline render points the Main Input's analysis at the same call
with `t = i / fps`.

A device-backed audio source gets the `!` and reads whatever it reads. A **sound file** — the
Main Input's, or a clip's own soundtrack — is exact and repeatable, and is what a music video
is rendered against.

## The writers

Both take RGBA8 rows-top-first, which is what `nodes::Frame` and the readback already are.

- **PNG sequence.** `video/png.rs` already writes exactly this through GStreamer's `pngenc`;
  a sequence is that call per frame into a numbered folder. Nearly free.
- **Video file.** `video/clip.rs` already probes the machine's hardware encoders for the import
  transcode (`Codec::probe`), and the same encoder writes a render. Frame rate and duration are
  the node's, not the clip's.

## Full-resolution readback, which is the real new GPU work

**The existing readback will not do.** `render/output.rs` has the whole discipline — issue the
read after the frame's own fence has been checked, collect it a frame or two later, never wait
— but `THUMBNAIL` is a fixed 240x135 and its buffer is sized for that.

A render needs the Output's real resolution, times supersampling, and it needs *every* frame
rather than one on request. That is a second readback path on the same pattern: a PBO ring deep
enough that the CPU is never waiting on the GPU, a fence per frame, and back-pressure that
slows the stepper rather than dropping a frame — **an offline render may not drop frames**,
which is the one place this differs from everything else in `render/`, where a late frame is
skipped on purpose.

## What changes elsewhere

| where | change |
| --- | --- |
| `nodes/output.rs` | the Render section: fps, duration, warm-up, supersampling, writer, destination |
| `nodes/mod.rs` | two registry bits per node: *integrates `dt`*, and *can answer at an arbitrary `t`* |
| `nodes/cpu.rs` | `CpuNode::reset()`, for warm-up and for starting a render from a known state |
| `clock.rs` | `set_elapsed`, and a stepper that owns the clock for the duration of a run |
| `render/output.rs` | a full-resolution readback path beside the thumbnail's, with a fence ring and no dropped frames |
| `render/mod.rs` | supersampled render targets for a run, released after it |
| `video/png.rs` | a sequence writer over the existing single-image `write` |
| `video/` | a render encoder, over `clip.rs`'s probed codecs |
| `audio/` | analysis at a given `t` for the Main Input, which `Reader::advance_to` already does for a clip |
| `app/` | the render as a job with progress, cancelable; the command bus closed for its length; the tick over every workspace |
| `ui/node_widget.rs` | the Render section, the progress bar, and the `!` with its popup |
| `docs/` | `nodes.md` for the merged node, `rendering.md` for the readback and the inverted clock, `media.md` for offline analysis, `decisions.md` for the merge and for warn-not-block |

## Order of work

1. **The clock inverts.** `set_elapsed`, a stepper, and a test that the same patch at the same
   `t` gives the same `u_time` twice. Nothing renders to a file yet.
2. **The two registry bits**, and `CpuNode::reset()`. Fill them in for all twenty nodes that
   read `dt`; the audit is the work.
3. **Full-resolution readback**, with a headless GL test that a known pattern comes back
   byte-exact at 1920x1080 and that no frame is dropped under an artificial stall.
4. **The stepper, warm-up and the PNG writer.** First real render. Warm-up modes tested
   against a feedback patch, which is the only thing that can tell them apart.
5. **The merged node's UI**, the banner, progress and cancel, and the closed command bus.
6. **The video writer** over the probed encoders.
7. **Offline audio**, pointing the analysis at virtual time.
8. **The `!`**, its walk and its popup.

Steps 1–3 are the ones with hidden difficulty. Step 8 is the only one a person sees and it is
last on purpose: it describes a limitation of the thing above it, and it cannot be written
before that thing exists.

## Open questions

- **Does a render use the mixer, or one Output?** The node is the Output, so one Output. But
  the thing on screen is usually the *mix*, and a render that cannot capture a crossfade is a
  render of the wrong thing. This may want *Render* on the Main Mixer panel too, driving the
  same engine with the mix as its source.
- **What happens to the live picture during a render?** Frozen, still running, or showing the
  render as it goes. Showing it is the nicest and costs a blit. The projector is the one place
  this is not cosmetic: a render started during a set puts the render on the wall.
- **Supersampling and `tap`.** A tap measures its input at a canonical grid; at 4x the grid is
  the same and the picture is not. Probably nothing to do, but it should be checked rather than
  assumed.
- **Can two Outputs render at once?** One at a time is simpler and almost certainly enough.
