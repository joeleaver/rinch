//! Reviewer probes for PR #1699: hit testing vs paint, and partial-frame
//! damage, for descendants of `0x0` boxes.

use super::hit_testing::hit_test;
use super::*;
use rinch_core::dom::DomDocument;

const VP: (u32, u32) = (400, 300);

fn app_with(html: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "background-color: rgb(255, 255, 255); width: 400px; height: 300px",
        );
        root.set_inner_html(html);
        root
    });
    app.mount_component(VP.0 as f32, VP.1 as f32);
    for _ in 0..3 {
        frame(&mut app);
    }
    app
}

fn frame(app: &mut RinchApp) -> (Vec<u8>, rinch_dom::perf::FrameStats) {
    let _ = app.handle_event(PlatformEvent::AboutToWait, VP, 1.0);
    app.resolve_and_repaint(VP.0 as f32, VP.1 as f32);
    let px = app.build_pixels(1.0, VP, false).0.to_vec();
    (px, app.end_perf_frame().expect("mounted"))
}

fn count(px: &[u8], (x0, x1, y0, y1): (u32, u32, u32, u32), rgb: [u8; 3]) -> usize {
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * VP.0 + x) * 4) as usize;
            if px[i..i + 3] == rgb && px[i + 3] == 255 {
                n += 1;
            }
        }
    }
    n
}

fn id(app: &RinchApp, sel: &str) -> u64 {
    app.doc.as_ref().unwrap().borrow().query_selector_all(sel)[0].0
}

fn hit(app: &RinchApp, x: f32, y: f32) -> Option<u64> {
    let doc = app.doc.as_ref().unwrap().borrow();
    hit_test(&doc.tree, x, y)
}

fn repaints(s: &rinch_dom::perf::FrameStats) -> (u64, u64) {
    (
        s.get(rinch_dom::perf::Counter::RepaintPartial),
        s.get(rinch_dom::perf::Counter::RepaintFull),
    )
}

const G: [u8; 3] = [0, 128, 0];
const R: [u8; 3] = [255, 0, 0];

