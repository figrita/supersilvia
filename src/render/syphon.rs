// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture published over Syphon on a Mac or Spout on Windows: the server, its shared
//! surface, and the blit that draws a published picture into it.
//!
//! **Our own drawing into the framework's surface.** A server is Syphon's base class, or a
//! Spout sender ([`crate::platform::syphon::Server`]); its surface is made a texture on the one device the
//! way a clip's frame is ([`super::dmabuf::surface_target`]), and a picture window's blit
//! ([`Viewer`]) draws the picture into it. So the bytes another app reads are the bytes a
//! picture window shows, 8-bit, at the picture's own size, and there is no second queue, no
//! raw command buffer and no copy. `proposals/syphon.md` is the argument.
//!
//! **Orientation is the convention's.** By default the surface holds the picture in the row
//! order its clients read it in, [`BOTTOM_FIRST`]: on a Mac the **bottom row first**, the way
//! OpenGL lays a texture out, which is what Syphon's clients have always read it as — Simple
//! Client, OBS, ofxSyphon — and on Windows the **top row first**, Direct3D's and so Spout's.
//! [`Look::flip`] writes it the other way up. `tests/syphon.rs` reads the surface back through
//! the framework's own client and holds both.
//!
//! **Alpha.** By default the picture is drawn over black and the surface is opaque, as a
//! picture window is; [`Look::transparent`] draws it over nothing, so the surface carries the
//! picture's own alpha, premultiplied as the graph holds it. Syphon defines no alpha convention,
//! and Core Animation and Metal composite premultiplied, so that is what a Mac app drawing the
//! surface over something expects; NDI, which defines straight, is unpremultiplied in
//! [`super::ndi`]. What a Spout sender carries is the Windows backend's
//! ([`crate::platform::syphon`]).
//!
//! **A frame is published once the GPU has drawn it**: [`Outlet::draw`] submits and hands back
//! the submission's ticket, and the caller calls [`Outlet::publish`] once that has finished, so
//! no client is told of a frame still being drawn. **A new size is zero flash**: the new
//! surface reaches clients with that publish, and until then they keep reading the old one,
//! which their own reference keeps.
//!
//! The thread that draws and publishes, and lets a server go only once its last frame is
//! drawn, is [`super::publish`]'s, which sends over NDI® the same way.

use super::publish::Look;
use super::{Gpu, Picture, Ticket, Viewer, viewer::Viewport};
use crate::platform::syphon::{BOTTOM_FIRST, Server, Surface};

/// The surface a server's clients read, and the texture drawn into it.
struct Drawn {
    /// Held for as long as the texture over its memory.
    _surface: Surface,
    view: wgpu::TextureView,
    size: (u32, u32),
}

/// One published picture: a server and what is drawn into its surface.
pub struct Outlet {
    server: Server,
    drawn: Option<Drawn>,
}

impl Outlet {
    /// A server named `name`, announced at once, with nothing drawn yet.
    pub fn new(name: &str) -> Result<Self, String> {
        Ok(Self {
            server: Server::new(name)?,
            drawn: None,
        })
    }

    /// The server, for its name and what the directory says of it.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Blit `picture` into the surface as `look` says, remaking the surface where the picture
    /// is another size, and submit. Publish once the ticket's submission has finished.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        viewer: &Viewer,
        picture: &Picture,
        look: Look,
    ) -> Result<Ticket, String> {
        let size = (picture.width.max(1), picture.height.max(1));
        if self.drawn.as_ref().is_none_or(|d| d.size != size) {
            let surface = self.server.surface(size.0, size.1)?;
            let texture =
                super::dmabuf::surface_target(gpu.device(), surface.raw(), size.0, size.1)?;
            self.drawn = Some(Drawn {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                _surface: surface,
                size,
            });
        }
        let drawn = self.drawn.as_ref().expect("made above");
        let mut encoder = gpu
            .device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("syphon"),
            });
        let ground = if look.transparent {
            wgpu::Color::TRANSPARENT
        } else {
            wgpu::Color::BLACK
        };
        super::shared::clear(&mut encoder, &drawn.view, ground);
        let mut shown = picture.clone();
        shown.flip = shown_flip(picture.flip, look);
        viewer.show(
            &mut encoder,
            &drawn.view,
            size,
            &shown,
            Viewport::whole(size),
            super::Fit::Letterbox,
            0.0,
        );
        Ok(gpu.submit([encoder.finish()]))
    }

    /// Tell the clients the surface holds a new frame: call once the GPU has finished the
    /// submission [`Self::draw`] handed back.
    pub fn publish(&self) {
        self.server.publish();
    }
}

/// Whether `look` writes the surface bottom row first: the convention's order, the other way
/// up under [`Look::flip`].
fn bottom_first(look: Look) -> bool {
    BOTTOM_FIRST != look.flip
}

/// The [`Picture::flip`] the blit is handed for a picture whose own is `flip`. The blit writes a
/// picture's top row first ([`Viewer::show`]); handed the other value, it draws the rows as
/// though they were the other way up, so writes the bottom row first.
fn shown_flip(flip: bool, look: Look) -> bool {
    flip != bottom_first(look)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flip off writes the convention's row order — bottom first on a Mac, top first on
    /// Windows — and Flip on the other, whichever way up the picture's own texture is.
    #[test]
    fn the_surface_is_the_conventions_way_up_unless_flipped() {
        for flip in [false, true] {
            for transparent in [false, true] {
                let look = Look { flip, transparent };
                assert_eq!(bottom_first(look), BOTTOM_FIRST != flip);
                for own in [false, true] {
                    // The blit writes top row first when handed the picture's own flip.
                    let written_bottom_first = shown_flip(own, look) != own;
                    assert_eq!(written_bottom_first, BOTTOM_FIRST != flip);
                }
            }
        }
        assert_eq!(BOTTOM_FIRST, cfg!(target_os = "macos"));
    }
}
