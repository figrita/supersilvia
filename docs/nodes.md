# Nodes

## The shape of a definition

A node is a `const NodeDef`: static data, plus one generator function per output. It is not
a trait implementation.

```rust
pub static DEF: NodeDef = NodeDef {
    slug: "checkerboard",
    icon: "🏁",
    label: "Checkerboard",
    tooltip: "Alternating checkerboard pattern. Frequency sets grid resolution.",
    inputs: &[
        InputDef { key: "frequency", label: "Frequency", ty: VaryingNumber,
            control: Control::num(8.0, 1.0, 64.0, 1.0, "/⬓") },
        InputDef { key: "color1", label: "Odd Color", ty: VaryingColor,
            control: Control::color("#ffffffff") },
        InputDef { key: "color2", label: "Even Color", ty: VaryingColor,
            control: Control::color("#000000ff") },
    ],
    outputs: &[OutputDef {
        key: "output", label: "Output", ty: VaryingColor, kind: OutputKind::Shader,
        wgsl: |node, ctx, _func| {
            let frequency = ctx.input(node, "frequency", "uv");
            let c1 = ctx.input(node, "color1", "uv");
            let c2 = ctx.input(node, "color2", "uv");
            format!(
                "    let frequency = {frequency};
    let grid = floor(uv * (frequency / 2.0));
    let checker = floor_mod(grid.x + grid.y, 2.0);
    return mix({c1}, {c2}, checker);"
            )
        },
        ..OutputDef::EMPTY
    }],
    ..NodeDef::EMPTY
};
```

