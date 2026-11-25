//! The software drag ghost is a pixmap of everything the ghost draws, and the
//! frames that move it repaint all of that (#3065).
//!
//! The pixmap used to be the dragged box's untransformed border box. A ghost
//! scaled up was cut to its middle, and whatever the subtree paints past its
//! box (an outline, a shadow, an overflowing child) was cut off, though the
//! GPU ghost drew it. These drive the software frame
//! ([`RinchApp::build_pixels`]) and read the framebuffer: where the ghost is
//! drawn, and that a frame which repainted only its damage holds the same
//! pixels as one that repainted everything.

use super::*;

const W: u32 = 600;
const H: u32 = 400;

/// A page with one draggable box at (400, 20), 60 x 30, dark grey, styled
/// `style` too and holding `inner`.
fn app(style: &str, inner: &str) -> RinchApp {
    let style = format!(
        "position: absolute; left: 400px; top: 20px; width: 60px; height: 30px; \
         background: rgb(40, 40, 40); {style}"
    );
    let inner = inner.to_string();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute(
            "style",
            "position: relative; width: 600px; height: 400px; background: rgb(255, 255, 255)",
        );
        let src = scope.create_element("div");
        src.set_attribute("draggable", "true");
        src.set_attribute("style", &style);
        src.set_inner_html(&inner);
        root.append_child(&src);
        root
    });
    app.mount_component(W as f32, H as f32);
    let _ = app.build_pixels(1.0, (W, H), false);
    app
}

fn event(app: &mut RinchApp, e: PlatformEvent) {
    app.handle_event(e, (W, H), 1.0);
    app.handle_event(PlatformEvent::AboutToWait, (W, H), 1.0);
    if app.has_pending_layout() {
        app.resolve_and_repaint(W as f32, H as f32);
    }
}

/// Grabs the box at its centre (430, 35) and drags it to (250, 200), so its
/// untransformed box is drawn at (220, 185); paints that frame.
fn drag(app: &mut RinchApp) {
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
    let _ = app.build_pixels(1.0, (W, H), false);
}

/// The framebuffer's pixel at (x, y), RGB.
fn pixel(app: &mut RinchApp, x: u32, y: u32) -> [u8; 3] {
    let (px, w, _) = app.build_pixels(1.0, (W, H), false);
    let i = ((y * w + x) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

/// Moves the ghost on to (100, 330), paints the frame the damage names, and
/// then the same frame in full: the pixels that differ (where the ghost was
/// and the partial frame left it behind).
fn trails_after_moving_on(app: &mut RinchApp) -> Vec<(u32, u32)> {
    event(app, PlatformEvent::MouseMove { x: 100.0, y: 330.0 });
    let partial = app.build_pixels(1.0, (W, H), false).0.to_vec();
    app.has_previous_frame = false;
    app.scene_dirty = true;
    let (full, w, _) = app.build_pixels(1.0, (W, H), false);
    partial
        .as_chunks::<4>()
        .0
        .iter()
        .zip(full.as_chunks::<4>().0)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| (i as u32 % w, i as u32 / w))
        .take(8)
        .collect()
}

const GHOST: [u8; 3] = [40, 40, 40];
const PAGE: [u8; 3] = [255, 255, 255];

/// Scaled 2x about its centre, holding a `position: sticky` box (a subtree
/// the ghost's walk cannot bound): the ghost is drawn 120 x 60 about the
/// box's centre, (190, 170) to (310, 230), clipped to that, on both
/// renderers. The software pixmap was the 60 x 30 untransformed box, so only
/// the middle quarter was drawn.
#[test]
fn a_scaled_up_ghost_is_drawn_whole() {
    let mut app = app(
        "transform: scale(2)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>",
    );
    drag(&mut app);
    // Inside the drawn box, outside the untransformed one, at each corner.
    for (x, y) in [(195, 222), (305, 175), (305, 225), (195, 200)] {
        assert_eq!(pixel(&mut app, x, y), GHOST, "the ghost at ({x}, {y})");
    }
    // Past the drawn box: the page.
    assert_eq!(pixel(&mut app, 187, 200), PAGE);
    assert_eq!(pixel(&mut app, 313, 200), PAGE);
    assert_eq!(trails_after_moving_on(&mut app), []);
}

/// Scaled down to half about its centre, unbounded the same way, with a
/// child 120px wide overflowing it: the ghost is clipped to its box as
/// drawn, (235, 192.5) to (265, 207.5), as the GPU ghost is, and moving it
/// on leaves nothing behind (#3065 review: the software pixmap was the
/// untransformed box, larger than the clip its damage was cut to).
#[test]
fn a_scaled_down_unbounded_ghost_is_clipped_to_its_box_as_drawn() {
    let mut app = app(
        "transform: scale(0.5)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>\
         <div style=\"width: 120px; height: 10px; background: rgb(0, 160, 0)\"></div>",
    );
    drag(&mut app);
    // The overflowing child, drawn from x = 235 on, is cut at the box's
    // drawn right edge, 265.
    assert_eq!(pixel(&mut app, 262, 200), [0, 160, 0]);
    assert_eq!(pixel(&mut app, 268, 200), PAGE);
    assert_eq!(trails_after_moving_on(&mut app), []);
}

/// An outline past the box (a subtree the walk bounds): the software ghost
/// draws it, as the GPU ghost does, and the frames that move it repaint it.
#[test]
fn an_outline_past_the_box_is_drawn_and_cleared() {
    let mut app = app("outline: 3px solid rgb(0, 120, 255)", "");
    drag(&mut app);
    assert_eq!(pixel(&mut app, 218, 200), [0, 120, 255]);
    assert_eq!(pixel(&mut app, 281, 214), [0, 120, 255]);
    assert_eq!(pixel(&mut app, 230, 200), GHOST);
    assert_eq!(trails_after_moving_on(&mut app), []);
}
