//! With both bars up, every thumb pixel paint draws is a pixel a press grabs,
//! and the bottom-right corner belongs to neither bar — in paint as well as in
//! hit testing (#444).
//!
//! Paint used to reserve `THICKNESS + MARGIN` (8px) at each track's far end
//! while the hit strips stopped `SCROLLBAR_HIT_THICKNESS` (16px) short, so at
//! full scroll the last ~6px of each thumb was drawn inside the square hit
//! testing gives to neither bar: visible and not grabbable.
//!
//! The oracle is the rasterised frame. Every fixture scrolls to the **far
//! end** as well as part-way: at scroll 0 the thumb sits at the track's start,
//! nowhere near the corner, so a fixture there passes against the bug.
//!
//! The last fixture is the #1135 review's L3: a track press that cannot move
//! the scroll (it clamps to the value it already has) writes, dirties and fires
//! nothing.

use super::hit_testing::{SCROLLBAR_HIT_THICKNESS, find_scrollbar_hit, pointer_on_scrollbar_thumb};
use super::*;

const SIZE: (u32, u32) = (800, 600);
const W: f32 = 200.0;
const H: f32 = 100.0;

/// A `W`x`H` scroller at the document origin, white on white, overflowing on
/// both axes, scrolled by `(wheel_x, wheel_y)` px.
fn mount(extra_style: &str, wheel: (f64, f64), onscroll: Option<String>) -> (RinchApp, usize) {
    let container_style = format!(
        "position: absolute; left: 0px; top: 0px; width: {W}px; height: {H}px; \
         overflow: auto; background: white; {extra_style}"
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 800px; height: 600px; background: white",
        );
        let scroller = scope.create_element("div");
        scroller.set_attribute("style", &container_style);
        scroller.set_attribute("id", "scroller");
        if let Some(h) = &onscroll {
            scroller.set_attribute("data-onscroll", h);
        }
        let inner = scope.create_element("div");
        inner.set_attribute("style", "width: 700px; height: 450px");
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
    if wheel != (0.0, 0.0) {
        app.handle_event(
            PlatformEvent::MouseWheel {
                x: 100.0,
                y: 50.0,
                delta_x: -wheel.0,
                delta_y: -wheel.1,
            },
            SIZE,
            1.0,
        );
        app.resolve_and_repaint(800.0, 600.0);
    }
    (app, id)
}

fn offsets(app: &RinchApp, id: usize) -> (f64, f64) {
    app.doc.as_ref().unwrap().borrow().tree.nodes[id].scroll_offset
}

fn bars(app: &RinchApp, id: usize) -> rinch_dom::paint::scrollbar::Scrollbars {
    let d = app.doc.as_ref().unwrap().borrow();
    rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, 1.0)
}

/// Every pixel inside the container with a channel at or below `max_channel`,
/// as `(x, y)` device pixels. `254` is "painted at all".
#[cfg(software_shell)]
fn painted_pixels(app: &mut RinchApp, max_channel: u8) -> Vec<(u32, u32)> {
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    app.has_previous_frame = false;
    let (pixels, w, _h) = app.build_pixels(1.0, SIZE, false);
    let mut out = Vec::new();
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let i = ((y * w + x) * 4) as usize;
            if pixels[i..i + 3].iter().any(|&c| c <= max_channel) {
                out.push((x, y));
            }
        }
    }
    out
}

/// The scroll positions every pixel fixture is run at: the far end on both
/// axes (where the defect was), and part-way on both, off the midpoint.
const WHEELS: [(f64, f64); 2] = [(5000.0, 5000.0), (137.0, 61.0)];

/// Every pixel of either thumb is on a hit strip, on the thumb that strip
/// belongs to. Before #444 the far-end rows of each thumb fell in the corner
/// square neither strip claims.
#[cfg(software_shell)]
#[test]
fn every_painted_thumb_pixel_is_grabbable_with_both_bars_up() {
    for wheel in WHEELS {
        let (mut app, id) = mount("", wheel, None);
        let b = bars(&app, id);
        let (v, h) = (b.vertical.unwrap(), b.horizontal.unwrap());
        let (sx, sy) = offsets(&app, id);
        if wheel.0 > 1000.0 {
            assert_eq!(
                (sx, sy),
                (h.max_scroll, v.max_scroll),
                "premise: at the far end"
            );
        } else {
            assert!(sx > 0.0 && sx < h.max_scroll && sy > 0.0 && sy < v.max_scroll);
        }
        // Pixels the thumb covers at least half of. The built-in thumb is 40%
        // black over white, 153 fully covered; 204 is half coverage. A pixel
        // the round cap or the thumb's end only grazes has its centre off the
        // thumb, and a press there is correctly not a grab.
        let px = painted_pixels(&mut app, 204);
        assert!(
            px.len() > 100,
            "premise: both thumbs are painted ({} px)",
            px.len()
        );
        let d = app.doc.as_ref().unwrap().borrow();
        let mut bad = Vec::new();
        for &(x, y) in &px {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let on_hit = find_scrollbar_hit(&d.tree, cx, cy).is_some_and(|hit| hit.node_id == id);
            if !on_hit || !pointer_on_scrollbar_thumb(&d.tree, cx, cy) {
                bad.push((x, y));
            }
        }
        assert!(
            bad.is_empty(),
            "at scroll {:?}, {} painted thumb pixels are not grabbable, e.g. {:?}",
            (sx, sy),
            bad.len(),
            &bad[..bad.len().min(6)]
        );
    }
}

