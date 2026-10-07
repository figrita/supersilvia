// SPDX-License-Identifier: AGPL-3.0-or-later

//! Help ▸ About supersilvia and Help ▸ Licences…: what the program is and under what terms,
//! and the notices of everything built into it or carried beside it.
//!
//! Two `egui::Window`s of ordinary widgets, like Preferences and the MIDI window, so both are
//! in the accessibility tree by construction. **Neither changes height with what it holds**:
//! About is the same few fixed lines every time, and Licences is one fixed frame whose text
//! scrolls inside it — twelve thousand lines of crate licences drawn as rows of one height,
//! wrapped once when a section is first opened and never again by egui, of which only the
//! rows in view are laid out.
//!
//! The Rust crates' notices are the machine's ([`crate::platform::notices`]): on Linux the
//! committed file `scripts/crate-licenses.py` writes, which `tests/notices.rs` holds to
//! Cargo.lock. The Mac keeps AppKit's own About panel in place of both windows; see
//! `platform::macos::menu`. docs/ui.md has the rest.

use crate::ui::theme::Theme;
use eframe::egui::{
    self, Align2, Color32, ColorImage, Context, FontId, RichText, ScrollArea, TextStyle,
    TextureHandle, TextureOptions, Ui, Window, vec2,
};
use std::fmt::Write as _;
use std::sync::OnceLock;

/// supersilvia's licence, whole: the notice and the section 7 permission, then the AGPL.
pub const LICENSE: &str = include_str!("../../LICENSE");

/// The repository's `licenses/` folder, file by file: the notices for the work this build
/// took from others. `tests/notices.rs` holds this list to the folder.
pub const ASSETS: &[(&str, &str)] = &[
    ("hack.txt", include_str!("../../licenses/hack.txt")),
    (
        "hash-without-sine.txt",
        include_str!("../../licenses/hash-without-sine.txt"),
    ),
    ("lucide.txt", include_str!("../../licenses/lucide.txt")),
    (
        "noto-emoji.txt",
        include_str!("../../licenses/noto-emoji.txt"),
    ),
    (
        "noto-sans-math.txt",
        include_str!("../../licenses/noto-sans-math.txt"),
    ),
    (
        "progress-pride.txt",
        include_str!("../../licenses/progress-pride.txt"),
    ),
    (
        "space-grotesk.txt",
        include_str!("../../licenses/space-grotesk.txt"),
    ),
    (
        "spatial-hash.txt",
        include_str!("../../licenses/spatial-hash.txt"),
    ),
    (
        "webgl-noise.txt",
        include_str!("../../licenses/webgl-noise.txt"),
    ),
];

/// The NDI® line and what it means for the runtime. The same on every machine.
pub const NDI: &str = "\
NDI®
====

NDI® is a registered trademark of Vizrt NDI AB. https://ndi.video

supersilvia sends and receives NDI through GStreamer's NDI plugin (gst-plugin-ndi, MPL-2.0,
listed under Rust crates), which opens the NDI runtime, libndi, when a sender or a receiver
starts. The runtime is not part of supersilvia and is never shipped with it: it is Vizrt's,
under Vizrt's own licence, and is installed from https://ndi.video by whoever uses it. The
additional permission at the head of supersilvia's licence, under section 7 of the GNU AGPL,
is what allows the two to run together.
";

/// The source, as `Cargo.toml` names it.
pub const SOURCE: &str = env!("CARGO_PKG_REPOSITORY");

/// Columns a notice is wrapped to. The Licences window is as wide as this many of the
/// monospace font's glyphs.
const COLUMNS: usize = 96;

/// What the About window asks for. It mutates nothing itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AboutAction {
    /// The Licences… button.
    OpenLicences,
    /// The window's close button.
    Close,
}

/// The About window.
pub fn show_about(
    ctx: &Context,
    theme: &Theme,
    saved: &crate::preferences::Placements,
) -> Vec<AboutAction> {
    let mut actions = Vec::new();
    let mut open = true;
    let (copyright, notice, heading, permission) = preamble();

    let window = Window::new(super::placed::ABOUT)
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .fixed_size(vec2(460.0, 0.0))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    super::placed::place(ctx, window, super::placed::ABOUT, saved).show(ctx, |ui| {
        ui.horizontal(|ui| {
            let mark = logo(ctx);
            ui.add(egui::Image::new(&mark).fit_to_exact_size(vec2(64.0, 64.0)));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("supersilvia").size(24.0).strong());
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(theme.text_secondary()),
                );
            });
        });
        ui.add_space(6.0);
        ui.label(env!("CARGO_PKG_DESCRIPTION"));
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        ui.label(RichText::new("Licence").strong());
        ui.label(copyright);
        ui.label(format!(
            "{} ({}). It comes with no warranty.",
            unwrap_lines(notice)
                .trim_end_matches(" The License follows this notice.")
                .trim_end_matches('.'),
            env!("CARGO_PKG_LICENSE"),
        ));
        ui.add_space(6.0);
        ui.label(RichText::new(heading).strong());
        ui.label(RichText::new(unwrap_lines(permission)).color(theme.text_secondary()));
        ui.add_space(6.0);
        ui.label(RichText::new("Source").strong());
        ui.hyperlink(SOURCE);
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        // The caption is a line of its own under the button, wrapped to the window's width.
        if ui.button("Licences…").clicked() {
            actions.push(AboutAction::OpenLicences);
        }
        ui.label(
            RichText::new("Every crate, font and library in it, and their terms")
                .color(theme.text_muted()),
        );
    });

    if !open {
        actions.push(AboutAction::Close);
    }
    actions
}

