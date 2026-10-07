// SPDX-License-Identifier: AGPL-3.0-or-later

//! The design tokens, ported from the silvia design system.
//!
//! Everything is derived from four HSL anchors — main UI, number, color, event — plus a
//! saturation ladder. Neutrals are never gray: they are the theme hue at 5-25% of the theme
//! saturation, which is why setting `main.s` to zero yields a genuinely grayscale theme
//! rather than a tinted one. Pick four hues and the whole editor re-tints.

use eframe::egui::{Color32, CornerRadius, Stroke, Visuals};
use serde::{Deserialize, Serialize};

/// One HSL anchor. `s` and `l` are 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

impl Hsl {
    pub const fn new(h: f32, s: f32, l: f32) -> Self {
        Self { h, s, l }
    }

    #[must_use]
    pub fn with(self, s: f32, l: f32) -> Self {
        Self { h: self.h, s, l }
    }

    /// Scale lightness, as the CSS does with `calc(var(--theme-light) * 1.4)`.
    #[must_use]
    pub fn lighten(self, factor: f32) -> Self {
        Self {
            l: (self.l * factor).clamp(0.0, 1.0),
            ..self
        }
    }

    #[must_use]
    pub fn saturate(self, factor: f32) -> Self {
        Self {
            s: (self.s * factor).clamp(0.0, 1.0),
            ..self
        }
    }

    pub fn color(self) -> Color32 {
        self.color_alpha(255)
    }

    pub fn color_alpha(self, a: u8) -> Color32 {
        let (h, s, l) = (
            self.h.rem_euclid(360.0),
            self.s.clamp(0.0, 1.0),
            self.l.clamp(0.0, 1.0),
        );
        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let hp = h / 60.0;
        let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = l - c / 2.0;
        let q = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
        Color32::from_rgba_unmultiplied(q(r), q(g), q(b), a)
    }

    /// The inverse of [`Self::color`], for the one direction that needs it: a color picked
    /// out of the `s-color` popup, which speaks RGB, becoming one of the four anchors, which
    /// are HSL.
    ///
    /// A gray has no hue to recover — every hue produces it — so it keeps the one it came in
    /// with. Without that, dragging an anchor's saturation to zero in the picker would throw
    /// the hue away and dragging back up would come back a different color, which is the
    /// same trap the picker's own square carries its angles across frames to avoid.
    pub fn from_rgb(r: f32, g: f32, b: f32, keep_hue: f32) -> Self {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = f32::midpoint(max, min);
        let c = max - min;
        if c <= f32::EPSILON {
            return Self::new(keep_hue, 0.0, l);
        }
        let s = c / (1.0 - (2.0 * l - 1.0).abs()).max(f32::EPSILON);
        let h = if max == r {
            60.0 * (((g - b) / c) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / c + 2.0)
        } else {
            60.0 * ((r - g) / c + 4.0)
        };
        Self::new(h.rem_euclid(360.0), s.clamp(0.0, 1.0), l)
    }
}

/// The four themeable anchors, plus everything derived from them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub main: Hsl,
    pub number: Hsl,
    pub color: Hsl,
    pub event: Hsl,
}

impl Default for Theme {
    /// The canonical "vapor" preset: cyan UI on near-black, violet number ports, magenta
    /// color ports, amber event ports.
    fn default() -> Self {
        Self {
            main: Hsl::new(186.0, 0.84, 0.58),
            number: Hsl::new(277.0, 0.84, 0.58),
            color: Hsl::new(330.0, 0.81, 0.60),
            event: Hsl::new(45.0, 0.91, 0.57),
        }
    }
}

/// One named look: four anchors under a name and the emoji silvia files it under.
///
/// A preset is data the person picks, not a second source of truth. Choosing one writes its
/// four numbers into the preference and nothing else happens — there is no "current preset"
/// to fall out of step with the anchors, because the moment an anchor is dragged the four
/// numbers are simply no longer any preset's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    /// Stable, lowercase, and what a file would be named if these ever become files.
    pub key: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub theme: Theme,
}

