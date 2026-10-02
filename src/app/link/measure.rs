// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the GPU reports back: cost probes, dropped frames and the ticks that waited for it.
//!
//! Nothing here waits on the GPU. A reading is collected a frame or two after it was asked
//! for, and the node says so rather than the frame stalling for it.

use super::SynthLink;
use crate::compile;
use crate::graph::{Graph, NodeId};
use std::collections::HashMap;

/// A running count from the renderer as a rate a second: an Output's dropped frames, or the
/// ticks that waited for the GPU.
///
/// The count is cumulative and a reader wants a rate, so it is sampled once a second and the
/// difference is what the second cost. A second's figure stays up for the second after, which
/// is long enough to read.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(in crate::app) struct Rate {
    /// The clock time and count at the start of the second being measured.
    since: f64,
    count: u64,
    /// What the last whole second measured.
    last: f32,
}

impl Rate {
    const WINDOW: f64 = 1.0;

    fn observe(&mut self, now: f64, count: u64) {
        if now - self.since >= Self::WINDOW {
            let span = (now - self.since).max(f64::EPSILON);
            self.last = if self.since == 0.0 && self.count == 0 && self.last == 0.0 {
                // The first sample begins the window rather than reporting every drop
                // since the renderer was made as a rate over the app's whole life.
                0.0
            } else {
                (count.saturating_sub(self.count) as f64 / span) as f32
            };
            self.since = now;
            self.count = count;
        }
    }

    pub(in crate::app) fn per_second(&self) -> f32 {
        self.last
    }
}

/// A count of evaluations as a short figure: `8.3M`, `921k`, `144`.
fn count_text(n: f64) -> String {
    if n >= 1.0e6 {
        format!("{:.1}M", n / 1.0e6)
    } else if n >= 1.0e3 {
        format!("{:.0}k", n / 1.0e3)
    } else {
        format!("{n:.0}")
    }
}

/// An Output's own resolution, which a probe's count is scaled to.
fn pixels_of(graph: &Graph, output: NodeId) -> f64 {
    let (w, h) = graph
        .get(output)
        .map_or(crate::nodes::output::DEFAULT_RESOLUTION, |n| {
            crate::nodes::output::resolution_of(n)
        });
    f64::from(w) * f64::from(h)
}

impl SynthLink {
    /// Collect what the probes counted, and turn each count into evaluations per frame.
    pub(in crate::app) fn collect_probes(&mut self, graph: &Graph) {
        for (_, probes) in self.snapshot.events.probes.unseen("ticks of probe counts") {
            for (output, words) in probes {
                ingest(&mut self.outputs, graph, *output, words);
            }
        }
    }

    /// Turn each Output's dropped-frame count, and the count of ticks that waited for the
    /// GPU, into a rate, once a second, while something shows one.
    pub(in crate::app) fn track_drops(&mut self, graph: &Graph, shown: bool) {
        if !shown {
            for link in self.outputs.values_mut() {
                link.drops = Rate::default();
            }
            self.waits = Rate::default();
            return;
        }
        if !self.snapshot.render.has_gpu {
            return;
        }
        let now = self.clock().elapsed();
        self.waits.observe(now, self.snapshot.render.gpu_waits);
        for (id, node) in graph.iter() {
            if !node.def.is_output {
                continue;
            }
            let count = self
                .snapshot
                .render
                .dropped_frames
                .get(&id)
                .copied()
                .unwrap_or(0);
            self.outputs
                .entry(id)
                .or_default()
                .drops
                .observe(now, count);
        }
    }

    /// One Output's dropped frames per second, as of the last whole second.
    pub(in crate::app) fn drops_per_second(&self, output: NodeId) -> f32 {
        self.outputs
            .get(&output)
            .map_or(0.0, |link| link.drops.per_second())
    }

    /// The worst of every Output's.
    pub(in crate::app) fn worst_drops_per_second(&self) -> f32 {
        self.outputs
            .values()
            .map(|link| link.drops.per_second())
            .fold(0.0, f32::max)
    }

    /// Ticks a second that waited for the GPU, as of the last whole second.
    pub(in crate::app) fn waits_per_second(&self) -> f32 {
        self.waits.per_second()
    }

    /// One probe's words, as the renderer read them back: each slot's count, over the
    /// probe's few pixels, scaled to the Output's own.
    pub(in crate::app) fn ingest_probe(&mut self, graph: &Graph, output: NodeId, words: &[u32]) {
        ingest(&mut self.outputs, graph, output, words);
    }

