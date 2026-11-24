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