/// [`PRESETS`] is written with these: a name, then each anchor in turn.
impl Preset {
    const fn new(key: &'static str, label: &'static str, icon: &'static str) -> Self {
        let black = Hsl::new(0.0, 0.0, 0.0);
        Self {
            key,
            label,
            icon,
            theme: Theme {
                main: black,
                number: black,
                color: black,
                event: black,
            },
        }
    }

    const fn main(mut self, h: f32, s: f32, l: f32) -> Self {
        self.theme.main = Hsl::new(h, s, l);
        self
    }

    const fn number(mut self, h: f32, s: f32, l: f32) -> Self {
        self.theme.number = Hsl::new(h, s, l);
        self
    }

    const fn color(mut self, h: f32, s: f32, l: f32) -> Self {
        self.theme.color = Hsl::new(h, s, l);
        self
    }

    const fn event(mut self, h: f32, s: f32, l: f32) -> Self {
        self.theme.event = Hsl::new(h, s, l);
        self
    }
}

/// silvia's sixteen looks, from `js/settings.js`, as four anchors each.
///
/// Ported rather than reinvented: sixteen is the right number because a look is *chosen*,
/// the way a VJ chooses one, and a list this long is browsed rather than read. silvia gives
/// each preset six anchors; the two this editor has no counterpart for — `audio` and `midi`,
/// which are port types there and not here — are dropped rather than invented a use for.
///
/// `vapor` is the exception to "ported verbatim": it carries this editor's own four numbers,
/// which are silvia's vapor rounded when the palette was first brought across. Keeping them
/// means [`Theme::default`] *is* a preset rather than something near one, so a person who
/// wanders off the default can get back to it by name.
pub const PRESETS: [Preset; 16] = [
    Preset::new("vanilla", "Vanilla", "🍦")
        .main(317.0, 0.91, 0.57)
        .number(160.0, 0.84, 0.39)
        .color(38.0, 0.92, 0.50)
        .event(258.0, 0.90, 0.66),
    Preset::new("mountain", "Mountain", "🏔️")
        .main(225.0, 0.36, 0.59)
        .number(157.0, 0.74, 0.24)
        .color(0.0, 0.72, 0.35)
        .event(22.0, 0.78, 0.26),
    Preset::new("honey", "Honey", "🍯")
        .main(38.0, 0.92, 0.50)
        .number(45.0, 0.93, 0.47)
        .color(21.0, 0.90, 0.48)
        .event(35.0, 0.92, 0.33),
    Preset::new("vapor", "Vapor", "💜")
        .main(186.0, 0.84, 0.58)
        .number(277.0, 0.84, 0.58)
        .color(330.0, 0.81, 0.60)
        .event(45.0, 0.91, 0.57),
    Preset::new("gothic", "Gothic", "🦇")
        .main(220.0, 0.09, 0.46)
        .number(215.0, 0.14, 0.34)
        .color(0.0, 1.00, 0.28)
        .event(215.0, 0.28, 0.17),
    Preset::new("av", "AV", "👨‍🔧")
        .main(0.0, 0.00, 0.46)
        .number(0.0, 0.00, 1.00)
        .color(48.0, 0.96, 0.53)
        .event(0.0, 0.96, 0.59),
    Preset::new("graphite", "Graphite", "✏️")
        .main(216.0, 0.19, 0.51)
        .number(210.0, 0.20, 0.98)
        .color(189.0, 0.94, 0.43)
        .event(0.0, 0.84, 0.60),
    Preset::new("paprika", "Paprika", "🌶️")
        .main(15.0, 0.90, 0.56)
        .number(50.0, 0.98, 0.64)
        .color(160.0, 0.84, 0.39)
        .event(17.0, 0.89, 0.60),
    Preset::new("rogue", "Rogue", "🗡️")
        .main(0.0, 0.66, 0.53)
        .number(350.0, 0.89, 0.60)
        .color(50.0, 0.98, 0.64)
        .event(175.0, 0.77, 0.26),
    Preset::new("blueprint", "Blueprint", "📐")
        .main(213.0, 0.84, 0.43)
        .number(50.0, 0.98, 0.64)
        .color(212.0, 0.96, 0.78)
        .event(221.0, 0.39, 0.64),
    Preset::new("hazard", "Hazard", "⚠️")
        .main(0.0, 0.00, 0.50)
        .number(48.0, 0.96, 0.53)
        .color(0.0, 0.84, 0.60)
        .event(142.0, 0.71, 0.45),
    Preset::new("aubergine", "Aubergine", "🍆")
        .main(274.0, 0.84, 0.51)
        .number(350.0, 0.89, 0.60)
        .color(158.0, 0.64, 0.52)
        .event(221.0, 0.39, 0.64),
    Preset::new("coralreef", "Coral reef", "🪸")
        .main(351.0, 0.95, 0.71)
        .number(199.0, 0.89, 0.48)
        .color(163.0, 0.88, 0.20)
        .event(221.0, 0.39, 0.64),
    Preset::new("polar", "Polar", "📄")
        .main(204.0, 1.00, 0.97)
        .number(199.0, 0.89, 0.48)
        .color(221.0, 0.39, 0.64)
        .event(142.0, 0.71, 0.45),
    Preset::new("forest", "Forest", "🌲")
        .main(142.0, 0.64, 0.39)
        .number(83.0, 0.78, 0.55)
        .color(25.0, 0.95, 0.53)
        .event(160.0, 0.84, 0.39),
    Preset::new("peachy", "Peachy", "🍑")
        .main(27.0, 0.96, 0.61)
        .number(26.0, 0.90, 0.37)
        .color(350.0, 0.89, 0.60)
        .event(160.0, 0.84, 0.39),
];

