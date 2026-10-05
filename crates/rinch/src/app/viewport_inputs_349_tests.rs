//! #349: the set of viewports a paint is handed — which have a frame, which
//! punch a hole — is a paint input, so a change in it is damage.
//!
//! A surface that unregisters while its `data-viewport` node stays in the
//! document changes no DOM attribute and delivers no frame, so nothing named
//! its box and the cached pixels kept the last frame (and its hole) until
//! something unrelated repainted that area. Each fixture changes the set and
//! nothing else, then reads the pixels a full repaint would give.
//!
//! Gated on `software_shell`: run under `cargo test -p rinch --features
//! embed,theme,clipboard`, like `game_viewport_inline_tests`.

use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, ViewportFrames, collect_viewport_frames_by_name,
    create_render_surface_with_name, create_video_surface, unregister_render_surface,
};
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::Cell;

const SIZE: (u32, u32) = (800, 600);
const MAGENTA: [u8; 3] = [255, 0, 255];
/// The cards' background: not the window's clear colour, so a hole cut through
/// a card can be told from the card.
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

/// An orange `overflow: hidden` card of `card` px, 30px down and 20px in from
/// the window's corner (so its box is not at an origin), holding a full-size
/// `data-viewport` node named `name`; and a 60x20 label far below, standing in
/// for anything that ticks. Returns the app and the label's node id.
fn mount(name: &'static str, card: (u32, u32), ready: bool) -> (RinchApp, usize) {
    let label_id: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    let captured = label_id.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");

        let holder = scope.create_element("div");
        holder.set_attribute(
            "style",
            &format!(
                "position: absolute; left: 20px; top: 30px; width: {}px; height: {}px; \
                 overflow: hidden; background-color: rgb(255, 200, 0);",
                card.0, card.1
            ),
        );
        let viewport = scope.create_element("div");
        viewport.set_attribute("style", "width: 100%; height: 100%;");
        viewport.set_attribute("data-viewport", name);
        if ready {
            viewport.set_attribute("data-viewport-ready", "true");
        }
        holder.append_child(&viewport);
        root.append_child(&holder);

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
    (app, label_id.get().expect("label id captured at mount"))
}

/// A 40x20 solid frame (2:1).
fn submit(surface: &RenderSurfaceHandle, rgb: [u8; 3]) {
    surface
        .writer()
        .submit_frame(&[rgb[0], rgb[1], rgb[2], 255].repeat(40 * 20), 40, 20);
}

/// One software frame with `frames` installed, the way `paint_software` does.
fn paint_with(app: &mut RinchApp, frames: ViewportFrames) -> FrameStats {
    app.install_viewport_frames(frames);
    app.build_pixels(1.0, SIZE, false);
    RinchApp::clear_viewport_frames();
    app.end_perf_frame().expect("a document is mounted")
}

fn paint(app: &mut RinchApp) -> FrameStats {
    paint_with(app, collect_viewport_frames_by_name())
}

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

/// The issue's own shape: a game's surface unregisters, its node stays, and
/// nothing else changes. The frame and its hole leave the screen, and the
/// damage is the viewport's box — not the window.
#[test]
fn a_game_surface_that_unregisters_gives_its_box_back() {
    let (mut app, _) = mount("game-349a", (400, 200), false);
    let game = create_render_surface_with_name("game-349a");
    submit(&game, MAGENTA);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, 300, 130), MAGENTA),
        "positive control: the game frame is on screen, got {:?}",
        pixel_at(&app, 300, 130)
    );

    unregister_render_surface(game.id());
    let stats = paint(&mut app);
    for (x, y) in [(300, 130), (21, 31), (418, 228)] {
        assert!(
            is(pixel_at(&app, x, y), CARD),
            "the card's own background is back at ({x}, {y}) once the game \
             has no surface, got {:?} ({stats:?})",
            pixel_at(&app, x, y)
        );
    }
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        400 * 200,
        "exactly the viewport's box is repainted: {stats:?}"
    );

    // And only once: the set is unchanged on the next paint.
    let stats = paint(&mut app);
    assert_eq!(stats.get(Counter::RepaintedPx), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::PaintCachedFrames), 1, "{stats:?}");
}

/// The same with damage elsewhere in the same frame — the case in which the
/// old unattributed full repaint would have been ignored anyway (#890), and in
/// which a stale frame would be left however the scene was dirtied.
#[test]
fn an_unregistered_game_is_repainted_beside_other_damage() {
    let (mut app, label) = mount("game-349b", (400, 200), false);
    let game = create_render_surface_with_name("game-349b");
    submit(&game, MAGENTA);
    paint(&mut app);

    dirty(&mut app, label);
    let control = paint(&mut app);
    let label_px = control.get(Counter::RepaintedPx);
    assert!(label_px > 0, "the control repainted the label: {control:?}");
    assert!(is(pixel_at(&app, 300, 130), MAGENTA));

    unregister_render_surface(game.id());
    dirty(&mut app, label);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, 300, 130), CARD),
        "got {:?} ({stats:?})",
        pixel_at(&app, 300, 130)
    );
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        label_px + 400 * 200,
        "the label and the viewport's box, nothing else: {stats:?}"
    );
}

