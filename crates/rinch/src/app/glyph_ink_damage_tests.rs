//! Glyph ink past an IFC root's box is damaged and repainted (#1724, #1632).
//!
//! An IFC root's damage is its box grown by its ink (`paint::ink_outsets_in`),
//! now and as last painted (`Node::painted`), and the paint prune tests the
//! same grown box. That ink held what the root draws past its *line boxes*
//! (shifted text, inline backgrounds, wavy underlines, #1815) but not its
//! glyphs where they leave the box on unshifted lines:
//!
//! - **vertically** (#1724): with `line-height` below the font's ascent plus
//!   descent, ascenders and descenders are drawn above and below the line
//!   box, so a colour change kept the old colour there, and a neighbour's
//!   damage over them pruned the root and erased them;
//! - **horizontally** (#1632): a `nowrap` line wider than its box draws past
//!   the box's right edge, so the glyphs there were left behind when the line
//!   moved.
//!
//! Each fixture is a whole-frame pixel oracle: the incremental frame after
//! the step must equal a from-scratch frame of the same state, pixel for
//! pixel, on the software painter, with the frame proven partial by the perf
//! counters. Text is the bundled Inter (`perf_expect::new_app`).

use super::*;
use rinch_dom::perf::{Counter, FrameStats};

const W: u32 = 600;
const H: u32 = 240;

