// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which of a solid's axes a field cabled into one of its numbers is read along.
//!
//! A field is a picture: a value for each point of a plane, across and up. A solid asks for a
//! value at a point in space, so one of its number inputs fed by a field reads the field at two
//! of the point's three coordinates, and the third is the way the picture runs through the
//! solid unchanged, a slide through a projector (`CompileContext::input_at_point`). [`Axes`]
//! says which two, in which order, and whether either runs backwards: the field's across is
//! read along one of X, Y and Z, its up along another, and each may be negated. Front, the
//! default, is across along X and up along Y, as it always was.
//!
//! Twenty-four arrangements, every way a picture can be laid on one of a box's three planes,
//! turned and mirrored. Kept per input on the node (`Node::axes`), only where not Front.

use std::fmt;

/// One of a field's two axes: the solid's axis it is read along, 0 for X, 1 for Y, 2 for Z,
/// and whether it runs backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Axis {
    pub of: u8,
    pub flipped: bool,
}

impl Axis {
    pub const fn new(of: u8, flipped: bool) -> Self {
        Self { of, flipped }
    }

    /// The axis's letter.
    pub fn letter(self) -> char {
        ['X', 'Y', 'Z'][usize::from(self.of.min(2))]
    }

    /// The coordinate of the WGSL point `q` it reads, negated where flipped.
    fn read(self, q: &str) -> String {
        let c = ['x', 'y', 'z'][usize::from(self.of.min(2))];
        if self.flipped {
            format!("-{q}.{c}")
        } else {
            format!("{q}.{c}")
        }
    }
}

/// A field's across and up, each read along one of the solid's axes, never the same one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Axes {
    pub across: Axis,
    pub up: Axis,
}

/// The plane a field is laid on: the one its two axes span, named by the side it is seen
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plane {
    /// X and Y, seen from the front.
    Front,
    /// X and Z, seen from above.
    Top,
    /// Z and Y, seen from the side.
    Side,
}

impl Axes {
    /// Across along X, up along Y: what a field on a solid has always been.
    pub const FRONT: Self = Self {
        across: Axis::new(0, false),
        up: Axis::new(1, false),
    };
    /// Across along X, up along Z.
    pub const TOP: Self = Self {
        across: Axis::new(0, false),
        up: Axis::new(2, false),
    };
    /// Across along Z, up along Y.
    pub const SIDE: Self = Self {
        across: Axis::new(2, false),
        up: Axis::new(1, false),
    };

    /// Two different axes, each X, Y or Z.
    pub fn is_valid(self) -> bool {
        self.across.of < 3 && self.up.of < 3 && self.across.of != self.up.of
    }

    /// The plane it lays the field on.
    pub fn plane(self) -> Plane {
        match 3 - self.across.of.min(2) - self.up.of.min(2) {
            0 => Plane::Side,
            1 => Plane::Top,
            _ => Plane::Front,
        }
    }

    /// The `vec2f` a field is read at for the WGSL point `q`.
    pub fn read(self, q: &str) -> String {
        if self == Self::FRONT {
            return format!("{q}.xy");
        }
        format!("vec2f({}, {})", self.across.read(q), self.up.read(q))
    }

    /// The solid's axis neither reads along: the one a picture runs through, a bend's axis,
    /// a twist's.
    pub fn third(self) -> u8 {
        3 - self.across.of.min(2) - self.up.of.min(2)
    }

    /// The WGSL point `p` in this frame: across, up, and the third axis, as a `vec3f`.
    pub fn into_frame(self, p: &str) -> String {
        let c = ['x', 'y', 'z'][usize::from(self.third().min(2))];
        format!(
            "vec3f({}, {}, {p}.{c})",
            self.across.read(p),
            self.up.read(p)
        )
    }

    /// The WGSL `vec3f` `v`, in this frame, back in the solid's own axes: [`Self::into_frame`]
    /// undone.
    pub fn out_of_frame(self, v: &str) -> String {
        let part = |i: u8| {
            let (from, flipped) = if i == self.across.of {
                ('x', self.across.flipped)
            } else if i == self.up.of {
                ('y', self.up.flipped)
            } else {
                ('z', false)
            };
            if flipped {
                format!("-{v}.{from}")
            } else {
                format!("{v}.{from}")
            }
        };
        format!("vec3f({}, {}, {})", part(0), part(1), part(2))
    }

    /// The WGSL `vec3f` of sizes `e`, across, up and through, laid on the solid's own axes:
    /// for a box symmetric about the origin, whose size has no sign.
    pub fn sizes_out(self, e: &str) -> String {
        let part = |i: u8| {
            if i == self.across.of {
                format!("{e}.x")
            } else if i == self.up.of {
                format!("{e}.y")
            } else {
                format!("{e}.z")
            }
        };
        format!("vec3f({}, {}, {})", part(0), part(1), part(2))
    }

