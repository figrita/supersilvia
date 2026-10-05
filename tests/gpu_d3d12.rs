// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's Direct3D 12 import (`render::dmabuf`), the Windows twin of
//! `tests/gpu_dmabuf.rs` and `tests/gpu_iosurface.rs`, in two halves.
//!
//! **The renderer's half**, on textures of the renderer's own device, written here and handed
//! over as a producer hands them: RGBA, NV12 in a texture per plane, a slice of a texture
//! array, and a texture whose fence is signalled only after the frame is submitted, each drawn
//! through the conversion pass into the picture its bytes make; a frame held until no viewer's
//! claim on it is left and the submission after has finished; and a texture of another device
//! with no handle, or in a layout this does not import, refused.
//!
//! **GStreamer's half**: `videotestsrc` through the zero-copy chain into Direct3D 12 textures
//! on GStreamer's device for the renderer's adapter — the camera path minus the camera, and a
//! clip's minus the decoder, which a machine with no Direct3D 12 video decode can still run.
//! On Windows that device is the renderer's own and the frame is the texture; where it is
//! another, the frame crosses as a shared handle, and where neither works it arrives as its
//! bytes. vkd3d, which Wine draws Direct3D 12 with, makes a device per call and implements no
//! shared handle, so under Wine this half proves the frames and the fallback, and the renderer's
//! half proves the import.
//!
//! The textures written here are reached through wgpu-hal, the one `unsafe` in this file.

#![cfg(target_os = "windows")]

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain};
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::nodes::{D3d12Plane, D3d12Texture, Frame, Layout, Mapped, Pixels, Planes, Yuv};
use supersilvia::render::dmabuf::{self, Imports};
use supersilvia::render::{FrameJob, Gpu, Renderer, SourceJob};
use supersilvia::video::{Camera, Delivery, Source};
use wgpu::hal::api::Dx12;
use windows::Win32::Graphics::Direct3D12::{D3D12_FENCE_FLAG_NONE, ID3D12Fence};
use windows::core::Interface;

/// The size every frame here is made at.
const SIZE: (u32, u32) = (64, 48);

/// A device and a renderer serving it, which is what a pipeline is handed the device of.
fn renderer() -> (Gpu, Renderer) {
    let gpu = gpu::gpu();
    let renderer = Renderer::new(gpu.clone()).expect("renderer");
    (gpu, renderer)
}

/// Hand the renderer a frame on each node's port in one tick, and let the GPU finish.
fn upload_all(gpu: &Gpu, renderer: &mut Renderer, frames: &[(NodeId, &Arc<Frame>)]) {
    draw(renderer, frames);
    drain(gpu);
}

/// Hand the renderer a frame on each node's port in one tick.
fn draw(renderer: &mut Renderer, frames: &[(NodeId, &Arc<Frame>)]) {
    renderer.draw(&FrameJob {
        sources: frames
            .iter()
            .map(|(node, frame)| SourceJob::new(PortRef::new(*node, "frame"), Arc::clone(frame)))
            .collect(),
        ..FrameJob::default()
    });
}

/// The two frames' pictures, as the renderer drew them, within a step of each other and not
/// black.
fn same_picture(gpu: &Gpu, renderer: &Renderer, a: NodeId, b: NodeId) {
    let drawn = renderer.texture_of(a).expect("drawn");
    assert!(
        drawn
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "drawn by the pass into a texture of the source's own"
    );
    assert_eq!(drawn.format(), wgpu::TextureFormat::Rgba8Unorm);
    let (got, want) = (
        bytes_of(gpu, &drawn),
        bytes_of(gpu, &renderer.texture_of(b).expect("uploaded")),
    );
    assert_eq!(got.len(), want.len());
    let worst = got
        .iter()
        .zip(&want)
        .map(|(g, w)| g.abs_diff(*w))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 1,
        "the import is its bytes' picture, to within {worst}"
    );
    assert!(
        want.chunks(4).any(|p| p[..3] != [0, 0, 0]),
        "and the picture is not black"
    );
}

// ------------------------------------------------------------------- the renderer's half

/// A frame's planes as vectors, for the bytes a texture is written with.
struct Copied(Vec<Vec<u8>>);

impl Planes for Copied {
    fn plane(&self, i: usize) -> &[u8] {
        &self.0[i]
    }
}

/// Bytes that make a picture with something in every channel: a gradient across and down.
fn gradient(width: u32, height: u32, texel: u32, seed: u8) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((width * height * texel) as usize);
    for y in 0..height {
        for x in 0..width {
            for c in 0..texel {
                bytes.push(((x * 4 + y * 3 + c * 61) as u8).wrapping_add(seed));
            }
        }
    }
    bytes
}

