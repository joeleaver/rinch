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

/// An absolute box inside a `position: relative` container: the container is
/// the containing block, so the insets are measured from *it*.
///
/// The container is deliberately offset from the page origin — with it at (0, 0)
/// this test cannot tell a container-relative box from a viewport-relative one,
/// and would pass either way (see `mod absolute_containing_block`, which pins
/// the other case).
#[test]
fn test_absolute_position() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "position: relative; display: flex; width: 200px; height: 200px; margin: 30px 40px",
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
    assert_eq!((labs.x, labs.y), (20.0, 10.0), "parent-relative layout");
    assert_eq!(
        rinch_dom::paint::compute_absolute_position(&doc.tree, abs.0, 1.0),
        (60.0, 40.0),
        "on screen: the positioned container's origin plus the insets"
    );
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

// ============================================================================
// display:contents tests
// ============================================================================

/// Block parent with display:contents child — children should get non-zero layout.
#[test]
fn test_display_contents_block_parent() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    // Block parent (default display for div)
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 300px");
    doc.append_child(body, container);

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 100px; height: 50px");
    doc.append_child(wrapper, child);

    doc.resolve_layout(800.0, 600.0);

    let lc = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(lc.width, 100.0, "child width should be 100px");
    assert_eq!(lc.height, 50.0, "child height should be 50px");
}

/// Incremental relayout with display:contents — calling resolve_layout twice
/// should produce the same correct result (idempotency).
#[test]
fn test_display_contents_incremental_relayout() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 300px",
    );
    doc.append_child(body, container);

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 100px; height: 50px");
    doc.append_child(wrapper, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 100px; height: 50px");
    doc.append_child(wrapper, b);

    // First layout
    doc.resolve_layout(800.0, 600.0);
    let la1 = doc.tree.get(a.0).unwrap().layout;
    let lb1 = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la1.x, 0.0);
    assert_eq!(lb1.x, 100.0);

    // Second layout (must be idempotent)
    doc.resolve_layout(800.0, 600.0);
    let la2 = doc.tree.get(a.0).unwrap().layout;
    let lb2 = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la2.x, 0.0, "second layout: a.x should still be 0");
    assert_eq!(lb2.x, 100.0, "second layout: b.x should still be 100");
    assert_eq!(la2.width, 100.0);
    assert_eq!(lb2.width, 100.0);
}

/// Adjacent display:contents siblings — children from both wrappers should
/// be laid out correctly in the parent.
#[test]
fn test_display_contents_adjacent_siblings() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 400px",
    );
    doc.append_child(body, container);

    // First display:contents wrapper with 2 children
    let wrapper1 = doc.create_element("div");
    doc.set_attribute(wrapper1, "style", "display: contents");
    doc.append_child(container, wrapper1);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 50px; height: 30px");
    doc.append_child(wrapper1, a);

    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "width: 50px; height: 30px");
    doc.append_child(wrapper1, b);

    // Second display:contents wrapper with 1 child
    let wrapper2 = doc.create_element("div");
    doc.set_attribute(wrapper2, "style", "display: contents");
    doc.append_child(container, wrapper2);

    let c = doc.create_element("div");
    doc.set_attribute(c, "style", "width: 50px; height: 30px");
    doc.append_child(wrapper2, c);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    let lc = doc.tree.get(c.0).unwrap().layout;
    assert_eq!(la.x, 0.0, "a should be at x=0");
    assert_eq!(lb.x, 50.0, "b should be at x=50");
    assert_eq!(lc.x, 100.0, "c should be at x=100");
}

/// Nested display:contents (simulating else-if chains) — deeply nested
/// children should still get correct layout.
#[test]
fn test_display_contents_nested() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 400px",
    );
    doc.append_child(body, container);

    // Outer display:contents
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "style", "display: contents");
    doc.append_child(container, outer);

    // Inner display:contents (nested — like else-if chains)
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "display: contents");
    doc.append_child(outer, inner);

    let child = doc.create_element("div");
    doc.set_attribute(child, "style", "width: 80px; height: 40px");
    doc.append_child(inner, child);

    doc.resolve_layout(800.0, 600.0);

    let lc = doc.tree.get(child.0).unwrap().layout;
    assert_eq!(
        lc.width, 80.0,
        "nested contents child should have correct width"
    );
    assert_eq!(
        lc.height, 40.0,
        "nested contents child should have correct height"
    );
}

/// display:contents with a mix of normal and contents siblings.
#[test]
fn test_display_contents_mixed_siblings() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; width: 400px",
    );
    doc.append_child(body, container);

    // Normal sibling first
    let normal = doc.create_element("div");
    doc.set_attribute(normal, "style", "width: 60px; height: 30px");
    doc.append_child(container, normal);

    // Then display:contents wrapper
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "width: 70px; height: 30px");
    doc.append_child(wrapper, a);

    // Another normal sibling after
    let normal2 = doc.create_element("div");
    doc.set_attribute(normal2, "style", "width: 80px; height: 30px");
    doc.append_child(container, normal2);

    doc.resolve_layout(800.0, 600.0);

    let ln = doc.tree.get(normal.0).unwrap().layout;
    let la = doc.tree.get(a.0).unwrap().layout;
    let ln2 = doc.tree.get(normal2.0).unwrap().layout;
    assert_eq!(ln.x, 0.0, "normal sibling at x=0");
    assert_eq!(la.x, 60.0, "contents child at x=60");
    assert_eq!(ln2.x, 130.0, "normal2 at x=130");
}

/// Issue #41: a flex container's `gap` must account for the widths of
/// **inline-level** children that live inside a `display:contents` wrapper
/// (as emitted by rsx `if`/`match`). Before the fix, the contents wrapper was
/// wrongly treated as an inline-formatting-context root, so its children were
/// laid out inline instead of as flex items — collapsing the gap and overlapping.
#[test]
fn test_display_contents_flex_gap_inline_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(
        container,
        "style",
        "display: flex; flex-direction: row; gap: 12px; width: 400px",
    );
    doc.append_child(body, container);

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    // Inline-level children (inline-block) with fixed widths → deterministic.
    let a = doc.create_element("span");
    doc.set_attribute(
        a,
        "style",
        "display: inline-block; width: 50px; height: 20px",
    );
    doc.append_child(wrapper, a);
    let b = doc.create_element("span");
    doc.set_attribute(
        b,
        "style",
        "display: inline-block; width: 60px; height: 20px",
    );
    doc.append_child(wrapper, b);
    let c = doc.create_element("span");
    doc.set_attribute(
        c,
        "style",
        "display: inline-block; width: 40px; height: 20px",
    );
    doc.append_child(wrapper, c);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    let lc = doc.tree.get(c.0).unwrap().layout;
    // gap = 12: a at 0, b at 50+12, c at 62+60+12 — no overlap.
    assert_eq!(la.x, 0.0, "a.x");
    assert_eq!(lb.x, 62.0, "b.x must account for a's width + gap");
    assert_eq!(
        lc.x, 134.0,
        "c.x must account for b's width + gap (not overlap b)"
    );
}

/// Issue #61: a **block** container whose inline content lives inside a
/// `display:contents` wrapper (as emitted by rsx `if`/`match`) must establish
/// the inline formatting context itself and flow the wrapped text in document
/// order. Before the fix the phantom contents wrapper was treated as the IFC
/// root, so the surrounding text nodes were dropped entirely (only inline-block
/// children with a real box survived).
#[test]
fn test_display_contents_block_parent_inline_text() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, container);

    // display:contents wrapper (rsx `if`/`match` emits one of these).
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    // "Hello " <span>bold</span> " world" — text before AND after the span.
    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "font-weight: bold");
    let t2 = doc.create_text("bold");
    let t3 = doc.create_text(" world");
    doc.append_child(wrapper, t1);
    doc.append_child(wrapper, span);
    doc.append_child(span, t2);
    doc.append_child(wrapper, t3);

    doc.resolve_layout(800.0, 600.0);

    // The block CONTAINER establishes the IFC (not the phantom contents wrapper).
    let container_node = doc.tree.get(container.0).unwrap();
    assert!(
        container_node.text_layout.is_some(),
        "the block container must be the IFC root"
    );
    let text_content = container_node
        .text_layout
        .as_ref()
        .unwrap()
        .text_content
        .clone();
    assert!(
        text_content.contains("Hello"),
        "leading text before the span must not be dropped, got {text_content:?}"
    );
    assert!(
        text_content.contains("bold"),
        "span text must be present, got {text_content:?}"
    );
    assert!(
        text_content.contains("world"),
        "trailing text after the span must not be dropped, got {text_content:?}"
    );

    // The display:contents wrapper generates no box → never an IFC root.
    let wrapper_node = doc.tree.get(wrapper.0).unwrap();
    assert!(
        wrapper_node.text_layout.is_none(),
        "display:contents wrapper must not establish an IFC"
    );
}

/// Issue #61: inline-block children inside a `display:contents` wrapper in a
/// **block** parent must flow inline (adjacent, no overlap), just as they would
/// without the wrapper.
#[test]
fn test_display_contents_block_parent_inline_block() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 400px");
    doc.append_child(body, container);

    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(container, wrapper);

    let a = doc.create_element("span");
    doc.set_attribute(
        a,
        "style",
        "display: inline-block; width: 50px; height: 20px",
    );
    doc.append_child(wrapper, a);
    let b = doc.create_element("span");
    doc.set_attribute(
        b,
        "style",
        "display: inline-block; width: 60px; height: 20px",
    );
    doc.append_child(wrapper, b);

    doc.resolve_layout(800.0, 600.0);

    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    // Inline flow: a at x=0, b immediately after at x=50 (no gap in block flow),
    // both on the same line — NOT overlapping.
    assert_eq!(la.x, 0.0, "a.x");
    assert_eq!(lb.x, 50.0, "b.x should follow a inline (not overlap)");
    assert_eq!(la.y, lb.y, "both inline-blocks on the same line");
    // The transparent contents wrapper must contribute ZERO offset — the
    // grandchildren's positions are already relative to the real container, so
    // parent-chain accumulation (paint/hit-test) must not double-count it.
    let lw = doc.tree.get(wrapper.0).unwrap().layout;
    assert_eq!(lw.x, 0.0, "contents wrapper must not add an x offset");
    assert_eq!(lw.y, 0.0, "contents wrapper must not add a y offset");
    assert!(
        doc.tree.get(container.0).unwrap().text_layout.is_some(),
        "the block container must be the IFC root"
    );
    assert!(
        doc.tree.get(wrapper.0).unwrap().text_layout.is_none(),
        "display:contents wrapper must not establish an IFC"
    );
}

