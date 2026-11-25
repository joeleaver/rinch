//! Marks, block types, wrapping, links and the toolbar queries.

use super::*;

#[test]
fn select_all_spans_the_document() {
    let state = editor("hello");
    let out = state.run("selectAll").expect("selectAll always applies");
    // doc > paragraph > "hello": content runs 1..6.
    assert_eq!((out.selection.from().0, out.selection.to().0), (1, 6));
    // Idempotent — still applies (consumes the key) when already fully selected.
    assert!(out.run("selectAll").is_some());
}

#[test]
fn toggle_bold_over_range() {
    let mut state = editor("abcd");
    state.selection = Selection::text(Pos(1), Pos(5)); // whole "abcd"
    let next = state.run("toggleBold").expect("toggleBold applies");
    let bolded = next
        .doc
        .child(0)
        .content()
        .iter()
        .all(|n| n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(bolded, "whole range should be bold");
    // toggling again removes it
    let cleared = next.run("toggleBold").unwrap();
    assert!(!range_has_mark(
        &cleared.doc,
        1,
        5,
        cleared.schema().mark_type("bold").unwrap()
    ));
}

#[test]
fn cursor_bold_then_type_is_bold() {
    // audit S2 — toggle bold at a collapsed cursor sets stored marks; typing applies them
    let mut state = editor("ab");
    state.selection = Selection::cursor(Pos(3)); // end of "ab"
    let state = state
        .run("toggleBold")
        .expect("toggleBold (cursor) applies");
    assert!(is_mark_active(
        &state,
        state.schema().mark_type("bold").unwrap()
    ));
    let mut tr = state.tr();
    tr.insert_text("X").unwrap();
    let state = state.apply(tr);
    let x_is_bold = state
        .doc
        .child(0)
        .content()
        .iter()
        .any(|n| n.text() == Some("X") && n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(x_is_bold, "typed X after cursor-bold should be bold");
}

#[test]
fn set_heading_and_back_to_paragraph() {
    let state = editor("title");
    let h = state.run("setHeading2").expect("setHeading2 applies");
    assert_eq!(h.doc.child(0).type_name(), "heading");
    assert_eq!(h.doc.child(0).attrs().get_int("level"), Some(2));
    // setHeading2 again is a no-op (already that markup) → not applicable
    assert!(!h.can_run("setHeading2"));
    let p = h.run("setParagraph").expect("setParagraph applies");
    assert_eq!(p.doc.child(0).type_name(), "paragraph");
}

#[test]
fn wrap_blockquote_and_lift_back() {
    let state = editor("x");
    let bq = state.run("wrapInBlockquote").expect("wrap applies");
    assert_eq!(bq.doc.child(0).type_name(), "blockquote");
    assert!(in_node_type(&bq, "blockquote"));
    let lifted = bq.run("liftListItem").expect("lift applies");
    assert_eq!(lifted.doc.child(0).type_name(), "paragraph");
}

#[test]
fn toggle_bullet_list_and_back() {
    let state = editor("item");
    let list = state.run("toggleBulletList").expect("wrap into list");
    assert_eq!(list.doc.child(0).type_name(), "bullet_list");
    assert_eq!(list.doc.child(0).child(0).type_name(), "list_item");
    assert!(in_node_type(&list, "bullet_list"));
    // toggling again lifts out of the list
    let back = list.run("toggleBulletList").expect("lift out of list");
    assert!(!in_node_type(&back, "bullet_list"));
    assert_eq!(back.doc.child(0).type_name(), "paragraph");
}

#[test]
fn link_command_adds_and_removes() {
    let mut state = editor("abcd");
    state.selection = Selection::text(Pos(1), Pos(5));
    let cmd = toggle_link("https://x.test".into());
    let linked = state.run_command(&cmd).expect("link applies");
    let has_link = linked.doc.child(0).content().iter().any(|n| {
        n.marks()
            .iter()
            .any(|m| m.type_name() == "link" && m.attrs.get_str("href") == Some("https://x.test"))
    });
    assert!(has_link);
    let unlinked = linked.run("removeLink").expect("removeLink applies");
    assert!(!range_has_mark(
        &unlinked.doc,
        1,
        5,
        unlinked.schema().mark_type("link").unwrap()
    ));
}

#[test]
fn toolbar_queries_read_state() {
    let mut state = editor("abcd");
    state.selection = Selection::text(Pos(1), Pos(5));
    let bold = state.run("toggleBold").unwrap();
    assert!(is_mark_active(
        &bold,
        bold.schema().mark_type("bold").unwrap()
    ));
    assert_eq!(
        current_block_type(&bold).map(|t| t.name().to_string()),
        Some("paragraph".to_string())
    );
    let listed = bold.run("toggleBulletList").unwrap();
    assert!(in_node_type(&listed, "bullet_list"));
    // inside a list the textblock is still a paragraph (A6's point)
    assert_eq!(
        current_block_type(&listed).map(|t| t.name().to_string()),
        Some("paragraph".to_string())
    );
}
