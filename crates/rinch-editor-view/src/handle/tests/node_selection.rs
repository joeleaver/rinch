//! Node selections, embeds and typing over a selected node.

use super::*;

fn hr(s: &Schema) -> Node {
    s.branch("horizontal_rule", Fragment::empty()).unwrap()
}

fn img(s: &Schema) -> Node {
    s.create_node(
        "image",
        rinch_editor_core::Attrs::from_iter([("src", rinch_editor_core::AttrValue::from("a.png"))]),
        Fragment::empty(),
    )
    .unwrap()
}

#[test]
fn node_selection_at_host_resolves_a_leaf_and_rejects_a_textblock() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
    let blocks = children(&h, h.container_id);
    assert_eq!(tag(&h, blocks[1]).as_deref(), Some("hr"));

    // The hr's host id → a NodeSelection of the hr (model pos 4..5).
    let sel = h
        .handle
        .node_selection_at_host(blocks[1].0)
        .expect("hr host resolves to a node selection");
    assert_eq!(sel.from(), Pos(4));
    assert_eq!(sel.to(), Pos(5));

    // The paragraph host is a node but not a selectable leaf → None.
    assert!(
        h.handle.node_selection_at_host(blocks[0].0).is_none(),
        "a textblock is not node-selectable"
    );
}

#[test]
fn backspace_deletes_a_node_selection() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
    let sel = h
        .handle
        .node_selection_at_host(children(&h, h.container_id)[1].0)
        .unwrap();
    h.handle.set_selection(sel);

    // Backspace over a (never-empty) node selection deletes the node.
    assert!(h.handle.command("deleteCharBackward"));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 1, "the hr was removed");
    assert_eq!(tag(&h, blocks[0]).as_deref(), Some("p"));
}

// ── The starter kit's `embed` (a block leaf atom an app draws) ───────────

fn embed(s: &Schema, attrs: &[(&str, &str)]) -> Node {
    s.create_node(
        "embed",
        rinch_editor_core::Attrs::from_iter(
            attrs
                .iter()
                .map(|(k, v)| (*k, rinch_editor_core::AttrValue::from(*v))),
        ),
        Fragment::empty(),
    )
    .unwrap()
}

/// An embed's host is a `<div data-pm-type="embed">` carrying the app's data
/// attributes, and it follows an attr change; it is node-selected from its
/// host, an arrow moves onto it, and Backspace on it deletes it — as for an hr.
#[test]
fn an_embed_renders_as_a_div_host_and_is_selected_and_deleted_as_an_hr_is() {
    let s = schema();
    let h = mount(doc_node(
        &s,
        vec![
            para(&s, "ab"),
            embed(&s, &[("data-sheet", "s1")]),
            para(&s, "cd"),
        ],
    ));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 3);
    assert_eq!(tag(&h, blocks[1]).as_deref(), Some("div"));
    let attr = |id: NodeId, name: &str| h.doc.borrow().get_attribute(id, name);
    assert_eq!(attr(blocks[1], "data-pm-type").as_deref(), Some("embed"));
    assert_eq!(attr(blocks[1], "data-sheet").as_deref(), Some("s1"));
    assert!(children(&h, blocks[1]).is_empty(), "an empty host");

    // An attr change reaches the host.
    assert!(h.handle.update(|state| {
        let mut tr = state.tr();
        tr.set_node_attr(4, "data-sheet", rinch_editor_core::AttrValue::from("s2"))
            .ok()?;
        Some(tr)
    }));
    let host = children(&h, h.container_id)[1];
    assert_eq!(attr(host, "data-sheet").as_deref(), Some("s2"));

    // Its host node-selects it (model 4..5).
    let sel = h
        .handle
        .node_selection_at_host(host.0)
        .expect("the embed's host resolves to a node selection");
    assert_eq!((sel.from(), sel.to()), (Pos(4), Pos(5)));

    // An arrow from the end of "ab" node-selects it too.
    let moved = rinch_editor_core::resolve_cursor_motion(
        &h.handle.doc(),
        Pos(3),
        Pos(3),
        rinch_editor_core::CursorMotion::CharRight,
        false,
    );
    assert_eq!(moved.as_ref(), Some(&sel));

    h.handle.set_selection(sel);
    assert!(h.handle.command("deleteCharBackward"));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 2, "the embed was removed");
    assert!(
        blocks.iter().all(|b| tag(&h, *b).as_deref() == Some("p")),
        "two paragraphs left"
    );
}

/// The default stylesheet gives an embed's host a line of height, so an
/// empty one is something to click.
#[test]
fn the_default_stylesheet_gives_an_embed_a_min_height() {
    let css = crate::styles::DEFAULT_EDITOR_CSS;
    let rule = css
        .lines()
        .find(|l| l.starts_with(r#"[data-pm-editor] [data-pm-type="embed"] {"#))
        .expect("an embed rule");
    assert!(rule.contains("min-height: 1lh"), "{rule}");
}

#[test]
fn typing_replaces_an_inline_image_node_selection() {
    let s = schema();
    // doc(paragraph(text "a", image)) — positions 0[p 1 a 2 (img) 3]4.
    let p = s
        .branch(
            "paragraph",
            Fragment::from_children(vec![s.text("a").unwrap(), img(&s)]),
        )
        .unwrap();
    let h = mount(doc_node(&s, vec![p]));
    // The image is the paragraph's 2nd inline host child; node-select it.
    let para_host = children(&h, h.container_id)[0];
    let img_host = children(&h, para_host)[1];
    let sel = h
        .handle
        .node_selection_at_host(img_host.0)
        .expect("inline image node-selects");
    h.handle.set_selection(sel);

    // A flat text insert *can* replace an inline image (text is valid inline
    // content) — the image becomes the typed text within the paragraph.
    assert!(h.handle.update(|state| {
        let mut tr = state.tr();
        tr.insert_text("X").ok()?;
        Some(tr)
    }));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("aX")
    );
    assert!(
        h.handle.selection().is_empty(),
        "cursor collapses after insert"
    );
}

#[test]
fn insert_text_over_a_block_node_selection_deletes_then_inserts() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab"), hr(&s)]));
    let sel = h
        .handle
        .node_selection_at_host(children(&h, h.container_id)[1].0)
        .unwrap();
    h.handle.set_selection(sel);

    // Typing over a *block* node selection (a selected hr, where a bare text
    // node isn't valid `doc` content) deletes the node, then inserts the text
    // at the resulting cursor — landing at the end of the previous paragraph.
    assert!(h.handle.insert_text("X"));
    let blocks = children(&h, h.container_id);
    assert_eq!(blocks.len(), 1, "the hr was removed");
    assert_eq!(text(&h, blocks[0]).as_deref(), Some("abX"));
    assert!(
        h.handle.selection().is_empty(),
        "cursor collapses after insert"
    );
}

// ── IME (input method editor) ────────────────────────────────────────────
