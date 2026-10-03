// SPDX-License-Identifier: AGPL-3.0-or-later

//! A decoder's or a screen cast's DMA-BUF as a texture, with no copy, and each frame back to
//! its producer once nothing on the GPU can still read it.
//!
//! **The import** is `wgpu_hal`'s Vulkan `texture_from_dmabuf_fd`, wrapped by
//! `Device::create_texture_from_hal`, behind `Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF`, which
//! [`super::gpu::WANTED_FEATURES`] asks for. Vulkan takes ownership of the fd it is handed, and
//! the fd is the producer's — GStreamer's — so it is `dup`ed first; the producer's own stays
//! open and a frame can be imported again. `proposals/wgpu.md`, 1.11.
//!
//! **What is importable is asked of Vulkan**, per fourcc, with
//! `VkDrmFormatModifierPropertiesListEXT` and then `vkGetPhysicalDeviceImageFormatProperties2`
//! for exactly the image the import makes — sampled and copied from, DMA-BUF memory, that
//! modifier — so a pair is offered only if the import would take it. The fourccs are
//! the four byte orders of 8-bit RGB a compositor shares a screen in. A modifier is offered
//! only where Vulkan says it is **one plane**, since the import passes one plane's layout; that
//! also keeps out every compressed tiling, whose metadata is a plane of its own. And
//! [`Imports::import`] checks each frame against the answer before calling into the driver:
//! an image made with a modifier the device does not support is not an error Vulkan reports
//! but invalid usage.
//!
//! **The first barrier on an imported image is from `UNDEFINED`**: wgpu cannot express an
//! acquire from `VK_QUEUE_FAMILY_FOREIGN_EXT`. A transition from `UNDEFINED` may discard what
//! an image holds, and in the drivers the only memory it touches is an auxiliary surface — a
//! compression or fast-clear plane — which a one-plane modifier does not have. So the
//! one-plane rule is also what keeps the producer's pixels through the first barrier.
//!
//! **An `x` byte is padding, and wgpu has no alpha swizzle**: `XR24` and `XB24` import as
//! `Bgra8Unorm` and `Rgba8Unorm` whose alpha is whatever the producer left there.
//! [`alpha_is_padding`] says which, so the frame can go through the conversion pass that writes
//! alpha 1, as an `x` layout of bytes does (`proposals/wgpu.md`, 1.10).
//!
//! **The return.** A frame goes back to its producer when its last `Arc` drops, and that waits
//! for two things: the renderer has let go of it and the submission after has finished, and no
//! viewer still holds a `Published` naming its texture. A viewer's blit may be submitted after
//! the synth's next submission, so a serial alone does not cover it. So a `Published` carries a
//! [`Held`] per imported texture it names, from [`Imports::claim`]; the renderer's own claim
//! ends at [`Imports::let_go`], a viewer's when it drops its `Published` — which it does after
//! the submit that carries its reads. Whichever is last puts the frame in the inbox, from any
//! thread; [`Imports::submitted`] stamps what the inbox holds with the next submission's serial,
//! which on the one queue finishes after everything submitted before it, the viewer's blit
//! included; and [`Imports::sweep`] lets go of every frame whose serial has finished. Lane 5a's
//! `sources.rs` calls these, [`Imports::import`] and [`Imports::import_surface`]; nothing else
//! does.
//!
//! **macOS has no DMA-BUF; its frames are `IOSurface`s**, and the same file imports them on
//! Metal. Each plane is made a Metal texture over the surface's own memory with
//! `newTextureWithDescriptor:iosurface:plane:` — NV12's luma as `R8Unorm` and its chroma as
//! `RG8Unorm`, BGR as `Bgra8Unorm` — and handed to wgpu with `texture_from_raw` and
//! `create_texture_from_hal`, with no drop callback: the frame holds the surface, and the return
//! rule above holds the frame. What is checked before Metal is called is the plane count, each
//! plane's size and the surface's pixel format. The `vulkan` module's non-Linux twin refuses
//! every DMA-BUF, and the `metal` module's twin off macOS every surface. A surface's pixel
//! format of zero is BGRA, as a Syphon server built before October 2025 leaves it.
//!
//! **A surface is also drawn into**: [`surface_target`] makes a Syphon server's shared BGRA
//! surface a texture a blit renders into, the same way and with the same rule — the caller,
//! not the texture, holds the surface.
//!
//! **Windows has neither; its frames are Direct3D 12 textures, and the same file imports them
//! on Direct3D 12.** GStreamer's Direct3D 12 decoders and uploads make their textures on a
//! `GstD3D12Device`, which `platform::windows::video` asks for on the renderer's adapter:
//! `D3D12CreateDevice` hands one process one device per adapter, so it is the renderer's own
//! `ID3D12Device`; where it is not — vkd3d, under Wine, makes a device per call — the texture
//! comes with an NT handle and is opened on the renderer's device through it, its writes
//! already waited for, and a texture of another device with no handle is refused. The
//! texture is wrapped whole with `wgpu_hal::dx12::Device::texture_from_raw` — NV12 as wgpu's
//! `NV12`, whose two planes are viewed as `R8Unorm` and `RG8Unorm`, as the macOS planes are, or
//! RGBA or BGRA; or, where the producer's device holds no NV12 texture and gives each plane a
//! texture of its own, each of those wrapped as `R8Unorm` and `RG8Unorm` — and its array slice
//! is the views' layer, since a decoder may hand out a slice of a texture array. **The renderer's queue waits on the producer's fence**: the wait is
//! staged with `add_wait_fence` for the next submission on the one queue, so nothing samples
//! the texture before the decoder's or the upload's own queue has written it, and nothing on
//! the CPU waits. The conversion pass always draws it into a texture of the source's own, and
//! a transition after the pass hands the texture back in the common state GStreamer's queues
//! expect of it. The return rule is the one above: the frame holds the producer's buffer.
//!
//! **This is the wgpu renderer's one `unsafe`**: a `dup` of a borrowed fd, the hal import, the
//! wrap, and two Vulkan queries through the instance's own function table; on macOS the
//! `IOSurfaceRef` rebuilt from its integer, the hal device, the texture made from raw and its
//! wrap; and on Windows the resource and the fence rebuilt from their integers, their COM
//! calls, the hal device and queue, and the texture made from raw and its wrap. Each block says
//! why it holds. The queries' structures are mirrored here from `vulkan_core.h` because `ash`,
//! which defines them, is wgpu-hal's dependency and not this crate's.

use crate::nodes::{D3d12Texture, DmaBuf, Frame, IoSurface};
use std::sync::{Arc, Mutex, PoisonError};

/// `DRM_FORMAT_XRGB8888`: B, G, R and a padding byte in memory.
pub const XR24: u32 = u32::from_le_bytes(*b"XR24");
/// `DRM_FORMAT_ARGB8888`: B, G, R, A in memory.
pub const AR24: u32 = u32::from_le_bytes(*b"AR24");
/// `DRM_FORMAT_XBGR8888`: R, G, B and a padding byte in memory.
pub const XB24: u32 = u32::from_le_bytes(*b"XB24");
/// `DRM_FORMAT_ABGR8888`: R, G, B, A in memory — RGBA8.
pub const AB24: u32 = u32::from_le_bytes(*b"AB24");

