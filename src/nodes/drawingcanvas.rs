// SPDX-License-Identifier: AGPL-3.0-or-later

//! A square of black on the node that a hand paints on, published as a picture.
//!
//! silvia's `drawingcanvas.js`: a canvas in the custom area, six tools — pen, eraser, line,
//! rectangle, circle and fill — with a key each while the canvas has focus, a brush size and a
//! color, a background that Clear fills with, eight symmetry modes that turn one stroke into a
//! mandala, and three ways out: the picture, a mask, and an event the moment a stroke lifts.
//! The arithmetic is silvia's, pixel for pixel where a 2D canvas pins it down — round caps,
//! source-over, the eraser's `destination-out`, `strokeRect`'s square corners, the fill's
//! scanline and its tolerance of twenty, and the symmetry transforms in silvia's order.
//!
//! **The painting is one of the node's own values, and it is saved.** silvia kept the brush
//! and lost the picture on a reload; here the picture is a [`Value::Painting`] in
//! `Node::values` — document data, so a stroke is an edit through the command bus, one undo
//! step, copied with the node, and written by the project's save into `assets/` as a PNG named
//! by its content (docs/decisions.md, *A painting is saved with the project*). The brush —
//! its size and its two colors — is three hidden controls the region draws, so a MIDI knob can
//! turn the size; the tool is an option the region's buttons set, and the symmetry an option
//! row.
//!
//! **Who does what.** The paint surface is a region that claims the pointer
//! (`widgets::paint`): it reads the painting off the `Node`, draws a stroke into a copy with
//! the functions here, and hands the copy back as a `SetValue` — once a frame while a pen
//! moves, all of it one gesture. The tick only publishes: the painting, or a blank of the
//! background where nothing has been painted. What the tick does to the document is Clear,
//! which a sequencer can fire, written back through [`TickContext::write_value`] like any value
//! a tick makes.
//!
//! **The painting is straight and what the node publishes is premultiplied.** The painting is
//! a 2D canvas's own pixels, source-over and `destination-out` on straight colors as silvia's
//! are, and a PNG saves them as they lie; every picture in the graph is premultiplied
//! ([decisions.md](../../../docs/decisions.md#colors-in-the-graph-are-premultiplied)). So the
//! tick publishes a premultiplied copy, made once for each painting and size and published
//! again until either moves, and an erased pixel's color stays in the painting under its
//! alpha of zero without reaching the picture.
//!
//! This module is pure: pixels in a `Vec`, and the tick. No egui.

