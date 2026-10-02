// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Input panel: what it draws from, and what a gesture on it does.
//!
//! The device, the clip and the analysis behind all of this are the synth's — see
//! [`crate::synth::maininput`]. What is here is the editor's half: a view assembled from
//! what the panel is showing, and the choices a hand makes on it, which are the project's
//! and saved with it.

impl crate::App {
    /// What the panel draws from, assembled once a frame.
    pub(super) fn main_input_view(&self) -> crate::ui::maininput::MainInputView<'_> {
        let collapsed = self.prefs.get().main_input_collapsed;
        // The scope wants the tuning the analysis was actually made with, which is the
        // project's: the panel sets it and the audio thread reads it, and nothing between the
        // two can disagree. `None` with nothing to hear, so the section says so rather than
        // drawing a flat line and looking broken.
        let live = &self.link.snapshot().main_input;
        let scope = live.hears.then(|| {
            let want = self.project.main_input();
            crate::audio::Scope::new(&live.analysis, want.bands, want.levels, live.sample_rate)
        });
        crate::ui::maininput::MainInputView {
            input: self.project.main_input(),
            video_status: &live.video_status,
            audio_status: &live.audio_status,
            error: live.error.as_deref(),
            devices: &live.devices,
            cameras: &live.cameras,
            syphon: crate::video::syphon::labels(),
            syphon_here: crate::platform::syphon::available(),
            // Listed only while an NDI source is chosen: the first listing starts NDI's
            // discovery on the network, which nobody who never chose NDI should pay for.
            ndi: if matches!(
                self.project.main_input().video,
                crate::maininput::VideoSource::Ndi { .. }
            ) {
                crate::video::ndi::labels()
            } else {
                Vec::new()
            },
            ndi_missing: crate::video::ndi::missing(),
            scope,
            has_picture: live.has_picture,
            collapsed,
            assets: self.media.assets(),
            posters: self.media.asset_thumbnails(),
            picker: self.main_input_picker,
        }
    }

    /// One gesture on the panel. None of these is a command: choosing a source is playing
    /// rather than editing, so none of them enters the undo history or marks the project
    /// unsaved. The next save writes it.
    pub fn handle_main_input(&mut self, action: crate::ui::maininput::MainInputAction) {
        use crate::ui::maininput::MainInputAction as A;
        match action {
            A::SetVideo(source) => self.project.main_input_mut().video = source,
            A::SetAudio(source) => self.project.main_input_mut().audio = source,
            A::SetGain(v) => self.project.main_input_mut().gain = v,
            A::SetMonitor(v) => self.project.main_input_mut().monitor = v,
            A::SetBand { band, freq, q } => {
                if let Some(cfg) = self.project.main_input_mut().bands.get_mut(band) {
                    cfg.freq = freq;
                    cfg.q = q;
                }
            }
            A::SetLevel { band, level } => {
                if let Some(slot) = self.project.main_input_mut().levels.get_mut(band) {
                    *slot = level;
                }
            }
            A::PickFile { audio } => {
                self.ask_for_file(crate::app::files::FileAsk::MainInput { audio });
            }
            A::OpenPicker { audio } => self.main_input_picker = Some(audio),
            A::ClosePicker => self.main_input_picker = None,
            // The desktop's own picker again. Nothing is resumed: no restore token is kept.
            A::ChooseScreenAgain => self.link.send(crate::synth::Msg::ReopenVideo),
            A::SetCollapsed(on) => self.prefs.set_main_input_collapsed(on),
            A::RefreshDevices => self.link.send(crate::synth::Msg::ForgetDevices),
        }
    }
}
