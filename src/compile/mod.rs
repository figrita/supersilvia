// SPDX-License-Identifier: AGPL-3.0-or-later

//! Turning a graph into one WGSL module per Output node. Pure string generation: no GPU, no
//! egui.
//!
//! Every node output carries a WGSL generator (`OutputDef::wgsl`); the walk, the uniforms,
//! the tap slots and the diagnostics are here, and the module around the functions the walk
//! emits — the prelude, the bindings and `fs_main` — is [`wgsl`]'s. See
//! `proposals/shader-path.md`.
//!
//! A graph is an expression, not a render graph. Every node output is a function, every
//! connection is a call, and [`wgsl::build`] is one recursive descent from the Output's input
//! that prints the whole thing. The four collections on `CompileContext` fill themselves in as a
//! side effect of that walk.
//!
//! A workspace's **pass**, [`wgsl::build_pass`], is the one module that is nobody's picture:
//! every measured node's measurement, and every varying output's thumbnail, each pulling its
//! own chain in. An Output's module measures nothing.

pub mod uniform;
pub mod wgsl;

pub use uniform::{UniformProvider, UniformType};

use crate::graph::{Graph, Node, NodeId, PortRef, PortType};
use crate::nodes::Control;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// Something the compiler could not resolve.
///
/// Every one of these means an invariant broke — a node or port that the graph said existed
/// did not. The shader is still produced, and the expression spliced in is the
/// same one an unconnected input gets, because a performance must not stop for a bug in us.
/// The difference is that it is no longer silent: the status line can say it and a test can
/// assert there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Diagnostic {
    NoSuchNode(NodeId),
    NoSuchInput {
        node: NodeId,
        key: &'static str,
    },
    NoSuchOutput {
        node: NodeId,
        key: &'static str,
    },
    /// A node reached the compiler without a generator for an output. The function is a
    /// placeholder of the right type — magenta, or zero — so the module still validates.
    Untranslated {
        node: NodeId,
        slug: &'static str,
    },
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchNode(id) => write!(f, "no node {id}"),
            Self::NoSuchInput { node, key } => write!(f, "node {node}: no input {key:?}"),
            Self::NoSuchOutput { node, key } => write!(f, "node {node}: no output {key:?}"),
            Self::Untranslated { node, slug } => write!(f, "node {node} ({slug}): no WGSL"),
        }
    }
}

/// A finished shader and the uniforms the renderer has to feed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shader {
    /// The whole module: both entry points, the bindings and every function.
    pub body: String,
    /// Uniform name to provider, sorted by name so the output is deterministic.
    ///
    /// Names are `Arc<str>` so the per-frame walk clones a refcount rather than a string.
    pub uniforms: BTreeMap<std::sync::Arc<str>, UniformProvider>,
    /// Invariants that broke while compiling. Empty on every well-formed graph.
    pub diagnostics: Vec<Diagnostic>,
    /// Which node owns each tap slot, in slot order. Empty for a shader with no side
    /// effects, which is then also a shader with no storage buffer.
    ///
    /// In a probe every emitted node has one, and its [`EVAL_WORD`] counts how many times
    /// that node's function ran.
    pub taps: Vec<(NodeId, TapKind)>,
    /// A workspace pass's ports, one thumbnail each, in tile order: see [`wgsl::build_pass`].
    /// Empty in an Output's module and a probe.
    pub thumbs: Vec<(PortRef, PortType)>,
    /// The side of the square from the target's corner a pass's measurements run in: the
    /// largest tap's grid, one for a `sample`, zero with none.
    pub grid: u32,
}

/// How far away "nothing" is in a solid's distance: an unconnected solid input, and the floor
/// of a scene with no floor. Far past any march's reach, and still exact in an `f32`.
pub const SOLID_FAR: &str = "1.0e4";

/// The point inside a solid a field is being read at, for a node that casts a field onto a
/// solid by a projection of its own (`nodes::solids::cast::CAST`): `.xyz` the point, `.w` 1
/// while a solid is asking and 0 in a picture. Set round each read by
/// [`CompileContext::input_at_point`], which reads the field itself at the point's front
/// view, `p.xy`.
pub const POINT_STATE: &str = "// The point a solid is reading a field at; w is 0 in a picture.
var<private> sd_at: vec4f;
";

/// A port thumbnail's grid: 16:9, square cells, over the frame the worldspace convention
/// fixes — `x` from `-16/9` to `16/9`, `y` from `-1` to `1`.
pub const THUMB_W: usize = 48;
pub const THUMB_H: usize = 27;
/// One thumbnail's words: a packed color or a float's bits per cell.
pub const THUMB_CELLS: usize = THUMB_W * THUMB_H;
/// How many thumbnails a workspace pass lays side by side before starting a row.
pub const PASS_COLS: usize = 8;

/// The target a workspace pass draws, one fragment per cell: its measurements' square of
/// `grid` at the corner and `thumbs` tiles to its right.
pub fn pass_size(grid: u32, thumbs: usize) -> (u32, u32) {
    if thumbs == 0 {
        return (grid.max(1), grid.max(1));
    }
    let cols = thumbs.min(PASS_COLS);
    let rows = thumbs.div_ceil(cols);
    (
        grid + (cols * THUMB_W) as u32,
        grid.max((rows * THUMB_H) as u32),
    )
}

/// Whether a workspace pass draws thumbnails. On unless `SUPERSILVIA_THUMBS=0`, which is for
/// measuring what they cost; a pass measures its taps either way.
pub fn thumbs_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SUPERSILVIA_THUMBS").map_or(true, |v| v != "0"))
}

/// What a tap slot holds, in `u32` words. Every slot is this long whatever its kind, so the
/// renderer's buffer is `slots * TAP_WORDS` and a slot is found by multiplying.
///
/// A `Stats` slot spends them on a count, four sums of two words each — the measured
/// quantity, the two coordinate sums and the weight — the two extremes, and four one-word
/// sums for the mean color, its three premultiplied channels and its alpha; the layout is in
/// `nodes::tap`. A `Sample` slot uses the first five.
pub const TAP_WORDS: usize = 16;

/// The buffer's initial contents for one slot. Sums and the maximum's key start at zero; the
/// minimum's key starts at the largest so the first fragment wins.
pub const TAP_TEMPLATE: [u32; TAP_WORDS] = {
    let mut t = [0u32; TAP_WORDS];
    t[4] = u32::MAX;
    t
};

