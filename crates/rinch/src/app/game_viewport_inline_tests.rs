//! #361: a `GameViewport` on the software backend paints **inline**, in paint
//! order, like video (#358) — so an overlay above it keeps its pixels.
//!
//! These drive the real frame path end to end, minus the window: a surface
//! registered with `create_render_surface_with_name` (what `GameViewport`
//! uses), the collector the shell calls, `install_viewport_frames`,
//! `build_pixels`. The shell used to blit the game frame over the finished
//! pixel buffer *after* `build_pixels`, which overwrote every HUD, modal and
//! dropdown above the viewport and named no damage (`mark_scene_dirty`).
//!
//! Gated on `software_shell`: run under `cargo test -p rinch --features
//! embed,theme,clipboard`, not `--workspace` (which unifies `gpu` on).

use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, collect_viewport_frames_by_name, create_render_surface_with_name,
    create_video_surface, unregister_render_surface,
};
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);
const MAGENTA: [u8; 3] = [255, 0, 255];
const CYAN: [u8; 3] = [0, 255, 255];
const BLUE: [u8; 3] = [0, 0, 255];

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

/// A 400x200 white `overflow: hidden` card at the origin holding a full-size
/// `data-viewport="game-361"` node — a `GameViewport`, so no readiness
/// attribute — then, painted after it, a 100x200 blue HUD over its left edge,
/// and a small label far below standing in for anything that ticks.
///
/// Returns the app, the label's node id, and the game's surface.
fn mount() -> (RinchApp, usize, RenderSurfaceHandle) {
    let game = create_render_surface_with_name("game-361");
    let label_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let captured = label_id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");

        let card = scope.create_element("div");
        card.set_attribute(
            "style",
            "width: 400px; height: 200px; overflow: hidden; background-color: white;",
        );
        let viewport = scope.create_element("div");
        viewport.set_attribute("style", "width: 100%; height: 100%;");
        viewport.set_attribute("data-viewport", "game-361");
        card.append_child(&viewport);
        root.append_child(&card);

        let hud = scope.create_element("div");
        hud.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 0px; width: 100px; height: 200px; \
             background-color: rgb(0, 0, 255);",
        );
        root.append_child(&hud);

        let label = scope.create_element("div");
        label.set_attribute(
            "style",
            "position: absolute; left: 0px; top: 400px; width: 60px; height: 20px; \
             background-color: rgb(0, 128, 0);",
        );
        captured.set(Some(label.node_id().0));
        root.append_child(&label);

        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let id = label_id.get().expect("label id captured at mount");
    (app, id, game)
}

/// A 40x20 solid frame: 2:1, the viewport's own aspect, so it covers the
/// whole box with no letterbox.
fn submit(game: &RenderSurfaceHandle, rgb: [u8; 3]) {
    game.writer()
        .submit_frame(&[rgb[0], rgb[1], rgb[2], 255].repeat(40 * 20), 40, 20);
}

/// One software frame, the way `paint_software` paints it. Returns the
/// frame's performance counters.
fn paint(app: &mut RinchApp) -> FrameStats {
    app.install_viewport_frames(collect_viewport_frames_by_name());
    app.build_pixels(1.0, SIZE, false);
    RinchApp::clear_viewport_frames();
    app.end_perf_frame().expect("a document is mounted")
}

/// Mark `label` paint-dirty, the way a hover or a ticking clock does.
fn dirty(app: &mut RinchApp, label: usize) {
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .tree
        .paint_dirty_nodes
        .push(label);
    app.request_repaint();
}

/// The pixel oracle: the HUD above the game keeps its pixels through game
/// frames, and the game shows wherever the HUD does not cover it — including
/// on a frame where something far away is also damaged, which is when the old
/// `mark_scene_dirty` was ignored and a partial frame was painted (#890).
#[test]
fn a_hud_above_a_game_viewport_keeps_its_pixels_through_game_frames() {
    let (mut app, label, game) = mount();

    submit(&game, MAGENTA);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, 300, 100), MAGENTA),
        "the game frame is painted where the HUD does not cover it, got {:?}",
        pixel_at(&app, 300, 100)
    );
    assert!(
        is(pixel_at(&app, 50, 100), BLUE),
        "the HUD over the game keeps its pixels (#361), got {:?}",
        pixel_at(&app, 50, 100)
    );

    // The next game frame, with a small dirty region elsewhere.
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .tree
        .paint_dirty_nodes
        .push(label);
    submit(&game, CYAN);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, 300, 100), CYAN),
        "the next game frame lands although the only other damage is far \
         away, got {:?}",
        pixel_at(&app, 300, 100)
    );
    assert!(
        is(pixel_at(&app, 50, 100), BLUE),
        "and the HUD is still drawn over it, got {:?}",
        pixel_at(&app, 50, 100)
    );

    unregister_render_surface(game.id());
}