/// An opaque RGBA gradient of [`SIZE`].
fn opaque(seed: u8) -> Vec<u8> {
    gradient(SIZE.0, SIZE.1, 4, seed)
        .chunks(4)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect()
}

/// A texture of the renderer's device in `format`, `layers` deep, with `bytes` in layer
/// `slice`, handed back in the common state a producer leaves a texture in.
fn written(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    layers: u32,
    slice: u32,
    bytes: &[u8],
) -> wgpu::Texture {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("producer"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let texel = format.block_copy_size(None).expect("a plain format");
    gpu.queue().write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: slice,
            },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * texel),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.transition_resources(
        std::iter::empty(),
        std::iter::once(wgpu::TextureTransition {
            texture: &texture,
            selector: None,
            state: wgpu::TextureUses::PRESENT,
        }),
    );
    gpu.queue().submit([encoder.finish()]);
    drain(gpu);
    texture
}

/// `texture`'s `ID3D12Resource`, as an integer.
fn resource_of(texture: &wgpu::Texture) -> usize {
    // SAFETY: the guard is only read through, for the resource's address, and dropped at once;
    // the texture outlives every use of the address, which the frame holding it keeps.
    let hal = unsafe { texture.as_hal::<Dx12>() }.expect("a Direct3D 12 texture");
    // SAFETY: the resource of a live texture, read for its address alone.
    unsafe { hal.raw_resource() }.as_raw() as usize
}

/// A frame of the renderer's device in `textures`, each at `slice` and waited on through
/// `fence` where there is one, held by the frame.
fn on_this_device(
    layout: Layout,
    textures: Vec<wgpu::Texture>,
    slice: u32,
    fence: Option<(ID3D12Fence, u64)>,
) -> Arc<Frame> {
    let here = dmabuf::d3d12_here().expect("a Direct3D 12 renderer");
    let (fence_raw, fence_value) = fence
        .as_ref()
        .map_or((0, 0), |(f, v)| (f.as_raw() as usize, *v));
    Arc::new(Frame {
        width: SIZE.0,
        height: SIZE.1,
        pixels: Pixels::D3d12(D3d12Texture {
            device: here.device,
            textures: textures
                .iter()
                .map(|t| D3d12Plane {
                    resource: resource_of(t),
                    shared: 0,
                    slice,
                    fence: fence_raw,
                    fence_value,
                })
                .collect(),
            layout,
            yuv: Yuv::default(),
            keep: Arc::new((textures, fence.map(|(f, _)| f))),
            refused: None,
        }),
    })
}

/// **An RGBA texture of the renderer's device is drawn as its bytes are**, through the pass,
/// into a texture of the source's own, and what is published holds the frame.
#[test]
fn an_rgba_texture_is_drawn_as_its_bytes_are() {
    let (gpu, mut renderer) = renderer();
    let bytes = opaque(0);
    let texture = written(&gpu, wgpu::TextureFormat::Rgba8Unorm, SIZE, 1, 0, &bytes);
    let imported = on_this_device(Layout::Rgba, vec![texture], 0, None);
    let uploaded = Arc::new(Frame {
        width: SIZE.0,
        height: SIZE.1,
        pixels: Pixels::Bytes(bytes),
    });
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &imported), (b, &uploaded)]);
    same_picture(&gpu, &renderer, a, b);
    assert_eq!(renderer.publish().imports.len(), 1, "one frame claimed");
}

/// **NV12 in a texture per plane is drawn as its bytes are**: luma as `R8Unorm`, chroma as
/// `RG8Unorm` at half size, which is what a producer whose device holds no NV12 texture hands
/// over, converted with the same matrix as the same planes uploaded.
#[test]
fn nv12_in_a_texture_per_plane_is_drawn_as_its_bytes_are() {
    let (gpu, mut renderer) = renderer();
    let (cw, ch) = (SIZE.0 / 2, SIZE.1 / 2);
    let luma = gradient(SIZE.0, SIZE.1, 1, 16);
    let chroma = gradient(cw, ch, 2, 90);
    let textures = vec![
        written(&gpu, wgpu::TextureFormat::R8Unorm, SIZE, 1, 0, &luma),
        written(&gpu, wgpu::TextureFormat::Rg8Unorm, (cw, ch), 1, 0, &chroma),
    ];
    let imported = on_this_device(Layout::Nv12, textures, 0, None);
    let uploaded = Arc::new(Frame {
        width: SIZE.0,
        height: SIZE.1,
        pixels: Pixels::Mapped(Mapped {
            layout: Layout::Nv12,
            strides: [SIZE.0, cw * 2, 0],
            yuv: Yuv::default(),
            straight_alpha: false,
            data: Arc::new(Copied(vec![luma, chroma])),
        }),
    });
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &imported), (b, &uploaded)]);
    same_picture(&gpu, &renderer, a, b);
}

