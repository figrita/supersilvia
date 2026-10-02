// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the editor's own painting costs on the GPU, on one tab of a project, and what in it
//! costs what.
//!
//! ```sh
//! cargo run --release --example editor_bench -- <project folder> <workspace name> [WxH] [paints]
//! ```
//!
//! Headless, on the integrated GPU, `render::adapter::Asked::integrated` — never a discrete GPU,
//! never a software adapter, unless `SUPERSILVIA_ADAPTER` names one — and on a copy of the project. The real `App` is run by hand the way
//! eframe runs it, with a synth inline on the same device, and its frame is painted by
//! egui_wgpu's own renderer into a target of the window's size, 3440x1440 unless asked
//! otherwise, cleared to the ground as eframe clears the window
//! (`SUPERSILVIA_BENCH_CLEARED=0` paints every ground instead). Every
//! workspace is opened and the named one is looked at; nodes that would open a device are
//! removed first. `SUPERSILVIA_PREFERENCES=<file>` wears a person's preferences — their panels,
//! their Status box, their theme — from a copy of that file.
//!
//! **Timed by the render engine's own counter for this process**, the `drm-engine-render` line
//! of its DRM clients' fdinfo, around a frame painted `paints` times back to back and waited
//! for. Not a timestamp: a timestamp is the GPU's clock, which runs on through whatever other
//! process's work the GPU switches to — the live app's, on a box where it is open.
//!
//! The attribution paints the same frame with one kind of shape left out at a time, and one
//! kind's cost is what leaving it out saves: the rounds interleave every variant, so a GPU
//! changing its clock moves them all alike, and each figure is a median. Last, the Status box's
//! own timestamps are read over frames painted once each, beside the counter's figure for them.
//!
//! There is no variant with the pictures shrunk: a picture's paint callback places its blit by
//! the rect it was built for, which leaving the shape's rect smaller does not change.

use eframe::App as _;
use eframe::egui::{self, Shape};
use supersilvia::platform::gpu::render_engine_ns;
use supersilvia::project::Active;
use supersilvia::render::Gpu;
use supersilvia::{App, Command};

/// What a variant leaves out of the frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drop {
    Nothing,
    /// Every paint callback: the pictures on nodes, the mixer's decks and preview.
    Pictures,
    /// Blurred rects clipped to less than the screen: the shadow under each node.
    NodeShadows,
    /// Blurred rects clipped to the screen: the shadows windows and popups cast.
    WindowShadows,
    /// Circles of radius one: the canvas's dot grid.
    Grid,
    /// Cubic Béziers: the data cables.
    Cables,
    Text,
    /// Unblurred rects covering a quarter of the screen or more: panel and canvas grounds.
    BigFills,
    /// The Status box, closed for real rather than filtered.
    StatusBox,
    /// Every shape but the pictures, which is the painting of the chrome alone.
    AllButPictures,
    /// Unblurred rects in the panel color, larger than a node: the panels' own grounds and
    /// the canvas's, which the clear could paint.
    PanelFills,
}

const VARIANTS: [Drop; 11] = [
    Drop::Nothing,
    Drop::Pictures,
    Drop::NodeShadows,
    Drop::WindowShadows,
    Drop::Grid,
    Drop::Cables,
    Drop::Text,
    Drop::BigFills,
    Drop::StatusBox,
    Drop::AllButPictures,
    Drop::PanelFills,
];