/// One tab of the Licences window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
    #[default]
    Supersilvia,
    RustCrates,
    Assets,
    GStreamer,
    Ndi,
}

impl Section {
    pub const ALL: [Self; 5] = [
        Self::Supersilvia,
        Self::RustCrates,
        Self::Assets,
        Self::GStreamer,
        Self::Ndi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Supersilvia => "supersilvia",
            Self::RustCrates => "Rust crates",
            Self::Assets => "Assets",
            Self::GStreamer => "GStreamer",
            Self::Ndi => "NDI®",
        }
    }

    /// The section's text, whole, before it is wrapped.
    pub fn text(self) -> String {
        match self {
            Self::Supersilvia => LICENSE.to_owned(),
            Self::RustCrates => crate::platform::notices::RUST_CRATES.to_owned(),
            Self::Assets => assets(),
            Self::GStreamer => crate::platform::notices::GSTREAMER.to_owned(),
            Self::Ndi => NDI.to_owned(),
        }
    }

    /// The section's text as the rows the window draws, wrapped once and kept.
    fn lines(self) -> &'static [String] {
        static LINES: [OnceLock<Vec<String>>; Section::ALL.len()] =
            [const { OnceLock::new() }; Section::ALL.len()];
        LINES[self as usize].get_or_init(|| wrap(&self.text(), COLUMNS))
    }
}

/// What the Licences window asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicencesAction {
    /// The window's close button.
    Close,
}

/// The Licences window's state across frames: which section is showing.
#[derive(Debug, Default)]
pub struct LicencesState {
    pub section: Section,
}

/// Which of Help's windows are open, and the Licences window's own state.
#[derive(Debug, Default)]
pub struct HelpWindows {
    pub about: bool,
    /// `None` is closed.
    pub licences: Option<LicencesState>,
    /// Help ▸ Keyboard shortcuts…, `F1`.
    pub shortcuts: bool,
}

/// The Licences window: a row of sections over one fixed frame of text that scrolls.
pub fn show_licences(
    ctx: &Context,
    state: &mut LicencesState,
    theme: &Theme,
    saved: &crate::preferences::Placements,
) -> Vec<LicencesAction> {
    let mut actions = Vec::new();
    let mut open = true;
    let font = TextStyle::Monospace.resolve(&ctx.global_style());
    let glyph = ctx.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let width = glyph * COLUMNS as f32 + 40.0;

    let window = Window::new(super::placed::LICENCES)
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        .fixed_size(vec2(width, 520.0))
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    super::placed::place(ctx, window, super::placed::LICENCES, saved).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for section in Section::ALL {
                if ui
                    .selectable_label(state.section == section, section.label())
                    .clicked()
                {
                    state.section = section;
                }
            }
        });
        ui.label(RichText::new(caption(state.section)).color(theme.text_secondary()));
        ui.separator();
        text(ui, state.section, &font, theme.text_primary());
    });

    if !open {
        actions.push(LicencesAction::Close);
    }
    actions
}

/// One line under the tabs saying what the section is.
fn caption(section: Section) -> &'static str {
    match section {
        Section::Supersilvia => "supersilvia's own licence: the GNU AGPL, version 3 or later",
        Section::RustCrates => "Every Rust crate compiled into this binary, with its licence",
        Section::Assets => "The notices carried over from silvia, for work it took from others",
        Section::GStreamer => "The media framework supersilvia plays and captures through",
        Section::Ndi => "The NDI® trademark, and the runtime supersilvia does not ship",
    }
}

/// The section's rows in a scroll area filling the rest of the window. Only the rows in view
/// are laid out, so the longest section costs what the shortest does.
fn text(ui: &mut Ui, section: Section, font: &FontId, ink: Color32) {
    let lines = section.lines();
    let row = ui.ctx().fonts_mut(|f| f.row_height(font));
    ScrollArea::both()
        .id_salt(section.label())
        .auto_shrink(false)
        .show_rows(ui, row, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.add(
                    egui::Label::new(RichText::new(line).font(font.clone()).color(ink)).extend(),
                );
            }
        });
}

/// The `licenses/` folder as one text, each file under its name.
fn assets() -> String {
    let mut out = String::from(
        "Assets\n======\n\nThe notices for the shader code, fonts and icons supersilvia takes \
         from others: the repository's licenses/ folder, file by file.\n",
    );
    for (name, text) in ASSETS {
        out.push('\n');
        out.push_str(&"-".repeat(78));
        let _ = write!(out, "\nlicenses/{name}\n\n{}\n", text.trim());
    }
    out
}

