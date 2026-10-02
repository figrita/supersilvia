// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: what a probe's counts become on the cost strip and the header warning.
//!
//! The GPU half — that the probe counts exactly — is layer 3's. Here: the arithmetic from
//! a count over the probe's few pixels to evaluations per pixel and per frame of the real
//! Output, and that the probe itself runs whether or not View ▸ Costs is on — the strip is
//! a preference, the reading underneath it is not, because a node's own header warns off it
//! too.

use emath::Pos2;
use supersilvia::compile::{EVAL_WORD, TAP_WORDS};
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::render::PROBE;
use supersilvia::{App, Command};

fn add(app: &mut App, slug: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

#[test]
fn a_probe_count_becomes_evaluations_per_pixel_and_per_frame() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let blur = add(&mut app, "blur");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(blur, "input"),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(blur, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let job = app.build_frame_job();
    assert_eq!(
        job.probes.len(),
        1,
        "the probe runs whether or not the view is on"
    );
    let probe = &job.probes[0];
    assert!(probe.shader.is_some(), "sent on the frame it was built");
    assert_eq!(
        probe.taps, 4,
        "the checkerboard, the blur's call of it, the blur, and the Output's call of the blur"
    );

    // What the GPU would hand back: the blur once per probe pixel, its input nine times,
    // the call site between them nine times, and the Output's call once. Slots are in the
    // order the compiler met them.
    let pixels = PROBE.0 * PROBE.1;
    let mut words = vec![0u32; probe.taps * TAP_WORDS];
    for (slot, per_pixel) in [9, 9, 1, 1].into_iter().enumerate() {
        words[slot * TAP_WORDS + EVAL_WORD] = per_pixel * pixels;
    }
    app.ingest_probe(out, &words);

    let (per_px, total) = app.evaluations(cb).expect("counted");
    assert!((per_px - 9.0).abs() < 1e-9, "{per_px}");
    assert!((total - 9.0 * 1280.0 * 720.0).abs() < 1.0, "{total}");
    let (per_px, _) = app.evaluations(blur).expect("counted");
    assert!((per_px - 1.0).abs() < 1e-9, "{per_px}");
    assert_eq!(app.taps(blur), Some(9.0), "the cause is on the blur");
    assert_eq!(app.taps(cb), None, "a source taps nothing");

    // The strip's preference moves neither the probe nor the reading under it: the
    // checkerboard's header still has to warn with the view off, and it can only do that
    // from a reading that survives. Both legs, since the store starts with the view off.
    for on in [true, false] {
        app.set_show_costs(on);
        let job = app.build_frame_job();
        assert_eq!(
            job.probes.len(),
            1,
            "view {on}: the preference is on the strip, not on the probe"
        );
        assert!(
            app.evaluations(cb).is_some(),
            "view {on}: the reading it already made survives"
        );
    }
}

/// A deleted Output stops feeding the header warning it was the only measurement behind.
///
/// `probes`, `evaluations` and `calls` are keyed by the Output whose pass counted them, and
/// `build_frame_job` only prunes an Output it still walks. A deleted one never reaches that
/// loop, so its last reading would sit there for the rest of the session, warning about taps
/// nothing performs any more.
#[test]
fn deleting_an_output_drops_the_reading_its_pass_made() {
    let mut app = App::headless();
    let cb = add(&mut app, "checkerboard");
    let blur = add(&mut app, "blur");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(blur, "input"),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(blur, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let job = app.build_frame_job();
    let probe = &job.probes[0];
    let pixels = PROBE.0 * PROBE.1;
    let mut words = vec![0u32; probe.taps * TAP_WORDS];
    for (slot, per_pixel) in [9, 9, 1, 1].into_iter().enumerate() {
        words[slot * TAP_WORDS + EVAL_WORD] = per_pixel * pixels;
    }
    app.ingest_probe(out, &words);
    assert_eq!(
        app.taps(blur),
        Some(9.0),
        "nine is over the threshold: the blur's header warns"
    );

    app.apply(Command::RemoveNodes(vec![out])).unwrap();
    app.tick(1.0 / 60.0);
    assert_eq!(
        app.taps(blur),
        None,
        "the reading outlived the Output whose pass made it"
    );
    assert_eq!(app.evaluations(blur), None, "and so did its evaluations");
}
