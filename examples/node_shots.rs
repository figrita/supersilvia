// SPDX-License-Identifier: AGPL-3.0-or-later

//! Headless screenshot of every node in the library, one PNG each, with no window.
//!
//! ```sh
//! cargo run --release --example node_shots -- shots/           # every node
//! cargo run --release --example node_shots -- shots/ blur mix   # just these
//! PPP=3 cargo run --release --example node_shots -- shots/      # sharper
//! HEADINGS=closed cargo run --release --example node_shots -- shots/   # every region shut
//! ```
//!
//! `HEADINGS=closed` turns every region heading the node declares off before the shot, which
//! is the other half of what a heading looks like and is otherwise only reachable by clicking.
//!
//! Each node is added to a fresh `App` through the command bus, drawn by egui_kittest for a
//! few frames, cropped to its `<name> body` rect and written to `OUT/<slug>.png`. A line per
//! node goes to `OUT/index.jsonl` with the node's accessibility subtree — every port, control
//! and select with its label and value — which is the same tree the egui MCP reads.
//!
//! The harness's app has no renderer, so no picture is blitted: an Output's preview is
//! blank and a device node (`camera`, `audioin`, `video`) shows what it shows with nothing
//! open. The node chrome, rows, controls and ports are exactly what the window draws.
//!
//! kittest draws on one device for the run, on the integrated GPU,
//! `render::adapter::Asked::integrated`, as the snapshot tests do: kittest's own selector
//! prefers a software adapter and then a discrete one.

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use serde_json::{Value, json};
use std::sync::OnceLock;
use supersilvia::nodes::REGISTRY;
use supersilvia::render::adapter;
use supersilvia::{App, Command};

/// Points of canvas around the body kept in the crop.
const PAD: f32 = 12.0;

