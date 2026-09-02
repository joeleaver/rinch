//! Integration tests for the M2 transform engine, exercised through the public
//! API only (the surface M4+ and the views will build on).

use rinch_editor_core::*;

fn sk() -> Schema {
    Schema::starter_kit()
}

/// A paragraph with a single text run.
fn para(s: &Schema, text: &str) -> Node {
    s.branch("paragraph", Fragment::from_node(s.text(text).unwrap()))
        .unwrap()
}

/// A doc wrapping the given block children.
fn doc(s: &Schema, blocks: Vec<Node>) -> Node {
    s.branch("doc", Fragment::from_children(blocks)).unwrap()
}

/// All text in a subtree, concatenated (block boundaries ignored).
fn all_text(node: &Node) -> String {
    if let Some(t) = node.text() {
        return t.to_string();
    }
    node.content().iter().map(all_text).collect()
}

fn bold(s: &Schema) -> Mark {
    Mark::simple(s.mark_type("bold").unwrap().clone())
}

fn inline_slice(s: &Schema, text: &str) -> Slice {
    Slice::from_fragment(Fragment::from_node(s.text(text).unwrap()))
}

// ---------------------------------------------------------------------------
// ReplaceStep — insert / delete
// ---------------------------------------------------------------------------

#[test]
fn insert_text_mid_run() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let step = ReplaceStep::new(2, 2, inline_slice(&s, "X")); // between a and b
    let after = step.apply(&d).unwrap();
    assert_eq!(all_text(&after), "aXb");
    // one merged text run, not fragmented
    assert_eq!(after.child(0).child_count(), 1);
}

#[test]
fn insert_text_at_block_start_and_end() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    assert_eq!(
        all_text(
            &ReplaceStep::new(1, 1, inline_slice(&s, "X"))
                .apply(&d)
                .unwrap()
        ),
        "Xab"
    );
    assert_eq!(
        all_text(
            &ReplaceStep::new(3, 3, inline_slice(&s, "X"))
                .apply(&d)
                .unwrap()
        ),
        "abX"
    );
}

#[test]
fn delete_text_mid_run() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let step = ReplaceStep::new(2, 3, Slice::empty()); // delete "b"
    let after = step.apply(&d).unwrap();
    assert_eq!(all_text(&after), "acd");
}

#[test]
fn delete_across_blocks_joins_them() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab"), para(&s, "cd")]);
    // delete from end of p1 content (3) to start of p2 content (5)
    let after = ReplaceStep::new(3, 5, Slice::empty()).apply(&d).unwrap();
    assert_eq!(after.child_count(), 1);
    assert_eq!(all_text(&after), "abcd");
}

#[test]
fn paste_open_slice_splits_block() {
    // Insert "X<split>Y" into "a|b" -> p("aX"), p("Yb")
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let slice = Slice::new(
        Fragment::from_children(vec![para(&s, "X"), para(&s, "Y")]),
        1,
        1,
    );
    let after = ReplaceStep::new(2, 2, slice).apply(&d).unwrap();
    assert_eq!(after.child_count(), 2);
    assert_eq!(all_text(after.child(0)), "aX");
    assert_eq!(all_text(after.child(1)), "Yb");
}

// ---------------------------------------------------------------------------
// ReplaceStep — schema enforcement (decision G): invalid steps are rejected
// ---------------------------------------------------------------------------

#[test]
fn block_into_inline_position_is_rejected() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    // Try to insert a whole paragraph inside the paragraph's inline content.
    let step = ReplaceStep::new(
        2,
        2,
        Slice::from_fragment(Fragment::from_node(para(&s, "x"))),
    );
    assert!(step.apply(&d).is_err());
    // The original document value is untouched (apply is pure).
    assert_eq!(all_text(&d), "ab");
}