/// The saturation ladder. Neutrals rise in saturation as they become more interactive.
impl Theme {
    fn sat(&self, fraction: f32) -> f32 {
        self.main.s * fraction
    }

    fn neutral(&self, fraction: f32, l: f32) -> Color32 {
        self.main.with(self.sat(fraction), l).color()
    }

    pub fn primary(&self) -> Color32 {
        self.main.color()
    }
    pub fn primary_muted(&self) -> Color32 {
        self.main.saturate(0.50).lighten(0.8).color()
    }
    pub fn accent(&self) -> Color32 {
        Hsl::new(self.main.h + 30.0, self.sat(0.50), 0.55).color()
    }

    pub fn bg_primary(&self) -> Color32 {
        self.neutral(0.15, 0.12)
    }
    pub fn bg_secondary(&self) -> Color32 {
        self.neutral(0.08, 0.16)
    }
    pub fn bg_tertiary(&self) -> Color32 {
        self.neutral(0.10, 0.20)
    }
    pub fn bg_interactive(&self) -> Color32 {
        self.neutral(0.20, 0.25)
    }
    pub fn bg_hover(&self) -> Color32 {
        self.neutral(0.25, 0.30)
    }
    pub fn bg_active(&self) -> Color32 {
        self.neutral(0.25, 0.35)
    }
    /// Node body and floating chrome sit below `bg_primary`.
    pub fn bg_sunken(&self) -> Color32 {
        self.neutral(0.20, 0.10)
    }
    /// Node header, darker still.
    pub fn bg_header(&self) -> Color32 {
        self.neutral(0.25, 0.05)
    }

    /// An Output's slot before its first frame: a screen that is off.
    pub fn screen_off(&self) -> Color32 {
        self.neutral(0.25, 0.0)
    }

    pub fn border_subtle(&self) -> Color32 {
        self.neutral(0.15, 0.25)
    }
    pub fn border_normal(&self) -> Color32 {
        self.neutral(0.20, 0.35)
    }
    pub fn border_strong(&self) -> Color32 {
        self.neutral(0.25, 0.45)
    }

