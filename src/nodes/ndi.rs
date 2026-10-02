// SPDX-License-Identifier: AGPL-3.0-or-later

//! A picture another machine sends over NDI®, as a texture — each node a receiver of its own,
//! so a patch can take several at once.
//!
//! The rig's one NDI source is the [Main Input panel's](../../../docs/media.md#the-main-input);
//! this is the node beside it, as `syphon` is beside the panel's Syphon server. The source is
//! chosen from a menu of what the network has now, by the name NDI gives it,
//! `MACHINE (Stream)`, which is what a project saves; one that is not there is looked for again,
//! and taken up when it appears ([`crate::video::ndi`]). **Transparent** keeps its alpha rather
//! than reading it opaque. Where the NDI runtime is missing, the node's status line says so.

use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, CpuDef, CpuNode, Frame, NodeDef, OptionDef, OptionKind, OutputDef, OutputKind,
    TickContext,
};
use crate::video::ndi::{NONE, Receiver, menu};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "ndi",
    category: Category::Source,
    icon: "🛰",
    label: "NDI",
    tooltip: "A picture another machine on the network sends over NDI®, as a texture. Choose \
              the source by its machine and its name; one that is not on the network is taken \
              up when it appears. Needs the NDI® runtime, from ndi.video.",
    outputs: &[OutputDef {
        key: "frame",
        label: "Frame",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // The camera's mapping exactly: worldspace into the frame's own [0,1] by its real
        // aspect, v flipped because the rows arrive top first, and mirrored outward past its
        // own edge by the texture's own wrap mode.
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
            key: "source",
            label: "Source",
            default: "",
            // The sources the network lists now; *None* alone where it lists none.
            found: Some(menu),
            choices: NONE,
            // Read by `tick`, which opens it.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef::check("transparent", "Transparent", false, OptionKind::Runtime),
        crate::nodes::SHOW_PREVIEW,
    ],
    regions: &[
        crate::nodes::Region::Status,
        crate::nodes::Region::Preview("frame"),
    ],
    cpu: Some(CpuDef {
        create: || Box::new(NdiNode::new()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

struct NdiNode {
    receiver: Option<Receiver>,
    /// Published until a frame arrives, so the sampler always has a texture.
    black: Arc<Frame>,
}

impl NdiNode {
    fn new() -> Self {
        Self {
            receiver: None,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
        }
    }
}

impl CpuNode for NdiNode {
    fn reset(&mut self) {
        // Nothing accumulates, and the receiver is kept, as a Syphon client is.
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let source = ctx.option(id, "source").to_string();
        let transparent = ctx.option(id, "transparent") == crate::nodes::ON;
        if source.is_empty() {
            self.receiver = None;
        } else if !self
            .receiver
            .as_ref()
            .is_some_and(|r| r.is(&source, transparent))
        {
            self.receiver = Some(Receiver::new(&source, transparent));
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
        if let Some(why) = crate::video::ndi::missing() {
            return Some(why.to_string());
        }
        Some(match &self.receiver {
            None => "no source chosen".to_string(),
            Some(r) if r.receiving() => format!("receiving {}", r.source()),
            Some(r) => format!("waiting for {}", r.source()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node with no source chosen says so, or says the runtime is missing, and publishes
    /// black rather than nothing.
    #[test]
    fn a_node_with_no_source_says_so() {
        let node = NdiNode::new();
        let status = node.status().expect("a status line");
        assert!(
            status == "no source chosen" || status.starts_with("The NDI® runtime"),
            "{status}"
        );
        assert!(node.error().is_none());
    }
}