#[test]
fn text_into_doc_top_level_is_rejected() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    // doc content is block+, so bare text at the top is invalid.
    let step = ReplaceStep::new(4, 4, inline_slice(&s, "loose"));
    assert!(step.apply(&d).is_err());
}

// ---------------------------------------------------------------------------
// ReplaceStep — invert round-trips
// ---------------------------------------------------------------------------

fn assert_roundtrip(d: &Node, step: &dyn Step) {
    let after = step.apply(d).unwrap();
    let inverse = step.invert(d);
    let restored = inverse.apply(&after).unwrap();
    assert_eq!(&restored, d, "apply∘invert must be identity");
}

#[test]
fn invert_insert() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    assert_roundtrip(&d, &ReplaceStep::new(2, 2, inline_slice(&s, "XYZ")));
}

#[test]
fn invert_delete() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    assert_roundtrip(&d, &ReplaceStep::new(2, 4, Slice::empty()));
}

#[test]
fn invert_join_blocks() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab"), para(&s, "cd")]);
    assert_roundtrip(&d, &ReplaceStep::new(3, 5, Slice::empty()));
}

#[test]
fn invert_paste_split() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let slice = Slice::new(
        Fragment::from_children(vec![para(&s, "X"), para(&s, "Y")]),
        1,
        1,
    );
    assert_roundtrip(&d, &ReplaceStep::new(2, 2, slice));
}

// ---------------------------------------------------------------------------
// ReplaceStep — merge (typing coalescing)
// ---------------------------------------------------------------------------

#[test]
fn consecutive_single_char_inserts_merge() {
    let s = sk();
    // type "X" at 2 then "Y" at 3 — the second is adjacent to the first
    let a = ReplaceStep::new(2, 2, inline_slice(&s, "X"));
    let b = ReplaceStep::new(3, 3, inline_slice(&s, "Y"));
    let merged = a.merge(&b).expect("adjacent inserts should merge");
    // Apply the merged step and confirm it equals applying both in sequence.
    let d = doc(&s, vec![para(&s, "ab")]);
    let seq = b.apply(&a.apply(&d).unwrap()).unwrap();
    let one = merged.apply(&d).unwrap();
    assert_eq!(one, seq);
    assert_eq!(all_text(&one), "aXYb");
}

#[test]
fn non_adjacent_inserts_do_not_merge() {
    let s = sk();
    let a = ReplaceStep::new(2, 2, inline_slice(&s, "X"));
    let c = ReplaceStep::new(1, 1, inline_slice(&s, "Z"));
    assert!(a.merge(&c).is_none());
}

// ---------------------------------------------------------------------------
// ReplaceAroundStep — wrap / lift
// ---------------------------------------------------------------------------

#[test]
fn wrap_paragraph_in_blockquote() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]); // paragraph spans 0..4
    let bq = s.branch("blockquote", Fragment::empty()).unwrap();
    let slice = Slice::new(Fragment::from_node(bq), 0, 0);
    let step = ReplaceAroundStep::new(0, 4, 0, 4, slice, 1, true);
    let after = step.apply(&d).unwrap();
    assert_eq!(after.child(0).type_name(), "blockquote");
    assert_eq!(after.child(0).child(0).type_name(), "paragraph");
    assert_eq!(all_text(&after), "ab");
}

#[test]
fn wrap_then_invert_restores() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "ab")]);
    let bq = s.branch("blockquote", Fragment::empty()).unwrap();
    let slice = Slice::new(Fragment::from_node(bq), 0, 0);
    assert_roundtrip(&d, &ReplaceAroundStep::new(0, 4, 0, 4, slice, 1, true));
}

// ---------------------------------------------------------------------------
// Mark steps
// ---------------------------------------------------------------------------