/// Issue #61: nested `display:contents` wrappers (as emitted by rsx `else if`
/// chains) in a block parent must still flow their inline content through the
/// block container's IFC.
#[test]
fn test_display_contents_block_parent_nested_inline() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, container);

    let outer = doc.create_element("div");
    doc.set_attribute(outer, "style", "display: contents");
    doc.append_child(container, outer);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "display: contents");
    doc.append_child(outer, inner);

    let t1 = doc.create_text("Nested ");
    let t2 = doc.create_text("text");
    doc.append_child(inner, t1);
    doc.append_child(inner, t2);

    doc.resolve_layout(800.0, 600.0);

    let container_node = doc.tree.get(container.0).unwrap();
    assert!(
        container_node.text_layout.is_some(),
        "the block container must be the IFC root for nested contents"
    );
    let text_content = &container_node.text_layout.as_ref().unwrap().text_content;
    assert!(
        text_content.contains("Nested") && text_content.contains("text"),
        "nested contents text must flow through the container IFC, got {text_content:?}"
    );
    assert!(
        doc.tree.get(outer.0).unwrap().text_layout.is_none(),
        "outer contents wrapper must not establish an IFC"
    );
    assert!(
        doc.tree.get(inner.0).unwrap().text_layout.is_none(),
        "inner contents wrapper must not establish an IFC"
    );
}

// --- min-height on childless block containers -------------------------------
//
// An empty block container is inflated to one line-height so an empty `<p></p>`
// doesn't collapse. That floor used to be written as an unconditional
// assignment, which silently discarded the author's `min-height` for ANY
// childless block — most visibly a `<textarea>` (its value is an attribute, not
// a text child, so it is always childless) once blockified by a flex parent.

#[test]
fn test_empty_block_honors_author_min_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "min-height: 200px; width: 100px");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(div.0).unwrap().layout;
    assert_eq!(
        layout.height, 200.0,
        "an author min-height must survive the empty-block line-height floor"
    );
}

#[test]
fn test_empty_block_min_height_below_line_height_keeps_floor() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    // 5px is far below one line — the line-height floor must still win, so an
    // empty <p> keeps its line box.
    doc.set_attribute(div, "style", "min-height: 5px; width: 100px");
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(div.0).unwrap().layout;
    assert!(
        layout.height > 5.0,
        "line-height floor must still apply when min-height is smaller, got {}",
        layout.height
    );
}

#[test]
fn test_empty_block_explicit_height_still_wins() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sep = doc.create_element("div");
    doc.set_attribute(sep, "style", "height: 1px; width: 100px");
    doc.append_child(body, sep);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(sep.0).unwrap().layout;
    assert_eq!(
        layout.height, 1.0,
        "an explicit height must not be inflated to line-height"
    );
}

#[test]
fn test_blockified_textarea_honors_min_height() {
    // Mirrors the shipped Textarea component: a flex-column wrapper blockifies
    // the <textarea>, which is childless, so it took the empty-block path.
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let style = doc.create_element("style");
    let css = doc
        .create_text(".wrap { display: flex; flex-direction: column; } .ta { min-height: 80px; }");
    doc.append_child(style, css);
    doc.append_child(body, style);

    let wrap = doc.create_element("div");
    doc.set_attribute(wrap, "class", "wrap");
    doc.append_child(body, wrap);
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "class", "ta");
    doc.append_child(wrap, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    assert_eq!(
        layout.height, 80.0,
        "a blockified <textarea> must honor its CSS min-height"
    );
}

#[test]
fn test_empty_block_line_floor_survives_a_restyle() {
    // The floor is written onto the Taffy style by the IFC pass, which only runs
    // on a structural change; `apply_stylo_styles_to_taffy` rebuilds that style
    // from the computed values on *every* restyle. Applied in the IFC pass
    // alone, the floor was discarded the first time anything re-resolved the
    // node's style, and nothing put it back.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 100px; font-size: 10px; line-height: 20px",
    );
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.height,
        20.0,
        "an empty block starts one line tall"
    );

    // A style-only change: no child added or removed, so the IFC pass does not
    // re-run.
    doc.set_attribute(div, "data-state", "open");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.height,
        20.0,
        "the line-height floor must survive a restyle"
    );
}

#[test]
fn test_blockified_input_keeps_its_height_when_focused() {
    // The shipped case: a search field is a flex item, so it blockifies and
    // takes the childless-block path — an `<input>` holds its value in an
    // attribute, so it has no child to give it a height. Focusing it writes
    // `data-focused`/`data-cursor-pos` and moves `:focus`, which re-resolves its
    // style. The field collapsed to zero height, and `paint_node` skips
    // zero-size boxes: the whole control — background, value and caret —
    // disappeared and did not come back while the process lived.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let row = doc.create_element("div");
    doc.set_attribute(
        row,
        "style",
        "display: flex; align-items: center; width: 300px",
    );
    doc.append_child(body, row);

    let input = doc.create_element("input");
    doc.set_attribute(
        input,
        "style",
        "flex: 1; font-size: 10px; line-height: 20px",
    );
    doc.set_attribute(input, "value", "wag");
    doc.append_child(row, input);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(
        doc.tree.get(input.0).unwrap().layout.height,
        20.0,
        "an unfocused input is one line tall"
    );

    doc.set_attribute(input, "data-focused", "true");
    doc.set_attribute(input, "data-cursor-pos", "3");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(input.0).unwrap().layout.height,
        20.0,
        "a focused input must keep its height, or nothing about it is painted"
    );
}

/// A childless block whose only height is the one-line floor, laid out once so
/// the floor is already on its Taffy style.
///
/// `apply_stylo_styles_to_taffy` is not the only pass that rebuilds a node's
/// Taffy style from the computed values: `tick_transitions` and
/// `tick_animations` do the same for every node with an active
/// transition/animation or sitting in `dirty_nodes`, and they run on the event
/// loop's idle tick *before* the next layout. Each must re-apply the floor.
///
/// The two ticks get a test apiece rather than one test that calls both: both
/// rebuild the *whole* style, so whichever runs last decides the outcome, and a
/// combined test passes with the earlier tick's call deleted.
fn empty_block_floored_at_one_line() -> (RinchDocument, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 100px; font-size: 10px; line-height: 20px",
    );
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.tree.get(div.0).unwrap().layout.height, 20.0);

    (doc, div.0)
}

/// The next layout after a tick must not find a floorless style.
fn assert_floor_survived_relayout(doc: &mut RinchDocument, div: usize, what: &str) {
    doc.tree.layout_dirty = true;
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(div).unwrap().layout.height,
        20.0,
        "{what} must not discard the line-height floor"
    );
}

#[test]
fn test_empty_block_line_floor_survives_a_transition_tick() {
    // A `<input class="rinch-number-input">` carries
    // `transition: border-color 150ms`, so focusing one puts it on exactly this
    // path — with no animation tick behind it to rebuild the style again.
    let (mut doc, div) = empty_block_floored_at_one_line();

    doc.tree.dirty_nodes.insert(div);
    doc.tick_transitions();

    assert_floor_survived_relayout(&mut doc, div, "a transition tick");
}

#[test]
fn test_empty_block_line_floor_survives_an_animation_tick() {
    let (mut doc, div) = empty_block_floored_at_one_line();

    doc.tree.dirty_nodes.insert(div);
    doc.tick_animations();

    assert_floor_survived_relayout(&mut doc, div, "an animation tick");
}

/// The line floor is not the only override those two ticks used to drop: an
/// out-of-flow box whose containing block is not its Taffy parent has its
/// *size* baked from that containing block, and rebuilding the style from the
/// computed values discards the bake just as thoroughly. `tick_transitions` and
/// `tick_animations` never had the `position: fixed` bake at all, so a
/// transition frame on a fixed box collapsed it onto its parent; the ICB
/// absolute of #204 rides the same helper and would have inherited the drift.
///
/// One document holds both cases; the split is by tick, for the reason
/// `empty_block_floored_at_one_line` records — whichever tick runs last decides.
fn out_of_flow_boxes_sized_from_the_viewport() -> (RinchDocument, usize, usize) {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    // Each box gets a `height: 100%` child: a fixed box's own `LayoutResult` is
    // rewritten from the viewport after layout either way, so only a child can
    // see whether the *Taffy* size survived.
    fn filling(
        doc: &mut RinchDocument,
        parent: rinch_core::dom::NodeId,
        style: &str,
    ) -> (rinch_core::dom::NodeId, rinch_core::dom::NodeId) {
        let box_ = doc.create_element("div");
        doc.set_attribute(box_, "style", style);
        doc.append_child(parent, box_);
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", "width: 100%; height: 100%");
        doc.append_child(box_, child);
        (box_, child)
    }

    // Both boxes hang off a small unpositioned host, so Taffy's own
    // parent-based answer (100x50) is nothing like the viewport and a dropped
    // bake is unmistakable.
    let host = doc.create_element("div");
    doc.set_attribute(host, "style", "width: 100px; height: 50px");
    doc.append_child(body, host);
    let (_icb_absolute, absolute_child) = filling(&mut doc, host, "position: absolute; inset: 0");
    let (_fixed, fixed_child) = filling(&mut doc, host, "position: fixed; inset: 0");

    doc.resolve_layout(800.0, 600.0);
    for (what, node) in [
        ("the fixed box", fixed_child),
        ("the ICB absolute", absolute_child),
    ] {
        let l = doc.tree.get(node.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (800.0, 600.0),
            "{what}'s child starts out filling the viewport"
        );
    }

    (doc, fixed_child.0, absolute_child.0)
}

