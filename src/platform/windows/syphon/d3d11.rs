// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Direct3D 11 side of Spout: one device on the renderer's adapter, the textures Spout
//! shares, and the bridges that carry a picture between them and Direct3D 12.
//!
//! **Spout shares Direct3D 11 textures by legacy handle, which Direct3D 12 cannot open**, and
//! one texture cannot carry a legacy handle and an NT handle both. So each sender and receiver
//! has a **bridge**: a texture made in Direct3D 12 on the renderer's own device, in a shared
//! heap and with simultaneous access, opened here by its NT handle. wgpu draws into a sender's
//! bridge or samples a receiver's, and one `CopyResource` a frame crosses between a bridge and
//! the Spout texture. Simultaneous access is what lets two APIs take turns on a bridge with no
//! barrier between them: each `ExecuteCommandLists` leaves it in the common state.
//!
//! **The device is made on the renderer's adapter, by its LUID**, since a legacy handle opens
//! only on the adapter its texture lives on, with BGRA support, which a BGRA shared texture
//! needs to open. It is made once a process, for the renderer's device, and its immediate
//! context, which is not free-threaded, is used under a lock.
//!
//! **A copy is waited for on the GPU where the driver can.** A Direct3D 12 fence made shared
//! and opened here is signalled by the immediate context after a copy, and the renderer's
//! queue waits on it; where Direct3D 11.4's fences are missing, an event query is waited for on
//! the calling thread instead.

use std::ffi::c_void;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    CloseHandle, E_INVALIDARG, GENERIC_ALL, HANDLE, HMODULE, S_OK, WAIT_ABANDONED,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_QUERY_DESC,
    D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_SHARED, D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11Device1, ID3D11Device5, ID3D11DeviceContext,
    ID3D11DeviceContext4, ID3D11Fence, ID3D11Query, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_CPU_PAGE_PROPERTY_UNKNOWN, D3D12_FENCE_FLAG_SHARED, D3D12_HEAP_FLAG_SHARED,
    D3D12_HEAP_PROPERTIES, D3D12_HEAP_TYPE_DEFAULT, D3D12_MEMORY_POOL_UNKNOWN, D3D12_RESOURCE_DESC,
    D3D12_RESOURCE_DIMENSION_TEXTURE2D, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS, D3D12_RESOURCE_STATE_COMMON,
    D3D12_TEXTURE_LAYOUT_UNKNOWN, ID3D12Device, ID3D12Fence, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1, IDXGIFactory4, IDXGIKeyedMutex,
    IDXGIResource,
};
use windows::core::{BOOL, Interface, PCWSTR};

/// How long a copy is waited for on the CPU before it is given up on.
const PATIENCE: Duration = Duration::from_secs(2);

/// The immediate context, used under the device's lock.
struct Context {
    immediate: ID3D11DeviceContext,
    /// Direct3D 11.4's, which signals a fence; `None` where the driver has none.
    signals: Option<ID3D11DeviceContext4>,
}

/// The process's Direct3D 11 device, on the renderer's adapter.
pub(super) struct Device {
    /// The renderer's own device, which every bridge is made on.
    d3d12: ID3D12Device,
    d3d11: ID3D11Device,
    device1: ID3D11Device1,
    /// Direct3D 11.4's, which opens a shared fence; `None` where the driver has none.
    device5: Option<ID3D11Device5>,
    context: Mutex<Context>,
}

// SAFETY: a Direct3D 11 and a Direct3D 12 device are free-threaded; the immediate context,
// which is not, is reached only under `context`'s lock.
unsafe impl Send for Device {}
// SAFETY: as above.
unsafe impl Sync for Device {}

/// The device made for the renderer's, kept for as long as the renderer is that device.
static DEVICE: Mutex<Option<Arc<Device>>> = Mutex::new(None);

/// The process's device, made the first time it is asked for on the renderer's adapter.
pub(super) fn here() -> Result<Arc<Device>, String> {
    let served = crate::render::dmabuf::d3d12_here()
        .ok_or("Spout waits for the renderer's Direct3D 12 device")?;
    let mut held = DEVICE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(device) = held.as_ref()
        && device.d3d12.as_raw() as usize == served.device
    {
        return Ok(Arc::clone(device));
    }
    let raw = served.device as *mut c_void;
    // SAFETY: the renderer's live `ID3D12Device`, which the renderer holds across this call;
    // the clone takes a reference of its own.
    let d3d12 = unsafe { ID3D12Device::from_raw_borrowed(&raw) }
        .ok_or("the renderer has no Direct3D 12 device")?
        .clone();
    let device = Arc::new(Device::new(d3d12)?);
    *held = Some(Arc::clone(&device));
    Ok(device)
}

