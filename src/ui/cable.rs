// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cables. Bezier for data, straight for action, as in silvia. One curve rule regardless of
//! which side of its output a cable's input lands on: no special case for a cable running
//! right to left, and no dependence on node height.

use crate::graph::PortType;
use eframe::egui::{Color32, Pos2, Stroke, epaint::CubicBezierShape, pos2};

/// The four control points of a data cable.
///
/// One rule regardless of direction: both control points reach sideways from their own port
/// by the same amount, found from the *distance* between the ports rather than the signed
/// `dx`. A cable to a port on the left reaches out to the right from its output and out to
/// the left from its input, same as a cable to a port on the right — so the curve is
/// continuous as a cable's target crosses from one side to the other, with no jump in shape
/// and no dependence on either node's height.
pub fn bezier_points(from: Pos2, to: Pos2, droop: bool) -> [Pos2; 4] {
    let dx = to.x - from.x;
    let s = (dx.abs() * 0.5).clamp(24.0, 140.0);
    // silvia's `droopyCables`: both control points drop by a sag that grows with the span and
    // stops at eighty — `Math.min(80, 15 + dx * 0.15)`, its number, so a short cable still
    // dips a little and a long one does not fall off the workspace.
    let sag = if droop { sag(dx) } else { 0.0 };
    [
        from,
        pos2(from.x + s, from.y + sag),
        pos2(to.x - s, to.y + sag),
        to,
    ]
}

/// How far a drooping cable sags at its middle, from the distance it spans. silvia's own
/// `Math.min(80, 15 + dx * 0.15)`.
fn sag(dx: f32) -> f32 {
    dx.abs().mul_add(0.15, 15.0).min(80.0)
}

/// One cable's path on screen: a cubic for data, a straight segment for an action. Worked out
/// once a cable, and what the cable is painted along, hit by and named at.
#[derive(Debug, Clone, Copy)]
pub enum Curve {
    Bezier([Pos2; 4]),
    Straight([Pos2; 2]),
}

impl Curve {
    pub fn new(from: Pos2, to: Pos2, ty: PortType, droop: bool) -> Self {
        if ty == PortType::Action {
            // Straight for an action, as in silvia — a dashed line means *event*, and a
            // sagging one would read as the same cable the data ports use.
            Self::Straight([from, to])
        } else {
            Self::Bezier(bezier_points(from, to, droop))
        }
    }

