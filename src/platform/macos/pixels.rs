// SPDX-License-Identifier: AGPL-3.0-or-later

//! A `CVPixelBuffer` as a frame: its `IOSurface`, which the renderer samples in place, beside
//! the buffer locked for reading and held where it lies, the counterpart of a mapped GStreamer
//! buffer, and unlocked when the last frame holding it drops. And the `IOSurface` behind a
//! buffer VideoToolbox decoded into.
//!
//! **A decoded buffer's `CVPixelBuffer` is reached through a mirrored struct.** applemedia
//! attaches a `GstCoreVideoMeta` — `{ GstMeta meta; CVBufferRef cvbuf; CVPixelBufferRef
//! pixbuf; }` in its `corevideobuffer.h`, which Homebrew does not install — so the layout is
//! mirrored here. Before the pointer is read the meta is found by its API's name and its
//! registered size is checked against the mirror's, and before it is used its CoreFoundation
//! type is checked against `CVPixelBuffer`'s; a buffer that fails either goes as bytes.
//!
//! **The `unsafe` here** is that read, CoreVideo's lock and unlock, the plane slices the lock
//! hands out as a pointer and a length, and `Send` and `Sync` on the holder, which a
//! CoreFoundation type is not by declaration. Each block says why it holds.

use crate::nodes::{Frame, IoSurface, Layout, Mapped, Pixels, Planes, Yuv};
use gstreamer as gst;
use objc2_core_foundation::{CFGetTypeID, CFRetained, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetIOSurface, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetPlaneCount, CVPixelBufferGetTypeID, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use std::ffi::c_void;
use std::sync::Arc;

/// `GstCoreVideoMeta`, as applemedia's `corevideobuffer.h` declares it.
#[repr(C)]
struct CoreVideoMeta {
    meta: gst::ffi::GstMeta,
    cvbuf: *mut c_void,
    pixbuf: *mut c_void,
}

/// The name applemedia registers its meta's API under.
const CORE_VIDEO_META: &str = "GstCoreVideoMetaAPI";

/// The `IOSurface` behind `buffer`, as an integer valid for as long as the buffer is, or
/// `None` for a buffer with no CoreVideo meta — a software decoder's — or with one that is not
/// the shape mirrored here, or a pixel buffer with no surface.
pub(super) fn surface_of(buffer: &gst::BufferRef) -> Option<usize> {
    let meta = buffer
        .iter_meta::<gst::Meta>()
        .find(|m| m.api().name() == CORE_VIDEO_META)?;
    let raw = meta.as_ptr().cast::<CoreVideoMeta>();
    // SAFETY: every meta points at the info GStreamer registered it with, which lives for the
    // process; its size is the size of the struct the meta is.
    let size = unsafe { (*(*raw).meta.info).size };
    if size != std::mem::size_of::<CoreVideoMeta>() {
        return None;
    }
    // SAFETY: the meta is at least the mirror's size, so its last field is inside it.
    let pixbuf = unsafe { (*raw).pixbuf };
    if pixbuf.is_null() {
        return None;
    }
    // SAFETY: a non-null pointer the meta holds a reference to, read as CoreFoundation's base
    // type only to ask what type it is.
    let object = unsafe { &*pixbuf.cast::<CFType>() };
    if CFGetTypeID(Some(object)) != CVPixelBufferGetTypeID() {
        return None;
    }
    // SAFETY: a `CVPixelBuffer`, checked above, which the meta keeps alive with the buffer.
    let pixbuf = unsafe { &*pixbuf.cast::<CVPixelBuffer>() };
    let surface = CVPixelBufferGetIOSurface(Some(pixbuf))?;
    // The pixel buffer holds the surface, so the address outlives this reference to it.
    Some(CFRetained::as_ptr(&surface).as_ptr() as usize)
}

/// A pixel buffer locked read-only, and its one plane's bytes while it is.
struct Locked {
    buffer: CFRetained<CVPixelBuffer>,
    base: *const u8,
    len: usize,
}

// SAFETY: a `CVPixelBuffer` is reference counted atomically and may be released on any thread;
// the lock is read-only, so the bytes it exposes are never written through this, and the
// unlock in `drop` runs once, on whichever thread lets go of the last frame.
unsafe impl Send for Locked {}
// SAFETY: as above; every access through `&Locked` reads.
unsafe impl Sync for Locked {}

impl Planes for Locked {
    fn plane(&self, i: usize) -> &[u8] {
        if i != 0 || self.base.is_null() {
            return &[];
        }
        // SAFETY: the buffer stays locked while `self` lives, and `len` bytes from `base` are
        // its plane as CoreVideo described it: the rows times the bytes between them.
        unsafe { std::slice::from_raw_parts(self.base, self.len) }
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        // SAFETY: locked read-only in `lock`, and unlocked here once, with the same flag.
        unsafe { CVPixelBufferUnlockBaseAddress(&self.buffer, CVPixelBufferLockFlags::ReadOnly) };
    }
}

/// `buffer` locked read-only, with its one plane's base address and length.
fn lock(buffer: CFRetained<CVPixelBuffer>) -> Result<Locked, String> {
    if CVPixelBufferGetPlaneCount(&buffer) != 0 {
        return Err("a planar pixel buffer is not one this reads".to_string());
    }
    // SAFETY: a valid pixel buffer, locked read-only; `Locked` unlocks it once it drops.
    let status = unsafe { CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) };
    if status != kCVReturnSuccess {
        return Err(format!("the pixel buffer could not be locked ({status})"));
    }
    let base = CVPixelBufferGetBaseAddress(&buffer)
        .cast::<u8>()
        .cast_const();
    let len = CVPixelBufferGetBytesPerRow(&buffer) * CVPixelBufferGetHeight(&buffer);
    Ok(Locked { buffer, base, len })
}

