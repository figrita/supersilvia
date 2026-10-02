// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ports every audio-capable source publishes, in one place.
//!
//! `video` and `audioin` declare the same bundle because a graph built against one has to
//! play against the other unchanged — the affordances audit calls that the single most
//! copyable idea in silvia's library, and it is only true if both nodes agree down to the
//! port key. Keeping the lists here is what stops them drifting apart.
//!
//! A band's center frequency and Q are **inputs**, not hidden state, for two reasons: they
//! are document data, so tuning a band to a track survives a save; and the scope drags them
//! through `Command::SetControl` like any other control, so the undo history has them.

use crate::audio::bands;

/// The action each threshold fires on, indexed the way `audio::Crossing::band` is: the three
/// bands, then volume.
pub const EVENTS: [&str; bands::BANDS + 1] = ["bassEvent", "midEvent", "highEvent", "volumeEvent"];

/// The control that sets each threshold, in the same order.
pub const LEVELS: [&str; bands::BANDS + 1] = ["bassLevel", "midLevel", "highLevel", "volumeLevel"];

/// Where each band listens, in band order.
pub const FREQS: [&str; bands::BANDS] = ["bassFreq", "midFreq", "highFreq"];

/// How narrowly, in band order.
pub const QS: [&str; bands::BANDS] = ["bassQ", "midQ", "highQ"];

/// A node's own inputs, then the monitor: the one threshold a meter drag cannot set, because
/// it has no meter.
///
/// A macro because `&'static [InputDef]` cannot be concatenated in a `const`, and the whole
/// point is that the two nodes cannot drift apart.
#[macro_export]
macro_rules! audio_inputs {
    ($($own:expr),* $(,)?) => {
        &[
            $($own,)*
            // Zero is off and anything above it is the level: one row rather than a switch
            // and a fader, on nodes that are already too tall. At zero nothing is queued and
            // no output device is opened.
            $crate::nodes::InputDef {
                key: "monitor",
                label: "Monitor",
                ty: $crate::graph::PortType::UniformNumber,
                control: $crate::nodes::Control::num(0.0, 0.0, 1.0, 0.01, ""),
            },
        ]
    };
}

/// A node's own outputs, then the three bands and the event each fires.
///
/// `volumeEvent` is not here: the microphone is the only source with a volume worth
/// thresholding on its own, and it declares that one itself.
#[macro_export]
macro_rules! audio_outputs {
    ($($own:expr),* $(,)?) => {
        &[
            $($own,)*
            $crate::nodes::OutputDef {
                key: "bass",
                label: "Bass",
                ty: $crate::graph::PortType::UniformNumber,
                kind: $crate::nodes::OutputKind::Uniform,
                ..OutputDef::EMPTY
            },
            $crate::nodes::OutputDef {
                key: "mid",
                label: "Mid",
                ty: $crate::graph::PortType::UniformNumber,
                kind: $crate::nodes::OutputKind::Uniform,
                ..OutputDef::EMPTY
            },
            $crate::nodes::OutputDef {
                key: "high",
                label: "High",
                ty: $crate::graph::PortType::UniformNumber,
                kind: $crate::nodes::OutputKind::Uniform,
                ..OutputDef::EMPTY
            },
            // A CPU array as a one-dimensional lookup texture a color output samples.
            // The waveform is one use; a palette, an automation curve, a sequencer lane
            // and an LFO table all want the same primitive.
            $crate::nodes::OutputDef {
                key: "oscilloscope",
                label: "Oscilloscope",
                ty: $crate::graph::PortType::VaryingColor,
                kind: $crate::nodes::OutputKind::Texture,
                wgsl: |node, ctx, _func| {
                    let tex = ctx.texture_uniform(node, "oscilloscope");
                    let sampler = ctx.sampler(node, "oscilloscope");
                    format!(
                        "    let wave = textureSampleLevel({tex}, {sampler}, vec2f((uv.x + 1.0) * 0.5, 0.5), 0.0).r;
    let line = (wave - 0.5) * 2.0 * 0.95;
    return select(vec4f(0.0), vec4f(1.0), abs(uv.y - line) < 0.02);"
                    )
                },
                ..OutputDef::EMPTY
            },
            $crate::nodes::OutputDef {
                key: "bassEvent",
                label: "Bass Event",
                ty: $crate::graph::PortType::Action,
                kind: $crate::nodes::OutputKind::Action,
                ..OutputDef::EMPTY
            },
            $crate::nodes::OutputDef {
                key: "midEvent",
                label: "Mid Event",
                ty: $crate::graph::PortType::Action,
                kind: $crate::nodes::OutputKind::Action,
                ..OutputDef::EMPTY
            },
            $crate::nodes::OutputDef {
                key: "highEvent",
                label: "High Event",
                ty: $crate::graph::PortType::Action,
                kind: $crate::nodes::OutputKind::Action,
                ..OutputDef::EMPTY
            },
        ]
    };
}

