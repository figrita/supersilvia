// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Input panel: silvia's left panel, over one video source and one audio source.
//!
//! The shape is silvia's — a Video Source section, an Audio Source section, and an analyzer
//! under them — and the two departures from it are deliberate.
//!
//! **No demo video.** It was a file bundled with a web page that had nothing else to show.
//!
//! **The analyzer is the scope this app already has**, not silvia's grid of nine numbers. Gain,
//! Expand and Smooth were three knobs per band behind the measurement, and shaping a band is
//! the graph's job here — `slew` smooths and an exciter expands, where they can be seen and
//! patched. What is left is the thing silvia's grid could not do: the spectrum, a handle per
//! band saying where it listens and how narrowly, and the threshold that fires its event
//! sitting on the meter it is compared against.
//!
//! Like every surface in `ui/`, it draws and returns: a [`MainInputAction`] per gesture, and a
//! slot for the picture, which `App` fills with a paint callback because `ui/` has no GPU.

use crate::audio::{BANDS, Scope, bands, device};
use crate::maininput::{AudioSource, MainInput, VideoSource};
use crate::ui::Thumbnail;
use crate::ui::number;
use crate::ui::panel::{self, ROW_GAP, SECTION_GAP, Scrubbed, heading};
use crate::ui::scope;
use crate::ui::theme::{self, Theme};
use eframe::egui::{ComboBox, FontId, Sense, Ui, vec2};

/// What the panel asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum MainInputAction {
    SetVideo(VideoSource),
    SetAudio(AudioSource),
    SetGain(f32),
    SetMonitor(f32),
    /// A band's center frequency and Q, from one drag on the scope.
    SetBand {
        band: usize,
        freq: f32,
        q: f32,
    },
    /// A band's threshold, from a drag on its meter.
    SetLevel {
        band: usize,
        level: f32,
    },
    /// Open a file dialog for a clip, or for a sound.
    PickFile {
        audio: bool,
    },
    /// Put the project's own media up, for a clip or for a sound — the same picker a
    /// `video` node's file button opens, with the dialog as its last entry.
    OpenPicker {
        audio: bool,
    },
    ClosePicker,
    /// Put the desktop's picker up again, forgetting whatever it last resumed.
    ChooseScreenAgain,
    /// Fold the panel to the edge, or unfold it.
    SetCollapsed(bool),
    /// Ask the machine what audio sources it has, again.
    RefreshDevices,
}

/// The read-only slice of the world the panel draws from.
pub struct MainInputView<'a> {
    /// What is chosen. The project's.
    pub input: &'a MainInput,
    /// What is actually happening, one line under each source.
    pub video_status: &'a str,
    pub audio_status: &'a str,
    pub error: Option<&'a str>,
    /// Every audio source the machine offers, for the list.
    pub devices: &'a [device::Listed],
    /// Every capture device, as `(path, name)`. Handed in already gathered: **this panel must
    /// never enumerate devices while drawing**, which is once a frame.
    pub cameras: &'a [(String, String)],
    /// Every Syphon server on the Mac by its label, as the directory's listing last read them.
    pub syphon: Vec<String>,
    /// Whether this machine has Syphon. Where it has not, the list does not offer it, and a
    /// project that chose it on a Mac reads as [`SYPHON_ELSEWHERE`] with nothing under it.
    pub syphon_here: bool,
    /// Every NDI source on the network by its name, as the provider last listed them — listed
    /// only while an NDI source is chosen.
    pub ndi: Vec<String>,
    /// Why NDI cannot be used here, where the runtime is missing.
    pub ndi_missing: Option<&'static str>,
    /// The analysis, tuning and thresholds as one picture. `None` with nothing to hear, and
    /// the analyzer section then says so rather than drawing a flat line.
    pub scope: Option<Scope>,
    /// True while the Main Input has a picture to show.
    pub has_picture: bool,
    pub collapsed: bool,
    /// Every file in `assets/`, for the picker; the panel filters them to clips or sounds.
    pub assets: &'a [crate::project::AssetInfo],
    /// Each asset's poster, by reference, as `App` has decoded them so far.
    pub posters: &'a std::collections::HashMap<String, Option<eframe::egui::TextureHandle>>,
    /// Which picker is up: `Some(false)` the clip's, `Some(true)` the sound's. `App`'s, so
    /// the panel stays a draw-and-return.
    pub picker: Option<bool>,
}