/// What kind of measurement a slot receives, so the app knows how to decode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapKind {
    /// Atomic sums and extrema over the points a measurement was called at.
    Stats,
    /// One point's value, written by the one call that measured it.
    Sample,
    /// How many times the node's function ran, counted by a probe shader. A probe emits no
    /// measurement, so every slot in one is this or [`TapKind::Taps`]; the count lives in a
    /// word no kind of measurement uses.
    Evaluations,
    /// How many times the node called the function feeding this input: one call site,
    /// counted by a probe. Divided by the node's own evaluations, it is the node's taps.
    Taps(&'static str),
}

/// The word of a slot a probe counts evaluations into. A `Stats` slot uses the first
/// fifteen and a `Sample` slot the first five, so the last is free in both.
pub const EVAL_WORD: usize = TAP_WORDS - 1;

/// A source's hash: of the module the compiler wrote. What the renderer keeps a program under, and what a job names the source of its uniforms and
/// tap slots by.
pub fn source_hash(body: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    body.hash(&mut hasher);
    hasher.finish()
}

impl Shader {
    /// The tap buffer's size in slots: the taps, then the thumbnails' words rounded up to
    /// whole slots. The renderer resets every slot from the template, and every thumbnail
    /// word is written over each frame, so the template's words there are never read.
    pub fn slots(&self) -> usize {
        self.taps.len() + (self.thumbs.len() * THUMB_CELLS).div_ceil(TAP_WORDS)
    }

    /// The word the first thumbnail starts at.
    pub fn thumb_base(&self) -> usize {
        self.taps.len() * TAP_WORDS
    }

    /// The target a workspace pass draws; nothing else reads it.
    pub fn pass_size(&self) -> (u32, u32) {
        pass_size(self.grid, self.thumbs.len())
    }

    /// The part of a pass's target its measurements run in, from the corner: all a pass
    /// draws while nobody is looking at its thumbnails.
    pub fn tap_region(&self) -> (u32, u32) {
        (self.grid.max(1), self.grid.max(1))
    }

    /// The complete source.
    pub fn source(&self) -> String {
        self.body.clone()
    }
}

pub struct CompileContext<'a> {
    /// Count every node function's evaluations into its tap slot. The probe shader.
    probe: bool,
    /// The largest square a measurement runs over: see [`Shader::grid`].
    grid: u32,
    graph: &'a Graph,
    /// Emitted functions, callees before callers. WGSL needs no forward declarations, and the
    /// order keeps a module readable top to bottom.
    functions: Vec<String>,
    emitted: HashSet<String>,
    /// Nodes whose `measure` has run. A measurement belongs to the node, so a node with two
    /// emitted outputs still measures once.
    measured: HashSet<NodeId>,
    /// The guarded calls `main` makes, one per measurement, in the order they were emitted.
    measurements: Vec<String>,
    utils: Vec<&'static str>,
    utils_seen: HashSet<&'static str>,
    uniforms: BTreeMap<String, UniformProvider>,
    diagnostics: Vec<Diagnostic>,
    taps: Vec<(NodeId, TapKind)>,
    /// A workspace pass's ports, their types and their functions' names, in tile order.
    thumbs: Vec<(PortRef, PortType, String)>,
    /// The type of the function whose generator is running, for [`CompileContext::untranslated`].
    returning: PortType,
    /// Which of a solid's two functions is being emitted: the whole of it, distance and
    /// color, or the distance alone. See [`CompileContext::solid_input`].
    solid_color: bool,
    /// Each solid node's bounds, once worked out: the call that gives them, or `None` where the
    /// node cannot say. See [`CompileContext::solid_bounds`].
    bounds: HashMap<NodeId, Option<String>>,
}

impl<'a> CompileContext<'a> {
    /// For unit tests in `nodes/` that need to run a generator without a whole compile.
    #[cfg(test)]
    pub fn new_for_test(graph: &'a Graph) -> Self {
        Self::new(graph)
    }

    fn new(graph: &'a Graph) -> Self {
        Self {
            graph,
            probe: false,
            grid: 0,
            functions: Vec::new(),
            emitted: HashSet::new(),
            measured: HashSet::new(),
            measurements: Vec::new(),
            diagnostics: Vec::new(),
            utils: Vec::new(),
            utils_seen: HashSet::new(),
            uniforms: BTreeMap::new(),
            taps: Vec::new(),
            thumbs: Vec::new(),
            returning: PortType::VaryingColor,
            solid_color: true,
            bounds: HashMap::new(),
        }
    }

    /// Claim a slot in this shader's tap buffer for `id`, and return its index.
    ///
    /// A measurement runs in one program, its workspace's pass, so a node has one slot. The
    /// word offset is `index * TAP_WORDS`.
    pub fn tap_slot(&mut self, id: NodeId, kind: TapKind) -> usize {
        if let Some(i) = self
            .taps
            .iter()
            .position(|(n, k)| *n == id && !matches!(k, TapKind::Taps(_)))
        {
            return i;
        }
        self.taps.push((id, kind));
        self.taps.len() - 1
    }

    /// Emit `id`'s measurement and the call `main` makes to it at every cell of an `N`x`N`
    /// grid over the unit square.
    ///
    /// The point handed to the body is the cell's center, `-1` to `1` on both axes, so what
    /// is measured is the node's input over the frame the worldspace convention fixes rather
    /// than over whatever coordinates a consumer asked at. `jitter` is an `int` expression
    /// where the node offers the choice: while it reads 1 the point moves by a hash of the
    /// cell and the clock, inside its own cell.
    ///
    /// The body is emitted as `fn {slug}{id}_measure(p: vec2f)` after whatever `ctx.input(id, key, "p")` calls inside it emitted, so a callee still
    /// precedes its caller. The caller claims its slot with [`CompileContext::tap_slot`]
    /// first, since the body is written against the slot's base.
    pub fn measure_grid(&mut self, id: NodeId, grid: u32, jitter: Option<&str>, body: &str) {
        let Some(name) = self.measure_name(id) else {
            return;
        };
        let center = format!("((vec2f(cell) + 0.5) / f32({grid})) * 2.0 - 1.0");
        let point = match jitter {
            Some(jitter) => format!(
                "{center} + select(vec2f(0.0), \
                 (hash2(vec2f(cell) + fract(u.u_time) * 7.0) - 0.5) * 2.0 / f32({grid}), \
                 ({jitter}) == 1)"
            ),
            None => center,
        };
        self.grid = self.grid.max(grid);
        self.functions
            .push(format!("fn {name}(p: vec2f) {{\n{body}\n}}"));
        self.measurements.push(format!(
            "    if (cell.x < {grid} && cell.y < {grid}) {{ {name}({point}); }}"
        ));
    }

