// SPDX-License-Identifier: AGPL-3.0-or-later

//! Words, as a picture.
//!
//! The one node in the library that puts a letter on the screen. The letters are rasterized by
//! GStreamer's pango plugin — [`crate::video::text`] — rather than by a font crate, because
//! `nodes/` may take no graphical dependency and a rasterizer is one.
//!
//! **The words are a value, not an option.** They are the multi-line box `note` already draws,
//! four lines tall, declared as a [`ValueKind::Text`] and drawn by the node's own code: an
//! option's row knows four shapes and a paragraph is none of them. See
//! docs/decisions.md under *A node's own values are not options*.
//!
//! **The colors are the patch's.** The pipeline draws white on black and only the coverage is
//! used: `ink` is that picture as a port, and `output` is WGSL mixing Text Color into
//! Background Color by it — silvia's own `mix(bg, textColor, mask)`, and the shape
//! `cellularautomata` takes, where the state is the port and the picture is the shader reading
//! it. So a color on a cable costs no rebuild, and neither does a keystroke: a new string is a
//! new pipeline, never a new shader.
//!
//! The fonts are the machine's. The Font menu is every family fontconfig lists, and a family
//! name goes to pango, which finds it through the same fontconfig. silvia's twenty faces stay
//! declared as the choices a machine that cannot list its fonts falls back to, and a project
//! naming a face this machine does not have falls back to one it does — which is what a
//! browser did for silvia.

use crate::graph::NodeId;
use crate::graph::PortType::VaryingColor;
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Frame, InputDef, NodeDef, NumberField, OptionDef,
    OptionKind, OutputDef, OutputKind, TickContext, ValueDef, ValueKind,
};
use crate::video::text::{Spec, Words};
use std::sync::Arc;

/// How tall the box is, in lines: silvia's four, whose default is four short lines stacked.
const LINES: u8 = 4;

/// silvia's own default, kept.
const WORDS: &str = "This\nMachine\nKills\nFascists";

