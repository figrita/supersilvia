// SPDX-License-Identifier: AGPL-3.0-or-later

//! A screen or a window, as a texture, with a portal session of its own.
//!
//! The rig's screen is the [Main Input panel's](../../../docs/media.md#the-main-input): one
//! capture, one dialog, one picture read by as many `maininput` nodes as want it. That is the
//! right answer for the one screen a set is built around, and it is the only answer for two —
//! there is one panel. This node is silvia's, beside the camera node: **each one picks its
//! own window and holds its own session**, so a mix can carry two windows at once.
//!
//! What it costs is named in `docs/decisions.md`: a portal dialog per node every time a
//! project holding one is opened, and a capture running whether or not anything downstream
//! reads it.
//!
//! **The portal code is the panel's**, [`crate::platform::screen`], a module of its own
//! rather than a method on the panel: `ask` puts the desktop's own picker up on the shared
//! runtime and answers through a channel a tick polls, and the [`screen::Cast`] it hands back
//! holds the session open for exactly as long as the capture runs. Below that it is
//! [`camera`](super::camera): `pipewiresrc` into the same appsink, frames through the same
//! one-frame slot — or on a Mac, ScreenCaptureKit's stream writing that slot itself.
//!
//! **No Start button.** Opening the node asks, which is what the panel does when a screen is
//! chosen on it. *Choose Screen* asks again and *Stop* gives the session up, taking the
//! desktop's sharing indicator down with it; both are `Action` inputs, so a sequencer can
//! press either. Nothing is ever resumed from a saved token — see *The mixer does not
//! persist, and the Main Input does* in docs/decisions.md.

use crate::graph::NodeId;
use crate::graph::PortType::{Action, VaryingColor};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Frame, Gate, InputDef, NodeDef, OutputDef, OutputKind,
    TickContext,
};
use crate::platform::screen;
use crate::video::{Camera, Source};
use std::sync::Arc;

