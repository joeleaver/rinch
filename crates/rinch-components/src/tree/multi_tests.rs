use super::*;

/// a (open: a1, a2 (open: a2x)), b, c
fn shape() -> Shape {
    Shape::of(
        &[
            TreeNodeData::new("a", "a").with_children(vec![
                TreeNodeData::new("a1", "a1"),
                TreeNodeData::new("a2", "a2").with_child(TreeNodeData::new("a2x", "a2x")),
            ]),
            TreeNodeData::new("b", "b"),
            TreeNodeData::new("c", "c"),
        ],
        None,
    )
}

fn set(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| v.to_string()).collect()
}

fn names(s: &Shape, rows: &[usize]) -> Vec<String> {
    rows.iter().map(|&r| s.rows[r].value.clone()).collect()
}

#[test]
fn a_run_is_the_rows_drawn_between_either_way() {
    let s = shape();
    let open = set(&["a", "a2"]);
    let i = |v: &str| s.index(v).unwrap();
    assert_eq!(
        names(&s, &s.run(&open, i("a1"), i("b"))),
        ["a1", "a2", "a2x", "b"]
    );
    assert_eq!(
        names(&s, &s.run(&open, i("b"), i("a1"))),
        ["a1", "a2", "a2x", "b"]
    );
    let closed = set(&["a"]);
    assert_eq!(
        names(&s, &s.run(&closed, i("a1"), i("b"))),
        ["a1", "a2", "b"]
    );
}

#[test]
fn a_selection_carries_top_down_without_rows_inside_selected_rows() {
    let s = shape();
    let open = set(&["a", "a2"]);
    let i = |v: &str| s.index(v).unwrap();
    // Out of order, with a child of a selected row.
    let carried = s.carried(&open, &set(&["c", "a2x", "a2", "a1"]), i("c"), true);
    assert_eq!(names(&s, &carried), ["a1", "a2", "c"]);
    // A row outside the selection carries itself.
    assert_eq!(
        names(&s, &s.carried(&open, &set(&["a1", "c"]), i("b"), true)),
        ["b"]
    );
    // Without multiple selection, always the row.
    assert_eq!(
        names(&s, &s.carried(&open, &set(&["a1", "c"]), i("c"), false)),
        ["c"]
    );
}

#[test]
fn around_a_block_skips_what_is_inside_it() {
    let s = shape();
    let open = set(&["a", "a2"]);
    let i = |v: &str| s.index(v).unwrap();
    let name = |r: Option<usize>| r.map(|r| s.rows[r].value.clone());
    // a2 is open: the row below it is b, not its child.
    let (above, below) = s.around(&open, &[i("a2")]);
    assert_eq!(
        (name(above), name(below)),
        (Some("a1".into()), Some("b".into()))
    );
    let (above, below) = s.around(&open, &[i("a1"), i("b")]);
    assert_eq!(
        (name(above), name(below)),
        (Some("a".into()), Some("c".into()))
    );
    let (above, below) = s.around(&open, &[i("c")]);
    assert_eq!((name(above), name(below)), (Some("b".into()), None));
}

#[test]
fn a_block_moves_as_one() {
    let s = Rc::new(shape());
    let key = |carried: &[&str]| TreeKey {
        key: String::new(),
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
        cursor: carried[carried.len() - 1].into(),
        parent: None,
        above: None,
        below: None,
        shape: s.clone(),
        rows: carried.iter().filter_map(|v| s.index(v)).collect(),
        carried: carried.iter().map(|v| v.to_string()).collect(),
        judge_drop: None,
    };
    let place = |d: Option<TreeDrop>| d.map(|d| (d.parent, d.slot));
    // b and c: up goes before a; indent goes into a (the row above the
    // first); down has nowhere to go.
    assert_eq!(
        place(key(&["b", "c"]).drop_for(TreeMove::Up)),
        Some((None, TreeSlot::First))
    );
    assert_eq!(
        place(key(&["b", "c"]).drop_for(TreeMove::Indent)),
        Some((Some("a".into()), TreeSlot::Last))
    );
    assert_eq!(place(key(&["b", "c"]).drop_for(TreeMove::Down)), None);
    // Rows of different parents do not move as one, even where the first
    // of them could move on its own (a2 has a1 above it).
    assert_eq!(place(key(&["a1", "b"]).drop_for(TreeMove::Up)), None);
    assert_eq!(place(key(&["a2", "b"]).drop_for(TreeMove::Up)), None);
    assert_eq!(place(key(&["a2", "b"]).drop_for(TreeMove::Indent)), None);
    // First and last share a parent, a row between them does not.
    assert_eq!(place(key(&["b", "a2x", "c"]).drop_for(TreeMove::Up)), None);
    assert_eq!(
        place(key(&["b", "c"]).drop_for(TreeMove::Up)),
        Some((None, TreeSlot::First)),
        "control: b and c alone move up"
    );
    assert_eq!(
        place(key(&["a2"]).drop_for(TreeMove::Up)),
        Some((Some("a".into()), TreeSlot::First)),
        "control: a2 alone moves up"
    );
}
