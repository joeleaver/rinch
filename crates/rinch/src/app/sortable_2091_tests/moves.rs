//! A card moved within and between columns by pointer, judged and delivered.

use super::*;

#[test]
fn a_card_moves_within_its_column() {
    let mut f = fixture();
    start_card(&mut f, "a");
    assert!(classes(&f, item(&f, "a")).contains("rinch-sortable__item--dragging"));

    // Below the middle of c: after it.
    let at = point(&f, item(&f, "c"), 0.5, 0.7);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c1", place(Some("c"), Some("i"), 2));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-after"),
        vec![item(&f, "c")],
        "the caret is drawn after c, and only there"
    );

    // Above the middle of c: before it, which is after b.
    let above = point(&f, item(&f, "c"), 0.5, 0.3);
    move_to(&mut f, above);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    move_to(&mut f, at);

    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["b", "c", "a", "i", "j"]);
    assert_eq!(OFFERS.with(|o| o.borrow().last().cloned()), Some(None));
    assert_clean(&f);
}

#[test]
fn a_card_over_its_own_place_offers_nothing() {
    let mut f = fixture();
    start_card(&mut f, "b");
    // The bottom of a: after a, which is where b is.
    let at = point(&f, item(&f, "a"), 0.5, 0.8);
    move_to(&mut f, at);
    assert_eq!(last_offer(&f), None);
    release_at(&mut f, at);
    assert!(drops().is_empty());
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "i", "j"]);
}

#[test]
fn a_card_moves_to_another_column() {
    let mut f = fixture();
    start_card(&mut f, "b");
    let at = point(&f, item(&f, "d"), 0.5, 0.25);
    move_to(&mut f, at);
    let expected = mv("cards", &["b"], "c1", "c2", place(None, Some("d"), 0));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-before"),
        vec![item(&f, "d")]
    );
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["a", "c", "i", "j"]);
    assert_eq!(cards(&f, "c2"), ["b", "d"]);
}

#[test]
fn over_a_list_between_its_cards_the_nearest_gap_is_offered() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let c2 = list_holding(&f, "d");
    let (lx, ly, lw, _) = box_of(&f, c2);
    let (_, dy, _, _) = box_of(&f, item(&f, "d"));
    assert!(dy >= ly + 6.0, "the list's padding is above d");
    // In the padding above d: first.
    move_to(&mut f, (lx + lw / 2.0, ly + 3.0));
    assert_eq!(last_offer(&f).unwrap().0.place, place(None, Some("d"), 0));
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-before"),
        vec![item(&f, "d")]
    );
    // Below d, in the list's empty space: last.
    move_to(&mut f, (lx + lw / 2.0, dy + 60.0));
    assert_eq!(last_offer(&f).unwrap().0.place, place(Some("d"), None, 1));
}

#[test]
fn a_card_drops_into_an_empty_column_of_another_lane() {
    let mut f = fixture();
    start_card(&mut f, "a");
    // c6 is empty: the pointer is over its card list itself.
    let c6_list = {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        d.tree
            .nodes
            .iter()
            .find(|(id, n)| {
                n.attributes.get("data-sortable-group").map(String::as_str) == Some("cards")
                    && connected(&d, *id)
                    && d.tree.nodes[n.parent.unwrap()]
                        .attributes
                        .get("data-sortable-key")
                        .map(String::as_str)
                        == Some("c6")
            })
            .map(|(id, _)| id)
            .unwrap()
    };
    let at = point(&f, c6_list, 0.5, 0.6);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c6", place(None, None, 0));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    assert!(expected.place.is_first() && expected.place.is_last());
    assert!(classes(&f, c6_list).contains("rinch-sortable--drop-into"));
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c6"), ["a"]);
}

