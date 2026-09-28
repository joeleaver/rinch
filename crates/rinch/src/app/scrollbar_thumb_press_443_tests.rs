//! A press on a scrollbar **thumb** grabs it where it is; only a press on the
//! empty **track** jumps (#443).
//!
//! Every scroller here is scrolled part-way before it is pressed, and the thumb
//! is pressed off its centre. Both are deliberate: at scroll 0 with a press on
//! the thumb's leading edge, jump-to-click and grab-in-place land on the same
//! offset, so a fixture sitting there passes against the bug. And a thumb test
//! that ignored the scroll offset (the thumb tested where scroll 0 put it)
//! would pass any press made at scroll 0.

use super::*;
use rinch_dom::paint::scrollbar::{MARGIN, THICKNESS};

const SIZE: (u32, u32) = (800, 600);

/// A 200x100 scroller at the document origin whose content overflows on
/// `axis` only, scrolled by `wheel` px along it.
fn mount(vertical: bool, wheel: f64) -> (RinchApp, usize) {
    let (overflow, content) = if vertical {
        ("overflow-y: auto", "width: 40px; height: 400px")
    } else {
        ("overflow-x: auto", "width: 800px; height: 40px")
    };
    let container_style =
        format!("position: absolute; left: 0px; top: 0px; width: 200px; height: 100px; {overflow}");
    let content = content.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 800px; height: 600px");
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", &container_style);
        scroller.set_attribute("id", "scroller");
        let inner = scope.create_element("div");
        inner.set_attribute("style", &content);
        scroller.append_child(&inner);
        root.append_child(&scroller);
        root
    });
    app.mount_component(800.0, 600.0);
    app.resolve_and_repaint(800.0, 600.0);
    let id = {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree
            .nodes
            .iter()
            .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some("scroller"))
            .map(|(id, _)| id)
            .unwrap()
    };
    let (dx, dy) = if vertical {
        (0.0, -wheel)
    } else {
        (-wheel, 0.0)
    };
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 100.0,
            y: 50.0,
            delta_x: dx,
            delta_y: dy,
        },
        SIZE,
        1.0,
    );
    app.resolve_and_repaint(800.0, 600.0);
    (app, id)
}

fn scroll(app: &RinchApp, id: usize, vertical: bool) -> f64 {
    let o = app.doc.as_ref().unwrap().borrow().tree.nodes[id].scroll_offset;
    if vertical { o.1 } else { o.0 }
}

fn track(app: &RinchApp, id: usize, vertical: bool) -> rinch_dom::paint::scrollbar::ScrollbarTrack {
    let d = app.doc.as_ref().unwrap().borrow();
    let bars = rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0);
    if vertical {
        bars.vertical.expect("a vertical bar")
    } else {
        bars.horizontal.expect("a horizontal bar")
    }
}

/// The window point `along` px down (or across) the bar, centred on its
/// thickness.
fn on_bar(vertical: bool, along: f64) -> (f32, f32) {
    let across = (MARGIN + THICKNESS / 2.0) as f32;
    if vertical {
        (200.0 - across, along as f32)
    } else {
        (along as f32, 100.0 - across)
    }
}

fn press(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(
        PlatformEvent::MouseDown {
            x,
            y,
            button: rinch_platform::MouseButton::Left,
        },
        SIZE,
        1.0,
    );
}

fn move_to(app: &mut RinchApp, (x, y): (f32, f32)) {
    app.handle_event(PlatformEvent::MouseMove { x, y }, SIZE, 1.0);
}

/// The scrolled, off-centre premise every thumb press below rests on, stated
/// rather than assumed: the scroller is neither at an end nor at the middle,
/// and jump-to-click at the press point would land somewhere measurably else.
fn assert_premise(t: &rinch_dom::paint::scrollbar::ScrollbarTrack, s: f64, along: f64) {
    assert!(
        s > 0.0 && s < t.max_scroll && (s - t.max_scroll / 2.0).abs() > 1.0,
        "the scroller must be scrolled part-way, off the midpoint (scroll {s} of {})",
        t.max_scroll
    );
    assert!(
        (t.scroll_for_click(along) - s).abs() > 5.0,
        "jump-to-click must land elsewhere, or this fixture cannot tell the two apart \
         (jump {} vs {s})",
        t.scroll_for_click(along)
    );
}