    /// The same for a measurement of one point, which the body names itself: emitted as
    /// `fn {slug}{id}_measure()` and called by the one fragment at the corner.
    pub fn measure_once(&mut self, id: NodeId, body: &str) {
        let Some(name) = self.measure_name(id) else {
            return;
        };
        self.grid = self.grid.max(1);
        self.functions.push(format!("fn {name}() {{\n{body}\n}}"));
        self.measurements
            .push(format!("    if (all(cell == vec2i(0))) {{ {name}(); }}"));
    }

    /// Run `id`'s measurement, once per module: its slot, its chain and the guarded call
    /// `fs_main` makes. Only a workspace's pass asks, for each measured node it holds; an
    /// Output's module and a probe measure nothing.
    fn measure_node(&mut self, id: NodeId) {
        if !self.measured.insert(id) {
            return;
        }
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return;
        };
        if let Some(measure) = node.def.measure_wgsl {
            measure(id, self);
        }
    }

    /// The name of one node's measurement function.
    fn measure_name(&mut self, id: NodeId) -> Option<String> {
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return None;
        };
        Some(format!("{}{id}_measure", node.def.slug))
    }

    /// A probe's slot for one call site: `id` calling whatever feeds its input `key`.
    fn call_slot(&mut self, id: NodeId, key: &'static str) -> usize {
        let kind = TapKind::Taps(key);
        if let Some(i) = self.taps.iter().position(|(n, k)| *n == id && *k == kind) {
            return i;
        }
        self.taps.push((id, kind));
        self.taps.len() - 1
    }

    /// The name of one output port's function.
    fn func_name(id: NodeId, node: &Node, port: &str) -> String {
        format!("{}{id}_{}", node.def.slug, port)
    }

    /// The node a generator is currently being run for.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.graph.get(id)
    }

    /// How a uniform the compiler declared is read: a member of the one uniform struct `u`.
    fn uniform_ref(name: &str) -> String {
        format!("u.{name}")
    }

    /// The expression to splice in for one of `id`'s inputs, sampled at `uv`.
    ///
    /// Four cases, and they are the whole of the compiler's cleverness: a connected port is
    /// a call; an unconnected port with a control is a uniform; an unconnected port bound to
    /// a prelude global is that global; anything else is a type-appropriate fallback.
    pub fn input(&mut self, id: NodeId, key: &'static str, uv: &str) -> String {
        // A Time is a count, `vec2f(whole, fraction)`: cabled, whatever arrives, split by the
        // synth; unplugged, what the synth publishes under the input's own key — the ambient
        // reading, or a free-running node's own playhead (`nodes::timing`).
        if crate::nodes::is_time(key)
            && let Some(node) = self.graph.get(id)
            && node.def.timing.is_some()
        {
            return match self.graph.source_of(PortRef::new(id, key)) {
                Some(src) => self.published_count(src.node, src.key),
                None => self.published_count(id, key),
            };
        }
        if let Some(src) = self.graph.source_of(PortRef::new(id, key)) {
            // A uniform number never becomes a function. It is one number per frame, so it
            // takes the same path an unconnected number control does: a uniform, and no
            // descent into the producer, which is a CPU node with no function to emit.
            let is_uniform = self
                .graph
                .get(src.node)
                .and_then(|n| n.output(src.key))
                .is_some_and(|p| p.ty.is_uniform());
            if is_uniform {
                let value = self.published_uniform(src.node, src.key);
                // An angle read round a whole turn takes the count's fraction, so a gear's
                // Cycles a whole number of turns on draws to the bit what it drew at zero.
                let turns = self
                    .graph
                    .get(id)
                    .and_then(|n| n.def.input(key))
                    .is_some_and(crate::nodes::InputDef::turns);
                return if turns {
                    format!("fract({value})")
                } else {
                    value
                };
            }
            let Some((func, ret)) = self.emit(src) else {
                return Self::fallback(PortType::VaryingColor, uv);
            };
            if !self.probe {
                return format!("{func}({uv})");
            }
            // The probe counts the call site too, so a consumer's taps — how many times
            // it runs its input per run of its own — are its calls over its evaluations.
            let slot = self.call_slot(id, key);
            let base = slot * TAP_WORDS;
            // WGSL has no sequence operator, so the call site calls a wrapper that counts and
            // then calls through: one per call site, named by its slot.
            let via = format!("{func}_via{slot}");
            if self.emitted.insert(via.clone()) {
                let param = Self::param(ret);
                let name = param.split(':').next().unwrap_or("uv");
                self.functions.push(format!(
                    "fn {via}({param}) -> {} {{\n    atomicAdd(&tap[{base} + {EVAL_WORD}], 1u);\n    return {func}({name});\n}}",
                    Self::wgsl_type(ret)
                ));
            }
            return format!("{via}({uv})");
        }

        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return Self::fallback(PortType::VaryingColor, uv);
        };
        let (def, slug) = (node.def, node.def.slug);
        let Some(input) = def.input(key) else {
            self.diagnostics
                .push(Diagnostic::NoSuchInput { node: id, key });
            return Self::fallback(PortType::VaryingColor, uv);
        };

        match &input.control {
            Control::Number { .. } => {
                let name = format!("u_control_{slug}{id}_{key}");
                let reference = Self::uniform_ref(&name);
                self.uniforms.insert(
                    name,
                    UniformProvider::Control {
                        node: id,
                        key,
                        ty: UniformType::Float,
                    },
                );
                reference
            }
            Control::Color { .. } => {
                let name = format!("u_control_{slug}{id}_{key}");
                let reference = Self::uniform_ref(&name);
                self.uniforms.insert(
                    name,
                    UniformProvider::Control {
                        node: id,
                        key,
                        ty: UniformType::Vec4,
                    },
                );
                reference
            }
            // An action input never reaches here from a working graph: the compiler walks
            // data ports, and an event is a CPU thing that never becomes a uniform. If one
            // is asked for anyway, it falls back like an unconnected port rather than
            // emitting a name nothing declares.
            Control::None | Control::Press => Self::fallback(input.ty, uv),
        }
    }

    /// The expression for one of `id`'s `Solid` inputs at the point `p`: the whole solid —
    /// its distance and its color — where `color`, and its distance alone where not.
    ///
    /// **A solid compiles to two functions.** Marching a ray, finding a normal, a shadow
    /// and an occlusion call a solid dozens to hundreds of times a pixel and read only its
    /// distance; its color is wanted once, where the ray landed. So every solid node's
    /// generator runs twice, once per variant, and the variant without color is named with
    /// a `_d` suffix. A generator asks [`CompileContext::solid_color`] which it is writing
    /// and leaves out whatever only colors — a texture's samples above all. The first
    /// component means the same in both; in the distance-only one the other three are
    /// whatever was cheapest.
    pub fn solid_input(&mut self, id: NodeId, key: &'static str, p: &str, color: bool) -> String {
        let was = std::mem::replace(&mut self.solid_color, color);
        let expr = self.input(id, key, p);
        self.solid_color = was;
        expr
    }

    /// Whether the solid function being emitted is the whole one, distance and color, or the
    /// distance alone. See [`CompileContext::solid_input`].
    pub fn solid_color(&self) -> bool {
        self.solid_color
    }

    /// One of `id`'s number inputs read at the point `p` inside a solid.
    ///
    /// A uniform or a control is what [`CompileContext::input`] makes of it. **A field is read
    /// at the point's front view**, `p.xy`, through a wrapper of its own that says which point
    /// it is being read at round the call (`sd_at`), so a node that casts it by another
    /// projection can find the point, and puts back whatever point was there before: a field
    /// may hold a render of its own, whose solids ask too.
    pub fn input_at_point(&mut self, id: NodeId, key: &'static str, p: &str) -> String {
        let field = self
            .graph
            .source_of(PortRef::new(id, key))
            .and_then(|src| self.graph.get(src.node)?.output(src.key))
            .map(|o| o.ty)
            .filter(|ty| ty.is_varying() && !ty.is_solid());
        let Some(ty) = field else {
            return self.input(id, key, p);
        };
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return Self::fallback(ty, p);
        };
        let name = format!("{}{id}_{key}_at", node.def.slug);
        if self.emitted.insert(name.clone()) {
            if self.utils_seen.insert(POINT_STATE) {
                self.utils.push(POINT_STATE);
            }
            let read = self.input(id, key, "q.xy");
            self.functions.push(format!(
                "fn {name}(q: vec3f) -> {} {{
    let was = sd_at;
    sd_at = vec4f(q, 1.0);
    let v = {read};
    sd_at = was;
    return v;
}}",
                Self::wgsl_type(ty)
            ));
        }
        format!("{name}({p})")
    }

    /// Is anything plugged into one of `id`'s inputs?
    ///
    /// For a generator whose emitted code turns on whether an input is driven rather than on
    /// what it carries — a `tap` measuring a connected field instead of the quantity its
    /// picker names. `Connect` and `Disconnect` both rebuild, so the answer cannot go stale.
    pub fn connected(&self, id: NodeId, key: &'static str) -> bool {
        self.graph.source_of(PortRef::new(id, key)).is_some()
    }

    /// `src` with every `{key}` hole naming one of `id`'s number inputs filled with its value
    /// as one number a frame: its knob, or the number cabled in. `None` where a field is cabled
    /// into a hole, since a field has no one value: what reads this — a solid's bounds — then
    /// knows nothing.
    pub fn uniform_fill(&mut self, id: NodeId, src: &str) -> Option<String> {
        let def = self.graph.get(id)?.def;
        let mut out = src.to_string();
        for input in def.inputs {
            let hole = ["{", input.key, "}"].concat();
            if !out.contains(&hole) {
                continue;
            }
            let field = self
                .graph
                .source_of(PortRef::new(id, input.key))
                .and_then(|s| self.graph.get(s.node)?.output(s.key))
                .is_some_and(|p| p.ty.is_varying() || p.ty.is_list());
            if field {
                return None;
            }
            let expr = self.input(id, input.key, "p");
            out = out.replace(&hole, &expr);
        }
        Some(out)
    }

    /// The bounds of the solid cabled into one of `id`'s solid inputs, as a WGSL `SdBounds`
    /// expression: a box that holds it, in the input's own space, worked out each frame from
    /// the same uniforms its distance reads, so a knob turned or an oscillator cabled in keeps
    /// it true. Empty where nothing is cabled in, and `None` where the solid cannot say — its
    /// node declares no `NodeDef::bounds`, or a field reaches a number its size depends on.
    /// Each node's are worked out once a module, in a function of their own.
    pub fn solid_bounds(&mut self, id: NodeId, key: &'static str) -> Option<String> {
        let Some(src) = self.graph.source_of(PortRef::new(id, key)) else {
            return Some("SdBounds(vec3f(1.0e9), vec3f(-1.0e9))".to_string());
        };
        if let Some(known) = self.bounds.get(&src.node) {
            return known.clone();
        }
        let node = self.graph.get(src.node)?;
        let (slug, bounds) = (node.def.slug, node.def.bounds);
        // Marked unknown first, so a loop in the graph cannot recurse for ever.
        self.bounds.insert(src.node, None);
        let answer = bounds.and_then(|f| f(src.node, self)).map(|expr| {
            let name = format!("sdb_{slug}{}", src.node);
            self.functions.push(format!(
                "fn {name}() -> SdBounds {{\n    return {expr};\n}}"
            ));
            format!("{name}()")
        });
        self.bounds.insert(src.node, answer.clone());
        answer
    }

    /// The function measuring the solid cabled into one of `id`'s solid inputs at a size
    /// scaled on each axis, `name(ps, sa)` with `ps` the point in the scaled space: a shape
    /// that declares `NodeDef::scaled` and lands on nothing, its body scaled where that is
    /// exact and its ordinary distance bounded by the smallest scale elsewhere. `None` for any
    /// other solid, which the caller measures by the bound itself.
    pub fn solid_scaled(&mut self, id: NodeId, key: &'static str) -> Option<String> {
        let src = self.graph.source_of(PortRef::new(id, key))?;
        let scaled = self.graph.get(src.node)?.def.scaled?;
        if self.connected(src.node, crate::nodes::solids::ONTO.key) {
            return None;
        }
        let (func, _) = self.emit(src)?;
        let name = format!("{func}_scaled");
        if self.emitted.insert(name.clone()) {
            let own = format!("{name}_own");
            let written = (scaled.wgsl)(src.node, self);
            let mut body = crate::nodes::solids::fill(src.node, self, &written);
            if self.probe {
                let base = self.tap_slot(src.node, TapKind::Evaluations) * TAP_WORDS;
                body = format!("    atomicAdd(&tap[{base} + {EVAL_WORD}], 1u);\n{body}");
            }
            self.functions.push(format!(
                "fn {own}(p: vec3f, q: vec3f, sc: vec3f) -> vec4f {{\n{body}\n}}"
            ));
            let wrap =
                crate::nodes::solids::scaled_chain(src.node, self, &func, &own, scaled.exact);
            self.functions.push(format!(
                "fn {name}(ps: vec3f, sa: vec3f) -> vec4f {{\n{wrap}\n}}"
            ));
        }
        Some(name)
    }

    /// The current value of one of `id`'s select options.
    ///
    /// An `OptionKind::Code` option changes generated code, which is why `SetOption` rebuilds
    /// the shader for one while `SetControl` never does. An option the shader should branch
    /// on at run time instead goes through `option_uniform`.
    pub fn option(&self, id: NodeId, key: &str) -> &str {
        self.graph
            .get(id)
            .and_then(|n| n.options.get(key))
            .map_or("", |v| v.as_str())
    }

    /// Register and name the `int` uniform carrying which of an option's choices is chosen.
    ///
    /// The counterpart to `option`: that one reads the value at compile time and bakes the
    /// branch, this one leaves every branch in the program and says at run time which runs.
    /// `SetOption` rebuilds for the first and not for the second, which is the whole point.
    pub fn option_uniform(&mut self, id: NodeId, key: &'static str) -> String {
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return "0".to_string();
        };
        let name = format!("u_opt_{}{id}_{key}", node.def.slug);
        let reference = Self::uniform_ref(&name);
        self.uniforms
            .insert(name, UniformProvider::Option { node: id, key });
        reference
    }

    /// Register and name the texture uniform for one of `id`'s texture outputs.
    ///
    /// A `texture_2d<f32>` binding, sampled with the sampler [`CompileContext::sampler`] names
    /// for the same port.
    pub fn texture_uniform(&mut self, id: NodeId, port: &'static str) -> String {
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return String::new();
        };
        let name = format!("u_texture_{}{id}_{}", node.def.slug, port);
        self.uniforms.insert(
            name.clone(),
            UniformProvider::NodeTexture { node: id, port },
        );
        name
    }

    /// The sampler one of `id`'s texture outputs is read through: one of the module's
    /// four shared samplers, chosen by the port's own `wrap` and `filter`.
    pub fn sampler(&mut self, id: NodeId, port: &'static str) -> String {
        let Some(out) = self.graph.get(id).and_then(|n| n.def.output(port)) else {
            self.diagnostics.push(Diagnostic::NoSuchOutput {
                node: id,
                key: port,
            });
            return wgsl::Sampler::MirrorLinear.name().to_string();
        };
        wgsl::Sampler::of(out.wrap, out.filter).name().to_string()
    }

    /// The uniform carrying one of `id`'s **own** uniform number outputs, for a node whose
    /// WGSL reads what its `tick` computed — an exposure reading the gain it published.
    /// The same provider a connected uniform number gets, so no cable is needed to close the
    /// loop.
    pub fn own_uniform(&mut self, id: NodeId, port: &'static str) -> String {
        self.published_uniform(id, port)
    }

    /// Register and name the uniform carrying one of `id`'s uniform outputs.
    ///
    /// A number and a color take the same path and differ only in the two things the module
    /// needs to be told: the name — `u_float_` or `u_color_`, which is silvia's own pair — and the
    /// type the renderer uploads it as. The kind comes from the port rather than from a
    /// slug, so a node publishing both gets one of each.
    fn published_uniform(&mut self, id: NodeId, port: &'static str) -> String {
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return "0.0".to_string();
        };
        let color = node
            .output(port)
            .is_some_and(|p| p.ty.kind() == crate::graph::Kind::Color);
        let (prefix, ty) = if color {
            ("u_color_", UniformType::Vec4)
        } else {
            ("u_float_", UniformType::Float)
        };
        let name = format!("{prefix}{}{id}_{}", node.def.slug, port);
        let reference = Self::uniform_ref(&name);
        self.uniforms
            .insert(name, UniformProvider::NodeUniform { node: id, port, ty });
        reference
    }

    /// The number list cabled into one of `id`'s inputs, as the stem of two functions in the
    /// module: `{stem}_len()`, its length as an `f32`, and `{stem}_at(item, fallback)`, the item
    /// at an `f32` index held to the list, or `fallback` where the list is empty. `None` where
    /// no list is cabled in — a single number or a knob, which [`CompileContext::input`] reads.
    ///
    /// A number list also carries `{stem}_most()`, the largest of its items' magnitudes.
    /// A color list's `{stem}_at(item, fallback)` takes and gives a `vec4f`, premultiplied, one
    /// color an element, at most [`uniform::COLOR_LIST_CAP`] of them.
    ///
    /// The list rides in the uniform block whole ([`UniformProvider::NodeList`]), so a length
    /// change rebuilds nothing and an item is one indexed read.
    pub fn list_input(&mut self, id: NodeId, key: &'static str) -> Option<String> {
        let src = self.graph.source_of(PortRef::new(id, key))?;
        let node = self.graph.get(src.node)?;
        if !node.output(src.key).is_some_and(|p| p.ty.is_list()) {
            return None;
        }
        let stem = format!("list_{}{}_{}", node.def.slug, src.node, src.key);
        let name = format!("u_{stem}");
        let reference = Self::uniform_ref(&name);
        self.uniforms.insert(
            name,
            UniformProvider::NodeList {
                node: src.node,
                port: src.key,
            },
        );
        let color = node
            .output(src.key)
            .is_some_and(|p| p.ty == PortType::ListColor);
        if color && self.emitted.insert(stem.clone()) {
            self.functions.push(format!(
                "fn {stem}_len() -> f32 {{
    return {reference}[0].x;
}}

fn {stem}_at(item: f32, fallback: vec4f) -> vec4f {{
    let n = min(u32({reference}[0].x), {cap}u);
    if (n == 0u) {{
        return fallback;
    }}
    let k = min(u32(max(item, 0.0)), n - 1u);
    return {reference}[1u + k];
}}",
                cap = uniform::COLOR_LIST_CAP
            ));
        }
        if self.emitted.insert(stem.clone()) {
            self.functions.push(format!(
                "fn {stem}_len() -> f32 {{
    return {reference}[0].x;
}}

fn {stem}_most() -> f32 {{
    return {reference}[0].y;
}}

fn {stem}_at(item: f32, fallback: f32) -> f32 {{
    let n = u32({reference}[0].x);
    if (n == 0u) {{
        return fallback;
    }}
    let k = min(u32(max(item, 0.0)), n - 1u);
    return {reference}[1u + k / 4u][k % 4u];
}}"
            ));
        }
        Some(stem)
    }

    /// Register and name the uniform carrying a count one of `id`'s ports published, as a
    /// Time reads it: `vec2f(whole, fraction)`, the whole part wrapped at
    /// [`crate::nodes::phasor::WHOLE_WRAP`] and the `f32` of the fraction. A body that needs
    /// the count modulo a period reduces `.x` by it and adds `.y`; one that needs only the
    /// fraction reads `.y`.
    fn published_count(&mut self, id: NodeId, port: &'static str) -> String {
        let Some(node) = self.graph.get(id) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(id));
            return "vec2f(0.0)".to_string();
        };
        let name = format!("u_count_{}{id}_{}", node.def.slug, port);
        let reference = Self::uniform_ref(&name);
        self.uniforms
            .insert(name, UniformProvider::NodeCount { node: id, port });
        reference
    }

    /// An unconnected input with no control. A color falls back to the hue wheel, which is why
    /// an unplugged color input shows something rather than black.
    fn fallback(ty: PortType, uv: &str) -> String {
        match ty {
            PortType::VaryingColor => format!("defaultUvMap({uv})"),
            PortType::UniformColor => "vec4f(0.0)".to_string(),
            PortType::VaryingNumber | PortType::UniformNumber => "0.0".to_string(),
            // An event and a list never reach a shader.
            PortType::Action | PortType::ListNumber | PortType::ListColor => String::new(),
            // Nothing, a long way off: a union with it is the other side alone.
            PortType::Solid => format!("vec4f({SOLID_FAR}, 0.5, 0.5, 0.5)"),
        }
    }

    /// The body a node without a generator gets for one of its outputs: a constant of
    /// the function's type — magenta, or zero — and a [`Diagnostic::Untranslated`] saying so.
    /// What `nodes::no_wgsl` returns.
    pub fn untranslated(&mut self, id: NodeId) -> String {
        let slug = self.graph.get(id).map_or("", |n| n.def.slug);
        self.diagnostics
            .push(Diagnostic::Untranslated { node: id, slug });
        match self.returning {
            PortType::VaryingNumber => "    return 0.0;".to_string(),
            PortType::Solid => format!("    return vec4f({SOLID_FAR}, 1.0, 0.0, 1.0);"),
            _ => "    return vec4f(1.0, 0.0, 1.0, 1.0);".to_string(),
        }
    }

    /// The parameter a function of port type `ty` takes: a point for a solid, a pixel's `uv`
    /// for everything else.
    fn param(ty: PortType) -> &'static str {
        if ty.is_solid() {
            "p: vec3f"
        } else {
            "uv: vec2f"
        }
    }

    /// The WGSL type a function of port type `ty` returns.
    fn wgsl_type(ty: PortType) -> &'static str {
        match ty {
            PortType::VaryingNumber => "f32",
            _ => "vec4f",
        }
    }

    /// Emit the function for `src` if it has not been emitted, and return its name and its
    /// port type — `None`, with a diagnostic, where the graph named something that is not
    /// there.
    fn emit(&mut self, src: PortRef) -> Option<(String, PortType)> {
        let Some(node) = self.graph.get(src.node) else {
            self.diagnostics.push(Diagnostic::NoSuchNode(src.node));
            return None;
        };
        let mut func = Self::func_name(src.node, node, src.key);
        let def = node.def;
        let Some(out) = def.output(src.key) else {
            self.diagnostics.push(Diagnostic::NoSuchOutput {
                node: src.node,
                key: src.key,
            });
            return None;
        };
        if out.ty.is_solid() && !self.solid_color {
            func.push_str("_d");
        }

        if self.emitted.contains(&func) {
            return Some((func, out.ty));
        }

        for util in def.wgsl_utils {
            if self.utils_seen.insert(util) {
                self.utils.push(util);
            }
        }

        // Mark before recursing so a diamond in the graph emits the function once. A cycle
        // is impossible: graph/ refuses to create one.
        self.emitted.insert(func.clone());

        // A texture output samples a uniform rather than calling upstream, so its own
        // function still has to exist for consumers to call.
        self.returning = out.ty;
        let mut body = (out.wgsl)(src.node, self, &func);
        // The probe: one atomic per evaluation, into the slot's spare word. Slots are
        // claimed by node, so a tapping node counts into the slot it already has.
        if self.probe {
            let base = self.tap_slot(src.node, TapKind::Evaluations) * TAP_WORDS;
            body = format!("    atomicAdd(&tap[{base} + {EVAL_WORD}], 1u);\n{body}");
        }
        // A shape in the solids chain: the generator wrote its own shape, which becomes a
        // function of its own, and the chain's turn, scale and combination wrap it.
        if out.ty.is_solid() && crate::nodes::solids::is_shape(def) {
            let own = format!("{func}_own");
            self.functions
                .push(format!("fn {own}(p: vec3f) -> vec4f {{\n{body}\n}}"));
            body = crate::nodes::solids::chain(src.node, self, &own);
        }
        self.functions.push(format!(
            "fn {func}({}) -> {} {{\n{body}\n}}",
            Self::param(out.ty),
            Self::wgsl_type(out.ty)
        ));

        Some((func, out.ty))
    }
}

