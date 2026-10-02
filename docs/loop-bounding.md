# Loop bounding

wgpu puts a hidden counter into every loop of every shader it compiles. wgpu's name for it is
**force loop bounding**. It is on in every shader supersilvia draws with. This document says
what the counter is, why wgpu adds it, what it costs here, and what could be done about it.

The short version:

- The counter is a few integer instructions and one extra exit test at the top of every loop
  iteration, in every pixel.
- It exists so a driver's compiler can never treat a loop as endless and use that as licence
  to delete other code, such as a bounds check. That matters for memory safety when a shader
  comes from a stranger, as it does in a web browser.
- It does not stop a runaway shader in practice: it allows about 18 quintillion iterations.
- On the Intel UHD 770 it cost `lyapunov` about a quarter of its GPU time, measured at 80
  iterations of its exponent loop, and `bloom` and `kuwahara` about a fifth. Nodes without
  loops pay nothing. `lyapunov` runs a fixed ten iterations, [below](#lyapunov), which is an
  eighth of that work and a loop of constant length.
- Turning it off makes `pixelsort` two and a half times slower, so it is not a switch to flip
  for everything.
- It has nothing to do with threads waiting for each other. Each pixel has a counter of its
  own.

The decision about it is item 3 of [proposals/wgpu.md's
decisions](../proposals/wgpu.md#decisions).
[The recommendation](#the-recommendation) is at the end of this document.

Every claim about wgpu and naga below cites the source it was read from. Those crates are
naga 30.0.1, wgpu 30.0.1, wgpu-core 30.0.1, wgpu-hal 30.0.1 and wgpu-types 30.0.1, the versions
in `Cargo.lock`, under `~/.cargo/registry/src/*/`.

## What it is

supersilvia's shaders are WGSL. wgpu does not hand WGSL to the driver. naga, the shader
translator inside wgpu, turns it into the language the platform's driver takes: SPIR-V for
Vulkan on Linux, MSL for Metal on macOS, HLSL for DirectX 12 on Windows. The counter is added
during that translation, by each of naga's three writers:

- SPIR-V: `write_force_bounded_loop_instructions` in `naga-30.0.1/src/back/spv/block.rs`,
  called from the `Statement::Loop` arm of the block writer.
- MSL: `gen_force_bounded_loop_statements` in `naga-30.0.1/src/back/msl/writer.rs`. Its doc
  comment is the one that explains *why*; the other two writers point at it.
- HLSL: `gen_force_bounded_loop_statements` in `naga-30.0.1/src/back/hlsl/writer.rs`.

Each writer has a `force_loop_bounding` option, and each option's default is `true`
(`back/spv/mod.rs`, `back/msl/mod.rs`, `back/hlsl/mod.rs`).

### The counter, step by step

The counter is a pair of 32-bit unsigned integers, a `vec2<u32>`. The pair stands in for one
64-bit number, because 64-bit integers are an optional feature on GPUs and WGSL does not have
them. The SPIR-V writer says so in a comment: "We use a vec2<u32> to simulate a 64-bit
counter."

1. Before the loop, the counter is declared with both halves at `u32::MAX`, 4294967295.
2. At the very top of every iteration, before the loop's own code, the writer adds a test. If
   both halves are zero, the loop breaks.
3. Otherwise the counter goes down by one. The low half always drops by one, and the high half
   drops by one when the low half was already at zero, which is how a borrow works.

So a loop can run at most 2⁶⁴ − 1 times, about 1.8 × 10¹⁹, before the counter stops it.

The counter counts down rather than up. The comment beside it in all three writers says why:
counting up from zero hung certain Intel drivers, <https://github.com/gfx-rs/wgpu/issues/7319>.
The MSL writer's doc comment still sketches the older count-up version. The code below it
counts down.

### What it looks like

Here is a small loop in WGSL, of the shape the library is full of: sixteen steps, each adding
something to a sum.

```wgsl
@fragment
fn fs_main(@builtin(position) p: vec4f) -> @location(0) vec4f {
    var sum = 0.0;
    for (var i = 0; i < 16; i++) {
        sum += sin(p.x + f32(i));
    }
    return vec4f(sum);
}
```

This is the MSL naga 30.0.1 writes for it with `force_loop_bounding` off. It was produced by a
throwaway program calling `naga::back::msl::write_string`, trimmed to the function:

```cpp
float sum = 0.0;
int i = 0;
bool loop_init = true;
while(true) {
    if (!loop_init) {
        int _e16 = i;
        i = as_type<int>(as_type<uint>(_e16) + as_type<uint>(1));
    }
    loop_init = false;
    int _e5 = i;
    if (_e5 < 16) {
    } else {
        break;
    }
    {
        float _e8 = sum;
        int _e10 = i;
        sum = _e8 + metal::sin(p.x + static_cast<float>(_e10));
    }
}
```

And this is the same function with it on, the default. The three marked lines are the
counter; nothing else changes:

```cpp
float sum = 0.0;
int i = 0;
uint2 loop_bound = uint2(4294967295u);                              // added
bool loop_init = true;
while(true) {
    if (metal::all(loop_bound == uint2(0u))) { break; }             // added
    loop_bound -= uint2(loop_bound.y == 0u, 1u);                    // added
    if (!loop_init) {
        int _e16 = i;
        i = as_type<int>(as_type<uint>(_e16) + as_type<uint>(1));
    }
    loop_init = false;
    int _e5 = i;
    if (_e5 < 16) {
    } else {
        break;
    }
    ...
}
```

The Vulkan path is what runs on Linux. SPIR-V is binary, so the easiest way to read it
is to have naga's own SPIR-V reader turn it back into WGSL. This is the loop from the SPIR-V
naga writes with the counter on, read back that way:

```wgsl
var loop_bound: vec2<u32> = vec2<u32>(4294967295u, 4294967295u);
loop {
    let _e19 = loop_bound;
    if all((vec2<u32>(0u, 0u) == _e19)) {
        break;
    }
    loop_bound = (_e19 - vec2<u32>(select(0u, 1u, (_e19.y == 0u)), 1u));
    let _e27 = i;
    if (_e27 < 16i) {
    } else {
        break;
    }
    let _e29 = sum;
    let _e31 = i;
    sum = (_e29 + sin((_e18.x + f32(_e31))));
    continue;
    continuing {
        let _e36 = i;
        i = (_e36 + 1i);
    }
}
```

In the raw SPIR-V, the counter is ten instructions at the head of the loop: a load, a
two-lane compare, an `All`, a conditional branch to the loop's exit, then an extract, a compare,
a select, a vector construct, a two-lane subtract and a store. Two more instructions and two
block labels are SPIR-V's bookkeeping for the new branch. The counter variable is declared in
the `Function` storage class, which in SPIR-V means private to one invocation. The
`OpLoopMerge` that opens the loop carries no unroll hint either way; whether to unroll is left
wholly to the driver.

What it adds to the modules the library really writes, by SPIR-V size, from the compile
snapshots in `tests/snapshots/`:

| module | loops | SPIR-V words, counter off → on |
| --- | ---: | ---: |
| the sixteen-step example | 1 | 274 → 377 |
| `lyapunov`, unconnected | 4 | 2614 → 2900 |
| `bloom`, connected | 4 | 3182 → 3468 |
| `kuwahara`, connected | 6 | 4028 → 4444 |
| `pixelsort`, connected | 4 | 4518 → 4800 |

The size is not the cost. A loop's body runs many times, and the counter runs once per
iteration. [Why it costs anything](#why-it-costs-anything) is about that.

### How it reaches the driver

- `wgpu::Device::create_shader_module` passes `wgt::ShaderRuntimeChecks::checked()`, every
  check on (`wgpu-30.0.1/src/api/device.rs`). Every module supersilvia makes goes through it —
  an Output's program in `render/program.rs`, the probes, the simulations' kernels, and the
  renderer's own stages.
- `Device::create_shader_module_trusted(desc, runtime_checks)` in the same file is the
  `unsafe` door that takes a different `ShaderRuntimeChecks`.
- wgpu-core carries `runtime_checks` on to wgpu-hal's shader module unchanged
  (`wgpu-core-30.0.1/src/device/resource.rs`).
- On Vulkan, naga writes the SPIR-V when the pipeline is made, not when the module is. The
  device's standing naga options have `force_loop_bounding: true`
  (`wgpu-hal-30.0.1/src/vulkan/adapter.rs`), and a module whose checks differ gets a copy of
  them with the option turned off (`wgpu-hal-30.0.1/src/vulkan/device.rs`, `compile_stage`).
- On Metal the option is copied straight from the module's checks into naga's MSL options
  (`wgpu-hal-30.0.1/src/metal/device.rs`). DirectX 12 does the same in
  `wgpu-hal-30.0.1/src/dx12/device.rs`.
- wgpu's own internal shaders, which wgpu wrote itself, are made with every check off:
  `ShaderRuntimeChecks::unchecked()` in `wgpu-core-30.0.1/src/timestamp_normalization/mod.rs`
  and `src/indirect_validation/`.

## Why wgpu adds it

### An endless loop lets the compiler assume things

The reason is written out in the doc comment on the MSL writer's
`gen_force_bounded_loop_statements`.

In Metal's shading language, a loop that never ends and has no side effects is undefined
behaviour. The rule is inherited from C++. It means the compiler is allowed to assume such a
loop is never reached. From that, it may conclude that whatever conditions lead to the loop are
never true, and use that conclusion to delete other code.

The comment's example goes like this. Suppose a shader says "if `i` is 10 or more, loop
forever". The compiler decides that loop can never be reached, so `i` must be under 10 there.
Suppose naga had also put a bounds check elsewhere — "only write `a[i]` if `i` is under 10" —
so that the shader cannot write outside its array. The compiler now "knows" `i` is under 10,
so it deletes the check. The comment adds that Metal's compiler has been seen deleting the
endless loop itself, so code after it runs. A hostile shader could use that to read or write
memory it should never touch, and then exit normally.

The counter defeats this. A loop with a counter that can run out is a loop that ends, so the
compiler may no longer assume it is unreachable.

The comment also records what naga tried before. It once added a `volatile` flag that was
always false but that the compiler could not see through. That also made every loop look
finite, but it "prevented the compiler from making important, and safe, optimizations such as
loop unrolling and was observed to significantly hurt performance". The comment says the
counter convinces the compiler that the loop is finite while still allowing optimizations such
as unrolling. The same approach is used by Dawn, Chromium's WebGPU implementation; the
comment links its `prevent_infinite_loops.cc`.

The SPIR-V and HLSL writers do not repeat the argument. Each points at the MSL comment and
emits the same counter. That is background worth knowing: Apple's Metal compiler and
Microsoft's DXC are both built on LLVM, whose optimizer may assume a loop makes progress when
the source language promises it does, as C++ and so MSL do.
Vulkan drivers' compilers vary, and naga does not try to guess which ones do the same. Mesa's
Intel driver, the one measured here, is not built on LLVM.

### The web is why it is on by default

wgpu is the engine behind Firefox's WebGPU. In a browser, a shader is text a web page sent.
WebGPU's security model is that such a shader must not be able to reach memory outside what it
was given, however it is written. So every check that guarantees this is on unless the caller
promises otherwise.

The doc comment on `ShaderRuntimeChecks::force_loop_bounding` in
`wgpu-types-30.0.1/src/shader.rs` is the whole contract:

> If false, the caller MUST ensure that all passed shaders do not contain any infinite loops.
>
> If it does, backend compilers MAY treat such a loop as unreachable code and draw
> conclusions about other safety-critical code paths. This option SHOULD NOT be disabled
> when running untrusted code.

`ShaderRuntimeChecks::default()` is `checked()`, everything on.

supersilvia runs no stranger's shader. Every line of WGSL it draws is written by a node
generator in `src/nodes/` and assembled by `compile::wgsl`. No text a person types becomes
shader code: the one free-text option in the library, `lyapunov`'s `sequence`, is parsed by
`scan_sequence` into an array of zeros and ones before it reaches a shader.

### It does not stop a runaway shader

It is natural to read "loop bounding" as "a loop that runs away gets stopped". In practice it
does not.

2⁶⁴ iterations is a very large number. The UHD 770 runs at up to about 1.6 GHz. Even at one
iteration per clock, one pixel's loop would take about 365 years to use its counter up. A
shader stuck in a loop with the counter on hangs the GPU just as it would without it.

What stops a hung GPU is the operating system. On Linux, the kernel's Intel graphics driver
notices a job that has run too long and resets the GPU. The application's device is then lost,
and that is the end of the session: supersilvia writes its unsaved work into `.autosave/`, says
so, and exits ([rendering.md](rendering.md#one-device)).

So the choice is not between "hangs" and "stops". With the counter, an endless loop hangs as
written. Without it, an endless loop is undefined: the compiler may do anything with it,
including deleting it or code near it.

## Why it costs anything

Some of this is established from source or from measurement. Some of it is inference, and some
is the likely mechanism, which nobody has confirmed on the hardware measured here. Each part says which.

### Established

**The counter's own work, every iteration.** From the SPIR-V above: a load, a two-lane
compare, an `All`, a branch, an extract, a compare, a select, a two-lane subtract and a store.
Once the driver has turned the variable into a register, that is roughly half a dozen to ten
integer instructions and one more exit test per iteration. It runs in every pixel. It also
keeps two more 32-bit values alive for the whole loop, in every pixel.

**No unroll hint.** naga writes each loop with no unroll or don't-unroll hint, counter or not.
Whether a loop is unrolled is the driver's own decision.

**The measurements.** With the counter off, `lyapunov`'s Output pass, at 80 iterations, went
from 17.33 to 13.08 ms, `kuwahara`'s from 6.07 to 5.27 ms and `bloom`'s from 2.75 to 2.15 ms. `pixelsort`
under `posterize` went from 4.13 to 10.10 ms. See [what we measured](#what-we-measured).

### Inferred from the measurement

**On anv, the counter seems to stop constant loops from being unrolled.** *Unrolling* is the
compiler writing a loop's body out once per iteration, so the loop disappears. A loop of eight
becomes eight copies of its body in a row.

The counter starts at a constant, so in an unrolled loop every copy of it would be a known
constant too, and every one of its exit tests would be known to be false. A compiler that
unrolled such a loop would delete the counter entirely, and the counter would cost nothing.

`bloom`'s loops have constant trip counts: eight arms, three rings each, in the demo. Yet the
counter costs `bloom` a fifth of its time. The simplest explanation is that anv's compiler does
not unroll those loops while the counter is there, and does unroll them when it is not. That
is an inference from the numbers; nobody has read anv's compiled output to confirm it. naga's
own comment says the counter was designed to *allow* unrolling, and on Apple's compiler it may.

### Likely, not measured

If a loop with a fixed count stays a loop, three further costs follow. These are standard
behaviour for GPU compilers, but none has been confirmed on the hardware measured here for these shaders.

- **Nothing that depends on the iteration is folded.** In `bloom`, each arm's direction is
  `cos(a)` and `sin(a)` of `a = i × 2π / 8`. Unrolled, those are eight pairs of constants
  worked out when the shader compiles. As a loop, they are two trigonometric instructions per
  arm, in every pixel. In `kuwahara`, each tap tests which of four quadrants it falls in with
  `x <= 0 && y <= 0` and three more. Unrolled, every test is known in advance and vanishes. As a
  loop, all four run for every tap.
- **Small arrays indexed by the loop counter may leave registers.** `kuwahara` keeps
  `mean` and `square` as arrays of four and walks them with `for (var q = 1; q < 4; q++)`. With
  a constant index, each element is just a register. With an index only known at run time, the
  compiler must either keep the whole array in registers and pick with a chain of compares, or
  put it in *scratch memory*, per-pixel storage in the GPU's memory that is far slower than
  registers.
- **Register pressure.** Every value alive across the loop takes registers in every pixel.
  Intel's compiler builds a fragment shader for several widths — 8, 16 or 32 pixels at a
  time — and a shader that needs more registers may only fit the narrower widths, or spill to
  scratch. Either is slower. The counter adds two live values to every loop, and a loop that
  is not unrolled keeps more values alive than one that is.

### Why it lands on a few nodes

The counter costs a little per loop iteration, per pixel. A node's bill is that little times
the number of iterations its pixels run:

- A node with no loop pays nothing. Most of the library has no loop.
- A 3×3 edge detection runs nine iterations per pixel; the counter's share of each is small
  beside a texture read.
- `kuwahara` at 9×9 runs 81 iterations per pixel, plus its quadrant loop. `bloom` at medium
  runs 24.
- `lyapunov` runs its exponent loop three times over — once for the height and twice more to
  light it. Measured with the demo's 80 iterations that was 240 per pixel; at its fixed ten it
  is 30. The loop's own body is small, two sines and a log, so the counter is a large part of
  each iteration.

That is why the whole of the difference lands on three or four per-pixel kernels, and every
other Output in the demo measured within a quarter of a millisecond of glow or faster.

## What it is not

It is not threads waiting for each other. There is no contention, no lock and no
synchronization involved.

A fragment shader runs once for every pixel. Each of those runs — an *invocation* — has its
own copy of every local variable, and the counter is one of them. The SPIR-V declares it in the
`Function` storage class, which is private to one invocation. Nothing about the counter is
shared between pixels. There are no atomic operations in it, no barriers and no shared
memory. One pixel's loop never waits for another's counter.

Contention was a reasonable guess, because "the GPU is slower and something is shared" is
often the answer. The nearest real thing to it is elsewhere. The GPU runs pixels in groups
that move in lockstep, so a loop takes as long as its slowest pixel in the group. The counter
does not change that, because every pixel's counter moves in step with its loop.

The cost is simply more instructions per iteration, and possibly worse code around them.

## What we measured

On the Intel UHD 770 under Mesa: glow drew through iris, Mesa's OpenGL driver, and wgpu draws
through anv, its Vulkan driver. Both use Mesa's one Intel shader compiler. glow's shaders were
GLSL, which had no counter.

Every `lyapunov` figure here, and the Games and simulations tab's, is with the node running 80
iterations of its exponent loop, the count the demo's Iterations control held. It runs a fixed
ten, and has not been measured at ten.

**Tick rates and per-node cost, glow against wgpu, on an idle machine.** Release builds, the
twelve-tab demo project `21 September` (not in the repository), `tick_bench <project> <tab> 100 8`, three alternating rounds of
eight seconds each.

| tab | glow, ticks/s | wgpu, ticks/s | wgpu short by |
| --- | ---: | ---: | ---: |
| Start here | 100 | 100 | — |
| Effect kernels | 56.5 | 52.8 | 7% |
| Effect manglers | 87.4 | 79.9 | 9% |
| Games and simulations | 58.4 | 45.1 | 23% |

| Output (the node upstream of it) | glow, GPU ms | wgpu, GPU ms |
| --- | ---: | ---: |
| `lyapunov` | 11.85 | 17.33 |
| `posterize` over `pixelsort` | 2.89 | 4.12 |
| `kuwahara` | 5.37 | 6.10 |
| `bloom` | 2.17 | 2.75 |
| `camcordercrt` | 3.15 | 2.83 |
| `geissflow` | 0.64 | 0.36 |

Every other Output was within 0.25 ms either way. The GPU was 98–99% busy under wgpu against
93–96% under glow. So the renderer is not leaving the GPU idle; what remains of the gap is the
shaders themselves.

**Each runtime check turned off in turn**, by the performance lane, per Output's GPU ms. The
table is also in [proposals/wgpu.md](../proposals/wgpu.md#the-synths-submissions-after-the-flip).

| Output (upstream) | checked | loop bounding off | int division checks off | bounds checks off | all off |
| --- | ---: | ---: | ---: | ---: | ---: |
| `lyapunov` | 17.33 | 13.08 | 17.33 | 17.16 | 12.46 |
| `kuwahara` | 6.07 | 5.27 | 6.09 | 6.08 | 5.26 |
| `bloom` | 2.75 | 2.15 | 2.75 | 2.75 | 2.15 |
| `posterize` over `pixelsort` | 4.13 | **10.10** | 4.13 | 4.11 | **10.28** |
| tab: Games and simulations, ticks/s | 42.3 | 51.9 | 42.2 | 42.4 | 53.0 |
| tab: Effect kernels | 46.9 | 50.3 | 47.1 | 46.7 | 51.3 |
| tab: Effect manglers | 72.3 | 50.1 | 71.8 | 71.6 | 49.9 |

Its per-Output figures agree with the idle ones above to a few hundredths of a millisecond. Its
tab rates are lower: its checked row matches the renderer as first flipped, one submission per
Output, before cheap Outputs were grouped into shared submissions. They compare with each
other, not with the table above.

What the two tables say together:

- **Loop bounding is the only check that costs anything.** Integer-division guards and
  bounds checks are within noise.
- **The counter accounts for all of `kuwahara`'s and `bloom`'s gap to glow**: with it off,
  5.27 against glow's 5.37, and 2.15 against 2.17.
- **It accounts for about three quarters of `lyapunov`'s**: 4.25 of the 5.48 ms. The rest is
  some other difference between the two compilers' input.
- **`pixelsort` goes the other way.** Without the counter, anv compiles its loops two and a
  half times slower, 10.10 against 4.13, and Effect manglers falls from 72 to 50 ticks a
  second.

**Why `pixelsort` gets slower — likely, not measured.** `pixelsort` gathers a run of 32 pixels
into two local arrays and insertion-sorts them: a loop of up to 31 with a loop inside it that
walks back through the arrays, both with early exits. The likely story is that, freed of the
counter, anv's compiler unrolls the outer loop. That writes the inner loop out up to 31 times,
and every copy still reaches into the arrays by an index only known at run time. The shader
grows many times over. The consequence would be some mix of the costs above: more registers
than a wide build can hold, spills to scratch, and more code than the GPU's instruction cache
keeps. With the counter, the loop stays a loop and stays small.

What argues it is a compiler choice and not the loop's own cost: glow drew the same Output
from the node's GLSL twin, with no counter, in 2.89 ms, through the same Mesa compiler. So the
compiler can make that loop fast; from naga's SPIR-V without the counter, it chose something
worse. Reading anv's compiled output (see [measuring it yourself](#measuring-it-yourself))
would settle it.

## Lyapunov

`lyapunov` is in `src/nodes/fractals.rs`. Its exponent is one loop, `EXPONENT_WGSL`, run a
fixed `ITERATIONS` times — ten:

```wgsl
for (var n = 0; n < 10; n++) {
    let r = select(BBB, AAA, seq[n % SEQLEN] == 0);
    let s = sin(x + r);
    x = amp * s * s;
    sum += log(max(abs(amp * sin(2.0 * (x + r))), 1e-12));
}
```

- **The count is a constant, and the node has no control for it.** Lyapunov needs
  about ten iterations, fixed, never eighty. The generator writes `10` into the
  loop's bound and into the average, `sum / f32(10)`, so the loop has no early exit and its
  trip count is known when the shader is written — a compiler is free to unroll it, and once
  unrolled `n % SEQLEN` is a constant in every copy and the counter folds away. `bloom`
  suggests anv keeps a constant loop a loop while the counter is there, so on anv the
  counter likely still runs, ten times rather than eighty.
- **It is an exception to a recorded rule.** [decisions.md](decisions.md#a-number-stays-varying)
  keeps counts such as iterations as fields, so a noise can drive them per pixel; this node is
  the one that does not, and says why there.
- **A project saved with an Iterations value or a cable into it** opens with the node whole: the
  value is reported as a control the node does not have and the cable as dropped, as for any
  control or port a node no longer has.
- **The loop runs three times per pixel** on the color output: once for the height `h`, and
  twice more, `hx` and `hy`, to light it as a height map. The `mask` output runs it once. The
  lighting runs only while `depth` is above 0.001.
- **`seq`** is a local array of the sequence, twelve long for `A6B6`, indexed by `n % SEQLEN`.
- **Presets.** The `sequence` option offers eight named sequences — `AB`, `AAB`, `ABB`,
  `AABB`, `AABAB`, `ABBB`, `A6B6`, `B6A6` — and takes typed text beside them. The demo sets
  `sequence` to `AABAB`, with `depth` at 1, so every pixel runs 30 iterations.

What the fixed count does:

- **Fewer iterations.** Ten in place of the demo's eighty is an eighth of the loop's work. The
  loop is almost all of the node's cost, so the 17 ms pass measured at eighty should fall to a
  few milliseconds. That figure is a proportion, not a measurement: the pass has not been timed
  at ten.
- **The long pass the editor waited behind.** Item 4 of [proposals/wgpu.md's
  decisions](../proposals/wgpu.md#decisions): the editor missed frames beside
  Games and simulations because an editor frame waited behind Lyapunov's single long pass, and
  a pass of a few milliseconds is short enough to wait behind.
- **Not done: the ten steps written out.** The generator knows the count and the sequence, so
  it could write each step with its rate, `a` or `b`, named directly. That shader would have no
  loop, no array, no remainder and no counter, whatever wgpu's setting, and is the sure way to
  the unrolled cost if a measurement says the constant loop does not get there.

## Every loop in the library

wgpu's contract for turning the counter off is that no shader contains an endless loop. So
whether the library can meet it is the first question for any option but the first.

There are 46 loops in the WGSL the library writes: 43 in node outputs and 3 in `slimemold`'s
compute kernels. The renderer's own stages and the prelude, `compile/prelude.wgsl`, have none.
None is a `loop {}` or a `while`; every one is a `for` with an integer counter. None counts in
floating point, which can stall when the step is too small to change the value. Every one ends
within a small constant number of iterations, in one of three shapes:

- **A constant count.** The bound is a literal, or an option's value the generator writes in as
  a literal: `lyapunov`'s exponent (10), the box, sharpen, 3-tap and 3×3 kernels in `convolve.rs`, `bloom`'s arms and rings,
  `kuwahara`'s window and its quadrant loop, the sinc window, the directional and radial blurs'
  sixteen taps and the supersampling grid in `multisample.rs`, `scatter`'s three layers and
  neighbourhood in `distort.rs`, `edgedetection`'s 3×3, `screentone`'s loops, the cellular
  noise's 3×3 in `noise.rs`, and `slimemold`'s diffusion 3×3.
- **A constant ceiling with an early exit.** The written bound is a literal and a `break`
  stops it sooner at a count that is a field: the polygon in `shapes.rs`
  (12), the three spirals (20), `phyllotaxis` (500), the fractal noise's octaves in `noise.rs`
  (8), `tunnel3d`'s ray march (32), and `pixelsort`'s gather loop and the outer loop of its
  sort.
- **A bound that is not a literal, but is bounded.** `mandelbrot` and `juliaset` run
  `for (; i < maxIter; i++)`, where `maxIter` is `i32(clamp({iterations}, 1.0, 500.0))`, so at
  most 500. `pixelsort`'s inner sort loop runs `j` from `i - 1` down to 0, where `i` is below
  the chunk size. `slimemold`'s tile fill steps `k` from the invocation's index by a constant
  up to a constant.

So the claim "every loop has a constant bound" is true in effect but not in form. Every loop
ends, but three of them end because of a clamp or a direction of travel, not because their
bound is written as a number.

**How a gate could hold it.** `tests/shader_targets.rs` already parses every module the
compiler can write, each node unconnected and connected, every probe and every choice of every
option that changes the code. It could walk each module's naga IR and require of every
`Statement::Loop`:

- that it opens with naga's lowering of a `for` condition, `if cond {} else { break; }`;
- that `cond` compares one local integer variable with a constant expression;
- that the loop's `continuing` block is exactly that variable stepped by one toward the
  constant, and nothing in the body stores to the variable.

A loop meeting that rule ends, whatever the variable starts at. `pixelsort`'s inner loop
meets it: it compares with the literal 0 and steps down by one. `mandelbrot` and `juliaset` do
not, since `maxIter` is not a constant, and would be rewritten to the ceiling-and-break shape
the rest of the library uses. A `loop` or `while` in a node's WGSL would fail the gate. So would
any future node that took WGSL from a person.

## The options

### 1. Keep it on everywhere

What it is today. Every module is made with `create_shader_module`.

What a user sees: nothing changes. As measured with Lyapunov at 80 iterations, Games and
simulations was about 45 ticks a second on an idle machine against glow's 58, Effect kernels
about 53 against 56 and Effect manglers about 80 against 87; Lyapunov's fixed ten take most of
its pass away whatever this setting, which leaves Bloom and Kuwahara paying the counter. Pixel
Sort stays at its faster speed. Nothing new to maintain.

### 2. Turn it off, per module, where it is safe

`Program::create` in `render/program.rs` would make the module with
`create_shader_module_trusted`, passing every check on except this one:
`ShaderRuntimeChecks { force_loop_bounding: false, ..ShaderRuntimeChecks::checked() }`.

What it takes:

- **`unsafe`.** The call is an `unsafe fn`. CONTRIBUTING.md allows `unsafe` in `render::picture` and
  `render::dmabuf` alone, so this is a third module in that rule, with a `// SAFETY:` line
  citing the gate above.
- **The gate above**, so that the promise wgpu asks for is checked on every module rather than
  trusted.
- **A per-module choice.** The switch is per shader module, and one module is one Output's
  whole chain with every node inlined. `posterize` over `pixelsort` is a single module. So a
  module could turn the counter off only if every node in it allows it, which would be a bit on
  the node's definition — `pixelsort`'s saying it keeps the counter — and read by the
  compiler, since nothing outside `nodes/` may match on a slug.

Two forms of it:

- **Off everywhere.** Games and simulations from 42 to 52 ticks a second and Effect kernels
  from 47 to 50, on the renderer as first flipped. But Effect manglers from 72 to 50, because
  Pixel Sort is two and a half times slower.
- **Off except in modules holding a node measured to lose.** Lyapunov, Kuwahara and Bloom get
  their quarter and fifth back — Lyapunov's quarter of a much shorter pass — and Pixel Sort keeps its speed. Every new node with a loop
  would then want measuring both ways.

What a user sees: some nodes faster, a Pixel Sort that must be kept out of it, and a rule
that each new loop is measured. A shader loop that never ended would be undefined rather than
merely hung, but the gate is what makes that loop impossible to write.

### 3. Rewrite the loops that pay

Leave the counter on, and change the loops so it stops costing:

- **`lyapunov` at a fixed count of 10**, which is built: an eighth of
  the demo's iterations in a loop of constant length, [as above](#lyapunov). This is the
  largest single win, and it holds under any setting of the counter. Writing the ten steps
  out, so no loop is left to bound, is the step after it if a measurement asks for one.
- **Constant kernels written out by the generator.** `bloom`'s and `kuwahara`'s loops have
  counts the generator already knows, because they come from options. The generator could
  write each tap out as its own line in place of a loop, which is what the driver would have
  done by unrolling. With no loop there is no counter. The loop-bounding-off figures are the
  upper bound on what this can win: Kuwahara from 6.1 to about 5.3 ms, Bloom from 2.75 to
  about 2.15. That it reaches those figures is not measured. A large window, such as
  Kuwahara's 13×13, makes a long shader, which should be measured before it is kept.

What a user sees: Lyapunov at its fixed count costs a few milliseconds;
Bloom and Kuwahara are as fast as with the counter off; Pixel Sort is untouched. No `unsafe`, no
new rule, and nothing that behaves differently on the Mac.

## The recommendation

**Option 3, with the counter left on.** Lyapunov's fixed ten iterations are its first half,
built; the rest is having the kernel generators write constant loops out where a measurement
says it wins, starting with Bloom and Kuwahara.

The reasons:

- Lyapunov was most of the loss, and its fixed count removes most of its cost whether or not
  the counter is on. It also shortens the long pass the editor waited behind.
- It needs no `unsafe`, no promise to wgpu and no new hard rule.
- It leaves Pixel Sort alone, which option 2 has to work around.
- It carries to the Mac unchanged. Nothing about the counter's cost has been measured on
  Apple's compiler, and naga's comment, written about that compiler, says the counter still
  lets it unroll, so the Mac may pay less than anv does.

Option 2 stays available, with its gate, for a node that still pays after its loops are
rewritten.

## Measuring it yourself

On an idle machine: no browser, no video player, nothing else drawing on the iGPU. Earlier runs
that overlapped a browser and a video read low and noisy.

```sh
cargo build --release
cargo run --release --example tick_bench -- "<project folder>" "<tab>" 100 8
```

The first line of the report is the tab's ticks a second. The table under it has one row per
drawn Output, named by the node upstream of it; the **GPU ms** column is that Output's pass,
which is the cost of the whole chain inlined into it — the named node and everything upstream
of it. The last lines give the GPU per tick and how busy the render engine
was. Run each setting several times, alternating the two, and compare like with like.

To compare with the counter off, change `Program::create` in `src/render/program.rs` locally,
for the measurement only:

```rust
// SAFETY: a local measurement; every loop in the library ends (docs/loop-bounding.md).
#[allow(unsafe_code)]
let module = unsafe {
    device.create_shader_module_trusted(
        wgpu::ShaderModuleDescriptor {
            label: Some("output"),
            source: wgpu::ShaderSource::Wgsl(shader.body.as_str().into()),
        },
        wgpu::ShaderRuntimeChecks {
            force_loop_bounding: false,
            ..wgpu::ShaderRuntimeChecks::checked()
        },
    )
};
```

To see the counter in the code naga writes, dump every module's MSL:
`SHADER_TARGETS_DUMP=/tmp/msl cargo test --test shader_targets`, then look for `loop_bound`.

To see what anv made of a shader — the width it compiled for, how many instructions, whether
it spilled — Mesa's `INTEL_DEBUG` environment variable prints the compiled fragment shaders
with a line of statistics for each. That is the way to confirm or refute the "likely" parts
above, and in particular what happens to Pixel Sort without the counter.
