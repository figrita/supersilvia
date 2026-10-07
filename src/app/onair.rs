// SPDX-License-Identifier: AGPL-3.0-or-later

//! The on-air interlock: what Quit, Restart, Open and New stop, said before they stop it.
//!
//! **Unsaved edits are not the only thing a Quit loses.** A picture window on the projector,
//! the mix going out over NDI or Syphon, and a render halfway through all end on Quit, and
//! Open and New take the decks off the mix. So the one confirm stands in front of those too:
//! with no unsaved edits it asks only whether to stop the show, and with them it says so
//! under the edits it names. Escape on a picture window is not guarded: it is the key for
//! closing one.
//!
//! A render is canceled rather than abandoned: going on asks the synth to stop it, and what
//! was asked for happens once the render has ended, so the frames written stay whole.

use super::{App, Pending};

/// What is going out of the app right now, which a Quit, an Open or a New would stop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OnAir {
    /// The mix has a window of its own.
    pub mix_window: bool,
    /// How many other pictures have windows of their own.
    pub windows: usize,
    /// The mix is sent over NDI.
    pub ndi: bool,
    /// The mix is published over Syphon.
    pub syphon: bool,
    /// A render is running: frames written, and how many there will be.
    pub render: Option<(u32, u32)>,
}

impl OnAir {
    /// Nothing is going out, so there is nothing to ask about.
    pub fn is_quiet(&self) -> bool {
        !self.showing() && self.render.is_none()
    }

    /// The show is going out: a picture window, NDI or Syphon.
    fn showing(&self) -> bool {
        self.mix_window || self.windows > 0 || self.ndi || self.syphon
    }

    /// One sentence per thing going out, as the question says them.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.mix_window {
            lines.push("The mix is on a screen.".to_string());
        }
        match self.windows {
            0 => {}
            1 => lines.push("A picture is in a window of its own.".to_string()),
            n => lines.push(format!("{n} pictures are in windows of their own.")),
        }
        match (self.ndi, self.syphon) {
            (true, true) => lines.push("The mix is sent over NDI and Syphon.".to_string()),
            (true, false) => lines.push("The mix is sent over NDI.".to_string()),
            (false, true) => lines.push("The mix is published over Syphon.".to_string()),
            (false, false) => {}
        }
        if let Some((written, frames)) = self.render {
            lines.push(format!("A render is running (frame {written} / {frames})."));
        }
        lines
    }

    /// The question, where there are no unsaved edits: `Stop the show?`, `Quit and cancel
    /// it?`.
    pub(super) fn question(&self, pending: &Pending) -> String {
        match (self.showing(), self.render.is_some()) {
            (true, true) => "Stop the show and cancel the render?".to_string(),
            (true, false) => "Stop the show?".to_string(),
            (false, _) => match pending {
                Pending::Quit => "Quit and cancel it?".to_string(),
                Pending::Restart => "Restart and cancel it?".to_string(),
                Pending::OpenDialog | Pending::Open(_) => {
                    "Cancel it and open another project?".to_string()
                }
                Pending::NewDialog => "Cancel it and start a new project?".to_string(),
            },
        }
    }

    /// What Save and Discard also do, where there are unsaved edits too.
    pub fn going_on(&self) -> Option<&'static str> {
        match (self.showing(), self.render.is_some()) {
            (true, true) => Some("Save or Discard stops the show and cancels the render."),
            (true, false) => Some("Save or Discard stops the show."),
            (false, true) => Some("Save or Discard cancels the render."),
            (false, false) => None,
        }
    }
}

/// The button that goes on, named for what it goes on to, so it is never mistaken for the
/// menu entry that asked.
pub(super) fn go_on_label(pending: &Pending) -> &'static str {
    match pending {
        Pending::Quit => "Quit anyway",
        Pending::Restart => "Restart anyway",
        Pending::OpenDialog | Pending::Open(_) => "Open anyway",
        Pending::NewDialog => "New anyway",
    }
}

impl App {
    /// What a Quit, an Open or a New would stop. See [`OnAir`].
    pub fn on_air(&self) -> OnAir {
        let open = self.wall.open();
        let mix_window = open.iter().any(|p| p.picture == crate::ui::PopOut::Mix);
        OnAir {
            mix_window,
            windows: open.len() - usize::from(mix_window),
            ndi: self.sending.mix.ndi,
            syphon: self.sending.mix.syphon,
            render: self.rendering().then(|| {
                self.render_progress()
                    .map_or((0, 0), |p| (p.written, p.frames))
            }),
        }
    }

    /// Whether a Quit, an Open or a New has to ask first: unsaved edits, or the show going
    /// out.
    pub(super) fn must_ask(&self) -> bool {
        self.dirty() || !self.on_air().is_quiet()
    }

    /// **Test accessor.** Whether the question in front of a Quit, an Open or a New is up.
    #[doc(hidden)]
    pub fn asking_to_go_on(&self) -> bool {
        self.confirm.is_some()
    }

    /// A Quit, an Open or a New held for a render to finish canceling, done once it has.
    pub(super) fn after_render(&mut self, ui: &eframe::egui::Ui) {
        if self.waiting.is_some()
            && !self.rendering()
            && let Some(pending) = self.waiting.take()
        {
            self.do_pending(ui, pending);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_going_out_asks_nothing() {
        assert!(OnAir::default().is_quiet());
        assert!(OnAir::default().lines().is_empty());
        assert_eq!(OnAir::default().going_on(), None);
    }

    #[test]
    fn the_show_and_a_render_are_said_in_plain_words() {
        let air = OnAir {
            mix_window: true,
            windows: 2,
            ndi: true,
            syphon: false,
            render: Some((120, 300)),
        };
        assert!(!air.is_quiet());
        assert_eq!(
            air.lines(),
            [
                "The mix is on a screen.",
                "2 pictures are in windows of their own.",
                "The mix is sent over NDI.",
                "A render is running (frame 120 / 300).",
            ]
        );
        assert_eq!(
            air.question(&Pending::Quit),
            "Stop the show and cancel the render?"
        );
    }

    #[test]
    fn a_render_alone_asks_to_cancel_it() {
        let air = OnAir {
            render: Some((120, 300)),
            ..OnAir::default()
        };
        assert_eq!(air.question(&Pending::Quit), "Quit and cancel it?");
        assert_eq!(
            air.question(&Pending::NewDialog),
            "Cancel it and start a new project?"
        );
        assert_eq!(air.going_on(), Some("Save or Discard cancels the render."));
    }
}
