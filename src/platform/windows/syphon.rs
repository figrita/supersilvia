// SPDX-License-Identifier: AGPL-3.0-or-later

//! Syphon on Windows: there is none, as on Linux. The directory is empty, a server or a client
//! is refused, and nothing here can be made, so every method is unreachable. Spout, which
//! shares a texture between apps on one Windows machine as Syphon does on a Mac, is the
//! counterpart, and is not written.

use crate::platform::syphon::{Described, Look, Stream};
use std::time::Duration;

/// Why each ask is refused.
const REFUSED: &str = crate::platform::syphon::MACOS_ONLY;

/// Why there is no Syphon here.
// Always `Some` here; the Option is the signature every backend answers `platform::syphon` with.
#[allow(clippy::unnecessary_wraps)]
pub fn unavailable() -> Option<&'static str> {
    Some(REFUSED)
}

/// No server is ever listed.
pub fn servers() -> Vec<Described> {
    Vec::new()
}

/// Nothing announces anything: waiting is all there is to do.
pub fn pump(time: Duration) {
    std::thread::sleep(time);
}

/// A shared surface, which no Windows build ever holds.
pub enum Surface {}

impl Surface {
    pub fn raw(&self) -> usize {
        match *self {}
    }

    pub fn size(&self) -> (u32, u32) {
        match *self {}
    }

    pub fn id(&self) -> u32 {
        match *self {}
    }

    pub fn pixel_format(&self) -> u32 {
        match *self {}
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        match *self {}
    }
}

/// A server, which is refused.
pub enum Server {}

impl Server {
    pub fn new(_name: &str) -> Result<Self, String> {
        Err(REFUSED.to_string())
    }

    pub fn surface(&self, _width: u32, _height: u32) -> Result<Surface, String> {
        match *self {}
    }

    pub fn publish(&self) {
        match *self {}
    }

    pub fn has_clients(&self) -> bool {
        match *self {}
    }

    pub fn rename(&self, _name: &str) {
        match *self {}
    }

    pub fn described(&self) -> Described {
        match *self {}
    }
}

/// A client, which is refused.
pub enum Client {}

impl Client {
    pub fn new(
        _server: &Described,
        _frames: impl Fn(Surface) + Send + 'static,
    ) -> Result<Self, String> {
        Err(REFUSED.to_string())
    }

    pub fn is_valid(&self) -> bool {
        match *self {}
    }
}

/// A client whose frames land in a stream's slots, which is refused.
pub enum Inlet {}

impl Inlet {
    pub fn open(_server: &Described, _look: Look) -> Result<Self, String> {
        Err(REFUSED.to_string())
    }

    pub fn stream(&self) -> Stream {
        match *self {}
    }

    pub fn is_valid(&self) -> bool {
        match *self {}
    }
}