fn assert_viewport_size_survived_relayout(
    doc: &mut RinchDocument,
    fixed_child: usize,
    absolute_child: usize,
    what: &str,
) {
    doc.tree.layout_dirty = true;
    doc.resolve_layout(800.0, 600.0);

    for (which, node) in [
        ("a fixed box", fixed_child),
        ("an ICB absolute", absolute_child),
    ] {
        let l = doc.tree.get(node).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (800.0, 600.0),
            "{what} must not drop {which}'s viewport-derived size — its child \
             is laid out against it"
        );
    }
}

#[test]
fn test_out_of_flow_viewport_size_survives_a_transition_tick() {
    let (mut doc, fixed_child, absolute_child) = out_of_flow_boxes_sized_from_the_viewport();

    let parents: Vec<usize> = [fixed_child, absolute_child]
        .iter()
        .map(|&n| doc.tree.get(n).unwrap().parent.unwrap())
        .collect();
    for node in parents {
        doc.tree.dirty_nodes.insert(node);
    }
    doc.tick_transitions();

    assert_viewport_size_survived_relayout(
        &mut doc,
        fixed_child,
        absolute_child,
        "a transition tick",
    );
}

#[test]
fn test_out_of_flow_viewport_size_survives_an_animation_tick() {
    let (mut doc, fixed_child, absolute_child) = out_of_flow_boxes_sized_from_the_viewport();

    let parents: Vec<usize> = [fixed_child, absolute_child]
        .iter()
        .map(|&n| doc.tree.get(n).unwrap().parent.unwrap())
        .collect();
    for node in parents {
        doc.tree.dirty_nodes.insert(node);
    }
    doc.tick_animations();

    assert_viewport_size_survived_relayout(
        &mut doc,
        fixed_child,
        absolute_child,
        "an animation tick",
    );
}

#[test]
fn test_empty_block_author_min_height_survives_a_restyle() {
    // The floor is a floor on both passes: re-applying it on restyle must not
    // start stomping an author `min-height` that is larger.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(
        div,
        "style",
        "width: 100px; font-size: 10px; line-height: 20px; min-height: 200px",
    );
    doc.append_child(body, div);

    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(div, "data-state", "open");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(div.0).unwrap().layout.height,
        200.0,
        "an author min-height must survive the floor on a restyle too"
    );
}

#[test]
fn test_explicit_height_survives_a_restyle_uninflated() {
    // The mirror of the above: a `height: 1px` separator must not be inflated to
    // a line by the floor's new call site either.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let sep = doc.create_element("div");
    doc.set_attribute(sep, "style", "width: 100px; height: 1px; line-height: 20px");
    doc.append_child(body, sep);

    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(sep, "data-state", "open");
    doc.resolve_layout(800.0, 600.0);

    assert_eq!(
        doc.tree.get(sep.0).unwrap().layout.height,
        1.0,
        "an explicit height must not be inflated on a restyle"
    );
}

#[test]
fn test_empty_block_percentage_min_height_resolves() {
    // Percentage min-height on a childless block used to be flattened to the
    // line-height floor, because Taffy 0.9 could not resolve a percentage
    // min-height against a block containing block at all. Since the 0.12
    // upgrade it resolves, so the floor must no longer stomp it.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "style", "height: 400px; width: 100px");
    doc.append_child(body, outer);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", "min-height: 50%");
    doc.append_child(outer, inner);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(inner.0).unwrap().layout;
    assert_eq!(
        layout.height, 200.0,
        "a percentage min-height must resolve against the containing block"
    );
}

// --- <textarea rows=N> intrinsic height -------------------------------------

#[test]
fn test_textarea_rows_sets_intrinsic_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "rows", "6");
    doc.set_attribute(ta, "style", "font-size: 10px; line-height: 20px");
    doc.append_child(body, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    assert_eq!(
        layout.height, 120.0,
        "rows=6 at line-height 20px must give a 6-line box"
    );
}

#[test]
fn test_textarea_default_rows_is_two() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "style", "font-size: 10px; line-height: 20px");
    doc.append_child(body, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    assert_eq!(
        layout.height, 40.0,
        "a <textarea> with no rows attribute defaults to 2 rows"
    );
}

#[test]
fn test_textarea_rows_includes_padding_and_border() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "rows", "3");
    doc.set_attribute(
        ta,
        "style",
        "font-size: 10px; line-height: 20px; padding: 5px; border: 2px solid black",
    );
    doc.append_child(body, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    // 3 * 20 line boxes + 5+5 padding + 2+2 border (min-height is border-box).
    assert_eq!(
        layout.height, 74.0,
        "rows height must add padding and border on top of the line boxes"
    );
}

#[test]
fn test_textarea_larger_min_height_beats_rows() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "rows", "2");
    doc.set_attribute(
        ta,
        "style",
        "font-size: 10px; line-height: 20px; min-height: 300px",
    );
    doc.append_child(body, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    assert_eq!(
        layout.height, 300.0,
        "an author min-height larger than the rows height must win"
    );
}

#[test]
fn test_textarea_explicit_height_beats_rows() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let ta = doc.create_element("textarea");
    doc.set_attribute(ta, "rows", "10");
    doc.set_attribute(
        ta,
        "style",
        "font-size: 10px; line-height: 20px; height: 55px",
    );
    doc.append_child(body, ta);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(ta.0).unwrap().layout;
    assert_eq!(
        layout.height, 55.0,
        "an explicit CSS height must override the rows-derived height"
    );
}

// --- window chrome inset for fixed overlays ---------------------------------
//
// rinch draws some window chrome itself (the Linux in-app menu bar, the
// BorderlessWindow titlebar) and reserves space for it with in-document
// padding. A `position: fixed` overlay resolves against the real viewport and
// so correctly ignores that padding — which is what a browser does, and what
// rinch-web does on the real DOM. The chrome therefore publishes its height as
// `--rinch-window-top-inset` and overlays opt in. These pin the mechanism the
// component stylesheets depend on.

#[test]
fn test_fixed_overlay_can_offset_by_inherited_inset_var() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "--rinch-window-top-inset: 28px");
    doc.append_child(body, wrapper);

    let overlay = doc.create_element("div");
    doc.set_attribute(
        overlay,
        "style",
        "position: fixed; top: var(--rinch-window-top-inset, 0px); \
         left: 0; width: 100px; height: 40px",
    );
    doc.append_child(wrapper, overlay);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(overlay.0).unwrap().layout;
    assert_eq!(
        layout.y, 28.0,
        "a fixed overlay must pick up the chrome inset published by an ancestor"
    );
}

#[test]
fn test_fixed_overlay_without_inset_var_uses_fallback() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let overlay = doc.create_element("div");
    doc.set_attribute(
        overlay,
        "style",
        "position: fixed; top: var(--rinch-window-top-inset, 0px); \
         left: 0; width: 100px; height: 40px",
    );
    doc.append_child(body, overlay);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(overlay.0).unwrap().layout;
    assert_eq!(
        layout.y, 0.0,
        "with no chrome, the fallback must leave the overlay at the viewport top"
    );
}

#[test]
fn test_inset_var_composes_in_calc() {
    // What the top-anchored Notification variants do.
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "--rinch-window-top-inset: 36px");
    doc.append_child(body, wrapper);

    let note = doc.create_element("div");
    doc.set_attribute(
        note,
        "style",
        "position: fixed; top: calc(var(--rinch-window-top-inset, 0px) + 16px); \
         left: 0; width: 100px; height: 40px",
    );
    doc.append_child(wrapper, note);

    doc.resolve_layout(800.0, 600.0);

    let layout = doc.tree.get(note.0).unwrap().layout;
    assert_eq!(layout.y, 52.0, "inset must compose inside calc()");
}

// --- percentage min/max-height on auto-height boxes -------------------------
//
// Taffy 0.9's block algorithm hard-coded the percentage-resolution basis for
// in-flow block children to an indefinite height, so a percentage min/max-height
// on an auto-height block child of a block parent silently evaporated (it read
// as the content height). Fixed by the 0.12 upgrade. These pin the behaviour,
// including the cases a fix must NOT over-resolve.

fn pct_doc(parent_style: &str, child_style: &str, content_height: Option<&str>) -> f32 {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let outer = doc.create_element("div");
    doc.set_attribute(outer, "style", parent_style);
    doc.append_child(body, outer);
    let inner = doc.create_element("div");
    doc.set_attribute(inner, "style", child_style);
    doc.append_child(outer, inner);
    if let Some(h) = content_height {
        let grandchild = doc.create_element("div");
        doc.set_attribute(grandchild, "style", h);
        doc.append_child(inner, grandchild);
    }
    doc.resolve_layout(800.0, 600.0);
    doc.tree.get(inner.0).unwrap().layout.height
}

const DEFINITE_PARENT: &str = "width: 400px; height: 400px;";

#[test]
fn test_percentage_min_height_on_auto_height_block() {
    let h = pct_doc(
        DEFINITE_PARENT,
        "min-height: 50%; width: 10px",
        Some("height: 22px"),
    );
    assert_eq!(
        h, 200.0,
        "percentage min-height must beat the content height"
    );
}

