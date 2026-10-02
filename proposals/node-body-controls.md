# Proposal: a node's body is its own, and the special cases fold into it

**Status: agreed, and being built — stages 1, 2, 3 and 4 of *Order* have landed.** Two things are
tangled here and this page used to hold only the first. Four silvia nodes put a hand directly on the node — `xypad` (a 2D pad with
physics, wells and a sling), `stepsequencer` (a 4 × 16 grid of toggles), `drawingcanvas` (a
paint surface), and the automaton's and the mold's paint and push on their previews. All were
left out of stage 10 of [context-free.md](context-free.md), or landed without that half.

The reason they were left out is the second thing, and it is the bigger one: **there is
nowhere for a node to draw.** `ui/` draws rows, and every body region that is not a row has
been added to `NodeDef` as a flag and to `ui/canvas.rs` as a branch, five times over. A sixth
would be `xypad`, a seventh `stepsequencer`, an eighth `drawingcanvas`. This page now proposes
retiring that pattern rather than adding to it.

## What this replaces

The previous version of this page proposed **three new control kinds** — a pad, a step grid,
a paint surface — each an `OptionDef`/`InputDef` kind with its own row height, "in the same
style the checkbox row and the free-text row were added". That is the wrong shape. It makes
`ui/` learn three more things about what a node might be, and the next node that wants
something none of the three cover is a fourth. The objection, on reading it: every special case belongs folded into the one custom area,
with composition favored over a monolith of nested `if`s.

The pad, the grid and the paint surface all survive as **things nodes draw in their own
area**. What does not survive is `ui/` knowing which of them a given node has.

## What the five flags were

Five bits on `NodeDef` existed only so that layout could size a body without consulting the
registry, and each was documented in place as an escape — "the third bit that escapes
`nodes/`", "the fourth", "the fifth":

| bit | what it buys |
| --- | --- |
| `is_output` | the Output's own render, flush at the foot of the body |
| `has_scope` | an audio node's scope band |
| `trace` | a fixed 200×48 band for a shape a number cannot show |
| `preview` | a source's own picture, named by the port it comes from |
| `width` | a wider body for a node whose longest row does not fit |

They were mentioned **64 times across four files** — 37 in `ui/canvas.rs`, 13 in
`nodes/mod.rs`, 12 in `ui/node_widget.rs`, 2 in `ui/mod.rs`.

`ui::canvas::Footer` is the nest itself: an enum of `None`, `Thumb`, `Scope`, `Preview`,
`Trace`, `Pad`, resolved by a priority ladder in `Footer::of`, with `footer_height` and
`footer_rect` matching on it again. The ladder is not only ugly, it is **a decision nobody
made on purpose**: a preview beats a scope, so an audio node that shows a picture silently
loses its scope band. `scope_band` is the bolt-on that half-answers this — a scope may
coexist with a footer, but only when that footer is not itself the scope.

## The reference shape: silvia's `customArea`

`js/snode.js` is 1,501 lines and contains **no branch on node kind**. No `isOutput`, no
scope, no preview, no trace; every `width` in it is generic geometry — tooltip placement, and
the workspace extent measured back off the DOM. What it emits, once, after the rows and the
options, is:

```html
<div class="node-custom" data-el="customArea"></div>
```

It hands that element to the node as `this.customArea` and calls the node's own `onCreate`.
That is the entire hook. On a rebuild it empties the div and calls `onCreate` again.

The nodes do the rest, and the Output is the case that settles the argument:

- **`nodes/video/output.js`** builds its status line and its `<canvas>` in `onCreate` and
  appends both to `customArea`. The base class does not know an Output has a picture. Even
  *being* an output is the node's own line — `SNode.outputs.add(this)` — rather than a flag
  the base reads.
- **`nodes/audio/analyzer.js`** imports `createAudioMetersUI` from `audioThresholds.js`,
  `createBandEQUI` and `drawScope` from `audioHistogram.js`, plus `assetManager.js` and
  `icons.js`, and calls them into its own area. The node imports its widgets; `snode.js` has
  never heard of any of them.
- **`nodes/video/video.js`** appends a drop zone, a controls fragment and a file selector.

Per-instance state falls out of the same shape. The node keeps `this.elements` and
`this.runtimeState`, and the constructor binds every function to the instance, so twenty
copies of a node have twenty sets of elements and no shared anything.

## The one thing we need that silvia does not

silvia never computes a node's height: the browser lays the node out and
`recalculateWorkspaceWidth` reads `offsetWidth` back afterwards.

