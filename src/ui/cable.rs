// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cables. Bezier for data, straight for action, as in silvia.

use crate::graph::PortType;
use eframe::egui::{Color32, Pos2, Stroke, epaint::CubicBezierShape, pos2};

/// The four control points of a data cable.
///
/// A cable always leaves an output going right and enters an input going right, however the
/// two are arranged. When the target is to the *left* of the source — a feedback loop — that
/// alone is not enough: the control points pull inward and the curve cuts back through the
/// space between the ports, which is exactly where both node bodies are. Such a cable also
/// bows vertically, so it leaves, goes around, and comes back.
pub fn bezier_points(from: Pos2, to: Pos2, clear_below: f32, droop: bool) -> [Pos2; 4] {
    let dx = to.x - from.x;
    if dx >= 0.0 {
        let s = (dx * 0.5).clamp(24.0, 140.0);
        // silvia's `droopyCables`: both control points drop by a sag that grows with the
        // span and stops at eighty — `Math.min(80, 15 + dx * 0.15)`, its number, so a short
        // cable still dips a little and a long one does not fall off the workspace.
        //
        // Only the forward case. A backward cable already bows downward to get round the
        // node bodies, and sagging it further would be two reasons for one curve.
        let sag = if droop { sag(dx) } else { 0.0 };
        return [
            from,
            pos2(from.x + s, from.y + sag),
            pos2(to.x - s, to.y + sag),
            to,
        ];
    }

    // Backward. Reach further sideways than a forward cable would, so the curve clears the
    // node edges, and drop both control points below the ports so it passes under the bodies
    // rather than through them. Downward always, because a predictable route is easier to
    // follow across a dense graph than one that picks a side per cable.
    //
    // `clear_below` is the bottom of the nodes at each end. Bowing by horizontal distance
    // alone is not enough: on a tall node — an Output, with its own render on it — a curve
    // sized from dx dips straight into the body it is trying to avoid.
    let back = -dx;
    let reach = (back * 0.4).clamp(70.0, 220.0);
    let deepest = from.y.max(to.y);
    // A cubic reaches roughly three quarters of the way to its control points, so aim past
    // the clearance rather than at it.
    let needed = ((clear_below - deepest) * 1.35).max(0.0);
    let bow = (back * 0.35).clamp(60.0, 200.0).max(needed);
    [
        from,
        pos2(from.x + reach, from.y + bow),
        pos2(to.x - reach, to.y + bow),
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
    pub fn new(from: Pos2, to: Pos2, ty: PortType, clear_below: f32, droop: bool) -> Self {
        if ty == PortType::Action {
            // Straight for an action, as in silvia — a dashed line means *event*, and a
            // sagging one would read as the same cable the data ports use.
            Self::Straight([from, to])
        } else {
            Self::Bezier(bezier_points(from, to, clear_below, droop))
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
    /// Not the midpoint of the endpoints: a backward cable bows below both nodes, and the
    /// straight-line midpoint would land inside a node body rather than on the cable.
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

    /// The tests below are about curve shape, not clearance; give them a clearance of zero.
    fn distance_to_x(from: Pos2, to: Pos2, ty: PortType, point: Pos2) -> f32 {
        Curve::new(from, to, ty, 0.0, false).distance_to(point)
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

    /// A feedback cable runs back between two ports, which is where both node bodies are.
    #[test]
    fn a_backward_cable_routes_below_both_ports_instead_of_between_them() {
        // Output on the right, input on the left: a loop back to an earlier node.
        let (from, to) = (pos2(400.0, 100.0), pos2(100.0, 100.0));
        let p = bezier_points(from, to, 0.0, false);

        assert_eq!(p[0], from, "still starts at the port");
        assert_eq!(p[3], to, "still ends at the port");
        assert!(
            p[1].y > from.y && p[2].y > to.y,
            "both control points bow downward, away from the bodies: {p:?}"
        );
        assert!(
            p[1].x > from.x && p[2].x < to.x,
            "and reach outward, so the curve clears the node edges: {p:?}"
        );

        // The midpoint of the curve is well below the line joining the ports.
        let mid = cubic_at(&p, 0.5);
        assert!(
            mid.y > from.y + 40.0,
            "curve should pass under the nodes, midpoint at {mid:?}"
        );
    }

    /// A forward cable is the common case and must be untouched by the above.
    #[test]
    fn a_forward_cable_stays_level_between_level_ports() {
        let (from, to) = (pos2(100.0, 100.0), pos2(400.0, 100.0));
        let p = bezier_points(from, to, 0.0, false);
        assert!(
            p.iter().all(|c| (c.y - 100.0).abs() < f32::EPSILON),
            "{p:?}"
        );
        assert!(p[1].x > from.x && p[2].x < to.x, "{p:?}");
    }

    /// Click-to-delete samples the same curve, so it follows the cable around the loop.
    #[test]
    fn clicking_finds_a_backward_cable_where_it_is_actually_drawn() {
        let (from, to) = (pos2(400.0, 100.0), pos2(100.0, 100.0));
        let on_curve = cubic_at(&bezier_points(from, to, 0.0, false), 0.5);
        assert!(distance_to_x(from, to, PortType::VaryingColor, on_curve) < 1.0);
        // And not on the straight line between the ports, where it no longer runs.
        assert!(distance_to_x(from, to, PortType::VaryingColor, pos2(250.0, 100.0)) > 30.0);
    }

    /// Droop drops the middle of a forward cable below its ports and leaves the ends alone,
    /// which is the whole of what silvia's `droopyCables` does.
    #[test]
    fn a_drooping_cable_sags_in_the_middle_and_not_at_its_ends() {
        let (from, to) = (pos2(0.0, 100.0), pos2(300.0, 100.0));
        let flat = cubic_at(&bezier_points(from, to, 0.0, false), 0.5);
        let sagged = cubic_at(&bezier_points(from, to, 0.0, true), 0.5);
        assert!(
            sagged.y > flat.y,
            "a drooping cable should hang below a flat one: {sagged:?} against {flat:?}",
        );
        let ends = bezier_points(from, to, 0.0, true);
        assert_eq!(ends[0], from, "the ends stay on their ports");
        assert_eq!(ends[3], to);
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
