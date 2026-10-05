//! #348: a viewport that says it is not ready shows no frame.
//!
//! `data-viewport-ready="false"` already kept paint from cutting the hole
//! (#186). The software backend's inline arm did not read it, so a video that
//! errored mid-stream kept its last frame on screen over the placeholder its
//! node declares, where the GPU compositor's layer sat hidden under that
//! placeholder. One DOM, two screens.
//!
//! Every fixture here uses a placeholder that is neither black (what the
//! inline arm fills under a video frame) nor the frame's colour, and a frame
//! narrower than its box, so the frame, the letterbox and the placeholder are
//! three different answers at the two probed points.
use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, collect_viewport_frames_by_name, create_render_surface_with_name,
    create_video_surface, unregister_render_surface,
};
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::RefCell;

const SIZE: (u32, u32) = (800, 600);
const MAGENTA: [u8; 3] = [255, 0, 255];
const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
/// The viewport node's own background: the placeholder.
const GREEN: [u8; 3] = [0, 128, 0];
/// The holder around it.
const CARD: [u8; 3] = [255, 200, 0];

/// The holder is 400x200 at (20, 30). A square frame is fitted into the
/// middle 200px of it, so `FRAME` is on the frame and `BAR` on its letterbox.
const FRAME: (u32, u32) = (220, 130);
const BAR: (u32, u32) = (40, 130);

fn pixel_at(app: &RinchApp, (x, y): (u32, u32)) -> [u8; 4] {
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
fn submit_square(surface: &RenderSurfaceHandle) {
    surface
        .writer()
        .submit_frame(&[255, 0, 255, 255].repeat(20 * 20), 20, 20);
}
fn paint(app: &mut RinchApp) -> FrameStats {
    app.install_viewport_frames(collect_viewport_frames_by_name());
    app.build_pixels(1.0, SIZE, false);
    RinchApp::clear_viewport_frames();
    app.end_perf_frame().expect("a document is mounted")
}
fn settle(app: &mut RinchApp) {
    if app.has_pending_layout() {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    }
}

/// `card (400x200 at 20,30) > viewport`, the viewport carrying
/// `viewport_style` on top of filling the card.
fn mount(name: &'static str, viewport_style: &'static str) -> (RinchApp, NodeHandle) {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let o2 = out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");
        let holder = scope.create_element("div");
        holder.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 30px; width: 400px; height: 200px; \
             overflow: hidden; background-color: rgb(255, 200, 0);",
        );
        let viewport = scope.create_element("div");
        viewport.set_attribute(
            "style",
            &format!("width: 100%; height: 100%; {viewport_style}"),
        );
        viewport.set_attribute("data-viewport", name);
        holder.append_child(&viewport);
        root.append_child(&holder);
        *o2.borrow_mut() = Some(viewport);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let v = out.borrow().clone().unwrap();
    (app, v)
}

const PLACEHOLDER: &str = "background-color: rgb(0, 128, 0);";

/// The issue's own case: a video with a frame on screen goes not-ready (an
/// error mid-stream). Its node's placeholder is what shows, and the flip is a
/// partial repaint of the viewport's box.
#[test]
fn a_video_that_goes_not_ready_shows_its_placeholder_not_its_last_frame() {
    let (mut app, vp) = mount("v348-err", PLACEHOLDER);
    let video = create_video_surface("v348-err");
    submit_square(&video);
    paint(&mut app);
    assert!(is(pixel_at(&app, FRAME), MAGENTA), "control: the frame");
    assert!(is(pixel_at(&app, BAR), BLACK), "control: the letterbox");

    vp.set_attribute("data-viewport-ready", "false");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, FRAME), GREEN),
        "a not-ready video still shows its last frame: {:?}",
        pixel_at(&app, FRAME)
    );
    assert!(
        is(pixel_at(&app, BAR), GREEN),
        "a not-ready video still shows its letterbox: {:?}",
        pixel_at(&app, BAR)
    );
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");
}