fn thumb_press_grabs_in_place(vertical: bool) {
    let (mut app, id) = mount(vertical, 110.0);
    let s = scroll(&app, id, vertical);
    let t = track(&app, id, vertical);
    // Three quarters of the way along the thumb: neither its leading edge nor
    // its centre.
    let along = t.thumb_start(s) + t.thumb_len * 0.75;
    assert_premise(&t, s, along);

    press(&mut app, on_bar(vertical, along));
    assert!(app.scrollbar_drag.is_some(), "the thumb press arms a drag");
    assert_eq!(
        scroll(&app, id, vertical),
        s,
        "pressing the thumb must not move it"
    );

    // And the drag then runs 1:1 from where the thumb was, not from where a
    // jump would have put it.
    move_to(&mut app, on_bar(vertical, along + 10.0));
    let expected = t.scroll_for_drag(s, 10.0);
    let got = scroll(&app, id, vertical);
    assert!(
        (got - expected).abs() < 1e-3,
        "a 10px drag from the grabbed thumb lands on {expected}, got {got}"
    );
}

#[test]
fn pressing_a_scrolled_vertical_thumb_grabs_it_in_place() {
    thumb_press_grabs_in_place(true);
}

#[test]
fn pressing_a_scrolled_horizontal_thumb_grabs_it_in_place() {
    thumb_press_grabs_in_place(false);
}

/// The other half of the rule: the empty track still jumps, both before and
/// after the thumb, and to exactly what `scroll_for_click` says.
#[test]
fn pressing_the_empty_track_still_jumps() {
    for vertical in [true, false] {
        for after_thumb in [false, true] {
            let (mut app, id) = mount(vertical, 110.0);
            let s = scroll(&app, id, vertical);
            let t = track(&app, id, vertical);
            let start = t.thumb_start(s);
            let along = if after_thumb {
                start + t.thumb_len + 6.0
            } else {
                start - 6.0
            };
            assert!(
                along > t.track_start && along < t.track_start + t.track_len,
                "the probe is on the track"
            );
            press(&mut app, on_bar(vertical, along));
            assert!(app.scrollbar_drag.is_some());
            let got = scroll(&app, id, vertical);
            assert!(
                (got - t.scroll_for_click(along)).abs() < 1e-3,
                "a track press jumps to {} (vertical {vertical}, after {after_thumb}), got {got}",
                t.scroll_for_click(along)
            );
            assert!((got - s).abs() > 1.0, "and that is a move");
        }
    }
}

/// The thumb is the thumb where it is **painted** now, not where scroll 0 put
/// it: a press on the track the thumb has scrolled away from jumps.
#[test]
fn the_track_the_thumb_scrolled_away_from_jumps() {
    let (mut app, id) = mount(true, 110.0);
    let s = scroll(&app, id, true);
    let t = track(&app, id, true);
    // Inside the span the thumb held at scroll 0, above where it is now.
    let along = t.track_start + t.thumb_len * 0.5;
    assert!(
        along < t.thumb_start(s),
        "the probe must be where the thumb was and no longer is"
    );
    press(&mut app, on_bar(true, along));
    assert!(
        (scroll(&app, id, true) - t.scroll_for_click(along)).abs() < 1e-3,
        "the vacated track jumps"
    );
}

/// The painted thumb, read out of the frame, stays put under the press — the
/// state assertions above could agree with a thumb that paint put elsewhere.
#[cfg(software_shell)]
#[test]
fn the_painted_thumb_does_not_move_under_the_press() {
    use super::scrollbar_drag_pixel_tests::thumb_span;
    let (mut app, id) = mount(true, 110.0);
    let s = scroll(&app, id, true);
    let t = track(&app, id, true);
    let before = thumb_span(&mut app, 200.0);
    let along = (before.0 + (before.1 - before.0) * 0.75) + 0.5;
    assert_premise(&t, s, along);
    press(&mut app, on_bar(true, along));
    let after = thumb_span(&mut app, 200.0);
    assert_eq!(
        before, after,
        "the painted thumb must not jump under the press"
    );
}
