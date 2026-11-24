//! IME composition through the handle.

use super::*;

#[test]
fn ime_preedit_is_view_local_and_commit_inserts_one_edit() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    h.handle.set_selection(Selection::cursor(Pos(3))); // end of "ab"

    // Composing shows a preedit overlay but never touches the document.
    h.handle.ime_set_preedit("ne", None);
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("ab"),
        "preedit is a view overlay, not part of the document"
    );

    // Commit inserts the final text as one ordinary edit.
    h.handle.ime_commit("ね");
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("abね")
    );
    // ...and it's a single undo step, exactly like typing.
    assert!(h.handle.command("undo"));
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("ab")
    );
}

#[test]
fn ime_clear_and_empty_commit_leave_doc_unchanged() {
    let s = schema();
    let h = mount(doc_node(&s, vec![para(&s, "ab")]));
    h.handle.set_selection(Selection::cursor(Pos(3)));

    h.handle.ime_set_preedit("xy", None);
    h.handle.ime_clear_preedit(); // composition cancelled
    h.handle.ime_commit(""); // an empty commit clears, inserts nothing
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("ab")
    );
}

#[test]
fn ime_delete_surrounding_deletes_around_caret() {
    let s = schema();
    // doc(paragraph "abcd") — positions 0[p 1 a 2 b 3 c 4 d 5]6.
    let h = mount(doc_node(&s, vec![para(&s, "abcd")]));
    h.handle.set_selection(Selection::cursor(Pos(3))); // between "b" and "c"
    h.handle.ime_delete_surrounding(1, 1); // delete "b" and "c"
    assert_eq!(
        text(&h, children(&h, h.container_id)[0]).as_deref(),
        Some("ad")
    );
}

#[test]
fn ime_delete_surrounding_across_a_cell_edge_repairs_the_table() {
    // #1930: from "e|f" (row 2, cell 1) to "z|w" after the table. The fit
    // leaves row 2 one cell short; the IME delete pads it, as Backspace
    // over the same range does.
    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());
    assert!(h.load_html(
        "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
         <tr><td><p>ef</p></td><td><p>gh</p></td></tr></table><p>zw</p>"
    ));
    h.set_selection(Selection::cursor(Pos(22)));
    h.ime_delete_surrounding(0, 13);
    assert_eq!(
        rinch_editor_core::serialize::node_to_html(&h.doc()),
        "<p>x</p><table><tr><td><p>ab</p></td><td><p>cd</p></td></tr>\
         <tr><td><p>ew</p></td><td><p></p></td></tr></table>"
    );
    assert!(rinch_editor_core::tables::table_problems(h.doc().child(1)).is_empty());
}

#[test]
fn ime_methods_are_safe_before_mount() {
    let s = Rc::new(schema());
    let h = EditorHandle::unmounted(s.clone(), empty_doc(&s), default_plugins());
    h.set_selection(Selection::cursor(Pos(1))); // inside the empty paragraph
    // No view yet → preedit overlay ops are safe no-ops, but commit still edits
    // the owned state.
    h.ime_set_preedit("ab", None);
    h.ime_clear_preedit();
    h.ime_commit("hi");
    // "hi" landed in the document even though the editor isn't mounted.
    let doc = h.doc();
    assert_eq!(doc.child_count(), 1);
    assert_eq!(doc.child(0).child(0).text(), Some("hi"));
}

// ── Deferred mount (create_editor → load → attach) ───────────────────────
