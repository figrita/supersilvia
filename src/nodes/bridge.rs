// SPDX-License-Identifier: AGPL-3.0-or-later

//! The castings a cable can be carried by: silvia's `js/typeConversions.js`, as a table.
//!
//! A cable dropped on a port it cannot land on offers the ways of *casting* it to what that
//! port takes — a color read as its luminosity, a number painted as a gray. The table below
//! is that list, and it is **curated**: each row is a casting a hand would ask for, named for
//! what it does rather than for the node that does it, with silvia's own labels and icons.
//!
//! It is written out rather than queried out of the registry. A query — every node with an
//! input the source feeds and an output of the target's type — is the same menu as *insert any
//! node that happens to fit*, which is an inline patchbay: fifteen rows for a color onto a
//! varying number, every measurement there is for a color onto a uniform number, and a new
//! row the day any node lands. silvia offers twelve rows in total, across the two pairs it
//! has, and that is the point of the gesture: a handful of sensible castings, chosen, at the
//! port. See docs/decisions.md, "The conversion menu is a curated table".
//!
//! What a row can do is a little more than a node with a cable on each side: `inputs` is a
//! list, so *Grayscale* fans one number into an `rgba`'s red, green and blue — silvia's
//! `float-to-color` closure, which no pairing of one input with one output could express.
//!
//! The menu that shows these is `ui/bridge.rs`; `can_bridge` below is the port state it draws.

use crate::graph::{Graph, PortRef, PortType};
use crate::nodes::{NodeDef, decompose, rgba, tap};

/// One casting: the node that performs it, the input or inputs the cable fans out to, the
/// output that carries on to the target, and the name the row wears.
#[derive(Clone, Copy)]
pub struct Bridge {
    /// What the row says. The casting's name, not the node's: *Red Channel* rather than
    /// *Channel Splitter · R*, since the row is an answer to "what should this become?".
    pub label: &'static str,
    /// The row's own icon, which for most rows is the node's and for a channel is the
    /// channel's.
    pub icon: &'static str,
    pub def: &'static NodeDef,
    /// The inputs the source feeds. More than one is a fan-out: the same cable lands on each,
    /// which is how a number becomes a gray.
    pub inputs: &'static [&'static str],
    /// The output that connects to the target.
    pub output: &'static str,
}

/// One row of a table.
macro_rules! cast {
    ($label:literal, $icon:literal, $def:expr, [$($input:literal),+] => $output:literal) => {
        Bridge {
            label: $label,
            icon: $icon,
            def: &$def,
            inputs: &[$($input),+],
            output: $output,
        }
    };
}

/// A color onto a varying number: silvia's `color-to-float`, whole. Four readings of the
/// color and its four channels — not the eleven `Convert` nodes, of which *Value*, *Chroma*
/// and the rest are reductions a hand asks for by name in the Nodes menu rather than castings
/// it wants offered on a drag.
static COLOR_TO_FRAGMENT: &[Bridge] = &[
    cast!("Luminosity", "🕯", decompose::LUMINOSITY, ["input"] => "output"),
    cast!("Lightness", "💡", decompose::LIGHTNESS, ["input"] => "output"),
    cast!("Hue", "🎨", decompose::HUE, ["input"] => "output"),
    cast!("Saturation", "🌈", decompose::SATURATION, ["input"] => "output"),
    cast!("Red Channel", "🔴", decompose::CHANNELSPLITTER, ["input"] => "r"),
    cast!("Green Channel", "🟢", decompose::CHANNELSPLITTER, ["input"] => "g"),
    cast!("Blue Channel", "🔵", decompose::CHANNELSPLITTER, ["input"] => "b"),
    cast!("Alpha Channel", "🌫", decompose::CHANNELSPLITTER, ["input"] => "a"),
];