/// The fourccs an import takes, in the order [`importable`] lists them.
pub const FOURCCS: [u32; 4] = [XR24, AR24, XB24, AB24];

/// The texture format a fourcc imports as, sampled as RGBA.
fn format_of(fourcc: u32) -> Option<wgpu::TextureFormat> {
    match fourcc {
        XR24 | AR24 => Some(wgpu::TextureFormat::Bgra8Unorm),
        XB24 | AB24 => Some(wgpu::TextureFormat::Rgba8Unorm),
        _ => None,
    }
}

/// Whether the fourcc's fourth byte is padding, so a sampler reads an undefined alpha.
pub fn alpha_is_padding(fourcc: u32) -> bool {
    matches!(fourcc, XR24 | XB24)
}

/// One `(fourcc, modifier)` the device imports, and the largest image it takes in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Importable {
    fourcc: u32,
    modifier: u64,
    max: (u32, u32),
}

/// The device the renderer draws on, for what asks whether a frame can be imported before it
/// has a device of its own to ask: a video pipeline choosing its caps. Set by
/// `Renderer::new`.
static SERVED: Mutex<Option<wgpu::Device>> = Mutex::new(None);

/// Make `device` the one [`imports`] and [`importable_here`] ask.
pub fn serve(device: &wgpu::Device) {
    *SERVED.lock().unwrap_or_else(PoisonError::into_inner) = Some(device.clone());
}

/// Whether the renderer's device imports frames without a copy: false before there is a
/// renderer.
pub fn imports() -> bool {
    SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(supported)
}

/// [`importable`] on the renderer's device: empty before there is a renderer.
pub fn importable_here() -> Vec<(u32, u64)> {
    let device = SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    device.as_ref().map(importable).unwrap_or_default()
}

/// Whether the device the renderer draws on imports frames without a copy: DMA-BUFs on a
/// Vulkan device opened with the import, `IOSurface`s on a Metal one, Direct3D 12 textures on a
/// Direct3D 12 one. Where it does not, every import is refused and [`importable`] is empty.
pub fn supported(device: &wgpu::Device) -> bool {
    dmabufs(device) || metal::supported(device) || d3d12::supported(device)
}

/// The renderer's Direct3D 12 device, as a producer is made on it and checked against it:
/// its `ID3D12Device` as an integer, its adapter's LUID as GStreamer writes one, and whether it
/// samples NV12. `None` before there is a renderer, and off Direct3D 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct D3d12Device {
    pub device: usize,
    pub luid: i64,
    pub nv12: bool,
}

/// [`D3d12Device`] for the device the renderer draws on.
pub fn d3d12_here() -> Option<D3d12Device> {
    let device = SERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    d3d12::device_of(&device)
}

/// One plane of an imported texture: the texture, the view a pass samples the plane through,
/// and the plane's size in texels.
pub struct ImportedPlane {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

/// A texture drawn into `surface`, an 8-bit BGRA `IOSurface` of `width` by `height` another
/// process reads — a Syphon server's shared surface — or why not. Sampled, drawn into and
/// copied both ways, over the surface's own memory, with nothing cleared: the caller holds the
/// surface for as long as the texture. Refused off macOS.
pub fn surface_target(
    device: &wgpu::Device,
    surface: usize,
    width: u32,
    height: u32,
) -> Result<wgpu::Texture, String> {
    metal::target(device, surface, width, height)
}

/// Whether the device was opened with the DMA-BUF import. Where it was not, every DMA-BUF is
/// refused and [`importable`] is empty.
fn dmabufs(device: &wgpu::Device) -> bool {
    cfg!(target_os = "linux")
        && device
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
}

/// Every `(fourcc, modifier)` of 8-bit RGB the device imports as a sampled texture, in one
/// plane: what a producer that allocates its own buffers — a compositor sharing a screen — is
/// told it may hand over. Empty where the device has no import.
pub fn importable(device: &wgpu::Device) -> Vec<(u32, u64)> {
    vulkan::importable(device)
        .into_iter()
        .map(|i| (i.fourcc, i.modifier))
        .collect()
}

/// Why `buf` cannot be imported at `width` by `height` on a device that imports `formats`, or
/// nothing: every check made before the driver is called.
fn refusal(formats: &[Importable], buf: &DmaBuf, width: u32, height: u32) -> Option<String> {
    let name = fourcc_name(buf.fourcc);
    if format_of(buf.fourcc).is_none() {
        return Some(format!("{name} is not one of the RGB formats this imports"));
    }
    if width == 0 || height == 0 {
        return Some(format!("a {width}×{height} frame has no pixels"));
    }
    if buf.fd < 0 {
        return Some(format!("fd {} is not a file descriptor", buf.fd));
    }
    if u64::from(buf.stride) < u64::from(width) * 4 {
        return Some(format!(
            "a stride of {} bytes is short of {width} pixels",
            buf.stride
        ));
    }
    let Some(found) = formats
        .iter()
        .find(|i| i.fourcc == buf.fourcc && i.modifier == buf.modifier)
    else {
        return Some(format!(
            "{name} with modifier {:#x} is not importable here",
            buf.modifier
        ));
    };
    if width > found.max.0 || height > found.max.1 {
        return Some(format!(
            "{width}×{height} is past the {}×{} {name} imports at",
            found.max.0, found.max.1
        ));
    }
    None
}

/// A fourcc as its four letters.
fn fourcc_name(fourcc: u32) -> String {
    fourcc
        .to_le_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_graphic() {
                char::from(*b)
            } else {
                '?'
            }
        })
        .collect()
}

/// Imported frames on their way back to their producers.
#[derive(Default)]
pub struct Imports {
    /// Let go of by the renderer, or by the last viewer holding it, since the last submission.
    inbox: Arc<Mutex<Vec<Arc<Frame>>>>,
    /// Each batch behind the serial of the submission after which nothing samples it.
    returning: Vec<(u64, Vec<Arc<Frame>>)>,
    /// The claim on each frame the renderer has imported and not let go of, which every
    /// `Published` naming its texture shares.
    claims: Vec<Held>,
    /// What the device imports, asked on the first import.
    formats: Option<Vec<Importable>>,
}

/// A claim on an imported frame: while any clone lives the frame does not go back, and when
/// the last drops the frame waits in its [`Imports`]' inbox for the next submission to be made
/// and to finish. A `Published` holds one for each imported texture it names.
#[derive(Clone)]
pub struct Held(Arc<Claim>);

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Held")
            .field(&(self.0.frame.width, self.0.frame.height))
            .finish()
    }
}

struct Claim {
    frame: Arc<Frame>,
    inbox: Arc<Mutex<Vec<Arc<Frame>>>>,
}