We cannot. `canvas::node_height` runs for **every node in the graph** — not every node on
screen — several times a frame, before anything is drawn, because cables, ports, culling and
the strip's own bounds all need it first. Its comments already say layout must not reach into
the registry to answer it.

So our region contract has one function silvia's does not: **a region reports its size
without drawing.** Pure, cheap, and from `&Node` alone. That is a two-function module instead
of a one-function module, and it is the only concession. It is emphatically not a reason for
the base to know what kind of region it is — `Footer::of` already proves the split works,
since `footer_height` answers from the same place the drawing does.

## The shape

A node declares regions the way it already declares a CPU half. `CpuDef` is the precedent in
the tree: `cpu: Option<CpuDef>` with `create: fn() -> Box<dyn CpuNode>`, a declarative node
naming an imperative half, made per instance on first tick. Regions follow it:

```rust
pub struct RegionDef {
    /// How tall this region's own content is on a node of this shape, in world units, with
    /// its heading's height excluded. Pure and cheap: `node_height` calls it for every node
    /// in the graph, several times a frame, and never on a closed region.
    pub size: fn(&Node) -> f32,
    /// Draw into the rect, and say what the hand did in it.
    pub show: fn(&mut RegionUi<'_>) -> Vec<RegionEvent>,
    /// The heading over this region: a tiny label and a disclosure triangle, if a hand may
    /// close it. Key, label, and whether it is open on a node nobody has asked.
    pub heading: Option<Heading>,
    /// Whether the pointer inside this region's rect is the region's or the node's.
    pub claims_pointer: bool,
    /// The narrowest body this region can be drawn in.
    pub width: Option<f32>,
    /// The picture this region shows, where it shows one: an Output's render, or a texture
    /// the node publishes on a named port.
    pub picture: Option<Picture>,
}
```

and `NodeDef` gains `regions: &'static [RegionDef]` — a **list**, in draw order, because a
node can have a custom area *and* a scope *and* whatever else. Height is a
fold over the list. `Footer` and its ladder go. The list is stamped onto the `Node` by
`apply_defaults`, the way `width` already is, so layout still answers from the `Node` alone
and never reaches the registry.

**Interactive from the first cut.** `RegionEvent` carries the event half in this stage rather
than waiting for the pad, because the pad is what the contract is being designed against: a
mechanism that could only draw would have its event half designed twice, once blind. A
read-only region returns an empty `Vec` and pays nothing for it — the trace does exactly that.

### The hit rect is the node's business, not one size

In the Slime Mold a drag between the options and the ticks carries the node, and should. A
node with an interactive drag, like the XY pad, is different and does not. It is not one size,
so the node decides.

So a region **declares** whether it claims the pointer, and `canvas` has no rule about it at
all. A region that does not claim it — a read-only trace, a thumb, the pad's own fallback —
lets a press through to the body under it, and a hand carries the node by that region exactly
as by the whitespace beside a port. A region that does claim it — an XY pad — owns every press
inside its rect and never starts a node drag.

The mechanism is the one egui already gives: the body's own ground is registered first, and a
claiming region asks for the pointer after it, so the drag goes to the region and `ground` sees
nothing. The claiming region is handed the `Response` to read its drag from. Fine-grained
interacts *inside* a non-claiming region still work the same way — the scope's band handles are
registered after the ground too — which is why a scope can be dragged by its handles and
carried by everything else at once.

### Hiding is a region declaring its own heading

A node that can hide part of itself declares **two** things today that have to agree:
`has_scope: true` on the definition, and a `SHOW_SCOPE` option in its option list. So every
read site is the pair — `node.has_scope && shown(node, "scope")`, at `canvas.rs:485` and
again at `canvas.rs:541`. A region that carries its own visibility cannot disagree with
itself.

Then hiding needs no mechanism at all beyond `size`. A closed region returns its heading's
height and nothing more, and the fold does the rest: no `shown` branch in layout, no
visibility concept anywhere above the region.

**A heading with a disclosure triangle, not a tick in a shared row.** This is the convention
in the property panes of every creative program — Blender's properties editor, After
Effects' effect controls, the Unity inspector — and the reason it is the convention is the
thing a tick row cannot do: **a closed region still says it is there.** Hide a scope today
and the node says nothing about having one; getting it back means knowing that a row of
ticks elsewhere on the node controls it. A closed heading is sixteen points that name the
thing and open it again.