Two fields beside the ports carry the halves a node may have outside its own WGSL:
`cpu: Option<CpuDef>` is per-instance state with a `tick` ([CPU nodes](#cpu-nodes)), and
`measure_wgsl: Option<fn(NodeId, &mut CompileContext)>` is the measurement a node that turns a
picture into uniform numbers declares — a function of a point, called once per node per
shader, whose body `fs_main` runs over a grid rather than at a consumer's coordinate
([Nodes with both halves](#nodes-with-both-halves)). A registry test holds a node with a
`measure_wgsl` to having a `cpu` half, which is what reads the slot back.

One field on an *output* does the same for arithmetic: `eval: Option<EvalFn>`, the body above
written in Rust, which makes the output **dual** — a varying number where it is fed fields
and a uniform number where it is fed uniforms ([Dual outputs](#dual-outputs)).

### The `node!` macro

A node that is nothing but WGSL — a port list, an option list and one body per output — is a
declaration rather than a struct literal. `nodes::macros::node!` expands to exactly the
`const NodeDef` above, so the registry, the compiler, the menu and the tests cannot tell one
from the other, and a node moves out of the macro the day it grows a `cpu` half.

```rust
node! {
    CIRCLE,
    slug: "circle", icon: "🔵", label: "Circle", category: Generate,
    tooltip: "A circle with an adjustable radius, position and edge softness.",
    inputs: [
        VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
        VaryingColor "background" "Background" = Control::color("#000000ff"),
        VaryingNumber "radius" "Radius" = Control::num(0.5, 0.0, 2.0, 0.01, "⬓"),
        VaryingNumber "softness" "Softness" = Control::num(0.01, 0.0, 0.5, 0.001, ""),
    ],
    wgsl_common: "    let radius = {radius};
    let softness = {softness};
    let mask = 1.0 - smoothstep(radius - softness, radius + softness, length(uv));
",
    outputs: [
        VaryingColor "color" "Color" = "    return mix({background}, {foreground}, mask);",
        VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
    ],
}
```

**A body is WGSL with `{key}` holes in it**, and a hole is filled by `ctx.input` — the same
one-character abstraction boundary a hand-written generator has. Two things follow from the
splice happening at run time rather than through `format!`: WGSL braces need no doubling,
because only an exact `{key}` is a hole; and a hole nobody wrote is never asked for, so an
output that ignores an input neither declares its uniform nor drags that input's producer
into the shader.

`wgsl_common` is WGSL prepended to every one of the node's bodies — the coverage test a color
and its mask share — and may be left out. An input may name the uv it is sampled at, with
`at "cellCenter"` before its `=`, which is how a polka dot reads its radius at the center of
the dot it is drawing rather than at the fragment; an output may name the range its maths
bounds it to, with `in "[0, 1]"` before its `=`, and its Rust twin with `eval(f)` after that
([Dual outputs](#dual-outputs)); and an option may say `free "placeholder"` after its
choices — with `checked by f` after that — for a row that is a typed field with those choices
as presets. A body whose WGSL depends on an option is `varying(|node, ctx| …)` instead of a
string; `ctx.option` is how it asks. `wgsl_utils: [...]` names the module-scope WGSL
functions the bodies call — a noise gradient, a hash — and they reach `NodeDef::wgsl_utils`,
which the compiler emits once into a shader that touches such a node and never into one that
does not.

`timing: Timing::periodic(0.5),` after `width` and before `inputs:` makes the node one
that moves with time, and reaches `NodeDef::timing` — [Timing](#timing); `timing_xy:` is the
same on two axes. The node writes no time row itself: the macro puts Time, Speed and Offset
after the `inputs:` list and before an optional `after_time:` list, and the Timing heading
and its mode after the options. A body reads where the node is through the Time hole and
Offset's, by the prelude's helper for its kind of period: `time_periodic({clock},
{phaseOffset})` is the cycle on a periodic node, `time_repeat({clock}, N, {phaseOffset})` its
time round a period `N`, and `time_unbounded({clock}, {phaseOffset})` its time on a line. The
arm adds no `cpu` half and no output: the Time hole is a count the synth writes each tick,
and a body never reads Speed.

Every definition also names a `Category`, which `NodeDef::EMPTY` supplies a default for and
`every_node_declares_a_category` insists is a real one. The Nodes menu is `Category::ALL`
crossed with `REGISTRY`, so a new node appears in it by existing — silvia keeps a list of
slugs beside its menu, and that list is the one place a new node there has to be added twice.

A definition may also carry `hidden` controls: inputs with **no port and no row**, stored on
the node, saved and undoable like any control, and edited somewhere other than a row of the
node's body. An audio source's band center and Q are the case they exist for — the handle on
the scope edits them — and a number a node draws in a region of its own is the same thing:
`cosinegradient`'s twelve coefficients and `euclideanrhythm`'s twelve lane numbers are hidden
controls laid out by [a region](ui.md), each cell the same inset s-number a row would have
carried. A hidden control is still a control everywhere else: the compiler resolves one to
the same `u.u_control_…` uniform, the loader seats it by key, the command bus writes it,
`capped_by` moves its range, and MIDI binds it under the same `{node, key}` a port's control
has. A definition's `unbindable` names the controls MIDI does not bind — an Output's render
numbers — and a registry test holds each to being a control of its node, as one holds that a
hidden control has no port.

A node that computes something outside a shader adds a `cpu` half — see [CPU
nodes](#cpu-nodes), and [cpu.md](cpu.md) for the whole model — and everything else about it
stays data.

Why data and not a trait: a node with several outputs is several `OutputDef`s, each carrying
its own generator, rather than one method dispatching on a port key. **Adding an output
without a generator is then a compile error** instead of a runtime panic on the day someone
connects it. 36 of silvia's 146 nodes have two or more generators and `math.js` has 14, so
that dispatch would not have been an edge case.

The registry is `&'static [&'static NodeDef]`. No trait objects, no vtables.

## Outputs are ports, and a node has as many as it declares

A `VaryingColor` output is an **expression**. `OutputKind::Shader` inlines one into the
consumer's module; `OutputKind::Texture` samples a frame the node's `tick` published. Nothing
limits how many of either a node declares, and `video` has two textures — its picture and its
oscilloscope — beside three uniform numbers and three actions.

**Every one of them is addressed by its port.** `publish_frame` takes a port key, `App::frames`
and `FrameJob::sources` are keyed by `PortRef`, and the renderer's upload and uniform map are
too, so two textures on one node are two uniforms sampling two frames. Three tests pin it —
`tests/compile.rs` for the naming, `tests/video.rs` for the publish, and `tests/gpu_upload.rs`
for the two textures actually reaching the GPU.

## The generator contract

A generator is `fn(node, ctx, func) -> String`, and it **returns only the function's body**, in
WGSL. The signature is derived from the port's `PortType`:

- `PortType::VaryingNumber` → `fn name(uv: vec2f) -> f32`
- `PortType::VaryingColor` → `fn name(uv: vec2f) -> vec4f`
- `PortType::UniformNumber` → no function. The output carries `no_wgsl`, and the compiler
  resolves the port to a uniform before it could ever call it. `Action` is the same, and never
  reaches the compiler at all.

A measurement is `fn name(p: vec2f)`, and its body may also write to the tap buffer, `tap`, at
the offset `ctx.tap_slot` returned; the buffer is declared only in a shader with at least one
slot. Its coordinate is `p`, the point `fs_main` handed it, and not `uv`.

`f32` and `vec4f` are the only two *function* shapes there are — across all 201 generator
functions in silvia there are no others. Deriving the signature means the declared port type
and the emitted return type cannot disagree.

`ctx.input(node, key, uv)` returns the expression to splice in. What comes back may be a
function call, a uniform, a prelude global, or a fallback, and the node neither knows nor
cares: all four are valid WGSL of the right type. **That interpolation site is the
abstraction boundary, and it is one character wide.**

## WGSL

The renderer draws WGSL, wgpu's own language, which naga validates and writes out as MSL
on Metal, SPIR-V on Vulkan and HLSL on Direct3D 12 with nothing between ([proposals/shader-path.md](../proposals/shader-path.md)).
The compiler's entry points are `compile::wgsl::build` for an Output, `build_probe` for the
cost probe, `build_pass` for a workspace's
[pass](rendering.md#the-workspace-pass) — its measurements and its thumbnails — and
`build_measure` for one node's measurement alone; each returns a `Shader` whose module holds
both entry points. An output nobody has written a body
for gets `no_wgsl`: a magenta (or zero) placeholder and a `Diagnostic::Untranslated`, so the
module still validates.

**The function** is `fn name(uv: vec2f) -> vec4f` for a color and `-> f32` for a number, and
a measurement is `fn name(p: vec2f)`. A parameter is immutable: a body that moves its
coordinate declares `var st = uv;` first.

**What `ctx` hands back.** Every uniform is a member of the module's one struct `u` —
`u.u_control_perlin3_scale`, `u.u_opt_mix1_method` (an `i32`) and
`u.u_float_autoexposure2_gain` — and the fallbacks are
`defaultUvMap(uv)`, `0.0` and `vec4f(0.0)`. The compiler writes the struct, the bindings and
`fs_main`; `compile::wgsl::uniform_layout` and `bindings` tell the renderer where each one is,
from the `Shader` alone.

**A texture** is two names: `ctx.texture_uniform(node, port)` is a `texture_2d<f32>`, and
`ctx.sampler(node, port)` is the one of four shared samplers that port's `wrap` and `filter`
pick. **Sample with `textureSampleLevel(tex, sampler, t, 0.0)`, always.** `textureSample`
takes implicit derivatives, which WGSL allows only in uniform control flow; naga does not
check that, and a driver answers garbage outside it. No texture here has mip levels, so level
0 is the whole picture. `textureSize(t, 0)` in a silvia body is
`vec2f(textureDimensions(t))`, `texelFetch(t, p, 0)` is `textureLoad(t, p, 0)`.

**The tap buffer** is `tap: array<atomic<u32>>` at binding 1, and every access to it is atomic:
`atomicAdd(&tap[i], v)`, `atomicMax`, `atomicMin`, a write is `atomicStore(&tap[i], v)`.
A float's bits are `bitcast<u32>(x)`. `tap::stats_wgsl` and `tap::color_wgsl` are the
shared measurements, and the probe's counting is the compiler's.

**A helper** in `wgsl_utils` is a module-scope function, and WGSL has no overloading: a helper
that wants two types is two names (`mod289_v3`, `mod289_v4`), and a name is unique across the
whole registry, which `every_wgsl_util_name_is_unique` checks by parsing every util into one
module. The prelude, `compile/prelude.wgsl`, has `PI`, `floor_mod` (and `floor_mod2`, `3`,
`4`), `whole_mod` for a clock's whole part (in integers, which Metal's fast math cannot fold into
the fraction added after it), `hash2`, `hsv2rgb2`, `defaultUvMap` and the vertex stage.

**Every loop in the library** is a `for` over an integer counter that ends within a constant
number of iterations: a literal bound, or an option's value the generator writes in as one,
with a `break` for a count that is a field — the fractal noise's `for (var i = 0; i < 8;
i++) { if (i >= octaves) { break; } … }`. `lyapunov` has no count to read: its exponent runs a
fixed ten, `fractals::ITERATIONS`, with no control, port or option to change it (see
[decisions.md](decisions.md#a-number-stays-varying)). No node writes a `loop {}` or a `while`, and no test holds
either claim. naga adds a counter to every loop it translates, and its cost falls on nodes
that run many iterations per pixel; [loop-bounding.md](loop-bounding.md) lists every loop,
the three that end by a clamp or a direction of travel rather than a literal, and what a loop
costs.

**Porting a silvia node.** silvia's shaders are GLSL, and these are the translations that
bite:

| silvia's GLSL | WGSL |
| --- | --- |
| `float`, `int`, `uint`, `vec2`…`vec4`, `ivec2`, `uvec2` | `f32`, `i32`, `u32`, `vec2f`…`vec4f`, `vec2i`, `vec2u` |
| `float x = …;` | `let x = …;`, or `var x = …;` if it is assigned again |
| a global `float g;` | `var<private> g: f32;` |
| `mod(x, y)` | `floor_mod(x, y)`, never `%`: WGSL's `%` truncates and differs for negatives |
| `atan(y, x)` | `atan2(y, x)` |
| `c ? a : b` | `select(b, a, c)`, which evaluates both; an `if` where one side writes or costs |
| `x * i` with `float x`, `int i` | `x * f32(i)`: no implicit conversion, except a literal |
| `clamp(v, 0.0, 1.0)`, `max(v, 0.0)`, `pow(v, 2.2)`, `step(0.5, v)`, `smoothstep(0.0, 1.0, v)` on a vector | splat the scalar, `clamp(v, vec3f(0.0), vec3f(1.0))`; `mix(a, b, t)`, `v * s` and `v + s` need none |
| `c.rgb = x;` | `c = vec4f(x, c.a);` — a single component, `c.r = …`, is fine |
| `a == b` on two vectors, as a `bool` | `all(a == b)` |
| `float k[3] = float[](…)` | `var k = array<f32, 3>(…)` |
| an `out` parameter | a returned struct |
| `for (int i = 0; i < n; i++)` | `for (var i = 0; i < n; i++)` |
| `if (x) y();` | `if (x) { y(); }` — a brace is required even for one statement |
| `<<`, `>>` chained, or mixed with `&`/`|` unparenthesized | each shift its own parenthesized step, and the shift amount a `u32`: `((by & 2) >> 1u) << 3u` |
| `c.rgb *= x;` | rebuilt like a plain swizzle assignment: `c = vec4f(c.rgb * x, c.a);` |
| `smoothstep(e0, e1, x)` with `e0 > e1` | undefined in MSL, and naga neither warns nor rewrites it: `1.0 - smoothstep(e1, e0, x)`, the same curve with the edges back in order |
| `floatBitsToUint(x)` | `bitcast<u32>(x)` |
| a name WGSL reserves: `mod`, `filter`, `common`, `target`, `smooth`, `type`, `self`, `set`, `match` | another name |
| a local named `u` | shadows `u`, the module's uniform struct instance, so every `u.u_…` read after it silently reads the wrong thing and nothing catches it — never name a local `u` |
| a helper inside a `node!` body named `wgsl_common` | collides with the `fn wgsl_common` the macro itself defines and shadows it — pick another name |

Constant expressions are evaluated when the module is parsed, and one that overflows is an
error; a run-time `u32` sum still wraps, which a tap's carry relies on. `fs_main` builds `uv`
from `@builtin(position)` with no flip, so row 0 of an Output's texture is the bottom of its
picture, and a CPU source's `1.0 - …` on `v` reads its top-first bytes the right way up.

`tests/shader_targets.rs` is the gate: every module — each node unconnected and connected,
each probe, every code option's every choice — is parsed, validated with the capabilities
wgpu gives a plain device, and written as MSL, SPIR-V and HLSL, and its struct and bindings are held
to `uniform_layout` and `bindings`; it refuses `textureSample(`, `dpdx`, `dpdy` and `fwidth`
outright, and a local named `u` outside the module's own `var<uniform> u: Uniforms;`.
`tests/compile.rs` snapshots every node as `wgsl_{group}_{slug}_{wiring}`; a new node's first
run writes none of them, so `INSTA_FORCE_PASS=1 cargo test --test compile` then
`cargo insta accept` takes every new snapshot in one pass rather than one prompt apiece.
The renderer's own stages and `slimemold`'s kernels are WGSL too: a kernel's barrier is
`workgroupBarrier()`, its shared memory `var<workgroup>`, an `r32f` image is
`texture_storage_2d<r32float, read_write>`, and the arrivals are `array<atomic<u32>>`.

## Input fallbacks

`Control` says what an input falls back to when nothing is connected:

| Variant | Unconnected behavior |
| --- | --- |
| `None` | `defaultUvMap(uv)` for a color, `0.0` for a number |
| `Number { default, min, max, step, unit, log }` | a `float` uniform, and an s-number control on the node |
| `Color { default: "#rrggbbaa" }` | a `vec4` uniform, and an s-color swatch |
| `Press` | on an `Action` input only: a momentary button in the row, reported as a level — see [cpu.md](cpu.md#the-event-half) |

`Control::num`, `Control::num_log` and `Control::color` are `const` constructors for these,
and are what a node file uses — the struct literals are for matching. `num_log` scrubs in log
space, which a range spanning decades — a zoom of 0.01 to 100 — needs, or a linear drag
spends its whole travel below 1.

`Control::num_capped` is a fourth: a number whose **top end is another input's value**. The
command bus writes the named control's number into this control's own `ControlRange` and
refits the value, inside the same command, so a kaleidoscope's Source Segment stops at the
segment count and comes down with it. It follows the control, not the port — a cable into the
capping input leaves the range alone. `kaleidoscope`'s `sourceSegment` is the one there is,
and `nodes::capped_ranges` is where the question is answered.

A typed value with its own row and no port — Lyapunov's `sequence` — is not a `Control`
variant: it is an `OptionDef` with `placeholder` set, below.

## A number stays varying

An input on a GPU node is a `VaryingNumber`, whether it is a strength, a radius, a seed or
a count of segments: a count that varies across the frame is a picture a person makes on
purpose. A `UniformNumber` input on a GPU node is the exception, kept for Time — one moment
per node, which the CPU reads as well ([Timing](#timing)) — and Speed, its rate — for a whole
number the node's cycle is built on, Rotozoom's Turns, which as a field would stop the node
coming back every cycle, and for an input the node could publish a uniform of its own from,
and weighed even then. A CPU node's inputs are uniforms by construction. See
[decisions.md](decisions.md#a-number-stays-varying).

## A node's own values

A node instance keeps three kinds of state. What separates them is who draws them.

| | stored under | drawn by | example |
| --- | --- | --- | --- |
| `controls` | an input port's name | the port's `Control` | a frequency knob |
| `options` | an `OptionDef`'s name | one piece of shared code, for every node | a blend-mode select |
| `values` | a `ValueDef`'s name, or the input a range narrows | the node's own `ValueKind` | a note's text, an `automation`'s recording, a `stepsequencer`'s pattern |

**An option is portable.** Its row is drawn by code that never learns which node it belongs
to. That code knows four shapes: a select, a file button, a tick, a one-line field. So a node
can declare a choice and get a row for free, without anyone writing drawing code for it.

**A value is drawn by the node's own code**, so the list of shapes stays open. There are
four: `Text`, a box of prose; `Points`, a curve a hand performed — `{ time, value }` each,
with the ends it is drawn against on the declaration; `Cells`, a grid of `lanes` by
`steps` a hand lights one cell at a time — `stepsequencer`'s pattern, stored as one string a
lane, `x` for a lit cell and `.` for an unlit one, the notation every rhythm table uses, so the
`.ssw` reads as the pattern it plays, a lane shorter than the grid or any character but `x` is
unlit, and a new node holds no value at all, which is every cell dark; and `Painting`, a
picture a hand painted — RGBA8 pixels behind an `Arc` in memory, and in the `.ssw` the
reference to a PNG the project's save writes into `assets/`, named by its content (see
[decisions.md](decisions.md#a-painting-is-saved-with-the-project)). A pad, a step grid and a
paint surface arrive with the rest of the node-body controls.

A value is a row of the node unless its kind takes none: `Points`, `Cells` and `Painting` are
drawn in one of the node's own regions instead — the ordered list in `NodeDef::regions` —
because a curve, a grid and a picture want the width of the body, and `ValueDef::rows`
returning zero is what says so. The region reads the saved value straight off the `Node`, so a
patch just opened draws its performance, its pattern or its picture before anything has ticked.
A region that edits one sends `RegionEvent::Value`, which is the `SetValue` a click is: one
step back.

A value the *tick* writes goes through `TickContext::write_value`, which is
`write_control`'s sibling and takes the same seam: the synth's graph now, the document through
the command bus a frame later. `automation` writes its recording that way when the recording
stops, which is one step of the undo history per performance.

There is one rule a `ValueKind` may not break: it cannot make `ui/` special-case a particular
node. The kind *is* what `ui/` draws from. That is what keeps node-specific drawing out of a
module every node shares.

A control's range on one particular node is also a value. It is stored under the name of the
input it narrows, not under a `ValueDef`, because every number control can have one and there
is nothing for a node to declare. A registry test stops a node declaring a value with the same
name as one of its own ports, which is the only way those two could collide.

None of the three is ever a step away from being saved. All three are document data: undoable,
and written into the `.ssw`.

## Option kinds

`OptionDef::kind` says what changing an option costs, and it is the only thing that separates
the four. It is the [recompile boundary](architecture.md#the-recompile-boundary) applied to
options: an option that does not change the emitted WGSL has no business paying for a rebuild.

| Kind | Read by | A change costs |
| --- | --- | --- |
| `Code` | the generator, through `ctx.option` | **a shader rebuild** — the choice is baked into the emitted WGSL |
| `Uniform` | the shader, through `ctx.option_uniform` | one `i32` in the uniform buffer: the index of the chosen value among `choices` |
| `Asset` | the renderer, as a texture to bind | nothing in the shader |
| `Runtime` | the node's `tick`, or the renderer | nothing in the shader; it lands on the next frame |
| `Presentation` | only the canvas | nothing at all |

`Code` is the default and most options are one. **`Uniform` pays the cost the other way**:
every branch is in the compiled program whether it runs or not, and the driver allocates
registers for the worst of them. So it is right where a rebuild is the thing that must not
happen — a crossfade method changed in the middle of a set, a tap's `jitter` turned on while
it is reading — and wrong for a choice that
changes the shape of the code rather than one term in it. A `Code` option with cheap,
similar branches is the case that can go either way, and the question to ask is whether
anybody changes it while the picture is live.

An **`Asset`** carries no `choices`: it holds a path into the project, the canvas offers a
file button instead of a select, `accepts` says which files, and `every_node_compiles_on_the_gpu`
treats it as one value. It is how a node names a file. `video`'s `file` is the one there is.

A `Code` option's row may also be **free text** rather than a closed list, when
`OptionDef::placeholder` is `Some`: the canvas draws an always-open field instead of a select,
its `choices` offered as presets a person can still type rather than the only values
`option_is_valid` accepts, and `validate` — a plain `fn(&str) -> bool` — says what the row's
border reads while what is typed would otherwise silently compile as the default.
`lyapunov`'s `sequence` is the one there is: `parse_sequence` already accepts any string its
grammar describes, so the option needed a field, not a new kind — the cost of changing it is
still `Code`, only how a person reaches a value differs. See
[decisions.md](decisions.md#a-free-text-option-not-a-new-control) for why this is an
`OptionDef` field and not a `Control` variant.

A **`Runtime`** option is read while running rather than while compiling: a camera's device
and a clip's loop are read by `tick`, and an Output's resolution is read into every frame's
job, from which the renderer resizes while keeping the program. A node whose outputs are all
`UniformNumber` or `Action` — `mastergear`, `counter` — emits no WGSL at all, which puts every
option it has here.

A **`Presentation`** option reaches no shader at all — `audio_ports::SHOW`, how much of an
audio node to draw — and gets the same answer `SetCollapsed` does: the ports and the cables
are untouched, so the shader that was running still describes the graph.

**Both directions are asserted, in `tests/compile.rs`.** A `Code` option whose choices all
generate identical WGSL fails `every_code_option_changes_the_shader` — it is a driver recompile
mid-set for nothing, and wants another kind. An option outside `Code` whose value *does* reach
the WGSL fails `no_option_outside_code_reaches_the_shader` — that one is a correctness bug
rather than a slow one, because nothing rebuilds and the running shader would describe a graph
that no longer exists. That second law is why `Code` stays the default: the safe mistake is
the slow one.

Registry tests hold each kind to its shape: an `Asset` has no choices, everything else has
them, and every option that has choices has its default among them — a `Uniform` doubly so,
because `index_of` falls back to the default's index and a default that is not a choice would
silently read as zero.

An option may also name an input that answers its question better than it does, in
`OptionDef::overridden_by`. While something is connected to that input the generator reads
the cable and not the choice, and the canvas draws the select **disabled**, showing the
input's label in place of the chosen value. It is a field rather than a case in the canvas,
so `ui/` draws the state without knowing which node it belongs to, and a registry test holds
the key to a real input of the same node. `tap.measure` is the one that has it.

## Timing

There is one clock, the transport's playhead in seconds ([cpu.md](cpu.md#the-transport)).
A node that moves with time says so once, in **`NodeDef::timing: Option<nodes::Timing>`**,
and `nodes::timing` — whose module doc is the one place these rules are written beside the
code — expands everything else from that declaration: its time rows, its heading, its mode,
the WGSL it reads them through, what a CPU node reads, and what the loop arithmetic reads.

**A `Timing`** is the node's pace, whether a new one stands still, its period and its axes.
`pace` is how many of its own cycles one second is, at a Speed of 1 in Free mode and on
ambient time in Loop mode; `still` starts a new node's Speed at 0, as silvia keeps it still;
`period: fn(&Node) -> Option<f64>` is
how long its picture takes to come back in its own cycles by what its options and controls
say — one for a periodic node, a noise's Repeat, a fraction of a cycle where a setting brings
the picture back sooner, `None` for a picture that never repeats; and `axes` is one, or X and
Y on Shaky Cam, whose Y keeps a pace and a period of its own (`pace_y`, `period_y`, read
through `Timing::pace_of` and `Timing::period_of`). `Timing::periodic(pace)` is a periodic
node, `Timing::repeating(pace, period)` one with a period of its own, `.still()` one that
stands still when new, `.xy()` a second axis, and `.y(pace, period)` a second axis with its
own cycle.

**One cycle is the least the node comes back after as it opens**, at its defaults, so a gear's
turn, the loop meter and the picture agree: Rotozoom's and Shaky Cam's X's 20π of silvia's
time, Shaky Cam's Y a quarter of that, the tunnel's flight of 64 units. **A period is the
least the picture comes back after, or none is claimed**, and a setting can shorten it to a
fraction of a cycle: a coefficient at zero stills the waves it scales and what still moves
comes back sooner — Rotozoom with its cosines off and an even Turns every half cycle, Shaky
Cam's X with its cosine off every tenth — and a straight tunnel, Twist at zero, every eighth
under Mirror. A period reads the node's controls, which keep the knob a cable has replaced, so
`nodes::timing::period_in` takes every cabled control off the node first and a period reads a
missing control as any value it could be: the period every value shares, the cycle on each of
these.

**Two modes, so the first time row means one thing at a time.** The option `clockMode`
(`nodes::timing::MODE`), values `free` and `loop`, shown "Free" and "Loop", default `free`:

- **Free** — the row is **Speed** (key `speed`, `nodes::timing::SPEED`): a uniform number with
  a knob and a port, −4 to 4, a multiple of the node's pace. 1 is its normal pace, 0 stands
  still, negative runs backwards. Its knob starts at 1, so a new one moves as silvia's does,
  and at 0 on a `still` one that silvia keeps still. The
  synth integrates it against the transport's advance — the same advance a gear integrates,
  never a frame's raw `dt` — into the node's own playhead in `f64`, one per axis
  (`nodes::timing::Pace`), and publishes `playhead × pace` under the Time key. So it pauses
  with the show, follows a seek by `speed × the jump`, wakes from a closed tab having moved by
  the gap, and is born where the playhead puts it, `speed × playhead`, as a Master Gear is: at
  Speed 1 it reads exactly what Loop mode's ambient reading does, and a render, which starts
  every playhead again, is deterministic. A knob turned glides to its new value over 50 ms in
  closed form, so a stepped knob bends the motion rather than kinking it and lands on the same
  phase at 30, 60 or 144 frames a second. A cable into Speed — an LFO, an envelope — changes
  how fast the node runs, not where it is.
- **Loop** — the row is **Time** (key `clock`, `nodes::TIME`): a diamond with **no knob**, in
  the node's own cycles. Unplugged it is ambient time, `playhead × pace`, so every node in
  Loop mode moves with the show, a `still` one too. Plugged, whatever
  arrives **replaces** it — usually a gear's Cycles, which drives the node exactly and closes
  a loop to the bit. Time is one moment per node, which the CPU reads as well as the shader,
  so a field cabled into it is an ordinary type mismatch. Where Speed's knob stands in Free
  mode, the row carries a **loop meter**: the node's period cut into its cycles and filled by
  how far through it the node's Time is, `3/4` in its middle, or on a picture that never comes
  back the cycle it is in with its right end open (`nodes::timing::Progress`,
  [ui.md](ui.md#the-loop-meter)).

**Time reads a count whole.** In both modes the synth publishes where the node is under the
Time key, `u_count_{slug}{id}_clock`: a gear's Cycles, the Time node's Seconds, the ambient
reading and a free-running playhead are counts, which a CPU node reads in `f64` and a shader
as its whole part and its fraction ([cpu.md](cpu.md#gears)), so a Time is as precise a
million cycles on as at the first. A CPU node's own reading, with nothing cabled in, is
published under its Time key by `TickContext::cycle`, before its Offset and at a clip's own
rate, for the loop meter to read. Anything else in a Time — a Phase, an oscillator, a count
through a Math node — is the one `f32` that arrives. A body never reads Speed.

**Switching mode** puts one row away and shows the other in place, at the same height, so the
node does not grow or shrink. The cable in the row that goes away is dropped by the same
step (`Graph::drop_inactive`, from `SetOption`), one undo: a gear's Cycles, a growing count,
left in a Speed would race off as a rate. A cable can never land on the row a mode puts away
(`nodes::timing::is_inactive`, which `Graph::can_connect` refuses as `ConnectError::Inactive`),
from a hand or from a file.

**Offset** (key `phaseOffset`, `nodes::timing::OFFSET`) is in the node's own cycles — 1 is one
cycle, or one unit on a node whose picture never repeats — from a knob at zero that adds
nothing, and it is **added** in both modes: to Time in Loop mode, to the node's own playhead in
Free mode. On a node that draws it is a varying number, so a field makes a ripple with one
cable: a radial into Offset spreads the picture outward as rings. On a CPU node, whose tick has
no pixel, it is a uniform number.

**Its knob reaches one whole period either way**, `−P` to `P`, by the period `P` the node's
options give it now (`Timing::period`, read by `nodes::timing::range`, which
`nodes::control_range` asks): −1 to 1 on a periodic node, −4 to 4 on a Perlin at Repeat 4 and
−16 to 16 at 16, a quarter either way on the tunnel's Helix, a sequencer's bars where its
lanes meet again, and −1 to 1 where the picture never comes back — Repeat Never, Depth Wrap
None, a clip on Hold. The range follows the options live, with nothing stored on the node, so
the range editor's Default column says it too and a hand's own range, once set, stays where it
was put. Its step is the finest of 0.001, 0.01, 0.1 and 1 that is at least a thousandth of `P`,
so a drag across ±1 and one across ±64 are about as long. **An edit that shrinks the period**
— a Repeat lowered, a depth wrap turned off, a lane shortened — takes a stored Offset round the
new period in the same undo step (`nodes::timing::fit_offsets`), keeping its sign: 10 at
Repeat 16 is 2 at Repeat 4, which draws the same picture; with no period left it is clamped to
one cycle either way, since nothing else draws the same. A value typed or loaded outside the
range is clamped, as every control's is. A field cabled into Offset is added as it arrives,
whatever the knob's range.

The time rows are adjacent — Time, Speed, Offset, one of the first two shown — folded under a
**Timing** heading (`nodes::timing::HEADING`, key `timing`) that starts closed, with the mode
as two segments, Free and Loop, at the right end of its bar, drawn whether the heading is open
or closed ([ui.md](ui.md#options-and-the-file-button) has the bar).

**What a body reads.** One prelude helper per kind of period, each taking the Time count and
Offset: `time_periodic` for a node that comes back every cycle, the fraction plus Offset;
`time_repeat(time, n, offset)` for one that comes back every `n`, the whole part reduced by
`n`, then the fraction, then Offset; and `time_unbounded` for one that never does, the whole
count plus Offset. The tunnel is `time_periodic`, a cycle a flight of 64 units. The body
takes the result round its own period where it needs to.

**The rate at rest is silvia's default speed** in the node's own cycles, so a node dropped in
moves as silvia's does. Where silvia's period was 20π it is rounded to whole seconds, which
is under 5% off. Where silvia is still at its default, the rate is zero — in Loop mode the
node sits still until a gear is cabled into its Time and Offset's knob places it — and its
pace is one chosen to look natural at Speed 1, with Speed starting at 0.

| node | one cycle, or one unit | rate at rest (Loop) | pace at Speed 1 (Free) | Speed starts | the body takes Time round |
| --- | --- | --- | --- | --- | --- |
| `mandelbrot`, `juliaset` | one drift of the Map orbit | 0.5: a drift every 2 s | 0.5 | 1 | 1 |
| `cosinegradient` | one shift of the palette | 0: still | 0.1: a shift every 10 s | 0 | 1 |
| `rotozoom` | silvia's 20π super-cycle: one set of zoom waves | 1/60: a cycle a minute | 1/60 | 1 | 1 |
| `shakycam` | X: the 20π super-cycle; Y: a quarter of it, where its two waves line up | 1/60 on X, 1/15 on Y | 1/60 on X, 1/15 on Y | 1, per axis | 1 |
| `geissflow` | the 20π flow cycle | 1/160 | 1/160 | 1 | 1 |
| `perlin` | a lattice cell along the time axis | 0.5 cells a second | 0.5 | 1 | `N` under Repeat, else nothing |
| `simplex`, `fractal`, `domainwarp` | a lattice cell | 0: still | 0.5 cells a second | 0 | `N` under Repeat, else nothing |
| `static` | a roll | 0: still | 6 rolls a second | 0 | `N` under Repeat, else nothing |
| `tunnel3d` | a flight of 64 units of camera depth | 1/128: half a unit a second | 1/128 | 1 | 1 while the depth wraps, else nothing |
| `oscillator` | one wave | 1: a wave a second | 1 | 1 | read unwrapped on the CPU |
| `video`, `imagegif` | one play of the clip | 1 ÷ the clip's length: its native speed | 1 ÷ the clip's length | 1 | read unwrapped on the CPU |
| `stepsequencer`, `euclideanrhythm` | one bar of sixteen steps | 0: stopped | 0.5: a bar every 2 s, 120 BPM | 0 | read unwrapped on the CPU |

Rotozoom, Shaky Cam and Geiss Flow keep silvia's uneven wave rates inside their cycle — 1,
0.7, 0.8 and 1.2, which line up over 20π — and scale the cycle, `time_periodic`, back to
silvia's units in the body, so their knobs and their looks are hers: Rotozoom's waves run 10,
7, 8 and 12 times a cycle. Shaky Cam's X runs its 1 and 0.7 as 10 and 7 waves a cycle, and its
Y, whose 0.8 and 1.2 line up four times over 20π, runs them as 2 and 3 a cycle of its own at
four times the pace, so the shake on screen is silvia's and each axis's cycle is its true
repeat. **Rotozoom's Turns**, a
whole number from −10 to 10, default 5, is how many turns one cycle makes: silvia's equal
speeds give five, 4π into 20π. Zero is zoom alone and a sign reverses the turn; being whole, it
is a ratio inside the node, which still comes back every cycle. **Shaky Cam has its time rows
per axis** — Time X, Speed X and Offset X keyed `clock`, `speed` and `phaseOffset`, Time Y,
Speed Y and Offset Y keyed `clockY`, `speedY` and `phaseOffsetY` — under one mode, so Y can
shake alone, at a speed of its own or on a gear of its own; all six rows fold under the one
Timing heading.

**Repeat.** A noise's picture never comes back on its own, so `perlin`, `simplex`, `fractal`
and `domainwarp` carry a **Repeat** option: Never, the default and silvia's look, or every 1,
2, 4, 8 or 16 cells. `static`'s is Never or every 4, 8, 16, 32, 64 or 128 rolls. With a length
`N`, a noise walks a circle of circumference `N` through a four-dimensional noise, turned
`time_repeat(Time, N, Offset) ÷ N` (`loopCircle`, with `PERLIN4D_WGSL`, `SIMPLEX4D_WGSL` and
`FBM_LOOP_WGSL`, Gustavson's `webgl-noise`), so its picture comes back every `N`; Static reads
its roll modulo `N`. Every `N` divides the 40320 the whole part wraps at, so a noise on a gear
never meets a seam. It is a `Code` option and rebuilds, and a person chooses it, because a
circle through four dimensions is not the line through three and the picture changes. Offset
on a noise does not wrap unless Repeat is on, and then it wraps at `N`, which is also how far
its knob reaches either way. At rest, Perlin at
Repeat 4 comes back every 8 s; driven by a gear at a cell a cycle, Repeat 1 comes back every
cycle.

**The tunnel's path is tuned so its flight repeats.** Every path frequency is silvia's times
5π/16: Sine's and Lissajous's 0.3 and 0.5 are 0.2945 and 0.4909, and the Helix's 0.4 is
0.3927. Sine and Lissajous then come back every 64 units of depth and the Helix every 16,
each a whole number of the wall's depth wrap, 8 units under Mirror and 4 under Repeat. One
cycle is a flight of 64 units, which the camera's depth is Time's fraction plus Offset of, so
a Time a cycle on draws Time 0 to the byte at any count. Its period is what its path and its
wall come back after together: a cycle on Sine and Lissajous, a quarter on the Helix, and with
Twist at zero — a straight tube — the wall's own eighth or sixteenth. Depth Wrap None never
repeats: the camera's depth still turns round the flight, which every path comes back after,
and the wall is read on from the whole flights behind it.

**What a CPU node reads.** No uniform is written for a CPU node. Its tick asks
`TickContext::cycle(id)` — where the node is with its Offset added: in Loop mode what is
cabled into Time, a count read whole in `f64`, or the playhead times its rate, and in Free
mode its own playhead times its pace — or `cycle_at(id, rate)` where the node works out its
own rate, a clip's one play over its length, which is both its rate and its pace. `oscillator`
plays its wave at that in waves, `video` and `imagegif` play it in plays of the clip, and the
sequencers step on crossings of `floor(16 × cycle)`, in bars — backwards, in reverse order,
under a negative Speed in Free mode, and never across a seek. Each is in
[the library](#the-library) below.

**Stateful nodes are not on Time.** A simulation, an envelope or a filter steps from wherever
it is, on the transport's `dt`, clamped at `transport::MAX_DT`, 0.1 s. A pause holds it and a
seek carries it across. Each keeps a rate of its own, labelled for what it is rather than as
a speed:

| node | what it keeps | label |
| --- | --- | --- |
| `slimemold` | steps owed at the label's times 30 a second | **Rate**, ×30/s |
| `cellularautomata` | generations owed at the label's times 30 a second, the fraction carried | **Rate**, ×30/s |
| `smoothcounter` | the ease toward its target | **Rate** |
| `autoexposure` | how fast the gain approaches the one it wants | **Response** |
| `stargate` | the drag, in pixels per drawn frame | **Drift** |

`animation`, `automation`, `adsr`, `slew`, `autogain`, `brickgame`, `xypad`,
`clockdivider`, `randomfire` and an Output's feedback keep their own rates and their own
starts: an envelope is something a person or an event starts, not a clock. `clock`, the time
of day, is live and outside the model.

**Offset is only ever the time input.** An input that shifts something else is named for what
it shifts, its key still silvia's `offset`: the oscillator's level added to the wave is
**Level**, `clock`'s hours from UTC **Zone**, the two gradients' and the kaleidoscope's slide
**Shift**, `chromaticaberration`'s parting of the channels **Spread**, Emboss's flat gray
**Gray**, and `stargate`'s slit **Position**. A compound label cannot be mistaken for the
time input, so Translate's X Offset and Y Offset, Tile's Offset X and Offset Y and the Slime
Mold's Sensor Offset keep theirs.

### When a loop closes

Nothing in the app switches into a loop. Whether a picture comes back is a property of the
clocks it is on, read by `nodes::chain` from its gears and from each node's `Timing` and mode:
the caption under every Master Gear reads it, and `examples/loop_gifs` renders each
workspace's Output through the ordinary render for as long as its Master Gear says. The rule:

- A node on a chain of Ratio Gears rooted at a Master Gear `M` advances `m × Πr ÷ P` of its
  periods over `m` cycles of `M`, `Πr` the product of the ratios on the chain and `P` its
  period in its own units (`Timing::period`: one on a periodic node and a looping clip, `N`
  under Repeat, a quarter on the tunnel's Helix, `lcm(16, lanes) ÷ 16` bars on a
  sequencer, where its lanes and the bar's sixteen steps meet again). It closes when that is
  whole. A gear's Phase in a Time comes back every cycle of that gear, `P` one, except in a
  sequencer, which reads it as a count. `chain::master_loop` is the least such `m` for every
  Ratio Gear under a master and every node they and the master drive through a Time — any
  of a node's Times (`nodes::is_time`), Shaky Cam's Time Y as well as its Time X — the
  least common multiple of what each asks for, so a ÷4 below asks for four cycles and a
  Perlin at Repeat 4 on the master four; a ratio that is not a
  fraction, or has a cable in it, leaves its chain open, and so does a node whose Time a gear
  reaches through anything but gears — a Math node between them, which the caption cannot
  follow.
- A node on its own clock closes over a length `L` when its rate times `L ÷ P` is whole: in
  Loop mode with nothing in its Time, `rate × L ÷ P`; running free with nothing in its Speed,
  `speed × pace × L ÷ P` (`chain::closes_alone`). A node standing still closes on anything.
- A clock in a Speed — a gear's Cycles or Phase, or a number a Math node made of one — is a
  rate that keeps changing, and the node never closes on it; the caption counts it.
- A picture that never repeats — a noise at Repeat Never, the tunnel at Depth Wrap None, a
  clip on Hold — never closes on a gear's Cycles, and the caption says it will not.
- An unconnected color input falls back to the hue wheel, `defaultUvMap`, which stands still
  and so closes on any loop.

**A loop that closes closes to the bit.** A node takes its Time round its own period before
it adds Offset — the prelude's `time_periodic`, `time_repeat` and `time_unbounded` — Time's
fraction plus Offset on a periodic node, `Time mod N` under Repeat, a
noise's circle and Static's roll alike, each the whole part
reduced and the fraction added — so a Time one whole period on draws exactly what a Time of
zero drew, whatever the Offset or the field in it. Added the other way round, `Time + Offset`
rounds the Offset differently a period on, and a loop comes back a grey level off in a few
pixels. And a count reaches a Time as its whole part and the `f32` of its fraction, so the
fraction a loop on is the fraction before it to the bit, before zero as after it: a render's
warm-up at negative time draws what the loop draws.

A stateful node closes only where it has settled or its state happens to come back, and
nothing reads that for it: `loop_gifs` compares the frame one loop on with the first, which
is the seam a person would see. Feedback through an Output's frame is the case to know: it
carries the rounding of every frame before it, and in eight bits a multiplicative fade rounds a
dim echo back up to itself, so the faint trails a loop on can differ from the first by a grey
level even where everything feeding them closes.

## Output kinds

| Kind | Meaning |
| --- | --- |
| `Shader` | a WGSL function, inlined into the consumer's module |
| `Texture` | a texture sampled through `u_texture_{slug}{id}_{key}`; also marks the port **delayed**. An Output's frame, or what a CPU node captured |
| `Uniform` | one `f32` per frame, published from `tick`, reaching a consumer as `u_float_{slug}{id}_{key}`; a count, published whole, reaches a Time as `u_count_{slug}{id}_{key}`, a `vec2f` of its whole part and its fraction |
| `Action` | an event, fired from `tick` and delivered inside the same one; never a uniform |

A `Shader` output may also be **dual**, by `OutputDef::eval`: its declared type is
`VaryingNumber` and its effective type on an instance is `UniformNumber` wherever every input
resolves to a uniform, so one node is both the field arithmetic and the uniform arithmetic.
See [Dual outputs](#dual-outputs).

A uniform output may also be **delayed**, by `OutputDef::delayed` rather than by its
kind: a tap's `color`, `mean`, `max`, `min`, `x` and `y`, a sample's `color` and its eight
numbers, and `autoexposure`'s `gain` and `luma` all set it, because the CPU half that publishes them read
the shader's measurement back a frame after it ran. `port_defs` builds the same delayed
`PortDef` from either source — a `Texture` kind or this flag — so `Graph::can_connect` and the
topological order treat the two alike; see
[architecture.md](architecture.md#delayed-ports-and-feedback).

A `Texture` output also says **how it is sampled**: `OutputDef::wrap` is `Mirror` or
`Repeat` and `OutputDef::filter` is `Linear` or `Nearest`. Mirrored and linear is
[the rule](rendering.md#texture-wrapping) and `OutputDef::EMPTY`'s answer, so almost no
definition mentions either. In wgpu the pair belongs to a sampler, not to the texture: the
compiler names the one the output declares in the module that reads it, out of four every
module with a texture binds, and the app copies both into the frame job per published texture
for the picture a viewer blits, because the renderer may not read the registry. A change picks
another sampler — never a reallocation.

A registry test holds the port type and the kind to each other: a `UniformNumber` port is a
`Uniform` kind, a node with a `UniformNumber` or a non-Output `Texture` output has a `cpu`
half, and a CPU node's inputs are never fields — it reads them in `tick`, which can hold a
number and not a `uv`. Another holds a delayed output to a `UniformNumber` type on a node with
both a `cpu` half and the `measure_wgsl` that fills it, and a `Texture` output to never carrying
the flag itself, since its kind delays it already. A third holds `wrap` and `filter` off their
defaults to a `Texture` output — a `Shader` output has no texture to parametrize — and names the
whole list of outputs that move either: `cellularautomata`'s `cells` (`Repeat`, `Nearest`) and
`slimemold`'s `trail` (`Repeat`), the two worlds that wrap in their own simulation.

## Nodes with both halves

A `tap` has WGSL — it returns its input — a `measure_wgsl`, and a `cpu` half that decodes the slot
the measurement wrote. The measurement claims the slot with `ctx.tap_slot(node, kind)`, reads
its inputs at the point through `ctx.input(node, key, "p")`, emits the atomic writes and
registers itself with `ctx.measure_grid(node, grid, jitter, body)` — or
`ctx.measure_once(node, body)` for a `sample`, whose point is its own two uniform numbers; its
`tick` reads `ctx.readback(node)`, and says *measuring* where it has one. The measurement
runs in its workspace's [pass](rendering.md#the-workspace-pass), never in an Output's module,
where the node is its pass-through alone. The registry test that forbids
field inputs on CPU nodes exempts a node that also has a `Shader` output, because that output is
what reads the field.

## CPU nodes

```rust
cpu: Some(CpuDef { create: || Box::new(Slew { value: None }), integrates: true, live: false }),
```

`create` builds the instance's state; the state implements `CpuNode`, whose `tick` runs once
a frame with a `TickContext` and whose `reset` puts it back where `create` made it.
`integrates` says the tick sums `dt` into that state and `live` that it reads a device — the
two facts an offline render needs, in [cpu.md](cpu.md#lifetime). `connected(id, key)` says whether an input is driven, for a
node whose behavior depends on it. The context resolves inputs the way the compiler does — a
connected uniform number reads its producer's published value, an unconnected one reads its
control — and takes what the node publishes: `publish(id, port, f32)` for a uniform,
`publish_frame(id, Arc<Frame>)` for a texture, and `publish_sim(id, port, Simulation)` for a
world the node steps on the GPU. `error()` reaches the status line; `status()`
and `progress()` describe work in progress and are drawn on the node itself — a transcoding
`video` node's file button is its progress bar — as well as beside the status line; `debug()`
goes to the Status box.

The state is where a device lives. `audioin` opens the microphone in `create` and closes it
on drop; `camera` opens its pipeline on the first tick, because the options that say which
device are on the node, and reopens it when they change.

A `tick` reads the newest thing a device thread published and never waits. The thread side
is `audio/` and `video/`.

The nodes with a `cpu` half and nothing else are `number`, `clock`, `oscillator`,
`animation`, `automation`, `slew`, `button`, `clockdivider`, `counter`, `adsr`,
`euclideanrhythm`, `stepsequencer` and the three in Gears, `mastergear`, `ratiogear` and
`time`; `tap`, `sample`, `autoexposure`, `autogain`, `cellularautomata`, `slimemold` and
`brickgame` have both halves, and
`audioin`, `camera`, `screencapture`, `syphon`, `ndi`, `video`, `imagegif`, `drawingcanvas`, `text` and
`maininput` are the sources. The twelve generators and transforms that move with time have
no `cpu` half: the synth writes their Time ([Timing](#timing)).
`ctx.downs(id, key, &mut gate)` is how a node reads a button that may also have a cable in
it: the number of downs this frame, hand included, so a toggle flips once per press wherever
the press came from.

## Dual outputs

```rust
outputs: &[OutputDef {
    key: "output", label: "Output", ty: VaryingNumber, kind: OutputKind::Shader,
    wgsl: |node, ctx, _func| { /* "    return ({a}) + ({b});" */ },
    eval: Some(|node, ctx| ctx.input(node, "a") + ctx.input(node, "b")),
    ..OutputDef::EMPTY
}],
```

An output with an `eval` is **dual**: it declares `VaryingNumber`, and its *effective* type
on an instance is whatever what feeds it makes it. There is nothing per-pixel about `a + b`
when `a` and `b` are uniform numbers, so a `math` node with knobs on it is a constant the CPU
can hold, and the same node with a noise cabled in is a field. One `add`, not two.

**The rule.** A dual output is `UniformNumber` when **every** input of its node resolves to a
uniform: a connected input whose source's effective type is `UniformNumber`, or an unconnected
one holding a `ControlValue::Float` or a `ControlValue::Color` — a control, which compiles
to a uniform anyway. A field on any input, or an input bound to a prelude global, makes
it `VaryingNumber`. The rule is written by rate rather than
by a pair of types: `Graph` computes one `Rate` for the node and hands every dual port
`PortType::with_rate` of it, so a dual color output would slot into the same rule now that
`UniformColor` exists. `Graph` writes
the answer into the instance's own `PortDef::ty` and recomputes it over the downstream closure
after every `connect`, `disconnect`, `add_node` and `remove_node`, and once at the end of a
file load or an import. So the compiler, `can_connect`, the port dot and the readout all read
the type they already read, and `PortDef::dual` — set by `port_defs` from `eval.is_some()`, on
the output and on every `VaryingNumber` input of the same node — is how `graph/` finds one
without matching on a slug.

**Demotion is unofferable, and a pinned node's inputs say so.** A `VaryingNumber` does not
feed a `UniformNumber` input, so a cable that flips a dual node to `VaryingNumber` while
something reads its number would break the cable that reads it. A dual node in that position
is **pinned**: it is in `UniformNumber` mode, and one of the outputs a flip would turn into a
field feeds — directly, or transitively through dual nodes that would flip with it — an input
that is `UniformNumber` of its own. **A pinned node's dual inputs have effective type
`UniformNumber`**, written into the instance's own `PortDef::ty` beside the output's, so a
field onto one is an ordinary `ConnectError::TypeMismatch`: the input *is* a uniform number,
the port draws the diamond that says so, and nothing has to be dragged at it to find out.
Unplug the `slew` first, or put the field somewhere else. Pinning never orphans a cable,
because a pinned node is in `UniformNumber` mode and every input of it that is connected at
all is fed by a uniform already. Promotion is never refused: a disconnect or a removal turns a
field back into a uniform number, which is what every `UniformNumber` input wanted. A **bulk**
change is the exception, because whether one of a file's cables demotes depends on which of
the others are in yet: a load and an import go in through `Graph::connect_loading`, which
walks past the pinning refusal and nothing else, and `settle_effective_types` then resolves
the types once and drops whatever cable is left reading a field as a uniform number, with a
`LoadWarning`.

The output modes settle over the downstream closure of a change; the pins run the other way,
upstream from the `UniformNumber` consumer back through the dual producers feeding it, so
`Graph::repin` computes them that way — one pass over the cables finds the dual nodes a
`UniformNumber` input reads directly, and a walk back through the dual producers feeding those
carries the answer to everything that would flip with them. One pass over the edges, beside
the order over immediate edges the output modes settle in; asking each dual node the
question separately costs a downstream walk per node and each of those rescans the edges.

**The tick evaluates one.** `Synth::tick` walks `tick_order`; a node with no `cpu` half whose
dual output is effectively `UniformNumber` has its `eval` run through the same `TickContext` a
CPU node gets — `ctx.input(id, key)` already resolves a connected uniform or the control — and
the result is published on that port. A dual output that has flipped to `VaryingNumber` is
withdrawn, so its row draws nothing rather than a stale number. Producers tick before
consumers, so a chain of duals resolves inside one frame.

**The family** is `add`, `subtract`, `multiply`, `divide`, `min`, `max`, `sine`, `cosine`,
`abs`, `ceil`, `floor`, `atan2`, `lerp`, `modulo`, `power`, `pythagorean`,
`smoothstep` and `threshold` — every `Math` node but `random`, which is a CPU node because
silvia's answers one number for the whole frame from a hash of its seed, and one number for
the frame is what a uniform number is — plus `reframerange` and `sliderule`, the two `Convert`
nodes built the same dual way. Each `eval` is the WGSL body in Rust with WGSL's semantics where they differ,
and the two implementations are held equal on the GPU by
`a_dual_nodes_two_implementations_agree` in `tests/gpu_nodes.rs`: the node in circle mode,
one input driven by a constant field and the rest knobs, measured through a tap's `number`
sidechain, against what the tick published on the same values. A dual node with no
equivalence case is itself a test failure.

**To add one**, write the `eval` beside the `wgsl` — `eval(f)` before the `=` in the `node!`
macro — and add a case to `DUAL_CASES`. The registry test
`a_dual_output_is_a_varying_number_a_tick_can_evaluate` holds the rest: a dual output is a
`VaryingNumber` of `OutputKind::Shader` on a node with no `cpu` half and no `measure_wgsl`, and
every input of that node is a `VaryingNumber` with a number control, since an input that
could never carry a uniform number would leave the `eval` unreachable.

## Naming

| Thing | Form |
| --- | --- |
| function | `{slug}{id}_{portKey}` |
| control uniform | `u_control_{slug}{id}_{key}` |
| texture uniform | `u_texture_{slug}{id}_{key}` |
| published uniform | `u_float_{slug}{id}_{key}` |

`id` is `NodeId`, the node's only identity: a `u32`, assigned from a counter that never
rewinds, so an id is never reused even after the node holding it is deleted. A stale
reference and a stale WGSL function name therefore can never address a live node.

This is silvia's scheme, kept because it is a good one — unique without a symbol table,
greppable, derived entirely from node identity. **supersilvia's WGSL is not required to match
silvia's GLSL**; port bodies freely.

## Worldspace

Square units, centerd at (0,0), height exactly 2.0, width 2·aspect. Nodes do not
aspect-correct unless they must — images and video. Preserve this or every ported node looks
wrong.

**A size in pixels is measured against a 720-high frame.** `nodes::REFERENCE_HEIGHT` is that
height, and every kernel and grid whose control is in pixels divides by it, so the offset a
control names is the same distance in world units whatever the Output's size: the node is one
field rather than one field per Output. Worldspace is 2.0 tall, so one world unit is half the
reference height in pixels — which is why a screentone's grid is
`uv * (REFERENCE_HEIGHT * 0.5)`, one cell per pixel of that frame. **Both axes divide by the
height**, so a step is square: it reaches as far across as it does down, which is the
worldspace convention applied to the step as well as to the coordinate.

**An angle is in turns.** 1 is one full turn on every angle knob and input — a shape's or a
pattern's Rotation, Rotate's, the gradients', Star Gate's, Emboss's, Motion Blur's, Halftone's
and Chromatic Aberration's Angle, Whirl & Pinch's Whirl Angle, Phyllotaxis's seed Angle,
Color Shift's Hue Shift, the Wave's Phase and Rotation and Sine's and Cosine's Phase — so a
gear's Phase, 0 to 1, cabled into one turns it once a cycle with no snap, and a body reads
`({angle}) * 2.0 * PI`. The knob's unit is `nodes::TURNS`, the glyph `↻`, which fits beside
three decimals in the 100-point number control as silvia's `π` did; its ranges are silvia's
halved, ±2 turns where she had ±4π. `every_angle_knob_is_in_turns` in `nodes/mod.rs` holds
the list and holds every knob in the registry off `π`. A knob in degrees — Slime Mold's
sensor and turn angles — is not an angle a gear drives and keeps silvia's degrees; the
camcorder's Rotate keeps its radians.

**No node body names `u_resolution`, `frag_coord` or `@builtin(position)`**, nor
`gl_FragCoord`, which a body ported from silvia would reach for. A node that reads any of them is a
different field in Outputs of different size, and a tap or a thumbnail behind it, drawn in
the workspace's pass, reads a field no Output of another size draws.
`no_node_function_reads_the_resolution` in `nodes/mod.rs` runs every output's generator of
every node in the registry, over every choice of every option, and holds the emitted function
text to it, with no exception. `u_resolution` belongs to the
uniform struct and `frag_coord` to `fs_main`, which is where `uv` is built and where a
measurement's cell comes from.
[decisions.md](decisions.md#no-node-reads-the-resolution) has the argument.

## What a generator publishes beside its picture

**Where a node computes a field on the way to its picture, that field is an output.** A
shape's coverage, a fractal's escape count, the polar sweep a spiral is drawn along: the
consumer should read the field the node already had rather than re-derive it from the
color. [decisions.md](decisions.md#a-node-publishes-the-field-it-already-computed)
has the argument.

The names are a closed vocabulary, and that is the point of them — a library this size
written without one is one idea under seventy names.

| Key | Type | Means |
| --- | --- | --- |
| `output` | `VaryingColor` | the picture, where it is the node's only one |
| `color` | `VaryingColor` | the picture, where a field is published beside it |
| `map` | `VaryingColor` | a second picture: the input read through the node's own geometry |
| `mask` | `VaryingNumber` | **coverage, 0 to 1, 1 inside.** Never anything else |
| `value` | `VaryingNumber` | the raw 0 to 1 quantity the picture was made from |
| `smooth` | `VaryingNumber` | an escape count, normalized, 0 inside the set |
| `angle` | `VaryingNumber` | a polar sweep, 0 to 1 |
| `x`, `y` | `VaryingNumber` | the point's own place, in worldspace |

`mask` is the load-bearing one: it *means* coverage or an inside/outside test, so a node
whose natural field is an escape fraction publishes that as `smooth` and keeps `mask` for the
set itself. Every other name is simply the quantity it is.

`value` is the one that is not a shape's field at all: a noise computes a number and *then*
chooses a color from it, and that number is neither coverage nor an escape count. The five
noises publish it, and so does `domainwarp`, whose displacement length is the same kind of
thing — the raw quantity, before the picture. It is the widest of the names, which is why the
narrow ones come first: a field that is coverage says `mask`, and only a field that is nothing
more specific says `value`.

`x` and `y` are the pair that is not a field at all: `worldcoordinates` publishes the
point's own place, which is what every other body's field is a function *of*. They are the
two names a coordinate has and they are spoken for by that one node.

**Not every generator has one.** `stripes`, `houndstooth`, `checkerboard`, `phyllotaxis`,
`sierpinski`, `prideflag` and both gradients publish no field, because in each of those the
field *is* the picture and a `VaryingNumber` beside it would be the same number twice.
`emboss` is the same case one category over: what it returns is `vec3(relief)`, so the relief
is the picture. The question per node is only *did this compute a field it is throwing away*.

**The vocabulary is not only a generator's.** Four categories draw a picture — a `Generate`
makes one, a `Color` maps one, a `Transform` resamples one, an `Effect` reads a
neighborhood or a cell of one — and all four name their fields out of the table above. Three
of them also follow the picture rule: one picture is `output`, and a picture with a field
beside it is `color`. A `Transform` is the exception, and deliberately: its picture is the
input passed through, so that port is `output` whether or not a mask sits beside it.
`Convert` and `Math` are outside this entirely, since they publish no picture at all and
their port *is* the quantity — a `channelsplitter`'s are `r`, `g`, `b` and `a`.

**A set is the one thing the vocabulary does not name.** `palette` draws eight pictures that
differ only in where along an arc they sit, so there is no picture that is *the* picture and
no name for any one of them that is not its index: its ports are `a` through `h`. The
vocabulary names a quantity, and the eighth of a fan is not one. So the picture rule does not
apply to a node whose every picture is a member of a set, which
`a_node_names_its_outputs_from_the_vocabulary` reads as: more than one picture, and every one
of them a letter in order.

`a_node_names_its_outputs_from_the_vocabulary` in `nodes/mod.rs` holds all four categories to
the table, and `tests/gpu_nodes.rs` holds a circle's and a Mandelbrot's `mask`, and a
Perlin's `value`, to what those words mean, on the GPU.

## The library

| | |
| --- | --- |
| `Source` | `audioin`, `camera`, `screencapture`, `syphon`, `ndi`, `maininput`, `video`, `imagegif`, `drawingcanvas`, `text`, `mouseinput`, `gamepad` |
| `Generate` | `color`, `checkerboard`, `circle`, `polygon`, `star`, `stripes`, `grid`, `polkadot`, `houndstooth`, `spiral`, `phyllotaxis`, `prideflag`, `lineargradient`, `radialgradient`, `mandelbrot`, `juliaset`, `lyapunov`, `sierpinski`, `perlin`, `simplex`, `worley`, `fractal`, `static`, `randomhurl`, `cellularautomata`, `slimemold`, `brickgame`, `worldcoordinates` |
| `Color` | `rgba`, `hsla`, `mix`, `layerblend`, `muxevent`, `muxnumber`, `cosinegradient`, `contrast`, `gamma`, `levels`, `invert`, `posterize`, `vignette`, `simplelight`, `saturate`, `vibrance`, `wavefold`, `palette`, `colorize`, `colormapping`, `colorshift`, `chromakey` |
| `Transform` | `zoom`, `rotate`, `fisheye`, `translate`, `mirror`, `stretchskew`, `perspective`, `polarcoords`, `rotozoom`, `shakycam`, `wave`, `whirlandpinch`, `kaleidoscope`, `tile`, `repeater`, `regionabsolute`, `regionsized`, `domainwarp`, `scatter`, `tunnel3d`, `wallpaper`, `geissflow` |
| `Effect` | `edgedetection`, `chromaticaberration`, `autoexposure`, `blur`, `sharpen`, `emboss`, `bloom`, `dilate`, `erode`, `heighttonormal`, `halftone`, `mosaic`, `dither`, `kuwahara`, `motionblur`, `radialblur`, `sincfilter`, `supersampling`, `pixelsort`, `stargate`, `glitch`, `camcordercrt` |
| `Convert` | `luminosity`, `lightness`, `value`, `average`, `hue`, `saturation`, `chroma`, `red`, `green`, `blue`, `alpha`, `channelsplitter`, `sliderule`, `reframerange` |
| `Tap` | `tap`, `sample` |
| `Math` | `add`, `subtract`, `multiply`, `divide`, `min`, `max`, `sine`, `cosine`, `abs`, `ceil`, `floor`, `atan2`, `lerp`, `modulo`, `power`, `pythagorean`, `smoothstep`, `threshold`, `random` |
| `Control` | `number`, `clock`, `oscillator`, `animation`, `automation`, `slew`, `autogain`, `button`, `clockdivider`, `counter`, `adsr`, `smoothcounter`, `triggeredrandom`, `triggeredcolor`, `randomfire`, `euclideanrhythm`, `stepsequencer`, `xypad` |
| `Gear` | `mastergear`, `ratiogear`, `time` |
| `Output` | `output` |

The left column is the node's own `Category`, not a heading invented here: the Nodes menu is
`Category::ALL` crossed with `REGISTRY` and nothing else.

The `Convert` nodes are one macro over one table: a `VaryingColor` in, a `VaryingNumber`
out, eleven ways of asking a picture for a number per pixel. `decompose::CONVERSIONS` holds
each one's slug, label, float expression over `color` and the range that expression bounds it
to, and the macro reads its own entry out of it by slug, so a slug that is not in the table is
a build failure. **The same table is a `tap`'s
`measure` choices**, which is what stops the eleven quantities and the picker saying different
things about any of them. `luminosity` weights the channels the way the eye does, `lightness`
and `value` disagree about what makes a saturated red bright, and `hue`, `saturation` and
`chroma` are guarded on a gray pixel so a flat region reads 0 rather than turning the frame
to NaN. Every one reads the color's own channels, its input through the prelude's
`unpremultiply`, so half-transparent red reads a red of one and an alpha of a half.
**`channelsplitter` is the twelfth**, and it reads the same table: one color in and
`r`, `g`, `b`, `a` out of one node, taking its four expressions from the table's `red`,
`green`, `blue` and `alpha` entries by slug, so it cannot say anything the four nodes beside
it do not. **`sliderule` and `reframerange` are the thirteenth and fourteenth and read no color at
all**: they are the category's other two conversions, one number's range onto another's, and
they sit here because that is the question they answer rather than because they decompose
anything.

The `Generate` family is five files grouped the way `decompose` groups: `shapes.rs` for the
things drawn around a center, `patterns.rs` for the ones that repeat across the frame,
`gradients.rs` for the two ramps, `fractals.rs` for the iterated ones, `noise.rs` for the
random ones. Every one of them is the `node!` macro except `checkerboard`, the hand-written
example above. `mandelbrot` and `juliaset` read Time and Offset on
their `map` output, where silvia read `u_time` directly, one cycle a drift of the orbit map,
a drift every two seconds at rest — [Timing](#timing) has the rule. `lyapunov`'s `sequence` is typed free
text, like silvia's own field, and silvia's Random Seq writes it: a
[`Region::Buttons`](ui.md#a-nodes-own-buttons) row of one under the rows, rolling two to
eleven letters of `A` and `B`, both present, exactly as silvia's `randomSequence` does. A press
is a `SetSettings`, so it rebuilds the shader, is one undo step and is saved; it is a button and
not silvia's action input, because a cable that rewrote a `Code` option would rebuild the
shader every time it fired.

The five noises — `perlin`, `simplex`, `worley`, `fractal`, `static` — each compute a number
and then choose a color from it, so each publishes that number as `value`. `worley` picks
between the distance to its nearest feature point, one flat tone per cell, and the Voronoi
edges; `fractal` sums octaves of `simplex`; `static` redraws its whole field once a roll,
the `floor` of `Time + Offset`. `perlin`, `simplex` and `fractal` walk their time axis by the
same sum, in lattice cells, and the three with `static` carry the Repeat option that walks a
circle instead ([Timing](#timing)); `worley` has no time input at all.
Their gradient functions are `NodeDef::wgsl_utils`, so a shader with no noise in it carries
none of them.

**`cellularautomata` is the first `Generate` whose picture is computed on the CPU**, and the
shape it takes is the one every simulation here takes, wherever its steps run. Its `tick` holds the grid and publishes
two channels of it as a `Texture` output — `cells`, red for the cell and green for silvia's
trail map — and its `output` is WGSL that samples that texture and mixes the `Alive Color` and
`Dead Color` inputs by the trail, which is silvia's own `mix(deadColor, aliveColor, state)`.
**The state is the port and the picture is the shader reading it**, so the colors stay
patchable and changing one rebuilds nothing. `algorithm` picks between silvia's four rulesets —
Life, HighLife, Day & Night and Brian's Brain, which is three-state — and `gridScale` sizes
the world in silvia's sixteens; both are `Runtime`, because a rule is a lookup table and a
size is a reallocation and neither is WGSL. **The grid is a torus**, so every neighborhood
count wraps in x and y and `cells` declares `Repeat` and `Nearest` — silvia's own parameters
for this texture, and [the declared exception](rendering.md#texture-wrapping) to the
mirror-wrap rule: the picture tiles across the world, and a mirrored tiling would fold a seam
through a world that has none. `Step` and `Randomize` are actions, `Auto-Run` is silvia's
checkbox as a 0/1 uniform number so a gate can start the world, and `Init Threshold`,
`Trail Decay` and **Rate** — silvia's Steps/Frame, the key still `stepsPerFrame` — are the
rest of silvia's s-numbers as inputs. Generations are owed to the transport's `dt` at Rate ×
30 a second, silvia's 30 Hz rather than the frame rate, the fraction of one carried from tick
to tick so the pace does not depend on the tick rate; a tick's batch is cut to what the grid
can afford — at 256×256 that is six generations rather than twenty, because a `tick` never
waits.
**The grid is on the node**, as silvia's is: `cells` is a
[`Region::Preview`](ui.md#preview-and-on-node-render) region under the standard **Preview**
heading, so a press of `Step` changes something a hand can see without the node first being
wired to an Output and that Output put on a deck. Drawing *into* the grid, which silvia also
does, is not here: it needs pointer input on a node's body, which nothing has. **`Init
Threshold` is read when `Randomize` fires**, not as the knob turns — a cable on it would
otherwise refill the grid at frame rate — and the node's help text says so, because a knob
that appears to do nothing under the hand otherwise reads as broken.

`slimemold` is the same shape over Jeff Jones' Physarum rules: agents that sniff a scent
field with three forward sensors, turn toward the strongest reading or away from it, and
deposit as they move, with the field diffusing and fading under them. Its `trail` texture is
the field — red the scent against its own running maximum, alpha where an agent is standing —
`color` is silvia's `bg → trail → agent` stack, and `value` is the density the picture was
made from, which is silvia's `trail × Heatmap`. **Its world is a torus too**: an agent that
walks off an edge comes back on the other one, its sensors sniff wrapped cells, its deposit
lands on the cell it came around to, and the diffusion reads across every edge, so `trail`
declares `Repeat` — keeping `Linear`, because a scent field is smooth where a grid of cells is
not. `gridScale` and `population` are the `Runtime` options, because both are allocations;
`mode` picks Attract or Repel; and the
`Trails` and `Agents` ticks are the one pair of **`Code`** checkboxes in the library, since
what they change is which layers the picture's WGSL mixes. **It is stepped on the GPU**: the
agents, the field and the picture are the renderer's, the rules are compute kernels in the
node's own file, and the tick decides how many steps the one clock's `dt` is worth —
**Rate**, silvia's Speed, × 30 a second, paid every tick rather than in a lump — and publishes
them as passes; see [cpu.md](cpu.md#a-world-on-the-gpu). `trail` is therefore a picture the kernels drew
rather than a frame uploaded, bound to its consumers the same way. **silvia's two ways to
find a look are both here.** `Randomize` is an action input, as silvia's is, and rolls Sense
Angle and Turn Angle anywhere from 1 to 180 degrees and Sense Dist a whole 1 to 39 cells onto
the knobs through `TickContext::write_control` — three uniforms, so a cable may fire it, and
one undo step for the three. The nine presets are silvia's preset bar, numbered from one, a
[`Region::Buttons`](ui.md#a-nodes-own-buttons) row under the rows: each writes silvia's
values for the three knobs and the Mode as one `SetSettings`, which rebuilds nothing, since
the knobs are uniforms and the Mode is `Runtime`. Both end in silvia's nudge — the scent
knocked back to nine tenths and one agent in eight thrown somewhere else — which the preset
bar asks for with a one-frame press under `slimemold::NUDGE`, the tick's to hear.

`brickgame` is the third, and it is the one that plays. Its `field` texture is the game
rasterized into a 300-square frame, `color` mixes `Foreground` over `Background` by the ink,
and `mask` is that ink as coverage — both clipped to the background outside a hairline
border, silvia's, which is what keeps a square field square in an Output of any shape. The
paddles are **`Action` inputs read as levels**: `ctx.pressed` for the hand and
`action::level_after` for whatever the cables last said, so a gamepad button, a sequencer lane
and a finger are the same thing to it. `score`, `bricksLeft` and `ballVelocity` are uniform
numbers the graph can drive anything with, and `brickBroken`, `ballLost`, `gameWon` and
`gameStarted` are one-frame gates: `Down` on the tick it happened, `Up` on the next, because
[an action is a gate, not a pulse](decisions.md#an-action-is-a-gate-not-a-pulse) and a moment
is the narrowest gate there is. silvia advances the game by a fixed step per animation frame;
here the step is `dt x 60`, bounded at four of silvia's frames, so it plays at silvia's pace
on any display and a late frame cannot tunnel the ball through a brick. `Launch Speed` and
`Paddle Width` are ports whose defaults are the numbers silvia hard-codes over its own dead
`values`. **The field is on the node**: `field` is a
[`Region::Preview`](ui.md#preview-and-on-node-render) region under the standard **Preview**
heading, because a game is the one node in the library where not being able to watch it is
not being able to use it. The knob that launches the ball is `Launch Speed` and the live
readout is `Ball Speed`: silvia calls both Ball Speed, and one thing you set and one thing
you read cannot share a name three rows apart. The key under `Launch Speed` is silvia's
`ballSpeed` still, so no saved patch has to be patched.

**`Effect` is the category that reads the picture somewhere other than under the fragment**,
and `Color` is the one whose answer is a function of the texel under it and nothing else.
That test, not silvia's menu, is what splits its `Effects` heading of thirty-three: `contrast`,
`gamma`, `levels`, `invert` and `posterize` are color maps however dramatic, and so is
`vignette`, whose factor comes from `uv` and whose color comes from the one texel it was
handed — it publishes that factor as `mask`. `blur`, `sharpen`, `emboss`, `bloom`, `dilate`
and `erode` are in `convolve.rs` because each reads a neighborhood, and `halftone`, `mosaic`
and `dither` are in `screentone.rs` because each reads a *cell*: what a fragment gets depends
on where it sits inside a tile. `dither` is the furthest from a color map of the three — it
takes a `VaryingNumber` field and two colors rather than a picture, and thresholds the field
against an ordered pattern over the pixel grid. `posterize` is the one on the line: its
Pattern and Scale are silvia's Color Dither, so at a scale wider than a pixel it reads its
input at the cell's center — but the default scale *is* one pixel of a 720-high frame, which
is the texel under the fragment, so it stays a `Color`.

**What is nonlinear in a color works on its own channels**
([decisions.md](decisions.md#colors-in-the-graph-are-premultiplied)). The color maps, the
recolorings, `palette`, `wavefold` and `halftone` take their input through the prelude's
`unpremultiply` and hand their answer back through `premultiply` at the input's alpha, so a
half-transparent picture comes out as the opaque one would, scaled by its alpha. A node that
builds one color out of several samples — `glitch` and `chromaticaberration` splitting the
channels, `edgedetection`, `kuwahara`, `emboss` and `sharpen` — reads each sample's own
channels and premultiplies the result at the center's alpha, so no channel outgrows its
coverage across an edge of alpha. A luminance a node compares or thresholds — `bloom`,
`dilate`, `erode`, `heighttonormal`, `pixelsort`'s keys — is the own color's. The sums are
left as they are: a blur, a bloom's gather and its add-back, a `vignette`'s factor and every
`mix` are exact on premultiplied colors.

`pixelsort` is the third shape an `Effect` takes, and it is its own file: it reads the
picture along a **run** rather than over a patch or inside a cell. Each row or column is cut
into chunks at hashed boundaries and the pixels of a chunk are sorted by brightness or hue,
so a fragment's color is whichever of its chunk's pixels sorts to its place. Chunk Size is
the loop's bound and so an option with its sample count on each label — at 64 that is
sixty-four samples of everything upstream per fragment and an insertion sort over them,
which makes it the most expensive node in the library. Its `mask` is where it sorted, 0
where a chunk's contrast fell under Threshold and the picture passed through. silvia's New
Seed button is a `seed` knob here, which a Triggered Random re-rolls and a moving number
animates — silvia's Animate switch was the second of those, so it is not a row.

`chromakey` is silvia's keyer and sits with the color maps by that same test: its answer is
a function of the two texels under the fragment, its own picture's and its Background's. The
distance is silvia's — the chroma difference in YCbCr plus a fifth of the luminance
difference — one `smoothstep` about Threshold is the key, and Spill Suppression pulls the
dominant channel of the key color off whatever survived, on a green or a blue key only,
because that is what the pull means. Both read the picture's own color, unpremultiplied, and
what is kept — the picture with its alpha times the key — is laid over the Background by
Porter–Duff over, so the Background shows through a transparent picture as it does through a
keyed color. It publishes that key as `mask`, 1 where the picture stays, so Mix and Layer
Blend can do the stacking.

`stargate` is the one `Effect` that reads a *second* picture somewhere other than under the
fragment. A thin slit shows the live picture and everywhere else is last frame, shifted along
the slit's own normal in opposite directions on its two sides. silvia samples the frame
history of whichever Output it was drawn into; here **last frame is a port**, so an Output's
Frame Out is cabled into it and the node drags any picture rather than only the one it sits
in. Its `mask` is the slit's coverage, **Position** places the slit, and the drag is
**Drift**, its key silvia's `speed`, in pixels of a 720-high frame per
frame — silvia steps one texel of the real output, which is a different speed in every
Output. Nothing on the node accumulates: the integral of the drag is the loop itself.

**A sampling radius is a `VaryingNumber`; a kernel size is an option.** A radius scales an
offset and changes no code, so a cable drives it. A loop's half-width *is* generated code, so
it is an option, and every loop in `convolve.rs` has a constant bound as a result. These are
single-pass and therefore quadratic — there is no intermediate target to separate a blur into
two passes — so a half-width of `r` is `(2r+1)²` reads of everything upstream, and the
tooltips carry the counts.

`layerblend` sits beside `mix` rather than replacing it. `mix` crossfades two colors by one
amount; `layerblend` composites a foreground, scaled by an opacity, over a background through
one of nine layer modes, by the W3C's compositing formula on premultiplied colors:
`fg·(1 − bg.a) + bg·(1 − fg.a) + fg.a·bg.a·B`, where `B` is the mode's blend function of the
two layers' own colors. Normal is Porter–Duff over, every mode blends only where both layers
cover, and opaque layers at full opacity draw silvia's modes. Its Normal mode is not the
crossfade, and a feedback patch wants the crossfade. `mix` also carries three of silvia's eight crossfades as its `method` option —
the plain mix and the two luminance fades, the three whose answer is a function of the color
under the fragment — and its fader warp as `curve`, both `OptionKind::Uniform` so a change is
a uniform write and not a rebuild; `amount` stays the port, 0…1, and `curve: fader` is what
reads it as silvia's −1…1 fade would. The five that sweep across a screen — the wipes, the
checkers — are [the Main Mixer's](rendering.md#the-mixer), which has all eight.
`cosinegradient` is Iñigo Quílez's `bias + amp · cos(2π(freq · t + phase))`: a
number in, a color out, so a mask or a noise's value becomes a ramp. It is drawn as silvia
draws it — a gradient strip and the three channel curves over a grid of the twelve
coefficients three across under R, G and B — and the twelve are the node's own values rather
than ports, so what a cable can reach is what goes through the palette and what drifts it.
Time and Offset shift the three cosines' own phases together, in cycles of the palette: still
on a new node, as silvia's Cycle of zero is, Offset placing the palette and a Speed turned up
or a gear cabled into Time drifting it; the strip on the node drifts with it. Phase on this node is only the grid's
per-channel coefficients. **Freq is a whole number, 1 to 4**, stepping by one: a shift scrolls
each channel's wave along the number's 0 to 1, and only whole waves in that span meet themselves
at its ends, so a Freq that is not whole drew a seam at the strip's edge, and in any picture fed a
number that wraps, while the shape between the ends changed through the cycle. A flat channel is
an Amp of 0. The shader and `cosinegradient::eval` round a stored value half up and clamp it to
1 to 4, so a value loaded or set past the knob draws what the knob would. `reframerange` maps
`in[min, max]` onto `out[min, max]` with all four bounds as ports
and an optional clamp, and carries silvia's five named bases — 0 to 1, 0 to 360, -1 to 1, 0
to 255, 0 to 2π — as two rows of buttons in a region of its own, `widgets::ranges`: a press
writes the two bounds through `SetControls` and lets go, so the conversions a patch asks for
over and over are one press rather than two typed numbers and the bounds stay knobs something
can drive. The Swap under them exchanges Output Min and Output Max, which is how this node
runs a map backwards. silvia's `sliderule` is the same map with the bounds *picked* rather
than written, one basis each side and a per-side invert, and it is filed beside it: what it
shows on the canvas is the range an instance is in — *From 0 to 1, To 0 to 360* — where
`reframerange` shows four numbers. Its picks are `OptionKind::Code`, so the shader carries
the one arithmetic they name rather than silvia's branch over two `int` uniforms. Both are
`Convert` and both are dual, so either maps a uniform number onto a uniform number on the
CPU.

**`muxevent` and `muxnumber` are the same four inputs under two hands.** `muxevent` walks its
channel on `next` and `prev`, jumps to one of the four on `rand` and returns to the first on
`reset` — silvia's three buttons plus ours — and publishes the channel as a `UniformNumber`,
counting from one so the number reads as the row labels Input 1 through Input 4 do. That
output declares `OutputDef::integral`, which is what draws it with no decimal places: a count
is not a measurement, and two zeroes on a number that only lands on whole ones say otherwise.
`muxnumber` takes the same choice from a number instead, which is where the crossfade lives —
`mode: switch` wraps the whole number round the four, `mode: crossfade` clamps it and blends
the two either side of the fraction, with the fourth fading into itself rather than back round
to the first. The asymmetry is silvia's and deliberate: a counter wants to come round, a fader
does not want to slam home. `mode` is `OptionKind::Uniform`, because a fade turned into a cut
mid-set must not rebuild a shader.

**`Transform` is the category a distortion is in.** A distortion moves the sampling
coordinate, which is what the category is defined as; how wild the result looks is not a
different kind of operation. So `wave`, `whirlandpinch`, `kaleidoscope`, `tile`, `repeater`,
`domainwarp`, `scatter` and `tunnel3d` sit in `distort.rs` beside `zoom`, `rotate`,
`fisheye` and the seven in `transform.rs` below them — `translate`, `mirror`, `stretchskew`,
`perspective`, `polarcoords`, `rotozoom` and `shakycam`, silvia's own Transform family — and
all eighteen read their input through the macro's `at`. `translate` is the plain slide,
`stretchskew` a scale per axis and a shear, `perspective` a divide by a leaning plane,
`mirror` a fold or a flip about a line through a center it can place, and `polarcoords` the
remap between straight and round coordinates in four directions, the two into round ones
each also Smooth, which folds the input's width around the circle as a curved mirror where the
plain mode cuts its two edges together; `rotozoom` and `shakycam`
read Time and Offset in their 20π cycle — a quarter of it on Shaky Cam's Y — where silvia
read `u_time`, and Rotozoom's Turns
says how many turns a cycle makes. `tile` wraps the whole
plane onto one rectangle; `repeater` lays one bounded rectangle out in a finite grid over a
background, and publishes `mask` — *Grid Mask* on the row, silvia's name, since a patch where
three nodes publish a Mask needs to know which one a cable came from — for where a copy
landed. `scatter` lays each of its copies over what is already under it, Porter–Duff over,
starting from its background. `regionabsolute` and `regionsized` are one crop reached by two sets of handles, four
edges or a center and a size, in `region.rs`: outside the rectangle is a background color, the
rectangle tiled, the rectangle mirror-tiled or the edge smeared out, and the coverage is the
`mask` beside the picture. The background color is behind the input too, composited
premultiplied, so it shows through wherever the input is transparent. `domainwarp` and `tunnel3d` read Time and Offset where silvia
read `u_time` and drove a CPU phase accumulator respectively: the warp in lattice cells with
the noises' Repeat, the tunnel in flights of 64 units of camera depth on its retuned path,
which comes back every flight while the depth wraps, its Helix every quarter.

`slew` is the reference CPU node: three `UniformNumber` inputs — `input`, `rise` and `fall` —
one `UniformNumber` out, and a `tick` that integrates `dt`. `Shape` picks which of the two
smoothings it is: `Rate Limit` leaves at a constant speed and arrives with a corner, and
`Ease` is the exponential approach silvia buried inside its Smooth Counter, `1 − exp(−rate ·
dt)` of the remaining gap each frame, so it slows as it lands. The same two knobs read as a
speed in one and as an approach rate in the other, which is what keeps a fall slower than a
rise in both. `audioin` publishes level, peak
and three bands from the default input device, smoothed only as much as its control asks —
zero by default. `camera` publishes a V4L2 device, or the GStreamer test pattern, as a
texture; `auto` probes for the first node that can capture, since `/dev/video0` is an
output-only loopback on any desktop running OBS. Its icon is silvia's camcorder, `📹`, not a
still camera. A 4:3 camera in an Output of another shape mirrors out to the sides rather than
reading a clamped edge — [the texture's own wrap mode](rendering.md#texture-wrapping), not an
option on the node.

`screencapture` is `camera` with the desktop's picker in front of it: each node holds a
portal session of its own, asked for when the node opens and given up with the node, so a mix
can carry two windows at once. The panel's screen source stays what it is — one capture for
the rig, read by as many `maininput` nodes as want it — and this is the node for the second
window. There is no Start button: opening the node puts the picker up, *Choose Screen* puts it
up again and *Stop* ends the session, both of them `Action` inputs so a sequencer can press
either, and the status line says which of the three it is in. Nothing is resumed from a saved
token; see [media.md](media.md#screen-capture-as-a-node).

`syphon` is a picture another app on the Mac publishes over Syphon, one server a node, so a
patch can take several — Resolume's composition in one corner, a VDMX layer in another. Its
`Server` menu lists what the Mac's Syphon directory has now, by "App – Server", which is what
the node saves; one that is not running is waited for and taken up when it starts, and the
status line says which it is doing. **Flip** reads the surface top row first rather than
Syphon's bottom row first, and **Transparent** keeps its alpha rather than reading it opaque.
Each frame is copied into a texture of the node's own, since the server draws into its surface
again. Linux has no Syphon: the library does not offer the node there (`NodeDef::offered`),
and one in a Mac's project loads and says so. See [media.md](media.md#syphon).

`ndi` is a picture another machine on the network sends over NDI®, one source a node, on Linux
as on a Mac. Its `Source` menu lists what the network has now, by the name NDI gives it,
`MACHINE (Stream)`, which is what the node saves; one that is not there is waited for, one that
goes keeps its last frame up, and either is taken up when it appears, the status line saying
which. **Transparent** keeps a source's alpha rather than reading it opaque. Where the NDI®
runtime is missing, the status line says so and where it looked. See [media.md](media.md#ndi);
NDI® is a registered trademark of Vizrt NDI AB, [ndi.video](https://ndi.video).

`text` is the one node that puts a letter on the screen. The words are the multi-line box
`note` draws, four lines tall and [a value](#a-nodes-own-values) rather than an option, and
the letters are rasterized by GStreamer's pango plugin — no font crate is taken on — into a
picture of the size **Texture Size** names. `ink` is that picture as a port, white on black,
and `output` is WGSL mixing **Text Color** into **Background Color** by it, which is silvia's
own `mix(bg, textColor, mask)` and the shape `cellularautomata` takes: the state is the port
and the picture is the shader reading it. So a color on a cable and a keystroke both cost no
rebuild — a new string is a new pipeline, never a new program. **Font** is silvia's twenty
faces, **Size** is a plain number field holding a whole number from silvia's 8 to 512, and **Weight**, **Align** and
**Baseline** are silvia's three selects. The fonts are the machine's in both, so a project
opened elsewhere may set the words in another face; see
[media.md](media.md#words-as-a-picture).

`maininput` is the odd one: **it owns nothing.** Every other source opens its own device; this
one reads whatever the [Main Input panel](media.md#the-main-input) is holding, so eight of them
in a project open one camera and analyze one signal. That is the whole reason it exists, and it
is silvia's shape. Its ports are the bundle `video` and `audioin` publish, down to the key, so
a patch built against a clip plays against the panel's camera unchanged.

**It draws silvia's three level bars**, with the panel's threshold square on each, and it
does not draw the rig's picture: the panel is already showing that, and a level wants setting
on the band it measures rather than at the far left of the window. The bars are read-only —
the threshold is the rig's one number — and the two ticks over them are `audioin`'s
**Uniforms** and **Events**, so a Main Input used for its picture alone is a short node with
one row rather than ten.

It therefore has **no tuning and no thresholds of its own**, and no `monitor`. Those are the
panel's, and not as a matter of taste: the bands are measured and the thresholds crossed on
the audio thread inside the one capture every reader shares, so a per-node copy would mean
whichever node ticked last decided what all of them saw. A node that wants its own tuning is
an `audioin`, which owns its capture and can have one. It reads the frame's
`TickContext::main_input` — one `MainInputFeed` the app assembles once and hands to every one
of them, which is what makes them agree.

`tap` and `sample` go the other way, from a picture to uniform numbers, and do it as a side
effect inside a shader evaluating the picture's expression — their workspace's pass — see
[architecture.md](architecture.md#going-down-side-effects-in-the-expression).
Both pass their input through untouched. **What each measures is its own input over the unit
square**, not the picture a consumer asked for, so a transform between the node and its
Output moves the picture and leaves the reading alone. `tap` publishes the `mean`, `max` and
`min` of one quantity and the centroid `x`, `y` weighted by it, over an `N`x`N` grid of
points: `grid` is the `N` — 32, 64, 128 or 256, default 128 — and `jitter`, off by default,
moves each point inside its own cell by a hash of the cell and the clock, so a pattern finer
than the grid stops biasing the mean. Turning jitter on rebuilds nothing; changing the grid
rebuilds the shader. **Which quantity is a picker**: `measure` offers the eleven `Convert`
reductions out of the one table they are defined in, and defaults to `luminosity`. **A
`VaryingNumber` plugged into `number` overrides it** and is measured instead — a tap that
measures a field is still a tap, and the picture still passes through — so the select goes
inert and reads `Number` while that cable is there. Whatever is measured is signed, and read
to 1/65536 over a range of -32768 to 32767. The centroid weights by the positive part of the
quantity, so `x` and `y` say where it is positive and a field that is negative everywhere
leaves them at the origin. The quantity is read from the input's own channels, as a `Convert`
node reads them. A tap publishes its input's **mean color** beside all of that, which is a different question
from the picked quantity — *what color is this picture on average*, rather than *how much of
this quantity is in it*. It is weighted by coverage and opaque, so a mostly-transparent
picture still shows the hue it is being asked about instead of reporting itself invisible, and
a transparent texel lends it nothing.
**Both label their pass-through row *Pass-Through* rather than *Output***, because on these
two nodes it is the input leaving untouched and the row under it is a color as well;
`autoexposure` keeps *Output*, since what leaves it is the picture times its gain.
`sample` takes `x` and `y` as uniform number inputs and publishes the
`color` of its input at exactly that point, as its own channels and its alpha, with `r`, `g`, `b`, `a`, `luma`, `hue`,
`saturation` and `lightness` beside it — one call, no tolerance, so a transform downstream
cannot put the point off the screen. The reading is published twice over because the two are
wanted for different things: the `color` lands on any color input and shows itself on that
input's swatch, while a number is what arithmetic takes.

**`hue`, `saturation` and `lightness` are `decompose::hsl`**, the same three rows of the
conversion table the `hue`, `saturation` and `lightness` nodes compile, written out in Rust.
They are on the node rather than left to those nodes because those are shaders: each turns a
picture into a *field*, and collapsing one back to a uniform number takes a `tap` over sixteen
thousand grid points — the wrong instrument for a question about one pixel the CPU is already
holding. Being a transcription, it is held to the table by a GPU test that renders the three
nodes over a color and compares their pixels with what the Rust returns. `luma` and
`lightness` are both there and are not the same number: luma is Rec.709 perceived brightness,
lightness the midpoint of the brightest and darkest channel. Both are one frame late, and **a
tap on a workspace is measured whether or not it reaches an Output**: its workspace's pass runs
the measurement, so the rows read and the status says *measuring*. Where nothing measures a
node — it is suspended, or no reading has come back yet — both **withdraw everything they
publish**, the sample's color included, so the rows draw nothing and the status says
*not measuring* rather than reading `0.00`. See
[cpu.md](cpu.md#taps-the-other-direction). A tap on a video, its `mean` into a slew, the
slew into a hue: the color changes on a flash.

`autogain` and `autoexposure` close loops over those numbers. `autogain` tracks a floor and
a ceiling of a uniform number — up at the attack rate, down at the release — and publishes
where the input sits between them, so the microphone reads the same in a loud room and a quiet
one; `span` is the smallest gap allowed, so silence is not stretched to full scale.
`autoexposure` measures its input the way `tap` does — over the unit square, at the grid it
offers no choice about — computes the gain that brings the mean
luminance to `target`, slews toward it in log space at the pace **Response** sets, clamps
it, and multiplies — open loop on
the input, so it cannot oscillate, and inside a feedback loop it holds the loop at a level
instead of letting it run away or die. Its WGSL reads the gain its own tick published,
through `ctx.own_uniform`, which is how a node closes a loop over itself without a cable.

`video` plays a file. On first use the file is transcoded into all-intra H.264 in the cache
(see [decisions.md](decisions.md#video-files-are-transcoded-on-import-into-all-intra-h264)),
so every frame costs the same to reach. **The position is the primitive**: each tick the node
decides which frame it wants, and **a clip is an oscillator whose shape is a frame lookup**.
One cycle is one play of the clip. **Time** counts plays: unplugged, it is ambient time at the
clip's native speed, a play every clip length, read through `TickContext::cycle_at`, and
running free its Speed is a multiple of that native speed, so Speed 2 plays it twice as fast and
−1 backwards; in Loop mode a gear cabled in replaces Time, so a Ratio Gear at ×2 plays it twice
as fast and one at −×1 plays it backwards. **Offset**, 1 a play and −1 to 1 on its knob, is **added**; `loop` wraps the sum, or at Hold
clamps it to one play, and the frame is `round(position × frames)`. The node keeps no position
of its own, so the same sum is the same frame however it was reached. A slow wave on Offset
scratches around the playing clip, and a Ratio Gear at ×0 into Time leaves a cable on Offset
the whole position, a scrub. The scrubber on the picture writes Offset, so the sum lands where
the hand let go. **In a render the clip waits for its frame**: live, the picture is whatever the decoder has
delivered, a tick late after a jump, and a render holds its frame until each clip has the one
its position names ([cpu.md](cpu.md#the-transport)), so a render of a clip is the same film
twice. Its `file` is an [`Asset` option](#option-kinds) — a
file button on the node, a dropped clip makes the node, and swapping a clip rebuilds no
shader — and `supersilvia friday/` opens
a project from the command line. A clip still being transcoded says
`Preparing clip… 40%  0:07` across its own preview band, the words, the encoder's position and
a clock, because a percentage alone cannot say how long a wait has left in it. A clip that
cannot play says why in the same place: a failed transcode's error, or a picture refused by
name — `loop.gif is a picture: an Image/GIF node shows it`, since a picture is `imagegif`'s
and the transcode has no clip to make of one. Sampled outside its own frame — an aspect mismatch with the
Output it feeds — it mirrors out to the sides the same way `camera` does, the same texture
wrap mode.

`imagegif` shows a still picture or an animated GIF, **on the node** — a
[`Region::Preview`](ui.md#preview-and-on-node-render) region under the standard **Preview**
heading, the clip node's own, so a row of them reads as a contact sheet. Its `file` is an
[`Asset` option](#option-kinds) like `video`'s — a file button on the node, the project's own
pictures offered before the dialog — and it takes png, jpg, jpeg, gif and webp; a **dropped**
one of those makes one of these, animated GIFs included, where a clip makes a `video`.
The decode
happens on a worker and the tick polls it, so a large file never costs a frame; `status` says
how many frames have arrived, across the picture band, which is a count and not a fraction
because a GIF's header does not say how long it is. **Time counts plays of the animation**, by
`video`'s rule: unplugged, ambient time at the GIF's own pace, a play every length of the
delays it was authored with; a gear cabled in replaces it, a Ratio Gear at ×2 twice as fast and
one at −×1 backwards. Offset, 1 across the animation laid over those delays and −1 to 1 on its knob, is added,
the sum wrapping as a GIF does, and the frame shown is the one whose delay the sum falls
inside, so the node keeps no playhead of its own. `frame` and `frames` publish where the sum
is and how many there are. A still is one frame and Time does nothing to it. Sampled outside its own picture it mirrors out to
the sides the way `camera` and `video` do — [the texture's own wrap
mode](rendering.md#texture-wrapping), which is what silvia asks for on this node too. See
[media.md](media.md#images-and-gifs).

`drawingcanvas` is a picture a hand paints on the node, silvia's: a surface that claims the
pointer, six tools — pen, eraser, line, rectangle, circle and fill — with silvia's keys while
it has focus, a brush size and color and a background as the node's own hidden controls, and
eight symmetry modes, None to 8-Fold, that repeat a stroke as a mirror or a mandala.
**The painting is one of its values, `Value::Painting`, and it is saved** — written by the
project's save into `assets/` as `painting-<print>.png`, named by its content, read back on
open, carried by the autosave, a copy and an export, and a stroke is one undo step
([decisions.md](decisions.md#a-painting-is-saved-with-the-project)). The painting is the node's
`output`, a `Texture` published as the value's straight pixels premultiplied — a copy made
once for each painting and size — sampled by the aspect it has
with silvia's Wrap outside it — Mirror by default, Repeat and Clamp by folding the coordinate;
`mask` is its luminance times its alpha, so an erased pixel is no mask; and `strokeDone` fires
for one frame when a stroke lifts, which is what makes the canvas a drum pad as well. **Canvas
Size** is silvia's five sizes, and moving it stretches the painting rather than clearing it;
**Clear** is an action input that fills the canvas with the background, a step of the history
each time and none for a canvas already blank. The surface and its brush are
two regions — ui.md, *A region may be painted on*; the pixel arithmetic — round caps, source-over, the eraser's
`destination-out`, `strokeRect`'s square corners, the fill's tolerance of twenty — is
`nodes::drawingcanvas`, pure and tested without a window.

The gears, `time`, `oscillator` and `animation` are all CPU halves, and none of them
multiplies a speed by a time.

**The gears** are the one place a rate is set, changed or divided, and they hold the only
state in the time model besides a free-running node's own playhead, which the synth keeps:
everything they drive is a function of what they publish ([Timing](#timing)). They are the `Gear` category, **Gears** in the menu under ⚙,
between Control and Output, in `nodes::gear`, with `time` beside them; see
[cpu.md](cpu.md#gears) for their tick.

**`mastergear`, the Master Gear**, is the show's own clock at a length: **Length** in seconds,
two by default, a bar at 120 BPM. It integrates the playhead's advance over one cycle's
seconds in `f64`, so a Length turned bends from where it is and never jumps, and it is born at
`playhead ÷ length`: two Master Gears of one length agree, and at the playhead's zero every one
of them is at the start of its cycle. **Gate** is how long its Trigger stays down, in cycles.
A beat is a Master Gear a beat long, and a bar a Ratio Gear at ÷4 below it.

**`ratiogear`, the Ratio Gear**, is a clock in and a clock out at a ratio. **Clock In** (key
`clock`) is a diamond with no knob. The gear publishes `ratio × ΔClock In`, a count read whole
in `f64`, and anything else unwrapped where the output it comes from declares its wrap
(`OutputDef::wraps_at`: one for a Phase, 2520 for anything else), so a 0..1 Phase cabled in
runs forward across its wrap; with nothing cabled it counts the playhead's seconds. **Ratio**, −64 to 64, is a ladder
(`nodes::gear::ladder`, the number control's `NumberSpec::ladder` mode): a drag walks ÷16 ÷8
÷6 ÷4 ÷3 ÷2 ×1 ×2 ×3 ×4 ×6 ×8 ×12 ×16, and on through ×0 into reverse; typed, it takes ×5,
÷7, 3/2, 0.3 or a leading minus; and a MIDI knob sweeps the ladder. **A ratio change lands on
the input's next whole cycle.** Until then the old ratio runs and the display shows the new
one pending, so the output's downbeat stays on the input's, and a chain of whole ratios closes
whatever phase the change landed at; a glide would leave the downbeat wherever the glide
ended. The gear is born at the ratio times its input's reading, the count as it is published,
so one born again after a seek, a reopened tab or a render's warm-up is where playing would
have put it and its downbeat is the input's.

**Both publish the same four.** **Cycles** are a count, a time rather than a fraction, since
a wrap at one would put a seam in front of every reader not periodic at one cycle. A Time
reads it whole: a CPU node in `f64`, never wrapped, and a shader as its whole part, wrapped at
40320 and centered on zero, and the `f32` of its fraction, so a gear reset and stepped back
reads −0.01, a render's warm-up counts before zero as precisely as after it, and the
millionth cycle is as precise as the first. 40320 is the least common multiple of 2520 and 128,
which every Repeat and Static's 128 divide. Anything else — a Math node, an
input that is not a Time, the row — reads one `f32`, wrapped at 2520 and centered on zero,
−1260 up to 1260, the least common multiple of one to ten, which a reader at a whole ratio or
a ratio in tenths passes with no seam: through a Math node, a gear at a cycle a second rolls
over, from 1260 to −1260, 21 minutes in and every 42 minutes after. **Phase** (key
`wrapped`) is the fraction alone, 0 to 1; **Ping-pong** a triangle that reaches 1 at one cycle
and 0 at two; and **Trigger** an event on each whole cycle, placed where inside the frame it
fell, down for the Master Gear's Gate or for half a cycle of the Ratio Gear. **Hold** is a
toggle that freezes the gear where it stands and closes the gate it left open; **Reset** puts
it at the start of a cycle and is a beat. The tick walks the frame's holds and resets moment
by moment, so two inside one frame land in the order they happened. A seek — the time
readout's reset among them — and a render's start are a jump: every gear is born again where
the playhead puts it, and fires nothing on the way but the beat it lands on, where it lands on
a whole cycle. A Ratio Gear whose Clock In a Reset above puts back,
by any distance, or that is sent back more than a cycle in a frame, is born again where the
clock now is and fires at most one downbeat: its own, where the clock's motion this frame
carried it past one.

**A gear draws itself** in `Region::Gear` (`widgets::gear`), 92 units tall under the rows, by
its **Display** option, a `Runtime` one that changes nothing it publishes. **Rosette**, the
default, is a still spirograph: for a ratio `p/q`, a turn round the picture each input cycle,
so `q` loops, with `p` petals waving in and out across them — a ÷4 is one petal wound over
four loops — a tick at the top where each loop begins, and a dot riding the curve at the
output's phase, a Master Gear's a ring of a clock face's twelve
ticks. **Gears** is the input's gear of `k·p` teeth meshing with the output's of `k·q`, a
Master Gear's one gear of twelve teeth. Both turn by the gear's own phases, never an animation
clock, so a paused show is still. Beside the picture are the ratio, what it closes in and a
pending change. Under a Master Gear's is a caption, `nodes::chain::caption` — "loops in 4
cycles · 8.000 s (÷4 on ratiogear12)", or "2 nodes will not close" — shown up to its " ("
with the whole on hover ([When a loop closes](#when-a-loop-closes)).

`time` is **ambient time as a number**: `Seconds`, the playhead published as a count, so it
jumps with a seek, holds with a pause, is negative before the playhead's zero and **counts
on**: a Time reads the `f64` playhead whole, and through a Math node it is the playhead as one
`f32`, unwrapped, which resolves a millisecond for the first two hours of a show and a
sixtieth of a second for the first 36. It has no inputs, and no cycles or phase of its own:
a rate against the show is a Ratio Gear, and a count of cycles a Master Gear's.

`oscillator` is silvia's, and it publishes a `UniformNumber`: seven waveforms in silvia's own
phases at `Time + Offset`, in waves, times Amplitude, lifted by **Level**, silvia's Offset.
Unplugged, Time is a wave a second, silvia's default 1 Hz; a gear cabled in is where a
frequency is set, turned, stopped or restarted, a Ratio Gear's Ratio, Hold and Reset standing
for silvia's Frequency, Start/Stop and Reset. Offset is added, in waves, so a slow wave there
modulates the phase of this one. The node keeps nothing, so its value is the same wherever the
show was sought or played to; **Noise** draws a fresh value each wave, keyed by
`floor(Time + Offset)` and the node's id, so it holds for a wave and is the same twice. A
one-shot is `animation`'s. Its body carries a trace of what it published, the ring
`CpuNode::trace` keeps, so the waveform on the node is the wave the graph is getting. The
field version, whose frequency and phase could themselves be pictures, is
[proposals/field-oscillator.md](../proposals/field-oscillator.md).

`animation` is silvia's travel: `startValue` to `endValue` over `duration`, with an approach
curve out and a return curve home — `linear`, `smooth`, `ease_in`, `ease_out`, plus `jump`
(start the next pass from the near end) and `stay` (never leave the far one) on the return.
It starts stopped, holding its start value, because a travel is something a performer fires;
`startStop` toggles, and in `stay` a second press replays rather than pausing at a value that
is not going anywhere. It carries a trace of what it published, as the oscillator does.

The four event nodes are the event half of the library, and each proves one thing about it.
`button` is one action out and a `Control::Press` on it — the hello-world. A beat at a tempo
is a Master Gear's Trigger.
`clockdivider` takes actions and emits actions, which is the composability
test — Clock In and Divided Out, silvia's names, with a status-line region counting where it
is in the group and a count that starts over when Division moves. `counter` and `adsr` are the door out: actions in, a `UniformNumber` out, and a uniform
number feeds a varying number input for free, so an envelope reaches a shader with no
plumbing of its own. `counter` is silvia's four buttons — Increment, Decrement, Reset and
Set to Max — over silvia's own default range, 0 to 1 in hundredths, which is a fade rather
than an index; `smoothcounter` is that plus an ease at its **Rate**, publishing `value`, `normalized` and
`target`. Every audio source fires its band and volume thresholds as actions too,
dated by the sample they crossed on. `adsr`'s body carries a trace of the envelope itself —
the last three seconds of `value`, a ring `CpuNode::trace` keeps and `tick` pushes into every
frame — because a stage and a number do not say what a shape looks like, and under it the two
cells `CpuNode::caption` publishes: whether the gate is on, and which stage the envelope is
in, which is silvia's own pair of readouts.

`adsr`'s three moving stages each follow a **curve over the stage's own time** rather than a
constant rate: silvia's `Linear`, `Exponential` and `Logarithmic`, with silvia's defaults —
linear on the attack, exponential on the decay and the release, which is what gives a fall its
taper. The four times are silvia's too (0.01, 0.1, 0.7, 0.2, nudged in thousandths), and so is
the gate: a fresh gate snaps to zero and attacks from there rather than carrying on from what
is left of a release. There is no Max Level — the envelope is 0 to 1 and a gain is a multiply
downstream, as every other normalized output here works. What stays ours is the sub-frame
timing and the gate being the sum of every source rather than whichever spoke last.

**The two step sequencers are one clock under two patterns.** `euclideanrhythm` and
`stepsequencer` both run `nodes::sequencer::Transport` — their time rows in bars, silvia's Step
as an action row, Gate as a knob with a port, four lanes out — and each hands it only what a
lane plays at a step: Bjorklund's figure over three numbers a lane, or the cells a hand lit.
**A step is a crossing of `floor(16 × cycle)`**, `TickContext::cycle` in bars, stamped where
in the frame it fell. A new one stands still — Speed at 0, a sequencer starts stopped, as
silvia's does — and Speed 1 runs it at a bar every two seconds, 120 BPM; in Loop mode a Master
Gear a bar long cabled into Time is the tempo, and its Hold and Reset are the play and the
reset silvia's Start/Stop and Reset were. Nothing is integrated: the node remembers only last tick's
reading, to see what it crossed, and a cabled Time read as a count whole, or unwrapped where
its source declares its wrap, so a gear's Phase passing one is a frame's motion. A reading
that moves more than a bar in a tick, across a seek or onto another clock as its Time cable
is moved is a jump and fires nothing, and so is the first reading; so is one that moves
backwards on a clock, while running free a negative Speed plays the steps backwards, each
entered at its top boundary and held a gate length as forwards. A step it lands exactly on, with a gear driving Time or a Speed
moving it and the show playing, plays at once, so a render's first frame is its bar's downbeat; any other step it
lands in plays on the next tick, so a gear's Reset lands on the downbeat. A Time that stands
still — a gear held, the show paused — closes whatever a step opened as the Time passed it,
and leaves what a landing opened on the step it stands on open until the Time moves on, so a
render's first frame has step 0's gate open after a Hold warm-up as after Black or Run. Each
lit step opens its lane and the gate closes it a gate length later, and each lane reads the
absolute step modulo its own length, so a lane of five against sixteen keeps its phase. **Step is the one stateful
path**: while something is cabled into it, each down advances the grid by exactly one and the
sequencer ignores its Time, because an event clock — a tap, a threshold — is not a gear. Which
step last played stays in the `CpuNode` — `CpuNode::playhead`, which the grid rings, taken
modulo a multiple of every lane's length that an `f32` holds whole — and is never saved: a
patch opens with nothing lit. `stepsequencer`'s pattern is its one value, `Cells`, four lanes
of silvia's sixteen, drawn and edited in [the grid](ui.md) with silvia's Clear under it; a
click on a cell is one step back.

`xypad` is silvia's XY Pad, the one node here you throw: a puck on a square in the node's
body with drag, bounce, a spring to the middle, gravity on each axis, a temperature that
jitters it, gravity wells and tethers, and silvia's nine presets, from a DVD bounce to a
three-body tangle. It publishes `x` and `y` between the four range ends, `speed` in the same
units, and `bounced`, a one-frame gate on every edge it bounces off; each edge is silvia's
Bounce, Wrap, Clamp or Unbound, per axis, and **Click** is silvia's Slingshot or Cursor. The ten
knobs are rows with ports; the pad is [`Region::XyPad`](ui.md#the-xy-pad), a region that
claims the pointer. **Where the hand put the puck is four hidden controls** — `padX` and
`padY`, drawn under the pad as silvia's X and Y and bindable to a knob like any number, and
`vx` and `vy`, the velocity a throw or a preset leaves it with — and the tick takes each the
moment the document changes it and integrates everything in between, so a knob on Pad X moves
the puck and a throw flies on after the hand lets go. The flight, the trail and the wells are
runtime, as silvia's `runtimeState` is: a patch reopens with the puck at rest where the hand
last put it. A right-click on a well takes it away and a preset replaces them all; there is no
button that clears them. Its one action input is **Reset**, which puts the puck at rest in the
middle. The physics runs on the square, -1 to 1, and the range ends map it onto `x` and
`y`, which at silvia's default -1 to 1 are the same numbers; see
[decisions.md](decisions.md#the-xy-pad-the-hands-place-is-the-document-and-the-flight-is-the-ticks).

The math nodes come from two macros, since the only thing differing between them is one
expression. Node definitions are data, so a macro over them is ordinary — not the case if
they were trait implementations.

## Adding a node

1. `src/nodes/yourname.rs` with a `pub static DEF: NodeDef`.
2. Add `pub mod yourname;` and `&yourname::DEF` to `REGISTRY` in `src/nodes/mod.rs`.
3. `cargo test` — the registry tests check slug uniqueness, port key uniqueness, that option
   defaults are among their choices, that color defaults parse, that a hidden control has no
   port, and that a `Control::Press` sits on an `Action` input and nowhere else. Seven of them
   are constraints to satisfy rather than facts to discover:
   `uniform_ports_are_uniform_output_kinds` holds a uniform port — a number or a color — and
   an `OutputKind::Uniform` to each other — and an `Action` port to an `OutputKind::Action` — and
   refuses a field input on a CPU node, `uniform_outputs_and_cpu_state_come_together` refuses
   a CPU output without a `cpu` half or a `cpu` half with nothing to publish,
   `a_measurement_belongs_to_a_node_with_a_cpu_half` refuses a `measure_wgsl` with no `tick` to
   read the slot back, `a_dual_output_is_a_varying_number_a_tick_can_evaluate` refuses an
   `OutputDef::eval` anywhere its formula could not be the same function as the WGSL beside
   it, `a_moving_node_has_the_rows_its_timing_expands_into` holds a node with
   `NodeDef::timing` to the time rows, heading and mode `nodes::timing` expands it into, and
   `the_moving_nodes_and_their_rates_and_paces` names every node that moves with time with its
   rate and its pace,
   `no_node_function_reads_the_resolution` runs every generator of every
   node over every choice of every option and refuses `u_resolution`, `frag_coord`, the position
   builtin or `gl_FragCoord` in the emitted text, and `every_node_declares_a_category`
   refuses a node that is not in one of the menu's groups.
4. **Ask what field the body computed and threw away.** A shape's coverage, a noise value
   before it became a color, an escape count, a vignette's falloff, a halftone's ink: if the
   generator calculated one, it is an `OutputDef` beside the color. Name it out of the
   vocabulary in [what a generator
   publishes](#what-a-generator-publishes-beside-its-picture) — `mask` **only** for coverage
   or an inside/outside test, `value` for a raw quantity that is nothing more specific — and
  if the name it wants is not there, widen the list in
   `a_node_names_its_outputs_from_the_vocabulary` deliberately rather than by
   accident. If the field *is* the picture, as in a gradient, a checkerboard or an emboss,
   there is nothing to publish.
5. Snapshot the generated [WGSL](#wgsl), review it once by eye, and accept its `wgsl_`
   snapshots; `tests/shader_targets.rs` takes the module through naga to MSL, SPIR-V and HLSL.
6. Compile it on the GPU. `every_node_compiles_on_the_gpu` builds a shader for **every node
   in the registry, in every option combination, with inputs connected and unconnected**, and
   hands each to the driver. That is what makes porting cheap: a typo in a shader body is a
   test failure carrying the driver's own message, not a black frame found mid-set.
7. For a CPU node, give it CPU inputs, a `cpu` half, and the output kind that matches what
   it publishes — see [cpu.md](cpu.md). A parameter it wants to *write* rather than read is
   still an input with a control: `TickContext::write_control` moves the node's own knob
   through the command bus, which is how a roll of `slimemold`'s knobs lands somewhere a
   person can see it, and a private field would not be saved, shown or undoable. A node publishing a uniform number takes
   `OutputKind::Uniform` with `no_wgsl`, and is tested in `tests/uniform.rs` by ticking a
   headless `App` and reading `App::uniform` back. A node publishing a uniform **color** takes
   the same kind and is read back through `App::uniform_color`; `color` is the whole of that
   shape — a swatch its tick reads with `TickContext::color` and republishes with
   `publish_color`, which is what puts one color on every swatch downstream. A node publishing a *picture* — `camera`,
   `video` — takes `OutputKind::Texture` instead, publishes an `Arc<Frame>` from `tick`, and
   is tested the way `tests/video.rs` does it: tick until a frame arrives, then read the
   pixels back. A
   device it opens must fail into `error()`, never a panic: a missing microphone is a status
   line, not a crash.
