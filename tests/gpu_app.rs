// SPDX-License-Identifier: AGPL-3.0-or-later

//! The whole app on the wgpu renderer: what only an `App` with a real GPU can show, each on a
//! device of its own through `tests/common/gpu.rs`, on the adapter `render::adapter` picks.
//!
//! A Snap end to end, into the project's own `snaps/` folder; the render engine writing every
//! kept frame with the document closed; a loop as long as its Master Gear says, the tunnel's
//! flight and a clip's whole play each coming back to their first frame to the byte; a picture
//! on ambient time the same at one moment whatever came before; a slime mold's world stepped
//! by its node's own CPU
//! half, twice alike for one seed and one clock, reallocated, cleared, reset, drawn into an
//! Output and born again when its project is reopened; an idle Output shown its last frame and
//! drawn again on the tick it is switched to, a thumbnail or a Snap asks of it; the hollow node
//! shadow painted by egui_wgpu to the pixel of the whole one; a `drawingcanvas`'s painting,
//! and a stroke after it, sampled by an Output; the editor timing its own painting while
//! the Status box is open, run and painted by hand the way eframe runs it; and a workspace's
//! pass thumbnailing every varying output on it, with no Output there at all.
//!
//! The renderer-level tests of the same things are `tests/gpu_*.rs`; these are their app
//! halves.

#[path = "common/gpu.rs"]
mod gpu;

use eframe::egui;
use supersilvia::render::{Gpu, shared};

/// A straight channel `c` at alpha `a` over black, as a byte.
fn over_black(c: u8, a: u8) -> u8 {
    ((u16::from(c) * u16::from(a) + 127) / 255) as u8
}

/// A target of `size` in `Rgba8Unorm`, cleared to opaque black and painted by egui_wgpu's own
/// renderer with `primitives` at `ppp`, the frame's texture changes made first and then
/// cleared: its bytes, rows top first.
fn paint_egui(
    gpu: &Gpu,
    renderer: &mut egui_wgpu::Renderer,
    size: [u32; 2],
    ppp: f32,
    primitives: &[egui::ClippedPrimitive],
    textures: &mut egui::TexturesDelta,
) -> Vec<u8> {
    let device = gpu.device();
    for (id, deltas) in &textures.set {
        for delta in deltas {
            renderer.update_texture(device, gpu.queue(), *id, delta);
        }
    }
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: ppp,
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("editor"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("editor"),
    });
    let prepared = renderer.update_buffers(device, gpu.queue(), &mut encoder, primitives, &screen);
    {
        let mut pass = shared::begin(
            &mut encoder,
            &view,
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            "editor",
        )
        .forget_lifetime();
        renderer.render(&mut pass, primitives, &screen);
    }
    gpu.submit(prepared.into_iter().chain([encoder.finish()]));
    for id in &textures.free {
        renderer.free_texture(id);
    }
    textures.clear();
    gpu::bytes_of(gpu, &target)
}

// ---------------------------------------------------------------------- Snap and render

