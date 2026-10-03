// SPDX-License-Identifier: AGPL-3.0-or-later

//! GStreamer's Direct3D 12 library, `gstreamer-d3d12-1.0`, for the frames that reach the
//! renderer with no copy: the device a pipeline makes its textures on, and the texture, the
//! array slice and the fence behind a sample. gstreamer-rs has no binding for it, so the few
//! calls are declared here and linked from the release's own import library.
//!
//! **The device is the renderer's adapter's.** [`context`] hands a pipeline, as a `GstContext`,
//! the `GstD3D12Device` GStreamer makes for the renderer's adapter LUID. On Windows
//! `D3D12CreateDevice` hands one process one device per adapter, so that is the renderer's own
//! `ID3D12Device`, and a frame is the texture itself, waited on by the renderer's queue through
//! the producer's fence. **Where it is another** — vkd3d, which Wine draws Direct3D 12 with,
//! makes a device per call — a frame crosses as an NT handle the renderer opens on its own
//! device, which GStreamer's textures allow since its pools make them in shared heaps, and its
//! fence, which belongs to the other device, is waited on here, on GStreamer's streaming
//! thread, before the frame is handed on; a texture that cannot be shared is mapped as bytes,
//! which GStreamer copies down. Every frame is checked by the device its memory names. The
//! renderer's device is asked of `render/dmabuf.rs`, the edge out of the pure modules' reach
//! that `tests/rules.rs` names, as it names `video.rs`'s.
//!
//! **A frame holds its buffer and its fence.** The buffer keeps the texture, and its handle,
//! out of the producer's pool, and the fence — the producer's queue's, signalled once the
//! frame is written — is taken with a reference of its own, so the renderer's queue can wait
//! on it for as long as the frame lives. A memory written on the CPU and not yet uploaded is
//! uploaded by the map that asks for its texture, which leaves its fence on the upload.
//!
//! **A decoder on the renderer's adapter is preferred.** GStreamer registers a decoder element
//! per adapter, ranked by the order DXGI lists them, so on a machine with two GPUs the first
//! one `decodebin` takes may decode on a device the renderer is not on; [`context`] ranks the
//! decoders whose `adapter-luid` is the renderer's above every other, once.
//!
//! **`unsafe` is the library's calls**, which are C, and the memory's device read off its
//! public structure. Every block carries a `// SAFETY:` line.

use crate::nodes::{D3d12Plane, D3d12Texture, Frame, Layout, Pixels};
use crate::render::dmabuf::D3d12Device;
use gstreamer as gst;
use gstreamer::glib;
use gstreamer::glib::translate::{FromGlibPtrFull, IntoGlib};
use gstreamer::prelude::*;
use gstreamer_video as gst_video;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, Once, PoisonError};
use windows::Win32::Graphics::Direct3D12::ID3D12Fence;
use windows::core::Interface;

/// The caps feature of a sample in Direct3D 12 memory.
pub const MEMORY: &str = "memory:D3D12Memory";

/// `GST_MAP_D3D12`: a map that asks for the texture rather than its bytes.
const MAP_D3D12: u32 = gst::ffi::GST_MAP_FLAG_LAST << 1;

/// `GstD3D12Memory`'s public head: the memory, then the device it was made on.
#[repr(C)]
struct D3d12Memory {
    mem: gst::ffi::GstMemory,
    device: *mut c_void,
}

#[link(name = "gstd3d12-1.0")]
unsafe extern "C" {
    fn gst_is_d3d12_memory(mem: *mut gst::ffi::GstMemory) -> glib::ffi::gboolean;
    fn gst_d3d12_memory_get_resource_handle(mem: *mut gst::ffi::GstMemory) -> *mut c_void;
    fn gst_d3d12_memory_get_subresource_index(
        mem: *mut gst::ffi::GstMemory,
        plane: u32,
        index: *mut u32,
    ) -> glib::ffi::gboolean;
    fn gst_d3d12_memory_get_fence(
        mem: *mut gst::ffi::GstMemory,
        fence: *mut *mut c_void,
        value: *mut u64,
    ) -> glib::ffi::gboolean;
    fn gst_d3d12_memory_get_nt_handle(
        mem: *mut gst::ffi::GstMemory,
        handle: *mut *mut c_void,
    ) -> glib::ffi::gboolean;
    fn gst_d3d12_memory_sync(mem: *mut gst::ffi::GstMemory) -> glib::ffi::gboolean;
    fn gst_d3d12_device_new_for_adapter_luid(luid: i64) -> *mut gst::ffi::GstObject;
    fn gst_d3d12_device_get_device_handle(device: *mut c_void) -> *mut c_void;
    fn gst_d3d12_context_new(device: *mut gst::ffi::GstObject) -> *mut gst::ffi::GstContext;
}

/// What a frame keeps alive: the producer's buffer, which holds the textures and their handles,
/// and the fences they are signalled on.
struct Held {
    _buffer: gst::Buffer,
    _fences: Vec<ID3D12Fence>,
}

