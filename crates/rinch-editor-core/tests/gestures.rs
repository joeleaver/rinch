//! One test per row of the design's "editing gesture → steps" table, driven
//! through the [`Transform`] builder (the public gesture API M4 commands compose).

use rinch_editor_core::*;

fn sk() -> Schema {
    Schema::starter_kit()
}

fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

fn bold(s: &Schema) -> Mark {
    Mark::simple(s.mark_type("bold").unwrap().clone())
}

/// Invert every step of `tr` (in reverse, against its pre-step snapshots) and
/// confirm we land back on `original` — the whole-transform undo invariant.
fn assert_undo(tr: &Transform, original: &Node) {
    let mut d = tr.doc.clone();
    for i in (0..tr.steps().len()).rev() {
        let pre = &tr.docs()[i];
        let inv = tr.steps()[i].invert(pre);
        d = inv.apply(&d).unwrap();
    }
    assert_eq!(
        &d, original,
        "inverting the whole transform must restore the doc"
    );
}

// --- type a character ------------------------------------------------------

#[test]
fn type_a_character() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.insert_text(2, 2, "X").unwrap();
    assert_eq!(all_text(&tr.doc), "aXb");
    assert!(tr.doc_changed());
    assert_undo(&tr, &d);
}

// --- backspace mid-text ----------------------------------------------------

#[test]
fn backspace_mid_text() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abc")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.delete(2, 3).unwrap(); // remove 'b'
    assert_eq!(all_text(&tr.doc), "ac");
    assert_undo(&tr, &d);
}

// --- backspace at block start (join) ---------------------------------------

#[test]
fn backspace_at_block_start_joins() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab"), para(&s, "cd")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.delete(3, 5).unwrap(); // join the two paragraphs
    assert_eq!(tr.doc.child_count(), 1);
    assert_eq!(all_text(&tr.doc), "abcd");
    assert_undo(&tr, &d);
}

// --- delete forward --------------------------------------------------------

#[test]
fn delete_forward() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abc")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.delete(1, 2).unwrap(); // delete 'a'
    assert_eq!(all_text(&tr.doc), "bc");
}

// --- enter / split block ---------------------------------------------------

#[test]
fn enter_splits_block() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.split(2, 1, None).unwrap(); // split between a and b
    assert_eq!(tr.doc.child_count(), 2);
    assert_eq!(all_text(tr.doc.child(0)), "a");
    assert_eq!(all_text(tr.doc.child(1)), "b");
    assert_undo(&tr, &d);
}

#[test]
fn enter_at_end_of_heading_makes_paragraph() {
    let s = sk();
    let h = s
        .branch("heading", Fragment::from_node(s.text("Title").unwrap()))
        .unwrap();
    let d = doc(&s, vec![h]);
    let mut tr = Transform::new(&s, d.clone());
    // split at end of heading content (pos 6 = after "Title"), new block = paragraph
    let para_ty = s.node_type("paragraph").unwrap().clone();
    tr.split(6, 1, Some(vec![Some((para_ty, Attrs::new()))]))
        .unwrap();
    assert_eq!(tr.doc.child(0).type_name(), "heading");
    assert_eq!(tr.doc.child(1).type_name(), "paragraph");
    assert_undo(&tr, &d);
}

// --- shift+enter / hard break ----------------------------------------------

#[test]
fn shift_enter_inserts_hard_break() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let hb = s.branch("hard_break", Fragment::empty()).unwrap();
    let mut tr = Transform::new(&s, d.clone());
    tr.replace_with(2, 2, Fragment::from_node(hb)).unwrap();
    // paragraph now: text "a", hard_break, text "b"
    assert_eq!(tr.doc.child(0).child_count(), 3);
    assert_eq!(tr.doc.child(0).child(1).type_name(), "hard_break");
    assert_undo(&tr, &d);
}

// --- toggle bold over a range ----------------------------------------------

#[test]
fn toggle_bold_over_range() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let mut tr = Transform::new(&s, d.clone());
    tr.add_mark(2, 4, bold(&s)).unwrap(); // bold "bc"
    let has_bold = tr
        .doc
        .child(0)
        .content()
        .iter()
        .any(|n| n.text() == Some("bc") && !n.marks().is_empty());
    assert!(has_bold);
    assert_undo(&tr, &d);
}