impl Held {
    /// The frame claimed.
    pub fn frame(&self) -> &Arc<Frame> {
        &self.0.frame
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::clone(&self.frame));
    }
}

impl Imports {
    /// The frame `buf` describes as a texture of `width` by `height`, or why not.
    ///
    /// The texture samples the producer's memory directly, rows as they lie in it; the caller
    /// keeps the frame — and so `buf.keep` — until [`Imports::let_go`], and treats the alpha of
    /// a fourcc [`alpha_is_padding`] names as undefined.
    pub fn import(
        &mut self,
        device: &wgpu::Device,
        buf: &DmaBuf,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, String> {
        if !dmabufs(device) {
            return Err("this device was opened without DMA-BUF import".to_owned());
        }
        let formats = self
            .formats
            .get_or_insert_with(|| vulkan::importable(device));
        if let Some(why) = refusal(formats, buf, width, height) {
            return Err(why);
        }
        vulkan::import(device, buf, width, height)
    }

    /// The frame in `frame`'s `IOSurface` as a texture per plane, of a frame of `width` by
    /// `height`, or why not: NV12's luma and chroma, or one BGR plane.
    ///
    /// Each texture samples the surface's memory directly; the caller keeps the frame — and so
    /// `frame.mapped.data`, which holds the surface — until [`Imports::let_go`], and treats a
    /// BGR plane's fourth byte as padding where the layout says so.
    pub fn import_surface(
        &mut self,
        device: &wgpu::Device,
        frame: &IoSurface,
        width: u32,
        height: u32,
    ) -> Result<Vec<wgpu::Texture>, String> {
        metal::import(device, frame, width, height)
    }

    /// The frame in `frame`'s Direct3D 12 textures as a plane per view, of a frame of `width` by
    /// `height`, or why not: NV12's luma and chroma, or one RGB plane. Each texture is wrapped
    /// whole and its planes view it, and `queue`'s next submission waits on the producer's
    /// fences.
    ///
    /// The caller keeps the frame — and so `frame.keep`, which holds the textures — until
    /// [`Imports::let_go`], samples it only after the submission the wait is staged for, and
    /// hands the texture back in the common state once it has sampled it.
    pub fn import_d3d12(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &D3d12Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<ImportedPlane>, String> {
        d3d12::import(device, queue, frame, width, height)
    }

    /// The claim a `Published` naming `frame`'s texture holds: one per frame, shared by every
    /// `Published` until the renderer lets go of the frame.
    pub fn claim(&mut self, frame: &Arc<Frame>) -> Held {
        if let Some(held) = self.claims.iter().find(|h| Arc::ptr_eq(h.frame(), frame)) {
            return held.clone();
        }
        let held = Held(Arc::new(Claim {
            frame: Arc::clone(frame),
            inbox: Arc::clone(&self.inbox),
        }));
        self.claims.push(held.clone());
        held
    }

    /// A frame no texture the renderer draws with samples any longer: it goes back once the
    /// next submission has finished and no viewer holds a claim on it.
    pub fn let_go(&mut self, frame: Arc<Frame>) {
        self.claims.retain(|h| !Arc::ptr_eq(h.frame(), &frame));
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(frame);
    }

    /// The submission numbered `serial` has been made: everything in the inbox before it goes
    /// back once it finishes.
    pub fn submitted(&mut self, serial: u64) {
        let batch = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(PoisonError::into_inner));
        if !batch.is_empty() {
            self.returning.push((serial, batch));
        }
    }

    /// Whether a frame waits in the inbox for a submission to stamp it: one the last viewer
    /// holding it let go of since the renderer's last submission.
    pub fn waiting(&self) -> bool {
        !self
            .inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    }

    /// Let every frame whose submission has finished go back to its producer.
    pub fn sweep(&mut self, completed: u64) {
        self.returning.retain(|(serial, _)| *serial > completed);
    }
}

#[cfg(target_os = "linux")]
mod vulkan {
    //! The import and the query, on wgpu-hal's Vulkan device.

    use super::{Importable, format_of};
    use crate::nodes::DmaBuf;
    use std::ffi::c_void;
    use std::os::fd::BorrowedFd;
    use wgpu::hal::api::Vulkan;

    /// The usages an imported texture carries: sampled by a draw, copied from by a readback.
    const USAGE: wgpu::TextureUsages =
        wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_SRC);

    /// `VK_API_VERSION_1_1`: the two queries are core there.
    const API_1_1: u32 = (1 << 22) | (1 << 12);

    // `VkStructureType`s.
    const FORMAT_PROPERTIES_2: i32 = 1_000_059_002;
    const IMAGE_FORMAT_PROPERTIES_2: i32 = 1_000_059_003;
    const PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2: i32 = 1_000_059_004;
    const PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO: i32 = 1_000_071_000;
    const EXTERNAL_IMAGE_FORMAT_PROPERTIES: i32 = 1_000_071_001;
    const DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT: i32 = 1_000_158_000;
    const PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT: i32 = 1_000_158_002;

    /// `VkFormat`s, as wgpu-hal maps `Rgba8Unorm` and `Bgra8Unorm`.
    const R8G8B8A8_UNORM: i32 = 37;
    const B8G8R8A8_UNORM: i32 = 44;
    const IMAGE_TYPE_2D: i32 = 1;
    const TILING_DRM_FORMAT_MODIFIER_EXT: i32 = 1_000_158_000;
    const SHARING_MODE_EXCLUSIVE: i32 = 0;
    /// `VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_SAMPLED_BIT`, as wgpu-hal maps
    /// [`USAGE`].
    const IMAGE_USAGE: u32 = 0x1 | 0x4;
    /// `VK_FORMAT_FEATURE_SAMPLED_IMAGE_BIT | VK_FORMAT_FEATURE_TRANSFER_SRC_BIT`.
    const TILING_FEATURES: u32 = 0x1 | 0x4000;
    const HANDLE_TYPE_DMA_BUF_EXT: u32 = 0x200;
    const EXTERNAL_MEMORY_FEATURE_IMPORTABLE: u32 = 0x4;
    const SUCCESS: i32 = 0;

    // The structures, as `vulkan_core.h` declares them.

    #[repr(C)]
    struct FormatProperties2 {
        s_type: i32,
        p_next: *mut c_void,
        linear_tiling_features: u32,
        optimal_tiling_features: u32,
        buffer_features: u32,
    }

    #[repr(C)]
    struct DrmFormatModifierPropertiesList {
        s_type: i32,
        p_next: *mut c_void,
        count: u32,
        properties: *mut DrmFormatModifierProperties,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct DrmFormatModifierProperties {
        modifier: u64,
        plane_count: u32,
        tiling_features: u32,
    }

    #[repr(C)]
    struct PhysicalDeviceImageFormatInfo2 {
        s_type: i32,
        p_next: *const c_void,
        format: i32,
        ty: i32,
        tiling: i32,
        usage: u32,
        flags: u32,
    }