use crate::graph::PortType::{Action, VaryingColor, VaryingNumber};
use crate::graph::{ControlValue, Node, NodeId, Painting, Value};
use crate::nodes::{
    Category, Control, CpuDef, CpuNode, Edge, Frame, Gate, InputDef, NodeDef, OptionDef,
    OptionKind, OutputDef, OutputKind, Pixels, Region, TickContext, ValueDef, ValueKind, alpha,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The value the picture is kept under.
pub const PAINTING: &str = "painting";
/// The texture the picture leaves by, which the paint surface also shows.
pub const OUTPUT: &str = "output";
/// Which tool the hand is holding, set by the region's buttons.
pub const TOOL: &str = "tool";
/// How one stroke is repeated.
pub const SYMMETRY: &str = "symmetry";
/// How many pixels the picture is.
pub const CANVAS_SIZE: &str = "canvas_res";
/// The brush: its width in pixels of the picture, and its color.
pub const BRUSH_SIZE: &str = "brushSize";
pub const BRUSH_COLOR: &str = "brushColor";
/// What Clear fills the picture with, and what an unpainted canvas is.
pub const BACKGROUND: &str = "backgroundColor";
/// The action input that wipes the picture.
pub const CLEAR: &str = "clear";
/// The action output that fires when a stroke lifts.
pub const STROKE_DONE: &str = "strokeDone";

/// The picture as a `uv` sample: worldspace into the picture's own `[0,1]` by its real aspect,
/// v flipped because rows are uploaded top first — the mapping `imagegif` and the camera use —
/// then silvia's Wrap outside it. Mirror is the sampler's own; Repeat and Clamp fold the
/// coordinate first.
fn sample(node: NodeId, ctx: &mut crate::compile::CompileContext) -> String {
    let tex = ctx.texture_uniform(node, OUTPUT);
    let sampler = ctx.sampler(node, OUTPUT);
    let wrap = match ctx.option(node, "wrap") {
        "repeat" => "\n    t = fract(t);",
        "clamp" => "\n    t = clamp(t, vec2f(0.0), vec2f(1.0));",
        _ => "",
    };
    format!(
        "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / max(texSize.y, 1.0);
    var t = vec2f((uv.x / aspect + 1.0) * 0.5, 1.0 - (uv.y + 1.0) * 0.5);{wrap}
    let c = textureSampleLevel({tex}, {sampler}, t, 0.0);"
    )
}

pub static DEF: NodeDef = NodeDef {
    slug: "drawingcanvas",
    category: Category::Source,
    // silvia's is 👨🏻‍🎨, a sequence of four codepoints joined by a zero-width joiner, which the
    // editor's text draws as its parts; the brush is the one glyph that says the same thing.
    icon: "🖌",
    label: "Drawing Canvas",
    tooltip: "A canvas to paint on with the mouse: pen, eraser, line, rectangle, circle and \
              fill, with a key each while the canvas has focus — B E L R C F, and [ ] for the \
              size. Symmetry repeats a stroke as a mirror or a mandala. The painting is saved \
              with the project; Clear fills it with the background, and Stroke Done fires \
              when a stroke lifts.",
    inputs: &[InputDef {
        key: CLEAR,
        label: "Clear",
        ty: Action,
        control: Control::Press,
    }],
    // silvia's three `values`, drawn under the canvas by the node's own region rather than as
    // rows: a brush is set while painting, beside the thing it paints.
    hidden: &[
        InputDef {
            key: BRUSH_SIZE,
            label: "Size",
            ty: crate::graph::PortType::UniformNumber,
            control: Control::num(5.0, 1.0, 200.0, 1.0, "px"),
        },
        InputDef {
            key: BRUSH_COLOR,
            label: "Color",
            ty: VaryingColor,
            control: Control::color("#ffffffff"),
        },
        InputDef {
            key: BACKGROUND,
            label: "Background",
            ty: VaryingColor,
            control: Control::color("#000000ff"),
        },
    ],
    outputs: &[
        OutputDef {
            key: OUTPUT,
            label: "Output",
            ty: VaryingColor,
            kind: OutputKind::Texture,
            wgsl: |node, ctx, _func| format!("{}\n    return c;", sample(node, ctx)),
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: "mask",
            label: "Mask",
            ty: VaryingNumber,
            kind: OutputKind::Shader,
            // silvia's: the picture's luminance, by Rec.601's weights, where it is painted —
            // an erased pixel is transparent, and transparent is no mask.
            wgsl: |node, ctx, _func| {
                format!(
                    "{}\n    return dot(c.rgb, vec3f(0.299, 0.587, 0.114)) * c.a;",
                    sample(node, ctx)
                )
            },
            range: Some("[0, 1]"),
            ..OutputDef::EMPTY
        },
        OutputDef {
            key: STROKE_DONE,
            label: "Stroke Done",
            ty: Action,
            kind: OutputKind::Action,
            ..OutputDef::EMPTY
        },
    ],
    options: &[
        OptionDef {
            key: CANVAS_SIZE,
            label: "Canvas Size",
            default: "512x512",
            choices: &[
                ("256x256", "256x256"),
                ("512x512", "512x512"),
                ("1024x1024", "1024x1024"),
                ("512x256", "512x256"),
                ("1024x512", "1024x512"),
            ],
            // Read by the tick and the paint surface. The picture reaches the fragment as a
            // bound texture whose size the shader asks for, so no WGSL changes.
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: "wrap",
            label: "Wrap",
            // The house rule for a texture, where silvia's default was Clamp: see
            // docs/rendering.md, *Texture wrapping*.
            default: "mirror",
            choices: &[
                ("mirror", "Mirror"),
                ("repeat", "Repeat"),
                ("clamp", "Clamp"),
            ],
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: SYMMETRY,
            label: "Symmetry",
            default: "none",
            choices: &[
                ("none", "None"),
                ("h", "H Mirror"),
                ("v", "V Mirror"),
                ("hv", "HV Mirror"),
                ("r3", "3-Fold"),
                ("r4", "4-Fold"),
                ("r6", "6-Fold"),
                ("r8", "8-Fold"),
            ],
            // Read by the paint surface alone: it decides where a stroke lands, and the
            // picture that results is what everything downstream reads.
            kind: OptionKind::Presentation,
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: TOOL,
            label: "Tool",
            default: "pen",
            choices: &[
                ("pen", "Pen"),
                ("eraser", "Eraser"),
                ("line", "Line"),
                ("rect", "Rect"),
                ("circle", "Circle"),
                ("fill", "Fill"),
            ],
            kind: OptionKind::Presentation,
            // silvia's row of six buttons under the canvas, which is the brush region's.
            in_region: true,
            ..OptionDef::EMPTY
        },
    ],
    values: &[ValueDef {
        key: PAINTING,
        label: "",
        kind: ValueKind::Painting,
    }],
    regions: &[Region::Paint(OUTPUT), Region::Brush],
    cpu: Some(CpuDef {
        create: || Box::new(Canvas::new()),
        integrates: false,
        live: false,
    }),
    ..NodeDef::EMPTY
};

// --------------------------------------------------------------------------------- the tools

/// What the hand is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pen,
    Eraser,
    Line,
    Rect,
    Circle,
    Fill,
}

impl Tool {
    /// silvia's order, which is the buttons' order.
    pub const ALL: [Self; 6] = [
        Self::Pen,
        Self::Eraser,
        Self::Line,
        Self::Rect,
        Self::Circle,
        Self::Fill,
    ];

