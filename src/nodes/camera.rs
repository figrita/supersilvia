// SPDX-License-Identifier: AGPL-3.0-or-later

//! A camera, as a texture.
//!
//! The pipeline is opened on the first tick and rebuilt whenever the options change. A
//! camera another node or the Main Input already has open is read from the pipeline running
//! on it, at the size it was opened at, and a different Size is the node's error
//! ([`crate::video`]). The texture output is `delayed`, like an Output's `frame`: a consumer samples what the
//! camera last published rather than descending into anything.

use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, CpuDef, CpuNode, Frame, NodeDef, OptionDef, OutputDef, OutputKind, TickContext,
};
use crate::video::silence::{NOT_RESPONDING_SAYS, Silence};
use crate::video::{Camera, Source};
use std::sync::Arc;
use std::time::Instant;

pub static DEF: NodeDef = NodeDef {
    slug: "camera",
    category: Category::Source,
    icon: "📹",
    label: "Camera",
    tooltip: "A video device, published as a texture. Black until the first frame arrives.",
    outputs: &[OutputDef {
        key: "frame",
        label: "Frame",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // Worldspace to the frame's own [0,1], using the frame's real aspect, so a 4:3
        // camera in a 16:9 Output sits centerd and mirrors outward past its own edge — the
        // texture's own wrap mode, set in render/mod.rs. Rows are uploaded top first, so v
        // is flipped here once.
        wgsl: |node, ctx, _func| {
            let tex = ctx.texture_uniform(node, "frame");
            let sampler = ctx.sampler(node, "frame");
            // `t` is a `var` only where the mirror assigns it again.
            let (binding, mirror) = if ctx.option(node, "mirror") == "yes" {
                ("var", "    t.x = 1.0 - t.x;\n")
            } else {
                ("let", "")
            };
            format!(
                "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    {binding} t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);
{mirror}    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "device",
            label: "Device",
            default: "auto",
            // The cameras the Main Input's listing found, by name, where a camera is saved by
            // an ID a person never reads: AVFoundation's unique ID on a Mac. Linux's paths are
            // the choices below.
            found: Some(crate::video::camera_menu),
            devices: true,
            choices: &[
                ("auto", "Auto"),
                ("/dev/video0", "/dev/video0"),
                ("/dev/video1", "/dev/video1"),
                ("/dev/video2", "/dev/video2"),
                ("/dev/video3", "/dev/video3"),
                ("test", "Test pattern"),
            ],
            // Read by `tick`, which opens the pipeline on it.
            kind: crate::nodes::OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "size",
            label: "Size",
            default: "auto",
            choices: &[
                ("auto", "Auto"),
                ("640x480", "640x480"),
                ("1280x720", "1280x720"),
                ("1920x1080", "1920x1080"),
            ],
            // Read by `tick` as the capture caps; the frame arrives as a texture.
            kind: crate::nodes::OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "mirror",
            label: "Mirror",
            default: "yes",
            choices: &[("yes", "Yes"), ("no", "No")],
            ..OptionDef::EMPTY
        },
    ],
    cpu: Some(CpuDef {
        create: || Box::new(CameraNode::new()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// What the options asked for, so a tick can tell whether the pipeline still matches.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wanted {
    source: Source,
    size: Option<(u32, u32)>,
}

struct CameraNode {
    camera: Option<Camera>,
    wanted: Option<Wanted>,
    error: Option<String>,
    /// How long the open camera has delivered nothing new.
    silence: Silence,
    /// The frame last delivered, so a new one is told from the same one handed out again.
    last: Option<Arc<Frame>>,
    /// The camera is open and has delivered nothing for ten seconds.
    stalled: bool,
    /// Published until the first real frame, so the sampler always has a texture.
    black: Arc<Frame>,
    frames: u64,
}

impl CameraNode {
    fn new() -> Self {
        Self {
            camera: None,
            wanted: None,
            error: None,
            silence: Silence::new(Instant::now()),
            last: None,
            stalled: false,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            frames: 0,
        }
    }

    fn open(&mut self, wanted: Wanted) {
        self.silence = Silence::new(Instant::now());
        self.last = None;
        self.stalled = false;
        match Camera::open(&wanted.source, wanted.size) {
            Ok(c) => {
                self.camera = Some(c);
                self.error = None;
            }
            Err(e) => {
                self.camera = None;
                self.error = Some(e);
            }
        }
        self.wanted = Some(wanted);
    }
}

impl CpuNode for CameraNode {
    fn reset(&mut self) {
        self.frames = 0;
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let source = match ctx.option(id, "device") {
            "auto" | "" => Source::Auto,
            "test" => Source::Test,
            path => Source::Device(path.to_string()),
        };
        let size = crate::nodes::output::parse_resolution(ctx.option(id, "size"));
        let wanted = Wanted { source, size };
        if self.wanted.as_ref() != Some(&wanted) {
            // Let go of the old reader before opening the new one, so a device this node
            // alone reads stops before it is opened at another size.
            self.camera = None;
            self.open(wanted);
        }

        let frame = match self.camera.as_mut() {
            Some(camera) => {
                if let Some(e) = camera.error() {
                    self.error = Some(e);
                }
                camera.latest()
            }
            None => None,
        };
        let now = Instant::now();
        if let Some(fresh) = &frame
            && !self.last.as_ref().is_some_and(|l| Arc::ptr_eq(l, fresh))
        {
            self.silence.heard(now);
            self.last = Some(Arc::clone(fresh));
        }
        self.stalled = self.camera.is_some() && self.silence.not_responding(now);
        let frame = frame.unwrap_or_else(|| Arc::clone(&self.black));
        self.frames += 1;
        ctx.publish_frame(id, "frame", frame);
    }

    /// A device that would not open or that failed, and otherwise one that has delivered
    /// nothing for ten seconds: what the flag on the header says.
    fn error(&self) -> Option<String> {
        self.error
            .clone()
            .or_else(|| self.stalled.then(|| NOT_RESPONDING_SAYS.to_string()))
    }

    fn debug(&self) -> Option<String> {
        let camera = self.camera.as_ref()?;
        Some(format!("camera {}", camera.description))
    }
}
