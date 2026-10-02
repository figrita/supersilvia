// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the graph costs per frame, on a chain of N nodes.
//!
//! `drag` is the expensive one: it is `can_connect_within` for every candidate port against
//! the one `Reach` the drag keeps, which is what one frame of a cable drag does. Run with `cargo run --release --example graph_bench`;
//! a debug build measures the optimizer, not the code.

use emath::Pos2;
use supersilvia::graph::PortRef;
use supersilvia::{Graph, NodeId, nodes};

fn chain(n: usize) -> (Graph, Vec<NodeId>) {
    let mut g = Graph::new();
    let mut ids = Vec::new();
    let mut prev = nodes::add_to_graph(&mut g, "checkerboard", Pos2::ZERO).unwrap();
    ids.push(prev);
    for _ in 0..n {
        let z = nodes::add_to_graph(&mut g, "zoom", Pos2::ZERO).unwrap();
        g.connect(PortRef::new(prev, "output"), PortRef::new(z, "input"))
            .unwrap();
        ids.push(z);
        prev = z;
    }
    (g, ids)
}

fn main() {
    let frames = 60;
    for n in [16usize, 64, 256, 1024, 4096] {
        let (g, ids) = chain(n);
        let src = PortRef::new(ids[0], "output");
        let targets: Vec<PortRef> = ids.iter().map(|i| PortRef::new(*i, "input")).collect();

        let reach = g.reach(src);
        let t = std::time::Instant::now();
        for _ in 0..frames {
            for to in &targets {
                let _ = g.can_connect_within(&reach, src, *to);
            }
        }
        let drag = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);

        // The frame-job path: topological order every frame.
        let t = std::time::Instant::now();
        for _ in 0..frames {
            let _ = g.topological_order().len();
        }
        let topo = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);

        // What the canvas does per frame: which input is fed, for every port.
        let t = std::time::Instant::now();
        for _ in 0..frames {
            for to in &targets {
                let _ = g.source_of(*to);
            }
        }
        let src_of = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);

        println!(
            "{n:4} nodes | drag {drag:7.3} ms | topo {topo:7.4} ms | source_of {src_of:7.3} ms"
        );
    }
}
