// SPDX-License-Identifier: AGPL-3.0-or-later

//! The wgpu renderer's DMA-BUF import (`render::dmabuf`): what Vulkan says it takes, a real
//! frame the video engine exported imported and sampled with no copy, the producer's fd left
//! open by every import, what is refused before the driver sees it, and a frame let go held
//! until the submission after it has finished and no viewer's claim on it is left.
//!
//! The frames are real: `videotestsrc` through `vapostproc` into DMA-BUF memory, the camera path
//! minus the camera. A box with no VA-API
//! export or no Vulkan import skips those tests and says so.

#[path = "common/gpu.rs"]
mod gpu;

use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::nodes::{DmaBuf, Frame, Pixels};
use supersilvia::render::Gpu;
use supersilvia::render::dmabuf::{self, AR24, FOURCCS, Imports, XR24};
use supersilvia::video::{Camera, Delivery, Source};

/// A device, or `None` where it has no DMA-BUF import. A Mac's imports `IOSurface`s instead,
/// which `tests/gpu_iosurface.rs` holds.
fn importing_gpu() -> Option<Gpu> {
    let gpu = gpu::gpu();
    if cfg!(target_os = "linux") && dmabuf::supported(gpu.device()) {
        Some(gpu)
    } else {
        eprintln!("this device has no DMA-BUF import; skipping");
        None
    }
}

/// A 64×64 colour-bar frame the video engine exported as a DMA-BUF, with the pipeline that
/// keeps it valid — or `None` where the box exports none.
fn exported_frame() -> Option<(Camera, Arc<Frame>)> {
    if supersilvia::platform::video::dmabuf_format().is_none() {
        eprintln!("no DMA-BUF export here; skipping");
        return None;
    }
    let mut camera = Camera::open_with(&Source::Test, Some((64, 64)), Delivery::DmaBuf)
        .expect("a DMA-BUF pipeline");
    let deadline = Instant::now() + Duration::from_secs(5);
    let frame = loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        matches!(frame.pixels, Pixels::DmaBuf(_)),
        "delivered as a descriptor"
    );
    Some((camera, frame))
}

fn descriptor(frame: &Frame) -> &DmaBuf {
    match &frame.pixels {
        Pixels::DmaBuf(b) => b,
        _ => panic!("not a DMA-BUF"),
    }
}

/// `texture` sampled by a fragment shader into an `Rgba8Unorm` target of its size, with
/// `textureLoad` at the fragment's own texel, and read back: RGBA whatever the texture's byte
/// order, rows as they lie in the texture's memory.
fn sampled(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<[u8; 4]> {
    const SAMPLE: &str = "
        @group(0) @binding(0) var t: texture_2d<f32>;

        @vertex
        fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
            let p = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
            return vec4f(p * 2.0 - 1.0, 0.0, 1.0);
        }

        @fragment
        fn fs_main(@builtin(position) p: vec4f) -> @location(0) vec4f {
            return textureLoad(t, vec2i(p.xy), 0);
        }
    ";
    let device = gpu.device();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sample"),
        source: wgpu::ShaderSource::Wgsl(SAMPLE.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sample"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        multiview_mask: None,
        cache: None,
    });
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sampled"),
        size: texture.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sample"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(
                &texture.create_view(&wgpu::TextureViewDescriptor::default()),
            ),
        }],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sample"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.submit([encoder.finish()]);
    gpu::bytes_of(gpu, &target).as_chunks::<4>().0.to_vec()
}

/// **What a screen cast may be handed is asked of Vulkan**: every pair is one of the four RGB
/// fourccs, linear `XR24` — the one every compositor can share in — is among them, and so is
/// the tiling the video engine exports a clip's frames in.
#[test]
fn the_formats_a_producer_may_hand_over_are_asked_of_vulkan() {
    let Some(gpu) = importing_gpu() else { return };
    let formats = dmabuf::importable(gpu.device());
    println!("importable: {formats:x?}");
    assert!(
        formats.iter().all(|(f, _)| FOURCCS.contains(f)),
        "only the RGB fourccs: {formats:x?}"
    );
    assert!(
        formats.contains(&(XR24, 0)),
        "linear XR24 is importable: {formats:x?}"
    );
    if let Some(export) = supersilvia::platform::video::dmabuf_format() {
        let (fourcc, modifier) = export.split_once(':').expect("FOURCC:0xMODIFIER");
        let fourcc = u32::from_le_bytes(fourcc.as_bytes().try_into().expect("four letters"));
        let modifier = u64::from_str_radix(modifier.trim_start_matches("0x"), 16)
            .expect("a hexadecimal modifier");
        assert!(
            formats.contains(&(fourcc, modifier)),
            "what vapostproc exports ({export}) is importable: {formats:x?}"
        );
    }
}

