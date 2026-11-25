//! The drag ghost's damage on the desktop GPU path holds every pixel the
//! ghost drew.
//!
//! Vello appends the ghost's scene at the pointer's fractional translate, and
//! the scene holds what the dragged subtree paints past its box (an outline, a
//! shadow). The ghost's damage was the snapshot's box at the translate
//! truncated to whole pixels, so each frame that moved the ghost on left a
//! column of its right edge and a row of its bottom edge on screen, and all of
//! an outline: thin lines trailing every drag (Pimble's kanban, a card with a
//! chosen border dragged across the board). No GPU is needed: this reads the
//! rects [`RinchApp::build_gpu_frame`] hands the renderer.
use super::*;
use peniko::kurbo::Rect;

const W: u32 = 600;
const H: u32 = 400;

/// A page with one draggable box at (400, 20), 60 x 30, outlined 3px.
fn app() -> RinchApp {
    let mut app = RinchApp::new(|scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 600px; height: 400px");
        let src = scope.create_element("div");
        src.set_attribute("draggable", "true");
        src.set_attribute(
            "style",
            "position: absolute; left: 400px; top: 20px; width: 60px; height: 30px; \
             background: rgb(40, 40, 40); outline: 3px solid rgb(0, 120, 255)",
        );
        root.append_child(&src);
        root
    });
    app.mount_component(W as f32, H as f32);
    assert!(matches!(
        app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame)
        ),
        GpuFrame::Full(_)
    ));
    app
}

fn event(app: &mut RinchApp, e: PlatformEvent) {
    app.handle_event(e, (W, H), 1.0);
    app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(W as f32, H as f32);
    }
}

fn rects(frame: GpuFrame<'_>) -> Vec<Rect> {
    match frame {
        GpuFrame::Partial { rects, .. } => rects
            .into_iter()
            .map(|(x, y, w, h)| Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64))
            .collect(),
        GpuFrame::Full(_) => panic!("a full frame"),
        GpuFrame::Unchanged => panic!("an unchanged frame"),
    }
}

fn covers(rects: &[Rect], r: Rect) -> bool {
    let (x0, y0, x1, y1) = (r.x0 as i64, r.y0 as i64, r.x1 as i64, r.y1 as i64);
    (y0..y1).all(|y| {
        (x0..x1).all(|x| {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            rects
                .iter()
                .any(|r| r.x0 <= px && px < r.x1 && r.y0 <= py && py < r.y1)
        })
    })
}

#[test]
fn a_ghost_at_a_fractional_place_damages_its_edges_and_outline() {
    let mut app = app();
    // Grabbed 30, 15 into the box.
    event(
        &mut app,
        PlatformEvent::MouseDown {
            x: 430.0,
            y: 35.0,
            button: MouseButton::Left,
        },
    );
    event(&mut app, PlatformEvent::MouseMove { x: 250.4, y: 200.6 });
    assert!(app.active_dnd.is_some(), "the drag activated");
    let _ = app.build_gpu_frame(1.0, (W, H), None);
    // The box at (220.4, 185.6), its outline 3px past it on every side,
    // rounded out to the pixels that touches.
    let drawn = Rect::new(217.0, 182.0, 284.0, 219.0);
    let ghost = app.last_ghost_rect.expect("the ghost was drawn");
    assert!(
        ghost.contains_rect(drawn),
        "the ghost's rect {ghost:?} holds what it drew, {drawn:?}"
    );

    // Moved on: the frame repaints every pixel it drew where it was.
    event(&mut app, PlatformEvent::MouseMove { x: 100.0, y: 300.0 });
    let moved = rects(app.build_gpu_frame(1.0, (W, H), None));
    assert!(covers(&moved, drawn), "{moved:?} repaints {drawn:?}");

    // Dropped: the same, where it last was.
    let last = app.last_ghost_rect.expect("the ghost was drawn");
    event(
        &mut app,
        PlatformEvent::MouseUp {
            x: 100.0,
            y: 300.0,
            button: MouseButton::Left,
        },
    );
    assert!(app.active_dnd.is_none());
    let dropped = rects(app.build_gpu_frame(1.0, (W, H), None));
    assert!(covers(&dropped, last), "{dropped:?} repaints {last:?}");
}

/// A page with one draggable box at (400, 20), 60 x 30, holding `inner`.
fn app_holding(inner: &str) -> RinchApp {
    app_holding_styled("", inner)
}

