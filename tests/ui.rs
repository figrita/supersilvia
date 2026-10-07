// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 2: UI behavior and appearance, headless.
//!
//! egui_kittest drives the app through the AccessKit tree — find a widget by label, click
//! it, run frames, assert on state — with no window and no GPU. This is why every
//! interactive thing has to be a real egui `Response`: a hand-painted rect the user can
//! click but the accessibility tree cannot see is a bug, and it is also what makes the
//! agent-driven layer (egui_mcp) pleasant, since both go through the same tree.
//!
//! These tests use `step()`, never `Harness::run()`. `run()` repaints until the UI settles,
//! and supersilvia's never does: `App::ui` calls `request_repaint` unconditionally because it
//! is a synth. That is permanent, so every test here drives a fixed number of frames.
//!
//! The snapshots cover the editor chrome only. kittest hands the app no device, so every picture
//! is a placeholder and the pictures themselves are layer 3's job (`tests/gpu_app.rs`): a real
//! picture in a snapshot would make the snapshot depend on the GPU and the clock.
//!
//! Every harness here shares one wgpu device, because kittest's own default does not.
//! `WgpuTestRenderer::new` builds an instance, an adapter and a device per `Harness`, and
//! libtest runs these tests on as many threads as the box has cores: two of them reaching
//! their first snapshot together put one thread inside `vkCreateInstance` while the other
//! walks the Vulkan loader's handle tables from `vkSetDebugUtilsObjectNameEXT`, reading the
//! tables the creation is writing. `loader_get_icd_and_device` faults and the process dies
//! with `SIGSEGV` after a different passing test each time. One `WgpuSetup::Existing` for
//! the process means `vkCreateInstance` happens once, with no other thread in the loader,
//! and nothing destroys it either. Sharing one instance is also why that instance offers the
//! app's own backends, `adapter::BACKENDS` — Vulkan on Linux, Metal on macOS — and never GL:
//! every harness's renderer enumerates its adapters, a GL adapter's `AdapterContext` clones
//! share one EGL context, and an EGL context made current on two threads at once is the
//! `BadAccess` `wgpu_hal::gles::egl` unwraps. `SETUP` keeps that
//! enumeration — the only work left that reaches into the shared instance — to one thread at
//! a time. See [docs/testing.md](../docs/testing.md).

use std::sync::{Mutex, OnceLock, PoisonError};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use emath::Pos2;
use supersilvia::graph::{LayoutMode, PortRef};
use supersilvia::project::Active;
use supersilvia::render::adapter;
use supersilvia::{App, Command};

/// A node's menu path: its category group, then the entry. The Nodes menu groups by
/// `NodeDef::category`, so reaching one is two clicks rather than one.
const ADD_BRICKGAME: (&str, &str) = ("Generate", "add brickgame");
const ADD_CELLULARAUTOMATA: (&str, &str) = ("Generate", "add cellularautomata");
const ADD_CHECKERBOARD: (&str, &str) = ("Generate", "add checkerboard");
const ADD_COLOR: (&str, &str) = ("Generate", "add color");
const ADD_COSINEGRADIENT: (&str, &str) = ("Color", "add cosinegradient");
const ADD_EUCLIDEANRHYTHM: (&str, &str) = ("Control", "add euclideanrhythm");
const ADD_STEPSEQUENCER: (&str, &str) = ("Control", "add stepsequencer");
const ADD_OUTPUT: (&str, &str) = ("Output", "add output");
const ADD_PHASE: (&str, &str) = ("Gears", "add ratiogear");
const ADD_MASTERGEAR: (&str, &str) = ("Gears", "add mastergear");
const ADD_SLEW: (&str, &str) = ("Control", "add slew");
const ADD_TAP: (&str, &str) = ("Tap", "add tap");
const ADD_ADD: (&str, &str) = ("Math", "add add");
const ADD_ADSR: (&str, &str) = ("Control", "add adsr");
const ADD_LUMINOSITY: (&str, &str) = ("Convert", "add luminosity");
const ADD_LYAPUNOV: (&str, &str) = ("Generate", "add lyapunov");
const ADD_SLIMEMOLD: (&str, &str) = ("Generate", "add slimemold");
const ADD_REFRAMERANGE: (&str, &str) = ("Convert", "add reframerange");
const ADD_VIDEO: (&str, &str) = ("Source", "add video");
const ADD_IMAGEGIF: (&str, &str) = ("Source", "add imagegif");
const ADD_MAININPUT: (&str, &str) = ("Source", "add maininput");
const ADD_ZOOM: (&str, &str) = ("Transform", "add zoom");

/// The one wgpu instance, adapter and device this process renders through, over Vulkan or
/// Metal, on the integrated GPU: `render::adapter::choose` with `Asked::integrated`, a test's
/// choice rather than the app's default of the strongest GPU, so a snapshot is never drawn on
/// a discrete GPU or on llvmpipe, where kittest's own selector prefers a software adapter and
/// then a discrete one. Created on the first snapshot and held for the run.
fn shared_gpu() -> egui_wgpu::WgpuSetup {
    static GPU: OnceLock<egui_wgpu::WgpuSetupExisting> = OnceLock::new();
    egui_wgpu::WgpuSetup::Existing(
        GPU.get_or_init(|| {
            let mut setup = egui_kittest::wgpu::default_wgpu_setup();
            if let egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut setup {
                create_new.instance_descriptor.backends = adapter::BACKENDS;
                create_new.native_adapter_selector = Some(std::sync::Arc::new(
                    |adapters: &[egui_wgpu::wgpu::Adapter], _surface| {
                        adapter::pick(adapters, &adapter::Asked::integrated())
                    },
                ));
            }
            let state = egui_kittest::wgpu::create_render_state(
                setup,
                egui_wgpu::RendererOptions::PREDICTABLE,
            );
            egui_wgpu::WgpuSetupExisting {
                instance: state.instance.clone(),
                adapter: state.adapter.clone(),
                device: state.device.clone(),
                queue: state.queue.clone(),
            }
        })
        .clone(),
    )
}

/// The snapshots are drawn on the machine's integrated GPU, which the harness asks for by
/// name: an Intel iGPU (Mesa) on Linux, never a discrete GPU and never a software rasterizer,
/// and the Apple GPU on a Mac.
#[test]
fn the_test_adapter_is_the_igpu() {
    let egui_wgpu::WgpuSetup::Existing(gpu) = shared_gpu() else {
        unreachable!("shared_gpu hands out an existing device");
    };
    let info = gpu.adapter.get_info();
    assert_eq!(
        info.device_type,
        egui_wgpu::wgpu::DeviceType::IntegratedGpu,
        "{}",
        adapter::describe(&info)
    );
    assert_ne!(info.vendor, adapter::NVIDIA, "{}", adapter::describe(&info));
}

/// Held for as long as one harness is building its renderer over `shared_gpu`.
static SETUP: Mutex<()> = Mutex::new(());

/// A renderer over `shared_gpu`, still lazy: a test that takes no snapshot reaches no GPU,
/// and `cc.wgpu_render_state` stays `None` as `LazyRenderer`'s default leaves it.
fn renderer() -> egui_kittest::LazyRenderer {
    egui_kittest::LazyRenderer::new(|| {
        let _held = SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        egui_kittest::wgpu::WgpuTestRenderer::from_setup(shared_gpu())
    })
}

/// The app under test, with preferences that live for the test only. `Store::in_memory` is
/// what keeps a test off the real `preferences.json`.
///
/// **What the machine offers is pinned to what a Linux machine without the NDI® runtime
/// offers**, so a snapshot is the same on every machine: NDI reads as not installed and
/// Syphon as absent (`video::ndi::pretend_missing`, `platform::syphon::pretend_unavailable`),
/// on the test's own thread, which the editor and its inline synth both run on. An Output's
/// NDI row says *runtime not installed*, and there is no Syphon row, mark or node, on a box
/// with the runtime and on a Mac alike. The tests about sending take [`machine_app`].
fn app(cc: &mut eframe::CreationContext<'_>) -> App {
    supersilvia::video::ndi::pretend_missing(true);
    supersilvia::platform::syphon::pretend_unavailable(true);
    App::new(cc, supersilvia::preferences::Store::in_memory())
}

/// The app under test as this machine answers: NDI as its runtime probe found it, waited for
/// so the first frame is drawn from the answer rather than from whichever side of the probe it
/// lands, and Syphon where this is a Mac. For the tests about sending out and choosing a
/// source, which take no snapshot, since what they draw is this machine's.
fn machine_app(cc: &mut eframe::CreationContext<'_>) -> App {
    supersilvia::video::ndi::pretend_missing(false);
    supersilvia::platform::syphon::pretend_unavailable(false);
    supersilvia::video::ndi::runtime();
    App::new(cc, supersilvia::preferences::Store::in_memory())
}

/// An app with preferences that live for the test only.
fn harness<'a>() -> Harness<'a, App> {
    Harness::builder().renderer(renderer()).build_eframe(app)
}

/// The same app in a window tall enough to show the whole Main Mixer panel without
/// scrolling, for the tests that reach its lower controls.
fn tall_harness<'a>() -> Harness<'a, App> {
    Harness::builder()
        .renderer(renderer())
        .with_size(egui::vec2(1000.0, 1100.0))
        .build_eframe(app)
}

/// [`tall_harness`], answering as this machine does ([`machine_app`]).
fn tall_machine_harness<'a>() -> Harness<'a, App> {
    Harness::builder()
        .renderer(renderer())
        .with_size(egui::vec2(1000.0, 1100.0))
        .build_eframe(machine_app)
}

/// The same app in a window wide enough that a window floating on the canvas leaves room
/// beside it, for the tests that need the canvas both under a window and clear of it.
fn wide_harness<'a>() -> Harness<'a, App> {
    Harness::builder()
        .renderer(renderer())
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(app)
}

/// Double-click a widget by name.
///
/// Both press/release pairs go into **one frame**. kittest's clock advances 0.75 s per
/// interaction, which is past egui's 0.3 s `max_double_click_delay`, so two `click()` calls
/// can never be a double-click however close together they are written. Events inside a
/// frame all carry that frame's time, so the delta is zero and the count reaches two.
fn double_click(h: &mut Harness<'_, App>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(at));
    for _ in 0..2 {
        for pressed in [true, false] {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    h.step();
}

/// Node entries live under the Nodes menu, so a test has to open it and pick the category
/// first. The menu is built from the registry, and every entry in it — there and in the
/// browser, which draws the same rows — is named for the node it adds. A category of one,
/// Output, is its node: the click on its row adds it, and there is no entry to pick.
fn add_node(h: &mut Harness<'_, App>, path: (&str, &str)) {
    let (group, label) = path;
    let before = h.state().graph().len();
    h.get_by_label("Nodes").click();
    // Two frames. The first time an `Area` appears egui runs a sizing pass — the content is
    // laid out at a provisional position to measure it, then discarded and re-run. A real
    // frame never shows it, because `Context::run` repeats the pass before painting; kittest
    // runs exactly one pass per step, so the tree after the opening step is that provisional
    // geometry and a click made from it lands nowhere.
    h.run_steps(2);
    // A button: an Output's name field says `supersilvia Output …` too.
    h.get(
        egui_kittest::kittest::By::new()
            .role(egui::accesskit::Role::Button)
            .label_contains(group),
    )
    .click();
    h.step();
    if h.state().graph().len() == before {
        h.get_by_label(label).click();
    }
    // Two frames: the entry is clicked on the first, which has already drawn the menu, and
    // the second is the one where the menu is gone.
    h.run_steps(2);
}

/// One node laid out alone, as the canvas lays it out.
fn laid_out(
    h: &Harness<'_, App>,
    id: supersilvia::graph::NodeId,
) -> supersilvia::ui::canvas::Layouts {
    supersilvia::ui::canvas::Layouts::one(h.state().graph(), id)
}

/// A node's body in world units.
fn node_rect(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> egui::Rect {
    laid_out(h, id).find(id).expect("the node").rect
}

/// How tall a node is in world units.
fn node_height(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> f32 {
    laid_out(h, id).find(id).expect("the node").height
}

/// Where a node's own picture of what it publishes is drawn, below its heading, in world
/// units: an empty rect at the body's foot for a node that shows none. Every picture region
/// wears a heading.
fn preview_band(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> egui::Rect {
    let node = h.state().graph().get(id).expect("the node");
    let laid = laid_out(h, id);
    let l = laid.find(id).expect("the node");
    l.regions
        .iter()
        .find(|b| match b.region {
            supersilvia::ui::canvas::Region::Declared(i) => matches!(
                node.def.regions[i].picture(),
                Some(supersilvia::nodes::Picture::Port(_))
            ),
            supersilvia::ui::canvas::Region::Pad => false,
        })
        .map_or(
            egui::Rect::from_two_pos(l.rect.left_bottom(), l.rect.right_bottom()),
            |b| {
                let top = b.rect.min.y + supersilvia::ui::canvas::HEADING_HEIGHT;
                egui::Rect::from_min_max(
                    egui::pos2(b.rect.min.x, top.min(b.rect.max.y)),
                    b.rect.max,
                )
            },
        )
}

#[test]
fn starts_empty_and_without_a_gl_context() {
    let mut h = harness();
    h.step();

    // §7b: the app must run with no GPU.
    assert!(h.state().graph().is_empty());
    assert!(h.query_by_label("Nodes").is_some(), "the menu bar is there");
}

#[test]
fn clicking_add_node_adds_a_node() {
    let mut h = harness();
    h.step();

    add_node(&mut h, ADD_CHECKERBOARD);

    assert_eq!(h.state().graph().len(), 1);
    assert_eq!(h.state().history().len(), 1);
    h.snapshot("one_node");
}

/// Open the Preferences window the way a hand does: `Edit ▸ Preferences…`.
fn open_preferences(h: &mut Harness<'_, App>) {
    h.get_by_label("Edit").click();
    // The sizing pass, as `add_node` documents: a menu's first frame is provisional
    // geometry and a click taken from it lands nowhere.
    h.run_steps(2);
    h.get_by_label_contains("Preferences").click();
    h.run_steps(2);
}

/// Show one of the Preferences window's tabs, by its name on the strip.
fn preferences_tab(h: &mut Harness<'_, App>, tab: &str) {
    h.get_by_label(tab).click();
    h.run_steps(2);
}

#[test]
fn preferences_opens_from_the_edit_menu_with_the_four_anchors() {
    let mut h = harness();
    h.step();
    open_preferences(&mut h);

    // Each anchor is a caption and a swatch, and both carry the caption's words — so the
    // swatch is matched by the color `color::swatch` appends to it, which only it has.
    for label in ["Main UI", "Number ports", "Color ports", "Event ports"] {
        assert!(
            h.query_by_label_contains(&format!("{label} #")).is_some(),
            "{label} has no swatch in the window",
        );
    }
    // The window is ordinary egui, so it is in the accessibility tree by construction —
    // the reason it is a `Window` and not a hand-painted surface.
    assert!(
        h.query_by_label_contains("🍦 Vanilla").is_some(),
        "the presets are there"
    );
    h.snapshot("preferences_window");
}

/// **The Preferences window is four tabs at one height**, and opens again on the tab it was
/// closed on. Every tab is drawn at the tallest one's height, so a click on the strip leaves
/// the window exactly where and as large as it was.
#[test]
fn the_preferences_tabs_keep_one_height_and_the_last_tab_comes_back() {
    let mut h = harness_with_gpus(egui::vec2(820.0, 1000.0));
    h.step();
    open_preferences(&mut h);
    let window = h.get_by_label("Preferences").rect();
    for (tab, holds) in [
        ("Editing", "Lock the cursor while scrubbing"),
        ("Performance", "Tick rate"),
        ("Files", "Projects folder"),
        ("Appearance", "Interface size"),
    ] {
        preferences_tab(&mut h, tab);
        assert!(h.query_by_label(holds).is_some(), "{tab} holds {holds}");
        assert_eq!(
            h.get_by_label("Preferences").rect(),
            window,
            "{tab} moved the window"
        );
        // Appearance is `preferences_window`, and Files and Performance are drawn with a file
        // and a folder of their own in the test after this; the Editing tab holds nothing of
        // the machine's.
        if tab == "Editing" {
            h.snapshot("preferences_editing");
        }
    }

    preferences_tab(&mut h, "Performance");
    h.get_by_label("Close window").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Tick rate").is_none(),
        "the window is closed"
    );
    open_preferences(&mut h);
    assert!(
        h.query_by_label("Tick rate").is_some() && h.query_by_label("Interface size").is_none(),
        "the window opens again on Performance"
    );
}

/// An app on the default preferences, its window `size`, with [`fake_gpus`] as its machine.
fn harness_with_gpus<'a>(size: egui::Vec2) -> Harness<'a, App> {
    Harness::builder()
        .renderer(renderer())
        .with_size(size)
        .build_eframe(move |cc| {
            supersilvia::video::ndi::pretend_missing(true);
            supersilvia::platform::syphon::pretend_unavailable(true);
            let mut app = App::new(
                cc,
                supersilvia::preferences::Store::of(
                    supersilvia::preferences::Preferences::default(),
                ),
            );
            app.use_gpu_choice(Some(fake_gpus()));
            app
        })
}

/// An app started on these preferences, held in memory, with the machine pinned as [`app`]
/// pins it.
fn harness_with<'a>(prefs: supersilvia::preferences::Preferences) -> Harness<'a, App> {
    Harness::builder()
        .renderer(renderer())
        .build_eframe(move |cc| {
            supersilvia::video::ndi::pretend_missing(true);
            supersilvia::platform::syphon::pretend_unavailable(true);
            App::new(cc, supersilvia::preferences::Store::of(prefs.clone()))
        })
}

/// **The editor's windows open where the last run left them**, and at the interface size the
/// last run's preferences hold: both are `preferences.json`'s, read at start, with no
/// `app.ron` beside it. A window moved is written back, by its title.
#[test]
fn the_windows_and_the_size_open_where_the_last_run_left_them() {
    use supersilvia::preferences::{InterfaceSize, Placement, Preferences};
    let mut h = harness_with(Preferences {
        interface_size: InterfaceSize::Percent125,
        windows: [(
            "Preferences".to_string(),
            Placement {
                pos: [200.0, 40.0],
                size: None,
            },
        )]
        .into(),
        ..Preferences::default()
    });
    h.step();
    assert!(
        (h.ctx.zoom_factor() - 1.25).abs() < 1e-4,
        "the size is the file's"
    );

    open_preferences(&mut h);
    let window = h.get_by_label("Preferences").rect();
    assert!(
        (window.min.x - 200.0).abs() < 1.0 && (window.min.y - 40.0).abs() < 1.0,
        "the window opens at its saved corner, not where egui would put it: {window:?}"
    );

    let grip = Pos2::new(window.center().x, window.min.y + 8.0);
    drag_with(
        &mut h,
        grip,
        grip + egui::vec2(60.0, 30.0),
        egui::Modifiers::NONE,
    );
    h.run_steps(2);
    let kept = h.state().preferences().windows["Preferences"];
    assert_eq!(kept.pos, [260.0, 70.0], "the move is written back");
    assert_eq!(
        kept.size, None,
        "and a window that sizes itself keeps no size"
    );
}

/// **The interface size scales the whole editor, and the zoom keys are the canvas's.**
/// Preferences ▸ Interface size sets egui's zoom factor, so the window and everything in it
/// grows in points, and it is kept. `Ctrl` with `+`, `-` and `0` zoom the canvas about its
/// middle and leave the editor's size alone.
#[test]
fn the_interface_size_scales_the_editor_and_the_zoom_keys_zoom_the_canvas() {
    use supersilvia::preferences::InterfaceSize;
    let mut h = harness();
    h.step();
    open_preferences(&mut h);
    let before = h.ctx.content_rect();
    h.get_by_label("125%").click();
    h.run_steps(3);
    assert_eq!(
        h.state().preferences().interface_size,
        InterfaceSize::Percent125
    );
    assert!(
        (h.ctx.zoom_factor() - 1.25).abs() < 1e-4,
        "{}",
        h.ctx.zoom_factor()
    );
    // The same window holds fewer points, each drawn larger.
    let after = h.ctx.content_rect();
    assert!(
        (after.width() * 1.25 - before.width()).abs() < 1.0,
        "{before:?} {after:?}"
    );
    h.key_press(egui::Key::Escape);
    h.run_steps(2);

    let zoom = h.state().canvas_transform().zoom;
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Plus);
    h.run_steps(2);
    assert!(
        h.state().canvas_transform().zoom > zoom,
        "Ctrl+ zooms the canvas in"
    );
    assert!(
        (h.ctx.zoom_factor() - 1.25).abs() < 1e-4,
        "and leaves the editor's size alone"
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num0);
    h.run_steps(2);
    assert!(
        (h.state().canvas_transform().zoom - 1.0).abs() < 1e-4,
        "Ctrl+0 puts the canvas back to actual size"
    );

    // The next run opens at the size this one was left at.
    let kept = h.state().preferences().clone();
    let mut next = harness_with(kept);
    next.step();
    assert!((next.ctx.zoom_factor() - 1.25).abs() < 1e-4);
}

/// A machine with an integrated GPU, a discrete one and a software one, rendering on the first
/// because `SUPERSILVIA_ADAPTER` names it: what the GPU section is drawn from in a test, so its
/// snapshot does not depend on the GPUs of the machine it runs on.
fn fake_gpus() -> adapter::Choice {
    let gpu = |name: &str, kind, driver: &str, info: &str| eframe::wgpu::AdapterInfo {
        name: name.to_owned(),
        driver: driver.to_owned(),
        driver_info: info.to_owned(),
        ..eframe::wgpu::AdapterInfo::new(kind, eframe::wgpu::Backend::Vulkan)
    };
    adapter::Choice {
        offered: vec![
            gpu(
                "Intel(R) Graphics (RPL-S)",
                eframe::wgpu::DeviceType::IntegratedGpu,
                "Intel open-source Mesa driver",
                "Mesa 25.2.4",
            ),
            gpu(
                "NVIDIA GeForce RTX 3090",
                eframe::wgpu::DeviceType::DiscreteGpu,
                "NVIDIA",
                "580.95.05 with a driver string long enough to be cut at the window's edge",
            ),
            gpu(
                "llvmpipe (LLVM 22.1.8, 256 bits)",
                eframe::wgpu::DeviceType::Cpu,
                "llvmpipe",
                "Mesa 25.2.4 (LLVM 22.1.8)",
            ),
        ],
        chosen: 0,
        asked: adapter::Asked {
            adapter: Some("intel".to_owned()),
            software: false,
        },
    }
}

/// **Where things are kept, and what it draws on**, the Preferences window's Files and
/// Performance tabs.
///
/// Files: the projects folder with Show in Files (Show in Finder on the Mac) and Change…, the preferences file with Open
/// and Show, each path one line in monospace and cut in the middle where it is long, and the
/// line saying when an edit to the file takes effect. GPU: the adapter in use, how it was
/// picked, and every adapter offered with the one in use marked. Each tab is drawn whole, in a
/// window tall enough to hold it.
///
/// The preferences are a file under `/tmp` of this test's own, so the row shows a path and the
/// snapshot is the same on every run; the projects folder is a preference naming a folder
/// nobody makes, long enough to be cut; and the GPUs are [`fake_gpus`].
#[test]
fn the_preferences_window_says_where_things_are_kept_and_what_it_draws_on() {
    let dir = std::path::PathBuf::from("/tmp/supersilvia-ui-files");
    std::fs::remove_dir_all(&dir).ok();
    let file = dir.join("preferences.json");
    let projects = std::path::PathBuf::from(
        "/home/tester/Documents/a folder whose name runs on past the window/supersilvia",
    );
    let store = {
        let mut store = supersilvia::preferences::Store::load(Some(file.clone()));
        store.set_projects_dir(projects.clone());
        store
    };
    let store = std::sync::Mutex::new(Some(store));
    let mut h = Harness::builder()
        .renderer(renderer())
        .with_size(egui::vec2(820.0, 1300.0))
        .build_eframe(move |cc| {
            supersilvia::video::ndi::pretend_missing(true);
            supersilvia::platform::syphon::pretend_unavailable(true);
            let mut app = App::new(cc, store.lock().unwrap().take().expect("one app"));
            app.use_gpu_choice(Some(fake_gpus()));
            app
        });
    h.step();
    open_preferences(&mut h);
    preferences_tab(&mut h, "Files");

    let show_in = format!("Show in {}", supersilvia::platform::files::MANAGER);
    for label in [
        "Projects folder",
        "Change…",
        show_in.as_str(),
        "Preferences file",
        "Open",
        "Show",
    ] {
        assert!(
            h.query_by_label(label).is_some(),
            "{label} is in the window"
        );
    }
    assert!(
        h.query_by_label_contains(&file.display().to_string())
            .is_some(),
        "the preferences file's path, whole"
    );
    assert!(
        h.query_by_label_contains("/home/tester/Doc").is_some()
            && h.query_by_label_contains("past the window/supersilvia")
                .is_some()
            && h.query_by_label_contains(&projects.display().to_string())
                .is_none(),
        "the projects folder's, cut in the middle"
    );
    assert!(
        h.query_by_label_contains("Edits take effect the next time supersilvia starts")
            .is_some()
    );
    #[cfg(target_os = "linux")]
    h.snapshot("preferences_files");

    preferences_tab(&mut h, "Performance");
    for label in [
        "Intel(R) Graphics (RPL-S)",
        "SUPERSILVIA_ADAPTER=intel",
        "● Intel(R) Graphics (RPL-S)  (in use)",
        "○ NVIDIA GeForce RTX 3090",
        "○ llvmpipe (LLVM 22.1.8, 256 bits)",
    ] {
        assert!(
            h.query_by_label(label).is_some(),
            "{label} is in the GPU section"
        );
    }
    // The one answer of the machine's this window shows is the file manager's name, Files or
    // Finder, so the pictures are Linux's and a Mac checks the labels alone.
    #[cfg(target_os = "linux")]
    h.snapshot("preferences_performance");
    std::fs::remove_dir_all(&dir).ok();
}

/// **A projects folder that cannot be read says so beside its path**, in the Preferences window
/// and in New project's, rather than standing in for another folder.
#[cfg(unix)]
#[test]
fn a_projects_folder_that_cannot_be_read_says_so_in_both_windows() {
    use std::os::unix::fs::PermissionsExt as _;
    let locked = std::env::temp_dir().join(format!("ssw-ui-locked-{}", std::process::id()));
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = || {
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&locked);
    };
    if std::fs::read_dir(&locked).is_ok() {
        eprintln!("permissions do not bind here; skipping");
        unlock();
        return;
    }
    let mut h = harness_with(supersilvia::preferences::Preferences {
        projects_dir: Some(locked.clone()),
        ..Default::default()
    });
    h.step();
    open_preferences(&mut h);
    preferences_tab(&mut h, "Files");
    let said = format!("could not read the projects folder {}", locked.display());
    assert!(h.query_by_label_contains(&said).is_some(), "under the path");

    h.get_by_label("Close window").click();
    h.run_steps(2);
    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label("New project…").click();
    h.run_steps(3);
    assert!(
        h.query_by_label_contains(&format!(
            "Could not read the projects folder {}",
            locked.display()
        ))
        .is_some(),
        "in New project's window"
    );
    assert!(h.get_by_label("Create").accesskit_node().is_disabled());
    unlock();
}

/// **A Save of the launch's scratch project is Save as…**: with nowhere to make `Untitled`,
/// the editor came up on an empty project in the temp folder, and `Ctrl+S` there puts the
/// folder dialog up rather than writing into the temp folder. The project saved is the
/// folder chosen, and the next `Ctrl+S` is an ordinary Save.
#[cfg(unix)]
#[test]
fn saving_the_launchs_scratch_project_asks_for_a_folder() {
    use std::os::unix::fs::PermissionsExt as _;
    let base = std::env::temp_dir().join(format!("ssw-ui-unplaced-{}", std::process::id()));
    let locked = base.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let cleanup = || {
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&base);
    };
    if std::fs::read_dir(&locked).is_ok() {
        eprintln!("permissions do not bind here; skipping");
        cleanup();
        return;
    }
    let mut h = harness_with(supersilvia::preferences::Preferences {
        projects_dir: Some(locked.join("supersilvia")),
        ..Default::default()
    });
    h.step();
    h.state_mut().open_last_or_untitled();
    assert!(h.state().save_asks_for_a_folder());
    let scratch = h.state().project().root().to_path_buf();
    let chosen = base.join("friday");
    h.state_mut().answer_file_dialogs(Some(chosen.clone()));
    add_node(&mut h, ADD_CHECKERBOARD);

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.run_steps(3);
    assert_eq!(
        h.state().project().root(),
        chosen,
        "{}",
        h.state().file_status()
    );
    assert!(supersilvia::project::Project::is_project(&chosen));
    assert!(!h.state().dirty(), "what was on screen is in it");
    assert!(
        !scratch.join(supersilvia::project::MANIFEST).exists(),
        "and nothing was saved into the temp folder"
    );
    assert!(
        !h.state().save_asks_for_a_folder(),
        "the next Save is a Save"
    );
    cleanup();
}

/// **Project ▸ New project… asks for a name**, offering the next free *Untitled N* in the
/// projects folder, selected so typing replaces it, and makes the project there itself. A name
/// already there is refused in the window, with the reason on a line of its own and Create
/// off, and the buttons stay where they were.
///
/// The projects folder is one of this test's own under `/tmp`, holding an `Untitled`
/// already, so the window offers `Untitled 2` and the snapshot is the same on every run.
#[test]
fn new_project_asks_for_a_name_and_makes_it_in_the_projects_folder() {
    let projects = std::path::PathBuf::from("/tmp/supersilvia-ui-new/Documents/supersilvia");
    std::fs::remove_dir_all("/tmp/supersilvia-ui-new").ok();
    std::fs::create_dir_all(projects.join("Untitled")).unwrap();
    let mut h = harness_with(supersilvia::preferences::Preferences {
        projects_dir: Some(projects.clone()),
        ..Default::default()
    });
    h.step();
    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label("New project…").click();
    h.run_steps(3);

    assert!(h.state().asking_project_name());
    let field = h.get_by_label(supersilvia::ui::project_name::NAME);
    assert_eq!(field.value().as_deref(), Some("Untitled 2"));
    h.snapshot("new_project");

    let create = h.get_by_label("Create").rect();
    h.input_mut()
        .events
        .push(egui::Event::Text("Untitled".to_string()));
    h.run_steps(2);
    assert!(
        h.query_by_label("There is already a folder called Untitled here.")
            .is_some(),
        "the reason is in the window"
    );
    assert!(
        h.get_by_label("Create").accesskit_node().is_disabled(),
        "and Create is off"
    );
    assert_eq!(h.get_by_label("Create").rect(), create, "and nothing moved");

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text("Friday".to_string()));
    h.run_steps(2);
    h.get_by_label("Create").click();
    h.run_steps(2);

    assert!(!h.state().asking_project_name(), "the window is down");
    assert_eq!(h.state().project().root(), projects.join("Friday"));
    assert!(supersilvia::project::Project::is_project(
        &projects.join("Friday")
    ));
    std::fs::remove_dir_all("/tmp/supersilvia-ui-new").ok();
}

/// **Project ▸ Save as… asks for a name too**, the same window as New project's under its own
/// heading, offering the project's own name or the next free one after it, and copies the
/// project into the projects folder under the name typed, carrying on there. `Ctrl`+Shift+S
/// puts it up as the menu entry does.
#[test]
fn save_as_asks_for_a_name_and_copies_into_the_projects_folder() {
    let projects = std::path::PathBuf::from("/tmp/supersilvia-ui-save-as/Documents/supersilvia");
    std::fs::remove_dir_all("/tmp/supersilvia-ui-save-as").ok();
    std::fs::create_dir_all(&projects).unwrap();
    let mut h = harness_with(supersilvia::preferences::Preferences {
        projects_dir: Some(projects.clone()),
        ..Default::default()
    });
    h.step();
    h.state_mut().new_project(projects.join("Friday"));
    h.step();
    h.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::S,
    );
    h.run_steps(3);

    assert!(h.state().asking_project_name());
    assert!(
        h.query_by_label("Save project as").is_some(),
        "its own heading"
    );
    let field = h.get_by_label(supersilvia::ui::project_name::NAME);
    assert_eq!(field.value().as_deref(), Some("Friday 2"));

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text("Saturday".to_string()));
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(2);

    assert!(!h.state().asking_project_name(), "the window is down");
    assert_eq!(h.state().project().root(), projects.join("Saturday"));
    assert!(supersilvia::project::Project::is_project(
        &projects.join("Saturday")
    ));
    assert!(
        supersilvia::project::Project::is_project(&projects.join("Friday")),
        "and Friday is where it was"
    );
    std::fs::remove_dir_all("/tmp/supersilvia-ui-save-as").ok();
}

/// **A file just written says so where a person sees it**: a Snap's or a render's toast, at
/// the foot of the window over the canvas, carries a **Show** after its text, as the Status
/// box's line does. It floats, so nothing under it moves.
#[test]
fn a_file_just_written_says_so_in_the_toast_with_a_show() {
    let mut h = harness();
    h.step();
    let nodes = h.get_by_label("Nodes").rect();
    h.state_mut().say_written(
        "snapped 1280x720 to snaps/output3-20261002-101500.png",
        std::path::PathBuf::from("/tmp/supersilvia-ui-toast/output3-20261002-101500.png"),
    );
    h.run_steps(2);
    assert!(
        h.query_by_label("snapped 1280x720 to snaps/output3-20261002-101500.png")
            .is_some()
    );
    assert!(h.query_by_label("Show").is_some(), "with a Show beside it");
    assert_eq!(h.get_by_label("Nodes").rect(), nodes, "and nothing moved");
    h.snapshot("toast_with_show");
}

/// A folder of this test's own under the temp dir, empty.
fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("supersilvia-ui-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// **A failure toasts**, marked by the `⚠` and an edge in the accent, with the Status box
/// closed as it is by default — and **two at once both stand**: the toast keeps a short
/// queue, so a Snap's line is not erased by the failure that follows it.
#[test]
fn a_failure_toasts_beside_what_was_said_before_it() {
    let dir = scratch_dir("fail");
    let mut h = harness();
    h.step();
    assert!(
        !h.state().preferences().show_status_box,
        "the box starts closed"
    );
    h.state_mut().say_written(
        "snapped 1280x720 to snaps/output3-20261002-101500.png",
        dir.join("output3-20261002-101500.png"),
    );
    h.state_mut().open_project(dir.join("nowhere"));
    h.run_steps(2);
    let toasts: Vec<String> = h.state().toasts().into_iter().map(String::from).collect();
    assert_eq!(toasts.len(), 2, "{toasts:?}");
    assert!(toasts[0].starts_with("snapped "), "{toasts:?}");
    assert!(toasts[1].starts_with("open failed"), "{toasts:?}");
    assert!(
        h.query_by_label("⚠").is_some(),
        "the failure wears its mark"
    );
    assert!(
        h.query_by_label("Show").is_some(),
        "and the Snap keeps its Show"
    );
    assert!(
        h.state().file_status().starts_with("open failed"),
        "the status line says it too"
    );
    h.snapshot("toast_failure_under_a_snap");
    std::fs::remove_dir_all(&dir).ok();
}

/// The queue is short: a fourth toast pushes the oldest off, and the same failure said again
/// stands once rather than twice.
#[test]
fn the_toast_queue_is_short_and_says_a_thing_once() {
    let dir = scratch_dir("queue");
    let mut h = harness();
    h.step();
    // A folder with something in it is refused as a new project's, by name.
    for name in ["a", "b", "c", "d"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(dir.join(name).join("x"), "x").unwrap();
    }
    for name in ["a", "b", "a", "c", "d"] {
        h.state_mut().new_project(dir.join(name));
    }
    h.run_steps(2);
    let toasts: Vec<String> = h.state().toasts().into_iter().map(String::from).collect();
    assert_eq!(
        toasts,
        ["a is not empty", "c is not empty", "d is not empty"],
        "the newest at the foot, b pushed off, and a once"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// **The problems badge counts what is wrong, and its list puts errors first.** Every
/// warning of the last Open is there, not only the first the status line had room for, and a
/// failure since sits above them; **Clear** forgets what was said, and the badge's slot goes
/// back to empty without moving the time readout beside it.
#[test]
fn the_problems_list_holds_every_warning_of_the_last_open_errors_first() {
    let dir = scratch_dir("problems");
    let root = dir.join("Friday");
    let mut h = harness();
    h.step();
    let time = h.get_by_label("time.reset").rect();
    assert!(
        h.query_by_label_contains("problems ").is_none(),
        "nothing to count: an empty slot"
    );
    h.state_mut().new_project(root.clone());
    let workspace = h.state().graph().default_workspace();
    for slug in ["checkerboard", "spiral", "output"] {
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at: Pos2::ZERO,
                workspace,
            })
            .unwrap();
    }
    h.state_mut().save_project().unwrap();
    // Two kinds this build does not have, which an Open drops with a warning each.
    for entry in std::fs::read_dir(root.join("workspaces")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = text
            .replace("\"checkerboard\"", "\"flanger9000\"")
            .replace("\"spiral\"", "\"wobbler\"");
        assert_ne!(text, edited);
        std::fs::write(&path, edited).unwrap();
    }
    h.state_mut().open_project(root.clone());
    h.state_mut().open_project(dir.join("nowhere"));
    h.run_steps(2);

    let problems = h.state().problems();
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems[0].text.starts_with("open failed"), "{problems:?}");
    assert!(problems[1].text.contains("flanger9000"), "{problems:?}");
    assert!(problems[2].text.contains("wobbler"), "{problems:?}");
    assert_eq!(
        h.get_by_label("time.reset").rect(),
        time,
        "the slot moved nothing"
    );

    h.get_by_label("problems 3").click();
    h.run_steps(3);
    assert!(h.state().problems_open());
    h.get_by_label_contains("problem 1 open failed");
    h.get_by_label_contains("problem 3 opening: ");
    h.snapshot("problems_list");

    h.get_by_label("Clear problems").click();
    h.run_steps(2);
    assert!(h.state().problems().is_empty());
    assert!(h.query_by_label_contains("problems ").is_none());
    assert_eq!(h.get_by_label("time.reset").rect(), time);
    std::fs::remove_dir_all(&dir).ok();
}

/// **A node at fault wears a flag on its header**, whatever the Status box is doing, named
/// with the reason its hover gives; its row in the problems list goes to it, across tabs.
#[test]
fn a_node_at_fault_wears_a_flag_and_its_row_goes_there() {
    let mut h = harness();
    h.step();
    let workspace = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddNode {
            slug: "imagegif",
            at: Pos2::ZERO,
            workspace,
        })
        .unwrap();
    let id = h.state().graph().iter().next().expect("one node").0;
    h.state_mut()
        .apply(Command::SetOption {
            node: id,
            key: "file",
            value: "assets/nowhere.gif".to_string(),
        })
        .unwrap();
    let flag = format!("imagegif{id} fault");
    for _ in 0..200 {
        h.step();
        if h.query_by_label_contains(&flag).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.get_by_label_contains(&flag);
    h.run_steps(2);
    h.snapshot("node_fault_flag");
    let problems = h.state().problems();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].go, Some(id));
    assert!(problems[0].text.starts_with(&format!("imagegif{id}: ")));

    h.state_mut().activate(Active::Project);
    h.run_steps(2);
    h.get_by_label("problems 1").click();
    h.run_steps(3);
    h.get_by_label("problem 1 go").click();
    h.run_steps(2);
    assert_eq!(
        h.state().active(),
        Active::Workspace(workspace),
        "it went there"
    );
    assert!(!h.state().problems_open(), "and the list closed behind it");
}

/// Choose an entry from the Help menu, the way a hand does.
fn from_help(h: &mut Harness<'_, App>, entry: &str) {
    h.get_by_label("Help").click();
    h.run_steps(2);
    h.get_by_label(entry).click();
    h.run_steps(2);
}

/// Help ▸ About supersilvia opens About, whose Licences… opens Licences beside it, and each
/// closes by its own ✕.
#[test]
fn help_about_shows_the_version_the_licence_and_the_source() {
    let mut h = harness();
    h.step();
    from_help(&mut h, "About supersilvia");

    let version = format!("Version {}", env!("CARGO_PKG_VERSION"));
    assert!(h.query_by_label(&version).is_some(), "the version is shown");
    assert!(
        h.query_by_label_contains("GNU Affero General Public License")
            .is_some(),
        "the licence is named"
    );
    assert!(
        h.query_by_label_contains("section 7").is_some(),
        "and its NDI® permission"
    );
    assert!(
        h.query_by_label(env!("CARGO_PKG_REPOSITORY")).is_some(),
        "the source is linked"
    );
    let edge = h.get_by_label("Close window").rect().right();
    for label in [
        "Licences…",
        "Every crate, font and library in it, and their terms",
    ] {
        assert!(
            h.get_by_label(label).rect().right() <= edge,
            "{label} stays inside the window"
        );
    }
    h.snapshot("about_window");

    h.get_by_label("Licences…").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Rust crates").is_some(),
        "Licences opened beside About"
    );
    assert!(h.query_by_label(&version).is_some(), "About stayed up");

    for _ in 0..2 {
        h.get_all_by_label("Close window")
            .last()
            .expect("a window to close")
            .click();
        h.run_steps(2);
    }
    assert!(h.query_by_label(&version).is_none(), "About closed");
    assert!(h.query_by_label("Rust crates").is_none(), "Licences closed");
}

/// Help ▸ Licences… opens on supersilvia's own licence, and each section is a tab of the one
/// frame — the crates' this machine's notices, from their first line.
#[test]
fn help_licences_shows_every_section() {
    let mut h = harness();
    h.step();
    from_help(&mut h, "Licences…");

    assert!(
        h.query_by_label_contains("GNU AFFERO GENERAL PUBLIC LICENSE")
            .is_some(),
        "supersilvia's licence is the first section"
    );
    h.snapshot("licences_window");

    let crates = supersilvia::platform::notices::RUST_CRATES
        .lines()
        .next()
        .expect("a heading");
    for (tab, line) in [
        ("Rust crates", crates),
        ("Assets", "licenses/hack.txt"),
        ("GStreamer", "GNU Lesser General Public License"),
        ("NDI®", "NDI® is a registered trademark of Vizrt NDI AB."),
    ] {
        h.get_by_label(tab).click();
        h.run_steps(2);
        assert!(
            h.query_by_label_contains(line).is_some(),
            "{tab} shows {line:?}"
        );
    }
}

/// Open Help ▸ Report a problem… and wait for what it says about this computer.
fn open_report(h: &mut Harness<'_, App>) {
    h.step();
    from_help(h, "Report a problem…");
    assert!(h.state().reporting(), "the form is open");
    for _ in 0..600 {
        if h.state().report_gathered() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.step();
    }
    assert!(h.state().report_gathered(), "the metadata is gathered");
    h.run_steps(2);
}

/// What the last frame asked of the desktop: the text it copied and the pages it opened.
fn asked_of_desktop(h: &Harness<'_, App>) -> (Vec<String>, Vec<String>) {
    let mut copied = Vec::new();
    let mut opened = Vec::new();
    for command in &h.output().platform_output.commands {
        match command {
            egui::OutputCommand::CopyText(text) => copied.push(text.clone()),
            egui::OutputCommand::OpenUrl(url) => opened.push(url.url.clone()),
            egui::OutputCommand::CopyImage(_) => {}
        }
    }
    (copied, opened)
}

/// Help ▸ Report a problem… opens a form with What's up? holding the keyboard and this
/// computer's version, OS and GPU under it; Copy report puts the words and the metadata on the
/// clipboard as one block and opens nothing; typing more does not move the window's rows.
#[test]
fn help_report_a_problem_copies_the_form_and_the_metadata() {
    use supersilvia::ui::report::{COPIED, COPY, TITLE};
    let mut h = harness();
    open_report(&mut h);
    assert!(h.query_by_label(TITLE).is_some());
    assert!(h.query_by_label("Version").is_some());
    assert!(h.query_by_label("GPU").is_some());
    assert!(
        h.query_by_label_contains("--check and the end of this run's log")
            .is_some()
    );

    let copy = h.get_by_label(COPY).rect();
    type_text(&mut h, "The mix went black");
    h.get_by_label(COPY).click();
    h.step();
    let (copied, opened) = asked_of_desktop(&h);
    assert!(opened.is_empty(), "copying opens nothing");
    let [text] = copied.as_slice() else {
        panic!("one block is copied: {copied:?}");
    };
    assert!(
        text.starts_with("**What's up?**\nThe mix went black\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "\nsupersilvia {}\nOS: ",
            env!("CARGO_PKG_VERSION")
        )),
        "{text}"
    );
    assert!(text.contains("\nGPU: "), "{text}");
    assert!(
        text.contains("\n--check\n```\n") && text.contains("GStreamer"),
        "--check's lines are in it: {text}"
    );
    assert!(text.contains("This run's log"), "{text}");
    assert!(
        !text.contains("Last run:"),
        "a run that closed says nothing of it"
    );
    h.step();
    assert!(
        h.query_by_label(COPIED).is_some(),
        "the button says it copied"
    );

    // Twenty more lines scroll inside the field rather than growing the window.
    h.get_by_label(supersilvia::ui::report::UP).click();
    h.step();
    type_text(&mut h, &"\nand then".repeat(20));
    h.run_steps(2);
    assert!(
        h.query_by_label(COPY).is_some(),
        "typing makes the copy stale"
    );
    assert_eq!(h.get_by_label(COPY).rect(), copy, "and nothing moved");

    h.get_by_label("Close window").click();
    h.run_steps(2);
    assert!(!h.state().reporting(), "the ✕ closes it");
}

/// Each link opens its page and copies nothing: the Discord's bug channel, and a new GitHub
/// issue titled with the first line of What's up?.
#[test]
fn the_report_s_two_links_open_their_pages() {
    use supersilvia::app::crashlog::{BUG_CHANNEL, issue_url};
    use supersilvia::ui::report::{DISCORD, GITHUB};
    let mut h = harness();
    open_report(&mut h);
    type_text(&mut h, "Black screen\nafter a reload");

    h.get_by_label(DISCORD).click();
    h.step();
    assert_eq!(
        asked_of_desktop(&h),
        (Vec::new(), vec![BUG_CHANNEL.to_owned()])
    );

    h.get_by_label(GITHUB).click();
    h.step();
    let want = issue_url("Black screen\nafter a reload");
    assert!(want.ends_with("/issues/new?title=Black%20screen"), "{want}");
    assert_eq!(asked_of_desktop(&h), (Vec::new(), vec![want]));
}

/// A project left with unsaved edits by a run that went down: saved once, edited, autosaved.
fn crashed_project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-ui-crashed-{}-{name}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join("friday");
    let mut app = App::headless();
    app.new_project(root.clone());
    let workspace = app.graph().default_workspace();
    app.apply(Command::AddNode {
        slug: "checkerboard",
        at: Pos2::new(40.0, 40.0),
        workspace,
    })
    .unwrap();
    app.save_project().unwrap();
    app.apply(Command::AddNode {
        slug: "circle",
        at: Pos2::new(240.0, 40.0),
        workspace,
    })
    .unwrap();
    app.autosave_tick(0.0);
    app.wait_for_autosave();
    root
}

/// *supersilvia closed unexpectedly* says why the last run went down, with Show log, and the
/// recovery question waits behind it until OK.
#[test]
fn the_last_run_is_noticed_before_the_recovery_question() {
    use supersilvia::app::crashlog::Unexpected;
    let root = crashed_project("noticed");
    let last = Unexpected {
        why: Some("thread 'synth' panicked at src/a.rs:1:2: boom".to_owned()),
        log: std::path::PathBuf::from("/nowhere/previous.log"),
    };
    let mut h = Harness::builder()
        .renderer(renderer())
        .build_eframe(move |cc| {
            let mut app = app(cc);
            app.notice_last_run(Some(last.clone()));
            app.open_project(root.clone());
            app
        });
    h.run_steps(3);

    assert!(
        h.query_by_label("supersilvia closed unexpectedly")
            .is_some()
    );
    assert!(
        h.query_by_label_contains("it stopped with: thread 'synth' panicked at src/a.rs:1:2: boom")
            .is_some(),
        "the reason is said"
    );
    assert!(h.query_by_label("Show log").is_some());
    assert!(
        h.state().recovery_offered().is_some(),
        "the autosave is on offer"
    );
    assert!(
        h.query_by_label("Recover").is_none(),
        "and its question waits"
    );

    h.get_by_label("OK").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("supersilvia closed unexpectedly")
            .is_none()
    );
    assert!(h.state().last_run_noticed().is_none());
    assert!(
        h.query_by_label("Recover").is_some(),
        "the recovery question follows"
    );
}

/// A run that wrote no reason is said to have been stopped from outside.
#[test]
fn a_last_run_with_no_reason_says_what_that_means() {
    use supersilvia::app::crashlog::Unexpected;
    let mut h = Harness::builder().renderer(renderer()).build_eframe(|cc| {
        let mut app = app(cc);
        app.notice_last_run(Some(Unexpected {
            why: None,
            log: std::path::PathBuf::from("/nowhere/previous.log"),
        }));
        app
    });
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("it wrote no reason").is_some(),
        "the notice says there is none"
    );
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("supersilvia closed unexpectedly")
            .is_none()
    );
}

/// Every checkbox in the window writes the preference it names, both ways, and nothing else.
#[test]
fn every_preferences_checkbox_writes_its_own_preference() {
    use supersilvia::preferences::Flag;
    let mut h = harness();
    h.step();
    open_preferences(&mut h);
    let boxes = [
        (Flag::NodeShadow, "Appearance", "Nodes cast a shadow"),
        (Flag::CableDroop, "Appearance", "Droopy cables"),
        (Flag::PhiCables, "Appearance", "Phi-spaced cable colors"),
        (
            Flag::CursorLock,
            "Editing",
            "Lock the cursor while scrubbing",
        ),
        (
            Flag::PortHover,
            "Editing",
            "Light cables and ports on hover",
        ),
        (
            Flag::ScrollXInverted,
            "Editing",
            "Invert scrolling along a strip",
        ),
        (Flag::SoftTakeover, "Editing", "MIDI soft takeover"),
        (Flag::Fps, "Performance", "Frame rate in the menu bar"),
        (Flag::StatusBox, "Performance", "Show the Status box"),
    ];
    for (flag, tab, label) in boxes {
        preferences_tab(&mut h, tab);
        let before = h.state().preferences().clone();
        // A tab scrolls inside the window, and its lower rows can be below its foot here.
        h.get_by_label(label).scroll_to_me();
        h.run_steps(10);
        h.get_by_label(label).click();
        h.step();
        let after = h.state().preferences().clone();
        assert_eq!(
            after.flag(flag),
            !before.flag(flag),
            "{label} did not toggle"
        );
        for (other, _, _) in boxes.iter().filter(|(f, _, _)| *f != flag) {
            assert_eq!(
                after.flag(*other),
                before.flag(*other),
                "{label} moved {other:?}"
            );
        }
        h.get_by_label(label).click();
        h.step();
        // Where the windows stand is written as they are drawn, and the Status box is drawn
        // while it is ticked: that is the box remembering its place, not the checkbox.
        let back = supersilvia::preferences::Preferences {
            windows: before.windows.clone(),
            ..h.state().preferences().clone()
        };
        assert_eq!(back, before, "{label} did not toggle back");
    }
}

/// The headline claim of the design system, as a test: *a whole re-theme is four numbers*.
///
/// Two images of the same graph under two presets. Nothing else is touched between them —
/// no node moves, no text changes — so every pixel that differs differs because a color
/// derived from an anchor, and any pixel that stayed is a color that did not. A literal
/// `Color32` anywhere in the editor shows up here as a pixel that refused to move.
#[test]
fn one_graph_under_two_presets_moves_every_themed_pixel() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.snapshot("theme_vapor");

    open_preferences(&mut h);
    h.get_by_label_contains("🍦 Vanilla").click();
    // One frame to take the click, one to draw with the four new numbers.
    h.run_steps(2);

    let theme = h.state().preferences().theme;
    let vanilla = supersilvia::ui::theme::PRESETS
        .iter()
        .find(|p| p.key == "vanilla")
        .expect("vanilla is one of the sixteen");
    assert_eq!(
        theme, vanilla.theme,
        "the preset did not reach the preference"
    );

    // Shut the window before the second picture. Both images are then the same graph with
    // nothing else on screen, which is the whole point: every pixel that differs differs
    // because of the four numbers, not because a window is sitting over the node.
    h.get_by_label_contains("Close").click();
    h.run_steps(2);
    h.snapshot("theme_vanilla");
}

/// The swatches are the real `s-color`, not a settings-shaped lookalike — so the popup that
/// opens over the window is the same instrument, hex field and all, that a color port
/// carries. Typing a hex into it moves the anchor, which moves the whole editor.
#[test]
fn an_anchors_swatch_opens_the_real_picker_and_typing_a_hex_retints_everything() {
    let mut h = harness();
    h.step();
    open_preferences(&mut h);

    h.get_by_label_contains("Main UI #").click();
    h.step();
    type_hex(&mut h, "ff0000ff");

    let main = h.state().preferences().theme.main;
    assert_eq!(
        main.color(),
        egui::Color32::from_rgb(0xff, 0x00, 0x00),
        "the anchor is what was typed",
    );
    // And it is no longer any preset, because the four numbers are nobody's look now.
    assert!(
        !supersilvia::ui::theme::PRESETS
            .iter()
            .any(|p| p.theme == h.state().preferences().theme),
        "a hand-picked anchor should match no preset",
    );
}

/// A preference is not an edit. Re-theming the whole editor leaves the document exactly as
/// it was: no command, nothing to undo.
#[test]
fn choosing_a_preset_is_not_an_edit() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let history = h.state().history().len();
    let undo = h.state().undo_len();

    open_preferences(&mut h);
    h.get_by_label_contains("🦇 Gothic").click();
    h.run_steps(2);

    assert_eq!(
        h.state().history().len(),
        history,
        "a theme is not a command"
    );
    assert_eq!(h.state().undo_len(), undo, "and it is not an undo step");
}

/// A note is a node with no ports at all — the first in the library — whose whole body is
/// one value. Typing in it writes `Node::values`, and none of it is a control, an option or
/// a port.
#[test]
fn a_note_holds_text_that_is_not_a_control_an_option_or_a_port() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));

    let id = h.state().graph().iter().next().expect("one node").0;
    {
        let node = h.state().graph().get(id).expect("the note");
        assert!(
            node.inputs.is_empty() && node.outputs.is_empty(),
            "a note has no ports"
        );
        assert!(node.options.is_empty(), "and no options");
        assert!(node.values.is_empty(), "and nothing written in it yet");
    }

    h.get_by_label_contains("note1.text").click();
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::Text("wired backwards on purpose".to_string()));
    h.run_steps(2);

    let node = h.state().graph().get(id).expect("the note");
    assert_eq!(
        node.values
            .get("text")
            .and_then(supersilvia::graph::Value::text),
        Some("wired backwards on purpose"),
        "the text is in the node's own values",
    );
    assert!(node.controls.is_empty() && node.options.is_empty());
    h.snapshot("note");
}

/// A note is the one node a hand can drag wider, and the width it is dragged to is the
/// node's own from then on: it goes through the bus, so it is one undo step and it rides in
/// the file. The grip sits in the box's bottom-right corner, where silvia's `resize: both`
/// textarea puts a browser's own.
#[test]
fn a_notes_grip_drags_its_body_wider_in_one_undo_step() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));
    h.run_steps(2);

    let id = h.state().graph().iter().next().expect("one node").0;
    let (was, at) = {
        let node = h.state().graph().get(id).expect("the note");
        assert_eq!(node.dragged_width, None, "a new note is its kind's width");
        assert_eq!(node.dragged_height, None, "and its kind's height");
        (supersilvia::ui::canvas::node_width(node), node.pos)
    };
    let undo = h.state().undo_len();

    let grip = h.get_by_label_contains("note1.resize").rect().center();
    drag_with(
        &mut h,
        grip,
        grip + egui::vec2(120.0, 0.0),
        egui::Modifiers::NONE,
    );

    let node = h.state().graph().get(id).expect("the note");
    let width = supersilvia::ui::canvas::node_width(node);
    assert!(
        (width - (was + 120.0)).abs() <= 2.0,
        "the body followed the grip: {width} from {was}"
    );
    assert_eq!(node.dragged_width, Some(width), "and the node kept it");
    assert!(
        node.dragged_height.is_some(),
        "the same drag fixes the height too, even where the pointer never moved along it"
    );
    assert_eq!(node.pos, at, "the grip is not the body's own drag handle");
    assert_eq!(
        h.state().undo_len(),
        undo + 1,
        "a drag is one undo step, not one a frame"
    );

    h.state_mut().undo();
    h.run_steps(2);
    let node = h.state().graph().get(id).expect("the note");
    assert_eq!(
        node.dragged_width, None,
        "and undo hands the width back to the kind"
    );
    assert_eq!(node.dragged_height, None, "and the height too");
}

/// A note is never dragged narrower than what it is drawn with: the floor is the width the
/// kind asks for, so nothing a grip, a file or an undo hands over can put the box outside
/// the body holding it.
#[test]
fn a_note_cannot_be_dragged_narrower_than_its_kind() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));
    h.run_steps(2);

    let id = h.state().graph().iter().next().expect("one node").0;
    let floor = supersilvia::ui::canvas::natural_width(h.state().graph().get(id).unwrap());
    let grip = h.get_by_label_contains("note1.resize").rect().center();
    drag_with(
        &mut h,
        grip,
        grip - egui::vec2(400.0, 0.0),
        egui::Modifiers::NONE,
    );

    let node = h.state().graph().get(id).expect("the note");
    assert_eq!(
        supersilvia::ui::canvas::node_width(node),
        floor,
        "the body stopped at the width its rows need"
    );
}

/// The Text node is not a note: its body width is set by the kind, not by a hand, so its
/// grip only ever moves along one axis. Dragging it sideways as well as down must still
/// change nothing but the height.
#[test]
fn the_text_nodes_grip_changes_only_its_height() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Source", "add text"));
    h.run_steps(2);

    let id = h.state().graph().iter().next().expect("one node").0;
    let width = supersilvia::ui::canvas::node_width(h.state().graph().get(id).unwrap());

    let grip = h.get_by_label_contains("text1.resize").rect().center();
    drag_with(
        &mut h,
        grip,
        grip + egui::vec2(120.0, 40.0),
        egui::Modifiers::NONE,
    );

    let node = h.state().graph().get(id).expect("the text node");
    assert_eq!(
        node.dragged_width, None,
        "the grip's horizontal reach did nothing: the text node has no width of its own to set"
    );
    assert_eq!(
        supersilvia::ui::canvas::node_width(node),
        width,
        "and the body stayed the kind's own width"
    );
    assert!(
        node.dragged_height.is_some(),
        "but the drag did set its height"
    );
}

/// A note no longer grows to fit what is in it: its box is a fixed size, the lines the kind
/// declares or whatever a hand dragged it to, and text past that scrolls inside the box
/// instead of pushing the node taller.
///
/// There was a round trip here: the field measured its own wrapped height while it drew, and
/// the node was that tall on the next frame. Wrapping needs the font and the width, which
/// changed with the zoom, so the box visibly jumped a line at a time as a hand zoomed in or
/// out. A height that is the document's own, or the kind's declared lines, does not move.
#[test]
fn a_long_note_does_not_grow_to_fit_its_text() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));
    let id = h.state().graph().iter().next().expect("one node").0;
    let empty = node_height(&h, id);

    h.get_by_label_contains("note1.text").click();
    h.step();
    // Two hundred hard lines, well past the four the node declares and past the 320-point
    // cap the box used to grow to.
    h.input_mut().events.push(egui::Event::Text((1..=200).fold(
        String::new(),
        |mut s, n| {
            use std::fmt::Write;
            let _ = writeln!(s, "line {n}");
            s
        },
    )));
    h.run_steps(3);

    assert_eq!(
        node_height(&h, id),
        empty,
        "two hundred lines moved the node, so the box is still sized from its text",
    );
}

/// Zooming wraps a note's text at a different width, which is exactly the geometry that
/// drove the node's height before the box stopped measuring its text. A node's height is
/// world geometry and must not move when somebody zooms in.
#[test]
fn zooming_does_not_change_a_notes_height() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));
    let id = h.state().graph().iter().next().expect("one node").0;

    h.get_by_label_contains("note1.text").click();
    h.step();
    h.input_mut().events.push(egui::Event::Text(
        "a few\nhard\nlines\nof text\nto wrap".to_string(),
    ));
    h.run_steps(3);
    let before = node_height(&h, id);

    wheel(&mut h, -2.0);
    h.run_steps(2);

    assert_eq!(
        node_height(&h, id),
        before,
        "zooming moved the node's height"
    );
}

/// Typing a sentence into a note is one undo step, not one per keystroke — the same
/// coalescing a control's drag and a typed option's row get.
#[test]
fn typing_into_a_note_is_one_undo_step() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add note"));
    let undo = h.state().undo_len();

    h.get_by_label_contains("note1.text").click();
    h.step();
    for word in ["one ", "two ", "three"] {
        h.input_mut()
            .events
            .push(egui::Event::Text(word.to_string()));
        h.step();
    }
    h.run_steps(2);

    assert_eq!(h.state().undo_len(), undo + 1, "three keystrokes, one step");
}

/// A new workspace opens in the mode the preference names. The workspace's own mode is
/// document data from then on — this is only what it is born with.
#[test]
fn a_new_workspace_opens_in_the_preferred_layout() {
    use supersilvia::graph::LayoutMode;
    let mut h = harness();
    h.step();

    open_preferences(&mut h);
    preferences_tab(&mut h, "Editing");
    h.get_by_label_contains("Linear").click();
    h.run_steps(2);
    h.get_by_label_contains("Close").click();
    h.run_steps(2);

    // The `+` beside the tabs, and the one kind under it.
    h.get_by_label("+").click();
    h.run_steps(2);
    h.get_by_label("Video").click();
    h.run_steps(2);

    let made = h
        .state()
        .graph()
        .workspaces()
        .last()
        .expect("a workspace was added");
    assert_eq!(made.layout, LayoutMode::Linear);
    // And the one that was already there is untouched: a preference is not retroactive.
    let first = h.state().graph().workspaces().first().expect("the first");
    assert_eq!(first.layout, LayoutMode::Canvas);
}

/// **The Output's render folds under a heading, and its Snap is a button beside the decks.**
///
/// Two of the same change: the render was behind a tick in the row of ticks at the foot of
/// the node, which is the affordance for hiding a run of *ports* and cannot say *this part
/// of the node is here and closed*; and there was no way at all to take a picture out of a
/// running patch. The heading is the same bar a region wears, and its state is the same
/// option the tick was — `offline`, so a saved file lands where it always did.
#[test]
fn the_render_folds_under_a_heading_and_snap_is_a_button() {
    let mut h = harness();
    h.step();
    let out = {
        let app = h.state_mut();
        let ws = app.graph().default_workspace();
        app.apply(Command::AddNode {
            slug: "output",
            at: egui::pos2(60.0, 60.0),
            workspace: ws,
        })
        .unwrap();
        app.graph().iter().map(|(id, _)| id).max().unwrap()
    };
    h.run_steps(2);

    // Snap is an action input like the two decks, so it is a button a hand can press and a
    // port a sequencer can cable into.
    assert!(
        h.query_by_label(&format!("output{out}.snap")).is_some(),
        "Snap is a press row on the Output"
    );
    assert!(
        h.query_by_label_contains(&format!("output{out}.snap (action input)"))
            .is_some(),
        "and an action port, so a sequencer can fire it"
    );

    // There is no tick row on an Output any more: `offline` is the one option that was one,
    // and it is a heading now.
    assert!(
        h.query_by_label(&format!("output{out}.render")).is_none(),
        "the section is closed to start with"
    );
    let heading = format!("output{out}.offline");
    h.get_by_label(&heading).click();
    h.run_steps(2);
    assert_eq!(
        h.state().graph().get(out).unwrap().options.get("offline"),
        Some(&supersilvia::nodes::ON.to_string()),
        "the triangle writes the same option the tick did"
    );
    assert!(
        h.query_by_label(&format!("output{out}.render")).is_some(),
        "and the section is open"
    );
    // Still there, closed: a heading says what is under it whether or not it is open, which
    // is the whole of why it is not a tick.
    h.get_by_label(&heading).click();
    h.run_steps(2);
    assert!(
        h.query_by_label(&heading).is_some(),
        "the heading stays on the node"
    );
    assert!(
        h.query_by_label(&format!("output{out}.render")).is_none(),
        "with its rows folded away"
    );
}

/// **An action port throbs on the frame it fires**, and so does the button of an input
/// something fires into.
///
/// The firing is otherwise invisible: an action is a moment, and a node whose whole point is
/// that it fires at a time you cannot predict has nothing on it that moves. The throb is one
/// rule in the port drawing rather than a readout per node, so this checks the pixels — the
/// dot of the output that fired, and the button of the input at the far end of the cable,
/// which nothing is holding. Both are brighter on the frame the firing lands than they were
/// before it.
#[test]
fn an_action_port_throbs_on_the_frame_it_fires() {
    // A frame of its own length: the throb is a sixth of a second, and kittest's default
    // quarter-second step is longer than the whole of it.
    let mut h = Harness::builder()
        .renderer(renderer())
        .with_step_dt(1.0 / 60.0)
        .build_eframe(app);
    h.step();
    let (first, second) = {
        let app = h.state_mut();
        let ws = app.graph().default_workspace();
        let mut add = |at| {
            app.apply(Command::AddNode {
                slug: "button",
                at,
                workspace: ws,
            })
            .unwrap();
            app.graph().iter().map(|(id, _)| id).max().unwrap()
        };
        let first = add(egui::pos2(60.0, 60.0));
        let second = add(egui::pos2(60.0, 260.0));
        app.apply(Command::Connect {
            from: PortRef::new(first, "trigger"),
            to: PortRef::new(second, "press"),
        })
        .unwrap();
        (first, second)
    };
    h.run_steps(2);

    let dot = h
        .get_by_label_contains(&format!("button{first}.trigger (action output)"))
        .rect()
        .center();
    // Off the centered caption: the glyph inverts with the fill, so a pixel on it says
    // nothing about whether the button as a whole got brighter.
    let button = {
        let rect = h.get_by_label(&format!("button{second}.press")).rect();
        rect.left_center() + egui::vec2(6.0, 0.0)
    };
    let before = (luma_at(&mut h, dot), luma_at(&mut h, button));

    // Not a click: a press through the bus is what a sequencer, a MIDI note and a finger all
    // arrive as, and nothing is holding the second button — its throb is the cable's.
    h.state_mut().press(PortRef::new(first, "press"), true);
    // One step for the tick that fires it and the frame that draws the throb.
    h.step();
    let after = (luma_at(&mut h, dot), luma_at(&mut h, button));

    assert!(
        after.0 > before.0 + 8.0,
        "the dot that fired throbs: {} was {}",
        after.0,
        before.0
    );
    assert!(
        after.1 > before.1 + 8.0,
        "and the button of the input it fired into: {} was {}",
        after.1,
        before.1
    );
}

/// How bright the rendered frame is at one point, for a test about a thing lighting up.
fn luma_at(h: &mut Harness<'_, App>, at: egui::Pos2) -> f32 {
    let image = h.render().expect("the harness renders");
    let px = image.get_pixel(at.x.round() as u32, at.y.round() as u32).0;
    0.2126 * f32::from(px[0]) + 0.7152 * f32::from(px[1]) + 0.0722 * f32::from(px[2])
}

/// Hovering a port lights every port a cable carries it to, and the cables themselves.
///
/// silvia's `glowOnHover` without the glow: a blur is several extra strokes a cable a frame
/// and the UI must never make the render miss one. The color lightens, which is what hover
/// already does everywhere else.
#[test]
fn hovering_a_port_lights_the_port_at_the_far_end() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ("📺 Output", "add output"));
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .expect("color into color");
    h.run_steps(2);

    // `.output` alone matches the row's label and the port itself; the port names its type.
    let at = h
        .get_by_label_contains(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.snapshot("port_hover_lights_its_cable");
}

/// **Hovering a cable lights the ports at both of its ends**, which is the other half of
/// hovering a port lighting its cables.
///
/// The two answer the same question from whichever end the hand is on. Lighting only one way
/// round was a gap you met the moment you followed a wire with the pointer rather than from
/// its socket.
#[test]
fn hovering_a_cable_lights_the_ports_at_both_ends() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ("📺 Output", "add output"));
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .expect("color into color");
    h.run_steps(2);

    // The cable's own hit widget, which sits at the curve's midpoint.
    let at = h
        .get_by_label(&format!(
            "cable checkerboard{cb}.output to output{out}.input"
        ))
        .rect()
        .center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.snapshot("cable_hover_lights_its_ports");
}

/// The preference reaches the canvas: ticking **Phi-spaced cable colors** turns it on, and
/// the canvas is drawn with it.
///
/// The colors themselves are `ui::tests` — the walk is arithmetic and belongs where
/// arithmetic is tested. What a kittest can say is that the box exists, that it is off until
/// asked, and that a tick lands in the preferences the canvas reads.
#[test]
fn the_phi_cable_preference_is_off_until_it_is_ticked() {
    let mut h = harness();
    h.step();
    assert!(
        !h.state().preferences().phi_cables,
        "the default look is this editor's own, not silvia's"
    );

    open_preferences(&mut h);
    h.get_by_label_contains("Phi-spaced cable colors").click();
    h.run_steps(2);
    assert!(
        h.state().preferences().phi_cables,
        "the checkbox did not reach the preference"
    );
}

/// **The cable being dragged already wears the color it will keep.**
///
/// silvia's `CursorWire` takes a color and hands that same one to the `Connection` it
/// becomes. A wire that picked a fresh color on the frame it landed would have one frame
/// where the color said nothing — the step is reserved when the drag arms and spent when it
/// connects, so the first cable ever dragged wears step zero and not step one.
#[test]
fn a_dragged_cable_keeps_the_color_it_was_drawn_in() {
    let mut h = harness();
    h.step();
    tick_phi_cables(&mut h);

    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ("📺 Output", "add output"));
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);

    // Dragged port to port, which is the gesture that reserves a step and then spends it.
    let from = h
        .get_by_label_contains(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center();
    let to = h
        .get_by_label_contains(&format!("output{out}.input (varying color input)"))
        .rect()
        .center();
    press_at(&mut h, from);
    // A few frames of travel: a drag is reported on the frame the pointer has crossed the
    // threshold, and one jump from port to port is a press and a release to egui.
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        move_to(&mut h, from + (to - from) * t);
    }
    release_at(&mut h, to);
    h.run_steps(2);

    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "the drag did not connect, so there is no color to have kept"
    );
    assert_eq!(
        h.state().cable_hue(cb, "output", out, "input"),
        Some(0),
        "the cable took a fresh step on landing instead of the one it was drawn in"
    );
}

/// Tick `phi_cables` in the Preferences window and close it again, as a hand does.
fn tick_phi_cables(h: &mut Harness<'_, App>) {
    open_preferences(h);
    h.get_by_label_contains("Phi-spaced cable colors").click();
    h.run_steps(2);
    // Out of the way before the canvas is touched: the window sits over the middle of it,
    // which is exactly where a new node lands. Its own close button, at the top right.
    let window = h.get_by_label("Preferences").rect();
    click_at(h, Pos2::new(window.max.x - 12.0, window.min.y + 12.0));
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Phi-spaced cable colors")
            .is_none(),
        "the Preferences window is still open and over the canvas"
    );
}

/// **A connected port wears its cable's color as a ring of its own shape** — a square round
/// an action port and a diamond round a uniform one — under `phi_cables`.
#[test]
fn phi_cables_outline_each_port_in_its_own_shape() {
    let mut h = wide_harness();
    h.step();
    tick_phi_cables(&mut h);
    let button = add_at(&mut h, "button", Pos2::new(20.0, 40.0));
    let counter = add_at(&mut h, "counter", Pos2::new(270.0, 40.0));
    let slew = add_at(&mut h, "slew", Pos2::new(520.0, 40.0));
    for (from, to) in [
        (
            PortRef::new(button, "trigger"),
            PortRef::new(counter, "increment"),
        ),
        (PortRef::new(counter, "value"), PortRef::new(slew, "input")),
    ] {
        h.state_mut()
            .apply(Command::Connect { from, to })
            .expect("a legal connection");
    }
    h.run_steps(3);
    assert_eq!(
        h.state().cable_hue(button, "trigger", counter, "increment"),
        Some(0)
    );
    assert_eq!(
        h.state().cable_hue(counter, "value", slew, "input"),
        Some(1)
    );
    h.snapshot("phi_cables_outlines");
}

/// **A cable lands on the port that lights under it, and nowhere else.** The hover and the
/// drop ask one question of one shape — the square the port's own widget answers in, which
/// is the rect the accessibility tree reports for it — so a release in its corner lands, and
/// a release a couple of points past its edge does not, however near the dot it is.
#[test]
fn a_cable_lands_where_the_port_answers_the_pointer() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ("📺 Output", "add output"));
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    let from = h
        .get_by_label_contains(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center();
    let square = h
        .get_by_label_contains(&format!("output{out}.input (varying color input)"))
        .rect();
    let drag = |h: &mut Harness<'_, App>, to: Pos2| {
        press_at(h, from);
        for step in 1..=4 {
            let t = step as f32 / 4.0;
            move_to(h, from + (to - from) * t);
        }
        release_at(h, to);
    };

    // Just past the square's right edge: inside the reach a round capture of the same radius
    // had, and outside the port.
    drag(&mut h, Pos2::new(square.max.x + 2.0, square.center().y));
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "a cable let go beside the port landed on it"
    );

    // In the square's own corner: the port answers there, so the cable lands.
    drag(&mut h, square.min + square.size() * 0.1);
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "a cable let go inside the port's square did not land on it"
    );
}

/// Drag a cable from one point to another over a few frames and let it go: a drag is
/// reported on the frame the pointer crosses the threshold, and one jump is a click to egui.
fn drag_cable(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    press_at(h, from);
    for step in 1..=4 {
        let t = step as f32 / 4.0;
        move_to(h, from + (to - from) * t);
    }
    release_at(h, to);
}

/// A node added through the bus at a world position, settled; answers its id.
fn add_at(h: &mut Harness<'_, App>, slug: &'static str, at: Pos2) -> supersilvia::graph::NodeId {
    let workspace = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddNode {
            slug,
            at,
            workspace,
        })
        .expect("in the registry");
    let id = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    h.run_steps(2);
    id
}

/// Move one node so that a point on it, `at` on screen, lands on `to`; settled.
fn move_point_to(h: &mut Harness<'_, App>, node: supersilvia::graph::NodeId, at: Pos2, to: Pos2) {
    let by = (to - at) / h.state().canvas_transform().zoom;
    let pos = h.state().graph().get(node).unwrap().pos;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(node, pos + by)],
        })
        .unwrap();
    h.run_steps(2);
}

/// A checkerboard and an Output, the pair every drop test below drags a cable between:
/// answers their ids.
fn a_source_and_an_output(
    h: &mut Harness<'_, App>,
) -> (supersilvia::graph::NodeId, supersilvia::graph::NodeId) {
    h.step();
    let cb = add_at(h, "checkerboard", Pos2::new(40.0, 200.0));
    let out = add_at(h, "output", Pos2::new(360.0, 120.0));
    (cb, out)
}

/// The center of a checkerboard's output port, from the square its widget answers in.
fn source_output(h: &Harness<'_, App>, cb: supersilvia::graph::NodeId) -> Pos2 {
    h.get_by_label(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center()
}

/// The center of an Output's input port, from the square its widget answers in.
fn output_input(h: &Harness<'_, App>, out: supersilvia::graph::NodeId) -> Pos2 {
    h.get_by_label(&format!("output{out}.input (varying color input)"))
        .rect()
        .center()
}

/// **A cable let go over the tab bar lands on nothing**, though a port lies under it: the
/// canvas is clipped at its top edge, and a port scrolled up past it is out of sight.
#[test]
fn a_cable_let_go_over_the_tab_bar_lands_on_nothing_under_it() {
    let mut h = harness();
    let (cb, out) = a_source_and_an_output(&mut h);
    let from = source_output(&h, cb);
    let port = output_input(&h, out);
    let top = h.state().canvas_origin().y;
    let hidden = Pos2::new(port.x, top - 12.0);
    move_point_to(&mut h, out, port, hidden);
    drag_cable(&mut h, from, hidden);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "a cable let go over the tab bar landed on the port clipped out of sight under it"
    );

    // Back in sight, the same port takes the same drag.
    move_point_to(&mut h, out, hidden, Pos2::new(port.x, top + 40.0));
    let port = output_input(&h, out);
    drag_cable(&mut h, from, port);
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "the port in sight"
    );
}

/// **A cable let go on a window over the canvas lands on nothing**, though a port lies under
/// the window: the Status box here, and the Preferences and MIDI windows by the same test.
#[test]
fn a_cable_let_go_over_the_status_box_lands_on_nothing_under_it() {
    let mut h = wide_harness();
    let (cb, out) = a_source_and_an_output(&mut h);
    h.state_mut().set_show_status_box(true);
    h.run_steps(4);
    let window = h
        .query_all_by_label("Status box")
        .find(|n| n.accesskit_node().role() == egui::accesskit::Role::Window)
        .expect("the Status box is open")
        .rect();
    // The source clear of the window, beside it; the port under the window's middle.
    let from = source_output(&h, cb);
    let beside = Pos2::new(window.max.x + 200.0, window.center().y);
    move_point_to(&mut h, cb, from, beside);
    let from = source_output(&h, cb);
    let port = output_input(&h, out);
    move_point_to(&mut h, out, port, window.center());
    assert!(
        !window.contains(from),
        "the drag starts clear of the window"
    );
    drag_cable(&mut h, from, window.center());
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "a cable let go over the Status box landed on the port under it"
    );

    // With the window gone, the same port takes the same drag.
    h.state_mut().set_show_status_box(false);
    h.run_steps(2);
    let port = output_input(&h, out);
    drag_cable(&mut h, from, port);
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "the port uncovered"
    );
}

/// **A cable let go on a node's body lands on nothing under it.** A node painted later covers
/// an earlier one's port, so the release is on the body the hand can see and not on the port.
#[test]
fn a_cable_let_go_on_a_body_lands_on_nothing_it_covers() {
    let mut h = harness();
    let (cb, out) = a_source_and_an_output(&mut h);
    let from = source_output(&h, cb);
    let port = output_input(&h, out);
    // A later node, its body's middle on the port.
    let cover = add_at(&mut h, "color", Pos2::new(40.0, 360.0));
    let body = h.get_by_label(&format!("color{cover} body")).rect();
    move_point_to(&mut h, cover, body.center(), port);
    assert!(
        h.get_by_label(&format!("color{cover} body"))
            .rect()
            .contains(port),
        "the later node covers the port"
    );
    drag_cable(&mut h, from, port);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "a cable let go on a node's body landed on the port that body covers"
    );

    // Moved off it, the port takes the same drag.
    let body = h.get_by_label(&format!("color{cover} body")).rect();
    move_point_to(&mut h, cover, body.center(), port + egui::vec2(0.0, 300.0));
    let port = output_input(&h, out);
    drag_cable(&mut h, from, port);
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "the port uncovered"
    );
}

/// **A cable from a hidden output row is drawn from the header.** Hiding the uniform numbers
/// takes their rows, and a hidden output that carries a cable gathers on the header's right
/// edge as a collapsed node's outputs do: the cable is drawn from there, and the gathered slot
/// lights, takes a cable let go on it and clears on a right-click like any port. A hidden
/// output with nothing on it has no slot, so an unwired header wears no dot.
#[test]
fn a_cable_from_a_hidden_output_row_is_drawn_from_the_header() {
    let mut h = wide_harness();
    h.step();
    let mi = add_at(&mut h, "maininput", Pos2::new(40.0, 60.0));
    let cb = add_at(&mut h, "checkerboard", Pos2::new(600.0, 80.0));
    let other = add_at(&mut h, "checkerboard", Pos2::new(600.0, 420.0));
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(mi, "level"),
            to: PortRef::new(cb, "frequency"),
        })
        .expect("a uniform number into a number");
    h.run_steps(2);
    h.state_mut()
        .apply(Command::SetOption {
            node: mi,
            key: "uniforms",
            value: "off".to_string(),
        })
        .expect("the tick");
    h.run_steps(3);

    // The wired port is on the header's right edge, at the header's middle; an unwired one
    // that is hidden too has no port at all.
    let level = format!("maininput{mi}.level (uniform number output");
    let header = h.get_by_label(&format!("maininput{mi}")).rect();
    let port = h.get_by_label_contains(&level).rect().center();
    assert!(
        (port.y - header.center().y).abs() < 1.0 && port.x > header.max.x,
        "the hidden port gathers on the header's right edge: {port:?}, header {header:?}"
    );
    assert!(
        h.query_by_label_contains(&format!("maininput{mi}.peak ("))
            .is_none(),
        "a hidden output with nothing on it has no slot"
    );
    let cable = format!("cable maininput{mi}.level to checkerboard{cb}.frequency");
    assert!(h.query_by_label(&cable).is_some(), "the cable is drawn");

    // A cable let go on the gathered slot lands on the hidden output there.
    let from = h
        .get_by_label(&format!(
            "checkerboard{other}.frequency (varying number input)"
        ))
        .rect()
        .center();
    drag_cable(&mut h, from, port);
    assert!(
        h.state()
            .graph()
            .connections()
            .iter()
            .any(|c| c.from == PortRef::new(mi, "level") && c.to.node == other),
        "the gathered slot took the cable"
    );

    // The cable is deleted by the same double-click as any cable.
    double_click(&mut h, &cable);
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "double-clicking the cable deletes it"
    );

    // And a right-click on the gathered slot clears what is left, which takes the dot with it.
    right_click(&mut h, port);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "a right-click on the gathered slot clears its cable"
    );
    h.run_steps(2);
    assert!(
        h.query_by_label_contains(&level).is_none(),
        "with nothing on it, the hidden output has no slot"
    );
}

#[test]
fn every_port_is_in_the_accessibility_tree_and_named() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    // The agent clicks these by name rather than guessing coordinates, and the same
    // labeling is what makes egui_mcp usable. A port the tree cannot see is a bug.
    for label in [
        "checkerboard1.frequency (varying number input)",
        "checkerboard1.color1 (varying color input)",
        "checkerboard1.color2 (varying color input)",
        "checkerboard1.output (varying color output)",
    ] {
        assert!(
            h.query_by_label(label).is_some(),
            "port {label:?} is missing"
        );
    }
    // The header is the drag handle and is a real widget too.
    assert!(h.query_by_label("checkerboard1").is_some());
}

#[test]
fn an_output_nodes_ports_are_named_too() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);

    assert!(
        h.query_by_label("output1.input (varying color input)")
            .is_some()
    );
    assert!(
        h.query_by_label("output1.frame (varying color output)")
            .is_some()
    );
}

#[test]
fn deleting_a_node_goes_through_the_command_bus() {
    let mut h = harness();
    h.step();

    add_node(&mut h, ADD_CHECKERBOARD);
    // Nothing selected, so the verb has no object and is disabled rather than guessing one.
    h.get_by_label("Edit").click();
    h.step();
    assert!(
        h.get_by_label("Delete node Delete")
            .accesskit_node()
            .is_disabled(),
        "a destructive verb with no object is disabled"
    );
    h.get_by_label("Edit").click();
    h.step();

    // Select it, and the same entry acts on it.
    h.get_by_label("checkerboard1").click();
    h.step();
    h.get_by_label("Edit").click();
    h.step();
    h.get_by_label("Delete node Delete").click();
    h.step();

    assert!(h.state().graph().is_empty());
    assert_eq!(h.state().history().len(), 2);
}

/// The Edit menu's Delete says what it deletes: *Delete node* for one, *Delete nodes* for
/// several, each the whole selection in one undo step.
#[test]
fn the_edit_menus_delete_names_one_node_or_several() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_CHECKERBOARD);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();

    h.state_mut().select_only(&ids[..1]);
    h.get_by_label("Edit").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Delete nodes Delete").is_none(),
        "one node is not nodes"
    );
    h.get_by_label("Delete node Delete").click();
    h.step();
    assert_eq!(h.state().graph().len(), 1, "the selected one went");
    assert!(h.state().graph().get(ids[1]).is_some(), "and only it");

    add_node(&mut h, ADD_CHECKERBOARD);
    let both: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut().select_only(&both);
    h.get_by_label("Edit").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Delete node Delete").is_none(),
        "two nodes are nodes"
    );
    h.get_by_label("Delete nodes Delete").click();
    h.step();
    assert!(h.state().graph().is_empty(), "both went");
    assert_eq!(
        h.state().history().len(),
        5,
        "three adds and one step per delete"
    );
}

/// The Edit menu's node verbs are the context menu's, for the hand that looks in a menu bar
/// before it thinks to right-click: same commands, same undo steps, one more door.
///
/// They act on **the selection on the workspace being looked at**. The canvas holds one
/// selection across the project, so a node selected on a workspace that is not open must not
/// be what a menu naming one node acts on.
#[test]
fn the_edit_menus_node_verbs_act_on_the_selection() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).max().unwrap();

    // Nothing selected: every one of them is disabled, not just Delete.
    h.get_by_label("Edit").click();
    h.step();
    for label in [
        "Duplicate Ctrl+D",
        "Collapse",
        "Reset controls",
        "Disconnect all",
    ] {
        assert!(
            h.get_by_label(label).accesskit_node().is_disabled(),
            "{label} with no selection"
        );
    }
    h.get_by_label("Edit").click();
    h.step();

    h.get_by_label("checkerboard1").click();
    h.step();

    // Collapse reads the selection's own state, the way the context menu does.
    assert!(!h.state().graph().get(cb).unwrap().collapsed);
    h.get_by_label("Edit").click();
    h.step();
    h.get_by_label("Collapse").click();
    h.step();
    assert!(h.state().graph().get(cb).unwrap().collapsed);

    // And having collapsed it, the same entry offers the way back.
    h.get_by_label("Edit").click();
    h.step();
    h.get_by_label("Expand").click();
    h.step();
    assert!(!h.state().graph().get(cb).unwrap().collapsed);

    // Through the bus, so both are undo steps: the add, and the two collapses.
    assert_eq!(h.state().history().len(), 3);

    h.get_by_label("Edit").click();
    h.step();
    h.get_by_label("Duplicate Ctrl+D").click();
    h.step();
    assert_eq!(h.state().graph().iter().count(), 2, "duplicated");
}

#[test]
fn a_connected_graph_draws_a_cable() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);

    // Built through the command bus rather than a simulated drag: three apply calls instead
    // of a pointer choreography. The drag itself is exercised by hand through egui_mcp.
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .expect("color into color");
    h.step();

    assert_eq!(h.state().graph().connections().len(), 1);
    h.snapshot("connected_graph");
}

#[test]
fn moving_a_node_does_not_flood_the_history() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();

    for x in 1..=10 {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(id, Pos2::new(x as f32, 0.0))],
            })
            .unwrap();
    }

    // One AddNode plus one coalesced move: a drag is a single undo step, not sixty.
    assert_eq!(h.state().history().len(), 2);
    assert_eq!(h.state().graph().get(id).unwrap().pos, Pos2::new(10.0, 0.0));
}

#[test]
fn the_clock_advances_one_tick_per_frame() {
    let mut h = harness();
    h.step();
    let after_one = h.state().clock().ticks();

    h.run_steps(3);
    assert_eq!(
        h.state().clock().ticks(),
        after_one + 3,
        "the clock must tick exactly once per frame, from exactly one place",
    );
}

#[test]
fn clicking_a_color_swatch_opens_the_picker_and_it_stays_open() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    assert!(h.state().open_control().is_none());

    // The swatch's accessible name carries the current value.
    h.get_by_label_contains("checkerboard1.color1 #").click();
    h.step();
    assert!(
        h.state().open_control().is_some(),
        "the picker must survive the click that opened it — the dismissal check sees that \
         same click, so it has to ignore the frame the popup went up",
    );

    // And it must still be open on the next frame, with no further input.
    h.step();
    assert!(h.state().open_control().is_some());
}

#[test]
fn clicking_the_same_swatch_again_closes_the_picker() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    h.get_by_label_contains("checkerboard1.color1 #").click();
    h.step();
    assert!(h.state().open_control().is_some());

    h.get_by_label_contains("checkerboard1.color1 #").click();
    h.step();
    assert!(
        h.state().open_control().is_none(),
        "a second click toggles it shut"
    );
}

#[test]
fn a_connected_color_input_does_not_open_a_picker() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).next().unwrap();

    // Feed color1 from the node's own frequency? No — use a second checkerboard's output.
    add_node(&mut h, ADD_CHECKERBOARD);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let other = *ids.iter().find(|id| **id != cb).unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(other, "output"),
            to: PortRef::new(cb, "color1"),
        })
        .expect("color into color");
    h.step();

    // Fed a *picture*, so the swatch has no one color to show and says so — see
    // `a_control_fed_by_a_varying_value_says_varying`. Either way it is inert.
    h.get_by_label_contains("checkerboard1.color1 varying")
        .click();
    h.step();
    assert!(
        h.state().open_control().is_none(),
        "a connection overrides the control, so its swatch is inert",
    );
}

/// A swatch on a color input fed by a `color` node meters the color arriving down the cable,
/// exactly as a number knob fed by a uniform number meters the number. The swatch's own
/// stored color is a different one, so the hex on the row can only have come from the cable.
#[test]
fn a_swatch_fed_by_a_color_node_shows_the_arriving_color() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    add_node(&mut h, ADD_COLOR);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let color = *ids.iter().find(|id| **id != cb).unwrap();

    h.state_mut()
        .apply(Command::SetControl {
            node: color,
            key: "color",
            value: supersilvia::graph::ControlValue::Color([0.0, 0.5, 1.0, 1.0]),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(color, "output"),
            to: PortRef::new(cb, "color1"),
        })
        .expect("a uniform color into a varying color");

    // The `color` node's tick has to run before there is anything to meter, and the canvas
    // is drawn from the map that tick fills.
    h.run_steps(2);
    let row = h.get_by_label_contains("checkerboard1.color1 #");
    let label = row.accesskit_node().label().unwrap_or_default();
    assert!(
        label.contains("#0080ffff"),
        "the arriving color, not the swatch underneath: {label:?}"
    );

    row.click();
    h.step();
    assert!(
        h.state().open_control().is_none(),
        "and still inert: a connection overrides the swatch"
    );
}

/// Click into the hex field of an open color popup and select all of it, so typed text
/// replaces the whole string rather than landing beside it.
///
/// The field deliberately does not take focus when the popup opens, so reaching it is a
/// gesture the test has to make, exactly as a user does.
fn focus_hex(h: &mut Harness<'_, App>) {
    h.get_by_role(egui::accesskit::Role::TextInput).focus();
    h.step();
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.step();
}

/// Type into the hex field of an open color popup and commit it.
///
/// The second `step` after `Enter` is the frame the command bus applies the edit on.
fn type_hex(h: &mut Harness<'_, App>, text: &str) {
    focus_hex(h);
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_string()));
    h.step();
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.step();
}

/// The color a node holds on `color1`, if it holds one.
fn color(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> Option<[f32; 4]> {
    match h
        .state()
        .graph()
        .get(id)
        .and_then(|n| n.controls.get("color1"))
    {
        Some(supersilvia::graph::ControlValue::Color(c)) => Some(*c),
        _ => None,
    }
}

/// One checkerboard with its color popup open on `color1`.
///
/// The popup, and the hex field inside it, are drawn on the same frame the click that opens
/// it is processed — unlike the s-number's typed entry, there is no separate control to
/// switch into.
fn checkerboard_with_the_picker_open(h: &mut Harness<'_, App>) -> supersilvia::graph::NodeId {
    h.step();
    add_node(h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    h.get_by_label_contains("checkerboard1.color1 #").click();
    h.step();
    id
}

/// The one thing silvia's popup has that egui's picker alone does not: a color typed, pasted
/// or matched to a brand rather than dragged to.
#[test]
fn typing_a_hex_value_into_the_picker_sets_the_color() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);
    type_hex(&mut h, "ff0000ff");
    assert_eq!(color(&h, id), Some([1.0, 0.0, 0.0, 1.0]));
}

/// Anything that does not parse leaves the stored color alone, the same contract the
/// s-number's typed entry has.
#[test]
fn an_unparseable_hex_value_leaves_the_color_alone() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);
    let before = color(&h, id);
    type_hex(&mut h, "not a color");
    assert_eq!(color(&h, id), before);
}

/// The field defers to the picker: opening a popup is a gesture towards *picking*, and a
/// field that took the caret would take the first keystroke of a shortcut with it.
#[test]
fn the_hex_field_does_not_take_focus_when_the_picker_opens() {
    let mut h = harness();
    checkerboard_with_the_picker_open(&mut h);
    assert!(
        !h.get_by_role(egui::accesskit::Role::TextInput).is_focused(),
        "the hex field must not steal focus from the picker it sits under",
    );
}

/// The bug this guards: a blur committed whatever string the field was showing, whether or
/// not anybody had typed it. Clicking into the picker square is a blur, and on that frame the
/// string is a frame behind the color the click just picked — so every pick was immediately
/// overwritten by the color it replaced.
#[test]
fn blurring_a_hex_field_nobody_typed_into_is_not_an_edit() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);
    let before = color(&h, id);
    let steps = h.state().undo_len();

    focus_hex(&mut h);
    // `Enter` surrenders a singleline's focus, which is the same blur a click into the
    // picker square makes — without a pick landing on top of it to confuse the reading.
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.step();

    assert_eq!(color(&h, id), before);
    assert_eq!(
        h.state().undo_len(),
        steps,
        "committing the string the field was already showing opened an undo step that \
         restores nothing",
    );
}

/// Press, release and settle the primary button at one point, as its own frames.
///
/// The move is a frame of its own: egui hit-tests a press against where the pointer already
/// is, so a move and a press in one frame land on nothing.
fn click_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.step();
}

/// The s-number's rect for one control. The port row carries the same `{slug}{id}.{key}`
/// with its type on the end, and the control carries it with its value, so the control is the
/// match that does not say "input".
fn number_rect(h: &Harness<'_, App>, name: &str) -> egui::Rect {
    h.get_all_by_label_contains(name)
        .find(|n| {
            !n.accesskit_node()
                .label()
                .is_some_and(|l| l.contains("input"))
        })
        .expect("the s-number")
        .rect()
}

/// Press, release and settle with `Alt` held, which is the MIDI learn gesture.
///
/// **`ModifiersChanged`, not the modifiers on the press.** `InputState::modifiers` — what
/// `ui.ctx().input(|i| i.modifiers)` reads — is only updated by that event; the `modifiers`
/// a `PointerButton` carries is read by the shortcut matcher and by nothing else. A window
/// manager sends both, so a press alone looks right and does nothing.
fn alt_click_at(h: &mut Harness<'_, App>, at: Pos2) {
    let alt = egui::Modifiers {
        alt: true,
        ..Default::default()
    };
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(alt));
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: alt,
        });
        h.step();
    }
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.step();
}

/// A point in one of the picker's instruments, as a fraction across and down it.
///
/// By name, not by geometry: the square, the hue bar and the alpha bar each report themselves
/// to the accessibility tree, which egui's own picker never did. Fractions rather than insets
/// from an edge, because the rect a widget answers to is a little larger than the one it
/// paints — each instrument reaches into half the gap around it.
fn within(h: &Harness<'_, App>, what: &str, across: f32, down: f32) -> Pos2 {
    let r = h.get_by_label(what).rect();
    Pos2::new(
        egui::lerp(r.left()..=r.right(), across),
        egui::lerp(r.top()..=r.bottom(), down),
    )
}

/// A point inside the open popup's saturation/value square.
fn inside_the_picker_square(h: &Harness<'_, App>) -> Pos2 {
    h.get_by_label("saturation and value").rect().center()
}

/// The gesture the whole popup exists for: a color picked out of the saturation/value square
/// has to still be there on the following frames.
///
/// This is what the stale hex commit broke. The field lost focus to the click, and the string
/// it committed on the way out was a frame behind the color the click had just picked — so
/// the old color went straight back and the pick never survived the gesture that made it.
#[test]
fn a_color_picked_out_of_the_square_sticks() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);
    let before = color(&h, id).expect("the checkerboard holds a color");

    let at = inside_the_picker_square(&h);
    click_at(&mut h, at);

    let picked = color(&h, id).expect("still a color");
    assert_ne!(picked, before, "the click in the square picked nothing");
    h.step();
    h.step();
    assert_eq!(
        color(&h, id),
        Some(picked),
        "the picked color reverted on the frames after the click",
    );
}

/// The bars are the other two axes of the same instrument, so they have to move the color.
#[test]
fn the_hue_and_alpha_bars_move_the_color() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);

    // Saturated first: hue is an angle a color has none of while it is white, so turning the
    // hue bar on the default color would quite correctly change nothing.
    let at = within(&h, "saturation and value", 0.9, 0.1);
    click_at(&mut h, at);
    let saturated = color(&h, id).expect("a color");
    assert_ne!(saturated, [1.0, 1.0, 1.0, 1.0], "the square moved nothing");

    let at = within(&h, "hue", 0.5, 0.4);
    click_at(&mut h, at);
    assert_ne!(color(&h, id), Some(saturated), "the hue bar moved nothing");

    let at = within(&h, "alpha", 0.5, 0.5);
    click_at(&mut h, at);
    let faded = color(&h, id).expect("a color");
    assert!(faded[3] < 0.9, "the alpha bar left the color opaque");
}

/// The hue a color is at is not always in the color: black is black at every hue, so
/// dragging to the bottom of the square and back would come back red if the picker had
/// nowhere to keep the angle it was working at.
#[test]
fn a_hue_survives_a_trip_through_black() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);

    let bright = within(&h, "saturation and value", 0.9, 0.1);
    click_at(&mut h, bright);
    let at = within(&h, "hue", 0.5, 0.4);
    click_at(&mut h, at);
    let hued = color(&h, id).expect("a color");
    assert_ne!(hued[0], hued[2], "the hue bar left the color on red");

    // The bottom-left of the square is black whatever the hue, which is exactly where the
    // angle has nowhere left to live in the color itself.
    let at = within(&h, "saturation and value", 0.06, 0.94);
    click_at(&mut h, at);
    let dark = color(&h, id).expect("a color");
    assert!(
        dark[..3].iter().all(|c| *c < 0.1),
        "expected near-black, got {dark:?}"
    );

    click_at(&mut h, bright);
    assert_eq!(
        color(&h, id),
        Some(hued),
        "the trip through black lost the hue the bar had been set to",
    );
}

/// A tap is as good a way to set a hue as a drag, so it must not dismiss the popup it lands
/// in. An `Area` is movable by default and a movable one senses drags, not clicks — which is
/// exactly why dragging kept the picker open and clicking closed it.
#[test]
fn a_click_inside_the_picker_does_not_dismiss_it() {
    let mut h = harness();
    checkerboard_with_the_picker_open(&mut h);

    let at = inside_the_picker_square(&h);
    click_at(&mut h, at);

    assert!(
        h.state().open_control().is_some(),
        "a tap in the square dismissed the picker it was aimed at",
    );
}

/// The popup's own padding is part of the popup. A click that falls through it reaches the
/// node underneath — in the worst case another swatch, which silently re-aims the picker at a
/// different control mid-edit.
#[test]
fn a_click_on_the_pickers_padding_stays_in_the_picker() {
    let mut h = harness();
    let id = checkerboard_with_the_picker_open(&mut h);
    let open = h.state().open_control();
    let before = color(&h, id);

    // The numbers column is shorter than the square beside it, so the popup's own background
    // shows below the last channel — exactly the dead space a click used to fall through.
    let square = h.get_by_label("saturation and value").rect();
    let alpha = h.get_by_label("alpha").rect();
    click_at(
        &mut h,
        Pos2::new(alpha.right() + 40.0, square.bottom() - 6.0),
    );

    assert_eq!(
        h.state().open_control(),
        open,
        "the click went through the popup to whatever was underneath it",
    );
    assert_eq!(color(&h, id), before, "and it moved a color on the way");
}

#[test]
fn off_screen_nodes_are_not_drawn() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    assert_eq!(h.state().canvas_drawn(), 1);

    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    // Well outside any plausible viewport.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(50_000.0, 50_000.0))],
        })
        .unwrap();
    h.step();

    assert_eq!(
        h.state().canvas_drawn(),
        0,
        "a node nobody can see must cost neither drawing nor hit-testing",
    );
    // It is still in the graph, and still compiles.
    assert_eq!(h.state().graph().len(), 1);
}

#[test]
fn a_culled_node_leaves_no_widgets_behind() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    assert!(
        h.query_by_label("checkerboard1.frequency (varying number input)")
            .is_some()
    );

    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(50_000.0, 50_000.0))],
        })
        .unwrap();
    h.step();

    assert!(
        h.query_by_label("checkerboard1.frequency (varying number input)")
            .is_none(),
        "culled means gone from the accessibility tree too, not merely unpainted",
    );
}

/// egui's `Modifiers::matches_logically` only rejects a shortcut when the *pattern* needs a
/// modifier that is not held — it does not reject extra modifiers that are. So the pattern
/// `Ctrl+Z` also matches a `Ctrl+Shift+Z` press, and whichever shortcut is consumed first
/// wins. Consuming undo first made redo step backwards instead of forwards.
///
/// Invisible to every other test: the model was right, the binding was wrong.
/// `Ctrl+S` is Project ▸ Save: the folder is written and the edits are no longer unsaved.
#[test]
fn ctrl_s_saves_the_project() {
    let dir = std::env::temp_dir().join(format!("ssw-ui-ctrl-s-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let root = dir.join("friday");
    let mut h = harness();
    h.step();
    h.state_mut().new_project(root.clone());
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    assert!(h.state().dirty());

    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
    h.step();
    assert!(!h.state().dirty(), "Ctrl+S saved");
    let (_, graph, _) = supersilvia::project::Project::open(root).expect("a project on disk");
    assert_eq!(graph.len(), 1, "with the node in it");
    std::fs::remove_dir_all(&dir).ok();
}

/// `Ctrl+O` is Project ▸ Open project…, and asks about unsaved edits first as the entry does.
#[test]
fn ctrl_o_opens_behind_the_unsaved_question() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    assert!(h.state().dirty());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    h.run_steps(2);
    assert!(h.query_by_label("Discard").is_some(), "Ctrl+O asked first");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.query_by_label("Discard").is_none());
    assert_eq!(h.state().graph().len(), 1, "and Cancel changed nothing");
}

#[test]
fn ctrl_shift_z_redoes_rather_than_undoing_again() {
    use egui::Modifiers;

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    assert_eq!(h.state().graph().len(), 2);

    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.step();
    assert_eq!(h.state().graph().len(), 1, "Ctrl+Z undoes");

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::Z);
    h.step();
    assert_eq!(
        h.state().graph().len(),
        2,
        "Ctrl+Shift+Z must redo, not undo a second time"
    );
}

#[test]
fn ctrl_y_also_redoes() {
    use egui::Modifiers;

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Z);
    h.step();
    assert_eq!(h.state().graph().len(), 0);

    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::Y);
    h.step();
    assert_eq!(h.state().graph().len(), 1);
}

/// Press the clipboard chord the way a window delivers it.
///
/// **Not a key press.** `egui-winit` recognizes `Ctrl+C`, `Ctrl+X` and `Ctrl+V` itself and
/// pushes `Event::Copy`, `Event::Cut` or `Event::Paste` *instead of* the key event — it
/// returns before the `Event::Key` every other shortcut is matched against ever reaches the
/// queue. A test that synthesizes the key press is therefore testing a path no window takes,
/// which is how the first cut of this shipped broken with a passing test.
fn clipboard_event(h: &mut Harness<'_, App>, event: egui::Event) {
    h.input_mut().events.push(event);
}

/// **The clipboard keys are guarded where `Ctrl+Z` is not.**
///
/// Undo is consumed above whatever holds the keyboard, because `Ctrl+Z` in a text field is
/// an undo of the graph either way. `Ctrl+V` is not: every `TextEdit` in the app answers it
/// itself, and none of them guards itself — so a canvas paste taken from under a focused
/// field would take the clipboard away from every field there is.
#[test]
fn ctrl_v_with_a_text_field_focused_does_not_paste_into_the_graph() {
    use egui::Modifiers;

    let mut h = harness();
    h.step();
    // A note is a node whose whole body is a text field, so focusing one is one click. It
    // is added first so that it lands where the canvas is showing.
    add_node(&mut h, ("Control", "add note"));
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).max().unwrap();
    h.state_mut().select_only(&[cb]);
    h.step();
    clipboard_event(&mut h, egui::Event::Copy);
    h.step();
    assert!(h.state().can_paste(), "Ctrl+C copied nothing");

    h.get_by_label_contains("note1.text").click();
    h.run_steps(2);
    let before = h.state().graph().len();

    clipboard_event(
        &mut h,
        egui::Event::Paste("pasted into the note".to_owned()),
    );
    h.run_steps(2);
    assert_eq!(
        h.state().graph().len(),
        before,
        "Ctrl+V was taken from the field the person was typing in"
    );
    // Stronger than "the graph did not change": the text actually arrived in the field. A
    // guard that swallowed the event instead of leaving it for the `TextEdit` would pass the
    // assertion above and still have broken every text field in the app.
    assert!(
        h.get_by_label_contains("note1.text")
            .accesskit_node()
            .value()
            .unwrap_or_default()
            .contains("pasted into the note"),
        "the field never got the paste"
    );

    // And with the field let go, the very same press pastes — so it is the guard that
    // stopped it and not a paste that never worked.
    click_with(&mut h, Pos2::new(340.0, 500.0), Modifiers::NONE);
    h.run_steps(2);
    clipboard_event(
        &mut h,
        egui::Event::Paste("1 node: checkerboard".to_owned()),
    );
    h.run_steps(2);
    assert_eq!(
        h.state().graph().len(),
        before + 1,
        "Ctrl+V on the canvas should paste the copy"
    );
}

/// A paste that named no point lands where the copy was **in the window** — the same spot on
/// screen, whatever the view has done since. Pan away and paste, and the nodes come back
/// where they visually were, which is a different world coordinate by exactly the pan.
#[test]
fn an_unaimed_paste_lands_where_the_copy_was_in_the_window() {
    use egui::Modifiers;

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).max().unwrap();
    let was = h.state().graph().get(cb).expect("the original").pos;
    h.get_by_label("checkerboard1").click();
    h.step();
    clipboard_event(&mut h, egui::Event::Copy);
    h.step();

    let before = h.state().canvas_transform().pan;
    drag_with(
        &mut h,
        Pos2::new(300.0, 480.0),
        Pos2::new(370.0, 520.0),
        Modifiers::NONE,
    );
    let moved = h.state().canvas_transform().pan - before;
    assert!(moved != egui::Vec2::ZERO, "the drag did not pan the view");

    clipboard_event(
        &mut h,
        egui::Event::Paste("1 node: checkerboard".to_owned()),
    );
    h.run_steps(2);

    let copy = *h.state().canvas_selection().first().expect("the copy");
    let landed = h.state().graph().get(copy).expect("the copy").pos;
    let expected = was - moved / h.state().canvas_transform().zoom;
    assert!(
        (landed - expected).length() < 0.5,
        "the paste landed at {landed:?}, not back under the pointer at {expected:?}"
    );
}

/// Copy and Cut act on the selection, so they are disabled without one. Paste does not: it
/// wants something on the clipboard and a workspace to put it on.
#[test]
fn the_edit_menus_clipboard_entries_follow_the_selection_and_the_clipboard() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    h.get_by_label("Edit").click();
    h.step();
    // By what the label contains: an entry with a shortcut carries it in its own name.
    for label in ["Copy", "Cut", "Paste"] {
        assert!(
            h.get_by_label_contains(label)
                .accesskit_node()
                .is_disabled(),
            "{label} with nothing selected and nothing copied"
        );
    }
    h.get_by_label("Edit").click();
    h.step();

    h.get_by_label("checkerboard1").click();
    h.run_steps(2);
    assert_eq!(
        h.state().canvas_selection().len(),
        1,
        "the node is selected"
    );
    h.get_by_label("Edit").click();
    h.run_steps(2);
    assert!(
        !h.get_by_label_contains("Copy")
            .accesskit_node()
            .is_disabled(),
        "Copy with a selection"
    );
    assert!(
        h.get_by_label_contains("Paste")
            .accesskit_node()
            .is_disabled(),
        "Paste before anything has been copied"
    );
    h.get_by_label_contains("Copy").click();
    h.run_steps(2);

    // Copying is not an edit, so the history is still just the add.
    assert_eq!(h.state().history().len(), 1, "a copy reached the bus");

    h.get_by_label("Edit").click();
    h.step();
    h.get_by_label_contains("Paste").click();
    h.run_steps(2);
    assert_eq!(
        h.state().graph().len(),
        2,
        "the menu's Paste planted nothing"
    );
    assert_eq!(h.state().history().len(), 2, "and it is one undo step");
}

/// A cable is clickable, so a cable must be in the accessibility tree.
///
/// This is the module's own rule applied to the canvas's most-cited gesture. It is also
/// what lets the agent-driven layer delete a specific cable by name instead of guessing
/// where the curve runs.
#[test]
fn a_cable_is_in_the_accessibility_tree_and_deletable_by_name() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();

    let label = format!("cable checkerboard{cb}.output to output{out}.input");
    assert!(
        h.query_by_label(&label).is_some(),
        "the cable is not in the tree as {label:?}"
    );

    // One click is not the gesture: near a bundle of cables it would delete one on the way
    // to doing something else.
    h.get_by_label(&label).click();
    h.step();
    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "a single click leaves the cable alone"
    );

    double_click(&mut h, &label);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "double-clicking the cable's widget deletes that edge"
    );
}

/// silvia's one substitute-less affordance: right-click a port and every connection on it
/// goes, whichever side of the cable that port is. Unlike the double-click on the cable
/// itself, the port is an exact target, so one click is enough.
#[test]
fn right_clicking_a_port_clears_its_connections() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();
    assert_eq!(h.state().graph().connections().len(), 1);

    // The output side: nothing on the canvas could clear this in one gesture before —
    // `Disconnect` only ever reached a port from its input.
    let at = h
        .get_by_label(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center();
    right_click(&mut h, at);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "the cable leaving the output is gone"
    );
}

/// Several sources into one action input — the case the proposal named — go in one click and
/// one undo step, not one double-click per cable.
#[test]
fn right_clicking_an_input_clears_every_source_in_one_step() {
    let mut h = harness();
    h.step();
    for _ in 0..3 {
        add_node(&mut h, ("Control", "add button"));
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (a, b, c) = (ids[0], ids[1], ids[2]);
    // The Nodes menu staggers each drop rightward, which walks the third node out past the
    // canvas panel's own width in this window — well within any plausible *world*, but the
    // cabling below only cares about the graph, and the click after it only cares that `c`
    // is somewhere the pointer can reach.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(c, Pos2::new(48.0, 300.0))],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(a, "trigger"),
            to: PortRef::new(c, "press"),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(b, "trigger"),
            to: PortRef::new(c, "press"),
        })
        .unwrap();
    h.step();
    assert_eq!(h.state().graph().connections().len(), 2);
    let before = h.state().history().len();

    let at = h
        .get_by_label(&format!("button{c}.press (action input)"))
        .rect()
        .center();
    right_click(&mut h, at);
    assert_eq!(
        h.state().graph().connections().len(),
        0,
        "both sources are gone"
    );
    assert_eq!(
        h.state().history().len(),
        before + 1,
        "one right-click, one undo step, however many edges it carried"
    );
}

/// A right-click on a port with nothing connected is refused by the command bus rather than
/// answered by the canvas: nothing here has to know in advance whether there is anything to
/// clear.
#[test]
fn right_clicking_a_bare_port_does_nothing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().next().expect("one node").0;
    let before = h.state().history().len();

    let at = h
        .get_by_label(&format!("checkerboard{cb}.output (varying color output)"))
        .rect()
        .center();
    right_click(&mut h, at);
    assert_eq!(h.state().graph().connections().len(), 0);
    assert_eq!(
        h.state().history().len(),
        before,
        "a refused command leaves no undo step"
    );
}

// ---------------------------------------------------------------- linear mode

/// Put the pointer somewhere and scroll it, as one device or the other.
fn scroll_at(h: &mut Harness<'_, App>, at: Pos2, unit: egui::MouseWheelUnit, delta: egui::Vec2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit,
        delta,
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    });
    h.step();
}

/// Put the pointer over the canvas and scroll it, as one device or the other.
fn scroll(h: &mut Harness<'_, App>, unit: egui::MouseWheelUnit, delta: egui::Vec2) {
    scroll_at(h, Pos2::new(400.0, 400.0), unit, delta);
}

/// A mouse wheel: one axis, reported in lines.
fn wheel(h: &mut Harness<'_, App>, notches: f32) {
    scroll(h, egui::MouseWheelUnit::Line, egui::Vec2::new(0.0, notches));
}

/// A trackpad: two axes, reported in points.
fn trackpad(h: &mut Harness<'_, App>, delta: egui::Vec2) {
    scroll(h, egui::MouseWheelUnit::Point, delta);
}

/// The active workspace's layout mode.
fn layout_of(h: &Harness<'_, App>) -> LayoutMode {
    let graph = h.state().graph();
    graph.layout_of(
        h.state()
            .active_workspace()
            .expect("a workspace is showing"),
    )
}

fn set_layout(h: &mut Harness<'_, App>, label: &str) {
    h.get_by_label("Workspace").click();
    h.step();
    h.get_by_label_contains("Layout").click();
    h.step();
    h.get_by_label(label).click();
    h.step();
}

/// Auto-arrange lives under Workspace: arranging is an edit of one canvas.
fn auto_arrange(h: &mut Harness<'_, App>) {
    h.get_by_label("Workspace").click();
    h.step();
    h.get_by_label("Auto-arrange").click();
    h.step();
}

#[test]
fn the_menu_switches_the_workspace_between_canvas_and_linear() {
    let mut h = harness();
    h.step();
    assert_eq!(layout_of(&h), LayoutMode::Canvas);

    set_layout(&mut h, "Linear");
    assert_eq!(layout_of(&h), LayoutMode::Linear);

    // Layout is document data, so the switch is undoable like a move.
    assert!(h.state().can_undo());
    h.state_mut().undo();
    h.step();
    assert_eq!(layout_of(&h), LayoutMode::Canvas);
}

/// The whole point of the mode: one axis, one scale.
#[test]
fn the_wheel_scrolls_along_the_strip_in_linear_and_zooms_on_the_plane() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    wheel(&mut h, -2.0);
    assert!(
        h.state().canvas_transform().zoom < 1.0,
        "the plane zooms with the wheel"
    );

    set_layout(&mut h, "Linear");
    h.step();
    let before = h.state().canvas_transform();
    assert_eq!(before.zoom, 1.0, "the strip is pinned at one scale");

    wheel(&mut h, -2.0);
    assert_eq!(
        h.state().canvas_transform().zoom,
        1.0,
        "and the wheel never moves it"
    );
}

/// A wheel notch on a graph wide enough to scroll actually moves the view.
#[test]
fn the_wheel_moves_the_view_along_a_graph_wider_than_the_window() {
    let mut h = harness();
    h.step();
    for _ in 0..3 {
        add_node(&mut h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 2000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.step();

    let before = h.state().canvas_transform().pan.x;
    wheel(&mut h, -3.0);
    assert!(
        h.state().canvas_transform().pan.x < before,
        "a notch moves the strip along"
    );
}

/// How far right the strip can be scrolled: wheel it to the end and read where it stopped.
fn far_end(h: &mut Harness<'_, App>) -> f32 {
    for _ in 0..30 {
        wheel(h, -20.0);
    }
    h.run_steps(60);
    h.state().canvas_transform().pan.x
}

/// Every button in the MIDI window does what it says: Rescan, Cancel on a learn, the monitor
/// and its Clear, a mapping's node link and the mixer's, its `✕`, and Clear all. The node link
/// goes to the open tab the node is on rather than opening the closed one it was first on.
#[test]
fn the_midi_windows_buttons() {
    use supersilvia::midi::{Kind, Message, Target};

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let node = h.state().graph().iter().next().expect("the node").0;
    let first = h.state().graph().default_workspace();
    let port = PortRef::new(node, "frequency");
    // A second workspace showing the node, and the first one closed: the node's link has an
    // open tab to go to and a closed one it must not open.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::T);
    h.step();
    let second = *h.state().open_workspaces().iter().max().unwrap();
    h.state_mut()
        .apply(Command::ShowOn {
            nodes: vec![node],
            workspace: second,
        })
        .unwrap();
    h.state_mut().close_workspace(first);
    h.state_mut().activate(Active::Project);
    h.step();
    // The mixer folded, for its link to unfold.
    let bar = h.get_by_label("Hide Main Mixer").rect();
    click_at(&mut h, egui::pos2(bar.left() + 8.0, bar.center().y));
    assert!(h.state().preferences().mixer_collapsed);

    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label("MIDI…").click();
    h.run_steps(2);
    // Whatever this machine's sequencer holds, a rescan leaves the window up.
    h.get_by_label("Rescan").click();
    h.run_steps(2);
    assert!(h.query_by_label("Rescan").is_some());

    let bind = |h: &mut Harness<'_, App>, target: Target, cc: u8| {
        h.state_mut().learn_midi(target);
        h.step();
        h.state_mut().apply_midi(Message {
            channel: 0,
            kind: Kind::Control { cc, value: 64 },
        });
        h.run_steps(2);
    };

    // Cancel on a learn waiting for a message.
    h.state_mut().learn_midi(port);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Waiting for a message").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(h.state().midi_learning(), None, "Cancel stopped the learn");

    // The monitor, and its Clear.
    h.get_by_label("Monitor").click();
    h.run_steps(2);
    assert!(h.query_by_label("Nothing yet — turn something").is_some());
    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 5, value: 9 },
    });
    h.run_steps(2);
    assert!(h.query_by_label("Nothing yet — turn something").is_none());
    h.get_by_label("Clear").click();
    h.run_steps(2);
    assert!(h.query_by_label("Nothing yet — turn something").is_some());
    h.get_by_label("Monitor").click();
    h.run_steps(2);
    assert!(h.query_by_label("Not watching").is_some());

    // One mapping, and its ✕.
    bind(&mut h, port.into(), 21);
    assert!(h.state().midi_trigger_of(port).is_some(), "bound");
    h.get_by_label("unbind").click();
    h.run_steps(2);
    assert!(
        h.state().midi_trigger_of(port).is_none(),
        "the ✕ unbound it"
    );

    // Two, the links on them, and Clear all.
    bind(&mut h, port.into(), 21);
    bind(&mut h, Target::Balance, 22);
    h.get_by_label("\u{1f39a} Main Mixer").click();
    h.run_steps(2);
    assert!(
        !h.state().preferences().mixer_collapsed,
        "the mixer unfolded"
    );
    h.get_by_label_contains("Checkerboard1").click();
    h.run_steps(2);
    assert_eq!(
        h.state().project().session().active,
        Active::Workspace(second),
        "the link went to the open tab"
    );
    assert!(
        !h.state().open_workspaces().contains(&first),
        "and opened no other"
    );
    h.get_by_label("Clear all").click();
    h.run_steps(2);
    assert!(h.state().midi_trigger_of(port).is_none());
    assert!(h.state().midi_trigger_of(Target::Balance).is_none());
    assert!(h.query_by_label_contains("Nothing bound").is_some());
}

/// `Alt` + click on a control leaves the MIDI window shut, marks the control as waiting, and
/// the next message binds it.
///
/// The whole gesture without a device: `App::apply_midi` is what the reader thread calls with
/// a message, so a test calls it with one.
#[test]
fn alt_clicking_a_control_learns_it() {
    use supersilvia::midi::{Kind, Message};

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (node, _) = h.state().graph().iter().next().expect("the node");
    let port = PortRef::new(node, "frequency");
    let name = format!("checkerboard{node}.frequency");

    assert!(
        h.query_by_label("MIDI").is_none(),
        "the window is shut to begin with"
    );

    // The s-number's own rect. The port row carries the same name with
    // `(varying number input)` on the end of it, so the control is the match that does not.
    let at = number_rect(&h, &name).center();
    alt_click_at(&mut h, at);
    h.step();

    assert_eq!(
        h.state().midi_learning(),
        Some(port.into()),
        "the control is waiting for a message"
    );
    assert!(
        h.query_by_label("MIDI").is_none(),
        "and the window stayed shut"
    );
    let marked = h
        .get_by_label_contains("(learning MIDI)")
        .accesskit_node()
        .label()
        .unwrap_or_default();
    assert!(
        marked.starts_with(&format!("{name} ")),
        "the control says it is waiting: {marked}"
    );

    h.state_mut().apply_midi(Message {
        channel: 2,
        kind: Kind::Control { cc: 21, value: 0 },
    });
    h.run_steps(2);
    assert_eq!(h.state().midi_learning(), None, "learning is over");
    assert_eq!(
        h.state().midi_trigger_of(port),
        Some(supersilvia::midi::Trigger::Control { channel: 2, cc: 21 }),
        "and the knob drives it"
    );
    assert!(
        h.query_by_label_contains("(learning MIDI)").is_none(),
        "and the control stopped saying it is waiting"
    );
    // The mark the control now wears says so, and is named. Channels are counted from one,
    // as a desk counts them.
    assert!(
        h.query_by_label_contains(&format!("checkerboard{node} frequency bound to CC 21 ch 3"))
            .is_some(),
        "the mark names what drives the control"
    );
}

/// `Escape` stops a control waiting for a MIDI message: the mark goes, the next message binds
/// nothing, and the key does nothing else on that frame — the node browser, which `Escape`
/// otherwise closes, stays open.
#[test]
fn escape_stops_a_learn_and_binds_nothing() {
    use supersilvia::midi::{Kind, Message};

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (node, _) = h.state().graph().iter().next().expect("the node");
    let port = PortRef::new(node, "frequency");

    h.state_mut().learn_midi(port);
    h.step();
    assert!(h.query_by_label_contains("(learning MIDI)").is_some());
    h.key_press(egui::Key::Slash);
    h.run_steps(2);
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "the browser is open"
    );

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().midi_learning(), None, "Escape stopped the learn");
    assert!(
        h.query_by_label_contains("(learning MIDI)").is_none(),
        "and the mark went with it"
    );
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "and the key did not also close the browser"
    );

    // With nothing waiting, `Escape` is the browser's again.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label("add checkerboard").is_none());

    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 7, value: 0 },
    });
    h.run_steps(2);
    assert_eq!(h.state().midi_trigger_of(port), None, "nothing was bound");
}

/// Learning a bound control replaces its binding: the old one goes the moment the learn is
/// armed, so an `Escape` leaves the control unbound.
#[test]
fn learning_a_bound_control_forgets_its_binding_at_once() {
    use supersilvia::midi::Trigger;

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (node, _) = h.state().graph().iter().next().expect("the node");
    let port = PortRef::new(node, "frequency");
    let old = Trigger::Control { channel: 0, cc: 7 };
    h.state_mut().bind_midi_unchecked(old, port);
    h.step();
    assert_eq!(h.state().midi_trigger_of(port), Some(old));

    h.state_mut().learn_midi(port);
    h.step();
    assert_eq!(
        h.state().midi_trigger_of(port),
        None,
        "the old binding went as the learn was armed"
    );

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(h.state().midi_learning(), None);
    assert_eq!(
        h.state().midi_trigger_of(port),
        None,
        "and Escape left the control unbound"
    );
}

/// The Main Mixer's fade learns as a node's control does: `Alt` + click marks it as waiting
/// and leaves the MIDI window shut.
#[test]
fn alt_clicking_the_fade_marks_it_and_opens_nothing() {
    use supersilvia::midi::Target;

    let mut h = tall_harness();
    h.step();
    let at = h.get_by_label("A / B balance -1.00").rect().center();
    alt_click_at(&mut h, at);
    h.step();
    assert_eq!(h.state().midi_learning(), Some(Target::Balance));
    assert!(h.query_by_label("MIDI").is_none(), "the window stayed shut");
    assert!(
        h.query_by_label("A / B balance -1.00 (learning MIDI)")
            .is_some(),
        "the fade says it is waiting"
    );
}

/// A number a region draws learns and wears its binding exactly as a row's does: `Alt` +
/// click breathes, the message binds it, and the dot names what drives it — on the cell's own
/// corner, in the grid's gutter, since a cell has a neighbour on either side.
#[test]
fn a_regions_own_number_learns_and_wears_its_binding() {
    use supersilvia::midi::{Kind, Message, Trigger};

    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_COSINEGRADIENT);
    let id = h.state().graph().iter().next().expect("the palette").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);
    let name = format!("cosinegradient{id}.freqG");
    let cell = h.get_by_label_contains(&name).rect();

    alt_click_at(&mut h, cell.center());
    h.step();
    assert_eq!(
        h.state().midi_learning(),
        Some(PortRef::new(id, "freqG").into()),
        "the cell is waiting for a message"
    );
    assert!(
        h.query_by_label_contains(&format!("{name} "))
            .and_then(|n| n.accesskit_node().label())
            .is_some_and(|l| l.ends_with("(learning MIDI)")),
        "and says so"
    );

    h.state_mut().apply_midi(Message {
        channel: 2,
        kind: Kind::Control { cc: 21, value: 0 },
    });
    h.run_steps(2);
    assert_eq!(
        h.state().midi_trigger_of(PortRef::new(id, "freqG")),
        Some(Trigger::Control { channel: 2, cc: 21 }),
    );
    let mark = h
        .query_by_label_contains(&format!("cosinegradient{id} freqG bound to CC 21 ch 3"))
        .expect("the cell wears the mark that names what drives it")
        .rect();
    assert!(
        mark.contains(cell.left_top()),
        "on the cell's top-left corner: {mark:?} for {cell:?}"
    );

    // The sequencer's lane numbers are the other grid of a region's own numbers.
    add_node(&mut h, ADD_EUCLIDEANRHYTHM);
    let lanes = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| n.def.slug == "euclideanrhythm")
        .expect("the sequencer")
        .0;
    h.state_mut().bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 60,
        },
        PortRef::new(lanes, "lane3rotation"),
    );
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(lanes, Pos2::new(20.0, 440.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);
    assert!(
        h.query_by_label_contains(&format!(
            "euclideanrhythm{lanes} lane3rotation bound to Note C4 ch 1"
        ))
        .is_some(),
        "a lane number wears it too"
    );

    h.snapshot("cosine_gradient_bound_number");
}

/// The Main Mixer's fade wears the dot every bound control wears, after the Mix heading over
/// it and inside the panel, named for what drives it.
#[test]
fn a_bound_fade_wears_the_mark_beside_its_heading() {
    use supersilvia::midi::{Target, Trigger};

    let mut h = tall_harness();
    h.step();
    assert!(
        h.query_by_label_contains("A / B balance bound to")
            .is_none()
    );
    h.state_mut()
        .bind_midi_unchecked(Trigger::Control { channel: 0, cc: 1 }, Target::Balance);
    h.run_steps(2);
    let mark = h.get_by_label("A / B balance bound to CC 1 ch 1").rect();
    let fade = h.get_by_label("A / B balance -1.00").rect();
    assert!(
        mark.max.y <= fade.min.y && mark.min.x >= fade.min.x && mark.max.x <= fade.max.x,
        "over the fade, beside its heading: {mark:?} over {fade:?}"
    );
    h.snapshot("mixer_fade_bound");
}

/// A press from anywhere but the pointer survives the canvas redrawing.
///
/// `App::press` is the one seam a MIDI note, a test and the agent-driven layer all go
/// through. The canvas replaces *its* half of the held set every frame, from the buttons the
/// pointer is on — and it used to replace the whole thing, so a note went down and was let go
/// of on the same frame it arrived. The device delivers mid-frame, after the `tick` that
/// would have read it, so nothing ever saw the edge.
///
/// Asserted on the held set rather than through a note, because a test can only hand a
/// message in *between* frames, where it survives into the next tick either way. That timing
/// is exactly what hid this.
#[test]
fn a_press_from_elsewhere_survives_the_canvas_redrawing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    h.step();
    let (out, _) = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| n.def.is_output)
        .expect("an Output");
    let port = PortRef::new(out, "show_a");

    h.state_mut().press(port, true);
    h.run_steps(4);
    assert!(
        h.state().is_held(port),
        "four canvas passes and the finger is still on it"
    );
    assert_eq!(h.state().mixer().a, Some(out), "and it claimed the deck");

    h.state_mut().press(port, false);
    h.run_steps(2);
    assert!(!h.state().is_held(port), "and letting go lets go");
}

/// The right-click editor binds and unbinds too: `Alt` + click is not the only way in, and a
/// gesture nobody has been told about is not findable.
#[test]
fn the_range_editor_binds_and_unbinds() {
    use supersilvia::midi::{Kind, Message};

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (node, _) = h.state().graph().iter().next().expect("the node");
    let port = PortRef::new(node, "frequency");
    let name = format!("checkerboard{node}.frequency");

    // Right-click the control: the editor, with the offer on it.
    let at = number_rect(&h, &name).center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.step();
    h.get_by_label_contains("Bind MIDI").click();
    h.run_steps(2);
    assert_eq!(
        h.state().midi_learning(),
        Some(port.into()),
        "the editor asked for a binding"
    );

    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 7, value: 0 },
    });
    h.run_steps(3);
    assert!(h.state().midi_trigger_of(port).is_some(), "bound");

    // And again, to unbind: the same menu, now offering the other half.
    let at = number_rect(&h, &name).center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
    }
    h.step();
    h.get_by_label_contains("Unbind").click();
    h.run_steps(2);
    assert_eq!(
        h.state().midi_trigger_of(port),
        None,
        "and the editor forgot it"
    );
}

/// `Alt` + click does not also move the control it is binding.
#[test]
fn learning_a_control_does_not_move_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (node, _) = h.state().graph().iter().next().expect("the node");
    let before = h.state().graph().get(node).unwrap().controls["frequency"];

    let at = number_rect(&h, &format!("checkerboard{node}.frequency")).center();
    alt_click_at(&mut h, at);

    assert_eq!(
        h.state().graph().get(node).unwrap().controls["frequency"],
        before,
        "an Alt drag binds rather than scrubs"
    );
    assert_eq!(h.state().undo_len(), 1, "and opens no undo step of its own");
}

/// silvia's status line, on the Output and on no other node.
///
/// Read by `value` rather than by label: a `Label`'s text is its accessible *value*, and the
/// name is empty.
#[test]
fn an_output_says_what_it_is_showing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    add_node(&mut h, ADD_CHECKERBOARD);
    h.step();
    let (out, _) = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| n.def.is_output)
        .expect("an Output");

    assert!(
        h.query_by_value(&format!("output{out} status no input hidden ready"))
            .is_some(),
        "an Output with nothing cabled in, on no deck, not rendering"
    );
    assert_eq!(
        h.get_all_by_role(egui::accesskit::Role::Label)
            .filter(|n| n.value().is_some_and(|v| v.contains(" status ")))
            .count(),
        1,
        "the line is the Output's, and the checkerboard beside it carries none"
    );
}

/// The status line is live: it follows the cable, the deck claim and the render.
#[test]
fn the_status_line_follows_the_cable_and_the_deck() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    h.step();
    let (out, _) = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| n.def.is_output)
        .expect("an Output");
    let (src, _) = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| !n.def.is_output)
        .expect("a source");

    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(src, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();
    assert!(
        h.query_by_value(&format!("output{out} status input hidden ready"))
            .is_some(),
        "the first cell followed the cable"
    );

    h.state_mut().press(PortRef::new(out, "show_a"), true);
    h.step();
    h.step();
    assert!(
        h.query_by_value(&format!("output{out} status input on A ready"))
            .is_some(),
        "and the second followed the deck"
    );
}

/// silvia's `#workspace-controls`: only over a strip, because a plane already pans as far
/// as it is pushed and has no ranks to arrange into columns.
///
/// One button now. Extend and Crop were beside it and are gone: the strip makes its own
/// room under a drag and takes it back on its own, so neither had anything left to ask for.
#[test]
fn the_strip_controls_belong_to_the_strip() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    assert!(
        h.query_by_label("Auto-arrange").is_none(),
        "a plane has nothing to arrange"
    );

    set_layout(&mut h, "Linear");
    h.step();
    assert!(
        h.query_by_label("Auto-arrange").is_some(),
        "the strip carries Auto-arrange"
    );
    for gone in ["Extend", "Crop"] {
        assert!(
            h.query_by_label(gone).is_none(),
            "{gone} is a button for a length the strip decides for itself"
        );
    }
}

/// **The strip crops itself.** Nothing asks it to: a node moved in from the far end leaves
/// room past the last node that nothing can be reached in, and the strip gives it back.
///
/// Which is only bearable because the length the view is held to eases — the pan is clamped
/// to it, so a strip that shortened in the frame the node landed would take the camera with
/// it. The glide itself is
/// `deleting_a_node_at_the_far_end_glides_the_view_rather_than_jumping_it`; what is under
/// test here is that the trim happens at all, with no button pressed.
#[test]
fn a_node_moved_in_from_the_far_end_leaves_the_strip_trimming_itself() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    let out = far_end(&mut h);

    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[1], Pos2::new(200.0, 60.0))],
        })
        .unwrap();
    h.run_steps(60);
    assert!(
        far_end(&mut h) > out,
        "the strip kept room past a node that had moved in: {} against {out}",
        far_end(&mut h)
    );
}

/// **The map frames a whole viewport across the strip, whatever the nodes occupy.**
///
/// The band a node can be in is `clamp_to_strip`'s, which is the viewport's height. Framed
/// to the nodes' own top and bottom instead, a node's place on the map means something
/// different from one frame to the next: move one node down and every other node slides up,
/// on a map nobody touched. Here the second node moves the length of the band and the first
/// one does not move on the map at all.
#[test]
fn the_minimap_frames_a_whole_viewport_however_tall_the_nodes_are() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    h.run_steps(60);
    let name = format!("rail checkerboard{}", ids[0]);
    let before = h.get_by_label(&name).rect();

    // Down the band but still inside it, which is the whole of what used to rescale the map.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[1], Pos2::new(1200.0, 140.0))],
        })
        .unwrap();
    h.run_steps(60);
    let after = h.get_by_label(&name).rect();
    assert!(
        (after.min - before.min).length() < 0.01 && (after.size() - before.size()).length() < 0.01,
        "another node moving down the band moved this one on the map: {after:?} against {before:?}"
    );
}

/// The map is a map of the strip and not only of what is on it, so a strip that trims itself
/// rescales the map as it goes.
///
/// A graph wide enough that the map's scale is set by its width. On a short one the rail's
/// own height is what binds, and a thousand points along change nothing.
#[test]
fn the_minimap_rescales_as_the_strip_trims_itself() {
    let mut h = harness();
    h.step();
    for _ in 0..2 {
        add_node(&mut h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 3000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.run_steps(60);
    let name = format!("rail checkerboard{}", ids[0]);
    let before = h.get_by_label(&name).rect().width();

    // The far node brought in, which shortens the strip the map is drawn from.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[1], Pos2::new(600.0, 60.0))],
        })
        .unwrap();
    // The strip eases to its new length rather than snapping, so the map is still on its way
    // for a few frames.
    h.run_steps(60);
    assert!(
        h.get_by_label(&name).rect().width() > before,
        "the node stayed small on a map of a shorter strip: {} is not over {before}",
        h.get_by_label(&name).rect().width()
    );
}

/// Press the primary button where a drag is to begin. The move is a frame of its own, as
/// `click_at` explains: a press is hit-tested against where the pointer already is.
fn press_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

/// Move the held pointer, one frame.
fn move_to(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
}

/// Let go where the pointer already is, and settle.
fn release_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
}

/// The canvas area, which is the whole of the workspace above the rail.
fn canvas_rect(h: &Harness<'_, App>) -> egui::Rect {
    egui::Rect::from_min_size(
        h.state().canvas_origin(),
        egui::vec2(h.state().canvas_width(), h.state().canvas_height()),
    )
}

/// Two nodes a long way apart on a linear workspace, which is a strip with somewhere to
/// scroll to. Answers their ids in the order they sit along it.
fn a_long_strip(h: &mut Harness<'_, App>) -> Vec<supersilvia::graph::NodeId> {
    h.step();
    for _ in 0..2 {
        add_node(h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 1200.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(h, "Linear");
    h.run_steps(2);
    ids
}

/// The node under the hand is part of the strip the view is clamped to, so dragging the
/// outermost one inward shortens the strip under the drag: the clamp narrows, the pan is
/// pulled along with it and the thing being placed moves while it is being placed. The
/// bounds are frozen for the length of the gesture instead, and settle when it ends.
#[test]
fn dragging_the_outermost_node_inward_leaves_the_pan_alone_until_it_is_dropped() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    // At the far end of the strip, hard against the clamp, which is where a strip that
    // shortens takes the view with it.
    let end = far_end(&mut h);

    // Twenty points a frame, which keeps the pointer clear of the margin at the near edge:
    // what is under test here is the strip shortening, not the creep.
    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    for step in 1..=4 {
        move_to(&mut h, from - egui::vec2(20.0 * step as f32, 0.0));
        assert_eq!(
            h.state().canvas_transform().pan.x,
            end,
            "the strip shortened under the drag and took the view with it"
        );
    }
    let last = from - egui::vec2(80.0, 0.0);
    release_at(&mut h, last);
    assert_eq!(
        h.state().graph().get(ids[1]).unwrap().pos.x,
        1200.0 - 80.0,
        "the node landed where the pointer left it"
    );
    assert!(
        h.state().canvas_transform().pan.x > end,
        "the strip never settled to the shorter graph it now is"
    );
}

/// The map is a map of the strip the view can reach, so a strip held still under a drag
/// holds the map's scale still with it. A map that rescaled on every frame of a drag would
/// be a map nobody can read while they are using it.
///
/// Short nodes a long way apart, for the reason `extending_the_strip_rescales_the_minimap`
/// gives: on a strip whose scale is set by the rail's own height, nothing along it changes
/// the map at all.
#[test]
fn the_minimap_keeps_its_scale_while_a_node_is_dragged() {
    let mut h = harness();
    h.step();
    for _ in 0..2 {
        add_node(&mut h, ADD_COLOR);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 3000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.run_steps(2);
    // At the far end, which is where the node that is the far end of the strip can be
    // reached: an off-screen node is culled and has no drag handle to take.
    far_end(&mut h);
    let name = format!("rail color{}", ids[0]);
    let before = h.get_by_label(&name).rect().width();

    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    for step in 1..=4 {
        move_to(&mut h, from - egui::vec2(20.0 * step as f32, 0.0));
        assert_eq!(
            h.get_by_label(&name).rect().width(),
            before,
            "the map rescaled under the drag"
        );
    }
    release_at(&mut h, from - egui::vec2(80.0, 0.0));
    assert!(
        h.get_by_label(&name).rect().width() > before,
        "the map never settled to the shorter strip"
    );
}

/// A node held against the near edge of the viewport creeps the view that way, and keeps its
/// own place under the cursor while the world moves beneath it.
#[test]
fn dragging_a_node_against_an_edge_creeps_the_view_and_the_node_stays_under_the_pointer() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    far_end(&mut h);

    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    // Into the margin at the near edge, and then held there: the creep is the hand staying
    // still, not the hand moving.
    let at = Pos2::new(canvas_rect(&h).min.x + 8.0, from.y);
    move_to(&mut h, at);

    // Where the node is on screen: its world position and the pan it was placed at, which
    // are one frame's pair. A node that keeps its place under a still pointer keeps this.
    let grip = |h: &Harness<'_, App>| {
        h.state().graph().get(ids[1]).unwrap().pos.x + h.state().canvas_transform().pan.x
    };
    let held = grip(&h);
    let mut pan = h.state().canvas_transform().pan.x;
    for _ in 0..4 {
        h.step();
        let now = h.state().canvas_transform().pan.x;
        assert!(now > pan, "the view did not creep: {now} is not past {pan}");
        pan = now;
        assert!(
            (grip(&h) - held).abs() < 0.001,
            "the node slid out from under the pointer: {} is not {held}",
            grip(&h)
        );
    }

    // And it stops the moment the pointer comes out of the margin.
    let middle = Pos2::new(canvas_rect(&h).center().x, from.y);
    move_to(&mut h, middle);
    let still = h.state().canvas_transform().pan.x;
    h.run_steps(2);
    assert_eq!(
        h.state().canvas_transform().pan.x,
        still,
        "the creep ran on with the pointer nowhere near an edge"
    );
}

/// The creep's rate is read from how deep into the margin the pointer is and from nothing
/// the creep itself changes, so two frames of the same hand move the view by the same
/// amount rather than by a growing one.
#[test]
fn the_creep_does_not_compound_from_one_frame_to_the_next() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    far_end(&mut h);

    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    let at = Pos2::new(canvas_rect(&h).min.x + 8.0, from.y);
    move_to(&mut h, at);

    let mut pan = h.state().canvas_transform().pan.x;
    let mut steps = Vec::new();
    for _ in 0..4 {
        h.step();
        let now = h.state().canvas_transform().pan.x;
        steps.push(now - pan);
        pan = now;
    }
    assert!(steps[0] > 0.0, "nothing moved at all: {steps:?}");
    for step in &steps {
        assert!(
            (step - steps[0]).abs() < 0.001,
            "the creep accelerated into itself: {steps:?}"
        );
    }
}

/// The far end of a strip yields as a drag pushes at it, so a node can be carried past the
/// last one on the strip — and it yields **faster than the view follows**.
///
/// Room made at the creep's own rate is room used the instant it exists, which leaves the pan
/// against its own clamp for the whole push. Leading by a little fixes that. It is only a
/// little, because how fast the gesture *feels* is the creep and not this — the node is
/// under the cursor. What is made and not used is given back, which is the other half of
/// this test.
#[test]
fn dragging_against_the_far_edge_makes_room_faster_than_the_view_scrolls() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    far_end(&mut h);

    // The view is already as far along as the strip goes, so every point of creep from here
    // is a point of room that did not exist.
    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    let at = Pos2::new(canvas_rect(&h).max.x - 8.0, from.y);
    move_to(&mut h, at);

    let reach = |h: &Harness<'_, App>| h.state().strip_reach().unwrap().max.x;
    let mut pan = h.state().canvas_transform().pan.x;
    let mut far = reach(&h);
    let (mut panned, mut grew) = (0.0, 0.0);
    for _ in 0..4 {
        h.step();
        let now = h.state().canvas_transform().pan.x;
        panned += pan - now;
        pan = now;
        grew += reach(&h) - far;
        far = reach(&h);
    }
    assert!(panned > 0.0, "the far end never gave: {panned}");
    assert!(
        grew > panned * 1.5,
        "the room came no faster than the view that wanted it: {grew} against {panned}"
    );

    release_at(&mut h, at);
    let held = reach(&h);
    h.run_steps(90);
    assert!(
        reach(&h) < held - 1.0,
        "the room the drag made was never given back: {} against {held}",
        reach(&h)
    );
}

/// A drag holds the node at the place on it the hand closed on, so a pointer brought back to
/// where it pressed brings the node back to where it was.
#[test]
fn a_drag_that_comes_back_to_where_it_started_leaves_the_node_where_it_started() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);
    let before = h.state().graph().get(id).unwrap().pos;

    // Every point of the way clear of the margins at the edges, where the view would creep
    // and the world under the pointer would no longer be the world it pressed on.
    let from = header_at(&h, id);
    press_at(&mut h, from);
    move_to(&mut h, from + egui::vec2(140.0, 180.0));
    move_to(&mut h, from + egui::vec2(60.0, 40.0));
    move_to(&mut h, from);
    release_at(&mut h, from);
    assert_eq!(h.state().graph().get(id).unwrap().pos, before);
}

/// The strip's floor decides where a node is *placed*; it does not take anything away from
/// the hand holding it. A node pushed into the floor and lifted off it again is back at the
/// offset from the cursor it was grabbed at, rather than at one the clamp has eaten.
#[test]
fn a_node_pushed_into_the_strips_floor_keeps_the_grip_it_was_taken_with() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    set_layout(&mut h, "Linear");
    h.run_steps(2);

    // Where the node sits while the drag is in flight and nothing is clamping it, which is
    // what it has to come back to. The strip's top margin has a say in where a node lands
    // the moment one is dragged, so this is read from the drag rather than from before it.
    let from = header_at(&h, id);
    press_at(&mut h, from);
    let start = from + egui::vec2(20.0, 20.0);
    move_to(&mut h, start);
    let held = h.state().graph().get(id).unwrap().pos;

    // Hard into the floor, and far enough past it that the clamp is holding the node well
    // above where the pointer has gone.
    let down = Pos2::new(start.x, canvas_rect(&h).max.y - 4.0);
    move_to(&mut h, down);
    let floored = h.state().graph().get(id).unwrap().pos;
    assert!(
        floored.y < held.y + (down.y - start.y),
        "the floor never caught the node, so this proves nothing"
    );
    move_to(&mut h, start);
    release_at(&mut h, start);
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        held,
        "the clamp ate the grip on the way down"
    );
}

/// Arrange is the Workspace menu's auto-arrange under the pointer, so it is an edit and one
/// undo step — where Extend and Crop are neither.
#[test]
fn the_arrange_button_is_one_undoable_arrange() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(900.0, 60.0))],
        })
        .unwrap();
    set_layout(&mut h, "Linear");
    h.step();
    let steps = h.state().undo_len();
    let before = h.state().graph().get(id).unwrap().pos;

    let at = h.get_by_label("Auto-arrange").rect().center();
    click_at(&mut h, at);
    let after = h.state().graph().get(id).unwrap().pos;
    assert_ne!(after, before, "the button arranged the graph");
    assert_eq!(h.state().undo_len(), steps + 1, "and it is one step");

    h.state_mut().undo();
    h.step();
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        before,
        "which undoes on its own"
    );
}

/// A wheel has one axis and a trackpad has two, and the strip treats them as the different
/// devices they are: notches run along the graph, fingers go where they are pushed.
#[test]
fn a_trackpad_pans_both_axes_where_a_wheel_only_runs_along_the_strip() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_CHECKERBOARD);
    // Two nodes far apart, so the graph is taller than any window kittest gives us and
    // there is somewhere vertical to go.
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(60.0, n as f32 * 4000.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.step();
    // Nodes that far apart sit outside the strip's height, so the switch asks whether to
    // arrange them. A modal's backdrop is over the canvas and takes the wheel with it.
    h.get_by_label("Leave them").click();
    h.run_steps(2);

    let before = h.state().canvas_transform().pan;
    trackpad(&mut h, egui::Vec2::new(0.0, 60.0));
    assert!(
        h.state().canvas_transform().pan.y != before.y,
        "fingers move the view across the strip"
    );

    let before = h.state().canvas_transform().pan;
    wheel(&mut h, 3.0);
    assert_eq!(
        h.state().canvas_transform().pan.y,
        before.y,
        "a wheel notch never does"
    );
}

/// A flick keeps going after the fingers stop, and then stops rather than drifting.
#[test]
fn a_flick_glides_and_settles() {
    let mut h = harness();
    h.step();
    for _ in 0..3 {
        add_node(&mut h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 3000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.step();

    trackpad(&mut h, egui::Vec2::new(-90.0, 0.0));
    let launched = h.state().canvas_transform().pan.x;

    // No further input: whatever moves now is momentum.
    h.step();
    let gliding = h.state().canvas_transform().pan.x;
    assert!(gliding < launched, "the strip carries on after the flick");

    h.run_steps(120);
    let settled = h.state().canvas_transform().pan.x;
    h.run_steps(10);
    assert_eq!(
        h.state().canvas_transform().pan.x,
        settled,
        "and comes to rest rather than drifting"
    );
}

// ---------------------------------------------------------- who the wheel belongs to

/// Ten notches at one point, well past whatever the surface under it can absorb.
fn notches_at(h: &mut Harness<'_, App>, at: Pos2) {
    for _ in 0..10 {
        scroll_at(
            h,
            at,
            egui::MouseWheelUnit::Line,
            egui::Vec2::new(0.0, -3.0),
        );
    }
}

/// A surface that takes the wheel takes all of it. egui's `ScrollArea` consumes the delta
/// only while it can move, so at the end of a list there is a full notch left over; the
/// canvas must not read it.
#[test]
fn the_nodes_menu_takes_the_whole_wheel_at_the_end_of_its_list() {
    let mut h = harness();
    h.step();

    h.get_by_label("Nodes").click();
    // Two frames: the first pass an `Area` is shown is a sizing pass, and its geometry is
    // provisional.
    h.run_steps(2);
    h.get_by_label_contains("Generate").click();
    h.run_steps(2);

    let at = h.get_by_label("add checkerboard").rect().center();
    let before = h.state().canvas_transform().zoom;
    notches_at(&mut h, at);
    assert_eq!(
        h.state().canvas_transform().zoom,
        before,
        "the wheel over the submenu zoomed the canvas underneath it"
    );

    // The other half of the rule: the canvas still has the wheel where the menu is not.
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    wheel(&mut h, -2.0);
    assert!(
        h.state().canvas_transform().zoom < before,
        "the canvas stopped zooming on its own background"
    );
}

/// The browser draws the same list and is the same rule.
#[test]
fn the_browser_takes_the_whole_wheel_at_the_end_of_its_list() {
    let mut h = harness();
    h.run_steps(2);

    h.key_press(egui::Key::Slash);
    h.run_steps(2);
    // A query short enough that the list cannot scroll at all, which is the case with
    // nothing left for the `ScrollArea` to consume.
    h.input_mut()
        .events
        .push(egui::Event::Text("checker".into()));
    h.run_steps(2);

    let at = h.get_by_label("add checkerboard").rect().center();
    let before = h.state().canvas_transform().zoom;
    notches_at(&mut h, at);
    assert_eq!(
        h.state().canvas_transform().zoom,
        before,
        "the wheel over the browser's list zoomed the canvas underneath it"
    );
}

/// A panel is not the canvas either, though it shares the canvas's layer.
#[test]
fn the_wheel_over_the_preview_panel_leaves_the_canvas_alone() {
    let mut h = harness();
    h.run_steps(2);

    let origin = h.state().canvas_origin();
    let at = Pos2::new(
        origin.x + h.state().canvas_width() + 40.0,
        origin.y + h.state().canvas_height() * 0.5,
    );
    let before = h.state().canvas_transform().zoom;
    notches_at(&mut h, at);
    assert_eq!(
        h.state().canvas_transform().zoom,
        before,
        "the wheel over the preview panel zoomed the canvas"
    );
}

/// The strip reads the frame's `MouseWheel` events rather than the smoothed delta, so it
/// needs the same gate: a `ScrollArea` never touches those events at all.
#[test]
fn the_wheel_over_a_menu_does_not_move_the_strip() {
    let mut h = harness();
    h.step();
    for _ in 0..3 {
        add_node(&mut h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 2000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.step();

    // The canvas has the wheel while nothing is over it.
    let before = h.state().canvas_transform().pan.x;
    wheel(&mut h, -3.0);
    assert!(
        h.state().canvas_transform().pan.x < before,
        "a notch on the canvas did not move the strip"
    );

    h.get_by_label("Nodes").click();
    h.run_steps(2);
    h.get_by_label_contains("Generate").click();
    h.run_steps(2);
    let at = h.get_by_label("add checkerboard").rect().center();

    // Let any glide from the notch above run out first, so what is measured is the wheel.
    h.run_steps(120);
    let before = h.state().canvas_transform().pan.x;
    notches_at(&mut h, at);
    assert_eq!(
        h.state().canvas_transform().pan.x,
        before,
        "the wheel over the submenu scrolled the strip underneath it"
    );
}

/// Only *new* wheel input is gated. A glide started on the canvas was launched there, so
/// the pointer crossing a menu does not stop it.
#[test]
fn a_glide_survives_the_pointer_crossing_a_menu() {
    let mut h = harness();
    h.step();
    for _ in 0..3 {
        add_node(&mut h, ADD_CHECKERBOARD);
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    for (n, id) in ids.iter().enumerate() {
        h.state_mut()
            .apply(Command::MoveNodes {
                moves: vec![(*id, Pos2::new(n as f32 * 3000.0, 60.0))],
            })
            .unwrap();
    }
    set_layout(&mut h, "Linear");
    h.step();

    trackpad(&mut h, egui::Vec2::new(-90.0, 0.0));
    let launched = h.state().canvas_transform().pan.x;

    // The pointer goes to the button and then into the submenu, and no wheel input follows.
    h.get_by_label("Nodes").click();
    h.run_steps(2);
    h.get_by_label_contains("Generate").click();
    h.run_steps(2);
    assert!(
        h.state().canvas_transform().pan.x < launched,
        "the glide stopped when the pointer left the canvas"
    );
}

/// Letting go ends a gesture. Without that, a gesture ended only when some *other* command
/// interrupted it, so grabbing the same node twice collapsed into one step and one undo put
/// it back at the start of the first drag.
#[test]
fn two_drags_of_one_node_are_two_undo_steps() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    // Somewhere with room on either side, so both drags land on canvas.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(200.0, 200.0))],
        })
        .unwrap();
    h.step();

    let header = |h: &Harness<'_, App>| {
        let node = h.state().graph().get(id).unwrap();
        let origin = h.state().canvas_origin();
        let world = node.pos + egui::vec2(40.0, 8.0);
        h.state().canvas_transform().to_screen(origin, world)
    };
    let from = header(&h);
    drag_with(
        &mut h,
        from,
        from + egui::vec2(60.0, 0.0),
        egui::Modifiers::NONE,
    );
    let after_first = h.state().graph().get(id).unwrap().pos;
    let steps = h.state().history().len();

    let from = header(&h);
    drag_with(
        &mut h,
        from,
        from + egui::vec2(0.0, 60.0),
        egui::Modifiers::NONE,
    );

    assert_eq!(
        h.state().history().len(),
        steps + 1,
        "the second grab is its own step: {:?}",
        h.state().history()
    );
    assert!(h.state_mut().undo());
    h.step();
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        after_first,
        "and one undo walks back only the second drag"
    );
}

/// The file button offers what the project already holds before it offers the file system.
///
/// The gesture this is for: a rig is built out of clips that are already imported, and
/// reaching one of those through a file dialog means navigating into a folder the project
/// owns to find a copy it made itself.
#[test]
fn the_file_button_offers_the_projects_own_media() {
    let root = std::env::temp_dir().join(format!("supersilvia-picker-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let mut h = harness();
    h.step();
    h.state_mut().new_project(root.clone());
    // Two clips in `assets/` and one thing a video node cannot play.
    let assets = root.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    for name in ["corridor.webm", "kick.mp4", "notes.txt"] {
        std::fs::write(assets.join(name), b"not really media").unwrap();
    }
    // Reopening is what re-reads the folder, as it does for anything written from outside.
    h.state_mut().open_project(root.clone());
    h.step();

    add_node(&mut h, ADD_VIDEO);
    h.get_by_label_contains("video1.file").click();
    h.step();

    assert!(
        h.query_by_label("asset corridor.webm").is_some(),
        "the project's own clips are the first thing offered"
    );
    assert!(h.query_by_label("asset kick.mp4").is_some());
    assert!(
        h.query_by_label("asset notes.txt").is_none(),
        "and only what the node can play"
    );
    assert!(
        h.query_by_label("Import a file…").is_some(),
        "with the dialog under them"
    );

    // The pointer arrives before it presses. kittest's `click()` is a move, a press and a
    // release in one frame, and a popup that opened under a pointer which was somewhere else
    // is not hovered on the frame that click lands.
    let at = h.get_by_label("asset kick.mp4").rect().center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    h.get_by_label("asset kick.mp4").click();
    h.step();

    let id = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    assert_eq!(
        h.state().graph().get(id).unwrap().options.get("file"),
        Some(&"assets/kick.mp4".to_string()),
        "choosing one sets the option to the reference, with nothing copied"
    );
    assert!(
        h.state().open_control().is_none(),
        "and the picker closes behind it"
    );
    assert!(
        h.state().project().resolve("assets/kick.mp4").is_file(),
        "the file it names is the one already in the folder"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A project with nothing of the kind is the case the dialog was always for, so the button
/// still goes straight there rather than opening an empty menu in front of it.
#[test]
fn the_file_button_with_nothing_to_offer_asks_for_a_file() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_VIDEO);
    h.get_by_label_contains("video1.file").click();
    h.step();

    assert!(
        h.query_by_label("Import a file…").is_none(),
        "a scratch project holds no clips, so there is no menu to put in the way"
    );
}

#[test]
fn auto_arrange_is_one_undo_step_and_a_later_drag_survives_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef {
                node: ids[0],
                key: "output",
            },
            to: PortRef {
                node: ids[1],
                key: "input",
            },
        })
        .unwrap();
    let steps = h.state().history().len();

    auto_arrange(&mut h);

    assert_eq!(
        h.state().history().len(),
        steps + 1,
        "one command for the whole arrangement"
    );
    let source = h.state().graph().get(ids[0]).unwrap().pos;
    let sink = h.state().graph().get(ids[1]).unwrap().pos;
    assert!(sink.x > source.x, "the consumer lands in a later column");

    // Manual placement outlives an arrange. Linear constrains navigation, not authorship.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[0], Pos2::new(source.x, source.y + 120.0))],
        })
        .unwrap();
    h.step();
    assert_eq!(
        h.state().graph().get(ids[0]).unwrap().pos.y,
        source.y + 120.0,
        "the node stays where it was put"
    );
}

#[test]
fn a_minimap_node_is_in_the_accessibility_tree_and_moves_the_view() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(4000.0, 60.0))],
        })
        .unwrap();
    set_layout(&mut h, "Linear");
    h.step();

    assert!(
        h.query_by_label("rail checkerboard1").is_some(),
        "the minimap names its node"
    );
    h.get_by_label("rail checkerboard1").click();
    h.step();
    h.step();
    let pan = h.state().canvas_transform().pan.x;
    let center = 4000.0 + supersilvia::ui::canvas::NODE_WIDTH * 0.5;
    assert!(
        (pan + center).abs() < 1200.0,
        "clicking a node on the map brings it into view (pan {pan})"
    );
}

/// **A click on the minimap during a glide stays where it was sent.** A node let go of out of
/// sight sends the view after it; the map clicked on the way is the later word, and a glide
/// left running would pull the view back to the node.
#[test]
fn a_minimap_click_during_a_glide_is_not_pulled_back() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    let grip = header_at(&h, ids[0]);
    press_at(&mut h, grip);
    let away = Pos2::new(canvas_rect(&h).min.x - 260.0, grip.y);
    move_to(&mut h, away);
    release_at(&mut h, away);
    let gliding = h.state().canvas_transform().pan.x;
    h.step();
    assert_ne!(
        h.state().canvas_transform().pan.x,
        gliding,
        "the drop sent the view after the node"
    );

    h.get_by_label(&format!("rail checkerboard{}", ids[1]))
        .click();
    h.run_steps(2);
    let sent = h.state().canvas_transform().pan.x;
    h.run_steps(60);
    let settled = h.state().canvas_transform().pan.x;
    assert!(
        (settled - sent).abs() < 1.0,
        "the view went to the map's node at {sent} and was pulled back to {settled}"
    );
}

/// The minimap is drawn only where it means something.
#[test]
fn the_minimap_appears_with_the_strip_and_not_on_the_plane() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    assert!(h.query_by_label("rail checkerboard1").is_none());

    set_layout(&mut h, "Linear");
    h.step();
    assert!(h.query_by_label("rail checkerboard1").is_some());
    h.snapshot("linear_rail");
}

/// Press, move and release the primary button, with modifiers held throughout.
///
/// `drag()` in kittest has no modifier argument, and Shift is what separates a rubber band
/// from a pan — so the events are synthesised. Each `step` is a frame, which is what lets
/// egui see a press, a move and a release as one drag rather than three unrelated events.
fn drag_with(h: &mut Harness<'_, App>, from: Pos2, to: Pos2, modifiers: egui::Modifiers) {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    };
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.input_mut().events.push(button(from, true));
    h.step();
    h.input_mut().events.push(egui::Event::PointerMoved(to));
    h.step();
    h.input_mut().events.push(button(to, false));
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.step();
}

fn click_with(h: &mut Harness<'_, App>, at: Pos2, modifiers: egui::Modifiers) {
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.step();
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.step();
}

/// Press and release the secondary button, which is what opens a context menu.
fn right_click(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.run_steps(2);
}

/// Where a node's header is on screen, which is its drag handle and its click target.
fn header_at(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> Pos2 {
    let t = h.state().canvas_transform();
    let world = node_rect(h, id);
    // The canvas starts below the menu bar; its origin is where the panel does.
    let origin = h.state().canvas_origin();
    t.to_screen(origin, world.min) + egui::vec2(30.0, 8.0)
}

/// A point on a node's body away from every control: the empty half of an output row, whose
/// port sits on the right edge and whose label is beside it.
fn body_at(h: &Harness<'_, App>, id: supersilvia::graph::NodeId) -> Pos2 {
    let t = h.state().canvas_transform();
    let laid = laid_out(h, id);
    let l = laid.find(id).expect("the node");
    let row = l
        .rows
        .iter()
        .find(|r| matches!(r.row, supersilvia::ui::canvas::Row::Output(_)))
        .expect("an output row");
    let origin = h.state().canvas_origin();
    t.to_screen(
        origin,
        Pos2::new(
            l.rect.min.x + 60.0,
            l.rect.min.y + row.top + row.height * 0.5,
        ),
    )
}

/// A node's body selects it the way its header does, Shift included.
#[test]
fn clicking_a_nodes_body_selects_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (a, b) = (ids[0], ids[1]);
    h.run_steps(2);

    let at = body_at(&h, a);
    click_with(&mut h, at, egui::Modifiers::NONE);
    assert_eq!(
        h.state().canvas_selection(),
        vec![a],
        "the body did not select"
    );

    let at = body_at(&h, b);
    click_with(&mut h, at, egui::Modifiers::SHIFT);
    assert_eq!(
        h.state().canvas_selection(),
        vec![a, b],
        "Shift on a body did not add"
    );

    let at = body_at(&h, b);
    click_with(&mut h, at, egui::Modifiers::SHIFT);
    assert_eq!(
        h.state().canvas_selection(),
        vec![a],
        "Shift on a body did not remove"
    );
}

/// Dragging a node by its body moves it, as dragging the header does.
#[test]
fn dragging_a_nodes_body_moves_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);

    let before = h.state().graph().get(id).unwrap().pos;
    let from = body_at(&h, id);
    drag_with(
        &mut h,
        from,
        from + egui::vec2(50.0, 30.0),
        egui::Modifiers::NONE,
    );
    assert_eq!(
        h.state().graph().get(id).unwrap().pos - before,
        egui::vec2(50.0, 30.0),
        "the body is not a drag handle"
    );
}

/// A control keeps its own gesture: the body ground is under it, not over it.
#[test]
fn a_control_beats_the_body_ground() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.run_steps(2);

    h.get_by_label_contains("checkerboard1.color1 #").click();
    h.step();
    assert!(
        h.state().open_control().is_some(),
        "the ground swallowed the swatch's click"
    );
    assert!(
        h.state().canvas_selection().is_empty(),
        "a click that a control answered is not also a selection"
    );
}

/// Shift-click adds a node to the selection, and a second one takes it back out.
#[test]
fn shift_click_toggles_a_node_in_the_selection() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (a, b) = (ids[0], ids[1]);
    h.run_steps(2);

    let at = header_at(&h, a);
    click_with(&mut h, at, egui::Modifiers::NONE);
    assert_eq!(h.state().canvas_selection(), vec![a]);

    let at = header_at(&h, b);
    click_with(&mut h, at, egui::Modifiers::SHIFT);
    assert_eq!(
        h.state().canvas_selection(),
        vec![a, b],
        "shift did not add"
    );

    let at = header_at(&h, b);
    click_with(&mut h, at, egui::Modifiers::SHIFT);
    assert_eq!(
        h.state().canvas_selection(),
        vec![a],
        "shift did not remove"
    );

    // Without Shift it replaces rather than adds.
    let at = header_at(&h, b);
    click_with(&mut h, at, egui::Modifiers::NONE);
    assert_eq!(h.state().canvas_selection(), vec![b]);
}

/// Dragging one node of a selection moves all of them, keeping their relative layout.
#[test]
fn dragging_a_selection_moves_every_node_in_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (a, b) = (ids[0], ids[1]);
    h.run_steps(2);

    let before_a = h.state().graph().get(a).unwrap().pos;
    let before_b = h.state().graph().get(b).unwrap().pos;
    h.state_mut().select_only(&[a, b]);
    h.run_steps(2);

    let from = header_at(&h, a);
    drag_with(
        &mut h,
        from,
        from + egui::vec2(60.0, 40.0),
        egui::Modifiers::NONE,
    );

    let after_a = h.state().graph().get(a).unwrap().pos;
    let after_b = h.state().graph().get(b).unwrap().pos;
    assert_ne!(after_a, before_a, "the dragged node did not move");
    assert_eq!(
        after_a - before_a,
        after_b - before_b,
        "the selection did not move together"
    );
}

/// Shift-dragging the background bands nodes into the selection; a plain drag still pans.
#[test]
fn shift_drag_bands_nodes_and_a_plain_drag_still_pans() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.run_steps(2);

    // A band around everything. The canvas is the middle of the window below the tab bar: the
    // right is the preview panel and the left is the Main Input's folded spine, and a drag in
    // either is not a canvas drag at all.
    drag_with(
        &mut h,
        Pos2::new(40.0, 80.0),
        Pos2::new(410.0, 585.0),
        egui::Modifiers::SHIFT,
    );
    assert_eq!(h.state().canvas_selection(), ids, "the band missed nodes");
    let panned = h.state().canvas_transform().pan;

    // The same gesture without Shift pans instead, and bands nothing.
    h.state_mut().select_only(&[]);
    drag_with(
        &mut h,
        Pos2::new(200.0, 500.0),
        Pos2::new(260.0, 540.0),
        egui::Modifiers::NONE,
    );
    assert_ne!(h.state().canvas_transform().pan, panned, "it did not pan");
    assert!(
        h.state().canvas_selection().is_empty(),
        "a plain drag selected something"
    );
}

/// **A strip has no up and down, so dragging its background vertically moves nothing.**
///
/// Every node on a strip is held inside the viewport's own height by `clamp_to_strip`, so
/// there is nothing above or below the view to reach for. The pan used to be left wherever
/// the hand put it across the strip — which meant a drag on the background could push the
/// whole strip off the top of the window with nothing on screen to bring it back. Along the
/// strip the same drag still pans, which is the half that has somewhere to go.
#[test]
fn dragging_the_background_of_a_strip_moves_it_along_and_never_across() {
    let mut h = harness();
    // Long enough that there is somewhere to pan *along* to, which the second half needs:
    // a strip that fits the window is clamped on both axes and proves nothing about either.
    let _ = a_long_strip(&mut h);
    h.run_steps(30);

    // Well inside the canvas, which in Linear stops above the rail — a drag starting on the
    // rail is the map's own click-to-centre and not a background pan at all.
    let straight_up = h.state().canvas_transform().pan;
    drag_with(
        &mut h,
        Pos2::new(200.0, 400.0),
        Pos2::new(200.0, 120.0),
        egui::Modifiers::NONE,
    );
    h.run_steps(30);
    assert_eq!(
        h.state().canvas_transform().pan.y,
        straight_up.y,
        "the background drag pushed the strip up out of the window"
    );

    // Along it, the same gesture still pans: there the strip has ends rather than none.
    let along = h.state().canvas_transform().pan;
    drag_with(
        &mut h,
        Pos2::new(300.0, 400.0),
        Pos2::new(200.0, 400.0),
        egui::Modifiers::NONE,
    );
    assert_ne!(
        h.state().canvas_transform().pan.x,
        along.x,
        "a strip that cannot be panned along is not a strip"
    );
}

/// Collapsing a wired node must not take its cables with it.
///
/// A cable whose endpoint has no port slot is dropped by the canvas — silently, and only on
/// screen. So a collapsed node keeps a slot per port, gathered on its header.
#[test]
fn a_collapsed_node_keeps_its_cables() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();

    let label = format!("cable checkerboard{cb}.output to output{out}.input");
    assert!(
        h.query_by_label(&label).is_some(),
        "missing before collapse"
    );

    h.state_mut()
        .apply(Command::SetCollapsed {
            nodes: vec![cb],
            collapsed: true,
        })
        .unwrap();
    h.step();
    assert!(
        h.query_by_label(&label).is_some(),
        "collapsing took the cable with it"
    );
}

/// The header carries a close button, and it deletes through the bus like everything else.
#[test]
fn the_header_close_button_deletes_the_node() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);

    // The `?` sits immediately left of it, so this is also the test that a neighbor on the
    // header did not take the close button's hit.
    assert!(
        h.query_by_label(&format!("help checkerboard{id}"))
            .is_some()
    );

    h.get_by_label(&format!("close checkerboard{id}")).click();
    h.run_steps(2);
    assert!(h.state().graph().is_empty(), "the node is still there");

    // Through the bus, so one Ctrl+Z brings it back.
    h.state_mut().undo();
    assert_eq!(h.state().graph().len(), 1);
}

/// The header's `?` is in the tree, and hovering it puts `NodeDef::tooltip` there too.
///
/// The tooltip is what a `?` is for, and `on_hover_text` draws it as an ordinary egui
/// `Label` in a tooltip `Area` — so the text lands in the accessibility tree and a headless
/// test can read it. Several frames after the move: egui's `tooltip_delay` is half a second
/// and a kittest step is a quarter of one.
#[test]
fn hovering_the_headers_help_mark_shows_the_nodes_tooltip() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);

    let tooltip = supersilvia::nodes::find("checkerboard").unwrap().tooltip;
    assert!(
        h.query_by_label_contains(tooltip).is_none(),
        "the tooltip is up before anything was hovered"
    );

    h.get_by_label(&format!("help checkerboard{id}")).hover();
    h.run_steps(4);
    assert!(
        h.query_by_label_contains(tooltip).is_some(),
        "hovering the `?` showed no tooltip"
    );
}

/// Every node kind has something for its `?` to show.
///
/// The header draws the mark only where the definition has a tooltip, so a `?` that opens
/// on nothing is impossible — but a node whose tooltip was left empty would then quietly
/// lose the mark. Walking the registry is what keeps the two facts one fact.
#[test]
fn every_node_kind_has_a_tooltip_for_its_header_to_show() {
    let empty: Vec<&str> = supersilvia::nodes::REGISTRY
        .iter()
        .filter(|def| def.tooltip.trim().is_empty())
        .map(|def| def.slug)
        .collect();
    assert!(
        empty.is_empty(),
        "these draw no `?` on their header, having nothing to say: {}",
        empty.join(" ")
    );
}

/// A drag that starts on the `?` moves nothing.
///
/// The mark senses drags as well as clicks, unlike the close button beside it: the body
/// ground underneath is a drag handle, and egui gives a drag to the topmost widget that
/// wants one. Without that, pointing at the `?` and twitching would move the node.
#[test]
fn a_drag_on_the_help_mark_moves_nothing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);

    let before = h.state().graph().get(id).unwrap().pos;
    let at = h
        .get_by_label(&format!("help checkerboard{id}"))
        .rect()
        .center();
    drag_with(
        &mut h,
        at,
        at + egui::vec2(120.0, 40.0),
        egui::Modifiers::NONE,
    );
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        before,
        "a drag from the `?` moved the node"
    );
    // And a click on it is not a select either.
    click_with(&mut h, at, egui::Modifiers::NONE);
    assert!(
        h.state().graph().get(id).is_some(),
        "the node went somewhere"
    );
}

/// A collapsed node is its header, and it keeps the `?`.
///
/// A node with nothing but a title is the one most likely to be asked what it does.
#[test]
fn a_collapsed_node_keeps_its_help_mark() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut()
        .apply(Command::SetCollapsed {
            nodes: vec![id],
            collapsed: true,
        })
        .unwrap();
    h.run_steps(2);
    assert!(
        h.query_by_label(&format!("help checkerboard{id}"))
            .is_some()
    );
}

/// The `?` goes at the zoom the title goes, because a mark with no title beside it names
/// nothing.
#[test]
fn the_help_mark_leaves_with_the_title() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(2);
    assert!(
        h.query_by_label(&format!("help checkerboard{id}"))
            .is_some()
    );

    while h.state().canvas_transform().zoom > 0.4 {
        wheel(&mut h, -2.0);
    }
    h.run_steps(2);
    assert!(
        h.query_by_label(&format!("help checkerboard{id}"))
            .is_none(),
        "the `?` outlived the title it belongs to"
    );
    // The close button goes at the same zoom, and is the threshold this one is derived from.
    assert!(
        h.query_by_label(&format!("close checkerboard{id}"))
            .is_none()
    );
}

/// Every string the frame painted, read off the shapes rather than the accessibility tree.
///
/// A port label and a uniform number readout are `Painter::text`, not widgets: the tree
/// carries the port and the control, never the text drawn beside them, so a label that stops
/// painting is invisible to `query_by_label` and visible only here.
fn painted_text(h: &Harness<'_, App>) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().to_owned()),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// A port label leaves at the zoom the control on its row leaves, all the way down.
///
/// One threshold, `canvas::DETAIL_ZOOM`, because a node that draws every control and no
/// label reads as text that failed to paint rather than as a zoom level — and the paint is
/// the only place the difference shows, since both the port and the control stay in the
/// accessibility tree either way.
#[test]
fn a_port_label_leaves_at_the_zoom_its_control_does() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_SLEW);
    h.run_steps(2);

    let (mut with_both, mut with_neither) = (false, false);
    while h.state().canvas_transform().zoom > supersilvia::ui::canvas::MIN_ZOOM {
        wheel(&mut h, -0.5);
        h.step();
        let zoom = h.state().canvas_transform().zoom;
        // The control names itself with its value; the port names itself with its type, so
        // this matches the control alone.
        let control = h.query_by_label_contains("slew1.rise 10.00").is_some();
        let label = painted_text(&h).iter().any(|text| text == "Rise");
        assert_eq!(control, label, "at zoom {zoom} the row is half drawn");
        with_both |= control;
        with_neither |= !control;
    }
    assert!(with_both, "the sweep started above the threshold");
    assert!(with_neither, "and ended below it");
}

/// Every character the registry draws has a glyph in the font stack the app installs.
///
/// Not only icons. `⬓` is the half-height — worldspace's length unit, since the height is
/// exactly 2.0 — and it is the unit on a dozen number controls. It is in none of egui's four
/// faces, so it drew as `◻` on every transform node, which is exactly the sort of thing
/// nothing else catches. So this walks everything in a `NodeDef` that reaches the screen as
/// text: icons, labels, tooltips, port keys, control units, option choices.
///
/// **The oracle is `glyph_width`, not `has_glyph`.** `has_glyph` is
/// `resolve_face(c) != replacement_face`, and the replacement face is simply the first face
/// in the family carrying `◻` — Hack. So it answers "no" for every character Hack itself
/// provides, which is most symbols. `glyph_width` resolves the face and then asks it for the
/// glyph, and is 0.0 only when the character is genuinely in none of them.
#[test]
fn every_character_the_registry_draws_has_a_glyph() {
    let mut h = harness();
    h.step();

    let font = egui::FontId::monospace(14.0);
    let missing: Vec<String> = h.ctx.fonts_mut(|fonts| {
        let mut out = Vec::new();
        for def in supersilvia::nodes::REGISTRY {
            let mut texts: Vec<(&str, &str)> = vec![
                ("icon", def.icon),
                ("label", def.label),
                ("tooltip", def.tooltip),
            ];
            for p in def.inputs {
                texts.push(("input", p.key));
                texts.push(("input label", p.label));
                if let supersilvia::nodes::Control::Number { unit, .. } = p.control {
                    texts.push(("unit", unit));
                }
            }
            for p in def.outputs {
                texts.push(("output", p.key));
                texts.push(("output label", p.label));
            }
            for o in def.options {
                texts.push(("option", o.label));
                for (value, shown) in o.choices {
                    texts.push(("choice", value));
                    texts.push(("choice label", shown));
                }
            }
            for (what, text) in texts {
                let bad: Vec<String> = text
                    .chars()
                    .filter(|c| !c.is_ascii() && fonts.glyph_width(&font, *c) == 0.0)
                    .map(|c| format!("U+{:04X}", c as u32))
                    .collect();
                if !bad.is_empty() {
                    out.push(format!("{} {what} {text:?} — {}", def.slug, bad.join(" ")));
                }
            }
        }
        out
    });

    assert!(
        missing.is_empty(),
        "these draw as `◻`:\n  {}\n\
         A variation selector (U+FE0F) counts: epaint has no glyph for one and draws the\n\
         replacement, so paste the bare emoji. Anything else needs a face that has it — widen\n\
         `scripts/subset-fonts.py` and re-run it.",
        missing.join("\n  ")
    );
}

/// Right-click on a header inside a live selection opens that selection's menu.
///
/// The menu used to live on the node and act on the node, so right-clicking one of several
/// selected nodes silently narrowed the target to one — the entries looked the same and did
/// something else.
#[test]
fn a_header_in_a_selection_opens_the_selections_menu() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut().select_only(&ids);
    h.run_steps(2);

    let at = header_at(&h, ids[0]);
    right_click(&mut h, at);
    assert!(
        h.query_by_label_contains("2 selected").is_some(),
        "the header opened a single-node menu while two were selected"
    );

    // And it acts on both, in one undo step.
    let before = h.state().history().len();
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().graph().is_empty(), "it deleted only one");
    assert_eq!(h.state().history().len(), before + 1, "not one step");
}

/// `Reset controls` on the selection's menu puts every control back to its definition's
/// default, in one undo step.
#[test]
fn the_menus_reset_controls_puts_every_control_back() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut()
        .apply(Command::SetControl {
            node: ids[0],
            key: "frequency",
            value: supersilvia::graph::ControlValue::Float(23.0),
        })
        .unwrap();
    h.state_mut().select_only(&ids);
    h.run_steps(2);

    let at = header_at(&h, ids[0]);
    right_click(&mut h, at);
    let before = h.state().history().len();
    h.get_by_label("Reset controls").click();
    h.run_steps(2);

    assert_eq!(
        h.state().graph().get(ids[0]).unwrap().controls["frequency"],
        supersilvia::graph::ControlValue::Float(8.0),
    );
    assert_eq!(h.state().history().len(), before + 1, "not one step");
}

/// `Disconnect all` takes every cable touching the selection and leaves the nodes.
#[test]
fn the_menus_disconnect_all_takes_every_cable_off_the_selection() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(ids[0], "output"),
            to: PortRef::new(ids[1], "input"),
        })
        .expect("color into an Output");
    h.state_mut().select_only(&ids[..1]);
    h.run_steps(2);

    let at = header_at(&h, ids[0]);
    right_click(&mut h, at);
    let before = h.state().history().len();
    h.get_by_label("Disconnect all").click();
    h.run_steps(2);

    assert_eq!(h.state().graph().connections().len(), 0);
    assert_eq!(h.state().graph().len(), 2, "the nodes stayed");
    assert_eq!(h.state().history().len(), before + 1, "not one step");
}

// ---------------------------------------------------------------- the Nodes menu

/// A submenu opens on a delay and closes on a longer one. Between them sits the gesture
/// every menu without timers gets wrong: crossing other categories on the way to a submenu,
/// and cutting the diagonal out of one.
///
/// kittest's clock advances 0.25 s a frame, which straddles both delays: one frame is inside
/// the 0.2 s open and the 0.3 s close, two frames are past them.
#[test]
fn a_submenu_opens_on_a_delay_and_survives_the_pointer_leaving() {
    let mut h = harness();
    h.step();
    h.get_by_label("Nodes").click();
    h.run_steps(2);

    // One frame: the pointer crosses into the category and the 200 ms is armed. kittest's
    // clock advances 0.25 s a frame, which straddles both delays — one frame is inside them,
    // two are past.
    h.get_by_label_contains("Generate").hover();
    h.step();
    assert!(
        h.query_by_label("add checkerboard").is_none(),
        "the submenu opened the instant the pointer touched the category"
    );
    h.step();
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "the submenu never opened"
    );

    // Away, but not for long: the diagonal to the submenu passes over nothing at all.
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(Pos2::new(700.0, 500.0)));
    h.step();
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "it closed the moment the pointer left, which is the valley problem"
    );
    h.run_steps(2);
    assert!(
        h.query_by_label("add checkerboard").is_none(),
        "it never closed"
    );
}

/// The menu stands on its button, and a submenu stands beside its category. Geometry a test
/// can assert is worth having: the panel's height is computed rather than measured, so a row
/// gaining spacing would push the panel down over the button, where every click on it would
/// land on the last category instead.
#[test]
fn the_nodes_menu_stands_on_its_button() {
    let mut h = harness();
    h.step();
    h.get_by_label("Nodes").click();
    h.run_steps(2);

    let button = h.get_by_label("Nodes").rect();
    let last = h.get_by_label_contains("📺 Output").rect();
    assert!(
        last.max.y <= button.min.y + 1.0,
        "the menu hangs over its own button: last category to {}, button from {}",
        last.max.y,
        button.min.y
    );

    h.get_by_label_contains("Generate").hover();
    h.run_steps(2);
    let category = h.get_by_label_contains("Generate").rect();
    let entry = h.get_by_label("add checkerboard").rect();
    assert!(
        entry.min.x >= category.max.x,
        "the submenu covers the categories it opened from"
    );
    h.snapshot("nodes_menu");
}

/// Windows's keys: a menu opens with nothing selected, so `Up` is the *last* category. Then
/// `Right` goes in and `Enter` takes the entry.
#[test]
fn up_from_nothing_selected_is_the_last_category() {
    let mut h = harness();
    h.step();
    h.get_by_label("Nodes").click();
    h.run_steps(2);

    h.key_press(egui::Key::ArrowUp);
    h.step();
    h.key_press(egui::Key::ArrowRight);
    h.step();
    h.key_press(egui::Key::Enter);
    h.run_steps(2);

    let slugs: Vec<_> = h.state().graph().iter().map(|(_, n)| n.def.slug).collect();
    assert_eq!(
        slugs,
        vec!["output"],
        "Up from nothing selected must reach the last category, which is Output"
    );
}

/// Escape backs out of a submenu before it closes the menu, and closes it after.
#[test]
fn escape_leaves_the_submenu_before_it_leaves_the_menu() {
    let mut h = harness();
    h.step();
    h.get_by_label("Nodes").click();
    h.run_steps(2);
    // Down, Down: past Source to Generate, whose first entry is a known one.
    h.key_press(egui::Key::ArrowDown);
    h.step();
    h.key_press(egui::Key::ArrowDown);
    h.step();
    h.key_press(egui::Key::ArrowRight);
    h.step();
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "Right did not go into the submenu"
    );

    h.key_press(egui::Key::Escape);
    h.step();
    assert!(
        h.query_by_label("add checkerboard").is_none(),
        "Escape did not leave the submenu"
    );
    assert!(
        h.query_by_label_contains("Source").is_some(),
        "Escape closed the whole menu instead of the submenu"
    );

    h.key_press(egui::Key::Escape);
    h.step();
    assert!(
        h.query_by_label_contains("Source").is_none(),
        "a second Escape did not close the menu"
    );
    assert!(h.state().graph().is_empty(), "nothing was added");
}

// ---------------------------------------------------------------- the node browser

/// `/` opens the browser at the top of the canvas, typing narrows it, and Enter takes the
/// best match — the whole quake bar in one gesture.
#[test]
fn the_quake_bar_searches_and_enter_adds_the_best_match() {
    let mut h = harness();
    h.run_steps(2);

    h.key_press(egui::Key::Slash);
    h.run_steps(2);
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "the list is not up"
    );

    // The search field has focus, so text goes into it.
    h.input_mut().events.push(egui::Event::Text("out".into()));
    h.run_steps(2);
    assert!(
        h.query_by_label("add checkerboard").is_none(),
        "the query did not narrow the list"
    );

    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    let slugs: Vec<_> = h.state().graph().iter().map(|(_, n)| n.def.slug).collect();
    assert_eq!(slugs, vec!["output"], "Enter did not add the best match");
    assert!(
        h.query_by_label("add output").is_none(),
        "the browser stayed open after adding"
    );
}

/// Escape closes it, and nothing was added.
#[test]
fn escape_closes_the_browser() {
    let mut h = harness();
    h.run_steps(2);
    h.key_press(egui::Key::Slash);
    h.run_steps(2);
    assert!(h.query_by_label("add checkerboard").is_some());

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.query_by_label("add checkerboard").is_none(), "still open");
    assert!(h.state().graph().is_empty(), "Escape added a node");
}

/// Right-clicking the canvas offers the same list, and the node lands where the pointer was.
#[test]
fn right_clicking_the_canvas_puts_the_node_under_the_pointer() {
    let mut h = harness();
    h.run_steps(2);

    let at = Pos2::new(200.0, 300.0);
    right_click(&mut h, at);
    h.run_steps(2);
    h.input_mut()
        .events
        .push(egui::Event::Text("checker".into()));
    h.run_steps(2);
    h.get_by_label("add checkerboard").click();
    h.run_steps(2);

    let (_, node) = h.state().graph().iter().next().expect("a node");
    let world = h
        .state()
        .canvas_transform()
        .to_world(h.state().canvas_origin(), at);
    assert_eq!(node.def.slug, "checkerboard");
    assert!(
        (node.pos - world).length() < 1.0,
        "landed at {:?}, not at the pointer {world:?}",
        node.pos
    );
}

/// With several nodes selected, right-clicking the background opens their menu: the pointer
/// has no single node to be over then, and a right-click anywhere in the editor means
/// "these". The browser is what the background offers otherwise.
#[test]
fn the_background_carries_the_selections_menu_while_several_are_selected() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut().select_only(&ids);
    h.run_steps(2);

    right_click(&mut h, Pos2::new(200.0, 300.0));
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("2 selected").is_some(),
        "the background did not open the selection's menu"
    );
    assert!(
        h.query_by_label("add checkerboard").is_none(),
        "the background opened the browser over a live selection"
    );

    // And it acts on both, in one undo step, like the same menu from a header.
    let before = h.state().history().len();
    h.get_by_label("Delete").click();
    h.run_steps(2);
    assert!(h.state().graph().is_empty(), "it deleted only one");
    assert_eq!(h.state().history().len(), before + 1, "not one step");
}

/// With one node selected, right-clicking the background asks for a node rather than
/// opening that node's menu: one node's menu is on the node.
#[test]
fn the_background_offers_a_node_while_one_is_selected() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut().select_only(&ids);
    h.run_steps(2);

    right_click(&mut h, Pos2::new(200.0, 300.0));
    h.run_steps(2);
    assert!(
        h.query_by_label("Duplicate").is_none(),
        "the background still opens a menu that acts on the selection"
    );
    assert!(
        h.query_by_label("add checkerboard").is_some(),
        "the background did not offer a node"
    );
}

/// Right-clicking a node outside the selection makes it the selection, so the menu that
/// opens acts on what was clicked.
#[test]
fn a_header_outside_the_selection_takes_it_over() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    h.state_mut().select_only(&[ids[0]]);
    h.run_steps(2);

    let at = header_at(&h, ids[1]);
    right_click(&mut h, at);
    assert_eq!(h.state().canvas_selection(), vec![ids[1]]);
    assert!(
        h.query_by_label_contains("selected").is_none(),
        "one node should get a plain menu, with no count"
    );
}

// ---------------------------------------------------------------- the number control

/// One node's number control, read out of the graph rather than off the screen.
fn float(h: &Harness<'_, App>, id: supersilvia::graph::NodeId, key: &str) -> f32 {
    match h.state().graph().get(id).and_then(|n| n.controls.get(key)) {
        Some(supersilvia::graph::ControlValue::Float(v)) => *v,
        other => panic!("expected a number control, got {other:?}"),
    }
}

/// The gesture whose absence people notice fastest: until it existed there was no way to
/// enter an exact number at all, only to scrub towards one.
#[test]
fn clicking_a_number_control_types_an_exact_value() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);

    h.get_by_label_contains("checkerboard1.frequency 8").click();
    // Two: the click is one frame, and the field it asked for is drawn on the next.
    h.step();
    h.step();

    // The control is a text field for as long as the edit is open, and the scrub's own
    // readout is gone from the tree while it is.
    assert!(
        h.query_by_label_contains("checkerboard1.frequency 8")
            .is_none(),
        "the field takes the control over while it is open"
    );

    h.input_mut()
        .events
        .push(egui::Event::Text("37".to_string()));
    h.step();
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.step();

    let id = h.state().graph().iter().next().expect("one node").0;
    assert_eq!(
        h.state()
            .graph()
            .get(id)
            .and_then(|n| n.controls.get("frequency")),
        Some(&supersilvia::graph::ControlValue::Float(37.0)),
    );
}

/// `D` puts the value back; `R` puts the value *and* the range back. They differ only
/// because a range belongs to the instance, which is the whole reason both exist.
#[test]
fn r_resets_the_range_where_d_only_resets_the_value() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    h.state_mut()
        .apply(supersilvia::Command::SetRange {
            node: id,
            key: "frequency",
            range: supersilvia::graph::ControlRange {
                min: 1.0,
                max: 30.0,
                step: 1.0,
            },
        })
        .unwrap();
    h.state_mut()
        .apply(supersilvia::Command::SetControl {
            node: id,
            key: "frequency",
            value: supersilvia::graph::ControlValue::Float(20.0),
        })
        .unwrap();
    h.step();

    let has_range = |h: &Harness<'_, App>| {
        h.state()
            .graph()
            .get(id)
            .is_some_and(|n| n.values.contains_key("frequency"))
    };
    let value = |h: &Harness<'_, App>| match h
        .state()
        .graph()
        .get(id)
        .and_then(|n| n.controls.get("frequency"))
    {
        Some(supersilvia::graph::ControlValue::Float(v)) => *v,
        other => panic!("expected a float, got {other:?}"),
    };

    // Hover the control, since that is where the keys are read.
    let at = h
        .get_by_label_contains("checkerboard1.frequency 20")
        .rect()
        .center();
    for (key, expected_value, expected_range) in
        [(egui::Key::D, 8.0, true), (egui::Key::R, 8.0, false)]
    {
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.step();
        h.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.step();
        assert_eq!(value(&h), expected_value, "after {key:?}");
        assert_eq!(has_range(&h), expected_range, "after {key:?}");
    }
}

/// The keyboard's own scrub: one step per press, with the wheel's own multipliers, read
/// while the pointer is over the control because the canvas has no focus to give.
#[test]
fn the_arrow_keys_step_a_hovered_number_control() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    let at = h
        .get_by_label_contains("checkerboard1.frequency 8")
        .rect()
        .center();
    // `checkerboard.frequency` is 8 over 1..64 step 1, so a step is a whole number.
    for (key, modifiers, expected) in [
        (egui::Key::ArrowUp, egui::Modifiers::NONE, 9.0),
        (egui::Key::ArrowDown, egui::Modifiers::NONE, 8.0),
        (egui::Key::ArrowUp, egui::Modifiers::CTRL, 18.0),
        (egui::Key::ArrowDown, egui::Modifiers::CTRL, 8.0),
    ] {
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.step();
        h.input_mut()
            .events
            .push(egui::Event::ModifiersChanged(modifiers));
        h.input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        h.step();
        h.input_mut()
            .events
            .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
        assert_eq!(
            float(&h, id, "frequency"),
            expected,
            "after {key:?} with {modifiers:?}"
        );
    }
}

/// `Escape` mid-drag puts the value back — and takes the undo step the drag opened with it.
///
/// A scrub coalesces into one step, so setting the value back would leave a step in the ring
/// that restores nothing and an edit serial the title reads as unsaved work. The step is
/// dropped instead, which is why the ring is exactly as long afterwards as it was before.
#[test]
fn escape_mid_drag_restores_the_value_and_collapses_the_step() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    let steps_before = h.state().undo_len();

    let rect = h.get_by_label_contains("checkerboard1.frequency 8").rect();
    let from = rect.center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.input_mut().events.push(button(from, true));
    h.step();
    let to = from + egui::vec2(40.0, 0.0);
    h.input_mut().events.push(egui::Event::PointerMoved(to));
    h.step();
    assert_ne!(float(&h, id, "frequency"), 8.0, "the drag scrubbed nothing");
    assert_eq!(
        h.state().undo_len(),
        steps_before + 1,
        "a scrub is one step, however many frames it took"
    );

    // Still held: the key is read from the drag, not from a release.
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.input_mut().events.push(button(to, false));
    h.step();

    assert_eq!(
        float(&h, id, "frequency"),
        8.0,
        "the value did not come back"
    );
    assert_eq!(
        h.state().undo_len(),
        steps_before,
        "the abandoned drag left a step behind"
    );
    assert_eq!(h.state().history().len(), steps_before);
    // And the pointer is still down, so the frames left in the press move nothing.
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(to + egui::vec2(40.0, 0.0)));
    h.step();
    assert_eq!(float(&h, id, "frequency"), 8.0, "the drag scrubbed on");
}

/// The cursor lock and the Status box are Preferences answers and nowhere else: the View menu
/// holds what changes what the canvas shows.
#[test]
fn the_view_menu_has_no_cursor_lock_and_no_status_box() {
    let mut h = harness();
    h.step();
    h.get_by_label("View").click();
    h.run_steps(2);
    assert!(h.query_by_label("Costs").is_some(), "the View menu is open");
    assert!(h.query_by_label_contains("Lock").is_none());
    assert!(h.query_by_label_contains("Status box").is_none());
}

/// With the lock-cursor preference on, a drag reads `egui::Event::MouseMoved` — the raw,
/// relative motion `DeviceEvent::MouseMotion` delivers — rather than `PointerMoved`'s
/// absolute position. This is the platform-independent half of the pointer-capture feature:
/// whether or not a given compositor actually grants `CursorGrab::Locked` (kittest has no
/// OS cursor to grab in the first place), the value still has to move from raw motion alone,
/// because that is what a locked cursor leaves the widget with.
#[test]
fn locking_the_cursor_scrubs_from_raw_motion_with_no_pointer_moved() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    assert!(
        !h.state().preferences().lock_cursor_while_scrubbing,
        "off by default"
    );
    open_preferences(&mut h);
    preferences_tab(&mut h, "Editing");
    h.get_by_label("Lock the cursor while scrubbing").click();
    h.step();
    assert!(h.state().preferences().lock_cursor_while_scrubbing);
    // Closed again, so the window is not over the node the drag below starts on.
    h.get_by_label("Close window").click();
    h.run_steps(2);

    let rect = h.get_by_label_contains("checkerboard1.frequency 8").rect();
    let at = rect.center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.input_mut().events.push(button(at, true));
    // egui only calls a press a *drag* once it has moved further than its own click-vs-drag
    // threshold (`InputState::has_moved_too_much_for_a_click`, set from `PointerMoved` alone
    // — never from `MouseMoved`). This is the same handful of points of ordinary absolute
    // movement a real cursor covers before the app's first `drag_started` frame engages the
    // OS grab; small next to the big raw motion below, but enough to cross that threshold.
    let nudge = at + egui::vec2(10.0, 0.0);
    h.input_mut().events.push(egui::Event::PointerMoved(nudge));
    h.step();
    let after_nudge = float(&h, id, "frequency");

    // No further `PointerMoved` from here — only the relative motion a locked, invisible
    // cursor still delivers, at the same reported screen position every subsequent frame.
    // Once dragging has started, egui keeps `Response::dragged()` true from the button alone
    // — the threshold check above never re-arms itself mid-drag — so this frame's move has
    // to come from `i.pointer.motion()`, which only `MouseMoved` feeds.
    h.input_mut()
        .events
        .push(egui::Event::MouseMoved(egui::vec2(400.0, 0.0)));
    h.step();

    assert_ne!(
        float(&h, id, "frequency"),
        after_nudge,
        "raw motion alone should have scrubbed the value further, with no new cursor \
         position for Response::drag_delta to read"
    );

    h.input_mut().events.push(button(nudge, false));
    h.step();
}

/// The text is the value and the rest of the bar is the track: a press on the bar jumps
/// straight to that position, where a press on the number scrubs from where it stands.
#[test]
fn a_drag_on_the_track_jumps_to_that_position() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    let rect = h.get_by_label_contains("checkerboard1.frequency 8").rect();
    // Three quarters along a 1..64 track, clear of both the steppers and the number.
    let from = Pos2::new(rect.left() + rect.width() * 0.75, rect.center().y);
    drag_with(
        &mut h,
        from,
        from + egui::vec2(20.0, 0.0),
        egui::Modifiers::NONE,
    );
    let jumped = float(&h, id, "frequency");
    assert!(
        (jumped - 48.0).abs() <= 2.0,
        "a press on the track lands where it was pressed, got {jumped}"
    );

    // The same drag beginning on the number scrubs instead — four pixels a step, so twenty
    // pixels is five steps and nothing like a jump to the far end.
    h.state_mut()
        .apply(supersilvia::Command::SetControl {
            node: id,
            key: "frequency",
            value: supersilvia::graph::ControlValue::Float(8.0),
        })
        .unwrap();
    h.run_steps(2);
    // The pointer is still over the control from the drag above, so its tooltip — the range,
    // added to the hover text — is now in the tree too, carrying the same label as a
    // substring. The control itself is a `SpinButton`; the tooltip is a `Label`.
    let rect = h
        .get(
            egui_kittest::kittest::By::new()
                .role(egui::accesskit::Role::SpinButton)
                .label_contains("checkerboard1.frequency 8"),
        )
        .rect();
    drag_with(
        &mut h,
        rect.center(),
        rect.center() + egui::vec2(20.0, 0.0),
        egui::Modifiers::NONE,
    );
    assert_eq!(
        float(&h, id, "frequency"),
        13.0,
        "a drag that began on the number is a scrub, wherever it went"
    );
}

/// `Ctrl` on a stepper changes the *step* rather than taking ten of them. It is a range, so
/// it is document data: undoable, in the file, and shown by the range editor.
#[test]
fn ctrl_clicking_a_stepper_changes_the_step() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    let rect = h.get_by_label_contains("checkerboard1.frequency 8").rect();
    let plus = Pos2::new(rect.right() - 9.0, rect.center().y);
    let minus = Pos2::new(rect.left() + 9.0, rect.center().y);
    let step = |h: &Harness<'_, App>| {
        h.state()
            .graph()
            .get(id)
            .and_then(|n| n.values.get("frequency"))
            .and_then(supersilvia::graph::Value::range)
            .as_ref()
            .map(|r| r.step)
    };

    click_with(&mut h, plus, egui::Modifiers::CTRL);
    assert_eq!(step(&h), Some(10.0), "ten times coarser");
    assert_eq!(
        float(&h, id, "frequency"),
        8.0,
        "changing the quantum is not moving the value"
    );

    click_with(&mut h, minus, egui::Modifiers::CTRL);
    click_with(&mut h, minus, egui::Modifiers::CTRL);
    assert_eq!(step(&h), Some(0.1), "and a tenth twice over");

    // Without Ctrl the same button is still one step.
    click_with(&mut h, plus, egui::Modifiers::NONE);
    assert_eq!(float(&h, id, "frequency"), 8.1);

    // And the editor shows it rather than rounding it away: the step's field is bounded by
    // what a step can be, where the other two are bounded by the definition's ends.
    right_click(&mut h, Pos2::new(rect.center().x, rect.center().y));
    h.run_steps(3);
    assert_eq!(step(&h), Some(0.1), "the range editor swallowed the step");
}

/// Right-click a number control and wait out the range editor's sizing pass.
fn open_range_editor(h: &mut Harness<'_, App>, name: &str) {
    let at = number_rect(h, name).center();
    right_click(h, at);
    h.step();
}

/// Whether the range editor is up, read off its header, which is a label and so carries its
/// text as its value.
fn range_editor_open(h: &Harness<'_, App>) -> bool {
    h.query_all(
        egui_kittest::kittest::By::new()
            .predicate(|n| n.value().is_some_and(|v| v.starts_with("declared "))),
    )
    .next()
    .is_some()
}

/// A click on a default in the range editor copies it into its field and leaves the editor
/// open: a click on a widget inside a popup is a click inside the popup.
#[test]
fn clicking_a_default_in_the_range_editor_applies_it_and_keeps_the_editor() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    h.state_mut()
        .apply(supersilvia::Command::SetControl {
            node: id,
            key: "frequency",
            value: supersilvia::graph::ControlValue::Float(20.0),
        })
        .unwrap();
    h.step();

    open_range_editor(&mut h, "checkerboard1.frequency");
    assert!(range_editor_open(&h), "the right-click opened the editor");

    // `checkerboard.frequency` defaults to 8, and the value row's default is the only 8 in
    // the editor: min, step and max default to 1, 1 and 64.
    let default = h
        .get(egui_kittest::kittest::By::new().value("8"))
        .rect()
        .center();
    click_at(&mut h, default);

    assert_eq!(float(&h, id, "frequency"), 8.0, "the default was applied");
    assert!(
        range_editor_open(&h),
        "the editor closed on a click inside it"
    );
}

/// The range editor's fields are typed rather than dragged, and a click elsewhere in the
/// editor commits what was typed, on the frame it lands.
#[test]
fn a_range_editor_field_is_typed_and_a_click_elsewhere_commits_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    let max = |h: &Harness<'_, App>| {
        h.state()
            .graph()
            .get(id)
            .and_then(|n| n.values.get("frequency"))
            .and_then(supersilvia::graph::Value::range)
            .map(|r| r.max)
    };

    open_range_editor(&mut h, "checkerboard1.frequency");
    h.get_by_label_contains("checkerboard1.frequency.max")
        .click();
    h.step();
    key(&mut h, egui::Key::End);
    key(&mut h, egui::Key::Backspace);
    key(&mut h, egui::Key::Backspace);
    type_text(&mut h, "3x2");
    assert_eq!(max(&h), None, "nothing is committed while typing");

    let header = h
        .get(
            egui_kittest::kittest::By::new()
                .predicate(|n| n.value().is_some_and(|v| v.starts_with("declared "))),
        )
        .rect()
        .center();
    click_at(&mut h, header);
    assert_eq!(
        max(&h),
        Some(32.0),
        "the letter never landed, and the click committed"
    );
    assert!(range_editor_open(&h), "a click inside the editor keeps it");
}

/// The Text node's size is a plain number field: a newline or a letter never lands in it, and
/// it commits once, on Enter, clamped into silvia's 8 to 512.
#[test]
fn a_number_option_takes_digits_and_commits_once_clamped() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Source", "add text"));
    let id = h.state().graph().iter().next().expect("one node").0;
    let size = |h: &Harness<'_, App>| {
        h.state()
            .graph()
            .get(id)
            .and_then(|n| n.options.get("size").cloned())
    };
    let undo = h.state().undo_len();

    h.get_by_label_contains("text1.size").click();
    h.step();
    key(&mut h, egui::Key::End);
    type_text(&mut h, "x5");
    h.input_mut().events.push(egui::Event::Paste("1\n".into()));
    h.step();
    assert!(
        h.query_by_label_contains("text1.size 6451").is_some(),
        "digits only in the draft"
    );
    assert_eq!(
        size(&h).as_deref(),
        Some("64"),
        "nothing committed while typing"
    );

    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(
        size(&h).as_deref(),
        Some("512"),
        "clamped to silvia's largest"
    );
    assert_eq!(h.state().undo_len(), undo + 1, "one commit, one step");
}

/// A key pressed alone, with nothing held.
fn key(h: &mut Harness<'_, App>, key: egui::Key) {
    h.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
}

/// Text typed into whatever has the keyboard.
fn type_text(h: &mut Harness<'_, App>, text: &str) {
    h.input_mut()
        .events
        .push(egui::Event::Text(text.to_string()));
    h.step();
}

/// Click checkerboard1's frequency to type into it, and wait for the field it asked for.
fn type_into_frequency(h: &mut Harness<'_, App>) {
    h.get_by_label_contains("checkerboard1.frequency 8").click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("checkerboard1.frequency 8")
            .is_none(),
        "the field took the control over"
    );
}

/// A wheel notch over a number scrubs it and is spent there: the canvas under it does not
/// also zoom.
#[test]
fn the_wheel_over_a_number_scrubs_it_and_leaves_the_zoom() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    let zoom = h.state().canvas_transform().zoom;

    let at = number_rect(&h, "checkerboard1.frequency").center();
    scroll_at(
        &mut h,
        at,
        egui::MouseWheelUnit::Line,
        egui::Vec2::new(0.0, 1.0),
    );
    h.run_steps(4);

    assert!(float(&h, id, "frequency") > 8.0, "the notch scrubbed up");
    assert_eq!(
        h.state().canvas_transform().zoom,
        zoom,
        "and the canvas did not zoom on it"
    );
}

/// `[` and `]` put a hovered number at its own ends.
#[test]
fn the_brackets_send_a_hovered_number_to_its_ends() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    // `checkerboard.frequency` runs 1 to 64.
    for (bracket, end) in [
        (egui::Key::CloseBracket, 64.0),
        (egui::Key::OpenBracket, 1.0),
    ] {
        let at = number_rect(&h, "checkerboard1.frequency").center();
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.step();
        key(&mut h, bracket);
        h.step();
        assert_eq!(float(&h, id, "frequency"), end, "after {bracket:?}");
    }
}

/// The arrows step the value while it is being typed, and the field stays open on the new
/// value; `Escape` then closes it on the value the arrow left.
#[test]
fn the_arrows_step_a_number_while_it_is_typed() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    type_into_frequency(&mut h);
    key(&mut h, egui::Key::ArrowUp);
    h.step();
    assert_eq!(float(&h, id, "frequency"), 9.0, "up is a step");
    assert!(
        h.query_by_label_contains("checkerboard1.frequency 9")
            .is_none(),
        "and the field is still open"
    );

    key(&mut h, egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(float(&h, id, "frequency"), 9.0, "Escape keeps the step");
    assert!(
        h.query_by_label_contains("checkerboard1.frequency 9")
            .is_some(),
        "and closes the field"
    );
}

/// `Escape` abandons what was typed.
#[test]
fn escape_abandons_a_typed_number() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    type_into_frequency(&mut h);
    type_text(&mut h, "37");
    key(&mut h, egui::Key::Escape);
    h.run_steps(2);

    assert_eq!(
        float(&h, id, "frequency"),
        8.0,
        "the typed 37 was abandoned"
    );
    assert!(
        h.query_by_label_contains("checkerboard1.frequency 8")
            .is_some(),
        "and the control is back"
    );
}

/// A click elsewhere commits what was typed, as `Enter` does.
#[test]
fn a_click_elsewhere_commits_a_typed_number() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    type_into_frequency(&mut h);
    type_text(&mut h, "37");
    let elsewhere = header_at(&h, id);
    click_at(&mut h, elsewhere);

    assert_eq!(float(&h, id, "frequency"), 37.0, "the blur committed 37");
}

/// Text that is not a number leaves the value where it was rather than zeroing it.
#[test]
fn a_typed_number_that_does_not_parse_leaves_the_value() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;
    let steps = h.state().undo_len();

    type_into_frequency(&mut h);
    type_text(&mut h, "abc");
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);

    assert_eq!(float(&h, id, "frequency"), 8.0, "abc left 8 alone");
    assert_eq!(h.state().undo_len(), steps, "and is not an edit");
}

/// `Shift` scrubs a tenth of a step per four points, carrying the remainder from frame to
/// frame so a fine drag moves at all, and drops the remainder when `Shift` is let go so the
/// value does not jump by what fine mode had saved up.
#[test]
fn a_shift_drag_carries_its_remainder_and_drops_it_with_shift() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().next().expect("one node").0;

    let shift = egui::Modifiers {
        shift: true,
        ..Default::default()
    };
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut at = number_rect(&h, "checkerboard1.frequency").center();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.input_mut().events.push(button(at, true));
    h.step();
    let mut move_by = |h: &mut Harness<'_, App>, dx: f32| {
        at += egui::vec2(dx, 0.0);
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.step();
        at
    };

    // Past egui's click distance, unshifted, so the drag has begun.
    move_by(&mut h, 8.0);
    let began = float(&h, id, "frequency");

    // Six points under Shift is 0.15 of a step, which rounds away alone: three of them move
    // nothing, and the fourth crosses half a step.
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(shift));
    for _ in 0..3 {
        move_by(&mut h, 6.0);
    }
    assert_eq!(
        float(&h, id, "frequency"),
        began,
        "0.45 of a step is not a step"
    );
    move_by(&mut h, 6.0);
    assert_eq!(
        float(&h, id, "frequency"),
        began + 1.0,
        "the carried remainder made a step"
    );

    // 0.45 saved up again, then Shift let go: one point unshifted is a quarter step, which
    // moves nothing on its own and would have made a step on top of the savings.
    for _ in 0..3 {
        move_by(&mut h, 6.0);
    }
    h.input_mut()
        .events
        .push(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    let last = move_by(&mut h, 1.0);
    assert_eq!(
        float(&h, id, "frequency"),
        began + 1.0,
        "letting go of Shift dropped the remainder"
    );

    h.input_mut().events.push(button(last, false));
    h.step();
}

/// `Alt` + click on an action's button learns a MIDI binding for it and does not press it.
#[test]
fn alt_clicking_an_action_button_learns_it_and_fires_nothing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add button"));
    let id = h.state().graph().iter().next().expect("one node").0;
    let press = PortRef::new(id, "press");
    let trigger = PortRef::new(id, "trigger");

    let at = h.get_by_label("button1.press").rect().center();
    alt_click_at(&mut h, at);

    assert_eq!(
        h.state().midi_learning(),
        Some(press.into()),
        "the button is waiting for a message"
    );
    assert!(
        h.query_by_label("button1.press (learning MIDI)").is_some(),
        "and says so"
    );
    assert!(h.query_by_label("MIDI").is_none(), "with the window shut");
    let mut seen = Vec::new();
    for _ in 0..4 {
        h.step();
        seen.extend(h.state().edges(trigger).iter().map(|e| e.edge));
    }
    assert!(seen.is_empty(), "and it never fired: {seen:?}");
}

// ---------------------------------------------------------------- action buttons

/// A tap on an action input's button. The gesture that cannot be tested below the UI,
/// because what `tick` sees is a *level* and what a hand does is a click: press and release
/// inside one frame. Reading only "is the pointer down on it" loses that entirely — the level
/// never rises and the gate never opens.
#[test]
fn clicking_an_action_button_fires_a_gate() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ("Control", "add button"));
    let id = h.state().graph().iter().next().expect("one node").0;
    let trigger = supersilvia::graph::PortRef::new(id, "trigger");

    h.get_by_label("button1.press").click();
    // Gathered over several frames rather than asserted per frame: how long the pointer is
    // reported down is the harness's business, and what matters is what came out of it.
    let mut seen = Vec::new();
    for _ in 0..8 {
        h.step();
        seen.extend(h.state().edges(trigger).iter().map(|e| e.edge));
    }
    assert_eq!(
        seen,
        vec![supersilvia::nodes::Edge::Down, supersilvia::nodes::Edge::Up],
        "a tap is one gate opening and closing — not none, and not one per frame"
    );
}

// ------------------------------------------------------------------ preferences

/// The Recent submenu lists the projects that have been opened, and clicking one opens it.
#[test]
fn the_recent_submenu_reopens_a_project() {
    let dir = std::env::temp_dir().join(format!("ssw-ui-recent-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let root = dir.join("friday");
    let elsewhere = dir.join("saturday");

    let mut source = App::headless();
    source.new_project(root.clone());
    source
        .apply(Command::AddNode {
            slug: "checkerboard",
            at: Pos2::ZERO,
            workspace: source.graph().default_workspace(),
        })
        .unwrap();
    source
        .apply(Command::AddNode {
            slug: "output",
            at: Pos2::ZERO,
            workspace: source.graph().default_workspace(),
        })
        .unwrap();
    source.save_project().unwrap();

    let mut h = harness();
    h.step();
    h.state_mut().open_project(root.clone());
    h.step();
    assert_eq!(h.state().graph().len(), 2);
    assert_eq!(h.state().preferences().recent, std::slice::from_ref(&root));

    // Away from that project, so reopening it is visible.
    h.state_mut().new_project(elsewhere);
    h.step();
    assert!(h.state().graph().is_empty());

    h.get_by_label("Project").click();
    h.step();
    h.get_by_label_contains("Recent").click();
    h.step();
    h.get_by_label("friday").click();
    h.step();

    assert_eq!(
        h.state().graph().len(),
        2,
        "clicking the entry opened the project"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Project ▸ Save writes the folder the project should have: a manifest and one file per
/// workspace.
#[test]
fn the_project_menu_saves_the_folder() {
    let dir = std::env::temp_dir().join(format!("ssw-ui-save-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let root = dir.join("friday");

    let mut h = harness();
    h.step();
    // Through the handler rather than the dialog: the folder picker is a thread and a
    // portal round trip, and none of that is what could break here.
    h.state_mut().new_project(root.clone());
    h.step();
    assert!(root.join("project.ssp").is_file(), "the project file");

    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    assert!(h.state().dirty(), "two nodes since the save");

    h.get_by_label("Project").click();
    h.step();
    // The label carries the shortcut, because the menu shows it.
    h.get_by_label("Save Ctrl+S").click();
    h.step();

    assert!(!h.state().dirty(), "Save cleared it");
    assert_eq!(h.state().file_status(), "saved friday");
    let workspaces: Vec<String> = std::fs::read_dir(root.join("workspaces"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(workspaces, ["Workspace 1.ssw"], "one file per workspace");

    let (_, graph, warnings) = supersilvia::project::Project::open(root).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(graph.len(), 2, "and both nodes are in it");
    std::fs::remove_dir_all(&dir).ok();
}

/// The unsaved-edits confirm is the one dialog, and it is modal.
///
/// It is an `egui::Modal`, so it draws at `Order::Foreground` — which is where the Nodes
/// menu and the browser draw, and an `egui::Window`'s `Order::Middle` let a list of nodes
/// cover the question. Both lists are closed while it stands, `n` included, and its backdrop
/// takes the clicks the canvas underneath would otherwise answer.
#[test]
fn the_confirm_closes_the_library_and_the_canvas_under_it_is_deaf() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    assert!(h.state().dirty(), "a node since the last save");

    h.get_by_label("Nodes").click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Generate").is_some(),
        "the Nodes menu is open over the canvas"
    );

    h.get_by_label("Project").click();
    h.step();
    h.get_by_label_contains("New project").click();
    h.run_steps(2);

    assert!(h.query_by_label("Discard").is_some(), "the confirm is up");
    assert!(
        h.query_by_label_contains("Generate").is_none(),
        "and it took the Nodes menu with it"
    );

    h.key_press(egui::Key::N);
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Generate").is_none(),
        "the key that opens the menu cannot open it under the question"
    );

    // The close button is drawn and in the tree; the backdrop is what the click lands on.
    h.get_by_label(&format!("close checkerboard{id}")).click();
    h.run_steps(2);
    assert_eq!(
        h.state().graph().len(),
        1,
        "a click under the modal reached the canvas"
    );

    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Discard").is_none(),
        "Cancel took the question away"
    );
    assert_eq!(h.state().graph().len(), 1, "and changed nothing");
}

/// Every file and folder under `root` made read-only, or writable again.
#[cfg(unix)]
fn read_only(root: &std::path::Path, locked: bool) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = root.is_dir();
    if dir && !locked {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    if dir {
        for entry in std::fs::read_dir(root).unwrap() {
            read_only(&entry.unwrap().path(), locked);
        }
    }
    let mode = match (dir, locked) {
        (true, true) => 0o555,
        (true, false) => 0o755,
        (false, true) => 0o444,
        (false, false) => 0o644,
    };
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// Whether the last frame told the window to close.
fn closed(h: &Harness<'_, App>) -> bool {
    h.output()
        .viewport_output
        .values()
        .any(|v| v.commands.contains(&egui::ViewportCommand::Close))
}

/// A Save the confirm offers that does not save leaves the question up with the reason on
/// it, and does none of what it stood in front of — Quit, Open project…, a Recent project
/// or New project… — so the edits are still on screen and the answer is asked again.
#[cfg(unix)]
#[test]
fn a_save_that_fails_under_the_confirm_keeps_the_question_and_the_edits() {
    let dir = std::env::temp_dir().join(format!("ssw-ui-failed-save-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let root = dir.join("friday");
    let other = dir.join("saturday");

    let mut h = harness();
    h.step();
    // Saturday first, so it is on the Recent list behind Friday.
    h.state_mut().new_project(other.clone());
    h.state_mut().new_project(root.clone());
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    read_only(&root, true);

    let asks: [&[&str]; 4] = [
        &["Quit"],
        &["Open project…"],
        &["Recent", "saturday"],
        &["New project…"],
    ];
    for path in asks {
        h.get_by_label("Project").click();
        h.run_steps(2);
        for label in path {
            h.get_by_label_contains(label).click();
            h.run_steps(2);
        }
        assert!(h.query_by_label("Discard").is_some(), "{path:?} asks first");

        h.get_by_label("Save").click();
        h.step();
        assert!(!closed(&h), "{path:?}: a failed save does not quit");
        h.run_steps(2);
        assert!(
            h.query_by_label("Discard").is_some(),
            "{path:?}: the question is still up"
        );
        // Twice: under the question, and in the toast every failure says itself in.
        assert_eq!(
            h.query_all_by_label_contains("save failed").count(),
            2,
            "{path:?}: and says why"
        );
        assert!(
            h.state()
                .toast()
                .is_some_and(|t| t.starts_with("save failed")),
            "{path:?}: in the toast too"
        );
        assert!(h.state().dirty(), "{path:?}: the edits are unsaved");
        assert_eq!(h.state().graph().len(), 1, "{path:?}: and on screen");
        assert_eq!(h.state().project().root(), root, "{path:?}: in friday");

        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.query_by_label("Discard").is_none(), "{path:?}: Cancel");
    }

    // Asked again with the folder writable, Save saves and the Recent project opens.
    read_only(&root, false);
    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label_contains("Recent").click();
    h.run_steps(2);
    h.get_by_label_contains("saturday").click();
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("Discard").is_none(),
        "the question is answered"
    );
    assert_eq!(h.state().project().root(), other, "and saturday is open");
    let (_, graph, _) = supersilvia::project::Project::open(root).unwrap();
    assert_eq!(graph.len(), 1, "with friday's edit on disk");
    std::fs::remove_dir_all(&dir).ok();
}

/// The frame-pacing readout is a preference: it is app state, not graph data, so it goes
/// nowhere near the command bus.
#[test]
fn the_frame_pacing_toggle_is_a_preference_and_not_a_command() {
    let mut h = harness();
    h.step();
    assert!(!h.state().preferences().show_status_box);

    open_preferences(&mut h);
    preferences_tab(&mut h, "Performance");
    h.get_by_label("Show the Status box").scroll_to_me();
    h.run_steps(10);
    h.get_by_label("Show the Status box").click();
    h.run_steps(2);
    assert!(
        h.query_all_by_label("Status box")
            .any(|n| n.accesskit_node().role() == egui::accesskit::Role::Window),
        "the box is open"
    );

    assert!(h.state().preferences().show_status_box);
    assert!(h.state().history().is_empty(), "no command was issued");
    assert!(!h.state().can_undo(), "and there is nothing to undo");
}

/// **A window covers the Nodes button**, which is canvas furniture and not a window.
///
/// The button lives in an `Area` so its menu can stand on it, and an `Area` at
/// `Order::Foreground` paints over an `egui::Window`, which is `Order::Middle`. So the
/// button was drawn on top of the Status box and the MIDI window wherever they met.
/// `Order::Middle` is not the fix — same-order layers fall back to insertion order and the
/// button still won — `Order::Background` is: above the canvas it belongs to, below anything
/// floating over it.
///
/// The Preferences window rather than the Status box, whose every line is a live timing: a
/// snapshot of one would differ from itself between runs.
#[test]
fn a_window_dragged_over_the_nodes_button_covers_it() {
    let mut h = harness();
    h.step();
    open_preferences(&mut h);

    // Down onto the button, by the window's title bar. It opens over the top of the canvas
    // and the button is at the bottom, so the two do not meet until one is moved.
    let button = h.get_by_label("Nodes").rect();
    let window = h.get_by_label("Preferences").rect();
    let grip = Pos2::new(window.center().x, window.min.y + 8.0);
    drag_with(
        &mut h,
        grip,
        Pos2::new(grip.x, grip.y + (button.center().y - window.max.y) + 30.0),
        egui::Modifiers::NONE,
    );
    h.run_steps(2);
    assert!(
        h.get_by_label("Preferences")
            .rect()
            .contains(button.center()),
        "the window is not over the button, so this proves nothing about either"
    );
    h.snapshot("a_window_covers_the_nodes_button");
}

/// The overlay's GPU line is drawn only where there is a reading.
///
/// A headless app has no renderer and therefore no query, and the honest answer to "how much
/// GPU is this costing" is silence: a `gpu 0.0 ms` would read as an idle GPU rather than as
/// an absent measurement.
#[test]
fn the_overlay_shows_no_gpu_line_without_a_reading() {
    let mut h = harness();
    h.step();
    open_status_box(&mut h);

    assert!(
        h.query_by_label_contains("p99").is_some(),
        "the overlay is on screen"
    );
    assert!(
        h.query_by_label_contains("gpu").is_none(),
        "and it says nothing about the GPU, because nothing measured one"
    );
}

// ---------------------------------------------------------------- tabs and the project tab

/// Click a tab by the name the tree carries, which is what `Ctrl+N` reaches too.
///
/// Two frames: the bar is drawn before its actions are answered, so the tree catches up on
/// the frame after the one the click landed on.
fn click_tab(h: &mut Harness<'_, App>, name: &str) {
    h.get_by_label(name).click();
    h.run_steps(2);
}

/// `Ctrl+<n>`: the tab in that position, the project tab first.
fn tab_key(h: &mut Harness<'_, App>, key: egui::Key) {
    h.input_mut().events.push(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(2);
}

/// A second workspace, added the way a person adds one: the `+` at the end of the bar.
fn add_workspace(h: &mut Harness<'_, App>) {
    h.get_by_label("+").click();
    h.run_steps(2);
    h.get_by_label("Video").click();
    h.run_steps(2);
}

/// A second workspace with **nothing on it**, for the tests that are about a node they put
/// there themselves.
///
/// The `+` tab seeds a video workspace with a Main Input and an Output, which is the right
/// thing for a hand and the wrong thing for a test whose subject is one cable: two more nodes
/// and a second Output would be scenery in every assertion. The gesture itself is covered by
/// `the_plus_tab_adds_a_workspace_opens_it_and_shows_it`.
fn add_empty_workspace(h: &mut Harness<'_, App>) {
    let name = format!("Workspace {}", h.state().graph().workspaces().len() + 1);
    h.state_mut()
        .apply(Command::AddWorkspace {
            name,
            kind: supersilvia::graph::WorkspaceKind::Video,
            layout: supersilvia::graph::LayoutMode::default(),
            seed: supersilvia::command::Seed::Empty,
        })
        .expect("a workspace can always be added");
    let added = h
        .state()
        .graph()
        .workspaces()
        .last()
        .expect("just added")
        .id;
    h.state_mut().open_workspace(added);
    h.run_steps(2);
}

#[test]
fn the_plus_tab_adds_a_workspace_opens_it_and_shows_it() {
    let mut h = harness();
    h.step();
    assert_eq!(h.state().graph().workspaces().len(), 1);

    add_workspace(&mut h);

    assert_eq!(h.state().graph().workspaces().len(), 2);
    let second = h.state().graph().workspaces()[1].id;
    assert_eq!(h.state().active(), Active::Workspace(second));
    assert!(h.state().open_workspaces().contains(&second));
    assert!(
        h.query_by_label("tab Workspace 2").is_some(),
        "and it has a tab"
    );
}

/// **A new tab counts up until its name is free**, rather than naming itself after how many
/// workspaces there are.
///
/// The count is where to start and not the answer. Delete Workspace 2 of three and the
/// count says three, which is a tab already on the bar; rename one to `Workspace 7` and the
/// trap is set further out. Two tabs with one name is a name that names neither.
#[test]
fn a_new_tab_counts_past_a_name_that_is_taken() {
    let mut h = harness();
    h.step();
    add_workspace(&mut h);
    add_workspace(&mut h);
    let names = |h: &Harness<'_, App>| -> Vec<String> {
        h.state()
            .graph()
            .workspaces()
            .iter()
            .map(|w| w.name.clone())
            .collect()
    };
    assert_eq!(names(&h), ["Workspace 1", "Workspace 2", "Workspace 3"]);

    // The middle one deleted, so the count is two and `Workspace 3` is taken.
    let second = h.state().graph().workspaces()[1].id;
    h.state_mut()
        .apply(Command::RemoveWorkspace(second))
        .expect("a workspace can be removed");
    h.run_steps(2);
    add_workspace(&mut h);
    assert_eq!(
        names(&h),
        ["Workspace 1", "Workspace 3", "Workspace 4"],
        "the new tab took a name that was already on the bar"
    );

    // And it counts past a name a person chose, not only past one it made itself: three
    // workspaces named 5, 3 and 4 start the search at four and have to walk to six.
    let first = h.state().graph().workspaces()[0].id;
    h.state_mut()
        .apply(Command::RenameWorkspace {
            id: first,
            name: "Workspace 5".into(),
        })
        .expect("a workspace can be renamed");
    h.run_steps(2);
    add_workspace(&mut h);
    assert_eq!(
        names(&h),
        ["Workspace 5", "Workspace 3", "Workspace 4", "Workspace 6"],
        "a name the person chose is a name a new tab has to count past"
    );
}

/// The shortcut and the click are the same gesture, which is what the `widget_info` on every
/// tab is for.
#[test]
fn ctrl_2_and_clicking_the_tab_activate_the_same_workspace() {
    let mut h = harness();
    h.step();
    // Held, so the playhead's time on the strip reads the same in both trees.
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    let first = h.state().graph().workspaces()[0].id;
    add_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;

    click_tab(&mut h, "tab Workspace 1");
    assert_eq!(h.state().active(), Active::Workspace(first));

    // Ctrl+2 is the second tab, and the project tab is the first.
    tab_key(&mut h, egui::Key::Num2);
    assert_eq!(h.state().active(), Active::Workspace(first));
    tab_key(&mut h, egui::Key::Num3);
    assert_eq!(h.state().active(), Active::Workspace(second));

    let by_key: Vec<String> = tree_labels(&h);
    click_tab(&mut h, "tab Workspace 1");
    click_tab(&mut h, "tab Workspace 2");
    assert_eq!(h.state().active(), Active::Workspace(second));
    assert_eq!(tree_labels(&h), by_key, "the tree says the same either way");

    tab_key(&mut h, egui::Key::Num1);
    assert_eq!(h.state().active(), Active::Project, "Ctrl+1 is the project");
}

/// Every name in the accessibility tree, sorted. What "the same state" means to a test.
fn tree_labels(h: &Harness<'_, App>) -> Vec<String> {
    use egui_kittest::kittest::NodeT as _;
    let mut labels: Vec<String> = h
        .root()
        .children_recursive()
        .filter_map(|node| node.accesskit_node().label())
        .collect();
    labels.sort();
    labels
}

#[test]
fn a_node_lands_on_the_active_workspace_and_is_absent_from_the_other() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    assert!(h.query_by_label("checkerboard1").is_some());

    // A new video tab is born holding a Main Input and an Output, so the ids the node added
    // below gets are past those two.
    add_workspace(&mut h);
    let seeded = h.state().graph().len();
    assert_eq!(seeded, 3, "the first node, plus the new tab's own two");
    assert!(
        h.query_by_label("checkerboard1").is_none(),
        "the first workspace's node is not on the second"
    );
    add_node(&mut h, ADD_OUTPUT);
    assert!(h.query_by_label("output4").is_some());

    click_tab(&mut h, "tab Workspace 1");
    assert!(h.query_by_label("checkerboard1").is_some());
    assert!(
        h.query_by_label("output4").is_none(),
        "and the second's node is not on the first"
    );
    assert_eq!(
        h.state().graph().len(),
        seeded + 1,
        "all of them are in the one graph"
    );
}

/// `Ctrl+T` is the same gesture as the `+` tab.
#[test]
fn ctrl_t_adds_a_video_workspace_and_shows_it() {
    let mut h = harness();
    h.step();
    tab_key(&mut h, egui::Key::T);
    assert_eq!(h.state().graph().workspaces().len(), 2);
    assert_eq!(
        h.state().active(),
        Active::Workspace(h.state().graph().workspaces()[1].id)
    );
}

/// Every workspace's name, in project order.
fn workspace_names(h: &Harness<'_, App>) -> Vec<String> {
    h.state()
        .graph()
        .workspaces()
        .iter()
        .map(|w| w.name.clone())
        .collect()
}

/// Right-click a tab and choose an entry on its menu.
fn tab_menu(h: &mut Harness<'_, App>, tab: &str, entry: &str) {
    let at = h.get_by_label(tab).rect().center();
    right_click(h, at);
    h.get_by_label(entry).click();
    h.run_steps(2);
}

/// **Duplicate on a tab's menu** copies the workspace beside it as `<name> copy`, with its
/// nodes, opens the copy and shows it: one undo step.
#[test]
fn duplicate_on_a_tabs_menu_copies_the_workspace_beside_it_and_shows_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_empty_workspace(&mut h);
    click_tab(&mut h, "tab Workspace 1");
    let undo = h.state().undo_len();

    tab_menu(&mut h, "tab Workspace 1", "Duplicate");

    assert_eq!(
        workspace_names(&h),
        ["Workspace 1", "Workspace 1 copy", "Workspace 2"]
    );
    let copy = h.state().graph().workspaces()[1].id;
    assert_eq!(h.state().active(), Active::Workspace(copy));
    assert!(h.query_by_label("tab Workspace 1 copy").is_some());
    assert!(
        h.query_by_label("checkerboard2").is_some(),
        "the copy's node, on the copy's canvas"
    );
    assert_eq!(h.state().undo_len(), undo + 1);

    h.state_mut().undo();
    h.run_steps(2);
    assert_eq!(workspace_names(&h), ["Workspace 1", "Workspace 2"]);
}

/// The workspace card's Duplicate is the same gesture.
#[test]
fn duplicate_on_a_workspace_card_is_the_same_gesture() {
    let mut h = harness();
    h.step();
    click_tab(&mut h, "tab project");
    h.get_by_label("duplicate Workspace 1").click();
    h.run_steps(2);
    assert_eq!(workspace_names(&h), ["Workspace 1", "Workspace 1 copy"]);
    assert_eq!(
        h.state().active(),
        Active::Workspace(h.state().graph().workspaces()[1].id)
    );
}

/// `Ctrl+Tab`, with or without `Shift`, as egui-winit reports it on Linux: `command` beside
/// `ctrl`. Without it a `Ctrl+Shift+Tab` reads to egui's focus as `Shift+Tab`, which no
/// keyboard here sends.
fn ctrl_tab(h: &mut Harness<'_, App>, back: bool) {
    let ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
    let modifiers = if back {
        ctrl | egui::Modifiers::SHIFT
    } else {
        ctrl
    };
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    });
    h.run_steps(2);
}

/// **`Ctrl+Tab` and `Ctrl+Shift+Tab` walk the bar** and wrap at its ends, the project tab
/// among the rest, as `Ctrl+1..9` counts them.
#[test]
fn ctrl_tab_and_ctrl_shift_tab_walk_the_tabs_and_wrap() {
    let mut h = harness();
    h.step();
    let first = h.state().graph().workspaces()[0].id;
    add_empty_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;
    assert_eq!(h.state().active(), Active::Workspace(second));

    ctrl_tab(&mut h, false);
    assert_eq!(
        h.state().active(),
        Active::Project,
        "past the end, the start"
    );
    ctrl_tab(&mut h, false);
    assert_eq!(h.state().active(), Active::Workspace(first));
    ctrl_tab(&mut h, true);
    assert_eq!(h.state().active(), Active::Project);
    ctrl_tab(&mut h, true);
    assert_eq!(
        h.state().active(),
        Active::Workspace(second),
        "and back round"
    );
}

/// **The list at the end of the bar names every workspace**, open or not, and a closed one
/// chosen there opens a tab.
#[test]
fn the_list_at_the_end_of_the_bar_opens_a_closed_workspace() {
    let mut h = harness();
    h.step();
    add_empty_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;
    h.state_mut().close_workspace(second);
    h.run_steps(2);
    assert!(h.query_by_label("tab Workspace 2").is_none());

    h.get_by_label("all workspaces").click();
    h.run_steps(2);
    assert!(h.query_by_label("show Workspace 1").is_some());
    h.get_by_label("show Workspace 2").click();
    h.run_steps(2);
    assert_eq!(h.state().active(), Active::Workspace(second));
    assert!(h.state().open_workspaces().contains(&second));
}

/// **Tabs that do not fit leave the bar, and the one showing stays on it**, with the list
/// at its end still inside the window.
#[test]
fn tabs_that_do_not_fit_leave_the_bar_and_the_one_showing_stays() {
    let mut h = harness();
    h.step();
    for _ in 0..14 {
        add_empty_workspace(&mut h);
    }
    let last = h.state().graph().workspaces().last().unwrap().id;
    assert_eq!(h.state().active(), Active::Workspace(last));
    let on_bar = |h: &Harness<'_, App>| {
        workspace_names(h)
            .iter()
            .filter(|n| h.query_by_label(&format!("tab {n}")).is_some())
            .count()
    };
    assert!(on_bar(&h) < 15, "fifteen tabs do not fit in this window");
    assert!(
        h.query_by_label("tab Workspace 15").is_some(),
        "the one showing"
    );
    let window = h.ctx.content_rect();
    assert!(
        h.get_by_label("all workspaces").rect().max.x <= window.max.x,
        "the list is inside the window"
    );

    click_tab(&mut h, "tab Workspace 1");
    assert!(h.query_by_label("tab Workspace 1").is_some());
    h.get_by_label("all workspaces").click();
    h.run_steps(2);
    h.get_by_label("show Workspace 15").click();
    h.run_steps(2);
    assert_eq!(h.state().active(), Active::Workspace(last));
    assert!(h.query_by_label("tab Workspace 15").is_some());
}

/// The list's mark is a glyph the fonts have, not a box.
#[test]
fn the_lists_mark_has_a_glyph() {
    let mut h = harness();
    h.step();
    let c = supersilvia::ui::tabs::OVERFLOW.chars().next().unwrap();
    let width = h
        .ctx
        .fonts_mut(|f| f.glyph_width(&egui::FontId::proportional(14.0), c));
    assert!(width > 0.0, "U+{:04X} draws as a box", c as u32);
}

#[test]
fn closing_from_the_project_tab_takes_the_tab_and_the_card_says_closed() {
    let mut h = harness();
    h.step();
    add_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;

    click_tab(&mut h, "tab project");
    assert!(h.query_by_label("workspace card Workspace 2").is_some());

    // Clicking the card opens it. **Over the picture**, not the middle: the blurb is a text
    // editor in place and the buttons are buttons, so they take their own clicks, and what
    // is left over is the card's — which is what `UiBuilder::sense` registers underneath.
    let card = h.get_by_label("workspace card Workspace 2").rect();
    click_with(
        &mut h,
        Pos2::new(card.min.x + 30.0, card.center().y),
        egui::Modifiers::NONE,
    );
    h.run_steps(2);
    assert_eq!(h.state().active(), Active::Workspace(second));

    click_tab(&mut h, "tab project");
    let buttons: Vec<_> = h.get_all_by_label("Close").collect();
    assert_eq!(buttons.len(), 2, "one per open workspace");
    drop(buttons);
    h.get_all_by_label("Close").last().unwrap().click();
    h.run_steps(2);

    assert!(!h.state().open_workspaces().contains(&second));
    assert!(
        h.query_by_label("tab Workspace 2").is_none(),
        "the tab is gone"
    );
    assert!(
        h.query_by_label("workspace card Workspace 2").is_some(),
        "the card is not: a closed workspace is still in the project"
    );
    assert!(
        h.query_by_label("Open").is_some(),
        "and the card offers to open it again"
    );
}

/// There is nothing for a Workspace menu to act on while the project tab is showing.
#[test]
fn the_workspace_menu_is_absent_on_the_project_tab_and_present_on_a_workspace() {
    let mut h = harness();
    h.step();
    assert!(h.query_by_label("Workspace").is_some());

    click_tab(&mut h, "tab project");
    assert!(h.query_by_label("Workspace").is_none());

    click_tab(&mut h, "tab Workspace 1");
    assert!(h.query_by_label("Workspace").is_some());
}

/// Layout is the active workspace's, so switching a tab does not switch the other one.
#[test]
fn layout_under_workspace_ticks_only_the_active_workspaces_mode() {
    let mut h = harness();
    h.step();
    let first = h.state().graph().workspaces()[0].id;
    add_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;

    set_layout(&mut h, "Linear");
    assert_eq!(h.state().graph().layout_of(second), LayoutMode::Linear);
    assert_eq!(
        h.state().graph().layout_of(first),
        LayoutMode::Canvas,
        "the other workspace kept its own"
    );
}

/// Switching brings back the view you left, because it is the project's session state.
#[test]
fn switching_tabs_brings_back_the_view_each_was_left_at() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    // A pan on the first workspace.
    drag_with(
        &mut h,
        Pos2::new(200.0, 400.0),
        Pos2::new(260.0, 460.0),
        egui::Modifiers::NONE,
    );
    let panned = h.state().canvas_transform().pan;
    assert_ne!(panned, egui::Vec2::ZERO);

    add_workspace(&mut h);
    assert_eq!(
        h.state().canvas_transform().pan,
        egui::Vec2::ZERO,
        "a workspace never looked at opens at the origin"
    );

    click_tab(&mut h, "tab Workspace 1");
    assert_eq!(
        h.state().canvas_transform().pan,
        panned,
        "and it comes back"
    );
}

/// A project of its own for a test that touches the folder, with the scratch cleaned first.
fn project_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-ui-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

/// Put a picture where a save's readback would have left one. There is no GPU here,
/// so the readback itself is layer 3's; what this covers is the card finding the file,
/// turning it into a texture, and naming itself in the tree.
fn write_thumbnail(h: &mut Harness<'_, App>, workspace: supersilvia::graph::WorkspaceId) {
    let path = h
        .state()
        .project()
        .thumbnail_path(workspace)
        .expect("the workspace has been saved, so it has a file");
    let mut image = supersilvia::video::png::Image::new(240, 135);
    for (i, pixel) in image.rgba.chunks_mut(4).enumerate() {
        let x = u8::try_from(i % 240).unwrap();
        pixel.copy_from_slice(&[x, 40, 200 - x / 2, 255]);
    }
    supersilvia::video::png::write(&path, &image).unwrap();
}

#[test]
fn a_saved_workspace_card_shows_its_picture() {
    let dir = project_dir("thumb");
    let mut h = harness();
    h.step();
    h.state_mut().new_project(dir.join("friday"));
    h.step();
    let first = h.state().graph().workspaces()[0].id;
    h.state_mut().save_project().unwrap();
    write_thumbnail(&mut h, first);

    // Measured before the reopen, which is what makes the app look again: the first look
    // found no file and allocated nothing, so anything counted after this is the picture.
    let before = h.ctx.tex_manager().read().num_allocated();
    h.state_mut().open_project(dir.join("friday"));
    click_tab(&mut h, "tab project");
    // One picture is read per frame, so the card has its texture a frame or two after the
    // page first draws — which is the point: reading a PNG never costs the render a frame.
    h.run_steps(3);

    assert!(
        h.ctx.tex_manager().read().num_allocated() > before,
        "the picture was registered as a texture"
    );
    assert!(h.query_by_label("workspace card Workspace 1").is_some());
    std::fs::remove_dir_all(&dir).ok();
}

/// A workspace leaves from the thing, so Export is under Workspace — and it does something,
/// which is the half that used to be missing: the entry was drawn disabled until now.
///
/// **Enabled-ness is asserted by the work rather than by the tree.** Every entry in an egui
/// menu comes through AccessKit marked disabled, `Save` included, so the flag says nothing
/// here; what the entry reaches is `App::export_workspace`, and that is what is driven.
/// Clicking it would put up a folder dialog on its own thread, which is a portal round trip
/// and not what could break.
#[test]
fn workspace_export_is_offered_and_writes_the_workspace_out() {
    let dir = project_dir("export");
    let mut h = harness();
    h.step();
    h.state_mut().new_project(dir.join("friday"));
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    h.state_mut().save_project().unwrap();

    h.get_by_label("Workspace").click();
    h.step();
    assert!(
        h.query_by_label("Export…").is_some(),
        "Export… is on the Workspace menu"
    );
    // And on the card, because a thing leaves from the thing.
    click_tab(&mut h, "tab project");
    h.run_steps(2);
    assert!(h.query_by_label("export Workspace 1").is_some());

    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let first = h.state().graph().workspaces()[0].id;
    h.state_mut().export_workspace(first, &out);

    assert!(
        out.join("Workspace 1.ssw").is_file(),
        "the file was written"
    );
    assert!(
        h.state().file_status().starts_with("exported Workspace 1"),
        "and the report went to the status line: {:?}",
        h.state().file_status()
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Put a real file in `assets/` and point a node at it, the way a drop would.
fn asset_and_user(
    h: &mut Harness<'_, App>,
    dir: &std::path::Path,
    workspace: supersilvia::graph::WorkspaceId,
) -> String {
    let source = dir.join("gumbasia.webm");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(&source, b"a clip").unwrap();
    let reference = h.state().project().import_asset(&source).unwrap();
    h.state_mut()
        .apply(Command::AddNode {
            slug: "video",
            at: Pos2::new(40.0, 40.0),
            workspace,
        })
        .unwrap();
    let node = h.state().graph().iter().map(|(id, _)| id).last().unwrap();
    h.state_mut()
        .apply(Command::SetOption {
            node,
            key: "file",
            value: reference.clone(),
        })
        .unwrap();
    h.step();
    reference
}

#[test]
fn the_assets_section_lists_a_file_its_user_and_refuses_to_remove_it() {
    let dir = project_dir("assets");
    let mut h = harness();
    h.step();
    h.state_mut().new_project(dir.join("friday"));
    h.step();
    let first = h.state().graph().workspaces()[0].id;
    asset_and_user(&mut h, &dir.join("elsewhere"), first);

    click_tab(&mut h, "tab project");
    h.run_steps(2);

    assert!(
        h.query_by_label("asset card gumbasia.webm").is_some(),
        "the card is in the tree, named by the file"
    );
    assert!(
        h.query_by_label("video1.file on Workspace 1").is_some(),
        "and so is the node using it, on the workspace it is on"
    );
    assert!(
        h.query_by_label("import asset").is_some(),
        "an asset enters the project at the list it joins"
    );

    // Removing it is refused while the node points at it, and the reason says by what.
    h.get_by_label("remove gumbasia.webm").click();
    h.run_steps(2);
    assert!(
        h.state().file_status().contains("video1.file"),
        "refused, and it says by what: {:?}",
        h.state().file_status()
    );
    assert!(
        h.state()
            .project()
            .resolve("assets/gumbasia.webm")
            .is_file(),
        "and the file is still there"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// The other half: nothing points at it, so it goes, and the card goes with it.
#[test]
fn removing_an_unused_asset_takes_the_file_and_the_card() {
    let dir = project_dir("assets-remove");
    let mut h = harness();
    h.step();
    h.state_mut().new_project(dir.join("friday"));
    h.step();
    let source = dir.join("elsewhere/logo.png");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();
    std::fs::write(&source, b"not really a png").unwrap();
    h.state().project().import_asset(&source).unwrap();

    click_tab(&mut h, "tab project");
    h.run_steps(2);
    assert!(h.query_by_label("asset card logo.png").is_some());

    h.get_by_label("remove logo.png").click();
    h.run_steps(2);

    assert!(!h.state().project().resolve("assets/logo.png").is_file());
    assert!(
        h.query_by_label("asset card logo.png").is_none(),
        "and the list is what the folder holds"
    );
    std::fs::remove_dir_all(&dir).ok();
}

/// Import… is at the top of the Workspaces list, because that is the list it adds to.
#[test]
fn the_workspaces_section_offers_import() {
    let mut h = harness();
    h.step();
    click_tab(&mut h, "tab project");
    h.run_steps(2);
    // The workspaces one says what it says; the assets one carries its own name in the
    // tree, because four cards each with a Remove would otherwise be four widgets called
    // Remove and a test could reach none of them.
    let buttons: Vec<_> = h.get_all_by_label("Import…").collect();
    assert_eq!(buttons.len(), 1, "at the top of the Workspaces list");
    drop(buttons);
    assert!(h.query_by_label("import asset").is_some());
}

/// A tab renames in place: a double-click opens the editor on its name, Enter commits it as
/// one undo step, and Escape drops what was typed.
#[test]
fn a_tab_renames_inline() {
    let mut h = harness();
    h.step();
    let id = h.state().graph().default_workspace();
    let name = h.state().graph().workspace(id).unwrap().name.clone();
    let before = h.state().history().len();

    let type_over = |h: &mut Harness<'_, App>, text: &str| {
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.step();
        h.input_mut()
            .events
            .push(egui::Event::Text(text.to_string()));
        h.step();
    };

    double_click(&mut h, &format!("tab {name}"));
    h.step();
    type_over(&mut h, "Cameras");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().graph().workspace(id).unwrap().name, "Cameras");
    assert_eq!(h.state().history().len(), before + 1, "one undo step");
    assert!(h.query_by_label("tab Cameras").is_some(), "the tab says so");

    double_click(&mut h, "tab Cameras");
    h.step();
    type_over(&mut h, "Nothing");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(
        h.state().graph().workspace(id).unwrap().name,
        "Cameras",
        "Escape dropped it"
    );
    assert_eq!(h.state().history().len(), before + 1);
}

/// Project ▸ Recent shows a folder that is no longer there, disabled, beside the ones that
/// are: the list is what was opened, and a missing entry says so better than an absence.
#[test]
fn recent_shows_a_missing_folder_disabled() {
    let dir = std::env::temp_dir().join(format!("ssv-ui-recent-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let (gone, here) = (dir.join("gone"), dir.join("here"));
    let mut h = harness();
    h.step();
    h.state_mut().new_project(gone.clone());
    h.state_mut().new_project(here);
    std::fs::remove_dir_all(&gone).unwrap();
    h.step();

    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label_contains("Recent").click();
    h.run_steps(2);
    assert!(
        h.get_by_label("gone").accesskit_node().is_disabled(),
        "a missing folder is disabled"
    );
    assert!(
        !h.get_by_label("here").accesskit_node().is_disabled(),
        "a folder that is there is not"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_tab_bar_and_the_project_tab_look_like_this() {
    // A project of its own, so the name on the page is a name and not this run's scratch
    // folder — a snapshot that carries a process id is a snapshot that never matches.
    let dir = std::env::temp_dir().join(format!("ssv-ui-tabs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let mut h = harness();
    h.step();
    h.state_mut().new_project(dir.join("friday"));
    h.step();
    add_workspace(&mut h);
    add_workspace(&mut h);
    h.run_steps(2);
    assert_eq!(h.state().graph().workspaces().len(), 3, "three tabs");
    h.snapshot("tab_bar");

    // The project tab with something to show: a saved workspace's picture, and an asset
    // with the node that uses it. Added after the bar's snapshot, so that one stays about
    // the bar.
    h.state_mut().save_project().unwrap();
    let first = h.state().graph().workspaces()[0].id;
    write_thumbnail(&mut h, first);
    // Closed, so the card shows what a closed workspace looks like *and* keeps its last
    // picture — and so the `video` node put on it is suspended and never opens the six
    // bytes standing in for a clip, which would put this machine's paths in the snapshot.
    h.state_mut().close_workspace(first);
    asset_and_user(&mut h, &dir.join("elsewhere"), first);

    click_tab(&mut h, "tab project");
    h.run_steps(4);
    h.snapshot("project_tab");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------- across two workspaces

/// A cable whose two ends are on different tabs.
///
/// Both nodes are added on the first workspace and the Output is moved to the second, which
/// is the shape a person reaches by dragging one node to another tab. The second is showing
/// when this returns.
fn across_two_workspaces(
    h: &mut Harness<'_, App>,
) -> (
    supersilvia::graph::NodeId,
    supersilvia::graph::NodeId,
    supersilvia::graph::WorkspaceId,
) {
    add_node(h, ADD_CHECKERBOARD);
    add_node(h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (source, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(source, "output"),
            to: PortRef::new(out, "input"),
        })
        .expect("color into color");
    let first = h.state().graph().workspaces()[0].id;
    add_empty_workspace(h);
    let second = h.state().graph().workspaces()[1].id;
    h.state_mut()
        .apply(Command::MoveTo {
            nodes: vec![out],
            workspace: second,
        })
        .expect("the second workspace is there");
    h.run_steps(2);
    (source, out, first)
}

/// The tag on the input port names the node the cable comes from and the workspace it is
/// on, and the cable itself is not drawn — neither end can be, because one of them is
/// somewhere else.
#[test]
fn a_cable_with_one_end_elsewhere_is_a_tag_and_not_a_cable() {
    let mut h = harness();
    h.step();
    across_two_workspaces(&mut h);

    assert!(
        h.query_by_label("tag from checkerboard1.output on Workspace 1")
            .is_some(),
        "the tag names the source port and the workspace it is on"
    );
    assert!(
        h.query_by_label("cable checkerboard1.output to output2.input")
            .is_none(),
        "and the cable is not in the tree, because it is not drawn"
    );
    // The pill takes its height from the icon in it, which is four points larger than the
    // name — but it still has to sit in a port row, so a bump that grew it past the row
    // pitch would put two tags into each other.
    let pill = h
        .get_by_label("tag from checkerboard1.output on Workspace 1")
        .rect();
    assert!(
        pill.height() <= supersilvia::ui::canvas::PORT_PITCH,
        "the tag is {} tall, taller than the row it sits in",
        pill.height()
    );
    h.snapshot("cross_workspace_tag");

    // Back on the first workspace the same cable is a tag on the *output*'s side: the
    // source is here and the Output is not, so the port keeps its border and says where.
    click_tab(&mut h, "tab Workspace 1");
    assert!(h.query_by_label("checkerboard1").is_some());
    assert!(
        h.query_by_label("tag from checkerboard1.output on Workspace 1")
            .is_none(),
        "a node does not tag itself"
    );
}

/// Clicking a tag activates the workspace the source is on and puts the view on it.
#[test]
fn clicking_a_tag_goes_to_the_node_the_cable_came_from() {
    let mut h = harness();
    h.step();
    let (source, _, first) = across_two_workspaces(&mut h);
    assert!(
        h.query_by_label("checkerboard1").is_none(),
        "the source is not on the workspace showing"
    );
    let edits = h.state().history().len();

    h.get_by_label("tag from checkerboard1.output on Workspace 1")
        .click();
    h.run_steps(2);

    assert_eq!(h.state().active(), Active::Workspace(first), "it switched");
    assert!(
        h.query_by_label("checkerboard1").is_some(),
        "and the source node is on screen"
    );
    // Centerd, not merely visible: the view is put on the node rather than left where the
    // tab was.
    let at = h
        .state()
        .canvas_transform()
        .to_screen(h.state().canvas_origin(), node_rect(&h, source).center());
    let middle = h.state().canvas_origin()
        + egui::vec2(h.state().canvas_width(), h.state().canvas_height()) * 0.5;
    assert!((at - middle).length() < 1.0, "{at:?} against {middle:?}");

    // Following a tag is session state, like switching a tab: it never enters the history.
    assert_eq!(h.state().history().len(), edits);
}

/// A tag naming a **closed** workspace is still there, and clicking it opens that workspace.
#[test]
fn a_tag_survives_the_source_workspace_being_closed_and_opens_it_again() {
    let mut h = harness();
    h.step();
    let (_, _, first) = across_two_workspaces(&mut h);
    h.state_mut().close_workspace(first);
    h.run_steps(2);
    assert!(!h.state().open_workspaces().contains(&first));
    assert!(
        h.query_by_label("tab Workspace 1").is_none(),
        "the source workspace has no tab"
    );

    let tag = "tag from checkerboard1.output on Workspace 1";
    assert!(
        h.query_by_label(tag).is_some(),
        "and the tag still names it"
    );
    h.get_by_label(tag).click();
    h.run_steps(2);
    assert!(h.state().open_workspaces().contains(&first));
    assert_eq!(h.state().active(), Active::Workspace(first));
}

/// The node menu's Workspaces ▸ puts a node on a second workspace, and it is one node.
#[test]
fn ticking_a_box_in_the_workspaces_submenu_shows_a_node_on_both() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    // Empty, because this test counts the nodes in the graph to show that a node on two
    // workspaces is still one node; a seeded tab would put two more in the count.
    add_empty_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;
    click_tab(&mut h, "tab Workspace 1");

    let at = header_at(&h, id);
    right_click(&mut h, at);
    h.get_by_label_contains("Workspaces").click();
    h.run_steps(2);
    h.get_by_label("Workspace 2").click();
    h.run_steps(2);

    assert_eq!(
        h.state().graph().get(id).unwrap().workspaces.len(),
        2,
        "it is on both"
    );
    assert!(h.query_by_label("checkerboard1").is_some());
    click_tab(&mut h, "tab Workspace 2");
    assert!(
        h.query_by_label("checkerboard1").is_some(),
        "one node, in both trees"
    );
    assert_eq!(h.state().graph().len(), 1, "and one node in the graph");
    assert!(
        h.state()
            .graph()
            .get(id)
            .unwrap()
            .workspaces
            .contains(&second)
    );
}

/// The last workspace a node is on cannot be unticked: `HideFrom` refuses it, so the box is
/// drawn disabled rather than offered and then refused.
#[test]
fn the_last_workspace_a_node_is_on_is_a_disabled_tick() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    add_workspace(&mut h);
    click_tab(&mut h, "tab Workspace 1");

    let at = header_at(&h, id);
    right_click(&mut h, at);
    h.get_by_label_contains("Workspaces").click();
    h.run_steps(2);
    h.get_by_label("Workspace 1").click();
    h.run_steps(2);

    assert_eq!(
        h.state().graph().get(id).unwrap().workspaces.len(),
        1,
        "the click did nothing, because the box is disabled"
    );
}

/// Dragging a node's header onto a tab shows it there too — silvia's gesture, and the one
/// people use. The canvas reports the drag, the tab bar reports the tab, `App` joins them.
#[test]
fn dragging_a_node_onto_a_tab_shows_it_there() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    // Empty, because this test counts the nodes in the graph to show that a node on two
    // workspaces is still one node; a seeded tab would put two more in the count.
    add_empty_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;
    click_tab(&mut h, "tab Workspace 1");

    let from = header_at(&h, id);
    let onto = h.get_by_label("tab Workspace 2").rect().center();
    drag_with(&mut h, from, onto, egui::Modifiers::NONE);

    assert!(
        h.state()
            .graph()
            .get(id)
            .unwrap()
            .workspaces
            .contains(&second),
        "the drop put it on the second workspace"
    );
    click_tab(&mut h, "tab Workspace 2");
    assert!(h.query_by_label("checkerboard1").is_some());
}

/// Dropping on the project tab does nothing: a node cannot be shown on it.
#[test]
fn dropping_a_node_on_the_project_tab_does_nothing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    add_workspace(&mut h);
    click_tab(&mut h, "tab Workspace 1");
    let before = h.state().graph().get(id).unwrap().workspaces.clone();

    let from = header_at(&h, id);
    let onto = h.get_by_label("tab project").rect().center();
    drag_with(&mut h, from, onto, egui::Modifiers::NONE);

    assert_eq!(h.state().graph().get(id).unwrap().workspaces, before);
}

/// A dropped file, as the windowing integration hands one over.
///
/// egui models a dropped file as a trait so it stays independent of file APIs, and the
/// native implementation belongs to egui-winit and is private, so a test brings its own.
#[derive(Debug)]
struct DroppedPath(std::path::PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// A file dropped on the window becomes a `video` node whose file is *in the project*.
///
/// The drop is an import: the option holds a reference and the bytes are under `assets/`, so
/// the folder still plays after it is moved to another machine. The node is asserted before
/// it ever ticks, so nothing here opens a decoder.
#[test]
fn dropping_a_file_imports_it_into_the_project() {
    let mut h = harness();
    h.step();

    let dir = std::env::temp_dir().join(format!("ssv-ui-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("gumbasia.webm");
    std::fs::write(&source, b"a clip").unwrap();

    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(DroppedPath(source)));
    h.step();

    let app = h.state();
    let (_, node) = app.graph().iter().next().expect("the drop made a node");
    assert_eq!(node.def.slug, "video");
    assert_eq!(
        node.options.get("file").map(String::as_str),
        Some("assets/gumbasia.webm"),
    );
    let copied = app.project().resolve("assets/gumbasia.webm");
    assert_eq!(std::fs::read(copied).unwrap(), b"a clip");
}

/// **A dropped picture lands on the node that can show one.**
///
/// silvia's Image/GIF node is mostly a dashed panel saying *drop image here*, so a picture
/// reaches a patch in one gesture. Here the gesture is the window's, and a picture dropped on
/// a `video` node is a black node to delete: the transcode has no clip to make of a png or a
/// GIF. So every picture the Image/GIF node lists, an animated GIF included, makes one of
/// those, and a clip makes a `video`.
#[test]
fn a_dropped_picture_makes_an_image_node_and_a_clip_still_makes_a_video() {
    let dir = std::env::temp_dir().join(format!("ssv-ui-drop-kind-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();

    for (name, slug) in [
        ("logo.png", "imagegif"),
        ("photo.JPEG", "imagegif"),
        ("sticker.webp", "imagegif"),
        ("loop.gif", "imagegif"),
        ("SHOUTING.GIF", "imagegif"),
        ("gumbasia.webm", "video"),
        ("tape.mov", "video"),
    ] {
        let mut h = harness();
        h.step();
        let source = dir.join(name);
        std::fs::write(&source, b"bytes").unwrap();
        h.input_mut()
            .dropped_files
            .push(std::sync::Arc::new(DroppedPath(source)));
        h.step();
        let app = h.state();
        let (_, node) = app.graph().iter().next().expect("the drop made a node");
        assert_eq!(node.def.slug, slug, "{name}");
        // Whichever node it is, the drop is still an import and the option still holds a
        // reference into the project's own `assets/`.
        assert_eq!(
            node.options.get("file").map(String::as_str),
            Some(format!("assets/{name}").as_str()),
            "{name}",
        );
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A file of this test's own to drop, in a folder of its own.
fn file_to_drop(test: &str, name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssv-ui-{test}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, b"a clip").unwrap();
    file
}

fn drop_file(h: &mut Harness<'_, App>, file: std::path::PathBuf) {
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(DroppedPath(file)));
    h.step();
}

/// **A dropped file lands where it is pointed**: on the workspace showing, not the first in
/// project order, with the node's corner under the pointer.
#[test]
fn a_dropped_file_lands_on_the_workspace_showing_under_the_pointer() {
    let mut h = harness();
    h.step();
    add_empty_workspace(&mut h);
    let second = h.state().graph().workspaces()[1].id;
    let origin = h.state().canvas_origin();
    let at = origin + egui::vec2(220.0, 140.0);
    hover_at(&mut h, at);

    drop_file(&mut h, file_to_drop("drop-here", "gumbasia.webm"));

    let (_, node) = h
        .state()
        .graph()
        .iter()
        .next()
        .expect("the drop made a node");
    assert_eq!(
        node.workspaces,
        std::collections::BTreeSet::from([second]),
        "on the workspace showing"
    );
    let want = h.state().canvas_transform().to_world(origin, at);
    assert!(
        (node.pos - want).length() < 0.5,
        "under the pointer: {:?} against {want:?}",
        node.pos
    );
}

/// Where the window was given no pointer — a drag from another app, on Wayland — a drop
/// lands at the centre of the view.
#[test]
fn with_no_pointer_a_drop_lands_at_the_views_centre() {
    let mut h = harness();
    h.step();
    h.input_mut().events.push(egui::Event::PointerGone);
    h.step();
    drop_file(&mut h, file_to_drop("drop-centre", "gumbasia.webm"));

    let (_, node) = h
        .state()
        .graph()
        .iter()
        .next()
        .expect("the drop made a node");
    let origin = h.state().canvas_origin();
    let centre = origin
        + egui::vec2(
            h.state().canvas_width() * 0.5,
            h.state().canvas_height() * 0.5,
        );
    let want = h.state().canvas_transform().to_world(origin, centre);
    assert!(
        (node.pos - want).length() < 0.5,
        "{:?} against {want:?}",
        node.pos
    );
}

/// **On the project tab a dropped file is an asset and nothing else**: there is no canvas to
/// put a node on, and the Assets list is what it joins.
#[test]
fn a_file_dropped_on_the_project_tab_is_an_asset_and_makes_no_node() {
    let mut h = harness();
    h.step();
    click_tab(&mut h, "tab project");
    drop_file(&mut h, file_to_drop("drop-asset", "gumbasia.webm"));
    h.run_steps(2);

    assert!(h.state().graph().is_empty(), "no node");
    let copied = h.state().project().resolve("assets/gumbasia.webm");
    assert_eq!(std::fs::read(copied).unwrap(), b"a clip");
    assert!(h.query_by_label("asset card gumbasia.webm").is_some());
}

/// Files held over the window, as the windowing integration reports them before a drop.
fn hold_files(h: &mut Harness<'_, App>, names: &[&str]) {
    h.input_mut().hovered_files = names
        .iter()
        .map(|n| egui::HoveredFile {
            path: Some(std::path::PathBuf::from(n)),
            ..Default::default()
        })
        .collect();
    h.run_steps(2);
}

/// **While a file is held over the window, one line says what the drop will make**, and the
/// line goes when the file does.
#[test]
fn a_file_held_over_the_window_says_what_the_drop_will_make() {
    let mut h = harness();
    h.step();
    hold_files(&mut h, &["gumbasia.webm"]);
    assert!(h.query_by_label("new Video node on Workspace 1").is_some());

    hold_files(&mut h, &["tunnel.ssw"]);
    assert!(
        h.query_by_label("tunnel.ssw: import as workspace")
            .is_some()
    );

    hold_files(&mut h, &[]);
    click_tab(&mut h, "tab project");
    hold_files(&mut h, &["gumbasia.webm"]);
    assert!(h.query_by_label("gumbasia.webm: add to assets").is_some());

    hold_files(&mut h, &[]);
    assert!(h.query_by_label("gumbasia.webm: add to assets").is_none());
}

// ----------------------------------------------------------------- uniform readouts

/// A Ratio Gear and a `slew`, added through the menu, moved apart so both are on screen and
/// neither's rows sit under the other's.
fn two_control_nodes(
    h: &mut Harness<'_, App>,
) -> (supersilvia::graph::NodeId, supersilvia::graph::NodeId) {
    add_node(h, ADD_PHASE);
    add_node(h, ADD_SLEW);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (phase, slew) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (phase, Pos2::new(40.0, 40.0)),
                // Clear of the gear's rows and its picture, so the snapshot shows both nodes
                // whole.
                (slew, Pos2::new(40.0, 420.0)),
            ],
        })
        .unwrap();
    h.step();
    (phase, slew)
}

/// A uniform number output carries the number it published, so the row a person reads and
/// the tree an agent reads say the same thing.
#[test]
fn a_uniform_output_shows_the_value_it_published() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_PHASE);
    let phase = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.run_steps(4);

    let value = h
        .state()
        .uniform(PortRef::new(phase, "cycles"))
        .expect("a gear publishes its cycles every tick");
    assert!(value > 0.0, "ambient seconds at ×1: {value}");
    assert!(
        h.query_by_label(&format!(
            "ratiogear1.cycles (uniform number output) {value:.2}"
        ))
        .is_some(),
        "the published value is on the port",
    );
}

/// **A count's row reads the count**, however far into the show: a Ratio Gear on ambient
/// seconds at a playhead of 123456.78 publishes 123456.78 on its Cycles and prints it there,
/// and the row's label is still drawn whole beside it.
#[test]
fn a_counts_row_reads_the_count_far_into_the_show() {
    use supersilvia::transport::Command as Transport;
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_PHASE);
    let gear = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut().transport(Transport::Pause);
    h.state_mut().transport(Transport::Seek(123_456.78));
    h.run_steps(4);

    let cycles = PortRef::new(gear, "cycles");
    assert_eq!(
        h.state().uniform(cycles),
        Some(123_456.78),
        "the count, whole"
    );
    assert!(
        h.query_by_label("ratiogear1.cycles (uniform number output) 123456.78")
            .is_some(),
        "the port names the count"
    );
    let painted = painted_text(&h);
    assert!(
        painted.iter().any(|t| t == "123456.78"),
        "the row prints the count: {painted:?}"
    );
    assert!(
        painted.iter().any(|t| t == "Cycles"),
        "and its label whole beside it: {painted:?}"
    );
}

/// A control whose input is connected is a meter of what arrives, not of what is stored.
#[test]
fn a_connected_control_shows_the_arriving_value_and_not_its_own() {
    let mut h = harness();
    h.step();
    let (phase, slew) = two_control_nodes(&mut h);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(phase, "cycles"),
            to: PortRef::new(slew, "input"),
        })
        .expect("uniform number into uniform number");
    h.run_steps(4);

    let arriving = h
        .state()
        .uniform(PortRef::new(phase, "cycles"))
        .expect("the producer published");
    assert!(arriving > 0.0);
    assert!(
        h.query_by_label_contains(&format!("slew2.input {arriving:.2}"))
            .is_some(),
        "the disabled control shows the value the node sees",
    );
    assert!(
        h.query_by_label_contains("slew2.input 0.00").is_none(),
        "and not the stored default a connection overrode",
    );
    h.snapshot("uniform_readouts");
}

/// Nothing connected, nothing to override: the control still says what it holds.
#[test]
fn an_unfed_input_shows_its_own_stored_value() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_SLEW);
    h.run_steps(2);

    assert!(
        h.query_by_label_contains("slew1.rise 10.00").is_some(),
        "an unconnected control is the value it stores",
    );
}

/// A port that has published nothing draws nothing — a node that has not run, not a node
/// reading zero.
///
/// A dropped file becomes a node above the canvas, so the frame it lands on is the one
/// frame it is drawn without having ticked: every uniform number it will publish is still
/// empty.
#[test]
fn a_port_that_has_published_nothing_shows_no_value() {
    let mut h = harness();
    h.step();

    let dir = std::env::temp_dir().join(format!("ssv-ui-readout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("gumbasia.webm");
    std::fs::write(&source, b"a clip").unwrap();
    h.input_mut()
        .dropped_files
        .push(std::sync::Arc::new(DroppedPath(source)));
    h.step();

    let video = h
        .state()
        .graph()
        .iter()
        .map(|(id, _)| id)
        .next()
        .expect("the drop made a node");
    assert!(
        h.state().uniform(PortRef::new(video, "bass")).is_none(),
        "nothing has ticked it",
    );
    assert!(
        h.query_by_label("video1.bass (uniform number output)")
            .is_some(),
        "the port is in the tree carrying no number",
    );
}

/// An option a cable answers goes inert and says which input answered it.
///
/// `OptionDef::overridden_by` is what the canvas draws from, so nothing in `ui/` knows this
/// is a tap. The select stays in the accessibility tree in both states, carrying what it
/// shows, so a test — and an agent — can tell them apart without a screenshot.
#[test]
fn a_connected_sidechain_disables_the_option_it_overrides() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_TAP);
    add_node(&mut h, ADD_LUMINOSITY);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (tap, luminosity) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (tap, Pos2::new(40.0, 40.0)),
                (luminosity, Pos2::new(340.0, 40.0)),
            ],
        })
        .unwrap();
    h.run_steps(2);

    assert!(
        h.query_by_label_contains("tap1.measure Luminosity")
            .is_some(),
        "unconnected, the select reads the quantity it names",
    );

    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(luminosity, "output"),
            to: PortRef::new(tap, "number"),
        })
        .expect("a varying number into the tap's sidechain");
    h.run_steps(2);

    let overridden = h
        .query_by_label_contains("tap1.measure Number")
        .expect("the select reads the input that overrode it");
    assert!(
        overridden.accesskit_node().is_disabled(),
        "and it is disabled, so clicking opens nothing",
    );
    assert!(
        h.query_by_label_contains("tap1.measure Luminosity")
            .is_none(),
        "the chosen value is not what the row says while a cable answers it",
    );

    h.state_mut()
        .apply(Command::Disconnect {
            to: PortRef::new(tap, "number"),
        })
        .expect("the cable comes off");
    h.run_steps(2);

    let restored = h
        .query_by_label_contains("tap1.measure Luminosity")
        .expect("the picker is back to what it holds");
    assert!(
        !restored.accesskit_node().is_disabled(),
        "and pressable again",
    );
}

/// `Show on A` is a button on the Output node, in the accessibility tree by its port's
/// name, and a click on it puts the Output on deck A.
#[test]
fn show_on_a_is_a_button_on_the_output_node() {
    let mut h = harness();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    assert_eq!(h.state().mixer().a, None);

    h.get_by_label(&format!("output{out}.show_a")).click();
    // The click is a level for one frame, and the tick at the top of the next reads it.
    h.run_steps(2);
    assert_eq!(h.state().mixer().a, Some(out));
    assert_eq!(h.state().mixer().b, None);

    h.get_by_label(&format!("output{out}.show_b")).click();
    h.run_steps(2);
    assert_eq!(h.state().mixer().decks_of(out), (true, true));
}

/// The Main Mixer panel: the balance is an `s-number` found by its name, and the method is
/// a select whose rows are the methods' names.
#[test]
fn the_mixer_panel_sets_the_fade_and_the_method() {
    use supersilvia::mixer::Method;

    let mut h = tall_harness();
    assert_eq!(h.state().mixer().balance, -1.0);
    // The balance is the canvas's number control, named, so what a scrub and a typed entry
    // do is covered where that control is. Here: it is there, and the two selects work.
    h.get_by_label("A / B balance -1.00");
    h.get_by_value("Simple mix").click();
    // The list is an `Area`: its first frame is a sizing pass.
    h.run_steps(2);
    h.get_by_label("Radial wipe").click();
    h.step();
    assert_eq!(h.state().mixer().method, Method::RadialWipe);

    // The resolution is the picker: Match display and Match viewport above the strip, then a
    // shape and a short side, two clicks that each keep the other.
    h.get_by_label("mix resolution Match display").click();
    h.run_steps(2);
    h.snapshot("mixer_resolution_picker");
    h.get_by_label("mix resolution 720").click();
    h.run_steps(2);
    assert_eq!(
        h.state().mixer().resolution,
        supersilvia::mixer::Resolution::Fixed(1280, 720)
    );
    h.get_by_label("mix resolution Tall").click();
    h.run_steps(2);
    assert_eq!(
        h.state().mixer().resolution,
        supersilvia::mixer::Resolution::Fixed(720, 1280),
        "Tall keeps the short side"
    );
    h.get_by_label("mix resolution 21:9").click();
    h.run_steps(2);
    h.get_by_label("mix resolution 1440").click();
    h.run_steps(2);
    assert_eq!(
        h.state().mixer().resolution,
        supersilvia::mixer::Resolution::Fixed(1440, 3440),
        "21:9 at 1440 is the size ultrawides are sold at, stood on end"
    );
    h.get_by_label("mix resolution viewport").click();
    h.run_steps(2);
    assert_eq!(
        h.state().mixer().resolution,
        supersilvia::mixer::Resolution::Viewport
    );
    h.get_by_label("mix resolution display").click();
    h.run_steps(2);
    assert_eq!(
        h.state().mixer().resolution,
        supersilvia::mixer::Resolution::Display
    );
}

/// The Main Input panel unfolds from its spine and folds from its header, and its Gain and
/// Monitor are numbers a hand steps like any other, written to the Main Input and not to the
/// undo history.
#[test]
fn the_main_input_panel_folds_and_its_gain_and_monitor_step() {
    let mut h = tall_harness();
    h.step();
    assert!(
        h.state().preferences().main_input_collapsed,
        "it starts folded"
    );
    // The spine answers a pointer, not the accessibility click, as `double_click` notes.
    let spine = h.get_by_label("Show Main Input").rect().center();
    click_at(&mut h, spine);
    // A panel slides open: let it land.
    h.run_steps(30);
    assert!(
        !h.state().preferences().main_input_collapsed,
        "the spine unfolded it"
    );

    let step = |h: &mut Harness<'_, App>, name: &str, key: egui::Key| {
        let at = number_rect(h, name).center();
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.step();
        h.key_press(key);
        h.step();
    };
    step(&mut h, "Gain 1.00", egui::Key::ArrowUp);
    assert!((h.state().project().main_input().gain - 1.01).abs() < 1e-4);
    step(&mut h, "Monitor 0.00", egui::Key::ArrowUp);
    assert!((h.state().project().main_input().monitor - 0.01).abs() < 1e-4);
    step(&mut h, "Monitor 0.01", egui::Key::ArrowDown);
    assert_eq!(h.state().project().main_input().monitor, 0.0);
    assert!(h.state().history().is_empty(), "none of it is an edit");

    let bar = h.get_by_label("Hide Main Input").rect();
    click_at(&mut h, egui::pos2(bar.right() - 8.0, bar.center().y));
    assert!(
        h.state().preferences().main_input_collapsed,
        "the header folded it"
    );
    assert!(h.query_by_label("Show Main Input").is_some());
}

/// A claimed deck names the workspace its Output lives on, and an empty one says so in its box.
#[test]
fn the_mixer_panel_names_the_deck_workspace() {
    let mut h = tall_harness();
    assert!(h.query_by_label("Channel A: no Output assigned").is_some());
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.get_by_label(&format!("output{out}.show_a")).click();
    h.run_steps(2);
    assert!(h.query_by_label("Channel A: no Output assigned").is_none());
    // The tab is `tab Workspace 1`; the bare name is the channel's link.
    h.get_by_label("Workspace 1");
    assert!(h.query_by_label("Channel B: no Output assigned").is_some());
}

/// **There is no projector.** The mix is a picture like any other, and the Main Mixer
/// panel carries the same two marks every picture wears. The window itself is the pictures
/// thread's, which a harness has none of — so what is asserted here is the editor's own
/// list, which is the half a harness has.
#[test]
fn the_mix_pops_out_from_the_main_mixer_panel() {
    let mut h = tall_harness();
    assert!(h.state().popped_out().is_empty());
    assert!(
        h.query_by_label("Open projector").is_none(),
        "the projector's button is gone with the projector"
    );

    h.get_by_label("pop out the mix").click();
    h.run_steps(2);
    assert_eq!(
        h.state().popped_out(),
        [supersilvia::ui::Popped {
            picture: supersilvia::ui::PopOut::Mix,
            fullscreen: false,
        }],
        "the mix has a window of its own"
    );

    // The mark is a toggle, as **Open projector** was: the same click puts the window away.
    h.get_by_label("pop out the mix").click();
    h.run_steps(2);
    assert!(h.state().popped_out().is_empty());

    // And the fullscreen mark is its own button, opening the window already fullscreen.
    h.get_by_label("fullscreen the mix").click();
    h.run_steps(2);
    assert_eq!(h.state().popped_out().len(), 1);
}

/// **Syphon from the editor.** On a Mac, an Output's Syphon row's button asks the publisher for
/// it under its name, with the shared Alpha as its look, and the Main Mixer's Syphon mark
/// beside the pop-out pair asks for the mix and is a toggle. On a machine without Syphon
/// neither the row nor the mark is there. What is asserted is what the publisher is asked,
/// which is the half a harness has.
#[test]
fn syphon_publishes_an_output_by_its_row_and_the_mix_by_its_mark() {
    let mut h = tall_machine_harness();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    assert!(h.state_mut().sent_wanted().is_empty());
    if !supersilvia::platform::syphon::available() {
        assert!(
            h.query_by_label(&format!("output{out}.syphon")).is_none(),
            "a machine without Syphon draws no Syphon row"
        );
        assert!(
            h.query_by_label("publish the mix over Syphon").is_none(),
            "nor the mix's Syphon mark"
        );
        return;
    }

    open_send(&mut h, out);
    h.get_by_label(&format!("output{out}.syphon")).click();
    h.run_steps(2);
    h.get_by_label(&format!("output{out}.transparent Transparent"))
        .click();
    h.run_steps(2);
    let wanted = h.state_mut().sent_wanted();
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].name, format!("supersilvia Output {}", out.0));
    assert!(wanted[0].look.transparent && !wanted[0].look.flip);
    assert!(
        h.query_by_label_contains(&format!(
            "output{out}.syphon.status on air · supersilvia Output {}",
            out.0
        ))
        .is_some(),
        "the row says it is on air, and as what"
    );

    h.get_by_label("publish the mix over Syphon").click();
    h.run_steps(2);
    let names: Vec<String> = h
        .state_mut()
        .sent_wanted()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert_eq!(
        names,
        [format!("supersilvia Output {}", out.0), "Mix".to_string()]
    );

    h.get_by_label("publish the mix over Syphon").click();
    h.run_steps(2);
    assert_eq!(h.state_mut().sent_wanted().len(), 1, "the mark is a toggle");
}

/// Open an Output's Send section, which a new Output starts with closed.
fn open_send(h: &mut Harness<'_, App>, id: supersilvia::graph::NodeId) {
    h.state_mut()
        .apply(Command::SetOption {
            node: id,
            key: supersilvia::nodes::output::SEND,
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.run_steps(2);
}

/// **NDI from the editor.** An Output's NDI row's button — **Send**, then **Stop** — asks the
/// publisher for it as `supersilvia Output <id>` and lets it go, the row saying which, and the
/// Main Mixer's NDI mark beside the pop-out pair asks for the mix as `supersilvia Mix` — where
/// the NDI runtime loads. Where it does not, the row says so and its button is **Get it**, which
/// asks the publisher for nothing, and the mark does nothing.
#[test]
fn ndi_sends_an_output_by_its_row_and_the_mix_by_its_mark() {
    let mut h = tall_machine_harness();
    let runtime = supersilvia::video::ndi::runtime();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    open_send(&mut h, out);
    let status = |h: &Harness<'_, App>, says: &str| {
        h.query_by_label_contains(&format!("output{out}.ndi.status {says}"))
            .is_some()
    };
    if !runtime {
        assert!(status(&h, "runtime not installed"));
        h.get_by_label(&format!("output{out}.ndi")).click();
        h.run_steps(2);
        assert!(
            h.state_mut().sent_wanted().is_empty(),
            "Get it sends nothing"
        );
        return;
    }
    assert!(status(&h, "off"));
    assert!(
        h.query_by_label(&format!("output{out}.transparent Opaque"))
            .is_none(),
        "no alpha to choose while nothing is sent"
    );
    h.get_by_label(&format!("output{out}.ndi")).click();
    h.run_steps(2);
    let names: Vec<String> = h
        .state_mut()
        .sent_wanted()
        .into_iter()
        .map(|w| w.name)
        .collect();
    assert_eq!(names, [format!("supersilvia Output {}", out.0)]);
    assert!(status(
        &h,
        &format!("on air · supersilvia Output {}", out.0)
    ));
    assert!(
        h.query_by_label(&format!("output{out}.transparent Opaque"))
            .is_some(),
        "and the alpha it goes out with, once it goes out"
    );

    h.get_by_label("send the mix over NDI").click();
    h.run_steps(2);
    assert_eq!(h.state_mut().sent_wanted().len(), 2, "the mark");

    // Stop is the same button.
    h.get_by_label(&format!("output{out}.ndi")).click();
    h.run_steps(2);
    assert!(status(&h, "off"));
    assert_eq!(h.state_mut().sent_wanted().len(), 1, "the mix alone");
}

/// **The name an Output goes out under** is a field under Send showing the whole of it — the
/// default until it has one of its own — committed on Enter and not a keystroke at a time, one
/// undo step each. A name another Output has takes a ` copy`, and empty goes back to the
/// default, which is stored as nothing.
#[test]
fn an_outputs_send_name_is_typed_whole_and_kept_its_own() {
    use supersilvia::nodes::output::{SEND_NAME, send_name};
    let mut h = tall_harness();
    let a = add_at(&mut h, "output", Pos2::new(48.0, 48.0));
    let b = add_at(&mut h, "output", Pos2::new(348.0, 48.0));
    open_send(&mut h, a);
    open_send(&mut h, b);
    let name = |h: &Harness<'_, App>, id| send_name(id, h.state().graph().get(id).unwrap());
    let stored = |h: &Harness<'_, App>, id: supersilvia::graph::NodeId| {
        h.state().graph().get(id).unwrap().options[SEND_NAME].clone()
    };
    assert!(
        h.query_by_label(&format!("output{a}.sendName supersilvia Output {}", a.0))
            .is_some(),
        "the field shows the default"
    );

    let rename = |h: &mut Harness<'_, App>, id, text: &str| {
        h.get_by_label_contains(&format!("output{id}.sendName "))
            .click();
        h.run_steps(2);
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        h.step();
        if text.is_empty() {
            key(h, egui::Key::Backspace);
        } else {
            type_text(h, text);
        }
    };

    let undo = h.state().undo_len();
    rename(&mut h, a, "warpzone");
    assert_eq!(stored(&h, a), "", "nothing is sent while typing");
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(name(&h, a), "warpzone");
    assert_eq!(h.state().undo_len(), undo + 1, "one commit, one step");
    assert!(
        h.query_by_label(&format!("output{a}.sendName warpzone"))
            .is_some()
    );

    rename(&mut h, b, "warpzone");
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(name(&h, b), "warpzone copy", "the name is its own");

    rename(&mut h, a, "");
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(name(&h, a), format!("supersilvia Output {}", a.0));
    assert_eq!(stored(&h, a), "", "the default is stored as nothing");
}

/// The View menu has no **Projector** item, because there is no projector to tick.
#[test]
fn the_view_menu_has_no_projector() {
    let mut h = tall_harness();
    h.get_by_label("View").click();
    h.run_steps(2);
    assert!(h.query_by_label("Projector").is_none());
}

/// `H` hides the canvas — nodes, cables, the start button — and says so; the side panels
/// stay. It is a bare key, so a text field with the keyboard takes it as a letter.
#[test]
fn h_hides_the_editor_with_a_toast_and_not_while_typing() {
    let mut h = harness();
    add_node(&mut h, ADD_CHECKERBOARD);
    let on_screen = |h: &mut Harness<'_, App>| {
        h.query_all_by_label_contains("checkerboard1.frequency")
            .count()
            > 0
    };
    assert!(on_screen(&mut h));
    assert!(!h.state().editor_hidden());

    h.key_press(egui::Key::H);
    h.run_steps(2);
    assert!(h.state().editor_hidden());
    assert!(
        h.query_by_label_contains("checkerboard1.frequency")
            .is_none(),
        "the node is off screen"
    );
    assert!(
        h.query_by_label("Nodes").is_none(),
        "and so is the start button"
    );
    h.get_by_label("Editor hidden — press H to show");
    h.get_by_label("Channel A: no Output assigned");

    h.key_press(egui::Key::H);
    h.run_steps(2);
    assert!(!h.state().editor_hidden());
    assert!(on_screen(&mut h));
    h.get_by_label("Editor visible — press H to hide");

    // The quake bar's search field has the keyboard: `H` is a letter there.
    h.key_press(egui::Key::Slash);
    h.run_steps(2);
    h.key_press(egui::Key::H);
    h.run_steps(2);
    assert!(!h.state().editor_hidden(), "H went into the search field");
}

/// *Project to background* is a checkbox on the mixer panel, off until a hand asks: it is a
/// way of looking at the patch in front of you, not a property of the show, so it is neither
/// on at launch nor written to the project.
#[test]
fn project_to_background_is_a_checkbox_on_the_panel() {
    let mut h = tall_harness();
    assert!(
        !h.state().mixer().background,
        "off at launch: the canvas is not covered until a hand asks"
    );
    h.get_by_label("Project to background").click();
    h.run_steps(2);
    assert!(h.state().mixer().background);
    h.get_by_label("Project to background").click();
    h.run_steps(2);
    assert!(!h.state().mixer().background);
}

/// The browser opens at its full height after a search that narrowed it. An `Area` hands
/// its content last frame's size as its room, so without care a two-hit list is the room
/// the next opening has, and the panel stays two rows tall for the rest of the session.
#[test]
fn the_browser_grows_back_after_a_narrow_search() {
    let mut h = harness();
    h.run_steps(2);
    let area_height = |h: &mut Harness<'_, App>| {
        egui::AreaState::load(&h.ctx, egui::Id::new("node-browser"))
            .and_then(|state| state.size)
            .map_or(0.0, |size| size.y)
    };

    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    let full = area_height(&mut h);
    assert!(full > 200.0, "the list opened tall: {full}");

    h.input_mut().events.push(egui::Event::Text("bloom".into()));
    h.run_steps(3);
    let narrow = area_height(&mut h);
    assert!(narrow < full / 2.0, "one hit is a short panel: {narrow}");

    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::Slash);
    h.run_steps(3);
    let again = area_height(&mut h);
    assert!(
        (again - full).abs() < 2.0,
        "reopened at {again}, was {full}: the short search stuck"
    );
}

/// View → Costs draws the strip, named for its node and its text, once a probe has counted.
#[test]
fn the_cost_view_puts_a_strip_under_the_node() {
    use supersilvia::compile::{EVAL_WORD, TAP_WORDS};
    use supersilvia::graph::PortRef;
    use supersilvia::render::PROBE;

    let mut h = harness();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_OUTPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, out) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();
    assert!(h.query_by_label_contains("cost").is_none());

    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Costs").click();
    h.run_steps(2);
    assert!(h.state().preferences().show_costs, "a preference");

    // No GL here, so the count is handed in as the renderer would hand it.
    let mut words = vec![0u32; TAP_WORDS];
    words[EVAL_WORD] = PROBE.0 * PROBE.1;
    h.state_mut().ingest_probe(out, &words);
    h.run_steps(2);
    // A count, not a float: `1/px`. It is a ratio underneath, and a node reaching two
    // Outputs of different sizes lands between two integers, but the strip is a badge.
    h.get_by_label_contains("checkerboard1 cost 1/px");
}

/// A node whose measured taps have multiplied wears a header warning whether or not View ▸
/// Costs is on — the strip is a preference, the reading under it is not, and the person who
/// most needs the warning is the one who has never opened the strip.
#[test]
fn a_node_that_taps_its_input_wears_a_header_warning_with_the_view_off() {
    use supersilvia::compile::{EVAL_WORD, TAP_WORDS};
    use supersilvia::graph::PortRef;
    use supersilvia::render::PROBE;

    let mut h = harness();
    let workspace = h.state().graph().default_workspace();
    for slug in ["checkerboard", "blur", "output"] {
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at: Pos2::ZERO,
                workspace,
            })
            .unwrap();
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, blur, out) = (ids[0], ids[1], ids[2]);
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(blur, "input"),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(blur, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    h.step();
    assert!(
        h.query_by_label_contains("sampling").is_none(),
        "nothing measured yet"
    );
    assert!(!h.state().preferences().show_costs, "the view starts off");

    // No GL here, so the counts are handed in as the renderer would hand them: the same
    // layout `a_probe_count_becomes_evaluations_per_pixel_and_per_frame` in tests/costs.rs
    // uses — the blur samples its checkerboard input nine times per pixel.
    let pixels = PROBE.0 * PROBE.1;
    let mut words = vec![0u32; 4 * TAP_WORDS];
    words[EVAL_WORD] = 9 * pixels;
    words[TAP_WORDS + EVAL_WORD] = 9 * pixels;
    words[2 * TAP_WORDS + EVAL_WORD] = pixels;
    words[3 * TAP_WORDS + EVAL_WORD] = pixels;
    h.state_mut().ingest_probe(out, &words);
    h.run_steps(2);

    let name = format!("blur{blur} sampling 9.0/px");
    assert!(!h.state().preferences().show_costs, "still off");
    h.get_by_label_contains(&name);

    // Turning the strip on does not duplicate or replace the header's own warning.
    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Costs").click();
    h.run_steps(2);
    h.get_by_label_contains(&name);
}

/// **A trace folds under a Trace heading, open on a new node**, on every node that draws one
/// and on Automation's recorded curve: the bar stays when it is closed, and the band goes.
#[test]
fn a_trace_folds_under_its_heading() {
    for slug in ["adsr", "animation", "automation", "oscillator"] {
        let mut h = tall_harness();
        let workspace = h.state().graph().default_workspace();
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at: Pos2::ZERO,
                workspace,
            })
            .unwrap();
        h.run_steps(3);
        let id = h.state().graph().iter().next().expect("one node").0;
        let heading = format!("{slug}{id}.trace");
        assert!(
            h.query_by_label(&heading).is_some(),
            "{slug} has a Trace heading"
        );
        let tall = node_height(&h, id);
        h.get_by_label(&heading).click();
        h.run_steps(2);
        assert_eq!(
            h.state()
                .graph()
                .get(id)
                .unwrap()
                .options
                .get("trace")
                .map(String::as_str),
            Some("off"),
            "{slug}: the heading turns an ordinary option"
        );
        assert!(node_height(&h, id) < tall, "{slug}: the band went");
        assert!(
            h.query_by_label(&heading).is_some(),
            "{slug}: the heading stays to open it again"
        );
    }
}

/// **The time rows fold under a Timing heading that starts closed**, on every node that
/// moves with time, with its mode on the bar. Closed, no time row is drawn and the heading
/// says they are there; open, Speed and Offset are, and Time is not, since a new node runs
/// free. A cable into a folded Time still lands somewhere: its port gathers on the heading's
/// left edge, where the rows would be, so the cable is drawn into the heading.
#[test]
fn the_time_rows_fold_under_a_closed_timing_heading() {
    let mut h = tall_harness();
    let workspace = h.state().graph().default_workspace();
    for slug in ["time", "perlin"] {
        let at = if slug == "time" {
            Pos2::new(0.0, 400.0)
        } else {
            Pos2::ZERO
        };
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at,
                workspace,
            })
            .unwrap();
    }
    h.run_steps(3);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (seconds, perlin) = (ids[0], ids[1]);
    let heading = format!("perlin{perlin}.timing");
    let offset = format!("perlin{perlin}.phaseOffset");
    let speed = format!("perlin{perlin}.speed");
    assert!(h.query_by_label(&heading).is_some(), "a Timing heading");
    assert!(
        h.query_by_label(&format!("perlin{perlin}.clockMode.free"))
            .is_some(),
        "its mode on the bar, closed as it is"
    );
    assert!(
        h.query_all_by_label_contains(&offset).next().is_none(),
        "closed on a new node: no Offset row"
    );
    let closed = node_height(&h, perlin);
    let rows_of = |h: &Harness<'_, App>| {
        laid_out(h, perlin)
            .find(perlin)
            .unwrap()
            .rows
            .iter()
            .map(|r| r.row)
            .collect::<Vec<_>>()
    };
    assert!(
        !rows_of(&h)
            .iter()
            .any(|r| matches!(r, supersilvia::ui::canvas::Row::Input(3..=5))),
        "no time row has a row"
    );

    // A clock cabled into the folded Time, in Loop mode, is drawn into the heading.
    h.state_mut()
        .apply(Command::SetOption {
            node: perlin,
            key: "clockMode",
            value: "loop".to_string(),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: supersilvia::graph::PortRef::new(seconds, "seconds"),
            to: supersilvia::graph::PortRef::new(perlin, "clock"),
        })
        .unwrap();
    h.run_steps(2);
    let laid = laid_out(&h, perlin);
    let node = laid.find(perlin).unwrap();
    let bar = node
        .block(supersilvia::ui::canvas::Row::TimingHeading)
        .expect("the heading has a row");
    let slot = node
        .ports
        .iter()
        .find(|p| p.port.key == "clock")
        .expect("a cabled Time keeps a dot while folded");
    assert_eq!(slot.center.x, node.rect.min.x, "on the body's left edge");
    assert!(
        (slot.center.y - bar.center().y).abs() < 0.5,
        "at the heading's middle"
    );
    assert!(
        node.ports.iter().all(|p| p.port.key != "phaseOffset"),
        "an Offset with nothing on it has no dot while folded"
    );

    h.get_by_label(&heading).click();
    h.run_steps(2);
    assert_eq!(
        h.state()
            .graph()
            .get(perlin)
            .unwrap()
            .options
            .get("timing")
            .map(String::as_str),
        Some("on")
    );
    assert!(
        h.query_all_by_label_contains(&offset).next().is_some(),
        "open, the Offset row is there"
    );
    assert!(
        h.query_all_by_label_contains(&speed).next().is_none(),
        "and in Loop mode Speed is not"
    );
    let looping = node_height(&h, perlin);
    assert!(looping > closed, "and the node grew by the rows");
    assert!(h.query_by_label(&heading).is_some(), "the heading stays");

    // Free on the bar: the Time's cable goes, Speed stands where Time stood, and the node
    // keeps its height.
    h.get_by_label(&format!("perlin{perlin}.clockMode.free"))
        .click();
    h.run_steps(2);
    let node = h.state().graph().get(perlin).unwrap();
    assert_eq!(
        node.options.get("clockMode").map(String::as_str),
        Some("free")
    );
    assert!(
        h.state()
            .graph()
            .source_of(supersilvia::graph::PortRef::new(perlin, "clock"))
            .is_none(),
        "the switch dropped the Time's cable"
    );
    assert!(
        h.query_all_by_label_contains(&speed).next().is_some(),
        "Speed is drawn"
    );
    assert_eq!(
        node_height(&h, perlin),
        looping,
        "in place, at the same height"
    );
    assert_eq!(
        h.state()
            .graph()
            .get(perlin)
            .unwrap()
            .options
            .get("timing")
            .map(String::as_str),
        Some("on"),
        "a click on the mode is not a click on the fold"
    );
}

/// **A Time row in Loop mode carries a loop meter where the Speed knob stands in Free mode**,
/// in the knob's own place. Held at 5.25 seconds: a Perlin at Repeat 4, half a cell a second,
/// is 2.625 cells in, the third of four; one at Repeat Never is two whole cells in, its end
/// open; an Oscillator, a CPU node a cycle a second, is in its one cycle. A gear's Cycles
/// cabled in is what it reads, and Free mode puts the knob back where the meter was.
#[test]
fn a_loop_meter_stands_where_the_speed_knob_does() {
    use supersilvia::graph::PortRef;
    let mut h = tall_harness();
    h.step();
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    h.state_mut()
        .transport(supersilvia::transport::Command::Seek(5.25));
    let workspace = h.state().graph().default_workspace();
    for (slug, at) in [
        ("perlin", Pos2::new(40.0, 40.0)),
        ("perlin", Pos2::new(300.0, 40.0)),
        ("oscillator", Pos2::new(40.0, 520.0)),
    ] {
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at,
                workspace,
            })
            .unwrap();
    }
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (looped, open, osc) = (ids[0], ids[1], ids[2]);
    h.state_mut()
        .apply(Command::SetOption {
            node: looped,
            key: "repeat",
            value: "4".to_string(),
        })
        .unwrap();
    for id in [looped, open, osc] {
        for (key, value) in [("clockMode", "loop"), ("timing", "on")] {
            h.state_mut()
                .apply(Command::SetOption {
                    node: id,
                    key,
                    value: value.to_string(),
                })
                .unwrap();
        }
    }
    h.run_steps(4);
    assert!(
        h.query_by_label(&format!("perlin{looped}.clock 3/4"))
            .is_some()
    );
    assert!(h.query_by_label(&format!("perlin{open}.clock 2")).is_some());
    assert!(
        h.query_by_label(&format!("oscillator{osc}.clock 1/1"))
            .is_some()
    );
    h.snapshot("loop_meter");

    // A Ratio Gear with nothing in it counts ambient seconds: 5.25, the second of four.
    add_node(&mut h, ADD_PHASE);
    let gear = h
        .state()
        .graph()
        .iter()
        .map(|(id, _)| id)
        .find(|id| ![looped, open, osc].contains(id))
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(gear, "cycles"),
            to: PortRef::new(looped, "clock"),
        })
        .unwrap();
    h.run_steps(4);
    assert!(
        h.query_by_label(&format!("perlin{looped}.clock 2/4"))
            .is_some(),
        "the gear's Cycles is what it reads"
    );

    // Free: the Speed knob stands exactly where the meter stood.
    let meter = h.get_by_label(&format!("perlin{open}.clock 2")).rect();
    h.state_mut()
        .apply(Command::SetOption {
            node: open,
            key: "clockMode",
            value: "free".to_string(),
        })
        .unwrap();
    h.run_steps(2);
    assert!(
        h.query_by_label(&format!("perlin{open}.clock 2")).is_none(),
        "no meter in Free mode"
    );
    assert_eq!(rect_of(&h, &format!("perlin{open}.speed 1")), meter);
}

/// The trace nodes widen for the trace on their body, the same 300 the audio scope already
/// uses — end to end from `NodeDef::trace` through `canvas::node_width` to what the body
/// ground actually measures.
#[test]
fn a_trace_node_widens_for_its_trace() {
    let mut h = harness();
    let workspace = h.state().graph().default_workspace();
    for slug in ["adsr", "animation", "checkerboard"] {
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at: Pos2::ZERO,
                workspace,
            })
            .unwrap();
    }
    h.run_steps(3);

    let width_of = |h: &mut Harness<'_, App>, id: supersilvia::graph::NodeId, slug: &str| {
        h.get_by_label_contains(&format!("{slug}{id} body"))
            .rect()
            .width()
    };
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (adsr, animation, checkerboard) = (ids[0], ids[1], ids[2]);

    assert_eq!(
        width_of(&mut h, adsr, "adsr"),
        supersilvia::ui::canvas::SCOPE_NODE_WIDTH,
        "adsr carries a trace"
    );
    assert_eq!(
        width_of(&mut h, animation, "animation"),
        supersilvia::ui::canvas::SCOPE_NODE_WIDTH,
        "animation carries a trace"
    );
    assert_eq!(
        width_of(&mut h, checkerboard, "checkerboard"),
        supersilvia::ui::canvas::NODE_WIDTH,
        "an ordinary node stays the default width"
    );
}

/// The divider's status line is on the node, not only in the node's own report: silvia's
/// one line of feedback, which is what says a divider on a slow clock is alive and how far
/// off the next pass is. Drawn as a region, and named, so the tree carries what it says.
#[test]
fn a_dividers_status_line_is_drawn_on_its_body() {
    let mut h = harness();
    let workspace = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddNode {
            slug: "clockdivider",
            at: Pos2::ZERO,
            workspace,
        })
        .unwrap();
    h.run_steps(3);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();

    let name = format!("clockdivider{id}.status");
    let band = h.get_by_label_contains(&name).rect();
    let body = h
        .get_by_label_contains(&format!("clockdivider{id} body"))
        .rect();
    assert!(
        body.contains_rect(band),
        "the line is inside the body: {band:?} in {body:?}"
    );
    h.get_by_label_contains(&format!("clockdivider{id}.status Ready ÷4"));
}

/// A node's name in the Status box is a link: clicking it shows the node's workspace.
#[test]
fn a_name_in_the_status_box_goes_to_the_node() {
    use supersilvia::graph::WorkspaceKind;
    use supersilvia::project::Active;

    // Tall enough for the whole box: the Nodes section is last, under every other.
    let mut h = tall_harness();
    let first = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddWorkspace {
            name: "Second".to_string(),
            kind: WorkspaceKind::Video,
            layout: supersilvia::graph::LayoutMode::default(),
            // These tests want a bare workspace; the app seeds a video tab with a patch.
            seed: supersilvia::command::Seed::Empty,
        })
        .unwrap();
    let second = h.state().graph().workspaces().last().unwrap().id;
    // A gear has a line of its own in the box; put it on the other workspace.
    h.state_mut()
        .apply(Command::AddNode {
            slug: "ratiogear",
            at: Pos2::ZERO,
            workspace: second,
        })
        .unwrap();
    let phase = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut().open_workspace(second);
    h.state_mut().activate(Active::Workspace(first));
    h.run_steps(2);
    assert_eq!(h.state().active(), Active::Workspace(first));

    // The tick's phases land a few ticks after the box opens, into rows already there.
    open_status_box(&mut h);
    // The Nodes section opens folded; its heading unfolds it.
    h.get_by_label("Nodes section").click();
    h.run_steps(2);
    h.get_by_label(&format!("go to ratiogear{phase}")).click();
    h.run_steps(2);
    assert_eq!(
        h.state().active(),
        Active::Workspace(second),
        "the link showed the node's workspace"
    );
}

/// Open the Status box and let the tick's phases land. The preference a hand ticks under
/// Preferences ▸ Performance, which `the_frame_pacing_toggle_is_a_preference_and_not_a_command`
/// ticks by hand.
fn open_status_box(h: &mut Harness<'_, App>) {
    h.state_mut().set_show_status_box(true);
    h.run_steps(6);
}

/// The verdict is the box's first line: the rate, then what holds it back. With no GPU
/// reading — a headless app has none — the CPU is what it names, and there is no GPU section.
#[test]
fn the_status_box_opens_on_a_verdict() {
    let mut h = tall_harness();
    h.step();
    open_status_box(&mut h);
    let verdict = h
        .query_all_by_label_contains("CPU  ")
        // A label's text is its accesskit value.
        .filter_map(|n| n.accesskit_node().value())
        .find(|l| l.contains("held back by") || l.contains("busiest"));
    assert!(verdict.is_some(), "the verdict names the CPU");
    assert!(
        h.query_by_label("GPU per tick section").is_none(),
        "no GPU section without a GPU reading"
    );
    assert!(h.query_by_label("CPU per tick section").is_some());
}

/// A section folds on its heading, and the fold is a preference, not a command.
#[test]
fn a_status_box_section_folds_and_is_remembered() {
    let mut h = tall_harness();
    h.step();
    open_status_box(&mut h);
    assert!(
        h.state().preferences().status_folds.editor,
        "folded by default"
    );
    assert!(
        h.query_by_label_contains("CPU per frame").is_none(),
        "so its rows are not drawn"
    );

    h.get_by_label("Editor section").click();
    h.run_steps(2);
    assert!(!h.state().preferences().status_folds.editor);
    assert!(h.query_by_label_contains("CPU per frame").is_some());
    assert!(h.state().history().is_empty(), "no command was issued");

    h.get_by_label("CPU per tick section").click();
    h.run_steps(2);
    assert!(h.state().preferences().status_folds.cpu);
    assert!(h.query_by_label_contains("waiting on GPU").is_none());
}

/// **The copy is the whole box.** Ten Outputs: the window lists the costliest eight and
/// counts the rest, and the text on the clipboard lists all ten with every section open.
#[test]
fn the_status_box_copies_everything_it_holds() {
    let mut h = tall_harness();
    let ws = h.state().graph().default_workspace();
    for i in 0..10 {
        h.state_mut()
            .apply(Command::AddNode {
                slug: "output",
                at: Pos2::new(i as f32 * 260.0, 0.0),
                workspace: ws,
            })
            .unwrap();
    }
    h.step();
    open_status_box(&mut h);
    assert_eq!(
        h.query_all_by_label_contains("go to output").count(),
        8,
        "the window lists eight"
    );
    assert!(
        h.query_by_label_contains("…2 more").is_some(),
        "and counts the rest"
    );

    h.get_by_label("copy the Status box").click();
    h.run_steps(1);
    let copied = h
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .expect("the click put the box on the clipboard");
    assert!(copied.starts_with("┌ STATUS"), "{copied}");
    assert!(copied.trim_end().ends_with('┘'), "{copied}");
    assert_eq!(
        copied.lines().filter(|l| l.contains("▸ go")).count(),
        10,
        "every Output is in the copy:\n{copied}"
    );
    assert!(
        !copied.lines().any(|l| l.starts_with("├ ▸")),
        "with every section open:\n{copied}"
    );
    assert!(copied.contains("CPU per frame"), "the Editor's rows too");
    let width = copied.lines().next().unwrap().chars().count();
    assert!(
        copied.lines().all(|l| l.chars().count() == width),
        "every line the box's width:\n{copied}"
    );
}

/// Every character the Status box draws is one cell of the monospace face, or its frame
/// would not meet its own right edge: the box-drawing, the blocks, the folds and the copy mark.
#[test]
fn the_status_box_draws_in_whole_cells() {
    let mut h = harness();
    h.step();
    let font = egui::FontId::monospace(supersilvia::ui::theme::FONT_BASE);
    let odd: Vec<String> = h.ctx.fonts_mut(|fonts| {
        let cell = fonts.glyph_width(&font, 'a');
        let mut odd = Vec::new();
        for c in "┌┐└┘├┤│─█░▏▎▍▌▋▊▉▾▸◰…·—×".chars() {
            let width = fonts.glyph_width(&font, c);
            if (width - cell).abs() > 0.01 {
                odd.push(format!("{c} U+{:04X} {width}", c as u32));
            }
        }
        odd
    });
    assert!(odd.is_empty(), "not one cell wide: {odd:?}");
}

/// The node under test, as the graph now holds it.
fn only_node(h: &Harness<'_, App>) -> supersilvia::graph::Node {
    let graph = h.state().graph();
    let id = graph.iter().next().expect("one node").0;
    graph.get(id).expect("just found").clone()
}

/// silvia's show/hide checkmarks and the headings beside them. **The ticks row is the
/// port-visibility control and nothing else** — `uniforms` and `events` hide runs of *rows*,
/// which have no heading to sit on — while a region that can close carries its own heading and
/// its own triangle. All four are the same ordinary option underneath: in the tree, in the
/// file and in the undo history.
#[test]
fn the_show_hide_ticks_and_the_region_headings_hide_their_own_part_of_the_node() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_VIDEO);

    for label in [
        "video1.uniforms",
        "video1.events",
        "video1.scope",
        "video1.preview",
    ] {
        assert!(
            h.query_by_label(label).is_some(),
            "tick {label:?} is missing"
        );
    }
    assert!(
        h.query_by_label("video1.show").is_none(),
        "and the select they replaced is gone"
    );
    assert_eq!(
        only_node(&h).def.checks(),
        2,
        "the ticks row is the two port groups"
    );
    assert_eq!(
        only_node(&h).def.headings(),
        3,
        "and the scope and the preview are headings on their own regions, and Time on its rows"
    );

    let video = h.state().graph().iter().next().expect("one node").0;
    let tall = node_height(&h, video);
    assert_eq!(
        only_node(&h).options.get("scope").map(String::as_str),
        Some("on")
    );

    h.get_by_label("video1.scope").click();
    h.step();
    assert_eq!(
        only_node(&h).options.get("scope").map(String::as_str),
        Some("off"),
        "the triangle set the same option the tick did"
    );
    let closed = node_height(&h, video);
    assert!(closed < tall, "and the node lost the scope's height");
    // A closed region still says it is there: the heading stays, which is the whole argument
    // for a triangle over a tick in a row somewhere else on the node.
    assert!(
        h.query_by_label("video1.scope").is_some(),
        "the heading is still there to open it again"
    );
    assert_eq!(
        closed,
        tall - supersilvia::ui::canvas::SCOPE_HEIGHT,
        "the band went and the heading over it stayed"
    );
    // An edit, not a gesture: adding the node and hiding the scope are two steps back.
    assert_eq!(h.state().history().len(), 2);
}

/// The node's hit rect is the node's business and not one size.
///
/// silvia's two readouts under its envelope graph: whether the gate is on, and which stage
/// the envelope is in, on the node rather than away from it in the Status box.
#[test]
fn an_envelope_captions_its_trace_with_the_gate_and_the_stage() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_ADSR);
    let id = h.state().graph().iter().next().expect("the adsr").0;
    h.run_steps(4);

    h.get_by_label_contains(&format!("adsr{id}.gate OFF"));
    h.get_by_label_contains(&format!("adsr{id}.stage Idle"));

    h.state_mut()
        .press(supersilvia::graph::PortRef::new(id, "gate"), true);
    h.run_steps(4);
    h.get_by_label_contains(&format!("adsr{id}.gate ON"));
    // A default attack is a hundredth of a second, so four frames in it is holding at its
    // sustain rather than still rising.
    h.get_by_label_contains(&format!("adsr{id}.stage Sustain"));
    h.snapshot("adsr_caption");
}

/// A region that does not claim the pointer — a read-only trace, a thumb — lets a drag
/// through to the body under it, so a hand carries the node by its trace exactly as by the
/// whitespace beside a port. The region that *does* claim it is the pad's, and that half is
/// held in `widgets`' own tests, where a region can be declared without a node to hang it on.
#[test]
fn a_drag_on_a_region_that_does_not_claim_the_pointer_carries_the_node() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_ADSR);
    let id = h.state().graph().iter().next().expect("the adsr").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(60.0, 40.0))],
        })
        .expect("a node that is there");
    h.step();

    let before = h.state().graph().get(id).expect("the adsr").pos;
    let at = {
        let t = h.state().canvas_transform();
        let origin = h.state().canvas_origin();
        let band = laid_out(&h, id)
            .find(id)
            .expect("the adsr")
            .region(supersilvia::ui::canvas::Region::Declared(0));
        assert!(band.height() > 0.0, "the adsr draws its trace");
        t.to_screen(origin, band.center())
    };
    let by = egui::vec2(30.0, 20.0);
    press_at(&mut h, at);
    move_to(&mut h, at + by);
    release_at(&mut h, at + by);

    assert_eq!(
        h.state().graph().get(id).expect("the adsr").pos,
        before + by,
        "the drag reached the body under the trace"
    );
}

/// A picture carries the two marks a video player puts on one — pop out, and fullscreen —
/// and the first of them asks for the window. The window itself is the pictures thread's,
/// which a harness has none of, so what is asserted is the editor's own list of what should
/// have one — the same list the marks read to know whether to light.
#[test]
fn a_pictures_pop_out_mark_opens_a_window_of_its_own() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    let id = h.state().graph().iter().next().expect("the output").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(40.0, 40.0))],
        })
        .expect("a node that is there");
    h.step();

    assert!(h.state().popped_out().is_empty());
    assert!(
        h.query_by_label("fullscreen output1").is_some(),
        "and the fullscreen mark is its own button"
    );
    h.get_by_label("pop out output1").click();
    h.run_steps(2);

    assert_eq!(
        h.state().popped_out().len(),
        1,
        "one window, for one picture"
    );
    // The mark is a toggle, as **Open projector** was: the same click puts the window away.
    h.get_by_label("pop out output1").click();
    h.run_steps(2);
    assert!(h.state().popped_out().is_empty());
}

/// What the ticks, the headings and the picture look like: the row of checkmarks at the foot
/// of the options, the closed scope's own heading under it, and the clip's band under that
/// with the expand mark on it.
#[test]
fn a_video_node_draws_its_ticks_and_its_picture() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_VIDEO);
    let id = h.state().graph().iter().next().expect("the video").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(40.0, 20.0))],
        })
        .expect("a node that is there");
    h.step();
    // The numbers, the events and the scope hidden: what is left is the node's picture,
    // which is the combination the one `show` select could not reach.
    // Two frames a tick: the command lands at the end of the frame the click is on, so the
    // row the next click is aimed at has not moved up yet on the frame after it.
    for tick in ["video1.uniforms", "video1.events", "video1.scope"] {
        h.get_by_label(tick).click();
        h.run_steps(2);
    }
    h.snapshot("video_ticks_and_picture");
}

/// The two simulations that are worth watching draw themselves, under the same Preview
/// heading a clip's band is under.
///
/// A game and an automaton are the two nodes where the state *is* the thing: a press of Step
/// or of Paddle Left that changes nothing on screen is a control nobody can learn. Both
/// publish a texture already, so the picture is the ordinary
/// `widgets::picture::preview` region and nothing new in `render/`. The snapshot is the two
/// bands and the rows above them — including the knob that used to be a second Ball Speed and
/// is now Launch Speed.
#[test]
fn the_game_and_the_automaton_draw_their_state_on_the_node() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_BRICKGAME);
    add_node(&mut h, ADD_CELLULARAUTOMATA);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2, "one of each");
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (ids[0], Pos2::new(20.0, 10.0)),
                (ids[1], Pos2::new(300.0, 10.0)),
            ],
        })
        .expect("nodes that are there");
    h.run_steps(2);

    for (slug, port) in [("brickgame", "field"), ("cellularautomata", "cells")] {
        let id = h
            .state()
            .graph()
            .iter()
            .find(|(_, n)| n.def.slug == slug)
            .map(|(id, _)| id)
            .expect("the node");
        let name = format!("{slug}{id}");
        assert!(
            h.query_by_label(&format!("{name}.preview")).is_some(),
            "{name} has no Preview heading to close its picture by"
        );
        assert!(
            preview_band(&h, id).height() > 0.0,
            "{name} reserves no band for its {port}"
        );
    }

    // The knob and the readout are two different names now, both on the same game.
    let game = supersilvia::nodes::find("brickgame").expect("in the registry");
    let knob = game
        .inputs
        .iter()
        .find(|i| i.key == "ballSpeed")
        .expect("the launch knob");
    let readout = game
        .outputs
        .iter()
        .find(|o| o.key == "ballVelocity")
        .expect("the live readout");
    assert_eq!(knob.label, "Launch Speed");
    assert_eq!(readout.label, "Ball Speed");
    assert_ne!(knob.label, readout.label, "two rows, two names");

    h.snapshot("simulation_nodes_draw_their_state");
}

/// The two sources that swapped: a picture onto the node whose whole content is one, and
/// meters onto the node whose picture is already on the panel.
///
/// `imagegif` gets the clip node's own **Preview** region — silvia draws the loaded image in
/// the node, so a row of them reads as a contact sheet, and here the only sign of what was
/// loaded was a file name elided from the front. `maininput` loses it and gets silvia's three
/// level bars instead, with the panel's threshold square on each: a level is set while the
/// band it measures is being watched, and the picture is a few inches to the left already.
#[test]
fn the_picture_moves_to_the_image_node_and_the_meters_to_the_main_input() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_IMAGEGIF);
    add_node(&mut h, ADD_MAININPUT);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    assert_eq!(ids.len(), 2, "one of each");
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (ids[0], Pos2::new(20.0, 10.0)),
                (ids[1], Pos2::new(280.0, 10.0)),
            ],
        })
        .expect("nodes that are there");
    // Enough frames for the synth to have ticked both and for its snapshot to have come
    // back: the meters draw what `CpuNode::scope` published, and a node that has not ticked
    // has published nothing.
    h.run_steps(6);

    let node = |h: &Harness<'_, App>, slug: &str| {
        h.state()
            .graph()
            .iter()
            .find(|(_, n)| n.def.slug == slug)
            .map(|(id, _)| id)
            .expect("the node")
    };

    let id = node(&h, "imagegif");
    assert!(
        h.query_by_label(&format!("imagegif{id}.preview")).is_some(),
        "the Image/GIF node has no Preview heading to close its picture by"
    );
    assert!(
        preview_band(&h, id).height() > 0.0,
        "and reserves no band for it"
    );

    let id = node(&h, "maininput");
    assert_eq!(
        preview_band(&h, id).height(),
        0.0,
        "the Main Input still draws the picture the panel is already showing"
    );
    // Named as the scope's meters are, by the node kind: `ui::scope::Owner::node` salts its
    // egui ids with the node id and names the widget for the slug.
    for band in ["bass", "mid", "high"] {
        assert!(
            h.query_by_label_contains(&format!("maininput.{band} threshold"))
                .is_some(),
            "no {band} meter on the node"
        );
    }
    // The two ticks silvia has, in place of the ten rows that were there whatever the patch
    // listened to.
    for tick in ["uniforms", "events"] {
        assert!(
            h.query_by_label(&format!("maininput{id}.{tick}")).is_some(),
            "no {tick} tick"
        );
    }

    h.snapshot("the_picture_and_the_meters");
}

/// The strip a picture carries: a scrubber where the node is playing something, a speaker and
/// a volume where it has a monitor to set. Both are second editors for things that already
/// exist — the node's own position, the `monitor` control — and neither is drawn until a hand
/// is on the picture.
#[test]
fn a_pictures_strip_scrubs_and_sets_the_monitor() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_VIDEO);
    let id = h.state().graph().iter().next().expect("the video").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(40.0, 20.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);

    // Nothing until the pointer is on the picture, and the picture is the foot of the body.
    let picture = preview_band(&h, id);
    assert!(picture.height() > 0.0, "the clip's band is drawn");
    assert!(
        h.query_by_label_contains("video1 volume").is_none(),
        "the strip is not there with the pointer elsewhere"
    );

    let at = h.state().canvas_origin() + picture.center().to_vec2();
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    h.get_by_label_contains("video1 volume");
    // The speaker says which way it goes: a monitor at zero offers to unmute.
    h.get_by_label("video1 unmute");
    // No clip is open in a headless test, so there is nothing to scrub through — the speaker
    // is the half that does not need a file.
    assert!(h.query_by_label_contains("video1 scrubber").is_none());

    // The speaker sets the `monitor` control, so it is an ordinary undoable edit and the
    // number row on the node moves with it.
    let before = h.state().history().len();
    h.get_by_label("video1 unmute").click();
    h.run_steps(2);
    assert_eq!(
        only_node(&h).controls.get("monitor"),
        Some(&supersilvia::graph::ControlValue::Float(0.5)),
        "an unmute goes to half"
    );
    assert_eq!(h.state().history().len(), before + 1, "one edit, one step");
    h.get_by_label("video1 mute").click();
    h.run_steps(2);
    assert_eq!(
        only_node(&h).controls.get("monitor"),
        Some(&supersilvia::graph::ControlValue::Float(0.0)),
        "and a mute silences it"
    );
}

/// **A dual output's port and readout follow its mode.** Two knobs and nothing connected is
/// a constant, so the `add` draws a diamond with the number it published against it.
#[test]
fn a_math_node_with_knobs_draws_a_diamond_and_a_readout() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_ADD);
    let sum = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut()
        .apply(Command::SetControl {
            node: sum,
            key: "a",
            value: supersilvia::graph::ControlValue::Float(1.5),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::SetControl {
            node: sum,
            key: "b",
            value: supersilvia::graph::ControlValue::Float(0.25),
        })
        .unwrap();
    h.run_steps(3);

    assert_eq!(
        h.state().uniform(PortRef::new(sum, "output")),
        Some(1.75),
        "the tick evaluated the formula"
    );
    assert!(
        h.query_by_label("add1.output (uniform number output) 1.75")
            .is_some(),
        "the port is a uniform number and says what it published",
    );
    h.snapshot("dual_math_diamond");
}

/// A field on one input and it is a field node again: a circle, and nothing to read off it.
#[test]
fn a_field_cabled_into_a_math_node_draws_a_circle() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_ADD);
    add_node(&mut h, ADD_LUMINOSITY);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (sum, luma) = (ids[0], ids[1]);
    // Below the `add` rather than beside it, so both nodes and the cable between them are
    // in the frame.
    let below = h.state().graph().get(sum).unwrap().pos + egui::vec2(0.0, 180.0);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(luma, below)],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(luma, "output"),
            to: PortRef::new(sum, "a"),
        })
        .expect("a field into A");
    h.run_steps(3);

    assert!(
        h.query_by_label("add1.output (varying number output)")
            .is_some(),
        "the port went back to being a field",
    );
    assert_eq!(
        h.state().uniform(PortRef::new(sum, "output")),
        None,
        "and there is no number on it"
    );
    h.snapshot("dual_math_circle");
}

/// **Reframe Range's named ranges write its knobs and let go.** silvia's Slide Rule picks the
/// common conversions as two names; here they are two rows of buttons on the general node, so
/// a press is one edit that leaves the bounds knobs with ports on them.
#[test]
fn a_named_range_writes_reframe_ranges_bounds_and_swap_turns_the_map_round() {
    use supersilvia::graph::ControlValue::Float;

    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_REFRAMERANGE);
    let before = h.state().history().len();

    h.get_by_label("reframerange1.inMin.degrees").click();
    h.run_steps(2);
    assert_eq!(only_node(&h).controls.get("inMin"), Some(&Float(0.0)));
    assert_eq!(only_node(&h).controls.get("inMax"), Some(&Float(360.0)));
    assert_eq!(
        h.state().history().len(),
        before + 1,
        "one press, one undo step"
    );

    h.get_by_label("reframerange1.outMin.byte").click();
    h.run_steps(2);
    assert_eq!(only_node(&h).controls.get("outMin"), Some(&Float(0.0)));
    assert_eq!(only_node(&h).controls.get("outMax"), Some(&Float(255.0)));

    // Swap is Slide Rule's pair of Invert boxes: an inside-out output range runs the map
    // backwards, which this node already does without a mode of its own.
    h.get_by_label("reframerange1.swap").click();
    h.run_steps(2);
    assert_eq!(only_node(&h).controls.get("outMin"), Some(&Float(255.0)));
    assert_eq!(only_node(&h).controls.get("outMax"), Some(&Float(0.0)));

    h.snapshot("reframerange_named_ranges");
}

/// **Random Seq rolls a sequence onto Lyapunov's own field.** The one button under its rows
/// writes `sequence` through the bus — one press, one undo step — with a sequence silvia's
/// `randomSequence` could have rolled, and the field shows it.
#[test]
fn random_seq_writes_a_fresh_sequence_in_one_step() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_LYAPUNOV);
    let sequence = |h: &Harness<'_, App>| only_node(h).options["sequence"].clone();
    assert_eq!(sequence(&h), "A6B6");
    let before = h.state().history().len();

    h.get_by_label("lyapunov1.randomize").click();
    h.run_steps(2);
    let rolled = sequence(&h);
    assert_ne!(rolled, "A6B6");
    assert!((2..=11).contains(&rolled.len()), "{rolled:?}");
    assert!(rolled.contains('A') && rolled.contains('B'), "{rolled:?}");
    assert_eq!(
        h.state().history().len(),
        before + 1,
        "one press, one undo step"
    );

    h.get_by_label("lyapunov1.randomize").click();
    h.run_steps(2);
    assert_eq!(
        h.state().history().len(),
        before + 2,
        "and the next press is its own"
    );

    h.snapshot("lyapunov_random_seq");
}

/// **silvia's preset bar, on the slime mold.** Nine buttons numbered from one under the rows;
/// a press writes the three sensing knobs and the Mode as one edit, and holds the nudge for
/// the tick to hear, as a press button holds its action input.
#[test]
fn a_slime_mold_preset_writes_its_knobs_and_mode_in_one_step() {
    use supersilvia::graph::ControlValue::Float;

    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_SLIMEMOLD);
    let id = h.state().graph().iter().next().expect("one node").0;
    let before = h.state().history().len();

    h.get_by_label("slimemold1.preset6").click();
    let nudge = PortRef::new(id, supersilvia::nodes::slimemold::NUDGE);
    let mut held = h.state().is_held(nudge);
    for _ in 0..2 {
        h.step();
        held |= h.state().is_held(nudge);
    }
    let node = only_node(&h);
    assert_eq!(node.options["mode"], "repel");
    assert_eq!(node.controls.get("sensorAngle"), Some(&Float(168.0)));
    assert_eq!(node.controls.get("rotationAngle"), Some(&Float(44.2)));
    assert_eq!(node.controls.get("sensorOffset"), Some(&Float(37.0)));
    assert_eq!(
        h.state().history().len(),
        before + 1,
        "one press, one undo step"
    );
    assert!(held, "and the tick was handed the nudge");

    h.snapshot("slimemold_presets");
}

/// **A pinned dual node's inputs are diamonds too.** With a `slew` reading the `add`'s
/// number, a field can no longer land on either input — so both draw as an unconnected
/// `UniformNumber` input does, an outlined diamond, and a hand sees what may go there
/// before it drags anything at it.
#[test]
fn a_math_node_a_slew_reads_draws_diamond_inputs() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_ADD);
    add_node(&mut h, ADD_SLEW);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (sum, slew) = (ids[0], ids[1]);
    // To the right, so the cable runs the way the data does and both nodes are in the frame.
    let beside = h.state().graph().get(sum).unwrap().pos + egui::vec2(300.0, 0.0);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(slew, beside)],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(sum, "output"),
            to: PortRef::new(slew, "input"),
        })
        .expect("a diamond feeds a diamond");
    h.run_steps(3);

    for key in ["a", "b"] {
        assert!(
            h.query_by_label(&format!("add1.{key} (uniform number input)"))
                .is_some(),
            "add1.{key} is not a number: {:?}",
            h.state().graph().get(sum).unwrap().inputs,
        );
    }
    h.snapshot("dual_math_pinned");
}

/// Where a port's dot is on screen, by the name the accessibility tree carries.
fn port_at(h: &Harness<'_, App>, label: &str) -> Pos2 {
    h.get_by_label(label).rect().center()
}

/// A color released on a number opens the conversion menu, and a row in it is one command.
///
/// The whole gesture, end to end: the cable cannot land on a `VaryingNumber` input, so the
/// port is convertible rather than dead, the release opens the menu at it, and choosing
/// *Luminosity* adds one node and two cables — which one undo takes back together.
#[test]
fn a_color_dropped_on_a_number_offers_a_conversion_and_takes_it() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_ZOOM);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, zoom) = (ids[0], ids[1]);
    // Clear to the right, so the two nodes' ports do not overlap and the drag crosses open
    // canvas the way a hand's would.
    let beside = h.state().graph().get(cb).unwrap().pos + egui::vec2(300.0, 0.0);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(zoom, beside)],
        })
        .unwrap();
    h.run_steps(2);

    let from = port_at(&h, "checkerboard1.output (varying color output)");
    let to = port_at(&h, "zoom2.zoom (varying number input)");
    drag_with(&mut h, from, to, egui::Modifiers::NONE);
    // Two frames: the menu's `Area` appears on the first, which is a sizing pass, and the
    // tree is only geometry on the second.
    h.run_steps(2);

    assert!(
        h.state().graph().connections().is_empty(),
        "the cable itself cannot land"
    );
    let row = "convert with luminosity.input to output";
    assert!(
        h.query_by_label(row).is_some(),
        "the conversion menu is not on screen"
    );
    assert!(
        h.query_by_label("convert with channelsplitter.input to r")
            .is_some(),
        "and the splitter's channels are rows of their own"
    );
    // The curated table, in a picture: eight castings named for what they do, not eight nodes
    // and not every node that could sit between the two.
    h.snapshot("conversion_menu");

    h.get_by_label(row).click();
    h.run_steps(2);

    assert_eq!(h.state().graph().len(), 3, "the luminosity is not there");
    assert_eq!(h.state().graph().connections().len(), 2, "wired both sides");
    assert!(
        h.query_by_label(row).is_none(),
        "the menu stayed open after a choice"
    );

    assert!(h.state_mut().undo());
    h.step();
    assert_eq!(h.state().graph().len(), 2, "one undo, all three");
    assert!(h.state().graph().connections().is_empty());
}

/// The third port state, in a picture: a ring of dashes around every port a node could carry
/// this cable to.
///
/// Mid-drag, with no release — the cable is still in flight, which is the only time the ring
/// is drawn. The `zoom`'s three numbers wear one and its color input does not, because that
/// one can simply take the cable; the checkerboard's own inputs are dimmed rather than ringed,
/// since a node cannot feed itself whatever sits between.
#[test]
fn a_port_a_conversion_could_reach_wears_a_dotted_ring() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_ZOOM);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let beside = h.state().graph().get(ids[0]).unwrap().pos + egui::vec2(300.0, 0.0);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[1], beside)],
        })
        .unwrap();
    h.run_steps(2);

    let from = port_at(&h, "checkerboard1.output (varying color output)");
    let over = port_at(&h, "zoom2.zoom (varying number input)");
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.input_mut().events.push(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.input_mut().events.push(egui::Event::PointerMoved(over));
    // Three frames and no release: the first is where egui calls the press a drag, and the
    // ring is drawn from the frame after that.
    h.run_steps(3);

    assert!(
        h.query_by_label("zoom2.zoom (varying number input) (convertible)")
            .is_some(),
        "the port does not say it is convertible"
    );
    assert!(
        h.query_by_label("zoom2.input (varying color input)")
            .is_some(),
        "a port the cable can simply land on says nothing extra"
    );
    h.snapshot("convertible_ring");
}

/// A color dropped on a **pinned** input is offered the `tap`'s rows, exactly as one dropped
/// on a `slew`'s input would be.
///
/// The pin is what makes the gesture possible: while the input drew a circle the cable was
/// refused outright and the menu had nothing to offer, because every casting onto a field
/// publishes a field and the far cable would have been refused in turn. As a number, the
/// port's question is the one the table already answers — how should this picture be read as
/// a number — and the answer is a measurement.
#[test]
fn a_mask_dropped_on_a_pinned_input_offers_the_taps_rows() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    add_node(&mut h, ADD_ADD);
    add_node(&mut h, ADD_SLEW);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (cb, sum, slew) = (ids[0], ids[1], ids[2]);
    // The checkerboard keeps the landing point and the other two move right of it, so the
    // drag crosses open canvas the way a hand's would and every port is in the frame.
    let at = h.state().graph().get(cb).unwrap().pos;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (sum, at + egui::vec2(300.0, 0.0)),
                (slew, at + egui::vec2(300.0, 180.0)),
            ],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(sum, "output"),
            to: PortRef::new(slew, "input"),
        })
        .unwrap();
    h.run_steps(2);

    let from = port_at(&h, "checkerboard1.output (varying color output)");
    let to = port_at(&h, "add2.a (uniform number input)");
    drag_with(&mut h, from, to, egui::Modifiers::NONE);
    // Two frames: the menu's `Area` appears on the first, which is a sizing pass, and the
    // tree is only geometry on the second.
    h.run_steps(2);

    assert_eq!(
        h.state().graph().connections().len(),
        1,
        "the cable itself cannot land: a field is not a uniform number"
    );
    for output in ["mean", "max", "min"] {
        assert!(
            h.query_by_label(&format!("convert with tap.input to {output}"))
                .is_some(),
            "the tap's {output} is not a row of the menu"
        );
    }
    assert!(
        h.query_by_label("convert with luminosity.input to output")
            .is_none(),
        "a casting onto a field is not offered at a uniform number"
    );

    // And taking one lands a `tap` between them, whose uniform number the pinned input
    // accepts.
    h.get_by_label("convert with tap.input to mean").click();
    h.run_steps(2);
    assert_eq!(h.state().graph().len(), 4, "the tap is not there");
    assert_eq!(
        h.state()
            .graph()
            .source_of(PortRef::new(sum, "a"))
            .map(|p| p.key),
        Some("mean"),
        "the measurement feeds the input the color could not"
    );
}

/// The render section carries silvia's supersampling multiplier, and it is the render's
/// alone: what the multiplier changes is how large the frames are drawn before they come
/// back down, so the Output's own resolution — what is on screen, and what the film is — is
/// the same at 1x and at 4x.
#[test]
fn the_render_section_carries_the_supersampling_multiplier() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().next().expect("one node").0;
    // Left of where a new node lands, so the whole section is clear of the Main Mixer panel
    // and the snapshot shows every row of it.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(out, egui::pos2(60.0, 60.0))],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: "offline",
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.run_steps(2);

    assert!(
        h.query_by_label_contains("output1.supersampling 1x (off)")
            .is_some(),
        "the multiplier is on the render section, off to start with"
    );
    assert_eq!(
        h.state().render_settings_of(out).map(|s| s.supersample),
        Some(1),
        "and a render asked for now is the render as it always was"
    );

    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: "supersampling",
            value: "4".to_string(),
        })
        .unwrap();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("output1.supersampling 4x")
            .is_some(),
        "the row says what it is set to"
    );

    assert_eq!(
        h.state().render_settings_of(out).map(|s| s.supersample),
        Some(4),
        "the render draws four times the size"
    );
    assert_eq!(
        supersilvia::nodes::output::resolution_of(h.state().graph().get(out).unwrap()),
        supersilvia::nodes::output::DEFAULT_RESOLUTION,
        "and the Output itself is untouched: the multiplier is the render's alone"
    );
    h.snapshot("output_render_section");
}

/// **An Output's Resolution is the resolution picker.** Closed, the row is the shape, the
/// ratio and the short side; open, a strip of shapes, a row of short sides, the size's cost
/// and a width and a height to type. A short side keeps the shape, a shape keeps the short
/// side, Tall keeps both, and a typed size off the strip is Free — even, as the strip's are.
#[test]
fn an_outputs_resolution_is_picked_by_shape_and_short_side() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().next().expect("one node").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(out, egui::pos2(60.0, 40.0))],
        })
        .unwrap();
    h.run_steps(2);
    let resolution = |h: &Harness<'_, App>| {
        supersilvia::nodes::output::resolution_of(h.state().graph().get(out).unwrap())
    };
    let name = format!("output{out}.resolution");
    h.get_by_label(&format!("{name} 16:9 · 720"));
    h.snapshot("output_resolution_row");

    h.get_by_label(&format!("{name} 16:9 · 720")).click();
    // The popover is an `Area`: its first frame is a sizing pass.
    h.run_steps(2);
    h.get_by_label(&format!("{name} 1080")).click();
    h.run_steps(2);
    assert_eq!(resolution(&h), (1920, 1080), "a short side keeps the shape");
    h.snapshot("output_resolution_picker");

    h.get_by_label(&format!("{name} 4:3")).click();
    h.run_steps(2);
    assert_eq!(resolution(&h), (1440, 1080), "a shape keeps the short side");
    h.get_by_label(&format!("{name} Tall")).click();
    h.run_steps(2);
    assert_eq!(resolution(&h), (1080, 1440), "Tall stands it on end");

    h.get_by_label_contains(&format!("{name} width")).click();
    h.step();
    key(&mut h, egui::Key::End);
    for _ in 0..4 {
        key(&mut h, egui::Key::Backspace);
    }
    type_text(&mut h, "1001");
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(resolution(&h), (1002, 1440), "typed, and even");
    assert!(
        h.query_by_label(&format!("{name} 1002×1440")).is_some(),
        "a size off the strip reads as itself"
    );

    click_at(&mut h, egui::pos2(700.0, 500.0));
    h.run_steps(2);
    assert!(
        h.query_by_label(&format!("{name} 1080")).is_none(),
        "a click away closes it"
    );
}

/// **An Output's Record section**, under a heading of its own between Render and Send, closed
/// on a new Output: its own FPS, then the Record row with a status and one button. Pressed on
/// an Output with nothing cabled into it, the row says why rather than recording, and nothing
/// on the node moves whatever the row says.
#[test]
fn an_outputs_record_row_says_what_it_is_doing() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().next().expect("one node").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(out, egui::pos2(60.0, 60.0))],
        })
        .unwrap();
    h.run_steps(2);
    assert!(
        h.query_by_label("output1.record").is_none(),
        "closed on a new Output"
    );
    let heading = h.get_by_label("output1.recording").rect();
    let render_heading = h.get_by_label("output1.offline").rect();
    let send_heading = h.get_by_label("output1.send").rect();
    assert!(
        render_heading.max.y <= heading.min.y && heading.max.y <= send_heading.min.y,
        "Render, Record, Send"
    );
    let closed = h.state().graph().get(out).unwrap().options.clone();

    h.get_by_label("output1.recording").click();
    h.run_steps(2);
    assert_eq!(
        h.state()
            .graph()
            .get(out)
            .unwrap()
            .options
            .get(supersilvia::nodes::output::RECORD),
        Some(&supersilvia::nodes::ON.to_string()),
        "the triangle writes its option, saved like the other headings'"
    );
    assert_ne!(h.state().graph().get(out).unwrap().options, closed);
    assert!(
        h.query_by_label("output1.record.status off").is_some(),
        "off on a new Output"
    );
    let fps = h.get_by_label_contains("output1.recordFps 30").rect();
    let button = h.get_by_label("output1.record").rect();
    assert!(
        fps.min.y > heading.max.y && button.min.y > fps.max.y,
        "the FPS under the heading, the Record row under it"
    );
    assert!(
        h.query_by_label("output1.render").is_none(),
        "the Render section folds on its own"
    );
    h.snapshot("output_record_section");

    h.get_by_label("output1.record").click();
    h.run_steps(2);
    assert!(!h.state().recording(), "nothing to record");
    assert!(
        h.query_by_label_contains("output1.record.status the Output has nothing connected")
            .is_some(),
        "the row says why"
    );
    assert_eq!(
        h.get_by_label("output1.record").rect(),
        button,
        "the row is the same height whatever it says"
    );

    // The Render section open above it moves it down and leaves the Record button out of it.
    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: "offline",
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.run_steps(2);
    let render = h.get_by_label("output1.render").rect();
    let heading = h.get_by_label("output1.recording").rect();
    let below = h.get_by_label("output1.record").rect();
    assert!(
        render.max.y <= heading.min.y && heading.max.y <= below.min.y,
        "the Record heading is under the Render button, and its row under that"
    );
}

/// **An Output's Send rows.** Under the Send heading, a row per way out with its status and
/// one button, and — once one is on — the Alpha both ways share, a choice between its two
/// values by their names. A click on either is one undo step, and the heading folds the lot.
#[test]
fn an_outputs_send_rows_say_how_it_leaves() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_OUTPUT);
    let out = h.state().graph().iter().next().expect("one node").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(out, egui::pos2(60.0, -170.0))],
        })
        .unwrap();
    h.run_steps(2);
    open_send(&mut h, out);
    assert!(
        h.query_by_label("output1.transparent Opaque").is_none(),
        "no alpha while nothing leaves"
    );

    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: supersilvia::nodes::output::NDI,
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.run_steps(2);
    let before = h.state().undo_len();
    h.get_by_label("output1.transparent Transparent").click();
    h.run_steps(2);
    assert_eq!(
        h.state()
            .graph()
            .get(out)
            .and_then(|n| n.options.get("transparent"))
            .map(String::as_str),
        Some(supersilvia::nodes::ON)
    );
    assert_eq!(h.state().undo_len(), before + 1, "one undo step");
    assert!(
        h.query_by_label_contains("output1.ndi.status").is_some(),
        "the NDI row says what it is doing"
    );
    h.snapshot("output_send_rows");

    // The heading folds every row under it, and the option it turns is saved like any other.
    h.get_by_label("output1.send").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("output1.ndi.status").is_none());
    assert!(
        h.query_by_label("output1.transparent Transparent")
            .is_none()
    );
}

/// Open a select by name and pick one of its choices by what the list shows, as a hand does.
fn pick(h: &mut Harness<'_, App>, select: &str, choice: &str) {
    h.get_by_label_contains(select).click();
    // The sizing pass, as `add_node` documents: the list's first frame is provisional.
    h.run_steps(2);
    h.get_by_label(choice).click();
    h.run_steps(2);
}

/// **A value picked through a select's list lands on the node**, in the option block and in
/// an Output's Render section alike, as one undo step each, and the list closes behind it.
#[test]
fn a_value_picked_from_a_nodes_select_lands_on_the_node() {
    let mut h = wide_harness();
    h.step();
    let slew = add_at(&mut h, "slew", Pos2::new(40.0, 40.0));
    let out = add_at(&mut h, "output", Pos2::new(300.0, 40.0));
    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: "offline",
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.run_steps(2);
    let before = h.state().history().len();
    let option = |h: &Harness<'_, App>, id, key| {
        h.state().graph().get(id).unwrap().options.get(key).cloned()
    };

    pick(&mut h, &format!("slew{slew}.shape"), "Ease");
    assert_eq!(option(&h, slew, "shape").as_deref(), Some("ease"));
    assert!(
        h.query_by_label("Rate Limit").is_none(),
        "the list closed on the pick"
    );

    pick(&mut h, &format!("output{out}.supersampling"), "2x");
    assert_eq!(option(&h, out, "supersampling").as_deref(), Some("2"));
    h.get_by_label_contains(&format!("output{out}.supersampling 2x"));
    assert_eq!(h.state().history().len(), before + 2, "one step a pick");
}

/// A render is modal and says so: a band across the editor with the progress and the one
/// button that cancels, an Output whose button reads *Cancel*, and a document that refuses
/// every command until it is over. In this harness nothing draws, so the render sits at
/// frame zero — which is exactly the state the band is for.
#[test]
fn a_render_puts_a_band_across_the_editor_until_it_is_canceled() {
    use supersilvia::app::render::{Format, RenderSettings};
    use supersilvia::clock::Warmup;
    use supersilvia::graph::PortRef;
    use supersilvia::{Command, CommandError};

    let mut h = harness();
    h.step();
    let (cb, out) = {
        let app = h.state_mut();
        let ws = app.graph().default_workspace();
        let mut add = |slug| {
            app.apply(Command::AddNode {
                slug,
                at: egui::pos2(100.0, 100.0),
                workspace: ws,
            })
            .unwrap();
            app.graph().iter().map(|(id, _)| id).max().unwrap()
        };
        let cb = add("checkerboard");
        let out = add("output");
        app.apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
        (cb, out)
    };
    h.step();
    assert!(
        h.query_by_label("output2.render").is_none(),
        "the render section is under its heading, closed on a new Output"
    );
    h.state_mut()
        .apply(Command::SetOption {
            node: out,
            key: "offline",
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    h.step();
    assert!(
        h.query_by_label("output2.render").is_some(),
        "the Output carries its Render button"
    );
    assert!(
        h.query_by_label_contains("output2.fps 30").is_some(),
        "and its numbers"
    );

    let destination =
        std::env::temp_dir().join(format!("supersilvia-ui-render-{}", std::process::id()));
    h.state_mut()
        .start_render(
            out,
            &RenderSettings {
                fps: 10.0,
                frames: 20,
                warmup: Warmup::Black,
                supersample: 1,
                format: Format::PngSequence,
                destination: destination.clone(),
            },
        )
        .unwrap();
    h.step();
    assert!(h.state().rendering());
    assert!(
        h.query_by_label("Cancel render").is_some(),
        "the band carries the one button"
    );
    assert_eq!(
        h.state_mut().apply(Command::RemoveNodes(vec![cb])),
        Err(CommandError::Rendering),
        "the document is closed"
    );

    h.get_by_label("Cancel render").click();
    h.step();
    h.step();
    assert!(!h.state().rendering(), "canceled");
    assert!(
        h.query_by_label("Cancel render").is_none(),
        "and the band is gone"
    );
    assert!(h.state_mut().apply(Command::RemoveNodes(vec![cb])).is_ok());
    let _ = std::fs::remove_dir_all(destination);
}

/// The Main Input's file is picked the way a node's file button picks one: the project's own
/// clips as cards, with the dialog under them, rather than a dialog every time.
#[test]
fn the_main_inputs_video_file_offers_the_projects_own_clips() {
    use supersilvia::maininput::VideoSource;

    let root = std::env::temp_dir().join(format!(
        "supersilvia-main-input-picker-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);

    let mut h = harness();
    h.step();
    h.state_mut().new_project(root.clone());
    let assets = root.join("assets");
    std::fs::create_dir_all(&assets).unwrap();
    for name in ["corridor.webm", "notes.txt"] {
        std::fs::write(assets.join(name), b"not really media").unwrap();
    }
    h.state_mut().open_project(root.clone());
    h.step();

    // Unfold the panel. The spine answers a pointer, not the accessibility click `click()`
    // sends, so the press goes in as events, as `double_click` does.
    let at = h.get_by_label("Show Main Input").rect().center();
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.step();
    h.step();
    // Then choose *Video file* in its first select.
    h.get_all_by_role(egui::accesskit::Role::ComboBox)
        .next()
        .expect("the video source select")
        .click();
    // Two frames: the first time a popup appears egui runs a sizing pass.
    h.step();
    h.step();
    h.get_by_label("Video file").click();
    h.step();
    h.step();

    assert!(
        h.query_by_label("asset corridor.webm").is_some(),
        "the project's clip is offered as a card"
    );
    assert!(
        h.query_by_label("asset notes.txt").is_none(),
        "and only clips"
    );
    assert!(
        h.query_by_label("Import a file…").is_some(),
        "with the dialog under it"
    );

    h.get_by_label("asset corridor.webm").click();
    h.step();
    h.step();
    match &h.state().project().main_input().video {
        VideoSource::File { asset } => assert!(asset.ends_with("corridor.webm"), "{asset}"),
        other => panic!("the card set the source: {other:?}"),
    }
    assert!(
        h.query_by_label("asset corridor.webm").is_none(),
        "and the picker closed with the choice"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The `!` on an Output whose render would read a source that can only answer now, and the
/// popup under it that takes you to that source — here the Main Input panel, pointed at the
/// test pattern, so the panel unfolds.
#[test]
fn an_outputs_bang_names_the_live_source_and_goes_there() {
    use supersilvia::Command;
    use supersilvia::graph::PortRef;
    use supersilvia::maininput::VideoSource;
    use supersilvia::ui::maininput::MainInputAction;

    let mut h = harness();
    h.step();
    {
        let app = h.state_mut();
        let ws = app.graph().default_workspace();
        let mut add = |slug| {
            app.apply(Command::AddNode {
                slug,
                at: egui::pos2(100.0, 100.0),
                workspace: ws,
            })
            .unwrap();
            app.graph().iter().map(|(id, _)| id).max().unwrap()
        };
        let input = add("maininput");
        let out = add("output");
        app.apply(Command::Connect {
            from: PortRef::new(input, "frame"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
        app.apply(Command::SetOption {
            node: out,
            key: "offline",
            value: supersilvia::nodes::ON.to_string(),
        })
        .unwrap();
    }
    h.step();
    assert!(
        h.query_by_label("output2.live").is_none(),
        "the panel points at nothing, so there is nothing to warn about"
    );

    h.state_mut()
        .handle_main_input(MainInputAction::SetVideo(VideoSource::Camera {
            device: "test".to_string(),
        }));
    h.step();
    h.step();
    h.get_by_label("output2.live").click();
    h.step();
    h.step();
    assert!(
        h.query_by_label_contains("Main Input panel").is_some(),
        "the popup names the panel's source"
    );
    assert!(
        h.query_by_label("Show Main Input").is_some(),
        "the panel is folded"
    );
    h.get_by_label_contains("Main Input panel").click();
    h.step();
    h.step();
    assert!(
        h.query_by_label("Show Main Input").is_none(),
        "going there unfolds the panel"
    );
}

/// A Mac's project whose Main Input is a Syphon server, opened where there is no Syphon: the
/// select says so, the list does not offer Syphon, and the choice is kept for the Mac. On this
/// machine's own answers, so a Mac holds the other half: Syphon offered, and not called
/// macOS only.
#[test]
fn a_macs_syphon_source_reads_as_macos_only_where_there_is_none() {
    use supersilvia::maininput::VideoSource;
    use supersilvia::ui::maininput::MainInputAction;
    let mut h = Harness::builder()
        .renderer(renderer())
        .build_eframe(machine_app);
    h.step();
    let at = h.get_by_label("Show Main Input").rect().center();
    let events = &mut h.input_mut().events;
    events.push(egui::Event::PointerMoved(at));
    for pressed in [true, false] {
        events.push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.step();
    h.step();
    let chosen = VideoSource::Syphon {
        server: "Arena – Composition".to_string(),
        flip: false,
        transparent: false,
    };
    h.state_mut()
        .handle_main_input(MainInputAction::SetVideo(chosen.clone()));
    h.run_steps(3);
    let here = supersilvia::platform::syphon::available();
    assert_eq!(
        h.query_by_value("Syphon (macOS only)").is_some(),
        !here,
        "the select says where Syphon is"
    );

    h.get_all_by_role(egui::accesskit::Role::ComboBox)
        .next()
        .expect("the video source select")
        .click();
    h.step();
    h.step();
    assert!(h.query_by_label("NDI®").is_some(), "the list is up");
    assert_eq!(
        h.query_by_label("Syphon").is_some(),
        here,
        "and offers Syphon only where there is one"
    );
    assert_eq!(
        h.state().project().main_input().video,
        chosen,
        "the choice is kept"
    );
}

/// Put the pointer somewhere and let the frame settle, without pressing anything.
fn hover_at(h: &mut Harness<'_, App>, at: Pos2) {
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.step();
    h.step();
}

/// A folded panel's edge is not a resize handle.
///
/// `exact_size` pins the width a panel may take but leaves it **resizable**, which is
/// egui's default — so the spine's edge lit under the pointer and offered a resize cursor
/// for a drag that could move nothing.
#[test]
fn a_folded_panels_edge_is_not_a_resize_handle() {
    let mut h = harness();
    h.step();
    // The Main Input starts folded. The panel's outer edge is `SPINE` from the window's, not
    // the spine widget's own right-hand side — that sits inside the frame's margin, where the
    // cursor was never going to change whatever the panel said.
    let spine = h.get_by_label("Show Main Input").rect();
    hover_at(
        &mut h,
        Pos2::new(supersilvia::ui::panel::SPINE, spine.center().y),
    );
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::Default,
        "a folded panel's edge is not a handle"
    );

    // Unfolded it is one, which is what says the point probed above is on the edge rather
    // than somewhere the cursor was never going to change.
    click_at(&mut h, spine.center());
    // egui slides a panel open, and forces `resizable(false)` for the whole animation so the
    // handle cannot move under the pointer mid-slide. So let it land before asking.
    h.run_steps(30);
    let bar = h.get_by_label("Hide Main Input").rect();
    hover_at(&mut h, Pos2::new(bar.max.x + 8.0, bar.center().y));
    assert_eq!(
        h.output().platform_output.cursor_icon,
        egui::CursorIcon::ResizeHorizontal,
        "an open panel's edge still is"
    );
}

/// The cursor the last frame asked for.
fn cursor(h: &Harness<'_, App>) -> egui::CursorIcon {
    h.output().platform_output.cursor_icon
}

/// **A button wears the pointing hand**, hand-drawn or egui's own: the Nodes button, a node's
/// `⊗` and an entry on the menu bar.
#[test]
fn a_button_wears_the_pointing_hand() {
    let mut h = harness();
    h.step();
    let nodes = h.get_by_label("Nodes").rect().center();
    hover_at(&mut h, nodes);
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::PointingHand,
        "the Nodes button"
    );

    let id = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let close = h.get_by_label(&format!("close checkerboard{id}")).rect();
    hover_at(&mut h, close.center());
    assert_eq!(cursor(&h), egui::CursorIcon::PointingHand, "the header's ⊗");

    let view = h.get_by_label("View").rect().center();
    hover_at(&mut h, view);
    assert_eq!(cursor(&h), egui::CursorIcon::PointingHand, "a menu entry");
}

/// **The header's `?` says help**, where the header beside it says the node can be carried.
#[test]
fn the_help_mark_says_help() {
    let mut h = harness();
    let id = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let help = h.get_by_label(&format!("help checkerboard{id}")).rect();
    hover_at(&mut h, help.center());
    assert_eq!(cursor(&h), egui::CursorIcon::Help);
}

/// **A scrub keeps its cursor for the whole drag**, off the control and over the canvas; a
/// cap is a button; and with the pointer locked there is none, so a hidden pointer stays
/// hidden.
#[test]
fn a_scrub_keeps_its_cursor_wherever_the_pointer_goes() {
    let mut h = harness();
    let id = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let rect = h
        .get_by_label_contains(&format!("checkerboard{id}.frequency 8"))
        .rect();
    hover_at(&mut h, rect.center());
    assert_eq!(cursor(&h), egui::CursorIcon::ResizeHorizontal, "the track");
    hover_at(&mut h, Pos2::new(rect.min.x + 3.0, rect.center().y));
    assert_eq!(cursor(&h), egui::CursorIcon::PointingHand, "the − cap");

    press_at(&mut h, rect.center());
    move_to(&mut h, rect.center() + egui::vec2(20.0, 0.0));
    move_to(&mut h, rect.center() + egui::vec2(60.0, 200.0));
    assert_ne!(float(&h, id, "frequency"), 8.0, "the drag scrubbed nothing");
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::ResizeHorizontal,
        "held off the control"
    );
    release_at(&mut h, rect.center() + egui::vec2(60.0, 200.0));
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::Default,
        "let go over the canvas"
    );

    let mut h = harness_with(supersilvia::preferences::Preferences {
        lock_cursor_while_scrubbing: true,
        ..supersilvia::preferences::Preferences::default()
    });
    let id = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let rect = h
        .get_by_label_contains(&format!("checkerboard{id}.frequency 8"))
        .rect();
    press_at(&mut h, rect.center());
    move_to(&mut h, rect.center() + egui::vec2(20.0, 0.0));
    assert_eq!(cursor(&h), egui::CursorIcon::None, "locked and hidden");
    release_at(&mut h, rect.center() + egui::vec2(20.0, 0.0));
}

/// **A node is an open hand, and carried a closed one** for the whole drag, over the empty
/// canvas as over the node.
#[test]
fn carrying_a_node_closes_the_hand() {
    let mut h = harness();
    let id = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let header = h.get_by_label(&format!("checkerboard{id}")).rect();
    let at = header.left_center() + egui::vec2(20.0, 0.0);
    hover_at(&mut h, at);
    assert_eq!(cursor(&h), egui::CursorIcon::Grab, "the header");

    press_at(&mut h, at);
    for step in 1..=4 {
        move_to(&mut h, at + egui::vec2(60.0, 40.0) * (step as f32 / 4.0));
    }
    assert_eq!(cursor(&h), egui::CursorIcon::Grabbing, "carried");
    release_at(&mut h, at + egui::vec2(60.0, 40.0));
    assert_ne!(
        h.state().graph().get(id).unwrap().pos,
        Pos2::new(40.0, 200.0),
        "the node moved"
    );
}

/// **A cable in flight is a closed hand, and not allowed over a port it cannot land on** —
/// its own node's input, which would make a loop — while a port it can land on and the empty
/// canvas keep the hand. A port at rest is an open hand.
#[test]
fn a_cable_over_a_port_it_cannot_land_on_is_not_allowed() {
    let mut h = harness();
    let (cb, out) = a_source_and_an_output(&mut h);
    let from = source_output(&h, cb);
    let own = h
        .get_by_label(&format!("checkerboard{cb}.color1 (varying color input)"))
        .rect()
        .center();
    let legal = output_input(&h, out);
    hover_at(&mut h, from);
    assert_eq!(cursor(&h), egui::CursorIcon::Grab, "a port at rest");

    press_at(&mut h, from);
    let open = from + egui::vec2(60.0, -60.0);
    for step in 1..=4 {
        move_to(&mut h, from + (open - from) * (step as f32 / 4.0));
    }
    assert_eq!(cursor(&h), egui::CursorIcon::Grabbing, "over the canvas");
    move_to(&mut h, own);
    move_to(&mut h, own);
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::NotAllowed,
        "over its own input"
    );
    move_to(&mut h, legal);
    move_to(&mut h, legal);
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::Grabbing,
        "over a port it lands on"
    );
    release_at(&mut h, legal);
    assert_eq!(h.state().graph().connections().len(), 1);
}

/// **A control a cable answers for is not allowed**: the swatch a `color` node feeds.
#[test]
fn a_cabled_control_is_not_allowed() {
    let mut h = harness();
    let cb = add_at(&mut h, "checkerboard", Pos2::new(40.0, 200.0));
    let swatch = h
        .get_by_label_contains(&format!("checkerboard{cb}.color1 #"))
        .rect()
        .center();
    hover_at(&mut h, swatch);
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::PointingHand,
        "a swatch to open"
    );

    let color = add_at(&mut h, "color", Pos2::new(40.0, 600.0));
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(color, "output"),
            to: PortRef::new(cb, "color1"),
        })
        .expect("a uniform color into a varying color");
    h.run_steps(2);
    hover_at(&mut h, swatch);
    assert_eq!(
        cursor(&h),
        egui::CursorIcon::NotAllowed,
        "a swatch a cable answers"
    );
}

/// How wide a side panel is drawn, by its id: `preview` is the Main Mixer's.
fn panel_width(h: &Harness<'_, App>, id: &str) -> f32 {
    egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new(id))
        .expect("the panel has been drawn")
        .size()
        .x
}

/// **A side panel opens at the width it was left at**, which `preferences.json` keeps.
#[test]
fn a_side_panel_opens_at_the_width_it_was_left_at() {
    let mut h = harness_with(supersilvia::preferences::Preferences {
        mixer_width: Some(450.0),
        ..supersilvia::preferences::Preferences::default()
    });
    h.run_steps(3);
    assert!(
        (panel_width(&h, "preview") - 450.0).abs() < 1.0,
        "{}",
        panel_width(&h, "preview")
    );
}

/// **Dragging a side panel's edge keeps the width** in the preferences, on the release.
#[test]
fn dragging_a_side_panels_edge_keeps_the_width() {
    let mut h = wide_harness();
    h.run_steps(3);
    assert_eq!(h.state().preferences().mixer_width, None);
    let before = panel_width(&h, "preview");
    let bar = h.get_by_label("Hide Main Mixer").rect();
    let edge = Pos2::new(bar.min.x - 8.0, bar.center().y);
    drag_with(
        &mut h,
        edge,
        edge - egui::vec2(60.0, 0.0),
        egui::Modifiers::NONE,
    );
    h.run_steps(2);
    let after = panel_width(&h, "preview");
    assert!(after > before + 40.0, "{before} to {after}");
    assert_eq!(h.state().preferences().mixer_width, Some(after));
}

/// The whole of an open panel's header folds it, and not only the arrow at its outer end.
///
/// Clicked at the title, which is the far end of the bar from the arrow: a click that lands
/// there is a click on the bar itself and could be nothing else.
#[test]
fn a_panel_header_folds_from_anywhere_along_it() {
    let mut h = harness();
    h.step();
    assert!(
        h.query_by_label("Show Main Mixer").is_none(),
        "the Mixer starts open"
    );
    let bar = h.get_by_label("Hide Main Mixer").rect();
    click_at(&mut h, egui::pos2(bar.left() + 8.0, bar.center().y));
    assert!(
        h.query_by_label("Show Main Mixer").is_some(),
        "the bar folded the panel"
    );
}

/// A node dragged out over a side panel comes back with the cursor.
///
/// The canvas culls a node whose rect has left it, which is what keeps chrome proportional to
/// what is visible. A node **in hand** cannot be culled by that rule: taking its widget out of
/// the tree takes the drag with it, and the hand comes back over the canvas holding nothing
/// while the node sits where it was abandoned.
#[test]
fn a_node_dragged_out_over_a_panel_comes_back_with_the_cursor() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).max().unwrap();

    // Where the node sits under the cursor when it is taken, in screen points. That is the
    // whole invariant: whatever the pointer does, the node keeps this offset from it.
    let grip = header_at(&h, cb);
    let held = |h: &Harness<'_, App>, at: Pos2| -> egui::Vec2 {
        let t = h.state().canvas_transform();
        let origin = h.state().canvas_origin();
        t.to_screen(origin, node_rect(h, cb).min) - at
    };
    press_at(&mut h, grip);
    let offset = held(&h, grip);

    // Out past the left edge of the canvas and well into the panel, far enough that the
    // node's whole rect has left the canvas — which is the condition the cull tests.
    let canvas = canvas_rect(&h);
    let away = Pos2::new(canvas.min.x - 260.0, grip.y);
    move_to(&mut h, away);
    h.run_steps(2);
    assert_eq!(
        h.state().canvas_drawn(),
        0,
        "the node in hand was drawn anyway, so the cull is not what is under test here"
    );

    // And back onto the canvas, somewhere else.
    let back = Pos2::new(canvas.min.x + 420.0, grip.y + 60.0);
    move_to(&mut h, back);
    h.run_steps(2);
    let returned = held(&h, back);
    assert!(
        (returned - offset).length() < 1.0,
        "the node let go of the cursor while it was over the panel: {returned:?} against {offset:?}"
    );
    release_at(&mut h, back);
}

/// **A node let go of where you cannot see it is fetched back into view.**
///
/// The cull has no exemption for the thing in hand, so a drag can carry a node out over a
/// side panel and the drop can put it down with nothing on screen to show for it. The view
/// goes after it — smoothly, and only far enough to clear the edge, because the hand knows
/// where it put the thing and a swing to centre it throws away the rest of the graph.
#[test]
fn a_node_dropped_out_of_sight_is_glided_back_into_view() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_CHECKERBOARD);
    let cb = h.state().graph().iter().map(|(id, _)| id).max().unwrap();

    let grip = header_at(&h, cb);
    press_at(&mut h, grip);
    let canvas = canvas_rect(&h);
    let away = Pos2::new(canvas.min.x - 260.0, grip.y);
    move_to(&mut h, away);
    h.run_steps(2);
    assert_eq!(
        h.state().canvas_drawn(),
        0,
        "the node is not out of sight, so there is nothing here to fetch back"
    );

    // Let go out there, over the panel.
    release_at(&mut h, away);
    let start = h.state().canvas_transform().pan.x;
    h.run_steps(2);
    let first = h.state().canvas_transform().pan.x;
    h.run_steps(60);
    let settled = h.state().canvas_transform().pan.x;

    assert_eq!(
        h.state().canvas_drawn(),
        1,
        "the node is still out of sight"
    );
    let shown = h
        .state()
        .canvas_transform()
        .to_screen(h.state().canvas_origin(), node_rect(&h, cb).min);
    assert!(
        shown.x >= canvas.min.x,
        "the node is on the canvas but off its near edge: {shown:?} against {canvas:?}"
    );

    // A glide and not a jump: the first frames cover a fraction of it, not all of it.
    let whole = (settled - start).abs();
    assert!(whole > 1.0, "the view never went after it: {whole}");
    assert!(
        (first - start).abs() < whole * 0.9,
        "the view cut to the node instead of gliding: {} of {whole} in two frames",
        (first - start).abs()
    );
}

/// Letting go of a node **eases** the strip back rather than dropping it.
///
/// Grow-never-shrink holds the strip still under the hand, which is what stops the view
/// being yanked mid-drag. Handing all that room back on the frame the button comes up would
/// be the same yank moved to the end of the gesture: the pan is clamped to the strip, so a
/// strip that shortens in one frame takes the camera with it in one frame. Nothing does the
/// handing back — the growth rule simply stops applying and the ease carries it home.
#[test]
fn letting_go_of_a_node_eases_the_strip_back_rather_than_dropping_it() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    let end = far_end(&mut h);

    // Take the outermost node and carry it a long way back along the strip, which is what
    // shortens the strip and so moves the view when the strip is allowed to follow.
    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    for step in 1..=12 {
        move_to(&mut h, from - egui::vec2(step as f32 * 40.0, 0.0));
    }
    // Where the view sits with the hand still closed. The creep may have moved it on the way
    // — that is the edge scroll doing its job — so this is read rather than asserted; what is
    // under test is what happens when the hand opens.
    let _ = end;
    let held = h.state().canvas_transform().pan.x;

    // Let go. The room comes back over frames, so the view comes back over frames with it.
    release_at(&mut h, from - egui::vec2(480.0, 0.0));
    let first = h.state().canvas_transform().pan.x;
    h.run_steps(60);
    let settled = h.state().canvas_transform().pan.x;

    assert!(
        (settled - held).abs() > 1.0,
        "the strip did settle in the end: {settled} against {held}"
    );
    assert!(
        (first - held).abs() < (settled - held).abs() * 0.9,
        "the camera snapped home instead of gliding: {} of its {} on the first frames",
        (first - held).abs(),
        (settled - held).abs()
    );
}

/// **Deleting the node at the far end of the strip glides the view home rather than jumping
/// it.** The strip is one node shorter the instant the delete lands, and the pan is clamped
/// to the strip — so a view sitting at the far end is yanked by however much room went away.
///
/// The same lag the freeze gives a drag, given to every other way a strip changes length.
#[test]
fn deleting_a_node_at_the_far_end_glides_the_view_rather_than_jumping_it() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    let out = far_end(&mut h);

    h.state_mut()
        .apply(Command::RemoveNodes(vec![ids[1]]))
        .unwrap();
    h.step();
    let first = h.state().canvas_transform().pan.x;
    h.run_steps(60);
    let home = h.state().canvas_transform().pan.x;

    assert!(
        (home - out).abs() > 1.0,
        "the delete should have moved the view at all: {home} against {out}"
    );
    assert!(
        (first - out).abs() < (home - out).abs() * 0.9,
        "the camera jumped instead of gliding: it went {} of its {} on the first frame",
        (first - out).abs(),
        (home - out).abs()
    );
}

/// And the same for a node moved by anything that is not a drag. An undo puts the strip back
/// to whatever length it was, in one frame, with no gesture anywhere near it.
#[test]
fn undoing_a_move_that_lengthened_the_strip_glides_the_view_rather_than_jumping_it() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);

    // Carried a long way out, which is what makes the strip long enough to have somewhere to
    // come back from. Through the bus, so the undo ring has the step.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(ids[1], Pos2::new(4000.0, 60.0))],
        })
        .unwrap();
    h.run_steps(60);
    let out = far_end(&mut h);

    assert!(h.state_mut().undo());
    h.step();
    let first = h.state().canvas_transform().pan.x;
    h.run_steps(60);
    let home = h.state().canvas_transform().pan.x;

    assert!(
        (home - out).abs() > 1.0,
        "the undo should have moved the view at all: {home} against {out}"
    );
    assert!(
        (first - out).abs() < (home - out).abs() * 0.9,
        "the camera jumped instead of gliding: it went {} of its {} on the first frame",
        (first - out).abs(),
        (home - out).abs()
    );
}

/// The drag, whole: a node held against the near edge creeps the view, keeps its own place
/// under the cursor while the world moves beneath it, and cannot shorten the strip it is
/// being carried along — all of it driven from the pointer rather than from the node's own
/// widget, which is why the node being culled below changes none of it.
#[test]
fn a_drag_creeps_the_view_keeps_its_grip_and_cannot_shrink_the_strip() {
    let mut h = harness();
    let ids = a_long_strip(&mut h);
    far_end(&mut h);

    // The map's scale is the strip's length: a strip that shrank under the hand shows up
    // here as a node on the map changing width.
    let name = format!("rail checkerboard{}", ids[0]);
    let mapped = |h: &Harness<'_, App>| h.get_by_label(&name).rect().width();
    let before = mapped(&h);

    // The node's screen position against the pointer's, which is the grip it was taken with.
    let grip = |h: &Harness<'_, App>, at: Pos2| -> egui::Vec2 {
        let t = h.state().canvas_transform();
        let origin = h.state().canvas_origin();
        t.to_screen(origin, node_rect(h, ids[1]).min) - at
    };

    let from = header_at(&h, ids[1]);
    press_at(&mut h, from);
    // Into the margin at the near edge and held there, which is the creep: the hand stays
    // still and the view comes to it, carrying the node along at the same place on screen.
    let at = Pos2::new(canvas_rect(&h).min.x + 8.0, from.y);
    move_to(&mut h, at);
    h.step();
    let held = grip(&h, at);
    let mut pan = h.state().canvas_transform().pan.x;

    for _ in 0..6 {
        h.step();
        let now = h.state().canvas_transform().pan.x;
        assert!(now > pan, "the view did not creep: {now} is not past {pan}");
        pan = now;
        assert!(
            (grip(&h, at) - held).length() < 0.001,
            "the node slid out from under the pointer: {:?} is not {held:?}",
            grip(&h, at)
        );
        assert_eq!(
            mapped(&h),
            before,
            "the strip shrank under the hand that was dragging along it"
        );
    }
    release_at(&mut h, at);
}

// ----------------------------------------------------------------- the viewfinder

/// A camcorder with its viewfinder on screen, and the region's own name.
fn camcorder(h: &mut Harness<'_, App>) -> (supersilvia::graph::NodeId, String) {
    add_node(h, ("Effect", "add camcordercrt"));
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    // Sixteen rows and a square viewfinder is a tall node: put its head at the top of the
    // canvas so the band at its foot is on screen, which is where the pointer has to reach
    // it. A widget egui has clipped away is not hovered however right its rect looks.
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .unwrap();
    h.step();
    (id, format!("camcordercrt{id}.region"))
}

/// The gesture the node exists for: a drag in the viewfinder aims the loop, and the node it
/// is drawn on does not move — which is what `claims_pointer` buys.
#[test]
fn dragging_the_viewfinder_drifts_the_loop_and_leaves_the_node_where_it_is() {
    let mut h = tall_harness();
    h.step();
    let (id, region) = camcorder(&mut h);
    let where_it_was = h.state().graph().get(id).unwrap().pos;
    let steps = h.state().undo_len();
    assert_eq!(float(&h, id, "fbDriftX"), 0.0, "aimed straight ahead");

    let from = h.get_by_label(&region).rect().center();
    drag_with(
        &mut h,
        from,
        from + egui::vec2(40.0, 0.0),
        egui::Modifiers::NONE,
    );
    h.snapshot("viewfinder");

    assert!(
        float(&h, id, "fbDriftX") > 0.0,
        "the drag did not drift the camera: {}",
        float(&h, id, "fbDriftX"),
    );
    assert_eq!(float(&h, id, "fbTiltX"), 0.0, "a plain drag does not tilt");
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        where_it_was,
        "the node came along with the drag",
    );
    assert_eq!(
        h.state().undo_len(),
        steps + 1,
        "a gesture is one undo step"
    );
}

/// Shift is the other axis of the same drag: the camera angles rather than slides.
#[test]
fn shift_dragging_the_viewfinder_tilts_the_camera() {
    let mut h = tall_harness();
    h.step();
    let (id, region) = camcorder(&mut h);

    let from = h.get_by_label(&region).rect().center();
    drag_with(
        &mut h,
        from,
        from + egui::vec2(0.0, -30.0),
        egui::Modifiers::SHIFT,
    );

    assert!(
        float(&h, id, "fbTiltY") > 0.0,
        "shift-dragging up did not tilt the camera: {}",
        float(&h, id, "fbTiltY"),
    );
    assert_eq!(float(&h, id, "fbDriftY"), 0.0, "and it did not drift");
}

/// The recorded curve is drawn in a band of the node's own, off the value the document
/// holds — so a patch just opened draws its performance before anything has ticked.
///
/// And the value takes no row: a curve is a picture, and a row of nothing above the picture
/// is what asking for one would draw.
#[test]
fn an_automations_curve_is_drawn_from_the_value_the_node_holds() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ("Control", "add automation"));
    let id = h.state().graph().iter().next().expect("one node").0;
    h.run_steps(2);

    let node = h.state().graph().get(id).unwrap();
    assert_eq!(node.def.values.len(), 1, "the curve is the one value");
    assert!(
        supersilvia::ui::canvas::value_height(node, 0) == 0.0,
        "the curve is a region, so its value asks for no row"
    );
    h.get_by_label_contains("automation1.curve 0 points");

    let points = vec![
        supersilvia::graph::Point {
            time: 0.0,
            value: 0.0,
        },
        supersilvia::graph::Point {
            time: 0.5,
            value: 1.0,
        },
        supersilvia::graph::Point {
            time: 1.0,
            value: 0.25,
        },
    ];
    h.state_mut()
        .apply(Command::SetValue {
            node: id,
            key: "recording",
            value: supersilvia::graph::Value::Points(points),
        })
        .expect("the node declares it");
    h.run_steps(2);
    h.get_by_label_contains("automation1.curve 3 points");
    h.snapshot("automation_curve");
}

// -------------------------------------------------- the node's own values, drawn by the node

/// The palette is a picture of itself and a grid of twelve, not fifteen rows with nothing
/// drawn.
///
/// silvia's `cosinegradient` fills its custom area with a gradient strip, the three channel
/// curves and a `28px 1fr 1fr 1fr` grid of `<s-number>`s under R, G and B. This is that: the
/// twelve coefficients are the node's own values — hidden controls, no port, no cable — drawn
/// as the same inset s-number a port row carries, and what is left on a row is what a patch
/// drives.
#[test]
fn the_cosine_palette_draws_itself_over_a_grid_of_its_own_twelve() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_COSINEGRADIENT);
    let id = h.state().graph().iter().next().expect("the palette").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);

    let node = only_node(&h);
    for key in [
        "biasR", "biasG", "biasB", "ampR", "ampG", "ampB", "freqR", "freqG", "freqB", "phaseR",
        "phaseG", "phaseB",
    ] {
        assert!(
            node.controls.contains_key(key),
            "{key} is not one of the node's own values"
        );
        assert!(
            node.inputs.iter().all(|p| p.key != key),
            "{key} still has a port for a cable to land on"
        );
        // The cell carries the name the row's control had, so what a hand or an agent asks
        // for is the coefficient rather than the affordance it happens to wear.
        h.get_by_label_contains(&format!("cosinegradient{id}.{key}"));
    }
    // What is left takes a cable: what goes through the palette, its Time and its Offset.
    assert_eq!(
        node.inputs.iter().map(|p| p.key).collect::<Vec<_>>(),
        ["t", "clock", "speed", "phaseOffset"],
        "the rows that are left are the ones a patch drives"
    );
    assert_eq!(
        supersilvia::ui::canvas::node_width(&node),
        supersilvia::widgets::palette::WIDTH,
        "the body is as wide as silvia's grid needs"
    );

    h.snapshot("cosine_gradient_palette");
}

/// A coefficient is edited exactly as a knob on a row is: one command through the bus, one
/// step back.
#[test]
fn a_coefficient_in_the_grid_is_one_undo_step() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_COSINEGRADIENT);
    let id = h.state().graph().iter().next().expect("the palette").0;
    h.run_steps(2);
    let steps = h.state().undo_len();

    let cell = h
        .get_by_label_contains(&format!("cosinegradient{id}.phaseG"))
        .rect()
        .center();
    drag_with(
        &mut h,
        cell,
        cell + egui::vec2(20.0, 0.0),
        egui::Modifiers::NONE,
    );

    assert!(
        float(&h, id, "phaseG") > 0.33,
        "the drag did not move the coefficient: {}",
        float(&h, id, "phaseG")
    );
    assert_eq!(
        h.state().undo_len(),
        steps + 1,
        "a gesture is one undo step"
    );
}

/// The sequencer draws the rhythm it is playing, and its twelve lane numbers are its own.
///
/// silvia's four lanes of lit cells with the playhead walking across them is the one thing a
/// hand reads while it is playing, and this node had none of it. The grid is read-only: what
/// shapes a lane is the three numbers on the slab under it, which are hidden controls and not
/// ports.
#[test]
fn a_euclidean_rhythm_draws_its_four_lanes_over_its_own_numbers() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_EUCLIDEANRHYTHM);
    let id = h.state().graph().iter().next().expect("the sequencer").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);

    let node = only_node(&h);
    for lane in 1..=4 {
        for part in ["steps", "pulses", "rotation"] {
            let key = format!("lane{lane}{part}");
            assert!(
                node.controls.contains_key(key.as_str()),
                "{key} is not one of the node's own values"
            );
            assert!(
                node.inputs.iter().all(|p| p.key != key),
                "{key} still has a port for a cable to land on"
            );
            h.get_by_label_contains(&format!("euclideanrhythm{id}.{key}"));
        }
    }
    assert_eq!(
        node.inputs.iter().map(|p| p.key).collect::<Vec<_>>(),
        ["clock", "speed", "phaseOffset", "step", "gateLength"],
        "the rows that are left are the ones a patch drives"
    );

    // A lane can be filled the whole way now: Pulses follows that lane's own Steps rather
    // than stopping at silvia's sixteen.
    h.state_mut()
        .apply(Command::SetControl {
            node: id,
            key: "lane1steps",
            value: supersilvia::graph::ControlValue::Float(32.0),
        })
        .expect("a control that is there");
    h.run_steps(2);
    assert_eq!(
        h.state()
            .graph()
            .get(id)
            .and_then(|n| supersilvia::nodes::control_range(
                supersilvia::nodes::find("euclideanrhythm").expect("in the registry"),
                n,
                "lane1pulses",
            ))
            .map(|r| r.max),
        Some(32.0),
        "Pulses stops where the lane does"
    );

    h.snapshot("euclidean_rhythm_grid");

    // silvia's third body button: four lanes to nothing, in one edit.
    h.state_mut()
        .apply(Command::SetControl {
            node: id,
            key: "lane1pulses",
            value: supersilvia::graph::ControlValue::Float(7.0),
        })
        .expect("a control that is there");
    h.run_steps(2);
    let steps = h.state().undo_len();
    h.get_by_label_contains(&format!("euclideanrhythm{id}.clear"))
        .click();
    h.run_steps(2);
    for lane in 1..=4 {
        assert_eq!(
            float(&h, id, &format!("lane{lane}pulses")),
            0.0,
            "lane {lane} still has pulses in it"
        );
    }
    assert_eq!(h.state().undo_len(), steps + 1, "Clear is one step back");
}

/// The Step Sequencer is its grid: a click lights a cell, a second click puts it out, each
/// click is one step back, and Clear empties all four lanes in one more.
///
/// silvia's `stepsequencer.js` is four `.seq-lane`s of sixteen `.seq-step`s over Start, Reset
/// and Clear. Here the cells are the ones Euclidean Rhythm's figure is drawn in, the pattern
/// is the node's own value; Time, Offset and Step are the rows every sequencer has, a gear in
/// Time being its play and its reset.
#[test]
fn a_step_sequencers_cells_light_one_click_at_a_time() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_STEPSEQUENCER);
    let id = h.state().graph().iter().next().expect("the sequencer").0;
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .expect("a node that is there");
    h.run_steps(2);

    let node = only_node(&h);
    assert_eq!(
        node.inputs.iter().map(|p| p.key).collect::<Vec<_>>(),
        ["clock", "speed", "phaseOffset", "step", "gateLength"],
        "the time rows and Step Euclidean Rhythm has"
    );
    assert_eq!(
        supersilvia::ui::canvas::node_width(&node),
        supersilvia::widgets::steps::GRID_WIDTH,
        "the body is as wide as silvia's sixteen cells need"
    );
    let lit = |h: &Harness<'_, App>, lane: usize, step: usize| {
        h.state()
            .graph()
            .get(id)
            .and_then(|n| n.values.get("pattern"))
            .is_some_and(|p| p.lit(lane, step))
    };
    assert!(!lit(&h, 0, 0), "a new sequencer's grid is empty");
    let height = node_height(&h, id);

    let steps = h.state().undo_len();
    for (lane, step) in [
        (1, 1),
        (1, 5),
        (1, 9),
        (1, 13),
        (2, 5),
        (2, 13),
        (3, 3),
        (4, 16),
    ] {
        h.get_by_label(&format!("stepsequencer{id}.lane{lane}.step{step}"))
            .click();
        h.run_steps(2);
        assert!(
            lit(&h, lane - 1, step - 1),
            "lane {lane} step {step} did not light"
        );
    }
    assert_eq!(
        h.state().undo_len(),
        steps + 8,
        "each click is its own step back"
    );
    assert_eq!(
        node_height(&h, id),
        height,
        "lighting cells moves nothing on the node"
    );

    // A second click puts a cell out, and undo lights it again.
    h.get_by_label(&format!("stepsequencer{id}.lane4.step16"))
        .click();
    h.run_steps(2);
    assert!(!lit(&h, 3, 15), "the second click puts it out");
    assert!(h.state_mut().undo());
    h.run_steps(2);
    assert!(lit(&h, 3, 15), "and one undo takes back one click");
    assert!(lit(&h, 0, 12), "and nothing else");

    // The playhead, which Step moves, rings the column it is on.
    h.get_by_label(&format!("stepsequencer{id}.step")).click();
    h.run_steps(3);
    h.snapshot("step_sequencer_grid");

    let steps = h.state().undo_len();
    h.get_by_label_contains(&format!("stepsequencer{id}.clear"))
        .click();
    h.run_steps(2);
    for lane in 0..4 {
        for step in 0..16 {
            assert!(
                !lit(&h, lane, step),
                "lane {lane} step {step} survived Clear"
            );
        }
    }
    assert_eq!(h.state().undo_len(), steps + 1, "Clear is one step back");
}

/// **A window cleared to the ground paints no ground of its own.** eframe clears the editor's
/// window to the panels' color before each frame, so no panel and not the canvas fills that
/// color again over a screenful — and the canvas is still drawn on it. Under kittest, which
/// clears to a color of its own, every ground is painted as before.
#[test]
fn a_window_cleared_to_the_ground_paints_no_ground() {
    let grounds = |h: &Harness<'_, App>| {
        let ground = supersilvia::ui::theme::Theme::default().bg_primary();
        let big = h.ctx.content_rect().area() / 8.0;
        h.output()
            .shapes
            .iter()
            .filter(|c| {
                matches!(&c.shape, egui::Shape::Rect(r)
                    if r.fill == ground && r.rect.area() > big)
            })
            .count()
    };
    let dots = |h: &Harness<'_, App>| {
        h.output()
            .shapes
            .iter()
            .filter(|c| matches!(&c.shape, egui::Shape::Circle(d) if d.radius == 1.0))
            .count()
    };
    let mut h = harness();
    h.run_steps(2);
    assert!(
        grounds(&h) >= 2,
        "the central panel's ground and the canvas's, painted over a clear of another color"
    );
    let grid = dots(&h);
    assert!(grid > 0, "the canvas draws its grid");

    supersilvia::ui::theme::apply_cleared(&h.ctx, &supersilvia::ui::theme::Theme::default());
    h.state_mut().set_cleared(true);
    h.run_steps(2);
    assert_eq!(grounds(&h), 0, "the clear is the ground");
    assert_eq!(dots(&h), grid, "and the canvas is drawn on it as before");
}

// -------------------------------------------------------------------------- the paint surface

const ADD_DRAWINGCANVAS: (&str, &str) = ("Source", "add drawingcanvas");

/// A Drawing Canvas at the top of the canvas, so its surface and its brush are on screen, and
/// the surface's own name.
fn drawing_canvas(h: &mut Harness<'_, App>) -> (supersilvia::graph::NodeId, String) {
    add_node(h, ADD_DRAWINGCANVAS);
    let id = h.state().graph().iter().map(|(id, _)| id).next().unwrap();
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, 10.0))],
        })
        .unwrap();
    h.state_mut().end_gesture();
    h.step();
    (id, format!("drawingcanvas{id}.region"))
}

/// The picture the node holds, as its size and bytes.
fn painting_of(
    h: &Harness<'_, App>,
    id: supersilvia::graph::NodeId,
) -> Option<(u32, u32, Vec<u8>)> {
    let node = h.state().graph().get(id)?;
    let painting = supersilvia::nodes::drawingcanvas::painting(node)?;
    let (w, h, px) = painting.pixels()?;
    Some((w, h, px.to_vec()))
}

/// How many pixels of a picture are not the black it starts as.
fn painted(pixels: &[u8]) -> usize {
    pixels.chunks(4).filter(|p| *p != [0, 0, 0, 255]).count()
}

/// The gesture the node exists for: a drag on the surface paints, the node it is drawn on
/// stays where it was — the surface claims the pointer — and the whole stroke is one undo
/// step.
#[test]
fn dragging_on_the_canvas_paints_one_undo_step_and_leaves_the_node_where_it_is() {
    let mut h = tall_harness();
    h.step();
    let (id, surface) = drawing_canvas(&mut h);
    let where_it_was = h.state().graph().get(id).unwrap().pos;
    let steps = h.state().undo_len();
    assert!(painting_of(&h, id).is_none(), "nothing painted yet");

    let rect = h.get_by_label(&surface).rect();
    let from = rect.center() - egui::vec2(60.0, 0.0);
    press_at(&mut h, from);
    for i in 1..=4 {
        move_to(&mut h, from + egui::vec2(30.0 * i as f32, 10.0 * i as f32));
    }
    release_at(&mut h, from + egui::vec2(120.0, 40.0));
    h.snapshot("drawing_canvas");

    let (w, height, px) = painting_of(&h, id).expect("the stroke is on the node");
    assert_eq!((w, height), (512, 512), "at the size Canvas Size names");
    assert!(
        painted(&px) > 200,
        "a stroke's worth of pixels: {}",
        painted(&px)
    );
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        where_it_was,
        "the node came along with the drag"
    );
    assert_eq!(h.state().undo_len(), steps + 1, "a stroke is one undo step");

    // A second stroke is a second step, and undo takes them back one at a time.
    press_at(&mut h, from + egui::vec2(0.0, 60.0));
    move_to(&mut h, from + egui::vec2(80.0, 60.0));
    release_at(&mut h, from + egui::vec2(80.0, 60.0));
    assert_eq!(h.state().undo_len(), steps + 2);
    let two = painted(&painting_of(&h, id).unwrap().2);
    assert!(two > painted(&px));
    h.state_mut().undo();
    h.step();
    assert_eq!(
        painting_of(&h, id).map(|p| p.2),
        Some(px),
        "the first stroke stays"
    );
    h.state_mut().undo();
    h.step();
    assert!(painting_of(&h, id).is_none(), "and goes with one more");
}

/// Symmetry is read where the stroke lands: a stroke down the left under H Mirror paints
/// its mirror down the right.
#[test]
fn a_stroke_under_a_mirror_paints_both_sides() {
    let mut h = tall_harness();
    h.step();
    let (id, surface) = drawing_canvas(&mut h);
    h.state_mut()
        .apply(Command::SetOption {
            node: id,
            key: supersilvia::nodes::drawingcanvas::SYMMETRY,
            value: "h".to_string(),
        })
        .unwrap();
    h.step();

    let rect = h.get_by_label(&surface).rect();
    let from = rect.center() - egui::vec2(100.0, 40.0);
    press_at(&mut h, from);
    move_to(&mut h, from + egui::vec2(0.0, 80.0));
    release_at(&mut h, from + egui::vec2(0.0, 80.0));

    let (w, _, px) = painting_of(&h, id).expect("painted");
    let w = w as usize;
    let left = (0..w / 2)
        .filter(|x| px[(256 * w + x) * 4] > 128)
        .collect::<Vec<_>>();
    let right = (w / 2..w)
        .filter(|x| px[(256 * w + x) * 4] > 128)
        .collect::<Vec<_>>();
    assert!(!left.is_empty(), "the stroke");
    assert!(!right.is_empty(), "its mirror");
    let mirrored: Vec<usize> = left.iter().map(|x| w - 1 - x).rev().collect();
    let near = mirrored
        .iter()
        .zip(&right)
        .all(|(a, b)| a.abs_diff(*b) <= 1);
    assert!(near, "at the mirrored place: {left:?} and {right:?}");
}

/// A tool button puts that tool in the hand, and a shape is written once, when the pointer
/// lets go — the drag before it draws a preview and writes nothing.
#[test]
fn a_tool_button_picks_the_tool_and_a_shape_lands_when_it_lets_go() {
    let mut h = tall_harness();
    h.step();
    let (id, surface) = drawing_canvas(&mut h);
    h.get_by_label(&format!("drawingcanvas{id}.tool.rect"))
        .click();
    h.run_steps(2);
    assert_eq!(
        h.state().graph().get(id).unwrap().options["tool"],
        "rect",
        "the button set the tool"
    );
    let steps = h.state().undo_len();

    let rect = h.get_by_label(&surface).rect();
    let from = rect.center() - egui::vec2(50.0, 50.0);
    press_at(&mut h, from);
    move_to(&mut h, from + egui::vec2(100.0, 100.0));
    assert!(
        painting_of(&h, id).is_none(),
        "a shape being dragged is a preview, not a write"
    );
    release_at(&mut h, from + egui::vec2(100.0, 100.0));
    let (w, _, px) = painting_of(&h, id).expect("the shape landed");
    let w = w as usize;
    let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 4];
    assert_eq!(at(256, 256), [0, 0, 0, 255], "hollow in the middle");
    assert!(painted(&px) > 100, "an outline");
    assert_eq!(h.state().undo_len(), steps + 1, "one write, one step");
}

/// While the surface has focus the keys are its own: a letter picks a tool and a bracket
/// moves the size, as silvia's hint under the canvas says.
#[test]
fn the_canvas_keys_pick_a_tool_and_size_the_brush_while_it_has_focus() {
    let mut h = tall_harness();
    h.step();
    let (id, surface) = drawing_canvas(&mut h);
    // A press on the surface gives it the keys.
    let at = h.get_by_label(&surface).rect().center();
    press_at(&mut h, at);
    release_at(&mut h, at);

    h.key_press(egui::Key::E);
    h.run_steps(2);
    assert_eq!(h.state().graph().get(id).unwrap().options["tool"], "eraser");
    h.key_press(egui::Key::CloseBracket);
    h.run_steps(2);
    assert_eq!(
        float(&h, id, "brushSize"),
        6.0,
        "one pixel up from silvia's 5"
    );
    h.key_press(egui::Key::OpenBracket);
    h.key_press(egui::Key::OpenBracket);
    h.run_steps(2);
    assert_eq!(float(&h, id, "brushSize"), 4.0);

    // A press anywhere else gives the keys back to the editor.
    let elsewhere = h.get_by_label(&surface).rect().right_center() + egui::vec2(200.0, 0.0);
    press_at(&mut h, elsewhere);
    release_at(&mut h, elsewhere);
    h.key_press(egui::Key::L);
    h.run_steps(2);
    assert_eq!(
        h.state().graph().get(id).unwrap().options["tool"],
        "eraser",
        "the canvas let go of the keys"
    );
}

/// The brush's swatches are the node's own colors and open the picker a row's swatch opens.
#[test]
fn the_brush_swatch_opens_the_picker() {
    let mut h = tall_harness();
    h.step();
    let (id, _) = drawing_canvas(&mut h);
    assert!(h.state().open_control().is_none());
    h.get_by_label_contains(&format!("drawingcanvas{id}.brushColor"))
        .click();
    h.step();
    assert!(h.state().open_control().is_some(), "the picker is up");
}

/// A click whose press and release land in one frame is still a stroke: the fill takes the
/// canvas, one step, and Stroke Done's number moves.
#[test]
fn a_click_inside_one_frame_fills() {
    let mut h = tall_harness();
    h.step();
    let (id, surface) = drawing_canvas(&mut h);
    h.state_mut()
        .apply(Command::SetOption {
            node: id,
            key: supersilvia::nodes::drawingcanvas::TOOL,
            value: "fill".to_string(),
        })
        .unwrap();
    h.state_mut().end_gesture();
    h.step();
    let steps = h.state().undo_len();
    let at = h.get_by_label(&surface).rect().center();
    click_with(&mut h, at, egui::Modifiers::NONE);
    h.run_steps(2);
    let (_, _, px) = painting_of(&h, id).expect("the fill landed");
    assert!(
        px.chunks(4).all(|p| p == [255, 255, 255, 255]),
        "the whole blank canvas, in the brush's white"
    );
    assert_eq!(h.state().undo_len(), steps + 1);
}

// ------------------------------------------------------------------------------ the XY Pad

/// An XY Pad with its square on screen, the square itself, and the region's own name.
///
/// Twelve rows over a pad is a tall node: it is put so its foot is at the foot of the canvas,
/// which is where the pointer has to reach the square. A widget egui has clipped away is not
/// hovered however right its rect looks.
fn xy_pad(h: &mut Harness<'_, App>) -> (supersilvia::graph::NodeId, egui::Rect, String) {
    add_node(h, ("Control", "add xypad"));
    let id = h
        .state()
        .graph()
        .iter()
        .find(|(_, n)| n.def.slug == "xypad")
        .expect("the pad")
        .0;
    let zoom = h.state().canvas_transform().zoom;
    let bottom = (h.state().canvas_height() - 12.0) / zoom;
    let height = node_height(h, id);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![(id, Pos2::new(20.0, bottom - height))],
        })
        .unwrap();
    h.run_steps(2);
    let name = format!("xypad{id}.region");
    let region = h.get_by_label(&name).rect();
    let pad = 8.0 * zoom;
    let side = region.width() - 2.0 * pad;
    let square =
        egui::Rect::from_min_size(region.min + egui::vec2(pad, pad), egui::vec2(side, side));
    (id, square, name)
}

/// The gesture the node exists for, in silvia's default Slingshot: pulled back from where it
/// was pressed and let go, the puck snaps back and flies the other way — and the node it is
/// drawn on stays where it was put, which is what `claims_pointer` buys. The whole throw is
/// one step back.
#[test]
fn pulling_the_pad_back_slings_the_puck_and_leaves_the_node_where_it_is() {
    let mut h = tall_harness();
    h.step();
    let (id, square, _) = xy_pad(&mut h);
    let where_it_was = h.state().graph().get(id).unwrap().pos;
    let steps = h.state().undo_len();

    let from = square.center();
    drag_with(
        &mut h,
        from,
        from + egui::vec2(square.width() * 0.25, 0.0),
        egui::Modifiers::NONE,
    );
    assert!(
        float(&h, id, "padX").abs() < 1e-3 && float(&h, id, "padY").abs() < 1e-3,
        "back where it was drawn from: {} {}",
        float(&h, id, "padX"),
        float(&h, id, "padY"),
    );
    assert!(
        (float(&h, id, "vx") + 2.5).abs() < 0.05,
        "flung left at five times the half-pad pull: {}",
        float(&h, id, "vx")
    );
    assert_eq!(
        h.state().graph().get(id).unwrap().pos,
        where_it_was,
        "the node came along with the drag",
    );
    assert_eq!(h.state().undo_len(), steps + 1, "a throw is one undo step");

    h.run_steps(4);
    let x = h.state().uniform(PortRef::new(id, "x")).expect("X");
    assert!(x < -0.05, "the puck is on its way left: {x}");
    h.snapshot("xypad");

    h.state_mut().undo();
    h.step();
    for key in ["padX", "padY", "vx", "vy"] {
        assert_eq!(float(&h, id, key), 0.0, "{key} is back where it was");
    }
}

/// In Cursor mode the puck goes where the hand takes it, inside the pad, and flies on at the
/// speed it was carried at.
#[test]
fn carrying_the_puck_in_cursor_mode_puts_it_under_the_hand() {
    let mut h = tall_harness();
    h.step();
    let (id, square, _) = xy_pad(&mut h);
    h.state_mut()
        .apply(Command::SetOption {
            node: id,
            key: "clickMode",
            value: "cursor".to_string(),
        })
        .unwrap();
    // As the select's own click would have let go: the next gesture is a step of its own.
    h.state_mut().end_gesture();
    h.step();
    let steps = h.state().undo_len();

    // Past the pad's right edge: the puck stops at it.
    let from = square.center();
    drag_with(
        &mut h,
        from,
        egui::pos2(
            square.max.x + 40.0,
            square.center().y - square.height() * 0.25,
        ),
        egui::Modifiers::NONE,
    );
    assert_eq!(float(&h, id, "padX"), 1.0, "clamped to the pad's edge");
    assert!(
        (float(&h, id, "padY") - 0.5).abs() < 0.02,
        "half way up: {}",
        float(&h, id, "padY")
    );
    assert!(float(&h, id, "vx") > 0.0, "and thrown to the right");
    assert_eq!(h.state().undo_len(), steps + 1, "one undo step");
}

/// A preset is silvia's numbered button: it writes its edges and its numbers as one step,
/// and starts its wells, which the document never holds.
#[test]
fn a_preset_on_the_pad_is_one_step_and_starts_its_wells() {
    let mut h = tall_harness();
    h.step();
    let (id, _, _) = xy_pad(&mut h);
    let steps = h.state().undo_len();

    h.get_by_label(&format!("xypad{id}.preset3")).click();
    h.run_steps(3);
    let node = h.state().graph().get(id).unwrap().clone();
    assert_eq!(
        node.options.get("edgeX").map(String::as_str),
        Some("unbound")
    );
    assert_eq!(
        node.options.get("edgeY").map(String::as_str),
        Some("unbound")
    );
    assert_eq!(float(&h, id, "padX"), 0.7, "Orbit starts to the right");
    assert_eq!(float(&h, id, "vy"), 1.6, "moving up");
    assert_eq!(h.state().undo_len(), steps + 1, "a preset is one undo step");
    assert_eq!(
        h.state().puck(id).map(|p| p.wells.len()),
        Some(1),
        "Orbit's one well"
    );
    h.snapshot("xypad_orbit");
}

/// Press, drag and let go with the secondary button, one frame each.
fn right_drag(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.input_mut().events.push(button(from, true));
    h.step();
    h.input_mut().events.push(egui::Event::PointerMoved(to));
    h.step();
    h.input_mut().events.push(button(to, false));
    h.run_steps(3);
}

/// A right-drag on the pad drops a well as strong as the drag was long, and a right-click
/// on it takes it away; neither is an edit, and neither opens a menu.
#[test]
fn a_right_drag_on_the_pad_drops_a_well_and_a_right_click_takes_it() {
    use supersilvia::nodes::cpu::Pull;

    let mut h = tall_harness();
    h.step();
    let (id, square, _) = xy_pad(&mut h);
    let steps = h.state().undo_len();

    let at = square.center() + egui::vec2(-square.width() * 0.25, square.height() * 0.25);
    right_drag(&mut h, at, at + egui::vec2(square.width() * 0.25, 0.0));
    let wells = h
        .state()
        .puck(id)
        .expect("the pad reports its puck")
        .wells
        .clone();
    assert_eq!(wells.len(), 1, "{wells:?}");
    assert!((wells[0].at[0] + 0.5).abs() < 0.02 && (wells[0].at[1] + 0.5).abs() < 0.02);
    let Pull::Gravity(strength) = wells[0].pull else {
        panic!("a gravity well by default: {:?}", wells[0]);
    };
    // A quarter of the square is half a pad unit, silvia's world units at -1 to 1.
    assert!(
        (strength - 1.5).abs() < 0.05,
        "three times the drag: {strength}"
    );
    assert_eq!(h.state().undo_len(), steps, "a well is not an edit");
    assert!(h.query_by_label("Delete").is_none(), "and no menu opened");

    right_click(&mut h, at);
    h.run_steps(2);
    assert!(
        h.state().puck(id).unwrap().wells.is_empty(),
        "the right-click on it took it away"
    );
}

/// The pad's X and Y are the node's own numbers, drawn as the s-number every number is: `Alt`
/// + click learns, the message binds, the cell wears the mark, and the knob moves the puck.
#[test]
fn the_pads_x_learns_a_knob_and_the_knob_moves_the_puck() {
    use supersilvia::midi::{Kind, Message, Trigger};

    let mut h = tall_harness();
    h.step();
    let (id, _, _) = xy_pad(&mut h);
    let name = format!("xypad{id}.padX");
    let cell = h.get_by_label_contains(&name).rect();

    alt_click_at(&mut h, cell.center());
    h.step();
    assert_eq!(
        h.state().midi_learning(),
        Some(PortRef::new(id, "padX").into()),
        "the cell is waiting for a message"
    );
    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 7, value: 0 },
    });
    h.run_steps(2);
    assert_eq!(
        h.state().midi_trigger_of(PortRef::new(id, "padX")),
        Some(Trigger::Control { channel: 0, cc: 7 }),
    );
    h.get_by_label_contains(&format!("xypad{id} padX bound to CC 7 ch 1"));

    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 7, value: 127 },
    });
    h.run_steps(3);
    assert_eq!(
        float(&h, id, "padX"),
        1.0,
        "the knob at the top is the pad's edge"
    );
    assert_eq!(
        h.state().uniform(PortRef::new(id, "x")),
        Some(1.0),
        "and the puck went there"
    );
}

// -------------------------------------------------------------------- the time readout

/// The transport as the last tick left it.
fn transport(h: &Harness<'_, App>) -> supersilvia::transport::Report {
    h.state().transport_state()
}

/// A widget's rect by the start of its accessible name, which for the readout's time carries
/// the reading after it.
fn rect_of(h: &Harness<'_, App>, name: &str) -> egui::Rect {
    h.get_by_label_contains(name).rect()
}

/// **The readout's two buttons and Space do what they say**, and none of them is an edit:
/// pause holds the playhead, Space plays and pauses again, and reset puts the playhead back at
/// zero — with the undo history untouched throughout.
#[test]
fn the_time_readout_pauses_and_resets() {
    let mut h = harness();
    h.step();
    let undo = h.state().undo_len();
    assert!(transport(&h).playing, "a new project plays");
    h.get_by_label_contains(supersilvia::ui::timecode::TIME);

    h.get_by_label(supersilvia::ui::timecode::PAUSE).click();
    h.run_steps(2);
    assert!(!transport(&h).playing, "the button pauses");
    let held = transport(&h).playhead;
    assert!(held > 0.0, "it had played: {held}");
    h.run_steps(5);
    assert_eq!(transport(&h).playhead, held, "and the playhead holds");

    key(&mut h, egui::Key::Space);
    h.step();
    assert!(!transport(&h).playing, "Space is no key of the show's");
    key(&mut h, egui::Key::F8);
    h.step();
    assert!(transport(&h).playing, "F8 plays");
    h.run_steps(3);
    assert!(transport(&h).playhead > held, "and the playhead moves on");
    key(&mut h, egui::Key::F8);
    h.step();
    assert!(!transport(&h).playing, "and pauses");

    h.get_by_label(supersilvia::ui::timecode::RESET).click();
    h.run_steps(2);
    assert_eq!(transport(&h).playhead, 0.0, "back to zero, paused");
    h.get_by_label_contains(&format!("{} 00:00.00", supersilvia::ui::timecode::TIME));
    assert_eq!(
        h.state().undo_len(),
        undo,
        "a hand on the transport is not an edit"
    );
}

/// **View ▸ Time shows and hides the readout**, on by default, and a preference rather than
/// an edit.
#[test]
fn view_time_shows_and_hides_the_readout() {
    let mut h = harness();
    h.step();
    let undo = h.state().undo_len();
    assert!(h.state().preferences().show_time, "on by default");
    h.get_by_label(supersilvia::ui::timecode::PAUSE);

    h.get_by_label("View").click();
    h.run_steps(2);
    h.get_by_label("Time").click();
    h.run_steps(2);
    assert!(!h.state().preferences().show_time);
    assert!(
        h.query_by_label(supersilvia::ui::timecode::PAUSE).is_none(),
        "the readout is gone"
    );

    if h.query_by_label("Time").is_none() {
        h.get_by_label("View").click();
        h.run_steps(2);
    }
    h.get_by_label("Time").click();
    h.run_steps(2);
    assert!(h.state().preferences().show_time);
    h.get_by_label(supersilvia::ui::timecode::PAUSE);
    assert_eq!(h.state().undo_len(), undo, "a preference, not an edit");
}

/// **The readout never moves.** The time is laid out for its widest reading, so a playhead
/// past a hundred minutes is the width one at zero was and neither button beside it shifts.
#[test]
fn the_time_readout_holds_its_width() {
    let mut h = harness();
    h.step();
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    h.state_mut()
        .transport(supersilvia::transport::Command::Seek(0.0));
    h.run_steps(2);
    let time = rect_of(&h, supersilvia::ui::timecode::TIME);
    let pause = rect_of(&h, supersilvia::ui::timecode::PAUSE);
    let reset = rect_of(&h, supersilvia::ui::timecode::RESET);
    h.snapshot("time_readout");

    h.state_mut()
        .transport(supersilvia::transport::Command::Seek(6123.45));
    h.run_steps(2);
    let far = rect_of(
        &h,
        &format!("{} 102:03.45", supersilvia::ui::timecode::TIME),
    );
    assert_eq!(far.width(), time.width(), "the time is fixed width");
    assert_eq!(rect_of(&h, supersilvia::ui::timecode::PAUSE), pause);
    assert_eq!(
        rect_of(&h, supersilvia::ui::timecode::RESET),
        reset,
        "nothing beside it moved"
    );
}

// -------------------------------------------------------------------------- the gears

/// **A gear draws its own picture.** A Master Gear is two seconds a cycle, its rosette a ring
/// of a clock face's twelve ticks; a Ratio Gear at Teeth 1 : 4 on its Cycles says ÷4, and that
/// it closes in four of its input's cycles; the master's caption says a loop of it needs four
/// cycles, eight seconds. Teeth typed 2 : 8 stay 2 : 8 on the row and draw as ÷4. Display ▸
/// Gears draws the same as meshing gears, six teeth driving twenty-four, and its words say the
/// Teeth as typed, 2 : 8.
#[test]
fn a_gear_draws_its_rosette_and_its_gears() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_MASTERGEAR);
    add_node(&mut h, ADD_PHASE);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (master, gear) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (master, Pos2::new(40.0, 40.0)),
                (gear, Pos2::new(300.0, 40.0)),
            ],
        })
        .unwrap();
    h.state_mut()
        .apply(Command::SetControl {
            node: gear,
            key: "q",
            value: supersilvia::graph::ControlValue::Float(4.0),
        })
        .unwrap();
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(master, "cycles"),
            to: PortRef::new(gear, "clock"),
        })
        .unwrap();
    h.run_steps(6);
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    h.run_steps(2);
    h.get_by_label_contains(&format!("ratiogear{gear}.gear ÷4 · every 4 cycles in"));
    h.get_by_label_contains(&format!("ratiogear{gear}.teeth 1 : 4"));
    h.get_by_label_contains(&format!("mastergear{master}.gear 2.000 s · a cycle"));
    h.get_by_label(&format!(
        "mastergear{master}.loop loops in 4 cycles · 8.000 s (÷4 on ratiogear{gear})"
    ));
    h.snapshot("gear_rosette");

    for (key, value) in [("p", 2.0), ("q", 8.0)] {
        h.state_mut()
            .apply(Command::SetControl {
                node: gear,
                key,
                value: supersilvia::graph::ControlValue::Float(value),
            })
            .unwrap();
    }
    h.run_steps(2);
    h.get_by_label_contains(&format!("ratiogear{gear}.teeth 2 : 8"));
    h.get_by_label_contains(&format!("ratiogear{gear}.gear ÷4 · every 4 cycles in"));

    for node in [master, gear] {
        h.state_mut()
            .apply(Command::SetOption {
                node,
                key: "display",
                value: "gears".to_string(),
            })
            .unwrap();
    }
    h.run_steps(2);
    h.get_by_label_contains(&format!(
        "ratiogear{gear}.gear ÷4 · every 4 cycles in · 2 : 8"
    ));
    h.snapshot("gear_gears");
}

/// **Teeth take whole numbers of one or more, typed or dragged.** A Ratio Gear's two numbers
/// stand on one row, `p : q`, with no port: 2.5 typed into p is 3, 0 and −4 typed into q are
/// 1, and a drag along p steps it by whole teeth, up and back down to 1 and no further. The
/// row says what it is set to.
#[test]
fn teeth_take_whole_numbers_of_one_or_more_typed_or_dragged() {
    let mut h = harness();
    h.step();
    add_node(&mut h, ADD_PHASE);
    let id = h.state().graph().iter().next().expect("one node").0;
    assert!(
        h.query_by_label_contains(&format!("ratiogear{id}.p ("))
            .is_none()
            && h.query_by_label_contains(&format!("ratiogear{id}.q ("))
                .is_none(),
        "neither number has a port"
    );
    h.get_by_label_contains(&format!("ratiogear{id}.teeth 1 : 1"));

    let typed = |h: &mut Harness<'_, App>, key: &str, now: &str, text: &str| {
        h.get_by_label_contains(&format!("ratiogear{id}.{key} {now}"))
            .click();
        h.run_steps(2);
        type_text(h, text);
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
    };
    typed(&mut h, "p", "1", "2.5");
    assert_eq!(float(&h, id, "p"), 3.0, "2.5 typed is a whole 3");
    typed(&mut h, "q", "1", "0");
    assert_eq!(float(&h, id, "q"), 1.0, "0 typed is 1");
    typed(&mut h, "q", "1", "-4");
    assert_eq!(float(&h, id, "q"), 1.0, "−4 typed is 1");
    h.get_by_label_contains(&format!("ratiogear{id}.teeth 3 : 1"));

    let rect = number_rect(&h, &format!("ratiogear{id}.p 3"));
    drag_with(
        &mut h,
        rect.center(),
        rect.center() + egui::vec2(22.0, 0.0),
        egui::Modifiers::NONE,
    );
    let up = float(&h, id, "p");
    assert!(
        up > 3.0 && up.fract() == 0.0,
        "dragged up by whole teeth: {up}"
    );
    let rect = number_rect(&h, &format!("ratiogear{id}.p {up}"));
    drag_with(
        &mut h,
        rect.center(),
        rect.center() - egui::vec2(400.0, 0.0),
        egui::Modifiers::SHIFT,
    );
    let down = float(&h, id, "p");
    assert!(
        down >= 1.0 && down.fract() == 0.0,
        "a fine drag is whole too: {down}"
    );
    let rect = number_rect(&h, &format!("ratiogear{id}.p {down}"));
    drag_with(
        &mut h,
        rect.center(),
        rect.center() - egui::vec2(600.0, 0.0),
        egui::Modifiers::NONE,
    );
    assert_eq!(float(&h, id, "p"), 1.0, "and no further down than 1");
}

/// **A Ratio Gear's direction is a switch under its Teeth.** Forward | Reverse, two segments
/// under `p : q`, Forward lit on a new gear; a click on Reverse sets the option, the row says
/// so, and the gear counts down — minus its parent times 3 ÷ 2 — at the same height; a click
/// on Forward puts it back.
#[test]
fn a_ratio_gears_direction_is_a_switch_under_its_teeth() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_MASTERGEAR);
    add_node(&mut h, ADD_PHASE);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (master, gear) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (master, Pos2::new(40.0, 40.0)),
                (gear, Pos2::new(300.0, 40.0)),
            ],
        })
        .unwrap();
    for (key, value) in [("p", 3.0), ("q", 2.0)] {
        h.state_mut()
            .apply(Command::SetControl {
                node: gear,
                key,
                value: supersilvia::graph::ControlValue::Float(value),
            })
            .unwrap();
    }
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(master, "cycles"),
            to: PortRef::new(gear, "clock"),
        })
        .unwrap();
    h.run_steps(6);
    let direction = |h: &Harness<'_, App>| {
        h.state()
            .graph()
            .get(gear)
            .and_then(|n| n.options.get("direction").cloned())
    };
    assert_eq!(
        direction(&h).as_deref(),
        Some("forward"),
        "Forward is the default"
    );
    h.get_by_label(&format!("ratiogear{gear}.direction.forward"));
    h.get_by_label_contains(&format!("ratiogear{gear}.teeth 3 : 2"));
    let forward = node_height(&h, gear);

    h.get_by_label(&format!("ratiogear{gear}.direction.reverse"))
        .click();
    h.run_steps(4);
    assert_eq!(direction(&h).as_deref(), Some("reverse"));
    h.get_by_label(&format!("ratiogear{gear}.teeth 3 : 2 in reverse"));
    assert_eq!(node_height(&h, gear), forward, "at the same height");
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    h.run_steps(2);
    let count = |port| h.state().uniform(PortRef::new(port, "cycles")).unwrap();
    let (m, g) = (count(master), count(gear));
    assert!(m > 0.0, "{m}");
    assert_eq!(g, -(m * 3.0 / 2.0), "it counts down");
    h.snapshot("gear_reverse");

    h.get_by_label(&format!("ratiogear{gear}.direction.forward"))
        .click();
    h.run_steps(2);
    assert_eq!(direction(&h).as_deref(), Some("forward"));
    h.get_by_label(&format!("ratiogear{gear}.teeth 3 : 2"));
}

/// **Below one to one, a rosette winds its one petal over its loops.** A Ratio Gear at ÷2 and
/// one at ÷4 on ambient seconds: each turns once round its picture an input cycle, two loops
/// and four, its radius going in and coming back out once across them, a single petal wound
/// over the cycles it takes to close, with one tick at the top where every loop begins.
#[test]
fn a_rosette_below_one_winds_its_petal_over_its_loops() {
    let mut h = tall_harness();
    h.step();
    add_node(&mut h, ADD_PHASE);
    add_node(&mut h, ADD_PHASE);
    let ids: Vec<_> = h.state().graph().iter().map(|(id, _)| id).collect();
    let (half, quarter) = (ids[0], ids[1]);
    h.state_mut()
        .apply(Command::MoveNodes {
            moves: vec![
                (half, Pos2::new(40.0, 40.0)),
                (quarter, Pos2::new(300.0, 40.0)),
            ],
        })
        .unwrap();
    for (node, q) in [(half, 2.0), (quarter, 4.0)] {
        h.state_mut()
            .apply(Command::SetControl {
                node,
                key: "q",
                value: supersilvia::graph::ControlValue::Float(q),
            })
            .unwrap();
    }
    h.run_steps(6);
    h.state_mut()
        .transport(supersilvia::transport::Command::Pause);
    h.run_steps(2);
    h.get_by_label_contains(&format!(
        "ratiogear{half}.gear ÷2 · every 2 cycles in · 1 petal · 2 loops"
    ));
    h.get_by_label_contains(&format!(
        "ratiogear{quarter}.gear ÷4 · every 4 cycles in · 1 petal · 4 loops"
    ));
    h.snapshot("gear_rosette_below_one");
}

// ------------------------------------------------------------------------- the show's safety net

/// Whether a widget says it is held: the toggled state the mixer's presses carry.
fn lit(h: &Harness<'_, App>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// **Blackout and Freeze are two presses under Mix**, each lit while it holds — named, and
/// selected in the accessibility tree — and each answering `Alt` + click by learning a MIDI
/// binding rather than by pressing, then wearing the dot every bound control wears.
#[test]
fn blackout_and_freeze_are_presses_under_mix_lit_while_they_hold() {
    use supersilvia::midi::{Kind, Message, Target};

    let mut h = tall_harness();
    h.step();
    assert!(
        !lit(&h, "Blackout") && !lit(&h, "Freeze"),
        "neither held at launch"
    );

    h.get_by_label("Blackout").click();
    h.run_steps(2);
    assert!(h.state().mixer().blackout, "the press holds Blackout");
    assert!(lit(&h, "Blackout"), "and it is lit");
    h.get_by_label("Freeze").click();
    h.run_steps(2);
    assert!(h.state().mixer().freeze && lit(&h, "Freeze"));
    h.get_by_label("Blackout").click();
    h.run_steps(2);
    assert!(
        !h.state().mixer().blackout && !lit(&h, "Blackout"),
        "pressed again: let go"
    );
    assert!(h.state().mixer().freeze, "Freeze is its own");
    assert_eq!(h.state().undo_len(), 0, "played, not edited");

    let at = h.get_by_label("Freeze").rect().center();
    alt_click_at(&mut h, at);
    h.step();
    assert_eq!(h.state().midi_learning(), Some(Target::Freeze));
    assert!(h.state().mixer().freeze, "learning is not also a press");
    assert!(h.query_by_label("Freeze (learning MIDI)").is_some());

    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Note { note: 37, on: true },
    });
    h.run_steps(3);
    assert!(
        h.query_by_label("Freeze bound to Note C#2 ch 1").is_some(),
        "the bound press wears the dot"
    );
}

/// Project ▸ Quit, Project ▸ New project…
fn from_project_menu(h: &mut Harness<'_, App>, entry: &str) {
    h.get_by_label("Project").click();
    h.run_steps(2);
    h.get_by_label_contains(entry).click();
    h.run_steps(2);
}

/// **The on-air interlock.** With nothing unsaved, Quit, Open and New still ask while the show
/// is going out — the mix in a window of its own, the mix over NDI — and say what is going
/// out. Cancel leaves it on; the button that goes on is named for what it goes on to.
#[test]
fn quit_and_new_ask_before_stopping_the_show() {
    let mut h = tall_harness();
    h.step();
    assert!(!h.state().dirty());
    assert!(h.state().on_air().is_quiet());

    h.get_by_label("pop out the mix").click();
    h.run_steps(2);
    from_project_menu(&mut h, "Quit");
    assert!(h.query_by_label("The mix is on a screen.").is_some());
    assert!(h.query_by_label("Stop the show?").is_some());
    assert!(
        h.query_by_label("Discard").is_none(),
        "nothing unsaved: the show is the whole question"
    );
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(
        !closed(&h) && !h.state().asking_to_go_on(),
        "Cancel: the show goes on"
    );

    h.state_mut().set_ndi_mix(true);
    from_project_menu(&mut h, "New project");
    assert!(h.query_by_label("The mix is sent over NDI.").is_some());
    assert!(h.query_by_label("New anyway").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);

    from_project_menu(&mut h, "Quit");
    h.get_by_label("Quit anyway").click();
    h.step();
    assert!(closed(&h), "Quit anyway quits");
}

/// The window's own close button asks the same question while the show is going out.
#[test]
fn the_windows_close_asks_while_the_show_is_out() {
    let mut h = tall_harness();
    h.step();
    h.get_by_label("pop out the mix").click();
    h.run_steps(2);
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.step();
    assert!(
        h.output()
            .viewport_output
            .values()
            .any(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)),
        "the close is held"
    );
    h.run_steps(2);
    assert!(h.query_by_label("Stop the show?").is_some());
}

/// A render running is said with its frame, and going on cancels it and waits for it to end
/// before quitting, so what was written stays whole. Unsaved edits ask as they always have,
/// with the render under them.
#[test]
fn quit_during_a_render_cancels_it_and_then_quits() {
    use supersilvia::app::render::{Format, RenderSettings};
    use supersilvia::clock::Warmup;

    let mut h = harness();
    h.step();
    let ws = h.state().graph().default_workspace();
    let add = |h: &mut Harness<'_, App>, slug| {
        h.state_mut()
            .apply(Command::AddNode {
                slug,
                at: Pos2::ZERO,
                workspace: ws,
            })
            .unwrap();
        h.state().graph().iter().map(|(id, _)| id).max().unwrap()
    };
    let cb = add(&mut h, "checkerboard");
    let out = add(&mut h, "output");
    h.state_mut()
        .apply(Command::Connect {
            from: PortRef::new(cb, "output"),
            to: PortRef::new(out, "input"),
        })
        .unwrap();
    let destination =
        std::env::temp_dir().join(format!("supersilvia-ui-onair-{}", std::process::id()));
    h.state_mut()
        .start_render(
            out,
            &RenderSettings {
                fps: 10.0,
                frames: 300,
                warmup: Warmup::Black,
                supersample: 1,
                format: Format::PngSequence,
                destination: destination.clone(),
            },
        )
        .unwrap();
    h.step();
    assert!(h.state().rendering());

    from_project_menu(&mut h, "Quit");
    let said = h
        .query_by_label_contains("A render is running (frame")
        .map(|n| n.accesskit_node().value().unwrap_or_default());
    assert!(
        said.as_deref().is_some_and(|s| s.ends_with("/ 300).")),
        "the render is said with its frame: {said:?}"
    );
    assert!(
        h.query_by_label("Save or Discard cancels the render.")
            .is_some()
    );
    h.get_by_label("Discard").click();
    h.step();
    assert!(!closed(&h), "not while the render is still running");
    let mut quit = false;
    for _ in 0..20 {
        h.step();
        if closed(&h) {
            quit = true;
            break;
        }
    }
    assert!(quit, "it quit once the render had ended");
    assert!(!h.state().rendering(), "canceled, not abandoned");
    let _ = std::fs::remove_dir_all(destination);
}

/// **Soft takeover is a preference, off by default**, in Preferences ▸ Editing.
#[test]
fn soft_takeover_is_a_preference_off_by_default() {
    let mut h = harness();
    h.step();
    open_preferences(&mut h);
    preferences_tab(&mut h, "Editing");
    assert!(!h.state().preferences().midi_soft_takeover);
    h.get_by_label("MIDI soft takeover").scroll_to_me();
    h.run_steps(10);
    assert!(!lit(&h, "MIDI soft takeover"));
    h.get_by_label("MIDI soft takeover").click();
    h.run_steps(2);
    assert!(h.state().preferences().midi_soft_takeover);
}

/// With soft takeover on, a fader out of pick-up leaves its control where it is and the
/// control says where the fader is — the ghost mark, and its accessible name.
#[test]
fn a_fader_out_of_pick_up_wears_a_ghost_on_the_fade() {
    use supersilvia::midi::{Kind, Message, Target, Trigger};

    let mut h = tall_harness();
    h.step();
    h.state_mut().set_soft_takeover(true);
    h.state_mut()
        .bind_midi_unchecked(Trigger::Control { channel: 0, cc: 1 }, Target::Balance);
    h.run_steps(2);
    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Control { cc: 1, value: 127 },
    });
    h.run_steps(3);
    assert_eq!(h.state().mixer().balance, -1.0, "the fade did not jump");
    assert!(
        h.query_by_label("A / B balance -1.00 (MIDI fader at 1.00)")
            .is_some(),
        "the fade says where the fader is"
    );
}

/// **Release all**, in the MIDI window, lets go of every action input a note is holding.
#[test]
fn release_all_in_the_midi_window_lets_go_of_held_notes() {
    use supersilvia::midi::{Kind, Message, Trigger};

    let mut h = harness();
    h.step();
    let ws = h.state().graph().default_workspace();
    h.state_mut()
        .apply(Command::AddNode {
            slug: "output",
            at: Pos2::ZERO,
            workspace: ws,
        })
        .unwrap();
    let out = h.state().graph().iter().next().unwrap().0;
    let port = PortRef::new(out, "show_b");
    h.state_mut().bind_midi_unchecked(
        Trigger::Note {
            channel: 0,
            note: 60,
        },
        port,
    );
    h.step();
    h.state_mut().apply_midi(Message {
        channel: 0,
        kind: Kind::Note { note: 60, on: true },
    });
    h.run_steps(2);
    assert!(h.state().is_held(port), "the note holds it");

    from_project_menu(&mut h, "MIDI…");
    h.get_by_label("Release all").click();
    h.run_steps(3);
    assert!(!h.state().is_held(port), "Release all let it go");
}
