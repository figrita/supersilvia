// SPDX-License-Identifier: AGPL-3.0-or-later

//! Layer 1: the picture-window protocol, with no compositor and no GPU.
//!
//! What a mark asks for, what the thread is told, and what the editor believes as a result —
//! the whole open/close/fullscreen state machine of `render::picture::Wall`, and the pure
//! geometry a window's hand is read with. The windows themselves are Wayland's on Linux and
//! AppKit's on macOS and are not testable here; this is the half that decides *which* windows
//! there should be, which is the half a mark reads to know whether to light.

use supersilvia::graph::NodeId;
use supersilvia::render::picture::{Ask, Popped, Request, Shown, Told, Wall};

fn mix(fullscreen: bool) -> Request {
    Request {
        picture: Shown::Mix,
        fullscreen,
    }
}

fn node(id: u32) -> Shown {
    Shown::Node {
        node: NodeId(id),
        port: None,
    }
}

const SIZE: (u32, u32) = (1920, 1080);

fn title() -> String {
    "Mix".to_string()
}

#[test]
fn the_pop_out_mark_is_a_toggle() {
    let mut wall = Wall::default();
    assert!(wall.open().is_empty());

    let ask = wall.mark(mix(false), SIZE, title());
    assert_eq!(
        ask,
        Some(Ask::Open {
            picture: Shown::Mix,
            size: SIZE,
            title: title(),
            fullscreen: false,
        })
    );
    assert_eq!(
        wall.open(),
        [Popped {
            picture: Shown::Mix,
            fullscreen: false
        }]
    );

    // The same click puts the window away, and the mark goes dark on the click rather than
    // on the answer — which is what makes it usable for a window on a screen nobody can see.
    assert_eq!(
        wall.mark(mix(false), SIZE, title()),
        Some(Ask::Close(Shown::Mix))
    );
    assert!(wall.open().is_empty());
}

#[test]
fn the_fullscreen_mark_opens_a_window_already_fullscreen() {
    let mut wall = Wall::default();
    assert_eq!(
        wall.mark(mix(true), SIZE, title()),
        Some(Ask::Open {
            picture: Shown::Mix,
            size: SIZE,
            title: title(),
            fullscreen: true,
        }),
        "one path to fullscreen, opening included"
    );
    // Until the thread says otherwise, the window is not fullscreen: what a mark shows is
    // what the window is *doing*, not what it was asked for.
    assert!(!wall.open()[0].fullscreen);

    wall.told(Told::Fullscreen(Shown::Mix, true));
    assert!(wall.open()[0].fullscreen);

    // And on a window that is already up, the mark flips it.
    assert_eq!(
        wall.mark(mix(true), SIZE, title()),
        Some(Ask::Fullscreen(Shown::Mix, false))
    );
    wall.told(Told::Fullscreen(Shown::Mix, false));
    assert!(!wall.open()[0].fullscreen);
}

#[test]
fn a_window_that_closed_itself_leaves_the_list() {
    let mut wall = Wall::default();
    wall.mark(mix(false), SIZE, title());
    assert_eq!(wall.told(Told::Closed(Shown::Mix)), None, "nothing to say");
    assert!(wall.open().is_empty());
}

#[test]
fn a_window_that_could_not_open_says_why_and_leaves_no_mark_lit() {
    let mut wall = Wall::default();
    wall.mark(mix(false), SIZE, title());
    assert_eq!(
        wall.told(Told::Failed(Shown::Mix, "no Wayland".to_string())),
        Some("no Wayland".to_string()),
        "the reason reaches the toast"
    );
    assert!(
        wall.open().is_empty(),
        "and the mark does not stay lit for a window that does not exist"
    );
}

#[test]
fn every_picture_is_its_own_window() {
    let mut wall = Wall::default();
    for picture in [Shown::Mix, node(1), node(2)] {
        wall.mark(
            Request {
                picture,
                fullscreen: false,
            },
            SIZE,
            title(),
        );
    }
    assert_eq!(wall.open().len(), 3, "the mix is a picture like any other");
    assert!(wall.has(node(1)));

    // A picture whose node has gone takes its window with it.
    assert_eq!(wall.forget(node(1)), Some(Ask::Close(node(1))));
    assert!(!wall.has(node(1)));
    assert_eq!(wall.forget(node(1)), None, "and only once");
    assert_eq!(wall.open().len(), 2);
}

#[test]
fn a_source_port_is_a_different_window_from_its_nodes_render() {
    let mut wall = Wall::default();
    let render = Shown::Node {
        node: NodeId(7),
        port: None,
    };
    let source = Shown::Node {
        node: NodeId(7),
        port: Some("out"),
    };
    for picture in [render, source] {
        wall.mark(
            Request {
                picture,
                fullscreen: false,
            },
            SIZE,
            title(),
        );
    }
    assert_eq!(wall.open().len(), 2);
    assert!(render.is(NodeId(7), None));
    assert!(!render.is(NodeId(7), Some("out")));
    assert_eq!(Shown::Mix.node(), None);
    assert_eq!(render.node(), Some(NodeId(7)));
}