/// Where each band listens, how narrowly, and the level it fires at. **Hidden**: no port, no
/// row. The frequency and Q are dragged on the scope as one two-dimensional handle each; the
/// level is the meter's own threshold square, in `audio_ports::LEVELS` order — a number row
/// beside it would only say what the square already shows.
///
/// Log ranges on frequency and Q, because a band moved from 100 Hz to 8 kHz spends its whole
/// linear travel in the bottom decade otherwise — and the plot it is dragged on has a log axis
/// for the same reason. The level is linear: it is a fraction of the meter, not a span.
pub const TUNING: &[crate::nodes::InputDef] = &[
    tuned("bassFreq", "Bass Freq", 100.0, 20.0, 20_000.0, "Hz"),
    tuned("bassQ", "Bass Q", 1.0, Q_MIN, Q_MAX, ""),
    tuned("midFreq", "Mid Freq", 1000.0, 20.0, 20_000.0, "Hz"),
    tuned("midQ", "Mid Q", 1.0, Q_MIN, Q_MAX, ""),
    tuned("highFreq", "High Freq", 8000.0, 20.0, 20_000.0, "Hz"),
    tuned("highQ", "High Q", 1.0, Q_MIN, Q_MAX, ""),
    level("bassLevel", "Bass At"),
    level("midLevel", "Mid At"),
    level("highLevel", "High At"),
];

/// The ends of the Q axis, which the plot maps its height to.
pub const Q_MIN: f32 = 0.3;
pub const Q_MAX: f32 = 12.0;

/// The ends of the frequency axis, which the plot maps its width to.
pub const FREQ_MIN: f32 = 20.0;
pub const FREQ_MAX: f32 = 20_000.0;

const fn tuned(
    key: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    unit: &'static str,
) -> crate::nodes::InputDef {
    crate::nodes::InputDef {
        key,
        label,
        ty: crate::graph::PortType::UniformNumber,
        control: crate::nodes::Control::num_log(default, min, max, 0.01, unit),
    }
}

/// One by default, which is the top of a band's range: nothing fires until a hand moves it
/// down, because a node that opened firing is not one anybody asked for.
const fn level(key: &'static str, label: &'static str) -> crate::nodes::InputDef {
    crate::nodes::InputDef {
        key,
        label,
        ty: crate::graph::PortType::UniformNumber,
        control: crate::nodes::Control::num(1.0, 0.0, 1.0, 0.01, ""),
    }
}

/// How much of an audio node is drawn: one tick per part, in one row of their own.
///
/// silvia's own `Numbers` / `Events` / `Scope`, in silvia's order — the first two as ticks
/// sharing one row, the third as the heading over the region it opens. They were a
/// single three-way `show` select here — `All` / `Meters only` / `Ports only` — which could
/// not say "the events and the scope, but not the uniform numbers", because the three
/// answers are not a ladder. A tick is still an option, so the file, the undo history and
/// the command bus are unchanged; only the row's shape is. See
/// docs/decisions.md#the-scope-a-two-axis-handle-and-a-row-of-ticks-for-the-rest.
///
/// `Presentation`, all three: only the canvas reads them, so hiding half a node must not
/// rebuild the shaders downstream of it.
pub const SHOW_UNIFORMS: crate::nodes::OptionDef = crate::nodes::OptionDef::check(
    "uniforms",
    "Uniforms",
    true,
    crate::nodes::OptionKind::Presentation,
);
pub const SHOW_EVENTS: crate::nodes::OptionDef = crate::nodes::OptionDef::check(
    "events",
    "Events",
    true,
    crate::nodes::OptionKind::Presentation,
);
pub const SHOW_SCOPE: crate::nodes::OptionDef = crate::nodes::OptionDef::heading(
    "scope",
    "Scope",
    true,
    crate::nodes::OptionKind::Presentation,
);

