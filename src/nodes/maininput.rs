// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Main Input, as a node: the picture and the analysis the left panel is holding open.
//!
//! **It owns nothing.** Every other source node here opens its own device — that is what
//! `camera`, `video` and `audioin` are — and this one is the other shape silvia has: one
//! source chosen once, read wherever it is wanted. Eight of these in a project open one
//! camera and analyze one signal, which is the whole reason the node exists. Choose the
//! source in the panel; put one of these wherever the patch needs it.
//!
//! **So it has no tuning and no thresholds of its own.** They are the panel's, and not as a
//! matter of taste: the bands are measured and the thresholds are crossed on the audio thread,
//! inside the one capture all of these read, so a per-node copy would mean whichever node
//! ticked last decided what every one of them saw. A node that wants its own tuning is an
//! `audioin`, which owns its capture and can have one.
//!
//! Its ports are the bundle `video` and `audioin` publish, down to the key, so a patch built
//! against a clip plays against the panel's camera unchanged.

use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, CpuDef, CpuNode, NodeDef, OutputDef, OutputKind, TickContext, audio_ports,
};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "maininput",
    category: Category::Source,
    icon: "🎛",
    label: "Main Input",
    tooltip: "The source chosen in the Main Input panel: a camera, a screen, a clip, the \
              microphone or the system audio. Every one of these reads the same one.",
    // No `monitor` of its own: the panel has one, and one input heard through one slider is
    // the point. So not `audio_inputs!`, which would add a second.
    inputs: &[],
    outputs: crate::audio_outputs![
        OutputDef {
            key: "frame",
            label: "Frame",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The camera's mapping exactly: worldspace into the frame's own [0,1] by its real
            // aspect, v flipped once because rows are uploaded top first, and mirrored outward
            // past its own edge by the texture's own wrap mode, set in render/mod.rs.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "frame");
                let sampler = ctx.sampler(node, "frame");
                format!(
                    "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "level",
            label: "Level",
            ty: crate::graph::PortType::UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "peak",
            label: "Peak",
            ty: crate::graph::PortType::UniformNumber,
            kind: OutputKind::Uniform,
            ..OutputDef::EMPTY
        },
    ],
    // silvia's own two ticks, the ones `audioin` carries: a Main Input used for its picture
    // alone is a short node with one row, where it used to be ten whether or not anything in
    // the patch listened to the sound.
    options: &[audio_ports::SHOW_UNIFORMS, audio_ports::SHOW_EVENTS],
    // **Meters, not a picture.** silvia draws three level bars on this node with the trigger
    // handle on each, and that is where a level wants to be set: on the band being watched,
    // beside the row being cabled. The picture came off with them — the panel is already
    // showing it, a few inches to the left, and one rig's picture drawn twice is once too
    // many. See docs/decisions.md.
    regions: &[crate::nodes::Region::Meters],
    cpu: Some(CpuDef {
        create: || Box::new(MainInputNode::default()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// The four published numbers, in output order.
const OUTPUTS: [&str; 5] = ["bass", "mid", "high", "level", "peak"];

#[derive(Default)]
struct MainInputNode {
    /// Silence as a waveform, made once: what the oscilloscope draws with nothing playing.
    flat: Option<Arc<crate::nodes::Frame>>,
    /// What the last tick read, for the meters on the body. Kept on the node because
    /// `CpuNode::scope` is handed no context — and read rather than measured, since the
    /// measuring is the one capture's.
    last: Option<crate::audio::Scope>,
    /// The oscilloscope's frames, reused from tick to tick.
    scope: audio_ports::Waveform,
}

impl CpuNode for MainInputNode {
    fn reset(&mut self) {
        // Nothing accumulates: the panel holds the source, and `flat` is a picture of silence.
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let input = ctx.main_input();
        let gain = input.gain;
        let frame = input.frame.map(Arc::clone);
        let analysis = input.analysis;
        let raw = [
            analysis.bands[0],
            analysis.bands[1],
            analysis.bands[2],
            analysis.rms,
            analysis.peak,
        ];

        // Every one of these nodes fires on the same edges of the same signal, so the
        // crossings are collected once a frame by the app and handed to all of them. Placing
        // one inside the frame is `audioin`'s arithmetic: how far back in the block it was,
        // plus how long the block has been sitting in the triple buffer.
        let rate = input.sample_rate.max(1.0);
        let stale = analysis.age().map_or(0.0, |a| a.as_secs_f32());
        for crossing in input.crossings {
            let Some(port) = audio_ports::EVENTS.get(crossing.band as usize) else {
                continue;
            };
            let behind = analysis.samples.saturating_sub(crossing.at) as f32 / rate;
            let at = (ctx.dt - (stale + behind)).clamp(0.0, ctx.dt);
            let edge = if crossing.down {
                crate::nodes::Edge::Down
            } else {
                crate::nodes::Edge::Up
            };
            ctx.fire_at(id, port, edge, at);
        }

        // The picture the meters draw, built here because this is where the feed is: the
        // panel's tuning, the panel's thresholds, and the levels *before* the gain, so the
        // three bars on a node say what the three bars on the panel say.
        self.last = Some(crate::audio::Scope::new(
            &analysis,
            input.config,
            input.thresholds,
            rate,
        ));

        let flat = self.flat.get_or_insert_with(|| {
            Arc::new(audio_ports::waveform_frame(
                &[128; crate::audio::WAVEFORM_LEN],
            ))
        });
        ctx.publish_frame(
            id,
            "oscilloscope",
            if analysis.published.is_some() {
                self.scope.frame(&analysis.waveform)
            } else {
                Arc::clone(flat)
            },
        );
        if let Some(frame) = frame {
            ctx.publish_frame(id, "frame", frame);
        }
        for (i, key) in OUTPUTS.iter().enumerate() {
            ctx.publish(id, key, f64::from(raw[i] * gain));
        }
    }

    /// The rig's bands, for the meters region.
    ///
    /// `Some` from the first tick and never `None`, for the reason `video`'s is: silence is a
    /// picture of silence, and a node just spawned with an empty band inside its border reads
    /// as broken rather than as quiet.
    fn scope(&self) -> Option<crate::audio::Scope> {
        self.last
    }
}