/// **Snap writes the frame in front of you**, at the Output's full resolution, into the
/// project's own `snaps/` folder, with the time in its name.
///
/// End to end and on a real GPU, because the whole of what Snap is happens between the two:
/// an action input the synth reads like a deck claim, a full-size readback that never waits,
/// and a PNG. Nothing here asks for a render — the point of Snap is that the performance
/// does not stop.
#[test]
fn snap_writes_a_full_resolution_png_into_the_projects_snaps_folder() {
    use supersilvia::graph::PortRef;
    use supersilvia::video::png;
    use supersilvia::{App, Command};

    let root =
        std::env::temp_dir().join(format!("supersilvia-snap-project-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a project folder");

    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    app.new_project(root.clone());
    let cb = add_node(&mut app, "checkerboard");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    // A few frames so there is something published to take a picture of.
    for _ in 0..4 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }

    // The button a hand presses and a sequencer fires: the same port either way.
    app.press(PortRef::new(out, "snap"), true);

    let snaps = root.join("snaps");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let file = loop {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        if let Some(entry) = std::fs::read_dir(&snaps)
            .ok()
            .and_then(|d| d.flatten().next())
        {
            break entry.path();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no snap was ever written to {}",
            snaps.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    };

    assert_eq!(
        app.file_status_shows(),
        Some(file.as_path()),
        "the status line offers to show the picture: {}",
        app.file_status()
    );
    assert_eq!(
        app.toast_shows(),
        Some(file.as_path()),
        "and so does the toast, with the Status box closed"
    );
    assert!(app.toast().is_some_and(|t| t.starts_with("snapped ")));
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with(&format!("output{out}-")) && file.extension().is_some_and(|e| e == "png"),
        "the Output that took it, and a PNG: {name}"
    );
    // `output3-20260921-134501.png`: the stamp is eight digits, a dash and six.
    let stamp = name
        .trim_start_matches(&format!("output{out}-"))
        .trim_end_matches(".png");
    let (day, clock) = stamp.split_once('-').expect("a dated name");
    assert_eq!(day.len(), 8, "the date: {name}");
    assert_eq!(clock.len(), 6, "the time: {name}");
    assert!(
        day.chars().chain(clock.chars()).all(|c| c.is_ascii_digit()),
        "the name carries the time: {name}"
    );

    let image = png::read(&file).expect("the snap reads back");
    assert_eq!(
        (image.width, image.height),
        (1280, 720),
        "the Output's own resolution, not a thumbnail"
    );
    assert!(
        image.rgba.iter().step_by(4).any(|r| *r > 200),
        "and it is the checkerboard, not a black frame"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Everything the render engine is: the Output to a numbered PNG sequence, every kept frame
/// on disk at the Output's size, the document closed while it runs, and the three warm-ups
/// told apart by the one patch that can — feedback. A `mix` of a checkerboard with the
/// Output's own frame climbs toward the checkerboard one frame at a time, so the first kept
/// frame is dim after a black warm-up and settled after a hold or a run.
#[test]
fn a_render_writes_every_kept_frame_and_closes_the_document() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::video::png;
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let render = |warmup: Warmup,
                  frames: u32,
                  supersample: u32,
                  format: Format,
                  name: &str|
     -> (Outcome, std::path::PathBuf) {
        let mut app = App::headless();
        // The renderer on a device of its own, inline: the synth is on this thread here,
        // so the render loop runs inside `app.tick` exactly as it runs inside the synth
        // thread's own step.
        app.attach_gpu_on(gpu::gpu());
        let cb = add_node(&mut app, "checkerboard");
        let mix = add_node(&mut app, "mix");
        let out = add_node(&mut app, "output");
        for (from, to) in [
            (PortRef::new(cb, "output"), PortRef::new(mix, "a")),
            (PortRef::new(out, "frame"), PortRef::new(mix, "b")),
            (PortRef::new(mix, "output"), PortRef::new(out, "input")),
        ] {
            app.apply(Command::Connect { from, to }).unwrap();
        }
        let destination = root.join(name);
        app.start_render(
            out,
            &RenderSettings {
                fps: 10.0,
                frames,
                warmup,
                supersample,
                format,
                destination: destination.clone(),
            },
        )
        .expect("a connected Output renders");
        assert_eq!(
            app.apply(Command::SetOption {
                node: out,
                key: "resolution",
                value: "1024x768".to_string(),
            }),
            Err(supersilvia::CommandError::Rendering),
            "the document is closed while a render runs"
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.rendering() {
            assert!(
                std::time::Instant::now() < deadline,
                "the render never finished"
            );
            // One tick of the synth, which while a render runs is collect, step, draw —
            // whatever thread it is on. Nothing else drives it.
            app.publish_plan();
            app.tick(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(
            app.apply(Command::SetOption {
                node: out,
                key: "resolution",
                value: "1024x768".to_string(),
            })
            .is_ok(),
            "and open again after"
        );
        let outcome = app.render_outcome().cloned().unwrap();
        if matches!(outcome, Outcome::Done { .. }) {
            assert_eq!(
                app.file_status_shows(),
                Some(destination.as_path()),
                "the status line offers to show what was rendered"
            );
            assert_eq!(
                app.toast_shows(),
                Some(destination.as_path()),
                "and so does the toast, with the Status box closed"
            );
        }
        (outcome, destination)
    };

    // The brightest red over black, as a viewer shows it: the PNG is straight, and a frame
    // the feedback has not filled is partly transparent.
    let brightest = |dir: &std::path::Path, frame: u32| -> u8 {
        let image = png::read(&dir.join(format!("{frame:05}.png"))).expect("a kept frame");
        assert_eq!(
            (image.width, image.height),
            (1280, 720),
            "the Output's own size"
        );
        image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| over_black(p[0], p[3]))
            .max()
            .unwrap()
    };

    let (outcome, black) = render(Warmup::Black, 3, 1, Format::PngSequence, "black");
    assert_eq!(
        outcome,
        Outcome::Done {
            frames: 3,
            destination: black.clone(),
        }
    );
    let files: Vec<_> = std::fs::read_dir(&black).unwrap().flatten().collect();
    assert_eq!(files.len(), 3, "three kept frames and nothing else");
    let dim = brightest(&black, 0);
    assert!(
        dim < 200,
        "after a black warm-up the first frame is one step in: {dim}"
    );
    assert!(
        brightest(&black, 2) > dim,
        "and the feedback climbs frame by frame"
    );

    let (outcome, run) = render(Warmup::Run(12), 1, 1, Format::PngSequence, "run");
    assert!(
        matches!(outcome, Outcome::Done { frames: 1, .. }),
        "{outcome:?}"
    );
    let settled = brightest(&run, 0);
    assert!(
        settled > 240,
        "after a run the feedback has settled: {settled}"
    );

    let (outcome, hold) = render(Warmup::Hold(12), 1, 1, Format::PngSequence, "hold");
    assert!(
        matches!(outcome, Outcome::Done { frames: 1, .. }),
        "{outcome:?}"
    );
    assert!(brightest(&hold, 0) > 240, "a hold settles feedback too");

    // The same engine into a video file: the frames the capture read back, encoded as they
    // land, and a clip of exactly that many frames at the render's rate. It needs the
    // hardware codec pair.
    if supersilvia::video::clip::Codec::probe().is_some() {
        let (outcome, video) = render(Warmup::Black, 5, 1, Format::Video, "film.mp4");
        assert!(
            matches!(outcome, Outcome::Done { frames: 5, .. }),
            "{outcome:?}"
        );
        let info = supersilvia::video::clip::discover(&video).expect("a clip");
        assert_eq!((info.width, info.height), (1280, 720));
        assert_eq!(info.frames, 5, "{info:?}");
        assert!((info.fps - 10.0).abs() < 1e-3);
    } else {
        eprintln!("no hardware codec pair here; skipping the video file");
    }

    // Supersampled: the Output is drawn at twice its resolution for the length of the
    // render and every frame comes back down to the Output's own size, which is what the
    // film is and what an editor is handed.
    let (outcome, big) = render(Warmup::Black, 2, 2, Format::PngSequence, "supersampled");
    assert!(
        matches!(outcome, Outcome::Done { frames: 2, .. }),
        "{outcome:?}"
    );
    let _ = brightest(&big, 1);

    let _ = std::fs::remove_dir_all(&root);
}

/// **During a render a picture window shows each rendered frame as it is made.** A window
/// blits what the synth publishes: for the Output, the newest frame of its own that has
/// finished, and for the mix, the mix of its decks. While the Output renders, both step
/// through the film's own frames — here a feedback loop climbing out of a black warm-up —
/// one each, in order, where the live picture had settled bright.
#[test]
fn a_render_is_shown_frame_by_frame_as_it_is_made() {
    use supersilvia::app::render::{Format, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::mixer::Channel;
    use supersilvia::render::readback::half_to_rgba8;
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-shown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let device = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(device.clone());
    let cb = add_node(&mut app, "checkerboard");
    let mix = add_node(&mut app, "mix");
    let out = add_node(&mut app, "output");
    for (from, to) in [
        (PortRef::new(cb, "output"), PortRef::new(mix, "a")),
        (PortRef::new(out, "frame"), PortRef::new(mix, "b")),
        (PortRef::new(mix, "output"), PortRef::new(out, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.show_on(Channel::A, out);
    let brightest = |bytes: &[u8]| bytes.iter().step_by(4).copied().max().unwrap_or(0);
    // What a window on the Output and a window on the mix are handed this tick.
    let shown = |app: &App| -> (u8, u8) {
        let published = app.ticked().expect("inline").published();
        let output = published
            .outputs
            .get(&out)
            .expect("the Output is published");
        let mixed = published.mixer.as_ref().expect("the mix is published");
        (
            brightest(&half_to_rgba8(&gpu::bytes_of(
                &device,
                output.texture.texture(),
            ))),
            brightest(&gpu::bytes_of(&device, mixed.texture.texture())),
        )
    };
    for _ in 0..60 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    assert!(shown(&app).0 > 240, "the live loop has settled bright");

    let dir = root.join("film");
    app.start_render(
        out,
        &RenderSettings {
            fps: 10.0,
            frames: 4,
            warmup: Warmup::Black,
            supersample: 1,
            format: Format::PngSequence,
            destination: dir.clone(),
        },
    )
    .expect("a connected Output renders");
    let (mut on_output, mut on_mix) = (Vec::new(), Vec::new());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.rendering() {
        assert!(
            std::time::Instant::now() < deadline,
            "the render never finished"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
        let (output, mixed) = shown(&app);
        if on_output.last() != Some(&output) {
            on_output.push(output);
        }
        if on_mix.last() != Some(&mixed) {
            on_mix.push(mixed);
        }
    }
    // The PNG is straight and what a window is handed premultiplied: the film over black.
    let film: Vec<u8> = (0..4)
        .map(|i| {
            let image = supersilvia::video::png::read(&dir.join(format!("{i:05}.png")))
                .expect("a kept frame");
            let premultiplied: Vec<u8> = image
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [over_black(p[0], p[3]), p[1], p[2], p[3]])
                .collect();
            brightest(&premultiplied)
        })
        .collect();
    assert!(
        film.windows(2).all(|w| w[0] < w[1]),
        "the film climbs: {film:?}"
    );
    for (what, seen) in [("the Output's window", &on_output), ("the mix's", &on_mix)] {
        let mut frames = film.iter();
        let mut next = frames.next();
        for b in seen {
            if next.is_some_and(|f| f.abs_diff(*b) <= 1) {
                next = frames.next();
            }
        }
        assert!(
            next.is_none(),
            "{what} was shown every frame of the film in order: {seen:?} against {film:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A render steps the transport and hands it back: during it the playhead is the render's own
/// time, and after it the playhead is where the live show left it and a generator's Time,
/// ambient time at its own rate, is in phase with the playhead again.
#[test]
fn a_render_hands_the_playhead_back() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-seek-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let perlin = add_node(&mut app, "perlin");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(perlin, "color"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    for _ in 0..120 {
        app.tick(1.0 / 60.0);
    }
    let phase = |app: &App| {
        app.uniform(PortRef::new(perlin, supersilvia::nodes::TIME))
            .unwrap()
    };
    let live = app.transport_state().playhead;
    assert!((phase(&app) - 0.5 * live).abs() < 1e-3, "in phase before");
    let clip = app.snapshot().main_input.position;

    app.start_render(
        out,
        &RenderSettings {
            fps: 10.0,
            frames: 3,
            warmup: Warmup::Black,
            supersample: 1,
            format: Format::PngSequence,
            destination: root.clone(),
        },
    )
    .expect("a connected Output renders");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app.rendering() {
        assert!(
            std::time::Instant::now() < deadline,
            "the render never finished"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
        if app.rendering() {
            let state = app.transport_state();
            assert!(state.rendering && state.playhead <= 0.2 + 1e-9, "{state:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(matches!(
        app.render_outcome(),
        Some(Outcome::Done { frames: 3, .. })
    ));

    app.tick(1.0 / 60.0);
    let back = app.transport_state();
    assert!(!back.rendering);
    assert!(
        (back.playhead - live).abs() < 0.1,
        "the playhead is where the live show left it: {} against {live}",
        back.playhead
    );
    assert!(
        (phase(&app) - 0.5 * back.playhead).abs() < 1e-3,
        "and the generator's Time is the playhead's: {} at {}",
        phase(&app),
        back.playhead
    );
    let resumed = app.snapshot().main_input.position;
    assert!(
        (resumed - clip).abs() < 0.1,
        "the Main Input's clip is put back where live play had it, and not moved again by the \
         return: {resumed} against {clip}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A clip rendered twice is the same film to the byte: a render waits for the frame each
/// clip's position names, where live play shows whatever its decoder has delivered. At
/// Speed one a 30 fps clip rendered at 30 fps asks for a new frame every frame, so a render
/// that did not wait would draw the frame the decoder last had — on the first frame, wherever
/// live play had left the clip, which is different each time. The clip is a moving ball made here with the box's own encoder; with none, the test
/// says so and passes.
#[test]
fn a_clip_renders_the_same_twice_to_the_byte() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};
    const FRAMES: u32 = 12;

    let root = std::env::temp_dir().join(format!("supersilvia-clip-render-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let Some(source) = ball_clip(&root) else {
        eprintln!("no hardware codec pair here; skipping");
        return;
    };

    let render = |name: &str, preroll: u32| -> Vec<Vec<u8>> {
        let mut app = App::headless();
        app.attach_gpu_on(gpu::gpu());
        let video = add_node(&mut app, "video");
        let out = add_node(&mut app, "output");
        app.apply(Command::Connect {
            from: PortRef::new(video, "frame"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
        app.apply(Command::SetOption {
            node: video,
            key: "file",
            value: source.display().to_string(),
        })
        .unwrap();
        // The transcode and the first decode happen off the tick.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app
            .frame(PortRef::new(video, "frame"))
            .is_none_or(|f| f.width <= 2)
        {
            assert!(
                std::time::Instant::now() < deadline,
                "the clip never played"
            );
            app.publish_plan();
            app.tick(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // Played live for a while first, a different while each time: where the live show
        // left the clip is nothing the render may draw.
        for _ in 0..preroll {
            app.publish_plan();
            app.tick(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let destination = root.join(name);
        app.start_render(
            out,
            &RenderSettings {
                fps: 30.0,
                frames: FRAMES,
                warmup: Warmup::Black,
                supersample: 1,
                format: Format::PngSequence,
                destination: destination.clone(),
            },
        )
        .expect("a connected Output renders");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.rendering() {
            assert!(
                std::time::Instant::now() < deadline,
                "the render never finished"
            );
            app.publish_plan();
            app.tick(1.0 / 60.0);
        }
        assert!(matches!(
            app.render_outcome(),
            Some(Outcome::Done { frames: FRAMES, .. })
        ));
        (0..FRAMES)
            .map(|i| std::fs::read(destination.join(format!("{i:05}.png"))).unwrap())
            .collect()
    };

    let once = render("once", 20);
    let twice = render("twice", 45);
    for (i, (a, b)) in once.iter().zip(&twice).enumerate() {
        assert!(a == b, "frame {i} differs between the two renders");
    }
    assert!(
        once.windows(2).all(|w| w[0] != w[1]),
        "every frame is the clip's next one, so no two in a row are alike"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Two seconds of a moving ball, 60 frames at 30 fps, encoded into `root` with the box's own
/// hardware encoder; `None` where there is none.
fn ball_clip(root: &std::path::Path) -> Option<std::path::PathBuf> {
    use gstreamer as gst;
    use gstreamer::prelude::*;
    use supersilvia::video::clip::Codec;

    let codec = Codec::probe()?;
    let source = root.join("ball.mp4");
    gst::init().unwrap();
    let encode = gst::parse::launch(&format!(
        "videotestsrc pattern=ball num-buffers=60 \
         ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert \
         ! {} ! mp4mux ! filesink location=\"{}\"",
        codec.encode_chain(),
        source.display()
    ))
    .unwrap();
    encode.set_state(gst::State::Playing).unwrap();
    encode
        .bus()
        .unwrap()
        .timed_pop_filtered(
            gst::ClockTime::from_seconds(30),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
        .expect("encode finished");
    encode.set_state(gst::State::Null).unwrap();
    Some(source)
}

/// **A clip rendered for a whole play comes back to its first frame.** A two-second clip on
/// ambient time plays once in two seconds, so frame 20 of a render at 10 fps, the clip's Time
/// wrapped to zero, asks for the clip's first frame again — a frame its decoder, which has
/// just played the last one, has not got. The render holds the frame for it as it holds any
/// other, so frame 20 is frame zero to the byte, where drawing what the decoder had would leave
/// the clip's last frame there.
#[test]
fn a_clip_rendered_for_a_whole_play_comes_back_to_its_first_frame() {
    use supersilvia::graph::{ControlValue, PortRef};
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-clip-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let Some(source) = ball_clip(&root) else {
        eprintln!("no hardware codec pair here; skipping");
        return;
    };
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let video = add_node(&mut app, "video");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(video, "frame"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.apply(Command::SetOption {
        node: video,
        key: "file",
        value: source.display().to_string(),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: out,
        key: "fps",
        value: ControlValue::Float(10.0),
    })
    .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while app
        .frame(PortRef::new(video, "frame"))
        .is_none_or(|f| f.width <= 2)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the clip never played"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let frame = render_frames(&mut app, out, 10.0, 21, &root.join("play"));
    assert!(
        frame(0) == frame(20),
        "frame 20 waits for the clip's first frame"
    );
    assert!(frame(0) != frame(10), "half a play in is not the start");
    let _ = std::fs::remove_dir_all(&root);
}

/// **A node that moves with time draws the same picture at one moment, whatever came
/// before.** A Rotozoom turning a Perlin, both on ambient time, rendered three ways — one
/// frame a second from zero, two a second from zero, and one a second after five seconds of
/// warm-up — draws its twelve-second frame to the byte each time: neither node keeps anything
/// of the path there. And the frame half a second earlier is not that frame.
#[test]
fn a_time_driven_picture_is_the_same_at_one_moment_whatever_came_before() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::video::png;
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-stateless-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let perlin = add_node(&mut app, "perlin");
    let turn = add_node(&mut app, "rotozoom");
    let out = add_node(&mut app, "output");
    for (from, to) in [
        (PortRef::new(perlin, "color"), PortRef::new(turn, "input")),
        (PortRef::new(turn, "output"), PortRef::new(out, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    let mut render = |name: &str, fps: f64, frames: u32, warmup: Warmup| {
        let destination = root.join(name);
        app.start_render(
            out,
            &RenderSettings {
                fps,
                frames,
                warmup,
                supersample: 1,
                format: Format::PngSequence,
                destination: destination.clone(),
            },
        )
        .expect("a connected Output renders");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while app.rendering() {
            assert!(
                std::time::Instant::now() < deadline,
                "{name} never finished"
            );
            app.publish_plan();
            app.tick(1.0 / 60.0);
        }
        assert!(matches!(app.render_outcome(), Some(Outcome::Done { .. })));
        destination
    };
    let one = render("one", 1.0, 13, Warmup::Black);
    let two = render("two", 2.0, 25, Warmup::Black);
    let warm = render("warm", 1.0, 13, Warmup::Run(5));
    let frame =
        |dir: &std::path::Path, i: u32| png::read(&dir.join(format!("{i:05}.png"))).unwrap().rgba;
    let twelve = frame(&one, 12);
    assert!(
        twelve == frame(&two, 24),
        "two frames a second reach the same picture"
    );
    assert!(
        twelve == frame(&warm, 12),
        "and so does a run with a warm-up behind it"
    );
    assert!(
        twelve != frame(&two, 23),
        "half a second earlier is another picture"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// **A render waits for its program.** An Output whose new program is still linking when a
/// render starts draws nothing into the film, rather than the program it replaces: with the
/// link held for sixty ticks, no frame is written; let go, the film completes, and its first
/// frame is the new picture and not the old.
#[test]
fn a_render_waits_for_its_program() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};

    let root = std::env::temp_dir().join(format!("supersilvia-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let perlin = add_node(&mut app, "perlin");
    let gradient = add_node(&mut app, "cosinegradient");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(perlin, "color"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    // A frame rendered is a program linked: the Perlin's, to compare with.
    let old = render_frames(&mut app, out, 10.0, 1, &root.join("old"))(0);

    app.hold_link(out, true);
    app.apply(Command::Connect {
        from: PortRef::new(gradient, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let destination = root.join("new");
    app.start_render(
        out,
        &RenderSettings {
            fps: 10.0,
            frames: 5,
            warmup: Warmup::Black,
            supersample: 1,
            format: Format::PngSequence,
            destination: destination.clone(),
        },
    )
    .expect("a connected Output renders");
    for _ in 0..60 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(app.rendering(), "still rendering, held");
    assert_eq!(
        app.render_progress().expect("a render runs").written,
        0,
        "nothing is drawn while the program links"
    );

    app.hold_link(out, false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while app.rendering() {
        assert!(
            std::time::Instant::now() < deadline,
            "the render never finished"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    assert!(
        matches!(app.render_outcome(), Some(Outcome::Done { frames: 5, .. })),
        "{:?}",
        app.render_outcome()
    );
    let first = supersilvia::video::png::read(&destination.join("00000.png"))
        .unwrap()
        .rgba;
    assert!(first != old, "the film opens on the new program's picture");
    let _ = std::fs::remove_dir_all(&root);
}

/// **A render puts a feedback Output's frame back.** A Perlin mixed over an Output's own Frame
/// Out, zoomed — a loop whose every frame is built on the last — played live, then paused,
/// which holds it: an Output in a feedback loop draws only when the playhead moves. A render
/// with a black warm-up blanks that Output and draws its own frames into it; on the first live
/// tick after, still paused, the frame the loop builds on is the one before the render, to
/// the bit.
#[test]
fn a_render_puts_a_feedback_outputs_frame_back() {
    use supersilvia::graph::{ControlValue, PortRef};
    use supersilvia::{Command, transport};

    let root = std::env::temp_dir().join(format!("supersilvia-fb-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = supersilvia::App::headless();
    app.attach_gpu_on(gpu::gpu());
    let noise = add_node(&mut app, "perlin");
    let zoom = add_node(&mut app, "zoom");
    let mix = add_node(&mut app, "mix");
    let out = add_node(&mut app, "output");
    for (from, to) in [
        (PortRef::new(out, "frame"), PortRef::new(zoom, "input")),
        (PortRef::new(zoom, "output"), PortRef::new(mix, "a")),
        (PortRef::new(noise, "color"), PortRef::new(mix, "b")),
        (PortRef::new(mix, "output"), PortRef::new(out, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    for (node, key, value) in [(zoom, "zoom", 1.05), (mix, "amount", 0.2)] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.publish_plan();
    for _ in 0..90 {
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    app.transport(transport::Command::Pause);
    for _ in 0..3 {
        app.tick(1.0 / 60.0);
    }
    let frame = |app: &supersilvia::App| app.read_output(out).expect("a frame").2;
    let before = frame(&app);
    app.tick(1.0 / 60.0);
    assert!(frame(&app) == before, "paused, the loop holds");

    let _ = render_frames(&mut app, out, 10.0, 5, &root);
    app.tick(1.0 / 60.0);
    assert!(
        frame(&app) == before,
        "after the render the loop's frame is the one it left"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------------------------------------------------------------------- live recording

/// A project of its own with a checkerboard into an Output, recording at `fps`: the app, the
/// Output and the project folder.
fn recording_patch(
    name: &str,
    fps: f32,
) -> (
    supersilvia::App,
    supersilvia::graph::NodeId,
    std::path::PathBuf,
) {
    use supersilvia::graph::{ControlValue, PortRef};
    use supersilvia::{App, Command};
    let root = std::env::temp_dir().join(format!("supersilvia-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a project folder");
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    app.new_project(root.clone());
    let cb = add_node(&mut app, "checkerboard");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(cb, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.apply(Command::SetControls {
        node: out,
        values: vec![(
            supersilvia::nodes::output::RECORD_FPS,
            ControlValue::Float(fps),
        )],
    })
    .unwrap();
    for _ in 0..4 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    (app, out, root)
}

/// Tick until the recording of `out` has ended and say how.
fn recording_ends(
    app: &mut supersilvia::App,
    out: supersilvia::graph::NodeId,
) -> Result<supersilvia::synth::Recorded, String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        if app.recording_of(out).is_none()
            && let Some(outcome) = app.record_outcome(out)
        {
            return outcome.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the recording never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// **A recording of live ticks is a film as long as the show was.** Two seconds of the show,
/// a hundred and twenty ticks at sixty hertz, recorded at the Output's 30 fps, is sixty
/// frames at the Output's own size, in `recordings/` under the Output's name and the time, and
/// the picture in it is the checkerboard rather than black.
#[test]
fn a_recording_of_live_ticks_is_a_film_as_long_as_the_show() {
    if supersilvia::video::clip::Codec::probe().is_none() {
        eprintln!("no hardware codec pair here; skipping");
        return;
    }
    let (mut app, out, root) = recording_patch("record", 30.0);
    app.start_recording(out)
        .expect("a connected Output records");
    let mut parts = 0;
    for _ in 0..120 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
        let view = app.recording_of(out).expect("recording while it runs");
        assert!(
            view.seconds <= 2.01,
            "the row's clock is the show's: {}",
            view.seconds
        );
        parts = std::fs::read_dir(root.join("recordings")).map_or(0, |d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "part"))
                .count()
        });
    }
    assert_eq!(parts, 1, "written as a .part while it runs");
    // The clock starts on the first tick that asks for a picture, its first slot, so a hundred
    // and twenty ticks are a hundred and nineteen of the show's sixtieths after it.
    let seconds = app.recording_of(out).expect("still recording").seconds;
    assert!(
        (seconds - 119.0 / 60.0).abs() < 0.01,
        "two seconds of the show, less the tick it began on: {seconds}"
    );
    app.stop_recording(out);
    let recorded = recording_ends(&mut app, out).expect("the recording closes");

    let file = recorded.destination.clone();
    let name = file.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(file.parent(), Some(root.join("recordings").as_path()));
    assert!(
        name.starts_with(&format!("output{out}-")) && file.extension().is_some_and(|e| e == "mp4"),
        "the Output, the time, and an .mp4: {name}"
    );
    assert!(
        !file.with_extension("part").exists(),
        "the .part is renamed"
    );
    let info = supersilvia::video::clip::discover(&file).expect("a playable clip");
    assert_eq!(
        (info.width, info.height),
        (1280, 720),
        "the Output's own size"
    );
    assert!((info.fps - 30.0).abs() < 1e-3, "{}", info.fps);
    assert_eq!(info.frames, 60, "two seconds at 30 fps: {info:?}");
    assert_eq!(recorded.frames, 60);
    eprintln!("{} of 60 frames dropped", recorded.dropped);
    assert!(
        recorded.dropped < 60,
        "a GPU that kept up gave most slots a picture of their own"
    );
    assert_eq!(
        app.file_status_shows(),
        Some(file.as_path()),
        "the status line offers to show the film: {}",
        app.file_status()
    );

    let poster = root.join("poster.png");
    supersilvia::video::clip::write_poster(&file, &poster).expect("a frame decodes");
    let picture = supersilvia::video::png::read(&poster).expect("the frame reads back");
    assert!(
        picture.rgba.iter().step_by(4).any(|r| *r > 200),
        "the checkerboard, not black"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// **A recording's first frame is a picture, however long the encoder took to open.** A slow
/// first tick — a hardware encoder opening takes tens of milliseconds — must not put the
/// recording past its first slot before it has asked for it: the clock starts on the tick that
/// first asks, so nothing is dropped and the file does not open on black.
#[test]
fn a_slow_first_tick_drops_no_frame() {
    if supersilvia::video::clip::Codec::probe().is_none() {
        eprintln!("no hardware codec pair here; skipping");
        return;
    }
    let (mut app, out, root) = recording_patch("record-slow-start", 30.0);
    app.start_recording(out)
        .expect("a connected Output records");
    // The tick the encoder opens on, as long as one is in the app.
    app.publish_plan();
    app.tick(0.1);
    for _ in 0..60 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    app.stop_recording(out);
    let recorded = recording_ends(&mut app, out).expect("the recording closes");
    assert_eq!(
        recorded.dropped, 0,
        "no slot went without a picture: {recorded:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// **Deleting the Output mid-recording closes the file**: what was recorded up to the delete
/// is a playable film under its name, and the Output's row has nothing left to say.
#[test]
fn deleting_the_output_mid_recording_finalizes_the_file() {
    use supersilvia::Command;
    if supersilvia::video::clip::Codec::probe().is_none() {
        eprintln!("no hardware codec pair here; skipping");
        return;
    }
    let (mut app, out, root) = recording_patch("record-delete", 30.0);
    app.start_recording(out)
        .expect("a connected Output records");
    // Twenty-nine ticks, and the delete lands on the thirtieth: half a second of the show.
    for _ in 0..29 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    app.apply(Command::RemoveNodes(vec![out])).unwrap();
    let recorded = recording_ends(&mut app, out).expect("the recording closes");
    assert!(
        recorded
            .why
            .as_deref()
            .is_some_and(|w| w.contains("deleted")),
        "it says why it stopped: {:?}",
        recorded.why
    );
    assert!(!recorded.destination.with_extension("part").exists());
    let info = supersilvia::video::clip::discover(&recorded.destination).expect("a playable clip");
    assert_eq!(info.frames, 15, "half a second at 30 fps: {info:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Add `slug` to `app`'s default workspace, returning its id.
fn add_node(app: &mut supersilvia::App, slug: &'static str) -> supersilvia::graph::NodeId {
    let ws = app.graph().default_workspace();
    app.apply(supersilvia::Command::AddNode {
        slug,
        at: emath::Pos2::ZERO,
        workspace: ws,
    })
    .unwrap();
    app.graph().iter().map(|(id, _)| id).max().unwrap()
}

/// Render `frames` frames of `out` at `fps` from playhead zero, with a black warm-up, as a PNG
/// sequence into `dir`: a reader of frame `i`'s bytes.
fn render_frames(
    app: &mut supersilvia::App,
    out: supersilvia::graph::NodeId,
    fps: f64,
    frames: u32,
    dir: &std::path::Path,
) -> impl Fn(u32) -> Vec<u8> + use<> {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;
    app.start_render(
        out,
        &RenderSettings {
            fps,
            frames,
            warmup: Warmup::Black,
            supersample: 1,
            format: Format::PngSequence,
            destination: dir.to_path_buf(),
        },
    )
    .expect("a connected Output renders");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while app.rendering() {
        assert!(
            std::time::Instant::now() < deadline,
            "the render never finished"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    assert!(
        matches!(app.render_outcome(), Some(Outcome::Done { frames: f, .. }) if *f == frames),
        "{:?}",
        app.render_outcome()
    );
    let dir = dir.to_path_buf();
    move |i: u32| {
        supersilvia::video::png::read(&dir.join(format!("{i:05}.png")))
            .unwrap()
            .rgba
    }
}

/// **A render puts the live show back.** A Master Gear held part way through its cycle, an XY
/// Pad with a well dropped on it and an Animation a hand started, each read live, then a
/// render with a warm-up of its own run, which starts every one of them from scratch: on the
/// first live tick after it, the gear is held where it was, the pad has its well, and the
/// animation is a frame further on its travel, not back at its start.
#[test]
fn a_render_puts_the_live_show_back() {
    use supersilvia::graph::{ControlValue, PortRef};
    use supersilvia::nodes::cpu::Touch;
    use supersilvia::{App, Command};
    const FRAME: f32 = 1.0 / 60.0;

    let root = std::env::temp_dir().join(format!("supersilvia-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let board = add_node(&mut app, "checkerboard");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(board, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    let master = add_node(&mut app, "mastergear");
    let pad = add_node(&mut app, "xypad");
    let anim = add_node(&mut app, "animation");
    for (node, key, value) in [(master, "length", 1.0), (anim, "duration", 10.0)] {
        app.apply(Command::SetControl {
            node,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    app.tick(FRAME);
    app.touch(
        pad,
        Touch::Well {
            at: [0.5, 0.5],
            reach: None,
        },
    );
    app.press(PortRef::new(anim, "startStop"), true);
    app.tick(FRAME);
    app.press(PortRef::new(anim, "startStop"), false);
    for _ in 0..30 {
        app.tick(FRAME);
    }
    app.press(PortRef::new(master, "hold"), true);
    app.tick(FRAME);
    app.press(PortRef::new(master, "hold"), false);
    app.tick(FRAME);
    let read = |app: &App, node, key| app.uniform(PortRef::new(node, key)).unwrap();
    let held = read(&app, master, "cycles");
    let travel = read(&app, anim, "output");
    assert!(held > 0.4 && held < 0.6, "held part way: {held}");
    assert!(travel > 0.001 && travel < 0.2, "on its way: {travel}");
    assert_eq!(app.puck(pad).unwrap().wells.len(), 1);

    let _ = render_frames(&mut app, out, 10.0, 3, &root);
    app.tick(FRAME);

    assert_eq!(
        read(&app, master, "cycles"),
        held,
        "the gear is held where it was"
    );
    assert_eq!(
        app.puck(pad).unwrap().wells.len(),
        1,
        "the pad has its well"
    );
    let on = read(&app, anim, "output") - travel;
    assert!(
        on > 0.0 && on < 0.01,
        "the animation is a frame further on: {travel} then {}",
        read(&app, anim, "output")
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A headless app with a GPU holding `source`'s `port` in an Output at 10 fps, with a Master
/// Gear of `master` seconds and, under it, a Ratio Gear at Teeth `p : q` cabled into the
/// source's Time: the app, the Output and the Master Gear.
fn geared(
    source: &'static str,
    port: &'static str,
    master: f32,
    (p, q): (f32, f32),
) -> (
    supersilvia::App,
    supersilvia::graph::NodeId,
    supersilvia::graph::NodeId,
) {
    use supersilvia::graph::{ControlValue, PortRef};
    use supersilvia::{App, Command};

    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let src = add_node(&mut app, source);
    let gear = add_node(&mut app, "mastergear");
    app.apply(Command::SetControl {
        node: gear,
        key: "length",
        value: ControlValue::Float(master),
    })
    .unwrap();
    let under = add_node(&mut app, "ratiogear");
    for (key, value) in [("p", p), ("q", q)] {
        app.apply(Command::SetControl {
            node: under,
            key,
            value: ControlValue::Float(value),
        })
        .unwrap();
    }
    let out = add_node(&mut app, "output");
    app.apply(Command::SetOption {
        node: src,
        key: "clockMode",
        value: "loop".to_string(),
    })
    .unwrap();
    for (from, to) in [
        (PortRef::new(gear, "cycles"), PortRef::new(under, "clock")),
        (
            PortRef::new(under, "cycles"),
            PortRef::new(src, supersilvia::nodes::TIME),
        ),
        (PortRef::new(src, port), PortRef::new(out, "input")),
    ] {
        app.apply(Command::Connect { from, to }).unwrap();
    }
    app.apply(Command::SetControl {
        node: out,
        key: "fps",
        value: ControlValue::Float(10.0),
    })
    .unwrap();
    // Live for a second first, so the programs have linked and the Output draws.
    app.publish_plan();
    for _ in 0..60 {
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (app, out, gear)
}

/// **A loop as long as its Master Gear says closes.** A Cosine Gradient on a Ratio Gear at ÷2
/// under a one-second Master Gear: the master's chains ask for two of its cycles, two seconds,
/// and a render from playhead zero draws frame 20, two seconds in, to the byte of frame zero,
/// while frame 10, a second in, is half a cycle of the palette away. The same length as a GIF
/// through the Output's own writer is twenty frames.
#[test]
fn a_loop_as_long_as_its_master_gear_says_closes() {
    use supersilvia::app::render::{Format, Outcome, RenderSettings};
    use supersilvia::clock::Warmup;

    let root = std::env::temp_dir().join(format!("supersilvia-loop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (mut app, out, master) = geared("cosinegradient", "output", 1.0, (1.0, 2.0));
    let length = supersilvia::nodes::chain::master_length(app.graph(), master);
    assert_eq!(length, Some(2.0), "÷2 asks for two cycles of the master");
    let frame = render_frames(&mut app, out, 10.0, 21, &root.join("png"));
    assert!(frame(0) == frame(20), "two seconds come back to the start");
    assert!(frame(0) != frame(10), "and one second in is not the start");

    let gif = root.join("loop.gif");
    app.start_render(
        out,
        &RenderSettings {
            fps: 10.0,
            frames: 20,
            warmup: Warmup::Black,
            supersample: 1,
            format: Format::Gif,
            destination: gif.clone(),
        },
    )
    .unwrap();
    while app.rendering() {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    assert!(matches!(
        app.render_outcome(),
        Some(Outcome::Done { frames: 20, .. })
    ));
    {
        use image::AnimationDecoder as _;
        let file = std::io::BufReader::new(std::fs::File::open(&gif).unwrap());
        let frames = image::codecs::gif::GifDecoder::new(file)
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(frames.len(), 20);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// **A noise under Repeat closes, Offset and all.** A Perlin repeating every two cells on a
/// Ratio Gear at ×2 under a one-second Master Gear walks its circle once in a second, and with
/// its Offset at −1.7 frame 20 is still frame zero to the byte: Time is taken round its circle
/// before Offset is added, whatever its sign, so a whole turn is no turn at all to the bit.
#[test]
fn a_noise_under_repeat_closes_with_its_offset() {
    use supersilvia::Command;
    use supersilvia::graph::ControlValue;
    let root = std::env::temp_dir().join(format!("supersilvia-repeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (mut app, out, _) = geared("perlin", "color", 1.0, (2.0, 1.0));
    let perlin = app
        .graph()
        .iter()
        .find(|(_, n)| n.def.slug == "perlin")
        .unwrap()
        .0;
    app.apply(Command::SetOption {
        node: perlin,
        key: "repeat",
        value: "2".to_string(),
    })
    .unwrap();
    app.apply(Command::SetControl {
        node: perlin,
        key: supersilvia::nodes::timing::OFFSET,
        value: ControlValue::Float(-1.7),
    })
    .unwrap();
    app.publish_plan();
    for _ in 0..30 {
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let frame = render_frames(&mut app, out, 10.0, 21, &root.join("png"));
    assert!(
        frame(0) == frame(20),
        "two cells on the circle is the start"
    );
    assert!(frame(0) != frame(5), "and half a cell on is not");
    let _ = std::fs::remove_dir_all(&root);
}

/// **The tunnel's flight closes every cycle.** A Tunnel on a Ratio Gear at ÷2 under a
/// one-second Master Gear flies its cycle of 64 units in two seconds, and frame 20 is frame
/// zero to the byte, since its depth is taken round the cycle; three tenths of a second in is
/// another picture. Its wall is a checkerboard, so a frame has hard edges to tell apart.
#[test]
fn the_tunnels_flight_closes_every_cycle() {
    use supersilvia::Command;
    use supersilvia::graph::PortRef;
    let root = std::env::temp_dir().join(format!("supersilvia-tunnel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (mut app, out, _) = geared("tunnel3d", "output", 1.0, (1.0, 2.0));
    let tunnel = app
        .graph()
        .iter()
        .find(|(_, n)| n.def.slug == "tunnel3d")
        .unwrap()
        .0;
    let wall = add_node(&mut app, "checkerboard");
    app.apply(Command::Connect {
        from: PortRef::new(wall, "output"),
        to: PortRef::new(tunnel, "input"),
    })
    .unwrap();
    app.publish_plan();
    for _ in 0..30 {
        app.tick(1.0 / 60.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let frame = render_frames(&mut app, out, 10.0, 21, &root.join("png"));
    assert!(
        frame(0) == frame(20),
        "a flight of 64 units on is the start"
    );
    assert!(frame(0) != frame(3), "and 9.6 units on is not");
    let _ = std::fs::remove_dir_all(&root);
}

/// **A noise on a gear passes 2520 with no seam.** A Perlin repeating every 16 and a Static
/// repeating every 128 rolls, each on a Ratio Gear at ×64 under an eighth-of-a-second Master
/// Gear: 51.2 cycles a frame at 10 fps, so every five frames is 256 cycles, a whole number of
/// either one's repeats, and frame 47, at 2406.4 cycles, is frame 52, at 2662.4, to the byte.
/// Between them the count passes 2520, which neither 16 nor 128 divides: the shader reads the
/// count whole on either side of it.
#[test]
fn a_noise_on_a_gear_passes_2520_with_no_seam() {
    use supersilvia::Command;
    for (slug, repeat) in [("perlin", "16"), ("static", "128")] {
        let root = std::env::temp_dir().join(format!(
            "supersilvia-past-2520-{slug}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (mut app, out, _) = geared(slug, "color", 0.125, (64.0, 1.0));
        let node = app
            .graph()
            .iter()
            .find(|(_, n)| n.def.slug == slug)
            .unwrap()
            .0;
        app.apply(Command::SetOption {
            node,
            key: "repeat",
            value: repeat.to_string(),
        })
        .unwrap();
        app.publish_plan();
        for _ in 0..30 {
            app.tick(1.0 / 60.0);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let frame = render_frames(&mut app, out, 10.0, 53, &root.join("png"));
        assert!(
            frame(47) == frame(52),
            "{slug} at Repeat {repeat}: 256 cycles on, past 2520, is the same picture"
        );
        assert!(frame(47) != frame(48), "{slug}: and a frame on is not");
        let _ = std::fs::remove_dir_all(&root);
    }
}

// -------------------------------------------------------------------- the simulations

/// A headless app with a GPU, holding one `slimemold` — optionally cabled into an Output —
/// with its plan published so the job carries the node's sampling, and the device it draws on.
fn mold_on_the_gpu(with_output: bool) -> (supersilvia::App, supersilvia::graph::NodeId, Gpu) {
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};

    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.clone());
    let mold = add_node(&mut app, "slimemold");
    if with_output {
        let out = add_node(&mut app, "output");
        app.apply(Command::Connect {
            from: PortRef::new(mold, "trail"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    }
    app.publish_plan();
    (app, mold, gpu)
}

/// Tick with nothing in the tick until every tick the world was handed has run, and read it.
/// A tick of no time takes no step and moves no maximum, so it changes nothing it draws.
fn settled(
    app: &mut supersilvia::App,
    port: supersilvia::graph::PortRef,
) -> supersilvia::render::SimReadback {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let read = app.read_simulation(port).expect("a world on the GPU");
        if read.queued == 0 {
            return read;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the kernels never linked"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
        app.tick(0.0);
    }
}

/// **The same seed and the same clock make the same world.** Two apps, each holding a
/// `slimemold` with the same id, ticked through one uneven sequence of `dt`s — a stall, a tick
/// of no time, a late one — hold the same agents, the same field and the same picture to the
/// last bit. That is what lets an offline render, which steps virtual time at whatever speed
/// the GPU manages, make the world live play made.
#[test]
fn a_slime_mold_is_the_same_world_twice_for_one_seed_and_one_clock() {
    use supersilvia::graph::PortRef;

    let dts = [0.016, 0.017, 0.1, 0.0, 0.004, 0.033, 0.0167, 0.05];
    let run = || {
        let (mut app, id, _) = mold_on_the_gpu(false);
        let port = PortRef::new(id, "trail");
        for _ in 0..6 {
            for dt in dts {
                app.tick(dt);
            }
        }
        let read = settled(&mut app, port);
        let steps = app.simulation(port).expect("published").steps;
        (read, steps)
    };
    let (first, steps) = run();
    let (second, again) = run();

    assert_eq!(steps, again);
    assert!(steps > 100, "the world moved: {steps} steps");
    assert_eq!(first.size, 192, "silvia's default grid");
    assert_eq!(first.agents.len(), 7372, "and a fifth of it alive");
    assert!(
        first.field.iter().any(|v| *v > 0.0),
        "the agents left scent"
    );
    assert!(
        first.picture.as_chunks::<4>().0.iter().any(|p| p[0] > 0),
        "and the picture shows it"
    );
    assert!(
        first.agents == second.agents,
        "the agents went somewhere else the second time"
    );
    assert!(first.field == second.field, "the field differs");
    assert_eq!(first.picture, second.picture, "the picture differs");
}

/// A digest of a world: its agents, its field and its picture, bit for bit.
fn digest(read: &supersilvia::render::SimReadback) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            hash = (hash ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for agent in &read.agents {
        for v in agent {
            eat(&v.to_bits().to_le_bytes());
        }
    }
    for v in &read.field {
        eat(&v.to_bits().to_le_bytes());
    }
    eat(&read.picture);
    hash
}

/// **A render puts the slime mold's world back.** A mold grown live for a second, then a render
/// of the Output it feeds, which grows a world of its own from the mold's seed: after it, the
/// live world is where the render found it — its agents, its field and its picture to the bit —
/// and a live tick steps it on from there.
#[test]
fn a_render_puts_the_slime_molds_world_back() {
    use supersilvia::graph::PortRef;

    let root = std::env::temp_dir().join(format!("supersilvia-mold-back-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (mut app, mold, _) = mold_on_the_gpu(true);
    let out = app.graph().iter().find(|(_, n)| n.def.is_output).unwrap().0;
    let port = PortRef::new(mold, "trail");
    for _ in 0..60 {
        app.tick(1.0 / 60.0);
    }
    let before = settled(&mut app, port);
    let steps = app.simulation(port).expect("published").steps;
    assert!(steps > 100, "the world grew: {steps} steps");

    let _ = render_frames(&mut app, out, 10.0, 4, &root);
    let after = settled(&mut app, port);
    assert_eq!(
        digest(&after),
        digest(&before),
        "the live world is back as the render found it"
    );

    app.tick(1.0 / 60.0);
    let on = settled(&mut app, port);
    assert_ne!(digest(&on), digest(&before), "and steps on from there");
    let _ = std::fs::remove_dir_all(&root);
}

/// **A new grid reallocates the world and keeps its pattern.** A world of 192 halved to 96
/// is a new field, resampled nearest from the old one — each new cell is the old cell its
/// center falls in — and a new picture texture; a smaller population keeps the first of its
/// agents where they stood.
#[test]
fn a_new_grid_reallocates_the_world_and_keeps_its_pattern() {
    use supersilvia::Command;
    use supersilvia::graph::PortRef;

    let (mut app, id, _) = mold_on_the_gpu(false);
    let port = PortRef::new(id, "trail");
    for _ in 0..30 {
        app.tick(1.0 / 60.0);
    }
    let before = settled(&mut app, port);

    app.apply(Command::SetOption {
        node: id,
        key: "gridScale",
        value: "6".to_string(),
    })
    .expect("a choice of the option");
    // No time in the tick, so no step lands on the resampled field before it is read.
    app.tick(0.0);
    let after = settled(&mut app, port);
    assert_eq!(after.size, 96);
    assert_eq!(after.field.len(), 96 * 96);
    assert_eq!(after.agents.len(), 96 * 96 / 5, "a fifth of the new grid");
    for (x, y) in [(0usize, 0usize), (10, 40), (95, 95), (48, 3)] {
        assert_eq!(
            after.field[y * 96 + x],
            before.field[(2 * y + 1) * 192 + 2 * x + 1],
            "cell ({x}, {y}) is not the old cell its center fell in"
        );
    }
    for (a, b) in after.agents.iter().zip(&before.agents) {
        assert_eq!(
            [a[0], a[1], a[2]],
            [b[0] * 0.5, b[1] * 0.5, b[2]],
            "an agent stayed where it stood on the old grid"
        );
    }

    app.apply(Command::SetOption {
        node: id,
        key: "population",
        value: "1".to_string(),
    })
    .expect("a choice of the option");
    app.tick(0.0);
    let fewer = settled(&mut app, port);
    assert_eq!(fewer.agents.len(), 96 * 96 / 100);
    assert_eq!(
        fewer.agents[..],
        after.agents[..fewer.agents.len()],
        "the first of them, where they stood"
    );
}

/// **An Output samples the world the tick left.** `slimemold`'s `trail` cabled straight into
/// an Output draws red where the scent is and nothing where it is not — the simulation's
/// picture bound to a consumer as an upload would be, with nothing read back or uploaded.
#[test]
fn an_output_samples_the_picture_the_simulation_drew() {
    use supersilvia::graph::PortRef;

    let (mut app, id, gpu) = mold_on_the_gpu(true);
    let port = PortRef::new(id, "trail");
    for _ in 0..60 {
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    let world = settled(&mut app, port);
    let lit = world
        .picture
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 0)
        .count();
    assert!(lit > 0, "the picture is empty");

    let out = app
        .graph()
        .iter()
        .map(|(id, _)| id)
        .max()
        .expect("the Output");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let frame = loop {
        app.publish_plan();
        app.tick(0.0);
        let shown = app
            .ticked()
            .and_then(|s| s.published().outputs.get(&out).cloned())
            .map(|picture| gpu::rgba_of(&gpu, picture.texture.texture()).concat());
        if let Some(px) = shown
            && px.as_chunks::<4>().0.iter().any(|p| p[0] > 0)
        {
            break px;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the Output never drew the world"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    let red = frame.as_chunks::<4>().0.iter().filter(|p| p[0] > 0).count();
    assert!(
        red > 1280 * 720 / 100,
        "{red} pixels of the Output carry scent"
    );
    assert!(
        frame
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[1] == 0 && p[2] == 0),
        "and only in red, as the picture holds it"
    );
}

/// **A project reopened in the running app has a world of its own on the GPU.** Its
/// `slimemold` holds the id it had, and the world is born again rather than carried on: the
/// first tick after the reopen makes the world a fresh app's first tick makes, to the last
/// bit, and the Output the old world was drawn into has no frame left to sample.
#[test]
fn a_reopened_project_starts_a_fresh_world_and_keeps_no_old_frame() {
    use supersilvia::graph::PortRef;

    let (mut app, id, _) = mold_on_the_gpu(true);
    let port = PortRef::new(id, "trail");
    let out = app.graph().iter().map(|(id, _)| id).max().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.read_output(out).is_none() || app.simulation(port).map_or(0, |s| s.steps) < 30 {
        assert!(
            std::time::Instant::now() < deadline,
            "the Output never drew"
        );
        app.publish_plan();
        app.tick(1.0 / 60.0);
    }
    app.save_project().unwrap();
    let root = app.project().root().to_path_buf();

    app.open_project(root.clone());
    app.tick(0.0);
    assert!(
        app.read_output(out).is_none(),
        "the old project's Output is gone from the renderer"
    );
    let reopened = settled(&mut app, port);

    let mut fresh = supersilvia::App::headless();
    fresh.attach_gpu_on(gpu::gpu());
    fresh.open_project(root);
    fresh.tick(0.0);
    let born = settled(&mut fresh, port);
    assert_eq!(
        app.simulation(port).map(|s| s.steps),
        fresh.simulation(port).map(|s| s.steps)
    );
    assert!(reopened.agents == born.agents, "the agents were carried on");
    assert!(reopened.field == born.field, "the field was carried on");
    assert_eq!(reopened.picture, born.picture, "and so was the picture");
}

/// **Clear is silvia's**: the scent and the stamp of where anyone stands go, the agents stay
/// where they are, and the picture says so on the tick it was pressed. Before it, the picture
/// is a fraction of the field's own maximum and never a saturated sheet.
#[test]
fn clear_empties_the_field_and_leaves_the_agents() {
    use supersilvia::graph::PortRef;

    let (mut app, id, _) = mold_on_the_gpu(false);
    let port = PortRef::new(id, "trail");
    for _ in 0..60 {
        app.tick(1.0 / 60.0);
    }
    let before = settled(&mut app, port);
    let texels = before.picture.as_chunks::<4>().0;
    assert!(texels.iter().any(|p| p[0] > 0), "the field never got wet");
    let full = texels.iter().filter(|p| p[0] == 255).count();
    assert!(full < texels.len() / 2, "{full} cells are saturated");
    assert!(
        before.picture.as_chunks::<4>().0.iter().any(|p| p[3] > 0),
        "somebody is standing somewhere"
    );

    // A tick of no time, so no step deposits over the clear before it is read.
    app.press(PortRef::new(id, "clearTrails"), true);
    app.tick(0.0);
    app.press(PortRef::new(id, "clearTrails"), false);
    let after = settled(&mut app, port);
    assert!(after.field.iter().all(|v| *v == 0.0), "scent survived");
    assert!(
        after
            .picture
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[0] == 0 && p[3] == 0),
        "the picture still shows scent or somebody standing"
    );
    assert_eq!(after.agents, before.agents, "the agents went with it");
}

/// **A reset world is a new one.** Reset in the middle of a run — what an offline render does
/// to every node before frame zero — and the next frames make the world a fresh node makes over
/// the same frames, on the GPU as well as in the tick.
#[test]
fn a_reset_mold_steps_the_world_a_fresh_one_does() {
    use supersilvia::graph::PortRef;

    let (mut fresh, id, _) = mold_on_the_gpu(false);
    let port = PortRef::new(id, "trail");
    for _ in 0..20 {
        fresh.tick(1.0 / 30.0);
    }
    let expected = settled(&mut fresh, port);

    let (mut reset, same, _) = mold_on_the_gpu(false);
    assert_eq!(same, id);
    for _ in 0..37 {
        reset.tick(1.0 / 45.0);
    }
    settled(&mut reset, port);
    reset.reset_cpu();
    for _ in 0..20 {
        reset.tick(1.0 / 30.0);
    }
    let got = settled(&mut reset, port);
    assert!(got.agents == expected.agents, "the agents differ");
    assert!(got.field == expected.field, "the field differs");
    assert_eq!(got.picture, expected.picture, "the picture differs");
    assert_eq!(got.state, expected.state, "the running maximum differs");
}

/// **A preset nudges the world, on the GPU, as silvia's `_nudgeSimulation` does.** The
/// preset bar's pulse, held for one tick of no time: the field comes back at nine tenths of
/// what it was, one agent in eight — the first and every eighth after it — is somewhere else,
/// and the other seven are where they stood. Randomize ends in the same nudge through the
/// same pass pair.
#[test]
fn a_presets_pulse_nudges_the_world_on_the_gpu() {
    use supersilvia::graph::PortRef;

    let (mut app, id, _) = mold_on_the_gpu(false);
    let port = PortRef::new(id, "trail");
    for _ in 0..10 {
        app.tick(1.0 / 30.0);
    }
    let before = settled(&mut app, port);
    let pulse = PortRef::new(id, supersilvia::nodes::slimemold::NUDGE);
    app.press(pulse, true);
    app.tick(0.0);
    app.press(pulse, false);
    let after = settled(&mut app, port);

    let total = |field: &[f32]| field.iter().map(|v| f64::from(*v)).sum::<f64>();
    let (was, is) = (total(&before.field), total(&after.field));
    assert!(was > 0.0, "the world has scent to knock back");
    assert!((is / was - 0.9).abs() < 1e-4, "{is} of {was}");
    assert_eq!(before.agents.len(), after.agents.len());
    let moved = |i: usize| before.agents[i] != after.agents[i];
    let thrown = (0..before.agents.len()).filter(|i| moved(*i)).count();
    assert!(
        (0..before.agents.len()).all(|i| i % 8 == 0 || !moved(i)),
        "only every eighth agent moves"
    );
    assert!(
        thrown * 8 > before.agents.len() * 9 / 10,
        "and nearly all of those do: {thrown} of {}",
        before.agents.len()
    );
}

// ------------------------------------------------------------------------ idle Outputs

/// A headless app with a GPU and two open tabs, the first one showing, and a moving picture
/// on the second — a `perlin`, whose phase its CPU half integrates, into an Output — which
/// is therefore idle; and the device it draws on. Every app this makes is the same graph
/// under the same ids.
fn idle_picture() -> (
    Drawn,
    supersilvia::graph::WorkspaceId,
    supersilvia::graph::NodeId,
) {
    use supersilvia::graph::{PortRef, WorkspaceKind};
    use supersilvia::project::Active;
    use supersilvia::{App, Command};

    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.clone());
    let first = app.graph().default_workspace();
    app.apply(Command::AddWorkspace {
        name: "Second".to_string(),
        kind: WorkspaceKind::Video,
        layout: supersilvia::graph::LayoutMode::default(),
        seed: supersilvia::command::Seed::Empty,
    })
    .unwrap();
    let second = app.graph().workspaces().last().unwrap().id;
    app.open_workspace(second);
    let add = |app: &mut App, slug| {
        app.apply(Command::AddNode {
            slug,
            at: emath::Pos2::ZERO,
            workspace: second,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    let noise = add(&mut app, "perlin");
    let out = add(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(noise, "color"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();
    app.activate(Active::Workspace(first));
    (Drawn { app, gpu }, second, out)
}

/// An app and the device its renderer draws on.
struct Drawn {
    app: supersilvia::App,
    gpu: Gpu,
}

/// One tick of each app in lockstep, the GPU finished after each so the latest frame is the
/// one the tick drew and nothing of it is still in flight.
fn lockstep(apps: &mut [&mut Drawn]) {
    for drawn in apps.iter_mut() {
        drawn.app.publish_plan();
        drawn.app.tick(1.0 / 60.0);
        gpu::drain(&drawn.gpu);
    }
}

/// Tick both until each has linked the Output's program, so from here on a draw is a draw.
fn until_linked(idle: &mut Drawn, seen: &mut Drawn, out: supersilvia::graph::NodeId) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        lockstep(&mut [&mut *idle, &mut *seen]);
        let linked = |app: &supersilvia::App| {
            app.snapshot().published.outputs.contains_key(&out)
                && app.snapshot().render.linking.is_empty()
        };
        if linked(&idle.app) && linked(&seen.app) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the Output's program never linked"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// What a viewer of this Output is shown: the published frame, as half floats.
fn shown(drawn: &Drawn, out: supersilvia::graph::NodeId) -> Vec<u8> {
    let picture = drawn.app.snapshot().published.outputs[&out].clone();
    gpu::bytes_of(&drawn.gpu, picture.texture.texture())
}

/// **An idle Output is shown its last frame, and switched to draws what continuous drawing
/// would have.** Two apps holding the same patch tick in lockstep: in one the Output's tab
/// is looked at throughout, in the other it is idle until the switch. Idle, it drew once —
/// its first picture, once its program had landed — and nothing after, so a viewer is shown
/// that picture and never the blank ring it was made with. The picture is a function of the
/// clock, so the frame drawn on the tick after the switch is the other's to the bit.
#[test]
fn a_tab_switched_to_draws_what_continuous_drawing_draws() {
    use supersilvia::project::Active;

    let (mut idle, second, out) = idle_picture();
    let (mut seen, _, _) = idle_picture();
    seen.app.activate(Active::Workspace(second));
    until_linked(&mut idle, &mut seen, out);

    for _ in 0..5 {
        lockstep(&mut [&mut idle, &mut seen]);
    }
    let first = shown(&idle, out);
    assert!(
        first.iter().any(|b| *b != 0),
        "a viewer of the idle Output is shown its first picture, not a blank ring"
    );
    let latest = idle
        .app
        .read_output(out)
        .expect("an idle Output has its ring");
    for _ in 0..3 {
        lockstep(&mut [&mut idle, &mut seen]);
        assert_eq!(
            shown(&idle, out),
            first,
            "and only that: idle, it draws nothing"
        );
    }
    assert_eq!(idle.app.read_output(out).as_ref(), Some(&latest));
    let before = seen.app.read_output(out).expect("drawn");
    assert_ne!(before.2, latest.2, "while the one looked at has moved on");

    idle.app.activate(Active::Workspace(second));
    lockstep(&mut [&mut idle, &mut seen]);
    let now = seen.app.read_output(out).expect("drawn");
    assert_ne!(now.2, before.2, "the noise moves from tick to tick");
    assert_eq!(
        idle.app.read_output(out).expect("drawn"),
        now,
        "switched to, it draws the frame continuous drawing draws on the same tick"
    );
    for _ in 0..3 {
        assert!(
            shown(&idle, out).iter().any(|b| *b != 0),
            "and a viewer is shown a picture on every tick"
        );
        lockstep(&mut [&mut idle, &mut seen]);
    }
}

/// **A save's thumbnail and a Snap of an idle Output draw it for the tick they ask on,** and
/// on no other: the picture is the frame continuous drawing gives on that tick, and the tick
/// after, the Output is idle again.
#[test]
fn a_thumbnail_and_a_snap_of_an_idle_output_draw_it_on_demand() {
    use supersilvia::graph::PortRef;
    use supersilvia::project::Active;
    use supersilvia::video::png;

    let (mut idle, second, out) = idle_picture();
    let (mut seen, _, _) = idle_picture();
    seen.app.activate(Active::Workspace(second));
    until_linked(&mut idle, &mut seen, out);
    for _ in 0..5 {
        lockstep(&mut [&mut idle, &mut seen]);
    }
    assert_ne!(
        idle.app.read_output(out).unwrap(),
        seen.app.read_output(out).unwrap(),
        "idle: its latest frame is its first picture, not this tick's"
    );

    // The save asks for a picture of every open workspace; the second's is this Output's.
    idle.app.save_project().unwrap();
    lockstep(&mut [&mut idle, &mut seen]);
    assert_eq!(
        idle.app.read_output(out).unwrap(),
        seen.app.read_output(out).unwrap(),
        "drawn on the tick the thumbnail asked on, as continuous drawing draws it"
    );
    let drawn = idle.app.read_output(out).unwrap();
    lockstep(&mut [&mut idle, &mut seen]);
    assert_eq!(
        idle.app.read_output(out).unwrap(),
        drawn,
        "and idle again on the tick after"
    );
    let thumb = idle
        .app
        .project()
        .thumbnail_path(second)
        .expect("a workspace file to put it beside");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !thumb.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "no thumbnail was written"
        );
        lockstep(&mut [&mut idle, &mut seen]);
    }
    let image = png::read(&thumb).expect("the thumbnail reads back");
    assert!(
        image
            .rgba
            .chunks(4)
            .any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0),
        "a picture of the noise, not of a frame never drawn"
    );

    // A Snap, the same way: a hand on the button of an idle Output.
    idle.app.press(PortRef::new(out, "snap"), true);
    lockstep(&mut [&mut idle, &mut seen]);
    idle.app.press(PortRef::new(out, "snap"), false);
    assert_eq!(
        idle.app.read_output(out).unwrap(),
        seen.app.read_output(out).unwrap(),
        "drawn on the tick the Snap asked on"
    );
    let snaps = idle.app.project().root().join("snaps");
    let file = loop {
        if let Some(entry) = std::fs::read_dir(&snaps)
            .ok()
            .and_then(|d| d.flatten().next())
        {
            break entry.path();
        }
        assert!(std::time::Instant::now() < deadline, "no snap was written");
        lockstep(&mut [&mut idle, &mut seen]);
    };
    let image = png::read(&file).expect("the snap reads back");
    assert_eq!((image.width, image.height), (1280, 720));
    assert!(
        image
            .rgba
            .chunks(4)
            .any(|p| p[0] > 0 || p[1] > 0 || p[2] > 0),
        "the noise, at full size"
    );
    let _ = std::fs::remove_dir_all(idle.app.project().root());
    let _ = std::fs::remove_dir_all(seen.app.project().root());
}

// ----------------------------------------------------------------------- the editor

/// **A node's hollow shadow is the whole shadow, to the pixel**, under the opaque body it is
/// cut to fit: at several zooms, offsets that throw the shadow past the body on any side, a
/// position off the pixel grid and a scale that is not one.
///
/// Both are painted by egui_wgpu, as the editor paints them, over a ground and under the body,
/// and read back byte for byte.
#[test]
fn a_hollow_shadow_paints_what_the_whole_one_did() {
    use egui::{Color32, CornerRadius, Shape};
    const W: u32 = 360;
    const H: u32 = 420;
    const GROUND: [u8; 3] = [40, 44, 52];
    const BODY: [u8; 3] = [20, 22, 26];

    let gpu = gpu::gpu();
    let mut renderer = egui_wgpu::Renderer::new(
        gpu.device(),
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::PREDICTABLE,
    );
    for ppp in [1.0, 1.25] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        // One pass, so the fonts and the white texel the meshes sample exist.
        let first = ctx.run_ui(egui::RawInput::default(), |_| {});
        let mut textures = first.textures_delta;
        let screen =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(W as f32 / ppp, H as f32 / ppp));
        let base = egui::Visuals::dark().window_shadow;
        for (zoom, offset, at) in [
            (1.0, base.offset, egui::pos2(60.0, 40.0)),
            (0.6, base.offset, egui::pos2(60.3, 40.7)),
            (1.4, [-30, -25], egui::pos2(90.0, 80.0)),
            (1.0, [40, 0], egui::pos2(50.5, 60.25)),
            (0.35, [3, 7], egui::pos2(70.0, 70.0)),
        ] {
            let scaled = |v: f32| (v * zoom).round() as u8;
            let shadow = egui::Shadow {
                offset,
                blur: scaled(f32::from(base.blur)),
                spread: scaled(f32::from(base.spread)),
                color: base.color,
            };
            let body = egui::Rect::from_min_size(at, egui::vec2(200.0, 260.0) * zoom);
            let (top, foot) = ((12.0 * zoom).round() as u8, (6.0 * zoom).round() as u8);
            let corners = CornerRadius {
                nw: top,
                ne: top,
                sw: foot,
                se: foot,
            };
            let covered = body.shrink(f32::from(top.max(foot)) + 1.0);
            let whole = shadow.as_shape(body, corners);
            let hollow = supersilvia::ui::node_widget::hollow(&ctx, whole.clone(), covered);
            assert!(
                matches!(hollow, Shape::Mesh(_)),
                "zoom {zoom}: the shadow was cut rather than left whole"
            );
            let mut paint = |shadow: Shape, textures: &mut egui::TexturesDelta| {
                let shapes = [
                    Shape::rect_filled(screen, 0.0, Color32::from_rgb(40, 44, 52)),
                    shadow,
                    Shape::rect_filled(body, corners, Color32::from_rgb(20, 22, 26)),
                ]
                .into_iter()
                .map(|shape| egui::epaint::ClippedShape {
                    clip_rect: screen,
                    shape,
                })
                .collect();
                let primitives = ctx.tessellate(shapes, ppp);
                paint_egui(&gpu, &mut renderer, [W, H], ppp, &primitives, textures)
            };
            let was = paint(Shape::Rect(whole), &mut textures);
            let is = paint(hollow, &mut textures);
            let differ = was
                .chunks(4)
                .zip(is.chunks(4))
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                differ, 0,
                "ppp {ppp}, zoom {zoom}, offset {offset:?}: {differ} pixels differ"
            );
            // And there was a shadow to compare: it darkened the ground somewhere.
            assert!(
                is.chunks(4).any(|p| p[..3] != GROUND && p[..3] != BODY),
                "ppp {ppp}, zoom {zoom}: no shadow was painted at all"
            );
        }
    }
}

/// **The editor times its own painting**, while the Status box is open: a timestamp ahead of
/// egui_wgpu's pass and one inside it, placed by the frame's first and last paint callbacks,
/// read frames later without waiting, and a reading that lands in the box's GPU row. Closed,
/// it marks nothing and forgets the reading.
///
/// The real `App`, run and painted by hand the way eframe runs and paints it. Skipped where the
/// device has no timestamps inside a pass, which is where the editor has no GPU figure.
#[test]
fn the_editor_times_its_own_painting_while_the_status_box_is_open() {
    use eframe::App as _;
    const SIZE: [u32; 2] = [640, 400];

    let gpu = gpu::gpu();
    if !gpu
        .device()
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    {
        eprintln!("this device has no timestamps inside a pass; skipping");
        return;
    }
    let mut app = supersilvia::App::headless();
    app.attach_viewer_on(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let ctx = egui::Context::default();
    supersilvia::ui::theme::apply(&ctx, &supersilvia::ui::theme::Theme::default());
    let mut renderer = egui_wgpu::Renderer::new(
        gpu.device(),
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::default(),
    );
    let mut frame = eframe::Frame::_new_kittest();
    let mut time = 0.0;
    let mut paint = |app: &mut supersilvia::App, frames: usize| {
        for _ in 0..frames {
            time += 0.01;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
                )),
                time: Some(time),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut frame));
            let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
            // A test may wait: each frame is finished before the next, so its marks have
            // landed by the time a later one reads them.
            paint_egui(
                &gpu,
                &mut renderer,
                SIZE,
                out.pixels_per_point,
                &primitives,
                &mut out.textures_delta,
            );
            gpu::drain(&gpu);
        }
    };

    paint(&mut app, 4);
    assert_eq!(
        app.paint_gpu(),
        None,
        "the box is closed, so nothing was timed"
    );

    app.set_show_status_box(true);
    paint(&mut app, 6);
    let (now, avg, worst) = app
        .paint_gpu()
        .expect("a reading, frames after the first mark");
    assert!(
        now > 0.0 && avg > 0.0,
        "the painting took some time: {now} ms"
    );
    assert!(
        worst >= now,
        "the worst is at least this frame: {worst} against {now}"
    );
    assert!(now < 1000.0, "a frame, not a wrapped counter: {now} ms");

    app.set_show_status_box(false);
    paint(&mut app, 2);
    assert_eq!(app.paint_gpu(), None, "closed, the reading is forgotten");
}

/// **An Output's node holds its picture inside itself while its resolution changes.** The
/// node takes the new shape on the frame after the change, from the option, and the picture
/// published is the last one drawn at the old size for a tick or two after that: on every one
/// of those frames the band shows a picture, cropped into the new shape, and every pixel of the
/// canvas outside the band is what it is once the new picture has landed — the node's rows,
/// the other node, the cable and the ground untouched by a picture of the old shape.
///
/// The real `App` with its renderer, painted by hand frame by frame the way eframe paints it,
/// through a tall shape, back to a wide one and to a square.
#[test]
fn an_outputs_picture_stays_inside_its_node_while_its_resolution_changes() {
    use eframe::App as _;
    use supersilvia::graph::PortRef;
    use supersilvia::ui::canvas::{Layouts, Region};
    use supersilvia::{App, Command};
    const SIZE: [u32; 2] = [1200, 900];

    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.clone());
    app.attach_viewer_on(&gpu, wgpu::TextureFormat::Rgba8Unorm);
    let ctx = egui::Context::default();
    supersilvia::ui::theme::apply(&ctx, &supersilvia::ui::theme::Theme::default());
    let mut renderer = egui_wgpu::Renderer::new(
        gpu.device(),
        wgpu::TextureFormat::Rgba8Unorm,
        egui_wgpu::RendererOptions::PREDICTABLE,
    );
    let ws = app.graph().default_workspace();
    let mut add = |slug, at| {
        app.apply(Command::AddNode {
            slug,
            at,
            workspace: ws,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    let checks = add("checkerboard", emath::pos2(20.0, 40.0));
    let out = add("output", emath::pos2(260.0, 40.0));
    app.apply(Command::Connect {
        from: PortRef::new(checks, "output"),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let mut frame = eframe::Frame::_new_kittest();
    let mut time = 0.0;
    let mut paint = |app: &mut App| {
        time += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            )),
            time: Some(time),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| app.ui(ui, &mut frame));
        let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
        let bytes = paint_egui(
            &gpu,
            &mut renderer,
            SIZE,
            out.pixels_per_point,
            &primitives,
            &mut out.textures_delta,
        );
        gpu::drain(&gpu);
        bytes
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !(app.snapshot().published.outputs.contains_key(&out)
        && app.snapshot().render.linking.is_empty())
    {
        paint(&mut app);
        assert!(
            std::time::Instant::now() < deadline,
            "the Output's program never linked"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    // Long enough for the chrome's own fades to have finished, so only a picture differs.
    for _ in 0..60 {
        paint(&mut app);
    }

    let at = |bytes: &[u8], x: u32, y: u32| {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        [bytes[i], bytes[i + 1], bytes[i + 2]]
    };
    for (size, wanted) in [
        ("720x1280", (720, 1280)),
        ("1920x1080", (1920, 1080)),
        ("1080x1080", (1080, 1080)),
    ] {
        app.apply(Command::SetOption {
            node: out,
            key: "resolution",
            value: size.to_string(),
        })
        .unwrap();
        let frames: Vec<Vec<u8>> = (0..5).map(|_| paint(&mut app)).collect();
        let shown = &app.snapshot().published.outputs[&out];
        assert_eq!(
            (shown.width, shown.height),
            wanted,
            "{size}: the picture of the new size has landed by the last frame"
        );

        let (t, origin) = (app.canvas_transform(), app.canvas_origin());
        let laid = Layouts::one(app.graph(), out);
        let band = t
            .to_screen_rect(origin, laid.find(out).unwrap().region(Region::Declared(0)))
            .expand(1.0);
        let canvas =
            egui::Rect::from_min_size(origin, egui::vec2(app.canvas_width(), app.canvas_height()));
        let settled = frames.last().unwrap();
        for (i, bytes) in frames[..frames.len() - 1].iter().enumerate() {
            let mut lit = false;
            for y in canvas.min.y as u32..(canvas.max.y as u32).min(SIZE[1]) {
                for x in canvas.min.x as u32..(canvas.max.x as u32).min(SIZE[0]) {
                    let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                    if band.contains(p) {
                        lit |= at(bytes, x, y).iter().all(|c| *c > 200);
                        continue;
                    }
                    assert_eq!(
                        at(bytes, x, y),
                        at(settled, x, y),
                        "{size}, frame {i} after the change: ({x}, {y}) is outside the band \
                         {band:?} and was drawn over"
                    );
                }
            }
            assert!(
                lit,
                "{size}, frame {i} after the change: the band shows a picture, not the ground"
            );
        }
    }
}

// -------------------------------------------------------------------------- the painting

/// **What is painted is what the patch gets.** A `drawingcanvas` cabled into an Output: its
/// left half painted white is the Output's left, sampled through the picture the node
/// publishes, and a stroke painted after it reaches the Output too.
#[test]
fn an_output_samples_the_painting() {
    use supersilvia::graph::{PortRef, Value};
    use supersilvia::nodes::drawingcanvas::{self, Sheet};
    use supersilvia::{App, Command};

    let gpu = gpu::gpu();
    let mut app = App::headless();
    app.attach_gpu_on(gpu.clone());
    let canvas = add_node(&mut app, "drawingcanvas");
    let out = add_node(&mut app, "output");
    app.apply(Command::Connect {
        from: PortRef::new(canvas, drawingcanvas::OUTPUT),
        to: PortRef::new(out, "input"),
    })
    .unwrap();

    let paint = |app: &mut App, sheet: Sheet| {
        app.apply(Command::SetValue {
            node: canvas,
            key: drawingcanvas::PAINTING,
            value: Value::Painting(sheet.into_painting(0)),
        })
        .unwrap();
    };
    // The left half white, the right half as it was.
    let mut sheet = Sheet::of(app.graph().get(canvas).unwrap());
    for y in 0..512 {
        for x in 0..256 {
            let i = (y * 512 + x) * 4;
            sheet.rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    paint(&mut app, sheet);

    // The middle row of the Output: the picture is square and centered, so a quarter of its
    // height either side of the middle is inside it.
    let row = |app: &App| -> Option<(u8, u8)> {
        let picture = app
            .ticked()
            .and_then(|s| s.published().outputs.get(&out).cloned())?;
        let texture = picture.texture.texture();
        let (w, h) = (texture.width() as usize, texture.height() as usize);
        let px = gpu::rgba_of(&gpu, texture);
        let y = h / 2;
        Some((px[y * w + w / 2 - h / 4][0], px[y * w + w / 2 + h / 4][0]))
    };
    let until = |app: &mut App, want: &dyn Fn((u8, u8)) -> bool, what: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.publish_plan();
            app.tick(1.0 / 60.0);
            if row(app).is_some_and(want) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{what}: {:?}",
                row(app)
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    until(
        &mut app,
        &|(left, right)| left > 200 && right < 20,
        "the Output never drew the painting",
    );

    // A stroke down the right half lands there too.
    let mut sheet = Sheet::of(app.graph().get(canvas).unwrap());
    sheet.fill([255, 255, 255, 255], (400.0, 256.0));
    paint(&mut app, sheet);
    until(
        &mut app,
        &|(left, right)| left > 200 && right > 200,
        "the second stroke never reached the Output",
    );
}

/// **A workspace's pass draws every varying output on it, whatever it reaches, and measures
/// every tap on it.** A workspace with no Output at all, a `worldcoordinates` and a
/// `checkerboard` on it and nothing connected but a tap on the checkerboard: each port's
/// thumbnail is its own function over the frame, so `x` reads each cell's own `x` across
/// `-16/9` to `16/9` and `y` its `y` from `-1` to `1`, bottom row first, the checkerboard's
/// color is two colors, and the tap — its slot first in the same buffer — reads every point of
/// its grid, half of them light. An Output's own `frame` has no thumbnail.
#[test]
fn a_workspace_pass_thumbnails_every_varying_output_whatever_it_reaches() {
    use supersilvia::compile::{THUMB_CELLS, THUMB_H, THUMB_W};
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};

    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let ws = app.graph().default_workspace();
    let mut add = |slug| {
        app.apply(Command::AddNode {
            slug,
            at: emath::Pos2::ZERO,
            workspace: ws,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    let coords = add("worldcoordinates");
    let checks = add("checkerboard");
    let out = add("output");
    let tap = add("tap");
    app.apply(Command::Connect {
        from: PortRef::new(checks, "output"),
        to: PortRef::new(tap, "input"),
    })
    .unwrap();
    let (x, y, color) = (
        PortRef::new(coords, "x"),
        PortRef::new(coords, "y"),
        PortRef::new(checks, "output"),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        let thumbs = &app.snapshot().thumbs;
        let measured = app
            .readback(tap)
            .is_some_and(|w| supersilvia::nodes::tap::decode(w).count > 0);
        if measured && [x, y, color].iter().all(|p| thumbs.contains_key(p)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the pass never drew: {:?}",
            thumbs.keys().collect::<Vec<_>>()
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let thumbs = &app.snapshot().thumbs;
    let cells = |p: PortRef| -> Vec<u32> { thumbs[&p].words.clone() };
    let (xs, ys) = (cells(x), cells(y));
    assert!(thumbs[&x].number && thumbs[&y].number && !thumbs[&color].number);
    assert_eq!(xs.len(), THUMB_CELLS);
    let aspect = THUMB_W as f32 / THUMB_H as f32;
    for row in 0..THUMB_H {
        for col in 0..THUMB_W {
            let i = row * THUMB_W + col;
            let want_x = ((col as f32 + 0.5) / THUMB_W as f32 * 2.0 - 1.0) * aspect;
            let want_y = (row as f32 + 0.5) / THUMB_H as f32 * 2.0 - 1.0;
            let (got_x, got_y) = (f32::from_bits(xs[i]), f32::from_bits(ys[i]));
            assert!(
                (got_x - want_x).abs() < 1e-5 && (got_y - want_y).abs() < 1e-5,
                "cell ({col}, {row}): ({got_x}, {got_y}), not ({want_x}, {want_y})"
            );
        }
    }
    let distinct: std::collections::BTreeSet<u32> = cells(color).into_iter().collect();
    assert!(
        distinct.len() >= 2,
        "a checkerboard is more than one color: {distinct:?}"
    );
    assert!(!thumbs.contains_key(&PortRef::new(out, "frame")));

    let stats = supersilvia::nodes::tap::decode(app.readback(tap).expect("measured"));
    assert_eq!(
        stats.count,
        128 * 128,
        "every point of the default grid, once"
    );
    assert!(
        stats.min < 0.01 && stats.max > 0.99 && (0.3..0.7).contains(&stats.mean),
        "a black and white checkerboard: {stats:?}"
    );
    // The node reads a reading on the tick after it lands.
    app.tick(1.0 / 60.0);
    assert!(
        app.cpu_report(tap).contains("measuring") && !app.cpu_report(tap).contains("not"),
        "and it says so: {}",
        app.cpu_report(tap)
    );
}

/// **A crowded workspace draws in batches, and every thumbnail arrives.** Twenty cellular
/// automata bind more textures than one pass may; the workspace's pass is split, each batch a
/// renderer of its own, and every port's thumbnail and the tap's reading come back.
#[test]
fn a_crowded_workspaces_batches_all_draw() {
    use supersilvia::graph::PortRef;
    use supersilvia::{App, Command};

    let mut app = App::headless();
    app.attach_gpu_on(gpu::gpu());
    let ws = app.graph().default_workspace();
    let mut add = |slug| {
        app.apply(Command::AddNode {
            slug,
            at: emath::Pos2::ZERO,
            workspace: ws,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    let cells: Vec<_> = (0..20).map(|_| add("cellularautomata")).collect();
    let tap = add("tap");
    app.apply(Command::Connect {
        from: PortRef::new(cells[0], "output"),
        to: PortRef::new(tap, "input"),
    })
    .unwrap();
    app.publish_plan();
    let batches = app.build_frame_job().passes.len();
    assert!(batches > 1, "{batches} batches");

    let ports: Vec<PortRef> = cells.iter().map(|c| PortRef::new(*c, "output")).collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        app.publish_plan();
        app.tick(1.0 / 60.0);
        let snap = app.snapshot();
        if ports.iter().all(|p| snap.thumbs.contains_key(p)) && app.readback(tap).is_some() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{} of {} thumbnails, reading {:?}",
            ports.iter().filter(|p| snap.thumbs.contains_key(p)).count(),
            ports.len(),
            app.readback(tap).is_some()
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
