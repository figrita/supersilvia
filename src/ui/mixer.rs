// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Mixer panel: silvia's right panel over the two decks and the fade.
//!
//! Channel A and Channel B, each a live picture of the Output on it and the name of the
//! workspace it lives on, or *No assignment*; the balance as an `s-number`, because it is a
//! control and gets what every control gets; **Blackout** and **Freeze**, the two presses a
//! show is caught with; the crossfade method; and the mix's
//! resolution, *Project to background*, and the mix's own pop-out marks. What it draws over is [`crate::mixer::Mixer`], the rig's, and like every
//! surface in `ui/` it draws and returns what was asked for — a [`MixerAction`] per gesture,
//! and a slot per picture for `App` to fill with a paint callback, since `ui/` has no GPU.
//!
//! **Every control here is a real `Response` with a name**, so a test and the agent-driven
//! layer find the balance as `A / B balance` and the method by the name it shows.

use crate::graph::{NodeId, WorkspaceId};
use crate::mixer::{Method, Resolution};
use crate::ui::Thumbnail;
use crate::ui::number;
use crate::ui::panel::{self, Scrubbed, heading};
use crate::ui::theme::{self, Theme};
use eframe::egui::{Align2, ComboBox, FontId, Modifiers, Rect, Sense, Ui, vec2};

/// What the panel asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum MixerAction {
    SetBalance(f32),
    /// `Alt` + click on the fade, Blackout or Freeze: bind it to the next MIDI message, as
    /// any control.
    LearnMidi(crate::midi::Target),
    /// Blackout's press: the mix black wherever it is shown, or back.
    SetBlackout(bool),
    /// Freeze's press: the mix's last frame wherever it is shown, or back.
    SetFreeze(bool),
    SetMethod(Method),
    SetResolution(Resolution),
    /// One of the mix's two picture marks: a window of its own, or that window fullscreen.
    /// The projector was this, with a button of its own and a window of its own kind; the
    /// mix is a picture like any other and wears the pair every picture wears.
    PopOut(crate::ui::PopOutRequest),
    /// Paint the mix behind the canvas, or stop.
    SetBackground(bool),
    /// Publish the mix over Syphon, or stop.
    Syphon(bool),
    /// Send the mix over NDI, or stop.
    Ndi(bool),
    /// Fold the panel to the right edge, or unfold it.
    SetCollapsed(bool),
    /// The channel's workspace name was clicked: show that workspace, centerd on the deck.
    GoTo {
        workspace: WorkspaceId,
        node: NodeId,
    },
}

/// One deck as the panel draws it.
pub struct ChannelView<'a> {
    pub node: NodeId,
    /// The first workspace the Output is shown on, and its name. `None` for an Output on
    /// no workspace at all, which a project file can describe and the graph cannot draw.
    pub workspace: Option<(WorkspaceId, &'a str)>,
}

/// One of the mixer's two presses as the panel draws it.
#[derive(Debug, Clone, Default)]
pub struct SwitchView {
    /// Held: lit, until it is pressed again.
    pub on: bool,
    /// What drives it, as the trigger's label, where a note has been bound to it.
    pub bound: Option<String>,
    /// Waiting for a MIDI message: it wears the learning ring.
    pub learning: bool,
}

/// The read-only slice of the mixer the panel draws from.
pub struct MixerView<'a> {
    pub a: Option<ChannelView<'a>>,
    pub b: Option<ChannelView<'a>>,
    pub balance: f32,
    /// What drives the fade, as the trigger's label, where a knob has been bound to it.
    pub bound: Option<String>,
    /// The fade is waiting for a MIDI message: it wears the learning ring.
    pub learning: bool,
    /// Where the fader bound to the fade is, while soft takeover holds it out of pick-up.
    pub ghost: Option<f32>,
    pub blackout: SwitchView,
    pub freeze: SwitchView,
    pub method: Method,
    pub resolution: Resolution,
    /// Whether the mix has a window of its own, and whether that window is fullscreen.
    pub popped: Option<bool>,
    /// Why there are no picture windows in this session — no Wayland, no GPU, a
    /// thread that died. `None` where there can be. The marks say it rather than failing
    /// silently when they are pressed.
    pub no_windows: Option<&'a str>,
    /// Whether the mix is painted behind the canvas.
    pub background: bool,
    /// Whether the mix is published over Syphon; `None` on a machine without Syphon, where the
    /// mark is not drawn.
    pub syphon: Option<bool>,
    /// Whether the mix is sent over NDI, or why NDI cannot be used here: the runtime is missing.
    pub ndi: Result<bool, &'a str>,
    /// Folded to the right edge.
    pub collapsed: bool,
}

