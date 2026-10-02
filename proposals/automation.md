# Proposal: `automation`

**Status: built** — `src/nodes/automation.rs`, `widgets::curve`, `ValueKind::Points` and
`Value::Points`, with the round trip in `tests/project.rs` and the reasoning in
[docs/decisions.md](../docs/decisions.md) under *The wall clock and the recorded curve*.
Agreed on 21 September 2026, in silvia's own shape: persisted values, and runtime state kept
apart from them, with the values saved. silvia's `automation` records a knob's movement
as a list of `{time, value}` points, plays it back on a loop, and saves the recording in the
patch as a node value. It was left out of stage 10 of
[context-free.md](context-free.md) because a recorded curve looked like a kind of saved
state the file had no place for; the third store answers that, so it has one.

## The one shape: the recording is a value, the playhead is not

**The recording is one of the node's own values.** `Node::values` is the store for state
that is neither a port's control nor a choice out of a list, and a curve is exactly that —
see [docs/decisions.md](../docs/decisions.md) under *A node's own values are not options*,
and `src/nodes/note.rs` for how a node declares one: a `ValueDef { key, label, kind }` in
the `NodeDef`, drawn by the node's own code rather than by the shared option row. It is
saved with the patch, it travels with it, and it comes back where it was left.

**It needs a new `ValueKind`, and a new `Value` beside it.** `ValueKind` today has one
variant, `Text { rows, placeholder }`, and `graph::Value` has `Text(String)` and
`Range(ControlRange)` (`src/nodes/mod.rs`, `src/graph/node.rs`). Neither holds a list of
points. So the node adds **`ValueKind::Points`** — the declaration, with the bounds the
curve is drawn against — and **`Value::Points(Vec<Point>)`**, a `{ time: f32, value: f32 }`
each, as what an instance stores. That is the one new piece of machinery in this node.

**Runtime state is not saved.** Whether the node is recording or playing, and where the
playhead is, live in the `CpuNode` and are dropped on save, the way every other transient
does. Reopening a patch gets the curve back and the transport stopped at the start, which
is what a performer means by *the automation is still there*. Record, play, stop and clear
are actions; `input` is a `UniformNumber` it records and `output` a `UniformNumber` it
plays.

Rejected: making the curve a timeline lane ([timelines.md](timelines.md)) and letting
`automation` be the name of its record button — that makes a node wait on a workspace kind
it does not need, and the value store is where silvia put it too.

Done when: a knob's movement recorded into the node plays back on a loop, the curve
survives save, close and reopen with the transport stopped, a recorded node round-trips
through `tests/project.rs`, and the curve draws in the node's own region rather than in an
option row.
