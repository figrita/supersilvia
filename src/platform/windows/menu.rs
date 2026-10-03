// SPDX-License-Identifier: AGPL-3.0-or-later

//! No native menu bar: a Windows application draws its menus inside its own window, so the egui
//! bar is the one, as on Linux. A `Bar` that cannot be made keeps `App`'s one path through the
//! menus free of the target's name — `install` answers `None`, and nothing else can be reached.

use crate::platform::menu::Menu;

/// A menu bar the operating system draws. There is none here, so there is no value of it.
pub enum Bar {}

impl Bar {
    pub fn install(_wake: impl Fn() + 'static) -> Option<Self> {
        None
    }

    pub fn show(&mut self, _menus: &[Menu]) {
        match *self {}
    }

    pub fn take(&self) -> Vec<usize> {
        match *self {}
    }

    pub fn pasteboard(&self) -> String {
        match *self {}
    }
}
