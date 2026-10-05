//! #349, second file: the fixtures PR #1383's review added.
//!
//! The first file changes the *size* of the frame and hole sets in every
//! fixture, which is a fixed point for a comparison by count; the two `a_swap_*`
//! fixtures here change membership at a constant count. The rest drive the
//! same path through shapes the first file does not have (a scrolled, clipped
//! and translated viewport; a viewport hidden while its surface goes; a node
//! removed and re-added; a rename; `data-viewport-ready`; a same-name swap).
//! The idle-turns scenario is `perf_regression_tests::idle_viewports_repaint_nothing`.
use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, ViewportFrames, collect_viewport_frames_by_name,
    create_render_surface_with_name, unregister_render_surface,
};
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::RefCell;

const SIZE: (u32, u32) = (800, 600);
const MAGENTA: [u8; 3] = [255, 0, 255];
const BLUE: [u8; 3] = [0, 0, 255];
const CARD: [u8; 3] = [255, 200, 0];

fn pixel_at(app: &RinchApp, x: u32, y: u32) -> [u8; 4] {
    let p = app.skia_painter.as_ref().expect("a software painter");
    let idx = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[idx], d[idx + 1], d[idx + 2], d[idx + 3]]
}
fn is(p: [u8; 4], rgb: [u8; 3]) -> bool {
    p[3] == 255
        && p[0].abs_diff(rgb[0]) < 6
        && p[1].abs_diff(rgb[1]) < 6
        && p[2].abs_diff(rgb[2]) < 6
}
fn submit(surface: &RenderSurfaceHandle, rgb: [u8; 3], w: u32, h: u32) {
    surface.writer().submit_frame(
        &[rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize),
        w,
        h,
    );
}
fn paint_with(app: &mut RinchApp, frames: ViewportFrames) -> FrameStats {
    app.install_viewport_frames(frames);
    app.build_pixels(1.0, SIZE, false);
    RinchApp::clear_viewport_frames();
    app.end_perf_frame().expect("a document is mounted")
}
fn paint(app: &mut RinchApp) -> FrameStats {
    paint_with(app, collect_viewport_frames_by_name())
}
fn settle(app: &mut RinchApp) {
    if app.has_pending_layout() {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    }
}

/// Mount `holder_style` card > viewport node(s). Returns app + handles.
fn mount_with(
    names: &'static [&'static str],
    outer_style: &'static str,
    holder_style: &'static str,
) -> (RinchApp, Vec<NodeHandle>, NodeHandle) {
    let out: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let outer_h: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let (o2, oh2) = (out.clone(), outer_h.clone());
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");
        let outer = scope.create_element("div");
        outer.set_attribute("style", outer_style);
        for name in names {
            let holder = scope.create_element("div");
            holder.set_attribute("style", holder_style);
            let viewport = scope.create_element("div");
            viewport.set_attribute("style", "width: 100%; height: 100%;");
            viewport.set_attribute("data-viewport", name);
            holder.append_child(&viewport);
            outer.append_child(&holder);
            o2.borrow_mut().push(viewport);
            o2.borrow_mut().push(holder);
        }
        root.append_child(&outer);
        *oh2.borrow_mut() = Some(outer);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let v = out.borrow().clone();
    let o = outer_h.borrow().clone().unwrap();
    (app, v, o)
}

const HOLDER: &str = "width: 400px; height: 200px; overflow: hidden; \
                      background-color: rgb(255, 200, 0);";

