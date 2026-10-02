// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the editor's own windows open: where the person last left each one.
//!
//! egui keeps a window's place in its own memory, which lives for the run. What outlives it is
//! `preferences.json`'s `windows`, by title: [`read`] takes each window's outer rect out of
//! egui's memory once a frame, and [`place`] hands it back as the window's default the next
//! time it is built, so the first frame of a new run opens it where the last run left it. A
//! resizable window keeps its size too; one that sizes itself to its content keeps only its
//! corner. See [docs/ui.md](../../docs/ui.md#preferences).

use crate::preferences::{Placement, Placements};
use eframe::egui::{Align2, Context, Id, Window};

/// The Status box's title, which is its id.
pub const STATUS: &str = "Status box";
/// The Preferences window's.
pub const PREFERENCES: &str = "Preferences";
/// The MIDI window's.
pub const MIDI: &str = "MIDI";
/// Help ▸ About supersilvia's.
pub const ABOUT: &str = "About supersilvia";
/// Help ▸ Licences…'s.
pub const LICENCES: &str = "Licences";
/// Help ▸ Keyboard shortcuts…'s.
pub const SHORTCUTS: &str = "Keyboard shortcuts";
/// Edit ▸ Undo History…'s.
pub const UNDO_HISTORY: &str = "Undo History";

/// Every window whose place is kept, and whether its size is kept with it.
const KEPT: [(&str, bool); 7] = [
    (STATUS, true),
    (PREFERENCES, false),
    (MIDI, true),
    (ABOUT, false),
    (LICENCES, false),
    (SHORTCUTS, false),
    (UNDO_HISTORY, false),
];

/// The window, opening where `saved` says this title was left, if it says so and egui has not
/// laid it out yet this run. Its corner is the saved one whatever pivot it was built with. Its
/// id is the title's own, which is what [`read`] finds it by.
pub fn place<'a>(ctx: &Context, window: Window<'a>, title: &str, saved: &Placements) -> Window<'a> {
    let id = Id::new(title);
    let window = window.id(id);
    if ctx.memory(|m| m.area_rect(id)).is_some() {
        return window;
    }
    let Some(placement) = saved.get(title) else {
        return window;
    };
    let window = window.pivot(Align2::LEFT_TOP).default_pos(placement.pos);
    match placement.size {
        Some(size) => window.default_size(size),
        None => window,
    }
}

/// Where each kept window is now, as egui last laid it out, in whole points. A window this run
/// has not shown, or one not laid out yet, is not in it.
pub fn read(ctx: &Context) -> Vec<(&'static str, Placement)> {
    KEPT.iter()
        .filter_map(|&(title, sized)| {
            let rect = ctx.memory(|m| m.area_rect(Id::new(title)))?;
            (rect.width() >= 1.0 && rect.height() >= 1.0).then(|| {
                (
                    title,
                    Placement {
                        pos: [rect.min.x.round(), rect.min.y.round()],
                        size: sized.then(|| [rect.width().round(), rect.height().round()]),
                    },
                )
            })
        })
        .collect()
}
