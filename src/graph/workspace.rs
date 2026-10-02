// SPDX-License-Identifier: AGPL-3.0-or-later

//! A workspace: one view onto the graph. Pure data.
//!
//! A node is on a non-empty set of workspaces, and a node on two of them is one node with
//! one position and one set of controls. Nothing nests and nothing is instanced, so the
//! graph knows nothing about a workspace beyond a set of ids on each node.

/// Workspace identity: one stable integer per workspace per project, never reused.
///
/// The same rule as `NodeId`, and for the same reason: a node's set names workspaces by id,
/// and undo re-inserts a removed workspace under the one it had.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct WorkspaceId(pub u32);

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What a workspace holds, and therefore what its tab shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum WorkspaceKind {
    /// Nodes and cables on a canvas.
    #[default]
    Video,
}

impl WorkspaceKind {
    /// Does a workspace of this kind mean anything on its own?
    ///
    /// A `Video` workspace is nodes and cables: it is self-contained, and exporting one
    /// hands somebody a file that works. A timeline will not be — everything in one is a
    /// reference into other workspaces — so exporting one alone is refused with the reason
    /// and it travels with the project instead. This is the branch that kind lands in; there
    /// is no such kind yet.
    pub fn is_portable(self) -> bool {
        match self {
            Self::Video => true,
        }
    }
}

/// How a workspace's canvas is navigated, and what constrains node layout on it.
///
/// Document data, not a preference: a workspace laid out as a strip is laid out as a strip,
/// and opens that way for whoever opens it. One per workspace, saved in that workspace's own
/// file, because it describes that canvas and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutMode {
    /// An unbounded plane with pan and zoom.
    #[default]
    Canvas,
    /// A strip at a fixed scale: the wheel scrolls along the dataflow, and node positions
    /// are clamped to the viewport's height.
    Linear,
}

/// One workspace. A view, not a unit: it owns no node and holds no state of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub kind: WorkspaceKind,
    /// A line about what this workspace is for, shown on its card in the project tab.
    ///
    /// Document data, like a name: it rides in the workspace's own file, so it travels with
    /// an export, and it is set through the bus so it is undoable like a rename.
    pub blurb: String,
    /// How this workspace's canvas is navigated.
    pub layout: LayoutMode,
}