/// A screen's frame: a `32BGRA` pixel buffer's `IOSurface`, which the renderer samples where
/// it lies, beside the same buffer locked and held at its own stride, which it uploads where
/// it cannot — or those bytes alone, for a buffer with no surface. Its fourth byte is not
/// taken for alpha, since ScreenCaptureKit leaves what is outside a window clear, so either
/// way the frame goes through the renderer's conversion pass, which writes alpha one.
pub(super) fn frame(buffer: CFRetained<CVPixelBuffer>) -> Result<Frame, String> {
    let format = CVPixelBufferGetPixelFormatType(&buffer);
    if format != kCVPixelFormatType_32BGRA {
        return Err(format!(
            "a '{}' frame is not one this reads",
            String::from_utf8_lossy(&format.to_be_bytes())
        ));
    }
    let (width, height) = (
        CVPixelBufferGetWidth(&buffer) as u32,
        CVPixelBufferGetHeight(&buffer) as u32,
    );
    let stride = CVPixelBufferGetBytesPerRow(&buffer) as u32;
    // The pixel buffer holds the surface, and the lock below holds the pixel buffer.
    let surface =
        CVPixelBufferGetIOSurface(Some(&buffer)).map(|s| CFRetained::as_ptr(&s).as_ptr() as usize);
    let mapped = Mapped {
        layout: Layout::Bgrx,
        strides: [stride, 0, 0],
        yuv: Yuv::default(),
        straight_alpha: false,
        data: Arc::new(lock(buffer)?),
    };
    Ok(Frame {
        width,
        height,
        pixels: match surface {
            Some(surface) => Pixels::IoSurface(IoSurface {
                surface,
                mapped,
                redrawn: None,
            }),
            None => Pixels::Mapped(mapped),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_foundation::{CFDictionary, CFString};
    use objc2_core_video::{CVPixelBufferCreate, kCVPixelBufferIOSurfacePropertiesKey};
    use std::ptr::NonNull;

    /// A `32BGRA` pixel buffer backed by an `IOSurface`, as ScreenCaptureKit hands one over,
    /// every pixel `at(x, y)`.
    fn bgra(
        width: usize,
        height: usize,
        at: impl Fn(usize, usize) -> [u8; 4],
    ) -> CFRetained<CVPixelBuffer> {
        let empty = CFDictionary::<CFString, CFString>::empty();
        // SAFETY: a CoreVideo key, a static string valid for the life of the process.
        let key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
        let attributes = CFDictionary::<CFString, CFDictionary<CFString, CFString>>::from_slices(
            &[key],
            &[&empty],
        );
        let mut out: *mut CVPixelBuffer = std::ptr::null_mut();
        // SAFETY: the attributes are a dictionary of CoreVideo's own key, and `out` is a
        // pointer to a local that the call writes a +1 reference into.
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
        // SAFETY: locked for writing, written within its rows, and unlocked below.
        unsafe {
            assert_eq!(
                CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::empty()),
                kCVReturnSuccess
            );
            let stride = CVPixelBufferGetBytesPerRow(&buffer);
            let base = CVPixelBufferGetBaseAddress(&buffer).cast::<u8>();
            for y in 0..height {
                for x in 0..width {
                    let texel = std::slice::from_raw_parts_mut(base.add(y * stride + x * 4), 4);
                    texel.copy_from_slice(&at(x, y));
                }
            }
            CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::empty());
        }
        buffer
    }

    /// **A pixel buffer is a frame at its own stride, and its surface**: the size, the
    /// layout, the stride CoreVideo padded its rows to, every pixel where it was written, and
    /// the `IOSurface` behind them.
    #[test]
    fn a_pixel_buffer_is_its_surface_and_its_bytes_at_their_own_stride() {
        let at = |x: usize, y: usize| [x as u8, y as u8, (x + y) as u8, 0];
        let buffer = bgra(190, 96, at);
        let stride = CVPixelBufferGetBytesPerRow(&buffer);
        let behind = CVPixelBufferGetIOSurface(Some(&buffer)).expect("backed by a surface");
        let frame = frame(buffer).expect("a frame");
        assert_eq!((frame.width, frame.height), (190, 96));
        let Pixels::IoSurface(IoSurface {
            surface, mapped: m, ..
        }) = &frame.pixels
        else {
            panic!("a surface, not {:?}", frame.pixels);
        };
        assert_eq!(*surface, CFRetained::as_ptr(&behind).as_ptr() as usize);
        assert_eq!(m.layout, Layout::Bgrx);
        assert_eq!(m.strides[0] as usize, stride);
        assert!(stride >= 190 * 4, "a row is at least its pixels");
        let plane = m.plane(0, 190, 96).expect("the whole plane");
        for (x, y) in [(0, 0), (189, 0), (7, 50), (189, 95)] {
            let i = y * stride + x * 4;
            assert_eq!(plane[i..i + 4], at(x, y), "({x}, {y})");
        }
    }
}
