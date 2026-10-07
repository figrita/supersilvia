// SPDX-License-Identifier: AGPL-3.0-or-later

//! The only primitive that owns memory: a render target, and a published `outputTexture` that
//! its `frame` port hands to any other graph as a texture. That is what makes an Output
//! simultaneously the screen, an intermediate buffer, and a feedback source (§2b).

use crate::graph::PortType::{Action, UniformNumber, VaryingColor};
use crate::nodes::{
    Category, Control, InputDef, NodeDef, OFF, ON, OptionDef, OptionKind, OutputDef, OutputKind,
};

/// The render's three numbers, in the order their rows are drawn: silvia's offline output's
/// FPS, Duration and Warm-up. Hidden controls — document data with no port — that the
/// Output's Render section draws on rows of its own.
pub const RENDER_CONTROLS: [&str; 3] = ["fps", "duration", "warmup"];

/// The render's three selects, drawn above its numbers in the same section rather than in
/// the option block, so the whole of the render folds under the one **Render** heading.
///
/// Supersampling first, because it is the one that says what the render is *of*: the
/// Output's resolution times a multiplier. Then what was on screen before frame zero, then
/// what the frames are written into.
pub const RENDER_SELECTS: [&str; 3] = ["supersampling", "warmupMode", "writer"];

/// The heading the render section folds under. Closed on a new Output: silvia's output has
/// no render on it, and a set that never renders should not carry six rows of it.
///
/// The key stays `offline` — silvia's own name for the section, and what every saved file
/// holds — while the heading reads **Render**, which is what is under it.
pub const OFFLINE: &str = "offline";

/// The live recording's frame rate: a hidden control of its own, drawn on the Record
/// section's first row and read once, at Record. Apart from the Render section's FPS, so a
/// film rendered at one rate and a set recorded at another are both set once.
pub const RECORD_FPS: &str = "recordFps";

/// The heading the live recording folds under: its FPS and its Record row. Closed on a new
/// Output, as the Render section is.
///
/// Not `record`, which is the Record row's button's name.
pub const RECORD: &str = "recording";

/// The Output's numbers no MIDI or OSC binding reaches: the render's three and the
/// recording's FPS.
const UNBINDABLE: [&str; 4] = ["fps", "duration", "warmup", RECORD_FPS];

