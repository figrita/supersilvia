// SPDX-License-Identifier: AGPL-3.0-or-later

//! Milestone 8: a second Output, and feedback. §2b's claim is that Output nodes are the
//! only primitive that owns memory, and that their `frame` port is simultaneously the
//! screen, an intermediate buffer, and a feedback source. These are the tests for that.

use emath::Pos2;
use supersilvia::compile::{self, UniformProvider};
use supersilvia::graph::{ConnectError, Graph, NodeId, PortRef};
use supersilvia::nodes;

fn add(g: &mut Graph, slug: &str) -> NodeId {
    nodes::add_to_graph(g, slug, Pos2::ZERO).expect("slug is registered")
}

fn wire(g: &mut Graph, from: (NodeId, &'static str), to: (NodeId, &'static str)) {
    g.connect(PortRef::new(from.0, from.1), PortRef::new(to.0, to.1))
        .unwrap_or_else(|e| panic!("{from:?} -> {to:?}: {e}"));
}

// ---------------------------------------------------------------- A -> B

#[test]
fn one_output_can_feed_another() {
    // checkerboard -> A, then A.frame -> B. B samples A's texture rather than recompiling
    // A's whole chain into its own shader: this is silvia's only multi-pass mechanism.
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    wire(&mut g, (cb, "output"), (a, "input"));
    wire(&mut g, (a, "frame"), (b, "input"));

    let shader_a = compile::wgsl::build(&g, a).expect("A is connected");
    let shader_b = compile::wgsl::build(&g, b).expect("B is connected");

    assert!(shader_a.body.contains("checkerboard1_output"));
    assert!(
        !shader_b.body.contains("checkerboard1_output"),
        "B must sample A's frame, not inline A's tree",
    );
    assert_eq!(
        shader_b.uniforms["u_texture_output2_frame"],
        UniformProvider::NodeTexture {
            node: a,
            port: "frame"
        },
    );
}

#[test]
fn a_producer_output_is_ordered_before_its_consumer() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    wire(&mut g, (cb, "output"), (a, "input"));
    wire(&mut g, (a, "frame"), (b, "input"));

    let order = g.topological_order().to_vec();
    let at = |n| order.iter().position(|x| *x == n).unwrap();
    // B must render after A in the same frame, so it shows A's current frame and not the
    // previous one. This is the thing silvia's independent rAF loops cannot guarantee.
    assert!(at(a) < at(b));
}

#[test]
fn rewiring_a_frame_port_recompiles_the_consumer() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let a = add(&mut g, "output");
    let b = add(&mut g, "output");
    wire(&mut g, (cb, "output"), (a, "input"));
    wire(&mut g, (a, "frame"), (b, "input"));

    // Changing anything upstream of A does not recompile B — B only samples A's texture.
    // But the frame edge itself is a dependency, so A's own rebuild set includes B.
    let mut downstream = g.downstream_outputs(cb);
    downstream.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(downstream, want);
}

// ---------------------------------------------------------------- feedback

#[test]
fn an_output_can_feed_back_into_its_own_tree() {
    // checkerboard -> mix.a, A.frame -> mix.b, mix -> A.input.
    // The loop closes through A's frame port, which reads a finished frame, so it is
    // feedback rather than infinite recursion. §2b: these cycles are legal.
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let mix = add(&mut g, "mix");
    let a = add(&mut g, "output");

    wire(&mut g, (cb, "output"), (mix, "a"));
    wire(&mut g, (mix, "output"), (a, "input"));
    // The edge that closes the loop.
    wire(&mut g, (a, "frame"), (mix, "b"));

    assert_eq!(g.connections().len(), 3);
}

#[test]
fn an_immediate_cycle_is_still_rejected() {
    // The same shape without a frame port in it would make the compiler recurse forever.
    let mut g = Graph::new();
    let one = add(&mut g, "mix");
    let two = add(&mut g, "mix");
    wire(&mut g, (one, "output"), (two, "a"));

    assert_eq!(
        g.connect(PortRef::new(two, "output"), PortRef::new(one, "a")),
        Err(ConnectError::WouldCycle { from: two, to: one }),
    );
}

#[test]
fn a_feedback_patch_compiles_and_terminates() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let mix = add(&mut g, "mix");
    let a = add(&mut g, "output");
    wire(&mut g, (cb, "output"), (mix, "a"));
    wire(&mut g, (mix, "output"), (a, "input"));
    wire(&mut g, (a, "frame"), (mix, "b"));

    // If the compiler descended through the frame port this would never return.
    let shader = compile::wgsl::build(&g, a).expect("A is connected");

    // The loop resolves to a texture sample of A's own published frame.
    assert_eq!(
        shader.uniforms["u_texture_output3_frame"],
        UniformProvider::NodeTexture {
            node: a,
            port: "frame"
        },
    );
    assert!(shader.body.contains("checkerboard1_output"));
    assert!(shader.body.contains("mix2_output"));
    // One emission each, even though the graph has a loop in it.
    assert_eq!(
        shader
            .body
            .matches("fn mix2_output(uv: vec2f) -> vec4f")
            .count(),
        1
    );
    assert_eq!(
        shader
            .body
            .matches("fn output3_frame(uv: vec2f) -> vec4f")
            .count(),
        1
    );

    insta::assert_snapshot!(shader.body);
}

#[test]
fn a_feedback_loop_still_yields_a_total_order() {
    let mut g = Graph::new();
    let cb = add(&mut g, "checkerboard");
    let mix = add(&mut g, "mix");
    let a = add(&mut g, "output");
    wire(&mut g, (cb, "output"), (mix, "a"));
    wire(&mut g, (mix, "output"), (a, "input"));
    wire(&mut g, (a, "frame"), (mix, "b"));

    let order = g.topological_order().to_vec();
    // Every node appears exactly once, even though Kahn cannot place the loop members.
    assert_eq!(order.len(), 3);
    let mut sorted = order.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 3);

    // The break is deterministic: same graph, same order, every time.
    let mut again = Graph::new();
    let cb2 = add(&mut again, "checkerboard");
    let mix2 = add(&mut again, "mix");
    let a2 = add(&mut again, "output");
    wire(&mut again, (cb2, "output"), (mix2, "a"));
    wire(&mut again, (mix2, "output"), (a2, "input"));
    wire(&mut again, (a2, "frame"), (mix2, "b"));
    let order2 = again.topological_order().to_vec();
    assert_eq!(order, order2);
}
