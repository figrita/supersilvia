// SPDX-License-Identifier: AGPL-3.0-or-later

//! The MIDI window: silvia's, in its three sections.
//!
//! **A window of its own, not a page of Preferences.** A binding is project data and a
//! preference is about the person, so the two would be the one place holding both tiers —
//! and this is a thing somebody opens while patching, watches, and shuts, which is what a
//! window is for. It is not a panel either: the two panel slots are the rig's.
//!
//! Devices, then Mappings, then Monitor, which is silvia's own order. The header carries the
//! gesture, because a person who opens this window is looking for how to bind something.
//!
//! An `egui::Window` and ordinary egui widgets, like the Preferences window and for the same
//! reason: the canvas is hand-painted because it is an instrument, and this is a settings
//! surface that should be in the accessibility tree by construction.

use crate::graph::{Graph, NodeId};
use crate::midi::{Kind, Message, Target, Trigger};
use crate::ui::theme::Theme;
use eframe::egui::{Context, Modifiers, RichText, ScrollArea, Ui, Window};

/// What the window asks `App` to do. It mutates nothing itself, the shape every surface in
/// `ui/` has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiAction {
    /// Re-scan the sequencer and wire in anything new. silvia's Refresh.
    Rescan,
    /// Let go of every action input a note is holding: the way out of a note-off that never
    /// came.
    ReleaseAll,
    /// Forget one binding.
    Unbind(Trigger),
    /// Forget all of them.
    ClearAll,
    /// Watch, or stop watching, arriving messages.
    SetMonitor(bool),
    /// Empty the monitor's log.
    ClearLog,
    /// Show the node a binding names, as the Status box's own links do.
    GoTo(NodeId),
    /// Unfold the Main Mixer, which is where the fade a binding names lives.
    ShowMixer,
    /// Stop waiting for a message.
    CancelLearn,
    /// The window's close button.
    Close,
}

/// The read-only slice of the app the window draws from.
pub struct MidiView<'a> {
    /// Every source the sequencer offers. Empty where there is no sequencer at all.
    pub sources: Vec<crate::midi::Source>,
    /// True where a sequencer could not be opened — a box with no `snd_seq`, or a container
    /// with no `/dev/snd`. A different thing from having one and no devices on it.
    pub absent: bool,
    pub bindings: &'a crate::midi::Bindings,
    /// The control waiting for a message, if a hand is learning one.
    pub learning: Option<Target>,
    pub monitor: bool,
    /// Newest last.
    pub log: &'a std::collections::VecDeque<Message>,
    /// The last value each CC carried, for the bar on its row.
    pub values: &'a std::collections::HashMap<Trigger, u8>,
    pub graph: &'a Graph,
}

/// Draw the window.
pub fn show(
    ctx: &Context,
    view: &MidiView<'_>,
    theme: &Theme,
    saved: &crate::preferences::Placements,
) -> Vec<MidiAction> {
    let mut actions = Vec::new();
    let mut open = true;

    let window = Window::new(super::placed::MIDI)
        .open(&mut open)
        .resizable(true)
        .default_width(520.0)
        .default_height(560.0);
    super::placed::place(ctx, window, super::placed::MIDI, saved).show(ctx, |ui| {
        ui.label(
            RichText::new(format!(
                "{} + click any number or button to bind it",
                ctx.format_modifiers(Modifiers::ALT)
            ))
            .color(theme.text_secondary()),
        );
        ui.add_space(8.0);

        if let Some(target) = view.learning {
            learning(ui, target, view.graph, theme, &mut actions);
            ui.add_space(8.0);
        }

        heading(ui, "Devices");
        ui.horizontal(|ui| {
            if ui.button("Rescan").clicked() {
                actions.push(MidiAction::Rescan);
            }
            if ui
                .button("Release all")
                .on_hover_text(
                    "Let go of every button a held note is pressing. A device that goes away \
                     lets go of its own.",
                )
                .clicked()
            {
                actions.push(MidiAction::ReleaseAll);
            }
        });
        devices(ui, view, theme);

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            heading(ui, "Mappings");
            if !view.bindings.is_empty() && ui.button("Clear all").clicked() {
                actions.push(MidiAction::ClearAll);
            }
        });
        mappings(ui, view, theme, &mut actions);

        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let mut watching = view.monitor;
            if ui.checkbox(&mut watching, "Monitor").changed() {
                actions.push(MidiAction::SetMonitor(watching));
            }
            if !view.log.is_empty() && ui.button("Clear").clicked() {
                actions.push(MidiAction::ClearLog);
            }
        });
        monitor(ui, view, theme);
    });

    if !open {
        actions.push(MidiAction::Close);
    }
    actions
}

fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

/// The banner while a hand is waiting for a message, and the one way out of it besides
/// turning something.
fn learning(
    ui: &mut Ui,
    target: Target,
    graph: &Graph,
    theme: &Theme,
    actions: &mut Vec<MidiAction>,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "Waiting for a message — {}",
                control_name(graph, target)
            ))
            .color(theme.accent()),
        );
        if ui.button("Cancel").clicked() {
            actions.push(MidiAction::CancelLearn);
        }
    });
}

fn devices(ui: &mut Ui, view: &MidiView<'_>, theme: &Theme) {
    if view.absent {
        ui.label(RichText::new("No sequencer on this machine").color(theme.text_muted()));
        return;
    }
    if view.sources.is_empty() {
        ui.label(RichText::new("No MIDI devices").color(theme.text_muted()));
        return;
    }
    for source in &view.sources {
        // Every source is wired in at once, so the mark says what is true rather than
        // offering a choice: which box is on the table is the rig's business, not the map's.
        let ink = if source.connected {
            theme.text_primary()
        } else {
            theme.text_muted()
        };
        ui.label(
            RichText::new(format!(
                "{} {}",
                if source.connected {
                    '\u{25cf}'
                } else {
                    '\u{25cb}'
                },
                source.name
            ))
            .color(ink)
            .monospace(),
        );
    }
}