pub static DEF: NodeDef = NodeDef {
    slug: "output",
    category: Category::Output,
    icon: "📺",
    label: "Output",
    tooltip: "Renders its input. Its frame port republishes the result as a texture.",
    inputs: &[
        InputDef {
            key: "input",
            label: "Input",
            ty: VaryingColor,
            // Must connect. An Output with nothing plugged in has no shader and renders black.
            control: Control::None,
        },
        // Action inputs that are also buttons: a hand claims a deck, and so can a sequencer,
        // which is what lets a patch cut between two Outputs. `App` reads them; no shader
        // ever sees them.
        InputDef {
            key: "show_a",
            label: "Show on A",
            ty: Action,
            control: Control::Press,
        },
        InputDef {
            key: "show_b",
            label: "Show on B",
            ty: Action,
            control: Control::Press,
        },
        // silvia's Snap, and an action like the two above it: a picture leaves the running
        // patch at the moment it is asked for, whether a hand or a sequencer asks. The
        // renderer reads it, as it reads the decks; no shader ever sees it.
        InputDef {
            key: "snap",
            label: "Snap",
            ty: Action,
            control: Control::Press,
        },
    ],
    hidden: &[
        InputDef {
            key: "fps",
            label: "FPS",
            ty: UniformNumber,
            control: Control::num(30.0, 1.0, 120.0, 1.0, ""),
        },
        InputDef {
            key: "duration",
            label: "Duration",
            ty: UniformNumber,
            control: Control::num(10.0, 0.1, 3600.0, 0.1, "s"),
        },
        InputDef {
            key: "warmup",
            label: "Warm-up",
            ty: UniformNumber,
            control: Control::num(0.0, 0.0, 600.0, 1.0, "fr"),
        },
        InputDef {
            key: RECORD_FPS,
            label: "FPS",
            ty: UniformNumber,
            control: Control::num(30.0, 1.0, 120.0, 1.0, ""),
        },
    ],
    outputs: &[OutputDef {
        key: "frame",
        label: "Frame Out",
        ty: VaryingColor,
        kind: OutputKind::Texture,
        // Remaps worldspace uv into the texture's own [0,1] space using its real aspect, so
        // an Output of one resolution can sample an Output of another.
        wgsl: |node, ctx, _func| {
            let tex = ctx.texture_uniform(node, "frame");
            let sampler = ctx.sampler(node, "frame");
            format!(
                "    let texSize = vec2f(textureDimensions({tex}));
    let aspect = texSize.x / texSize.y;
    let t = vec2f((uv.x / aspect + 1.0) * 0.5, (uv.y + 1.0) * 0.5);
    return textureSampleLevel({tex}, {sampler}, t, 0.0);"
            )
        },
        ..OutputDef::EMPTY
    }],
    options: &[
        OptionDef {
            key: "resolution",
            label: "Resolution",
            default: "1280x720",
            // Any `WIDTHxHEIGHT` the resolution picker writes; the one choice is the
            // default's home.
            choices: &[("1280x720", "1280x720")],
            // `App` reads it into every frame's job and the renderer resizes from that,
            // keeping the program: the shader is written against `u_resolution` and does
            // not change.
            kind: OptionKind::Runtime,
            resolution: true,
            ..OptionDef::EMPTY
        },
        // How much larger than the Output the render is drawn, before it comes back down
        // to the Output's own size: silvia's three, in silvia's words. The live picture is
        // untouched — this is read once, when a render starts, and put back when it ends.
        OptionDef {
            key: "supersampling",
            label: "Supersampling",
            default: "1",
            choices: &[("1", "1x (off)"), ("2", "2x"), ("4", "4x")],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        // What was on screen before frame zero of a render. silvia's three, in silvia's
        // words; `clock::Warmup` is what each becomes.
        OptionDef {
            key: "warmupMode",
            label: "Warm-up Mode",
            default: "sequence",
            choices: &[
                ("black", "Black"),
                ("hold", "Hold First Frame"),
                ("sequence", "Run Sequence"),
            ],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        // What a render writes: a PNG sequence is the lossless hand-off to an editor, a
        // video file is the thing you send someone, an animated GIF the thing you post.
        OptionDef {
            key: "writer",
            label: "Writer",
            default: "png",
            choices: &[
                ("png", "PNG sequence"),
                ("video", "Video file"),
                ("gif", "Animated GIF"),
            ],
            kind: OptionKind::Runtime,
            ..OptionDef::EMPTY
        },
        OptionDef::heading(OFFLINE, "Render", false, OptionKind::Presentation),
        OptionDef::heading(RECORD, "Record", false, OptionKind::Presentation),
        // Sent over NDI to every machine on the network and, on a Mac, published over Syphon to
        // every other app there, top row first rather than Syphon's bottom row first where Flip
        // says; both ways with its own alpha rather than opaque over black where Alpha says.
        // Read by `App`, which hands the publisher what is on; no shader ever sees them. Drawn
        // under the Send heading as rows of their own (`SendRow`), not as selects.
        OptionDef::heading(SEND, "Send", false, OptionKind::Presentation),
        // The whole name it goes out under, both ways: empty for the default, which is never
        // written out, so a copy of an Output with no name of its own takes its own number.
        // Typed rather than chosen, so it has no choices; see [`send_name`].
        OptionDef {
            key: SEND_NAME,
            label: "Name",
            default: "",
            kind: OptionKind::Runtime,
            placeholder: Some(""),
            ..OptionDef::EMPTY
        },
        OptionDef {
            key: NDI,
            label: "NDI®",
            ..SWITCH
        },
        OptionDef {
            key: SYPHON,
            label: "Syphon",
            ..SWITCH
        },
        OptionDef {
            key: SYPHON_FLIP,
            label: "Flip",
            choices: &[(OFF, "Bottom first"), (ON, "Top first")],
            ..SWITCH
        },
        OptionDef {
            key: TRANSPARENT,
            label: "Alpha",
            choices: &[(OFF, "Opaque"), (ON, "Transparent")],
            ..SWITCH
        },
    ],
    is_output: true,
    row_headings: &[OFFLINE, RECORD, SEND],
    // silvia's Frame History is `midi-disabled`, and these are its kin: settings for a
    // render or a recording, which nobody turns mid-set.
    unbindable: &UNBINDABLE,
    regions: &[crate::nodes::Region::Render],
    ..NodeDef::EMPTY
};

/// A send option's shape: `on` or `off`, read at run time, off on a new Output.
const SWITCH: OptionDef = OptionDef {
    default: OFF,
    choices: crate::nodes::CHECK_CHOICES,
    kind: OptionKind::Runtime,
    ..OptionDef::EMPTY
};

/// The heading the send rows fold under. Open on a new Output, so the ways out are in view.
pub const SEND: &str = "send";

/// The options that send an Output out — over NDI, over Syphon, top row first where
/// `syphonFlip` says — and the alpha both ways share. See [`sent_of`]. Drawn in the Send
/// section as [`SendRow`]s rather than in the option block.
pub const NDI: &str = "ndi";
pub const SYPHON: &str = "syphon";
pub const SYPHON_FLIP: &str = "syphonFlip";
pub const TRANSPARENT: &str = "transparent";
pub const SWITCHES: [&str; 4] = [NDI, SYPHON, SYPHON_FLIP, TRANSPARENT];

/// The name it is sent under, both ways; empty for [`default_name`]. See [`send_name`].
pub const SEND_NAME: &str = "sendName";

/// Every option the Send section draws: the name and the four switches.
pub const SEND_OPTIONS: [&str; 5] = [SEND_NAME, NDI, SYPHON, SYPHON_FLIP, TRANSPARENT];

/// Which ways an Output is sent out, and how: its four send options, as this machine can
/// act on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)] // four switches, each on or off on its own
pub struct Sent {
    /// Published over Syphon. Never on a machine without Syphon, whatever the file says.
    pub syphon: bool,
    /// Top row first, rather than Syphon's own bottom row first. Syphon's alone.
    pub flip: bool,
    /// Sent over NDI.
    pub ndi: bool,
    /// The picture's own alpha, rather than opaque over black, both ways.
    pub transparent: bool,
}

impl Sent {
    /// Whether it leaves at all.
    pub fn any(self) -> bool {
        self.syphon || self.ndi
    }
}

/// How this Output is sent out, by its options.
///
/// The keys and how to read them belong to this module, as `resolution` does. A file from a
/// Mac keeps its `syphon` and `syphonFlip` on a machine without Syphon, and they read as off
/// there: nothing is published, and nothing about it is drawn.
pub fn sent_of(node: &crate::graph::Node) -> Sent {
    let on = |key: &str| node.options.get(key).is_some_and(|v| v == ON);
    let syphon = crate::platform::syphon::available();
    Sent {
        syphon: syphon && on(SYPHON),
        flip: syphon && on(SYPHON_FLIP),
        ndi: on(NDI),
        transparent: on(TRANSPARENT),
    }
}

/// The name an Output goes out under when it has none of its own: the app's, the kind's and
/// its id, which no other node in the project shares and which a save keeps.
pub fn default_name(id: crate::graph::NodeId) -> String {
    format!("supersilvia Output {}", id.0)
}

/// The one name an Output is sent under, over NDI and over Syphon alike: its own where it has
/// one, [`default_name`] where it has none. NDI puts the machine's name beside it and Syphon
/// the app's.
pub fn send_name(id: crate::graph::NodeId, node: &crate::graph::Node) -> String {
    node.options
        .get(SEND_NAME)
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map_or_else(|| default_name(id), str::to_string)
}

/// The longest name an Output is sent under, in bytes. NDI announces a stream over mDNS as
/// `MACHINE (name)`, one DNS label of 63 bytes at most; 48 leaves the machine room.
pub const NAME_BYTES: usize = 48;

/// What a name that clashes has appended, as often as it takes.
const COPY: &str = " copy";

/// The mix's own name over NDI, which no Output may take.
pub const MIX_NAME: &str = "supersilvia Mix";

/// `wanted` as a name no other sender holds: trimmed, cut to [`NAME_BYTES`], `default` where
/// it is empty, and then ` copy` appended until `taken` says no — or, where that would pass
/// [`NAME_BYTES`], `default` itself, which is the Output's number and so its own.
///
/// Names are told apart as the network tells them, without regard to case.
pub fn unique_name(wanted: &str, default: &str, taken: impl Fn(&str) -> bool) -> String {
    let wanted = wanted.trim();
    let mut name = if wanted.is_empty() {
        default.to_string()
    } else {
        let mut end = wanted.len().min(NAME_BYTES);
        while !wanted.is_char_boundary(end) {
            end -= 1;
        }
        wanted[..end].trim_end().to_string()
    };
    while taken(&name) {
        name.push_str(COPY);
        if name.len() > NAME_BYTES {
            return default.to_string();
        }
    }
    name
}

/// Whether two names are the same sender's, as the network tells them.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// What an Output's name option should hold for `wanted` typed into it: [`unique_name`]
/// against every other Output in the project and the mix, and empty where that is the
/// Output's default.
pub fn fit_name(graph: &crate::graph::Graph, id: crate::graph::NodeId, wanted: &str) -> String {
    let others: Vec<String> = graph
        .iter()
        .filter(|(other, node)| *other != id && node.def.is_output)
        .map(|(other, node)| send_name(other, node))
        .collect();
    stored(
        id,
        &unique_name(wanted, &default_name(id), taken_by(&others)),
    )
}

/// Make the names of `newcomers` — Outputs just pasted, duplicated, imported or opened —
/// unique among every Output in the graph, in the order given: each is held against the
/// Outputs that are not newcomers and the newcomers before it, so of two sharing a name the
/// later one takes the ` copy`. Returns whether any name changed.
pub fn settle_names(graph: &mut crate::graph::Graph, newcomers: &[crate::graph::NodeId]) -> bool {
    let mut names: Vec<String> = graph
        .iter()
        .filter(|(id, node)| node.def.is_output && !newcomers.contains(id))
        .map(|(id, node)| send_name(id, node))
        .collect();
    let mut changed = false;
    for &id in newcomers {
        let Some(node) = graph.get(id).filter(|n| n.def.is_output) else {
            continue;
        };
        let now = send_name(id, node);
        let name = unique_name(&now, &default_name(id), taken_by(&names));
        if name != now {
            let value = stored(id, &name);
            if let Some(node) = graph.get_mut(id) {
                node.options.insert(SEND_NAME, value);
            }
            changed = true;
        }
        names.push(name);
    }
    changed
}

/// Whether a name is held by one of `names` or by the mix.
fn taken_by(names: &[String]) -> impl Fn(&str) -> bool + '_ {
    |name| same_name(name, MIX_NAME) || names.iter().any(|n| same_name(n, name))
}

/// What the option holds for `name`: empty for the default, which is never written down.
fn stored(id: crate::graph::NodeId, name: &str) -> String {
    if name == default_name(id) {
        String::new()
    } else {
        name.to_string()
    }
}

/// One way an Output leaves: a row of its own under **Send**.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    Ndi,
    Syphon,
}

