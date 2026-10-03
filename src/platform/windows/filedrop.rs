// SPDX-License-Identifier: AGPL-3.0-or-later

//! Nothing to do: winit hears a file dropped on the window through the shell's own drag and drop
//! and hands it to egui as `DroppedFile`. The names are Linux's, so the editor asks every
//! machine the same thing.

use std::path::PathBuf;

/// What a drag did over the editor's window. None arrives here.
#[derive(Debug, Clone, PartialEq)]
pub enum Drag {
    Entered { files: Vec<PathBuf>, at: (f32, f32) },
    Moved((f32, f32)),
    Left,
    Dropped { files: Vec<PathBuf>, at: (f32, f32) },
}

pub struct FileDrop;

impl FileDrop {
    pub fn none() -> Self {
        Self
    }

    pub fn for_window(
        _window: &(impl raw_window_handle::HasDisplayHandle + raw_window_handle::HasWindowHandle),
        _wake: impl Fn() + Send + 'static,
    ) -> Self {
        Self
    }

    pub fn take(&self) -> Vec<Drag> {
        Vec::new()
    }

    pub fn stop(&mut self) {}
}
