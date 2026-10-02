// SPDX-License-Identifier: AGPL-3.0-or-later

//! The renderer's `IOSurface` import on Metal (`render::dmabuf`), the macOS twin of
//! `tests/gpu_dmabuf.rs`: a frame VideoToolbox decoded, sampled where it lies as two planes
//! and drawn through the conversion pass into the same picture its bytes make, a surface that
//! will not import uploaded as its bytes instead, a frame let go held until the submission
//! after it has finished and no viewer's claim on it is left, and a screen's one BGR plane
//! drawn opaque.
//!
//! The clip's frames are real: `videotestsrc` encoded through the machine's first codec row,
//! H.264 on `vtenc_h264_hw`, and decoded by `vtdec_hw` into its own `CVPixelBuffer`s. The
//! screen's is a `CVPixelBuffer` made here on an `IOSurface`, as ScreenCaptureKit's are. Nothing
//! here opens a camera or a screen.

#![cfg(target_os = "macos")]

#[path = "common/gpu.rs"]
mod gpu;

use gpu::{bytes_of, drain};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use supersilvia::graph::{NodeId, PortRef};
use supersilvia::nodes::{Frame, IoSurface, Layout, Pixels};
use supersilvia::render::dmabuf::Imports;
use supersilvia::render::{FrameJob, Gpu, Renderer, SourceJob};
use supersilvia::video::Delivery;
use supersilvia::video::clip::{Codec, Player};

/// The frame every test here decodes: far enough in that it is not the first the decoder
/// hands out.
const INDEX: u64 = 7;

/// A 30-frame 320×240 clip with a moving ball, encoded as an import encodes, once for the
/// process. `None` where there is no hardware codec pair.
fn clip() -> Option<PathBuf> {
    static CLIP: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    CLIP.get_or_init(|| {
        let Some(codec) = Codec::probe() else {
            eprintln!("no hardware codec pair here; skipping");
            return None;
        };
        gst::init().unwrap();
        let dir =
            std::env::temp_dir().join(format!("supersilvia-iosurface-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp4");
        let description = format!(
            "videotestsrc pattern=ball num-buffers=30 \
             ! video/x-raw,width=320,height=240,framerate=30/1 ! videoconvert \
             ! {} ! mp4mux ! filesink location=\"{}\"",
            codec.encode_chain(),
            path.display()
        );
        let p = gst::parse::launch(&description)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        supersilvia::platform::video::settle_before_eos(&p);
        p.set_state(gst::State::Playing).unwrap();
        let msg = p
            .bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(30),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .expect("encode finished");
        assert!(!matches!(msg.view(), gst::MessageView::Error(_)), "{msg:?}");
        p.set_state(gst::State::Null).unwrap();
        Some(path)
    })
    .clone()
}

/// Frame [`INDEX`] of the clip, decoded with `delivery`, and the player holding it.
fn decoded(delivery: Delivery) -> Option<(Player, Arc<Frame>)> {
    let path = clip()?;
    let mut player = Player::open_with(&path, delivery).expect("the clip opens");
    player.request(INDEX);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some((index, frame)) = player.latest()
            && index == INDEX
        {
            return Some((player, frame));
        }
        assert!(Instant::now() < deadline, "{:?}", player.error());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn surface(frame: &Frame) -> &IoSurface {
    match &frame.pixels {
        Pixels::IoSurface(s) => s,
        other => panic!("not an IOSurface: {other:?}"),
    }
}

/// Hand the renderer one frame on `node`'s port, with nothing drawing it, and let the GPU
/// finish.
fn upload(gpu: &Gpu, renderer: &mut Renderer, node: NodeId, frame: &Arc<Frame>) {
    upload_all(gpu, renderer, &[(node, frame)]);
}

/// Hand the renderer a frame on each node's port in one tick, and let the GPU finish.
fn upload_all(gpu: &Gpu, renderer: &mut Renderer, frames: &[(NodeId, &Arc<Frame>)]) {
    renderer.draw(&FrameJob {
        sources: frames
            .iter()
            .map(|(node, frame)| SourceJob::new(PortRef::new(*node, "frame"), Arc::clone(frame)))
            .collect(),
        ..FrameJob::default()
    });
    drain(gpu);
}

/// **A decoded frame is VideoToolbox's own `IOSurface`, and imports as two planes that hold
/// what its bytes do**: NV12's luma as `R8Unorm` at the frame's size and its chroma as
/// `RG8Unorm` at half, each read back byte for byte equal to the same memory mapped.
#[test]
fn a_decoded_frame_imports_as_two_planes_holding_its_own_bytes() {
    let Some((_player, frame)) = decoded(Delivery::DmaBuf) else {
        return;
    };
    let s = surface(&frame);
    assert_eq!(s.mapped.layout, Layout::Nv12);
    assert_ne!(s.surface, 0);

    let gpu = gpu::gpu();
    let planes = Imports::default()
        .import_surface(gpu.device(), s, frame.width, frame.height)
        .expect("the surface imports");
    assert_eq!(planes.len(), 2, "luma and chroma");
    let wanted = [
        (wgpu::TextureFormat::R8Unorm, 320, 240, 1),
        (wgpu::TextureFormat::Rg8Unorm, 160, 120, 2),
    ];
    for (i, (texture, (format, width, height, texel))) in planes.iter().zip(wanted).enumerate() {
        assert_eq!(texture.format(), format, "plane {i}");
        assert_eq!(
            (texture.width(), texture.height()),
            (width, height),
            "plane {i}"
        );
        let got = bytes_of(&gpu, texture);
        let mapped = s.mapped.plane(i, 320, 240).expect("the plane is mapped");
        let (row, stride) = ((width * texel) as usize, s.mapped.strides[i] as usize);
        for y in 0..height as usize {
            assert_eq!(
                got[y * row..(y + 1) * row],
                mapped[y * stride..y * stride + row],
                "plane {i}, row {y}"
            );
        }
    }
}

/// **An imported frame is drawn through the conversion pass into the picture its bytes make**:
/// the same frame decoded as bytes and uploaded comes out within a step of it, texel for
/// texel, into a texture of the source's own, and what is published holds the frame. The
/// frame is the one asked for: zero copy reads the same stamp on the decoder's input as bytes
/// do.
#[test]
fn an_imported_frame_is_drawn_as_its_bytes_are() {
    let Some((_player, imported)) = decoded(Delivery::DmaBuf) else {
        return;
    };
    let Some((_other, bytes)) = decoded(Delivery::Bytes) else {
        return;
    };
    surface(&imported);
    assert!(
        matches!(bytes.pixels, Pixels::Mapped(_)),
        "a bytes chain stays bytes"
    );

    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &imported), (b, &bytes)]);
    let drawn = renderer.texture_of(a).expect("drawn");
    assert!(
        drawn
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "drawn by the pass into a texture of the source's own"
    );
    assert_eq!(drawn.format(), wgpu::TextureFormat::Rgba8Unorm);
    let (got, want) = (
        bytes_of(&gpu, &drawn),
        bytes_of(&gpu, &renderer.texture_of(b).expect("uploaded")),
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
    assert_eq!(renderer.publish().imports.len(), 1, "one frame claimed");
}

