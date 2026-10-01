//! #1224 review fixtures: `BatchStep::map` and duplicate attribute changes.

use rinch_editor_core::*;

/// R1: `Step::map` of a batch whose kept edits come to meet end to end
/// (an insert right after a deletion, once the content between them is gone)
/// drops the WHOLE step — `BatchStep::new` refuses the pair and `map` turns
/// that into `None`. The same edits as separate `ReplaceStep`s both survive.
#[test]
fn mapping_a_batch_keeps_edits_a_sequence_keeps() {
    let s = Schema::starter_kit();
    let p = Fragment::from_node(
        s.branch("paragraph", Fragment::from_node(s.text("p").unwrap()))
            .unwrap(),
    );
    let batch = BatchStep::new(vec![
        BatchEdit::delete(2, 4),
        BatchEdit::insert(6, p.clone()),
    ])
    .unwrap();
    // Someone else deleted 4..6, the content between the two edits.
    let mut over = Mapping::new();
    over.append_map(StepMap::new(vec![4, 2, 0]));

    let del = ReplaceStep::new(2, 4, Slice::empty()).map(&over);
    let ins = ReplaceStep::new(6, 6, Slice::from_fragment(p)).map(&over);
    assert!(
        del.is_some() && ins.is_some(),
        "positive control: as separate steps both survive"
    );
    assert!(
        batch.map(&over).is_some(),
        "the batch dropped both its edits under a mapping each survives"
    );
}

/// R2: two attribute changes on the same node and attribute. The spliced
/// path keeps the last; the one-at-a-time path (any level whose replace cuts
/// into a child) keeps the first. One step, two answers.
#[test]
fn duplicate_attribute_changes_resolve_the_same_way_on_both_paths() {
    let s = Schema::starter_kit();
    let heading = s
        .create_node(
            "heading",
            Attrs::from_iter([("level", AttrValue::Int(1))]),
            Fragment::from_node(s.text("h").unwrap()),
        )
        .unwrap();
    let para = s
        .branch("paragraph", Fragment::from_node(s.text("ab").unwrap()))
        .unwrap();
    let doc = s
        .branch("doc", Fragment::from_children(vec![heading, para]))
        .unwrap();
    let attrs = || {
        vec![
            BatchEdit::set_attr(0, "level", AttrValue::Int(2)),
            BatchEdit::set_attr(0, "level", AttrValue::Int(3)),
        ]
    };
    let level = |d: &Node| d.child(0).attrs().get_int("level");

    let spliced = BatchStep::new(attrs()).unwrap().apply(&doc).unwrap();
    // The same two changes, plus a deletion that joins the heading and the
    // paragraph (cuts into both children → the doc level goes one by one).
    let mut edits = attrs();
    edits.push(BatchEdit::delete(2, 5));
    let one_by_one = BatchStep::new(edits).unwrap().apply(&doc).unwrap();
    assert_eq!(level(&spliced), Some(3), "the last change given wins");
    assert_eq!(
        level(&spliced),
        level(&one_by_one),
        "spliced {spliced:?}\n one by one {one_by_one:?}"
    );
}
