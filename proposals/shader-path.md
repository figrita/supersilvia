# Proposal: the shader path to wgpu

**Status: decided, being built.** Every shader wgpu draws is written in **WGSL**. The compiler
writes a WGSL module per Output and per cost probe beside the GLSL shader glow draws, each
node carries a WGSL generator beside its GLSL one, and `tests/shader_targets.rs` takes every
module through naga to MSL and SPIR-V. The node-by-node conversion and the picture comparison
are steps of [proposals/wgpu.md](wgpu.md#the-steps). The conventions a node's WGSL is written
to are [docs/nodes.md](../docs/nodes.md#wgsl).

## The decision

> node generators write **WGSL** → naga `wgsl-in` → naga validation → wgpu (naga writes MSL
> on Metal and SPIR-V on Vulkan)

naga is inside wgpu already. With WGSL there is nothing between the compiler and it: no
dialect transform, no second compiler, no SPIR-V read back in. The error a node author sees is
naga's, with a span in the module, from the validator wgpu itself runs.

The spike before this recommended keeping GLSL and translating it through glslang. That was
overruled: the node library may drift from silvia's GLSL, and the simpler path wins. The
price is rewriting about 135 node shaders once, which is mechanical, parallel by file, and
gated by a test that fails on any module Metal or Vulkan would refuse.

## What the spike measured, and why glslang lost

It took all 421 shaders the program could write then — every node both ways, the probes, the
renderer's stages and `slimemold`'s kernels — down three paths:

| path | pass | median |
| --- | --- | --- |
| naga `glsl-in`, GL 4.30 with the version line moved | 1 / 421 | — |
| naga `glsl-in`, dialect-shifted to Vulkan GLSL | 268 / 421 | 0.41 ms |
| glslang → SPIR-V → naga `spv-in`, dialect-shifted | 420 / 421 | 1.48 ms |

naga's GLSL frontend has no atomics, so it could take no tap and no probe. glslang took
everything but one kernel, which a naga `spv-in` bug refused (`OpSMod` on an `ivec2`, lowered
through a cast of the whole vector's width). It worked. It lost on what it costs to keep:

- **A C++ build and a transform.** glslang is C++17 compiled by `cc`: 180 CPU-seconds from
  clean, an HLSL frontend compiled whether or not it is used, and +3.2 MB on the stripped
  binary. And GLSL for Vulkan is not the GLSL the compiler wrote: loose uniforms into a block,
  combined samplers split, bindings renumbered, locations added. That transform would have
  lived in the compiler for good.
- **Two readers of one program.** glslang's SPIR-V is read back by naga's `spv-in`, whose bugs
  are the ones the spike hit, before naga writes MSL.
- **The language wgpu speaks is WGSL.** Its examples, its errors and its validator's
  diagnostics are about WGSL, and so is everything that goes wrong on a Mac.

The whole WGSL path — parse, validate, MSL and SPIR-V — measures 0.3 ms median and 1.9 ms at
most over the gate's 385 modules (release, Linux, Intel UHD 770), the largest converted noise
modules about 1.4 ms. glslang alone was 1.2 ms median. The driver's own compile of what naga writes
comes after either, and is the cost that matters (`proposals/wgpu.md`, linking).

## The conventions, in brief

The whole of them is [docs/nodes.md](../docs/nodes.md#wgsl). The decisions a renderer lane
leans on:

- **One module per Output**, with the shared vertex stage `vs_main` (a triangle from
  `vertex_index`) and `fs_main` in it. `fs_main` builds `uv` from `@builtin(position)` with no
  flip, so an Output's texture holds GL's rows: row 0 is the bottom of the picture.
- **Everything in group 0, computable without reflection.** The uniform struct `u` at 0, the
  tap buffer at 1 where there are taps, four shared samplers at 2 to 5 (mirror or repeat,
  linear or nearest — what an output's `wrap` and `filter` can say) where there is any
  texture, and the textures from 6 up in their uniform names' order.
  `compile::wgsl::bindings` lists them and `compile::wgsl::uniform_layout` gives the struct's
  offsets by WGSL's uniform layout rules, which the gate checks against naga's own.
- **Every uniform is a member of `u`**, the two standard ones first: `u.u_resolution`,
  `u.u_time`, then the generated ones by name. Four samplers leave Metal's sixteen per stage
  and wgpu's default of sixteen textures per stage to the textures.
- **Sampling is `textureSampleLevel(…, 0.0)`, always.** naga does not check WGSL's uniformity
  rule for implicit derivatives, so the gate refuses `textureSample(`, `dpdx`, `dpdy` and
  `fwidth` in any module. No texture has mips, so level 0 is what GL read.
- **The tap buffer is `array<atomic<u32>>`**, and a probe counts a call site through a
  wrapper function, because WGSL has no comma operator.
- **A node without WGSL still compiles**, to a magenta placeholder and a
  `Diagnostic::Untranslated`, so a half-converted library always validates and the gate
  counts how much is left.

## What did not map, and what naga does not do

- **No sequence operator.** The probe's `(atomicAdd(...), call)` became a counting wrapper
  per call site.
- **No overloading.** A GLSL helper set overloaded on type became one name per type, and a
  test parses every helper in the registry into one module so a clash across nodes fails
  before a graph finds it.
- **Immutable parameters.** GLSL bodies that stepped a parameter (`fbmNoise`'s `freq`,
  `hashBits`' `x`) copy it into a `var`.
- **No scalar splat in builtins.** `clamp(v, 0.0, 1.0)`, `max(v, 0.0)`, `pow(v, 2.2)`,
  `step(0.0, v)` and `smoothstep(0.0, 1.0, v)` on a vector are errors; `mix(a, b, t)` and
  arithmetic with a scalar are not.
- **`mod` is not `%`.** GLSL's `mod` floors; WGSL's `%` truncates. The prelude carries
  `floor_mod` for it.
- **A vector `==` is a vector.** GLSL's `cell == ivec2(0)` is `all(cell == vec2i(0))`.
- **An atomic buffer has no plain write.** `sample`'s `tap[i] = …` is `atomicStore`.
- **Reserved words.** `mod`, `filter`, `common`, `target`, `smooth` and more cannot name a
  local.
- **Constant expressions that overflow are errors**, where GLSL wrapped; run-time `u32`
  arithmetic still wraps, which a tap's carry relies on.
- **naga accepts `textureSample` in non-uniform control flow**, which WGSL forbids and a
  driver answers badly. Hence the gate's ban.

## What this does not prove

- **The MSL has never been compiled by Metal.** On a Mac:
  `SHADER_TARGETS_DUMP=<dir> cargo test --test shader_targets`, then every `*.metal` through
  `xcrun metal -c`.
- **The SPIR-V has not been through `spirv-val`**, which is not installed here.
- **Nothing is drawn.** A module that validates type-checks; whether it draws the picture
  GLSL drew is the per-node comparison in [wgpu.md](wgpu.md#7-staging), once the renderer
  can draw.
- **The renderer's own stages and `slimemold`'s kernels** are not in the gate yet: they are
  written in WGSL with the renderer, and join the corpus there.
