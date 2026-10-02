// SPDX-License-Identifier: AGPL-3.0-or-later

//! The mixer: two decks, a fade, and eight ways to cross between them.
//!
//! It is silvia's `MainMixer`, and it is not a node. An Output claims a deck with `Show on A`
//! or `Show on B`; the mixer samples the two claimed Outputs' published textures and draws
//! the crossfade into one target, which is what the preview, the editor background and the
//! projector all show. **The mixer's program is the one program that never recompiles**: it
//! is linked once, it is written against two textures rather than any graph, and an edit,
//! an undo or a rebuild of either deck's graph never touches it. That is what lets a
//! performer build deck B while deck A is on air — a DJ cueing the next record on the deck
//! that is not playing. See [docs/rendering.md](../docs/rendering.md#the-mixer).
//!
//! Pure: what the mixer *is* lives here, in the project's tier of saved state, and
//! `render/mixer.rs` is what draws it.

use crate::graph::{Graph, NodeId};
use serde::{Deserialize, Serialize};

/// One of the two decks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    A,
    B,
}

/// How the crossfade gets from deck A to deck B. The index is what the shader branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Blend,
    HorizontalWipe,
    VerticalWipe,
    RadialWipe,
    DarkFirst,
    LightFirst,
    Checkerboard,
    HorizontalLines,
}

impl Method {
    /// Every method, in the order the shader numbers them.
    pub const ALL: [Method; 8] = [
        Method::Blend,
        Method::HorizontalWipe,
        Method::VerticalWipe,
        Method::RadialWipe,
        Method::DarkFirst,
        Method::LightFirst,
        Method::Checkerboard,
        Method::HorizontalLines,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Method::Blend => "Simple mix",
            Method::HorizontalWipe => "Horizontal wipe",
            Method::VerticalWipe => "Vertical wipe",
            Method::RadialWipe => "Radial wipe",
            Method::DarkFirst => "Dark fade first",
            Method::LightFirst => "Light fade first",
            Method::Checkerboard => "Checkerboard",
            Method::HorizontalLines => "Horizontal lines",
        }
    }

    /// The branch the shader takes: the method's position in [`Method::ALL`].
    pub fn index(self) -> i32 {
        Method::ALL
            .iter()
            .position(|m| *m == self)
            .map_or(0, |i| i as i32)
    }
}

/// How large the mix is drawn.
///
/// `Viewport` follows the editor's canvas, capped at 1080 rows, which is silvia's *Match
/// Viewport*; `Fixed` is a preset, for a projector whose shape is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Resolution {
    #[default]
    Viewport,
    Fixed(u32, u32),
}

impl Resolution {
    /// The presets offered beside *Match viewport*, the Output's own list.
    pub const PRESETS: [(u32, u32); 7] = [
        (1280, 720),
        (1920, 1080),
        (3440, 1440),
        (1024, 768),
        (1080, 1080),
        (720, 1280),
        (1080, 1920),
    ];

    /// The tallest a viewport-matched mix is drawn. A 4K editor window does not make the
    /// mix 4K; the projector is the size it is.
    pub const VIEWPORT_MAX_HEIGHT: u32 = 1080;

    pub fn label(self) -> String {
        match self {
            Resolution::Viewport => "Match viewport".to_string(),
            Resolution::Fixed(w, h) => format!("{}:{} ({w}x{h})", ratio(w, h).0, ratio(w, h).1),
        }
    }

    /// The size to draw at, given the editor's canvas in pixels.
    pub fn pixels(self, viewport: (u32, u32)) -> (u32, u32) {
        match self {
            Resolution::Fixed(w, h) => (w.max(1), h.max(1)),
            Resolution::Viewport => {
                let (vw, vh) = (viewport.0.max(1), viewport.1.max(1));
                let height = vh.min(Self::VIEWPORT_MAX_HEIGHT);
                // Rounded, not truncated, so a 16:9 canvas at 1080 rows is 1920 wide and
                // not 1919.
                let width = (f64::from(height) * f64::from(vw) / f64::from(vh)).round() as u32;
                (width.max(1), height)
            }
        }
    }
}

/// A resolution's aspect as small integers, for its label: `16:9`, `21:9`, `4:3`.
fn ratio(w: u32, h: u32) -> (u32, u32) {
    // 3440x1440 is 43:18 exactly, which nobody calls it; the labels are the names the
    // presets are sold under.
    if (w, h) == (3440, 1440) {
        return (21, 9);
    }
    let g = gcd(w, h).max(1);
    (w / g, h / g)
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

impl From<Resolution> for String {
    fn from(r: Resolution) -> Self {
        match r {
            Resolution::Viewport => "viewport".to_string(),
            Resolution::Fixed(w, h) => format!("{w}x{h}"),
        }
    }
}

impl TryFrom<String> for Resolution {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        if s == "viewport" {
            return Ok(Resolution::Viewport);
        }
        crate::nodes::output::parse_resolution(&s)
            .filter(|(w, h)| *w > 0 && *h > 0)
            .map(|(w, h)| Resolution::Fixed(w, h))
            .ok_or_else(|| format!("{s:?} is neither \"viewport\" nor WIDTHxHEIGHT"))
    }
}

