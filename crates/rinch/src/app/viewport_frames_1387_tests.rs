//! What paint is handed for a surface, beyond the frame itself, is a paint
//! input whose change is damage — and a frame paint will not draw is not.
//!
//! - #1387: a `RenderSurface` id that leaves the id-keyed frame map (its
//!   surface unregistered, or its buffer emptied, while the node stays) changed
//!   what its `data-render-surface` node draws, and nothing named the box.
//! - #1388: two surfaces registered under one viewport name; when the one on
//!   screen goes, the other's frame takes the name's slot. The name set is
//!   unchanged and the survivor's frame is not fresh, so the dead surface's
//!   frame stayed on screen. The name's *surface id* is the input.
//! - #1400: a not-ready viewport (`data-viewport-ready="false"`) shows its
//!   placeholder, not its frame (#348), yet every fresh frame it received
//!   repainted its box to draw the same placeholder.
//!
//! Gated on `software_shell`: run under `cargo test -p rinch --features
//! embed,theme,clipboard`, like `viewport_inputs_349_tests`.

use super::*;
use crate::render_surface::{
    RenderSurfaceHandle, collect_viewport_frames_by_name, create_render_surface_with_name,
    create_video_surface, registered_surface_ids, unregister_render_surface,
};
use rinch_dom::perf::{Counter, FrameStats};
use std::cell::RefCell;
use std::collections::HashMap;

const SIZE: (u32, u32) = (800, 600);
const MAGENTA: [u8; 3] = [255, 0, 255];
const BLUE: [u8; 3] = [0, 0, 255];
/// The viewport node's own background while not ready: the placeholder.
const GREEN: [u8; 3] = [0, 128, 0];

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

/// A 40x20 solid frame (2:1, the card's own ratio, so it covers the card).
fn submit(surface: &RenderSurfaceHandle, rgb: [u8; 3]) {
    surface
        .writer()
        .submit_frame(&[rgb[0], rgb[1], rgb[2], 255].repeat(40 * 20), 40, 20);
}

/// One software frame of the named viewports, the way `paint_software` does it.
fn paint(app: &mut RinchApp) -> FrameStats {
    app.install_viewport_frames(collect_viewport_frames_by_name(&registered_surface_ids()));
    app.build_pixels(1.0, SIZE, false);
    RinchApp::clear_viewport_frames();
    app.end_perf_frame().expect("a document is mounted")
}

fn settle(app: &mut RinchApp) {
    if app.has_pending_layout() {
        app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    }
}

/// A 400x200 card at (20, 30), off the origin, holding one full-size node
/// carrying `attr = value` and `node_style`. Returns the app and that node.
fn mount(attr: &'static str, value: String, node_style: &'static str) -> (RinchApp, NodeHandle) {
    let out: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let o2 = out.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "width: 800px; height: 600px; background-color: white;",
        );
        let holder = scope.create_element("div");
        holder.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 30px; width: 400px; height: 200px; \
             overflow: hidden; background-color: rgb(255, 200, 0);",
        );
        let node = scope.create_element("div");
        node.set_attribute("style", &format!("width: 100%; height: 100%; {node_style}"));
        node.set_attribute(attr, &value);
        holder.append_child(&node);
        root.append_child(&holder);
        *o2.borrow_mut() = Some(node);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let node = out.borrow().clone().unwrap();
    (app, node)
}

/// A point inside the card, off its corner.
const INSIDE: (u32, u32) = (300, 130);

// ── #1387 ───────────────────────────────────────────────────────────────────

/// One software frame of the `RenderSurface`s, the way `paint_software` hands
/// them over: the id-keyed map, its ids noted, the map installed.
fn paint_surfaces(
    app: &mut RinchApp,
    pixels: HashMap<usize, rinch_dom::paint::SurfacePixelData>,
) -> FrameStats {
    app.install_surface_pixels(pixels);
    let stats = paint(app);
    rinch_dom::paint::set_surface_pixels(None);
    stats
}

fn solid(rgb: [u8; 3]) -> rinch_dom::paint::SurfacePixelData {
    rinch_dom::paint::SurfacePixelData {
        data: [rgb[0], rgb[1], rgb[2], 255].repeat(40 * 20).into(),
        width: 40,
        height: 20,
        opaque: true,
    }
}