/// **A surface that cannot be imported uploads its bytes instead**: the same frame with no
/// `IOSurface` behind it is refused before Metal is asked, and the picture is its mapped
/// memory, uploaded and converted as a bytes chain's frame is. Nothing is claimed.
#[test]
fn a_surface_that_does_not_import_uploads_its_bytes() {
    let Some((_player, imported)) = decoded(Delivery::DmaBuf) else {
        return;
    };
    let mapped = surface(&imported).mapped.clone();
    let refused = Arc::new(Frame {
        width: imported.width,
        height: imported.height,
        pixels: Pixels::IoSurface(IoSurface {
            surface: 0,
            mapped: mapped.clone(),
            redrawn: None,
        }),
    });
    let as_bytes = Arc::new(Frame {
        width: imported.width,
        height: imported.height,
        pixels: Pixels::Mapped(mapped),
    });

    let gpu = gpu::gpu();
    assert!(
        Imports::default()
            .import_surface(gpu.device(), surface(&refused), 320, 240)
            .is_err(),
        "no surface, no import"
    );
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &refused), (b, &as_bytes)]);
    assert_eq!(
        bytes_of(&gpu, &renderer.texture_of(a).expect("uploaded")),
        bytes_of(&gpu, &renderer.texture_of(b).expect("uploaded")),
        "the refused surface's bytes, uploaded"
    );
    assert!(renderer.publish().imports.is_empty(), "nothing imported");
}

