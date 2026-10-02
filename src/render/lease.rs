// SPDX-License-Identifier: AGPL-3.0-or-later

//! A claim on a slot a viewer is shown, and nothing else.
//!
//! Keeping a texture alive and ordering a viewer's reads before the next draw are both wgpu's,
//! on one device: a `Published` holds texture
//! views, a view keeps its texture alive, and wgpu keeps whatever a submitted command buffer
//! names alive until the GPU is done; a viewer's blit and the synth's next draw into the slot
//! are ordered by the queue, since the synth draws into a slot only once its lease is free and
//! so submits after the viewer did. What is left is what a viewer is *shown*: no slot a held
//! `Published` names is drawn into, so a viewer never shows a frame that changes under it.
//! See `proposals/wgpu.md`, section 1.5.
//!
//! **A viewer holds its `Published` until after the `queue.submit` that carries its reads.**

use std::sync::Arc;

/// The owner keeps one; every `Published` naming the slot keeps a clone.
#[derive(Debug, Clone, Default)]
pub struct Lease(Arc<()>);

impl Lease {
    /// True when no `Published` names the slot any longer. Asked only on the thread that
    /// publishes, which is the only thread that makes new claims.
    pub fn is_free(&mut self) -> bool {
        Arc::get_mut(&mut self.0).is_some()
    }
}
