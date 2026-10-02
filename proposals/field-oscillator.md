# Proposal: the field oscillator

**Status: cut from `oscillator` on 2026-09-11, parked under its own name.** Nothing here
exists. It records the node that was cut when `oscillator` became silvia's CPU node in
[context-free.md](context-free.md) stage 6 — and the shape it comes back in if it does:
`waveform`, a name of its own, because one name for a circle and a diamond is the ambiguity
the context-free graph exists to remove. Not rejected, not queued: to be built when
something wants it. See [decisions.md](../docs/decisions.md#oscillator-is-a-cpu-node).

## What was cut

The `oscillator` that shipped before stage 6 was pure GLSL: `time`, `frequency`, `phase`,
`amplitude` and `offset` as `VaryingNumber` inputs, `time` falling back to `u_time`, and a
waveform evaluated per fragment. Its output was a *field*. Fed a constant time it was an
LFO; fed a field — a gradient, a noise, a distance — it was a spatial wave, ripples over a
picture at a frequency and a phase that could themselves be pictures. silvia has no such
node, and the CPU oscillator cannot do it, because a `tick` has no `uv`.

## The shape, if it is wanted

A **`waveform`** node, `Category::Generate`, with the cut node's ports exactly, `time`
included with its `Control::Global("u_time")` fallback, and the seven waveforms as a `Code`
option. It publishes `output` as a `VaryingNumber` field and nothing else; a uniform number
use is what `oscillator` is for. The footer trace it carried — `NodeDef::trace_samples`, a
pure function of the controls — comes back with it unchanged.

The reason not to keep both under one name is the reason it was cut: two nodes that share a
name and differ in whether their output is a circle or a diamond would be the one ambiguity
the context-free graph exists to remove.