#[test]
fn a_refused_drop_shows_its_reason_before_the_release_and_is_not_delivered() {
    let mut f = fixture();
    start_card(&mut f, "a");
    // f in c3, whose WIP limit refuses cards from elsewhere. c3 starts at
    // x 400; the row shows up to 500.
    let (fx, _, _, _) = box_of(&f, item(&f, "f"));
    let (_, fy, _, fh) = box_of(&f, item(&f, "f"));
    let at = (fx + 20.0, fy + fh * 0.75);
    move_to(&mut f, at);
    let expected = mv("cards", &["a"], "c1", "c3", place(Some("f"), Some("g"), 2));
    assert_eq!(
        last_offer(&f),
        Some((expected, SortVerdict::Refuse("WIP limit 3".into())))
    );
    let f_item = item(&f, "f");
    let class = classes(&f, f_item);
    assert!(
        class.contains("rinch-sortable__item--drop-after"),
        "{class}"
    );
    assert!(class.contains("rinch-sortable__item--refused"), "{class}");
    // The reason is on screen, beside the caret, before any release.
    let shown = carrying(&f, "rinch-sortable__reason--shown");
    assert_eq!(shown.len(), 1);
    {
        let doc = f.app.doc.as_ref().unwrap();
        let d = doc.borrow();
        assert_eq!(d.tree.nodes[shown[0]].parent, Some(f_item));
        let text: String = d.tree.nodes[shown[0]]
            .children
            .iter()
            .filter_map(|c| d.tree.nodes[*c].text_content())
            .collect();
        assert_eq!(text, "WIP limit 3");
    }
    let (_, ry, rw, rh) = box_of(&f, shown[0]);
    assert!(rw > 0.0 && rh > 0.0, "the reason has a box");
    // At the caret's edge of f (its bottom), inside f's box: a box hanging
    // below it would grow the scrolling list's overflow.
    assert!(
        ry + rh <= fy + fh && ry + rh > fy + fh / 2.0,
        "at f's bottom edge, inside it: {ry}+{rh} vs {fy}+{fh}"
    );

    release_at(&mut f, at);
    assert!(drops().is_empty(), "a refused drop is not delivered");
    assert_eq!(cards(&f, "c1"), ["a", "b", "c", "i", "j"]);
    assert_eq!(cards(&f, "c3"), ["e", "f", "g"]);
    assert_clean(&f);
}

#[test]
fn a_drag_survives_a_rerender_of_its_list_and_offers_from_the_new_rows() {
    let mut f = fixture();
    start_card(&mut f, "a");
    let at = point(&f, item(&f, "c"), 0.5, 0.75);
    move_to(&mut f, at);
    // A collaborator's edit re-renders the lane under the drag: every node
    // of it, the dragged card's included, is new.
    let old_a = item(&f, "a");
    f.board.update(|b| b[0].cols[0].cards.push("k".into()));
    f.app.resolve_and_repaint(VIEWPORT.0, VIEWPORT.1);
    assert_ne!(item(&f, "a"), old_a, "the card was re-rendered");
    // The next move offers from the new rows.
    let at = point(&f, item(&f, "b"), 0.5, 0.75);
    move_to(&mut f, at);
    assert_eq!(
        last_offer(&f).unwrap().0.place,
        place(Some("b"), Some("c"), 1)
    );
    assert_eq!(
        carrying(&f, "rinch-sortable__item--drop-after"),
        vec![item(&f, "b")]
    );
    release_at(&mut f, at);
    assert_eq!(cards(&f, "c1"), ["b", "a", "c", "i", "j", "k"]);
    assert_clean(&f);
}

#[test]
fn several_selected_cards_move_together_in_order() {
    let mut f = fixture();
    SELECTION.with(|s| *s.borrow_mut() = vec!["a".into(), "c".into()]);
    start_card(&mut f, "c");
    let dimmed = carrying(&f, "rinch-sortable__item--dragging");
    assert_eq!(dimmed.len(), 2);
    assert!(dimmed.contains(&item(&f, "a")) && dimmed.contains(&item(&f, "c")));
    let at = point(&f, item(&f, "d"), 0.5, 0.75);
    move_to(&mut f, at);
    let expected = mv("cards", &["a", "c"], "c1", "c2", place(Some("d"), None, 1));
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(cards(&f, "c1"), ["b", "i", "j"]);
    assert_eq!(cards(&f, "c2"), ["d", "a", "c"]);
}