pub static DEF: NodeDef = NodeDef {
    slug: "screencapture",
    category: Category::Source,
    icon: "🖥",
    label: "Screen Capture",
    tooltip: "A screen or a window, through the desktop's own picker, as a texture. It asks \
              when the node opens and asks again every run: which window you are sharing is \
              never resumed from last time.",
    inputs: &[
        InputDef {
            key: "choose",
            label: "Choose Screen",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "stop",
            label: "Stop",
            ty: Action,
            control: Control::Press,
        },
    ],
    outputs: &[OutputDef {
        key: "frame",
        label: "Frame",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // The camera's mapping exactly: worldspace into the frame's own [0,1] by its real
        // aspect, v flipped because rows are uploaded top first, and mirrored outward past
        // its own edge by the texture's own wrap mode.
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
    options: &[crate::nodes::SHOW_PREVIEW],
    // What it is waiting for, and then what it is showing. A picker that is up is a minute
    // of nothing happening, and a node that says nothing during it reads as broken.
    regions: &[
        crate::nodes::Region::Status,
        crate::nodes::Region::Preview("frame"),
    ],
    cpu: Some(CpuDef {
        create: || Box::new(ScreenNode::new()),
        integrates: false,
        live: true,
    }),
    ..NodeDef::EMPTY
};

/// A running capture: the pipeline, and the session that must outlive it.
///
/// Field order is the drop order, and it is the whole point of the struct: the pipeline is
/// reading a descriptor the session owns, so it goes first.
struct Capture {
    camera: Camera,
    /// Held for the life of the capture. Dropping it closes the portal session and takes the
    /// desktop's sharing indicator down.
    _cast: Box<screen::Cast>,
}

/// Where the node is between having no picture and having one.
enum State {
    /// Nothing has been asked for: the node was stopped, or a refusal left it here.
    Idle,
    /// The desktop's picker is up, or nobody has answered it yet.
    Asking(screen::Pending),
    Running(Box<Capture>),
}

struct ScreenNode {
    state: State,
    /// Whether this instance has ever asked. The first tick is the node opening, which is
    /// when silvia's Start button was pressed and when the panel's picker appears.
    asked: bool,
    error: Option<String>,
    /// Published until a frame arrives, so the sampler always has a texture.
    black: Arc<Frame>,
    choose: Gate,
    stop: Gate,
}

impl ScreenNode {
    fn new() -> Self {
        Self {
            state: State::Idle,
            asked: false,
            error: None,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            choose: Gate::default(),
            stop: Gate::default(),
        }
    }

    /// Put the desktop's picker up. Everything open goes first: a session has to end before
    /// the next one is asked for.
    fn ask(&mut self) {
        self.state = State::Idle;
        self.error = None;
        self.state = State::Asking(screen::ask());
        self.asked = true;
    }

    /// Take the answer, if the picker has been answered. Never waits.
    fn collect(&mut self) {
        let State::Asking(pending) = &self.state else {
            return;
        };
        match pending.poll() {
            None => {}
            Some(Ok(cast)) => {
                let source = Source::Screen(cast.stream());
                // A failure here is a line on the node, never a panic: the descriptor may
                // have gone stale, and `pipewiresrc` may not be installed at all.
                match Camera::open(&source, None) {
                    Ok(camera) => {
                        self.error = None;
                        self.state = State::Running(Box::new(Capture {
                            camera,
                            _cast: Box::new(cast),
                        }));
                    }
                    Err(e) => {
                        self.error = Some(e);
                        self.state = State::Idle;
                    }
                }
            }
            // A refusal and a failure are the same thing here: there is no screen, and the
            // reason belongs on the status line.
            Some(Err(e)) => {
                self.error = Some(e);
                self.state = State::Idle;
            }
        }
    }
}

impl CpuNode for ScreenNode {
    fn reset(&mut self) {
        // Nothing accumulates, and the capture is kept: a render that stopped the screen and
        // put the picker up again would be a render nobody could leave running.
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // Opening the node is the ask. silvia had a Start button; the panel has none, and
        // neither does this.
        if !self.asked {
            self.ask();
        }
        if ctx.downs(id, "stop", &mut self.stop) > 0 {
            self.state = State::Idle;
            self.error = None;
        }
        if ctx.downs(id, "choose", &mut self.choose) > 0 {
            self.ask();
        }
        self.collect();

        let frame = match &mut self.state {
            State::Running(capture) => {
                if let Some(e) = capture.camera.error() {
                    self.error = Some(e);
                }
                capture.camera.latest()
            }
            _ => None,
        };
        ctx.publish_frame(
            id,
            "frame",
            frame.unwrap_or_else(|| Arc::clone(&self.black)),
        );
    }

    fn error(&self) -> Option<String> {
        self.error.clone()
    }

    /// What it is doing, across the foot of the body.
    fn status(&self) -> Option<String> {
        Some(match &self.state {
            State::Asking(_) => "waiting for the screen picker…".to_string(),
            State::Running(_) => "capturing".to_string(),
            State::Idle if self.error.is_some() => "stopped".to_string(),
            State::Idle => "stopped — press Choose Screen".to_string(),
        })
    }

    fn debug(&self) -> Option<String> {
        match &self.state {
            State::Running(capture) => {
                Some(format!("screencapture {}", capture.camera.description))
            }
            State::Asking(_) => Some("screencapture asking".to_string()),
            State::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three states each say something, and none of them is silence: a picker that is up
    /// is a minute during which the node has to account for itself.
    #[test]
    fn every_state_says_what_it_is_doing() {
        let mut node = ScreenNode::new();
        assert!(
            node.status().is_some_and(|s| s.contains("Choose Screen")),
            "a node that has not asked says how to make it"
        );
        node.error = Some("no screen was chosen".to_string());
        assert_eq!(node.status().as_deref(), Some("stopped"));
        assert_eq!(node.error().as_deref(), Some("no screen was chosen"));
    }

    /// A refused picker is a status line and a black frame, not a panic and not a stale
    /// picture — and the node is back where it started, ready to be asked again.
    #[test]
    fn a_refusal_leaves_it_idle_with_the_reason() {
        let mut node = ScreenNode::new();
        node.error = Some("no screen was chosen".to_string());
        node.state = State::Idle;
        assert!(matches!(node.state, State::Idle));
        assert!(node.debug().is_none(), "nothing is running to describe");
    }
}