    pub fn text_primary(&self) -> Color32 {
        self.neutral(0.10, 0.95)
    }
    pub fn text_secondary(&self) -> Color32 {
        self.neutral(0.08, 0.80)
    }
    pub fn text_muted(&self) -> Color32 {
        self.neutral(0.05, 0.60)
    }
    pub fn text_disabled(&self) -> Color32 {
        self.neutral(0.05, 0.40)
    }

    /// The canvas dot grid.
    ///
    /// Bright enough to read as a grid. It was a tenth of an alpha step off the ground and
    /// only legible on a dark theme at full brightness — and the grid is what tells you where
    /// the plane is and how far you have panned, so a grid you have to hunt for is doing none
    /// of its job.
    pub fn grid_dot(&self) -> Color32 {
        self.main.with(self.sat(0.28), 0.55).color_alpha(64)
    }

    /// The anchor for one port type.
    pub fn port_anchor(&self, ty: crate::graph::PortType) -> Hsl {
        let anchor = match ty.kind() {
            crate::graph::Kind::Number => self.number,
            crate::graph::Kind::Color => self.color,
            crate::graph::Kind::Event => self.event,
        };
        // A uniform is its kind's color exactly; the diamond alone says which rate it is,
        // so the two rates read as one family under any theme.
        let _ = ty.rate();
        anchor
    }

    /// The color of one audio band, `0` low to `2` high: red, green, blue.
    ///
    /// **The one place a literal color is right.** Everything else in the editor derives from
    /// the four anchors so a re-theme moves it; these three do not, because they are not
    /// decoration. Low-mid-high as red-green-blue is silvia's convention and a convention is
    /// what makes a dot on a meter legible without a label — the same reason dashed *means*
    /// action. A themed triple would rotate with the anchors and stop meaning anything.
    pub fn band(&self, index: usize) -> Color32 {
        match index {
            0 => Color32::from_rgb(0xff, 0x66, 0x66),
            1 => Color32::from_rgb(0x66, 0xff, 0x66),
            _ => Color32::from_rgb(0x66, 0x66, 0xff),
        }
    }

    pub fn port(&self, ty: crate::graph::PortType) -> Color32 {
        self.port_anchor(ty).color()
    }

    /// An unconnected port's interior: a hole rather than empty canvas.
    ///
    /// The ring around it is the port's own hue at full strength, so the inside is that same
    /// hue sunk far enough to read as a shadow — a hole punched in the body, not a gap in it.
    /// Silvia's dots are filled solid; this is that shape's answer to "nothing is plugged in
    /// here" without falling back to transparent canvas or a flat neutral, which is what a
    /// second, smaller dot inside the ring used to stand in for.
    pub fn port_hole(&self, ty: crate::graph::PortType) -> Color32 {
        let a = self.port_anchor(ty);
        a.with(a.s * 0.5, 0.12).color()
    }

    /// The value a uniform number port published, drawn on its row.
    ///
    /// The uniform number port's own hue, sunk to a neutral label's weight: a readout
    /// belongs to the dot it is measured at, and a number as bright as the port would
    /// compete with it.
    pub fn readout(&self) -> Color32 {
        let a = self.port_anchor(crate::graph::PortType::UniformNumber);
        a.with(a.s * 0.65, 0.66).color()
    }

    /// The ground of a cross-workspace tag: the port's own hue, sunk far enough to sit
    /// under its own label.
    pub fn tag_fill(&self, ty: crate::graph::PortType) -> Color32 {
        let a = self.port_anchor(ty);
        a.with(a.s * 0.55, 0.16).color()
    }

    /// Its outline, a step brighter than the ground and dimmer than the label.
    pub fn tag_border(&self, ty: crate::graph::PortType) -> Color32 {
        let a = self.port_anchor(ty);
        a.with(a.s * 0.70, 0.40).color()
    }

