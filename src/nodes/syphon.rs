// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture another app publishes over Syphon, as a texture — each node a client of its own,
//! so a patch can take several at once.
//!
//! The rig's one Syphon source is the [Main Input panel's](../../../docs/media.md#the-main-input);
//! this is the node beside it, as `screencapture` is beside the panel's screen. The server is
//! chosen from a menu of what the Mac's Syphon directory lists now, by "App – Server", which
//! is what a project saves; one that is not running is looked for again every second, so it is
//! taken up when it starts ([`crate::video::syphon`]). **Flip** reads the surface top row first
//! rather than Syphon's bottom row first, and **Transparent** keeps its alpha rather than
//! reading it opaque. Linux has no Syphon: the library does not offer the node there, and one
//! that arrives in a file from a Mac lists nothing and says so.

use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, CpuDef, CpuNode, Frame, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
    TickContext,
};
use crate::platform::syphon::Look;
use crate::video::syphon::{NONE, Receiver, menu};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "syphon",
    category: Category::Source,
    icon: "📡",
    label: "Syphon",
    tooltip: "A picture another app on this Mac publishes over Syphon, as a texture. Choose the \
              server by its app and its name; one that is not running is taken up when it starts.",
    outputs: &[OutputDef {
        key: "frame",
        label: "Frame",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // The camera's mapping exactly: worldspace into the frame's own [0,1] by its real
        // aspect, v flipped because the copy leaves the rows top first, and mirrored outward
        // past its own edge by the texture's own wrap mode.
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
    }],
    options: &[
        OptionDef {
            key: "server",
            label: "Server",
            default: "",
            // The servers the directory lists now; *None* alone where it lists none.
            found: Some(menu),
            choices: NONE,
            // Read by `tick`, which connects to it.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef::check("flip", "Flip", false, OptionKind::Runtime),
        OptionDef::check("transparent", "Transparent", false, OptionKind::Runtime),
        crate::nodes::SHOW_PREVIEW,
    ],
    regions: &[
        crate::nodes::Region::Status,
        crate::nodes::Region::Preview("frame"),
    ],
    cpu: Some(CpuDef {
        create: || Box::new(SyphonNode::new()),
        integrates: false,
        live: true,
    }),
    offered: crate::platform::syphon::available,
    ..NodeDef::EMPTY
};

struct SyphonNode {
    receiver: Option<Receiver>,
    /// Published until a frame arrives, so the sampler always has a texture.
    black: Arc<Frame>,
}

impl SyphonNode {
    fn new() -> Self {
        Self {
            receiver: None,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
        }
    }
}

impl CpuNode for SyphonNode {
    fn reset(&mut self) {
        // Nothing accumulates, and the connection is kept, as a screen capture's is.
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let server = ctx.option(id, "server").to_string();
        let on = |key: &str| ctx.option(id, key) == crate::nodes::ON;
        let look = Look {
            flip: on("flip"),
            transparent: on("transparent"),
        };
        if server.is_empty() {
            self.receiver = None;
        } else if !self.receiver.as_ref().is_some_and(|r| r.is(&server, look)) {
            self.receiver = Some(Receiver::new(&server, look));
        }
        let frame = self.receiver.as_mut().and_then(Receiver::latest);
        ctx.publish_frame(
            id,
            "frame",
            frame.unwrap_or_else(|| Arc::clone(&self.black)),
        );
    }

    fn error(&self) -> Option<String> {
        self.receiver.as_ref().and_then(Receiver::error)
    }

    /// What it is doing, across the foot of the body.
    fn status(&self) -> Option<String> {
        Some(match &self.receiver {
            None => crate::platform::syphon::unavailable()
                .map_or_else(|| "no server chosen".to_string(), str::to_string),
            Some(r) if r.connected() => format!("receiving {}", r.server()),
            Some(r) => format!("waiting for {}", r.server()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node with no server chosen says so, and publishes black rather than nothing.
    #[test]
    fn a_node_with_no_server_says_so() {
        let node = SyphonNode::new();
        let status = node.status().expect("a status line");
        assert!(
            status == "no server chosen" || status.contains("Syphon"),
            "{status}"
        );
        assert!(node.error().is_none());
    }
}
