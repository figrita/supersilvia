// SPDX-License-Identifier: AGPL-3.0-or-later

//! One declaration form for a node that is nothing but a shader.
//!
//! `math.rs` and `decompose.rs` each carry a macro over one shape — two floats and an
//! expression, a color and a conversion. This is the same idea widened until a shader-only
//! node of any shape fits: a port list, an option list, and one body per output. It exists
//! because 99 of silvia's 144 nodes are exactly that and nothing else, so the alternative was
//! seventy hand-written `NodeDef` literals differing only in their strings.
//!
//! It expands to a `const NodeDef` and nothing else. The registry, the compiler, the menu and
//! `every_node_compiles_on_the_gpu` cannot tell a macro node from a hand-written one, which
//! is the property that lets a node start here and move out when it grows a `cpu` half.
//!
//! **A body is WGSL with `{key}` holes in it.** The hole is filled by `ctx.input`, so it is
//! the same one-character abstraction boundary a hand-written generator has: what comes back
//! may be a call, a uniform, a prelude global or a fallback. Two things differ from writing
//! `format!` by hand, and both are why the splice happens at run time rather than through
//! `format!`. WGSL braces need no doubling, because only an exact `{key}` is a hole. And a
//! hole nobody wrote is never asked for, so an output that ignores an input neither declares
//! its uniform nor drags its producer's function into the shader.

use crate::compile::CompileContext;
use crate::graph::NodeId;
use crate::nodes::EvalFn;

/// Fill every `{key}` hole in `source` from the graph.
///
/// `inputs` pairs each input key with the uv expression it is sampled at — `"uv"` unless the
/// definition said otherwise, as a polka dot samples its radius at the center of the cell it
/// is drawing rather than at the fragment.
pub fn splice(
    node: NodeId,
    ctx: &mut CompileContext,
    source: String,
    inputs: &'static [(&'static str, &'static str)],
) -> String {
    let mut out = source;
    for (key, uv) in inputs {
        let hole = ["{", key, "}"].concat();
        if !out.contains(&hole) {
            continue;
        }
        let expr = ctx.input(node, key, uv);
        out = out.replace(&hole, &expr);
    }
    out
}

/// The uv an input is sampled at: what the definition named, or `uv`.
///
/// A `const fn` over a zero-or-one element slice, because a `macro_rules!` repetition cannot
/// carry a default for an optional piece of itself.
pub const fn sample_at(named: &'static [&'static str]) -> &'static str {
    if named.is_empty() { "uv" } else { named[0] }
}

/// The range an output declares, or none.
///
/// A `const fn` like `sample_at` rather than `..OutputDef::EMPTY`: the macro's literal names
/// every other field, and a struct update over a complete literal has no effect.
pub const fn range_of(named: &'static [&'static str]) -> Option<&'static str> {
    if named.is_empty() {
        None
    } else {
        Some(named[0])
    }
}

/// The evaluator an output declares, or none. `range_of`'s shape, for the same reason.
pub const fn eval_of(named: &'static [EvalFn]) -> Option<EvalFn> {
    if named.is_empty() {
        None
    } else {
        Some(named[0])
    }
}