    #[repr(C)]
    struct PhysicalDeviceExternalImageFormatInfo {
        s_type: i32,
        p_next: *const c_void,
        handle_type: u32,
    }

    #[repr(C)]
    struct PhysicalDeviceImageDrmFormatModifierInfo {
        s_type: i32,
        p_next: *const c_void,
        modifier: u64,
        sharing_mode: i32,
        queue_family_index_count: u32,
        queue_family_indices: *const u32,
    }

    #[repr(C)]
    struct ImageFormatProperties2 {
        s_type: i32,
        p_next: *mut c_void,
        max_extent: [u32; 3],
        max_mip_levels: u32,
        max_array_layers: u32,
        sample_counts: u32,
        max_resource_size: u64,
    }

    #[repr(C)]
    struct ExternalImageFormatProperties {
        s_type: i32,
        p_next: *mut c_void,
        external_memory_features: u32,
        export_from_imported_handle_types: u32,
        compatible_handle_types: u32,
    }

    /// `vkGetPhysicalDeviceFormatProperties2`.
    type FormatPropertiesFn = unsafe extern "system" fn(*mut c_void, i32, *mut FormatProperties2);
    /// `vkGetPhysicalDeviceImageFormatProperties2`.
    type ImageFormatPropertiesFn = unsafe extern "system" fn(
        *mut c_void,
        *const PhysicalDeviceImageFormatInfo2,
        *mut ImageFormatProperties2,
    ) -> i32;

    fn vk_format(format: wgpu::TextureFormat) -> i32 {
        match format {
            wgpu::TextureFormat::Bgra8Unorm => B8G8R8A8_UNORM,
            _ => R8G8B8A8_UNORM,
        }
    }

    /// Every one-plane `(fourcc, modifier)` the device imports as a texture of [`USAGE`], with
    /// the largest extent Vulkan allows it.
    pub(super) fn importable(device: &wgpu::Device) -> Vec<Importable> {
        if !super::dmabufs(device) {
            return Vec::new();
        }
        // SAFETY: the guard is only read through — a physical device handle and the
        // instance's function table — and nothing it names is destroyed here.
        let Some(hal) = (unsafe { device.as_hal::<Vulkan>() }) else {
            return Vec::new();
        };
        let instance = hal.shared_instance();
        if instance.instance_api_version() < API_1_1 {
            return Vec::new();
        }
        let table = instance.raw_instance().fp_v1_1();
        let format_properties = table.get_physical_device_format_properties2 as *const ();
        let image_format_properties =
            table.get_physical_device_image_format_properties2 as *const ();
        // SAFETY: both are the entry points of a Vulkan 1.1 instance, which the check above
        // established, with these signatures: every parameter here has the layout of the one
        // `ash` declares — a dispatchable handle is a pointer, an enum an `i32`, and each
        // structure is mirrored field for field from `vulkan_core.h`.
        let (format_properties, image_format_properties) = unsafe {
            (
                std::mem::transmute::<*const (), FormatPropertiesFn>(format_properties),
                std::mem::transmute::<*const (), ImageFormatPropertiesFn>(image_format_properties),
            )
        };
        // SAFETY: a `VkPhysicalDevice` is a pointer-sized handle, which `ash` wraps
        // transparently.
        let physical: *mut c_void = unsafe { std::mem::transmute(hal.raw_physical_device()) };

        let mut found = Vec::new();
        for fourcc in super::FOURCCS {
            let Some(format) = format_of(fourcc).map(vk_format) else {
                continue;
            };
            let modifiers = modifiers(format_properties, physical, format);
            for m in modifiers {
                if m.plane_count != 1 || m.tiling_features & TILING_FEATURES != TILING_FEATURES {
                    continue;
                }
                if let Some(max) = extent(image_format_properties, physical, format, m.modifier) {
                    found.push(Importable {
                        fourcc,
                        modifier: m.modifier,
                        max,
                    });
                }
            }
        }
        found
    }

    /// Every modifier Vulkan lists for `format`, with its plane count and tiling features.
    fn modifiers(
        query: FormatPropertiesFn,
        physical: *mut c_void,
        format: i32,
    ) -> Vec<DrmFormatModifierProperties> {
        let mut list = DrmFormatModifierPropertiesList {
            s_type: DRM_FORMAT_MODIFIER_PROPERTIES_LIST_EXT,
            p_next: std::ptr::null_mut(),
            count: 0,
            properties: std::ptr::null_mut(),
        };
        let mut properties = FormatProperties2 {
            s_type: FORMAT_PROPERTIES_2,
            p_next: (&raw mut list).cast(),
            linear_tiling_features: 0,
            optimal_tiling_features: 0,
            buffer_features: 0,
        };
        // SAFETY: a list with a null array asks only for the count; both structures live
        // across the call.
        unsafe { query(physical, format, &raw mut properties) };
        if list.count == 0 {
            return Vec::new();
        }
        let mut modifiers = vec![DrmFormatModifierProperties::default(); list.count as usize];
        list.properties = modifiers.as_mut_ptr();
        properties.p_next = (&raw mut list).cast();
        // SAFETY: the array holds `count` elements, the count the same query just returned.
        unsafe { query(physical, format, &raw mut properties) };
        modifiers.truncate(list.count as usize);
        modifiers
    }

    /// The largest `(width, height)` of an image of `format` with `modifier`, sampled and
    /// copied from, in imported DMA-BUF memory — or `None` where Vulkan will not make one.
    fn extent(
        query: ImageFormatPropertiesFn,
        physical: *mut c_void,
        format: i32,
        modifier: u64,
    ) -> Option<(u32, u32)> {
        let drm = PhysicalDeviceImageDrmFormatModifierInfo {
            s_type: PHYSICAL_DEVICE_IMAGE_DRM_FORMAT_MODIFIER_INFO_EXT,
            p_next: std::ptr::null(),
            modifier,
            sharing_mode: SHARING_MODE_EXCLUSIVE,
            queue_family_index_count: 0,
            queue_family_indices: std::ptr::null(),
        };
        let external = PhysicalDeviceExternalImageFormatInfo {
            s_type: PHYSICAL_DEVICE_EXTERNAL_IMAGE_FORMAT_INFO,
            p_next: (&raw const drm).cast(),
            handle_type: HANDLE_TYPE_DMA_BUF_EXT,
        };
        let info = PhysicalDeviceImageFormatInfo2 {
            s_type: PHYSICAL_DEVICE_IMAGE_FORMAT_INFO_2,
            p_next: (&raw const external).cast(),
            format,
            ty: IMAGE_TYPE_2D,
            tiling: TILING_DRM_FORMAT_MODIFIER_EXT,
            usage: IMAGE_USAGE,
            flags: 0,
        };
        let mut external_properties = ExternalImageFormatProperties {
            s_type: EXTERNAL_IMAGE_FORMAT_PROPERTIES,
            p_next: std::ptr::null_mut(),
            external_memory_features: 0,
            export_from_imported_handle_types: 0,
            compatible_handle_types: 0,
        };
        let mut properties = ImageFormatProperties2 {
            s_type: IMAGE_FORMAT_PROPERTIES_2,
            p_next: (&raw mut external_properties).cast(),
            max_extent: [0; 3],
            max_mip_levels: 0,
            max_array_layers: 0,
            sample_counts: 0,
            max_resource_size: 0,
        };
        // SAFETY: every structure in both chains lives across the call, and the modifier is
        // one the same device listed for this format.
        let result = unsafe { query(physical, &raw const info, &raw mut properties) };
        let importable =
            external_properties.external_memory_features & EXTERNAL_MEMORY_FEATURE_IMPORTABLE != 0;
        (result == SUCCESS && importable)
            .then_some((properties.max_extent[0], properties.max_extent[1]))
    }