#[test]
fn test_percentage_min_height_100_percent() {
    // The canonical "fill the scroll viewport" idiom.
    let h = pct_doc(
        DEFINITE_PARENT,
        "min-height: 100%; width: 10px",
        Some("height: 22px"),
    );
    assert_eq!(h, 400.0);
}

#[test]
fn test_percentage_max_height_on_auto_height_block() {
    let h = pct_doc(
        DEFINITE_PARENT,
        "max-height: 50%; width: 10px",
        Some("height: 900px"),
    );
    assert_eq!(
        h, 200.0,
        "percentage max-height must clamp the content height"
    );
}

#[test]
fn test_percentage_min_height_indefinite_parent_not_over_resolved() {
    // CSS: a percentage against an indefinite containing block height does not
    // resolve. The box must stay at its content height, NOT jump to some
    // invented value — this is the case a fix can most easily get wrong.
    let h = pct_doc(
        "width: 400px;",
        "min-height: 50%; width: 10px",
        Some("height: 22px"),
    );
    assert_eq!(h, 22.0);
}

#[test]
fn test_percentage_min_height_block_inside_flex_column() {
    // The containing block takes its height from its own style rather than from
    // known_dimensions here, which is a distinct resolution path.
    let h = pct_doc(
        "width: 400px; height: 400px; display: flex; flex-direction: column;",
        "min-height: 50%; width: 10px",
        Some("height: 22px"),
    );
    assert_eq!(h, 200.0);
}

#[test]
fn test_empty_block_line_height_floor_survives_percentage_work() {
    // The floor must still apply when the author asked for nothing...
    let bare = pct_doc(DEFINITE_PARENT, "width: 10px", None);
    assert!(bare > 0.0 && bare < 30.0, "expected one line, got {bare}");

    // ...and when the author's min-height is smaller than a line.
    let tiny = pct_doc(DEFINITE_PARENT, "min-height: 5px; width: 10px", None);
    assert_eq!(
        tiny, bare,
        "a sub-line min-height must not shrink the floor"
    );

    // ...but an explicit height still wins outright (separators).
    let sep = pct_doc(DEFINITE_PARENT, "height: 1px; width: 10px", None);
    assert_eq!(sep, 1.0);
}
// ── Viewport-resize relayout (native prose "min-content" bug) ─────────────
//
// The native-only bug: prose injected via `set_inner_html` and first laid out
// at a degenerate/tiny viewport (the window before its first real resize) kept
// its narrow first-layout width — often min-content, one word per line — after
// the window grew. The viewport IS the layout root's available space, so a size
// change must force a Taffy recompute even when no node's Taffy *style* changed
// (an all-`auto`/fixed subtree produces identical Taffy styles at every size).
// Before the fix, `resolve_layout`'s `if !layout_dirty { return }` early-out
// stranded such trees at their first-layout width. See `resolve_layout`.

/// Prose injected via `set_inner_html` and first laid out in a tiny viewport
/// must fill the container once the viewport grows — not stay stuck at the
/// narrow first-layout (min-content) width. This is the reader/typography-page
/// symptom (prose wrapping at ~its longest word).
#[test]
fn test_prose_fills_after_viewport_grows() {
    const PROSE: &str = "The quick brown fox jumps over the lazy dog and \
continues running across the meadow toward the distant treeline where the \
shadows gather at dusk.";

    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    // width:auto — the container derives its width from the viewport.
    doc.append_child(body, container);
    doc.set_inner_html(container, &format!("<p>{PROSE}</p>"));

    // First layout while the window is still tiny (e.g. before the first real
    // resize event), then grow to a real size.
    doc.resolve_layout(20.0, 600.0);
    doc.resolve_layout(1000.0, 600.0);

    let container_w = doc.tree.get(container.0).unwrap().layout.width;
    let p_id = rinch_dom::testing::query_selector(&doc.tree, "p")[0];
    let p_w = doc.tree.get(p_id).unwrap().layout.width;

    assert!(
        (container_w - 1000.0).abs() < 1.0,
        "auto-width container must track the grown viewport, got {container_w}"
    );
    assert!(
        p_w > 900.0,
        "prose <p> must fill the grown container, got {p_w} (container {container_w})"
    );
}

/// General correctness: a `grid-template-columns: 120px 1fr` grid in a
/// definite-width block resolves the `1fr` track to the remaining space.
/// (This path was already correct — the Stylo→Taffy `fr` translation works;
/// see the note on the ignored test below and the investigation report.)
#[test]
fn test_grid_1fr_track_fills_remaining_space() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let grid = doc.create_element("div");
    doc.set_attribute(
        grid,
        "style",
        "display: grid; grid-template-columns: 120px 1fr; width: 600px; height: 40px;",
    );
    doc.append_child(body, grid);
    let fixed = doc.create_element("div");
    doc.append_child(grid, fixed);
    let flex = doc.create_element("div");
    doc.append_child(grid, flex);

    doc.resolve_layout(1000.0, 600.0);

    let fixed_w = doc.tree.get(fixed.0).unwrap().layout.width;
    let flex_w = doc.tree.get(flex.0).unwrap().layout.width;
    assert!((fixed_w - 120.0).abs() < 1.0, "fixed track, got {fixed_w}");
    assert!(
        (flex_w - 480.0).abs() < 1.0,
        "1fr track should be 480, got {flex_w}"
    );
}

// ── inline-block percentage width (was: native grid "1fr → 0" bug) ─────────
//
// Regression test for issue #120.
//
// The native "grid collapses / children squeezed to ~22×14" report was an
// *inline-block* bug, not a grid-track bug: an inline-block (an `<input>`
// defaults to inline-block) with a percentage main-size collapsed to
// min-content. `compute_inline_block_layouts` pre-measures inline-blocks
// detached from Taffy under `AvailableSpace::MaxContent`, so a `width: 100%`
// had no definite containing block to resolve against and fell to ~0 (just
// padding). A block-level `width: 100%` always resolved correctly, and a
// *definite* inline-block width (`200px`) passed through — only percentages
// broke.
//
// Fixed by `resolve_percentage_inline_blocks`, which re-measures percentage
// inline-blocks against their containing block's *computed* width after the
// root Taffy compute, then re-runs that compute so the enclosing IFCs
// line-break against the corrected boxes.
#[test]
fn test_inline_block_percent_width_fills_containing_block() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 700px; height: 40px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; width: 100%;");
    doc.append_child(cell, ib);

    doc.resolve_layout(1000.0, 800.0);

    let ib_w = doc.tree.get(ib.0).unwrap().layout.width;
    // Desired (browser) behaviour: 100% of the 700px containing block.
    // Currently produces ~0 — this assert fails until the bug is fixed.
    assert!(
        ib_w > 690.0,
        "inline-block width:100% should fill its 700px containing block, got {ib_w}"
    );
}

/// A `position: fixed` block box with `height: auto` must size to the sum of its
/// stacked block children — not collapse to one child's height. Regression for a
/// bug that made a fixed popup/menu appended to <body> show only its first row.
#[test]
fn test_fixed_block_auto_height_sums_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: fixed; left: 10px; top: 10px; padding: 4px;",
    );
    doc.append_child(body, panel);
    for _ in 0..8 {
        let row = doc.create_element("div");
        doc.set_attribute(row, "style", "height: 30px;");
        doc.append_child(panel, row);
    }
    doc.resolve_layout(1000.0, 800.0);
    let h = doc.tree.get(panel.0).unwrap().layout.height;
    // 8 * 30 + 4 + 4 padding = 248.
    assert!(
        (h - 248.0).abs() < 2.0,
        "fixed auto-height should sum children (expected ~248), got {h}"
    );
}

/// The same fixed box clamps to its `max-height` (and scrolls the overflow)
/// rather than growing past the cap.
#[test]
fn test_fixed_block_auto_height_clamps_to_max_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let panel = doc.create_element("div");
    doc.set_attribute(
        panel,
        "style",
        "position: fixed; left: 10px; top: 10px; max-height: 120px; overflow-y: auto; padding: 4px;",
    );
    doc.append_child(body, panel);
    for _ in 0..8 {
        let row = doc.create_element("div");
        doc.set_attribute(row, "style", "height: 30px;");
        doc.append_child(panel, row);
    }
    doc.resolve_layout(1000.0, 800.0);
    let h = doc.tree.get(panel.0).unwrap().layout.height;
    assert!(
        (h - 120.0).abs() < 1.0,
        "fixed auto-height must clamp to max-height 120, got {h}"
    );
}

/// The percentage resolves against the containing block's *content* box, so
/// horizontal padding on the containing block shrinks it.
#[test]
fn test_inline_block_percent_width_excludes_container_padding() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(
        cell,
        "style",
        "width: 700px; height: 40px; padding: 0 20px;",
    );
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; width: 100%;");
    doc.append_child(cell, ib);

    doc.resolve_layout(1000.0, 800.0);

    let ib_w = doc.tree.get(ib.0).unwrap().layout.width;
    assert!(
        (ib_w - 660.0).abs() < 1.0,
        "should be 700 - 2*20 padding = 660, got {ib_w}"
    );
}

/// A percentage `max-width` must clamp the inline-block *and* re-wrap its text.
/// This is the case that proves the second layout pass does its job: the box is
/// corrected after the first compute, so the enclosing IFC has to be measured
/// again for the taller, narrower result to appear.
#[test]
fn test_inline_block_percent_max_width_clamps_and_rewraps() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 700px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; max-width: 175px;");
    doc.append_child(cell, ib);
    let text = doc.create_text("hello world this is some text that could wrap somewhere");
    doc.append_child(ib, text);

    doc.resolve_layout(1000.0, 800.0);

    let l = doc.tree.get(ib.0).unwrap().layout;
    assert!(
        (l.width - 175.0).abs() < 1.0,
        "max-width 25% of 700 should clamp to 175, got {}",
        l.width
    );
    assert!(
        l.height > 40.0,
        "clamped text must wrap to multiple lines, got height {}",
        l.height
    );
}