/// **A surface let go is held until the submission after it has finished**: replaced by
/// bytes, the frame stays out while that submission may still sample it, and goes back once
/// it has finished.
#[test]
fn a_surface_let_go_is_held_until_the_next_submission_has_finished() {
    let Some((_player, imported)) = decoded(Delivery::DmaBuf) else {
        return;
    };
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(5);
    upload(&gpu, &mut renderer, node, &imported);
    let held = Arc::strong_count(&imported);

    let red = Arc::new(Frame::solid(320, 240, [255, 0, 0, 255]));
    // Not finished: nothing has asked whether the submission after its last sampler is done.
    renderer.draw(&FrameJob {
        sources: vec![SourceJob::new(
            PortRef::new(node, "frame"),
            Arc::clone(&red),
        )],
        ..FrameJob::default()
    });
    assert_eq!(
        Arc::strong_count(&imported),
        held,
        "replaced, and held until the GPU is done with it"
    );
    drain(&gpu);
    upload(&gpu, &mut renderer, node, &red);
    assert_eq!(
        Arc::strong_count(&imported),
        held - 1,
        "and let go once it is"
    );
}

/// **A viewer's claim holds a surface past the serial**: a frame whose picture is published
/// stays out after the renderer has replaced it and its submissions have finished, and goes
/// back once the `Published` is dropped and the next submission has finished.
#[test]
fn a_surface_a_viewer_still_holds_is_not_let_go_by_the_serial_alone() {
    let Some((_player, imported)) = decoded(Delivery::DmaBuf) else {
        return;
    };
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let node = NodeId(5);
    upload(&gpu, &mut renderer, node, &imported);
    let held = Arc::strong_count(&imported);
    let published = renderer.publish();
    assert_eq!(
        published.imports.len(),
        1,
        "the published picture claims it"
    );

    let red = Arc::new(Frame::solid(320, 240, [255, 0, 0, 255]));
    upload(&gpu, &mut renderer, node, &red);
    upload(&gpu, &mut renderer, node, &red);
    assert!(
        Arc::strong_count(&imported) >= held,
        "a frame a Published still claims does not go back"
    );
    drop(published);
    upload(&gpu, &mut renderer, node, &red);
    upload(&gpu, &mut renderer, node, &red);
    assert_eq!(
        Arc::strong_count(&imported),
        held - 1,
        "and goes back once nothing claims it"
    );
}

/// Planes held in vectors: the bytes a frame carries beside its surface.
struct Copied(Vec<u8>);

impl supersilvia::nodes::Planes for Copied {
    fn plane(&self, i: usize) -> &[u8] {
        if i == 0 { &self.0 } else { &[] }
    }
}

/// A `32BGRA` pixel buffer on an `IOSurface`, as ScreenCaptureKit hands one over, every pixel
/// `at(x, y)`, as the frame a screen publishes: its surface, and its bytes beside it as a
/// padded BGR layout. The buffer is returned too, since it holds the surface.
fn screen_frame(
    width: usize,
    height: usize,
    at: impl Fn(usize, usize) -> [u8; 4],
) -> (
    objc2_core_foundation::CFRetained<objc2_core_video::CVPixelBuffer>,
    Arc<Frame>,
) {
    use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
    use objc2_core_video::{
        CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress,
        CVPixelBufferGetBytesPerRow, CVPixelBufferGetIOSurface, CVPixelBufferLockBaseAddress,
        CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
        kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    };
    use std::ptr::NonNull;

    let empty = CFDictionary::<CFString, CFString>::empty();
    // SAFETY: a CoreVideo key, a static string valid for the life of the process.
    let key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
    let attributes =
        CFDictionary::<CFString, CFDictionary<CFString, CFString>>::from_slices(&[key], &[&empty]);
    let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: the attributes are a dictionary of CoreVideo's own key, and `out` is a local the
    // call writes a +1 reference into.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width,
            height,
            kCVPixelFormatType_32BGRA,
            Some(attributes.as_ref()),
            NonNull::from(&mut out),
        )
    };
    assert_eq!(status, kCVReturnSuccess, "CVPixelBufferCreate");
    // SAFETY: a successful create wrote a buffer we own one reference to.
    let buffer = unsafe { CFRetained::from_raw(NonNull::new(out).expect("a buffer")) };
    let stride = CVPixelBufferGetBytesPerRow(&buffer);
    // SAFETY: locked for writing, written and read within its rows, and unlocked below.
    let bytes = unsafe {
        assert_eq!(
            CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()),
            kCVReturnSuccess
        );
        let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
        for y in 0..height {
            for x in 0..width {
                std::slice::from_raw_parts_mut(base.add(y * stride + x * 4), 4)
                    .copy_from_slice(&at(x, y));
            }
        }
        let bytes = std::slice::from_raw_parts(base, stride * height).to_vec();
        CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        bytes
    };
    let surface = CVPixelBufferGetIOSurface(Some(&buffer)).expect("backed by a surface");
    let frame = Frame {
        width: width as u32,
        height: height as u32,
        pixels: Pixels::IoSurface(IoSurface {
            surface: CFRetained::as_ptr(&surface).as_ptr() as usize,
            mapped: supersilvia::nodes::Mapped {
                layout: Layout::Bgrx,
                strides: [stride as u32, 0, 0],
                yuv: supersilvia::nodes::Yuv::default(),
                data: Arc::new(Copied(bytes)),
            },
            redrawn: None,
        }),
    };
    (buffer, Arc::new(frame))
}