#[test]
fn add_mark_over_range() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    // bold "bc" -> positions 2..4
    let after = AddMarkStep::new(2, 4, bold(&s)).apply(&d).unwrap();
    // paragraph should now be [text "a", text "bc"(bold), text "d"]
    let p = after.child(0);
    assert_eq!(all_text(&after), "abcd");
    // find the bold run
    let has_bold = p
        .content()
        .iter()
        .any(|n| n.text() == Some("bc") && n.marks().iter().any(|m| m.type_name() == "bold"));
    assert!(has_bold, "expected a bold 'bc' run, got {p:?}");
}

#[test]
fn add_mark_invert_removes_it() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    assert_roundtrip(&d, &AddMarkStep::new(2, 4, bold(&s)));
}

#[test]
fn remove_mark_invert_adds_it() {
    let s = sk();
    // start with a fully-bold paragraph
    let bolded = s
        .branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks("abcd", vec![bold(&s)]).unwrap()),
        )
        .unwrap();
    let d = doc(&s, vec![bolded]);
    assert_roundtrip(&d, &RemoveMarkStep::new(1, 5, bold(&s)));
}

#[test]
fn add_mark_disallowed_in_code_block_is_a_noop_not_error() {
    let s = sk();
    // code_block has MarkSet::None; adding bold should leave content unmarked
    let cb = s
        .branch("code_block", Fragment::from_node(s.text("x").unwrap()))
        .unwrap();
    let d = doc(&s, vec![cb]);
    let after = AddMarkStep::new(1, 2, bold(&s)).apply(&d).unwrap();
    let marked = after.child(0).child(0).marks().is_empty();
    assert!(marked, "bold must not be applied inside a code_block");
}

// ---------------------------------------------------------------------------
// Foreign-schema marks (issue #217)
// ---------------------------------------------------------------------------

/// `MarkType` equality is `Rc::ptr_eq`, so a mark built from a *second*
/// `Schema::starter_kit()` can never equal one built from the first. `remove_mark` used
/// to answer that by matching nothing and returning `Ok` — a removal that silently
/// removed nothing, which is how a collab regression test passed vacuously for months.
#[test]
fn remove_mark_with_a_mark_from_another_schema_fails_loud() {
    let s = sk();
    let other = sk();
    let bolded = s
        .branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks("abcd", vec![bold(&s)]).unwrap()),
        )
        .unwrap();
    let d = doc(&s, vec![bolded]);
    let Err(err) = Transform::new(&s, d).remove_mark(1, 5, bold(&other)) else {
        panic!("a mark from another Schema must fail loud, not remove nothing");
    };
    assert!(
        err.to_string().contains("different Schema"),
        "expected a foreign-schema diagnosis, got: {err}"
    );
}

/// The hazard that actually bit (#217): the *mark* belongs to the transform's schema, so
/// a membership check passes — it is the **document** that was built by another
/// `Schema`. Only comparing the document's own mark handles catches this one.
#[test]
fn remove_mark_on_a_document_from_another_schema_fails_loud() {
    let s = sk();
    let other = sk();
    // The document's marks come from `other`; the transform (and the mark) from `s`.
    let bolded = other
        .branch(
            "paragraph",
            Fragment::from_node(other.text_with_marks("abcd", vec![bold(&other)]).unwrap()),
        )
        .unwrap();
    let d = other.branch("doc", Fragment::from_node(bolded)).unwrap();
    let Err(err) = Transform::new(&s, d).remove_mark(1, 5, bold(&s)) else {
        panic!("a document from another Schema must fail loud, not remove nothing");
    };
    assert!(
        err.to_string().contains("different Schema"),
        "expected a foreign-schema diagnosis, got: {err}"
    );
}

/// `add_mark`'s version is worse than a no-op: the foreign mark does not match, so it
/// used to be *added* beside the document's real one, leaving a node carrying two
/// same-named marks of different types.
#[test]
fn add_mark_with_a_mark_from_another_schema_fails_loud() {
    let s = sk();
    let other = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let Err(err) = Transform::new(&s, d).add_mark(2, 4, bold(&other)) else {
        panic!("a mark from another Schema must fail loud, not be added beside the real one");
    };
    assert!(
        err.to_string().contains("different Schema"),
        "expected a foreign-schema diagnosis, got: {err}"
    );
}