impl Way {
    /// The option that turns it on.
    pub fn key(self) -> &'static str {
        match self {
            Self::Ndi => NDI,
            Self::Syphon => SYPHON,
        }
    }

    /// What its row is called.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ndi => "NDI®",
            Self::Syphon => "Syphon",
        }
    }

    /// What it does, for the row's hover.
    fn about(self, name: &str) -> String {
        match self {
            Self::Ndi => format!(
                "Sent over NDI® as “{name}”, for other machines on the network to take as a \
                 source. ndi.video"
            ),
            Self::Syphon => format!(
                "Published over Syphon as “{name}”, for another app on this Mac to take as a \
                 source."
            ),
        }
    }
}

/// One row under an Output's **Send** heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRow {
    /// The name it goes out under, both ways: a field, first, whether or not anything sends.
    Name,
    /// A way out: its name, one line saying what it is doing, and the button that changes it.
    Way(Way),
    /// Syphon's Flip, under the Syphon row while it publishes.
    Flip,
    /// The alpha both ways share, once, after the ways, while either sends.
    Alpha,
}

impl SendRow {
    /// Every row there can be, top to bottom.
    pub const ALL: [Self; 5] = [
        Self::Name,
        Self::Way(Way::Ndi),
        Self::Way(Way::Syphon),
        Self::Flip,
        Self::Alpha,
    ];