fn main() {
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().expect("a project folder"));
    let tab = args.next().expect("a workspace name");
    let size: [u32; 2] = args.next().map_or([3440, 1440], |a| {
        let (w, h) = a.split_once('x').expect("WxH");
        [w.parse().expect("a width"), h.parse().expect("a height")]
    });
    let frames: usize = args
        .next()
        .map_or(20, |a| a.parse().expect("paints a round"));

    let gpu = Gpu::headless(&supersilvia::render::adapter::Asked::integrated())
        .unwrap_or_else(|e| panic!("{e}"));
    println!(
        "adapter: {}",
        supersilvia::render::adapter::describe(&gpu.adapter().get_info())
    );

    let scratch = std::env::temp_dir().join(format!("editor-bench-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("a scratch folder");
    let copy = scratch.join("project");
    let copied = std::process::Command::new("cp")
        .arg("-a")
        .arg(&root)
        .arg(&copy)
        .status()
        .expect("cp");
    assert!(copied.success(), "the project copied");

    let mut app = App::headless();
    let ctx = egui::Context::default();
    let theme = match std::env::var_os(supersilvia::preferences::PATH_ENV) {
        Some(prefs) => {
            let theirs = scratch.join("preferences.json");
            std::fs::copy(prefs, &theirs).expect("the preferences copied");
            let store = supersilvia::preferences::Store::load(Some(theirs));
            let theme = store.get().theme;
            app.use_preferences(store);
            theme
        }
        None => supersilvia::ui::theme::Theme::default(),
    };
    // Painted as eframe paints it: cleared to the ground, with no ground of its own.
    // `SUPERSILVIA_BENCH_CLEARED=0` paints every ground as a window cleared to anything else.
    let cleared = std::env::var("SUPERSILVIA_BENCH_CLEARED").map_or(true, |v| v != "0");
    if cleared {
        supersilvia::ui::theme::apply_cleared(&ctx, &theme);
    } else {
        supersilvia::ui::theme::apply(&ctx, &theme);
    }
    app.set_cleared(cleared);
    let clear = supersilvia::ui::theme::clear_color(&theme);
    app.attach_gpu_on(gpu.clone());
    app.attach_viewer_on(&gpu, FORMAT);
    app.open_project(copy);
    let live: Vec<_> = app
        .graph()
        .iter()
        .filter(|(_, n)| n.def.cpu.as_ref().is_some_and(|c| c.live))
        .map(|(id, _)| id)
        .collect();
    if !live.is_empty() {
        app.apply(Command::RemoveNodes(live)).expect("removable");
    }
    let workspaces: Vec<_> = app
        .graph()
        .workspaces()
        .iter()
        .map(|w| (w.id, w.name.clone()))
        .collect();
    for (id, _) in &workspaces {
        app.open_workspace(*id);
    }
    let (looked, _) = workspaces
        .iter()
        .find(|(_, name)| *name == tab)
        .unwrap_or_else(|| panic!("no workspace named {tab:?}"));
    app.activate(Active::Workspace(*looked));

    let mut painter = Painter::new(&gpu, size);

    let mut bench = Bench {
        app,
        ctx,
        frame: eframe::Frame::_new_kittest(),
        time: 0.0,
        size,
        clear,
        ground: theme.bg_primary(),
    };
    // Every program linked and every picture published, and the GPU clocked up, first.
    let started = std::time::Instant::now();
    loop {
        let shapes = bench.run(Drop::Nothing);
        painter.paint(&bench, shapes, 1);
        if bench.app.snapshot().render.linking.is_empty() && started.elapsed().as_secs() >= 5 {
            break;
        }
        assert!(started.elapsed().as_secs() < 120, "never linked");
    }

    let stats = census(&bench.run(Drop::Nothing).0, bench.content(), bench.ground);
    println!(
        "{tab} at {}x{}: {} shapes, {} callbacks ({} px of pictures), {} grid dots, {} blurred \
         rects, {} cables, {} texts",
        size[0],
        size[1],
        stats.shapes,
        stats.callbacks,
        stats.callback_px,
        stats.dots,
        stats.blurred,
        stats.cables,
        stats.texts,
    );
    let screen = bench.content().area();
    println!(
        "  rect fills cover {:.2} screens, {:.2} of them in the panel color: {:?}",
        stats.filled / screen,
        stats.panel_filled / screen,
        stats.panel_rects
    );
    for (w, h, n) in &stats.callback_rects {
        println!("  picture slot {w}x{h} x{n}");
    }

    let rounds = 5;
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); VARIANTS.len()];
    let mut prims = [0usize; VARIANTS.len()];
    for _ in 0..rounds {
        for (i, drop) in VARIANTS.iter().enumerate() {
            let shapes = bench.run(*drop);
            let (ms, n) = painter.paint(&bench, shapes, frames);
            times[i].push(ms);
            prims[i] = n;
        }
    }
    let median = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let base = median(&mut times[0]);
    println!("| left out | primitives | GPU ms a frame | saves |");
    println!("| --- | ---: | ---: | ---: |");
    for (i, drop) in VARIANTS.iter().enumerate() {
        let ms = median(&mut times[i]);
        println!("| {drop:?} | {} | {ms:.3} | {:.3} |", prims[i], base - ms);
    }
    let cleared_ms = painter.clears(bench.clear, 100);
    println!("a clear alone: {cleared_ms:.3} ms");
    // The Status box's own reading, over frames painted once each as eframe paints them. Its
    // timestamps count whatever else the GPU ran between them, so beside another process's
    // work it reads high; the counter's figure is this process's alone.
    let mut once: Vec<f64> = (0..120)
        .map(|_| {
            let shapes = bench.run(Drop::Nothing);
            painter.paint(&bench, shapes, 1).0
        })
        .collect();
    let counted = median(&mut once);
    match bench.app.paint_gpu() {
        Some((now, avg, worst)) => println!(
            "painted once a frame: {counted:.3} ms by the counter; the Status box reads {now:.2} \
             ms now, {avg:.2} avg, {worst:.2} worst"
        ),
        None => println!("painted once a frame: {counted:.3} ms; the Status box read nothing"),
    }

    let _ = std::fs::remove_dir_all(&scratch);
}

