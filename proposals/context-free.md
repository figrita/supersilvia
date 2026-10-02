# Proposal: the context-free graph

**Status: stages 1–9 built; stage 10 is half of its twenty nodes.** Decided on
2026-09-10 and 2026-09-11 and built over 2026-09-11 and 2026-09-12. Every stage below was
built in order, each landing with its tests and its docs in one commit. When a stage lands, its reasoning moves to [docs/decisions.md](../docs/decisions.md)
and its behavior to `docs/`, and the stage is struck here.

| stage | |
| --- | --- |
| 1 | retire `feedback` and the history array |
| 2 | the resolution leaves every node body |
| 3 | the canonical tap |
| 4 | tap uniform numbers are delayed |
| 5 | a tap on a workspace is measured |
| 6 | time on the CPU |
| 7 | struck, then reinstated: a node with a speed accumulates its own phase |
| 8 | dual-mode math |
| 9 | the conversion menu, then curated to a handful |
| 10 | ten of twenty nodes; the other ten are proposals of their own |

The porting calls made during stage 10 were reviewed on 2026-09-12. Three of the eight
changed the tree — `euclideanrhythm` draws the textbook figure at rotation zero,
`muxevent` lost a crossfade that did nothing, and `smoothcounter`'s three actions became a
decision rather than a porting call.

## The principle

**A node's output is a function of `uv`, the clock, its own uniforms, and textures named by
node. Nothing about the program it is compiled into.** And **a tap measures its input**, over
the unit square at a canonical grid, once per point per frame, whoever calls it and however.

Once both hold, a uniform number has one value everywhere because there is only one value to
have, and every ambiguity below stops being a policy question.

## What is wrong today, as found in the code

A `UniformNumber` is one number per port per frame, keyed by node and not by Output. A tap
compiled into two Outputs has a slot in each, and `App::collect_readbacks` pools them:
counts and sums add, extremes combine (`tap::merge`); for a sample the later slot wins, and
later is the renderer's `HashMap` order (`sample::merge`), so the winner is arbitrary. When
one Output drops a frame and the other does not, the merge is rebuilt from the one that
reported and the reading swings between the pool and one side of it.

Three things make the two slots differ in the first place:

1. **`feedback` reads the history of whichever Output it is compiled into.** It is the only
   node with `reads_frame_history`, and the only node whose *input picture* depends on the
   consumer. A feedback into a tap under two Outputs measures two different pictures.
2. **Seven node files read `u_resolution`,** all but two to turn a size in pixels into a
   world offset: blur, sharpen, dilate, erode, emboss, edge detection, and the three
   screentones (which scale `uv` by half the height in pixels for their grid). The two
   others are `sample`'s texel tolerance and the wipes in `mix`, which divide by the aspect
   to reach the screen's edges. A resolution-dependent node is a different field in Outputs
   of different size.
3. **The coordinates a tap is asked at come from downstream.** The tap measures inside its
   pass-through function, so it measures at whatever `uv` its caller passed. The emitted GLSL
   for gradient → tap → zoom → Output is `zoom3_output(uv)` computing `zoomedUV` and calling
   `tap2_output(zoomedUV)`; the tap evaluates and measures the gradient at the zoomed point.
   A transform between a tap and its Output, or a difference in aspect, changes the region
   measured. This is the expression model doing what it says — a transform moves the
   sampling coordinate — and it is the one place a texture pipeline would behave otherwise.

Four smaller things, found on the way:

- A tap that no Output reaches publishes zeros (`Tap::tick` maps a missing readback to
  `Stats::NONE`), so its rows print `0.00`, a reading that is not one. A tap that *was*
  compiled and then cut loose keeps its last readback forever: `App::tick` retains readbacks
  for any node still in the graph and `collect_readbacks` only ever inserts for nodes in a
  compiled shader.
- A tap's uniform number cannot be cabled to anything upstream of the tap:
  `Graph::can_connect` treats a uniform edge as immediate, though a tap's uniform numbers
  are a frame old by construction. `autoexposure` closes its loop through `own_uniform`
  because the cable would be refused.