/// What one frame of the panel produced.
#[derive(Default)]
pub struct MainInputOutput {
    pub actions: Vec<MainInputAction>,
    /// Where the picture goes. `App` fills it, exactly as it fills a deck's.
    pub preview: Option<Thumbnail>,
}

/// The analyzer's height. The same proportion a node's scope region gives it: a third spectrum
/// above, the three meters below.
const SCOPE_HEIGHT: f32 = 120.0;

/// What the panel is called, in its header and up its spine.
pub const TITLE: &str = "Main Input";

/// What the select reads on a machine without Syphon, for a project that chose it on a Mac.
const SYPHON_ELSEWHERE: &str = "Syphon (macOS only)";

/// The video sources, in the order the panel lists them.
const VIDEO_CHOICES: [(&str, &str); 6] = [
    ("none", "None"),
    ("camera", "Camera"),
    ("screen", "Screen or window"),
    ("syphon", "Syphon"),
    ("ndi", "NDI®"),
    ("file", "Video file"),
];

/// Draw the panel.
pub fn show(
    ui: &mut Ui,
    view: &MainInputView<'_>,
    theme: &Theme,
    lock_cursor: bool,
) -> MainInputOutput {
    let mut out = MainInputOutput::default();
    if view.collapsed {
        if panel::spine(ui, TITLE, panel::Side::Left, theme) {
            out.actions.push(MainInputAction::SetCollapsed(false));
        }
        return out;
    }
    if panel::header(ui, TITLE, panel::Side::Left, theme) {
        out.actions.push(MainInputAction::SetCollapsed(true));
    }

    // A source's status line is there only while a source is: with none, the picture box and
    // the analyzer say so themselves, and a line under them saying it again is noise.
    heading(ui, "Video", theme);
    video_source(ui, view, theme, &mut out);
    ui.add_space(ROW_GAP);
    picture(ui, view, theme, &mut out);
    if view.input.video != VideoSource::None {
        status_line(ui, view.video_status, theme);
    }
    ui.add_space(SECTION_GAP);

    heading(ui, "Audio", theme);
    audio_source(ui, view, theme, &mut out);
    if view.input.audio != AudioSource::None {
        status_line(ui, view.audio_status, theme);
    }
    ui.add_space(ROW_GAP);
    row(
        ui,
        "Gain",
        view.input.gain,
        GAIN,
        theme,
        lock_cursor,
        &mut |v| MainInputAction::SetGain(v),
        &mut out.actions,
    );
    row(
        ui,
        "Monitor",
        view.input.monitor,
        MONITOR,
        theme,
        lock_cursor,
        &mut MainInputAction::SetMonitor,
        &mut out.actions,
    );
    ui.add_space(SECTION_GAP);

    heading(ui, "Analyzer", theme);
    analyzer(ui, view, theme, &mut out);

    if let Some(error) = view.error {
        ui.add_space(SECTION_GAP);
        ui.label(
            eframe::egui::RichText::new(error)
                .font(FontId::proportional(theme::FONT_BASE))
                .color(theme.accent()),
        );
    }
    out
}

/// Gain's range. silvia's own: one is unity and three is as far as its panel went.
const GAIN: crate::graph::ControlRange = crate::graph::ControlRange {
    min: 0.0,
    max: 3.0,
    step: 0.01,
};

/// The monitor's, which is every monitor's here: zero is off.
const MONITOR: crate::graph::ControlRange = crate::graph::ControlRange {
    min: 0.0,
    max: 1.0,
    step: 0.01,
};

/// Which of the four kinds a source is, for the select.
fn video_key(source: &VideoSource) -> &'static str {
    match source {
        VideoSource::None => "none",
        VideoSource::Camera { .. } => "camera",
        VideoSource::Screen => "screen",
        VideoSource::Syphon { .. } => "syphon",
        VideoSource::Ndi { .. } => "ndi",
        VideoSource::File { .. } => "file",
    }
}

