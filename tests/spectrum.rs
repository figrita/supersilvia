// SPDX-License-Identifier: AGPL-3.0-or-later

//! The spectrum as a list, through the whole tick: an audio source's Spectrum reaches the List
//! nodes in the frame it is published, Bands gathers it, and Lag eases a list and starts again
//! with its node.
//!
//! No device is opened. A Main Input with no source chosen and a Video with no file are
//! silence, which is 128 buckets of zero; the analysis behind a real signal is the pure
//! `Analyzer`'s, tested in `src/audio/`.

use emath::Pos2;
use supersilvia::graph::{ControlValue, NodeId, PortRef, PortType};
use supersilvia::nodes;
use supersilvia::{App, Command};

fn added(app: &mut App, slug: &'static str) -> NodeId {
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace,
    })
    .expect("slug is in the registry");
    app.graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("just added")
}

fn set(app: &mut App, node: NodeId, key: &'static str, value: f32) {
    app.apply(Command::SetControl {
        node,
        key,
        value: ControlValue::Float(value),
    })
    .unwrap();
}

fn connect(app: &mut App, from: PortRef, to: PortRef) {
    app.apply(Command::Connect { from, to }).unwrap();
}

fn list(app: &App, port: PortRef) -> Option<Vec<f64>> {
    app.snapshot().lists.get(&port).cloned()
}

/// Every audio source declares the one Spectrum, a number list.
#[test]
fn every_audio_source_has_a_spectrum_list() {
    for slug in ["audioin", "maininput", "video"] {
        let def = nodes::find(slug).unwrap();
        let out = def.output("spectrum").expect("a Spectrum output");
        assert_eq!(out.ty, PortType::ListNumber, "{slug}");
        assert_eq!(out.label, "Spectrum", "{slug}");
    }
}

/// A Main Input's Spectrum, silent with no source chosen, is 128 items, read by a Length, a
/// Get and a Bands in the frame it is published.
#[test]
fn the_spectrum_reaches_length_get_and_bands() {
    for source in ["maininput", "video"] {
        let mut app = App::headless();
        let input = added(&mut app, source);
        let length = added(&mut app, "length");
        let get = added(&mut app, "get");
        let bands = added(&mut app, "bands");
        let counted = added(&mut app, "length");
        let spectrum = PortRef::new(input, "spectrum");
        connect(&mut app, spectrum, PortRef::new(length, "list"));
        connect(&mut app, spectrum, PortRef::new(get, "list"));
        connect(&mut app, spectrum, PortRef::new(bands, "spectrum"));
        connect(
            &mut app,
            PortRef::new(bands, "bands"),
            PortRef::new(counted, "list"),
        );
        set(&mut app, get, "index", 100.0);
        app.tick(1.0 / 60.0);

        assert_eq!(list(&app, spectrum), Some(vec![0.0; 128]), "{source}");
        assert_eq!(
            app.uniform(PortRef::new(length, "count")),
            Some(128.0),
            "{source}"
        );
        assert_eq!(
            app.uniform(PortRef::new(get, "item")),
            Some(0.0),
            "{source}"
        );
        assert_eq!(
            app.uniform(PortRef::new(counted, "count")),
            Some(16.0),
            "{source}: sixteen bands by default"
        );
    }
}

/// Bands over a Range standing in for a spectrum, 0 to 1 over 128 items: a ramp averaged
/// into its halves.
#[test]
fn bands_reads_whatever_list_is_cabled_in() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let bands = added(&mut app, "bands");
    set(&mut app, range, "count", 128.0);
    set(&mut app, bands, "high", 20_000.0);
    set(&mut app, bands, "count", 2.0);
    connect(
        &mut app,
        PortRef::new(range, "list"),
        PortRef::new(bands, "spectrum"),
    );
    app.tick(1.0 / 60.0);
    let out = list(&app, PortRef::new(bands, "bands")).unwrap();
    assert_eq!(out.len(), 2);
    assert!((out[0] - 31.5 / 127.0).abs() < 1.0e-6, "{out:?}");
    assert!((out[1] - 95.5 / 127.0).abs() < 1.0e-6, "{out:?}");
}

/// Lag on a list whose items jump: the first frame starts at the input, the next eases toward
/// the jump on the frame's `dt`, a longer list keeps the eased items and starts the new ones
/// at their input, and a reset forgets them all.
#[test]
fn lag_eases_keeps_its_items_and_resets() {
    let mut app = App::headless();
    let range = added(&mut app, "range");
    let lag = added(&mut app, "lag");
    set(&mut app, range, "count", 2.0);
    set(&mut app, range, "from", 0.0);
    set(&mut app, range, "to", 0.0);
    set(&mut app, lag, "rise", 0.1);
    connect(
        &mut app,
        PortRef::new(range, "list"),
        PortRef::new(lag, "list"),
    );
    let out = PortRef::new(lag, "output");
    app.tick(0.1);
    assert_eq!(list(&app, out), Some(vec![0.0, 0.0]));

    set(&mut app, range, "from", 1.0);
    set(&mut app, range, "to", 1.0);
    app.tick(0.1);
    let eased = list(&app, out).unwrap();
    let up = 1.0 - (-1.0f64).exp();
    assert!(
        eased.iter().all(|v| (v - up).abs() < 1.0e-3),
        "a rise of 0.1 s covers 63% in 0.1 s: {eased:?}"
    );

    set(&mut app, range, "count", 3.0);
    app.tick(0.0);
    let grown = list(&app, out).unwrap();
    assert_eq!(grown.len(), 3);
    assert!((grown[0] - eased[0]).abs() < 1.0e-9, "{grown:?}");
    assert_eq!(grown[2], 1.0, "a new item starts at its input");

    app.reset_cpu();
    set(&mut app, range, "from", 0.5);
    set(&mut app, range, "to", 0.5);
    app.tick(0.1);
    assert_eq!(list(&app, out), Some(vec![0.5; 3]), "a reset starts again");
}
