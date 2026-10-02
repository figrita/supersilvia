// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture sent over NDI®: drawn into a BGRA target of its own, read back off the GPU, and
//! handed to a [`Sender`] as bytes.
//!
//! **The Syphon outlet's blit, then a readback.** The picture is drawn with a picture window's
//! [`Viewer`] into an 8-bit BGRA texture at the picture's own size — top row first, since NDI
//! has one orientation — then copied into a staging buffer ([`super::readback::Read`]), and the
//! map is asked for once the submission is made. **Nothing waits**: [`Outlet::collect`] takes
//! each map that has landed, oldest first, and hands its rows to the sender; a picture that
//! arrives while both staging buffers are still out is not drawn, so the newest frame that can
//! be read wins and one that cannot is dropped, as a camera's is. `proposals/ndi.md`, route A.
//!
//! **Alpha.** Opaque by default: drawn over black and sent as BGRx, which NDI carries without an
//! alpha plane. [`Look::transparent`] draws over nothing and sends BGRA, the picture's alpha
//! premultiplied as the blit blends it. [`Look::flip`] is Syphon's and means nothing here.

use super::publish::Look;
use super::readback::{Landed, Read, padded_row};
use super::{Gpu, Picture, Ticket, Viewer, viewer::Viewport};
use crate::video::ndi::{Format, Sender};

/// How many frames may be on their way back off the GPU at once.
const IN_FLIGHT: usize = 2;

/// The texture a picture is drawn into, and the staging buffers its reads land in.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
    reads: Vec<Staged>,
}

/// One staging buffer, and what the frame read into it was.
struct Staged {
    read: Read,
    format: Format,
    /// The tick that drew the picture read into it, so reads are sent in order.
    tick: u64,
}

/// One picture sent over NDI: a sender, and what it is drawn into.
pub struct Outlet {
    sender: Sender,
    target: Option<Target>,
    /// The tick of the last frame handed to the sender.
    sent: Option<u64>,
}

impl Outlet {
    /// A sender announced as `name`, with nothing drawn yet. Refused where the NDI runtime is
    /// missing.
    pub fn new(name: &str) -> Result<Self, String> {
        Ok(Self {
            sender: Sender::new(name)?,
            target: None,
            sent: None,
        })
    }

    /// Whether a staging buffer is free for another frame.
    pub fn can_draw(&self) -> bool {
        self.target
            .as_ref()
            .is_none_or(|t| t.reads.iter().any(|s| s.read.is_idle()))
    }

    /// Whether a frame is still on its way back.
    pub fn busy(&self) -> bool {
        self.target
            .as_ref()
            .is_some_and(|t| t.reads.iter().any(|s| !s.read.is_idle()))
    }

    /// Draw `picture` as `look` says, copy it into a free staging buffer and submit, asking for
    /// the map. `rate` is what the stream declares, `tick` the tick that drew the picture. Call
    /// only where [`Self::can_draw`].
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        viewer: &Viewer,
        picture: &Picture,
        look: Look,
        rate: u32,
        tick: u64,
    ) -> Result<Ticket, String> {
        let size = (picture.width.max(1), picture.height.max(1));
        if self.target.as_ref().is_none_or(|t| t.size != size) {
            // A read still out on the old size is dropped with it, and never sent.
            self.target = Some(target(gpu, size));
        }
        let target = self.target.as_mut().expect("made above");
        let staged = target
            .reads
            .iter_mut()
            .find(|s| s.read.is_idle())
            .ok_or("no staging buffer is free")?;
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ndi") });
        let ground = if look.transparent {
            wgpu::Color::TRANSPARENT
        } else {
            wgpu::Color::BLACK
        };
        super::shared::clear(&mut encoder, &target.view, ground);
        // The blit writes a picture's top row first, which is NDI's.
        viewer.show(
            &mut encoder,
            &target.view,
            size,
            picture,
            Viewport::whole(size),
            super::Fit::Letterbox,
            0.0,
        );
        encoder.copy_texture_to_buffer(
            target.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: staged.read.buffer(),
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row(size.0)),
                    rows_per_image: None,
                },
            },
            target.texture.size(),
        );
        staged.read.copied();
        staged.format = Format {
            width: size.0,
            height: size.1,
            alpha: look.transparent,
            rate,
        };
        staged.tick = tick;
        let ticket = gpu.submit([encoder.finish()]);
        staged.read.map(&ticket);
        Ok(ticket)
    }

    /// Hand the sender every frame whose map has landed, oldest first, and free its buffer.
    /// Never waits.
    pub fn collect(&mut self) -> Result<(), String> {
        let Some(target) = &mut self.target else {
            return Ok(());
        };
        let mut order: Vec<usize> = (0..target.reads.len()).collect();
        order.sort_by_key(|&i| target.reads[i].tick);
        let mut result = Ok(());
        for i in order {
            let staged = &mut target.reads[i];
            let (format, tick) = (staged.format, staged.tick);
            let sender = &mut self.sender;
            let late = self.sent.is_some_and(|sent| tick <= sent);
            let landed = staged.read.take(|bytes| {
                if late {
                    return Ok(());
                }
                sender.send(format, |out| unpad(bytes, out, format.width))
            });
            match landed {
                Landed::Pending | Landed::Lost => {}
                Landed::Bytes(sent) => {
                    if !late {
                        self.sent = Some(tick);
                    }
                    if let Err(e) = sent {
                        result = Err(e);
                    }
                }
            }
        }
        if let Some(e) = self.sender.error() {
            result = Err(e);
        }
        result
    }
}

/// A BGRA target of `size`, and its staging buffers.
fn target(gpu: &Gpu, size: (u32, u32)) -> Target {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("ndi"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bytes = u64::from(padded_row(size.0)) * u64::from(size.1);
    let reads = (0..IN_FLIGHT)
        .map(|_| Staged {
            read: Read::new(gpu, bytes, "ndi"),
            format: Format {
                width: size.0,
                height: size.1,
                alpha: false,
                rate: 0,
            },
            tick: 0,
        })
        .collect();
    Target {
        texture,
        view,
        size,
        reads,
    }
}

/// Rows of `width` four-byte texels out of a read padded to a texture copy's row, top first,
/// into `out`, packed.
fn unpad(bytes: &[u8], out: &mut [u8], width: u32) {
    let (row, padded) = ((width * 4) as usize, padded_row(width) as usize);
    if row == padded {
        let n = out.len().min(bytes.len());
        out[..n].copy_from_slice(&bytes[..n]);
        return;
    }
    for (dst, src) in out.chunks_exact_mut(row).zip(bytes.chunks(padded)) {
        dst.copy_from_slice(&src[..row]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A padded read comes out packed, every row whole and nothing of the padding.
    #[test]
    fn a_padded_read_comes_out_packed() {
        let width = 3;
        let padded = padded_row(width) as usize;
        let mut bytes = vec![0xee; padded * 2];
        for (r, row) in bytes.chunks_mut(padded).enumerate() {
            for (i, b) in row[..12].iter_mut().enumerate() {
                *b = (r * 12 + i) as u8;
            }
        }
        let mut out = vec![0; 24];
        unpad(&bytes, &mut out, width);
        assert_eq!(out, (0..24).collect::<Vec<u8>>());
    }
}
