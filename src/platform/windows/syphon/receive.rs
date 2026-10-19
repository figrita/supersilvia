// SPDX-License-Identifier: AGPL-3.0-or-later

//! A receiver's thread: its sender looked at every few milliseconds, as Spout's receivers look
//! at theirs each frame, and each new frame copied into a bridge of its own and handed on.

use super::d3d11::Device;
use super::registry::{self, Access, Info};
use super::{Held, Surface, Waiter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};

/// How often a sender that counts its frames is looked at.
const COUNTED: Duration = Duration::from_millis(4);
/// How often one that does not is looked at, and copied each time.
const UNCOUNTED: Duration = Duration::from_millis(8);
/// The most bridges one receiver copies into: one being copied into while the renderer still
/// holds the last two.
const BRIDGES: usize = 3;

/// What a client and its thread share.
#[derive(Default)]
pub(super) struct State {
    /// Set by the client to end the thread.
    pub stop: AtomicBool,
    /// Set by the thread once the sender's map no longer opens.
    pub gone: AtomicBool,
    /// Why the sender's frames are refused, while they are.
    pub problem: Mutex<Option<String>>,
}

impl State {
    fn refuse(&self, why: Option<String>) {
        let mut problem = self.problem.lock().unwrap_or_else(PoisonError::into_inner);
        if *problem != why {
            if let Some(why) = &why {
                log::warn!("spout: {why}");
            }
            *problem = why;
        }
    }
}

/// The sender's texture as this receiver has it open, by the handle its map named.
struct Opened {
    handle: u32,
    /// `None` where it would not open or is refused, kept so it is not tried again until
    /// the handle changes.
    texture: Option<ID3D11Texture2D>,
    /// The bridges' format, size, and the sender's lock and count.
    format: DXGI_FORMAT,
    width: u32,
    height: u32,
    access: Option<Access>,
}

/// One receiver's thread.
pub(super) struct Receiving {
    name: Vec<u8>,
    device: Arc<Device>,
    waiter: Waiter,
    state: Arc<State>,
    opened: Option<Opened>,
    bridges: Vec<Arc<Held>>,
}

impl Receiving {
    pub fn new(name: Vec<u8>, device: Arc<Device>, waiter: Waiter, state: Arc<State>) -> Self {
        Self {
            name,
            device,
            waiter,
            state,
            opened: None,
            bridges: Vec::new(),
        }
    }

    /// Look until the client stops it or the sender goes.
    pub fn run(mut self, frames: &impl Fn(Surface)) {
        while !self.state.stop.load(Ordering::Acquire) {
            let Some(counted) = self.look(frames) else {
                self.state.gone.store(true, Ordering::Release);
                return;
            };
            std::thread::sleep(if counted { COUNTED } else { UNCOUNTED });
        }
    }

    /// One look: the sender's map read, its texture opened anew where its handle changed, and
    /// a new frame copied and handed on. Whether the sender counts its frames, or `None` once it
    /// has gone.
    fn look(&mut self, frames: &impl Fn(Surface)) -> Option<bool> {
        let info = match registry::info(&self.name) {
            Ok(Some(info)) => info,
            Ok(None) => return None,
            Err(e) => {
                log::debug!("spout: {e}");
                return Some(true);
            }
        };
        if info.handle == 0 {
            self.opened = None;
            self.state.refuse(Some(format!(
                "{} shares its picture through memory, as Spout 2.006 could, which is not \
                 received",
                registry::text_of(&self.name)
            )));
            return Some(true);
        }
        if self.opened.as_ref().is_none_or(|o| o.handle != info.handle) {
            self.bridges.clear();
            self.opened = Some(self.open(info));
        }
        let opened = self.opened.as_ref().expect("opened above");
        let (Some(texture), Some(access)) = (&opened.texture, &opened.access) else {
            return Some(true);
        };
        let Some(locked) = access.lock() else {
            return Some(true);
        };
        let fresh = access.fresh();
        let counted = access.counts();
        if !fresh {
            return Some(counted);
        }
        let Some(bridge) = free_bridge(&mut self.bridges, &self.device, opened, &self.state) else {
            log::debug!("spout: every bridge is still held; a frame is skipped");
            return Some(counted);
        };
        let done = self.waiter.next();
        let copied = self.device.copy(&bridge.bridge.texture, texture, &done);
        drop(locked);
        if let Err(e) = copied {
            log::debug!("spout: {e}");
            return Some(counted);
        }
        let (fence, value) = match &self.waiter {
            Waiter::Fence { fence, value } => (Some(Arc::clone(fence)), *value),
            Waiter::Query(_) => {
                if let Err(e) = self.waiter.wait(&self.device) {
                    log::debug!("spout: {e}");
                    return Some(counted);
                }
                (None, 0)
            }
        };
        frames(Surface {
            held: bridge,
            fence,
            value,
        });
        Some(counted)
    }

