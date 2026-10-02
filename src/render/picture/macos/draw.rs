// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture window's drawing on macOS: a thread of its own per window, on the one device.
//!
//! **Paced by its own display.** The surface presents in `Fifo`, which is Metal's display
//! sync, with two drawables. The thread blocks in `get_current_texture` until one is free, so
//! it paints once per refresh of the display the window is on and is at most one refresh
//! behind. A thread per window, rather than one for all, because that acquire blocks: on one
//! thread a window on a 60 Hz projector would hold back one on a 120 Hz laptop screen. Metal
//! has no `Mailbox`, and a display link would need Objective-C of our own and macOS 14.
//!
//! **A window nobody can see stops painting.** While the main thread says the window is
//! occluded, the thread blocks on its channel with no timeout, so a hidden window costs no
//! wake-ups. wgpu's own occlusion check answers `Occluded` at once for such a window rather
//! than waiting for a drawable; the thread sleeps on that answer too, with a quarter-second
//! timeout in case wgpu saw the occlusion before AppKit told winit.
//!
//! **It owns the surface and borrows everything else**: the one `Gpu`, the synth's `Live`,
//! and a [`Viewer`] per surface format shared with the other windows. The surface holds an
//! `Arc` of its window, and the main thread holds another until this thread has ended — a
//! winit window dropped off the main thread closes itself by waiting on the main thread.

use super::super::{Shown, Told};
use crate::render::viewer::Viewport;
use crate::render::{Fit, Gpu, Live, Viewer};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// How long the thread sleeps on wgpu's `Occluded` before trying again, where the main thread
/// has not said the window is hidden.
const QUIET: Duration = Duration::from_millis(250);

/// How long the thread waits after a `Timeout` from the surface before trying again.
const BACKOFF: Duration = Duration::from_millis(4);

/// What the main thread tells a window's thread.
#[derive(Clone, Copy)]
pub(super) enum Msg {
    /// The window's size in physical pixels.
    Resized((u32, u32)),
    /// Whether the window can be seen, as AppKit says.
    Occluded(bool),
    /// End: the window is going.
    Close,
}

/// One blit pipeline per surface format, shared by every window's thread. Made on a window's
/// thread, never on the main thread, which is the editor's frame thread on a Mac.
#[derive(Default)]
pub(super) struct Viewers(Mutex<Vec<Arc<Viewer>>>);

impl Viewers {
    fn get(&self, gpu: &Gpu, format: wgpu::TextureFormat) -> Result<Arc<Viewer>, String> {
        let mut viewers = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(viewer) = viewers.iter().find(|v| v.format() == format) {
            return Ok(Arc::clone(viewer));
        }
        let viewer =
            Arc::new(Viewer::new(gpu, format).map_err(|e| format!("the pictures' viewer: {e}"))?);
        viewers.push(Arc::clone(&viewer));
        Ok(viewer)
    }
}

/// Everything a window's thread is handed when it starts.
pub(super) struct Job {
    pub picture: Shown,
    pub surface: wgpu::Surface<'static>,
    /// The window's size in physical pixels when it was made.
    pub physical: (u32, u32),
    pub gpu: Gpu,
    pub live: Arc<Live>,
    pub viewers: Arc<Viewers>,
    pub told: Sender<Told>,
}

/// Start the thread that draws one window. It ends on [`Msg::Close`], when its sender is
/// dropped, or when its surface cannot be configured, having said why.
pub(super) fn spawn(job: Job) -> std::io::Result<(Sender<Msg>, std::thread::JoinHandle<()>)> {
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("picture".into())
        .spawn(move || run(job, &rx))?;
    Ok((tx, thread))
}

/// Why the thread is not painting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hidden {
    /// AppKit says the window cannot be seen: wait for it to say otherwise.
    Occluded,
    /// wgpu said so first: wait, but not for ever.
    Unready,
}

