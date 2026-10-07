// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Mixer panel: silvia's right panel over the two decks and the fade.
//!
//! Channel A and Channel B, each a live picture of the Output on it with the name of the
//! workspace it lives on beside its heading, or *No Output assigned* inside the box; the balance as an `s-number`, because it is a
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
use crate::ui::panel::{self, ROW_GAP, SECTION_GAP, Scrubbed, heading};
use crate::ui::theme::{self, Theme};
use eframe::egui::{Align2, ComboBox, FontId, Modifiers, Rect, Sense, Ui, vec2};

/// A picture mark's painter: what `node_widget` draws a mark on a picture with.
type Mark = fn(&eframe::egui::Painter, Rect, eframe::egui::Color32);

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
        channel_preview(ui, name, channel.as_ref(), theme, &mut out);
        ui.add_space(SECTION_GAP);
    }

    let mix = heading(ui, "Mix", theme);
    midi_mark(ui, mix, view.bound.as_deref(), theme);
    panel::row(ui, "Crossfade", theme, |ui| {
        let mut method = view.method;
        ComboBox::from_id_salt("crossfade-method")
            .selected_text(method.label())
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for m in Method::ALL {
                    ui.selectable_value(&mut method, m, m.label());
                }
            })
            .response
            .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
        if method != view.method {
            out.actions.push(MixerAction::SetMethod(method));
        }
    });
    ui.add_space(ROW_GAP);
    balance(ui, view, theme, lock_cursor, &mut out.actions);
    ui.add_space(ROW_GAP);
    holds(ui, view, theme, &mut out.actions);
    ui.add_space(SECTION_GAP);

    heading(ui, "Projection", theme);
    panel::row(ui, "Resolution", theme, |ui| {
        let mut resolution = view.resolution;
        ComboBox::from_id_salt("mix-resolution")
            .selected_text(resolution.label())
            .width(ui.available_width())
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
            })
            .response
            .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
        if resolution != view.resolution {
            out.actions.push(MixerAction::SetResolution(resolution));
        }
    });
    ui.add_space(ROW_GAP);
    let mut background = view.background;
    if panel::row(ui, "", theme, |ui| {
        ui.checkbox(&mut background, "Project to background")
            .on_hover_cursor(eframe::egui::CursorIcon::PointingHand)
            .on_hover_text("Paint the mix behind the canvas, the nodes and cables floating on the show. H hides them.")
            .changed()
    }) {
        out.actions.push(MixerAction::SetBackground(background));
    }
    ui.add_space(ROW_GAP);
    // The mix's own two picture marks, the pair every picture in the app carries, as worded
    // buttons: the panel has the room a node's header does not, and a word is read where an
    // icon is decoded. Each keeps its icon beside the word, so it is still the same gesture
    // as the marks on a picture. Lit while the thing it asks for is already true: the pop-out
    // says the window is up, fullscreen says it is fullscreen.
    panel::row(ui, "Window", theme, |ui| {
        let [left, right] = halves(ui);
        for (rect, fullscreen, word, draw) in [
            (
                left,
                false,
                "Pop out",
                crate::ui::node_widget::popout_mark as Mark,
            ),
            (
                right,
                true,
                "Fullscreen",
                crate::ui::node_widget::expand_mark,
            ),
        ] {
            let lit = match (view.popped, fullscreen) {
                (Some(full), true) => full,
                (Some(_), false) => true,
                (None, _) => false,
            };
            let w = worded_mark(
                ui,
                rect,
                ui.id().with(("mix-mark", fullscreen)),
                if fullscreen {
                    "fullscreen the mix"
                } else {
                    "pop out the mix"
                },
                word,
                draw,
                lit,
                theme,
            )
            .on_hover_text(view.no_windows.unwrap_or(if fullscreen {
                "The mix in a window of its own, filling a screen. Escape leaves it."
            } else {
                "The mix in a window of its own: no decorations, dragged by the picture, F for fullscreen."
            }));
            if w.clicked() {
                out.actions
                    .push(MixerAction::PopOut(crate::ui::PopOutRequest {
                        picture: crate::ui::PopOut::Mix,
                        fullscreen,
                    }));
            }
        }
    });
    ui.add_space(ROW_GAP);
    // The mix sent to other apps and other machines rather than to a window, lit while it is.
    // NDI is everywhere; Syphon only on a Mac, where it takes the first half.
    panel::row(ui, "Send", theme, |ui| {
        let [left, right] = halves(ui);
        let ndi_at = if let Some(on) = view.syphon {
            let w = worded_mark(
                ui,
                left,
                ui.id().with("mix-syphon"),
                "publish the mix over Syphon",
                "Syphon",
                crate::ui::node_widget::syphon_mark,
                on,
                theme,
            )
            .on_hover_text(
                "The mix published over Syphon as “Mix”, for another app on this Mac to take as a source. Not saved.",
            );
            if w.clicked() {
                out.actions.push(MixerAction::Syphon(!on));
            }
            right
        } else {
            left
        };
        let w = worded_mark(
            ui,
            ndi_at,
            ui.id().with("mix-ndi"),
            "send the mix over NDI",
            "NDI®",
            crate::ui::node_widget::ndi_mark,
            view.ndi == Ok(true),
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
    ui.add_space(ROW_GAP * 2.0);

    out
}

/// A deck: its heading with the name of its workspace at the far end of the same line, and
/// its picture 16:9 across the panel. A deck with nothing on it is the same black with
/// *No Output assigned* in it, named `Channel A: no Output assigned` for the tree.
fn channel_preview(
    ui: &mut Ui,
    name: &str,
    channel: Option<&ChannelView<'_>>,
    theme: &Theme,
    out: &mut MixerOutput,
) {
    ui.horizontal(|ui| {
        heading(ui, name, theme);
        ui.with_layout(
            eframe::egui::Layout::right_to_left(eframe::egui::Align::Center),
            |ui| workspace_link(ui, channel, theme, out),
        );
    });
    // A deck is an Output, so the picture is the node's own render.
    let unassigned = format!("{name}: no Output assigned");
    out.previews.extend(panel::picture(
        ui,
        channel.map(|c| (c.node, None)),
        Some(("No Output assigned", &unassigned)),
        theme,
    ));
}

/// The workspace a deck's Output lives on, as a link that goes there; nothing for an empty
/// deck, whose box says so.
fn workspace_link(
    ui: &mut Ui,
    channel: Option<&ChannelView<'_>>,
    theme: &Theme,
    out: &mut MixerOutput,
) {
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
            let response = response
                .on_hover_text(format!("Show {ws_name}"))
                .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
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
        None => {}
    }
}

/// The fade, as the `s-number` it is: `A` at one end, `B` at the other, the control between.
/// No caption of its own: the section is the mix, and the letters at its two ends say the
/// rest.
fn balance(
    ui: &mut Ui,
    view: &MixerView<'_>,
    theme: &Theme,
    lock_cursor: bool,
    actions: &mut Vec<MixerAction>,
) {
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(vec2(width, number::HEIGHT), Sense::hover());
    let side = 14.0;
    let font = FontId::proportional(theme::FONT_BASE);
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

/// The space between Blackout and Freeze, wide enough for a bound press's dot in the gutter,
/// and between the two buttons of any row split in half.
const HOLD_GAP: f32 = 8.0;

/// What is left of a row, an `s-number` tall, as two halves with [`HOLD_GAP`] between them.
fn halves(ui: &mut Ui) -> [Rect; 2] {
    let (row, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), number::HEIGHT), Sense::hover());
    let half = (row.width() - HOLD_GAP) / 2.0;
    [
        Rect::from_min_size(row.min, vec2(half, row.height())),
        Rect::from_min_size(
            row.min + vec2(half + HOLD_GAP, 0.0),
            vec2(half, row.height()),
        ),
    ]
}