/// A LUID as the 64-bit number Spout's tools and the peers print.
pub(super) fn luid_bits(high: i32, low: u32) -> u64 {
    (u64::from(high as u32) << 32) | u64::from(low)
}

impl Device {
    fn new(d3d12: ID3D12Device) -> Result<Self, String> {
        // SAFETY: a plain query of a live device.
        let luid = unsafe { d3d12.GetAdapterLuid() };
        // SAFETY: a factory of this process's own, used and dropped here.
        let factory: IDXGIFactory4 =
            unsafe { CreateDXGIFactory1() }.map_err(|e| format!("Spout's DXGI factory: {e}"))?;
        // SAFETY: a live factory asked for the adapter of a LUID it listed to the renderer.
        let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapterByLuid(luid) }
            .map_err(|e| format!("the renderer's adapter, by its LUID: {e}"))?;
        // SAFETY: a plain query of a live adapter.
        let desc = unsafe { adapter.GetDesc1() }.map_err(|e| format!("the adapter: {e}"))?;
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .to_owned();
        let software = desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        let made = make(&adapter, &levels).or_else(|e| {
            // A runtime without Direct3D 11.1 refuses a list that names it.
            if e.code() == E_INVALIDARG {
                make(&adapter, &levels[1..])
            } else {
                Err(e)
            }
        });
        let (device, immediate) =
            made.map_err(|e| format!("Spout's Direct3D 11 device on {name}: {e}"))?;
        log::info!(
            "spout: Direct3D 11 on {name}{}, LUID {:016x}, the renderer's adapter",
            if software { " (software)" } else { "" },
            luid_bits(luid.HighPart, luid.LowPart)
        );
        let device1: ID3D11Device1 = device
            .cast()
            .map_err(|e| format!("Direct3D 11.1, which opens a bridge: {e}"))?;
        let device5: Option<ID3D11Device5> = device.cast().ok();
        let signals: Option<ID3D11DeviceContext4> = immediate.cast().ok();
        if device5.is_none() || signals.is_none() {
            log::info!("spout: no Direct3D 11.4 fences here; each copy is waited for on the CPU");
        }
        Ok(Self {
            d3d12,
            d3d11: device,
            device1,
            device5,
            context: Mutex::new(Context { immediate, signals }),
        })
    }

    /// The renderer's `ID3D12Device`, as an integer.
    pub fn d3d12_raw(&self) -> usize {
        self.d3d12.as_raw() as usize
    }

    fn context(&self) -> std::sync::MutexGuard<'_, Context> {
        self.context.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A bridge of `width` by `height` in `format`: made in Direct3D 12 on the renderer's
    /// device, in a shared heap with simultaneous access, a render target, and opened here by
    /// its NT handle.
    pub fn bridge(&self, width: u32, height: u32, format: DXGI_FORMAT) -> Result<Bridge, String> {
        let heap = D3D12_HEAP_PROPERTIES {
            Type: D3D12_HEAP_TYPE_DEFAULT,
            CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
            CreationNodeMask: 0,
            VisibleNodeMask: 0,
        };
        // A render target whether or not anything draws into it: Direct3D 11 refuses to open
        // a Direct3D 12 texture that is not one ("The parameter is incorrect", on WARP).
        let flags =
            D3D12_RESOURCE_FLAG_ALLOW_SIMULTANEOUS_ACCESS | D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET;
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Alignment: 0,
            Width: u64::from(width),
            Height: height,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
            Flags: flags,
        };
        let mut resource: Option<ID3D12Resource> = None;
        // SAFETY: descriptions of a default heap and a two-dimensional texture that outlive
        // the call; the texture made is this value's.
        unsafe {
            self.d3d12.CreateCommittedResource(
                &raw const heap,
                D3D12_HEAP_FLAG_SHARED,
                &raw const desc,
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &raw mut resource,
            )
        }
        .map_err(|e| format!("a {width}×{height} shared Direct3D 12 texture: {e}"))?;
        let resource = resource.ok_or("Direct3D 12 made no texture")?;
        // SAFETY: a texture in a shared heap, shared with every access and no name.
        let handle = unsafe {
            self.d3d12
                .CreateSharedHandle(&resource, None, GENERIC_ALL.0, PCWSTR::null())
        }
        .map_err(|e| format!("the bridge's NT handle: {e}"))?;
        // SAFETY: an NT handle Direct3D 12 just made, opened on this device on the same
        // adapter; the handle is closed after, which the opened texture survives.
        let opened = unsafe { self.device1.OpenSharedResource1::<ID3D11Texture2D>(handle) };
        // SAFETY: the handle made above, closed once.
        let _ = unsafe { CloseHandle(handle) };
        let texture = opened.map_err(|e| format!("the bridge in Direct3D 11: {e}"))?;
        Ok(Bridge {
            resource,
            texture,
            width,
            height,
            format,
        })
    }

    /// A texture Spout's receivers open: B8G8R8A8, drawn into and sampled, shared by a legacy
    /// handle — as every Spout sender makes one — and that handle, cut to the 32 bits a
    /// sender's map carries.
    pub fn spout_texture(&self, width: u32, height: u32) -> Result<(ID3D11Texture2D, u32), String> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        // SAFETY: a description that outlives the call, no initial data.
        unsafe {
            self.d3d11
                .CreateTexture2D(&raw const desc, None, Some(&raw mut texture))
        }
        .map_err(|e| format!("a {width}×{height} Spout texture: {e}"))?;
        let texture = texture.ok_or("Direct3D 11 made no texture")?;
        let resource: IDXGIResource = texture
            .cast()
            .map_err(|e| format!("the Spout texture's DXGI resource: {e}"))?;
        // SAFETY: a texture made shared; its legacy handle is the runtime's, never closed.
        let handle = unsafe { resource.GetSharedHandle() }
            .map_err(|e| format!("the Spout texture's share handle: {e}"))?;
        // SAFETY: a live context, under its lock.
        unsafe { self.context().immediate.Flush() };
        let bits = handle.0 as usize;
        let cut = bits as u32;
        if cut as usize != bits {
            log::warn!("spout: a share handle {bits:#x} wider than the 32 bits Spout carries");
        }
        Ok((texture, cut))
    }

    /// The texture a sender's map names by `handle`, opened here: zero-extended, as Spout
    /// 2.007 reads it, and sign-extended where that fails and the top bit is set, as older
    /// receivers read it.
    pub fn open_legacy(&self, handle: u32) -> Result<ID3D11Texture2D, String> {
        let open = |raw: usize| {
            let mut texture: Option<ID3D11Texture2D> = None;
            // SAFETY: a legacy handle another process's runtime made, which this device's
            // runtime looks up; nothing is closed.
            unsafe {
                self.d3d11
                    .OpenSharedResource(HANDLE(raw as *mut c_void), &raw mut texture)
            }
            .and_then(|()| texture.ok_or_else(|| windows::core::Error::from(E_INVALIDARG)))
        };
        let zero = handle as usize;
        let sign = handle as i32 as isize as usize;
        match open(zero) {
            Ok(texture) => Ok(texture),
            Err(_) if sign != zero => open(sign),
            Err(e) => Err(e),
        }
        .map_err(|e| {
            format!(
                "the sender's texture {handle:#x} would not open here, which a sender on \
                 another GPU does: {e}"
            )
        })
    }

    /// A fence the renderer's queue and this device share, or `None` where the driver cannot
    /// share one.
    pub fn fence(&self) -> Option<Fence> {
        let device5 = self.device5.as_ref()?;
        self.context().signals.as_ref()?;
        let made = (|| -> Result<Fence, windows::core::Error> {
            // SAFETY: a fence on the renderer's device, made shared, at zero.
            let d3d12: ID3D12Fence = unsafe { self.d3d12.CreateFence(0, D3D12_FENCE_FLAG_SHARED) }?;
            // SAFETY: a fence made shared, shared with every access and no name.
            let handle = unsafe {
                self.d3d12
                    .CreateSharedHandle(&d3d12, None, GENERIC_ALL.0, PCWSTR::null())
            }?;
            let mut d3d11: Option<ID3D11Fence> = None;
            // SAFETY: the NT handle just made, opened on this device on the same adapter, and
            // closed after, which the opened fence survives.
            let opened = unsafe { device5.OpenSharedFence(handle, &raw mut d3d11) };
            // SAFETY: the handle made above, closed once.
            let _ = unsafe { CloseHandle(handle) };
            opened?;
            let d3d11 = d3d11.ok_or_else(|| windows::core::Error::from(E_INVALIDARG))?;
            Ok(Fence { d3d12, d3d11 })
        })();
        match made {
            Ok(fence) => Some(fence),
            Err(e) => {
                log::info!("spout: no shared fence ({e}); each copy is waited for on the CPU");
                None
            }
        }
    }

    /// An event query, for waiting on a copy where there is no fence.
    pub fn query(&self) -> Result<ID3D11Query, String> {
        let desc = D3D11_QUERY_DESC {
            Query: D3D11_QUERY_EVENT,
            MiscFlags: 0,
        };
        let mut query: Option<ID3D11Query> = None;
        // SAFETY: a description that outlives the call.
        unsafe {
            self.d3d11
                .CreateQuery(&raw const desc, Some(&raw mut query))
        }
        .map_err(|e| format!("an event query: {e}"))?;
        query.ok_or_else(|| "Direct3D 11 made no query".to_owned())
    }

    /// Copy `from` into `into` on the immediate context, then mark it done as `done` says and
    /// flush, as Spout's senders and receivers flush after their copy. A source shared with a
    /// keyed mutex is copied under it, or not at all where it stays taken.
    pub fn copy(
        &self,
        into: &ID3D11Texture2D,
        from: &ID3D11Texture2D,
        done: &Done<'_>,
    ) -> Result<(), String> {
        let keyed = keyed_mutex(from);
        if let Some(keyed) = &keyed
            && !acquire_keyed(keyed)
        {
            return Err("the sender's keyed mutex stayed taken".to_owned());
        }
        let context = self.context();
        // SAFETY: two live textures of this device, of the same size and format family, the
        // copy recorded on the context its lock holds.
        unsafe { context.immediate.CopyResource(into, from) };
        if let Some(keyed) = &keyed {
            // SAFETY: the keyed mutex acquired above, released once.
            let _ = unsafe { keyed.ReleaseSync(0) };
        }
        let signalled = match done {
            Done::Fence(fence, value) => match &context.signals {
                // SAFETY: a fence this device opened, signalled after the copy.
                Some(signals) => unsafe { signals.Signal(&fence.d3d11, *value) }
                    .map_err(|e| format!("the fence after a copy: {e}")),
                None => Err("no context signals a fence".to_owned()),
            },
            Done::Query(query) => {
                // SAFETY: an event query of this device, ended after the copy.
                unsafe { context.immediate.End(*query) };
                Ok(())
            }
        };
        // SAFETY: a live context, under its lock.
        unsafe { context.immediate.Flush() };
        signalled
    }

    /// Wait on this thread for the copy an event query was ended after.
    pub fn wait_query(&self, query: &ID3D11Query) -> Result<(), String> {
        let since = Instant::now();
        loop {
            let mut done = BOOL(0);
            {
                let context = self.context();
                // SAFETY: an event query of this device, whose data is a `BOOL` of the size
                // passed, read under the context's lock.
                let _ = unsafe {
                    context.immediate.GetData(
                        query,
                        Some((&raw mut done).cast::<c_void>()),
                        std::mem::size_of::<BOOL>() as u32,
                        0,
                    )
                };
            }
            if done.as_bool() {
                return Ok(());
            }
            if since.elapsed() > PATIENCE {
                return Err(format!("a Direct3D 11 copy took more than {PATIENCE:?}"));
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// The bytes of `texture`, an 8-bit texture of four channels, rows top first and packed:
    /// copied to a staging texture and mapped. Waits for the GPU; for a test.
    pub fn read(&self, texture: &ID3D11Texture2D) -> Result<Vec<u8>, String> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a plain query of a live texture.
        unsafe { texture.GetDesc(&raw mut desc) };
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..desc
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        // SAFETY: a description that outlives the call, no initial data.
        unsafe {
            self.d3d11
                .CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging))
        }
        .map_err(|e| format!("a staging texture: {e}"))?;
        let staging = staging.ok_or("Direct3D 11 made no staging texture")?;
        let context = self.context();
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: two live textures of the same description but their use, copied and the
        // copy mapped for reading, which waits for it, on the context its lock holds.
        unsafe {
            context.immediate.CopyResource(&staging, texture);
            context
                .immediate
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped))
        }
        .map_err(|e| format!("the staging texture would not map: {e}"))?;
        let row = desc.Width as usize * 4;
        let mut out = Vec::with_capacity(row * desc.Height as usize);
        for y in 0..desc.Height as usize {
            // SAFETY: the mapped staging texture holds `Height` rows `RowPitch` bytes apart,
            // each at least `Width × 4` bytes, until it is unmapped below.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    mapped.pData.cast::<u8>().add(y * mapped.RowPitch as usize),
                    row,
                )
            };
            out.extend_from_slice(bytes);
        }
        // SAFETY: mapped above, unmapped once.
        unsafe { context.immediate.Unmap(&staging, 0) };
        Ok(out)
    }
}