    /// The import itself, for a descriptor [`super::refusal`] has passed on this device.
    pub(super) fn import(
        device: &wgpu::Device,
        buf: &DmaBuf,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, String> {
        let format = format_of(buf.fourcc).ok_or("not an RGB fourcc")?;
        // SAFETY: `buf.fd` is non-negative (`refusal`) and open for as long as `buf.keep` is
        // alive, which the caller's `buf` holds across this call.
        let producer = unsafe { BorrowedFd::borrow_raw(buf.fd) };
        let fd = producer
            .try_clone_to_owned()
            .map_err(|e| format!("dup of the DMA-BUF's fd: {e}"))?;
        let desc = wgpu::TextureDescriptor {
            label: Some("dmabuf"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: USAGE,
            view_formats: &[],
        };
        let hal_desc = wgpu::hal::TextureDescriptor {
            label: desc.label,
            size: desc.size,
            mip_level_count: desc.mip_level_count,
            sample_count: desc.sample_count,
            dimension: desc.dimension,
            format,
            usage: wgpu::TextureUses::RESOURCE | wgpu::TextureUses::COPY_SRC,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let texture = {
            // SAFETY: the guard is used to make one image on this device, which is handed to
            // wgpu below and destroyed by it; nothing the guard names is destroyed here.
            let hal = unsafe { device.as_hal::<Vulkan>() }.ok_or("not a Vulkan device")?;
            // SAFETY: the device has `VULKAN_EXTERNAL_MEMORY_DMA_BUF` (`supported`); `fd` is a
            // descriptor of our own, a `dup` of the producer's DMA-BUF, which Vulkan owns on
            // success and wgpu-hal closes on failure; the fourcc, modifier, stride and offset
            // are the producer's own description of that buffer, and the modifier is one this
            // device imports for the format at this size (`refusal`).
            unsafe {
                hal.texture_from_dmabuf_fd(
                    fd,
                    &hal_desc,
                    buf.modifier,
                    u64::from(buf.stride),
                    u64::from(buf.offset),
                )
            }
            .map_err(|e| format!("Vulkan refused the DMA-BUF: {e}"))?
        };
        // SAFETY: the image was made on this device from `hal_desc`, which is `desc` in
        // wgpu-hal's terms; its memory holds the producer's frame; and its layout is
        // `UNDEFINED`, which a first barrier from `UNINITIALIZED` states — a transition that
        // leaves a one-plane modifier's memory as it is (the module doc).
        Ok(unsafe {
            device.create_texture_from_hal::<Vulkan>(
                texture,
                &desc,
                wgpu::TextureUses::UNINITIALIZED,
            )
        })
    }
}

#[cfg(not(target_os = "linux"))]
mod vulkan {
    //! No DMA-BUF off Linux: every import is refused and nothing is importable.

    use super::Importable;
    use crate::nodes::DmaBuf;

    pub(super) fn importable(_device: &wgpu::Device) -> Vec<Importable> {
        Vec::new()
    }

    pub(super) fn import(
        _device: &wgpu::Device,
        _buf: &DmaBuf,
        _width: u32,
        _height: u32,
    ) -> Result<wgpu::Texture, String> {
        Err("DMA-BUF import is Linux's".to_owned())
    }
}

#[cfg(target_os = "macos")]
mod metal {
    //! The import of an `IOSurface`, a texture per plane, on wgpu-hal's Metal device.

    use crate::nodes::{IoSurface, Layout};
    use objc2_io_surface::IOSurfaceRef;
    use objc2_metal::{
        MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
        MTLTextureUsage,
    };
    use wgpu::hal::api::Metal;

    /// The usages an imported texture carries: sampled by a draw, copied from by a readback.
    const USAGE: wgpu::TextureUsages =
        wgpu::TextureUsages::TEXTURE_BINDING.union(wgpu::TextureUsages::COPY_SRC);

    /// NV12 in studio and in full range, and BGRA, as CoreVideo names them.
    const NV12_VIDEO: u32 = u32::from_be_bytes(*b"420v");
    const NV12_FULL: u32 = u32::from_be_bytes(*b"420f");
    const BGRA: u32 = u32::from_be_bytes(*b"BGRA");

    /// Whether the device is wgpu-hal's Metal one.
    pub(super) fn supported(device: &wgpu::Device) -> bool {
        // SAFETY: the guard is only asked whether it exists, and dropped at once.
        unsafe { device.as_hal::<Metal>() }.is_some()
    }

    /// A plane's texture format, as wgpu names it and as Metal does.
    type Plane = (wgpu::TextureFormat, MTLPixelFormat);

    /// Each plane a frame of `layout` imports as, and the surface's pixel formats that hold it.
    fn planes(layout: Layout) -> Option<(&'static [Plane], &'static [u32])> {
        match layout {
            Layout::Nv12 => Some((
                &[
                    (wgpu::TextureFormat::R8Unorm, MTLPixelFormat::R8Unorm),
                    (wgpu::TextureFormat::Rg8Unorm, MTLPixelFormat::RG8Unorm),
                ],
                &[NV12_VIDEO, NV12_FULL],
            )),
            Layout::Bgra | Layout::Bgrx => Some((
                &[(wgpu::TextureFormat::Bgra8Unorm, MTLPixelFormat::BGRA8Unorm)],
                &[BGRA],
            )),
            _ => None,
        }
    }

    /// Why `surface` cannot hold a frame of `layout` at `width` by `height`, or nothing.
    ///
    /// A pixel format of zero is read as BGRA: Syphon frameworks built before October 2025
    /// leave their shared surface's unset, and it is always BGRA.
    fn refusal(surface: &IOSurfaceRef, layout: Layout, width: u32, height: u32) -> Option<String> {
        let (formats, pixel_formats) = planes(layout)?;
        let format = surface.pixel_format();
        let unset_bgra = format == 0 && pixel_formats.contains(&BGRA);
        if !pixel_formats.contains(&format) && !unset_bgra {
            return Some(format!(
                "an IOSurface in '{}' does not hold {layout:?}",
                String::from_utf8_lossy(&format.to_be_bytes())
            ));
        }
        let count = surface.plane_count();
        if count.max(1) != formats.len() {
            return Some(format!(
                "an IOSurface of {count} planes does not hold {layout:?}"
            ));
        }
        for i in 0..formats.len() {
            let (w, h) = layout.plane_size(i, width, height);
            let (sw, sh) = if count == 0 {
                (surface.width(), surface.height())
            } else {
                (surface.width_of_plane(i), surface.height_of_plane(i))
            };
            if w as usize > sw || h as usize > sh {
                return Some(format!(
                    "plane {i} of {w}×{h} is past the IOSurface's {sw}×{sh}"
                ));
            }
        }
        None
    }

    /// What a texture drawn into a shared surface is made with: drawn into by a blit, sampled
    /// and copied either way by a test.
    const TARGET_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
        .union(wgpu::TextureUsages::TEXTURE_BINDING)
        .union(wgpu::TextureUsages::COPY_SRC)
        .union(wgpu::TextureUsages::COPY_DST);

    /// A BGRA surface as a texture to draw into.
    pub(super) fn target(
        device: &wgpu::Device,
        surface: usize,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, String> {
        if surface == 0 {
            return Err("no IOSurface".to_owned());
        }
        if width == 0 || height == 0 {
            return Err(format!("a {width}×{height} surface has no pixels"));
        }
        // SAFETY: a non-null `IOSurfaceRef`, which the caller holds across this call and for as
        // long as the texture made from it.
        let surface = unsafe { &*(surface as *const IOSurfaceRef) };
        let format = surface.pixel_format();
        if format != BGRA && format != 0 {
            return Err(format!(
                "an IOSurface in '{}' is not BGRA",
                String::from_utf8_lossy(&format.to_be_bytes())
            ));
        }
        if (surface.width(), surface.height()) != (width as usize, height as usize) {
            return Err(format!(
                "the IOSurface is {}×{}, not {width}×{height}",
                surface.width(),
                surface.height()
            ));
        }
        let raw = {
            // SAFETY: the guard is used to make one texture on this device, handed to wgpu
            // below; nothing the guard names is destroyed here.
            let hal = unsafe { device.as_hal::<Metal>() }.ok_or("not a Metal device")?;
            // SAFETY: a plain descriptor of a two-dimensional texture, one level.
            let descriptor = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    MTLPixelFormat::BGRA8Unorm,
                    width as usize,
                    height as usize,
                    false,
                )
            };
            descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
            descriptor.setStorageMode(MTLStorageMode::Shared);
            hal.raw_device()
                .newTextureWithDescriptor_iosurface_plane(&descriptor, surface, 0)
                .ok_or("Metal made no texture of the surface")?
        };
        let format = wgpu::TextureFormat::Bgra8Unorm;
        // SAFETY: a two-dimensional texture of one level and one layer, in `format`, of `width`
        // by `height`, as it was made above; nothing is called when it drops, since the caller,
        // not the texture, holds the surface.
        let hal_texture = unsafe {
            wgpu::hal::metal::Device::texture_from_raw(
                raw,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
                None,
            )
        };
        let desc = wgpu::TextureDescriptor {
            label: Some("syphon"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: TARGET_USAGE,
            view_formats: &[],
        };
        // SAFETY: the texture was made on this device's Metal device, respecting `desc`; wgpu
        // clears no texture made from hal, and every frame drawn into it covers it whole.
        Ok(unsafe {
            device.create_texture_from_hal::<Metal>(
                hal_texture,
                &desc,
                wgpu::TextureUses::UNINITIALIZED,
            )
        })
    }

    /// The import itself: a texture per plane of `frame`'s surface.
    pub(super) fn import(
        device: &wgpu::Device,
        frame: &IoSurface,
        width: u32,
        height: u32,
    ) -> Result<Vec<wgpu::Texture>, String> {
        let layout = frame.mapped.layout;
        let (formats, _) =
            planes(layout).ok_or_else(|| format!("{layout:?} is not a layout this imports"))?;
        if frame.surface == 0 {
            return Err("no IOSurface".to_owned());
        }
        if width == 0 || height == 0 {
            return Err(format!("a {width}×{height} frame has no pixels"));
        }
        // SAFETY: a non-null `IOSurfaceRef`, which the frame's mapped data holds for as long as
        // the caller holds the frame, across this call and every texture made from it.
        let surface = unsafe { &*(frame.surface as *const IOSurfaceRef) };
        if let Some(why) = refusal(surface, layout, width, height) {
            return Err(why);
        }
        let raw = {
            // SAFETY: the guard is used to make textures on this device, which are handed to
            // wgpu below; nothing the guard names is destroyed here.
            let hal = unsafe { device.as_hal::<Metal>() }.ok_or("not a Metal device")?;
            let mut raw = Vec::with_capacity(formats.len());
            for (i, (_, mtl_format)) in formats.iter().enumerate() {
                let (w, h) = layout.plane_size(i, width, height);
                // SAFETY: a plain descriptor of a two-dimensional texture, one level.
                let descriptor = unsafe {
                    MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                        *mtl_format,
                        w as usize,
                        h as usize,
                        false,
                    )
                };
                descriptor.setUsage(MTLTextureUsage::ShaderRead);
                descriptor.setStorageMode(MTLStorageMode::Shared);
                let texture = hal
                    .raw_device()
                    .newTextureWithDescriptor_iosurface_plane(&descriptor, surface, i)
                    .ok_or_else(|| format!("Metal made no texture of plane {i}"))?;
                raw.push(texture);
            }
            raw
        };
        Ok(raw
            .into_iter()
            .zip(formats)
            .enumerate()
            .map(|(i, (texture, (format, _)))| {
                let (w, h) = layout.plane_size(i, width, height);
                let size = wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                };
                // SAFETY: a two-dimensional texture of one level and one layer, in `format`,
                // of `w` by `h`, as it was made above; nothing is called when it drops, since
                // the frame, not the texture, holds the surface.
                let hal_texture = unsafe {
                    wgpu::hal::metal::Device::texture_from_raw(
                        texture,
                        *format,
                        MTLTextureType::Type2D,
                        1,
                        1,
                        wgpu::hal::CopyExtent {
                            width: w,
                            height: h,
                            depth: 1,
                        },
                        None,
                    )
                };
                let desc = wgpu::TextureDescriptor {
                    label: Some("iosurface"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: *format,
                    usage: USAGE,
                    view_formats: &[],
                };
                // SAFETY: the texture was made on this device's Metal device, respecting `desc`,
                // and its memory holds the producer's frame; wgpu clears no texture made from
                // hal, and Metal has no layouts for a first barrier from `UNINITIALIZED` to
                // discard.
                unsafe {
                    device.create_texture_from_hal::<Metal>(
                        hal_texture,
                        &desc,
                        wgpu::TextureUses::UNINITIALIZED,
                    )
                }
            })
            .collect())
    }
}

#[cfg(not(target_os = "macos"))]
mod metal {
    //! No `IOSurface` off macOS: every import is refused.

    use crate::nodes::IoSurface;

    pub(super) fn supported(_device: &wgpu::Device) -> bool {
        false
    }

    pub(super) fn import(
        _device: &wgpu::Device,
        _frame: &IoSurface,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<wgpu::Texture>, String> {
        Err("IOSurface import is macOS's".to_owned())
    }

    pub(super) fn target(
        _device: &wgpu::Device,
        _surface: usize,
        _width: u32,
        _height: u32,
    ) -> Result<wgpu::Texture, String> {
        Err("an IOSurface is macOS's".to_owned())
    }
}

#[cfg(target_os = "windows")]
mod d3d12 {
    //! The import of a frame's Direct3D 12 textures, a view per plane, on wgpu-hal's Direct3D
    //! 12 device.

    use super::{D3d12Device, ImportedPlane};
    use crate::nodes::{D3d12Plane, D3d12Texture, Layout};
    use std::ffi::c_void;
    use wgpu::hal::api::Dx12;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_RESOURCE_DIMENSION_TEXTURE2D, ID3D12Fence, ID3D12Resource,
    };
    use windows::Win32::Graphics::Dxgi::Common::{
        DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_FORMAT_R8_UNORM,
        DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
    };
    use windows::core::Interface;

    /// Whether the device is wgpu-hal's Direct3D 12 one.
    pub(super) fn supported(device: &wgpu::Device) -> bool {
        // SAFETY: the guard is only asked whether it exists, and dropped at once.
        unsafe { device.as_hal::<Dx12>() }.is_some()
    }

    /// The device's `ID3D12Device`, its adapter's LUID and whether it samples NV12.
    pub(super) fn device_of(device: &wgpu::Device) -> Option<D3d12Device> {
        let nv12 = device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_NV12);
        // SAFETY: the guard is only read through — the device's own handle — and nothing it
        // names is destroyed here.
        let hal = unsafe { device.as_hal::<Dx12>() }?;
        let raw = hal.raw_device();
        // SAFETY: a plain query of a live device, which the guard keeps alive.
        let luid = unsafe { raw.GetAdapterLuid() };
        Some(D3d12Device {
            device: raw.as_raw() as usize,
            luid: (i64::from(luid.HighPart) << 32) | i64::from(luid.LowPart),
            nv12,
        })
    }

    /// What each of a frame's textures must be, as wgpu names its format and as DXGI does, for
    /// a frame of `layout` in `count` textures: NV12 whole, NV12's planes in two, or RGB.
    fn wanted(
        layout: Layout,
        count: usize,
    ) -> Option<&'static [(wgpu::TextureFormat, DXGI_FORMAT)]> {
        Some(match (layout, count) {
            (Layout::Nv12, 1) => &[(wgpu::TextureFormat::NV12, DXGI_FORMAT_NV12)],
            (Layout::Nv12, 2) => &[
                (wgpu::TextureFormat::R8Unorm, DXGI_FORMAT_R8_UNORM),
                (wgpu::TextureFormat::Rg8Unorm, DXGI_FORMAT_R8G8_UNORM),
            ],
            (Layout::Rgba, 1) => &[(wgpu::TextureFormat::Rgba8Unorm, DXGI_FORMAT_R8G8B8A8_UNORM)],
            (Layout::Bgra, 1) => &[(wgpu::TextureFormat::Bgra8Unorm, DXGI_FORMAT_B8G8R8A8_UNORM)],
            _ => return None,
        })
    }

