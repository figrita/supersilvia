// SPDX-License-Identifier: AGPL-3.0-or-later

//! A box of text on the canvas. No ports, no picture, nothing downstream.
//!
//! It exists to say something to whoever opens the patch next — the same job a comment does
//! in code.
//!
//! silvia's note is a node with no inputs, no outputs and a resizable textarea, keeping its
//! text in `values`: the third store, beside `input` and `options`, for state that fits
//! neither. This is that node, and it is what brought that third store here.
//!
//! **Why the text is not an option.** An option could hold the string — this node was built
//! that way first. The problem is that an option's row is drawn by code shared by every node
//! in the library, and that code knows four shapes: a select, a file button, a tick, a
//! one-line field. Those four are all there will ever be. A value is drawn by the node's own
//! code, so the shapes stay open: a box of prose here, a pad or a step grid next.
//!
//! **Why the text is not a control.** A `Control` belongs to an input port, and every port
//! has a type. There is no port type a paragraph could be — nothing downstream can read it,
//! it has no wire color, and the compiler has no name for it.
//!
//! A note compiles to nothing. It has no outputs, so no Output can reach it and it never
//! enters a shader. Typing in one marks nothing for rebuild and costs only the frame the
//! keystroke landed in.
//!
//! Being a node rather than some new kind of canvas object gets it a position, a header to
//! drag, selection, undo, saving, workspace membership and a place in the accessibility
//! tree. Every one of those already works for every node.

use crate::nodes::{Category, NodeDef, ValueDef, ValueKind};

/// How tall the box is, in lines. Enough for a sentence about why a patch is wired the way
/// it is, which is what the node is for.
const LINES: u8 = 4;

pub static DEF: NodeDef = NodeDef {
    slug: "note",
    category: Category::Control,
    icon: "📝",
    label: "Note",
    tooltip: "A box of text on the canvas. Connects to nothing and changes nothing.",
    // Wider than the default: prose in a column the width of a number control is a column of
    // single words. And the one node in the library a hand can drag wider still, which is
    // what silvia's own note is: a comment box is sized to the thing it is commenting on —
    // a label beside one node, or a paragraph across the top of a canvas — and that is the
    // one thing only the person writing it knows. The height keeps looking after itself.
    width: Some(232.0),
    resizable: true,
    values: &[ValueDef {
        key: "text",
        // No label. The box is the whole row, and a caption reading "Text" above a box of
        // text is a caption that says nothing.
        label: "",
        kind: ValueKind::Text {
            rows: LINES,
            placeholder: "a note to whoever opens this next",
            // A note starts empty: it says what its author types and nothing else.
            default: "",
        },
    }],
    ..NodeDef::EMPTY
};
