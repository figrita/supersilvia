// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Example Library: workspace files compiled into the binary, offered under Help ▸
//! Example Library, each added to the project on screen as a tab of its own.
//!
//! The files are `assets/examples/<NN-Category>/<NN-Title>.ssw`, written by
//! `scripts/examples/make.py`, and `build.rs` lists them in the order of their `NN` prefixes.
//! An example is the same file a workspace export writes, and it arrives through the same
//! import a file from disk does, with no folder of media beside it: an example names no file
//! and opens no device (`docs/architecture.md`).

/// One example: where the menu shows it, and the workspace file itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Example {
    /// Its category's folder name, less the `NN-` prefix: a submenu's title.
    pub category: &'static str,
    /// Its file's name, less the `NN-` prefix and the extension: its entry's label.
    pub title: &'static str,
    /// The `.ssw` file's text.
    pub file: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/examples.rs"));

/// The examples by category, in menu order: each category with its examples, which the list
/// holds side by side.
pub fn categories() -> impl Iterator<Item = (&'static str, &'static [Example])> {
    EXAMPLES
        .chunk_by(|a, b| a.category == b.category)
        .map(|run| (run[0].category, run))
}
