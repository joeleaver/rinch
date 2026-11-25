//! A drag targets only lists of its own group, at every level.
use super::*;

fn columns(f: &Fixture, lane: &str) -> Vec<String> {
    f.board
        .get()
        .into_iter()
        .find(|l| l.id == lane)
        .unwrap()
        .cols
        .into_iter()
        .map(|c| c.id)
        .collect()
}

#[test]
fn a_card_drags_alone_and_not_its_column() {
    let mut f = fixture();
    start_card(&mut f, "b");
    assert_eq!(
        carrying(&f, "rinch-sortable__item--dragging"),
        vec![item(&f, "b")]
    );
    // Over its own column's header: the columns are not a card's place.
    let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
    move_to(&mut f, header);
    assert_eq!(last_offer(&f), None);
    release_at(&mut f, header);
    assert!(drops().is_empty());
}

#[test]
fn a_column_drags_by_its_header_along_the_row() {
    let mut f = fixture();
    let header = point(&f, handle_of(&f, "c1"), 0.5, 0.5);
    start_at(&mut f, header);
    assert_eq!(
        carrying(&f, "rinch-sortable__item--dragging"),
        vec![item(&f, "c1")]
    );
    // The right part of c2's header: after c2.
    let at = point(&f, handle_of(&f, "c2"), 0.8, 0.5);
    move_to(&mut f, at);
    let expected = mv(
        "columns",
        &["c1"],
        "l1",
        "l1",
        place(Some("c2"), Some("c3"), 1),
    );
    assert_eq!(
        last_offer(&f),
        Some((expected.clone(), SortVerdict::Accept))
    );
    let class = classes(&f, item(&f, "c2"));
    assert!(
        class.contains("rinch-sortable__item--drop-after")
            && class.contains("rinch-sortable__item--drop-horizontal"),
        "{class}"
    );
    release_at(&mut f, at);
    assert_eq!(drops(), vec![expected]);
    assert_eq!(columns(&f, "l1"), ["c2", "c1", "c3", "c4"]);
}

#[test]
fn a_column_dragged_over_cards_is_placed_among_the_columns() {
    let mut f = fixture();
    let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
    start_at(&mut f, header);
    // Over card b, in the left half of column c1: before c1.
    let at = point(&f, item(&f, "b"), 0.2, 0.5);
    move_to(&mut f, at);
    assert_eq!(
        last_offer(&f).map(|(m, _)| (m.group, m.place)),
        Some(("columns".to_string(), place(None, Some("c1"), 0)))
    );
    assert!(
        carrying(&f, "rinch-sortable__item--drop-before") == vec![item(&f, "c1")],
        "drawn on the column, not the card"
    );
    release_at(&mut f, at);
    assert_eq!(columns(&f, "l1"), ["c2", "c1", "c3", "c4"]);
}

#[test]
fn a_column_moves_to_another_lane() {
    let mut f = fixture();
    let header = point(&f, handle_of(&f, "c2"), 0.5, 0.5);
    start_at(&mut f, header);
    let at = point(&f, handle_of(&f, "c6"), 0.7, 0.5);
    move_to(&mut f, at);
    assert_eq!(
        last_offer(&f).map(|(m, _)| (m.to_list, m.place)),
        Some(("l2".to_string(), place(Some("c6"), None, 2)))
    );
    release_at(&mut f, at);
    assert_eq!(columns(&f, "l1"), ["c1", "c3", "c4"]);
    assert_eq!(columns(&f, "l2"), ["c5", "c6", "c2"]);
    assert_eq!(cards(&f, "c2"), ["d"], "its cards went with it");
}

#[test]
fn a_lane_drags_by_its_handle() {
    let mut f = fixture();
    let handle = point(&f, handle_of(&f, "l2"), 0.5, 0.5);
    start_at(&mut f, handle);
    // The top of lane l1's handle: before l1.
    let at = point(&f, handle_of(&f, "l1"), 0.5, 0.2);
    move_to(&mut f, at);
    assert_eq!(
        last_offer(&f).map(|(m, _)| (m.group, m.place)),
        Some(("lanes".to_string(), place(None, Some("l1"), 0)))
    );
    release_at(&mut f, at);
    let order: Vec<String> = f.board.get().into_iter().map(|l| l.id).collect();
    assert_eq!(order, ["l2", "l1"]);
}

#[test]
fn a_press_on_a_card_that_never_passes_the_threshold_is_a_click() {
    let mut f = fixture();
    let at = point(&f, item(&f, "b"), 0.5, 0.5);
    press_at(&mut f, at);
    move_to(&mut f, (at.0 + 2.0, at.1));
    release_at(&mut f, (at.0 + 2.0, at.1));
    assert!(f.app.active_dnd.is_none());
    assert!(OFFERS.with(|o| o.borrow().is_empty()));
    // The click focused the card (it is focusable), as a click does.
    assert_eq!(f.app.focus_target, FocusTarget::Node(item(&f, "b")));
}