    /// The option value that names it.
    pub fn key(self) -> &'static str {
        match self {
            Self::Pen => "pen",
            Self::Eraser => "eraser",
            Self::Line => "line",
            Self::Rect => "rect",
            Self::Circle => "circle",
            Self::Fill => "fill",
        }
    }

    /// What its button says under the pointer.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pen => "Pen",
            Self::Eraser => "Eraser",
            Self::Line => "Line",
            Self::Rect => "Rect",
            Self::Circle => "Circle",
            Self::Fill => "Fill",
        }
    }

    /// silvia's key for it while the canvas has focus: B for the brush, then the initials.
    pub fn shortcut(self) -> char {
        match self {
            Self::Pen => 'B',
            Self::Eraser => 'E',
            Self::Line => 'L',
            Self::Rect => 'R',
            Self::Circle => 'C',
            Self::Fill => 'F',
        }
    }

    /// The tool an option value names; the pen for anything else.
    pub fn of(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|t| t.key() == value)
            .unwrap_or(Self::Pen)
    }

    /// A shape dragged from a corner to a corner, drawn once when the pointer lets go.
    pub fn is_shape(self) -> bool {
        matches!(self, Self::Line | Self::Rect | Self::Circle)
    }
}

/// How one stroke is repeated across the picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symmetry {
    None,
    /// Mirrored left to right, top to bottom, or both.
    H,
    V,
    Hv,
    /// Turned about the center this many times.
    Fold(u32),
}

impl Symmetry {
    /// The mode an option value names; none for anything else.
    pub fn of(value: &str) -> Self {
        match value {
            "h" => Self::H,
            "v" => Self::V,
            "hv" => Self::Hv,
            _ => value
                .strip_prefix('r')
                .and_then(|n| n.parse().ok())
                .filter(|n| *n >= 2)
                .map_or(Self::None, Self::Fold),
        }
    }

    /// Every place `p` lands on a `w` by `h` picture, in silvia's order: the point itself, then
    /// its mirrors — or, for a fold, its turns, the first of which is the point.
    pub fn images(self, w: f32, h: f32, (x, y): (f32, f32)) -> Vec<(f32, f32)> {
        match self {
            Self::None => vec![(x, y)],
            Self::H => vec![(x, y), (w - x, y)],
            Self::V => vec![(x, y), (x, h - y)],
            Self::Hv => vec![(x, y), (w - x, y), (x, h - y), (w - x, h - y)],
            Self::Fold(n) => {
                let (cx, cy) = (w / 2.0, h / 2.0);
                (0..n)
                    .map(|i| {
                        let angle = std::f32::consts::TAU * i as f32 / n as f32;
                        let (sin, cos) = angle.sin_cos();
                        let (dx, dy) = (x - cx, y - cy);
                        (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
                    })
                    .collect()
            }
        }
    }
}

/// The brush as the node holds it: its width in pixels of the picture, its color as bytes,
/// and whether it erases.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
    pub size: f32,
    pub color: [u8; 4],
    pub erase: bool,
}