struct Bench {
    app: App,
    ctx: egui::Context,
    frame: eframe::Frame,
    time: f64,
    size: [u32; 2],
    /// The clear before each paint, as eframe's.
    clear: [f32; 4],
    /// The panels' and the canvas's ground color.
    ground: egui::Color32,
}

impl Bench {
    fn content(&self) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(self.size[0] as f32, self.size[1] as f32),
        )
    }

    /// One editor frame at 100 Hz, and its shapes with `drop` left out.
    fn run(&mut self, drop: Drop) -> (Vec<egui::epaint::ClippedShape>, egui::TexturesDelta) {
        self.app.set_show_status_box(drop != Drop::StatusBox);
        self.time += 0.01;
        let mut raw = egui::RawInput {
            screen_rect: Some(self.content()),
            time: Some(self.time),
            focused: true,
            ..Default::default()
        };
        let viewport = raw.viewports.entry(egui::ViewportId::ROOT).or_default();
        viewport.native_pixels_per_point = Some(1.0);
        viewport.focused = Some(true);
        viewport.inner_rect = Some(self.content());
        let (app, frame) = (&mut self.app, &mut self.frame);
        let out = self.ctx.run_ui(raw, |ui| app.ui(ui, frame));
        let content = self.content();
        let panel = self.ground;
        let shapes = out
            .shapes
            .into_iter()
            .filter_map(|mut c| {
                c.shape = keep(c.shape, drop, c.clip_rect, content, panel)?;
                Some(c)
            })
            .collect();
        (shapes, out.textures_delta)
    }
}

/// The shape with `drop`'s kind taken out of it, or `None` where nothing is left.
fn keep(
    shape: Shape,
    drop: Drop,
    clip: egui::Rect,
    content: egui::Rect,
    panel: egui::Color32,
) -> Option<Shape> {
    if let Shape::Vec(shapes) = shape {
        let kept: Vec<Shape> = shapes
            .into_iter()
            .filter_map(|s| keep(s, drop, clip, content, panel))
            .collect();
        return (!kept.is_empty()).then_some(Shape::Vec(kept));
    }
    let screen = clip.contains_rect(content);
    let out = match (&shape, drop) {
        // The two full-screen callbacks are the Status box's own paint timer, not pictures.
        (Shape::Callback(c), Drop::Pictures) => c.rect != content,
        (Shape::Callback(c), Drop::AllButPictures) => c.rect == content,
        (_, Drop::AllButPictures)
        | (Shape::CubicBezier(_), Drop::Cables)
        | (Shape::Text(_), Drop::Text) => true,
        (Shape::Rect(r), Drop::NodeShadows) => r.blur_width > 0.0 && !screen,
        // A node's shadow with its covered core cut out, which is a mesh in the shadow's
        // color fading to nothing.
        (Shape::Mesh(m), Drop::NodeShadows) => m.vertices.iter().all(|v| {
            v.color == egui::Color32::TRANSPARENT
                || v.color == egui::Visuals::dark().window_shadow.color
        }),
        (Shape::Rect(r), Drop::WindowShadows) => r.blur_width > 0.0 && screen,
        (Shape::Rect(r), Drop::BigFills) => {
            r.blur_width == 0.0 && r.rect.area() >= content.area() / 4.0
        }
        (Shape::Rect(r), Drop::PanelFills) => {
            r.blur_width == 0.0 && r.fill == panel && r.rect.area() > 120_000.0
        }
        (Shape::Circle(c), Drop::Grid) => c.radius == 1.0,
        _ => false,
    };
    (!out).then_some(shape)
}

/// The format the editor's window is painted in, as egui_wgpu picks it on this box.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// egui_wgpu's renderer and a target of the window's size, on the bench's device.
struct Painter {
    gpu: Gpu,
    renderer: egui_wgpu::Renderer,
    view: wgpu::TextureView,
    size: [u32; 2],
}

impl Painter {
    fn new(gpu: &Gpu, size: [u32; 2]) -> Self {
        let target = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("editor"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            gpu: gpu.clone(),
            renderer: egui_wgpu::Renderer::new(
                gpu.device(),
                FORMAT,
                egui_wgpu::RendererOptions::default(),
            ),
            view: target.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        }
    }

