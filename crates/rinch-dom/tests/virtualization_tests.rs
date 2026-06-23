//! Isolation tests for the `estimated_height` block-collapse mechanism that
//! block virtualization depends on.
//!
//! These reproduce the **new editor's** block structure — an element with a
//! single text child, built directly through the `DomDocument` trait the way
//! `RinchDomEditorView::ViewDesc::build` does (`<p data-pm-type="paragraph">` +
//! a text child) — and verify rinch-dom honors `estimated_height` exactly the
//! way `CeVirtualWindow` relies on. If a far off-screen editor paragraph keeps
//! its full multi-line height after `estimated_height` is set, the collapse is
//! broken at the layout layer (not in the driver). See the
//! `project-editor-virtualization` handover.

use rinch_core::dom::DomDocument;
use rinch_dom::RinchDocument;

/// Build a scroll container holding `n` paragraphs, each with text long enough
/// to wrap onto several lines in a narrow (150px) column. Mirrors the editor's
/// `<div data-pm-type="doc">` container with `<p data-pm-type="paragraph">`
/// children. Returns `(container_id, block_ids)` (raw node ids).
fn build_editor_like_doc(doc: &mut RinchDocument, n: usize) -> (usize, Vec<usize>) {
    let body = doc.body();
    let container = doc.create_element("div");
    doc.set_attribute(container, "data-pm-type", "doc");
    doc.set_attribute(
        container,
        "style",
        "width: 150px; height: 300px; overflow-y: auto; position: relative",
    );
    doc.append_child(body, container);

    let mut blocks = Vec::new();
    for i in 0..n {
        let p = doc.create_element("p");
        doc.set_attribute(p, "data-pm-type", "paragraph");
        doc.append_child(container, p);
        let text = doc.create_text(&format!(
            "Block {i} with plenty of words here so it wraps across at least two lines in a narrow column"
        ));
        doc.append_child(p, text);
        blocks.push(p.0);
    }
    (container.0, blocks)
}

/// The editor `<p>` must be an IFC root — its text child carries
/// `ifc_root == Some(<p>)`, which is what makes the `<p>` a
/// `NodeContext::InlineRoot` Taffy leaf and lets the `estimated_height`
/// short-circuit fire.
#[test]
fn editor_paragraph_text_child_points_at_its_ifc_root() {
    let mut doc = RinchDocument::new();
    let (_c, blocks) = build_editor_like_doc(&mut doc, 3);
    doc.resolve_layout(400.0, 600.0);

    for &p in &blocks {
        let text_child = doc.tree.nodes[p].children[0];
        assert_eq!(
            doc.tree.nodes[text_child].ifc_root,
            Some(p),
            "the <p>'s text child must point at the <p> as its IFC root"
        );
    }
}

/// Setting `estimated_height` (with the same dirty markers `CeVirtualWindow`
/// uses) on a far off-screen editor paragraph must collapse it to the estimate
/// on the next layout pass.
#[test]
fn estimated_height_collapses_an_editor_paragraph() {
    let mut doc = RinchDocument::new();
    let (_c, blocks) = build_editor_like_doc(&mut doc, 80);
    doc.resolve_layout(400.0, 600.0);

    let probe = blocks[50];
    let full_h = doc.tree.nodes[probe].layout.height;
    assert!(
        full_h > 30.0,
        "expected a multi-line paragraph (>30px) before collapse, got {full_h}"
    );

    // Mimic CeVirtualWindow::new exactly: set the estimate + dirty markers.
    for &b in &blocks[40..] {
        doc.tree.nodes[b].estimated_height = Some(24.0);
        doc.tree.style_dirty_nodes.push(b);
    }
    doc.tree.layout_dirty = true;
    doc.tree.ifc_dirty = true;
    doc.tree.styles_dirty = true;

    doc.resolve_layout(400.0, 600.0);

    let collapsed_h = doc.tree.nodes[probe].layout.height;
    assert!(
        (collapsed_h - 24.0).abs() < 1.0,
        "block 50 should collapse to ~24px after estimated_height was set, \
         got {collapsed_h} (full height was {full_h})"
    );
}