/// A piece of a generator: WGSL, or a rule for building some.
///
/// The literal case is most of them. `varying` is the other: a body whose text depends on an
/// option, which is the only thing a shader-only node ever branches on. It is a trait rather
/// than "the body is always a closure" so that the ninety per cent that are a string stay a
/// string, and it takes `node` and `ctx` as arguments rather than reading them out of the
/// macro's own scope, because a closure written at the call site cannot see a name the macro
/// introduced.
pub trait Body {
    fn build(self, node: NodeId, ctx: &CompileContext<'_>) -> String;
}

impl Body for &str {
    fn build(self, _node: NodeId, _ctx: &CompileContext<'_>) -> String {
        self.to_string()
    }
}

impl Body for String {
    fn build(self, _node: NodeId, _ctx: &CompileContext<'_>) -> String {
        self
    }
}

/// A body built from the node's options.
pub struct Varying<F>(F);

impl<F: FnOnce(NodeId, &CompileContext<'_>) -> String> Body for Varying<F> {
    fn build(self, node: NodeId, ctx: &CompileContext<'_>) -> String {
        (self.0)(node, ctx)
    }
}

/// A body whose WGSL depends on what the node's options say.
pub fn varying<F: FnOnce(NodeId, &CompileContext<'_>) -> String>(f: F) -> Varying<F> {
    Varying(f)
}

/// A node that is a port list, an option list and one WGSL body per output.
///
/// ```ignore
/// node! {
///     DEF,
///     slug: "circle", icon: "🔵", label: "Circle", category: Generate,
///     tooltip: "A circle with a soft edge.",
///     inputs: [
///         VaryingColor "foreground" "Foreground" = Control::color("#ffffffff"),
///         VaryingNumber "radius" "Radius" = Control::num(0.5, 0.0, 2.0, 0.01, "⬓"),
///     ],
///     wgsl_common: "    let mask = 1.0 - smoothstep(...);",
///     outputs: [
///         VaryingColor "color" "Color" = "    return mix(vec4f(0.0), {foreground}, mask);",
///         VaryingNumber "mask" "Mask" in "[0, 1]" = "    return mask;",
///     ],
/// }
/// ```
///
/// `hidden:` after the inputs is a list of controls with no port — values the node keeps for
/// itself, drawn by a region of its own rather than on a row, and spliced into a body by the
/// same `{key}` hole a port's control is. `regions:` after it is the node's own area.
///
/// `wgsl_common` is WGSL prepended to every body, for the work an output shares with the field
/// beside it, and may be left out. A body is a string, or `varying(|node, ctx| …)` where its
/// WGSL depends on an option. An input may name the uv it is sampled at with `at "expr"`
/// before its `=`, and an output the range its maths bounds it to with `in "[0, 1]"`.
/// `wgsl_utils` names the module-scope functions the bodies call, which the compiler emits
/// once into any module that reaches this node and never into one that does not. See
/// [WGSL](../../docs/nodes.md#wgsl) for the conventions a body is written to.
///
/// `eval(f)` after that — before the `=` — names the [`EvalFn`] that makes the output
/// **dual**: the same formula in Rust, which `tick` evaluates where every input of the node
/// resolves to a uniform number. See [Dual outputs](../../docs/nodes.md#dual-outputs).
///
/// An option's row is a closed list of its `choices` unless it says `free "placeholder"`
/// after them, which draws a typed field with the choices as presets; `checked by f` after
/// that names the `fn(&str) -> bool` the row's border reads.
///
/// `timing: Timing::periodic(0.5),` before `inputs:` makes the node one that moves with
/// time ([`crate::nodes::timing`]), and `timing_xy:` one that moves on two axes. The macro
/// writes its time rows — Time, Speed and a varying Offset per axis — after `inputs:` and
/// before `after_time:`, its Timing heading and mode after its options, and a body reads where
/// it is through the Time hole and the prelude's time helpers: `time_periodic({clock},
/// {phaseOffset})` is the cycle, `time_repeat({clock}, n, {phaseOffset})` the time modulo a
/// period `n` dividing `phasor::WHOLE_WRAP`, and `time_unbounded({clock}, {phaseOffset})` the
/// whole count. Time is what the synth publishes each tick — the ambient reading or a cable in
/// Loop mode, the node's own integrated Speed in Free mode — so a body never reads Speed.
macro_rules! node {
    (
        $(#[$attr:meta])*
        $name:ident,
        slug: $slug:literal,
        icon: $icon:literal,
        label: $label:literal,
        category: $category:ident,
        tooltip: $tooltip:literal,
        $( width: $width:literal, )?
        $( timing: $timing:expr, )?
        $( timing_xy: $timing_xy:expr, )?
        inputs: [ $(
            $ity:ident $ikey:literal $ilabel:literal $(at $iat:literal)? = $ictl:expr
        ),* $(,)? ],
        $( after_time: [ $(
            $aty:ident $akey:literal $alabel:literal $(at $aat:literal)? = $actl:expr
        ),* $(,)? ], )?
        $( hidden: [ $(
            $hkey:literal $hlabel:literal = $hctl:expr
        ),* $(,)? ], )?
        $( regions: $regions:expr, )?
        $( options: [ $(
            $okey:literal $olabel:literal = $odefault:literal [ $( $ovalue:literal => $oname:literal ),* $(,)? ]
            $( free $oplaceholder:literal $( checked by $ovalidate:expr )? )?
        ),* $(,)? ], )?
        $( wgsl_utils: [ $( $wutil:expr ),* $(,)? ], )?
        $( wgsl_common: $wcommon:expr, )?
        outputs: [ $(
            $pty:ident $pkey:literal $plabel:literal
            $( in $prange:literal )? $( eval($peval:expr) )? = $wbody:expr
        ),* $(,)? ] $(,)?
    ) => {
        $(#[$attr])*
        pub static $name: NodeDef = {
            /// Every input, with the uv expression its holes are sampled at. Hoisted out of
            /// the outputs so its repetition and theirs do not have to be the same length.
            const INPUTS: &[(&str, &str)] = &[
                $( ($ikey, $crate::nodes::macros::sample_at(&[$($iat)?])), )*
                // The time rows, at the fragment: a body reads Time and Offset, never Speed.
                $(
                    ($crate::nodes::timing::TIME, $crate::nodes::macros::at_fragment($timing)),
                    ($crate::nodes::timing::OFFSET, "uv"),
                )?
                $(
                    ($crate::nodes::timing::TIME, $crate::nodes::macros::at_fragment($timing_xy)),
                    ($crate::nodes::timing::OFFSET, "uv"),
                    ($crate::nodes::timing::TIME_Y, "uv"),
                    ($crate::nodes::timing::OFFSET_Y, "uv"),
                )?
                $($( ($akey, $crate::nodes::macros::sample_at(&[$($aat)?])), )*)?
                // A hidden control fills a hole exactly as a port does: `ctx.input` resolves
                // it to the same `u_control_…` uniform, having found it through
                // `NodeDef::input`, which searches the hidden list too. It is sampled at the
                // fragment, because a value with no port has nowhere else to be read at.
                $($( ($hkey, "uv"), )*)?
            ];
            /// `wgsl_common`, or nothing. A function rather than the expression inline, for
            /// the reason `INPUTS` is hoisted: it is matched outside the outputs' repetition.
            #[allow(dead_code, unused_mut, unused_variables)]
            fn wgsl_common(
                node: $crate::graph::NodeId,
                ctx: &$crate::compile::CompileContext<'_>,
            ) -> String {
                #[allow(unused_imports)]
                use $crate::nodes::macros::Body as _;
                let mut source = String::new();
                $( source.push_str(&($wcommon).build(node, ctx)); )?
                source
            }
            NodeDef {
            slug: $slug,
            icon: $icon,
            label: $label,
            tooltip: $tooltip,
            $( width: Some($width), )?
            category: Category::$category,
            $( timing: Some($timing), )?
            $( timing: Some(($timing_xy).xy()), )?
            inputs: &[
                $( InputDef { key: $ikey, label: $ilabel, ty: $ity, control: $ictl }, )*
                // The time rows, expanded from the declaration: an Offset on a node that draws
                // is a field.
                $(
                    $crate::nodes::timing::time_row($timing, 0),
                    $crate::nodes::timing::speed_row($timing, 0),
                    $crate::nodes::timing::offset_row(
                        $timing,
                        0,
                        $crate::graph::PortType::VaryingNumber,
                    ),
                )?
                $(
                    $crate::nodes::timing::time_row(($timing_xy).xy(), 0),
                    $crate::nodes::timing::speed_row(($timing_xy).xy(), 0),
                    $crate::nodes::timing::offset_row(
                        ($timing_xy).xy(),
                        0,
                        $crate::graph::PortType::VaryingNumber,
                    ),
                    $crate::nodes::timing::time_row(($timing_xy).xy(), 1),
                    $crate::nodes::timing::speed_row(($timing_xy).xy(), 1),
                    $crate::nodes::timing::offset_row(
                        ($timing_xy).xy(),
                        1,
                        $crate::graph::PortType::VaryingNumber,
                    ),
                )?
                $($( InputDef { key: $akey, label: $alabel, ty: $aty, control: $actl }, )*)?
            ],
            // A hidden control has no port, so its type is not a port's type: it is one
            // number for the whole frame, which is what `UniformNumber` says and what the
            // compiler emits for it either way.
            hidden: &[ $($(
                InputDef {
                    key: $hkey,
                    label: $hlabel,
                    ty: $crate::graph::PortType::UniformNumber,
                    control: $hctl,
                },
            )*)? ],
            $( regions: $regions, )?
            options: &[ $($(
                OptionDef {
                    key: $okey,
                    label: $olabel,
                    default: $odefault,
                    choices: &[ $( ($ovalue, $oname) ),* ],
                    $(
                        placeholder: Some($oplaceholder),
                        $( validate: Some($ovalidate), )?
                    )?
                    ..OptionDef::EMPTY
                },
            )*)?
            // A node that moves with time folds its time rows under a heading, with its mode
            // on the heading's bar, last because both are drawn on its rows and never in the
            // option block.
            $( $crate::nodes::macros::timing_heading!($timing), $crate::nodes::timing::MODE, )?
            $( $crate::nodes::macros::timing_heading!($timing_xy), $crate::nodes::timing::MODE, )?
            ],
            $( row_headings: $crate::nodes::macros::row_headings!($timing), )?
            $( row_headings: $crate::nodes::macros::row_headings!($timing_xy), )?
            wgsl_utils: &[ $($($wutil),*)? ],
            outputs: &[ $(
                OutputDef {
                    key: $pkey,
                    label: $plabel,
                    ty: $pty,
                    kind: OutputKind::Shader,
                    range: $crate::nodes::macros::range_of(&[$($prange)?]),
                    eval: $crate::nodes::macros::eval_of(&[$($peval)?]),
                    wgsl: |node, ctx, _func| {
                        use $crate::nodes::macros::Body as _;
                        let mut source = wgsl_common(node, ctx);
                        source.push_str(&($wbody).build(node, ctx));
                        $crate::nodes::macros::splice(node, ctx, source, INPUTS)
                    },
                    // A node this macro can express publishes no texture, so the sampling
                    // fields are the rule's and never spelled here.
                    ..OutputDef::EMPTY
                },
            )* ],
            ..NodeDef::EMPTY
            }
        };
    };
}

pub(crate) use node;

/// The Timing heading, for a node `node!` declares with a `timing:` — the expression is only
/// how the macro knows there is one.
macro_rules! timing_heading {
    ($timing:expr) => {
        $crate::nodes::timing::HEADING
    };
}

pub(crate) use timing_heading;

/// The row headings of a node `node!` declares with a `timing:`, by the same trick.
macro_rules! row_headings {
    ($timing:expr) => {
        $crate::nodes::timing::ROW_HEADINGS
    };
}

pub(crate) use row_headings;

/// Where a time row is sampled: at the fragment. A function of the declaration only so that
/// `node!`'s repetition over it has something to repeat on.
pub const fn at_fragment(_timing: crate::nodes::Timing) -> &'static str {
    "uv"
}
