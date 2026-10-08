// SPDX-License-Identifier: AGPL-3.0-or-later

//! The library's shape on each kind of workspace: the Nodes menu as a tree, which nodes the
//! browser searches before it is widened, and what a new workspace is born holding.
//!
//! The menu is a pure registry query: each node's [`Category`] and, inside Solids, its
//! [`Family`] say where it goes, and the menu names no node. **A kind shapes the library and
//! never the canvas** — every node can be placed and cabled on a workspace of either kind — so
//! this module and [`Category::kind`] are the only places the kind reaches into `nodes/`. See
//! [docs/ui.md](../../../docs/ui.md#the-nodes-menu-a-start-button-two-timers-and-a-mouse-that-speaks-in-crossings).
//!
//! On a video workspace the menu is the video categories, then the numbers and events, then
//! **Solids**, whose submenu holds Render 3D and the families, then the Output. On a solids
//! workspace it is the families, the numbers and events, **Video**, whose submenu holds the
//! video categories, then Render 3D and the Output. Math, Control and Gears drive either kind,
//! so both menus hold them at the top.

use crate::graph::WorkspaceKind;
use crate::nodes::{Category, Family, NodeDef, REGISTRY};

/// One row of the Nodes menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A node, added by choosing the row.
    Node(&'static NodeDef),
    /// A row that opens a submenu.
    Group(Group),
}

/// A named run of rows, shown as a submenu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub icon: &'static str,
    pub label: &'static str,
    pub entries: Vec<Entry>,
}

impl Entry {
    /// The submenu this row opens, if it opens one.
    pub fn entries(&self) -> Option<&[Entry]> {
        match self {
            Self::Node(_) => None,
            Self::Group(group) => Some(&group.entries),
        }
    }

    /// A group of `entries`, or the one node it would hold: a submenu holding a single entry of
    /// the same name chooses nothing.
    fn group(icon: &'static str, label: &'static str, mut entries: Vec<Entry>) -> Option<Self> {
        match entries.len() {
            0 => None,
            1 if matches!(entries[0], Self::Node(_)) => entries.pop(),
            _ => Some(Self::Group(Group {
                icon,
                label,
                entries,
            })),
        }
    }
}

/// The name and icon of the group that holds the other kind's library on a workspace of this
/// one.
const VIDEO: (&str, &str) = ("🎬", "Video");

/// The nodes this machine offers, in registry order.
fn offered() -> impl Iterator<Item = &'static NodeDef> {
    REGISTRY.iter().copied().filter(|d| (d.offered)())
}

/// A category as a menu row: its members, or its one member.
fn category(category: Category) -> Option<Entry> {
    let members = offered()
        .filter(|d| d.category == category)
        .map(Entry::Node)
        .collect();
    Entry::group(category.icon(), category.label(), members)
}

/// The categories of `kind` in menu order, each a row.
fn categories(kind: Option<WorkspaceKind>) -> impl Iterator<Item = Entry> {
    Category::ALL
        .iter()
        .filter(move |c| c.kind() == kind && **c != Category::Output)
        .filter_map(|c| category(*c))
}

/// The solid families, each a row.
fn families() -> impl Iterator<Item = Entry> {
    Family::ALL.iter().filter_map(|family| {
        let members = offered()
            .filter(|d| d.family == Some(*family))
            .map(Entry::Node)
            .collect();
        Entry::group(family.icon(), family.label(), members)
    })
}

/// The solid nodes in no family: what turns a solid into a picture.
fn renders() -> impl Iterator<Item = Entry> {
    offered()
        .filter(|d| d.category == Category::Solid && d.family.is_none())
        .map(Entry::Node)
}

/// The Nodes menu on a workspace of `kind`, top level first.
pub fn menu(kind: WorkspaceKind) -> Vec<Entry> {
    let shared = || categories(None);
    let output = || category(Category::Output);
    match kind {
        WorkspaceKind::Video => {
            let solids: Vec<Entry> = renders().chain(families()).collect();
            categories(Some(WorkspaceKind::Video))
                .chain(shared())
                .chain(Entry::group(
                    Category::Solid.icon(),
                    Category::Solid.label(),
                    solids,
                ))
                .chain(output())
                .collect()
        }
        WorkspaceKind::Solids => {
            let video: Vec<Entry> = categories(Some(WorkspaceKind::Video)).collect();
            families()
                .chain(shared())
                .chain(Entry::group(VIDEO.0, VIDEO.1, video))
                .chain(renders())
                .chain(output())
                .collect()
        }
    }
}

/// Is `def` in the library of a workspace of `kind` — its own, or one that serves either?
/// What the browser searches until it is told to search everything.
pub fn belongs(def: &NodeDef, kind: WorkspaceKind) -> bool {
    def.category.kind().is_none_or(|k| k == kind)
}

