// SPDX-License-Identifier: AGPL-3.0-or-later

//! The video node, driven through a headless `App`: import, free-running play in both
//! directions, and scrubbing from a uniform number. The file is a synthetic clip made
//! here; the transcode into the cache and the player are the real ones.

use emath::Pos2;
use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use supersilvia::graph::{ControlValue, NodeId, PortRef};
use supersilvia::nodes::Frame;
use supersilvia::video::clip::Codec;
use supersilvia::{App, Command};

/// One tick of the transport at sixty hertz, as the Main Input's clip is handed one.
const FRAME_TIME: supersilvia::transport::Time = supersilvia::transport::Time {
    playhead: 0.0,
    advance: 1.0 / 60.0,
    jumped: false,
    landed: None,
    playing: true,
};

/// The machine's hardware codec pair, which the clips here are encoded with because it is
/// what an import transcodes with — or `None`, having said the test is skipped.
fn codec() -> Option<Codec> {
    let codec = Codec::probe();
    if codec.is_none() {
        eprintln!("no hardware codec pair here; skipping");
    }
    codec
}

/// A 60-frame clip with a moving ball, so successive frames differ and the same frame is
/// the same picture. `None` where there is no codec to encode or import it with.
fn clip(name: &str) -> Option<PathBuf> {
    let codec = codec()?;
    gst::init().unwrap();
    let dir = std::env::temp_dir().join(format!("supersilvia-video-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.mp4"));
    let description = format!(
        "videotestsrc pattern=ball num-buffers=60 \
         ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert \
         ! {} ! mp4mux ! filesink location=\"{}\"",
        codec.encode_chain(),
        path.display()
    );
    let p = gst::parse::launch(&description).unwrap();
    p.set_state(gst::State::Playing).unwrap();
    let msg = p
        .bus()
        .unwrap()
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(30),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("encode finished");
    assert!(!matches!(msg.view(), gst::MessageView::Error(_)), "{msg:?}");
    p.set_state(gst::State::Null).unwrap();
    Some(path)
}

fn app_with_video(file: &Path) -> (App, NodeId) {
    // A headless app's project is a scratch folder under the system temp, and the cache is
    // inside the project, so a test run leaves nothing behind anywhere else.
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "video",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let id = app.graph().iter().next().unwrap().0;
    app.apply(Command::SetOption {
        node: id,
        key: "file",
        value: file.display().to_string(),
    })
    .unwrap();
    (app, id)
}

/// Tick at 60 Hz until the node publishes a real frame — the transcode, if needed, and
/// the first decode both happen off the tick.
fn tick_until_playing(app: &mut App, id: NodeId) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.tick(1.0 / 60.0);
        if let Some(f) = app.frame(PortRef::new(id, "frame"))
            && f.width > 2
        {
            return (**f).clone();
        }
        assert!(Instant::now() < deadline, "no frame within a minute");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Tick with the given dt until the published frame changes from `from`, or give up.
fn tick_until_changed(app: &mut App, id: NodeId, from: &Frame, dt: f32) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.tick(dt);
        if let Some(f) = app.frame(PortRef::new(id, "frame"))
            && **f != *from
        {
            return (**f).clone();
        }
        assert!(Instant::now() < deadline, "the picture never changed");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Imported, a clip plays forward at its own rate: 60 frames at 30 fps is a play every two
/// seconds, so half a second of ticks moves it a quarter of the clip, and the picture follows.
#[test]
fn a_file_is_imported_and_plays_forward_at_its_own_rate() {
    let Some(file) = clip("forward") else { return };
    let (mut app, id) = app_with_video(&file);
    let first = tick_until_playing(&mut app, id);
    assert_eq!((first.width, first.height), (320, 240));
    let next = tick_until_changed(&mut app, id, &first, 1.0 / 60.0);
    assert_ne!(first, next);

    let from = position(&app, id);
    settle(&mut app, 30);
    let moved = (position(&app, id) - from).rem_euclid(1.0);
    assert!(
        (moved - 0.25).abs() < 1e-3,
        "half a second is a quarter of a two-second clip, forward: {moved}"
    );
}