fn mappings(ui: &mut Ui, view: &MidiView<'_>, theme: &Theme, actions: &mut Vec<MidiAction>) {
    if view.bindings.is_empty() {
        ui.label(
            RichText::new(format!(
                "Nothing bound — {} + click a number or a button",
                ui.ctx().format_modifiers(Modifiers::ALT)
            ))
            .color(theme.text_muted()),
        );
        return;
    }
    ScrollArea::vertical()
        .max_height(200.0)
        .id_salt("midi-mappings")
        .show(ui, |ui| {
            for (trigger, binding) in view.bindings.iter() {
                ui.horizontal(|ui| {
                    // The node's name is a link, exactly as every name in the Status box is;
                    // the mixer's unfolds the panel, which is where the fade is.
                    match binding.target {
                        Target::Port(port) => {
                            if ui.link(node_name(view.graph, port.node)).clicked() {
                                actions.push(MidiAction::GoTo(port.node));
                            }
                            ui.label(RichText::new(port.key).color(theme.text_secondary()));
                        }
                        Target::Balance | Target::Blackout | Target::Freeze => {
                            if ui.link(MIXER).clicked() {
                                actions.push(MidiAction::ShowMixer);
                            }
                            ui.label(
                                RichText::new(mixer_control(binding.target))
                                    .color(theme.text_secondary()),
                            );
                        }
                    }
                    ui.label(RichText::new(trigger.label()).monospace());
                    // What the knob last said, so a row can be matched to a hand on a desk
                    // without watching the monitor.
                    if let Some(value) = view.values.get(&trigger) {
                        ui.label(
                            RichText::new(format!("{value:>3}"))
                                .monospace()
                                .color(theme.text_muted()),
                        );
                    }
                    if unbind_button(ui, theme).clicked() {
                        actions.push(MidiAction::Unbind(trigger));
                    }
                });
            }
        });
}

fn monitor(ui: &mut Ui, view: &MidiView<'_>, theme: &Theme) {
    if !view.monitor {
        ui.label(RichText::new("Not watching").color(theme.text_muted()));
        return;
    }
    if view.log.is_empty() {
        ui.label(RichText::new("Nothing yet — turn something").color(theme.text_muted()));
        return;
    }
    ScrollArea::vertical()
        .max_height(160.0)
        .id_salt("midi-monitor")
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for message in view.log {
                ui.label(RichText::new(line(*message)).monospace());
            }
        });
}

/// The button that forgets a binding: an `✕` **drawn**, not typed.
///
/// The glyph is not in the fonts this ships with, so a `"✕"` button is a tofu box. Every
/// other cross in the editor is its own geometry for the same reason — see
/// `node_widget::close_mark`, which is silvia's `close.svg` — and this is the plain two-bar
/// version of it, at the size a table row can carry.
fn unbind_button(ui: &mut Ui, theme: &Theme) -> eframe::egui::Response {
    let size = eframe::egui::vec2(18.0, 18.0);
    let (rect, response) = ui.allocate_exact_size(size, eframe::egui::Sense::click());
    let ink = if response.hovered() {
        theme.accent()
    } else {
        theme.text_muted()
    };
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            eframe::egui::CornerRadius::same(crate::ui::theme::RADIUS_SM),
            theme.bg_hover(),
        );
    }
    // Two bars through the middle, inset so the cross has air inside its own box.
    let arm = rect.shrink(5.0);
    let stroke = eframe::egui::Stroke::new(1.5, ink);
    ui.painter()
        .line_segment([arm.left_top(), arm.right_bottom()], stroke);
    ui.painter()
        .line_segment([arm.right_top(), arm.left_bottom()], stroke);
    crate::ui::accessible(&response, eframe::egui::WidgetType::Button, "unbind");
    response.on_hover_text("Unbind")
}

/// One message as the monitor writes it: the address a binding would use, then the value.
fn line(message: Message) -> String {
    match message.kind {
        Kind::Control { value, .. } => {
            format!("{:<18} {value:>3}", message.trigger().label())
        }
        Kind::Note { .. } => {
            let Kind::Note { on, .. } = message.kind else {
                unreachable!("matched a note")
            };
            format!(
                "{:<18} {}",
                message.trigger().label(),
                if on { "down" } else { "up" }
            )
        }
    }
}

/// A node as a person reads it: its icon, its label and its id.
fn node_name(graph: &Graph, node: NodeId) -> String {
    let Some(n) = graph.get(node) else {
        return format!("node {node}");
    };
    format!("{} {}{node}", n.def.icon, n.def.label)
}

/// The mixer as its row names it, with the icon its own Mix nodes wear.
const MIXER: &str = "\u{1f39a} Main Mixer";
/// A mixer control as the panel names it.
fn mixer_control(target: Target) -> &'static str {
    match target {
        Target::Blackout => "Blackout",
        Target::Freeze => "Freeze",
        Target::Port(_) | Target::Balance => "A / B balance",
    }
}

/// A control as a person reads it: the node, then the port's own label; or the mixer's fade.
fn control_name(graph: &Graph, target: Target) -> String {
    match target {
        Target::Port(port) => {
            let label = graph
                .get(port.node)
                .and_then(|n| n.def.input(port.key))
                .map_or(port.key, |i| i.label);
            format!("{} {label}", node_name(graph, port.node))
        }
        Target::Balance | Target::Blackout | Target::Freeze => {
            format!("{MIXER} {}", mixer_control(target))
        }
    }
}
