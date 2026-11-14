//! Windows sized by their content (#2552): made, mounted and measured
//! through the runtime without an event loop (no OS window is made).

use super::*;
use rinch_core::element::{ContentSizing, LayerAnchor, LayerAxes, LayerSurface, WindowProps};

thread_local! {
    static CHILD: RefCell<Option<NodeHandle>> = const { RefCell::new(None) };
}

/// A 120 x 40 box.
fn fixed(scope: &mut RenderScope) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute("style", "width: 120px; height: 40px");
    CHILD.with(|c| *c.borrow_mut() = Some(node.clone()));
    node
}

/// 120 wide and 1000 tall: more than any largest size here.
fn tall(scope: &mut RenderScope) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute("style", "width: 120px; height: 1000px");
    node
}

fn sized(sizing: ContentSizing) -> WindowProps {
    WindowProps {
        size_to_content: Some(sizing),
        ..Default::default()
    }
}

/// A second window with `props`, made and mounted as `open_pending_window`
/// does, with no OS window.
fn window(props: WindowProps, content: fn(&mut RenderScope) -> NodeHandle) -> ShellWindow {
    let (title, width, height) = (props.title.clone(), props.width, props.height);
    let app = app_for_window(content, &props);
    let mut app = app;
    app.window_props = Some(props);
    let mut window = ShellWindow::new(app, &title, width, height);
    window.mount_before_window(1.0);
    window
}

fn change_child(style: &str) {
    CHILD.with(|c| c.borrow().as_ref().unwrap().set_attribute("style", style));
}

#[test]
fn a_window_is_made_in_the_mode_only_when_its_props_ask_and_an_axis_is_left() {
    let plain = app_for_window(fixed, &WindowProps::default());
    assert!(!plain.is_content_sized());
    let content = app_for_window(fixed, &sized(ContentSizing::new()));
    assert!(content.is_content_sized());
    let all = LayerAnchor::TOP | LayerAnchor::BOTTOM | LayerAnchor::LEFT | LayerAnchor::RIGHT;
    let assigned = WindowProps {
        wayland_layer: Some(LayerSurface {
            anchor: all,
            compositor_size: LayerAxes::BOTH,
            ..Default::default()
        }),
        ..sized(ContentSizing::new())
    };
    assert!(
        !app_for_window(fixed, &assigned).is_content_sized(),
        "the compositor sizes both axes"
    );
}

#[test]
fn a_window_is_mounted_at_its_largest_size_and_made_at_its_contents() {
    // No outputs are listed here, so the largest size is the props' size,
    // narrowed by the app's maximum.
    let mut w = window(sized(ContentSizing::new().with_max_width(300)), fixed);
    let viewport = w.app.doc.as_ref().unwrap().borrow().tree.viewport;
    assert_eq!((viewport.width, viewport.height), (300.0, 600.0));
    assert_eq!(w.content_surface_size(), Some((120, 40)));
    assert_eq!(w.hidden_size(1.0), (120, 40), "and shown or prepared at it");
}

#[test]
fn content_past_the_largest_size_scrolls_and_the_window_stays_at_it() {
    let mut w = window(sized(ContentSizing::new().with_max_height(200)), tall);
    assert_eq!(w.content_surface_size(), Some((120, 200)));
    let app = &w.app;
    let d = app.doc.as_ref().unwrap().borrow();
    let content_box = d
        .tree
        .get(d.tree.body_id)
        .and_then(|b| b.children.first().copied())
        .expect("the content box");
    let (_, extent) = rinch_dom::paint::scrollbar::content_extents(&d.tree, content_box);
    assert!(
        extent > 200.0,
        "the content is taller than the box: {extent}"
    );
    let box_height = crate::app::hit_testing::painted_element_box(&d.tree, content_box).3;
    assert_eq!(box_height, 200.0, "the box, and so the window, is not");
}

#[test]
fn each_change_of_the_content_asks_once_and_lays_out_once() {
    let mut w = window(sized(ContentSizing::new()), fixed);
    let first = w.content_surface_size().unwrap();
    w.app.set_granted_size(first.0 as f32, first.1 as f32);
    w.app.resize_layout(first.0, first.1);
    let layouts = |w: &ShellWindow| {
        w.app
            .doc
            .as_ref()
            .unwrap()
            .borrow()
            .tree
            .perf
            .get(rinch_dom::perf::Counter::LayoutResolves)
    };
    let mut asks = 0;
    let mut size = first;
    for n in 1..=10u32 {
        change_child(&format!("width: {}px; height: 40px", 120 + n * 10));
        let before = layouts(&w);
        // The frame, as `follow_content_size_before_paint` runs it.
        w.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        assert_eq!(layouts(&w) - before, 1, "one layout for change {n}");
        if let Some(asked) = w.app.content_size_changed() {
            asks += 1;
            size = asked;
            // The window applies it at once and says so: the grant, then the
            // `Resized` event's layout at the new viewport.
            w.app.set_granted_size(size.0 as f32, size.1 as f32);
            w.app.resize_layout(size.0, size.1);
        }
        // A frame with nothing changed asks nothing.
        w.app.resolve_and_repaint(size.0 as f32, size.1 as f32);
        assert_eq!(
            w.app.content_size_changed(),
            None,
            "settled after change {n}"
        );
    }
    assert_eq!(asks, 10);
    assert_eq!(size, (220, 40));
}

#[test]
fn a_new_surface_starts_free_at_its_largest_size() {
    let mut w = window(sized(ContentSizing::new()), fixed);
    w.content_surface_size();
    // The compositor gave less than was asked: limited.
    w.app.set_granted_size(100.0, 40.0);
    assert_eq!(w.app.content_box_limit(), Some((100.0, 600.0)));
    // A show (or a re-creation): free again, and made at the content's size.
    assert_eq!(w.content_surface_size(), Some((120, 40)));
    assert_eq!(w.app.content_box_limit(), Some((800.0, 600.0)));
}

#[test]
fn width_and_height_beside_size_to_content_are_not_used() {
    let mut w = window(
        WindowProps {
            width: 500,
            height: 400,
            ..sized(
                ContentSizing::new()
                    .with_max_width(300)
                    .with_max_height(300),
            )
        },
        fixed,
    );
    assert_eq!(
        w.content_surface_size(),
        Some((120, 40)),
        "the content's size, not 500 x 400"
    );
}