/// Nothing is painted in the corner square hit testing gives to neither bar —
/// not a thumb, and not a `--rinch-scrollbar-color` track either, which runs
/// the track's whole length. The positive control is the pixel count just
/// outside the square: the bars do run right up to it.
#[cfg(software_shell)]
#[test]
fn nothing_is_painted_in_the_corner_neither_bar_claims() {
    for style in ["", "--rinch-scrollbar-color: #808080 #c0c0c0"] {
        for wheel in WHEELS {
            let (mut app, _id) = mount(style, wheel, None);
            let px = painted_pixels(&mut app, 254);
            let t = SCROLLBAR_HIT_THICKNESS;
            let in_corner: Vec<_> = px
                .iter()
                .filter(|&&(x, y)| x as f32 >= W - t && y as f32 >= H - t)
                .collect();
            assert!(
                in_corner.is_empty(),
                "style {style:?}, wheel {wheel:?}: {} px painted in the corner, e.g. {:?}",
                in_corner.len(),
                &in_corner[..in_corner.len().min(6)]
            );
            // Just outside it, on each axis: painted when the thumb (or the
            // track) is at that end, which the far-end run and the track run
            // both guarantee.
            if wheel.0 > 1000.0 || !style.is_empty() {
                let above = px.iter().any(|&(x, y)| {
                    x as f32 >= W - t && (y as f32) < H - t && y as f32 >= H - t - 4.0
                });
                let left = px.iter().any(|&(x, y)| {
                    y as f32 >= H - t && (x as f32) < W - t && x as f32 >= W - t - 4.0
                });
                assert!(
                    above && left,
                    "the bars reach the corner ({style:?}, {wheel:?})"
                );
            }
        }
    }
}