/// A game frame names its damage — the viewport's box — instead of asking for
/// an unattributed full repaint, which is what the blit had to do.
#[test]
fn a_game_frame_is_named_damage_not_a_full_repaint() {
    let (mut app, _, game) = mount();
    submit(&game, MAGENTA);
    paint(&mut app);

    submit(&game, CYAN);
    let stats = paint(&mut app);
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintPartial), 1, "{stats:?}");
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        400 * 200,
        "exactly the viewport's box is repainted: {stats:?}"
    );
    assert!(is(pixel_at(&app, 300, 100), CYAN));
    // The frame vouched opaque at submit and lands on whole pixels, so it is
    // copied, not premultiplied and sampled (#361's cost).
    assert_eq!(stats.get(Counter::OpaqueImageCopies), 1, "{stats:?}");
    assert_eq!(stats.get(Counter::ImagePremultiplies), 0, "{stats:?}");

    unregister_render_surface(game.id());
}

/// A paint with **no** new game frame does not repaint the game: a hover or a
/// tick elsewhere over a paused game costs what it costs with no game at all.
/// The frame is still installed, so the game stays on screen.
#[test]
fn a_paint_with_no_new_game_frame_does_not_repaint_the_viewport() {
    let (mut app, label, game) = mount();
    submit(&game, MAGENTA);
    paint(&mut app);

    // The control: the same label damage with no viewport frames at all.
    dirty(&mut app, label);
    app.build_pixels(1.0, SIZE, false);
    let control = app.end_perf_frame().expect("a document is mounted");
    let label_px = control.get(Counter::RepaintedPx);
    assert!(label_px > 0, "the control repainted the label: {control:?}");

    dirty(&mut app, label);
    let stats = paint(&mut app);
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        label_px,
        "only the label is repainted, not the 400x200 game: {stats:?}"
    );
    assert_eq!(stats.get(Counter::OpaqueImageCopies), 0, "{stats:?}");
    assert!(
        is(pixel_at(&app, 300, 100), MAGENTA),
        "and the game's last frame is still on screen, got {:?}",
        pixel_at(&app, 300, 100)
    );

    unregister_render_surface(game.id());
}

/// Video and a game on one page, through the same install: the video's
/// pillarbox bars are opaque black (#354: it punches no hole, and paints its
/// own backdrop), the game's letterbox is not (it is its hole). Pins the hole
/// set `install_viewport_frames` hands paint — installing `None`, "every
/// viewport punches", would take video's black away.
#[test]
fn video_keeps_its_black_bars_beside_a_game_viewport() {
    let video = create_video_surface("video-361");
    let game = create_render_surface_with_name("game-361b");
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");
        for (name, ready) in [("video-361", true), ("game-361b", false)] {
            let card = scope.create_element("div");
            card.set_attribute(
                "style",
                "width: 400px; height: 100px; overflow: hidden; background-color: white;",
            );
            let viewport = scope.create_element("div");
            viewport.set_attribute("style", "width: 100%; height: 100%;");
            viewport.set_attribute("data-viewport", name);
            if ready {
                viewport.set_attribute("data-viewport-ready", "true");
            }
            card.append_child(&viewport);
            root.append_child(&card);
        }
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);

    // 2:1 frames into 4:1 boxes: a 200x100 frame with 100px bars either side.
    submit(&video, MAGENTA);
    submit(&game, MAGENTA);
    paint(&mut app);

    assert!(is(pixel_at(&app, 200, 50), MAGENTA), "the video frame");
    assert!(is(pixel_at(&app, 200, 150), MAGENTA), "the game frame");
    assert!(
        is(pixel_at(&app, 40, 50), [0, 0, 0]),
        "the video's bar is opaque black (#354), got {:?}",
        pixel_at(&app, 40, 50)
    );
    assert!(
        !is(pixel_at(&app, 40, 150), [0, 0, 0]),
        "the game's bar is its hole, not black, got {:?}",
        pixel_at(&app, 40, 150)
    );

    unregister_render_surface(video.id());
    unregister_render_surface(game.id());
}