    /// Its label, in the cable's own color: the pill stands for the cable that is not
    /// drawn, so it is colored like one.
    pub fn tag_text(&self, ty: crate::graph::PortType) -> Color32 {
        self.wire(ty)
    }

    /// Wires are a lighter, less saturated version of their port, so a dense graph reads as
    /// cables over nodes rather than a second set of dots.
    pub fn wire(&self, ty: crate::graph::PortType) -> Color32 {
        let a = self.port_anchor(ty);
        match ty {
            crate::graph::PortType::Action => a.saturate(0.7).lighten(1.4).color(),
            _ => a.saturate(0.8).lighten(1.2).color(),
        }
    }
}

/// Radii, from the design system: sharp everywhere except nodes, which are pillowy.
pub const RADIUS_SHARP: u8 = 2;
pub const RADIUS_SM: u8 = 4;
pub const RADIUS_MD: u8 = 6;
pub const RADIUS_LG: u8 = 12;

/// The cross-workspace tag: the pill beside an input port whose cable comes from a node
/// that is not on this workspace. Sharp like everything that is not a node, and sized from
/// the port row it sits in.
pub const TAG_HEIGHT: f32 = 18.0;
/// Space either side of a tag's label.
pub const TAG_PAD: f32 = 6.0;
/// Clear space between the port dot and the pill's near edge, and between two pills on one
/// port.
pub const TAG_GAP: f32 = 12.0;
/// The widest a tag gets, however long the workspace's name is. A longer one is elided.
pub const TAG_MAX_WIDTH: f32 = 132.0;
/// What a tag naming a **closed** workspace is multiplied by. Closed is still in the
/// project, so the tag is quieter rather than absent.
pub const TAG_CLOSED: f32 = 0.45;

/// Base font size. silvia sets `html { font-size: 12px }` and scales in rem from there.
pub const FONT_BASE: f32 = 12.0;
pub const FONT_TINY: f32 = FONT_BASE * 0.8;
/// A side panel's name in its header bar, one step above the text under it.
pub const FONT_TITLE: f32 = 13.0;
/// A section's heading inside a panel: small capitals' worth of size, in the strong face.
pub const FONT_SECTION: f32 = 10.5;

/// The family of the strong face, Space Grotesk SemiBold: headings, and a name that has to stand out
/// from the text around it. egui picks a face by family rather than by weight, so the weight
/// is a family of its own.
pub const STRONG: &str = "strong";

/// The text face at `size`: Space Grotesk, with tabular digits.
pub fn ui_font(size: f32) -> eframe::egui::FontId {
    eframe::egui::FontId::proportional(size)
}

/// The strong face at `size`: Space Grotesk SemiBold.
pub fn strong_font(size: f32) -> eframe::egui::FontId {
    eframe::egui::FontId::new(size, eframe::egui::FontFamily::Name(STRONG.into()))
}

/// How much larger an icon is drawn than the text beside it, in points before zoom.
///
/// An emoji at the text's own size reads as small next to it: the glyph sits inside a square
/// em box with its own padding, where a letter's fills the line it is on. Giving the icon
/// the difference back is what makes a header or a menu entry look like one object rather
/// than a small picture next to some words.
pub const ICON_BUMP: f32 = 4.0;

/// The font for an icon standing beside text of size `base`. Proportional, because the icon
/// faces are, and a monospace cell would letterbox it.
pub fn icon_font(base: f32, zoom: f32) -> eframe::egui::FontId {
    eframe::egui::FontId::proportional(font_size(base + ICON_BUMP, zoom))
}

/// A canvas font size, quantized to whole pixels.
///
/// egui caches glyph rasterization per size, so a continuously varying size — which is what
/// `base * zoom` is during a smooth zoom — misses the cache every frame and re-lays-out and
/// re-rasterizes every glyph on screen. Rounding bounds the whole zoom range to a few dozen
/// sizes, all cached after first use. The cost is that text steps rather than glides while
/// zooming, which is the normal behavior of every editor that does this.
pub fn font_size(base: f32, zoom: f32) -> f32 {
    (base * zoom).round().max(6.0)
}