/// `D3D11CreateDevice` on `adapter` at the first of `levels` it takes, with BGRA support.
fn make(
    adapter: &IDXGIAdapter1,
    levels: &[D3D_FEATURE_LEVEL],
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    // SAFETY: a live adapter, its driver type therefore unknown, no software module, and
    // places for the device and its immediate context.
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(levels),
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            Some(&raw mut context),
        )
    }?;
    match (device, context) {
        (Some(device), Some(context)) => Ok((device, context)),
        _ => Err(E_INVALIDARG.into()),
    }
}

/// The keyed mutex a texture was shared with, where it was.
fn keyed_mutex(texture: &ID3D11Texture2D) -> Option<IDXGIKeyedMutex> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: a plain query of a live texture.
    unsafe { texture.GetDesc(&raw mut desc) };
    if desc.MiscFlags & D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX.0 as u32 == 0 {
        return None;
    }
    texture.cast().ok()
}

/// `keyed` taken with key zero, waited for as Spout waits; abandoned is held.
fn acquire_keyed(keyed: &IDXGIKeyedMutex) -> bool {
    // SAFETY: the method called through its own vtable, for the result the binding folds
    // into `Ok`: a timeout and an abandoned mutex are both success codes.
    let result = unsafe {
        (Interface::vtable(keyed).AcquireSync)(keyed.as_raw(), 0, super::registry::WAIT_MS)
    };
    result == S_OK || result.0 as u32 == WAIT_ABANDONED.0
}

