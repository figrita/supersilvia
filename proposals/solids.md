# Proposal: solids, a 3D workspace kind

**Status: proposed, not agreed.** Nothing here exists. When it is agreed, the decision moves
to `docs/decisions.md` as *Agreed, not built*, and this file goes; when it is built, the
behavior moves to `docs/nodes.md` and `docs/rendering.md`. It depends on a workspace having
a kind, which is built: [docs/architecture.md](../docs/architecture.md#workspaces).
[solids-demo.html](solids-demo.html) is a working sketch in HTML and JS: the same compiler
shape, one raymarched shader, and green nodes that would sit on any workspace.

## What this is for

A workspace kind where a graph describes a **solid** rather than a picture: primitives, booleans,
transforms and surface modifiers wired into an Output that raymarches the result. Signed
distance fields, which is how this is done in one fragment shader with no geometry. The
model is exactly silvia's and supersilvia's: **every node output is a GLSL function, every
cable is a function call, an Output terminates one shader.** The function takes a point
instead of a uv, and a solid is `vec4(distance, r, g, b)` so color rides through a boolean
the way geometry does.

Why it earns a workspace kind rather than a node category: a solid function has a different
signature from a field, `vec4 f(vec3 p)` against `vec4 f(vec2 uv)`, and a solid Output
renders differently, marching rays instead of evaluating once per pixel. A video workspace
cannot offer a `sphere` node because nothing on it can consume one. The kind is what silvia's
audio branch used to keep a synth's nodes off a video tab and vice versa (`workspaceType` on
the definition, the menu filtered by `WorkspaceManager.getActiveType()`), and it is what
[`WorkspaceKind`](../docs/architecture.md#workspaces) carries over.

## Which nodes are on which kind

silvia's audio branch answered this by hand: every node names its tab. This proposal
derives it, because the answer is already in the port types.

| port type | lives | on which kind |
| --- | --- | --- |
| `VaryingNumber`, `VaryingColor` | GPU, `f(vec2 uv)` | video |
| **`Solid`** | GPU, `vec4 f(vec3 p)` | solids |
| `UniformNumber`, `Action` | CPU | **any** |

A node belongs on a workspace kind when every port it has is meaningful there. `slew`,
`audioin`, `autogain`, a MIDI knob, an oscillator whose ports are uniform numbers: **all of
them sit on both kinds**, unchanged, because a uniform number is one `f32` per frame and a
GLSL uniform is a uniform in any shader. That is the shared-node story the audio branch had
for its analyzer and envelope follower, which published floats a video shader could read,
made a property of the type system instead of a per-node declaration.

Two bridges cross the boundary, and both already exist:

- **Down, by uniform number.** A tap on a video Output publishes a mean; a slew smooths it;
  a sphere's radius reads it. Nothing new: the sphere's `r` input takes a uniform number or
  a control like any number port.
- **Up, by texture.** A solid Output publishes its render as a texture, so its `frame` port
  is a `VaryingColor` and a video workspace samples it like a camera. A video Output's
  `frame` is a texture a solids workspace can sample too, once a node wants it: a `decal`
  that projects a picture onto a surface is a solid node with a `VaryingColor` texture
  input, and a texture input is legal anywhere because it is a sampler, not a field.

One consequence to say plainly: a node with a `VaryingNumber` input and a `Solid` output, a
displacement driven by a picture, is not expressible, because a field needs a uv and a solid
has a point. `displace` takes uniform numbers and computes its own noise. If a picture is
wanted on a surface it comes in as a texture with a projection the node chooses.

## The model

### A fifth port type

```rust
PortType::Solid     // vec4 f(vec3 p): distance and color
```

`feeds` admits `Solid → Solid` and nothing else. The compiler derives the signature from the
type as it does for `VaryingNumber` and `VaryingColor`, so a generator returns only the
body. The worldspace convention is silvia's lifted one axis: units where the viewport's
height is 2, y up, origin at the center, the camera looking down negative z by default.

### The generator contract, one axis up

`ctx.input(node, key, "p")` returns the expression to splice, and a transform passes a
different point upstream exactly as `transform.rs` passes `zoomedUV`:

```rust
// twist
let k = ctx.input(node, "k", "p");
let a = ctx.input(node, "a", "q");
format!("vec3 q = twistP(p, {k} * 6.2831853); return {a};")
```

The demo's compiler is that contract in forty lines: recursive descent from the Output,
functions emitted callee-before-caller, emitted before recursing so a diamond emits once,
unconnected controls as uniforms, an unconnected solid as a far-away gray ghost the way an
unconnected color is the hue wheel. A cycle is refused at connect time by the same
ancestor check the video graph uses.

`time` is an input port with `Control::Global("u_time")`, so `displace` and an oscillator
animate by default and follow a cable when given one. A solids workspace under a timeline is
no different from a video one.

### The Output

`solid_output` is an Output: it owns a render target, its `frame` port publishes a texture,
and it terminates one shader. The shader is the raymarcher. Its options are the ones that
change GLSL, shading mode among them; its controls are the ones that are uniforms, the
camera and the floor. The camera is three controls, so a MIDI knob orbits it and a lane can
fly it, which is the point of having made it controls.

It renders into the same per-Output FBO chain, gets the same frame history, and can be
placed on a timeline as a clip like any Output. The compositor does not know it is 3D.

Cost is the one thing different about it. A field is evaluated once per pixel; a solid is
evaluated once per march step, up to 150 times per pixel, plus normals, shadows and
occlusion. The demo's stats line shows why the resolution button exists. The render target
gets a **scale** option, and the debug overlay shows the march count the way it shows
`worst_dt`.

### What is not new

Nothing in `graph/`, `ui/`, `app/`, `workspace.rs`, `project.rs` or the command bus knows
what a solid is. A `Solid` port is a port; a solids workspace is a `WorkspaceKind` whose menu
is `REGISTRY` filtered by "every port meaningful here"; a solid Output is an Output.
Controls, undo, save, export, the tag, lanes, and MIDI all work on a solids workspace the day
it exists, because none of them ever looked at a port type. `WorkspaceKind::is_portable` is
the one place that has to answer for it, and a solids workspace is nodes and cables, so it
travels whole.

## The library

Ported from the demo, which has them all working:

| | |
| --- | --- |
| `Primitive` | sphere, box, torus, cylinder, capsule, cone, octahedron, plane |
| `Boolean` | union, subtract, intersect, each with a smoothing radius |
| `Transform` | translate, rotate, scale, mirror, repeat, twist, bend |
| `Surface` | round, shell, paint, displace |
| `Output` | solid output |

Every primitive carries its own position and color, so a simple scene needs no transform
nodes; transforms exist for what is wired below them. Booleans blend color with the
distance, so a smooth union of an orange sphere and a blue box has an orange-to-blue seam
for free.

Not in the first cut, and worth listing so nobody rediscovers them: `decal` (texture in,
projected), `extrude` and `revolve` (a 2D `VaryingNumber` field as a profile is the one
place a field could legally enter, since a profile has a uv), `noise` as a surface, and a
`material` richer than a color.

## What changes elsewhere

| where | change |
| --- | --- |
| `graph/port.rs` | `PortType::Solid`; `feeds` |
| `compile/` | signature for `Solid`; a second prelude, the SDF helpers, included when any solid function is emitted |
| `nodes/` | the library above; `NodeDef::kinds()` derived from port types |
| `render/` | a raymarching Output program: same FBO chain, a resolution scale, the march count in the overlay |
| `ui/` | a `Solid` port color from the theme's anchors; the library filtered by the active workspace's kind |
| `docs/nodes.md`, `docs/rendering.md` | the behavior, when built |

## Tests

- **Layer 1.** GLSL snapshots for every solid node, connected and unconnected;
  `every_node_compiles_on_the_gpu` extends to the solids prelude. A registry test that a
  node's kinds follow from its ports and that a node whose ports are all uniform numbers is
  on every kind.
- **Layer 2.** A solids workspace's menu offers `sphere` and `slew` and not `checkerboard`;
  a video workspace's offers `checkerboard` and `slew` and not `sphere`.
- **Layer 3.** Headless GL: a sphere at the origin renders a disc; the pixels are read back
  and the disc's radius is what the control says.

## Order of work

1. `PortType::Solid`, the signature, the prelude, `sphere` and `solid_output` with a fixed
   camera. A sphere on screen.
2. The library.
3. The kind filter on the Nodes menu and the browser, over the `WorkspaceKind` workspaces
   already carry.
4. Camera as controls; resolution scale; the march count in the overlay.
5. `frame` on the solid Output sampled from a video workspace. The bridge, proved.

Step 1 touches four files and needs no kind: a solids graph with a video Output beside it is
a legal graph, since nothing stops a `sphere` being added to a video workspace. The kind is
for the menu, not the compiler.

## Open questions

- **Does the kind filter the menu or the canvas?** A `sphere` on a video workspace can
  connect to nothing there, and silvia's rule is that an illegal thing is unofferable.
  Filtering the menu is enough; filtering the canvas would make a uniform number node's two
  homes awkward.
- **Where does a `VaryingNumber` field enter a solid, if ever?** Extrude and revolve are
  the honest door: a profile has a uv. Anything else is a projection the node has to choose.
- **One prelude or two?** The SDF helpers are twenty lines; the question is whether a video
  shader should carry them when nothing uses them.
