// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the pointer is over a picture, in that picture's own world units.
//!
//! **Two sources, one port.** A picture in a window of its own is not egui's: on Linux it is a
//! Wayland surface and its pointer arrives through the pictures thread's own `wl_pointer`, and
//! on macOS it is a winit window whose pointer arrives on the main thread's event loop. The
//! mixer's preview and the canvas are ordinary egui regions and their pointer arrives through
//! egui. Both write a [`Reading`] into the one [`Feed`] and the tick reads the
//! newest, which is the shape every device here takes: nothing waits, and a surface the
//! pointer left says *nothing* rather than saying where the hand last was.
//!
//! `proposals/mouseinput.md` is the argument; the option that picks the surface is
//! `mouseinput`'s own.

/// Which surface a position is framed in.
///
/// The three a hand can be over, and the node's own option picks one. They are not a
//  ranking: a patch that wants the canvas says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Surface {
    /// A picture in a window of its own — a node's pop-out, or the mix. The default,
    /// because it is the picture the audience is looking at.
    Picture,
    /// The mixer's preview, in the panel beside the canvas.
    Preview,
    /// The editor canvas, which is what silvia measured against.
    Canvas,
}

impl Surface {
    /// Every surface, in the order the option lists them.
    pub const ALL: [Self; 3] = [Self::Picture, Self::Preview, Self::Canvas];

    /// The option's value for this surface.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Picture => "picture",
            Self::Preview => "preview",
            Self::Canvas => "canvas",
        }
    }

    /// The surface an option value names. Anything else is the default, which is the
    /// picture.
    pub fn of(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|s| s.key() == key)
            .unwrap_or(Self::Picture)
    }

    const fn index(self) -> usize {
        match self {
            Self::Picture => 0,
            Self::Preview => 1,
            Self::Canvas => 2,
        }
    }
}

/// The pointer over one surface: where it is in that picture's world units, and which
/// buttons are down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// Worldspace, so height is 2 and width is 2·aspect — the units a shape's center is in.
    pub x: f32,
    /// Worldspace, up positive, which is the opposite of every windowing system's y.
    pub y: f32,
    pub left: bool,
    pub right: bool,
}

/// The one slot the pointer's sources write and a tick reads.
///
/// A mutex held for a copy of three small structs and for nothing else. There is no queue:
/// a tick wants where the hand *is*, and a motion nobody read is a motion nobody needed.
#[derive(Default)]
pub struct Feed(std::sync::Mutex<[Option<Reading>; 3]>);