/// Which of the mixer's MIDI-bound controls a hand moved since the editor last told the
/// synth: what lets a hand's move win over a knob's or a note's write the editor had not seen,
/// even where the hand put the control back where the write found it — which, for a press,
/// is every time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hands {
    pub balance: bool,
    pub blackout: bool,
    pub freeze: bool,
}

/// What the mixer is set to. Project data: the rig, not any one workspace.
///
/// **Never an edit.** Claiming a deck, moving the fade and picking a method are playing the
/// instrument, and none of them enters the undo history — which is also what keeps undo,
/// which rebuilds every Output, from ever touching what is on air.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mixer {
    /// The Output on deck A, if one has claimed it.
    pub a: Option<NodeId>,
    /// The Output on deck B.
    pub b: Option<NodeId>,
    /// −1 is deck A alone, +1 is deck B alone. silvia's fade, curved through
    /// [`mix_amount`] so the ends are hard.
    pub balance: f32,
    pub method: Method,
    pub resolution: Resolution,
    /// Paint the mix behind the canvas, nodes and cables floating on the show.
    ///
    /// **Off at every launch, and never written to the project.** It is a way of *looking* at
    /// the patch you are editing right now, not a property of the show: opening a project to
    /// find the canvas already covered by the mix is a surprise, and one that hides the very
    /// graph the project was opened to work on. `H` is the same idea at full strength and is
    /// also not remembered.
    #[serde(skip)]
    pub background: bool,
    /// **Blackout**: the mix is black until it is pressed again, wherever the mix is shown —
    /// the panel, the canvas behind the graph, a picture window, NDI and Syphon. The decks go
    /// on drawing under it, so letting go shows the mix as it stands and not as it was.
    /// Playing, not editing, and never saved.
    #[serde(skip)]
    pub blackout: bool,
    /// **Freeze**: the mix's last frame is shown again and again until it is pressed again,
    /// wherever the mix is shown, while the decks go on underneath. Blackout outranks it:
    /// both held is black, and letting Blackout go shows the frozen frame. Never saved.
    #[serde(skip)]
    pub freeze: bool,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            a: None,
            b: None,
            balance: -1.0,
            method: Method::Blend,
            resolution: Resolution::Viewport,
            background: false,
            blackout: false,
            freeze: false,
        }
    }
}

impl Mixer {
    /// Put an Output on a deck. The one that was there is simply replaced; an Output may be
    /// on both decks at once, as in silvia.
    pub fn claim(&mut self, channel: Channel, node: NodeId) {
        match channel {
            Channel::A => self.a = Some(node),
            Channel::B => self.b = Some(node),
        }
    }

    /// Take an Output off whichever decks it is on.
    pub fn release(&mut self, node: NodeId) {
        if self.a == Some(node) {
            self.a = None;
        }
        if self.b == Some(node) {
            self.b = None;
        }
    }

    pub fn on(&self, channel: Channel) -> Option<NodeId> {
        match channel {
            Channel::A => self.a,
            Channel::B => self.b,
        }
    }

    /// Which decks this Output is on: `(A, B)`.
    pub fn decks_of(&self, node: NodeId) -> (bool, bool) {
        (self.a == Some(node), self.b == Some(node))
    }

    /// Every Output on a deck, each once.
    pub fn claimed(&self) -> impl Iterator<Item = NodeId> {
        self.a
            .into_iter()
            .chain(self.b.filter(|b| Some(*b) != self.a))
    }

    pub fn set_balance(&mut self, balance: f32) {
        self.balance = if balance.is_finite() {
            balance.clamp(-1.0, 1.0)
        } else {
            -1.0
        };
    }

    /// Drop a deck whose Output is gone, or whose id belongs to something that is not an
    /// Output any more. Run against the graph on both sides of the file and after any graph
    /// replacement, as the session is.
    pub fn reconcile(&mut self, graph: &Graph) {
        let is_output = |id: NodeId| graph.get(id).is_some_and(|n| n.def.is_output);
        if let Some(a) = self.a
            && !is_output(a)
        {
            self.a = None;
        }
        if let Some(b) = self.b
            && !is_output(b)
        {
            self.b = None;
        }
        self.set_balance(self.balance);
    }
}

/// How far short of the pole the fade stops. `tan` at exactly ±π/2 in 32-bit float lands
/// on whichever side of the pole the rounding put it, so hard B can come out as hard A;
/// 0.999 of the way is already hundreds of times past the unit range, so the ends are hard
/// and the sign is never in doubt.
const FADE_LIMIT: f32 = 0.999;

