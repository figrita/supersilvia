// SPDX-License-Identifier: AGPL-3.0-or-later

//! A surface configures while the synth submits on the same device.
//!
//! `Surface::configure` in wgpu-core 30.0.1 waits for the device to go idle and refuses the
//! configuration if another thread submitted during the wait — which on the one device the
//! synth, the editor and the picture windows share is every configure of a busy synth: the
//! editor's surface at launch and on a resize, and each picture window's. The refusal is an
//! uncaptured error, and wgpu's default handler panics on it. `vendor/wgpu-core` carries the
//! upstream fix, gfx-rs/wgpu#10296, and this is the test that holds it there: without the
//! fix nearly every configure here fails.
//!
//! **macOS only.** The surface is a bare `CAMetalLayer`, which Metal presents to with no
//! window and no display; Vulkan has no surface without a window system, so elsewhere the
//! test says it is skipped and passes. The layer is made through the Objective-C runtime
//! directly, since no crate the tests may name makes one — hence this file's `unsafe`, which
//! is three calls into that runtime and the surface made on the layer.

#[path = "common/gpu.rs"]
mod gpu;

/// How many times the surface is configured while the synth submits.
#[cfg(target_os = "macos")]
const CONFIGURES: usize = 30;

/// A few milliseconds of fragment work per submission at 1080p on an M2, so the synth is
/// on the GPU for most of every configure's wait.
#[cfg(target_os = "macos")]
const SHADER: &str = "
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array(vec2f(-1.0, -3.0), vec2f(3.0, 1.0), vec2f(-1.0, 1.0));
    return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fs_main(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    var x = pos.x * 0.001;
    for (var k = 0; k < 96; k++) { x = fract(sin(x * 12.9898 + f32(k)) * 43758.5453); }
    return vec4f(x, x, x, 1.0);
}
";

/// **A surface configures while another thread submits**, as the editor's and each picture
/// window's do while the synth runs: the synth's pace behind the renderer's own throttle,
/// back to back, and the surface configured again and again at two sizes meanwhile, with
/// not one configure refused.
#[test]
#[cfg(target_os = "macos")]
fn a_surface_configures_while_the_synth_submits() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;
    use supersilvia::render::queue::{Recording, Throttle};

    let gpu = gpu::gpu();
    let refused = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let refused = Arc::clone(&refused);
        gpu.device().on_uncaptured_error(Arc::new(move |e| {
            refused
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(e.to_string());
        }));
    }

    let layer = metal_layer::Layer::new();
    let surface = layer.surface(gpu.instance());
    let caps = surface.get_capabilities(gpu.adapter());
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: caps.formats[0],
        color_space: wgpu::SurfaceColorSpace::default(),
        width: 1470,
        height: 920,
        present_mode: wgpu::PresentMode::Fifo,
        desired_maximum_frame_latency: 1,
        alpha_mode: caps.alpha_modes[0],
        view_formats: Vec::new(),
    };
    surface.configure(gpu.device(), &config);

    let module = gpu
        .device()
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("busy"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
    let pipeline = gpu
        .device()
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("busy"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
    let target = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("busy"),
        size: wgpu::Extent3d {
            width: 1920,
            height: 1080,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let stop = Arc::new(AtomicBool::new(false));
    let synth = {
        let (gpu, stop) = (gpu.for_synth(), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut throttle = Throttle::default();
            while !stop.load(Ordering::Relaxed) {
                let mut recording = Recording::new(&gpu, "busy");
                {
                    let mut pass =
                        recording
                            .encoder()
                            .begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("busy"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                ..wgpu::RenderPassDescriptor::default()
                            });
                    pass.set_pipeline(&pipeline);
                    pass.draw(0..3, 0..1);
                }
                throttle.submit(&gpu, recording);
            }
            throttle.submissions
        })
    };

    std::thread::sleep(Duration::from_millis(100));
    for i in 0..CONFIGURES {
        config.width = if i % 2 == 0 { 1400 } else { 1470 };
        surface.configure(gpu.device(), &config);
        std::thread::sleep(Duration::from_millis(2));
    }
    stop.store(true, Ordering::Relaxed);
    let submissions = synth.join().expect("the synth thread ends");
    gpu::drain(&gpu);
    drop(surface);
    drop(layer);

    assert!(
        submissions > CONFIGURES as u64,
        "the synth submitted {submissions} times across {CONFIGURES} configures, too few to race them",
    );
    let refused = refused.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(
        refused.is_empty(),
        "{} of {CONFIGURES} configures refused while the synth submitted {submissions} times; the first: {}",
        refused.len(),
        refused[0],
    );
}

#[test]
#[cfg(not(target_os = "macos"))]
fn a_surface_configures_while_the_synth_submits() {
    eprintln!("no surface without a window here; skipping");
}

/// A `CAMetalLayer` of its own, and a surface on it.
#[cfg(target_os = "macos")]
mod metal_layer {
    use std::ffi::{CStr, c_void};
    use std::ptr::NonNull;

    #[link(name = "objc")]
    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        fn objc_getClass(name: *const std::ffi::c_char) -> *mut c_void;
        fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
        /// Declared at the one signature it is called with here: a receiver and a selector,
        /// and an object pointer back, which `+new` returns and `-release`'s caller ignores.
        fn objc_msgSend(receiver: *mut c_void, selector: *mut c_void) -> *mut c_void;
    }

    /// Send `selector`, which takes no argument, to `receiver`.
    ///
    /// # Safety
    /// `receiver` is a live object or class that answers `selector` with an object or nothing.
    unsafe fn send(receiver: *mut c_void, selector: &CStr) -> *mut c_void {
        // SAFETY: `selector` is a NUL-terminated name, and the caller vouches for `receiver`.
        unsafe { objc_msgSend(receiver, sel_registerName(selector.as_ptr())) }
    }

    /// One retained `CAMetalLayer`, released on drop.
    pub struct Layer(NonNull<c_void>);

    impl Layer {
        pub fn new() -> Self {
            // SAFETY: QuartzCore is linked, so the class exists, and `+new` on it returns a
            // retained layer this `Layer` owns.
            let layer = unsafe { send(objc_getClass(c"CAMetalLayer".as_ptr()), c"new") };
            Self(NonNull::new(layer).expect("a CAMetalLayer"))
        }

        /// A surface on the layer. It must be dropped before the layer is.
        pub fn surface(&self, instance: &wgpu::Instance) -> wgpu::Surface<'static> {
            // SAFETY: the pointer is a live `CAMetalLayer`, and the test drops the surface
            // before the layer.
            unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                    self.0.as_ptr(),
                ))
            }
            .expect("a surface on the layer")
        }
    }

    impl Drop for Layer {
        fn drop(&mut self) {
            // SAFETY: the layer is live and this `Layer` holds the retain `+new` gave it.
            unsafe { send(self.0.as_ptr(), c"release") };
        }
    }
}