/// What [`context`] made, for the renderer device it was made for.
static MADE: Mutex<Option<(D3d12Device, Option<gst::Context>)>> = Mutex::new(None);

/// The `GstContext` that hands a pipeline GStreamer's device on the renderer's adapter, made
/// once per renderer device: `None` before there is a renderer, off Direct3D 12, or where
/// GStreamer makes no device there.
pub fn context() -> Option<gst::Context> {
    let here = crate::render::dmabuf::d3d12_here()?;
    let mut made = MADE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((device, context)) = made.as_ref()
        && *device == here
    {
        return context.clone();
    }
    let context = make(here);
    *made = Some((here, context.clone()));
    if context.is_some() {
        prefer_decoders_on(here.luid);
    }
    context
}

/// GStreamer's device for `here`'s adapter, as a context.
fn make(here: D3d12Device) -> Option<gst::Context> {
    gst::init().ok()?;
    // SAFETY: a plain constructor; it hands back a reference of our own, or null where the
    // adapter has no Direct3D 12 device.
    let device = unsafe { gst_d3d12_device_new_for_adapter_luid(here.luid) };
    if device.is_null() {
        log::warn!("d3d12: GStreamer made no device on the renderer's adapter");
        return None;
    }
    // SAFETY: the device is live, and owned here until the `from_glib_full` below.
    let handle = unsafe { gst_d3d12_device_get_device_handle(device.cast()) } as usize;
    // SAFETY: the device is live; the context takes a reference of its own.
    let raw = unsafe { gst_d3d12_context_new(device) };
    // SAFETY: the reference the constructor handed back, which nothing else holds.
    drop(unsafe { gst::Object::from_glib_full(device) });
    if raw.is_null() {
        return None;
    }
    if handle != here.device {
        log::info!(
            "d3d12: GStreamer's device on the renderer's adapter is another; frames cross as \
             shared handles"
        );
    }
    // SAFETY: a new context, whose one reference is handed over.
    Some(unsafe { gst::Context::from_glib_full(raw) })
}

/// Rank every Direct3D 12 decoder made for adapter `luid` above every other decoder, once.
fn prefer_decoders_on(luid: i64) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let decoders = gst::ElementFactory::factories_with_type(
            gst::ElementFactoryType::DECODER | gst::ElementFactoryType::MEDIA_VIDEO,
            gst::Rank::NONE,
        );
        for factory in decoders {
            if !factory.name().starts_with("d3d12") {
                continue;
            }
            let Ok(loaded) = factory.load() else {
                continue;
            };
            let on = glib::object::Class::<glib::Object>::from_type(loaded.element_type())
                .and_then(|class| class.find_property("adapter-luid"))
                .and_then(|spec| spec.default_value().get::<i64>().ok());
            if on == Some(luid) {
                loaded.set_rank(gst::Rank::from(gst::Rank::PRIMARY.into_glib() + 3));
            }
        }
    });
}

