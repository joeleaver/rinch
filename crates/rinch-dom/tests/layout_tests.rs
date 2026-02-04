//! Layout tests for Taffy integration in rinch-dom.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

#[test]
fn test_single_fixed_size_element() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; height: 50px");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(div.0).unwrap().layout;
    assert_eq!(layout.width, 100.0);
    assert_eq!(layout.height, 50.0);
}

#[test]
fn test_flex_row() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 300px",
    );
    doc.append_child(body, container);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 100px; height: 50px");
    doc.append_child(container, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 100px; height: 50px");
    doc.append_child(container, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.x, 0.0);
    assert_eq!(la.width, 100.0);
    assert_eq!(lb.x, 100.0);
    assert_eq!(lb.width, 100.0);
}

#[test]
fn test_flex_column() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: column; width: 300px",
    );
    doc.append_child(body, container);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 100px; height: 50px");
    doc.append_child(container, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 100px; height: 50px");
    doc.append_child(container, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.y, 0.0);
    assert_eq!(la.height, 50.0);
    assert_eq!(lb.y, 50.0);
    assert_eq!(lb.height, 50.0);
}

#[test]
fn test_flex_gap() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; gap: 10px; width: 300px",
    );
    doc.append_child(body, container);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 50px; height: 50px");
    doc.append_child(container, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 50px; height: 50px");
    doc.append_child(container, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.x, 0.0);
    assert_eq!(lb.x, 60.0);
}

#[test]
fn test_padding() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; padding: 20px; width: 200px; height: 100px",
    );
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50px; height: 50px");
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);

    let lchild = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lchild.x, 20.0);
    assert_eq!(lchild.y, 20.0);
}

#[test]
fn test_margin() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "display: flex; width: 300px");
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(
        child,
        "style",
        "width: 50px; height: 50px; margin-left: 10px",
    );
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);

    let lchild = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lchild.x, 10.0);
}

#[test]
fn test_flex_grow() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "display: flex; width: 300px");
    doc.append_child(body, container);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "flex-grow: 1; height: 50px");
    doc.append_child(container, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "flex-grow: 2; height: 50px");
    doc.append_child(container, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.width, 100.0);
    assert_eq!(lb.width, 200.0);
}

#[test]
fn test_nested_flex() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let outer = doc.create_element("div");
    doc.set_attribute(
        outer,
        "style",
        "display: flex; flex-direction: column; width: 200px",
    );
    doc.append_child(body, outer);

    let row1 = doc.create_element("div");
    doc.set_attribute(
        row1,
        "style",
        "display: flex; flex-direction: row; height: 40px",
    );
    doc.append_child(outer, row1);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 80px; height: 40px");
    doc.append_child(row1, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 80px; height: 40px");
    doc.append_child(row1, b);

    let row2 = doc.create_element("div");
    doc.set_attribute(row2, "style", "height: 60px");
    doc.append_child(outer, row2);

    doc.resolve_layout(800.0, 600.0);

    let lr1 = doc.tree.get(row1.0).unwrap().layout;
    let lr2 = doc.tree.get(row2.0).unwrap().layout;
    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;

    assert_eq!(lr1.y, 0.0);
    assert_eq!(lr2.y, 40.0);
    assert_eq!(la.x, 0.0);
    assert_eq!(lb.x, 80.0);
}

#[test]
fn test_display_none() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "display: flex; width: 300px");
    doc.append_child(body, container);

    let hidden = doc.create_element("div");
    doc.set_attribute(hidden, "style", "display: none; width: 100px; height: 50px");
    doc.append_child(container, hidden);

    let visible = doc.create_element("div");
    doc.set_attribute(visible, "style", "width: 100px; height: 50px");
    doc.append_child(container, visible);

    doc.resolve_layout(800.0, 600.0);

    let lhidden = doc.tree.get(hidden.0).unwrap().layout;
    let lvisible = doc.tree.get(visible.0).unwrap().layout;
    assert_eq!(lhidden.width, 0.0);
    assert_eq!(lhidden.height, 0.0);
    assert_eq!(lvisible.x, 0.0);
}

#[test]
fn test_percent_width() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "display: flex; width: 400px");
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50%; height: 50px");
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);

    let lchild = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lchild.width, 200.0);
}

#[test]
fn test_align_items_center() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; align-items: center; width: 200px; height: 100px",
    );
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50px; height: 20px");
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);

    let lchild = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lchild.y, 40.0);
}

#[test]
fn test_justify_content_center() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; justify-content: center; width: 200px; height: 100px",
    );
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 50px; height: 20px");
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);

    let lchild = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lchild.x, 75.0);
}

#[test]
fn test_justify_content_space_between() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; justify-content: space-between; width: 200px",
    );
    doc.append_child(body, container);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 40px; height: 20px");
    doc.append_child(container, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 40px; height: 20px");
    doc.append_child(container, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.x, 0.0);
    assert_eq!(lb.x, 160.0);
}

#[test]
fn test_absolute_position() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; width: 200px; height: 200px",
    );
    doc.append_child(body, container);

    let abs = doc.create_element("div");
    doc.set_attribute(
        abs,
        "style",
        "position: absolute; top: 10px; left: 20px; width: 50px; height: 50px",
    );
    doc.append_child(container, abs);

    doc.resolve_layout(800.0, 600.0);

    let labs = doc.tree.get(abs.0).unwrap().layout;
    assert_eq!(labs.x, 20.0);
    assert_eq!(labs.y, 10.0);
}

#[test]
fn test_display_contents() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 300px",
    );
    doc.append_child(body, container);

    let wrapper = doc.create_element("span");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 100px; height: 50px");
    doc.append_child(wrapper, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 100px; height: 50px");
    doc.append_child(wrapper, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.x, 0.0);
    assert_eq!(lb.x, 100.0);
}

#[test]
fn test_resolve_layout_incremental() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "display: flex; width: 300px");
    doc.append_child(body, container);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 100px; height: 50px");
    doc.append_child(container, child);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(child.0).unwrap().layout.width, 100.0);

    doc.set_attribute(child, "style", "width: 200px; height: 50px");
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(child.0).unwrap().layout.width, 200.0);
}

#[test]
fn test_body_fills_viewport() {
    let mut doc = RinchDocument::new();
    doc.resolve_layout(800.0, 600.0);

    let body_layout = doc.tree.get(doc.body().0).unwrap().layout;
    assert_eq!(body_layout.width, 800.0);
}
