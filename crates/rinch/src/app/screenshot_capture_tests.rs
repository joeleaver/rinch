//! Issue #364: a software debug screenshot must capture what the screen shows.
//!
//! The debug `screenshot` command runs `paint()` and then captures. On the
//! software path the capture used to call `build_pixels` a second time with no
//! frame map installed — no `SURFACE_PIXELS`, no `VIEWPORT_PIXELS`. Whenever
//! the scene was dirty again by then, that second paint drew every
//! `RenderSurface` and inline video as an empty box, and it drew it into the
//! painter's own buffer — the one the next frame presents from — so the PNG
//! and the live buffer both lost the surface's content.
//!
//! These fixtures drive the shell's sequence: a frame painted exactly the way
//! `paint_software` paints it (frame map installed, surface node marked), then
//! the scene dirtied again, then `RinchApp::screenshot_pixels` — the call
//! `capture_screenshot_impl` makes.

use super::*;
use rinch_dom::paint::SurfacePixelData;
use std::collections::{HashMap, HashSet};

const SIZE: (u32, u32) = (400, 300);
const SURFACE_ID: usize = 7;
/// Deliberately not white, black or transparent — the colours an empty
/// surface box paints as — so a lost frame cannot pass for a kept one.
const MAGENTA: [u8; 3] = [255, 0, 255];

fn solid(rgb: [u8; 3], w: u32, h: u32) -> SurfacePixelData {
    SurfacePixelData {
        data: [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize),
        width: w,
        height: h,
    }
}

fn pixel(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
}

fn is(p: [u8; 4], rgb: [u8; 3]) -> bool {
    p[3] == 255
        && p[0].abs_diff(rgb[0]) < 6
        && p[1].abs_diff(rgb[1]) < 6
        && p[2].abs_diff(rgb[2]) < 6
}

/// A 200×100 box at (40, 50) carrying `attr = value`, on a white page.
fn mount(attr: &'static str, value: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "width: 400px; height: 300px; background-color: white;",
        );
        let surface = scope.create_element("div");
        surface.set_attribute(
            "style",
            "position: absolute; left: 40px; top: 50px; width: 200px; height: 100px; \
             background: transparent;",
        );
        surface.set_attribute(attr, value);
        if attr == "data-viewport" {
            surface.set_attribute("data-viewport-ready", "true");
        }
        root.append_child(&surface);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app
}

/// One `paint_software` frame for a `RenderSurface` with a submitted frame.
fn paint_surface_frame(app: &mut RinchApp) {
    rinch_dom::paint::set_active_viewports(Some(HashSet::new()));
    app.request_repaint();
    app.mark_surface_nodes_paint_dirty(&[SURFACE_ID]);
    rinch_dom::paint::set_surface_pixels(Some(HashMap::from([(
        SURFACE_ID,
        solid(MAGENTA, 20, 10),
    )])));
    app.build_pixels(1.0, SIZE, false);
    rinch_dom::paint::set_surface_pixels(None);
    rinch_dom::paint::set_active_viewports(None);
}

/// One `paint_software` frame for an inline video with a decoded frame.
fn paint_video_frame(app: &mut RinchApp) {
    rinch_dom::paint::set_active_viewports(Some(HashSet::new()));
    app.request_repaint();
    app.mark_viewport_nodes_paint_dirty(&["v"]);
    rinch_dom::paint::set_viewport_pixels(Some(HashMap::from([(
        "v".to_string(),
        solid(MAGENTA, 20, 10),
    )])));
    app.build_pixels(1.0, SIZE, false);
    rinch_dom::paint::set_viewport_pixels(None);
    rinch_dom::paint::set_active_viewports(None);
}

fn live_pixel(app: &RinchApp, x: u32, y: u32) -> [u8; 4] {
    let p = app.skia_painter.as_ref().expect("a software painter");
    pixel(p.pixels(), p.width(), x, y)
}

/// Capture after the scene went dirty again, and check both the capture and
/// the buffer the next frame presents from.
fn capture_keeps_the_frame(mut app: RinchApp, what: &str) {
    assert!(
        is(live_pixel(&app, 140, 100), MAGENTA),
        "positive control: the {what} frame reached the buffer, got {:?}",
        live_pixel(&app, 140, 100)
    );
    // Off the surface: the page, so the capture is not simply a solid fill.
    assert!(is(live_pixel(&app, 300, 250), [255, 255, 255]));

    // Something dirties the scene between the paint and the capture — the
    // video controls' timestamp tick, a queued main-thread callback.
    app.mark_scene_dirty();

    let (pixels, w, h) = app.screenshot_pixels(1.0, SIZE);
    assert_eq!((w, h), SIZE);
    let shot = pixel(pixels, w, 140, 100);
    let page = pixel(pixels, w, 300, 250);
    assert!(
        is(shot, MAGENTA),
        "the screenshot shows the {what}'s frame, not an empty box (#364), got {shot:?}"
    );
    assert!(is(page, [255, 255, 255]), "and the page around it, got {page:?}");
    assert!(
        is(live_pixel(&app, 140, 100), MAGENTA),
        "the capture left the live buffer's {what} frame in place (#364), got {:?}",
        live_pixel(&app, 140, 100)
    );
}

#[test]
fn a_screenshot_of_a_dirty_scene_keeps_a_render_surfaces_frame() {
    let mut app = mount("data-render-surface", "7");
    paint_surface_frame(&mut app);
    capture_keeps_the_frame(app, "RenderSurface");
}

#[test]
fn a_screenshot_of_a_dirty_scene_keeps_an_inline_videos_frame() {
    let mut app = mount("data-viewport", "v");
    paint_video_frame(&mut app);
    capture_keeps_the_frame(app, "video");
}

/// The capture does not consume the dirt it found: the next real frame still
/// paints (with its frame map), so nothing the screenshot skipped is lost.
#[test]
fn a_screenshot_leaves_the_scene_dirty_for_the_next_frame() {
    let mut app = mount("data-render-surface", "7");
    paint_surface_frame(&mut app);
    app.mark_scene_dirty();
    let _ = app.screenshot_pixels(1.0, SIZE);
    assert!(
        app.scene_dirty,
        "the scene is still dirty: the next paint_software repaints it"
    );
}

/// Before any frame was painted there is nothing on screen to capture; the
/// screenshot still answers a full-size picture of the document.
#[test]
fn a_screenshot_before_the_first_frame_paints_the_document() {
    let mut app = mount("data-render-surface", "7");
    let (pixels, w, h) = app.screenshot_pixels(1.0, SIZE);
    assert_eq!((w, h), SIZE);
    assert!(is(pixel(pixels, w, 300, 250), [255, 255, 255]));
}