/// the viewport is inside a scrolled scroller, partly clipped, and
/// translated. Old pixels gone, damage is the visible part only.
#[test]
fn a_scrolled_clipped_transformed_viewport_gives_its_visible_box_back() {
    static NAMES: [&str; 1] = ["rv-scroll"];
    let (mut app, _, outer) = mount_with(
        &NAMES,
        "position: absolute; left: 50px; top: 40px; width: 300px; height: 150px; overflow: auto; \
         background-color: rgb(0, 128, 0);",
        "margin-top: 100px; width: 400px; height: 200px; overflow: hidden; \
         transform: translate(10px, 0px); background-color: rgb(255, 200, 0);",
    );
    outer.set_scroll_top(70.0);
    settle(&mut app);
    let game = create_render_surface_with_name("rv-scroll");
    submit(&game, MAGENTA, 40, 20);
    paint(&mut app);
    // holder top in window = 40 + 100 - 70 = 70; left = 60. visible: x 60..350, y 70..190
    assert!(
        is(pixel_at(&app, 200, 130), MAGENTA),
        "control: {:?}",
        pixel_at(&app, 200, 130)
    );
    let left_before = pixel_at(&app, 55, 130);
    let below_before = pixel_at(&app, 200, 300);

    unregister_render_surface(game.id());
    let stats = paint(&mut app);
    for (x, y) in [(200, 130), (61, 71), (348, 188)] {
        assert!(
            is(pixel_at(&app, x, y), CARD),
            "({x},{y}) {:?} {stats:?}",
            pixel_at(&app, x, y)
        );
    }
    eprintln!(
        "P4 left before {left_before:?} after {:?}; stats {stats:?}",
        pixel_at(&app, 55, 130)
    );
    assert_eq!(pixel_at(&app, 200, 300), below_before);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    let px = stats.get(Counter::RepaintedPx);
    assert!(
        px > 0 && px <= 300 * 150,
        "no more than the scroller's box: {px} {stats:?}"
    );
}

/// the surface goes while the viewport is display:none; shown later.
#[test]
fn a_surface_that_goes_while_its_viewport_is_hidden_leaves_nothing_when_shown() {
    static NAMES: [&str; 1] = ["rv-hidden"];
    let (mut app, _, outer) =
        mount_with(&NAMES, "position: absolute; left: 20px; top: 30px;", HOLDER);
    let game = create_render_surface_with_name("rv-hidden");
    submit(&game, MAGENTA, 40, 20);
    paint(&mut app);
    assert!(is(pixel_at(&app, 200, 130), MAGENTA), "control");
    outer.set_style("display", "none");
    settle(&mut app);
    paint(&mut app);
    assert!(!is(pixel_at(&app, 200, 130), MAGENTA), "hidden");
    unregister_render_surface(game.id());
    paint(&mut app);
    outer.set_style("display", "block");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, 200, 130), CARD),
        "{:?} {stats:?}",
        pixel_at(&app, 200, 130)
    );
}

/// node removed and re-added while the surface stays.
#[test]
fn a_viewport_node_removed_and_re_added_shows_its_frame_again() {
    static NAMES: [&str; 1] = ["rv-readd"];
    let (mut app, vps, outer) =
        mount_with(&NAMES, "position: absolute; left: 20px; top: 30px;", HOLDER);
    let game = create_render_surface_with_name("rv-readd");
    submit(&game, MAGENTA, 40, 20);
    paint(&mut app);
    let holder = vps[1].clone();
    holder.remove();
    settle(&mut app);
    paint(&mut app);
    assert!(!is(pixel_at(&app, 200, 130), MAGENTA), "removed");
    outer.append_child(&holder);
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, 200, 130), MAGENTA),
        "{:?} {stats:?}",
        pixel_at(&app, 200, 130)
    );
    let _ = vps;
    unregister_render_surface(game.id());
}

/// the node is renamed to a name with no surface (a DOM write, the set is
/// unchanged). Pre-existing behaviour probe.
#[test]
fn renaming_the_viewport_node_away_from_its_surface() {
    static NAMES: [&str; 1] = ["rv-rename"];
    let (mut app, vps, _) =
        mount_with(&NAMES, "position: absolute; left: 20px; top: 30px;", HOLDER);
    let game = create_render_surface_with_name("rv-rename");
    submit(&game, MAGENTA, 40, 20);
    paint(&mut app);
    assert!(is(pixel_at(&app, 200, 130), MAGENTA), "control");
    vps[0].set_attribute("data-viewport", "rv-rename-nobody");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        !is(pixel_at(&app, 200, 130), MAGENTA),
        "the frame of a surface this node no longer names is still on screen ({stats:?})"
    );
    unregister_render_surface(game.id());
}

/// data-viewport-ready flips to "false" under a punching game frame.
#[test]
fn ready_false_fills_the_hole() {
    static NAMES: [&str; 1] = ["rv-ready"];
    let (mut app, vps, _) = mount_with(
        &NAMES,
        "position: absolute; left: 20px; top: 30px;",
        "width: 400px; height: 100px; overflow: hidden; background-color: rgb(255, 200, 0);",
    );
    let game = create_render_surface_with_name("rv-ready");
    submit(&game, MAGENTA, 40, 20);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, 60, 80), [255, 255, 255]),
        "control: bar is the hole {:?}",
        pixel_at(&app, 60, 80)
    );
    vps[0].set_attribute("data-viewport-ready", "false");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        !is(pixel_at(&app, 60, 80), [255, 255, 255]),
        "the hole is still cut after ready=false ({stats:?}) {:?}",
        pixel_at(&app, 60, 80)
    );
    unregister_render_surface(game.id());
}

