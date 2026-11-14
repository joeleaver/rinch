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
    // As `open_pending_window` does, with a main window at scale 1.
    let scale = window.mount_scale(1.0);
    window.mount_before_window(scale);
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
        // The frame, as `follow_content_size` lays it out before asking.
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

// ── the scale a content-sized surface starts at (#2652) ────────────────────

/// Two outputs: `DP-1` at `scale` and `DP-2` at 1.
fn publish_outputs(scale: f64) -> [crate::outputs::OutputId; 2] {
    let ids = [
        crate::outputs::OutputId::from_raw(1),
        crate::outputs::OutputId::from_raw(2),
    ];
    let output = |id, name: &str, scale| crate::outputs::Output {
        id,
        name: Some(name.into()),
        position: (0.0, 0.0),
        size: (1280.0, 720.0),
        scale,
    };
    crate::outputs::publish(
        vec![output(ids[0], "DP-1", scale), output(ids[1], "DP-2", 1.0)],
        crate::outputs::OutputTracking::Live,
    );
    ids
}

/// "hello, sized" in a 16 px face, in a half-pixel border: the border snaps
/// to whole device pixels (1 logical px at scale 1, half of one at 2), so the
/// content's logical size depends on the scale it is measured at.
fn text(scope: &mut RenderScope) -> NodeHandle {
    let node = scope.create_element("div");
    node.set_attribute(
        "style",
        "font: 16px/20px sans-serif; padding: 3px; border: 0.5px solid black",
    );
    node.append_child(&scope.create_text("hello, sized"));
    node
}

/// A content-sized layer surface on `output`.
fn on_output(output: crate::outputs::OutputId) -> WindowProps {
    WindowProps {
        wayland_layer: Some(LayerSurface {
            anchor: LayerAnchor::TOP,
            output_id: Some(output),
            ..Default::default()
        }),
        ..sized(ContentSizing::new())
    }
}

/// The compositor's answer to a request for `asked` (logical) at `scale`:
/// the buffer the fork makes for it, the `SurfaceResized` that reports it,
/// the scale it confirms, then frames. Answers whether a second request
/// follows while the content does not change.
fn asks_again_after(w: &mut ShellWindow, asked: (u32, u32), scale: f64) -> bool {
    let physical = (
        (asked.0 as f64 * scale).round() as u32,
        (asked.1 as f64 * scale).round() as u32,
    );
    let logical = rinch_platform::to_logical(physical, scale);
    w.app.set_granted_size(logical.0 as f32, logical.1 as f32);
    let _ = w
        .app
        .handle_event(PlatformEvent::ScaleFactorChanged(scale), physical, scale);
    w.app.resize_layout(logical.0, logical.1);
    let mut again = false;
    for _ in 0..3 {
        w.app
            .resolve_and_repaint(logical.0 as f32, logical.1 as f32);
        again |= w.app.content_size_changed().is_some();
    }
    again
}

#[test]
fn a_content_sized_surface_is_measured_at_the_scale_it_starts_at() {
    let [dp1, _] = publish_outputs(2.0);
    let w = window(on_output(dp1), text);
    assert_eq!(w.app.device_pixel_ratio, 2.0, "its output's scale");
    let d = w.app.doc.as_ref().unwrap().borrow();
    assert_eq!(
        d.tree.device_scale, 2.0,
        "and the document is laid out at it"
    );
}

#[test]
fn a_window_not_sized_by_its_content_still_mounts_at_the_main_windows_scale() {
    let [dp1, _] = publish_outputs(2.0);
    let plain = WindowProps {
        wayland_layer: Some(LayerSurface {
            output_id: Some(dp1),
            ..Default::default()
        }),
        ..Default::default()
    };
    let w = window(plain, text);
    assert_eq!(w.mount_scale(1.25), 1.25);
    assert_eq!(
        w.app.device_pixel_ratio, 1.0,
        "the main window's, as before"
    );
}

#[test]
fn a_surface_made_for_a_scale_2_output_asks_once() {
    let [dp1, _] = publish_outputs(2.0);
    let mut w = window(on_output(dp1), text);
    w.start_at_surface_scale(true);
    let asked = w.content_surface_size().expect("its content's size");
    assert!(!asks_again_after(&mut w, asked, 2.0), "no second request");
}

#[test]
fn a_window_hidden_at_scale_1_and_shown_at_scale_2_asks_once() {
    let [dp1, dp2] = publish_outputs(2.0);
    let mut w = window(on_output(dp2), text);
    w.start_at_surface_scale(true);
    let first = w.content_surface_size().unwrap();
    assert!(!asks_again_after(&mut w, first, 1.0));
    // Hidden, and moved to the scale-2 output before it is shown again.
    if let Some(props) = w.app.window_props.as_mut() {
        props.wayland_layer.as_mut().unwrap().output_id = Some(dp1);
    }
    w.start_at_surface_scale(false);
    assert_eq!(w.app.device_pixel_ratio, 2.0, "measured at the new scale");
    let shown = w.content_surface_size().unwrap();
    assert!(!asks_again_after(&mut w, shown, 2.0), "no second request");
}

#[test]
fn at_scale_1_5_the_first_buffer_matches_the_ask() {
    let [dp1, _] = publish_outputs(1.5);
    let mut w = window(on_output(dp1), text);
    w.start_at_surface_scale(true);
    let asked = w.content_surface_size().unwrap();
    // An odd width rounds on the way to the buffer and back.
    let physical = (
        (asked.0 as f64 * 1.5).round() as u32,
        (asked.1 as f64 * 1.5).round() as u32,
    );
    assert_eq!(rinch_platform::to_logical(physical, 1.5), asked);
    assert!(!asks_again_after(&mut w, asked, 1.5), "no second request");
}

#[test]
fn a_shown_window_moved_to_another_scale_asks_once_when_its_size_moved() {
    let [_, dp2] = publish_outputs(2.0);
    let mut w = window(on_output(dp2), text);
    w.start_at_surface_scale(true);
    let at_1 = w.content_surface_size().unwrap();
    assert!(!asks_again_after(&mut w, at_1, 1.0));
    // The compositor moves it to a scale-2 output: the half-pixel border is
    // half a logical pixel there, so its size moved.
    let physical = (at_1.0 * 2, at_1.1 * 2);
    let _ = w
        .app
        .handle_event(PlatformEvent::ScaleFactorChanged(2.0), physical, 2.0);
    w.app.resolve_and_repaint(at_1.0 as f32, at_1.1 as f32);
    let at_2 = w.app.content_size_changed().expect("asked for once");
    assert_ne!(at_2, at_1);
    assert!(!asks_again_after(&mut w, at_2, 2.0), "and only once");
}