/// What the editor's window is cleared to before egui paints a frame: the ground every panel
/// and the canvas stand on, opaque, in the gamma-space floats eframe hands `glClear`.
///
/// A clear is nearly free where a fill is not: each full-window fill is a read and a write
/// of every pixel it covers, and the panels' grounds and the canvas's are a screenful of
/// them. With the window cleared to this color,
/// [`apply_cleared`] takes the panels' fills away and the canvas leaves its ground out, and
/// the pixels are the ones the fills wrote.
pub fn clear_color(theme: &Theme) -> [f32; 4] {
    theme.bg_primary().to_normalized_gamma_f32()
}

/// [`apply`], for a window cleared to [`clear_color`]: every panel's fill is transparent,
/// because the clear under it is already that color. A window's fill stays, since a window
/// floats over something else.
pub fn apply_cleared(ctx: &eframe::egui::Context, theme: &Theme) {
    apply(ctx, theme);
    ctx.all_styles_mut(|style| style.visuals.panel_fill = Color32::TRANSPARENT);
}

/// Apply the theme to egui's own chrome, so panels and buttons match the canvas.
pub fn apply(ctx: &eframe::egui::Context, theme: &Theme) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = theme.bg_primary();
    visuals.window_fill = theme.bg_primary();
    visuals.extreme_bg_color = theme.bg_sunken();
    visuals.faint_bg_color = theme.bg_secondary();
    visuals.override_text_color = Some(theme.text_primary());
    visuals.hyperlink_color = theme.primary();
    visuals.selection.bg_fill = theme.primary().gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, theme.primary());
    visuals.window_stroke = Stroke::new(1.0, theme.border_subtle());

    let radius = CornerRadius::same(RADIUS_SM);
    for (widget, fill, stroke) in [
        (
            &mut visuals.widgets.noninteractive,
            theme.bg_secondary(),
            theme.border_subtle(),
        ),
        (
            &mut visuals.widgets.inactive,
            theme.bg_interactive(),
            theme.border_subtle(),
        ),
        (
            &mut visuals.widgets.hovered,
            theme.bg_hover(),
            theme.primary_muted(),
        ),
        (
            &mut visuals.widgets.active,
            theme.bg_active(),
            theme.primary_muted(),
        ),
        (
            &mut visuals.widgets.open,
            theme.bg_interactive(),
            theme.border_normal(),
        ),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0, stroke);
        widget.corner_radius = radius;
        widget.fg_stroke = Stroke::new(1.0, theme.text_primary());
    }
    ctx.set_visuals(visuals);

    ctx.set_fonts(fonts());

    // Text is Space Grotesk, headings its SemiBold. Monospace is kept for what is read column by
    // column: a path, a MIDI message, the Status box.
    ctx.all_styles_mut(|style| {
        use eframe::egui::{FontId, TextStyle};
        style.text_styles = [
            (TextStyle::Heading, strong_font(FONT_TITLE)),
            (TextStyle::Body, ui_font(FONT_BASE)),
            (TextStyle::Monospace, FontId::monospace(FONT_BASE)),
            (TextStyle::Button, ui_font(FONT_BASE)),
            (TextStyle::Small, ui_font(FONT_TINY)),
        ]
        .into();

        // **No label anywhere selects its text.** egui defaults this on, for document-like
        // UIs; here it is nothing but a way to lose clicks. A selectable label senses click
        // and drag to select, so it takes the press before whatever is under it — which made
        // the name and size on an asset card, and on a workspace card, dead to the pointer
        // while the padding around them worked. Both cards are one target with text drawn on
        // them, and that shape is the normal one here rather than the exception. Turning
        // this back on breaks them again; a `TextEdit` is unaffected, and is where text is
        // selected.
        style.interaction.selectable_labels = false;
    });
}