/// **A slice of a texture array is the slice drawn**, as a decoder's array hands one out: the
/// frame is in layer 1 of 3, and the picture is that layer's bytes.
#[test]
fn a_slice_of_a_texture_array_is_the_slice_drawn() {
    let (gpu, mut renderer) = renderer();
    let bytes = opaque(40);
    let texture = written(&gpu, wgpu::TextureFormat::Rgba8Unorm, SIZE, 3, 1, &bytes);
    let imported = on_this_device(Layout::Rgba, vec![texture], 1, None);
    let uploaded = Arc::new(Frame {
        width: SIZE.0,
        height: SIZE.1,
        pixels: Pixels::Bytes(bytes),
    });
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &imported), (b, &uploaded)]);
    same_picture(&gpu, &renderer, a, b);
}

/// **The renderer's queue waits on the producer's fence**: a frame whose fence is signalled only
/// after the tick that samples it is submitted holds that submission back until it is, and is
/// then drawn as its bytes are. Nothing on the CPU waits for it.
#[test]
fn the_queue_waits_on_the_producers_fence() {
    let (gpu, mut renderer) = renderer();
    let bytes = opaque(7);
    let texture = written(&gpu, wgpu::TextureFormat::Rgba8Unorm, SIZE, 1, 0, &bytes);
    let fence: ID3D12Fence = {
        // SAFETY: the guard is only read through, to make one fence on the device, and dropped
        // at once.
        let hal = unsafe { gpu.device().as_hal::<Dx12>() }.expect("a Direct3D 12 device");
        // SAFETY: a plain constructor on a live device.
        unsafe { hal.raw_device().CreateFence(0, D3D12_FENCE_FLAG_NONE) }.expect("a fence")
    };
    let imported = on_this_device(Layout::Rgba, vec![texture], 0, Some((fence.clone(), 1)));
    let node = NodeId(1);
    draw(&mut renderer, &[(node, &imported)]);
    std::thread::sleep(Duration::from_millis(200));
    let status = gpu
        .device()
        .poll(wgpu::PollType::Poll)
        .expect("the device answers");
    assert!(
        !status.is_queue_empty(),
        "the tick waits on the fence the producer has not signalled"
    );
    // SAFETY: a fence of this device, signalled from the CPU as the producer's queue would.
    unsafe { fence.Signal(1) }.expect("signalled");
    drain(&gpu);
    let uploaded = Arc::new(Frame {
        width: SIZE.0,
        height: SIZE.1,
        pixels: Pixels::Bytes(bytes),
    });
    let other = NodeId(2);
    upload_all(
        &gpu,
        &mut renderer,
        &[(node, &imported), (other, &uploaded)],
    );
    same_picture(&gpu, &renderer, node, other);
}

/// **A texture let go is held until no viewer's claim on it is left and the submission after
/// that has finished**: replaced by bytes while its picture is published, the frame stays out,
/// and goes back once the `Published` is dropped and the next submission has finished.
#[test]
fn a_texture_let_go_is_held_until_the_next_submission_has_finished() {
    let (gpu, mut renderer) = renderer();
    let texture = written(
        &gpu,
        wgpu::TextureFormat::Rgba8Unorm,
        SIZE,
        1,
        0,
        &opaque(3),
    );
    let imported = on_this_device(Layout::Rgba, vec![texture], 0, None);
    let node = NodeId(5);
    upload_all(&gpu, &mut renderer, &[(node, &imported)]);
    let held = Arc::strong_count(&imported);
    let published = renderer.publish();
    assert_eq!(
        published.imports.len(),
        1,
        "the published picture claims it"
    );

    let red = Arc::new(Frame::solid(SIZE.0, SIZE.1, [255, 0, 0, 255]));
    upload_all(&gpu, &mut renderer, &[(node, &red)]);
    upload_all(&gpu, &mut renderer, &[(node, &red)]);
    assert!(
        Arc::strong_count(&imported) >= held,
        "replaced, and held while a viewer's claim names it"
    );
    drop(published);
    upload_all(&gpu, &mut renderer, &[(node, &red)]);
    upload_all(&gpu, &mut renderer, &[(node, &red)]);
    assert_eq!(
        Arc::strong_count(&imported),
        held - 1,
        "and let go once nothing on the GPU can read it"
    );
}