/// A texture in Direct3D 12 on the renderer's device, and the same texture in Direct3D 11.
pub(super) struct Bridge {
    pub resource: ID3D12Resource,
    pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    pub format: DXGI_FORMAT,
}

impl Bridge {
    /// The `ID3D12Resource`, as an integer.
    pub fn resource_raw(&self) -> usize {
        self.resource.as_raw() as usize
    }
}

// SAFETY: Direct3D 11 textures and Direct3D 12 resources are free-threaded; the immediate
// context that copies them is used under its lock.
unsafe impl Send for Bridge {}
// SAFETY: as above.
unsafe impl Sync for Bridge {}

/// A fence the renderer's Direct3D 12 queue waits on and this device signals.
pub(super) struct Fence {
    pub d3d12: ID3D12Fence,
    d3d11: ID3D11Fence,
}

// SAFETY: fences are free-threaded.
unsafe impl Send for Fence {}
// SAFETY: as above.
unsafe impl Sync for Fence {}

impl Fence {
    /// The Direct3D 12 fence, as an integer.
    pub fn raw(&self) -> usize {
        self.d3d12.as_raw() as usize
    }

    /// Wait on this thread for the fence to reach `value`.
    pub fn wait(&self, value: u64) -> Result<(), String> {
        let since = Instant::now();
        // SAFETY: a plain query of a live fence.
        while unsafe { self.d3d12.GetCompletedValue() } < value {
            if since.elapsed() > PATIENCE {
                return Err(format!("a Direct3D 11 copy took more than {PATIENCE:?}"));
            }
            std::thread::sleep(Duration::from_micros(250));
        }
        Ok(())
    }
}

/// How a copy is marked done: a fence signalled to a value, or an event query ended.
pub(super) enum Done<'a> {
    Fence(&'a Fence, u64),
    Query(&'a ID3D11Query),
}