    /// How many times each node runs an input per run of its own — its busiest input's,
    /// over every Output it is compiled into. A node with no connected function input, or
    /// one no probe has counted, is absent.
    ///
    /// One walk of the probes' counts for every reader, rather than one per node asked
    /// about: the strip's `×9 taps` prefix and the header warning are then the same figure
    /// because they read the same entry, not because two functions agree.
    pub(in crate::app) fn taps_by_node(&self) -> HashMap<NodeId, f64> {
        let mut by_site: HashMap<(NodeId, &'static str), (f64, f64)> = HashMap::new();
        for link in self.outputs.values() {
            for ((node, key), calls) in &link.calls {
                let Some(evals) = link.evaluations.get(node) else {
                    continue;
                };
                let entry = by_site.entry((*node, *key)).or_insert((0.0, 0.0));
                entry.0 += calls;
                entry.1 += evals;
            }
        }
        let mut taps: HashMap<NodeId, f64> = HashMap::new();
        for ((node, _), (calls, evals)) in by_site {
            if evals > 0.0 {
                let per_run = calls / evals;
                taps.entry(node)
                    .and_modify(|busiest| *busiest = busiest.max(per_run))
                    .or_insert(per_run);
            }
        }
        taps
    }

    /// How often a node's function runs: per pixel of the Outputs it is compiled into, and
    /// per frame in all. `None` until a probe has counted it.
    pub(in crate::app) fn evaluations(&self, graph: &Graph, node: NodeId) -> Option<(f64, f64)> {
        let mut total = 0.0;
        let mut pixels = 0.0;
        let mut seen = false;
        for (output, link) in &self.outputs {
            let Some(evals) = link.evaluations.get(&node) else {
                continue;
            };
            total += evals;
            pixels += pixels_of(graph, *output);
            seen = true;
        }
        seen.then(|| (total / pixels.max(1.0), total))
    }

    /// Nodes whose measured taps of their own input earn a header warning, mapped to that
    /// count — regardless of View ▸ Costs, since the probe that measures it always runs.
    /// The same figure the strip's `×9 taps` prefix reads off `taps`; silvia declares this
    /// per input, so the threshold is checked once here rather than once per row.
    /// See docs/decisions.md#a-header-warns-on-what-the-probe-measures-not-on-the-view.
    pub(in crate::app) fn sampling_warnings(
        graph: &Graph,
        taps: &HashMap<NodeId, f64>,
    ) -> HashMap<NodeId, f64> {
        const THRESHOLD: f64 = 8.0;
        graph
            .iter()
            .filter_map(|(id, _)| {
                let taps = *taps.get(&id)?;
                (taps > THRESHOLD).then_some((id, taps))
            })
            .collect()
    }

    /// The strip under every node, while the cost view is on: an Output's GPU time against
    /// one interval of the display, and every other node's evaluations against the busiest
    /// node's.
    pub(in crate::app) fn cost_strips(
        &self,
        graph: &Graph,
        taps: &HashMap<NodeId, f64>,
        refresh_ms: f32,
    ) -> HashMap<NodeId, crate::ui::Cost> {
        let mut costs = HashMap::new();
        let gpu_times = &self.snapshot.render.gpu_times;
        let busiest = graph
            .iter()
            .filter_map(|(id, _)| self.evaluations(graph, id))
            .map(|(_, total)| total)
            .fold(0.0_f64, f64::max);
        for (id, node) in graph.iter() {
            let cost = if node.def.is_output {
                let Some(gpu) = gpu_times.get(&id).copied() else {
                    continue;
                };
                let dropped = self.drops_per_second(id);
                // `latest / worst ms`, and the drop count only where there is one. The
                // budget the two are measured against is the strip itself: the bar's full
                // width is one interval of the display the window is on, so writing the
                // figure out would draw the same fact twice.
                crate::ui::Cost {
                    left: format!("{:.1} / {:.1} ms", gpu.latest, gpu.worst),
                    right: if dropped > 0.0 {
                        format!("{dropped:.0} drop/s")
                    } else {
                        String::new()
                    },
                    fraction: gpu.latest / refresh_ms,
                    hot: gpu.latest > refresh_ms || dropped > 0.0,
                }
            } else {
                let Some((per_px, total)) = self.evaluations(graph, id) else {
                    continue;
                };
                // The cause before the effect: a node that taps its input nine times says
                // so on its own strip, and the ninefold shows on the node it fell on.
                let taps = match taps.get(&id) {
                    Some(t) if *t > 1.001 => format!("×{t:.0} taps"),
                    _ => String::new(),
                };
                // Evaluations per pixel is a count and is written as one. It is a ratio
                // underneath — the evaluations over the pixels of every Output this node
                // reaches — so a node feeding two Outputs of different sizes at different
                // tap counts lands between two integers, and is rounded to the nearer.
                crate::ui::Cost {
                    left: taps,
                    right: format!("{per_px:.0}/px {}", count_text(total)),
                    fraction: (total / busiest.max(1.0)) as f32,
                    hot: false,
                }
            };
            costs.insert(id, cost);
        }
        costs
    }
}

/// [`SynthLink::ingest_probe`], on the one field it writes.
fn ingest(
    outputs: &mut HashMap<NodeId, super::OutputLink>,
    graph: &Graph,
    output: NodeId,
    words: &[u32],
) {
    let Some(link) = outputs.get_mut(&output) else {
        return;
    };
    let Some(shader) = &link.probe else {
        return;
    };
    let pixels = pixels_of(graph, output);
    let probed = f64::from(crate::render::PROBE.0) * f64::from(crate::render::PROBE.1);
    let mut per_node = HashMap::new();
    let mut per_site = HashMap::new();
    for (i, (node, kind)) in shader.taps.iter().enumerate() {
        let Some(count) = words.get(i * compile::TAP_WORDS + compile::EVAL_WORD) else {
            continue;
        };
        let evals = f64::from(*count) / probed * pixels;
        match kind {
            compile::TapKind::Taps(key) => {
                *per_site.entry((*node, *key)).or_insert(0.0) += evals;
            }
            _ => *per_node.entry(*node).or_insert(0.0) += evals,
        }
    }
    link.evaluations = per_node;
    link.calls = per_site;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate is the drops in the last whole second, and the first sample only starts
    /// the clock: a renderer that dropped a hundred frames before the view came on does
    /// not report them as this second's.
    #[test]
    fn a_drop_rate_is_the_last_seconds_drops() {
        let mut r = Rate::default();
        r.observe(10.0, 100);
        assert_eq!(r.per_second(), 0.0, "the first sample begins the window");
        r.observe(10.5, 130);
        assert_eq!(r.per_second(), 0.0, "half a second is not a reading");
        r.observe(11.0, 130);
        assert_eq!(r.per_second(), 30.0);
        r.observe(12.0, 130);
        assert_eq!(r.per_second(), 0.0, "a quiet second reads zero");
    }
}