/// A percentage `min-width` floors an otherwise shrink-to-fit inline-block.
#[test]
fn test_inline_block_percent_min_width_floors_shrink_to_fit() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 700px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; min-width: 50%;");
    doc.append_child(cell, ib);
    let text = doc.create_text("hi");
    doc.append_child(ib, text);

    doc.resolve_layout(1000.0, 800.0);

    let ib_w = doc.tree.get(ib.0).unwrap().layout.width;
    assert!(
        (ib_w - 350.0).abs() < 1.0,
        "min-width 50% of 700 should floor at 350, got {ib_w}"
    );
}

/// An `auto`-width inline-block must keep shrink-to-fit sizing — the percentage
/// fix must not start stretching every inline-block to its containing block.
#[test]
fn test_inline_block_auto_width_still_shrinks_to_fit() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 700px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block;");
    doc.append_child(cell, ib);
    let text = doc.create_text("hi");
    doc.append_child(ib, text);

    doc.resolve_layout(1000.0, 800.0);

    let ib_w = doc.tree.get(ib.0).unwrap().layout.width;
    assert!(
        ib_w > 0.0 && ib_w < 200.0,
        "auto inline-block must hug its content, not fill 700, got {ib_w}"
    );
}

/// The percentage must be re-resolved on every layout, not frozen at first
/// layout — otherwise a window resize strands the inline-block at its old width.
#[test]
fn test_inline_block_percent_width_tracks_container_resize() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 50%; height: 40px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; width: 100%;");
    doc.append_child(cell, ib);

    doc.resolve_layout(1000.0, 800.0);
    let before = doc.tree.get(ib.0).unwrap().layout.width;
    assert!(
        (before - 500.0).abs() < 1.0,
        "100% of a 50%-of-1000 container should be 500, got {before}"
    );

    doc.resolve_layout(600.0, 800.0);
    let after = doc.tree.get(ib.0).unwrap().layout.width;
    assert!(
        (after - 300.0).abs() < 1.0,
        "after resize to 600 viewport it should be 300, got {after}"
    );
}

/// The symptom that originally surfaced #120: an `<input>` (inline-block by
/// default) with `width: 100%` inside a grid cell rendered ~22px wide, which
/// read as "the grid collapsed". The grid tracks were always correct.
#[test]
fn test_percent_width_input_in_grid_cell_fills_track() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let grid = doc.create_element("div");
    doc.set_attribute(
        grid,
        "style",
        "display: grid; grid-template-columns: 120px 1fr; width: 600px;",
    );
    doc.append_child(body, grid);
    let label = doc.create_element("div");
    doc.append_child(grid, label);
    let cell = doc.create_element("div");
    doc.append_child(grid, cell);
    let input = doc.create_element("input");
    doc.set_attribute(input, "style", "width: 100%;");
    doc.append_child(cell, input);

    doc.resolve_layout(1000.0, 600.0);

    let input_w = doc.tree.get(input.0).unwrap().layout.width;
    assert!(
        input_w > 470.0,
        "input width:100% should fill the 480px 1fr track, got {input_w}"
    );
}

/// The correction must converge: laying out repeatedly at the same size has to
/// settle on the same width. If it did not, every frame would pay for a second
/// Taffy pass and the box could visibly oscillate.
#[test]
fn test_inline_block_percent_width_is_stable_across_relayouts() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let cell = doc.create_element("div");
    doc.set_attribute(cell, "style", "width: 700px; height: 40px;");
    doc.append_child(body, cell);
    let ib = doc.create_element("div");
    doc.set_attribute(ib, "style", "display: inline-block; width: 100%;");
    doc.append_child(cell, ib);

    let mut widths = Vec::new();
    for _ in 0..4 {
        doc.resolve_layout(1000.0, 800.0);
        widths.push(doc.tree.get(ib.0).unwrap().layout.width);
    }

    assert!(
        widths.iter().all(|w| (w - 700.0).abs() < 1.0),
        "width must settle at 700 on every layout, got {widths:?}"
    );
}

/// #144: layout-time scroll clamping must not be a silent mutation. When a
/// scroll container's content shrinks, `resolve_layout` clamps the stale
/// offset AND queues a (node, clamped offset) pair for the runtime to drain
/// and dispatch as a scroll event — coalesced to one entry per node with the
/// final value, even when layout resolves more than once before the drain.
#[test]
fn test_scroll_clamp_is_queued_for_notification() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "height: 100px; overflow-y: auto");
    doc.append_child(body, container);
    let content = doc.create_element("div");
    doc.set_attribute(content, "style", "height: 500px");
    doc.append_child(container, content);

    doc.resolve_layout(800.0, 600.0);

    // Scroll to max (500 content - 100 visible = 400). A valid offset is
    // never clamped, so nothing is queued.
    doc.set_scroll_top(container, 400.0);
    doc.resolve_layout(800.0, 600.0);
    assert_eq!(doc.scroll_top(container), 400.0);
    assert!(
        doc.drain_scroll_clamps().is_empty(),
        "a valid offset must not queue a clamp event"
    );

    // Shrink the content twice before the runtime drains — the queue must
    // coalesce to a single entry carrying the final clamped value.
    doc.set_attribute(content, "style", "height: 250px");
    doc.resolve_layout(800.0, 600.0);
    doc.set_attribute(content, "style", "height: 180px");
    doc.resolve_layout(800.0, 600.0);

    // Offset clamped to the final max (180 - 100 = 80)...
    assert_eq!(doc.scroll_top(container), 80.0);
    // ...and the clamp was queued exactly once, not applied silently.
    assert_eq!(doc.drain_scroll_clamps(), vec![(container, 80.0)]);
    // Drained — a second drain yields nothing.
    assert!(doc.drain_scroll_clamps().is_empty());
}

// --- #236: the set_style inset fast path ------------------------------------
//
// `set_style("left" | "top" | "right" | "bottom", …)` on an absolutely
// positioned element skips the Stylo cascade and writes the Taffy inset
// directly. Whatever that shortcut does, the laid-out box must be
// indistinguishable from a full resolve of the same declaration — so every
// test here compares against a *twin* document that had the value in its style
// attribute from the start. Taffy's containing-block arithmetic and rounding
// are the oracle, never hand-computed. Each test also pins *which* path ran,
// so a regression that quietly routes everything through Stylo — or nothing —
// cannot hide behind a correct final position.
//
// Before the fix the fast path assigned `layout.x = left_px + margin` itself
// and skipped `layout_dirty`, so the number it wrote (padding-box-relative, no
// parent border, percentages/auto/viewport units as 0, `em`/`calc()`/`var()`
// as `auto`) persisted until an unrelated mutation triggered a real layout —
// at which point the element visibly jumped.

mod inset_fast_path {
    use super::*;
    use rinch_core::dom::NodeId;
    use rinch_dom::LayoutResult;
    use rinch_dom::computed_style::LengthPercentageAutoValue;

    /// A bordered containing block: the border is what separates the
    /// padding-box-relative inset from the border-box-relative `LayoutResult`.
    const PARENT: &str = "position: relative; width: 300px; height: 200px; \
                          border-left: 5px solid black; border-top: 7px solid black";
    const CHILD: &str = "position: absolute; left: 0; top: 0; width: 10px; height: 10px";

    /// body > parent > child, laid out once; returns the document and the child.
    fn positioned(parent_style: &str, child_style: &str) -> (RinchDocument, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let parent = doc.create_element("div");
        doc.set_attribute(parent, "style", parent_style);
        doc.append_child(body, parent);
        let child = doc.create_element("div");
        doc.set_attribute(child, "style", child_style);
        doc.append_child(parent, child);
        doc.resolve_layout(800.0, 600.0);
        (doc, child)
    }

    fn layout_of(doc: &RinchDocument, node: NodeId) -> LayoutResult {
        doc.tree.get(node.0).unwrap().layout
    }

    /// `style` with `overrides` appended (a later declaration wins).
    fn with_overrides(style: &str, overrides: &[(&str, &str)]) -> String {
        let mut style = style.to_string();
        for (property, value) in overrides {
            style.push_str(&format!("; {property}: {value}"));
        }
        style
    }

    /// The oracle: the same tree with `overrides` appended to the child's
    /// style attribute, fully resolved.
    fn twin(parent_style: &str, child_style: &str, overrides: &[(&str, &str)]) -> LayoutResult {
        let (doc, child) = positioned(parent_style, &with_overrides(child_style, overrides));
        layout_of(&doc, child)
    }

