use super::*;

/// a (a1, a2), b, c (c1)
fn data() -> Vec<TreeNodeData> {
    vec![
        TreeNodeData::new("a", "A").with_children(vec![
            TreeNodeData::new("a1", "A1"),
            TreeNodeData::new("a2", "A2"),
        ]),
        TreeNodeData::new("b", "B"),
        TreeNodeData::new("c", "C").with_child(TreeNodeData::new("c1", "C1")),
    ]
}

fn key(shape: Shape, cursor: &str, carried: &[&str]) -> TreeKey {
    let s = Rc::new(shape);
    TreeKey {
        key: String::new(),
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
        cursor: cursor.into(),
        parent: None,
        above: None,
        below: None,
        rows: vec![s.index(cursor).unwrap()],
        shape: s,
        carried: carried.iter().map(|c| c.to_string()).collect(),
        judge_drop: None,
    }
}

/// RU1: a drag never offers Inside on a row `accepts_children` refuses;
/// the keyboard's Indent does.
#[test]
fn ru1_indent_into_a_row_that_takes_no_children_is_none() {
    let leaves_take_none: AcceptsChildren = Rc::new(|n: &TreeNodeData| n.has_children());
    let shape = Shape::of(&data(), Some(&leaves_take_none));
    // c's previous sibling is b, a leaf that takes no children.
    let drop = key(shape, "c", &["c"]).drop_for(TreeMove::Indent);
    assert!(
        drop.is_none(),
        "Indent into b, which accepts no children: {:?}",
        drop.map(|d| (d.parent, d.slot))
    );
}

/// A keyboard move the app's judge refuses is no move, as a dragged one
/// is not delivered; one it allows is the same drop as without a judge.
#[test]
fn a_move_the_app_refuses_is_none() {
    let mut k = key(Shape::of(&data(), None), "c", &["c"]);
    k.judge_drop = Some(Rc::new(|d: &TreeDrop| {
        if d.parent.as_deref() == Some("b") {
            DropVerdict::Refused
        } else {
            DropVerdict::Allowed
        }
    }));
    assert!(k.drop_for(TreeMove::Indent).is_none(), "into b is refused");
    assert_eq!(
        k.drop_for(TreeMove::Up).map(|d| d.slot),
        Some(TreeSlot::After("a".into())),
        "control: another move is allowed"
    );
}

/// RU2: with drag_values carrying more than the cursor row (a
/// multi-selection), Indent can put a carried row into itself. A drag
/// refuses that (`judge`'s cycle check); the keyboard hands it over.
#[test]
fn ru2_indent_into_a_carried_row_is_none() {
    let shape = Shape::of(&data(), None);
    let drop = key(shape, "a2", &["a1", "a2"]).drop_for(TreeMove::Indent);
    assert!(
        drop.is_none(),
        "a1 would move into itself: {:?}",
        drop.map(|d| (d.dragged, d.parent, d.slot))
    );
}
