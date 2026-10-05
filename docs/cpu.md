# CPU nodes

Everything a node computes outside a shader. The `UniformNumber` port type, the per-instance
state behind it, and the once-a-frame tick that drives both.

For the sources built on this — the microphone, cameras and video files — see
[media.md](media.md). For why `UniformNumber` is a type rather than a convention, see
[decisions.md](decisions.md#a-fourth-port-type-uniform-numbers).

## Two kinds of state, and which one a saved file holds

silvia draws this line explicitly, and it is worth naming the same way here because the
distinction is the whole of what a save file is. `registry.js` gives a node two records:

- **`values`** — "serializable, user-controlled state that doesn't fit the standard `input`
  or `options` model. **WILL** be saved in patches."
- **`runtimeState`** — "non-serializable, runtime-only state, such as WebGL contexts, media
  streams, or interval IDs. **NOT** saved in patches."

The cellular automata node uses both: `gridScale` and `initThreshold` are `values`, while the
`Uint8Array` grid, the interval id and the preview canvas context are `runtimeState`.

supersilvia has the same split with different names, and one deliberate difference — and the
split is a struct rather than a habit. **`Synth` is the running state**: the clock and the
transport over it, every `Box<dyn CpuNode>`, every uniform number, color, frame and event they
published, the buttons held, the seeks asked for, the touches a hand made on a node's own surface — a well dropped on
a pad — the tap readings a tick reads, and the Main Input's capture. `App`
keeps the document — the graph, the project, the undo history — and the editor around it. The
graph the tick reads is a *copy* of the document's, an `Arc<Graph>` replaced on an edit and on
nothing else, so what a tick sees cannot change under it. A
field is on the synth exactly when a tick writes it or reads it as input, which is why `held`
is there and the popup it was pressed from is not. The transport is running state, and
nothing of it is saved: a project opens playing, at zero ([the transport](#the-transport)).

| silvia | here | saved | lives in |
| --- | --- | --- | --- |
| an `input`'s control | `Node.controls` | yes | the graph, and the workspace file |
| an `options` entry | `Node.options` | yes | the graph, and the workspace file |
| `values` | `Node.values` | yes | the graph, and the workspace file |
| `runtimeState` | `Box<dyn CpuNode>` | no | `Synth::cpu`, keyed by node |

**A parameter is never a value.** A number, a color or a choice is a control on an input port
or it is an option, and those are the only two a parameter may be. That is what makes
`Node.controls` the single source of truth for a parameter, and it is what lets automation, a
uniform number, MIDI and a direct drag all be writers to one address rather than four parallel
systems. A node wanting a persistent number that is not a port declares an input with a
control and leaves it unconnected: it costs nothing, it is scrubbable, and it saves for free.

`Node.values` is the third store for what is neither — a note's prose, `text`'s words, a
control's own range, a recorded curve — and unlike silvia's untyped bag every one of them is
declared up front by a `ValueDef`. What separates it from an option is who draws it; see
[nodes.md](nodes.md#a-nodes-own-values) and
[decisions.md](decisions.md#a-nodes-own-values-are-not-options). A `tick` reads one with
`ctx.text(id, key)`, the way it reads an option with `ctx.option`.

The consequence to know when porting: a silvia node that keeps a *number* in `values` becomes
a node with another input here, not a node with a private field.

**A tick may write one of its node's own values**, through `TickContext::write_value`:
`automation`'s recording, written when the recording stops. It takes the seam
`write_control` takes — this thread's graph now, the document through the command bus a frame
later, as the `SetValue` a hand would have sent — and it is deliberately not a once-a-frame
write, because each one is a step of the undo history. What is drawn *while* a performance is
happening comes the other way instead, on the snapshot, as `CpuNode::curve`.

**A number a tick moves every frame is not written back at all.** `xypad` keeps silvia's `x`,
`y`, `vx` and `vy` as four hidden controls, which is where the hand, a preset or a knob puts
the puck; the tick takes each of them on the tick the document changes it and flies the puck
from there in its own state, so the flight is runtime and the controls say where it was last
put. The flight reaches the pad on the snapshot as `CpuNode::puck`, and what a hand does on
the pad that is not a control — a well dropped, a preset's wells — reaches the tick as
`TickContext::touches`, one-shot as a seek is. A hand holding the puck is a level in `held`,
which `TickContext::pressed` reads as it reads a finger on a button.

**Runtime state is not addressable at all.** Nothing reads another node's `CpuNode`. What a
CPU node wants to publish it publishes through a port, which is the same rule GPU nodes obey.

## Lifetime

`CpuDef::create` builds the state the first time the node ticks, and it is dropped the first
tick after the node leaves the graph. Nothing else owns it, so a camera's pipeline and a
microphone's device stream close by the node being deleted.

Node ids restart at 1 in every project, so **opening a project, or starting a new one, drops
every state there is**, with everything published, every world on the GPU and every Output's
frames — rather than handing a `video` node's player to the `audioin` node that now holds
id 3, or a playing transport to the `automation` that now holds its id. An undo keeps all of
it, because the graph it restores is the same patch; see
[architecture.md](architecture.md#saving-and-opening).

`CpuNode::reset` puts the state back where `create` made it while keeping what was expensive
to acquire — the device, the decoder, the decoded file — so an offline render can start every
node from a known place. A render resets in place only a device's node (`CpuDef::live`) and
one whose `CpuNode::reset_in_place` says it holds such a thing and nothing a render disturbs
but where it is in time — `video`, `imagegif`, `text`; every other node's live instance it sets
aside and runs a fresh one, and hands the live one back when it ends. `Synth::reset` does it for the graph, forgetting every published
uniform and event with it; `tests/reset.rs` holds every CPU node to publishing, after
a reset, what a fresh instance publishes.

Two facts about the state are registry data, on `CpuDef`, and both are for that render.
**`integrates`**: the tick sums `dt` or the transport's advance into its state — a gear's
cycles, a slew, an envelope, a game — so the state at frame *n* is a function of every frame
before it and can be run but never seeked. **`live`**: the tick reads a device — a camera, a microphone, the Main Input's
capture — so stepped to an arbitrary `t` it can only hand back what the wall clock gave it.
A registry test holds both to the source: a tick that reads `ctx.dt` or `ctx.elapsed` is one
or the other, and one that claims to integrate reads `dt` or the transport.

## Where a node lives

Every `Box<dyn CpuNode>` is created, ticked and dropped on the **synth thread**, and never
crosses to another. That is what lets the trait stay `!Send`, which it has to be: a cpal
stream is not `Send`, and neither is a GStreamer pipeline handle. Nothing moves — state is
created on the first tick after a node appears and dropped on the first tick after it is gone,
so the node's whole life is on the thread that ticks it. The Main Input's capture and analysis
are there for the same reason: they are the running state of one source.

What the *editor* needs from a device — the list of cameras, the audio devices — is gathered
on the synth thread too and reaches the panel in the snapshot, refreshed only on *Look for
devices*.

## The tick

`Synth::tick` runs once per tick of the synth's own clock, **on the synth thread** — see
[architecture.md](architecture.md#one-clock-on-a-thread-of-its-own) — walking its own graph's
`tick_order()` so a producer is ahead of its consumers within the same frame. That order is
`topological_order()` **plus action edges**: an event is delivered inside the tick that fired
it, so a node receiving one has to run after the node firing it, or the event half would gain
a frame of latency per hop. Action edges are not cycle-checked, so a loop among them is ordered
as a feedback loop is, as one block ahead of everything it fires, and broken inside at its
lowest id that reads no value of this frame from the loop; the node the break lands on sees
its event one frame late — which is what a loop means. Nothing else may see an action edge:
the compiler does not walk them and `downstream_outputs` must not mark a recompile for one.

**Two things get evaluated in that walk.** A node with a `cpu` half has its `tick` run, which
is most of this page. A node with **no** `cpu` half but a
[dual output](nodes.md#dual-outputs) in diamond mode has that output's `eval` run instead —
its WGSL body in Rust — and the result published on the port, with a dual output that has
flipped to a field withdrawn. So a `math` node fed by knobs and diamonds is CPU arithmetic
with no state of its own, and a chain of them resolves inside one frame because producers come
first in this order like anything else.

Each node's half gets a `TickContext`, which is the entire surface between a CPU node and the
rest of the app:

- `ctx.input(id, key)` — the value of a `UniformNumber` input: what the producer published this
  frame, or the input's own control if nothing is connected.
- `ctx.color(id, key)` — the same resolution for a `UniformColor` input: the producer's
  published color, or the input's own swatch.
- `ctx.connected(id, key)` — whether anything is plugged into an input, for a node whose
  behavior depends on it: a Ratio Gear counts ambient seconds until a clock is cabled into
  its Clock In, and a sequencer ignores its Time while something is cabled into Step.
- `ctx.option(id, key)` — the current value of any option, whatever its
  [kind](nodes.md#option-kinds).
- `ctx.text(id, key)` — the text one of the node's own
  [values](nodes.md#a-nodes-own-values) holds. `text`'s words are the one there is.
- `ctx.path(id, key)` — where an `Asset` option points, as a path. The option holds a
  reference — `assets/gumbasia.webm` for a file in the project, an absolute path for one
  somebody typed — and this resolves it through the project. `None` where the option is
  empty. A node asks for a path and never learns where the project is.
- `ctx.cache_dir()` — where the files derived from the project's media go: a transcode, a
  decoded soundtrack. Inside the project folder, so it travels with it.
- `ctx.cycle(id)` — where a node that moves with time is this tick, in its own cycles and
  unwrapped, its Offset added: in Loop mode what is cabled into its Time, read as a count, or
  the playhead at its rate; in Free mode its own playhead at its pace, which the synth
  integrated from its Speed. `ctx.cycle_at(id, rate)` is the same at a rate the node works out
  itself, and `ctx.runs_free(id)` says which mode it is in. With nothing cabled into its
  Time, either publishes the reading before Offset as a count under the Time key, which the
  loop meter on the row reads. See [ambient time](#ambient-time).
- `ctx.count(id, key)` — what arrives at an input read as a **count**, in `f64`: a count its
  source published whole, unbounded, or else the one `f32` `ctx.input` reads.
  `ctx.counted(id, key)` says which. `ctx.publish_count(id, port, count, one)` publishes a
  count: `count` for a Time, and `one`, the `f32` everything else reads. See
  [gears](#gears).
- `ctx.wraps_at(id, key)` — where the clock cabled into an input wraps as `ctx.count` reads
  it: never for a count published whole, and otherwise the feeding output's
  `OutputDef::wraps_at`, 1 for a Phase, 2520 for anything else, a count's one `f32` through a
  Math node among them (−1260 up to 1260). Where a Ratio Gear unwraps its Clock In to find the
  whole cycles its output passed in a frame.
- `ctx.jump(id, key)` — one of the node's number outputs was put where it is this frame
  rather than moved there; `ctx.jumped(id, key)` asks it of the output cabled into an input,
  which ticked first. A gear says it of its three readings on a Reset, on a change of a Ratio
  Gear's Teeth and on a birth the transport did not make, so a gear counting one takes the
  step as a jump and not as a clock running backwards. Cleared at the top of each tick, as an event is.
- `ctx.time` — what this node sees of the [transport](#the-transport): the playhead, the
  advance since **this node** last ticked, whether a jump is inside it, and the play bit.
  What a Master [Gear](#gears) integrates.
- `ctx.dt` — a **stateful** node's step: the transport's advance for this node, clamped to
  `transport::MAX_DT` (100 ms) live, and zero on a jump and while paused. What a
  simulation, a slew, an envelope and a pad's physics integrate, and what an event's moment
  is measured in: an `Event::at` is in `0..dt`, a fraction of the way
  through the advance. `ctx.fraction(at)` and `ctx.moment(fraction)` go between the two.
- `ctx.elapsed` — the one clock's true time since the first tick: the wall's, for dating a
  trace or timing a tap, never for integrating. Under an offline render it is the frame's own
  `(i − warmup) / fps`. See [architecture.md](architecture.md#one-clock-on-a-thread-of-its-own).
- `ctx.write_control(id, key, f32)` — move one of the node's **own** number controls, as a
  hand on the knob would. The one thing a tick says about the *document* rather than about
  this frame: a number someone has to be able to see, nudge and save belongs on the knob
  rather than in a field the node keeps to itself. `slimemold`'s `Randomize` says it: it rolls
  three knobs to silvia's ranges, and a roll is a look someone found and wants to nudge from.
  A clip's scrubber is the other: it moves Offset, so `Time + Offset` lands where the hand dropped it
  and Time plays on from there. The write takes the seam a bound MIDI knob crosses — the synth's
  own graph on the tick it was asked for, and the document through the command bus a frame
  later, where it is the `SetControl` a hand would have sent and therefore one undo step. It
  is fitted to the control's own range. Everything else a tick has to say goes out through a
  port.
- `ctx.publish(id, key, f32)` — a `UniformNumber` output's value for this frame. The canvas
  draws it on that output's row, and a control fed by it draws it too — see
  [ui.md](ui.md#the-value-on-the-row).
- `ctx.publish_color(id, key, [f32; 4])` — the same for a `UniformColor` output. The row
  draws a swatch where a number would print, and every color control fed by it draws the
  arriving color.
- `ctx.publish_frame(id, Arc<Frame>)` — the node's texture for this frame. The renderer
  uploads it and skips an `Arc` it has already seen, so republishing the same frame costs
  nothing. It is also what the node can show *itself*: a `Texture` output is addressable by
  port, so declaring a `nodes::Region::Preview(port)` region draws it on the node's own
  body under a **Preview** heading and costs nothing beyond the upload that already happened —
  `cellularautomata`'s `cells` and `brickgame`'s `field` are the two simulations that take it,
  because the state of a world and the state of a game are the things worth watching.
  See [ui.md](ui.md#preview-and-on-node-render).
- `ctx.publish_sim(id, port, Simulation)` — this tick of a world the node steps on the GPU:
  its shape, the numbers its kernels read, and the passes to run, appended to any the renderer
  has not run yet so no step is lost between two frame jobs. See
  [a world on the GPU](#a-world-on-the-gpu).
- `ctx.main_input()` — what the [Main Input panel](media.md#the-main-input) is holding this
  frame: the picture, the analysis, the rate it was made at, the panel's gain, and the
  thresholds it crossed. One `MainInputFeed`, assembled once by the app and handed to every
  `maininput` node, which is what makes eight of them agree about one signal.
- `ctx.pointer(surface)` — where the pointer is over one of three picture surfaces, in that
  picture's own world units, or `None` where it is not over it. Assembled once a tick from
  one slot the pointer's two sources write into — a picture window's own `wl_pointer` on the
  pictures thread, and egui's pointer for the preview and the canvas — so two `mouseinput`
  nodes framed the same way agree about where the hand is. See
  [media.md](media.md#the-pointer).
- `ctx.readback(id)` — what the node's tap slot held after the last frame the GPU finished.
  `None` in a headless app. See [taps](#taps-the-other-direction).
- `ctx.edges(id, key)` — every event that arrived on an `Action` input this frame, oldest
  first. Gathered across sources, because an action input takes many.
- `ctx.downs(id, key, &mut gate)` — how many of those were `Down`, counting the hand on the
  button as one more source. A node with a Start/Stop of its own — `animation` — flips on an
  odd count, so a press flips once wherever it came from, and the `Gate` it hands in is the one
  place a held button becomes an edge.
- `ctx.last_down(id, key, &mut gate)` — the moment inside this frame an action input last went
  down, the hand counted as a down at the top of the frame: where a node with a start of its
  own restarts, as `animation`'s Restart does.
- `ctx.fire(id, key, edge)` / `ctx.fire_at(id, key, edge, at)` — an event on an `Action`
  output, at the top of the frame or at a known moment inside it. See
  [the event half](#the-event-half).
- `ctx.pressed(id, key)` — is a hand holding the button on this action input. Unlike a number
  control it does not go inert when something is connected: an action input takes many
  sources and the hand is one more.

And one thing on the compile side rather than the tick: a generator may call
`ctx.own_uniform(node, port)` to read a uniform number its own tick publishes, as the same
uniform a connected one would be. `autoexposure` multiplies by the gain it computed
that way.

What a node has to say about itself goes the other way, through `CpuNode` methods the app
polls: `error()` for the status line, `status()` and `progress()` for the node's own body —
a transcode's bar, or the line `widgets::status` draws for a node that declares one, which is
where `clockdivider` counts down to its next pass — `debug()` for the Status box, `trace()`
for the picture of what it has been publishing, `curve()` for the recording a transport is
performing right now — `automation`'s, which the region draws over the saved curve the `Node`
already carries — and `caption()` for the cells under that
picture: `(key, value)` pairs drawn by `widgets::trace::CAPTION`, which is `adsr`'s gate and
its stage and silvia's own pair of readouts under an envelope — and `gear()` for a gear's
reading, which its region turns by ([gears](#gears)). Each is gathered only while something is
drawing it. `waiting()` is the one a render asks: a clip whose decoder has not yet
delivered the frame its position names says so, and the render holds its frame. `App::node_status` is that line from the outside.

A node that publishes nothing this frame keeps its last published value. That is what makes a
dropped frame or a device that has not delivered yet a hold rather than a gap.

**A node on no open workspace does not tick at all.** It is suspended — see
[architecture.md](architecture.md#open-and-closed-and-what-suspension-means) — and the same
rule covers it: **a uniform number published by a suspended producer and read by a running
consumer holds its last value.** Nothing is cleared, because the node is still in the graph and
so is what it published; a suspended producer looks to its consumer exactly like a device that
has not delivered a new number yet. Its `CpuNode` state is kept too, so closing a tab does not
close a microphone. Its time is not suspended: the tick it wakes on hands it the whole
distance the transport moved while it slept, as a jump, so a gear on it is born again where
the playhead puts it and a node on ambient time reads the playhead as it is —
[the transport](#the-transport).

## The transport

**Ambient time is the playhead.** `crate::transport` is a playhead `T` in seconds over the one
clock, with a play bit. It reads `T = T_anchor + playing × (elapsed − elapsed_anchor)` and
re-anchors on every play, pause and seek. It plays, pauses and seeks, and does nothing else:
there is no speed dial and no loop, because a rate is a free-running node's Speed or a
[gear's](#gears) and a loop is a property of a gear chain. Beside the playhead it keeps **travel**, the total distance the
playhead has moved, carrying a seek's jump as a jump, and a count of seeks, so a node can tell
a jump inside its advance from a motion.

**Every node's advance is its own.** The synth keeps where each node was on the transport
when it last ticked (`transport::Seen`), and hands it `ctx.time` measured from there:

- a node ticking every tick gets that tick's motion;
- on a seek it is the jump's distance, with `jumped` set, and `landed` is where the seek put
  the playhead, before the rest of the tick's motion;
- paused it is zero, and an Output in a feedback loop stops drawing, so the loop stands still
  too ([rendering.md](rendering.md#which-outputs-draw));
- a node waking on a reopened tab gets the whole distance it slept through, as a jump — a gear
  in it is born again where the playhead puts it, so it lands where a tab left open has it,
  and it fires nothing and steps no simulation across the gap;
- a node ticking for the first time gets this tick's motion, as a node made a frame ago would.

The advance is what a gear integrates, what a free-running node's Speed integrates
([ambient time](#ambient-time)) and what a stateful node steps. A looping node on ambient
time needs none of it: it reads the playhead.

**What moves it.** `App::transport(Command)` — `Play`, `Pause`, `Seek(t)` — is a hand on the
instrument rather than an edit: it never enters the undo history. It crosses as
`Msg::Transport` and lands at the top of the next tick. **Nothing of the transport is saved.**
Open and New play from zero (`App::replace_project` sends a `Play` and a `Seek(0)`). The
editor reads the
playhead back as `Snapshot::transport`, or `App::transport_state` — a `transport::Report` of
the playhead, the play bit and whether a render owns the playhead.

**Where a hand moves it** is the time readout, `ui/timecode.rs`: the playhead as `mm:ss.ff`
at a fixed width, a pause button that wears a play glyph while paused, and a reset that is a
seek to zero. It sits at the right end of the egui menu bar, immediately left of the
frame-rate meter, and at the end of the tab row beside the meter where the menu bar is the
operating system's. View ▸ Time shows and hides it, as the `show_time` preference, on by
default, and `F8` pauses and plays whatever is showing; `Space` does not, since a stray one
would stop the show. It is disabled while a render owns
the playhead — [ui.md](ui.md#the-time-readout).

**What a live tick does.** A wall tick moves the clock and the transport follows it; a
`Beat::Delta` from a test or the inline host moves both by the `dt` it is handed. A stall is
the whole of its length in every advance and at most `transport::MAX_DT`, 100 ms, of a
stateful node's step. `u_time` is the playhead's fraction of a second, which is all a shader
reads of it — a measurement's jitter — at the same precision at every second of the show; it
holds while paused and jumps on a seek.

**What a render does.** It seeks to its first frame, `T = −warm / fps`
(`Transport::start_render`), and drives the playhead to each frame's `(i − warm) / fps`
(`Transport::drive`), so each frame's advance is exactly the frame and a stateful node's step
is unclamped; `Beat::At(t)` is the same step for a test with no GPU. While it runs the clock is
not followed. Every node starts the render from scratch — a fresh instance in place of its
live one, or a device's reset where it is — and the first frame is a seek, so every gear
is born again where the playhead puts it — at playhead zero every Master Gear is at the start
of its cycle — every free-running node is born again at its Speed times that moment, and
everything on ambient time reads the frame's own moment. **A render waits
for its clips.** Live, a `video` and the Main Input's clip show whatever frame their decoder
has delivered, a tick late after a jump; a render asks every node's `CpuNode::waiting` and the
Main Input's after it steps a frame, and while one is waiting it holds the frame — the Output
is not drawn and the transport does not move, and only what is waiting ticks again, with no
advance, until the frame it asked for has arrived or five seconds have passed. So a render of
a clip is the same film twice, whatever live play left behind, and nothing else takes a second
step while it waits. A frame held for a clip is not counted against the Output, and neither is
a tick spent waiting on a full writer. **When the render ends, the live show comes back as the
render found it**: the transport whole, its playhead, travel and seeks (`Transport::resume`),
every node's live instance the render set aside, every free-running node's playhead, and where
each node last read the transport, so no node sees a jump — a held gear is held where it was, an XY Pad keeps its wells, a
simulation and a hand-started animation run on. What the live show reads next off the GPU is
set aside with them (`Renderer::park_live`): a simulation's world, which the render grows
afresh from the node's seed, so `slimemold`'s agents, field and picture come back where the
render found them; and every Output's latest frame, copied before the render blanks or draws
over it and copied back after, so a feedback loop builds on the frame it left, Star Gate's and
the camcorder's included. The frames the CPU nodes published and what the taps last measured
come back with them. The mix holds nothing of its own: it is drawn from the decks' frames,
which are back. A Pause or Play pressed during the render is kept.

## Ambient time

**A node that moves with time declares it once and keeps no state for it**: its
`NodeDef::timing`, a `nodes::Timing` — a rate at rest and a pace, in its own cycles a second,
a period by its options, and one axis or two — from which `nodes::timing` expands its rows,
its Timing heading and its mode, `clockMode`. The module doc there is the one place the rules
sit beside the code; [nodes.md](nodes.md#timing) has them from the shader's side, with each
kind's rate and pace.

**Loop mode**: the first row is **Time**, `nodes::TIME` (key `clock`), a diamond with no knob,
in the node's own cycles. Unplugged it is ambient time, the playhead at the node's pace: for a
node that draws, with no `cpu` half, the synth writes `playhead × pace`, from the `f64`
playhead, as a count ([gears](#gears)) under `(node, "clock")` at the node's place in each
tick's walk. Plugged, what arrives replaces it, usually a gear's Cycles. Time is a
`UniformNumber`, so a field cabled into it is an ordinary type mismatch.

**Free mode**, the default: the first row is **Speed** (key `speed`), a uniform number with a
knob and a port, a multiple of the node's pace. **The synth integrates it**, the one place a
node's time is kept: each tick, for each free-running node in its walk, it reads the Speed —
the knob, or what is cabled in — and steps that node's `nodes::timing::Pace` by its own
advance (`Transport::time_since` from where the node was last seen), a `phasor::Phasor` per
axis with a 50 ms closed-form glide. So the playhead it keeps pauses with the show, moves by
`speed × the jump` on a seek, catches up a closed tab's gap, and is born at `speed × playhead`
— a render starts every one again with the rest of the nodes, and puts the live ones back
after. For a node that draws, the synth writes `playhead × pace` under `(node, "clock")`
exactly where Loop mode writes its ambient reading, so the compiler, which reads an unplugged
Time as that published count, `vec2f(whole, fraction)`, cannot tell the two modes apart.

A body takes its Time round its period through the prelude's helpers — `time_periodic`,
`time_repeat`, `time_cells` and `time_unbounded` — by reducing the whole part first, adding the fraction, then
adding **Offset**, `nodes::timing::OFFSET` (key `phaseOffset`), in the node's cycles, added in
both modes; its knob reaches one period either way, −P to P (`nodes::timing::range`). Offset is
a varying input on a node that draws, where a field makes a ripple, and a uniform number on a
CPU node.

**A CPU node reads where it is through `TickContext::cycle(id)`**, its Offset added: in Loop
mode what is cabled into its Time, read as a count in `f64`, or the playhead times its rate;
in Free mode its own playhead times its pace, handed to the tick by the synth
(`TickContext::running_free`). `ctx.cycle_at(id, rate)` is the same at a rate the node works
out itself — a clip's one play over its own length, which is its pace too. `ctx.runs_free(id)`
says which mode it is in. With nothing cabled into its Time, either publishes the reading
before Offset under the Time key, as the synth does for a node that draws, so the loop meter
on the Time row ([ui.md](ui.md#the-loop-meter)) reads a CPU node as it reads any other.

### The oscillator, the sequencers and the clips

Each is a function of `Time + Offset`. The most any of them keeps is last tick's reading, to
see what it crossed: an edge detector, not an accumulator.

- **`oscillator`** is a wave at `ctx.cycle`, one wave a second at rest and at Speed 1,
  silvia's 1 Hz. A frequency is its Speed, or a gear's Teeth in Loop mode, where a stop and a
  restart are the Master Gear's Hold and Reset, and a one-shot is `animation`'s. The Noise waveform draws one value a cycle, keyed by `floor(Time + Offset)`
  and the node's id, so it holds for a wave and is the same twice; it never comes back, so its
  period is none.
  **Level** is the level added to the wave.
- **`stepsequencer` and `euclideanrhythm`** read `ctx.cycle` in bars, sixteen steps a bar,
  through one reading, `nodes::sequencer`, under two patterns. A new one stands still, its Speed
  at 0; Speed 1 is a bar every two seconds, and in Loop mode a Master Gear a bar long is the
  tempo, its Hold and Reset the play and the reset. A cabled Time is read through `ctx.count`: a count whole, in `f64`,
  and anything else unwrapped where its source declares its wrap, so a gear's Phase passing
  one, or a count's one `f32` through a Math node stepping from 1260 to −1260, is a frame's
  motion; a jump, or a cable plugged in, let go or moved onto another
  output, puts the reading back at what the source publishes. **A step fires on each crossing of
  `floor(16 × cycle)`**, stamped with its moment inside the frame, and each lane
  reads the absolute step modulo its own length, so a lane of five against sixteen keeps its
  phase. **A reading that moves more than a bar in one tick, across a seek or onto another
  clock's cable is a jump**, and so is the first reading, a render's start among them, and one
  that moves backwards on a clock: it fires nothing on the way and closes every open gate.
  Running free, a negative Speed plays the steps backwards — each boundary crossed going down
  enters the step below it, which opens its lanes there and closes them a gate length further
  down. A step it lands exactly on, with a gear driving Time or a Speed moving it and the show
  playing, has been crossed by nobody
  and plays at once, so a render's first frame is its bar's downbeat; any other step it landed
  in plays on the next tick where it landed no further past it than that tick moves, so a
  Master Gear's Reset lands on the downbeat. A Time that stands still —
  a gear held, the show paused, the node at rest — closes every gate a step opened as the
  Time passed it; what a landing opened on the step it stands on stays open until the Time
  moves on, so a render's Hold warm-up, which lands on step 0 on its first frame and stands
  there, shows step 0's gate open on the first kept frame, as Black and Run do. **Step is the
  one stateful path**: each down on it, a hand's or a cable's, advances the grid by one step,
  and while something is cabled into Step the sequencer ignores its Time, because an event
  clock — a tap, a threshold — is not a gear. Gate is how much of a step a lane's gate stays
  open.
- **`video` and `imagegif`** read where they are in plays, `ctx.cycle_at(id, 1 ÷ length)`, so
  at rest and at Speed 1 a clip plays at its native speed and a GIF at its own delays. Offset is added, 1 a play
  and its knob −1 to 1; `video`'s Loop option wraps the sum and Hold clamps it to one play, and
  a GIF wraps. The frame is a function of the sum, so the node keeps no position: a hand on the
  scrubber writes Offset through `write_control`, so the sum lands where it was dropped and
  Time plays on from there. Reverse is Speed −1, and twice as fast Speed 2 or, in Loop mode, a
  Ratio Gear at 2 : 1. A
  render still waits for a clip's frame — [media.md](media.md#video-files).

The Main Input's clip is not a node and has no Time: it plays at its own speed on the
transport's advance — [media.md](media.md#the-main-input).

### Stateful nodes step on `dt`

A simulation, an envelope, a slew, a pad's physics and a game are not functions of a moment:
their state at frame *n* is every frame before it. **Time does not drive them.** Each keeps a
rate and steps on `ctx.dt`, the transport's advance clamped to `MAX_DT` live and zero on a
jump, so a pause holds it and a seek carries it across unchanged. What its pace is, its label
says:

- **`slimemold`'s and `cellularautomata`'s Rate**, in ×30/s, is silvia's 30 Hz batches **owed
  to `dt`**: the tick adds `dt × Rate × 30` to what it owes, pays the whole steps on the tick
  they fall due and carries the fraction, so a second is Rate × 30 generations at any tick
  rate, and a driven `dt` makes the same world as a live one. An automaton with Auto-Run off
  owes nothing, and each press of its Step is one generation more.
- **`smoothcounter`'s Rate**, **`autoexposure`'s Response**, a time constant, and
  **`stargate`'s Drift**, in pixels per drawn frame.
- `animation`, `automation`, `adsr`, `slew`, `autogain`, `brickgame`, `xypad`, `clockdivider`,
  `randomfire` and an Output's feedback keep their own. `animation` and `automation`'s playback
  are started by a hand or an event, so each keeps its own start, on a `nodes::phasor::Phasor`:
  a rate integrated in `f64` against the transport's advance with an exact glide,
  `ΔΦ = S·Δ + (s₀ − S)·τ·(1 − e^(−Δ/τ))` per step, so a stepped knob lands on the same phase at
  any frame rate, and it pauses with the show. A seek carries both across where they stood:
  they integrate `Time::carried`, the advance with a jump's distance taken out, so the readout's
  reset leaves a running animation where it was rather than running its pass backwards by the
  jump. The Main Input's clip is the third, and moves by the jump, since it is a source that is
  on — [media.md](media.md#the-main-input).
- `clock`, the time of day, is `live` and outside the model: its reading is the world's.

## Gears

**Gears hold the shared time state.** A rate several nodes keep time by is set, changed or
divided in a gear, and everything a gear drives is a function of what it publishes; a
free-running node's own Speed is the one other rate, and turns that node alone. `nodes::gear`
holds the two, in the Nodes menu's **Gears** between Control and Output (`Category::Gear`),
beside the Time node.

**The Master Gear** (`mastergear`) is the show's clock at a length. Its Length is in seconds,
two by default, a bar at 120 BPM. It integrates the playhead's advance over one cycle's
seconds in `f64`, so a Length turned bends from where the gear is and never jumps. It is
**born at `playhead ÷ length`**: two Master Gears of one length agree however late the second
was made, and at playhead zero every Master Gear is at the start of its cycle. Gate is the
Trigger's length, as a fraction of a cycle. A beat is a Master Gear a beat long, and a bar a
Ratio Gear at ÷4 below it.

**The Ratio Gear** (`ratiogear`) is **a pure product of its parent**: `parent × p ÷ q`,
worked out in `f64` each tick from what arrives at Clock In — a count read whole
(`ctx.count`), anything else the one `f32` it is, a Phase among them — or from the playhead's
seconds with nothing cabled. Nothing is integrated or carried from one tick to the next, so it
has no position of its own: a seek, a render's warm-up, a relaunch and a reopened tab land it
on the same count to the bit, and its loop is its parent's times `p/q` exactly. Its **Teeth**,
`p : q`, are two whole numbers from 1 to 64 with no port (`nodes::gear::TEETH_P` and
`TEETH_Q`, hidden controls on the Teeth row): it turns `p` times for every `q` turns of its
parent, so 3 : 2 is three turns against two and 2 : 1 twice the parent. They are kept as
typed and reduced only in the arithmetic (`gear::product`, `p ÷ q` in lowest terms), so 2 : 4
and 1 : 2 count the same to the bit and the row still says 2 : 4. **A change of Teeth jumps**:
the output lands at once on the parent times the new `p ÷ q`, where it would be had it always
run at that ratio, and fires nothing for the cycles it went over; the gear says its readings
jumped (`ctx.jump`), so a gear counting it is born again with it. Its Trigger fires at the
whole cycles of its output: the one thing it keeps is the parent's last reading, to find the
cycles a frame passed, unwrapped where the output feeding it declares its wrap
(`OutputDef::wraps_at`, through `ctx.wraps_at`: 1 for a Phase, 2520 for a count's one `f32`
through a Math node; a step over half the wrap is the wrap, `phasor::unwrap_at`). A Phase in
Clock In is a parent like any other: the gear is the Phase times `p ÷ q`, and comes round with
it. Plugging Clock In, pulling it out or moving its cable onto another output is a birth,
since the clock it counts is then another one, and fires nothing for the distance between the
two. **A cabled clock its source says was put where it is** (`ctx.jumped`) — a Master Gear's
Reset above, a gear above born again or its Teeth changed — **or one sent back more than a
cycle in one frame** has jumped too: the gear fires nothing for the cycles it went over, only
one downbeat, as a Reset is a beat, where a whole cycle of its own lies between where it now is
and a frame's motion at the pace the clock was going — so a hand's Reset, which lands at the
top of the frame and runs on, is a beat to every gear counting the clock, whatever its Teeth,
however little of its first cycle the clock had run. The pace is the clock's last step over
the advance it took, with the rounding of the readings it was measured on allowed: an `f64`'s
for a count, an `f32`'s for anything else. A step back of under a cycle that no source calls a
jump is a clock running backwards — an oscillator scrubbing, a number turned down — and is
motion, and forwards a fast gear passes several cycles a frame, which is motion. A gear has no
Hold, no Reset and no way to bend: a layer that should pause or bend runs free on its Speed,
and one that should be placed is placed by its own Offset. See
[decisions.md](decisions.md#a-ratio-gear-is-a-pure-product-of-its-parent).

**Both publish the same four.** **Cycles** is a **count**, published whole
(`ctx.publish_count`), which a Time reads at the same precision at every count forever. A CPU
node reads it in `f64`, unbounded, through `ctx.count` and `ctx.cycle`. A shader reads it as
`vec2f(whole, fraction)` (`compile::UniformProvider::NodeCount`, `phasor::split`): the whole
part wrapped at `phasor::WHOLE_WRAP`, 80640, centered on zero, −40320 up to 40320, and the
`f32` of the fraction, zero within `phasor::REACH` of a whole number. 80640 is twice the least
common multiple of 2520 and 128, so every period a body takes its Time round — one cycle, a
noise's Repeat to 16, Static's to 128 — divides it, and an `f32` holds every
whole number to 2²⁴ exactly: a body reduces the whole part by its period, which is exact, then
adds the fraction and Offset, so a gear a million cycles on draws what it drew near zero, to
the bit. A picture that never repeats can be told apart only round the wrap: a noise at Repeat
Never takes its lattice cell round 80640 and keeps the fraction apart (`time_cells`), so it
comes back there with no seam, Static 56 minutes on at Speed 4; the tunnel at Depth Wrap None
adds the two parts, an `f32` that resolves 2⁻⁸ of a cycle at worst, and jumps where the whole
part wraps, 40320 flights in, fifteen days at Speed 4. **Everything else reads one
`f32`**: a Math node, any input that is not a Time, the row's number. That is the count
wrapped at `phasor::WRAP`, 2520 — the least common multiple of one to ten, so a reader at a
whole ratio, or at one whose denominator is ten or less, passes the wrap with no seam — and
centered on zero, −1260 up to 1260 (`phasor::wrap_count`), so a count near zero is exact and
within `phasor::REACH` of a whole number of wraps it is zero. A count put through a Math node
is that one `f32` from then on, and a noise at Repeat 16 behind one meets a seam once every
2520 cycles. **Phase** (key `wrapped`) is the fraction alone, 0 up to 1. **Ping-pong** is a
triangle over two cycles. **Trigger** is an event on each whole cycle, stamped where inside the
frame it fell (`phasor::Step::crossings`, bounded per frame); a boundary a step ends within
`phasor::REACH`, 10⁻⁹, short of is passed by that step and not again, so a render that lands a
whole cycle exactly on a frame fires it on that frame.

**A Master Gear's Hold** is a toggle that freezes it where it stands and closes the gate it
left open, so nothing downstream is held by a clock that is not moving. **Its Reset** puts it
at the start of a cycle, and is a beat. A frame's Holds and Resets, a hand's and each cable's,
are walked in the order they fell, with the advance between them integrated. **A seek — the
readout's reset among them — a render's start and a tab reopened are jumps: every gear is born
again where the playhead puts it, and fires nothing on the way.** **A gear a seek or a render's
start puts on a whole cycle fires that beat**, to within `phasor::REACH`: a Master Gear where
`landed ÷ length` is whole, a Ratio Gear where its own count is, with the clock's motion since
the landing allowed at the pace it was going, as for a Reset above it. So the readout's reset
is a downbeat on every gear whose count starts there, and a render's first frame is one
whatever the warm-up; a seek that lands mid-cycle, a tab reopened and a cable moved fire
nothing.

**The Teeth row** is `Region::Teeth` (`widgets::gear::TEETH`): the word Teeth and the two
numbers either side of a colon on one row the height of a port row's, each the s-number every
whole-number control is, stepping by one from 1 to 64, so a drag, the steppers, the arrows and a
typed number all land on a whole number of one or more. The tick reads each as the whole
number nearest it, so a stored value no hand could type runs as the nearest whole one
(`gear::teeth`). Hovering the word says `Turns 3 times for every 2 turns of its parent.`

**A gear's picture** is `Region::Gear` (`widgets/gear.rs`), one fixed 92 units tall for either
of its Display option's two: the **Rosette**, the default, a still rosette of `p` petals wound
over `q` loops for Teeth `p : q` in lowest terms, a turn an input cycle, with a dot at the
output's phase; or the **Gears**, meshing gears of `k·p` and `k·q`
teeth, a Master Gear one gear of twelve. Both turn by the gear's own phases, so a
paused show is still. The reading reaches it as `CpuNode::gear`, gathered into
`Snapshot::gears`, and beside the picture are the ratio and what it closes in —
[ui.md](ui.md#the-gear-region).

**Under a Master Gear, a caption says what a loop of it needs**, worked out from the graph by
`nodes::chain`, and **a claim is exact and the shortest that is true, or it is not made**.
`chain::master_loop` walks every node downstream of the master and works out, in the master's
cycles as an exact `chain::Fraction`, what each comes back in: a Ratio Gear multiplies its Clock
In's rate by its Teeth, `p/q`, exactly, and with anything but a count in its Clock In comes
back when that does; a node that moves with time is where its Times and Offsets put it, read
round its period, with what every other input it reads adds; a Trigger into a sequencer's Step
moves it a step a beat, and a Clock Divider divides the beats; a node on its own clock moves at
its rate over the master's seconds, read as a fraction; a node with no CPU half comes back when
its inputs do. The loop is the master's cycles that bring all of them back — the least common
multiple of what each asks for, a gear counted through what reads it and a gear nothing reads
by its own turn — with the one that asks the most, the nodes that never close, and the nodes
past which it cannot tell. `master_seconds` is one cycle and `master_length` a loop, one
cycle's seconds times its cycles, with none where anything never closes or cannot be told.
`chain::caption` reads "loops in 4 cycles · 8.000 s (÷4 on ratiogear12)", "loops in 4 cycles ·
8.000 s (÷4 on perlin5)", "2 nodes will not close (perlin5)" or "can't tell when 1 node closes
(multiply3)"; the node shows it up to its " (" and the whole of it on hover.

**When a loop closes.** A node on a chain from a Master Gear `M` advances `m × Πr ÷ P` of its
own periods over `m` cycles of `M`, where `Πr` is the chain's product and `P` the node's period
as the graph gives it (`timing::period_in`) — one for a periodic node and a looping clip, `N`
under a noise's or Static's Repeat, a quarter on the tunnel's Helix, a sequencer's lanes'
figures coming round together, a quarter of a bar for four on the floor, and none for a clip
on Hold or the oscillator's Noise — and it closes when that is whole; a period may be any
positive fraction, and is read as one. A gear's Phase in a Time comes back every cycle of that
gear, or every `P ÷ Πr` where `P` divides one, and a sequencer reads it unwrapped as a count; a
Ping-pong comes back every two. A node on its own clock closes over a length `L` when its rate
— in Loop mode with nothing in Time — or its Speed times its pace — running free with nothing
in Speed — times `L ÷ P` is whole (`chain::closes_alone`), and beside a master it closes where
that length is a whole number of the master's cycles; a clip's own rate is a length the graph
does not hold. A clock cabled into a Speed never closes, and the caption counts it. An
unconnected color input falls back to the hue wheel, which stands still, so it closes on any
loop. [nodes.md](nodes.md#when-a-loop-closes) has every rule. **A loop is rendered by the
Output's ordinary render**, for as long as the caption says; there is no loop export.
`examples/loop_gifs` does it headless for every tab of a project
([testing.md](testing.md#3-the-gpu--the-actual-pixels)).

**The Time node** (`time`), in Gears too, is ambient time as a number, with no inputs:
Seconds, the playhead, published as a count, so it **counts on**. A Time reads it whole — a
CPU node the `f64` playhead, a shader its whole part wrapped at 80640 and its fraction —
and its one `f32`, what a Math node reads, is the playhead unwrapped, which resolves a
millisecond for the first two hours of a show and a sixtieth of a second for the first 36.

## The event half

`Action` is the port type that carries an event rather than a value. Two rules decide
everything about it, both argued in [decisions.md](decisions.md#an-action-is-a-gate-not-a-pulse).

**An action is a gate, not a pulse.** An edge fires `Down` when a condition becomes true and
`Up` when it stops. A receiver gets both, which is what lets an envelope be held rather than
retriggered, a note be released, and a band say when it *stopped* being loud. A node whose
source is a *level* rather than an event holds a `Gate`, which is the one place the transition
rule lives.

**An event carries when it happened.** `Event` is an edge plus `at`, seconds into the frame
that delivered it. A frame is 16 ms and a beat is not obliged to land on one, so a source with
a phase of its own — a clock, or an analyzer running at 48 kHz — says where the crossing
actually fell, and an integrator advances *in segments between the events* rather than in one
lump per frame. `adsr` is written that way and `tests/actions.rs` asserts it: a gate that opens
three quarters of the way through a frame gets a quarter of a frame of attack. Zero means *as
far as this source knows, now*, which is what a hand on a button means.

Three consequences worth stating:

- Events live for **exactly one frame**. The synth's `actions` are cleared at the top of each tick, so
  a consumer that wants to remember something remembers it itself.
- Several events on one port in one frame are several events. A clock whose subdivision is
  faster than the display emits all of its beats, in order, each stamped; a counter that
  collapsed them would drift against the clock driving it.
- An event never reaches a shader. It is a CPU thing with no uniform and no WGSL, and the one
  door out of the event half is a node that publishes a `UniformNumber` — `counter` and
  `adsr` are that door, and a uniform number feeds a varying number input for free.

What sub-frame time does **not** buy is a picture faster than a frame: a uniform number is
sampled into a uniform once per frame however precisely it was computed. It removes
jitter and fixes phase; it does not make a strobe faster than the display visible.

An action input may declare `Control::Press`, which draws a momentary button in its row —
silvia's rule that the control which fires the event sits on the port that carries it. The
canvas reports it as a level in `Effects::held` and the app turns held-and-then-not into
a down and an up. It is not a `Command`: pressing a button is playing the instrument, not
editing the graph, so it never enters the undo history.

## What the type system enforces

Three registry tests, so none of this is a person remembering:

- A `UniformNumber` port and an `OutputKind::Uniform` say the same thing and must agree, and
  so do an `Action` port and an `OutputKind::Action`. Either with a `Shader` kind would be asked
  for a function it cannot write.
- A `Control::Press` belongs to an `Action` input and nowhere else: a number input already has
  a control and a color input already has a swatch.
- A node publishing a uniform — a number or a color — or a texture without being an Output,
  has a `cpu` half
  — and a node with a `cpu` half has something to publish. Either alone is a definition that can
  never carry a value.
- **Every input of a CPU-only node is a CPU type.** `tick` can hold a uniform number and not
  a field: there is no `uv` outside a shader. So a CPU node's inputs are `UniformNumber`,
  `UniformColor` or `Action`, never `VaryingNumber` or `VaryingColor` — unless the node also
  has a `Shader` output, because then that output is what reads the field. A tap is such a node.

That last one is the rule people trip on. A `slew` that smooths a value coming out of a
picture is not spelled by relaxing it; it is spelled by a `tap` on the picture and the slew
on the tap's `mean`.

## Taps: the other direction

A tap is a node with both halves. Its WGSL returns its input unchanged; the measuring is a
second thing the definition declares, `NodeDef::measure_wgsl`, and it is a **property of the
node** rather than of one of its outputs. The compiler calls it once, in one module: the
[pass](rendering.md#the-workspace-pass) of the node's workspace. An Output's module and a
probe measure nothing, so a tap cabled into an Output is a pure pass-through in that Output's
shader. The measurement claims a slot with `ctx.tap_slot`, evaluates the node's inputs at the
point it is handed — `ctx.input(node, key, "p")`, which pulls the node's own input chain into
the pass — and registers itself with `ctx.measure_grid` or `ctx.measure_once`; the pass's
`fs_main` then calls it at every cell of an `N`x`N` grid over the unit square, or once at the
corner fragment for a `sample`. The renderer reads the buffer back once the pass's submission
has finished and the node's `tick` reads `ctx.readback(id)` the next frame.

So **a tap measures its input**, over its own domain, one evaluation per point per frame,
whoever draws it and at whatever coordinates. `grid` is the `N` — 32, 64, 128 or 256, default
128 — and `jitter` moves each point inside its own cell by a hash of the cell and the clock,
which is the answer to a pattern finer than the grid. What a `tap` reduces is its `measure`
picker — the eleven `Convert` quantities — or, while a `VaryingNumber` is plugged into its
`number` input, that field instead, which is the field case the registry rule above exempts a
node with both halves for. **What it measures is signed**, and exact to 1/65536: a sample is
stored biased by 32768 so an unsigned word carries sign, and each sum is 64 bits across two
words, so no grid overflows one and no fraction is thrown away to make room. The extremes are
ordered by a monotonic map of the float's bits, which orders negatives too, and the centroid
weights by the positive part of the quantity, so `x` and `y` say where it is positive.

**A measured node is measured once, in its workspace's pass, whatever it reaches.** Cabled into
an Output, into three, or into nothing at all, a tap's reading is the same, because the one
place its measurement runs is the pass, and the pass is context-free: a tap reads while its
output port is still bare, and on a workspace with no Output. `link::plan::measured_on` is the
rule, worked out by every plan built: each awake measured node goes to the pass of the first
of its workspaces in project order that has a tab, and a node awake only because a deck or a
send reads it — its workspaces all closed — to the pass of its first workspace, which then
exists for its measurements alone, with no thumbnails. **Nothing is ranked and nothing is
picked**: a node has one slot, in one pass, and its reading is that slot's. No Output is
chosen to carry it, so a cut, a cable into an Output or one taken out moves no measurement.
A suspended node is measured nowhere: it does not tick, so nothing would read the slot back. **A measurement that samples a frame reads this tick's**, because every pass
is drawn after every Output ([rendering.md](rendering.md#the-order-a-tick-submits-in)), and
when its reading is read, each Output whose frame it samples draws
([rendering.md](rendering.md#which-outputs-draw)). The node's `status()` says *measuring*
where it has a reading (`tap::MEASURING`) and *not measuring* where it has none
(`tap::DORMANT`).

**A tap with no reading says nothing rather than zero.** With no reading — no pass holds a
slot for the node, or none has reported yet — the tick calls `ctx.withdraw(id, port)` on each
of its uniform number outputs instead of publishing, and `ctx.withdraw_color` on a sample's
`color` for the same reason. The rows then draw nothing, under the rule an unpublished port
already follows, because `0.00` is a reading and *nothing measured this* is not. A readback is
kept while the node holds a slot in a pass the plan carries, so a pass that stopped drawing
holds its last value, a pass that did not report replaces nothing, and a node no pass measures
— suspended with its workspace — loses its uniform number. *Not measuring* therefore means what
it says: nothing is running this node's measurement.

**A tap's uniform numbers are delayed ports**, the same way an Output's `frame` is: `mean`,
`max`, `min`, `x` and `y` read last frame's measurement by construction, and so do `sample`'s
five and `autoexposure`'s `gain` and `luma`. That is what lets a cable close a loop through one
— `tap.mean` into an input of a node upstream of the tap, say — where the identical cable out
of an ordinary uniform number would be `WouldCycle`: the reading is a frame old either way, so
it is feedback rather than recursion. See
[architecture.md](architecture.md#delayed-ports-and-feedback). `autoexposure` keeps
`own_uniform` for the one-drop case — a performer wants exposure in one node, not a node and a
cable back onto itself — but its loop no longer has to go through it.

The reasoning, and the render-target and mip-chain designs that lost to it, are in
[decisions.md](decisions.md#going-down-is-a-side-effect-not-a-render-target) and
[decisions.md](decisions.md#a-tap-measures-its-input-over-the-unit-square); the buffer itself
is in [rendering.md](rendering.md#tap-buffers).

## A world on the GPU

`slimemold` steps seven thousand agents over a scent field, and that is too much to do in a
tick: on the CPU it cost the synth thread 3 to 6 ms a batch, and the Status box read 12 ms on
the demo. So **the world lives on the GPU and the tick keeps what has to be on the CPU** —
the one clock, the options, the presses, the seed and the status line. Every tick it
publishes a `nodes::Simulation` through `ctx.publish_sim`, whose passes the renderer runs
before any Output draws; see [rendering.md](rendering.md#simulations) for that half and
[decisions.md](decisions.md#a-simulation-steps-on-the-gpu-in-kernels-its-node-writes) for
the numbers.

The rules are compute kernels the node writes, as a generator writes WGSL — `static`s in
`nodes/slimemold.rs`, a string each, so `nodes/` still takes no graphical dependency. What
the tick decides:

- **How many steps.** silvia's rate, `Rate` × 30 a second, is **owed to the one clock**:
  the tick adds `dt` times the rate to what it owes and pays the whole steps, so the world
  moves every tick, at the same speed whatever the display does, and a driven `dt` makes the
  same world as a live one. Nothing else keeps time.
- **What the world is.** Its size and population, from the two `Runtime` options. The
  first tick is a birth — a clean field, every agent thrown — and a change of either is a
  reshape the renderer does before the passes that follow it.
- **What a press asks for.** Clear, Scatter, and the nudge a Randomize or a preset ends in,
  each a pass or two. Randomize also rolls Sense Angle, Turn Angle and Sense Dist onto the
  knobs through `write_control`; a preset's settings are the preset bar's own edit, and the
  bar hands the tick the nudge as a one-frame press under `slimemold::NUDGE`.
- **The seeds.** Every pass that draws a random number carries a seed from the node's own
  seeded generator, so the same node over the same `dt`s is the same world.

`reset` puts the tick back where `create` made it, and its next tick is a birth, so the world
the renderer holds is born again with it — `tests/gpu_app.rs` holds a reset world equal to
a fresh one on the GPU, and `tests/reset.rs` the tick's half. A headless app has no renderer:
the passes are published and dropped, and the tick's own tests read the `Simulation`.

## The reference node

`slew` is the smallest CPU node there is and the one to copy: three `UniformNumber` inputs, one
`UniformNumber` output, and a `tick` that integrates `ctx.dt`. It holds one `Option<f32>`,
`None` until the first tick so that it snaps to its input rather than ramping up from zero.
Its one `Runtime` option picks between the two smoothings a slide can be — a rate it cannot
exceed, or silvia's exponential approach, which slows as it arrives.

The rest of the CPU-only library is the gears, `mastergear` and `ratiogear` — the only time
state there is ([gears](#gears)) — `time`, the playhead as a number, `oscillator`, a wave at
its Time, `animation`, an envelope a hand or an event starts, on a phasor of its own, `clock`,
the wall clock, whose reading is the world's rather than the graph's and which is therefore
`live`, `automation`, which records a knob moving and plays it back off one of its node's own
values, `button`, `clockdivider`, `counter`, `adsr`, `smoothcounter`, `triggeredrandom`,
`randomfire`, `euclideanrhythm` and `stepsequencer` in the event half — the last two one
reading of Time, `nodes::sequencer`, under two patterns — `autogain`, and the sources in
[media.md](media.md) — `audioin`, `camera`, `video`, `imagegif`, `mouseinput` and `gamepad`. A
node with both halves is a tap, a sample, an `autoexposure`, a `triggeredcolor`, a `muxevent`,
a `cellularautomata`, a `slimemold` or a `brickgame`. The twelve generators and transforms that
move with time — mandelbrot, juliaset, perlin, simplex, fractal, static, cosinegradient,
domainwarp, tunnel3d, rotozoom, shakycam and geissflow — have no CPU half at all: their Time is
the synth's [ambient reading or free-running playhead](#ambient-time), or a cable's.

## Adding one

1. Write the definition with `UniformNumber` inputs and outputs, `OutputKind::Uniform`,
   `no_wgsl` as the generator, and `cpu: Some(CpuDef { create, integrates, live })` — does the
   tick integrate `dt` or the transport, does it read a device. A node that moves with time
   declares `timing`, takes its rows from `nodes::timing::inputs!` and its heading and mode from
   `options!`, and reads `ctx.cycle` — the synth integrates its Speed; a stateful one steps on
   `ctx.dt`. A tap adds a `Shader` output
   that passes its input through and a `measure_wgsl` that claims a slot with
   `ctx.tap_slot(node, kind)`, writes to `tap` and registers itself with `ctx.measure_grid`
   or `ctx.measure_once`.
2. Implement `CpuNode::tick`, and `reset`: back to what `create` made, keeping any device.
3. Add it to `REGISTRY` and give it a `Category`.
4. Add a test to `tests/uniform.rs` that ticks a headless `App` and reads `App::uniform` back.
   A node publishing a texture instead is tested the way `tests/video.rs` does it, and a node
   that fires events the way `tests/actions.rs` does: `App::press` puts a hand on a button and
   `App::edges` reads what a port fired.