/// Does this Output have a shader at all — is the input the whole expression descends from
/// connected? The one thing [`wgsl::build`] answers `None` for, asked without building anything.
pub fn has_shader(graph: &Graph, output: NodeId) -> bool {
    root_of(graph, output).is_some()
}

/// The key of the input an Output's expression descends from — its first, by definition
/// order — and `None` unless something is connected to it.
fn root_of(graph: &Graph, output: NodeId) -> Option<&'static str> {
    let key = graph.get(output)?.def.inputs.first()?.key;
    graph.source_of(PortRef::new(output, key)).map(|_| key)
}

fn build_with(graph: &Graph, output: NodeId, probe: bool) -> Option<Shader> {
    let root_key = root_of(graph, output)?;

    let mut ctx = CompileContext::new(graph);
    ctx.probe = probe;
    let root = ctx.input(output, root_key, "uv");

    let body = wgsl::assemble(&ctx, &root);
    Some(Shader {
        diagnostics: ctx.diagnostics,
        thumbs: Vec::new(),
        grid: 0,
        taps: ctx.taps,
        uniforms: ctx
            .uniforms
            .into_iter()
            .map(|(name, provider)| (std::sync::Arc::from(&*name), provider))
            .collect(),
        body,
    })
}

/// The most textures one pass binds: WebGPU's floor for sampled textures in a stage, which
/// every adapter wgpu runs on offers. A workspace whose pass would bind more is drawn in
/// batches, each under it. See [`wgsl::build_pass`].
pub const PASS_TEXTURES: usize = 16;