/// The issue's shape: the surface's id leaves the map while its node stays.
/// Its frame leaves the screen, and the damage is the node's box only.
#[test]
fn a_render_surface_id_that_leaves_the_map_gives_its_box_back() {
    const ID: usize = 4_241_387;
    let (mut app, _) = mount("data-render-surface", ID.to_string(), "");
    paint_surfaces(&mut app, HashMap::from([(ID, solid(MAGENTA))]));
    assert!(
        is(pixel_at(&app, INSIDE), MAGENTA),
        "positive control: the surface's frame is on screen, got {:?}",
        pixel_at(&app, INSIDE)
    );

    let stats = paint_surfaces(&mut app, HashMap::new());
    assert!(
        is(pixel_at(&app, INSIDE), [255, 200, 0]),
        "the surface left the map, its last frame is still on screen: {:?} ({stats:?})",
        pixel_at(&app, INSIDE)
    );
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");

    // An unchanged (empty) map afterwards is no damage.
    let stats = paint_surfaces(&mut app, HashMap::new());
    assert_eq!(stats.get(Counter::RepaintedPx), 0, "{stats:?}");
}

/// The other direction, with nothing else asking: an id that joins is damage
/// too (here through the noted set; the shell also marks every id it hands).
#[test]
fn noting_an_id_that_joins_marks_its_node_only() {
    const ID: usize = 4_241_388;
    let (mut app, node) = mount("data-render-surface", ID.to_string(), "");
    paint_surfaces(&mut app, HashMap::new());
    app.note_render_surface_ids([ID].into_iter().collect());
    let dirty = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .paint_dirty_nodes
        .clone();
    assert_eq!(dirty, vec![node.node_id().0], "the joining id's node");
    assert!(app.scene_dirty);
    // Noted again unchanged: nothing more.
    app.doc
        .as_ref()
        .unwrap()
        .borrow_mut()
        .tree
        .paint_dirty_nodes
        .clear();
    app.scene_dirty = false;
    app.note_render_surface_ids([ID].into_iter().collect());
    assert!(
        app.doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .paint_dirty_nodes
            .is_empty()
    );
    assert!(!app.scene_dirty);
}

// ── #1388 ───────────────────────────────────────────────────────────────────

/// Two `GameViewport` surfaces under one name: the later-registered one wins
/// the name. When it goes, the earlier one's frame is what paint is handed,
/// and it is drawn, as a partial repaint of the viewport's box.
#[test]
fn the_surviving_surface_of_two_with_one_name_is_drawn() {
    let (mut app, _) = mount("data-viewport", "twin-1388".into(), "");
    let old = create_render_surface_with_name("twin-1388");
    submit(&old, MAGENTA);
    paint(&mut app);
    assert!(is(pixel_at(&app, INSIDE), MAGENTA), "control: old shown");
    let new = create_render_surface_with_name("twin-1388");
    submit(&new, BLUE);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, INSIDE), BLUE),
        "control: the later-registered surface wins"
    );

    unregister_render_surface(new.id());
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, INSIDE), MAGENTA),
        "the surviving surface's frame is handed to paint, the screen keeps the \
         dead one's: {:?} ({stats:?})",
        pixel_at(&app, INSIDE)
    );
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");

    let stats = paint(&mut app);
    assert_eq!(stats.get(Counter::RepaintedPx), 0, "idle again: {stats:?}");
    unregister_render_surface(old.id());
}

/// The same for video, on the GPU window's half (`install_gpu_viewport_frames`,
/// where video is painted inline in the scene): the swap of surface behind an
/// unchanged name marks the node.
#[test]
fn the_gpu_half_marks_a_name_whose_surface_changed() {
    let (mut app, node) = mount("data-viewport", "twin-1388-gpu".into(), "");
    let old = create_video_surface("twin-1388-gpu");
    submit(&old, MAGENTA);
    let new = create_video_surface("twin-1388-gpu");
    submit(&new, BLUE);
    let gpu_paint = |app: &mut RinchApp| {
        app.install_gpu_viewport_frames(
            collect_viewport_frames_by_name(&registered_surface_ids()),
            Some(Default::default()),
        );
        app.build_pixels(1.0, SIZE, false);
        RinchApp::clear_viewport_frames();
        let _ = app.end_perf_frame();
    };
    gpu_paint(&mut app);
    gpu_paint(&mut app);

    unregister_render_surface(new.id());
    app.install_gpu_viewport_frames(
        collect_viewport_frames_by_name(&registered_surface_ids()),
        Some(Default::default()),
    );
    RinchApp::clear_viewport_frames();
    let dirty = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .paint_dirty_nodes
        .clone();
    assert_eq!(
        dirty,
        vec![node.node_id().0],
        "the viewport whose surface changed"
    );
    assert!(app.scene_dirty);
    unregister_render_surface(old.id());
}

