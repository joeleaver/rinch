//! #2221: a blended background (`background-blend-mode`) on partial frames.
//! Each frame is asserted partial, then compared with a from-scratch frame
//! over the whole surface, with a positive control that the frame changed
//! pixels.
//!
//! - Changing the blend mode repaints the box (a restyle pushes it
//!   paint-dirty) with the new mode.
//! - A box drawn over part of a blended one changes colour: the partial
//!   frame repaints the blended background under the damage clip only, and
//!   its isolated group and blend layer composite there exactly as in a full
//!   frame — under the box's opacity, its rounded corners, a rotation and an
//!   ancestor's clip, at 1x and at a fractional scale.
use super::*;
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::RefCell;

const W: u32 = 600;
const H: u32 = 400;

const BLENDED: &str = "position: absolute; left: 37px; top: 23px; width: 300px; height: 200px; \
    background-image: linear-gradient(to right, rgb(51, 153, 230), rgba(240, 40, 90, 0.6)); \
    background-color: rgb(204, 102, 51); background-blend-mode: multiply";

struct Fixture {
    app: RinchApp,
    blended: NodeHandle,
    over: NodeHandle,
}

fn mount(extra: &str) -> Fixture {
    let slot: Rc<RefCell<Option<(NodeHandle, NodeHandle)>>> = Rc::new(RefCell::new(None));
    let out = slot.clone();
    let extra = extra.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 600px; height: 400px; background: rgb(250, 240, 200);",
        );
        let clip = scope.create_element("div");
        clip.set_attribute(
            "style",
            "position: absolute; left: 11px; top: 9px; width: 320px; height: 210px; overflow: hidden;",
        );
        let blended = scope.create_element("div");
        blended.set_attribute("style", &format!("{BLENDED}; {extra}"));
        clip.append_child(&blended);
        root.append_child(&clip);
        let over = scope.create_element("div");
        over.set_attribute(
            "style",
            "position: absolute; left: 141px; top: 83px; width: 57px; height: 33px; background: rgb(0, 200, 0);",
        );
        root.append_child(&over);
        *out.borrow_mut() = Some((blended, over));
        root
    });
    app.mount_component(W as f32, H as f32);
    let (blended, over) = slot.borrow_mut().take().unwrap();
    Fixture { app, blended, over }
}

fn size(scale: f64) -> (u32, u32) {
    ((W as f64 * scale) as u32, (H as f64 * scale) as u32)
}

fn frame(app: &mut RinchApp, scale: f64) -> (Vec<u8>, FrameStats) {
    app.resolve_and_repaint(W as f32, H as f32);
    let px = app.build_pixels(scale, size(scale), false).0.to_vec();
    (px, app.end_perf_frame().expect("mounted"))
}

fn full_frame(app: &mut RinchApp, scale: f64) -> Vec<u8> {
    app.scene_dirty = true;
    app.has_previous_frame = false;
    let px = app.build_pixels(scale, size(scale), false).0.to_vec();
    let _ = app.end_perf_frame();
    px
}

fn ndiff(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x.iter().zip(y.iter()).any(|(p, q)| p != q))
        .count()
}

/// Settle, apply `change`, and check the next frame: partial, changed, and
/// byte for byte what a full frame paints.
fn check(extra: &str, scale: f64, change: impl FnOnce(&Fixture)) {
    let mut f = mount(extra);
    frame(&mut f.app, scale);
    frame(&mut f.app, scale);
    let (before, _) = frame(&mut f.app, scale);
    change(&f);
    let (after, stats) = frame(&mut f.app, scale);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert!(ndiff(&before, &after) > 0, "positive control");
    let full = full_frame(&mut f.app, scale);
    assert_eq!(
        ndiff(&after, &full),
        0,
        "{extra} @ {scale}: partial != full"
    );
}

#[test]
fn a_blend_mode_change_repaints_the_box() {
    for scale in [1.0, 1.25] {
        check("", scale, |f| {
            f.blended.set_style("background-blend-mode", "difference");
        });
    }
}

#[test]
fn a_partial_frame_over_a_blended_background_matches_a_full_one() {
    for extra in [
        "",
        "opacity: 0.7; border-radius: 24px",
        "transform: rotate(3deg)",
    ] {
        for scale in [1.0, 1.25] {
            check(extra, scale, |f| {
                f.over.set_style("background", "rgb(0, 0, 0)");
            });
        }
    }
}
