// SPDX-License-Identifier: AGPL-3.0-or-later

//! supersilvia — modular video synthesizer for live performance.
//!
//! `render::picture`, `render::dmabuf`, on Linux `platform::linux::ndi`, and on macOS
//! `platform::macos::audio`, `platform::macos::screen`, `platform::macos::pixels`,
//! `platform::macos::gpu`, `platform::macos::menu` and `platform::macos::syphon`, are the only
//! modules allowed `unsafe`, each where its parent declares it (`tests/rules.rs`), and every
//! block there carries a `// SAFETY:` line. Everywhere else it is denied.

#![deny(unsafe_code)]

pub mod app;
pub mod audio;
pub mod check;
pub mod clock;
pub mod command;
pub mod compile;
pub mod gamepad;
pub mod graph;
pub mod maininput;
pub mod midi;
pub mod mixer;
pub mod nodes;
pub mod platform;
pub mod pointer;
pub mod preferences;
pub mod project;
pub mod synth;
pub mod transport;
pub mod ui;
pub mod video;
pub mod widgets;
pub mod workspace;

/// The renderer, on wgpu. Its `unsafe` is the picture windows' borrowed display and the
/// surfaces made on it, and the DMA-BUF and `IOSurface` imports, allowed on those two modules
/// alone.
pub mod render;

pub use app::App;
pub use clock::Clock;
pub use command::{Command, CommandError};
pub use graph::{Graph, NodeId};
pub use synth::Synth;
