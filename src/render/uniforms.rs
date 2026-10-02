// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every draw's uniform values, in one buffer a tick.
//!
//! Each Output's and each probe's block is packed by its program's `wgsl::UniformLayout` —
//! `u_resolution` and `u_time` first, then the uniforms by name, at WGSL's own offsets — one
//! after another at the device's dynamic-offset alignment, and the whole tick goes up in one
//! `queue.write_buffer` before the tick's first submission. Each draw binds the buffer at its
//! own block's offset. The buffer is reused tick after tick and grows, never shrinks: a write
//! lands in queue order behind every submission already made, so a tick still on the GPU reads
//! its own values. Never a buffer per draw (`proposals/wgpu.md`, costs to watch).

use super::UniformValue;
use super::gpu::Gpu;
use crate::compile::wgsl::UniformLayout;
use std::sync::Arc;

/// One block's bytes: `layout.size` of them, the standard two and every value `layout` has a
/// member for. A value it has no member for — a texture, or a name the program before did not
/// know — is not written, and a member no value names stays zero.
pub fn pack(
    layout: &UniformLayout,
    (width, height): (u32, u32),
    time: f32,
    values: &[(Arc<str>, UniformValue)],
) -> Vec<u8> {
    let mut block = vec![0u8; layout.size as usize];
    let mut put = |offset: u32, bytes: &[u8]| {
        let at = offset as usize;
        block[at..at + bytes.len()].copy_from_slice(bytes);
    };
    for field in &layout.fields {
        match &*field.name {
            "u_resolution" => {
                put(field.offset, &(width as f32).to_ne_bytes());
                put(field.offset + 4, &(height as f32).to_ne_bytes());
            }
            "u_time" => put(field.offset, &time.to_ne_bytes()),
            name => match values.iter().find(|(n, _)| &**n == name).map(|(_, v)| v) {
                Some(UniformValue::Float(v)) => put(field.offset, &v.to_ne_bytes()),
                Some(UniformValue::Int(v)) => put(field.offset, &v.to_ne_bytes()),
                Some(UniformValue::Vec2(v)) => {
                    for (k, c) in v.iter().enumerate() {
                        put(field.offset + 4 * k as u32, &c.to_ne_bytes());
                    }
                }
                Some(UniformValue::Vec4(v)) => {
                    for (k, c) in v.iter().enumerate() {
                        put(field.offset + 4 * k as u32, &c.to_ne_bytes());
                    }
                }
                Some(UniformValue::NodeTexture(_)) | None => {}
            },
        }
    }
    block
}

/// The tick's blocks, and the buffer they go up in.
pub struct Arena {
    bytes: Vec<u8>,
    buffer: Option<wgpu::Buffer>,
    /// The device's `min_uniform_buffer_offset_alignment`.
    align: u32,
}

impl Arena {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            bytes: Vec::new(),
            buffer: None,
            align: gpu.device().limits().min_uniform_buffer_offset_alignment,
        }
    }

    /// Start a tick's blocks.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Add one block and say at which offset it will be.
    pub fn push(
        &mut self,
        layout: &UniformLayout,
        size: (u32, u32),
        time: f32,
        values: &[(Arc<str>, UniformValue)],
    ) -> u32 {
        let offset = (self.bytes.len() as u32).next_multiple_of(self.align);
        self.bytes.resize(offset as usize, 0);
        self.bytes.extend(pack(layout, size, time, values));
        offset
    }

    /// Put the tick's blocks in the buffer, growing it first where they do not fit. Queued
    /// ahead of the next submission.
    pub fn upload(&mut self, gpu: &Gpu) {
        if self.bytes.is_empty() {
            return;
        }
        let needed = self.bytes.len() as u64;
        if self.buffer.as_ref().is_none_or(|b| b.size() < needed) {
            self.buffer = Some(gpu.device().create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: needed.next_power_of_two().max(u64::from(self.align)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            gpu.queue().write_buffer(buffer, 0, &self.bytes);
        }
    }

    /// The buffer every block is bound from. `None` before the first upload.
    pub fn buffer(&self) -> Option<&wgpu::Buffer> {
        self.buffer.as_ref()
    }
}