/// **A screen's surface is one BGR plane, drawn opaque**: ScreenCaptureKit leaves what is
/// outside a window clear, so its fourth byte is padding. The plane imports as `Bgra8Unorm`,
/// and the pass draws it into a texture of the source's own with every colour where it was
/// written and alpha one — the same picture its bytes make.
#[test]
fn a_screens_surface_is_drawn_opaque_from_its_one_plane() {
    let at = |x: usize, y: usize| [(x * 4) as u8, (y * 5) as u8, 100, 0];
    let (_buffer, frame) = screen_frame(64, 48, at);
    let s = surface(&frame);

    let gpu = gpu::gpu();
    let planes = Imports::default()
        .import_surface(gpu.device(), s, 64, 48)
        .expect("the surface imports");
    assert_eq!(planes.len(), 1, "one BGR plane");
    assert_eq!(planes[0].format(), wgpu::TextureFormat::Bgra8Unorm);

    let as_bytes = Arc::new(Frame {
        width: frame.width,
        height: frame.height,
        pixels: Pixels::Mapped(s.mapped.clone()),
    });
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let (a, b) = (NodeId(1), NodeId(2));
    upload_all(&gpu, &mut renderer, &[(a, &frame), (b, &as_bytes)]);
    let drawn = renderer.texture_of(a).expect("drawn");
    assert!(
        drawn
            .usage()
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
        "drawn by the pass into a texture of the source's own"
    );
    let got = bytes_of(&gpu, &drawn);
    assert_eq!(
        got,
        bytes_of(&gpu, &renderer.texture_of(b).expect("uploaded")),
        "the picture its bytes make"
    );
    for (x, y) in [(0, 0), (63, 0), (10, 20), (63, 47)] {
        let i = (y * 64 + x) * 4;
        let [b, g, r, _] = at(x, y);
        assert_eq!(got[i..i + 4], [r, g, b, 255], "({x}, {y})");
    }
    assert_eq!(renderer.publish().imports.len(), 1, "one frame claimed");
}

// SAFETY: IOSurface's own lock and unlock, as `IOSurfaceRef.h` declares them.
unsafe extern "C-unwind" {
    fn IOSurfaceLock(buffer: &objc2_io_surface::IOSurfaceRef, options: u32, seed: *mut u32) -> i32;
    fn IOSurfaceUnlock(
        buffer: &objc2_io_surface::IOSurfaceRef,
        options: u32,
        seed: *mut u32,
    ) -> i32;
}

/// A bare BGRA `IOSurface` with **no pixel format set**, as a Syphon server built before
/// October 2025 makes its shared surface, every pixel of memory row `y` `at(x, y)`.
fn unformatted_surface(
    width: usize,
    height: usize,
    at: impl Fn(usize, usize) -> [u8; 4],
) -> objc2_core_foundation::CFRetained<objc2_io_surface::IOSurfaceRef> {
    use objc2_core_foundation::{CFDictionary, CFNumber, CFString};
    use objc2_io_surface::{
        IOSurfaceRef, kIOSurfaceBytesPerElement, kIOSurfaceHeight, kIOSurfaceWidth,
    };
    // SAFETY: IOSurface's own keys, static strings valid for the life of the process.
    let keys: [&CFString; 3] =
        unsafe { [kIOSurfaceWidth, kIOSurfaceHeight, kIOSurfaceBytesPerElement] };
    let values = [
        CFNumber::new_i32(width as i32),
        CFNumber::new_i32(height as i32),
        CFNumber::new_i32(4),
    ];
    let values: Vec<&CFNumber> = values.iter().map(|v| &**v).collect();
    let properties = CFDictionary::<CFString, CFNumber>::from_slices(&keys, &values);
    // SAFETY: a dictionary of IOSurface's own keys, each a number.
    let surface =
        unsafe { IOSurfaceRef::new(properties.as_opaque()) }.expect("an IOSurface is made");
    assert_eq!(surface.pixel_format(), 0, "no pixel format was set");
    // SAFETY: locked for writing, written within its rows, and unlocked with the same options.
    unsafe {
        assert_eq!(IOSurfaceLock(&surface, 0, std::ptr::null_mut()), 0);
        let stride = surface.bytes_per_row();
        let base = surface.base_address().as_ptr().cast::<u8>();
        for y in 0..height {
            for x in 0..width {
                std::slice::from_raw_parts_mut(base.add(y * stride + x * 4), 4)
                    .copy_from_slice(&at(x, y));
            }
        }
        IOSurfaceUnlock(&surface, 0, std::ptr::null_mut());
    }
    surface
}

