# Proposal: `clock`, the wall clock

**Status: built** — `src/nodes/clock.rs`, `tests/uniform.rs`, and the reasoning in
[docs/decisions.md](../docs/decisions.md) under *The wall clock and the recorded curve*. One
departure from what is below: local time is the `offset` knob rather than `libc::localtime_r`,
because declaring `libc` and putting an `unsafe` call outside `render/` are two of CONTRIBUTING.md's
rules and changing either is a maintainer's call; and the node is `Category::Control`, where silvia
files it, rather than `Source`. silvia's `clock` (`js/nodes/video/clock.js`) publishes the
time of day — seconds, minutes, hours, seconds of the day, and each as a fraction of its
period — from `new Date()`, with no inputs and no reset. It is the one silvia number node
that reads the world rather than the graph's clock, and it was left out of stage 10 of
[context-free.md](context-free.md) because local time needs a timezone.

## The shape

A `Category::Source` node with no inputs, an `output` option (`Runtime`: seconds, minutes,
hours, hours12, daySeconds, dayProgress, and their normalized forms) and one `UniformNumber`
output `value`, ticked from the system clock once a frame. It never integrates `dt`; it
samples the world, so it is the one node an offline render cannot make deterministic, and
its module doc says so.

## The decision it needs

Local time. Two ways, neither free:

- **`libc::localtime_r`.** `libc` is already compiled into the binary as a transitive
  dependency, so declaring it directly adds no code, and glibc reads `TZ` and
  `/etc/localtime` for us. One `unsafe` call, which the rules put in `render/` only — so
  either the rule gains a second home for `unsafe`, or the call is wrapped once in a tiny
  module with its `// SAFETY:` line and the rule text names it.
- **UTC only, with an `offset` hours knob.** No dependency and no `unsafe`; the performer
  types their zone once. Wrong twice a year on its own.

The first is the honest clock. Done when: `tests/uniform.rs` holds `daySeconds` to the
system's local time within a second, and the `unsafe` rule in `CONTRIBUTING.md` names the site.