fn video_source(ui: &mut Ui, view: &MainInputView<'_>, theme: &Theme, out: &mut MainInputOutput) {
    let current = video_key(&view.input.video);
    let mut chosen = current;
    let offered = |key: &str| view.syphon_here || key != "syphon";
    let select = ComboBox::from_id_salt("main-input-video")
        .selected_text(if offered(current) {
            VIDEO_CHOICES
                .iter()
                .find(|(key, _)| *key == current)
                .map_or("None", |(_, label)| *label)
        } else {
            SYPHON_ELSEWHERE
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (key, label) in VIDEO_CHOICES.into_iter().filter(|(key, _)| offered(key)) {
                ui.selectable_value(&mut chosen, key, label);
            }
        })
        .response
        .rect;
    // Where the picker hangs: under the file's own button where there is one, else under
    // the select that asked for a file.
    let mut anchor = select;
    if chosen != current {
        match chosen {
            "none" => out
                .actions
                .push(MainInputAction::SetVideo(VideoSource::None)),
            "camera" => out
                .actions
                .push(MainInputAction::SetVideo(VideoSource::Camera {
                    device: String::new(),
                })),
            // Choosing it *is* asking: the desktop's picker comes up, and the reconcile that
            // opens it is the same one any other source goes through.
            "screen" => out
                .actions
                .push(MainInputAction::SetVideo(VideoSource::Screen)),
            // The first server listed, where there is one: a menu of one is a choice made.
            "syphon" => out
                .actions
                .push(MainInputAction::SetVideo(VideoSource::Syphon {
                    server: view.syphon.first().cloned().unwrap_or_default(),
                    flip: false,
                    transparent: false,
                })),
            // Nothing chosen yet: the network is listed from now on, and the menu fills.
            "ndi" => out
                .actions
                .push(MainInputAction::SetVideo(VideoSource::Ndi {
                    source: String::new(),
                    transparent: false,
                })),
            // A file source with no file is nothing to show, so the picker opens with it —
            // or the dialog, where the project has no clip to offer yet.
            _ => out.actions.push(open_file(view, false)),
        }
    }

    // A camera has devices to choose between, and a file can be swapped for another.
    match &view.input.video {
        VideoSource::Camera { device } => {
            let label = match device.as_str() {
                "" => "First one found".to_string(),
                "test" => "Test pattern".to_string(),
                id if crate::platform::video::NAMED_CAMERAS => view
                    .cameras
                    .iter()
                    .find(|(listed, _)| listed == id)
                    .map_or_else(|| id.to_string(), |(_, name)| name.clone()),
                path => path.to_string(),
            };
            let mut pick = device.clone();
            ComboBox::from_id_salt("main-input-camera-device")
                .selected_text(label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pick, String::new(), "First one found");
                    for (id, name) in view.cameras {
                        let label = crate::video::camera_label(id, name);
                        ui.selectable_value(&mut pick, id.clone(), label);
                    }
                    ui.separator();
                    ui.selectable_value(&mut pick, "test".to_string(), "Test pattern")
                        .on_hover_text(
                            "GStreamer's color bars. Always there, so a black picture means \
                             the patch rather than the hardware.",
                        );
                    if ui.button("Look for devices again").clicked() {
                        out.actions.push(MainInputAction::RefreshDevices);
                    }
                });
            if pick != *device {
                out.actions
                    .push(MainInputAction::SetVideo(VideoSource::Camera {
                        device: pick,
                    }));
            }
        }
        VideoSource::File { asset } => {
            let button = ui.button(short(asset)).on_hover_text(asset.as_str());
            if button.clicked() {
                out.actions.push(open_file(view, false));
            }
            anchor = button.rect;
        }
        VideoSource::Screen => {
            if ui
                .button("Choose another screen…")
                .on_hover_text("Ask the desktop again. The picker it shows is its own.")
                .clicked()
            {
                out.actions.push(MainInputAction::ChooseScreenAgain);
            }
        }
        VideoSource::Syphon {
            server,
            flip,
            transparent,
        } if view.syphon_here => syphon_source(ui, view, server, *flip, *transparent, out),
        VideoSource::Ndi {
            source,
            transparent,
        } => ndi_source(ui, view, source, *transparent, out),
        // A Mac's Syphon source where there is none: the select says so, and nothing is under it.
        VideoSource::Syphon { .. } | VideoSource::None => {}
    }
    let current_file = match &view.input.video {
        VideoSource::File { asset } => asset.as_str(),
        _ => "",
    };
    picker(
        ui,
        view,
        false,
        current_file,
        anchor,
        theme,
        &mut out.actions,
    );
}

