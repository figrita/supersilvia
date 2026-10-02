# Proposal: `gamepad`

**Status: built.** `src/gamepad/` is the device thread over `gilrs`, `src/nodes/gamepad.rs`
the node, and `tests/actions.rs` drives a button through the fake pad `gamepad::bench`
installs, so nothing has to be plugged in.
[docs/media.md](../docs/media.md#game-controllers) and
[docs/decisions.md](../docs/decisions.md) carry it. Agreed on 21 September 2026, which settled the
`gilrs` dependency below.

silvia's `gamepad` (`js/nodes/video/gamepad.js`) publishes
six axes as numbers and twenty buttons as actions with edge detection, from the browser's
Gamepad API. Left out of stage 10 of [context-free.md](context-free.md) because it needs a
dependency.

## The shape

A `Category::Source` node with an option naming the device (as `camera` and `audioin` name
theirs), six `UniformNumber` outputs for the sticks and triggers in `[-1, 1]`, and one
`Action` output per button, fired down and up from the device thread's edges with the same
sub-frame stamps the audio thresholds carry. A device thread publishes through a triple
buffer; the tick reads the newest and never waits, as every device here does.

## The dependency, settled

**`gilrs`** is the crate everyone uses; it is pure Rust on Linux over evdev, with its own
thread-free polling that a device thread here would wrap. It is a new dependency, and the
rules say to ask before adding one; that ask is answered. The alternative, reading evdev by
hand through `libc`, is a driver, not a node.

Done when: a controller plugged in appears in the option, its sticks read on the rows,
`tests/actions.rs` drives a button through a fake event source, and `doctor.sh` does not
require one to be present.
