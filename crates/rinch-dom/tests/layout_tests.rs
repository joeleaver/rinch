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