/// The other direction: the frame comes back when the node says ready again,
/// with no new frame submitted, and again as a partial repaint.
#[test]
fn a_video_that_becomes_ready_again_shows_the_frame_it_has() {
    let (mut app, vp) = mount("v348-back", PLACEHOLDER);
    vp.set_attribute("data-viewport-ready", "false");
    let video = create_video_surface("v348-back");
    submit_square(&video);
    settle(&mut app);
    paint(&mut app);
    assert!(is(pixel_at(&app, FRAME), GREEN), "control: the placeholder");

    vp.set_attribute("data-viewport-ready", "true");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, FRAME), MAGENTA),
        "{:?}",
        pixel_at(&app, FRAME)
    );
    assert!(is(pixel_at(&app, BAR), BLACK), "{:?}", pixel_at(&app, BAR));
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");
}

/// A node that carries the attribute has to say exactly `"true"`, the rule the
/// hole already follows: a mis-stamped value shows the placeholder.
#[test]
fn a_mis_stamped_value_is_not_ready() {
    for value in ["yes", "1", "TRUE", ""] {
        let (mut app, vp) = mount("v348-bad", PLACEHOLDER);
        vp.set_attribute("data-viewport-ready", value);
        let video = create_video_surface("v348-bad");
        submit_square(&video);
        settle(&mut app);
        paint(&mut app);
        assert!(
            is(pixel_at(&app, FRAME), GREEN),
            "{value:?}: {:?}",
            pixel_at(&app, FRAME)
        );
        unregister_render_surface(video.id());
    }
}

/// Absence means ready (`GameViewport` stamps nothing), and so does `"true"`.
#[test]
fn absence_and_true_are_ready() {
    for value in [None, Some("true")] {
        let (mut app, vp) = mount("v348-ok", PLACEHOLDER);
        if let Some(v) = value {
            vp.set_attribute("data-viewport-ready", v);
        }
        let video = create_video_surface("v348-ok");
        submit_square(&video);
        settle(&mut app);
        paint(&mut app);
        assert!(
            is(pixel_at(&app, FRAME), MAGENTA),
            "{value:?}: {:?}",
            pixel_at(&app, FRAME)
        );
        unregister_render_surface(video.id());
    }
}

/// A game viewport that says not-ready: no hole and no frame. Its own
/// background is transparent, so what shows is the card it sits in.
#[test]
fn a_game_viewport_that_goes_not_ready_shows_neither_hole_nor_frame() {
    let (mut app, vp) = mount("g348", "");
    let game = create_render_surface_with_name("g348");
    submit_square(&game);
    paint(&mut app);
    assert!(is(pixel_at(&app, FRAME), MAGENTA), "control: the frame");
    assert!(is(pixel_at(&app, BAR), WHITE), "control: the hole");

    vp.set_attribute("data-viewport-ready", "false");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, FRAME), CARD),
        "{:?}",
        pixel_at(&app, FRAME)
    );
    assert!(is(pixel_at(&app, BAR), CARD), "{:?}", pixel_at(&app, BAR));
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");
    unregister_render_surface(game.id());
}

/// `RinchApp::viewport_ready` is what the GPU shell gates its compositor
/// layers on, and it is paint's rule.
#[test]
fn viewport_ready_answers_what_paint_reads() {
    let (mut app, vp) = mount("a348", PLACEHOLDER);
    settle(&mut app);
    assert!(app.viewport_ready("a348"), "absence means ready");
    vp.set_attribute("data-viewport-ready", "false");
    assert!(!app.viewport_ready("a348"));
    vp.set_attribute("data-viewport-ready", "yes");
    assert!(!app.viewport_ready("a348"), "only \"true\" is ready");
    vp.set_attribute("data-viewport-ready", "true");
    assert!(app.viewport_ready("a348"));
    assert!(
        app.viewport_ready("a348-nobody"),
        "a name no node carries has no node to say otherwise"
    );
}