/// A picture mark with its word beside it, as a button filling `rect`: the field every press
/// here wears, the icon and the word centerd in it together. Lit, it is ringed and inked in
/// the primary hue, which is what a lit mark on a picture is. Named `name` in the tree, which
/// is the name the mark carries everywhere else.
#[allow(clippy::too_many_arguments)]
fn worded_mark(
    ui: &mut Ui,
    rect: Rect,
    id: eframe::egui::Id,
    name: &str,
    word: &str,
    draw: fn(&eframe::egui::Painter, Rect, eframe::egui::Color32),
    lit: bool,
    theme: &Theme,
) -> eframe::egui::Response {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        eframe::egui::WidgetInfo::selected(eframe::egui::WidgetType::Button, true, lit, name)
    });
    let fill = if response.hovered() {
        theme.bg_hover()
    } else {
        theme.bg_interactive()
    };
    let border = if lit {
        theme.primary()
    } else {
        theme.border_normal()
    };
    crate::ui::field(ui.painter(), rect, fill, None, border);
    let ink = if lit {
        theme.primary()
    } else {
        theme.text_secondary()
    };
    let galley =
        ui.painter()
            .layout_no_wrap(word.to_string(), theme::ui_font(theme::FONT_BASE), ink);
    let icon = rect.height();
    let gap = 2.0;
    let width = icon + gap + galley.size().x;
    let left = rect.center().x - width * 0.5;
    draw(
        ui.painter(),
        Rect::from_min_size(eframe::egui::pos2(left, rect.min.y), vec2(icon, icon)),
        ink,
    );
    ui.painter().galley(
        eframe::egui::pos2(left + icon + gap, rect.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    response
}

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
    crate::ui::cursor(&response, eframe::egui::CursorIcon::PointingHand);
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

/// The mark a bound fade wears: the dot every bound control wears, after the section's
/// heading rather than beside the slot, since the fade's slot has `A` and `B` on its two sides. Named as a
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