    /// The import itself: each texture opened and wrapped, its fence waited on, and a view per
    /// plane.
    pub(super) fn import(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &D3d12Texture,
        width: u32,
        height: u32,
    ) -> Result<Vec<ImportedPlane>, String> {
        let layout = frame.layout;
        let count = frame.textures.len();
        let wanted = wanted(layout, count)
            .ok_or_else(|| format!("{layout:?} in {count} textures is not a frame this imports"))?;
        if wanted[0].0 == wgpu::TextureFormat::NV12
            && !device
                .features()
                .contains(wgpu::Features::TEXTURE_FORMAT_NV12)
        {
            return Err("this device does not sample NV12".to_owned());
        }
        if width == 0 || height == 0 {
            return Err(format!("a {width}×{height} frame has no pixels"));
        }
        let mut planes = Vec::with_capacity(2);
        for (i, (plane, &(format, dxgi))) in frame.textures.iter().zip(wanted).enumerate() {
            let resource = open(device, frame.device, plane)?;
            // The plane's own size where the planes are textures of their own.
            let (w, h) = if count == 1 {
                (width, height)
            } else {
                layout.plane_size(i, width, height)
            };
            let texture = wrap(device, resource, format, dxgi, plane.slice, (w, h))?;
            wait(queue, plane)?;
            let view = |format, aspect| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("d3d12"),
                    format: Some(format),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    aspect,
                    base_array_layer: plane.slice,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            };
            let size = texture.size();
            if format == wgpu::TextureFormat::NV12 {
                planes.push((
                    texture.clone(),
                    view(wgpu::TextureFormat::R8Unorm, wgpu::TextureAspect::Plane0),
                    (size.width, size.height),
                ));
                planes.push((
                    texture.clone(),
                    view(wgpu::TextureFormat::Rg8Unorm, wgpu::TextureAspect::Plane1),
                    (size.width.div_ceil(2), size.height.div_ceil(2)),
                ));
            } else {
                let all = view(format, wgpu::TextureAspect::All);
                planes.push((texture, all, (size.width, size.height)));
            }
        }
        Ok(planes
            .into_iter()
            .map(|(texture, view, (width, height))| ImportedPlane {
                texture,
                view,
                width,
                height,
            })
            .collect())
    }