#[test]
fn the_pop_out_mark_closes_a_window_that_is_fullscreen() {
    let mut wall = Wall::default();
    wall.mark(mix(true), SIZE, title());
    wall.told(Told::Fullscreen(Shown::Mix, true));
    assert!(wall.open()[0].fullscreen);

    // The pop-out mark is the same toggle whatever the window is doing: it puts it away.
    assert_eq!(
        wall.mark(mix(false), SIZE, title()),
        Some(Ask::Close(Shown::Mix))
    );
    assert!(wall.open().is_empty());
}

#[test]
fn a_picture_reopens_after_it_was_closed_and_does_not_remember_fullscreen() {
    let mut wall = Wall::default();
    wall.mark(mix(true), SIZE, title());
    wall.told(Told::Fullscreen(Shown::Mix, true));
    wall.told(Told::Closed(Shown::Mix));
    assert!(wall.open().is_empty());

    assert_eq!(
        wall.mark(mix(false), SIZE, title()),
        Some(Ask::Open {
            picture: Shown::Mix,
            size: SIZE,
            title: title(),
            fullscreen: false,
        }),
        "a closed window opens again rather than being remembered as open"
    );
    assert!(
        !wall.open()[0].fullscreen,
        "and it does not come back wearing the last window's fullscreen"
    );
}

#[test]
fn a_report_about_a_picture_with_no_window_changes_nothing() {
    let mut wall = Wall::default();
    // A `Told` can arrive after the mark that closed the window: the answer is in flight
    // while the click lands. It must not resurrect anything or panic.
    assert_eq!(wall.told(Told::Fullscreen(Shown::Mix, true)), None);
    assert_eq!(wall.told(Told::Closed(node(1))), None);
    assert!(wall.open().is_empty());

    wall.mark(mix(false), SIZE, title());
    assert_eq!(wall.told(Told::Fullscreen(node(1), true)), None);
    assert_eq!(
        wall.open(),
        [Popped {
            picture: Shown::Mix,
            fullscreen: false
        }],
        "another picture's report leaves this one alone"
    );
}

#[test]
fn a_thread_that_is_gone_takes_every_window_with_it() {
    let mut wall = Wall::default();
    wall.mark(mix(false), SIZE, title());
    wall.mark(
        Request {
            picture: node(1),
            fullscreen: false,
        },
        SIZE,
        title(),
    );
    assert_eq!(wall.open().len(), 2);

    assert_eq!(
        wall.told(Told::Gone("the pictures thread has stopped".to_string())),
        Some("the pictures thread has stopped".to_string())
    );
    assert!(
        wall.open().is_empty(),
        "no mark stays lit for a window nothing is holding"
    );
}

/// The nine regions of a window, plus the window that has none.
///
/// A band on every edge is what makes a borderless window resizable at all; the table is
/// here because the geometry is the whole of it, and it is pure.
/// Another project opening closes every window on a node — a render and a source's port
/// alike, fullscreen or not — and asks the thread to close each; the mix's window stays.
#[test]
fn another_project_closes_every_nodes_window_and_keeps_the_mixs() {
    let mut wall = Wall::default();
    let port = Shown::Node {
        node: NodeId(3),
        port: Some("frame"),
    };
    for picture in [node(1), Shown::Mix, port] {
        wall.mark(
            Request {
                picture,
                fullscreen: false,
            },
            SIZE,
            title(),
        );
    }
    wall.told(Told::Fullscreen(node(1), true));
    assert_eq!(wall.forget_nodes(), [Ask::Close(node(1)), Ask::Close(port)]);
    assert_eq!(
        wall.open(),
        [Popped {
            picture: Shown::Mix,
            fullscreen: false
        }]
    );
    assert!(
        wall.forget_nodes().is_empty(),
        "and nothing is left to close"
    );
}