- The conversion menu is a decision without an implementation. silvia's
  `js/typeConversions.js` is real: a dotted ring on a port that can take the cable through a
  conversion, a menu on release, the converter inserted midway and wired both sides. Here
  `ui::legal` offers only what `can_connect` accepts, and a release elsewhere does nothing.
- silvia's `oscillator` is a CPU node — an accumulator with start/stop, reset, one-shot, a
  pulse waveform and a scope — and it is the one silvia CPU node that went varying here.
  silvia has no CPU math on the video side at all: a float input becomes a uniform and its
  template says the value cannot be read from JavaScript.

## Decisions

Each becomes a `decisions.md` entry when its stage lands.

- **`feedback` is retired.** A loop is a cable from an Output's frame port, which names its
  Output and is already legal and tested. silvia's other history readers get an explicit
  previous-frame input when they are ported.
- **The frame-history array goes with it.** A delay node that names its Output is
  [history-delay.md](history-delay.md), to be built when something wants it.
- **The resolution leaves every node body.** Kernels and grids in world units on a reference
  height of 360 units-per-half-frame, so 720p looks as it does today and other sizes scale
  with the picture. **Dither included:** the moiré a world-locked dither makes at other
  resolutions is accepted.
- **Wipes are the mixer's.** `mix` keeps the blend and the two luminance fades, which need
  no screen; the five screen-space methods go, and the mixer already has all eight.
- **A tap measures its input over the unit square on a fixed grid.** Grid size is a `Code`
  option, default 128, choices 32/64/128/256. Jitter is a `Uniform` option, default off.
- **Readings are picked, not pooled.** With identical readings the pool is the same number
  twice; the pick is on air first, then the active workspace, then the lowest id, and the
  node says which.
- **A tap on a workspace is measured whether or not it reaches an Output.** The ghost
  context, done inside an existing pass. This reverses "make an Output of it".
- **Tap uniform numbers are delayed ports,** so a tap closes a loop with a cable.
- **The field oscillator is cut.** `oscillator` becomes silvia's CPU node;
  [field-oscillator.md](field-oscillator.md) keeps the other idea for later.
- **Uniform math is dual-mode.** One `add`, a diamond when everything feeding it is a
  diamond or a knob, a circle otherwise.
- **The `time` port stays on every generator, as the override.** A node with a speed
  accumulates its own phase and reads it through `own_uniform`; a cable into `time` replaces
  that.

## ~~Stage 1 — retire `feedback` and the history array~~ — built

**Landed.**