/// A Syphon source's server, by the label the directory lists it under, and its two looks.
/// A server that is not running stays chosen, and is taken up when it starts.
fn syphon_source(
    ui: &mut Ui,
    view: &MainInputView<'_>,
    server: &str,
    flip: bool,
    transparent: bool,
    out: &mut MainInputOutput,
) {
    let mut pick = server.to_string();
    ComboBox::from_id_salt("main-input-syphon-server")
        .selected_text(if server.is_empty() {
            "No server chosen"
        } else {
            server
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for label in &view.syphon {
                ui.selectable_value(&mut pick, label.clone(), label.as_str());
            }
            if view.syphon.is_empty() {
                ui.label("No Syphon server is running");
            }
        });
    let (mut flipped, mut see_through) = (flip, transparent);
    ui.horizontal(|ui| {
        ui.checkbox(&mut flipped, "Flip")
            .on_hover_text("Top row first, for a server that publishes the other way up.");
        ui.checkbox(&mut see_through, "Transparent")
            .on_hover_text("The server's own alpha, rather than opaque.");
    });
    if (pick.as_str(), flipped, see_through) != (server, flip, transparent) {
        out.actions
            .push(MainInputAction::SetVideo(VideoSource::Syphon {
                server: pick,
                flip: flipped,
                transparent: see_through,
            }));
    }
}

/// An NDI source, by the name the network lists it under, and whether its alpha is kept. A
/// source that is not on the network stays chosen, and is taken up when it appears. Where the
/// runtime is missing, the panel says so under the menu.
fn ndi_source(
    ui: &mut Ui,
    view: &MainInputView<'_>,
    source: &str,
    transparent: bool,
    out: &mut MainInputOutput,
) {
    let mut pick = source.to_string();
    ComboBox::from_id_salt("main-input-ndi-source")
        .selected_text(if source.is_empty() {
            "No source chosen"
        } else {
            source
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for name in &view.ndi {
                ui.selectable_value(&mut pick, name.clone(), name.as_str());
            }
            if view.ndi.is_empty() {
                ui.label(
                    view.ndi_missing
                        .unwrap_or("No NDI® source is on the network"),
                );
            }
        });
    if let Some(why) = view.ndi_missing {
        ui.label(why);
    }
    let mut see_through = transparent;
    ui.checkbox(&mut see_through, "Transparent")
        .on_hover_text("The source's own alpha, rather than opaque.");
    if (pick.as_str(), see_through) != (source, transparent) {
        out.actions
            .push(MainInputAction::SetVideo(VideoSource::Ndi {
                source: pick,
                transparent: see_through,
            }));
    }
}

/// A reference's last part, which is what a button has room for.
fn short(reference: &str) -> String {
    reference
        .rsplit('/')
        .next()
        .unwrap_or(reference)
        .to_string()
}

/// The picture, 16:9 across the panel: the same box a deck gets. Not a node's output: the
/// Main Input's own frame, uploaded under a port of its own so the ordinary blit path draws
/// it. See `maininput::PREVIEW`.
fn picture(ui: &mut Ui, view: &MainInputView<'_>, theme: &Theme, out: &mut MainInputOutput) {
    let preview = crate::maininput::PREVIEW;
    // Words in the box only for no source at all: a source with no frame yet is a black box
    // with the status line under it saying why.
    out.preview = panel::picture(
        ui,
        view.has_picture
            .then_some((preview.node, Some(preview.key))),
        (view.input.video == VideoSource::None).then_some(("No video source", "No video source")),
        theme,
    );
}

fn status_line(ui: &mut Ui, text: &str, theme: &Theme) {
    ui.label(
        eframe::egui::RichText::new(text)
            .font(FontId::proportional(theme::FONT_TINY))
            .color(theme.text_muted()),
    );
}

