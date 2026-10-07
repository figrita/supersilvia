// SPDX-License-Identifier: AGPL-3.0-or-later

//! A small ring of render targets: where an Output and the mix draw, and what they publish.
//! CPU bookkeeping over the slots, with a completion serial saying which have finished
//! (`proposals/wgpu.md`, 1.3).
//!
//! **The frame is drawn where it is published.** Each frame goes straight into a free slot,
//! which becomes the ring's **latest**. What the synth reads — an Output sampling its own
//! `frame`, another Output sampling it, the mix sampling a deck — is the latest slot, ordered
//! by the queue. An Output reading its own `frame` while it draws reads the latest slot
//! *before* this frame's, which is feedback's one-frame delay, and never the slot it draws
//! into: wgpu forbids one texture as both a sampled binding and an attachment in one pass,
//! and the ring never asks for it.
//!
//! **A viewer is shown only what has finished.** A slot carries the serial of the submission
//! that last drew into it, and [`Ring::poll`] compares it with the completed serial. The
//! **shown** slot is the newest that has finished; it is the one a `Published` names.
//!
//! **A slot is drawn into again only when nothing can still read it**: not the latest, not the
//! shown one, not one whose [`Lease`] a `Published` still holds, and not one still being
//! drawn, which would be the next shown once it finishes. Five slots at most; a tick with none
//! free is skipped. A slot of an old size is dropped as soon as nothing reads it, finished on
//! the GPU or not: wgpu frees its memory once the GPU is done with it. See
//! [docs/rendering.md](../../docs/rendering.md#per-output-per-frame).

use super::gpu::Gpu;
use super::lease::Lease;
use super::shared::{self, Shared};

/// Maximum slots: latest, shown, one to draw, and two held by slower viewers.
pub const RING: usize = 5;

/// What a ring's targets hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `Rgba16Float`: an Output's frame, which feedback reads back into itself.
    Half,
    /// `Rgba8Unorm`: the mix, which only ever goes to a screen.
    Byte,
}

impl Format {
    pub fn texture_format(self) -> wgpu::TextureFormat {
        match self {
            Self::Half => wgpu::TextureFormat::Rgba16Float,
            Self::Byte => wgpu::TextureFormat::Rgba8Unorm,
        }
    }

    /// What this ring's "nothing" is: transparent black for a frame feedback reads, opaque
    /// black for the mix.
    fn blank(self) -> wgpu::Color {
        match self {
            Self::Half => wgpu::Color::TRANSPARENT,
            Self::Byte => wgpu::Color::BLACK,
        }
    }
}

/// One render target.
pub struct Slot {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    /// The claim every `Published` naming this slot holds a clone of.
    lease: Lease,
    /// The submission that last drew into this slot. `None` from the moment it is drawn
    /// until that submission is made.
    serial: Option<u64>,
    /// That submission has finished.
    done: bool,
    /// The ring's count when this slot was last drawn. Zero for a slot never drawn.
    drawn: u64,
    /// The synth tick that drew this frame.
    tick: u64,
}

pub struct Ring {
    slots: Vec<Slot>,
    format: Format,
    /// The size a new frame is drawn at.
    width: u32,
    height: u32,
    /// Frames drawn into this ring, counting every clear and carry as one.
    count: u64,
    tick: u64,
}