    /// Which path a `set_style`/`set_styles` call took, read from the tree's
    /// dirty state before the next resolve.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Path {
        /// The inset fast path: Taffy inset written, position deferred to
        /// layout, no restyle queued.
        Fast,
        /// The normal path: the node is queued for Stylo.
        Stylo,
    }

    fn assert_path(doc: &RinchDocument, node: NodeId, expected: Path) {
        match expected {
            Path::Fast => {
                assert!(
                    !doc.tree.styles_dirty && doc.tree.style_roots.is_empty(),
                    "the inset fast path must skip the Stylo restyle"
                );
                assert!(
                    doc.tree.layout_dirty,
                    "the inset fast path must defer the position to layout"
                );
            }
            Path::Stylo => assert!(
                doc.tree.styles_dirty && doc.tree.style_roots.contains(&node.0),
                "this value must reach Stylo"
            ),
        }
    }

    /// Apply `overrides` one `set_style` at a time, check the path taken,
    /// then run the normal resolve cycle.
    fn set_and_resolve(
        doc: &mut RinchDocument,
        node: NodeId,
        overrides: &[(&str, &str)],
        path: Path,
    ) {
        for (property, value) in overrides {
            doc.set_style(node, property, value);
        }
        assert_path(doc, node, path);
        doc.resolve_layout(800.0, 600.0);
    }

    /// Apply `overrides` as one `set_styles` batch, check the path, resolve.
    fn set_batch_and_resolve(
        doc: &mut RinchDocument,
        node: NodeId,
        overrides: &[(&str, &str)],
        path: Path,
    ) {
        doc.set_styles(node, overrides);
        assert_path(doc, node, path);
        doc.resolve_layout(800.0, 600.0);
    }

    /// (a) A pixel inset is padding-box-relative; the layout field is
    /// border-box-relative. The parent's border must survive the fast path.
    #[test]
    fn set_style_left_top_px_includes_parent_border() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let baseline = layout_of(&doc, child);
        assert_eq!(
            (baseline.x, baseline.y),
            (5.0, 7.0),
            "baseline: `left: 0; top: 0` sits inside the parent's border"
        );

        let overrides = [("left", "10px"), ("top", "20px")];
        set_and_resolve(&mut doc, child, &overrides, Path::Fast);

        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!(
            (expected.x, expected.y),
            (15.0, 27.0),
            "oracle sanity: full layout places the child border + inset"
        );
        assert_eq!(
            layout_of(&doc, child),
            expected,
            "set_style must land where a full resolve of the same value lands"
        );
    }

    /// (b) A percentage inset needs the containing block; it must not read as 0.
    #[test]
    fn set_style_left_percent_matches_full_layout() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "50%")];
        set_and_resolve(&mut doc, child, &overrides, Path::Fast);

        let expected = twin(PARENT, CHILD, &overrides);
        assert!(
            expected.x > 100.0,
            "oracle sanity: 50% of a 300px block is far from the left edge, got {}",
            expected.x
        );
        assert_eq!(layout_of(&doc, child), expected);
    }

    /// (c) `left: auto` with a `right` set anchors to the right edge; the
    /// fast path used to place it at `0 + margin`.
    #[test]
    fn set_style_left_auto_defers_to_right_anchor() {
        let child = "position: absolute; left: 0; right: 20px; top: 0; width: 10px; height: 10px";
        let (mut doc, node) = positioned(PARENT, child);
        let overrides = [("left", "auto")];
        set_and_resolve(&mut doc, node, &overrides, Path::Fast);

        let expected = twin(PARENT, child, &overrides);
        assert!(
            expected.x > 200.0,
            "oracle sanity: right-anchored child sits near the right edge, got {}",
            expected.x
        );
        assert_eq!(layout_of(&doc, node), expected);
    }

    /// `right`/`bottom` go through the fast path like `left`/`top`.
    #[test]
    fn set_style_right_bottom_take_the_fast_path() {
        let child = "position: absolute; right: 0; bottom: 0; width: 10px; height: 10px";
        let (mut doc, node) = positioned(PARENT, child);
        let overrides = [("right", "20px"), ("bottom", "30px")];
        set_and_resolve(&mut doc, node, &overrides, Path::Fast);

        let expected = twin(PARENT, child, &overrides);
        assert!(
            expected.x > 200.0 && expected.y > 100.0,
            "oracle sanity: anchored to the far edges, got ({}, {})",
            expected.x,
            expected.y
        );
        assert_eq!(layout_of(&doc, node), expected);
    }

    /// Any absolute unit is a plain length to Stylo's tokenizer, so it takes
    /// the fast path — converted by Stylo, not by a unit table of ours.
    #[test]
    fn set_style_absolute_units_take_the_fast_path() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "15pt")];
        set_and_resolve(&mut doc, child, &overrides, Path::Fast);

        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!(
            expected.x, 25.0,
            "oracle sanity: 15pt is 20px, plus the border"
        );
        assert_eq!(layout_of(&doc, child), expected);
    }

    /// (d) Values that need the cascade — font-relative, viewport-relative,
    /// `calc()`, `var()` — must reach Stylo rather than being written by a
    /// parser of our own (as `auto`, as `16 × rem`, as `vw` of some viewport).
    #[test]
    fn set_style_values_that_need_the_cascade_reach_stylo() {
        let parent = format!("{PARENT}; --x: 25px");
        let (mut doc, child) = positioned(&parent, CHILD);

        for value in ["2em", "2rem", "10vw", "calc(10px + 5px)", "var(--x)"] {
            let overrides = [("left", value)];
            set_and_resolve(&mut doc, child, &overrides, Path::Stylo);

            let computed = doc.tree.get(child.0).unwrap().computed_style.left;
            assert!(
                !matches!(computed, LengthPercentageAutoValue::Auto),
                "`left: {value}` was written as Auto — the author's value was lost"
            );
            let expected = twin(&parent, CHILD, &overrides);
            assert!(
                expected.x > 5.0,
                "oracle sanity: `left: {value}` must move the child, got {}",
                expected.x
            );
            assert_eq!(
                layout_of(&doc, child),
                expected,
                "`left: {value}` must lay out like a full resolve"
            );
        }
    }

    /// A unitless non-zero number is not a length in standards mode: Stylo
    /// drops the declaration. The fast path must not apply it as pixels — that
    /// would move the element now and snap it back at the next full restyle.
    #[test]
    fn set_style_unitless_number_is_dropped_like_stylo() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "120")];
        set_and_resolve(&mut doc, child, &overrides, Path::Stylo);

        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!(
            expected.x, 5.0,
            "oracle sanity: `left: 120` is invalid, so `left: auto`"
        );
        assert_eq!(layout_of(&doc, child), expected);

        // A viewport change re-cascades every node from its declaration block;
        // nothing may move.
        doc.resolve_layout(900.0, 600.0);
        assert_eq!(
            layout_of(&doc, child).x,
            5.0,
            "must not jump on the next full restyle"
        );
    }

    /// `position: fixed` bakes its Taffy size from its insets
    /// (`apply_stylo_styles_to_taffy`), so an inset change is not inset-only
    /// for it: the fast path would move the box and leave its children laid
    /// out against the old size. It must take the normal path.
    #[test]
    fn set_style_on_fixed_reaches_stylo() {
        const BAR: &str = "position: fixed; left: 0; right: 0; top: 0; height: 50px";
        const INNER: &str = "width: 100%; height: 10px";

        fn fixed_bar(bar_style: &str) -> (RinchDocument, NodeId, NodeId) {
            let mut doc = RinchDocument::new();
            let body = doc.body();
            let bar = doc.create_element("div");
            doc.set_attribute(bar, "style", bar_style);
            doc.append_child(body, bar);
            let inner = doc.create_element("div");
            doc.set_attribute(inner, "style", INNER);
            doc.append_child(bar, inner);
            doc.resolve_layout(800.0, 600.0);
            (doc, bar, inner)
        }

        let overrides = [("left", "200px")];
        let (mut doc, bar, inner) = fixed_bar(BAR);
        set_and_resolve(&mut doc, bar, &overrides, Path::Stylo);

        let (twin_doc, twin_bar, twin_inner) = fixed_bar(&with_overrides(BAR, &overrides));
        let expected_bar = layout_of(&twin_doc, twin_bar);
        let expected_inner = layout_of(&twin_doc, twin_inner);
        assert_eq!(
            (expected_bar.x, expected_bar.width, expected_inner.width),
            (200.0, 600.0, 600.0),
            "oracle sanity: the bar shrinks to the viewport minus its insets, and so does its child"
        );
        assert_eq!(layout_of(&doc, bar), expected_bar);
        assert_eq!(
            layout_of(&doc, inner),
            expected_inner,
            "the child must be laid out against the bar's new size"
        );
    }

    /// (e) rinch's fixed override in `read_layout_results` places the border
    /// box at `left`, ignoring the margin — a known deviation from CSS, which
    /// puts the *margin* edge there. The twin shares the override, so this
    /// pins consistency with it, not CSS: a `set_style` must land where a
    /// full resolve lands, whichever formula that is.
    #[test]
    fn set_style_left_on_fixed_ignores_margin() {
        let child = "position: fixed; margin-left: 4px; left: 0; top: 0; width: 10px; height: 10px";
        let (mut doc, node) = positioned(PARENT, child);
        let overrides = [("left", "10px")];
        set_and_resolve(&mut doc, node, &overrides, Path::Stylo);

        let expected = twin(PARENT, child, &overrides);
        assert_eq!(
            expected.x, 10.0,
            "oracle sanity: the fixed override is viewport-relative and drops the margin"
        );
        assert_eq!(layout_of(&doc, node), expected);
    }

    /// (g) The batch path defers to layout too.
    #[test]
    fn set_styles_inset_batch_matches_full_layout() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "10px"), ("top", "10px")];
        set_batch_and_resolve(&mut doc, child, &overrides, Path::Fast);

        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!((expected.x, expected.y), (15.0, 17.0));
        assert_eq!(layout_of(&doc, child), expected);
    }

    /// The batch path with a value that needs the cascade must decline too.
    #[test]
    fn set_styles_unparseable_length_reaches_stylo() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "2em"), ("top", "1em")];
        set_batch_and_resolve(&mut doc, child, &overrides, Path::Stylo);

        let computed = &doc.tree.get(child.0).unwrap().computed_style;
        assert!(
            !matches!(computed.left, LengthPercentageAutoValue::Auto)
                && !matches!(computed.top, LengthPercentageAutoValue::Auto),
            "em insets were written as Auto — the author's values were lost"
        );
        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!((expected.x, expected.y), (37.0, 23.0));
        assert_eq!(layout_of(&doc, child), expected);
    }

    /// An absolute with no positioned ancestor must decline for the same
    /// reason `fixed` does (#204): its Taffy *size* is baked from its insets
    /// against the initial containing block, so an inset change is not
    /// inset-only for it. Taking the fast path would leave the stale width.
    ///
    /// The insets are the `inset: 0` shorthand, which is what an app writes.
    /// This used to be spelled as four longhands to dodge #265:
    /// `merged_inline_style` joined a `HashMap`, so a `set_style` longhand
    /// landed on a random side of a shorthand already in the attribute and was
    /// silently lost about half the time. The order is deterministic now, and
    /// the appended longhand wins.
    #[test]
    fn set_style_left_on_an_icb_absolute_reaches_stylo() {
        const UNPOSITIONED: &str = "width: 300px; height: 200px";
        const FILLING: &str = "position: absolute; inset: 0";

        let (mut doc, node) = positioned(UNPOSITIONED, FILLING);
        assert_eq!(
            (layout_of(&doc, node).width, layout_of(&doc, node).height),
            (800.0, 600.0),
            "baseline: the viewport is the containing block"
        );

        let overrides = [("left", "100px")];
        set_and_resolve(&mut doc, node, &overrides, Path::Stylo);

        let expected = twin(UNPOSITIONED, FILLING, &overrides);
        assert_eq!(
            (expected.width, expected.x),
            (700.0, 100.0),
            "oracle sanity: the box re-sizes to the viewport minus the new inset"
        );
        assert_eq!(
            layout_of(&doc, node),
            expected,
            "the fast path would have kept the 800px width"
        );
    }

    /// A batch is all-or-nothing: one value that needs the cascade sends the
    /// whole batch to Stylo, so the two insets never come from different paths.
    #[test]
    fn set_styles_mixed_batch_declines_as_a_whole() {
        let (mut doc, child) = positioned(PARENT, CHILD);
        let overrides = [("left", "10px"), ("top", "2em")];
        set_batch_and_resolve(&mut doc, child, &overrides, Path::Stylo);

        let expected = twin(PARENT, CHILD, &overrides);
        assert_eq!((expected.x, expected.y), (15.0, 39.0));
        assert_eq!(layout_of(&doc, child), expected);
    }

    /// #265, end to end: a `set_style` longhand must beat a shorthand that was
    /// already in the attribute, because it is written after it. Half of all
    /// runs used to lay this out at `left: 0` — the same code, the same input,
    /// a different hash seed.
    ///
    /// `positioned` gives the child a positioned parent, so this is the fast
    /// path too: the fast path and the oracle must agree on which declaration
    /// won, which is the invariant the `merged → parse_inline_style → cache`
    /// pipeline exists to keep.
    #[test]
    fn set_style_longhand_beats_a_shorthand_already_in_the_attribute() {
        const FILLING: &str = "position: absolute; inset: 0";
        let (mut doc, child) = positioned(PARENT, FILLING);

        let overrides = [("left", "25px")];
        set_and_resolve(&mut doc, child, &overrides, Path::Fast);

        assert_eq!(
            doc.get_attribute(child, "style").unwrap(),
            "position: absolute; inset: 0; left: 25px",
            "the appended longhand must come last, every run"
        );

        let expected = twin(PARENT, FILLING, &overrides);
        assert_eq!(
            expected.x, 30.0,
            "oracle sanity: 25px inside the parent's 5px left border"
        );
        assert_eq!(
            layout_of(&doc, child),
            expected,
            "`left: 25px` was written after `inset: 0`, so it wins"
        );
    }
}