#[test]
fn remove_bold_over_range() {
    let s = sk();
    let bolded = s
        .branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks("abcd", vec![bold(&s)]).unwrap()),
        )
        .unwrap();
    let d = doc(&s, vec![bolded]);
    let mut tr = Transform::new(&s, d.clone());
    tr.remove_mark(2, 4, bold(&s)).unwrap();
    // the middle run "bc" loses bold
    assert_eq!(all_text(&tr.doc), "abcd");
    assert_undo(&tr, &d);
}

// --- set heading level (attr) ----------------------------------------------

#[test]
fn set_heading_level() {
    let s = sk();
    let h = s
        .branch("heading", Fragment::from_node(s.text("T").unwrap()))
        .unwrap();
    let d = doc(&s, vec![h]);
    let mut tr = Transform::new(&s, d.clone());
    tr.set_node_attr(0, "level", AttrValue::Int(3)).unwrap();
    assert_eq!(tr.doc.child(0).attrs().get_int("level"), Some(3));
    assert_undo(&tr, &d);
}

// --- paragraph -> heading (set block type) ---------------------------------

#[test]
fn paragraph_to_heading() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let heading_ty = s.node_type("heading").unwrap().clone();
    let attrs = Attrs::from_iter([("level", AttrValue::Int(2))]);
    let mut tr = Transform::new(&s, d.clone());
    tr.set_block_type(0, 4, heading_ty, attrs).unwrap();
    assert_eq!(tr.doc.child(0).type_name(), "heading");
    assert_eq!(tr.doc.child(0).attrs().get_int("level"), Some(2));
    assert_eq!(all_text(&tr.doc), "ab");
    assert_undo(&tr, &d);
}

// --- wrap in blockquote ----------------------------------------------------

#[test]
fn wrap_in_blockquote() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let r_from = d.resolve(Pos(1)).unwrap();
    let r_to = d.resolve(Pos(3)).unwrap();
    let range = block_range_simple(&r_from, &r_to).unwrap();
    let bq = s.node_type("blockquote").unwrap().clone();
    let mut tr = Transform::new(&s, d.clone());
    tr.wrap(&range, &[(bq, Attrs::new())]).unwrap();
    assert_eq!(tr.doc.child(0).type_name(), "blockquote");
    assert_eq!(tr.doc.child(0).child(0).type_name(), "paragraph");
    assert_undo(&tr, &d);
}

// --- toggle bullet list (wrap into list_item + list) -----------------------

#[test]
fn wrap_in_bullet_list() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let r_from = d.resolve(Pos(1)).unwrap();
    let r_to = d.resolve(Pos(3)).unwrap();
    let range = block_range_simple(&r_from, &r_to).unwrap();
    let list = s.node_type("bullet_list").unwrap().clone();
    let item = s.node_type("list_item").unwrap().clone();
    let mut tr = Transform::new(&s, d.clone());
    tr.wrap(&range, &[(list, Attrs::new()), (item, Attrs::new())])
        .unwrap();
    assert_eq!(tr.doc.child(0).type_name(), "bullet_list");
    assert_eq!(tr.doc.child(0).child(0).type_name(), "list_item");
    assert_eq!(tr.doc.child(0).child(0).child(0).type_name(), "paragraph");
    assert_undo(&tr, &d);
}

// --- outdent (lift out of blockquote) --------------------------------------

#[test]
fn lift_paragraph_out_of_blockquote() {
    let s = sk();
    let inner = para(&s, "x");
    let bq = s.branch("blockquote", Fragment::from_node(inner)).unwrap();
    let d = doc(&s, vec![bq]);
    // resolve inside the paragraph (pos 2), range over it within the blockquote
    let r = d.resolve(Pos(2)).unwrap();
    let range = block_range_simple(&r, &r).unwrap();
    assert_eq!(range.depth, 1); // parent = blockquote
    let mut tr = Transform::new(&s, d.clone());
    tr.lift(&range, 0).unwrap(); // lift to doc level
    assert_eq!(tr.doc.child(0).type_name(), "paragraph");
    assert_eq!(all_text(&tr.doc), "x");
    assert_undo(&tr, &d);
}

// --- insert hr -------------------------------------------------------------

#[test]
fn insert_horizontal_rule() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let hr = s.branch("horizontal_rule", Fragment::empty()).unwrap();
    let mut tr = Transform::new(&s, d.clone());
    // insert between the two paragraphs-to-be: at the doc-level gap after the paragraph (pos 4)
    tr.replace_with(4, 4, Fragment::from_node(hr)).unwrap();
    assert_eq!(tr.doc.child_count(), 2);
    assert_eq!(tr.doc.child(1).type_name(), "horizontal_rule");
    assert_undo(&tr, &d);
}