fn main() {
    use std::io::Write as _;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first() else {
        eprintln!("usage: node_shots OUT_DIR [slug ...]");
        std::process::exit(2);
    };
    std::fs::create_dir_all(out).expect("create OUT_DIR");
    let slugs: Vec<&str> = if args.len() > 1 {
        args[1..].iter().map(String::as_str).collect()
    } else {
        REGISTRY.iter().map(|d| d.slug).collect()
    };
    let ppp: f32 = std::env::var("PPP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0);
    let mut index = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{out}/index.jsonl"))
        .expect("open index.jsonl");

    for slug in slugs {
        let line = match shoot(slug, out, ppp) {
            Ok(line) => {
                eprintln!("{slug} ok");
                line
            }
            Err(e) => {
                eprintln!("{slug} ERROR {e}");
                json!({ "slug": slug, "error": e })
            }
        };
        writeln!(index, "{line}").unwrap();
    }
}

/// kittest's renderer over the run's one device, on the integrated GPU.
fn renderer() -> egui_kittest::wgpu::WgpuTestRenderer {
    static GPU: OnceLock<egui_wgpu::WgpuSetupExisting> = OnceLock::new();
    let existing = GPU.get_or_init(|| {
        let mut setup = egui_kittest::wgpu::default_wgpu_setup();
        if let egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut setup {
            create_new.instance_descriptor.backends = adapter::BACKENDS;
            create_new.native_adapter_selector = Some(std::sync::Arc::new(
                |adapters: &[egui_wgpu::wgpu::Adapter], _surface| {
                    adapter::pick(adapters, &adapter::Asked::integrated())
                },
            ));
        }
        let state =
            egui_kittest::wgpu::create_render_state(setup, egui_wgpu::RendererOptions::PREDICTABLE);
        egui_wgpu::WgpuSetupExisting {
            instance: state.instance.clone(),
            adapter: state.adapter.clone(),
            device: state.device.clone(),
            queue: state.queue.clone(),
        }
    });
    egui_kittest::wgpu::WgpuTestRenderer::from_setup(egui_wgpu::WgpuSetup::Existing(
        existing.clone(),
    ))
}

fn shoot(slug: &str, out: &str, ppp: f32) -> Result<Value, String> {
    // The registry's own `&'static str`, which is what `Command::AddNode` takes.
    let def =
        supersilvia::nodes::find(slug).ok_or_else(|| format!("no node with slug {slug:?}"))?;
    let slug = def.slug;
    // A tall window: the whole node has to be on screen to be cropped out of one frame.
    let mut h = Harness::builder()
        .renderer(renderer())
        .with_size(egui::vec2(1200.0, 2000.0))
        .with_pixels_per_point(ppp)
        .build_eframe(|cc| App::new(cc, supersilvia::preferences::Store::in_memory()));
    h.step();
    let workspace = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddNode {
            slug,
            at: emath::pos2(60.0, 40.0),
            workspace,
        })
        .map_err(|e| format!("{e:?}"))?;
    let id = h
        .state()
        .graph()
        .iter()
        .map(|(id, _)| id.0)
        .max()
        .ok_or("the node was not added")?;
    let name = format!("{slug}{id}");
    if std::env::var("HEADINGS").as_deref() == Ok("closed") {
        let keys: Vec<&'static str> = def
            .regions
            .iter()
            .filter_map(|r| r.heading().map(|h| h.key))
            .collect();
        for key in keys {
            h.state_mut()
                .apply(Command::SetOption {
                    node: supersilvia::graph::NodeId(id),
                    key,
                    value: supersilvia::nodes::OFF.to_string(),
                })
                .map_err(|e| format!("{e:?}"))?;
        }
    }
    // A few frames so the sizing pass, the fonts and any CPU node's first tick have run.
    h.run_steps(8);

    let body = h
        .query_by_label(format!("{name} body").as_str())
        .ok_or_else(|| format!("no `{name} body` in the tree"))?
        .rect();
    // The rows hang ports off both sides of the body and a cost strip under it.
    let crop = body.expand2(egui::vec2(24.0, 0.0)).expand(PAD);

    let img = h.render().map_err(|e| format!("render: {e}"))?;
    let px = |v: f32| (v * ppp).round().max(0.0) as u32;
    let x = px(crop.min.x).min(img.width());
    let y = px(crop.min.y).min(img.height());
    let w = px(crop.width()).min(img.width() - x);
    let hh = px(crop.height()).min(img.height() - y);
    let cut_off = px(crop.max.y) > img.height();
    let path = format!("{out}/{slug}.png");
    image::imageops::crop_imm(&img, x, y, w, hh)
        .to_image()
        .save(&path)
        .map_err(|e| format!("save: {e}"))?;

    // The node's accessibility subtree: everything whose rect lies inside the crop.
    let mut tree = Vec::new();
    for n in h.root().children_recursive() {
        let ak = n.accesskit_node();
        // Not every node has bounds (the root, a hidden popup), and `NodeT::rect` panics on one.
        let Some(bb) = ak.bounding_box() else {
            continue;
        };
        // AccessKit holds physical pixels; the crop is in points.
        let r = egui::Rect::from_min_max(
            egui::pos2(bb.x0 as f32 / ppp, bb.y0 as f32 / ppp),
            egui::pos2(bb.x1 as f32 / ppp, bb.y1 as f32 / ppp),
        );
        if r.width() <= 0.0 || !crop.contains(r.min) {
            continue;
        }
        tree.push(json!({
            "role": format!("{:?}", ak.role()),
            "label": ak.label(),
            "value": ak.value(),
            "bounds": bounds(r),
        }));
    }
    Ok(json!({
        "slug": slug,
        "name": name,
        "png": path,
        "ppp": ppp,
        "body": bounds(body),
        "cut_off": cut_off,
        "tooltip": def.tooltip,
        "tree": tree,
    }))
}

/// A rect as `[x, y, width, height]`, rounded to whole points.
fn bounds(r: egui::Rect) -> [i64; 4] {
    let round = |v: f32| v.round() as i64;
    [
        round(r.min.x),
        round(r.min.y),
        round(r.width()),
        round(r.height()),
    ]
}