impl Feed {
    /// The pointer is here, or — with `None` — it has left this surface.
    pub fn set(&self, surface: Surface, reading: Option<Reading>) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)[surface.index()] = reading;
    }

    /// What every surface holds, for one tick.
    pub fn read(&self) -> Frame {
        Frame(
            *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// What the pointer was doing when this tick started. Taken once and handed to every node
/// that reads it, the way the Main Input's analysis is.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame([Option<Reading>; 3]);

impl Frame {
    /// One surface's reading, or `None` where the pointer is not over it.
    ///
    /// **`Picture` falls back to the preview**, which is the proposal's own rule: *a
    /// popped-out picture, else the mixer's preview*. A performer pops the mix out for the
    /// show and works against the preview before it, and the patch should not have to be
    /// edited between the two.
    pub fn of(self, surface: Surface) -> Option<Reading> {
        match surface {
            Surface::Picture => self.0[0].or(self.0[1]),
            Surface::Preview | Surface::Canvas => self.0[surface.index()],
        }
    }

    /// A reading on one surface, for a test feeding a synthetic pointer.
    #[must_use]
    pub fn with(mut self, surface: Surface, reading: Option<Reading>) -> Self {
        self.0[surface.index()] = reading;
        self
    }
}

/// Where `pos` falls inside a picture of `aspect` drawn letterboxed into a rect `size` big,
/// in worldspace — or `None` where it falls on a letterbox bar or outside the rect.
///
/// Both `pos` and `size` are in whatever units the surface uses; only their ratio matters.
/// The picture is centered and fills the shorter axis, which is what `Fit::Letterbox` does
/// on the GPU, so the number lands where the hand is rather than where the rect is.
pub fn place(pos: (f32, f32), size: (f32, f32), aspect: f32) -> Option<Reading> {
    if !(size.0 > 0.0 && size.1 > 0.0 && aspect.is_finite() && aspect > 0.0) {
        return None;
    }
    let target = size.0 / size.1;
    // The picture's own size inside the rect: the wider of the two loses height.
    let (w, h) = if aspect > target {
        (size.0, size.0 / aspect)
    } else {
        (size.1 * aspect, size.1)
    };
    let (left, top) = ((size.0 - w) * 0.5, (size.1 - h) * 0.5);
    let nx = (pos.0 - left) / w * 2.0 - 1.0;
    let ny = (pos.1 - top) / h * 2.0 - 1.0;
    if !(-1.0..=1.0).contains(&nx) || !(-1.0..=1.0).contains(&ny) {
        return None;
    }
    Some(Reading {
        x: nx * aspect,
        y: -ny,
        left: false,
        right: false,
    })
}

#[cfg(test)]
mod tests {
    use super::{Feed, Frame, Reading, Surface, place};

    /// The center of a picture is the origin, and its corners are ±aspect by ±1 with up
    /// positive — worldspace, which is what a shape's center is in.
    #[test]
    fn the_middle_of_a_picture_is_the_origin() {
        let mid = place((160.0, 45.0), (320.0, 90.0), 320.0 / 90.0).unwrap();
        assert!(mid.x.abs() < 1e-5 && mid.y.abs() < 1e-5, "{mid:?}");
        let top_left = place((0.0, 0.0), (320.0, 90.0), 320.0 / 90.0).unwrap();
        assert!((top_left.x + 320.0 / 90.0).abs() < 1e-4, "{top_left:?}");
        assert!((top_left.y - 1.0).abs() < 1e-5, "y is up: {top_left:?}");
    }

    /// A picture narrower than the rect it is drawn in has bars, and the bars are not the
    /// picture: a hand on one is off the surface.
    #[test]
    fn a_letterbox_bar_is_off_the_picture() {
        // A square picture in a 2:1 rect: a quarter of the width on each side is bar.
        assert!(place((10.0, 50.0), (400.0, 200.0), 1.0).is_none());
        let inside = place((200.0, 50.0), (400.0, 200.0), 1.0).unwrap();
        assert!(inside.x.abs() < 1e-5, "{inside:?}");
        assert!(place((-1.0, 50.0), (400.0, 200.0), 1.0).is_none());
    }

    /// The picture falls back to the preview and nothing else does: a patch built against
    /// the preview goes on reading when the mix is popped out, and a canvas asked for is
    /// the canvas or nothing.
    #[test]
    fn the_picture_falls_back_to_the_preview() {
        let preview = Reading {
            x: 0.5,
            ..Reading::default()
        };
        let frame = Frame::default().with(Surface::Preview, Some(preview));
        assert_eq!(frame.of(Surface::Picture), Some(preview));
        assert_eq!(frame.of(Surface::Canvas), None);

        let window = Reading {
            x: -0.25,
            ..Reading::default()
        };
        let frame = frame.with(Surface::Picture, Some(window));
        assert_eq!(frame.of(Surface::Picture), Some(window), "the window wins");
    }

    /// A surface the pointer left says nothing, rather than holding where the hand was.
    #[test]
    fn leaving_a_surface_clears_it() {
        let feed = Feed::default();
        feed.set(Surface::Canvas, Some(Reading::default()));
        assert!(feed.read().of(Surface::Canvas).is_some());
        feed.set(Surface::Canvas, None);
        assert!(feed.read().of(Surface::Canvas).is_none());
    }
}