#[test]
fn every_edge_and_corner_is_a_band_and_the_middle_is_not() {
    use supersilvia::render::picture::{BAND, Edge, edge_at};

    const SIZE: (f64, f64) = (400.0, 300.0);
    let table = [
        ("top left", (2.0, 2.0), Some(Edge::TopLeft)),
        ("top", (200.0, 3.0), Some(Edge::Top)),
        ("top right", (397.0, 1.0), Some(Edge::TopRight)),
        ("left", (4.0, 150.0), Some(Edge::Left)),
        ("the middle", (200.0, 150.0), None),
        ("right", (396.0, 150.0), Some(Edge::Right)),
        ("bottom left", (1.0, 299.0), Some(Edge::BottomLeft)),
        ("bottom", (200.0, 295.0), Some(Edge::Bottom)),
        ("bottom right", (399.0, 298.0), Some(Edge::BottomRight)),
    ];
    for (region, pos, want) in table {
        assert_eq!(edge_at(pos, SIZE, BAND), want, "{region}");
    }

    // The band is twelve points wide, so a point ten in from an edge is still on it.
    assert_eq!(
        edge_at((10.0, 150.0), SIZE, BAND),
        Some(Edge::Left),
        "ten points in"
    );
    assert_eq!(
        edge_at((10.0, 292.0), SIZE, BAND),
        Some(Edge::BottomLeft),
        "ten points in from two edges"
    );

    // A point one band in from an edge is inside, which is the boundary the table straddles.
    assert_eq!(
        edge_at((BAND, 150.0), SIZE, BAND),
        None,
        "just inside the left band"
    );
    assert_eq!(
        edge_at((200.0, SIZE.1 - BAND), SIZE, BAND),
        None,
        "just inside the bottom band"
    );

    // **A fullscreen window has no bands**, which it says by passing a band of zero: there
    // is nothing to resize it to, and a press anywhere in it is a press in the middle.
    for (region, pos, _) in table {
        assert_eq!(edge_at(pos, SIZE, 0.0), None, "{region}, fullscreen");
    }

    // A window narrower than two bands still has two sides rather than one overlapping one.
    assert_eq!(edge_at((5.0, 5.0), (12.0, 12.0), BAND), Some(Edge::TopLeft));
    assert_eq!(
        edge_at((7.0, 7.0), (12.0, 12.0), BAND),
        Some(Edge::BottomRight)
    );
}

/// A locked resize answers the compositor's proposal with the picture's own aspect.
#[test]
fn an_aspect_locked_resize_derives_the_other_side_from_the_edge_being_dragged() {
    use supersilvia::render::picture::{Edge, fit_aspect};

    // 16:9, which is what a picture window most often opens at.
    const A: f64 = 16.0 / 9.0;
    let table = [
        // A vertical edge is a width, so the height follows it.
        ("a vertical edge", (800, 600), Edge::Right, (800, 450)),
        (
            "the other vertical edge",
            (800, 600),
            Edge::Left,
            (800, 450),
        ),
        // A horizontal edge is a height, so the width follows it.
        ("a horizontal edge", (800, 600), Edge::Bottom, (1067, 600)),
        (
            "the other horizontal edge",
            (800, 600),
            Edge::Top,
            (1067, 600),
        ),
        // A corner is both, so the answer is the one that fits inside the proposal.
        (
            "a corner, wider than tall",
            (800, 600),
            Edge::BottomRight,
            (800, 450),
        ),
        (
            "a corner, taller than wide",
            (400, 600),
            Edge::TopLeft,
            (400, 225),
        ),
        // A proposal already at the aspect comes back unchanged, from every anchor.
        (
            "already at the aspect",
            (1280, 720),
            Edge::BottomRight,
            (1280, 720),
        ),
        (
            "already at the aspect, edge",
            (1280, 720),
            Edge::Top,
            (1280, 720),
        ),
    ];
    for (what, proposed, edge, want) in table {
        assert_eq!(fit_aspect(proposed, A, edge), want, "{what}");
    }

    // A corner never answers with a size bigger than what was proposed, on either axis.
    for proposed in [(800u32, 600u32), (400, 600), (1920, 200), (120, 1000)] {
        let (w, h) = fit_aspect(proposed, A, Edge::BottomRight);
        assert!(
            w <= proposed.0 && h <= proposed.1,
            "{proposed:?} grew to {w}x{h}"
        );
    }
}

/// What `Escape` does, which is undo the last thing the hand did to the window.
#[test]
fn escape_closes_a_window_that_was_opened_fullscreen_in_one() {
    use supersilvia::render::picture::{Escaped, escaped};

    // Opened by the fullscreen mark: there is no pop-out behind it to go back to.
    assert_eq!(escaped(true, true), Escaped::Closed);
    // Opened as a pop-out and then fullscreened by `F` or a double-click: back to the
    // pop-out first, and the next `Escape` closes it.
    assert_eq!(escaped(true, false), Escaped::Windowed);
    assert_eq!(escaped(false, false), Escaped::Closed);
    // And a window opened fullscreen that was taken out of fullscreen by hand still closes.
    assert_eq!(escaped(false, true), Escaped::Closed);
}