fn audio_source(ui: &mut Ui, view: &MainInputView<'_>, theme: &Theme, out: &mut MainInputOutput) {
    let current = view.input.audio.clone();
    let mut chosen = current.clone();
    let select = ComboBox::from_id_salt("main-input-audio")
        .selected_text(audio_label(&current, view.devices))
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut chosen, AudioSource::None, "None");
            // The loopback first, above the devices: it is the one a person comes looking
            // for, and it is a standing instruction rather than a device — it follows the
            // default output rather than pinning whichever one is default right now.
            ui.selectable_value(
                &mut chosen,
                AudioSource::system(),
                "System audio (what you hear)",
            );
            ui.selectable_value(
                &mut chosen,
                AudioSource::Live {
                    device: crate::audio::Device::Default,
                },
                "Microphone or line in",
            );
            ui.selectable_value(&mut chosen, AudioSource::Video, "Video source");
            ui.separator();
            for listed in view.devices {
                // The default monitor is already above, under a better name.
                if listed.device.is_system() {
                    continue;
                }
                let label = if listed.monitor {
                    format!("↺ {}", listed.label)
                } else {
                    listed.label.clone()
                };
                ui.selectable_value(
                    &mut chosen,
                    AudioSource::Live {
                        device: listed.device.clone(),
                    },
                    label,
                )
                .on_hover_text(if listed.monitor {
                    "What this output is playing"
                } else {
                    "A capture device"
                });
            }
            ui.separator();
            if ui.button("Sound file…").clicked() {
                out.actions.push(open_file(view, true));
            }
            if ui.button("Look for devices again").clicked() {
                out.actions.push(MainInputAction::RefreshDevices);
            }
        })
        .response
        .rect;
    if chosen != current {
        out.actions.push(MainInputAction::SetAudio(chosen));
    }
    let mut anchor = select;
    let mut current_file = "";
    if let AudioSource::File { asset } = &view.input.audio {
        let button = ui.button(short(asset)).on_hover_text(asset.as_str());
        if button.clicked() {
            out.actions.push(open_file(view, true));
        }
        anchor = button.rect;
        current_file = asset;
    }
    picker(
        ui,
        view,
        true,
        current_file,
        anchor,
        theme,
        &mut out.actions,
    );
}

/// What the project offers for one of the two files: its clips, or its sounds.
fn offered<'a>(
    view: &MainInputView<'a>,
    audio: bool,
) -> impl Iterator<Item = &'a crate::project::AssetInfo> {
    let accepts = if audio {
        crate::nodes::Accepts::AUDIO
    } else {
        crate::nodes::Accepts::VIDEO
    };
    view.assets.iter().filter(move |a| accepts.matches(&a.name))
}

/// The gesture behind a file button: the picker where the project has something of this
/// kind to offer, and straight to the dialog where it does not — a picker with nothing in
/// it is not a choice.
fn open_file(view: &MainInputView<'_>, audio: bool) -> MainInputAction {
    if offered(view, audio).next().is_some() {
        MainInputAction::OpenPicker { audio }
    } else {
        MainInputAction::PickFile { audio }
    }
}

/// The picker under a file button, while it is up: the project's media of this kind as
/// cards, and the dialog as the last entry. The same popup a `video` node's button opens.
fn picker(
    ui: &mut Ui,
    view: &MainInputView<'_>,
    audio: bool,
    current: &str,
    under: eframe::egui::Rect,
    theme: &Theme,
    actions: &mut Vec<MainInputAction>,
) {
    use crate::ui::node_widget::{AssetChoice, Pick, asset_popup};
    if view.picker != Some(audio) {
        return;
    }
    let choices: Vec<AssetChoice<'_>> = offered(view, audio)
        .map(|info| AssetChoice {
            picture: view.posters.get(&info.reference).and_then(Option::as_ref),
            info,
        })
        .collect();
    let picked = asset_popup(
        ui,
        under.left_bottom(),
        ui.available_width().max(under.width()),
        current,
        &choices,
        theme,
    );
    match picked.chosen {
        Some(Pick::Asset(asset)) => {
            actions.push(if audio {
                MainInputAction::SetAudio(AudioSource::File { asset })
            } else {
                MainInputAction::SetVideo(VideoSource::File { asset })
            });
            actions.push(MainInputAction::ClosePicker);
        }
        Some(Pick::File) => {
            actions.push(MainInputAction::PickFile { audio });
            actions.push(MainInputAction::ClosePicker);
        }
        None if picked.dismissed => actions.push(MainInputAction::ClosePicker),
        None => {}
    }
}