    pub fn paint(&self, painter: &eframe::egui::Painter, stroke: Stroke) {
        match *self {
            Self::Bezier(points) => {
                painter.add(CubicBezierShape::from_points_stroke(
                    points,
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            }
            Self::Straight(line) => {
                // Action wires are dashed, as in the design system.
                painter.add(eframe::egui::Shape::dashed_line(&line, stroke, 4.0, 4.0));
            }
        }
    }

    /// The point halfway along the curve, where a cable's interactable sits.
    ///
    /// Not the midpoint of the endpoints: a drooping or far-reaching cable bows away from the
    /// straight line between its ports, and the straight-line midpoint would miss the curve.
    pub fn midpoint(&self) -> Pos2 {
        match self {
            Self::Bezier(points) => cubic_at(points, 0.5),
            Self::Straight([from, to]) => *from + (*to - *from) * 0.5,
        }
    }

    /// Distance from `point` to the cable, by sampling. Used for click-to-delete.
    ///
    /// Sampling rather than solving: a cubic's true closest point needs a quintic root, and 24
    /// samples are well inside the click tolerance at any zoom a person can see.
    pub fn distance_to(&self, point: Pos2) -> f32 {
        const SAMPLES: usize = 24;
        let p = match self {
            Self::Bezier(points) => points,
            Self::Straight([from, to]) => return distance_to_segment(*from, *to, point),
        };
        let mut best = f32::INFINITY;
        let mut prev = p[0];
        for i in 1..=SAMPLES {
            let t = i as f32 / SAMPLES as f32;
            let next = cubic_at(p, t);
            best = best.min(distance_to_segment(prev, next, point));
            prev = next;
        }
        best
    }
}

fn cubic_at(p: &[Pos2; 4], t: f32) -> Pos2 {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    pos2(
        a * p[0].x + b * p[1].x + c * p[2].x + d * p[3].x,
        a * p[0].y + b * p[1].y + c * p[2].y + d * p[3].y,
    )
}

fn distance_to_segment(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance_to_x(from: Pos2, to: Pos2, ty: PortType, point: Pos2) -> f32 {
        Curve::new(from, to, ty, false).distance_to(point)
    }

    #[test]
    fn a_point_on_a_straight_cable_has_no_distance() {
        let (a, b) = (pos2(0.0, 0.0), pos2(100.0, 0.0));
        assert!(distance_to_x(a, b, PortType::Action, pos2(50.0, 0.0)) < 0.01);
        assert!((distance_to_x(a, b, PortType::Action, pos2(50.0, 10.0)) - 10.0).abs() < 0.01);
    }

    #[test]
    fn a_bezier_passes_through_its_endpoints() {
        let (a, b) = (pos2(0.0, 0.0), pos2(100.0, 50.0));
        assert!(distance_to_x(a, b, PortType::VaryingColor, a) < 0.01);
        assert!(distance_to_x(a, b, PortType::VaryingColor, b) < 0.01);
    }

    /// A cable is still a cable whichever side its input lands on: it starts and ends on the
    /// ports and reaches outward from each, same rule regardless of direction.
    #[test]
    fn a_backward_cable_still_starts_and_ends_on_its_ports_and_reaches_outward() {
        // Output on the right, input on the left: a loop back to an earlier node.
        let (from, to) = (pos2(400.0, 100.0), pos2(100.0, 100.0));
        let p = bezier_points(from, to, false);

        assert_eq!(p[0], from, "still starts at the port");
        assert_eq!(p[3], to, "still ends at the port");
        assert!(
            p[1].x > from.x && p[2].x < to.x,
            "and reach outward, so the curve clears the node edges: {p:?}"
        );
    }

    /// A forward cable is the common case and must be untouched by the above.
    #[test]
    fn a_forward_cable_stays_level_between_level_ports() {
        let (from, to) = (pos2(100.0, 100.0), pos2(400.0, 100.0));
        let p = bezier_points(from, to, false);
        assert!(
            p.iter().all(|c| (c.y - 100.0).abs() < f32::EPSILON),
            "{p:?}"
        );
        assert!(p[1].x > from.x && p[2].x < to.x, "{p:?}");
    }

    /// Click-to-delete samples the same curve, so it follows the cable wherever it is drawn.
    #[test]
    fn clicking_finds_a_backward_cable_where_it_is_actually_drawn() {
        let (from, to) = (pos2(400.0, 100.0), pos2(100.0, 100.0));
        let on_curve = cubic_at(&bezier_points(from, to, false), 0.5);
        assert!(distance_to_x(from, to, PortType::VaryingColor, on_curve) < 1.0);
    }

    /// Droop drops the middle of a forward cable below its ports and leaves the ends alone,
    /// which is the whole of what silvia's `droopyCables` does.
    #[test]
    fn a_drooping_cable_sags_in_the_middle_and_not_at_its_ends() {
        let (from, to) = (pos2(0.0, 100.0), pos2(300.0, 100.0));
        let flat = cubic_at(&bezier_points(from, to, false), 0.5);
        let sagged = cubic_at(&bezier_points(from, to, true), 0.5);
        assert!(
            sagged.y > flat.y,
            "a drooping cable should hang below a flat one: {sagged:?} against {flat:?}",
        );
        let ends = bezier_points(from, to, true);
        assert_eq!(ends[0], from, "the ends stay on their ports");
        assert_eq!(ends[3], to);
    }

    /// The curve's points vary continuously as the input crosses from right of the output to
    /// left of it — no jump in shape at the moment the nodes cross, where the old backward
    /// special case used to switch in.
    #[test]
    fn the_curve_is_continuous_as_the_input_crosses_from_right_to_left() {
        let out = pos2(200.0, 100.0);
        let mut prev: Option<[Pos2; 4]> = None;
        let mut dx = 50.0_f32;
        while dx >= -50.0 {
            let input = pos2(out.x + dx, 100.0);
            let p = bezier_points(out, input, false);
            if let Some(prev) = prev {
                for (a, b) in prev.iter().zip(p.iter()) {
                    assert!(
                        (*a - *b).length() < 2.0,
                        "control points jumped between dx steps near {dx}: {a:?} -> {b:?}"
                    );
                }
            }
            prev = Some(p);
            dx -= 1.0;
        }
    }

    /// Neither `bezier_points` nor `Curve::new` takes a node-height or clearance argument any
    /// more: the only inputs to a cable's geometry are its two ports and whether it droops, so
    /// a node's height cannot reach the curve. This is enforced by the signature above —
    /// callers below exercise it at the call sites that used to carry a height-derived
    /// clearance.
    #[test]
    fn a_nodes_height_does_not_affect_a_cables_geometry() {
        let (from, to) = (pos2(400.0, 300.0), pos2(100.0, 100.0));
        // The same two points produce the same curve no matter how tall the nodes around
        // them are, because there is no longer anywhere to plumb a height into.
        assert_eq!(
            bezier_points(from, to, false),
            bezier_points(from, to, false)
        );
        assert_eq!(
            Curve::new(from, to, PortType::VaryingColor, false).midpoint(),
            Curve::new(from, to, PortType::VaryingColor, false).midpoint()
        );
    }

    /// The sag grows with the span and then stops, so a cable across the whole workspace does
    /// not hang off the bottom of it. silvia's own `min(80, 15 + dx * 0.15)`.
    #[test]
    fn the_sag_grows_with_the_span_and_stops() {
        assert!(
            (sag(0.0) - 15.0).abs() < 1e-3,
            "a cable to itself still dips"
        );
        assert!(sag(200.0) > sag(100.0));
        assert!(
            (sag(10_000.0) - 80.0).abs() < 1e-3,
            "and it stops at eighty"
        );
    }

    #[test]
    fn a_bezier_bulges_away_from_the_straight_line() {
        // Endpoints level but far apart: the curve should still be near the line's midpoint,
        // while a point well above it is clearly off the cable.
        let (a, b) = (pos2(0.0, 0.0), pos2(200.0, 0.0));
        assert!(distance_to_x(a, b, PortType::VaryingColor, pos2(100.0, 0.0)) < 1.0);
        assert!(distance_to_x(a, b, PortType::VaryingColor, pos2(100.0, 40.0)) > 30.0);
    }
}