/// What a new workspace of `kind` is born holding: kinds left to right, each cabled into the
/// next, the shortest patch that shows a picture. An empty plane is a worse first thing to meet
/// than a few nodes you can delete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Starter {
    /// The kinds, left to right.
    pub nodes: &'static [&'static str],
    /// Each cable as the output key of one node and the input key of the next.
    pub cables: &'static [(&'static str, &'static str)],
}

/// The patch a new workspace of `kind` is born holding.
pub fn starter(kind: WorkspaceKind) -> Starter {
    match kind {
        WorkspaceKind::Video => Starter {
            nodes: &["maininput", "output"],
            cables: &[("frame", "input")],
        },
        WorkspaceKind::Solids => Starter {
            nodes: &["sphere", "solidrender", "output"],
            cables: &[("solid", "solid"), ("output", "input")],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every node a menu holds, however deep.
    fn every(entries: &[Entry]) -> Vec<&'static str> {
        entries
            .iter()
            .flat_map(|e| match e {
                Entry::Node(def) => vec![def.slug],
                Entry::Group(group) => every(&group.entries),
            })
            .collect()
    }

    fn labels(entries: &[Entry]) -> Vec<&'static str> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Node(def) => def.label,
                Entry::Group(group) => group.label,
            })
            .collect()
    }

    fn group<'a>(entries: &'a [Entry], label: &str) -> &'a [Entry] {
        entries
            .iter()
            .find_map(|e| match e {
                Entry::Group(g) if g.label == label => Some(&g.entries[..]),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no group {label}"))
    }

    /// Both menus hold every node this machine offers, once: the kind decides the shape of
    /// the library, never what is in it.
    #[test]
    fn each_menu_holds_every_node_once() {
        let mut all: Vec<&str> = offered().map(|d| d.slug).collect();
        all.sort_unstable();
        for kind in WorkspaceKind::ALL {
            let mut held = every(&menu(*kind));
            held.sort_unstable();
            assert_eq!(held, all, "{kind:?}");
        }
    }

    #[test]
    fn a_video_menu_ends_in_solids_and_the_output() {
        let top = menu(WorkspaceKind::Video);
        assert_eq!(
            labels(&top),
            [
                "Source",
                "Generate",
                "Color",
                "Transform",
                "Effect",
                "Convert",
                "Tap",
                "Math",
                "Control",
                "Gears",
                "Solids",
                "Output"
            ]
        );
        assert!(matches!(top.last(), Some(Entry::Node(d)) if d.slug == "output"));
        let solids = group(&top, "Solids");
        assert!(matches!(&solids[0], Entry::Node(d) if d.slug == "solidrender"));
        let families: Vec<_> = Family::ALL.iter().map(|f| f.label()).collect();
        assert_eq!(labels(&solids[1..]), families);
    }

    #[test]
    fn a_solids_menu_leads_with_the_families_and_holds_video_as_a_group() {
        let top = menu(WorkspaceKind::Solids);
        let mut want: Vec<_> = Family::ALL.iter().map(|f| f.label()).collect();
        want.extend(["Math", "Control", "Gears", "Video", "Render 3D", "Output"]);
        assert_eq!(labels(&top), want);
        assert_eq!(
            labels(group(&top, "Video")),
            [
                "Source",
                "Generate",
                "Color",
                "Transform",
                "Effect",
                "Convert",
                "Tap"
            ]
        );
    }

    /// A kind's own library and the shared one; never the other kind's.
    #[test]
    fn a_kind_searches_its_own_nodes_and_the_shared_ones() {
        let find = |slug: &str| REGISTRY.iter().find(|d| d.slug == slug).unwrap();
        for (slug, video, solids) in [
            ("checkerboard", true, false),
            ("sphere", false, true),
            ("solidrender", false, true),
            ("output", true, true),
            ("mastergear", true, true),
            ("add", true, true),
            ("note", true, true),
        ] {
            assert_eq!(belongs(find(slug), WorkspaceKind::Video), video, "{slug}");
            assert_eq!(belongs(find(slug), WorkspaceKind::Solids), solids, "{slug}");
        }
    }

    /// Each starter is kinds that exist, cabled by ports that do.
    #[test]
    fn every_starter_names_real_ports() {
        for kind in WorkspaceKind::ALL {
            let s = starter(*kind);
            assert_eq!(s.cables.len() + 1, s.nodes.len(), "{kind:?}");
            for (i, (out, into)) in s.cables.iter().enumerate() {
                let from = REGISTRY.iter().find(|d| d.slug == s.nodes[i]).unwrap();
                let to = REGISTRY.iter().find(|d| d.slug == s.nodes[i + 1]).unwrap();
                assert!(
                    from.outputs.iter().any(|o| o.key == *out),
                    "{}.{out}",
                    from.slug
                );
                assert!(
                    to.inputs.iter().any(|p| p.key == *into),
                    "{}.{into}",
                    to.slug
                );
            }
            assert!(
                REGISTRY
                    .iter()
                    .find(|d| d.slug == *s.nodes.last().unwrap())
                    .unwrap()
                    .is_output,
                "{kind:?} ends in an Output"
            );
        }
    }
}