/// The #1135 review's L3: a track press that cannot move the scroll — past
/// the thumb at the far end, or before it at the start — keeps the offset,
/// dirties nothing and fires no `onscroll`, as a grab does. It still arms the
/// drag, so a move after it scrolls; the positive control is a track press
/// that does move, which fires.
#[test]
fn a_track_press_that_cannot_move_the_scroll_fires_nothing() {
    use rinch_core::events::{ScrollCallback, ScrollEvent, register_scroll_handler};
    use std::cell::Cell;

    let fired = Rc::new(Cell::new(0usize));
    let fired_in = fired.clone();
    let handler = register_scroll_handler(ScrollCallback::from(move |_: ScrollEvent| {
        fired_in.set(fired_in.get() + 1);
    }));
    let release = |app: &mut RinchApp, (x, y): (f32, f32)| {
        app.handle_event(
            PlatformEvent::MouseUp {
                x,
                y,
                button: rinch_platform::MouseButton::Left,
            },
            SIZE,
            1.0,
        );
    };
    let press = |app: &mut RinchApp, (x, y): (f32, f32)| {
        app.handle_event(
            PlatformEvent::MouseDown {
                x,
                y,
                button: rinch_platform::MouseButton::Left,
            },
            SIZE,
            1.0,
        );
    };
    // Vertical-only, so the whole right edge is its strip: `overflow-x:
    // hidden` keeps the horizontal bar down.
    for far_end in [true, false] {
        let wheel = if far_end { (0.0, 5000.0) } else { (0.0, 0.0) };
        let (mut app, id) = mount("overflow-x: hidden", wheel, Some(handler.0.to_string()));
        let v = bars(&app, id).vertical.unwrap();
        assert!(bars(&app, id).horizontal.is_none(), "premise: one bar");
        let s = offsets(&app, id).1;
        assert_eq!(s, if far_end { v.max_scroll } else { 0.0 }, "premise");
        // In the track's end margin, beyond the thumb: the jump clamps to the
        // offset the scroller already has.
        let along = if far_end {
            v.track_start + v.track_len + 1.3
        } else {
            v.track_start - 1.3
        };
        assert!(!v.thumb_contains(s, along), "premise: off the thumb");
        assert_eq!(v.scroll_for_click(along), s, "premise: the jump is a no-op");
        let at = (W - 5.0, along as f32);
        {
            let d = app.doc.as_ref().unwrap().borrow();
            assert!(
                find_scrollbar_hit(&d.tree, at.0, at.1).is_some(),
                "premise: on the strip"
            );
            assert!(
                !d.tree.dirty_nodes.contains(&id),
                "premise: clean before the press"
            );
        }
        let base = fired.get();
        press(&mut app, at);
        assert!(app.scrollbar_drag.is_some(), "the press arms a drag");
        assert_eq!(offsets(&app, id).1, s);
        assert_eq!(
            fired.get(),
            base,
            "a press that moved nothing fires nothing"
        );
        assert!(
            !app.doc
                .as_ref()
                .unwrap()
                .borrow()
                .tree
                .dirty_nodes
                .contains(&id),
            "and dirties nothing"
        );
        // The drag it armed still scrolls, back toward the other end.
        let back = if far_end { -9.0 } else { 9.0 };
        app.handle_event(
            PlatformEvent::MouseMove {
                x: at.0,
                y: at.1 + back,
            },
            SIZE,
            1.0,
        );
        assert!((offsets(&app, id).1 - s).abs() > 1.0, "the drag moves");
        release(&mut app, (at.0, at.1 + back));

        // Positive control: a track press that does move fires.
        app.resolve_and_repaint(800.0, 600.0);
        let s = offsets(&app, id).1;
        let far = if far_end {
            v.track_start + 3.0
        } else {
            v.track_start + v.track_len - 3.0
        };
        assert!(!v.thumb_contains(s, far));
        let base = fired.get();
        press(&mut app, (W - 5.0, far as f32));
        assert!((offsets(&app, id).1 - s).abs() > 1.0);
        assert_eq!(fired.get(), base + 1, "a track press that moves fires");
    }
}

/// The geometry the two pixel fixtures rest on, at every thickness and at a
/// device scale of 2: each bar's painted track (its whole length, across the
/// width it is drawn) lies inside that bar's hit strip, and the two strips do
/// not meet — the corner is neither's. Asked of `scrollbars` itself, which is
/// what paint and hit testing both read.
#[test]
fn each_painted_track_lies_inside_its_hit_strip_at_every_scale_and_width() {
    use rinch_dom::paint::scrollbar::ScrollbarAxis;
    for width in ["", "--rinch-scrollbar-width: thin"] {
        let (app, id) = mount(width, (0.0, 0.0), None);
        for scale in [1.0, 2.0] {
            let d = app.doc.as_ref().unwrap().borrow();
            let b = rinch_dom::paint::scrollbar::scrollbars(&d.tree, id, scale);
            let (bw, bh) = (W as f64 * scale, H as f64 * scale);
            assert_eq!((b.box_width, b.box_height), (bw, bh));
            let v = b.vertical.expect("a vertical bar");
            let h = b.horizontal.expect("a horizontal bar");
            let vs = b.hit_strip(ScrollbarAxis::Vertical).unwrap();
            let hs = b.hit_strip(ScrollbarAxis::Horizontal).unwrap();
            // Painted rects, as paint draws them (`paint_node`).
            let vx0 = bw - b.thickness - b.margin;
            let hy0 = bh - b.thickness - b.margin;
            let v_paint = (
                vx0,
                v.track_start,
                vx0 + b.thickness,
                v.track_start + v.track_len,
            );
            let h_paint = (
                h.track_start,
                hy0,
                h.track_start + h.track_len,
                hy0 + b.thickness,
            );
            let inside = |p: (f64, f64, f64, f64), s: (f64, f64, f64, f64)| {
                p.0 >= s.0 && p.1 >= s.1 && p.2 <= s.2 && p.3 <= s.3
            };
            assert!(
                inside(v_paint, vs),
                "{width:?} x{scale}: {v_paint:?} in {vs:?}"
            );
            assert!(
                inside(h_paint, hs),
                "{width:?} x{scale}: {h_paint:?} in {hs:?}"
            );
            // The corner: each strip stops at the `hit_thickness` square at the
            // bottom-right, so its interior is on neither.
            assert_eq!((vs.3, hs.2), (bh - b.hit_thickness, bw - b.hit_thickness));
        }
    }
}