/// One thing a pass does: run a node's measurement, or draw a port's thumbnail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Measure(NodeId),
    Thumb(PortRef),
}

/// Everything one workspace's pass does, in order: `measured`'s measurements, then — where
/// `thumbs` — every varying output of every node shown on the workspace, connected or not,
/// in id order.
fn pass_items(
    graph: &Graph,
    workspace: crate::graph::WorkspaceId,
    measured: &[NodeId],
    thumbs: bool,
) -> Vec<Item> {
    let mut items: Vec<Item> = measured.iter().map(|id| Item::Measure(*id)).collect();
    if !thumbs || !thumbs_enabled() {
        return items;
    }
    for (id, node) in graph.iter() {
        if !node.workspaces.contains(&workspace) || node.def.is_output {
            continue;
        }
        for port in &node.outputs {
            // A port with no WGSL would only be the placeholder's magenta.
            let translated = node.def.output(port.key).is_some_and(|o| {
                !std::ptr::fn_addr_eq(o.wgsl, crate::nodes::no_wgsl as crate::nodes::GenFn)
            });
            if port.ty.is_varying() && translated {
                items.push(Item::Thumb(PortRef::new(id, port.key)));
            }
        }
    }
    items
}

/// One module doing `items`, in order: the measurements' slots come first, so a thumbnail's
/// words start after the last. `None` where it does nothing.
fn build_items(graph: &Graph, items: &[Item]) -> Option<Shader> {
    let mut ctx = CompileContext::new(graph);
    for item in items {
        if let Item::Measure(id) = item {
            ctx.measure_node(*id);
        }
    }
    for item in items {
        if let Item::Thumb(src) = item
            && let Some((func, ty)) = ctx.emit(*src)
        {
            // A solid's thumbnail is a small render of it, which marches its distance alone.
            if ty.is_solid() {
                ctx.solid_color = false;
                let _ = ctx.emit(*src);
                ctx.solid_color = true;
            }
            ctx.thumbs.push((*src, ty, func));
        }
    }
    if ctx.thumbs.is_empty() && ctx.taps.is_empty() {
        return None;
    }
    let body = wgsl::assemble_pass(&ctx);
    Some(Shader {
        diagnostics: ctx.diagnostics,
        thumbs: ctx.thumbs.iter().map(|(p, t, _)| (*p, *t)).collect(),
        grid: ctx.grid,
        taps: ctx.taps,
        uniforms: ctx
            .uniforms
            .into_iter()
            .map(|(name, provider)| (std::sync::Arc::from(&*name), provider))
            .collect(),
        body,
    })
}