/// **A frame the video engine exported is imported and sampled without a copy**: the colour
/// bars come out of the texture in their own colours — white first, then yellow, then cyan —
/// which a black, garbage or byte-swapped texture would not give.
#[test]
fn an_exported_frame_is_imported_and_sampled_in_its_own_colours() {
    let Some(gpu) = importing_gpu() else { return };
    let Some((_camera, frame)) = exported_frame() else {
        return;
    };
    let buf = descriptor(&frame);
    println!("importing {buf:?}");
    let mut imports = Imports::default();
    let texture = imports
        .import(gpu.device(), buf, frame.width, frame.height)
        .expect("the exported frame imports");
    assert_eq!((texture.width(), texture.height()), (64, 64));
    let expected = if buf.fourcc == AR24 || buf.fourcc == XR24 {
        wgpu::TextureFormat::Bgra8Unorm
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    assert_eq!(texture.format(), expected);

    let px = sampled(&gpu, &texture);
    // The frame's top row is the first in memory, as the producer wrote it. Seven bars across
    // the top two thirds; their centres are at 64 × (k + ½) / 7.
    let at = |x: usize, y: usize| px[y * 64 + x];
    let bar = |k: usize| at((64 * (2 * k + 1)) / 14, 4);
    let [white, yellow, cyan] = [bar(0), bar(1), bar(2)];
    println!("white {white:?}, yellow {yellow:?}, cyan {cyan:?}");
    assert!(
        white[..3].iter().all(|c| *c > 150)
            && white[..3].iter().max().unwrap() - white[..3].iter().min().unwrap() < 30,
        "the first bar is white: {white:?}"
    );
    assert!(
        yellow[0] > yellow[2] + 100 && yellow[1] > yellow[2] + 100,
        "the second bar is yellow, red and green over blue: {yellow:?}"
    );
    assert!(
        cyan[1] > cyan[0] + 100 && cyan[2] > cyan[0] + 100,
        "the third bar is cyan, green and blue over red: {cyan:?}"
    );
    if !dmabuf::alpha_is_padding(buf.fourcc) {
        assert!(white[3] > 250, "an opaque frame is opaque: {white:?}");
    }
}

/// **Every import takes a `dup` of the producer's fd**, never the fd itself: the same frame
/// imports twice while both textures live, and again once both have gone. Were Vulkan handed
/// the producer's own fd it would close it — Mesa at the import, any driver once the memory is
/// freed — and a later import would find it gone.
#[test]
fn an_import_leaves_the_producers_fd_open() {
    let Some(gpu) = importing_gpu() else { return };
    let Some((_camera, frame)) = exported_frame() else {
        return;
    };
    let buf = descriptor(&frame);
    let mut imports = Imports::default();
    let mut import = || {
        imports
            .import(gpu.device(), buf, frame.width, frame.height)
            .expect("the frame imports")
    };
    let first = import();
    let second = import();
    drop((first, second));
    gpu::drain(&gpu);
    let third = import();
    let px = sampled(&gpu, &third);
    assert!(
        px.iter().any(|p| p[..3] != [0, 0, 0]),
        "the third import still holds the picture"
    );
}

/// **What the driver would be handed wrong is refused before it**, each with a reason: a closed
/// fd, a fourcc that is not RGB, a modifier the device does not list, a frame with no pixels, a
/// stride short of the row, and a frame past the largest the format imports at.
#[test]
fn a_descriptor_the_device_cannot_take_is_refused_with_a_reason() {
    let Some(gpu) = importing_gpu() else { return };
    let base = DmaBuf {
        fd: 0,
        fourcc: XR24,
        modifier: 0,
        stride: 256,
        offset: 0,
        keep: Arc::new(()),
        refused: None,
    };
    let listed: Vec<u64> = dmabuf::importable(gpu.device())
        .into_iter()
        .map(|(_, m)| m)
        .collect();
    // A modifier with a vendor no driver claims.
    let unlisted = 0x7f00_0000_0000_0001;
    assert!(!listed.contains(&unlisted));
    let cases = [
        (
            DmaBuf {
                fd: -1,
                ..base.clone()
            },
            64,
            64,
        ),
        (
            DmaBuf {
                fourcc: u32::from_le_bytes(*b"NV12"),
                ..base.clone()
            },
            64,
            64,
        ),
        (
            DmaBuf {
                modifier: unlisted,
                ..base.clone()
            },
            64,
            64,
        ),
        (base.clone(), 0, 64),
        (
            DmaBuf {
                stride: 64 * 4 - 4,
                ..base.clone()
            },
            64,
            64,
        ),
        (
            DmaBuf {
                stride: u32::MAX,
                ..base.clone()
            },
            1 << 20,
            64,
        ),
    ];
    let mut imports = Imports::default();
    for (buf, width, height) in cases {
        let why = imports
            .import(gpu.device(), &buf, width, height)
            .expect_err("refused");
        println!("{width}×{height} {buf:?}: {why}");
        assert!(!why.is_empty());
    }
}

/// **A frame let go is held until the submission after it has finished**, and a frame let go
/// after that submission is not released by it.
#[test]
fn a_frame_let_go_is_held_until_the_next_submission_has_finished() {
    let gpu = gpu::gpu();
    let mut imports = Imports::default();
    let early = Arc::new(Frame::solid(4, 4, [255, 0, 0, 255]));
    let late = Arc::new(Frame::solid(4, 4, [0, 255, 0, 255]));

    imports.let_go(Arc::clone(&early));
    let encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let ticket = gpu.submit([encoder.finish()]);
    imports.submitted(ticket.serial);
    imports.let_go(Arc::clone(&late));

    imports.sweep(ticket.serial - 1);
    assert_eq!(
        Arc::strong_count(&early),
        2,
        "held while its submission runs"
    );

    gpu::drain(&gpu);
    gpu.wait(&ticket, Duration::from_secs(5)).expect("finished");
    imports.sweep(gpu.completed());
    assert_eq!(Arc::strong_count(&early), 1, "back once it has finished");
    assert_eq!(
        Arc::strong_count(&late),
        2,
        "let go after that submission, so not released by it"
    );

    let encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let next = gpu.submit([encoder.finish()]);
    imports.submitted(next.serial);
    gpu.wait(&next, Duration::from_secs(5)).expect("finished");
    imports.sweep(gpu.completed());
    assert_eq!(Arc::strong_count(&late), 1, "back after the next one");
}

/// An empty submission on `gpu`, waited for, and its serial.
fn finished_submission(gpu: &Gpu) -> u64 {
    let encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let ticket = gpu.submit([encoder.finish()]);
    gpu.wait(&ticket, Duration::from_secs(5)).expect("finished");
    ticket.serial
}

/// **A viewer's claim outlives the renderer's**: a frame the renderer has let go of, whose
/// submission has finished, stays out while a `Published` holds its claim — a viewer's blit of
/// it may be submitted after that serial — and goes back only after the viewer drops the claim,
/// on its own thread, and the synth's next submission has finished.
#[test]
fn a_frame_a_viewer_still_holds_is_not_let_go_by_the_serial_alone() {
    let gpu = gpu::gpu();
    let mut imports = Imports::default();
    let frame = Arc::new(Frame::solid(4, 4, [0, 0, 255, 255]));

    let first = imports.claim(&frame);
    let again = imports.claim(&frame);
    assert!(
        Arc::ptr_eq(first.frame(), again.frame()),
        "one claim a frame"
    );
    drop(again);

    imports.let_go(Arc::clone(&frame));
    let serial = finished_submission(&gpu);
    imports.submitted(serial);
    imports.sweep(gpu.completed());
    assert!(
        Arc::strong_count(&frame) > 1,
        "the viewer's claim keeps it out after the serial"
    );

    std::thread::spawn(move || drop(first))
        .join()
        .expect("the viewer thread");
    imports.sweep(gpu.completed());
    assert!(
        Arc::strong_count(&frame) > 1,
        "a serial from before the viewer let go does not cover it"
    );

    let next = finished_submission(&gpu);
    imports.submitted(next);
    imports.sweep(gpu.completed());
    assert_eq!(Arc::strong_count(&frame), 1, "back once both are done");
}

/// **A frame whose every claim has gone goes back on the serial alone.**
#[test]
fn an_unheld_frame_goes_back_on_the_next_submission() {
    let gpu = gpu::gpu();
    let mut imports = Imports::default();
    let frame = Arc::new(Frame::solid(4, 4, [0, 0, 255, 255]));
    drop(imports.claim(&frame));
    imports.let_go(Arc::clone(&frame));
    let serial = finished_submission(&gpu);
    imports.submitted(serial);
    imports.sweep(gpu.completed());
    assert_eq!(Arc::strong_count(&frame), 1);
}
