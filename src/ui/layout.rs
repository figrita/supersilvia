// SPDX-License-Identifier: AGPL-3.0-or-later

//! Automatic node arrangement: topological rank to column, column to a stack of nodes.
//!
//! Pure. It reads a graph and returns positions; nothing here mutates, and the caller turns
//! the result into one `Command::AutoArrange` step.

use crate::graph::{Graph, NodeId, PortType, WorkspaceId};
use crate::ui::canvas;
use emath::Pos2;
use std::collections::HashMap;

/// Space above the first node in a column and below the last.
pub const VERTICAL_MARGIN: f32 = 50.0;
/// Space either side of a column, so a column's pitch is this twice plus its widest node.
pub const HORIZONTAL_MARGIN: f32 = 50.0;
/// A column packs no tighter than this between nodes, and spreads no wider.
const MIN_SPACING: f32 = 10.0;
const MAX_SPACING: f32 = 30.0;

/// Where every node on one workspace goes, for a strip of this height.
///
/// The height is an argument rather than something read from the viewport, because it is
/// what makes the result reproducible: the same command redone later must place nodes where
/// it placed them the first time.
///
/// Ranks come from the whole graph and only this workspace's nodes are placed: a node fed
/// from another workspace still lands to the right of what feeds it.
///
/// `measured` is the canvas's own heights for the value fields that have drawn, so a long note
/// takes the room it is drawn with.
pub fn arrange(
    graph: &Graph,
    measured: &canvas::Measured,
    workspace: WorkspaceId,
    height: f32,
) -> Vec<(NodeId, Pos2)> {
    let ranks = ranks(graph);
    let mut by_rank: Vec<(u32, Vec<NodeId>)> = Vec::new();
    let mut ordered: Vec<(u32, NodeId)> = graph
        .on_workspace(workspace)
        .filter_map(|(id, _)| Some((*ranks.get(&id)?, id)))
        .collect();
    ordered.sort();
    for (rank, id) in ordered {
        match by_rank.last_mut() {
            Some((r, ids)) if *r == rank => ids.push(id),
            _ => by_rank.push((rank, vec![id])),
        }
    }

    let mut columns: Vec<Vec<NodeId>> = Vec::new();
    for (_, ids) in by_rank {
        columns.extend(split(graph, measured, &ids, height));
    }

    let mut placed = Vec::with_capacity(columns.iter().map(Vec::len).sum());
    let mut x = HORIZONTAL_MARGIN;
    for column in columns {
        let widest = column
            .iter()
            .filter_map(|id| graph.get(*id))
            .map(canvas::node_width)
            .fold(0.0_f32, f32::max);
        for (id, y) in distribute(graph, measured, &column, height) {
            placed.push((id, Pos2::new(x, y)));
        }
        x += 2.0 * HORIZONTAL_MARGIN + widest;
    }
    placed
}

/// A node's column index: zero when nothing feeds it, otherwise one past its deepest source.
///
/// Action edges do not count. An action cable is a CPU event rather than a value, so letting
/// one contribute would drag a sequencer into the column after the thing it triggers.
///
/// A cycle is ranked zero rather than refused. Feedback graphs are ordinary here.
fn ranks(graph: &Graph) -> HashMap<NodeId, u32> {
    let mut sources: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in graph.connections() {
        let ty = graph
            .get(c.to.node)
            .and_then(|n| n.input(c.to.key))
            .map(|p| p.ty);
        if ty == Some(PortType::Action) {
            continue;
        }
        sources.entry(c.to.node).or_default().push(c.from.node);
    }

    let mut ranks: HashMap<NodeId, u32> = HashMap::new();
    for (id, _) in graph.iter() {
        rank_of(id, &sources, &mut ranks, &mut Vec::new());
    }
    ranks
}

fn rank_of(
    id: NodeId,
    sources: &HashMap<NodeId, Vec<NodeId>>,
    ranks: &mut HashMap<NodeId, u32>,
    stack: &mut Vec<NodeId>,
) -> u32 {
    if let Some(r) = ranks.get(&id) {
        return *r;
    }
    if stack.contains(&id) {
        ranks.insert(id, 0);
        return 0;
    }
    stack.push(id);
    let rank = match sources.get(&id) {
        None => 0,
        Some(from) => {
            let deepest = from
                .iter()
                .map(|src| rank_of(*src, sources, ranks, stack))
                .max();
            deepest.map_or(0, |d| d + 1)
        }
    };
    stack.pop();
    ranks.insert(id, rank);
    rank
}

/// How tall one node is drawn, or nothing for an id the graph does not hold.
fn node_height(graph: &Graph, measured: &canvas::Measured, id: NodeId) -> f32 {
    canvas::Layouts::one(graph, id, measured.of(id))
        .find(id)
        .map_or(0.0, |l| l.height)
}

/// Break one rank into as many columns as it takes for each to fit the strip's height.
fn split(
    graph: &Graph,
    measured: &canvas::Measured,
    ids: &[NodeId],
    height: f32,
) -> Vec<Vec<NodeId>> {
    let mut columns = Vec::new();
    let mut current: Vec<NodeId> = Vec::new();
    let mut used = VERTICAL_MARGIN;
    for id in ids {
        let node_height = node_height(graph, measured, *id);
        let spacing = if current.is_empty() { 0.0 } else { MIN_SPACING };
        if used + spacing + node_height > height && !current.is_empty() {
            columns.push(std::mem::take(&mut current));
            used = VERTICAL_MARGIN + node_height;
            current.push(*id);
        } else {
            used += spacing + node_height;
            current.push(*id);
        }
    }
    if !current.is_empty() {
        columns.push(current);
    }
    columns
}

