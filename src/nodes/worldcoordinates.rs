// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the pixel is, as two fields.
//!
//! Ported from silvia's `worldcoordinates.js`, and the shortest node in either library: no
//! inputs, no controls, two outputs. It is the first node of every patch that draws a shape
//! nobody wrote a node for — without it the math nodes have nothing to be about, because
//! every other number here is one value across the whole frame until something varies it.
//!
//! silvia files it under Source. Here `Source` means a device, a file or a previous frame,
//! and this makes what it publishes out of nothing but `uv`, which is what `Generate`
//! means — so it is a `Generate` node with no picture, only the two fields a picture would
//! be built from. `x` and `y` widen the output vocabulary by the two names a coordinate
//! has; `nodes/mod.rs` holds the list.
//!
//! The coordinates are [worldspace](../../../docs/nodes.md#worldspace) and nothing else:
//! centered on the frame, two units tall, `2 · aspect` across.

use crate::graph::PortType::VaryingNumber;
use crate::nodes::macros::node;
use crate::nodes::{Category, NodeDef, OutputDef, OutputKind};

node! {
    /// The pixel's own place, X and Y.
    DEF,
    slug: "worldcoordinates",
    icon: "🌍",
    label: "World Coordinates",
    category: Generate,
    tooltip: "The pixel's own place, as two fields: X across the frame and Y from -1 at the \
              bottom to 1 at the top. What arithmetic on coordinates starts from.",
    inputs: [],
    outputs: [
        VaryingNumber "x" "X" = "    return uv.x;",
        VaryingNumber "y" "Y" in "[-1, 1]" = "    return uv.y;",
    ],
}