/// An inline-block nested inside an inline element still belongs to the block's
/// inline formatting context, and must be measured.
///
/// `mark_inline_descendants` used to mark an inline child with its `ifc_root`
/// and stop there. That was enough for text — `walk_inline_children` recurses
/// either way — but it left any inline-block *descendant* with
/// `ifc_root == None`, so `compute_inline_block_layouts` never measured it and
/// the Parley `InlineBox` pushed for it read a `layout` that was still zero.
///
/// The symptom in an app is an `<img>` inside an `<a>` vanishing: right `src`,
/// a computed width and height from its own style, and a 0x0 layout box, while
/// the identical image as a direct child of the block lays out correctly. Found
/// rendering a saved web page, where every site's logo is wrapped in an anchor.
#[test]
fn test_inline_block_inside_an_inline_element_is_measured() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.append_child(body, container);
    doc.set_inner_html(
        container,
        "<div id=\"direct\"><i style=\"display: inline-block; width: 90px; height: 30px\"></i></div>\
         <div id=\"nested\"><a><i style=\"display: inline-block; width: 90px; height: 30px\"></i></a></div>",
    );

    doc.resolve_layout(800.0, 600.0);

    let boxes: Vec<(f32, f32)> = rinch_dom::testing::query_selector(&doc.tree, "i")
        .into_iter()
        .map(|id| {
            let l = doc.tree.get(id).unwrap().layout;
            (l.width, l.height)
        })
        .collect();

    assert_eq!(boxes.len(), 2, "both inline-blocks should be in the tree");
    assert_eq!(
        boxes[0],
        (90.0, 30.0),
        "a direct inline-block child of the block has always worked"
    );
    assert_eq!(
        boxes[1],
        (90.0, 30.0),
        "and one wrapped in an <a> must be laid out the same way"
    );
}

/// The IFC flows an inline-block *box*, but not its interior.
///
/// `mark_inline_descendants` has to mark exactly the set
/// `walk_inline_children` flows into the line, and that walk stops at an
/// `inline-block` — it pushes one `InlineBox` for the box and never looks
/// inside. Marking the interior anyway hands the outer root's `ifc_root` to
/// boxes Taffy owns, and several passes read that field as "the IFC positions
/// this": `read_layout_results` then keeps the box's stale x/y instead of
/// Taffy's, `ifc_content_box_offset` adds the outer root's padding to its hit
/// rect, `resolve_percentage_inline_blocks` resolves its percentage sizes
/// against the wrong containing block, and `copy_cached_text_layouts` drops the
/// cached Parley layout for the text inside every `<button>`.
#[test]
fn test_inline_block_inside_an_inline_block_is_not_joined_to_the_outer_ifc() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.append_child(body, container);
    doc.set_inner_html(
        container,
        "<div style=\"padding: 20px\">\
           <span id=\"host\" style=\"display: inline-block; padding: 7px\">\
             <i id=\"inner\" style=\"display: inline-block; width: 40px; height: 12px\"></i>\
           </span>\
         </div>",
    );

    doc.resolve_layout(800.0, 600.0);

    let host = rinch_dom::testing::query_selector(&doc.tree, "[id=host]")[0];
    let inner = rinch_dom::testing::query_selector(&doc.tree, "[id=inner]")[0];

    // The block's IFC owns the inline-block box itself…
    assert!(
        doc.tree.get(host).unwrap().ifc_root.is_some(),
        "the inline-block is inline content of the block"
    );
    // …and stops there.
    assert_eq!(
        doc.tree.get(inner).unwrap().ifc_root,
        None,
        "the box inside an inline-block belongs to Taffy, not the outer IFC"
    );

    let l = doc.tree.get(inner).unwrap().layout;
    assert_eq!((l.width, l.height), (40.0, 12.0));
    assert_eq!(
        (l.x, l.y),
        (7.0, 7.0),
        "Taffy places it inside its parent's padding"
    );
}

/// The inline-block wrapped in an inline element is *placed* by the IFC too,
/// not just measured: `write_inline_positions` writes the Parley inline box's
/// origin onto it, so text before it pushes it along the line.
#[test]
fn test_inline_block_inside_an_inline_element_is_positioned_by_the_ifc() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.append_child(body, container);
    doc.set_inner_html(
        container,
        "<div><a>some text before it<i id=\"boxed\" \
         style=\"display: inline-block; width: 20px; height: 10px\"></i></a></div>",
    );

    doc.resolve_layout(800.0, 600.0);

    let boxed = rinch_dom::testing::query_selector(&doc.tree, "[id=boxed]")[0];
    let l = doc.tree.get(boxed).unwrap().layout;
    assert_eq!((l.width, l.height), (20.0, 10.0));
    assert!(
        l.x > 0.0,
        "the text before it advances the line, so the box is not at the line origin (got x = {})",
        l.x
    );
}

// Issue #204: the containing block of an absolutely positioned box is its
// nearest *positioned* ancestor, and the initial containing block — the
// viewport at the origin — when it has none. Taffy only ever uses the direct
// parent, so `inset: 0` inside an unpositioned 300x200 div used to give a
// 300x200 box where a browser (and `rinch-web`, which is the browser) gives the
// whole 800x600 viewport.
//
// The correction keeps `LayoutResult` parent-relative and writes the delta, so
// each test below pins **both** numbers: `layout` (what Taffy's tree says) and
// `compute_absolute_position` (what paint and hit testing put on screen).

mod absolute_containing_block {
    use super::*;
    use rinch_core::dom::NodeId;

    /// body > container > abs, laid out once at 800x600.
    fn nested(container_style: &str, abs_style: &str) -> (RinchDocument, NodeId, NodeId) {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let container = doc.create_element("div");
        doc.set_attribute(container, "style", container_style);
        doc.append_child(body, container);
        let abs = doc.create_element("div");
        doc.set_attribute(abs, "style", abs_style);
        doc.append_child(container, abs);
        doc.resolve_layout(800.0, 600.0);
        (doc, container, abs)
    }

    fn on_screen(doc: &RinchDocument, node: NodeId) -> (f64, f64) {
        rinch_dom::paint::compute_absolute_position(&doc.tree, node.0, 1.0)
    }