    /// Whether this row is drawn, from the options and the machine alone: what a sender
    /// reports changes what a row says and never whether it is there.
    pub fn shown(self, sent: Sent) -> bool {
        match self {
            Self::Name | Self::Way(Way::Ndi) => true,
            Self::Way(Way::Syphon) => crate::platform::syphon::available(),
            Self::Flip => sent.syphon,
            Self::Alpha => sent.any(),
        }
    }

    /// The option this row sets.
    pub fn key(self) -> &'static str {
        match self {
            Self::Name => SEND_NAME,
            Self::Way(way) => way.key(),
            Self::Flip => SYPHON_FLIP,
            Self::Alpha => TRANSPARENT,
        }
    }
}

/// The rows this Output draws under **Send**, top to bottom.
pub fn send_rows(node: &crate::graph::Node) -> impl Iterator<Item = SendRow> + use<> {
    let sent = sent_of(node);
    SendRow::ALL.into_iter().filter(move |row| row.shown(sent))
}

/// What a way's button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Turn it on.
    Send,
    /// Turn it off.
    Stop,
    /// Open the page the runtime is downloaded from.
    GetIt,
    /// Start recording.
    Record,
}

impl Press {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Send => "Send",
            Self::Stop => "Stop",
            Self::GetIt => "Get it",
            Self::Record => "Record",
        }
    }
}

/// What a way's row says: whether it is on air, its one line, and its button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status<'a> {
    pub on_air: bool,
    pub line: std::borrow::Cow<'a, str>,
    pub press: Press,
}

/// What a way's row says, from its option, the name it goes out under, what the sender last
/// reported and, for NDI, why the machine cannot send at all.
///
/// A missing runtime wins over everything: nothing is sent, the line says whether there is no
/// runtime or one that will not load, and the button is the download until the way is switched
/// off — where it was on, from a file made on a machine that had the runtime, it is **Stop**,
/// so it can still be switched off here.
pub fn status<'a>(
    on: bool,
    name: &str,
    error: Option<&'a str>,
    missing: Option<crate::video::ndi::Missing>,
) -> Status<'a> {
    use std::borrow::Cow;
    let press = if on { Press::Stop } else { Press::Send };
    match (missing, on, error) {
        (Some(missing), _, _) => Status {
            on_air: false,
            line: Cow::Borrowed(if missing.found {
                "runtime won't load"
            } else {
                "runtime not installed"
            }),
            press: if on { Press::Stop } else { Press::GetIt },
        },
        (None, false, _) => Status {
            on_air: false,
            line: Cow::Borrowed("off"),
            press,
        },
        (None, true, Some(error)) => Status {
            on_air: false,
            line: Cow::Borrowed(error),
            press,
        },
        (None, true, None) => Status {
            on_air: true,
            line: Cow::Owned(format!("on air · {name}")),
            press,
        },
    }
}

/// What a way's row says under the pointer: the whole of whatever its line had to cut short,
/// or what the way does.
pub fn status_hover(way: Way, name: &str, error: Option<&str>, missing: Option<&str>) -> String {
    missing
        .or(error)
        .map_or_else(|| way.about(name), str::to_string)
}

/// What the name field says under the pointer.
pub fn name_hover(default: &str) -> String {
    format!(
        "The whole name this Output is sent under, every way it goes out. Enter sets it, and \
         empty goes back to “{default}”. A name another Output has gets “ copy” until it is \
         its own. Renamed while on air, the stream starts again under the new name: receivers \
         see the old source go and a new one appear."
    )
}

/// What the Record row under the Record heading says, the way a way out's row does: lit while
/// it records, with how long on the show's clock and how many frames repeat the one before for
/// lack of a picture; why not while a render runs; and otherwise why the last press failed, or
/// *off*. `recording` is the seconds and the dropped frames.
pub fn record_status(
    recording: Option<(f64, u64)>,
    error: Option<&str>,
    rendering: bool,
) -> Status<'_> {
    use std::borrow::Cow;
    match (recording, rendering, error) {
        (Some((seconds, dropped)), _, _) => Status {
            on_air: true,
            line: Cow::Owned(format!("{} · {dropped} dropped", clock(seconds))),
            press: Press::Stop,
        },
        (None, true, _) => Status {
            on_air: false,
            line: Cow::Borrowed("not while rendering"),
            press: Press::Record,
        },
        (None, false, Some(error)) => Status {
            on_air: false,
            line: Cow::Borrowed(error),
            press: Press::Record,
        },
        (None, false, None) => Status {
            on_air: false,
            line: Cow::Borrowed("off"),
            press: Press::Record,
        },
    }
}