    /// Wait for everything submitted so far.
    fn finish(&self) {
        let _ = self.gpu.device().poll(wgpu::PollType::wait_indefinitely());
    }

    /// Paint one frame's shapes `times` times over, back to back, and return the render
    /// engine time this process spent on each, in milliseconds, and how many primitives egui
    /// handed the painter.
    ///
    /// **The engine's own count, not a timer query.** A timestamp is the GPU's clock, and the
    /// clock runs on through whatever other process's work the GPU switches to — the live
    /// app's, on a box where it is open. The kernel's per-client counter is this process's time
    /// on the engine and nothing else's. Each paint is a clear and the frame, as eframe's is.
    fn paint(
        &mut self,
        bench: &Bench,
        (shapes, mut textures): (Vec<egui::epaint::ClippedShape>, egui::TexturesDelta),
        times: usize,
    ) -> (f64, usize) {
        let primitives = bench.ctx.tessellate(shapes, 1.0);
        let device = self.gpu.device();
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.renderer
                    .update_texture(device, self.gpu.queue(), *id, delta);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: 1.0,
        };
        // The synth's draw inside the frame is finished before the painting is timed.
        self.finish();
        let before = render_engine_ns();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("editor"),
        });
        let prepared = self.renderer.update_buffers(
            device,
            self.gpu.queue(),
            &mut encoder,
            &primitives,
            &screen,
        );
        let [r, g, b, a] = bench.clear.map(f64::from);
        for _ in 0..times {
            let mut pass = supersilvia::render::shared::begin(
                &mut encoder,
                &self.view,
                wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                "editor",
            )
            .forget_lifetime();
            self.renderer.render(&mut pass, &primitives, &screen);
        }
        self.gpu
            .submit(prepared.into_iter().chain([encoder.finish()]));
        self.finish();
        let spent = render_engine_ns() - before;
        for id in &textures.free {
            self.renderer.free_texture(id);
        }
        textures.clear();
        (spent as f64 / 1e6 / times as f64, primitives.len())
    }

    /// The engine time of a clear of the whole target alone, in milliseconds, over `times`.
    fn clears(&self, clear: [f32; 4], times: usize) -> f64 {
        self.finish();
        let before = render_engine_ns();
        let [r, g, b, a] = clear.map(f64::from);
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("clear"),
                });
        for _ in 0..times {
            supersilvia::render::shared::clear(
                &mut encoder,
                &self.view,
                wgpu::Color { r, g, b, a },
            );
        }
        self.gpu.submit([encoder.finish()]);
        self.finish();
        (render_engine_ns() - before) as f64 / 1e6 / times as f64
    }
}

#[derive(Default)]
struct Census {
    shapes: usize,
    callbacks: usize,
    callback_px: f32,
    callback_rects: Vec<(u32, u32, usize)>,
    dots: usize,
    blurred: usize,
    cables: usize,
    texts: usize,
    /// The area of every unblurred rect fill, in pixels, and of those in the panel color.
    filled: f32,
    panel_filled: f32,
    panel_rects: Vec<(f32, f32)>,
    panel: egui::Color32,
}

fn census(
    shapes: &[egui::epaint::ClippedShape],
    content: egui::Rect,
    panel: egui::Color32,
) -> Census {
    fn walk(s: &Shape, c: &mut Census, content: egui::Rect) {
        c.shapes += 1;
        match s {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, c, content)),
            Shape::Callback(cb) if cb.rect != content => {
                c.callbacks += 1;
                c.callback_px += cb.rect.area();
                let key = (cb.rect.width() as u32, cb.rect.height() as u32);
                match c.callback_rects.iter_mut().find(|r| (r.0, r.1) == key) {
                    Some(r) => r.2 += 1,
                    None => c.callback_rects.push((key.0, key.1, 1)),
                }
            }
            Shape::Circle(ci) if ci.radius == 1.0 => c.dots += 1,
            Shape::Rect(r) if r.blur_width > 0.0 => c.blurred += 1,
            Shape::Rect(r) if r.fill.a() > 0 => {
                let area = r.rect.intersect(content).area().max(0.0);
                c.filled += area;
                if r.fill == c.panel {
                    c.panel_filled += area;
                    if area > 120_000.0 {
                        c.panel_rects.push((r.rect.width(), r.rect.height()));
                    }
                }
            }
            Shape::CubicBezier(_) => c.cables += 1,
            Shape::Text(_) => c.texts += 1,
            _ => {}
        }
    }
    let mut c = Census {
        panel,
        ..Census::default()
    };
    for s in shapes {
        walk(&s.shape, &mut c, content);
    }
    c
}