/// A varying number onto a color: silvia's `float-to-color`, whole. One `rgba` each time,
/// and which of its inputs the cable lands on is the whole difference — *Grayscale* takes
/// three of them. The alpha is left at its default, which is opaque.
///
/// A uniform number onto a color is the same table: a uniform number feeds a varying number
/// input with nothing in between, so the same four rows do the same four things.
static NUMBER_TO_COLOR: &[Bridge] = &[
    cast!("Grayscale", "⬜", rgba::DEF, ["r", "g", "b"] => "output"),
    cast!("Red Only", "🔴", rgba::DEF, ["r"] => "output"),
    cast!("Green Only", "🟢", rgba::DEF, ["g"] => "output"),
    cast!("Blue Only", "🔵", rgba::DEF, ["b"] => "output"),
];

/// A color onto a uniform number, which silvia has no version of: its floats never came back
/// off the GPU. A reading of the whole frame, which is the `tap`'s three summaries of what it
/// measures — luminosity, until the node's own picker says otherwise.
///
/// Three rows and not the twelve the registry can pair: the `tap`'s centroid is a position
/// rather than a casting of the color, and `sample` and `autoexposure` are nodes a hand
/// reaches for deliberately, with a point or a response time to set.
static COLOR_TO_UNIFORM: &[Bridge] = &[
    cast!("Average", "⚖", tap::DEF, ["input"] => "mean"),
    cast!("Brightest", "🔆", tap::DEF, ["input"] => "max"),
    cast!("Darkest", "🌑", tap::DEF, ["input"] => "min"),
];

/// A picture onto a uniform color: what color it is on average, which is the `tap`'s own
/// mean color.
///
/// One row, not three. *Brightest* and *Darkest* are extremes of a chosen quantity, and the
/// extreme of a quantity is a number; there is no pixel a tap keeps to hand back as *the
/// darkest color*, and inventing one out of three separate channel minima would be a color
/// that was never in the picture.
static COLOR_TO_UNIFORM_COLOR: &[Bridge] =
    &[cast!("Mean Color", "⚖", tap::DEF, ["input"] => "color")];

/// A varying number onto a uniform number: the same three summaries, through the `tap`'s
/// sidechain, which measures the field it is handed rather than the picture passing through
/// it.
static FRAGMENT_TO_UNIFORM: &[Bridge] = &[
    cast!("Average", "⚖", tap::DEF, ["number"] => "mean"),
    cast!("Highest", "🔆", tap::DEF, ["number"] => "max"),
    cast!("Lowest", "🌑", tap::DEF, ["number"] => "min"),
];

/// The castings offered for a cable of type `from` released on an input of type `to`.
///
/// Empty where `from` feeds `to`, since a cable is then the whole answer — a uniform number
/// into a varying number input promotes with nothing in between — and empty where either
/// side is an `Action`, which has nothing to cast to or from.
pub fn bridges(from: PortType, to: PortType) -> &'static [Bridge] {
    use PortType::{UniformColor, UniformNumber, VaryingColor, VaryingNumber};
    match (from, to) {
        // A uniform color is a color that already fits every one of these nodes' inputs, so
        // it reads the same two tables a varying color does, for the same reason a uniform
        // number reads the varying number's: the promotion is free and the rows do not
        // change.
        (VaryingColor | UniformColor, VaryingNumber) => COLOR_TO_FRAGMENT,
        (VaryingColor | UniformColor, UniformNumber) => COLOR_TO_UNIFORM,
        (VaryingNumber | UniformNumber, VaryingColor) => NUMBER_TO_COLOR,
        (VaryingNumber, UniformNumber) => FRAGMENT_TO_UNIFORM,
        (VaryingColor, UniformColor) => COLOR_TO_UNIFORM_COLOR,
        _ => &[],
    }
}

