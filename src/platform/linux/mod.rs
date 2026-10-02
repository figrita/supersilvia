// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Linux backend: what a KDE or GNOME desktop on Wayland, PipeWire and Mesa offers.
//!
//! Each module is one service of [`crate::platform`]. `portal` is not a service: it is the
//! one tokio runtime every xdg-desktop-portal conversation happens on, which `files` and
//! `screen` share.
//!
//! **`unsafe` is allowed in one file**, `ndi`, where it is declared below: opening the NDI®
//! runtime by its path runs the library's initialisers, which `libloading` marks `unsafe`. Its
//! one block carries a `// SAFETY:` line. The crate root denies it everywhere else.

pub mod alert;
pub mod audio;
pub mod check;
pub mod dirs;
pub mod files;
pub mod fonts;
pub mod gpu;
pub mod menu;
pub mod midi;
#[allow(unsafe_code)]
pub mod ndi;
pub mod notices;
mod portal;
pub mod screen;
pub mod syphon;
pub mod video;