    /// How it reads as text: each axis's letter, a minus before one that runs backwards —
    /// `XY`, `X−Z`, `−ZY`.
    pub fn label(self) -> String {
        let one = |a: Axis| {
            if a.flipped {
                format!("\u{2212}{}", a.letter())
            } else {
                a.letter().to_string()
            }
        };
        format!("{}{}", one(self.across), one(self.up))
    }

    /// Read back from [`fmt::Display`]'s form, `x,-z`: `None` for anything else or for one
    /// axis twice.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split(',').map(str::trim);
        let one = |s: Option<&str>| -> Option<Axis> {
            let s = s?;
            let (flipped, letter) = match s.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, s),
            };
            let of = match letter {
                "x" | "X" => 0,
                "y" | "Y" => 1,
                "z" | "Z" => 2,
                _ => return None,
            };
            Some(Axis::new(of, flipped))
        };
        let axes = Self {
            across: one(parts.next())?,
            up: one(parts.next())?,
        };
        (parts.next().is_none() && axes.is_valid()).then_some(axes)
    }
}

impl Default for Axes {
    fn default() -> Self {
        Self::FRONT
    }
}

/// The file's form: `x,y`, `x,-z`.
impl fmt::Display for Axes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let one = |a: Axis| {
            let c = ['x', 'y', 'z'][usize::from(a.of.min(2))];
            if a.flipped {
                format!("-{c}")
            } else {
                c.to_string()
            }
        };
        write!(f, "{},{}", one(self.across), one(self.up))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every one of the twenty-four reads back from its file form, and nothing else does.
    #[test]
    fn every_arrangement_survives_the_file() {
        let mut seen = 0;
        for a in 0..3 {
            for b in 0..3 {
                for fa in [false, true] {
                    for fb in [false, true] {
                        let axes = Axes {
                            across: Axis::new(a, fa),
                            up: Axis::new(b, fb),
                        };
                        if a == b {
                            assert!(!axes.is_valid());
                            continue;
                        }
                        seen += 1;
                        assert_eq!(Axes::parse(&axes.to_string()), Some(axes), "{axes}");
                    }
                }
            }
        }
        assert_eq!(seen, 24);
        for bad in ["", "x", "x,x", "x,y,z", "w,y", "--x,y"] {
            assert_eq!(Axes::parse(bad), None, "{bad:?}");
        }
    }

    /// Into a frame and back out is where it started, for every arrangement: checked on the
    /// three unit axes by evaluating the expressions' sign-and-letter form by hand.
    #[test]
    fn a_frame_undoes_itself() {
        let eval = |expr: &str, v: [f32; 3]| -> [f32; 3] {
            let inner = expr.trim_start_matches("vec3f(").trim_end_matches(')');
            let mut out = [0.0; 3];
            for (i, part) in inner.split(", ").enumerate() {
                let (sign, rest) = part.strip_prefix('-').map_or((1.0, part), |r| (-1.0, r));
                let c = rest.rsplit('.').next().unwrap();
                let k = match c {
                    "x" => 0,
                    "y" => 1,
                    _ => 2,
                };
                out[i] = sign * v[k];
            }
            out
        };
        for a in 0..3 {
            for b in 0..3 {
                if a == b {
                    continue;
                }
                for fa in [false, true] {
                    for fb in [false, true] {
                        let axes = Axes {
                            across: Axis::new(a, fa),
                            up: Axis::new(b, fb),
                        };
                        for v in [[1.0, 2.0, 3.0], [-4.0, 0.5, 7.0]] {
                            let f = eval(&axes.into_frame("p"), v);
                            assert_eq!(eval(&axes.out_of_frame("v"), f), v, "{axes}");
                        }
                    }
                }
            }
        }
    }

    /// Front reads the point's front view as a field on a solid always has; the rest swizzle.
    #[test]
    fn front_is_the_front_view_and_the_rest_swizzle() {
        assert_eq!(Axes::FRONT.read("q"), "q.xy");
        let axes = Axes::parse("-z,x").unwrap();
        assert_eq!(axes.read("q"), "vec2f(-q.z, q.x)");
        assert_eq!(axes.label(), "\u{2212}ZX");
        assert_eq!(axes.plane(), Plane::Top);
        assert_eq!(Axes::SIDE.plane(), Plane::Side);
        assert_eq!(Axes::FRONT.plane(), Plane::Front);
    }
}