/// The textures a module binds, by uniform name.
fn textures_of(shader: &Shader) -> BTreeSet<std::sync::Arc<str>> {
    shader
        .uniforms
        .iter()
        .filter(|(_, p)| matches!(p, UniformProvider::NodeTexture { .. }))
        .map(|(name, _)| std::sync::Arc::clone(name))
        .collect()
}

/// One workspace's pass, in as many batches as keep each under [`PASS_TEXTURES`]: one module
/// where the whole binds few enough, which is every workspace but a crowded one. See
/// [`wgsl::build_pass`].
fn build_pass(
    graph: &Graph,
    workspace: crate::graph::WorkspaceId,
    measured: &[NodeId],
    thumbs: bool,
) -> Vec<Shader> {
    let items = pass_items(graph, workspace, measured, thumbs);
    let Some(whole) = build_items(graph, &items) else {
        return Vec::new();
    };
    if textures_of(&whole).len() <= PASS_TEXTURES {
        return vec![whole];
    }
    // Greedy, in the pass's own order: an item joins the batch being filled while the
    // textures they bind together stay under the limit. One that alone binds more is a
    // batch of its own, and fails to link as an Output that binds as many would.
    let mut batches: Vec<(Vec<Item>, BTreeSet<std::sync::Arc<str>>)> = Vec::new();
    for item in items {
        let needs = build_items(graph, &[item]).map_or_else(BTreeSet::new, |s| textures_of(&s));
        match batches.last_mut() {
            Some((members, bound))
                if bound.union(&needs).count() <= PASS_TEXTURES || members.is_empty() =>
            {
                members.push(item);
                bound.extend(needs);
            }
            _ => batches.push((vec![item], needs)),
        }
    }
    batches
        .iter()
        .filter_map(|(members, _)| build_items(graph, members))
        .collect()
}