/// Hack Regular, vendored whole: the monospace face, for what is read column by column — a
/// path, a MIDI message, the Status box.
///
/// `eframe`'s `default_fonts` feature is off, so egui carries no faces of its own; this is
/// the one file it would have supplied, taken unmodified from its own crate source
/// (`epaint_default_fonts`), MIT and Bitstream Vera.
const HACK: &[u8] = include_bytes!("../../assets/fonts/Hack-Regular.ttf");

/// Noto Emoji, whole, vendored.
///
/// `default_fonts` off means egui carries no emoji font to fall back on at all, subset or
/// otherwise, so this is the only one in the stack: every emoji the library draws, not the
/// 887 of some other font's cut-down set.
///
/// It is vendored rather than read from the system so the build does not depend on which
/// fonts a machine happens to have: an icon that renders here renders everywhere.
const NOTO_EMOJI: &[u8] = include_bytes!("../../assets/fonts/NotoEmoji-Regular.ttf");

/// Noto Sans Math, cut to the blocks a UI draws icons from by `scripts/subset-fonts.py`.
///
/// The symbol icons — `∿` for Sine, `∼` for Cosine — are Mathematical Operators, which no
/// emoji font carries. 81 KB against the full face's 967.
const NOTO_SANS_MATH: &[u8] = include_bytes!("../../assets/fonts/NotoSansMath-Subset.ttf");

/// Space Grotesk Regular and SemiBold, cut to text by `scripts/text-fonts.py`, with tabular
/// digits.
const TEXT: &[u8] = include_bytes!("../../assets/fonts/SpaceGrotesk-Regular.ttf");
const TEXT_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/SpaceGrotesk-SemiBold.ttf");