pub static DEF: NodeDef = NodeDef {
    slug: "text",
    category: Category::Source,
    icon: "✏",
    label: "Text",
    tooltip: "Words as a picture. Type into the box; Text Color and Background Color are \
              mixed over the letters, so the patch colors them. The fonts are the machine's, \
              and a face it does not have falls back to one it does.",
    // Prose in a column the width of a number control is a column of single words, which is
    // what widened `note`.
    width: Some(260.0),
    inputs: &[
        InputDef {
            key: "textColor",
            label: "Text Color",
            ty: VaryingColor,
            control: Control::color("#ffffffff"),
        },
        InputDef {
            key: "backgroundColor",
            label: "Background Color",
            ty: VaryingColor,
            control: Control::color("#000000ff"),
        },
    ],
    outputs: &[
        OutputDef {
            key: "ink",
            label: "Ink",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            // The rasterized string: white letters on black, mapped from worldspace into the
            // texture's own [0,1] by its real aspect, with v flipped because rows are
            // uploaded top first — the same mapping the camera and the clip use, and
            // mirrored outward past its own edge by the texture's own wrap mode.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "ink");
                let sampler = ctx.sampler(node, "ink");
                format!(
                    "{}\n    return textureSampleLevel({tex}, {sampler}, t, 0.0);",
                    sample_wgsl(&tex)
                )
            },
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "output",
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Shader,
            // silvia's: the coverage is a mask between the two colors, and nothing of the
            // rendered picture's own color survives.
            // The sibling port's own sampler, since `ink` is the texture being read.
            wgsl: |node, ctx, _func| {
                let tex = ctx.texture_uniform(node, "ink");
                let sampler = ctx.sampler(node, "ink");
                let ink = ctx.input(node, "textColor", "uv");
                let ground = ctx.input(node, "backgroundColor", "uv");
                format!(
                    "{}\n    let mask = textureSampleLevel({tex}, {sampler}, t, 0.0).r;
    return mix({ground}, {ink}, mask);",
                    sample_wgsl(&tex)
                )
            },
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        OptionDef {
            key: "font",
            label: "Font",
            // silvia's own default face.
            default: "Palatino Linotype",
            // What the menu offers: the machine's own families.
            found: Some(crate::video::text::families),
            // silvia's twenty, as family names, for a machine that cannot list its own: pango
            // takes a family and fontconfig finds the nearest thing the machine has, which is
            // the fallback a browser did for silvia's own comma-separated stacks.
            choices: &[
                ("Arial", "Arial"),
                ("Verdana", "Verdana"),
                ("Tahoma", "Tahoma"),
                ("Trebuchet MS", "Trebuchet MS"),
                ("Times New Roman", "Times New Roman"),
                ("Georgia", "Georgia"),
                ("Garamond", "Garamond"),
                ("Courier New", "Courier New"),
                ("Lucida Console", "Lucida Console"),
                ("Impact", "Impact"),
                ("Comic Sans MS", "Comic Sans MS"),
                ("Consolas", "Consolas"),
                ("Monaco", "Monaco"),
                ("Brush Script MT", "Brush Script MT"),
                ("Palatino Linotype", "Palatino Linotype"),
                ("Segoe UI", "Segoe UI"),
                ("Sans", "Generic Sans-Serif"),
                ("Serif", "Generic Serif"),
                ("Monospace", "Generic Monospace"),
                ("Cursive", "Generic Cursive"),
            ],
            // Read by `tick`, which builds the pipeline on it. No shader depends on it.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "size",
            label: "Size",
            default: "64",
            // A plain number field: silvia's own control is a number from 8 to 512, and a
            // select of eight sizes would be a worse answer than a field that takes any of
            // them. The one choice is the default's home; the field takes any size.
            choices: &[("64", "64")],
            kind: OptionKind::Runtime,
            number: Some(SIZE),
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "weight",
            label: "Weight",
            default: "bold",
            choices: &[
                ("normal", "Normal"),
                ("bold", "Bold"),
                ("lighter", "Lighter"),
            ],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "align",
            label: "Align",
            default: "center",
            choices: &[("left", "Left"), ("center", "Center"), ("right", "Right")],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "baseline",
            label: "Baseline",
            default: "middle",
            choices: &[("top", "Top"), ("middle", "Middle"), ("bottom", "Bottom")],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "texsize",
            label: "Texture Size",
            default: "1280x720",
            // silvia's seven shapes, 16:9 through 9:16.
            choices: &[
                ("1280x720", "16:9 (1280x720)"),
                ("1920x1080", "16:9 (1920x1080)"),
                ("3440x1440", "21:9 (3440x1440)"),
                ("1024x768", "4:3 (1024x768)"),
                ("1080x1080", "1:1 (1080x1080)"),
                ("720x1280", "9:16 (720x1280)"),
                ("1080x1920", "9:16 (1080x1920)"),
            ],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        crate::nodes::SHOW_PREVIEW,
    ],
    values: &[ValueDef {
        key: "words",
        // No label: the box is the whole row, and a caption reading "Text" over a box of text
        // is a caption that says nothing.
        label: "",
        kind: ValueKind::Text {
            rows: LINES,
            placeholder: "the words",
            default: WORDS,
        },
    }],
    // The letters on the node, under the heading every other source's picture is under: a
    // font or an alignment changed with nothing wired up otherwise changes nothing anybody
    // can see.
    regions: &[crate::nodes::Region::Preview("ink")],
    cpu: Some(CpuDef {
        create: || Box::new(TextNode::new()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

/// The worldspace-to-texture mapping both bodies start from, declaring `t`.
fn sample_wgsl(tex: &str) -> String {
    format!(
        "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);"
    )
}

/// The sizes silvia's own control would take, whole, from 8 to 512 pixels.
const SIZE: NumberField = NumberField {
    min: 8.0,
    max: 512.0,
    integer: true,
    unit: "px",
};

/// Size as the pipeline takes it, with anything unparseable landing on the default.
fn size_of(text: &str) -> u32 {
    SIZE.read(text).map_or(64, |v| v as u32)
}

/// silvia's weight names, as pango's.
fn weight_of(option: &str) -> &'static str {
    match option {
        "normal" => "Normal",
        "lighter" => "Light",
        _ => "Bold",
    }
}

/// silvia's vertical alignment, as `textoverlay`'s.
fn baseline_of(option: &str) -> &'static str {
    match option {
        "top" => "top",
        "bottom" => "bottom",
        _ => "center",
    }
}

/// And its horizontal one, which the two spell the same way.
fn align_of(option: &str) -> &'static str {
    match option {
        "left" => "left",
        "right" => "right",
        _ => "center",
    }
}

struct TextNode {
    /// The rendering in flight, or the one that finished. Dropped when the string changes.
    words: Option<Words>,
    /// What that rendering was for, so a tick can tell whether it still matches.
    wanted: Option<Spec>,
    /// The frame it produced, republished untouched until something changes: the renderer
    /// has seen this `Arc` and skips it, so a still string costs one upload in its life.
    frame: Option<Arc<Frame>>,
    /// Published until the first one arrives, so the sampler always has a texture.
    black: Arc<Frame>,
    error: Option<String>,
}

impl TextNode {
    fn new() -> Self {
        Self {
            words: None,
            wanted: None,
            frame: None,
            black: Arc::new(Frame::solid(2, 2, [0, 0, 0, 255])),
            error: None,
        }
    }

    /// What the node is asking for this frame.
    fn spec(id: NodeId, ctx: &TickContext<'_>) -> Spec {
        let (width, height) = crate::nodes::output::parse_resolution(ctx.option(id, "texsize"))
            .unwrap_or((1280, 720));
        Spec {
            text: ctx.text(id, "words").to_string(),
            family: ctx.option(id, "font").to_string(),
            weight: weight_of(ctx.option(id, "weight")),
            size: size_of(ctx.option(id, "size")),
            align: align_of(ctx.option(id, "align")),
            baseline: baseline_of(ctx.option(id, "baseline")),
            width,
            height,
        }
    }
}

impl CpuNode for TextNode {
    fn reset_in_place(&self) -> bool {
        true
    }

    fn reset(&mut self) {
        // Nothing accumulates. The picture is a function of the string and the options, and a
        // render starting over would draw the same letters.
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        let wanted = Self::spec(id, ctx);
        if self.wanted.as_ref() != Some(&wanted) {
            // The old picture goes with the old string: keeping it would leave the previous
            // word on screen for as long as the new one took to draw.
            self.frame = None;
            self.words = match Words::render(&wanted) {
                Ok(words) => {
                    self.error = None;
                    Some(words)
                }
                Err(e) => {
                    self.error = Some(e);
                    None
                }
            };
            self.wanted = Some(wanted);
        }

        if let Some(words) = self.words.as_mut() {
            if let Some(e) = words.error() {
                self.error = Some(e);
            }
            if let Some(frame) = words.latest() {
                self.frame = Some(frame);
                // The pipeline has said everything it was going to.
                self.words = None;
            }
        }

        let frame = self
            .frame
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.black));
        ctx.publish_frame(id, "ink", frame);
    }

    fn error(&self) -> Option<String> {
        self.error.clone()
    }

    fn debug(&self) -> Option<String> {
        let spec = self.wanted.as_ref()?;
        Some(format!(
            "text {}x{} {} {}px, {} chars",
            spec.width,
            spec.height,
            spec.family,
            spec.size,
            spec.text.chars().count(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anything silvia's own control could hold is a size, and nothing else is.
    #[test]
    fn a_size_is_a_number_in_silvias_range() {
        for good in ["8", "64", " 512 "] {
            assert!(SIZE.holds(good), "{good}");
        }
        for bad in ["", "7", "513", "64px", "big"] {
            assert!(!SIZE.holds(bad), "{bad}");
        }
        assert_eq!(size_of("nonsense"), 64, "and a fallback is the default");
        assert_eq!(size_of("96"), 96);
        assert_eq!(size_of("1000"), 512, "and a size past the end is the end");
    }

    /// silvia's names for the weights and the baselines, as the renderer's.
    #[test]
    fn the_options_map_onto_pango() {
        assert_eq!(weight_of("lighter"), "Light");
        assert_eq!(weight_of("normal"), "Normal");
        assert_eq!(weight_of("bold"), "Bold");
        assert_eq!(baseline_of("middle"), "center");
        assert_eq!(baseline_of("top"), "top");
        assert_eq!(align_of("center"), "center");
    }

    /// The node starts holding silvia's four lines, so it draws something before it is typed
    /// into.
    #[test]
    fn the_words_start_at_silvias_own() {
        let value = DEF.value("words").expect("the text value");
        assert_eq!(value.default(), WORDS);
        assert_eq!(value.rows(), LINES);
    }
}