/// Paint clips a `0x0` `overflow: hidden` box's child away; is the child hit?
#[test]
fn h1_zero_overflow_hidden_child_paint_vs_hit() {
    let app = app_with(
        "<div style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0; overflow: hidden\">\
         <div data-c style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let c = id(&app, "[data-c]");
    let h = hit(&app, 60.0, 60.0);
    eprintln!("PROBE h1 hit at child center = {h:?} (child={c})");
    assert_ne!(
        h,
        Some(c),
        "paint clips the child away, so it must not be hit"
    );
}

/// An inline-block marker in a `0x0` stacking context: hit vs paint.
#[test]
fn h2_inline_block_marker_in_zero_sc_paint_vs_hit() {
    let mut app = app_with(
        "<div style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0; z-index: 2\">\
         <span data-c style=\"display: inline-block; width: 30px; height: 30px; background-color: rgb(0,128,0)\"></span></div>",
    );
    let c = id(&app, "[data-c]");
    let (px, _) = frame(&mut app);
    let g = count(&px, (0, 400, 0, 300), G);
    let h = hit(&app, 60.0, 60.0);
    eprintln!("PROBE h2 painted green={g}, hit at (60,60) = {h:?} (marker={c})");
    assert_eq!(g, 900, "Chrome paints the marker");
}

/// A `0x0` stacking context inside a scrolled scroller.
#[test]
fn h3_zero_sc_in_scroller() {
    let mut app = app_with(
        "<div data-s style=\"position: relative; width: 200px; height: 100px; overflow: auto\">\
         <div style=\"height: 400px\"></div>\
         <div style=\"position: absolute; left: 40px; top: 150px; width: 0; height: 0; z-index: 3\">\
         <div data-c style=\"position: absolute; left: 10px; top: 5px; width: 20px; height: 20px; background-color: rgb(0,128,0)\"></div></div></div>",
    );
    let s = id(&app, "[data-s]");
    let c = id(&app, "[data-c]");
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_scroll_top(rinch_core::dom::NodeId(s), 100.0);
    let (px, _) = frame(&mut app);
    // child at scroller-content (50, 155) -> viewport (50, 55)
    let g = count(&px, (50, 70, 55, 75), G);
    let all = count(&px, (0, 400, 0, 300), G);
    let h = hit(&app, 60.0, 65.0);
    eprintln!("PROBE h3 green at expected={g} total={all} hit={h:?} child={c}");
    assert_eq!((g, all), (400, 400));
    assert_eq!(h, Some(c));
}

/// In-flow child of a static `0x0` box changes colour: partial frame repaints it.
#[test]
fn d1_in_flow_child_colour_change_partial() {
    let mut app = app_with(
        "<div style=\"padding: 40px 0 0 60px\"><div style=\"width: 0; height: 0\">\
         <div data-c style=\"width: 30px; height: 20px; background-color: rgb(0,128,0)\"></div></div></div>",
    );
    let c = id(&app, "[data-c]");
    let (px, _) = frame(&mut app);
    assert_eq!(count(&px, (60, 90, 40, 60), G), 600, "drawn before");
    app.doc.as_ref().unwrap().borrow_mut().set_style(
        rinch_core::dom::NodeId(c),
        "background-color",
        "rgb(255, 0, 0)",
    );
    let (px, st) = frame(&mut app);
    eprintln!(
        "PROBE d1 repaints={:?} red={} green={}",
        repaints(&st),
        count(&px, (60, 90, 40, 60), R),
        count(&px, (0, 400, 0, 300), G)
    );
    assert_eq!(repaints(&st), (1, 0));
    assert_eq!(count(&px, (60, 90, 40, 60), R), 600);
    assert_eq!(count(&px, (0, 400, 0, 300), G), 0);
}

/// The `0x0` box's own transform changes: its child moves on a partial frame.
#[test]
fn d2_zero_box_transform_change_moves_child() {
    let mut app = app_with(
        "<div data-b style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0; z-index: 2\">\
         <div style=\"position: absolute; left: 10px; top: 10px; width: 20px; height: 20px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let b = id(&app, "[data-b]");
    let (px, _) = frame(&mut app);
    assert_eq!(count(&px, (60, 80, 60, 80), G), 400, "drawn before");
    app.doc.as_ref().unwrap().borrow_mut().set_style(
        rinch_core::dom::NodeId(b),
        "transform",
        "translate(100px, 70px)",
    );
    let (px, st) = frame(&mut app);
    let at_new = count(&px, (160, 180, 130, 150), G);
    let at_old = count(&px, (60, 80, 60, 80), G);
    eprintln!(
        "PROBE d2 repaints={:?} new={at_new} old={at_old}",
        repaints(&st)
    );
    assert_eq!((at_new, at_old), (400, 0));
}

/// A `0x0` `overflow: hidden` box becomes `visible`: the child appears.
#[test]
fn d3_zero_box_unclips() {
    let mut app = app_with(
        "<div data-b style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0; overflow: hidden\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let b = id(&app, "[data-b]");
    let (px, _) = frame(&mut app);
    assert_eq!(count(&px, (0, 400, 0, 300), G), 0, "clipped before");
    app.doc.as_ref().unwrap().borrow_mut().set_style(
        rinch_core::dom::NodeId(b),
        "overflow",
        "visible",
    );
    let (px, st) = frame(&mut app);
    let g = count(&px, (50, 80, 50, 80), G);
    eprintln!("PROBE d3 repaints={:?} green={g}", repaints(&st));
    assert_eq!(g, 900);
}

/// Opacity of the `0x0` box changes 1 -> 0.5 on a partial frame.
#[test]
fn d4_zero_box_opacity_change() {
    let mut app = app_with(
        "<div data-b style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0\">\
         <div style=\"width: 30px; height: 30px; background-color: rgb(0,128,0)\"></div></div>",
    );
    let b = id(&app, "[data-b]");
    let (px, _) = frame(&mut app);
    assert_eq!(count(&px, (50, 80, 50, 80), G), 900);
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .set_style(rinch_core::dom::NodeId(b), "opacity", "0.5");
    let (px, st) = frame(&mut app);
    let g = count(&px, (0, 400, 0, 300), G);
    eprintln!(
        "PROBE d4 repaints={:?} still-opaque-green={g}",
        repaints(&st)
    );
    assert_eq!(g, 0, "faded");
}

/// (Needs the reviewer's text candidate.) A `0x0` IFC root's text changes
/// colour on a partial frame: is the overflowing text repainted?
#[test]
fn d5_zero_ifc_root_text_colour_change() {
    let mut app = app_with(
        "<div data-b style=\"position: absolute; left: 50px; top: 50px; width: 0; height: 0; \
         font-size: 40px; line-height: 40px; color: rgb(0,128,0)\">HH</div>",
    );
    let b = id(&app, "[data-b]");
    let (px, _) = frame(&mut app);
    let g0 = count(&px, (0, 400, 0, 300), G);
    app.doc.as_ref().unwrap().borrow_mut().set_style(
        rinch_core::dom::NodeId(b),
        "color",
        "rgb(255, 0, 0)",
    );
    let (px, st) = frame(&mut app);
    let (g1, r1) = (
        count(&px, (0, 400, 0, 300), G),
        count(&px, (0, 400, 0, 300), R),
    );
    eprintln!(
        "PROBE d5 before green={g0}; after repaints={:?} green={g1} red={r1}",
        repaints(&st)
    );
    assert!(g0 > 50);
    assert_eq!(g1, 0, "old colour gone");
    assert!(r1 > 50, "new colour drawn");
}