// ── Cost: the inline frame against the blit it replaced (review of PR #1002) ──
//
// `cargo test --release -p rinch --features embed,theme,clipboard --lib --
// game_viewport_inline_bench --ignored --nocapture`. The "old" arm replays the
// shell's sequence at 952ee7b4: collect, `mark_scene_dirty`, `build_pixels`,
// then the nearest-neighbour `blit_rgba` over the finished pixels, copied
// verbatim below.
#[allow(clippy::too_many_arguments)]
fn old_blit_rgba(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
    blit_x: i32,
    blit_y: i32,
    blit_w: u32,
    blit_h: u32,
) {
    let (clip_min_x, clip_min_y, clip_max_x, clip_max_y) = (0, 0, dst_w as i32, dst_h as i32);
    let dst_stride = dst_w as usize * 4;
    let src_stride = src_w as usize * 4;
    for dy in 0..blit_h {
        let out_y = blit_y + dy as i32;
        if out_y < 0 || out_y >= dst_h as i32 || out_y < clip_min_y || out_y >= clip_max_y {
            continue;
        }
        let sy = ((dy as f32 / blit_h as f32) * src_h as f32) as u32;
        let sy = sy.min(src_h - 1) as usize;
        for dx in 0..blit_w {
            let out_x = blit_x + dx as i32;
            if out_x < 0 || out_x >= dst_w as i32 || out_x < clip_min_x || out_x >= clip_max_x {
                continue;
            }
            let sx = ((dx as f32 / blit_w as f32) * src_w as f32) as u32;
            let sx = sx.min(src_w - 1) as usize;
            let src_off = sy * src_stride + sx * 4;
            let dst_off = out_y as usize * dst_stride + out_x as usize * 4;
            if src_off + 3 < src.len() && dst_off + 3 < dst.len() {
                let a = src[src_off + 3];
                if a == 255 {
                    dst[dst_off..dst_off + 4].copy_from_slice(&src[src_off..src_off + 4]);
                } else if a > 0 {
                    let af = a as f32 / 255.0;
                    dst[dst_off] = (src[src_off] as f32 * af) as u8;
                    dst[dst_off + 1] = (src[src_off + 1] as f32 * af) as u8;
                    dst[dst_off + 2] = (src[src_off + 2] as f32 * af) as u8;
                    dst[dst_off + 3] = a;
                }
            }
        }
    }
}

fn bench_mount(win: (u32, u32), vp: (u32, u32), name: &'static str) -> RinchApp {
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            &format!(
                "width: {}px; height: {}px; background: white; overflow: hidden;",
                win.0, win.1
            ),
        );
        for i in 0..40 {
            let row = scope.create_element("div");
            row.set_attribute("style", &format!("position: absolute; left: {}px; top: {}px; width: 30px; height: 10px; background: rgb({},0,0);", (i*37)%win.0, (i*23)%win.1, i*5));
            root.append_child(&row);
        }
        let viewport = scope.create_element("div");
        viewport.set_attribute(
            "style",
            &format!(
                "position: absolute; left: 0px; top: 0px; width: {}px; height: {}px;",
                vp.0, vp.1
            ),
        );
        viewport.set_attribute("data-viewport", name);
        root.append_child(&viewport);
        let hud = scope.create_element("div");
        hud.set_attribute("style", "position: absolute; left: 10px; top: 10px; width: 200px; height: 40px; background: rgb(0,0,255);");
        root.append_child(&hud);
        root
    });
    app.mount_component(win.0 as f32, win.1 as f32);
    app
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[test]
#[ignore]
fn game_viewport_inline_bench() {
    for &(win, vp, frame, name) in &[
        (
            (1920u32, 1080u32),
            (1920u32, 1080u32),
            (1920u32, 1080u32),
            "b-full",
        ),
        ((1920, 1080), (960, 540), (1920, 1080), "b-half"),
        ((1280, 720), (640, 360), (320, 180), "b-up"),
    ] {
        let game = create_render_surface_with_name(name);
        let px: Vec<u8> = (0..frame.0 * frame.1)
            .flat_map(|i| [(i % 251) as u8, (i % 13) as u8, 7, 255])
            .collect();
        let mut app = bench_mount(win, vp, name);
        // new path
        let mut t_new = Vec::new();
        let mut t_submit = Vec::new();
        for i in 0..25 {
            // The submitting (game) thread's side: copy plus the opacity scan.
            let t = std::time::Instant::now();
            game.writer().submit_frame(&px, frame.0, frame.1);
            if i > 4 {
                t_submit.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let t = std::time::Instant::now();
            app.install_viewport_frames(collect_viewport_frames_by_name());
            app.build_pixels(1.0, win, false);
            RinchApp::clear_viewport_frames();
            if i > 4 {
                t_new.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let _ = app.end_perf_frame();
        }
        // old path (base 952ee7b4 shell semantics): collect clone, mark_scene_dirty, hole set, build, blit
        let mut t_old = Vec::new();
        for i in 0..25 {
            game.writer().submit_frame(&px, frame.0, frame.1);
            let t = std::time::Instant::now();
            let f = collect_viewport_frames_by_name();
            app.mark_scene_dirty();
            rinch_dom::paint::set_active_viewports(Some(f.holes.clone()));
            app.build_pixels(1.0, win, false);
            rinch_dom::paint::set_active_viewports(None);
            let fr = &f.frames[name];
            let p = app.skia_painter.as_mut().unwrap();
            let (w, h) = (p.width(), p.height());
            old_blit_rgba(
                p.pixels_mut(),
                w,
                h,
                &fr.data,
                fr.width,
                fr.height,
                0,
                0,
                vp.0,
                vp.1,
            );
            if i > 4 {
                t_old.push(t.elapsed().as_secs_f64() * 1e3);
            }
            let _ = app.end_perf_frame();
        }
        eprintln!(
            "BENCH-361 {name} win={win:?} vp={vp:?} frame={frame:?}: new {:.2} ms, old {:.2} ms \
             (submit, on the game thread: {:.2} ms)",
            median(t_new),
            median(t_old),
            median(t_submit)
        );
        unregister_render_surface(game.id());
    }
}
