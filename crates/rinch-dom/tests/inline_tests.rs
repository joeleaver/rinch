//! Tests for CSS inline layout (IFC) in rinch-dom.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

// ============================================================
// IFC Detection
// ============================================================

#[test]
fn test_ifc_detected_for_multiple_text_nodes() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let t2 = doc.create_text("world!");
    doc.append_child(div, t1);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    // The div should be an IFC root with a text_layout
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(
        div_node.text_layout.is_some(),
        "div with multiple text children should be an IFC root"
    );
}

#[test]
fn test_ifc_detected_for_inline_element() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);

    let div_node = doc.tree.get(div.0).unwrap();
    assert!(
        div_node.text_layout.is_some(),
        "div with text + span should be an IFC root"
    );
}

#[test]
fn test_no_ifc_for_single_text_child() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let text = doc.create_text("Just one text node");
    doc.append_child(div, text);

    doc.resolve_layout(800.0, 600.0);

    // Single text child doesn't need IFC — uses Taffy text measurement
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(
        div_node.text_layout.is_none(),
        "div with single text child should NOT be an IFC root"
    );
}

#[test]
fn test_no_ifc_for_block_only_children() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "style", "width: 400px");
    doc.append_child(body, container);

    let child1 = doc.create_element("div");
    let child2 = doc.create_element("div");
    doc.set_attribute(child1, "style", "height: 20px");
    doc.set_attribute(child2, "style", "height: 20px");
    doc.append_child(container, child1);
    doc.append_child(container, child2);

    doc.resolve_layout(800.0, 600.0);

    let container_node = doc.tree.get(container.0).unwrap();
    assert!(
        container_node.text_layout.is_none(),
        "div with only block children should NOT be an IFC root"
    );
}

// ============================================================
// Inline Layout Measurement
// ============================================================

#[test]
fn test_ifc_root_has_nonzero_height() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);

    let div_layout = doc.tree.get(div.0).unwrap().layout;
    assert!(
        div_layout.height > 0.0,
        "IFC root height should be > 0, got {}",
        div_layout.height
    );
    assert!(
        div_layout.width > 0.0,
        "IFC root width should be > 0, got {}",
        div_layout.width
    );
}

#[test]
fn test_ifc_text_wraps_in_narrow_container() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 100px; font-size: 14px");
    doc.append_child(body, div);

    let t1 = doc.create_text("This is ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("a long sentence that should wrap across multiple lines.");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);

    let div_layout = doc.tree.get(div.0).unwrap().layout;
    // Should be multiple lines tall
    assert!(
        div_layout.height > 30.0,
        "wrapped IFC content should be taller than one line, got {}",
        div_layout.height
    );
}

#[test]
fn test_ifc_multiple_text_nodes_flow_inline() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 800px; font-size: 16px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let t2 = doc.create_text("world!");
    doc.append_child(div, t1);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    let div_layout = doc.tree.get(div.0).unwrap().layout;
    // Two short text fragments on a wide container = single line
    // Height should be approximately one line height (not two)
    assert!(
        div_layout.height < 40.0,
        "two short texts in wide container should be ~1 line, got height {}",
        div_layout.height
    );
}

// ============================================================
// Inline Element Styles
// ============================================================

#[test]
fn test_ifc_with_bold_span() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Normal ");
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "font-weight: bold");
    let t2 = doc.create_text("bold");
    let t3 = doc.create_text(" text");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);
    doc.append_child(div, t3);

    doc.resolve_layout(800.0, 600.0);

    // Should produce a valid IFC layout
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some());
    assert!(div_node.layout.height > 0.0);
}

#[test]
fn test_ifc_with_colored_span() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Black ");
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "color: red");
    let t2 = doc.create_text("red");
    let t3 = doc.create_text(" text");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);
    doc.append_child(div, t3);

    doc.resolve_layout(800.0, 600.0);

    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some());
}

#[test]
fn test_ifc_with_nested_inline_elements() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    // "Hello <em><strong>bold italic</strong></em> world"
    let t1 = doc.create_text("Hello ");
    let em = doc.create_element("em");
    let strong = doc.create_element("strong");
    doc.set_attribute(em, "style", "font-style: italic");
    doc.set_attribute(strong, "style", "font-weight: bold");
    let t2 = doc.create_text("bold italic");
    let t3 = doc.create_text(" world");

    doc.append_child(div, t1);
    doc.append_child(div, em);
    doc.append_child(em, strong);
    doc.append_child(strong, t2);
    doc.append_child(div, t3);

    doc.resolve_layout(800.0, 600.0);

    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some());
    assert!(div_node.layout.height > 0.0);
}