/// Where a window a hand drags goes, where the window system does not drag it for us: the
/// middle moves it whole, and an edge or a corner follows the pointer while the opposite side
/// stays put.
#[test]
fn a_dragged_window_moves_from_the_middle_and_resizes_from_an_edge() {
    use supersilvia::render::picture::{Bounds, Edge, dragged};

    const MIN: (f64, f64) = (160.0, 90.0);
    let start = Bounds {
        x: 100.0,
        y: 100.0,
        width: 400.0,
        height: 300.0,
    };
    let at = |x: f64, y: f64, width: f64, height: f64| Bounds {
        x,
        y,
        width,
        height,
    };
    let travel = (10.0, 20.0);
    let table = [
        ("the middle", None, at(110.0, 120.0, 400.0, 300.0)),
        ("left", Some(Edge::Left), at(110.0, 100.0, 390.0, 300.0)),
        ("right", Some(Edge::Right), at(100.0, 100.0, 410.0, 300.0)),
        ("top", Some(Edge::Top), at(100.0, 120.0, 400.0, 280.0)),
        ("bottom", Some(Edge::Bottom), at(100.0, 100.0, 400.0, 320.0)),
        (
            "top left",
            Some(Edge::TopLeft),
            at(110.0, 120.0, 390.0, 280.0),
        ),
        (
            "top right",
            Some(Edge::TopRight),
            at(100.0, 120.0, 410.0, 280.0),
        ),
        (
            "bottom left",
            Some(Edge::BottomLeft),
            at(110.0, 100.0, 390.0, 320.0),
        ),
        (
            "bottom right",
            Some(Edge::BottomRight),
            at(100.0, 100.0, 410.0, 320.0),
        ),
    ];
    for (what, edge, want) in table {
        assert_eq!(dragged(start, travel, edge, None, MIN), want, "{what}");
    }

    // Never smaller than the minimum, and the side that was not dragged stays where it was.
    assert_eq!(
        dragged(start, (-1000.0, 0.0), Some(Edge::Right), None, MIN),
        at(100.0, 100.0, 160.0, 300.0),
        "the right edge dragged past the left"
    );
    assert_eq!(
        dragged(start, (1000.0, 0.0), Some(Edge::Left), None, MIN),
        at(340.0, 100.0, 160.0, 300.0),
        "the left edge dragged past the right keeps the right edge"
    );
    assert_eq!(
        dragged(start, (0.0, 1000.0), Some(Edge::Top), None, MIN),
        at(100.0, 310.0, 400.0, 90.0),
        "the top edge dragged past the bottom keeps the bottom edge"
    );
}

/// With `Shift` or `Ctrl` held, a dragged edge keeps the picture's aspect, anchored on the
/// edge being dragged, and the opposite corner stays put.
#[test]
fn a_locked_drag_keeps_the_aspect_and_the_opposite_corner() {
    use supersilvia::render::picture::{Bounds, Edge, dragged};

    const MIN: (f64, f64) = (160.0, 90.0);
    const A: f64 = 16.0 / 9.0;
    let start = Bounds {
        x: 0.0,
        y: 0.0,
        width: 320.0,
        height: 180.0,
    };
    let at = |x: f64, y: f64, width: f64, height: f64| Bounds {
        x,
        y,
        width,
        height,
    };
    let table = [
        // An edge: the other side follows it.
        (
            "right",
            (160.0, 0.0),
            Edge::Right,
            at(0.0, 0.0, 480.0, 270.0),
        ),
        (
            "left",
            (-160.0, 0.0),
            Edge::Left,
            at(-160.0, 0.0, 480.0, 270.0),
        ),
        (
            "bottom",
            (0.0, 90.0),
            Edge::Bottom,
            at(0.0, 0.0, 480.0, 270.0),
        ),
        ("top", (0.0, -90.0), Edge::Top, at(0.0, -90.0, 480.0, 270.0)),
        // A corner: whichever fits inside where the pointer is.
        (
            "bottom right",
            (160.0, 30.0),
            Edge::BottomRight,
            at(0.0, 0.0, 373.0, 210.0),
        ),
        (
            "top left",
            (-160.0, -30.0),
            Edge::TopLeft,
            at(-53.0, -30.0, 373.0, 210.0),
        ),
        // Shrunk past the minimum: held there, at the aspect.
        (
            "right, past the minimum",
            (-300.0, 0.0),
            Edge::Right,
            at(0.0, 0.0, 160.0, 90.0),
        ),
    ];
    for (what, travel, edge, want) in table {
        assert_eq!(
            dragged(start, travel, Some(edge), Some(A), MIN),
            want,
            "{what}"
        );
    }

    // A square shrunk below the minimum height grows back to the minimum width at its own
    // aspect, rather than giving the aspect up.
    let square = at(0.0, 0.0, 200.0, 200.0);
    assert_eq!(
        dragged(square, (0.0, -150.0), Some(Edge::Bottom), Some(1.0), MIN),
        at(0.0, 0.0, 160.0, 160.0)
    );

    // The middle moves the window whatever is held.
    assert_eq!(
        dragged(start, (5.0, 7.0), None, Some(A), MIN),
        at(5.0, 7.0, 320.0, 180.0)
    );
}