/// LICENSE's head, above the rule that opens the AGPL: the copyright line, the notice, the
/// section 7 heading, and the permission, each as the file has it.
pub fn preamble() -> (&'static str, &'static str, &'static str, &'static str) {
    let head = LICENSE
        .split("\n---")
        .next()
        .expect("split yields at least one piece");
    let mut paragraphs = head.split("\n\n").map(str::trim).filter(|p| !p.is_empty());
    let copyright = paragraphs.next().unwrap_or_default();
    let notice = paragraphs.next().unwrap_or_default();
    let heading = paragraphs.next().unwrap_or_default();
    let permission = paragraphs.next().unwrap_or_default();
    (copyright, notice, heading, permission)
}

/// A paragraph hard-wrapped in the file, as one line egui wraps to the window.
fn unwrap_lines(paragraph: &str) -> String {
    paragraph.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Hard-wrap `text` to `columns`: tabs to four spaces, a long line broken at spaces with its
/// indent carried to the lines it continues onto, and a word longer than a line cut.
pub fn wrap(text: &str, columns: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        let line = line.trim_end();
        if line.chars().count() <= columns {
            out.push(line.to_owned());
            continue;
        }
        // A rule of one repeated character is shortened, not continued onto a second line.
        let mut chars = line.chars();
        if let Some(first) = chars.next()
            && !first.is_alphanumeric()
            && !first.is_whitespace()
            && chars.all(|c| c == first)
        {
            out.push(line.chars().take(columns).collect());
            continue;
        }
        let indent: String = line.chars().take_while(|c| *c == ' ').collect();
        let indent = if indent.len() * 2 > columns {
            String::new()
        } else {
            indent
        };
        let mut current = indent.clone();
        for word in line.split(' ').filter(|w| !w.is_empty()) {
            let mut word = word.to_owned();
            loop {
                let used = current.chars().count();
                let room = columns - used;
                let fresh = current.len() == indent.len();
                let needed = word.chars().count() + usize::from(!fresh);
                if needed <= room {
                    if !fresh {
                        current.push(' ');
                    }
                    current.push_str(&word);
                    break;
                }
                if fresh {
                    // Longer than a whole line: cut it where the line ends.
                    let cut: String = word.chars().take(room).collect();
                    word = word.chars().skip(room).collect();
                    current.push_str(&cut);
                }
                out.push(std::mem::replace(&mut current, indent.clone()));
            }
        }
        if current.len() > indent.len() {
            out.push(current);
        }
    }
    out
}

/// The mark, decoded once and kept in egui's memory.
fn logo(ctx: &Context) -> TextureHandle {
    const PNG: &[u8] = include_bytes!("../../assets/icon/supersilvia-128.png");
    let id = egui::Id::new("about-logo");
    if let Some(texture) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return texture;
    }
    let image = image::load_from_memory(PNG)
        .expect("the mark is compiled in, so it decodes or the build is broken")
        .into_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let texture = ctx.load_texture(
        "about-logo",
        ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
        TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    texture
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About reads its licence lines out of LICENSE's head, so a LICENSE that loses that shape
    /// fails here rather than showing an empty paragraph.
    #[test]
    fn the_preamble_is_the_notice_the_heading_and_the_permission() {
        let (copyright, notice, heading, permission) = preamble();
        assert!(copyright.starts_with("Copyright (C) "));
        assert!(notice.starts_with("supersilvia is free software"));
        assert!(notice.contains("GNU Affero General Public License"));
        assert_eq!(
            heading,
            "Additional permission under GNU AGPL version 3 section 7"
        );
        assert!(permission.contains("NDI®"));
        assert!(permission.contains("Vizrt NDI AB"));
    }

    #[test]
    fn wrapping_keeps_every_word_and_every_line_fits() {
        let long = format!("  {}", "word ".repeat(60));
        let giant = "x".repeat(250);
        let text = format!("short\n\ttabbed\n{long}\n{giant}\n\nafter");
        let lines = wrap(&text, 40);
        assert!(lines.iter().all(|l| l.chars().count() <= 40), "{lines:#?}");
        assert_eq!(lines[0], "short");
        assert_eq!(lines[1], "    tabbed");
        assert!(lines[2].starts_with("  word"), "the indent is kept");
        assert!(lines[3].starts_with("  word"), "and carried on");
        let words: usize = lines.iter().map(|l| l.matches("word").count()).sum();
        assert_eq!(words, 60);
        let xs: usize = lines.iter().map(|l| l.matches('x').count()).sum();
        assert_eq!(xs, 250);
        assert_eq!(lines.last().map(String::as_str), Some("after"));
        assert!(lines.contains(&String::new()), "a blank line stays");
    }

    /// Every section wraps to the window, and the crates' is the machine's notices whole.
    #[test]
    fn every_section_fits_its_columns() {
        for section in Section::ALL {
            let lines = section.lines();
            assert!(!lines.is_empty(), "{section:?} is empty");
            assert!(
                lines.iter().all(|l| l.chars().count() <= COLUMNS),
                "{section:?} has a line past {COLUMNS} columns"
            );
        }
        assert_eq!(
            Section::RustCrates.text(),
            crate::platform::notices::RUST_CRATES
        );
    }
}