// ============================================================
// Reactive Invalidation
// ============================================================

#[test]
fn test_ifc_invalidated_on_text_change() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);
    let h1 = doc.tree.get(div.0).unwrap().layout.height;

    // Change text content — should invalidate and rebuild IFC
    doc.set_text_content(t2, "a much longer replacement text that is wider");
    doc.resolve_layout(800.0, 600.0);

    let h2 = doc.tree.get(div.0).unwrap().layout.height;
    // The layout should have been rebuilt (may or may not change height,
    // but it should not panic and IFC should still be valid)
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some(), "IFC should be rebuilt after text change");
    assert!(h2 > 0.0);
}

#[test]
fn test_ifc_invalidated_on_child_removal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    let t3 = doc.create_text("!");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);
    doc.append_child(div, t3);

    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.get(div.0).unwrap().text_layout.is_some());

    // Remove the span — IFC should be invalidated and rebuilt
    doc.remove_child(div, span);
    doc.resolve_layout(800.0, 600.0);

    // Should not panic, layout should still work
    let div_layout = doc.tree.get(div.0).unwrap().layout;
    assert!(div_layout.height > 0.0);
}

#[test]
fn test_ifc_invalidated_on_child_addition() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);

    // Add another text node — IFC should be invalidated and rebuilt
    let t3 = doc.create_text(" and more text!");
    doc.append_child(div, t3);
    doc.resolve_layout(800.0, 600.0);

    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some(), "IFC should be rebuilt after child addition");
    assert!(div_node.layout.height > 0.0);
}

#[test]
fn test_ifc_invalidated_on_style_change() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 14px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);
    let h1 = doc.tree.get(div.0).unwrap().layout.height;

    // Change span style — IFC should rebuild
    doc.set_attribute(span, "style", "font-size: 32px");
    doc.resolve_layout(800.0, 600.0);
    let h2 = doc.tree.get(div.0).unwrap().layout.height;

    // Larger font in span should increase the line height
    assert!(
        h2 > h1,
        "larger span font should increase IFC height: {} vs {}",
        h2,
        h1
    );
}

#[test]
fn test_no_stale_refs_after_rapid_mutations() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 300px");
    doc.append_child(body, div);

    // Build initial inline content
    let t1 = doc.create_text("Start ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("middle");
    let t3 = doc.create_text(" end");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);
    doc.append_child(div, t3);
    doc.resolve_layout(800.0, 600.0);

    // Rapid mutations: remove, add, change text, re-layout
    for i in 0..5 {
        doc.remove_child(div, span);
        doc.resolve_layout(800.0, 600.0);

        doc.append_child(div, span);
        doc.set_text_content(t2, &format!("iteration {}", i));
        doc.resolve_layout(800.0, 600.0);
    }

    // Should not have panicked. Final layout should be valid.
    let div_layout = doc.tree.get(div.0).unwrap().layout;
    assert!(div_layout.height > 0.0, "layout should be valid after rapid mutations");
}

#[test]
fn test_no_stale_refs_after_replace_node() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);
    doc.resolve_layout(800.0, 600.0);

    // Replace the span with a new span
    let new_span = doc.create_element("span");
    doc.set_attribute(new_span, "style", "color: blue");
    let t3 = doc.create_text("universe");
    doc.append_child(new_span, t3);
    doc.replace_node(span, new_span);
    doc.resolve_layout(800.0, 600.0);

    // Should not panic
    let div_layout = doc.tree.get(div.0).unwrap().layout;
    assert!(div_layout.height > 0.0);
}

// ============================================================
// IFC root field cleanup
// ============================================================

#[test]
fn test_ifc_root_cleared_on_child_removal() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);

    // Span should have ifc_root set
    assert_eq!(
        doc.tree.get(span.0).unwrap().ifc_root,
        Some(div.0),
        "span should belong to div's IFC"
    );

    // Remove span — ifc_root should be cleared
    doc.remove_child(div, span);
    assert_eq!(
        doc.tree.get(span.0).unwrap().ifc_root,
        None,
        "removed span should have ifc_root cleared"
    );
}

#[test]
fn test_ifc_text_layout_cleared_on_mutation() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.get(div.0).unwrap().text_layout.is_some());

    // Mutate inline content — text_layout should be cleared before rebuild
    doc.set_text_content(t2, "changed");
    // After mutation but before layout, text_layout should be cleared
    assert!(
        doc.tree.get(div.0).unwrap().text_layout.is_none(),
        "text_layout should be cleared after mutation"
    );

    // Rebuild
    doc.resolve_layout(800.0, 600.0);
    assert!(doc.tree.get(div.0).unwrap().text_layout.is_some());
}

