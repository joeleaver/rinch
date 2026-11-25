use super::*;

/// a (open: a1, a2), b, c (closed: c1)
fn shape() -> Shape {
    Shape::of(
        &[
            TreeNodeData::new("a", "Apples").with_children(vec![
                TreeNodeData::new("a1", "Avocado"),
                TreeNodeData::new("a2", "Banana"),
            ]),
            TreeNodeData::new("b", "Beets"),
            TreeNodeData::new("c", "Carrots").with_child(TreeNodeData::new("c1", "Celery")),
        ],
        None,
    )
}

fn open() -> HashSet<String> {
    ["a".to_string()].into_iter().collect()
}

fn to(shape: &Shape, step: Option<Nav>) -> String {
    match step {
        Some(Nav::To(row)) => shape.rows[row].value.clone(),
        other => panic!("expected a move, got {other:?}"),
    }
}

#[test]
fn arrows_walk_the_rows_drawn() {
    let s = shape();
    let i = |v: &str| s.index(v).unwrap();
    assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowDown")), "a1");
    assert_eq!(to(&s, nav(&s, &open(), i("a2"), "ArrowDown")), "b");
    assert_eq!(
        to(&s, nav(&s, &open(), i("c"), "ArrowDown")),
        "c",
        "the last row stays"
    );
    assert_eq!(to(&s, nav(&s, &open(), i("b"), "ArrowUp")), "a2");
    assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowUp")), "a");
    assert_eq!(to(&s, nav(&s, &open(), i("a1"), "End")), "c");
    assert_eq!(to(&s, nav(&s, &open(), i("c"), "Home")), "a");
}

/// #3062: PageUp and PageDown are not in the WAI-ARIA tree pattern, so the
/// tree does not handle them, from any row.
#[test]
fn page_up_and_page_down_are_not_the_trees() {
    let s = shape();
    for row in ["a", "a1", "b", "c"] {
        let at = s.index(row).unwrap();
        assert_eq!(nav(&s, &open(), at, "PageDown"), None, "PageDown on {row}");
        assert_eq!(nav(&s, &open(), at, "PageUp"), None, "PageUp on {row}");
    }
}

#[test]
fn right_and_left_open_close_and_step() {
    let s = shape();
    let i = |v: &str| s.index(v).unwrap();
    assert_eq!(
        nav(&s, &open(), i("c"), "ArrowRight"),
        Some(Nav::Expand(i("c")))
    );
    assert_eq!(to(&s, nav(&s, &open(), i("a"), "ArrowRight")), "a1");
    assert_eq!(
        nav(&s, &open(), i("b"), "ArrowRight"),
        None,
        "a leaf has nowhere to go"
    );
    assert_eq!(
        nav(&s, &open(), i("a"), "ArrowLeft"),
        Some(Nav::Collapse(i("a")))
    );
    assert_eq!(to(&s, nav(&s, &open(), i("a2"), "ArrowLeft")), "a");
    assert_eq!(nav(&s, &open(), i("a1"), "Enter"), None);
    assert_eq!(
        nav(&s, &open(), i("b"), "*"),
        Some(Nav::ExpandAll(vec![i("a"), i("c")]))
    );
}

#[test]
fn type_ahead_finds_the_next_row_and_steps_on_one_letter() {
    let s = shape();
    let labels: HashMap<String, String> = [
        ("a", "Apples"),
        ("a1", "Avocado"),
        ("a2", "Banana"),
        ("b", "Beets"),
        ("c", "Carrots"),
        ("c1", "Celery"),
    ]
    .into_iter()
    .map(|(v, l)| (v.to_string(), l.to_string()))
    .collect();
    let text = |v: &str| labels[v].clone();
    let i = |v: &str| s.index(v).unwrap();
    let name = |row: Option<usize>| row.map(|r| s.rows[r].value.clone());
    assert_eq!(
        name(type_ahead(&s, &open(), i("a"), "b", &text)),
        Some("a2".into())
    );
    assert_eq!(
        name(type_ahead(&s, &open(), i("a2"), "b", &text)),
        Some("b".into())
    );
    assert_eq!(
        name(type_ahead(&s, &open(), i("a2"), "be", &text)),
        Some("b".into())
    );
    assert_eq!(
        name(type_ahead(&s, &open(), i("a"), "aa", &text)),
        Some("a1".into()),
        "a repeated letter steps on"
    );
    assert_eq!(
        name(type_ahead(&s, &open(), i("a"), "ce", &text)),
        None,
        "Celery is not drawn"
    );
}

fn key(cursor: &str) -> TreeKey {
    let s = Rc::new(shape());
    let parent = s.rows[s.index(cursor).unwrap()]
        .parent
        .map(|p| s.rows[p].value.clone());
    TreeKey {
        key: String::new(),
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
        cursor: cursor.into(),
        parent,
        above: None,
        below: None,
        rows: vec![s.index(cursor).unwrap()],
        shape: s,
        carried: vec![cursor.into()],
        judge_drop: None,
    }
}

fn place(drop: Option<TreeDrop>) -> Option<(Option<String>, TreeSlot)> {
    drop.map(|d| (d.parent, d.slot))
}

#[test]
fn structural_moves_are_drops() {
    let after = |v: &str| TreeSlot::After(v.into());
    assert_eq!(
        place(key("a2").drop_for(TreeMove::Indent)),
        Some((Some("a1".into()), TreeSlot::Last))
    );
    assert_eq!(
        place(key("a1").drop_for(TreeMove::Indent)),
        None,
        "no sibling above"
    );
    assert_eq!(
        place(key("a2").drop_for(TreeMove::Outdent)),
        Some((None, after("a")))
    );
    assert_eq!(
        place(key("b").drop_for(TreeMove::Outdent)),
        None,
        "the top level"
    );
    assert_eq!(
        place(key("a2").drop_for(TreeMove::Up)),
        Some((Some("a".into()), TreeSlot::First))
    );
    assert_eq!(
        place(key("c").drop_for(TreeMove::Up)),
        Some((None, after("a")))
    );
    assert_eq!(place(key("a").drop_for(TreeMove::Up)), None);
    assert_eq!(
        place(key("a1").drop_for(TreeMove::Down)),
        Some((Some("a".into()), after("a2")))
    );
    assert_eq!(place(key("c").drop_for(TreeMove::Down)), None);
}