/// One of the node's colors as bytes, the way the swatch and the picker write them.
fn bytes(color: [f32; 4]) -> [u8; 4] {
    color.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn control_color(node: &Node, key: &str) -> [u8; 4] {
    match node.controls.get(key) {
        Some(ControlValue::Color(c)) => bytes(*c),
        _ => match node.def.input(key).map(|i| &i.control) {
            Some(Control::Color { default }) => bytes(crate::nodes::parse_hex_rgba(default)),
            _ => [0, 0, 0, 255],
        },
    }
}

/// The picture's size an option value names, silvia's `WxH`; 512 square for anything else.
pub fn size_of(value: &str) -> (u32, u32) {
    value
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        .filter(|&(w, h): &(u32, u32)| (1..=4096).contains(&w) && (1..=4096).contains(&h))
        .unwrap_or((512, 512))
}

/// The picture's size on this node.
pub fn canvas_size(node: &Node) -> (u32, u32) {
    size_of(node.options.get(CANVAS_SIZE).map_or("", String::as_str))
}

/// The tool in the hand on this node.
pub fn tool(node: &Node) -> Tool {
    Tool::of(node.options.get(TOOL).map_or("", String::as_str))
}

/// The symmetry this node paints with.
pub fn symmetry(node: &Node) -> Symmetry {
    Symmetry::of(node.options.get(SYMMETRY).map_or("", String::as_str))
}

/// The brush this node paints with.
pub fn brush(node: &Node) -> Brush {
    let size = match node.controls.get(BRUSH_SIZE) {
        Some(ControlValue::Float(v)) if v.is_finite() => v.max(0.5),
        _ => 5.0,
    };
    Brush {
        size,
        color: control_color(node, BRUSH_COLOR),
        erase: tool(node) == Tool::Eraser,
    }
}

/// What an unpainted canvas is, and what Clear fills with.
pub fn background(node: &Node) -> [u8; 4] {
    control_color(node, BACKGROUND)
}

/// The picture this node holds, where it holds one it has read.
pub fn painting(node: &Node) -> Option<&Painting> {
    node.values
        .get(PAINTING)
        .and_then(Value::painting)
        .filter(|p| p.is_read())
}

/// A stroke number no earlier stroke in this process was given. What a finished stroke
/// stamps its picture with, so the tick can tell a stroke that just lifted from an undo back
/// to an older picture.
pub fn next_stroke() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

// --------------------------------------------------------------------------------- the pixels

/// A picture being painted: RGBA8, rows top first, unpremultiplied, as a 2D canvas's
/// `getImageData` hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Sheet {
    /// A picture of one color.
    pub fn blank(width: u32, height: u32, color: [u8; 4]) -> Self {
        Self {
            width,
            height,
            rgba: color.repeat(width as usize * height as usize),
        }
    }

    /// What a hand paints onto on this node: its picture at the size Canvas Size names now —
    /// stretched to it where the menu has moved since it was painted — or a blank of the
    /// background where it has none.
    pub fn of(node: &Node) -> Self {
        let (w, h) = canvas_size(node);
        match painting(node).and_then(Painting::pixels) {
            Some((pw, ph, px)) if (pw, ph) == (w, h) => Self {
                width: w,
                height: h,
                rgba: px.to_vec(),
            },
            Some((pw, ph, px)) => resampled(pw, ph, px, w, h),
            None => Self::blank(w, h, background(node)),
        }
    }

    /// Hand the pixels over as the node's value, finished by `stroke`.
    pub fn into_painting(self, stroke: u64) -> Painting {
        Painting::new(self.width, self.height, self.rgba, stroke)
    }

    /// The pixel rows and columns a shape reaching `pad` past these points can touch.
    fn span(&self, points: &[(f32, f32)], pad: f32) -> Option<(usize, usize, usize, usize)> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in points {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        let clamp = |v: f32, n: u32| v.floor().clamp(0.0, n as f32) as usize;
        let (x0, x1) = (
            clamp(x0 - pad, self.width),
            clamp(x1 + pad + 1.0, self.width),
        );
        let (y0, y1) = (
            clamp(y0 - pad, self.height),
            clamp(y1 + pad + 1.0, self.height),
        );
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    /// Cover every pixel in the rows and columns `points` and `pad` reach by what `coverage`
    /// says of its center, with the brush.
    fn cover(
        &mut self,
        brush: &Brush,
        points: &[(f32, f32)],
        pad: f32,
        coverage: impl Fn(f32, f32) -> f32,
    ) {
        let Some((x0, y0, x1, y1)) = self.span(points, pad) else {
            return;
        };
        let w = self.width as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let c = coverage(x as f32 + 0.5, y as f32 + 0.5);
                if c <= 0.0 {
                    continue;
                }
                let at = (y * w + x) * 4;
                let px = &mut self.rgba[at..at + 4];
                if brush.erase {
                    erase(px, c);
                } else {
                    over(px, brush.color, c);
                }
            }
        }
    }

    /// One piece of a pen's stroke, or a line: `from` to `to` at the brush's width with round
    /// ends, at every place symmetry puts it. A press that has not moved is a dot.
    pub fn segment(&mut self, brush: &Brush, symmetry: Symmetry, from: (f32, f32), to: (f32, f32)) {
        let (w, h) = (self.width as f32, self.height as f32);
        let r = brush.size * 0.5;
        for (a, b) in symmetry
            .images(w, h, from)
            .into_iter()
            .zip(symmetry.images(w, h, to))
        {
            self.cover(brush, &[a, b], r + 1.0, |x, y| {
                (r + 0.5 - distance_to_segment((x, y), a, b)).clamp(0.0, 1.0)
            });
        }
    }

    /// A shape dragged from `from` to `to` — a line, `strokeRect`'s outline with its square
    /// corners, or the ellipse inside that box — at every place symmetry puts its corners.
    ///
    /// silvia's: each corner is carried by the symmetry and the shape is built again from the
    /// two it lands on, so a rectangle turned by a fold is the upright box around its turned
    /// corners rather than a turned rectangle.
    pub fn shape(
        &mut self,
        tool: Tool,
        brush: &Brush,
        symmetry: Symmetry,
        from: (f32, f32),
        to: (f32, f32),
    ) {
        if tool == Tool::Line {
            self.segment(brush, symmetry, from, to);
            return;
        }
        let (w, h) = (self.width as f32, self.height as f32);
        let half = brush.size * 0.5;
        for (a, b) in symmetry
            .images(w, h, from)
            .into_iter()
            .zip(symmetry.images(w, h, to))
        {
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
            match tool {
                Tool::Rect => {
                    let outer = (x0 - half, y0 - half, x1 + half, y1 + half);
                    let inner = (x0 + half, y0 + half, x1 - half, y1 - half);
                    self.cover(brush, &[a, b], half + 1.0, |x, y| {
                        inside(outer, x, y) * (1.0 - inside(inner, x, y))
                    });
                }
                Tool::Circle => {
                    let (cx, cy) = (f32::midpoint(x0, x1), f32::midpoint(y0, y1));
                    let (rx, ry) = ((x1 - x0) * 0.5, (y1 - y0) * 0.5);
                    if rx < 0.5 || ry < 0.5 {
                        // An ellipse with no width is the line down its middle.
                        let (p, q) = if rx < ry {
                            ((cx, y0), (cx, y1))
                        } else {
                            ((x0, cy), (x1, cy))
                        };
                        self.cover(brush, &[p, q], half + 1.0, |x, y| {
                            (half + 0.5 - distance_to_segment((x, y), p, q)).clamp(0.0, 1.0)
                        });
                        continue;
                    }
                    self.cover(brush, &[a, b], half + 1.0, |x, y| {
                        (half + 0.5 - distance_to_ellipse(x - cx, y - cy, rx, ry).abs())
                            .clamp(0.0, 1.0)
                    });
                }
                Tool::Pen | Tool::Eraser | Tool::Line | Tool::Fill => {}
            }
        }
    }

    /// silvia's flood fill: every pixel reachable from `at` through pixels within twenty of the
    /// one clicked, in every channel, takes the brush's color outright. Symmetry does not
    /// repeat it, which is silvia's too — a fill already reaches everything it can.
    pub fn fill(&mut self, color: [u8; 4], at: (f32, f32)) {
        const TOLERANCE: u8 = 20;
        let (w, h) = (self.width as usize, self.height as usize);
        let (sx, sy) = (at.0.round(), at.1.round());
        if sx < 0.0 || sy < 0.0 || sx >= w as f32 || sy >= h as f32 {
            return;
        }
        let (sx, sy) = (sx as usize, sy as usize);
        let start = (sy * w + sx) * 4;
        let target: [u8; 4] = self.rgba[start..start + 4]
            .try_into()
            .expect("four bytes a pixel");
        if target == color {
            return;
        }
        let data = &mut self.rgba;
        let matches = |data: &[u8], i: usize| {
            (0..4).all(|c| data[i * 4 + c].abs_diff(target[c]) <= TOLERANCE)
        };
        let mut visited = vec![false; w * h];
        let mut stack = vec![(sx, sy)];
        while let Some((x, y)) = stack.pop() {
            let row = y * w;
            if visited[row + x] || !matches(data, row + x) {
                continue;
            }
            let mut left = x;
            while left > 0 && !visited[row + left - 1] && matches(data, row + left - 1) {
                left -= 1;
            }
            let mut right = left;
            while right < w && !visited[row + right] && matches(data, row + right) {
                data[(row + right) * 4..(row + right) * 4 + 4].copy_from_slice(&color);
                visited[row + right] = true;
                right += 1;
            }
            for ny in [y.checked_sub(1), (y + 1 < h).then_some(y + 1)]
                .into_iter()
                .flatten()
            {
                let next = ny * w;
                for nx in left..right {
                    if !visited[next + nx] && matches(data, next + nx) {
                        stack.push((nx, ny));
                    }
                }
            }
        }
    }
}

