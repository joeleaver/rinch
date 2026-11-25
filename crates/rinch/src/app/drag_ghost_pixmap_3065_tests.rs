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

/// The window's size, logical.
const W: u32 = 600;
const H: u32 = 400;

/// The app, at a device scale factor (`scale` device pixels per logical
/// one).
struct Page {
    app: RinchApp,
    scale: f64,
}

/// A page with one draggable box at (400, 20), 60 x 30, dark grey, styled
/// `style` too and holding `inner`, at device scale `scale`.
fn page(scale: f64, style: &str, inner: &str) -> Page {
    let style = format!(
        "position: absolute; left: 400px; top: 20px; width: 60px; height: 30px; \
         background: rgb(40, 40, 40); {style}"
    );
    let inner = inner.to_string();
    let app = RinchApp::new(move |scope: &mut RenderScope| {
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
    let mut page = Page { app, scale };
    page.app.mount_component(W as f32, H as f32);
    if scale != 1.0 {
        page.event(PlatformEvent::ScaleFactorChanged(scale));
    }
    let _ = page.frame();
    page
}

impl Page {
    /// The window's size in device pixels.
    fn size(&self) -> (u32, u32) {
        (
            (W as f64 * self.scale) as u32,
            (H as f64 * self.scale) as u32,
        )
    }

    fn event(&mut self, e: PlatformEvent) {
        let (size, scale) = (self.size(), self.scale);
        self.app.handle_event(e, size, scale);
        self.app
            .handle_event(PlatformEvent::AboutToWait, size, scale);
        if self.app.has_pending_layout() {
            self.app.resolve_and_repaint(W as f32, H as f32);
        }
    }

    /// Paints a frame: its pixels and their row width.
    fn frame(&mut self) -> (Vec<u8>, u32) {
        let (size, scale) = (self.size(), self.scale);
        let (px, w, _) = self.app.build_pixels(scale, size, false);
        (px.to_vec(), w)
    }

    /// Grabs the box at its centre (430, 35) and drags it to `to` (logical);
    /// paints that frame.
    fn drag_to(&mut self, to: (f32, f32)) {
        self.event(PlatformEvent::MouseDown {
            x: 430.0,
            y: 35.0,
            button: MouseButton::Left,
        });
        self.event(PlatformEvent::MouseMove { x: to.0, y: to.1 });
        assert!(self.app.active_dnd.is_some(), "the drag activated");
        let _ = self.frame();
    }

    /// Drags the box to (250, 200), so its untransformed box is drawn at
    /// (220, 185) (logical).
    fn drag(&mut self) {
        self.drag_to((250.0, 200.0));
    }

    /// The framebuffer's pixel at device pixel (x, y), RGB.
    fn pixel(&mut self, x: u32, y: u32) -> [u8; 3] {
        let (px, w) = self.frame();
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2]]
    }

    /// Moves the ghost on to (100, 330), paints the frame the damage names,
    /// and then the same frame in full: the pixels that differ (where the
    /// ghost was and the partial frame left it behind), in device pixels.
    fn trails_after_moving_on(&mut self) -> Vec<(u32, u32)> {
        self.event(PlatformEvent::MouseMove { x: 100.0, y: 330.0 });
        let (partial, _) = self.frame();
        self.app.has_previous_frame = false;
        self.app.scene_dirty = true;
        let (full, w) = self.frame();
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

    /// The ghost pixmap's size, device pixels.
    fn pixmap_size(&self) -> (u32, u32) {
        let drag = self.app.active_dnd.as_ref().expect("a drag");
        (drag.snapshot_width, drag.snapshot_height)
    }
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
    let mut app = page(
        1.0,
        "transform: scale(2)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>",
    );
    app.drag();
    // Inside the drawn box, outside the untransformed one, at each corner.
    for (x, y) in [(195, 222), (305, 175), (305, 225), (195, 200)] {
        assert_eq!(app.pixel(x, y), GHOST, "the ghost at ({x}, {y})");
    }
    // Past the drawn box: the page.
    assert_eq!(app.pixel(187, 200), PAGE);
    assert_eq!(app.pixel(313, 200), PAGE);
    assert_eq!(app.trails_after_moving_on(), []);
}

/// Scaled down to half about its centre, unbounded the same way, with a
/// child 120px wide overflowing it: the ghost is clipped to its box as
/// drawn, (235, 192.5) to (265, 207.5), as the GPU ghost is, and moving it
/// on leaves nothing behind (#3065 review: the software pixmap was the
/// untransformed box, larger than the clip its damage was cut to).
#[test]
fn a_scaled_down_unbounded_ghost_is_clipped_to_its_box_as_drawn() {
    let mut app = page(
        1.0,
        "transform: scale(0.5)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>\
         <div style=\"width: 120px; height: 10px; background: rgb(0, 160, 0)\"></div>",
    );
    app.drag();
    // The overflowing child, drawn from x = 235 on, is cut at the box's
    // drawn right edge, 265.
    assert_eq!(app.pixel(262, 200), [0, 160, 0]);
    assert_eq!(app.pixel(268, 200), PAGE);
    assert_eq!(app.trails_after_moving_on(), []);
}

/// An outline past the box (a subtree the walk bounds): the software ghost
/// draws it, as the GPU ghost does, and the frames that move it repaint it.
#[test]
fn an_outline_past_the_box_is_drawn_and_cleared() {
    let mut app = page(1.0, "outline: 3px solid rgb(0, 120, 255)", "");
    app.drag();
    assert_eq!(app.pixel(218, 200), [0, 120, 255]);
    assert_eq!(app.pixel(281, 214), [0, 120, 255]);
    assert_eq!(app.pixel(230, 200), GHOST);
    assert_eq!(app.trails_after_moving_on(), []);
}

/// At 2x, the scaled-up, unbounded ghost of [`a_scaled_up_ghost_is_drawn_whole`]
/// is drawn whole in device pixels, 240 x 120 about the box's centre, (380,
/// 340) to (620, 460), and moving it on leaves nothing behind: the box, its
/// clip and the pixmap are sized at the device scale, not the logical one.
#[test]
fn at_2x_a_scaled_up_ghost_is_drawn_whole() {
    let mut app = page(
        2.0,
        "transform: scale(2)",
        "<div style=\"position: sticky; top: 0; width: 20px; height: 10px; \
         background: rgb(200, 0, 0)\"></div>",
    );
    app.drag();
    assert_eq!(app.pixmap_size(), (240, 120));
    for (x, y) in [(390, 444), (610, 350), (610, 450), (390, 400)] {
        assert_eq!(app.pixel(x, y), GHOST, "the ghost at ({x}, {y})");
    }
    assert_eq!(app.pixel(374, 400), PAGE);
    assert_eq!(app.pixel(626, 400), PAGE);
    assert_eq!(app.trails_after_moving_on(), []);
}

/// At 2x, an outline past the box (a subtree the walk bounds) is drawn 6
/// device pixels wide around the box's 120 x 60, and cleared when the ghost
/// moves on.
#[test]
fn at_2x_an_outline_past_the_box_is_drawn_and_cleared() {
    let mut app = page(2.0, "outline: 3px solid rgb(0, 120, 255)", "");
    app.drag();
    assert_eq!(app.pixmap_size(), (132, 72));
    assert_eq!(app.pixel(436, 400), [0, 120, 255]);
    assert_eq!(app.pixel(562, 428), [0, 120, 255]);
    assert_eq!(app.pixel(460, 400), GHOST);
    assert_eq!(app.trails_after_moving_on(), []);
}

/// A shadow spread 9999px, at 2x (a 1200 x 800 device surface): the pixmap
/// holds what the ghost paints only within the surface's width and height of
/// the grab point, 2400 x 1600, however far the shadow reaches (#3065
/// review: it was bounded around the whole box, and a 1920 x 1080 window at
/// 2x asked for about 175 MB). Dragged to the window's top-left corner, the
/// ghost's shadow still reaches the opposite corner.
#[test]
fn a_far_reaching_shadow_gets_a_pixmap_at_most_twice_the_surface() {
    let mut app = page(2.0, "box-shadow: 0 0 0 9999px rgba(0, 0, 200, 0.5)", "");
    let before = app.pixel(1195, 795);
    app.drag_to((5.0, 5.0));
    assert_eq!(app.pixmap_size(), (2400, 1600));
    // The ghost's own box, at the pointer.
    assert_eq!(app.pixel(10, 10), GHOST);
    // Its shadow, over the source's, at the far corner as in the middle.
    let far = app.pixel(1195, 795);
    assert_ne!(far, before, "the ghost's shadow reaches the far corner");
    assert_eq!(far, app.pixel(600, 400));
}

/// The pixmap's rect: `reach` cut to the surface's width and height either
/// side of the grab point, out to whole pixels, and `reach` itself where it
/// is smaller.
#[test]
fn the_pixmap_rect_is_reach_within_a_surface_of_the_grab_point() {
    use super::drag_ghost_software::ghost_pixmap_rect;
    use peniko::kurbo::Rect;
    let huge = Rect::new(-20_000.0, -20_000.0, 20_000.0, 20_000.0);
    assert_eq!(
        ghost_pixmap_rect(huge, (60.5, 30.0), (1200.0, 800.0)),
        Rect::new(-1140.0, -770.0, 1261.0, 830.0)
    );
    let small = Rect::new(-3.5, -3.0, 63.0, 33.0);
    assert_eq!(
        ghost_pixmap_rect(small, (30.0, 15.0), (1200.0, 800.0)),
        Rect::new(-4.0, -3.0, 63.0, 33.0)
    );
}