    /// The sender's texture at `info.handle`, opened, checked, and the lock and count beside
    /// it; a refusal is said through the client.
    fn open(&self, info: Info) -> Opened {
        let mut opened = Opened {
            handle: info.handle,
            texture: None,
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            width: 0,
            height: 0,
            access: None,
        };
        let sender = registry::text_of(&self.name);
        let texture = match self.device.open_legacy(info.handle) {
            Ok(texture) => texture,
            Err(e) => {
                self.state.refuse(Some(format!("{sender}: {e}")));
                return opened;
            }
        };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a plain query of a live texture.
        unsafe { texture.GetDesc(&raw mut desc) };
        let format = match desc.Format {
            DXGI_FORMAT_B8G8R8A8_UNORM | DXGI_FORMAT_B8G8R8A8_UNORM_SRGB => {
                DXGI_FORMAT_B8G8R8A8_UNORM
            }
            DXGI_FORMAT_R8G8B8A8_UNORM | DXGI_FORMAT_R8G8B8A8_UNORM_SRGB => {
                DXGI_FORMAT_R8G8B8A8_UNORM
            }
            other => {
                self.state.refuse(Some(format!(
                    "{sender} sends DXGI format {}, and only 8-bit BGRA and RGBA are received",
                    other.0
                )));
                return opened;
            }
        };
        if desc.MipLevels != 1 || desc.ArraySize != 1 || desc.SampleDesc.Count != 1 {
            self.state.refuse(Some(format!(
                "{sender}'s texture has {} levels, {} layers and {} samples, not one of each",
                desc.MipLevels, desc.ArraySize, desc.SampleDesc.Count
            )));
            return opened;
        }
        if (desc.Width, desc.Height) != (info.width, info.height) {
            log::debug!(
                "spout: {sender} says {}×{} and its texture is {}×{}; the texture's is taken",
                info.width,
                info.height,
                desc.Width,
                desc.Height
            );
        }
        match Access::new(&self.name) {
            Ok(access) => opened.access = Some(access),
            Err(e) => {
                self.state.refuse(Some(format!("{sender}: {e}")));
                return opened;
            }
        }
        log::info!(
            "spout: receiving {sender} at {}×{}, DXGI format {}",
            desc.Width,
            desc.Height,
            desc.Format.0
        );
        self.state.refuse(None);
        opened.texture = Some(texture);
        opened.format = format;
        opened.width = desc.Width;
        opened.height = desc.Height;
        opened
    }
}

/// A bridge of `bridges` no frame holds, made for `opened` where there are fewer than
/// [`BRIDGES`]; `None` where every one is held.
fn free_bridge(
    bridges: &mut Vec<Arc<Held>>,
    device: &Arc<Device>,
    opened: &Opened,
    state: &State,
) -> Option<Arc<Held>> {
    if let Some(free) = bridges.iter().find(|b| Arc::strong_count(b) == 1) {
        return Some(Arc::clone(free));
    }
    if bridges.len() >= BRIDGES {
        return None;
    }
    match device.bridge(opened.width, opened.height, opened.format) {
        Ok(bridge) => {
            let held = Arc::new(Held {
                device: Arc::clone(device),
                bridge,
                spout: None,
            });
            bridges.push(Arc::clone(&held));
            Some(held)
        }
        Err(e) => {
            state.refuse(Some(e));
            None
        }
    }
}