/// The font stack, built from nothing: `default_fonts` is off, so
/// [`eframe::egui::FontDefinitions::default`] returns empty and every family here is one this
/// function writes in full, rather than one it reorders.
///
/// Monospace is Hack alone, plus the two symbol fallbacks every family carries. Proportional
/// and [`STRONG`] put their own text face first — Space Grotesk Regular and SemiBold — for the
/// same reason: it carries letters, digits and punctuation only, so an icon drawn as text
/// still falls through to a face that has it.
///
/// The symbol faces go last on every family, deliberately. epaint walks a family's list in
/// order and moves on when a face has no glyph, so they are reached only where the
/// alternative is a box.
pub fn fonts() -> eframe::egui::FontDefinitions {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};

    let mut defs = FontDefinitions::default();
    for (name, bytes) in [
        ("text", TEXT),
        ("text-semibold", TEXT_SEMIBOLD),
        ("hack", HACK),
        ("noto-emoji-full", NOTO_EMOJI),
        ("noto-sans-math", NOTO_SANS_MATH),
    ] {
        defs.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
    }

    let fallbacks = ["noto-emoji-full", "noto-sans-math"].map(str::to_owned);
    defs.families.insert(
        FontFamily::Proportional,
        std::iter::once("text".to_owned())
            .chain(fallbacks.clone())
            .collect(),
    );
    defs.families.insert(
        FontFamily::Monospace,
        std::iter::once("hack".to_owned())
            .chain(fallbacks.clone())
            .collect(),
    );
    defs.families.insert(
        FontFamily::Name(STRONG.into()),
        std::iter::once("text-semibold".to_owned())
            .chain(fallbacks)
            .collect(),
    );
    defs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color32) -> String {
        format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
    }

    #[test]
    fn the_anchors_resolve_to_the_vapor_preset() {
        let t = Theme::default();
        // The HSL triples are the tokens; the hex in the design system's comments is the
        // color they were rounded from, so the two differ by a shade. hsl(186,84%,58%) is
        // #3adcee, while the comment reads #3adfed = hsl(184.6,83.3%,57.8%). The tokens win,
        // because they are what the theme system actually interpolates.
        assert_eq!(hex(t.main.color()), "#3adcee");
        assert_eq!(hex(t.number.color()), "#a93aee");
        assert_eq!(hex(t.color.color()), "#ec4699");
        assert_eq!(hex(t.event.color()), "#f5c32e");
    }

    #[test]
    fn a_grayscale_theme_produces_true_gray_neutrals() {
        let mut t = Theme::default();
        t.main.s = 0.0;
        let bg = t.bg_primary();
        assert_eq!(bg.r(), bg.g());
        assert_eq!(bg.g(), bg.b());
    }

    /// The default is a preset by name, not merely near one. It is silvia's `vapor` rounded
    /// when the palette was first ported, and the table carries those rounded numbers so a
    /// person who wanders off the default can get back to it by clicking its name.
    #[test]
    fn choosing_vapor_returns_to_the_default() {
        let vapor = PRESETS
            .iter()
            .find(|p| p.key == "vapor")
            .expect("silvia's vapor is one of the sixteen");
        assert_eq!(vapor.theme, Theme::default());
    }

    /// Sixteen distinct looks. A duplicate key would make two buttons that cannot be told
    /// apart, and a duplicate theme would light two of them up at once.
    #[test]
    fn the_presets_are_sixteen_distinct_looks() {
        assert_eq!(PRESETS.len(), 16);
        for (i, a) in PRESETS.iter().enumerate() {
            for b in &PRESETS[i + 1..] {
                assert_ne!(a.key, b.key, "two presets named {}", a.key);
                assert_ne!(
                    a.theme, b.theme,
                    "{} and {} are the same look",
                    a.key, b.key
                );
            }
        }
    }

    /// Every preset survives the trip a preference makes: four anchors out to JSON and back.
    #[test]
    fn a_theme_round_trips_through_the_preferences_file() {
        for preset in &PRESETS {
            let json = serde_json::to_string(&preset.theme).expect("serialize");
            let back: Theme = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, preset.theme, "{} did not survive", preset.key);
        }
    }

    /// `from_rgb` is the inverse of `color`, which is what makes a color picked in the
    /// popup land back on the anchor it came from rather than one step off it.
    #[test]
    fn a_color_picked_out_of_the_popup_lands_back_on_its_anchor() {
        for preset in &PRESETS {
            for anchor in [
                preset.theme.main,
                preset.theme.number,
                preset.theme.color,
                preset.theme.event,
            ] {
                let c = anchor.color();
                let back = Hsl::from_rgb(
                    f32::from(c.r()) / 255.0,
                    f32::from(c.g()) / 255.0,
                    f32::from(c.b()) / 255.0,
                    anchor.h,
                );
                // Through eight bits a channel and back, so the tolerance is one step of
                // that quantization rather than a float epsilon.
                assert_eq!(
                    back.color(),
                    c,
                    "{} {anchor:?} came back as {back:?}",
                    preset.key
                );
            }
        }
    }

    /// A gray has no hue of its own — every hue makes it — so it keeps the one it had.
    /// Without this, dragging an anchor's saturation to nothing and back would return a
    /// different color than it left with.
    #[test]
    fn a_gray_keeps_the_hue_it_came_in_with() {
        let gray = Hsl::from_rgb(0.5, 0.5, 0.5, 277.0);
        assert_eq!(gray.h, 277.0);
        assert_eq!(gray.s, 0.0);
    }

    #[test]
    fn neutrals_climb_in_lightness_as_they_get_more_interactive() {
        let t = Theme::default();
        let l = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert!(l(t.bg_header()) < l(t.bg_sunken()));
        assert!(l(t.bg_sunken()) < l(t.bg_primary()));
        assert!(l(t.bg_primary()) < l(t.bg_secondary()));
        assert!(l(t.bg_secondary()) < l(t.bg_tertiary()));
        assert!(l(t.bg_tertiary()) < l(t.bg_interactive()));
        assert!(l(t.bg_interactive()) < l(t.bg_hover()));
        assert!(l(t.bg_hover()) < l(t.bg_active()));
    }
}