    /// `plane`'s texture on `device`: the producer's own where it was made on this device, and
    /// opened through its handle where it was made on another.
    fn open(
        device: &wgpu::Device,
        made_on: usize,
        plane: &D3d12Plane,
    ) -> Result<ID3D12Resource, String> {
        if plane.resource == 0 {
            return Err("no Direct3D 12 texture".to_owned());
        }
        // SAFETY: the guard is only read through — the device's own handle — and dropped
        // before any texture is handed to wgpu.
        let hal = unsafe { device.as_hal::<Dx12>() }.ok_or("not a Direct3D 12 device")?;
        let raw_device = hal.raw_device();
        if raw_device.as_raw() as usize == made_on {
            let raw = plane.resource as *mut c_void;
            // SAFETY: a non-null `ID3D12Resource`, which the frame's `keep` holds for as long as
            // the caller holds the frame; the clone takes a reference of its own, which the wgpu
            // texture made from it releases.
            return Ok(unsafe { ID3D12Resource::from_raw_borrowed(&raw) }
                .ok_or("no Direct3D 12 texture")?
                .clone());
        }
        if plane.shared == 0 {
            return Err("the texture is on another Direct3D 12 device".to_owned());
        }
        let mut opened: Option<ID3D12Resource> = None;
        // SAFETY: an NT handle the producer made for the texture, which the frame's `keep` holds
        // open across this call; the resource opened is this device's own, with a reference the
        // wgpu texture made from it releases.
        unsafe {
            raw_device.OpenSharedHandle(HANDLE(plane.shared as *mut c_void), &raw mut opened)
        }
        .map_err(|e| format!("the shared texture would not open: {e}"))?;
        opened.ok_or_else(|| "the shared texture opened as nothing".to_owned())
    }

