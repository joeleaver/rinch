//! `build_scene` — the GPU and embed paint — consumes the dirty lists.
//!
//! It has no dirty region, so nothing it draws depends on them; what depends
//! on it drawing them away is everything *after*: before #880 no GPU or embed
//! build ever drained `paint_dirty_nodes`, which grew by every change for the
//! life of the app, and a node's painted state (`Node::painted`) has to be
//! advanced by whichever paint draws it. Run with
//! `cargo test -p rinch --features embed --lib build_scene_consume`.

use super::*;

const SIZE: (u32, u32) = (400, 300);

fn mounted() -> (RinchApp, NodeHandle) {
    let slot: Rc<RefCell<Option<NodeHandle>>> = Rc::new(RefCell::new(None));
    let slot_in = slot.clone();
    let mut app = RinchApp::new(move |scope: &mut RenderScope| {
        let root = scope.create_element("div");
        root.set_attribute("style", "position: relative; width: 400px; height: 300px");
        let b = scope.create_element("div");
        b.set_attribute(
            "style",
            "position: absolute; left: 20px; top: 20px; width: 40px; height: 40px; \
             background: rgb(0, 0, 200)",
        );
        root.append_child(&b);
        *slot_in.borrow_mut() = Some(b);
        root
    });
    app.mount_component(SIZE.0 as f32, SIZE.1 as f32);
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let b = slot.borrow().clone().unwrap();
    (app, b)
}

#[test]
fn build_scene_consumes_the_dirty_lists_and_records_what_it_painted() {
    let (mut app, b) = mounted();
    let _ = app.build_scene(1.0, SIZE);
    b.set_style("left", "200px");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let id = b.node_id().0;
    {
        let d = app.doc.as_ref().unwrap().borrow();
        assert!(
            d.tree.paint_dirty_nodes.contains(&id),
            "positive control: the move is pending"
        );
    }
    app.scene_dirty = true;
    let _ = app.build_scene(1.0, SIZE);
    let d = app.doc.as_ref().unwrap().borrow();
    assert!(
        d.tree.paint_dirty_nodes.is_empty(),
        "build_scene left {} paint-dirty entries",
        d.tree.paint_dirty_nodes.len()
    );
    let node = &d.tree.nodes[id];
    assert_eq!(
        node.prev_layout, node.layout,
        "the painted box is the new one"
    );
    assert!(node.painted.is_some(), "and it is recorded as painted");
}

/// A GPU frame leaves the software pixmap behind: the next software frame —
/// a window that switched renderers — must repaint in full, not over a region
/// the GPU frame already consumed.
#[cfg(software_shell)]
#[test]
fn build_scene_invalidates_the_software_frame() {
    let (mut app, b) = mounted();
    let _ = app.build_pixels(1.0, SIZE, false);
    assert!(
        app.has_previous_frame,
        "positive control: a software frame is kept"
    );
    b.set_style("left", "200px");
    app.resolve_and_repaint(SIZE.0 as f32, SIZE.1 as f32);
    let _ = app.build_scene(1.0, SIZE);
    assert!(
        !app.has_previous_frame,
        "the software pixmap no longer has a region that brings it up to date"
    );
}