impl Ring {
    /// One slot, cleared into `encoder`, so there is a frame to read before anything has been
    /// drawn.
    pub fn new(
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
        format: Format,
    ) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let mut ring = Self {
            slots: Vec::with_capacity(RING),
            format,
            width,
            height,
            count: 0,
            tick: 0,
        };
        ring.slots.push(new_slot(gpu, width, height, format));
        shared::clear(encoder, &ring.slots[0].view, format.blank());
        ring.commit(0);
        ring
    }

    pub fn set_tick(&mut self, tick: u64) {
        self.tick = tick;
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn format(&self) -> Format {
        self.format
    }

    fn latest_index(&self) -> usize {
        // Never empty: `new` pushes one and nothing removes the latest.
        let mut best = 0;
        for (i, s) in self.slots.iter().enumerate() {
            if s.drawn > self.slots[best].drawn {
                best = i;
            }
        }
        best
    }

    fn shown_index(&self) -> Option<usize> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.done)
            .max_by_key(|(_, s)| s.drawn)
            .map(|(i, _)| i)
    }

    /// The slot drawn last. What feedback, another Output and the mix read.
    pub fn latest(&self) -> &Slot {
        &self.slots[self.latest_index()]
    }

    /// The count the latest slot was drawn at, which names that frame.
    pub fn latest_drawn(&self) -> u64 {
        self.latest().drawn
    }

    /// The newest slot whose drawing has finished on the GPU, and a claim on it. What a
    /// viewer is shown. `None` only until the ring's first clear has been found finished.
    pub fn shown(&self) -> Option<(&Slot, Lease)> {
        let s = &self.slots[self.shown_index()?];
        Some((s, s.lease.clone()))
    }

    pub fn shown_tick(&self) -> Option<u64> {
        Some(self.slots[self.shown_index()?].tick)
    }

    /// Every texture the ring holds. For a test asserting nothing was allocated.
    pub fn textures(&self) -> Vec<wgpu::Texture> {
        self.slots.iter().map(|s| s.texture.clone()).collect()
    }

    /// Mark every slot whose submission has finished as done. Never waits.
    pub fn poll(&mut self, completed: u64) {
        for s in &mut self.slots {
            if !s.done && s.serial.is_some_and(|serial| serial <= completed) {
                s.done = true;
            }
        }
    }

    /// Whether the frame drawn at `drawn` has finished.
    ///
    /// Asked of the oldest slot drawn at or after that frame: its own where it still holds
    /// it, and otherwise the one that replaced it — a frame blanked in place, or a slot freed —
    /// which was submitted after it and so finishes after it. A frame is never taken for
    /// finished because its slot has gone.
    pub fn finished(&self, drawn: u64, completed: u64) -> bool {
        self.slots
            .iter()
            .filter(|s| s.drawn >= drawn)
            .min_by_key(|s| s.drawn)
            .is_none_or(|s| s.serial.is_some_and(|serial| serial <= completed))
    }

    /// A slot at the ring's size that nothing can still read, making one if every slot is
    /// taken and the ring is below [`RING`]. `None` at the limit.
    ///
    /// A slot whose frame the GPU has not finished is not taken: it is the frame the next
    /// publish shows once it finishes, and a ring that kept drawing over it would show one
    /// frame for as long as the GPU stayed behind. Of several free slots the one drawn most
    /// recently is taken.
    pub fn free_slot(&mut self, gpu: &Gpu, completed: u64) -> Option<usize> {
        self.poll(completed);
        let (latest, shown) = (self.latest_index(), self.shown_index());
        let (width, height) = (self.width, self.height);
        let found = self
            .slots
            .iter_mut()
            .enumerate()
            .filter(|(i, s)| {
                *i != latest
                    && Some(*i) != shown
                    && (s.drawn == 0 || s.done)
                    && (s.width, s.height) == (width, height)
            })
            .filter_map(|(i, s)| s.lease.is_free().then_some((i, s.drawn)))
            .max_by_key(|(_, drawn)| *drawn)
            .map(|(i, _)| i);
        if found.is_some() {
            return found;
        }
        if self.slots.len() >= RING {
            self.free_stale();
        }
        if self.slots.len() >= RING {
            return None;
        }
        self.slots.push(new_slot(gpu, width, height, self.format));
        Some(self.slots.len() - 1)
    }

    /// Slot `i`, to draw into.
    pub fn slot(&self, i: usize) -> &Slot {
        &self.slots[i]
    }

    /// Slot `i` has been drawn: it becomes the latest, unfinished until its submission is
    /// made and has finished.
    pub fn commit(&mut self, i: usize) {
        self.count += 1;
        let s = &mut self.slots[i];
        s.drawn = self.count;
        s.tick = self.tick;
        s.serial = None;
        s.done = false;
    }

    /// Everything drawn since the last submission went in the one numbered `serial`.
    pub fn submitted(&mut self, serial: u64) {
        for s in &mut self.slots {
            if s.drawn > 0 && s.serial.is_none() {
                s.serial = Some(serial);
            }
        }
    }

    /// Make the latest frame black, in a slot of its own, and say whether it did.
    ///
    /// A ring at its limit clears its latest slot in place instead, where no viewer holds
    /// it. Where one does, nothing is cleared and the answer is no.
    pub fn blank(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, completed: u64) -> bool {
        let i = if let Some(i) = self.free_slot(gpu, completed) {
            i
        } else {
            let latest = self.latest_index();
            if !self.slots[latest].lease.is_free() {
                return false;
            }
            latest
        };
        shared::clear(encoder, &self.slots[i].view, self.format.blank());
        self.commit(i);
        true
    }

    /// Draw at a new size from now on, carrying the latest frame into it, and say whether it
    /// did.
    ///
    /// Zero flash: the carried frame is the latest from here, so feedback and a consumer read
    /// the last picture scaled rather than a fresh black target, and viewers go on being shown
    /// the old size until the carry has finished. A ring still at [`RING`] once what nothing
    /// reads has gone stays at the old size, and the next call asks again.
    pub fn resize(
        &mut self,
        gpu: &Gpu,
        shared: &Shared,
        encoder: &mut wgpu::CommandEncoder,
        width: u32,
        height: u32,
    ) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        if (self.width, self.height) == (width, height) {
            return false;
        }
        let old = (self.width, self.height);
        (self.width, self.height) = (width, height);
        self.free_stale();
        if self.slots.len() >= RING {
            (self.width, self.height) = old;
            return false;
        }
        let slot = new_slot(gpu, width, height, self.format);
        shared.carry(
            gpu,
            encoder,
            (&self.latest().view, old),
            &slot.view,
            (width, height),
            self.format,
        );
        self.slots.push(slot);
        self.commit(self.slots.len() - 1);
        true
    }

    /// Drop every slot of another size than the ring's that is neither the latest, the shown
    /// one nor leased — whether or not the GPU has finished it. It will never be drawn into
    /// again; a frame of the old size still being drawn is one a viewer is then never shown,
    /// and the one before it stays up until the new size's first has finished.
    fn free_stale(&mut self) {
        let latest = self.latest_drawn();
        let shown = self.shown_index().map(|i| self.slots[i].drawn);
        let (width, height) = (self.width, self.height);
        self.slots.retain_mut(|s| {
            let stale = (s.width, s.height) != (width, height);
            !stale || s.drawn == latest || Some(s.drawn) == shown || !s.lease.is_free()
        });
    }

    /// A copy of the latest frame, recorded into `encoder`, for [`Ring::put_back`]: what a
    /// render keeps of an Output's frame while it draws its own into the ring.
    pub fn keep(&self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder) -> wgpu::Texture {
        let latest = self.latest();
        let kept = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("kept frame"),
            size: latest.texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format.texture_format(),
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        encoder.copy_texture_to_texture(
            latest.texture.as_image_copy(),
            kept.as_image_copy(),
            latest.texture.size(),
        );
        kept
    }

    /// Make a frame [`Ring::keep`] copied the latest again, at its own size, which the ring
    /// draws at from here, and say whether it did. Copied, not scaled: the frame feedback
    /// reads next is the one kept, to the bit. A ring with no slot free for it and its latest
    /// held by a viewer says no, and the next call asks again.
    pub fn put_back(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        kept: &wgpu::Texture,
        completed: u64,
    ) -> bool {
        (self.width, self.height) = (kept.width(), kept.height());
        self.free_stale();
        let i = if let Some(i) = self.free_slot(gpu, completed) {
            i
        } else {
            let latest = self.latest_index();
            let s = &mut self.slots[latest];
            if !s.lease.is_free() || (s.width, s.height) != (self.width, self.height) {
                return false;
            }
            latest
        };
        encoder.copy_texture_to_texture(
            kept.as_image_copy(),
            self.slots[i].texture.as_image_copy(),
            kept.size(),
        );
        self.commit(i);
        true
    }

    /// Drop old-size slots that no viewer or feedback path can read. Once a tick.
    pub fn sweep(&mut self) {
        self.free_stale();
    }
}

/// One slot at this size and format.
fn new_slot(gpu: &Gpu, width: u32, height: u32, format: Format) -> Slot {
    let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("ring slot"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: format.texture_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Slot {
        texture,
        view,
        width,
        height,
        lease: Lease::default(),
        serial: None,
        done: false,
        drawn: 0,
        tick: 0,
    }
}
