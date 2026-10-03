# Time

**Status: built, on `main`, with one answer changed.** This is the time model: ambient time,
Time and Offset on every moving node, and gears — and since, a second mode beside it. Every
moving node now runs **Free** by default, on a **Speed** knob the synth integrates against the
transport, and this file's design is its **Loop** mode, which a gear's Cycles drive exactly;
answer 2 below, *no moving node has a speed knob*, holds only in Loop mode. The spec carries it
in pieces — [docs/cpu.md](../docs/cpu.md#the-transport) (the transport and the gears),
[docs/nodes.md](../docs/nodes.md#timing) (the two modes and Offset on each node, and
`nodes::timing`), [docs/rendering.md](../docs/rendering.md) (a render) and
[docs/ui.md](../docs/ui.md) (the time readout and the gear region). This file is the whole
argument for Loop mode in one place. What lost, and why, is in
[docs/decisions.md](../docs/decisions.md): *Time is an input port*, *Offset, added, in the
node's own cycles*, *One transport over the one clock*, *Gears: a rate shared, and a loop
closed* and *Two modes, and a loop is read from the clocks*.

## The answers, in short

1. **How time works.** There is one transport, a coordinate system over the one clock: a
   playhead in seconds that plays, pauses and seeks. That playhead is **ambient time**, the
   default time for everything. A node that moves with time reads **Time + Offset** each frame
   and integrates nothing. Its Time is ambient time at a rate the node declares, or whatever
   is cabled in. Its Offset is added.
2. **Rate.** A rate lives only in **gears**. No moving node has a speed knob. A Master Gear is
   the show's clock at a length; a Ratio Gear turns one clock into another at a ratio. Gears
   hold the only time state in the model.
3. **Transport.** One, global. Nothing of it is saved: a project opens playing, at zero.
4. **A type for time.** No. Time is a plain float in cycles of some clock, and seconds are the
   cycles of a 1 Hz clock.
5. **A clip and an oscillator.** One rule drives both: a shape read at `Time + Offset`. A clip
   is an oscillator whose shape is a frame lookup.
6. **Loops.** A loop closes when every clock in it comes from gears with whole ratios of one
   Master Gear. The Master Gear's caption says how long that is, and a loop is the Output's
   ordinary render for that long.

## What it answers

- **A speed turned live jumps the picture.** `u_time × speed` moves by `Δspeed × u_time` when
  the speed moves, and `u_time` grows forever. silvia spends 286 lines of
  `phaseAccumulator.js` smoothing it. Here no node multiplies a time by a speed: a node has no
  speed, a gear's length change bends, and a Ratio Gear's ratio change lands on a whole cycle.
- **`f32` time stalls after hours.** At 144 Hz an `f32` at 131 072 stops moving. Here state is
  `f64` and what reaches a wire or a shader is wrapped.
- **A stall or a closed tab drifts.** Every node's advance is its own since it last ticked, so
  a stall is caught up and a tab reopened after a minute is in phase with one left open.
- **Nothing can be sought, and a render needs a path.** A node is a function of its Time, so
  the frame at a moment is the frame at that moment however the show got there.
- **Almost nothing loops.** A gear chain says whether it closes, and a noise repeats by an
  option.

## What other programs do

**Two layers.** Every mature tool ends up with a few accumulators at the bottom and everything
else a stateless function of them. TouchDesigner's Speed CHOP, MadMapper's `time_base`,
Synesthesia's audio clocks, Hydra's global `time`, Link's beat timeline and Bitwig's device
phase are the accumulators. `sin(2πΦ)`, a frame chosen from a position, a sequencer step read
from a phase are the functions. The jump bug appears exactly where someone multiplies a live
speed by an accumulated time. MadMapper's documentation works it through with numbers.
TouchDesigner's forum answers it by pointing at the Speed CHOP, which one user found
"somewhat misleading" as a name for an integrator. Hydra's `osc(freq, sync)` and Tidal's
`fast k` still have it one level down. Here the accumulators are the gears and everything else
is a function of them.

**Re-anchoring.** Tidal, Ableton Link, SuperCollider's TempoClock and Strudel all keep a
position the same way:

```
position = anchorPosition + rate × (now − anchorTime)
```

When the rate changes they re-anchor: the anchor becomes the current position and time, and
the new rate runs from there. Inside a segment this is `time × speed`, stateless and seekable.
Across segments it is an accumulator, and a rate change bends. Strudel documents the naive
form's failure: with `cycle = time × cps` and a changing cps, "the scheduling might repeat
notes… or jump over notes".

**A global transport with local ones under it.** Notch has an incoming playhead, a global
playhead it advances, and a playhead per layer. TouchDesigner has `absTime` and a Time COMP per
component. Max has a global transport and named ones chosen per object. Live has one transport
and derives each clip's time from its launch. What they share: local time is derived from
global time by an offset, a ratio and a play bit of its own. What makes it work there is that
every node belongs to exactly one component, layer or clip. Here a node can be shown on several
workspaces and cables cross them, so a node has no single "its timeline" to ask. Local time is
a gear.

**Phase as a signal.** Bitwig's Grid makes phase a purple signal from 0 to just under 1, with
a family of phase operators around the type. It buys wrap arithmetic for free and a clock the
eye can see. It costs the cycle count, since a wrapped phase cannot say which bar it is in, and
a second way to say time that users must learn. Bitwig does not refuse a plain number on a
phase input. The convention does the work, not the type.

**Video.** Every tool offers a clip *locked* to time, where the frame is a function of the
position and a scrub moves it, and *free-running* with a rate, which is an accumulator. The good
ones expose the position as a number anything can drive: TouchDesigner's Specify Index,
Resolume's Clip Position, Quartz Composer's external timebase. Max's `wave~` is where oscillator
and sample player meet: a phasor at audio rate makes it a wavetable, at `1/length` a loop
player.

**Loops.** A loop of length `L` closes when every periodic term makes a whole number of cycles
in `L`. Resolume and MadMapper quantize a synced speed to powers of two; Link locks phase to a
quantum. Noise loops by walking a circle through an extra dimension. Feedback that decays
converges to its loop after a pre-roll. A loop must not include the frame at `L`, which is the
frame at zero.

**Render.** The shared rule is to replace the clock with a fixed step and never teleport:
vvvv's Incremental main loop, TouchDesigner with Realtime off, CCapture's virtual clock. p5's
`saveGif` jumping `frameCount` instead of running the frames is the documented failure.

**Precision.** `f32` seconds in a shader degrade to steps of 2 ms after 4.5 hours and 7.8 ms
after a day. The cure every tool that found it chose is an `f64` on the CPU and a wrapped value
on the wire.

## The rules

1. **Ambient time.** There is one clock for the show: the transport's playhead, in seconds,
   `f64`. It is the default time for everything.
2. **Rate lives only in gears.** No node that moves with time has a speed knob. A gear is the
   one place a rate is set, changed or divided.
3. **A time-driven node has two inputs, Time and Offset.**
   - **Time** (key `clock`, `nodes::TIME`) is a diamond with **no knob**, in the node's own
     cycles. Unplugged, it reads ambient time at the node's declared rate. Plugged, what
     arrives **replaces** it, usually a gear's Cycles.
   - **Offset** (key `phaseOffset`, `phasor::OFFSET`) has a small knob: 0 to 1 is one of the
     node's cycles. It wraps, it is **added** every frame, and on a node that draws it may be
     a field.
   - The node computes `Time + Offset` each frame. It integrates nothing.
   - The two are adjacent rows, Time then Offset, folded under a **Time** heading that starts
     closed; a cable into either draws into the closed heading's edge.
4. **Nodes are stateless; gears hold the only state.** A node may remember last tick's reading
   to fire an event on a crossing. That is an edge detector, not an accumulator.
5. **Nothing jumps.** There is no speed to turn. A Ratio Gear's change lands on the next whole
   cycle of its input. A Master Gear's length change bends from where it is. Offset
   moves the picture by exactly what Offset moved. What still moves the picture at once is an
   edit or a hand on the transport: a cable plugged into Time, a Reset, a seek.
6. **Stateful nodes keep a Rate** and step from wherever they are, on the transport's clamped
   `dt`. Time does not drive them.
7. **Events fire at crossings of whole numbers**, stamped with the moment inside the frame. A
   jump fires none on the way: a seek over forty beats does not fire forty gates, only the
   beat it lands on where it lands on one.
8. **A loop closes when every clock in it comes from gears with whole ratios of one master.**

**Why Time is a diamond.** Time says which moment the node is at, one number per node, and the
CPU needs it: for a crossing, a clip's frame, the gear's caption. A per-pixel lag is Offset's
job. So a field cabled into Time is an ordinary type mismatch, and Offset is the circle beside
it.

**Why per-pixel time is added, never a rate.** A rate must be integrated, and a field has no
single number to integrate. Two pixels whose rates differ by `δ` drift apart by `δ × t`: a field
of rates from 0 to 5 across the frame draws 8 stripes after 10 s, 48 after a minute and 955
after 20 minutes, which at 1920 px is noise. An offset field is stable forever: `Φ(x) = Time +
o(x)` moves by exactly `Δo` when the offset moves, never more. A travelling wave, a ripple, a
slit-scan shear and a lag per pixel from any picture are all offsets. This is phase
modulation, and it expresses every per-pixel rate whose deviation integrates to something
bounded, which is every one that does not shred.

## The convention: time is a float in cycles

> **A count is a number in cycles. Its whole part is how many, its fractional part is where in
> the current one.**

Seconds are the cycles of a 1 Hz clock. Beats are the cycles of a Master Gear a beat long, and
bars the cycles of a Ratio Gear at ÷4 below it. So "time" and "phase" are one quantity on the one kind of
wire, and the unit is which clock it counts. Two labels carry it:

- **Cycles** is always the count and the fraction together.
- **Phase** is always the fraction, 0 up to 1.

**An angle is in turns** for the same reason: 1 is a full turn on every angle knob and input,
its unit the glyph `↻`, so a gear's Phase cabled into a Rotation turns it once a cycle with no
snap and no Math node between them.

No new port type carries it. A type would keep the count exact forever and mark a clock's
wire. Here the wrap keeps precision (below), a loop is read from the gear chains, and a clock
is recognisable by its label.

## One transport

The transport (`transport.rs`) is a coordinate system over the one clock, not a second timer.
It holds a playhead `T` in seconds, `f64`, and whether it is playing. It re-anchors the way Link
does:

```
T = T_anchor + playing × (elapsed − elapsed_anchor)
```

A press on play or pause and a seek each set a new anchor. The clock is the same `Clock`, and
nothing else keeps a timer.

Each tick the synth hands every node:

- **the playhead** `T`, which jumps on a seek;
- **the advance**, how far the transport moved since *this node* last ticked. It is the jump's
  distance on a seek, with a flag saying it was one, and zero while paused;
- **`dt`** for a stateful node: the advance clamped to 100 ms (`transport::MAX_DT`), zero on a
  jump and zero while paused.

A node that was asleep on a closed tab gets the whole advance on the tick it wakes, taken as a
jump: its gears land where a tab left open has them, and nothing inside the gap fires. **A gear
read from an open tab does not sleep**: a gear cabled straight into a node on an open tab is
live with everything upstream of it (`link::live_nodes`), as a deck's Output is, so a Master
Gear on a closed tab keeps counting for the noise it drives on an open one. Any other producer
on a closed tab holds its last value.

**Pause** stops the advance. Everything that reads the transport holds, simulations included.
A camera is still a camera, so a paused live set still shows its inputs. **Feedback stands
still**: an Output in a loop draws only on a tick the playhead moved, so trails, Star Gate and
the CRT hold while paused, and a seek while paused draws them once. **Seek** moves the playhead, and every gear is born again where the
playhead puts it. **Nothing is saved.** A project opens playing, at zero.

## The time readout

All a person sees of the transport (`ui/timecode.rs`).

- **Where.** At the top right, immediately left of the frame-rate meter, on the egui menu bar.
  Where the menu bar is the operating system's, it sits at the end of the tab row beside the
  meter.
- **What.** The playhead as `mm:ss.ff`, hundredths of a second, monospace, at a fixed width.
  Past 99 minutes the minutes grow to three digits inside the same reserved width.
- **Controls.** Two buttons, and nothing else: **pause** (a play glyph while paused) and
  **reset to zero**. `Space` is pause's key. A reset is a seek to zero: every gear is born
  again at the start of its cycle and fires that downbeat, and stateful nodes carry across.
- **Toggle.** View ▸ Time, a checkbox beside View ▸ Status box, saved in the preferences. It
  is on by default, since pause lives there.

A hand on the readout is playing, as a deck claim is, and enters no undo history.

## Gears

**A category of their own, Gears**, `Category::Gear`, between Control and Output in the menu
(`nodes::gear`).

**Master Gear** (`mastergear`). The show's own clock at a length.
- **Inputs.** Length in seconds (default 2, a bar at 120 BPM); Reset; Hold (freezes it where
  it is, a toggle); Gate (the Trigger's length, as a fraction of a cycle).
- **Options.** Display.
- **Behaviour.** It integrates the playhead's advance over its length in `f64`, born at
  `playhead ÷ length`. So two Master Gears of one length agree, and at the playhead's zero every
  Master Gear is at the start of its cycle. A length change re-anchors at once: it bends and
  never jumps. There is no tempo and no tap: a beat is a Master Gear a beat long, and a bar a
  Ratio Gear at ÷4 below it.

**Ratio Gear** (`ratiogear`). A clock in, a clock out at a ratio.
- **Inputs.** Clock In (key `clock`), a diamond with no knob: unplugged, it counts ambient
  seconds. Ratio, −64 to 64. Reset. Hold.
- **Ratio.** A drag walks a ladder, ÷16, ÷8, ÷6, ÷4, ÷3, ÷2, ×1, ×2, ×3, ×4, ×6, ×8, ×12, ×16,
  and through ×0 into reverse. Typed, it takes `×5`, `÷7`, `3/2`, `0.3` or a leading minus
  (`-×1`). A MIDI knob sweeps the ladder. Clock In takes any number, on purpose: an oscillator
  or a Ping-pong scrubbing a gear's rosette is a feature.
- **Behaviour.** It integrates `ratio × ΔClock In`: a count read whole, in `f64`, and anything
  else unwrapped where the output feeding it declares its wrap (`OutputDef::wraps_at`: 1 for a
  Phase, 2520 for a count's one `f32` through a Math node; a step over half the wrap is the
  wrap, `phasor::unwrap_at`). **A ratio change lands on the input's
  next whole cycle.** Until then the old ratio runs, and the display shows the new one pending.
  It waits at most one input cycle. Every input cycle after it adds a whole number of output
  cycles, so a chain of whole ratios closes whatever phase the change landed at. Landing keeps
  the output's downbeat on the input's, which is what a person listening expects of a gear
  change; a glide would leave the downbeat wherever the glide ended. The gear is born at the
  ratio times its input's reading, so one born again after a seek, a reopened tab or a render's
  warm-up is where playing would have put it.

**Both publish the same four**, every frame: **Cycles** (the count, published whole — see
[Precision](#precision-f64-state-a-count-published-whole)), **Phase** (key `wrapped`, the fraction alone), **Ping-pong** (a
triangle over two cycles) and **Trigger** (an event on each whole cycle, placed where inside
the frame it fell).

**Hold, Reset and a seek.** Hold is a toggle that freezes the gear where it stands and closes
any gate it left open. Reset puts it at the start of a cycle, and is a beat. A seek — the
readout's reset among them — and a render's start are jumps: every gear is born again where the
playhead puts it, and fires nothing on the way. **A gear born on a whole cycle fires that
beat**: a Master Gear born where the playhead ÷ its length is whole, and a Ratio Gear whose own
count lands on one, so the readout's reset and a render's first frame are a downbeat whatever
the warm-up, while a seek mid-cycle, a reopened tab and a moved cable fire nothing. A Ratio
Gear's Clock In plugged, pulled or moved onto another output is a birth too, and fires nothing
for the distance between the two clocks. **A Reset above is a beat below**: a gear's Reset says
of its readings that they were put where they are (`TickContext::jump`), and a Ratio Gear
reading one, or a clock sent back more than a cycle in a frame, is born again where the clock
now is and fires at most one downbeat, its own, counted at the pace clock and ratio together
run, so a hand's Reset half a cycle in gives a ×1, a ×2 and a ÷4 under it their one downbeat
on the master's frame. A step back of under a cycle that no source calls a jump — a −×1 gear
upstream, an oscillator scrubbing — is motion. silvia's Start/Stop and Reset on the
oscillator and the sequencers are the Hold and Reset of the gear that drives them.

**Time** (`time`). Ambient time as a number: Seconds, the playhead published as a count, which
counts on, and nothing else. A rate against the show is a Ratio Gear, and a count of cycles a Master
Gear's.

**The display: one option, Display, with Rosette and Gears**, drawn in one body region,
`Region::Gear`, below the port rows, one fixed height for both (`widgets::gear`). Both turn at
the real rate: their angles are the gears' own phases, not an animation clock, so a paused
show is still.
- **Rosette**, the default. A still spirograph that turns once round its picture each input
  cycle. For a ratio `p/q` it winds `q` loops, the input cycles it takes to close, and waves in
  and out `p` times across them, the output's cycles, its petals: ×3 is three petals in one
  loop, ÷4 one petal wound over four loops. A tick at the top marks where each loop begins, and
  a dot moves on the curve at the output's phase. A Master Gear's rosette is a ring of a clock
  face's twelve ticks and the dot.
- **Gears.** A Ratio Gear draws two meshing gears, `k·p` teeth driving `k·q`, with `k` keeping
  both between 6 and 48; past that the hub prints the ratio. A Master Gear draws one gear of
  twelve teeth, turning once a cycle.
- A typed ratio that is no small fraction draws the nearest one, with the rim broken where it
  does not close.

**A Master Gear's caption** under the display says what a loop of it needs, read from the
chains below it and every node they drive through a Time (`nodes::chain`): "loops in 1 cycle ·
2.000 s", "loops in 4 cycles · 8.000 s (÷4 on ratiogear12)", naming the gear or node that asks
the most, or "2 nodes will not close".

## Periodic nodes

Each declares **`NodeDef::ambient`**: how many of its own cycles one ambient second is, and
where its Time wraps. For an unplugged Time the synth writes the uniform `(id, "clock")` every
tick, `(playhead × rate) mod wrap`, from the `f64` playhead. This is the same for every node,
with no CPU half of its own, so a node never integrates. A CPU node reads the same through
`TickContext::clock`.

**The rate at rest is silvia's default speed**, in the node's own cycles an ambient second, so
the picture at rest is silvia's, with two exceptions:
- A rate that is irrational in seconds, a period of 20π, is rounded to a nearby whole number of
  seconds. The difference is under 5%, which is invisible, and a whole number of seconds is a
  length a loop can have.
- **Where silvia is still at rest** (a default speed of zero, or a sequencer that starts
  stopped), the rate is **zero**. The node stays still until a gear is plugged into Time, and
  Offset's knob places it.

| node | one cycle | ambient rate at rest | silvia's default | Offset | notes |
|---|---|---|---|---|---|
| `mandelbrot`, `juliaset` | one drift of the Map orbit | 1/2 cycle a second (2 s) | Time Speed 0.5, period 1 | in drifts | |
| `cosinegradient` | one shift of the palette | 0, still | Cycle 0 | in palette cycles: places the palette | |
| `rotozoom` | the 20π super-cycle: one set of zoom waves | 1/60 (60 s) | 1/(20π) at speeds 1 and 1: 62.8 s | in super-cycles | **Turns**, a whole number −10 to 10, default 5, is how many turns one cycle makes: silvia's equal speeds give 5. 0 is zoom alone, and a sign reverses. It is a whole ratio inside the node, so the node closes on every cycle |
| `shakycam` | the 20π super-cycle on each axis | 1/60 (60 s) | 1/(20π) at 1 and 1 | in super-cycles | **A Time and an Offset per axis**: Time X and Offset X (`clock`, `phaseOffset`), Time Y and Offset Y (`clockY`, `phaseOffsetY`), so Y can shake alone or on a gear of its own. Unplugged, both read the one ambient time and the shake is silvia's. All four fold under the one Time heading |
| `geissflow` | the 20π flow cycle | 1/160 (160 s) | 0.4 ÷ 20π: 157 s | in flow cycles | The feedback half steps per drawn frame |
| `oscillator` | one wave | 1 (1 Hz) | Frequency 1 Hz | in waves | The Noise waveform draws a value per cycle, keyed by `floor(Time + Offset)` and the node's id, so it is stateless and loops. A one-shot is `animation`'s. The DC level is **Level** |
| `video` | one play of the clip | 1 ÷ the clip's length (native speed) | Speed ×1 | in plays | Loop/Hold: Hold clamps `Time + Offset` to one play. Reverse is a Ratio Gear at `-×1`. A render waits for its frame |
| `imagegif` | one play of the GIF's delays | 1 ÷ the GIF's length | Speed ×1 | in plays | as `video` |
| `stepsequencer`, `euclideanrhythm` | one bar of 16 steps | 0, stopped | they start stopped | in bars; 1/16 is a step | A Master Gear a bar long is the tempo, and its Hold and Reset are the play and the reset. Steps fire on crossings of `floor(16 × (Time + Offset))`. A reading that moves backwards, more than one bar in a tick, across a seek, or onto another clock as its Time cable is plugged, pulled or moved is a jump: it fires nothing and closes the gates. **Step**: while cabled, the sequencer counts Step events and ignores Time, the one stateful path, because an event clock (a tap, a threshold) is not a gear |

## Aperiodic nodes

The same two inputs, in the node's own natural units: Offset 0 to 1 is one unit.

| node | unit | ambient rate at rest | silvia's default | repeats |
|---|---|---|---|---|
| `perlin` | a lattice cell along the time axis | 0.5 | Time Speed 0.5 | only with Repeat |
| `simplex`, `fractal`, `domainwarp` | a lattice cell | 0, still | 0 | only with Repeat |
| `static` | a roll | 0, still | 0 | only with Repeat |
| `tunnel3d` | a unit of camera depth | 0.5 | Speed 0.5 | every 64 units while the wall wraps |

- **Repeat is an option on each noise**: Never, or every 1, 2, 4, 8 or 16 units. Static's list
  is Never, 4, 8, 16, 32, 64 or 128 rolls. Default Never, silvia's look.
  - With a length `N`, a noise walks a circle of circumference `N` through its 4D noise, at
    `turn = (Time + Offset) ÷ N`. Static reads its roll modulo `N`.
  - It is an option and not automatic because repeating changes the picture (4D noise is not
    3D noise), so a person chooses it. It is an ordinary option that rebuilds.
  - Driven by a gear at one unit a cycle, Repeat 1 repeats every cycle. At rest, Perlin at
    Repeat 4 repeats every 8 s.
- **Offset on a noise does not wrap** unless Repeat is on. Then it wraps at `N`.
- **Wraps.** An unplugged Time is written as a count, and the body takes it modulo `N` under
  Repeat and modulo 64 on the tunnel by its whole part, which every such period divides.
- **The tunnel's flight repeats.** Every path frequency is silvia's times 5π/16: Sine and
  Lissajous run at 0.2945 and 0.4909 and repeat every 64 units of depth, and Helix at 0.3927
  repeats every 16. Each is a whole number of both depth wraps, 8 (Mirror) and 4 (Repeat). So
  the flight repeats every 64 units under Mirror or Repeat, and while the depth wraps the
  camera's depth is taken modulo 64, so a Time of 64 draws a Time of 0 to the byte. The wiggle
  is 1.8% slower than silvia's, which nobody sees. Depth Wrap None never repeats.

## Stateful nodes

Not driven by Time. Each keeps its rate and steps on the transport's `dt`, so pause holds it
and a seek carries it across.

| node | rate it keeps | label |
|---|---|---|
| `slimemold` | steps owed at the label's times 30 a second | **Rate**, ×30/s |
| `cellularautomata` | generations owed at the label's times 30 a second, the fraction carried, so the pace is the same at any tick rate | **Rate**, ×30/s |
| `smoothcounter` | the ease toward its target | **Rate** |
| `autoexposure` | a time constant in seconds | **Response** |
| `stargate` | a drag in pixels per drawn frame | **Drift**, per frame, not per second |
| `animation` | Duration, with Start/Stop and Restart | an envelope a person or an event starts, like `adsr`, so it keeps its own start |
| `automation` | Duration, with Record, Play and Restart | a take is aligned to when it was played, not to a clock, and Play is the performer's own |
| `adsr`, `slew`, `autogain`, `brickgame`, `xypad`, `clockdivider`, `randomfire`, Output feedback | their own | |

`clock`, the time of day, is Live and outside the model.

## Offset means the time input

Where a port that is not the time input would read Offset, it has a label for what it does. Its
key stays `offset`.

| node | key | label | what it is |
|---|---|---|---|
| `oscillator` | `offset` | **Level** | the DC level added to the wave |
| `clock` | `offset` | **Zone** | hours from UTC |
| `lineargradient`, `radialgradient` | `offset` | **Shift** | slides the ramp |
| `kaleidoscope` | `offset` | **Shift** | slides the pattern outward |
| `chromaticaberration` | `offset` | **Spread** | how far the channels part |
| `convolve` (Emboss) | `offset` | **Gray** | the gray the flat parts go to |
| `stargate` | `offset` | **Position** | where the slit sits |

A compound label cannot be mistaken for the time input, so Translate's X Offset and Y Offset,
Tile's Offset X and Offset Y and the Slime Mold's Sensor Offset keep theirs.

## Precision: `f64` state, a count published whole

Every gear and the playhead keep `f64`. A count — a gear's Cycles, the Time node's Seconds, an
unplugged Time's ambient reading — is published whole, and a Time reads it at the same
precision at every count forever: a CPU node in `f64`, unbounded, and a shader as its whole
part, wrapped at **40320** centered on zero, and the `f32` of its fraction (`phasor::split`).
40320 is the least common multiple of 2520 and 128, so every period a body takes its Time
round — one cycle, a noise's Repeat, Static's to 128, the tunnel's 64 — divides it, and an
`f32` holds every whole number to 2²⁴ exactly: the body reduces the whole part by its period,
which is exact, then adds the fraction. A loop's frame a loop on reads its fraction to the
bit, before zero as after it.

Anything else that reads a count — a Math node, an input that is not a Time — reads one `f32`:
a gear's Cycles wrapped at **2520**, centered on zero, −1260 up to 1260 (`phasor::wrap_count`),
the least common multiple of 1 to 10, so a reader with a whole-number ratio or one whose
denominator is 10 or less passes the wrap with no seam, and Seconds' the playhead unwrapped. A
step over half the wrap is the wrap, not a motion, wherever such a clock is unwrapped. What
sees that wrap is a reader behind a Math node whose period does not divide 2520. `u_time` is
the playhead's fraction of a second.

## Render

A render steps the same transport. Frame `n` is at `T = (n − warm) ÷ fps`; each frame's advance
is `1 ÷ fps`; everything that reads the transport sees exactly that.

- **A render is the same twice, and exact at any frame rate.** A node is a function of its
  Time, and a gear integrating a constant rate telescopes to `rate × T` in `f64`.
- **A continuously moving cable is sampled once a frame.** That is exact for the same frame
  rate and close for another.
- **A clip waits for its frame**, where live it shows whatever the decoder has. The render
  holds the frame and ticks only what waits.
- **Every gear is born again at the render's first frame**, where the playhead puts it, so the
  first frame does not depend on how long the warm-up was, and is a downbeat: every Master Gear
  is on a whole cycle there and fires it.
- **Stateful nodes** step on `dt`, which in a render is `1 ÷ fps`.
- **A render is shown as it is made**: a picture window on the rendered Output and the mix of
  a deck it is on step through the film's own frames one at a time.
- **A render puts the live show back.** It sets every CPU node's live instance aside for a
  fresh one, resetting in place only a device's node and the clip, GIF and text nodes that say
  so, and when it ends hands back those instances, where each last read the transport, and the
  transport whole. So the live show carries on as the render found it and nothing sees a jump:
  a held gear stays held where it was, an XY Pad keeps its wells, a hand-started animation runs
  on. A simulation's world on the GPU is the render's last.

## Loops

**When a loop closes.** Walk back from the Output through every Time cable.
- A node on a gear chain rooted at master `M` advances `m × Πr ÷ P` of its own periods over
  `m` cycles of `M`. Here `Πr` is the product of the chain's ratios and `P` is the node's period
  in its own units: 1 for a periodic node, `N` under Repeat. It closes when that is whole.
- A node on ambient time closes over a length `L` when `rate × L ÷ P` is whole.
- A chain rooted at a Ratio Gear with Clock In unplugged is ambient time at the product of its
  ratios.
- An unconnected color input is the hue wheel, which stands still, so it closes on any loop.
- A simulation, a counter, a game or a live source carries its state across and never closes.

**The Master Gear says how long its loop is.** `nodes::chain` multiplies the fractions down each
chain from a master and follows the master's and each Ratio Gear's Cycles and Phase into every
time-driven node's Time, counting each node at its own period: a noise's Repeat, the tunnel's
64 while its depth wraps, a sequencer's lanes against the bar's sixteen steps, a looping clip's
one play. A loop is as many master cycles as the least common multiple of what they all ask
for, so a ÷4 below makes it four cycles, and so does a Perlin at Repeat 4 on the master. A
ratio that is no fraction with a denominator up to 64, or has a cable in it, runs and never
closes, and so does a node that never repeats, a clip on Hold, or a Time a gear reaches
through a Math node, which the caption cannot follow; the caption counts them.

**A loop is the Output's ordinary render**, with its PNG, video and GIF writers, for as long as
the caption says. A render starts at the playhead's zero, where every Master Gear is born at its
cycle's start. `examples/loop_gifs` renders that length for every tab of a project: the only
Master Gear upstream of the Output, else the first by id, else a typed length; a loop of
warm-up and one frame more; frame `F` compared with frame zero, and frames `0..F` written as a
GIF.

## Worked examples

**A tempo change.** A Master Gear half a second long, a beat at 120 BPM, has its Length turned
to 0.46875 s, 128 BPM, at 60 s. Its Cycles read 120 at 60 s and then
`120 + (T − 60) ÷ 0.46875`: 122.13 a second later, with no jump, because they are an integral.
A Ratio Gear ÷4 on those Cycles is at bar 30 and carries on a little faster. A `video` on that
bar gear plays a clip a bar and stretches to the new tempo. A
Mandelbrot on ambient time does not notice: it counts seconds. What follows the tempo is
exactly what reads the gear, and nothing else.

**A loop of four bars.** A Master Gear of Length 2, a bar at 120 BPM: a cycle is 2 s. A Ratio
Gear ÷4 under it drives a Cosine Gradient's Time, and a ×2 drives a Perlin at Repeat 1. The ÷4
asks for four master cycles, the ×2 for one, so the caption reads "loops in 4 cycles · 8.000
s (÷4 on ratiogear…)", and a render that long closes.

## Timelines

[timelines.md](timelines.md) is designed against an earlier transport. What it takes from this
model when it is built:

- **The transport is global**: a playhead and a play bit over the one clock, and a timeline
  tab is where it is edited. A timeline workspace carries its own playhead and could drive a
  Master Gear.
- **Live jumps.** A seek re-births every gear where the playhead puts it, stateful nodes carry
  across, and a jump fires no crossings.
- **Render mode.** A node is a function of its Time, so a render is exact at any frame rate
  for constant rates. A clip waits for its frame.
- **Loops** are a gear chain's, read by the Master Gear's caption. A loop is the ordinary
  render; there is no loop export window.
- **Per-pixel time** is answered: a field is added as Offset, never used as a rate.
- **Whatever must be aligned to a point on the timeline reads the playhead; whatever runs
  free reads a gear.** A clip that must start at its first frame when its region starts is a
  **media track**, which asks for the frame the playhead implies. A `video` node is for a clip
  whose position is its gear's or a cable's.

## Sources

- Ableton Link, [concepts](https://ableton.github.io/link/) and
  [`Link.hpp`](https://raw.githubusercontent.com/Ableton/link/master/include/ableton/Link.hpp):
  the beat, time and tempo triple, phase against a quantum.
- Tidal, [`Sound.Tidal.Tempo`](https://hackage.haskell.org/package/tidal-1.0.6/docs/src/Sound.Tidal.Tempo.html):
  `timeToCycles` and re-anchoring on `setCps`.
- Strudel, [issue #51](https://codeberg.org/uzu/strudel/issues/51): `time × cps` repeating and
  skipping notes.
- SuperCollider, [TempoClock](https://doc.sccode.org/Classes/TempoClock.html).
- Max, [`transport`](https://docs.cycling74.com/legacy/max8/refpages/transport),
  [`phasor~`](https://docs.cycling74.com/reference/phasor~/),
  [`rate~`](https://docs.cycling74.com/reference/rate~/),
  [`wave~`](https://docs.cycling74.com/reference/wave~/).
- Bitwig, [On Grid Signals](https://www.bitwig.com/userguide/latest/on_grid_signals/) and
  [Grid modules](https://www.bitwig.com/userguide/latest/grid_modules/).
- HetrickCV, [Phasors](https://github.com/mhetrick/hetrickcv/blob/master/docs/Topics/Phasors.md).
- TouchDesigner, [Speed CHOP](https://docs.derivative.ca/Speed_CHOP),
  [Time COMP](https://docs.derivative.ca/index.php?title=Time_COMP),
  [Movie File In TOP](https://docs.derivative.ca/Movie_File_In_TOP), and the forum thread
  [Noise TOP speed ramp](https://forum.derivative.ca/t/solved-noise-top-translation-speed-ramp/551955).
- Notch, [Jump to Time](http://manual.notch.one/0.9.23/en/topic/nodes-logic-jump-to-time) and
  [Video Loader](https://manual.notch.one/0.9.23/en/docs/nodes/video-processing/input-output/video-loader/).
- MadMapper, [Materials documentation](https://github.com/madmappersoftware/MadMapper-Materials/blob/main/MaterialsDoc.md):
  the worked `sin(speed × TIME)` jump and `time_base`.
- Resolume, [video transport](https://resolume.com/support/en/7.18/video) and
  [parameter animation](https://resolume.com/support/en/parameter-animation).
- Hydra, [`hydra-synth.js`](https://github.com/hydra-synth/hydra-synth/blob/main/src/hydra-synth.js).
- vvvv, [video recording](https://thegraybook.vvvv.org/reference/best-practice/video-recording.html);
  CCapture, [README](https://github.com/spite/ccapture.js/blob/master/README.md); p5.js,
  [issue #8888](https://github.com/processing/p5.js/issues/8888).
- Loops: [necessary disorder, noise loops](https://necessarydisorder.wordpress.com/2017/11/15/drawing-from-noise-and-then-making-animated-loopy-gifs-from-there/);
  Gustavson and McEwan, [psrdnoise](https://jcgt.org/published/0011/01/02/paper.pdf).
- Precision: the [WGSL specification](https://www.w3.org/TR/WGSL/) on `sin` accuracy;
  [fosfora #95](https://github.com/kevinraymond/fosfora/issues/95).
- FRP's boundary between pure behaviours and causal integrals: Perez,
  [Back to the Future: Time Travel in FRP](https://dl.acm.org/doi/pdf/10.1145/3122955.3122957).
