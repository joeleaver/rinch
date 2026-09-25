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
    unregister_render_surface,
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

    unregister_render_surface(game.id());
}