/// Video punches no hole, so only the frame map says it went away.
#[test]
fn a_video_surface_that_unregisters_takes_its_frame_off_screen() {
    let (mut app, _) = mount("video-349", (400, 200), true);
    let video = create_video_surface("video-349");
    submit(&video, MAGENTA);
    paint(&mut app);
    assert!(is(pixel_at(&app, 300, 130), MAGENTA), "positive control");

    unregister_render_surface(video.id());
    let stats = paint(&mut app);
    assert!(
        !is(pixel_at(&app, 300, 130), MAGENTA),
        "the dead video's last frame is not left on screen ({stats:?})"
    );
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");
}

/// The other direction: a frame that is in the map without being *fresh* — it
/// was collected by a paint that did not install it — is still new to this
/// paint.
#[test]
fn a_frame_that_arrives_without_being_fresh_is_painted() {
    let (mut app, _) = mount("game-349c", (400, 200), false);
    let game = create_render_surface_with_name("game-349c");
    paint(&mut app);
    assert!(is(pixel_at(&app, 300, 130), CARD), "no frame yet");

    submit(&game, MAGENTA);
    let _ = collect_viewport_frames_by_name(); // somebody else's collection
    let frames = collect_viewport_frames_by_name();
    assert!(
        frames.fresh.is_empty(),
        "positive control: nothing is fresh"
    );
    assert!(frames.frames.contains_key("game-349c"));
    let stats = paint_with(&mut app, frames);
    assert!(
        is(pixel_at(&app, 300, 130), MAGENTA),
        "got {:?} ({stats:?})",
        pixel_at(&app, 300, 130)
    );

    unregister_render_surface(game.id());
}

/// The hole set alone: the same frame, no longer punching. A 2:1 frame in a
/// 4:1 box leaves 100px bars either side, and a bar is the hole — the window's
/// clear colour. A frame that punches none is fitted over an opaque black
/// backdrop instead (#354), so with the hole gone the bar is black.
#[test]
fn a_viewport_that_stops_punching_is_repainted_without_its_hole() {
    let (mut app, _) = mount("game-349d", (400, 100), false);
    let game = create_render_surface_with_name("game-349d");
    submit(&game, MAGENTA);
    paint(&mut app);
    assert!(is(pixel_at(&app, 220, 80), MAGENTA), "the frame");
    assert!(
        is(pixel_at(&app, 60, 80), [255, 255, 255]),
        "positive control: the bar is the hole, got {:?}",
        pixel_at(&app, 60, 80)
    );

    let mut frames = collect_viewport_frames_by_name();
    assert!(frames.fresh.is_empty());
    assert!(frames.holes.remove("game-349d"));
    let stats = paint_with(&mut app, frames);
    assert!(
        is(pixel_at(&app, 60, 80), [0, 0, 0]),
        "the bar is the frame's black backdrop once nothing punches, got \
         {:?} ({stats:?})",
        pixel_at(&app, 60, 80)
    );
    assert!(is(pixel_at(&app, 220, 80), MAGENTA), "the frame stays");

    unregister_render_surface(game.id());
}

/// Two viewports, one of which goes: the other is not damaged.
#[test]
fn only_the_viewport_whose_inputs_changed_is_damaged() {
    let a = create_render_surface_with_name("game-349e");
    let b = create_render_surface_with_name("game-349f");
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "width: 800px; height: 600px;");
        for (name, w) in [("game-349e", 400), ("game-349f", 300)] {
            let card = scope.create_element("div");
            card.set_attribute(
                "style",
                &format!(
                    "width: {w}px; height: 100px; overflow: hidden; background-color: rgb(255, 200, 0);"
                ),
            );
            let viewport = scope.create_element("div");
            viewport.set_attribute("style", "width: 100%; height: 100%;");
            viewport.set_attribute("data-viewport", name);
            card.append_child(&viewport);
            root.append_child(&card);
        }
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    submit(&a, MAGENTA);
    submit(&b, MAGENTA);
    paint(&mut app);
    assert!(is(pixel_at(&app, 150, 150), MAGENTA), "b's frame");

    unregister_render_surface(b.id());
    let stats = paint(&mut app);
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        300 * 100,
        "b's box, not a's 400x100: {stats:?}"
    );
    assert!(is(pixel_at(&app, 150, 150), CARD));
    assert!(is(pixel_at(&app, 200, 50), MAGENTA), "a keeps its frame");

    unregister_render_surface(a.id());
}

