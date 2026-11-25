use super::*;

/// a
/// ├ a1
/// │ ├ a1x
/// │ └ a1y
/// └ a2
/// b
/// c
/// └ c1
fn data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "a").with_children(vec![
            TreeNodeData::new("a1", "a1").with_children(vec![
                TreeNodeData::new("a1x", "a1x"),
                TreeNodeData::new("a1y", "a1y"),
            ]),
            TreeNodeData::new("a2", "a2"),
        ]),
        TreeNodeData::new("b", "b"),
        TreeNodeData::new("c", "c").with_child(TreeNodeData::new("c1", "c1")),
    ]
}

fn expanded(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| v.to_string()).collect()
}

/// The placement as values: (parent, slot, line level).
fn place(
    shape: &Shape,
    over: &str,
    zone: DropZone,
    pointer: Option<usize>,
    open: &HashSet<String>,
) -> (Option<String>, TreeSlot, usize) {
    let p = shape.placement(
        shape.index(over).unwrap(),
        zone,
        pointer,
        open,
        AfterOpenRow::FirstChild,
    );
    let name = |row: usize| shape.rows[row].value.clone();
    let slot = match p.slot {
        Slot::First => TreeSlot::First,
        Slot::Last => TreeSlot::Last,
        Slot::After(row) => TreeSlot::After(name(row)),
    };
    (p.parent.map(name), slot, p.line_level)
}

fn after(v: &str) -> TreeSlot {
    TreeSlot::After(v.to_string())
}

#[test]
fn zones_split_a_row_into_quarter_half_quarter() {
    assert_eq!(zone_at(0.0, true), DropZone::Before);
    assert_eq!(zone_at(0.24, true), DropZone::Before);
    assert_eq!(zone_at(0.5, true), DropZone::Inside);
    assert_eq!(zone_at(0.76, true), DropZone::After);
    assert_eq!(zone_at(1.0, true), DropZone::After);
    // Review of #1589: off the fixed points, so a half/half split fails.
    assert_eq!(zone_at(0.3, true), DropZone::Inside);
    assert_eq!(zone_at(0.7, true), DropZone::Inside);
}

#[test]
fn before_lands_after_the_previous_sibling_or_first() {
    let shape = Shape::of(&data(), None);
    let open = expanded(&["a", "a1", "c"]);
    assert_eq!(
        place(&shape, "a2", DropZone::Before, None, &open),
        (Some("a".into()), after("a1"), 1)
    );
    assert_eq!(
        place(&shape, "a1", DropZone::Before, None, &open),
        (Some("a".into()), TreeSlot::First, 1)
    );
    assert_eq!(
        place(&shape, "a", DropZone::Before, None, &open),
        (None, TreeSlot::First, 0)
    );
}

#[test]
fn inside_lands_last() {
    let shape = Shape::of(&data(), None);
    let open = expanded(&[]);
    assert_eq!(
        place(&shape, "b", DropZone::Inside, None, &open),
        (Some("b".into()), TreeSlot::Last, 0)
    );
    assert_eq!(
        place(&shape, "a", DropZone::Inside, None, &open),
        (Some("a".into()), TreeSlot::Last, 0)
    );
}

#[test]
fn after_an_open_row_with_children_lands_first_inside_it() {
    let shape = Shape::of(&data(), None);
    let open = expanded(&["a"]);
    assert_eq!(
        place(&shape, "a", DropZone::After, None, &open),
        (Some("a".into()), TreeSlot::First, 1)
    );
    // Closed, the same zone is after it among its siblings.
    let closed = expanded(&[]);
    assert_eq!(
        place(&shape, "a", DropZone::After, None, &closed),
        (None, after("a"), 0)
    );
}