// ── #1400 ───────────────────────────────────────────────────────────────────

/// A not-ready viewport that keeps receiving frames repaints nothing: what it
/// shows is its placeholder, unchanged. Becoming ready again (an attribute
/// write, which names the box) shows the newest frame.
#[test]
fn a_not_ready_viewport_that_keeps_receiving_frames_repaints_nothing() {
    let (mut app, vp) = mount(
        "data-viewport",
        "idle-1400".into(),
        "background-color: rgb(0, 128, 0);",
    );
    let game = create_render_surface_with_name("idle-1400");
    submit(&game, MAGENTA);
    paint(&mut app);
    vp.set_attribute("data-viewport-ready", "false");
    settle(&mut app);
    paint(&mut app);
    assert!(
        is(pixel_at(&app, INSIDE), GREEN),
        "control: the placeholder"
    );

    submit(&game, BLUE);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, INSIDE), GREEN),
        "{:?}",
        pixel_at(&app, INSIDE)
    );
    assert_eq!(
        stats.get(Counter::RepaintedPx),
        0,
        "a fresh frame under a not-ready node repainted its placeholder: {stats:?}"
    );
    assert_eq!(stats.get(Counter::PaintCachedFrames), 1, "{stats:?}");

    vp.set_attribute("data-viewport-ready", "true");
    settle(&mut app);
    let stats = paint(&mut app);
    assert!(
        is(pixel_at(&app, INSIDE), BLUE),
        "ready again shows the newest frame: {:?}",
        pixel_at(&app, INSIDE)
    );
    assert_eq!(stats.get(Counter::RepaintFull), 0, "{stats:?}");
    assert_eq!(stats.get(Counter::RepaintedPx), 400 * 200, "{stats:?}");
    unregister_render_surface(game.id());
}

/// Readiness is each node's own: a fresh frame under a ready node is still
/// its damage when another node of the name says not ready — the rule paint
/// follows (`rinch_dom::paint::viewport_ready(node)`).
#[test]
fn a_fresh_frame_marks_only_the_ready_nodes_of_its_name() {
    let pair: Rc<RefCell<Vec<NodeHandle>>> = Rc::new(RefCell::new(Vec::new()));
    let p2 = pair.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        for ready in ["false", "true"] {
            let vp = scope.create_element("div");
            vp.set_attribute("style", "width: 200px; height: 100px;");
            vp.set_attribute("data-viewport", "pair-1400");
            vp.set_attribute("data-viewport-ready", ready);
            root.append_child(&vp);
            p2.borrow_mut().push(vp);
        }
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    let video = create_video_surface("pair-1400");
    submit(&video, MAGENTA);
    paint(&mut app);
    submit(&video, BLUE);
    app.install_viewport_frames(collect_viewport_frames_by_name(&registered_surface_ids()));
    RinchApp::clear_viewport_frames();
    let dirty = app
        .doc
        .as_ref()
        .unwrap()
        .borrow()
        .tree
        .paint_dirty_nodes
        .clone();
    assert_eq!(
        dirty,
        vec![pair.borrow()[1].node_id().0],
        "the ready node only"
    );
    assert!(app.scene_dirty);
    unregister_render_surface(video.id());
}

/// `viewport_ready`'s connectivity walk (from the #1396 review, `r6`): a
/// detached node of the name does not answer for it.
#[test]
fn a_detached_node_of_the_name_does_not_answer_for_its_readiness() {
    let (mut app, vp) = mount("data-viewport", "orphan-1400".into(), "");
    settle(&mut app);
    let holder = vp.parent_node().expect("holder");
    vp.set_attribute("data-viewport-ready", "false");
    assert!(
        !app.viewport_ready("orphan-1400"),
        "control: connected, not ready"
    );
    vp.remove();
    assert!(
        app.viewport_ready("orphan-1400"),
        "the only node of the name is detached"
    );
    holder.append_child(&vp);
    assert!(!app.viewport_ready("orphan-1400"));
}