    /// `resource` as a wgpu texture in `format`, once its own description says it is one of
    /// `dxgi`, two-dimensional, of one level and one sample, holding `slice`, and at least
    /// `least` in size.
    fn wrap(
        device: &wgpu::Device,
        resource: ID3D12Resource,
        format: wgpu::TextureFormat,
        dxgi: DXGI_FORMAT,
        slice: u32,
        least: (u32, u32),
    ) -> Result<wgpu::Texture, String> {
        // SAFETY: a plain query of a live resource.
        let desc = unsafe { resource.GetDesc() };
        let layers = u32::from(desc.DepthOrArraySize);
        if desc.Dimension != D3D12_RESOURCE_DIMENSION_TEXTURE2D
            || desc.Format != dxgi
            || desc.MipLevels != 1
            || desc.SampleDesc.Count != 1
        {
            return Err(format!(
                "a texture in DXGI format {} of {} levels is not {format:?}",
                desc.Format.0, desc.MipLevels
            ));
        }
        if slice >= layers {
            return Err(format!("slice {slice} is past the texture's {layers}"));
        }
        let size = wgpu::Extent3d {
            width: u32::try_from(desc.Width).map_err(|_| "a texture wider than u32")?,
            height: desc.Height,
            depth_or_array_layers: layers,
        };
        if least.0 > size.width || least.1 > size.height {
            return Err(format!(
                "{}×{} is past the {}×{} texture",
                least.0, least.1, size.width, size.height
            ));
        }
        // SAFETY: a two-dimensional texture of one level and `layers` layers, in `format`, of
        // `size`, as its own description says; dropping it releases the reference the caller
        // took, and the frame, not the texture, keeps the producer's.
        let hal_texture = unsafe {
            wgpu::hal::dx12::Device::texture_from_raw(
                resource,
                format,
                wgpu::TextureDimension::D2,
                size,
                1,
                1,
            )
        };
        let desc = wgpu::TextureDescriptor {
            label: Some("d3d12"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        // SAFETY: the texture is this device's own `ID3D12Device`'s, made on it or opened on it,
        // respecting `desc`, and its memory holds the producer's frame once its fence has been
        // waited on; a first barrier from `UNINITIALIZED` is one from the common state, which is
        // the state a producer's queue leaves a texture in.
        Ok(unsafe {
            device.create_texture_from_hal::<Dx12>(
                hal_texture,
                &desc,
                wgpu::TextureUses::UNINITIALIZED,
            )
        })
    }

    /// Stage a wait on `plane`'s fence for `queue`'s next submission, where it has one it has
    /// not passed yet.
    fn wait(queue: &wgpu::Queue, plane: &D3d12Plane) -> Result<(), String> {
        if plane.fence == 0 {
            return Ok(());
        }
        let raw = plane.fence as *mut c_void;
        // SAFETY: a non-null `ID3D12Fence`, which the frame's `keep` holds for as long as the
        // caller holds the frame; the clone takes a reference of its own, which the queue
        // releases once it has waited.
        let fence = unsafe { ID3D12Fence::from_raw_borrowed(&raw) }
            .ok_or("no fence")?
            .clone();
        // SAFETY: a plain query of a live fence.
        if unsafe { fence.GetCompletedValue() } >= plane.fence_value {
            return Ok(());
        }
        // SAFETY: the guard is used to stage one wait on the queue and dropped at once; nothing
        // it names is destroyed here.
        let hal = unsafe { queue.as_hal::<Dx12>() }.ok_or("not a Direct3D 12 queue")?;
        hal.add_wait_fence(fence, plane.fence_value);
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod d3d12 {
    //! No Direct3D 12 off Windows: every import is refused.

    use super::{D3d12Device, ImportedPlane};
    use crate::nodes::D3d12Texture;

    pub(super) fn supported(_device: &wgpu::Device) -> bool {
        false
    }

    pub(super) fn device_of(_device: &wgpu::Device) -> Option<D3d12Device> {
        None
    }

    pub(super) fn import(
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _frame: &D3d12Texture,
        _width: u32,
        _height: u32,
    ) -> Result<Vec<ImportedPlane>, String> {
        Err("Direct3D 12 import is Windows'".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(fourcc: u32, modifier: u64, stride: u32) -> DmaBuf {
        DmaBuf {
            fd: 3,
            fourcc,
            modifier,
            stride,
            offset: 0,
            keep: Arc::new(()),
            refused: None,
        }
    }

    const LINEAR_RGBA: Importable = Importable {
        fourcc: AB24,
        modifier: 0,
        max: (16384, 16384),
    };

    #[test]
    fn each_fourcc_samples_as_rgba_in_its_own_byte_order() {
        assert_eq!(format_of(AB24), Some(wgpu::TextureFormat::Rgba8Unorm));
        assert_eq!(format_of(XB24), Some(wgpu::TextureFormat::Rgba8Unorm));
        assert_eq!(format_of(AR24), Some(wgpu::TextureFormat::Bgra8Unorm));
        assert_eq!(format_of(XR24), Some(wgpu::TextureFormat::Bgra8Unorm));
        assert_eq!(format_of(u32::from_le_bytes(*b"NV12")), None);
        assert!(alpha_is_padding(XR24) && alpha_is_padding(XB24));
        assert!(!alpha_is_padding(AR24) && !alpha_is_padding(AB24));
    }

    #[test]
    fn a_descriptor_the_device_takes_passes() {
        assert_eq!(refusal(&[LINEAR_RGBA], &buf(AB24, 0, 256), 64, 64), None);
    }

    #[test]
    fn what_the_driver_would_be_handed_wrong_is_refused_before_it() {
        let formats = [LINEAR_RGBA];
        let refused = |b: &DmaBuf, w, h| refusal(&formats, b, w, h).is_some();
        assert!(refused(&buf(u32::from_le_bytes(*b"NV12"), 0, 256), 64, 64));
        assert!(refused(&buf(AB24, 0, 256), 0, 64));
        assert!(refused(&buf(AB24, 0, 255), 64, 64), "a short stride");
        assert!(
            refused(&buf(AB24, 1 << 56 | 1, 256), 64, 64),
            "a modifier not listed"
        );
        assert!(refused(&buf(AR24, 0, 256), 64, 64), "a fourcc not listed");
        assert!(
            refused(&buf(AB24, 0, 1 << 20), 20000, 64),
            "past the extent"
        );
        let mut closed = buf(AB24, 0, 256);
        closed.fd = -1;
        assert!(refused(&closed, 64, 64));
    }

    #[test]
    fn a_refusal_names_the_fourcc() {
        let why = refusal(&[], &buf(XR24, 0, 256), 64, 64).unwrap();
        assert!(why.contains("XR24"), "{why}");
    }
}
