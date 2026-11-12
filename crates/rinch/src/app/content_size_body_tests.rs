//! The content box beside the app's `<html>` and `<body>`, and as the
//! scroller (#2552, R0).
//!
//! rinch's UA sheet gives `<body>` `margin: 0; overflow-y: auto`: the body
//! clips what it holds, so a box pinned to the window under it has to be out
//! of its reach (`position: fixed`), or a body margin or border would cut it.

use super::tests::{CHILD, MAX, fixed, grant, styled};
use super::*;
use crate::app::hit_testing::{hit_test, painted_element_box};
use rinch_platform::PlatformEvent;

thread_local! {
    static BODY: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

fn child_id() -> u64 {
    CHILD.with(|c| c.borrow().as_ref().unwrap().node_id().0)
}

/// A content-sized app under the stylesheet `css`, laid out at `MAX`.
fn sized_under(css: &str, build: fn(&mut RenderScope) -> NodeHandle) -> RinchApp {
    let mut app = RinchApp::new_content_sized(build, MAX);
    app.mount_component(MAX.0, MAX.1);
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.load_css(css);
        d.recompute_all_styles_full();
    }
    app.resolve_and_repaint(MAX.0, MAX.1);
    app
}

/// The pixel at `(x, y)` of a software paint at `size`.
fn pixel(app: &mut RinchApp, size: (u32, u32), (x, y): (u32, u32)) -> [u8; 3] {
    app.build_pixels(1.0, size, false);
    let p = app.skia_painter.as_ref().expect("a software painter");
    let i = ((y * p.width() + x) * 4) as usize;
    let d = p.pixels();
    [d[i], d[i + 1], d[i + 2]]
}

const BODY_BOX: &str = "html, body { margin: 20px; padding: 30px; border: 5px solid black }";

fn blue_fixed(scope: &mut RenderScope) -> NodeHandle {
    styled(scope, "width: 300px; height: 40px; background: blue", None)
}

#[test]
fn margin_padding_and_border_on_html_and_body_neither_shift_nor_cut_the_content() {
    let mut app = sized_under(BODY_BOX, blue_fixed);
    assert_eq!(app.content_size(), Some((300, 40)), "not part of the size");
    grant(&mut app, (300, 40));
    {
        let d = app.doc.as_ref().unwrap().borrow();
        let content_box = app.content_box().unwrap().0;
        assert_eq!(
            painted_element_box(&d.tree, content_box),
            (0.0, 0.0, 300.0, 40.0)
        );
        assert_eq!(
            painted_element_box(&d.tree, child_id()),
            (0.0, 0.0, 300.0, 40.0)
        );
        for (x, y) in [(1.0, 1.0), (299.0, 1.0), (1.0, 39.0), (299.0, 39.0)] {
            assert_eq!(hit_test(&d.tree, x, y), Some(child_id()), "hit at {x},{y}");
        }
    }
    for at in [(0, 0), (299, 0), (0, 39), (299, 39)] {
        assert_eq!(
            pixel(&mut app, (300, 40), at),
            [0, 0, 255],
            "painted at {at:?}"
        );
    }
}

#[test]
fn a_background_on_body_covers_the_window_behind_the_content() {
    let mut app = sized_under("body { background: red }", fixed);
    let size = app.content_size().unwrap();
    grant(&mut app, size);
    for at in [(0, 0), (150, 20), (299, 39)] {
        assert_eq!(pixel(&mut app, size, at), [255, 0, 0], "body red at {at:?}");
    }
}

// ── the app's own <body> style, in every order with rinch's writes ─────────

fn body_styled_in_the_component(scope: &mut RenderScope) -> NodeHandle {
    let body = scope.body_handle();
    body.set_attribute("style", "background: red");
    BODY.with(|b| *b.borrow_mut() = Some(body));
    fixed(scope)
}

fn body_kept(scope: &mut RenderScope) -> NodeHandle {
    BODY.with(|b| *b.borrow_mut() = Some(scope.body_handle()));
    fixed(scope)
}

fn body_style(app: &RinchApp) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    let body = d.tree.get(d.tree.body_id).unwrap();
    body.attributes.get("style").cloned().unwrap_or_default()
}

fn box_style_now(app: &RinchApp) -> String {
    let d = app.doc.as_ref().unwrap().borrow();
    let id = app.content_box().unwrap().0;
    d.tree
        .get(id)
        .unwrap()
        .attributes
        .get("style")
        .cloned()
        .unwrap()
}