// --- insert link (add link mark) -------------------------------------------

#[test]
fn insert_link_mark() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let link = Mark::new(
        s.mark_type("link").unwrap().clone(),
        Attrs::from_iter([("href", AttrValue::from("https://x.test"))]),
    );
    let mut tr = Transform::new(&s, d.clone());
    tr.add_mark(2, 4, link).unwrap();
    let linked = tr.doc.child(0).content().iter().any(|n| {
        n.text() == Some("bc")
            && n.marks().iter().any(|m| {
                m.type_name() == "link" && m.attrs.get_str("href") == Some("https://x.test")
            })
    });
    assert!(
        linked,
        "expected a linked 'bc' run, got {:?}",
        tr.doc.child(0)
    );
    assert_undo(&tr, &d);
}

// --- insert image (inline atom) --------------------------------------------

#[test]
fn insert_image() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let img = s
        .create_node(
            "image",
            Attrs::from_iter([("src", AttrValue::from("a.png"))]),
            Fragment::empty(),
        )
        .unwrap();
    let mut tr = Transform::new(&s, d.clone());
    tr.replace_with(2, 2, Fragment::from_node(img)).unwrap();
    assert_eq!(tr.doc.child(0).child_count(), 3);
    assert_eq!(tr.doc.child(0).child(1).type_name(), "image");
    assert_undo(&tr, &d);
}

// --- paste (open slice) ----------------------------------------------------

#[test]
fn paste_open_slice() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let slice = Slice::new(
        Fragment::from_children(vec![para(&s, "X"), para(&s, "Y")]),
        1,
        1,
    );
    let mut tr = Transform::new(&s, d.clone());
    tr.replace(2, 2, slice).unwrap();
    assert_eq!(tr.doc.child_count(), 2);
    assert_eq!(all_text(tr.doc.child(0)), "aX");
    assert_eq!(all_text(tr.doc.child(1)), "Yb");
    assert_undo(&tr, &d);
}

// --- regression: split depth must not underflow (verification must-fix 1) ---

#[test]
fn split_depth_beyond_position_errors_not_panics() {
    let s = sk();
    // pos 4 is the top-level gap between the two paragraphs (resolves to depth 0).
    let d = doc(&s, vec![para(&s, "ab"), para(&s, "cd")]);
    let mut tr = Transform::new(&s, d.clone());
    assert!(tr.split(4, 1, None).is_err());
    // depth 2 at a depth-1 position.
    let d2 = doc(&s, vec![para(&s, "ab")]);
    let mut tr2 = Transform::new(&s, d2);
    assert!(tr2.split(2, 2, None).is_err());
}

// --- regression: degenerate block range must not panic (must-fix 2) ---------

#[test]
fn degenerate_cross_boundary_range_does_not_panic() {
    let s = sk();
    // doc(blockquote(paragraph "ab"), paragraph "cd")
    let bq = s
        .branch("blockquote", Fragment::from_node(para(&s, "ab")))
        .unwrap();
    let d = doc(&s, vec![bq, para(&s, "cd")]);
    let r_from = d.resolve(Pos(2)).unwrap(); // inside blockquote>paragraph (depth 2)
    let r_to = d.resolve(Pos(6)).unwrap(); // the doc-level gap after the blockquote (depth 0)
    let range = block_range_simple(&r_from, &r_to).unwrap();
    // start()/end() must be total (no out-of-bounds path index)
    let _ = range.start();
    let _ = range.end();
    // wrap/lift over the degenerate range must return a Result, not panic
    let bqty = s.node_type("blockquote").unwrap().clone();
    let mut tr = Transform::new(&s, d.clone());
    let _ = tr.wrap(&range, &[(bqty, Attrs::new())]);
}

// --- mapping accumulates across steps --------------------------------------

#[test]
fn mapping_tracks_position_through_multiple_inserts() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let mut tr = Transform::new(&s, d.clone());
    // a position at the end of "ab" (pos 3) tracked through two leading inserts
    tr.insert_text(1, 1, "XX").unwrap(); // "XXab"
    tr.insert_text(1, 1, "Y").unwrap(); // "YXXab"
    let mapped = tr.mapping().map(3, 1);
    // original pos 3 ("ab|") shifted by +2 then +1 = +3 -> 6
    assert_eq!(mapped, 6);
    assert_eq!(all_text(&tr.doc), "YXXab");
}