/// What one frame of the panel produced.
#[derive(Default)]
pub struct MixerOutput {
    pub actions: Vec<MixerAction>,
    /// Where each deck's picture goes. `App` fills the slot with a paint callback blitting
    /// that Output's published frame, exactly as it fills a node's on-body render.
    pub previews: Vec<Thumbnail>,
}

/// What the panel is called, in its header and up its spine.
pub const TITLE: &str = "Main Mixer";

/// The balance control's range: −1 is deck A alone, +1 is deck B alone.
const BALANCE: crate::graph::ControlRange = crate::graph::ControlRange {
    min: -1.0,
    max: 1.0,
    step: 0.01,
};

/// Space between sections.
const SECTION_GAP: f32 = 10.0;

/// Draw the panel. The mix's own preview is drawn under it by the caller, in the space
/// this leaves.
pub fn show(ui: &mut Ui, view: &MixerView<'_>, theme: &Theme, lock_cursor: bool) -> MixerOutput {
    let mut out = MixerOutput::default();
    if view.collapsed {
        if panel::spine(ui, TITLE, panel::Side::Right, theme) {
            out.actions.push(MixerAction::SetCollapsed(false));
        }
        return out;
    }
    if panel::header(ui, TITLE, panel::Side::Right, theme) {
        out.actions.push(MixerAction::SetCollapsed(true));
    }

    for (name, channel) in [("Channel A", &view.a), ("Channel B", &view.b)] {
        heading(ui, name, theme);
        channel_preview(ui, name, channel.as_ref(), theme, &mut out);
        ui.add_space(SECTION_GAP);
    }

    heading(ui, "Mix", theme);
    balance(ui, view, theme, lock_cursor, &mut out.actions);
    ui.add_space(4.0);
    holds(ui, view, theme, &mut out.actions);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Crossfade method");
        let mut method = view.method;
        ComboBox::from_id_salt("crossfade-method")
            .selected_text(method.label())
            .show_ui(ui, |ui| {
                for m in Method::ALL {
                    ui.selectable_value(&mut method, m, m.label());
                }
            });
        if method != view.method {
            out.actions.push(MixerAction::SetMethod(method));
        }
    });
    ui.add_space(SECTION_GAP);

    heading(ui, "Projection", theme);
    ui.horizontal(|ui| {
        ui.label("Resolution");
        let mut resolution = view.resolution;
        ComboBox::from_id_salt("mix-resolution")
            .selected_text(resolution.label())
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut resolution,
                    Resolution::Viewport,
                    Resolution::Viewport.label(),
                );
                for (w, h) in Resolution::PRESETS {
                    let r = Resolution::Fixed(w, h);
                    ui.selectable_value(&mut resolution, r, r.label());
                }
            });
        if resolution != view.resolution {
            out.actions.push(MixerAction::SetResolution(resolution));
        }
    });
    let mut background = view.background;
    if ui
        .checkbox(&mut background, "Project to background")
        .on_hover_text("Paint the mix behind the canvas, the nodes and cables floating on the show. H hides them.")
        .changed()
    {
        out.actions.push(MixerAction::SetBackground(background));
    }
    ui.add_space(4.0);
    // The mix's own two marks, the pair every picture in the app carries, in a row of their
    // own because the panel has no picture here to hang them on. The projector was a button
    // saying *Open projector*; this is the same gesture, with the same meaning everywhere.
    ui.horizontal(|ui| {
        heading(ui, "Window", theme);
        let side = crate::ui::canvas::MARK_SIZE;
        for (fullscreen, draw) in [
            (
                false,
                crate::ui::node_widget::popout_mark
                    as fn(&eframe::egui::Painter, Rect, eframe::egui::Color32),
            ),
            (true, crate::ui::node_widget::expand_mark),
        ] {
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            // Lit while the thing it asks for is already true, which is what a picture's
            // marks do: the pop-out says the window is up, fullscreen says it is fullscreen.
            let lit = match (view.popped, fullscreen) {
                (Some(full), true) => full,
                (Some(_), false) => true,
                (None, _) => false,
            };
            let w = crate::ui::node_widget::picture_mark(
                ui,
                rect,
                ui.id().with(("mix-mark", fullscreen)),
                if fullscreen {
                    "fullscreen the mix"
                } else {
                    "pop out the mix"
                },
                draw,
                Some(if lit { theme.primary() } else { theme.text_primary() }),
                theme,
            )
            .on_hover_text(view.no_windows.unwrap_or(if fullscreen {
                "The mix in a window of its own, filling a screen. Escape leaves it."
            } else {
                "The mix in a window of its own: no decorations, dragged by the picture, F for fullscreen."
            }));
            if w.clicked() {
                out.actions.push(MixerAction::PopOut(crate::ui::PopOutRequest {
                    picture: crate::ui::PopOut::Mix,
                    fullscreen,
                }));
            }
        }
        // Beside the pair: the mix sent to other apps rather than to a window, lit while it is.
        if let Some(on) = view.syphon {
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            let w = crate::ui::node_widget::picture_mark(
                ui,
                rect,
                ui.id().with("mix-syphon"),
                "publish the mix over Syphon",
                crate::ui::node_widget::syphon_mark,
                Some(if on {
                    theme.primary()
                } else {
                    theme.text_primary()
                }),
                theme,
            )
            .on_hover_text(
                "The mix published over Syphon as “Mix”, for another app on this Mac to take as a source. Not saved.",
            );
            if w.clicked() {
                out.actions.push(MixerAction::Syphon(!on));
            }
        }
        // And to other machines on the network.
        let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        let w = crate::ui::node_widget::picture_mark(
            ui,
            rect,
            ui.id().with("mix-ndi"),
            "send the mix over NDI",
            crate::ui::node_widget::ndi_mark,
            Some(if view.ndi == Ok(true) {
                theme.primary()
            } else {
                theme.text_primary()
            }),
            theme,
        )
        .on_hover_text(view.ndi.err().unwrap_or(
            "The mix sent over NDI® as “supersilvia Mix”, for other machines on the network to take as a source. Not saved. ndi.video",
        ));
        if w.clicked()
            && let Ok(on) = view.ndi
        {
            out.actions.push(MixerAction::Ndi(!on));
        }
    });
    ui.add_space(SECTION_GAP);

    out
}

