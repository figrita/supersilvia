// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NDI® runtime on Windows is left to the plugin: NDI's installer sets
//! `NDI_RUNTIME_DIR_V6` to the folder holding `Processing.NDI.Lib.x64.dll`, and the plugin
//! opens the runtime from that folder before it asks the loader, so nothing is opened ahead
//! of it.

use crate::platform::ndi::Preload;
use std::path::PathBuf;

/// Opens nothing: the plugin finds the runtime itself.
pub fn preload(_order: &[PathBuf]) -> Preload {
    Preload::Left
}
