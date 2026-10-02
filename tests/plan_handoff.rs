// SPDX-License-Identifier: AGPL-3.0-or-later

//! A one-shot shader survives a plan the editor sent before the synth's next tick.
//!
//! The editor builds a plan every frame; the synth drains every waiting plan at the top of a
//! tick and draws from the last. A source carried by a plan in the middle of that drain must
//! reach the renderer — it was lost, and a fresh Output stayed black until the next recompile.

mod common;

use std::sync::Arc;
use supersilvia::compile::Shader;
use supersilvia::graph::NodeId;
use supersilvia::synth::{Msg, OutputPlan, Plan, ProbePlan, Synth};

/// A module whose source is `body`: all the handoff reads of one is which it is.
fn module(body: String) -> Arc<Shader> {
    Arc::new(Shader {
        body,
        uniforms: std::collections::BTreeMap::new(),
        diagnostics: Vec::new(),
        taps: Vec::new(),
        thumbs: Vec::new(),
        grid: 0,
    })
}

/// Which module a job carries, by its source.
fn body(shader: Option<&Arc<Shader>>) -> Option<&str> {
    shader.map(|s| s.body.as_str())
}

fn plan(node: NodeId, shader: Option<&str>, generation: u64) -> Box<Plan> {
    Box::new(Plan {
        generation,
        outputs: vec![OutputPlan {
            node,
            resolution: (16, 9),
            shader: shader.map(|s| module(s.to_string())),
            source: 0,
            mode: supersilvia::synth::Mode::Draw {
                why: supersilvia::synth::Why::Tab,
            },
            reads: Vec::new(),
            needs: Vec::new(),
            feeds_back: false,
            uniforms: Vec::new(),
            taps: 0,
        }],
        probes: vec![ProbePlan {
            output: node,
            shader: shader.map(|s| module(format!("probe {s}"))),
            source: 0,
            uniforms: Vec::new(),
            taps: 0,
        }],
        ..Plan::default()
    })
}

#[test]
fn a_shader_survives_a_plan_that_supersedes_it_before_the_tick() {
    let node = NodeId(7);
    let mut synth = Synth::default();
    synth.handle(Msg::Plan(plan(node, Some("fresh"), 1)));
    synth.handle(Msg::Plan(plan(node, None, 2)));
    synth.handle(Msg::Plan(plan(node, None, 3)));

    let job = synth.job();
    assert_eq!(body(job.outputs[0].shader.as_ref()), Some("fresh"));
    assert_eq!(body(job.probes[0].shader.as_ref()), Some("probe fresh"));

    // Taken once: a plan after the tick that consumed it carries nothing.
    synth.handle(Msg::Plan(plan(node, None, 4)));
    let job = synth.job();
    assert_eq!(job.outputs[0].shader, None);
    assert_eq!(job.probes[0].shader, None);
}

/// **A waiting shader is the project's that compiled it.** One the job has not taken when
/// another project's graph arrives is not carried into that project's plan, where the same id
/// is another Output: the renderer would link the old project's picture onto it.
#[test]
fn a_shader_waiting_when_another_project_opens_is_not_carried_into_it() {
    let node = NodeId(7);
    let mut synth = Synth::default();
    synth.handle(Msg::Plan(plan(node, Some("old project"), 1)));
    synth.handle(Msg::Graph {
        graph: std::sync::Arc::default(),
        project: 1,
    });
    synth.handle(Msg::Plan(plan(node, None, 2)));
    let job = synth.job();
    assert_eq!(job.outputs[0].shader, None);
    assert_eq!(job.probes[0].shader, None);

    // The same project's next graph is an edit, and the carry holds across it.
    synth.handle(Msg::Plan(plan(node, Some("fresh"), 3)));
    synth.handle(Msg::Graph {
        graph: std::sync::Arc::default(),
        project: 1,
    });
    synth.handle(Msg::Plan(plan(node, None, 4)));
    assert_eq!(body(synth.job().outputs[0].shader.as_ref()), Some("fresh"));
}

#[test]
fn a_newer_shader_wins_over_one_still_waiting() {
    let node = NodeId(7);
    let mut synth = Synth::default();
    synth.handle(Msg::Plan(plan(node, Some("old"), 1)));
    synth.handle(Msg::Plan(plan(node, Some("new"), 2)));
    let job = synth.job();
    assert_eq!(body(job.outputs[0].shader.as_ref()), Some("new"));
}

/// A shader on an Output the job suspends waits in the plan: the renderer ignores a
/// suspended Output's source, so taking it there would lose it.
#[test]
fn a_shader_waits_while_its_output_is_suspended() {
    let node = NodeId(7);
    let mut synth = Synth::default();
    let mut suspended = plan(node, Some("fresh"), 1);
    suspended.outputs[0].mode = supersilvia::synth::Mode::Suspended;
    synth.handle(Msg::Plan(suspended));
    let job = synth.job();
    assert_eq!(
        job.outputs[0].mode,
        supersilvia::render::OutputMode::Suspended
    );
    assert_eq!(job.outputs[0].shader, None);

    synth.handle(Msg::Plan(plan(node, None, 2)));
    let job = synth.job();
    assert_eq!(body(job.outputs[0].shader.as_ref()), Some("fresh"));
}

/// A render that holds suspends every Output in the job, and a shader that arrives during
/// the hold stays in the plan rather than going out on a suspended Output, which the renderer
/// ignores. Headless there is no GPU, so the render holds from its first tick.
#[test]
fn a_shader_sent_while_a_render_holds_is_not_spent_on_the_hold() {
    use common::{add_on, connect, picture_on, two_tabs};
    let (mut app, first, second) = two_tabs();
    let rendered = picture_on(&mut app, first);
    let x = picture_on(&mut app, second);
    let _ = app.build_frame_job();
    // An edit to `x` while its tab is closed, built when the tab comes back.
    app.close_workspace(second);
    let other = add_on(&mut app, "checkerboard", second);
    connect(&mut app, (other, "output"), (x, "input"));
    let _ = app.build_frame_job();

    let destination = std::env::temp_dir().join(format!("ssw-hold-shader-{}", std::process::id()));
    let mut settings = app.render_settings_of(rendered).expect("an Output");
    settings.destination = destination.clone();
    app.start_render(rendered, &settings)
        .expect("a connected Output renders");
    // The tab comes back during the render, and `x`'s rebuilt source goes out.
    app.open_workspace(second);
    let sent = app.sources_sent();
    let job = app.build_frame_job();
    assert!(app.sources_sent() > sent, "the rebuilt source is sent");
    let held = job
        .outputs
        .iter()
        .find(|o| o.node == x)
        .expect("in the job");
    assert_eq!(held.mode, supersilvia::render::OutputMode::Suspended);
    assert_eq!(held.shader, None, "kept for the first job that draws it");

    app.cancel_render();
    while app.rendering() {
        app.tick(1.0 / 60.0);
    }
    let _ = std::fs::remove_dir_all(&destination);
}