/// A deck's picture, 16:9 across the panel, with the name of its workspace under it. A deck
/// with nothing on it is the same black with *No assignment* under it.
fn channel_preview(
    ui: &mut Ui,
    name: &str,
    channel: Option<&ChannelView<'_>>,
    theme: &Theme,
    out: &mut MixerOutput,
) {
    // A deck is an Output, so the picture is the node's own render.
    out.previews
        .extend(panel::picture(ui, channel.map(|c| (c.node, None)), theme));

    match channel {
        Some(ChannelView {
            node,
            workspace: Some((workspace, ws_name)),
        }) => {
            let response = ui.add(
                eframe::egui::Label::new(
                    eframe::egui::RichText::new(*ws_name).color(theme.primary()),
                )
                .sense(Sense::click()),
            );
            let response = response.on_hover_text(format!("Show {ws_name}"));
            if response.clicked() {
                out.actions.push(MixerAction::GoTo {
                    workspace: *workspace,
                    node: *node,
                });
            }
        }
        Some(ChannelView {
            workspace: None, ..
        }) => {
            ui.label(eframe::egui::RichText::new("On no workspace").color(theme.text_muted()));
        }
        None => {
            ui.label(
                eframe::egui::RichText::new(format!("{name}: no assignment"))
                    .color(theme.text_muted()),
            );
        }
    }
}

/// The fade, as the `s-number` it is: `A` at one end, `B` at the other, the control between.
fn balance(
    ui: &mut Ui,
    view: &MixerView<'_>,
    theme: &Theme,
    lock_cursor: bool,
    actions: &mut Vec<MixerAction>,
) {
    let caption = ui.label("A / B balance");
    midi_mark(ui, caption.rect, view.bound.as_deref(), theme);
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, number::HEIGHT), Sense::hover());
    let side = 14.0;
    let font = FontId::monospace(theme::FONT_BASE);
    ui.painter().text(
        row.left_center(),
        Align2::LEFT_CENTER,
        "A",
        font.clone(),
        theme.text_secondary(),
    );
    ui.painter().text(
        row.right_center(),
        Align2::RIGHT_CENTER,
        "B",
        font,
        theme.text_secondary(),
    );
    let control = Rect::from_min_max(row.min + vec2(side, 0.0), row.max - vec2(side, 0.0));
    // The one control that is not a node's port and still answers to a knob: the map has an
    // address for it, `midi::Target::Balance`. See docs/media.md.
    match panel::ghosted(
        ui,
        control,
        "A / B balance",
        (view.balance, -1.0),
        BALANCE,
        theme,
        lock_cursor,
        view.learning,
        view.ghost,
    ) {
        Some(Scrubbed::Set(v)) => actions.push(MixerAction::SetBalance(v)),
        Some(Scrubbed::Learn) => actions.push(MixerAction::LearnMidi(crate::midi::Target::Balance)),
        None => {}
    }
}

/// The space between Blackout and Freeze, wide enough for a bound press's dot in the gutter.
const HOLD_GAP: f32 = 8.0;