/// One node's measurement alone, as its pass runs it.
fn build_measure(graph: &Graph, id: NodeId) -> Option<Shader> {
    build_items(graph, &[Item::Measure(id)])
}

/// Which output ports carry a texture, so the renderer knows what to bind.
pub fn texture_outputs(shader: &Shader) -> Vec<(&str, NodeId, &'static str)> {
    shader
        .uniforms
        .iter()
        .filter_map(|(name, p)| match p {
            UniformProvider::NodeTexture { node, port } => Some((&**name, *node, *port)),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes;
    use emath::Pos2;

    fn probe() -> (Graph, NodeId) {
        let mut g = Graph::new();
        let n = nodes::add_to_graph(&mut g, "probe", Pos2::ZERO).unwrap();
        (g, n)
    }

    #[test]
    fn a_connected_varying_input_becomes_a_call() {
        let mut g = Graph::new();
        let a = nodes::add_to_graph(&mut g, "probe", Pos2::ZERO).unwrap();
        let b = nodes::add_to_graph(&mut g, "probe", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(a, "output"), PortRef::new(b, "input"))
            .unwrap();

        let mut ctx = CompileContext::new(&g);
        // A field cabled in is the upstream node's function, evaluated at this point.
        assert_eq!(ctx.input(b, "input", "uv"), "probe1_output(uv)");
    }

    #[test]
    fn a_bare_float_input_falls_back_to_zero() {
        let (g, n) = probe();
        let mut ctx = CompileContext::new(&g);
        assert_eq!(ctx.input(n, "bare", "uv"), "0.0");
        assert!(ctx.uniforms.is_empty(), "and declares nothing");
    }

    #[test]
    fn a_bare_color_input_falls_back_to_the_generative_map() {
        let mut g = Graph::new();
        let out = nodes::add_to_graph(&mut g, "output", Pos2::ZERO).unwrap();
        let mut ctx = CompileContext::new(&g);
        assert_eq!(ctx.input(out, "input", "uv"), "defaultUvMap(uv)");
    }

    /// The hue wheel stands still: it is a function of the point alone, so an unplugged color
    /// input draws the same wheel at every moment and a loop closes on it at any length.
    #[test]
    fn the_hue_wheel_reads_no_time() {
        let prelude = super::wgsl::PRELUDE;
        let start = prelude
            .find("fn defaultUvMap")
            .expect("the prelude has the wheel");
        let body = &prelude[start..];
        let body = &body[..body.find("\n}").expect("the function ends")];
        assert!(!body.contains("u_time"), "{body}");
    }

    fn add(g: &mut Graph, slug: &str) -> NodeId {
        nodes::add_to_graph(g, slug, Pos2::ZERO).expect("in the registry")
    }

    /// The body of the function `name` in `module`, from its signature to its closing brace.
    fn function<'m>(module: &'m str, name: &str) -> &'m str {
        let start = module
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("no fn {name} in:\n{module}"));
        let body = &module[start..];
        &body[..body.find("\n}").expect("the function ends")]
    }

    /// `chain`'s nodes cabled solid to solid in order, into a `solidrender`, into an Output:
    /// the graph, the chain's ids and the Output's.
    fn solid_chain(chain: &[&str]) -> (Graph, Vec<NodeId>, NodeId) {
        let mut g = Graph::new();
        let ids: Vec<NodeId> = chain.iter().map(|slug| add(&mut g, slug)).collect();
        for pair in ids.windows(2) {
            g.connect(
                PortRef::new(pair[0], "solid"),
                PortRef::new(pair[1], "input"),
            )
            .unwrap();
        }
        let render = add(&mut g, "solidrender");
        let out = add(&mut g, "output");
        g.connect(
            PortRef::new(*ids.last().unwrap(), "solid"),
            PortRef::new(render, "solid"),
        )
        .unwrap();
        g.connect(PortRef::new(render, "output"), PortRef::new(out, "input"))
            .unwrap();
        (g, ids, out)
    }

    /// Every solid on the way to a render is two functions of a point: the whole, and the
    /// distance alone with a `_d` suffix, each calling the same variant of its input.
    #[test]
    fn a_solid_compiles_whole_and_as_its_distance_alone() {
        let (g, ids, out) = solid_chain(&["sphere", "place", "inflate"]);
        let shader = wgsl::build(&g, out).expect("connected");
        assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);
        let (sphere, place, round) = (ids[0], ids[1], ids[2]);
        for name in [
            format!("sphere{sphere}_solid"),
            format!("place{place}_solid"),
            format!("inflate{round}_solid"),
        ] {
            for variant in [name.clone(), format!("{name}_d")] {
                assert!(
                    shader
                        .body
                        .contains(&format!("fn {variant}(p: vec3f) -> vec4f {{")),
                    "{variant}:\n{}",
                    shader.body
                );
            }
        }
        let whole = function(&shader.body, &format!("inflate{round}_solid"));
        assert!(whole.contains(&format!("place{place}_solid(")), "{whole}");
        assert!(
            !whole.contains(&format!("place{place}_solid_d(")),
            "{whole}"
        );
        let dist = function(&shader.body, &format!("inflate{round}_solid_d"));
        assert!(dist.contains(&format!("place{place}_solid_d(")), "{dist}");
    }

    /// The distance alone of a Texture and of a Paint is its input's distance alone: no
    /// picture is sampled and no color is read, which the whole of each does.
    #[test]
    fn the_distance_alone_reads_no_picture_and_no_color() {
        let (mut g, ids, out) = solid_chain(&["sphere", "surfacetexture", "paint"]);
        let (sphere, texture, paint) = (ids[0], ids[1], ids[2]);
        let picture = add(&mut g, "checkerboard");
        g.connect(
            PortRef::new(picture, "output"),
            PortRef::new(texture, "picture"),
        )
        .unwrap();
        for projection in ["box", "front", "sphere"] {
            g.get_mut(texture)
                .unwrap()
                .options
                .insert("projection", projection.to_string());
            let shader = wgsl::build(&g, out).expect("connected");
            assert!(shader.diagnostics.is_empty(), "{:?}", shader.diagnostics);

            let whole = function(&shader.body, &format!("surfacetexture{texture}_solid"));
            assert!(whole.contains("checkerboard"), "{projection}: {whole}");
            let dist = function(&shader.body, &format!("surfacetexture{texture}_solid_d"));
            assert!(!dist.contains("checkerboard"), "{projection}: {dist}");
            assert!(!dist.contains("u."), "{projection}: {dist}");
            assert!(
                dist.contains(&format!("sphere{sphere}_solid_d(p)")),
                "{dist}"
            );

            let whole = function(&shader.body, &format!("paint{paint}_solid"));
            assert!(whole.contains("u."), "the whole reads its color: {whole}");
            let dist = function(&shader.body, &format!("paint{paint}_solid_d"));
            assert!(!dist.contains("u."), "{dist}");
            assert!(
                dist.contains(&format!("surfacetexture{texture}_solid_d(p)")),
                "{dist}"
            );
        }
    }

    /// An unconnected solid input is nothing, `SOLID_FAR` away, in both variants.
    #[test]
    fn an_unconnected_solid_input_is_nothing_far_away() {
        let mut g = Graph::new();
        let render = add(&mut g, "solidrender");
        let union = add(&mut g, "union");
        let nothing = format!("vec4f({SOLID_FAR}, 0.5, 0.5, 0.5)");
        let mut ctx = CompileContext::new(&g);
        assert_eq!(ctx.input(render, "solid", "p"), nothing);
        for color in [true, false] {
            assert_eq!(ctx.solid_input(render, "solid", "pp", color), nothing);
            assert_eq!(ctx.solid_input(union, "b", "p", color), nothing);
        }
        assert!(
            ctx.solid_color(),
            "solid_input restores the variant it found"
        );
        assert!(ctx.uniforms.is_empty(), "and declares nothing");
        assert!(SOLID_FAR.parse::<f32>().is_ok_and(|far| far > 80.0));
    }

    /// A solid connects to a solid and to nothing else, either way; a solid node's number
    /// takes a field, read at the point, and a CPU node's number as well.
    #[test]
    fn a_solid_feeds_only_a_solid() {
        use PortType::*;
        let all = [
            VaryingNumber,
            VaryingColor,
            UniformNumber,
            UniformColor,
            Action,
            Solid,
        ];
        for ty in all {
            assert_eq!(Solid.feeds(ty), ty == Solid, "Solid into {ty:?}");
            assert_eq!(ty.feeds(Solid), ty == Solid, "{ty:?} into Solid");
        }
        assert!(!VaryingNumber.feeds(UniformNumber));

        let mut g = Graph::new();
        let sphere = add(&mut g, "sphere");
        let union = add(&mut g, "union");
        let out = add(&mut g, "output");
        let picture = add(&mut g, "checkerboard");
        let field = add(&mut g, "vignette");
        let slew = add(&mut g, "slew");
        assert!(
            g.connect(PortRef::new(sphere, "solid"), PortRef::new(out, "input"))
                .is_err()
        );
        assert!(
            g.connect(PortRef::new(picture, "output"), PortRef::new(union, "a"))
                .is_err()
        );
        assert!(
            g.connect(PortRef::new(field, "mask"), PortRef::new(sphere, "radius"))
                .is_ok()
        );
        assert!(
            g.connect(PortRef::new(slew, "output"), PortRef::new(sphere, "x"))
                .is_ok()
        );
        assert!(
            g.connect(PortRef::new(sphere, "solid"), PortRef::new(union, "a"))
                .is_ok()
        );
    }
}