/// **A texture on another device with no handle to it is refused**, and so is a texture in a
/// layout this does not import, in more textures than its layout has planes, or in a format its
/// layout does not name.
#[test]
fn a_texture_of_another_device_or_layout_is_refused() {
    let (gpu, _renderer) = renderer();
    let texture = written(
        &gpu,
        wgpu::TextureFormat::Rgba8Unorm,
        SIZE,
        1,
        0,
        &opaque(0),
    );
    let imported = on_this_device(Layout::Rgba, vec![texture], 0, None);
    let Pixels::D3d12(t) = &imported.pixels else {
        unreachable!("made above");
    };
    let refused = |frame: &D3d12Texture| {
        Imports::default()
            .import_d3d12(gpu.device(), gpu.queue(), frame, SIZE.0, SIZE.1)
            .err()
    };
    let elsewhere = D3d12Texture {
        device: t.device ^ 0x10,
        ..t.clone()
    };
    let why = refused(&elsewhere).expect("another device, refused");
    assert!(why.contains("another"), "{why}");
    let i420 = D3d12Texture {
        layout: Layout::I420,
        ..t.clone()
    };
    assert!(refused(&i420).is_some(), "a layout this does not import");
    let three = D3d12Texture {
        layout: Layout::Nv12,
        textures: vec![t.textures[0]; 3],
        ..t.clone()
    };
    assert!(refused(&three).is_some(), "NV12 in three textures");
    let wrong = D3d12Texture {
        layout: Layout::Bgra,
        ..t.clone()
    };
    let why = refused(&wrong).expect("an RGBA texture called BGRA, refused");
    assert!(why.contains("format"), "{why}");
}

// --------------------------------------------------------------------- GStreamer's half

/// The test pattern in the format the device samples, delivered with `delivery`, and the
/// camera holding it.
fn pattern(delivery: Delivery) -> (Camera, Arc<Frame>) {
    let here = dmabuf::d3d12_here().expect("a Direct3D 12 renderer");
    let format = if here.nv12 { "NV12" } else { "RGBA" };
    let source = Source::Described(format!(
        "videotestsrc is-live=true pattern=smpte ! video/x-raw,format={format}"
    ));
    let mut camera = Camera::open_with(&source, Some(SIZE), delivery).expect("a pipeline");
    let deadline = Instant::now() + Duration::from_secs(10);
    let frame = loop {
        if let Some(f) = camera.latest() {
            break f;
        }
        assert!(Instant::now() < deadline, "{:?}", camera.error());
        std::thread::sleep(Duration::from_millis(5));
    };
    (camera, frame)
}

/// **A zero-copy pipeline's frames reach the renderer as its bytes' picture**: GStreamer makes a
/// device on the renderer's adapter, the frames come out of the chain on it, and the picture
/// they make is the one the same pattern makes as bytes — sampled where it lies where the
/// device is the renderer's or shares the texture, and copied down where neither, which is what
/// this half finds under Wine. Which it was is printed.
#[test]
fn a_zero_copy_pipelines_frames_are_drawn_as_its_bytes_are() {
    let (gpu, mut renderer) = renderer();
    let here = dmabuf::d3d12_here().expect("a Direct3D 12 renderer");
    assert!(
        dmabuf::imports(),
        "the renderer imports Direct3D 12 textures"
    );
    assert!(
        supersilvia::platform::video::clip_dmabuf(),
        "GStreamer makes a device on the renderer's adapter"
    );
    let (_a, imported) = pattern(Delivery::DmaBuf);
    let (_b, bytes) = pattern(Delivery::Bytes);
    assert_eq!((imported.width, imported.height), SIZE);
    match &imported.pixels {
        Pixels::D3d12(t) => {
            assert!(
                t.device == here.device || t.textures.iter().all(|p| p.shared != 0),
                "made on the renderer's device, or shared from another"
            );
            println!(
                "d3d12: {:?} in {} texture(s) {}, {} fence(s) to wait on",
                t.layout,
                t.textures.len(),
                if t.device == here.device {
                    "on the renderer's device"
                } else {
                    "shared from another device"
                },
                t.textures.iter().filter(|p| p.fence != 0).count()
            );
        }
        Pixels::Mapped(_) => println!(
            "d3d12: GStreamer's device is not the renderer's and shares no texture; the frames \
             came as bytes copied down from the textures"
        ),
        other => panic!("neither a texture nor bytes: {other:?}"),
    }
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &imported), (b, &bytes)]);
    let (got, want) = (
        bytes_of(&gpu, &renderer.texture_of(a).expect("drawn")),
        bytes_of(&gpu, &renderer.texture_of(b).expect("uploaded")),
    );
    let worst = got
        .iter()
        .zip(&want)
        .map(|(g, w)| g.abs_diff(*w))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 1,
        "the frames are their bytes' picture, to within {worst}"
    );
}
