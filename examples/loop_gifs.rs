// SPDX-License-Identifier: AGPL-3.0-or-later

//! One loop of every workspace's Output as a small animated GIF, headless, with the seam
//! measured beside the file: the Output's ordinary render, run on each tab of a project in
//! turn for as long as its Master Gear says a loop is.
//!
//! ```sh
//! cargo run --release --example loop_gifs -- <project folder> [--only name,name] [--out dir]
//!     [--width 480] [--square 400] [--colors 64] [--warm seconds] [--length seconds]
//!     [--pre-roll loops] [--keep]
//! ```
//!
//! On the integrated GPU, `render::adapter::Asked::integrated`, never a software adapter.
//! The project is opened through the `App`, as the editor opens it, and any load warning is
//! printed and fails the run. Each workspace is then the one open tab, as a person would look
//! at it, and its Output is rendered through `App::render_settings_of` and
//! `App::start_render`. **The length is the Master Gear's**: of the Master Gears upstream of the
//! Output, the only one, or else the first by id, for as many of its cycles as everything
//! downstream of it needs to come back whole (`nodes::chain::master_length`); a workspace with
//! none, or whose loop never closes or cannot be told, takes `--length` seconds, or else the
//! Output's own Duration. The
//! render is that length in whole frames at the Output's FPS, `F`, after `--pre-roll` loops of
//! warm-up at negative time, three by default — an echo dimmed by 0.94 a frame leaves 8-bit
//! crumbs that take that long to settle onto the loop — and one frame more: frame `F` is compared with frame zero for the seam, and only frames
//! `0..F` go into the GIF. It writes a PNG sequence at the Output's own resolution into a
//! scratch folder, and each frame is composited over black — the PNGs are straight and the GIF
//! is opaque, so a transparent part kept at its own color would show at full strength — then
//! brought down to a size a page can carry
//! — 480 wide, or 400 for a square or upright Output — mapped onto one palette the loop
//! shares, and written through `video::gif`, the app's own GIF writer, as
//! `renders/<workspace>.gif`. The Output's resolution is a closed list whose smallest is
//! 1024x768, so the small size is this tool's rather than the export's. `--out` copies each GIF
//! into a folder besides.
//!
//! Before the export the workspace plays live for `--warm` seconds, one by default, so its
//! programs have linked — longer lets a simulation grow before its loop is taken.
//!
//! A workspace whose Output reaches a node with an asset option naming `assets/<x>.gif` that is
//! not on disk, where this run has just written `renders/<x>.gif`, has it copied in first, the
//! way an import stores a file: that is how a project can play back a loop it exported.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use supersilvia::App;
use supersilvia::app::render::{Format, Outcome};
use supersilvia::clock::Warmup;
use supersilvia::graph::NodeId;
use supersilvia::project::Project;
use supersilvia::render::Gpu;

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("a project folder"));
    let mut only: Option<Vec<String>> = None;
    let mut copy_to: Option<PathBuf> = None;
    let mut size = Size {
        width: 480,
        square: 400,
        colors: 64,
    };
    let mut warm = 1.0_f64;
    let mut length: Option<f64> = None;
    let mut keep = false;
    let mut pre_roll = 3_u32;
    let number = |args: &mut std::iter::Skip<std::env::Args>, what: &str| -> f64 {
        args.next()
            .and_then(|a| a.parse().ok())
            .unwrap_or_else(|| panic!("a number after {what}"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            // The whole argument is a name too, since a workspace's may hold a comma.
            "--only" => {
                let names = args.next().expect("names after --only");
                let mut list: Vec<String> = names.split(',').map(str::to_string).collect();
                list.push(names);
                only = Some(list);
            }
            "--out" => copy_to = Some(PathBuf::from(args.next().expect("a folder after --out"))),
            "--width" => size.width = number(&mut args, "--width") as u32,
            "--square" => size.square = number(&mut args, "--square") as u32,
            "--colors" => size.colors = number(&mut args, "--colors") as usize,
            "--warm" => warm = number(&mut args, "--warm"),
            "--length" => length = Some(number(&mut args, "--length")),
            "--pre-roll" => pre_roll = number(&mut args, "--pre-roll").max(0.0) as u32,
            "--keep" => keep = true,
            other => panic!("unknown argument {other:?}"),
        }
    }

    // The loader's own word on the files, before anything renders.
    let (project, graph, warnings) =
        Project::open(root.clone()).unwrap_or_else(|e| panic!("{}: {e}", root.display()));
    for w in &warnings {
        eprintln!("load warning: {w}");
    }
    assert!(warnings.is_empty(), "the project loads with warnings");
    println!(
        "{}: {} workspaces, {} nodes, {} where no Master Gear says",
        root.display(),
        graph.workspaces().len(),
        graph.iter().count(),
        length.map_or("the Output's Duration".to_string(), |s| format!("{s} s"))
    );
    let workspaces: Vec<_> = graph
        .workspaces()
        .iter()
        .map(|w| (w.id, w.name.clone()))
        .collect();
    drop((project, graph));

    let gpu = Gpu::headless(&supersilvia::render::adapter::Asked::integrated())
        .unwrap_or_else(|e| panic!("{e}"));
    println!(
        "adapter: {}",
        supersilvia::render::adapter::describe(&gpu.adapter().get_info())
    );
    let scratch =
        std::env::temp_dir().join(format!("supersilvia-loop-gifs-{}", std::process::id()));
    let renders = root.join("renders");
    std::fs::create_dir_all(&renders).expect("a renders folder");
    if let Some(dir) = &copy_to {
        std::fs::create_dir_all(dir).expect("the --out folder");
    }

    for (index, (ws, name)) in workspaces.iter().enumerate() {
        if only
            .as_ref()
            .is_some_and(|names| !names.iter().any(|n| n == name))
        {
            continue;
        }
        fill_assets(&root, *ws);
        let mut app = App::headless();
        app.attach_gpu_on(gpu.for_synth());
        app.open_project(root.clone());
        let open: Vec<_> = app.open_workspaces().iter().copied().collect();
        for id in open {
            app.close_workspace(id);
        }
        app.open_workspace(*ws);
        let outputs: Vec<NodeId> = app
            .graph()
            .iter()
            .filter(|(_, n)| n.def.is_output && n.workspaces.contains(ws))
            .map(|(id, _)| id)
            .collect();
        let [output] = outputs[..] else {
            println!("{name}: {} Outputs, skipped", outputs.len());
            continue;
        };
        // Live for a while, so every program has linked and the Output draws.
        app.publish_plan();
        for _ in 0..(warm * 60.0).ceil() as u32 {
            app.tick(1.0 / 60.0);
            std::thread::sleep(Duration::from_millis(15));
        }
        let mut settings = app.render_settings_of(output).expect("an Output");
        let fps = settings.fps;
        let (seconds, from) = match (loop_length(&app, output), length) {
            (Some((master, seconds)), _) => (seconds, format!("mastergear{}", master.0)),
            (None, Some(seconds)) => (seconds, "typed".to_string()),
            (None, None) => (f64::from(settings.frames) / fps, "the Output".to_string()),
        };
        let frames = (seconds * fps).round().max(1.0) as u32;
        let frames_dir = scratch.join(format!("{index:02}"));
        let _ = std::fs::remove_dir_all(&frames_dir);
        settings.format = Format::PngSequence;
        settings.frames = frames + 1;
        settings.warmup = Warmup::Run(frames * pre_roll);
        settings.destination.clone_from(&frames_dir);
        let started = Instant::now();
        app.start_render(output, &settings)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        while app.rendering() {
            assert!(
                started.elapsed() < Duration::from_secs(600),
                "{name}: the render never finished"
            );
            app.publish_plan();
            app.tick(1.0 / 60.0);
        }
        let outcome = app.render_outcome().cloned().expect("an outcome");
        let Outcome::Done { .. } = outcome else {
            panic!("{name}: {outcome:?}");
        };
        drop(app);
        let seam = Seam::between(&frames_dir, frames);

        let file = renders.join(format!("{}.gif", file_stem(name)));
        let (w, h) = write_gif(&frames_dir, frames, fps, &file, size);
        if keep {
            println!("    frames kept in {}", frames_dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&frames_dir);
        }
        let bytes = std::fs::metadata(&file).map_or(0, |m| m.len());
        println!(
            "{name}: {seconds} s from {from}, {frames} frames at {fps} fps, {w}x{h}, {:.2} MB, \
             {:.1} s to render — {}",
            bytes as f64 / 1e6,
            started.elapsed().as_secs_f64(),
            seam.describe()
        );
        println!("    {}", file.display());
        if let Some(dir) = &copy_to {
            std::fs::copy(&file, dir.join(file.file_name().expect("a name")))
                .expect("a copy in --out");
        }
    }
    if !keep {
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

/// How long a loop of `output` is, from a Master Gear upstream of it: the only one, or else the
/// first by id, for as many cycles as its chains need. `None` where there is none, or where its
/// chains never close.
fn loop_length(app: &App, output: NodeId) -> Option<(NodeId, f64)> {
    let graph = app.graph();
    let mut upstream = std::collections::BTreeSet::new();
    let mut stack = vec![output];
    while let Some(id) = stack.pop() {
        if upstream.insert(id) {
            stack.extend(graph.cables_into(id).iter().map(|c| c.from.node));
        }
    }
    let master = upstream
        .into_iter()
        .find(|id| graph.get(*id).is_some_and(|n| n.def.slug == "mastergear"))?;
    Some((
        master,
        supersilvia::nodes::chain::master_length(graph, master)?,
    ))
}

/// How far frame `F` of a loop is from frame zero, which it is in a loop that closes.
struct Seam {
    /// Pixels where any channel differs at all, out of `pixels`.
    differing: u64,
    pixels: u64,
    /// The largest difference in any channel, 0 to 255.
    max: u8,
}

impl Seam {
    /// Frames zero and `frames` of the PNG sequence in `dir`, compared.
    fn between(dir: &Path, frames: u32) -> Self {
        let read = |i: u32| {
            let path = dir.join(format!("{i:05}.png"));
            image::open(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .to_rgba8()
                .into_raw()
        };
        let (first, last) = (read(0), read(frames));
        let mut seam = Self {
            differing: 0,
            pixels: (first.len() / 4) as u64,
            max: 0,
        };
        for (a, b) in first.as_chunks::<4>().0.iter().zip(last.as_chunks::<4>().0) {
            let d = a
                .iter()
                .zip(b)
                .map(|(x, y)| x.abs_diff(*y))
                .max()
                .unwrap_or(0);
            if d > 0 {
                seam.differing += 1;
                seam.max = seam.max.max(d);
            }
        }
        seam
    }

    fn describe(&self) -> String {
        if self.differing == 0 {
            "seam 0: the frame after the last is the first to the byte".to_string()
        } else {
            format!(
                "seam {:.2} % of pixels, by at most {} of 255",
                self.differing as f64 * 100.0 / self.pixels.max(1) as f64,
                self.max
            )
        }
    }
}

/// A workspace's name as a file name: lower case, words joined by dashes.
fn file_stem(name: &str) -> String {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Frames `0..frames` of a PNG sequence, over black, brought down to `width` (`square` for a
/// square or upright picture), mapped onto one palette of `colors` shared by the whole loop
/// and written as a GIF at `fps`. The size written.
///
/// One palette for every frame, rather than the writer's own palette per frame: a colour does
/// not shimmer from one frame to the next, and a frame of at most 256 colours goes through the
/// writer as it is, which with fewer colours and no dithering is what keeps a loop of noise
/// small enough to post.
fn write_gif(dir: &Path, frames: u32, fps: f64, file: &Path, size: Size) -> (u32, u32) {
    let small: Vec<image::RgbaImage> = (0..frames)
        .map(|i| {
            let path = dir.join(format!("{i:05}.png"));
            let mut image = image::open(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
                .to_rgba8();
            over_black(&mut image);
            let (w, h) = image.dimensions();
            let width = if w > h { size.width } else { size.square };
            let height = (u64::from(h) * u64::from(width) / u64::from(w)) as u32;
            image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle)
        })
        .collect();
    let (width, height) = small[0].dimensions();
    let palette = Palette::of(&small, size.colors);
    let mut writer = supersilvia::video::gif::Writer::start(file, width, height, fps)
        .unwrap_or_else(|e| panic!("{e}"));
    for frame in small {
        writer
            .push(&palette.map(frame.into_raw()))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    writer.finish().unwrap_or_else(|e| panic!("{e}"));
    (width, height)
}

/// A straight frame, as the PNG sequence is written, composited over black and made opaque,
/// as a viewer shows it.
fn over_black(image: &mut image::RgbaImage) {
    for p in image.pixels_mut() {
        let a = u16::from(p[3]);
        for c in 0..3 {
            p[c] = ((u16::from(p[c]) * a + 127) / 255) as u8;
        }
        p[3] = u8::MAX;
    }
}

/// How small a GIF is written.
#[derive(Clone, Copy)]
struct Size {
    /// The width of a landscape picture.
    width: u32,
    /// The width of a square or upright one.
    square: u32,
    /// The colours in the palette the whole loop shares, at most 256.
    colors: usize,
}

/// A palette the whole loop shares: k-means over a sample of its pixels, and every colour at
/// six bits a channel looked up once.
struct Palette {
    colors: Vec<[f32; 3]>,
    nearest: std::cell::RefCell<Vec<u8>>,
}

/// A colour at six bits a channel, as an index into [`Palette::nearest`].
fn key(r: u8, g: u8, b: u8) -> usize {
    (usize::from(r >> 2) << 12) | (usize::from(g >> 2) << 6) | usize::from(b >> 2)
}

impl Palette {
    fn of(frames: &[image::RgbaImage], k: usize) -> Self {
        // Every seventh pixel of every fourth frame.
        let mut sample: Vec<[f32; 3]> = Vec::new();
        for frame in frames.iter().step_by(4) {
            for p in frame.pixels().step_by(7) {
                sample.push([f32::from(p[0]), f32::from(p[1]), f32::from(p[2])]);
            }
        }
        // Seeded along the sample's order of brightness, so the start is spread and the same
        // every run.
        sample.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
        let k = k.clamp(2, 255).min(sample.len());
        let mut colors: Vec<[f32; 3]> = (0..k)
            .map(|i| sample[(2 * i + 1) * sample.len() / (2 * k)])
            .collect();
        let mut owner = vec![0usize; sample.len()];
        for _ in 0..12 {
            for (s, o) in sample.iter().zip(owner.iter_mut()) {
                *o = nearest(&colors, *s);
            }
            let mut sums = vec![[0.0f64; 4]; k];
            for (s, o) in sample.iter().zip(&owner) {
                for c in 0..3 {
                    sums[*o][c] += f64::from(s[c]);
                }
                sums[*o][3] += 1.0;
            }
            for (color, sum) in colors.iter_mut().zip(&sums) {
                if sum[3] > 0.0 {
                    *color = [0, 1, 2].map(|c| (sum[c] / sum[3]) as f32);
                }
            }
        }
        Self {
            colors,
            nearest: std::cell::RefCell::new(vec![u8::MAX; 1 << 18]),
        }
    }

    /// `rgba` with every pixel the palette's nearest colour, opaque.
    fn map(&self, mut rgba: Vec<u8>) -> Vec<u8> {
        let mut nearest = self.nearest.borrow_mut();
        for p in rgba.as_chunks_mut::<4>().0 {
            let at = key(p[0], p[1], p[2]);
            if nearest[at] == u8::MAX {
                let center = [p[0], p[1], p[2]].map(|v| f32::from((v & 0xfc) | 2));
                nearest[at] = nearest_index(&self.colors, center);
            }
            let c = self.colors[usize::from(nearest[at])];
            *p = [c[0] as u8, c[1] as u8, c[2] as u8, 255];
        }
        rgba
    }
}

fn nearest(colors: &[[f32; 3]], s: [f32; 3]) -> usize {
    usize::from(nearest_index(colors, s))
}

fn nearest_index(colors: &[[f32; 3]], s: [f32; 3]) -> u8 {
    let mut best = (f32::MAX, 0u8);
    for (i, c) in colors.iter().enumerate() {
        let d = (c[0] - s[0]).powi(2) + (c[1] - s[1]).powi(2) + (c[2] - s[2]).powi(2);
        if d < best.0 {
            best = (d, i as u8);
        }
    }
    best.1
}

/// Copy `renders/<x>.gif` to `assets/<x>.gif` for every asset option on `workspace` that names
/// the second and finds nothing there.
fn fill_assets(root: &Path, workspace: supersilvia::graph::WorkspaceId) {
    let Ok((_, graph, _)) = Project::open(root.to_path_buf()) else {
        return;
    };
    for (_, node) in graph
        .iter()
        .filter(|(_, n)| n.workspaces.contains(&workspace))
    {
        for value in node.options.values() {
            let Some(name) = value.strip_prefix("assets/") else {
                continue;
            };
            let asset = root.join("assets").join(name);
            let render = root.join("renders").join(name);
            if !asset.exists() && render.is_file() {
                std::fs::create_dir_all(root.join("assets")).expect("an assets folder");
                std::fs::copy(&render, &asset).expect("the render copied in");
                println!("copied renders/{name} into assets/");
            }
        }
    }
}