fn on_body(write: impl FnOnce(&NodeHandle)) {
    BODY.with(|b| write(b.borrow().as_ref().unwrap()));
}

/// The app's body style is intact and the box is rinch's, at `granted`.
fn both_intact(app: &RinchApp, body_has: &[&str], granted: (f32, f32)) {
    let body = body_style(app);
    for want in body_has {
        assert!(body.contains(want), "{want} in the body's style: {body:?}");
    }
    assert_eq!(box_style_now(app), box_style(granted), "the box is rinch's");
}

#[test]
fn the_app_styles_body_inline_in_every_order_with_rinchs_writes() {
    // In the component: after the box was made (nothing of the app's runs
    // before it), before the first layout.
    let mut app = RinchApp::new_content_sized(body_styled_in_the_component, MAX);
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size(), Some((300, 40)));
    both_intact(&app, &["background"], MAX);

    // After the first measure, by `set_attribute` and by `set_style`.
    on_body(|b| b.set_attribute("style", "color: green"));
    on_body(|b| b.set_style("padding", "7px"));
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size_changed(), None, "the body is not the size");
    both_intact(&app, &["color", "padding"], MAX);

    // Across a grant: the grant leaves the body alone, and a body write
    // after it leaves the box alone.
    grant(&mut app, (300, 40));
    both_intact(&app, &["color", "padding"], (300.0, 40.0));
    on_body(|b| b.set_style("margin", "9px"));
    on_body(|b| b.set_attribute("style", "border: 3px solid black"));
    app.resolve_and_repaint(300.0, 40.0);
    assert_eq!(app.content_size_changed(), None);
    both_intact(&app, &["border"], (300.0, 40.0));
    grant(&mut app, (300, 40));
    both_intact(&app, &["border"], (300.0, 40.0));
    assert_eq!(app.content_size(), Some((300, 40)), "the mode still works");
}

#[test]
fn set_style_on_body_before_the_first_layout_is_kept() {
    let mut app = RinchApp::new_content_sized(body_kept, MAX);
    app.mount_component(MAX.0, MAX.1);
    on_body(|b| b.set_style("background", "red"));
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size(), Some((300, 40)));
    both_intact(&app, &["background"], MAX);
}

// ── the box is the scroller ────────────────────────────────────────────────

thread_local! {
    static TARGET: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

/// 300 by 1020: a 1000px spacer, then a 20px target.
fn tall(scope: &mut RenderScope) -> NodeHandle {
    let root = styled(scope, "width: 300px", None);
    let spacer = scope.create_element("div");
    spacer.set_attribute("style", "height: 1000px");
    let target = scope.create_element("div");
    target.set_attribute("style", "height: 20px");
    root.append_child(&spacer);
    root.append_child(&target);
    TARGET.with(|t| *t.borrow_mut() = Some(target));
    root
}

fn box_scroll(app: &RinchApp) -> (f64, f64) {
    let d = app.doc.as_ref().unwrap().borrow();
    d.tree
        .get(app.content_box().unwrap().0)
        .unwrap()
        .scroll_offset
}

/// Content taller than the largest size, in the window it asked for.
fn scrolling() -> RinchApp {
    let mut app = RinchApp::new_content_sized(tall, MAX);
    app.mount_component(MAX.0, MAX.1);
    app.resolve_and_repaint(MAX.0, MAX.1);
    assert_eq!(app.content_size(), Some((300, 600)), "at most the largest");
    grant(&mut app, (300, 600));
    app
}

#[test]
fn the_wheel_scrolls_the_box() {
    let mut app = scrolling();
    app.handle_event(
        PlatformEvent::MouseWheel {
            x: 150.0,
            y: 300.0,
            delta_x: 0.0,
            delta_y: -100.0,
        },
        (300, 600),
        1.0,
    );
    assert!(box_scroll(&app).1 > 0.0, "scrolled: {:?}", box_scroll(&app));
    assert_eq!(app.content_size_changed(), None, "a scroll asks nothing");
}

#[test]
fn scroll_into_view_reaches_the_box() {
    let mut app = scrolling();
    TARGET.with(|t| t.borrow().as_ref().unwrap().scroll_into_view());
    app.resolve_and_repaint(300.0, 600.0);
    // The target spans 1000..1020 in a 600px view: its bottom at the bottom.
    assert_eq!(box_scroll(&app).1, 420.0);
}