**Read:** [docs/architecture.md](../docs/architecture.md#delayed-ports-and-feedback),
[docs/rendering.md](../docs/rendering.md#per-output-per-frame), `src/nodes/feedback.rs`,
`src/render/output.rs` (`History`, `new_history`, `free_history`, `needs_history`),
`src/compile/mod.rs` (`Shader::uses_frame_history`, `reads_frame_history`),
`src/compile/prelude.glsl`, `src/nodes/mod.rs` (`NodeDef::reads_frame_history` and the
registry test holding it to the emitted text), `tests/compile.rs`, `tests/feedback.rs`,
`tests/headless_gl.rs`, `src/workspace.rs` (`SLUG_ALIASES` and the load path).

**Change:**

- Remove the node, the field, the flag on `Shader`, the three history uniforms from the
  prelude, and the renderer's history array with its per-frame blit. Nothing may flash: the
  array is simply never allocated, and the published `output_texture` the frame port samples
  is untouched.
- **A saved file with a `feedback` node still opens.** Where the node reaches exactly one
  Output, the loader replaces it with cables from that Output's frame port to every input
  the feedback fed, and reports the substitution the way an import reports its glue. Where it
  reaches none or several, it is dropped and the report says so. `tests/workspace.rs` opens
  both shapes.
- `tests/feedback.rs` keeps every loop-through-the-frame-port test; the one compile test
  asserting `uses_frame_history` goes; snapshots that carried the history uniforms are
  re-accepted.

**Docs:** the "Delayed ports and feedback" section keeps the frame port and loses the node;
"The frame history" leaves rendering.md; nodes.md loses `reads_frame_history`; decisions.md
gains *`feedback` is a cable, not a node* with the pooling finding as its why, and
[history-delay.md](history-delay.md) is named as where the deeper delay went.

**Done when:** no source file names `u_frame_history`; `./check.sh` is green; a file with a
feedback node opens as a frame-port loop.

## ~~Stage 2 — the resolution leaves every node body~~ — built

**Landed.**

**Read:** `src/nodes/convolve.rs`, `src/nodes/edgedetection.rs`, `src/nodes/screentone.rs`
(its module doc already states "one world unit is `u_resolution.y / 2` pixels"),
`src/nodes/mix.rs`, `src/nodes/debug.rs` (the precedent: "the size is in world units"),
[silvia-node-parity.md](silvia-node-parity.md) for every default being converted.

**Change:**

- **The reference height is a constant in `nodes/`, 360.** One world unit is 360 pixels of a
  720p frame, and every size that was in pixels is now in world units with a default of the
  old pixels over 360, so a 720p Output is pixel-identical to today. Units on the controls
  change from `px` to the world glyph `sample` already uses.
- Blur, sharpen, dilate, erode, emboss, edge detection: the step is the control divided by
  nothing. The screentones: `PIXEL_GRID` and the mosaic's quantization multiply `uv` by the
  reference height rather than by half the resolution. Dither goes with them.
- `mix` drops `h_wipe`, `v_wipe`, `radial`, `checker` and `h_lines`; a saved file carrying one
  of those values loads as `blend` without an `UnknownOption`, through the value-alias path
  `workspace.rs` already has for one renamed value.
- **A registry test compiles every node into a probe graph and asserts no emitted node
  function names `u_resolution` or `gl_FragCoord`.** `sample` is allowlisted in this stage
  and the allowlist is emptied in stage 3. The prelude keeps `u_resolution` for `main`,
  which is where `uv` is built.

**Docs:** nodes.md gains a paragraph under Worldspace: sizes are in world units, the
reference height, and the rule the test enforces; decisions.md gains *No node reads the
resolution* with the dither cost stated and accepted; the parity doc's per-node lines for
the converted controls are updated.

**Done when:** the test above passes with only `sample` allowlisted; a 720p snapshot of each
converted node is unchanged; `./check.sh` is green.

## ~~Stage 3 — the canonical tap~~ — built

**Landed.**

**Read:** [docs/architecture.md](../docs/architecture.md#going-down-side-effects-in-the-expression),
[docs/cpu.md](../docs/cpu.md#taps-the-other-direction), [docs/rendering.md](../docs/rendering.md#tap-buffers),
`src/compile/mod.rs`, `src/nodes/tap.rs`, `src/nodes/sample.rs`,
`src/nodes/autoexposure.rs`, `src/app.rs` (`collect_readbacks`, `tick`),
`tests/headless_gl.rs` from `tap_words` down, `tests/uniform.rs`.

**Change:**

- **Measurement becomes a property of the node, not of an output.** `NodeDef` gains
  `measure: Option<fn(NodeId, &mut CompileContext)>`. The compiler calls it for every node it
  emits that has one. A measurement claims its slot with `tap_slot` as today, emits a
  function `void {slug}{id}_measure(vec2 p)` whose body evaluates the node's inputs at `p`
  through `ctx.input(node, key, "p")`, and registers the call `main` makes. A tap's `output`
  generator becomes `return {input};` and nothing else; `stats_glsl` takes the coordinate's
  name rather than assuming `uv`.
- **`main` runs the grid.** After building `uv`, `main` computes
  `ivec2 cell = ivec2(gl_FragCoord.xy)`, and for each registered measurement with grid `N`
  emits `if (cell.x < N && cell.y < N) tapK_measure((vec2(cell) + 0.5) / float(N) * 2.0 - 1.0);`.
  A sample registers a single call under `cell == ivec2(0)`, evaluating its input at
  `vec2(x, y)` exactly. Jitter adds a per-cell hash of `u_time` to the point, scaled to the
  cell, under the `Uniform` option's `int`, so turning it on rebuilds nothing.
- **`tap`:** options `grid` (`Code`; 32, 64, 128, 256; default 128) and `jitter` (`Uniform`;
  off, on). Same measurement as today over the square. **`sample`:** no tolerance, no hold
  of its own; the slot is written every frame the pass runs. **`autoexposure`:** measures
  through `measure` and its pass-through multiplies by the gain it read.
- **Pick, do not pool.** `collect_readbacks` ranks the Outputs that reported — on a mixer
  deck, then on the active workspace, then lowest id — and takes the first slot per node.
  `tap::merge` and `sample::merge` go. Readbacks are retained only for nodes that hold a slot
  in some current shader, so a tap cut loose loses its number instead of freezing it.
- **A dormant tap says so.** `TickContext` gains `measured_in(id) -> Option<NodeId>` and
  `withdraw(id, port)`. A tap with no readback withdraws its uniform numbers — its rows then
  draw nothing, under the rule an unpublished port already follows — and its `status()`
  reads *not measuring*; a tap with one reads *in Output N*.
- **The cost probe keeps working.** The measure function's call into the input chain is one
  more call site under the same input key, so a tap's own taps figure includes its grid;
  rendering.md says so.
- **Tests.** `tests/headless_gl.rs`: a solid gray reads exactly; a checkerboard whose period
  divides the grid reads exactly half; **a zoom between the tap and the Output does not
  change the reading**; two Outputs of different aspect and size produce identical words; a
  sample reads the color at its point with a transform downstream that would have put the
  point off screen. `tests/uniform.rs`: a tap with no readback publishes nothing and reports
  not measuring. `tests/compile.rs`: the emitted main carries the grid guard; snapshots
  re-accepted.

**Docs:** "Going down" in architecture.md and "Taps" in cpu.md are rewritten around the
grid; nodes.md's tap section and the two new options; rendering.md's tap buffers gain the
grid and the pick; decisions.md gains *A tap measures its input over the unit square*, states
the two costs (extremes are the grid's, a fine pattern can bias the mean) and the jitter
answer, and amends *Going down is a side effect, not a render target*: still inside the pass,
no longer at the caller's coordinate, and the feedback objection to a canonical domain is
moot once stage 1 has landed.

**Done when:** the zoom test passes; the registry test from stage 2 passes with an empty
allowlist; the real app shows a tap reading the same number in two Outputs with a zoom
between one of them; `./check.sh` is green.

## ~~Stage 4 — tap uniform numbers are delayed~~ — built

**Landed.**

**Read:** `src/graph/port.rs` (`PortDef::delayed`), `src/nodes/mod.rs` (`port_defs`),
`src/graph/mod.rs` (`can_connect`, `compute_topological_order`), `tests/uniform.rs`.

**Change:** `OutputDef` gains `delayed: bool`, false in `EMPTY`, true on every
`UniformNumber` output of `tap`, `sample` and `autoexposure`; `port_defs` marks a `Texture`
output delayed as today and a delayed uniform number besides. A registry test holds a
delayed uniform number to a node with both halves. `can_connect` needs no change: it already
skips delayed edges in the cycle check. `tick_order` includes the edge, so a genuine loop
takes the deterministic break, which is right because the tap's reading is last frame's
anyway. `tests/uniform.rs`: a tap's `mean` cabled to a `zoom` that feeds the tap is
accepted, compiles, and ticks in a total order; `autoexposure` may then close its own loop
with a cable in a test, though it keeps `own_uniform` for the one-drop case.

**Docs:** cpu.md's tap section and architecture.md's delayed-ports section name the second
kind of delayed port; decisions.md gains a line under the uniform number entry.

**Done when:** the loop test passes; `./check.sh` is green.

## ~~Stage 5 — a tap on a workspace is measured~~ — built

**Landed.**

**Read:** stage 3 as landed; `src/app.rs` (`build_frame_job`, `needs_recompile` and the
commands that mark it, `live_nodes`), `src/graph/mod.rs` (`downstream_outputs`),
[docs/architecture.md](../docs/architecture.md#the-recompile-boundary).

**Change:**

- **Assignment.** For each workspace with at least one rendering Output, the ranked-first
  Output — on a deck, then lowest id — hosts every node with a `measure` that is on that
  workspace and is not emitted into any rendering Output's shader. A node on several
  workspaces is hosted once. `compile::build` takes the host's extra list and runs each extra
  node's `measure`, which pulls that node's input chain into the shader.
- **Rebuild.** A structural edit anywhere upstream of a hosted node marks its host, and only
  its host; an edit elsewhere does not. The assignment is recomputed on every structural
  command and a host whose extra list changed is marked. `tests/controls.rs` asserts both
  halves.
- **The node says which.** `measured_in` now answers for hosted nodes too, so the status
  reads *in Output N* either way, and *not measuring* only when nothing on the node's
  workspaces renders.
- **Tests.** `tests/uniform.rs` with headless GL: a tap on a chain that reaches no Output,
  on a workspace whose Output draws something unrelated, reads its chain.

**Docs:** cpu.md's tap section; decisions.md strikes *What it cannot do: a number about an
expression that no Output draws* and says what replaced it, with the cost — a hosted tap's
grid evaluates its chain inside a pass that did not need it.

**Done when:** the hosted-tap test passes; the real app shows a tap on an unconnected chain
reading; `./check.sh` is green.

## ~~Stage 6 — time on the CPU: `time`, `phase`'s outputs, `oscillator`, `animation`~~ — built

**Landed.**

**Read:** `src/nodes/phase.rs`, `src/nodes/oscillator.rs`, `src/nodes/adsr.rs` (the trace
ring), `src/nodes/counter.rs` (two outputs from one count), `src/workspace.rs` (the alias
tables and how a missing port on load is treated),
[silvia-node-parity.md](silvia-node-parity.md) for `oscillator` and `animation`,
silvia's `js/nodes/video/oscillator.js` and `animation.js`.

**Change:**

- **`time`**, `Category::Control`: inputs `speed` (UniformNumber, default 1, `×`),
  `startStop` (Action, `Press`, flips on each down, as silvia's toggles do), `reset`
  (Action, `Press`); output `seconds` (UniformNumber): the integral of `speed` over `dt`
  since the last reset, held while stopped. Its debug line is the value.
- **`phase`** keeps its `phase` output and its key, relabeled *Cycles*, and gains `wrapped`
  (*Phase*, `fract` of the cycles, 0 to 1) and `pingpong` (*Ping-pong*, 0 to 1 and back over
  two cycles). Three outputs and no mode option, the way `counter` publishes `value` and
  `normalized`.
- **`oscillator`** is silvia's: inputs `frequency` (UniformNumber, log, 0.01 to 30 Hz),
  `amplitude`, `offset` (UniformNumber), `startStop` and `reset` (Action, `Press`); options
  `waveform` (sine, cosine, triangle, square, sawtooth, pulse, noise) and `mode` (free,
  oneshot); output `output` (UniformNumber). An accumulator like `phase`'s inside, with
  silvia's rate smoothing as a glide of 0.05 s. The footer trace becomes `CpuNode::trace`, a
  ring of what it published, as `adsr`'s is. Waveform phases match silvia's formulas, not
  today's. **Saved files:** the three number controls carry over by key; a cable into the
  old `time` or `phase` port is dropped with a report line; a cable from `output` still
  lands, since a uniform number feeds a varying number. The field version is
  [field-oscillator.md](field-oscillator.md).
- **`animation`** is silvia's: inputs `startValue`, `endValue`, `duration` (UniformNumber,
  with controls), `startStop` and `restart` (Action, `Press`); options `approach_curve`
  (linear, smooth, ease_in, ease_out) and `return_curve` (the same plus jump, stay); output
  `output` (UniformNumber, ranged `[start, end]`); a trace.
- **Tests.** `tests/uniform.rs` for each: `time` integrates and holds; `phase`'s two new
  outputs; `oscillator` at frequency 1 crosses zero at the right moments and a step in
  frequency bends rather than jumps; `animation`'s three return modes. `tests/workspace.rs`:
  an old oscillator loads with its controls and without its time cable.

**Docs:** nodes.md's library table; cpu.md's list of CPU nodes; decisions.md amends *Time is
an input port* with the answer to the parked question and adds *`oscillator` is a CPU node*
with why the field one lost.

**Done when:** the four nodes are in the registry with tests; `./check.sh` is green.

## ~~Stage 7 — every node with a speed accumulates its own phase~~ — built

**Landed.**

**Read:** `src/nodes/phase.rs`, `src/nodes/autoexposure.rs` (`own_uniform`),
[docs/decisions.md](../docs/decisions.md#time-is-an-input-port-not-an-ambient-global), the
parity doc's C10 note, silvia's `tunnel3d.js` (`_getCurrentPhase`).

**Change:** one shared `nodes::accumulator` CPU half, created per node with the key of its
speed control, integrating that control with `phase`'s glide into a `UniformNumber` output
`phase` the node publishes. Every node whose body has `time * timeSpeed` — mandelbrot,
juliaset, perlin, simplex, fractal, static, cosinegradient's `cycle`, tunnel3d's speed —
reads `own_uniform("phase")` there when `time` is unconnected, and `time(uv) * timeSpeed`
when it is; `ctx.connected` decides at compile time. The `time` port stays,
`Control::Global`, as the override.

**Tests:** `tests/uniform.rs`: a perlin publishes a phase that bends on a speed step;
`tests/compile.rs`: connected and unconnected `time` emit the two forms.

**Docs:** nodes.md's "Time is a parameter" section gains the accumulator; decisions.md's
time entry gains the resolution.

**Done when:** every listed node publishes `phase`; `./check.sh` is green.

## ~~Stage 8 — dual-mode math~~ — built

**Landed.**

**Follow-up landed:** a pinned dual node's inputs are `UniformNumber`.

The largest stage, and the one that adds a concept: **a node whose output type is decided by
what feeds it.**

**Read:** `src/nodes/math.rs`, `src/nodes/reframerange.rs`, `src/graph/node.rs`
(`Node::outputs` is per instance), `src/graph/mod.rs` (`can_connect`, `connect`,
`disconnect`, the adjacency caches), `src/compile/mod.rs` (`input`'s uniform-number branch),
`src/app.rs` (`tick`), `src/ui/node_widget.rs` (`port`, the readout),
[docs/decisions.md](../docs/decisions.md#a-fourth-port-type-uniform-numbers).

**Change, 8a — the type and the rule:**

- `OutputDef` gains `eval: Option<fn(NodeId, &TickContext) -> f32>`. An output with `eval`
  is **dual**: its type is `VaryingNumber` in the definition and its *effective* type on an
  instance is `UniformNumber` when every connected input's source has effective type
  `UniformNumber` and every unconnected input has a `Number` control. `Graph` recomputes
  effective types for the downstream closure on every connect, disconnect, add and remove,
  and writes them into the instance's `outputs[i].ty`, so the compiler, `can_connect` and
  the canvas read the answer they already read. A file loads with definition types and is
  recomputed once.
- **Demotion is unofferable.** `can_connect` refuses, as a new `ConnectError::WouldDemote`,
  a cable that would flip a dual output to `VaryingNumber` while any consumer of it, or of
  a dual node downstream of it, is a `UniformNumber` input. The canvas dims it like any
  other illegal target.
- **The tick evaluates diamond-mode duals.** `App::tick` walks `tick_order` and, for a node
  with no `cpu` half whose dual outputs are effectively `UniformNumber`, publishes `eval`. A
  dual node in circle mode is untouched by the tick and compiles as today; in diamond mode
  the compiler's uniform-number branch resolves it to a GLSL uniform and emits no function.
- The dual family: `add`, `subtract`, `multiply`, `divide`, `min`, `max`, `sine`, `cosine`,
  `reframerange`. Each `eval` is the GLSL body in Rust; a test evaluates both on a grid of
  inputs and holds them equal to 1e-6.

**Change, 8b — what a hand sees:**

- The port draws by the instance type, so a diamond appears when the mode flips, and the row
  prints the published number in diamond mode through the readout that already exists. A
  dual node with nothing connected is a constant: two knobs and a diamond.
- Registry tests: an `eval` belongs to a `VaryingNumber` output on a node with no `cpu`
  half whose inputs are all `VaryingNumber` with `Number` controls.
- `tests/graph.rs`: the inference on a chain; the demotion refusal; a disconnect that
  promotes. `tests/uniform.rs`: `add` with two knobs publishes their sum; `add` of two
  uniform numbers feeds `phase.rate`. `tests/compile.rs`: an `add` in diamond mode reaches
  its consumer as a bare GLSL uniform. `tests/ui.rs`: the port is a diamond in one snapshot
  and a circle in another.

**Docs:** architecture.md's four kinds of value gain the dual output; nodes.md gains a "Dual
outputs" section beside "CPU nodes"; decisions.md gains *Uniform math is dual-mode*
with the twin family as what lost and the demotion rule as the price.

**Done when:** every test above passes; the real app shows an `add` flip from diamond to
circle when a field is cabled in and refuse the cable when a slew hangs off it; `./check.sh`
is green.

## ~~Stage 9 — the conversion menu~~ — built

**Landed.**

**Read:** [docs/decisions.md](../docs/decisions.md#a-fourth-port-type-uniform-numbers) (the
two paragraphs on the menu), silvia's `js/typeConversions.js` and
`styles/conversions.css`, `src/ui/mod.rs` (`legal`, the drag's release), `src/ui/browse.rs`
(the popup to copy), `src/command.rs`.

**Change:**

- ~~**The bridges are a registry query:**~~ **Struck**, after it was built once that way. The
  query — every node with an input of type A and an output of type B, one row per pairing —
  is an inline patchbay: fifteen rows for a color onto a circle, every measurement there is
  for a color onto a diamond. silvia's `typeConversions.js` is a *curated* table, twelve rows
  across the two pairs it has, each named for the casting rather than the node, and that is
  what the menu is: `nodes::bridge`. A row may also fan out, which no pairing could express —
  *Grayscale* is one number into an `rgba`'s red, green and blue.
- **Three port states during a drag,** silvia's: a legal target as today, an illegal one
  dimmed, and a convertible one with a dotted ring, because at the diamond boundary a
  conversion costs a readback and a frame, which is more than a luminosity node and should
  not be hidden.
- **Release on a convertible port opens the menu** at the port; choosing a row issues one
  undoable `Command::Bridge { from, to, slug, input, output, at }` that adds the node at the
  midpoint of the two nodes, connects both cables, and lands the node on the workspace.
  Dismissal is a click elsewhere or Escape.
- `tests/ui.rs`: drop a color output on a varying number input, choose Luminosity, three
  nodes and two cables; one undo removes all three; the dotted ring is in a snapshot.

**Docs:** ui.md gains "The conversion menu"; decisions.md's two paragraphs are rewritten:
the query is "an input of A and an output of B", and the two-state rule loses to silvia's
three.

**Done when:** the kittest passes; the real app shows the ring and the menu; `./check.sh` is
green.

## Stage 10 — silvia's CPU inventory — half built

**Landed in part:** ten of the twenty, in two batches; the other ten wait on a decision each and are proposals of their own.

Every silvia video node whose value is computed in JavaScript, by the `floatUniformUpdate`
signal in its output, that is not here yet. One item each, in this order, each in silvia's
shape with every node-local `value` as a `UniformNumber` input with a control, and each with
its `tests/uniform.rs` test:

1. `smoothcounter` · 2. `triggeredrandom` · 3. `triggeredcolor` · 4. `clock` · 5. `muxevent` ·
6. `automation` · 7. `xypad` · 8. `mouseinput` · 9. `gamepad` · ~~10. `maininput`~~ (built, as a
panel and a node) — numbers.
11. `randomfire` · 12. `stepsequencer` · 13. `euclideanrhythm` — events.
14. `cellularautomata` · 15. `slimemold` · 16. `brickgame` · 17. `drawingcanvas` · 18. `text` ·
~~19. `screencapture`~~ (built, as a Main Input source) · ~~20. `imagegif`~~ (built, on the
asset kind) — textures.


## Order, and what depends on what

1 → 2 → 3 → 4 → 5 is one line: nothing after 1 makes sense with a consumer-dependent node in
the graph, 3 needs 2's test to become strict, 4 and 5 are refinements of 3. 6 and 7 depend on
nothing above and can interleave. 8 depends on nothing above but is the one that needs the
most care. 9 depends on 3 for the tap bridges to be worth offering. 10 is the long tail and
goes last, one node at a time.