/// What the audio select shows for what is chosen.
fn audio_label(source: &AudioSource, devices: &[device::Listed]) -> String {
    match source {
        AudioSource::None => "None".to_string(),
        AudioSource::Video => "Video source".to_string(),
        AudioSource::File { asset } => short(asset),
        AudioSource::Live { device } if device.is_system() => {
            "System audio (what you hear)".to_string()
        }
        AudioSource::Live {
            device: crate::audio::Device::Default,
        } => "Microphone or line in".to_string(),
        AudioSource::Live { device } => devices.iter().find(|d| &d.device == device).map_or_else(
            || "A device that is no longer here".to_string(),
            |d| d.label.clone(),
        ),
    }
}

/// One labeled number across the panel, its label in the panel's label column.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    label: &str,
    value: f32,
    range: crate::graph::ControlRange,
    theme: &Theme,
    lock_cursor: bool,
    make: &mut dyn FnMut(f32) -> MainInputAction,
    actions: &mut Vec<MainInputAction>,
) {
    panel::row(ui, label, theme, |ui| {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, number::HEIGHT), Sense::hover());
        let default = if range.max > 1.0 { 1.0 } else { 0.0 };
        // `Learn` is nothing here: a binding's target is a node's port or the fade, and the
        // Main Input's controls are neither — see docs/media.md.
        if let Some(Scrubbed::Set(v)) = panel::number(
            ui,
            rect,
            label,
            value,
            default,
            range,
            theme,
            lock_cursor,
            false,
        ) {
            actions.push(make(v));
        }
    });
}

/// The spectrum, the band handles and the three meters — the node's scope, in the panel,
/// because here the tuning belongs to the input rather than to any one reader.
fn analyzer(ui: &mut Ui, view: &MainInputView<'_>, theme: &Theme, out: &mut MainInputOutput) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, SCOPE_HEIGHT), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::RADIUS_SM, theme.bg_sunken());
    let Some(scope) = &view.scope else {
        ui.painter().text(
            rect.center(),
            eframe::egui::Align2::CENTER_CENTER,
            "No audio source",
            FontId::proportional(theme::FONT_TINY),
            theme.text_muted(),
        );
        return;
    };
    let inner = rect.shrink(2.0);
    for edit in scope::show(ui, inner, scope::Owner::MAIN_INPUT, scope, theme, 1.0) {
        // The scope speaks in the control keys every audio node uses. Here they are not
        // controls on anything, so they are read back into the band they name.
        let mut freq = None;
        let mut q = None;
        for (key, value) in edit {
            for band in 0..BANDS {
                if key == crate::nodes::audio_ports::FREQS[band] {
                    freq = Some((band, value));
                } else if key == crate::nodes::audio_ports::QS[band] {
                    q = Some(value);
                } else if key == crate::nodes::audio_ports::LEVELS[band] {
                    out.actions
                        .push(MainInputAction::SetLevel { band, level: value });
                }
            }
        }
        if let Some((band, freq)) = freq {
            out.actions.push(MainInputAction::SetBand {
                band,
                freq,
                q: q.unwrap_or(view.input.bands[band].q),
            });
        }
    }
    // What each band is listening to, under its meter: the numbers the two-axis handle sets,
    // said once rather than given six rows of their own.
    ui.horizontal(|ui| {
        for band in 0..BANDS {
            let cfg = view.input.bands[band];
            ui.label(
                eframe::egui::RichText::new(format!("{} {:.0}Hz", bands::NAMES[band], cfg.freq))
                    .font(FontId::proportional(theme::FONT_TINY))
                    .color(theme.band(band)),
            );
        }
    });
}