/// a surface whose buffer is replaced by a same-name re-registration in
/// one turn (old out, new in with a frame nobody saw as fresh).
#[test]
fn same_name_swap_between_paints() {
    static NAMES: [&str; 1] = ["rv-swap"];
    let (mut app, _, _) = mount_with(&NAMES, "position: absolute; left: 20px; top: 30px;", HOLDER);
    let old = create_render_surface_with_name("rv-swap");
    submit(&old, MAGENTA, 40, 20);
    paint(&mut app);
    unregister_render_surface(old.id());
    let new = create_render_surface_with_name("rv-swap");
    submit(&new, BLUE, 40, 20);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, 200, 130), BLUE),
        "{:?} {stats:?}",
        pixel_at(&app, 200, 130)
    );
    unregister_render_surface(new.id());
}

/// Kills M2 (frames compared by count): one viewport loses its frame in the
/// same turn another gains its first, so the count stays 1.
#[test]
fn a_swap_of_which_viewport_has_a_frame_at_constant_count_is_a_change() {
    static NAMES: [&str; 2] = ["rv-swap-a", "rv-swap-b"];
    let (mut app, _, _) = mount_with(
        &NAMES,
        "",
        "width: 300px; height: 100px; overflow: hidden; background-color: rgb(255, 200, 0);",
    );
    // Video: in no hole set, so only the frame map can say it.
    let a = crate::render_surface::create_video_surface("rv-swap-a");
    let b = crate::render_surface::create_video_surface("rv-swap-b");
    submit(&a, MAGENTA, 60, 20);
    paint(&mut app);
    assert!(is(pixel_at(&app, 150, 50), MAGENTA), "control: a shown");
    assert!(!is(pixel_at(&app, 150, 150), BLUE), "control: b empty");

    unregister_render_surface(a.id());
    submit(&b, BLUE, 60, 20);
    let stats = paint(&mut app);
    assert!(is(pixel_at(&app, 150, 150), BLUE), "b's first frame");
    assert!(
        !is(pixel_at(&app, 150, 50), MAGENTA),
        "a's dead frame is still on screen: {:?} ({stats:?})",
        pixel_at(&app, 150, 50)
    );
    unregister_render_surface(b.id());
}

/// Kills M3 (holes compared by count): {a} -> {b} with both frames unchanged.
#[test]
fn a_swap_of_which_viewport_punches_at_constant_count_is_a_change() {
    static NAMES: [&str; 2] = ["rv-hswap-a", "rv-hswap-b"];
    // 4:1 boxes and 2:1 frames: 100px bars either side; the bar is the hole.
    let (mut app, _, _) = mount_with(
        &NAMES,
        "",
        "width: 400px; height: 100px; overflow: hidden; background-color: rgb(255, 200, 0);",
    );
    let a = create_render_surface_with_name("rv-hswap-a");
    let b = create_render_surface_with_name("rv-hswap-b");
    submit(&a, MAGENTA, 40, 20);
    submit(&b, MAGENTA, 40, 20);
    let only = |n: &str| {
        let mut f = collect_viewport_frames_by_name();
        f.holes.retain(|h| h == n);
        f
    };
    paint_with(&mut app, only("rv-hswap-a"));
    assert!(
        is(pixel_at(&app, 40, 50), [255, 255, 255]),
        "control: a punches"
    );
    assert!(
        is(pixel_at(&app, 40, 150), [0, 0, 0]),
        "control: b does not"
    );
    let f = only("rv-hswap-b");
    assert!(f.fresh.is_empty(), "control: nothing fresh");
    let stats = paint_with(&mut app, f);
    assert!(
        is(pixel_at(&app, 40, 50), [0, 0, 0]),
        "a stopped: {:?} {stats:?}",
        pixel_at(&app, 40, 50)
    );
    assert!(
        is(pixel_at(&app, 40, 150), [255, 255, 255]),
        "b started: {:?}",
        pixel_at(&app, 40, 150)
    );
    unregister_render_surface(a.id());
    unregister_render_surface(b.id());
}