/// A sample in Direct3D 12 memory as a `Frame`: the texture on the renderer's device, or a
/// handle to it from another, its slice, its layout and the fence it is written by, with the
/// buffer held behind it — or the same buffer mapped as bytes, where the texture can be
/// neither sampled nor shared or is in a layout the renderer does not import. `None` where the
/// caps do not say Direct3D 12 memory.
pub fn frame(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Option<Result<Frame, String>> {
    let features = caps.features(0)?;
    if !features.contains(MEMORY) {
        return None;
    }
    Some(match texture(caps, buffer) {
        Ok(Some(frame)) => Ok(frame),
        Ok(None) => crate::video::clip::mapped(caps, buffer),
        Err(e) => Err(e),
    })
}

/// The textures behind a sample whose caps say Direct3D 12 memory, or `None` where the renderer
/// cannot sample them where they lie.
fn texture(caps: &gst::CapsRef, buffer: &gst::BufferRef) -> Result<Option<Frame>, String> {
    let info = gst_video::VideoInfo::from_caps(caps).map_err(|e| e.to_string())?;
    let layout = match info.format() {
        gst_video::VideoFormat::Nv12 => Layout::Nv12,
        gst_video::VideoFormat::Rgba => Layout::Rgba,
        gst_video::VideoFormat::Bgra => Layout::Bgra,
        _ => return Ok(None),
    };
    let Some(here) = crate::render::dmabuf::d3d12_here() else {
        return Ok(None);
    };
    // NV12 whole wants the renderer to sample NV12; NV12 in a texture per plane does not.
    let count = buffer.n_memory();
    let fits = match (layout, count) {
        (Layout::Nv12, 1) => here.nv12,
        (Layout::Nv12, 2) | (_, 1) => true,
        _ => false,
    };
    if !fits {
        return Ok(None);
    }
    let mut device = None;
    let mut textures = Vec::with_capacity(count);
    let mut fences = Vec::new();
    for i in 0..count {
        let Some(found) = plane(buffer.peek_memory(i).as_mut_ptr(), here, &mut fences)? else {
            return Ok(None);
        };
        if device.is_some_and(|d| d != found.0) {
            return Ok(None);
        }
        device = Some(found.0);
        textures.push(found.1);
    }
    let Some(device) = device else {
        return Ok(None);
    };
    Ok(Some(Frame {
        width: info.width(),
        height: info.height(),
        pixels: Pixels::D3d12(D3d12Texture {
            device,
            textures,
            layout,
            yuv: crate::video::clip::yuv_of(&info),
            keep: Arc::new(Held {
                _buffer: buffer.to_owned(),
                _fences: fences,
            }),
            refused: None,
        }),
    }))
}

/// One memory's texture, and the device it was made on: on the renderer's device with the
/// fence it is written by, pushed onto `fences`, which then owns it; on another with a handle
/// to it and its writes already waited for. `None` where it is not Direct3D 12 memory, or is on
/// another device with no handle to share it by.
fn plane(
    mem: *mut gst::ffi::GstMemory,
    here: D3d12Device,
    fences: &mut Vec<ID3D12Fence>,
) -> Result<Option<(usize, D3d12Plane)>, String> {
    // SAFETY: a memory of a buffer the caller holds, asked what it is.
    if unsafe { gst_is_d3d12_memory(mem) } == glib::ffi::GFALSE {
        return Ok(None);
    }
    // SAFETY: the memory is a `GstD3D12Memory`, whose public head is mirrored by
    // `D3d12Memory`, and its device lives as long as the memory does.
    let device =
        unsafe { gst_d3d12_device_get_device_handle((*mem.cast::<D3d12Memory>()).device) } as usize;
    let mut map = std::mem::MaybeUninit::<gst::ffi::GstMapInfo>::zeroed();
    // SAFETY: a read map of a Direct3D 12 memory, which uploads what the CPU wrote and hands
    // back the texture; it is unmapped at once, and the texture stays the memory's.
    let mapped = unsafe {
        gst::ffi::gst_memory_map(mem, map.as_mut_ptr(), gst::ffi::GST_MAP_READ | MAP_D3D12)
    };
    if mapped == glib::ffi::GFALSE {
        return Err("the Direct3D 12 memory could not be mapped".to_owned());
    }
    // SAFETY: the map just made, and nothing read through it.
    unsafe { gst::ffi::gst_memory_unmap(mem, map.as_mut_ptr()) };
    let shared = if device == here.device {
        0
    } else {
        let mut handle = std::ptr::null_mut();
        // SAFETY: a Direct3D 12 memory and a place for its handle, which the memory keeps and
        // closes; null where its heap is not shared.
        let made = unsafe { gst_d3d12_memory_get_nt_handle(mem, &raw mut handle) };
        if made == glib::ffi::GFALSE || handle.is_null() {
            return Ok(None);
        }
        // SAFETY: a Direct3D 12 memory; this waits for its fence on the CPU, on GStreamer's
        // streaming thread.
        unsafe { gst_d3d12_memory_sync(mem) };
        handle as usize
    };
    // SAFETY: a Direct3D 12 memory, whose texture it owns for as long as it lives.
    let resource = unsafe { gst_d3d12_memory_get_resource_handle(mem) } as usize;
    let mut slice = 0;
    // SAFETY: a Direct3D 12 memory and a place for the index of its first plane, which with one
    // level is the texture's array slice.
    if unsafe { gst_d3d12_memory_get_subresource_index(mem, 0, &raw mut slice) }
        == glib::ffi::GFALSE
    {
        return Err("the Direct3D 12 memory has no first plane".to_owned());
    }
    let (mut fence, mut fence_value) = (std::ptr::null_mut(), 0);
    // SAFETY: a Direct3D 12 memory and places for its fence, which comes back with a reference
    // of its own, and the fence's value.
    let fenced = shared == 0
        && unsafe { gst_d3d12_memory_get_fence(mem, &raw mut fence, &raw mut fence_value) }
            != glib::ffi::GFALSE
        && !fence.is_null();
    let fence = if fenced {
        // SAFETY: the reference the call handed over, which `fences` then owns.
        let owned = unsafe { ID3D12Fence::from_raw(fence) };
        let raw = owned.as_raw() as usize;
        fences.push(owned);
        raw
    } else {
        0
    };
    Ok(Some((
        device,
        D3d12Plane {
            resource,
            shared,
            slice,
            fence,
            fence_value: if fence == 0 { 0 } else { fence_value },
        },
    )))
}

/// Hand `pipeline` GStreamer's device on the renderer's adapter, through the context
/// [`context`] made: every Direct3D 12 element in it, and every one a `decodebin` adds later,
/// takes its device from it.
pub fn hand_device(pipeline: &gst::Pipeline) {
    if let Some(context) = context() {
        pipeline.set_context(&context);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample in memory is not this module's.
    #[test]
    fn a_sample_in_memory_is_not_a_texture() {
        gst::init().expect("gstreamer");
        let caps: gst::Caps = "video/x-raw,format=NV12,width=4,height=4,framerate=30/1"
            .parse()
            .expect("caps");
        let buffer = gst::Buffer::with_size(24).expect("a buffer");
        assert!(frame(&caps, &buffer).is_none());
    }
}