/// What the Record row says under the pointer: the whole of a failure its line cut short, or
/// what recording does.
pub fn record_hover(error: Option<&str>) -> String {
    error.map_or_else(
        || {
            "Records this Output's picture to a video in recordings/ as the show plays, at \
             the FPS above and the Output's resolution, until Stop. Picture only. A \
             frame the show did not draw in time repeats the one before and counts as \
             dropped. No render runs beside it."
                .to_string()
        },
        str::to_string,
    )
}

/// A recording's length as its row and the status line say it: minutes and seconds, and hours
/// in front past the first.
pub fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (whole / 3600, (whole % 3600) / 60, whole % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// What an Output renders at when its option says nothing this build understands. Kept
/// beside the `default` above, and held to it by a test.
pub const DEFAULT_RESOLUTION: (u32, u32) = (1280, 720);

/// The narrowest and the widest side a resolution option holds, in pixels. The wide end is
/// the 2D texture limit of every GPU the app runs on.
pub const MIN_SIDE: u32 = 16;
pub const MAX_SIDE: u32 = 16384;

/// Parse a `resolution` option value. Returns `None` for anything not `WIDTHxHEIGHT`.
pub fn parse_resolution(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Whether `value` is a `WIDTHxHEIGHT` with both sides inside [`MIN_SIDE`] and [`MAX_SIDE`]:
/// what an option drawn by the resolution picker accepts from a file.
pub fn holds_resolution(value: &str) -> bool {
    parse_resolution(value).is_some_and(|(w, h)| {
        (MIN_SIDE..=MAX_SIDE).contains(&w) && (MIN_SIDE..=MAX_SIDE).contains(&h)
    })
}

/// A resolution option's value as a size to draw at: clamped into [`MIN_SIDE`] and
/// [`MAX_SIDE`], and [`DEFAULT_RESOLUTION`] for anything not `WIDTHxHEIGHT`.
pub fn read_resolution(value: &str) -> (u32, u32) {
    parse_resolution(value).map_or(DEFAULT_RESOLUTION, |(w, h)| {
        (w.clamp(MIN_SIDE, MAX_SIDE), h.clamp(MIN_SIDE, MAX_SIDE))
    })
}

/// One Output node's render size.
///
/// The `resolution` key and how to read it belong to this module. Everywhere else asks for
/// the answer rather than for the option.
pub fn resolution_of(node: &crate::graph::Node) -> (u32, u32) {
    node.options
        .get("resolution")
        .map_or(DEFAULT_RESOLUTION, |v| read_resolution(v))
}

/// How much larger than its own resolution an Output renders a film, and comes back down
/// from: 1, 2 or 4.
///
/// The key and how to read it belong to this module, as `resolution` does. Anything the
/// option does not say — a file from a build that had no supersampling, a value this one
/// does not know — is 1x, which is the render as it was before the multiplier existed.
pub fn supersampling_of(node: &crate::graph::Node) -> u32 {
    match node.options.get("supersampling").map(String::as_str) {
        Some("2") => 2,
        Some("4") => 4,
        _ => 1,
    }
}

/// How many render targets one Output holds at its own resolution: the temporary one every
/// frame is drawn into and the published one it is copied to. Both are RGBA16F, which is
/// eight bytes a pixel — see `new_target`.
const TARGETS: u64 = 2;
const BYTES_PER_PIXEL: u64 = 8;

/// How much VRAM this Output has committed, in bytes.
///
/// silvia prints a memory figure under its resolution and moves it as the resolution moves,
/// so a second Output at 4K can be thought about before it is made. This is that figure: the
/// two half-float targets the renderer allocates per Output and nothing else. A capture ring
/// is not in it — it exists only while a render runs, and the number is here to be read
/// while one is not.
pub fn memory_bytes(node: &crate::graph::Node) -> u64 {
    let (w, h) = resolution_of(node);
    memory_at(w, h)
}

/// What an Output of `width` x `height` commits, in bytes: [`memory_bytes`] for a size rather
/// than a node, which the resolution picker prints for the size it holds and each it offers.
pub fn memory_at(width: u32, height: u32) -> u64 {
    u64::from(width) * u64::from(height) * BYTES_PER_PIXEL * TARGETS
}

/// [`memory_at`] in whole megabytes, rounded up: the grain the choice is made at.
pub fn megabytes_at(width: u32, height: u32) -> u64 {
    memory_at(width, height).div_ceil(1024 * 1024)
}

/// That figure as a node prints it: whole megabytes.
pub fn memory_label(node: &crate::graph::Node) -> String {
    let (w, h) = resolution_of(node);
    format!("{} MB", megabytes_at(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a file can carry into the option, and what the renderer is handed for it: a size
    /// inside the bounds as it is, one outside them clamped, anything else the default.
    #[test]
    fn a_resolution_is_read_inside_its_bounds() {
        assert!(holds_resolution("1920x1080"));
        assert!(holds_resolution("16x16384"));
        assert!(!holds_resolution("8x1080"));
        assert!(!holds_resolution("1920x99999"));
        assert!(!holds_resolution("1920"));
        assert_eq!(read_resolution("2001x999"), (2001, 999));
        assert_eq!(read_resolution("0x99999"), (MIN_SIDE, MAX_SIDE));
        assert_eq!(read_resolution("banana"), DEFAULT_RESOLUTION);
    }

    #[test]
    fn the_fallback_matches_the_definitions_default() {
        let default = DEF.option("resolution").unwrap().default;
        assert_eq!(parse_resolution(default), Some(DEFAULT_RESOLUTION));
    }

    #[test]
    fn every_supersampling_choice_is_a_multiplier_this_build_knows() {
        let mut g = crate::graph::Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO)
            .expect("in the registry");
        for (value, _) in DEF.option("supersampling").unwrap().choices {
            g.get_mut(id)
                .expect("just added")
                .options
                .insert("supersampling", (*value).to_string());
            let scale = supersampling_of(g.get(id).expect("just added"));
            assert_eq!(scale.to_string(), *value, "{value:?} is not a multiplier");
            assert!(matches!(scale, 1 | 2 | 4));
        }
    }

    #[test]
    fn an_unknown_supersampling_is_the_render_as_it_was() {
        let mut g = crate::graph::Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO)
            .expect("in the registry");
        g.get_mut(id)
            .expect("just added")
            .options
            .insert("supersampling", "8".to_string());
        assert_eq!(
            supersampling_of(g.get(id).expect("just added")),
            1,
            "a multiplier this build does not know is 1x"
        );
    }

    /// The figure moves with the resolution, because that is the whole of what it is for.
    #[test]
    fn the_memory_figure_follows_the_resolution() {
        let mut g = crate::graph::Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO)
            .expect("in the registry");
        let at = |g: &crate::graph::Graph, id| memory_bytes(g.get(id).expect("just added"));
        let small = at(&g, id);
        assert_eq!(small, 1280 * 720 * 8 * 2);
        assert_eq!(memory_label(g.get(id).expect("just added")), "15 MB");
        g.get_mut(id)
            .expect("just added")
            .options
            .insert("resolution", "1920x1080".to_string());
        assert!(at(&g, id) > small, "a bigger picture costs more");
    }

    fn with(options: &[(&'static str, &str)]) -> (crate::graph::Graph, crate::graph::NodeId) {
        let mut g = crate::graph::Graph::new();
        let id = crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO)
            .expect("in the registry");
        for (key, value) in options {
            g.get_mut(id)
                .expect("just added")
                .options
                .insert(key, (*value).to_string());
        }
        (g, id)
    }

    fn rows(options: &[(&'static str, &str)]) -> Vec<SendRow> {
        let (g, id) = with(options);
        send_rows(g.get(id).expect("just added")).collect()
    }

    /// A new Output sends nothing and says so with its name and one row per way this machine
    /// has: NDI everywhere, Syphon on a Mac alone.
    #[test]
    fn a_new_output_has_a_row_per_way_this_machine_has() {
        let expected: Vec<SendRow> = if crate::platform::syphon::available() {
            vec![
                SendRow::Name,
                SendRow::Way(Way::Ndi),
                SendRow::Way(Way::Syphon),
            ]
        } else {
            vec![SendRow::Name, SendRow::Way(Way::Ndi)]
        };
        assert_eq!(rows(&[]), expected);
    }

    /// Alpha appears once either way is on, after the ways; Flip only under Syphon, while it
    /// publishes.
    #[test]
    fn alpha_and_flip_appear_with_what_they_shape() {
        let ndi = rows(&[(NDI, ON)]);
        assert_eq!(ndi.last(), Some(&SendRow::Alpha));
        assert!(!ndi.contains(&SendRow::Flip), "Flip is Syphon's alone");

        let syphon = rows(&[(SYPHON, ON)]);
        if crate::platform::syphon::available() {
            assert_eq!(
                syphon,
                [
                    SendRow::Name,
                    SendRow::Way(Way::Ndi),
                    SendRow::Way(Way::Syphon),
                    SendRow::Flip,
                    SendRow::Alpha,
                ]
            );
        } else {
            assert_eq!(
                syphon,
                [SendRow::Name, SendRow::Way(Way::Ndi)],
                "a Mac's Syphon, opened here, draws nothing and shapes nothing"
            );
        }
    }

    /// A file from a Mac keeps its Syphon options on a machine without Syphon, and nothing is
    /// published: the value is the file's, the answer is the machine's.
    #[test]
    fn syphon_from_a_file_is_kept_and_read_as_this_machine_can() {
        let (g, id) = with(&[(SYPHON, ON), (SYPHON_FLIP, ON)]);
        let node = g.get(id).expect("just added");
        assert_eq!(node.options.get(SYPHON).map(String::as_str), Some(ON));
        let sent = sent_of(node);
        let here = crate::platform::syphon::available();
        assert_eq!((sent.syphon, sent.flip, sent.any()), (here, here, here));
    }

    #[test]
    fn every_send_switch_is_on_or_off_and_off_on_a_new_output() {
        for key in SWITCHES {
            let option = DEF.option(key).expect("declared");
            assert_eq!(option.default, OFF, "{key}");
            let values: Vec<&str> = option.choices.iter().map(|(v, _)| *v).collect();
            assert_eq!(values.len(), 2, "{key}");
            assert!(values.contains(&ON) && values.contains(&OFF), "{key}");
            assert!(!option.checkbox && !option.heading, "{key} is a Send row");
        }
    }

    /// What a way's row says in each state it can be in.
    #[test]
    fn a_ways_row_says_what_it_is_doing() {
        let off = status(false, "", None, None);
        assert_eq!(
            (off.on_air, &*off.line, off.press),
            (false, "off", Press::Send)
        );

        let on = status(true, "supersilvia Output 12", None, None);
        assert_eq!(
            (on.on_air, &*on.line, on.press),
            (true, "on air · supersilvia Output 12", Press::Stop)
        );

        let failing = status(
            true,
            "supersilvia Output 12",
            Some("the pipeline stopped"),
            None,
        );
        assert_eq!(
            (failing.on_air, &*failing.line, failing.press),
            (false, "the pipeline stopped", Press::Stop),
            "on and failing: the error, and still a way to stop"
        );

        let none = crate::video::ndi::Missing {
            why: crate::video::ndi::MISSING,
            found: false,
        };
        let missing = status(false, "", None, Some(none));
        assert_eq!(
            (missing.on_air, &*missing.line, missing.press),
            (false, "runtime not installed", Press::GetIt)
        );
        let missing_on = status(true, "x", Some("ignored"), Some(none));
        assert_eq!(
            (missing_on.on_air, &*missing_on.line, missing_on.press),
            (false, "runtime not installed", Press::Stop),
            "switched on elsewhere: it can still be switched off here"
        );

        let broken = crate::video::ndi::Missing {
            why: "The NDI® runtime at /usr/local/lib/libndi.so.6 would not load: \
                  libavahi-client.so.3: cannot open shared object file",
            found: true,
        };
        let refused = status(false, "", None, Some(broken));
        assert_eq!(
            (refused.on_air, &*refused.line, refused.press),
            (false, "runtime won't load", Press::GetIt),
            "a runtime that is there and will not load is not called missing"
        );
        let refused_on = status(true, "x", Some("ignored"), Some(broken));
        assert_eq!(
            (refused_on.on_air, &*refused_on.line, refused_on.press),
            (false, "runtime won't load", Press::Stop)
        );
        assert_eq!(Press::GetIt.caption(), "Get it");
    }

    /// What the Record row says in each state it can be in: how long and how many dropped
    /// while it runs, the reason where the last press failed, and why not while a render runs.
    #[test]
    fn the_record_row_says_what_it_is_doing() {
        let off = record_status(None, None, false);
        assert_eq!(
            (off.on_air, &*off.line, off.press),
            (false, "off", Press::Record)
        );
        let on = record_status(Some((83.4, 2)), None, false);
        assert_eq!(
            (on.on_air, &*on.line, on.press),
            (true, "1:23 · 2 dropped", Press::Stop)
        );
        let failed = record_status(None, Some("no hardware video encoder"), false);
        assert_eq!(
            (failed.on_air, &*failed.line, failed.press),
            (false, "no hardware video encoder", Press::Record)
        );
        let rendering = record_status(None, Some("old news"), true);
        assert_eq!(
            (rendering.on_air, &*rendering.line, rendering.press),
            (false, "not while rendering", Press::Record)
        );
        assert_eq!(Press::Record.caption(), "Record");
    }

    #[test]
    fn a_recordings_clock_is_minutes_and_seconds_then_hours() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(7.9), "0:07");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00");
    }

    /// The hover is the whole of what the line cut short, or what the way does.
    #[test]
    fn a_ways_hover_is_the_whole_story() {
        assert_eq!(
            status_hover(Way::Ndi, "n", Some("e"), Some(crate::video::ndi::MISSING)),
            crate::video::ndi::MISSING
        );
        let refused = "The NDI® runtime at /usr/local/lib/libndi.so.6 would not load: \
                       libavahi-client.so.3: cannot open shared object file";
        assert_eq!(
            status_hover(Way::Ndi, "n", None, Some(refused)),
            refused,
            "the hover is the path and the loader's reason"
        );
        assert_eq!(status_hover(Way::Syphon, "n", Some("e"), None), "e");
        assert!(
            status_hover(Way::Ndi, "supersilvia Output 3", None, None)
                .contains("“supersilvia Output 3”")
        );
        assert!(status_hover(Way::Syphon, "Output 3", None, None).contains("“Output 3”"));
    }

    /// One name both ways: the default until the option says otherwise, and the option,
    /// trimmed, once it does.
    #[test]
    fn an_output_goes_out_under_one_name() {
        let (g, id) = with(&[]);
        let node = g.get(id).expect("just added");
        assert_eq!(send_name(id, node), format!("supersilvia Output {}", id.0));
        assert_eq!(send_name(id, node), default_name(id));
        let (g, id) = with(&[(SEND_NAME, "  warpzone ")]);
        assert_eq!(send_name(id, g.get(id).expect("just added")), "warpzone");
        let (g, id) = with(&[(SEND_NAME, "   ")]);
        assert_eq!(
            send_name(id, g.get(id).expect("just added")),
            default_name(id),
            "blank is the default"
        );
    }

    fn held<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        |name| names.contains(&name)
    }

    #[test]
    fn a_free_name_is_kept_trimmed() {
        assert_eq!(
            unique_name(" warpzone ", "supersilvia Output 3", held(&[])),
            "warpzone"
        );
    }

    #[test]
    fn a_name_someone_has_gets_copy() {
        assert_eq!(
            unique_name("warpzone", "supersilvia Output 3", held(&["warpzone"])),
            "warpzone copy"
        );
    }

    #[test]
    fn copy_is_appended_until_the_name_is_free() {
        assert_eq!(
            unique_name(
                "warpzone",
                "supersilvia Output 3",
                held(&["warpzone", "warpzone copy", "warpzone copy copy"])
            ),
            "warpzone copy copy copy"
        );
    }

    /// A default name is a name like any other: typing another Output's is a clash.
    #[test]
    fn another_outputs_default_is_taken() {
        assert_eq!(
            unique_name(
                "supersilvia Output 7",
                "supersilvia Output 3",
                held(&["supersilvia Output 7"])
            ),
            "supersilvia Output 7 copy"
        );
    }

    #[test]
    fn empty_is_the_default() {
        assert_eq!(
            unique_name("", "supersilvia Output 3", held(&["warpzone"])),
            "supersilvia Output 3"
        );
        assert_eq!(
            unique_name("  ", "supersilvia Output 3", held(&[])),
            "supersilvia Output 3"
        );
    }

    /// Past the cap, the ` copy`s stop and the Output's own default is what is left.
    #[test]
    fn the_cap_reached_is_the_default() {
        let long = "x".repeat(NAME_BYTES - 2);
        assert_eq!(
            unique_name(&long, "supersilvia Output 3", held(&[&long])),
            "supersilvia Output 3"
        );
        let mut chain = vec!["w".to_string()];
        while chain.last().expect("one").len() + COPY.len() <= NAME_BYTES {
            let next = format!("{}{COPY}", chain.last().expect("one"));
            chain.push(next);
        }
        let names: Vec<&str> = chain.iter().map(String::as_str).collect();
        assert_eq!(
            unique_name("w", "supersilvia Output 3", held(&names)),
            "supersilvia Output 3",
            "every copy that fits is taken"
        );
    }

    /// A typed name longer than the cap is cut to it, on a character's boundary.
    #[test]
    fn a_long_name_is_cut_to_the_cap() {
        let long = "é".repeat(NAME_BYTES);
        let cut = unique_name(&long, "d", held(&[]));
        assert!(cut.len() <= NAME_BYTES && cut.len() >= NAME_BYTES - 1);
        assert!(long.starts_with(&cut));
    }

    fn outputs(n: usize) -> (crate::graph::Graph, Vec<crate::graph::NodeId>) {
        let mut g = crate::graph::Graph::new();
        let ids = (0..n)
            .map(|_| {
                crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO)
                    .expect("in the registry")
            })
            .collect();
        (g, ids)
    }

    fn name_of(g: &crate::graph::Graph, id: crate::graph::NodeId) -> String {
        send_name(id, g.get(id).expect("in the graph"))
    }

    fn set_name(g: &mut crate::graph::Graph, id: crate::graph::NodeId, name: &str) {
        g.get_mut(id)
            .expect("in the graph")
            .options
            .insert(SEND_NAME, name.to_string());
    }

    /// Typed into one Output: held against the others and the mix, and the default stored
    /// as nothing.
    #[test]
    fn a_typed_name_is_fitted_against_the_project() {
        let (mut g, ids) = outputs(2);
        set_name(&mut g, ids[0], "warpzone");
        assert_eq!(fit_name(&g, ids[1], "warpzone"), "warpzone copy");
        assert_eq!(fit_name(&g, ids[0], "warpzone"), "warpzone", "its own name");
        assert_eq!(fit_name(&g, ids[1], "WARPZONE"), "WARPZONE copy", "case");
        assert_eq!(
            fit_name(&g, ids[1], ""),
            "",
            "the default is stored as nothing"
        );
        assert_eq!(fit_name(&g, ids[1], &default_name(ids[1])), "");
        assert_eq!(fit_name(&g, ids[1], MIX_NAME), format!("{MIX_NAME} copy"));
    }

    /// Two Outputs arriving under one name — a paste, a file — leave the first as it was and
    /// give the later one the ` copy`.
    #[test]
    fn the_later_of_two_takes_the_copy() {
        let (mut g, ids) = outputs(3);
        set_name(&mut g, ids[0], "warpzone");
        set_name(&mut g, ids[1], "warpzone");
        assert!(
            !settle_names(&mut g, &[ids[2]]),
            "a default is already its own"
        );
        assert!(settle_names(&mut g, &ids));
        assert_eq!(name_of(&g, ids[0]), "warpzone");
        assert_eq!(name_of(&g, ids[1]), "warpzone copy");
        assert!(!settle_names(&mut g, &ids), "settled stays settled");

        // A paste of the first: the newcomer yields to the one already there.
        let (mut g, ids) = outputs(2);
        set_name(&mut g, ids[0], "warpzone");
        set_name(&mut g, ids[1], "warpzone");
        assert!(settle_names(&mut g, &[ids[0]]));
        assert_eq!(name_of(&g, ids[0]), "warpzone copy");
        assert_eq!(name_of(&g, ids[1]), "warpzone");
    }

    #[test]
    fn rubbish_resolution_is_none_not_a_panic() {
        assert_eq!(parse_resolution("banana"), None);
        assert_eq!(parse_resolution("1280x"), None);
        assert_eq!(parse_resolution(""), None);
    }
}