/// A picture stretched to another size, nearest pixel: what a painting becomes when Canvas
/// Size moves under it. silvia clears the canvas instead; a menu that throws a painting away
/// is a hostile menu, and one that stretches it is undone by moving the menu back.
pub fn resampled(w0: u32, h0: u32, src: &[u8], w1: u32, h1: u32) -> Sheet {
    let mut rgba = Vec::with_capacity(w1 as usize * h1 as usize * 4);
    for y in 0..h1 {
        let sy = ((y as u64 * u64::from(h0)) / u64::from(h1)) as usize;
        for x in 0..w1 {
            let sx = ((x as u64 * u64::from(w0)) / u64::from(w1)) as usize;
            let at = (sy * w0 as usize + sx) * 4;
            rgba.extend_from_slice(&src[at..at + 4]);
        }
    }
    Sheet {
        width: w1,
        height: h1,
        rgba,
    }
}

/// How far `p` is from the segment `a`–`b`.
fn distance_to_segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (px, py) = (p.0 - a.0, p.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len > 0.0 {
        ((px * dx + py * dy) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (px - dx * t).hypot(py - dy * t)
}

/// How far the point `(x, y)` from an ellipse's center is from its outline, near enough: the
/// implicit function over the length of its gradient, which is exact on the outline and good
/// for a brush's width either side of it.
fn distance_to_ellipse(x: f32, y: f32, rx: f32, ry: f32) -> f32 {
    let k0 = (x / rx).hypot(y / ry);
    let k1 = (x / (rx * rx)).hypot(y / (ry * ry));
    if k1 <= f32::EPSILON {
        return -rx.min(ry);
    }
    k0 * (k0 - 1.0) / k1
}

/// How much of the pixel centered on `(x, y)` a box covers, a half-pixel ramp at each edge.
fn inside((x0, y0, x1, y1): (f32, f32, f32, f32), x: f32, y: f32) -> f32 {
    if x0 > x1 || y0 > y1 {
        return 0.0;
    }
    ((x - x0).min(x1 - x).min(y - y0).min(y1 - y) + 0.5).clamp(0.0, 1.0)
}

/// A 2D canvas's `source-over`, on unpremultiplied bytes, with the color's alpha scaled by how
/// much of the pixel the brush covers.
fn over(dst: &mut [u8], src: [u8; 4], coverage: f32) {
    let sa = f32::from(src[3]) / 255.0 * coverage;
    if sa <= 0.0 {
        return;
    }
    let da = f32::from(dst[3]) / 255.0;
    let oa = sa + da * (1.0 - sa);
    for i in 0..3 {
        let c = (f32::from(src[i]) * sa + f32::from(dst[i]) * da * (1.0 - sa)) / oa;
        dst[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// A 2D canvas's `destination-out` under an opaque brush: what is there loses the coverage
/// from its alpha, and keeps its color.
fn erase(dst: &mut [u8], coverage: f32) {
    let da = f32::from(dst[3]) / 255.0 * (1.0 - coverage);
    dst[3] = (da * 255.0).round().clamp(0.0, 255.0) as u8;
}

// ----------------------------------------------------------------------------------- the tick

/// What the tick last published, and what it was made from, so a tick that changed nothing
/// publishes the same `Arc` and costs no upload.
enum Made {
    /// A painting at the size Canvas Size names, stretched to it where it is another size.
    Painted { from: Arc<Frame>, size: (u32, u32) },
    /// A canvas nobody has painted on.
    Blank { size: (u32, u32), color: [u8; 4] },
}

struct Canvas {
    published: Option<(Made, Arc<Frame>)>,
    /// The newest stroke this instance has seen finish, or `None` before its first tick.
    seen: Option<u64>,
    /// Stroke Done went down last tick and comes up on this one.
    owed: bool,
    clear: Gate,
}

impl Canvas {
    fn new() -> Self {
        Self {
            published: None,
            seen: None,
            owed: false,
            clear: Gate::default(),
        }
    }

    /// The frame to publish for this painting at this size, premultiplied, made again only
    /// when what it is made from moved.
    fn frame(
        &mut self,
        painting: Option<&Painting>,
        size: (u32, u32),
        color: [u8; 4],
    ) -> Arc<Frame> {
        let fresh = match painting.and_then(Painting::frame) {
            Some(frame) => match &self.published {
                Some((Made::Painted { from, size: s }, out))
                    if Arc::ptr_eq(from, frame) && *s == size =>
                {
                    return Arc::clone(out);
                }
                _ => {
                    let Some(bytes) = frame.bytes() else {
                        let mut color = color;
                        alpha::premultiply_rgba8(&mut color);
                        return Arc::new(Frame::solid(size.0, size.1, color));
                    };
                    let sheet = if (frame.width, frame.height) == size {
                        Sheet {
                            width: frame.width,
                            height: frame.height,
                            rgba: bytes.to_vec(),
                        }
                    } else {
                        resampled(frame.width, frame.height, bytes, size.0, size.1)
                    };
                    (
                        Made::Painted {
                            from: Arc::clone(frame),
                            size,
                        },
                        sheet,
                    )
                }
            },
            None => match &self.published {
                Some((Made::Blank { size: s, color: c }, out)) if *s == size && *c == color => {
                    return Arc::clone(out);
                }
                _ => (
                    Made::Blank { size, color },
                    Sheet::blank(size.0, size.1, color),
                ),
            },
        };
        let (made, mut sheet) = fresh;
        alpha::premultiply_rgba8(&mut sheet.rgba);
        let frame = Arc::new(Frame {
            width: sheet.width,
            height: sheet.height,
            pixels: Pixels::Bytes(sheet.rgba),
        });
        self.published = Some((made, Arc::clone(&frame)));
        frame
    }
}

impl CpuNode for Canvas {
    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self, id: NodeId, ctx: &mut TickContext<'_>) {
        // A pulse is a gate open for one frame.
        if std::mem::take(&mut self.owed) {
            ctx.fire(id, STROKE_DONE, Edge::Up);
        }
        let size = size_of(ctx.option(id, CANVAS_SIZE));
        let color = bytes(ctx.color(id, BACKGROUND));
        let mut painting = ctx.value(id, PAINTING).and_then(Value::painting).cloned();

        // silvia's `_clearCanvas`: the background, at the size the canvas is. A painting that
        // is already exactly that is left alone, so a sequencer clearing a canvas nobody has
        // touched writes nothing into the history.
        if ctx.downs(id, CLEAR, &mut self.clear) > 0
            && let Some(held) = &painting
        {
            let blank = Sheet::blank(size.0, size.1, color);
            let already = held
                .pixels()
                .is_some_and(|(w, h, px)| (w, h) == size && px == blank.rgba.as_slice());
            if !already {
                let cleared = blank.into_painting(held.stroke());
                ctx.write_value(id, PAINTING, Value::Painting(cleared.clone()));
                painting = Some(cleared);
            }
        }

        let stroke = painting.as_ref().map_or(0, Painting::stroke);
        if self.seen.is_some_and(|seen| stroke > seen) {
            ctx.fire(id, STROKE_DONE, Edge::Down);
            self.owed = true;
        }
        self.seen = Some(self.seen.map_or(stroke, |seen| seen.max(stroke)));

        let frame = self.frame(painting.as_ref(), size, color);
        ctx.publish_frame(id, OUTPUT, frame);
    }

    fn debug(&self) -> Option<String> {
        let (_, frame) = self.published.as_ref()?;
        Some(format!("drawingcanvas {}x{}", frame.width, frame.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    fn pen(size: f32) -> Brush {
        Brush {
            size,
            color: WHITE,
            erase: false,
        }
    }

    fn at(sheet: &Sheet, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * sheet.width + x) * 4) as usize;
        sheet.rgba[i..i + 4].try_into().unwrap()
    }

    fn painted(sheet: &Sheet) -> usize {
        sheet.rgba.chunks(4).filter(|p| *p != BLACK).count()
    }

    /// A stroke covers the pixels along it at the brush's width, and nothing past its round
    /// ends.
    #[test]
    fn a_segment_covers_its_width_and_stops_at_its_caps() {
        let mut sheet = Sheet::blank(64, 64, BLACK);
        sheet.segment(&pen(4.0), Symmetry::None, (10.0, 32.0), (50.0, 32.0));
        assert_eq!(at(&sheet, 30, 32), WHITE, "the middle of the stroke");
        assert_eq!(at(&sheet, 30, 31), WHITE, "a pixel inside its width");
        assert_eq!(at(&sheet, 30, 40), BLACK, "well outside its width");
        assert_eq!(at(&sheet, 5, 32), BLACK, "past the round end");
        // Round ends: the cap reaches two pixels past the point, and a corner of a square
        // cap would not be covered by a round one.
        assert_ne!(at(&sheet, 51, 32), BLACK, "the cap");
        assert_eq!(at(&sheet, 52, 34), BLACK, "a square cap's corner");
    }

    /// A press that has not moved is a dot, silvia's `_drawStroke(x, y, x, y)`.
    #[test]
    fn a_press_that_does_not_move_is_a_dot() {
        let mut sheet = Sheet::blank(32, 32, BLACK);
        sheet.segment(&pen(6.0), Symmetry::None, (16.0, 16.0), (16.0, 16.0));
        assert_eq!(at(&sheet, 16, 16), WHITE);
        let n = painted(&sheet);
        assert!((20..=50).contains(&n), "a dot six across, not {n} pixels");
    }

    /// Source-over on unpremultiplied bytes: half-transparent white over black is gray, and
    /// the pixel stays opaque.
    #[test]
    fn a_translucent_brush_blends_over_what_is_there() {
        let mut sheet = Sheet::blank(16, 16, BLACK);
        let brush = Brush {
            size: 8.0,
            color: [255, 255, 255, 128],
            erase: false,
        };
        sheet.segment(&brush, Symmetry::None, (8.0, 8.0), (8.0, 8.0));
        let p = at(&sheet, 8, 8);
        assert_eq!(p[3], 255, "opaque stays opaque");
        assert!((120..=136).contains(&p[0]), "gray, not {p:?}");
    }

    /// The eraser is `destination-out`: the painting goes transparent rather than taking the
    /// background, and its color stays under the alpha.
    #[test]
    fn the_eraser_takes_the_alpha_away() {
        let mut sheet = Sheet::blank(16, 16, [200, 10, 10, 255]);
        let eraser = Brush {
            erase: true,
            ..pen(6.0)
        };
        sheet.segment(&eraser, Symmetry::None, (8.0, 8.0), (8.0, 8.0));
        assert_eq!(at(&sheet, 8, 8), [200, 10, 10, 0]);
        assert_eq!(at(&sheet, 0, 0), [200, 10, 10, 255], "and nothing else");
    }

    /// Each symmetry puts one stroke where silvia's transforms do.
    #[test]
    fn symmetry_repeats_a_stroke_where_silvia_does() {
        let (w, h) = (100.0, 100.0);
        assert_eq!(Symmetry::None.images(w, h, (10.0, 20.0)), [(10.0, 20.0)]);
        assert_eq!(
            Symmetry::H.images(w, h, (10.0, 20.0)),
            [(10.0, 20.0), (90.0, 20.0)]
        );
        assert_eq!(
            Symmetry::V.images(w, h, (10.0, 20.0)),
            [(10.0, 20.0), (10.0, 80.0)]
        );
        assert_eq!(
            Symmetry::Hv.images(w, h, (10.0, 20.0)),
            [(10.0, 20.0), (90.0, 20.0), (10.0, 80.0), (90.0, 80.0)]
        );
        let four = Symmetry::Fold(4).images(w, h, (50.0, 10.0));
        assert_eq!(four.len(), 4);
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(near(four[0], (50.0, 10.0)), "the point itself first");
        assert!(
            near(four[1], (90.0, 50.0)),
            "then a quarter turn: {:?}",
            four[1]
        );
        assert!(near(four[2], (50.0, 90.0)));
        assert!(near(four[3], (10.0, 50.0)));
        for (value, _) in DEF.option(SYMMETRY).unwrap().choices {
            let expected = match *value {
                "none" => 1,
                "h" | "v" => 2,
                "hv" => 4,
                n => n[1..].parse().unwrap(),
            };
            assert_eq!(
                Symmetry::of(value).images(w, h, (1.0, 2.0)).len(),
                expected,
                "{value}"
            );
        }
    }

    /// A stroke under a mirror lands on both sides of the picture.
    #[test]
    fn a_mirrored_stroke_paints_both_halves() {
        let mut sheet = Sheet::blank(64, 64, BLACK);
        sheet.segment(&pen(3.0), Symmetry::H, (8.0, 8.0), (8.0, 20.0));
        assert_eq!(at(&sheet, 8, 14), WHITE);
        assert_eq!(at(&sheet, 55, 14), WHITE, "its mirror, at 64 - 8");
        assert_eq!(at(&sheet, 32, 14), BLACK);
    }

    /// `strokeRect`: an outline with square corners, hollow inside.
    #[test]
    fn a_rectangle_is_an_outline_with_square_corners() {
        let mut sheet = Sheet::blank(64, 64, BLACK);
        sheet.shape(
            Tool::Rect,
            &pen(2.0),
            Symmetry::None,
            (10.0, 10.0),
            (50.0, 40.0),
        );
        assert_eq!(at(&sheet, 30, 10), WHITE, "the top edge");
        assert_eq!(at(&sheet, 50, 25), WHITE, "the right edge");
        assert_eq!(at(&sheet, 30, 25), BLACK, "hollow");
        assert_eq!(at(&sheet, 9, 9), WHITE, "a square outer corner");
    }

    /// The ellipse inside the dragged box, outline only.
    #[test]
    fn a_circle_is_the_ellipse_inside_its_box() {
        let mut sheet = Sheet::blank(64, 64, BLACK);
        sheet.shape(
            Tool::Circle,
            &pen(2.0),
            Symmetry::None,
            (12.0, 12.0),
            (52.0, 52.0),
        );
        assert_ne!(at(&sheet, 32, 12), BLACK, "the top of the circle");
        assert_ne!(at(&sheet, 51, 32), BLACK, "its right");
        assert_eq!(at(&sheet, 32, 32), BLACK, "hollow");
        assert_eq!(
            at(&sheet, 13, 13),
            BLACK,
            "the box's corner is not the circle's"
        );
    }

    /// The fill takes the region the click is in and stops at a stroke across it.
    #[test]
    fn a_fill_stops_at_the_line_around_it() {
        let mut sheet = Sheet::blank(32, 32, BLACK);
        sheet.segment(&pen(2.0), Symmetry::None, (16.0, 0.0), (16.0, 32.0));
        let red = [255, 0, 0, 255];
        sheet.fill(red, (4.0, 4.0));
        assert_eq!(at(&sheet, 0, 0), red);
        assert_eq!(at(&sheet, 10, 30), red);
        assert_eq!(at(&sheet, 25, 10), BLACK, "the far side of the line");
        // Filling with the color already there does nothing, and so does a click off it.
        let before = sheet.clone();
        sheet.fill(red, (4.0, 4.0));
        sheet.fill(red, (-3.0, 4.0));
        assert_eq!(sheet, before);
    }

    /// A painting at one size stretched to another keeps where things are.
    #[test]
    fn a_resized_canvas_keeps_its_picture() {
        let mut sheet = Sheet::blank(8, 8, BLACK);
        sheet.rgba[0..4].copy_from_slice(&WHITE);
        let big = resampled(8, 8, &sheet.rgba, 16, 16);
        assert_eq!((big.width, big.height), (16, 16));
        assert_eq!(at(&big, 0, 0), WHITE);
        assert_eq!(at(&big, 1, 1), WHITE, "one pixel is four now");
        assert_eq!(at(&big, 2, 2), BLACK);
    }

    /// Every Canvas Size is a size this reads, and the default is silvia's 512 square.
    #[test]
    fn every_canvas_size_is_a_size() {
        for (value, _) in DEF.option(CANVAS_SIZE).unwrap().choices {
            let (w, h) = size_of(value);
            assert_eq!(format!("{w}x{h}"), *value);
        }
        assert_eq!(
            size_of(DEF.option(CANVAS_SIZE).unwrap().default),
            (512, 512)
        );
        assert_eq!(size_of("rubbish"), (512, 512));
    }

    /// Every tool choice is a tool, in silvia's order, with silvia's keys.
    #[test]
    fn every_tool_choice_is_a_tool() {
        let choices = DEF.option(TOOL).unwrap().choices;
        assert_eq!(choices.len(), Tool::ALL.len());
        for ((value, label), tool) in choices.iter().zip(Tool::ALL) {
            assert_eq!(Tool::of(value), tool);
            assert_eq!(tool.label(), *label);
        }
        let keys: String = Tool::ALL.iter().map(|t| t.shortcut()).collect();
        assert_eq!(keys, "BELRCF");
    }

    /// A stroke number is never handed out twice, so a finished stroke is always newer than
    /// every picture before it.
    #[test]
    fn stroke_numbers_only_go_up() {
        let a = next_stroke();
        let b = next_stroke();
        assert!(b > a && a > 0);
    }
}
