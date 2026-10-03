// SPDX-License-Identifier: AGPL-3.0-or-later

//! The GPU's use on Windows: not read.
//!
//! Windows keeps a per-process count, the *GPU Engine* performance counters Task Manager reads,
//! one instance per process, adapter and engine. Reading them is a PDH query over a wildcard
//! path, expanded again whenever an engine appears, for a figure the Status box only shows
//! beside the GPU times the renderer measures itself. So [`Clients::read`] answers `None`,
//! which the Status box shows as a dash, and [`render_engine_ns`] zero, as on a Mac.

use crate::platform::gpu::Read;

/// What the dash stands for: the process's own share, as Task Manager would give it.
pub const WHOLE_GPU: bool = false;

/// Nothing to read.
#[derive(Debug, Default)]
pub struct Clients;

impl Clients {
    /// Finds nothing.
    pub fn scan() -> Self {
        Self
    }

    /// `None`: no counter is read.
    // The signature every backend answers `platform::gpu` with, over nothing to read.
    #[allow(clippy::unused_self)]
    pub fn read(&self) -> Option<Read> {
        None
    }
}

/// Zero: no counter is read.
pub fn render_engine_ns() -> u64 {
    0
}