fn full_frame(app: &mut RinchApp) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(1.0, (W, H), false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn incremental_frame(app: &mut RinchApp) -> (Vec<u8>, FrameStats) {
    let px = app.build_pixels(1.0, (W, H), false).0.to_vec();
    let stats = app.end_perf_frame().expect("mounted");
    (px, stats)
}

/// The pixels that differ between two frames, and their bounding box.
fn differ(a: &[u8], b: &[u8]) -> (usize, Option<(u32, u32, u32, u32)>) {
    let mut n = 0;
    let mut bb: Option<(u32, u32, u32, u32)> = None;
    for (i, (pa, pb)) in a.chunks(4).zip(b.chunks(4)).enumerate() {
        if pa != pb {
            n += 1;
            let (x, y) = (i as u32 % W, i as u32 / W);
            bb = Some(bb.map_or((x, y, x, y), |(x0, y0, x1, y1)| {
                (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
            }));
        }
    }
    (n, bb)
}

fn el(scope: &mut RenderScope, parent: &NodeHandle, style: &str) -> NodeHandle {
    let e = scope.create_element("div");
    e.set_attribute("style", style);
    parent.append_child(&e);
    e
}

fn text(scope: &mut RenderScope, parent: &NodeHandle, s: &str) {
    let t = scope.create_text(s);
    parent.append_child(&t);
}

type Build = Box<dyn FnOnce(&mut RenderScope, &NodeHandle) -> Vec<NodeHandle>>;

/// Mount `build` under a white page and paint it whole.
fn mount(build: Build) -> (RinchApp, Vec<NodeHandle>) {
    let slot: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let slot_in = slot.clone();
    let mut app = super::perf_expect::new_app(move |scope: &mut RenderScope| {
        let outer = scope.create_element("div");
        outer.set_attribute(
            "style",
            "width: 600px; height: 240px; background: rgb(255, 255, 255); \
             font-family: sans-serif; color: rgb(0, 0, 0)",
        );
        *slot_in.borrow_mut() = build(scope, &outer);
        outer
    });
    app.mount_component(W as f32, H as f32);
    app.resolve_and_repaint(W as f32, H as f32);
    let _ = full_frame(&mut app);
    let handles = slot.borrow().clone();
    (app, handles)
}

/// Apply `step`, paint incrementally, and return how the frame differs from a
/// from-scratch paint of the same state (a failure message, or `None`).
fn check(name: &str, mut app: RinchApp, step: impl FnOnce()) -> Option<String> {
    step();
    app.resolve_and_repaint(W as f32, H as f32);
    let (inc, stats) = incremental_frame(&mut app);
    if stats.get(Counter::RepaintFull) != 0 || stats.get(Counter::RepaintPartial) != 1 {
        return Some(format!("{name}: not a partial frame: {stats:?}"));
    }
    let full = full_frame(&mut app);
    match differ(&inc, &full) {
        (0, _) => None,
        (n, bb) => Some(format!(
            "{name}: {n} px differ from a full repaint, in {bb:?} (repainted {} px)",
            stats.get(Counter::RepaintedPx)
        )),
    }
}

/// Three paragraphs at `font: 22px/10px`, whose ascenders and descenders
/// overlap the lines above and below (#1724's shape).
fn tight_lines(scope: &mut RenderScope, outer: &NodeHandle) -> Vec<NodeHandle> {
    let wrap = el(
        scope,
        outer,
        "font-size: 22px; line-height: 10px; padding: 20px",
    );
    let mut ps = Vec::new();
    for s in [
        "Ascenders lbdh and descenders gjpqy overlap",
        "the line next to them, ldk gjpqy",
        "and a third line, lbd gjpqy",
    ] {
        let p = el(scope, &wrap, "margin: 0");
        text(scope, &p, s);
        ps.push(p);
    }
    ps
}

#[test]
fn recolouring_text_whose_glyphs_leave_their_line_box_repaints_them() {
    let (app, ps) = mount(Box::new(tight_lines));
    let p = ps[1].clone();
    let fail = check("recolour", app, move || {
        p.set_attribute("style", "margin: 0; color: rgb(200, 0, 0)")
    });
    assert!(fail.is_none(), "{}", fail.unwrap());
}

/// A neighbour's damage over glyphs that overflow into it: a band above a
/// paragraph whose ascenders reach 6px past a gap into the band changes
/// colour. The paint prune must keep the paragraph (its box does not meet the
/// damage), or its ascenders are erased there.
#[test]
fn damage_over_a_neighbours_overflowing_glyphs_repaints_them() {
    const BAND: &str = "height: 20px; margin-bottom: 6px; background: rgb(0, 200, 0)";
    let (app, h) = mount(Box::new(|s, o| {
        let wrap = el(s, o, "padding: 30px");
        let band = el(s, &wrap, BAND);
        let p = el(s, &wrap, "font-size: 22px; line-height: 4px");
        text(s, &p, "Ascenders lbdh and descenders gjpqy");
        vec![band]
    }));
    let band = h[0].clone();
    let fail = check("neighbour background", app, move || {
        band.set_attribute("style", &BAND.replace("0, 200, 0", "0, 0, 200"))
    });
    assert!(fail.is_none(), "{}", fail.unwrap());
}

/// `nowrap` text past its 100px box: a block above it shrinks, so the line
/// moves up and its glyphs past the box must be cleared where they were.
#[test]
fn a_nowrap_line_that_overflows_its_box_is_cleared_when_it_moves() {
    let (app, h) = mount(Box::new(|s, o| {
        let above = el(
            s,
            o,
            "height: 50px; background: rgb(0, 200, 0); width: 100px",
        );
        let p = el(
            s,
            o,
            "width: 100px; white-space: nowrap; font-size: 16px; line-height: 20px",
        );
        text(s, &p, "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM");
        vec![above]
    }));
    let above = h[0].clone();
    let fail = check("line moves", app, move || {
        above.set_attribute(
            "style",
            "height: 10px; background: rgb(0, 200, 0); width: 100px",
        )
    });
    assert!(fail.is_none(), "{}", fail.unwrap());
}

/// #1632's own shape: the text is in an anonymous block box beside a block
/// child; the block goes, the anonymous box is freed and the line moves up to
/// the container's own lines.
#[test]
fn an_anonymous_box_freed_with_overflowing_text_is_cleared() {
    let (app, h) = mount(Box::new(|s, o| {
        let c = el(
            s,
            o,
            "width: 100px; white-space: nowrap; font-size: 16px; line-height: 20px",
        );
        let b = el(s, &c, "height: 50px; background: rgb(0, 200, 0)");
        text(s, &c, "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM");
        vec![b]
    }));
    let b = h[0].clone();
    let fail = check("anonymous box freed", app, move || b.remove());
    assert!(fail.is_none(), "{}", fail.unwrap());
}

/// A `nowrap` line past its box, recoloured: the glyphs past the box take the
/// new colour too.
#[test]
fn recolouring_a_nowrap_line_past_its_box_repaints_the_overflow() {
    let (app, h) = mount(Box::new(|s, o| {
        let p = el(
            s,
            o,
            "margin: 30px; width: 100px; white-space: nowrap; font-size: 16px; \
             line-height: 20px",
        );
        text(s, &p, "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM");
        vec![p]
    }));
    let p = h[0].clone();
    let fail = check("nowrap recolour", app, move || {
        p.set_attribute(
            "style",
            "margin: 30px; width: 100px; white-space: nowrap; font-size: 16px; \
             line-height: 20px; color: rgb(200, 0, 0)",
        )
    });
    assert!(fail.is_none(), "{}", fail.unwrap());
}

/// The `text-shadow` of a `nowrap` line past its box is cast from where the
/// glyphs are drawn, so recolouring the shadow repaints it past the box.
#[test]
fn the_shadow_of_a_nowrap_line_past_its_box_is_repainted() {
    const P: &str = "margin: 30px; width: 100px; white-space: nowrap; font-size: 16px; \
                     line-height: 20px; text-shadow: 3px 3px 0 rgb(0, 160, 0)";
    let (app, h) = mount(Box::new(|s, o| {
        let p = el(s, o, P);
        text(s, &p, "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM");
        vec![p]
    }));
    let p = h[0].clone();
    let fail = check("nowrap shadow", app, move || {
        p.set_attribute("style", &P.replace("0, 160, 0", "160, 0, 160"))
    });
    assert!(fail.is_none(), "{}", fail.unwrap());
}