/// [`AfterOpenRow::ByColumn`]: just below an open row, the pointer's
/// column picks first inside (one indent in or further) or after it
/// among its siblings (its own indent or left); no column is inside;
/// just above its first child is first inside either way.
#[test]
fn after_an_open_row_by_column_is_inside_or_beside_it() {
    let shape = Shape::of(&data(), None);
    let open = expanded(&["a", "a1"]);
    let at = |row: &str, zone, pointer| {
        let p = shape.placement(
            shape.index(row).unwrap(),
            zone,
            pointer,
            &open,
            AfterOpenRow::ByColumn,
        );
        (
            p.parent.map(|r| shape.rows[r].value.clone()),
            p.slot.clone(),
            p.line_level,
        )
    };
    let after = |row: &str| Slot::After(shape.index(row).unwrap());
    // Top level `a`: column 0 or left is after it; 1 or more is inside.
    assert_eq!(at("a", DropZone::After, Some(0)), (None, after("a"), 0));
    assert_eq!(
        at("a", DropZone::After, Some(1)),
        (Some("a".into()), Slot::First, 1)
    );
    assert_eq!(
        at("a", DropZone::After, Some(9)),
        (Some("a".into()), Slot::First, 1)
    );
    assert_eq!(
        at("a", DropZone::After, None),
        (Some("a".into()), Slot::First, 1)
    );
    // Nested `a1` (level 1): column 1 or left (0 too: no shallower depth
    // is offered below an open row) is after it in `a`; 2 is inside.
    assert_eq!(
        at("a1", DropZone::After, Some(0)),
        (Some("a".into()), after("a1"), 1)
    );
    assert_eq!(
        at("a1", DropZone::After, Some(1)),
        (Some("a".into()), after("a1"), 1)
    );
    assert_eq!(
        at("a1", DropZone::After, Some(2)),
        (Some("a1".into()), Slot::First, 2)
    );
    // Above the first child: first inside.
    assert_eq!(
        at("a1x", DropZone::Before, Some(0)),
        (Some("a1".into()), Slot::First, 2)
    );
    // A leaf is unchanged: after it, the end-of-group depths from the
    // pointer (`a2` follows, so no shallower than level 1).
    assert_eq!(
        at("a1y", DropZone::After, Some(0)),
        (Some("a".into()), after("a1"), 1)
    );
    assert_eq!(
        at("a1y", DropZone::After, Some(9)),
        (Some("a1".into()), after("a1y"), 2)
    );
}

/// The drag that outdents: after the last row of a group, the pointer's
/// indent column picks the depth, between the row's own level and the
/// level of the row drawn below it.
#[test]
fn after_the_end_of_a_group_takes_its_depth_from_the_pointer() {
    let shape = Shape::of(&data(), None);
    let open = expanded(&["a", "a1"]);
    // a1y is the last of a1, which is not the last of a; below it is a2
    // (level 1), so levels 1..=2 are on offer.
    assert_eq!(
        place(&shape, "a1y", DropZone::After, Some(9), &open),
        (Some("a1".into()), after("a1y"), 2)
    );
    assert_eq!(
        place(&shape, "a1y", DropZone::After, Some(2), &open),
        (Some("a1".into()), after("a1y"), 2)
    );
    assert_eq!(
        place(&shape, "a1y", DropZone::After, Some(1), &open),
        (Some("a".into()), after("a1"), 1)
    );
    assert_eq!(
        place(&shape, "a1y", DropZone::After, Some(0), &open),
        (Some("a".into()), after("a1"), 1)
    );
    // Without a pointer column, the row's own depth.
    assert_eq!(
        place(&shape, "a1y", DropZone::After, None, &open),
        (Some("a1".into()), after("a1y"), 2)
    );
    // a2 ends a, and b (level 0) is below: level 0 means after a.
    assert_eq!(
        place(&shape, "a2", DropZone::After, Some(0), &open),
        (None, after("a"), 0)
    );
    // The last row of the tree can go all the way out.
    let open = expanded(&["c"]);
    assert_eq!(
        place(&shape, "c1", DropZone::After, Some(0), &open),
        (None, after("c"), 0)
    );
    // A row in the middle of its siblings offers only its own level.
    let open = expanded(&["a", "a1"]);
    assert_eq!(
        place(&shape, "a1x", DropZone::After, Some(0), &open),
        (Some("a1".into()), after("a1x"), 2)
    );
}