/// Can this cable be carried by a bridge node, and would a hand be offered one?
///
/// The third port state during a drag, and the whole of it, so `ui/` asks one question rather
/// than three. True when the graph refuses the cable **only** on type — an action, a cycle or
/// a self-connection is a no and stays a no — when the table holds a casting for the pair,
/// and when a node between the two would not itself close a loop.
///
/// A pinned dual node's input is `UniformNumber`, so the pair is read there as it is anywhere
/// else: the rows offered are the `tap`'s, the same three a `slew`'s input gets, and every
/// one of them lands a uniform number on the port rather than the field that was refused.
///
/// It lives here rather than beside `Graph::can_connect` because the middle clause is a
/// question about nodes and `graph/` is topology: `nodes/` is the layer that can see both.
pub fn can_bridge(graph: &Graph, from: PortRef, to: PortRef) -> bool {
    can_bridge_within(graph, None, from, to)
}

/// [`can_bridge`], with the loop question answered from a cable drag's
/// [`Reach`](crate::graph::Reach) where it can be.
pub fn can_bridge_within(
    graph: &Graph,
    reach: Option<&crate::graph::Reach>,
    from: PortRef,
    to: PortRef,
) -> bool {
    let checked = match reach {
        Some(reach) => graph.can_connect_within(reach, from, to),
        None => graph.can_connect(from, to),
    };
    let Err(crate::graph::ConnectError::TypeMismatch { from: a, to: b }) = checked else {
        return false;
    };
    if bridges(a, b).is_empty() {
        return false;
    }
    // The bridge's second cable lands on `to`, carrying everything above `from` with it, so
    // the loop a plain cable would have closed is the loop a bridged one closes. Asked of
    // immediate edges, which is the question `can_connect` asks of a cable of its own.
    !graph.can_reach_within(reach, to.node, from.node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::REGISTRY;

    /// Every pair the table answers, with the types each row has to line up with.
    const TABLES: &[(PortType, PortType, &[Bridge])] = &[
        (
            PortType::VaryingColor,
            PortType::VaryingNumber,
            COLOR_TO_FRAGMENT,
        ),
        (
            PortType::VaryingColor,
            PortType::UniformNumber,
            COLOR_TO_UNIFORM,
        ),
        (
            PortType::VaryingNumber,
            PortType::VaryingColor,
            NUMBER_TO_COLOR,
        ),
        (
            PortType::UniformNumber,
            PortType::VaryingColor,
            NUMBER_TO_COLOR,
        ),
        (
            PortType::VaryingNumber,
            PortType::UniformNumber,
            FRAGMENT_TO_UNIFORM,
        ),
        (
            PortType::UniformColor,
            PortType::VaryingNumber,
            COLOR_TO_FRAGMENT,
        ),
        (
            PortType::UniformColor,
            PortType::UniformNumber,
            COLOR_TO_UNIFORM,
        ),
        (
            PortType::VaryingColor,
            PortType::UniformColor,
            COLOR_TO_UNIFORM_COLOR,
        ),
    ];

    /// A hand-written table can name a port that is not there, which a registry query never
    /// could. So the ports are checked here instead: every row names a node in the registry,
    /// inputs the source can feed, and an output of the target's own type — the last being
    /// what lets `can_bridge` answer for the whole menu at once, since a row whose output is
    /// the target's type is a row whose far cable lands.
    #[test]
    fn every_casting_names_a_node_and_ports_that_line_up() {
        for (from, to, table) in TABLES {
            for cast in *table {
                let def = cast.def;
                let name = format!("{}.{:?}→{}", def.slug, cast.inputs, cast.output);
                assert!(
                    REGISTRY.iter().any(|d| d.slug == def.slug),
                    "{name}: not in the registry"
                );
                assert!(!cast.inputs.is_empty(), "{name}: no input to land on");
                for key in cast.inputs {
                    let input = def
                        .input(key)
                        .unwrap_or_else(|| panic!("{name}: no input {key}"));
                    assert!(
                        from.feeds(input.ty),
                        "{name}: a {from:?} does not feed {key}, a {:?}",
                        input.ty
                    );
                    // An input and an output under one key would be one `PortRef` to
                    // everything downstream of here, and the two cables could not be told
                    // apart.
                    assert_ne!(*key, cast.output, "{name}: one key, two ports");
                }
                let output = def
                    .output(cast.output)
                    .unwrap_or_else(|| panic!("{name}: no output {}", cast.output));
                assert_eq!(output.ty, *to, "{name}: the output is not a {to:?}");
            }
        }
    }

    /// silvia's two tables, row for row, which is what the menu shows for the two pairs it
    /// has. A color onto a varying number is four readings and four channels; a fragment
    /// number onto a color is one `rgba` wired four ways.
    #[test]
    fn the_two_pairs_silvia_has_are_silvias_rows() {
        let labels = |from, to| {
            bridges(from, to)
                .iter()
                .map(|b| b.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels(PortType::VaryingColor, PortType::VaryingNumber),
            [
                "Luminosity",
                "Lightness",
                "Hue",
                "Saturation",
                "Red Channel",
                "Green Channel",
                "Blue Channel",
                "Alpha Channel",
            ]
        );
        assert_eq!(
            labels(PortType::VaryingNumber, PortType::VaryingColor),
            ["Grayscale", "Red Only", "Green Only", "Blue Only"]
        );
    }

    /// A uniform number onto a color is the varying number's table: the promotion means the
    /// same four rows do the same four things.
    #[test]
    fn a_number_onto_a_color_is_the_same_four_rows() {
        assert_eq!(
            bridges(PortType::UniformNumber, PortType::VaryingColor).len(),
            bridges(PortType::VaryingNumber, PortType::VaryingColor).len()
        );
    }

    /// The diamond boundary, which silvia never had: a reading of the whole frame, and the
    /// one node whose job that is.
    #[test]
    fn a_number_is_read_out_of_a_picture_by_a_tap() {
        for (from, expected) in [
            (PortType::VaryingColor, ["Average", "Brightest", "Darkest"]),
            (PortType::VaryingNumber, ["Average", "Highest", "Lowest"]),
        ] {
            let rows = bridges(from, PortType::UniformNumber);
            assert!(rows.iter().all(|b| b.def.slug == "tap"), "{from:?}");
            assert_eq!(
                rows.iter().map(|b| b.label).collect::<Vec<_>>(),
                expected,
                "{from:?}"
            );
        }
    }

    /// A picture onto a uniform color is one row, and it is the tap's mean color: the
    /// extremes are extremes of a number and have no color to hand back.
    #[test]
    fn a_picture_onto_a_uniform_color_is_the_taps_mean_color() {
        let rows = bridges(PortType::VaryingColor, PortType::UniformColor);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Mean Color");
        assert_eq!(rows[0].def.slug, "tap");
        assert_eq!(rows[0].output, "color");
    }

    /// A uniform color reads the same tables a varying color does — the promotion is free,
    /// so every casting that takes a picture takes one of these too.
    #[test]
    fn a_uniform_color_casts_the_way_a_picture_does() {
        for to in [PortType::VaryingNumber, PortType::UniformNumber] {
            let uniform = bridges(PortType::UniformColor, to);
            let fragment = bridges(PortType::VaryingColor, to);
            assert_eq!(
                uniform.iter().map(|b| b.label).collect::<Vec<_>>(),
                fragment.iter().map(|b| b.label).collect::<Vec<_>>(),
                "{to:?}"
            );
        }
    }

    /// A pair a cable already joins has nothing to cast, and an action has nothing to convert
    /// to or from.
    #[test]
    fn a_direct_feed_and_an_action_offer_nothing() {
        for (a, b) in [
            (PortType::VaryingColor, PortType::VaryingColor),
            (PortType::VaryingNumber, PortType::VaryingNumber),
            // The free promotion: a uniform number feeds a varying number input with no
            // node in between.
            (PortType::UniformNumber, PortType::VaryingNumber),
            (PortType::UniformColor, PortType::VaryingColor),
            (PortType::UniformColor, PortType::UniformColor),
            (PortType::Action, PortType::VaryingNumber),
            (PortType::VaryingColor, PortType::Action),
        ] {
            assert!(bridges(a, b).is_empty(), "{a:?} → {b:?}");
        }
    }
}