/// Blackout and Freeze, side by side under the fade, each half the panel and the height of an
/// `s-number`, so a lit one changes no width and moves nothing.
fn holds(ui: &mut Ui, view: &MixerView<'_>, theme: &Theme, actions: &mut Vec<MixerAction>) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, number::HEIGHT), Sense::hover());
    let half = (row.width() - HOLD_GAP) / 2.0;
    let left = Rect::from_min_size(row.min, vec2(half, row.height()));
    let right = Rect::from_min_size(
        row.min + vec2(half + HOLD_GAP, 0.0),
        vec2(half, row.height()),
    );
    // Alt as the machine names it: `Alt` here, `⌥` on a Mac.
    let alt = ui.ctx().format_modifiers(Modifiers::ALT);
    for (rect, name, switch, target, hint) in [
        (
            left,
            "Blackout",
            &view.blackout,
            crate::midi::Target::Blackout,
            format!(
                "Blackout: the mix black wherever it is shown — this panel, the canvas, a \
                 picture window, NDI and Syphon — until pressed again. Not saved. {alt} + click \
                 binds a MIDI note."
            ),
        ),
        (
            right,
            "Freeze",
            &view.freeze,
            crate::midi::Target::Freeze,
            format!(
                "Freeze: the mix's last frame wherever it is shown, while the decks go on \
                 underneath, until pressed again. Not saved. {alt} + click binds a MIDI note."
            ),
        ),
    ] {
        match hold(ui, rect, name, switch, theme, &hint) {
            Some(Hold::Toggle) => actions.push(if target == crate::midi::Target::Blackout {
                MixerAction::SetBlackout(!switch.on)
            } else {
                MixerAction::SetFreeze(!switch.on)
            }),
            Some(Hold::Learn) => actions.push(MixerAction::LearnMidi(target)),
            None => {}
        }
    }
}

/// What a hand did to one of the two presses.
enum Hold {
    Toggle,
    Learn,
}

/// One press: the press button's field, lit in the accent with a `●` before its name while it
/// holds — a glyph and the accent, since the palette carries no red for an alarm. A real
/// `Response` named `Blackout` or `Freeze`, selected while it holds.
fn hold(
    ui: &mut Ui,
    rect: Rect,
    name: &str,
    switch: &SwitchView,
    theme: &Theme,
    hint: &str,
) -> Option<Hold> {
    let response = ui.interact(rect, ui.id().with(("mixer-hold", name)), Sense::click());
    let learn = ui.ctx().input(|i| i.modifiers.alt) && response.clicked();
    let (fill, border, ink) = if switch.on {
        (theme.accent(), theme.accent(), theme.bg_primary())
    } else if response.hovered() {
        (
            theme.bg_hover(),
            theme.border_normal(),
            theme.text_primary(),
        )
    } else {
        (
            theme.bg_interactive(),
            theme.border_normal(),
            theme.text_secondary(),
        )
    };
    crate::ui::field(ui.painter(), rect, fill, None, border);
    if switch.learning {
        crate::ui::learning_ring(ui, rect, 1.0, theme);
    }
    let caption = if switch.on {
        format!("\u{25cf} {name}")
    } else {
        name.to_string()
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(theme::FONT_BASE),
        ink,
    );
    let label = if switch.learning {
        format!("{name} {}", crate::ui::LEARNING)
    } else {
        name.to_string()
    };
    response.widget_info(|| {
        eframe::egui::WidgetInfo::selected(
            eframe::egui::WidgetType::Button,
            true,
            switch.on,
            &label,
        )
    });
    if let Some(trigger) = &switch.bound {
        crate::ui::midi_mark(
            ui,
            crate::ui::MarkAt::Corner(rect),
            1.0,
            theme,
            name,
            trigger,
            "unbind it in Project ▸ MIDI",
        );
    }
    let response = response.on_hover_text(hint);
    if learn {
        Some(Hold::Learn)
    } else if response.clicked() {
        Some(Hold::Toggle)
    } else {
        None
    }
}

/// The mark a bound fade wears: the dot every bound control wears, after the caption rather
/// than beside the slot, since the fade's slot has `A` and `B` on its two sides. Named as a
/// node's is, `A / B balance bound to CC 1 ch 1`. Unbinding is in the MIDI window, on the
/// fade's own row. See [`crate::ui::midi_mark`].
fn midi_mark(ui: &Ui, caption: Rect, bound: Option<&str>, theme: &Theme) {
    let Some(trigger) = bound else {
        return;
    };
    crate::ui::midi_mark(
        ui,
        crate::ui::MarkAt::After(caption),
        1.0,
        theme,
        "A / B balance",
        trigger,
        "unbind it in Project ▸ MIDI",
    );
}