/// `None` — "every viewport punches", the GPU shell's retention fallback — is
/// a set like any other: going to it punches a viewport no name mentioned, and
/// coming back from it fills that hole in.
#[test]
fn every_viewport_punches_is_a_change_for_every_viewport() {
    let (mut app, _) = mount("game-349i", (400, 200), false);
    let frame = |app: &mut RinchApp, holes: Option<std::collections::HashSet<String>>| {
        app.install_viewport_holes(holes);
        app.build_pixels(1.0, SIZE, false);
        rinch_dom::paint::set_active_viewports(None);
        app.end_perf_frame().expect("a document is mounted")
    };
    let nobody = || Some(std::collections::HashSet::new());

    frame(&mut app, nobody());
    assert!(is(pixel_at(&app, 300, 130), CARD), "nothing punches");

    let stats = frame(&mut app, None);
    assert!(
        is(pixel_at(&app, 300, 130), [255, 255, 255]),
        "the viewport punches through the card, got {:?} ({stats:?})",
        pixel_at(&app, 300, 130)
    );
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");

    let stats = frame(&mut app, nobody());
    assert!(
        is(pixel_at(&app, 300, 130), CARD),
        "and the card is whole again, got {:?} ({stats:?})",
        pixel_at(&app, 300, 130)
    );
}

// ── The GPU compositor's half: the hole set, through `build_scene` ──────────
//
// There the frames are layers under the scene, so the hole set is the whole
// of the input, and "repainted" is "the scene was re-encoded rather than
// handed back cached". No pixel oracle: a Vello scene needs a device.
#[cfg(any(feature = "gpu", feature = "embed"))]
mod scene {
    use super::*;
    use std::collections::HashSet;

    fn scene_frame(app: &mut RinchApp, holes: Option<HashSet<String>>) -> FrameStats {
        app.install_viewport_holes(holes);
        app.build_scene(1.0, SIZE);
        rinch_dom::paint::set_active_viewports(None);
        app.end_perf_frame().expect("a document is mounted")
    }

    fn set(names: &[&str]) -> Option<HashSet<String>> {
        Some(names.iter().map(|n| n.to_string()).collect())
    }

    fn viewport_is_dirty(app: &RinchApp) -> bool {
        let d = app.doc.as_ref().unwrap().borrow();
        d.tree.paint_dirty_nodes.iter().any(|&id| {
            d.tree
                .get(id)
                .is_some_and(|n| n.attributes.contains_key("data-viewport"))
        })
    }

    #[test]
    fn a_changed_hole_set_re_encodes_the_scene_and_an_unchanged_one_does_not() {
        let (mut app, _) = mount("game-349g", (400, 200), false);
        scene_frame(&mut app, set(&[]));

        // Unchanged: the cached scene is right.
        let stats = scene_frame(&mut app, set(&[]));
        assert_eq!(stats.get(Counter::PaintCachedFrames), 1, "{stats:?}");
        assert_eq!(stats.get(Counter::PaintFrames), 0, "{stats:?}");

        // The game's first frame: its name joins the set.
        let stats = scene_frame(&mut app, set(&["game-349g"]));
        assert_eq!(stats.get(Counter::PaintFrames), 1, "{stats:?}");
        let stats = scene_frame(&mut app, set(&["game-349g"]));
        assert_eq!(stats.get(Counter::PaintFrames), 0, "{stats:?}");

        // Its surface unregisters: the name leaves (the issue's shape).
        let stats = scene_frame(&mut app, set(&[]));
        assert_eq!(stats.get(Counter::PaintFrames), 1, "{stats:?}");

        // The retention fallback, `None`, is "every viewport punches".
        let stats = scene_frame(&mut app, None);
        assert_eq!(stats.get(Counter::PaintFrames), 1, "{stats:?}");
        let stats = scene_frame(&mut app, None);
        assert_eq!(stats.get(Counter::PaintFrames), 0, "{stats:?}");
        let stats = scene_frame(&mut app, set(&[]));
        assert_eq!(stats.get(Counter::PaintFrames), 1, "{stats:?}");
    }

    /// The damage is the viewport node, so a software frame that follows (a
    /// window re-created onto the other backend) is told the same thing.
    #[test]
    fn the_changed_viewport_is_what_is_marked() {
        let (mut app, _) = mount("game-349h", (400, 200), false);
        scene_frame(&mut app, set(&[]));
        assert!(!viewport_is_dirty(&app), "consumed by the paint");

        app.install_viewport_holes(set(&["somebody-else"]));
        assert!(
            !viewport_is_dirty(&app),
            "a name that is no viewport here marks nothing"
        );
        app.install_viewport_holes(set(&["somebody-else", "game-349h"]));
        assert!(viewport_is_dirty(&app));
        rinch_dom::paint::set_active_viewports(None);
    }
}