fn run(job: Job, rx: &Receiver<Msg>) {
    let Job {
        picture,
        surface,
        physical,
        gpu,
        live,
        viewers,
        told,
    } = job;
    let (config, viewer) = match configure(&gpu, &surface, physical, &viewers) {
        Ok(pair) => pair,
        Err(why) => {
            log::error!("picture window: {why}");
            let _ = told.send(Told::Failed(picture, why));
            return;
        }
    };
    let mut window = Window {
        picture,
        surface,
        config,
        viewer,
        gpu,
        live,
        hidden: None,
    };
    loop {
        let waited = match window.hidden {
            Some(Hidden::Occluded) => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some(Hidden::Unready) => rx.recv_timeout(QUIET),
            None => Err(RecvTimeoutError::Timeout),
        };
        match waited {
            Ok(msg) => {
                if !window.apply(msg) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if window.hidden == Some(Hidden::Unready) {
                    window.hidden = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if !window.apply(msg) {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if window.hidden.is_none() {
            window.paint();
        }
    }
}

/// The surface configured at `physical`, and the viewer for its format.
fn configure(
    gpu: &Gpu,
    surface: &wgpu::Surface<'static>,
    physical: (u32, u32),
    viewers: &Viewers,
) -> Result<(wgpu::SurfaceConfiguration, Arc<Viewer>), String> {
    let caps = surface.get_capabilities(gpu.adapter());
    // Not sRGB, so the values written are the values the synth drew, as the editor's are.
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        })
        .or_else(|| caps.formats.first().copied())
        .ok_or("the window offers no format")?;
    // Opaque, so nothing behind the window shows through a picture whose alpha is below one.
    let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
        wgpu::CompositeAlphaMode::Opaque
    } else {
        caps.alpha_modes
            .first()
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto)
    };
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        color_space: wgpu::SurfaceColorSpace::default(),
        width: physical.0.max(1),
        height: physical.1.max(1),
        // The display's own pacing, which is the whole of this thread's clock.
        present_mode: wgpu::PresentMode::Fifo,
        // Two drawables: a picture at most one refresh behind.
        desired_maximum_frame_latency: 1,
        alpha_mode,
        view_formats: Vec::new(),
    };
    surface.configure(gpu.device(), &config);
    let viewer = viewers.get(gpu, format)?;
    Ok((config, viewer))
}

/// One window's surface and what it paints with.
struct Window {
    picture: Shown,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    viewer: Arc<Viewer>,
    gpu: Gpu,
    live: Arc<Live>,
    hidden: Option<Hidden>,
}

impl Window {
    /// Act on one message. `false` is the end.
    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Close => return false,
            Msg::Occluded(true) => self.hidden = Some(Hidden::Occluded),
            Msg::Occluded(false) => self.hidden = None,
            Msg::Resized(physical) => self.resize(physical),
        }
        true
    }

    /// Configure the surface again at `physical` pixels, where it is not already.
    fn resize(&mut self, physical: (u32, u32)) {
        let (width, height) = (physical.0.max(1), physical.1.max(1));
        if (self.config.width, self.config.height) != (width, height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(self.gpu.device(), &self.config);
        }
    }

    /// Blit the newest picture the synth has finished, and present it.
    ///
    /// Nothing waits on the synth. The `Published` is held until after the submit that carries
    /// the blit, so the synth draws nothing into what it names before the GPU has read it.
    fn paint(&mut self) {
        let published = self.live.get();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Occluded => {
                self.hidden = Some(Hidden::Unready);
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(self.gpu.device(), &self.config);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Validation => {
                std::thread::sleep(BACKOFF);
                return;
            }
        };
        let size = (self.config.width, self.config.height);
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("picture window"),
                });
        crate::render::shared::clear(&mut encoder, &view, wgpu::Color::BLACK);
        let whole = Viewport::whole(size);
        match self.picture {
            Shown::Node { node, port } => self.viewer.show_node(
                &mut encoder,
                &view,
                size,
                &published,
                node,
                port,
                whole,
                Fit::Letterbox,
                0.0,
            ),
            Shown::Mix => {
                self.viewer.show_mixer(
                    &mut encoder,
                    &view,
                    size,
                    &published,
                    whole,
                    Fit::Letterbox,
                );
            }
        }
        self.gpu.submit([encoder.finish()]);
        self.gpu.queue().present(frame);
        // Only now: the submit carrying the blit is made.
        drop(published);
    }
}