/// **A Syphon server's surface is copied, whatever it is laid out as, and a pixel format of
/// zero is BGRA.** A surface whose producer draws into it again goes through the pass into a
/// texture of the source's own — alpha kept where the layout carries it, written one where it
/// is padding — and read bottom row first where it is laid out so, so the source's texture is
/// top row first either way. The frame's bytes are not there at all: nothing falls back.
#[test]
fn a_redrawn_surface_is_copied_the_right_way_up_and_format_zero_is_bgra() {
    use supersilvia::nodes::{Mapped, Redrawn, Yuv};
    // Memory row 0 is the picture's bottom: blue and a half-covered white; the last row is
    // its top, red and green. BGRA in memory.
    let (w, h) = (8usize, 4usize);
    let at = |x: usize, y: usize| match (y < h / 2, x < w / 2) {
        (true, true) => [255, 0, 0, 255],
        (true, false) => [128, 128, 128, 128],
        (false, true) => [0, 0, 255, 255],
        (false, false) => [0, 255, 0, 255],
    };
    let surface = unformatted_surface(w, h, at);
    let frame = |layout: Layout, bottom_first: bool| {
        Arc::new(Frame {
            width: w as u32,
            height: h as u32,
            pixels: Pixels::IoSurface(IoSurface {
                surface: objc2_core_foundation::CFRetained::as_ptr(&surface).as_ptr() as usize,
                mapped: Mapped {
                    layout,
                    strides: [surface.bytes_per_row() as u32, 0, 0],
                    yuv: Yuv::default(),
                    data: Arc::new(Copied(Vec::new())),
                },
                redrawn: Some(Redrawn { bottom_first }),
            }),
        })
    };
    let gpu = gpu::gpu();
    let mut renderer = Renderer::new(gpu.clone()).expect("renderer");
    let (see_through, opaque) = (NodeId(1), NodeId(2));
    let (a, b) = (frame(Layout::Bgra, true), frame(Layout::Bgrx, false));
    upload_all(&gpu, &mut renderer, &[(see_through, &a), (opaque, &b)]);
    let rgba = |texture: wgpu::Texture, x: usize, y: usize| {
        let got = bytes_of(&gpu, &texture);
        let i = (y * w + x) * 4;
        <[u8; 4]>::try_from(&got[i..i + 4]).unwrap()
    };
    let flipped = renderer.texture_of(see_through).expect("copied");
    assert_eq!(
        flipped.format(),
        wgpu::TextureFormat::Rgba8Unorm,
        "the source's own"
    );
    for (x, y, want) in [
        (0, 0, [255, 0, 0, 255]),
        (w - 1, 0, [0, 255, 0, 255]),
        (0, h - 1, [0, 0, 255, 255]),
        (w - 1, h - 1, [128, 128, 128, 128]),
    ] {
        assert_eq!(
            rgba(flipped.clone(), x, y),
            want,
            "bottom first, alpha kept: ({x}, {y})"
        );
    }
    let as_laid = renderer.texture_of(opaque).expect("copied");
    for (x, y, want) in [
        (0, 0, [0, 0, 255, 255]),
        (w - 1, 0, [128, 128, 128, 255]),
        (0, h - 1, [255, 0, 0, 255]),
    ] {
        assert_eq!(
            rgba(as_laid.clone(), x, y),
            want,
            "as laid, opaque: ({x}, {y})"
        );
    }
}
