// SPDX-License-Identifier: AGPL-3.0-or-later

//! An audio node's scope, as a region: a spectrum with a two-axis handle per band, then a
//! meter each.
//!
//! The drawing is `ui::scope`, which the Main Input panel shares — the analyzer is one widget
//! in two places, and only the region around it is the node's. The band handles register their
//! own interacts and so take the pointer where they are; everywhere else in the band the drag
//! reaches the node's body, which is what lets a hand carry a node by its scope.

use super::{RegionDef, RegionEvent, RegionUi};
use crate::graph::Node;
use crate::ui::canvas;

/// An audio source's scope, behind a heading a hand closes.
pub const SCOPE: RegionDef = RegionDef {
    size,
    show,
    width: Some(canvas::SCOPE_NODE_WIDTH),
    ..RegionDef::EMPTY
};

/// The three band meters on their own, read-only: what a `maininput` node draws.
///
/// silvia draws three level bars on its Main Input node with the trigger handle on each, and
/// the reason is the whole argument for having them there: a level is set while the band it
/// measures is being watched, and a meter on a panel at the far left of the window is a trip
/// away from the row being cabled. So the bars come to the node.
///
/// **No spectrum and no handles.** They are about tuning, and this node's tuning is the
/// panel's — there is one capture and one analysis, so a per-node copy would mean whichever
/// node ticked last decided what all of them saw. The threshold square is drawn where the
/// panel has it and does not offer to move it, which is [`ui::scope::Hands::Off`].
///
/// **No heading.** A region that can close says so with a triangle; these are the node, the
/// way an Output's render is, and silvia's node has no tick for them either.
pub const METERS: RegionDef = RegionDef {
    size: meters_size,
    show: meters_show,
    width: Some(canvas::SCOPE_NODE_WIDTH),
    ..RegionDef::EMPTY
};

fn meters_size(_node: &Node) -> f32 {
    canvas::METERS_HEIGHT
}

fn meters_show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(scope) = r.live.scope else {
        return Vec::new();
    };
    let owner = crate::ui::scope::Owner::node(r.id, r.node.def.slug);
    crate::ui::scope::meters(
        r.ui,
        r.rect,
        owner,
        scope,
        r.theme,
        r.zoom,
        crate::ui::scope::Hands::Off,
    );
    // Read-only: nothing a hand did here is an edit, and `meters` says so by handing back an
    // empty list. Returning it unmapped would be the same thing said twice.
    Vec::new()
}

/// silvia's canvas is 160 px in a node 320 wide, and the size is load-bearing: at half that
/// the spectrum is a smear, the handles overlap, and there is nowhere to put a threshold.
fn size(_node: &Node) -> f32 {
    canvas::SCOPE_HEIGHT
}

fn show(r: &mut RegionUi<'_>) -> Vec<RegionEvent> {
    let Some(scope) = r.live.scope else {
        return Vec::new();
    };
    let owner = crate::ui::scope::Owner::node(r.id, r.node.def.slug);
    crate::ui::scope::show(r.ui, r.rect, owner, scope, r.theme, r.zoom)
        .into_iter()
        .map(|edit| {
            RegionEvent::Controls(
                edit.into_iter()
                    .map(|(key, value)| (key, crate::graph::ControlValue::Float(value)))
                    .collect(),
            )
        })
        .collect()
}