/// Stack one column, spreading into the space left over up to `MAX_SPACING`.
fn distribute(
    graph: &Graph,
    measured: &canvas::Measured,
    ids: &[NodeId],
    height: f32,
) -> Vec<(NodeId, f32)> {
    let heights: Vec<f32> = ids
        .iter()
        .map(|id| node_height(graph, measured, *id))
        .collect();
    let spacing = if ids.len() < 2 {
        0.0
    } else {
        let total: f32 = heights.iter().sum();
        let left = height - total - VERTICAL_MARGIN;
        (left / (ids.len() - 1) as f32).clamp(MIN_SPACING, MAX_SPACING)
    };
    let mut y = VERTICAL_MARGIN;
    let mut out = Vec::with_capacity(ids.len());
    for (id, h) in ids.iter().zip(heights) {
        out.push((*id, y));
        y += h + spacing;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::graph::PortRef;

    fn app_with(slugs: &[&'static str]) -> (crate::App, Vec<NodeId>) {
        let mut app = crate::App::headless();
        let ids = slugs
            .iter()
            .map(|slug| {
                let workspace = app.graph().default_workspace();
                app.apply(Command::AddNode {
                    slug,
                    at: Pos2::ZERO,
                    workspace,
                })
                .expect("a registry slug");
                app.graph().iter().map(|(id, _)| id).last().unwrap()
            })
            .collect();
        (app, ids)
    }

    fn connect(app: &mut crate::App, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
        app.apply(Command::Connect {
            from: PortRef {
                node: from.0,
                key: from.1,
            },
            to: PortRef {
                node: to.0,
                key: to.1,
            },
        })
        .expect("a legal connection");
    }

    /// The workspace an `App::headless` graph puts everything on.
    fn ws(app: &crate::App) -> WorkspaceId {
        app.graph().default_workspace()
    }

    #[test]
    fn a_chain_ranks_one_per_hop() {
        let (mut app, ids) = app_with(&["checkerboard", "edgedetection", "output"]);
        connect(&mut app, (ids[0], "output"), (ids[1], "input"));
        connect(&mut app, (ids[1], "output"), (ids[2], "input"));

        let ranks = ranks(app.graph());
        assert_eq!(ranks[&ids[0]], 0);
        assert_eq!(ranks[&ids[1]], 1);
        assert_eq!(ranks[&ids[2]], 2);
    }

    /// The longest path, not the shortest: a node waits for its deepest source.
    #[test]
    fn a_diamond_ranks_by_the_longest_path() {
        let (mut app, ids) = app_with(&["checkerboard", "edgedetection", "mix", "output"]);
        connect(&mut app, (ids[0], "output"), (ids[1], "input"));
        connect(&mut app, (ids[0], "output"), (ids[2], "a"));
        connect(&mut app, (ids[1], "output"), (ids[2], "b"));

        let ranks = ranks(app.graph());
        assert_eq!(ranks[&ids[2]], 2, "mix waits for the longer arm");
    }

    /// A feedback loop is the normal case in a video synth, so ranking terminates on one.
    #[test]
    fn a_feedback_loop_terminates() {
        let (mut app, ids) = app_with(&["output", "output"]);
        connect(&mut app, (ids[0], "frame"), (ids[1], "input"));
        connect(&mut app, (ids[1], "frame"), (ids[0], "input"));

        let ranks = ranks(app.graph());
        assert_eq!(ranks.len(), 2, "every node is ranked");
    }

    #[test]
    fn a_rank_taller_than_the_strip_becomes_two_columns() {
        let (app, ids) = app_with(&["checkerboard", "checkerboard", "checkerboard"]);
        let one = node_height(app.graph(), &canvas::Measured::default(), ids[0]);

        let short = VERTICAL_MARGIN + one * 2.0 + MIN_SPACING;
        let placed = arrange(app.graph(), &canvas::Measured::default(), ws(&app), short);
        let xs: std::collections::BTreeSet<i32> = placed.iter().map(|(_, p)| p.x as i32).collect();
        assert_eq!(xs.len(), 2, "three nodes, room for two: two columns");

        let tall = VERTICAL_MARGIN + one * 3.0 + MIN_SPACING * 2.0;
        let placed = arrange(app.graph(), &canvas::Measured::default(), ws(&app), tall);
        let xs: std::collections::BTreeSet<i32> = placed.iter().map(|(_, p)| p.x as i32).collect();
        assert_eq!(xs.len(), 1, "room for all three: one column");
    }

    /// Redo replays the command, so the same height must give the same answer.
    #[test]
    fn arranging_twice_gives_the_same_positions() {
        let (mut app, ids) = app_with(&["checkerboard", "edgedetection", "output"]);
        connect(&mut app, (ids[0], "output"), (ids[1], "input"));
        connect(&mut app, (ids[1], "output"), (ids[2], "input"));

        assert_eq!(
            arrange(app.graph(), &canvas::Measured::default(), ws(&app), 800.0),
            arrange(app.graph(), &canvas::Measured::default(), ws(&app), 800.0)
        );
    }
}