#[test]
fn next_visible_follows_what_is_expanded() {
    let shape = Shape::of(&data(), None);
    let next = |v: &str, open: &[&str]| {
        shape
            .next_visible(shape.index(v).unwrap(), &expanded(open))
            .map(|r| shape.rows[r].value.clone())
    };
    assert_eq!(next("a", &[]), Some("b".into()));
    assert_eq!(next("a", &["a"]), Some("a1".into()));
    assert_eq!(next("a1y", &["a", "a1"]), Some("a2".into()));
    assert_eq!(next("a2", &["a"]), Some("b".into()));
    assert_eq!(next("c1", &["c"]), None);
}

#[test]
fn within_is_the_row_or_under_it() {
    let shape = Shape::of(&data(), None);
    let i = |v: &str| shape.index(v).unwrap();
    assert!(shape.is_within(i("a1x"), i("a")));
    assert!(shape.is_within(i("a"), i("a")));
    assert!(!shape.is_within(i("a"), i("a1")));
    assert!(!shape.is_within(i("b"), i("a")));
}

/// A row that takes no children has no middle: half before, half after.
#[test]
fn a_row_that_takes_no_children_splits_in_half() {
    assert_eq!(zone_at(0.3, false), DropZone::Before);
    assert_eq!(zone_at(0.49, false), DropZone::Before);
    assert_eq!(zone_at(0.5, false), DropZone::After);
    assert_eq!(zone_at(0.7, false), DropZone::After);
}

/// The predicate reaches the shape, and after an open row that takes no
/// children is after it, not first inside it.
#[test]
fn after_an_open_row_that_takes_no_children_is_after_it() {
    let only_a: AcceptsChildren = Rc::new(|node: &TreeNodeData| node.value == "a");
    let shape = Shape::of(&data(), Some(&only_a));
    assert!(shape.rows[shape.index("a").unwrap()].accepts);
    assert!(!shape.rows[shape.index("c").unwrap()].accepts);
    let open = expanded(&["a", "c"]);
    assert_eq!(
        place(&shape, "a", DropZone::After, None, &open),
        (Some("a".into()), TreeSlot::First, 1)
    );
    assert_eq!(
        place(&shape, "c", DropZone::After, None, &open),
        (None, after("c"), 0)
    );
}

/// A drop into a dragged row's own subtree is refused whatever the app
/// says. No drag reaches it any more (the guard); it is pinned here.
#[test]
fn a_drop_into_a_dragged_rows_subtree_is_refused() {
    let shape = Shape::of(&data(), None);
    let i = |v: &str| shape.index(v).unwrap();
    let drop = TreeDrop {
        dragged: vec!["a".into()],
        parent: Some("a1".into()),
        slot: TreeSlot::Last,
        over: "a1".into(),
        zone: DropZone::Inside,
    };
    let allow: JudgeDrop = Rc::new(|_: &TreeDrop| DropVerdict::Allowed);
    let into = Placement {
        parent: Some(i("a1")),
        slot: Slot::Last,
        line_level: 1,
    };
    assert_eq!(
        judge(&shape, &into, &[i("a")], &drop, Some(&allow)),
        DropVerdict::Refused
    );
    assert_eq!(
        judge(&shape, &into, &[i("a")], &drop, None),
        DropVerdict::Refused
    );
    // Positive control: the same drop of a row from elsewhere is the app's.
    assert_eq!(
        judge(&shape, &into, &[i("b")], &drop, Some(&allow)),
        DropVerdict::Allowed
    );
    let refuse: JudgeDrop = Rc::new(|_: &TreeDrop| DropVerdict::Refused);
    assert_eq!(
        judge(&shape, &into, &[i("b")], &drop, Some(&refuse)),
        DropVerdict::Refused
    );
}