/// silvia's fade curve: the control runs −1 to +1 and the ends are hard A and hard B.
///
/// The shader computes the same curve; this is for the tests and the panel.
pub fn mix_amount(balance: f32) -> f32 {
    if balance.is_nan() {
        // A NaN fade would be a black frame; deck A is the answer instead.
        return 0.0;
    }
    let curved =
        0.5 + 0.5 * (balance.clamp(-FADE_LIMIT, FADE_LIMIT) * std::f32::consts::FRAC_PI_2).tan();
    curved.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_is_hard_at_both_ends_and_even_in_the_middle() {
        assert_eq!(mix_amount(-1.0), 0.0);
        assert_eq!(mix_amount(1.0), 1.0);
        assert!((mix_amount(0.0) - 0.5).abs() < 1e-6);
        assert_eq!(mix_amount(-7.0), 0.0, "clamped, not wrapped");
        assert_eq!(
            mix_amount(f32::NAN),
            0.0,
            "a NaN fade is deck A, not a black frame"
        );
    }

    #[test]
    fn a_resolution_round_trips_as_a_string() {
        for r in [
            Resolution::Viewport,
            Resolution::Fixed(1280, 720),
            Resolution::Fixed(720, 1280),
        ] {
            let s: String = r.into();
            assert_eq!(Resolution::try_from(s).unwrap(), r);
        }
        assert!(Resolution::try_from("banana".to_string()).is_err());
        assert!(Resolution::try_from("0x0".to_string()).is_err());
    }

    #[test]
    fn a_viewport_resolution_keeps_the_canvas_aspect_under_the_cap() {
        assert_eq!(Resolution::Viewport.pixels((1600, 900)), (1600, 900));
        assert_eq!(Resolution::Viewport.pixels((3840, 2160)), (1920, 1080));
        assert_eq!(Resolution::Viewport.pixels((0, 0)), (1, 1), "never zero");
        assert_eq!(
            Resolution::Fixed(1080, 1920).pixels((1600, 900)),
            (1080, 1920)
        );
    }

    #[test]
    fn presets_are_labeled_by_their_shape() {
        assert_eq!(Resolution::Fixed(1920, 1080).label(), "16:9 (1920x1080)");
        assert_eq!(Resolution::Fixed(3440, 1440).label(), "21:9 (3440x1440)");
        assert_eq!(Resolution::Fixed(1080, 1080).label(), "1:1 (1080x1080)");
    }

    #[test]
    fn the_default_is_hard_a_matching_the_viewport_and_off_the_background() {
        let m = Mixer::default();
        assert_eq!(m.balance, -1.0);
        assert_eq!(m.method, Method::Blend);
        assert_eq!(m.resolution, Resolution::Viewport);
        assert!(!m.background, "the canvas is not covered until a hand asks");
        assert!(!m.blackout && !m.freeze, "the show is not held at launch");
        assert_eq!(m.claimed().count(), 0);
    }

    /// Projecting to the background is about the editor in front of you this minute, so a
    /// project that was saved with it on opens with it off.
    #[test]
    fn projecting_to_the_background_is_not_saved() {
        let m = Mixer {
            background: true,
            blackout: true,
            freeze: true,
            ..Mixer::default()
        };
        let json = serde_json::to_string(&m).expect("serializes");
        for field in ["background", "blackout", "freeze"] {
            assert!(!json.contains(field), "{field} is not written: {json}");
        }
        let back: Mixer = serde_json::from_str(&json).expect("reads back");
        assert!(!back.background, "and it comes back off");
        assert!(!back.blackout && !back.freeze, "and so do the holds");
    }

    #[test]
    fn one_output_on_both_decks_is_claimed_once() {
        let mut m = Mixer::default();
        m.claim(Channel::A, NodeId(3));
        m.claim(Channel::B, NodeId(3));
        assert_eq!(m.claimed().collect::<Vec<_>>(), vec![NodeId(3)]);
        assert_eq!(m.decks_of(NodeId(3)), (true, true));
        m.release(NodeId(3));
        assert_eq!(m.claimed().count(), 0);
    }

    #[test]
    fn reconcile_drops_a_deck_whose_output_is_gone() {
        let mut g = Graph::new();
        let out = crate::nodes::add_to_graph(&mut g, "output", emath::Pos2::ZERO).unwrap();
        let not_an_output =
            crate::nodes::add_to_graph(&mut g, "checkerboard", emath::Pos2::ZERO).unwrap();
        let mut m = Mixer::default();
        m.claim(Channel::A, out);
        m.claim(Channel::B, not_an_output);
        m.balance = 4.0;
        m.reconcile(&g);
        assert_eq!(m.a, Some(out));
        assert_eq!(m.b, None, "a checkerboard cannot be on a deck");
        assert_eq!(m.balance, 1.0, "a fade outside its range is brought back");
        g.remove_node(out);
        m.reconcile(&g);
        assert_eq!(m.a, None);
    }
}