/// The guard must stay narrow. Finding no matching mark in the range is an **ordinary
/// no-op**, not an error — it is what `toggleBold` over unbolded text does — and so is a
/// same-type mark whose *attrs* differ (removing `link[href=a]` from `link[href=b]`).
/// Only a type-*identity* mismatch is reported.
#[test]
fn removing_a_mark_that_is_simply_absent_is_still_a_quiet_no_op() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "abcd")]);
    let mut tf = Transform::new(&s, d.clone());
    tf.remove_mark(1, 5, bold(&s))
        .expect("removing an absent mark is a no-op, not an error");
    assert_eq!(tf.doc, d, "and it changes nothing");

    // Same mark type, different attrs: a genuine non-match.
    let link = |href: &str| {
        Mark::new(
            s.mark_type("link").unwrap().clone(),
            Attrs::new().with("href", href.to_string()),
        )
    };
    let linked = s
        .branch(
            "paragraph",
            Fragment::from_node(s.text_with_marks("abcd", vec![link("b")]).unwrap()),
        )
        .unwrap();
    let d2 = doc(&s, vec![linked]);
    let mut tf2 = Transform::new(&s, d2.clone());
    tf2.remove_mark(1, 5, link("a"))
        .expect("a different href is an ordinary non-match, not a schema error");
    assert_eq!(tf2.doc, d2);
}

// ---------------------------------------------------------------------------
// Attr steps
// ---------------------------------------------------------------------------

#[test]
fn set_heading_level() {
    let s = sk();
    let h = s
        .branch("heading", Fragment::from_node(s.text("Title").unwrap()))
        .unwrap();
    let d = doc(&s, vec![h]);
    let step = SetNodeAttrStep::new(0, "level", AttrValue::Int(2));
    let after = step.apply(&d).unwrap();
    assert_eq!(after.child(0).attrs().get_int("level"), Some(2));
    assert_eq!(all_text(&after), "Title");
}

#[test]
fn set_node_attr_invert_restores_absent_attr() {
    let s = sk();
    // heading built via branch() has NO level attr set; setting then inverting
    // must restore "no level attr", not level=Null.
    let h = s
        .branch("heading", Fragment::from_node(s.text("T").unwrap()))
        .unwrap();
    let d = doc(&s, vec![h]);
    assert!(d.child(0).attrs().get("level").is_none());
    assert_roundtrip(&d, &SetNodeAttrStep::new(0, "level", AttrValue::Int(3)));
}

#[test]
fn set_doc_attr_and_invert() {
    let s = sk();
    let d = doc(&s, vec![para(&s, "x")]);
    assert_roundtrip(&d, &SetDocAttrStep::new("dir", AttrValue::from("rtl")));
    let after = SetDocAttrStep::new("dir", AttrValue::from("rtl"))
        .apply(&d)
        .unwrap();
    assert_eq!(after.attrs().get_str("dir"), Some("rtl"));
}

// ---------------------------------------------------------------------------
// StepMap / Mapping
// ---------------------------------------------------------------------------

#[test]
fn step_map_shifts_positions_after_an_insert() {
    // Insert of size 3 at position 2: positions >= 2 shift by +3.
    let map = StepMap::new(vec![2, 0, 3]);
    assert_eq!(map.map(1, 1), 1); // before the insert
    assert_eq!(map.map(5, 1), 8); // after the insert
    // a position exactly at the insertion point biases by assoc
    assert_eq!(map.map(2, 1), 5);
    assert_eq!(map.map(2, -1), 2);
}