/// The waveform as a 512x1 frame, for the oscilloscope's texture output.
///
/// RGBA, which wastes three quarters of two kilobytes a frame. A single-channel layout would
/// be a change to `render/` for a saving nobody could measure. A node publishing one every
/// tick goes through [`Waveform`], which reuses the allocation.
pub fn waveform_frame(waveform: &[u8; crate::audio::WAVEFORM_LEN]) -> crate::nodes::Frame {
    let mut pixels = vec![0; waveform.len() * 4];
    fill(&mut pixels, waveform);
    crate::nodes::Frame {
        width: waveform.len() as u32,
        height: 1,
        pixels: crate::nodes::Pixels::Bytes(pixels),
    }
}

/// The oscilloscope's frames, rewritten in place rather than allocated each tick.
///
/// The renderer lets go of a frame once it has copied it up, keeping only a `Weak` to know it
/// by, and the next frame replaces that. So a tick or two later the frame published before
/// is held by nothing but this, `Arc::get_mut` finds it unique, and the samples are written
/// into the allocation it already has. A frame somebody still holds is left alone, since the
/// renderer tells frames apart by pointer and a rewritten one would look already uploaded.
#[derive(Default)]
pub struct Waveform {
    frames: Vec<std::sync::Arc<crate::nodes::Frame>>,
}

impl Waveform {
    /// How many frames are kept for reuse. Two is the steady state; the rest is room for a
    /// tick in which the renderer or the editor is still holding one.
    const KEPT: usize = 4;

    /// This tick's waveform as a frame, in an allocation of the last few where one is free.
    pub fn frame(
        &mut self,
        waveform: &[u8; crate::audio::WAVEFORM_LEN],
    ) -> std::sync::Arc<crate::nodes::Frame> {
        let free = self
            .frames
            .iter_mut()
            .position(|f| std::sync::Arc::get_mut(f).is_some());
        if let Some(i) = free
            && let Some(crate::nodes::Frame {
                pixels: crate::nodes::Pixels::Bytes(bytes),
                ..
            }) = std::sync::Arc::get_mut(&mut self.frames[i])
        {
            fill(bytes, waveform);
            return std::sync::Arc::clone(&self.frames[i]);
        }
        let frame = std::sync::Arc::new(waveform_frame(waveform));
        if self.frames.len() == Self::KEPT {
            self.frames.remove(0);
        }
        self.frames.push(std::sync::Arc::clone(&frame));
        frame
    }
}

/// Each sample as an opaque gray texel, into bytes already the right length.
fn fill(bytes: &mut [u8], waveform: &[u8; crate::audio::WAVEFORM_LEN]) {
    for (texel, sample) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(waveform) {
        *texel = [*sample, *sample, *sample, 255];
    }
}

/// Read the band tuning a node's controls are asking for.
pub fn config(
    id: crate::graph::NodeId,
    ctx: &crate::nodes::TickContext<'_>,
) -> [bands::BandConfig; bands::BANDS] {
    std::array::from_fn(|b| bands::BandConfig {
        freq: ctx.input(id, FREQS[b]).max(1.0),
        q: ctx.input(id, QS[b]).max(0.05),
        ..bands::DEFAULT[b]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::WAVEFORM_LEN;
    use std::sync::Arc;

    /// A frame nobody else holds any more is rewritten in place; one still held — by the
    /// renderer, by the editor — is left alone and another is made beside it.
    #[test]
    fn the_waveform_reuses_a_frame_nobody_holds() {
        let mut scope = Waveform::default();
        let first = scope.frame(&[10; WAVEFORM_LEN]);
        let at = Arc::as_ptr(&first);
        let second = scope.frame(&[20; WAVEFORM_LEN]);
        assert_ne!(Arc::as_ptr(&second), at, "the first is still held");
        assert_eq!(first.bytes().unwrap()[0], 10, "and was not written over");

        drop(first);
        let third = scope.frame(&[30; WAVEFORM_LEN]);
        assert_eq!(Arc::as_ptr(&third), at, "the released one is reused");
        assert_eq!(&third.bytes().unwrap()[..4], &[30, 30, 30, 255]);

        // A `Weak` is how the renderer remembers what it uploaded; a frame it still knows by
        // pointer must not be rewritten, or it would look already uploaded.
        let weak = Arc::downgrade(&third);
        drop((second, third));
        let fourth = scope.frame(&[40; WAVEFORM_LEN]);
        assert_ne!(Arc::as_ptr(&fourth), weak.as_ptr());
    }
}