// ============================================================
// Edge Cases
// ============================================================

#[test]
fn test_ifc_with_empty_span() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    // Empty span — no text children
    let t2 = doc.create_text(" world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    // Should not panic, layout should work
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some());
    assert!(div_node.layout.height > 0.0);
}

#[test]
fn test_ifc_with_only_inline_elements_no_direct_text() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let span1 = doc.create_element("span");
    let t1 = doc.create_text("First ");
    doc.append_child(span1, t1);

    let span2 = doc.create_element("span");
    let t2 = doc.create_text("second");
    doc.append_child(span2, t2);

    doc.append_child(div, span1);
    doc.append_child(div, span2);

    doc.resolve_layout(800.0, 600.0);

    let div_node = doc.tree.get(div.0).unwrap();
    assert!(div_node.text_layout.is_some());
    assert!(div_node.layout.height > 0.0);
}

#[test]
fn test_multiple_resolve_layout_calls_stable() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Hello ");
    let span = doc.create_element("span");
    doc.set_attribute(span, "style", "font-weight: bold");
    let t2 = doc.create_text("world");
    doc.append_child(div, t1);
    doc.append_child(div, span);
    doc.append_child(span, t2);

    // Call resolve_layout multiple times — results should be stable
    doc.resolve_layout(800.0, 600.0);
    let h1 = doc.tree.get(div.0).unwrap().layout.height;

    doc.resolve_layout(800.0, 600.0);
    let h2 = doc.tree.get(div.0).unwrap().layout.height;

    doc.resolve_layout(800.0, 600.0);
    let h3 = doc.tree.get(div.0).unwrap().layout.height;

    assert_eq!(h1, h2, "height should be stable across layout passes");
    assert_eq!(h2, h3, "height should be stable across layout passes");
}

// ============================================================
// Inline-Block Elements
// ============================================================

#[test]
fn test_inline_block_in_text_flow() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Click ");
    let button = doc.create_element("button");
    doc.set_attribute(button, "style", "width: 60px; height: 24px");
    let btn_text = doc.create_text("OK");
    doc.append_child(button, btn_text);
    let t2 = doc.create_text(" to continue.");
    doc.append_child(div, t1);
    doc.append_child(div, button);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    // The div should be an IFC root (has inline-block child mixed with text)
    let div_node = doc.tree.get(div.0).unwrap();
    assert!(
        div_node.text_layout.is_some(),
        "div with text + inline-block button should be an IFC root"
    );

    // Button should have a valid position from Parley
    let btn_layout = doc.tree.get(button.0).unwrap().layout;
    assert!(
        btn_layout.x > 0.0,
        "inline-block button should have x > 0 (after 'Click '), got {}",
        btn_layout.x
    );
    assert!(btn_layout.y.is_finite(), "button y should be finite, got {}", btn_layout.y);
}

#[test]
fn test_inline_block_preserves_dimensions() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 16px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Before ");
    let inline_div = doc.create_element("div");
    doc.set_attribute(inline_div, "style", "display: inline-block; width: 80px; height: 30px; background-color: red");
    let t2 = doc.create_text(" after");
    doc.append_child(div, t1);
    doc.append_child(div, inline_div);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    // The inline-block div should preserve its explicit dimensions
    let ib_layout = doc.tree.get(inline_div.0).unwrap().layout;
    assert!(
        (ib_layout.width - 80.0).abs() < 1.0,
        "inline-block should preserve width=80, got {}",
        ib_layout.width
    );
    assert!(
        (ib_layout.height - 30.0).abs() < 1.0,
        "inline-block should preserve height=30, got {}",
        ib_layout.height
    );
}

#[test]
fn test_inline_block_ifc_root_height_includes_block() {
    let mut doc = RinchDocument::new();
    let body = doc.body();
    let div = doc.create_element("div");
    doc.set_attribute(div, "style", "width: 400px; font-size: 14px");
    doc.append_child(body, div);

    let t1 = doc.create_text("Text ");
    let big_box = doc.create_element("div");
    doc.set_attribute(big_box, "style", "display: inline-block; width: 50px; height: 50px");
    let t2 = doc.create_text(" more text");
    doc.append_child(div, t1);
    doc.append_child(div, big_box);
    doc.append_child(div, t2);

    doc.resolve_layout(800.0, 600.0);

    // IFC root height should accommodate the 50px tall inline-block
    let div_layout = doc.tree.get(div.0).unwrap().layout;
    assert!(
        div_layout.height >= 50.0,
        "IFC root should be at least as tall as the 50px inline-block, got {}",
        div_layout.height
    );
}