#[test]
fn step_map_invert_round_trips_undeleted_positions() {
    // A replace of old size 2 with new size 4 at pos 3.
    let map = StepMap::new(vec![3, 2, 4]);
    let inv = map.invert();
    for pos in [0usize, 1, 2, 3, 7, 9, 12] {
        let fwd = map.map(pos, 1);
        // positions outside the replaced range round-trip exactly
        if pos <= 3 || pos >= 5 {
            assert_eq!(inv.map(fwd, 1), pos, "round-trip failed for {pos}");
        }
    }
}

#[test]
fn mapping_through_step_and_its_mirror_recovers_position() {
    // map then its inverse, registered as mirrors, must recover the original.
    let map = StepMap::new(vec![3, 2, 4]);
    let mut mapping = Mapping::new();
    mapping.append_map(map.clone());
    mapping.append_map_mirrored(map.invert(), 0);
    for pos in [0usize, 3, 4, 5, 9] {
        assert_eq!(mapping.map(pos, 1), pos, "mirror recovery failed for {pos}");
    }
}

// ---------------------------------------------------------------------------
// Property tests — exhaustive over small spaces (stronger than random sampling)
// ---------------------------------------------------------------------------

/// `apply∘invert == identity` for an insert at every textblock position and a
/// delete over every valid range, across several document shapes.
#[test]
fn invert_is_identity_exhaustively() {
    let s = sk();
    let nested = s
        .branch("blockquote", Fragment::from_node(para(&s, "xy")))
        .unwrap();
    let docs = vec![
        doc(&s, vec![para(&s, "hello")]),
        doc(&s, vec![para(&s, "ab"), para(&s, "cd")]),
        doc(&s, vec![nested]),
    ];
    for d in &docs {
        let size = d.content_size();
        // Insert text at every position that resolves inside a textblock.
        for pos in 0..=size {
            if let Ok(r) = d.resolve(Pos(pos))
                && r.parent().is_textblock()
            {
                let step = ReplaceStep::new(pos, pos, inline_slice(&s, "Z"));
                let after = step.apply(d).unwrap();
                assert_eq!(&step.invert(d).apply(&after).unwrap(), d, "insert@{pos}");
            }
        }
        // Delete every valid range that applies cleanly.
        for from in 0..=size {
            for to in from..=size {
                let step = ReplaceStep::new(from, to, Slice::empty());
                if let Ok(after) = step.apply(d) {
                    assert_eq!(
                        &step.invert(d).apply(&after).unwrap(),
                        d,
                        "delete {from}..{to}"
                    );
                }
            }
        }
    }
}

/// A position not inside the replaced range maps forward then back to itself
/// through a replace map and its inverse, for every range/size pair in a window.
#[test]
fn step_map_round_trips_outside_replaced_range_exhaustively() {
    for start in 0..6usize {
        for old_size in 0..6usize {
            for new_size in 0..6usize {
                let map = StepMap::new(vec![start, old_size, new_size]);
                let inv = map.invert();
                for pos in 0..12usize {
                    // Only positions STRICTLY outside the replaced range round-trip
                    // through a bare map+inverse; the boundary positions are
                    // intentionally ambiguous (recovered only via Mapping mirrors).
                    if pos < start || pos > start + old_size {
                        let fwd = map.map(pos, 1);
                        assert_eq!(
                            inv.map(fwd, 1),
                            pos,
                            "round-trip {pos} via [{start},{old_size},{new_size}]"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn replace_step_map_rebases_over_an_earlier_insert() {
    let s = sk();
    // An AddMark over 2..4, rebased over an insert of size 2 at position 0,
    // should shift to 4..6.
    let add = AddMarkStep::new(2, 4, bold(&s));
    let mut mapping = Mapping::new();
    mapping.append_map(StepMap::new(vec![0, 0, 2]));
    let mapped = add.map(&mapping).expect("step should survive mapping");
    let mapped = mapped.as_any().downcast_ref::<AddMarkStep>().unwrap();
    assert_eq!((mapped.from, mapped.to), (4, 6));
}