/// Tick for a while at 60 Hz, giving the worker time to deliver.
fn settle(app: &mut App, ticks: usize) {
    for _ in 0..ticks {
        app.tick(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Tick until the node's picture is no longer `was`, or give up after `ticks`.
///
/// A fixed number of ticks is a wall-clock assumption about a hardware decoder, and under a
/// parallel `cargo test` the decoder is contended: the same six ticks that are ample alone
/// are not always enough beside the other clip tests. Waiting for the thing the assertion is
/// about removes the assumption without weakening it — the caller still asserts the change
/// happened.
fn settle_until_changed(app: &mut App, id: NodeId, was: &Frame, ticks: usize) {
    for _ in 0..ticks {
        app.tick(1.0 / 60.0);
        std::thread::sleep(Duration::from_millis(2));
        if app
            .frame(PortRef::new(id, "frame"))
            .is_some_and(|f| **f != *was)
        {
            return;
        }
    }
}

fn picture(app: &App, id: NodeId) -> Frame {
    (**app.frame(PortRef::new(id, "frame")).unwrap()).clone()
}

/// A node of kind `slug` cabled from its `port` into the clip's Time, in Loop mode: a Ratio
/// Gear on ambient seconds plays it, a Number holds it still.
fn clocked(app: &mut App, id: NodeId, slug: &'static str, port: &'static str) -> NodeId {
    app.apply(Command::AddNode {
        slug,
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let clock = app.graph().iter().map(|(i, _)| i).last().unwrap();
    app.apply(Command::SetOption {
        node: id,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(clock, port),
        to: PortRef::new(id, supersilvia::nodes::TIME),
    })
    .unwrap();
    clock
}

/// A Number at zero in the clip's Time: it stands still.
fn still(app: &mut App, id: NodeId) -> NodeId {
    clocked(app, id, "number", "output")
}

/// Offset is added to the clip's Time. With a still number in Time, a cable on Offset is the
/// whole position, so it scrubs and a held position holds its frame; with a 1 : 1 gear in
/// Time the clip plays forward on top of the cable, a play a second, and running free at
/// Speed −2 it plays backward, a play a second the other way, with Offset still added.
#[test]
fn offset_is_added_to_time_in_either_direction() {
    let Some(file) = clip("scrub") else { return };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);
    still(&mut app, id);

    app.apply(Command::AddNode {
        slug: "slew",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let slew = app.graph().iter().map(|(i, _)| i).last().unwrap();
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(0.5),
    })
    .unwrap();
    app.apply(Command::Connect {
        from: PortRef::new(slew, "output"),
        to: PortRef::new(id, supersilvia::nodes::timing::OFFSET),
    })
    .unwrap();
    // Half a clip on: let the slew arrive and the frame land.
    settle(&mut app, 60);
    let mid = picture(&app, id);
    assert!(
        (position(&app, id) - 0.5).abs() < 1e-3,
        "Offset is the whole position with Time still: {}",
        position(&app, id)
    );

    // Scrub to a quarter: a different picture, and holding there keeps it still.
    app.apply(Command::SetControl {
        node: slew,
        key: "input",
        value: ControlValue::Float(0.25),
    })
    .unwrap();
    settle(&mut app, 60);
    let quarter = picture(&app, id);
    assert!(quarter != mid, "the scrub moved the picture");
    assert!((position(&app, id) - 0.25).abs() < 1e-3, "to a quarter");
    settle(&mut app, 30);
    assert!(
        picture(&app, id) == quarter,
        "a held position holds its frame with Time still"
    );

    // A 1 : 1 gear in Time with the cable still in Offset: the clip plays on top of it, and a
    // fifth of a second is a fifth of a play on.
    clocked(&mut app, id, "ratiogear", "cycles");
    settle_until_changed(&mut app, id, &quarter, 150);
    let playing = picture(&app, id);
    assert!(playing != quarter, "the playback is added to the cable");
    let from = position(&app, id);
    settle(&mut app, 12);
    let moved = (position(&app, id) - from).rem_euclid(1.0);
    assert!((moved - 0.2).abs() < 1e-3, "1 : 1 plays forward: {moved}");

    // And running free at Speed −2 on a clip two seconds long, backward: a fifth of a second
    // is a fifth of a play back.
    app.apply(Command::SetOption {
        node: id,
        key: "clockMode",
        value: "free".to_string(),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: id,
        key: supersilvia::nodes::timing::SPEED,
        value: ControlValue::Float(-2.0),
    })
    .unwrap();
    settle(&mut app, 10);
    let from = position(&app, id);
    let before = picture(&app, id);
    settle(&mut app, 12);
    let moved = (position(&app, id) - from).rem_euclid(1.0);
    assert!(
        (moved - 0.8).abs() < 1e-3,
        "Speed −2 plays backward: {moved}"
    );
    settle_until_changed(&mut app, id, &before, 150);
    assert!(picture(&app, id) != before, "and the picture moves with it");
}

/// **A negative Offset on a clip.** With a still number in Time the Offset is the whole
/// position: on Loop −0.25 wraps to three quarters through, and on Hold, where the clip never
/// comes back, the sum is held at the first frame until the Time has made the Offset up.
#[test]
fn a_negative_offset_wraps_on_loop_and_waits_on_hold() {
    let Some(file) = clip("behind") else { return };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);
    still(&mut app, id);
    app.apply(Command::SetControl {
        node: id,
        key: supersilvia::nodes::timing::OFFSET,
        value: ControlValue::Float(-0.25),
    })
    .unwrap();
    settle(&mut app, 10);
    assert!(
        (position(&app, id) - 0.75).abs() < 1e-3,
        "a quarter back on Loop: {}",
        position(&app, id)
    );
    app.apply(Command::SetOption {
        node: id,
        key: "loop",
        value: "hold".to_string(),
    })
    .unwrap();
    settle(&mut app, 10);
    assert_eq!(position(&app, id), 0.0, "on Hold, the first frame");
}

/// Where the clip is, 0 to 1, as the node's own report line says it.
fn position(app: &App, id: NodeId) -> f64 {
    let report = app.cpu_report(id);
    let at = report
        .split(", at ")
        .nth(1)
        .expect("a playing clip reports where it is");
    at.split(',').next().unwrap().trim().parse().unwrap()
}

/// A clip's Time is on the transport: paused, it holds its frame, and a seek moves it by what
/// the playhead moved at its gear's rate — at a quarter of a play a second, a second of the show
/// is a quarter of the clip.
#[test]
fn a_clip_holds_on_pause_and_a_seek_moves_it_by_its_rate_times_the_jump() {
    use supersilvia::transport::Command as Transport;
    let Some(file) = clip("seek") else { return };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);
    let gear = clocked(&mut app, id, "ratiogear", "cycles");
    app.apply(Command::SetControl {
        node: gear,
        key: "q",
        value: ControlValue::Float(4.0),
    })
    .unwrap();
    settle(&mut app, 10);

    app.transport(Transport::Pause);
    app.tick(1.0 / 60.0);
    let held = position(&app, id);
    let picture_held = {
        settle(&mut app, 30);
        picture(&app, id)
    };
    settle(&mut app, 30);
    assert_eq!(position(&app, id), held, "paused, the clip holds");
    assert!(picture(&app, id) == picture_held, "and so does its frame");

    let at = app.transport_state().playhead;
    app.transport(Transport::Seek(at + 1.0));
    app.tick(1.0 / 60.0);
    let moved = (position(&app, id) - held).rem_euclid(1.0);
    assert!(
        (moved - 0.25).abs() < 1e-3,
        "a quarter of a play a second over a second: {moved}"
    );
}

/// **A held clip stays held past the count's wrap.** A clip on Hold plays once and holds its
/// last frame. On a one-second Master Gear's Cycles, or on the Time node's Seconds — plays of
/// the clip either way — it is long past its one play at 2519.5 and stays on its last frame
/// through 2520, where a count read as one `f32` rolls over to zero, which would send it back
/// to its first.
#[test]
fn a_held_clip_stays_held_past_2520() {
    let Some(file) = clip("held") else { return };
    for (slug, port) in [("mastergear", "cycles"), ("time", "seconds")] {
        let (mut app, id) = app_with_video(&file);
        app.apply(Command::SetOption {
            node: id,
            key: "loop",
            value: "hold".to_string(),
        })
        .unwrap();
        tick_until_playing(&mut app, id);
        app.apply(Command::AddNode {
            slug,
            at: Pos2::ZERO,
            workspace: app.graph().default_workspace(),
        })
        .unwrap();
        let source = app.graph().iter().map(|(i, _)| i).last().unwrap();
        if slug == "mastergear" {
            app.apply(Command::SetControl {
                node: source,
                key: "length",
                value: ControlValue::Float(1.0),
            })
            .unwrap();
        }
        app.apply(Command::SetOption {
            node: id,
            key: "clockMode",
            value: "loop".to_string(),
        })
        .unwrap();
        app.apply(Command::Connect {
            from: PortRef::new(source, port),
            to: PortRef::new(id, supersilvia::nodes::TIME),
        })
        .unwrap();
        app.transport(supersilvia::transport::Command::Seek(2519.5));
        for _ in 0..60 {
            app.tick(1.0 / 60.0);
            assert_eq!(
                position(&app, id),
                1.0,
                "{slug}: held on its last frame at {}",
                app.transport_state().playhead
            );
        }
        assert!(app.transport_state().playhead > 2520.4, "{slug}: past 2520");
    }
}

#[test]
fn a_missing_file_is_a_status_line_not_a_panic() {
    let (mut app, id) = app_with_video(&PathBuf::from("/nowhere/at/all.mp4"));
    for _ in 0..3 {
        app.tick(1.0 / 60.0);
    }
    // The node stays alive publishing its placeholder.
    assert_eq!(
        app.frame(PortRef::new(id, "frame")).map(|f| f.width),
        Some(2)
    );
}

// ---------------------------------------------------------------- the soundtrack

/// An audio encoder and a container this machine actually has.
///
/// Which one is not the point of the test — the point is that the file has *a* soundtrack —
/// and the set installed differs by distribution, so the test picks rather than assumes.
fn audio_encoder() -> Option<(&'static str, &'static str, &'static str)> {
    [
        ("opusenc", "matroskamux", "mkv"),
        ("vorbisenc", "matroskamux", "mkv"),
        ("flacenc", "matroskamux", "mkv"),
        ("fdkaacenc", "mp4mux", "mp4"),
        ("avenc_aac", "mp4mux", "mp4"),
    ]
    .into_iter()
    .find(|(enc, _, _)| gst::ElementFactory::find(enc).is_some())
}

/// A clip with a 100 Hz sine on its audio track, so a bass band has something to find —
/// or `None`, having said why the test is skipped.
fn clip_with_sound(name: &str) -> Option<PathBuf> {
    let codec = codec()?;
    gst::init().unwrap();
    let Some((encoder, mux, extension)) = audio_encoder() else {
        eprintln!("no audio encoder installed; skipping");
        return None;
    };
    let dir = std::env::temp_dir().join(format!("supersilvia-video-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.{extension}"));
    let description = format!(
        "{mux} name=mux ! filesink location=\"{}\" \
         videotestsrc pattern=ball num-buffers=60 \
         ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert \
         ! {} ! mux. \
         audiotestsrc num-buffers=120 wave=sine freq=100 volume=0.9 \
         ! audioconvert ! {encoder} ! mux.",
        path.display(),
        codec.encode_chain(),
    );
    let p = gst::parse::launch(&description).unwrap();
    p.set_state(gst::State::Playing).unwrap();
    let msg = p
        .bus()
        .unwrap()
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(30),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("encode finished");
    assert!(!matches!(msg.view(), gst::MessageView::Error(_)), "{msg:?}");
    p.set_state(gst::State::Null).unwrap();
    Some(path)
}

/// The bug this was written for: the transcode sends every non-video stream to a `fakesink`,
/// so the cache has no soundtrack and for a long time neither did the node. The audio comes
/// from the source file now, and this is what says so.
#[test]
fn a_video_analyzes_its_own_soundtrack() {
    let Some(file) = clip_with_sound("sound") else {
        return;
    };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);

    // The decode runs on a worker; it is a couple of seconds of audio, so it lands fast, but
    // the tick never waits for it and neither does this.
    let bass = PortRef::new(id, "bass");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.tick(1.0 / 60.0);
        if app.uniform(bass).is_some_and(|v| v > 0.05) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the soundtrack never reached the bass band: {:?}",
            app.uniform(bass)
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    // A 100 Hz tone is bass and nothing else.
    let (b, m, h) = (
        app.uniform(PortRef::new(id, "bass")).unwrap(),
        app.uniform(PortRef::new(id, "mid")).unwrap(),
        app.uniform(PortRef::new(id, "high")).unwrap(),
    );
    assert!(b > m && b > h, "a 100 Hz tone is bass: {b}, {m}, {h}");
}

#[test]
fn a_band_crossing_its_level_fires_an_event_placed_inside_the_frame() {
    let Some(file) = clip_with_sound("events") else {
        return;
    };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);

    app.apply(Command::SetControl {
        node: id,
        key: "bassLevel",
        value: ControlValue::Float(0.05),
    })
    .unwrap();

    let trigger = PortRef::new(id, "bassEvent");
    let dt = 1.0 / 60.0;
    let deadline = Instant::now() + Duration::from_secs(30);
    let fired = loop {
        app.tick(dt);
        let events: Vec<_> = app.edges(trigger).to_vec();
        if !events.is_empty() {
            break events;
        }
        assert!(
            Instant::now() < deadline,
            "a bass band under a 100 Hz tone never crossed a level of 0.05"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        fired.iter().all(|e| (0.0..=dt).contains(&e.at)),
        "every event is placed inside the frame that found it: {fired:?}"
    );
}

/// The bug this file's `PortRef` keys exist for.
///
/// A node has as many outputs as it declares, and `video` has two textures: its picture and
/// its oscilloscope. They were keyed by node all the way from `publish_frame` to the
/// renderer's upload, so the second overwrote the first and both uniforms sampled whichever
/// was published last. The pictures have different shapes, so this fails loudly.
#[test]
fn a_node_publishes_a_frame_for_each_of_its_texture_outputs() {
    let Some(file) = clip_with_sound("two-textures") else {
        return;
    };
    let (mut app, id) = app_with_video(&file);
    tick_until_playing(&mut app, id);

    let picture = PortRef::new(id, "frame");
    let waveform = PortRef::new(id, "oscilloscope");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.tick(1.0 / 60.0);
        if app.frame(picture).is_some() && app.frame(waveform).is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "both textures should be published"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let picture = app.frame(picture).expect("the picture").clone();
    let waveform = app.frame(waveform).expect("the waveform").clone();
    assert_eq!(
        (waveform.width, waveform.height),
        (512, 1),
        "the oscilloscope is a one-dimensional lookup"
    );
    assert!(
        picture.width > 2 && picture.height > 2,
        "and the picture is a picture: {}x{}",
        picture.width,
        picture.height
    );
    assert_ne!(
        (picture.width, picture.height),
        (waveform.width, waveform.height),
        "one texture per node would have made these the same frame"
    );
}

/// The Main Input's *Video source* audio follows whichever clip the video source is, in
/// whichever order the two were chosen: chosen before any clip, it is heard once a clip
/// arrives, and the panel's levels fire its events as they fire a capture's.
#[test]
fn the_main_inputs_video_source_sound_follows_the_clip_chosen_after_it() {
    use supersilvia::maininput::{AudioSource, MainInput, VideoSource};
    use supersilvia::synth::maininput::Live;
    let Some(file) = clip_with_sound("main-input") else {
        return;
    };
    let cache = std::env::temp_dir().join(format!(
        "supersilvia-main-input-sound-{}",
        std::process::id()
    ));
    let resolve = |r: &str| PathBuf::from(r);
    let mut live = Live::default();
    let mut want = MainInput {
        audio: AudioSource::Video,
        ..MainInput::default()
    };
    live.reconcile(&want, &resolve, &cache, &FRAME_TIME);
    assert!(!live.hears(), "no clip, nothing to hear");

    want.video = VideoSource::File {
        asset: file.display().to_string(),
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while live.analysis().bands[0] <= 0.05 {
        live.reconcile(&want, &resolve, &cache, &FRAME_TIME);
        assert!(
            Instant::now() < deadline,
            "the clip's soundtrack never reached the bass band: {}, {:?}",
            live.audio_status(&want.audio),
            live.error()
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    want.levels[0] = 0.05;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        live.reconcile(&want, &resolve, &cache, &FRAME_TIME);
        live.set_levels(want.levels);
        let (edges, total) = live.crossings();
        live.delivered(total);
        if edges.iter().any(|c| c.band == 0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a bass band under a 100 Hz tone never crossed the panel's level of 0.05"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = std::fs::remove_dir_all(&cache);
}

/// A clip with no audio track decodes to a silent track and returns, rather than waiting
/// for ever on an appsink nothing is linked to.
#[test]
fn a_clip_with_no_soundtrack_decodes_to_silence() {
    let Some(file) = clip("silent") else {
        return;
    };
    let cache =
        std::env::temp_dir().join(format!("supersilvia-silent-sound-{}", std::process::id()));
    let (tx, rx) = std::sync::mpsc::channel();
    let (source, into) = (file.clone(), cache.clone());
    std::thread::spawn(move || {
        let _ = tx.send(supersilvia::audio::Track::decode(&source, &into));
    });
    let track = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the decode of a clip with no audio never returned")
        .expect("and it is not an error");
    assert!(track.is_empty(), "a clip with no audio is silence");
    let _ = std::fs::remove_dir_all(&cache);
}

/// Two decodes of one soundtrack at once — a `video` node and the Main Input on the same
/// clip — each write a temporary of their own, so both come back with the whole track.
#[test]
fn two_decodes_of_one_soundtrack_at_once_both_get_it() {
    let Some(file) = clip_with_sound("twice") else {
        return;
    };
    let cache =
        std::env::temp_dir().join(format!("supersilvia-twice-sound-{}", std::process::id()));
    let decodes: Vec<_> = (0..2)
        .map(|_| {
            let (source, into) = (file.clone(), cache.clone());
            std::thread::spawn(move || supersilvia::audio::Track::decode(&source, &into))
        })
        .collect();
    let lengths: Vec<usize> = decodes
        .into_iter()
        .map(|d| d.join().unwrap().expect("each decode succeeds").len())
        .collect();
    assert!(
        lengths[0] > 0 && lengths[0] == lengths[1],
        "both hold the whole track: {lengths:?}"
    );
    let _ = std::fs::remove_dir_all(&cache);
}

/// PNG in and out, which is what a workspace thumbnail is written and read through.
///
/// There is no image crate here and there is not going to be one: GStreamer already encodes
/// and decodes PNG, so two short pipelines are the whole of it. What has to hold is that the
/// bytes come back exactly — a thumbnail read as a texture is compared by eye against the
/// render it was taken from.
#[test]
fn png_write_then_read_round_trips_a_small_image() {
    use supersilvia::video::png;

    let dir = std::env::temp_dir().join(format!("ssv-png-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("small.png");

    let mut image = png::Image::new(5, 3);
    for (i, pixel) in image.rgba.chunks_mut(4).enumerate() {
        // Every channel varying, and alpha at full: a PNG that round-trips a flat color
        // proves nothing about strides.
        let i = u8::try_from(i).unwrap();
        pixel.copy_from_slice(&[i * 17, 255 - i * 13, i * 5 + 3, 255]);
    }

    png::write(&path, &image).unwrap();
    assert!(path.is_file());
    assert_eq!(png::read(&path).unwrap(), image);

    // A file that is not a PNG is an error rather than a panic or an empty picture.
    let text = dir.join("not.png");
    std::fs::write(&text, b"hello").unwrap();
    assert!(png::read(&text).is_err());
    assert!(png::read(&dir.join("missing.png")).is_err());

    std::fs::remove_dir_all(&dir).ok();
}

/// A render's video file: frames go in one at a time on the hardware encoder and come out
/// as a clip the importer understands, at the size and rate they were pushed at.
#[test]
fn an_encoder_writes_a_clip_frame_by_frame() {
    use supersilvia::video::encode::Encoder;
    if codec().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("supersilvia-encode-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dest = dir.join("out/render.mp4");
    let (w, h) = (320, 180);
    let mut encoder = Encoder::start(&dest, w, h, 24.0).expect("a hardware encoder");
    for i in 0..12u8 {
        let frame = [i * 20, 0, 255 - i * 20, 255].repeat((w * h) as usize);
        encoder.push(&frame).unwrap();
    }
    assert_eq!(encoder.frames(), 12);
    assert!(
        encoder.push(&[0; 16]).is_err(),
        "a frame of the wrong size is refused rather than written"
    );
    let written = encoder.finish().expect("the file closes");
    assert_eq!(written, dest);
    assert!(!dest.with_extension("part").exists(), "the partial is gone");
    let info = supersilvia::video::clip::discover(&dest).expect("a clip");
    assert_eq!((info.width, info.height), (w, h));
    assert!((info.fps - 24.0).abs() < 1e-3, "{}", info.fps);
    assert_eq!(info.frames, 12, "{info:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **What a dropped clip says while it is being prepared**, and where it says it.
///
/// A first import transcodes the file into the cache, which on a long clip is minutes, and
/// all the node used to say was `transcoding 40%` — a percentage cannot tell you whether the
/// wait is ten seconds or ten minutes. So the line is the words a person would use for the
/// wait, the percentage, and a **clock**: `Preparing clip… 40%  0:07`. It is the node's own
/// `CpuNode::status`, which `widgets::picture` draws across the middle of the preview band —
/// the black band the eye is already on, rather than a row under the rows.
///
/// The clip here is 60 frames and the encode is hardware, so this races the transcode
/// finishing: the test ticks until the node has either said it or started playing, and asks
/// only that whatever it said while transcoding had the shape above.
#[test]
fn a_clip_being_prepared_says_so_with_a_clock_on_it() {
    let Some(file) = clip("preparing") else {
        return;
    };
    // A cache of its own, so a previous run's entry cannot let this one skip the transcode.
    let (mut app, id) = app_with_video(&file);
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut said: Option<String> = None;
    loop {
        app.tick(1.0 / 60.0);
        if let Some(line) = app.node_status(id) {
            said = Some(line.to_string());
            break;
        }
        if app
            .frame(PortRef::new(id, "frame"))
            .is_some_and(|f| f.width > 2)
        {
            break;
        }
        assert!(Instant::now() < deadline, "neither prepared nor playing");
        std::thread::sleep(Duration::from_millis(2));
    }
    let Some(line) = said else {
        // The cache already held it, or the encode outran the first tick. Nothing was said,
        // which is the right thing to say about a clip that needed no preparing.
        return;
    };
    assert!(
        line.starts_with("Preparing clip… "),
        "the words a person would use: {line:?}"
    );
    let rest = line.trim_start_matches("Preparing clip… ");
    let (percent, elapsed) = rest.split_once("  ").expect("a percentage and a clock");
    let percent = percent.strip_suffix('%').expect("a real percentage");
    let percent: f32 = percent.parse().expect("a number");
    assert!((0.0..=100.0).contains(&percent), "{line:?}");
    let (minutes, seconds) = elapsed.split_once(':').expect("m:ss");
    assert!(minutes.parse::<u64>().is_ok(), "{line:?}");
    assert_eq!(seconds.len(), 2, "seconds are padded: {line:?}");
    assert!(seconds.parse::<u64>().is_ok_and(|s| s < 60), "{line:?}");
}

/// **A GIF in a video node says why it shows nothing, on the node.**
///
/// A GIF is the Image/GIF node's: a drop makes one and a video node's file button does not
/// offer it. A project saved before that can still hold one in a video node, and GStreamer
/// gives a GIF no length, so the transcode failed with `no frames` into the Status box and
/// the node stayed black. The node refuses it by name before any encoder is asked, and says
/// so across its own picture band, where the eye already is, naming the node that plays it.
#[test]
fn a_gif_in_a_video_node_says_why_on_the_node() {
    // Without a codec the node's line is the missing encoder, which it says first.
    if codec().is_none() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("supersilvia-video-gif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("loop.gif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = image::codecs::gif::GifEncoder::new(file);
        for rgba in [[220, 20, 20, 255], [20, 20, 220, 255]] {
            let picture = image::RgbaImage::from_pixel(33, 17, image::Rgba(rgba));
            encoder
                .encode_frame(image::Frame::from_parts(
                    picture,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                ))
                .expect("a gif of our own");
        }
    }

    let (mut app, id) = app_with_video(&path);
    let deadline = Instant::now() + Duration::from_secs(10);
    let line = loop {
        app.tick(1.0 / 60.0);
        if let Some(line) = app.node_status(id)
            && !line.starts_with("Preparing clip")
        {
            break line.to_string();
        }
        assert!(
            Instant::now() < deadline,
            "the node says nothing on itself: {}",
            app.cpu_report(id)
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(line.contains("loop.gif"), "names the file: {line:?}");
    assert!(
        line.contains("Image/GIF"),
        "names the node that plays it: {line:?}"
    );
    std::fs::remove_dir_all(&dir).ok();
}

// ------------------------------------------------------------------------------ the words

/// A headless app holding one `text` node, with the string and the options a test wants.
fn app_with_words(words: &str, options: &[(&'static str, &str)]) -> (App, NodeId) {
    let mut app = App::headless();
    app.apply(Command::AddNode {
        slug: "text",
        at: Pos2::ZERO,
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let id = app.graph().iter().next().unwrap().0;
    app.apply(Command::SetValue {
        node: id,
        key: "words",
        value: supersilvia::graph::Value::Text(words.to_string()),
    })
    .unwrap();
    for (key, value) in options {
        app.apply(Command::SetOption {
            node: id,
            key,
            value: (*value).to_string(),
        })
        .unwrap();
    }
    (app, id)
}

/// Tick until the pipeline has drawn the letters, which happens on GStreamer's own threads.
fn tick_until_drawn(app: &mut App, id: NodeId) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.tick(1.0 / 60.0);
        if let Some(f) = app.frame(PortRef::new(id, "ink"))
            && f.width > 2
        {
            return (**f).clone();
        }
        assert!(Instant::now() < deadline, "nothing was drawn within 30s");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How much ink is in a rectangle of the frame, as a fraction of its pixels. The picture is
/// white letters on black, so the red channel is the coverage.
fn ink(frame: &Frame, from: (f32, f32), to: (f32, f32)) -> f32 {
    let bytes = frame.bytes().expect("bytes were asked for");
    let x0 = (from.0 * frame.width as f32) as u32;
    let x1 = (to.0 * frame.width as f32) as u32;
    let y0 = (from.1 * frame.height as f32) as u32;
    let y1 = (to.1 * frame.height as f32) as u32;
    let mut lit = 0;
    let mut seen = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let at = ((y * frame.width + x) * 4) as usize;
            seen += 1;
            if bytes[at] > 128 {
                lit += 1;
            }
        }
    }
    lit as f32 / seen.max(1) as f32
}

/// A known string is rasterized, and the ink is where the glyphs are.
///
/// One tall centered letter on a wide frame: there has to be white in the middle and none in
/// the corners, which is the difference between drawing the letter and drawing anything at
/// all.
#[test]
fn a_string_is_rasterized_with_ink_where_the_glyphs_are() {
    let (mut app, id) = app_with_words(
        "I",
        &[
            ("font", "Monospace"),
            ("size", "256"),
            ("weight", "bold"),
            ("align", "center"),
            ("baseline", "middle"),
            ("texsize", "1280x720"),
        ],
    );
    let frame = tick_until_drawn(&mut app, id);
    assert_eq!((frame.width, frame.height), (1280, 720));

    let middle = ink(&frame, (0.45, 0.3), (0.55, 0.7));
    assert!(middle > 0.05, "the letter is in the middle: {middle}");
    for (name, from, to) in [
        ("top left", (0.0, 0.0), (0.2, 0.2)),
        ("top right", (0.8, 0.0), (1.0, 0.2)),
        ("bottom left", (0.0, 0.8), (0.2, 1.0)),
        ("bottom right", (0.8, 0.8), (1.0, 1.0)),
    ] {
        let corner = ink(&frame, from, to);
        assert_eq!(corner, 0.0, "no ink in the {name}: {corner}");
    }
}

/// Where the words sit is the Baseline option's answer, and nothing else moves with it.
#[test]
fn the_baseline_moves_the_block_up_the_frame() {
    let options = |baseline: &'static str| {
        [
            ("font", "Monospace"),
            ("size", "128"),
            ("weight", "bold"),
            ("align", "center"),
            ("baseline", baseline),
            ("texsize", "1280x720"),
        ]
    };
    let (mut app, id) = app_with_words("HI", &options("top"));
    let top = tick_until_drawn(&mut app, id);
    let (mut app, id) = app_with_words("HI", &options("bottom"));
    let bottom = tick_until_drawn(&mut app, id);

    assert!(
        ink(&top, (0.0, 0.0), (1.0, 0.4)) > ink(&top, (0.0, 0.6), (1.0, 1.0)),
        "Top puts the words in the upper third"
    );
    assert!(
        ink(&bottom, (0.0, 0.6), (1.0, 1.0)) > ink(&bottom, (0.0, 0.0), (1.0, 0.4)),
        "and Bottom in the lower one"
    );
}

/// Typing changes the picture and not the program.
///
/// The words are a value, so they reach no generator and no uniform: the Output's shader
/// comes out character for character the same. That is the whole reason the string is not an
/// option — an option that reached the WGSL would rebuild every Output downstream on every
/// keystroke.
#[test]
fn changing_the_string_rebuilds_no_shader() {
    let (mut app, id) = app_with_words("first", &[]);
    app.apply(Command::AddNode {
        slug: "output",
        at: Pos2::new(400.0, 0.0),
        workspace: app.graph().default_workspace(),
    })
    .unwrap();
    let out = app
        .graph()
        .iter()
        .find(|(_, n)| n.def.is_output)
        .map(|(id, _)| id)
        .expect("an Output");
    app.apply(Command::Connect {
        from: PortRef::new(id, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let before = supersilvia::compile::wgsl::build(app.graph(), out).expect("it compiles");
    app.apply(Command::SetValue {
        node: id,
        key: "words",
        value: supersilvia::graph::Value::Text("second\nand third".to_string()),
    })
    .unwrap();
    let after = supersilvia::compile::wgsl::build(app.graph(), out).expect("it still compiles");
    assert_eq!(
        before.source(),
        after.source(),
        "a keystroke is a new picture, never a new program"
    );
}
