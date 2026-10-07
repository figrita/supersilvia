// SPDX-License-Identifier: AGPL-3.0-or-later

#![forbid(unsafe_code)]

use supersilvia::App;
use supersilvia::app::crashlog;

fn main() -> eframe::Result {
    // `--check`, `--version` and `--help` answer on the terminal and exit before anything opens
    // a window, logging to stderr alone, so a flag leaves the last run's log as it found it.
    // See `supersilvia::check`.
    if let Some(flag) = std::env::args()
        .nth(1)
        .filter(|arg| supersilvia::check::is_flag(arg))
    {
        env_logger::init();
        std::process::exit(supersilvia::check::command_line(&flag).unwrap_or(2));
    }
    // The log file beside stderr, with the last run's ending read out of it first. See
    // `app::crashlog`.
    crashlog::start();
    backtrace_on_panic();
    // The NDI plugin compiled in, registered before any GStreamer element is made, and the
    // user's NDI runtime looked for on a thread of its own. See `video::ndi`.
    supersilvia::video::ndi::start();

    // Read before the window exists, because the window is one of the things remembered. The
    // one store: eframe's `persistence` feature is off, so no `app.ron` is read or written,
    // and the window, the UI zoom and where each of our own windows was left are all here.
    let prefs = supersilvia::preferences::Store::load(supersilvia::preferences::path());
    let window = prefs.get().window;

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("supersilvia")
        // The app id is what a Wayland desktop matches against `supersilvia.desktop` to
        // find the icon for the task switcher, the dock and the window's own decorations,
        // and it is the name the picture windows already give themselves in
        // `render/picture/thread.rs`. The three have to stay the same string.
        .with_app_id("supersilvia")
        .with_icon(icon())
        .with_inner_size(window.size)
        .with_maximized(window.maximized);
    if let Some(position) = window.position {
        viewport = viewport.with_position(position);
    }

    // **The one device**, made before the window: on the adapter `render::adapter` picks —
    // the strongest GPU, discrete before integrated, whatever `SUPERSILVIA_ADAPTER` names
    // instead, and never a software one unless told — with the features the renderer takes. eframe paints through it, the synth draws on it and the
    // picture windows blit on it; a box with no adapter the rule accepts refuses to start and
    // says what it was offered, in a box on the desktop as well as on stderr. See
    // `proposals/wgpu.md`, sections 3 and 4.
    let gpu = match supersilvia::render::Gpu::headless(
        &supersilvia::render::adapter::Asked::from_env(),
    ) {
        Ok(gpu) => gpu,
        Err(why) => {
            log::error!("no GPU to render on: {why}");
            eprintln!("supersilvia: no GPU to render on: {why}");
            crashlog::why(&format!("no GPU to render on: {why}"));
            crashlog::cannot_start(&format!("There is no GPU it can render on.\n\n{why}"));
            std::process::exit(1);
        }
    };
    // Every adapter the instance offered and the one taken, for the Preferences window: the
    // enumeration the pick already made, with no device opened on any other.
    let choice = gpu.choice().cloned();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::Existing(
                eframe::egui_wgpu::WgpuSetupExisting {
                    instance: gpu.instance().clone(),
                    adapter: gpu.adapter().clone(),
                    device: gpu.device().clone(),
                    queue: gpu.queue().clone(),
                },
            ),
            ..Default::default()
        },
        // The editor paints flat: no depth and no MSAA, which a picture's blit inside egui's
        // pass assumes of the pass it is drawn in.
        depth_buffer: 0,
        multisampling: 0,
        viewport,
        ..Default::default()
    };

    // `supersilvia friday/` opens a project, `supersilvia friday/workspaces/tunnel.ssw`
    // the project that file is inside, and a loose `.ssw` becomes a project of its own.
    // The only argument there is past the flags above.
    let open = std::env::args().nth(1).map(std::path::PathBuf::from);

    // `eframe::run_native` on Linux, exactly. On macOS eframe runs inside an event loop the
    // picture windows are made in; see `render::picture::run`.
    let ran = supersilvia::render::picture::run(
        "supersilvia",
        options,
        Box::new(move |cc| {
            let mut app = App::new(cc, prefs);
            app.use_native_menu(&cc.egui_ctx);
            app.use_gpu_choice(choice);
            // Said first, before the recovery question the project below may raise.
            app.notice_last_run(crashlog::last_run());
            match open {
                Some(path) => app.open_argument(&path),
                None => app.open_last_or_untitled(),
            }
            crashlog::window_open();
            Ok(Box::new(app))
        }),
    );
    // The log's last line says how the run ended, for the next launch to read.
    match &ran {
        Ok(()) => crashlog::closed(),
        Err(e) => {
            log::error!("{e}");
            crashlog::why(&e.to_string());
            if !crashlog::window_is_open() {
                crashlog::cannot_start(&format!("It could not open its window.\n\n{e}"));
            }
        }
    }
    ran
}

/// A panic prints its backtrace whether or not `RUST_BACKTRACE` is set; where it is set, the
/// standard hook does as it says. Either way the panic and its backtrace are written into the
/// log, every frame with its address, then where this function was loaded, and one on the
/// main thread before the window is up says so in a box on the desktop.
///
/// The AppImage's binary carries no symbols, so there a frame reads `<unknown>`: the
/// addresses and that last line are what `packaging/appimage/symbolize.sh` names the frames
/// from, with the symbols file its build kept.
fn backtrace_on_panic() {
    let standard = std::panic::take_hook();
    let asked = std::env::var_os("RUST_BACKTRACE").is_some();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>");
        let backtrace = std::backtrace::Backtrace::force_capture();
        if asked {
            standard(info);
        } else {
            eprintln!("\nthread '{name}' {info}\nstack backtrace:\n{backtrace}");
        }
        let loaded = backtrace_on_panic as fn() as usize;
        crashlog::panicked(
            &format!("thread '{name}' {info}"),
            &format!(
                "{}\nsymbols: backtrace_on_panic at {loaded:#x}",
                format!("{backtrace:#}").trim_end()
            ),
        );
        if name == "main" && !crashlog::window_is_open() {
            crashlog::cannot_start(&format!("It stopped while starting.\n\n{info}"));
        }
    }));
}

/// The window icon, for the desktops that take one from the process rather than from a
/// `.desktop` file — X11, Windows, and a Wayland compositor speaking `xdg-toplevel-icon`.
///
/// Baked in at 256 rather than read off disk so the binary is still the whole app when it
/// is run out of a build directory. Every other size, and every packaged form of it, is in
/// `assets/icon/`, rendered from the two SVGs there by `scripts/make-icons.py`.
fn icon() -> eframe::egui::IconData {
    const PNG: &[u8] = include_bytes!("../assets/icon/supersilvia-256.png");
    let image = image::load_from_memory(PNG)
        .expect("the icon is compiled in, so it decodes or the build is broken")
        .into_rgba8();
    let (width, height) = image.dimensions();
    eframe::egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}