/// [`app_holding`], the box also styled `style`.
fn app_holding_styled(style: &str, inner: &str) -> RinchApp {
    let inner = inner.to_string();
    let style = format!(
        "position: absolute; left: 400px; top: 20px; width: 60px; height: 30px; \
         background: rgb(40, 40, 40); {style}"
    );
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 600px; height: 400px");
        let src = scope.create_element("div");
        src.set_attribute("draggable", "true");
        src.set_attribute("style", &style);
        src.set_inner_html(&inner);
        root.append_child(&src);
        root
    });
    app.mount_component(W as f32, H as f32);
    assert!(matches!(
        app.build_gpu_frame(
            1.0,
            (W, H),
            Some(rinch_dom::perf::FullRepaintReason::FirstFrame)
        ),
        GpuFrame::Full(_)
    ));
    app
}

/// Grabs the box 30, 15 into it and drags it to (250, 200), so it is drawn
/// at (220, 185); returns the ghost's rect on that frame and the damage of
/// the frame after it moved on to (100, 300).
fn drag_away(app: &mut RinchApp) -> (Rect, Vec<Rect>) {
    event(
        app,
        PlatformEvent::MouseDown {
            x: 430.0,
            y: 35.0,
            button: MouseButton::Left,
        },
    );
    event(app, PlatformEvent::MouseMove { x: 250.0, y: 200.0 });
    assert!(app.active_dnd.is_some(), "the drag activated");
    let _ = app.build_gpu_frame(1.0, (W, H), None);
    let ghost = app.last_ghost_rect.expect("the ghost was drawn");
    event(app, PlatformEvent::MouseMove { x: 100.0, y: 300.0 });
    let moved = rects(app.build_gpu_frame(1.0, (W, H), None));
    (ghost, moved)
}

fn area(rects: &[Rect]) -> f64 {
    rects.iter().map(|r| r.area()).sum()
}

/// More nodes than the paint's per-frame visit budget: the ghost's reach is
/// measured once, without it, so the ghost damages its box, not the whole
/// surface on every frame (#3065).
#[test]
fn a_ghost_past_the_visit_budget_damages_only_its_box() {
    let many = "<i style=\"position: absolute; left: 1px; top: 1px; width: 1px; height: 1px\"></i>"
        .repeat(700);
    let mut app = app_holding(&many);
    let (ghost, moved) = drag_away(&mut app);
    let drawn = Rect::new(220.0, 185.0, 280.0, 215.0);
    assert!(ghost.contains_rect(drawn), "{ghost:?} holds {drawn:?}");
    assert!(
        ghost.area() < 2.0 * drawn.area(),
        "the ghost's rect {ghost:?} is its box, not the surface"
    );
    assert!(covers(&moved, drawn), "{moved:?} repaints {drawn:?}");
    assert!(
        area(&moved) < (W * H) as f64 / 4.0,
        "moving the ghost repaints around it, not the surface: {moved:?}"
    );
}

/// A `position: fixed` descendant cannot be placed by the ghost's walk: the
/// snapshot is drawn clipped to its box (as the software pixmap is), and its
/// damage is that box (#3065: it damaged the whole surface every frame).
#[test]
fn a_ghost_with_a_fixed_descendant_damages_only_its_box() {
    let mut app = app_holding(
        "<div style=\"position: fixed; left: 0; top: 300px; width: 50px; height: 50px; \
         background: rgb(200, 0, 0)\"></div>",
    );
    let (ghost, moved) = drag_away(&mut app);
    assert_eq!(ghost, Rect::new(220.0, 185.0, 280.0, 215.0));
    assert!(
        area(&moved) < (W * H) as f64 / 4.0,
        "moving the ghost repaints around it, not the surface: {moved:?}"
    );
}

/// The same box scaled 2x about its centre, holding a `position: sticky`
/// box (a transformed root contains a `fixed` one, so the walk places that;
/// a sticky one it cannot bound): the ghost is drawn scaled, so it is clipped
/// to, and damages, its box as drawn, 120 x 60 about the same centre, not the
/// 60 x 30 untransformed box (#3065 review: the scaled ghost was cut to its
/// middle quarter, and its edges left behind).
#[test]
fn a_transformed_unbounded_ghost_damages_its_box_as_drawn() {
    let mut app = app_holding_styled(
        "transform: scale(2)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>",
    );
    let (ghost, moved) = drag_away(&mut app);
    let (w, h) = (ghost.width(), ghost.height());
    assert!(
        (120.0..122.0).contains(&w) && (60.0..62.0).contains(&h),
        "{ghost:?}"
    );
    // Drawn 120 x 60 about the untransformed box's centre.
    let c = ghost.center();
    let drawn = Rect::new(c.x - 60.0, c.y - 30.0, c.x + 60.0, c.y + 30.0);
    assert!(ghost.contains_rect(drawn), "{ghost:?} holds {drawn:?}");
    assert!(covers(&moved, drawn), "{moved:?} repaints {drawn:?}");
    assert!(
        area(&moved) < (W * H) as f64 / 4.0,
        "moving the ghost repaints around it, not the surface: {moved:?}"
    );
}