That is a revision of *The scope: a two-axis handle, and a row of ticks for the rest* in
`docs/decisions.md`, and only of half of it. **The storage is unchanged and was the load-
bearing half**: a heading's state is an option in `Node::options`, `OptionKind::Presentation`,
document data, undoable, and read off the `Node` by `canvas` without consulting the registry
— exactly what that decision established and the reason the `show` select was replaced. What
changes is the affordance, from a checkbox in `Row::Checks` to a triangle on the region it
governs.

**The ticks row stays, as the port-visibility control and nothing else.** `uniforms` and
`events` hide *rows*, not regions: there is no heading for them to sit on, because the thing
they hide is a run of ports. `scope` leaves the row and becomes a heading on the region it was
always about, and so does `preview`. A node carries both idioms, and they are honestly about
two different things — `Row::Checks` for which ports are drawn, a heading for whether a region
is open. Rows stay rows, so the distinction is permanent rather than a stage on the way to
somewhere.

**Open by default, and the default belongs on the region.** `canvas::tick` documents that
today the polarity is the caller's on purpose — a tick that *hides* part of a node reads
absent as shown, because a node nobody has asked has all of itself; a tick that *adds* a
hundred and seventy points of picture reads absent as off, because a node nobody has asked
did not ask for one. On a heading the same fact is one field, per region, and `shown` and
`shows_preview` collapse into one rule.

**A region may have no heading at all**, which is `Option`. An Output's render is the node,
and a triangle that closes it turns an Output into a header with ports.

## What folds

All five. This is the test of whether the mechanism is real.

- **`has_scope`** → a scope region. `CpuNode::scope` already supplies the data.
- **`trace`** → a trace region over the same ring the tick pushes into.
- **`preview`** → a picture region naming its port.
- **`is_output`** → the render becomes a region like any other. The flag keeps only its
  compile meaning — this node terminates a shader and owns a renderer — and the node can
  register *that* itself, the way silvia's Output adds itself to `SNode.outputs`.
- **`width`** → the widest region asks for it, so an Output is 240 because its render says so
  and an audio node is 300 because its scope does. **Rows stay rows**, which settles the rest:
  `width` stays on `NodeDef` as the body's own property and is honest about being about rows —
  a short tested list of nodes whose longest label does not fit 200. A region asks for its
  width whether it is open or closed, so folding a scope away does not reflow the node under
  the hand that closed it.

**Not every region is closeable — the region chooses.** `heading: Option<Heading>` is what
keeps an Output's render from growing a triangle that turns an Output into a header with
ports. Every region closeable is one fewer field and a worse default.

**Rows do not become regions.** It would finish the job — the body would be nothing but a list
— and it is a far larger change than the five bits, since rows carry ports and ports carry the
whole cable system. The body is rows, then the ordered region list, and the two idioms for
hiding stay honestly separate.

## Order

The composition change and the module dispatch are two pieces of work and doing them together
means a snapshot diff cannot tell you which half broke.

1. **`Footer` becomes an ordered list**, with today's exact region kinds still hardcoded and
   no new mechanism. A pure refactor: the node snapshots are strong proof it changed nothing,
   and the one intended behavior change — an audio node keeping its scope *and* its preview —
   is a snapshot that moves on purpose.
2. **`RegionDef` and `src/widgets/`**, with the existing five ported onto it and the flags
   deleted. Still no new node.
3. **The pad — built.** `xypad`, the smallest node that needs nothing else: a region that
   claims the pointer, `widgets::xypad`.
4. **The grid — built.** `stepsequencer` edits one; **`euclideanrhythm` reads one**, because, looking at
   that node in the app, some things are better off with custom UI than with value inputs. It is twelve number rows — `Lane N Steps`,
   `Pulses` and `Rotation`, four times over — and what a hand is setting is a figure: where
   the pulses fall, and whether rotating moved them where it wanted. Twelve numbers cannot
   show that; sixteen cells show nothing else. Read-only is the cheap half — no editing, no
   option to write back, just the pattern the tick already computes, drawn where the numbers
   are. Whether the numbers stay beside it or collapse behind it is a question for then.
5. **The paint surface.** `drawingcanvas` is nothing but this and a texture; the automaton
   and the mold gain their missing halves. **`drawingcanvas` has landed** — `widgets::paint`,
   a surface that claims the pointer over a brush that does not; the two halves have not.

Done when: `grep -c 'is_output\|has_scope\|trace\|preview' src/ui/canvas.rs` is a small
number and none of the hits are about what kind of node it is; each new kind has a kittest
that drags on the body and reads the control or the option back; and `docs/ui.md` describes
one region list rather than a footer ladder.