    /// The issue as filed: `inset: 0` under an unpositioned parent fills the
    /// viewport, not the parent.
    #[test]
    fn inset_zero_under_unpositioned_parent_fills_the_viewport() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px",
            "position: absolute; inset: 0",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (800.0, 600.0),
            "the initial containing block is the viewport, not the 300x200 parent"
        );
        assert_eq!(on_screen(&doc, abs), (0.0, 0.0));
    }

    /// The same box under a parent that is *offset* from the page origin. This
    /// is what tells an ICB-relative box from a parent-relative one: the two
    /// coincide only when the parent sits at (0, 0).
    #[test]
    fn an_offset_parent_does_not_move_an_icb_absolute() {
        let (doc, container, abs) = nested(
            "width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; inset: 0",
        );

        assert_eq!(
            on_screen(&doc, container),
            (40.0, 50.0),
            "sanity: the parent really is off the origin"
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!((l.width, l.height), (800.0, 600.0));
        assert_eq!(
            (l.x, l.y),
            (-40.0, -50.0),
            "`LayoutResult` stays parent-relative — the correction is a delta, \
             so every coordinate walk in the codebase keeps working"
        );
        assert_eq!(
            on_screen(&doc, abs),
            (0.0, 0.0),
            "on screen it is pinned to the viewport origin"
        );
    }

    /// The regression guard: a `position: relative` direct parent *is* the
    /// containing block, so Taffy's answer stands and nothing is corrected.
    #[test]
    fn a_positioned_parent_is_still_the_containing_block() {
        let (doc, _container, abs) = nested(
            "position: relative; width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; inset: 0",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!((l.width, l.height), (300.0, 200.0));
        assert_eq!((l.x, l.y), (0.0, 0.0));
        assert_eq!(on_screen(&doc, abs), (40.0, 50.0));
    }

    /// A transformed ancestor is the containing block for its absolutely
    /// positioned descendants even when its `position` is `static`.
    #[test]
    fn a_transformed_parent_is_the_containing_block() {
        let (doc, _container, abs) = nested(
            "transform: translateX(10px); width: 300px; height: 200px",
            "position: absolute; inset: 0",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (300.0, 200.0),
            "the transform stops the walk, so the parent is the containing block"
        );
    }

    /// With both insets on an axis `auto`, CSS keeps the box at its *static*
    /// position — which does come from the flow position in the DOM parent. So
    /// that axis is left exactly as Taffy laid it out.
    #[test]
    fn auto_insets_keep_the_static_position() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; width: 50px; height: 50px",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!((l.width, l.height), (50.0, 50.0), "auto insets do not size");
        assert_eq!((l.x, l.y), (0.0, 0.0), "static position, parent-relative");
        assert_eq!(
            on_screen(&doc, abs),
            (40.0, 50.0),
            "so it sits where it would have sat in flow, inside the parent"
        );
    }

    /// One axis inset, the other `auto`: only the inset axis is corrected.
    #[test]
    fn one_axis_is_corrected_without_disturbing_the_other() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; left: 12px; width: 50px; height: 50px",
        );

        assert_eq!(
            on_screen(&doc, abs),
            (12.0, 50.0),
            "x measured from the viewport, y still the static position"
        );
    }

    /// A percentage size on an ICB-absolute resolves against the viewport.
    #[test]
    fn a_percentage_size_resolves_against_the_viewport() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px",
            "position: absolute; left: 0; top: 0; width: 50%; height: 50%",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (400.0, 300.0),
            "half the viewport, not half the 300x200 parent"
        );
    }

    /// The size is baked into the *Taffy* style before layout, not patched
    /// afterwards — so the box's own children lay out inside the right box.
    /// A post-layout-only correction would give this child 200px.
    #[test]
    fn a_percentage_height_child_sees_the_corrected_box() {
        let (mut doc, _container, abs) = nested(
            "width: 300px; height: 200px",
            "position: absolute; inset: 0",
        );
        let kid = doc.create_element("div");
        doc.set_attribute(kid, "style", "width: 100%; height: 100%");
        doc.append_child(abs, kid);
        doc.resolve_layout(800.0, 600.0);

        let l = doc.tree.get(kid.0).unwrap().layout;
        assert_eq!((l.width, l.height), (800.0, 600.0));
    }

    /// `right`/`bottom` anchoring measures from the viewport's far edges.
    #[test]
    fn right_and_bottom_anchor_to_the_viewport_edges() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; right: 10px; bottom: 20px; width: 50px; height: 30px",
        );

        assert_eq!(on_screen(&doc, abs), (740.0, 550.0));
    }

    /// A margin on the box offsets it from the inset, as it does in flow.
    #[test]
    fn a_margin_offsets_the_box_from_its_inset() {
        let (doc, _container, abs) = nested(
            "width: 300px; height: 200px; margin: 50px 40px",
            "position: absolute; left: 10px; top: 20px; margin-left: 5px; \
             margin-top: 7px; width: 50px; height: 30px",
        );

        assert_eq!(on_screen(&doc, abs), (15.0, 27.0));
    }

    /// An intervening *positioned* ancestor that is not the direct parent is
    /// still resolved against the direct parent — the half of #204 this change
    /// deliberately leaves alone (#386), pinned so the follow-up has a starting
    /// point.
    #[test]
    fn a_positioned_grandparent_is_not_yet_honoured() {
        let mut doc = RinchDocument::new();
        let body = doc.body();
        let outer = doc.create_element("div");
        doc.set_attribute(
            outer,
            "style",
            "position: relative; width: 400px; height: 300px",
        );
        doc.append_child(body, outer);
        let middle = doc.create_element("div");
        doc.set_attribute(middle, "style", "width: 300px; height: 200px");
        doc.append_child(outer, middle);
        let abs = doc.create_element("div");
        doc.set_attribute(abs, "style", "position: absolute; inset: 0");
        doc.append_child(middle, abs);
        doc.resolve_layout(800.0, 600.0);

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!(
            (l.width, l.height),
            (300.0, 200.0),
            "known gap: this should be 400x300 (the positioned grandparent)"
        );
    }

    /// An ICB-absolute in a `display: none` subtree is left alone — its Taffy
    /// box is 0x0 and correcting it would drag a hidden node onto the viewport.
    #[test]
    fn a_hidden_subtree_is_left_alone() {
        let (doc, _container, abs) = nested(
            "display: none; width: 300px; height: 200px",
            "position: absolute; inset: 0",
        );

        let l = doc.tree.get(abs.0).unwrap().layout;
        assert_eq!((l.width, l.height), (0.0, 0.0));
    }
}

/// Removing a `display: contents` wrapper must free the Taffy space its
/// *children* were spliced into, not the wrapper's own (already-detached)
/// Taffy id (card K48).
///
/// `sync_display_contents` polyfills `display: contents` by hiding the
/// wrapper itself (`display: none` in Taffy) and splicing its children's
/// Taffy ids directly into the parent's Taffy child list
/// (`collect_effective_taffy_children`). So the wrapper's own `taffy_id` is
/// never actually present among the parent's Taffy children — `remove_node`
/// looking for *that* id to remove is therefore a silent no-op
/// (`taffy_remove_child_safe` exists precisely to swallow exactly this
/// "not actually a child" case without panicking), and the child that really
/// did occupy the slot is orphaned in the Taffy tree forever: a permanent,
/// invisible sibling that keeps claiming its share of `flex-grow`.
///
/// On the moto g this surfaced as: `Route::Library` is the one screen whose
/// content is a reactive `if` (first-run vs. the real library) rather than a
/// single element, so RSX wraps it in a `display: contents` marker div to
/// give that `if` somewhere to insert into. The instant the app navigated
/// away from Library for the first time, that wrapper's removal orphaned
/// Library's real root as a phantom `flex: 1` sibling of the app's own root
/// column — forever after, every screen's flex column split the remaining
/// height with a ghost nobody could see, taking only half its rightful
/// space. Two `flex: 1` divs under a `flex: 1` column, one of them behind a
/// `display: contents` wrapper, is the whole reproduction: no Android, no
/// scroller, no route table required.
#[test]
fn test_removing_display_contents_wrapper_frees_its_childs_taffy_slot() {
    let mut doc = RinchDocument::new();
    let body = doc.body();

    let column = doc.create_element("div");
    doc.set_attribute(
        column,
        "style",
        "display: flex; flex-direction: column; height: 200px;",
    );
    doc.append_child(body, column);

    // The wrapper stands in for a route's `if`/`match` arm: a single
    // `display: contents` div whose one child is the arm's real content.
    let wrapper = doc.create_element("div");
    doc.set_attribute(wrapper, "style", "display: contents");
    doc.append_child(column, wrapper);

    let a = doc.create_element("div");
    doc.set_attribute(a, "style", "flex: 1;");
    doc.append_child(wrapper, a);

    // The survivor: a second `flex: 1` child of the column, mounted directly
    // (no wrapper) — every other route in the app looks like this.
    let b = doc.create_element("div");
    doc.set_attribute(b, "style", "flex: 1;");
    doc.append_child(column, b);

    // First layout: two flex:1 children split the column's 200px evenly,
    // exactly like Library's own root sharing the app root with nothing else.
    doc.resolve_layout(800.0, 600.0);
    let la = doc.tree.get(a.0).unwrap().layout;
    let lb = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(la.height, 100.0, "sanity: even split before removal");
    assert_eq!(lb.height, 100.0, "sanity: even split before removal");

    // Navigate away: remove the wrapper, exactly as `match_dom`/`show_dom`
    // remove an outgoing route's content via `NodeHandle::remove()`.
    doc.remove_node(wrapper);
    doc.resolve_layout(800.0, 600.0);

    let lb_after = doc.tree.get(b.0).unwrap().layout;
    assert_eq!(
        lb_after.height, 200.0,
        "the survivor must claim the whole column once its sibling is gone — \
         a height of 100.0 here means `a` is still a phantom Taffy child \
         nobody could remove"
    );
}
